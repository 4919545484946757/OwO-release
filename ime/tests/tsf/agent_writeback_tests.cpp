#include "owo/tsf/agent_writeback.h"
#include <richedit.h>
#include <iostream>
#include <vector>

// Deterministic host refusal injection; no external app or real document.
static WNDPROC original_editor_proc{};
static LRESULT CALLBACK refuse_replace(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == EM_REPLACESEL) return 0;
    return CallWindowProcW(original_editor_proc, window, message, wparam, lparam);
}

static void probe_undo(HWND editor, const wchar_t* source, const char* label, bool unicode_write = false) {
    SetWindowTextW(editor, L"PREFIXtail");
    SendMessageW(editor, EM_EMPTYUNDOBUFFER, 0, 0);
    CHARRANGE selected{6, 10};
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selected));
    // Deliberately bypass preparation to retain evidence of the native defect.
    const std::wstring native(source);
    const auto mode = SendMessageW(editor, EM_GETTEXTMODE, 0, 0);
    SendMessageW(editor, EM_STOPGROUPTYPING, 0, 0);
    if (unicode_write) {
        SETTEXTEX spec{ST_SELECTION | ST_KEEPUNDO | ST_UNICODE, 1200};
        const auto written = SendMessageW(editor, EM_SETTEXTEX, reinterpret_cast<WPARAM>(&spec), reinterpret_cast<LPARAM>(native.c_str()));
        std::cout << "unicode_write_count=" << written << "\n";
    } else {
        SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(native.c_str()));
    }
    SendMessageW(editor, EM_STOPGROUPTYPING, 0, 0);
    const auto name = SendMessageW(editor, EM_GETUNDONAME, 0, 0);
    const auto undone = SendMessageW(editor, EM_UNDO, 0, 0);
    wchar_t text[128]{};
    GetWindowTextW(editor, text, 128);
    std::cout << "probe=" << label << " mode=" << mode << " undo_name=" << name
              << " undo_return=" << undone << " restored=" << (std::wstring_view(text) == L"PREFIXtail")
              << " undo_remaining=" << SendMessageW(editor, EM_CANUNDO, 0, 0) << " codes=";
    for (const auto c : std::wstring_view(text)) std::cout << static_cast<unsigned>(c) << ',';
    std::cout << "\n";
}

// Read native paragraph positions without GetWindowText's CRLF expansion.
static std::wstring read_native_document(HWND editor) {
    GETTEXTLENGTHEX spec{GTL_PRECISE | GTL_NUMCHARS, 1200};
    const auto length = SendMessageW(editor, EM_GETTEXTLENGTHEX,
        reinterpret_cast<WPARAM>(&spec), 0);
    if (length < 0 || length > 4096) return L"READ_FAILED";
    std::vector<wchar_t> buffer(static_cast<std::size_t>(length) + 1);
    TEXTRANGEW range{{0, static_cast<LONG>(length)}, buffer.data()};
    const auto count = length == 0 ? 0 : SendMessageW(editor, EM_GETTEXTRANGE,
        0, reinterpret_cast<LPARAM>(&range));
    if (count != length) return L"READ_FAILED";
    return std::wstring(buffer.data(), static_cast<std::size_t>(count));
}

static bool check_position_boundaries(HWND editor) {
    struct Case { const wchar_t* before; LONG start, length; const wchar_t* replacement; };
    const Case cases[] = {
        {L"head\rOLD\rfoot", 5, 3, L"\u4F60\n\u597D\r\nend"},
        {L"same|same|same", 5, 4, L"new\nline"},
        {L"tail", 0, 0, L"start\n"},
        {L"prefixSUFFIX", 6, 0, L"\u4F60\n\u597D"},
        {L"prefix", 6, 0, L"\nend"},
        {L"", 0, 0, L"\u4F60\n\u597D"},
        {L"head\rOLD\rfoot", 0, 13, L"a\n\nend"},
    };
    bool passed = true;
    int index = 0;
    for (const auto& sample : cases) {
        const std::wstring before(sample.before);
        const auto prepared = owo::tsf::richedit_native_text(sample.replacement);
        if (!prepared) return false;
        SetWindowTextW(editor, sample.before);
        SendMessageW(editor, EM_EMPTYUNDOBUFFER, 0, 0);
        CHARRANGE selected{sample.start, sample.start + sample.length};
        SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selected));
        std::wstring expected = before;
        expected.replace(static_cast<std::size_t>(sample.start),
            static_cast<std::size_t>(sample.length), *prepared);
        SendMessageW(editor, EM_STOPGROUPTYPING, 0, 0);
        SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(prepared->c_str()));
        SendMessageW(editor, EM_STOPGROUPTYPING, 0, 0);
        const bool exact = read_native_document(editor) == expected;
        const bool verified = owo::tsf::verify_richedit_write(editor, sample.start,
            *prepared, static_cast<LRESULT>(expected.size()));
        const bool undone = SendMessageW(editor, EM_UNDO, 0, 0) != 0 &&
            read_native_document(editor) == before;
        const bool no_extra_undo = SendMessageW(editor, EM_CANUNDO, 0, 0) == 0;
        passed = exact && verified && undone && no_extra_undo && passed;
        std::cout << "position_case=" << index++ << " exact_whole_document=" << exact
                  << " verified=" << verified << " one_undo=" << undone
                  << " no_extra_undo=" << no_extra_undo << "\n";
    }
    return passed;
}

int main() {
    const auto library = LoadLibraryW(L"Msftedit.dll");
    const auto editor = library ? CreateWindowExW(0, MSFTEDIT_CLASS, L"",
        WS_POPUP | ES_MULTILINE, -30000, -30000, 100, 100,
        nullptr, nullptr, GetModuleHandleW(nullptr), nullptr) : nullptr;
    if (!editor) return 1;
    int probe_index = 0;
    for (const auto text : {L"e\u0301\n\u4F60\u597D", L"a\n\u4F60\u597D",
                            L"e\u0301\u4F60\u597D", L"e\u0301\nABC"}) {
        const auto fresh = CreateWindowExW(0, MSFTEDIT_CLASS, L"", WS_POPUP | ES_MULTILINE,
            -30000, -30000, 100, 100, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
        if (!fresh) return 2;
        std::cout << "probe_index=" << probe_index++ << "\n";
        probe_undo(fresh, text, "fresh");
        probe_undo(fresh, text, "unicode-message", true);
        probe_undo(editor, text, "reused");
        DestroyWindow(fresh);
    }
    bool passed = check_position_boundaries(editor);
    const owo::agent::CommandEffect refused{owo::agent::EffectKind::replace_selection,
        "full synthetic e\xCC\x81 result", "original-context", "original-hash"};
    const auto copy = owo::tsf::native_writeback_copy_option(
        owo::tsf::NativeWriteResult::unsupported, refused, true);
    passed = copy && copy->effect && copy->effect->kind == owo::agent::EffectKind::copy_text &&
        copy->effect->text == refused.text && copy->effect->target_context_id.empty() &&
        copy->effect->selection_sha256.empty() && !copy->commit_task && copy->slot_updates.empty() && passed;
    for (const auto status : {owo::tsf::NativeWriteResult::uncertain,
                              owo::tsf::NativeWriteResult::rejected,
                              owo::tsf::NativeWriteResult::completed,
                              owo::tsf::NativeWriteResult::not_applicable})
        passed = !owo::tsf::native_writeback_copy_option(status, refused, true) && passed;
    passed = !owo::tsf::native_writeback_copy_option(
        owo::tsf::NativeWriteResult::unsupported, refused, false) && passed;
    std::cout << "explicit_copy_fallback_contract=" << passed << "\n";
    // A rejected write can still match an existing prefix and older undo history.
    SetWindowTextW(editor, L"PREFIXtail");
    CHARRANGE history{10, 10};
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&history));
    SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(L"Z"));
    CHARRANGE unchanged_selection{6, 10};
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&unchanged_selection));
    original_editor_proc = reinterpret_cast<WNDPROC>(SetWindowLongPtrW(
        editor, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(refuse_replace)));
    SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(L"ta"));
    SetWindowLongPtrW(editor, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(original_editor_proc));
    wchar_t prefix[3]{};
    TEXTRANGEW prefix_range{{6, 8}, prefix};
    const bool legacy_false_positive = SendMessageW(editor, EM_GETTEXTRANGE, 0,
        reinterpret_cast<LPARAM>(&prefix_range)) == 2 &&
        std::wstring_view(prefix, 2) == L"ta" && SendMessageW(editor, EM_CANUNDO, 0, 0) != 0;
    const bool refused_verified = owo::tsf::verify_richedit_write(editor, 6, L"ta", 9);
    passed = legacy_false_positive && !refused_verified && passed;
    // Correct length alone cannot hide the unchanged selection.
    passed = !owo::tsf::verify_richedit_write(editor, 6, L"ta", 11) && passed;
    // Even a matching caret cannot hide an incorrect whole-document length.
    CHARRANGE matching_caret{8, 8};
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&matching_caret));
    passed = !owo::tsf::verify_richedit_write(editor, 6, L"ta", 9) && passed;

    std::cout << "blocked_write_legacy_false_positive=" << legacy_false_positive
              << " strengthened_verification=" << !refused_verified << "\n";
    int case_index = 0;
    for (const auto source : {L"Hello\nWorld", L"Hello\r\nWorld", L"Hello\rWorld",
                              L"\n\n", L"\r\n\r\n", L"a\t b\u2028c", L"",
                              L"a\u2029b", L"a\u0085b", L"a\vb", L"a\fb",
                              L"a\U0001F600b", L"e\u0301\n\u4F60\u597D", L"a\r\r\nb\n",
                              L"e\u0301\u4F60\u597D", L"e\u0301ABC", L"\u0915\u093E",
                              L"\u00E9\u4F60\u597D"}) {
        SetWindowTextW(editor, L"PREFIXtail");
        SendMessageW(editor, EM_EMPTYUNDOBUFFER, 0, 0);
        CHARRANGE selected{6, 10};
        SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selected));
        const auto prepared = owo::tsf::richedit_native_text(source);
        if (!prepared) {
            wchar_t unchanged[32]{};
            GetWindowTextW(editor, unchanged, 32);
            passed = (case_index == 5 || case_index == 7 || case_index == 12 || (case_index >= 14 && case_index <= 16)) && std::wstring_view(unchanged) == L"PREFIXtail" &&
                SendMessageW(editor, EM_CANUNDO, 0, 0) == 0 && passed;
            std::cout << "case=" << case_index++ << " rejected_before_write=1 unchanged=1\n";
            continue;
        }
        const auto& native = *prepared;
        SendMessageW(editor, EM_STOPGROUPTYPING, 0, 0);
        SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(native.c_str()));
        SendMessageW(editor, EM_STOPGROUPTYPING, 0, 0);
        std::vector<wchar_t> buffer(native.size() + 1);
        TEXTRANGEW range{{6, 6 + static_cast<LONG>(native.size())}, buffer.data()};
        const auto count = SendMessageW(editor, EM_GETTEXTRANGE, 0, reinterpret_cast<LPARAM>(&range));
        CHARRANGE caret{};
        SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&caret));
        const bool text_ok = count == static_cast<LRESULT>(native.size()) &&
            std::wstring_view(buffer.data(), native.size()) == native &&
            caret.cpMin == 6 + static_cast<LONG>(native.size()) &&
            caret.cpMin == caret.cpMax;
        if (!text_ok) {
            std::cout << "mismatch_case=" << case_index << " actual_codes=";
            for (LRESULT i = 0; i < count; ++i) std::cout << static_cast<unsigned>(buffer[i]) << ',';
            std::cout << "\n";
        }
        passed = text_ok && owo::tsf::verify_richedit_write(
            editor, 6, native, static_cast<LRESULT>(6 + native.size())) && passed;
        passed = SendMessageW(editor, EM_CANUNDO, 0, 0) != 0 && passed;
        passed = SendMessageW(editor, EM_UNDO, 0, 0) != 0 && passed;
        wchar_t restored[32]{};
        GetWindowTextW(editor, restored, 32);
        const bool undo_ok = std::wstring_view(restored) == L"PREFIXtail";
        if (!undo_ok) {
            std::cout << "undo_mismatch_case=" << case_index << " codes=";
            for (const auto c : std::wstring_view(restored)) std::cout << static_cast<unsigned>(c) << ',';
            std::cout << " can_undo_again=" << SendMessageW(editor, EM_CANUNDO, 0, 0) << "\n";
        }
        std::cout << "case=" << case_index++ << " count=" << count << " expected=" << native.size() << " caret=" << caret.cpMin << " text_ok=" << text_ok << " undo_ok=" << undo_ok << "\n";
        passed = undo_ok && passed;
    }
    // Confirm the actual legacy mismatch, independently of the normalizer.
    SetWindowTextW(editor, L"");
    SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(L"Hello\nWorld"));
    wchar_t raw[32]{};
    TEXTRANGEW raw_range{{0, 11}, raw};
    const auto actual = SendMessageW(editor, EM_GETTEXTRANGE, 0, reinterpret_cast<LPARAM>(&raw_range));
    passed = actual == 11 && raw[5] == L'\r' && std::wstring_view(raw, 11) != L"Hello\nWorld" && passed;
    std::cout << "legacy_break_code=" << static_cast<unsigned>(raw[5])
              << " normalized_roundtrip_and_single_undo=" << passed << '\n';
    // Independently show why U+2028 must not reach the production write call.
    SetWindowTextW(editor, L"");
    SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(L"a\t b\u2028c"));
    wchar_t separator[16]{};
    TEXTRANGEW separator_range{{0, 6}, separator};
    const auto separator_count = SendMessageW(editor, EM_GETTEXTRANGE, 0,
        reinterpret_cast<LPARAM>(&separator_range));
    passed = separator_count == 6 && separator[4] == 11 && passed;
    passed = !owo::tsf::richedit_native_text(std::wstring_view(L"a\0b", 3)).has_value() && passed;
    std::cout << "raw_unicode_separator_code=" << static_cast<unsigned>(separator[4])
              << " guarded_cases_pass=" << passed << '\n';
    SetWindowTextW(editor, L"");
    SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(L"a\u2029b"));
    wchar_t paragraph[8]{};
    TEXTRANGEW paragraph_range{{0, 3}, paragraph};
    const auto paragraph_count = SendMessageW(editor, EM_GETTEXTRANGE, 0,
        reinterpret_cast<LPARAM>(&paragraph_range));
    passed = paragraph_count == 3 && paragraph[1] == L'\r' && passed;
    std::cout << "raw_unicode_paragraph_code=" << static_cast<unsigned>(paragraph[1]) << '\n';
    DestroyWindow(editor);
    FreeLibrary(library);
    return passed ? 0 : 1;
}

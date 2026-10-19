#pragma once

#include "owo/agent/agent_protocol.h"

#include <windows.h>
#include <msctf.h>
#include <richedit.h>
#include <optional>
#include <limits>
#include <vector>
#include <string>
#include <string_view>

namespace owo::tsf {

// RICHEDIT50W stores paragraph breaks as CR, while model text commonly uses LF
// or CRLF. Normalize before issuing the single native write, so both the ACP
// range and the exact read-back comparison describe what the control stores.
// No other whitespace or Unicode characters are changed. Known lossy native
// representations (U+2028 -> VT, U+2029 -> CR) and embedded NUL are rejected before mutation.
// Legacy RichEdit can lose undo history for nonspacing/vowel-mark text mixed
// with other scripts. Until that capability is verified, refuse those marks
// rather than normalize the output or mutate text with an unsafe undo record.
[[nodiscard]] inline std::optional<std::wstring> richedit_native_text(std::wstring_view text) {
    if (text.find(L'\u2028') != std::wstring_view::npos ||
        text.find(L'\u2029') != std::wstring_view::npos ||
        text.find(L'\0') != std::wstring_view::npos) return std::nullopt;
    if (text.size() > static_cast<std::size_t>((std::numeric_limits<int>::max)()))
        return std::nullopt;
    if (!text.empty()) {
        std::vector<WORD> types(text.size()), letters(text.size());
        if (!GetStringTypeW(CT_CTYPE3, text.data(), static_cast<int>(text.size()), types.data()) ||
            !GetStringTypeW(CT_CTYPE1, text.data(), static_cast<int>(text.size()), letters.data()))
            return std::nullopt;
        for (std::size_t index = 0; index < text.size(); ++index) {
            // NLS also labels precomposed letters such as U+00E9 NONSPACING.
            // Use C1_ALPHA to preserve precomposed letters in this capability check.
            if ((types[index] & C3_VOWELMARK) != 0 ||
                ((types[index] & C3_NONSPACING) != 0 && (letters[index] & C1_ALPHA) == 0))
                return std::nullopt;
        }
    }
    std::wstring result;
    result.reserve(text.size());
    for (std::size_t index = 0; index < text.size(); ++index) {
        const auto character = text[index];
        if (character == L'\r') {
            result.push_back(L'\r');
            if (index + 1 < text.size() && text[index + 1] == L'\n') ++index;
        } else {
            result.push_back(character == L'\n' ? L'\r' : character);
        }
    }
    return result;
}

// A matching prefix and an older undo record do not prove this write happened.
// Also require the expected document length and collapsed caret at the new end.
[[nodiscard]] inline bool verify_richedit_write(
    HWND window, LONG start, std::wstring_view value, LRESULT expected_length) {
    if (!window || start < 0 || expected_length < 0 ||
        value.size() > static_cast<std::size_t>((std::numeric_limits<LONG>::max)() - start))
        return false;
    GETTEXTLENGTHEX specification{GTL_PRECISE | GTL_NUMCHARS, 1200};
    if (SendMessageW(window, EM_GETTEXTLENGTHEX, reinterpret_cast<WPARAM>(&specification), 0) != expected_length)
        return false;
    CHARRANGE caret{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&caret));
    const auto end = start + static_cast<LONG>(value.size());
    if (caret.cpMin != end || caret.cpMax != end) return false;
    std::vector<wchar_t> after(value.size() + 1);
    TEXTRANGEW written{{start, end}, after.data()};
    const auto count = SendMessageW(window, EM_GETTEXTRANGE, 0, reinterpret_cast<LPARAM>(&written));
    return count == static_cast<LRESULT>(value.size()) &&
        std::wstring_view(after.data(), value.size()) == value &&
        SendMessageW(window, EM_CANUNDO, 0, 0) != 0;
}

enum class NativeWriteResult { not_applicable, rejected, completed, uncertain, unsupported };

// A failed native write never becomes another document write. Only a known
// before-write capability refusal may offer a separate, explicit clipboard action.
[[nodiscard]] inline std::optional<agent::Command> native_writeback_copy_option(
    NativeWriteResult result, const agent::CommandEffect& original, bool negotiated) {
    if (result != NativeWriteResult::unsupported || !negotiated ||
        original.kind == agent::EffectKind::copy_text || original.text.empty() ||
        original.text.find('\0') != std::string::npos) return std::nullopt;
    agent::Command command;
    command.id = "copy-unsupported-result";
    command.label = "复制结果";
    command.description = "复制生成内容到剪贴板";
    command.category = "expression";
    command.effect = agent::CommandEffect{agent::EffectKind::copy_text, original.text, {}, {}};
    return command;
}

// Called only inside the TSF write session after the origin/hash/privacy checks.
// Legacy RichEdit's TSF SetText path does not necessarily populate its undo stack.
[[nodiscard]] NativeWriteResult replace_richedit_selection(
    ITfContext* context, TfEditCookie cookie, ITfRange* expected,
    std::wstring_view original, std::wstring_view replacement);

}  // namespace owo::tsf

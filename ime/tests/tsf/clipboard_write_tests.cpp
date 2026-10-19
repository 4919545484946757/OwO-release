#include "owo/tsf/clipboard_write.h"
#include <iostream>
#include <string>
#include <vector>

struct FakeClipboard {
    int failure{}; // 1=allocate,2=lock,3=open,4=empty,5=publish
    int opens{}, empties{}, publishes{}, frees{}, closes{};
    HWND owner{};
    bool old_contents{true}, system_owns{};
    std::vector<wchar_t> buffer;
    HGLOBAL allocate(SIZE_T size) {
        if (failure == 1) return nullptr;
        buffer.resize(size / sizeof(wchar_t));
        return reinterpret_cast<HGLOBAL>(this);
    }
    void* lock(HGLOBAL) { return failure == 2 ? nullptr : buffer.data(); }
    void unlock(HGLOBAL) {}
    void release(HGLOBAL) { ++frees; }
    bool open(HWND window) { ++opens; owner = window; return failure != 3; }
    bool empty() { ++empties; if (failure == 4) return false; old_contents = false; return true; }
    bool publish(HGLOBAL) {
        ++publishes;
        if (failure == 5 || !owner) return false;
        system_owns = true;
        return true;
    }
    void close() { ++closes; }
};

int main() {
    bool passed = true;
    const auto check = [&](bool ok, const char* name) {
        if (!ok) { std::cerr << "FAIL " << name << '\n'; passed = false; }
    };
    const auto owner = reinterpret_cast<HWND>(static_cast<UINT_PTR>(1));
    const std::wstring text = L"synthetic \u4F60\u597D\n\U0001F600";
    for (int failure = 1; failure <= 5; ++failure) {
        FakeClipboard api; api.failure = failure;
        const auto outcome = owo::tsf::write_clipboard_text(owner, text, api);
        check(outcome == (failure <= 3 ? owo::tsf::ClipboardWriteResult::unchanged
                                      : owo::tsf::ClipboardWriteResult::uncertain),
              "failure stage determines whether unchanged may be promised");
        if (failure <= 3) check(api.old_contents && api.empties == 0, "prepare/open failure preserves clipboard");
        if (failure == 4) check(api.old_contents && api.publishes == 0, "empty failure stops publication");
        if (failure == 5) check(!api.old_contents && !api.system_owns, "publish failure has no atomic rollback guarantee");
        check(api.frees == (failure == 1 ? 0 : 1), "untransferred allocation released once");
        check(api.closes == (failure <= 3 ? 0 : 1), "only successful open is closed");
    }
    FakeClipboard success;
    check(owo::tsf::write_clipboard_text(owner, text, success) == owo::tsf::ClipboardWriteResult::completed, "successful publication");
    check(success.owner == owner && success.empties == 1 && success.publishes == 1, "valid owner and single write");
    check(success.system_owns && success.frees == 0 && success.closes == 1, "ownership transferred without free");
    check(std::wstring(success.buffer.data()) == text && success.buffer.back() == L'\0', "exact Unicode and terminator");
    for (int invalid = 0; invalid < 3; ++invalid) {
        FakeClipboard api;
        const auto content = invalid == 1 ? std::wstring_view(L"") : invalid == 2 ? std::wstring_view(L"a\0b", 3) : std::wstring_view(text);
        check(owo::tsf::write_clipboard_text(invalid == 0 ? nullptr : owner, content, api) ==
            owo::tsf::ClipboardWriteResult::unchanged, "invalid copy refused without clipboard modification");
        check(api.opens == 0 && api.empties == 0 && api.old_contents, "invalid input does not open clipboard");
    }
    std::cout << "clipboard_failure_contract=" << passed << " real_clipboard_access=0\n";
    return passed ? 0 : 1;
}

#pragma once
#include <windows.h>
#include <cstring>
#include <limits>
#include <string_view>

namespace owo::tsf {
enum class ClipboardWriteResult { completed, unchanged, uncertain };
struct NativeClipboardApi {
    HGLOBAL allocate(SIZE_T size) { return GlobalAlloc(GMEM_MOVEABLE, size); }
    void* lock(HGLOBAL memory) { return GlobalLock(memory); }
    void unlock(HGLOBAL memory) { (void)GlobalUnlock(memory); }
    void release(HGLOBAL memory) { (void)GlobalFree(memory); }
    bool open(HWND owner) { return OpenClipboard(owner) != FALSE; }
    bool empty() { return EmptyClipboard() != FALSE; }
    bool publish(HGLOBAL memory) { return SetClipboardData(CF_UNICODETEXT, memory) != nullptr; }
    void close() { (void)CloseClipboard(); }
};

// The API boundary allows ownership and failure-order tests without accessing
// the user's clipboard. Production uses NativeClipboardApi below.
template<class Api>
[[nodiscard]] ClipboardWriteResult write_clipboard_text(HWND owner, std::wstring_view text, Api& api) {
    if (!owner || text.empty() || text.find(L'\0') != std::wstring_view::npos ||
        text.size() > (std::numeric_limits<SIZE_T>::max)() / sizeof(wchar_t) - 1)
        return ClipboardWriteResult::unchanged;
    const auto bytes = (text.size() + 1) * sizeof(wchar_t);
    const auto memory = api.allocate(bytes);
    if (!memory) return ClipboardWriteResult::unchanged;
    void* target = api.lock(memory);
    if (!target) { api.release(memory); return ClipboardWriteResult::unchanged; }
    std::memcpy(target, text.data(), text.size() * sizeof(wchar_t));
    static_cast<wchar_t*>(target)[text.size()] = L'\0';
    api.unlock(memory);
    if (!api.open(owner)) { api.release(memory); return ClipboardWriteResult::unchanged; }
    // Empty/publish is not an atomic OS transaction. Once EmptyClipboard
    // succeeds, a later publish failure cannot guarantee prior data survives.
    const bool written = api.empty() && api.publish(memory);
    api.close();
    if (!written) api.release(memory); // Success transfers ownership to Windows.
    return written ? ClipboardWriteResult::completed : ClipboardWriteResult::uncertain;
}

[[nodiscard]] inline ClipboardWriteResult write_clipboard_text(HWND owner, std::wstring_view text) {
    NativeClipboardApi api;
    return write_clipboard_text(owner, text, api);
}
} // namespace owo::tsf

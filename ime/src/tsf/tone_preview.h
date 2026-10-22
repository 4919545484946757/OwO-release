#pragma once
#include <Windows.h>
#include <string>
#include <string_view>

namespace owo::tsf {
// Modeless, non-activating preview: keyboard input stays in the original TSF host.
class TonePreview final {
public:
    ~TonePreview();
    bool show(HWND owner, POINT anchor, std::wstring_view status, std::wstring_view text);
    void hide() noexcept;
    void destroy() noexcept;
private:
    static LRESULT CALLBACK window_proc(HWND, UINT, WPARAM, LPARAM);
    HWND window_{};
    std::wstring status_, text_;
    int scroll_{}, maximum_scroll_{};
};
}  // namespace owo::tsf

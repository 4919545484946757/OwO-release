#include "tone_preview.h"
#include <algorithm>
#include <windowsx.h>

namespace owo::tsf {
TonePreview::~TonePreview() { destroy(); }
void TonePreview::destroy() noexcept { if (window_) DestroyWindow(window_); window_ = nullptr; }
void TonePreview::hide() noexcept { if (window_) ShowWindow(window_, SW_HIDE); }
bool TonePreview::show(HWND owner, POINT anchor, std::wstring_view status, std::wstring_view text) {
    if (text_ != text) scroll_ = 0;
    status_ = status; text_ = text;
    if (!window_) {
        HMODULE module{};
        if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
            GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            reinterpret_cast<LPCWSTR>(&window_proc), &module)) return false;
        WNDCLASSW cls{};
        cls.lpfnWndProc = window_proc; cls.hInstance = module;
        cls.hCursor = LoadCursorW(nullptr, IDC_ARROW);
        cls.lpszClassName = L"OwO.Tone.Preview.v1";
        if (!RegisterClassW(&cls) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) return false;
        window_ = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            cls.lpszClassName, L"语气草稿预览", WS_POPUP | WS_BORDER | WS_VSCROLL,
            0, 0, 620, 360, owner, nullptr, module, this);
        if (!window_) return false;
    }
    const auto dpi = GetDpiForWindow(window_);
    int width = MulDiv(620, dpi, 96), height = MulDiv(360, dpi, 96);
    MONITORINFO monitor{sizeof(monitor)};
    GetMonitorInfoW(MonitorFromPoint(anchor, MONITOR_DEFAULTTONEAREST), &monitor);
    const auto& area = monitor.rcWork;
    width = std::min(width, static_cast<int>(area.right - area.left));
    height = std::min(height, static_cast<int>(area.bottom - area.top));
    const auto x = std::clamp(anchor.x, area.left, std::max(area.left, area.right - width));
    const auto y = std::clamp(anchor.y, area.top, std::max(area.top, area.bottom - height));
    SetWindowPos(window_, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    InvalidateRect(window_, nullptr, FALSE);
    return true;
}
LRESULT CALLBACK TonePreview::window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* self = reinterpret_cast<TonePreview*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        self = static_cast<TonePreview*>(reinterpret_cast<CREATESTRUCTW*>(lparam)->lpCreateParams);
        SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    }
    if (message == WM_MOUSEACTIVATE) return MA_NOACTIVATE;
    if (self && (message == WM_MOUSEWHEEL || message == WM_VSCROLL)) {
        int movement = 0;
        if (message == WM_MOUSEWHEEL) movement = -GET_WHEEL_DELTA_WPARAM(wparam) / WHEEL_DELTA * 40;
        else switch (LOWORD(wparam)) {
        case SB_LINEUP: movement = -20; break; case SB_LINEDOWN: movement = 20; break;
        case SB_PAGEUP: movement = -120; break; case SB_PAGEDOWN: movement = 120; break;
        case SB_THUMBPOSITION: case SB_THUMBTRACK: {
            SCROLLINFO info{sizeof(info), SIF_TRACKPOS}; GetScrollInfo(window, SB_VERT, &info);
            movement = info.nTrackPos - self->scroll_; break;
        }
        }
        self->scroll_ = std::clamp(self->scroll_ + movement, 0, self->maximum_scroll_);
        SetScrollPos(window, SB_VERT, self->scroll_, TRUE); InvalidateRect(window, nullptr, FALSE); return 0;
    }
    if (message == WM_ERASEBKGND) return 1;
    if (message == WM_PAINT && self) {
        PAINTSTRUCT paint{}; const auto dc = BeginPaint(window, &paint);
        RECT area{}; GetClientRect(window, &area);
        const auto background = CreateSolidBrush(RGB(249, 250, 252));
        FillRect(dc, &area, background); DeleteObject(background);
        const auto dpi = GetDpiForWindow(window);
        const int gap = MulDiv(16, dpi, 96);
        const auto font = CreateFontW(-MulDiv(15, dpi, 96), 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
            DEFAULT_PITCH, L"Microsoft YaHei UI");
        const auto previous = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT); SetTextColor(dc, RGB(35, 39, 46));
        area.left += gap; area.right -= gap; area.top += gap;
        RECT status = area; status.bottom = status.top + MulDiv(44, dpi, 96);
        DrawTextW(dc, self->status_.c_str(), -1, &status, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX);
        area.top = status.bottom + gap; area.bottom -= MulDiv(65, dpi, 96);
        RECT measure = area;
        DrawTextW(dc, self->text_.c_str(), -1, &measure, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_CALCRECT);
        self->maximum_scroll_ = std::max(0L, measure.bottom - measure.top - (area.bottom - area.top));
        self->scroll_ = std::clamp(self->scroll_, 0, self->maximum_scroll_);
        SCROLLINFO scroll{sizeof(scroll), SIF_RANGE | SIF_PAGE | SIF_POS, 0,
            self->maximum_scroll_ + 1, 1, self->scroll_, 0};
        SetScrollInfo(window, SB_VERT, &scroll, TRUE);
        const auto saved = SaveDC(dc);
        IntersectClipRect(dc, area.left, area.top, area.right, area.bottom);
        RECT body = area; body.top -= self->scroll_; body.bottom = body.top + (measure.bottom - measure.top);
        DrawTextW(dc, self->text_.c_str(), -1, &body, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX);
        RestoreDC(dc, saved);
        area.top = area.bottom + gap; area.bottom += MulDiv(60, dpi, 96);
        SetTextColor(dc, RGB(85, 90, 100));
        DrawTextW(dc, L"F6 下一条 · F7 修改/完成修改 · Ctrl+Enter 确认替换选中文字（不会点击发送）\nEsc 放弃当前草稿；切换应用或移动原选区会取消。", -1,
            &area, DT_LEFT | DT_WORDBREAK | DT_NOPREFIX);
        SelectObject(dc, previous); DeleteObject(font); EndPaint(window, &paint); return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}
}  // namespace owo::tsf

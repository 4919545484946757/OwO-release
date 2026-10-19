#include "text_service.h"
#include "owo/tsf/input_method_context.h"
#include "owo/tsf/agent_writeback.h"
#include "owo/tsf/clipboard_write.h"
#include "owo/tsf/diagnostics.h"
#include "owo/tsf/candidate_window_order.h"
#include "owo/tsf/candidate_anchor.h"
#include <functional>
#include "owo/tsf/language_context.h"

#include "owo/tsf/pinyin_cursor.h"

#include "owo/config/config_paths.h"
#include "owo/agent/agent_protocol.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <d2d1.h>
#include <d2d1helper.h>
#include <dwmapi.h>
#include <dwrite.h>
#include <ctfutb.h>
#include <ctffunc.h>
#include <olectl.h>
#include <windowsx.h>
#include <bcrypt.h>

#include <algorithm>
#include <charconv>
#include <cstring>
#include <cmath>
#include <chrono>
#include <iostream>
#include <limits>
#include <memory>
#include <new>
#include <string_view>
#include <utility>
#include <vector>

namespace owo::tsf {
namespace {
LONG object_count = 0;
LONG lock_count = 0;
constexpr wchar_t kMessageClass[] = L"OwO.P1.MessageWindow";
constexpr wchar_t kCandidateShadowClass[] = L"OwO.P1.CandidateShadowWindow";
constexpr wchar_t kCandidateClass[] = L"OwO.P1.CandidateWindow";
constexpr wchar_t kCandidateThemeChangedMessageName[] = L"OwO.CandidateTheme.Refresh.v1";
constexpr UINT kCandidateReady = WM_APP + 1;
constexpr UINT kLanguageShortcutPressed = WM_APP + 2;
constexpr UINT kAltCodeDigitPressed = WM_APP + 3;
constexpr UINT kAltCodeFinished = WM_APP + 4;
constexpr UINT kAgentReady = WM_APP + 5;
constexpr UINT_PTR kPluginInputTimer = 0x4f57;
constexpr UINT_PTR kCandidateWindowOrderTimer = 0x4f58;
constexpr UINT kPluginCommitReady = WM_APP + 6;
constexpr float kCandidateWindowMinWidthDip = 240.0F;
constexpr float kCandidateWindowMaxWidthDip = 1400.0F;
constexpr float kHeaderHeightDip = 40.0F;
constexpr float kCandidateItemHeightDip = 32.0F;
constexpr float kCandidateTextLineHeightDip = 20.0F;
constexpr float kCandidateRowGapDip = 6.0F;
constexpr float kHorizontalPaddingDip = 14.0F;
constexpr float kCandidateGapDip = 6.0F;
constexpr float kCandidateGroupGapDip = 18.0F;
constexpr float kCandidatePillBaseWidthDip = 45.0F;
constexpr float kButtonWidthDip = 30.0F;
constexpr float kExpandButtonWidthDip = 52.0F;
constexpr float kControlGapDip = 6.0F;
constexpr float kCandidateControlGapDip = 10.0F;
constexpr float kUnicodeEntryWidthDip = 30.0F;
constexpr float kUnicodeCategoryHeightDip = 30.0F;
constexpr float kUnicodeCellSizeDip = 42.0F;
constexpr std::size_t kUnicodeItemsPerRow = 12;
constexpr std::size_t kUnicodeVisibleRows = 6;
constexpr std::size_t kExpandedVisibleRows = 5;
constexpr std::size_t kMaximumPinyinInputLength = 256;
constexpr ULONGLONG kShortcutConfigRefreshIntervalMs = 500;
constexpr auto kCandidateRequestBaseTimeout = std::chrono::milliseconds(900);
constexpr auto kCandidateRequestMaximumTimeout = std::chrono::milliseconds(2500);
constexpr auto kCandidateRefinementDebounce = std::chrono::milliseconds(4);
constexpr auto kFeedbackRequestTimeout = std::chrono::milliseconds(100);
thread_local TextService* keyboard_hook_service = nullptr;

std::string utf8_from_wide(std::wstring_view input);

std::wstring compact_feedback_input(const std::wstring_view input) {
    std::wstring result;
    result.reserve(input.size());
    std::copy_if(input.begin(), input.end(), std::back_inserter(result),
                 [](const wchar_t value) { return value != L'\''; });
    return result;
}

std::string new_agent_session_id() {
    GUID value{};
    if (FAILED(CoCreateGuid(&value))) return {};
    constexpr char digits[] = "0123456789abcdef";
    const auto* bytes = reinterpret_cast<const unsigned char*>(&value);
    std::string result;
    result.reserve(sizeof(value) * 2);
    for (std::size_t index = 0; index < sizeof(value); ++index) {
        result.push_back(digits[bytes[index] >> 4]);
        result.push_back(digits[bytes[index] & 15]);
    }
    return result;
}

std::wstring agent_status_label(const agent::Status status) {
    switch (status) {
    case agent::Status::connecting: return L"连接中";
    case agent::Status::agent_mode: return L"Agent 模式";
    case agent::Status::submitting: return L"发送中";
    case agent::Status::thinking: return L"思考中";
    case agent::Status::waiting_for_user: return L"等待输入";
    case agent::Status::executing: return L"执行中";
    case agent::Status::waiting_for_confirmation: return L"等待确认";
    case agent::Status::cancelling: return L"取消中";
    case agent::Status::cancelled: return L"已取消";
    case agent::Status::completed: return L"已完成";
    case agent::Status::disconnected: return L"未连接";
    case agent::Status::timeout: return L"超时";
    case agent::Status::error: return L"错误";
    }
    return L"错误";
}

// `scope_sensitive` comes from the TSF InputScope inspection of the focused context; any
// result other than `allowed` (including unknown) must be passed as true.
agent::ApplicationContext current_agent_application(const std::string_view context_id,
                                                    const bool scope_sensitive) {
    agent::ApplicationContext result;
    result.context_id.assign(context_id);
    result.sensitive_input = scope_sensitive;
    HWND window = GetForegroundWindow();
    if (window == nullptr) {
        result.sensitive_input = true;
        return result;
    }
    DWORD process_id = 0;
    const DWORD thread_id = GetWindowThreadProcessId(window, &process_id);
    result.process_id = process_id;
    result.thread_id = thread_id;
    wchar_t class_name[129]{};
    const int class_length = GetClassNameW(window, class_name,
                                           static_cast<int>(std::size(class_name)));
    if (class_length > 0)
        result.window_class = utf8_from_wide(
            std::wstring_view(class_name, static_cast<std::size_t>(class_length)));
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, process_id);
    if (process != nullptr) {
        wchar_t path[32768]{};
        DWORD size = static_cast<DWORD>(std::size(path));
        if (QueryFullProcessImageNameW(process, 0, path, &size) && size != 0)
            result.executable = utf8_from_wide(
                std::filesystem::path(std::wstring_view(path, size)).filename().wstring());
        CloseHandle(process);
    }
    GUITHREADINFO gui{sizeof(gui)};
    if (thread_id != 0 && GetGUIThreadInfo(thread_id, &gui) && gui.hwndFocus != nullptr) {
        wchar_t focus_class[129]{};
        const int focus_length = GetClassNameW(
            gui.hwndFocus, focus_class, static_cast<int>(std::size(focus_class)));
        if (focus_length > 0)
            result.focus_window_class = utf8_from_wide(
                std::wstring_view(focus_class, static_cast<std::size_t>(focus_length)));
        const auto style = static_cast<DWORD>(GetWindowLongPtrW(gui.hwndFocus, GWL_STYLE));
        if (_wcsicmp(focus_class, L"Edit") == 0 && (style & ES_PASSWORD) != 0)
            result.sensitive_input = true;
    }
    if (result.sensitive_input) {
        result.executable.clear();
        result.window_class.clear();
        result.focus_window_class.clear();
    }
    return result;
}

std::optional<std::wstring> alt_code_symbol(const std::wstring_view digits) {
    if (digits.empty()) return std::nullopt;
    std::uint64_t value = 0;
    for (const wchar_t digit : digits) {
        if (digit < L'0' || digit > L'9' ||
            value > (std::numeric_limits<std::uint64_t>::max)() / 10)
            return std::nullopt;
        value = value * 10 + static_cast<unsigned>(digit - L'0');
    }
    if (value <= 255) {
        if (value < 32 || value == 127) return std::nullopt;
        const char byte = static_cast<char>(static_cast<unsigned char>(value));
        const UINT code_page = digits.size() > 1 && digits.front() == L'0'
                                   ? CP_ACP : CP_OEMCP;
        wchar_t output[2]{};
        const int length = MultiByteToWideChar(
            code_page, MB_ERR_INVALID_CHARS, &byte, 1, output,
            static_cast<int>(std::size(output)));
        if (length != 1 || output[0] < L' ') return std::nullopt;
        return std::wstring(1, output[0]);
    }
    if (value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff) ||
        (value >= 0xfdd0 && value <= 0xfdef) || (value & 0xffff) >= 0xfffe)
        return std::nullopt;
    if (value <= 0xffff) return std::wstring(1, static_cast<wchar_t>(value));
    value -= 0x10000;
    std::wstring result;
    result.push_back(static_cast<wchar_t>(0xd800 + (value >> 10)));
    result.push_back(static_cast<wchar_t>(0xdc00 + (value & 0x3ff)));
    return result;
}

bool system_uses_light_theme() noexcept {
    DWORD value = 1;
    DWORD bytes = sizeof(value);
    const LSTATUS status = RegGetValueW(
        HKEY_CURRENT_USER,
        L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
        L"SystemUsesLightTheme", RRF_RT_REG_DWORD, nullptr, &value, &bytes);
    return status != ERROR_SUCCESS || value != 0;
}

std::wstring_view candidate_status_text(const bool pending, const bool failed,
                                        const std::wstring_view failure_detail) noexcept {
    if (pending) return L"正在查找…";
    if (!failed) return L"无候选";
    return failure_detail.empty() ? L"候选服务暂不可用" : failure_detail;
}

template <typename Interface>
void release_interface(Interface*& value) noexcept {
    if (value == nullptr) return;
    value->Release();
    value = nullptr;
}

float measure_text_width(IDWriteFactory* factory,
                         IDWriteTextFormat* format,
                         const std::wstring_view text) {
    if (factory == nullptr || format == nullptr || text.empty()) return 0.0F;
    IDWriteTextLayout* layout = nullptr;
    const HRESULT result = factory->CreateTextLayout(
        text.data(), static_cast<UINT32>(text.size()), format, 4096.0F,
        4096.0F, &layout);
    if (FAILED(result)) return 0.0F;
    DWRITE_TEXT_METRICS metrics{};
    const HRESULT metrics_result = layout->GetMetrics(&metrics);
    layout->Release();
    return SUCCEEDED(metrics_result) ? metrics.widthIncludingTrailingWhitespace : 0.0F;
}

std::size_t wrapped_line_count(const std::wstring_view text,
                               const std::size_t wrap_length) noexcept {
    if (text.empty()) return 1;
    return (text.size() + wrap_length - 1) / wrap_length;
}

float wrapped_candidate_height(const std::wstring_view text,
                               const std::size_t wrap_length) noexcept {
    return kCandidateItemHeightDip +
           static_cast<float>(wrapped_line_count(text, wrap_length) - 1) *
               kCandidateTextLineHeightDip;
}

std::wstring wrapped_candidate_text(const std::wstring_view text,
                                    const std::size_t wrap_length) {
    if (text.size() <= wrap_length) return std::wstring(text);
    std::wstring wrapped;
    wrapped.reserve(text.size() + text.size() / wrap_length);
    for (std::size_t offset = 0; offset < text.size(); offset += wrap_length) {
        if (!wrapped.empty()) wrapped.push_back(L'\n');
        wrapped.append(text.substr(offset, std::min(wrap_length, text.size() - offset)));
    }
    return wrapped;
}

int dips_to_pixels(const float dips, const UINT dpi) noexcept {
    return static_cast<int>(std::ceil(dips * static_cast<float>(dpi) / 96.0F));
}

RECT dip_rect_to_pixels(const D2D1_RECT_F bounds, const UINT dpi) noexcept {
    const float scale = static_cast<float>(dpi) / 96.0F;
    return RECT{static_cast<LONG>(std::floor(bounds.left * scale)),
                static_cast<LONG>(std::floor(bounds.top * scale)),
                static_cast<LONG>(std::ceil(bounds.right * scale)),
                static_cast<LONG>(std::ceil(bounds.bottom * scale))};
}

float rounded_rect_coverage(const float x, const float y, const float left,
                            const float top, const float right, const float bottom,
                            const float radius) noexcept {
    const float center_x = (left + right) * 0.5F;
    const float center_y = (top + bottom) * 0.5F;
    const float half_width = std::max((right - left) * 0.5F - radius, 0.0F);
    const float half_height = std::max((bottom - top) * 0.5F - radius, 0.0F);
    const float qx = std::abs(x - center_x) - half_width;
    const float qy = std::abs(y - center_y) - half_height;
    const float distance = std::hypot(std::max(qx, 0.0F), std::max(qy, 0.0F)) +
                           std::min(std::max(qx, qy), 0.0F) - radius;
    return std::clamp(0.5F - distance, 0.0F, 1.0F);
}

std::string utf8_from_wide(const std::wstring_view input) {
    if (input.empty()) return {};
    const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, input.data(),
                                         static_cast<int>(input.size()), nullptr, 0,
                                         nullptr, nullptr);
    if (size <= 0) return {};
    std::string result(static_cast<std::size_t>(size), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, input.data(),
                            static_cast<int>(input.size()), result.data(), size,
                            nullptr, nullptr) != size) {
        return {};
    }
    return result;
}

std::wstring wide_from_utf8(const std::string_view input) {
    if (input.empty()) return {};
    const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, input.data(),
                                         static_cast<int>(input.size()), nullptr, 0);
    if (size <= 0) return {};
    std::wstring result(static_cast<std::size_t>(size), L'\0');
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, input.data(),
                            static_cast<int>(input.size()), result.data(), size) != size) {
        return {};
    }
    return result;
}

std::uint64_t current_unix_time_ms() {
    return static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::milliseconds>(
            std::chrono::system_clock::now().time_since_epoch()).count());
}

std::wstring unicode_scalar(const std::uint32_t codepoint) {
    if (codepoint <= 0xffff) return std::wstring(1, static_cast<wchar_t>(codepoint));
    const auto adjusted = codepoint - 0x10000;
    return {static_cast<wchar_t>(0xd800 + (adjusted >> 10)),
            static_cast<wchar_t>(0xdc00 + (adjusted & 0x3ff))};
}

std::wstring unicode_code_label(const std::uint32_t codepoint) {
    wchar_t buffer[12]{};
    swprintf_s(buffer, L"U+%04X", codepoint);
    return buffer;
}

bool key_down(const int key) noexcept {
    return (GetKeyState(key) & 0x8000) != 0;
}

bool is_control_key(const WPARAM key) noexcept {
    return key == VK_CONTROL || key == VK_LCONTROL || key == VK_RCONTROL;
}

bool is_alt_key(const WPARAM key) noexcept {
    return key == VK_MENU || key == VK_LMENU || key == VK_RMENU;
}

bool is_shift_key(const WPARAM key) noexcept {
    return key == VK_SHIFT || key == VK_LSHIFT || key == VK_RSHIFT;
}

std::string primary_shortcut_name(const WPARAM key) {
    if ((key >= 'A' && key <= 'Z') || (key >= '0' && key <= '9'))
        return std::string(1, static_cast<char>(key));
    if (key >= VK_F1 && key <= VK_F24)
        return "F" + std::to_string(key - VK_F1 + 1);
    switch (key) {
        case VK_SPACE: return "Space";
        case VK_RETURN: return "Enter";
        case VK_TAB: return "Tab";
        case VK_ESCAPE: return "Escape";
        case VK_BACK: return "Backspace";
        case VK_DELETE: return "Delete";
        case VK_INSERT: return "Insert";
        case VK_HOME: return "Home";
        case VK_END: return "End";
        case VK_PRIOR: return "PageUp";
        case VK_NEXT: return "PageDown";
        case VK_LEFT: return "Left";
        case VK_RIGHT: return "Right";
        case VK_UP: return "Up";
        case VK_DOWN: return "Down";
        case VK_OEM_4: return "[";
        case VK_OEM_6: return "]";
        case VK_OEM_MINUS: return "Minus";
        case VK_OEM_PLUS: return "Plus";
        case VK_OEM_COMMA: return "Comma";
        case VK_OEM_PERIOD: return "Period";
        case VK_OEM_2: return "Slash";
        case VK_OEM_1: return "Semicolon";
        case VK_OEM_7: return "Quote";
        case VK_OEM_3: return "Backtick";
        default: return {};
    }
}

std::string shortcut_for_key_event(const WPARAM key) {
    const bool control = is_control_key(key) || key_down(VK_CONTROL);
    const bool alt = is_alt_key(key) || key_down(VK_MENU);
    const bool shift = is_shift_key(key) || key_down(VK_SHIFT);
    std::string result;
    const auto append = [&result](const std::string_view token) {
        if (!result.empty()) result.push_back('+');
        result += token;
    };
    if (control) append("Ctrl");
    if (alt) append("Alt");
    if (shift) append("Shift");
    if (!is_control_key(key) && !is_alt_key(key) && !is_shift_key(key)) {
        const auto primary = primary_shortcut_name(key);
        if (primary.empty()) return {};
        append(primary);
    }
    return result;
}

UINT primary_shortcut_virtual_key(const std::string_view name) {
    if (name.size() == 1 && ((name.front() >= 'A' && name.front() <= 'Z') ||
                            (name.front() >= '0' && name.front() <= '9')))
        return static_cast<UINT>(name.front());
    if (name.size() >= 2 && name.front() == 'F') {
        unsigned number = 0;
        const auto parsed = std::from_chars(name.data() + 1, name.data() + name.size(), number);
        if (parsed.ec == std::errc{} && parsed.ptr == name.data() + name.size() &&
            number >= 1 && number <= 24)
            return VK_F1 + number - 1;
    }
    constexpr std::pair<std::string_view, UINT> named[]{
        {"Space", VK_SPACE}, {"Enter", VK_RETURN}, {"Tab", VK_TAB},
        {"Escape", VK_ESCAPE}, {"Backspace", VK_BACK}, {"Delete", VK_DELETE},
        {"Insert", VK_INSERT}, {"Home", VK_HOME}, {"End", VK_END},
        {"PageUp", VK_PRIOR}, {"PageDown", VK_NEXT}, {"Left", VK_LEFT},
        {"Right", VK_RIGHT}, {"Up", VK_UP}, {"Down", VK_DOWN},
        {"[", VK_OEM_4}, {"]", VK_OEM_6}, {"Minus", VK_OEM_MINUS},
        {"Plus", VK_OEM_PLUS}, {"Comma", VK_OEM_COMMA},
        {"Period", VK_OEM_PERIOD}, {"Slash", VK_OEM_2},
        {"Semicolon", VK_OEM_1}, {"Quote", VK_OEM_7},
        {"Backtick", VK_OEM_3}};
    const auto found = std::find_if(std::begin(named), std::end(named),
                                    [name](const auto& value) { return value.first == name; });
    return found == std::end(named) ? 0U : found->second;
}

bool preserved_shortcut(const std::string_view shortcut, TF_PRESERVEDKEY& result) {
    result = {};
    std::size_t offset = 0;
    while (offset < shortcut.size()) {
        const auto separator = shortcut.find('+', offset);
        const auto token = shortcut.substr(offset, separator == std::string_view::npos
                                                       ? shortcut.size() - offset
                                                       : separator - offset);
        if (token == "Ctrl") result.uModifiers |= TF_MOD_CONTROL;
        else if (token == "Alt") result.uModifiers |= TF_MOD_ALT;
        else if (token == "Shift") result.uModifiers |= TF_MOD_SHIFT;
        else if (result.uVKey == 0) result.uVKey = primary_shortcut_virtual_key(token);
        else return false;
        if (separator == std::string_view::npos) break;
        offset = separator + 1;
    }
    return result.uVKey != 0;
}

std::wstring case_preserving_segmented_input(const std::wstring_view input,
                                              const std::wstring_view segmented) {
    std::wstring result;
    result.reserve(segmented.size());
    std::size_t source = 0;
    for (const wchar_t character : segmented) {
        if (character == L'\'') {
            result.push_back(character);
            continue;
        }
        while (source < input.size() && input[source] == L'\'') ++source;
        if (source >= input.size()) return std::wstring(segmented);
        result.push_back(input[source++]);
    }
    return result;
}

bool command_modifier_down() noexcept {
    return key_down(VK_CONTROL) || key_down(VK_MENU) || key_down(VK_LWIN) ||
           key_down(VK_RWIN);
}

std::chrono::milliseconds candidate_request_timeout(const std::size_t input_length) {
    const auto length_allowance = std::chrono::milliseconds(input_length * 5);
    return std::min(kCandidateRequestBaseTimeout + length_allowance,
                    kCandidateRequestMaximumTimeout);
}

class SchemeEditSession final : public ITfEditSession {
public:
    SchemeEditSession(ITfContext* context, std::function<HRESULT(TfEditCookie)> action,
                      IUnknown* owner = nullptr)
        : context_(context), action_(std::move(action)), owner_(owner) {
        context_->AddRef(); if (owner_) owner_->AddRef();
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (!object) return E_POINTER;
        *object = nullptr;
        if (iid != IID_IUnknown && iid != IID_ITfEditSession) return E_NOINTERFACE;
        *object = static_cast<ITfEditSession*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return static_cast<ULONG>(InterlockedIncrement(&refs_)); }
    ULONG STDMETHODCALLTYPE Release() override {
        const auto count = InterlockedDecrement(&refs_); if (!count) delete this;
        return static_cast<ULONG>(count);
    }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        try { return action_(cookie); } catch (...) { return E_FAIL; }
    }
private:
    ~SchemeEditSession() { context_->Release(); if (owner_) owner_->Release(); }
    LONG refs_{1}; ITfContext* context_; std::function<HRESULT(TfEditCookie)> action_; IUnknown* owner_;
};

class CommitEditSession final : public ITfEditSession {
public:
    CommitEditSession(ITfContext* context, std::wstring text)
        : context_(context), text_(std::move(text)) {
        context_->AddRef();
    }

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (object == nullptr) return E_POINTER;
        *object = nullptr;
        if (iid == IID_IUnknown || iid == IID_ITfEditSession) {
            *object = static_cast<ITfEditSession*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override {
        return static_cast<ULONG>(InterlockedIncrement(&references_));
    }
    ULONG STDMETHODCALLTYPE Release() override {
        const auto remaining = InterlockedDecrement(&references_);
        if (remaining == 0) delete this;
        return static_cast<ULONG>(remaining);
    }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        TF_SELECTION selection{};
        ULONG fetched = 0;
        HRESULT result = context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1,
                                                &selection, &fetched);
        if (FAILED(result) || fetched != 1 || selection.range == nullptr) return result;
        result = selection.range->SetText(cookie, 0, text_.data(),
                                          static_cast<LONG>(text_.size()));
        if (SUCCEEDED(result)) {
            selection.range->Collapse(cookie, TF_ANCHOR_END);
            result = context_->SetSelection(cookie, 1, &selection);
        }
        selection.range->Release();
        return result;
    }

private:
    ~CommitEditSession() { context_->Release(); }
    LONG references_{1};
    ITfContext* context_;
    std::wstring text_;
};

class CaretEditSession final : public ITfEditSession {
public:
    CaretEditSession(ITfContext* context, POINT* anchor, bool* valid)
        : context_(context), anchor_(anchor), valid_(valid) {
        context_->AddRef();
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (object == nullptr) return E_POINTER;
        *object = nullptr;
        if (iid == IID_IUnknown || iid == IID_ITfEditSession) {
            *object = static_cast<ITfEditSession*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override {
        return static_cast<ULONG>(InterlockedIncrement(&references_));
    }
    ULONG STDMETHODCALLTYPE Release() override {
        const auto remaining = InterlockedDecrement(&references_);
        if (remaining == 0) delete this;
        return static_cast<ULONG>(remaining);
    }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        *valid_ = false;
        TF_SELECTION selection{};
        ULONG fetched = 0;
        HRESULT result = context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1,
                                                &selection, &fetched);
        trace_tsf_event("caret.selection", result, static_cast<HRESULT>(fetched));
        if (FAILED(result) || fetched != 1 || selection.range == nullptr) return result;
        ITfContextView* view = nullptr;
        result = context_->GetActiveView(&view);
        trace_tsf_event("caret.view", result);
        if (SUCCEEDED(result)) {
            RECT bounds{};
            BOOL clipped = FALSE;
            result = view->GetTextExt(cookie, selection.range, &bounds, &clipped);
            trace_tsf_event("caret.extent", result, clipped);
            if (SUCCEEDED(result)) {
                trace_tsf_event("caret.left_top", bounds.left, bounds.top);
                trace_tsf_event("caret.right_bottom", bounds.right, bounds.bottom);
                if (candidate_text_extent_usable(bounds)) {
                    anchor_->x = bounds.left;
                    anchor_->y = bounds.bottom;
                    *valid_ = true;
                } else {
                    trace_tsf_event("caret.invalid_extent", E_FAIL);
                }
            }
            view->Release();
        }
        selection.range->Release();
        return result;
    }

private:
    ~CaretEditSession() { context_->Release(); }
    LONG references_{1};
    ITfContext* context_;
    POINT* anchor_;
    bool* valid_;
};

class DocumentCaretEditSession final : public ITfEditSession {
public:
    DocumentCaretEditSession(ITfContext* context, WPARAM key, bool select)
        : context_(context), key_(key), select_(select) { context_->AddRef(); }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (!object) return E_POINTER; *object = nullptr;
        if (iid != IID_IUnknown && iid != IID_ITfEditSession) return E_NOINTERFACE;
        *object = static_cast<ITfEditSession*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&references_); }
    ULONG STDMETHODCALLTYPE Release() override {
        const auto left = InterlockedDecrement(&references_); if (!left) delete this;
        return left;
    }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        TF_SELECTION selection{}; ULONG fetched = 0;
        HRESULT result = context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1,
                                                &selection, &fetched);
        if (FAILED(result) || fetched != 1 || !selection.range) return result;
        ITfRange* target = nullptr;
        result = selection.range->Clone(&target);
        if (SUCCEEDED(result) && (key_ == VK_LEFT || key_ == VK_RIGHT)) {
            if (!select_) target->Collapse(cookie, key_ == VK_LEFT ? TF_ANCHOR_START
                                                                   : TF_ANCHOR_END);
            LONG moved = 0;
            result = key_ == VK_LEFT
                ? target->ShiftStart(cookie, -1, &moved, nullptr)
                : target->ShiftEnd(cookie, 1, &moved, nullptr);
            if (SUCCEEDED(result) && !select_)
                result = target->Collapse(cookie, key_ == VK_LEFT ? TF_ANCHOR_START
                                                                  : TF_ANCHOR_END);
        } else if (SUCCEEDED(result)) {
            ITfContextView* view = nullptr;
            result = context_->GetActiveView(&view);
            RECT bounds{}; BOOL clipped = FALSE;
            if (SUCCEEDED(result))
                result = view->GetTextExt(cookie, selection.range, &bounds, &clipped);
            if (SUCCEEDED(result)) {
                const LONG height = std::max<LONG>(16, bounds.bottom - bounds.top);
                POINT point{bounds.left, key_ == VK_UP ? bounds.top - height
                                                       : bounds.bottom + height};
                ITfRange* vertical = nullptr;
                result = view->GetRangeFromPoint(cookie, &point, GXFPF_NEAREST, &vertical);
                if (SUCCEEDED(result) && vertical) {
                    vertical->Collapse(cookie, TF_ANCHOR_START);
                    if (select_)
                        result = target->ShiftEndToRange(cookie, vertical, TF_ANCHOR_START);
                    else { target->Release(); target = vertical; vertical = nullptr; }
                    if (vertical) vertical->Release();
                }
            }
            if (view) view->Release();
        }
        if (SUCCEEDED(result) && target) {
            selection.range->Release(); selection.range = target; target = nullptr;
            selection.style.ase = TF_AE_NONE; selection.style.fInterimChar = FALSE;
            result = context_->SetSelection(cookie, 1, &selection);
        }
        if (target) target->Release();
        selection.range->Release();
        return result;
    }
private:
    ~DocumentCaretEditSession() { context_->Release(); }
    LONG references_{1}; ITfContext* context_; WPARAM key_; bool select_;
};
}  // namespace

class LanguageBarItem final : public ITfLangBarItemButton, public ITfSource {
public:
    explicit LanguageBarItem(TextService* owner) : owner_(owner) {
        info_.clsidService = kTextServiceClsid;
        // Windows 8 and later ignore input-mode items that use another GUID.
        info_.guidItem = GUID_LBI_INPUTMODE;
        info_.dwStyle = TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_TEXTCOLORICON;
        info_.ulSort = 0;
    }

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (object == nullptr) return E_POINTER;
        *object = nullptr;
        if (iid == IID_IUnknown || iid == IID_ITfLangBarItem ||
            iid == IID_ITfLangBarItemButton) {
            *object = static_cast<ITfLangBarItemButton*>(this);
        } else if (iid == IID_ITfSource) {
            *object = static_cast<ITfSource*>(this);
        } else {
            return E_NOINTERFACE;
        }
        AddRef();
        return S_OK;
    }

    ULONG STDMETHODCALLTYPE AddRef() override {
        return static_cast<ULONG>(InterlockedIncrement(&references_));
    }
    ULONG STDMETHODCALLTYPE Release() override {
        const auto remaining = InterlockedDecrement(&references_);
        if (remaining == 0) delete this;
        return static_cast<ULONG>(remaining);
    }

    HRESULT STDMETHODCALLTYPE GetInfo(TF_LANGBARITEMINFO* info) override {
        if (info == nullptr) return E_INVALIDARG;
        *info = info_;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetStatus(DWORD* status) override {
        if (status == nullptr) return E_INVALIDARG;
        *status = 0;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE Show(BOOL) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE GetTooltipString(BSTR* text) override {
        if (text == nullptr) return E_INVALIDARG;
        *text = SysAllocString(owner_ != nullptr && owner_->chinese_mode_
                                   ? L"OwO 中文输入"
                                   : L"OwO 英文输入");
        return *text == nullptr ? E_OUTOFMEMORY : S_OK;
    }

    HRESULT STDMETHODCALLTYPE OnClick(TfLBIClick, POINT, const RECT*) override {
        return owner_ == nullptr ? E_UNEXPECTED : owner_->toggle_language_mode_from_window();
    }
    HRESULT STDMETHODCALLTYPE InitMenu(ITfMenu*) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE OnMenuSelect(UINT) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE GetIcon(HICON* icon) override {
        if (icon == nullptr) return E_INVALIDARG;
        *icon = create_mode_icon(owner_ != nullptr && owner_->chinese_mode_ ? L"中" : L"英");
        return *icon == nullptr ? E_OUTOFMEMORY : S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetText(BSTR* text) override {
        if (text == nullptr) return E_INVALIDARG;
        *text = SysAllocString(owner_ != nullptr && owner_->chinese_mode_ ? L"中" : L"英");
        return *text == nullptr ? E_OUTOFMEMORY : S_OK;
    }

    HRESULT STDMETHODCALLTYPE AdviseSink(REFIID iid, IUnknown* unknown,
                                         DWORD* cookie) override {
        if (cookie == nullptr || unknown == nullptr) return E_INVALIDARG;
        *cookie = TF_INVALID_COOKIE;
        if (iid != IID_ITfLangBarItemSink) return CONNECT_E_CANNOTCONNECT;
        if (sink_ != nullptr) return CONNECT_E_ADVISELIMIT;
        const HRESULT result = unknown->QueryInterface(IID_PPV_ARGS(&sink_));
        if (FAILED(result)) return result;
        *cookie = 1;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE UnadviseSink(DWORD cookie) override {
        if (cookie != 1 || sink_ == nullptr) return CONNECT_E_NOCONNECTION;
        sink_->Release();
        sink_ = nullptr;
        return S_OK;
    }

    void notify_mode_changed() noexcept {
        if (sink_ != nullptr) sink_->OnUpdate(TF_LBI_ICON | TF_LBI_TEXT | TF_LBI_TOOLTIP);
    }

private:
    ~LanguageBarItem() {
        if (sink_ != nullptr) sink_->Release();
    }

    static HICON create_mode_icon(const wchar_t* label) {
        constexpr int size = 20;
        BITMAPV5HEADER header{};
        header.bV5Size = sizeof(header);
        header.bV5Width = size;
        header.bV5Height = -size;
        header.bV5Planes = 1;
        header.bV5BitCount = 32;
        header.bV5Compression = BI_BITFIELDS;
        header.bV5RedMask = 0x00ff0000;
        header.bV5GreenMask = 0x0000ff00;
        header.bV5BlueMask = 0x000000ff;
        header.bV5AlphaMask = 0xff000000;
        void* raw_pixels = nullptr;
        HDC screen = GetDC(nullptr);
        HBITMAP color = CreateDIBSection(screen, reinterpret_cast<BITMAPINFO*>(&header),
                                         DIB_RGB_COLORS, &raw_pixels, nullptr, 0);
        ReleaseDC(nullptr, screen);
        if (color == nullptr || raw_pixels == nullptr) return nullptr;
        auto* pixels = static_cast<std::uint32_t*>(raw_pixels);
        std::fill_n(pixels, size * size, 0x00000001U);

        HDC memory = CreateCompatibleDC(nullptr);
        HGDIOBJ old_bitmap = SelectObject(memory, color);
        HFONT font = CreateFontW(-17, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
                                 DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                 ANTIALIASED_QUALITY, DEFAULT_PITCH, L"Microsoft YaHei UI");
        HGDIOBJ old_font = SelectObject(memory, font);
        SetBkMode(memory, TRANSPARENT);
        const COLORREF foreground = system_uses_light_theme()
                                        ? RGB(0, 0, 0)
                                        : RGB(255, 255, 255);
        SetTextColor(memory, foreground);
        RECT bounds{0, 0, size, size};
        DrawTextW(memory, label, 1, &bounds,
                  DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(memory, old_font);
        SelectObject(memory, old_bitmap);
        DeleteObject(font);
        DeleteDC(memory);

        for (int index = 0; index < size * size; ++index) {
            if (pixels[index] == 0x00000001U) pixels[index] = 0;
            else pixels[index] |= 0xff000000U;
        }
        HBITMAP mask = CreateBitmap(size, size, 1, 1, nullptr);
        ICONINFO icon_info{};
        icon_info.fIcon = TRUE;
        icon_info.hbmColor = color;
        icon_info.hbmMask = mask;
        HICON icon = CreateIconIndirect(&icon_info);
        DeleteObject(mask);
        DeleteObject(color);
        return icon;
    }

    LONG references_{1};
    TextService* owner_{};
    TF_LANGBARITEMINFO info_{};
    ITfLangBarItemSink* sink_{};
};

TextService::TextService() noexcept {
    next_request_id_ = (static_cast<std::uint64_t>(GetCurrentProcessId()) << 32U) |
                       (GetTickCount64() & 0xffffffffULL);
    rotate_agent_context();
    InterlockedIncrement(&object_count);
}

TextService::~TextService() {
    Deactivate();
    InterlockedDecrement(&object_count);
}

HRESULT TextService::QueryInterface(REFIID iid, void** object) {
    if (object == nullptr) return E_POINTER;
    *object = nullptr;
    if (iid == IID_IUnknown || iid == IID_ITfTextInputProcessor ||
        iid == IID_ITfTextInputProcessorEx) {
        *object = static_cast<ITfTextInputProcessorEx*>(this);
    } else if (iid == IID_ITfKeyEventSink) {
        *object = static_cast<ITfKeyEventSink*>(this);
    } else if (iid == IID_ITfThreadMgrEventSink) {
        *object = static_cast<ITfThreadMgrEventSink*>(this);
    } else if (iid == IID_ITfThreadFocusSink) {
        *object = static_cast<ITfThreadFocusSink*>(this);
    } else if (iid == IID_ITfTextEditSink) {
        *object = static_cast<ITfTextEditSink*>(this);
    } else if (iid == IID_ITfCompartmentEventSink) {
        *object = static_cast<ITfCompartmentEventSink*>(this);
    } else {
        return E_NOINTERFACE;
    }
    AddRef();
    return S_OK;
}

ULONG TextService::AddRef() {
    return static_cast<ULONG>(InterlockedIncrement(&references_));
}

ULONG TextService::Release() {
    const auto remaining = InterlockedDecrement(&references_);
    if (remaining == 0) delete this;
    return static_cast<ULONG>(remaining);
}

HRESULT TextService::Activate(ITfThreadMgr* thread_manager, const TfClientId client_id) {
    return ActivateEx(thread_manager, client_id, 0);
}

HRESULT TextService::ActivateEx(ITfThreadMgr* thread_manager,
                                const TfClientId client_id,
                                DWORD) {
    if (thread_manager == nullptr) return E_INVALIDARG;
    if (thread_manager_ != nullptr) return E_UNEXPECTED;
    thread_manager_ = thread_manager;
    thread_manager_->AddRef();
    client_id_ = client_id;

    ITfKeystrokeMgr* keystroke_manager = nullptr;
    HRESULT result = thread_manager_->QueryInterface(IID_PPV_ARGS(&keystroke_manager));
    if (SUCCEEDED(result)) {
        result = keystroke_manager->AdviseKeyEventSink(client_id_, this, TRUE);
        keystroke_manager->Release();
    }
    if (FAILED(result)) {
        Deactivate();
        return result;
    }
    ITfSource* source = nullptr;
    result = thread_manager_->QueryInterface(IID_PPV_ARGS(&source));
    if (SUCCEEDED(result)) {
        result = source->AdviseSink(IID_ITfThreadMgrEventSink,
                                    static_cast<ITfThreadMgrEventSink*>(this),
                                    &thread_manager_event_sink_cookie_);
        if (SUCCEEDED(result)) {
            result = source->AdviseSink(IID_ITfThreadFocusSink,
                                        static_cast<ITfThreadFocusSink*>(this),
                                        &thread_focus_sink_cookie_);
        }
        source->Release();
    }
    if (FAILED(result)) {
        Deactivate();
        return result;
    }
    result = initialize_windows();
    if (FAILED(result)) {
        Deactivate();
        return result;
    }
    refresh_shortcut_config(true);
    initialize_language_compartment();
    initialize_language_bar();
    keyboard_hook_service = this;
    keyboard_hook_ = SetWindowsHookExW(WH_KEYBOARD, keyboard_hook_proc, nullptr,
                                       GetCurrentThreadId());
    worker_ = std::jthread([this](const std::stop_token token) { worker_loop(token); });
    return S_OK;
}

HRESULT TextService::Deactivate() {
    if (worker_.joinable()) {
        worker_.request_stop();
        request_ready_.notify_all();
        worker_.join();
    }
    if (candidate_cancellation_event_ != nullptr) {
        SetEvent(candidate_cancellation_event_);
        CloseHandle(candidate_cancellation_event_);
        candidate_cancellation_event_ = nullptr;
    }
    for (const auto event : retired_cancellation_events_) CloseHandle(event);
    retired_cancellation_events_.clear();
    {
        std::lock_guard lock(request_mutex_);
        pending_request_.reset();
        feedback_requests_.clear();
    }
    clear_composition();
    if (keyboard_hook_ != nullptr) {
        UnhookWindowsHookEx(keyboard_hook_);
        keyboard_hook_ = nullptr;
    }
    if (keyboard_hook_service == this) keyboard_hook_service = nullptr;
    language_space_key_suppressed_ = false;
    alt_code_numpad_down_mask_ = 0;
    destroy_windows();
    if (thread_manager_ != nullptr) {
        clear_preserved_language_key();
        uninitialize_language_bar();
        uninitialize_language_compartment();
        ITfSource* source = nullptr;
        if (SUCCEEDED(thread_manager_->QueryInterface(IID_PPV_ARGS(&source)))) {
            if (thread_focus_sink_cookie_ != TF_INVALID_COOKIE)
                source->UnadviseSink(thread_focus_sink_cookie_);
            if (thread_manager_event_sink_cookie_ != TF_INVALID_COOKIE)
                source->UnadviseSink(thread_manager_event_sink_cookie_);
            source->Release();
        }
        thread_focus_sink_cookie_ = TF_INVALID_COOKIE;
        thread_manager_event_sink_cookie_ = TF_INVALID_COOKIE;
        ITfKeystrokeMgr* keystroke_manager = nullptr;
        if (SUCCEEDED(thread_manager_->QueryInterface(IID_PPV_ARGS(&keystroke_manager)))) {
            keystroke_manager->UnadviseKeyEventSink(client_id_);
            keystroke_manager->Release();
        }
        thread_manager_->Release();
        thread_manager_ = nullptr;
    }
    client_id_ = TF_CLIENTID_NULL;
    return S_OK;
}

HRESULT TextService::OnSetFocus(const BOOL foreground) {
    const bool had_focus = foreground_focus_;
    foreground_focus_ = foreground != FALSE;
    if (!foreground_focus_) {
        if (is_agent_mode()) cancel_agent_session();
        else commit_pending_on_focus_loss();
        clear_composition();
        recent_language_context_.clear();
        recent_language_input_.clear();
        recent_agent_context_.clear();
        agent_context_id_.clear();
    } else {
        if (!had_focus) rotate_agent_context();
        if (has_composition()) update_candidate_window();
    }
    return S_OK;
}

HRESULT TextService::OnInitDocumentMgr(ITfDocumentMgr*) {
    return S_OK;
}

HRESULT TextService::OnUninitDocumentMgr(ITfDocumentMgr*) {
    if (is_agent_mode()) cancel_agent_session();
    reset_agent_origin();
    // Retire experimental results even if a host omits a separate focus-loss callback.
    if (plugin_scheme_selected()) clear_composition();
    else reset_plugin_input();
    return S_OK;
}

HRESULT TextService::OnSetFocus(ITfDocumentMgr* document_manager,
                                ITfDocumentMgr* previous_document_manager) {
    if (document_manager != previous_document_manager) {
        const bool agent_mode = is_agent_mode();
        if (agent_mode) cancel_agent_session();
        refresh_shortcut_config();
        if (!plugin_scheme_selected() && !agent_mode && shortcut_config_.focus_loss_commit_enabled && has_composition() &&
            previous_document_manager != nullptr) {
            ITfContext* previous_context = nullptr;
            if (SUCCEEDED(previous_document_manager->GetTop(&previous_context)) &&
                previous_context != nullptr) {
                if (staged_candidates_.empty()) commit_raw_input(previous_context);
                else commit_pending_composition(previous_context);
                previous_context->Release();
            }
        }
        clear_composition();
    }
    if (document_manager != previous_document_manager) {
        recent_language_context_.clear();
        recent_language_input_.clear();
        rotate_agent_context();
    }
    foreground_focus_ = document_manager != nullptr;
    return S_OK;
}

HRESULT TextService::OnPushContext(ITfContext*) {
    if (is_agent_mode()) cancel_agent_session();
    clear_composition();
    recent_language_context_.clear();
    recent_language_input_.clear();
    rotate_agent_context();
    return S_OK;
}

HRESULT TextService::OnPopContext(ITfContext* context) {
    const bool agent_mode = is_agent_mode();
    if (agent_mode) cancel_agent_session();
    refresh_shortcut_config();
    if (!plugin_scheme_selected() && !agent_mode && shortcut_config_.focus_loss_commit_enabled && has_composition() &&
        context != nullptr) {
        if (staged_candidates_.empty()) commit_raw_input(context);
        else commit_pending_composition(context);
    }
    clear_composition();
    recent_language_context_.clear();
    recent_language_input_.clear();
    rotate_agent_context();
    return S_OK;
}

HRESULT TextService::OnSetThreadFocus() {
    foreground_focus_ = true;
    if (agent_context_id_.empty()) rotate_agent_context();
    if (has_composition()) update_candidate_window();
    return S_OK;
}

HRESULT TextService::OnKillThreadFocus() {
    if (is_agent_mode()) cancel_agent_session();
    else commit_pending_on_focus_loss();
    foreground_focus_ = false;
    clear_composition();
    recent_language_context_.clear();
    recent_language_input_.clear();
    recent_agent_context_.clear();
    agent_context_id_.clear();
    return S_OK;
}

HRESULT TextService::OnChange(REFGUID guid) {
    if (!IsEqualGUID(guid, GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)) return S_OK;
    const auto mode = system_language_mode();
    if (!mode.has_value() || *mode == chinese_mode_) return S_OK;

    ITfDocumentMgr* document_manager = nullptr;
    ITfContext* context = nullptr;
    if (thread_manager_ != nullptr &&
        SUCCEEDED(thread_manager_->GetFocus(&document_manager)) &&
        document_manager != nullptr) {
        document_manager->GetTop(&context);
        document_manager->Release();
    }
    const HRESULT result = apply_language_mode(*mode, context);
    if (context != nullptr) context->Release();
    return result;
}

bool TextService::should_eat_key(const WPARAM key) const noexcept {
    if (unicode_palette_visible_) {
        if (key == VK_ESCAPE || key == VK_PRIOR || key == VK_NEXT ||
            key == VK_OEM_4 || key == VK_OEM_6) return true;
        if (shortcut_config_.previous_page_shortcut_enabled &&
            shortcut_matches(shortcut_config_.previous_page_shortcuts, key)) return true;
        if (shortcut_config_.next_page_shortcut_enabled &&
            shortcut_matches(shortcut_config_.next_page_shortcuts, key)) return true;
    }
    if (shortcut_config_.input_scheme == "full_pinyin" &&
        shortcut_config_.correction_shortcut_enabled &&
        shortcut_matches(shortcut_config_.correction_shortcuts, key)) return true;
    if (shortcut_config_.language_shortcut_enabled &&
        shortcut_matches(shortcut_config_.language_shortcuts, key)) return true;
    if (shortcut_config_.punctuation_shortcut_enabled &&
        shortcut_matches(shortcut_config_.punctuation_shortcuts, key)) return true;
    if (has_composition() && shortcut_config_.raw_input_shortcut_enabled &&
        shortcut_matches(shortcut_config_.raw_input_shortcuts, key)) return true;
    if (has_composition() && shortcut_config_.text_navigation_shortcut_enabled &&
        shortcut_matches(shortcut_config_.text_navigation_shortcuts, key)) return true;
    if (has_composition() && shortcut_config_.cursor_left_shortcut_enabled &&
        shortcut_matches(shortcut_config_.cursor_left_shortcuts, key)) return true;
    if (has_composition() && shortcut_config_.cursor_right_shortcut_enabled &&
        shortcut_matches(shortcut_config_.cursor_right_shortcuts, key)) return true;
    if (!input_buffer_.empty() && shortcut_config_.previous_page_shortcut_enabled &&
        shortcut_matches(shortcut_config_.previous_page_shortcuts, key)) return true;
    if (!input_buffer_.empty() && shortcut_config_.next_page_shortcut_enabled &&
        shortcut_matches(shortcut_config_.next_page_shortcuts, key)) return true;
    if (!chinese_mode_) return false;
    if (is_agent_mode() && (key == VK_RETURN || key == VK_SPACE)) return true;
    if (key >= 'A' && key <= 'Z') return !command_modifier_down();
    if (chinese_punctuation_enabled_ && !command_modifier_down() &&
        const_cast<TextService*>(this)->chinese_punctuation_for_key(key).has_value())
        return true;
    if (!has_composition()) return false;
    if (key == VK_OEM_7) return GetKeyState(VK_SHIFT) >= 0 && !command_modifier_down();
    if (key == VK_BACK || key == VK_ESCAPE) return true;
    if ((key == VK_UP || key == VK_DOWN) && candidates_expanded_ &&
        !key_down(VK_SHIFT) && !command_modifier_down()) return true;
    if (key == VK_NEXT || key == VK_OEM_6)
        return candidates_expanded_ ||
               candidate_request_pending_ || has_more_candidates_;
    if (key == VK_PRIOR || key == VK_OEM_4)
        return candidates_expanded_ ||
               candidate_page_ > 0;
    if (key == VK_SPACE)
        return is_agent_mode() || candidate_request_pending_ || !candidates_.empty() ||
               (!staged_candidates_.empty() && input_buffer_.empty());
    return key >= '1' && key <= '9' &&
           (candidate_request_pending_ ||
            static_cast<std::size_t>(key - '1') < candidates_.size());
}

HRESULT TextService::OnTestKeyDown(ITfContext* context, WPARAM key, LPARAM flags, BOOL* eaten) {
    if (eaten == nullptr) return E_POINTER;
    refresh_shortcut_config();
    *eaten = (plugin_scheme_selected() ? plugin_should_eat_key(context, key, flags)
                                     : should_eat_key(key)) ? TRUE : FALSE;
    return S_OK;
}

HRESULT TextService::OnKeyDown(ITfContext* context, WPARAM key, LPARAM flags, BOOL* eaten) {
    if (eaten == nullptr) return E_POINTER;
    refresh_shortcut_config();
    if (plugin_scheme_selected()) return handle_plugin_key(context, key, flags, eaten);
    *eaten = should_eat_key(key) ? TRUE : FALSE;
    if (!*eaten) return S_OK;
    if (context != nullptr) update_candidate_anchor(context);
    refresh_agent_context_safety(context);

    if (unicode_palette_visible_) {
        const bool previous = key == VK_PRIOR || key == VK_OEM_4 ||
            (shortcut_config_.previous_page_shortcut_enabled &&
             shortcut_matches(shortcut_config_.previous_page_shortcuts, key));
        const bool next = key == VK_NEXT || key == VK_OEM_6 ||
            (shortcut_config_.next_page_shortcut_enabled &&
             shortcut_matches(shortcut_config_.next_page_shortcuts, key));
        if (key == VK_ESCAPE) {
            toggle_unicode_palette();
            return S_OK;
        }
        if (previous || next) {
            change_unicode_palette_page(previous ? -1 : 1);
            return S_OK;
        }
    }

    if (shortcut_config_.input_scheme == "full_pinyin" &&
        shortcut_config_.correction_shortcut_enabled &&
        shortcut_matches(shortcut_config_.correction_shortcuts, key)) {
        correction_enabled_ = !correction_enabled_;
        if (!input_buffer_.empty()) {
            // A mode change is intentional, so immediately discard any
            // corrected preview from the previous mode. The response will
            // replace this raw snapshot with the strict source segmentation.
            segmented_input_ = input_buffer_;
            candidates_.clear();
            candidate_consumed_.clear();
            candidate_groups_.clear();
            candidate_failure_detail_.clear();
            candidate_page_ = 0;
            has_more_candidates_ = false;
            candidates_expanded_ = false;
            ++context_generation_;
            queue_candidate_request();
        }
    } else if (shortcut_config_.language_shortcut_enabled &&
               shortcut_matches(shortcut_config_.language_shortcuts, key)) {
        return toggle_language_mode(context);
    } else if (shortcut_config_.punctuation_shortcut_enabled &&
               shortcut_matches(shortcut_config_.punctuation_shortcuts, key)) {
        chinese_punctuation_enabled_ = !chinese_punctuation_enabled_;
        return S_OK;
    } else if (is_agent_mode() &&
               (key == VK_RETURN || key == VK_SPACE)) {
        if (input_buffer_ == L"v" && staged_candidates_.empty() &&
            !agent_response_visible_ && !agent_request_submitted_) {
            cancel_agent_session();
            if (context == nullptr) return E_UNEXPECTED;
            const HRESULT committed = commit_text(context, L"v");
            if (FAILED(committed)) return committed;
            clear_composition();
            return S_OK;
        }
        if (agent_response_visible_ && !agent_commands_.empty() && key == VK_SPACE) {
            if (agent_commands_.front().high_risk) {
                agent_status_ = agent::Status::error;
                agent_message_ = L"高风险指令必须在第三方 Agent 内单独确认";
                update_candidate_window();
                return S_OK;
            }
            select_agent_command(0);
        } else if (input_buffer_.size() > 1 || !staged_candidates_.empty()) {
            queue_agent_action(agent::Action::submit);
        }
        return S_OK;
    } else if (has_composition() && shortcut_config_.raw_input_shortcut_enabled &&
               shortcut_matches(shortcut_config_.raw_input_shortcuts, key)) {
        if (context != nullptr) return staged_candidates_.empty()
            ? commit_raw_input(context) : commit_pending_composition(context);
        clear_composition();
    } else if (has_composition() &&
               shortcut_config_.text_navigation_shortcut_enabled &&
               shortcut_matches(shortcut_config_.text_navigation_shortcuts, key)) {
        return move_document_caret(
            context, key, shortcut_config_.text_navigation_select_enabled);
    } else if (has_composition() && shortcut_config_.cursor_left_shortcut_enabled &&
               shortcut_matches(shortcut_config_.cursor_left_shortcuts, key)) {
        move_pinyin_cursor(-1);
    } else if (has_composition() && shortcut_config_.cursor_right_shortcut_enabled &&
               shortcut_matches(shortcut_config_.cursor_right_shortcuts, key)) {
        move_pinyin_cursor(1);
    } else if (!input_buffer_.empty() && shortcut_config_.previous_page_shortcut_enabled &&
               shortcut_matches(shortcut_config_.previous_page_shortcuts, key)) {
        move_candidate_page_from_shortcut(-1);
    } else if (!input_buffer_.empty() && shortcut_config_.next_page_shortcut_enabled &&
               shortcut_matches(shortcut_config_.next_page_shortcuts, key)) {
        move_candidate_page_from_shortcut(1);
    } else if (key >= 'A' && key <= 'Z') {
        if (input_buffer_.size() >= kMaximumPinyinInputLength) return S_OK;
        if (is_agent_mode() && agent_response_visible_) {
            if (!agent_message_.empty()) {
                input_buffer_ = L"v";
                input_cursor_ = 1;
            }
            segmented_input_.clear();
            candidates_.clear();
            candidate_consumed_.clear();
            candidate_groups_.clear();
            agent_commands_.clear();
            agent_message_.clear();
            agent_executing_command_.clear();
            agent_response_visible_ = false;
        }
        if (staged_cursor_ < staged_candidates_.size())
            restore_staged_suffix(staged_cursor_);
        const bool shift = key_down(VK_SHIFT);
        const bool caps_lock = (GetKeyState(VK_CAPITAL) & 1) != 0;
        const wchar_t base = shift != caps_lock ? L'A' : L'a';
        const wchar_t character = static_cast<wchar_t>(base + (key - 'A'));
        if (character == L'v' && input_buffer_.empty() && staged_candidates_.empty())
            (void)capture_agent_origin(context);
        insert_at_pinyin_cursor(input_buffer_, segmented_input_, input_cursor_,
                                character);
        // Keep the last confirmed segmentation visible until the new parse and
        // candidates arrive together. Editing that snapshot in place prevents
        // the preview from briefly falling back to completely unsegmented input.
        candidate_page_ = 0;
        has_more_candidates_ = false;
        candidates_expanded_ = false;
        ++context_generation_;
        queue_candidate_request();
    } else if (key == VK_OEM_7 && !key_down(VK_SHIFT) && !input_buffer_.empty() &&
               input_buffer_.size() < kMaximumPinyinInputLength) {
        if (staged_cursor_ < staged_candidates_.size())
            restore_staged_suffix(staged_cursor_);
        input_cursor_ = std::min(input_cursor_, input_buffer_.size());
        const bool left_separator = input_cursor_ > 0 &&
                                    input_buffer_[input_cursor_ - 1] == L'\'';
        const bool right_separator = input_cursor_ < input_buffer_.size() &&
                                     input_buffer_[input_cursor_] == L'\'';
        if (left_separator || right_separator) return S_OK;
        insert_at_pinyin_cursor(input_buffer_, segmented_input_, input_cursor_,
                                L'\'');
        candidate_page_ = 0;
        has_more_candidates_ = false;
        candidates_expanded_ = false;
        ++context_generation_;
        queue_candidate_request();
    } else if (chinese_punctuation_enabled_ && !command_modifier_down() &&
               chinese_punctuation_for_key(key).has_value()) {
        const auto punctuation = chinese_punctuation_for_key(key, true);
        if (!punctuation.has_value()) return E_UNEXPECTED;
        if (context == nullptr) return E_UNEXPECTED;
        if (!input_buffer_.empty() && candidate_request_pending_) {
            defer_punctuation(*punctuation, context);
            return S_OK;
        }
        if (!input_buffer_.empty() && !candidate_request_pending_ && !candidates_.empty())
            return commit_candidate(context, 0, *punctuation);
        if (!input_buffer_.empty()) {
            const std::wstring raw = input_buffer_ + *punctuation;
            clear_composition();
            return commit_text(context, raw);
        }
        return commit_text(context, *punctuation);
    } else if (key == VK_BACK) {
        const bool was_agent_mode = is_agent_mode();
        // Local opt-in diagnostics: state bits/counts only, never input text.
        trace_tsf_event("edit.backspace.state",
            static_cast<HRESULT>(static_cast<unsigned>(was_agent_mode) | (agent_request_pending_ << 1) |
                (agent_response_visible_ << 2) | (agent_request_submitted_ << 3) |
                (!agent_session_id_.empty() << 4)),
            static_cast<HRESULT>(input_buffer_.size()));
        trace_tsf_event("edit.backspace.cursor", static_cast<HRESULT>(input_cursor_),
            static_cast<HRESULT>(staged_cursor_));
        trace_tsf_event("edit.backspace.staged", static_cast<HRESULT>(staged_candidates_.size()),
            static_cast<HRESULT>(agent_last_submitted_pinyin_.size()));
        if (was_agent_mode && (agent_request_pending_ || agent_response_visible_) &&
            !agent_last_submitted_pinyin_.empty()) {
            const std::wstring restored_pinyin =
                std::exchange(agent_last_submitted_pinyin_, {});
            // This is a local edit rollback, not a session cancellation. Keep
            // the session id so the restored request can be edited and sent
            // again as the next turn. Queuing the candidate request below
            // interrupts local polling of the old response without emitting
            // cancel(session_id) to the third-party Agent.
            input_buffer_ = L"v" + restored_pinyin;
            input_cursor_ = input_buffer_.size();
            staged_candidates_.clear();
            staged_cursor_ = 0;
            segmented_input_.clear();
            candidates_.clear();
            candidate_consumed_.clear();
            candidate_groups_.clear();
            candidate_failure_detail_.clear();
            candidate_page_ = 0;
            has_more_candidates_ = false;
            candidate_request_pending_ = false;
            candidates_expanded_ = false;
            hovered_target_.reset();
            pressed_target_.reset();
            agent_status_ = agent::Status::agent_mode;
            agent_message_.clear();
            agent_executing_command_.clear();
            agent_response_visible_ = false;
            ++context_generation_;
            queue_candidate_request();
            return S_OK;
        }
        std::optional<std::size_t> restored_staged_cursor;
        if (was_agent_mode && !staged_candidates_.empty() &&
            staged_cursor_ == staged_candidates_.size() && input_cursor_ == 1) {
            // In Agent mode input cursor 1 is the boundary immediately after
            // the reserved `v` and all staged candidates. Backspace at that
            // visual boundary targets the last staged candidate, not `v`.
            restored_staged_cursor = staged_candidates_.size() - 1;
        } else if (input_cursor_ == 0 && staged_cursor_ > 0) {
            restored_staged_cursor = staged_cursor_ - 1;
        }
        if (restored_staged_cursor.has_value()) {
            const auto restored_cursor = *restored_staged_cursor;
            const auto restored_length = staged_candidates_[restored_cursor].source_input.size();
            const bool agent_prefix = !input_buffer_.empty() &&
                                      input_buffer_.front() == L'v';
            restore_staged_suffix(restored_cursor);
            input_cursor_ = std::min(
                restored_length + static_cast<std::size_t>(agent_prefix),
                input_buffer_.size());
            candidate_page_ = 0;
            has_more_candidates_ = false;
            candidates_expanded_ = false;
            ++context_generation_;
            queue_candidate_request();
            return S_OK;
        }
        if (!erase_before_pinyin_cursor(
                input_buffer_, segmented_input_, input_cursor_)) return S_OK;
        candidate_page_ = 0;
        has_more_candidates_ = false;
        candidates_expanded_ = false;
        ++context_generation_;
        if (was_agent_mode && (input_buffer_.empty() || input_buffer_.front() != L'v'))
            cancel_agent_session();
        if (input_buffer_.empty()) clear_composition();
        else queue_candidate_request();
    } else if (key == VK_ESCAPE) {
        cancel_agent_session();
        clear_composition();
    } else if (candidates_expanded_ &&
               ((key == VK_DOWN && !key_down(VK_SHIFT) && !command_modifier_down()) ||
                key == VK_NEXT || key == VK_OEM_6)) {
        scroll_expanded_candidates(1);
    } else if (candidates_expanded_ &&
               ((key == VK_UP && !key_down(VK_SHIFT) && !command_modifier_down()) ||
                key == VK_PRIOR || key == VK_OEM_4)) {
        scroll_expanded_candidates(-1);
    } else if (!candidates_expanded_ &&
               (key == VK_NEXT || key == VK_OEM_6) &&
               (candidate_request_pending_ || has_more_candidates_)) {
        change_candidate_page(1);
    } else if (!candidates_expanded_ &&
               (key == VK_PRIOR || key == VK_OEM_4) && candidate_page_ > 0) {
        change_candidate_page(-1);
    } else if (key == VK_SPACE && input_buffer_.empty() &&
               !staged_candidates_.empty()) {
        if (context != nullptr) return commit_pending_composition(context);
        clear_composition();
    } else if (candidate_request_pending_) {
        const std::size_t index = key == VK_SPACE ? 0 : static_cast<std::size_t>(key - '1');
        defer_candidate_selection(index, context);
    } else if (context != nullptr && !candidates_.empty()) {
        const std::size_t index = key == VK_SPACE ? 0 : static_cast<std::size_t>(key - '1');
        if (index < candidates_.size()) {
            if (is_agent_mode() && index < agent_commands_.size()) {
                if (agent_commands_[index].high_risk) {
                    agent_status_ = agent::Status::error;
                    agent_message_ = L"高风险指令必须在第三方 Agent 内单独确认";
                    agent_response_visible_ = true;
                    update_candidate_window();
                    return S_OK;
                }
                select_agent_command(index);
                return S_OK;
            }
            if (is_agent_mode()) return stage_agent_candidate(context, index);
            return commit_candidate(context, index);
        }
    }
    return S_OK;
}

HRESULT TextService::OnTestKeyUp(ITfContext*, const WPARAM key, LPARAM, BOOL* eaten) {
    if (eaten == nullptr) return E_POINTER;
    *eaten = is_alt_key(key) && alt_code_preview_active_ ? TRUE : FALSE;
    return S_OK;
}

HRESULT TextService::OnKeyUp(ITfContext* context, const WPARAM key, LPARAM, BOOL* eaten) {
    if (eaten == nullptr) return E_POINTER;
    if (!is_alt_key(key) || !alt_code_preview_active_) {
        *eaten = FALSE;
        return S_OK;
    }
    *eaten = TRUE;
    const auto symbol = alt_code_symbol(alt_code_digits_);
    finish_alt_code_preview();
    if (!symbol.has_value() || context == nullptr) return S_OK;
    return commit_text(context, *symbol);
}

HRESULT TextService::OnPreservedKey(ITfContext* context, REFGUID guid, BOOL* eaten) {
    if (eaten == nullptr) return E_POINTER;
    refresh_shortcut_config();
    if (!IsEqualGUID(guid, kLanguageModePreservedKeyGuid) ||
        !shortcut_config_.language_shortcut_enabled) {
        *eaten = FALSE;
        return S_OK;
    }
    *eaten = TRUE;
    return toggle_language_mode(context);
}

HRESULT TextService::initialize_windows() {
    HMODULE module = nullptr;
    if (!GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            reinterpret_cast<LPCWSTR>(&TextService::window_proc), &module)) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    WNDCLASSW message_class{};
    message_class.lpfnWndProc = window_proc;
    message_class.hInstance = module;
    message_class.lpszClassName = kMessageClass;
    if (RegisterClassW(&message_class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    WNDCLASSW candidate_class = message_class;
    candidate_class.style = 0;
    candidate_class.hbrBackground = nullptr;
    candidate_class.lpszClassName = kCandidateClass;
    if (RegisterClassW(&candidate_class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    WNDCLASSW shadow_class = candidate_class;
    shadow_class.lpszClassName = kCandidateShadowClass;
    if (RegisterClassW(&shadow_class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    message_window_ = CreateWindowExW(0, kMessageClass, L"", 0, 0, 0, 0, 0,
                                      HWND_MESSAGE, nullptr, message_class.hInstance, this);
    if (message_window_ == nullptr) return HRESULT_FROM_WIN32(GetLastError());
    candidate_shadow_window_ = CreateWindowExW(
        WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_LAYERED |
            WS_EX_TRANSPARENT,
        kCandidateShadowClass, L"", WS_POPUP, 0, 0, 1, 1, nullptr, nullptr,
        message_class.hInstance, this);
    if (candidate_shadow_window_ == nullptr) return HRESULT_FROM_WIN32(GetLastError());
    candidate_window_ = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE |
                                            WS_EX_TOPMOST | WS_EX_LAYERED,
                                        kCandidateClass, L"", WS_POPUP,
                                        0, 0, 780, 88, nullptr, nullptr,
                                        message_class.hInstance, this);
    if (candidate_window_ == nullptr) return HRESULT_FROM_WIN32(GetLastError());
    const HRESULT result = initialize_rendering();
    if (FAILED(result)) return result;
    apply_candidate_window_effects();
    return S_OK;
}

void TextService::destroy_windows() noexcept {
    discard_rendering();
    if (candidate_window_ != nullptr) DestroyWindow(candidate_window_);
    if (candidate_shadow_window_ != nullptr) DestroyWindow(candidate_shadow_window_);
    if (message_window_ != nullptr) DestroyWindow(message_window_);
    candidate_window_ = nullptr;
    candidate_shadow_window_ = nullptr;
    message_window_ = nullptr;
}

HRESULT TextService::initialize_rendering() {
    HRESULT result = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &d2d_factory_);
    if (FAILED(result)) return result;
    result = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, __uuidof(IDWriteFactory),
                                 reinterpret_cast<IUnknown**>(&dwrite_factory_));
    if (FAILED(result)) {
        discard_rendering();
        return result;
    }

    result = dwrite_factory_->CreateTextFormat(
        L"Microsoft YaHei UI", nullptr, DWRITE_FONT_WEIGHT_SEMI_BOLD,
        DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 15.0F, L"zh-CN",
        &input_text_format_);
    if (SUCCEEDED(result)) {
        result = dwrite_factory_->CreateTextFormat(
            L"Microsoft YaHei UI", nullptr, DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 15.0F, L"zh-CN",
            &candidate_text_format_);
    }
    if (SUCCEEDED(result)) {
        result = dwrite_factory_->CreateTextFormat(
            L"Microsoft YaHei UI", nullptr, DWRITE_FONT_WEIGHT_SEMI_BOLD,
            DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 11.0F, L"zh-CN",
            &label_text_format_);
    }
    if (FAILED(result)) {
        discard_rendering();
        return result;
    }

    input_text_format_->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
    input_text_format_->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    candidate_text_format_->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
    candidate_text_format_->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    label_text_format_->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
    label_text_format_->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    label_text_format_->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
    return S_OK;
}

HRESULT TextService::ensure_device_resources(SIZE surface_size) {
    surface_size.cx = std::max<LONG>(surface_size.cx, 1);
    surface_size.cy = std::max<LONG>(surface_size.cy, 1);
    if (render_target_ != nullptr &&
        layered_surface_size_.cx == surface_size.cx &&
        layered_surface_size_.cy == surface_size.cy) return S_OK;
    if (render_target_ != nullptr) discard_device_resources();
    if (candidate_window_ == nullptr || d2d_factory_ == nullptr) return E_UNEXPECTED;

    const UINT dpi = std::max(GetDpiForWindow(candidate_window_), 96U);
    const D2D1_SIZE_U pixel_size = D2D1::SizeU(
        static_cast<UINT32>(surface_size.cx),
        static_cast<UINT32>(surface_size.cy));
    BITMAPINFO bitmap_info{};
    bitmap_info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bitmap_info.bmiHeader.biWidth = static_cast<LONG>(pixel_size.width);
    bitmap_info.bmiHeader.biHeight = -static_cast<LONG>(pixel_size.height);
    bitmap_info.bmiHeader.biPlanes = 1;
    bitmap_info.bmiHeader.biBitCount = 32;
    bitmap_info.bmiHeader.biCompression = BI_RGB;
    void* pixels = nullptr;
    layered_surface_dc_ = CreateCompatibleDC(nullptr);
    if (layered_surface_dc_ == nullptr) return HRESULT_FROM_WIN32(GetLastError());
    layered_surface_bitmap_ = CreateDIBSection(
        layered_surface_dc_, &bitmap_info, DIB_RGB_COLORS, &pixels, nullptr, 0);
    if (layered_surface_bitmap_ == nullptr || pixels == nullptr) {
        const auto error = GetLastError();
        discard_device_resources();
        return HRESULT_FROM_WIN32(error == ERROR_SUCCESS ? ERROR_NOT_ENOUGH_MEMORY : error);
    }
    layered_surface_pixels_ = pixels;
    layered_surface_previous_bitmap_ =
        SelectObject(layered_surface_dc_, layered_surface_bitmap_);
    if (layered_surface_previous_bitmap_ == nullptr ||
        layered_surface_previous_bitmap_ == HGDI_ERROR) {
        const auto error = GetLastError();
        layered_surface_previous_bitmap_ = nullptr;
        discard_device_resources();
        return HRESULT_FROM_WIN32(error == ERROR_SUCCESS ? ERROR_INVALID_HANDLE : error);
    }
    layered_surface_size_ = {static_cast<LONG>(pixel_size.width),
                             static_cast<LONG>(pixel_size.height)};
    const auto properties = D2D1::RenderTargetProperties(
        D2D1_RENDER_TARGET_TYPE_DEFAULT,
        D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM,
                          D2D1_ALPHA_MODE_PREMULTIPLIED),
        static_cast<float>(dpi), static_cast<float>(dpi));
    HRESULT result = d2d_factory_->CreateDCRenderTarget(&properties, &render_target_);
    if (FAILED(result)) {
        discard_device_resources();
        return result;
    }
    const RECT surface_bounds{0, 0, layered_surface_size_.cx, layered_surface_size_.cy};
    result = render_target_->BindDC(layered_surface_dc_, &surface_bounds);
    if (FAILED(result)) {
        discard_device_resources();
        return result;
    }

    const auto create_brush = [this](const D2D1_COLOR_F color,
                                     ID2D1SolidColorBrush** brush) {
        return render_target_->CreateSolidColorBrush(color, brush);
    };
    const auto theme_color = [](const plugin::CandidateThemeColor color) {
        return D2D1::ColorF(color.rgb, color.opacity);
    };
    result = create_brush(theme_color(candidate_theme_.background), &background_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color(candidate_theme_.border), &border_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color(candidate_theme_.divider), &divider_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color(candidate_theme_.candidate_text), &text_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color({candidate_theme_.candidate_text.rgb,
            candidate_theme_.candidate_text.opacity * 0.64F}), &secondary_text_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color(candidate_theme_.pinyin_text), &accent_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color(candidate_theme_.candidate_block), &highlight_brush_);
    if (SUCCEEDED(result))
        result = create_brush(theme_color(candidate_theme_.candidate_number),
                              &strict_accent_brush_);
    if (FAILED(result)) discard_device_resources();
    return result;
}

void TextService::discard_device_resources() noexcept {
    release_interface(strict_accent_brush_);
    release_interface(highlight_brush_);
    release_interface(accent_brush_);
    release_interface(secondary_text_brush_);
    release_interface(text_brush_);
    release_interface(divider_brush_);
    release_interface(border_brush_);
    release_interface(background_brush_);
    release_interface(render_target_);
    if (layered_surface_dc_ != nullptr && layered_surface_previous_bitmap_ != nullptr)
        SelectObject(layered_surface_dc_, layered_surface_previous_bitmap_);
    layered_surface_previous_bitmap_ = nullptr;
    if (layered_surface_bitmap_ != nullptr) DeleteObject(layered_surface_bitmap_);
    layered_surface_bitmap_ = nullptr;
    layered_surface_pixels_ = nullptr;
    if (layered_surface_dc_ != nullptr) DeleteDC(layered_surface_dc_);
    layered_surface_dc_ = nullptr;
    layered_surface_size_ = {};
}

void TextService::discard_rendering() noexcept {
    discard_device_resources();
    release_interface(label_text_format_);
    release_interface(candidate_text_format_);
    release_interface(input_text_format_);
    release_interface(dwrite_factory_);
    release_interface(d2d_factory_);
}

void TextService::apply_candidate_window_effects() noexcept {
    if (candidate_window_ == nullptr) return;
    // The bitmap owns the rounded, per-pixel alpha surface. Asking DWM for a
    // client backdrop behind a layered HWND introduces an opaque black plate.
    const DWM_WINDOW_CORNER_PREFERENCE corners = DWMWCP_DONOTROUND;
    DwmSetWindowAttribute(candidate_window_, DWMWA_WINDOW_CORNER_PREFERENCE,
                          &corners, sizeof(corners));
    const COLORREF border_color = DWMWA_COLOR_NONE;
    DwmSetWindowAttribute(candidate_window_, DWMWA_BORDER_COLOR,
                          &border_color, sizeof(border_color));
}

SIZE TextService::desired_candidate_window_size() const {
    const UINT dpi = candidate_window_ == nullptr
                         ? 96U
                         : std::max(GetDpiForWindow(candidate_window_), 96U);
    const auto page_size = std::max<std::size_t>(
        1, static_cast<std::size_t>(candidate_page_size_));
    const auto wrap_length = std::max<std::size_t>(
        1, static_cast<std::size_t>(shortcut_config_.candidate_wrap_length));
    if (alt_code_preview_active_) {
        const auto preview_page_size = std::max<std::size_t>(
            2, static_cast<std::size_t>(shortcut_config_.alt_code_preview_page_size));
        const bool has_strict = !alt_code_preview_items_.empty() &&
                                alt_code_preview_items_.front().strict;
        const auto tail_capacity = has_strict ? preview_page_size - 1
                                              : preview_page_size;
        const auto tail_begin = (has_strict ? std::size_t{1} : std::size_t{0}) +
                                alt_code_preview_page_ * tail_capacity;
        std::vector<std::size_t> visible;
        if (has_strict) visible.push_back(0);
        for (std::size_t index = tail_begin;
             index < alt_code_preview_items_.size() &&
             visible.size() < preview_page_size; ++index)
            visible.push_back(index);
        float items_width = 0.0F;
        for (std::size_t shown = 0; shown < visible.size(); ++shown) {
            if (shown != 0) items_width += kCandidateGapDip;
            const auto index = visible[shown];
            const auto& item = alt_code_preview_items_[index];
            const float badge_width = std::max(
                26.0F, measure_text_width(dwrite_factory_, label_text_format_, item.code) +
                           12.0F);
            items_width += badge_width + 19.0F +
                           measure_text_width(dwrite_factory_,
                                              candidate_text_format_, item.result);
        }
        const auto header = std::wstring(L"Alt ＋ ") + alt_code_digits_;
        const float width = std::clamp(
            std::max(kHorizontalPaddingDip * 2.0F +
                         measure_text_width(dwrite_factory_, input_text_format_, header),
                     kHorizontalPaddingDip * 2.0F + items_width),
            kCandidateWindowMinWidthDip, kCandidateWindowMaxWidthDip);
        return SIZE{dips_to_pixels(width, dpi),
                    dips_to_pixels(kHeaderHeightDip + 16.0F +
                                       kCandidateItemHeightDip, dpi)};
    }
    if (unicode_palette_visible_ && !unicode_palette_.categories.empty()) {
        const float width = kHorizontalPaddingDip * 2.0F +
                            kUnicodeCellSizeDip * kUnicodeItemsPerRow;
        const float height = kHeaderHeightDip + 8.0F + kUnicodeCategoryHeightDip +
                             9.0F + kUnicodeVisibleRows * kUnicodeCellSizeDip + 8.0F;
        return SIZE{dips_to_pixels(width, dpi), dips_to_pixels(height, dpi)};
    }
    const auto row_height = [this, wrap_length](const std::size_t begin,
                                                const std::size_t end) {
        float height = kCandidateItemHeightDip;
        for (std::size_t index = begin; index < end; ++index)
            height = std::max(height,
                              wrapped_candidate_height(candidates_[index], wrap_length));
        return height;
    };
    float content_height = kCandidateItemHeightDip;
    if (!candidates_.empty() && candidates_expanded_) {
        const auto first = std::min(candidates_.size(), expanded_scroll_row_ * page_size);
        const auto visible_end = std::min(
            candidates_.size(), first + kExpandedVisibleRows * page_size);
        content_height = 0.0F;
        std::size_t rows = 0;
        for (std::size_t begin = first; begin < visible_end; begin += page_size) {
            if (rows++ != 0) content_height += kCandidateRowGapDip;
            content_height += row_height(begin, std::min(visible_end, begin + page_size));
        }
    } else if (!candidates_.empty()) {
        content_height = row_height(0, candidates_.size());
    }
    const float height = kHeaderHeightDip + 16.0F + content_height;
    const float controls_width = candidates_expanded_ && !plugin_scheme_selected()
                                     ? kExpandButtonWidthDip
                                     : kButtonWidthDip * 2.0F + kExpandButtonWidthDip +
                                           kControlGapDip * 2.0F;
    const bool partitioned = !plugin_scheme_selected() && page_size >= 4 &&
        candidate_groups_.size() == candidates_.size();
    const auto item_width = [this, wrap_length](const std::size_t index) {
        const auto display = wrapped_candidate_text(candidates_[index], wrap_length);
        return kCandidatePillBaseWidthDip +
               measure_text_width(dwrite_factory_, candidate_text_format_, display);
    };
    const auto row_width = [this, partitioned, &item_width](const std::size_t begin,
                                                            const std::size_t end) {
        float width = 0.0F;
        for (std::size_t index = begin; index < end; ++index) {
            if (index != begin) {
                const bool group_boundary = partitioned &&
                    candidate_groups_[index] != candidate_groups_[index - 1];
                width += group_boundary ? kCandidateGroupGapDip : kCandidateGapDip;
            }
            width += item_width(index);
        }
        return width;
    };
    float candidates_width = 0.0F;
    if (candidates_.empty()) {
        const auto status = is_agent_mode() && !agent_response_visible_ && input_buffer_.size() <= 1
            ? (staged_candidates_.empty()
                ? std::wstring_view(L"Agent 模式：继续输入请求")
                : std::wstring_view(L"完整选词后自动发送；Enter/空格直接发送"))
            : is_agent_mode() && agent_response_visible_
            ? (!agent_executing_command_.empty()
                ? std::wstring_view(agent_executing_command_)
                : agent_request_pending_ ? std::wstring_view(L"正在等待 Agent…")
                                         : std::wstring_view(L"暂无候选指令"))
            : input_buffer_.empty() && !staged_candidates_.empty()
            ? std::wstring_view(L"按 Enter 或空格上屏")
            : candidate_status_text(candidate_request_pending_,
                                    candidate_request_failed_,
                                    candidate_failure_detail_);
        candidates_width = kCandidatePillBaseWidthDip +
                           measure_text_width(dwrite_factory_, candidate_text_format_, status);
    } else if (candidates_expanded_) {
        for (std::size_t begin = 0; begin < candidates_.size(); begin += page_size) {
            const auto end = std::min(candidates_.size(), begin + page_size);
            candidates_width = std::max(candidates_width, row_width(begin, end));
        }
    } else {
        candidates_width = row_width(0, candidates_.size());
    }
    const std::wstring agent_preview = agent_preview_text();
    const std::wstring reading = agent_preview.empty()
        ? composition_preview() : agent_preview;
    const float preview_width = kHorizontalPaddingDip * 2.0F +
        measure_text_width(dwrite_factory_, input_text_format_, reading) +
        (unicode_palette_.categories.empty()
             ? 0.0F : kUnicodeEntryWidthDip + kCandidateGapDip);
    const float candidate_row_width = kHorizontalPaddingDip * 2.0F + candidates_width +
                                      kCandidateControlGapDip + controls_width;
    const float width = std::clamp(std::max(preview_width, candidate_row_width),
                                   kCandidateWindowMinWidthDip,
                                   kCandidateWindowMaxWidthDip);
    return SIZE{dips_to_pixels(width, dpi),
                dips_to_pixels(height, dpi)};
}

bool TextService::prepare_blurred_candidate_background() noexcept {
    if (!candidate_theme_.background_blur_enabled || layered_surface_pixels_ == nullptr ||
        layered_surface_dc_ == nullptr || layered_surface_size_.cx <= 0 ||
        layered_surface_size_.cy <= 0 || candidate_window_ == nullptr)
        return false;

    struct Pixel { std::uint8_t blue, green, red, alpha; };
    const int surface_width = layered_surface_size_.cx;
    const int surface_height = layered_surface_size_.cy;
    const int width = std::clamp<int>(candidate_content_size_.cx, 1, surface_width);
    const int height = std::clamp<int>(candidate_content_size_.cy, 1, surface_height);
    const auto count = static_cast<std::size_t>(surface_width) *
                       static_cast<std::size_t>(surface_height);
    auto* destination = static_cast<Pixel*>(layered_surface_pixels_);

    const UINT dpi = std::max(GetDpiForWindow(candidate_window_), 96U);
    const int radius = std::clamp(
        dips_to_pixels(candidate_theme_.background_blur_radius, dpi), 0, 48);
    if (blurred_candidate_background_.size() != count) {
        // Capture and blur the stable backing canvas, not just the current card.
        // Candidate text and card width can then change without another screen
        // capture, Gaussian pass, or compositor synchronization.
        const int capture_width = surface_width + radius * 2;
        const int capture_height = surface_height + radius * 2;
        const auto capture_count = static_cast<std::size_t>(capture_width) *
                                   static_cast<std::size_t>(capture_height);

        BITMAPINFO capture_info{};
        capture_info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        capture_info.bmiHeader.biWidth = capture_width;
        capture_info.bmiHeader.biHeight = -capture_height;
        capture_info.bmiHeader.biPlanes = 1;
        capture_info.bmiHeader.biBitCount = 32;
        capture_info.bmiHeader.biCompression = BI_RGB;
        HDC capture_dc = CreateCompatibleDC(nullptr);
        if (capture_dc == nullptr) return false;
        void* capture_pixels = nullptr;
        HBITMAP capture_bitmap = CreateDIBSection(
            capture_dc, &capture_info, DIB_RGB_COLORS, &capture_pixels, nullptr, 0);
        if (capture_bitmap == nullptr || capture_pixels == nullptr) {
            if (capture_bitmap != nullptr) DeleteObject(capture_bitmap);
            DeleteDC(capture_dc);
            return false;
        }
        HGDIOBJ capture_previous = SelectObject(capture_dc, capture_bitmap);
        if (capture_previous == nullptr || capture_previous == HGDI_ERROR) {
            DeleteObject(capture_bitmap);
            DeleteDC(capture_dc);
            return false;
        }

        POINT capture_origin = candidate_present_position_;
        if (!candidate_present_geometry_valid_) {
            RECT window_bounds{};
            if (!GetWindowRect(candidate_window_, &window_bounds)) {
                SelectObject(capture_dc, capture_previous);
                DeleteObject(capture_bitmap);
                DeleteDC(capture_dc);
                return false;
            }
            capture_origin = {window_bounds.left, window_bounds.top};
        }
        HDC screen = GetDC(nullptr);
        if (screen == nullptr) {
            SelectObject(capture_dc, capture_previous);
            DeleteObject(capture_bitmap);
            DeleteDC(capture_dc);
            return false;
        }
        // SRCCOPY deliberately omits CAPTUREBLT, so the layered candidate and
        // shadow windows do not feed back into their own backdrop. Avoiding the
        // previous display-affinity toggle also prevents DWM from withdrawing
        // the candidate surface for a frame.
        const BOOL copied = BitBlt(
            capture_dc, 0, 0, capture_width, capture_height, screen,
            capture_origin.x - radius, capture_origin.y - radius, SRCCOPY);
        ReleaseDC(nullptr, screen);
        if (!copied) {
            SelectObject(capture_dc, capture_previous);
            DeleteObject(capture_bitmap);
            DeleteDC(capture_dc);
            return false;
        }

        const auto* captured = static_cast<const Pixel*>(capture_pixels);
        std::vector<Pixel> source(captured, captured + capture_count);
        SelectObject(capture_dc, capture_previous);
        DeleteObject(capture_bitmap);
        DeleteDC(capture_dc);

        std::vector<Pixel> blurred;
        if (radius > 0) {
            std::vector<Pixel> horizontal(capture_count);
            blurred.resize(capture_count);
            const float sigma = std::max(static_cast<float>(radius) * 0.5F, 0.5F);
            std::vector<float> kernel(static_cast<std::size_t>(radius * 2 + 1));
            float kernel_sum = 0.0F;
            for (int offset = -radius; offset <= radius; ++offset) {
                const float value = std::exp(
                    -0.5F * static_cast<float>(offset * offset) / (sigma * sigma));
                kernel[static_cast<std::size_t>(offset + radius)] = value;
                kernel_sum += value;
            }
            for (auto& value : kernel) value /= kernel_sum;
            for (int y = 0; y < capture_height; ++y) {
                for (int x = 0; x < capture_width; ++x) {
                    float blue = 0.0F, green = 0.0F, red = 0.0F;
                    for (int offset = -radius; offset <= radius; ++offset) {
                        const int sample_x = std::clamp(x + offset, 0, capture_width - 1);
                        const auto& pixel = source[
                            static_cast<std::size_t>(y) * capture_width + sample_x];
                        const float weight =
                            kernel[static_cast<std::size_t>(offset + radius)];
                        blue += pixel.blue * weight;
                        green += pixel.green * weight;
                        red += pixel.red * weight;
                    }
                    auto& pixel = horizontal[
                        static_cast<std::size_t>(y) * capture_width + x];
                    pixel = {
                        static_cast<std::uint8_t>(std::clamp(std::lround(blue), 0L, 255L)),
                        static_cast<std::uint8_t>(std::clamp(std::lround(green), 0L, 255L)),
                        static_cast<std::uint8_t>(std::clamp(std::lround(red), 0L, 255L)),
                        255};
                }
            }
            for (int y = 0; y < capture_height; ++y) {
                for (int x = 0; x < capture_width; ++x) {
                    float blue = 0.0F, green = 0.0F, red = 0.0F;
                    for (int offset = -radius; offset <= radius; ++offset) {
                        const int sample_y = std::clamp(y + offset, 0, capture_height - 1);
                        const auto& pixel = horizontal[
                            static_cast<std::size_t>(sample_y) * capture_width + x];
                        const float weight =
                            kernel[static_cast<std::size_t>(offset + radius)];
                        blue += pixel.blue * weight;
                        green += pixel.green * weight;
                        red += pixel.red * weight;
                    }
                    auto& pixel = blurred[
                        static_cast<std::size_t>(y) * capture_width + x];
                    pixel = {
                        static_cast<std::uint8_t>(std::clamp(std::lround(blue), 0L, 255L)),
                        static_cast<std::uint8_t>(std::clamp(std::lround(green), 0L, 255L)),
                        static_cast<std::uint8_t>(std::clamp(std::lround(red), 0L, 255L)),
                        255};
                }
            }
        } else {
            blurred = std::move(source);
        }

        blurred_candidate_background_.resize(count);
        auto* cached = reinterpret_cast<Pixel*>(blurred_candidate_background_.data());
        for (int y = 0; y < surface_height; ++y) {
            for (int x = 0; x < surface_width; ++x) {
                cached[static_cast<std::size_t>(y) * surface_width + x] =
                    blurred[static_cast<std::size_t>(y + radius) * capture_width +
                            x + radius];
            }
        }
    }

    const auto* backdrop_pixels =
        reinterpret_cast<const Pixel*>(blurred_candidate_background_.data());
    const float scale = static_cast<float>(dpi) / 96.0F;
    const float inset = 0.5F * scale;
    const float corner = 10.0F * scale;
    const float tint_strength = std::clamp(
        candidate_theme_.background.opacity, 0.0F, 1.0F);
    const float tint_red = static_cast<float>((candidate_theme_.background.rgb >> 16) & 0xff);
    const float tint_green = static_cast<float>((candidate_theme_.background.rgb >> 8) & 0xff);
    const float tint_blue = static_cast<float>(candidate_theme_.background.rgb & 0xff);
    std::fill_n(static_cast<std::uint32_t*>(layered_surface_pixels_), count, 0U);
    for (int y = 0; y < height; ++y) {
        for (int x = 0; x < width; ++x) {
            auto& pixel = destination[
                static_cast<std::size_t>(y) * surface_width + x];
            const auto& backdrop = backdrop_pixels[
                static_cast<std::size_t>(y) * surface_width + x];
            const float coverage = rounded_rect_coverage(
                x + 0.5F, y + 0.5F, inset, inset,
                static_cast<float>(width) - inset,
                static_cast<float>(height) - inset, corner);
            const float red = backdrop.red * (1.0F - tint_strength) +
                              tint_red * tint_strength;
            const float green = backdrop.green * (1.0F - tint_strength) +
                                tint_green * tint_strength;
            const float blue = backdrop.blue * (1.0F - tint_strength) +
                               tint_blue * tint_strength;
            pixel.red = static_cast<std::uint8_t>(std::clamp(
                std::lround(red * coverage), 0L, 255L));
            pixel.green = static_cast<std::uint8_t>(std::clamp(
                std::lround(green * coverage), 0L, 255L));
            pixel.blue = static_cast<std::uint8_t>(std::clamp(
                std::lround(blue * coverage), 0L, 255L));
            pixel.alpha = static_cast<std::uint8_t>(std::clamp(
                std::lround(coverage * 255.0F), 0L, 255L));
        }
    }
    const RECT surface_bounds{0, 0, surface_width, surface_height};
    return SUCCEEDED(render_target_->BindDC(layered_surface_dc_, &surface_bounds));
}

bool TextService::present_candidate_surface() noexcept {
    if (candidate_window_ == nullptr || layered_surface_dc_ == nullptr) return false;
    POINT destination = candidate_present_position_;
    if (!candidate_present_geometry_valid_) {
        RECT window_bounds{};
        if (!GetWindowRect(candidate_window_, &window_bounds)) return false;
        destination = {window_bounds.left, window_bounds.top};
    }
    POINT source{};
    SIZE layered_size{layered_surface_size_.cx, layered_surface_size_.cy};
    BLENDFUNCTION blend{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
    HDC screen = GetDC(nullptr);
    if (screen == nullptr) return false;
    // UpdateLayeredWindow copies the completed off-screen bitmap and changes
    // the HWND geometry as one compositor operation. The visible window never
    // observes an empty intermediate size.
    candidate_resize_in_progress_ = true;
    const BOOL updated = UpdateLayeredWindow(
        candidate_window_, screen, &destination, &layered_size,
        layered_surface_dc_, &source, 0, &blend, ULW_ALPHA);
    candidate_resize_in_progress_ = false;
    ReleaseDC(nullptr, screen);
    if (!updated) return false;
    // WS_EX_TOPMOST places the window in the topmost band at creation, but
    // another topmost foreground window can subsequently cover it. Publishing
    // a layered bitmap does not restore Z order. Reassert it on every completed
    // frame, without activating, moving, resizing or clearing the bitmap.
    const HWND host = GetForegroundWindow();
    if (foreground_focus_) bind_candidate_owner(candidate_window_, candidate_shadow_window_, host);
    const bool shown = SetWindowPos(candidate_window_, HWND_TOPMOST, 0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE |
                            SWP_NOOWNERZORDER | SWP_SHOWWINDOW) != FALSE;
    if (shown && message_window_) SetTimer(message_window_, kCandidateWindowOrderTimer, 100, nullptr);
    return shown;
}

void TextService::maintain_candidate_window_order() noexcept {
    if (!candidate_window_ || !IsWindowVisible(candidate_window_)) {
        if (message_window_) KillTimer(message_window_, kCandidateWindowOrderTimer);
        return;
    }
    const HWND host = GetForegroundWindow();
    // A foreground frame may embed an editor from this process. Require its
    // actual focused descendant, not merely a matching executable or owner.
    // A null foreground HWND is transient during activation; wait for TSF focus.
    if (!foreground_focus_ || (host && !candidate_host_has_process_focus(host))) {
        hide_candidate_windows();
        return;
    }
    bind_candidate_owner(candidate_window_, candidate_shadow_window_, host);
    if (candidate_needs_raise(candidate_window_, host)) {
        raise_candidate_windows(candidate_window_, candidate_shadow_window_);
    }
}

void TextService::update_candidate_shadow() noexcept {
    if (candidate_shadow_window_ == nullptr || candidate_window_ == nullptr ||
        !candidate_theme_.shadow_enabled || candidate_theme_.shadow.opacity <= 0.0F ||
        !IsWindowVisible(candidate_window_)) {
        if (candidate_shadow_window_ != nullptr) ShowWindow(candidate_shadow_window_, SW_HIDE);
        return;
    }
    RECT candidate{};
    if (candidate_present_geometry_valid_) {
        candidate = {
            candidate_present_position_.x,
            candidate_present_position_.y,
            candidate_present_position_.x + candidate_content_size_.cx,
            candidate_present_position_.y + candidate_content_size_.cy};
    } else if (!GetWindowRect(candidate_window_, &candidate)) {
        return;
    }
    const UINT dpi = std::max(GetDpiForWindow(candidate_window_), 96U);
    const float scale = static_cast<float>(dpi) / 96.0F;
    const float blur = std::max(candidate_theme_.shadow_blur_radius * scale, 1.0F);
    const float offset_x = candidate_theme_.shadow_offset_x * scale;
    const float offset_y = candidate_theme_.shadow_offset_y * scale;
    const float sigma = std::max(blur * 0.5F, 0.5F);
    const int kernel_radius = std::max(1, static_cast<int>(std::ceil(sigma * 3.0F)));
    const int margin = kernel_radius + static_cast<int>(std::ceil(
        std::max(std::abs(offset_x), std::abs(offset_y)))) + 2;
    const int candidate_width = candidate.right - candidate.left;
    const int candidate_height = candidate.bottom - candidate.top;
    const int width = candidate_width + margin * 2;
    const int height = candidate_height + margin * 2;
    if (width <= 0 || height <= 0) return;
    POINT destination{candidate.left - margin, candidate.top - margin};
    if (candidate_shadow_surface_valid_ &&
        candidate_shadow_surface_size_.cx == width &&
        candidate_shadow_surface_size_.cy == height &&
        candidate_shadow_theme_ == candidate_theme_) {
        SetWindowPos(candidate_shadow_window_, candidate_window_, destination.x,
                     destination.y, width, height,
                     SWP_NOACTIVATE | SWP_SHOWWINDOW);
        return;
    }

    BITMAPINFO info{};
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    HDC memory = CreateCompatibleDC(nullptr);
    if (memory == nullptr) return;
    void* raw_pixels = nullptr;
    HBITMAP bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &raw_pixels, nullptr, 0);
    if (bitmap == nullptr || raw_pixels == nullptr) {
        if (bitmap != nullptr) DeleteObject(bitmap);
        DeleteDC(memory);
        return;
    }
    HGDIOBJ previous = SelectObject(memory, bitmap);
    auto* pixels = static_cast<std::uint32_t*>(raw_pixels);
    const float corner = 10.0F * scale;
    const float inset = 0.5F * scale;
    const float actual_left = margin + inset;
    const float actual_top = margin + inset;
    const float actual_right = margin + candidate_width - inset;
    const float actual_bottom = margin + candidate_height - inset;
    const auto count = static_cast<std::size_t>(width) * static_cast<std::size_t>(height);
    std::vector<float> source_mask(count);
    std::vector<float> horizontal_mask(count);
    std::vector<float> blurred_mask(count);
    for (int y = 0; y < height; ++y) {
        for (int x = 0; x < width; ++x) {
            source_mask[static_cast<std::size_t>(y) * width + x] =
                rounded_rect_coverage(
                    x + 0.5F, y + 0.5F,
                    actual_left + offset_x, actual_top + offset_y,
                    actual_right + offset_x, actual_bottom + offset_y, corner);
        }
    }
    std::vector<float> kernel(static_cast<std::size_t>(kernel_radius * 2 + 1));
    float kernel_sum = 0.0F;
    for (int offset = -kernel_radius; offset <= kernel_radius; ++offset) {
        const float value = std::exp(-0.5F * static_cast<float>(offset * offset) /
                                     (sigma * sigma));
        kernel[static_cast<std::size_t>(offset + kernel_radius)] = value;
        kernel_sum += value;
    }
    for (auto& value : kernel) value /= kernel_sum;
    for (int y = 0; y < height; ++y) {
        for (int x = 0; x < width; ++x) {
            float alpha = 0.0F;
            for (int offset = -kernel_radius; offset <= kernel_radius; ++offset) {
                const int sample_x = x + offset;
                if (sample_x < 0 || sample_x >= width) continue;
                alpha += source_mask[static_cast<std::size_t>(y) * width + sample_x] *
                         kernel[static_cast<std::size_t>(offset + kernel_radius)];
            }
            horizontal_mask[static_cast<std::size_t>(y) * width + x] = alpha;
        }
    }
    for (int y = 0; y < height; ++y) {
        for (int x = 0; x < width; ++x) {
            float alpha = 0.0F;
            for (int offset = -kernel_radius; offset <= kernel_radius; ++offset) {
                const int sample_y = y + offset;
                if (sample_y < 0 || sample_y >= height) continue;
                alpha += horizontal_mask[static_cast<std::size_t>(sample_y) * width + x] *
                         kernel[static_cast<std::size_t>(offset + kernel_radius)];
            }
            blurred_mask[static_cast<std::size_t>(y) * width + x] = alpha;
        }
    }
    const unsigned red = (candidate_theme_.shadow.rgb >> 16) & 0xff;
    const unsigned green = (candidate_theme_.shadow.rgb >> 8) & 0xff;
    const unsigned blue = candidate_theme_.shadow.rgb & 0xff;
    const float maximum_alpha = std::clamp(candidate_theme_.shadow.opacity, 0.0F, 1.0F);
    for (int y = 0; y < height; ++y) {
        for (int x = 0; x < width; ++x) {
            const auto index = static_cast<std::size_t>(y) * width + x;
            const float card_coverage = rounded_rect_coverage(
                x + 0.5F, y + 0.5F, actual_left, actual_top,
                actual_right, actual_bottom, corner);
            const float falloff = blurred_mask[index] * (1.0F - card_coverage);
            const unsigned alpha = static_cast<unsigned>(std::clamp(
                std::lround(maximum_alpha * falloff * 255.0F), 0L, 255L));
            const unsigned premultiplied_red = red * alpha / 255;
            const unsigned premultiplied_green = green * alpha / 255;
            const unsigned premultiplied_blue = blue * alpha / 255;
            pixels[index] =
                (alpha << 24) | (premultiplied_red << 16) |
                (premultiplied_green << 8) | premultiplied_blue;
        }
    }
    POINT source{};
    SIZE size{width, height};
    BLENDFUNCTION blend{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
    HDC screen = GetDC(nullptr);
    if (screen != nullptr) {
        const BOOL updated = UpdateLayeredWindow(
            candidate_shadow_window_, screen, &destination, &size,
            memory, &source, 0, &blend, ULW_ALPHA);
        ReleaseDC(nullptr, screen);
        if (updated) {
            candidate_shadow_surface_size_ = size;
            candidate_shadow_theme_ = candidate_theme_;
            candidate_shadow_surface_valid_ = true;
            SetWindowPos(candidate_shadow_window_, candidate_window_, 0, 0, 0, 0,
                         SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
    }
    SelectObject(memory, previous);
    DeleteObject(bitmap);
    DeleteDC(memory);
}

void TextService::hide_candidate_windows() noexcept {
    if (message_window_) KillTimer(message_window_, kCandidateWindowOrderTimer);
    if (candidate_window_ != nullptr) ShowWindow(candidate_window_, SW_HIDE);
    if (candidate_shadow_window_ != nullptr) ShowWindow(candidate_shadow_window_, SW_HIDE);
    blurred_candidate_background_.clear();
}

bool TextService::render_candidate_window() {
    hit_regions_.clear();
    SIZE surface_size = candidate_present_size_;
    if (!candidate_present_geometry_valid_) {
        RECT bounds{};
        if (!GetClientRect(candidate_window_, &bounds)) return false;
        surface_size = {bounds.right - bounds.left, bounds.bottom - bounds.top};
        candidate_content_size_ = surface_size;
    }
    if (FAILED(ensure_device_resources(surface_size))) return false;
    const bool blurred_background = prepare_blurred_candidate_background();
    render_target_->BeginDraw();
    render_target_->SetTransform(D2D1::Matrix3x2F::Identity());
    render_target_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    auto* mode_background_brush = background_brush_;
    auto* mode_accent_brush = accent_brush_;
    auto* mode_highlight_brush = highlight_brush_;
    auto* mode_border_brush = border_brush_;
    auto* candidate_number_brush = strict_accent_brush_;
    if (!blurred_background) render_target_->Clear(D2D1::ColorF(0x000000, 0.0F));

    const UINT dpi = std::max(GetDpiForWindow(candidate_window_), 96U);
    const float pixel_to_dip = 96.0F / static_cast<float>(dpi);
    const D2D1_SIZE_F size = D2D1::SizeF(
        static_cast<float>(candidate_content_size_.cx) * pixel_to_dip,
        static_cast<float>(candidate_content_size_.cy) * pixel_to_dip);
    const D2D1_ROUNDED_RECT card{
        D2D1::RectF(0.5F, 0.5F, size.width - 0.5F, size.height - 0.5F), 10.0F, 10.0F};
    if (!blurred_background)
        render_target_->FillRoundedRectangle(card, mode_background_brush);
    render_target_->DrawRoundedRectangle(card, mode_border_brush, 1.0F);

    if (alt_code_preview_active_) {
        const auto header = std::wstring(L"Alt ＋ ") + alt_code_digits_;
        render_target_->DrawTextW(
            header.data(), static_cast<UINT32>(header.size()), input_text_format_,
            D2D1::RectF(kHorizontalPaddingDip, 0.0F,
                        size.width - kHorizontalPaddingDip, kHeaderHeightDip),
            mode_accent_brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        render_target_->DrawLine(
            D2D1::Point2F(10.0F, kHeaderHeightDip),
            D2D1::Point2F(size.width - 10.0F, kHeaderHeightDip),
            divider_brush_, 1.0F);

        const auto page_size = std::max<std::size_t>(
            2, static_cast<std::size_t>(shortcut_config_.alt_code_preview_page_size));
        const bool has_strict = !alt_code_preview_items_.empty() &&
                                alt_code_preview_items_.front().strict;
        const auto tail_capacity = has_strict ? page_size - 1 : page_size;
        const auto tail_begin = (has_strict ? std::size_t{1} : std::size_t{0}) +
                                alt_code_preview_page_ * tail_capacity;
        std::vector<std::size_t> visible;
        if (has_strict) visible.push_back(0);
        for (std::size_t index = tail_begin;
             index < alt_code_preview_items_.size() && visible.size() < page_size;
             ++index)
            visible.push_back(index);
        float x = kHorizontalPaddingDip;
        constexpr float top = kHeaderHeightDip + 8.0F;
        for (const auto index : visible) {
            const auto& item = alt_code_preview_items_[index];
            const float badge_width = std::max(
                26.0F, measure_text_width(dwrite_factory_, label_text_format_, item.code) +
                           12.0F);
            const float requested = badge_width + 19.0F +
                measure_text_width(dwrite_factory_, candidate_text_format_,
                                   item.result);
            const float right = std::min(size.width - kHorizontalPaddingDip,
                                         x + requested);
            const D2D1_ROUNDED_RECT pill{
                D2D1::RectF(x, top, right, top + kCandidateItemHeightDip), 7.0F, 7.0F};
            if (item.strict)
                render_target_->FillRoundedRectangle(pill, mode_highlight_brush);

            const float badge_center = (pill.rect.top + pill.rect.bottom) * 0.5F;
            const D2D1_ROUNDED_RECT badge{
                D2D1::RectF(pill.rect.left + 6.0F, badge_center - 11.0F,
                            pill.rect.left + 6.0F + badge_width,
                            badge_center + 11.0F),
                5.0F, 5.0F};
            render_target_->FillRoundedRectangle(badge, mode_border_brush);
            render_target_->DrawTextW(
                item.code.data(), static_cast<UINT32>(item.code.size()),
                label_text_format_, badge.rect, mode_accent_brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP);
            render_target_->DrawTextW(
                item.result.data(), static_cast<UINT32>(item.result.size()),
                candidate_text_format_,
                D2D1::RectF(badge.rect.right + 7.0F, top, right - 6.0F,
                            top + kCandidateItemHeightDip), text_brush_,
                D2D1_DRAW_TEXT_OPTIONS_CLIP);
            x = right + kCandidateGapDip;
        }
        if (visible.empty()) {
            constexpr std::wstring_view status = L"当前代码暂无可显示符号";
            render_target_->DrawTextW(
                status.data(), static_cast<UINT32>(status.size()), candidate_text_format_,
                D2D1::RectF(kHorizontalPaddingDip, top,
                            size.width - kHorizontalPaddingDip,
                            top + kCandidateItemHeightDip),
                secondary_text_brush_, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
        const HRESULT result = render_target_->EndDraw();
        if (FAILED(result)) {
            discard_device_resources();
            return false;
        }
        return present_candidate_surface();
    }

    if (unicode_palette_visible_ && !unicode_palette_.categories.empty()) {
        const auto target_matches = [](const std::optional<HitTarget>& value,
                                       const HitTarget target) {
            return value.has_value() && *value == target;
        };
        const auto add_hit = [this, dpi](const D2D1_RECT_F bounds,
                                         const HitTarget target) {
            hit_regions_.push_back({dip_rect_to_pixels(bounds, dpi), target});
        };
        const auto draw_pill = [this, &target_matches, &add_hit, mode_accent_brush,
                                mode_border_brush, mode_highlight_brush](
                                   const D2D1_RECT_F bounds, const HitTarget target,
                                   const std::wstring_view label, const bool selected,
                                   const bool enabled = true) {
            const bool hovered = enabled && target_matches(hovered_target_, target);
            const bool pressed = enabled && target_matches(pressed_target_, target);
            const D2D1_ROUNDED_RECT pill{bounds, 7.0F, 7.0F};
            if (selected || hovered || pressed)
                render_target_->FillRoundedRectangle(pill, mode_highlight_brush);
            render_target_->DrawRoundedRectangle(pill, mode_border_brush, 1.0F);
            render_target_->DrawTextW(
                label.data(), static_cast<UINT32>(label.size()), label_text_format_,
                bounds, enabled ? (hovered || pressed ? mode_accent_brush : text_brush_)
                                : secondary_text_brush_,
                D2D1_DRAW_TEXT_OPTIONS_CLIP);
            if (enabled) add_hit(bounds, target);
        };

        const std::wstring header = agent_preview_text().empty()
            ? composition_preview() : agent_preview_text();
        const D2D1_RECT_F entry_bounds = D2D1::RectF(
            size.width - kHorizontalPaddingDip - kUnicodeEntryWidthDip, 5.0F,
            size.width - kHorizontalPaddingDip, kHeaderHeightDip - 5.0F);
        render_target_->DrawTextW(
            header.data(), static_cast<UINT32>(header.size()), input_text_format_,
            D2D1::RectF(kHorizontalPaddingDip, 0.0F,
                        entry_bounds.left - kCandidateGapDip, kHeaderHeightDip),
            mode_accent_brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        draw_pill(entry_bounds, {HitKind::unicode_toggle, 0}, L"Ω", true);
        render_target_->DrawLine(
            D2D1::Point2F(10.0F, kHeaderHeightDip),
            D2D1::Point2F(size.width - 10.0F, kHeaderHeightDip), divider_brush_, 1.0F);

        unicode_category_index_ = std::min(
            unicode_category_index_, unicode_palette_.categories.size() - 1);
        const float usable_width = size.width - kHorizontalPaddingDip * 2.0F;
        const auto category_count = unicode_palette_.categories.size();
        float total_category_width = 0.0F;
        for (const auto& category : unicode_palette_.categories) {
            if (total_category_width != 0.0F)
                total_category_width += kCandidateGapDip;
            total_category_width += std::max(
                56.0F, measure_text_width(dwrite_factory_, label_text_format_,
                                           wide_from_utf8(category.label)) + 18.0F);
        }
        unicode_category_scroll_offset_dip_ = std::clamp(
            unicode_category_scroll_offset_dip_, 0.0F,
            std::max(0.0F, total_category_width - usable_width));
        float x = kHorizontalPaddingDip - unicode_category_scroll_offset_dip_;
        const float category_top = kHeaderHeightDip + 8.0F;
        const D2D1_RECT_F category_clip = D2D1::RectF(
            kHorizontalPaddingDip, category_top,
            size.width - kHorizontalPaddingDip,
            category_top + kUnicodeCategoryHeightDip);
        render_target_->PushAxisAlignedClip(
            category_clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        for (std::size_t index = 0; index < category_count; ++index) {
            const auto label = wide_from_utf8(unicode_palette_.categories[index].label);
            const float category_width = std::max(
                56.0F, measure_text_width(dwrite_factory_, label_text_format_, label) +
                           18.0F);
            const auto bounds = D2D1::RectF(
                x, category_top, x + category_width,
                category_top + kUnicodeCategoryHeightDip);
            if (bounds.right > category_clip.left &&
                bounds.left < category_clip.right) {
                draw_pill(bounds, {HitKind::unicode_category, index}, label,
                          index == unicode_category_index_);
                if (!hit_regions_.empty()) {
                    auto& hit = hit_regions_.back().bounds;
                    hit.left = std::max(hit.left,
                        static_cast<LONG>(dips_to_pixels(category_clip.left, dpi)));
                    hit.right = std::min(hit.right,
                        static_cast<LONG>(dips_to_pixels(category_clip.right, dpi)));
                }
            }
            x += category_width + kCandidateGapDip;
        }
        render_target_->PopAxisAlignedClip();

        const auto& category = unicode_palette_.categories[unicode_category_index_];
        const auto total_rows = (category.items.size() + kUnicodeItemsPerRow - 1) /
                                kUnicodeItemsPerRow;
        const auto maximum_scroll = total_rows > kUnicodeVisibleRows
                                        ? total_rows - kUnicodeVisibleRows : 0;
        unicode_scroll_row_ = std::min(unicode_scroll_row_, maximum_scroll);
        const auto begin = std::min(category.items.size(),
                                    unicode_scroll_row_ * kUnicodeItemsPerRow);
        const auto end = std::min(
            category.items.size(),
            begin + kUnicodeItemsPerRow * kUnicodeVisibleRows);
        const float divider_top = category_top + kUnicodeCategoryHeightDip + 4.0F;
        render_target_->DrawLine(
            D2D1::Point2F(kHorizontalPaddingDip, divider_top),
            D2D1::Point2F(size.width - kHorizontalPaddingDip, divider_top),
            divider_brush_, 1.0F);
        const float item_top = divider_top + 5.0F;
        const float item_width = usable_width /
                                 static_cast<float>(kUnicodeItemsPerRow);
        candidate_text_format_->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
        for (std::size_t index = begin; index < end; ++index) {
            const auto shown = index - begin;
            const auto row = shown / kUnicodeItemsPerRow;
            const auto column = shown % kUnicodeItemsPerRow;
            const float left = kHorizontalPaddingDip + column * item_width;
            const float top = item_top + row * item_width;
            const D2D1_RECT_F bounds = D2D1::RectF(
                left, top, left + item_width, top + item_width);
            const HitTarget target{HitKind::unicode_item, index};
            const bool hovered = target_matches(hovered_target_, target);
            const bool pressed = target_matches(pressed_target_, target);
            if (hovered || pressed)
                render_target_->FillRectangle(bounds, mode_highlight_brush);
            const auto glyph = unicode_scalar(category.items[index].codepoint);
            render_target_->DrawTextW(
                glyph.data(), static_cast<UINT32>(glyph.size()),
                candidate_text_format_, bounds,
                hovered || pressed ? mode_accent_brush : text_brush_,
                D2D1_DRAW_TEXT_OPTIONS_CLIP);
            add_hit(bounds, target);
        }
        candidate_text_format_->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);

        if (hovered_target_ && hovered_target_->kind == HitKind::unicode_item &&
            hovered_target_->candidate_index < category.items.size()) {
            const auto& item = category.items[hovered_target_->candidate_index];
            std::wstring info = unicode_scalar(item.codepoint);
            info.append(L"  ");
            info.append(unicode_code_label(item.codepoint));
            if (!item.name.empty()) {
                info.append(L"  ");
                info.append(wide_from_utf8(item.name));
            }
            const float info_width = std::min(
                usable_width,
                std::max(120.0F, measure_text_width(
                    dwrite_factory_, label_text_format_, info) + 24.0F));
            constexpr float info_height = 30.0F;
            const float pointer_x = static_cast<float>(unicode_hover_point_.x) *
                                    96.0F / static_cast<float>(dpi);
            const float pointer_y = static_cast<float>(unicode_hover_point_.y) *
                                    96.0F / static_cast<float>(dpi);
            const float info_left = std::clamp(
                pointer_x + 14.0F, kHorizontalPaddingDip,
                size.width - kHorizontalPaddingDip - info_width);
            const float info_top = std::clamp(
                pointer_y + 18.0F, kHeaderHeightDip,
                size.height - kHorizontalPaddingDip - info_height);
            const D2D1_ROUNDED_RECT info_bounds{
                D2D1::RectF(info_left, info_top, info_left + info_width,
                            info_top + info_height), 6.0F, 6.0F};
            render_target_->FillRoundedRectangle(info_bounds, background_brush_);
            render_target_->DrawRoundedRectangle(info_bounds, mode_border_brush, 1.0F);
            render_target_->DrawTextW(
                info.data(), static_cast<UINT32>(info.size()), label_text_format_,
                info_bounds.rect, text_brush_, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        }
        const HRESULT result = render_target_->EndDraw();
        if (FAILED(result)) {
            discard_device_resources();
            return false;
        }
        return present_candidate_surface();
    }

    std::wstring reading = segmented_input_.empty() ? input_buffer_ : segmented_input_;
    const std::wstring staged = staged_text();
    const std::wstring agent_preview = agent_preview_text();
    const bool editable_agent_preview = agent_preview.empty() && is_agent_mode() &&
        !input_buffer_.empty() && input_buffer_.front() == L'v';
    std::wstring logical_input = input_buffer_;
    std::size_t agent_prefix_length = 0;
    if (editable_agent_preview) {
        agent_prefix_length = 4;  // "v + "
        if (!reading.empty() && reading.front() == L'v') reading.erase(reading.begin());
        if (!logical_input.empty() && logical_input.front() == L'v')
            logical_input.erase(logical_input.begin());
    }
    const std::wstring preview = agent_preview.empty()
        ? composition_preview() : agent_preview;
    const bool unicode_entry_visible = !unicode_palette_.categories.empty();
    const float preview_right = unicode_entry_visible
        ? size.width - kHorizontalPaddingDip - kUnicodeEntryWidthDip - kCandidateGapDip
        : size.width - kHorizontalPaddingDip;
    IDWriteTextLayout* preview_layout = nullptr;
    const HRESULT preview_layout_result = dwrite_factory_->CreateTextLayout(
        preview.data(), static_cast<UINT32>(preview.size()), input_text_format_,
        std::max(1.0F, preview_right - kHorizontalPaddingDip),
        kHeaderHeightDip, &preview_layout);
    if (SUCCEEDED(preview_layout_result) && preview_layout != nullptr) {
        render_target_->DrawTextLayout(
            D2D1::Point2F(kHorizontalPaddingDip, 0.0F), preview_layout,
            mode_accent_brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);

        std::vector<float> caret_positions(preview.size() + 1, 0.0F);
        if (agent_preview.empty()) for (std::size_t shown = 0; shown <= preview.size(); ++shown) {
            FLOAT x = 0.0F;
            FLOAT y = 0.0F;
            DWRITE_HIT_TEST_METRICS metrics{};
            if (SUCCEEDED(preview_layout->HitTestTextPosition(
                    static_cast<UINT32>(shown), FALSE, &x, &y, &metrics)))
                caret_positions[shown] = x;
            else if (shown != 0)
                caret_positions[shown] = caret_positions[shown - 1];
        }
        if (agent_preview.empty()) for (std::size_t shown = 0; shown <= preview.size(); ++shown) {
            const float left = shown == 0
                                   ? kHorizontalPaddingDip
                                   : kHorizontalPaddingDip +
                                         (caret_positions[shown - 1] +
                                          caret_positions[shown]) * 0.5F;
            const float right = shown == preview.size()
                                    ? size.width - kHorizontalPaddingDip
                                    : kHorizontalPaddingDip +
                                          (caret_positions[shown] +
                                           caret_positions[shown + 1]) * 0.5F;
            hit_regions_.push_back({
                dip_rect_to_pixels(
                    D2D1::RectF(left, 0.0F, std::max(left + 1.0F, right),
                                kHeaderHeightDip), dpi),
                {HitKind::pinyin_cursor,
                 [this, &staged, &reading, &logical_input, editable_agent_preview,
                  agent_prefix_length, shown] {
                    if (plugin_scheme_selected()) return shown;
                    if (editable_agent_preview && shown <= agent_prefix_length)
                        return std::size_t{1};
                    const auto relative = shown -
                        std::min(shown, agent_prefix_length);
                    if (relative <= staged.size()) {
                        std::size_t offset = 0;
                        for (std::size_t index = 0;
                             index < staged_candidates_.size(); ++index) {
                            const auto next = offset + staged_candidates_[index].text.size();
                            if (relative <= next) {
                                if (editable_agent_preview)
                                    return index + 2;
                                return relative * 2 < offset + next ? index : index + 1;
                            }
                            offset = next;
                        }
                        return staged_candidates_.size() +
                               static_cast<std::size_t>(editable_agent_preview);
                    }
                    return staged_candidates_.size() + input_cursor_for_preview_index(
                        logical_input, reading, relative - staged.size()) +
                        (editable_agent_preview ? 1 : 0);
                }()}});
        }
        if (agent_preview.empty()) {
            std::size_t shown_cursor = 0;
            if (staged_cursor_ < staged_candidates_.size()) {
                for (std::size_t index = 0; index < staged_cursor_; ++index)
                    shown_cursor += staged_candidates_[index].text.size();
            } else {
                shown_cursor = plugin_scheme_selected() ? input_cursor_ : staged.size() + preview_index_for_input_cursor(
                    logical_input, reading, editable_agent_preview && input_cursor_ > 0
                        ? input_cursor_ - 1 : input_cursor_);
            }
            shown_cursor += agent_prefix_length;
            const float caret_x = kHorizontalPaddingDip +
                                  caret_positions[std::min(shown_cursor, preview.size())];
            render_target_->DrawLine(
                D2D1::Point2F(caret_x, 7.0F),
                D2D1::Point2F(caret_x, kHeaderHeightDip - 7.0F),
                mode_accent_brush, 1.5F);
        }
        preview_layout->Release();
    } else {
        render_target_->DrawTextW(
            preview.data(), static_cast<UINT32>(preview.size()), input_text_format_,
            D2D1::RectF(kHorizontalPaddingDip, 0.0F,
                        preview_right, kHeaderHeightDip),
            mode_accent_brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
    }
    if (unicode_entry_visible) {
        const HitTarget target{HitKind::unicode_toggle, 0};
        const D2D1_RECT_F bounds = D2D1::RectF(
            size.width - kHorizontalPaddingDip - kUnicodeEntryWidthDip, 5.0F,
            size.width - kHorizontalPaddingDip, kHeaderHeightDip - 5.0F);
        const bool hovered = hovered_target_.has_value() && *hovered_target_ == target;
        const bool pressed = pressed_target_.has_value() && *pressed_target_ == target;
        const D2D1_ROUNDED_RECT button{bounds, 7.0F, 7.0F};
        if (hovered || pressed)
            render_target_->FillRoundedRectangle(button, mode_highlight_brush);
        render_target_->DrawRoundedRectangle(button, mode_border_brush, 1.0F);
        constexpr std::wstring_view label = L"Ω";
        render_target_->DrawTextW(
            label.data(), static_cast<UINT32>(label.size()), label_text_format_,
            bounds, hovered || pressed ? mode_accent_brush : text_brush_,
            D2D1_DRAW_TEXT_OPTIONS_CLIP);
        hit_regions_.push_back({dip_rect_to_pixels(bounds, dpi), target});
    }
    render_target_->DrawLine(
        D2D1::Point2F(10.0F, kHeaderHeightDip),
        D2D1::Point2F(size.width - 10.0F, kHeaderHeightDip), divider_brush_, 1.0F);

    const auto target_matches = [](const std::optional<HitTarget>& value,
                                   const HitTarget target) {
        return value.has_value() && *value == target;
    };
    const auto add_hit_region = [this, dpi](const D2D1_RECT_F bounds,
                                            const HitTarget target) {
        hit_regions_.push_back({dip_rect_to_pixels(bounds, dpi), target});
    };
    const auto wrap_length = std::max<std::size_t>(
        1, static_cast<std::size_t>(shortcut_config_.candidate_wrap_length));
    const auto draw_candidate = [this, &target_matches, &add_hit_region, wrap_length,
                                 mode_accent_brush, mode_border_brush,
                                 mode_highlight_brush, candidate_number_brush](
                                    const D2D1_RECT_F bounds, const std::size_t index) {
        const HitTarget target{HitKind::candidate, index};
        const D2D1_ROUNDED_RECT pill{bounds, 7.0F, 7.0F};
        const bool hovered = target_matches(hovered_target_, target);
        const bool pressed = target_matches(pressed_target_, target);
        if (index == 0 || hovered || pressed)
            render_target_->FillRoundedRectangle(pill, mode_highlight_brush);
        if (hovered || pressed)
            render_target_->DrawRoundedRectangle(pill, mode_accent_brush, 1.0F);

        const float badge_center = (bounds.top + bounds.bottom) * 0.5F;
        const D2D1_ROUNDED_RECT badge{
            D2D1::RectF(bounds.left + 6.0F, badge_center - 11.0F,
                        bounds.left + 26.0F, badge_center + 11.0F),
            5.0F, 5.0F};
        render_target_->FillRoundedRectangle(badge, mode_border_brush);
        const std::wstring label = std::to_wstring(index + 1);
        render_target_->DrawTextW(label.data(), static_cast<UINT32>(label.size()),
                                  label_text_format_, badge.rect, candidate_number_brush,
                                  D2D1_DRAW_TEXT_OPTIONS_CLIP);
        const auto display = wrapped_candidate_text(candidates_[index], wrap_length);
        render_target_->DrawTextW(
            display.data(), static_cast<UINT32>(display.size()),
            candidate_text_format_,
            D2D1::RectF(bounds.left + 33.0F, bounds.top, bounds.right - 6.0F,
                        bounds.bottom),
            text_brush_, D2D1_DRAW_TEXT_OPTIONS_CLIP);
        add_hit_region(bounds, target);
    };

    constexpr float content_top = kHeaderHeightDip + 8.0F;
    const float controls_width = candidates_expanded_ && !plugin_scheme_selected()
                                     ? kExpandButtonWidthDip
                                     : kButtonWidthDip * 2.0F + kExpandButtonWidthDip +
                                           kControlGapDip * 2.0F;
    const float controls_left = size.width - kHorizontalPaddingDip - controls_width;
    const float candidate_right = controls_left - kCandidateControlGapDip;
    const bool partitioned = !plugin_scheme_selected() && candidate_page_size_ >= 4 &&
        candidate_groups_.size() == candidates_.size();
    const auto draw_candidate_row = [this, partitioned, candidate_right,
                                     &draw_candidate, wrap_length](const std::size_t begin,
                                                        const std::size_t end,
                                                        const float top,
                                                        const float row_height) {
        if (begin >= end) return;
        float gaps_width = 0.0F;
        for (std::size_t index = begin + 1; index < end; ++index) {
            const bool boundary = partitioned &&
                candidate_groups_[index] != candidate_groups_[index - 1];
            gaps_width += boundary ? kCandidateGroupGapDip : kCandidateGapDip;
        }
        float requested_total = 0.0F;
        for (std::size_t index = begin; index < end; ++index) {
            const auto display = wrapped_candidate_text(candidates_[index], wrap_length);
            requested_total += kCandidatePillBaseWidthDip +
                               measure_text_width(dwrite_factory_, candidate_text_format_,
                                                  display);
        }
        const float usable_width = std::max(
            1.0F, candidate_right - kHorizontalPaddingDip - gaps_width);
        const float width_scale = requested_total > usable_width
                                      ? usable_width / requested_total
                                      : 1.0F;
        float x = kHorizontalPaddingDip;
        for (std::size_t index = begin; index < end; ++index) {
            if (index != begin) {
                const bool boundary = partitioned &&
                    candidate_groups_[index] != candidate_groups_[index - 1];
                if (boundary) {
                    x += kCandidateGroupGapDip * 0.5F;
                    render_target_->DrawLine(
                        D2D1::Point2F(x, top + 3.0F),
                        D2D1::Point2F(x, top + row_height - 3.0F),
                        divider_brush_, 1.0F);
                    x += kCandidateGroupGapDip * 0.5F;
                } else {
                    x += kCandidateGapDip;
                }
            }
            const auto display = wrapped_candidate_text(candidates_[index], wrap_length);
            const float requested_width =
                kCandidatePillBaseWidthDip +
                measure_text_width(dwrite_factory_, candidate_text_format_, display);
            const float width = requested_width * width_scale;
            draw_candidate(D2D1::RectF(x, top, x + width, top + row_height), index);
            x += width;
        }
    };

    if (candidates_.empty()) {
        const auto status = is_agent_mode() && !agent_response_visible_ && input_buffer_.size() <= 1
            ? (staged_candidates_.empty()
                ? std::wstring_view(L"Agent 模式：继续输入请求")
                : std::wstring_view(L"完整选词后自动发送；Enter/空格直接发送"))
            : is_agent_mode() && agent_response_visible_
            ? (!agent_executing_command_.empty()
                ? std::wstring_view(agent_executing_command_)
                : agent_request_pending_ ? std::wstring_view(L"正在等待 Agent…")
                                         : std::wstring_view(L"暂无候选指令"))
            : input_buffer_.empty() && !staged_candidates_.empty()
            ? std::wstring_view(L"按 Enter 或空格上屏")
            : candidate_status_text(candidate_request_pending_,
                                    candidate_request_failed_,
                                    candidate_failure_detail_);
        render_target_->DrawTextW(
            status.data(), static_cast<UINT32>(status.size()), candidate_text_format_,
            D2D1::RectF(kHorizontalPaddingDip, content_top, candidate_right,
                        content_top + kCandidateItemHeightDip),
            secondary_text_brush_, D2D1_DRAW_TEXT_OPTIONS_CLIP);
    } else if (candidates_expanded_) {
        const auto page_size = std::max<std::size_t>(
            1, static_cast<std::size_t>(candidate_page_size_));
        const auto first_candidate = std::min(
            candidates_.size(), expanded_scroll_row_ * page_size);
        const auto visible_end = std::min(
            candidates_.size(), first_candidate + kExpandedVisibleRows * page_size);
        float top = content_top;
        for (std::size_t begin = first_candidate; begin < visible_end;
             begin += page_size) {
            const auto end = std::min(candidates_.size(), begin + page_size);
            float row_height = kCandidateItemHeightDip;
            for (std::size_t index = begin; index < end; ++index) {
                row_height = std::max(
                    row_height, wrapped_candidate_height(candidates_[index], wrap_length));
            }
            draw_candidate_row(begin, end, top, row_height);
            top += row_height + kCandidateRowGapDip;
        }
    } else {
        float row_height = kCandidateItemHeightDip;
        for (const auto& candidate : candidates_)
            row_height = std::max(row_height,
                                  wrapped_candidate_height(candidate, wrap_length));
        draw_candidate_row(0, candidates_.size(), content_top, row_height);
    }

    const auto draw_button = [this, &target_matches, &add_hit_region,
                              mode_accent_brush, mode_border_brush,
                              mode_highlight_brush](
                                 const D2D1_RECT_F bounds, const HitTarget target,
                                 const std::wstring_view label, const bool enabled) {
        const D2D1_ROUNDED_RECT button{bounds, 7.0F, 7.0F};
        const bool hovered = enabled && target_matches(hovered_target_, target);
        const bool pressed = enabled && target_matches(pressed_target_, target);
        if (hovered || pressed)
            render_target_->FillRoundedRectangle(button, mode_highlight_brush);
        render_target_->DrawRoundedRectangle(button, mode_border_brush, 1.0F);
        render_target_->DrawTextW(
            label.data(), static_cast<UINT32>(label.size()), label_text_format_, bounds,
            enabled ? (hovered || pressed ? mode_accent_brush : text_brush_)
                    : secondary_text_brush_,
            D2D1_DRAW_TEXT_OPTIONS_CLIP);
        if (enabled) add_hit_region(bounds, target);
    };

    float control_x = controls_left;
    if (!candidates_expanded_ || plugin_scheme_selected()) {
        const D2D1_RECT_F previous_bounds =
            D2D1::RectF(control_x, content_top, control_x + kButtonWidthDip,
                        content_top + kCandidateItemHeightDip);
        draw_button(previous_bounds, {HitKind::previous_page, 0}, L"◀",
                    candidate_page_ > 0);
        control_x += kButtonWidthDip + kControlGapDip;
        const D2D1_RECT_F next_bounds =
            D2D1::RectF(control_x, content_top, control_x + kButtonWidthDip,
                        content_top + kCandidateItemHeightDip);
        draw_button(next_bounds, {HitKind::next_page, 0}, L"▶",
                    candidate_request_pending_ || has_more_candidates_);
        control_x += kButtonWidthDip + kControlGapDip;
    }
    const D2D1_RECT_F expand_bounds =
        D2D1::RectF(control_x, content_top, control_x + kExpandButtonWidthDip,
                    content_top + kCandidateItemHeightDip);
    draw_button(expand_bounds, {HitKind::toggle_expanded, 0},
                candidates_expanded_ ? L"收起" : L"展开",
                !candidate_request_pending_ &&
                    (!candidates_.empty() || candidates_expanded_));

    const HRESULT result = render_target_->EndDraw();
    if (FAILED(result)) {
        discard_device_resources();
        if (candidate_window_ != nullptr) InvalidateRect(candidate_window_, nullptr, FALSE);
        return false;
    }
    return present_candidate_surface();
}

LRESULT CALLBACK TextService::window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* service = reinterpret_cast<TextService*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        const auto* creation = reinterpret_cast<CREATESTRUCTW*>(lparam);
        service = static_cast<TextService*>(creation->lpCreateParams);
        SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(service));
    }
    static const UINT candidate_theme_changed =
        RegisterWindowMessageW(kCandidateThemeChangedMessageName);
    if (message == candidate_theme_changed && service != nullptr) {
        service->next_candidate_theme_refresh_ = 0;
        service->refresh_candidate_theme(true);
        return 0;
    }
    if (message == kCandidateReady && service != nullptr) {
        service->handle_candidate_result(reinterpret_cast<CandidateResult*>(lparam));
        return 0;
    }
    if (message == WM_TIMER && wparam == kPluginInputTimer && service != nullptr) {
        service->poll_plugin_input(); return 0;
    }
    if (message == WM_TIMER && wparam == kCandidateWindowOrderTimer && service != nullptr) {
        service->maintain_candidate_window_order(); return 0;
    }
    if (message == kPluginCommitReady && service != nullptr) {
        service->finish_plugin_input_on_ui(); return 0;
    }
    if (message == kAgentReady && service != nullptr) {
        service->handle_agent_result(reinterpret_cast<AgentResult*>(lparam));
        return 0;
    }
    if (message == kLanguageShortcutPressed && service != nullptr) {
        service->toggle_language_mode_from_window();
        return 0;
    }
    if (message == kAltCodeDigitPressed && service != nullptr) {
        service->append_alt_code_digit(static_cast<unsigned>(wparam));
        return 0;
    }
    if (message == kAltCodeFinished && service != nullptr) {
        service->commit_alt_code_preview_from_window();
        return 0;
    }
    if (message == WM_NCHITTEST && service != nullptr &&
        window == service->candidate_shadow_window_) return HTTRANSPARENT;
    if (message == WM_NCHITTEST && service != nullptr &&
        window == service->candidate_window_ &&
        service->candidate_present_geometry_valid_) {
        const POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
        const int local_x = point.x - service->candidate_present_position_.x;
        const int local_y = point.y - service->candidate_present_position_.y;
        if (local_x < 0 || local_y < 0 ||
            local_x >= service->candidate_content_size_.cx ||
            local_y >= service->candidate_content_size_.cy)
            return HTTRANSPARENT;
    }
    if (message == WM_PAINT && service != nullptr) {
        PAINTSTRUCT paint{};
        BeginPaint(window, &paint);
        if (window == service->candidate_window_)
            static_cast<void>(service->render_candidate_window());
        EndPaint(window, &paint);
        return 0;
    }
    if (message == WM_ERASEBKGND && service != nullptr &&
        window == service->candidate_window_) {
        return 1;
    }
    if (message == WM_MOUSEACTIVATE && service != nullptr &&
        window == service->candidate_window_) {
        return MA_NOACTIVATE;
    }
    if (message == WM_MOUSEWHEEL && service != nullptr &&
        window == service->candidate_window_) {
        const int delta = GET_WHEEL_DELTA_WPARAM(wparam);
        const int notches = std::max(1, (delta < 0 ? -delta : delta) / WHEEL_DELTA);
        if (service->alt_code_preview_active_) {
            if (service->shortcut_config_.alt_code_preview_wheel_enabled)
                service->change_alt_code_preview_page(
                    delta > 0 ? -notches : notches);
            return 0;
        }
        if (service->unicode_palette_visible_) {
            POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
            ScreenToClient(window, &point);
            const UINT dpi = GetDpiForWindow(window);
            const int category_top = dips_to_pixels(kHeaderHeightDip + 8.0F, dpi);
            const int category_bottom = dips_to_pixels(
                kHeaderHeightDip + 8.0F + kUnicodeCategoryHeightDip, dpi);
            if (point.y >= category_top && point.y <= category_bottom)
                service->scroll_unicode_categories(
                    -static_cast<float>(delta) * 48.0F /
                    static_cast<float>(WHEEL_DELTA));
            else
                service->scroll_unicode_palette_rows(
                    delta > 0 ? -notches : notches);
            return 0;
        }
        service->scroll_expanded_candidates(delta > 0 ? -notches : notches);
        return 0;
    }
    if (message == WM_MOUSEMOVE && service != nullptr &&
        window == service->candidate_window_) {
        const POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
        service->unicode_hover_point_ = point;
        const auto target = service->hit_test(point);
        if (target != service->hovered_target_) {
            const bool old_cursor = service->hovered_target_.has_value() &&
                                    service->hovered_target_->kind ==
                                        HitKind::pinyin_cursor;
            const bool new_cursor = target.has_value() &&
                                    target->kind == HitKind::pinyin_cursor;
            service->hovered_target_ = target;
            if (!(old_cursor && new_cursor))
                InvalidateRect(window, nullptr, FALSE);
        } else if (service->unicode_palette_visible_ && target.has_value() &&
                   target->kind == HitKind::unicode_item) {
            InvalidateRect(window, nullptr, FALSE);
        }
        TRACKMOUSEEVENT tracking{sizeof(tracking), TME_LEAVE, window, 0};
        TrackMouseEvent(&tracking);
        return 0;
    }
    if (message == WM_MOUSELEAVE && service != nullptr &&
        window == service->candidate_window_) {
        service->hovered_target_.reset();
        InvalidateRect(window, nullptr, FALSE);
        return 0;
    }
    if (message == WM_LBUTTONDOWN && service != nullptr &&
        window == service->candidate_window_) {
        const POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
        service->pressed_target_ = service->hit_test(point);
        if (service->pressed_target_) SetCapture(window);
        InvalidateRect(window, nullptr, FALSE);
        return 0;
    }
    if (message == WM_LBUTTONUP && service != nullptr &&
        window == service->candidate_window_) {
        const POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
        const auto released_target = service->hit_test(point);
        const auto pressed_target = service->pressed_target_;
        service->pressed_target_.reset();
        if (GetCapture() == window) ReleaseCapture();
        InvalidateRect(window, nullptr, FALSE);
        if (pressed_target && released_target && *pressed_target == *released_target)
            service->invoke_hit_target(*released_target);
        return 0;
    }
    if (message == WM_CAPTURECHANGED && service != nullptr &&
        window == service->candidate_window_) {
        service->pressed_target_.reset();
        InvalidateRect(window, nullptr, FALSE);
        return 0;
    }
    if (message == WM_SETCURSOR && service != nullptr &&
        window == service->candidate_window_ && LOWORD(lparam) == HTCLIENT) {
        POINT point{};
        GetCursorPos(&point);
        ScreenToClient(window, &point);
        const auto target = service->hit_test(point);
        if (target) {
            SetCursor(LoadCursorW(
                nullptr, target->kind == HitKind::pinyin_cursor ? IDC_IBEAM : IDC_HAND));
            return TRUE;
        }
    }
    if (message == WM_SIZE && service != nullptr &&
        window == service->candidate_window_ && service->render_target_ != nullptr) {
        if (service->candidate_resize_in_progress_) return 0;
        service->blurred_candidate_background_.clear();
        service->discard_device_resources();
        service->candidate_present_size_ = {
            static_cast<LONG>(LOWORD(lparam)),
            static_cast<LONG>(HIWORD(lparam))};
        InvalidateRect(window, nullptr, FALSE);
        return 0;
    }
    if (message == WM_DPICHANGED && service != nullptr &&
        window == service->candidate_window_) {
        const auto* suggested = reinterpret_cast<const RECT*>(lparam);
        service->candidate_present_position_ = {suggested->left, suggested->top};
        service->candidate_present_size_ = {
            suggested->right - suggested->left,
            suggested->bottom - suggested->top};
        service->candidate_present_geometry_valid_ = true;
        service->blurred_candidate_background_.clear();
        service->discard_device_resources();
        service->update_candidate_window();
        return 0;
    }
    if ((message == WM_THEMECHANGED || message == WM_SETTINGCHANGE ||
         message == WM_DWMCOLORIZATIONCOLORCHANGED) && service != nullptr) {
        service->update_language_bar();
        if (window == service->candidate_window_) {
            service->apply_candidate_window_effects();
            InvalidateRect(window, nullptr, FALSE);
        }
        return 0;
    }
    if (message == WM_DISPLAYCHANGE && service != nullptr &&
        window == service->candidate_window_) {
        service->blurred_candidate_background_.clear();
        service->discard_device_resources();
        InvalidateRect(window, nullptr, FALSE);
        return 0;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

LRESULT CALLBACK TextService::keyboard_hook_proc(const int code, const WPARAM key,
                                                  const LPARAM flags) {
    auto* service = keyboard_hook_service;
    if (code >= 0 && service != nullptr) {
        const auto bits = static_cast<ULONG_PTR>(flags);
        const bool released = (bits & (1ULL << 31)) != 0;
        const bool repeated = (bits & (1ULL << 30)) != 0;
        const bool alt_context = (bits & (1ULL << 29)) != 0 || key_down(VK_MENU);
        if (key >= VK_NUMPAD0 && key <= VK_NUMPAD9) {
            const auto digit = static_cast<unsigned>(key - VK_NUMPAD0);
            const auto digit_mask = static_cast<std::uint16_t>(1U << digit);
            if (released) {
                service->alt_code_numpad_down_mask_ &=
                    static_cast<std::uint16_t>(~digit_mask);
            } else if (!repeated && alt_context && !key_down(VK_CONTROL) &&
                       (service->alt_code_numpad_down_mask_ & digit_mask) == 0) {
                service->alt_code_numpad_down_mask_ |= digit_mask;
                service->refresh_shortcut_config();
                if (service->shortcut_config_.alt_code_preview_enabled &&
                    service->message_window_ != nullptr)
                    PostMessageW(service->message_window_, kAltCodeDigitPressed,
                                 digit, 0);
            }
        } else if (is_alt_key(key) && released && service->message_window_ != nullptr) {
            service->alt_code_numpad_down_mask_ = 0;
            PostMessageW(service->message_window_, kAltCodeFinished, 0, 0);
        }
    }
    if (code >= 0 && service != nullptr && key == VK_SPACE) {
        const auto bits = static_cast<ULONG_PTR>(flags);
        const bool released = (bits & (1ULL << 31)) != 0;
        const bool repeated = (bits & (1ULL << 30)) != 0;
        if (!released) {
            service->refresh_shortcut_config();
            if (service->shortcut_config_.language_shortcut_enabled &&
                service->shortcut_matches(service->shortcut_config_.language_shortcuts,
                                           VK_SPACE)) {
                service->language_space_key_suppressed_ = true;
                if (!repeated && service->message_window_ != nullptr)
                    PostMessageW(service->message_window_, kLanguageShortcutPressed, 0, 0);
                return 1;
            }
        } else if (service->language_space_key_suppressed_) {
            service->language_space_key_suppressed_ = false;
            return 1;
        }
    }
    return CallNextHookEx(service == nullptr ? nullptr : service->keyboard_hook_,
                          code, key, flags);
}

void TextService::queue_candidate_request() {
    if (plugin_scheme_selected()) { queue_plugin_compose(); return; }
    schedule_candidate_request(true);
}

bool TextService::is_agent_mode() const noexcept {
    return !plugin_scheme_selected() && ((!input_buffer_.empty() && input_buffer_.front() == L'v') ||
           !agent_session_id_.empty());
}

std::wstring TextService::agent_preview_text() const {
    if (!is_agent_mode() || !agent_response_visible_) return {};
    std::wstring preview = L"v + ";
    preview.append(agent_message_);
    preview.append(L"    [");
    preview.append(agent_status_label(agent_status_));
    preview.push_back(L']');
    return preview;
}

void TextService::queue_agent_action(const agent::Action action,
                                     std::string command_id,
                                     const std::uint32_t page,
                                     const std::uint64_t task_revision,
                                     std::vector<agent::SlotUpdate> slot_updates) {
    if (action != agent::Action::cancel && !agent_origin_valid_) {
        trace_tsf_event("origin.action_denied", E_ACCESSDENIED);
        const bool safety_blocked = agent_origin_safety_blocked_;
        cancel_agent_session();
        agent_status_ = agent::Status::error;
        agent_message_ = safety_blocked
            ? L"当前输入框未通过安全属性检查，暂不支持 Agent，请改用 Agent 界面"
            : L"输入位置已变化或无法安全读取，请重新选择文字后开始";
        agent_response_visible_ = true;
        update_candidate_window();
        return;
    }
    if (agent_session_id_.empty()) {
        agent_session_id_ = new_agent_session_id();
        agent_last_request_id_.clear();
        agent_state_revision_ = 0;
    }
    if (agent_session_id_.empty()) {
        agent_status_ = agent::Status::error;
        agent_message_ = L"无法创建 Agent 会话";
        agent_response_visible_ = true;
        update_candidate_window();
        return;
    }
    agent::Request request;
    request.action = action;
    request.session_id = agent_session_id_;
    const auto request_id = next_request_id_++;
    request.request_id = "req-" + std::to_string(request_id);
    request.parent_request_id = agent_last_request_id_;
    request.idempotency_key = request.request_id;
    request.capabilities = {"protocol.negotiation", "input.structured",
                            "context.entries", "context.privacy",
                            "commands.risk", "errors.structured", "state.progress",
                            "tasks.slots", "tasks.incremental",
                            agent::kAgentCapabilityOutputEffects,
                            agent::kAgentCapabilityInputSelection};
    request.required_features = {"protocol.negotiation", "input.structured",
                                 "commands.risk", "errors.structured", "tasks.slots"};
    if (action == agent::Action::submit) {
        std::wstring input = staged_text();
        if (input_buffer_.size() > 1)
            input.append(std::wstring_view(input_buffer_).substr(1));
        request.user_input = utf8_from_wide(input);
        agent_last_submitted_pinyin_.clear();
        for (const auto& fragment : staged_candidates_)
            agent_last_submitted_pinyin_.append(fragment.source_input);
        if (input_buffer_.size() > 1)
            agent_last_submitted_pinyin_.append(
                std::wstring_view(input_buffer_).substr(1));
        std::wstring segmented = segmented_input_;
        if (!segmented.empty() && segmented.front() == L'v') segmented.erase(0, 1);
        std::wstring pending = input_buffer_.size() > 1
            ? std::wstring(std::wstring_view(input_buffer_).substr(1)) : std::wstring{};
        request.input_details.raw_pinyin = utf8_from_wide(agent_last_submitted_pinyin_);
        request.input_details.segmented_pinyin = utf8_from_wide(segmented);
        request.input_details.selected_text = utf8_from_wide(staged_text());
        request.input_details.pending_pinyin = utf8_from_wide(pending);
        request.input_details.natural_language = request.user_input;
        request.input_details.input_mode = "agent";
        request.input_details.correction_enabled = correction_enabled_;
    }
    if (action == agent::Action::submit) {
        request.application = current_agent_application(agent_context_id_, agent_context_sensitive_);
        if (!request.application.sensitive_input) {
            const auto filtered = agent::filter_sensitive_text(
                utf8_from_wide(recent_agent_context_), 4096);
            if (filtered.ok) request.session_context = filtered.value;
            if (!request.session_context.empty()) {
                request.context_entries.push_back({
                    agent_context_id_, request.application, request.session_context,
                    GetTickCount64(), "current_input", "owo_commit",
                    current_unix_time_ms(), 1000, true, true, filtered.redacted, false});
            }
        }
    } else if (action == agent::Action::select) {
        // Identity only: lets effects produced by this turn bind to the current position.
        request.application = current_agent_application(agent_context_id_, agent_context_sensitive_);
    }
    request.application = current_agent_application(agent_context_id_, agent_context_sensitive_);
    request.selection = agent_origin_selection_;
    request.command_id = std::move(command_id);
    request.page = page;
    request.task_revision = task_revision != 0 ? task_revision : agent_task_.revision;
    request.slot_updates = std::move(slot_updates);
    const auto encoded = agent::encode_request(request);
    if (encoded.empty()) {
        agent_status_ = agent::Status::error;
        agent_message_ = L"Agent 请求为空或格式无效";
        agent_response_visible_ = true;
        update_candidate_window();
        return;
    }
    ++context_generation_;
    {
        std::lock_guard lock(request_mutex_);
        pending_request_ = PendingRequest{
            static_cast<std::uint8_t>(protocol::MessageType::agent_request),
            request_id, context_generation_, 0, wide_from_utf8(encoded), false, true,
            {}, {}};
    }
    active_candidate_request_id_ = request_id;
    latest_candidate_request_id_.store(request_id, std::memory_order_release);
    agent_last_request_id_ = request.request_id;
    if (action == agent::Action::submit) agent_request_submitted_ = true;
    agent_request_pending_ = action != agent::Action::cancel;
    agent_status_ = action == agent::Action::cancel
        ? agent::Status::completed : agent::Status::thinking;
    if (action != agent::Action::poll) {
        agent_message_.clear();
        agent_executing_command_.clear();
        agent_commands_.clear();
        candidates_.clear();
        candidate_consumed_.clear();
        candidate_groups_.clear();
        agent_response_visible_ = true;
    }
    request_ready_.notify_one();
    update_candidate_window();
}

void TextService::apply_agent_task(const agent::TaskDraft& task) {
    agent_task_ = task;
    if (task.intent.empty() || agent_last_submitted_pinyin_.empty()) return;
    std::wstring remaining;
    for (const auto& range : task.unconsumed_ranges) {
        if (range.start >= range.end || range.end > agent_last_submitted_pinyin_.size()) continue;
        remaining.append(std::wstring_view(agent_last_submitted_pinyin_).substr(
            range.start, range.end - range.start));
    }
    input_buffer_ = L"v" + remaining;
    input_cursor_ = input_buffer_.size();
    segmented_input_ = remaining;
    staged_candidates_.clear();
    staged_cursor_ = 0;
    learning_chain_text_.clear();
    learning_chain_input_.clear();
    learning_chain_context_.clear();
}

bool TextService::select_agent_command(const std::size_t index) {
    if (index >= agent_commands_.size()) return false;
    const auto command = agent_commands_[index];
    if (command.high_risk) return false;
    if (command.effect) {
        const auto outcome = apply_agent_effect(*command.effect);
        if (outcome == NativeWriteResult::completed) {
            cancel_agent_session();
            clear_composition();
            update_candidate_window();
            return true;
        }
        std::wstring preview = wide_from_utf8(command.effect->text);
        if (preview.size() > 200) preview = preview.substr(0, 199) + L"…";
        agent_status_ = agent::Status::agent_mode;
        const bool copy_failed = command.effect->kind == agent::EffectKind::copy_text;
        agent_message_ = copy_failed
            ? (outcome == NativeWriteResult::uncertain
                ? L"复制未完成，剪贴板内容可能已改变；结果仍保留，可重试。内容："
                : L"复制未完成，未修改剪贴板；结果仍保留，可稍后重试。内容：") + preview
            : outcome == NativeWriteResult::unsupported
                ? L"当前输入框暂不支持这类文本的原位替换，原文未改。内容：" + preview
                : L"无法确认写入或目标已失效，请检查原文，勿重复操作。内容：" + preview;
        agent_commands_.clear();
        candidates_.clear(); candidate_consumed_.clear(); candidate_groups_.clear();
        candidate_page_ = 0; has_more_candidates_ = false;
        const auto copy = copy_failed && agent_effects_negotiated_
            ? std::optional<agent::Command>(command)
            : native_writeback_copy_option(outcome, *command.effect, agent_effects_negotiated_);
        if (copy) {
            agent_commands_.push_back(*copy);
            candidates_.push_back(wide_from_utf8(copy->label));
            candidate_consumed_.push_back(1); candidate_groups_.push_back(0);
        }
        agent_response_visible_ = true;
        update_candidate_window();
        return false;
    }
    if ((command.commit_task || !command.slot_updates.empty()) &&
        (agent_task_.intent.empty() || command.task_revision != agent_task_.revision)) {
        agent_status_ = agent::Status::error;
        agent_message_ = L"任务状态已更新，请重新选择";
        agent_response_visible_ = true;
        update_candidate_window();
        return false;
    }
    if (!command.slot_updates.empty()) {
        auto updated = agent_task_;
        for (const auto& patch : command.slot_updates) {
            auto slot = std::find_if(updated.slots.begin(), updated.slots.end(),
                [&](const auto& item) { return item.id == patch.slot_id; });
            if (slot != updated.slots.end() && slot->locked && slot->value != patch.value) {
                agent_status_ = agent::Status::error;
                agent_message_ = L"该任务槽位已经锁定";
                agent_response_visible_ = true;
                update_candidate_window();
                return false;
            }
            if (slot == updated.slots.end()) {
                updated.slots.push_back({patch.slot_id, patch.slot_id, patch.value,
                                         patch.consumed_ranges, patch.lock, 1000});
            } else {
                slot->value = patch.value;
                slot->source_ranges = patch.consumed_ranges;
                slot->locked = patch.lock;
                slot->confidence_milli = 1000;
            }
            for (const auto& consumed : patch.consumed_ranges) {
                std::vector<agent::PinyinRange> remainder;
                for (const auto& range : updated.unconsumed_ranges) {
                    if (consumed.end <= range.start || consumed.start >= range.end) {
                        remainder.push_back(range);
                        continue;
                    }
                    if (range.start < consumed.start)
                        remainder.push_back({range.start, consumed.start});
                    if (consumed.end < range.end)
                        remainder.push_back({consumed.end, range.end});
                }
                updated.unconsumed_ranges = std::move(remainder);
            }
        }
        updated.revision = command.task_revision + 1;
        apply_agent_task(updated);
    }
    queue_agent_action(agent::Action::select, command.id, 0,
                       command.task_revision, command.slot_updates);
    return true;
}

void TextService::cancel_agent_session() {
    reset_agent_origin();
    if (agent_session_id_.empty()) return;
    agent::Request cancel;
    cancel.action = agent::Action::cancel;
    cancel.session_id = agent_session_id_;
    const auto request_id = next_request_id_++;
    cancel.request_id = "req-" + std::to_string(request_id);
    cancel.parent_request_id = agent_last_request_id_;
    cancel.idempotency_key = cancel.request_id;
    cancel.capabilities = {"protocol.negotiation", "input.structured",
                           "context.entries", "context.privacy", "commands.risk",
                           "errors.structured", "state.progress", "tasks.slots",
                           "tasks.incremental", agent::kAgentCapabilityOutputEffects,
                           agent::kAgentCapabilityInputSelection};
    cancel.required_features = {"protocol.negotiation", "errors.structured"};
    const auto encoded = agent::encode_request(cancel);
    if (!encoded.empty()) {
        std::lock_guard lock(request_mutex_);
        feedback_requests_.push_back(PendingRequest{
            static_cast<std::uint8_t>(protocol::MessageType::agent_request),
            request_id, context_generation_, 0, wide_from_utf8(encoded),
            false, true, {}, {}});
        request_ready_.notify_one();
    }
    agent_session_id_.clear();
    agent_last_request_id_.clear();
    agent_state_revision_ = 0;
    agent_task_ = {};
    agent_request_pending_ = false;
    agent_response_visible_ = false;
    agent_effects_negotiated_ = false;
    agent_message_.clear();
    agent_executing_command_.clear();
    agent_commands_.clear();
}

void TextService::refresh_shortcut_config(const bool force) {
    const auto now = GetTickCount64();
    if (!force && shortcut_config_initialized_ && now < next_shortcut_config_refresh_) return;
    next_shortcut_config_refresh_ = now + kShortcutConfigRefreshIntervalMs;
    refresh_candidate_theme(force);
    if (!shortcut_config_initialized_) {
        const auto path = config::default_config_path();
        if (path.empty()) return;
        const auto loaded = shortcut_config_store_.load(path);
        if (!loaded.success) return;
        shortcut_config_initialized_ = true;
        shortcut_config_ = shortcut_config_store_.snapshot();
        if (shortcut_config_.input_scheme != "full_pinyin")
            correction_enabled_ = true;
        chinese_punctuation_enabled_ = shortcut_config_.chinese_punctuation_enabled;
        sync_preserved_language_key();
        return;
    }
    const auto reloaded = shortcut_config_store_.reload();
    if (reloaded.success) {
        const bool configured_punctuation = shortcut_config_.chinese_punctuation_enabled;
        const auto configured_scheme = shortcut_config_.input_scheme;
        shortcut_config_ = shortcut_config_store_.snapshot();
        if (configured_scheme != shortcut_config_.input_scheme) clear_composition();
        if (configured_punctuation != shortcut_config_.chinese_punctuation_enabled)
            chinese_punctuation_enabled_ = shortcut_config_.chinese_punctuation_enabled;
        if (!shortcut_config_.alt_code_preview_enabled)
            finish_alt_code_preview();
        if (configured_scheme != shortcut_config_.input_scheme &&
            shortcut_config_.input_scheme != "full_pinyin")
            correction_enabled_ = true;
        sync_preserved_language_key();
    }
}

void TextService::refresh_candidate_theme(const bool force) {
    const auto now = GetTickCount64();
    if (!force && now < next_candidate_theme_refresh_) return;
    next_candidate_theme_refresh_ = now + 1000;
    const auto data_root = config::local_data_root();
    const auto loaded = plugin::load_active_candidate_theme(
        data_root.empty() ? std::filesystem::path{} : data_root / L"plugins");
    const auto unicode_loaded = plugin::load_active_unicode_palette(
        data_root.empty() ? std::filesystem::path{} : data_root / L"plugins");
    const plugin::CandidateTheme next = loaded.active ? loaded.theme
                                                       : plugin::CandidateTheme{};
    const auto source = loaded.active ? loaded.plugin_id + "@" + loaded.version
                                      : std::string{};
    const auto unicode_source = unicode_loaded.active
        ? unicode_loaded.plugin_id + "@" + unicode_loaded.version : std::string{};
    const bool unicode_changed = unicode_loaded.palette != unicode_palette_ ||
                                 unicode_source != unicode_palette_source_;
    if (unicode_changed) {
        unicode_palette_ = unicode_loaded.active
            ? unicode_loaded.palette : plugin::UnicodePalette{};
        unicode_palette_source_ = unicode_source;
        unicode_category_index_ = 0;
        unicode_scroll_row_ = 0;
        unicode_category_scroll_offset_dip_ = 0.0F;
        if (unicode_palette_.categories.empty()) unicode_palette_visible_ = false;
    }
    if (next == candidate_theme_ && source == candidate_theme_source_) {
        if (unicode_changed) update_candidate_window();
        return;
    }
    candidate_theme_ = next;
    candidate_theme_source_ = source;
    blurred_candidate_background_.clear();
    discard_device_resources();
    if (candidate_window_ != nullptr) {
        update_candidate_shadow();
        InvalidateRect(candidate_window_, nullptr, FALSE);
    }
}

void TextService::sync_preserved_language_key() {
    if (thread_manager_ == nullptr || client_id_ == TF_CLIENTID_NULL) return;
    const std::vector<std::string> desired = shortcut_config_.language_shortcut_enabled
                                    ? shortcut_config_.language_shortcuts
                                    : std::vector<std::string>{};
    if (desired == preserved_language_shortcuts_) return;
    clear_preserved_language_key();
    if (desired.empty()) return;
    ITfKeystrokeMgr* manager = nullptr;
    if (FAILED(thread_manager_->QueryInterface(IID_PPV_ARGS(&manager)))) return;
    constexpr wchar_t description[] = L"OwO 切换中英文输入";
    for (const auto& shortcut : desired) {
        TF_PRESERVEDKEY key{};
        if (!preserved_shortcut(shortcut, key)) continue;
        if (SUCCEEDED(manager->PreserveKey(
                client_id_, kLanguageModePreservedKeyGuid, &key, description,
                static_cast<ULONG>(std::size(description) - 1)))) {
            preserved_language_keys_.push_back(key);
        }
    }
    manager->Release();
    if (!preserved_language_keys_.empty()) preserved_language_shortcuts_ = desired;
}

void TextService::clear_preserved_language_key() noexcept {
    if (preserved_language_keys_.empty() || thread_manager_ == nullptr) return;
    ITfKeystrokeMgr* manager = nullptr;
    if (SUCCEEDED(thread_manager_->QueryInterface(IID_PPV_ARGS(&manager)))) {
        for (auto& key : preserved_language_keys_)
            manager->UnpreserveKey(kLanguageModePreservedKeyGuid, &key);
        manager->Release();
    }
    preserved_language_keys_.clear();
    preserved_language_shortcuts_.clear();
}

void TextService::initialize_language_compartment() {
    if (thread_manager_ == nullptr ||
        language_compartment_sink_cookie_ != TF_INVALID_COOKIE) return;
    ITfCompartmentMgr* compartments = nullptr;
    if (FAILED(thread_manager_->QueryInterface(IID_PPV_ARGS(&compartments)))) return;
    ITfCompartment* compartment = nullptr;
    const HRESULT found = compartments->GetCompartment(
        GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, &compartment);
    compartments->Release();
    if (FAILED(found) || compartment == nullptr) return;

    // OwO starts in Chinese mode whenever a new TSF instance is activated.
    // Do not inherit a host process's stale closed value: applications often
    // retain this compartment after another IME was last used in English.
    VARIANT initial_mode{};
    initial_mode.vt = VT_I4;
    initial_mode.lVal = 1;
    const HRESULT initialized = compartment->SetValue(client_id_, &initial_mode);
    if (SUCCEEDED(initialized)) chinese_mode_ = true;

    ITfSource* source = nullptr;
    if (SUCCEEDED(compartment->QueryInterface(IID_PPV_ARGS(&source)))) {
        source->AdviseSink(IID_ITfCompartmentEventSink,
                           static_cast<ITfCompartmentEventSink*>(this),
                           &language_compartment_sink_cookie_);
        source->Release();
    }
    compartment->Release();
    if (FAILED(initialized)) {
        const auto mode = system_language_mode();
        if (mode.has_value()) chinese_mode_ = *mode;
    }
}

void TextService::uninitialize_language_compartment() noexcept {
    if (thread_manager_ == nullptr ||
        language_compartment_sink_cookie_ == TF_INVALID_COOKIE) return;
    ITfCompartmentMgr* compartments = nullptr;
    if (SUCCEEDED(thread_manager_->QueryInterface(IID_PPV_ARGS(&compartments)))) {
        ITfCompartment* compartment = nullptr;
        if (SUCCEEDED(compartments->GetCompartment(
                GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, &compartment)) &&
            compartment != nullptr) {
            ITfSource* source = nullptr;
            if (SUCCEEDED(compartment->QueryInterface(IID_PPV_ARGS(&source)))) {
                source->UnadviseSink(language_compartment_sink_cookie_);
                source->Release();
            }
            compartment->Release();
        }
        compartments->Release();
    }
    language_compartment_sink_cookie_ = TF_INVALID_COOKIE;
}

std::optional<bool> TextService::system_language_mode() const {
    if (thread_manager_ == nullptr) return std::nullopt;
    ITfCompartmentMgr* compartments = nullptr;
    if (FAILED(thread_manager_->QueryInterface(IID_PPV_ARGS(&compartments))))
        return std::nullopt;
    ITfCompartment* compartment = nullptr;
    const HRESULT found = compartments->GetCompartment(
        GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, &compartment);
    compartments->Release();
    if (FAILED(found) || compartment == nullptr) return std::nullopt;
    VARIANT value{};
    const HRESULT read = compartment->GetValue(&value);
    compartment->Release();
    if (FAILED(read) || value.vt != VT_I4) return std::nullopt;
    return value.lVal != 0;
}

void TextService::initialize_language_bar() {
    if (thread_manager_ == nullptr || language_bar_item_ != nullptr) return;
    ITfLangBarItemMgr* manager = nullptr;
    if (FAILED(thread_manager_->QueryInterface(IID_PPV_ARGS(&manager)))) return;
    auto* item = new (std::nothrow) LanguageBarItem(this);
    if (item != nullptr) {
        if (SUCCEEDED(manager->AddItem(item))) language_bar_item_ = item;
        else item->Release();
    }
    manager->Release();
}

void TextService::uninitialize_language_bar() noexcept {
    if (language_bar_item_ == nullptr) return;
    if (thread_manager_ != nullptr) {
        ITfLangBarItemMgr* manager = nullptr;
        if (SUCCEEDED(thread_manager_->QueryInterface(IID_PPV_ARGS(&manager)))) {
            manager->RemoveItem(language_bar_item_);
            manager->Release();
        }
    }
    language_bar_item_->Release();
    language_bar_item_ = nullptr;
}

void TextService::update_language_bar() noexcept {
    if (language_bar_item_ != nullptr) language_bar_item_->notify_mode_changed();
}

bool TextService::shortcut_matches(const std::vector<std::string>& shortcuts,
                                   const WPARAM key) const {
    const auto event = shortcut_for_key_event(key);
    return std::find(shortcuts.begin(), shortcuts.end(), event) != shortcuts.end();
}

void TextService::schedule_candidate_request(const bool reset_retry) {
    const bool agent_mode = is_agent_mode();
    const std::wstring request_input = agent_mode && !input_buffer_.empty()
        ? input_buffer_.substr(1) : input_buffer_;
    if (request_input.empty()) {
        candidate_request_pending_ = false;
        update_candidate_window();
        return;
    }
    candidate_request_pending_ = true;
    candidate_request_failed_ = false;
    candidate_failure_detail_.clear();
    if (reset_retry) candidate_retry_count_ = 0;
    clear_deferred_candidate_selection();
    if (reset_retry) clear_deferred_punctuation();
    hovered_target_.reset();
    pressed_target_.reset();
    hit_regions_.clear();
    if (!candidates_expanded_) expanded_scroll_row_ = 0;
    {
        std::lock_guard lock(request_mutex_);
        if (candidate_cancellation_event_ != nullptr) {
            SetEvent(candidate_cancellation_event_);
            retired_cancellation_events_.push_back(candidate_cancellation_event_);
            candidate_cancellation_event_ = nullptr;
            // The Core opens the event as soon as it receives the request. Keep
            // a generous tail to cover startup/connection races without leaking
            // one kernel handle per keystroke for the lifetime of the host app.
            if (retired_cancellation_events_.size() > 32) {
                CloseHandle(retired_cancellation_events_.front());
                retired_cancellation_events_.erase(retired_cancellation_events_.begin());
            }
        }
        active_candidate_request_id_ = next_request_id_++;
        latest_candidate_request_id_.store(active_candidate_request_id_,
                                           std::memory_order_release);
        const auto event_name = ipc::candidate_cancellation_event_name(
            active_candidate_request_id_, context_generation_);
        candidate_cancellation_event_ = CreateEventW(
            nullptr, TRUE, FALSE, event_name.c_str());
        std::wstring request_context = recent_language_context_;
        append_bounded_language_context(request_context, staged_text());
        std::wstring previous_input = staged_candidates_.empty()
            ? recent_language_input_ : staged_candidates_.back().source_input;
        while (!previous_input.empty() && previous_input.back() == L'\'')
            previous_input.pop_back();
        auto pending = PendingRequest{
            static_cast<std::uint8_t>(protocol::MessageType::candidate_request),
            active_candidate_request_id_, context_generation_, candidate_page_, request_input,
            candidates_expanded_, correction_enabled_, previous_input,
            std::move(request_context)};
        pending.defer_expensive_candidates = candidate_page_ == 0 &&
                                              !candidates_expanded_;
        pending_request_ = std::move(pending);
    }
    request_ready_.notify_one();
    // Draw the edited pinyin immediately but retain the current geometry until
    // Core returns its matching candidate snapshot. Recomputing the layered
    // window size with stale candidates and then again with the response caused
    // two visible resizes for every key press.
    if (candidate_window_ != nullptr && IsWindowVisible(candidate_window_)) {
        RedrawWindow(candidate_window_, nullptr, nullptr,
                     RDW_INVALIDATE | RDW_UPDATENOW | RDW_NOERASE |
                         RDW_NOCHILDREN);
    } else {
        update_candidate_window();
    }
}

void TextService::append_language_context(const std::wstring_view text) {
    append_bounded_language_context(recent_language_context_, text);
    append_bounded_language_context(recent_agent_context_, text, 4096);
    queue_agent_context(std::wstring(text));
}

void TextService::rotate_agent_context() {
    recent_agent_context_.clear();
    agent_context_sensitive_ = true;
    const auto id = new_agent_session_id();
    agent_context_id_ = id.empty()
        ? "ctx-" + std::to_string(GetCurrentProcessId()) + "-" +
              std::to_string(GetTickCount64()) + "-" + std::to_string(next_request_id_)
        : "ctx-" + id;
}

void TextService::queue_agent_context(std::wstring text) {
    if (text.empty()) return;
    if (agent_context_id_.empty()) rotate_agent_context();
    const auto application = current_agent_application(agent_context_id_, agent_context_sensitive_);
    if (application.sensitive_input) return;
    std::lock_guard lock(request_mutex_);
    PendingRequest request;
    request.type = static_cast<std::uint8_t>(
        protocol::MessageType::agent_context_recorded);
    request.request_id = next_request_id_++;
    request.generation = context_generation_;
    request.input = std::move(text);
    request.application_context = application;
    feedback_requests_.push_back(std::move(request));
    request_ready_.notify_one();
}

void TextService::queue_commit_feedback(std::wstring candidate, std::wstring input,
                                        std::wstring context) {
    std::lock_guard lock(request_mutex_);
    feedback_requests_.push_back(PendingRequest{
        static_cast<std::uint8_t>(protocol::MessageType::candidate_committed),
        next_request_id_++, context_generation_, 0, std::move(candidate), false, true,
        std::move(input), std::move(context)});
    request_ready_.notify_one();
}

void TextService::worker_loop(const std::stop_token stop_token) {
    ipc::PersistentPipeClient core_client(ipc::kCorePipeName);
    while (!stop_token.stop_requested()) {
        PendingRequest request{};
        {
            std::unique_lock lock(request_mutex_);
            const bool ready = request_ready_.wait_for(
                lock, stop_token, std::chrono::milliseconds(50), [this] {
                    return pending_request_.has_value() || !feedback_requests_.empty();
                });
            if (stop_token.stop_requested()) break;
            if (!ready) {
                lock.unlock();
                // Reuse the pipe across rapid input and page requests, then
                // release it after a short idle period so another TSF host is
                // never starved by Core's serialized connection loop.
                core_client.reset();
                continue;
            }
            if (pending_request_.has_value()) {
                request = std::move(*pending_request_);
                pending_request_.reset();
            } else {
                request = std::move(feedback_requests_.front());
                feedback_requests_.pop_front();
            }
        }
        const auto request_type = static_cast<protocol::MessageType>(request.type);
        const protocol::Message message{request_type,
                                        request.request_id, request.generation,
                                        utf8_from_wide(request.input)};
        auto paged_message = message;
        paged_message.input = utf8_from_wide(request.feedback_input);
        paged_message.context = utf8_from_wide(request.language_context);
        paged_message.page = request.page;
        paged_message.expanded = request.expanded;
        paged_message.correction_enabled = request.correction_enabled;
        paged_message.defer_expensive_candidates =
            request.defer_expensive_candidates;
        paged_message.application_context_id = request.application_context.context_id;
        paged_message.application_process_id = request.application_context.process_id;
        paged_message.application_thread_id = request.application_context.thread_id;
        paged_message.application_executable = request.application_context.executable;
        paged_message.application_window_class = request.application_context.window_class;
        paged_message.application_focus_window_class =
            request.application_context.focus_window_class;
        paged_message.application_sensitive_input =
            request.application_context.sensitive_input;
        const auto post_candidate_failure = [this, &request, request_type](
                                                std::wstring detail) {
            if (request_type != protocol::MessageType::candidate_request) return;
            auto result = std::make_unique<CandidateResult>();
            result->request_id = request.request_id;
            result->generation = request.generation;
            result->page = request.page;
            result->expanded = request.expanded;
            result->request_failed = true;
            result->failure_detail = std::move(detail);
            if (PostMessageW(message_window_, kCandidateReady, 0,
                             reinterpret_cast<LPARAM>(result.get())))
                result.release();
        };
        const auto post_agent_failure = [this, &request, request_type](
                                             std::wstring detail,
                                             const agent::Status status = agent::Status::error) {
            if (request_type != protocol::MessageType::agent_request) return;
            const auto original = agent::decode_request(utf8_from_wide(request.input));
            if (!original.ok) return;
            auto result = std::make_unique<AgentResult>();
            result->request_id = request.request_id;
            result->generation = request.generation;
            result->transport_failure = true;
            result->response = {original.value.session_id, utf8_from_wide(detail), {}, {},
                                status, 0, false, "ipc_error"};
            if (PostMessageW(message_window_, kAgentReady, 0,
                             reinterpret_cast<LPARAM>(result.get()))) result.release();
        };
        const auto timeout = request_type == protocol::MessageType::candidate_request
            ? candidate_request_timeout(request.input.size())
            : request_type == protocol::MessageType::agent_request
                ? std::chrono::milliseconds(2500) : kFeedbackRequestTimeout;
        const auto exchange_started = std::chrono::steady_clock::now();
        const auto exchanged = core_client.exchange(
            protocol::encode_message(paged_message), timeout);
        const auto exchange_finished = std::chrono::steady_clock::now();
        if (request_type == protocol::MessageType::candidate_request &&
            latest_candidate_request_id_.load(std::memory_order_acquire) !=
                request.request_id) {
            std::clog
                << R"({"process":"tsf","module":"candidate","level":"info","event_id":"candidate_request_superseded","request_id":)"
                << request.request_id
                << R"(,"generation":)" << request.generation
                << R"(,"ipc_us":)"
                << std::chrono::duration_cast<std::chrono::microseconds>(
                       exchange_finished - exchange_started).count()
                << "}\n";
            continue;
        }
        if (!exchanged.status || stop_token.stop_requested()) {
            if (!stop_token.stop_requested()) {
                auto detail = exchanged.status.error == protocol::ErrorCode::timeout
                                  ? std::wstring(L"候选生成稍慢，请继续输入或重试")
                                  : std::wstring(L"候选服务暂不可用");
                post_candidate_failure(std::move(detail));
                if (request_type == protocol::MessageType::agent_request)
                    post_agent_failure(L"Agent IPC 服务不可用", agent::Status::disconnected);
            }
            continue;
        }
        const auto decoded = protocol::decode_message(exchanged.response);
        if (!decoded.validation) {
            if (decoded.validation.error == protocol::ErrorCode::unsupported_protocol)
                core_client.reset();
            auto detail = decoded.validation.error == protocol::ErrorCode::unsupported_protocol
                              ? std::wstring(L"输入法组件版本不一致，请重启 OwO 或重新安装当前版本")
                              : wide_from_utf8(decoded.validation.message);
            if (detail.empty()) detail = L"候选响应协议无效";
            post_candidate_failure(std::move(detail));
            if (request_type == protocol::MessageType::agent_request)
                post_agent_failure(L"Agent 响应格式错误");
            continue;
        }
        if (decoded.message.request_id != request.request_id) {
            core_client.reset();
            post_candidate_failure(L"候选服务响应错位，正在重试");
            if (request_type == protocol::MessageType::agent_request)
                post_agent_failure(L"Agent 会话响应错位");
            continue;
        }
        if (decoded.message.type == protocol::MessageType::error_response) {
            auto detail = wide_from_utf8(decoded.message.text);
            if (detail.empty()) detail = L"候选服务返回错误";
            post_candidate_failure(std::move(detail));
            if (request_type == protocol::MessageType::agent_request)
                post_agent_failure(L"Agent 插件调用失败");
            continue;
        }
        if (request_type == protocol::MessageType::agent_request) {
            const auto original = agent::decode_request(utf8_from_wide(request.input));
            if (!original.ok || decoded.message.type != protocol::MessageType::agent_response) {
                post_agent_failure(L"Agent 响应类型错误");
                continue;
            }
            auto current = agent::decode_response(decoded.message.text);
            if (!current.ok || current.value.session_id != original.value.session_id) {
                post_agent_failure(L"Agent 会话 ID 不匹配");
                continue;
            }
            std::uint64_t poll_revision = 0;
            const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(75);
            while ((current.value.status == agent::Status::connecting ||
                    current.value.status == agent::Status::submitting ||
                    current.value.status == agent::Status::thinking ||
                    current.value.status == agent::Status::executing) &&
                   std::chrono::steady_clock::now() < deadline &&
                   !stop_token.stop_requested()) {
                if (current.value.status == agent::Status::executing ||
                    !current.value.message.empty() ||
                    !current.value.executing_command.empty()) {
                    auto interim = std::make_unique<AgentResult>();
                    interim->request_id = request.request_id;
                    interim->generation = request.generation;
                    interim->response = current.value;
                    if (PostMessageW(message_window_, kAgentReady, 0,
                                     reinterpret_cast<LPARAM>(interim.get()))) interim.release();
                }
                {
                    std::unique_lock lock(request_mutex_);
                    if (request_ready_.wait_for(lock, stop_token,
                            std::chrono::milliseconds(150), [this] {
                                return pending_request_.has_value();
                            })) break;
                }
                const auto poll = agent::make_poll_request(original.value, ++poll_revision);
                const protocol::Message poll_message{
                    protocol::MessageType::agent_request, request.request_id,
                    request.generation, agent::encode_request(poll)};
                const auto polled = core_client.exchange(
                    protocol::encode_message(poll_message), std::chrono::milliseconds(2500));
                if (!polled.status) {
                    post_agent_failure(L"Agent IPC 轮询超时");
                    current.ok = false;
                    break;
                }
                const auto poll_envelope = protocol::decode_message(polled.response);
                if (!poll_envelope.validation ||
                    poll_envelope.message.type != protocol::MessageType::agent_response ||
                    poll_envelope.message.request_id != request.request_id) {
                    post_agent_failure(L"Agent 轮询响应无效");
                    current.ok = false;
                    break;
                }
                current = agent::decode_response(poll_envelope.message.text);
                if (!current.ok || current.value.session_id != original.value.session_id) {
                    post_agent_failure(L"Agent 轮询会话不匹配");
                    current.ok = false;
                    break;
                }
            }
            if (current.ok && std::chrono::steady_clock::now() >= deadline &&
                (current.value.status == agent::Status::connecting ||
                 current.value.status == agent::Status::submitting ||
                 current.value.status == agent::Status::thinking ||
                 current.value.status == agent::Status::executing)) {
                post_agent_failure(L"Agent request timed out; no automatic retry");
                current.ok = false;
            }
            if (current.ok && !stop_token.stop_requested()) {
                auto result = std::make_unique<AgentResult>();
                result->request_id = request.request_id;
                result->generation = request.generation;
                result->response = std::move(current.value);
                if (PostMessageW(message_window_, kAgentReady, 0,
                                 reinterpret_cast<LPARAM>(result.get()))) result.release();
            }
            continue;
        }
        if (request_type == protocol::MessageType::candidate_committed) {
            if (decoded.message.type != protocol::MessageType::acknowledgement) continue;
            continue;
        }
        if (decoded.message.type != protocol::MessageType::candidate_response) {
            post_candidate_failure(L"候选响应类型错误");
            continue;
        }
        if (decoded.message.correction_enabled != request.correction_enabled) {
            post_candidate_failure(L"候选响应纠错模式不匹配");
            continue;
        }
        auto result = std::make_unique<CandidateResult>();
        result->request_id = decoded.message.request_id;
        result->generation = decoded.message.context_generation;
        result->page = decoded.message.page;
        result->has_more = decoded.message.has_more;
        result->expanded = decoded.message.expanded;
        result->page_size = decoded.message.page_size;
        result->refinement_pending =
            decoded.message.candidate_refinement_pending;
        for (const auto& syllable : decoded.message.syllables) {
            auto converted = wide_from_utf8(syllable);
            if (converted.empty()) continue;
            if (!result->segmented_input.empty()) result->segmented_input.push_back(L'\'');
            result->segmented_input += converted;
        }
        result->candidates.reserve(decoded.message.candidates.size());
        result->candidate_consumed.reserve(decoded.message.candidate_consumed.size());
        result->candidate_groups.reserve(decoded.message.candidate_groups.size());
        for (std::size_t index = 0; index < decoded.message.candidates.size(); ++index) {
            auto converted = wide_from_utf8(decoded.message.candidates[index]);
            if (converted.empty()) continue;
            result->candidates.push_back(std::move(converted));
            result->candidate_consumed.push_back(
                decoded.message.candidate_consumed[index]);
            if (!decoded.message.candidate_groups.empty())
                result->candidate_groups.push_back(
                    decoded.message.candidate_groups[index]);
        }
        const auto converted_at = std::chrono::steady_clock::now();
        std::clog
            << R"({"process":"tsf","module":"candidate","level":"info","event_id":"candidate_response_received","request_id":)"
            << request.request_id
            << R"(,"generation":)" << request.generation
            << R"(,"queue_us":)"
            << std::chrono::duration_cast<std::chrono::microseconds>(
                   exchange_started - request.queued_at).count()
            << R"(,"ipc_us":)"
            << std::chrono::duration_cast<std::chrono::microseconds>(
                   exchange_finished - exchange_started).count()
            << R"(,"conversion_us":)"
            << std::chrono::duration_cast<std::chrono::microseconds>(
                   converted_at - exchange_finished).count()
            << R"(,"candidate_count":)" << result->candidates.size()
            << R"(,"refinement_pending":)"
            << (decoded.message.candidate_refinement_pending ? "true" : "false")
            << "}\n";
        if (decoded.message.candidate_refinement_pending &&
            request.defer_expensive_candidates) {
            if (PostMessageW(message_window_, kCandidateReady, 0,
                             reinterpret_cast<LPARAM>(result.get())))
                result.release();
            bool queued_refinement = false;
            {
                std::unique_lock lock(request_mutex_);
                request_ready_.wait_for(
                    lock, stop_token, kCandidateRefinementDebounce,
                    [this, &request] {
                        return pending_request_.has_value() ||
                               latest_candidate_request_id_.load(
                                   std::memory_order_acquire) != request.request_id;
                    });
                if (!stop_token.stop_requested() &&
                    latest_candidate_request_id_.load(std::memory_order_acquire) ==
                        request.request_id && !pending_request_.has_value()) {
                    request.defer_expensive_candidates = false;
                    request.queued_at = std::chrono::steady_clock::now();
                    pending_request_ = std::move(request);
                    queued_refinement = true;
                }
            }
            if (queued_refinement) {
                request_ready_.notify_one();
            } else {
                std::clog
                    << R"({"process":"tsf","module":"candidate","level":"info","event_id":"candidate_refinement_superseded","request_id":)"
                    << request.request_id
                    << R"(,"generation":)" << request.generation << "}\n";
            }
            continue;
        }
        if (!decoded.message.model_pending) {
            if (PostMessageW(message_window_, kCandidateReady, 0,
                             reinterpret_cast<LPARAM>(result.get())))
                result.release();
            continue;
        }

        // Show deterministic base candidates immediately. A later model result
        // is posted as a preserve-paging update, so it atomically replaces only
        // candidate rows without clearing the reading or recreating the window.
        // This removes the model timeout from first-candidate latency.
        if (!PostMessageW(message_window_, kCandidateReady, 0,
                          reinterpret_cast<LPARAM>(result.get())))
            continue;
        result.release();

        for (int attempt = 0; attempt < 1 && !stop_token.stop_requested(); ++attempt) {
            {
                std::unique_lock lock(request_mutex_);
                const bool interrupted = request_ready_.wait_for(
                    lock, stop_token, std::chrono::milliseconds(2), [this] {
                        return pending_request_.has_value() || !feedback_requests_.empty();
                    });
                if (interrupted || stop_token.stop_requested()) break;
            }
            const protocol::Message update_request{
                protocol::MessageType::candidate_update_request,
                request.request_id, request.generation, {}};
            const auto update_exchange = core_client.exchange(
                protocol::encode_message(update_request), std::chrono::milliseconds(90));
            if (!update_exchange.status) break;
            const auto update = protocol::decode_message(update_exchange.response);
            if (!update.validation ||
                update.message.type != protocol::MessageType::candidate_update_response ||
                update.message.request_id != request.request_id ||
                update.message.context_generation != request.generation) break;
            if (update.message.model_pending) continue;
            if (update.message.candidates.empty()) break;
            std::vector<std::wstring> intelligent_candidates;
            std::vector<std::uint64_t> intelligent_consumed;
            std::vector<std::uint64_t> intelligent_groups;
            for (std::size_t index = 0; index < update.message.candidates.size(); ++index) {
                auto converted = wide_from_utf8(update.message.candidates[index]);
                if (converted.empty()) continue;
                intelligent_candidates.push_back(std::move(converted));
                intelligent_consumed.push_back(
                    update.message.candidate_consumed[index]);
                if (!update.message.candidate_groups.empty())
                    intelligent_groups.push_back(
                        update.message.candidate_groups[index]);
            }
            if (!intelligent_candidates.empty() &&
                intelligent_candidates.size() == intelligent_consumed.size()) {
                auto ranked = std::make_unique<CandidateResult>();
                ranked->request_id = request.request_id;
                ranked->generation = request.generation;
                ranked->page = request.page;
                ranked->expanded = request.expanded;
                ranked->preserve_paging = true;
                ranked->candidates = std::move(intelligent_candidates);
                ranked->candidate_consumed = std::move(intelligent_consumed);
                ranked->candidate_groups = std::move(intelligent_groups);
                if (PostMessageW(message_window_, kCandidateReady, 0,
                                 reinterpret_cast<LPARAM>(ranked.get())))
                    ranked.release();
            }
            break;
        }
    }
}

void TextService::handle_candidate_result(CandidateResult* raw_result) {
    std::unique_ptr<CandidateResult> result(raw_result);
    if (result == nullptr || result->generation != context_generation_ ||
        result->request_id != active_candidate_request_id_ ||
        result->page != candidate_page_ || result->expanded != candidates_expanded_ ||
        input_buffer_.empty()) return;
    if (result->request_failed && candidate_retry_count_ == 0) {
        ++candidate_retry_count_;
        schedule_candidate_request(false);
        return;
    }
    candidate_request_pending_ = result->refinement_pending &&
                                 result->candidates.empty();
    candidate_request_failed_ = result->request_failed;
    candidate_failure_detail_ = std::move(result->failure_detail);
    if (!result->request_failed) candidate_retry_count_ = 0;
    candidates_ = std::move(result->candidates);
    candidate_consumed_ = std::move(result->candidate_consumed);
    candidate_groups_ = std::move(result->candidate_groups);
    if (candidate_consumed_.size() != candidates_.size()) {
        candidates_.clear();
        candidate_consumed_.clear();
        candidate_groups_.clear();
    }
    if (!candidate_groups_.empty() && candidate_groups_.size() != candidates_.size())
        candidate_groups_.clear();
    if (!result->preserve_paging && result->page_size >= 1 && result->page_size <= 9)
        candidate_page_size_ = result->page_size;
    if (candidates_expanded_) {
        const auto page_size = std::max<std::size_t>(
            1, static_cast<std::size_t>(candidate_page_size_));
        const auto total_rows = (candidates_.size() + page_size - 1) / page_size;
        const auto maximum_scroll = total_rows > kExpandedVisibleRows
                                        ? total_rows - kExpandedVisibleRows
                                        : 0;
        expanded_scroll_row_ = std::min(expanded_scroll_row_, maximum_scroll);
    }
    if (!result->preserve_paging) {
        has_more_candidates_ = result->has_more;
        if (!result->segmented_input.empty()) {
            if (is_agent_mode() && !input_buffer_.empty() && input_buffer_.front() == L'v')
                segmented_input_ = L"v" + case_preserving_segmented_input(
                    std::wstring_view(input_buffer_).substr(1), result->segmented_input);
            else
                segmented_input_ = case_preserving_segmented_input(
                    input_buffer_, result->segmented_input);
        }
    }
    if (!result->preserve_paging && deferred_page_direction_ != 0) {
        const int movement = std::exchange(deferred_page_direction_, 0);
        if ((movement > 0 && has_more_candidates_) ||
            (movement < 0 && candidate_page_ > 0)) {
            const auto target = std::clamp<std::int64_t>(
                static_cast<std::int64_t>(candidate_page_) + movement,
                0, 100);
            candidate_page_ = static_cast<std::uint64_t>(target);
            has_more_candidates_ = false;
            queue_candidate_request();
            return;
        }
    }
    if (deferred_punctuation_) {
        std::wstring punctuation = std::move(*deferred_punctuation_);
        deferred_punctuation_.reset();
        ITfContext* selected_context =
            std::exchange(deferred_punctuation_context_, nullptr);
        if (selected_context != nullptr) {
            HRESULT committed = E_FAIL;
            if (!candidates_.empty()) {
                committed = commit_candidate(selected_context, 0, punctuation);
            } else {
                std::wstring raw = input_buffer_ + punctuation;
                clear_composition();
                committed = commit_text(selected_context, std::move(raw));
            }
            selected_context->Release();
            if (SUCCEEDED(committed)) return;
        }
    }
    if (deferred_candidate_text_) {
        const auto selected = std::find(candidates_.begin(), candidates_.end(),
                                        *deferred_candidate_text_);
        ITfContext* selected_context =
            std::exchange(deferred_candidate_context_, nullptr);
        deferred_candidate_text_.reset();
        if (selected != candidates_.end() && selected_context != nullptr) {
            const auto index = static_cast<std::size_t>(selected - candidates_.begin());
            const HRESULT committed = is_agent_mode()
                ? stage_agent_candidate(selected_context, index)
                : commit_candidate(selected_context, index);
            selected_context->Release();
            if (SUCCEEDED(committed)) return;
        } else if (selected_context != nullptr) {
            selected_context->Release();
        }
    }
    const auto render_started = std::chrono::steady_clock::now();
    update_candidate_window();
    const auto render_us = std::chrono::duration_cast<std::chrono::microseconds>(
        std::chrono::steady_clock::now() - render_started).count();
    std::clog
        << R"({"process":"tsf","module":"candidate","level":"info","event_id":"candidate_rendered","request_id":)"
        << result->request_id
        << R"(,"generation":)" << result->generation
        << R"(,"render_us":)" << render_us
        << R"(,"candidate_count":)" << candidates_.size()
        << R"(,"model_update":)" << (result->preserve_paging ? "true" : "false")
        << "}\n";
}

void TextService::handle_agent_result(AgentResult* raw_result) {
    std::unique_ptr<AgentResult> result(raw_result);
    if (result == nullptr || result->generation != context_generation_ ||
        result->request_id != active_candidate_request_id_ ||
        agent_session_id_.empty() ||
        result->response.session_id != agent_session_id_) return;
    if (!result->transport_failure && result->response.schema_version >= 3 &&
        result->response.state_revision < agent_state_revision_) return;
    agent_state_revision_ = std::max(
        agent_state_revision_, result->response.state_revision);
    agent_effects_negotiated_ = std::find(
        result->response.capabilities.begin(), result->response.capabilities.end(),
        agent::kAgentCapabilityOutputEffects) != result->response.capabilities.end();
    agent_request_pending_ = result->response.status == agent::Status::thinking ||
                             result->response.status == agent::Status::executing ||
                             result->response.status == agent::Status::connecting ||
                             result->response.status == agent::Status::submitting ||
                             result->response.status == agent::Status::cancelling;
    agent_status_ = result->response.status;
    agent_message_ = wide_from_utf8(result->response.message);
    agent_executing_command_ = wide_from_utf8(result->response.executing_command);
    if (!result->response.task.intent.empty()) apply_agent_task(result->response.task);
    candidates_.clear();
    candidate_consumed_.clear();
    candidate_groups_.clear();
    agent_commands_.clear();
    const bool terminal = agent_status_ == agent::Status::completed ||
                          agent_status_ == agent::Status::cancelled ||
                          agent_status_ == agent::Status::disconnected ||
                          agent_status_ == agent::Status::timeout ||
                          agent_status_ == agent::Status::error;
    if (!terminal) {
        for (const auto& command : result->response.commands) {
            if (!command.enabled) continue;
            auto label = wide_from_utf8(command.label);
            if (label.empty()) continue;
            candidates_.push_back(std::move(label));
            candidate_consumed_.push_back(1);
            candidate_groups_.push_back(0);
            agent_commands_.push_back(command);
        }
    }
    candidate_page_ = terminal ? 0 : result->response.page;
    has_more_candidates_ = !terminal && result->response.has_more;
    agent_response_visible_ = true;
    if (result->response.task.intent.empty() && !agent_message_.empty() &&
        agent_status_ != agent::Status::error &&
        agent_status_ != agent::Status::disconnected) {
        input_buffer_ = L"v";
        input_cursor_ = 1;
        staged_candidates_.clear();
        staged_cursor_ = 0;
        segmented_input_.clear();
    }
    if (terminal) {
        agent_request_pending_ = false;
        agent_session_id_.clear();
        agent_last_request_id_.clear();
        agent_state_revision_ = 0;
        agent_task_ = {};
    }
    hovered_target_.reset();
    pressed_target_.reset();
    update_candidate_window();
}

void TextService::update_candidate_window() {
    if (candidate_window_ == nullptr) return;
    if ((!alt_code_preview_active_ &&
         (!chinese_mode_ || !has_composition())) || !foreground_focus_) {
        hide_candidate_windows();
        return;
    }
    POINT position = candidate_anchor_;
    if (!candidate_anchor_valid_) GetCursorPos(&position);
    const UINT dpi = std::max(GetDpiForWindow(candidate_window_), 96U);
    SIZE window_size = desired_candidate_window_size();
    LONG stable_surface_width = dips_to_pixels(kCandidateWindowMaxWidthDip, dpi);
    int x = position.x + (candidate_anchor_valid_ ? 0 : dips_to_pixels(12.0F, dpi));
    int y = position.y + (candidate_anchor_valid_ ? dips_to_pixels(6.0F, dpi)
                                                  : dips_to_pixels(20.0F, dpi));

    const HMONITOR monitor = MonitorFromPoint(position, MONITOR_DEFAULTTONEAREST);
    MONITORINFO monitor_info{sizeof(monitor_info)};
    if (GetMonitorInfoW(monitor, &monitor_info)) {
        const RECT& work_area = monitor_info.rcWork;
        const int available_width = static_cast<int>(work_area.right - work_area.left) -
                                    dips_to_pixels(16.0F, dpi);
        const int available_height = static_cast<int>(work_area.bottom - work_area.top) -
                                     dips_to_pixels(16.0F, dpi);
        window_size.cx = std::min(window_size.cx, static_cast<LONG>(available_width));
        window_size.cy = std::min(window_size.cy, static_cast<LONG>(available_height));
        stable_surface_width = std::min(
            stable_surface_width, static_cast<LONG>(available_width));
        x = std::min(x, static_cast<int>(work_area.right - window_size.cx));
        if (y + window_size.cy > work_area.bottom) {
            y = position.y - window_size.cy - dips_to_pixels(6.0F, dpi);
        }
        x = std::max(x, static_cast<int>(work_area.left));
        y = std::max(y, static_cast<int>(work_area.top));
    }
    const SIZE surface_size{
        std::max(stable_surface_width, window_size.cx), window_size.cy};
    const bool content_changed = !candidate_present_geometry_valid_ ||
        candidate_content_size_.cx != window_size.cx ||
        candidate_content_size_.cy != window_size.cy;
    const bool geometry_changed = !candidate_present_geometry_valid_ ||
        candidate_present_position_.x != x || candidate_present_position_.y != y ||
        candidate_present_size_.cx != surface_size.cx ||
        candidate_present_size_.cy != surface_size.cy;
    if (geometry_changed) {
        blurred_candidate_background_.clear();
    }
    if (geometry_changed || content_changed) {
        candidate_present_position_ = {x, y};
        candidate_present_size_ = surface_size;
        candidate_content_size_ = window_size;
        candidate_present_geometry_valid_ = true;
    }
    // Build the complete frame in memory and atomically publish pixels and
    // geometry. Never resize the visible layered HWND before its replacement
    // surface is ready.
    if (!render_candidate_window()) return;
    // A synchronous state update may overtake a queued WM_PAINT. The frame now
    // on screen already contains the latest snapshot, so validate that stale
    // paint request instead of submitting the same layered bitmap twice.
    ValidateRect(candidate_window_, nullptr);
    // Present the shadow only after the main layered bitmap is complete, so it
    // cannot briefly outline an empty resized surface.
    update_candidate_shadow();
}

void TextService::append_alt_code_digit(const unsigned digit) {
    if (digit > 9 || !foreground_focus_ || has_composition() ||
        !shortcut_config_.alt_code_preview_enabled) return;
    if (!alt_code_preview_active_) {
        ITfDocumentMgr* document_manager = nullptr;
        ITfContext* context = nullptr;
        if (thread_manager_ != nullptr &&
            SUCCEEDED(thread_manager_->GetFocus(&document_manager)) &&
            document_manager != nullptr) {
            document_manager->GetTop(&context);
            document_manager->Release();
        }
        if (context != nullptr) {
            update_candidate_anchor(context);
            context->Release();
        }
        alt_code_preview_active_ = true;
        alt_code_digits_.clear();
    }
    if (alt_code_digits_.size() >= 10) return;
    alt_code_digits_.push_back(static_cast<wchar_t>(L'0' + digit));
    alt_code_preview_page_ = 0;
    alt_code_preview_items_.clear();
    const auto append = [this](std::wstring code, const bool strict) {
        const auto symbol = alt_code_symbol(code);
        if (!symbol.has_value()) return;
        alt_code_preview_items_.push_back({std::move(code), *symbol, strict});
    };
    append(alt_code_digits_, true);
    for (wchar_t tail = L'0'; tail <= L'9'; ++tail) {
        auto code = alt_code_digits_;
        code.push_back(tail);
        append(std::move(code), false);
    }
    hovered_target_.reset();
    pressed_target_.reset();
    update_candidate_window();
}

void TextService::finish_alt_code_preview() noexcept {
    if (!alt_code_preview_active_) return;
    alt_code_preview_active_ = false;
    alt_code_digits_.clear();
    alt_code_preview_items_.clear();
    alt_code_preview_page_ = 0;
    hovered_target_.reset();
    pressed_target_.reset();
    hide_candidate_windows();
}

HRESULT TextService::commit_alt_code_preview_from_window() {
    if (!alt_code_preview_active_) return S_FALSE;
    const auto symbol = alt_code_symbol(alt_code_digits_);
    finish_alt_code_preview();
    if (!symbol.has_value() || thread_manager_ == nullptr) return S_FALSE;

    ITfDocumentMgr* document_manager = nullptr;
    HRESULT result = thread_manager_->GetFocus(&document_manager);
    if (FAILED(result) || document_manager == nullptr)
        return FAILED(result) ? result : E_FAIL;
    ITfContext* context = nullptr;
    result = document_manager->GetTop(&context);
    document_manager->Release();
    if (FAILED(result) || context == nullptr) return FAILED(result) ? result : E_FAIL;
    result = commit_text(context, *symbol);
    context->Release();
    return result;
}

void TextService::change_alt_code_preview_page(const int direction) {
    if (!alt_code_preview_active_ || direction == 0) return;
    const auto page_size = std::max<std::size_t>(
        2, static_cast<std::size_t>(shortcut_config_.alt_code_preview_page_size));
    const bool has_strict = !alt_code_preview_items_.empty() &&
                            alt_code_preview_items_.front().strict;
    const auto tail_count = alt_code_preview_items_.size() -
                            (has_strict ? std::size_t{1} : std::size_t{0});
    const auto tail_capacity = has_strict ? page_size - 1 : page_size;
    const auto page_count = std::max<std::size_t>(
        1, (tail_count + tail_capacity - 1) / tail_capacity);
    const auto requested = static_cast<std::int64_t>(alt_code_preview_page_) +
                           static_cast<std::int64_t>(direction);
    const auto clamped = std::clamp<std::int64_t>(
        requested, 0, static_cast<std::int64_t>(page_count - 1));
    if (clamped == static_cast<std::int64_t>(alt_code_preview_page_)) return;
    alt_code_preview_page_ = static_cast<std::size_t>(clamped);
    update_candidate_window();
}

void TextService::change_candidate_page(const int direction) {
    if (plugin_scheme_selected()) { change_plugin_page(direction); return; }
    if (candidates_expanded_ || direction == 0) return;
    if (is_agent_mode() && !agent_session_id_.empty()) {
        if (agent_request_pending_) return;
        if (direction > 0 && !has_more_candidates_) return;
        if (direction < 0 && candidate_page_ == 0) return;
        const auto next = static_cast<std::uint32_t>(direction > 0
            ? candidate_page_ + 1 : candidate_page_ - 1);
        queue_agent_action(agent::Action::page, {}, next);
        return;
    }
    if (candidate_request_pending_) {
        deferred_page_direction_ = std::clamp(
            deferred_page_direction_ + (direction > 0 ? 1 : -1), -100, 100);
        return;
    }
    if (direction > 0) {
        if (!has_more_candidates_) return;
        ++candidate_page_;
    } else if (direction < 0) {
        if (candidate_page_ == 0) return;
        --candidate_page_;
    } else return;
    has_more_candidates_ = false;
    candidates_expanded_ = false;
    hovered_target_.reset();
    pressed_target_.reset();
    queue_candidate_request();
}

bool TextService::has_composition() const noexcept {
    return !input_buffer_.empty() || !staged_candidates_.empty();
}

std::wstring TextService::staged_text() const {
    std::wstring result;
    for (const auto& fragment : staged_candidates_) result.append(fragment.text);
    return result;
}

std::wstring TextService::composition_preview() const {
    if (is_agent_mode() && !input_buffer_.empty() && input_buffer_.front() == L'v') {
        std::wstring result = L"v + ";
        result.append(staged_text());
        std::wstring_view remaining = segmented_input_.empty()
            ? std::wstring_view(input_buffer_).substr(1)
            : std::wstring_view(segmented_input_);
        if (!remaining.empty() && remaining.front() == L'v') remaining.remove_prefix(1);
        result.append(remaining);
        return result;
    }
    std::wstring result = staged_text();
    result.append(segmented_input_.empty() ? input_buffer_ : segmented_input_);
    return result;
}

std::size_t TextService::composition_cursor() const noexcept {
    if (is_agent_mode() && !input_buffer_.empty() && input_buffer_.front() == L'v') {
        if (staged_cursor_ < staged_candidates_.size()) return staged_cursor_ + 1;
        if (input_cursor_ == 0) return 0;
        return staged_candidates_.size() +
               std::min(input_cursor_, input_buffer_.size());
    }
    if (staged_cursor_ < staged_candidates_.size()) return staged_cursor_;
    return staged_candidates_.size() + std::min(input_cursor_, input_buffer_.size());
}

void TextService::toggle_unicode_palette() {
    if (unicode_palette_.categories.empty()) return;
    unicode_palette_visible_ = !unicode_palette_visible_;
    unicode_category_index_ = std::min(
        unicode_category_index_, unicode_palette_.categories.size() - 1);
    unicode_scroll_row_ = 0;
    candidates_expanded_ = false;
    hovered_target_.reset();
    pressed_target_.reset();
    update_candidate_window();
}

void TextService::change_unicode_palette_page(const int direction) {
    scroll_unicode_palette_rows(direction *
                                static_cast<int>(kUnicodeVisibleRows));
}

void TextService::scroll_unicode_palette_rows(const int rows) {
    if (!unicode_palette_visible_ || unicode_palette_.categories.empty() ||
        rows == 0) return;
    const auto& category = unicode_palette_.categories[
        std::min(unicode_category_index_, unicode_palette_.categories.size() - 1)];
    const auto total_rows = (category.items.size() + kUnicodeItemsPerRow - 1) /
                            kUnicodeItemsPerRow;
    const auto maximum_scroll = total_rows > kUnicodeVisibleRows
                                    ? total_rows - kUnicodeVisibleRows : 0;
    const auto requested = static_cast<std::int64_t>(unicode_scroll_row_) + rows;
    const auto clamped = std::clamp<std::int64_t>(
        requested, 0, static_cast<std::int64_t>(maximum_scroll));
    if (clamped == static_cast<std::int64_t>(unicode_scroll_row_)) return;
    unicode_scroll_row_ = static_cast<std::size_t>(clamped);
    hovered_target_.reset();
    pressed_target_.reset();
    update_candidate_window();
}

void TextService::scroll_unicode_categories(const float dip_delta) {
    if (!unicode_palette_visible_ || unicode_palette_.categories.empty() ||
        dip_delta == 0.0F) return;
    float total_width = 0.0F;
    for (const auto& category : unicode_palette_.categories) {
        if (total_width != 0.0F) total_width += kCandidateGapDip;
        total_width += std::max(
            56.0F, measure_text_width(dwrite_factory_, label_text_format_,
                                      wide_from_utf8(category.label)) + 18.0F);
    }
    const UINT dpi = candidate_window_ == nullptr ? 96 : GetDpiForWindow(candidate_window_);
    RECT client{};
    if (candidate_window_ != nullptr) GetClientRect(candidate_window_, &client);
    const float client_width = candidate_window_ == nullptr
        ? kHorizontalPaddingDip * 2.0F +
              kUnicodeCellSizeDip * kUnicodeItemsPerRow
        : static_cast<float>(client.right - client.left) * 96.0F /
              static_cast<float>(dpi);
    const float maximum = std::max(
        0.0F, total_width - (client_width - kHorizontalPaddingDip * 2.0F));
    const float clamped = std::clamp(
        unicode_category_scroll_offset_dip_ + dip_delta, 0.0F, maximum);
    if (std::abs(clamped - unicode_category_scroll_offset_dip_) < 0.01F) return;
    unicode_category_scroll_offset_dip_ = clamped;
    hovered_target_.reset();
    pressed_target_.reset();
    update_candidate_window();
}

HRESULT TextService::commit_unicode_item_from_window(const std::size_t item_index) {
    if (!unicode_palette_visible_ || unicode_palette_.categories.empty() ||
        thread_manager_ == nullptr) return E_UNEXPECTED;
    const auto& category = unicode_palette_.categories[
        std::min(unicode_category_index_, unicode_palette_.categories.size() - 1)];
    if (item_index >= category.items.size()) return E_INVALIDARG;
    ITfDocumentMgr* document_manager = nullptr;
    HRESULT result = thread_manager_->GetFocus(&document_manager);
    if (FAILED(result) || document_manager == nullptr)
        return FAILED(result) ? result : E_FAIL;
    ITfContext* context = nullptr;
    result = document_manager->GetTop(&context);
    document_manager->Release();
    if (FAILED(result) || context == nullptr) return FAILED(result) ? result : E_FAIL;
    result = commit_text(context, unicode_scalar(category.items[item_index].codepoint));
    context->Release();
    if (SUCCEEDED(result)) update_candidate_window();
    return result;
}

void TextService::set_composition_cursor(const std::size_t cursor) {
    const auto staged_count = staged_candidates_.size();
    if (is_agent_mode() && !input_buffer_.empty() && input_buffer_.front() == L'v') {
        if (cursor == 0) {
            staged_cursor_ = staged_count;
            input_cursor_ = 0;
        } else if (cursor <= staged_count) {
            staged_cursor_ = cursor - 1;
            input_cursor_ = 0;
        } else {
            staged_cursor_ = staged_count;
            input_cursor_ = std::min(cursor - staged_count, input_buffer_.size());
        }
        return;
    }
    if (cursor <= staged_count) {
        staged_cursor_ = cursor;
        input_cursor_ = 0;
    } else {
        staged_cursor_ = staged_count;
        input_cursor_ = std::min(cursor - staged_count, input_buffer_.size());
    }
}

void TextService::restore_staged_suffix(const std::size_t first) {
    if (first >= staged_candidates_.size()) {
        staged_cursor_ = staged_candidates_.size();
        return;
    }
    std::wstring restored;
    for (std::size_t index = first; index < staged_candidates_.size(); ++index)
        restored.append(staged_candidates_[index].source_input);
    const bool agent_prefix = !input_buffer_.empty() &&
                              input_buffer_.front() == L'v';
    input_buffer_.insert(agent_prefix ? 1 : 0, restored);
    if (agent_prefix && input_cursor_ == 0) input_cursor_ = 1;
    staged_candidates_.erase(staged_candidates_.begin() +
                                 static_cast<std::ptrdiff_t>(first),
                             staged_candidates_.end());
    staged_cursor_ = staged_candidates_.size();
    segmented_input_.clear();
}

void TextService::move_pinyin_cursor(const int direction) {
    const auto current = static_cast<std::int64_t>(composition_cursor());
    const auto requested = current + static_cast<std::int64_t>(direction);
    const auto clamped = std::clamp<std::int64_t>(
        requested, 0, static_cast<std::int64_t>(
            staged_candidates_.size() + input_buffer_.size()));
    if (clamped == current) return;
    set_composition_cursor(static_cast<std::size_t>(clamped));
    hovered_target_.reset();
    pressed_target_.reset();
    if (candidate_window_ != nullptr)
        InvalidateRect(candidate_window_, nullptr, FALSE);
}

void TextService::move_candidate_page_from_shortcut(const int direction) {
    if (candidates_expanded_)
        scroll_expanded_candidates(direction);
    else
        change_candidate_page(direction);
}

void TextService::scroll_expanded_candidates(const int rows) {
    if (!candidates_expanded_ || rows == 0) return;
    const auto page_size = std::max<std::size_t>(
        1, static_cast<std::size_t>(candidate_page_size_));
    const auto total_rows = (candidates_.size() + page_size - 1) / page_size;
    const auto maximum_scroll = total_rows > kExpandedVisibleRows
                                    ? total_rows - kExpandedVisibleRows
                                    : 0;
    const auto current = static_cast<std::int64_t>(expanded_scroll_row_);
    const auto requested = current + static_cast<std::int64_t>(rows);
    const auto clamped = std::clamp<std::int64_t>(
        requested, 0, static_cast<std::int64_t>(maximum_scroll));
    if (clamped == current) return;
    expanded_scroll_row_ = static_cast<std::size_t>(clamped);
    hovered_target_.reset();
    pressed_target_.reset();
    update_candidate_window();
}

std::optional<TextService::HitTarget> TextService::hit_test(const POINT point) const {
    for (auto region = hit_regions_.rbegin(); region != hit_regions_.rend(); ++region) {
        if (PtInRect(&region->bounds, point)) return region->target;
    }
    return std::nullopt;
}

void TextService::invoke_hit_target(const HitTarget& target) {
    if (plugin_scheme_selected()) {
        if (target.kind == HitKind::candidate) select_plugin_candidate(target.candidate_index);
        else if (target.kind == HitKind::previous_page) change_plugin_page(-1);
        else if (target.kind == HitKind::next_page) change_plugin_page(1);
        else if (target.kind == HitKind::toggle_expanded && plugin_input_descriptor_ && !plugin_input_committing_) {
            plugin_pending_selection_.reset();
            candidates_expanded_ = !candidates_expanded_; candidate_page_ = 0;
            expanded_scroll_row_ = 0; queue_plugin_compose();
        }
        else if (target.kind == HitKind::pinyin_cursor && !plugin_input_opening_ && !plugin_input_committing_) {
            if (plugin_input_document_.composition().set_caret_utf16(target.candidate_index)) {
                plugin_pending_selection_.reset();
                sync_plugin_display(); queue_plugin_compose();
            }
        }
        return;
    }
    if (target.kind == HitKind::unicode_toggle) {
        toggle_unicode_palette();
        return;
    }
    if (target.kind == HitKind::unicode_category) {
        if (target.candidate_index < unicode_palette_.categories.size()) {
            unicode_category_index_ = target.candidate_index;
            unicode_scroll_row_ = 0;
            hovered_target_.reset();
            pressed_target_.reset();
            update_candidate_window();
        }
        return;
    }
    if (target.kind == HitKind::unicode_item) {
        commit_unicode_item_from_window(target.candidate_index);
        return;
    }
    if (target.kind == HitKind::pinyin_cursor) {
        set_composition_cursor(target.candidate_index);
        hovered_target_.reset();
        pressed_target_.reset();
        if (candidate_window_ != nullptr)
            InvalidateRect(candidate_window_, nullptr, FALSE);
        return;
    }
    if (candidate_request_pending_) {
        if (target.kind == HitKind::candidate)
            defer_candidate_selection(target.candidate_index, nullptr);
        else if (target.kind == HitKind::previous_page)
            change_candidate_page(-1);
        else if (target.kind == HitKind::next_page)
            change_candidate_page(1);
        return;
    }
    switch (target.kind) {
        case HitKind::pinyin_cursor:
            break;
        case HitKind::candidate:
            if (is_agent_mode() && target.candidate_index < agent_commands_.size()) {
                if (agent_commands_[target.candidate_index].high_risk) {
                    agent_status_ = agent::Status::error;
                    agent_message_ = L"高风险指令必须在第三方 Agent 内单独确认";
                    agent_response_visible_ = true;
                    update_candidate_window();
                } else {
                    select_agent_command(target.candidate_index);
                }
            } else if (is_agent_mode()) {
                stage_agent_candidate(nullptr, target.candidate_index);
            } else {
                commit_candidate_from_window(target.candidate_index);
            }
            break;
        case HitKind::previous_page:
            change_candidate_page(-1);
            break;
        case HitKind::next_page:
            change_candidate_page(1);
            break;
        case HitKind::toggle_expanded:
            if (!candidates_.empty() || candidates_expanded_) {
                const bool expanding = !candidates_expanded_;
                candidates_expanded_ = expanding;
                candidate_page_ = 0;
                has_more_candidates_ = false;
                expanded_scroll_row_ = 0;
                hovered_target_.reset();
                pressed_target_.reset();
                if (is_agent_mode() && agent_response_visible_)
                    update_candidate_window();
                else
                    queue_candidate_request();
            }
            break;
        case HitKind::unicode_toggle:
        case HitKind::unicode_category:
        case HitKind::unicode_item:
            break;
    }
}

HRESULT TextService::move_document_caret(ITfContext* context, const WPARAM key,
                                         const bool select) {
    if (context == nullptr || (key != VK_LEFT && key != VK_RIGHT &&
                               key != VK_UP && key != VK_DOWN)) return E_INVALIDARG;
    auto* session = new (std::nothrow) DocumentCaretEditSession(context, key, select);
    if (session == nullptr) return E_OUTOFMEMORY;
    HRESULT session_result = E_FAIL;
    const HRESULT request = context->RequestEditSession(
        client_id_, session, TF_ES_SYNC | TF_ES_READWRITE, &session_result);
    session->Release();
    if (SUCCEEDED(request) && SUCCEEDED(session_result)) update_candidate_anchor(context);
    return FAILED(request) ? request : session_result;
}

void TextService::defer_candidate_selection(const std::size_t index,
                                            ITfContext* context) {
    if (index >= candidates_.size()) return;
    ITfContext* selected_context = context;
    if (selected_context != nullptr) {
        selected_context->AddRef();
    } else if (thread_manager_ != nullptr) {
        ITfDocumentMgr* document_manager = nullptr;
        if (SUCCEEDED(thread_manager_->GetFocus(&document_manager)) &&
            document_manager != nullptr) {
            document_manager->GetTop(&selected_context);
            document_manager->Release();
        }
    }
    if (selected_context == nullptr) return;
    clear_deferred_candidate_selection();
    deferred_candidate_text_ = candidates_[index];
    deferred_candidate_context_ = selected_context;
}

void TextService::clear_deferred_candidate_selection() noexcept {
    deferred_candidate_text_.reset();
    release_interface(deferred_candidate_context_);
}

void TextService::defer_punctuation(std::wstring punctuation, ITfContext* context) {
    if (context == nullptr || punctuation.empty()) return;
    clear_deferred_punctuation();
    context->AddRef();
    deferred_punctuation_ = std::move(punctuation);
    deferred_punctuation_context_ = context;
}

void TextService::clear_deferred_punctuation() noexcept {
    deferred_punctuation_.reset();
    release_interface(deferred_punctuation_context_);
}

void TextService::update_candidate_anchor(ITfContext* context) {
    candidate_anchor_valid_ = false; // A failed read must not reuse another caret's anchor.
    auto* session = new (std::nothrow)
        CaretEditSession(context, &candidate_anchor_, &candidate_anchor_valid_);
    if (session == nullptr) return;
    HRESULT session_result = E_FAIL;
    const auto request = context->RequestEditSession(client_id_, session, TF_ES_SYNC | TF_ES_READ,
                                &session_result);
    trace_tsf_event("caret.request", request, session_result);
    session->Release();
}

void TextService::clear_composition() {
    trace_tsf_event("edit.clear", static_cast<HRESULT>(context_generation_),
        static_cast<HRESULT>(static_cast<unsigned>(agent_request_pending_) | (agent_response_visible_ << 1)));
    reset_agent_origin();
    reset_plugin_input();
    ++context_generation_;
    latest_candidate_request_id_.store(0, std::memory_order_release);
    if (candidate_cancellation_event_ != nullptr) SetEvent(candidate_cancellation_event_);
    input_buffer_.clear();
    input_cursor_ = 0;
    staged_candidates_.clear();
    staged_cursor_ = 0;
    agent_last_submitted_pinyin_.clear();
    agent_task_ = {};
    agent_commands_.clear();
    agent_request_submitted_ = false;
    segmented_input_.clear();
    candidates_.clear();
    candidate_consumed_.clear();
    candidate_groups_.clear();
    candidate_failure_detail_.clear();
    hit_regions_.clear();
    hovered_target_.reset();
    pressed_target_.reset();
    clear_deferred_candidate_selection();
    clear_deferred_punctuation();
    candidate_page_ = 0;
    has_more_candidates_ = false;
    candidate_request_pending_ = false;
    deferred_page_direction_ = 0;
    candidate_request_failed_ = false;
    candidate_retry_count_ = 0;
    candidates_expanded_ = false;
    expanded_scroll_row_ = 0;
    unicode_palette_visible_ = false;
    unicode_scroll_row_ = 0;
    unicode_category_scroll_offset_dip_ = 0.0F;
    candidate_anchor_valid_ = false;
    alt_code_preview_active_ = false;
    alt_code_digits_.clear();
    alt_code_preview_items_.clear();
    alt_code_preview_page_ = 0;
    hide_candidate_windows();
}

std::optional<std::wstring> TextService::chinese_punctuation_for_key(
    const WPARAM key, const bool advance_quote_state) {
    const bool shift = key_down(VK_SHIFT);
    char source = 0;
    switch (key) {
        case VK_OEM_COMMA: source = shift ? '<' : ','; break;
        case VK_OEM_PERIOD: source = shift ? '>' : '.'; break;
        case VK_OEM_2: source = shift ? '?' : '/'; break;
        case VK_OEM_1: source = shift ? ':' : ';'; break;
        case VK_OEM_4: source = shift ? '{' : '['; break;
        case VK_OEM_6: source = shift ? '}' : ']'; break;
        case VK_OEM_5: source = shift ? '|' : '\\'; break;
        case VK_OEM_MINUS: source = shift ? '_' : '-'; break;
        case VK_OEM_PLUS: source = shift ? '+' : '='; break;
        case VK_OEM_3: source = shift ? '~' : '`'; break;
        case VK_OEM_7: source = shift ? '"' : '\''; break;
        case '9': if (shift) source = '('; break;
        case '0': if (shift) source = ')'; break;
        case '1': if (shift) source = '!'; break;
        case '4': if (shift) source = '$'; break;
        case '6': if (shift) source = '^'; break;
        default: break;
    }
    if (!config::punctuation_selected(shortcut_config_.chinese_punctuation_mask, source))
        return std::nullopt;
    switch (key) {
        case VK_OEM_COMMA: return shift ? L"《" : L"，";
        case VK_OEM_PERIOD: return shift ? L"》" : L"。";
        case VK_OEM_2: if (shift) return L"？"; break;
        case VK_OEM_1: return shift ? L"：" : L"；";
        case VK_OEM_4: return shift ? L"【" : L"【";
        case VK_OEM_6: return shift ? L"】" : L"】";
        case VK_OEM_5: return shift ? L"｜" : L"、";
        case VK_OEM_MINUS: return shift ? L"——" : L"－";
        case VK_OEM_PLUS: return shift ? L"＋" : L"＝";
        case VK_OEM_3: return shift ? L"～" : L"·";
        case VK_OEM_7: {
            bool& open = shift ? double_quote_open_ : single_quote_open_;
            const std::wstring result = shift
                ? (open ? L"“" : L"”")
                : (open ? L"‘" : L"’");
            if (advance_quote_state) open = !open;
            return result;
        }
        case '9': if (shift) return L"（"; break;
        case '0': if (shift) return L"）"; break;
        case '1': if (shift) return L"！"; break;
        case '4': if (shift) return L"￥"; break;
        case '6': if (shift) return L"……"; break;
        default: break;
    }
    return std::nullopt;
}

HRESULT TextService::commit_candidate(ITfContext* context, const std::size_t index,
                                      const std::wstring_view suffix) {
    if (candidate_request_pending_) return E_PENDING;
    if (index >= candidates_.size() || index >= candidate_consumed_.size())
        return E_INVALIDARG;
    const auto consumed = candidate_consumed_[index];
    if (consumed == 0 || consumed > input_buffer_.size()) return E_INVALIDARG;
    const std::wstring committed = candidates_[index];
    std::wstring inserted = committed;
    inserted.append(suffix);
    const std::wstring previous_context = recent_language_context_;
    std::wstring learned_input = input_buffer_.substr(0, static_cast<std::size_t>(consumed));
    while (!learned_input.empty() && learned_input.back() == L'\'') learned_input.pop_back();
    if (!shortcut_config_.candidate_immediate_commit_enabled && suffix.empty()) {
        auto source_end = static_cast<std::size_t>(consumed);
        while (source_end < input_buffer_.size() && input_buffer_[source_end] == L'\'')
            ++source_end;
        staged_candidates_.push_back({committed, input_buffer_.substr(0, source_end)});
        staged_cursor_ = staged_candidates_.size();
        recent_language_input_ = learned_input;
        input_buffer_.erase(0, source_end);
        input_cursor_ = input_buffer_.size();
        ++context_generation_;
        segmented_input_.clear();
        candidates_.clear();
        candidate_consumed_.clear();
        candidate_groups_.clear();
        candidate_failure_detail_.clear();
        candidate_page_ = 0;
        has_more_candidates_ = false;
        candidate_request_pending_ = false;
        candidates_expanded_ = false;
        hovered_target_.reset();
        pressed_target_.reset();
        update_candidate_anchor(context);
        if (input_buffer_.empty()) update_candidate_window();
        else queue_candidate_request();
        return S_OK;
    }
    const auto prior_staged = staged_candidates_;
    const std::wstring prior_chain_text = learning_chain_text_;
    const std::wstring prior_chain_input = learning_chain_input_;
    const std::wstring prior_chain_context = learning_chain_context_;
    std::wstring remaining_input = input_buffer_.substr(static_cast<std::size_t>(consumed));
    while (!remaining_input.empty() && remaining_input.front() == L'\'')
        remaining_input.erase(remaining_input.begin());
    const bool finishes_composition = remaining_input.empty();
    if (!staged_candidates_.empty()) inserted.insert(0, staged_text());
    auto* session = new (std::nothrow) CommitEditSession(context, std::move(inserted));
    if (session == nullptr) return E_OUTOFMEMORY;
    HRESULT session_result = E_FAIL;
    const HRESULT request_result = context->RequestEditSession(
        client_id_, session, TF_ES_SYNC | TF_ES_READWRITE, &session_result);
    session->Release();
    if (SUCCEEDED(request_result) && SUCCEEDED(session_result)) {
        if (!staged_candidates_.empty()) append_language_context(staged_text());
        append_language_context(committed);
        append_language_context(suffix);
        recent_language_input_ = learned_input;
        staged_candidates_.clear();
        staged_cursor_ = 0;
        if (finishes_composition) {
            clear_composition();
        } else {
            input_buffer_ = std::move(remaining_input);
            input_cursor_ = input_buffer_.size();
            ++context_generation_;
            segmented_input_.clear();
            candidate_page_ = 0;
            has_more_candidates_ = false;
            candidate_request_pending_ = false;
            candidates_expanded_ = false;
            hovered_target_.reset();
            pressed_target_.reset();
            update_candidate_anchor(context);
            queue_candidate_request();
        }

        std::wstring feedback_context = previous_context;
        std::wstring composite_text;
        std::wstring composite_input;
        for (const auto& fragment : prior_staged) {
            std::wstring fragment_input = fragment.source_input;
            while (!fragment_input.empty() && fragment_input.back() == L'\'')
                fragment_input.pop_back();
            queue_commit_feedback(fragment.text, fragment_input, feedback_context);
            append_bounded_language_context(feedback_context, fragment.text);
            composite_text += fragment.text;
            composite_input += compact_feedback_input(fragment_input);
        }
        queue_commit_feedback(committed, learned_input, feedback_context);

        composite_text += committed;
        composite_input += compact_feedback_input(learned_input);
        if (!prior_chain_text.empty()) {
            composite_text.insert(0, prior_chain_text);
            composite_input.insert(0, prior_chain_input);
            if (finishes_composition)
                queue_commit_feedback(composite_text, composite_input,
                                      prior_chain_context);
            else {
                learning_chain_text_ = std::move(composite_text);
                learning_chain_input_ = std::move(composite_input);
                learning_chain_context_ = prior_chain_context;
            }
        } else if (!prior_staged.empty()) {
            queue_commit_feedback(composite_text, composite_input, previous_context);
        } else if (!finishes_composition) {
            learning_chain_text_ = std::move(composite_text);
            learning_chain_input_ = std::move(composite_input);
            learning_chain_context_ = previous_context;
        }
    }
    return FAILED(request_result) ? request_result : session_result;
}

HRESULT TextService::stage_agent_candidate(ITfContext* context,
                                           const std::size_t index) {
    if (!is_agent_mode() || agent_response_visible_ || candidate_request_pending_)
        return E_PENDING;
    if (input_buffer_.empty() || input_buffer_.front() != L'v' ||
        index >= candidates_.size() || index >= candidate_consumed_.size())
        return E_INVALIDARG;
    const auto consumed = static_cast<std::size_t>(candidate_consumed_[index]);
    if (consumed == 0 || consumed > input_buffer_.size() - 1) return E_INVALIDARG;
    auto source_end = 1 + consumed;
    while (source_end < input_buffer_.size() && input_buffer_[source_end] == L'\'')
        ++source_end;
    staged_candidates_.push_back({candidates_[index],
                                  input_buffer_.substr(1, source_end - 1)});
    staged_cursor_ = staged_candidates_.size();
    input_buffer_.erase(1, source_end - 1);
    input_cursor_ = input_buffer_.size();
    ++context_generation_;
    segmented_input_.clear();
    candidates_.clear();
    candidate_consumed_.clear();
    candidate_groups_.clear();
    candidate_failure_detail_.clear();
    candidate_page_ = 0;
    has_more_candidates_ = false;
    candidate_request_pending_ = false;
    candidates_expanded_ = false;
    hovered_target_.reset();
    pressed_target_.reset();
    if (context != nullptr) update_candidate_anchor(context);
    if (input_buffer_.size() > 1) {
        queue_candidate_request();
    } else {
        // The mode marker is the only remaining input, so the selected
        // candidate has consumed the complete request. Submit immediately;
        // Enter and Space remain available for sending a partially converted
        // request early.
        queue_agent_action(agent::Action::submit);
    }
    return S_OK;
}

HRESULT TextService::commit_text(ITfContext* context, std::wstring text) {
    if (context == nullptr || text.empty()) return E_INVALIDARG;
    const std::wstring committed = text;
    auto* session = new (std::nothrow) CommitEditSession(context, std::move(text));
    if (session == nullptr) return E_OUTOFMEMORY;
    HRESULT session_result = E_FAIL;
    const HRESULT request_result = context->RequestEditSession(
        client_id_, session, TF_ES_SYNC | TF_ES_READWRITE, &session_result);
    session->Release();
    if (SUCCEEDED(request_result) && SUCCEEDED(session_result))
        append_language_context(committed);
    return FAILED(request_result) ? request_result : session_result;
}

HRESULT TextService::commit_raw_input(ITfContext* context) {
    if (context == nullptr || input_buffer_.empty()) return E_INVALIDARG;
    const std::wstring committed = input_buffer_;
    auto* session = new (std::nothrow) CommitEditSession(context, committed);
    if (session == nullptr) return E_OUTOFMEMORY;
    HRESULT session_result = E_FAIL;
    const HRESULT request_result = context->RequestEditSession(
        client_id_, session, TF_ES_SYNC | TF_ES_READWRITE, &session_result);
    session->Release();
    if (SUCCEEDED(request_result) && SUCCEEDED(session_result)) {
        append_language_context(committed);
        clear_composition();
        recent_language_context_.clear();
        recent_language_input_.clear();
    }
    return FAILED(request_result) ? request_result : session_result;
}

HRESULT TextService::commit_pending_composition(ITfContext* context) {
    if (context == nullptr || !has_composition()) return E_INVALIDARG;
    const std::wstring previous_context = recent_language_context_;
    const auto staged = staged_candidates_;
    std::wstring text = staged_text();
    text.append(input_buffer_);
    const HRESULT result = commit_text(context, std::move(text));
    if (SUCCEEDED(result)) {
        std::wstring feedback_context = previous_context;
        std::wstring composite_text;
        std::wstring composite_input;
        for (const auto& fragment : staged) {
            std::wstring feedback_input = fragment.source_input;
            while (!feedback_input.empty() && feedback_input.back() == L'\'')
                feedback_input.pop_back();
            queue_commit_feedback(fragment.text, feedback_input, feedback_context);
            append_bounded_language_context(feedback_context, fragment.text);
            composite_text += fragment.text;
            composite_input += compact_feedback_input(feedback_input);
            recent_language_input_ = std::move(feedback_input);
        }
        if (staged.size() > 1)
            queue_commit_feedback(std::move(composite_text),
                                  std::move(composite_input), previous_context);
        clear_composition();
    }
    return result;
}

void TextService::commit_pending_on_focus_loss() {
    refresh_shortcut_config();
    if (plugin_scheme_selected()) return; // no blind writes to a retired document
    if (!shortcut_config_.focus_loss_commit_enabled || !has_composition() ||
        thread_manager_ == nullptr) return;
    ITfDocumentMgr* document_manager = nullptr;
    if (FAILED(thread_manager_->GetFocus(&document_manager)) ||
        document_manager == nullptr) return;
    ITfContext* context = nullptr;
    const HRESULT top = document_manager->GetTop(&context);
    document_manager->Release();
    if (FAILED(top) || context == nullptr) return;
    if (staged_candidates_.empty()) commit_raw_input(context);
    else commit_pending_composition(context);
    context->Release();
}

HRESULT TextService::toggle_language_mode(ITfContext* context) {
    return apply_language_mode(!chinese_mode_, context);
}

HRESULT TextService::toggle_language_mode_from_window() {
    if (thread_manager_ == nullptr) return E_UNEXPECTED;
    ITfDocumentMgr* document_manager = nullptr;
    HRESULT result = thread_manager_->GetFocus(&document_manager);
    if (FAILED(result) || document_manager == nullptr)
        return FAILED(result) ? result : E_FAIL;
    ITfContext* context = nullptr;
    result = document_manager->GetTop(&context);
    document_manager->Release();
    if (FAILED(result) || context == nullptr) return FAILED(result) ? result : E_FAIL;
    result = toggle_language_mode(context);
    context->Release();
    return result;
}

HRESULT TextService::apply_language_mode(const bool chinese, ITfContext* context) {
    if (chinese == chinese_mode_) return S_OK;
    if (plugin_scheme_selected() && !chinese && has_composition()) {
        commit_plugin_text(input_buffer_, false, true); return S_OK;
    }
    if (!chinese && is_agent_mode()) {
        const bool commit_marker = input_buffer_ == L"v" &&
                                   staged_candidates_.empty() &&
                                   !agent_request_submitted_;
        cancel_agent_session();
        if (commit_marker) {
            if (context == nullptr) return E_UNEXPECTED;
            const HRESULT committed = commit_text(context, L"v");
            if (FAILED(committed)) return committed;
        }
        clear_composition();
        rotate_agent_context();
    }
    if (!chinese && has_composition()) {
        if (context == nullptr) return E_UNEXPECTED;
        const HRESULT committed = staged_candidates_.empty()
            ? commit_raw_input(context) : commit_pending_composition(context);
        if (FAILED(committed)) return committed;
    }
    chinese_mode_ = chinese;
    if (!chinese_mode_) clear_composition();
    update_language_bar();
    return S_OK;
}

HRESULT TextService::commit_candidate_from_window(const std::size_t index) {
    if (thread_manager_ == nullptr || index >= candidates_.size()) return E_INVALIDARG;
    ITfDocumentMgr* document_manager = nullptr;
    HRESULT result = thread_manager_->GetFocus(&document_manager);
    if (FAILED(result) || document_manager == nullptr)
        return FAILED(result) ? result : E_FAIL;
    ITfContext* context = nullptr;
    result = document_manager->GetTop(&context);
    document_manager->Release();
    if (FAILED(result) || context == nullptr) return FAILED(result) ? result : E_FAIL;
    result = commit_candidate(context, index);
    context->Release();
    return result;
}

namespace {
std::wstring scheme_key_text(WPARAM key, LPARAM flags) {
    if (command_modifier_down()) return {};
    BYTE state[256]{};
    if (!GetKeyboardState(state)) return {};
    wchar_t text[8]{};
    // Flag 4 avoids modifying the keyboard dead-key state while probing keys.
    const auto count = ToUnicodeEx(static_cast<UINT>(key),
        static_cast<UINT>((flags >> 16) & 0xff), state, text, 8, 4, GetKeyboardLayout(0));
    if (count <= 0 || count > 8 || text[0] < L' ') return {};
    return std::wstring(text, static_cast<std::size_t>(count));
}
bool same_context(ITfContext* a, ITfContext* b) {
    if (!a || !b) return false;
    IUnknown *left = nullptr, *right = nullptr;
    (void)a->QueryInterface(IID_IUnknown, reinterpret_cast<void**>(&left));
    (void)b->QueryInterface(IID_IUnknown, reinterpret_cast<void**>(&right));
    const bool same = left && right && left == right;
    if (left) left->Release(); if (right) right->Release();
    return same;
}
// Lowercase hex SHA-256 of UTF-8 text; empty on failure.
std::string sha256_hex(const std::string_view data) {
    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    unsigned char digest[32]{};
    bool ok = data.size() <= std::numeric_limits<ULONG>::max() &&
        BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0 &&
        BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) >= 0 &&
        BCryptHashData(hash, reinterpret_cast<PUCHAR>(const_cast<char*>(data.data())),
                       static_cast<ULONG>(data.size()), 0) >= 0 &&
        BCryptFinishHash(hash, digest, sizeof(digest), 0) >= 0;
    if (hash) BCryptDestroyHash(hash);
    if (algorithm) BCryptCloseAlgorithmProvider(algorithm, 0);
    if (!ok) return {};
    constexpr char hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(64);
    for (const auto byte : digest) {
        output.push_back(hex[byte >> 4]);
        output.push_back(hex[byte & 0x0f]);
    }
    return output;
}

// Reads the selection text (bounded); nullopt when the selection cannot be read.
std::optional<std::wstring> read_range_text(TfEditCookie cookie, ITfRange* range) {
    constexpr std::size_t kMaximumSelection = 64 * 1024;
    ITfRange* cursor = nullptr;
    if (FAILED(range->Clone(&cursor)) || !cursor) return std::nullopt;
    std::wstring text;
    wchar_t buffer[256];
    bool ok = true;
    for (;;) {
        ULONG fetched = 0;
        if (FAILED(cursor->GetText(cookie, TF_TF_MOVESTART, buffer,
                                   static_cast<ULONG>(std::size(buffer)), &fetched))) {
            ok = false;
            break;
        }
        if (fetched == 0) break;
        text.append(buffer, fetched);
        if (text.size() > kMaximumSelection) { ok = false; break; }
    }
    cursor->Release();
    if (!ok) return std::nullopt;
    return text;
}

bool scheme_selection_matches(ITfContext* context, TfEditCookie cookie, ITfRange* expected) {
    if (!expected) return true;
    TF_SELECTION selection{}; ULONG count{}; LONG start{}, end{};
    const auto read = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count);
    const bool same = SUCCEEDED(read) && count == 1 && selection.range &&
        SUCCEEDED(selection.range->CompareStart(cookie, expected, TF_ANCHOR_START, &start)) &&
        SUCCEEDED(selection.range->CompareEnd(cookie, expected, TF_ANCHOR_END, &end)) && !start && !end;
    if (selection.range) selection.range->Release();
    return same;
}
}

void TextService::reset_agent_origin() noexcept {
    agent_origin_valid_ = false;
    agent_origin_safety_blocked_ = false;
    agent_origin_selection_.reset();
    agent_origin_sha256_.clear();
    if (agent_origin_source_ && agent_edit_sink_cookie_ != TF_INVALID_COOKIE)
        (void)agent_origin_source_->UnadviseSink(agent_edit_sink_cookie_);
    agent_edit_sink_cookie_ = TF_INVALID_COOKIE;
    release_interface(agent_origin_source_);
    release_interface(agent_origin_range_);
    release_interface(agent_origin_context_);
}

bool TextService::capture_agent_origin(ITfContext* context) {
    reset_agent_origin();
    rotate_agent_context();
    if (!plugin_context_is_current(context)) { trace_tsf_event("origin.not_current", E_FAIL); return false; }
    bool safety_blocked = false;
    auto* edit = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
        InputMethodContextFacts observed;
        const auto safety = inspect_input_method_context(context, cookie, &observed);
        const unsigned facts = static_cast<unsigned>(observed.status_known) | (observed.compartments_known << 1) |
            (observed.scope_known << 2) | (observed.disabled << 3) |
            (observed.empty_context << 4) | (observed.read_only << 5) |
            (observed.loading << 6) | (observed.sensitive_scope << 7);
        trace_tsf_event("origin.safety", static_cast<HRESULT>(safety), static_cast<HRESULT>(facts));
        if (safety != InputMethodContextSafety::allowed) {
            safety_blocked = true;
            return E_ACCESSDENIED;
        }
        TF_SELECTION selections[2]{};
        ULONG count = 0;
        const auto read = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 2, selections, &count);
        trace_tsf_event("origin.selection", read, static_cast<HRESULT>(count));
        bool ok = SUCCEEDED(read) && count == 1 && selections[0].range;
        if (ok) {
            const auto raw = read_range_text(cookie, selections[0].range);
            ok = raw.has_value();
            trace_tsf_event("origin.read", ok ? S_OK : E_FAIL);
            if (ok) {
                const auto utf8 = utf8_from_wide(*raw);
                const auto points = std::count_if(utf8.begin(), utf8.end(),
                    [](unsigned char c) { return (c & 0xc0U) != 0x80U; });
                ok = utf8.size() <= 4096 && points <= 1000;
                agent_origin_sha256_ = ok ? sha256_hex(utf8) : std::string{};
                ok = ok && !agent_origin_sha256_.empty();
                if (ok && !raw->empty()) {
                    // Bound before filtering: never transform a silently truncated source.
                    const auto filtered = agent::filter_sensitive_text(utf8, utf8.size());
                    ok = filtered.ok && !filtered.value.empty() && filtered.value.size() <= 4096 &&
                        std::count_if(filtered.value.begin(), filtered.value.end(),
                            [](unsigned char c) { return (c & 0xc0U) != 0x80U; }) <= 1000;
                    if (ok) agent_origin_selection_ = agent::SelectionSummary{
                        filtered.value, agent_origin_sha256_, filtered.redacted};
                }
                if (ok) ok = SUCCEEDED(selections[0].range->Clone(&agent_origin_range_)) && agent_origin_range_;
            }
        }
        for (auto& selection : selections) if (selection.range) selection.range->Release();
        return ok ? S_OK : E_FAIL;
    });
    HRESULT captured = E_FAIL;
    const auto request = edit ? context->RequestEditSession(
        client_id_, edit, TF_ES_SYNC | TF_ES_READ, &captured) : E_FAIL;
    if (edit) edit->Release();
    trace_tsf_event("origin.request", request, captured);
    if (FAILED(request) || FAILED(captured)) {
        reset_agent_origin();
        agent_origin_safety_blocked_ = safety_blocked;
        return false;
    }
    context->AddRef();
    agent_origin_context_ = context;
    if (FAILED(context->QueryInterface(IID_PPV_ARGS(&agent_origin_source_))) ||
        FAILED(agent_origin_source_->AdviseSink(IID_ITfTextEditSink,
            static_cast<ITfTextEditSink*>(this), &agent_edit_sink_cookie_))) {
        reset_agent_origin(); return false;
    }
    trace_tsf_event("origin.captured");
    agent_origin_valid_ = true;
    agent_context_sensitive_ = false;
    return true;
}

HRESULT TextService::OnEndEdit(ITfContext* context, TfEditCookie, ITfEditRecord* record) {
    if (!agent_origin_valid_ || !same_context(context, agent_origin_context_)) return S_OK;
    BOOL selection_changed = TRUE;
    IEnumTfRanges* updates = nullptr;
    bool changed = !record || FAILED(record->GetSelectionStatus(&selection_changed)) || selection_changed;
    if (!changed) {
        changed = FAILED(record->GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, nullptr, 0, &updates)) || !updates;
        if (updates) {
            ITfRange* range = nullptr;
            ULONG count = 0;
            const auto next = updates->Next(1, &range, &count);
            changed = changed || FAILED(next) || count != 0;
            if (range) range->Release();
            updates->Release();
        }
    }
    // Invalidation is permanent even if the caret/text later returns to its old value.
    if (changed) {
        trace_tsf_event("origin.invalidated", S_OK, selection_changed);
        agent_origin_valid_ = false;
    }
    return S_OK;
}

bool TextService::plugin_scheme_selected() const noexcept {
    return config::is_plugin_input_scheme(shortcut_config_.input_scheme);
}

bool TextService::plugin_context_is_current(ITfContext* context) const {
    if (!foreground_focus_ || !thread_manager_ || !context) return false;
    ITfDocumentMgr* document = nullptr; ITfContext* top = nullptr;
    if (SUCCEEDED(thread_manager_->GetFocus(&document)) && document) {
        (void)document->GetTop(&top); document->Release();
    }
    const bool same = same_context(context, top);
    if (top) top->Release();
    return same;
}

NativeWriteResult TextService::apply_agent_effect(const agent::CommandEffect& effect) {
    if (effect.kind == agent::EffectKind::copy_text) {
        const agent::EffectTargetFacts facts{agent_effects_negotiated_, false, false, {}, {}};
        if (agent::decide_effect(effect, facts) != agent::EffectDecision::write)
            return NativeWriteResult::rejected;
        switch (write_clipboard_text(message_window_, wide_from_utf8(effect.text))) {
        case ClipboardWriteResult::completed: return NativeWriteResult::completed;
        case ClipboardWriteResult::unchanged: return NativeWriteResult::rejected;
        case ClipboardWriteResult::uncertain: return NativeWriteResult::uncertain;
        }
        return NativeWriteResult::uncertain;
    }
    // The immutable TSF capture, not just the current context, identifies the target.
    if (!agent_origin_valid_ || !agent_origin_context_ || !agent_origin_range_ ||
        effect.target_context_id.empty() || effect.target_context_id != agent_context_id_ ||
        !plugin_context_is_current(agent_origin_context_) ||
        (agent_origin_selection_.has_value() != (effect.kind == agent::EffectKind::replace_selection))) return NativeWriteResult::rejected;
    ITfContext* context = agent_origin_context_;
    context->AddRef();
    const std::wstring text = wide_from_utf8(effect.text);
    bool written = false;
    auto outcome = NativeWriteResult::rejected;
    auto* edit = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
        agent::EffectTargetFacts facts;
        facts.capability_negotiated = agent_effects_negotiated_;
        facts.same_document = agent_origin_valid_ && same_context(context, agent_origin_context_) &&
            plugin_context_is_current(context) && scheme_selection_matches(context, cookie, agent_origin_range_);
        facts.context_allowed = inspect_input_method_context(context, cookie) == InputMethodContextSafety::allowed;
        facts.current_context_id = agent_context_id_;
        TF_SELECTION selections[2]{};
        ULONG fetched = 0;
        const auto result = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 2, selections, &fetched);
        if (SUCCEEDED(result) && fetched == 1 && selections[0].range) {
            const auto current = read_range_text(cookie, selections[0].range);
            const auto digest = current ? sha256_hex(utf8_from_wide(*current)) : std::string{};
            facts.same_document = facts.same_document && !digest.empty() && digest == agent_origin_sha256_;
            if (effect.kind == agent::EffectKind::replace_selection) facts.selection_sha256 = digest;
            const auto decision = agent::decide_effect(effect, facts);
            if (decision != agent::EffectDecision::write)
                std::clog << "[agent-writeback] effect denied: decision=" << static_cast<int>(decision)
                          << " same_document=" << facts.same_document
                          << " context_allowed=" << facts.context_allowed << '\n';
            if (decision == agent::EffectDecision::write) {
                agent_origin_valid_ = false; // Consume before any write; never replay an attempted effect.
                const auto native = replace_richedit_selection(
                    context, cookie, selections[0].range, *current, text);
                outcome = native;
                if (native == NativeWriteResult::completed) {
                    written = true;
                } else if (native == NativeWriteResult::not_applicable &&
                           SUCCEEDED(selections[0].range->SetText(
                               cookie, 0, text.data(), static_cast<LONG>(text.size())))) {
                    outcome = NativeWriteResult::completed;
                    written = true; // A caret failure must never permit a second write.
                    (void)selections[0].range->Collapse(cookie, TF_ANCHOR_END);
                    (void)context->SetSelection(cookie, 1, selections);
                }
            }
        }
        for (auto& selection : selections) if (selection.range) selection.range->Release();
        return S_OK;
    });
    if (edit) {
        HRESULT session_result = E_FAIL;
        (void)context->RequestEditSession(client_id_, edit, TF_ES_SYNC | TF_ES_READWRITE, &session_result);
        edit->Release();
    }
    context->Release();
    if (written) append_language_context(text);
    return outcome;
}

void TextService::refresh_agent_context_safety(ITfContext* context) {
    // Unknown, read-only, disabled and sensitive contexts all keep Agent context off.
    agent_context_sensitive_ = true;
    if (!context) return;
    InputMethodContextSafety safety = InputMethodContextSafety::unknown;
    auto* edit = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
        safety = inspect_input_method_context(context, cookie);
        return S_OK;
    });
    if (!edit) return;
    HRESULT result = E_FAIL;
    const auto request = context->RequestEditSession(client_id_, edit, TF_ES_SYNC | TF_ES_READ, &result);
    edit->Release();
    agent_context_sensitive_ = !(SUCCEEDED(request) && SUCCEEDED(result) &&
                                 safety == InputMethodContextSafety::allowed);
}

bool TextService::plugin_context_allowed(ITfContext* context) {
    if (!plugin_context_is_current(context)) return false;
    InputMethodContextSafety safety = InputMethodContextSafety::unknown;
    auto* edit = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
        safety = inspect_input_method_context(context, cookie);
        if (!scheme_selection_matches(context, cookie, plugin_input_selection_)) safety = InputMethodContextSafety::unknown;
        return S_OK;
    });
    if (!edit) return false;
    HRESULT result = E_FAIL;
    const auto request = context->RequestEditSession(client_id_, edit, TF_ES_SYNC | TF_ES_READ, &result);
    edit->Release();
    return SUCCEEDED(request) && SUCCEEDED(result) && safety == InputMethodContextSafety::allowed;
}

bool TextService::plugin_should_eat_key(ITfContext* context, WPARAM key, LPARAM flags) {
    if (plugin_input_context_ && !same_context(context, plugin_input_context_)) clear_composition();
    if (shortcut_config_.language_shortcut_enabled &&
        shortcut_matches(shortcut_config_.language_shortcuts, key)) return true;
    if (!chinese_mode_ || !plugin_context_allowed(context)) return false;
    if (has_composition()) {
        if (key == VK_PRIOR || key == VK_NEXT) return !command_modifier_down();
        if (key == VK_ESCAPE || key == VK_BACK || key == VK_DELETE || key == VK_RETURN || key == VK_SPACE)
            return !command_modifier_down();
        if (shortcut_config_.raw_input_shortcut_enabled && shortcut_matches(shortcut_config_.raw_input_shortcuts, key)) return true;
        if (shortcut_config_.cursor_left_shortcut_enabled && shortcut_matches(shortcut_config_.cursor_left_shortcuts, key)) return true;
        if (shortcut_config_.cursor_right_shortcut_enabled && shortcut_matches(shortcut_config_.cursor_right_shortcuts, key)) return true;
        if (shortcut_config_.previous_page_shortcut_enabled && shortcut_matches(shortcut_config_.previous_page_shortcuts, key)) return true;
        if (shortcut_config_.next_page_shortcut_enabled && shortcut_matches(shortcut_config_.next_page_shortcuts, key)) return true;
        if (key >= '1' && key <= '9' && !command_modifier_down() && !key_down(VK_SHIFT)) return true;
    }
    const auto text = scheme_key_text(key, flags);
    if (text.empty()) return false;
    if (plugin_input_committing_) return true; // defer until the prepared write completes
    if (!plugin_input_descriptor_) return key != VK_SPACE; // open asynchronously; preserve raw text if rejected
    auto probe = plugin_input_document_.composition();
    return probe.insert(utf8_from_wide(text));
}

void TextService::reset_plugin_input() {
    plugin_input_document_.invalidate();
    if (plugin_input_client_) (void)plugin_input_client_->close();
    plugin_input_future_ = {};
    plugin_pending_selection_.reset();
    plugin_input_descriptor_.reset(); plugin_input_session_.clear();
    plugin_input_opening_ = plugin_input_committing_ = false;
    plugin_deferred_keys_.clear();
    release_interface(plugin_input_context_);
    release_interface(plugin_input_selection_);
    release_interface(plugin_replay_context_);
    release_interface(plugin_replay_selection_);
    plugin_input_needs_compose_ = false;
    plugin_switch_after_commit_ = false;
    if (message_window_) KillTimer(message_window_, kPluginInputTimer);
}

HRESULT TextService::handle_plugin_key(ITfContext* context, WPARAM key, LPARAM flags, BOOL* eaten) {
    if (plugin_replay_context_) finish_plugin_input_on_ui();
    *eaten = plugin_should_eat_key(context, key, flags) ? TRUE : FALSE;
    if (!*eaten) return S_OK;
    if (shortcut_config_.language_shortcut_enabled && shortcut_matches(shortcut_config_.language_shortcuts, key))
        return toggle_language_mode(context);
    if (!plugin_context_allowed(context)) { *eaten = FALSE; return S_OK; }
    if (plugin_input_context_ && !same_context(context, plugin_input_context_)) clear_composition();
    if (!plugin_input_context_) {
        plugin_input_context_ = context; context->AddRef();
        auto* capture = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
            TF_SELECTION selection{}; ULONG count{};
            if (SUCCEEDED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count)) && count == 1)
                plugin_input_selection_ = selection.range;
            else if (selection.range) selection.range->Release();
            return plugin_input_selection_ ? S_OK : E_FAIL;
        });
        HRESULT captured = E_FAIL;
        const auto request = capture ? context->RequestEditSession(client_id_, capture, TF_ES_SYNC | TF_ES_READ, &captured) : E_FAIL;
        if (capture) capture->Release();
        if (FAILED(request) || FAILED(captured)) { reset_plugin_input(); *eaten = FALSE; return S_OK; }
    }
    update_candidate_anchor(context);
    if (key == VK_ESCAPE) { clear_composition(); return S_OK; }
    if (plugin_input_committing_) {
        PluginDeferredKey deferred{key, flags, {}};
        if (GetKeyboardState(deferred.state)) plugin_deferred_keys_.push_back(deferred);
        else *eaten = FALSE;
        return S_OK;
    }
    if (key == VK_RETURN || (shortcut_config_.raw_input_shortcut_enabled &&
        shortcut_matches(shortcut_config_.raw_input_shortcuts, key))) {
        commit_plugin_text(input_buffer_, false); return S_OK;
    }
    if (key == VK_SPACE) { select_plugin_candidate(0, true); return S_OK; }
    if (has_composition() && key >= '1' && key <= '9' && !key_down(VK_SHIFT) && !command_modifier_down()) {
        select_plugin_candidate(static_cast<std::size_t>(key - '1'), true); return S_OK;
    }
    if (has_composition() && shortcut_config_.previous_page_shortcut_enabled && shortcut_matches(shortcut_config_.previous_page_shortcuts, key)) {
        if (candidates_expanded_) scroll_expanded_candidates(-1); else change_plugin_page(-1);
        return S_OK;
    }
    if (has_composition() && shortcut_config_.next_page_shortcut_enabled && shortcut_matches(shortcut_config_.next_page_shortcuts, key)) {
        if (candidates_expanded_) scroll_expanded_candidates(1); else change_plugin_page(1);
        return S_OK;
    }
    if (key == VK_PRIOR || key == VK_NEXT) { change_plugin_page(key == VK_PRIOR ? -1 : 1); return S_OK; }
    if (!plugin_input_descriptor_) {
        // The descriptor is not known until open completes. Raw encoding never disappears on open failure.
        plugin_pending_selection_.reset();
        input_cursor_ = std::min(input_cursor_, input_buffer_.size());
        if (key == VK_BACK) {
            if (input_cursor_) {
                auto begin = input_cursor_ - 1;
                if (begin && input_buffer_[begin] >= 0xdc00 && input_buffer_[begin] <= 0xdfff &&
                    input_buffer_[begin - 1] >= 0xd800 && input_buffer_[begin - 1] <= 0xdbff) --begin;
                input_buffer_.erase(begin, input_cursor_ - begin); input_cursor_ = begin;
            }
        } else if (key == VK_DELETE) {
            if (input_cursor_ < input_buffer_.size()) {
                auto size = std::size_t{1};
                if (input_buffer_[input_cursor_] >= 0xd800 && input_buffer_[input_cursor_] <= 0xdbff &&
                    input_cursor_ + 1 < input_buffer_.size()) size = 2;
                input_buffer_.erase(input_cursor_, size);
            }
        } else {
            const auto text = scheme_key_text(key, flags);
            if (utf8_from_wide(input_buffer_).size() + utf8_from_wide(text).size() <= 16384) {
                input_buffer_.insert(input_cursor_, text); input_cursor_ += text.size();
            }
        }
        if (input_buffer_.empty()) { clear_composition(); return S_OK; }
        if (!plugin_input_opening_) {
            if (!plugin_input_client_) plugin_input_client_ = std::make_unique<ipc::InputMethodAsyncClient>();
            (void)plugin_input_document_.enter(++context_generation_);
            const auto id = shortcut_config_.input_scheme.substr(7);
            plugin_input_session_ = new_agent_session_id();
            plugin_input_future_ = plugin_input_client_->open(context_generation_,
                id.substr(0, id.find('/')), id, plugin_input_session_, false);
            plugin_input_opening_ = candidate_request_pending_ = true;
            candidate_request_failed_ = false; candidate_failure_detail_.clear();
            SetTimer(message_window_, kPluginInputTimer, 16, nullptr);
        }
        segmented_input_.clear(); update_candidate_window(); return S_OK;
    }
    auto& composition = plugin_input_document_.composition();
    bool changed = false;
    if (key == VK_BACK) changed = composition.backspace();
    else if (key == VK_DELETE) changed = composition.erase_forward();
    else if (shortcut_config_.cursor_left_shortcut_enabled && shortcut_matches(shortcut_config_.cursor_left_shortcuts, key))
        changed = composition.move_caret(-1);
    else if (shortcut_config_.cursor_right_shortcut_enabled && shortcut_matches(shortcut_config_.cursor_right_shortcuts, key))
        changed = composition.move_caret(1);
    else changed = composition.insert(utf8_from_wide(scheme_key_text(key, flags)));
    if (changed) {
        plugin_pending_selection_.reset();
        if (composition.input().empty()) { clear_composition(); return S_OK; }
        candidate_page_ = 0; sync_plugin_display(); queue_plugin_compose();
    }
    return S_OK;
}

void TextService::sync_plugin_display() {
    const auto& composition = plugin_input_document_.composition();
    if (plugin_input_descriptor_) {
        input_buffer_ = wide_from_utf8(composition.input()); input_cursor_ = composition.caret_utf16();
    }
    // Display raw source for caret fidelity until the provider supports an explicit preview mapping.
    segmented_input_.clear(); candidate_groups_.clear(); candidate_consumed_.clear();
    candidate_page_size_ = shortcut_config_.candidate_page_size;
    const auto& display = composition.display();
    if (display) {
        candidates_.clear();
        for (const auto& candidate : display->candidates) candidates_.push_back(wide_from_utf8(candidate.text));
        has_more_candidates_ = display->has_more;
    }
    update_candidate_window();
}

void TextService::queue_plugin_compose() {
    if (!plugin_input_descriptor_ || plugin_input_committing_ || !plugin_input_client_ ||
        !plugin_context_allowed(plugin_input_context_)) return;
    auto request = plugin_input_document_.composition().compose_request(++plugin_input_request_id_,
        static_cast<std::size_t>(candidate_page_), candidates_expanded_
            ? plugin_input_descriptor_->max_candidates : std::min<std::size_t>(
                plugin_input_descriptor_->max_candidates, shortcut_config_.candidate_page_size));
    if (!request) return;
    plugin_input_future_ = plugin_input_client_->submit(plugin_input_document_.generation(), std::move(*request), false);
    candidate_request_pending_ = true; candidate_request_failed_ = false; candidate_failure_detail_.clear();
    SetTimer(message_window_, kPluginInputTimer, 16, nullptr);
    sync_plugin_display();
}

void TextService::select_plugin_candidate(std::size_t index, bool defer) {
    auto& composition = plugin_input_document_.composition();
    if (defer && candidate_request_pending_ && !plugin_input_committing_) {
        plugin_pending_selection_ = PluginPendingSelection{index, input_buffer_, plugin_input_document_.generation(), candidate_page_};
        return;
    }
    if (plugin_input_committing_ || !composition.selectable() || !composition.display() ||
        index >= composition.display()->candidates.size() || !plugin_context_allowed(plugin_input_context_)) return;
    auto request = composition.select_request(++plugin_input_request_id_, composition.display()->candidates[index].id);
    if (!request) return;
    plugin_input_future_ = plugin_input_client_->submit(plugin_input_document_.generation(), std::move(*request), false);
    candidate_request_pending_ = true;
    SetTimer(message_window_, kPluginInputTimer, 16, nullptr);
    update_candidate_window();
}

void TextService::change_plugin_page(int direction) {
    if (plugin_input_committing_ || candidate_request_pending_ || !plugin_input_descriptor_) return;
    if (direction > 0 && has_more_candidates_) ++candidate_page_;
    else if (direction < 0 && candidate_page_) --candidate_page_;
    else return;
    expanded_scroll_row_ = 0;
    queue_plugin_compose();
}

void TextService::poll_plugin_input() {
    if (!plugin_input_context_) return; // ignore a retired WM_TIMER after the final commit
    if (!plugin_scheme_selected() || !plugin_context_is_current(plugin_input_context_)) { clear_composition(); return; }
    if (plugin_input_committing_) {
        if (GetTickCount64() >= plugin_commit_deadline_) {
            // Invalidate the queued edit before allowing another one; late callbacks cannot write.
            plugin_input_document_.invalidate(); plugin_input_descriptor_.reset();
            plugin_input_committing_ = false; plugin_input_opening_ = false;
            if (plugin_input_client_) (void)plugin_input_client_->close();
            candidate_request_pending_ = false; candidate_request_failed_ = true;
            candidate_failure_detail_ = L"上屏请求超时；原编码已保留";
            release_interface(plugin_replay_context_);
            plugin_replay_context_ = plugin_input_context_; plugin_replay_context_->AddRef();
            release_interface(plugin_replay_selection_);
            plugin_replay_selection_ = plugin_input_selection_;
            if (plugin_replay_selection_) plugin_replay_selection_->AddRef();
            plugin_replay_generation_ = context_generation_;
            PostMessageW(message_window_, kPluginCommitReady, 0, 0);
            update_candidate_window();
        }
        return;
    }
    if (!plugin_context_allowed(plugin_input_context_)) { clear_composition(); return; }
    if (!plugin_input_future_.valid() || plugin_input_future_.wait_for(std::chrono::seconds(0)) != std::future_status::ready) return;
    ipc::InputMethodClientResult result;
    try { result = plugin_input_future_.get(); }
    catch (...) {
        plugin_input_opening_ = candidate_request_pending_ = false; candidate_request_failed_ = true;
        candidate_failure_detail_ = L"方案服务异常；编码已保留"; update_candidate_window(); return;
    }
    SetTimer(message_window_, kPluginInputTimer, 100, nullptr); // monitor focus/selection while the list is idle
    if (result.document_generation != plugin_input_document_.generation()) return;
    candidate_request_pending_ = false;
    if (!plugin_context_allowed(plugin_input_context_)) { clear_composition(); return; }
    if (!result.status) {
        plugin_input_opening_ = false; candidate_request_failed_ = true;
        candidate_failure_detail_ = L"输入方案不可用或授权已撤销；编码已保留，可按 Enter 原样上屏";
        update_candidate_window(); return;
    }
    if (plugin_input_opening_) {
        plugin_input_opening_ = false;
        if (!plugin_input_document_.bind(result, plugin_input_session_)) return;
        plugin_input_descriptor_ = result.descriptors.front();
        auto& composition = plugin_input_document_.composition();
        if (!composition.insert(utf8_from_wide(input_buffer_))) {
            plugin_input_descriptor_.reset(); candidate_request_failed_ = true;
            candidate_failure_detail_ = L"方案不接受当前编码；可删除修改或按 Enter 原样上屏";
            update_candidate_window(); return;
        }
        (void)composition.set_caret_utf16(input_cursor_);
        queue_plugin_compose(); return;
    }
    const auto applied = plugin_input_document_.apply(result);
    if (!applied.accepted) {
        candidate_request_failed_ = true; candidate_failure_detail_ = L"方案返回无效候选；编码已保留";
        update_candidate_window(); return;
    }
    if (applied.commit) { commit_plugin_text(wide_from_utf8(applied.commit->text), true); return; }
    if (result.response && result.response->status != engine::InputMethodStatus::success) {
        candidate_request_failed_ = true;
        candidate_failure_detail_ = L"输入方案暂不可用；编码已保留";
    }
    sync_plugin_display();
    if (auto selected = std::exchange(plugin_pending_selection_, {}); selected &&
        selected->input == input_buffer_ && selected->generation == plugin_input_document_.generation() &&
        selected->page == candidate_page_) select_plugin_candidate(selected->index);
}

void TextService::commit_plugin_text(std::wstring text, bool selected, bool switch_language) {
    if (text.empty() || plugin_input_committing_ || !plugin_context_allowed(plugin_input_context_)) return;
    if (!plugin_input_document_.generation()) (void)plugin_input_document_.enter(++context_generation_);
    const auto generation = plugin_input_document_.generation();
    auto* context = plugin_input_context_;
    if (!selected) {
        plugin_input_future_ = {};
        plugin_pending_selection_.reset();
        if (plugin_input_client_) (void)plugin_input_client_->close();
        plugin_input_opening_ = false; candidate_request_pending_ = false;
    }
    plugin_input_committing_ = true;
    plugin_commit_deadline_ = GetTickCount64() + 2000;
    const auto deadline = plugin_commit_deadline_;
    SetTimer(message_window_, kPluginInputTimer, 16, nullptr);
    auto* edit = new (std::nothrow) SchemeEditSession(context,
        [this, context, generation, deadline, selected, switch_language, text = std::move(text)](TfEditCookie cookie) {
            const bool current = plugin_context_is_current(context);
            const bool valid = input_method_write_allowed({generation, plugin_input_document_.generation(),
                deadline, plugin_commit_deadline_, GetTickCount64(), plugin_scheme_selected(),
                plugin_input_committing_, current, current && scheme_selection_matches(context, cookie, plugin_input_selection_),
                current ? inspect_input_method_context(context, cookie) : InputMethodContextSafety::unknown});
            HRESULT written = E_ABORT;
            if (valid) {
                TF_SELECTION selection{}; ULONG count{};
                written = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count);
                if (SUCCEEDED(written) && count == 1 && selection.range) {
                    written = selection.range->SetText(cookie, 0, text.data(), static_cast<LONG>(text.size()));
                    if (SUCCEEDED(written)) {
                        // SetText already committed even if caret positioning fails: never duplicate it on retry.
                        (void)selection.range->Collapse(cookie, TF_ANCHOR_END);
                        (void)context->SetSelection(cookie, 1, &selection);
                        release_interface(plugin_input_selection_);
                        TF_SELECTION fresh{}; ULONG fetched{};
                        if (SUCCEEDED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &fresh, &fetched)) && fetched == 1)
                            plugin_input_selection_ = fresh.range;
                        else if (fresh.range) fresh.range->Release();
                    }
                } else written = E_FAIL;
                if (selection.range) selection.range->Release();
            }
            if (plugin_input_document_.generation() != generation || plugin_commit_deadline_ != deadline) return written;
            auto* replay_selection = plugin_input_selection_;
            if (replay_selection) replay_selection->AddRef();
            plugin_input_committing_ = false;
            if (selected) (void)plugin_input_document_.finish_commit(generation, SUCCEEDED(written));
            auto deferred = std::move(plugin_deferred_keys_);
            if (SUCCEEDED(written)) {
                if (!selected || plugin_input_document_.composition().input().empty()) clear_composition();
                else { candidate_page_ = 0; sync_plugin_display(); plugin_input_needs_compose_ = true; }
            } else {
                candidate_request_failed_ = true; candidate_failure_detail_ = L"上屏失败；编码已保留";
                update_candidate_window();
            }
            // Re-enter outside the TSF write lock; nested read sessions would otherwise drop queued keys.
            if (valid) {
                plugin_switch_after_commit_ = switch_language && SUCCEEDED(written);
                plugin_deferred_keys_ = std::move(deferred);
                release_interface(plugin_replay_context_);
                plugin_replay_context_ = context; context->AddRef();
                release_interface(plugin_replay_selection_);
                plugin_replay_selection_ = replay_selection;
                plugin_replay_generation_ = context_generation_;
                PostMessageW(message_window_, kPluginCommitReady, 0, 0);
            } else if (replay_selection) replay_selection->Release();
            return written;
        }, static_cast<ITfTextInputProcessorEx*>(this));
    HRESULT result = E_OUTOFMEMORY, request = E_OUTOFMEMORY;
    if (edit) {
        request = context->RequestEditSession(client_id_, edit, TF_ES_ASYNC | TF_ES_READWRITE, &result);
        edit->Release();
    }
    if (FAILED(request) || FAILED(result)) {
        plugin_input_committing_ = false;
        if (selected) (void)plugin_input_document_.finish_commit(generation, false);
        candidate_request_failed_ = true; candidate_failure_detail_ = L"无法申请上屏会话；编码已保留";
        update_candidate_window();
    }
}

void TextService::finish_plugin_input_on_ui() {
    auto* context = std::exchange(plugin_replay_context_, nullptr);
    auto* selection = std::exchange(plugin_replay_selection_, nullptr);
    if (!context) { if (selection) selection->Release(); return; }
    bool same_selection = false;
    auto* check = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
        same_selection = selection && scheme_selection_matches(context, cookie, selection); return S_OK;
    });
    HRESULT checked = E_FAIL;
    const auto request = check ? context->RequestEditSession(client_id_, check, TF_ES_SYNC | TF_ES_READ, &checked) : E_FAIL;
    if (check) check->Release();
    const bool valid = plugin_replay_generation_ == context_generation_ && plugin_scheme_selected() &&
        plugin_context_is_current(context) && SUCCEEDED(request) && SUCCEEDED(checked) && same_selection;
    if (valid) {
        if (std::exchange(plugin_switch_after_commit_, false)) {
            // Keys entered while switching to English are raw output, never new scheme selections.
            std::wstring tail;
            BYTE current[256]{}; GetKeyboardState(current);
            for (auto& key : plugin_deferred_keys_) {
                SetKeyboardState(key.state);
                if (key.key == VK_BACK) {
                    if (!tail.empty()) {
                        const auto last = tail.back(); tail.pop_back();
                        if (last >= 0xdc00 && last <= 0xdfff && !tail.empty() &&
                            tail.back() >= 0xd800 && tail.back() <= 0xdbff) tail.pop_back();
                    }
                }
                else if (key.key == VK_RETURN) tail += L"\r\n";
                else if (key.key == VK_SPACE) tail += L' ';
                else tail += scheme_key_text(key.key, key.flags);
            }
            SetKeyboardState(current); plugin_deferred_keys_.clear();
            if (tail.empty()) { chinese_mode_ = false; clear_composition(); update_language_bar(); }
            else {
                if (!plugin_input_context_) { plugin_input_context_ = context; context->AddRef(); }
                if (!plugin_input_selection_) { plugin_input_selection_ = selection; selection->AddRef(); }
                input_buffer_ = std::move(tail); input_cursor_ = input_buffer_.size();
                (void)plugin_input_document_.enter(++context_generation_);
                commit_plugin_text(input_buffer_, false, true);
            }
            if (selection) selection->Release();
            context->Release(); return;
        }
        if (std::exchange(plugin_input_needs_compose_, false)) queue_plugin_compose();
        auto deferred = std::move(plugin_deferred_keys_);
        BYTE current[256]{}; GetKeyboardState(current);
        for (auto& key : deferred) {
            if (!plugin_context_is_current(context)) break;
            SetKeyboardState(key.state); BOOL eaten{};
            (void)handle_plugin_key(context, key.key, key.flags, &eaten);
            if (!eaten) {
                const auto text = scheme_key_text(key.key, key.flags);
                if (!text.empty()) {
                    // A key accepted before the write may now exceed the descriptor limit. Preserve it locally.
                    input_buffer_ += text; input_cursor_ = input_buffer_.size();
                    plugin_input_document_.invalidate(); plugin_input_descriptor_.reset();
                    candidate_request_failed_ = true;
                    candidate_failure_detail_ = L"编码暂不可解析；内容已保留，可按 Enter 原样上屏";
                    update_candidate_window();
                }
            }
        }
        SetKeyboardState(current);
    } else {
        plugin_deferred_keys_.clear(); plugin_input_needs_compose_ = false;
        plugin_switch_after_commit_ = false;
    }
    if (selection) selection->Release();
    context->Release();
}

HRESULT create_text_service(REFIID iid, void** object) {
    if (object == nullptr) return E_POINTER;
    *object = nullptr;
    auto* service = new (std::nothrow) TextService();
    if (service == nullptr) return E_OUTOFMEMORY;
    const HRESULT result = service->QueryInterface(iid, object);
    service->Release();
    return result;
}

void increment_server_lock() noexcept { InterlockedIncrement(&lock_count); }
void decrement_server_lock() noexcept { InterlockedDecrement(&lock_count); }
long server_lock_count() noexcept { return object_count + lock_count; }

}  // namespace owo::tsf

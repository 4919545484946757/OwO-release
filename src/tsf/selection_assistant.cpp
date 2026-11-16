#include "selection_assistant.h"
#include "activation_trace.h"
#include "text_service.h"
#include "owo/config/config_paths.h"
#include "owo/tsf/input_method_context.h"
#include "owo/tsf/language_context.h"
#include <algorithm>
#include <array>
#include <filesystem>
#include <functional>
#include <vector>

namespace owo::tsf {
namespace {
using Microsoft::WRL::ComPtr;
constexpr const wchar_t* labels[]{L"语言翻译", L"语言润色", L"代码纠错", L"内容生成"};
constexpr const char* operations[]{"translate", "polish", "code", "generate"};
class Session final : public ITfEditSession {
public:
    explicit Session(std::function<HRESULT(TfEditCookie)> fn) : fn_(std::move(fn)) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        if (!out) return E_POINTER; *out = nullptr;
        if (id != IID_IUnknown && id != IID_ITfEditSession) return E_NOINTERFACE;
        *out = static_cast<ITfEditSession*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&refs_); }
    ULONG STDMETHODCALLTYPE Release() override { const auto refs = InterlockedDecrement(&refs_); if (!refs) delete this; return refs; }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override { return fn_(cookie); }
private:
    LONG refs_{1}; std::function<HRESULT(TfEditCookie)> fn_;
};
bool edit(ITfContext* context, TfClientId client, bool write, std::function<HRESULT(TfEditCookie)> fn) {
    if (!context) return false;
    auto* session = new (std::nothrow) Session(std::move(fn));
    if (!session) return false;
    HRESULT result = E_FAIL;
    const auto hr = context->RequestEditSession(client, session, TF_ES_SYNC | (write ? TF_ES_READWRITE : TF_ES_READ), &result);
    session->Release();
    return SUCCEEDED(hr) && SUCCEEDED(result);
}
std::filesystem::path models_root() {
    wchar_t override_path[32768]{};
    const auto length = GetEnvironmentVariableW(L"OWO_CONFIG_PATH", override_path, 32768);
    const auto path = length != 0 && length < 32768 ? std::filesystem::path(override_path) : config::default_config_path();
    return path.parent_path().parent_path() / L"models";
}
bool gate(const wchar_t* name) {
    const auto path = models_root() / name;
    const auto attributes = GetFileAttributesW(path.c_str());
    return attributes != INVALID_FILE_ATTRIBUTES && !(attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT));
}
bool trigger(WPARAM key) {
    return key == 'V' && GetKeyState(VK_SHIFT) >= 0 && GetKeyState(VK_CONTROL) >= 0 &&
        GetKeyState(VK_MENU) >= 0 && GetKeyState(VK_LWIN) >= 0 && GetKeyState(VK_RWIN) >= 0;
}
HWND focus() { GUITHREADINFO info{sizeof(info)}; return GetGUIThreadInfo(GetCurrentThreadId(), &info) ? info.hwndFocus : nullptr; }
struct Snapshot {
    ComPtr<ITfRange> range;
    std::wstring text, context;
    EditWindowIdentity window{};
    bool needs_trust{};
};
std::optional<Snapshot> capture(ITfContext* context, TfClientId client, bool with_context,
                               bool metadata_only = false, const EditWindowIdentity& trusted = {},
                               ITfRange* expected = nullptr) {
    wchar_t executable[32768]{};
    GetModuleFileNameW(nullptr, executable, 32768);
    auto application = std::filesystem::path(executable).filename().wstring();
    std::transform(application.begin(), application.end(), application.begin(), [](wchar_t c) { return static_cast<wchar_t>(towlower(c)); });
    for (const auto* blocked : {L"windowsterminal.exe", L"cmd.exe", L"powershell.exe", L"pwsh.exe", L"conhost.exe", L"wsl.exe"})
        if (application == blocked) return std::nullopt;
    Snapshot snapshot;
    const auto ok = edit(context, client, false, [&](TfEditCookie cookie) {
        const auto resolution = resolve_input_context(context, cookie);
        if (!resolution.ordinary_input) return E_ACCESSDENIED;
        snapshot.window = edit_window_identity(resolution);
        if (expected && snapshot.window != trusted) return E_ACCESSDENIED;
        snapshot.needs_trust = !advanced_edit_allowed(resolution, trusted);
        TF_SELECTION selected{}; ULONG count{};
        if (FAILED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selected, &count)) || count != 1 || !selected.range) return E_FAIL;
        snapshot.range.Attach(selected.range);
        if (selected.style.fInterimChar) return E_FAIL;
        BOOL empty = TRUE;
        if (FAILED(snapshot.range->IsEmpty(cookie, &empty)) || empty) return E_FAIL;
        if (expected) {
            LONG start{}, end{};
            if (FAILED(snapshot.range->CompareStart(cookie, expected, TF_ANCHOR_START, &start)) || start ||
                FAILED(snapshot.range->CompareEnd(cookie, expected, TF_ANCHOR_END, &end)) || end) return E_ABORT;
        }
        // Neither the key probe nor the trust prompt reads protected text.
        if (metadata_only || (snapshot.needs_trust && !expected)) return S_OK;
        if (snapshot.needs_trust) return E_ACCESSDENIED;
        wchar_t buffer[8193]{}; ULONG read{};
        if (FAILED(snapshot.range->GetText(cookie, 0, buffer, 8193, &read)) || read == 0 || read > 8192) return E_FAIL;
        snapshot.text.assign(buffer, read);
        if (snapshot.text.find(L'\0') != std::wstring::npos ||
            std::all_of(snapshot.text.begin(), snapshot.text.end(), [](wchar_t c) { return iswspace(c) != 0; })) return E_FAIL;
        if (with_context) {
            const auto nearby = [&](bool before) {
                ComPtr<ITfRange> range;
                if (FAILED(snapshot.range->Clone(&range))) return std::wstring{};
                range->Collapse(cookie, before ? TF_ANCHOR_START : TF_ANCHOR_END);
                LONG moved{};
                const auto hr = before ? range->ShiftStart(cookie, -1000, &moved, nullptr) : range->ShiftEnd(cookie, 1000, &moved, nullptr);
                wchar_t text[1000]{}; ULONG size{};
                if (FAILED(hr) || FAILED(range->GetText(cookie, 0, text, 1000, &size))) return std::wstring{};
                return std::wstring(text, size);
            };
            snapshot.context = nearby(true) + L"\n[选区]\n" + nearby(false);
        }
        return S_OK;
    });
    return ok ? std::optional<Snapshot>(std::move(snapshot)) : std::nullopt;
}
std::string hex(const std::wstring& text) {
    const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(), static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
    if (size < 0 || (!text.empty() && size == 0)) return {};
    std::string utf8(size, '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(), static_cast<int>(text.size()), utf8.data(), size, nullptr, nullptr);
    constexpr char digits[] = "0123456789ABCDEF";
    std::string output; output.reserve(utf8.size() * 2);
    for (const auto byte : utf8) { output += digits[static_cast<unsigned char>(byte) >> 4]; output += digits[byte & 15]; }
    return output;
}
std::wstring unhex(std::string_view text) {
    if (text.size() % 2 != 0 || text.size() > 131072) return {};
    const auto digit = [](char c) { if (c >= '0' && c <= '9') return c - '0'; if (c >= 'A' && c <= 'F') return c - 'A' + 10; if (c >= 'a' && c <= 'f') return c - 'a' + 10; return -1; };
    std::string bytes;
    for (std::size_t i = 0; i < text.size(); i += 2) {
        const int high = digit(text[i]), low = digit(text[i + 1]);
        if (high < 0 || low < 0) return {};
        bytes += static_cast<char>((high << 4) | low);
    }
    const int length = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(), static_cast<int>(bytes.size()), nullptr, 0);
    if (length <= 0 || length > 16384) return {};
    std::wstring output(length, L'\0');
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(), static_cast<int>(bytes.size()), output.data(), length);
    if (output.find(L'\0') != std::wstring::npos) return {};
    return output;
}
std::filesystem::path host_path() {
    HMODULE module{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(&host_path), &module)) return {};
    wchar_t path[32768]{};
    if (!GetModuleFileNameW(module, path, 32768)) return {};
    const auto parent = std::filesystem::path(path).parent_path().parent_path();
    for (const auto& candidate : {parent / L"settings" / L"OwO.Settings.exe", parent / L"settings-publish" / L"win-x64" / L"OwO.Settings.exe"}) {
        std::error_code error;
        if (std::filesystem::is_regular_file(candidate, error)) return candidate;
    }
    return {};
}
bool transfer(HANDLE pipe, void* buffer, DWORD count, bool write, std::stop_token stop) {
    DWORD offset{};
    const auto deadline = GetTickCount64() + 125000;
    while (offset < count && !stop.stop_requested()) {
        OVERLAPPED overlapped{}; overlapped.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (!overlapped.hEvent) return false;
        DWORD actual{};
        const BOOL ok = write ? WriteFile(pipe, static_cast<char*>(buffer) + offset, count - offset, &actual, &overlapped)
                              : ReadFile(pipe, static_cast<char*>(buffer) + offset, count - offset, &actual, &overlapped);
        bool pending = !ok && GetLastError() == ERROR_IO_PENDING;
        bool success = ok != FALSE;
        if (pending) {
            while (!stop.stop_requested() && GetTickCount64() < deadline && WaitForSingleObject(overlapped.hEvent, 50) == WAIT_TIMEOUT) {}
            if (stop.stop_requested() || GetTickCount64() >= deadline) CancelIoEx(pipe, &overlapped);
            success = GetOverlappedResult(pipe, &overlapped, &actual, TRUE) != FALSE && !stop.stop_requested();
        }
        CloseHandle(overlapped.hEvent);
        if (!success || actual == 0) return false;
        offset += actual;
    }
    return offset == count && !stop.stop_requested();
}
std::pair<bool, std::wstring> request(const std::filesystem::path& exe, const std::string& payload,
                                     std::stop_token stop, const bool zero = false) {
    GUID guid{}; if (FAILED(CoCreateGuid(&guid))) return {false, L"无法创建请求标识"};
    wchar_t id[40]{}; StringFromGUID2(guid, id, 40);
    std::wstring nonce; for (wchar_t c : std::wstring(id)) if (iswxdigit(c)) nonce += c;
    const auto name = L"OwO.TextAssistant." + nonce;
    auto command = L"\"" + exe.wstring() + L"\" --text-assistant-host " + name + L" --parent-pid " + std::to_wstring(GetCurrentProcessId());
    STARTUPINFOW startup{sizeof(startup)}; PROCESS_INFORMATION process{};
    if (!CreateProcessW(exe.c_str(), command.data(), nullptr, nullptr, FALSE, CREATE_NO_WINDOW, nullptr, exe.parent_path().c_str(), &startup, &process)) {
        if (zero) activation_trace("ZERO_HELPER_START_FAILED", HRESULT_FROM_WIN32(GetLastError()));
        return {false, L"无法启动模型辅助进程，请更新设置中心"};
    }
    CloseHandle(process.hThread);
    HANDLE pipe = INVALID_HANDLE_VALUE;
    const auto deadline = GetTickCount64() + 15000;
    const auto full_name = L"\\\\.\\pipe\\" + name;
    while (!stop.stop_requested() && GetTickCount64() < deadline && WaitForSingleObject(process.hProcess, 0) == WAIT_TIMEOUT) {
        pipe = CreateFileW(full_name.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, FILE_FLAG_OVERLAPPED, nullptr);
        if (pipe != INVALID_HANDLE_VALUE) break;
        std::this_thread::sleep_for(std::chrono::milliseconds(50));
    }
    bool ok = false; std::wstring text = L"模型辅助进程连接失败，原文未修改";
    if (pipe != INVALID_HANDLE_VALUE) {
        ULONG server{};
        if (GetNamedPipeServerProcessId(pipe, &server) && server == process.dwProcessId) {
            std::uint32_t size = static_cast<std::uint32_t>(payload.size());
            if (transfer(pipe, &size, 4, true, stop) && transfer(pipe, const_cast<char*>(payload.data()), size, true, stop) &&
                transfer(pipe, &size, 4, false, stop) && size <= 128 * 1024 && size != 0) {
                std::string response(size, '\0');
                if (transfer(pipe, response.data(), size, false, stop)) {
                    const auto split = response.find('\n', 9);
                    if (response.starts_with("OWOTEXT1\n") && split != std::string::npos && response.back() == '\n' &&
                        response.find('\n', split + 1) == response.size() - 1) {
                        text = unhex(std::string_view(response).substr(split + 1, response.size() - split - 2));
                        const auto status = response.substr(9, split - 9);
                        ok = status == "OK" && !text.empty();
                        if (status != "OK" && status != "ERROR") text.clear();
                    }
                }
            }
        }
        CloseHandle(pipe);
    }
    if (zero && !stop.stop_requested()) {
        // Never log the response text. Only record fixed, known categories
        // (and numeric HTTP status) from the authenticated local worker.
        if (ok) activation_trace("ZERO_MODEL_RETURNED");
        else if (pipe == INVALID_HANDLE_VALUE) activation_trace("ZERO_HELPER_NO_PIPE", E_FAIL);
        else if (text == L"zero-error:consent") activation_trace("ZERO_MODEL_CONSENT", E_ACCESSDENIED);
        else if (text == L"zero-error:input") activation_trace("ZERO_MODEL_INPUT", E_INVALIDARG);
        else if (text == L"zero-error:sensitive") activation_trace("ZERO_MODEL_SENSITIVE", E_ACCESSDENIED);
        else if (text == L"zero-error:timeout") activation_trace("ZERO_MODEL_TIMEOUT", HRESULT_FROM_WIN32(ERROR_TIMEOUT));
        else if (text == L"zero-error:network") activation_trace("ZERO_MODEL_NETWORK", E_FAIL);
        else if (text == L"zero-error:length") activation_trace("ZERO_MODEL_TRUNCATED", E_FAIL);
        else if (text == L"zero-error:content") activation_trace("ZERO_MODEL_CONTENT", E_FAIL);
        else if (text == L"zero-error:pronunciation") activation_trace("ZERO_MODEL_PRONUNCIATION", E_FAIL);
        else if (text == L"zero-error:local-not-installed") activation_trace("ZERO_LOCAL_NOT_READY", E_FAIL);
        else if (text == L"zero-error:local-busy") activation_trace("ZERO_LOCAL_BUSY", E_FAIL);
        else if (text == L"zero-error:local-start") activation_trace("ZERO_LOCAL_START_FAILED", E_FAIL);
        else if (text == L"zero-error:payload-or-config") activation_trace("ZERO_MODEL_PAYLOAD_OR_CONFIG", E_FAIL);
        else if (text.size() == 19 && text.starts_with(L"zero-error:http-") &&
                 std::all_of(text.begin() + 16, text.end(), [](wchar_t c) { return c >= L'0' && c <= L'9'; })) {
            const unsigned status = (text[16] - L'0') * 100 + (text[17] - L'0') * 10 + (text[18] - L'0');
            activation_trace("ZERO_MODEL_HTTP", MAKE_HRESULT(SEVERITY_ERROR, FACILITY_HTTP, status));
        } else activation_trace("ZERO_WORKER_RESPONSE_FAILED", E_FAIL);
    }
    if (WaitForSingleObject(process.hProcess, 100) == WAIT_TIMEOUT) TerminateProcess(process.hProcess, 1);
    CloseHandle(process.hProcess);
    if (text.empty()) text = L"模型返回格式无效，原文未修改";
    return {ok, text};
}
std::string read_zero_candidate_mode() {
    const auto path = models_root() / L"zero-candidate.mode";
    const HANDLE file = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE | FILE_SHARE_WRITE,
        nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    if (file == INVALID_HANDLE_VALUE) return {};
    BY_HANDLE_FILE_INFORMATION info{};
    char data[48]{}; DWORD read{};
    const bool ok = GetFileInformationByHandle(file, &info) &&
        !(info.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) &&
        ReadFile(file, data, sizeof(data), &read, nullptr);
    CloseHandle(file);
    if (!ok) return {};
    const std::string_view value(data, read);
    const auto delimiter = value.find(':');
    if (delimiter == value.npos) return {};
    const auto mode = value.substr(0, delimiter), approval = value.substr(delimiter + 1);
    if ((mode != "eco" && mode != "performance" && mode != "pinyin") || approval.size() != 32 ||
        !std::ranges::all_of(approval, [](char c) { return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'); })) return {};
    return std::string(value);
}

std::pair<bool, std::wstring> request_zero_candidate(const std::string_view mode,
    const std::wstring& pinyin, const std::wstring& context, const std::wstring& application,
    const std::wstring& references, std::stop_token stop) {
    if (stop.stop_requested()) return {};
    const auto exe = host_path();
    if (exe.empty()) { activation_trace("ZERO_HELPER_MISSING"); return {}; }
    const auto code = hex(pinyin), nearby = hex(context), host = hex(application);
    if (code.empty() || (!context.empty() && nearby.empty()) || (!application.empty() && host.empty())) return {};
    const auto delimiter = mode.find(':');
    if (delimiter == mode.npos) return {};
    auto payload = "OWOTEXT1\nzero-" + std::string(mode.substr(0, delimiter)) + '\n' + code + '\n' + nearby + '\n' + host + '\n' +
        std::string(mode.substr(delimiter + 1)) + '\n';
    if (!references.empty()) payload += hex(references) + '\n';
    return request(exe, payload, stop, true);
}
} // namespace

#ifdef OWO_TSF_TEST_ACCESS
std::pair<bool, std::wstring> test_selection_worker(const wchar_t* executable) {
    return request(executable, "OWOTEXT1\ntranslate\n68656C6C6F\n\n46697874757265\n", {});
}
#endif

bool ZeroCandidate::eligible() const noexcept {
    const auto& input = owner_.input_buffer_;
    if (!owner_.foreground_focus_ || !owner_.chinese_mode_ || owner_.shortcut_config_.input_scheme != "full_pinyin" ||
        owner_.is_agent_mode() || owner_.selection_assistant_.active() || owner_.agent_trust_pending_ ||
        owner_.unicode_palette_visible_ || owner_.alt_code_preview_active_ || input.size() < 2 || input.size() > 96)
        return false;
    unsigned letters = 0;
    const bool apostrophe_is_separator = owner_.apostrophe_is_separator();
    for (std::size_t i = 0; i < input.size(); ++i) {
        const auto c = input[i];
        if ((c >= L'a' && c <= L'z') || (c >= L'A' && c <= L'Z')) ++letters;
        else if (c != L'\'' || (!apostrophe_is_separator &&
            !std::binary_search(owner_.manual_breaks_.begin(), owner_.manual_breaks_.end(), i))) return false;
    }
    return letters >= 2;
}

bool ZeroCandidate::refresh_mode(bool force) {
    const auto now = GetTickCount64();
    if (!force && now < next_mode_check_) return false;
    next_mode_check_ = now + 600;
    auto mode = read_zero_candidate_mode();
    if (mode_ == mode) return false;
    mode_ = std::move(mode);
    return true;
}

bool ZeroCandidate::read_position(bool first, std::wstring* surroundings) {
    if (!context_ || !owner_.plugin_context_is_current(context_.Get())) return false;
    return edit(context_.Get(), owner_.client_id_, false, [&](TfEditCookie cookie) {
        const auto resolution = resolve_input_context(context_.Get(), cookie);
        if (resolution.safety != InputMethodContextSafety::allowed || !resolution.focus || !resolution.view) return E_ACCESSDENIED;
        auto identity = edit_window_identity(resolution);
        if (!first && identity != window_) return E_ABORT;
        ComPtr<ITfRange> composition;
        if (owner_.inline_preedit_ && owner_.inline_preedit_->owns_range(context_.Get())) {
            if (!owner_.inline_preedit_->range_for_context(context_.Get(), cookie, &composition)) return E_ABORT;
        } else {
            TF_SELECTION selection{}; ULONG count{}; BOOL empty{};
            const auto result = context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count);
            composition.Attach(selection.range);
            if (FAILED(result) || count != 1 || !composition || selection.style.fInterimChar ||
                FAILED(composition->IsEmpty(cookie, &empty)) || !empty) return E_ABORT;
        }
        ComPtr<ITfRange> start;
        if (FAILED(composition->Clone(&start)) || FAILED(start->Collapse(cookie, TF_ANCHOR_START))) return E_FAIL;
        if (first) {
            (void)start->SetGravity(cookie, TF_GRAVITY_BACKWARD, TF_GRAVITY_BACKWARD);
            anchor_ = start;
            window_ = identity;
        } else {
            LONG compare{};
            if (!anchor_ || FAILED(start->CompareStart(cookie, anchor_.Get(), TF_ANCHOR_START, &compare)) || compare) return E_ABORT;
        }
        if (!surroundings) return S_OK;
        const auto side = [&](bool before, LONG limit) {
            ComPtr<ITfRange> range;
            LONG moved{}; ULONG count{}; wchar_t buffer[256]{};
            if (FAILED(composition->Clone(&range)) || FAILED(range->Collapse(cookie, before ? TF_ANCHOR_START : TF_ANCHOR_END))) return std::wstring{};
            const auto shifted = before ? range->ShiftStart(cookie, -limit, &moved, nullptr) : range->ShiftEnd(cookie, limit, &moved, nullptr);
            if (FAILED(shifted) || FAILED(range->GetText(cookie, 0, buffer, static_cast<ULONG>(limit * 2), &count)) ||
                count > static_cast<ULONG>(limit * 2)) return std::wstring{};
            std::wstring result(buffer, count);
            if (result.find(L'\0') != std::wstring::npos) return std::wstring{};
            engine::redact_ranking_numbers(result);
            if (before) append_bounded_language_context(result, {}, static_cast<std::size_t>(limit));
            else if (result.size() > static_cast<std::size_t>(limit)) result.resize(limit);
            if (!result.empty() && result.front() >= 0xdc00 && result.front() <= 0xdfff) result.erase(result.begin());
            if (!result.empty() && result.back() >= 0xd800 && result.back() <= 0xdbff) result.pop_back();
            return result;
        };
        *surroundings = side(true, 128) + L"\n<光标>\n" + side(false, 48);
        return S_OK;
    });
}

void ZeroCandidate::observe() {
    if (!owner_.message_window_) return;
    if (refresh_mode()) invalidate();
    if (!eligible() || mode_.empty()) { invalidate(); return; }
    if (input_ == owner_.input_buffer_ && composition_generation_ == owner_.context_generation_) return;
    cancel();
    ComPtr<ITfDocumentMgr> document;
    if (!owner_.thread_manager_ || FAILED(owner_.thread_manager_->GetFocus(&document)) || !document ||
        FAILED(document->GetTop(&context_)) || !context_ || !read_position(true)) { cancel(); return; }
    wchar_t executable[32768]{};
    const auto length = GetModuleFileNameW(nullptr, executable, ARRAYSIZE(executable));
    if (!length || length >= ARRAYSIZE(executable)) { cancel(); return; }
    application_ = std::filesystem::path(executable).filename().wstring();
    std::transform(application_.begin(), application_.end(), application_.begin(), [](wchar_t c) { return static_cast<wchar_t>(towlower(c)); });
    for (const auto* blocked : {L"windowsterminal.exe", L"cmd.exe", L"powershell.exe", L"pwsh.exe", L"conhost.exe", L"wsl.exe"})
        if (application_ == blocked) { cancel(); return; }
    input_ = owner_.input_buffer_;
    composition_generation_ = owner_.context_generation_;
    deadline_ = (std::max)(next_request_, GetTickCount64() + (mode_.starts_with("performance:") ? 420 : 750));
    if (!SetTimer(owner_.message_window_, kZeroCandidateTimer, 90, nullptr)) cancel();
}

void ZeroCandidate::poll() {
    if (refresh_mode() || !eligible() || input_ != owner_.input_buffer_ || composition_generation_ != owner_.context_generation_ || mode_.empty() ||
        !context_ || !owner_.plugin_context_is_current(context_.Get())) { invalidate(); return; }
    const auto now = GetTickCount64();
    if (!text_.empty()) {
        if (now >= hide_at_ || !read_position(false)) invalidate();
        return;
    }
    if (worker_.joinable()) {
        if (!job_->finished.load(std::memory_order_acquire)) return;
        worker_.join();
        auto result = std::exchange(job_, {});
        if (result->generation == generation_ && result->succeeded && !result->result.empty()) {
            if (refresh_mode(true) || !read_position(false)) {
                activation_trace("ZERO_RESULT_POSITION_CHANGED");
                invalidate(); return;
            }
            text_ = std::move(result->result);
            hide_at_ = now + 20000;
            SetTimer(owner_.message_window_, kZeroCandidateTimer, 250, nullptr);
            owner_.update_candidate_window();
            activation_trace("ZERO_CANDIDATE_PRESENTED");
            return;
        }
    }
    if (submitted_) { KillTimer(owner_.message_window_, kZeroCandidateTimer); return; }
    if (now < deadline_) return;
    if (refresh_mode(true) || !owner_.candidate_window_ || !IsWindowVisible(owner_.candidate_window_)) {
        invalidate(); return;
    }
    const auto performance = mode_.starts_with("performance:");
    std::wstring surroundings;
    if (!read_position(false, performance ? &surroundings : nullptr)) { invalidate(); return; }
    if (mode_.starts_with("eco:")) {
        surroundings = owner_.recent_language_context_;
        append_ranking_language_context(surroundings, owner_.staged_text());
        append_bounded_language_context(surroundings, {}, 16);
    }
    std::wstring references;
    if (!owner_.candidate_request_pending_ && !owner_.candidate_request_failed_ && !owner_.candidate_symbol_preview_ &&
        owner_.candidate_page_ == 0 && owner_.candidate_consumed_.size() == owner_.candidates_.size()) {
        unsigned count = 0;
        for (std::size_t i = 0; i < owner_.candidates_.size() && count < 3; ++i) {
            const auto& candidate = owner_.candidates_[i];
            if (owner_.candidate_consumed_[i] != input_.size() || candidate.empty() || candidate.size() > 48 ||
                candidate.find(L'\n') != std::wstring::npos) continue;
            if (count++) references.push_back(L'\n');
            references.append(candidate);
        }
    }
    submitted_ = true;
    activation_trace("ZERO_CANDIDATE_REQUESTED");
    next_request_ = now + 1500;
    auto job = job_ = std::make_shared<Job>();
    job->generation = generation_;
    try {
        worker_ = std::jthread([job, policy = mode_, pinyin = input_, surroundings = std::move(surroundings),
                application = performance ? application_ : std::wstring{}, references = std::move(references)](std::stop_token stop) {
            try {
                auto [ok, result] = request_zero_candidate(policy, pinyin, surroundings, application, references, stop);
                job->succeeded = ok && !stop.stop_requested() && !result.empty() && result.size() <= 48;
                if (job->succeeded) job->result = std::move(result);
            } catch (...) { /* A failed model does not disturb normal candidates. */ }
            job->finished.store(true, std::memory_order_release);
        });
    } catch (...) { job_.reset(); }
}

void ZeroCandidate::cancel() noexcept {
    if (input_.empty() && !context_) return;
    ++generation_;
    worker_.request_stop();
    if (owner_.message_window_) KillTimer(owner_.message_window_, kZeroCandidateTimer);
    input_.clear(); text_.clear(); application_.clear(); anchor_.Reset(); context_.Reset();
    window_ = {}; submitted_ = false; hide_at_ = 0;
}

void ZeroCandidate::invalidate() {
    const bool was_visible = !text_.empty();
    cancel();
    if (was_visible) owner_.update_candidate_window();
}

std::wstring_view ZeroCandidate::text() const noexcept {
    return !text_.empty() && eligible() && input_ == owner_.input_buffer_ &&
        composition_generation_ == owner_.context_generation_ && GetTickCount64() < hide_at_
        ? std::wstring_view(text_) : std::wstring_view{};
}

bool ZeroCandidate::handles(WPARAM key) const noexcept {
    return key == static_cast<WPARAM>(owner_.shortcut_config_.candidate_selection_numpad ? VK_NUMPAD0 : '0') &&
        GetKeyState(VK_SHIFT) >= 0 && GetKeyState(VK_CONTROL) >= 0 && GetKeyState(VK_MENU) >= 0 &&
        GetKeyState(VK_LWIN) >= 0 && GetKeyState(VK_RWIN) >= 0 && !text().empty() &&
        owner_.candidate_window_ && IsWindowVisible(owner_.candidate_window_);
}

HRESULT ZeroCandidate::choose(ITfContext* expected) {
    if (refresh_mode(true) || text().empty() || (expected && expected != context_.Get()) || !read_position(false)) {
        invalidate(); return S_FALSE;
    }
    auto context = context_;
    auto undo_input = owner_.collect_candidate_undo_input();
    std::optional<CandidateUndoEvidence> undo;
    auto output = owner_.staged_text() + text_;
    const auto status = owner_.commit_text(context.Get(), std::move(output), nullptr, undo_input ? &undo : nullptr);
    if (SUCCEEDED(status)) {
        owner_.clear_composition();
        owner_.remember_candidate_undo(context.Get(), std::move(undo_input), std::move(undo));
    }
    return status;
}

SelectionAssistant::~SelectionAssistant() { cancel(); }
bool SelectionAssistant::test(ITfContext* context, WPARAM key) {
    if (active_) return key == VK_ESCAPE || ((GetKeyState(VK_CONTROL) >= 0 && GetKeyState(VK_MENU) >= 0 &&
        GetKeyState(VK_SHIFT) >= 0 && GetKeyState(VK_LWIN) >= 0 && GetKeyState(VK_RWIN) >= 0) &&
        (key == VK_SPACE || key == 'V' || (key >= '1' && key <= '4') || (key >= VK_NUMPAD1 && key <= VK_NUMPAD4)));
    return trigger(key) && !service_.has_composition() && gate(L"selection-assistant.enabled") &&
        service_.plugin_context_is_current(context) && capture(context, service_.client_id_, false, true).has_value();
}
bool SelectionAssistant::key(ITfContext* context, WPARAM key, LPARAM flags) {
    if (active_) {
        if (!test(context, key)) { service_.clear_composition(); return false; }
        if (key == 'V') return true; // Suppress auto-repeat of the trigger.
        if (key == VK_ESCAPE) { service_.clear_composition(); return true; }
        if ((static_cast<ULONG_PTR>(flags) & (1ULL << 30)) != 0) return true; // Trust and action need distinct presses.
        if (awaiting_trust_ && key == VK_SPACE) return true; // Never accept by default.
        if (key >= VK_NUMPAD1 && key <= VK_NUMPAD4) { choose(key - VK_NUMPAD1); return true; }
        if (key == VK_SPACE || (key >= '1' && key <= '4')) { choose(key == VK_SPACE ? 0 : key - '1'); return true; }
        service_.clear_composition(); return false;
    }
    if (!trigger(key) || service_.has_composition() || !gate(L"selection-assistant.enabled") ||
        !service_.plugin_context_is_current(context)) return false;
    auto selected = capture(context, service_.client_id_, gate(L"context.enabled"));
    if (!selected || !focus()) return false;
    service_.clear_composition(); // Invalidate any late ordinary-candidate update.
    context_ = context; range_ = selected->range; selection_ = std::move(selected->text); surrounding_ = std::move(selected->context); focus_ = focus();
    awaiting_trust_ = selected->needs_trust;
    if (awaiting_trust_) trust_window_ = selected->window;
    wchar_t executable[32768]{};
    GetModuleFileNameW(nullptr, executable, 32768); application_ = std::filesystem::path(executable).filename().wstring();
    active_ = true; busy_ = false; failed_ = false;
    service_.update_candidate_anchor(context);
    SetTimer(service_.message_window_, kSelectionAssistantTimer, 75, nullptr);
    show(); return true;
}
void SelectionAssistant::show() {
    service_.candidates_ = awaiting_trust_ ? std::vector<std::wstring>{L"仅本次信任", L"取消"}
        : busy_ ? std::vector<std::wstring>{L"正在请求模型 · Esc 取消"}
        : failed_ ? std::vector<std::wstring>{L"重试", L"取消"} : std::vector<std::wstring>(std::begin(labels), std::end(labels));
    service_.candidate_consumed_.assign(service_.candidates_.size(), 0);
    service_.candidate_groups_.clear(); service_.candidate_page_ = 0; service_.has_more_candidates_ = false;
    service_.candidate_request_pending_ = false; service_.candidates_expanded_ = false;
    service_.update_candidate_window();
}
std::wstring SelectionAssistant::header() const {
    if (awaiting_trust_) return L"v：无法验证当前窗口。信任后允许读取选区并发送给所选模型？";
    return failed_ ? error_ : busy_ ? L"v模式原文保留中" : L"v模式：选择功能";
}
void SelectionAssistant::confirm_window() {
    if (!service_.plugin_context_is_current(context_.Get()) || focus() != focus_ ||
        !gate(L"selection-assistant.enabled")) { service_.clear_composition(); return; }
    auto selected = capture(context_.Get(), service_.client_id_, gate(L"context.enabled"),
                            false, trust_window_, range_.Get());
    if (!selected) { service_.clear_composition(); return; }
    range_ = selected->range;
    selection_ = std::move(selected->text);
    surrounding_ = std::move(selected->context);
    awaiting_trust_ = false;
    // Trust opens the menu only. The user must separately select an operation.
    show();
}
void SelectionAssistant::choose(std::size_t index) {
    if (!active_ || busy_) return;
    if (awaiting_trust_) {
        if (index == 0) confirm_window();
        else if (index == 1) service_.clear_composition();
        return;
    }
    if (failed_) { if (index == 0) start(operation_); else if (index == 1) service_.clear_composition(); }
    else if (index < 4) start(index);
}
bool SelectionAssistant::unchanged(bool write, const std::wstring& replacement) {
    if (!active_ || !context_ || !range_ || !focus_ || focus() != focus_ ||
        !service_.plugin_context_is_current(context_.Get())) return false;
    DWORD process{}; const auto foreground = edit_foreground_window();
    if (foreground && (GetWindowThreadProcessId(foreground, &process), process != GetCurrentProcessId())) return false;
    return edit(context_.Get(), service_.client_id_, write, [&](TfEditCookie cookie) {
        const auto resolution = resolve_input_context(context_.Get(), cookie);
        if ((trust_window_.focus && trust_window_ != edit_window_identity(resolution)) ||
            !advanced_edit_allowed(resolution, trust_window_)) return E_ACCESSDENIED;
        TF_SELECTION selected{}; ULONG count{};
        if (FAILED(context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selected, &count)) || count != 1 || !selected.range) return E_FAIL;
        ComPtr<ITfRange> current; current.Attach(selected.range); LONG start{}, end{};
        if (FAILED(current->CompareStart(cookie, range_.Get(), TF_ANCHOR_START, &start)) || start != 0 ||
            FAILED(current->CompareEnd(cookie, range_.Get(), TF_ANCHOR_END, &end)) || end != 0) return E_FAIL;
        wchar_t buffer[8193]{}; ULONG read{};
        if (FAILED(current->GetText(cookie, 0, buffer, 8193, &read)) || std::wstring_view(buffer, read) != selection_) return E_FAIL;
        if (!write) return S_OK;
        const auto hr = current->SetText(cookie, 0, replacement.data(), static_cast<LONG>(replacement.size()));
        if (FAILED(hr)) return hr;
        current->Collapse(cookie, TF_ANCHOR_END);
        selected.range = current.Get(); selected.style.ase = TF_AE_NONE; selected.style.fInterimChar = FALSE;
        // Replacement already succeeded: do not retry it if caret placement fails.
        (void)context_->SetSelection(cookie, 1, &selected);
        return S_OK;
    });
}
void SelectionAssistant::start(std::size_t index) {
    if (!unchanged(false) || !gate(L"selection-assistant.enabled")) { service_.clear_composition(); return; }
    const auto exe = host_path();
    if (exe.empty()) { failed_ = true; error_ = L"找不到新版设置中心，请运行 open-owo-settings.cmd 更新"; show(); return; }
    operation_ = index; failed_ = false; busy_ = true;
    const auto payload = std::string("OWOTEXT1\n") + operations[index] + '\n' + hex(selection_) + '\n' + hex(surrounding_) + '\n' + hex(application_) + '\n';
    if (worker_.joinable()) { worker_.request_stop(); worker_.join(); }
    { std::lock_guard lock(mutex_); result_.reset(); }
    try { worker_ = std::jthread([this, exe, payload](std::stop_token stop) {
        try {
            auto [ok, text] = request(exe, payload, stop);
            if (!stop.stop_requested()) { std::lock_guard lock(mutex_); result_ = Result{ok, std::move(text)}; }
        } catch (...) {
            if (!stop.stop_requested()) { std::lock_guard lock(mutex_); result_ = Result{false, L"模型请求失败，原文未修改"}; }
        }
    }); } catch (...) { busy_ = false; failed_ = true; error_ = L"无法启动请求线程，原文未修改"; }
    show();
}
void SelectionAssistant::poll() {
    if (!active_) return;
    if (!unchanged(false)) { service_.clear_composition(); return; }
    std::optional<Result> value;
    { std::lock_guard lock(mutex_); value = std::move(result_); result_.reset(); }
    if (!value) return;
    busy_ = false;
    if (value->ok) {
        if (gate(L"selection-assistant.enabled") && unchanged(true, value->text)) service_.clear_composition();
        else { failed_ = true; error_ = L"选区或权限已变化，未替换原文"; show(); }
    } else { failed_ = true; error_ = std::move(value->text); show(); }
}
void SelectionAssistant::cancel() {
    active_ = false; busy_ = false; failed_ = false;
    if (service_.message_window_) KillTimer(service_.message_window_, kSelectionAssistantTimer);
    if (worker_.joinable()) { worker_.request_stop(); worker_.join(); }
    { std::lock_guard lock(mutex_); result_.reset(); }
    context_.Reset(); range_.Reset(); selection_.clear(); surrounding_.clear(); application_.clear(); error_.clear(); focus_ = nullptr;
    awaiting_trust_ = false; trust_window_ = {};
}
} // namespace owo::tsf

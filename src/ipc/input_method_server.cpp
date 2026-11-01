#include "owo/ipc/input_method_server.h"
#include "owo/ipc/input_method_control.h"
#include "owo/core/input_method_runtime.h"
#include "owo/plugin/input_method_protocol.h"
#include "owo/compatibility/input_context.h"
#include "owo/compatibility/accessible_edit.h"
#include "owo/compatibility/uia_text_verifier.h"

#include <Windows.h>
#include <ole2.h>
#include <UIAutomation.h>
#include <wrl/client.h>
#include <sddl.h>
#include <array>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <future>
#include <iostream>
#include <mutex>
#include <optional>
#include <thread>
#include <vector>

namespace owo::ipc {
namespace {
using Clock = std::chrono::steady_clock;
using namespace std::chrono_literals;
struct Handle {
    HANDLE value{};
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
bool token_info(HANDLE process, std::vector<std::byte>& user, DWORD& session) {
    Handle token;
    if (!OpenProcessToken(process, TOKEN_QUERY, &token.value)) return false;
    DWORD size{};
    GetTokenInformation(token.value, TokenUser, nullptr, 0, &size);
    if (!size) return false;
    user.resize(size);
    return GetTokenInformation(token.value, TokenUser, user.data(), size, &size) &&
        GetTokenInformation(token.value, TokenSessionId, &session, sizeof(session), &size);
}
struct AutomationConnection {
    HRESULT initialized{CoInitializeEx(nullptr,COINIT_MULTITHREADED)};
    HRESULT result{initialized};
    Microsoft::WRL::ComPtr<IUIAutomation2> automation;
    Microsoft::WRL::ComPtr<IUIAutomationCacheRequest> cache;
    AutomationConnection(){
        if(FAILED(result))return;
        result=CoCreateInstance(__uuidof(CUIAutomation8),nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&automation));
        if(SUCCEEDED(result))result=automation->put_ConnectionTimeout(100);
        if(SUCCEEDED(result))result=automation->put_TransactionTimeout(100);
        if(SUCCEEDED(result))result=automation->CreateCacheRequest(&cache);
        for(auto property:{UIA_ProcessIdPropertyId,UIA_IsPasswordPropertyId,UIA_HasKeyboardFocusPropertyId,UIA_ControlTypePropertyId})
            if(SUCCEEDED(result))result=cache->AddProperty(property);
    }
    ~AutomationConnection(){cache.Reset();automation.Reset();if(SUCCEEDED(initialized))CoUninitialize();}
};
AutomationConnection& automation_connection(){static thread_local AutomationConnection connection;return connection;}
InputMethodFocus automation_focus(std::uint32_t pid, HWND native_window) {
    const auto started=Clock::now();
    const auto denied = [pid,started](const char* stage, HRESULT hr=E_ACCESSDENIED) -> InputMethodFocus {
        static thread_local ULONGLONG last{};
        const auto now=GetTickCount64();
        if(now-last>=500){last=now;std::clog<<"INPUT_CONTEXT_UIA_UNKNOWN client_pid="<<pid<<" stage="<<stage
            <<" hr="<<static_cast<long>(hr)<<" duration_us="
            <<std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-started).count()<<'\n';}
        return {};
    };
    // Keep the MTA and UIA transport alive for the IO thread, not for one
    // 40ms focus tick. Only the client/recipe is reused; every element and all
    // security metadata are fetched afresh, including after application restart.
    auto& connection=automation_connection();
    if(FAILED(connection.result))return denied("automation",connection.result);
    const auto& automation=connection.automation;
    const auto& cache=connection.cache;
    Microsoft::WRL::ComPtr<IUIAutomationElement> element;
    const auto focused_result=automation->GetFocusedElementBuildCache(cache.Get(), &element);
    if (FAILED(focused_result) || !element) return denied("focused_element",focused_result);
    int process{}; CONTROLTYPEID type{}; BOOL password = TRUE, focused = FALSE;
    if (FAILED(element->get_CachedProcessId(&process)) || process != static_cast<int>(pid))return denied("process");
    if (FAILED(element->get_CachedIsPassword(&password)) || password)return denied("password");
    if (FAILED(element->get_CachedHasKeyboardFocus(&focused)) || !focused)return denied("focus");
    const auto type_result=element->get_CachedControlType(&type);
    bool accessible_verified=false;
    bool active_document=false;
    LONG accessible_identity{};
    if (FAILED(type_result) || type != UIA_EditControlTypeId) {
        DWORD owner{}; const auto thread=GetWindowThreadProcessId(native_window,&owner);
        const auto legacy=compatibility::inspect_accessible_edit(native_window,pid,thread);
        static thread_local ULONGLONG last_accessible_report{};
        if(GetTickCount64()-last_accessible_report>=1000){
        last_accessible_report=GetTickCount64();
        std::clog<<"INPUT_CONTEXT_ACCESSIBLE client_pid="<<pid<<" type="<<type<<" role="<<legacy.role<<" state="<<legacy.state
                 <<" hr="<<static_cast<long>(legacy.result)<<" decision="<<static_cast<int>(legacy.resolution.decision)
                 <<" ia2_hr="<<static_cast<long>(legacy.ia2_result)<<" ia2_states="<<legacy.ia2_states<<'\n';
        }
        if(FAILED(type_result))return denied("control_type",type_result);
        if(legacy.resolution.decision!=compatibility::Decision::allowed){
            if(legacy.resolution.decision==compatibility::Decision::denied)return denied("accessible_restriction");
            // Resolve the native text view's active caret as well as the
            // currently focused automation wrapper. Both identities are checked.
            Microsoft::WRL::ComPtr<IUIAutomationElement> native;
            if(SUCCEEDED(automation->ElementFromHandle(native_window,&native))&&native &&
               compatibility::inspect_uia_text_caret(native.Get(),pid).access.decision==compatibility::Decision::allowed)
                element=std::move(native);
            const auto caret=compatibility::inspect_uia_text_caret(element.Get(),pid);
            if(caret.access.decision!=compatibility::Decision::allowed)return denied("text_caret",caret.result);
            active_document=true;
        }
        accessible_verified=true;accessible_identity=legacy.ia2_id;
    }
    if(!accessible_verified){
    BOOL enabled = FALSE;
    VARIANT read_only; VariantInit(&read_only);
    auto readonly_result = element->GetCurrentPropertyValueEx(UIA_ValueIsReadOnlyPropertyId,TRUE,&read_only);
    if(SUCCEEDED(readonly_result) && read_only.vt!=VT_BOOL){
        VariantClear(&read_only);
        // Windowless rich editors may expose TextPattern but not ValuePattern.
        // Query a boolean attribute only; never retrieve document text.
        Microsoft::WRL::ComPtr<IUIAutomationTextPattern> pattern;
        Microsoft::WRL::ComPtr<IUIAutomationTextRange> range;
        readonly_result=element->GetCurrentPatternAs(UIA_TextPatternId,IID_PPV_ARGS(&pattern));
        if(SUCCEEDED(readonly_result) && pattern)readonly_result=pattern->get_DocumentRange(&range);
        if(SUCCEEDED(readonly_result) && range)readonly_result=range->GetAttributeValue(UIA_IsReadOnlyAttributeId,&read_only);
    }
    const bool properties_known = SUCCEEDED(readonly_result) && read_only.vt == VT_BOOL &&
        SUCCEEDED(element->get_CurrentIsEnabled(&enabled));
    const auto proof = compatibility::evaluate_edit({true, focused != FALSE, true, properties_known,
        enabled != FALSE, password != FALSE, read_only.vt == VT_BOOL && read_only.boolVal != VARIANT_FALSE});
    VariantClear(&read_only);
    if (proof.decision != compatibility::Decision::allowed) return denied("edit_properties",readonly_result);
    }
    SAFEARRAY* runtime{};
    if (FAILED(element->GetRuntimeId(&runtime)) || !runtime) return denied("runtime_id");
    struct Runtime { SAFEARRAY* value; ~Runtime() { SafeArrayDestroy(value); } } release{runtime};
    LONG first{}, last{};
    VARTYPE runtime_type{};
    if (FAILED(SafeArrayGetVartype(runtime, &runtime_type)) || runtime_type != VT_I4 ||
        SafeArrayGetElemsize(runtime) != sizeof(LONG) || SafeArrayGetDim(runtime) != 1 ||
        FAILED(SafeArrayGetLBound(runtime, 1, &first)) ||
        FAILED(SafeArrayGetUBound(runtime, 1, &last))) return denied("runtime_array");
    const auto count = static_cast<std::int64_t>(last) - first + 1;
    if (count <= 0 || count > 64) return {};
    std::uint64_t identity = 14695981039346656037ULL;
    for (std::int64_t offset = 0; offset < count; ++offset) {
        LONG index = static_cast<LONG>(static_cast<std::int64_t>(first) + offset);
        LONG part{}; if (FAILED(SafeArrayGetElement(runtime, &index, &part))) return {};
        identity = (identity ^ static_cast<std::uint32_t>(part)) * 1099511628211ULL;
    }
    if(accessible_verified)identity=(identity^static_cast<std::uint32_t>(accessible_identity))*1099511628211ULL;
    GUITHREADINFO current{sizeof(current)}; DWORD actual{};
    const auto thread = GetWindowThreadProcessId(GetForegroundWindow(), &actual);
    if (actual != pid || !GetGUIThreadInfo(thread, &current) || current.hwndFocus != native_window) return {};
    if(active_document){
        if(compatibility::inspect_uia_text_caret(element.Get(),pid).access.decision!=compatibility::Decision::allowed)
            return denied("changed_caret");
    }else if(FAILED(element->get_CurrentHasKeyboardFocus(&focused)) || !focused)return denied("changed_focus");
    return {identity ? identity : 1, false};
}
InputMethodFocus native_focus(std::uint32_t pid) {
    const auto foreground = GetForegroundWindow();
    DWORD actual{};
    const auto thread = GetWindowThreadProcessId(foreground, &actual);
    if (!thread || actual != pid) return {};
    GUITHREADINFO info{sizeof(info)};
    if (!GetGUIThreadInfo(thread, &info) || !info.hwndFocus) return {};
    const auto focused_thread=GetWindowThreadProcessId(info.hwndFocus, &actual);
    if (actual != pid) return {};
    std::array<wchar_t, 128> name{};
    if (!RealGetWindowClassW(info.hwndFocus, name.data(), static_cast<UINT>(name.size()))) return {};
    const std::wstring_view cls(name.data());
    const bool known = _wcsicmp(name.data(), L"Edit") == 0 ||
        (cls.size() >= 8 && _wcsnicmp(name.data(), L"RichEdit", 8) == 0);
    if (!known) {
        // A Chromium rich editor can expose only a Group through UIA while its
        // independently focused IA2 object explicitly advertises EDITABLE. Do
        // not make this proof depend on a successful global UIA focus query.
        auto& connection=automation_connection();
        if(FAILED(connection.initialized))return {};
        const auto accessible=compatibility::inspect_accessible_edit(info.hwndFocus,pid,focused_thread);
        if(accessible.resolution.decision==compatibility::Decision::denied)return {};
        if(accessible.ia2_verified){
            DWORD owner{};GUITHREADINFO current{sizeof(current)};
            const auto current_thread=GetWindowThreadProcessId(GetForegroundWindow(),&owner);
            if(owner!=pid || !GetGUIThreadInfo(current_thread,&current) || current.hwndFocus!=info.hwndFocus)return {};
            auto identity=(static_cast<std::uint64_t>(reinterpret_cast<std::uintptr_t>(info.hwndFocus))^
                (static_cast<std::uint64_t>(pid)<<32)^static_cast<std::uint32_t>(accessible.ia2_id))*1099511628211ULL;
            static thread_local ULONGLONG last{};
            if(GetTickCount64()-last>=1000){last=GetTickCount64();std::clog<<"INPUT_CONTEXT_IA2_VERIFIED client_pid="<<pid
                <<" states="<<accessible.ia2_states<<" object_id="<<accessible.ia2_id<<'\n';}
            return {identity?identity:1,false};
        }
        // Modern HWND hosts do not identify the focused child input. Require UIA
        // evidence for a focused, non-password Edit owned by the pipe client.
        const auto verified = automation_focus(pid, info.hwndFocus);
        if (verified.identity) return verified;
    }
    const auto native = compatibility::inspect_native_edit(foreground, info.hwndFocus, pid, focused_thread);
    if (!known || native.decision != compatibility::Decision::allowed) {
        // Class metadata only; never inspect the control's text or password value.
        static thread_local ULONGLONG last_report{};
        const auto now = GetTickCount64();
        if (now - last_report >= 1000) {
            last_report = now;
            std::clog << "INPUT_METHOD_FOCUS_DENIED client_pid=" << pid
                      << " known_edit=" << known << " class=";
            for (const auto c : cls) std::clog << (c >= 32 && c < 127 ? static_cast<char>(c) : '?');
            std::clog << '\n';
        }
        return {};
    }
    return {reinterpret_cast<std::uintptr_t>(info.hwndFocus), false};
}
InputMethodControlResponse rejected(std::uint64_t id, const char* why) {
    return {id, 0, InputMethodControlStatus::rejected, {}, why};
}
}

struct InputMethodPipeServer::Impl {
    core::InputMethodSessionRouter router;
    std::filesystem::path root;
    FocusResolver focus;
    HANDLE pipe{INVALID_HANDLE_VALUE};
    std::atomic<bool> stopping{};
    std::thread io_thread;
    struct Job {
        std::uint64_t owner{};
        InputMethodControlRequest request;
        std::shared_ptr<std::atomic<bool>> cancelled;
        std::promise<InputMethodControlResponse> promise;
    };
    std::mutex job_mutex;
    std::condition_variable job_cv;
    std::optional<Job> job;
    std::thread control_thread;
    std::uint64_t next_owner{};
    bool disconnect_requested{}; // IO thread only: release the pipe for a new foreground client
    explicit Impl(std::filesystem::path store, const std::wstring& name, FocusResolver resolver)
        : router(store), root(std::move(store)), focus(resolver ? std::move(resolver) : native_focus) {
        std::vector<std::byte> user; DWORD session{};
        if (!token_info(GetCurrentProcess(), user, session)) return;
        LPWSTR sid{};
        if (!ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid, &sid)) return;
        const std::wstring sddl = L"D:P(A;;GA;;;" + std::wstring(sid) + L")S:(ML;;NW;;;LW)";
        LocalFree(sid);
        PSECURITY_DESCRIPTOR descriptor{};
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1,
            &descriptor, nullptr)) return;
        SECURITY_ATTRIBUTES attributes{sizeof(attributes), descriptor, FALSE};
        pipe = CreateNamedPipeW(name.c_str(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED |
            FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT |
            PIPE_REJECT_REMOTE_CLIENTS, 1, 64 * 1024, 64 * 1024, 0, &attributes);
        LocalFree(descriptor);
        if (pipe == INVALID_HANDLE_VALUE) return;
        try {
            control_thread = std::thread([this] { controls(); });
            io_thread = std::thread([this] {
                try { serve(); }
                catch (...) { stopping = true; job_cv.notify_all(); DisconnectNamedPipe(pipe); }
            });
        } catch (...) {
            stopping = true; job_cv.notify_all();
            if (control_thread.joinable()) control_thread.join();
            CloseHandle(pipe); pipe = INVALID_HANDLE_VALUE;
        }
    }
    ~Impl() {
        stopping = true; job_cv.notify_all();
        if (io_thread.joinable()) io_thread.join();
        if (control_thread.joinable()) control_thread.join();
        if (pipe != INVALID_HANDLE_VALUE) CloseHandle(pipe);
    }
    void controls() {
        for (;;) {
            std::optional<Job> current;
            {
                std::unique_lock lock(job_mutex);
                job_cv.wait(lock, [&] { return stopping || job.has_value(); });
                if (stopping) return;
                current = std::move(job); job.reset();
            }
            auto& j = *current;
            auto result = rejected(j.request.request_id, "Control request cancelled");
            try {
                if (!j.cancelled->load()) {
                    if (j.request.action == InputMethodControlAction::discover) {
                        auto found = core::discover_plugin_input_methods(root, j.request.plugin_id);
                        if (found.validation.ok) result = {j.request.request_id, j.request.request_id,
                            InputMethodControlStatus::ready,
                            plugin::encode_input_method_descriptors(j.request.plugin_id, found.descriptors), {}};
                        else result = rejected(j.request.request_id, "Scheme discovery unavailable");
                    } else {
                        auto opened = router.open(j.owner, j.request.session_id,
                            j.request.plugin_id, j.request.scheme_id);
                        const auto d = router.descriptor(j.owner, j.request.session_id);
                        if (opened.ok && d) result = {j.request.request_id, j.request.request_id,
                            InputMethodControlStatus::ready,
                            plugin::encode_input_method_descriptors(j.request.plugin_id, {*d}), {}};
                        else result = rejected(j.request.request_id, "Scheme session unavailable");
                    }
                }
            } catch (...) { result = rejected(j.request.request_id, "Control request failed"); }
            if (j.cancelled->load() || stopping) {
                if (j.request.action == InputMethodControlAction::open)
                    (void)router.close(j.owner, j.request.session_id);
                result = rejected(j.request.request_id, "Control request cancelled");
            }
            j.promise.set_value(std::move(result));
        }
    }
    // Wait in short intervals so shutdown and focus invalidation also work while idle.
    bool finish(OVERLAPPED& op, DWORD& count, Clock::time_point deadline,
                const std::function<void()>& tick) {
        for (;;) {
            const auto wait = WaitForSingleObject(op.hEvent, 40);
            if (tick) tick();
            if (stopping || disconnect_requested || Clock::now() >= deadline || wait == WAIT_FAILED) {
                CancelIoEx(pipe, &op); GetOverlappedResult(pipe, &op, &count, TRUE); return false;
            }
            if (wait == WAIT_OBJECT_0) return GetOverlappedResult(pipe, &op, &count, FALSE) != FALSE;
        }
    }
    bool transfer(char* data, DWORD size, bool write, Clock::time_point deadline,
                  const std::function<void()>& tick) {
        DWORD offset{};
        while (offset < size && !stopping && !disconnect_requested) {
            Handle event{CreateEventW(nullptr, TRUE, FALSE, nullptr)};
            if (!event.value) return false;
            OVERLAPPED op{}; op.hEvent = event.value; DWORD count{};
            const auto ok = write ? WriteFile(pipe, data + offset, size - offset, &count, &op)
                                  : ReadFile(pipe, data + offset, size - offset, &count, &op);
            if (!ok && (GetLastError() != ERROR_IO_PENDING || !finish(op, count, deadline, tick))) return false;
            if (!count) return false;
            offset += count;
        }
        return offset == size;
    }
    bool read(std::string& payload, const std::function<void()>& tick) {
        std::array<char, 4> bytes{};
        if (!transfer(bytes.data(), 1, false, Clock::time_point::max(), tick) ||
            !transfer(bytes.data() + 1, 3, false, Clock::now() + 2s, tick)) return false;
        std::uint32_t size{};
        for (unsigned n = 0; n < 4; ++n) size |= static_cast<std::uint32_t>(static_cast<unsigned char>(bytes[n])) << (8 * n);
        if (!size || size > kMaximumInputMethodControlWire) return false;
        payload.resize(size);
        return transfer(payload.data(), size, false, Clock::now() + 2s, tick);
    }
    bool write(const InputMethodControlResponse& response, const std::function<void()>& tick) {
        if (tick) tick();
        if (disconnect_requested) return false;
        auto data = encode_input_method_control_response(response);
        if (data.empty()) return false;
        std::array<char, 4> bytes{};
        for (unsigned n = 0; n < 4; ++n) bytes[n] = static_cast<char>(data.size() >> (8 * n));
        const auto deadline = Clock::now() + 2s;
        return transfer(bytes.data(), 4, true, deadline, tick) &&
            transfer(data.data(), static_cast<DWORD>(data.size()), true, deadline, tick);
    }
    void connection(DWORD pid, HANDLE process) {
        const auto owner = ++next_owner;
        std::uint64_t last_id{}, ticket{}, bound_focus{};
        std::string session;
        std::optional<engine::InputMethodDescriptor> descriptor;
        std::future<InputMethodControlResponse> control;
        std::future<engine::InputMethodResponse> candidate;
        engine::InputMethodRequest input;
        std::shared_ptr<std::atomic<bool>> cancelled;
        const auto clear = [&] {
            if (cancelled) *cancelled = true;
            if (!session.empty()) (void)router.close(owner, session);
            session.clear(); descriptor.reset(); bound_focus = 0; ticket = 0;
            candidate = {}; // worker promise futures never block on destruction
        };
        const auto snapshot = [&]() -> InputMethodFocus {
            try { return focus(pid); } catch (...) { return {}; }
        };
        const auto tick = [&] {
            if (!session.empty()) {
                const auto f = snapshot();
                if (!f.identity || f.sensitive || f.identity != bound_focus ||
                    WaitForSingleObject(process, 0) != WAIT_TIMEOUT) {
                    std::clog<<"INPUT_METHOD_FOCUS_RETIRED client_pid="<<pid<<" session="<<session
                        <<" bound="<<bound_focus<<" current="<<f.identity<<" sensitive="<<f.sensitive<<'\n';
                    clear(); disconnect_requested = true;
                }
            }
        };
        try {
            while (!stopping) {
                std::string bytes;
                if (!read(bytes, tick)) break;
                tick();
                InputMethodControlRequest r;
                if (!decode_input_method_control_request(bytes, r)) break;
                auto result = rejected(r.request_id, "Request rejected");
                if (r.request_id <= last_id) result = rejected(r.request_id, "Request ID not increasing");
                else {
                    last_id = r.request_id;
                    if (r.sensitive_input) {
                        clear(); result = rejected(r.request_id, "Sensitive input blocked");
                    } else if (r.action == InputMethodControlAction::close) {
                        if (session == r.session_id) {
                            clear(); result = {r.request_id, 0, InputMethodControlStatus::ready, {}, {}};
                        }
                    } else if (r.action == InputMethodControlAction::discover || r.action == InputMethodControlAction::open) {
                        const auto f = snapshot();
                        // Do not replace an outstanding control future; control queue stays bounded.
                        if (control.valid() || (r.action == InputMethodControlAction::open && (!f.identity || f.sensitive)))
                            result = rejected(r.request_id, "Control busy or unsafe focus");
                        else {
                            clear();
                            if (r.action == InputMethodControlAction::open) { session = r.session_id; bound_focus = f.identity;
                                std::clog<<"INPUT_METHOD_OPEN client_pid="<<pid<<" request="<<r.request_id
                                    <<" session="<<session<<" focus="<<bound_focus<<'\n';}
                            cancelled = std::make_shared<std::atomic<bool>>(false);
                            Job next{owner, r, cancelled, {}};
                            control = next.promise.get_future();
                            { std::lock_guard lock(job_mutex); job = std::move(next); }
                            job_cv.notify_one(); ticket = r.request_id;
                            result = {r.request_id, ticket, InputMethodControlStatus::pending, {}, {}};
                        }
                    } else if (r.action == InputMethodControlAction::submit) {
                        engine::InputMethodRequest proposed;
                        if (descriptor && plugin::decode_input_method_request(*descriptor, r.payload, proposed).ok &&
                            proposed.request_id == r.request_id && proposed.session_id == session && proposed.scheme_id == descriptor->id) {
                            input = std::move(proposed);
                            candidate = router.submit(owner, input, false); ticket = r.request_id;
                            result = {r.request_id, ticket, InputMethodControlStatus::pending, {}, {}};
                        }
                    } else if (r.action == InputMethodControlAction::poll && ticket && r.ticket == ticket) {
                        result = {r.request_id, ticket, InputMethodControlStatus::pending, {}, {}};
                        if (control.valid() && control.wait_for(0ms) == std::future_status::ready) {
                            result = control.get(); result.request_id = r.request_id;
                            if (result.status == InputMethodControlStatus::ready && !session.empty()) {
                                descriptor = router.descriptor(owner, session);
                                if (!descriptor) result = rejected(r.request_id, "Session closed");
                            } else if (result.status != InputMethodControlStatus::ready) clear();
                            ticket = 0;
                        } else if (candidate.valid() && candidate.wait_for(0ms) == std::future_status::ready && descriptor) {
                            auto response = candidate.get();
                            tick();
                            if (descriptor && router.descriptor(owner, session)) result = {r.request_id, ticket, InputMethodControlStatus::ready,
                                plugin::encode_input_method_response(*descriptor, input, response), {}};
                            else { clear(); result = rejected(r.request_id, "Session closed or authorization changed"); }
                            ticket = 0;
                        }
                    }
                }
                tick();
                // Never publish a completed open/input result after focus invalidation.
                if (result.status == InputMethodControlStatus::ready && !result.payload.empty() &&
                    cancelled && cancelled->load()) result = rejected(r.request_id, "Session closed");
                if (result.status == InputMethodControlStatus::rejected)
                    std::clog << "INPUT_METHOD_REJECT pid=" << GetCurrentProcessId()
                              << " request_id=" << result.request_id << " reason=" << result.diagnostic << '\n';
                if(r.action==InputMethodControlAction::submit || result.status==InputMethodControlStatus::ready)
                    std::clog<<"INPUT_METHOD_RESULT client_pid="<<pid<<" request="<<r.request_id
                        <<" session="<<session<<" action="<<static_cast<int>(r.action)
                        <<" status="<<static_cast<int>(result.status)<<" bytes="<<result.payload.size()<<'\n';
                if (!write(result, tick)) break;
                // A cancelled control must finish before another discovery/open is accepted.
                if (!ticket && control.valid() && control.wait_for(0ms) == std::future_status::ready)
                    (void)control.get();
            }
        } catch (...) {} // fail closed, no input/exception text logged
        clear();
    }
    void serve() {
        std::vector<std::byte> host_user; DWORD host_session{};
        if (!token_info(GetCurrentProcess(), host_user, host_session)) return;
        while (!stopping) {
            disconnect_requested = false;
            Handle event{CreateEventW(nullptr, TRUE, FALSE, nullptr)};
            if (!event.value) return;
            OVERLAPPED op{}; op.hEvent = event.value; DWORD count{};
            bool connected = ConnectNamedPipe(pipe, &op) != FALSE;
            if (!connected) {
                const auto error = GetLastError();
                connected = error == ERROR_PIPE_CONNECTED ||
                    (error == ERROR_IO_PENDING && finish(op, count, Clock::time_point::max(), {}));
            }
            if (!connected) { DisconnectNamedPipe(pipe); continue; }
            ULONG pid{};
            if (GetNamedPipeClientProcessId(pipe, &pid)) {
                Handle process{OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, FALSE, pid)};
                std::vector<std::byte> user; DWORD session{};
                if (process.value && token_info(process.value, user, session) && session == host_session &&
                    EqualSid(reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid,
                             reinterpret_cast<TOKEN_USER*>(host_user.data())->User.Sid))
                    connection(pid, process.value);
            }
            DisconnectNamedPipe(pipe);
        }
    }
};
InputMethodPipeServer::InputMethodPipeServer(std::filesystem::path root, std::wstring name, FocusResolver focus)
    : impl_(std::make_unique<Impl>(std::move(root), name, std::move(focus))) {}
InputMethodPipeServer::~InputMethodPipeServer() = default;
bool InputMethodPipeServer::started() const noexcept { return impl_->pipe != INVALID_HANDLE_VALUE; }
} // namespace owo::ipc

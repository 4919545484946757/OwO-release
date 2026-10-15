#include "owo/ipc/input_method_server.h"
#include "owo/ipc/input_method_control.h"
#include "owo/core/input_method_runtime.h"
#include "owo/plugin/input_method_protocol.h"

#include <Windows.h>
#include <sddl.h>
#include <array>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <future>
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
InputMethodFocus native_focus(std::uint32_t pid) {
    const auto foreground = GetForegroundWindow();
    DWORD actual{};
    const auto thread = GetWindowThreadProcessId(foreground, &actual);
    if (!thread || actual != pid) return {};
    GUITHREADINFO info{sizeof(info)};
    if (!GetGUIThreadInfo(thread, &info) || !info.hwndFocus) return {};
    GetWindowThreadProcessId(info.hwndFocus, &actual);
    if (actual != pid) return {};
    std::array<wchar_t, 128> name{};
    if (!GetClassNameW(info.hwndFocus, name.data(), static_cast<int>(name.size()))) return {};
    const std::wstring_view cls(name.data());
    const bool known = _wcsicmp(name.data(), L"Edit") == 0 ||
        (cls.size() >= 8 && _wcsnicmp(name.data(), L"RichEdit", 8) == 0);
    if (!known || (GetWindowLongPtrW(info.hwndFocus, GWL_STYLE) & ES_PASSWORD)) return {};
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
                            if (r.action == InputMethodControlAction::open) { session = r.session_id; bound_focus = f.identity; }
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

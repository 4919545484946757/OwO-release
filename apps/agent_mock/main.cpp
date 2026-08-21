#include "owo/agent/agent_protocol.h"
#include "owo/agent/agent_pipe.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <sddl.h>

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <iostream>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace {

std::atomic_bool running{true};

BOOL WINAPI console_handler(const DWORD event) {
    if (event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT || event == CTRL_CLOSE_EVENT) {
        running.store(false, std::memory_order_release);
        return TRUE;
    }
    return FALSE;
}

class CurrentUserSecurity final {
public:
    CurrentUserSecurity() {
        HANDLE token = nullptr;
        if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return;
        DWORD size = 0;
        GetTokenInformation(token, TokenUser, nullptr, 0, &size);
        std::vector<std::byte> storage(size);
        if (size == 0 || !GetTokenInformation(token, TokenUser, storage.data(), size, &size)) {
            CloseHandle(token);
            return;
        }
        const auto* user = reinterpret_cast<const TOKEN_USER*>(storage.data());
        LPWSTR sid = nullptr;
        if (!ConvertSidToStringSidW(user->User.Sid, &sid)) {
            CloseHandle(token);
            return;
        }
        CloseHandle(token);
        const std::wstring sddl = L"D:P(A;;GA;;;" + std::wstring(sid) +
            L")(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;LW)";
        LocalFree(sid);
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.c_str(), SDDL_REVISION_1, &descriptor_, nullptr)) return;
        attributes_ = {sizeof(attributes_), descriptor_, FALSE};
    }
    ~CurrentUserSecurity() { if (descriptor_ != nullptr) LocalFree(descriptor_); }
    CurrentUserSecurity(const CurrentUserSecurity&) = delete;
    CurrentUserSecurity& operator=(const CurrentUserSecurity&) = delete;
    explicit operator bool() const noexcept { return descriptor_ != nullptr; }
    SECURITY_ATTRIBUTES* get() noexcept { return descriptor_ ? &attributes_ : nullptr; }

private:
    PSECURITY_DESCRIPTOR descriptor_{};
    SECURITY_ATTRIBUTES attributes_{};
};

bool transfer_exact(const HANDLE pipe, void* data, const DWORD size, const bool write) {
    DWORD transferred = 0;
    const BOOL ok = write ? WriteFile(pipe, data, size, &transferred, nullptr)
                          : ReadFile(pipe, data, size, &transferred, nullptr);
    return ok != FALSE && transferred == size;
}

bool read_frame(const HANDLE pipe, std::string& output) {
    std::array<unsigned char, 4> header{};
    if (!transfer_exact(pipe, header.data(), static_cast<DWORD>(header.size()), false))
        return false;
    const auto size = static_cast<std::uint32_t>(header[0]) |
        (static_cast<std::uint32_t>(header[1]) << 8) |
        (static_cast<std::uint32_t>(header[2]) << 16) |
        (static_cast<std::uint32_t>(header[3]) << 24);
    if (size == 0 || size > owo::agent::kMaximumAgentPayloadBytes) return false;
    output.resize(size);
    return transfer_exact(pipe, output.data(), size, false);
}

bool write_frame(const HANDLE pipe, const std::string_view payload) {
    if (payload.empty() || payload.size() > owo::agent::kMaximumAgentPayloadBytes)
        return false;
    std::array<unsigned char, 4> header{
        static_cast<unsigned char>(payload.size()),
        static_cast<unsigned char>(payload.size() >> 8),
        static_cast<unsigned char>(payload.size() >> 16),
        static_cast<unsigned char>(payload.size() >> 24)};
    return transfer_exact(pipe, header.data(), static_cast<DWORD>(header.size()), true) &&
        transfer_exact(pipe, const_cast<char*>(payload.data()),
                       static_cast<DWORD>(payload.size()), true);
}

struct Session {
    owo::agent::Request last_request;
    bool executing{};
};

owo::agent::Response page_response(const owo::agent::Request& request) {
    owo::agent::Response response;
    response.session_id = request.session_id;
    response.message = "模拟 Agent 已收到：" + request.user_input;
    response.status = owo::agent::Status::agent_mode;
    response.page = request.page;
    if (request.page == 0) {
        response.commands = {
            {"echo", "回显本次请求", false},
            {"inspect-context", "显示收到的应用上下文", false},
            {"simulate-execution", "模拟执行中状态", false},
            {"delete-demo", "模拟删除文件（高风险测试）", true},
        };
        response.has_more = true;
    } else {
        response.commands = {
            {"finish", "模拟完成任务", false},
            {"error-demo", "模拟 Agent 错误", false},
        };
    }
    return response;
}

owo::agent::Response process(const owo::agent::Request& request,
                             std::unordered_map<std::string, Session>& sessions) {
    if (request.action == owo::agent::Action::cancel) {
        sessions.erase(request.session_id);
        return {request.session_id, "模拟会话已取消", {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.action == owo::agent::Action::submit) {
        sessions[request.session_id] = {request, false};
        return page_response(request);
    }
    const auto found = sessions.find(request.session_id);
    if (found == sessions.end())
        return {request.session_id, "模拟 Agent 找不到该会话", {}, {},
                owo::agent::Status::error, 0, false, "session_not_found"};
    if (request.action == owo::agent::Action::page) {
        auto paged = found->second.last_request;
        paged.page = request.page;
        return page_response(paged);
    }
    if (request.action == owo::agent::Action::poll) {
        if (found->second.executing) {
            found->second.executing = false;
            return {request.session_id, "模拟操作已经完成", {}, {},
                    owo::agent::Status::completed, request.page, false, {}};
        }
        return {request.session_id, "模拟 Agent 当前没有执行中的任务", {}, {},
                owo::agent::Status::agent_mode, request.page, false, {}};
    }
    if (request.command_id == "echo") {
        return {request.session_id,
                "回显：" + found->second.last_request.user_input, {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "inspect-context") {
        const auto& source = found->second.last_request;
        const std::string message = "应用=" + source.application.executable +
            "，窗口类=" + source.application.window_class +
            "，局部上下文=" + source.session_context;
        return {request.session_id, message, {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "simulate-execution") {
        found->second.executing = true;
        return {request.session_id, "模拟 Agent 正在处理", {}, "模拟等待 1 次轮询",
                owo::agent::Status::executing, 0, false, {}};
    }
    if (request.command_id == "finish") {
        return {request.session_id, "模拟任务完成", {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "error-demo") {
        return {request.session_id, "这是模拟错误，不会执行任何操作", {}, {},
                owo::agent::Status::error, 0, false, "mock_error"};
    }
    return {request.session_id, "模拟 Agent 收到未知指令 ID", {}, {},
            owo::agent::Status::error, 0, false, "unknown_command"};
}

}  // namespace

int wmain(const int argc, wchar_t** argv) {
    std::wstring pipe_name = LR"(\\.\pipe\OwO.Agent.External.v1)";
    if (argc == 3 && std::wstring_view(argv[1]) == L"--pipe") pipe_name = argv[2];
    else if (argc != 1) {
        std::cerr << "usage: owo_agent_mock [--pipe \\\\.\\pipe\\Name]\n";
        return 1;
    }
    if (!owo::agent::valid_agent_pipe_name(pipe_name)) {
        std::cerr << "invalid local Agent pipe name\n";
        return 2;
    }
    SetConsoleOutputCP(CP_UTF8);
    SetConsoleCtrlHandler(console_handler, TRUE);
    CurrentUserSecurity security;
    if (!security) {
        std::cerr << "cannot create current-user pipe security descriptor\n";
        return 3;
    }
    std::unordered_map<std::string, Session> sessions;
    std::cout << "OwO 模拟第三方 Agent 已启动。\n"
              << "监听本机管道；按 Ctrl+C 停止。\n\n";
    while (running.load(std::memory_order_acquire)) {
        const HANDLE pipe = CreateNamedPipeW(
            pipe_name.c_str(), PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1,
            owo::agent::kMaximumAgentPayloadBytes + 4,
            owo::agent::kMaximumAgentPayloadBytes + 4, 1000, security.get());
        if (pipe == INVALID_HANDLE_VALUE) return 4;
        const bool connected = ConnectNamedPipe(pipe, nullptr) != FALSE ||
                               GetLastError() == ERROR_PIPE_CONNECTED;
        if (connected) {
            std::string payload;
            if (read_frame(pipe, payload)) {
                const auto decoded = owo::agent::decode_request(payload);
                if (decoded.ok) {
                    const auto& request = decoded.value;
                    std::cout << "[接收] action=" << owo::agent::action_name(request.action)
                              << " session=" << request.session_id
                              << " page=" << request.page << '\n'
                              << "       input=" << request.user_input << '\n'
                              << "       app=" << request.application.executable
                              << " class=" << request.application.window_class << '\n'
                              << "       context=" << request.session_context
                              << " command_id=" << request.command_id << '\n';
                    const auto response = process(request, sessions);
                    const auto encoded = owo::agent::encode_response(response);
                    std::cout << "[发送] status=" << owo::agent::status_name(response.status)
                              << " commands=" << response.commands.size()
                              << " message=" << response.message << "\n\n";
                    write_frame(pipe, encoded);
                } else {
                    std::cerr << "[拒绝] " << decoded.diagnostic << "\n\n";
                }
            }
            FlushFileBuffers(pipe);
            DisconnectNamedPipe(pipe);
        }
        CloseHandle(pipe);
    }
    return 0;
}

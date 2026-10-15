#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <Windows.h>

#include <chrono>
#include <iostream>
#include <string>
#include <thread>

namespace {

bool exchange_and_check(const std::uint64_t request_id,
                        const std::uint64_t generation,
                        const owo::protocol::MessageType type,
                        const std::string& expected,
                        const std::vector<std::string>& expected_candidates = {}) {
    const owo::protocol::Message request{type, request_id, generation, "nihao"};
    for (int attempt = 0; attempt < 100; ++attempt) {
        const auto result = owo::ipc::exchange(
            owo::ipc::kCorePipeName, owo::protocol::encode_message(request),
            std::chrono::milliseconds(100));
        if (result.status) {
            const auto decoded = owo::protocol::decode_message(result.response);
            const std::vector<std::string> expected_syllables =
                type == owo::protocol::MessageType::candidate_request
                    ? std::vector<std::string>{"ni", "hao"}
                    : std::vector<std::string>{};
            const std::vector<std::uint64_t> expected_consumed =
                type == owo::protocol::MessageType::candidate_request
                    ? std::vector<std::uint64_t>(expected_candidates.size(), 5)
                    : std::vector<std::uint64_t>{};
            return decoded.validation && decoded.message.request_id == request_id &&
                   decoded.message.context_generation == generation &&
                   decoded.message.text == expected &&
                   decoded.message.candidates == expected_candidates &&
                   decoded.message.syllables == expected_syllables &&
                   decoded.message.candidate_consumed == expected_consumed;
        }
        Sleep(20);
    }
    return false;
}

bool start_core(const wchar_t* path, PROCESS_INFORMATION& process) {
    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    const HANDLE null_handle = CreateFileW(L"NUL", GENERIC_READ | GENERIC_WRITE,
                                           FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (null_handle == INVALID_HANDLE_VALUE) return false;
    SetHandleInformation(null_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = null_handle;
    startup.hStdOutput = null_handle;
    startup.hStdError = null_handle;
    std::wstring command = L"\"" + std::wstring(path) + L"\" --no-config";
    const BOOL created = CreateProcessW(nullptr, command.data(), nullptr, nullptr, TRUE,
                                        CREATE_NO_WINDOW, nullptr, nullptr, &startup, &process);
    CloseHandle(null_handle);
    if (!created) return false;
    CloseHandle(process.hThread);
    process.hThread = nullptr;
    return true;
}

}  // namespace

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;

    PROCESS_INFORMATION process{};
    if (!start_core(argv[1], process)) {
        std::cerr << "CreateProcessW failed: " << GetLastError() << '\n';
        return 3;
    }
    CloseHandle(process.hThread);

    const bool first = exchange_and_check(
        10, 1, owo::protocol::MessageType::candidate_request, "你好", {"你好", "你号"});
    const bool second = exchange_and_check(
        11, 2, owo::protocol::MessageType::candidate_request, "你好", {"你好", "你号"});
    const bool shutdown = exchange_and_check(
        12, 3, owo::protocol::MessageType::shutdown_request, "shutdown_ack");
    const DWORD wait_result = WaitForSingleObject(process.hProcess, 3000);
    DWORD exit_code = STILL_ACTIVE;
    GetExitCodeProcess(process.hProcess, &exit_code);
    if (wait_result != WAIT_OBJECT_0) {
        // 仅终止本测试创建且未能通过协议关停的子进程，避免污染后续测试。
        TerminateProcess(process.hProcess, 5);
        WaitForSingleObject(process.hProcess, 1000);
    }
    CloseHandle(process.hProcess);

    const auto missing = owo::ipc::exchange(
        LR"(\\.\pipe\OwO.InputMethod.IntentionallyMissing)", "{}",
        std::chrono::milliseconds(10));
    const bool disconnected = !missing.status;

    PROCESS_INFORMATION crashed_process{};
    const bool crash_started = start_core(argv[1], crashed_process);
    const bool before_crash = crash_started && exchange_and_check(
        30, 5, owo::protocol::MessageType::candidate_request, "你好", {"你好", "你号"});
    if (crash_started) {
        TerminateProcess(crashed_process.hProcess, 99);
        WaitForSingleObject(crashed_process.hProcess, 1000);
        CloseHandle(crashed_process.hProcess);
    }
    PROCESS_INFORMATION restarted_process{};
    const bool restart_started = start_core(argv[1], restarted_process);
    const bool after_restart = restart_started && exchange_and_check(
        31, 6, owo::protocol::MessageType::candidate_request, "你好", {"你好", "你号"});
    const bool restart_shutdown = restart_started && exchange_and_check(
        32, 7, owo::protocol::MessageType::shutdown_request, "shutdown_ack");
    DWORD restart_exit = STILL_ACTIVE;
    DWORD restart_wait = WAIT_FAILED;
    if (restart_started) {
        restart_wait = WaitForSingleObject(restarted_process.hProcess, 3000);
        GetExitCodeProcess(restarted_process.hProcess, &restart_exit);
        if (restart_wait != WAIT_OBJECT_0) {
            TerminateProcess(restarted_process.hProcess, 5);
            WaitForSingleObject(restarted_process.hProcess, 1000);
        }
        CloseHandle(restarted_process.hProcess);
    }

    constexpr wchar_t stalled_pipe_name[] = LR"(\\.\pipe\OwO.InputMethod.StalledTest)";
    const HANDLE ready = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    std::jthread stalled_server([ready] {
        const HANDLE pipe = CreateNamedPipeW(
            stalled_pipe_name, PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, nullptr);
        SetEvent(ready);
        if (pipe != INVALID_HANDLE_VALUE) {
            ConnectNamedPipe(pipe, nullptr);
            Sleep(250);
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
        }
    });
    WaitForSingleObject(ready, 1000);
    CloseHandle(ready);
    const auto timeout_start = std::chrono::steady_clock::now();
    const auto stalled = owo::ipc::exchange(
        stalled_pipe_name,
        owo::protocol::encode_message({owo::protocol::MessageType::candidate_request,
                                       20, 4, "stall"}),
        std::chrono::milliseconds(30));
    const auto timeout_elapsed = std::chrono::steady_clock::now() - timeout_start;
    const bool response_timeout = !stalled.status &&
        stalled.status.error == owo::protocol::ErrorCode::timeout &&
        timeout_elapsed < std::chrono::milliseconds(200);

    return first && second && shutdown && disconnected && response_timeout &&
                   before_crash && after_restart && restart_shutdown &&
                   restart_wait == WAIT_OBJECT_0 && restart_exit == 0 &&
                   wait_result == WAIT_OBJECT_0 && exit_code == 0
               ? 0
               : 4;
}

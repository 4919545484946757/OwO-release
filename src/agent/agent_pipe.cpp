#include "owo/agent/agent_pipe.h"
#include "owo/agent/agent_protocol.h"

#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#endif

#include <algorithm>
#include <array>
#include <limits>

namespace owo::agent {

bool valid_agent_pipe_name(const std::wstring_view pipe_name) noexcept {
    constexpr std::wstring_view prefix = LR"(\\.\pipe\)";
    if (!pipe_name.starts_with(prefix) || pipe_name.size() <= prefix.size() ||
        pipe_name.size() > 240) return false;
    return std::all_of(pipe_name.begin() + static_cast<std::ptrdiff_t>(prefix.size()),
                       pipe_name.end(), [](const wchar_t value) {
        return (value >= L'a' && value <= L'z') || (value >= L'A' && value <= L'Z') ||
               (value >= L'0' && value <= L'9') || value == L'.' || value == L'-' ||
               value == L'_';
    });
}

PipeExchangeResult exchange_agent_pipe(const std::wstring_view pipe_name,
                                        const std::string_view request,
                                        const std::chrono::milliseconds timeout) {
#ifdef _WIN32
    if (!valid_agent_pipe_name(pipe_name) || request.empty() ||
        request.size() > kMaximumAgentPayloadBytes || timeout.count() <= 0 ||
        timeout > std::chrono::seconds(30))
        return {false, {}, "invalid bounded Agent pipe request"};
    const auto deadline = std::chrono::steady_clock::now() + timeout;
    const auto remaining = [&]() -> DWORD {
        const auto value = std::chrono::duration_cast<std::chrono::milliseconds>(
            deadline - std::chrono::steady_clock::now()).count();
        return value <= 0 ? 0 : static_cast<DWORD>(
            std::min<long long>(value, INFINITE - 1));
    };
    if (!WaitNamedPipeW(std::wstring(pipe_name).c_str(), remaining()))
        return {false, {}, "third-party Agent is not connected"};
    const HANDLE pipe = CreateFileW(std::wstring(pipe_name).c_str(),
        GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
        FILE_FLAG_OVERLAPPED, nullptr);
    if (pipe == INVALID_HANDLE_VALUE)
        return {false, {}, "third-party Agent pipe open failed"};
    const auto transfer = [&](void* buffer, const DWORD size, const bool writing) {
        OVERLAPPED operation{};
        operation.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (operation.hEvent == nullptr) return false;
        DWORD transferred = 0;
        const BOOL immediate = writing
            ? WriteFile(pipe, buffer, size, &transferred, &operation)
            : ReadFile(pipe, buffer, size, &transferred, &operation);
        bool ok = immediate != FALSE;
        if (!ok && GetLastError() == ERROR_IO_PENDING) {
            const DWORD wait = remaining();
            ok = wait != 0 && WaitForSingleObject(operation.hEvent, wait) == WAIT_OBJECT_0 &&
                 GetOverlappedResult(pipe, &operation, &transferred, FALSE) != FALSE;
        }
        if (!ok) {
            CancelIoEx(pipe, &operation);
            GetOverlappedResult(pipe, &operation, &transferred, TRUE);
        }
        CloseHandle(operation.hEvent);
        return ok && transferred == size;
    };
    const auto transfer_chunks = [&](char* bytes, const std::size_t size, const bool writing) {
        std::size_t offset = 0;
        while (offset < size) {
            const auto chunk = static_cast<DWORD>(std::min(
                size - offset, static_cast<std::size_t>((std::numeric_limits<DWORD>::max)())));
            if (!transfer(bytes + offset, chunk, writing)) return false;
            offset += chunk;
        }
        return true;
    };
    std::array<char, 4> header{
        static_cast<char>(request.size()), static_cast<char>(request.size() >> 8),
        static_cast<char>(request.size() >> 16), static_cast<char>(request.size() >> 24)};
    bool ok = transfer_chunks(header.data(), header.size(), true) &&
              transfer_chunks(const_cast<char*>(request.data()), request.size(), true) &&
              transfer_chunks(header.data(), header.size(), false);
    const auto response_size =
        static_cast<std::uint32_t>(static_cast<unsigned char>(header[0])) |
        (static_cast<std::uint32_t>(static_cast<unsigned char>(header[1])) << 8) |
        (static_cast<std::uint32_t>(static_cast<unsigned char>(header[2])) << 16) |
        (static_cast<std::uint32_t>(static_cast<unsigned char>(header[3])) << 24);
    std::string response;
    if (ok && response_size > 0 && response_size <= kMaximumAgentPayloadBytes) {
        response.resize(response_size);
        ok = transfer_chunks(response.data(), response.size(), false);
    } else ok = false;
    CloseHandle(pipe);
    return ok ? PipeExchangeResult{true, std::move(response), {}}
              : PipeExchangeResult{false, {}, "third-party Agent IPC timed out or disconnected"};
#else
    static_cast<void>(pipe_name); static_cast<void>(request); static_cast<void>(timeout);
    return {false, {}, "Agent IPC is supported on Windows only"};
#endif
}

}  // namespace owo::agent

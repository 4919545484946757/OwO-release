#include "owo/ipc/named_pipe.h"
#include "owo/protocol/envelope.h"

#include <Windows.h>

#include <array>
#include <atomic>
#include <chrono>
#include <cstdint>
#include <string>
#include <string_view>
#include <thread>

namespace {

bool read_exact(const HANDLE pipe, char* output, const DWORD size) {
    DWORD offset = 0;
    while (offset < size) {
        DWORD read = 0;
        if (!ReadFile(pipe, output + offset, size - offset, &read, nullptr) || read == 0)
            return false;
        offset += read;
    }
    return true;
}

bool write_all(const HANDLE pipe, const std::string_view bytes) {
    std::size_t offset = 0;
    while (offset < bytes.size()) {
        DWORD written = 0;
        if (!WriteFile(pipe, bytes.data() + offset,
                       static_cast<DWORD>(bytes.size() - offset),
                       &written, nullptr) || written == 0)
            return false;
        offset += written;
    }
    return true;
}

bool serve_once(const std::wstring& pipe_name, const HANDLE ready,
                const std::string_view expected_request,
                const std::string_view response) {
    const HANDLE pipe = CreateNamedPipeW(
        pipe_name.c_str(), PIPE_ACCESS_DUPLEX,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1, 4096, 4096, 0, nullptr);
    SetEvent(ready);
    if (pipe == INVALID_HANDLE_VALUE) return false;
    if (!ConnectNamedPipe(pipe, nullptr) && GetLastError() != ERROR_PIPE_CONNECTED) {
        CloseHandle(pipe);
        return false;
    }
    std::array<unsigned char, 4> prefix{};
    bool success = read_exact(pipe, reinterpret_cast<char*>(prefix.data()), 4);
    const std::uint32_t size = static_cast<std::uint32_t>(prefix[0]) |
                               (static_cast<std::uint32_t>(prefix[1]) << 8U) |
                               (static_cast<std::uint32_t>(prefix[2]) << 16U) |
                               (static_cast<std::uint32_t>(prefix[3]) << 24U);
    std::string request(size, '\0');
    success = success && size <= 4096 && read_exact(pipe, request.data(), size) &&
              request == expected_request;
    const auto framed = owo::protocol::frame(response);
    success = success && write_all(pipe, framed);
    FlushFileBuffers(pipe);
    DisconnectNamedPipe(pipe);
    CloseHandle(pipe);
    return success;
}

}  // namespace

int main() {
    const auto suffix = std::chrono::steady_clock::now().time_since_epoch().count();
    const std::wstring pipe_name =
        LR"(\\.\pipe\OwO.InputMethod.ReconnectTest.)" + std::to_wstring(suffix);
    const HANDLE first_ready = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    const HANDLE gap_started = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    const HANDLE second_ready = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (first_ready == nullptr || gap_started == nullptr || second_ready == nullptr) return 2;

    std::atomic<bool> server_ok{false};
    std::jthread server([&] {
        const bool first = serve_once(pipe_name, first_ready, "first", "first-response");
        SetEvent(gap_started);
        Sleep(40);
        const bool second = serve_once(pipe_name, second_ready, "second", "second-response");
        server_ok = first && second;
    });

    const bool first_available = WaitForSingleObject(first_ready, 1000) == WAIT_OBJECT_0;
    const auto first = owo::ipc::exchange(pipe_name.c_str(), "first",
                                           std::chrono::milliseconds(500));
    const bool gap = WaitForSingleObject(gap_started, 1000) == WAIT_OBJECT_0;
    const auto second = owo::ipc::exchange(pipe_name.c_str(), "second",
                                            std::chrono::milliseconds(500));
    if (!second.status && WaitForSingleObject(second_ready, 1000) == WAIT_OBJECT_0) {
        [[maybe_unused]] const auto cleanup = owo::ipc::exchange(
            pipe_name.c_str(), "second", std::chrono::milliseconds(500));
    }
    server.join();

    CloseHandle(first_ready);
    CloseHandle(gap_started);
    CloseHandle(second_ready);
    return first_available && gap && first.status && first.response == "first-response" &&
                   second.status && second.response == "second-response" && server_ok
               ? 0
               : 1;
}

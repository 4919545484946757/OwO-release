#include "../../apps/runtime_launcher/process_launch.h"
#include <iostream>
#include <string>

int wmain(int argc, wchar_t** argv) {
    if (argc > 1 && std::wstring_view(argv[1]) == L"--isolated-child") {
        std::cout << "isolated child\n" << std::flush;
        Sleep(1500);
        return 0;
    }
    if (argc > 1 && std::wstring_view(argv[1]) == L"--held-child") { Sleep(6000); return 0; }
    wchar_t executable[32768]{};
    GetModuleFileNameW(nullptr, executable, 32768);
    if (argc > 1 && std::wstring_view(argv[1]) == L"--hold-output-parent") {
        auto command = std::wstring(L"\"") + executable + L"\" --held-child";
        STARTUPINFOW startup{sizeof(startup)};
        PROCESS_INFORMATION child{};
        if (!CreateProcessW(executable, command.data(), nullptr, nullptr, TRUE,
                            CREATE_NO_WINDOW, nullptr, nullptr, &startup, &child)) return 10;
        CloseHandle(child.hThread); CloseHandle(child.hProcess);
        std::cout << "backend completed\n" << std::flush;
        return 0;
    }
    if (argc > 1 && std::wstring_view(argv[1]) == L"--emit-progress") {
        std::cout << "OWO_SETUP_PROGRESS|3|prepare\n" << std::flush;
        Sleep(300);
        std::cout << "OWO_SETUP_PROGRESS|99|complete\n" << std::flush;
        std::cerr << "diagnostic\n";
        return 0;
    }
    if (argc > 1 && std::wstring_view(argv[1]) == L"--exit-error") { std::cerr << "backend failure\n"; return 7; }

    SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE};
    HANDLE read{}, write{};
    if (!CreatePipe(&read, &write, &security, 0)) return 1;
    SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0);
    const auto log = CreateFileW(L"NUL", GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                                 &security, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (log == INVALID_HANDLE_VALUE) return 2;
    auto command = std::wstring(L"\"") + executable + L"\" --isolated-child";
    PROCESS_INFORMATION child{};
    const bool launched = owo::runtime::create_logged_process(executable, command.data(), nullptr, log, &child);
    CloseHandle(log); CloseHandle(write);
    if (!launched) { CloseHandle(read); return 3; }
    DWORD available = 0;
    const bool leaked = PeekNamedPipe(read, nullptr, 0, nullptr, &available, nullptr) != FALSE || GetLastError() != ERROR_BROKEN_PIPE;
    const bool alive = WaitForSingleObject(child.hProcess, 0) == WAIT_TIMEOUT;
    CloseHandle(read);
    WaitForSingleObject(child.hProcess, 5000);
    CloseHandle(child.hThread); CloseHandle(child.hProcess);
    if (leaked || !alive) { std::cerr << "Unrelated redirected pipe leaked to child\n"; return 4; }
    std::cout << "Runtime log-only handle inheritance test passed\n";
    return 0;
}

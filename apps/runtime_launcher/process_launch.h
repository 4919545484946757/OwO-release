#pragma once
#include <Windows.h>
#include <vector>

namespace owo::runtime {
// A redirected parent can own inheritable pipes unrelated to the child's log.
// STARTF_USESTDHANDLES alone does not restrict their inheritance.
inline bool create_logged_process(const wchar_t* executable, wchar_t* command,
                                  const wchar_t* directory, HANDLE log,
                                  PROCESS_INFORMATION* process) {
    STARTUPINFOEXW startup{};
    startup.StartupInfo.cb = sizeof(STARTUPINFOW);
    const bool inherit = log != nullptr && log != INVALID_HANDLE_VALUE;
    std::vector<unsigned char> attributes;
    if (inherit) {
        SIZE_T bytes = 0;
        InitializeProcThreadAttributeList(nullptr, 1, 0, &bytes);
        if (bytes == 0) return false;
        attributes.resize(bytes);
        startup.lpAttributeList = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(attributes.data());
        if (!InitializeProcThreadAttributeList(startup.lpAttributeList, 1, 0, &bytes)) return false;
        if (!UpdateProcThreadAttribute(startup.lpAttributeList, 0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &log, sizeof(log), nullptr, nullptr)) {
            DeleteProcThreadAttributeList(startup.lpAttributeList);
            return false;
        }
        startup.StartupInfo.cb = sizeof(startup);
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = log;
        startup.StartupInfo.hStdOutput = log;
        startup.StartupInfo.hStdError = log;
    }
    const bool launched = CreateProcessW(executable, command, nullptr, nullptr,
        inherit ? TRUE : FALSE, CREATE_NO_WINDOW | (inherit ? EXTENDED_STARTUPINFO_PRESENT : 0),
        nullptr, directory, &startup.StartupInfo, process) != FALSE;
    const auto error = GetLastError();
    if (inherit) DeleteProcThreadAttributeList(startup.lpAttributeList);
    SetLastError(error);
    return launched;
}
}

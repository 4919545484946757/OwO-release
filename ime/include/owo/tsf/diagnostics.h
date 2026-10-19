#pragma once
#include <windows.h>
#include <filesystem>
#include <fstream>

namespace owo::tsf {
// Explicit local opt-in beside the loaded DLL. Only fixed event labels and
// numeric metadata belong here; never pass text, keys, paths or host content.
// Each process has a bounded file, and diagnostics must never affect input.
inline void trace_tsf_event(const char* event, HRESULT result = S_OK,
                            HRESULT detail = S_OK) noexcept {
    try {
        static const std::filesystem::path destination = [] {
            HMODULE module{};
            if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                                   GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                reinterpret_cast<LPCWSTR>(&trace_tsf_event), &module))
                return std::filesystem::path{};
            wchar_t name[32768]{};
            const DWORD count = GetModuleFileNameW(module, name, 32768);
            if (!count || count >= 32768) return std::filesystem::path{};
            const auto folder = std::filesystem::path(name).parent_path();
            if (!std::filesystem::is_regular_file(folder / L"OwO.TSF.diagnostics.enabled"))
                return std::filesystem::path{};
            return folder / (L"OwO.TSF." + std::to_wstring(GetCurrentProcessId()) +
                             L".diagnostics.log");
        }();
        if (destination.empty()) return;
        if (std::filesystem::exists(destination) &&
            std::filesystem::file_size(destination) >= 1024 * 1024) return;
        std::ofstream log(destination, std::ios::app);
        log << "tick=" << GetTickCount64() << " pid=" << GetCurrentProcessId()
            << " tid=" << GetCurrentThreadId() << " event=" << event
            << " result=" << std::hex << static_cast<unsigned long>(result)
            << " detail=" << static_cast<unsigned long>(detail) << '\n';
    } catch (...) {}
}
} // namespace owo::tsf

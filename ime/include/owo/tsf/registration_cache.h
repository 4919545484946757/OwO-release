#pragma once

#include <Windows.h>

namespace owo::tsf {

// COM registration alone does not invalidate TSF's per-session assembly
// description cache. Resolve the documented API from the system DLL only.
[[nodiscard]] inline HRESULT invalidate_registration_cache() noexcept {
    const HMODULE module = LoadLibraryExW(L"msctf.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!module) return HRESULT_FROM_WIN32(GetLastError());
    using Invalidate = HRESULT(WINAPI*)();
    const auto invalidate = reinterpret_cast<Invalidate>(
        GetProcAddress(module, "TF_InvalidAssemblyListCacheIfExist"));
    const HRESULT result = invalidate ? invalidate() : HRESULT_FROM_WIN32(ERROR_PROC_NOT_FOUND);
    FreeLibrary(module);
    return result;
}

}  // namespace owo::tsf

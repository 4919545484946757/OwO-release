#include "owo/config/config_paths.h"

#ifdef _WIN32
#include <Windows.h>
#include <ShlObj.h>
#else
#include <cstdlib>
#endif

namespace owo::config {

std::filesystem::path local_data_root() {
#ifdef _WIN32
    PWSTR raw_path = nullptr;
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_DEFAULT,
                                       nullptr, &raw_path))) {
        const std::filesystem::path result(raw_path);
        CoTaskMemFree(raw_path);
        return result / L"OwO" / L"InputMethod";
    }
    if (raw_path != nullptr) CoTaskMemFree(raw_path);
    return {};
#else
    const char* home = std::getenv("HOME");
    return home != nullptr ? std::filesystem::path(home) / ".local" / "share" /
                                 "OwO" / "InputMethod"
                           : std::filesystem::path{};
#endif
}

std::filesystem::path default_config_path() {
    const auto root = local_data_root();
    return root.empty() ? std::filesystem::path{} : root / "config" / "owo.conf";
}

}  // namespace owo::config

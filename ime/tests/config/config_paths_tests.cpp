#include "owo/config/config_paths.h"

#include <filesystem>

int main() {
    const auto root = owo::config::local_data_root();
    const auto config = owo::config::default_config_path();
    if (root.empty() || config.empty() || config.parent_path().filename() != "config" ||
        config.filename() != "owo.conf" || config.parent_path().parent_path() != root)
        return 1;
#ifdef _WIN32
    if (root.filename() != L"InputMethod" || root.parent_path().filename() != L"OwO" ||
        !root.is_absolute())
        return 1;
#endif
    return 0;
}

#pragma once

#include <filesystem>

namespace owo::config {

/// 当前用户的非漫游 OwO 数据根目录。Windows 使用 FOLDERID_LocalAppData。
[[nodiscard]] std::filesystem::path local_data_root();
[[nodiscard]] std::filesystem::path default_config_path();

}  // namespace owo::config

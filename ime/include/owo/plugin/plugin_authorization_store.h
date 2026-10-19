#pragma once

#include "owo/plugin/plugin_authorization.h"

#include <filesystem>
#include <string>
#include <string_view>

namespace owo::plugin {

struct PluginAuthorizationStoreResult {
    bool ok{};
    PluginAuthorization value;
    std::filesystem::path record_path;
    std::string diagnostic;
};

/// Atomically stores a grant only when it exactly matches an installed version binding.
[[nodiscard]] PluginAuthorizationStoreResult save_plugin_authorization(
    const std::filesystem::path& plugin_store_root,
    const PluginAuthorization& authorization);

/// Loads and revalidates a grant against the current installed manifest and binding.
/// Missing records fail closed; callers must treat any failure as no permissions granted.
[[nodiscard]] PluginAuthorizationStoreResult load_plugin_authorization(
    const std::filesystem::path& plugin_store_root, std::string_view plugin_id,
    std::string_view version);

}  // namespace owo::plugin

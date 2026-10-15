#pragma once

#include <filesystem>
#include <string>
#include <string_view>

namespace owo::plugin {

struct PluginSandboxProfileResult {
    bool ok{};
    bool created{};
    std::wstring profile_name;
    std::wstring sid_string;
    std::filesystem::path profile_directory;
    std::string diagnostic;
};

/// Derives the only AppContainer profile name owned by a valid plugin ID without creating it.
[[nodiscard]] std::wstring plugin_sandbox_profile_name(std::string_view plugin_id);

/// Creates or opens the deterministic zero-capability AppContainer profile for a plugin ID.
/// Creating the profile does not launch code or grant filesystem/network capabilities.
[[nodiscard]] PluginSandboxProfileResult prepare_plugin_sandbox_profile(
    std::string_view plugin_id);

/// Deletes only a profile name produced by prepare_plugin_sandbox_profile.
/// Installed plugin files and data are not removed.
[[nodiscard]] PluginSandboxProfileResult delete_plugin_sandbox_profile(
    std::wstring_view profile_name);

}  // namespace owo::plugin

#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace owo::plugin {

inline constexpr std::uint32_t kPluginApiVersion = 1;

struct PluginManifest {
    std::string id;
    std::string name;
    std::string version;
    std::uint32_t api_version{};
    std::string runtime;
    std::string entry;
    std::vector<std::string> permissions;
    bool network{};
    std::string config_schema;
};

struct ManifestResult {
    bool ok{};
    PluginManifest value;
    std::string diagnostic;
};

/// Parse the closed P3C v1 JSON schema. This validates metadata only and never loads plugin code.
[[nodiscard]] ManifestResult parse_manifest(std::string_view json);

/// Reads at most 64 KiB and applies the same strict schema validation.
[[nodiscard]] ManifestResult load_manifest(const std::filesystem::path& path);

}  // namespace owo::plugin

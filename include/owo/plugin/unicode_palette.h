#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace owo::plugin {

struct UnicodePaletteItem {
    std::uint32_t codepoint{};
    std::string name;
    bool operator==(const UnicodePaletteItem&) const = default;
};

struct UnicodePaletteCategory {
    std::string id;
    std::string label;
    std::vector<UnicodePaletteItem> items;
    bool operator==(const UnicodePaletteCategory&) const = default;
};

struct UnicodePalette {
    std::vector<UnicodePaletteCategory> categories;
    bool operator==(const UnicodePalette&) const = default;
};

struct UnicodePaletteResult {
    bool active{};
    UnicodePalette palette;
    std::string plugin_id;
    std::string version;
    std::string diagnostic;
};

/// Loads one bounded declarative *.palette resource.
[[nodiscard]] UnicodePaletteResult load_unicode_palette_resource(
    const std::filesystem::path& resource);

/// Resolves the first valid active ui.unicode_palette resource deterministically.
[[nodiscard]] UnicodePaletteResult load_active_unicode_palette(
    const std::filesystem::path& plugin_store_root);

}  // namespace owo::plugin

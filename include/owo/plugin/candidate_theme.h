#pragma once

#include <cstdint>
#include <filesystem>
#include <string>

namespace owo::plugin {

struct CandidateThemeColor {
    std::uint32_t rgb{};
    float opacity{1.0F};
    bool operator==(const CandidateThemeColor&) const = default;
};

struct CandidateTheme {
    CandidateThemeColor background{0x202124, 1.0F};
    CandidateThemeColor border{0xffffff, 0.14F};
    CandidateThemeColor divider{0xffffff, 0.14F};
    CandidateThemeColor pinyin_text{0x8ab4f8, 1.0F};
    CandidateThemeColor candidate_text{0xffffff, 0.96F};
    CandidateThemeColor candidate_block{0x8ab4f8, 0.18F};
    CandidateThemeColor candidate_number{0x8ab4f8, 1.0F};
    bool background_blur_enabled{};
    float background_blur_radius{12.0F};
    bool shadow_enabled{};
    CandidateThemeColor shadow{0x000000, 0.32F};
    float shadow_blur_radius{14.0F};
    float shadow_offset_x{};
    float shadow_offset_y{4.0F};
    bool operator==(const CandidateTheme&) const = default;
};

struct CandidateThemeResult {
    bool active{};
    CandidateTheme theme;
    std::string plugin_id;
    std::string version;
    std::string diagnostic;
};

/// Loads one bounded declarative *.theme resource and optional flat settings JSON.
[[nodiscard]] CandidateThemeResult load_candidate_theme_resource(
    const std::filesystem::path& resource,
    const std::filesystem::path& settings = {});

/// Resolves the first valid active ui.candidate_theme resource deterministically.
[[nodiscard]] CandidateThemeResult load_active_candidate_theme(
    const std::filesystem::path& plugin_store_root);

}  // namespace owo::plugin

#include "owo/plugin/candidate_theme.h"

#include <algorithm>
#include <array>
#include <charconv>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string_view>

namespace {

constexpr std::string_view theme =
    "schema_version=1\n"
    "background.color=#101827\nbackground.opacity=0.72\n"
    "border.color=#88AACC\nborder.opacity=0.35\n"
    "divider.color=#FFFFFF\ndivider.opacity=0.16\n"
    "pinyin_text.color=#9CCBFF\npinyin_text.opacity=1\n"
    "candidate_text.color=#F7FAFF\ncandidate_text.opacity=0.96\n"
    "candidate_block.color=#78AEFF\ncandidate_block.opacity=0.22\n"
    "candidate_number.color=#B9D9FF\ncandidate_number.opacity=1\n"
    "background_blur.enabled=true\nbackground_blur.radius=9\n"
    "shadow.enabled=true\nshadow.color=#010203\nshadow.opacity=0.4\n"
    "shadow.blur_radius=16\nshadow.offset_x=2\nshadow.offset_y=5\n";

bool write(const std::filesystem::path& path, const std::string_view bytes) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
    return static_cast<bool>(output);
}

std::uint32_t blend(const owo::plugin::CandidateThemeColor color, const std::uint32_t background) {
    std::uint32_t result = 0;
    for (const auto shift : {0, 8, 16}) {
        const auto value = std::lround(((color.rgb >> shift) & 255) * color.opacity +
                                      ((background >> shift) & 255) * (1.0F - color.opacity));
        result |= static_cast<std::uint32_t>(value) << shift;
    }
    return result;
}

double luminance(const std::uint32_t rgb) {
    const auto channel = [rgb](int shift) {
        const double value = ((rgb >> shift) & 255) / 255.0;
        return value <= 0.04045 ? value / 12.92 : std::pow((value + 0.055) / 1.055, 2.4);
    };
    return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
}

bool readable(const owo::plugin::CandidateTheme& palette) {
    for (const auto underlying : {0x000000U, 0xffffffU}) {
        const auto background = blend(palette.background, underlying);
        for (const auto foreground : {palette.candidate_text, palette.pinyin_text, palette.candidate_number}) {
            const auto a = luminance(background), b = luminance(blend(foreground, background));
            if ((std::max(a, b) + 0.05) / (std::min(a, b) + 0.05) < 4.5) return false;
        }
    }
    return true;
}

bool monochrome(const owo::plugin::CandidateTheme& palette) {
    for (const auto color : {palette.background, palette.border, palette.divider, palette.pinyin_text,
                             palette.candidate_text, palette.candidate_block, palette.candidate_number, palette.shadow})
        if (((color.rgb >> 16) & 255) != ((color.rgb >> 8) & 255) ||
            ((color.rgb >> 8) & 255) != (color.rgb & 255)) return false;
    return true;
}

}  // namespace

int main(const int argc, char** argv) {
    if ((argc == 3 || argc == 4) && (std::string_view(argv[1]) == "--store" ||
                                   std::string_view(argv[1]) == "--store-light")) {
        const auto active = owo::plugin::load_active_candidate_theme(
            argv[2], std::string_view(argv[1]) == "--store-light");
        std::cout << "active=" << (active.active ? "true" : "false")
                  << " plugin=" << active.plugin_id << " version=" << active.version
                  << " background=#" << std::hex << active.theme.background.rgb << std::dec
                  << " opacity=" << active.theme.background.opacity
                  << " diagnostic=" << active.diagnostic << '\n';
        std::uint32_t expected = std::string_view(argv[1]) == "--store-light" ? 0xf5f5f5 : 0x101827;
        if (argc == 4) {
            const auto parsed = std::from_chars(argv[3], argv[3] + std::strlen(argv[3]),
                                                expected, 16);
            if (parsed.ec != std::errc{} || *parsed.ptr != '\0') return 10;
        }
        return active.active && active.plugin_id == "org.owo.candidate-glass" &&
                       active.theme.background.rgb == expected ? 0 : 9;
    }
    if (argc != 2 && argc != 3) return 1;
    const std::filesystem::path root(argv[1]);
    std::error_code error;
    std::filesystem::remove_all(root, error);
    error.clear();
    if (!std::filesystem::create_directories(root, error) || error) return 2;
    const auto resource = root / "candidate.theme";
    const auto settings = root / "settings.json";
    if (!write(resource, theme)) return 3;
    auto loaded = owo::plugin::load_candidate_theme_resource(resource);
    if (!loaded.active || loaded.theme.background.rgb != 0x101827 ||
        std::abs(loaded.theme.background.opacity - 0.72F) > 0.001F ||
        loaded.theme.candidate_number.rgb != 0xB9D9FF ||
        !loaded.theme.background_blur_enabled ||
        std::abs(loaded.theme.background_blur_radius - 9.0F) > 0.001F ||
        !loaded.theme.shadow_enabled || loaded.theme.shadow.rgb != 0x010203 ||
        std::abs(loaded.theme.shadow.opacity - 0.4F) > 0.001F) return 4;
    if (!write(settings,
        R"({"background_color":"#223344","background_opacity":0.5,"candidate_number_color":"#ABCDEF","background_blur_enabled":false,"background_blur_radius":18,"shadow_enabled":false,"shadow_color":"#445566","shadow_opacity":0.6,"shadow_blur_radius":20,"shadow_offset_x":-3,"shadow_offset_y":7})"))
        return 5;
    loaded = owo::plugin::load_candidate_theme_resource(resource, settings);
    if (!loaded.active || loaded.theme.background.rgb != 0x223344 ||
        std::abs(loaded.theme.background.opacity - 0.5F) > 0.001F ||
        loaded.theme.candidate_number.rgb != 0xABCDEF ||
        loaded.theme.background_blur_enabled ||
        std::abs(loaded.theme.background_blur_radius - 18.0F) > 0.001F ||
        loaded.theme.shadow_enabled || loaded.theme.shadow.rgb != 0x445566 ||
        std::abs(loaded.theme.shadow.opacity - 0.6F) > 0.001F ||
        std::abs(loaded.theme.shadow_blur_radius - 20.0F) > 0.001F ||
        std::abs(loaded.theme.shadow_offset_x + 3.0F) > 0.001F ||
        std::abs(loaded.theme.shadow_offset_y - 7.0F) > 0.001F) return 6;
    if (owo::plugin::load_candidate_theme_resource(resource, settings, true).theme != loaded.theme)
        return 11; // v1 themes remain fixed, even on a light system.
    const auto builtin_light = owo::plugin::default_candidate_theme(true);
    const auto builtin_dark = owo::plugin::default_candidate_theme(false);
    if (builtin_dark != owo::plugin::CandidateTheme{} || builtin_light == builtin_dark ||
        builtin_light.background.rgb != 0xf5f5f5 || builtin_light.candidate_text.rgb != 0x202020)
        return 12;
    if (!readable(builtin_dark) || !readable(builtin_light) || !monochrome(builtin_light) ||
        std::abs(builtin_light.candidate_block.opacity - 0.05F) > 0.001F) return 24;
    if (argc == 3) for (const auto light_mode : {false, true}) {
        const auto official = owo::plugin::load_candidate_theme_resource(argv[2], {}, light_mode);
        if (!official.active || !readable(official.theme) ||
            official.theme.background.rgb != (light_mode ? 0xf5f5f5U : 0x101827U) ||
            (light_mode && (!monochrome(official.theme) ||
                std::abs(official.theme.candidate_block.opacity - 0.05F) > 0.001F))) return 25;
    }
    std::string adaptive(theme);
    adaptive.replace(0, std::string_view("schema_version=1").size(), "schema_version=2");
    for (const auto role : {"background", "border", "divider", "pinyin_text", "candidate_text",
                             "candidate_block", "candidate_number"}) {
        adaptive += "light." + std::string(role) + ".color=#112233\n";
        adaptive += "light." + std::string(role) + ".opacity=0.8\n";
    }
    adaptive += "light.shadow.color=#334455\nlight.shadow.opacity=0.18\n";
    if (!write(resource, adaptive)) return 13;
    const auto dark = owo::plugin::load_candidate_theme_resource(resource, settings, false);
    const auto light = owo::plugin::load_candidate_theme_resource(resource, settings, true);
    if (!light.active || !dark.active || !light.light_palette || dark.light_palette ||
        dark.theme.background.rgb != 0x223344 ||
        light.theme.background.rgb != 0x112233 || std::abs(light.theme.background.opacity - 0.8F) > 0.001F ||
        light.theme.shadow.rgb != 0x334455 || std::abs(light.theme.shadow.opacity - 0.18F) > 0.001F ||
        light.theme.background_blur_enabled != dark.theme.background_blur_enabled ||
        light.theme.background_blur_radius != dark.theme.background_blur_radius ||
        light.theme.shadow_enabled != dark.theme.shadow_enabled || light.theme.shadow_offset_y != 7.0F)
        return 14;
    if (!write(settings, R"({"theme_mode":"dark","light_candidate_text_color":"#445566"})")) return 15;
    const auto forced_dark = owo::plugin::load_candidate_theme_resource(resource, settings, true);
    if (!forced_dark.active || forced_dark.light_palette || forced_dark.theme.background.rgb != 0x101827) return 16;
    if (!write(settings, R"({"theme_mode":"light","light_candidate_text_color":"#445566","light_background_opacity":0.6})")) return 17;
    const auto forced_light = owo::plugin::load_candidate_theme_resource(resource, settings, false);
    if (!forced_light.active || !forced_light.light_palette || forced_light.theme.candidate_text.rgb != 0x445566 ||
        std::abs(forced_light.theme.background.opacity - 0.6F) > 0.001F) return 18;
    if (!write(settings, R"({"theme_mode":"system"})")) return 19;
    for (int index = 0; index < 20; ++index) {
        const bool mode = (index % 2) != 0;
        const auto switched = owo::plugin::load_candidate_theme_resource(resource, settings, mode);
        if (!switched.active || switched.light_palette != mode ||
            switched.theme.background.rgb != (mode ? 0x112233U : 0x101827U)) return 20;
    }
    const std::array<std::pair<std::string_view, owo::plugin::CandidateThemeColor owo::plugin::CandidateTheme::*>, 8> roles{{
        {"background", &owo::plugin::CandidateTheme::background},
        {"border", &owo::plugin::CandidateTheme::border},
        {"divider", &owo::plugin::CandidateTheme::divider},
        {"pinyin_text", &owo::plugin::CandidateTheme::pinyin_text},
        {"candidate_text", &owo::plugin::CandidateTheme::candidate_text},
        {"candidate_block", &owo::plugin::CandidateTheme::candidate_block},
        {"candidate_number", &owo::plugin::CandidateTheme::candidate_number},
        {"shadow", &owo::plugin::CandidateTheme::shadow},
    }};
    for (const bool edit_light : {false, true}) {
        std::string independent = R"({"theme_mode":"system")";
        for (std::size_t index = 0; index < roles.size(); ++index) {
            const auto name = std::string(roles[index].first);
            independent += ",\"" + name + "_color\":\"#10000" + std::to_string(index) + "\"";
            independent += ",\"" + name + "_opacity\":" + std::to_string((index + 1) / 10.0);
            independent += ",\"light_" + name + "_color\":\"#" + (edit_light ? std::string("30000") : std::string("20000")) +
                           std::to_string(index) + "\"";
            independent += ",\"light_" + name + "_opacity\":" + std::to_string((index + 1) / (edit_light ? 25.0 : 20.0));
        }
        independent += "}";
        if (!write(settings, independent)) return 26;
        for (const bool light_mode : {false, true}) {
            const auto configured = owo::plugin::load_candidate_theme_resource(resource, settings, light_mode);
            if (!configured.active || configured.light_palette != light_mode) return 27;
            for (std::size_t index = 0; index < roles.size(); ++index) {
                const auto color = configured.theme.*roles[index].second;
                const auto expected_rgb = (light_mode ? (edit_light ? 0x300000U : 0x200000U) : 0x100000U) + index;
                const auto expected_opacity = (index + 1) / (light_mode ? (edit_light ? 25.0 : 20.0) : 10.0);
                if (color.rgb != expected_rgb || std::abs(color.opacity - expected_opacity) > 0.001) return 28;
            }
        }
    }
    if (!write(resource, adaptive + "light.border.color=#000000\n") ||
        owo::plugin::load_candidate_theme_resource(resource).active) return 21;
    if (!write(resource, std::string(theme) + "light.border.color=#000000\n") ||
        owo::plugin::load_candidate_theme_resource(resource).active) return 22;
    if (!write(resource, "schema_version=2\n" + std::string(theme).substr(theme.find('\n') + 1)) ||
        owo::plugin::load_candidate_theme_resource(resource).active) return 23;
    if (!write(resource, "schema_version=1\nbackground.color=#000000\n")) return 7;
    if (owo::plugin::load_candidate_theme_resource(resource).active) return 8;
    std::filesystem::remove_all(root, error);
    return 0;
}

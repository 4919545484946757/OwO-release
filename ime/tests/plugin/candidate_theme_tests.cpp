#include "owo/plugin/candidate_theme.h"

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

}  // namespace

int main(const int argc, char** argv) {
    if ((argc == 3 || argc == 4) && std::string_view(argv[1]) == "--store") {
        const auto active = owo::plugin::load_active_candidate_theme(argv[2]);
        std::cout << "active=" << (active.active ? "true" : "false")
                  << " plugin=" << active.plugin_id << " version=" << active.version
                  << " background=#" << std::hex << active.theme.background.rgb << std::dec
                  << " opacity=" << active.theme.background.opacity
                  << " diagnostic=" << active.diagnostic << '\n';
        std::uint32_t expected = 0x101827;
        if (argc == 4) {
            const auto parsed = std::from_chars(argv[3], argv[3] + std::strlen(argv[3]),
                                                expected, 16);
            if (parsed.ec != std::errc{} || *parsed.ptr != '\0') return 10;
        }
        return active.active && active.plugin_id == "org.owo.candidate-glass" &&
                       active.theme.background.rgb == expected ? 0 : 9;
    }
    if (argc != 2) return 1;
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
    if (!write(resource, "schema_version=1\nbackground.color=#000000\n")) return 7;
    if (owo::plugin::load_candidate_theme_resource(resource).active) return 8;
    std::filesystem::remove_all(root, error);
    return 0;
}

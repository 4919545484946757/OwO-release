#include "owo/plugin/candidate_theme.h"

#include "owo/plugin/plugin_store.h"

#include <algorithm>
#include <array>
#include <charconv>
#include <cmath>
#include <fstream>
#include <optional>
#include <string_view>
#include <vector>

namespace owo::plugin {
namespace {

constexpr std::size_t kMaximumThemeBytes = 64 * 1024;
constexpr std::size_t kMaximumSettingsBytes = 256 * 1024;

struct Role {
    std::string_view name;
    CandidateThemeColor CandidateTheme::*member;
};

constexpr std::array<Role, 7> kRoles{{
    {"background", &CandidateTheme::background},
    {"border", &CandidateTheme::border},
    {"divider", &CandidateTheme::divider},
    {"pinyin_text", &CandidateTheme::pinyin_text},
    {"candidate_text", &CandidateTheme::candidate_text},
    {"candidate_block", &CandidateTheme::candidate_block},
    {"candidate_number", &CandidateTheme::candidate_number},
}};

std::string trim(std::string_view input) {
    const auto begin = input.find_first_not_of(" \t\r\n");
    if (begin == std::string_view::npos) return {};
    const auto end = input.find_last_not_of(" \t\r\n");
    return std::string(input.substr(begin, end - begin + 1));
}

std::optional<std::string> read_bounded(const std::filesystem::path& path,
                                        const std::size_t maximum) {
    std::error_code error;
    const auto size = std::filesystem::file_size(path, error);
    if (error || size == 0 || size > maximum) return std::nullopt;
    std::ifstream input(path, std::ios::binary);
    if (!input) return std::nullopt;
    std::string result(static_cast<std::size_t>(size), '\0');
    input.read(result.data(), static_cast<std::streamsize>(result.size()));
    if (!input || input.peek() != std::char_traits<char>::eof()) return std::nullopt;
    return result;
}

bool parse_color(const std::string_view text, std::uint32_t& result) {
    if (text.size() != 7 || text.front() != '#') return false;
    std::uint32_t value = 0;
    const auto parsed = std::from_chars(text.data() + 1, text.data() + text.size(),
                                        value, 16);
    if (parsed.ec != std::errc{} || parsed.ptr != text.data() + text.size()) return false;
    result = value;
    return true;
}

bool parse_opacity(const std::string_view text, float& result) {
    float value = 0.0F;
    const auto parsed = std::from_chars(text.data(), text.data() + text.size(), value);
    if (parsed.ec != std::errc{} || parsed.ptr != text.data() + text.size() ||
        !std::isfinite(value) || value < 0.0F || value > 1.0F) return false;
    result = value;
    return true;
}

bool parse_number(const std::string_view text, const float minimum,
                  const float maximum, float& result) {
    float value = 0.0F;
    const auto parsed = std::from_chars(text.data(), text.data() + text.size(), value);
    if (parsed.ec != std::errc{} || parsed.ptr != text.data() + text.size() ||
        !std::isfinite(value) || value < minimum || value > maximum) return false;
    result = value;
    return true;
}

bool parse_boolean(const std::string_view text, bool& result) {
    if (text == "true") { result = true; return true; }
    if (text == "false") { result = false; return true; }
    return false;
}

const Role* find_role(const std::string_view name) {
    const auto found = std::find_if(kRoles.begin(), kRoles.end(),
        [name](const Role& role) { return role.name == name; });
    return found == kRoles.end() ? nullptr : &*found;
}

bool parse_descriptor(const std::string_view bytes, CandidateTheme& theme,
                      std::string& diagnostic) {
    bool version = false;
    std::array<bool, kRoles.size()> colors{};
    std::array<bool, kRoles.size()> opacities{};
    std::size_t offset = 0;
    while (offset <= bytes.size()) {
        const auto end = bytes.find('\n', offset);
        const auto line = trim(bytes.substr(offset, end == std::string_view::npos
            ? bytes.size() - offset : end - offset));
        offset = end == std::string_view::npos ? bytes.size() + 1 : end + 1;
        if (line.empty() || line.front() == '#') continue;
        const auto equal = line.find('=');
        if (equal == std::string::npos || line.find('=', equal + 1) != std::string::npos) {
            diagnostic = "candidate theme contains an invalid line";
            return false;
        }
        const auto key = std::string_view(line).substr(0, equal);
        const auto value = std::string_view(line).substr(equal + 1);
        if (key == "schema_version") {
            if (version || value != "1") {
                diagnostic = "candidate theme schema_version must be exactly 1";
                return false;
            }
            version = true;
            continue;
        }
        bool effect_valid = true;
        bool effect_key = true;
        if (key == "background_blur.enabled")
            effect_valid = parse_boolean(value, theme.background_blur_enabled);
        else if (key == "background_blur.radius")
            effect_valid = parse_number(value, 0.0F, 24.0F,
                                        theme.background_blur_radius);
        else if (key == "shadow.enabled")
            effect_valid = parse_boolean(value, theme.shadow_enabled);
        else if (key == "shadow.color")
            effect_valid = parse_color(value, theme.shadow.rgb);
        else if (key == "shadow.opacity")
            effect_valid = parse_opacity(value, theme.shadow.opacity);
        else if (key == "shadow.blur_radius")
            effect_valid = parse_number(value, 1.0F, 32.0F,
                                        theme.shadow_blur_radius);
        else if (key == "shadow.offset_x")
            effect_valid = parse_number(value, -16.0F, 16.0F,
                                        theme.shadow_offset_x);
        else if (key == "shadow.offset_y")
            effect_valid = parse_number(value, -16.0F, 16.0F,
                                        theme.shadow_offset_y);
        else effect_key = false;
        if (effect_key) {
            if (!effect_valid) {
                diagnostic = "candidate theme contains an invalid effect value";
                return false;
            }
            continue;
        }
        const auto dot = key.rfind('.');
        const auto* role = dot == std::string_view::npos
            ? nullptr : find_role(key.substr(0, dot));
        if (role == nullptr) {
            diagnostic = "candidate theme contains an unknown role";
            return false;
        }
        const auto index = static_cast<std::size_t>(role - kRoles.data());
        auto& color = theme.*(role->member);
        const auto property = key.substr(dot + 1);
        if (property == "color") {
            if (colors[index] || !parse_color(value, color.rgb)) {
                diagnostic = "candidate theme contains an invalid or duplicate color";
                return false;
            }
            colors[index] = true;
        } else if (property == "opacity") {
            if (opacities[index] || !parse_opacity(value, color.opacity)) {
                diagnostic = "candidate theme contains an invalid or duplicate opacity";
                return false;
            }
            opacities[index] = true;
        } else {
            diagnostic = "candidate theme contains an unknown property";
            return false;
        }
    }
    if (!version || std::find(colors.begin(), colors.end(), false) != colors.end() ||
        std::find(opacities.begin(), opacities.end(), false) != opacities.end()) {
        diagnostic = "candidate theme must define every semantic color and opacity";
        return false;
    }
    return true;
}

std::size_t skip_space(const std::string_view input, std::size_t offset) {
    while (offset < input.size() &&
           (input[offset] == ' ' || input[offset] == '\t' ||
            input[offset] == '\r' || input[offset] == '\n')) ++offset;
    return offset;
}

std::optional<std::string_view> json_scalar(const std::string_view json,
                                            const std::string_view key) {
    const std::string marker = "\"" + std::string(key) + "\"";
    auto offset = json.find(marker);
    if (offset == std::string_view::npos) return std::nullopt;
    offset = skip_space(json, offset + marker.size());
    if (offset >= json.size() || json[offset] != ':') return std::nullopt;
    offset = skip_space(json, offset + 1);
    if (offset >= json.size()) return std::nullopt;
    if (json[offset] == '"') {
        const auto end = json.find('"', offset + 1);
        if (end == std::string_view::npos || json.substr(offset + 1, end - offset - 1).find('\\')
            != std::string_view::npos) return std::nullopt;
        return json.substr(offset, end - offset + 1);
    }
    const auto end = json.find_first_of(",}\r\n\t ", offset);
    return json.substr(offset, end == std::string_view::npos ? json.size() - offset
                                                             : end - offset);
}

void apply_settings(const std::string_view json, CandidateTheme& theme) {
    for (const auto& role : kRoles) {
        auto& value = theme.*(role.member);
        if (const auto scalar = json_scalar(json, std::string(role.name) + "_color");
            scalar && scalar->size() == 9 && scalar->front() == '"' &&
            scalar->back() == '"') {
            std::uint32_t rgb = 0;
            if (parse_color(scalar->substr(1, 7), rgb)) value.rgb = rgb;
        }
        if (const auto scalar = json_scalar(json, std::string(role.name) + "_opacity")) {
            float opacity = 0.0F;
            if (parse_opacity(*scalar, opacity)) value.opacity = opacity;
        }
    }
    const auto apply_bool = [&json](const std::string_view key, bool& target) {
        if (const auto scalar = json_scalar(json, key)) parse_boolean(*scalar, target);
    };
    const auto apply_number = [&json](const std::string_view key, const float minimum,
                                      const float maximum, float& target) {
        if (const auto scalar = json_scalar(json, key))
            parse_number(*scalar, minimum, maximum, target);
    };
    apply_bool("background_blur_enabled", theme.background_blur_enabled);
    apply_number("background_blur_radius", 0.0F, 24.0F,
                 theme.background_blur_radius);
    apply_bool("shadow_enabled", theme.shadow_enabled);
    if (const auto scalar = json_scalar(json, "shadow_color");
        scalar && scalar->size() == 9 && scalar->front() == '"' &&
        scalar->back() == '"') {
        std::uint32_t rgb = 0;
        if (parse_color(scalar->substr(1, 7), rgb)) theme.shadow.rgb = rgb;
    }
    if (const auto scalar = json_scalar(json, "shadow_opacity"))
        parse_opacity(*scalar, theme.shadow.opacity);
    apply_number("shadow_blur_radius", 1.0F, 32.0F,
                 theme.shadow_blur_radius);
    apply_number("shadow_offset_x", -16.0F, 16.0F, theme.shadow_offset_x);
    apply_number("shadow_offset_y", -16.0F, 16.0F, theme.shadow_offset_y);
}

}  // namespace

CandidateThemeResult load_candidate_theme_resource(
    const std::filesystem::path& resource, const std::filesystem::path& settings) {
    CandidateThemeResult result;
    const auto descriptor = read_bounded(resource, kMaximumThemeBytes);
    if (!descriptor) {
        result.diagnostic = "candidate theme resource is missing or invalid";
        return result;
    }
    if (!parse_descriptor(*descriptor, result.theme, result.diagnostic)) return result;
    if (!settings.empty()) {
        std::error_code error;
        if (std::filesystem::exists(settings, error) && !error) {
            if (const auto values = read_bounded(settings, kMaximumSettingsBytes))
                apply_settings(*values, result.theme);
        }
    }
    result.active = true;
    return result;
}

CandidateThemeResult load_active_candidate_theme(
    const std::filesystem::path& plugin_store_root) {
    CandidateThemeResult fallback;
    if (plugin_store_root.empty()) return fallback;
    std::error_code error;
    const auto active_root = plugin_store_root / L"active";
    if (!std::filesystem::is_directory(active_root, error) || error) return fallback;
    std::vector<std::filesystem::path> records;
    for (std::filesystem::directory_iterator iterator(active_root, error), end;
         !error && iterator != end; iterator.increment(error)) {
        if (iterator->is_regular_file(error) && !error &&
            iterator->path().extension() == L".record") records.push_back(iterator->path());
    }
    if (error) return fallback;
    std::sort(records.begin(), records.end());
    for (const auto& record : records) {
        const auto id = record.stem().string();
        const auto installed = query_active_plugin_version(plugin_store_root, id);
        if (!installed.ok || installed.manifest.runtime != "resource" ||
            std::find(installed.manifest.permissions.begin(), installed.manifest.permissions.end(),
                      "ui.candidate_theme") == installed.manifest.permissions.end()) continue;
        auto loaded = load_candidate_theme_resource(
            installed.installed_path / std::filesystem::path(installed.manifest.entry),
            plugin_store_root / L"data" / std::filesystem::path(id) / L"settings.json");
        if (!loaded.active) {
            fallback.diagnostic = loaded.diagnostic;
            continue;
        }
        loaded.plugin_id = installed.manifest.id;
        loaded.version = installed.manifest.version;
        return loaded;
    }
    return fallback;
}

}  // namespace owo::plugin

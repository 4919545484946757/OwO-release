#pragma once

#include <array>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <optional>
#include <string_view>

#ifdef _WIN32
#include <Windows.h>
#endif

namespace owo::plugin {

using InputSchemeIcon = std::array<std::uint16_t, 16>;

// A passive, bounded asset: no codecs, SVG/XAML, or plugin code in the TSF host.
inline std::optional<InputSchemeIcon> parse_input_scheme_icon(std::string_view text) {
    const auto next = [&]() {
        const auto end = text.find('\n');
        auto line = text.substr(0, end);
        text.remove_prefix(end == text.npos ? text.size() : end + 1);
        if (line.ends_with('\r')) line.remove_suffix(1);
        return line;
    };
    if (next() != "OWO_SCHEME_ICON_MASK_V1") return {};
    InputSchemeIcon result{};
    bool ink{};
    for (auto& row : result) {
        const auto line = next();
        if (line.size() != 16) return {};
        for (char bit : line) {
            if (bit != '0' && bit != '1') return {};
            row = static_cast<std::uint16_t>((row << 1) | (bit == '1'));
        }
        ink |= row != 0;
    }
    return ink && text.empty() ? std::optional(result) : std::nullopt;
}

inline std::optional<InputSchemeIcon> read_input_scheme_icon(
    const std::filesystem::path& installed, std::string_view local) {
    // Caller supplies the authorized active version; local must be one plain path segment.
    if (local.empty() || local.size() > 128 || local == "." || local == ".." || local.back() == '.') return {};
    for (char c : local) {
        if (!(c >= 'a' && c <= 'z') && !(c >= 'A' && c <= 'Z') &&
            !(c >= '0' && c <= '9') && c != '_' && c != '-' && c != '.') return {};
    }
    const auto file = installed / "input-schemes" / local / "icon.mask";
    for (auto path = file; !path.empty(); path = path.parent_path()) {
#ifdef _WIN32
        const auto attributes = GetFileAttributesW(path.c_str());
        if (attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_REPARSE_POINT) ||
            (path == file && (attributes & FILE_ATTRIBUTE_DIRECTORY))) return {};
#else
        std::error_code error;
        const auto type = std::filesystem::symlink_status(path, error).type();
        if (error || type != (path == file ? std::filesystem::file_type::regular
                                          : std::filesystem::file_type::directory)) return {};
#endif
        if (path == path.parent_path()) break;
    }
    std::ifstream input(file, std::ios::binary);
    if (!input) return {};
    char bytes[384]{};
    input.read(bytes, sizeof bytes);
    if (input.bad() || input.gcount() == sizeof bytes) return {};
    return parse_input_scheme_icon(std::string_view(bytes, static_cast<std::size_t>(input.gcount())));
}

} // namespace owo::plugin

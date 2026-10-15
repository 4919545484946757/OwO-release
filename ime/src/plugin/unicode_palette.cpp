#include "owo/plugin/unicode_palette.h"

#include "owo/plugin/plugin_store.h"

#include <algorithm>
#include <charconv>
#include <fstream>
#include <optional>
#include <set>
#include <string_view>

namespace owo::plugin {
namespace {

constexpr std::size_t kMaximumPaletteBytes = 256 * 1024;
constexpr std::size_t kMaximumCategories = 32;
constexpr std::size_t kMaximumItemsPerCategory = 512;

std::string trim(const std::string_view input) {
    const auto begin = input.find_first_not_of(" \t\r\n");
    if (begin == std::string_view::npos) return {};
    const auto end = input.find_last_not_of(" \t\r\n");
    return std::string(input.substr(begin, end - begin + 1));
}

std::optional<std::string> read_bounded(const std::filesystem::path& path) {
    std::error_code error;
    const auto size = std::filesystem::file_size(path, error);
    if (error || size == 0 || size > kMaximumPaletteBytes) return std::nullopt;
    std::ifstream input(path, std::ios::binary);
    if (!input) return std::nullopt;
    std::string result(static_cast<std::size_t>(size), '\0');
    input.read(result.data(), static_cast<std::streamsize>(result.size()));
    if (!input || input.peek() != std::char_traits<char>::eof()) return std::nullopt;
    return result;
}

bool valid_identifier(const std::string_view value) {
    return !value.empty() && value.size() <= 32 &&
        std::all_of(value.begin(), value.end(), [](const unsigned char character) {
            return (character >= 'a' && character <= 'z') ||
                   (character >= '0' && character <= '9') ||
                   character == '-' || character == '_';
        });
}

bool valid_codepoint(const std::uint32_t value) {
    return value <= 0x10ffff && !(value >= 0xd800 && value <= 0xdfff);
}

}  // namespace

UnicodePaletteResult load_unicode_palette_resource(
    const std::filesystem::path& resource) {
    UnicodePaletteResult result;
    const auto descriptor = read_bounded(resource);
    if (!descriptor) {
        result.diagnostic = "unicode palette resource is missing or invalid";
        return result;
    }
    bool version = false;
    UnicodePaletteCategory* category = nullptr;
    std::set<std::string> category_ids;
    std::size_t offset = 0;
    while (offset <= descriptor->size()) {
        const auto end = descriptor->find('\n', offset);
        const auto line = trim(std::string_view(*descriptor).substr(
            offset, end == std::string_view::npos ? descriptor->size() - offset
                                                  : end - offset));
        offset = end == std::string_view::npos ? descriptor->size() + 1 : end + 1;
        if (line.empty() || line.front() == '#') continue;
        const auto equal = line.find('=');
        if (equal == std::string::npos) {
            result.diagnostic = "unicode palette contains an invalid line";
            return result;
        }
        const auto key = std::string_view(line).substr(0, equal);
        const auto value = std::string_view(line).substr(equal + 1);
        if (key == "schema_version") {
            if (version || value != "1") {
                result.diagnostic = "unicode palette schema_version must be exactly 1";
                return result;
            }
            version = true;
        } else if (key == "category") {
            const auto separator = value.find('|');
            if (separator == std::string_view::npos ||
                !valid_identifier(value.substr(0, separator)) ||
                separator + 1 == value.size() || value.size() - separator - 1 > 64 ||
                result.palette.categories.size() >= kMaximumCategories) {
                result.diagnostic = "unicode palette contains an invalid category";
                return result;
            }
            const std::string id(value.substr(0, separator));
            if (!category_ids.insert(id).second) {
                result.diagnostic = "unicode palette contains a duplicate category";
                return result;
            }
            result.palette.categories.push_back(
                {id, std::string(value.substr(separator + 1)), {}});
            category = &result.palette.categories.back();
        } else if (key == "item") {
            if (category == nullptr || category->items.size() >= kMaximumItemsPerCategory) {
                result.diagnostic = "unicode palette item has no valid category";
                return result;
            }
            const auto separator = value.find('|');
            const auto code = separator == std::string_view::npos ? value
                                                                   : value.substr(0, separator);
            std::uint32_t codepoint = 0;
            const auto parsed = std::from_chars(code.data(), code.data() + code.size(),
                                                codepoint, 16);
            if (parsed.ec != std::errc{} || parsed.ptr != code.data() + code.size() ||
                !valid_codepoint(codepoint)) {
                result.diagnostic = "unicode palette contains an invalid codepoint";
                return result;
            }
            const auto name = separator == std::string_view::npos
                ? std::string{} : std::string(value.substr(separator + 1));
            if (name.size() > 96) {
                result.diagnostic = "unicode palette item name is too long";
                return result;
            }
            category->items.push_back({codepoint, name});
        } else {
            result.diagnostic = "unicode palette contains an unknown field";
            return result;
        }
    }
    if (!version || result.palette.categories.empty() ||
        std::any_of(result.palette.categories.begin(), result.palette.categories.end(),
                    [](const UnicodePaletteCategory& value) {
                        return value.items.empty();
                    })) {
        result.diagnostic = "unicode palette must define non-empty categories";
        return result;
    }
    result.active = true;
    return result;
}

UnicodePaletteResult load_active_unicode_palette(
    const std::filesystem::path& plugin_store_root) {
    UnicodePaletteResult fallback;
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
                      "ui.unicode_palette") == installed.manifest.permissions.end()) continue;
        auto loaded = load_unicode_palette_resource(
            installed.installed_path / std::filesystem::path(installed.manifest.entry));
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

#include "owo/plugin/unicode_palette.h"

#include <filesystem>
#include <fstream>
#include <string>

namespace {

bool write_file(const std::filesystem::path& path, const std::string& bytes) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
    return static_cast<bool>(output);
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) return 1;
    const std::filesystem::path root(argv[1]);
    std::error_code error;
    std::filesystem::remove_all(root, error);
    std::filesystem::create_directories(root, error);
    if (error) return 2;
    const auto valid = root / L"valid.palette";
    if (!write_file(valid,
            "schema_version=1\ncategory=common|Common\n"
            "item=2605|Black star\nitem=1F642|Slight smile\n")) return 3;
    const auto loaded = owo::plugin::load_unicode_palette_resource(valid);
    if (!loaded.active || loaded.palette.categories.size() != 1 ||
        loaded.palette.categories.front().items.size() != 2 ||
        loaded.palette.categories.front().items[1].codepoint != 0x1f642) return 4;
    const auto surrogate = root / L"surrogate.palette";
    if (!write_file(surrogate,
            "schema_version=1\ncategory=bad|Bad\nitem=D800|Invalid\n") ||
        owo::plugin::load_unicode_palette_resource(surrogate).active) return 5;
    const auto empty = root / L"empty.palette";
    if (!write_file(empty, "schema_version=1\ncategory=empty|Empty\n") ||
        owo::plugin::load_unicode_palette_resource(empty).active) return 6;
    std::filesystem::remove_all(root, error);
    return 0;
}

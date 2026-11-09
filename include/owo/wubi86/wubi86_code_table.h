#pragma once
#include "wubi86_dictionary.h"
#include <map>
#include <span>
namespace owo::wubi86 {
struct RootKey { char key; int position; std::string_view key_name; std::string_view roots; };
// Font-specific source_slot must never be interpreted as a Unicode root character.
struct RootGlyph { std::string_view id; char key; std::string_view source_slot; std::string_view svg_path; };
class Wubi86CodeTable {
public:
 explicit Wubi86CodeTable(const Wubi86Dictionary& dictionary);
 [[nodiscard]] std::vector<std::string> full_codes(std::string_view character) const;
 [[nodiscard]] std::vector<std::string> short_codes(std::string_view character) const;
 // Every source-chart shape, including non-Unicode variants, has a stable ID and exact outline.
 [[nodiscard]] static std::span<const RootGlyph> root_glyphs();
 [[nodiscard]] static std::optional<RootGlyph> root_glyph(std::string_view id);
 // Convenience lookup for unambiguous ordinary Unicode roots.
 [[nodiscard]] static std::span<const RootKey> keys();
 [[nodiscard]] static std::optional<char> root_key(std::string_view root);
private:
 std::map<std::string,std::vector<std::string>,std::less<>> full_, short_;
};
}

#pragma once
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>
namespace owo::wubi86 {
enum class CodeKind { full, short_code, unclassified, phrase };
struct Entry { std::string code; std::string text; std::uint32_t frequency{}; CodeKind kind{CodeKind::unclassified}; };
struct LoadResult { bool ok{}; std::string error; std::size_t line{}; std::size_t duplicates{}; std::uint64_t parse_us{}, index_us{}; };
struct EncodeResult { bool ok{}; std::vector<std::string> codes; std::string error; };
[[nodiscard]] bool valid_code(std::string_view code) noexcept;
// Rejects overlong UTF-8, surrogates, out-of-range scalars, NUL and control characters.
[[nodiscard]] std::optional<std::size_t> utf8_scalar_count(std::string_view text);
[[nodiscard]] std::optional<std::vector<std::string>> split_utf8(std::string_view text);
}

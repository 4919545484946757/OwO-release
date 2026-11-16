#pragma once

#include <algorithm>
#include <cstdint>
#include <iterator>
#include <string_view>
#include <utility>

namespace owo::engine::syntax_collocations {

enum class Relation : std::uint8_t { subject, object, modifier, compound, complement, classifier };
struct Record {
    std::string_view left;
    std::string_view right;
    Relation relation;
    std::uint16_t count;
};

inline constexpr Record kRecords[]{
#include "../../../data/syntax/zh-gsdsimp-r2.17.inc"
};
inline constexpr std::int64_t kMaximumPairBonus = 360;
inline constexpr std::int64_t kMaximumPathBonus = 900;
inline constexpr std::uint32_t kMaximumQueries = 4096;

[[nodiscard]] inline std::int64_t score(std::string_view left, std::string_view right) noexcept {
    const auto key = std::pair{left, right};
    const auto found = std::lower_bound(std::begin(kRecords), std::end(kRecords), key,
        [](const Record& record, const auto& value) {
            return std::pair{record.left, record.right} < value;
        });
    if (found == std::end(kRecords) || found->left != left || found->right != right) return 0;
    // Subject/classifier edges are less diagnostic than lexical complements
    // and objects. Damp corpus frequency so common constructions stay a prior.
    const std::int64_t base = found->relation == Relation::subject ||
        found->relation == Relation::classifier ? 72 : 112;
    return std::min(kMaximumPairBonus, base + std::min<std::int64_t>(found->count, 10) * 24);
}

}  // namespace owo::engine::syntax_collocations

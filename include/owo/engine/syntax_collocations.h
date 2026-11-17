#pragma once

#include <algorithm>
#include <array>
#include <cstdint>
#include <iterator>
#include <span>
#include <string_view>
#include <utility>

namespace owo::engine::syntax_collocations {

enum class Relation : std::uint8_t { subject, object, adnominal, adverbial, compound, complement, classifier };
struct Record {
    std::string_view left;
    std::string_view right;
    Relation relation;
    bool head_on_right;
    std::uint16_t adjacent;
    std::uint16_t separated;
};

inline constexpr Record kRecords[]{
#include "../../../data/syntax/zh-gsdsimp-r2.17.inc"
};
inline constexpr std::int64_t kMaximumPairBonus = 360;
inline constexpr std::int64_t kMaximumPathBonus = 900;
inline constexpr std::uint32_t kMaximumQueries = 4096;

enum class Role : std::uint8_t {
    none = 0, predicate_support = 1, aspect = 2, quantity = 4, adjective = 8,
    nominal = 16, link = 32, verb = 64, boundary = 128
};
constexpr Role operator|(const Role left, const Role right) noexcept {
    return static_cast<Role>(static_cast<unsigned>(left) | static_cast<unsigned>(right));
}
constexpr bool has(const Role given, const Role wanted) noexcept {
    return (static_cast<unsigned>(given) & static_cast<unsigned>(wanted)) != 0;
}

struct RoleRecord { std::string_view word; Role role; };
inline constexpr RoleRecord kRoles[]{
#include "../../../data/syntax/zh-gsdsimp-r2.17-roles.inc"
};

[[nodiscard]] inline Role atomic_role(const std::string_view word) noexcept {
    const auto found = std::lower_bound(std::begin(kRoles), std::end(kRoles), word,
        [](const RoleRecord& record, const auto& sought) { return record.word < sought; });
    return found != std::end(kRoles) && found->word == word ? found->role : Role::none;
}

[[nodiscard]] inline Role classify(const std::string_view word) noexcept {
    const auto role = atomic_role(word);
    // A decoder dictionary can store a determiner+classifier as one word even
    // when UD annotated it as two. Only compose quantity chunks, not arbitrary POS.
    if (role == Role::none || role == Role::nominal) {
        for (std::size_t split = 3; split < word.size(); split += 3) {
            if (has(atomic_role(word.substr(0, split)), Role::quantity) &&
                has(atomic_role(word.substr(split)), Role::quantity)) return role | Role::quantity;
        }
    }
    return role;
}

using RelationMask = std::uint8_t;
constexpr RelationMask flag(const Relation relation) noexcept {
    return static_cast<RelationMask>(1U << static_cast<unsigned>(relation));
}
inline constexpr RelationMask kAllRelations = 0x7f;

// Accept intervening words only if they can be assigned to the construction.
// For V–O parse [aspect particles] [quantifier/modifier phrases]; a
// bare noun or verb inside this gap may only be a modifier if followed by 的.
[[nodiscard]] inline RelationMask gap_relations(const std::span<const Role> roles) noexcept {
    if (roles.empty()) return kAllRelations;
    bool subject = true;
    bool adnominal = has(roles.front(), Role::link);
    bool before_np = true, within_np = false, needs_link = false;
    for (const auto role : roles) {
        if (role == Role::none || has(role, Role::boundary)) return 0;
        subject &= has(role, Role::predicate_support);
        const auto next_before_np = before_np && has(role, Role::aspect);
        const auto next_within_np = ((before_np || within_np) && has(role, Role::quantity | Role::adjective)) ||
            (needs_link && has(role, Role::link));
        needs_link = (before_np || within_np) && has(role, Role::nominal | Role::adjective | Role::verb);
        before_np = next_before_np;
        within_np = next_within_np;
    }
    for (const auto role : roles.subspan(1)) adnominal &= has(role, Role::quantity);
    return static_cast<RelationMask>((subject ? flag(Relation::subject) : RelationMask{}) |
        (before_np || within_np ? flag(Relation::object) : RelationMask{}) |
        (adnominal ? flag(Relation::adnominal) : RelationMask{}));
}

struct Match { std::int64_t bonus{}; Relation relation{}; std::uint8_t gap{}; };

[[nodiscard]] inline Match dependency(std::string_view left, std::string_view right,
                                     const RelationMask allowed, const std::uint8_t gap) noexcept {
    const auto key = std::pair{left, right};
    auto found = std::lower_bound(std::begin(kRecords), std::end(kRecords), key,
        [](const Record& record, const auto& value) {
            return std::pair{record.left, record.right} < value;
        });
    Match best{};
    for (; found != std::end(kRecords) && found->left == left && found->right == right; ++found) {
        const auto relation = found->relation;
        if (!(allowed & flag(relation))) continue;
        if (gap != 0 && found->head_on_right != (relation != Relation::object)) continue;
        const std::int64_t base = relation == Relation::subject || relation == Relation::classifier ? 72 : 112;
        auto bonus = base + std::min<std::int64_t>(found->adjacent + found->separated, 10) * 24;
        if (gap != 0) bonus -= (found->separated ? 12 : 48) + 12 * (gap - 1);
        if (bonus > best.bonus) best = {std::min(kMaximumPairBonus, bonus), relation, gap};
    }
    return best;
}

// Three intervening decoder words. Store only byte widths and roles: views are
// reconstructed from the owning beam state's text and never outlive that state.
inline constexpr std::size_t kWindow = 4;
struct Token { std::uint8_t bytes{}; Role role{}; };
struct History {
    std::array<Token, kWindow> tokens{};
    void push(const std::size_t bytes, const Role role) noexcept {
        if (bytes < 3 || bytes > 12 || bytes % 3 != 0 || has(role, Role::boundary)) { tokens = {}; return; }
        for (auto i = kWindow - 1; i > 0; --i) tokens[i] = tokens[i - 1];
        tokens[0] = {static_cast<std::uint8_t>(bytes), role};
    }
};

struct Anchor { std::string_view word; RelationMask allowed{}; Role role{}; };
using Context = std::array<Anchor, kWindow>;

[[nodiscard]] inline Context context(std::string_view text, const History& history) noexcept {
    Context result{};
    std::array<Role, kWindow - 1> gap{};
    for (std::size_t distance = 0; distance < kWindow; ++distance) {
        const auto [bytes, role] = history.tokens[distance];
        if (bytes == 0 || bytes > text.size()) break;
        result[distance] = {text.substr(text.size() - bytes), gap_relations(std::span(gap).last(distance)), role};
        text.remove_suffix(bytes);
        if (distance == kWindow - 1 || role == Role::none) break;
        gap[kWindow - 2 - distance] = role;
    }
    return result;
}

[[nodiscard]] inline Match score(const Context& context, const std::string_view current,
                                 std::uint32_t& queries) noexcept {
    Match best{};
    for (std::uint8_t gap = 0; gap < kWindow; ++gap) {
        if (context[gap].word.empty() || queries == kMaximumQueries) break;
        if (!context[gap].allowed) continue;
        ++queries;
        const auto candidate = dependency(context[gap].word, current, context[gap].allowed, gap);
        if (candidate.bonus > best.bonus) best = candidate;
    }
    return best;
}

}  // namespace owo::engine::syntax_collocations

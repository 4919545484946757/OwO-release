#include "owo/engine/indexed_lexicon.h"
#include "lexicon_match.h"
#include <algorithm>
#include <iterator>
#include <set>
#include <stdexcept>
#include <unordered_set>

namespace owo::engine {
std::span<const std::uint16_t> IndexedLexicon::reading(const Row& row) const { return {ids_.data() + row.reading, row.count}; }
std::string_view IndexedLexicon::text(const Row& row) const { return {pool_.data() + row.text, row.bytes}; }
LexiconEntry IndexedLexicon::materialize(const Row& row) const {
    LexiconEntry result{{}, std::string(text(row)), row.frequency};
    for (auto id : reading(row)) result.syllables.push_back(syllables_[id]);
    return result;
}
void IndexedLexicon::add(std::span<const std::string> syllables, std::string_view word, std::uint32_t frequency) {
    if (syllables.empty()) return;
    for (const auto& syllable : syllables)
        if (syllable.empty() || syllable.size() > 6 || syllable.find_first_not_of("abcdefghijklmnopqrstuvwxyz") != std::string::npos)
            throw std::invalid_argument("invalid indexed syllable");
    if (syllables.size() > 256 || word.size() > 1024 || rows_.size() >= 2000000 ||
        ids_.size() + syllables.size() > 128 * 1024 * 1024 || pool_.size() + word.size() > 1024ULL * 1024 * 1024)
        throw std::length_error("imported phonetic index exceeds limits");
    Row row{static_cast<std::uint32_t>(ids_.size()), static_cast<std::uint32_t>(pool_.size()), frequency,
            static_cast<std::uint16_t>(syllables.size()), static_cast<std::uint16_t>(word.size())};
    for (const auto& syllable : syllables) {
        auto [it, inserted] = syllable_ids_.try_emplace(syllable, static_cast<std::uint16_t>(syllables_.size()));
        if (inserted) {
            if (syllables_.size() >= 65535) throw std::length_error("too many indexed syllables");
            syllables_.push_back(syllable);
        }
        ids_.push_back(it->second);
    }
    pool_.append(word); rows_.push_back(row); maximum_ = std::max(maximum_, syllables.size());
}
void IndexedLexicon::finish() {
    std::sort(rows_.begin(), rows_.end(), [&](const Row& a, const Row& b) {
        auto left = reading(a), right = reading(b);
        if (!std::ranges::equal(left, right)) return std::lexicographical_compare(left.begin(), left.end(), right.begin(), right.end());
        if (text(a) != text(b)) return text(a) < text(b);
        return a.frequency > b.frequency;
    });
    rows_.erase(std::unique(rows_.begin(), rows_.end(), [&](const Row& a, const Row& b) {
        return std::ranges::equal(reading(a), reading(b)) && text(a) == text(b);
    }), rows_.end());
    std::sort(rows_.begin(), rows_.end(), [&](const Row& a, const Row& b) {
        const auto left = reading(a), right = reading(b);
        if (!std::ranges::equal(left, right)) return std::lexicographical_compare(left.begin(), left.end(), right.begin(), right.end());
        if (a.frequency != b.frequency) return a.frequency > b.frequency;
        return text(a) < text(b);
    });
    for (auto& bucket : initials_) bucket.clear(); mixed_.clear();
    for (std::uint32_t i = 0; i < rows_.size(); ++i) {
        const auto ids = reading(rows_[i]);
        if (ids.size() == 1) initials_[syllables_[ids[0]][0] - 'a'].push_back(i);
        else mixed_[(std::uint32_t(ids[0]) << 8) | static_cast<unsigned char>(syllables_[ids[1]][0])].push_back(i);
    }
    const auto frequent = [&](auto a, auto b) {
        if (rows_[a].frequency != rows_[b].frequency) return rows_[a].frequency > rows_[b].frequency;
        if (rows_[a].count != rows_[b].count) return rows_[a].count < rows_[b].count;
        return text(rows_[a]) < text(rows_[b]);
    };
    for (auto& bucket : initials_) std::sort(bucket.begin(), bucket.end(), frequent);
    for (auto& [key, bucket] : mixed_) std::sort(bucket.begin(), bucket.end(), frequent);
}
std::pair<std::size_t, std::size_t> IndexedLexicon::exact_range(std::span<const std::string_view> syllables) const {
    std::vector<std::uint16_t> key;
    for (auto syllable : syllables) {
        const auto it = syllable_ids_.find(std::string(syllable));
        if (it == syllable_ids_.end()) return {0, 0};
        key.push_back(it->second);
    }
    if (key.empty()) return {0, 0};
    auto it = std::lower_bound(rows_.begin(), rows_.end(), key, [&](const Row& row, const auto& value) {
        const auto ids = reading(row); return std::lexicographical_compare(ids.begin(), ids.end(), value.begin(), value.end());
    });
    const auto end = std::upper_bound(it, rows_.end(), key, [&](const auto& value, const Row& row) {
        const auto ids = reading(row); return std::lexicographical_compare(value.begin(), value.end(), ids.begin(), ids.end());
    });
    return {static_cast<std::size_t>(it - rows_.begin()), static_cast<std::size_t>(end - rows_.begin())};
}
std::vector<LexiconTextEntry> IndexedLexicon::lookup_texts(std::span<const std::string_view> syllables) const {
    const auto [begin, end] = exact_range(syllables);
    std::vector<LexiconTextEntry> result; result.reserve(end - begin);
    for (auto index = begin; index < end; ++index) result.push_back({text(rows_[index]), rows_[index].frequency});
    return result;
}
std::vector<LexiconEntry> IndexedLexicon::lookup(std::span<const std::string_view> syllables) const {
    std::vector<LexiconEntry> result;
    for (auto word : lookup_texts(syllables)) {
        LexiconEntry entry{{}, std::string(word.text), word.frequency};
        for (auto syllable : syllables) entry.syllables.emplace_back(syllable);
        result.push_back(std::move(entry));
    }
    return result;
}
std::uint32_t IndexedLexicon::lookup_max_frequency(std::span<const std::string_view> syllables) const {
    const auto [begin, end] = exact_range(syllables);
    return begin == end ? 0 : rows_[begin].frequency;
}
std::vector<InitialLexiconTextEntry> IndexedLexicon::lookup_initial_texts(char initial, std::size_t limit) const {
    if (initial < 'a' || initial > 'z') return {};
    std::vector<InitialLexiconTextEntry> result;
    for (auto i : initials_[initial - 'a']) {
        if (result.size() >= limit) break;
        const auto& row = rows_[i]; result.push_back({syllables_[reading(row)[0]], text(row), row.frequency});
    }
    return result;
}
std::vector<LexiconEntry> IndexedLexicon::lookup_initial(char initial, std::size_t limit) const {
    if (initial < 'a' || initial > 'z') return {};
    std::vector<LexiconEntry> result;
    for (auto i : initials_[initial - 'a']) { if (result.size() >= limit) break; result.push_back(materialize(rows_[i])); }
    return result;
}
std::vector<AbbreviatedLexiconTextMatch> IndexedLexicon::lookup_mixed_abbreviation_texts(std::string_view input, std::size_t limit) const {
    if (input.size() < 2 || input.size() > 256 || !limit || input.find('\'') != std::string_view::npos) return {};
    std::vector<AbbreviatedLexiconTextMatch> result;
    std::unordered_set<std::uint32_t> visited;
    constexpr std::size_t work_limit = 16384;
    for (std::size_t length = 1; length <= std::min<std::size_t>(6, input.size() - 1); ++length) {
        for (std::size_t id = 0; id < syllables_.size(); ++id) {
            if (!std::string_view(syllables_[id]).starts_with(input.substr(0, length))) continue;
            const auto bucket = mixed_.find((std::uint32_t(id) << 8) | static_cast<unsigned char>(input[length]));
            if (bucket == mixed_.end()) continue;
            for (auto i : bucket->second) {
                if (visited.size() >= work_limit) break;
                if (!visited.insert(i).second) continue;
                const auto& row = rows_[i];
                std::vector<std::string_view> syllables;
                for (auto syllable : reading(row)) syllables.push_back(syllables_[syllable]);
                auto segments = detail::mixed_abbreviation_segments(syllables, input);
                if (!segments) continue;
                result.push_back({std::move(syllables), text(row), row.frequency, std::move(*segments)});
                if (result.size() >= std::max<std::size_t>(64, std::min<std::size_t>(limit, 16384) * 4))
                    detail::retain_best_mixed_matches(result, limit, input.size() == 2 && input.find_first_of("aeiouv") == std::string_view::npos);
            }
        }
    }
    detail::retain_best_mixed_matches(result, limit, input.size() == 2 && input.find_first_of("aeiouv") == std::string_view::npos);
    return result;
}
std::vector<AbbreviatedLexiconMatch> IndexedLexicon::lookup_mixed_abbreviation(std::string_view input, std::size_t limit) const {
    std::vector<AbbreviatedLexiconMatch> result;
    for (auto& match : lookup_mixed_abbreviation_texts(input, limit)) {
        LexiconEntry word{{}, std::string(match.text), match.frequency};
        for (auto syllable : match.syllables) word.syllables.emplace_back(syllable);
        result.push_back({std::move(word), std::move(match.source_segments)});
    }
    return result;
}
namespace {
template<class T> void append_unique(std::vector<T>& result, std::vector<T> extra) {
    // Each exact query has one reading; merge duplicate text using the higher
    // weight rather than adding independent dictionary frequencies together.
    std::unordered_map<std::string_view, std::size_t> seen;
    for (std::size_t i = 0; i < result.size(); ++i) seen.emplace(result[i].text, i);
    for (auto& entry : extra) {
        const auto found = seen.find(entry.text);
        if (found != seen.end()) result[found->second].frequency = std::max(result[found->second].frequency, entry.frequency);
        else { seen.emplace(entry.text, result.size()); result.push_back(std::move(entry)); }
    }
}
}
std::vector<LexiconTextEntry> CombinedLexicon::lookup_texts(std::span<const std::string_view> syllables) const {
    auto result = base_.lookup_texts(syllables); append_unique(result, imported_.lookup_texts(syllables)); return result;
}
std::vector<LexiconEntry> CombinedLexicon::lookup(std::span<const std::string_view> syllables) const {
    auto result = base_.lookup(syllables);
    for (auto& entry : imported_.lookup(syllables)) {
        auto it = std::find_if(result.begin(), result.end(), [&](const auto& word) { return word.text == entry.text; });
        if (it == result.end()) result.push_back(std::move(entry)); else it->frequency = std::max(it->frequency, entry.frequency);
    }
    return result;
}
std::uint32_t CombinedLexicon::lookup_max_frequency(std::span<const std::string_view> syllables) const { return std::max(base_.lookup_max_frequency(syllables), imported_.lookup_max_frequency(syllables)); }
std::size_t CombinedLexicon::maximum_reading_length() const noexcept { return std::max(base_.maximum_reading_length(), imported_.maximum_reading_length()); }
std::vector<InitialLexiconTextEntry> CombinedLexicon::lookup_initial_texts(char initial, std::size_t limit) const {
    auto result = base_.lookup_initial_texts(initial, limit);
    auto extra = imported_.lookup_initial_texts(initial, limit); result.insert(result.end(), extra.begin(), extra.end());
    std::sort(result.begin(), result.end(), [](const auto& a, const auto& b) {
        if (a.frequency != b.frequency) return a.frequency > b.frequency;
        if (a.syllable != b.syllable) return a.syllable < b.syllable; return a.text < b.text;
    });
    std::set<std::pair<std::string_view, std::string_view>> seen;
    std::erase_if(result, [&](const auto& row) { return !seen.emplace(row.syllable, row.text).second; });
    if (result.size() > limit) result.resize(limit); return result;
}
std::vector<LexiconEntry> CombinedLexicon::lookup_initial(char initial, std::size_t limit) const {
    std::vector<LexiconEntry> result;
    for (auto row : lookup_initial_texts(initial, limit)) result.push_back({{std::string(row.syllable)}, std::string(row.text), row.frequency});
    return result;
}
std::vector<AbbreviatedLexiconTextMatch> CombinedLexicon::lookup_mixed_abbreviation_texts(std::string_view input, std::size_t limit) const {
    auto result = base_.lookup_mixed_abbreviation_texts(input, limit);
    auto extra = imported_.lookup_mixed_abbreviation_texts(input, limit);
    result.insert(result.end(), std::make_move_iterator(extra.begin()), std::make_move_iterator(extra.end()));
    // Different readings of the same text remain separate candidates.
    std::sort(result.begin(), result.end(), [](const auto& a, const auto& b) {
        if (a.syllables != b.syllables) return a.syllables < b.syllables;
        if (a.text != b.text) return a.text < b.text; return a.frequency > b.frequency;
    });
    result.erase(std::unique(result.begin(), result.end(), [](const auto& a, const auto& b) { return a.syllables == b.syllables && a.text == b.text; }), result.end());
    detail::retain_best_mixed_matches(result, limit, input.size() == 2 && input.find_first_of("aeiouv") == std::string_view::npos); return result;
}
std::vector<AbbreviatedLexiconMatch> CombinedLexicon::lookup_mixed_abbreviation(std::string_view input, std::size_t limit) const {
    std::vector<AbbreviatedLexiconMatch> result;
    for (auto& row : lookup_mixed_abbreviation_texts(input, limit)) {
        LexiconEntry entry{{}, std::string(row.text), row.frequency}; for (auto syllable : row.syllables) entry.syllables.emplace_back(syllable);
        result.push_back({std::move(entry), std::move(row.source_segments)});
    }
    return result;
}
}

#include "owo/engine/candidate_generator.h"
#include "owo/engine/user_lexicon_alignment.h"
#include "owo/engine/candidate_scoring.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <chrono>
#include <deque>
#include <iterator>
#include <limits>
#include <optional>
#include <string_view>
#include <unordered_map>
#include <unordered_set>

namespace owo::engine {
namespace {

struct SearchState {
    std::string text;
    std::string previous;
    std::vector<std::string> syllables;
    std::size_t segment_count{};
    std::int64_t score{};
    std::vector<std::size_t> segment_lengths;
};

struct OrderedParsePath {
    const ParsePath* path{};
    std::uint32_t full_span_frequency{};
    std::size_t phrase_covered_syllables{};
    std::uint64_t phrase_frequency_evidence{};
};

// Raw corpus frequencies are attached to entries of different lengths. Without
// a token transition cost, summing several high-frequency characters always
// overwhelms a valid multi-syllable word and rewards pathological over-segmentation.
constexpr std::int64_t kAdditionalSegmentPenalty = kCandidateAdditionalSegmentPenalty;
constexpr std::int64_t kIncompleteCharacterPenalty =
    kCandidateIncompleteCharacterPenalty;
constexpr std::int64_t kAbbreviationBasePenalty =
    kCandidateAbbreviationBasePenalty;
constexpr std::int64_t kMixedAbbreviationPhraseBonus = 8'000;
constexpr std::int64_t kPreferredInitialBonus = 6'000;
constexpr std::int64_t kCommonInitialSeedBonus = 100'000;
constexpr std::int64_t kMaximumBoundaryCoherenceBonus = 12'000;
constexpr std::int64_t kUncoveredBoundaryCharacterPenalty = 12'000;
constexpr std::int64_t kPreferredExactSegmentationBonus =
    kCandidatePreferredExactSegmentationBonus;
constexpr std::int64_t kAlternativeSegmentationPromotionMargin = 16'000;
constexpr std::int64_t kHighConfidenceAlternativeScore = 12'000;

// A request-local snapshot: nothing survives a dictionary/config/user update.
// Keep full lookup rows; beam-specific trimming must never poison a later
// prefix/coherence lookup needing more entries from the same reading.
class RequestLexiconCache {
    using Reading = std::vector<std::string>;
    struct ReadingHash {
        using is_transparent = void;
        template<class R> std::size_t operator()(const R& reading) const {
            std::size_t hash = reading.size();
            for (const auto& syllable : reading)
                hash ^= std::hash<std::string_view>{}(syllable) +
                    std::size_t{0x9e3779b9} + (hash << 6) + (hash >> 2);
            return hash;
        }
    };
    struct ReadingEqual {
        using is_transparent = void;
        template<class L, class R> bool operator()(const L& left, const R& right) const {
            return left.size() == right.size() &&
                std::equal(left.begin(), left.end(), right.begin());
        }
    };
    struct PhraseKey { Reading reading; std::string text; };
    struct PhraseView { std::span<const std::string_view> reading; std::string_view text; };
    struct PhraseHash {
        using is_transparent = void;
        template<class P> std::size_t operator()(const P& phrase) const {
            return ReadingHash{}(phrase.reading) ^ (std::hash<std::string_view>{}(phrase.text) << 1);
        }
    };
    struct PhraseEqual {
        using is_transparent = void;
        template<class L, class R> bool operator()(const L& left, const R& right) const {
            return left.text == right.text && ReadingEqual{}(left.reading, right.reading);
        }
    };
public:
    RequestLexiconCache(const Lexicon& lexicon, CandidateGenerationMetrics* metrics)
        : lexicon_(lexicon), metrics_(metrics) {}

    const std::vector<LexiconTextEntry>& lookup(std::span<const std::string_view> reading) {
        auto found = rows_.find(reading);
        if (found != rows_.end()) {
            if (metrics_) ++metrics_->exact_lookup_cache_hits;
            return found->second;
        }
        const auto started = std::chrono::steady_clock::now();
        auto entries = lexicon_.lookup_texts(reading);
        record_lookup(started);
        if (rows_.size() < 1024 && cached_entries_ + entries.size() <= 32768) {
            cached_entries_ += entries.size();
            return rows_.emplace(own_reading(reading), std::move(entries)).first->second;
        }
        scratch_ = std::move(entries);
        return scratch_;
    }

    std::uint32_t maximum(std::span<const std::string_view> reading) {
        if (const auto found = maxima_.find(reading); found != maxima_.end()) {
            if (metrics_) ++metrics_->exact_lookup_cache_hits;
            return found->second;
        }
        const auto started = std::chrono::steady_clock::now();
        const auto value = lexicon_.lookup_max_frequency(reading);
        record_lookup(started);
        if (maxima_.size() < 1024) maxima_.emplace(own_reading(reading), value);
        return value;
    }

    std::optional<std::int64_t> phrase_score(std::span<const std::string_view> reading,
                                           std::string_view text) {
        const PhraseView key{reading, text};
        if (const auto found = phrases_.find(key); found != phrases_.end()) {
            if (metrics_) ++metrics_->coherence_cache_hits;
            return found->second;
        }
        std::optional<std::int64_t> score;
        for (const auto& entry : lookup(reading)) {
            if (entry.text != text) continue;
            const auto value = std::min(kMaximumBoundaryCoherenceBonus,
                candidate_unigram_score(entry.frequency) * 3 / 2);
            score = score ? std::max(*score, value) : value;
        }
        if (phrases_.size() < 4096)
            phrases_.emplace(PhraseKey{own_reading(reading), std::string(text)}, score);
        return score;
    }

private:
    static Reading own_reading(std::span<const std::string_view> reading) {
        Reading key;
        key.reserve(reading.size());
        for (const auto syllable : reading) key.emplace_back(syllable);
        return key;
    }
    void record_lookup(std::chrono::steady_clock::time_point started) {
        if (!metrics_) return;
        ++metrics_->lexicon_lookup_count;
        metrics_->lexicon_lookup_us += static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::microseconds>(
                std::chrono::steady_clock::now() - started).count());
    }
    const Lexicon& lexicon_;
    CandidateGenerationMetrics* metrics_;
    std::size_t cached_entries_{};
    std::unordered_map<Reading, std::vector<LexiconTextEntry>, ReadingHash, ReadingEqual> rows_;
    std::unordered_map<Reading, std::uint32_t, ReadingHash, ReadingEqual> maxima_;
    std::unordered_map<PhraseKey, std::optional<std::int64_t>, PhraseHash, PhraseEqual> phrases_;
    std::vector<LexiconTextEntry> scratch_;
};

std::int64_t common_initial_seed_bonus(const char initial,
                                       const std::string_view text) {
    // Raw dictionary counts mix several corpora and otherwise put 大/多 ahead
    // of the conventional high-utility d shortcuts. Keep a very small seed
    // list for the default single-initial experience; all remaining characters
    // still follow learned and dictionary frequency.
    if (initial != 'd') return 0;
    constexpr std::array<std::string_view, 6> seeds{
        "的", "都", "对", "等", "到", "但"};
    const auto found = std::find(seeds.begin(), seeds.end(), text);
    if (found == seeds.end()) return 0;
    return kCommonInitialSeedBonus -
           static_cast<std::int64_t>(found - seeds.begin()) * 10'000;
}

std::string_view preferred_initial_syllable(const char initial) {
    switch (initial) {
        case 'n': return "ni";
        case 'm': return "ma";
        default: return {};
    }
}

std::int64_t source_initial_bonus(const ParseResult& parsed, const ParsePath& path) {
    if (!parsed.phonetic_assistance) return 0;
    std::int64_t bonus = 0;
    for (const auto& syllable : path.syllables) {
        if (syllable.end != syllable.begin + 1 ||
            syllable.end > parsed.normalized_input.size()) continue;
        if (preferred_initial_syllable(parsed.normalized_input[syllable.begin]) ==
            syllable.text)
            bonus += kPreferredInitialBonus;
    }
    return bonus;
}

bool search_state_better(const SearchState& left, const SearchState& right) {
    if (left.score != right.score) return left.score > right.score;
    return left.text < right.text;
}

void prune(std::vector<SearchState>& states, const std::size_t width) {
    if (states.size() > width)
        std::partial_sort(states.begin(), states.begin() + width, states.end(),
                          search_state_better);
    else
        std::sort(states.begin(), states.end(), search_state_better);
    if (states.size() > width) states.resize(width);
}

// Future chart cells stay as worst-first heaps, bounded by the existing beam
// width. Reject losing transitions BEFORE copying strings and syllable vectors.
// A cell is sorted only when consumed, preserving score/text iteration order.
template <class Extend>
void append_successor(std::vector<SearchState>& destination, const std::size_t width,
                      const SearchState& source, const std::string_view text,
                      const std::int64_t score, const std::size_t extra_syllables, Extend&& extend,
                      CandidateGenerationMetrics* metrics) {
    if (metrics != nullptr) ++metrics->beam_transitions;
    if (destination.size() == width && score < destination.front().score) return;
    std::string joined_text;
    joined_text.reserve(source.text.size() + text.size());
    joined_text = source.text;
    joined_text += text;
    if (destination.size() == width && score == destination.front().score &&
        joined_text >= destination.front().text) return;
    SearchState next;
    next.text = std::move(joined_text);
    next.score = score;
    next.segment_count = source.segment_count;
    next.segment_lengths.reserve(source.segment_lengths.size() + 1);
    next.segment_lengths.insert(next.segment_lengths.end(),
        source.segment_lengths.begin(), source.segment_lengths.end());
    next.syllables.reserve(source.syllables.size() + extra_syllables);
    next.syllables.insert(next.syllables.end(), source.syllables.begin(), source.syllables.end());
    extend(next);
    if (metrics != nullptr) ++metrics->beam_materialized;
    if (destination.size() == width) {
        std::pop_heap(destination.begin(), destination.end(), search_state_better);
        destination.back() = std::move(next);
    } else {
        destination.push_back(std::move(next));
    }
    std::push_heap(destination.begin(), destination.end(), search_state_better);
}

void retain_frequent_entries(std::vector<LexiconEntry>& entries,
                             const std::size_t limit) {
    if (entries.size() <= limit) return;
    const auto better = [](const LexiconEntry& left, const LexiconEntry& right) {
        if (left.frequency != right.frequency) return left.frequency > right.frequency;
        return left.text < right.text;
    };
    std::nth_element(entries.begin(), entries.begin() + limit, entries.end(), better);
    entries.resize(limit);
    std::sort(entries.begin(), entries.end(), better);
}

void retain_frequent_entries(std::vector<LexiconTextEntry>& entries,
                             const std::size_t limit) {
    if (entries.size() <= limit) return;
    const auto better = [](const LexiconTextEntry& left,
                           const LexiconTextEntry& right) {
        if (left.frequency != right.frequency) return left.frequency > right.frequency;
        return left.text < right.text;
    };
    std::nth_element(entries.begin(), entries.begin() + limit, entries.end(), better);
    entries.resize(limit);
    std::sort(entries.begin(), entries.end(), better);
}

std::vector<std::string> own_syllables(
    const std::span<const std::string_view> reading) {
    std::vector<std::string> result;
    result.reserve(reading.size());
    for (const auto syllable : reading) result.emplace_back(syllable);
    return result;
}

std::size_t mixed_completion_characters(
    const std::span<const std::string_view> syllables,
    const std::span<const std::string> source_segments) {
    std::size_t result = 0;
    const auto shared = std::min(syllables.size(), source_segments.size());
    for (std::size_t index = 0; index < shared; ++index)
        result += syllables[index].size() - source_segments[index].size();
    return result;
}

std::vector<std::string> source_segments(const ParseResult& parsed,
                                         const ParsePath& path) {
    std::vector<std::string> segments;
    segments.reserve(path.syllables.size());
    for (const auto& syllable : path.syllables) {
        if (syllable.begin >= syllable.end ||
            syllable.end > parsed.normalized_input.size()) return {};
        segments.push_back(parsed.normalized_input.substr(
            syllable.begin, syllable.end - syllable.begin));
    }
    return segments;
}

std::string candidate_structure_key(const Candidate& candidate) {
    std::string key;
    for (const auto& segment : candidate.source_segments) {
        key += segment;
        key.push_back('\'');
    }
    key.push_back('#');
    for (const auto length : candidate.segment_lengths) {
        key += std::to_string(length);
        key.push_back(',');
    }
    return key;
}

bool candidate_better(const Candidate& candidate, const Candidate& existing) {
    if (candidate.consumed_input_bytes != existing.consumed_input_bytes)
        return candidate.consumed_input_bytes > existing.consumed_input_bytes;
    if (candidate.user_preferred() != existing.user_preferred())
        return candidate.user_preferred();
    // A remembered word remains a word after its one-off priority expires.
    // A native whole-word match can replace its stale duplicate below, but
    // an arbitrary composition must not erase the user-lexicon identity.
    if ((candidate.segment_count == 1) != (existing.segment_count == 1))
        return candidate.segment_count == 1;
    if (candidate.provisional_user_word != existing.provisional_user_word)
        return !candidate.provisional_user_word;
    if (candidate.score != existing.score) return candidate.score > existing.score;
    return candidate.match_kind < existing.match_kind;
}

void replace_candidate_payload(Candidate& existing, Candidate replacement) {
    // The accumulator's string_view key points at existing.text. Equal-text
    // candidates can replace every other field without allocating a second
    // owning hash key or invalidating that view.
    existing.syllables = std::move(replacement.syllables);
    existing.score = replacement.score;
    existing.match_kind = replacement.match_kind;
    existing.source_segments = std::move(replacement.source_segments);
    existing.consumed_input_bytes = replacement.consumed_input_bytes;
    existing.segment_count = replacement.segment_count;
    existing.coherence_score = replacement.coherence_score;
    existing.segment_lengths = std::move(replacement.segment_lengths);
    existing.model_only = replacement.model_only;
    existing.user_lexicon = replacement.user_lexicon;
    existing.provisional_user_word = replacement.provisional_user_word;
    existing.joint_phrase_partition = replacement.joint_phrase_partition;
}

class CandidateAccumulator {
public:
    explicit CandidateAccumulator(const std::size_t expected = 64) {
        index_.reserve(expected);
    }

    void store(Candidate candidate) {
        const auto found = index_.find(candidate.text);
        if (found != index_.end()) {
            if (candidate_better(candidate, *found->second))
                replace_candidate_payload(*found->second, std::move(candidate));
            return;
        }
        candidates_.push_back(std::move(candidate));
        auto& stored = candidates_.back();
        index_.emplace(stored.text, &stored);
    }

    [[nodiscard]] std::size_t size() const noexcept { return candidates_.size(); }

    [[nodiscard]] bool contains(const std::string_view text) const {
        return index_.contains(text);
    }

    [[nodiscard]] std::vector<Candidate> take() {
        std::vector<Candidate> output;
        output.reserve(candidates_.size());
        for (auto& candidate : candidates_)
            output.push_back(std::move(candidate));
        index_.clear();
        candidates_.clear();
        return output;
    }

private:
    std::deque<Candidate> candidates_;
    std::unordered_map<std::string_view, Candidate*> index_;
};

bool score_less(const Candidate& left, const Candidate& right) {
    if (left.user_preferred() != right.user_preferred())
        return left.user_preferred();
    if (left.score != right.score) return left.score > right.score;
    if (left.match_kind != right.match_kind) return left.match_kind < right.match_kind;
    if (left.syllables.size() != right.syllables.size())
        return left.syllables.size() < right.syllables.size();
    return left.text < right.text;
}

std::size_t utf8_character_count(const std::string_view text) {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](const unsigned char byte) {
            return (byte & 0xc0U) != 0x80U;
        }));
}

std::vector<std::size_t> utf8_boundaries(const std::string_view text) {
    std::vector<std::size_t> result{0};
    for (std::size_t offset = 1; offset < text.size(); ++offset) {
        if ((static_cast<unsigned char>(text[offset]) & 0xc0U) != 0x80U)
            result.push_back(offset);
    }
    result.push_back(text.size());
    return result;
}

std::size_t utf8_difference_count(const std::string_view left,
                                  const std::string_view right) {
    const auto character_bytes = [](const unsigned char lead) {
        if ((lead & 0x80U) == 0) return std::size_t{1};
        if ((lead & 0xe0U) == 0xc0U) return std::size_t{2};
        if ((lead & 0xf0U) == 0xe0U) return std::size_t{3};
        return std::size_t{4};
    };
    std::size_t left_offset = 0;
    std::size_t right_offset = 0;
    std::size_t differences = 0;
    while (left_offset < left.size() && right_offset < right.size()) {
        const auto left_size = character_bytes(
            static_cast<unsigned char>(left[left_offset]));
        const auto right_size = character_bytes(
            static_cast<unsigned char>(right[right_offset]));
        if (left.substr(left_offset, left_size) !=
            right.substr(right_offset, right_size))
            ++differences;
        left_offset += left_size;
        right_offset += right_size;
    }
    differences += utf8_character_count(left.substr(left_offset));
    differences += utf8_character_count(right.substr(right_offset));
    return differences;
}

std::string_view utf8_first_character(const std::string_view text) {
    if (text.empty()) return {};
    std::size_t end = 1;
    while (end < text.size() &&
           (static_cast<unsigned char>(text[end]) & 0xc0U) == 0x80U)
        ++end;
    return text.substr(0, end);
}

void prioritize_two_character_words(std::vector<Candidate>& candidates) {
    std::vector<std::size_t> positions;
    std::vector<Candidate> two_character;
    std::vector<Candidate> single_character;
    const bool preserve_leading_whole_input = !candidates.empty() &&
        utf8_character_count(candidates.front().text) == 2 &&
        std::all_of(candidates.begin() + 1, candidates.end(),
                    [&candidates](const Candidate& candidate) {
                        return candidate.consumed_input_bytes <=
                               candidates.front().consumed_input_bytes;
                    });
    const std::size_t begin = preserve_leading_whole_input ? 1 : 0;
    for (std::size_t index = begin; index < candidates.size(); ++index) {
        const auto characters = utf8_character_count(candidates[index].text);
        if (characters != 1 && characters != 2) continue;
        positions.push_back(index);
    }
    const auto two_count = static_cast<std::size_t>(std::count_if(
        positions.begin(), positions.end(), [&candidates](const std::size_t index) {
            return utf8_character_count(candidates[index].text) == 2;
        }));
    if (two_count == 0 || two_count == positions.size()) return;
    two_character.reserve(two_count);
    single_character.reserve(positions.size() - two_count);
    for (const auto index : positions) {
        if (utf8_character_count(candidates[index].text) == 2)
            two_character.push_back(std::move(candidates[index]));
        else
            single_character.push_back(std::move(candidates[index]));
    }
    std::sort(two_character.begin(), two_character.end(), score_less);
    std::sort(single_character.begin(), single_character.end(), score_less);

    constexpr std::int64_t kPreferredTwoCharacterMinimumScore = 5'000;
    constexpr std::size_t kMaximumPreferredTwoCharacterWords = 10;
    std::size_t preferred = 0;
    while (preferred < two_character.size() &&
           preferred < kMaximumPreferredTwoCharacterWords &&
           two_character[preferred].score >= kPreferredTwoCharacterMinimumScore)
        ++preferred;

    std::vector<Candidate> reordered;
    reordered.reserve(positions.size());
    for (std::size_t index = 0; index < preferred; ++index)
        reordered.push_back(std::move(two_character[index]));
    for (auto& candidate : single_character)
        reordered.push_back(std::move(candidate));
    for (std::size_t index = preferred; index < two_character.size(); ++index)
        reordered.push_back(std::move(two_character[index]));
    for (std::size_t index = 0; index < positions.size(); ++index)
        candidates[positions[index]] = std::move(reordered[index]);
}

void promote_alternative_exact_segmentation(std::vector<Candidate>& candidates,
                                            const std::size_t full_input_bytes) {
    if (candidates.size() <= 2 ||
        candidates.front().match_kind != InputMatchKind::exact) return;
    const auto preferred_segments = candidates.front().source_segments;
    // A composed fallback and a whole dictionary word use different scoring
    // scales: the former pays a segment transition penalty while the latter
    // does not.  Comparing their totals with a small relative margin buries a
    // strong alternative boundary such as kuan'gao behind every kuang'ao word.
    // Require a useful absolute composition score instead; only one best
    // alternative segmentation is promoted, so weak Cartesian products still
    // remain outside the early candidate slots.
    constexpr std::int64_t kPrimaryCompositionMinimumScore = 12'000;
    const auto alternative = std::find_if(
        candidates.begin() + 1, candidates.end(), [&](const Candidate& candidate) {
            if (candidate.match_kind != InputMatchKind::exact ||
                candidate.consumed_input_bytes != full_input_bytes ||
                candidate.source_segments == preferred_segments) return false;
            const bool dictionary_word = candidate.segment_count == 1 &&
                candidate.score >= kHighConfidenceAlternativeScore &&
                candidate.score + kAlternativeSegmentationPromotionMargin >=
                    candidates.front().score;
            const bool primary_character_composition =
                candidate.segment_count == candidate.syllables.size() &&
                utf8_character_count(candidate.text) == candidate.syllables.size() &&
                candidate.score >= kPrimaryCompositionMinimumScore;
            return dictionary_word || primary_character_composition;
        });
    const auto destination = candidates.begin() +
        static_cast<std::ptrdiff_t>(std::min<std::size_t>(2, candidates.size() - 1));
    if (alternative != candidates.end() && alternative > destination)
        std::rotate(destination, alternative, alternative + 1);
}

void promote_full_candidate_heads(const std::vector<Candidate>& sentences,
                                  const std::vector<Candidate>& dictionary,
                                  const std::size_t full_input_bytes,
                                  std::vector<Candidate>& single_characters) {
    if (single_characters.size() < 2) return;
    std::vector<std::string> preferred;
    preferred.reserve(2);
    const auto collect = [&](const std::vector<Candidate>& source) {
        for (const auto& candidate : source) {
            if (preferred.size() == 2) break;
            if (candidate.consumed_input_bytes != full_input_bytes) continue;
            const auto head = std::string(utf8_first_character(candidate.text));
            if (head.empty() || std::find(preferred.begin(), preferred.end(), head) !=
                                    preferred.end()) continue;
            preferred.push_back(head);
        }
    };
    collect(sentences);
    collect(dictionary);
    if (preferred.empty()) return;
    std::vector<Candidate> promoted;
    promoted.reserve(single_characters.size());
    std::vector<bool> used(single_characters.size());
    // Explicit source-bound learning outranks a sentence/dictionary head hint.
    // Keep its score order; the structural hint only reorders the remainder.
    for (std::size_t index = 0; index < single_characters.size(); ++index) {
        if (!single_characters[index].user_preferred()) continue;
        used[index] = true;
        promoted.push_back(std::move(single_characters[index]));
    }
    for (const auto& head : preferred) {
        const auto found = std::find_if(
            single_characters.begin(), single_characters.end(),
            [&](const Candidate& candidate) { return candidate.text == head; });
        if (found == single_characters.end()) continue;
        const auto index = static_cast<std::size_t>(found - single_characters.begin());
        if (used[index]) continue;
        used[index] = true;
        promoted.push_back(std::move(*found));
    }
    for (std::size_t index = 0; index < single_characters.size(); ++index)
        if (!used[index]) promoted.push_back(std::move(single_characters[index]));
    single_characters = std::move(promoted);
}

void limit_two_syllable_full_candidates(std::vector<Candidate>& candidates) {
    const auto is_composed = [](const Candidate& candidate) {
        return candidate.syllables.size() == 2 && candidate.segment_count > 1;
    };
    const bool has_dictionary_word = std::any_of(
        candidates.begin(), candidates.end(), [](const Candidate& candidate) {
            return candidate.syllables.size() == 2 && candidate.segment_count == 1;
        });
    constexpr std::size_t kMaximumTwoSyllableDictionaryWords = 2;
    std::size_t dictionary_words_kept = 0;
    bool fallback_kept = false;
    candidates.erase(std::remove_if(
        candidates.begin(), candidates.end(), [&](const Candidate& candidate) {
            if (candidate.syllables.size() == 2 && candidate.segment_count == 1) {
                if (dictionary_words_kept >= kMaximumTwoSyllableDictionaryWords)
                    return true;
                ++dictionary_words_kept;
                return false;
            }
            if (!is_composed(candidate)) return false;
            if (has_dictionary_word || fallback_kept) return true;
            fallback_kept = true;
            return false;
        }), candidates.end());
}

}  // namespace

std::vector<Candidate> CandidateGenerator::generate(const ParseResult& parsed,
                                                    const std::size_t limit,
                                                    const bool contextual_ranking,
                                                    const std::string_view language_context,
                                                    CandidateGenerationMetrics* const metrics,
                                                    const std::function<bool()>& cancelled,
                                                    const bool include_model_alternatives,
                                                    const std::size_t partition_page_size,
                                                    const bool user_completion_enabled,
                                                    const std::size_t user_completion_max_syllables) const {
    if (!parsed.valid || limit == 0 || (cancelled && cancelled())) return {};
    // A normal first page needs only page_size + one look-ahead candidate.
    // Keeping a small quality margin avoids the former 32-result floor, which
    // expanded every beam to at least 128 states even when only five results
    // were visible.
    constexpr std::size_t kMinimumSearchLimit = 12;
    const auto search_limit = std::max(kMinimumSearchLimit, limit);
    RequestLexiconCache lookup_cache(lexicon_, metrics);
    const auto apply_learned_preference = [this, &parsed](Candidate& candidate) {
        if (user_frequency_ == nullptr || candidate.consumed_input_bytes == 0 ||
            candidate.consumed_input_bytes > parsed.normalized_input.size()) return;
        const auto source = std::string_view(parsed.normalized_input).substr(
            0, candidate.consumed_input_bytes);
        if (user_frequency_->preferred(source, candidate.text)) {
            candidate.user_lexicon = true;
            candidate.provisional_user_word = false;
        }
    };
    const bool phonetic_assistance = parsed.phonetic_assistance;
    // A correction-only parse is an isolated fallback source. Looking up
    // abbreviations from the raw typo here can overwrite a full corrected
    // character with a higher-scoring, one-letter prefix of the same text.
    const bool correction_only = !parsed.paths.empty() &&
        std::all_of(parsed.paths.begin(), parsed.paths.end(), [](const ParsePath& path) {
            return path.match_kind == InputMatchKind::corrected;
        });

    // A lone consonant is a request for common characters across every valid
    // final, not for the first parser completion bucket (da, dai, dan, ...).
    // The lexicon's prebuilt initial index is globally frequency ordered, so
    // use it directly and retain only single-syllable, single-character
    // entries. This gives d -> 的/都/对/等/到/但... instead of allowing one
    // completion path to fill the whole result budget.
    constexpr std::string_view vowels = "aeiouv";
    const bool trailing_separator = parsed.normalized_input.ends_with('\'');
    const auto initial_sequence_input = std::string_view(parsed.normalized_input).substr(
        0, parsed.normalized_input.size() - (trailing_separator ? 1 : 0));
    const bool pure_initial_sequence = phonetic_assistance && !correction_only &&
        initial_sequence_input.size() >= 2 &&
        initial_sequence_input.size() <= 256 &&
        std::all_of(initial_sequence_input.begin(), initial_sequence_input.end(),
                    [vowels](const char value) {
            return value >= 'a' && value <= 'z' &&
                   vowels.find(value) == std::string_view::npos;
        });
    const bool single_initial = phonetic_assistance && !correction_only &&
        initial_sequence_input.size() == 1 &&
        initial_sequence_input.front() >= 'a' &&
        initial_sequence_input.front() <= 'z' &&
        vowels.find(initial_sequence_input.front()) == std::string_view::npos;
    if (single_initial) {
        const auto lookup_started = std::chrono::steady_clock::now();
        auto entries = lexicon_.lookup_initial_texts(
            initial_sequence_input.front(), std::max<std::size_t>(256, search_limit * 16));
        if (metrics != nullptr) {
            metrics->lexicon_lookup_us += static_cast<std::uint64_t>(
                std::chrono::duration_cast<std::chrono::microseconds>(
                    std::chrono::steady_clock::now() - lookup_started).count());
            ++metrics->lexicon_lookup_count;
        }
        CandidateAccumulator initial_candidates(entries.size());
        for (const auto& entry : entries) {
            if (cancelled && cancelled()) return {};
            if (utf8_character_count(entry.text) != 1)
                continue;
            auto score = candidate_unigram_score(entry.frequency) +
                         common_initial_seed_bonus(parsed.normalized_input.front(),
                                                   entry.text);
            if (user_frequency_ != nullptr) {
                score += user_frequency_->score(entry.text);
                if (contextual_ranking)
                    score += user_frequency_->contextual_score(
                        parsed.normalized_input, entry.text);
                if (contextual_ranking && !language_context.empty())
                    score += user_frequency_->language_context_score(
                        language_context, parsed.normalized_input, entry.text);
            }
            Candidate candidate{std::string(entry.text), {std::string(entry.syllable)}, score,
                                InputMatchKind::incomplete_completion,
                                {std::string(initial_sequence_input)},
                                parsed.normalized_input.size()};
            apply_learned_preference(candidate);
            initial_candidates.store(std::move(candidate));
        }
        auto candidates = initial_candidates.take();
        const auto sort_started = std::chrono::steady_clock::now();
        std::sort(candidates.begin(), candidates.end(), score_less);
        if (candidates.size() > limit) candidates.resize(limit);
        if (metrics != nullptr)
            metrics->sort_us += static_cast<std::uint64_t>(
                std::chrono::duration_cast<std::chrono::microseconds>(
                    std::chrono::steady_clock::now() - sort_started).count());
        return candidates;
    }

    const bool double_initial = pure_initial_sequence &&
                                initial_sequence_input.size() == 2;
    if (double_initial) {
        // A double-initial request has two distinct candidate sources:
        // real two-character dictionary words and common characters for the
        // first initial. Never synthesize the first group by combining
        // arbitrary characters. Alternation derives the ratio from the
        // requested result count, so no candidate page size is hard-coded.
        CandidateAccumulator dictionary_unique;
        CandidateAccumulator prefix_unique;

        const auto lookup_started = std::chrono::steady_clock::now();
        auto dictionary_matches = lexicon_.lookup_mixed_abbreviation_texts(
            initial_sequence_input, std::max<std::size_t>(64, search_limit * 16));
        auto prefix_entries = lexicon_.lookup_initial_texts(
            initial_sequence_input.front(),
            std::max<std::size_t>(64, search_limit * 8));
        if (metrics != nullptr) {
            metrics->lexicon_lookup_us += static_cast<std::uint64_t>(
                std::chrono::duration_cast<std::chrono::microseconds>(
                    std::chrono::steady_clock::now() - lookup_started).count());
            metrics->lexicon_lookup_count += 2;
        }

        for (auto& match : dictionary_matches) {
            if (cancelled && cancelled()) return {};
            if (match.syllables.size() != 2 ||
                match.source_segments.size() != 2 ||
                utf8_character_count(match.text) != 2)
                continue;
            auto score = candidate_unigram_score(match.frequency) +
                         kMixedAbbreviationPhraseBonus - kAbbreviationBasePenalty -
                         static_cast<std::int64_t>(
                             match.syllables[0].size() +
                             match.syllables[1].size() - 2) *
                             kIncompleteCharacterPenalty;
            if (user_frequency_ != nullptr) {
                score += user_frequency_->score(match.text);
                if (contextual_ranking)
                    score += user_frequency_->contextual_score(
                        parsed.normalized_input, match.text);
                if (contextual_ranking && !language_context.empty())
                    score += user_frequency_->language_context_score(
                        language_context, parsed.normalized_input,
                        match.text);
            }
            Candidate candidate{std::string(match.text), own_syllables(match.syllables), score,
                                InputMatchKind::abbreviated_completion,
                                std::move(match.source_segments),
                                parsed.normalized_input.size()};
            apply_learned_preference(candidate);
            dictionary_unique.store(std::move(candidate));
        }

        for (const auto& entry : prefix_entries) {
            if (cancelled && cancelled()) return {};
            if (utf8_character_count(entry.text) != 1)
                continue;
            auto score = candidate_unigram_score(entry.frequency);
            if (user_frequency_ != nullptr) {
                score += user_frequency_->score(entry.text);
                if (contextual_ranking)
                    score += user_frequency_->contextual_score(
                        std::string_view(parsed.normalized_input).substr(0, 1),
                        entry.text);
                if (contextual_ranking && !language_context.empty())
                    score += user_frequency_->language_context_score(
                        language_context,
                        std::string_view(parsed.normalized_input).substr(0, 1),
                        entry.text);
            }
            Candidate candidate{std::string(entry.text), {std::string(entry.syllable)}, score,
                                InputMatchKind::abbreviated_completion,
                                {std::string(initial_sequence_input.substr(0, 1))}, 1};
            apply_learned_preference(candidate);
            prefix_unique.store(std::move(candidate));
        }

        auto dictionary = dictionary_unique.take();
        auto prefixes = prefix_unique.take();
        std::sort(dictionary.begin(), dictionary.end(), score_less);
        std::sort(prefixes.begin(), prefixes.end(), score_less);

        std::vector<Candidate> candidates;
        candidates.reserve(std::min(limit, dictionary.size() + prefixes.size()));
        std::size_t dictionary_index = 0;
        std::size_t prefix_index = 0;
        bool dictionary_turn = true;
        while (candidates.size() < limit &&
               (dictionary_index < dictionary.size() ||
                prefix_index < prefixes.size())) {
            if (dictionary_turn && dictionary_index < dictionary.size())
                candidates.push_back(std::move(dictionary[dictionary_index++]));
            else if (!dictionary_turn && prefix_index < prefixes.size())
                candidates.push_back(std::move(prefixes[prefix_index++]));
            else if (dictionary_index < dictionary.size())
                candidates.push_back(std::move(dictionary[dictionary_index++]));
            else
                candidates.push_back(std::move(prefixes[prefix_index++]));
            dictionary_turn = !dictionary_turn;
        }
        return candidates;
    }

    CandidateAccumulator unique(std::max<std::size_t>(64, search_limit * 2));
    const auto store_candidate = [&unique, &apply_learned_preference](Candidate candidate) {
        apply_learned_preference(candidate);
        unique.store(std::move(candidate));
    };
    if (user_frequency_ != nullptr && !correction_only) {
        std::string lookup_input;
        lookup_input.reserve(parsed.normalized_input.size());
        std::copy_if(parsed.normalized_input.begin(), parsed.normalized_input.end(),
                     std::back_inserter(lookup_input),
                     [](const char value) { return value != '\''; });
        const auto exact_path = std::find_if(
            parsed.paths.begin(), parsed.paths.end(), [](const ParsePath& value) {
                return value.match_kind == InputMatchKind::exact;
            });
        if (exact_path != parsed.paths.end()) {
            const auto learned_entries = user_frequency_->lookup(
                lookup_input, search_limit);
            for (const auto& entry : learned_entries) {
                // Old prefix-completion selections may have left a long word
                // bound to a short exact spelling. Ignore that alias without
                // deleting user data. Check every exact segmentation so a
                // legitimate ambiguous spelling such as xian -> xi'an survives.
                const auto learned_path = std::find_if(
                    parsed.paths.begin(), parsed.paths.end(), [&](const ParsePath& path) {
                        return path.match_kind == InputMatchKind::exact &&
                               user_word_matches_source_path(parsed, path, entry.text);
                    });
                if (learned_path == parsed.paths.end()) continue;
                const auto learned_segments = source_segments(parsed, *learned_path);
                std::vector<std::string> learned_syllables;
                learned_syllables.reserve(learned_path->syllables.size());
                for (const auto& syllable : learned_path->syllables)
                    learned_syllables.push_back(syllable.text);
                auto score = (entry.preferred ? 12'000 : 0) + candidate_unigram_score(entry.count) +
                             user_frequency_->score(entry.text) +
                             user_frequency_->contextual_score(lookup_input,
                                                                entry.text);
                if (contextual_ranking && !language_context.empty())
                    score += user_frequency_->language_context_score(
                        language_context, lookup_input, entry.text);
                Candidate candidate{entry.text, learned_syllables, score,
                                    InputMatchKind::exact, learned_segments,
                                    parsed.normalized_input.size(), 1};
                candidate.segment_lengths = {learned_syllables.size()};
                candidate.user_lexicon = true;
                candidate.provisional_user_word = !entry.preferred;
                store_candidate(std::move(candidate));
            }
        }

        const auto prefix_path = std::find_if(
            parsed.paths.begin(), parsed.paths.end(), [](const ParsePath& value) {
                return value.match_kind == InputMatchKind::exact ||
                       value.match_kind == InputMatchKind::incomplete_completion;
            });
        if (phonetic_assistance && user_completion_enabled &&
            prefix_path != parsed.paths.end()) {
            const auto learned_prefixes = user_frequency_->lookup_prefix(
                lookup_input, search_limit);
            const auto learned_segments = source_segments(parsed, *prefix_path);
            std::vector<std::string> learned_syllables;
            learned_syllables.reserve(prefix_path->syllables.size());
            for (const auto& syllable : prefix_path->syllables)
                learned_syllables.push_back(syllable.text);
            for (const auto& entry : learned_prefixes) {
                // Keep distant user-lexicon predictions out of ordinary
                // candidates unless the user explicitly raises the completion
                // range. Zero still allows completing the current syllable.
                if (utf8_character_count(entry.text) >
                    learned_syllables.size() + user_completion_max_syllables)
                    continue;
                const auto completion_characters =
                    entry.input.size() - lookup_input.size();
                auto score = (entry.preferred ? 12'000 : 0) + candidate_unigram_score(entry.count) +
                             user_frequency_->score(entry.text) - 1'500 -
                             static_cast<std::int64_t>(completion_characters) *
                                 kIncompleteCharacterPenalty;
                Candidate candidate{
                    entry.text, learned_syllables, score,
                    InputMatchKind::incomplete_completion, learned_segments,
                    parsed.normalized_input.size(), 1};
                candidate.segment_lengths = {learned_syllables.size()};
                candidate.user_lexicon = true;
                candidate.provisional_user_word = !entry.preferred;
                store_candidate(std::move(candidate));
            }
        }
    }
    const auto mixed_limit = search_limit > 32 ? std::size_t{256}
                                               : std::max<std::size_t>(64, search_limit * 8);
    bool direct_mixed_candidate_found = false;
    const auto has_consonant_tail = [&] {
        if (!phonetic_assistance || correction_only) return false;
        if (parsed.normalized_input.size() < 4 ||
            parsed.normalized_input.size() > 12 ||
            parsed.normalized_input.find('\'') != std::string::npos)
            return false;
        const auto maximum_prefix = std::min<std::size_t>(
            6, parsed.normalized_input.size() - 2);
        for (std::size_t prefix_size = 2; prefix_size <= maximum_prefix;
             ++prefix_size) {
            const auto tail = std::string_view(parsed.normalized_input).substr(
                prefix_size);
            if (std::all_of(tail.begin(), tail.end(), [vowels](const char value) {
                    return value >= 'a' && value <= 'z' &&
                           vowels.find(value) == std::string_view::npos;
                }))
                return true;
        }
        return false;
    }();
    if (has_consonant_tail) {
        const auto direct_matches = lexicon_.lookup_mixed_abbreviation_texts(
            parsed.normalized_input, mixed_limit);
        for (const auto& match : direct_matches) {
            if (match.source_segments.size() < 3 ||
                match.source_segments.front().size() < 2)
                continue;
            const bool consonant_tail = std::all_of(
                match.source_segments.begin() + 1, match.source_segments.end(),
                [vowels](const std::string& segment) {
                    return segment.size() == 1 &&
                           vowels.find(segment.front()) == std::string_view::npos;
                });
            if (!consonant_tail) continue;
            auto score = candidate_unigram_score(match.frequency) +
                         kMixedAbbreviationPhraseBonus -
                         static_cast<std::int64_t>(mixed_completion_characters(
                             match.syllables, match.source_segments)) *
                             kIncompleteCharacterPenalty;
            if (user_frequency_ != nullptr) {
                score += user_frequency_->score(match.text);
                if (contextual_ranking)
                    score += user_frequency_->contextual_score(
                        parsed.normalized_input, match.text);
                if (contextual_ranking && !language_context.empty())
                    score += user_frequency_->language_context_score(
                        language_context, parsed.normalized_input, match.text);
            }
            store_candidate(Candidate{
                std::string(match.text), own_syllables(match.syllables), score,
                InputMatchKind::abbreviated_completion, match.source_segments,
                parsed.normalized_input.size()});
            direct_mixed_candidate_found = true;
        }
    }
    std::vector<OrderedParsePath> ordered_paths;
    ordered_paths.reserve(parsed.paths.size());
    const bool long_ambiguous_input = phonetic_assistance &&
                                      parsed.normalized_input.size() >= 12 &&
                                      parsed.normalized_input.find('\'') ==
                                          std::string::npos;
    // Short input also evaluates a small exact-path set: min'gan must not be
    // hidden merely because ming'an is the parser's first legal split.
    const auto exact_path_count = static_cast<std::size_t>(std::count_if(
        parsed.paths.begin(), parsed.paths.end(), [](const ParsePath& path) {
            return path.match_kind == InputMatchKind::exact;
        }));
    const std::size_t exact_path_limit = long_ambiguous_input
        ? std::min(exact_path_count,
                   std::max<std::size_t>(8, search_limit / 2))
        : 4;
    const ParsePath* preferred_exact_path = nullptr;
    for (const auto& path : parsed.paths) {
        if (path.match_kind == InputMatchKind::exact) {
            preferred_exact_path = &path;
            break;
        }
    }
    std::size_t exact_paths_added = 0;
    for (const auto kind : {InputMatchKind::exact,
                            InputMatchKind::incomplete_completion,
                            InputMatchKind::corrected,
                            InputMatchKind::abbreviated_completion}) {
        for (const auto& path : parsed.paths) {
            if (cancelled && cancelled()) return {};
            if (path.match_kind != kind) continue;
            // The first exact path is also the segmentation shown by the
            // pinyin preview. Evaluating other equally valid segmentations
            // lets corpus frequency silently replace wan'an with wa'nan (and
            // nan'an with na'nan), making the candidates contradict the UI.
            // Assisted paths remain plural because they intentionally offer
            // alternative completions and corrections.
            if (kind == InputMatchKind::exact) {
                if (exact_paths_added >= exact_path_limit) continue;
                ++exact_paths_added;
            }
            std::uint32_t full_span_frequency = 0;
            if (std::all_of(path.syllables.begin(), path.syllables.end(),
                            [](const Syllable& syllable) {
                                return syllable.complete;
                            })) {
                std::vector<std::string_view> reading;
                reading.reserve(path.syllables.size());
                for (const auto& syllable : path.syllables)
                    reading.push_back(syllable.text);
                full_span_frequency = lookup_cache.maximum(reading);
            }
            ordered_paths.push_back({&path, full_span_frequency, 0, 0});
        }
    }
    const bool assisted_full_span_found = std::any_of(
        ordered_paths.begin(), ordered_paths.end(), [](const OrderedParsePath& value) {
            return value.path->match_kind != InputMatchKind::exact &&
                   value.full_span_frequency != 0;
        });
    // Whole-phrase matches outrank partial phrase coverage. Only run the more
    // expensive dynamic-programming evidence pass when no assisted path has a
    // whole dictionary match. Exact path ordering intentionally follows the
    // preview segmentation and never consumes this evidence.
    if (!assisted_full_span_found) {
        for (auto& ordered : ordered_paths) {
            if (cancelled && cancelled()) return {};
            const auto& path = *ordered.path;
            if (path.match_kind == InputMatchKind::exact) continue;
            std::vector<std::size_t> covered(path.syllables.size() + 1);
            std::vector<std::uint64_t> phrase_evidence(path.syllables.size() + 1);
            const auto maximum_phrase_length = std::min<std::size_t>(
                4, lexicon_.maximum_reading_length());
            for (std::size_t begin = 0; begin < path.syllables.size(); ++begin) {
                if (cancelled && cancelled()) return {};
                if (begin != 0 && covered[begin] == 0) continue;
                if (covered[begin + 1] < covered[begin]) {
                    covered[begin + 1] = covered[begin];
                    phrase_evidence[begin + 1] = phrase_evidence[begin];
                }
                std::vector<std::string_view> reading;
                reading.reserve(maximum_phrase_length);
                for (std::size_t end = begin + 1;
                     end <= path.syllables.size() &&
                     end - begin <= maximum_phrase_length; ++end) {
                    reading.push_back(path.syllables[end - 1].text);
                    if (end - begin < 2) continue;
                    const auto best_frequency =
                        lookup_cache.maximum(reading);
                    if (best_frequency == 0) continue;
                    const auto next_covered = covered[begin] + end - begin;
                    const auto next_evidence = phrase_evidence[begin] + best_frequency;
                    if (next_covered > covered[end] ||
                        (next_covered == covered[end] &&
                         next_evidence > phrase_evidence[end])) {
                        covered[end] = next_covered;
                        phrase_evidence[end] = next_evidence;
                    }
                }
            }
            ordered.phrase_covered_syllables = covered.back();
            ordered.phrase_frequency_evidence = phrase_evidence.back();
        }
    }
    // Parser ordering is deliberately independent from a concrete lexicon.
    // Before applying the assisted-path budget, use dictionary evidence to
    // prevent an alphabetically early completion family from starving a
    // complete word. Exact paths keep their preview-defined order.
    std::stable_sort(
        ordered_paths.begin(), ordered_paths.end(),
        [](const OrderedParsePath& left, const OrderedParsePath& right) {
            if (left.path->match_kind != right.path->match_kind)
                return left.path->match_kind < right.path->match_kind;
            if (left.path->match_kind == InputMatchKind::exact) return false;
            if ((left.full_span_frequency != 0) !=
                (right.full_span_frequency != 0))
                return left.full_span_frequency != 0;
            if (left.full_span_frequency != right.full_span_frequency)
                return left.full_span_frequency > right.full_span_frequency;
            if (left.phrase_covered_syllables != right.phrase_covered_syllables)
                return left.phrase_covered_syllables > right.phrase_covered_syllables;
            if (left.phrase_frequency_evidence != right.phrase_frequency_evidence)
                return left.phrase_frequency_evidence > right.phrase_frequency_evidence;
            if (left.path->completion_characters !=
                right.path->completion_characters)
                return left.path->completion_characters <
                       right.path->completion_characters;
            return false;
        });
    const bool expose_long_alternatives =
        long_ambiguous_input && exact_paths_added > 1;
    bool exact_candidate_found = false;
    std::size_t evaluated_exact_paths = 0;
    const std::size_t maximum_assisted_paths_per_kind =
        direct_mixed_candidate_found ? 2 : 16;
    std::array<std::size_t,
               static_cast<std::size_t>(InputMatchKind::abbreviated_completion) + 1>
        evaluated_paths{};
    for (const auto& ordered_path : ordered_paths) {
        if (cancelled && cancelled()) return {};
        const auto& path = *ordered_path.path;
        // Corrections are a fallback for spellings not covered by the lexicon.
        // Prefix completions remain available beside exact candidates.
        if (path.match_kind == InputMatchKind::corrected && exact_candidate_found) continue;
        if (std::any_of(path.syllables.begin(), path.syllables.end(),
                        [](const Syllable& value) { return !value.complete; })) continue;

        const auto raw_segments = source_segments(parsed, path);
        if (raw_segments.size() != path.syllables.size()) continue;
        const auto kind_index = static_cast<std::size_t>(path.match_kind);
        if (path.match_kind != InputMatchKind::exact &&
            evaluated_paths[kind_index] >= maximum_assisted_paths_per_kind) continue;
        ++evaluated_paths[kind_index];
        if (path.match_kind == InputMatchKind::exact) ++evaluated_exact_paths;

        std::vector<std::vector<SearchState>> chart(path.syllables.size() + 1);
        SearchState initial;
        initial.score = -candidate_input_match_penalty(path) +
                        source_initial_bonus(parsed, path);
        if (exact_path_count > 1 && &path == preferred_exact_path)
            initial.score += kPreferredExactSegmentationBonus;
        chart[0].push_back(std::move(initial));
        const std::size_t beam_width = long_ambiguous_input
            ? path.match_kind == InputMatchKind::exact
                  ? std::max<std::size_t>(32, search_limit * 4)
                  : std::max<std::size_t>(24, search_limit * 3)
            : path.match_kind == InputMatchKind::exact
                  ? std::max<std::size_t>(16, search_limit * 4)
                  : std::max<std::size_t>(12, search_limit * 2);
        const std::size_t maximum_reading_length = lexicon_.maximum_reading_length();
        for (std::size_t begin = 0; begin < path.syllables.size(); ++begin) {
            if (cancelled && cancelled()) return {};
            if (chart[begin].empty()) continue;
            prune(chart[begin], beam_width);
            const auto maximum_end = std::min(path.syllables.size(),
                                              begin + maximum_reading_length);
            for (std::size_t end = begin + 1; end <= maximum_end; ++end) {
                std::vector<std::string_view> reading;
                reading.reserve(end - begin);
                for (std::size_t index = begin; index < end; ++index)
                    reading.push_back(path.syllables[index].text);
                auto entries = lookup_cache.lookup(reading);
                retain_frequent_entries(entries, beam_width);
                std::vector<std::int64_t> entry_scores;
                entry_scores.reserve(entries.size());
                for (const auto& entry : entries)
                    entry_scores.push_back(candidate_unigram_score(entry.frequency));
                for (const auto& state : chart[begin]) {
                    if (cancelled && cancelled()) return {};
                    for (std::size_t entry_index = 0; entry_index < entries.size(); ++entry_index) {
                        const auto& entry = entries[entry_index];
                        auto score = state.score + entry_scores[entry_index];
                        if (!state.previous.empty()) score -= kAdditionalSegmentPenalty;
                        if (bigram_ != nullptr && !state.previous.empty())
                            score += bigram_->score(state.previous, entry.text);
                        append_successor(chart[end], beam_width, state, entry.text, score, 0,
                            [&](SearchState& next) {
                                next.previous.assign(entry.text);
                                ++next.segment_count;
                                next.segment_lengths.push_back(reading.size());
                            }, metrics);
                    }
                }
            }
        }

        prune(chart.back(), beam_width);
        // Every exact/assisted path has one fixed reading, independent of
        // which word transitions won. Materialize it only for final results,
        // not in every intermediate beam state.
        std::vector<std::string> full_reading;
        full_reading.reserve(path.syllables.size());
        for (const auto& syllable : path.syllables) full_reading.push_back(syllable.text);
        for (auto& state : chart.back()) {
            if (user_frequency_ != nullptr) {
                state.score += user_frequency_->score(state.text);
                if (contextual_ranking)
                    state.score += user_frequency_->contextual_score(
                        parsed.normalized_input, state.text);
                if (contextual_ranking && !language_context.empty())
                    state.score += user_frequency_->language_context_score(
                        language_context, parsed.normalized_input, state.text);
            }
            Candidate candidate{std::move(state.text), full_reading, state.score,
                                path.match_kind, raw_segments,
                                path.syllables.back().end, state.segment_count};
            candidate.segment_lengths = std::move(state.segment_lengths);
            if (path.match_kind == InputMatchKind::exact) exact_candidate_found = true;
            store_candidate(std::move(candidate));
        }

        // In addition to whole-input sentences, expose dictionary words that
        // consume a leading range. TSF can commit one of these and request new
        // candidates for the unconsumed suffix.
        const auto maximum_prefix = std::min(path.syllables.size() - 1,
                                             lexicon_.maximum_reading_length());
        for (std::size_t end = 1; end <= maximum_prefix; ++end) {
            std::vector<std::string_view> reading;
            reading.reserve(end);
            for (std::size_t index = 0; index < end; ++index)
                reading.push_back(path.syllables[index].text);
            auto prefix_entries = lookup_cache.lookup(reading);
            retain_frequent_entries(prefix_entries, search_limit);
            for (const auto& entry : prefix_entries) {
                auto score = candidate_unigram_score(entry.frequency) -
                             candidate_input_match_penalty(path) +
                             source_initial_bonus(parsed, path);
                if (user_frequency_ != nullptr) {
                    score += user_frequency_->score(entry.text);
                    if (contextual_ranking)
                        score += user_frequency_->contextual_score(
                            std::string_view(parsed.normalized_input).substr(
                                0, path.syllables[end - 1].end), entry.text);
                    if (contextual_ranking && !language_context.empty())
                        score += user_frequency_->language_context_score(
                            language_context,
                            std::string_view(parsed.normalized_input).substr(
                                0, path.syllables[end - 1].end), entry.text);
                }
                store_candidate(Candidate{std::string(entry.text), own_syllables(reading), score,
                                          path.match_kind, raw_segments,
                                          path.syllables[end - 1].end});
            }
        }

        // Long input can have dozens of valid pinyin segmentations. Once an
        // exact full-input path has already supplied the requested number of
        // candidates, evaluating every lower-priority segmentation only adds
        // latency and cannot improve paging capacity.
        if (((exact_candidate_found &&
              evaluated_exact_paths >= exact_paths_added) ||
             path.match_kind == InputMatchKind::incomplete_completion) &&
            unique.size() >= search_limit)
            break;
    }

    // A string such as "nm" is technically parseable as two interjection
    // syllables, but users normally intend one abbreviated character per
    // consonant. This lexicon-aware fallback avoids spending the parser's
    // bounded path budget on every possible syllable completion.
    if (pure_initial_sequence) {
        std::vector<std::string> raw_segments;
        raw_segments.reserve(initial_sequence_input.size());
        for (const char initial : initial_sequence_input)
            raw_segments.emplace_back(1, initial);

        const auto beam_width = std::max<std::size_t>(16, search_limit * 2);
        std::vector<std::vector<SearchState>> chart(initial_sequence_input.size() + 1);
        SearchState initial;
        initial.score = -kAbbreviationBasePenalty;
        chart.front().push_back(std::move(initial));
        for (std::size_t offset = 0; offset < initial_sequence_input.size(); ++offset) {
            if (cancelled && cancelled()) return {};
            prune(chart[offset], beam_width);
            const auto lookup_started = std::chrono::steady_clock::now();
            auto entries = lexicon_.lookup_initial_texts(
                initial_sequence_input[offset], beam_width);
            if (metrics != nullptr) {
                metrics->lexicon_lookup_us += static_cast<std::uint64_t>(
                    std::chrono::duration_cast<std::chrono::microseconds>(
                        std::chrono::steady_clock::now() - lookup_started).count());
                ++metrics->lexicon_lookup_count;
            }
            std::vector<std::int64_t> entry_scores;
            entry_scores.reserve(entries.size());
            for (const auto& entry : entries) {
                auto score = candidate_unigram_score(entry.frequency) -
                    static_cast<std::int64_t>(entry.syllable.size() - 1) *
                        kIncompleteCharacterPenalty;
                if (preferred_initial_syllable(initial_sequence_input[offset]) == entry.syllable)
                    score += kPreferredInitialBonus;
                entry_scores.push_back(score);
            }
            for (const auto& state : chart[offset]) {
                if (cancelled && cancelled()) return {};
                for (std::size_t entry_index = 0; entry_index < entries.size(); ++entry_index) {
                    const auto& entry = entries[entry_index];
                    auto score = state.score + entry_scores[entry_index];
                    if (!state.previous.empty()) score -= kAdditionalSegmentPenalty;
                    if (bigram_ != nullptr && !state.previous.empty())
                        score += bigram_->score(state.previous, entry.text);
                    append_successor(chart[offset + 1], beam_width, state, entry.text, score, 1,
                        [&](SearchState& next) {
                            next.previous.assign(entry.text);
                            ++next.segment_count;
                            next.segment_lengths.push_back(1);
                            next.syllables.emplace_back(entry.syllable);
                        }, metrics);
                }
            }
            // Also consume two or more initials as one real dictionary word.
            // This lets rw+ky+y form 认为+可以+用 instead of limiting the
            // search graph to arbitrary one-character combinations.
            const auto maximum_word_end = std::min(
                initial_sequence_input.size(), offset + std::size_t{4});
            for (std::size_t end = offset + 2; end <= maximum_word_end; ++end) {
                const auto source = initial_sequence_input.substr(offset, end - offset);
                const auto phrase_started = std::chrono::steady_clock::now();
                auto phrases = lexicon_.lookup_mixed_abbreviation_texts(
                    source, std::max<std::size_t>(64, beam_width));
                if (metrics != nullptr) {
                    metrics->lexicon_lookup_us += static_cast<std::uint64_t>(
                        std::chrono::duration_cast<std::chrono::microseconds>(
                            std::chrono::steady_clock::now() - phrase_started).count());
                    ++metrics->lexicon_lookup_count;
                }
                for (const auto& phrase : phrases) {
                    if (cancelled && cancelled()) return {};
                    if (phrase.syllables.size() != source.size() ||
                        phrase.source_segments.size() != source.size() ||
                        !std::all_of(phrase.source_segments.begin(),
                                     phrase.source_segments.end(),
                                     [](const std::string& segment) {
                                         return segment.size() == 1;
                                     }))
                        continue;
                    const auto phrase_score = candidate_unigram_score(phrase.frequency) +
                        kMixedAbbreviationPhraseBonus - kAbbreviationBasePenalty -
                        static_cast<std::int64_t>(mixed_completion_characters(
                            phrase.syllables, phrase.source_segments)) * kIncompleteCharacterPenalty;
                    for (const auto& state : chart[offset]) {
                        auto score = state.score + phrase_score;
                        if (!state.previous.empty())
                            score -= kAdditionalSegmentPenalty;
                        if (bigram_ != nullptr && !state.previous.empty())
                            score += bigram_->score(state.previous, phrase.text);
                        append_successor(chart[end], beam_width, state, phrase.text, score,
                            phrase.syllables.size(),
                            [&](SearchState& next) {
                                next.previous.assign(phrase.text);
                                ++next.segment_count;
                                next.segment_lengths.push_back(phrase.syllables.size());
                                for (const auto syllable : phrase.syllables)
                                    next.syllables.emplace_back(syllable);
                            }, metrics);
                    }
                }
            }
        }
        prune(chart.back(), beam_width);
        for (std::size_t consumed = 1; consumed < chart.size(); ++consumed) {
            for (auto state : chart[consumed]) {
                if (user_frequency_ != nullptr) {
                    state.score += user_frequency_->score(state.text);
                    if (contextual_ranking)
                        state.score += user_frequency_->contextual_score(
                            std::string_view(parsed.normalized_input).substr(0, consumed),
                            state.text);
                    if (contextual_ranking && !language_context.empty())
                        state.score += user_frequency_->language_context_score(
                            language_context,
                            std::string_view(parsed.normalized_input).substr(0, consumed),
                            state.text);
                }
                Candidate candidate{
                    std::move(state.text), std::move(state.syllables), state.score,
                    InputMatchKind::abbreviated_completion, raw_segments,
                    trailing_separator && consumed == initial_sequence_input.size()
                        ? parsed.normalized_input.size()
                        : consumed,
                    state.segment_count};
                candidate.segment_lengths = std::move(state.segment_lengths);
                store_candidate(std::move(candidate));
            }
        }
    }

    // If the leading syllable is complete, let the lexicon resolve the
    // remaining compact prefixes as a whole phrase. This covers inputs such
    // as "bugd" -> bu/gan/dang without enumerating and pruning thousands of
    // g*/d* parser combinations before dictionary evidence is available.
    if (cancelled && cancelled()) return {};
    const bool mixed_fallback = unique.size() == 0 || pure_initial_sequence;
    const auto mixed_lookup_started = std::chrono::steady_clock::now();
    const auto mixed_matches = !phonetic_assistance || correction_only ||
                               direct_mixed_candidate_found || exact_candidate_found
                                   ? std::vector<AbbreviatedLexiconTextMatch>{}
                                   : lexicon_.lookup_mixed_abbreviation_texts(
                                         parsed.normalized_input, mixed_limit);
    if (metrics != nullptr && phonetic_assistance && !correction_only &&
        !direct_mixed_candidate_found && !exact_candidate_found) {
        metrics->lexicon_lookup_us += static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::microseconds>(
                std::chrono::steady_clock::now() - mixed_lookup_started).count());
        ++metrics->lexicon_lookup_count;
    }
    bool mixed_abbreviation_candidate_found = direct_mixed_candidate_found ||
                                               pure_initial_sequence;
    for (auto match : mixed_matches) {
        if (cancelled && cancelled()) return {};
        if (unique.contains(match.text)) continue;
        std::size_t abbreviated_segments = 0;
        std::size_t exact_segments = 0;
        for (std::size_t index = 0; index < match.syllables.size(); ++index)
            if (match.source_segments[index].size() < match.syllables[index].size())
                ++abbreviated_segments;
            else
                ++exact_segments;
        const auto completion_characters = mixed_completion_characters(
            match.syllables, match.source_segments);
        const bool close_mixed_word = exact_segments != 0 &&
                                      completion_characters <= 2;
        if ((mixed_fallback && abbreviated_segments < 2) ||
            (!mixed_fallback && !close_mixed_word))
            continue;
        auto score = candidate_unigram_score(match.frequency) +
                     kMixedAbbreviationPhraseBonus - kAbbreviationBasePenalty -
                     static_cast<std::int64_t>(completion_characters) *
                         kIncompleteCharacterPenalty;
        if (user_frequency_ != nullptr) {
            score += user_frequency_->score(match.text);
            if (contextual_ranking)
                score += user_frequency_->contextual_score(
                    parsed.normalized_input, match.text);
            if (contextual_ranking && !language_context.empty())
                score += user_frequency_->language_context_score(
                    language_context, parsed.normalized_input, match.text);
        }
        store_candidate(Candidate{std::string(match.text), own_syllables(match.syllables), score,
                                  InputMatchKind::abbreviated_completion,
                                  std::move(match.source_segments),
                                  parsed.normalized_input.size()});
        mixed_abbreviation_candidate_found = true;
    }

    // Whole-input abbreviation matches are only one candidate source. Keep a
    // leading-character source as well so compact input such as nji, wsm and
    // rwkyy can still populate the single-character side of a partitioned
    // page. Previously those inputs were filled entirely by whole words or
    // sentence compositions before Core had anything it could balance.
    if (mixed_abbreviation_candidate_found && !exact_candidate_found &&
        parsed.normalized_input.size() >= 2 &&
        parsed.normalized_input.find('\'') == std::string::npos &&
        parsed.normalized_input.front() >= 'a' &&
        parsed.normalized_input.front() <= 'z' &&
        vowels.find(parsed.normalized_input.front()) == std::string_view::npos) {
        const auto leading_source = std::string_view(parsed.normalized_input).substr(0, 1);
        const auto lookup_started = std::chrono::steady_clock::now();
        auto leading_entries = lexicon_.lookup_initial_texts(
            parsed.normalized_input.front(),
            std::max<std::size_t>(64, search_limit * 8));
        if (metrics != nullptr) {
            metrics->lexicon_lookup_us += static_cast<std::uint64_t>(
                std::chrono::duration_cast<std::chrono::microseconds>(
                    std::chrono::steady_clock::now() - lookup_started).count());
            ++metrics->lexicon_lookup_count;
        }
        for (const auto& entry : leading_entries) {
            if (cancelled && cancelled()) return {};
            if (utf8_character_count(entry.text) != 1) continue;
            auto score = candidate_unigram_score(entry.frequency) +
                         common_initial_seed_bonus(parsed.normalized_input.front(),
                                                   entry.text);
            if (user_frequency_ != nullptr) {
                score += user_frequency_->score(entry.text);
                if (contextual_ranking)
                    score += user_frequency_->contextual_score(
                        leading_source, entry.text);
                if (contextual_ranking && !language_context.empty())
                    score += user_frequency_->language_context_score(
                        language_context, leading_source, entry.text);
            }
            store_candidate(Candidate{
                std::string(entry.text), {std::string(entry.syllable)}, score,
                InputMatchKind::abbreviated_completion,
                {std::string(leading_source)}, leading_source.size()});
        }
    }

    auto candidates = unique.take();
    if (parsed.normalized_input.ends_with('\'')) {
        const auto separator = parsed.normalized_input.size() - 1;
        for (auto& candidate : candidates) {
            if (candidate.consumed_input_bytes == separator)
                candidate.consumed_input_bytes = parsed.normalized_input.size();
        }
    }
    const auto sort_started = std::chrono::steady_clock::now();
    const auto full_input_bytes = parsed.normalized_input.size();
    std::vector<Candidate> full;
    std::vector<Candidate> prefixes;
    for (auto& candidate : candidates) {
        if (candidate.consumed_input_bytes == full_input_bytes)
            full.push_back(std::move(candidate));
        else
            prefixes.push_back(std::move(candidate));
    }
    std::sort(full.begin(), full.end(), score_less);
    // Keep no more than two whole dictionary words for one two-syllable
    // reading. A composition assembled from arbitrary single-character entries
    // is not a dictionary word: omit it when any whole word exists, otherwise
    // retain one fallback. This prevents both corpus noise and Beam permutations
    // from filling pages for inputs such as chan+wu or ni+jian.
    if (partition_page_size == 0)
        limit_two_syllable_full_candidates(full);
    const bool preserve_learned_alternative = partition_page_size != 0 &&
        std::any_of(full.begin(), full.end(), [](const auto& candidate) { return candidate.user_preferred(); });
    std::optional<Candidate> coherent_sentence_alternative;
    const auto apply_cross_boundary_coherence = [this, &lookup_cache, preserve_learned_alternative,
                                                &coherent_sentence_alternative](Candidate& candidate) {
        constexpr std::size_t kMaximumPhraseCharacters = 4;
        const auto character_count = candidate.syllables.size();
        if (character_count < 3 ||
            utf8_character_count(candidate.text) != character_count)
            return;

        std::vector<std::size_t> boundaries;
        boundaries.reserve(character_count + 1);
        boundaries.push_back(0);
        for (std::size_t offset = 1; offset < candidate.text.size(); ++offset) {
            if ((static_cast<unsigned char>(candidate.text[offset]) & 0xc0U) != 0x80U)
                boundaries.push_back(offset);
        }
        boundaries.push_back(candidate.text.size());
        if (boundaries.size() != character_count + 1) return;

        std::vector<std::size_t> dictionary_boundaries;
        if (candidate.match_kind == InputMatchKind::exact && candidate.segment_count > 1) {
            std::size_t offset = 0;
            for (std::size_t index = 0; index + 1 < candidate.segment_lengths.size(); ++index) {
                offset += candidate.segment_lengths[index];
                // Require an existing multi-character fragment on at least
                // one side, not arbitrary all-character permutations.
                if (candidate.segment_lengths[index] >= 2 ||
                    candidate.segment_lengths[index + 1] >= 2)
                    dictionary_boundaries.push_back(offset);
            }
        }
        bool dictionary_bridge = false;
        std::vector<bool> dictionary_covered(character_count, false);

        // Weighted interval scheduling over dictionary phrases gives a phrase
        // chain such as “以我” + “的想法” a continuity advantage without
        // repeatedly counting overlapping fragments such as every adjacent
        // pair in a noisy character sequence.
        constexpr auto unreachable = (std::numeric_limits<std::int64_t>::min)() / 4;
        std::vector<std::int64_t> best(character_count + 1, unreachable);
        best[0] = 0;
        std::vector<bool> dictionary_chain(character_count + 1, false);
        dictionary_chain[0] = true;
        const auto maximum_phrase = std::min(
            {kMaximumPhraseCharacters, character_count,
             lexicon_.maximum_reading_length()});
        for (std::size_t begin = 0; begin < character_count; ++begin) {
            if (best[begin] == unreachable) continue;
            best[begin + 1] = std::max(
                best[begin + 1],
                best[begin] - kUncoveredBoundaryCharacterPenalty);
            std::vector<std::string_view> reading;
            reading.reserve(maximum_phrase);
            for (std::size_t end = begin + 1;
                 end <= character_count && end - begin <= maximum_phrase; ++end) {
                reading.push_back(candidate.syllables[end - 1]);
                if (end - begin < 2 || (begin == 0 && end == character_count))
                    continue;
                const auto expected = std::string_view(candidate.text).substr(
                    boundaries[begin], boundaries[end] - boundaries[begin]);
                const auto phrase_score = lookup_cache.phrase_score(reading, expected);
                // A missing word is not a zero-cost phrase edge: it must
                // retain the uncovered-character penalty instead of letting
                // unrelated fragments jump across unsupported text.
                if (!phrase_score) continue;
                if (dictionary_chain[begin]) dictionary_chain[end] = true;
                std::fill(dictionary_covered.begin() + begin, dictionary_covered.begin() + end, true);
                if (end - begin >= 3 &&
                    std::any_of(dictionary_boundaries.begin(), dictionary_boundaries.end(),
                        [begin, end](const auto boundary) { return begin < boundary && boundary < end; }))
                    dictionary_bridge = true;
                best[end] = std::max(best[end], best[begin] + *phrase_score);
            }
        }
        candidate.coherence_score = std::max<std::int64_t>(0, best.back());
        // A real overlapping phrase that crosses a selected word boundary
        // provides evidence the two fragments belong together only when the
        // matched multi-character words cover the entire text. A local match
        // must not reward unsupported leading/trailing character permutations.
        // A complete chain of non-overlapping multi-character words already
        // received its phrase scores. Overlapping dictionary substrings must
        // not count as another independent signal and overwhelm a stronger
        // homophone chain. Reserve bridge rescue for text that actually needs
        // overlapping words to cover its isolated characters.
        if (dictionary_bridge && !dictionary_chain.back() &&
            std::all_of(dictionary_covered.begin(), dictionary_covered.end(),
                        [](const bool covered) { return covered; }))
            candidate.coherence_score += kMaximumBoundaryCoherenceBonus * 2;
        candidate.score += candidate.coherence_score;
        if (preserve_learned_alternative && candidate.match_kind == InputMatchKind::exact &&
            candidate.segment_count > 1 && candidate.coherence_score > 0 &&
            (!coherent_sentence_alternative || score_less(candidate, *coherent_sentence_alternative)))
            coherent_sentence_alternative = candidate;
    };
    // Only the candidates already close enough to reach the visible/model
    // comparison set need the more detailed phrase-chain pass.
    const auto coherence_limit = std::min(
        full.size(), std::max<std::size_t>(16, limit * 2));
    std::vector<bool> coherence_scored(full.size(), false);
    for (std::size_t index = 0; index < coherence_limit; ++index) {
        if (cancelled && cancelled()) return {};
        apply_cross_boundary_coherence(full[index]);
        coherence_scored[index] = true;
    }
    if (expose_long_alternatives) {
        std::vector<std::string> scored_segmentations;
        for (std::size_t index = 0; index < coherence_limit; ++index) {
            if (full[index].match_kind == InputMatchKind::exact)
                scored_segmentations.push_back(candidate_structure_key(full[index]));
        }
        for (std::size_t index = coherence_limit; index < full.size(); ++index) {
            if (cancelled && cancelled()) return {};
            if (full[index].match_kind != InputMatchKind::exact) continue;
            auto key = candidate_structure_key(full[index]);
            if (std::find(scored_segmentations.begin(), scored_segmentations.end(), key) !=
                scored_segmentations.end()) continue;
            scored_segmentations.push_back(std::move(key));
            apply_cross_boundary_coherence(full[index]);
            coherence_scored[index] = true;
            if (scored_segmentations.size() >= exact_path_limit) break;
        }
    }
    if (exact_paths_added > 1) {
        // A high-frequency reading can fill the raw top-N pool with character
        // permutations. Reserve a bounded comparison slice for every exact
        // source reading, including short inputs. Otherwise increasing the
        // page/search budget can hide an entire dictionary-backed phrase chain.
        constexpr std::size_t representatives_per_reading = 2;
        std::vector<std::pair<std::vector<std::string>, std::size_t>> readings;
        const auto count_for = [&readings](const Candidate& candidate) -> std::size_t& {
            const auto found = std::find_if(readings.begin(), readings.end(), [&](const auto& item) {
                return item.first == candidate.source_segments;
            });
            if (found != readings.end()) return found->second;
            readings.emplace_back(candidate.source_segments, 0);
            return readings.back().second;
        };
        for (std::size_t index = 0; index < full.size(); ++index) {
            if (coherence_scored[index] && full[index].match_kind == InputMatchKind::exact)
                ++count_for(full[index]);
        }
        for (std::size_t index = coherence_limit; index < full.size(); ++index) {
            if (cancelled && cancelled()) return {};
            auto& candidate = full[index];
            if (coherence_scored[index] || candidate.match_kind != InputMatchKind::exact) continue;
            auto& count = count_for(candidate);
            if (count >= representatives_per_reading) continue;
            apply_cross_boundary_coherence(candidate);
            ++count;
            if (readings.size() >= exact_paths_added &&
                std::all_of(readings.begin(), readings.end(), [](const auto& item) {
                    return item.second >= representatives_per_reading;
                })) break;
        }
    }
    const auto sentence_less = [](const Candidate& left, const Candidate& right) {
        if (left.user_preferred() != right.user_preferred())
            return left.user_preferred();
        const auto full_dictionary_word = [](const Candidate& candidate) {
            return candidate.match_kind == InputMatchKind::exact &&
                   !candidate.provisional_user_word &&
                   candidate.segment_count == 1 &&
                   utf8_character_count(candidate.text) ==
                       candidate.syllables.size();
        };
        if (full_dictionary_word(left) != full_dictionary_word(right))
            return full_dictionary_word(left);
        constexpr std::int64_t kCoherenceDecisionMargin = 3'000;
        if (left.segment_count > 1 && right.segment_count > 1) {
            const auto difference = left.coherence_score - right.coherence_score;
            if (difference >= kCoherenceDecisionMargin) return true;
            if (difference <= -kCoherenceDecisionMargin) return false;
        }
        return score_less(left, right);
    };
    std::sort(full.begin(), full.end(), sentence_less);
    // Expiring a one-off preference removes its boost, not the user's word.
    // Keep one independently recalled full word available when the sentence
    // budget below discards weaker homophone compositions of the same input.
    const auto recalled_user_word = std::find_if(full.begin(), full.end(), [](const Candidate& value) {
        return value.user_lexicon && value.segment_count == 1 &&
               utf8_character_count(value.text) == value.syllables.size();
    });
    std::optional<Candidate> retained_user_word;
    if (recalled_user_word != full.end()) retained_user_word = *recalled_user_word;
    // Prefer a whole-dictionary correction over generated
    // character/phrase permutations of that same repaired reading. Such
    // permutations otherwise enter the sentence partition ahead of the
    // complete word, undoing the minimal-edit repair during layout. Corpus
    // frequencies of multiple fragments are not comparable to one whole word.
    std::vector<Candidate> corrected_words;
    for (const auto& candidate : full) {
        if (candidate.match_kind == InputMatchKind::corrected &&
            candidate.segment_count == 1 && candidate.syllables.size() >= 3 &&
            utf8_character_count(candidate.text) == candidate.syllables.size())
            corrected_words.push_back(candidate);
    }
    if (!corrected_words.empty()) {
        std::erase_if(full, [&corrected_words](const Candidate& candidate) {
            return candidate.match_kind == InputMatchKind::corrected &&
                candidate.segment_count > 1 && !candidate.user_lexicon &&
                std::any_of(corrected_words.begin(), corrected_words.end(), [&](const auto& word) {
                    return word.syllables == candidate.syllables &&
                           word.source_segments == candidate.source_segments;
                });
        });
    }
    std::vector<Candidate> model_seeds;
    if (include_model_alternatives && !full.empty() &&
        full.front().syllables.size() >= 3) {
        constexpr std::size_t kMaximumModelStructures = 24;
        const auto& reference = full.front();
        std::unordered_set<std::string_view> texts;
        texts.reserve(kMaximumModelStructures);
        for (const auto& candidate : full) {
            if (candidate.consumed_input_bytes != full_input_bytes) continue;
            // A model rank is not evidence that an arbitrarily fragmented
            // homophone sequence is a valid alternative. Keep genuine source
            // readings and learned words, but do not let a much weaker split
            // of the same exact reading bypass the visible sentence budget.
            if (!candidate.user_preferred() && reference.match_kind == InputMatchKind::exact &&
                reference.coherence_score > candidate.coherence_score &&
                reference.score - candidate.score >= kAlternativeSegmentationPromotionMargin &&
                (candidate.match_kind != InputMatchKind::exact ||
                 (candidate.source_segments == reference.source_segments &&
                  candidate.syllables == reference.syllables &&
                  candidate.segment_count > reference.segment_count)))
                continue;
            if (!texts.insert(candidate.text).second) continue;
            model_seeds.push_back(candidate);
            if (model_seeds.size() == kMaximumModelStructures) break;
        }
    }
    // Long compact input may expose a second segmentation, but a noisier
    // composition from the winning segmentation must not consume that slot.
    // Preserve equal-complexity dictionary variants such as 自定/自订; skip a
    // candidate such as 一宽高度自定义 when it needs more lexicon fragments
    // than 一款高度自定义.
    if (expose_long_alternatives && full.size() > 1) {
        const auto first_segments = full.front().source_segments;
        const auto first_segment_count = full.front().segment_count;
        const auto first_segment_lengths = full.front().segment_lengths;
        const auto first_text = full.front().text;
        std::vector<Candidate> selected;
        selected.reserve(2);
        selected.push_back(std::move(full.front()));
        const auto lexical_variant = std::find_if(
            full.begin() + 1, full.end(), [&](const Candidate& candidate) {
                return candidate.source_segments == first_segments &&
                       candidate.segment_lengths == first_segment_lengths &&
                       candidate.segment_count == first_segment_count &&
                       utf8_difference_count(first_text, candidate.text) == 1;
            });
        auto alternative = lexical_variant;
        if (alternative == full.end()) {
            alternative = std::find_if(
                full.begin() + 1, full.end(), [&](const Candidate& candidate) {
                    return candidate.source_segments != first_segments &&
                           candidate.segment_lengths != first_segment_lengths;
                });
        }
        if (alternative == full.end()) {
            alternative = std::find_if(
                full.begin() + 1, full.end(), [&](const Candidate& candidate) {
                    return candidate.source_segments != first_segments;
                });
        }
        if (alternative == full.end()) {
            alternative = std::find_if(
                full.begin() + 1, full.end(), [&](const Candidate& candidate) {
                    return candidate.segment_count <= first_segment_count;
                });
        }
        if (alternative != full.end())
            selected.push_back(std::move(*alternative));
        full = std::move(selected);
    }
    std::sort(prefixes.begin(), prefixes.end(), [](const Candidate& left,
                                                   const Candidate& right) {
        if (left.consumed_input_bytes != right.consumed_input_bytes)
            return left.consumed_input_bytes > right.consumed_input_bytes;
        return score_less(left, right);
    });
    const auto exact = std::find_if(full.begin(), full.end(), [](const Candidate& candidate) {
        return candidate.match_kind == InputMatchKind::exact;
    });
    if (exact != full.end() && exact != full.begin() &&
        (full.empty() || !full.front().user_preferred()))
        std::rotate(full.begin(), exact, exact + 1);

    std::vector<Candidate> ordered;
    ordered.reserve(full.size() + prefixes.size());
    const bool multiword_input = !pure_initial_sequence && !full.empty() &&
                                 full.front().syllables.size() >= 3;
    if (multiword_input) {
        constexpr std::int64_t kSecondSentenceCoherenceMargin = 2'000;
        constexpr std::int64_t kSecondSentenceScoreMargin = 4'000;
        std::size_t sentence_count = expose_long_alternatives
                                         ? std::min<std::size_t>(2, full.size())
                                         : 1;
        if (!expose_long_alternatives && full.size() >= 2) {
            const auto& first = full[0];
            const auto& second = full[1];
            const auto exact_full_dictionary_word = [](const Candidate& candidate) {
                return candidate.match_kind == InputMatchKind::exact &&
                       !candidate.provisional_user_word &&
                       candidate.segment_count == 1 &&
                       utf8_character_count(candidate.text) ==
                           candidate.syllables.size();
            };
            const auto coherence_gap = first.coherence_score >= second.coherence_score
                ? first.coherence_score - second.coherence_score
                : second.coherence_score - first.coherence_score;
            const auto score_gap = first.score >= second.score
                ? first.score - second.score : second.score - first.score;
            const auto first_head = utf8_first_character(first.text);
            const auto second_head = utf8_first_character(second.text);
            const auto first_tail = std::string_view(first.text).substr(first_head.size());
            const auto second_tail = std::string_view(second.text).substr(second_head.size());
            // A homophone substitution followed by the identical suffix is a
            // generated variant, not a useful second sentence. A genuinely
            // different sentence may occupy the second slot when both scores
            // remain close enough.
            if ((exact_full_dictionary_word(first) &&
                 exact_full_dictionary_word(second)) ||
                (first_tail != second_tail &&
                 coherence_gap <= kSecondSentenceCoherenceMargin &&
                 score_gap <= kSecondSentenceScoreMargin))
                sentence_count = 2;
        }
        for (std::size_t index = 0; index < sentence_count; ++index)
            ordered.push_back(std::move(full[index]));

        if (retained_user_word && std::none_of(ordered.begin(), ordered.end(),
                [&](const Candidate& value) { return value.text == retained_user_word->text; })) {
            // Do not restore a permanent first-place preference or enlarge the
            // two-item long-candidate budget. The current leading result stays.
            if (ordered.size() == 2) ordered.back() = std::move(*retained_user_word);
            else ordered.push_back(std::move(*retained_user_word));
        }

        const auto leading_text = utf8_first_character(ordered.front().text);
        std::vector<Candidate> initial_characters;
        std::vector<Candidate> repeated_initial;
        std::vector<Candidate> matching_prefixes;
        std::vector<Candidate> other_prefixes;
        for (auto& candidate : prefixes) {
            if (candidate.syllables.size() == 1 &&
                utf8_character_count(candidate.text) == 1) {
                if (candidate.text == leading_text)
                    repeated_initial.push_back(std::move(candidate));
                else
                    initial_characters.push_back(std::move(candidate));
            } else if (candidate.syllables.size() == 2 &&
                       std::string_view(ordered.front().text).starts_with(
                           candidate.text)) {
                matching_prefixes.push_back(std::move(candidate));
            } else {
                other_prefixes.push_back(std::move(candidate));
            }
        }
        std::sort(initial_characters.begin(), initial_characters.end(), score_less);
        // Put the most useful partial commit from the winning sentence near
        // that sentence, followed by its first character. This keeps 这款/这
        // reachable without allowing every nested prefix to fill the page.
        const bool promoted_leading_word = !matching_prefixes.empty();
        if (promoted_leading_word) {
            ordered.push_back(std::move(matching_prefixes.front()));
            matching_prefixes.erase(matching_prefixes.begin());
        }
        if (promoted_leading_word && !repeated_initial.empty()) {
            ordered.push_back(std::move(repeated_initial.front()));
            repeated_initial.erase(repeated_initial.begin());
        }
        std::size_t initial_index = 0;
        if (!promoted_leading_word && !repeated_initial.empty()) {
            // The caller asks for one look-ahead candidate beyond its visible
            // page. Preserve the normal frequency order, but reserve the last
            // visible position for the character already used by the winning
            // sentence so it cannot disappear behind every homophone.
            const auto before_leading = limit > ordered.size() + 2
                ? std::min(initial_characters.size(),
                           limit - ordered.size() - 2)
                : std::size_t{0};
            while (initial_index < before_leading)
                ordered.push_back(std::move(initial_characters[initial_index++]));
            ordered.push_back(std::move(repeated_initial.front()));
            repeated_initial.erase(repeated_initial.begin());
        }
        while (initial_index < initial_characters.size())
            ordered.push_back(std::move(initial_characters[initial_index++]));
        for (auto& candidate : matching_prefixes)
            ordered.push_back(std::move(candidate));
        for (auto& candidate : other_prefixes)
            ordered.push_back(std::move(candidate));
    } else {
        for (auto& candidate : full) ordered.push_back(std::move(candidate));
        for (auto& candidate : prefixes) ordered.push_back(std::move(candidate));
    }
    // The pure-initial branch already groups complete abbreviation matches
    // ahead of prefix commits. The general word-length reorder intentionally
    // preserves only its first leading whole-input candidate, which would
    // recreate the old one-double-initial-candidate limit.
    if (!pure_initial_sequence && !multiword_input)
        prioritize_two_character_words(ordered);
    // Reserve one early dictionary slot for the strongest alternative exact
    // segmentation. Otherwise a valid high-frequency word such as min'gan can
    // be generated but buried behind every ming'an word and composition. Keep
    // the preferred split at index zero so the preview and first candidate stay
    // stable for inputs such as wan'an and nan'an.
    if (!pure_initial_sequence && !multiword_input)
        promote_alternative_exact_segmentation(ordered, full_input_bytes);
    if (partition_page_size != 0) {
        // A learned whole-input phrase belongs to the dictionary source. It
        // must not consume the only sentence slot and hide every independently
        // dictionary-backed coherent alternative to a selected composition.
        // Keep one such alternative without deleting or demoting learned data.
        if (!ordered.empty() && ordered.front().user_preferred() &&
            coherent_sentence_alternative &&
            std::none_of(ordered.begin(), ordered.end(), [&](const auto& candidate) {
                return candidate.text == coherent_sentence_alternative->text &&
                    candidate.consumed_input_bytes == coherent_sentence_alternative->consumed_input_bytes;
            }))
            ordered.push_back(std::move(*coherent_sentence_alternative));
        std::vector<Candidate> sentences;
        std::vector<Candidate> dictionary_words;
        std::vector<Candidate> compositions;
        std::vector<Candidate> single_characters;
        std::vector<Candidate> other;
        for (auto& candidate : ordered) {
            const auto characters = utf8_character_count(candidate.text);
            const bool sentence = candidate.consumed_input_bytes ==
                                      parsed.normalized_input.size() &&
                                  candidate.syllables.size() >= 3 &&
                                  characters == candidate.syllables.size() &&
                                  candidate.segment_count > 1;
            if (sentence)
                sentences.push_back(std::move(candidate));
            else if (characters >= 2 && candidate.segment_count == 1)
                dictionary_words.push_back(std::move(candidate));
            else if (characters >= 2)
                compositions.push_back(std::move(candidate));
            else if (characters == 1)
                single_characters.push_back(std::move(candidate));
            else
                other.push_back(std::move(candidate));
        }
        const auto composition_capacity = partition_page_size * 2;
        const auto composition_limit =
            dictionary_words.size() < composition_capacity
                ? composition_capacity - dictionary_words.size()
                : std::size_t{0};
        if (compositions.size() > composition_limit)
            compositions.resize(composition_limit);
        std::vector<Candidate> dictionary;
        dictionary.reserve(dictionary_words.size() + compositions.size());
        for (auto& candidate : dictionary_words)
            dictionary.push_back(std::move(candidate));
        for (auto& candidate : compositions)
            dictionary.push_back(std::move(candidate));
        std::stable_sort(dictionary.begin(), dictionary.end(),
            [full_input_bytes](const Candidate& left, const Candidate& right) {
                const auto complete_word = [full_input_bytes](const Candidate& candidate) {
                    return candidate.match_kind == InputMatchKind::exact &&
                           !candidate.provisional_user_word &&
                           candidate.segment_count == 1 &&
                           candidate.consumed_input_bytes == full_input_bytes &&
                           utf8_character_count(candidate.text) ==
                               candidate.syllables.size();
                };
                if (complete_word(left) != complete_word(right))
                    return complete_word(left);
                if (left.consumed_input_bytes != right.consumed_input_bytes)
                    return left.consumed_input_bytes > right.consumed_input_bytes;
                return score_less(left, right);
            });
        promote_alternative_exact_segmentation(dictionary, full_input_bytes);
        promote_full_candidate_heads(
            sentences, dictionary, full_input_bytes, single_characters);
        // Preserve every source in the bounded pool passed to Core. Appending
        // all sentences, then all dictionary words, and only then characters
        // meant the final `limit` truncation could erase entire groups. Core's
        // page arranger cannot reconstruct candidates it never receives.
        // Reserve at most two sentence slots and split the remaining pool with
        // the same dictionary-floor / character-ceiling rule used by visible
        // pages. A short source may borrow unused capacity from another source.
        ordered.clear();
        ordered.reserve(std::min(
            limit, sentences.size() + dictionary.size() +
                       single_characters.size() + other.size()));
        const auto sentence_count = std::min<std::size_t>(2, sentences.size());
        for (std::size_t index = 0; index < sentence_count; ++index)
            ordered.push_back(std::move(sentences[index]));

        const auto remaining = limit > ordered.size() ? limit - ordered.size() : 0;
        auto dictionary_count = std::min(remaining / 2, dictionary.size());
        auto single_count = std::min((remaining + 1) / 2, single_characters.size());
        auto unfilled = remaining - dictionary_count - single_count;
        const auto extra_dictionary = std::min(
            unfilled, dictionary.size() - dictionary_count);
        dictionary_count += extra_dictionary;
        unfilled -= extra_dictionary;
        single_count += std::min(
            unfilled, single_characters.size() - single_count);

        for (std::size_t index = 0; index < dictionary_count; ++index)
            ordered.push_back(std::move(dictionary[index]));
        for (std::size_t index = 0; index < single_count; ++index)
            ordered.push_back(std::move(single_characters[index]));
        for (auto& candidate : other) {
            if (ordered.size() == limit) break;
            ordered.push_back(std::move(candidate));
        }
    }
    if (ordered.size() > limit) ordered.resize(limit);
    if (include_model_alternatives && !model_seeds.empty()) {
        constexpr std::size_t kMaximumModelAlternatives = 64;
        constexpr std::size_t kAlternativesPerReading = 32;
        ordered.reserve(limit + kMaximumModelAlternatives);
        std::unordered_set<std::string_view> known_texts;
        known_texts.reserve(ordered.size() + kMaximumModelAlternatives);
        for (const auto& candidate : ordered)
            known_texts.insert(candidate.text);

        // Keep a small hidden pool of already generated whole-input variants
        // for sentence-level ranking. They never occupy the basic candidate
        // page, but allow the external language model to choose a coherent
        // homophone sequence that raw single-character frequency would prune.
        for (auto seed : model_seeds) {
            if (cancelled && cancelled()) return {};
            if (ordered.size() >= limit + kMaximumModelAlternatives) break;
            if (known_texts.contains(seed.text)) continue;
            seed.model_only = true;
            ordered.push_back(std::move(seed));
            known_texts.insert(ordered.back().text);
        }

        for (const auto& seed : model_seeds) {
            if (cancelled && cancelled()) return {};
            if (ordered.size() >= limit + kMaximumModelAlternatives) break;
            if (seed.syllables.size() < 5 ||
                utf8_character_count(seed.text) != seed.syllables.size())
                continue;
            const auto boundaries = utf8_boundaries(seed.text);
            std::size_t syllable_offset = 0;
            for (std::size_t segment_index = 0;
                 segment_index < seed.segment_lengths.size(); ++segment_index) {
                const auto segment_length = seed.segment_lengths[segment_index];
                const bool bridge_segment =
                    segment_length == 1 && segment_index > 0 &&
                    segment_index + 1 < seed.segment_lengths.size() &&
                    seed.segment_lengths[segment_index - 1] >= 2 &&
                    seed.segment_lengths[segment_index + 1] >= 2;
                if (bridge_segment && syllable_offset > 0 &&
                    syllable_offset + 1 < seed.syllables.size()) {
                    const std::array<std::string_view, 1> reading{
                        seed.syllables[syllable_offset]};
                    auto entries = lookup_cache.lookup(reading);
                    retain_frequent_entries(entries, kAlternativesPerReading);
                    for (const auto& entry : entries) {
                        if (utf8_character_count(entry.text) != 1) continue;
                        auto text = seed.text;
                        text.replace(boundaries[syllable_offset],
                                     boundaries[syllable_offset + 1] -
                                         boundaries[syllable_offset],
                                     entry.text);
                        if (known_texts.contains(text)) continue;
                        auto alternative = seed;
                        alternative.text = std::move(text);
                        alternative.model_only = true;
                        ordered.push_back(std::move(alternative));
                        known_texts.insert(ordered.back().text);
                        if (ordered.size() >= limit + kMaximumModelAlternatives)
                            break;
                    }
                }
                syllable_offset += segment_length;
                if (ordered.size() >= limit + kMaximumModelAlternatives) break;
            }
        }
    }
    if (metrics != nullptr)
        metrics->sort_us += static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::microseconds>(
                std::chrono::steady_clock::now() - sort_started).count());
    return ordered;
}

}  // namespace owo::engine

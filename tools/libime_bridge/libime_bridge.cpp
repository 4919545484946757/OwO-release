#include "owo/model/libime_bridge.h"
#include "owo/model/terminal_scoring.h"
#include "owo/model/medial_scoring.h"

#include <libime/core/languagemodel.h>
#include <libime/core/lattice.h>

#include <algorithm>
#include <cmath>
#include <cstring>
#include <exception>
#include <memory>
#include <limits>
#include <string_view>
#include <vector>

namespace {

struct BridgeState {
    explicit BridgeState(const char* path) : model(path) {}
    libime::LanguageModel model;
};

std::vector<std::size_t> utf8_boundaries(const std::string_view text) {
    std::vector<std::size_t> result{0};
    for (std::size_t offset = 1; offset < text.size(); ++offset) {
        if ((static_cast<unsigned char>(text[offset]) & 0xc0U) != 0x80U)
            result.push_back(offset);
    }
    result.push_back(text.size());
    return result;
}

std::size_t utf8_character_count(const std::string_view text) {
    return static_cast<std::size_t>(std::count_if(text.begin(), text.end(),
        [](const unsigned char byte) { return (byte & 0xc0U) != 0x80U; }));
}

struct TokenizationState {
    libime::State language_state{};
    float score{};
    bool inspected_singleton{};
    float score_after_singleton{};
};

TokenizationState score_best_tokenization(
    const libime::LanguageModel& model, const libime::State& initial_state,
    const std::string_view text, float* independent_ending = nullptr,
    std::size_t inspected_position = std::numeric_limits<std::size_t>::max()) {
    constexpr std::size_t kMaximumTokenCharacters = 4;
    constexpr std::size_t kTokenizationBeamWidth = 8;
    const auto boundaries = utf8_boundaries(text);
    if (text.empty() || boundaries.size() <= 1) return {initial_state, 0.0F};
    const auto character_count = boundaries.size() - 1;
    std::vector<std::vector<TokenizationState>> chart(character_count + 1);
    chart[0].push_back({initial_state, 0.0F});
    for (std::size_t begin = 0; begin < character_count; ++begin) {
        if (chart[begin].empty()) continue;
        const auto maximum_end =
            std::min(character_count, begin + kMaximumTokenCharacters);
        for (std::size_t end = begin + 1; end <= maximum_end; ++end) {
            const auto token = text.substr(boundaries[begin],
                                           boundaries[end] - boundaries[begin]);
            const auto index = model.index(token);
            // Unknown multi-character spans are never useful: the same unknown
            // cost can be represented by their individual characters while
            // retaining every real word boundary for the n-gram model.
            if (end > begin + 1 && model.isUnknown(index, token)) continue;
            const libime::WordNode word(token, index);
            for (const auto& previous : chart[begin]) {
                libime::State next_state{};
                const auto transition =
                    model.score(previous.language_state, word, next_state);
                chart[end].push_back(
                    {next_state, previous.score + transition,
                     previous.inspected_singleton || (begin == inspected_position && end == begin + 1),
                     begin == inspected_position && end == begin + 1
                         ? previous.score + transition : previous.score_after_singleton});
            }
        }
        for (std::size_t end = begin + 1; end <= maximum_end; ++end) {
            auto& states = chart[end];
            if (states.size() <= kTokenizationBeamWidth) continue;
            std::partial_sort(states.begin(),
                              states.begin() + kTokenizationBeamWidth,
                              states.end(),
                              [](const auto& left, const auto& right) {
                                  return left.score > right.score;
                              });
            states.resize(kTokenizationBeamWidth);
        }
    }
    const auto& completed = chart.back();
    if (completed.empty()) return {initial_state, 0.0F};
    const auto result = *std::max_element(completed.begin(), completed.end(),
                             [](const auto& left, const auto& right) {
                                 return left.score < right.score;
                             });
    if (independent_ending) {
        *independent_ending = result.score;
        if (character_count >= 3 && !chart[character_count - 1].empty()) {
            const auto tail = text.substr(boundaries[character_count - 1]);
            const libime::WordNode last_word(tail, model.index(tail));
            float best = -std::numeric_limits<float>::infinity();
            for (const auto& prefix : chart[character_count - 1]) {
                libime::State next{};
                best = std::max(best, prefix.score + model.score(prefix.language_state, last_word, next));
            }
            *independent_ending = best;
        }
    }
    return result;
}

bool calibrate_singleton_pair(const libime::LanguageModel& model, const char* context,
                          const char* const* candidates, const std::size_t count, float* scores,
                          const bool standalone_head = false) {
    // Sentence-initial priors dominate short function-word alternatives.
    // Calibrate only without external context; do not discount a real
    // application-context preference for a particular sentence head.
    if (count < 2 || (standalone_head && context && context[0])) return false;
    std::size_t anchor_index = count;
    for (std::size_t i = 0; i < count; ++i) {
        const auto length = utf8_character_count(candidates[i]);
        if (length >= 5 && length <= 32 &&
            (anchor_index == count || scores[i] > scores[anchor_index])) anchor_index = i;
    }
    if (anchor_index == count) return false;
    const std::string_view anchor(candidates[anchor_index]);
    const auto bounds = utf8_boundaries(anchor);
    const auto characters = bounds.size() - 1;
    // A bounded sentence-only comparison: either an independent head or a
    // medial singleton with two characters on each side; never a word-internal
    // or terminal substitution, nor a long-input rescan.
    if (characters < 5 || characters > 32) return false;
    std::size_t best = count, second = count, position = 0;
    for (std::size_t at = standalone_head ? 0 : 2;
         standalone_head ? at == 0 : at + 2 < characters; ++at) {
        const auto prefix = anchor.substr(0, bounds[at]);
        const auto suffix = anchor.substr(bounds[at + 1]);
        std::size_t first = count, runner = count;
        for (std::size_t i = 0; i < count; ++i) {
            const std::string_view text(candidates[i]);
            if (!text.starts_with(prefix) || !text.ends_with(suffix) ||
                text.size() <= prefix.size() + suffix.size()) continue;
            const auto middle = text.substr(prefix.size(), text.size() - prefix.size() - suffix.size());
            if (utf8_character_count(middle) != 1) continue;
            if (first == count || scores[i] > scores[first]) { runner = first; first = i; }
            else if (runner == count || scores[i] > scores[runner]) runner = i;
        }
        if (runner != count && (best == count || scores[first] > scores[best])) {
            best = first; second = runner; position = at;
        }
    }
    if (best == count || scores[best] <= scores[second] || scores[best] - scores[second] > 1.0F) return false;
    auto initial = model.beginState();
    if (context && context[0]) initial = score_best_tokenization(model, initial, context).language_state;
    const auto first = score_best_tokenization(model, initial, candidates[best], nullptr, position);
    const auto other = score_best_tokenization(model, initial, candidates[second], nullptr, position);
    if (!first.inspected_singleton || !other.inspected_singleton) return false;
    const auto prior = [&](const char* value) {
        const std::string_view text(value);
        const auto edges = utf8_boundaries(text);
        const auto character = text.substr(edges[position], edges[position + 1] - edges[position]);
        const auto index = model.index(character);
        if (model.isUnknown(index, character)) return 0.0F;
        libime::State next{};
        return model.score(model.nullState(), libime::WordNode(character, index), next);
    };
    const auto first_prior = prior(candidates[best]), other_prior = prior(candidates[second]);
    if (!std::isfinite(first_prior) || !std::isfinite(other_prior) ||
        first_prior >= 0 || other_prior >= 0) return false;
    if (owo::model::medial_pair_flips(scores[best], scores[second], first_prior, other_prior,
            first.score - first.score_after_singleton, other.score - other.score_after_singleton)) {
        std::swap(scores[best], scores[second]);
        return true;
    }
    return false;
}

void set_diagnostic(char* output, const size_t output_size, const std::string_view value) {
    if (output == nullptr || output_size == 0) return;
    const auto count = (std::min)(output_size - 1, value.size());
    std::memcpy(output, value.data(), count);
    output[count] = '\0';
}

}  // namespace

extern "C" uint32_t owo_libime_abi_version(void) { return OWO_LIBIME_BRIDGE_ABI_VERSION; }

extern "C" owo_libime_handle owo_libime_open(const char* model_path,
                                               char* diagnostic,
                                               const size_t diagnostic_size) {
    if (model_path == nullptr || model_path[0] == '\0') {
        set_diagnostic(diagnostic, diagnostic_size, "model path is empty");
        return nullptr;
    }
    try {
        auto state = std::make_unique<BridgeState>(model_path);
        set_diagnostic(diagnostic, diagnostic_size, {});
        return state.release();
    } catch (const std::exception& error) {
        set_diagnostic(diagnostic, diagnostic_size, error.what());
    } catch (...) {
        set_diagnostic(diagnostic, diagnostic_size, "unknown libime load failure");
    }
    return nullptr;
}

static int score_candidate(const owo_libime_handle handle,
                                 const char* context,
                                 const char* candidate,
                                 float* score,
                                 char* diagnostic,
                                 const size_t diagnostic_size, float* independent = nullptr) {
    if (handle == nullptr || candidate == nullptr || candidate[0] == '\0' || score == nullptr) {
        set_diagnostic(diagnostic, diagnostic_size, "invalid score request");
        return 0;
    }
    try {
        const auto& model = static_cast<const BridgeState*>(handle)->model;
        const std::string_view candidate_view(candidate);
        auto initial = model.beginState();
        if (context && context[0] != '\0')
            initial = score_best_tokenization(model, initial, context).language_state;
        *score = score_best_tokenization(model, initial, candidate_view, independent).score;
        set_diagnostic(diagnostic, diagnostic_size, {});
        return 1;
    } catch (const std::exception& error) {
        set_diagnostic(diagnostic, diagnostic_size, error.what());
    } catch (...) {
        set_diagnostic(diagnostic, diagnostic_size, "unknown libime scoring failure");
    }
    return 0;
}

extern "C" int owo_libime_score(const owo_libime_handle handle, const char* context,
                                 const char* candidate, float* score, char* diagnostic,
                                 const size_t diagnostic_size) {
    return score_candidate(handle, context, candidate, score, diagnostic, diagnostic_size);
}

extern "C" int owo_libime_score_batch(const owo_libime_handle handle,
                                        const char* context,
                                        const char* const* candidates,
                                        const std::size_t candidate_count,
                                        float* scores,
                                        char* diagnostic,
                                        const std::size_t diagnostic_size) {
    if (handle == nullptr || candidates == nullptr || candidate_count == 0 ||
        scores == nullptr) {
        set_diagnostic(diagnostic, diagnostic_size, "invalid batch score request");
        return 0;
    }
    bool paired_endings = false;
    std::string_view pair_prefix;
    std::size_t pair_characters = 0;
    if (candidate_count >= 2 && candidates[0] && candidates[1]) {
        const std::string_view first(candidates[0]), second(candidates[1]);
        const auto a = utf8_boundaries(first), b = utf8_boundaries(second);
        paired_endings = a.size() >= 4 && a.size() == b.size() && first != second &&
            first.substr(0, a[a.size()-2]) == second.substr(0, b[b.size()-2]);
        if (paired_endings) { pair_prefix = first.substr(0, a[a.size()-2]); pair_characters = a.size(); }
    }
    float independent[2]{};
    for (std::size_t index = 0; index < candidate_count; ++index) {
        if (candidates[index] == nullptr || candidates[index][0] == '\0' ||
            !score_candidate(handle, context, candidates[index], &scores[index],
                              diagnostic, diagnostic_size,
                              paired_endings && index < 2 ? &independent[index] : nullptr))
            return 0;
    }
    // The host's first two candidates are its bounded, quality-gated visible
    // alternatives. Calibrate only this same-prefix pair, and only if it
    // contains the model's original same-prefix-family winner. Swap original score slots
    // rather than penalizing the entire pool: unrelated hidden variants must
    // not rise merely because a plausible visible word was down-weighted.
    bool pair_has_family_winner = paired_endings;
    if (paired_endings) for (std::size_t index = 2; index < candidate_count; ++index) {
        const std::string_view text(candidates[index]);
        if (text.starts_with(pair_prefix) && utf8_boundaries(text).size() == pair_characters &&
            scores[index] > std::max(scores[0], scores[1])) { pair_has_family_winner = false; break; }
    }
    if (pair_has_family_winner &&
        owo::model::terminal_pair_flips(scores[0], scores[1], independent[0], independent[1]))
        std::swap(scores[0], scores[1]);
    try {
        const auto& model = static_cast<const BridgeState*>(handle)->model;
        if (!calibrate_singleton_pair(model, context, candidates, candidate_count, scores, true))
            calibrate_singleton_pair(model, context, candidates, candidate_count, scores);
    } catch (const std::exception& error) {
        set_diagnostic(diagnostic, diagnostic_size, error.what());
        return 0;
    } catch (...) {
        set_diagnostic(diagnostic, diagnostic_size, "unknown medial calibration failure");
        return 0;
    }
    set_diagnostic(diagnostic, diagnostic_size, {});
    return 1;
}

extern "C" void owo_libime_close(const owo_libime_handle handle) {
    delete static_cast<BridgeState*>(handle);
}

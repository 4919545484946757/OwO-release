#pragma once

#include "owo/engine/user_corpus.h"

#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

// Style profiles (learning layer 3) are an explainable summary of how the user
// writes, derived from the authorized corpus. Nothing here calls a model or
// leaves the machine; every number is computed from the sentences the user
// already agreed to keep.
//
// Scenes are inferred from the application executable, not from the text, so
// the same sentence is never classified twice and the mapping stays auditable.
enum class StyleScene : std::uint8_t { chat = 0, work = 1, general = 2 };

inline constexpr std::uint32_t kStyleSceneAll = (1U << 3) - 1;
// Only the newest sentences feed the profile, for the same reason as the
// collocation model: recent wording describes the user's current habits, and a
// bounded sample keeps the viewer responsive on a full corpus.
inline constexpr std::size_t kMaximumStyleProfileCorpusEntries = 6'000;
// Below this many sentences a formality score would be noise, so it is left
// unset and the report says the sample is too small.
inline constexpr std::size_t kMinimumStyleSampleSentences = 5;
inline constexpr std::size_t kMaximumStyleMarkers = 8;

struct StyleMarker {
    std::string text;
    std::uint32_t count{};
    bool operator==(const StyleMarker&) const = default;
};

// One explainable contribution to the formality score. The sum of the deltas
// plus the neutral base is the score, so a user can see why it is what it is.
struct StyleSignal {
    std::string label;
    int delta{};
    bool operator==(const StyleSignal&) const = default;
};

struct StyleSceneProfile {
    StyleScene scene{StyleScene::general};
    std::size_t sentence_count{};
    // Mean sentence length in code points.
    double average_characters{};
    // Share of sentences at or below 12 code points.
    double short_sentence_ratio{};
    // Share of sentences ending with terminal punctuation.
    double terminal_punctuation_ratio{};
    double exclamation_per_sentence{};
    double question_per_sentence{};
    double ellipsis_per_sentence{};
    double comma_per_sentence{};
    // Recurring colloquial markers, most frequent first.
    std::vector<StyleMarker> markers;
    // 0-100, or -1 when the scene has too few sentences to judge.
    int formality{-1};
    std::vector<StyleSignal> formality_signals;
    // Executables that contributed to this scene, most sentences first.
    std::vector<std::string> applications;
    bool operator==(const StyleSceneProfile&) const = default;
};

struct StyleProfile {
    std::size_t total_sentences{};
    std::vector<StyleSceneProfile> scenes;
    bool operator==(const StyleProfile&) const = default;
};

[[nodiscard]] StyleScene style_scene_for_application(std::string_view executable) noexcept;
[[nodiscard]] std::string_view style_scene_id(StyleScene scene) noexcept;
[[nodiscard]] std::string_view style_scene_name(StyleScene scene) noexcept;
[[nodiscard]] std::uint32_t style_scene_bit(StyleScene scene) noexcept;

/// Summarizes the given sentences per scene. Scenes disabled in `scene_mask`
/// are skipped, and scenes without sentences are omitted.
[[nodiscard]] StyleProfile build_style_profile(
    std::span<const UserCorpusEntry> entries,
    std::uint32_t scene_mask = kStyleSceneAll);

/// Human-readable Chinese report, one block per scene. Used by the corpus
/// shell and, later, by tone drafting to describe the user's habits to a model.
[[nodiscard]] std::string describe_style_profile(const StyleProfile& profile);
[[nodiscard]] std::string describe_style_scene(const StyleSceneProfile& scene);

}  // namespace owo::engine

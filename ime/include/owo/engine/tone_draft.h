#pragma once

#include "owo/engine/style_profile.h"
#include "owo/engine/user_corpus.h"

#include <cstddef>
#include <cstdint>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

// Tone drafting (learning layer 4) turns the authorized corpus into a bounded
// model request: a few of the user's own sentences from the same scene, an
// explainable style digest, and the exact instruction text. Everything here is
// local and side-effect free. Nothing is sent anywhere without an explicit
// model call made by the caller.
enum class ToneDraftMode : std::uint8_t {
    // Draft a reply to a message the user selected or pasted.
    reply = 0,
    // Continue an opening the user has already typed.
    continuation = 1,
};

inline constexpr std::size_t kDefaultToneExamples = 4;
inline constexpr std::size_t kMaximumToneExamples = 8;
// The selected message or written opening is also what the model receives, so
// it is bounded before any request is assembled.
inline constexpr std::size_t kMaximumToneMessageBytes = 1024;
// Model output beyond this is unlikely to be a usable reply and costs the user
// more to delete than to type.
inline constexpr std::size_t kMaximumToneDraftBytes = 1024;
inline constexpr std::size_t kMaximumToneDrafts = 3;
// A run this long that matches an example but not the current message means
// the model copied the example instead of the user's message.
inline constexpr std::size_t kMinimumCopiedExampleCodes = 12;
inline constexpr std::size_t kMaximumTonePromptBytes = 16 * 1024;
// Mirrors owo::config::kMaximumStylePreferencesBytes. The engine layer does not
// depend on the config layer, so a test pins the two together.
inline constexpr std::size_t kMaximumTonePreferencesBytes = 512;

// One retrieved sentence. `shared_units` is the number of distinct two-character
// sequences it shares with the current message, so the choice is auditable.
struct ToneExample {
    std::string text;
    std::uint32_t shared_units{};
    std::uint64_t entry_id{};
    bool operator==(const ToneExample&) const = default;
};

struct ToneDraftPlan {
    StyleScene scene{StyleScene::general};
    ToneDraftMode mode{ToneDraftMode::reply};
    // True only when the scene is enabled in the mask and has sentences.
    bool ready{};
    std::vector<ToneExample> examples;
    // One line per scene, for example "平均 9.2 字，短句 83%，常用口头语：吧、嗯".
    std::string style_digest;
    // Exactly what the caller may send to a model: style digest, examples,
    // the user's own preferences, the current message and the fact rule.
    std::string prompt;
    // Why no plan was produced; empty when ready.
    std::string diagnostic;
};

// Retrieves same-scene examples for `message`. `executable` selects the scene
// exactly as the style profile does. Sentences disabled by `scene_mask` are
// never used, so a scene switched off in settings also stops leaving this
// machine. Returns a plan with `ready == false` and a diagnostic when the
// message is empty, too long, invalid, or the scene has no allowed sentences.
[[nodiscard]] ToneDraftPlan build_tone_draft_plan(
    std::span<const UserCorpusEntry> entries, std::string_view message,
    std::string_view executable, std::uint32_t scene_mask,
    std::string_view preferences, ToneDraftMode mode = ToneDraftMode::reply,
    std::size_t maximum_examples = kDefaultToneExamples);

// Feedback hints (learning layer 5) ride along in the same request, after
// the examples and before the current message. Each line is already a full instruction
// sentence; the plan stores them so the exact prompt stays reproducible.
[[nodiscard]] ToneDraftPlan build_tone_draft_plan(
    std::span<const UserCorpusEntry> entries, std::string_view message,
    std::string_view executable, std::uint32_t scene_mask,
    std::string_view preferences, ToneDraftMode mode,
    std::size_t maximum_examples, std::span<const std::string> feedback_hints);

struct ToneDraftCheck {
    bool acceptable{};
    // Digit runs and Latin words in the draft that the current message does
    // not contain: time, price, names, links, codes.
    std::vector<std::string> unsupported_facts;
    // Longest run of characters copied from an example, when it is not part
    // of the current message.
    std::size_t copied_example_codes{};
    // Stable, user-facing explanation; empty when acceptable.
    std::string reason;
    bool operator==(const ToneDraftCheck&) const = default;
};

// Deterministic post-filter for "facts only from the current message". It
// catches new numbers and Latin words, text copied from an example, and empty,
// overlong or multi-line drafts. In continuation mode the draft must start
// with the user's opening, because it replaces the composition on commit.
//
// Chinese prose that invents a person or place without digits cannot be
// detected this way; the prompt rule and the preview remain the primary
// defence, and the documentation says so.
[[nodiscard]] ToneDraftCheck check_tone_draft(
    std::string_view draft, std::string_view message,
    std::span<const std::string> examples,
    ToneDraftMode mode = ToneDraftMode::reply);

struct ToneDraftCandidate {
    std::string text;
    bool operator==(const ToneDraftCandidate&) const = default;
};

struct ToneDraftSelection {
    std::vector<ToneDraftCandidate> candidates;
    // One line per dropped model candidate, for the preview UI and diagnostics.
    std::vector<std::string> rejected;
};

// Trims, bounds, de-duplicates and filters model output. Order is preserved,
// so the model's best candidate stays first. `drafts` may contain one reply
// per string or several replies separated by newlines.
[[nodiscard]] ToneDraftSelection filter_tone_drafts(
    std::span<const std::string> drafts, std::string_view message,
    std::span<const std::string> examples,
    ToneDraftMode mode = ToneDraftMode::reply,
    std::size_t maximum = kMaximumToneDrafts);

}  // namespace owo::engine

#pragma once

#include "owo/engine/style_profile.h"
#include "owo/engine/tone_draft.h"

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <functional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

// Feedback learning (learning layer 5) closes the loop around tone drafting:
// an adopted draft, a draft the user edited in the preview, a rejected one and
// an undo each become one bounded record. The records derive a small, readable
// list of phrase preferences that later prompts and candidate ordering use.
// Records never enter the authorized corpus and never affect ordinary input.
enum class ToneFeedbackKind : std::uint8_t {
    adopted = 0,
    edited = 1,
    rejected = 2,
    undone = 3,
};

[[nodiscard]] std::string_view tone_feedback_kind_id(ToneFeedbackKind kind) noexcept;
[[nodiscard]] bool parse_tone_feedback_kind(std::string_view id,
                                            ToneFeedbackKind& kind) noexcept;

// Old records are dropped first, so the feedback file stays small and a
// one-off expression ages out instead of accumulating forever.
inline constexpr std::size_t kMaximumToneFeedbackEntries = 500;
inline constexpr std::size_t kMaximumToneFeedbackTextBytes = 1024;
// Only local edits teach a correction ("把「好的」说成「好呀」"). A rewrite
// longer than this says nothing specific, so the record is kept for counting
// but no preference is derived from it.
inline constexpr std::size_t kMaximumToneCorrectionCodes = 16;
inline constexpr std::size_t kMaximumTonePhraseBytes = 96;
inline constexpr std::size_t kMaximumToneFeedbackPreferences = 64;
inline constexpr int kMaximumTonePreferenceWeight = 6;
// Prompt hints stay few; a long list would outweigh the actual message.
inline constexpr std::size_t kMaximumTonePromptHints = 3;
// A rejected whole sentence is not a phrase worth telling a model to avoid.
inline constexpr std::size_t kMaximumToneHintCodes = 12;

struct ToneFeedbackEntry {
    std::uint64_t id{};
    std::uint64_t created_at_ms{};
    ToneFeedbackKind kind{ToneFeedbackKind::adopted};
    std::string application;
    std::string draft;
    // What the user actually committed. Empty for rejection.
    std::string final_text;
    bool operator==(const ToneFeedbackEntry&) const = default;
};

struct ToneFeedbackIoResult {
    bool success{};
    bool recovered_from_backup{};
    std::string error;
};

/// Feedback records, stored like the corpus: checksum, atomic replace, a
/// backup, and DPAPI encryption for the current user on Windows.
class ToneFeedbackStore final {
public:
    /// A missing file and missing backup load as an empty history.
    [[nodiscard]] ToneFeedbackIoResult load(const std::filesystem::path& path);
    [[nodiscard]] ToneFeedbackIoResult save() const;

    /// Returns the assigned id, or 0 when the record was rejected (unknown
    /// kind, empty draft, oversized or invalid text). Rejected and undone
    /// records must carry a draft; adoption and edits carry the final text.
    std::uint64_t append(ToneFeedbackKind kind, std::string_view application,
                         std::string_view draft, std::string_view final_text,
                         std::uint64_t created_at_ms);
    // Changes the originating adoption/edit to an undo; deleted or mismatched
    // records cannot be recreated. A new id retires a receipt after one use.
    std::uint64_t undo(std::uint64_t id, std::string_view application,
        std::string_view draft, std::string_view final_text, std::uint64_t created_at_ms);
    bool erase(std::uint64_t id);
    std::size_t clear_all();

    [[nodiscard]] const std::vector<ToneFeedbackEntry>& entries() const noexcept {
        return entries_;
    }
    [[nodiscard]] std::size_t count(ToneFeedbackKind kind) const noexcept;

    [[nodiscard]] static ToneFeedbackIoResult modify(
        const std::filesystem::path& path,
        const std::function<void(ToneFeedbackStore&)>& change,
        bool reset_invalid = false);

private:
    std::filesystem::path path_;
    std::uint64_t next_id_{1};
    std::vector<ToneFeedbackEntry> entries_;
};

/// One learned phrase relation. `to` empty means a whole short phrasing:
/// positive weight reinforces it; negative weight avoids it. A nonempty `to`
/// describes an edited replacement for `from`.
/// Weight is clamped to +/-kMaximumTonePreferenceWeight and decays at the
/// bound, mirroring the user-frequency model.
struct TonePhrasePreference {
    std::string from;
    std::string to;
    int weight{};
    std::uint32_t observations{};
    std::uint64_t newest_id{};
    bool operator==(const TonePhrasePreference&) const = default;
};

struct ToneFeedbackProfile {
    std::size_t entry_count{};
    // Largest |weight| first, then newest; bounded by
    // kMaximumToneFeedbackPreferences.
    std::vector<TonePhrasePreference> preferences;
    bool operator==(const ToneFeedbackProfile&) const = default;
};

/// Derives preferences from records of one scene. `scene_mask` is honoured
/// like everywhere else, so a scene switched off teaches nothing.
[[nodiscard]] ToneFeedbackProfile build_tone_feedback_profile(
    std::span<const ToneFeedbackEntry> entries, StyleScene scene,
    std::uint32_t scene_mask = kStyleSceneAll);

/// The changed middle of an edit, in code points. Returns false when the two
/// texts are equal or the edit is not a local one, which means no correction
/// is learned.
[[nodiscard]] bool tone_feedback_correction(std::string_view draft,
                                            std::string_view final_text,
                                            std::string& from, std::string& to);

/// Prompt lines describing the strongest corrections and avoidances. Empty
/// when the profile has nothing specific enough to say.
[[nodiscard]] std::vector<std::string> describe_tone_feedback_hints(
    const ToneFeedbackProfile& profile);

/// Rank adjustment for one draft: preferred phrasings raise it, avoided ones
/// lower it. Zero when the profile is empty.
[[nodiscard]] int tone_feedback_adjustment(std::string_view draft,
                                           const ToneFeedbackProfile& profile);

/// Stable reorder by adjustment; ties keep the model's order, so the feedback
/// refines ranking without overturning it.
[[nodiscard]] ToneDraftSelection apply_tone_feedback(ToneDraftSelection selection,
                                                     const ToneFeedbackProfile& profile);

}  // namespace owo::engine

#pragma once

#include "owo/engine/candidate_generator.h"

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {

// Generative multimodal output starts from the same intent vocabulary the
// keyboard already understands. Only text-representable assets (emoji and
// kaomoji) can be committed through TSF today; image, animation, and audio
// assets are described here so a host UI can render or insert them later
// without redefining the engine contract.
enum class ExpressiveAssetKind : std::uint8_t {
    emoji,
    kaomoji,
    image,
    animation,
    audio,
};

struct ExpressiveAsset {
    ExpressiveAssetKind kind{ExpressiveAssetKind::emoji};
    // Insertion text for emoji/kaomoji. Empty for media assets, which are
    // referenced by resource instead of inlined into the document.
    std::string text;
    // Short user-facing label, for example "动态表情：墨仔瘫倒".
    std::string annotation;
    // Opaque resource identifier for media assets; never a raw URL.
    std::string resource;
    // Local assets stay on the machine. Remote generation is a separate,
    // explicit opt-in and is never performed by the engine.
    bool local_only{true};
};

class ExpressiveAssetSource {
public:
    virtual ~ExpressiveAssetSource() = default;
    // normalized_key is the whole preedit after ASCII lower-casing and
    // whitespace removal. Implementations must be bounded and side-effect free.
    [[nodiscard]] virtual std::vector<ExpressiveAsset> lookup(
        std::string_view normalized_key, std::size_t limit) const = 0;
};

// Built-in offline table covering common emotional intents in pinyin and in
// plain English. Nothing is sent anywhere; the table is compiled into the
// engine and bounded by kExpressiveMaximumAssets.
[[nodiscard]] const ExpressiveAssetSource& builtin_expressive_source();

// Normalizes the whole preedit the way expressive keys are matched.
[[nodiscard]] std::string expressive_lookup_key(std::string_view raw);

inline constexpr std::int64_t kExpressiveEmojiScore = 3'000;
inline constexpr std::int64_t kExpressiveCombinationScore = 2'900;
inline constexpr std::int64_t kExpressiveKaomojiScore = 2'600;
inline constexpr std::size_t kExpressiveMaximumAssets = 6;

// Whole-preedit expressive candidates. The key must match the complete
// preedit, so ordinary pinyin words are never decorated by accident. Returned
// candidates consume the full input and carry no source segmentation, keeping
// them interchangeable with dictionary candidates during commit.
[[nodiscard]] std::vector<Candidate> generate_expressive_candidates(
    std::string_view raw,
    const ExpressiveAssetSource& source = builtin_expressive_source(),
    std::size_t limit = 3);

// Multimodal combination: the decoded text of the whole preedit followed by
// one expressive asset ("累死😩"). It is only produced when the whole preedit
// is a single pinyin span that both decodes and matches an expressive key, so
// the text and the asset always describe the same intent.
[[nodiscard]] std::vector<Candidate> generate_combination_candidates(
    std::string_view text, std::string_view expressive_text,
    std::size_t consumed_bytes);

}  // namespace owo::engine

#pragma once

namespace owo::model {
// Discount only part of an isolated character's unigram prior when comparing
// near-tied sentences. The contextual LM score and a quarter of that prior
// remain; stronger continuation evidence is required separately.
[[nodiscard]] constexpr bool medial_pair_flips(
    float winner, float alternative, float winner_prior, float alternative_prior,
    float winner_continuation, float alternative_continuation) noexcept {
    constexpr float margin = 0.00001F;
    return winner > alternative && winner - alternative <= 1.0F &&
        alternative_continuation > winner_continuation + margin &&
        alternative - 0.75F * alternative_prior > winner - 0.75F * winner_prior + margin;
}
}  // namespace owo::model

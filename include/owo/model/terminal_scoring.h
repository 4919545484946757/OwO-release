#pragma once

namespace owo::model {
// Log-linear calibration: retain two parts contextual whole-word evidence
// and one part of the same text with a boundary before its final character.
// Both routes use the same preceding context. No language-specific tokens.
[[nodiscard]] constexpr float calibrated_terminal_score(float whole, float independent) noexcept {
    return whole + (independent - whole) / 3.0F;
}
[[nodiscard]] constexpr bool terminal_pair_flips(float first, float second,
                                                float independent_first, float independent_second) noexcept {
    const auto a = calibrated_terminal_score(first, independent_first);
    const auto b = calibrated_terminal_score(second, independent_second);
    constexpr float margin = 0.00001F;
    return (first > second && b > a + margin) || (second > first && a > b + margin);
}
}

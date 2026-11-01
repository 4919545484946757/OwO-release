#pragma once

#include <algorithm>

namespace owo::tsf {

// Use the visible content width, never the wider retained layered surface.
[[nodiscard]] inline float unicode_category_scroll_offset(
    float current, float delta, float total_width, float content_width,
    float horizontal_padding) noexcept {
    const float viewport = std::max(0.0F, content_width - 2.0F * horizontal_padding);
    return std::clamp(current + delta, 0.0F, std::max(0.0F, total_width - viewport));
}

} // namespace owo::tsf

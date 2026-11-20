#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <limits>
#include <span>
#include <vector>

namespace owo::tsf {

// Screen coordinates, independent of the candidate window's next geometry.
struct BackdropRect {
    std::int64_t left{}, top{}, right{}, bottom{};
    bool empty() const noexcept { return left >= right || top >= bottom; }
};

class BackdropSnapshot {
public:
    void clear() noexcept { bounds_ = {}; pixels_.clear(); }

    // Restore only areas covered by our currently visible surfaces. Never use
    // a blurred/tinted frame as the source of the next background snapshot.
    // If an occluded area is unknown, reject the capture before changing it.
    bool accept(BackdropRect bounds, std::span<std::uint32_t> frame,
                std::span<const BackdropRect> occluders) {
        if (bounds.empty()) return false;
        const auto width = static_cast<std::uint64_t>(bounds.right) - static_cast<std::uint64_t>(bounds.left);
        const auto height = static_cast<std::uint64_t>(bounds.bottom) - static_cast<std::uint64_t>(bounds.top);
        if (width > std::numeric_limits<std::size_t>::max() / height ||
            width * height != frame.size()) return false;
        for (const auto& occluder : occluders) {
            const auto overlap = intersect(bounds, occluder);
            if (!overlap.empty() && (pixels_.empty() || overlap.left < bounds_.left ||
                overlap.top < bounds_.top || overlap.right > bounds_.right ||
                overlap.bottom > bounds_.bottom)) return false;
        }
        for (const auto& occluder : occluders) {
            const auto overlap = intersect(bounds, occluder);
            if (overlap.empty()) continue;
            for (auto y = overlap.top; y < overlap.bottom; ++y) {
                const auto source = static_cast<std::size_t>(y - bounds_.top) *
                    static_cast<std::size_t>(bounds_.right - bounds_.left) +
                    static_cast<std::size_t>(overlap.left - bounds_.left);
                const auto destination = static_cast<std::size_t>(y - bounds.top) *
                    static_cast<std::size_t>(width) + static_cast<std::size_t>(overlap.left - bounds.left);
                std::copy_n(pixels_.begin() + source,
                    static_cast<std::size_t>(overlap.right - overlap.left), frame.begin() + destination);
            }
        }
        pixels_.assign(frame.begin(), frame.end());
        bounds_ = bounds;
        return true;
    }

private:
    static BackdropRect intersect(BackdropRect a, BackdropRect b) noexcept {
        return {std::max(a.left, b.left), std::max(a.top, b.top),
                std::min(a.right, b.right), std::min(a.bottom, b.bottom)};
    }
    BackdropRect bounds_{};
    std::vector<std::uint32_t> pixels_;
};

} // namespace owo::tsf

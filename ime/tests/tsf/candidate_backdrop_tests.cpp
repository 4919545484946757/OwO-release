#include "owo/tsf/backdrop_snapshot.h"

#include <array>
#include <iostream>
#include <limits>

namespace {
using owo::tsf::BackdropRect;
using owo::tsf::BackdropSnapshot;
constexpr std::uint32_t overlay = 0xffeeeeee;
std::uint32_t background(std::int64_t x, std::int64_t y) {
    return static_cast<std::uint32_t>((x + 128) * 257 + y + 128);
}
std::vector<std::uint32_t> capture(BackdropRect bounds, std::span<const BackdropRect> occluders = {}) {
    std::vector<std::uint32_t> result;
    for (auto y = bounds.top; y < bounds.bottom; ++y)
        for (auto x = bounds.left; x < bounds.right; ++x) {
            bool occluded = false;
            for (auto area : occluders)
                occluded |= x >= area.left && x < area.right && y >= area.top && y < area.bottom;
            result.push_back(occluded ? overlay : background(x, y));
        }
    return result;
}
}

int main() {
    BackdropSnapshot snapshot;
    // Clean initial canvas includes the body, shadow and Gaussian sampling pad.
    BackdropRect previous{-8, -8, 40, 24};
    auto initial = capture(previous);
    if (!snapshot.accept(previous, initial, {})) return 1;
    const std::array<BackdropRect, 4> modes{{{-8,-8,40,72}, {-12,-24,40,24},
                                           {-12,-24,52,56}, {-8,-8,40,24}}};
    for (int round = 0; round < 20; ++round) {
        for (const auto next : modes) {
            const std::array<BackdropRect, 2> surfaces{{
                {previous.left + 5, previous.top + 5, previous.right - 5, previous.bottom - 5},
                {previous.left + 1, previous.top + 1, previous.right - 1, previous.bottom - 1}}};
            auto contaminated = capture(next, surfaces);
            if (!snapshot.accept(next, contaminated, surfaces) || contaminated != capture(next)) return 2;
            previous = next;
        }
    }
    // Unknown self-occlusion must fail closed, without partially changing pixels
    // or overwriting the last trusted raw snapshot.
    const BackdropRect bounds{-16, -16, 48, 48};
    const std::array<BackdropRect, 1> unknown{{{-15,-15,-10,-10}}};
    auto contaminated = capture(bounds, unknown);
    const auto before = contaminated;
    if (snapshot.accept(bounds, contaminated, unknown) || contaminated != before) return 3;
    const std::array<BackdropRect, 1> known{{{0,0,8,8}}};
    contaminated = capture(bounds, known);
    if (!snapshot.accept(bounds, contaminated, known) || contaminated != capture(bounds)) return 4;
    snapshot.clear();
    contaminated = capture(bounds, known);
    if (snapshot.accept(bounds, contaminated, known)) return 5;
    // A hidden window may be recaptured cleanly, including a negative-origin monitor.
    const BackdropRect monitor{-120,-80,-40,-16};
    auto clean = capture(monitor);
    if (!snapshot.accept(monitor, clean, {})) return 6;
    const std::array<BackdropRect, 1> outside{{{0,0,10,10}}};
    if (!snapshot.accept(monitor, clean, outside)) return 7;
    std::array<std::uint32_t, 1> pixel{};
    if (snapshot.accept({}, pixel, {}) || snapshot.accept({0,0,2,2}, pixel, {}) ||
        snapshot.accept({std::numeric_limits<std::int64_t>::min(),0,
                         std::numeric_limits<std::int64_t>::max(),2}, pixel, {})) return 8;
    std::cout << "Clean backdrop reconstruction: expand/collapse, mode/position changes, shadow, invalidation and fail-closed checks passed.\n";
}

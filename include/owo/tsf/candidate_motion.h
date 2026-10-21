#pragma once

#include <algorithm>
#include <cmath>
#include <cstdint>

namespace owo::tsf {

// A bounded opacity/intensity transition for the candidate window's UI thread.
// No timer, thread or wall-clock reads are owned here: pass GetTickCount64 (or a
// deterministic test clock), update only while active(), then stop the timer.
class CandidateMotion {
public:
    explicit CandidateMotion(const float initial = 0.0F) noexcept {
        reset(initial);
    }

    // Clear an interaction when its target/window disappears. Non-finite reset
    // values fall back to zero; a reset starts a fresh monotonic clock epoch.
    void reset(const float initial = 0.0F) noexcept {
        value_ = std::isfinite(initial) ? std::clamp<double>(initial, 0.0, 1.0) : 0.0;
        target_ = value_;
        velocity_ = 0.0;
        last_tick_ = 0;
        clock_initialized_ = false;
        active_ = false;
    }

    // Retarget from the currently presented position AND velocity. Repeating
    // the same target every frame does not restart or prolong the transition.
    // response_ms is an approximate visual response, clamped to 1..1000 ms;
    // zero or animate=false gives an immediate accessible/reduced-motion state.
    // Non-finite targets are ignored rather than poisoning subsequent frames.
    void retarget(const float target, const std::uint64_t now_ms,
                  const std::uint32_t response_ms = 150,
                  const bool animate = true) noexcept {
        (void)update(now_ms);
        if (!std::isfinite(target)) return;
        target_ = std::clamp<double>(target, 0.0, 1.0);
        if (!animate || response_ms == 0) {
            settle();
            return;
        }
        omega_ = 8'000.0 / static_cast<double>(std::clamp(response_ms, 1U, 1'000U));
        active_ = std::abs(value_ - target_) > kSettleDistance ||
                  std::abs(velocity_) > omega_ * kSettleDistance;
        if (!active_) settle();
    }

    // Advance the analytic critically damped solution. It is independent of
    // frame rate, so a delayed WM_TIMER catches up without a simulation loop.
    // Backward timestamps do not rewind either the state or its clock epoch.
    [[nodiscard]] float update(const std::uint64_t now_ms) noexcept {
        if (!clock_initialized_) {
            last_tick_ = now_ms;
            clock_initialized_ = true;
            return value();
        }
        if (now_ms <= last_tick_) return value();
        const double elapsed = static_cast<double>(now_ms - last_tick_) / 1'000.0;
        last_tick_ = now_ms;
        if (!active_) return value();

        const double displacement = value_ - target_;
        const double decay = std::exp(-omega_ * elapsed);
        const double impulse = (velocity_ + omega_ * displacement) * elapsed;
        value_ = target_ + (displacement + impulse) * decay;
        velocity_ = (velocity_ - omega_ * impulse) * decay;
        if (!std::isfinite(value_) || !std::isfinite(velocity_)) {
            settle();
            return value();
        }
        // A direction reversal retains inertia but must never yield invalid
        // brush opacity or draw past the normalized interaction range.
        if (value_ < 0.0) {
            value_ = 0.0;
            velocity_ = std::max(0.0, velocity_);
        } else if (value_ > 1.0) {
            value_ = 1.0;
            velocity_ = std::min(0.0, velocity_);
        }
        if (std::abs(value_ - target_) <= kSettleDistance &&
            std::abs(velocity_) <= omega_ * kSettleDistance) settle();
        return value();
    }

    [[nodiscard]] float value() const noexcept { return static_cast<float>(value_); }
    [[nodiscard]] bool active() const noexcept { return active_; }

private:
    void settle() noexcept {
        value_ = target_;
        velocity_ = 0.0;
        active_ = false;
    }

    static constexpr double kSettleDistance = 0.001;
    double value_{};
    double target_{};
    double velocity_{};
    double omega_{8'000.0 / 150.0};
    std::uint64_t last_tick_{};
    bool clock_initialized_{};
    bool active_{};
};

}  // namespace owo::tsf

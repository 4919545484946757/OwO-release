#include "owo/tsf/candidate_motion.h"

#include <cmath>
#include <cstdint>
#include <iostream>
#include <limits>

namespace {
bool close(const float left, const float right, const float tolerance = 0.00001F) {
    return std::abs(left - right) <= tolerance;
}

int fail(const char* message) {
    std::cerr << message << '\n';
    return 1;
}
}

int main() {
    using owo::tsf::CandidateMotion;
    CandidateMotion hover;
    if (hover.value() != 0.0F || hover.active()) return fail("idle motion runs a timer");
    hover.retarget(1.0F, 1'000, 150);
    if (hover.value() != 0.0F || !hover.active()) return fail("retarget jumped immediately");
    const auto first = hover.update(1'016);
    const auto middle = hover.update(1'075);
    if (!(first > 0.0F && first < middle && middle < 1.0F))
        return fail("hover transition is not smooth and bounded");
    if (hover.update(1'300) != 1.0F || hover.active())
        return fail("settled motion did not stop exactly at its target");
    if (hover.update(1'400) != 1.0F || hover.active())
        return fail("idle update restarted settled motion");

    CandidateMotion redirected;
    redirected.retarget(1.0F, 1'000, 150);
    const auto presented = redirected.update(1'080);
    redirected.retarget(0.0F, 1'080, 150);
    if (!close(redirected.value(), presented)) return fail("direction reversal jumped");
    for (std::uint64_t tick = 1'081; tick <= 1'400; tick += 7) {
        const auto value = redirected.update(tick);
        if (!std::isfinite(value) || value < 0.0F || value > 1.0F)
            return fail("direction reversal produced invalid opacity");
    }
    if (redirected.update(1'500) != 0.0F || redirected.active())
        return fail("direction reversal did not settle");

    CandidateMotion repeated;
    repeated.retarget(1.0F, 2'000, 150);
    for (std::uint64_t tick = 2'010; tick <= 2'300; tick += 10)
        repeated.retarget(1.0F, tick, 150);
    if (repeated.value() != 1.0F || repeated.active())
        return fail("same-target redraws prolonged the animation");

    CandidateMotion rollback;
    rollback.retarget(1.0F, 3'000, 150);
    const auto before_rollback = rollback.update(3'060);
    if (!close(rollback.update(2'000), before_rollback))
        return fail("backward timestamp rewound the animation");
    rollback.retarget(0.0F, 2'100, 80);
    if (!close(rollback.value(), before_rollback))
        return fail("backward retarget rewound the animation");
    if (!close(rollback.update(3'060), before_rollback))
        return fail("backward retarget reset the clock epoch");
    rollback.retarget(1.0F, 3'061, 80, false);
    if (rollback.value() != 1.0F || rollback.active())
        return fail("reduced motion did not settle immediately");
    rollback.retarget(0.0F, 3'062, 0);
    if (rollback.value() != 0.0F || rollback.active())
        return fail("zero-duration motion was not immediate");

    CandidateMotion sparse;
    CandidateMotion frequent;
    sparse.retarget(1.0F, 4'000, 150);
    frequent.retarget(1.0F, 4'000, 150);
    for (std::uint64_t tick = 4'008; tick < 4'096; tick += 8) (void)frequent.update(tick);
    if (!close(sparse.update(4'096), frequent.update(4'096)))
        return fail("animation depends on timer frame rate");

    CandidateMotion finite;
    finite.reset(std::numeric_limits<float>::quiet_NaN());
    finite.retarget(std::numeric_limits<float>::infinity(), 5'000);
    if (!std::isfinite(finite.value()) || finite.active())
        return fail("invalid values poisoned motion state");
    finite.retarget(4.0F, 5'001);
    if (finite.update(std::numeric_limits<std::uint64_t>::max()) != 1.0F || finite.active())
        return fail("long suspension did not settle safely");
    finite.reset(-2.0F);
    if (finite.value() != 0.0F || finite.active()) return fail("reset did not clamp and stop");
    finite.retarget(1.0F, 1, 1'000);
    finite.reset();
    if (finite.value() != 0.0F || finite.active()) return fail("hide reset retained an animation");

    std::cout << "Candidate motion continuity, timing and reduced-motion checks passed\n";
}

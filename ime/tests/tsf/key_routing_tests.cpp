#include "owo/tsf/key_routing.h"

#include <iostream>
#include <string_view>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

using owo::tsf::DigitKeyAction;
using owo::tsf::DigitKeyState;

DigitKeyState composed(const std::size_t candidate_count,
                       const wchar_t previous_character,
                       const bool request_pending = false) {
    DigitKeyState state;
    state.has_composition = true;
    state.candidate_count = candidate_count;
    state.previous_character = previous_character;
    state.request_pending = request_pending;
    return state;
}

}  // namespace

int main() {
    // Candidate selection keeps priority over everything else.
    if (owo::tsf::route_digit_key(composed(5, L'i'), L'3') !=
        DigitKeyAction::select_candidate)
        return fail("third candidate was not selected");
    if (owo::tsf::route_digit_key(composed(0, L'i', true), L'9') !=
        DigitKeyAction::select_candidate)
        return fail("pending request did not keep the digit for selection");

    // Tone digits join the composition when no candidate occupies the slot.
    if (owo::tsf::route_digit_key(composed(0, L'i'), L'3') !=
        DigitKeyAction::join_composition)
        return fail("tone digit after a letter did not join the composition");
    if (owo::tsf::route_digit_key(composed(2, L'n'), L'5') !=
        DigitKeyAction::join_composition)
        return fail("neutral tone digit did not join the composition");
    if (owo::tsf::route_digit_key(composed(0, L'n'), L'1') !=
        DigitKeyAction::join_composition)
        return fail("first tone digit did not join the composition");

    // Digits beyond the candidate list still join the composition: `6-9` and
    // `0` are not tone digits, but they are mixed-input text.
    if (owo::tsf::route_digit_key(composed(0, L'i'), L'6') !=
        DigitKeyAction::join_composition)
        return fail("digit 6 did not join the composition");
    if (owo::tsf::route_digit_key(composed(0, L'i'), L'0') !=
        DigitKeyAction::join_composition)
        return fail("zero did not join the composition");
    if (owo::tsf::route_digit_key(composed(0, L'3'), L'2') !=
        DigitKeyAction::pass_through)
        return fail("digit after a digit joined the composition");
    if (owo::tsf::route_digit_key(composed(0, L'\''), L'2') !=
        DigitKeyAction::pass_through)
        return fail("digit after a separator joined the composition");
    {
        DigitKeyState no_composition;
        no_composition.candidate_count = 0;
        no_composition.previous_character = L'i';
        if (owo::tsf::route_digit_key(no_composition, L'3') !=
            DigitKeyAction::pass_through)
            return fail("compositon-less digit joined the composition");
    }
    // `0` never selects a candidate, even while a request is pending.
    if (owo::tsf::route_digit_key(composed(0, L'i', true), L'0') !=
        DigitKeyAction::join_composition)
        return fail("zero was routed as a candidate key");
    if (owo::tsf::route_digit_key(composed(0, L'i'), L'A') !=
        DigitKeyAction::pass_through)
        return fail("letter was routed as a digit");
    if (owo::tsf::route_digit_key(composed(0, 0), L'3') !=
        DigitKeyAction::pass_through)
        return fail("digit at the composition start joined the composition");

    if (!owo::tsf::is_composition_letter(L'z') ||
        !owo::tsf::is_composition_letter(L'Q') ||
        owo::tsf::is_composition_letter(L'1'))
        return fail("composition letter classification is wrong");

    std::cout << "key routing tests passed\n";
    return 0;
}

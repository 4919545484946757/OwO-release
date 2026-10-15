#include "owo/tsf/input_method_context.h"
#include <iostream>
int main() {
    using namespace owo::tsf;
    int failures{};
    const auto check = [&](bool ok, const char* label) { if (!ok) { ++failures; std::cerr << label << '\n'; } };
    check(evaluate_input_method_context({}) == InputMethodContextSafety::unknown, "unknown defaults to blocked");
    InputMethodContextFacts f;
    f.status_known = f.compartments_known = f.scope_known = true;
    check(evaluate_input_method_context(f) == InputMethodContextSafety::allowed, "known ordinary context");
    for (int n = 0; n < 3; ++n) {
        auto missing = f;
        if (n == 0) missing.status_known = false;
        if (n == 1) missing.compartments_known = false;
        if (n == 2) missing.scope_known = false;
        check(evaluate_input_method_context(missing) == InputMethodContextSafety::unknown, "missing metadata cannot authorize");
    }
    f.sensitive_scope = true;
    check(evaluate_input_method_context(f) == InputMethodContextSafety::sensitive, "sensitive scope blocked");
    f.sensitive_scope = false; f.disabled = true;
    check(evaluate_input_method_context(f) == InputMethodContextSafety::disabled, "keyboard disabled");
    f.disabled = false; f.empty_context = true;
    check(evaluate_input_method_context(f) == InputMethodContextSafety::disabled, "empty context");
    f.empty_context = false; f.read_only = true;
    check(evaluate_input_method_context(f) == InputMethodContextSafety::read_only, "read only");
    f.read_only = false; f.loading = true;
    check(evaluate_input_method_context(f) == InputMethodContextSafety::read_only, "loading");
    check(inspect_input_method_context(nullptr, 0) == InputMethodContextSafety::unknown, "null context blocked");
    const InputMethodWriteFacts write{1, 1, 2000, 2000, 1000, true, true, true, true, InputMethodContextSafety::allowed};
    check(input_method_write_allowed(write), "current safe write allowed");
    for (int n = 0; n < 10; ++n) {
        auto stale = write;
        switch (n) {
        case 0: stale.expected_generation = 0; break;
        case 1: stale.current_generation = 2; break;
        case 2: stale.current_deadline = 3000; break;
        case 3: stale.now = stale.expected_deadline; break;
        case 4: stale.scheme_selected = false; break;
        case 5: stale.write_pending = false; break;
        case 6: stale.same_context = false; break;
        case 7: stale.same_selection = false; break;
        case 8: stale.safety = InputMethodContextSafety::sensitive; break;
        case 9: stale.safety = InputMethodContextSafety::unknown; break;
        }
        check(!input_method_write_allowed(stale), "stale or unsafe write rejected");
    }
    return failures ? 1 : 0;
}

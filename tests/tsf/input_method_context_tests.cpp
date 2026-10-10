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
    return failures ? 1 : 0;
}

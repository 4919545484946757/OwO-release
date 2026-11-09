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
    check(native_scope_fallback_allowed(S_OK,VT_EMPTY), "absent native InputScope needs independent evidence");
    check(!native_scope_fallback_allowed(S_FALSE,VT_EMPTY), "mixed InputScope is not absence");
    check(!native_scope_fallback_allowed(S_OK,VT_UNKNOWN), "explicit unsupported scope cannot be ignored");
    check(!native_scope_fallback_allowed(E_FAIL,VT_UNKNOWN), "malformed failed scope cannot be ignored");
    const auto parent = CreateWindowExW(0, L"STATIC", L"", WS_OVERLAPPED, 0, 0, 100, 100,
                                        nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
    const auto plain = CreateWindowExW(0, L"EDIT", L"", WS_CHILD, 0, 0, 20, 20,
                                       parent, nullptr, GetModuleHandleW(nullptr), nullptr);
    const auto password = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | ES_PASSWORD, 0, 0, 20, 20,
                                          parent, nullptr, GetModuleHandleW(nullptr), nullptr);
    const auto readonly = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | ES_READONLY, 0, 0, 20, 20,
                                          parent, nullptr, GetModuleHandleW(nullptr), nullptr);
    check(parent && plain && password && readonly, "native control fixtures created");
    check(inspect_native_edit_scope(parent, plain) == InputMethodContextSafety::allowed,
          "ordinary OS edit supplies independent scope evidence");
    check(inspect_native_edit_scope(parent, password) == InputMethodContextSafety::sensitive,
          "native password remains denied without InputScope");
    check(inspect_native_edit_scope(parent, readonly) == InputMethodContextSafety::read_only,
          "native readonly remains denied");
    EnableWindow(plain, FALSE);
    check(inspect_native_edit_scope(parent, plain) == InputMethodContextSafety::read_only,
          "disabled native edit remains denied without InputScope");
    EnableWindow(plain, TRUE);
    check(inspect_native_edit_scope(plain, password) == InputMethodContextSafety::unknown,
          "unrelated view cannot authorize another control");
    check(inspect_native_edit_scope(parent, parent) == InputMethodContextSafety::unknown,
          "generic HWND remains unknown");
    check(inspect_native_edit_scope(nullptr, plain) == InputMethodContextSafety::unknown,
          "absent context view remains unknown");
    if (parent) DestroyWindow(parent);
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

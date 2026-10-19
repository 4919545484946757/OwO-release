#include "owo/tsf/input_method_context.h"
#include "owo/tsf/diagnostics.h"
#include <initguid.h>
#include <inputscope.h>
#include <wrl/client.h>

namespace owo::tsf {
using Microsoft::WRL::ComPtr;
bool sensitive_input_scope(LONG scope) noexcept {
    return scope == IS_PASSWORD || scope == IS_NUMERIC_PASSWORD ||
        scope == IS_NUMERIC_PIN || scope == IS_ALPHANUMERIC_PIN ||
        scope == IS_ALPHANUMERIC_PIN_SET || scope == IS_PRIVATE;
}
InputMethodContextSafety evaluate_input_method_context(const InputMethodContextFacts& f) noexcept {
    if (f.sensitive_scope) return InputMethodContextSafety::sensitive;
    if (f.disabled || f.empty_context) return InputMethodContextSafety::disabled;
    if (f.read_only || f.loading) return InputMethodContextSafety::read_only;
    if (!f.status_known || !f.compartments_known || !f.scope_known) return InputMethodContextSafety::unknown;
    return InputMethodContextSafety::allowed;
}
namespace {
bool flag(ITfCompartmentMgr* manager, REFGUID id, bool& value) {
    ComPtr<ITfCompartment> compartment;
    if (FAILED(manager->GetCompartment(id, &compartment)) || !compartment) return false;
    VARIANT result; VariantInit(&result);
    const auto hr = compartment->GetValue(&result);
    const bool known = SUCCEEDED(hr) && (result.vt == VT_EMPTY || result.vt == VT_I4 || result.vt == VT_BOOL);
    if (known) value = result.vt == VT_I4 ? result.lVal != 0 : result.vt == VT_BOOL && result.boolVal != VARIANT_FALSE;
    VariantClear(&result); return known;
}
}
InputMethodContextSafety inspect_input_method_context(ITfContext* context, TfEditCookie cookie, InputMethodContextFacts* observed) {
    if (!context) return InputMethodContextSafety::unknown;
    InputMethodContextFacts facts;
    const auto finish = [&] {
        if (observed) *observed = facts;
        return evaluate_input_method_context(facts);
    };
    TF_STATUS status{};
    facts.status_known = SUCCEEDED(context->GetStatus(&status));
    if (facts.status_known) {
        facts.read_only = (status.dwDynamicFlags & TS_SD_READONLY) != 0;
        facts.loading = (status.dwDynamicFlags & TS_SD_LOADING) != 0;
    }
    ComPtr<ITfCompartmentMgr> manager;
    if (SUCCEEDED(context->QueryInterface(IID_PPV_ARGS(&manager))))
        facts.compartments_known = flag(manager.Get(), GUID_COMPARTMENT_KEYBOARD_DISABLED, facts.disabled) &&
            flag(manager.Get(), GUID_COMPARTMENT_EMPTYCONTEXT, facts.empty_context);
    if (facts.disabled || facts.empty_context || facts.read_only || facts.loading)
        return finish();
    TF_SELECTION selection{}; ULONG count{};
    if (FAILED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count)) ||
        count != 1 || !selection.range) {
        if (selection.range) selection.range->Release();
        return finish();
    }
    ComPtr<ITfRange> range; range.Attach(selection.range);
    ComPtr<ITfReadOnlyProperty> property;
    const auto property_result = context->GetAppProperty(GUID_PROP_INPUTSCOPE, &property);
    trace_tsf_event("scope.property", property_result, property ? 1 : 0);
    if (FAILED(property_result) || !property) return finish();
    VARIANT value; VariantInit(&value);
    ComPtr<ITfInputScope> scope;
    const auto hr = property->GetValue(cookie, range.Get(), &value);
    trace_tsf_event("scope.value", hr, value.vt);
    if (SUCCEEDED(hr) && value.vt == VT_UNKNOWN && value.punkVal) {
        const auto query = value.punkVal->QueryInterface(IID_PPV_ARGS(&scope));
        trace_tsf_event("scope.interface", query);
    }
    VariantClear(&value);
    if (!scope) return finish();
    InputScope* scopes{}; UINT size{};
    const auto scopes_result = scope->GetInputScopes(&scopes, &size);
    trace_tsf_event("scope.list", scopes_result, static_cast<HRESULT>(size));
    if (SUCCEEDED(scopes_result) && scopes && size && size <= 128) {
        facts.scope_known = true;
        for (UINT n = 0; n < size; ++n) {
            if (sensitive_input_scope(scopes[n])) facts.sensitive_scope = true;
        }
    }
    CoTaskMemFree(scopes);
    return finish();
}
} // namespace owo::tsf

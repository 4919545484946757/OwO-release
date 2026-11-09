#include "owo/tsf/input_method_context.h"
#include "activation_trace.h"
#include "owo/compatibility/input_context.h"
#include "owo/compatibility/accessible_edit.h"
#include "owo/compatibility/uia_text_verifier.h"
#include <initguid.h>
#include <inputscope.h>
#include <wrl/client.h>

namespace owo::tsf {
using Microsoft::WRL::ComPtr;
InputMethodContextSafety inspect_native_edit_scope(HWND view, HWND focus) {
    const auto proof = compatibility::inspect_native_edit(view, focus, GetCurrentProcessId(), GetCurrentThreadId());
    using compatibility::Reason;
    if (proof.decision == compatibility::Decision::allowed) return InputMethodContextSafety::allowed;
    if (proof.reason == Reason::password) return InputMethodContextSafety::sensitive;
    // Preserve the legacy wrapper's denied category for existing callers.
    if (proof.reason == Reason::disabled) return InputMethodContextSafety::read_only;
    if (proof.reason == Reason::read_only) return InputMethodContextSafety::read_only;
    return InputMethodContextSafety::unknown;
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
InputContextResolution resolve_input_context(ITfContext* context, TfEditCookie cookie) {
    InputContextResolution resolved;
    InputMethodContextFacts facts;
    const auto finish = [&] {
        resolved.safety = evaluate_input_method_context(facts);
        using compatibility::Decision; using compatibility::Reason;
        switch (resolved.safety) {
        case InputMethodContextSafety::allowed: resolved.access = {Decision::allowed, Reason::verified}; break;
        case InputMethodContextSafety::sensitive: resolved.access = {Decision::denied, Reason::password}; break;
        case InputMethodContextSafety::disabled: resolved.access = {Decision::denied, Reason::disabled}; break;
        case InputMethodContextSafety::read_only: resolved.access = {Decision::denied, Reason::read_only}; break;
        default: resolved.access = {}; break;
        }
        activation_trace(resolved.access.decision == Decision::allowed ? "RESOLVE_ALLOWED" :
                         resolved.access.decision == Decision::denied ? "RESOLVE_DENIED" : "RESOLVE_UNKNOWN",
                         resolved.scope_result, context);
        return resolved;
    };
    if (!context) return finish();
    ComPtr<ITfContextView> active_view;
    GUITHREADINFO native_focus{sizeof(native_focus)};
    if (SUCCEEDED(context->GetActiveView(&active_view)) && active_view)
        (void)active_view->GetWnd(&resolved.view);
    if (GetGUIThreadInfo(GetCurrentThreadId(), &native_focus)) {
        resolved.focus = native_focus.hwndFocus;
        resolved.thread = GetWindowThreadProcessId(resolved.focus, &resolved.process);
    }
    // Explicit native restrictions take precedence even if a host mistakenly
    // exposes IS_DEFAULT for a password or readonly control.
    const auto native_restriction = compatibility::inspect_native_edit(
        resolved.view,resolved.focus,GetCurrentProcessId(),GetCurrentThreadId());
    if(native_restriction.reason==compatibility::Reason::password)facts.sensitive_scope=true;
    if(native_restriction.reason==compatibility::Reason::disabled)facts.disabled=true;
    if(native_restriction.reason==compatibility::Reason::read_only)facts.read_only=true;
    TF_STATUS status{};
    const auto status_result = context->GetStatus(&status);
    activation_trace("CONTEXT_STATUS", status_result, context);
    facts.status_known = SUCCEEDED(status_result);
    if (facts.status_known) {
        facts.read_only = facts.read_only || (status.dwDynamicFlags & TS_SD_READONLY) != 0;
        facts.loading = (status.dwDynamicFlags & TS_SD_LOADING) != 0;
    }
    ComPtr<ITfCompartmentMgr> manager;
    if (SUCCEEDED(context->QueryInterface(IID_PPV_ARGS(&manager)))) {
        bool compartment_disabled{};
        facts.compartments_known = flag(manager.Get(), GUID_COMPARTMENT_KEYBOARD_DISABLED, compartment_disabled) &&
            flag(manager.Get(), GUID_COMPARTMENT_EMPTYCONTEXT, facts.empty_context);
        facts.disabled = facts.disabled || compartment_disabled;
    }
    if (facts.disabled || facts.empty_context || facts.read_only || facts.loading)
        return finish();
    TF_SELECTION selection{}; ULONG count{};
    if (FAILED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count)) ||
        count != 1 || !selection.range) {
        if (selection.range) selection.range->Release();
        return finish();
    }
    ComPtr<ITfRange> range; range.Attach(selection.range);
    resolved.range_known = true;
    ComPtr<ITfReadOnlyProperty> property;
    const auto property_result = context->GetAppProperty(GUID_PROP_INPUTSCOPE, &property);
    activation_trace("CONTEXT_SCOPE_PROPERTY", property_result, context);
    VARIANT value; VariantInit(&value);
    ComPtr<ITfInputScope> scope;
    auto hr = property ? property->GetValue(cookie, range.Get(), &value) : property_result;
    // Some context owners expose their attributes directly rather than through
    // the range property. An explicit sensitive value still takes precedence.
    if ((hr == E_FAIL || hr == E_NOTIMPL || hr == E_NOINTERFACE) && value.vt == VT_EMPTY) {
        ComPtr<ITfContextOwner> owner;
        if (SUCCEEDED(context->QueryInterface(IID_PPV_ARGS(&owner))) && owner)
            hr = owner->GetAttribute(GUID_PROP_INPUTSCOPE, &value);
    }
    resolved.scope_result = hr;
    activation_trace("CONTEXT_SCOPE_VALUE", hr, context);
    if (value.vt == VT_EMPTY) activation_trace("CONTEXT_SCOPE_EMPTY", hr, context);
    if (SUCCEEDED(hr) && value.vt == VT_UNKNOWN && value.punkVal)
        (void)value.punkVal->QueryInterface(IID_PPV_ARGS(&scope));
    const bool native_fallback = native_scope_fallback_allowed(hr,value.vt);
    VariantClear(&value);
    if (!scope) {
        activation_trace("CONTEXT_SCOPE_UNAVAILABLE", E_NOINTERFACE, context);
        // RichEditD2DPT in current Notepad returns E_FAIL for InputScope. Do not
        // assume a default: independently verify the focused native text control.
        // Explicit scopes (including PRIVATE/password) never take this fallback.
        if (native_fallback) {
            ComPtr<ITfContextView> view; HWND window{};
            GUITHREADINFO focus{sizeof(focus)};
            if (SUCCEEDED(context->GetActiveView(&view)) && view && SUCCEEDED(view->GetWnd(&window)) &&
                GetGUIThreadInfo(GetCurrentThreadId(), &focus)) {
                const auto native = inspect_native_edit_scope(window, focus.hwndFocus);
                activation_trace(native == InputMethodContextSafety::allowed ? "CONTEXT_NATIVE_SCOPE_ALLOWED" :
                                 "CONTEXT_NATIVE_SCOPE_DENIED", S_OK, context);
                if (native == InputMethodContextSafety::sensitive) facts.sensitive_scope = true;
                if (native == InputMethodContextSafety::read_only) facts.read_only = true;
                if (native == InputMethodContextSafety::disabled) facts.disabled = true;
                facts.scope_known = native == InputMethodContextSafety::allowed;
                resolved.native_scope = facts.scope_known;
                if(native==InputMethodContextSafety::unknown && window && focus.hwndFocus &&
                   (window==focus.hwndFocus || IsChild(window,focus.hwndFocus))){
                    const auto accessible=compatibility::inspect_accessible_edit(focus.hwndFocus,GetCurrentProcessId(),GetCurrentThreadId());
                    activation_trace("CONTEXT_MSAA_RESULT",accessible.result,context);
                    activation_trace("CONTEXT_MSAA_ROLE",static_cast<HRESULT>(accessible.role),context);
                    activation_trace("CONTEXT_MSAA_STATE",static_cast<HRESULT>(accessible.state),context);
                    activation_trace("CONTEXT_IA2_RESULT",accessible.ia2_result,context);
                    facts.scope_known=accessible.resolution.decision==compatibility::Decision::allowed;
                    if(accessible.resolution.reason==compatibility::Reason::password)facts.sensitive_scope=true;
                    if(accessible.resolution.reason==compatibility::Reason::read_only)facts.read_only=true;
                    if(accessible.resolution.reason==compatibility::Reason::disabled)facts.disabled=true;
                    if(accessible.resolution.decision==compatibility::Decision::unknown && compatibility::current_text_verifier){
                        const auto proof=compatibility::current_text_verifier->snapshot(focus.hwndFocus,GetCurrentProcessId(),GetCurrentThreadId());
                        activation_trace("CONTEXT_UIA_METADATA",proof.result,context);
                        resolved.pending_verification=proof.result==E_PENDING;
                        facts.scope_known=proof.access.decision==compatibility::Decision::allowed;
                        if(proof.access.reason==compatibility::Reason::password)facts.sensitive_scope=true;
                        if(proof.access.reason==compatibility::Reason::read_only)facts.read_only=true;
                        if(proof.access.reason==compatibility::Reason::disabled)facts.disabled=true;
                    }
                }
            }
        }
        return finish();
    }
    InputScope* scopes{}; UINT size{};
    if (SUCCEEDED(scope->GetInputScopes(&scopes, &size)) && scopes && size && size <= 128) {
        facts.scope_known = true;
        for (UINT n = 0; n < size; ++n) {
            if (scopes[n] == IS_PASSWORD || scopes[n] == IS_NUMERIC_PASSWORD || scopes[n] == IS_NUMERIC_PIN ||
                scopes[n] == IS_PRIVATE) facts.sensitive_scope = true;
        }
    }
    CoTaskMemFree(scopes);
    return finish();
}
InputMethodContextSafety inspect_input_method_context(ITfContext* context, TfEditCookie cookie) {
    return resolve_input_context(context, cookie).safety;
}
} // namespace owo::tsf

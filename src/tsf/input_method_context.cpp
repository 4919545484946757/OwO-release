#include "owo/tsf/input_method_context.h"
#include "activation_trace.h"
#include <initguid.h>
#include <inputscope.h>
#include <wrl/client.h>

namespace owo::tsf {
using Microsoft::WRL::ComPtr;
InputMethodContextSafety inspect_native_edit_scope(HWND view, HWND focus) {
    if (!view || !focus || !IsWindow(view) || !IsWindow(focus) ||
        (view != focus && !IsChild(view, focus))) return InputMethodContextSafety::unknown;
    DWORD pid{};
    if (GetWindowThreadProcessId(view, &pid) != GetCurrentThreadId() || pid != GetCurrentProcessId())
        return InputMethodContextSafety::unknown;
    if (GetWindowThreadProcessId(focus, &pid) != GetCurrentThreadId() || pid != GetCurrentProcessId())
        return InputMethodContextSafety::unknown;
    wchar_t name[128]{};
    if (!GetClassNameW(focus, name, 128) ||
        (_wcsicmp(name, L"Edit") != 0 && _wcsnicmp(name, L"RichEdit", 8) != 0))
        return InputMethodContextSafety::unknown;
    const auto style = GetWindowLongPtrW(focus, GWL_STYLE);
    if (style & ES_PASSWORD) return InputMethodContextSafety::sensitive;
    if ((style & ES_READONLY) || !IsWindowEnabled(focus)) return InputMethodContextSafety::read_only;
    return InputMethodContextSafety::allowed;
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
InputMethodContextSafety inspect_input_method_context(ITfContext* context, TfEditCookie cookie) {
    if (!context) return InputMethodContextSafety::unknown;
    InputMethodContextFacts facts;
    TF_STATUS status{};
    const auto status_result = context->GetStatus(&status);
    activation_trace("CONTEXT_STATUS", status_result, context);
    facts.status_known = SUCCEEDED(status_result);
    if (facts.status_known) {
        facts.read_only = (status.dwDynamicFlags & TS_SD_READONLY) != 0;
        facts.loading = (status.dwDynamicFlags & TS_SD_LOADING) != 0;
    }
    ComPtr<ITfCompartmentMgr> manager;
    if (SUCCEEDED(context->QueryInterface(IID_PPV_ARGS(&manager))))
        facts.compartments_known = flag(manager.Get(), GUID_COMPARTMENT_KEYBOARD_DISABLED, facts.disabled) &&
            flag(manager.Get(), GUID_COMPARTMENT_EMPTYCONTEXT, facts.empty_context);
    if (facts.disabled || facts.empty_context || facts.read_only || facts.loading)
        return evaluate_input_method_context(facts);
    TF_SELECTION selection{}; ULONG count{};
    if (FAILED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &count)) ||
        count != 1 || !selection.range) {
        if (selection.range) selection.range->Release();
        return evaluate_input_method_context(facts);
    }
    ComPtr<ITfRange> range; range.Attach(selection.range);
    ComPtr<ITfReadOnlyProperty> property;
    const auto property_result = context->GetAppProperty(GUID_PROP_INPUTSCOPE, &property);
    activation_trace("CONTEXT_SCOPE_PROPERTY", property_result, context);
    if (FAILED(property_result) || !property)
        return evaluate_input_method_context(facts);
    VARIANT value; VariantInit(&value);
    ComPtr<ITfInputScope> scope;
    const auto hr = property->GetValue(cookie, range.Get(), &value);
    activation_trace("CONTEXT_SCOPE_VALUE", hr, context);
    if (value.vt == VT_EMPTY) activation_trace("CONTEXT_SCOPE_EMPTY", hr, context);
    if (SUCCEEDED(hr) && value.vt == VT_UNKNOWN && value.punkVal)
        (void)value.punkVal->QueryInterface(IID_PPV_ARGS(&scope));
    VariantClear(&value);
    if (!scope) {
        activation_trace("CONTEXT_SCOPE_UNAVAILABLE", E_NOINTERFACE, context);
        // RichEditD2DPT in current Notepad returns E_FAIL for InputScope. Do not
        // assume a default: independently verify the focused native text control.
        // Explicit scopes (including PRIVATE/password) never take this fallback.
        if (hr == E_FAIL || hr == E_NOTIMPL) {
            ComPtr<ITfContextView> view; HWND window{};
            GUITHREADINFO focus{sizeof(focus)};
            if (SUCCEEDED(context->GetActiveView(&view)) && view && SUCCEEDED(view->GetWnd(&window)) &&
                GetGUIThreadInfo(GetCurrentThreadId(), &focus)) {
                const auto native = inspect_native_edit_scope(window, focus.hwndFocus);
                activation_trace(native == InputMethodContextSafety::allowed ? "CONTEXT_NATIVE_SCOPE_ALLOWED" :
                                 "CONTEXT_NATIVE_SCOPE_DENIED", S_OK, context);
                if (native == InputMethodContextSafety::sensitive) facts.sensitive_scope = true;
                if (native == InputMethodContextSafety::read_only) facts.read_only = true;
                facts.scope_known = native == InputMethodContextSafety::allowed;
            }
        }
        return evaluate_input_method_context(facts);
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
    return evaluate_input_method_context(facts);
}
} // namespace owo::tsf

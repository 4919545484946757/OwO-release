#pragma once
#include "owo/compatibility/input_context.h"
#include <oleacc.h>
#include <wrl/client.h>
#include <utility>
#include <servprov.h>
#include "owo/compatibility/ia2/Accessible2.h"

namespace owo::compatibility {
// MSAA state is a typed bitmask, not a default inferred from a failed query.
[[nodiscard]] inline Resolution evaluate_accessible_edit(HRESULT role_result, const VARIANT& role,
                                                        HRESULT state_result, const VARIANT& state) noexcept {
    if(role_result!=S_OK || state_result!=S_OK || role.vt!=VT_I4 || state.vt!=VT_I4)return {};
    const auto flags=static_cast<DWORD>(state.lVal);
    if(flags&STATE_SYSTEM_PROTECTED)return {Decision::denied,Reason::password};
    if(flags&STATE_SYSTEM_READONLY)return {Decision::denied,Reason::read_only};
    if(flags&(STATE_SYSTEM_UNAVAILABLE|STATE_SYSTEM_INVISIBLE|STATE_SYSTEM_OFFSCREEN))return {Decision::denied,Reason::disabled};
    return evaluate_edit({true,(flags&STATE_SYSTEM_FOCUSED)!=0,
        role.lVal==ROLE_SYSTEM_TEXT || role.lVal==ROLE_SYSTEM_DOCUMENT,true,
        (flags&(STATE_SYSTEM_UNAVAILABLE|STATE_SYSTEM_INVISIBLE|STATE_SYSTEM_OFFSCREEN))==0,
        (flags&STATE_SYSTEM_PROTECTED)!=0,(flags&STATE_SYSTEM_READONLY)!=0});
}
struct AccessibleEditEvidence {
    Resolution resolution{};
    HRESULT result{E_FAIL};
    LONG role{}, state{};
    HRESULT ia2_result{E_NOINTERFACE};
    LONG ia2_states{}, ia2_id{};
    bool ia2_verified{};
};
// No names, values, document text, or application-specific object models.
// Resolve the accessible focus chain only within the verified native focus HWND.
[[nodiscard]] inline AccessibleEditEvidence inspect_accessible_edit(HWND focus,DWORD pid,DWORD thread) {
    AccessibleEditEvidence result;
    DWORD actual{};
    if(!focus || !pid || !thread || GetWindowThreadProcessId(focus,&actual)!=thread || actual!=pid)return result;
    struct Library { HMODULE value{LoadLibraryExW(L"oleacc.dll",nullptr,LOAD_LIBRARY_SEARCH_SYSTEM32)};
        ~Library(){if(value)FreeLibrary(value);} } library;
    if(!library.value)return result;
    using FromWindow=HRESULT(WINAPI*)(HWND,DWORD,REFIID,void**);
    using FromAccessible=HRESULT(WINAPI*)(IAccessible*,HWND*);
    const auto from_window=reinterpret_cast<FromWindow>(GetProcAddress(library.value,"AccessibleObjectFromWindow"));
    const auto from_accessible=reinterpret_cast<FromAccessible>(GetProcAddress(library.value,"WindowFromAccessibleObject"));
    if(!from_window || !from_accessible)return result;
    Microsoft::WRL::ComPtr<IAccessible> object;
    result.result=from_window(focus,static_cast<DWORD>(OBJID_CLIENT),__uuidof(IAccessible),reinterpret_cast<void**>(object.GetAddressOf()));
    if(FAILED(result.result)||!object)return result;
    VARIANT child{};child.vt=VT_I4;child.lVal=CHILDID_SELF;
    for(unsigned depth=0;depth<8;++depth){
        VARIANT selected{};
        const auto selected_result=object->get_accFocus(&selected);
        if(selected_result!=S_OK){VariantClear(&selected);break;}
        if(selected.vt==VT_DISPATCH && selected.pdispVal){
            Microsoft::WRL::ComPtr<IAccessible> next;
            const auto queried=selected.pdispVal->QueryInterface(IID_PPV_ARGS(&next));
            VariantClear(&selected);
            if(FAILED(queried)||!next)return result;
            object=std::move(next);
        } else if(selected.vt==VT_I4){child.lVal=selected.lVal;VariantClear(&selected);break;}
        else {VariantClear(&selected);return result;}
        if(depth==7)return result;
    }
    HWND window{};
    result.result=from_accessible(object.Get(),&window);
    if(FAILED(result.result)||!window || (window!=focus && !IsChild(focus,window)))return result;
    VARIANT role{},state{};
    const auto role_result=object->get_accRole(child,&role);
    const auto state_result=object->get_accState(child,&state);
    result.result=state_result;
    if(role.vt==VT_I4)result.role=role.lVal;
    if(state.vt==VT_I4)result.state=state.lVal;
    result.resolution=evaluate_accessible_edit(role_result,role,result.result,state);
    VariantClear(&role);VariantClear(&state);
    if(result.resolution.decision!=Decision::denied && role_result==S_OK && state_result==S_OK &&
       result.state && child.lVal==CHILDID_SELF){
        Microsoft::WRL::ComPtr<IServiceProvider> services;
        Microsoft::WRL::ComPtr<IAccessible2> rich;
        result.ia2_result=object.As(&services);
        if(SUCCEEDED(result.ia2_result))result.ia2_result=services->QueryService(__uuidof(IAccessible2),IID_PPV_ARGS(&rich));
        if(FAILED(result.ia2_result))result.ia2_result=object.As(&rich);
        if(SUCCEEDED(result.ia2_result)&&rich){
            HWND owner{};
            result.ia2_result=rich->get_states(&result.ia2_states);
            const auto owner_result=rich->get_windowHandle(&owner);
            const auto id_result=rich->get_uniqueID(&result.ia2_id);
            const auto flags=static_cast<DWORD>(result.state);
            const bool valid=result.ia2_result==S_OK && owner_result==S_OK && id_result==S_OK &&
                result.ia2_id && owner && (owner==focus || IsChild(focus,owner)) &&
                (result.ia2_states&(IA2_STATE_DEFUNCT|IA2_STATE_STALE))==0;
            result.resolution=evaluate_edit({valid,(flags&STATE_SYSTEM_FOCUSED)!=0,
                (result.ia2_states&IA2_STATE_EDITABLE)!=0,true,
                (flags&(STATE_SYSTEM_UNAVAILABLE|STATE_SYSTEM_INVISIBLE|STATE_SYSTEM_OFFSCREEN))==0,
                (flags&STATE_SYSTEM_PROTECTED)!=0,(flags&STATE_SYSTEM_READONLY)!=0});
            result.ia2_verified=result.resolution.decision==Decision::allowed;
        }
    }
    GUITHREADINFO current{sizeof(current)};
    if(!GetGUIThreadInfo(thread,&current) || current.hwndFocus!=focus)
        {result.resolution={Decision::denied,Reason::focus_mismatch};result.ia2_verified=false;}
    return result;
}
}

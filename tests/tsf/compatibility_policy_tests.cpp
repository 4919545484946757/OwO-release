#include "owo/compatibility/input_context.h"
#include "owo/compatibility/accessible_edit.h"
#include "owo/compatibility/uia_text_verifier.h"
#include <iostream>
#include <future>
#include <thread>
int main() {
    using namespace owo::compatibility;
    int failures{};
    auto check = [&](bool value, const char* label) { if (!value) { ++failures; std::cerr << label << '\n'; } };
    check(evaluate_edit({}).decision == Decision::unknown, "missing evidence is unknown");
    const auto handle=reinterpret_cast<HWND>(static_cast<UINT_PTR>(1));
    UiaTextVerifier::Proof sample{handle,10,20,1000,1,{Decision::allowed,Reason::verified},S_OK};
    check(UiaTextVerifier::current(sample,handle,10,20,1,1050),"current UIA proof identity");
    check(!UiaTextVerifier::current(sample,handle,10,20,1,1151),"expired UIA proof rejected");
    check(!UiaTextVerifier::current(sample,handle,10,20,2,1050),"old UIA revision rejected");
    check(!UiaTextVerifier::current(sample,handle,11,20,1,1050),"wrong UIA process rejected");
    check(!UiaTextVerifier::current(sample,handle,10,21,1,1050),"wrong UIA thread rejected");
    check(!UiaTextVerifier::current(sample,nullptr,10,20,1,1050),"wrong UIA focus rejected");
    check(!UiaTextVerifier::current(sample,handle,10,20,1,999),"invalid UIA clock rejected");
    VARIANT role{}, state{}; role.vt=state.vt=VT_I4;
    role.lVal=ROLE_SYSTEM_TEXT; state.lVal=STATE_SYSTEM_FOCUSED|STATE_SYSTEM_FOCUSABLE;
    check(evaluate_accessible_edit(S_OK,role,S_OK,state).decision==Decision::allowed,"focused MSAA writable text supplies independent metadata");
    for(auto restriction:{STATE_SYSTEM_PROTECTED,STATE_SYSTEM_READONLY,STATE_SYSTEM_UNAVAILABLE,STATE_SYSTEM_INVISIBLE}){
        state.lVal=STATE_SYSTEM_FOCUSED|restriction;
        check(evaluate_accessible_edit(S_OK,role,S_OK,state).decision!=Decision::allowed,"MSAA unsafe state rejected");
    }
    state.lVal=STATE_SYSTEM_FOCUSED;
    check(evaluate_accessible_edit(E_FAIL,role,S_OK,state).decision==Decision::unknown,"MSAA role failure is not evidence");
    check(evaluate_accessible_edit(S_OK,role,E_FAIL,state).decision==Decision::unknown,"MSAA state failure is not evidence");
    role.lVal=ROLE_SYSTEM_GROUPING;
    check(evaluate_accessible_edit(S_OK,role,S_OK,state).decision!=Decision::allowed,"generic MSAA grouping never authorizes");
    state.lVal=STATE_SYSTEM_FOCUSED|STATE_SYSTEM_PROTECTED;
    check(evaluate_accessible_edit(S_OK,role,S_OK,state).reason==Reason::password,"explicit MSAA password overrides unknown role and all fallback providers");
    state.lVal=STATE_SYSTEM_FOCUSED|STATE_SYSTEM_READONLY;
    check(evaluate_accessible_edit(S_OK,role,S_OK,state).reason==Reason::read_only,"explicit MSAA readonly overrides unknown role");
    role.lVal=ROLE_SYSTEM_TEXT; state.lVal=0;
    check(evaluate_accessible_edit(S_OK,role,S_OK,state).decision!=Decision::allowed,"unfocused MSAA text rejected");
    state.vt=VT_EMPTY;
    check(evaluate_accessible_edit(S_OK,role,S_OK,state).decision==Decision::unknown,"absent MSAA state is unknown");
    EditEvidence e{true, true, true, true, true, false, false};
    check(evaluate_edit(e).decision == Decision::allowed, "verified writable focused edit");
    e.password = true;
    check(evaluate_edit(e).reason == Reason::password, "password denied");
    e.password = false; e.read_only = true;
    check(evaluate_edit(e).decision == Decision::denied, "readonly denied");
    e.read_only = false; e.enabled = false;
    check(evaluate_edit(e).reason == Reason::disabled, "disabled denied");
    e.enabled = true; e.focus_matches = false;
    check(evaluate_edit(e).reason == Reason::focus_mismatch, "stale focus denied");
    e.focus_matches = true; e.properties_known = false;
    check(evaluate_edit(e).decision == Decision::unknown, "unknown readonly or password never authorizes");
    auto parent = CreateWindowExW(0, L"STATIC", L"", WS_OVERLAPPED, 0, 0, 100, 100, nullptr, nullptr, nullptr, nullptr);
    auto edit = CreateWindowExW(0, L"EDIT", L"", WS_CHILD, 0, 0, 20, 20, parent, nullptr, nullptr, nullptr);
    check(parent && edit, "real native fixtures");
    WNDCLASSW superclass{};
    check(GetClassInfoW(nullptr,L"EDIT",&superclass)!=FALSE,"system edit class metadata");
    superclass.hInstance=GetModuleHandleW(nullptr);superclass.lpszClassName=L"OwOCompatibilityEditSubclass";
    check(RegisterClassW(&superclass)!=0,"register real edit superclass fixture");
    auto subclass=CreateWindowW(superclass.lpszClassName,L"",WS_CHILD,0,0,20,20,parent,nullptr,superclass.hInstance,nullptr);
    check(inspect_native_edit(parent,subclass,GetCurrentProcessId(),GetCurrentThreadId()).decision==Decision::allowed,
          "native edit superclass identity is not its framework class name");
    check(inspect_native_edit(parent, edit, GetCurrentProcessId(), GetCurrentThreadId()).decision == Decision::allowed,
          "verified native evidence");
    EnableWindow(parent, FALSE);
    check(inspect_native_edit(parent, edit, GetCurrentProcessId(), GetCurrentThreadId()).decision == Decision::denied,
          "disabled ancestor cannot authorize child");
    EnableWindow(parent, TRUE);
    check(inspect_native_edit(parent, edit, 0, GetCurrentThreadId()).decision != Decision::allowed, "wrong PID rejected");
    check(inspect_native_edit(parent, edit, GetCurrentProcessId(), 0).decision != Decision::allowed, "wrong thread rejected");
    check(inspect_native_edit(edit, parent, GetCurrentProcessId(), GetCurrentThreadId()).decision != Decision::allowed,
          "unrelated view rejected");
    std::promise<HWND> ready;
    std::thread owner([&]{
        auto view=CreateWindowW(L"STATIC",L"",WS_OVERLAPPED,0,0,100,100,nullptr,nullptr,nullptr,nullptr);
        ready.set_value(view);MSG message{};
        while(GetMessageW(&message,nullptr,0,0)>0){TranslateMessage(&message);DispatchMessageW(&message);}
        DestroyWindow(view);
    });
    const auto cross_view=ready.get_future().get();
    auto cross_edit=CreateWindowW(L"EDIT",L"",WS_CHILD,0,0,20,20,cross_view,nullptr,nullptr,nullptr);
    check(inspect_native_edit(cross_view,cross_edit,GetCurrentProcessId(),GetCurrentThreadId()).decision==Decision::allowed,
          "same-process cross-thread view retains independently verified focused edit identity");
    if(cross_edit)DestroyWindow(cross_edit);
    PostThreadMessageW(GetWindowThreadProcessId(cross_view,nullptr),WM_QUIT,0,0);owner.join();
    if (parent) DestroyWindow(parent);
    return failures ? 1 : 0;
}

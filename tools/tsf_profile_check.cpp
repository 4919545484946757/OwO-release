#include <Windows.h>
#include <msctf.h>
#include "owo/tsf/registration_cache.h"
#include "owo/compatibility/accessible_edit.h"
#include "owo/compatibility/uia_text_verifier.h"

#include <iostream>
#include <string_view>

namespace {
constexpr CLSID kTextServiceClsid{
    0x6d31c9b1, 0x8978, 0x4f49, {0x89, 0xb4, 0x66, 0xeb, 0x1e, 0x74, 0x15, 0x91}};
constexpr GUID kLanguageProfileGuid{
    0x5d9f39c3, 0xbdb4, 0x453c, {0xa7, 0xba, 0xb9, 0xef, 0x82, 0x48, 0x76, 0x29}};
constexpr LANGID kSimplifiedChinese = 0x0804;
}

int main(int argc, char** argv) {
    if (argc == 2 && std::string_view(argv[1]) == "--focus-state") {
        DWORD pid{};
        const auto thread = GetWindowThreadProcessId(GetForegroundWindow(), &pid);
        GUITHREADINFO info{sizeof(info)};
        wchar_t name[128]{};
        if (!thread || !GetGUIThreadInfo(thread, &info) || !info.hwndFocus ||
            !GetClassNameW(info.hwndFocus, name, 128)) return 10;
        std::wcout << L"pid=" << pid << L" thread=" << thread << L" focus_class=" << name
                   << L" password=" << ((GetWindowLongPtrW(info.hwndFocus, GWL_STYLE) & ES_PASSWORD) != 0)
                   << L" layout=" << GetKeyboardLayout(thread);
        wchar_t original[128]{};
        if(RealGetWindowClassW(info.hwndFocus,original,128))std::wcout<<L" native_class="<<original;
        std::wcout<<L'\n';
        const auto apartment=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
        if(SUCCEEDED(apartment)){
            const auto proof=owo::compatibility::inspect_accessible_edit(info.hwndFocus,pid,
                GetWindowThreadProcessId(info.hwndFocus,nullptr));
            std::cout<<"accessible_role="<<proof.role<<" state="<<proof.state<<" hr="<<proof.result
                     <<" decision="<<static_cast<int>(proof.resolution.decision)
                     <<" ia2_hr="<<proof.ia2_result<<" ia2_states="<<proof.ia2_states<<'\n';
            Microsoft::WRL::ComPtr<IUIAutomation2> automation;
            auto hr=CoCreateInstance(__uuidof(CUIAutomation8),nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&automation));
            Microsoft::WRL::ComPtr<IUIAutomationElement> element;
            if(SUCCEEDED(hr)){automation->put_ConnectionTimeout(100);automation->put_TransactionTimeout(100);hr=automation->GetFocusedElement(&element);}
            std::cout<<"uia_focus_hr="<<hr;
            if(SUCCEEDED(hr)&&element){
                CONTROLTYPEID type{};BOOL password=TRUE,enabled=FALSE,focused=FALSE;
                const auto type_hr=element->get_CurrentControlType(&type);
                const auto password_hr=element->get_CurrentIsPassword(&password);
                const auto enabled_hr=element->get_CurrentIsEnabled(&enabled);
                const auto focus_hr=element->get_CurrentHasKeyboardFocus(&focused);
                Microsoft::WRL::ComPtr<IUIAutomationTextPattern2> pattern;
                Microsoft::WRL::ComPtr<IUIAutomationTextRange> range;
                auto caret_hr=element->GetCurrentPatternAs(UIA_TextPattern2Id,IID_PPV_ARGS(&pattern));
                BOOL active=FALSE; VARIANT attribute{};
                if(SUCCEEDED(caret_hr)&&pattern)caret_hr=pattern->GetCaretRange(&active,&range);
                if(SUCCEEDED(caret_hr)&&range)caret_hr=range->GetAttributeValue(UIA_IsReadOnlyAttributeId,&attribute);
                std::cout<<" type="<<type<<" type_hr="<<type_hr<<" password="<<password<<" password_hr="<<password_hr
                         <<" enabled="<<enabled<<" enabled_hr="<<enabled_hr<<" focused="<<focused<<" focus_hr="<<focus_hr
                         <<" pattern="<<static_cast<bool>(pattern)<<" caret="<<active<<" caret_hr="<<caret_hr
                         <<" readonly_vt="<<attribute.vt<<" readonly="<<(attribute.vt==VT_BOOL?attribute.boolVal:0);
                VariantClear(&attribute);
            }
            std::cout<<'\n';
            if(automation){
                Microsoft::WRL::ComPtr<IUIAutomationElement> native;
                const auto native_hr=automation->ElementFromHandle(info.hwndFocus,&native);
                std::cout<<"uia_native_hr="<<native_hr;
                if(SUCCEEDED(native_hr)&&native){const auto caret=owo::compatibility::inspect_uia_text_caret(native.Get(),pid);
                    std::cout<<" caret_hr="<<caret.result<<" decision="<<static_cast<int>(caret.access.decision);}
                std::cout<<'\n';
            }
            CoUninitialize();
        }
        return 0;
    }
    HRESULT result = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(result)) return 2;

    if (argc == 2 && std::string_view(argv[1]) == "--refresh-cache") {
        result = owo::tsf::invalidate_registration_cache();
        if (SUCCEEDED(result)) std::cout << "TSF assembly description cache refreshed\n";
        else std::cerr << "TSF cache refresh failed: " << std::hex
                       << static_cast<unsigned long>(result) << '\n';
        CoUninitialize();
        return SUCCEEDED(result) ? 0 : 8;
    }

    ITfInputProcessorProfiles* profiles = nullptr;
    result = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr,
                              CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&profiles));
    if (FAILED(result)) {
        CoUninitialize();
        return 3;
    }

    if (argc == 2) {
        const std::string_view action = argv[1];
        const bool activate_session = action == "--activate-session";
        const BOOL enable = action == "--enable" || activate_session ? TRUE : FALSE;
        if (action != "--enable" && action != "--disable" && !activate_session) {
            profiles->Release();
            CoUninitialize();
            return 5;
        }
        result = profiles->EnableLanguageProfile(
            kTextServiceClsid, kSimplifiedChinese, kLanguageProfileGuid, enable);
        if (FAILED(result)) {
            profiles->Release();
            CoUninitialize();
            return 6;
        }
        if (activate_session || !enable) {
            const auto input = LoadLibraryExW(L"input.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
            using InstallTip = BOOL(WINAPI*)(LPCWSTR, DWORD);
            const auto install = input ? reinterpret_cast<InstallTip>(GetProcAddress(input, "InstallLayoutOrTip")) : nullptr;
            const bool enabled = install && install(L"0x0804:{6D31C9B1-8978-4F49-89B4-66EB1E741591}{5D9F39C3-BDB4-453C-A7BA-B9EF82487629}", enable ? 0 : 1);
            if (input) FreeLibrary(input);
            if (!enabled) { std::cerr << "InstallLayoutOrTip failed\n"; profiles->Release(); CoUninitialize(); return 9; }
        }
        if (activate_session) {
            // ActivateProfile(DONTCARECURRENTINPUTLANGUAGE) can merely defer activation.
            result = profiles->ChangeCurrentLanguage(kSimplifiedChinese);
            if (FAILED(result)) {
                std::cerr << "Cannot switch to the profile input language: " << std::hex
                          << static_cast<unsigned long>(result) << '\n';
                profiles->Release(); CoUninitialize(); return 7;
            }
            ITfInputProcessorProfileMgr* manager = nullptr;
            result = CoCreateInstance(CLSID_TF_InputProcessorProfiles, nullptr,
                                      CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&manager));
            if (SUCCEEDED(result)) {
                result = manager->ActivateProfile(
                    TF_PROFILETYPE_INPUTPROCESSOR, kSimplifiedChinese,
                    kTextServiceClsid, kLanguageProfileGuid, nullptr,
                    TF_IPPMF_FORSESSION);
                manager->Release();
            }
            if (FAILED(result)) {
                profiles->Release();
                CoUninitialize();
                return 7;
            }
        }
        std::cout << "OwO TSF profile "
                  << (activate_session ? "activated for session"
                                       : enable ? "enabled" : "disabled")
                  << '\n';
    }

    IEnumTfLanguageProfiles* enumeration = nullptr;
    result = profiles->EnumLanguageProfiles(kSimplifiedChinese, &enumeration);
    bool found = false;
    if (SUCCEEDED(result)) {
        TF_LANGUAGEPROFILE profile{};
        ULONG fetched = 0;
        while (enumeration->Next(1, &profile, &fetched) == S_OK) {
            if (profile.clsid == kTextServiceClsid &&
                profile.guidProfile == kLanguageProfileGuid) {
                found = true;
                std::cout << "OwO TSF profile found; active="
                          << (profile.fActive ? "true" : "false") << '\n';
                break;
            }
        }
        enumeration->Release();
    }
    profiles->Release();
    CoUninitialize();
    return found ? 0 : 4;
}

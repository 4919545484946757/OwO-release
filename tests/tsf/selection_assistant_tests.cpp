#include "text_service.h"
#include "test_text_store.h"
#include <filesystem>
#include <fstream>
#include <iostream>

namespace owo::tsf {
class SelectSession final : public ITfEditSession {
public:
    SelectSession(ITfContext* context, LONG start, LONG end) : context_(context), start_(start), end_(end) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override { if (!out) return E_POINTER; *out=nullptr; if (id!=IID_IUnknown && id!=IID_ITfEditSession) return E_NOINTERFACE; *out=static_cast<ITfEditSession*>(this); AddRef();return S_OK; }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs_; }
    ULONG STDMETHODCALLTYPE Release() override { auto value=--refs_; if(!value)delete this;return value; }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        Microsoft::WRL::ComPtr<ITfRange> range;
        if (FAILED(context_->GetStart(cookie,&range))) return E_FAIL;
        LONG moved{}; range->ShiftEnd(cookie,end_,&moved,nullptr); range->ShiftStart(cookie,start_,&moved,nullptr);
        TF_SELECTION selected{range.Get(),{TF_AE_END,FALSE}};
        return context_->SetSelection(cookie,1,&selected);
    }
private: ULONG refs_{1}; ITfContext* context_; LONG start_,end_;
};
struct SelectionAssistantTestAccess {
    static int run(ITfThreadMgr* manager, TfClientId client, ITfContext* context, TestTextStore* store) {
        auto* service=new TextService;
        service->thread_manager_=manager;manager->AddRef();service->client_id_=client;
        service->foreground_focus_=true; service->chinese_mode_=false;
        service->message_window_=CreateWindowExW(0,L"STATIC",L"",0,0,0,0,0,HWND_MESSAGE,nullptr,GetModuleHandleW(nullptr),nullptr);
        int failures=0;
        const auto check=[&](bool ok,const char* label){if(!ok){std::cerr<<label<<'\n';++failures;}};
        const auto select=[&](LONG start,LONG end){
            auto* session=new SelectSession(context,start,end); HRESULT result=E_FAIL;
            const auto hr=context->RequestEditSession(client,session,TF_ES_SYNC|TF_ES_READWRITE,&result);
            session->Release();return SUCCEEDED(hr)&&SUCCEEDED(result);
        };
        store->text=L"前文 hello 后文";
        check(select(3,3),"set empty selection");
        check(!service->selection_assistant_.test(context,'V'),"v without selection must remain normal input");
        check(select(3,8),"set nonempty selection");
        service->candidate_page_size_=1;
        check(service->selection_assistant_.test(context,'V'),"selected v opens menu in English mode too");
        check(service->selection_assistant_.key(context,'V'),"trigger consumed");
        check(service->selection_assistant_.active() && service->candidates_.size()==4 && store->text==L"前文 hello 后文",
            "menu preserves original text and exposes four operations");
        check(service->has_composition(),"selection menu is an independent candidate presentation");
        check(service->candidate_page_width()==4,"all four actions visible even with normal one-candidate pages");
        check(!service->selection_assistant_.test(context,'A'),"normal letters not swallowed by menu");
        check(service->selection_assistant_.test(context,VK_NUMPAD4),"numpad can choose all four actions");
        const auto writes=store->writes;
        service->selection_assistant_.result_=SelectionAssistant::Result{true,L"你好"};
        service->selection_assistant_.poll();
        check(store->text==L"前文 你好 后文" && store->writes==writes+1 && !service->selection_assistant_.active(),
            "success directly replaces exactly the selection once, no result preview");
        check(service->candidate_page_width()==1,"normal candidate page preference preserved");
        store->text=L"前文 hello 后文";check(select(3,8),"restore fixture selection");
        check(service->selection_assistant_.key(context,'V'),"reopen menu");
        const auto before_stale=store->writes;
        check(select(0,2),"change selection while request pending");
        service->selection_assistant_.result_=SelectionAssistant::Result{true,L"禁止写入"};
        service->selection_assistant_.poll();
        check(store->writes==before_stale && store->text==L"前文 hello 后文" && !service->selection_assistant_.active(),
            "stale selection never overwritten");
        check(select(3,8),"select failure fixture");
        check(service->selection_assistant_.key(context,'V'),"open error fixture");
        service->selection_assistant_.result_=SelectionAssistant::Result{false,L"测试网络错误"};
        service->selection_assistant_.poll();
        check(store->text==L"前文 hello 后文" && service->candidates_==std::vector<std::wstring>{L"重试",L"取消"},
            "model failure preserves text and offers retry/cancel");
        check(service->selection_assistant_.key(context,VK_ESCAPE) && !service->selection_assistant_.active(),"escape cancels without writes");
        check(service->selection_assistant_.key(context,'V'),"open pass-through fixture");
        check(!service->selection_assistant_.key(context,'A') && !service->selection_assistant_.active(),
            "unrelated key cancels menu and passes through unchanged");
        check(service->selection_assistant_.key(context,'V'),"open focus-loss fixture");
        service->foreground_focus_=false;
        service->selection_assistant_.result_=SelectionAssistant::Result{true,L"禁止写入"};
        service->selection_assistant_.poll();
        check(store->writes==before_stale && store->text==L"前文 hello 后文","focus loss cannot replace text");
        service->Release();
        return failures;
    }
};
}

int wmain(int argc, wchar_t** argv) {
    const auto root=std::filesystem::temp_directory_path()/("owo-selection-test-"+std::to_string(GetCurrentProcessId()));
    if(std::filesystem::exists(root))return 90;
    std::filesystem::create_directories(root/L"models");std::filesystem::create_directories(root/L"config");
    std::ofstream(root/L"models"/L"selection-assistant.enabled")<<"1";
    const auto config=root/L"config"/L"owo.conf";
    SetEnvironmentVariableW(L"OWO_CONFIG_PATH",config.c_str());
    if(FAILED(CoInitializeEx(nullptr,COINIT_APARTMENTTHREADED)))return 91;
    auto* store=new TestTextStore;
    store->window=CreateWindowExW(0,L"STATIC",L"OwO isolated selection test",WS_OVERLAPPEDWINDOW,0,0,300,100,nullptr,nullptr,GetModuleHandleW(nullptr),nullptr);
    ShowWindow(store->window,SW_SHOW);SetForegroundWindow(store->window);SetFocus(store->window);
    ITfThreadMgr* manager{}; ITfDocumentMgr* document{}; ITfContext* context{}; TfClientId client{}; TfEditCookie cookie{};
    int result=92;bool activated=false,pushed=false;
    if(SUCCEEDED(CoCreateInstance(CLSID_TF_ThreadMgr,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&manager)))&&SUCCEEDED(manager->Activate(&client))){
        activated=true;
        if(SUCCEEDED(manager->CreateDocumentMgr(&document))&&SUCCEEDED(document->CreateContext(client,0,static_cast<ITextStoreACP*>(store),&context,&cookie))&&SUCCEEDED(document->Push(context))){
            pushed=true;
            if(SUCCEEDED(manager->SetFocus(document)))result=owo::tsf::SelectionAssistantTestAccess::run(manager,client,context,store);
        }
    }
    if(manager)manager->SetFocus(nullptr);if(pushed)document->Pop(TF_POPF_ALL);
    if(context)context->Release();if(document)document->Release();if(activated)manager->Deactivate();if(manager)manager->Release();
    DestroyWindow(store->window);store->Release();
    if (!result && argc == 2) {
        const auto [ok, message] = owo::tsf::test_selection_worker(argv[1]);
        if (ok || message.find(L"启用") == std::wstring::npos) { std::wcerr << L"Native/managed worker IPC failed: " << message << L'\n'; result = 93; }
        else std::cout << "PASS actual native/managed IPC peer authentication and UTF8 framing; disabled worker, no network\n";
    }
    CoUninitialize();SetEnvironmentVariableW(L"OWO_CONFIG_PATH",nullptr);
    std::filesystem::remove_all(root);
    if(!result)std::cout<<"PASS selected-only trigger, four actions, exact direct replacement, stale/focus/failure cancellation; no network requests\n";
    return result;
}

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
        test_edit_foreground=store->window;
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
        service->foreground_focus_=true;
        store->hide_input_scope=true;
        check(select(3,8), "restore selection without scope metadata");
        const auto reads=store->reads;
        check(service->selection_assistant_.test(context,'V') && service->selection_assistant_.key(context,'V'),
              "writable unknown scope can ask the user");
        check(service->selection_assistant_.awaiting_trust_ && store->reads==reads &&
              service->selection_assistant_.selection_.empty(),
              "trust prompt must not read selected text");
        service->selection_assistant_.key(context,VK_SPACE);
        check(service->selection_assistant_.awaiting_trust_, "space is not affirmative consent");
        service->selection_assistant_.key(context,'1');
        check(service->selection_assistant_.active() && !service->selection_assistant_.awaiting_trust_ &&
              service->candidates_.size()==4, "explicit consent opens the four-action menu");
        check(!service->selection_assistant_.selection_.empty() && !service->selection_assistant_.busy_,
              "explicit consent captures the selection but does not call the model");
        service->selection_assistant_.key(context,'1',1LL<<30);
        check(!service->selection_assistant_.busy_, "holding consent key cannot call the model");
        service->clear_composition();
        check(service->selection_assistant_.key(context,'V'), "new call does not inherit approval");
        check(select(0,2), "change untrusted selection before confirmation");
        const auto stale_reads=store->reads;
        service->selection_assistant_.key(context,'1');
        check(!service->selection_assistant_.active() && store->reads==stale_reads,
              "stale trust cannot read a changed selection");
        check(select(3,3), "place caret for ordinary input and Agent");
        check(service->plugin_context_allowed(context,nullptr,true) &&
              !service->plugin_context_allowed(context), "ordinary input stays available without advanced proof");
        check(SUCCEEDED(service->commit_text(context,L"你")) && store->text==L"前文 你hello 后文",
              "ordinary commit writes to the unclassified, writable document");
        service->input_buffer_=L"vnihao";
        service->queue_agent_action(agent::Action::submit);
        check(service->agent_trust_pending_ && !service->pending_request_ && !service->agent_request_submitted_,
              "unverified Agent window cannot send before consent");
        service->choose_agent_window_trust(true);
        check(!service->agent_trust_pending_ && service->pending_request_ && service->agent_request_submitted_,
              "explicit confirmation sends request from the matching window");
        service->cancel_agent_session();
        service->clear_composition();
        store->hide_input_scope=false;
        service->chinese_mode_=true;
        service->shortcut_config_.input_scheme="full_pinyin";
        service->shortcut_config_.candidate_immediate_commit_enabled=true;
        const auto select_candidate = [&](std::wstring input, std::wstring word, std::size_t consumed,
                                           std::vector<std::size_t> breaks = {}) {
            service->clear_composition();
            service->input_buffer_=std::move(input);
            service->input_cursor_=service->input_buffer_.size();
            service->manual_breaks_=std::move(breaks);
            service->candidates_={std::move(word)};
            service->candidate_consumed_={consumed};
            return SUCCEEDED(service->commit_candidate(context,0));
        };
        const auto ctrl_z = [&] {
            BYTE keys[256]{};
            GetKeyboardState(keys);
            const auto previous=keys[VK_CONTROL];
            keys[VK_CONTROL]=0x80;
            SetKeyboardState(keys);
            BOOL eaten=TRUE;
            const auto result=service->OnTestKeyDown(context,'Z',0,&eaten);
            keys[VK_CONTROL]=previous;
            SetKeyboardState(keys);
            return SUCCEEDED(result) && !eaten;
        };
        check(store->application_edit(0,static_cast<LONG>(store->text.size()),L"前后",1), "seed undo document");
        check(select_candidate(L"Ni'HaO",L"你好",6,{2}) && store->text==L"前你好后" &&
              service->candidate_undo_.has_value(), "candidate commit records only its latest original input");
        check(ctrl_z() && service->candidate_undo_probe_until_, "Ctrl+Z arms a matching write without eating the key");
        service->poll_candidate_undo();
        check(service->input_buffer_.empty() && service->candidate_undo_.has_value(),
              "no candidate opens before native Undo changes the document");
        const auto writes_before_undo=store->writes;
        check(store->application_edit(1,3,L"",1), "native Undo removes the candidate");
        service->poll_candidate_undo();
        check(service->input_buffer_==L"Ni'HaO" && service->manual_breaks_==std::vector<std::size_t>{2} &&
              service->candidate_request_pending_ && !service->candidate_undo_ &&
              store->text==L"前后" && store->writes==writes_before_undo,
              "Undo restores exact spelling and separators without writing to the host");

        check(select_candidate(L"nishi",L"你",2) && service->input_buffer_==L"shi", "commit a prefix with pending tail");
        check(ctrl_z() && service->candidate_undo_probe_until_, "native Undo also works with untouched pending pinyin");
        check(store->application_edit(1,2,L"",1), "native Undo removes prefix only");
        service->poll_candidate_undo();
        check(service->input_buffer_==L"nishi" && !service->candidate_undo_, "Undo restores the prefix and pending tail together");

        check(select_candidate(L"nihao",L"你好",5) && ctrl_z(), "arm probe for grouped undo");
        check(store->application_edit(0,3,L"",0), "application undoes a grouped edit");
        service->poll_candidate_undo();
        check(service->input_buffer_.empty(), "no replay when host also removes other text");
        if (service->candidate_undo_) service->candidate_undo_->expires=GetTickCount64();
        service->poll_candidate_undo();
        check(!service->candidate_undo_, "expired records are destroyed");

        check(store->application_edit(0,static_cast<LONG>(store->text.size()),L"前后",1), "seed latest-commit test");
        check(select_candidate(L"ni",L"你",2) && select_candidate(L"hao",L"好",3) && ctrl_z(),
              "second candidate supersedes the earlier record");
        check(store->application_edit(2,3,L"",2), "undo only the most recent commit");
        service->poll_candidate_undo();
        check(service->input_buffer_==L"hao" && !service->candidate_undo_, "only latest commit can be recovered");

        check(store->application_edit(0,static_cast<LONG>(store->text.size()),L"前后",1), "seed staged commit");
        service->shortcut_config_.candidate_immediate_commit_enabled=false;
        check(select_candidate(L"ni'hao",L"你好",6,{2}) && store->text==L"前后",
              "a staged choice does not create an app edit");
        check(SUCCEEDED(service->commit_pending_composition(context)) && store->text==L"前你好后" &&
              ctrl_z() && service->candidate_undo_probe_until_, "staged confirmation saves original pinyin");
        check(store->application_edit(1,3,L"",1), "application undoes staged confirmation");
        service->poll_candidate_undo();
        check(service->input_buffer_==L"ni'hao" && service->manual_breaks_==std::vector<std::size_t>{2},
              "staged candidate uses its original input and original manual separators");

        check(SUCCEEDED(service->commit_raw_input(context)) && !service->candidate_undo_ && ctrl_z(),
              "raw letters are normal native Undo without candidate recovery");
        test_edit_foreground=nullptr;
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
    // A STATIC HWND can legitimately report MSAA's READONLY flag. This
    // fixture represents a custom edit host without a native control role.
    WNDCLASSW host{};
    host.lpfnWndProc=DefWindowProcW;
    host.hInstance=GetModuleHandleW(nullptr);
    host.lpszClassName=L"OwO.TestTextStore";
    RegisterClassW(&host);
    store->window=CreateWindowExW(0,host.lpszClassName,L"OwO isolated text store",WS_OVERLAPPEDWINDOW,
                                  0,0,300,100,nullptr,nullptr,host.hInstance,nullptr);
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
    if(!result)std::cout<<"PASS selection, focus/trust, and native Undo with one-shot pinyin recovery; no network requests\n";
    return result;
}

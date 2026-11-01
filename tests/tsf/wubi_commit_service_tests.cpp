#include "text_service.h"
#include "test_text_store.h"
#include "owo/core/input_method_runtime.h"
#include "owo/plugin/plugin_store.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "../../plugins/wubi86/src/provider.h"
#include <iostream>
#include <fstream>
#include <limits>

namespace owo::tsf {
struct WubiCommitTestAccess {
    static int run(ITfThreadMgr* manager, TfClientId client, ITfContext* context, TestTextStore* store,
                   const std::wstring& pipe, const std::filesystem::path& root) {
        auto* service = new TextService;
        service->thread_manager_ = manager; manager->AddRef(); service->client_id_ = client;
        service->shortcut_config_.input_scheme = "plugin:fixture/wubi";
        service->shortcut_config_initialized_ = true;
        service->next_shortcut_config_refresh_ = (std::numeric_limits<ULONGLONG>::max)();
        auto prepare = [&] {
            service->clear_composition();
            service->plugin_input_context_ = context; context->AddRef();
            service->input_buffer_ = L"fixture";
            service->input_cursor_ = service->input_buffer_.size();
            (void)service->plugin_input_document_.enter(++service->context_generation_);
        };
        int failures{};
        auto check = [&](bool ok, const char* label) { if (!ok) { std::cerr << label << '\n'; ++failures; } };
        auto drain = [&] {
            const auto end = GetTickCount64() + 2000;
            do {
                MSG message{};
                while (PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageW(&message); }
                if (!service->plugin_input_committing_) break;
                Sleep(1);
            } while (GetTickCount64() < end);
        };
        auto await_lock = [&] {
            const auto end = GetTickCount64() + 500;
            while (!store->deferred_lock && service->plugin_input_committing_ && GetTickCount64() < end) {
                MSG message{};
                while (PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageW(&message); }
                Sleep(1);
            }
        };
        prepare();
        store->defer_read_lock=true;
        service->poll_plugin_input();
        const auto read_deadline=GetTickCount64()+500;
        while(!store->deferred_lock && !service->input_buffer_.empty() && GetTickCount64()<read_deadline){
            MSG message{};while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)){TranslateMessage(&message);DispatchMessageW(&message);}Sleep(1);
        }
        check(service->input_buffer_==L"fixture" && store->deferred_lock,
              "host deferring read lock preserves input and queues asynchronous metadata session");
        store->defer_read_lock=false;
        if(store->deferred_lock)(void)store->grant_deferred_lock();
        prepare();
        check(service->plugin_context_is_current(context), "real TSF focused context current");
        check(service->plugin_context_allowed(context), "application input scope and compartments authorize ordinary context");
        service->commit_plugin_text(L"\x4e2d\x6587", true); drain();
        check(store->writes == 1 && store->text == L"\x4e2d\x6587" && !service->plugin_input_committing_,
              "actual commit_plugin_text/SchemeEditSession writes through Windows TSF");
        prepare(); store->fail_write = true;
        service->commit_plugin_text(L"blocked", true); drain();
        check(store->writes == 2 && store->text == L"\x4e2d\x6587" && service->candidate_request_failed_ &&
              service->input_buffer_ == L"fixture" && !service->plugin_commit_future_.valid(),
              "actual failed SetText preserves input without commit acknowledgement");
        store->fail_write = false; prepare(); service->foreground_focus_ = false;
        service->commit_plugin_text(L"retired", true); drain();
        check(store->writes == 2 && !service->plugin_commit_future_.valid(), "actual invalid context cannot write or acknowledge");
        service->foreground_focus_ = true;
        prepare(); store->defer_write_lock = true;
        service->commit_plugin_text(L"once", true);
        await_lock();
        check(service->plugin_input_committing_ && store->deferred_lock && store->writes == 2,
              "actual async write awaits application lock");
        service->commit_plugin_text(L"duplicate", true);
        store->defer_write_lock = false;
        check(SUCCEEDED(store->grant_deferred_lock()), "grant actual queued production write lock"); drain();
        check(store->writes == 3 && store->text.find(L"once") != std::wstring::npos && store->text.find(L"duplicate") == std::wstring::npos,
              "actual duplicate pending commit inserts once");
        prepare(); store->defer_write_lock = true;
        service->commit_plugin_text(L"invalidated", true);
        await_lock();
        check(service->plugin_input_committing_ && store->deferred_lock, "second write queues before invalidation");
        service->foreground_focus_ = false; store->defer_write_lock = false;
        (void)store->grant_deferred_lock(); drain();
        check(store->writes == 3 && !service->plugin_commit_future_.valid(),
              "actual callback rechecks invalidated context and produces no acknowledgement");
        service->foreground_focus_ = true;
        prepare(); store->fail_selection = true;
        service->commit_plugin_text(L"caret", true); drain();
        check(store->writes == 4 && store->text.find(L"caret") != std::wstring::npos && !service->candidate_request_failed_,
              "actual successful SetText remains success despite caret failure");
        store->fail_selection = false;
        prepare();
        // Scheduling fixture only: the promise models an outstanding IPC result;
        // it does not claim a provider acknowledgement or a learning write.
        std::promise<ipc::InputMethodClientResult> outstanding;
        service->plugin_commit_future_ = outstanding.get_future();
        service->shortcut_config_.language_shortcut_enabled = true;
        service->shortcut_config_.language_shortcuts = {"F7"};
        BYTE previous[256]{}, cleared[256]{}; GetKeyboardState(previous); SetKeyboardState(cleared);
        for (const WPARAM key : {WPARAM(VK_RETURN), WPARAM(VK_ESCAPE), WPARAM(VK_F7)}) {
            BOOL eaten{};
            const auto hr = service->handle_plugin_key(context,key,0,&eaten);
            check(SUCCEEDED(hr) && eaten && service->input_buffer_ == L"fixture" && service->plugin_commit_future_.valid(),
                  "pending commit confirmation defers Enter/Escape/language shortcut");
        }
        SetKeyboardState(previous);
        check(service->plugin_deferred_keys_.size() == 3 && store->writes == 4,
              "pending acknowledgement keys queue without text replay");
        prepare();
        const auto descriptor = wubi86::descriptor();
        service->shortcut_config_.input_scheme = "plugin:" + descriptor.id;
        service->plugin_input_client_ = std::make_unique<ipc::InputMethodAsyncClient>(pipe);
        service->plugin_input_session_ = "actual-tsf-session";
        const auto generation = service->plugin_input_document_.generation();
        auto opened_future = service->plugin_input_client_->open(generation,descriptor.plugin_id,descriptor.id,service->plugin_input_session_,false);
        if (opened_future.wait_for(std::chrono::seconds(10)) != std::future_status::ready) {
            check(false,"real IPC open timed out"); service->Release(); return failures;
        }
        const auto opened = opened_future.get();
        check(service->plugin_input_document_.bind(opened,service->plugin_input_session_), "bind actual IPC document");
        service->plugin_input_descriptor_ = descriptor;
        check(service->plugin_input_document_.composition().insert("wqiy"), "insert actual Wubi code");
        service->sync_plugin_display(); service->queue_plugin_compose();
        auto poll_until = [&](auto done) {
            const auto end = GetTickCount64() + 10000;
            while (!done() && GetTickCount64() < end) {
                service->poll_plugin_input();
                MSG message{};
                while (PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) { TranslateMessage(&message); DispatchMessageW(&message); }
                Sleep(1);
            }
            return done();
        };
        check(poll_until([&]{ return !service->candidate_request_pending_; }) && !service->candidates_.empty(), "production compose displays real Wubi candidates");
        auto learned_file = [&]() {
            for (const auto& file : std::filesystem::recursive_directory_iterator(root))
                if (file.path().filename() == "wubi86-learning.v1.tsv") return file.path();
            return std::filesystem::path{};
        };
        check(learned_file().empty(), "compose creates no learning file");
        const auto before = store->writes;
        store->defer_write_lock = true;
        service->select_plugin_candidate(0);
        check(poll_until([&]{ return store->deferred_lock != 0; }), "real select acknowledgement schedules actual production edit callback");
        check(store->writes == before && learned_file().empty() && !service->plugin_commit_future_.valid(),
              "real selection acknowledgement learns nothing before SetText");
        store->fail_write = true; store->defer_write_lock = false; (void)store->grant_deferred_lock(); drain();
        check(store->writes == before + 1 && learned_file().empty() && !service->plugin_commit_future_.valid() &&
              service->plugin_input_document_.composition().input() == "wqiy",
              "real selected Wubi SetText failure preserves code and creates no learning or acknowledgement");
        store->fail_write = false;
        service->queue_plugin_compose();
        check(poll_until([&]{ return !service->candidate_request_pending_; }) && !service->candidates_.empty(),
              "real failed write can recompose retained code");
        store->defer_write_lock = true; service->select_plugin_candidate(0);
        check(poll_until([&]{ return store->deferred_lock != 0; }), "retry selection schedules actual write");
        service->commit_plugin_text(L"duplicate",true);
        store->defer_write_lock = false; (void)store->grant_deferred_lock();
        check(store->writes == before + 2 && service->plugin_commit_future_.valid(),
              "actual successful SetText queues real committed acknowledgement");
        check(poll_until([&]{ return !service->plugin_commit_future_.valid(); }) && !learned_file().empty(),
              "real provider learning persists only after successful production SetText and committed IPC");
        std::ifstream learned(learned_file(),std::ios::binary);
        const std::string persisted((std::istreambuf_iterator<char>(learned)),{});
        learned.close(); // Permit the provider's atomic replace on the next commit.
        check(persisted.find("entry\twqiy\t\xe4\xbd\xa0\t1\n") != std::string::npos,
              "failed write and duplicate pending commit do not double-count persisted learning");
        check(store->text.find(L"\x4f60") != std::wstring::npos, "real Wubi candidate inserted into Windows TSF test TextStore");
        // Lifetime preservation with the same test-only focus resolver. This
        // does not exercise or override Core's native foreground security policy.
        prepare();
        service->plugin_input_client_ = std::make_unique<ipc::InputMethodAsyncClient>(pipe);
        service->plugin_input_session_ = "retired-actual-tsf-session";
        const auto retired_generation = service->plugin_input_document_.generation();
        auto retired_open = service->plugin_input_client_->open(retired_generation,descriptor.plugin_id,descriptor.id,service->plugin_input_session_,false);
        if (retired_open.wait_for(std::chrono::seconds(10)) != std::future_status::ready) {
            check(false,"lifetime fixture IPC open timed out"); service->Release(); return failures;
        }
        check(service->plugin_input_document_.bind(retired_open.get(),service->plugin_input_session_), "bind lifetime fixture document");
        service->plugin_input_descriptor_ = descriptor;
        check(service->plugin_input_document_.composition().insert("wqiy"), "insert lifetime fixture code");
        service->sync_plugin_display(); service->queue_plugin_compose();
        check(poll_until([&]{ return !service->candidate_request_pending_; }) && !service->candidates_.empty(), "lifetime fixture real compose");
        store->defer_write_lock = true; service->select_plugin_candidate(0);
        check(poll_until([&]{ return store->deferred_lock != 0; }), "lifetime fixture actual edit queues");
        store->defer_write_lock = false; (void)store->grant_deferred_lock();
        check(service->plugin_commit_future_.valid(), "actual write submits confirmation before immediate reset");
        const auto writes_at_reset = store->writes;
        const auto text_at_reset = store->text;
        const auto locks_before_reset = server_lock_count();
        service->clear_composition(); // Deliberately do not poll the confirmation.
        check(!service->plugin_commit_future_.valid() && !service->plugin_input_context_, "reset detaches UI session from submitted feedback");
        auto persisted_count_two = [&] {
            std::ifstream file(learned_file(),std::ios::binary);
            const std::string contents((std::istreambuf_iterator<char>(file)),{});
            return contents.find("entry\twqiy\t\xe4\xbd\xa0\t2\n") != std::string::npos;
        };
        check(poll_until([&]{ return server_lock_count() == locks_before_reset; }) && persisted_count_two(),
              "submitted real confirmation survives immediate UI reset and learns once");
        check(store->writes == writes_at_reset && store->text == text_at_reset,
              "retired confirmation cannot replay application text");
        store->fail_selection = true;
        const auto before_native = store->writes;
        const auto native_commit = service->commit_text(context, L"native-host-commit");
        check(SUCCEEDED(native_commit) && store->writes == before_native + 1,
              "native TSF SetText success is not reversed by host caret failure");
        store->fail_selection = false;
        service->Release();
        return failures;
    }
};
}

int main(int argc, char** argv) {
    if (argc != 3) return 92;
    const auto root = std::filesystem::absolute(std::filesystem::path(argv[1]) / ("wubi-tsf-" + std::to_string(GetCurrentProcessId()) + "-" + std::to_string(GetTickCount64())));
    if (!owo::plugin::initialize_plugin_store(root).ok) return 93;
    const auto stage = root / "staging" / ".wubi";
    std::filesystem::copy(argv[2],stage,std::filesystem::copy_options::recursive);
    if (!owo::plugin::publish_staged_plugin(root,stage,std::string(64,'a'),std::string(64,'b')).ok) return 94;
    const auto active_plugin = owo::plugin::query_active_plugin_version(root,owo::wubi86::descriptor().plugin_id);
    if (!active_plugin.ok) return 95;
    const auto grant = owo::plugin::make_plugin_authorization(active_plugin.manifest,active_plugin.inventory_sha256,
        active_plugin.publisher_certificate_sha256,{"input.scheme"});
    if (!grant.ok || !owo::plugin::save_plugin_authorization(root,grant.value).ok) return 96;
    const auto pipe = std::wstring(LR"(\\.\pipe\OwO.Test.Wubi.TSF.)") + std::to_wstring(GetCurrentProcessId());
    owo::ipc::InputMethodPipeServer server(root,pipe,[](std::uint32_t){ return owo::ipc::InputMethodFocus{1,false}; });
    if (!server.started()) return 97;
    if (FAILED(CoInitializeEx(nullptr,COINIT_APARTMENTTHREADED))) return 90;
    ITfThreadMgr* manager{}; ITfDocumentMgr* document{}; ITfContext* context{};
    TfClientId client{}; TfEditCookie cookie{}; auto* store = new TestTextStore;
    store->window = CreateWindowExW(0,L"STATIC",L"OwO service test",WS_POPUP,0,0,100,100,nullptr,nullptr,GetModuleHandleW(nullptr),nullptr);
    bool active{}, pushed{}; int result = 91;
    if (SUCCEEDED(CoCreateInstance(CLSID_TF_ThreadMgr,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&manager))) &&
        SUCCEEDED(manager->Activate(&client))) {
        active = true;
        if (SUCCEEDED(manager->CreateDocumentMgr(&document)) && SUCCEEDED(document->CreateContext(client,0,static_cast<ITextStoreACP*>(store),&context,&cookie)) &&
            SUCCEEDED(document->Push(context))) {
            pushed = true;
            if (SUCCEEDED(manager->SetFocus(document))) result = owo::tsf::WubiCommitTestAccess::run(manager,client,context,store,pipe,root);
        }
    }
    if (manager) manager->SetFocus(nullptr);
    if (pushed) document->Pop(TF_POPF_ALL);
    if (context) context->Release(); if (document) document->Release();
    if (active) manager->Deactivate(); if (manager) manager->Release();
    DestroyWindow(store->window); store->Release(); CoUninitialize();
    if (!result) std::cout << "PASS actual TextService commit_plugin_text/SchemeEditSession with Windows TSF and test TextStore; real apps NOT TESTED\n";
    std::cout << "Retained isolated store: " << root.string() << '\n';
    return result;
}

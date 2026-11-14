#include "test_text_store.h"
#include "owo/tsf/scheme_commit.h"
#include "owo/tsf/input_method_context.h"
#include "../../src/tsf/inline_preedit.h"
#include <iostream>

namespace {
class Edit final : public ITfEditSession {
public:
    ITfContext* context{};
    ITfRange* refreshed{};
    bool current{true}, pending{true};
    unsigned confirmations{}, calls{};
    owo::tsf::InlinePreedit* inline_preview{};
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        if (!out) return E_POINTER; *out = nullptr;
        if (id != IID_IUnknown && id != IID_ITfEditSession) return E_NOINTERFACE;
        *out = static_cast<ITfEditSession*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs_; }
    ULONG STDMETHODCALLTYPE Release() override { auto n = --refs_; if (!n) delete this; return n; }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        ++calls;
        const owo::tsf::InputMethodWriteFacts facts{1,1,2000,2000,1000,true,pending,current,true,
            owo::tsf::InputMethodContextSafety::allowed};
        if (!owo::tsf::input_method_write_allowed(facts)) return E_ABORT;
        if (inline_preview && !inline_preview->allows_selection(context, cookie)) return E_ABORT;
        auto* range = inline_preview ? inline_preview->prepare_commit(context) : nullptr;
        auto hr = owo::tsf::commit_scheme_selection(context, cookie, L"\x4e2d\x6587", &refreshed, range);
        if (inline_preview) inline_preview->finish_commit(SUCCEEDED(hr));
        pending = false;
        // Observation of successful writes; no engine learning is simulated here.
        if (SUCCEEDED(hr)) ++confirmations;
        return hr;
    }
    ~Edit() { if (refreshed) refreshed->Release(); }
private:
    ULONG refs_{1};
};
bool require(HRESULT hr, const char* label) {
    if (SUCCEEDED(hr)) return true;
    std::cerr << label << " HRESULT=0x" << std::hex << hr << '\n'; return false;
}
}

int main() {
    if (!require(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED), "CoInitializeEx")) return 1;
    ITfThreadMgr* manager{}; ITfDocumentMgr* document{}; ITfContext* context{};
    TfClientId client{}; TfEditCookie cookie{};
    auto* store = new TestTextStore;
    store->window = CreateWindowExW(0, L"STATIC", L"OwO test text store", WS_POPUP,
                                  0,0,100,100,nullptr,nullptr,GetModuleHandleW(nullptr),nullptr);
    bool active{}, pushed{}; int failures{};
    if (require(CoCreateInstance(CLSID_TF_ThreadMgr,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&manager)), "ThreadMgr") &&
        require(manager->Activate(&client), "Activate")) {
        active = true;
        if (require(manager->CreateDocumentMgr(&document), "CreateDocumentMgr") &&
            require(document->CreateContext(client,0,static_cast<ITextStoreACP*>(store),&context,&cookie), "CreateContext") &&
            require(document->Push(context), "Push")) {
            pushed = true;
            auto* edit = new Edit; edit->context = context;
            auto run = [&] {
                HRESULT session = E_FAIL;
                auto request = context->RequestEditSession(client,edit,TF_ES_SYNC|TF_ES_READWRITE,&session);
                if (!require(request, "RequestEditSession")) return request;
                return session;
            };
            auto check = [&](bool ok, const char* label) { if (!ok) { std::cerr << label << '\n'; ++failures; } };
            check(SUCCEEDED(run()), "production write succeeds");
            check(store->text == L"\x4e2d\x6587" && store->writes == 1 && edit->confirmations == 1 && edit->refreshed,
                  "real TSF range writes test store and refreshes selection");
            check(run() == E_ABORT && store->writes == 1 && edit->confirmations == 1,
                  "duplicate callback produces no write/confirmation");
            edit->pending = true; store->fail_write = true;
            auto* prior = edit->refreshed;
            check(FAILED(run()) && store->writes == 2 && store->text == L"\x4e2d\x6587" &&
                  edit->refreshed == prior && edit->confirmations == 1,
                  "failed application write preserves selection and gives no confirmation");
            store->fail_write = false; edit->pending = true; edit->current = false;
            check(run() == E_ABORT && store->writes == 2 && edit->confirmations == 1,
                  "invalidated context gives no write/confirmation");
            edit->current = true; edit->pending = true; store->fail_selection = true;
            check(SUCCEEDED(run()) && store->writes == 3 && edit->confirmations == 2,
                  "successful SetText still confirms if caret positioning fails");
            check(run() == E_ABORT && store->writes == 3 && edit->confirmations == 2,
                  "caret failure cannot cause duplicate insertion");
            check(store->locks > 0 && edit->calls == 6, "Windows edit session and store lock callbacks executed");
            store->fail_selection = false;
            auto* preview = new owo::tsf::InlinePreedit;
            const auto committed_prefix = store->text;
            preview->update(context, client, L"pre", 2);
            check(preview->active(), "application accepted inline preedit");
            edit->inline_preview = preview; edit->pending = true;
            check(SUCCEEDED(run()) && !preview->active() &&
                store->text.size() == committed_prefix.size() + 2 && store->text.find(L"pre") == std::wstring::npos,
                "commit replaces the entire inline preview without appending duplicate text");
            check(store->compositions_started > 0 && store->compositions_ended > 0,
                "actual TSF composition was ended");
            edit->inline_preview = nullptr;
            store->reject_composition = true;
            const auto preserved = store->text;
            preview->update(context, client, L"would overwrite", 0);
            check(!preview->active() && store->text == preserved,
                "when a host rejects inline preview, no text is inserted");
            preview->cancel();
            preview->Release();
            edit->Release();
        } else ++failures;
    } else ++failures;
    if (pushed) document->Pop(TF_POPF_ALL);
    if (context) context->Release(); if (document) document->Release();
    if (active) manager->Deactivate(); if (manager) manager->Release();
    DestroyWindow(store->window); store->Release(); CoUninitialize();
    if (!failures) std::cout << "PASS: production TSF write helper through Windows context/edit session into test TextStore; real applications NOT TESTED\n";
    return failures ? 1 : 0;
}

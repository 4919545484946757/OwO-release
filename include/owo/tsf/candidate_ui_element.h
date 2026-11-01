#pragma once
#include <Windows.h>
#include <msctf.h>
#include <functional>
#include <string>
#include <vector>

namespace owo::tsf {
// A bounded snapshot of the current engine page, not a second candidate engine.
// This STA object never queries Core while a host reads the UIElement.
class CandidateUiElement final : public ITfCandidateListUIElementBehavior {
public:
    explicit CandidateUiElement(std::function<void()> release_server = {}) : release_server_(std::move(release_server)) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        if (!out) return E_POINTER; *out=nullptr;
        if(id!=IID_IUnknown && id!=__uuidof(ITfUIElement) && id!=__uuidof(ITfCandidateListUIElement) &&
           id!=__uuidof(ITfCandidateListUIElementBehavior)) return E_NOINTERFACE;
        *out=static_cast<ITfCandidateListUIElementBehavior*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&refs_); }
    ULONG STDMETHODCALLTYPE Release() override { auto n=InterlockedDecrement(&refs_); if(!n)delete this; return n; }
    void publish(std::vector<std::wstring> text, ITfDocumentMgr* document,
                 std::function<HRESULT(UINT)> select, std::function<HRESULT()> cancel) {
        if(text.size()>1024) text.resize(1024);
        text_=std::move(text); selection_=0; select_=std::move(select); cancel_=std::move(cancel);
        if(document) document->AddRef(); if(document_) document_->Release(); document_=document;
    }
    void detach() noexcept { select_={}; cancel_={}; text_.clear(); if(document_)document_->Release(); document_=nullptr; }
    HRESULT STDMETHODCALLTYPE GetDescription(BSTR* value) override {
        if(!value)return E_POINTER; *value=SysAllocString(L"OwO candidates"); return *value?S_OK:E_OUTOFMEMORY;
    }
    HRESULT STDMETHODCALLTYPE GetGUID(GUID* value) override {
        if(!value)return E_POINTER;
        *value={0xe395a169,0x2b0b,0x4b0c,{0x81,0x29,0xe3,0x9a,0x81,0x1e,0xa6,0x45}}; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE Show(BOOL show) override { shown_=show!=FALSE; return S_OK; }
    HRESULT STDMETHODCALLTYPE IsShown(BOOL* value) override { if(!value)return E_POINTER; *value=shown_;return S_OK; }
    HRESULT STDMETHODCALLTYPE GetUpdatedFlags(DWORD* value) override {
        if(!value)return E_POINTER;
        *value=TF_CLUIE_COUNT|TF_CLUIE_SELECTION|TF_CLUIE_STRING|TF_CLUIE_PAGEINDEX|TF_CLUIE_CURRENTPAGE;return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetDocumentMgr(ITfDocumentMgr** value) override {
        if(!value)return E_POINTER; *value=document_;if(document_)document_->AddRef();return document_?S_OK:E_FAIL;
    }
    HRESULT STDMETHODCALLTYPE GetCount(UINT* value) override { if(!value)return E_POINTER;*value=static_cast<UINT>(text_.size());return S_OK; }
    HRESULT STDMETHODCALLTYPE GetSelection(UINT* value) override { if(!value)return E_POINTER;*value=selection_;return S_OK; }
    HRESULT STDMETHODCALLTYPE GetString(UINT index, BSTR* value) override {
        if(!value)return E_POINTER; *value=nullptr; if(index>=text_.size())return E_INVALIDARG;
        *value=SysAllocStringLen(text_[index].data(),static_cast<UINT>(text_[index].size()));return *value?S_OK:E_OUTOFMEMORY;
    }
    HRESULT STDMETHODCALLTYPE GetPageIndex(UINT* indices, UINT capacity, UINT* count) override {
        if(!count)return E_POINTER; *count=text_.empty()?0:1;
        if(capacity<*count)return S_FALSE; if(*count && !indices)return E_POINTER; if(*count)indices[0]=0;return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetPageIndex(UINT* indices, UINT count) override {
        return count==1 && indices && *indices==0 ? S_OK:E_INVALIDARG;
    }
    HRESULT STDMETHODCALLTYPE GetCurrentPage(UINT* page) override {if(!page)return E_POINTER;*page=0;return S_OK;}
    HRESULT STDMETHODCALLTYPE SetSelection(UINT index) override {
        if(GetCurrentThreadId()!=thread_)return RPC_E_WRONG_THREAD;
        if(index>=text_.size())return E_INVALIDARG;selection_=index;return S_OK;
    }
    HRESULT STDMETHODCALLTYPE Finalize() override {
        if(GetCurrentThreadId()!=thread_)return RPC_E_WRONG_THREAD;
        if(!select_ || selection_>=text_.size())return E_ABORT;
        try { auto action=select_;return action(selection_); }catch(...){return E_FAIL;}
    }
    HRESULT STDMETHODCALLTYPE Abort() override {
        if(GetCurrentThreadId()!=thread_)return RPC_E_WRONG_THREAD;
        if(!cancel_)return E_ABORT;
        try {auto action=cancel_;return action();}catch(...){return E_FAIL;}
    }
private:
    ~CandidateUiElement(){detach();if(release_server_)release_server_();}
    LONG refs_{1}; DWORD thread_{GetCurrentThreadId()}; bool shown_{true}; UINT selection_{};
    ITfDocumentMgr* document_{}; std::vector<std::wstring> text_;
    std::function<HRESULT(UINT)> select_; std::function<HRESULT()> cancel_; std::function<void()> release_server_;
};
} // namespace owo::tsf

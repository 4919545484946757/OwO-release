#include "owo/tsf/candidate_ui_element.h"
#include <iostream>
struct HostSink final : ITfUIElementSink {
    ULONG refs{1}; UINT begin{}, update{}, end{};
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id,void** out) override {
        if(!out)return E_POINTER;*out=nullptr;
        if(id!=IID_IUnknown && id!=__uuidof(ITfUIElementSink))return E_NOINTERFACE;
        *out=static_cast<ITfUIElementSink*>(this);AddRef();return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override{return ++refs;}
    ULONG STDMETHODCALLTYPE Release() override{auto n=--refs;if(!n)delete this;return n;}
    HRESULT STDMETHODCALLTYPE BeginUIElement(DWORD,BOOL* show) override{++begin;*show=FALSE;return S_OK;}
    HRESULT STDMETHODCALLTYPE UpdateUIElement(DWORD) override{++update;return S_OK;}
    HRESULT STDMETHODCALLTYPE EndUIElement(DWORD) override{++end;return S_OK;}
};
int main() {
    auto* ui = new owo::tsf::CandidateUiElement;
    int failures{}; unsigned selected = 99;
    auto check=[&](bool value){if(!value) ++failures;};
    ui->publish({L"one", L"two"}, nullptr, [&](UINT index){ selected=index; return S_OK; }, []{return S_OK;});
    UINT count{}; check(ui->GetCount(&count)==S_OK && count==2);
    BSTR text{}; check(ui->GetString(1,&text)==S_OK && std::wstring_view(text)==L"two"); SysFreeString(text);
    check(ui->GetString(2,&text)==E_INVALIDARG);
    check(ui->SetSelection(2)==E_INVALIDARG);
    check(ui->SetSelection(1)==S_OK && ui->Finalize()==S_OK && selected==1);
    UINT page=0, size{}; check(ui->GetPageIndex(&page,1,&size)==S_OK && size==1 && page==0);
    page=1; check(ui->SetPageIndex(&page,1)==E_INVALIDARG);
    check(ui->GetCount(nullptr)==E_POINTER);
    ui->Show(FALSE); BOOL shown=TRUE; check(ui->IsShown(&shown)==S_OK && !shown);
    ui->detach(); check(ui->Finalize()==E_ABORT && ui->Abort()==E_ABORT);
    ui->Release();
    const auto apartment=CoInitializeEx(nullptr,COINIT_APARTMENTTHREADED);
    check(SUCCEEDED(apartment));
    ITfThreadMgr* manager{}; ITfUIElementMgr* elements{}; ITfSource* source{}; TfClientId client{};
    auto* sink=new HostSink; DWORD cookie=TF_INVALID_COOKIE, id{};
    if(SUCCEEDED(CoCreateInstance(CLSID_TF_ThreadMgr,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&manager)))) {
        check(SUCCEEDED(manager->Activate(&client)));
        check(SUCCEEDED(manager->QueryInterface(IID_PPV_ARGS(&elements))));
        check(SUCCEEDED(manager->QueryInterface(IID_PPV_ARGS(&source))));
        if(elements && source){
            check(SUCCEEDED(source->AdviseSink(__uuidof(ITfUIElementSink),sink,&cookie)));
            ui=new owo::tsf::CandidateUiElement; BOOL show=TRUE;
            check(SUCCEEDED(elements->BeginUIElement(ui,&show,&id)) && !show && sink->begin==1);
            check(SUCCEEDED(elements->UpdateUIElement(id)) && sink->update==1);
            check(SUCCEEDED(elements->EndUIElement(id)) && sink->end==1);
            ui->Release();
            if(cookie!=TF_INVALID_COOKIE)source->UnadviseSink(cookie);
        }
        if(elements)elements->Release();if(source)source->Release();
        manager->Deactivate();manager->Release();
    }else check(false);
    sink->Release();if(SUCCEEDED(apartment))CoUninitialize();
    if(failures)std::cerr<<failures<<" candidate UI failures\n";
    return failures?1:0;
}

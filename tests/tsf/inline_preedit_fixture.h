#pragma once
#include "inline_preedit.h"
#include <vector>
#include <memory>

namespace preedit_fixture {
using Microsoft::WRL::ComPtr;
struct Document { std::wstring text = L"前文"; LONG caret = 2, end = 2; };
#define COM_REFS(Type) LONG refs = 1; ULONG STDMETHODCALLTYPE AddRef() override { return ++refs; } \
    ULONG STDMETHODCALLTYPE Release() override { const auto n = --refs; if (!n) delete this; return n; }
#define STUB(name, ...) HRESULT STDMETHODCALLTYPE name(__VA_ARGS__) override { return E_NOTIMPL; }
struct Range final : ITfRange {
    COM_REFS(Range)
    Document* doc; LONG begin, end;
    Range(Document* d, LONG b, LONG e) : doc(d), begin(b), end(e) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        *out = nullptr; if (id != IID_IUnknown && id != IID_ITfRange) return E_NOINTERFACE;
        *out = static_cast<ITfRange*>(this); AddRef(); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetText(TfEditCookie, DWORD, WCHAR* out, ULONG maximum, ULONG* read) override {
        *read = std::min(maximum, static_cast<ULONG>(end-begin)); std::copy_n(doc->text.data()+begin, *read, out); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetText(TfEditCookie, DWORD, const WCHAR* text, LONG size) override {
        doc->text.replace(begin, end-begin, text, size); end = begin+size; doc->caret = doc->end = end; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE ShiftStart(TfEditCookie, LONG count, LONG* moved, const TF_HALTCOND*) override {
        const auto next = std::clamp(begin+count, 0L, end); *moved = next-begin; begin=next; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE ShiftEnd(TfEditCookie, LONG count, LONG* moved, const TF_HALTCOND*) override {
        const auto next = std::clamp(end+count, begin, static_cast<LONG>(doc->text.size())); *moved=next-end; end=next; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE IsEmpty(TfEditCookie, BOOL* out) override { *out = begin==end; return S_OK; }
    HRESULT STDMETHODCALLTYPE Collapse(TfEditCookie, TfAnchor anchor) override { if(anchor==TF_ANCHOR_START) end=begin; else begin=end; return S_OK; }
    HRESULT STDMETHODCALLTYPE Clone(ITfRange** out) override { *out = new Range(doc,begin,end); return S_OK; }
    HRESULT STDMETHODCALLTYPE CompareStart(TfEditCookie, ITfRange* other, TfAnchor anchor, LONG* out) override {
        const auto* r=static_cast<Range*>(other); *out=begin-(anchor==TF_ANCHOR_START?r->begin:r->end); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE CompareEnd(TfEditCookie, ITfRange* other, TfAnchor anchor, LONG* out) override {
        const auto* r=static_cast<Range*>(other); *out=end-(anchor==TF_ANCHOR_START?r->begin:r->end); return S_OK;
    }
    STUB(GetFormattedText, TfEditCookie, IDataObject**)
    STUB(GetEmbedded, TfEditCookie, REFGUID, REFIID, IUnknown**)
    STUB(InsertEmbedded, TfEditCookie, DWORD, IDataObject*)
    STUB(ShiftStartToRange, TfEditCookie, ITfRange*, TfAnchor)
    HRESULT STDMETHODCALLTYPE ShiftEndToRange(TfEditCookie, ITfRange* other, TfAnchor anchor) override {
        const auto* range = static_cast<Range*>(other); end = anchor == TF_ANCHOR_START ? range->begin : range->end; return S_OK;
    }
    STUB(ShiftStartRegion, TfEditCookie, TfShiftDir, BOOL*)
    STUB(ShiftEndRegion, TfEditCookie, TfShiftDir, BOOL*)
    STUB(IsEqualStart, TfEditCookie, ITfRange*, TfAnchor, BOOL*)
    STUB(IsEqualEnd, TfEditCookie, ITfRange*, TfAnchor, BOOL*)
    STUB(AdjustForInsert, TfEditCookie, ULONG, BOOL*)
    STUB(GetGravity, TfGravity*, TfGravity*)
    STUB(SetGravity, TfEditCookie, TfGravity, TfGravity)
    STUB(GetContext, ITfContext**)
};
struct Composition final : ITfComposition {
    COM_REFS(Composition)
    ComPtr<ITfRange> range; ComPtr<ITfCompositionSink> sink;
    bool notify_end{true};
    Composition(ITfRange* r, ITfCompositionSink* s) : range(r), sink(s) {}
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        *out=nullptr; if(id!=IID_IUnknown && id!=IID_ITfComposition) return E_NOINTERFACE;
        *out=static_cast<ITfComposition*>(this); AddRef(); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetRange(ITfRange** out) override { *out=range.Get(); range->AddRef(); return S_OK; }
    HRESULT STDMETHODCALLTYPE EndComposition(TfEditCookie cookie) override {
        auto saved=sink; sink.Reset(); if(saved && notify_end) saved->OnCompositionTerminated(cookie,this); return S_OK;
    }
    STUB(ShiftStart, TfEditCookie, ITfRange*)
    STUB(ShiftEnd, TfEditCookie, ITfRange*)
};
struct Context final : ITfContext, ITfContextComposition {
    COM_REFS(Context)
    Document doc;
    bool asynchronous{}, reject{}, reject_selection{};
    ComPtr<Composition> composition;
    std::vector<ComPtr<ITfEditSession>> queued;
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        *out=nullptr;
        if(id==IID_IUnknown || id==IID_ITfContext) *out=static_cast<ITfContext*>(this);
        else if(id==IID_ITfContextComposition) *out=static_cast<ITfContextComposition*>(this);
        else return E_NOINTERFACE;
        AddRef(); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE RequestEditSession(TfClientId, ITfEditSession* edit, DWORD flags, HRESULT* out) override {
        if(reject) { *out=E_ACCESSDENIED; return E_ACCESSDENIED; }
        if(asynchronous && !(flags&TF_ES_SYNC)) { queued.emplace_back(edit); *out=TF_S_ASYNC; }
        else *out=edit->DoEditSession(1);
        return S_OK;
    }
    void drain() { auto work=std::move(queued); queued.clear(); for(auto& edit:work) edit->DoEditSession(1); }
    HRESULT STDMETHODCALLTYPE GetSelection(TfEditCookie, ULONG, ULONG, TF_SELECTION* out, ULONG* fetched) override {
        *out={new Range(&doc,doc.caret,doc.end),{TF_AE_NONE,FALSE}}; *fetched=1; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetSelection(TfEditCookie, ULONG, const TF_SELECTION* selection) override {
        if (reject_selection) return E_FAIL;
        const auto* range=static_cast<Range*>(selection->range); doc.caret=range->begin; doc.end=range->end; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE StartComposition(TfEditCookie, ITfRange* range, ITfCompositionSink* sink, ITfComposition** out) override {
        composition.Attach(new Composition(range,sink)); *out=composition.Get(); composition->AddRef(); return S_OK;
    }
    STUB(InWriteSession, TfClientId, BOOL*)
    STUB(GetStart, TfEditCookie, ITfRange**)
    STUB(GetEnd, TfEditCookie, ITfRange**)
    STUB(GetActiveView, ITfContextView**)
    STUB(EnumViews, IEnumTfContextViews**)
    STUB(GetStatus, TF_STATUS*)
    STUB(GetProperty, REFGUID, ITfProperty**)
    STUB(GetAppProperty, REFGUID, ITfReadOnlyProperty**)
    STUB(TrackProperties, const GUID**, ULONG, const GUID**, ULONG, ITfReadOnlyProperty**)
    STUB(EnumProperties, IEnumTfProperties**)
    STUB(GetDocumentMgr, ITfDocumentMgr**)
    STUB(CreateRangeBackup, TfEditCookie, ITfRange*, ITfRangeBackup**)
    STUB(EnumCompositions, IEnumITfCompositionView**)
    STUB(FindComposition, TfEditCookie, ITfRange*, IEnumITfCompositionView**)
    STUB(TakeOwnership, TfEditCookie, ITfCompositionView*, ITfCompositionSink*, ITfComposition**)
};
#undef STUB
#undef COM_REFS
}

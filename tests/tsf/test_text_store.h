#pragma once
#include <Windows.h>
#include <msctf.h>
#include <textstor.h>
#include <inputscope.h>
#include <algorithm>
#include <string>

// GUID_PROP_INPUTSCOPE value; local storage keeps the helper-only target free
// from the production metadata inspection translation unit.
inline constexpr GUID kTestInputScopeProperty{0x1713dd5a,0x68e7,0x4a5b,{0x9a,0xf6,0x59,0x2a,0x59,0x5c,0x77,0x8d}};

class TestInputScope final : public ITfInputScope {
public:
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        *out = nullptr; if (id != IID_IUnknown && id != __uuidof(ITfInputScope)) return E_NOINTERFACE;
        *out = static_cast<ITfInputScope*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs_; }
    ULONG STDMETHODCALLTYPE Release() override { auto n = --refs_; if (!n) delete this; return n; }
    HRESULT STDMETHODCALLTYPE GetInputScopes(InputScope** scopes, UINT* count) override {
        *scopes = static_cast<InputScope*>(CoTaskMemAlloc(sizeof(InputScope))); if (!*scopes) return E_OUTOFMEMORY;
        **scopes = IS_DEFAULT; *count = 1; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetPhrase(BSTR** text, UINT* count) override { *text = nullptr; *count = 0; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetRegularExpression(BSTR* s) override { *s = nullptr; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetSRGS(BSTR* s) override { *s = nullptr; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetXML(BSTR* s) override { *s = nullptr; return S_OK; }
private:
    ULONG refs_{1};
};

// In-process application-owned text store. TSF contexts/ranges/edit cookies are
// provided by Windows msctf, not mocked by this fixture.
class TestTextStore final : public ITextStoreACP {
public:
    std::wstring text;
    bool fail_write{}, fail_selection{}, defer_write_lock{};
    DWORD deferred_lock{};
    DWORD last_lock_flags{};
    unsigned writes{}, locks{};
    HWND window{};
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        if (!out) return E_POINTER; *out = nullptr;
        if (id != IID_IUnknown && id != IID_ITextStoreACP) return E_NOINTERFACE;
        *out = static_cast<ITextStoreACP*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return ++refs_; }
    ULONG STDMETHODCALLTYPE Release() override { auto n = --refs_; if (!n) delete this; return n; }
    HRESULT STDMETHODCALLTYPE AdviseSink(REFIID id, IUnknown* object, DWORD) override {
        if (id != IID_ITextStoreACPSink) return E_NOINTERFACE;
        if (sink_) return E_UNEXPECTED;
        return object->QueryInterface(IID_PPV_ARGS(&sink_));
    }
    HRESULT STDMETHODCALLTYPE UnadviseSink(IUnknown*) override {
        if (sink_) { sink_->Release(); sink_ = nullptr; } return S_OK;
    }
    HRESULT STDMETHODCALLTYPE RequestLock(DWORD flags, HRESULT* result) override {
        last_lock_flags = flags;
        if (!result || !sink_) return E_INVALIDARG;
        if (lock_) { *result = TS_E_SYNCHRONOUS; return S_OK; }
        if (defer_write_lock && (flags & TS_LF_READWRITE) == TS_LF_READWRITE) {
            if (flags & TS_LF_SYNC) *result = TS_E_SYNCHRONOUS;
            else { deferred_lock = flags; *result = TS_S_ASYNC; }
            return S_OK;
        }
        lock_ = flags; ++locks; *result = sink_->OnLockGranted(flags); lock_ = 0; return S_OK;
    }
    HRESULT grant_deferred_lock() {
        if (!deferred_lock || lock_ || !sink_) return E_UNEXPECTED;
        lock_ = deferred_lock; deferred_lock = 0; ++locks;
        auto result = sink_->OnLockGranted(lock_); lock_ = 0; return result;
    }
    HRESULT STDMETHODCALLTYPE GetStatus(TS_STATUS* s) override { *s = {}; return S_OK; }
    HRESULT STDMETHODCALLTYPE QueryInsert(LONG a, LONG b, ULONG, LONG* x, LONG* y) override {
        if (a < 0 || b < a || b > static_cast<LONG>(text.size())) return E_INVALIDARG;
        *x = a; *y = b; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetSelection(ULONG index, ULONG count, TS_SELECTION_ACP* s, ULONG* got) override {
        if (!lock_) return TS_E_NOLOCK; *got = 0;
        if ((index != TS_DEFAULT_SELECTION && index != 0) || !count) return E_INVALIDARG;
        *s = selection_; *got = 1; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetSelection(ULONG count, const TS_SELECTION_ACP* s) override {
        if ((lock_ & TS_LF_READWRITE) != TS_LF_READWRITE) return TS_E_NOLOCK;
        if (fail_selection) return E_ACCESSDENIED;
        if (count != 1) return E_INVALIDARG; selection_ = *s; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetText(LONG a, LONG b, WCHAR* out, ULONG capacity, ULONG* got,
                                      TS_RUNINFO* runs, ULONG nruns, ULONG* ngot, LONG* next) override {
        if (!lock_) return TS_E_NOLOCK;
        if (b == -1) b = static_cast<LONG>(text.size());
        if (a < 0 || b < a || b > static_cast<LONG>(text.size())) return E_INVALIDARG;
        auto n = std::min<ULONG>(b - a, capacity);
        if (n) std::copy_n(text.data() + a, n, out);
        if (got) *got = n; if (next) *next = a + n;
        if (ngot) *ngot = nruns && n ? 1 : 0;
        if (nruns && n) runs[0] = {n, TS_RT_PLAIN}; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetText(DWORD, LONG a, LONG b, const WCHAR* value, ULONG n, TS_TEXTCHANGE* change) override {
        if ((lock_ & TS_LF_READWRITE) != TS_LF_READWRITE) return TS_E_NOLOCK;
        ++writes; if (fail_write) return E_ACCESSDENIED;
        if (a < 0 || b < a || b > static_cast<LONG>(text.size())) return E_INVALIDARG;
        text.replace(a, b - a, value, n); *change = {a, b, a + static_cast<LONG>(n)}; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetFormattedText(LONG, LONG, IDataObject**) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE GetEmbedded(LONG, REFGUID, REFIID, IUnknown**) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE QueryInsertEmbedded(const GUID*, const FORMATETC*, BOOL* value) override { *value = FALSE; return S_OK; }
    HRESULT STDMETHODCALLTYPE InsertEmbedded(DWORD, LONG, LONG, IDataObject*, TS_TEXTCHANGE*) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE InsertTextAtSelection(DWORD flags, const WCHAR* value, ULONG n, LONG* a, LONG* b, TS_TEXTCHANGE* change) override {
        if (!lock_) return TS_E_NOLOCK;
        if (a) *a = selection_.acpStart; if (b) *b = selection_.acpEnd;
        if (flags & TS_IAS_QUERYONLY) return S_OK;
        auto hr = SetText(0, selection_.acpStart, selection_.acpEnd, value, n, change);
        if (SUCCEEDED(hr)) { if (b) *b = change->acpNewEnd; selection_.acpEnd = change->acpNewEnd; }
        return hr;
    }
    HRESULT STDMETHODCALLTYPE InsertEmbeddedAtSelection(DWORD, IDataObject*, LONG*, LONG*, TS_TEXTCHANGE*) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE RequestSupportedAttrs(DWORD flags, ULONG count, const TS_ATTRID* attrs) override {
        return RequestAttrsAtPosition(0,count,attrs,flags);
    }
    HRESULT STDMETHODCALLTYPE RequestAttrsAtPosition(LONG, ULONG count, const TS_ATTRID* attrs, DWORD) override {
        scope_requested_ = false;
        for (ULONG i = 0; i < count; ++i) if (attrs[i] == kTestInputScopeProperty) scope_requested_ = true;
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE RequestAttrsTransitioningAtPosition(LONG, ULONG, const TS_ATTRID*, DWORD) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE FindNextAttrTransition(LONG, LONG halt, ULONG, const TS_ATTRID*, DWORD, LONG* next, BOOL* found, LONG* offset) override {
        *next = halt; *found = FALSE; *offset = 0; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE RetrieveRequestedAttrs(ULONG count, TS_ATTRVAL* attrs, ULONG* got) override {
        *got = 0; if (!count || !scope_requested_) return S_OK;
        attrs[0] = {}; attrs[0].idAttr = kTestInputScopeProperty;
        attrs[0].varValue.vt = VT_UNKNOWN; attrs[0].varValue.punkVal = new TestInputScope;
        *got = 1; scope_requested_ = false; return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetEndACP(LONG* end) override { if (!lock_) return TS_E_NOLOCK; *end = static_cast<LONG>(text.size()); return S_OK; }
    HRESULT STDMETHODCALLTYPE GetActiveView(TsViewCookie* view) override { *view = 1; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetACPFromPoint(TsViewCookie, const POINT*, DWORD, LONG*) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE GetTextExt(TsViewCookie, LONG, LONG, RECT* r, BOOL* clipped) override { *r = {0,0,100,20}; *clipped = FALSE; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetScreenExt(TsViewCookie, RECT* r) override { *r = {0,0,100,100}; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetWnd(TsViewCookie, HWND* w) override { *w = window; return S_OK; }
private:
    ~TestTextStore() { if (sink_) sink_->Release(); }
    ULONG refs_{1}; DWORD lock_{}; ITextStoreACPSink* sink_{}; bool scope_requested_{};
    TS_SELECTION_ACP selection_{0,0,{TS_AE_END,FALSE}};
};

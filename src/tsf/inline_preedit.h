#pragma once
#include <Windows.h>
#include <msctf.h>
#include <wrl/client.h>
#include <algorithm>
#include <functional>
#include <string>
#include <utility>
#include <cstdint>
#include <new>
#include <optional>

namespace owo::tsf {
// Per-host TSF composition, never an ordinary committed insertion. All edits
// run under the context's write lock. Queued stale updates are revision gated.
class InlinePreedit final : public ITfCompositionSink {
    template<class T> using Ptr = Microsoft::WRL::ComPtr<T>;
    class Edit final : public ITfEditSession {
        LONG refs_{1};
        Ptr<InlinePreedit> owner_;
        Ptr<ITfContext> context_;
        std::function<HRESULT(TfEditCookie)> action_;
    public:
        Edit(InlinePreedit* owner, ITfContext* context, std::function<HRESULT(TfEditCookie)> action)
            : owner_(owner), context_(context), action_(std::move(action)) {}
        HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
            if (!out) return E_POINTER; *out = nullptr;
            if (id != IID_IUnknown && id != IID_ITfEditSession) return E_NOINTERFACE;
            *out = static_cast<ITfEditSession*>(this); AddRef(); return S_OK;
        }
        ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&refs_); }
        ULONG STDMETHODCALLTYPE Release() override { const auto n = InterlockedDecrement(&refs_); if (!n) delete this; return n; }
        HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override { return action_(cookie); }
    };
    LONG refs_{1};
    static inline LONG live_{};
    Ptr<ITfContext> context_;
    Ptr<ITfComposition> composition_;
    TfClientId client_{};
    std::uint64_t revision_{};
    std::wstring requested_, applied_;
    std::size_t cursor_{};
    HWND notify_{};
    UINT message_{};
    bool failed_{}, committing_{}, terminated_{};
    std::optional<std::wstring> termination_commit_;
    void notify() { if (notify_) PostMessageW(notify_, message_, 0, 0); }
    HRESULT request(ITfContext* context, std::function<HRESULT(TfEditCookie)> action) {
        auto* edit = new (std::nothrow) Edit(this, context, std::move(action));
        if (!edit) return E_OUTOFMEMORY;
        HRESULT result = E_FAIL;
        const auto status = context->RequestEditSession(client_, edit, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE, &result);
        edit->Release();
        return FAILED(status) ? status : result;
    }
    static bool unchanged(ITfRange* range, TfEditCookie cookie, const std::wstring& text) {
        std::wstring buffer(text.size() + 1, L'\0'); ULONG read = 0;
        return SUCCEEDED(range->GetText(cookie, 0, buffer.data(), static_cast<ULONG>(buffer.size()), &read)) &&
            read == text.size() && buffer.compare(0, read, text) == 0;
    }
public:
    InlinePreedit() { InterlockedIncrement(&live_); }
    ~InlinePreedit() { InterlockedDecrement(&live_); }
    static LONG live_objects() { return InterlockedCompareExchange(&live_, 0, 0); }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void** out) override {
        if (!out) return E_POINTER; *out = nullptr;
        if (id != IID_IUnknown && id != IID_ITfCompositionSink) return E_NOINTERFACE;
        *out = static_cast<ITfCompositionSink*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&refs_); }
    ULONG STDMETHODCALLTYPE Release() override { const auto n = InterlockedDecrement(&refs_); if (!n) delete this; return n; }
    void set_notification(HWND window, UINT message) { notify_ = window; message_ = message; }
    void set_termination_commit(std::optional<std::wstring> text) { termination_commit_ = std::move(text); }
    bool active() const { return composition_ && !failed_; }
    bool owns_range(ITfContext* context) const { return composition_ && context_.Get() == context; }
    bool allows_selection(ITfContext* context, TfEditCookie cookie) const {
        if (!owns_range(context)) return false;
        Ptr<ITfRange> range;
        TF_SELECTION selection{}; ULONG fetched{};
        if (FAILED(composition_->GetRange(&range)) || !unchanged(range.Get(), cookie, applied_)) return false;
        const auto status = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &fetched);
        Ptr<ITfRange> caret; caret.Attach(selection.range);
        LONG left{}, right{}; BOOL empty{};
        return SUCCEEDED(status) && fetched == 1 && caret && SUCCEEDED(caret->IsEmpty(cookie, &empty)) && empty &&
            SUCCEEDED(caret->CompareStart(cookie, range.Get(), TF_ANCHOR_START, &left)) && left >= 0 &&
            SUCCEEDED(caret->CompareEnd(cookie, range.Get(), TF_ANCHOR_END, &right)) && right <= 0;
    }
    bool caret_offset(ITfContext* context, TfClientId client, std::size_t& offset) {
        if (!composition_ || context_.Get() != context) return false;
        bool inside = true;
        auto* edit = new (std::nothrow) Edit(this, context, [&](TfEditCookie cookie) -> HRESULT {
            Ptr<ITfRange> range; TF_SELECTION selection{}; ULONG fetched = 0;
            if (!composition_ || FAILED(composition_->GetRange(&range)) ||
                FAILED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &fetched)) || !selection.range)
                return E_FAIL;
            Ptr<ITfRange> caret; caret.Attach(selection.range);
            LONG left = 0, right = 0; BOOL empty = FALSE;
            if (FAILED(caret->CompareStart(cookie, range.Get(), TF_ANCHOR_START, &left)) ||
                FAILED(caret->CompareEnd(cookie, range.Get(), TF_ANCHOR_END, &right))) return E_FAIL;
            inside = left >= 0 && right <= 0 && SUCCEEDED(caret->IsEmpty(cookie, &empty)) && empty &&
                unchanged(range.Get(), cookie, applied_);
            if (!inside) return S_OK;
            Ptr<ITfRange> prefix_range;
            if (FAILED(range->Clone(&prefix_range)) ||
                FAILED(prefix_range->ShiftEndToRange(cookie, caret.Get(), TF_ANCHOR_START))) return E_FAIL;
            std::wstring prefix(applied_.size() + 1, L'\0'); ULONG count = 0;
            if (SUCCEEDED(prefix_range->GetText(cookie, 0, prefix.data(), static_cast<ULONG>(prefix.size()), &count))) offset = count;
            return S_OK;
        });
        if (!edit) return true;
        HRESULT result = E_FAIL;
        context->RequestEditSession(client, edit, TF_ES_SYNC | TF_ES_READ, &result);
        edit->Release();
        return inside;
    }
    bool take_termination() { return std::exchange(terminated_, false); }
    void update(ITfContext* context, TfClientId client, std::wstring text, std::size_t cursor) {
        if (!context || text.empty()) { cancel(); return; }
        if (context_.Get() != context) { cancel(); context_ = context; failed_ = false; }
        if (failed_ || (requested_ == text && cursor_ == cursor)) return;
        client_ = client; requested_ = std::move(text); cursor_ = cursor; committing_ = false;
        const auto revision = ++revision_;
        const auto status = request(context, [this, revision](TfEditCookie cookie) -> HRESULT {
            if (revision != revision_ || !context_) return S_FALSE;
            Ptr<ITfRange> range;
            HRESULT result = S_OK;
            if (!composition_) {
                TF_SELECTION selection{}; ULONG fetched = 0;
                result = context_->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &fetched);
                if (FAILED(result) || fetched != 1 || !selection.range) return fail(FAILED(result) ? result : E_FAIL);
                range.Attach(selection.range);
                // Do not overwrite selected application text merely to preview.
                BOOL empty = FALSE;
                if (FAILED(range->IsEmpty(cookie, &empty)) || !empty) return fail(E_FAIL);
                Ptr<ITfContextComposition> manager;
                result = context_.As(&manager);
                if (SUCCEEDED(result)) result = manager->StartComposition(cookie, range.Get(), this, &composition_);
                if (FAILED(result) || !composition_) return fail(FAILED(result) ? result : E_FAIL);
            }
            result = composition_->GetRange(&range);
            if (SUCCEEDED(result) && !applied_.empty() && !unchanged(range.Get(), cookie, applied_))
                return fail(E_ABORT);
            if (SUCCEEDED(result)) result = range->SetText(cookie, 0, requested_.data(), static_cast<LONG>(requested_.size()));
            if (FAILED(result)) return fail(result);
            applied_ = requested_;
            // A selection wholly within the composition is the real preedit
            // caret; it is never extended over committed application text.
            Ptr<ITfRange> caret;
            if (SUCCEEDED(range->Clone(&caret)) && SUCCEEDED(caret->Collapse(cookie, TF_ANCHOR_START))) {
                LONG moved = 0;
                if (SUCCEEDED(caret->ShiftEnd(cookie, static_cast<LONG>(std::min(cursor_, applied_.size())), &moved, nullptr)) &&
                    SUCCEEDED(caret->Collapse(cookie, TF_ANCHOR_END))) {
                    TF_SELECTION selection{caret.Get(), {TF_AE_NONE, FALSE}};
                    context_->SetSelection(cookie, 1, &selection);
                }
            }
            notify(); return S_OK;
        });
        if (FAILED(status)) fail(status);
    }
    ITfComposition* prepare_commit(ITfContext* context) {
        if (context_.Get() != context) { cancel(); return nullptr; }
        ++revision_; requested_.clear(); committing_ = true;
        return composition_.Get();
    }
    void finish_commit(bool success) {
        // EndComposition need not call the forced-termination sink. Never
        // reuse an ended range for the remaining pinyin after a short commit.
        if (success) { composition_.Reset(); context_.Reset(); applied_.clear(); }
        committing_ = false;
    }
    void cancel() {
        ++revision_; requested_.clear(); failed_ = false; committing_ = false;
        auto context = context_; auto composition = composition_; const auto text = applied_;
        composition_.Reset(); context_.Reset(); applied_.clear();
        if (!context || !composition) return;
        request(context.Get(), [context, composition, text](TfEditCookie cookie) -> HRESULT {
            Ptr<ITfRange> range;
            if (SUCCEEDED(composition->GetRange(&range)) && unchanged(range.Get(), cookie, text))
                range->SetText(cookie, 0, L"", 0);
            return composition->EndComposition(cookie);
        });
    }
    HRESULT STDMETHODCALLTYPE OnCompositionTerminated(TfEditCookie cookie, ITfComposition* composition) override {
        if (composition != composition_.Get()) return S_OK;
        if (!committing_) {
            Ptr<ITfRange> range;
            if (SUCCEEDED(composition->GetRange(&range)) && unchanged(range.Get(), cookie, applied_))
                range->SetText(cookie, 0, termination_commit_ ? termination_commit_->data() : L"",
                    termination_commit_ ? static_cast<LONG>(termination_commit_->size()) : 0);
            terminated_ = true;
        }
        ++revision_; composition_.Reset(); requested_.clear(); applied_.clear();
        notify(); return S_OK;
    }
private:
    HRESULT fail(HRESULT result) { failed_ = true; notify(); return result; }
};
}

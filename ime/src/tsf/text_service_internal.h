#pragma once
#include <Windows.h>
#include <msctf.h>
#include "owo/agent/agent_protocol.h"
#include <functional>
#include <optional>
#include <string>
#include <string_view>

namespace owo::tsf::detail {
inline constexpr UINT kToneReady = WM_APP + 7;
inline constexpr UINT_PTR kToneTimer = 0x4f59;
class SchemeEditSession final : public ITfEditSession {
public:
    SchemeEditSession(ITfContext* context, std::function<HRESULT(TfEditCookie)> action,
                      IUnknown* owner = nullptr)
        : context_(context), action_(std::move(action)), owner_(owner) {
        context_->AddRef(); if (owner_) owner_->AddRef();
    }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override {
        if (!object) return E_POINTER;
        *object = nullptr;
        if (iid != IID_IUnknown && iid != IID_ITfEditSession) return E_NOINTERFACE;
        *object = static_cast<ITfEditSession*>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return static_cast<ULONG>(InterlockedIncrement(&refs_)); }
    ULONG STDMETHODCALLTYPE Release() override {
        const auto count = InterlockedDecrement(&refs_); if (!count) delete this;
        return static_cast<ULONG>(count);
    }
    HRESULT STDMETHODCALLTYPE DoEditSession(TfEditCookie cookie) override {
        try { return action_(cookie); } catch (...) { return E_FAIL; }
    }
private:
    ~SchemeEditSession() { context_->Release(); if (owner_) owner_->Release(); }
    LONG refs_{1}; ITfContext* context_; std::function<HRESULT(TfEditCookie)> action_; IUnknown* owner_;
};

std::string utf8_from_wide(std::wstring_view);
std::wstring wide_from_utf8(std::string_view);
bool key_down(int) noexcept;
bool command_modifier_down() noexcept;
std::wstring scheme_key_text(WPARAM, LPARAM);
std::optional<std::wstring> read_range_text(TfEditCookie, ITfRange*);
bool scheme_selection_matches(ITfContext*, TfEditCookie, ITfRange*);
std::uint64_t current_unix_time_ms();
agent::ApplicationContext current_agent_application(std::string_view, bool);
}  // namespace owo::tsf::detail

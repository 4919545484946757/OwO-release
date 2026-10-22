#include "text_service.h"
#include "text_service_internal.h"
#include "owo/config/config_paths.h"
#include "owo/tsf/agent_writeback.h"
#include "owo/tsf/input_method_context.h"
#include <algorithm>
#include <new>
#include <utility>


namespace owo::tsf {
using namespace detail;
bool TextService::tone_shortcut(WPARAM key) const {
    return (key == 'R' || key == 'J') && key_down(VK_CONTROL) && key_down(VK_SHIFT) &&
        !key_down(VK_MENU) && !key_down(VK_LWIN) && !key_down(VK_RWIN) &&
        (key == 'R' ? shortcut_config_.tone_reply_enabled : shortcut_config_.tone_continuation_enabled);
}
bool TextService::tone_preview_key(WPARAM key) const {
    return key != VK_MENU && key != VK_LMENU && key != VK_RMENU &&
        key != VK_LWIN && key != VK_RWIN && !key_down(VK_MENU) &&
        !key_down(VK_LWIN) && !key_down(VK_RWIN);
}
void TextService::cancel_tone() noexcept {
    tone_active_ = tone_pending_ = tone_editing_ = false;
    tone_drafts_.clear(); tone_edited_.clear(); tone_message_.clear(); tone_notice_.clear();
    if (tone_worker_.joinable()) tone_worker_.request_stop();
    tone_preview_.hide();
    if (message_window_) KillTimer(message_window_, kToneTimer);
}
void TextService::begin_tone(ITfContext* context, engine::ToneDraftMode mode) {
    clear_composition();
    if (tone_worker_.joinable()) tone_worker_.join();
    { std::lock_guard lock(tone_result_mutex_); tone_result_.reset(); }
    if (!capture_agent_origin(context) || !agent_origin_selection_ || agent_origin_selection_->redacted) return;
    const auto application = current_agent_application(agent_context_id_, agent_context_sensitive_);
    if (application.sensitive_input || !engine::tone_session_allowed(shortcut_config_, application.executable, mode)) {
        reset_agent_origin(); return;
    }
    const auto filtered = agent::filter_sensitive_text(agent_origin_selection_->text, engine::kMaximumToneMessageBytes);
    if (!filtered.ok || filtered.redacted) { reset_agent_origin(); return; }
    tone_message_ = agent_origin_selection_->text;
    if (tone_message_.size() > engine::kMaximumToneMessageBytes) { reset_agent_origin(); return; }
    tone_application_ = application.executable;
    tone_original_ = tone_message_;
    tone_config_ = shortcut_config_; tone_mode_ = mode;
    tone_drafts_.clear(); tone_index_ = 0; tone_edited_.clear();
    tone_active_ = tone_pending_ = true;
    tone_notice_ = L"正在生成草稿；原选区保持不变，Esc 取消。";
    update_candidate_anchor(context); update_tone_preview();
    if (!tone_active_) return;
    if (!SetTimer(message_window_, kToneTimer, 200, nullptr)) { clear_composition(); return; }
    const auto root = config::local_data_root(); const auto path = config::default_config_path();
    const auto app = tone_application_, message = tone_message_;
    tone_worker_ = std::jthread([this, root, path, app, message, mode](std::stop_token stop) {
        auto result = engine::generate_tone_drafts(root, path, app, message, mode, stop);
        if (stop.stop_requested()) return;
        { std::lock_guard lock(tone_result_mutex_); tone_result_ = std::move(result); }
        PostMessageW(message_window_, kToneReady, 0, 0);
    });
}
void TextService::handle_tone_result() {
    std::optional<engine::ToneGeneration> result;
    { std::lock_guard lock(tone_result_mutex_); result.swap(tone_result_); }
    if (!result || !tone_active_ || !tone_pending_) return;
    refresh_shortcut_config(true);
    if (!agent_origin_valid_ || tone_config_ != shortcut_config_) { clear_composition(); return; }
    tone_pending_ = false;
    for (auto& draft : result->selection.candidates) tone_drafts_.push_back(std::move(draft.text));
    if (tone_drafts_.empty()) tone_notice_ = wide_from_utf8(result->diagnostic);
    else { tone_edited_ = wide_from_utf8(tone_drafts_.front()); tone_edit_cursor_ = tone_edited_.size(); tone_notice_.clear(); }
    update_tone_preview();
}
void TextService::update_tone_preview() {
    if (!tone_active_) return;
    auto status = tone_notice_;
    if (status.empty()) status = L"草稿 " + std::to_wstring(tone_index_ + 1) + L"/" +
        std::to_wstring(tone_drafts_.size()) + (tone_editing_ ? L" · 修改中（左右移动、退格/删除）" : L" · 请核对事实再确认");
    POINT anchor = candidate_anchor_; anchor.y += 75;
    auto shown = tone_edited_;
    if (tone_editing_) shown.insert(std::min(tone_edit_cursor_, shown.size()), L"▏");
    if (!tone_preview_.show(message_window_, anchor, status, shown)) clear_composition();
}
HRESULT TextService::append_tone_edit(std::wstring_view text) {
    if (!tone_active_ || !tone_editing_ || !agent_origin_valid_) return E_ACCESSDENIED;
    auto proposed = tone_edited_; proposed.insert(std::min(tone_edit_cursor_, proposed.size()), text);
    const auto encoded = utf8_from_wide(proposed);
    if (encoded.empty() || encoded.size() > engine::kMaximumToneDraftBytes ||
        std::any_of(encoded.begin(), encoded.end(), [](unsigned char ch) { return ch < 0x20 || ch == 0x7f; })) return E_INVALIDARG;
    tone_edited_ = std::move(proposed); tone_edit_cursor_ += text.size(); update_tone_preview(); return S_OK;
}
void TextService::handle_tone_key(ITfContext* context, WPARAM key, LPARAM flags) {
    if (!agent_origin_valid_ || !plugin_context_is_current(agent_origin_context_)) { clear_composition(); return; }
    refresh_agent_context_safety(context);
    if (agent_context_sensitive_) { clear_composition(); return; }
    if (key == VK_ESCAPE) {
        if (!tone_pending_ && tone_index_ < tone_drafts_.size())
            (void)record_tone_feedback(engine::ToneFeedbackKind::rejected, tone_drafts_[tone_index_], {});
        clear_composition(); return;
    }
    if (tone_pending_ || tone_drafts_.empty()) return;
    if (key == VK_F6 && !has_composition()) {
        tone_index_ = (tone_index_ + 1) % tone_drafts_.size();
        tone_edited_ = wide_from_utf8(tone_drafts_[tone_index_]); tone_edit_cursor_ = tone_edited_.size(); tone_notice_.clear(); update_tone_preview(); return;
    }
    if (key == VK_F7 && !has_composition()) {
        tone_editing_ = !tone_editing_; tone_notice_.clear(); update_tone_preview(); return;
    }
    if (key == VK_RETURN && key_down(VK_CONTROL) && !has_composition()) { confirm_tone(); return; }
    if (!tone_editing_) return;
    if (command_modifier_down()) return;
    if (key >= 'A' && key <= 'Z' && chinese_mode_) {
        if (input_buffer_.size() >= 256) return;
        const bool upper = key_down(VK_SHIFT) != ((GetKeyState(VK_CAPITAL) & 1) != 0);
        input_buffer_.push_back(static_cast<wchar_t>((upper ? L'A' : L'a') + key - 'A'));
        input_cursor_ = input_buffer_.size(); ++context_generation_;
        candidate_page_ = 0; queue_candidate_request(); return;
    }
    if (has_composition()) {
        if (key == VK_BACK) {
            if (!input_buffer_.empty()) input_buffer_.pop_back();
            input_cursor_ = input_buffer_.size(); ++context_generation_;
            if (!input_buffer_.empty()) queue_candidate_request();
            else { candidates_.clear(); candidate_consumed_.clear(); candidate_groups_.clear();
                candidate_request_pending_ = false; hide_candidate_windows(); }
        } else if (key == VK_RETURN) (void)commit_raw_input(context);
        else if (key == VK_SPACE || (key >= '1' && key <= '9')) {
            if (candidate_request_pending_) return;
            const auto index = key == VK_SPACE ? 0 : static_cast<std::size_t>(key - '1');
            if (index < numbered_candidate_count()) (void)commit_candidate(context, index);
        } else if (key == VK_NEXT && has_more_candidates_) change_candidate_page(1);
        else if (key == VK_PRIOR && candidate_page_) change_candidate_page(-1);
        return;
    }
    tone_edit_cursor_ = std::min(tone_edit_cursor_, tone_edited_.size());
    const auto previous = [&] {
        auto at = tone_edit_cursor_;
        if (at) { --at; if (at && tone_edited_[at] >= 0xdc00 && tone_edited_[at] <= 0xdfff) --at; }
        return at;
    };
    const auto next = [&] {
        auto at = tone_edit_cursor_;
        if (at < tone_edited_.size()) {
            if (tone_edited_[at] >= 0xd800 && tone_edited_[at] <= 0xdbff) ++at;
            ++at;
        }
        return std::min(at, tone_edited_.size());
    };
    if (key == VK_BACK) {
        const auto at = previous(); tone_edited_.erase(at, tone_edit_cursor_ - at); tone_edit_cursor_ = at;
    } else if (key == VK_DELETE) tone_edited_.erase(tone_edit_cursor_, next() - tone_edit_cursor_);
    else if (key == VK_LEFT) tone_edit_cursor_ = previous();
    else if (key == VK_RIGHT) tone_edit_cursor_ = next();
    else if (key == VK_HOME) tone_edit_cursor_ = 0;
    else if (key == VK_END) tone_edit_cursor_ = tone_edited_.size();
    else goto printable;
    update_tone_preview(); return;
printable:
    if (!command_modifier_down() && key != VK_RETURN && key != VK_TAB &&
        key != VK_DELETE && key != VK_LEFT && key != VK_RIGHT && key != VK_UP &&
        key != VK_DOWN && key != VK_HOME && key != VK_END) {
        const auto text = scheme_key_text(key, flags);
        if (!text.empty()) (void)append_tone_edit(text);
    }
    (void)context;
}
std::uint64_t TextService::record_tone_feedback(engine::ToneFeedbackKind kind,
    std::string_view draft, std::string_view final_text, std::uint64_t replace_id) {
    config::ConfigStore config;
    const auto root = config::local_data_root();
    if (root.empty() || !config.load(config::default_config_path()).success || !config.reload().success ||
        !engine::tone_session_allowed(config.snapshot(), tone_application_, tone_mode_)) return 0;
    std::uint64_t id = 0;
    const auto result = engine::ToneFeedbackStore::modify(root / L"data" / L"tone-feedback.owtf",
        [&](engine::ToneFeedbackStore& store) {
            if (replace_id) {
                id = store.undo(replace_id, tone_application_, tone_adopted_draft_,
                    tone_committed_, current_unix_time_ms());
                return;
            }
            id = store.append(kind, tone_application_, draft, final_text, current_unix_time_ms());
        });
    return result.success ? id : 0;
}
void TextService::confirm_tone() {
    refresh_shortcut_config(true);
    if (!tone_active_ || tone_config_ != shortcut_config_ || !agent_origin_valid_ ||
        !engine::tone_session_allowed(shortcut_config_, tone_application_, tone_mode_) ||
        tone_index_ >= tone_drafts_.size()) { clear_composition(); return; }
    config::ConfigStore live;
    if (!live.load(config::default_config_path()).success || !live.reload().success ||
        live.snapshot() != tone_config_) { clear_composition(); return; }
    const auto final_text = utf8_from_wide(tone_edited_);
    if (final_text.empty() || final_text.size() > engine::kMaximumToneDraftBytes ||
        std::any_of(final_text.begin(), final_text.end(), [](unsigned char ch) { return ch < 0x20 || ch == 0x7f; })) return;
    const auto draft = tone_drafts_[tone_index_];
    const auto original = tone_original_;
    auto* target = agent_origin_context_; target->AddRef();
    const agent::CommandEffect effect{agent::EffectKind::replace_selection, final_text,
        agent_context_id_, agent_origin_sha256_};
    const auto outcome = apply_agent_effect(effect, true);
    if (outcome == NativeWriteResult::completed) {
        const auto id = record_tone_feedback(final_text == draft ? engine::ToneFeedbackKind::adopted : engine::ToneFeedbackKind::edited,
            draft, final_text);
        clear_composition();
        tone_adopted_draft_ = draft; tone_committed_ = final_text; tone_original_ = original;
        // Capture the new caret. Any later host edit/focus change retires this receipt.
        if (capture_agent_origin(target) && !agent_origin_selection_) {
            tone_adoption_id_ = id; tone_undo_available_ = true;
        }
    } else {
        tone_notice_ = L"未确认上屏成功；本次目标已停用，请 Esc 关闭后重新选择。";
        update_tone_preview();
    }
    target->Release();
}
bool TextService::undo_tone() {
    if (!tone_undo_available_ || !agent_origin_valid_ || !agent_origin_context_ ||
        !plugin_context_is_current(agent_origin_context_)) return false;
    bool undone = false;
    const auto id = tone_adoption_id_;
    const auto committed = wide_from_utf8(tone_committed_);
    const auto original = wide_from_utf8(tone_original_);
    auto* context = agent_origin_context_;
    auto* edit = new (std::nothrow) SchemeEditSession(context, [&](TfEditCookie cookie) {
        if (!agent_origin_valid_ || !scheme_selection_matches(context, cookie, agent_origin_range_) ||
            inspect_input_method_context(context, cookie) != InputMethodContextSafety::allowed) return E_ACCESSDENIED;
        ITfRange* range = nullptr;
        if (FAILED(agent_origin_range_->Clone(&range)) || !range) return E_FAIL;
        LONG shifted{};
        const auto count = static_cast<LONG>(committed.size());
        const bool match = SUCCEEDED(range->ShiftStart(cookie, -count, &shifted, nullptr)) &&
            shifted == -count && read_range_text(cookie, range) == std::optional<std::wstring>(committed);
        if (match) {
            agent_origin_valid_ = false; tone_undo_available_ = false; tone_adoption_id_ = 0;
            if (SUCCEEDED(range->SetText(cookie, 0, original.data(), static_cast<LONG>(original.size())))) {
                undone = read_range_text(cookie, range) == std::optional<std::wstring>(original);
                (void)range->Collapse(cookie, TF_ANCHOR_END);
                TF_SELECTION selection{range, {TF_AE_NONE, FALSE}};
                (void)context->SetSelection(cookie, 1, &selection);
            }
        }
        range->Release(); return S_OK;
    });
    HRESULT result = E_FAIL;
    if (edit) { (void)context->RequestEditSession(client_id_, edit, TF_ES_SYNC | TF_ES_READWRITE, &result); edit->Release(); }
    if (undone) {
        if (id) (void)record_tone_feedback(engine::ToneFeedbackKind::undone, tone_committed_, {}, id);
        recent_language_context_.clear(); recent_language_input_.clear();
    }
    // Consume the key after a write attempt even when readback was uncertain.
    return undone || !tone_undo_available_;
}
}  // namespace owo::tsf

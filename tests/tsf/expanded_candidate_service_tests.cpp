#include "text_service.h"
#include <d2d1.h>
#include <cmath>
#include <iostream>
#include <limits>

namespace owo::tsf {
// 不激活 TSF/工作线程：验证真实按键、异步响应及离屏窗口的像素边界。
struct ExpandedCandidateTestAccess {
    static int run() {
        auto* service = new TextService();
        struct Release { TextService* value; ~Release() { value->Release(); } } release{service};
        service->shortcut_config_initialized_ = true;
        service->next_shortcut_config_refresh_ = std::numeric_limits<ULONGLONG>::max();
        service->input_buffer_ = L"nihao";
        service->input_cursor_ = 5;
        for (const bool light : {false, true}) for (const bool expanded : {false, true})
            for (const bool hovered : {false, true}) for (const bool pressed : {false, true}) {
                service->candidate_light_palette_ = light;
                service->candidates_expanded_ = expanded;
                const auto expected = light && expanded && !hovered && !pressed ? 0.8F : 1.0F;
                if (service->candidate_highlight_scale(hovered, pressed) != expected) return 14;
                if (light && expanded && !hovered && !pressed) {
                    const auto reference_fill = 0.05F * service->candidate_highlight_scale(hovered, pressed);
                    if (reference_fill < 0.0399F || reference_fill > 0.0401F) return 15;
                }
            }
        service->candidate_light_palette_ = false;
        service->candidates_expanded_ = false;
        auto respond = [&](std::size_t count, std::size_t size = 7) {
            auto* result = new TextService::CandidateResult();
            result->request_id = service->active_candidate_request_id_;
            result->generation = service->context_generation_;
            result->page = service->candidate_page_;
            result->expanded = service->candidates_expanded_;
            result->page_size = size;
            result->has_more = true;
            for (std::size_t i = 0; i < count; ++i) {
                result->candidates.push_back(L"fixture-" + std::to_wstring(i));
                result->candidate_consumed.push_back(i % 5 + 1);
                result->candidate_groups.push_back(i % 3);
            }
            service->handle_candidate_result(result);
        };
        auto press = [&](WPARAM key) {
            BOOL eaten = FALSE;
            return SUCCEEDED(service->OnKeyDown(nullptr, key, 0, &eaten)) && eaten;
        };
        // Exercise actual dispatch with modifier state, not just stored strings.
        service->shortcut_config_.separator_shortcuts = {"Shift+Backslash"};
        BYTE modifiers[256]{};
        if (service->shortcut_matches({"Shift+Backslash"}, VK_OEM_5) ||
            !service->shortcut_matches({"Backslash"}, VK_OEM_5)) return 48;
        modifiers[VK_SHIFT] = 0x80;
        if (!SetKeyboardState(modifiers) ||
            !service->shortcut_matches({"Shift+Backslash"}, VK_OEM_5) ||
            service->shortcut_matches({"Backslash"}, VK_OEM_5) ||
            !press(VK_OEM_5) || service->input_buffer_ != L"nihao'" ||
            service->manual_breaks_ != std::vector<std::size_t>{5}) return 49;
        modifiers[VK_CONTROL] = 0x80;
        if (!SetKeyboardState(modifiers) ||
            !service->shortcut_matches({"Ctrl+Shift+Backslash"}, VK_OEM_5) ||
            service->shortcut_matches({"Shift+Backslash"}, VK_OEM_5)) return 50;
        modifiers[VK_SHIFT] = modifiers[VK_CONTROL] = 0;
        if (!SetKeyboardState(modifiers)) return 51;
        service->clear_composition();
        service->input_buffer_ = L"nihao";
        service->input_cursor_ = 5;
        service->shortcut_config_.separator_shortcuts = {"F7"};
        service->shortcut_config_.pinyin_symbol_compatibility_enabled = true;
        service->shortcut_config_.input_scheme = "exact_code";
        if (service->should_eat_key(VK_F7) || service->symbol_composition_enabled()) return 34;
        service->shortcut_config_.input_scheme = "full_pinyin";
        service->input_cursor_ = 2;
        if (!press(VK_F7) || service->input_buffer_ != L"ni'hao" ||
            service->manual_breaks_ != std::vector<std::size_t>{2} ||
            !service->pending_request_->symbol_composition ||
            service->pending_request_->manual_breaks != std::vector<std::uint64_t>{2}) return 30;
        if (!press(VK_BACK) || service->input_buffer_ != L"nihao" || !service->manual_breaks_.empty()) return 31;
        service->staged_candidates_.push_back({L"你好", L"ni'hao", {2}});
        service->input_buffer_ = L"shijie";
        service->restore_staged_suffix(0);
        if (service->input_buffer_ != L"ni'haoshijie" || service->manual_breaks_ != std::vector<std::size_t>{2}) return 32;
        service->consume_manual_breaks(6);
        if (!service->manual_breaks_.empty()) return 33;
        service->clear_composition();
        service->shortcut_config_.separator_shortcuts = {"Quote"};
        service->shortcut_config_.pinyin_symbol_compatibility_enabled = false;
        service->input_buffer_ = L"nihao";
        service->input_cursor_ = 5;
        respond(7);
        const auto collapsed_first_request = service->active_candidate_request_id_;
        const auto collapsed_first_candidates = service->candidates_;
        for (int repeat = 0; repeat < 3; ++repeat)
            if (!press(VK_UP) || service->candidates_expanded_ || service->candidate_request_pending_ ||
                service->active_candidate_request_id_ != collapsed_first_request ||
                service->candidate_page_ != 0 || service->candidates_ != collapsed_first_candidates) return 59;
        service->candidate_request_pending_ = true;
        if (!press(VK_UP) || service->candidates_expanded_ || !service->candidate_request_pending_ ||
            service->active_candidate_request_id_ != collapsed_first_request) return 60;
        respond(7);
        if (!press(VK_DOWN) || !service->candidates_expanded_ || !service->candidate_request_pending_ ||
            service->expanded_pages_.selected != 1) return 1;
        const auto request = service->active_candidate_request_id_;
        if (!press(VK_DOWN) || service->active_candidate_request_id_ != request ||
            service->expanded_pages_.selected != 2) return 2;
        respond(49);
        if (service->candidate_request_pending_ || service->candidate_shortcut_index(1) != 15) return 3;
        if (!press(VK_DOWN) || service->active_candidate_request_id_ != request ||
            service->candidate_shortcut_index(1) != 22) return 4;
        service->invoke_hit_target({TextService::HitKind::candidate_page, 5});
        if (service->expanded_pages_.selected != 5 || service->candidate_shortcut_index(0) != 35) return 5;
        service->invoke_hit_target({TextService::HitKind::toggle_expanded, 0});
        if (service->candidates_expanded_ || service->candidate_page_ != 5) return 6;
        if (service->candidates_.size() != 7 || service->candidates_.front() != L"fixture-35" ||
            service->candidate_consumed_.size() != 7 || service->candidate_consumed_.front() != 1 ||
            service->candidate_groups_.size() != 7 || service->candidate_groups_.front() != 2) return 23;
        respond(7);
        service->invoke_hit_target({TextService::HitKind::toggle_expanded, 0});
        respond(49);
        if (!service->candidates_expanded_ || service->expanded_pages_.selected != 5) return 7;
        service->scroll_expanded_candidates(-20);
        if (service->expanded_pages_.first_visible != 0 || service->expanded_pages_.selected >= 5) return 8;
        service->shortcut_config_.expanded_previous_page_shortcut_enabled = false;
        service->shortcut_config_.expanded_next_page_shortcut_enabled = false;
        if (service->should_eat_key(VK_UP) || service->should_eat_key(VK_DOWN)) return 9;
        service->shortcut_config_.expanded_next_page_shortcut_enabled = true;
        service->shortcut_config_.expanded_next_page_shortcuts = {"F7"};
        if (service->should_eat_key(VK_DOWN) || !press(VK_F7)) return 10;
        respond(15, 7);
        service->expanded_pages_.selected = 2;
        service->expanded_pages_.ensure_visible(15, 7, 5);
        if (service->candidate_shortcut_index(0) != 14 || service->candidate_shortcut_index(1)) return 11;
        const auto selected = service->expanded_pages_.selected;
        auto* stale = new TextService::CandidateResult();
        stale->request_id = service->active_candidate_request_id_ + 1;
        stale->generation = service->context_generation_;
        stale->expanded = true;
        service->handle_candidate_result(stale);
        if (service->expanded_pages_.selected != selected || service->candidates_.size() != 15) return 12;
        service->shortcut_config_.expanded_previous_page_shortcut_enabled = true;
        service->shortcut_config_.expanded_previous_page_shortcuts = {"F8"};
        if (service->should_eat_key(VK_UP) || !press(VK_F8) ||
            !service->candidates_expanded_ || service->expanded_pages_.selected != 1) return 16;
        if (!press(VK_F8) || !service->candidates_expanded_ ||
            service->expanded_pages_.selected != 0) return 17;
        if (!press(VK_F8) || service->candidates_expanded_ ||
            service->candidate_page_ != 0 || service->candidate_shortcut_index(0) != 0 ||
            service->input_buffer_ != L"nihao") return 18;
        respond(7);
        // The same first-page guard applies to remapped previous-page keys.
        const auto collapsed_again_request = service->active_candidate_request_id_;
        if (!press(VK_F8) || service->candidates_expanded_ || service->candidate_request_pending_ ||
            service->active_candidate_request_id_ != collapsed_again_request) return 61;
        // Explicit expansion at page one remains supported while loading.
        service->invoke_hit_target({TextService::HitKind::toggle_expanded, 0});
        if (!service->candidates_expanded_ ||
            !service->candidate_request_pending_ || service->expanded_pages_.selected != 0) return 19;
        const auto expanded_request = service->active_candidate_request_id_;
        if (!press(VK_F8) || service->candidates_expanded_ || service->candidate_page_ != 0) return 20;
        auto* late_expanded = new TextService::CandidateResult();
        late_expanded->request_id = expanded_request;
        late_expanded->generation = service->context_generation_;
        late_expanded->expanded = true;
        service->handle_candidate_result(late_expanded);
        if (service->candidates_expanded_ || !service->candidate_request_pending_) return 21;
        respond(7);
        if (service->candidate_request_pending_ || service->candidates_.size() != 7 ||
            service->candidate_shortcut_index(1) != 1) return 22;
        service->candidate_page_ = 1;
        if (!press(VK_F8) || !service->candidates_expanded_ || !service->candidate_request_pending_ ||
            service->expanded_pages_.selected != 0) return 62;
        service->clear_composition();
        if (service->candidates_expanded_ || service->expanded_pages_.selected || service->expanded_pages_.first_visible) return 13;
        // Exercise actual D2D output off-screen, without activating TSF, reading
        // desktop pixels or contacting Core. The reserved canvas must stay clear
        // on both axes, including when stale expanded data exceeds one page.
        if (FAILED(service->initialize_windows())) return 24;
        service->candidate_theme_.candidate_badge = {0x123456, 0.21F};
        service->candidate_theme_.selected_page = {0x234567, 0.22F};
        service->candidate_theme_.hover_block = {0x345678, 0.23F};
        service->candidate_theme_.pressed_block = {0x456789, 0.24F};
        service->candidate_theme_.button_background = {0x56789a, 0.25F};
        service->candidate_theme_.button_border = {0x6789ab, 0.26F};
        service->candidate_theme_.button_text = {0x789abc, 0.27F};
        service->candidate_theme_.secondary_text = {0x89abcd, 0.28F};
        service->candidate_theme_.caret = {0x9abcde, 0.29F};
        service->input_buffer_ = L"nihao";
        service->candidate_present_geometry_valid_ = true;
        service->candidate_present_position_ = {-32000, -32000};
        service->candidate_page_size_ = 7;
        service->candidates_.assign(49, L"fixture");
        service->candidate_groups_.clear();
        for (std::size_t i = 0; i < 49; ++i) service->candidate_groups_.push_back(i % 3);
        for (const bool blur : {false, true}) for (int transition = 0; transition < 6; ++transition) {
            service->candidates_expanded_ = transition % 2 == 0;
            service->candidate_present_size_ = {1100, service->candidates_expanded_ ? 264L : 88L};
            // Intentionally constrained card: expose unbounded badges/rows.
            service->candidate_content_size_ = {160, 88};
            service->candidate_theme_.background_blur_enabled = blur;
            const auto count = static_cast<std::size_t>(service->candidate_present_size_.cx) *
                               service->candidate_present_size_.cy;
            if (blur) service->blurred_candidate_background_.assign(count, 0xff204060U);
            if (!service->render_candidate_window()) return 25;
            for (const auto& [brush, expected] : {
                std::pair{service->candidate_badge_brush_, *service->candidate_theme_.candidate_badge},
                std::pair{service->selected_page_brush_, *service->candidate_theme_.selected_page},
                std::pair{service->hover_brush_, *service->candidate_theme_.hover_block},
                std::pair{service->pressed_brush_, *service->candidate_theme_.pressed_block},
                std::pair{service->button_background_brush_, *service->candidate_theme_.button_background},
                std::pair{service->button_border_brush_, *service->candidate_theme_.button_border},
                std::pair{service->button_text_brush_, *service->candidate_theme_.button_text},
                std::pair{service->secondary_text_brush_, *service->candidate_theme_.secondary_text},
                std::pair{service->caret_brush_, *service->candidate_theme_.caret}}) {
                const auto color = brush->GetColor();
                if (std::abs(color.r * 255 - ((expected.rgb >> 16) & 255)) > 0.01F ||
                    std::abs(color.g * 255 - ((expected.rgb >> 8) & 255)) > 0.01F ||
                    std::abs(color.b * 255 - (expected.rgb & 255)) > 0.01F ||
                    std::abs(color.a - expected.opacity) > 0.001F) return 28;
            }
            const auto* pixels = static_cast<const std::uint32_t*>(service->layered_surface_pixels_);
            bool content = false;
            for (LONG y = 0; y < service->layered_surface_size_.cy; ++y)
                for (LONG x = 0; x < service->layered_surface_size_.cx; ++x) {
                    const auto pixel = pixels[static_cast<std::size_t>(y) * service->layered_surface_size_.cx + x];
                    if (x >= 160 || y >= 88) { if (pixel != 0) return 26; }
                    else content |= (pixel >> 24) != 0;
                }
            if (!content) return 27;
            service->hide_candidate_windows();
        }
        // Auxiliary first pages share the numbered-pill renderer and sizing,
        // but never add clickable/numeric targets belonging to the active row.
        service->candidates_expanded_ = false;
        service->candidates_.resize(7);
        service->candidate_groups_.resize(7);
        service->shortcut_config_.candidate_wrap_length = 4;
        const auto single = service->desired_candidate_window_size();
        service->following_previews_ = {
            {{L"世界", L"视界", L"时节", L"使节", L"是", L"时", L"事"}, {1, 1, 1, 1, 2, 2, 2}},
            {{L"后续长候选词换行仍保留数字序号与独立行高"}, {1}}};
        const auto multi = service->desired_candidate_window_size();
        if (multi.cy <= single.cy + 64 || multi.cx < single.cx) return 35;
        const auto scale = static_cast<float>(std::max(GetDpiForWindow(service->candidate_window_), 96U)) / 96.0F;
        const auto panel = service->layout_following_previews(static_cast<float>(multi.cx) / scale);
        if (!panel || panel->left != 8.0F || panel->top <= 80.0F ||
            panel->right >= static_cast<float>(multi.cx) / scale ||
            panel->bottom + 7.0F > static_cast<float>(multi.cy) / scale ||
            panel->row_heights.size() != 2 || panel->row_heights[1] <= panel->row_heights[0]) return 43;
        service->candidate_theme_.background_blur_enabled = false;
        service->candidate_present_size_ = multi;
        service->candidate_content_size_ = multi;
        if (!service->render_candidate_window()) return 36;
        std::size_t active_targets = 0;
        for (const auto& region : service->hit_regions_) {
            if (region.target.kind != TextService::HitKind::candidate) continue;
            ++active_targets;
            if (region.target.candidate_index >= 7 || region.bounds.top >= single.cy) return 37;
        }
        if (active_targets != 7 || service->candidate_shortcut_index(6) != 6) return 38;
        service->candidate_theme_.candidate_text.rgb = 0x12c47b;
        for (const bool light : {false, true}) {
            service->candidate_light_palette_ = light;
            service->discard_device_resources();
            if (!service->render_candidate_window()) return 40;
            const auto gray = service->preview_text_brush_->GetColor();
            const auto expected = static_cast<float>(light ? 0x70 : 0xa6) / 255.0F;
            if (std::abs(gray.r - expected) > 0.001F || std::abs(gray.g - expected) > 0.001F ||
                std::abs(gray.b - expected) > 0.001F ||
                std::abs(gray.a - service->candidate_theme_.candidate_text.opacity) > 0.001F) return 41;
            const auto main = service->text_brush_->GetColor();
            if (std::abs(main.g - 196.0F / 255.0F) > 0.001F) return 42;
        }
        // The dropdown adds its old-style outline, not another translucent
        // background layer. Compare its empty padding with the same canvas
        // without the dropdown so acrylic opacity cannot accidentally double.
        service->candidate_theme_.background = {0x202030, 0.35F};
        service->candidate_theme_.border = {0xd0d0d0, 1.0F};
        service->discard_device_resources();
        if (!service->render_candidate_window()) return 44;
        const auto pixel_at = [&](float x, float y) {
            const auto px = static_cast<std::size_t>(x * scale);
            const auto py = static_cast<std::size_t>(y * scale);
            const auto* pixels = static_cast<const std::uint32_t*>(service->layered_surface_pixels_);
            return pixels[py * service->layered_surface_size_.cx + px];
        };
        const auto inside = pixel_at(panel->left + 6, panel->top + 24);
        const auto outline = pixel_at(panel->left, panel->top + 24);
        auto saved_rows = std::move(service->following_previews_);
        service->following_previews_.clear();
        if (!service->render_candidate_window()) return 45;
        if (inside != pixel_at(panel->left + 6, panel->top + 24) ||
            outline == pixel_at(panel->left, panel->top + 24)) return 46;
        service->following_previews_ = std::move(saved_rows);
        service->candidates_expanded_ = true;
        if (service->layout_following_previews(static_cast<float>(multi.cx) / scale)) return 47;
        const auto expanded_with_aux = service->desired_candidate_window_size();
        service->following_previews_.clear();
        const auto expanded_without_aux = service->desired_candidate_window_size();
        if (expanded_with_aux.cy != expanded_without_aux.cy || expanded_with_aux.cx != expanded_without_aux.cx) return 39;
        service->hide_candidate_windows();

        // Hide only repeated strong delimiters in the presentation. The raw
        // candidates/consumption data remain suitable for committing any item.
        service->clear_composition();
        service->input_buffer_ = L"nihao，shijie；";
        service->input_cursor_ = service->input_buffer_.size();
        auto* symbols = new TextService::CandidateResult();
        symbols->request_id = service->active_candidate_request_id_;
        symbols->generation = service->context_generation_;
        symbols->page_size = 7;
        symbols->symbol_preview = true;
        symbols->segmented_input = L"ni'hao，shi'jie；";
        symbols->candidates = {L"你好，", L"第二候选字符内容，"};
        symbols->candidate_consumed = {6, 6};
        symbols->following_previews = {{{L"世界；", L"后续候选字符内容；"}, {1, 1}}};
        service->handle_candidate_result(symbols);
        for (const auto suffix : {L",", L"。", L"；", L"：", L"!", L"？", L"、", L"，；!?\n\r"}) {
            const auto text = std::wstring(L"候选") + suffix;
            if (service->candidate_display_text(text, 0) != text ||
                service->candidate_display_text(text, 1) != L"候选") return 52;
        }
        for (const auto text : {L"“你好”", L"你+好", L"你，好", L"你好🙂"})
            if (service->candidate_display_text(text, 1) != text) return 53;
        if (service->candidates_[1] != L"第二候选字符内容，" ||
            service->candidate_consumed_[1] != 6 ||
            service->candidate_display_text(service->following_previews_[0].candidates[1], 1) != L"后续候选字符内容") return 54;
        const auto symbol_size = service->desired_candidate_window_size();
        service->candidate_present_size_ = symbol_size;
        service->candidate_content_size_ = symbol_size;
        if (!service->render_candidate_window()) return 55;
        const auto* symbol_pixels = static_cast<const std::uint32_t*>(service->layered_surface_pixels_);
        const std::vector<std::uint32_t> visible_symbols(symbol_pixels, symbol_pixels +
            service->layered_surface_size_.cx * service->layered_surface_size_.cy);
        service->candidate_symbol_preview_ = false;
        service->candidates_[1].pop_back();
        service->following_previews_[0].candidates[1].pop_back();
        const auto plain_size = service->desired_candidate_window_size();
        if (symbol_size.cx != plain_size.cx || symbol_size.cy != plain_size.cy ||
            !service->render_candidate_window() ||
            !std::equal(visible_symbols.begin(), visible_symbols.end(),
                static_cast<const std::uint32_t*>(service->layered_surface_pixels_))) return 56;
        // Non-symbol responses and clear must not retain display suppression.
        if (service->candidate_display_text(L"候选，", 1) != L"候选，") return 57;
        service->candidate_symbol_preview_ = true;
        service->clear_composition();
        if (service->candidate_symbol_preview_) return 58;
        // Cold refinements must not clear an existing frame to a status-only
        // row on each keystroke. The stale snapshot remains non-committable;
        // the final response, including a genuine empty result, replaces it.
        service->input_buffer_ = L"nipingchang";
        service->input_cursor_ = service->input_buffer_.size();
        service->segmented_input_ = L"ni'ping'chang";
        respond(7);
        for (const wchar_t letter : std::wstring_view(L"yong")) {
            service->insert_composition_character(letter);
            ++service->context_generation_;
            ++service->active_candidate_request_id_;
            service->candidate_request_pending_ = true;
            const auto previous = service->candidates_;
            const auto consumed = service->candidate_consumed_;
            const auto groups = service->candidate_groups_;
            const auto preview = service->segmented_input_;
            const auto before = service->desired_candidate_window_size();
            auto* waiting = new TextService::CandidateResult();
            waiting->request_id = service->active_candidate_request_id_;
            waiting->generation = service->context_generation_;
            waiting->refinement_pending = true;
            service->handle_candidate_result(waiting);
            const auto after = service->desired_candidate_window_size();
            if (!service->candidate_request_pending_ || service->candidates_ != previous ||
                service->candidate_consumed_ != consumed || service->candidate_groups_ != groups ||
                service->segmented_input_ != preview || before.cx != after.cx || before.cy != after.cy ||
                service->commit_candidate(nullptr, 0) != E_PENDING) return 61;
            respond(7);
            if (service->candidate_request_pending_) return 62;
        }
        // Do not preserve a stale frame indefinitely on a completed no-match.
        respond(0);
        if (!service->candidates_.empty() || service->candidate_request_pending_) return 63;
        service->clear_composition();
        return 0;
    }
};
}
int main() {
    BYTE previous[256]{}, cleared[256]{};
    if (!GetKeyboardState(previous) || !SetKeyboardState(cleared)) return 99;
    const int result = owo::tsf::ExpandedCandidateTestAccess::run();
    SetKeyboardState(previous);
    if (!result) std::cout << "Real TSF dispatch, asynchronous expansion, numeric mapping, collapse snapshot and D2D pixel boundary checks passed.\n";
    return result;
}

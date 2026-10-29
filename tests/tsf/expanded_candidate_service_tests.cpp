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
        // Opening at page one must stay open, even while its response is pending.
        if (!press(VK_F8) || !service->candidates_expanded_ ||
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

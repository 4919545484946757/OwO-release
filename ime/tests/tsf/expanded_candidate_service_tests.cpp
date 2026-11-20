#include "text_service.h"
#include "owo/protocol/messages.h"
#include <iostream>
#include <limits>

namespace owo::tsf {
// 不激活 TSF、不创建工作线程/窗口，直接验证真实按键分派和异步响应接线。
struct ExpandedCandidateTestAccess {
    static int run() {
        auto* service = new TextService();
        struct Release { TextService* value; ~Release() { value->Release(); } } release{service};
        service->shortcut_config_initialized_ = true;
        service->next_shortcut_config_refresh_ = std::numeric_limits<ULONGLONG>::max();
        service->input_buffer_ = L"nihao";
        service->input_cursor_ = 5;
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
                result->candidate_consumed.push_back(5);
            }
            service->handle_candidate_result(result);
        };
        auto press = [&](WPARAM key) {
            BOOL eaten = FALSE;
            return SUCCEEDED(service->OnKeyDown(nullptr, key, 0, &eaten)) && eaten;
        };
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
        // A learned continuation is clickable but has no number or Space binding,
        // including when it occupies the final expanded page by itself.
        service->candidate_groups_.assign(15, 0);
        service->candidate_groups_.back() =
            static_cast<std::uint64_t>(protocol::CandidateGroup::continuation);
        if (service->candidate_shortcut_index(0) || service->should_eat_key(VK_SPACE)) return 14;
        service->candidate_groups_.clear();
        const auto selected = service->expanded_pages_.selected;
        auto* stale = new TextService::CandidateResult();
        stale->request_id = service->active_candidate_request_id_ + 1;
        stale->generation = service->context_generation_;
        stale->expanded = true;
        service->handle_candidate_result(stale);
        if (service->expanded_pages_.selected != selected || service->candidates_.size() != 15) return 12;
        service->clear_composition();
        if (service->candidates_expanded_ || service->expanded_pages_.selected || service->expanded_pages_.first_visible) return 13;
        return 0;
    }
};
}
int main() {
    BYTE previous[256]{}, cleared[256]{};
    if (!GetKeyboardState(previous) || !SetKeyboardState(cleared)) return 99;
    const int result = owo::tsf::ExpandedCandidateTestAccess::run();
    SetKeyboardState(previous);
    if (!result) std::cout << "Real TSF dispatch, asynchronous expansion, numeric mapping and collapse checks passed.\n";
    return result;
}

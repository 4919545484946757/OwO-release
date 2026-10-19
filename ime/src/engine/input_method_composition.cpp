#include "owo/engine/input_method_composition.h"
#include <algorithm>

namespace owo::engine {
namespace {
// Input has already passed descriptor/request UTF-8 validation.
std::size_t width(std::string_view input, std::size_t at) {
    const auto c = static_cast<unsigned char>(input[at]);
    return c < 0x80 ? 1 : c < 0xe0 ? 2 : c < 0xf0 ? 3 : 4;
}
std::size_t previous(std::string_view input, std::size_t at) {
    if (!at) return 0;
    --at;
    while (at && (static_cast<unsigned char>(input[at]) & 0xc0) == 0x80) --at;
    return at;
}
}
bool InputMethodComposition::begin(InputMethodDescriptor d, std::string session) {
    if (!validate_input_method_descriptor(d).ok) return false;
    InputMethodRequest probe;
    probe.request_id = 1; probe.session_id = session; probe.scheme_id = d.id; probe.page_size = 1;
    if (!validate_input_method_request(d, probe).ok) return false;
    close(); descriptor_ = std::move(d); session_ = std::move(session); return true;
}
void InputMethodComposition::close() {
    descriptor_.reset(); session_.clear(); input_.clear(); caret_ = 0;
    pending_.reset(); prepared_.reset(); display_.reset(); selectable_ = false; ++revision_;
    // Do not reset IDs: late responses must not match a reopened session.
}
void InputMethodComposition::edited() { ++revision_; pending_.reset(); selectable_ = false; }
bool InputMethodComposition::insert(std::string_view text) {
    if (!descriptor_ || prepared_ || text.empty() || text.size() > descriptor_->max_input_bytes - input_.size()) return false;
    InputMethodRequest probe;
    probe.request_id = 1; probe.session_id = session_; probe.scheme_id = descriptor_->id; probe.page_size = 1;
    probe.input = input_; probe.input.insert(caret_, text); probe.caret_byte = caret_ + text.size();
    if (!validate_input_method_request(*descriptor_, probe).ok) return false;
    input_ = std::move(probe.input); caret_ = probe.caret_byte; edited(); return true;
}
bool InputMethodComposition::backspace() {
    if (!descriptor_ || prepared_ || !caret_) return false;
    const auto begin = previous(input_, caret_);
    input_.erase(begin, caret_ - begin); caret_ = begin; edited(); return true;
}
bool InputMethodComposition::erase_forward() {
    if (!descriptor_ || prepared_ || caret_ == input_.size()) return false;
    input_.erase(caret_, width(input_, caret_)); edited(); return true;
}
bool InputMethodComposition::move_caret(int delta) {
    if (!descriptor_ || prepared_) return false;
    auto next = caret_;
    while (delta > 0 && next < input_.size()) { next += width(input_, next); --delta; }
    while (delta < 0 && next) { next = previous(input_, next); ++delta; }
    if (next == caret_) return false;
    caret_ = next; edited(); return true;
}
bool InputMethodComposition::set_caret_utf16(std::size_t target) {
    if (!descriptor_ || prepared_) return false;
    std::size_t at{}, units{};
    while (at < input_.size() && units < target) {
        const auto count = width(input_, at); units += count == 4 ? 2 : 1; at += count;
    }
    if (units != target) return false;
    if (at != caret_) { caret_ = at; edited(); }
    return true;
}
std::size_t InputMethodComposition::caret_utf16() const {
    std::size_t units{};
    for (std::size_t at = 0; at < caret_;) {
        const auto count = width(input_, at); units += count == 4 ? 2 : 1; at += count;
    }
    return units;
}
std::optional<InputMethodRequest> InputMethodComposition::compose_request(std::uint64_t id, std::size_t page, std::size_t size) {
    if (!descriptor_ || prepared_ || id <= last_id_) return std::nullopt;
    InputMethodRequest r;
    r.request_id = id; r.session_id = session_; r.scheme_id = descriptor_->id;
    r.input = input_; r.caret_byte = caret_; r.page = page; r.page_size = size;
    if (!validate_input_method_request(*descriptor_, r).ok) return std::nullopt;
    last_id_ = id; pending_ = Pending{r, revision_, {}}; selectable_ = false; return r;
}
std::optional<InputMethodRequest> InputMethodComposition::select_request(std::uint64_t id, std::string_view candidate_id) {
    if (!descriptor_ || prepared_ || !display_ || !selectable_ || id <= last_id_) return std::nullopt;
    const auto found = std::find_if(display_->candidates.begin(), display_->candidates.end(),
        [&](const auto& c) { return c.id == candidate_id; });
    if (found == display_->candidates.end()) return std::nullopt;
    InputMethodRequest r;
    r.request_id = id; r.session_id = session_; r.scheme_id = descriptor_->id;
    r.action = InputMethodAction::select; r.selected_candidate_id = found->id;
    r.input = input_; r.caret_byte = caret_; r.page = display_->page; r.page_size = displayed_page_size_;
    if (!validate_input_method_request(*descriptor_, r).ok) return std::nullopt;
    last_id_ = id; pending_ = Pending{r, revision_, *found}; selectable_ = false; return r;
}
InputMethodApplyResult InputMethodComposition::apply(const InputMethodResponse& response) {
    if (!descriptor_ || !pending_ || pending_->revision != revision_ ||
        !validate_input_method_response(*descriptor_, pending_->request, response).ok) return {};
    const auto pending = *pending_; pending_.reset();
    if (pending.selected && response.status == InputMethodStatus::success) {
        InputMethodCommit commit{pending.selected->text, {}};
        for (const auto& range : pending.selected->consumed_ranges) {
            commit.consumed_input.push_back(input_.substr(range.begin, range.end - range.begin));
        }
        prepared_ = pending.selected;
        return {true, std::move(commit)};
    }
    display_ = response; displayed_page_size_ = pending.request.page_size;
    selectable_ = !pending.selected && response.status == InputMethodStatus::success && !response.candidates.empty();
    return {true, {}};
}
bool InputMethodComposition::finish_commit(bool committed) {
    if (!prepared_) return false;
    if (committed) {
        std::size_t removed_before{};
        for (const auto& range : prepared_->consumed_ranges)
            if (caret_ > range.begin) removed_before += (std::min)(caret_, range.end) - range.begin;
        for (auto at = prepared_->consumed_ranges.rbegin(); at != prepared_->consumed_ranges.rend(); ++at)
            input_.erase(at->begin, at->end - at->begin);
        caret_ -= removed_before;
    }
    prepared_.reset(); edited(); display_.reset(); return true;
}
} // namespace owo::engine

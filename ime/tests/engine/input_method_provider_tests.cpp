#include "owo/engine/input_method_provider.h"

#include <functional>
#include <iostream>
#include <stdexcept>

using namespace owo::engine;
namespace {
int failures{};
void check(bool ok, const char* name) {
    if (!ok) { std::cerr << "FAIL: " << name << '\n'; ++failures; }
}
InputMethodDescriptor descriptor(std::string plugin, InputMethodKind kind = InputMethodKind::table) {
    InputMethodDescriptor d;
    d.plugin_id = std::move(plugin);
    d.id = d.plugin_id + "/default";
    d.name = "Test scheme";
    d.language_tag = "zh-Hans";
    d.kind = kind;
    d.alphabet = U"abcdefghijklmnopqrstuvwxyz'";
    return d;
}
InputMethodRequest request(const InputMethodDescriptor& d, std::string input = "ab") {
    InputMethodRequest r;
    r.request_id = 1;
    r.session_id = "session-1";
    r.scheme_id = d.id;
    r.input = std::move(input);
    r.caret_byte = r.input.size();
    return r;
}
InputMethodResponse response(const InputMethodRequest& r) {
    InputMethodResponse v;
    v.request_id = r.request_id;
    v.session_id = r.session_id;
    v.scheme_id = r.scheme_id;
    v.page = r.page;
    v.preview = r.input;
    if (r.action != InputMethodAction::cancel && !r.input.empty())
        v.candidates.push_back({"candidate-1", "字", "", {{0, r.input.size()}}});
    return v;
}
class MockProvider final : public InputMethodProvider {
public:
    std::function<InputMethodResponse(const InputMethodRequest&)> callback = response;
    InputMethodResponse handle(const InputMethodRequest& r, std::stop_token) override { return callback(r); }
};
}
int main() {
    InputMethodRegistry registry;
    const auto cangjie = descriptor("org.example.cangjie");
    const auto wubi = descriptor("org.example.wubi");
    auto foreign = descriptor("org.example.foreign", InputMethodKind::transliteration);
    foreign.language_tag = "fr";
    foreign.alphabet += U"é";
    const auto phonetic = descriptor("org.example.phonetic", InputMethodKind::phonetic);
    for (const auto& d : {cangjie, wubi, foreign, phonetic}) {
        check(registry.add(d, std::make_shared<MockProvider>()).ok, "register neutral scheme");
        check(registry.dispatch(request(d)).status == InputMethodStatus::success, "dispatch each kind");
    }
    check(registry.descriptors().size() == 4, "enumerate schemes");
    check(!registry.add(cangjie, std::make_shared<MockProvider>()).ok, "duplicate registration rejected");
    auto bad_descriptor = cangjie;
    bad_descriptor.id = wubi.id;
    check(!validate_input_method_descriptor(bad_descriptor).ok, "namespace ownership");
    bad_descriptor = cangjie;
    ++bad_descriptor.api_version;
    check(!validate_input_method_descriptor(bad_descriptor).ok, "API version");
    bad_descriptor = cangjie;
    bad_descriptor.alphabet += U"a";
    check(!validate_input_method_descriptor(bad_descriptor).ok, "duplicate alphabet");
    bad_descriptor.alphabet = {static_cast<char32_t>(0xd800)};
    check(!validate_input_method_descriptor(bad_descriptor).ok, "invalid Unicode scalar");
    bad_descriptor = cangjie;
    bad_descriptor.max_input_bytes = 16385;
    check(!validate_input_method_descriptor(bad_descriptor).ok, "input capacity limit");
    bad_descriptor = cangjie;
    bad_descriptor.max_candidates = 257;
    check(!validate_input_method_descriptor(bad_descriptor).ok, "candidate capacity limit");
    check(!registry.add(descriptor("org.example.null"), nullptr).ok, "missing provider");

    auto r = request(foreign, "éa");
    check(registry.dispatch(r).status == InputMethodStatus::success, "multibyte input");
    r.caret_byte = 1;
    check(!validate_input_method_request(foreign, r).ok, "caret splits UTF8");
    r = request(cangjie, "1");
    check(!validate_input_method_request(cangjie, r).ok, "encoding alphabet enforced");
    r.input = std::string("\xc0\xaf", 2);
    check(!validate_input_method_request(cangjie, r).ok, "overlong UTF8 rejected");
    r = request(cangjie);
    r.request_id = 0;
    check(!validate_input_method_request(cangjie, r).ok, "nonzero request ID required");
    r = request(cangjie);
    r.action = InputMethodAction::select;
    check(!validate_input_method_request(cangjie, r).ok, "selection needs ID");
    r.selected_candidate_id = "candidate-1";
    check(registry.dispatch(r).status == InputMethodStatus::success, "selection action");
    r.action = InputMethodAction::cancel;
    check(!validate_input_method_request(cangjie, r).ok, "cancel cannot select");
    r.selected_candidate_id.clear();
    check(registry.dispatch(r).candidates.empty(), "cancel returns no candidates");

    r = request(foreign, "éab");
    const auto good = response(r);
    auto v = good;
    ++v.request_id;
    check(!validate_input_method_response(foreign, r, v).ok, "request identity");
    v = good; v.session_id = "another";
    check(!validate_input_method_response(foreign, r, v).ok, "session identity");
    v = good; ++v.page;
    check(!validate_input_method_response(foreign, r, v).ok, "page identity");
    v = good; v.scheme_id = cangjie.id;
    check(!validate_input_method_response(foreign, r, v).ok, "scheme identity");
    v = good; v.candidates[0].consumed_ranges = {{0, 1}};
    check(!validate_input_method_response(foreign, r, v).ok, "range splits UTF8");
    v = good; v.candidates[0].consumed_ranges = {{0, 3}, {2, 4}};
    check(!validate_input_method_response(foreign, r, v).ok, "overlapping ranges");
    v = good; v.candidates[0].consumed_ranges = {{2, 3}, {0, 2}};
    check(!validate_input_method_response(foreign, r, v).ok, "unsorted ranges");
    v = good; v.candidates[0].consumed_ranges = {{0, 5}};
    check(!validate_input_method_response(foreign, r, v).ok, "out of bounds range");
    v = good; v.candidates[0].consumed_ranges.clear();
    check(!validate_input_method_response(foreign, r, v).ok, "missing consumed range");
    v = good; v.candidates.push_back(v.candidates.front());
    check(!validate_input_method_response(foreign, r, v).ok, "duplicate candidate IDs");
    v = good; v.candidates[0].text = std::string("\xed\xa0\x80", 3);
    check(!validate_input_method_response(foreign, r, v).ok, "invalid response UTF8");
    v = good; v.status = InputMethodStatus::error;
    check(!validate_input_method_response(foreign, r, v).ok, "no error candidates");
    r.page_size = 1;
    v = good; v.candidates.push_back({"second", "词", "", {{0, 4}}});
    check(!validate_input_method_response(foreign, r, v).ok, "page limit enforced");
    r.page_size = 32;
    v = good;
    v.candidates.clear();
    for (int i = 0; i < 32; ++i)
        v.candidates.push_back({std::to_string(i), std::string(4096, 'x'), "", {{0, 4}}});
    check(!validate_input_method_response(foreign, r, v).ok, "aggregate payload budget");

    auto mock = std::make_shared<MockProvider>();
    const auto custom = descriptor("org.example.custom");
    check(registry.add(custom, mock).ok, "custom registration");
    r = request(custom);
    mock->callback = [](const auto& req) { auto out = response(req); ++out.request_id; return out; };
    v = registry.dispatch(r);
    check(v.status == InputMethodStatus::error && v.candidates.empty(), "bad provider result isolated");
    mock->callback = [](const InputMethodRequest&) -> InputMethodResponse { throw std::runtime_error("private"); };
    check(registry.dispatch(r).diagnostic == "Input method provider failed", "exception isolated");
    std::stop_source stop;
    stop.request_stop();
    check(registry.dispatch(r, stop.get_token()).status == InputMethodStatus::cancelled, "cancel before work");
    std::stop_source during;
    mock->callback = [&](const auto& req) { during.request_stop(); return response(req); };
    check(registry.dispatch(r, during.get_token()).status == InputMethodStatus::cancelled, "cancel during work");
    mock->callback = [&](const auto& req) {
        registry.remove_plugin(custom.plugin_id);
        check(registry.add(custom, mock).ok, "re-register within callback without lock deadlock");
        return response(req);
    };
    check(registry.dispatch(r).status == InputMethodStatus::unavailable, "removed generation cannot publish");
    registry.remove_plugin(custom.plugin_id);
    check(registry.dispatch(r).status == InputMethodStatus::unavailable, "removed scheme unavailable");
    check(registry.descriptors().size() == 4, "unrelated providers preserved");
    if (!failures) std::cout << "Input method provider contract tests passed\n";
    return failures ? 1 : 0;
}

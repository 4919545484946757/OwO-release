#include "owo/core/input_method_runtime.h"
#include "owo/plugin/input_method_protocol.h"
#include "scheme.h"

#include <iostream>
#include <thread>

using namespace owo;
namespace {
int failures{};
void check(bool value, const char* name) {
    if (!value) { ++failures; std::cerr << "FAIL: " << name << '\n'; }
}
engine::InputMethodRequest request(const engine::InputMethodDescriptor& d) {
    engine::InputMethodRequest r;
    r.request_id = 1; r.session_id = "session"; r.scheme_id = d.id;
    r.input = "a"; r.caret_byte = 1; r.page_size = 2;
    return r;
}
class BlockingProvider final : public engine::InputMethodProvider {
public:
    std::promise<void> entered;
    std::promise<void> released;
    std::shared_future<void> release = released.get_future().share();
    engine::InputMethodResponse handle(const engine::InputMethodRequest& r, std::stop_token) override {
        if (r.request_id == 1) { entered.set_value(); release.wait(); }
        return example::table_response(r);
    }
};
}
int main() {
    const auto d = example::table_descriptor("owo.plugin.codec-test");
    const auto metadata = plugin::encode_input_method_descriptors(d.plugin_id, {d});
    std::vector<engine::InputMethodDescriptor> descriptors;
    check(!metadata.empty() && plugin::decode_input_method_descriptors(d.plugin_id, metadata, descriptors).ok &&
        descriptors.size() == 1 && descriptors.front().alphabet == d.alphabet, "scheme descriptor roundtrip");
    for (std::size_t n = 0; n < metadata.size(); ++n)
        check(!plugin::decode_input_method_descriptors(d.plugin_id, std::string_view(metadata).substr(0, n), descriptors).ok &&
            descriptors.empty(), "truncated metadata rejected atomically");
    check(!plugin::decode_input_method_descriptors("other.plugin", metadata, descriptors).ok, "descriptor namespace ownership");
    check(!plugin::decode_input_method_descriptors(d.plugin_id, metadata + "x", descriptors).ok, "metadata trailing bytes rejected");
    check(plugin::encode_input_method_descriptors(d.plugin_id, {d, d}).empty(), "duplicate scheme IDs rejected");
    std::vector<engine::InputMethodDescriptor> excessive(plugin::kMaximumPluginInputSchemes + 1, d);
    check(plugin::encode_input_method_descriptors(d.plugin_id, excessive).empty(), "scheme count bounded");
    auto foreign_descriptor = d;
    foreign_descriptor.id = d.plugin_id + "/foreign";
    foreign_descriptor.kind = engine::InputMethodKind::transliteration;
    foreign_descriptor.alphabet += U"é";
    check(plugin::decode_input_method_descriptors(d.plugin_id,
        plugin::encode_input_method_descriptors(d.plugin_id, {d, foreign_descriptor}), descriptors).ok &&
        descriptors.size() == 2 && descriptors.back().alphabet.back() == U'é' &&
        descriptors.back().kind == engine::InputMethodKind::transliteration, "Unicode multisource descriptions");
    auto r = request(d);
    const auto wire = plugin::encode_input_method_request(d, r);
    engine::InputMethodRequest decoded;
    check(!wire.empty() && plugin::decode_input_method_request(d, wire, decoded).ok &&
        decoded.input == "a" && decoded.page_size == 2, "request roundtrip");
    for (std::size_t n = 0; n < wire.size(); ++n)
        check(!plugin::decode_input_method_request(d, std::string_view(wire).substr(0, n), decoded).ok,
              "truncated request rejected");
    check(!plugin::decode_input_method_request(d, wire + "x", decoded).ok, "request trailing bytes");
    auto damaged = wire;
    damaged[4] = 2;
    check(!plugin::decode_input_method_request(d, damaged, decoded).ok, "unsupported payload version");
    const auto response = example::table_response(r);
    const auto out = plugin::encode_input_method_response(d, r, response);
    engine::InputMethodResponse result;
    check(plugin::decode_input_method_response(d, r, out, result).ok &&
        result.candidates.size() == 2 && result.has_more, "response roundtrip");
    for (std::size_t n = 0; n < out.size(); ++n)
        check(!plugin::decode_input_method_response(d, r, std::string_view(out).substr(0, n), result).ok,
              "truncated response rejected");
    check(!plugin::decode_input_method_response(d, r, out + "x", result).ok &&
        result.candidates.empty(), "no partial response on failure");
    auto wrong_request = r; ++wrong_request.request_id;
    check(!plugin::decode_input_method_response(d, wrong_request, out, result).ok, "response request mismatch");
    auto invalid = response;
    invalid.candidates.front().consumed_ranges = {{0, 2}};
    check(plugin::encode_input_method_response(d, r, invalid).empty(), "invalid ranges cannot encode");
    check(!plugin::decode_input_method_response(d, r,
        std::string(plugin::kMaximumInputMethodWireBytes + 1, 'x'), result).ok, "wire size limit");

    auto foreign = d; foreign.alphabet += U"é";
    auto unicode = r; unicode.input = "é"; unicode.caret_byte = 2;
    check(plugin::decode_input_method_request(foreign,
        plugin::encode_input_method_request(foreign, unicode), decoded).ok && decoded.caret_byte == 2,
        "Unicode caret survives wire");

    engine::InputMethodRegistry registry;
    auto provider = std::make_shared<BlockingProvider>();
    check(registry.add(d, provider).ok, "worker provider registered");
    {
        core::InputMethodWorker worker(registry);
        auto first = worker.submit(r);
        check(provider->entered.get_future().wait_for(std::chrono::seconds(2)) == std::future_status::ready,
              "worker started asynchronously");
        r.request_id = 2;
        auto pending = worker.submit(r);
        r.request_id = 3; r.input = "ab"; r.caret_byte = 2;
        auto latest = worker.submit(r);
        check(pending.wait_for(std::chrono::milliseconds(100)) == std::future_status::ready &&
              pending.get().status == engine::InputMethodStatus::cancelled, "pending request replaced");
        provider->released.set_value();
        check(first.get().status == engine::InputMethodStatus::cancelled, "stale active result discarded");
        const auto selected = latest.get();
        check(selected.status == engine::InputMethodStatus::success && selected.request_id == 3 &&
              selected.candidates.size() == 1 && selected.candidates.front().text == "明", "latest request wins");
    }
    if (!failures) std::cout << "Input scheme codec and worker tests passed\n";
    return failures ? 1 : 0;
}

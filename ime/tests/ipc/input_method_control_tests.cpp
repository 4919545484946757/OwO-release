#include "owo/ipc/input_method_control.h"

#include <iostream>
#include <vector>

using namespace owo::ipc;
namespace {
int failures{};
void check(bool ok, const char* message) {
    if (!ok) { ++failures; std::cerr << "FAIL: " << message << '\n'; }
}
}
int main() {
    std::vector<InputMethodControlRequest> requests;
    InputMethodControlRequest r;
    r.request_id = 1; r.plugin_id = "example.table";
    requests.push_back(r);
    r.action = InputMethodControlAction::open;
    r.scheme_id = "example.table/table"; r.session_id = "session-1";
    requests.push_back(r);
    r = {}; r.request_id = 2; r.action = InputMethodControlAction::submit;
    r.payload = std::string("OWIQ\0abc", 8); r.sensitive_input = true;
    requests.push_back(r);
    r = {}; r.request_id = 3; r.action = InputMethodControlAction::poll; r.ticket = 2;
    requests.push_back(r);
    r = {}; r.request_id = 4; r.action = InputMethodControlAction::close; r.session_id = "session-1";
    requests.push_back(r);
    for (const auto& item : requests) {
        const auto wire = encode_input_method_control_request(item);
        InputMethodControlRequest decoded;
        check(!wire.empty() && decode_input_method_control_request(wire, decoded) &&
            decoded.request_id == item.request_id && decoded.action == item.action &&
            decoded.ticket == item.ticket && decoded.plugin_id == item.plugin_id &&
            decoded.scheme_id == item.scheme_id && decoded.session_id == item.session_id &&
            decoded.payload == item.payload && decoded.sensitive_input == item.sensitive_input, "request roundtrip");
        for (std::size_t n = 0; n < wire.size(); ++n) {
            decoded = item;
            check(!decode_input_method_control_request(std::string_view(wire).substr(0, n), decoded) &&
                !decoded.request_id && decoded.payload.empty(), "truncated request resets output");
        }
        check(!decode_input_method_control_request(wire + "x", decoded), "request trailing bytes");
        auto damaged = wire; damaged[4] = 2;
        check(!decode_input_method_control_request(damaged, decoded), "request version");
        damaged = wire; damaged[16] = 99;
        check(!decode_input_method_control_request(damaged, decoded), "request unknown action");
        damaged = wire; damaged.back() = 2;
        check(!decode_input_method_control_request(damaged, decoded), "strict boolean");
        auto invalid = item; invalid.request_id = 0;
        check(encode_input_method_control_request(invalid).empty(), "zero request ID");
    }
    r = requests[1]; r.scheme_id = "foreign.plugin/table";
    check(encode_input_method_control_request(r).empty(), "scheme ownership");
    r = requests[0]; r.payload = "unexpected";
    check(encode_input_method_control_request(r).empty(), "discovery cannot carry input");
    r = requests[2]; r.plugin_id = "example.table";
    check(encode_input_method_control_request(r).empty(), "submit cannot rebind identity");
    r = requests[3]; r.ticket = 0;
    check(encode_input_method_control_request(r).empty(), "poll requires ticket");
    r = requests[2]; r.payload.assign(kMaximumInputMethodControlPayload, 'x');
    InputMethodControlRequest decoded;
    check(static_cast<bool>(decode_input_method_control_request(encode_input_method_control_request(r), decoded)),
        "maximum payload accepted");
    r.payload += 'x';
    check(encode_input_method_control_request(r).empty(), "oversized payload rejected");
    auto damaged = encode_input_method_control_request(requests[0]);
    for (std::size_t n = 25; n < 29; ++n) damaged[n] = static_cast<char>(255);
    check(!decode_input_method_control_request(damaged, decoded), "hostile string length");
    for (const auto status : {InputMethodControlStatus::pending, InputMethodControlStatus::ready,
                             InputMethodControlStatus::rejected, InputMethodControlStatus::error}) {
        InputMethodControlResponse result;
        result.request_id = 5; result.ticket = 2; result.status = status;
        if (status == InputMethodControlStatus::ready) result.payload = std::string("OWIR\0", 5);
        if (status == InputMethodControlStatus::rejected || status == InputMethodControlStatus::error)
            result.diagnostic = "Request rejected";
        auto wire = encode_input_method_control_response(result);
        InputMethodControlResponse out;
        check(!wire.empty() && decode_input_method_control_response(wire, out) &&
            out.request_id == result.request_id && out.ticket == result.ticket &&
            out.status == status && out.payload == result.payload && out.diagnostic == result.diagnostic,
            "response roundtrip");
        for (std::size_t n = 0; n < wire.size(); ++n) {
            out = result;
            check(!decode_input_method_control_response(std::string_view(wire).substr(0, n), out) &&
                out.payload.empty() && !out.request_id, "truncated response resets output");
        }
        check(!decode_input_method_control_response(wire + "x", out), "response trailing bytes");
        wire[24] = 99;
        check(!decode_input_method_control_response(wire, out), "unknown response status");
        result.diagnostic = "private\ntext";
        check(encode_input_method_control_response(result).empty(), "invalid diagnostic");
    }
    return failures ? 1 : 0;
}

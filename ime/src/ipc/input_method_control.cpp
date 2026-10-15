#include "owo/ipc/input_method_control.h"

#include <algorithm>

namespace owo::ipc {
namespace {
void number(std::string& out, std::uint64_t value, unsigned width) {
    for (unsigned n = 0; n < width; ++n) out.push_back(static_cast<char>(value >> (n * 8)));
}
void text(std::string& out, std::string_view value) { number(out, value.size(), 4); out.append(value); }
struct Reader {
    std::string_view input;
    std::size_t position{};
    bool number(std::uint64_t& value, unsigned width) {
        if (width > input.size() - position) return false;
        value = 0;
        for (unsigned n = 0; n < width; ++n)
            value |= static_cast<std::uint64_t>(static_cast<unsigned char>(input[position++])) << (n * 8);
        return true;
    }
    bool text(std::string& value, std::size_t maximum) {
        std::uint64_t size{};
        if (!number(size, 4) || size > maximum || size > input.size() - position) return false;
        value.assign(input.substr(position, static_cast<std::size_t>(size)));
        position += static_cast<std::size_t>(size);
        return true;
    }
    bool header(std::string_view magic) {
        if (input.size() > kMaximumInputMethodControlWire || !input.starts_with(magic)) return false;
        position = magic.size();
        std::uint64_t version{};
        return number(version, 4) && version == 1;
    }
};
bool token(std::string_view value, std::size_t maximum) {
    return !value.empty() && value.size() <= maximum && std::all_of(value.begin(), value.end(), [](unsigned char c) {
        return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') ||
            c == '-' || c == '_' || c == '.';
    });
}
bool plugin_id(std::string_view value) {
    return token(value, 128) && value.front() != '.' && value.back() != '.' &&
        value.find('.') != value.npos && value.find("..") == value.npos &&
        std::all_of(value.begin(), value.end(), [](unsigned char c) {
            return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '-';
        });
}
bool valid(const InputMethodControlRequest& r) {
    if (!r.request_id || r.payload.size() > kMaximumInputMethodControlPayload) return false;
    const bool no_names = r.plugin_id.empty() && r.scheme_id.empty() && r.session_id.empty();
    switch (r.action) {
    case InputMethodControlAction::discover:
        return plugin_id(r.plugin_id) && r.scheme_id.empty() && r.session_id.empty() &&
            !r.ticket && r.payload.empty() && !r.sensitive_input;
    case InputMethodControlAction::open:
        return plugin_id(r.plugin_id) && r.scheme_id.starts_with(r.plugin_id + "/") &&
            token(std::string_view(r.scheme_id).substr(r.plugin_id.size() + 1), 128) &&
            token(r.session_id, 128) && !r.ticket && r.payload.empty();
    case InputMethodControlAction::submit:
        return no_names && !r.ticket && !r.payload.empty();
    case InputMethodControlAction::poll:
        return no_names && r.ticket && r.payload.empty();
    case InputMethodControlAction::close:
        return r.plugin_id.empty() && r.scheme_id.empty() && token(r.session_id, 128) &&
            !r.ticket && r.payload.empty() && !r.sensitive_input;
    }
    return false;
}
bool valid(const InputMethodControlResponse& r) {
    if (!r.request_id || r.payload.size() > kMaximumInputMethodControlPayload || r.diagnostic.size() > 512 ||
        !std::all_of(r.diagnostic.begin(), r.diagnostic.end(), [](unsigned char c) { return c >= 32 && c <= 126; })) return false;
    switch (r.status) {
    case InputMethodControlStatus::pending: return r.ticket && r.payload.empty() && r.diagnostic.empty();
    case InputMethodControlStatus::ready: return r.diagnostic.empty();
    case InputMethodControlStatus::rejected:
    case InputMethodControlStatus::error: return r.payload.empty() && !r.diagnostic.empty();
    }
    return false;
}
protocol::ValidationResult invalid() { return {protocol::ErrorCode::invalid_payload, "Invalid input scheme control payload"}; }
}
std::string encode_input_method_control_request(const InputMethodControlRequest& r) {
    if (!valid(r)) return {};
    std::string out("OWCQ"); number(out, 1, 4); number(out, r.request_id, 8);
    number(out, static_cast<unsigned>(r.action), 1); number(out, r.ticket, 8);
    text(out, r.plugin_id); text(out, r.scheme_id); text(out, r.session_id); text(out, r.payload);
    number(out, r.sensitive_input ? 1 : 0, 1);
    return out;
}
protocol::ValidationResult decode_input_method_control_request(std::string_view bytes, InputMethodControlRequest& out) {
    out = {};
    Reader reader{bytes}; InputMethodControlRequest value; std::uint64_t action{}, sensitive{};
    if (!reader.header("OWCQ") || !reader.number(value.request_id, 8) || !reader.number(action, 1) ||
        !reader.number(value.ticket, 8) || !reader.text(value.plugin_id, 128) || !reader.text(value.scheme_id, 257) ||
        !reader.text(value.session_id, 128) || !reader.text(value.payload, kMaximumInputMethodControlPayload) ||
        !reader.number(sensitive, 1) || sensitive > 1 || reader.position != bytes.size()) return invalid();
    value.action = static_cast<InputMethodControlAction>(action); value.sensitive_input = sensitive != 0;
    if (!valid(value)) return invalid();
    out = std::move(value); return {};
}
std::string encode_input_method_control_response(const InputMethodControlResponse& r) {
    if (!valid(r)) return {};
    std::string out("OWCR"); number(out, 1, 4); number(out, r.request_id, 8); number(out, r.ticket, 8);
    number(out, static_cast<unsigned>(r.status), 1); text(out, r.payload); text(out, r.diagnostic);
    return out;
}
protocol::ValidationResult decode_input_method_control_response(std::string_view bytes, InputMethodControlResponse& out) {
    out = {};
    Reader reader{bytes}; InputMethodControlResponse value; std::uint64_t status{};
    if (!reader.header("OWCR") || !reader.number(value.request_id, 8) || !reader.number(value.ticket, 8) ||
        !reader.number(status, 1) || !reader.text(value.payload, kMaximumInputMethodControlPayload) ||
        !reader.text(value.diagnostic, 512) || reader.position != bytes.size()) return invalid();
    value.status = static_cast<InputMethodControlStatus>(status);
    if (!valid(value)) return invalid();
    out = std::move(value); return {};
}
} // namespace owo::ipc

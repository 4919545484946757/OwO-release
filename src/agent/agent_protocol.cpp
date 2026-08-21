#include "owo/agent/agent_protocol.h"

#include <algorithm>
#include <array>
#include <charconv>
#include <cctype>
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <variant>

namespace owo::agent {
namespace {

struct JsonValue;
using Object = std::unordered_map<std::string, JsonValue>;
using Array = std::vector<JsonValue>;
struct JsonValue : std::variant<std::string, std::uint64_t, bool, Object, Array> {
    using variant::variant;
};

class Parser {
public:
    explicit Parser(const std::string_view input) : input_(input) {}
    bool parse(JsonValue& output) {
        space();
        if (!value(output, 0)) return false;
        space();
        return offset_ == input_.size() || fail("trailing JSON data");
    }
    const std::string& diagnostic() const { return diagnostic_; }

private:
    bool value(JsonValue& output, const unsigned depth) {
        if (depth > 4 || offset_ >= input_.size()) return fail("invalid JSON value");
        if (input_[offset_] == '"') {
            std::string text;
            if (!string(text)) return false;
            output = std::move(text);
            return true;
        }
        if (input_[offset_] == '{') {
            Object object;
            if (!object_value(object, depth + 1)) return false;
            output = std::move(object);
            return true;
        }
        if (input_[offset_] == '[') {
            Array array;
            if (!array_value(array, depth + 1)) return false;
            output = std::move(array);
            return true;
        }
        if (take("true")) { output = true; return true; }
        if (take("false")) { output = false; return true; }
        std::uint64_t number{};
        const auto begin = input_.data() + offset_;
        const auto parsed = std::from_chars(begin, input_.data() + input_.size(), number);
        if (parsed.ec != std::errc{} || parsed.ptr == begin) return fail("expected JSON value");
        offset_ = static_cast<std::size_t>(parsed.ptr - input_.data());
        output = number;
        return true;
    }
    bool object_value(Object& output, const unsigned depth) {
        if (!character('{')) return false;
        space();
        if (character('}')) return true;
        while (true) {
            std::string key;
            if (!string(key)) return false;
            space();
            if (!character(':')) return fail("expected ':'");
            space();
            JsonValue item;
            if (!value(item, depth)) return false;
            if (!output.emplace(std::move(key), std::move(item)).second)
                return fail("duplicate JSON field");
            space();
            if (character('}')) return true;
            if (!character(',')) return fail("expected ',' or '}'");
            space();
        }
    }
    bool array_value(Array& output, const unsigned depth) {
        if (!character('[')) return false;
        space();
        if (character(']')) return true;
        while (true) {
            if (output.size() >= kMaximumAgentCommands) return fail("array exceeds limit");
            JsonValue item;
            if (!value(item, depth)) return false;
            output.push_back(std::move(item));
            space();
            if (character(']')) return true;
            if (!character(',')) return fail("expected ',' or ']'");
            space();
        }
    }
    bool string(std::string& output) {
        if (!character('"')) return fail("expected JSON string");
        while (offset_ < input_.size()) {
            const unsigned char byte = static_cast<unsigned char>(input_[offset_++]);
            if (byte == '"') return true;
            if (byte < 0x20) return fail("control character in JSON string");
            if (byte != '\\') { output.push_back(static_cast<char>(byte)); continue; }
            if (offset_ >= input_.size()) return fail("truncated JSON escape");
            switch (input_[offset_++]) {
            case '"': output.push_back('"'); break;
            case '\\': output.push_back('\\'); break;
            case '/': output.push_back('/'); break;
            case 'b': output.push_back('\b'); break;
            case 'f': output.push_back('\f'); break;
            case 'n': output.push_back('\n'); break;
            case 'r': output.push_back('\r'); break;
            case 't': output.push_back('\t'); break;
            case 'u': {
                std::uint32_t scalar{};
                if (!hex_quad(scalar)) return false;
                if (scalar >= 0xd800 && scalar <= 0xdbff) {
                    if (offset_ + 2 > input_.size() || input_[offset_] != '\\' ||
                        input_[offset_ + 1] != 'u') return fail("missing low surrogate");
                    offset_ += 2;
                    std::uint32_t low{};
                    if (!hex_quad(low) || low < 0xdc00 || low > 0xdfff)
                        return fail("invalid low surrogate");
                    scalar = 0x10000 + ((scalar - 0xd800) << 10) + (low - 0xdc00);
                } else if (scalar >= 0xdc00 && scalar <= 0xdfff) {
                    return fail("unexpected low surrogate");
                }
                append_utf8(output, scalar);
                break;
            }
            default: return fail("unsupported JSON escape");
            }
        }
        return fail("unterminated JSON string");
    }
    bool hex_quad(std::uint32_t& output) {
        if (offset_ + 4 > input_.size()) return fail("truncated Unicode escape");
        output = 0;
        for (unsigned index = 0; index < 4; ++index) {
            const unsigned char byte = static_cast<unsigned char>(input_[offset_++]);
            output <<= 4;
            if (byte >= '0' && byte <= '9') output |= byte - '0';
            else if (byte >= 'a' && byte <= 'f') output |= byte - 'a' + 10;
            else if (byte >= 'A' && byte <= 'F') output |= byte - 'A' + 10;
            else return fail("invalid Unicode escape");
        }
        return true;
    }
    static void append_utf8(std::string& output, const std::uint32_t scalar) {
        if (scalar <= 0x7f) output.push_back(static_cast<char>(scalar));
        else if (scalar <= 0x7ff) {
            output.push_back(static_cast<char>(0xc0 | (scalar >> 6)));
            output.push_back(static_cast<char>(0x80 | (scalar & 0x3f)));
        } else if (scalar <= 0xffff) {
            output.push_back(static_cast<char>(0xe0 | (scalar >> 12)));
            output.push_back(static_cast<char>(0x80 | ((scalar >> 6) & 0x3f)));
            output.push_back(static_cast<char>(0x80 | (scalar & 0x3f)));
        } else {
            output.push_back(static_cast<char>(0xf0 | (scalar >> 18)));
            output.push_back(static_cast<char>(0x80 | ((scalar >> 12) & 0x3f)));
            output.push_back(static_cast<char>(0x80 | ((scalar >> 6) & 0x3f)));
            output.push_back(static_cast<char>(0x80 | (scalar & 0x3f)));
        }
    }
    bool character(const char expected) {
        if (offset_ >= input_.size() || input_[offset_] != expected) return false;
        ++offset_;
        return true;
    }
    bool take(const std::string_view token) {
        if (input_.substr(offset_, token.size()) != token) return false;
        offset_ += token.size();
        return true;
    }
    void space() {
        while (offset_ < input_.size() &&
               (input_[offset_] == ' ' || input_[offset_] == '\t' ||
                input_[offset_] == '\r' || input_[offset_] == '\n')) ++offset_;
    }
    bool fail(std::string value) {
        if (diagnostic_.empty()) diagnostic_ = std::move(value);
        return false;
    }
    std::string_view input_;
    std::size_t offset_{};
    std::string diagnostic_;
};

bool valid_utf8(const std::string_view text) {
    std::size_t offset = 0;
    while (offset < text.size()) {
        const auto first = static_cast<unsigned char>(text[offset]);
        std::size_t count = 0;
        std::uint32_t scalar = 0;
        if (first <= 0x7f) { count = 1; scalar = first; }
        else if (first >= 0xc2 && first <= 0xdf) { count = 2; scalar = first & 0x1f; }
        else if (first >= 0xe0 && first <= 0xef) { count = 3; scalar = first & 0x0f; }
        else if (first >= 0xf0 && first <= 0xf4) { count = 4; scalar = first & 7; }
        else return false;
        if (offset + count > text.size()) return false;
        for (std::size_t index = 1; index < count; ++index) {
            const auto next = static_cast<unsigned char>(text[offset + index]);
            if ((next & 0xc0) != 0x80) return false;
            scalar = (scalar << 6) | (next & 0x3f);
        }
        if ((count == 2 && scalar < 0x80) || (count == 3 && scalar < 0x800) ||
            (count == 4 && scalar < 0x10000) ||
            (scalar >= 0xd800 && scalar <= 0xdfff) || scalar > 0x10ffff) return false;
        offset += count;
    }
    return true;
}

bool token(const std::string_view value, const std::size_t maximum = 128) {
    return !value.empty() && value.size() <= maximum &&
        std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
            return std::isalnum(byte) != 0 || byte == '-' || byte == '_' || byte == '.';
        });
}

bool session_id(const std::string_view value) {
    return value.size() >= 16 && value.size() <= 64 &&
        std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
            return std::isxdigit(byte) != 0 || byte == '-';
        });
}

std::string escape(const std::string_view value) {
    std::string output;
    output.reserve(value.size() + 8);
    for (const unsigned char byte : value) {
        switch (byte) {
        case '"': output += "\\\""; break;
        case '\\': output += "\\\\"; break;
        case '\b': output += "\\b"; break;
        case '\f': output += "\\f"; break;
        case '\n': output += "\\n"; break;
        case '\r': output += "\\r"; break;
        case '\t': output += "\\t"; break;
        default: if (byte < 0x20) return {}; output.push_back(static_cast<char>(byte));
        }
    }
    return output;
}

template <typename T>
const T* field(const Object& object, const char* name) {
    const auto found = object.find(name);
    return found == object.end() ? nullptr : std::get_if<T>(&found->second);
}

std::optional<Action> parse_action(const std::string_view value) {
    for (const auto action : {Action::submit, Action::select, Action::page,
                              Action::cancel, Action::poll})
        if (value == action_name(action)) return action;
    return std::nullopt;
}

std::optional<Status> parse_status(const std::string_view value) {
    for (const auto status : {Status::agent_mode, Status::thinking, Status::executing,
                              Status::completed, Status::disconnected, Status::error})
        if (value == status_name(status)) return status;
    return std::nullopt;
}

bool luhn_number(const std::string_view digits) {
    if (digits.size() < 13 || digits.size() > 19) return false;
    unsigned sum = 0;
    bool double_digit = false;
    for (auto index = digits.rbegin(); index != digits.rend(); ++index) {
        if (*index < '0' || *index > '9') return false;
        unsigned value = static_cast<unsigned>(*index - '0');
        if (double_digit && (value *= 2) > 9) value -= 9;
        sum += value;
        double_digit = !double_digit;
    }
    return sum % 10 == 0;
}

}  // namespace

const char* action_name(const Action action) noexcept {
    switch (action) {
    case Action::submit: return "submit";
    case Action::select: return "select";
    case Action::page: return "page";
    case Action::cancel: return "cancel";
    case Action::poll: return "poll";
    }
    return "submit";
}

const char* status_name(const Status status) noexcept {
    switch (status) {
    case Status::agent_mode: return "agent_mode";
    case Status::thinking: return "thinking";
    case Status::executing: return "executing";
    case Status::completed: return "completed";
    case Status::disconnected: return "disconnected";
    case Status::error: return "error";
    }
    return "error";
}

std::string encode_request(const Request& request) {
    if (!session_id(request.session_id) || request.user_input.size() > 4096 ||
        request.session_context.size() > 4096 || request.application.executable.size() > 260 ||
        request.application.window_class.size() > 128 || request.command_id.size() > 128 ||
        !valid_utf8(request.user_input) || !valid_utf8(request.session_context) ||
        !valid_utf8(request.application.executable) ||
        !valid_utf8(request.application.window_class) ||
        (!request.command_id.empty() && !token(request.command_id))) return {};
    if (request.action == Action::submit && request.user_input.empty()) return {};
    if (request.action == Action::select && request.command_id.empty()) return {};
    const auto input = escape(request.user_input);
    const auto executable = escape(request.application.executable);
    const auto window_class = escape(request.application.window_class);
    const auto context = escape(request.session_context);
    const auto command = escape(request.command_id);
    auto output = std::string("{\"schema_version\":1,\"action\":\"") +
        action_name(request.action) + "\",\"session_id\":\"" + request.session_id +
        "\",\"user_input\":\"" + input + "\",\"application\":{\"process_id\":" +
        std::to_string(request.application.process_id) + ",\"executable\":\"" + executable +
        "\",\"window_class\":\"" + window_class + "\",\"sensitive_input\":" +
        (request.application.sensitive_input ? "true" : "false") +
        "},\"session_context\":\"" + context + "\",\"command_id\":\"" + command +
        "\",\"page\":" + std::to_string(request.page) + "}";
    return output.size() <= kMaximumAgentPayloadBytes ? output : std::string{};
}

RequestDecodeResult decode_request(const std::string_view json) {
    RequestDecodeResult result;
    if (json.empty() || json.size() > kMaximumAgentPayloadBytes)
        return {false, {}, "agent request size is invalid"};
    JsonValue root;
    Parser parser(json);
    if (!parser.parse(root)) return {false, {}, parser.diagnostic()};
    const auto* object = std::get_if<Object>(&root);
    if (object == nullptr || object->size() != 8) return {false, {}, "agent request fields mismatch"};
    const auto* version = field<std::uint64_t>(*object, "schema_version");
    const auto* action = field<std::string>(*object, "action");
    const auto* id = field<std::string>(*object, "session_id");
    const auto* input = field<std::string>(*object, "user_input");
    const auto* application = field<Object>(*object, "application");
    const auto* context = field<std::string>(*object, "session_context");
    const auto* command = field<std::string>(*object, "command_id");
    const auto* page = field<std::uint64_t>(*object, "page");
    if (version == nullptr || *version != 1 || action == nullptr || id == nullptr ||
        input == nullptr || application == nullptr || context == nullptr || command == nullptr ||
        page == nullptr || *page > 1000 || application->size() != 4)
        return {false, {}, "agent request field type is invalid"};
    const auto parsed_action = parse_action(*action);
    const auto* pid = field<std::uint64_t>(*application, "process_id");
    const auto* executable = field<std::string>(*application, "executable");
    const auto* window_class = field<std::string>(*application, "window_class");
    const auto* sensitive = field<bool>(*application, "sensitive_input");
    if (!parsed_action || !session_id(*id) || pid == nullptr || *pid > 0xffffffffULL ||
        executable == nullptr || window_class == nullptr || sensitive == nullptr)
        return {false, {}, "agent request identity is invalid"};
    result.value = {*parsed_action, *id, *input,
                    {static_cast<std::uint32_t>(*pid), *executable, *window_class, *sensitive},
                    *context, *command, static_cast<std::uint32_t>(*page)};
    if (encode_request(result.value).empty()) return {false, {}, "agent request bounds are invalid"};
    result.ok = true;
    return result;
}

std::string encode_response(const Response& response) {
    if (!session_id(response.session_id) || response.message.size() > 16384 ||
        response.executing_command.size() > 4096 || response.error_code.size() > 64 ||
        response.commands.size() > kMaximumAgentCommands || !valid_utf8(response.message) ||
        !valid_utf8(response.executing_command) ||
        (!response.error_code.empty() && !token(response.error_code, 64))) return {};
    std::string commands = "[";
    for (std::size_t index = 0; index < response.commands.size(); ++index) {
        const auto& command = response.commands[index];
        if (!token(command.id) || command.label.empty() || command.label.size() > 512 ||
            !valid_utf8(command.label)) return {};
        if (index != 0) commands += ',';
        commands += "{\"id\":\"" + escape(command.id) + "\",\"label\":\"" +
            escape(command.label) + "\",\"high_risk\":" +
            (command.high_risk ? "true" : "false") + "}";
    }
    commands += ']';
    auto output = std::string("{\"schema_version\":1,\"session_id\":\"") +
        response.session_id + "\",\"message\":\"" + escape(response.message) +
        "\",\"commands\":" + commands + ",\"executing_command\":\"" +
        escape(response.executing_command) + "\",\"status\":\"" +
        status_name(response.status) + "\",\"page\":" + std::to_string(response.page) +
        ",\"has_more\":" + (response.has_more ? "true" : "false") +
        ",\"error_code\":\"" + escape(response.error_code) + "\"}";
    return output.size() <= kMaximumAgentPayloadBytes ? output : std::string{};
}

ResponseDecodeResult decode_response(const std::string_view json) {
    ResponseDecodeResult result;
    if (json.empty() || json.size() > kMaximumAgentPayloadBytes)
        return {false, {}, "agent response size is invalid"};
    JsonValue root;
    Parser parser(json);
    if (!parser.parse(root)) return {false, {}, parser.diagnostic()};
    const auto* object = std::get_if<Object>(&root);
    if (object == nullptr || object->size() != 9) return {false, {}, "agent response fields mismatch"};
    const auto* version = field<std::uint64_t>(*object, "schema_version");
    const auto* id = field<std::string>(*object, "session_id");
    const auto* message = field<std::string>(*object, "message");
    const auto* commands = field<Array>(*object, "commands");
    const auto* executing = field<std::string>(*object, "executing_command");
    const auto* status = field<std::string>(*object, "status");
    const auto* page = field<std::uint64_t>(*object, "page");
    const auto* more = field<bool>(*object, "has_more");
    const auto* error = field<std::string>(*object, "error_code");
    if (version == nullptr || *version != 1 || id == nullptr || message == nullptr ||
        commands == nullptr || executing == nullptr || status == nullptr || page == nullptr ||
        *page > 1000 || more == nullptr || error == nullptr)
        return {false, {}, "agent response field type is invalid"};
    const auto parsed_status = parse_status(*status);
    if (!parsed_status || !session_id(*id)) return {false, {}, "agent response identity is invalid"};
    result.value.session_id = *id;
    result.value.message = *message;
    result.value.executing_command = *executing;
    result.value.status = *parsed_status;
    result.value.page = static_cast<std::uint32_t>(*page);
    result.value.has_more = *more;
    result.value.error_code = *error;
    std::unordered_set<std::string> command_ids;
    for (const auto& value : *commands) {
        const auto* item = std::get_if<Object>(&value);
        if (item == nullptr || item->size() != 3) return {false, {}, "invalid agent command"};
        const auto* command_id = field<std::string>(*item, "id");
        const auto* label = field<std::string>(*item, "label");
        const auto* high_risk = field<bool>(*item, "high_risk");
        if (command_id == nullptr || label == nullptr || high_risk == nullptr)
            return {false, {}, "invalid agent command fields"};
        if (!command_ids.insert(*command_id).second)
            return {false, {}, "duplicate agent command id"};
        result.value.commands.push_back({*command_id, *label, *high_risk});
    }
    if (encode_response(result.value).empty()) return {false, {}, "agent response bounds are invalid"};
    result.ok = true;
    return result;
}

FilterResult filter_sensitive_text(const std::string_view utf8,
                                   const std::size_t maximum_bytes) {
    if (!valid_utf8(utf8)) return {false, {}, false, "sensitive filter received invalid UTF-8"};
    std::string output(utf8.substr(0, std::min(utf8.size(), maximum_bytes)));
    bool redacted = output.size() != utf8.size();
    std::string lower = output;
    std::transform(lower.begin(), lower.end(), lower.begin(), [](const unsigned char byte) {
        return static_cast<char>(std::tolower(byte));
    });
    constexpr std::array<std::string_view, 10> keys{
        "password", "passwd", "pwd", "token", "secret", "api_key", "apikey",
        "authorization", "bearer", "credential"};
    for (const auto key : keys) {
        std::size_t found = 0;
        while ((found = lower.find(key, found)) != std::string::npos) {
            auto begin = lower.find_first_of(":=", found + key.size());
            if (begin == std::string::npos || begin - found > 32) { found += key.size(); continue; }
            ++begin;
            while (begin < output.size() && std::isspace(static_cast<unsigned char>(output[begin])))
                ++begin;
            auto end = begin;
            while (end < output.size() && !std::isspace(static_cast<unsigned char>(output[end])) &&
                   output[end] != '&' && output[end] != ';' && output[end] != ',') ++end;
            if (end > begin) {
                output.replace(begin, end - begin, "[REDACTED]");
                lower.replace(begin, end - begin, "[redacted]");
                redacted = true;
                found = begin + 10;
            } else found = begin;
        }
    }
    for (std::size_t begin = 0; begin < output.size();) {
        if (!std::isdigit(static_cast<unsigned char>(output[begin]))) { ++begin; continue; }
        auto end = begin;
        while (end < output.size() && std::isdigit(static_cast<unsigned char>(output[end]))) ++end;
        const auto digits = std::string_view(output).substr(begin, end - begin);
        if (luhn_number(digits)) {
            output.replace(begin, digits.size(), "[PAYMENT_REDACTED]");
            redacted = true;
            begin += 18;
        } else begin = end;
    }
    return {true, std::move(output), redacted, {}};
}

}  // namespace owo::agent

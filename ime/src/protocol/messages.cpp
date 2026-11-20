#include "owo/protocol/messages.h"

#include <algorithm>
#include <array>
#include <charconv>
#include <cctype>
#include <optional>

namespace owo::protocol {
namespace {

std::string type_name(const MessageType type) {
    switch (type) {
        case MessageType::candidate_request: return "candidate_request";
        case MessageType::candidate_response: return "candidate_response";
        case MessageType::candidate_update_request: return "candidate_update_request";
        case MessageType::candidate_update_response: return "candidate_update_response";
        case MessageType::candidate_committed: return "candidate_committed";
        case MessageType::user_lexicon_request: return "user_lexicon_request";
        case MessageType::user_lexicon_response: return "user_lexicon_response";
        case MessageType::agent_context_recorded: return "agent_context_recorded";
        case MessageType::agent_request: return "agent_request";
        case MessageType::agent_response: return "agent_response";
        case MessageType::acknowledgement: return "acknowledgement";
        case MessageType::shutdown_request: return "shutdown_request";
        case MessageType::error_response: return "error_response";
    }
    return "error_response";
}

std::optional<MessageType> parse_type(const std::string_view value) {
    if (value == "candidate_request") return MessageType::candidate_request;
    if (value == "candidate_response") return MessageType::candidate_response;
    if (value == "candidate_update_request") return MessageType::candidate_update_request;
    if (value == "candidate_update_response") return MessageType::candidate_update_response;
    if (value == "candidate_committed") return MessageType::candidate_committed;
    if (value == "user_lexicon_request") return MessageType::user_lexicon_request;
    if (value == "user_lexicon_response") return MessageType::user_lexicon_response;
    if (value == "agent_context_recorded") return MessageType::agent_context_recorded;
    if (value == "agent_request") return MessageType::agent_request;
    if (value == "agent_response") return MessageType::agent_response;
    if (value == "acknowledgement") return MessageType::acknowledgement;
    if (value == "shutdown_request") return MessageType::shutdown_request;
    if (value == "error_response") return MessageType::error_response;
    return std::nullopt;
}

std::string escape_json(const std::string_view value) {
    std::string result;
    result.reserve(value.size());
    for (const unsigned char ch : value) {
        switch (ch) {
            case '"': result += "\\\""; break;
            case '\\': result += "\\\\"; break;
            case '\b': result += "\\b"; break;
            case '\f': result += "\\f"; break;
            case '\n': result += "\\n"; break;
            case '\r': result += "\\r"; break;
            case '\t': result += "\\t"; break;
            default:
                if (ch < 0x20U) return {};
                result.push_back(static_cast<char>(ch));
        }
    }
    return result;
}

bool consume(std::string_view input, std::size_t& offset, const std::string_view token) {
    if (input.substr(offset, token.size()) != token) return false;
    offset += token.size();
    return true;
}

std::optional<std::uint64_t> parse_uint(std::string_view input, std::size_t& offset) {
    const auto begin = input.data() + offset;
    const auto end = input.data() + input.size();
    std::uint64_t value{};
    const auto result = std::from_chars(begin, end, value);
    if (result.ec != std::errc{} || result.ptr == begin) return std::nullopt;
    offset = static_cast<std::size_t>(result.ptr - input.data());
    return value;
}

std::optional<std::string> parse_string(std::string_view input, std::size_t& offset) {
    if (!consume(input, offset, "\"")) return std::nullopt;
    std::string result;
    while (offset < input.size()) {
        const char ch = input[offset++];
        if (ch == '"') return result;
        if (static_cast<unsigned char>(ch) < 0x20U) return std::nullopt;
        if (ch != '\\') {
            result.push_back(ch);
            continue;
        }
        if (offset >= input.size()) return std::nullopt;
        switch (input[offset++]) {
            case '"': result.push_back('"'); break;
            case '\\': result.push_back('\\'); break;
            case 'b': result.push_back('\b'); break;
            case 'f': result.push_back('\f'); break;
            case 'n': result.push_back('\n'); break;
            case 'r': result.push_back('\r'); break;
            case 't': result.push_back('\t'); break;
            default: return std::nullopt;
        }
    }
    return std::nullopt;
}

std::optional<std::vector<std::string>> parse_string_array(std::string_view input,
                                                           std::size_t& offset) {
    if (!consume(input, offset, "[")) return std::nullopt;
    std::vector<std::string> values;
    if (consume(input, offset, "]")) return values;
    while (true) {
        auto value = parse_string(input, offset);
        if (!value) return std::nullopt;
        values.push_back(std::move(*value));
        if (consume(input, offset, "]")) return values;
        if (!consume(input, offset, ",")) return std::nullopt;
    }
}

std::optional<std::vector<std::uint64_t>> parse_uint_array(std::string_view input,
                                                           std::size_t& offset) {
    if (!consume(input, offset, "[")) return std::nullopt;
    std::vector<std::uint64_t> values;
    if (consume(input, offset, "]")) return values;
    while (true) {
        const auto value = parse_uint(input, offset);
        if (!value) return std::nullopt;
        values.push_back(*value);
        if (consume(input, offset, "]")) return values;
        if (!consume(input, offset, ",")) return std::nullopt;
    }
}

std::optional<bool> parse_bool(std::string_view input, std::size_t& offset) {
    if (consume(input, offset, "true")) return true;
    if (consume(input, offset, "false")) return false;
    return std::nullopt;
}

bool valid_syllables(const std::vector<std::string>& syllables) {
    if (syllables.size() > 256) return false;
    for (const auto& syllable : syllables) {
        if (syllable.empty() || syllable.size() > kMaximumSyllableBytes) return false;
        for (const unsigned char value : syllable) {
            if (value < 'a' || value > 'z') return false;
        }
    }
    return true;
}

bool valid_candidate_consumed(const Message& message) {
    const bool is_candidate_response =
        message.type == MessageType::candidate_response ||
        message.type == MessageType::candidate_update_response;
    if (!is_candidate_response)
        return message.candidate_consumed.empty();
    if (message.candidate_consumed.size() != message.candidates.size() ||
        message.candidate_consumed.size() > kMaximumCandidateCount) return false;
    return std::all_of(message.candidate_consumed.begin(),
                       message.candidate_consumed.end(), [](const std::uint64_t value) {
        return value > 0 && value <= kMaximumPayloadBytes;
    });
}

bool valid_candidate_groups(const Message& message) {
    const bool is_candidate_response =
        message.type == MessageType::candidate_response ||
        message.type == MessageType::candidate_update_response;
    if (!is_candidate_response) return message.candidate_groups.empty();
    if (message.candidate_groups.empty()) return true;
    if (message.candidate_groups.size() != message.candidates.size()) return false;
    return std::all_of(message.candidate_groups.begin(),
                       message.candidate_groups.end(), [](const std::uint64_t value) {
        return value <= static_cast<std::uint64_t>(CandidateGroup::continuation);
    });
}

bool valid_candidate_layout(const Message& message) {
    const bool is_candidate_request = message.type == MessageType::candidate_request;
    const bool is_candidate_response = message.type == MessageType::candidate_response;
    if (is_candidate_request) return message.page_size == 0;
    if (is_candidate_response) return message.page_size >= 1 && message.page_size <= 9;
    return !message.expanded && message.page_size == 0;
}

bool valid_feedback_input(const Message& message) {
    if (message.type != MessageType::candidate_committed &&
        message.type != MessageType::candidate_request)
        return message.input.empty();
    if (message.input.size() > 4096) return false;
    return std::all_of(message.input.begin(), message.input.end(), [](const char value) {
        return (value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z') ||
               value == '\'';
    });
}

bool valid_language_context(const Message& message) {
    if (message.type != MessageType::candidate_request &&
        message.type != MessageType::candidate_committed)
        return message.context.empty();
    return message.context.size() <= 256;
}

bool valid_candidate_refinement(const Message& message) {
    if (message.defer_expensive_candidates &&
        message.type != MessageType::candidate_request) return false;
    if (message.candidate_refinement_pending &&
        message.type != MessageType::candidate_response) return false;
    return true;
}

bool valid_corpus_eligibility(const Message& message) {
    return !message.corpus_eligible ||
           (message.type == MessageType::agent_context_recorded &&
            !message.application_sensitive_input);
}

bool valid_application_context(const Message& message) {
    const bool empty = message.application_context_id.empty() &&
        message.application_process_id == 0 && message.application_thread_id == 0 &&
        message.application_executable.empty() && message.application_window_class.empty() &&
        message.application_focus_window_class.empty() &&
        !message.application_sensitive_input;
    if (message.type != MessageType::candidate_committed &&
        message.type != MessageType::agent_context_recorded) return empty;
    if (message.type == MessageType::agent_context_recorded &&
        (message.text.empty() || message.text.size() > 32768)) return false;
    if (message.application_context_id.empty()) return empty;
    if (message.application_context_id.size() > 128 ||
        message.application_process_id > 0xffffffffULL ||
        message.application_thread_id > 0xffffffffULL ||
        message.application_executable.size() > 260 ||
        message.application_window_class.size() > 128 ||
        message.application_focus_window_class.size() > 128) return false;
    return std::all_of(message.application_context_id.begin(),
                       message.application_context_id.end(), [](const unsigned char value) {
        return std::isalnum(value) != 0 || value == '-' || value == '_' || value == '.';
    });
}

}  // namespace

std::string encode_message(const Message& message) {
    if (!valid_syllables(message.syllables) || !valid_candidate_consumed(message) ||
        !valid_candidate_groups(message) ||
        !valid_candidate_layout(message) || !valid_feedback_input(message) ||
        !valid_language_context(message) ||
        !valid_candidate_refinement(message) ||
        !valid_application_context(message) ||
        !valid_corpus_eligibility(message)) return {};
    const auto escaped = escape_json(message.text);
    if (!message.text.empty() && escaped.empty()) return {};
    const auto escaped_input = escape_json(message.input);
    if (!message.input.empty() && escaped_input.empty()) return {};
    const auto escaped_context = escape_json(message.context);
    if (!message.context.empty() && escaped_context.empty()) return {};
    const auto escaped_application_context_id = escape_json(message.application_context_id);
    const auto escaped_application_executable = escape_json(message.application_executable);
    const auto escaped_application_window_class = escape_json(message.application_window_class);
    const auto escaped_application_focus_window_class =
        escape_json(message.application_focus_window_class);
    std::string encoded_candidates = "[";
    for (std::size_t index = 0; index < message.candidates.size(); ++index) {
        const auto candidate = escape_json(message.candidates[index]);
        if (!message.candidates[index].empty() && candidate.empty()) return {};
        if (index != 0) encoded_candidates += ',';
        encoded_candidates += "\"" + candidate + "\"";
    }
    encoded_candidates += ']';
    std::string encoded_syllables = "[";
    for (std::size_t index = 0; index < message.syllables.size(); ++index) {
        const auto syllable = escape_json(message.syllables[index]);
        if (!message.syllables[index].empty() && syllable.empty()) return {};
        if (index != 0) encoded_syllables += ',';
        encoded_syllables += "\"" + syllable + "\"";
    }
    encoded_syllables += ']';
    std::string encoded_consumed = "[";
    for (std::size_t index = 0; index < message.candidate_consumed.size(); ++index) {
        if (index != 0) encoded_consumed += ',';
        encoded_consumed += std::to_string(message.candidate_consumed[index]);
    }
    encoded_consumed += ']';
    std::string encoded_groups = "[";
    for (std::size_t index = 0; index < message.candidate_groups.size(); ++index) {
        if (index != 0) encoded_groups += ',';
        encoded_groups += std::to_string(message.candidate_groups[index]);
    }
    encoded_groups += ']';
    return "{\"protocol_version\":" + std::to_string(kProtocolVersion) +
           ",\"type\":\"" + type_name(message.type) +
           "\",\"request_id\":" + std::to_string(message.request_id) +
           ",\"context_generation\":" + std::to_string(message.context_generation) +
           ",\"text\":\"" + escaped + "\",\"candidates\":" + encoded_candidates +
           ",\"page\":" + std::to_string(message.page) +
           ",\"has_more\":" + (message.has_more ? "true" : "false") +
           ",\"model_pending\":" + (message.model_pending ? "true" : "false") +
           ",\"defer_expensive_candidates\":" +
               (message.defer_expensive_candidates ? "true" : "false") +
           ",\"candidate_refinement_pending\":" +
               (message.candidate_refinement_pending ? "true" : "false") +
           ",\"syllables\":" + encoded_syllables +
           ",\"candidate_consumed\":" + encoded_consumed +
           ",\"candidate_groups\":" + encoded_groups +
           ",\"expanded\":" + (message.expanded ? "true" : "false") +
           ",\"page_size\":" + std::to_string(message.page_size) +
           ",\"correction_enabled\":" +
               (message.correction_enabled ? "true" : "false") +
           ",\"input\":\"" + escaped_input +
           "\",\"context\":\"" + escaped_context +
           "\",\"application_context_id\":\"" + escaped_application_context_id +
           "\",\"application_process_id\":" +
               std::to_string(message.application_process_id) +
           ",\"application_thread_id\":" +
               std::to_string(message.application_thread_id) +
           ",\"application_executable\":\"" + escaped_application_executable +
           "\",\"application_window_class\":\"" + escaped_application_window_class +
           "\",\"application_focus_window_class\":\"" +
               escaped_application_focus_window_class +
           "\",\"application_sensitive_input\":" +
               (message.application_sensitive_input ? "true" : "false") +
           ",\"corpus_eligible\":" + (message.corpus_eligible ? "true" : "false") + "}";
}

DecodeResult decode_message(const std::string_view json) {
    DecodeResult output{};
    if (json.size() > kMaximumPayloadBytes) {
        output.validation = {ErrorCode::payload_too_large, "message exceeds limit"};
        return output;
    }

    std::size_t offset = 0;
    if (!consume(json, offset, "{\"protocol_version\":")) goto invalid;
    {
        const auto version = parse_uint(json, offset);
        if (!version || *version != kProtocolVersion) {
            output.validation = {ErrorCode::unsupported_protocol, "unsupported protocol version"};
            return output;
        }
    }
    if (!consume(json, offset, ",\"type\":")) goto invalid;
    {
        const auto type_text = parse_string(json, offset);
        if (!type_text) goto invalid;
        const auto type = parse_type(*type_text);
        if (!type) goto invalid;
        output.message.type = *type;
    }
    if (!consume(json, offset, ",\"request_id\":")) goto invalid;
    {
        const auto value = parse_uint(json, offset);
        if (!value) goto invalid;
        output.message.request_id = *value;
    }
    if (!consume(json, offset, ",\"context_generation\":")) goto invalid;
    {
        const auto value = parse_uint(json, offset);
        if (!value) goto invalid;
        output.message.context_generation = *value;
    }
    if (!consume(json, offset, ",\"text\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.text = *value;
    }
    if (!consume(json, offset, ",\"candidates\":")) goto invalid;
    {
        auto values = parse_string_array(json, offset);
        if (!values) goto invalid;
        output.message.candidates = std::move(*values);
    }
    if (!consume(json, offset, ",\"page\":")) goto invalid;
    {
        const auto value = parse_uint(json, offset);
        if (!value) goto invalid;
        output.message.page = *value;
    }
    if (!consume(json, offset, ",\"has_more\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.has_more = *value;
    }
    if (!consume(json, offset, ",\"model_pending\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.model_pending = *value;
    }
    if (!consume(json, offset, ",\"defer_expensive_candidates\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.defer_expensive_candidates = *value;
    }
    if (!consume(json, offset, ",\"candidate_refinement_pending\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.candidate_refinement_pending = *value;
    }
    if (!consume(json, offset, ",\"syllables\":")) goto invalid;
    {
        auto values = parse_string_array(json, offset);
        if (!values || !valid_syllables(*values)) goto invalid;
        output.message.syllables = std::move(*values);
    }
    if (!consume(json, offset, ",\"candidate_consumed\":")) goto invalid;
    {
        auto values = parse_uint_array(json, offset);
        if (!values) goto invalid;
        output.message.candidate_consumed = std::move(*values);
    }
    if (!consume(json, offset, ",\"candidate_groups\":")) goto invalid;
    {
        auto values = parse_uint_array(json, offset);
        if (!values) goto invalid;
        output.message.candidate_groups = std::move(*values);
    }
    if (!consume(json, offset, ",\"expanded\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.expanded = *value;
    }
    if (!consume(json, offset, ",\"page_size\":")) goto invalid;
    {
        const auto value = parse_uint(json, offset);
        if (!value) goto invalid;
        output.message.page_size = *value;
    }
    if (!consume(json, offset, ",\"correction_enabled\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.correction_enabled = *value;
    }
    if (!consume(json, offset, ",\"input\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.input = *value;
    }
    if (!consume(json, offset, ",\"context\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.context = *value;
    }
    if (!consume(json, offset, ",\"application_context_id\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.application_context_id = *value;
    }
    if (!consume(json, offset, ",\"application_process_id\":")) goto invalid;
    {
        const auto value = parse_uint(json, offset);
        if (!value) goto invalid;
        output.message.application_process_id = *value;
    }
    if (!consume(json, offset, ",\"application_thread_id\":")) goto invalid;
    {
        const auto value = parse_uint(json, offset);
        if (!value) goto invalid;
        output.message.application_thread_id = *value;
    }
    if (!consume(json, offset, ",\"application_executable\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.application_executable = *value;
    }
    if (!consume(json, offset, ",\"application_window_class\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.application_window_class = *value;
    }
    if (!consume(json, offset, ",\"application_focus_window_class\":")) goto invalid;
    {
        const auto value = parse_string(json, offset);
        if (!value) goto invalid;
        output.message.application_focus_window_class = *value;
    }
    if (!consume(json, offset, ",\"application_sensitive_input\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.application_sensitive_input = *value;
    }
    if (!consume(json, offset, ",\"corpus_eligible\":")) goto invalid;
    {
        const auto value = parse_bool(json, offset);
        if (!value) goto invalid;
        output.message.corpus_eligible = *value;
    }
    if (!consume(json, offset, "}") || offset != json.size()) goto invalid;
    if (!valid_candidate_consumed(output.message) ||
        !valid_candidate_groups(output.message) ||
        !valid_candidate_layout(output.message) ||
        !valid_feedback_input(output.message) ||
        !valid_language_context(output.message) ||
        !valid_candidate_refinement(output.message) ||
        !valid_application_context(output.message) ||
        !valid_corpus_eligibility(output.message)) goto invalid;
    output.validation = {};
    return output;

invalid:
    output.validation = {ErrorCode::invalid_payload, "invalid internal message schema"};
    return output;
}

}  // namespace owo::protocol

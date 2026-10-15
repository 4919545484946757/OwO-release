#include "owo/plugin/plugin_protocol.h"

#include <algorithm>
#include <array>
#include <limits>
#include <utility>

namespace owo::plugin {
namespace {

constexpr std::array<char, 4> kMagic{'O', 'W', 'P', 'H'};
constexpr std::size_t kMaximumCapabilities = 32;
constexpr std::size_t kMaximumDiagnosticBytes = 4096;
constexpr std::uint32_t kMaximumTimeoutMs = 30000;

template <typename Integer>
void append_integer(std::string& output, const Integer value) {
    for (std::size_t shift = 0; shift < sizeof(Integer) * 8U; shift += 8U)
        output.push_back(static_cast<char>(value >> shift));
}

bool append_string(std::string& output, const std::string_view value) {
    if (value.size() > std::numeric_limits<std::uint32_t>::max()) return false;
    append_integer(output, static_cast<std::uint32_t>(value.size()));
    output.append(value);
    return true;
}

template <typename Integer>
bool read_integer(const std::string_view input, std::size_t& offset, Integer& value) {
    if (offset + sizeof(Integer) > input.size()) return false;
    value = 0;
    for (std::size_t index = 0; index < sizeof(Integer); ++index)
        value |= static_cast<Integer>(static_cast<unsigned char>(input[offset + index])) <<
                 (index * 8U);
    offset += sizeof(Integer);
    return true;
}

bool read_string(const std::string_view input, std::size_t& offset, std::string& value,
                 const std::size_t maximum) {
    std::uint32_t size{};
    if (!read_integer(input, offset, size) || size > maximum || offset + size > input.size())
        return false;
    value.assign(input.substr(offset, size));
    offset += size;
    return true;
}

bool plugin_id(const std::string_view value) {
    if (value.size() < 3 || value.size() > 128 || value.front() == '.' ||
        value.back() == '.' || value.find('.') == std::string_view::npos) return false;
    bool previous_dot = false;
    for (const unsigned char byte : value) {
        const bool valid = (byte >= 'a' && byte <= 'z') || (byte >= '0' && byte <= '9') ||
                           byte == '-' || byte == '.';
        if (!valid || (byte == '.' && previous_dot)) return false;
        previous_dot = byte == '.';
    }
    return true;
}

bool token(const std::string_view value) {
    if (value.empty() || value.size() > 128 || value.front() == '.' || value.back() == '.')
        return false;
    bool previous_dot = false;
    for (const unsigned char byte : value) {
        const bool valid = (byte >= 'a' && byte <= 'z') || (byte >= '0' && byte <= '9') ||
                           byte == '-' || byte == '_' || byte == '.';
        if (!valid || (byte == '.' && previous_dot)) return false;
        previous_dot = byte == '.';
    }
    return true;
}

bool valid_utf8(const std::string_view text) {
    std::size_t offset = 0;
    while (offset < text.size()) {
        const auto first = static_cast<unsigned char>(text[offset]);
        std::size_t count = 0;
        std::uint32_t scalar = 0;
        if (first <= 0x7fU) { count = 1; scalar = first; }
        else if (first >= 0xc2U && first <= 0xdfU) { count = 2; scalar = first & 0x1fU; }
        else if (first >= 0xe0U && first <= 0xefU) { count = 3; scalar = first & 0x0fU; }
        else if (first >= 0xf0U && first <= 0xf4U) { count = 4; scalar = first & 0x07U; }
        else return false;
        if (offset + count > text.size()) return false;
        for (std::size_t index = 1; index < count; ++index) {
            const auto next = static_cast<unsigned char>(text[offset + index]);
            if ((next & 0xc0U) != 0x80U) return false;
            scalar = (scalar << 6U) | (next & 0x3fU);
        }
        if ((count == 3 && scalar < 0x800U) || (count == 4 && scalar < 0x10000U) ||
            (scalar >= 0xd800U && scalar <= 0xdfffU) || scalar > 0x10ffffU) return false;
        offset += count;
    }
    return true;
}

bool valid_type(const PluginMessageType type) {
    return type >= PluginMessageType::hello_request && type <= PluginMessageType::error_response;
}

bool valid_status(const PluginStatus status) {
    return status >= PluginStatus::success && status <= PluginStatus::plugin_error;
}

bool valid_message(const PluginMessage& message) {
    if (!valid_type(message.type) || !valid_status(message.status) || message.request_id == 0 ||
        !plugin_id(message.plugin_id) || message.payload.size() > kMaximumPluginPayloadBytes ||
        message.diagnostic.size() > kMaximumDiagnosticBytes ||
        !valid_utf8(message.diagnostic) ||
        message.capabilities.size() > kMaximumCapabilities ||
        !std::all_of(message.capabilities.begin(), message.capabilities.end(), token)) return false;
    if (!std::is_sorted(message.capabilities.begin(), message.capabilities.end()) ||
        std::adjacent_find(message.capabilities.begin(), message.capabilities.end()) !=
            message.capabilities.end()) return false;
    const bool no_response_fields = message.status == PluginStatus::success &&
        message.diagnostic.empty();
    const bool valid_response_fields = message.status == PluginStatus::success
        ? message.diagnostic.empty() : !message.diagnostic.empty();
    switch (message.type) {
    case PluginMessageType::hello_request:
        return no_response_fields && message.target_request_id == 0 && message.timeout_ms == 0 &&
               message.service.empty() && message.payload.empty() && message.capabilities.empty();
    case PluginMessageType::hello_response:
        return no_response_fields && message.target_request_id == 0 && message.timeout_ms == 0 &&
               message.service.empty() && message.payload.empty();
    case PluginMessageType::invoke_request:
        return no_response_fields && message.target_request_id == 0 && message.timeout_ms > 0 &&
               message.timeout_ms <= kMaximumTimeoutMs && token(message.service) &&
               message.capabilities.empty();
    case PluginMessageType::invoke_response:
        return valid_response_fields && message.target_request_id == 0 && message.timeout_ms == 0 &&
               message.service.empty() && message.capabilities.empty();
    case PluginMessageType::cancel_request:
        return no_response_fields && message.target_request_id != 0 &&
               message.target_request_id != message.request_id && message.timeout_ms == 0 &&
               message.service.empty() && message.payload.empty() && message.capabilities.empty();
    case PluginMessageType::shutdown_request:
        return no_response_fields && message.target_request_id == 0 && message.timeout_ms == 0 &&
               message.service.empty() && message.payload.empty() && message.capabilities.empty();
    case PluginMessageType::acknowledgement:
        return message.status == PluginStatus::success && message.target_request_id == 0 &&
               message.timeout_ms == 0 && message.service.empty() && message.payload.empty() &&
               message.diagnostic.empty() && message.capabilities.empty();
    case PluginMessageType::error_response:
        return message.status != PluginStatus::success && message.target_request_id == 0 &&
               message.timeout_ms == 0 && message.service.empty() && message.payload.empty() &&
               !message.diagnostic.empty() && message.capabilities.empty();
    }
    return false;
}

}  // namespace

std::string encode_plugin_message(const PluginMessage& message) {
    if (!valid_message(message)) return {};
    std::string output(kMagic.begin(), kMagic.end());
    append_integer(output, kPluginProtocolVersion);
    append_integer(output, static_cast<std::uint8_t>(message.type));
    append_integer(output, static_cast<std::uint8_t>(message.status));
    append_integer(output, static_cast<std::uint16_t>(0));
    append_integer(output, message.request_id);
    append_integer(output, message.target_request_id);
    append_integer(output, message.timeout_ms);
    append_integer(output, static_cast<std::uint16_t>(message.capabilities.size()));
    append_integer(output, static_cast<std::uint16_t>(0));
    if (!append_string(output, message.plugin_id) || !append_string(output, message.service) ||
        !append_string(output, message.payload) || !append_string(output, message.diagnostic)) return {};
    for (const auto& capability : message.capabilities)
        if (!append_string(output, capability)) return {};
    if (output.size() > protocol::kMaximumPayloadBytes) return {};
    return output;
}

PluginDecodeResult decode_plugin_message(const std::string_view payload) {
    PluginDecodeResult result;
    if (payload.size() > protocol::kMaximumPayloadBytes) {
        result.validation = {protocol::ErrorCode::payload_too_large,
                             "plugin message exceeds transport limit"};
        return result;
    }
    if (payload.size() < kMagic.size() ||
        !std::equal(kMagic.begin(), kMagic.end(), payload.begin())) goto invalid;
    {
        std::size_t offset = kMagic.size();
        std::uint32_t version{};
        std::uint8_t type{};
        std::uint8_t status{};
        std::uint16_t reserved{};
        std::uint16_t capability_count{};
        std::uint16_t reserved_tail{};
        if (!read_integer(payload, offset, version) || version != kPluginProtocolVersion) {
            result.validation = {protocol::ErrorCode::unsupported_protocol,
                                 "unsupported plugin protocol version"};
            return result;
        }
        if (!read_integer(payload, offset, type) || !read_integer(payload, offset, status) ||
            !read_integer(payload, offset, reserved) || reserved != 0 ||
            !read_integer(payload, offset, result.message.request_id) ||
            !read_integer(payload, offset, result.message.target_request_id) ||
            !read_integer(payload, offset, result.message.timeout_ms) ||
            !read_integer(payload, offset, capability_count) ||
            capability_count > kMaximumCapabilities ||
            !read_integer(payload, offset, reserved_tail) || reserved_tail != 0)
            goto invalid;
        result.message.type = static_cast<PluginMessageType>(type);
        result.message.status = static_cast<PluginStatus>(status);
        if (!read_string(payload, offset, result.message.plugin_id, 128) ||
            !read_string(payload, offset, result.message.service, 128) ||
            !read_string(payload, offset, result.message.payload, kMaximumPluginPayloadBytes) ||
            !read_string(payload, offset, result.message.diagnostic, kMaximumDiagnosticBytes))
            goto invalid;
        result.message.capabilities.reserve(capability_count);
        for (std::uint16_t index = 0; index < capability_count; ++index) {
            std::string capability;
            if (!read_string(payload, offset, capability, 128)) goto invalid;
            result.message.capabilities.push_back(std::move(capability));
        }
        if (offset != payload.size() || !valid_message(result.message)) goto invalid;
    }
    result.validation = {};
    return result;

invalid:
    result.validation = {protocol::ErrorCode::invalid_payload, "invalid plugin message schema"};
    return result;
}

}  // namespace owo::plugin

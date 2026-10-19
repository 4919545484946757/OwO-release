#include "owo/protocol/envelope.h"

namespace owo::protocol {

ValidationResult validate(const Envelope& envelope) {
    if (envelope.protocol_version != kProtocolVersion) {
        return {ErrorCode::unsupported_protocol, "unsupported protocol version"};
    }
    if (envelope.payload_json.size() > kMaximumPayloadBytes) {
        return {ErrorCode::payload_too_large, "payload exceeds the P0 limit"};
    }
    if (envelope.payload_json.empty()) {
        return {ErrorCode::invalid_payload, "payload must not be empty"};
    }
    return {};
}

std::string frame(const std::string_view payload) {
    const auto size = static_cast<std::uint32_t>(payload.size());
    std::string result;
    result.reserve(sizeof(size) + payload.size());
    result.push_back(static_cast<char>(size & 0xFFU));
    result.push_back(static_cast<char>((size >> 8U) & 0xFFU));
    result.push_back(static_cast<char>((size >> 16U) & 0xFFU));
    result.push_back(static_cast<char>((size >> 24U) & 0xFFU));
    result.append(payload);
    return result;
}

}  // namespace owo::protocol


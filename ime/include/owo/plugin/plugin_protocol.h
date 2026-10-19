#pragma once

#include "owo/protocol/envelope.h"

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace owo::plugin {

inline constexpr std::uint32_t kPluginProtocolVersion = 1;
inline constexpr std::size_t kMaximumPluginPayloadBytes = 256U * 1024U;

enum class PluginMessageType : std::uint8_t {
    hello_request = 1,
    hello_response = 2,
    invoke_request = 3,
    invoke_response = 4,
    cancel_request = 5,
    shutdown_request = 6,
    acknowledgement = 7,
    error_response = 8,
};

enum class PluginStatus : std::uint8_t {
    success = 0,
    invalid_request = 1,
    permission_denied = 2,
    unsupported_protocol = 3,
    timeout = 4,
    cancelled = 5,
    plugin_error = 6,
};

struct PluginMessage {
    PluginMessageType type{PluginMessageType::error_response};
    PluginStatus status{PluginStatus::plugin_error};
    std::uint64_t request_id{};
    std::uint64_t target_request_id{};
    std::uint32_t timeout_ms{};
    std::string plugin_id;
    std::string service;
    std::string payload;
    std::string diagnostic;
    std::vector<std::string> capabilities;
};

struct PluginDecodeResult {
    PluginMessage message;
    protocol::ValidationResult validation;
};

/// Encodes one bounded PluginHost v1 message. Invalid type-specific combinations return empty.
[[nodiscard]] std::string encode_plugin_message(const PluginMessage& message);

/// Strictly decodes one complete PluginHost v1 message; trailing bytes and reserved bits fail.
[[nodiscard]] PluginDecodeResult decode_plugin_message(std::string_view payload);

}  // namespace owo::plugin

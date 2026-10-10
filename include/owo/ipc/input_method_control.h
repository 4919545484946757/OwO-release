#pragma once

#include "owo/protocol/envelope.h"
#include <cstddef>

namespace owo::ipc {

inline constexpr std::size_t kMaximumInputMethodControlPayload = 128 * 1024;
inline constexpr std::size_t kMaximumInputMethodControlWire = kMaximumInputMethodControlPayload + 2048;
// Experimental, additive to the existing Core candidate protocol.
enum class InputMethodControlAction { discover, open, submit, poll, close };
enum class InputMethodControlStatus { pending, ready, rejected, error };

struct InputMethodControlRequest {
    std::uint64_t request_id{};
    InputMethodControlAction action{InputMethodControlAction::discover};
    std::uint64_t ticket{};
    std::string plugin_id;
    std::string scheme_id;
    std::string session_id;
    std::string payload;
    // A client can request stricter blocking, never override the host's focus checks.
    bool sensitive_input{};
};
struct InputMethodControlResponse {
    std::uint64_t request_id{};
    std::uint64_t ticket{};
    InputMethodControlStatus status{InputMethodControlStatus::error};
    std::string payload;
    std::string diagnostic;
};
[[nodiscard]] std::string encode_input_method_control_request(const InputMethodControlRequest&);
[[nodiscard]] protocol::ValidationResult decode_input_method_control_request(
    std::string_view, InputMethodControlRequest&);
[[nodiscard]] std::string encode_input_method_control_response(const InputMethodControlResponse&);
[[nodiscard]] protocol::ValidationResult decode_input_method_control_response(
    std::string_view, InputMethodControlResponse&);

} // namespace owo::ipc

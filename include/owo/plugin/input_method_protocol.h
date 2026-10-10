#pragma once

#include "owo/engine/input_method_provider.h"

namespace owo::plugin {

// Experimental payload inside PluginHost v1; not a stable public wire ABI.
inline constexpr std::string_view kInputMethodService = "owo.input-method.experimental.v1";
inline constexpr std::size_t kMaximumInputMethodWireBytes = 128 * 1024;

[[nodiscard]] std::string encode_input_method_request(
    const engine::InputMethodDescriptor&, const engine::InputMethodRequest&);
[[nodiscard]] engine::InputMethodValidation decode_input_method_request(
    const engine::InputMethodDescriptor&, std::string_view, engine::InputMethodRequest&);
[[nodiscard]] std::string encode_input_method_response(
    const engine::InputMethodDescriptor&, const engine::InputMethodRequest&,
    const engine::InputMethodResponse&);
[[nodiscard]] engine::InputMethodValidation decode_input_method_response(
    const engine::InputMethodDescriptor&, const engine::InputMethodRequest&,
    std::string_view, engine::InputMethodResponse&);

} // namespace owo::plugin

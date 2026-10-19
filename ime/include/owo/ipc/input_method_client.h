#pragma once
#include "owo/ipc/input_method_server.h"
#include "owo/engine/input_method_provider.h"
#include "owo/protocol/envelope.h"
#include <future>
#include <optional>

namespace owo::ipc {
enum class InputMethodClientOperation { none, discover, open, submit, close };
struct InputMethodClientResult {
    std::uint64_t document_generation{};
    protocol::ValidationResult status;
    std::vector<engine::InputMethodDescriptor> descriptors;
    std::optional<engine::InputMethodResponse> response;
    InputMethodClientOperation operation{InputMethodClientOperation::none};
};
// One background thread, one latest pending task, persistent connection owned by that thread.
// No UI callbacks, COM pointers, plugin permissions or application writes.
class InputMethodAsyncClient final {
public:
    explicit InputMethodAsyncClient(std::wstring pipe_name = kInputMethodPipeName);
    ~InputMethodAsyncClient();
    InputMethodAsyncClient(const InputMethodAsyncClient&) = delete;
    InputMethodAsyncClient& operator=(const InputMethodAsyncClient&) = delete;
    [[nodiscard]] std::future<InputMethodClientResult> discover(
        std::uint64_t document_generation, std::string plugin_id);
    // Sensitive is supplied by trusted TSF/host code. Default is fail-closed.
    [[nodiscard]] std::future<InputMethodClientResult> open(std::uint64_t document_generation,
        std::string plugin_id, std::string scheme_id, std::string session_id, bool sensitive = true);
    [[nodiscard]] std::future<InputMethodClientResult> submit(std::uint64_t document_generation,
        engine::InputMethodRequest, bool sensitive = true);
    // Immediately invalidates local results; background reset closes the server-side session.
    [[nodiscard]] std::future<InputMethodClientResult> close();
private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};
} // namespace owo::ipc

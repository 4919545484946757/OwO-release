#pragma once

#include <cstdint>
#include <filesystem>
#include <functional>
#include <memory>
#include <string>

namespace owo::ipc {
inline constexpr wchar_t kInputMethodPipeName[] =
    LR"(\\.\pipe\OwO.InputMethod.Core.InputSchemes.experimental.v1)";

struct InputMethodFocus {
    std::uint64_t identity{};
    bool sensitive{true};
};
// Dedicated experimental transport, independent of the existing candidate pipe.
// The resolver is a trusted host/test hook, never supplied through IPC or plugins.
// Default policy accepts only foreground standard Edit/RichEdit controls, not password fields.
class InputMethodPipeServer final {
public:
    using FocusResolver = std::function<InputMethodFocus(std::uint32_t client_pid)>;
    explicit InputMethodPipeServer(std::filesystem::path plugin_store,
        std::wstring pipe_name = kInputMethodPipeName, FocusResolver resolver = {});
    ~InputMethodPipeServer();
    InputMethodPipeServer(const InputMethodPipeServer&) = delete;
    InputMethodPipeServer& operator=(const InputMethodPipeServer&) = delete;
    [[nodiscard]] bool started() const noexcept;
private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};
} // namespace owo::ipc

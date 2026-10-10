#pragma once

#include "owo/engine/input_method_provider.h"

#include <chrono>
#include <filesystem>
#include <future>

namespace owo::core {

// Host-approved metadata only. This does not install/enable a plugin or grant permission.
// Experimental gate: active process plugin must already hold input.context authorization.
[[nodiscard]] engine::InputMethodValidation register_plugin_input_method(
    engine::InputMethodRegistry&, const std::filesystem::path& plugin_store_root,
    engine::InputMethodDescriptor, std::chrono::milliseconds timeout = std::chrono::milliseconds(1000));

// One dedicated worker and one pending request. New input cancels previous work.
// Registry must outlive this object. Never wait on returned futures in a TSF key callback.
class InputMethodWorker {
public:
    explicit InputMethodWorker(engine::InputMethodRegistry&);
    ~InputMethodWorker();
    InputMethodWorker(const InputMethodWorker&) = delete;
    InputMethodWorker& operator=(const InputMethodWorker&) = delete;
    [[nodiscard]] std::future<engine::InputMethodResponse> submit(engine::InputMethodRequest);
private:
    struct Impl;
    std::unique_ptr<Impl> implementation_;
};

} // namespace owo::core

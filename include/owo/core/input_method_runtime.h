#pragma once

#include "owo/engine/input_method_provider.h"

#include <chrono>
#include <filesystem>
#include <future>
#include <optional>

namespace owo::core {

// Host-approved metadata only. This does not install/enable a plugin or grant permission.
// Active process plugin must already hold explicit input.scheme authorization.
[[nodiscard]] engine::InputMethodValidation register_plugin_input_method(
    engine::InputMethodRegistry&, const std::filesystem::path& plugin_store_root,
    engine::InputMethodDescriptor, std::chrono::milliseconds timeout = std::chrono::milliseconds(1000),
    std::string_view expected_version = {}, std::string_view expected_inventory = {},
    std::string_view expected_publisher = {});

struct InputMethodDiscovery {
    engine::InputMethodValidation validation;
    std::vector<engine::InputMethodDescriptor> descriptors;
    std::string version;
    std::string inventory;
    std::string publisher;
};

// Explicitly requested discovery only, on a background thread. Never grants/enables plugins.
[[nodiscard]] InputMethodDiscovery discover_plugin_input_methods(
    const std::filesystem::path& plugin_store_root, std::string_view plugin_id,
    std::chrono::milliseconds timeout = std::chrono::milliseconds(1000));

// One dedicated worker and one pending request. New input cancels previous work.
// Registry must outlive this object. Never wait on returned futures in a TSF key callback.
class InputMethodWorker {
public:
    explicit InputMethodWorker(engine::InputMethodRegistry&);
    ~InputMethodWorker();
    InputMethodWorker(const InputMethodWorker&) = delete;
    InputMethodWorker& operator=(const InputMethodWorker&) = delete;
    [[nodiscard]] std::future<engine::InputMethodResponse> submit(engine::InputMethodRequest);
    void cancel();
private:
    struct Impl;
    std::unique_ptr<Impl> implementation_;
};

// Core-side request gate for a single active composition. Owner IDs must be assigned/verified
// by the host, not taken from an untrusted plugin payload. Transport/TSF binding is separate.
class InputMethodSessionRouter {
public:
    explicit InputMethodSessionRouter(std::filesystem::path plugin_store_root);
    ~InputMethodSessionRouter();
    InputMethodSessionRouter(const InputMethodSessionRouter&) = delete;
    InputMethodSessionRouter& operator=(const InputMethodSessionRouter&) = delete;
    // Discovery/launch may wait; call open only on the host's control worker.
    [[nodiscard]] engine::InputMethodValidation open(std::uint64_t owner_id,
        std::string session_id, std::string_view plugin_id, std::string_view scheme_id);
    [[nodiscard]] std::future<engine::InputMethodResponse> submit(std::uint64_t owner_id,
        engine::InputMethodRequest, bool sensitive_input);
    [[nodiscard]] bool close(std::uint64_t owner_id, std::string_view session_id);
    // Snapshot revalidates active package/authorization; use on control/result publication paths,
    // not inside a key callback. Null on closed, replaced or revoked bindings.
    [[nodiscard]] std::optional<engine::InputMethodDescriptor> descriptor(
        std::uint64_t owner_id, std::string_view session_id) const;
private:
    struct Impl;
    std::unique_ptr<Impl> implementation_;
};

} // namespace owo::core

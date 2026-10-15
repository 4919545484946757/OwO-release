#pragma once

#include "owo/plugin/plugin_manifest.h"
#include "owo/plugin/plugin_protocol.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <memory>
#include <stop_token>
#include <string>
#include <string_view>
#include <vector>

namespace owo::plugin {

struct PluginHostOperationResult {
    bool ok{};
    bool forced_termination{};
    std::string diagnostic;
};

inline constexpr std::size_t kMaximumConcurrentPluginInvocations = 1;
inline constexpr auto kMaximumPluginInvocationTimeout = std::chrono::seconds(30);
inline constexpr auto kPluginCancellationGrace = std::chrono::milliseconds(500);

struct PluginInvokeRequest {
    std::string service;
    std::string payload;
    std::vector<std::string> required_permissions;
    bool user_initiated{};
    std::chrono::milliseconds timeout{};
    std::stop_token cancellation;
};

struct PluginInvokeResult {
    bool ok{};
    PluginStatus status{PluginStatus::plugin_error};
    bool forced_termination{};
    std::uint64_t request_id{};
    std::string payload;
    std::string diagnostic;
};

class PluginHostSession {
public:
    PluginHostSession();
    PluginHostSession(const PluginHostSession&) = delete;
    PluginHostSession& operator=(const PluginHostSession&) = delete;
    PluginHostSession(PluginHostSession&& other) noexcept;
    PluginHostSession& operator=(PluginHostSession&& other) noexcept;
    ~PluginHostSession();

    [[nodiscard]] bool valid() const noexcept;
    [[nodiscard]] std::string_view plugin_id() const noexcept;
    [[nodiscard]] std::string_view version() const noexcept;
    [[nodiscard]] unsigned long process_id() const noexcept;

    /// Performs one versioned service call from a non-TSF worker. P3C v1 permits one
    /// in-flight invocation per plugin process; a competing call fails immediately.
    [[nodiscard]] PluginInvokeResult invoke(PluginInvokeRequest request);

    /// Requests protocol shutdown and waits for both acknowledgement and process exit.
    /// Any protocol, timeout, or process failure terminates the entire plugin Job.
    [[nodiscard]] PluginHostOperationResult shutdown(std::chrono::milliseconds timeout);

    /// Terminates the entire plugin Job, serialized with active invocation cleanup.
    /// Repeated calls are harmless.
    void terminate() noexcept;

private:
    struct Impl;
    explicit PluginHostSession(std::unique_ptr<Impl> implementation);
    void terminate_unlocked() noexcept;
    std::unique_ptr<Impl> implementation_;

    friend struct PluginHostLaunchResult;
    friend PluginHostLaunchResult launch_active_plugin(
        const std::filesystem::path&, std::string_view, std::chrono::milliseconds);
};

struct PluginHostLaunchResult {
    PluginHostSession session;
    PluginManifest manifest;
    std::filesystem::path installed_path;
    std::filesystem::path data_path;
    std::string diagnostic;

    [[nodiscard]] explicit operator bool() const noexcept {
        return session.valid() && diagnostic.empty();
    }
};

/// Revalidates and launches the active installed process plugin in its zero-capability sandbox.
/// The plugin must send a valid hello_request before this call succeeds.
[[nodiscard]] PluginHostLaunchResult launch_active_plugin(
    const std::filesystem::path& plugin_store_root,
    std::string_view plugin_id,
    std::chrono::milliseconds startup_timeout);

}  // namespace owo::plugin

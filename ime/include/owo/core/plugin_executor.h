#pragma once

#include "owo/plugin/plugin_protocol.h"

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <future>
#include <memory>
#include <stop_token>
#include <string>
#include <vector>

namespace owo::core {

inline constexpr std::size_t kMaximumQueuedPluginCalls = 32;
inline constexpr std::uint32_t kMaximumPluginCallDepth = 8;

enum class PluginCallSource {
    tsf_input,
    core_background,
    // Internal attribution only. It is never decoded from Core IPC or command-line input.
    trusted_user_action,
    plugin_service,
};

enum class PluginExecutionStatus {
    success,
    invalid_request,
    rejected_source,
    queue_full,
    cancelled,
    timeout,
    launch_failed,
    invocation_failed,
    stopped,
};

struct PluginExecutionRequest {
    std::string plugin_id;
    std::string service;
    std::string payload;
    std::vector<std::string> required_permissions;
    PluginCallSource source{PluginCallSource::core_background};
    std::uint32_t call_depth{};
    std::chrono::milliseconds timeout{};
    std::stop_token cancellation;
};

struct PluginExecutionResult {
    bool ok{};
    std::uint64_t call_id{};
    PluginExecutionStatus status{PluginExecutionStatus::invocation_failed};
    plugin::PluginStatus plugin_status{plugin::PluginStatus::plugin_error};
    bool forced_termination{};
    std::string payload;
    std::string diagnostic;
};

struct PluginExecutionSubmission {
    std::uint64_t call_id{};
    std::future<PluginExecutionResult> completion;
};

/// Owns the Core Service's only plugin execution worker. submit() never launches or waits for a
/// plugin on the caller thread. TSF-originated calls are rejected even when permission-free.
class PluginExecutor {
public:
    explicit PluginExecutor(std::filesystem::path plugin_store_root);
    PluginExecutor(const PluginExecutor&) = delete;
    PluginExecutor& operator=(const PluginExecutor&) = delete;
    ~PluginExecutor();

    [[nodiscard]] PluginExecutionSubmission submit(PluginExecutionRequest request);
    [[nodiscard]] std::size_t queued_call_count() const;

private:
    struct Impl;
    std::unique_ptr<Impl> implementation_;
};

}  // namespace owo::core

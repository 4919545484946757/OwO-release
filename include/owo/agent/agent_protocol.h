#pragma once

#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace owo::agent {

inline constexpr std::uint32_t kAgentProtocolVersion = 1;
inline constexpr std::size_t kMaximumAgentPayloadBytes = 64U * 1024U;
inline constexpr std::size_t kMaximumAgentCommands = 64;
inline constexpr std::string_view kAgentPluginId = "org.owo.agent-ipc";
inline constexpr std::string_view kAgentPluginService = "owo.agent.exchange.v1";

enum class Action { submit, select, page, cancel, poll };
enum class Status {
    agent_mode,
    thinking,
    executing,
    completed,
    disconnected,
    error,
};

struct ApplicationContext {
    std::uint32_t process_id{};
    std::string executable;
    std::string window_class;
    bool sensitive_input{};
    bool operator==(const ApplicationContext&) const = default;
};

struct Request {
    Action action{Action::submit};
    std::string session_id;
    std::string user_input;
    ApplicationContext application;
    std::string session_context;
    std::string command_id;
    std::uint32_t page{};
    bool operator==(const Request&) const = default;
};

struct Command {
    std::string id;
    std::string label;
    bool high_risk{};
    bool operator==(const Command&) const = default;
};

struct Response {
    std::string session_id;
    std::string message;
    std::vector<Command> commands;
    std::string executing_command;
    Status status{Status::error};
    std::uint32_t page{};
    bool has_more{};
    std::string error_code;
    bool operator==(const Response&) const = default;
};

struct RequestDecodeResult {
    bool ok{};
    Request value;
    std::string diagnostic;
};

struct ResponseDecodeResult {
    bool ok{};
    Response value;
    std::string diagnostic;
};

struct FilterResult {
    bool ok{};
    std::string value;
    bool redacted{};
    std::string diagnostic;
};

[[nodiscard]] std::string encode_request(const Request& request);
[[nodiscard]] RequestDecodeResult decode_request(std::string_view json);
[[nodiscard]] std::string encode_response(const Response& response);
[[nodiscard]] ResponseDecodeResult decode_response(std::string_view json);
[[nodiscard]] FilterResult filter_sensitive_text(std::string_view utf8,
                                                 std::size_t maximum_bytes = 4096);
[[nodiscard]] const char* action_name(Action action) noexcept;
[[nodiscard]] const char* status_name(Status status) noexcept;

}  // namespace owo::agent

#pragma once

#include <chrono>
#include <string>
#include <string_view>

namespace owo::agent {

struct PipeExchangeResult {
    bool ok{};
    std::string response;
    std::string diagnostic;
};

[[nodiscard]] bool valid_agent_pipe_name(std::wstring_view pipe_name) noexcept;
[[nodiscard]] PipeExchangeResult exchange_agent_pipe(
    std::wstring_view pipe_name, std::string_view request,
    std::chrono::milliseconds timeout);

}  // namespace owo::agent

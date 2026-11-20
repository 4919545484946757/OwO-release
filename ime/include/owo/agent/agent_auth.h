#pragma once
#include <filesystem>
#include <string>
#include <string_view>
namespace owo::agent {
struct AuthenticatedRequest {
    bool ok{};
    std::string payload;
};
// Reads only a user-owned protected credential file; no legacy fallback on error.
[[nodiscard]] AuthenticatedRequest
authenticate_agent_request(std::string_view request, const std::filesystem::path &credential_file);
} // namespace owo::agent

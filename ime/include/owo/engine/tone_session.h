#pragma once
#include "owo/config/config_store.h"
#include "owo/engine/tone_feedback.h"
#include <filesystem>
#include <stop_token>

namespace owo::engine {
// All text stays in memory. No Agent task or tool invocation occurs here.
struct ToneGeneration {
    ToneDraftSelection selection;
    std::string diagnostic;
};
[[nodiscard]] bool tone_session_allowed(const config::AppConfig& config,
    std::string_view application, ToneDraftMode mode) noexcept;
[[nodiscard]] ToneGeneration generate_tone_drafts(const std::filesystem::path& root,
    const std::filesystem::path& config_path, std::string_view application,
    std::string_view message, ToneDraftMode mode, std::stop_token stop);
[[nodiscard]] bool tone_local_endpoint(std::wstring_view url) noexcept;
[[nodiscard]] std::string tone_response_content(std::string_view json);
[[nodiscard]] std::string tone_model_request(std::string_view model, std::string_view prompt);
}  // namespace owo::engine

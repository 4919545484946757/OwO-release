// Verifies that the assistive candidate switches in AppConfig actually reach
// the core service: each phase starts a fresh server with a config snapshot and
// checks which candidate families appear.
#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <atomic>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

std::wstring phase_pipe_name(const int phase) {
    const auto suffix =
        std::chrono::steady_clock::now().time_since_epoch().count();
    return LR"(\\.\pipe\OwO.InputMethod.ToggleTest.)" +
           std::to_wstring(phase) + L"." + std::to_wstring(suffix);
}

owo::protocol::DecodeResult send_request(const std::wstring& pipe_name,
                                         const owo::protocol::Message& request) {
    for (int attempt = 0; attempt < 100; ++attempt) {
        const auto result = owo::ipc::exchange(
            pipe_name.c_str(), owo::protocol::encode_message(request),
            std::chrono::milliseconds(100));
        if (result.status) return owo::protocol::decode_message(result.response);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return {};
}

owo::config::AppConfig toggles(const std::string_view disabled) {
    auto config = owo::config::AppConfig{};
    if (disabled == "smart") config.smart_candidates_enabled = false;
    if (disabled == "expressive") config.expressive_candidates_enabled = false;
    if (disabled == "concept") config.concept_candidates_enabled = false;
    return config;
}

struct Expectation {
    std::string input;
    std::vector<std::string> candidates;
};

int run_phase(const int phase, const std::string_view disabled,
              const owo::engine::BinaryLexicon& lexicon,
              const std::vector<Expectation>& expectations) {
    const auto pipe_name = phase_pipe_name(phase);
    const auto config_path = std::filesystem::temp_directory_path() /
                             ("owo-toggle-" + std::to_string(phase) + ".conf");
    {
        const auto text = owo::config::serialize_config(toggles(disabled));
        if (text.empty()) return fail("config serialization failed");
        std::ofstream output(config_path, std::ios::binary | std::ios::trunc);
        output.write(text.data(), static_cast<std::streamsize>(text.size()));
    }
    owo::config::ConfigMonitor monitor;
    const auto started = monitor.start(config_path);
    if (!started.success) return fail("config monitor failed to start");

    std::atomic<int> server_exit{-1};
    std::jthread server([&server_exit, &lexicon, &pipe_name, &monitor] {
        server_exit = owo::ipc::run_core_server(
            pipe_name.c_str(), lexicon, nullptr, nullptr, &monitor, nullptr,
            nullptr);
    });
    int result = 0;
    for (const auto& expectation : expectations) {
        const auto response = send_request(
            pipe_name, {owo::protocol::MessageType::candidate_request,
                        static_cast<std::uint64_t>(phase * 10 + 1), 1,
                        expectation.input});
        if (!response.validation ||
            response.message.candidates != expectation.candidates) {
            std::cerr << "phase " << phase << " input " << expectation.input
                      << " returned:";
            for (const auto& candidate : response.message.candidates)
                std::cerr << " [" << candidate << "]";
            std::cerr << '\n';
            result = fail("candidate toggle expectation failed");
            break;
        }
    }
    const auto shutdown = send_request(
        pipe_name,
        {owo::protocol::MessageType::shutdown_request,
         static_cast<std::uint64_t>(phase * 10 + 9), 1, ""});
    if (!shutdown.validation)
        result = fail("shutdown was not acknowledged");
    server.join();
    if (server_exit != 0 && result == 0)
        result = fail("core service exited with an error");
    std::error_code ignored;
    std::filesystem::remove(config_path, ignored);
    return result;
}

}  // namespace

int main() {
    const auto lexicon_path =
        std::filesystem::temp_directory_path() / "owo-toggle.owolx";
    const auto written = owo::engine::write_binary_lexicon(
        lexicon_path, {
                          {{"ni", "hao"}, "你好", 1000},
                          {{"lei", "si"}, "累死", 300},
                          {{"chong", "fu"}, "重复", 900},
                      });
    if (!written.success) return 2;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(lexicon_path).success) return 2;

    // Baseline: every family is on.
    if (run_phase(1, "", lexicon,
                  {{"1234", {"一千二百三十四", "壹仟贰佰叁拾肆", "1234"}},
                   {"leisi", {"累死", "累死😩", "(￣o￣) zZ", "😩"}},
                   {"chongfuzhixing", {"重复", "幂等"}}}) != 0)
        return 3;
    // Smart candidates off: literal stays, readings/date structures disappear,
    // expressive candidates keep working.
    if (run_phase(2, "smart", lexicon,
                  {{"1234", {"1234"}},
                   {"2026-10-19", {"2026-10-19"}},
                   {"leisi", {"累死", "累死😩", "(￣o￣) zZ", "😩"}}}) != 0)
        return 4;
    // Expressive off: only the dictionary decoding remains for leisi.
    if (run_phase(3, "expressive", lexicon,
                  {{"leisi", {"累死"}},
                   {"1234", {"一千二百三十四", "壹仟贰佰叁拾肆", "1234"}}}) != 0)
        return 5;
    // Concept off: the glossary candidate disappears, other families stay.
    if (run_phase(4, "concept", lexicon,
                  {{"chongfuzhixing", {"重复"}},
                   {"leisi", {"累死", "累死😩", "(￣o￣) zZ", "😩"}}}) != 0)
        return 6;

    std::error_code ignored;
    std::filesystem::remove(lexicon_path, ignored);
    std::cout << "candidate toggle tests passed\n";
    return 0;
}

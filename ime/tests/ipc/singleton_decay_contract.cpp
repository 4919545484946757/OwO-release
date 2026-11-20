#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <atomic>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <thread>

namespace {
owo::protocol::DecodeResult send(const std::wstring &pipe, const owo::protocol::Message &request) {
    const auto exchanged = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
                                              std::chrono::seconds(2));
    return exchanged.status ? owo::protocol::decode_message(exchanged.response)
                            : owo::protocol::DecodeResult{};
}
bool first_is(const owo::protocol::DecodeResult &result, const std::string_view expected) {
    return result.validation && !result.message.candidates.empty() &&
           result.message.candidates.front() == expected;
}
} // namespace
int main() {
    const auto nonce = std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / ("owo-singleton-decay-" + nonce);
    std::filesystem::create_directories(root);
    owo::config::ConfigStore writer;
    if (!writer.load(root / "owo.conf").success)
        return 1;
    auto config = writer.snapshot();
    config.user_learning_enabled = true;
    config.model_ranking_enabled = false;
    config.candidate_partition_enabled = true;
    config.candidate_page_size = 7;
    if (!writer.save(config).success)
        return 2;
    owo::config::ConfigMonitor monitor;
    if (!monitor.start(root / "owo.conf").success)
        return 3;
    std::atomic<std::uint64_t> now{1'000'000};
    owo::engine::UserFrequencyStore frequency([&] { return now.load(); });
    if (!frequency.load(root / "frequency.owuf").success)
        return 4;
    frequency.record("nihao", "你号");
    const owo::engine::MemoryLexicon lexicon(
        {{{"ni", "hao"}, "你好", 50000}, {{"ni", "hao"}, "你号", 1}, {{"ni"}, "你", 10000}});
    const auto suffix = std::wstring(nonce.begin(), nonce.end());
    const auto pipe = LR"(\\.\pipe\OwO.Test.SingletonDecay.)" + suffix;
    std::thread core([&] {
        static_cast<void>(
            owo::ipc::run_core_server(pipe.c_str(), lexicon, &frequency, nullptr, &monitor));
    });
    owo::protocol::Message query{owo::protocol::MessageType::candidate_request, 1, 90210, "nihao"};
    const auto fresh = send(pipe, query);
    now.store(1'000'060);
    query.request_id++;
    const auto expired = send(pipe, query); // Identical input must not reuse the fresh cached pool.
    owo::protocol::Message feedback{owo::protocol::MessageType::candidate_committed, 3, 90210,
                                    "你号"};
    feedback.input = "nihao";
    const auto acknowledged = send(pipe, feedback);
    query.request_id = 4;
    const auto repeated = send(pipe, query);
    now.store(2'000'000);
    query.request_id++;
    const auto stable = send(pipe, query);
    owo::protocol::Message stop;
    stop.type = owo::protocol::MessageType::shutdown_request;
    static_cast<void>(send(pipe, stop));
    core.join();
    monitor.stop();
    std::filesystem::remove_all(root);
    if (!first_is(fresh, "你号") || !first_is(expired, "你好") || !acknowledged.validation ||
        !first_is(repeated, "你号") || !first_is(stable, "你号")) {
        std::cerr << "Singleton decay/cache/reconfirmation contract failed\n";
        return 5;
    }
    std::cout << "Expired singleton invalidates cached priority; repeated learning stays stable\n";
}

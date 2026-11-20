#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/model/model_backend.h"
#include "owo/model/model_protocol.h"
#include "owo/protocol/messages.h"

#include <algorithm>
#include <atomic>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <thread>

namespace {
using namespace owo;
class ReverseRanker final : public model::IModelBackend {
  public:
    std::string_view id() const noexcept override { return "owo.test.learning.reverse"; }
    model::ModelResult rank(const model::ModelRequest &request, std::stop_token) override {
        auto result = request.candidates;
        std::reverse(result.begin(), result.end());
        return {request.request_id, model::ModelStatus::success, std::move(result), {}};
    }
};
protocol::DecodeResult send(const std::wstring &pipe, const protocol::Message &message) {
    const auto result =
        ipc::exchange(pipe.c_str(), protocol::encode_message(message), std::chrono::seconds(2));
    return result.status ? protocol::decode_message(result.response) : protocol::DecodeResult{};
}
} // namespace

int main() {
    const auto nonce = std::to_wstring(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / (L"owo-learning-contract-" + nonce);
    std::filesystem::create_directories(root);
    config::ConfigStore writer;
    if (!writer.load(root / "owo.conf").success)
        return 1;
    auto settings = writer.snapshot();
    settings.user_learning_enabled = true;
    settings.user_learning_sensitivity = 10;
    settings.model_ranking_enabled = true;
    settings.model_timeout_ms = 500;
    settings.candidate_partition_enabled = true;
    settings.candidate_page_size = 7;
    if (!writer.save(settings).success)
        return 2;
    config::ConfigMonitor monitor;
    if (!monitor.start(root / "owo.conf", std::chrono::milliseconds(10)).success)
        return 3;
    std::atomic<std::uint64_t> now{1'000'000};
    engine::UserFrequencyStore frequency([&] { return now.load(); });
    const auto frequency_path = root / "frequency.owuf";
    if (!frequency.load(frequency_path).success)
        return 4;
    frequency.record("dongkeyi", "董柯伊");
    const engine::MemoryLexicon lexicon({{{"ke", "yi"}, "可以", 1000000},
                                         {{"ke", "yi"}, "刻意", 1},
                                         {{"ke", "yi"}, "可疑", 100000},
                                         {{"ke"}, "可", 100000},
                                         {{"ke"}, "刻", 1000},
                                         {{"ke"}, "克", 100},
                                         {{"yi"}, "以", 10000},
                                         {{"yi"}, "意", 1000},
                                         {{"yi"}, "伊", 10},
                                         {{"dong"}, "动", 1000000},
                                         {{"dong"}, "东", 900000},
                                         {{"dong"}, "董", 10},
                                         {{"ke"}, "柯", 10},
                                         {{"dong", "ke"}, "冬科", 100000},
                                         {{"dong", "ke"}, "鮗科", 90000}});
    const auto core_pipe = LR"(\\.\pipe\OwO.Test.Learning.Core.)" + nonce;
    const auto model_pipe = LR"(\\.\pipe\OwO.Test.Learning.Model.)" + nonce;
    ReverseRanker backend;
    std::thread model(
        [&] { static_cast<void>(ipc::run_model_server(model_pipe.c_str(), backend)); });
    std::thread core([&] {
        static_cast<void>(ipc::run_core_server(core_pipe.c_str(), lexicon, &frequency,
                                               model_pipe.c_str(), &monitor));
    });
    bool ok = true;
    std::uint64_t id = 1;
    const auto check = [&](bool value, std::string_view detail) {
        if (!value) {
            ok = false;
            std::cerr << detail << '\n';
        }
    };
    const auto learn = [&](std::string input, std::string text) {
        protocol::Message request{protocol::MessageType::candidate_committed, ++id, 888,
                                  std::move(text)};
        request.input = std::move(input);
        const auto response = send(core_pipe, request);
        check(response.validation &&
                  response.message.type == protocol::MessageType::acknowledgement,
              "Learning feedback was not acknowledged");
    };
    const auto query = [&](std::string input, bool expanded, bool update) {
        protocol::Message request{protocol::MessageType::candidate_request, ++id, 888,
                                  std::move(input)};
        request.expanded = expanded;
        auto response = send(core_pipe, request);
        check(response.validation && !response.message.candidates.empty(),
              "Candidate request failed");
        if (update && response.message.model_pending) {
            request = {protocol::MessageType::candidate_update_request, id, 888};
            auto ranked = send(core_pipe, request);
            check(static_cast<bool>(ranked.validation), "Model update failed");
            if (!ranked.message.candidates.empty())
                response = std::move(ranked);
        }
        return response;
    };
    check(query("keyi", false, false).message.candidates.front() == "可以", "Baseline changed");
    learn("keyi", "刻意");
    for (const bool expanded : {false, true})
        for (const bool ranked : {false, true})
            check(query("keyi", expanded, ranked).message.candidates.front() == "刻意",
                  "Exact-word learning was overridden");
    learn("keyi", "刻意");
    now.fetch_add(60);
    check(query("keyi", false, true).message.candidates.front() == "刻意",
          "Repeated learning expired like a singleton");
    learn("ke", "克");
    learn("ke", "克");
    for (const bool expanded : {false, true}) {
        for (const bool ranked : {false, true}) {
            const auto response = query("keyi", expanded, ranked);
            const auto &message = response.message;
            const auto first_single =
                std::find(message.candidate_groups.begin(), message.candidate_groups.end(), 2);
            const auto index =
                static_cast<std::size_t>(first_single - message.candidate_groups.begin());
            check(index < message.candidates.size() && message.candidates[index] == "克",
                  "Prefix-character learning was overwritten by the dictionary-head layout");
        }
    }
    learn("ky", "刻意");
    learn("ky", "刻意");
    check(query("ky", false, false).message.candidates.front() == "刻意",
          "Abbreviation learning did not affect ranking");
    check(query("k", false, false).message.candidates.front() == "可",
          "A different source encoding incorrectly inherited strong learned priority");
    const auto retains_name = [&](const protocol::DecodeResult &response, std::size_t consumed) {
        const auto &value = response.message;
        const auto found = std::find(value.candidates.begin(), value.candidates.end(), "董柯伊");
        const auto index = static_cast<std::size_t>(found - value.candidates.begin());
        return response.validation && found != value.candidates.end() &&
               index < value.candidate_consumed.size() &&
               value.candidate_consumed[index] == consumed;
    };
    for (const bool confirmed : {false, true}) {
        if (confirmed)
            learn("dongkeyi", "董柯伊");
        // Match the TSF fast request followed by refinement with the SAME ID.
        for (const auto input :
             {"d", "do", "don", "dong", "dongk", "dongke", "dongkey", "dongkeyi"}) {
            protocol::Message request{protocol::MessageType::candidate_request, ++id, 888, input};
            for (const bool deferred : {true, false}) {
                request.defer_expensive_candidates = deferred;
                const auto result = send(core_pipe, request);
                check(result.validation && !result.message.candidates.empty(),
                      "Incremental user-word query failed");
                if (std::string_view(input) == "dongkey" || std::string_view(input) == "dongkeyi") {
                    check(retains_name(result, std::string_view(input).size()),
                          "User word disappeared during fast/refined incremental input");
                    if (confirmed && std::string_view(input) == "dongkeyi")
                        check(result.message.candidates.front() == "董柯伊",
                              "Confirmed full user word lost first place");
                }
            }
        }
        for (const auto input : {"dongkey", "dongkeyi"}) {
            for (const bool expanded : {false, true})
                for (const bool ranked : {false, true})
                    check(retains_name(query(input, expanded, ranked),
                                       std::string_view(input).size()),
                          "User word disappeared in the expanded/model candidate list");
        }
    }
    const auto generation = monitor.generation();
    settings.model_ranking_enabled = false;
    settings.user_learning_enabled = false;
    check(writer.save(settings).success &&
              monitor.wait_for_generation(generation, std::chrono::seconds(2)),
          "Learning/ranking configuration did not reload");
    learn("keyi", "刻意"); // Acknowledged, but the disabled learning switch must not record it.
    check(query("keyi", false, false).message.candidates.front() == "刻意" &&
              query("ke", false, false).message.candidates.front() == "克" &&
              query("ky", false, false).message.candidates.front() == "刻意",
          "Existing learning depends on external model availability");
    protocol::Message stop;
    stop.type = protocol::MessageType::shutdown_request;
    static_cast<void>(send(core_pipe, stop));
    core.join();
    static_cast<void>(
        ipc::exchange(model_pipe.c_str(),
                      model::encode_model_message({model::ModelMessageType::shutdown_request}),
                      std::chrono::seconds(2)));
    model.join();
    engine::UserFrequencyStore reloaded([&] { return now.load(); });
    check(reloaded.load(frequency_path).success && reloaded.contextual_count("keyi", "刻意") == 2 &&
              reloaded.contextual_count("ke", "克") == 2 &&
              reloaded.contextual_count("ky", "刻意") == 2,
          "Acknowledged learning was lost after orderly shutdown/reload");
    monitor.stop();
    std::filesystem::remove_all(root);
    return ok ? 0 : 5;
}

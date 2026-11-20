#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/binary_lexicon.h"
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
class WeakFirstModel final : public owo::model::IModelBackend {
  public:
    std::atomic<bool> saw_weak{false};
    std::string_view id() const noexcept override { return "owo.test.weak-first.v1"; }
    owo::model::ModelResult rank(const owo::model::ModelRequest &request,
                                 std::stop_token) override {
        auto ranked = request.candidates;
        const auto weak = std::find(ranked.begin(), ranked.end(), "朽腐类似问题");
        if (weak != ranked.end()) {
            saw_weak.store(true);
            std::rotate(ranked.begin(), weak, weak + 1);
        }
        return {request.request_id, owo::model::ModelStatus::success, std::move(ranked), {}};
    }
};
owo::protocol::DecodeResult send(const std::wstring &pipe, const owo::protocol::Message &request) {
    const auto exchanged = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
                                              std::chrono::seconds(2));
    return exchanged.status ? owo::protocol::decode_message(exchanged.response)
                            : owo::protocol::DecodeResult{};
}
} // namespace

int main(int argc, char **argv) {
    if (argc != 1 && argc != 4 && argc != 5)
        return 2;
    const auto nonce = std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / ("owo-model-confidence-" + nonce);
    std::filesystem::create_directories(root);
    const auto config_path = root / "owo.conf";
    owo::config::ConfigStore writer;
    if (!writer.load(config_path).success)
        return 1;
    auto config = writer.snapshot();
    config.candidate_page_size = 7;
    config.model_ranking_enabled = true;
    config.model_timeout_ms = 500;
    config.user_learning_enabled = true;
    if (!writer.save(config).success)
        return 2;
    owo::config::ConfigMonitor monitor;
    if (!monitor.start(config_path).success)
        return 3;
    const owo::engine::MemoryLexicon lexicon({{{"xiu", "fu"}, "修复", 10000000},
                                              {{"xiu", "fu"}, "朽腐", 1},
                                              {{"lei", "si", "wen", "ti"}, "类似问题", 50000},
                                              {{"xiu"}, "修", 10000},
                                              {{"xiu"}, "朽", 1000},
                                              {{"xiu"}, "秀", 800}});
    const auto suffix = std::wstring(nonce.begin(), nonce.end());
    const auto core_pipe = LR"(\\.\pipe\OwO.Test.ModelConfidence.Core.)" + suffix;
    const auto model_pipe = LR"(\\.\pipe\OwO.Test.ModelConfidence.Model.)" + suffix;
    WeakFirstModel backend;
    owo::engine::BinaryLexicon actual_lexicon;
    owo::model::LibimeBackendLoadResult actual_backend;
    if (argc >= 4) {
        if (!actual_lexicon.load(argv[1]).success)
            return 5;
        actual_backend = owo::model::load_libime_backend(argv[2], argv[3]);
        if (!actual_backend)
            return 6;
    }
    auto &selected_model =
        argc >= 4 ? *actual_backend.backend : static_cast<owo::model::IModelBackend &>(backend);
    const auto &selected_lexicon = argc >= 4
                                       ? static_cast<const owo::engine::Lexicon &>(actual_lexicon)
                                       : static_cast<const owo::engine::Lexicon &>(lexicon);
    const auto frequency_path = root / "frequency.owuf";
    if (argc == 5)
        std::filesystem::copy_file(argv[4], frequency_path);
    owo::engine::UserFrequencyStore frequency;
    if (!frequency.load(frequency_path).success)
        return 7;
    std::thread model(
        [&] { static_cast<void>(owo::ipc::run_model_server(model_pipe.c_str(), selected_model)); });
    std::thread core([&] {
        static_cast<void>(owo::ipc::run_core_server(core_pipe.c_str(), selected_lexicon, &frequency,
                                                    model_pipe.c_str(), &monitor));
    });
    bool success = true;
    std::uint64_t id = 10;
    for (const bool learned : {false, true}) {
        if (learned) {
            owo::protocol::Message feedback{owo::protocol::MessageType::candidate_committed, ++id,
                                            91234, "修复类似问题"};
            feedback.input = "xiufuleisiwenti";
            const auto acknowledged = send(core_pipe, feedback);
            success = success && acknowledged.validation &&
                      acknowledged.message.type == owo::protocol::MessageType::acknowledgement;
        }
        for (const bool expanded : {false, true}) {
            owo::protocol::Message request{owo::protocol::MessageType::candidate_request, ++id,
                                           91234, "xiufuleisiwenti"};
            request.expanded = expanded;
            const auto base = send(core_pipe, request);
            success = success && base.validation && !base.message.candidates.empty() &&
                      base.message.candidates.front() == "修复类似问题" &&
                      base.message.model_pending;
            request = {owo::protocol::MessageType::candidate_update_request, id, 91234};
            auto ranked = send(core_pipe, request);
            for (unsigned attempt = 0;
                 ranked.validation && ranked.message.model_pending && attempt < 20; ++attempt)
                ranked = send(core_pipe, request);
            success = success && ranked.validation && !ranked.message.model_pending &&
                      !ranked.message.candidates.empty() &&
                      ranked.message.candidates.front() == "修复类似问题" &&
                      ranked.message.candidate_groups == base.message.candidate_groups &&
                      ranked.message.candidates.size() == base.message.candidates.size();
            // Reordering inside the dictionary area may legitimately exchange
            // a short prefix for a hidden whole word. Validate consumption by
            // candidate identity, not by its old screen slot.
            for (std::size_t index = 0; index < ranked.message.candidates.size(); ++index) {
                const auto found =
                    std::find(base.message.candidates.begin(), base.message.candidates.end(),
                              ranked.message.candidates[index]);
                const auto expected =
                    found == base.message.candidates.end()
                        ? std::uint64_t{15}
                        : base.message.candidate_consumed[static_cast<std::size_t>(
                              found - base.message.candidates.begin())];
                success = success && index < ranked.message.candidate_consumed.size() &&
                          ranked.message.candidate_consumed[index] == expected;
            }
            if (!success) {
                std::cerr << "base=" << owo::protocol::encode_message(base.message) << '\n'
                          << "ranked=" << owo::protocol::encode_message(ranked.message) << '\n';
            }
        }
    }
    owo::protocol::Message stop;
    stop.type = owo::protocol::MessageType::shutdown_request;
    static_cast<void>(send(core_pipe, stop));
    core.join();
    owo::model::ModelMessage model_stop;
    model_stop.type = owo::model::ModelMessageType::shutdown_request;
    static_cast<void>(owo::ipc::exchange(
        model_pipe.c_str(), owo::model::encode_model_message(model_stop), std::chrono::seconds(2)));
    model.join();
    monitor.stop();
    std::filesystem::remove_all(root);
    if (!success || (argc == 1 && !backend.saw_weak.load())) {
        std::cerr << "Model ranking discarded strong lexical confidence or page layout\n";
        return 4;
    }
    std::cout << "Model confidence survives adversarial ranking in paged and expanded views\n";
}

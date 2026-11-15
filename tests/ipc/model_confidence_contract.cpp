#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/engine/ranking_context.h"
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
    std::atomic<bool> saw_number{false};
    std::atomic<bool> saw_unmasked_number{false};
    std::string_view id() const noexcept override { return "owo.test.weak-first.v1"; }
    owo::model::ModelResult rank(const owo::model::ModelRequest &request,
                                 std::stop_token) override {
        auto ranked = request.candidates;
        if (request.input == "hao" || request.input == "lou" || request.input == "ye") {
            if (owo::engine::ranking_numbers_need_redaction(request.context)) saw_unmasked_number.store(true);
            if (request.context == "0") saw_number.store(true);
            const auto ordinary = request.input == "hao" ? "好" : request.input == "lou" ? "漏" : "也";
            if (const auto found = std::find(ranked.begin(), ranked.end(), ordinary); found != ranked.end())
                std::rotate(ranked.begin(), found, found + 1);
        }
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
    const bool numeric_only = argc == 5 && std::string_view(argv[4]) == "--numeric-only";
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
                                              {{"xiu"}, "秀", 800},
                                              {{"hao"}, "好", 100000}, {{"hao"}, "号", 100},
                                              {{"lou"}, "漏", 100000}, {{"lou"}, "楼", 100},
                                              {{"ye"}, "也", 100000}, {{"ye"}, "页", 100}});
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
    if (argc == 5 && !numeric_only)
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
        if (numeric_only) break;
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
                      base.message.candidates.front() == "修复类似问题";
            request = {owo::protocol::MessageType::candidate_update_request, id, 91234};
            auto ranked = base.message.model_pending ? send(core_pipe, request) : base;
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
    const auto query = [&](std::string input, std::string context, std::string_view expected,
                           bool expanded = false, bool symbols = false) {
        owo::protocol::Message request{owo::protocol::MessageType::candidate_request, ++id, 91235, std::move(input)};
        request.context = std::move(context);
        request.expanded = expanded;
        request.symbol_composition = symbols;
        const auto base = send(core_pipe, request);
        owo::protocol::Message update{owo::protocol::MessageType::candidate_update_request, id, 91235};
        auto ranked = base.message.model_pending ? send(core_pipe, update) : base;
        for (unsigned attempt = 0; ranked.validation && ranked.message.model_pending && attempt < 20; ++attempt)
            ranked = send(core_pipe, update);
        const bool matched = base.validation && !base.message.candidates.empty() &&
            base.message.candidates.front() == expected && ranked.validation && !ranked.message.model_pending &&
            !ranked.message.candidates.empty() && ranked.message.candidates.front() == expected;
        if (!matched) {
            std::cerr << "Numeric context did not preserve " << expected << ": base="
                      << owo::protocol::encode_message(base.message) << " ranked="
                      << owo::protocol::encode_message(ranked.message) << '\n';
            success = false;
        }
    };
    if (argc == 1) {
        query("hao", "", "好");
        query("hao", "987654。", "好");
        query("hao", "987654", "号");
        query("hao", "１２３", "号", true);
        query("lou", "⑥", "楼");
        query("ye", "٣٥", "页");
        query("１２hao", "", "１２号", false, true);
        if (!backend.saw_number.load() || backend.saw_unmasked_number.load()) success = false;
        config.model_ranking_enabled = false;
        if (!writer.save(config).success) success = false;
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
        while (monitor.snapshot()->model_ranking_enabled && std::chrono::steady_clock::now() < deadline)
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        query("hao", "987654", "好");
    } else {
        query("hao", "987654", "号");
        query("hao", "１２３", "号", true);
        query("lou", "⑥", "楼");
        query("ye", "٣٥", "页");
        query("１２hao", "", "１２号", false, true);
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
    std::cout << (numeric_only ? "Numeric context ranks the expected suffixes with the real lexicon and libime\n" :
        "Model confidence survives adversarial ranking in paged and expanded views\n");
}

#include "owo/config/config_monitor.h"
#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"
#include "owo/model/model_protocol.h"
#include <Windows.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <thread>

class SymbolRanker final : public owo::model::IModelBackend {
public:
    std::atomic<bool> received_symbols{false};
    std::atomic<int> delay_ms{0};
    std::atomic<int> delayed_calls{0};
    std::string_view id() const noexcept override { return "owo.test.symbols"; }
    owo::model::ModelResult rank(const owo::model::ModelRequest& request, std::stop_token) override {
        if (request.input == "shijie" && delay_ms > 0) {
            ++delayed_calls;
            std::this_thread::sleep_for(std::chrono::milliseconds(delay_ms.load()));
        }
        if (request.input != "nihao" && request.input != "shijie" && request.input != "nihao'shijie") received_symbols = true;
        for (const auto& candidate : request.candidates)
            if (candidate.find('\'') != std::string::npos || candidate.find(',') != std::string::npos)
                received_symbols = true;
        auto choices = request.candidates;
        std::reverse(choices.begin(), choices.end());
        return {request.request_id, owo::model::ModelStatus::success, std::move(choices), {}};
    }
};

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    using namespace owo;
    const auto root = std::filesystem::path(argv[1]);
    std::filesystem::create_directories(root);
    config::ConfigStore writer;
    if (!writer.load(root / "owo.conf").success) return 2;
    config::AppConfig settings;
    settings.pinyin_symbol_compatibility_enabled = true;
    settings.model_ranking_enabled = false;
    if (!writer.save(settings).success) return 2;
    config::ConfigMonitor monitor;
    if (!monitor.start(root / "owo.conf", std::chrono::milliseconds(10)).success) return 2;
    engine::MemoryLexicon lexicon({
        {{"ni", "hao"}, "你好", 10000}, {{"ni", "hao"}, "拟好", 9000},
        {{"yi", "er"}, "一二", 100000}, {{"yi"}, "一", 1000000}, {{"er"}, "二", 1000000},
        {{"shi", "jie"}, "世界", 10000}, {{"ni"}, "你", 9000},
        {{"shi", "jie"}, "视界", 9000}, {{"shi", "jie"}, "时节", 8000},
        {{"shi", "jie"}, "使节", 7000}, {{"shi", "jie"}, "十届", 6000},
        {{"shi", "jie"}, "师姐", 5000}, {{"shi", "jie"}, "石阶", 4000},
        {{"shi", "jie"}, "失节", 3000}, {{"shi", "jie"}, "诗界", 2000},
        {{"shi"}, "是", 90000}, {{"shi"}, "时", 80000}, {{"shi"}, "事", 70000},
        {{"shi"}, "市", 60000}, {{"shi"}, "十", 50000}, {{"shi"}, "使", 40000},
        {{"wo"}, "我", 10000}, {{"hao"}, "好", 8000}});
    const auto pipe = LR"(\\.\pipe\OwO.SymbolContract.)" + std::to_wstring(GetCurrentProcessId());
    const auto model_pipe = pipe + L".model";
    SymbolRanker backend;
    std::jthread model_server([&] { static_cast<void>(ipc::run_model_server(model_pipe.c_str(), backend)); });
    int server_exit = -1, failures = 0;
    std::jthread server([&] { server_exit = ipc::run_core_server(pipe.c_str(), lexicon, nullptr, model_pipe.c_str(), &monitor); });
    std::uint64_t id = 0;
    auto send = [&](protocol::Message message) {
        if (message.request_id == 0) message.request_id = ++id;
        message.context_generation = 1;
        for (int retry = 0; retry < 40; ++retry) {
            const auto exchange = ipc::exchange(pipe.c_str(), protocol::encode_message(message), std::chrono::milliseconds(300));
            if (exchange.status) return protocol::decode_message(exchange.response);
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        }
        return protocol::DecodeResult{};
    };
    auto check = [&](bool value, const char* name) { if (!value) { std::cerr << name << '\n'; ++failures; } };
    auto query = [&](std::string text, bool expanded = false, std::vector<std::uint64_t> manual = {}) {
        protocol::Message request{protocol::MessageType::candidate_request};
        request.text = std::move(text); request.symbol_composition = true;
        request.expanded = expanded; request.manual_breaks = std::move(manual);
        return send(std::move(request));
    };
    auto first = query("nihao，shijie");
    check(first.validation && !first.message.candidates.empty() && first.message.candidates[0] == "你好，" &&
        first.message.candidate_consumed[0] == 8 && first.message.following_previews.size() == 1,
        "strong first segment + following row");
    const auto other = std::find(first.message.candidates.begin(), first.message.candidates.end(), "拟好，");
    check(other != first.message.candidates.end() &&
        first.message.candidate_consumed[static_cast<std::size_t>(other - first.message.candidates.begin())] == 8,
        "nonfirst candidate retains delimiter in commit payload");
    auto expanded = query("nihao，shijie", true);
    check(expanded.validation && expanded.message.following_previews.empty() &&
        expanded.message.symbol_preview == "ni'hao，" && expanded.message.candidates[0] == first.message.candidates[0],
        "expanded active segment only");
    auto next = query("shijie");
    check(next.validation && next.message.candidates[0] == "世界", "FIFO promoted segment");
    check(first.message.symbol_preview == "ni'hao，shi'jie" &&
        first.message.following_previews.front() == next.message.candidates &&
        first.message.following_groups.front() == next.message.candidate_groups,
        "following segmentation and complete canonical first page");
    for (const auto size : {5U, 7U, 9U}) for (const auto partition : {false, true}) {
        const auto before = monitor.generation();
        settings.candidate_page_size = size;
        settings.candidate_partition_enabled = partition;
        check(writer.save(settings).success && monitor.wait_for_generation(before, std::chrono::seconds(2)), "page layout reload");
        const auto multiple = query("nihao，shijie；nihao");
        const auto actual = query("shijie；nihao");
        check(multiple.validation && actual.validation && multiple.message.following_previews.size() == 1 &&
            multiple.message.following_previews[0] == actual.message.candidates &&
            multiple.message.following_groups[0] == actual.message.candidate_groups &&
            multiple.message.following_previews[0].size() > 3 && multiple.message.following_previews[0].size() <= size &&
            multiple.message.symbol_preview == "ni'hao，shi'jie；ni'hao", "page size/partition/segmentation equality");
        for (const auto separator : {"、", ";", ":", "!", "?", "；", "：", "！", "？"}) {
            const auto raw = std::string("yi") + separator + "er";
            const auto inline_choice = query(raw);
            check(inline_choice.validation && !inline_choice.message.candidates.empty() &&
                inline_choice.message.candidates.front() == std::string("一") + separator + "二" &&
                inline_choice.message.candidate_consumed.front() == raw.size() && inline_choice.message.following_previews.empty(),
                "short strong-symbol segments become one selectable candidate, not adjacent candidate blocks");
            const auto all = query(raw, true);
            check(all.validation && all.message.candidates.front() == inline_choice.message.candidates.front() &&
                all.message.following_previews.empty(), "expanded page retains combined candidate payload");
        }
    }
    auto quoted = query("wo“nihao”");
    check(quoted.validation && quoted.message.symbol_preview == "wo“ni'hao”" &&
        !quoted.message.candidates.empty() && quoted.message.candidates[0].find("“") != std::string::npos,
        "weak quote candidate projection");
    auto generation = monitor.generation();
    settings.separator_shortcuts = {"Ctrl+F7"};
    check(writer.save(settings).success && monitor.wait_for_generation(generation, std::chrono::seconds(2)), "remap reload");
    auto weak = query("'nihao'");
    check(weak.validation && !weak.message.candidates.empty() && weak.message.candidates[0] == "'你好'", "unbound ASCII apostrophe is literal weak quote");
    auto manual = query("ni'hao", false, {2});
    check(manual.validation && !manual.message.candidates.empty() && manual.message.candidates[0] == "你好", "remapped shortcut remains syntax");
    auto literals = query("，；");
    check(literals.validation && literals.message.candidates == std::vector<std::string>{"，；"} &&
        literals.message.candidate_consumed == std::vector<std::uint64_t>{6}, "literal-only safe consumption");
    generation = monitor.generation();
    settings.model_ranking_enabled = true;
    settings.model_timeout_ms = 500;
    check(writer.save(settings).success && monitor.wait_for_generation(generation, std::chrono::seconds(2)), "model config reload");
    const auto ranked_base = query("'nihao',shijie");
    check(ranked_base.validation && ranked_base.message.model_pending, "asynchronous model request");
    protocol::Message update_request{protocol::MessageType::candidate_update_request,
        ranked_base.message.request_id, 1};
    protocol::DecodeResult update;
    for (int attempt = 0; attempt < 10; ++attempt) {
        update = send(update_request);
        if (!update.message.model_pending) break;
    }
    check(update.validation && !update.message.candidates.empty() && !backend.received_symbols &&
        update.message.candidates.front().starts_with("'") && update.message.candidates.front().ends_with("',") &&
        update.message.candidate_consumed.front() == 8, "model sees native candidates; update retains raw symbol offsets");
    const auto ranked_expanded = query("'nihao',shijie", true);
    check(ranked_expanded.validation && !ranked_expanded.message.candidates.empty() &&
        ranked_expanded.message.candidates.front() == update.message.candidates.front(), "model/paged/expanded same snapshot");
    const auto ranked_next = query("shijie");
    check(ranked_next.validation && ranked_base.message.following_previews[0] == ranked_next.message.candidates &&
        ranked_base.message.following_groups[0] == ranked_next.message.candidate_groups,
        "following model ranking equals actual first page");
    const auto combined_base = query("nihao;shijie");
    protocol::Message combined_update{protocol::MessageType::candidate_update_request, combined_base.message.request_id, 1};
    protocol::DecodeResult combined_ranked;
    for (int attempt = 0; attempt < 10; ++attempt) {
        combined_ranked = send(combined_update);
        if (!combined_ranked.message.model_pending) break;
    }
    check(combined_base.validation && combined_ranked.validation && !backend.received_symbols &&
        combined_ranked.message.following_previews.empty() &&
        std::any_of(combined_ranked.message.candidates.begin(), combined_ranked.message.candidates.end(),
            [](const auto& candidate) { return candidate.find(';') != std::string::npos && !candidate.ends_with(';'); }),
        "model update retains an internal separator in one candidate without exposing symbols to model");
    generation = monitor.generation();
    settings.candidate_page_size = 7;
    check(writer.save(settings).success && monitor.wait_for_generation(generation, std::chrono::seconds(2)), "slow page config reload");
    backend.delay_ms = 35;
    protocol::Message slow_request{protocol::MessageType::candidate_request};
    slow_request.text = "nihao，shijie，nihao";
    slow_request.symbol_composition = true;
    const auto partial_rows = send(slow_request);
    check(partial_rows.validation && partial_rows.message.candidate_refinement_pending &&
        partial_rows.message.following_previews.size() == 2 && !partial_rows.message.following_previews[0].empty() &&
        partial_rows.message.following_previews[1].empty() && !partial_rows.message.candidates.empty(),
        "slow model is sliced without inventing reduced candidate rows");
    slow_request.request_id = partial_rows.message.request_id;
    const auto completed_rows = send(slow_request);
    check(completed_rows.validation && !completed_rows.message.candidate_refinement_pending &&
        completed_rows.message.following_previews.size() == 2 && !completed_rows.message.following_previews[1].empty() &&
        completed_rows.message.following_previews[0] == partial_rows.message.following_previews[0] && backend.delayed_calls == 1,
        "refinement reuses completed pages and finishes remaining rows");
    send({protocol::MessageType::shutdown_request});
    server.join();
    model::ModelMessage stop;
    stop.type = model::ModelMessageType::shutdown_request;
    static_cast<void>(ipc::exchange(model_pipe.c_str(), model::encode_model_message(stop), std::chrono::seconds(1)));
    model_server.join();
    return failures || server_exit != 0 ? 1 : 0;
}

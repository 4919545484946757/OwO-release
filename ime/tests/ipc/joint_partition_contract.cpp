#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/model/model_backend.h"
#include "owo/model/model_protocol.h"
#include "owo/protocol/messages.h"

#include <algorithm>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <thread>

namespace {
class PrefixFirst final : public owo::model::IModelBackend {
  public:
    std::string_view id() const noexcept override { return "owo.test.joint-prefix"; }
    owo::model::ModelResult rank(const owo::model::ModelRequest &request,
                                 std::stop_token) override {
        auto result = request.candidates;
        const auto prefix = std::find(result.begin(), result.end(), "自动站");
        if (prefix != result.end())
            std::rotate(result.begin(), prefix, prefix + 1);
        return {request.request_id, owo::model::ModelStatus::success, std::move(result), {}};
    }
};
owo::protocol::DecodeResult send(const std::wstring &pipe, const owo::protocol::Message &request) {
    const auto reply = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
                                          std::chrono::seconds(2));
    return reply.status ? owo::protocol::decode_message(reply.response)
                        : owo::protocol::DecodeResult{};
}
} // namespace

int main() {
    const auto nonce = std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / ("owo-joint-partition-" + nonce);
    std::filesystem::create_directories(root);
    owo::config::ConfigStore writer;
    const auto config_path = root / "owo.conf";
    if (!writer.load(config_path).success)
        return 2;
    auto config = writer.snapshot();
    config.model_ranking_enabled = true;
    config.model_timeout_ms = 500;
    config.user_learning_enabled = false;
    if (!writer.save(config).success)
        return 2;
    owo::config::ConfigMonitor monitor;
    if (!monitor.start(config_path, std::chrono::milliseconds(10)).success)
        return 2;
    const owo::engine::MemoryLexicon lexicon({{{"zi", "dong"}, "自动", 1000},
                                              {{"zhan", "kai"}, "展开", 1000},
                                              {{"zi", "dong", "zhan"}, "自动站", 1000000},
                                              {{"kai"}, "开", 1000},
                                              {{"zi"}, "自", 10000},
                                              {{"zi"}, "字", 9000},
                                              {{"zi"}, "子", 8000},
                                              {{"zi"}, "资", 7000},
                                              {{"zi"}, "紫", 6000},
                                              {{"zi"}, "姿", 5000},
                                              {{"zi"}, "滋", 4000},
                                              {{"zi"}, "兹", 3000},
                                              {{"zi"}, "梓", 2000},
                                              {{"wei", "shen", "me"}, "为什么", 10000},
                                              {{"wo"}, "我", 10000}, {{"shi"}, "市", 10000},
                                              {{"ma"}, "吗", 10000}});
    const auto suffix = std::wstring(nonce.begin(), nonce.end());
    const auto core_pipe = LR"(\\.\pipe\OwO.Test.Joint.Core.)" + suffix;
    const auto model_pipe = LR"(\\.\pipe\OwO.Test.Joint.Model.)" + suffix;
    PrefixFirst backend;
    std::thread model(
        [&] { static_cast<void>(owo::ipc::run_model_server(model_pipe.c_str(), backend)); });
    std::thread core([&] {
        static_cast<void>(owo::ipc::run_core_server(core_pipe.c_str(), lexicon, nullptr,
                                                    model_pipe.c_str(), &monitor));
    });
    std::uint64_t id = 10;
    bool ok = true;
    for (const auto page_size : {5U, 7U, 9U}) {
        const auto generation = monitor.generation();
        config.candidate_page_size = page_size;
        ok &= writer.save(config).success &&
              monitor.wait_for_generation(generation, std::chrono::seconds(2));
        std::vector<std::string> all;
        std::vector<std::uint64_t> groups, consumed;
        owo::protocol::Message request{owo::protocol::MessageType::candidate_request, ++id, 998,
                                       "zidongzhankai"};
        for (unsigned page = 0; page < 10; ++page) {
            request.page = page;
            request.request_id = ++id;
            const auto base = send(core_pipe, request);
            if (!base.validation || base.message.candidates.empty()) {
                ok = false;
                break;
            }
            all.insert(all.end(), base.message.candidates.begin(), base.message.candidates.end());
            groups.insert(groups.end(), base.message.candidate_groups.begin(),
                          base.message.candidate_groups.end());
            consumed.insert(consumed.end(), base.message.candidate_consumed.begin(),
                            base.message.candidate_consumed.end());
            if (page == 0) {
                const auto &value = base.message;
                ok &= value.candidates.front() == "自动展开" &&
                      value.candidate_consumed.front() == 13 &&
                      value.candidate_groups.front() == 0 && value.model_pending;
                const auto prefix = std::find(value.candidates.begin(), value.candidates.end(), "自动站");
                ok &= prefix != value.candidates.end() && value.candidate_groups[
                    static_cast<std::size_t>(prefix - value.candidates.begin())] == 1;
                const auto sentence =
                    std::find(value.candidates.begin(), value.candidates.end(), "自动展开");
                ok &= sentence != value.candidates.end();
            }
            if (!base.message.has_more)
                break;
        }
        request = {owo::protocol::MessageType::candidate_request, ++id, 998, "zidongzhankai"};
        request.expanded = true;
        const auto expanded = send(core_pipe, request);
        ok &= expanded.validation && expanded.message.candidates == all &&
              expanded.message.candidate_groups == groups &&
              expanded.message.candidate_consumed == consumed;
        auto ranked =
            send(core_pipe, {owo::protocol::MessageType::candidate_update_request, id, 998});
        for (unsigned retry = 0; ranked.message.model_pending && retry < 10; ++retry)
            ranked =
                send(core_pipe, {owo::protocol::MessageType::candidate_update_request, id, 998});
        ok &= ranked.validation && !ranked.message.candidates.empty() &&
              ranked.message.candidates.front() == "自动展开" &&
              ranked.message.candidate_consumed.front() == 13 &&
              ranked.message.candidate_groups == groups &&
              ranked.message.candidates.size() == all.size();
        if (!ok)
            std::cerr << "Joint partition/pagination/model contract failed at page size "
                      << page_size << '\n';
    }
    const auto equal_length = send(core_pipe,
        {owo::protocol::MessageType::candidate_request, ++id, 998, "wsm"});
    const auto whole = std::find(equal_length.message.candidates.begin(),
                                equal_length.message.candidates.end(), "为什么");
    ok &= equal_length.validation && whole != equal_length.message.candidates.end() &&
        equal_length.message.candidate_groups[static_cast<std::size_t>(
            whole - equal_length.message.candidates.begin())] == 0;
    const auto generation = monitor.generation();
    config.candidate_partition_enabled = false;
    ok &= writer.save(config).success &&
          monitor.wait_for_generation(generation, std::chrono::seconds(2));
    const auto legacy = send(
        core_pipe, {owo::protocol::MessageType::candidate_request, ++id, 998, "zidongzhankai"});
    ok &= legacy.validation && legacy.message.candidate_groups.empty() &&
          !legacy.message.candidates.empty() && legacy.message.candidates.front() == "自动展开";
    static_cast<void>(send(core_pipe, {owo::protocol::MessageType::shutdown_request, 999, 998}));
    core.join();
    owo::model::ModelMessage stop;
    stop.type = owo::model::ModelMessageType::shutdown_request;
    static_cast<void>(owo::ipc::exchange(model_pipe.c_str(), owo::model::encode_model_message(stop),
                                         std::chrono::seconds(2)));
    model.join();
    monitor.stop();
    std::filesystem::remove_all(root);
    return ok ? 0 : 1;
}

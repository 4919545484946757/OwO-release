#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/model/model_backend.h"
#include "owo/model/model_protocol.h"
#include "owo/protocol/messages.h"
#include <Windows.h>
#include <atomic>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <thread>

class GatedModel final : public owo::model::IModelBackend {
public:
    std::atomic<bool> release{};
    std::atomic<bool> entered{};
    std::string_view id() const noexcept override { return "owo.test.gated.v1"; }
    owo::model::ModelResult rank(const owo::model::ModelRequest& request, std::stop_token) override {
        entered = true;
        const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
        while (!release && std::chrono::steady_clock::now() < deadline) Sleep(1);
        return {request.request_id, owo::model::ModelStatus::success, request.candidates, {}};
    }
};

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    const auto root = std::filesystem::absolute(argv[1]) / std::to_wstring(GetCurrentProcessId());
    std::filesystem::create_directories(root);
    SetEnvironmentVariableW(L"OWO_DICTIONARY_STORE_PATH", (root / L"dictionaries").c_str());
    SetEnvironmentVariableW(L"LOCALAPPDATA", root.c_str());
    owo::config::ConfigStore writer;
    if (!writer.load(root / L"owo.conf").success) return 3;
    auto config = writer.snapshot(); config.model_ranking_enabled = true; config.model_timeout_ms = 500;
    if (!writer.save(config).success) return 3;
    owo::config::ConfigMonitor monitor;
    if (!monitor.start(root / L"owo.conf").success) return 3;
    owo::engine::MemoryLexicon lexicon({{{"ni", "hao"}, "你好", 10000}, {{"ni"}, "你", 1000}});
    const auto core_pipe = LR"(\\.\pipe\OwO.ModelCancel.Core.)" + std::to_wstring(GetCurrentProcessId());
    const auto model_pipe = core_pipe + L".Model";
    GatedModel backend;
    std::jthread model([&] { (void)owo::ipc::run_model_server(model_pipe.c_str(), backend); });
    std::jthread core([&] { (void)owo::ipc::run_core_server(core_pipe.c_str(), lexicon, nullptr, model_pipe.c_str(), &monitor); });
    auto send = [&](owo::protocol::Message request) {
        for (unsigned attempt = 0; attempt < 20; ++attempt) {
            auto response = owo::ipc::exchange(core_pipe.c_str(), owo::protocol::encode_message(request), std::chrono::milliseconds(500));
            if (response.status) return owo::protocol::decode_message(response.response);
            Sleep(10);
        }
        return owo::protocol::DecodeResult{};
    };
    const auto request_id = (static_cast<std::uint64_t>(GetCurrentProcessId()) << 32) | 101;
    const auto name = owo::ipc::candidate_cancellation_event_name(request_id, 1);
    const HANDLE cancel = CreateEventW(nullptr, TRUE, FALSE, name.c_str());
    const auto first = send({owo::protocol::MessageType::candidate_request, request_id, 1, "nihao"});
    const auto entered_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(1);
    while (!backend.entered && std::chrono::steady_clock::now() < entered_deadline) Sleep(1);
    if (cancel) SetEvent(cancel);
    const auto started = std::chrono::steady_clock::now();
    const auto cancelled = send({owo::protocol::MessageType::candidate_update_request, request_id, 1, {}});
    const auto elapsed = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count();
    backend.release = true;
    const auto repeat = send({owo::protocol::MessageType::candidate_update_request, request_id, 1, {}});
    const auto shutdown = send({owo::protocol::MessageType::shutdown_request, 102, 1, {}});
    if (!shutdown.validation) { TerminateProcess(GetCurrentProcess(), 4); return 4; }
    core.join();
    owo::model::ModelMessage stop; stop.type = owo::model::ModelMessageType::shutdown_request;
    const auto ack = owo::ipc::exchange(model_pipe.c_str(), owo::model::encode_model_message(stop), std::chrono::seconds(1));
    if (!ack.status) { TerminateProcess(GetCurrentProcess(), 5); return 5; }
    model.join(); if (cancel) CloseHandle(cancel);
    // Assert protocol/cancellation behavior, not a scheduler-dependent cutoff.
    if (!cancel || !backend.entered || !first.validation || !first.message.model_pending || !cancelled.validation ||
        cancelled.message.model_pending || !cancelled.message.candidates.empty() ||
        !repeat.validation || !repeat.message.candidates.empty()) return 6;
    std::cout << "Cancelled model wait not published/replayed; update roundtrip " << elapsed << " ms.\n";
    return 0;
}

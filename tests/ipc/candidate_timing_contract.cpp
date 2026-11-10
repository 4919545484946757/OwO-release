#include "owo/diagnostics/candidate_timing.h"
#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"
#include "owo/model/model_backend.h"
#include "owo/model/model_protocol.h"
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    // Explicit per-test AppData, private pipe, in-memory synthetic dictionary.
    // Never enable diagnostics for the user's running TSF/Core or alter learning.
    const auto root = std::filesystem::absolute(argv[1]) /
        (L"capture-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(GetTickCount64()));
    const auto directory = root / L"OwO" / L"InputMethod" / L"diagnostics";
    std::filesystem::create_directories(directory);
    if (!SetEnvironmentVariableW(L"LOCALAPPDATA", root.c_str())) return 3;
    auto& recorder = owo::diagnostics::CandidateTiming::instance();
    recorder.refresh();
    if (recorder.active()) return 4;
    constexpr char session[] = "0123456789abcdef0123456789abcdef";
    const auto control = directory / L"candidate-timing.control";
    FILETIME born{}, exited{}, kernel{}, user{};
    if (!GetProcessTimes(GetCurrentProcess(), &born, &exited, &kernel, &user)) return 20;
    const auto creation = (static_cast<std::uint64_t>(born.dwHighDateTime) << 32) | born.dwLowDateTime;
    auto marker = [&](const char* id, std::uint64_t expiry, std::uint64_t owner_creation) {
        std::ofstream out(control, std::ios::binary);
        out << id << '\n' << expiry << '\n' << GetCurrentProcessId() << '\n' << owner_creation << '\n';
    };
    marker(session, GetTickCount64() + 60000, creation);
    Sleep(270); recorder.refresh();
    if (!recorder.active()) return 5;
    const owo::engine::MemoryLexicon lexicon({
        {{"ni"}, "你", 1000}, {{"hao"}, "好", 1000}, {{"ni", "hao"}, "你好", 10000}});
    const auto pipe = LR"(\\.\pipe\OwO.CandidateTiming.Contract.)" + std::to_wstring(GetCurrentProcessId());
    const auto model_pipe = pipe + L".Model";
    owo::config::ConfigStore config_writer;
    if (!config_writer.load(root / L"owo.conf").success) return 16;
    auto config = config_writer.snapshot(); config.model_ranking_enabled = true; config.user_learning_enabled = false;
    if (!config_writer.save(config).success) return 17;
    owo::config::ConfigMonitor monitor;
    if (!monitor.start(root / L"owo.conf").success) return 18;
    owo::model::MockModelBackend backend;
    std::jthread model_server([&] { (void)owo::ipc::run_model_server(model_pipe.c_str(), backend); });
    int server_exit = -1;
    std::jthread server([&] { server_exit = owo::ipc::run_core_server(pipe.c_str(), lexicon, nullptr, model_pipe.c_str(), &monitor); });
    auto send = [&](const owo::protocol::Message& message) {
        for (int attempt = 0; attempt < 30; ++attempt) {
            auto result = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(message), std::chrono::milliseconds(100));
            if (result.status) return owo::protocol::decode_message(result.response);
            Sleep(10);
        }
        return owo::protocol::DecodeResult{};
    };
    const auto first = send({owo::protocol::MessageType::candidate_request, 100, 1, "nihao"});
    const auto cached = send({owo::protocol::MessageType::candidate_request, 101, 1, "nihao"});
    const auto update = send({owo::protocol::MessageType::candidate_update_request, 101, 1, {}});
    const auto shutdown = send({owo::protocol::MessageType::shutdown_request, 102, 1, {}});
    if (!shutdown.validation) { TerminateProcess(GetCurrentProcess(), 6); return 6; }
    server.join();
    owo::model::ModelMessage model_shutdown;
    model_shutdown.type = owo::model::ModelMessageType::shutdown_request;
    auto model_ack = owo::ipc::exchange(model_pipe.c_str(), owo::model::encode_model_message(model_shutdown), std::chrono::seconds(1));
    if (!model_ack.status) { TerminateProcess(GetCurrentProcess(), 19); return 19; }
    model_server.join(); recorder.flush();
    if (server_exit != 0 || !first.validation || first.message.candidates.empty() ||
        !cached.validation || !update.validation || first.message.candidates != cached.message.candidates) return 7;
    const auto file = directory / (std::string(session) + "-" + std::to_string(GetCurrentProcessId()) + ".jsonl");
    std::ifstream input(file, std::ios::binary);
    std::string records((std::istreambuf_iterator<char>(input)), {});
    input.close();
    for (const auto* event : {"received", "prepare", "generation", "response_ready", "encoded", "written", "model_queued", "model_begin", "model_end", "ranked"})
        if (records.find(std::string("\"event\":\"") + event + '"') == std::string::npos) return 8;
    if (records.find("\"cache_hit\":1") == std::string::npos || records.find("nihao") != std::string::npos ||
        records.find("你好") != std::string::npos) return 9;
    std::filesystem::remove(control); Sleep(270); recorder.refresh();
    if (recorder.active()) return 10;
    marker("../../unsafe", GetTickCount64() + 60000, creation);
    Sleep(270); recorder.refresh();
    if (recorder.active()) return 11;
    marker(session, GetTickCount64() + 31ULL * 60000, creation);
    Sleep(270); recorder.refresh();
    if (recorder.active()) return 12;
    marker(session, 1, creation);
    Sleep(270); recorder.refresh();
    if (recorder.active()) return 13;
    marker(session, GetTickCount64() + 60000, creation + 1);
    Sleep(270); recorder.refresh();
    if (recorder.active()) return 21;
    marker(session, GetTickCount64() + 60000, creation);
    Sleep(270); recorder.refresh();
    for (int i = 0; i < 1200; ++i) recorder.emit(200, "tsf", "queued");
    recorder.flush(); recorder.emit(201, "tsf", "queued"); recorder.flush();
    std::ifstream bounded_input(file, std::ios::binary);
    const std::string bounded_records((std::istreambuf_iterator<char>(bounded_input)), {});
    if (bounded_records.find("\"dropped_total\":176") == std::string::npos) return 14;
    bounded_input.close();
    std::filesystem::resize_file(file, 8ULL * 1024 * 1024);
    recorder.emit(202, "tsf", "queued"); recorder.flush();
    if (std::filesystem::file_size(file) != 8ULL * 1024 * 1024) return 15;
    std::filesystem::remove(control);
    std::cout << "Candidate timing native contract passed (Core phases, cache, privacy, control validation).\n";
    return 0;
}

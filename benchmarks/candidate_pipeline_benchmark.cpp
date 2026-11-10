#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"
#include <Windows.h>
#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>
#include <vector>

namespace {
using Clock = std::chrono::steady_clock;
std::uint64_t fingerprint(const owo::protocol::Message& response) {
    std::uint64_t hash = 14695981039346656037ULL;
    auto add = [&](std::string_view value) {
        for (const unsigned char c : value) { hash ^= c; hash *= 1099511628211ULL; }
        hash ^= 255; hash *= 1099511628211ULL;
    };
    for (const auto& text : response.candidates) add(text);
    for (const auto value : response.candidate_consumed) add(std::to_string(value));
    for (const auto value : response.candidate_groups) add(std::to_string(value));
    for (const auto& text : response.syllables) add(text);
    add(response.has_more ? "1" : "0");
    return hash;
}
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 3 && argc != 4) return 2;
    unsigned samples = argc == 4 ? static_cast<unsigned>(std::stoul(argv[3])) : 50;
    if (!samples || samples > 1000) return 2;
    const auto output = std::filesystem::absolute(argv[2]);
    const auto root = output.parent_path() / (L"fixture-" + std::to_wstring(GetCurrentProcessId()));
    std::filesystem::create_directories(root);
    if (!SetEnvironmentVariableW(L"OWO_DICTIONARY_STORE_PATH", (root / L"dictionaries").c_str())) return 3;
    if (!SetEnvironmentVariableW(L"LOCALAPPDATA", root.c_str())) return 3;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(argv[1]).success) return 4;
    owo::config::ConfigStore writer;
    if (!writer.load(root / L"owo.conf").success) return 5;
    auto config = writer.snapshot(); config.candidate_page_size = 7; config.model_ranking_enabled = false;
    if (!writer.save(config).success) return 5;
    owo::config::ConfigMonitor monitor;
    if (!monitor.start(root / L"owo.conf").success) return 5;
    owo::engine::UserFrequencyStore frequencies;
    if (!frequencies.load(root / L"user.owuf").success) return 5;
    const auto pipe = LR"(\\.\pipe\OwO.Pipeline.Benchmark.)" + std::to_wstring(GetCurrentProcessId());
    std::ofstream log(root / "core.log");
    std::ofstream report(output, std::ios::binary);
    if (!log || !report) return 5;
    auto* previous_log = std::clog.rdbuf(log.rdbuf());
    int server_exit = -1;
    std::jthread server([&] { server_exit = owo::ipc::run_core_server(pipe.c_str(), lexicon, &frequencies, nullptr, &monitor); });
    owo::ipc::PersistentPipeClient client(pipe);
    std::uint64_t id = 0;
    bool failed = false;
    auto request = [&](std::string query, std::string context, unsigned page, bool expanded) {
        owo::protocol::Message message{owo::protocol::MessageType::candidate_request, ++id, 1, std::move(query)};
        message.context = std::move(context); message.page = page; message.expanded = expanded;
        const auto result = client.exchange(owo::protocol::encode_message(message), std::chrono::seconds(5));
        if (!result.status) { failed = true; return owo::protocol::Message{}; }
        auto decoded = owo::protocol::decode_message(result.response);
        if (!decoded.validation || decoded.message.type != owo::protocol::MessageType::candidate_response ||
            decoded.message.request_id != id) failed = true;
        return std::move(decoded.message);
    };
    for (const auto query : {"nihao", "nipingchangyong", "gongnnegjiekou", "xianzaide", "rgs",
                             "xiufuleisiwt", "butianjia", "baoliudangqiangaidongbingdabaoshangchuan"}) {
        for (const auto mode : {"cache_miss", "hot_first_page", "hot_page_two", "expanded"}) {
            const bool expanded = std::string_view(mode) == "expanded";
            const unsigned page = std::string_view(mode) == "hot_page_two" ? 1 : 0;
            const bool miss = std::string_view(mode) == "cache_miss";
            std::vector<double> times;
            std::uint64_t expected = 0;
            std::size_t candidate_count = 0;
            for (unsigned i = 0; i < samples + 5 && !failed; ++i) {
                const auto start = Clock::now();
                auto response = request(query, miss ? "benchmark-" + std::to_string(i) : "benchmark", page, expanded);
                const auto elapsed = std::chrono::duration<double, std::micro>(Clock::now() - start).count();
                const auto current = fingerprint(response);
                if (i == 0) { expected = current; candidate_count = response.candidates.size(); }
                else if (current != expected) { failed = true; break; }
                if (i >= 5) times.push_back(elapsed);
            }
            std::sort(times.begin(), times.end());
            if (times.empty()) { failed = true; break; }
            report << "{\"query\":\"" << query << "\",\"mode\":\"" << mode << "\",\"samples\":" << times.size()
                << ",\"candidate_count\":" << candidate_count << ",\"fingerprint\":\"" << expected
                << "\",\"p50_us\":" << times[(times.size()-1)/2]
                << ",\"p95_us\":" << times[(times.size()*95+99)/100-1] << "}\n";
        }
    }
    owo::protocol::Message shutdown{owo::protocol::MessageType::shutdown_request, ++id, 1, {}};
    const auto stopped = client.exchange(owo::protocol::encode_message(shutdown), std::chrono::seconds(2));
    client.reset(); server.join(); std::clog.rdbuf(previous_log);
    if (failed || !stopped.status || server_exit != 0) return 6;
    std::cout << "Isolated Core benchmark completed; synthetic queries only, no user data: " << output.string() << '\n';
    return 0;
}

#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"
#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>

int main() {
    using namespace owo;
    using namespace std::chrono_literals;
    const auto nonce = std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / ("owo-imported-phonetic-" + nonce);
    const auto dictionaries = root / "dictionaries";
    const auto snapshot = dictionaries / "snapshots" / std::string(32, 'a');
    std::filesystem::create_directories(snapshot);
    // Default store resolution must remain isolated even under a developer's
    // custom dictionary environment. This process owns its environment only.
    _putenv_s("OWO_DICTIONARY_STORE_PATH", dictionaries.string().c_str());
    {
        std::ofstream file(snapshot / "active.tsv", std::ios::binary);
        file << "OWO_DICTIONARIES_V1\nce'shi'ci'ku\t测试词库\t128\nce'shi\t测试\t32\nci\t词\t16\nku\t库\t16\n"
             << "ce'shi'fang'fa\t测试方法\t128\ncs\t测试词库\t16\nxi'an\t西安\t8\nxian\t先\t16\n";
        for (const auto* text : {"测", "册", "侧", "策", "厕", "恻", "澈", "彻", "撤", "车"})
            file << "ce\t" << text << "\t16\n";
    }
    { std::ofstream file(snapshot / "sources.tsv", std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t0\nuser\t0\n"; }
    { std::ofstream file(dictionaries / "current", std::ios::binary); file << std::string(32, 'a'); }
    engine::UserFrequencyStore frequency;
    if (!frequency.load(root / "frequency.owuf").success) return 1;
    frequency.record("nihao", "你好", 10);
    config::ConfigStore writer;
    if (!writer.load(root / "owo.conf").success) return 2;
    auto settings = writer.snapshot();
    settings.user_learning_enabled = false; settings.model_ranking_enabled = false;
    settings.candidate_page_size = 7; settings.candidate_partition_enabled = true;
    if (!writer.save(settings).success) return 3;
    config::ConfigMonitor monitor;
    if (!monitor.start(root / "owo.conf", 10ms).success) return 4;
    const engine::MemoryLexicon base({{{"ni", "hao"}, "你好", 100000}});
    const auto pipe = LR"(\\.\pipe\OwO.ImportedPhonetic.)" + std::wstring(nonce.begin(), nonce.end());
    int server_exit = -1;
    std::thread server([&] { server_exit = ipc::run_core_server(pipe.c_str(), base, &frequency, nullptr, &monitor); });
    ipc::PersistentPipeClient client(pipe);
    bool ok = true; std::uint64_t id = 0;
    const auto send = [&](std::string text, bool correction = true, bool expanded = false, std::uint64_t page = 0) {
        protocol::Message request{protocol::MessageType::candidate_request, ++id, 991, std::move(text)};
        request.correction_enabled = correction; request.expanded = expanded; request.page = page;
        const auto reply = client.exchange(protocol::encode_message(request), 2s);
        const auto decoded = reply.status ? protocol::decode_message(reply.response) : protocol::DecodeResult{};
        if (!decoded.validation || decoded.message.type != protocol::MessageType::candidate_response ||
            decoded.message.request_id != request.request_id) {
            std::cerr << "Invalid reply for " << request.text << '\n'; ok = false;
        }
        return decoded.message;
    };
    const auto contains = [&](const std::string& input, const std::string& word, bool correction = true) {
        const auto result = send(input, correction, true);
        for (std::size_t i = 0; i < result.candidates.size(); ++i)
            if (result.candidates[i] == word && result.candidate_consumed[i] == input.size()) return true;
        return false;
    };
    for (const auto* input : {"ceshiciku", "ce'shi'ci'ku", "csck", "ceshick", "cs"}) {
        if (!contains(input, "测试词库")) { std::cerr << "Missing full candidate: " << input << '\n'; ok = false; }
    }
    if (!contains("ceshifangaf", "测试方法") || contains("ceshifangaf", "测试方法", false) ||
        !contains("ciku", "词库") || !contains("cikuu", "词库") || contains("cikuu", "词库", false) ||
        contains("nihao", "你好") || contains("xi'an", "先")) ok = false;
    const auto first = send("ce");
    const auto expanded = send("ce", true, true);
    std::vector<std::string> paged;
    for (std::uint64_t page = 0; page < 10; ++page) {
        const auto result = send("ce", true, false, page);
        paged.insert(paged.end(), result.candidates.begin(), result.candidates.end());
        if (!result.has_more) break;
    }
    if (first.candidates.size() != 7 || paged != expanded.candidates || paged.size() < 10) {
        std::cerr << "Imported pagination/expanded snapshot mismatch\n"; ok = false;
    }
    // Publish a new empty source snapshot: old full/abbreviated candidates
    // and generation caches must all disappear, not just exact aliases.
    const auto disabled = dictionaries / "snapshots" / std::string(32, 'b');
    std::filesystem::create_directories(disabled);
    { std::ofstream file(disabled / "active.tsv", std::ios::binary); file << "OWO_DICTIONARIES_V1\n"; }
    { std::ofstream file(disabled / "sources.tsv", std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t0\nuser\t0\n"; }
    { std::ofstream file(dictionaries / "current", std::ios::binary); file << std::string(32, 'b'); }
    std::this_thread::sleep_for(300ms);
    if (contains("ceshiciku", "测试词库") || contains("csck", "测试词库")) { std::cerr << "Disabled source leaked\n"; ok = false; }
    protocol::Message shutdown{protocol::MessageType::shutdown_request, ++id, 991, ""};
    const auto reply = client.exchange(protocol::encode_message(shutdown), 2s);
    if (!reply.status) ok = false;
    client.reset(); server.join(); monitor.stop();
    std::filesystem::remove_all(root);
    if (!ok || server_exit) return 5;
    std::cout << "Imported phonetic IPC: disabled builtins, abbreviation, mixed, correction toggle, composition, pagination, source invalidation passed.\n";
}

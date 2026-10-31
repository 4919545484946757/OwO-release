#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/model/model_protocol.h"
#include "owo/protocol/messages.h"
#include <algorithm>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <set>
#include <thread>

namespace {
using namespace owo;
class HiddenFirst final : public model::IModelBackend {
public:
    std::string_view id() const noexcept override { return "owo.test.snapshot"; }
    model::ModelResult rank(const model::ModelRequest& request, std::stop_token) override {
        auto candidates = request.candidates;
        const auto target = request.input == "mingtianshangwushidianqian" ? "明天上午十点前" :
            request.input == "yihuowuyunshudesudu" ? "以货物运输的速度" :
            request.input == "weiyonghutigongfuwu" ? "为用户提供服务" :
            request.input == "xiufuleisiwenti" ? "朽腐类似问题" :
            request.input == "biandeyouyisima" ? "变得有意思吗" :
            request.context == "伸手" ? "接过" : "你号世界";
        const auto found = std::find(candidates.begin(), candidates.end(), target);
        if (found != candidates.end()) std::rotate(candidates.begin(), found, found + 1);
        return {request.request_id, model::ModelStatus::success, std::move(candidates), {}};
    }
};
std::string character(unsigned code) {
    return {char(0xe0 | (code >> 12)), char(0x80 | ((code >> 6) & 63)), char(0x80 | (code & 63))};
}
bool same(const protocol::Message& left, const protocol::Message& right) {
    return left.candidates == right.candidates && left.candidate_consumed == right.candidate_consumed && left.candidate_groups == right.candidate_groups;
}
}

int main() {
    using namespace std::chrono_literals;
    struct QuietLog {
        struct Buffer : std::streambuf { int_type overflow(int_type c) override { return traits_type::not_eof(c); } } sink;
        std::streambuf* previous{std::clog.rdbuf(&sink)};
        ~QuietLog() { std::clog.rdbuf(previous); }
    } quiet;
    const auto nonce = std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / ("owo-candidate-snapshot-" + nonce);
    std::filesystem::create_directories(root);
    config::ConfigStore writer;
    if (!writer.load(root / "owo.conf").success) return 2;
    auto settings = writer.snapshot(); settings.model_ranking_enabled = true; settings.model_timeout_ms = 500;
    if (!writer.save(settings).success) return 2;
    config::ConfigMonitor monitor;
    if (!monitor.start(root / "owo.conf", 10ms).success) return 2;
    std::vector<engine::LexiconEntry> entries{
        {{"ni", "hao"}, "你好", 10000}, {{"ni", "hao"}, "你号", 9000},
        {{"shi", "jie"}, "世界", 10000}, {{"ni"}, "你", 100000}, {{"hao"}, "好", 100000},
        {{"shi"}, "世", 100000}, {{"jie"}, "界", 100000},
        {{"jie", "guo"}, "结果", 500000}, {{"jie", "guo"}, "接过", 128000},
        {{"jie", "guo"}, "解过", 100}, {{"jie"}, "结", 100000}, {{"jie"}, "解", 1000},
        {{"shou", "xuan"}, "首选", 234000}, {{"shou", "xuan"}, "手癣", 94},
        {{"shou"}, "首", 100000}, {{"shou"}, "手", 1000},
        {{"kai", "jie"}, "开解", 10000000}, {{"zai", "shou"}, "在手", 10000000},
        {{"kuang", "wang"}, "狂妄", 10000}, {{"wang"}, "王", 1000000}, {{"wang"}, "妄", 100}};
    entries.insert(entries.end(), {
        {{"ming", "tian", "shang", "wu"}, "明天上午", 500000},
        {{"shi"}, "十", 1000000}, {{"dian", "qian"}, "点钱", 500000},
        {{"shi", "dian"}, "十点", 20000}, {{"qian"}, "前", 1000000},
        {{"shang", "wu", "shi"}, "上午十", 100},
        {{"bian", "de"}, "变得", 10000000}, {{"bian", "de"}, "变的", 10000000},
        {{"you", "yi"}, "有一", 10000000}, {{"si", "ma"}, "司马", 10000000},
        {{"you", "yi", "si", "ma"}, "有意思吗", 100},
        {{"bian"}, "变", 100000}, {{"bian"}, "边", 100000},
        {{"bian"}, "便", 100000}, {{"bian"}, "编", 100000},
        {{"qian"}, "前", 10000}, {{"qian"}, "钱", 9000},
        {{"qi", "an"}, "奇案", 1000000}, {{"qi", "an"}, "起岸", 900000},
        {{"qi"}, "奇", 100000}, {{"qi"}, "起", 90000},
        {{"yi"}, "以", 1000000}, {{"yi"}, "易", 700000},
        {{"wei"}, "为", 1000000}, {{"wei"}, "位", 700000},
        {{"yi", "huo"}, "易货", 1}, {{"wei", "yong", "hu"}, "位用户", 1},
        {{"huo", "wu", "yun", "shu"}, "货物运输", 500000},
        {{"huo", "wu"}, "货物", 500000}, {{"yun", "shu"}, "运输", 500000},
        {{"de"}, "的", 1000000}, {{"de", "su", "du"}, "的速度", 1},
        {{"su", "du"}, "速度", 500000},
        {{"yong", "hu", "ti", "gong"}, "用户提供", 500000},
        {{"yong", "hu"}, "用户", 500000},
        {{"ti", "gong", "fu", "wu"}, "提供服务", 500000},
        {{"fu", "wu"}, "服务", 500000},
        {{"xiu", "fu"}, "修复", 100000000}, {{"xiu", "fu"}, "朽腐", 1},
        {{"lei", "si", "wen", "ti"}, "类似问题", 50000},
    });
    for (unsigned index = 0; index < 120; ++index) {
        const auto text = character(0x4e00 + index);
        entries.push_back({{"zi"}, text, 200000 - index * 1000});
        entries.push_back({{"zi", "ji"}, text + "己", 200000 - index * 1000});
        entries.push_back({{"ni"}, text, 2000 - index});
        entries.push_back({{"ni", "hao"}, text + "好", 2000 - index});
    }
    const engine::MemoryLexicon lexicon(std::move(entries));
    const auto suffix = std::wstring(nonce.begin(), nonce.end());
    const auto pipe = LR"(\\.\pipe\OwO.Test.Snapshot.Core.)" + suffix;
    const auto model_pipe = pipe + L".Model";
    HiddenFirst backend;
    std::thread model_server([&] { static_cast<void>(ipc::run_model_server(model_pipe.c_str(), backend)); });
    std::thread server([&] { static_cast<void>(ipc::run_core_server(pipe.c_str(), lexicon, nullptr, model_pipe.c_str(), &monitor)); });
    ipc::PersistentPipeClient client(pipe);
    bool ok = true; std::uint64_t id = 0;
    const auto send = [&](protocol::Message request, bool correlated = false) {
        request.request_id = correlated ? id : ++id; request.context_generation = 771;
        const auto reply = client.exchange(protocol::encode_message(request), 2s);
        const auto decoded = reply.status ? protocol::decode_message(reply.response) : protocol::DecodeResult{};
        if (!decoded.validation || decoded.message.type == protocol::MessageType::error_response) ok = false;
        return decoded.message;
    };
    const auto settle = [&](protocol::Message base) {
        if (!base.model_pending) return base;
        for (unsigned retry = 0; retry < 12; ++retry) {
            protocol::Message update; update.type = protocol::MessageType::candidate_update_request;
            auto ranked = send(update, true);
            if (ranked.model_pending) continue;
            if (ranked.candidates.empty()) { ok = false; return base; }
            base.candidates = std::move(ranked.candidates); base.candidate_consumed = std::move(ranked.candidate_consumed); base.candidate_groups = std::move(ranked.candidate_groups); base.model_pending = false;
            return base;
        }
        ok = false; return base;
    };
    for (const bool partition : {false, true}) for (const auto size : {5U, 7U, 9U}) {
        const auto generation = monitor.generation();
        settings.candidate_partition_enabled = partition; settings.candidate_page_size = size;
        ok &= writer.save(settings).success && monitor.wait_for_generation(generation, 2s);
        for (const auto& input : {"zj", "nihaoshijie", "biandeyouyisima",
                                 "yihuowuyunshudesudu", "weiyonghutigongfuwu"}) for (const bool expanded_first : {false, true}) {
            const auto verify = [&](bool value, const char* stage) {
                if (!value) std::cerr << stage << ": " << input << " size=" << size << " partition=" << partition << " expanded_first=" << expanded_first << '\n';
                ok &= value;
            };
            protocol::Message request; request.type = protocol::MessageType::candidate_request; request.text = input;
            request.context = expanded_first ? "expanded-first" : "paged-first";
            protocol::Message expanded;
            if (expanded_first) { request.expanded = true; expanded = settle(send(request)); request.expanded = false; }
            auto first = settle(send(request));
            if (request.text == "yihuowuyunshudesudu")
                verify(!first.candidates.empty() && first.candidates.front() == "以货物运输的速度", "standalone head model choice was blocked");
            if (request.text == "weiyonghutigongfuwu")
                verify(!first.candidates.empty() && first.candidates.front() == "为用户提供服务", "standalone head model choice was blocked");
            if (request.text == "nihaoshijie") verify(!first.candidates.empty() && first.candidates.front() == "你号世界", "model did not promote hidden target");
            if (request.text == "biandeyouyisima") {
                verify(!first.candidates.empty() && first.candidates.front() == "变得有意思吗", "model did not replace fragmented sentence");
                verify(std::find(first.candidates.begin(), first.candidates.end(), "变得有一司马") == first.candidates.end(), "weak second sentence survived model update");
            }
            auto paged = first; std::set<std::pair<std::string, std::uint64_t>> unique;
            const auto record = [&](const protocol::Message& row) {
                for (std::size_t i = 0; i < row.candidates.size(); ++i) verify(unique.emplace(row.candidates[i], row.candidate_consumed[i]).second, "duplicate");
            };
            record(first);
            for (unsigned page = 1; paged.has_more && page <= 100; ++page) {
                request.page = page; const auto row = send(request);
                if (row.candidates.empty()) { ok = false; break; }
                record(row); paged.has_more = row.has_more;
                paged.candidates.insert(paged.candidates.end(), row.candidates.begin(), row.candidates.end());
                paged.candidate_consumed.insert(paged.candidate_consumed.end(), row.candidate_consumed.begin(), row.candidate_consumed.end());
                paged.candidate_groups.insert(paged.candidate_groups.end(), row.candidate_groups.begin(), row.candidate_groups.end());
            }
            request.page = 0; const auto revisited = send(request);
            verify(same(first, revisited) && !revisited.model_pending, "revisit");
            request.expanded = true; const auto latest_expanded = settle(send(request));
            if (!same(paged, latest_expanded)) std::cerr << "counts " << paged.candidates.size() << '/' << latest_expanded.candidates.size() << '\n';
            verify(same(paged, latest_expanded), "paged vs expanded");
            if (request.text == "nihaoshijie")
                verify(std::count(latest_expanded.candidate_consumed.begin(), latest_expanded.candidate_consumed.end(), request.text.size()) <= 2,
                       "model exceeded original sentence quota");
            if (expanded_first) verify(same(expanded, latest_expanded), "expanded before vs after");
            request.expanded = false; verify(same(first, send(request)), "collapsed");
        }
    }
    const auto expect = [&](std::string input, std::string previous, std::string context, std::string word) {
        protocol::Message request; request.type = protocol::MessageType::candidate_request; request.text = input; request.input = previous; request.context = context;
        const auto first = settle(send(request));
        const bool matched = !first.candidates.empty() && first.candidates.front() == word;
        if (!matched) std::cerr << "context mismatch: " << input << ' '
                                << protocol::encode_message(first) << '\n';
        ok &= matched;
    };
    expect("jieguo", "zhankai", "展开", "结果");
    expect("shouxuan", "buzai", "不在", "首选");
    expect("wang", "kuang", "狂", "妄");
    expect("jieguo", "shenshou", "伸手", "接过"); // The model may still choose a credible homophone.
    expect("mingtianshangwushidianqian", "", "", "明天上午十点前");
    expect("qian", "fenge", "分隔", "前");
    expect("xiufuleisiwenti", "", "", "修复类似问题"); // An intact leading word keeps its lexical guard.
    protocol::Message stop; stop.type = protocol::MessageType::shutdown_request; static_cast<void>(send(stop));
    client.reset(); server.join();
    model::ModelMessage model_stop; model_stop.type = model::ModelMessageType::shutdown_request;
    static_cast<void>(ipc::exchange(model_pipe.c_str(), model::encode_model_message(model_stop), 2s)); model_server.join(); monitor.stop();
    std::filesystem::remove_all(root);
    return ok ? 0 : 1;
}

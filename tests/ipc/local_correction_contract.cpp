#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <algorithm>
#include <chrono>
#include <iostream>
#include <string_view>
#include <thread>
#include <utility>

namespace {
owo::protocol::DecodeResult send(const std::wstring &pipe, const owo::protocol::Message &request) {
    const auto result = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
                                           std::chrono::seconds(2));
    return result.status ? owo::protocol::decode_message(result.response)
                         : owo::protocol::DecodeResult{};
}
} // namespace

int main() {
    // Deliberately omit the whole sentence. A local repair must also work
    // when independent dictionary words cover its prefix and suffix.
    const owo::engine::MemoryLexicon lexicon({{{"gong", "neng"}, "功能", 10000},
                                              {{"jie", "kou"}, "接口", 9000},
                                              {{"cai", "dan"}, "菜单", 8000},
                                              {{"gong"}, "公", 1000000},
                                              {{"gong"}, "功", 990000},
                                              {{"gong"}, "工", 980000},
                                              {{"gong"}, "攻", 970000},
                                              {{"gong"}, "宫", 960000},
                                              {{"gong"}, "共", 950000},
                                              {{"gong"}, "供", 940000},
                                              {{"gong"}, "恭", 930000},
                                              {{"na"}, "哪", 1000000},
                                              {{"ne"}, "呢", 1000000},
                                              {{"ga"}, "尬", 1000000},
                                              {{"ge"}, "个", 900000},
                                              {{"bu", "gan", "dang"}, "不敢当", 5000},
                                              {{"wei", "shen", "me"}, "为什么", 5000},
                                              {{"wo"}, "我", 5000},
                                              {{"ren", "wei"}, "认为", 6000},
                                              {{"ke", "yi"}, "可以", 6000},
                                              {{"xian", "zai"}, "现在", 20000},
                                              {{"xian"}, "先", 5000}, {{"xian"}, "现", 4900},
                                              {{"xian"}, "县", 4800}, {{"xian"}, "线", 4700},
                                              {{"xian"}, "鲜", 4600}, {{"xian"}, "仙", 4500},
                                              {{"ce", "shi"}, "测试", 10000},
                                              {{"yi", "xia"}, "一下", 10000},
                                              {{"yi"}, "仪", 9000},
                                              {{"xi", "ai"}, "喜爱", 1000000},
                                              {{"ji", "xu"}, "继续", 10000},
                                              {{"jie", "mian"}, "界面", 10000},
                                              {{"chang", "du"}, "长度", 10000},
                                              {{"qing", "kuang"}, "情况", 10000},
                                              {{"zhe", "ge"}, "这个", 10000},
                                              {{"fang", "fa"}, "方法", 10000},
                                              {{"fang", "shi"}, "方式", 10000}});
    const auto nonce = std::chrono::steady_clock::now().time_since_epoch().count();
    const auto pipe = LR"(\\.\pipe\OwO.Test.LocalCorrection.)" + std::to_wstring(nonce);
    int exit_code = -1;
    std::thread server([&] { exit_code = owo::ipc::run_core_server(pipe.c_str(), lexicon); });
    owo::protocol::Message request{owo::protocol::MessageType::candidate_request, 0, 1, {}};
    bool ok = true;
    const auto query = [&](const std::string &input, bool correction, bool deferred = false,
                           bool expanded = false) {
        request.text = input;
        ++request.request_id;
        request.correction_enabled = correction;
        request.defer_expensive_candidates = deferred;
        request.expanded = expanded;
        return send(pipe, request);
    };
    for (const auto &input : {"gongnnegjiekou", "gongnegnjiekou", "gongnnegcaidan"}) {
        const std::string expected =
            std::string_view(input).ends_with("caidan") ? "功能菜单" : "功能接口";
        const auto base = query(input, true, true);
        const auto repaired = query(input, true);
        const auto expanded = query(input, true, false, true);
        const auto strict = query(input, false);
        const auto contains = [&](const auto &result) {
            return result.validation &&
                   std::find(result.message.candidates.begin(), result.message.candidates.end(),
                             expected) != result.message.candidates.end();
        };
        if (!base.validation || !base.message.candidate_refinement_pending || !contains(repaired) ||
            repaired.message.candidates.front() != expected ||
            repaired.message.candidate_refinement_pending || !contains(expanded) ||
            !strict.validation || contains(strict) ||
            repaired.message.candidate_consumed.front() != std::string_view(input).size() ||
            repaired.message.syllables.size() != 4 ||
            repaired.message.syllables[1] != std::string_view(input).substr(4, 4)) {
            std::cerr << "Fragmented abbreviation hid local repair: " << input << '\n';
            if (repaired.validation)
                std::cerr << owo::protocol::encode_message(repaired.message) << '\n';
            ok = false;
        }
    }
    for (const auto& [input, expected] : {std::pair{"ceshiyixa", "测试一下"},
            {"ceshiyixajixu", "测试一下继续"},
            {"ceshigongnneg", "测试功能"}, {"ceshijiemain", "测试界面"},
            {"ceshichnagdu", "测试长度"}, {"ceshiqignkuang", "测试情况"},
            {"zhegefnagfa", "这个方法"}, {"zhegefnagshi", "这个方式"},
            {"xianzaiceshiyixa", "现在测试一下"},
            {"xianzaixianceshiyixa", "现在先测试一下"}}) {
        const auto base = query(input, true, true);
        const auto repaired = query(input, true);
        const auto expanded = query(input, true, false, true);
        const auto strict = query(input, false);
        if (!base.validation || !base.message.candidate_refinement_pending ||
            !repaired.validation || repaired.message.candidates.empty() ||
            repaired.message.candidates.front() != expected ||
            repaired.message.candidate_consumed.front() != std::string_view(input).size() ||
            !expanded.validation || expanded.message.candidates.empty() ||
            expanded.message.candidates.front() != expected ||
            !strict.validation || std::find(strict.message.candidates.begin(),
                strict.message.candidates.end(), expected) != strict.message.candidates.end()) {
            std::cerr << "Fragmented vowel suffix hid missing-letter repair: " << input << '\n';
            ok = false;
        }
    }
    // Real whole-word abbreviations and dictionary-backed multiword phrases
    // must not be mistaken for fragmented character permutations.
    for (const auto &[input, text] : {std::pair{"bugd", "不敢当"},
                                      {"wsm", "为什么"},
                                      {"rwky", "认为可以"},
                                      {"gongnengjiekou", "功能接口"}}) {
        const auto response = query(input, true);
        if (!response.validation || response.message.candidates.empty() ||
            response.message.candidates.front() != text ||
            response.message.candidate_refinement_pending) {
            std::cerr << "Valid dictionary reading changed: " << input << '\n';
            ok = false;
        }
    }
    const auto shutdown = send(pipe, {owo::protocol::MessageType::shutdown_request, 999, 1, {}});
    server.join();
    return ok && shutdown.validation && exit_code == 0 ? 0 : 1;
}

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
                                              {{"ke", "yi"}, "可以", 6000}});
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

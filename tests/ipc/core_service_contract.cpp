#include "owo/ipc/named_pipe.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/protocol/messages.h"

#include <chrono>
#include <atomic>
#include <iostream>
#include <filesystem>
#include <thread>

namespace {

std::wstring contract_pipe_name() {
    const auto unique_suffix = std::chrono::steady_clock::now().time_since_epoch().count();
    return LR"(\\.\pipe\OwO.InputMethod.ContractTest.)" + std::to_wstring(unique_suffix);
}

owo::protocol::DecodeResult send_request(const std::wstring& pipe_name,
                                         const owo::protocol::Message& request) {
    for (int attempt = 0; attempt < 100; ++attempt) {
        const auto result = owo::ipc::exchange(pipe_name.c_str(),
                                               owo::protocol::encode_message(request),
                                               std::chrono::milliseconds(100));
        if (result.status) return owo::protocol::decode_message(result.response);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return {};
}

owo::protocol::DecodeResult send_payload(const std::wstring& pipe_name,
                                         const std::string& payload) {
    for (int attempt = 0; attempt < 100; ++attempt) {
        const auto result = owo::ipc::exchange(pipe_name.c_str(), payload,
                                               std::chrono::milliseconds(100));
        if (result.status) return owo::protocol::decode_message(result.response);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return {};
}

bool valid_response(const owo::protocol::DecodeResult& response,
                    const std::uint64_t request_id,
                    const std::uint64_t generation,
                    const std::vector<std::string>& candidates = {"你好", "你号"},
                    const std::vector<std::string>& syllables = {"ni", "hao"},
                    const std::uint64_t consumed = 5) {
    return response.validation &&
           response.message.type == owo::protocol::MessageType::candidate_response &&
           response.message.request_id == request_id &&
           response.message.context_generation == generation &&
           response.message.candidates == candidates &&
           response.message.syllables == syllables &&
           response.message.page_size == 7 && !response.message.expanded &&
           response.message.correction_enabled &&
           response.message.candidate_consumed ==
               std::vector<std::uint64_t>(candidates.size(), consumed);
}

}  // namespace

int main() {
    const auto pipe_name = contract_pipe_name();
    const auto path = std::filesystem::temp_directory_path() / "owo-core-contract.owolx";
    const auto user_path = std::filesystem::temp_directory_path() / "owo-core-contract-user.bin";
    std::error_code ignored;
    std::filesystem::remove(user_path, ignored);
    std::filesystem::remove(user_path.wstring() + L".bak", ignored);
    const auto written = owo::engine::write_binary_lexicon(path, {
        {{"ni", "hao"}, "你好", 1000}, {{"ni", "hao"}, "你号", 50},
        {{"ce", "shi"}, "测试一", 1000}, {{"ce", "shi"}, "测试二", 900},
        {{"ce", "shi"}, "测试三", 800}, {{"ce", "shi"}, "测试四", 700},
        {{"ce", "shi"}, "测试五", 600}, {{"ce", "shi"}, "测试六", 500},
        {{"ce", "shi"}, "测试七", 400}, {{"ce", "shi"}, "测试八", 300},
        {{"ce", "shi"}, "测试九", 200},
        {{"wo", "ai"}, "我爱", 1500}, {{"wo"}, "我", 1400},
        {{"ai"}, "爱", 1350}, {{"shi"}, "是", 1300},
        {{"ma"}, "吗", 1250},
        {{"wei", "shen", "me"}, "为什么", 9000},
        {{"wan", "shen", "me"}, "玩什么", 3000},
        {{"jun", "jiang"}, "均将", 5000},
        {{"he", "jun"}, "核菌", 4500},
        {{"he"}, "和", 5000}, {{"he"}, "合", 4000},
        {{"he"}, "盒", 3900}, {{"he"}, "喝", 3800},
        {{"gong", "neng"}, "功能", 1700},
        {{"shi", "jie"}, "世界", 1600},
        {{"liang"}, "两", 1800}, {{"liang"}, "亮", 1700},
        {{"liang"}, "量", 1600}, {{"liang"}, "梁", 1500},
        {{"liang"}, "良", 1400},
        {{"li", "ang"}, "里昂", 1300}, {{"li", "ang"}, "离昂", 1200},
        {{"li", "ang"}, "立昂", 1100},
        {{"ga"}, "A", 1000}, {{"ge"}, "B", 900},
        {{"gou"}, "C", 800}, {{"gu"}, "D", 700},
        {{"gang"}, "E", 600}, {{"gong"}, "F", 500},
        {{"ga", "da"}, "aa", 2000}, {{"ge", "de"}, "bb", 1900},
        {{"gou", "dong"}, "cc", 1800}, {{"gu", "dian"}, "dd", 1700},
        {{"gang", "du"}, "ee", 1600}, {{"gong", "di"}, "ff", 1500},
        {{"gui", "dao"}, "gg", 1400}});
    owo::engine::BinaryLexicon lexicon;
    const auto loaded = lexicon.load(path);
    owo::engine::UserFrequencyStore user_frequency;
    if (!written.success || !loaded.success || !user_frequency.load(user_path).success) return 2;
    std::atomic<int> server_exit{-1};
    std::jthread server([&server_exit, &lexicon, &user_frequency, &pipe_name] {
        server_exit = owo::ipc::run_core_server(pipe_name.c_str(), lexicon, &user_frequency);
    });
    owo::ipc::PersistentPipeClient persistent(pipe_name);
    const auto first = owo::protocol::decode_message(persistent.exchange(
        owo::protocol::encode_message({owo::protocol::MessageType::candidate_request,
                                       101, 7, "nihao"}),
        std::chrono::seconds(2)).response);
    const auto second = owo::protocol::decode_message(persistent.exchange(
        owo::protocol::encode_message({owo::protocol::MessageType::candidate_request,
                                       102, 8, "nihao"}),
        std::chrono::seconds(2)).response);
    persistent.reset();
    auto invalid_payload = owo::protocol::encode_message(
        {owo::protocol::MessageType::candidate_request, 120, 12, "nihao"});
    const auto page_size_offset = invalid_payload.find("\"page_size\":0");
    owo::protocol::DecodeResult invalid_request{};
    if (page_size_offset != std::string::npos) {
        invalid_payload.replace(page_size_offset, std::string("\"page_size\":0").size(),
                                "\"page_size\":1");
        invalid_request = send_payload(pipe_name, invalid_payload);
    }
    const bool invalid_request_identity_ok =
        invalid_request.validation &&
        invalid_request.message.type == owo::protocol::MessageType::error_response &&
        invalid_request.message.request_id == 120 &&
        invalid_request.message.context_generation == 12;
    const auto nihao_ranged = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    108, 8, "ni'hao'shi'jie"});
    owo::protocol::Message deferred_correction_request{
        owo::protocol::MessageType::candidate_request, 127, 8, "niaho"};
    deferred_correction_request.defer_expensive_candidates = true;
    const auto deferred_correction_base = send_request(
        pipe_name, deferred_correction_request);
    deferred_correction_request.defer_expensive_candidates = false;
    const auto deferred_correction_complete = send_request(
        pipe_name, deferred_correction_request);
    const auto corrected = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 110, 8, "niaho"});
    const auto corrected_second_syllable = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 121, 8, "gongnneg"});
    const bool corrected_second_syllable_ok =
        corrected_second_syllable.validation &&
        !corrected_second_syllable.message.candidates.empty() &&
        corrected_second_syllable.message.candidates.front() == "功能" &&
        corrected_second_syllable.message.syllables ==
            std::vector<std::string>{"gong", "nneg"};
    const auto stable_xingb = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 112, 8, "xingb"});
    const auto stable_xingbaf = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 113, 8, "xingbaf"});
    const auto stable_mingd = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 116, 8, "mingd"});
    const auto stable_kenengd = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 117, 8, "kenengd"});
    owo::protocol::Message double_initial_second_request{
        owo::protocol::MessageType::candidate_request, 119, 8, "gd"};
    const auto double_initial_first = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 118, 8, "gd"});
    double_initial_second_request.page = 1;
    const auto double_initial_second = send_request(
        pipe_name, double_initial_second_request);
    const auto sentence_first = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    122, 8, "woaishi"});
    owo::protocol::Message sentence_second_request{
        owo::protocol::MessageType::candidate_request, 123, 8, "woaishi"};
    sentence_second_request.page = 1;
    const auto sentence_second = send_request(pipe_name, sentence_second_request);
    const auto composed_non_dictionary = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    124, 8, "woshi"});
    const auto winning_sentence_head = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    125, 8, "hejunjiang"});
    const auto ambiguous_single_syllable = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    126, 8, "liang"});
    const auto abbreviation_partition_order = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    128, 8, "wsm"});
    owo::protocol::Message correction_disabled_request{
        owo::protocol::MessageType::candidate_request, 111, 8, "niaho"};
    correction_disabled_request.correction_enabled = false;
    const auto correction_disabled = send_request(pipe_name, correction_disabled_request);
    owo::protocol::Message separated_correction_disabled_request{
        owo::protocol::MessageType::candidate_request, 114, 8, "quan'loi"};
    separated_correction_disabled_request.correction_enabled = false;
    const auto separated_correction_disabled = send_request(
        pipe_name, separated_correction_disabled_request);
    const auto separated_correction_enabled = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request, 115, 8, "quan'loi"});
    bool commits_ok = true;
    for (std::uint64_t request = 0; request < 5; ++request) {
        const auto committed = send_request(pipe_name, {owo::protocol::MessageType::candidate_committed,
                                         200 + request, 8, "你号"});
        commits_ok = commits_ok && committed.validation &&
                     committed.message.type == owo::protocol::MessageType::acknowledgement;
    }
    owo::protocol::Message custom_word_commit{
        owo::protocol::MessageType::candidate_committed, 220, 8, "自定义词"};
    custom_word_commit.input = "zidingyici";
    const auto custom_word_ack = send_request(pipe_name, custom_word_commit);
    const auto custom_word_candidates = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    221, 8, "zidingyici"});
    const auto incomplete_custom_word_candidates = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    222, 8, "zidingyic"});
    const auto missing_syllable_custom_word_candidates = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    223, 8, "zidingyi"});
    const auto learned = send_request(pipe_name, {owo::protocol::MessageType::candidate_request, 104, 8, "nihao"});
    const auto first_page = send_request(pipe_name, {owo::protocol::MessageType::candidate_request, 106, 8, "ceshi"});
    owo::protocol::Message second_page_request{
        owo::protocol::MessageType::candidate_request, 105, 8, "ceshi"};
    second_page_request.page = 1;
    const auto second_page = send_request(pipe_name, second_page_request);
    owo::protocol::Message expanded_request{
        owo::protocol::MessageType::candidate_request, 109, 8, "ceshi"};
    expanded_request.expanded = true;
    const auto expanded = send_request(pipe_name, expanded_request);
    std::vector<std::string> paged_candidates = first_page.message.candidates;
    paged_candidates.insert(paged_candidates.end(),
                            second_page.message.candidates.begin(),
                            second_page.message.candidates.end());
    const auto ranged = send_request(
        pipe_name, {owo::protocol::MessageType::candidate_request,
                    107, 8, "wo'ai'shi'jie"});
    const auto shutdown = send_request(pipe_name, {owo::protocol::MessageType::shutdown_request, 103, 9, {}});
    server.join();
    std::filesystem::remove(path, ignored);
    owo::engine::UserFrequencyStore persisted;
    const auto persisted_result = persisted.load(user_path);
    std::filesystem::remove(user_path, ignored);
    std::filesystem::remove(user_path.wstring() + L".bak", ignored);
    if (!invalid_request_identity_ok) {
        std::cerr << "invalid request response did not preserve request identity\n";
        return 4;
    }
    if (!corrected_second_syllable_ok) {
        std::cerr << "second-syllable correction contract failed\n";
        return 5;
    }
    const bool abbreviation_partition_order_ok =
        abbreviation_partition_order.validation &&
        !abbreviation_partition_order.message.candidates.empty() &&
        abbreviation_partition_order.message.candidate_groups.size() ==
            abbreviation_partition_order.message.candidates.size() &&
        abbreviation_partition_order.message.candidates.front() == "为什么" &&
        abbreviation_partition_order.message.candidate_groups.front() == 1 &&
        std::find(abbreviation_partition_order.message.candidate_groups.begin(),
                  abbreviation_partition_order.message.candidate_groups.end(), 0) !=
            abbreviation_partition_order.message.candidate_groups.end() &&
        std::find(abbreviation_partition_order.message.candidate_groups.begin(),
                  abbreviation_partition_order.message.candidate_groups.end(), 2) !=
            abbreviation_partition_order.message.candidate_groups.end();
    if (!abbreviation_partition_order_ok) {
        std::cerr << "abbreviation partition confidence order failed\n";
        for (std::size_t index = 0;
             index < abbreviation_partition_order.message.candidates.size(); ++index) {
            std::cerr << abbreviation_partition_order.message.candidates[index];
            if (index < abbreviation_partition_order.message.candidate_groups.size())
                std::cerr << ':'
                          << abbreviation_partition_order.message.candidate_groups[index];
            std::cerr << '\n';
        }
        return 14;
    }
    const auto composed = std::find(
        composed_non_dictionary.message.candidates.begin(),
        composed_non_dictionary.message.candidates.end(), "我是");
    const auto composed_index = static_cast<std::size_t>(
        composed - composed_non_dictionary.message.candidates.begin());
    if (!composed_non_dictionary.validation ||
        composed == composed_non_dictionary.message.candidates.end() ||
        composed_index >= composed_non_dictionary.message.candidate_groups.size() ||
        composed_non_dictionary.message.candidate_groups[composed_index] != 1) {
        std::cerr << "composed dictionary-area candidate contract failed\n";
        return 6;
    }
    const auto winning_head = std::find(
        winning_sentence_head.message.candidates.begin(),
        winning_sentence_head.message.candidates.end(), "和");
    const auto winning_head_index = static_cast<std::size_t>(
        winning_head - winning_sentence_head.message.candidates.begin());
    const auto first_dictionary = std::find(
        ambiguous_single_syllable.message.candidate_groups.begin(),
        ambiguous_single_syllable.message.candidate_groups.end(), 1);
    const auto single_syllable_partition_ok =
        ambiguous_single_syllable.validation &&
        ambiguous_single_syllable.message.candidates.size() == 7 &&
        ambiguous_single_syllable.message.candidate_groups.size() == 7 &&
        ambiguous_single_syllable.message.candidate_groups.front() == 2 &&
        first_dictionary != ambiguous_single_syllable.message.candidate_groups.end() &&
        std::all_of(ambiguous_single_syllable.message.candidate_groups.begin(),
                    first_dictionary,
                    [](const std::uint64_t group) { return group == 2; }) &&
        std::all_of(first_dictionary,
                    ambiguous_single_syllable.message.candidate_groups.end(),
                    [](const std::uint64_t group) { return group == 1; });
    if (!commits_ok || !custom_word_ack.validation ||
        custom_word_ack.message.type != owo::protocol::MessageType::acknowledgement ||
        !custom_word_candidates.validation ||
        custom_word_candidates.message.candidates.empty() ||
        custom_word_candidates.message.candidates.front() != "自定义词" ||
        !incomplete_custom_word_candidates.validation ||
        incomplete_custom_word_candidates.message.candidates.empty() ||
        incomplete_custom_word_candidates.message.candidates.front() != "自定义词" ||
        !missing_syllable_custom_word_candidates.validation ||
        missing_syllable_custom_word_candidates.message.candidates.empty() ||
        missing_syllable_custom_word_candidates.message.candidates.front() !=
            "自定义词" ||
        !valid_response(first, 101, 7) || !valid_response(second, 102, 8) ||
        !valid_response(learned, 104, 8, {"你号", "你好"}) ||
        !nihao_ranged.validation ||
        nihao_ranged.message.candidates !=
            std::vector<std::string>{"你好世界", "你好", "你号"} ||
        nihao_ranged.message.syllables !=
            std::vector<std::string>{"ni", "hao", "shi", "jie"} ||
        nihao_ranged.message.candidate_consumed !=
            std::vector<std::uint64_t>{14, 6, 6} ||
        !deferred_correction_base.validation ||
        deferred_correction_base.message.type !=
            owo::protocol::MessageType::candidate_response ||
        !deferred_correction_base.message.candidate_refinement_pending ||
        !deferred_correction_complete.validation ||
        deferred_correction_complete.message.candidate_refinement_pending ||
        deferred_correction_complete.message.candidates !=
            std::vector<std::string>{"你好", "你号"} ||
        !valid_response(corrected, 110, 8, {"你好", "你号"}, {"ni", "aho"}) ||
        !stable_xingb.validation ||
        stable_xingb.message.syllables != std::vector<std::string>{"xing", "b"} ||
        !stable_xingbaf.validation ||
        stable_xingbaf.message.syllables !=
            std::vector<std::string>{"xing", "ba", "f"} ||
        !stable_mingd.validation ||
        stable_mingd.message.syllables != std::vector<std::string>{"ming", "d"} ||
        !stable_kenengd.validation ||
        stable_kenengd.message.syllables !=
            std::vector<std::string>{"ke", "neng", "d"} ||
        !double_initial_first.validation ||
        double_initial_first.message.candidate_consumed !=
            std::vector<std::uint64_t>{2, 2, 2, 1, 1, 1, 1} ||
        double_initial_first.message.candidate_groups !=
            std::vector<std::uint64_t>{1, 1, 1, 2, 2, 2, 2} ||
        !double_initial_second.validation ||
        double_initial_second.message.candidate_consumed !=
            std::vector<std::uint64_t>{2, 2, 2, 2, 1, 1} ||
        double_initial_second.message.candidate_groups !=
            std::vector<std::uint64_t>{1, 1, 1, 1, 2, 2} ||
        !sentence_first.validation || sentence_first.message.candidate_groups.empty() ||
        sentence_first.message.candidate_groups.front() != 0 ||
        std::count(sentence_first.message.candidate_groups.begin(),
                   sentence_first.message.candidate_groups.end(), 0) > 2 ||
        !sentence_second.validation ||
        std::find(sentence_second.message.candidate_groups.begin(),
                  sentence_second.message.candidate_groups.end(), 0) !=
            sentence_second.message.candidate_groups.end() ||
        !winning_sentence_head.validation ||
        !single_syllable_partition_ok ||
        winning_sentence_head.message.candidates.empty() ||
        winning_head == winning_sentence_head.message.candidates.end() ||
        winning_head_index >= winning_sentence_head.message.candidate_groups.size() ||
        winning_sentence_head.message.candidate_groups[winning_head_index] != 2 ||
        std::find(winning_sentence_head.message.candidates.begin(), winning_head,
                  "合") != winning_head ||
        !correction_disabled.validation ||
        correction_disabled.message.type != owo::protocol::MessageType::candidate_response ||
        correction_disabled.message.correction_enabled ||
        !correction_disabled.message.candidates.empty() ||
        !separated_correction_disabled.validation ||
        separated_correction_disabled.message.type !=
            owo::protocol::MessageType::candidate_response ||
        separated_correction_disabled.message.correction_enabled ||
        !separated_correction_disabled.message.candidates.empty() ||
        separated_correction_disabled.message.syllables !=
            std::vector<std::string>{"quan", "loi"} ||
        !separated_correction_enabled.validation ||
        separated_correction_enabled.message.type !=
            owo::protocol::MessageType::candidate_response ||
        !separated_correction_enabled.message.correction_enabled ||
        !separated_correction_enabled.message.candidates.empty() ||
        separated_correction_enabled.message.syllables !=
            std::vector<std::string>{"quan", "loi"} ||
        !valid_response(first_page, 106, 8,
                        {"测试一", "测试二", "测试三", "测试四", "测试五", "测试六", "测试七"},
                        {"ce", "shi"}) ||
        first_page.message.candidate_groups !=
            std::vector<std::uint64_t>(7, 1) ||
        first_page.message.page != 0 || !first_page.message.has_more ||
        !valid_response(second_page, 105, 8, {"测试八", "测试九"}, {"ce", "shi"}) ||
        second_page.message.page != 1 || second_page.message.has_more ||
        !expanded.validation || !expanded.message.expanded ||
        expanded.message.page != 0 || expanded.message.page_size != 7 ||
        expanded.message.has_more ||
        expanded.message.candidates != paged_candidates ||
        expanded.message.candidates !=
            std::vector<std::string>{"测试一", "测试二", "测试三", "测试四",
                                     "测试五", "测试六", "测试七", "测试八", "测试九"} ||
        expanded.message.candidate_consumed !=
            std::vector<std::uint64_t>(9, 5) ||
        expanded.message.candidate_groups !=
            std::vector<std::uint64_t>(9, 1) ||
        !ranged.validation ||
        ranged.message.candidates != std::vector<std::string>{"我爱世界", "我爱", "我"} ||
        ranged.message.syllables !=
            std::vector<std::string>{"wo", "ai", "shi", "jie"} ||
        ranged.message.candidate_consumed !=
            std::vector<std::uint64_t>{13, 5, 2} ||
        !persisted_result.success || persisted.count("你号") != 5 ||
        !shutdown.validation || shutdown.message.type != owo::protocol::MessageType::acknowledgement ||
        shutdown.message.text != "shutdown_ack" ||
        shutdown.message.request_id != 103 || shutdown.message.context_generation != 9 ||
        server_exit != 0) {
        const auto dump = [](const char* name, const owo::protocol::DecodeResult& value) {
            std::cerr << name << ": valid=" << static_cast<bool>(value.validation)
                      << " page=" << value.message.page
                      << " page_size=" << value.message.page_size
                      << " more=" << value.message.has_more << " candidates=";
            for (const auto& candidate : value.message.candidates)
                std::cerr << '[' << candidate << ']';
            std::cerr << " consumed=";
            for (const auto consumed : value.message.candidate_consumed)
                std::cerr << consumed << ',';
            std::cerr << " groups=";
            for (const auto group : value.message.candidate_groups)
                std::cerr << group << ',';
            std::cerr << '\n';
        };
        dump("double-first", double_initial_first);
        dump("double-second", double_initial_second);
        dump("sentence-first", sentence_first);
        dump("sentence-second", sentence_second);
        dump("winning-sentence-head", winning_sentence_head);
        dump("ambiguous-single-syllable", ambiguous_single_syllable);
        dump("first-page", first_page);
        dump("second-page", second_page);
        dump("expanded", expanded);
        std::cerr << "in-process core service contract failed\n";
        return 1;
    }
    return 0;
}

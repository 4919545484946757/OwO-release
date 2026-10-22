#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <algorithm>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <thread>

namespace {

owo::protocol::DecodeResult send(const std::wstring& pipe, const owo::protocol::Message& message) {
    for (int attempt = 0; attempt < 100; ++attempt) {
        const auto exchanged = owo::ipc::exchange(
            pipe.c_str(), owo::protocol::encode_message(message), std::chrono::milliseconds(100));
        if (exchanged.status) return owo::protocol::decode_message(exchanged.response);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return {};
}

bool acknowledged(const owo::protocol::DecodeResult& result, std::string_view text) {
    return result.validation &&
           result.message.type == owo::protocol::MessageType::acknowledgement &&
           result.message.text == text;
}

}  // namespace

int wmain(int argc, wchar_t** argv) {
    if (argc != 2) return 2;
    const std::filesystem::path root(argv[1]);
    std::error_code ignored;
    std::filesystem::remove_all(root, ignored);
    std::filesystem::create_directories(root, ignored);
    const auto config_path = root / "owo.conf";
    const auto frequency_path = root / "user.bin";

    owo::config::ConfigStore writer;
    if (!writer.load(config_path).success) return 2;
    auto config = writer.snapshot();
    config.candidate_page_size = 3;
    config.user_learning_enabled = false;
    if (!writer.save(config).success) return 2;

    owo::config::ConfigMonitor monitor;
    if (!monitor.start(config_path, std::chrono::milliseconds(10)).success) return 2;
    owo::engine::UserFrequencyStore frequencies;
    if (!frequencies.load(frequency_path).success) return 2;
    frequencies.record("tianjiagengduofuhao", "添加更多符号", 2);
    const owo::engine::MemoryLexicon lexicon({
        {{"ce"}, "测", 1000}, {{"ce"}, "侧", 900},
        {{"ce"}, "策", 800}, {{"ce"}, "册", 700},
        {{"ce"}, "厕", 600}, {{"ce"}, "恻", 500},
        {{"ce"}, "栅", 400}, {{"ni", "hao"}, "你好", 1000},
        {{"ni", "hao"}, "泥号", 2000},
        {{"wang"}, "王", 3000}, {{"wang"}, "望", 2000},
        {{"wang"}, "妄", 200}, {{"kuang", "wang"}, "狂妄", 5000},
        {{"heng"}, "横", 5000}, {{"hen"}, "很", 10000000},
        {{"he", "ni", "ge"}, "和你哥", 9000},
        {{"hen", "neng", "gan"}, "很能干", 10000},
        {{"ga"}, "甲", 1200}, {{"de"}, "的", 1100},
        {{"ga", "de"}, "噶的", 1000}, {{"ge", "de"}, "各地", 900},
        {{"liang"}, "梁", 3000}, {{"liang"}, "凉", 2900},
        {{"liang"}, "两", 2800}, {{"liang"}, "亮", 2700},
        {{"liang"}, "良", 2600}, {{"liang"}, "量", 2500},
        {{"liang"}, "粮", 2400}, {{"liang"}, "晾", 2300},
        {{"liang"}, "谅", 2200}, {{"liang"}, "辆", 2100},
        {{"liang"}, "粱", 2000}, {{"liang"}, "踉", 1900},
        {{"liang"}, "椋", 1800}, {{"liang"}, "魉", 1700},
        {{"liang"}, "俩", 1600}, {{"liang"}, "悢", 1500},
        {{"li", "ang"}, "里昂", 1400}, {{"li", "ang"}, "丽昂", 1300},
        {{"li", "ang"}, "利昂", 1200}, {{"li", "ang"}, "力昂", 1100},
        {{"li", "ang"}, "立昂", 1000}, {{"li", "ang"}, "礼昂", 900},
        {{"li", "ang"}, "李昂", 800}, {{"li", "ang"}, "理昂", 700},
        {{"li", "ang"}, "历昂", 600}, {{"li", "ang"}, "俐昂", 500},
        {{"li", "ang"}, "栗昂", 400}, {{"li", "ang"}, "梨昂", 300},
        {{"chang"}, "长", 3000}, {{"chang"}, "常", 2900},
        {{"cha"}, "刹", 100000}, {{"na"}, "那", 100000},
        {{"ge"}, "个", 100000}, {{"cha", "na"}, "刹那", 90000},
        {{"niao"}, "鸟", 1000}});
    const auto suffix = std::chrono::steady_clock::now().time_since_epoch().count();
    const auto pipe = LR"(\\.\pipe\OwO.InputMethod.ConfigContract.)" + std::to_wstring(suffix);
    const auto model_pipe = LR"(\\.\pipe\OwO.InputMethod.ConfigModelContract.)" +
                            std::to_wstring(suffix);

    int server_exit = -1;
    std::jthread server([&] {
        server_exit = owo::ipc::run_core_server(pipe.c_str(), lexicon, &frequencies,
                                                model_pipe.c_str(), &monitor);
    });
    const auto model_disabled = send(pipe, {owo::protocol::MessageType::candidate_request,
                                            10, 1, "nihao"});
    const auto page_of_three = send(pipe, {owo::protocol::MessageType::candidate_request,
                                           20, 1, "ce"});
    const auto compact_double_initial = send(
        pipe, {owo::protocol::MessageType::candidate_request, 23, 1, "gd"});
    const auto complete_heng = send(
        pipe, {owo::protocol::MessageType::candidate_request, 26, 1, "heng"});
    owo::protocol::Message expanded_request{
        owo::protocol::MessageType::candidate_request, 22, 1, "ce"};
    expanded_request.expanded = true;
    const auto expanded = send(pipe, expanded_request);
    const auto disabled = send(pipe, {owo::protocol::MessageType::candidate_committed,
                                      1, 1, "你好"});
    const auto generation = monitor.generation();
    config.user_learning_enabled = true;
    config.model_ranking_enabled = true;
    config.model_timeout_ms = 5;
    const bool enabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation, std::chrono::seconds(2));
    owo::protocol::Message enabled_feedback{
        owo::protocol::MessageType::candidate_committed, 2, 1, "你好"};
    enabled_feedback.input = "NiHao";
    enabled_feedback.context = "我说";
    const auto enabled = send(pipe, enabled_feedback);
    owo::protocol::Message language_ranked_request{
        owo::protocol::MessageType::candidate_request, 13, 1, "nihao"};
    language_ranked_request.context = "我说";
    const auto language_ranked = send(pipe, language_ranked_request);
    owo::protocol::Message dictionary_context_request{
        owo::protocol::MessageType::candidate_request, 15, 1, "wang"};
    dictionary_context_request.input = "kuang";
    dictionary_context_request.context = "狂";
    const auto dictionary_context_ranked = send(pipe, dictionary_context_request);
    const auto model_enabled = send(pipe, {owo::protocol::MessageType::candidate_request,
                                           11, 1, "nihao"});
    owo::protocol::Message smart_expanded_request{
        owo::protocol::MessageType::candidate_request, 14, 1, "nihao"};
    smart_expanded_request.expanded = true;
    const auto smart_expanded = send(pipe, smart_expanded_request);
    const auto generation_after_enable = monitor.generation();
    config.model_ranking_enabled = false;
    config.candidate_page_size = 2;
    const bool disabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_after_enable, std::chrono::seconds(2));
    const auto model_disabled_again = send(
        pipe, {owo::protocol::MessageType::candidate_request, 12, 1, "nihao"});
    owo::protocol::Message page_request{owo::protocol::MessageType::candidate_request,
                                        21, 1, "ce"};
    page_request.page = 1;
    const auto page_of_two = send(pipe, page_request);
    const auto generation_before_partition = monitor.generation();
    config.candidate_page_size = 5;
    config.candidate_partition_enabled = true;
    const bool partition_enabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_partition, std::chrono::seconds(2));
    const auto partitioned = send(
        pipe, {owo::protocol::MessageType::candidate_request, 24, 1, "gd"});
    const auto partitioned_heng = send(
        pipe, {owo::protocol::MessageType::candidate_request, 27, 1, "heng"});
    bool single_first_pages_ok = true;
    std::uint64_t single_first_request_id = 500;
    for (const auto size : {4u, 5u, 7u, 9u}) for (const bool single_first : {true, false}) {
        const auto before = monitor.generation();
        config.candidate_page_size = size;
        config.candidate_single_character_first_enabled = single_first;
        single_first_pages_ok &= writer.save(config).success &&
            monitor.wait_for_generation(before, std::chrono::seconds(2));
        std::vector<std::string> paged_text;
        std::vector<std::uint64_t> paged_groups;
        std::size_t mixed_pages = 0;
        for (std::uint64_t page = 0; page < 16; ++page) {
            owo::protocol::Message request{owo::protocol::MessageType::candidate_request,
                ++single_first_request_id, 500, "liang"};
            request.page = page;
            const auto result = send(pipe, request);
            const auto& message = result.message;
            single_first_pages_ok &= result.validation && message.page == page &&
                message.page_size == size && !message.candidates.empty() &&
                message.candidate_groups.size() == message.candidates.size();
            const auto singles = std::count(message.candidate_groups.begin(), message.candidate_groups.end(), 2);
            const auto words = std::count(message.candidate_groups.begin(), message.candidate_groups.end(), 1);
            if (singles && words) {
                ++mixed_pages;
                single_first_pages_ok &= std::is_sorted(message.candidate_groups.begin(),
                    message.candidate_groups.end(), [single_first](auto left, auto right) {
                        return single_first ? left > right : left < right;
                    });
                if (page < 2)
                    single_first_pages_ok &= singles == (size + 1) / 2 && words == size / 2;
            }
            paged_text.insert(paged_text.end(), message.candidates.begin(), message.candidates.end());
            paged_groups.insert(paged_groups.end(), message.candidate_groups.begin(), message.candidate_groups.end());
            if (!message.has_more) break;
        }
        owo::protocol::Message expanded_single{owo::protocol::MessageType::candidate_request,
            ++single_first_request_id, 500, "liang"};
        expanded_single.expanded = true;
        const auto all = send(pipe, expanded_single);
        single_first_pages_ok &= mixed_pages >= 3 && all.validation &&
            all.message.candidates == paged_text && all.message.candidate_groups == paged_groups;
        owo::protocol::Message repaired{owo::protocol::MessageType::candidate_request,
            ++single_first_request_id, 500, "chnag"};
        repaired.expanded = true;
        const auto repaired_all = send(pipe, repaired);
        const auto retains = [&](std::string_view text, std::uint64_t consumed,
                                 std::uint64_t group) {
            const auto& value = repaired_all.message;
            for (std::size_t index = 0; index < value.candidates.size(); ++index)
                if (value.candidates[index] == text &&
                    index < value.candidate_consumed.size() && value.candidate_consumed[index] == consumed &&
                    index < value.candidate_groups.size() && value.candidate_groups[index] == group)
                    return true;
            return false;
        };
        single_first_pages_ok &= repaired_all.validation &&
            !repaired_all.message.candidate_groups.empty() &&
            repaired_all.message.candidate_groups.front() == (single_first ? 2 : 1) &&
            retains("长", 5, 2) && retains("刹那", 4, 1) && retains("刹那个", 5, 0);
        repaired.request_id = ++single_first_request_id;
        repaired.text = "niaho";
        const auto multi_syllable_repair = send(pipe, repaired);
        single_first_pages_ok &= multi_syllable_repair.validation &&
            !multi_syllable_repair.message.candidate_groups.empty() &&
            multi_syllable_repair.message.candidate_groups.front() == 1;
    }
    config.candidate_page_size = 5;
    const auto generation_before_legacy = monitor.generation();
    config.candidate_partition_enabled = false;
    const bool legacy_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_legacy, std::chrono::seconds(2));
    const auto legacy_layout = send(
        pipe, {owo::protocol::MessageType::candidate_request, 25, 1, "gd"});
    owo::protocol::Message legacy_repair_request{owo::protocol::MessageType::candidate_request,
        ++single_first_request_id, 500, "chnag"};
    legacy_repair_request.expanded = true;
    const auto legacy_repair = send(pipe, legacy_repair_request);
    const auto completion_default = send(
        pipe, {owo::protocol::MessageType::candidate_request, 30, 1, "tianjiage"});
    const auto generation_before_completion = monitor.generation();
    config.user_lexicon_completion_max_syllables = 3;
    const bool completion_range_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_completion,
                                    std::chrono::seconds(2));
    const auto completion_extended = send(
        pipe, {owo::protocol::MessageType::candidate_request, 31, 1, "tianjiage"});
    const auto generation_before_completion_disable = monitor.generation();
    config.user_lexicon_completion_enabled = false;
    const bool completion_disabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_completion_disable,
                                    std::chrono::seconds(2));
    const auto completion_disabled = send(
        pipe, {owo::protocol::MessageType::candidate_request, 32, 1, "tianjiage"});
    const auto exact_learned_word = send(
        pipe, {owo::protocol::MessageType::candidate_request, 33, 1,
               "tianjiagengduofuhao"});
    const auto generation_before_scheme = monitor.generation();
    config.input_scheme = "exact_code";
    const bool exact_code_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_scheme, std::chrono::seconds(2));
    const auto exact_code_candidates = send(
        pipe, {owo::protocol::MessageType::candidate_request, 28, 1, "ni'hao"});
    const auto exact_code_without_abbreviation = send(
        pipe, {owo::protocol::MessageType::candidate_request, 29, 1, "gd"});
    const auto shutdown = send(pipe, {owo::protocol::MessageType::shutdown_request,
                                      3, 1, {}});
    server.join();
    if (!single_first_pages_ok) {
        std::cerr << "single-character-first per-page/configuration/expanded contract failed\n";
        return 3;
    }
    if (!legacy_repair.validation || legacy_repair.message.candidates.empty() ||
        legacy_repair.message.candidates.front() != "长" ||
        std::find(legacy_repair.message.candidates.begin(), legacy_repair.message.candidates.end(),
                  "刹那个") == legacy_repair.message.candidates.end()) {
        std::cerr << "legacy correction/original-segmentation coexistence failed\n";
        return 4;
    }

    owo::engine::UserFrequencyStore persisted;
    const auto loaded = persisted.load(frequency_path);
    const bool ok = enabled_reload && disabled_reload && partition_enabled_reload &&
                    legacy_reload && exact_code_reload &&
                    completion_range_reload && completion_disabled_reload &&
                    acknowledged(disabled, "commit_ack") &&
                    acknowledged(enabled, "commit_ack") &&
                    language_ranked.validation &&
                    !language_ranked.message.candidates.empty() &&
                    language_ranked.message.candidates.front() == "你好" &&
                    dictionary_context_ranked.validation &&
                    !dictionary_context_ranked.message.candidates.empty() &&
                    dictionary_context_ranked.message.candidates.front() == "妄" &&
                    acknowledged(shutdown, "shutdown_ack") && server_exit == 0 &&
                    model_disabled.validation && !model_disabled.message.model_pending &&
                    page_of_three.validation && page_of_three.message.candidates.size() == 3 &&
                    page_of_three.message.page_size == 3 && page_of_three.message.has_more &&
                    compact_double_initial.validation &&
                    compact_double_initial.message.page_size == 3 &&
                    compact_double_initial.message.candidate_groups.empty() &&
                    complete_heng.validation &&
                    complete_heng.message.syllables ==
                        std::vector<std::string>{"heng"} &&
                    expanded.validation && expanded.message.expanded &&
                    expanded.message.page_size == 3 && expanded.message.candidates.size() == 7 &&
                    !expanded.message.has_more &&
                    model_enabled.validation && model_enabled.message.model_pending &&
                    smart_expanded.validation && smart_expanded.message.expanded &&
                    smart_expanded.message.model_pending &&
                    model_disabled_again.validation &&
                    !model_disabled_again.message.model_pending &&
                    page_of_two.validation && page_of_two.message.page == 1 &&
                    page_of_two.message.candidates ==
                        std::vector<std::string>{"策", "册"} &&
                    page_of_two.message.has_more &&
                    partitioned.validation &&
                    !partitioned.message.candidates.empty() &&
                    partitioned.message.candidate_groups.size() ==
                        partitioned.message.candidates.size() &&
                    partitioned_heng.validation &&
                    partitioned_heng.message.syllables ==
                        std::vector<std::string>{"heng"} &&
                    !partitioned_heng.message.candidate_consumed.empty() &&
                    partitioned_heng.message.candidate_consumed.front() == 4 &&
                    legacy_layout.validation &&
                    !legacy_layout.message.candidates.empty() &&
                    legacy_layout.message.candidate_groups.empty() &&
                    completion_default.validation &&
                    std::find(completion_default.message.candidates.begin(),
                              completion_default.message.candidates.end(),
                              "添加更多符号") == completion_default.message.candidates.end() &&
                    completion_extended.validation &&
                    std::find(completion_extended.message.candidates.begin(),
                              completion_extended.message.candidates.end(),
                              "添加更多符号") != completion_extended.message.candidates.end() &&
                    completion_disabled.validation &&
                    std::find(completion_disabled.message.candidates.begin(),
                              completion_disabled.message.candidates.end(),
                              "添加更多符号") == completion_disabled.message.candidates.end() &&
                    exact_learned_word.validation &&
                    std::find(exact_learned_word.message.candidates.begin(),
                              exact_learned_word.message.candidates.end(),
                              "添加更多符号") != exact_learned_word.message.candidates.end() &&
                    exact_code_candidates.validation &&
                    !exact_code_candidates.message.candidates.empty() &&
                    exact_code_candidates.message.candidates.front() == "你好" &&
                    exact_code_candidates.message.syllables ==
                        std::vector<std::string>{"ni", "hao"} &&
                    exact_code_without_abbreviation.validation &&
                    exact_code_without_abbreviation.message.candidates.empty() &&
                    loaded.success && persisted.count("你好") == 1 &&
                    persisted.contextual_count("nihao", "你好") == 1 &&
                    persisted.language_context_score("我说", "nihao", "你好") > 0;
    std::filesystem::remove_all(root, ignored);
    if (!ok) {
        std::cerr << "core hot configuration contract failed\n";
        return 1;
    }
    return 0;
}

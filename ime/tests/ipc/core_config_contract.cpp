#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/user_corpus.h"
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
    const auto corpus_path = root / "user-corpus.owuc";

    // Seed an authorized corpus before Core starts. Two observations make the
    // continuation stable; the application matches the allowed list below. The
    // timestamps are current so retention pruning keeps the seed.
    {
        const auto now = static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::milliseconds>(
                std::chrono::system_clock::now().time_since_epoch()).count());
        const auto seeded = owo::engine::UserCorpusStore::modify(
            corpus_path, [now](owo::engine::UserCorpusStore& store) {
                (void)store.append("WeChat.exe", "老师我们这周把流程跑通了", now);
                (void)store.append("WeChat.exe", "老师我们这周把流程跑通了", now + 1);
            });
        if (!seeded.success) return 2;
    }

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
        {{"ga", "de"}, "噶的", 1000}, {{"ge", "de"}, "各地", 900}});
    const auto suffix = std::chrono::steady_clock::now().time_since_epoch().count();
    const auto pipe = LR"(\\.\pipe\OwO.InputMethod.ConfigContract.)" + std::to_wstring(suffix);
    const auto model_pipe = LR"(\\.\pipe\OwO.InputMethod.ConfigModelContract.)" +
                            std::to_wstring(suffix);

    int server_exit = -1;
    std::jthread server([&] {
        server_exit = owo::ipc::run_core_server(pipe.c_str(), lexicon, &frequencies,
                                                model_pipe.c_str(), &monitor, nullptr,
                                                nullptr, &corpus_path);
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
    const auto generation_before_legacy = monitor.generation();
    config.candidate_partition_enabled = false;
    const bool legacy_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_legacy, std::chrono::seconds(2));
    const auto legacy_layout = send(
        pipe, {owo::protocol::MessageType::candidate_request, 25, 1, "gd"});
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
    // Authorized corpus learning is off by default: eligible sentences from an
    // allowed application must not be recorded before the user opts in.
    auto corpus_entry = [](const std::uint64_t id, std::string text) {
        owo::protocol::Message message{owo::protocol::MessageType::agent_context_recorded,
                                       id, 1, std::move(text)};
        message.application_context_id = "ctx-corpus-" + std::to_string(id);
        message.application_executable = "WeChat.exe";
        message.corpus_eligible = true;
        return message;
    };
    const auto corpus_while_off = send(pipe, corpus_entry(41, "默认关闭时不该记住这句话。"));
    const auto generation_before_corpus = monitor.generation();
    config.corpus_learning_enabled = true;
    config.corpus_learning_allowed_apps = {"WeChat.exe"};
    // Prediction follows the ordinary pinyin page as an extra candidate.
    config.candidate_partition_enabled = true;
    config.input_scheme = "full_pinyin";
    const bool corpus_enabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_corpus, std::chrono::seconds(2));
    const auto corpus_allowed = send(pipe, corpus_entry(42, "开启后记住这句话。"));
    owo::protocol::Message prediction_request{
        owo::protocol::MessageType::candidate_request, 50, 1, "ba"};
    prediction_request.context = "老师我们这周";
    const auto predicted = send(pipe, prediction_request);
    // The learned continuation is visible during normal pinyin input, but the
    // typed reading keeps the numbered positions: Space and "1"/"2" still
    // select pinyin candidates.
    const auto& predicted_candidates = predicted.message.candidates;
    const bool prediction_ok = predicted.validation &&
        !predicted_candidates.empty() &&
        predicted_candidates.back() == "把流程跑通了" &&
        predicted.message.candidate_groups.size() == predicted_candidates.size() &&
        predicted.message.candidate_groups.back() ==
            static_cast<std::uint64_t>(owo::protocol::CandidateGroup::continuation) &&
        std::all_of(predicted.message.candidate_groups.begin(),
                    predicted.message.candidate_groups.end() - 1,
                    [](const std::uint64_t group) {
                        return group != static_cast<std::uint64_t>(
                                            owo::protocol::CandidateGroup::continuation);
                    }) &&
        predicted.message.candidate_consumed.size() == predicted_candidates.size() &&
        predicted.message.candidate_consumed.back() == 2;
    // The same reading without learned evidence shows no extra entry.
    owo::protocol::Message no_prediction_request{
        owo::protocol::MessageType::candidate_request, 51, 1, "ba"};
    no_prediction_request.context = "今天下午";
    const auto not_predicted = send(pipe, no_prediction_request);
    // Turning the separate prediction switch off removes it while recording
    // stays on.
    const auto generation_before_prediction_off = monitor.generation();
    config.corpus_prediction_enabled = false;
    const bool prediction_disabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_prediction_off,
                                    std::chrono::seconds(2));
    owo::protocol::Message disabled_prediction_request{
        owo::protocol::MessageType::candidate_request, 52, 1, "ba"};
    disabled_prediction_request.context = "老师我们这周";
    const auto prediction_disabled = send(pipe, disabled_prediction_request);
    const auto generation_before_prediction_on = monitor.generation();
    config.corpus_prediction_enabled = true;
    const bool prediction_enabled_reload = writer.save(config).success &&
        monitor.wait_for_generation(generation_before_prediction_on,
                                    std::chrono::seconds(2));
    owo::protocol::Message restored_prediction_request{
        owo::protocol::MessageType::candidate_request, 53, 1, "ba"};
    restored_prediction_request.context = "老师我们这周";
    const auto prediction_restored = send(pipe, restored_prediction_request);
    // Deleting the learned sentences also removes the prediction: the Core
    // rebuilds its statistics from the corpus file instead of keeping a copy.
    const auto deleted = owo::engine::UserCorpusStore::modify(
        corpus_path, [](owo::engine::UserCorpusStore& store) {
            (void)store.erase(1);
            (void)store.erase(2);
        });
    owo::protocol::Message deleted_prediction_request{
        owo::protocol::MessageType::candidate_request, 54, 1, "ba"};
    deleted_prediction_request.context = "老师我们这周";
    const auto prediction_deleted = send(pipe, deleted_prediction_request);
    auto disallowed = corpus_entry(43, "未授权的应用不该记住。");
    disallowed.application_executable = "other.exe";
    const auto corpus_disallowed = send(pipe, disallowed);
    auto sensitive = corpus_entry(44, "密码框里的话不该记住。");
    sensitive.application_sensitive_input = true;
    const auto corpus_sensitive = send(pipe, sensitive);
    auto ineligible = corpus_entry(45, "Agent 回写的内容不该记住。");
    ineligible.corpus_eligible = false;
    const auto corpus_ineligible = send(pipe, ineligible);
    const auto shutdown = send(pipe, {owo::protocol::MessageType::shutdown_request,
                                      3, 1, {}});
    server.join();
    owo::engine::UserCorpusStore corpus;
    const auto corpus_loaded = corpus.load(corpus_path);
    // The two seeded observations were deleted during the run; only the entry
    // recorded after opt-in remains, written by the shutdown flush.
    const bool corpus_ok = corpus_loaded.success && corpus.entries().size() == 1 &&
        corpus.entries().front().text == "开启后记住这句话。" &&
        corpus.entries().front().application == "WeChat.exe";

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
                    persisted.language_context_score("我说", "nihao", "你好") > 0 &&
                    acknowledged(corpus_while_off, "agent_context_ack") &&
                    acknowledged(corpus_allowed, "agent_context_ack") &&
                    acknowledged(corpus_disallowed, "agent_context_ack") &&
                    // A sensitive context entry is refused by validation and can
                    // never carry corpus eligibility.
                    corpus_sensitive.validation &&
                    corpus_sensitive.message.type ==
                        owo::protocol::MessageType::error_response &&
                    acknowledged(corpus_ineligible, "agent_context_ack") &&
                    corpus_enabled_reload && prediction_ok &&
                    predicted.message.syllables ==
                        std::vector<std::string>{"ba"} &&
                    not_predicted.validation &&
                    std::find(not_predicted.message.candidates.begin(),
                              not_predicted.message.candidates.end(),
                              "把流程跑通了") == not_predicted.message.candidates.end() &&
                    prediction_disabled_reload &&
                    std::find(prediction_disabled.message.candidates.begin(),
                              prediction_disabled.message.candidates.end(),
                              "把流程跑通了") ==
                        prediction_disabled.message.candidates.end() &&
                    prediction_enabled_reload &&
                    !prediction_restored.message.candidates.empty() &&
                    prediction_restored.message.candidates.back() == "把流程跑通了" &&
                    deleted.success && prediction_deleted.validation &&
                    std::find(prediction_deleted.message.candidates.begin(),
                              prediction_deleted.message.candidates.end(),
                              "把流程跑通了") ==
                        prediction_deleted.message.candidates.end() &&
                    corpus_ok;
    std::filesystem::remove_all(root, ignored);
    if (!ok) {
        std::cerr << "core hot configuration contract failed\n";
        return 1;
    }
    return 0;
}

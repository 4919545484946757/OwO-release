#include "owo/engine/collocation.h"

#include <iostream>
#include <string>

namespace {

using owo::engine::UserCollocationModel;

}  // namespace

int main() {
    int failures = 0;
    {
        UserCollocationModel model;
        if (model.continuation("老师我们这周") != "" || !model.empty()) {
            std::cerr << "empty model predicted something\n";
            ++failures;
        }
    }
    // A phrase seen once is not enough; the second observation makes the
    // continuation stable.
    {
        UserCollocationModel model;
        model.observe("老师我们这周把流程跑通了。");
        if (!model.continuation("老师我们这周").empty()) {
            std::cerr << "single observation was promoted\n";
            ++failures;
        }
        model.observe("老师我们这周把流程跑通了。");
        const auto prediction = model.continuation("老师我们这周");
        if (prediction != "把流程跑通了") {
            std::cerr << "unstable collocation prediction: " << prediction << "\n";
            ++failures;
        }
    }
    // The prediction stops before sentence punctuation.
    {
        UserCollocationModel model;
        model.observe("明天组会几点？");
        model.observe("明天组会几点？");
        const auto prediction = model.continuation("明天组会");
        if (prediction != "几点") {
            std::cerr << "punctuation leaked into continuation: " << prediction << "\n";
            ++failures;
        }
        // After a sentence end there is no continuation at all.
        if (!model.continuation("明天组会几点？").empty()) {
            std::cerr << "predicted beyond a sentence boundary\n";
            ++failures;
        }
    }
    // The longest context wins over a shorter one that sees different text.
    {
        UserCollocationModel model;
        for (int repeat = 0; repeat < 3; ++repeat) {
            model.observe("我已经把报告发出去了");
            model.observe("他们报告还没写");
        }
        const auto after_prefix = model.continuation("我已经把报告");
        if (after_prefix != "发出去了") {
            std::cerr << "long context was not preferred: " << after_prefix << "\n";
            ++failures;
        }
        if (model.continuation("报告") != "还没写" &&
            model.continuation("报告") != "发出去了") {
            std::cerr << "context without a unique predecessor failed\n";
            ++failures;
        }
    }
    // A short history still resolves through a one-character context.
    {
        UserCollocationModel model;
        model.observe("跑通了以后再说");
        model.observe("跑通了以后再说");
        const auto prediction = model.continuation("跑");
        if (prediction != "通了以后再说") {
            std::cerr << "short history continuation failed: " << prediction << "\n";
            ++failures;
        }
    }
    // Records are bounded and never split a UTF-8 sequence.
    {
        UserCollocationModel model;
        const std::string sentence = "这是一个很长的句子用来测试预测长度上限是否会被正确截断处理";
        model.observe(sentence);
        model.observe(sentence);
        const auto prediction = model.continuation("这是一个");
        if (prediction.empty() ||
            prediction.size() > owo::engine::kMaximumCollocationContinuationBytes) {
            std::cerr << "length bound failed: " << prediction.size() << "\n";
            ++failures;
        }
        for (std::size_t index = 0; index < prediction.size(); ++index) {
            if (prediction[index] == '\0') break;
        }
        if (prediction.size() % 3 != 0) {
            std::cerr << "continuation split a UTF-8 sequence: " << prediction << "\n";
            ++failures;
        }
    }
    // Clearing drops every statistic.
    {
        UserCollocationModel model;
        model.observe("清理以后不该有预测");
        model.observe("清理以后不该有预测");
        if (model.continuation("清理以后").empty()) {
            std::cerr << "setup observation missing\n";
            ++failures;
        }
        model.clear();
        if (!model.empty() || !model.continuation("清理以后").empty() ||
            model.sentence_count() != 0 || model.context_count() != 0) {
            std::cerr << "clear did not reset the model\n";
            ++failures;
        }
    }
    // Rebuilding from corpus entries matches feeding the same sentences.
    {
        std::vector<owo::engine::UserCorpusEntry> entries{
            {1, 0, "WeChat.exe", "这个功能下周上线"},
            {2, 0, "WeChat.exe", "这个功能下周上线"}};
        const auto model = owo::engine::build_user_collocation_model(entries);
        if (model.sentence_count() != 2 ||
            model.continuation("这个功能") != "下周上线") {
            std::cerr << "corpus rebuild failed\n";
            ++failures;
        }
    }
    // A corpus at the entry limit only feeds the newest sentences, so a
    // rebuild after a flush stays short.
    {
        std::vector<owo::engine::UserCorpusEntry> entries(
            owo::engine::kMaximumCollocationCorpusEntries + 10);
        for (std::size_t index = 0; index < entries.size(); ++index) {
            entries[index] = {index + 1, 0, "WeChat.exe",
                              index < 10 ? "旧的句子不该参与统计" : "新的句子参与统计"};
        }
        const auto model = owo::engine::build_user_collocation_model(entries);
        if (model.sentence_count() != owo::engine::kMaximumCollocationCorpusEntries ||
            model.continuation("旧的") != "" ||
            model.continuation("新的") != "句子参与统计") {
            std::cerr << "corpus entry bound failed\n";
            ++failures;
        }
    }
    if (failures == 0) std::cout << "collocation tests passed\n";
    return failures == 0 ? 0 : 1;
}

#include "owo/tsf/composition_learning.h"
#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/user_frequency.h"

#include <algorithm>
#include <iostream>

int main() {
    using owo::tsf::CompositionLearningChain;
    CompositionLearningChain chain;
    if (chain.append(L"自动", L"zi'dong", L"之前", false))
        return 1;
    const auto first = chain.append(L"学习", L"xuexi", L"之前自动", true);
    if (!first || first->text != L"自动学习" || first->input != L"zidongxuexi" ||
        first->context != L"之前" || !chain.empty())
        return 2;

    // A fresh composition must not inherit the previous completed sentence.
    if (chain.append(L"董", L"dong", L"新位置", false) ||
        chain.append(L"柯", L"ke", L"新位置董", false))
        return 3;
    const auto name = chain.append(L"伊", L"yi", L"新位置董柯", true);
    if (!name || name->text != L"董柯伊" || name->input != L"dongkeyi" ||
        name->context != L"新位置" || !chain.empty())
        return 4;
    for (const auto boundary : {"cancel", "focus-loss", "backspace-empty", "raw-commit", "agent"}) {
        static_cast<void>(boundary);
        if (chain.append(L"旧片段", L"jiupianduan", L"旧位置", false))
            return 5;
        chain.clear();
        const auto next = chain.append(L"用户学习", L"yonghuxuexi", L"新位置", true);
        if (!next || next->text != L"用户学习" || next->input != L"yonghuxuexi" ||
            next->context != L"新位置" || !chain.empty())
            return 6;
    }
    // TSF snapshots the chain before a successful final commit clears preedit.
    if (chain.append(L"董", L"dong", L"姓名", false))
        return 7;
    auto snapshot = chain;
    chain.clear();
    if (snapshot.append(L"柯", L"ke", L"姓名董", false))
        return 8;
    const auto completed = snapshot.append(L"伊", L"yi", L"姓名董柯", true);
    if (!completed || completed->text != L"董柯伊" || !snapshot.empty())
        return 9;
    chain = std::move(snapshot);
    if (!chain.empty())
        return 10;

    // The emitted mapping is independently recalled, including incomplete input.
    owo::engine::UserFrequencyStore frequency;
    frequency.record("dongkeyi", "董柯伊");
    const owo::engine::MemoryLexicon lexicon({{{"dong"}, "动", 50000},
                                              {{"dong"}, "董", 500},
                                              {{"ke", "yi"}, "可以", 50000},
                                              {{"ke"}, "柯", 500},
                                              {{"yi"}, "伊", 500}});
    const owo::engine::CandidateGenerator generator(lexicon, nullptr, &frequency);
    const owo::engine::FullPinyinSchema schema;
    for (const auto input : {"dongkeyi", "dongkey"}) {
        for (const auto page_size : {0U, 7U}) {
            const auto candidates = generator.generate(schema.parse(input, 32, false), 48, true, {},
                                                       nullptr, {}, true, page_size);
            if (std::none_of(candidates.begin(), candidates.end(),
                             [](const auto &candidate) { return candidate.text == "董柯伊"; })) {
                std::cerr << "Completed user phrase was not recalled: " << input << '\n';
                return 11;
            }
        }
    }
    std::cout << "Composition-local learning, resets and independent user-word recall passed\n";
}

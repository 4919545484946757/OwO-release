#include "owo/engine/candidate_generator.h"
#include "owo/engine/full_pinyin_schema.h"
#include <algorithm>
#include <iostream>

int main() {
    const owo::engine::FullPinyinSchema schema;
    std::vector<owo::engine::LexiconEntry> entries{
        {{"hai", "chan"}, "海产", 1000000}, {{"hai", "cha"}, "还差", 500000},
        {{"diao", "chan"}, "貂蝉", 1000000}, {{"diao", "cha"}, "调查", 500000},
        {{"na", "xie"}, "哪些", 500000}, {{"na", "ge"}, "哪个", 500000},
        {{"hai"}, "还", 1000000}, {{"hai"}, "海", 900000}, {{"diao"}, "调", 1000000}};
    for (const auto text : {"啊", "阿", "吖", "嗄", "锕", "呵", "腌", "嗳", "埃", "哀", "挨", "唉", "哎", "皑", "癌", "蔼"})
        entries.push_back({{"a"}, text, 1000000});
    for (const auto text : {"些", "谢", "写", "血", "鞋", "歇", "斜", "邪"})
        entries.push_back({{"xie"}, text, 1000000});
    for (const auto text : {"个", "各", "歌", "格", "隔", "哥", "割", "革"})
        entries.push_back({{"ge"}, text, 1000000});
    const owo::engine::MemoryLexicon lexicon(std::move(entries));
    const owo::engine::CandidateGenerator generator(lexicon);
    struct Case { std::string input; std::string expected; std::vector<std::string> reading; };
    for (const auto& item : std::vector<Case>{
            {"haichanaxie", "还差哪些", {"hai", "cha", "na", "xie"}},
            {"haichanage", "还差哪个", {"hai", "cha", "na", "ge"}},
            {"diaochanaxie", "调查哪些", {"diao", "cha", "na", "xie"}}}) {
        owo::engine::FullPinyinIncrementalState state;
        for (std::size_t length = 1; length <= item.input.size(); ++length)
            static_cast<void>(schema.parse_incremental(std::string_view(item.input).substr(0, length), 32, state));
        for (const bool incremental : {false, true}) {
            const auto parsed = incremental ? state.result : schema.parse(item.input, 32, false);
            for (const auto limit : {8U, 18U, 48U, 128U}) {
                for (const bool model : {false, true}) {
                    const auto result = generator.generate(parsed, limit, false, {}, nullptr, {}, model, 7);
                    if (result.empty() || result.front().text != item.expected || result.front().source_segments != item.reading) {
                        std::cerr << "Boundary lost: " << item.input << " limit=" << limit << " model=" << model << " incremental=" << incremental << '\n';
                        if (!result.empty()) std::cerr << "actual=" << result.front().text << '\n';
                        return 1;
                    }
                    if (result.front().consumed_input_bytes != item.input.size()) return 2;
                }
            }
        }
    }
    const auto explicit_result = generator.generate(schema.parse("hai'chan'a'xie", 32, false), 48, false, {}, nullptr, {}, true, 7);
    if (explicit_result.empty() || explicit_result.front().source_segments != std::vector<std::string>{"hai", "chan", "a", "xie"}) return 3;
    std::cout << "Ambiguous boundary budget, model, incremental and explicit-boundary tests passed\n";
    return 0;
}

#include "owo/engine/binary_lexicon.h"

#include <filesystem>
#include <fstream>
#include <iostream>

int main() {
    const auto first = std::filesystem::temp_directory_path() / "owo-lexicon-test-1.bin";
    const auto second = std::filesystem::temp_directory_path() / "owo-lexicon-test-2.bin";
    std::vector<owo::engine::LexiconEntry> entries{
        {{"xi", "an"}, "西安", 900}, {{"ni", "hao"}, "你好", 1000},
        {{"ni", "hao"}, "你号", 50}, {{"ni"}, "你", 1200}};
    entries.push_back({{"bu", "gan", "dang"}, "BGD", 2000});
    entries.push_back({{"na"}, "NA", 1100});
    entries.push_back({{"ne"}, "NE", 1000});
    entries.push_back({{"nong"}, "NONG", 900});
    entries.push_back({{"wei", "shen", "me"}, "为什么", 3000});
    entries.push_back({{"ni", "jiu"}, "你就", 2500});
    entries.push_back({{"jie", "zhe"}, "接着", 2400});
    if (!owo::engine::write_binary_lexicon(first, entries).success ||
        !owo::engine::write_binary_lexicon(second, {entries.rbegin(), entries.rend()}).success) return 1;

    std::ifstream left(first, std::ios::binary), right(second, std::ios::binary);
    const std::string left_bytes((std::istreambuf_iterator<char>(left)), {});
    const std::string right_bytes((std::istreambuf_iterator<char>(right)), {});
    if (left_bytes != right_bytes) { std::cerr << "output is not deterministic\n"; return 1; }

    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(first).success || lexicon.size() != 11 ||
        lexicon.maximum_reading_length() != 3) return 1;
    const std::string_view reading[]{"ni", "hao"};
    const auto matches = lexicon.lookup(reading);
    if (matches.size() != 2 || matches[0].text != "你好" || matches[1].text != "你号" ||
        !lexicon.exact_lookup_frequency_ordered()) return 1;
    const auto text_matches = lexicon.lookup_texts(reading);
    if (text_matches.size() != 2 || text_matches[0].text != "你好" ||
        text_matches[0].frequency != 1000 || text_matches[1].text != "你号" ||
        text_matches[1].frequency != 50 ||
        lexicon.lookup_max_frequency(reading) != 1000) return 1;
    const auto exact_first = lexicon.lookup_bounded(reading, 0, 1, 1);
    const auto exact_second = lexicon.lookup_bounded(
        reading, exact_first.next_index, 1, 1);
    if (exact_first.entries.size() != 1 || exact_first.exhausted ||
        exact_first.entries.front().text != "你好" ||
        exact_second.entries.size() != 1 || !exact_second.exhausted ||
        exact_second.entries.front().text != "你号") return 1;
    if (lexicon.exact_frequency_upper_bound(reading, 0) != std::uint32_t{1000} ||
        lexicon.exact_frequency_upper_bound(reading, 1) != std::uint32_t{50} ||
        lexicon.exact_frequency_upper_bound(reading, 2).has_value()) return 1;
    const std::string_view missing_reading[]{"bu", "cun", "zai"};
    if (!lexicon.lookup(missing_reading).empty() ||
        !lexicon.lookup_texts(missing_reading).empty() ||
        lexicon.lookup_max_frequency(missing_reading) != 0) return 1;
    const auto initials = lexicon.lookup_initial('n');
    if (initials.size() != 4 || initials.front().text != "你" ||
        !lexicon.lookup_initial('x').empty()) return 1;
    const auto initial_texts = lexicon.lookup_initial_texts('n');
    if (initial_texts.size() != 4 || initial_texts.front().text != "你" ||
        initial_texts.front().syllable != "ni" ||
        initial_texts.front().frequency != 1200 ||
        !lexicon.lookup_initial_texts('x').empty()) return 1;
    const auto initial_first = lexicon.lookup_initial_bounded('n', 0, 2, 2);
    const auto initial_second = lexicon.lookup_initial_bounded(
        'n', initial_first.next_index, 2, 2);
    if (initial_first.entries.size() != 2 || initial_first.exhausted ||
        initial_first.work_units_consumed != 2 || initial_first.next_index != 2 ||
        initial_second.entries.size() != 2 || !initial_second.exhausted ||
        initial_second.work_units_consumed != 2 ||
        initial_first.entries.front().text != "你" ||
        initial_second.entries.front().text != "NE")
        return 1;

    const auto abbreviated = lexicon.lookup_mixed_abbreviation("bugd", 8);
    if (abbreviated.size() != 1 || abbreviated.front().entry.text != "BGD" ||
        abbreviated.front().source_segments !=
            std::vector<std::string>{"bu", "g", "d"}) return 1;
    const auto abbreviated_texts =
        lexicon.lookup_mixed_abbreviation_texts("bugd", 8);
    if (abbreviated_texts.size() != 1 ||
        abbreviated_texts.front().text != "BGD" ||
        abbreviated_texts.front().syllables !=
            std::vector<std::string_view>{"bu", "gan", "dang"} ||
        abbreviated_texts.front().frequency != 2000 ||
        abbreviated_texts.front().source_segments !=
            std::vector<std::string>{"bu", "g", "d"}) return 1;
    const auto all_initials = lexicon.lookup_mixed_abbreviation_texts("wsm", 8);
    const auto leading_initial = lexicon.lookup_mixed_abbreviation_texts("nji", 8);
    const auto mixed_prefix = lexicon.lookup_mixed_abbreviation_texts("jzhe", 8);
    if (all_initials.empty() || all_initials.front().text != "为什么" ||
        all_initials.front().source_segments !=
            std::vector<std::string>{"w", "s", "m"} ||
        leading_initial.empty() || leading_initial.front().text != "你就" ||
        leading_initial.front().source_segments !=
            std::vector<std::string>{"n", "ji"} ||
        mixed_prefix.empty() || mixed_prefix.front().text != "接着" ||
        mixed_prefix.front().source_segments !=
            std::vector<std::string>{"j", "zhe"}) return 1;
    const auto double_initial = lexicon.lookup_mixed_abbreviation("nh", 8);
    if (double_initial.size() != 2 ||
        double_initial.front().source_segments !=
            std::vector<std::string>{"n", "h"} ||
        double_initial.front().entry.syllables !=
            std::vector<std::string>{"ni", "hao"}) return 1;

    auto corrupted = left_bytes;
    // Without an installer-created validation cache, V2 verifies its payload
    // checksum and rejects content corruption.
    corrupted.back() ^= 1;
    { std::ofstream output(second, std::ios::binary | std::ios::trunc); output.write(corrupted.data(), static_cast<std::streamsize>(corrupted.size())); }
    owo::engine::BinaryLexicon rejected;
    if (rejected.load(second).success) { std::cerr << "corruption was accepted\n"; return 1; }
    std::error_code ignored;
    std::filesystem::remove(first, ignored);
    std::filesystem::remove(second, ignored);
    return 0;
}

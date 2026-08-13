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
    if (!owo::engine::write_binary_lexicon(first, entries).success ||
        !owo::engine::write_binary_lexicon(second, {entries.rbegin(), entries.rend()}).success) return 1;

    std::ifstream left(first, std::ios::binary), right(second, std::ios::binary);
    const std::string left_bytes((std::istreambuf_iterator<char>(left)), {});
    const std::string right_bytes((std::istreambuf_iterator<char>(right)), {});
    if (left_bytes != right_bytes) { std::cerr << "output is not deterministic\n"; return 1; }

    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(first).success || lexicon.size() != 8 ||
        lexicon.maximum_reading_length() != 3) return 1;
    const std::string_view reading[]{"ni", "hao"};
    const auto matches = lexicon.lookup(reading);
    if (matches.size() != 2 || matches[0].text != "你好" || matches[1].text != "你号" ||
        !lexicon.exact_lookup_frequency_ordered()) return 1;
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
    if (!lexicon.lookup(missing_reading).empty()) return 1;
    const auto initials = lexicon.lookup_initial('n');
    if (initials.size() != 4 || initials.front().text != "你" ||
        !lexicon.lookup_initial('x').empty()) return 1;
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

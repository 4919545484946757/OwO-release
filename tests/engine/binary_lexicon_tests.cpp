#include "owo/engine/binary_lexicon.h"

#include <filesystem>
#include <fstream>
#include <iostream>
#include <algorithm>
#include <chrono>

int main(int argc, char** argv) {
    if (argc == 3) {
        owo::engine::BinaryLexicon before, after;
        if (!before.load(argv[1]).success || !after.load(argv[2]).success ||
            before.size() != after.size() || !after.exact_lookup_frequency_ordered() ||
            before.maximum_reading_length() != after.maximum_reading_length()) return 40;
        std::vector<std::vector<std::string>> samples;
        {
            const auto rows = before.materialize_entries();
            for (std::size_t i = 0; i < rows.size(); i += 4096) samples.push_back(rows[i].syllables);
        }
        std::vector<std::vector<std::string_view>> readings;
        for (const auto& sample : samples) {
            readings.emplace_back();
            for (const auto& syllable : sample) readings.back().push_back(syllable);
            auto old = before.lookup_texts(readings.back()), updated = after.lookup_texts(readings.back());
            const auto order = [](const auto& a, const auto& b) { return a.text != b.text ? a.text < b.text : a.frequency < b.frequency; };
            std::sort(old.begin(), old.end(), order); std::sort(updated.begin(), updated.end(), order);
            if (old.size() != updated.size()) return 41;
            for (std::size_t j = 0; j < old.size(); ++j)
                if (old[j].text != updated[j].text || old[j].frequency != updated[j].frequency) return 42;
            if (before.lookup_max_frequency(readings.back()) != after.lookup_max_frequency(readings.back())) return 43;
        }
        const auto benchmark = [&](const auto& lexicon) {
            const auto started = std::chrono::steady_clock::now();
            std::uint64_t sum = 0;
            for (int repeat = 0; repeat < 100; ++repeat)
                for (const auto& reading : readings) sum += lexicon.lookup_max_frequency(reading);
            const auto elapsed = std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - started).count();
            return std::pair{sum, elapsed};
        };
        const auto old = benchmark(before), updated = benchmark(after);
        if (old.first != updated.first) return 44;
        std::cout << "{\"entries\":" << after.size() << ",\"sampled_readings\":" << readings.size()
            << ",\"iterations\":100,\"v2_max_frequency_us\":" << old.second << ",\"v3_max_frequency_us\":" << updated.second << "}\n";
        return 0;
    }
    const auto empty_path = std::filesystem::temp_directory_path() / "owo-lexicon-empty.bin";
    if (!owo::engine::write_binary_lexicon(empty_path, {}).success) return 30;
    {
        owo::engine::BinaryLexicon empty;
        const std::string_view key[]{"ni"};
        if (!empty.load(empty_path).success || empty.size() || empty.maximum_reading_length() ||
            !empty.lookup_texts(key).empty() || !empty.lookup_initial_texts('n').empty() ||
            !empty.lookup_mixed_abbreviation_texts("nh", 7).empty()) return 31;
    }
    std::filesystem::remove(empty_path);
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
    entries.push_back({{"zhong"}, "重", 4000});
    entries.push_back({{"chong"}, "重", 3000});
    if (!owo::engine::write_binary_lexicon(first, entries).success ||
        !owo::engine::write_binary_lexicon(second, {entries.rbegin(), entries.rend()}).success) return 1;

    std::ifstream left(first, std::ios::binary), right(second, std::ios::binary);
    const std::string left_bytes((std::istreambuf_iterator<char>(left)), {});
    const std::string right_bytes((std::istreambuf_iterator<char>(right)), {});
    if (left_bytes != right_bytes) { std::cerr << "output is not deterministic\n"; return 1; }

    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.storage_path().empty()) return 2;
    if (!lexicon.load(first).success || lexicon.size() != entries.size() ||
        lexicon.maximum_reading_length() != 3) return 1;
    const std::string_view words[]{"你好", "重", "西安", "不存在", "重"};
    const auto reverse = lexicon.lookup_readings(words);
    if (reverse.size() != 4 || reverse[0].text != "你好" || reverse[0].syllables != std::vector<std::string>{"ni", "hao"} ||
        reverse[1].text != "重" || reverse[1].syllables != std::vector<std::string>{"zhong"} ||
        reverse[2].syllables != std::vector<std::string>{"chong"} || reverse[3].text != "西安" ||
        !lexicon.lookup_readings(words, 0).empty() || !lexicon.lookup_readings(words, 9).empty() ||
        lexicon.lookup_readings(words, 1).size() != 3) return 11;
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
    const auto initial_file = std::filesystem::temp_directory_path() / "owo-lexicon-initial-priority.bin";
    std::vector<owo::engine::LexiconEntry> initial_entries{{{"xiang", "fa"}, "想法", 5000}};
    for (unsigned index = 0; index < 300; ++index)
        initial_entries.push_back({{"xi", "fu"}, "短读音" + std::to_string(index), 1000});
    const owo::engine::MemoryLexicon initial_memory(initial_entries);
    if (!owo::engine::write_binary_lexicon(initial_file, initial_entries).success) return 8;
    owo::engine::BinaryLexicon initial_binary;
    if (!initial_binary.load(initial_file).success) return 8;
    for (const auto* source : std::vector<const owo::engine::Lexicon*>{&initial_memory, &initial_binary}) {
        const auto views = source->lookup_mixed_abbreviation_texts("xf", 1);
        const auto owned = source->lookup_mixed_abbreviation("xf", 1);
        if (views.size() != 1 || owned.size() != 1 || views[0].text != "想法" || owned[0].entry.text != "想法" ||
            views[0].source_segments != std::vector<std::string>{"x", "f"}) return 9;
        const std::vector<std::string_view> complete_reading{"xi", "fu"};
        const auto complete = source->lookup_texts(complete_reading);
        if (complete.size() != 300 || complete[0].frequency != 1000) return 10;
    }
    // Without an installer-created validation cache, V2 verifies its payload
    // checksum and rejects content corruption.
    corrupted.back() ^= 1;
    { std::ofstream output(second, std::ios::binary | std::ios::trunc); output.write(corrupted.data(), static_cast<std::streamsize>(corrupted.size())); }
    owo::engine::BinaryLexicon rejected;
    if (rejected.load(second).success) { std::cerr << "corruption was accepted\n"; return 1; }
    if (!rejected.storage_path().empty() || lexicon.storage_path() != std::filesystem::absolute(first)) return 2;
    std::error_code ignored;
    std::filesystem::remove(first, ignored);
    std::filesystem::remove(second, ignored);
    return 0;
}

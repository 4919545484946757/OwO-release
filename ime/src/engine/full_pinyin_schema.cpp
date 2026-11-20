#include "owo/engine/full_pinyin_schema.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <chrono>
#include <cstdlib>
#include <functional>
#include <iterator>
#include <limits>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>

namespace owo::engine {
namespace {

// Canonical tone-less Hanyu Pinyin syllables. Interjections and common
// orthographic forms used by dictionaries are included; tone digits are not.
constexpr std::string_view kSyllables[] = {
    "a", "ai", "an", "ang", "ao", "ba", "bai", "ban", "bang", "bao", "bei", "ben", "beng", "bi", "bian", "biao", "bie", "bin", "bing", "bo", "bu",
    "ca", "cai", "can", "cang", "cao", "ce", "cen", "ceng", "cha", "chai", "chan", "chang", "chao", "che", "chen", "cheng", "chi", "chong", "chou", "chu", "chua", "chuai", "chuan", "chuang", "chui", "chun", "chuo", "ci", "cong", "cou", "cu", "cuan", "cui", "cun", "cuo",
    "da", "dai", "dan", "dang", "dao", "de", "dei", "den", "deng", "di", "dia", "dian", "diao", "die", "ding", "diu", "dong", "dou", "du", "duan", "dui", "dun", "duo",
    "e", "ei", "en", "eng", "er",
    "fa", "fan", "fang", "fei", "fen", "feng", "fiao", "fo", "fou", "fu",
    "ga", "gai", "gan", "gang", "gao", "ge", "gei", "gen", "geng", "gong", "gou", "gu", "gua", "guai", "guan", "guang", "gui", "gun", "guo",
    "ha", "hai", "han", "hang", "hao", "he", "hei", "hen", "heng", "hm", "hng", "hong", "hou", "hu", "hua", "huai", "huan", "huang", "hui", "hun", "huo",
    "ji", "jia", "jian", "jiang", "jiao", "jie", "jin", "jing", "jiong", "jiu", "ju", "juan", "jue", "jun",
    "ka", "kai", "kan", "kang", "kao", "ke", "kei", "ken", "keng", "kong", "kou", "ku", "kua", "kuai", "kuan", "kuang", "kui", "kun", "kuo",
    "la", "lai", "lan", "lang", "lao", "le", "lei", "leng", "li", "lia", "lian", "liang", "liao", "lie", "lin", "ling", "liu", "lo", "long", "lou", "lu", "luan", "lun", "luo", "lv", "lve",
    "m", "ma", "mai", "man", "mang", "mao", "me", "mei", "men", "meng", "mi", "mian", "miao", "mie", "min", "ming", "miu", "mo", "mou", "mu",
    "n", "na", "nai", "nan", "nang", "nao", "ne", "nei", "nen", "neng", "ng", "ni", "nian", "niang", "niao", "nie", "nin", "ning", "niu", "nong", "nou", "nu", "nuan", "nun", "nuo", "nv", "nve",
    "o", "ou",
    "pa", "pai", "pan", "pang", "pao", "pei", "pen", "peng", "pi", "pian", "piao", "pie", "pin", "ping", "po", "pou", "pu",
    "qi", "qia", "qian", "qiang", "qiao", "qie", "qin", "qing", "qiong", "qiu", "qu", "quan", "que", "qun",
    "ran", "rang", "rao", "re", "ren", "reng", "ri", "rong", "rou", "ru", "rua", "ruan", "rui", "run", "ruo",
    "sa", "sai", "san", "sang", "sao", "se", "sen", "seng", "sha", "shai", "shan", "shang", "shao", "she", "shei", "shen", "sheng", "shi", "shou", "shu", "shua", "shuai", "shuan", "shuang", "shui", "shun", "shuo", "si", "song", "sou", "su", "suan", "sui", "sun", "suo",
    "ta", "tai", "tan", "tang", "tao", "te", "teng", "ti", "tian", "tiao", "tie", "ting", "tong", "tou", "tu", "tuan", "tui", "tun", "tuo",
    "wa", "wai", "wan", "wang", "wei", "wen", "weng", "wo", "wu",
    "xi", "xia", "xian", "xiang", "xiao", "xie", "xin", "xing", "xiong", "xiu", "xu", "xuan", "xue", "xun",
    "ya", "yan", "yang", "yao", "ye", "yi", "yin", "ying", "yo", "yong", "you", "yu", "yuan", "yue", "yun",
    "za", "zai", "zan", "zang", "zao", "ze", "zei", "zen", "zeng", "zha", "zhai", "zhan", "zhang", "zhao", "zhe", "zhei", "zhen", "zheng", "zhi", "zhong", "zhou", "zhu", "zhua", "zhuai", "zhuan", "zhuang", "zhui", "zhun", "zhuo", "zi", "zong", "zou", "zu", "zuan", "zui", "zun", "zuo"};

std::optional<std::string_view> canonical_complete_syllable(
    const std::string_view value) {
    const auto found = std::find(std::begin(kSyllables), std::end(kSyllables), value);
    if (found != std::end(kSyllables)) return *found;
    // On an ASCII keyboard, u is the conventional spelling of ü after l/n.
    // Dictionaries retain the unambiguous v spelling internally.
    if (value == "lue") return "lve";
    if (value == "nue") return "nve";
    // A non-leading standalone v is the keyboard spelling of ü/yu. Treat it
    // as a complete source segment so appending v cannot invalidate and
    // repartition an already valid pinyin prefix. A leading v is reserved by
    // TSF for Agent mode and is removed before parsing.
    if (value == "v") return "yu";
    return std::nullopt;
}

bool is_complete(const std::string_view value) {
    return canonical_complete_syllable(value).has_value();
}

bool is_prefix(const std::string_view value) {
    return std::any_of(std::begin(kSyllables), std::end(kSyllables),
                       [value](const std::string_view syllable) {
                           return syllable.size() > value.size() && syllable.starts_with(value);
                       });
}

// Last-resort readings used only when neither exact, incomplete, abbreviated,
// nor corrected parsing can consume the whole input.  Keeping this fallback
// deterministic makes arbitrary long letter sequences recoverable without
// perturbing the ranking of valid pinyin input.
std::string_view fallback_syllable(const char value) {
    switch (value) {
        case 'a': return "a";
        case 'b': return "ba";
        case 'c': return "ci";
        case 'd': return "de";
        case 'e': return "e";
        case 'f': return "fa";
        case 'g': return "ge";
        case 'h': return "he";
        case 'i': return "yi";
        case 'j': return "ji";
        case 'k': return "ke";
        case 'l': return "le";
        case 'm': return "ma";
        case 'n': return "ni";
        case 'o': return "o";
        case 'p': return "pa";
        case 'q': return "qi";
        case 'r': return "ren";
        case 's': return "shi";
        case 't': return "ta";
        case 'u': return "wu";
        case 'v': return "yu";
        case 'w': return "wo";
        case 'x': return "xi";
        case 'y': return "yi";
        case 'z': return "zi";
        default: return {};
    }
}

struct ChunkPath {
    std::vector<Syllable> syllables;
    bool incomplete{};
    InputMatchKind match_kind{InputMatchKind::exact};
    std::uint32_t completion_characters{};
};

std::size_t unlikely_internal_vowel_syllables(const ChunkPath& path) {
    // Unseparated a-family syllables are common intentional boundaries:
    // wanan -> wan'an, nanan -> nan'an, xingan -> xing'an. Penalising every
    // vowel-leading segment moved the final n/ng to the following syllable.
    // Standalone e/o-family readings are uncommon in compact multi-syllable
    // input, however, and retaining a small penalty for them preserves
    // keneng -> ke'neng instead of ken'eng.
    constexpr std::array<std::string_view, 7> unlikely{
        "e", "ei", "en", "eng", "er", "o", "ou"};
    std::size_t count = 0;
    for (std::size_t index = 1; index < path.syllables.size(); ++index) {
        const auto& text = path.syllables[index].text;
        if (std::find(unlikely.begin(), unlikely.end(), text) != unlikely.end())
            ++count;
    }
    return count;
}

bool chunk_path_less(const ChunkPath& left, const ChunkPath& right) {
    if (left.incomplete != right.incomplete) return !left.incomplete;
    if (left.syllables.size() != right.syllables.size())
        return left.syllables.size() < right.syllables.size();
    const auto left_unlikely = unlikely_internal_vowel_syllables(left);
    const auto right_unlikely = unlikely_internal_vowel_syllables(right);
    if (left_unlikely != right_unlikely) return left_unlikely < right_unlikely;
    return false;
}

bool assisted_path_less(const ParsePath& left, const ParsePath& right) {
    if (left.syllables.size() != right.syllables.size())
        return left.syllables.size() < right.syllables.size();
    if (left.completion_characters != right.completion_characters)
        return left.completion_characters < right.completion_characters;
    return std::lexicographical_compare(
        left.syllables.begin(), left.syllables.end(), right.syllables.begin(), right.syllables.end(),
        [](const Syllable& first, const Syllable& second) { return first.text < second.text; });
}

std::string assisted_reading_key(const ParsePath& path) {
    std::size_t size = 1;
    for (const auto& syllable : path.syllables) size += syllable.text.size() + 1;
    std::string key(1, static_cast<char>(path.match_kind));
    key.reserve(size);
    for (const auto& syllable : path.syllables) {
        key += syllable.text;
        key.push_back('\'');
    }
    return key;
}

ParsePath extend_path(const ParsePath& source, Syllable syllable) {
    ParsePath result;
    result.match_kind = source.match_kind;
    result.edit_count = source.edit_count;
    result.completion_characters = source.completion_characters;
    // Copy construction followed by push_back allocated/copies the entire
    // syllable vector twice for virtually every assisted transition.
    result.syllables.reserve(source.syllables.size() + 1);
    result.syllables.insert(result.syllables.end(), source.syllables.begin(), source.syllables.end());
    result.syllables.push_back(std::move(syllable));
    return result;
}

struct KeyboardPosition {
    int row{};
    int column{};
};

std::optional<KeyboardPosition> keyboard_position(const char key) {
    constexpr std::string_view rows[]{"qwertyuiop", "asdfghjkl", "zxcvbnm"};
    for (int row = 0; row < 3; ++row) {
        const auto column = rows[row].find(key);
        if (column != std::string_view::npos)
            return KeyboardPosition{row, static_cast<int>(column) * 2 + row};
    }
    return std::nullopt;
}

bool substitution_allowed(const char expected, const char actual) {
    if ((expected == 'l' && actual == 'n') || (expected == 'n' && actual == 'l') ||
        (expected == 'f' && actual == 'h') || (expected == 'h' && actual == 'f')) return true;
    const auto left = keyboard_position(expected);
    const auto right = keyboard_position(actual);
    return left && right && std::abs(left->row - right->row) <= 1 &&
           std::abs(left->column - right->column) <= 2;
}
// O(1) membership and prefix tests for the plausibility sweep. They are built
// once from the same table the parser uses, including the keyboard spellings
// canonical_complete_syllable() accepts.
bool is_known_syllable(const std::string_view value) {
    static const auto table = [] {
        std::unordered_set<std::string_view> result;
        result.reserve(512);
        for (const auto syllable : kSyllables) result.insert(syllable);
        result.insert("lue");
        result.insert("nue");
        result.insert("v");
        return result;
    }();
    return table.contains(value);
}

bool is_known_prefix(const std::string_view value) {
    static const auto table = [] {
        std::unordered_set<std::string_view> result;
        for (const auto syllable : kSyllables)
            for (std::size_t size = 1; size < syllable.size(); ++size)
                result.insert(syllable.substr(0, size));
        return result;
    }();
    return table.contains(value);
}

const std::vector<std::string_view>& syllable_prefix_matches(
    const std::string_view prefix) {
    static const auto table = [] {
        std::unordered_map<std::string, std::vector<std::string_view>> result;
        for (const auto syllable : kSyllables) {
            for (std::size_t size = 1; size <= syllable.size(); ++size)
                result[std::string(syllable.substr(0, size))].push_back(syllable);
        }
        return result;
    }();
    static const std::vector<std::string_view> empty;
    const auto found = table.find(std::string(prefix));
    return found == table.end() ? empty : found->second;
}

void append_incomplete_completions(const std::vector<ParsePath>& base_paths,
                                   std::vector<ParsePath>& paths,
                                   const std::size_t max_paths,
                                   const std::size_t inclusive_syllable_limit) {
    std::vector<ParsePath> completions;
    for (const auto& base : base_paths) {
        if (completions.size() >= max_paths) break;
        if (base.syllables.size() > inclusive_syllable_limit) continue;
        std::vector<std::size_t> incomplete;
        for (std::size_t index = 0; index < base.syllables.size(); ++index) {
            if (!base.syllables[index].complete) incomplete.push_back(index);
        }
        if (incomplete.empty()) continue;
        ParsePath current = base;
        current.match_kind = InputMatchKind::incomplete_completion;
        current.completion_characters = 0;
        std::function<void(std::size_t)> expand = [&](const std::size_t position) {
            if (completions.size() >= max_paths) return;
            if (position == incomplete.size()) {
                completions.push_back(current);
                return;
            }
            const auto index = incomplete[position];
            const auto prefix = base.syllables[index].text;
            for (const auto syllable : syllable_prefix_matches(prefix)) {
                if (syllable.size() <= prefix.size()) continue;
                current.syllables[index].text = std::string(syllable);
                current.syllables[index].complete = true;
                current.completion_characters +=
                    static_cast<std::uint32_t>(syllable.size() - prefix.size());
                expand(position + 1);
                current.completion_characters -=
                    static_cast<std::uint32_t>(syllable.size() - prefix.size());
                if (completions.size() >= max_paths) break;
            }
            current.syllables[index] = base.syllables[index];
        };
        expand(0);
    }
    std::sort(completions.begin(), completions.end(), assisted_path_less);
    for (auto& completion : completions) {
        if (paths.size() >= max_paths) break;
        paths.push_back(std::move(completion));
    }
}

void prune_assisted_paths(std::vector<ParsePath>& paths, const std::size_t limit,
                          const bool deduplicate = false) {
    struct KeyedPath { ParsePath path; std::string key; };
    std::vector<KeyedPath> keyed;
    keyed.reserve(paths.size());
    for (auto& path : paths) {
        auto key = assisted_reading_key(path);
        keyed.push_back({std::move(path), std::move(key)});
    }
    std::sort(keyed.begin(), keyed.end(), [](const auto& left, const auto& right) {
        if (left.path.syllables.size() != right.path.syllables.size())
            return left.path.syllables.size() < right.path.syllables.size();
        if (left.path.completion_characters != right.path.completion_characters)
            return left.path.completion_characters < right.path.completion_characters;
        // Canonical syllables contain a-z only. Apostrophe sorts before those
        // letters, exactly matching vector-of-syllables lexicographic order.
        // Match kind is part of dedup identity, NOT the original sort order.
        return std::string_view(left.key).substr(1) < std::string_view(right.key).substr(1);
    });
    if (!deduplicate) {
        paths.clear();
        for (std::size_t index = 0; index < std::min(keyed.size(), limit); ++index)
            paths.push_back(std::move(keyed[index].path));
        return;
    }
    std::vector<ParsePath> deduplicated;
    deduplicated.reserve(paths.size());
    std::unordered_set<std::string_view> seen;
    seen.reserve(paths.size() * 2);
    for (auto& item : keyed) {
        if (!seen.insert(item.key).second) continue;
        deduplicated.push_back(std::move(item.path));
    }
    if (deduplicated.size() <= limit) {
        paths = std::move(deduplicated);
        return;
    }

    // Keep the best half by the normal assisted-path score, then reserve room
    // for distinct completion families. A strict prefix cut lets variants of
    // one short completion consume the whole budget before the lexicon can
    // evaluate useful alternatives (for example, shqiyishi used to lose
    // sheng'qi'yi'shi behind sha/she variants).
    const auto completion_family = [](const ParsePath& path) {
        for (std::size_t index = 0; index < path.syllables.size(); ++index) {
            const auto& syllable = path.syllables[index];
            if (syllable.text.size() <= syllable.end - syllable.begin) continue;
            return std::to_string(index) + ':' + syllable.text;
        }
        return path.syllables.empty()
                   ? std::string{}
                   : std::string{"exact:"} + path.syllables.front().text;
    };
    const auto primary_family = [](const ParsePath& path) {
        for (std::size_t index = 0; index < path.syllables.size(); ++index) {
            const auto& syllable = path.syllables[index];
            if (syllable.text.size() <= syllable.end - syllable.begin) continue;
            return std::to_string(index) + ':' + syllable.text;
        }
        return std::string{};
    };

    std::vector<std::size_t> selected_indices;
    selected_indices.reserve(limit);
    std::vector<bool> selected(deduplicated.size(), false);
    constexpr auto kind_count =
        static_cast<std::size_t>(InputMatchKind::abbreviated_completion) + 1;
    std::array<std::vector<std::size_t>, kind_count> groups;
    for (std::size_t index = 0; index < deduplicated.size(); ++index)
        groups[static_cast<std::size_t>(deduplicated[index].match_kind)].push_back(index);
    const auto active_groups = static_cast<std::size_t>(std::count_if(
        groups.begin(), groups.end(), [](const auto& group) { return !group.empty(); }));
    std::size_t active_index = 0;
    for (const auto& group : groups) {
        if (group.empty()) continue;
        const auto quota = limit / active_groups +
                           (active_index++ < limit % active_groups ? 1 : 0);
        // Most of the quota is reserved for distinct completion families;
        // nearby variants of the highest-ranked family only need a small
        // leading slice.
        const auto leading_count = std::min(group.size(), (quota + 3) / 4);
        std::size_t selected_in_group = 0;
        std::unordered_set<std::string> families;
        families.reserve(quota * 2);
        const auto select = [&](const std::size_t index) {
            selected[index] = true;
            selected_indices.push_back(index);
            ++selected_in_group;
            families.insert(completion_family(deduplicated[index]));
        };
        for (std::size_t position = 0; position < leading_count; ++position)
            select(group[position]);
        // First guarantee one representative for every alternative at the
        // earliest completed input segment. Later-segment variants must not
        // crowd out sheng merely because they share the earlier sha family.
        std::unordered_set<std::string> primary_families;
        primary_families.reserve(quota * 2);
        for (std::size_t position = 0; position < leading_count; ++position)
            primary_families.insert(primary_family(deduplicated[group[position]]));
        for (std::size_t position = leading_count;
             position < group.size() && selected_in_group < quota; ++position) {
            const auto index = group[position];
            if (!primary_families.insert(primary_family(deduplicated[index])).second) continue;
            select(index);
        }
        for (std::size_t position = leading_count;
             position < group.size() &&
             selected_in_group < quota; ++position) {
            const auto index = group[position];
            if (families.insert(completion_family(deduplicated[index])).second) {
                selected[index] = true;
                selected_indices.push_back(index);
            }
        }
        for (const auto index : group) {
            if (selected_in_group >= quota) break;
            if (!selected[index]) select(index);
        }
    }
    for (std::size_t index = 0;
         index < deduplicated.size() && selected_indices.size() < limit; ++index) {
        if (selected[index]) continue;
        selected[index] = true;
        selected_indices.push_back(index);
    }

    std::sort(selected_indices.begin(), selected_indices.end());
    std::vector<ParsePath> selected_paths;
    selected_paths.reserve(limit);
    for (const auto index : selected_indices)
        selected_paths.push_back(std::move(deduplicated[index]));
    std::sort(selected_paths.begin(), selected_paths.end(), assisted_path_less);
    paths = std::move(selected_paths);
}

void append_abbreviated_completions(const std::string_view normalized,
                                    std::vector<ParsePath>& paths,
                                    const std::size_t max_paths,
                                    const std::function<bool()>& cancelled) {
    constexpr std::size_t kMinimumAbbreviatedInputBytes = 2;
    constexpr std::size_t kMaximumAbbreviatedInputBytes = 256;
    constexpr std::size_t kMaximumAbbreviatedSyllables = 256;
    if (normalized.size() < kMinimumAbbreviatedInputBytes ||
        normalized.size() > kMaximumAbbreviatedInputBytes ||
        normalized.find('\'') != std::string_view::npos || max_paths == 0) return;

    const auto beam_width = normalized.size() > 32
                                ? std::size_t{32}
                                : std::max<std::size_t>(
                                      32, std::min<std::size_t>(128, max_paths * 4));
    std::vector<std::vector<ParsePath>> chart(normalized.size() + 1);
    ParsePath initial;
    initial.match_kind = InputMatchKind::abbreviated_completion;
    chart[0].push_back(std::move(initial));

    for (std::size_t offset = 0; offset < normalized.size(); ++offset) {
        if (cancelled && cancelled()) return;
        if (chart[offset].empty()) continue;
        prune_assisted_paths(chart[offset], beam_width, true);
        struct Token { std::string_view syllable; std::size_t consumed; };
        std::vector<Token> tokens;
        const auto maximum = std::min<std::size_t>(6, normalized.size() - offset);
        for (std::size_t consumed = maximum; consumed > 0; --consumed)
            for (const auto syllable : syllable_prefix_matches(normalized.substr(offset, consumed)))
                tokens.push_back({syllable, consumed});
        for (const auto& state : chart[offset]) {
            if (cancelled && cancelled()) return;
            if (state.syllables.size() >= kMaximumAbbreviatedSyllables) continue;
            for (const auto& [syllable, consumed] : tokens) {
                auto next = extend_path(state, {std::string(syllable), offset,
                                                offset + consumed, true});
                next.completion_characters +=
                    static_cast<std::uint32_t>(syllable.size() - consumed);
                auto& destination = chart[offset + consumed];
                destination.push_back(std::move(next));
                if (destination.size() >= beam_width * 4)
                    prune_assisted_paths(destination, beam_width, true);
            }
        }
    }

    auto& completed = chart.back();
    completed.erase(std::remove_if(completed.begin(), completed.end(), [](const ParsePath& path) {
        return path.syllables.size() < 2 || path.completion_characters == 0;
    }), completed.end());
    // Keep a wider internal completion pool until all assisted match kinds are
    // merged. The public result is still capped later; early truncation here
    // prevents the final diversity pass from ever seeing useful syllable
    // families near the middle of the completion alphabet.
    prune_assisted_paths(completed, std::max<std::size_t>(max_paths, 128), true);
    for (auto& path : completed) paths.push_back(std::move(path));
}

struct FuzzyToken {
    std::string_view syllable;
    std::size_t consumed{};
    bool corrected{};
};

const std::unordered_map<std::string, std::vector<FuzzyToken>>& fuzzy_token_table() {
    static const auto table = [] {
        std::unordered_map<std::string, std::vector<FuzzyToken>> result;
        const auto add = [&result](std::string source, const std::string_view syllable,
                                   const bool corrected) {
            if (source.empty()) return;
            result[std::move(source)].push_back(
                {syllable, 0, corrected});
        };
        for (const auto syllable : kSyllables) {
            add(std::string(syllable), syllable, false);
            for (std::size_t index = 0; index < syllable.size(); ++index) {
                auto deleted = std::string(syllable);
                deleted.erase(index, 1);
                add(std::move(deleted), syllable, true);
                for (char actual = 'a'; actual <= 'z'; ++actual) {
                    if (actual == syllable[index] ||
                        !substitution_allowed(syllable[index], actual)) continue;
                    auto substituted = std::string(syllable);
                    substituted[index] = actual;
                    add(std::move(substituted), syllable, true);
                }
                if (index + 1 < syllable.size() &&
                    syllable[index] != syllable[index + 1]) {
                    auto transposed = std::string(syllable);
                    std::swap(transposed[index], transposed[index + 1]);
                    add(std::move(transposed), syllable, true);
                }
            }
            for (std::size_t index = 0; index <= syllable.size(); ++index) {
                for (char inserted = 'a'; inserted <= 'z'; ++inserted) {
                    auto source = std::string(syllable);
                    source.insert(source.begin() + static_cast<std::ptrdiff_t>(index), inserted);
                    add(std::move(source), syllable, true);
                }
            }
        }
        for (auto& [source, matches] : result) {
            for (auto& match : matches) match.consumed = source.size();
            std::sort(matches.begin(), matches.end(), [](const auto& left, const auto& right) {
                if (left.corrected != right.corrected) return !left.corrected;
                return left.syllable < right.syllable;
            });
            matches.erase(std::unique(matches.begin(), matches.end(), [](const auto& left,
                                                                         const auto& right) {
                return left.syllable == right.syllable &&
                       left.corrected == right.corrected;
            }), matches.end());
        }
        return result;
    }();
    return table;
}

std::vector<FuzzyToken> fuzzy_tokens(const std::string_view input,
                                     const std::size_t offset) {
    std::vector<FuzzyToken> matches;
    const auto remaining = input.size() - offset;
    const auto& table = fuzzy_token_table();
    const auto maximum = std::min<std::size_t>(7, remaining);
    for (std::size_t consumed = maximum; consumed > 0; --consumed) {
        const auto found = table.find(std::string(input.substr(offset, consumed)));
        if (found == table.end()) continue;
        matches.insert(matches.end(), found->second.begin(), found->second.end());
    }
    return matches;
}

void append_corrected_paths(const std::string_view normalized,
                            std::vector<ParsePath>& paths,
                            const std::size_t max_paths,
                            const std::size_t exclusive_syllable_limit,
                            const std::function<bool()>& cancelled) {
    constexpr std::size_t kMaximumCorrectedInputBytes = 128;
    constexpr std::size_t kMinimumCorrectedInputBytes = 3;
    if (normalized.size() < kMinimumCorrectedInputBytes ||
        normalized.size() > kMaximumCorrectedInputBytes ||
        normalized.find('\'') != std::string_view::npos || paths.size() >= max_paths ||
        exclusive_syllable_limit <= 1) return;
    // Fuzzy token discovery depends only on the source offset. Cache it and
    // use a bounded chart rather than recursively enumerating every exact
    // segmentation before the single corrected token. Long inputs otherwise
    // become exponential even though only a handful of paths are returned.
    std::vector<std::optional<std::vector<FuzzyToken>>> token_cache(normalized.size());
    const auto tokens_at = [&](const std::size_t offset) -> const std::vector<FuzzyToken>& {
        auto& cached = token_cache[offset];
        if (!cached.has_value()) cached = fuzzy_tokens(normalized, offset);
        return *cached;
    };
    struct CorrectionState {
        ParsePath path;
        std::uint32_t edit_count{};
        std::size_t first_edit_offset{};
        std::string reading_key;
    };
    const std::uint32_t maximum_edits = normalized.size() >= 12 ? 2U : 1U;
    const auto beam_width = normalized.size() >= 12
        ? std::max<std::size_t>(64, std::min<std::size_t>(128, max_paths * 8))
        : std::max<std::size_t>(
              8, std::min<std::size_t>(32, std::min<std::size_t>(max_paths, 16) * 2));
    const auto prune_states = [beam_width](std::vector<CorrectionState>& states) {
        // Sorting used to rebuild two allocating reading strings for EVERY
        // comparison. The key is immutable once a state reaches this cell.
        for (auto& state : states)
            if (state.reading_key.empty())
                state.reading_key = assisted_reading_key(state.path);
        std::stable_sort(states.begin(), states.end(), [](const auto& left, const auto& right) {
            if (left.edit_count != right.edit_count)
                return left.edit_count < right.edit_count;
            // Prefer a long exact prefix. It avoids spending the limited edit
            // budget on plausible early syllables before reaching the actual
            // local typos later in a sentence.
            if (left.path.syllables.size() != right.path.syllables.size())
                return left.path.syllables.size() < right.path.syllables.size();
            if (left.edit_count >= 2 &&
                left.first_edit_offset != right.first_edit_offset)
                return left.first_edit_offset > right.first_edit_offset;
            return left.reading_key < right.reading_key;
        });
        std::vector<CorrectionState> retained;
        retained.reserve(std::min(states.size(), beam_width * 2));
        std::unordered_set<std::string> seen;
        std::array<std::size_t, 3> counts{};
        for (auto& state : states) {
            const auto bucket = static_cast<std::size_t>(state.edit_count);
            if (bucket >= counts.size()) continue;
            if (counts[bucket] >= beam_width) continue;
            auto key = state.reading_key;
            key.push_back(static_cast<char>('0' + state.edit_count));
            if (!seen.insert(std::move(key)).second) continue;
            ++counts[bucket];
            retained.push_back(std::move(state));
        }
        states = std::move(retained);
    };

    std::vector<std::vector<CorrectionState>> chart(normalized.size() + 1);
    ParsePath initial;
    initial.match_kind = InputMatchKind::corrected;
    chart[0].push_back({std::move(initial), 0});
    for (std::size_t offset = 0; offset < normalized.size(); ++offset) {
        if (cancelled && cancelled()) return;
        if (chart[offset].empty()) continue;
        prune_states(chart[offset]);
        for (const auto& state : chart[offset]) {
            if (cancelled && cancelled()) return;
            if (state.path.syllables.size() + 1 > exclusive_syllable_limit) continue;
            for (const auto& token : tokens_at(offset)) {
                if (token.corrected && state.edit_count >= maximum_edits) continue;
                CorrectionState next{
                    extend_path(state.path, {std::string(token.syllable), offset,
                                offset + token.consumed, true}),
                    state.edit_count, state.first_edit_offset, {}};
                if (token.corrected) {
                    if (next.edit_count == 0) next.first_edit_offset = offset;
                    ++next.edit_count;
                }
                next.path.edit_count = next.edit_count;
                auto& destination = chart[offset + token.consumed];
                destination.push_back(std::move(next));
                if (destination.size() >= beam_width * 8) prune_states(destination);
            }
        }
    }
    auto& completed = chart.back();
    prune_states(completed);
    std::array<std::vector<CorrectionState*>, 3> edit_groups;
    for (auto& state : completed) {
        if (state.edit_count == 0 ||
            state.path.syllables.size() > exclusive_syllable_limit)
            continue;
        edit_groups[state.edit_count].push_back(&state);
    }
    // Within the two-edit bucket, reserve representatives of distinct first
    // repair positions before taking more variants of the same suffix. A
    // later repair is preferable, but it must not exclude an earlier real
    // typo merely because that suffix admits many one-letter substitutions.
    auto& two_edits = edit_groups[2];
    struct PositionRank { CorrectionState* state; std::size_t rank; };
    std::vector<PositionRank> position_ranks;
    std::vector<std::size_t> position_counts(normalized.size() + 1);
    std::size_t syllable_count = 0;
    for (auto* state : two_edits) {
        if (syllable_count != state->path.syllables.size()) {
            syllable_count = state->path.syllables.size();
            std::fill(position_counts.begin(), position_counts.end(), 0);
        }
        position_ranks.push_back({state, position_counts[state->first_edit_offset]++});
    }
    std::stable_sort(position_ranks.begin(), position_ranks.end(), [](const auto& left, const auto& right) {
        if (left.state->path.syllables.size() != right.state->path.syllables.size())
            return left.state->path.syllables.size() < right.state->path.syllables.size();
        return left.rank < right.rank;
    });
    two_edits.clear();
    for (const auto& entry : position_ranks) two_edits.push_back(entry.state);
    // Preserve the smallest repair first without allowing either edit bucket
    // to consume the whole public budget. In particular a one-edit typo in
    // the middle of an otherwise exact sentence must not be displaced by
    // two-edit variants that also change its already-correct suffix.
    for (std::size_t rank = 0; paths.size() < max_paths; ++rank) {
        bool emitted = false;
        for (std::size_t edits = 1; edits < edit_groups.size() && paths.size() < max_paths; ++edits) {
            if (rank >= edit_groups[edits].size()) continue;
            paths.push_back(std::move(edit_groups[edits][rank]->path));
            emitted = true;
        }
        if (!emitted) break;
    }
}

std::vector<ChunkPath> parse_chunk(const std::string_view normalized,
                                   const std::size_t begin,
                                   const std::size_t end,
                                   const std::size_t max_paths,
                                   const bool allow_incomplete,
                                   const std::function<bool()>& cancelled) {
    std::vector<ChunkPath> paths;
    // A small caller budget is enough for ordinary words, but a long compact
    // sentence can spend that budget on variants of its first boundary before
    // reaching another equally short segmentation. Only long chunks explore a
    // wider bounded pool; the returned path count remains unchanged.
    const auto exploration_limit = end - begin >= 12
        ? std::min<std::size_t>(256, std::max<std::size_t>(64, max_paths * 8))
        : max_paths;
    std::vector<Syllable> current;
    std::function<void(std::size_t)> visit = [&](const std::size_t offset) {
        if ((cancelled && cancelled()) || paths.size() >= exploration_limit) return;
        if (offset == end) {
            paths.push_back({current, false});
            return;
        }

        const auto remaining = end - offset;
        const auto maximum = std::min<std::size_t>(6, remaining);
        for (std::size_t length = maximum; length > 0; --length) {
            const auto token = normalized.substr(offset, length);
            if (const auto canonical = canonical_complete_syllable(token)) {
                current.push_back({std::string(*canonical), offset, offset + length, true});
                visit(offset + length);
                current.pop_back();
            }
        }

        // An unfinished syllable is useful only at the end of the whole chunk.
        const auto suffix = normalized.substr(offset, remaining);
        if (allow_incomplete && paths.size() < exploration_limit && is_prefix(suffix)) {
            current.push_back({std::string(suffix), offset, end, false});
            paths.push_back({current, true});
            current.pop_back();
        }
    };
    visit(begin);

    // Without an explicit apostrophe, prefer the complete interpretation that
    // uses the fewest syllables. Thus "lan" remains lan instead of la+n and
    // "duo" remains duo instead of du+o. Keep incomplete paths only when they
    // use the same number of source segments: this retains zhong+gu ->
    // zhong+guo without reintroducing la+n prefix candidates.
    const auto complete = std::min_element(paths.begin(), paths.end(), chunk_path_less);
    if (complete != paths.end() && !complete->incomplete) {
        const auto minimum = complete->syllables.size();
        const bool explicit_single_syllable = minimum == 1 && end - begin > 1;
        std::erase_if(paths, [minimum, explicit_single_syllable](const ChunkPath& path) {
            // A complete one-syllable input is an explicit reading, not an
            // unfinished prefix: lin must not reserve candidate slots for
            // ling, nor bin for bing. A complete alternative with a natural
            // vowel-starting final boundary remains meaningful, however:
            // xian can be xi+an and should expose dictionary-backed 西安. A
            // A single trailing `a` is also a useful independent syllable
            // after a complete prefix (kua -> ku+a). Keep that boundary while
            // still excluding du+o, whose generated placeholders previously
            // displaced the much more likely complete `duo` reading.
            const auto preserves_implicit_boundary = [&] {
                if (!explicit_single_syllable || path.incomplete ||
                    path.syllables.size() <= minimum) return false;
                const auto& suffix = path.syllables.back();
                if (!suffix.complete || suffix.text.empty()) return false;
                if (suffix.end - suffix.begin == 1)
                    return suffix.text == "a";
                return suffix.text.front() == 'a' || suffix.text.front() == 'e' ||
                       suffix.text.front() == 'o';
            }();
            return (path.syllables.size() > minimum &&
                    !preserves_implicit_boundary) ||
                   (explicit_single_syllable && path.incomplete);
        });
    }
    std::stable_sort(paths.begin(), paths.end(), chunk_path_less);
    if (paths.size() > max_paths) paths.resize(max_paths);
    return paths;
}

}  // namespace

FullPinyinSchema::FullPinyinSchema() {
    // Pay the immutable prefix/edit-table construction cost at service startup
    // instead of on the first user typo.
    static_cast<void>(syllable_prefix_matches("a"));
    static_cast<void>(fuzzy_token_table());
}

ParseResult FullPinyinSchema::parse_incremental(
    const std::string_view input, const std::size_t max_paths,
    FullPinyinIncrementalState& state, FullPinyinParseMetrics* const metrics,
    const std::function<bool()>& cancelled, bool* const reused) const {
    if (reused != nullptr) *reused = false;
    std::string normalized;
    normalized.reserve(input.size());
    for (const unsigned char character : input) {
        if (character == '\'' || (character >= 'a' && character <= 'z'))
            normalized.push_back(static_cast<char>(character));
        else if (character >= 'A' && character <= 'Z')
            normalized.push_back(static_cast<char>(std::tolower(character)));
        else {
            auto result = parse(input, max_paths, false, metrics, cancelled);
            state = {std::string(input), result, max_paths};
            return result;
        }
    }

    const auto& previous = state.result;
    bool eligible = max_paths != 0 && state.max_paths == max_paths &&
                    previous.valid && !previous.paths.empty() &&
                    normalized.size() == previous.normalized_input.size() + 1 &&
                    normalized.starts_with(previous.normalized_input) &&
                    normalized.find('\'') == std::string::npos;
    std::vector<Syllable> stable_prefix;
    if (eligible) {
        const auto& preferred = previous.paths.front().syllables;
        for (std::size_t index = 0; index < preferred.size(); ++index) {
            const auto& syllable = preferred[index];
            if (!syllable.complete || syllable.end >= previous.normalized_input.size()) break;
            const bool common = std::all_of(
                previous.paths.begin() + 1, previous.paths.end(),
                [index, &syllable](const ParsePath& path) {
                    return index < path.syllables.size() &&
                           path.syllables[index] == syllable;
                });
            if (!common) break;
            stable_prefix.push_back(syllable);
        }
        eligible = !stable_prefix.empty();
    }

    const auto boundary = stable_prefix.empty() ? 0 : stable_prefix.back().end;
    if (eligible) {
        // Appending a character may turn text immediately before the reused
        // boundary into a longer legal syllable. In that case the old prefix
        // is not stable and a complete parse is required.
        const auto earliest = boundary > 5 ? boundary - 5 : 0;
        for (std::size_t begin = earliest; begin < boundary && eligible; ++begin) {
            const auto maximum_end = std::min(normalized.size(), begin + 6);
            for (std::size_t end = boundary + 1; end <= maximum_end; ++end) {
                const auto token = std::string_view(normalized).substr(begin, end - begin);
                if (is_complete(token) || is_prefix(token)) {
                    eligible = false;
                    break;
                }
            }
        }
    }

    ParseResult result;
    if (eligible && !(cancelled && cancelled())) {
        auto suffix = parse(std::string_view(normalized).substr(boundary), max_paths,
                            false, metrics, cancelled);
        if (suffix.valid) {
            result.normalized_input = normalized;
            result.has_incomplete_syllable = suffix.has_incomplete_syllable;
            result.paths.reserve(suffix.paths.size());
            for (auto& suffix_path : suffix.paths) {
                for (auto& syllable : suffix_path.syllables) {
                    syllable.begin += boundary;
                    syllable.end += boundary;
                }
                ParsePath combined = suffix_path;
                combined.syllables.insert(combined.syllables.begin(),
                                          stable_prefix.begin(), stable_prefix.end());
                result.paths.push_back(std::move(combined));
            }
            // Parsing the suffix in isolation can retain a valid implicit
            // vowel boundary that the complete chunk would reject. For
            // example, after reusing jiao, parsing dian alone preserves both
            // dian and di+an; the full jiaodian parse correctly keeps only
            // jiao+dian because it uses fewer syllables. Apply that same
            // whole-chunk rule to complete exact paths after recombination.
            auto minimum_exact_syllables = (std::numeric_limits<std::size_t>::max)();
            for (const auto& path : result.paths) {
                if (path.match_kind != InputMatchKind::exact ||
                    std::any_of(path.syllables.begin(), path.syllables.end(),
                                [](const Syllable& syllable) {
                                    return !syllable.complete;
                                })) continue;
                minimum_exact_syllables = std::min(
                    minimum_exact_syllables, path.syllables.size());
            }
            if (minimum_exact_syllables > 1 &&
                minimum_exact_syllables !=
                    (std::numeric_limits<std::size_t>::max)()) {
                std::erase_if(result.paths, [minimum_exact_syllables](const ParsePath& path) {
                    return path.match_kind == InputMatchKind::exact &&
                           path.syllables.size() > minimum_exact_syllables &&
                           std::all_of(path.syllables.begin(), path.syllables.end(),
                                       [](const Syllable& syllable) {
                                           return syllable.complete;
                                       });
                });
            }
            result.valid = !result.paths.empty();
            if (result.valid && reused != nullptr) *reused = true;
        }
    }
    if (!result.valid)
        result = parse(input, max_paths, false, metrics, cancelled);
    state = {std::string(input), result, max_paths};
    return result;
}

bool FullPinyinSchema::is_single_syllable_transposition(
    const std::string_view normalized_input) const {
    // At least four letters avoids treating short intentional initials as
    // typos; six is the longest canonical toneless syllable.
    if (normalized_input.size() < 4 || normalized_input.size() > 6 ||
        is_complete(normalized_input) ||
        !std::all_of(normalized_input.begin(), normalized_input.end(),
                     [](const char value) { return value >= 'a' && value <= 'z'; }))
        return false;
    auto swapped = std::string(normalized_input);
    for (std::size_t index = 1; index < swapped.size(); ++index) {
        if (swapped[index - 1] == swapped[index]) continue;
        std::swap(swapped[index - 1], swapped[index]);
        if (is_complete(swapped)) return true;
        std::swap(swapped[index - 1], swapped[index]);
    }
    return false;
}

ParseResult FullPinyinSchema::parse(const std::string_view input,
                                    const std::size_t max_paths) const {
    return parse(input, max_paths, true);
}

bool FullPinyinSchema::is_plausible_reading(const std::string_view input) const {
    if (input.empty() || input.size() > 256) return false;
    std::size_t position = 0;
    while (position < input.size()) {
        const auto separator = input.find('\'', position);
        const auto end = separator == std::string_view::npos ? input.size() : separator;
        const auto chunk = input.substr(position, end - position);
        if (chunk.empty()) return false;
        // A linear reachability sweep over at most six source bytes per
        // syllable boundary. Unlike parse(), it never enumerates paths, so the
        // cost stays proportional to the reading length.
        std::vector<char> reachable(chunk.size() + 1, 0);
        reachable[0] = 1;
        for (std::size_t offset = 0; offset < chunk.size(); ++offset) {
            if (reachable[offset] == 0) continue;
            const auto maximum = std::min<std::size_t>(6, chunk.size() - offset);
            for (std::size_t length = 1; length <= maximum; ++length) {
                if (is_known_syllable(chunk.substr(offset, length)))
                    reachable[offset + length] = 1;
            }
        }
        bool plausible = reachable[chunk.size()] != 0;
        const bool final_chunk = separator == std::string_view::npos;
        for (std::size_t offset = 1; offset < chunk.size() && !plausible; ++offset) {
            if (reachable[offset] == 0) continue;
            // An unfinished syllable is only meaningful as the trailing part of
            // the whole reading; an explicit apostrophe demands complete ones.
            if (!final_chunk) break;
            if (is_known_prefix(chunk.substr(offset))) plausible = true;
        }
        if (!plausible) return false;
        if (separator == std::string_view::npos) break;
        position = separator + 1;
    }
    return true;
}

ParseResult FullPinyinSchema::parse(const std::string_view input,
                                    const std::size_t max_paths,
                                    const bool correction_enabled) const {
    return parse(input, max_paths, correction_enabled, nullptr);
}

ParseResult FullPinyinSchema::parse(const std::string_view input,
                                    const std::size_t max_paths,
                                    const bool correction_enabled,
                                    FullPinyinParseMetrics* const metrics,
                                    const std::function<bool()>& cancelled) const {
    const auto normalization_started = std::chrono::steady_clock::now();
    ParseResult result;
    if (input.empty() || max_paths == 0) return result;

    result.normalized_input.reserve(input.size());
    for (const unsigned char character : input) {
        if (character == '\'' || (character >= 'a' && character <= 'z')) {
            result.normalized_input.push_back(static_cast<char>(character));
        } else if (character >= 'A' && character <= 'Z') {
            result.normalized_input.push_back(static_cast<char>(std::tolower(character)));
        } else {
            return result;
        }
    }
    if (result.normalized_input.front() == '\'' ||
        result.normalized_input.find("''") != std::string::npos) return result;
    const auto normalization_finished = std::chrono::steady_clock::now();

    std::vector<ParsePath> combined(1);
    bool any_incomplete = false;
    bool base_valid = true;
    std::size_t chunk_begin = 0;
    while (chunk_begin < result.normalized_input.size()) {
        if (cancelled && cancelled()) return result;
        const auto separator = result.normalized_input.find('\'', chunk_begin);
        const auto chunk_end = separator == std::string::npos ? result.normalized_input.size() : separator;
        auto chunk_paths = parse_chunk(result.normalized_input, chunk_begin, chunk_end,
                                       max_paths, true, cancelled);
        const auto chunk = std::string_view(result.normalized_input).substr(
            chunk_begin, chunk_end - chunk_begin);
        const bool consonant_abbreviation = chunk.size() >= 2 &&
            std::all_of(chunk.begin(), chunk.end(), [](const char value) {
                constexpr std::string_view vowels = "aeiouv";
                return value >= 'a' && value <= 'z' &&
                       vowels.find(value) == std::string_view::npos;
            });
        if (chunk_paths.empty() && consonant_abbreviation) {
            std::vector<ParsePath> abbreviated;
            append_abbreviated_completions(chunk, abbreviated, max_paths, cancelled);
            chunk_paths.reserve(abbreviated.size());
            for (auto& path : abbreviated) {
                for (auto& syllable : path.syllables) {
                    syllable.begin += chunk_begin;
                    syllable.end += chunk_begin;
                }
                chunk_paths.push_back({std::move(path.syllables), false,
                                       path.match_kind,
                                       path.completion_characters});
            }
        }
        if (separator != std::string::npos &&
            std::any_of(chunk_paths.begin(), chunk_paths.end(),
                        [](const ChunkPath& path) { return !path.incomplete; })) {
            std::erase_if(chunk_paths,
                          [](const ChunkPath& path) { return path.incomplete; });
        }
        if (chunk_paths.empty()) {
            base_valid = false;
            combined.clear();
            break;
        }

        std::vector<ParsePath> next;
        for (const auto& prefix : combined) {
            for (const auto& suffix : chunk_paths) {
                if (next.size() >= max_paths) break;
                ParsePath path = prefix;
                path.syllables.insert(path.syllables.end(), suffix.syllables.begin(), suffix.syllables.end());
                if (static_cast<std::uint8_t>(suffix.match_kind) >
                    static_cast<std::uint8_t>(path.match_kind))
                    path.match_kind = suffix.match_kind;
                path.completion_characters += suffix.completion_characters;
                next.push_back(std::move(path));
                any_incomplete = any_incomplete || suffix.incomplete;
            }
            if (next.size() >= max_paths) break;
        }
        combined = std::move(next);
        if (separator == std::string::npos) break;
        chunk_begin = separator + 1;
    }
    const auto segmentation_finished = std::chrono::steady_clock::now();
    const auto previous_correction_us = metrics ? metrics->correction_us : 0;

    if (base_valid) result.paths = std::move(combined);
    const auto base_paths = result.paths;
    const bool stable_trailing_prefix = std::any_of(
        base_paths.begin(), base_paths.end(), [](const ParsePath& path) {
            if (path.syllables.size() < 2 || path.syllables.back().complete)
                return false;
            return std::all_of(path.syllables.begin(), path.syllables.end() - 1,
                               [](const Syllable& syllable) {
                                   return syllable.complete;
                               });
        });
    const bool long_stable_trailing_prefix = result.normalized_input.size() >= 12 &&
        std::any_of(base_paths.begin(), base_paths.end(), [](const ParsePath& path) {
            if (path.syllables.size() < 3 || path.syllables.back().complete) return false;
            constexpr std::string_view vowels = "aeiouv";
            for (std::size_t index = 0; index + 1 < path.syllables.size(); ++index) {
                const auto& syllable = path.syllables[index];
                if (!syllable.complete || syllable.text.empty()) return false;
                if (index != 0 && vowels.find(syllable.text.front()) != std::string_view::npos)
                    return false;
            }
            return true;
        });
    std::size_t corrected_syllable_limit = std::numeric_limits<std::size_t>::max();
    for (const auto& path : base_paths) {
        if (std::any_of(path.syllables.begin(), path.syllables.end(),
                        [](const Syllable& syllable) { return !syllable.complete; })) continue;
        corrected_syllable_limit = std::min(corrected_syllable_limit, path.syllables.size());
    }
    std::vector<ParsePath> incomplete_paths;
    append_incomplete_completions(base_paths, incomplete_paths, max_paths,
                                  corrected_syllable_limit);
    if (corrected_syllable_limit != std::numeric_limits<std::size_t>::max()) {
        for (auto& path : incomplete_paths) {
            if (result.paths.size() >= max_paths) break;
            result.paths.push_back(std::move(path));
        }
    } else {
        std::vector<ParsePath> abbreviated_paths;
        // Once the parser already has a complete prefix followed by one
        // incomplete syllable (zhong'g, ni'h), prefix completion covers the
        // useful candidates. Expanding the whole string as an abbreviation
        // creates hundreds of duplicate families that are later discarded.
        if (!long_stable_trailing_prefix && !stable_trailing_prefix)
            append_abbreviated_completions(result.normalized_input, abbreviated_paths,
                                           std::max<std::size_t>(max_paths, 256), cancelled);
        incomplete_paths.insert(incomplete_paths.end(),
                                std::make_move_iterator(abbreviated_paths.begin()),
                                std::make_move_iterator(abbreviated_paths.end()));
        prune_assisted_paths(incomplete_paths, max_paths, true);

        std::vector<ParsePath> corrected_paths;
        const auto correction_started = std::chrono::steady_clock::now();
        if (correction_enabled && !long_stable_trailing_prefix)
            append_corrected_paths(result.normalized_input, corrected_paths, max_paths,
                                   corrected_syllable_limit, cancelled);
        if (metrics != nullptr)
            metrics->correction_us += static_cast<std::uint64_t>(
                std::chrono::duration_cast<std::chrono::microseconds>(
                    std::chrono::steady_clock::now() - correction_started).count());

        // Raw incomplete paths cannot produce candidates. Retain one for
        // diagnostics, then reserve a bounded correction slice so a broad
        // one-letter suffix expansion cannot starve useful typo recovery.
        result.paths.clear();
        if (!base_paths.empty()) result.paths.push_back(base_paths.front());
        const auto correction_reserve = std::min<std::size_t>(
            8, std::min(corrected_paths.size(), max_paths - result.paths.size()));
        std::size_t incomplete_index = 0;
        while (incomplete_index < incomplete_paths.size() &&
               result.paths.size() + correction_reserve < max_paths) {
            result.paths.push_back(std::move(incomplete_paths[incomplete_index++]));
        }
        std::size_t corrected_index = 0;
        while (corrected_index < corrected_paths.size() && result.paths.size() < max_paths) {
            result.paths.push_back(std::move(corrected_paths[corrected_index++]));
        }
        while (incomplete_index < incomplete_paths.size() && result.paths.size() < max_paths) {
            result.paths.push_back(std::move(incomplete_paths[incomplete_index++]));
        }
        for (std::size_t index = 1; index < base_paths.size() && result.paths.size() < max_paths;
             ++index) {
            result.paths.push_back(base_paths[index]);
        }
    }
    if (result.paths.empty() &&
        result.normalized_input.size() <= 256 &&
        result.normalized_input.find('\'') == std::string::npos &&
        std::all_of(result.normalized_input.begin(), result.normalized_input.end(),
                    [](const char value) {
                        constexpr std::string_view vowels = "aeiouv";
                        return value >= 'a' && value <= 'z' &&
                               vowels.find(value) == std::string_view::npos;
                    })) {
        ParsePath initials;
        initials.match_kind = InputMatchKind::abbreviated_completion;
        initials.syllables.reserve(result.normalized_input.size());
        for (std::size_t index = 0; index < result.normalized_input.size(); ++index) {
            initials.syllables.push_back(
                {result.normalized_input.substr(index, 1), index, index + 1, false});
        }
        result.paths.push_back(std::move(initials));
        any_incomplete = true;
    }
    // An explicit apostrophe is a user-authored syllable boundary. If one of
    // its chunks is invalid, do not discard that boundary and reinterpret the
    // entire input as unrelated single-letter readings (for example,
    // quan'loi must not become q'u'a'n'l'o'i when correction is disabled).
    if (result.paths.empty() && result.normalized_input.size() <= 256 &&
        result.normalized_input.find('\'') == std::string::npos) {
        ParsePath fallback;
        fallback.match_kind = InputMatchKind::abbreviated_completion;
        fallback.syllables.reserve(result.normalized_input.size());
        for (std::size_t index = 0; index < result.normalized_input.size(); ++index) {
            const auto value = result.normalized_input[index];
            if (value == '\'') continue;
            const auto reading = fallback_syllable(value);
            if (reading.empty()) {
                fallback.syllables.clear();
                break;
            }
            fallback.syllables.push_back(
                {std::string(reading), index, index + 1, true});
            fallback.completion_characters +=
                static_cast<std::uint32_t>(reading.size() - 1);
        }
        if (fallback.syllables.size() >= 2) result.paths.push_back(std::move(fallback));
    }
    result.valid = !result.paths.empty();
    result.has_incomplete_syllable = any_incomplete;
    if (metrics != nullptr) {
        metrics->normalization_us = static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::microseconds>(
                normalization_finished - normalization_started).count());
        metrics->segmentation_us = static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::microseconds>(
                segmentation_finished - normalization_finished).count());
        const auto assisted_total = static_cast<std::uint64_t>(
            std::chrono::duration_cast<std::chrono::microseconds>(
                std::chrono::steady_clock::now() - segmentation_finished).count());
        const auto correction_us = metrics->correction_us - previous_correction_us;
        metrics->assistance_us = assisted_total - std::min(assisted_total, correction_us);
    }
    return result;
}

}  // namespace owo::engine

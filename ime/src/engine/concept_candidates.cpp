#include "owo/engine/concept_candidates.h"

#include <algorithm>
#include <array>
#include <fstream>
#include <unordered_set>
#include <utility>

namespace owo::engine {
namespace {

struct StarterConcept {
    std::string_view term;
    std::string_view reading;
    std::string_view cues;
    std::string_view description;
};

// Starter glossary only. These are conventional descriptions of common
// engineering terms; a real deployment should ship a project glossary as a TSV
// through load_concept_table instead of extending this table.
constexpr std::array<StarterConcept, 12> kStarterConcepts{{
    {"幂等", "mideng", "chongfuzhixing,chongfuzhixingjieguobubian",
     "重复执行结果不变"},
    {"死锁", "sisuo", "huxiangdengdai,xianghudengdai", "互相等待"},
    {"缓存", "huancun", "jiasuduqu,huancunjieguo", "加速读取"},
    {"并发", "bingfa", "tongshiyunxing,duorenwutongshi", "同时运行多个任务"},
    {"递归", "digui", "zishendiaoyong,zijidiaoyongziji", "函数调用自身"},
    {"解耦", "jiegou", "jiangdiyilai,jiangdiyilaiguanxi", "降低依赖关系"},
    {"抽象", "chouxiang", "tiqugongtongtezheng", "提取共同特征"},
    {"迭代", "diedai", "fanfuyouhua,zhubulunyouhua", "反复优化"},
    {"冗余", "rongyu", "chongfuduoyu,beifenyulan", "重复备份"},
    {"灰度", "huidu", "zhujianfangliang,xiaofanweifabu", "逐步放量发布"},
    {"埋点", "maidian", "jiluyonghuxingwei,shangbaoshijian", "记录用户行为"},
    {"兜底", "doudi", "baozhengkekaoyong,zhidingbaohucuoshi", "制定保护措施"},
}};

std::vector<std::string> split(const std::string_view text, const char separator) {
    std::vector<std::string> parts;
    std::size_t begin = 0;
    while (begin <= text.size()) {
        const auto end = text.find(separator, begin);
        const auto stop = end == std::string_view::npos ? text.size() : end;
        parts.push_back(std::string(text.substr(begin, stop - begin)));
        if (end == std::string_view::npos) break;
        begin = stop + 1;
    }
    return parts;
}

std::string trim(const std::string_view text) {
    std::size_t begin = 0;
    std::size_t end = text.size();
    while (begin < end && (text[begin] == ' ' || text[begin] == '\t' ||
                           text[begin] == '\r' || text[begin] == '\n'))
        ++begin;
    while (end > begin && (text[end - 1] == ' ' || text[end - 1] == '\t' ||
                           text[end - 1] == '\r' || text[end - 1] == '\n'))
        --end;
    return std::string(text.substr(begin, end - begin));
}

bool valid_utf8(const std::string_view value) {
    for (std::size_t index = 0; index < value.size();) {
        const auto lead = static_cast<unsigned char>(value[index++]);
        std::size_t continuation = 0;
        if (lead < 0x80) continue;
        if ((lead & 0xE0U) == 0xC0U) continuation = 1;
        else if ((lead & 0xF0U) == 0xE0U) continuation = 2;
        else if ((lead & 0xF8U) == 0xF0U) continuation = 3;
        else return false;
        if (continuation > value.size() - index) return false;
        for (std::size_t extra = 0; extra < continuation; ++extra)
            if ((static_cast<unsigned char>(value[index++]) & 0xC0U) != 0x80U)
                return false;
    }
    return true;
}

}  // namespace

ConceptTable::ConceptTable(std::vector<ConceptEntry> entries)
    : entries_(std::move(entries)) {
    rebuild_index();
}

void ConceptTable::rebuild_index() {
    exact_.clear();
    sorted_cues_.clear();
    for (std::size_t index = 0; index < entries_.size(); ++index) {
        for (const auto& cue : entries_[index].cue_readings) {
            auto& owners = exact_[cue];
            if (!owners.empty() && owners.back() == index) continue;
            owners.push_back(index);
            sorted_cues_.emplace_back(cue, index);
        }
    }
    std::sort(sorted_cues_.begin(), sorted_cues_.end());
}

std::vector<const ConceptEntry*> ConceptTable::lookup(
    const std::string_view normalized_key) const {
    std::vector<const ConceptEntry*> matches;
    if (normalized_key.empty()) return matches;
    const auto found = exact_.find(std::string(normalized_key));
    if (found == exact_.end()) return matches;
    matches.reserve(found->second.size());
    for (const auto index : found->second) matches.push_back(&entries_[index]);
    return matches;
}

std::vector<const ConceptEntry*> ConceptTable::lookup_prefix(
    const std::string_view normalized_key,
    const std::size_t minimum_prefix) const {
    std::vector<const ConceptEntry*> matches;
    if (normalized_key.size() < minimum_prefix) return matches;
    const auto compare = [](const std::pair<std::string, std::size_t>& entry,
                            const std::string_view key) {
        return std::string_view(entry.first) < key;
    };
    auto iterator =
        std::lower_bound(sorted_cues_.begin(), sorted_cues_.end(),
                         normalized_key, compare);
    // A cue equal to the key is an exact match, not a prefix one.
    while (iterator != sorted_cues_.end() && iterator->first == normalized_key)
        ++iterator;
    for (; iterator != sorted_cues_.end() &&
           iterator->first.size() > normalized_key.size() &&
           std::string_view(iterator->first).starts_with(normalized_key);
         ++iterator) {
        const auto* entry = &entries_[iterator->second];
        if (std::find(matches.begin(), matches.end(), entry) == matches.end())
            matches.push_back(entry);
    }
    return matches;
}

ConceptTableLoadResult load_concept_table(const std::filesystem::path& path) {
    ConceptTableLoadResult result;
    std::ifstream input(path, std::ios::binary);
    if (!input) {
        result.error = "concept table is not readable";
        return result;
    }
    std::vector<ConceptEntry> entries;
    std::string line;
    std::size_t line_number = 0;
    while (std::getline(input, line)) {
        ++line_number;
        if (!line.empty() && line.back() == '\r') line.pop_back();
        const auto trimmed = trim(line);
        if (trimmed.empty() || trimmed.front() == '#') continue;
        const auto fields = split(trimmed, '\t');
        if (fields.size() < 3) {
            result.error = "line " + std::to_string(line_number) +
                           " must be term<TAB>reading<TAB>cues";
            return result;
        }
        ConceptEntry entry;
        entry.term = fields[0];
        entry.term_reading = fields[1];
        entry.cue_text = fields.size() > 3 ? fields[3] : std::string{};
        if (entry.term.empty() || entry.term.size() > 64 ||
            !valid_utf8(entry.term)) {
            result.error = "line " + std::to_string(line_number) +
                           " has an invalid term";
            return result;
        }
        std::unordered_set<std::string> seen;
        for (const auto& cue : split(fields[2], ',')) {
            auto reading = concept_lookup_key(cue);
            if (reading.empty()) continue;
            if (seen.insert(reading).second)
                entry.cue_readings.push_back(std::move(reading));
        }
        if (entry.cue_readings.empty()) {
            result.error = "line " + std::to_string(line_number) +
                           " has no usable cue reading";
            return result;
        }
        entries.push_back(std::move(entry));
    }
    if (input.bad()) {
        result.error = "concept table read failed";
        return result;
    }
    result.table = ConceptTable(std::move(entries));
    return result;
}

const ConceptTable& builtin_concept_table() {
    static const ConceptTable table = [] {
        std::vector<ConceptEntry> entries;
        entries.reserve(kStarterConcepts.size());
        for (const auto& starter : kStarterConcepts) {
            ConceptEntry entry;
            entry.term = std::string(starter.term);
            entry.term_reading = std::string(starter.reading);
            entry.cue_text = std::string(starter.description);
            for (const auto& cue : split(starter.cues, ',')) {
                const auto reading = concept_lookup_key(cue);
                if (!reading.empty()) entry.cue_readings.push_back(reading);
            }
            entries.push_back(std::move(entry));
        }
        return ConceptTable(std::move(entries));
    }();
    return table;
}

std::string concept_lookup_key(const std::string_view raw) {
    // Cue readings are pure pinyin: keep ASCII letters, fold case, and drop
    // spacing/apostrophes/digits so the same cue matches however it is typed.
    std::string key;
    key.reserve(raw.size());
    for (const char character : raw) {
        if (character >= 'a' && character <= 'z') {
            key.push_back(character);
            continue;
        }
        if (character >= 'A' && character <= 'Z')
            key.push_back(static_cast<char>(character - 'A' + 'a'));
    }
    return key;
}

std::vector<Candidate> generate_concept_candidates(
    const std::string_view raw, const ConceptTable& table,
    const std::size_t limit) {
    std::vector<Candidate> candidates;
    if (raw.empty() || limit == 0 || table.empty()) return candidates;
    // Cue readings are letters only. A preedit that also carries digits or
    // symbols is a different token (version suffix, punctuation) and must not
    // be rewritten into a term; separators such as spaces and apostrophes stay
    // allowed so "chong fu zhi xing" still resolves.
    for (const char character : raw) {
        if ((character >= 'a' && character <= 'z') ||
            (character >= 'A' && character <= 'Z') || character == '\'' ||
            character == ' ' || character == '\t' || character == '\r' ||
            character == '\n')
            continue;
        return candidates;
    }
    const auto key = concept_lookup_key(raw);
    if (key.empty()) return candidates;
    const auto append = [&candidates, raw, limit](const ConceptEntry* entry,
                                                  const std::int64_t score) {
        if (candidates.size() >= std::min(limit, kConceptMaximumCandidates))
            return;
        const bool duplicate = std::any_of(
            candidates.begin(), candidates.end(),
            [entry](const Candidate& candidate) {
                return candidate.text == entry->term;
            });
        if (duplicate) return;
        Candidate candidate;
        candidate.text = entry->term;
        candidate.score = score;
        candidate.match_kind = InputMatchKind::exact;
        candidate.consumed_input_bytes = raw.size();
        candidate.segment_count = 1;
        candidates.push_back(std::move(candidate));
    };
    const auto exact = table.lookup(key);
    for (const auto* entry : exact) append(entry, kConceptCandidateScore);
    if (exact.empty()) {
        for (const auto* entry :
             table.lookup_prefix(key, kConceptPrefixMinimumLetters))
            append(entry, kConceptPrefixScore);
    }
    return candidates;
}

}  // namespace owo::engine

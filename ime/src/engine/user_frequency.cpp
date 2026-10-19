#include "owo/engine/user_frequency.h"

#ifdef _WIN32
#include <Windows.h>
#endif

#include <algorithm>
#include <array>
#include <fstream>
#include <iterator>
#include <span>
#include <vector>

namespace owo::engine {
namespace {

constexpr std::array<unsigned char, 4> kMagic{'O', 'W', 'U', 'F'};
constexpr std::uint32_t kVersion = 3;
constexpr std::size_t kHeaderSize = 20;
constexpr std::size_t kMaximumFileBytes = 16U * 1024U * 1024U;
constexpr std::uint32_t kGlobalCountCeiling = 256;
constexpr std::uint32_t kInputCountCeiling = 128;
constexpr std::uint32_t kLanguageContextCountCeiling = 64;
constexpr std::uint32_t kDecayNumerator = 3;
constexpr std::uint32_t kDecayDenominator = 4;

template <typename Predicate>
bool decay_matching_counts(
    std::unordered_map<std::string, std::uint32_t>& counts,
    Predicate matches) {
    bool changed = false;
    for (auto iterator = counts.begin(); iterator != counts.end();) {
        if (!matches(iterator->first)) {
            ++iterator;
            continue;
        }
        const auto reduced = static_cast<std::uint32_t>(
            static_cast<std::uint64_t>(iterator->second) * kDecayNumerator /
            kDecayDenominator);
        changed = changed || reduced != iterator->second;
        if (reduced == 0)
            iterator = counts.erase(iterator);
        else {
            iterator->second = reduced;
            ++iterator;
        }
    }
    return changed;
}

template <typename Predicate>
bool record_bounded(
    std::unordered_map<std::string, std::uint32_t>& counts,
    const std::string& key, const std::uint32_t amount,
    const std::uint32_t ceiling, Predicate same_competition_group) {
    const auto found = counts.find(key);
    const auto current = found == counts.end() ? 0U : found->second;
    if (static_cast<std::uint64_t>(current) + amount <= ceiling) {
        counts[key] = current + amount;
        return false;
    }

    // Reaching the ceiling is treated as an adaptive ageing epoch. Decay the
    // candidates that compete in the same scope, then add the newest evidence.
    // This preserves ordering while eventually removing stale one-off choices.
    const auto decayed = decay_matching_counts(counts, same_competition_group);
    auto& value = counts[key];
    value = static_cast<std::uint32_t>((std::min)(
        static_cast<std::uint64_t>(ceiling),
        static_cast<std::uint64_t>(value) + amount));
    return decayed;
}

std::int64_t bounded_learning_score(const std::uint32_t count,
                                    const std::int64_t gain,
                                    const std::uint32_t saturation_knee) {
    if (count == 0) return 0;
    // The first observation is worth exactly one gain unit. Later evidence
    // keeps helping, but approaches (knee + 1) units instead of growing
    // without bound.
    const auto numerator = static_cast<std::int64_t>(count) *
                           static_cast<std::int64_t>(saturation_knee + 1U) * gain;
    return numerator /
           (static_cast<std::int64_t>(count) + saturation_knee);
}

void append_u32(std::vector<unsigned char>& out, const std::uint32_t value) {
    for (unsigned shift = 0; shift < 32; shift += 8) out.push_back(static_cast<unsigned char>(value >> shift));
}
void append_u64(std::vector<unsigned char>& out, const std::uint64_t value) {
    for (unsigned shift = 0; shift < 64; shift += 8) out.push_back(static_cast<unsigned char>(value >> shift));
}
std::uint64_t checksum(const std::span<const unsigned char> bytes) {
    std::uint64_t value = 14695981039346656037ULL;
    for (const auto byte : bytes) { value ^= byte; value *= 1099511628211ULL; }
    return value;
}
bool read_u32(const std::span<const unsigned char> bytes, std::size_t& offset, std::uint32_t& value) {
    if (offset + 4 > bytes.size()) return false;
    value = 0;
    for (unsigned index = 0; index < 4; ++index) value |= static_cast<std::uint32_t>(bytes[offset + index]) << (index * 8U);
    offset += 4;
    return true;
}
bool read_u64(const std::span<const unsigned char> bytes, std::size_t& offset, std::uint64_t& value) {
    if (offset + 8 > bytes.size()) return false;
    value = 0;
    for (unsigned index = 0; index < 8; ++index) value |= static_cast<std::uint64_t>(bytes[offset + index]) << (index * 8U);
    offset += 8;
    return true;
}

std::string compact_input_key(const std::string_view input) {
    std::string result;
    result.reserve(input.size());
    std::copy_if(input.begin(), input.end(), std::back_inserter(result),
                 [](const char value) { return value != '\''; });
    return result;
}

std::string_view relevant_language_context(std::string_view context) {
    while (!context.empty()) {
        const auto value = static_cast<unsigned char>(context.back());
        if (value != ' ' && value != '\t' && value != '\r' && value != '\n') break;
        context.remove_suffix(1);
    }
    std::size_t sentence_begin = 0;
    constexpr std::array<std::string_view, 5> boundaries{
        "。", "！", "？", "\n", "\r"};
    for (const auto boundary : boundaries) {
        const auto position = context.rfind(boundary);
        if (position != std::string_view::npos)
            sentence_begin = (std::max)(sentence_begin,
                                        position + boundary.size());
    }
    context.remove_prefix((std::min)(sentence_begin, context.size()));
    while (!context.empty() &&
           (context.front() == ' ' || context.front() == '\t'))
        context.remove_prefix(1);
    return context;
}

std::vector<std::size_t> utf8_boundaries(const std::string_view text) {
    std::vector<std::size_t> boundaries;
    boundaries.reserve(text.size() + 1);
    for (std::size_t offset = 0; offset < text.size(); ++offset) {
        if ((static_cast<unsigned char>(text[offset]) & 0xc0U) != 0x80U)
            boundaries.push_back(offset);
    }
    boundaries.push_back(text.size());
    return boundaries;
}

struct ContextSuffix {
    std::string_view text;
    std::size_t character_count{};
};

std::vector<ContextSuffix> context_suffixes_for_record(
    const std::string_view raw_context) {
    const auto context = relevant_language_context(raw_context);
    if (context.empty()) return {};
    const auto boundaries = utf8_boundaries(context);
    if (boundaries.size() <= 1) return {};
    const auto characters = boundaries.size() - 1;
    constexpr std::array<std::size_t, 6> requested{1, 2, 3, 4, 8, 16};
    std::vector<ContextSuffix> result;
    result.reserve(requested.size() + 1);
    for (const auto length : requested) {
        const auto kept = (std::min)(length, characters);
        if (!result.empty() && result.back().character_count == kept) continue;
        result.push_back({context.substr(boundaries[characters - kept]), kept});
    }
    if (result.back().character_count != characters)
        result.push_back({context, characters});
    return result;
}

bool decode(const std::filesystem::path& path,
            std::unordered_map<std::string, std::uint32_t>& counts,
            std::unordered_map<std::string, std::uint32_t>& contextual_counts,
            std::unordered_map<std::string, std::uint32_t>& language_context_counts) {
    std::ifstream input(path, std::ios::binary);
    if (!input) return false;
    std::vector<unsigned char> bytes((std::istreambuf_iterator<char>(input)), {});
    if (bytes.size() < kHeaderSize || bytes.size() > kMaximumFileBytes ||
        !std::equal(kMagic.begin(), kMagic.end(), bytes.begin())) return false;
    std::size_t offset = 4;
    std::uint32_t version{}, entry_count{};
    std::uint64_t expected{};
    if (!read_u32(bytes, offset, version) || version < 1 || version > kVersion ||
        !read_u32(bytes, offset, entry_count) || !read_u64(bytes, offset, expected) ||
        checksum(std::span(bytes).subspan(offset)) != expected) return false;
    std::unordered_map<std::string, std::uint32_t> parsed;
    std::unordered_map<std::string, std::uint32_t> parsed_contextual;
    std::unordered_map<std::string, std::uint32_t> parsed_language_context;
    for (std::uint32_t index = 0; index < entry_count; ++index) {
        std::uint32_t context_size{}, input_size{}, size{}, count{};
        if (version >= 3 && !read_u32(bytes, offset, context_size)) return false;
        if (version >= 2 && !read_u32(bytes, offset, input_size)) return false;
        if (!read_u32(bytes, offset, size) || size == 0 || offset + size > bytes.size()) return false;
        std::string context_text;
        if (context_size != 0) {
            if (offset + context_size + input_size + size > bytes.size()) return false;
            context_text.assign(reinterpret_cast<const char*>(bytes.data() + offset), context_size);
            offset += context_size;
        }
        std::string input_text;
        if (input_size != 0) {
            if (offset + input_size + size > bytes.size()) return false;
            input_text.assign(reinterpret_cast<const char*>(bytes.data() + offset), input_size);
            offset += input_size;
        }
        std::string text(reinterpret_cast<const char*>(bytes.data() + offset), size);
        offset += size;
        if (!read_u32(bytes, offset, count) || count == 0) return false;
        if (!context_text.empty() && !input_text.empty()) {
            context_text.push_back('\0');
            context_text += input_text;
            context_text.push_back('\0');
            context_text += text;
            if (!parsed_language_context.emplace(std::move(context_text), count).second)
                return false;
        } else if (input_text.empty()) {
            if (!parsed.emplace(std::move(text), count).second) return false;
        } else {
            input_text.push_back('\0');
            input_text += text;
            if (!parsed_contextual.emplace(std::move(input_text), count).second) return false;
        }
    }
    if (offset != bytes.size()) return false;
    counts = std::move(parsed);
    contextual_counts = std::move(parsed_contextual);
    language_context_counts = std::move(parsed_language_context);
    return true;
}

std::vector<unsigned char> encode(
    const std::unordered_map<std::string, std::uint32_t>& counts,
    const std::unordered_map<std::string, std::uint32_t>& contextual_counts,
    const std::unordered_map<std::string, std::uint32_t>& language_context_counts) {
    struct Entry { std::string context; std::string input; std::string text; std::uint32_t count{}; };
    std::vector<Entry> ordered;
    ordered.reserve(counts.size() + contextual_counts.size() + language_context_counts.size());
    for (const auto& [text, count] : counts) ordered.push_back({{}, {}, text, count});
    for (const auto& [key, count] : contextual_counts) {
        const auto separator = key.find('\0');
        if (separator == std::string::npos || separator == 0 || separator + 1 >= key.size()) continue;
        ordered.push_back({{}, key.substr(0, separator), key.substr(separator + 1), count});
    }
    for (const auto& [key, count] : language_context_counts) {
        const auto first = key.find('\0');
        const auto second = first == std::string::npos ? std::string::npos
                                                       : key.find('\0', first + 1);
        if (first == std::string::npos || second == std::string::npos || first == 0 ||
            second == first + 1 || second + 1 >= key.size()) continue;
        ordered.push_back({key.substr(0, first), key.substr(first + 1, second - first - 1),
                           key.substr(second + 1), count});
    }
    std::sort(ordered.begin(), ordered.end(), [](const Entry& left, const Entry& right) {
        if (left.context != right.context) return left.context < right.context;
        if (left.input != right.input) return left.input < right.input;
        return left.text < right.text;
    });
    std::vector<unsigned char> payload;
    for (const auto& entry : ordered) {
        append_u32(payload, static_cast<std::uint32_t>(entry.context.size()));
        append_u32(payload, static_cast<std::uint32_t>(entry.input.size()));
        append_u32(payload, static_cast<std::uint32_t>(entry.text.size()));
        payload.insert(payload.end(), entry.context.begin(), entry.context.end());
        payload.insert(payload.end(), entry.input.begin(), entry.input.end());
        payload.insert(payload.end(), entry.text.begin(), entry.text.end());
        append_u32(payload, entry.count);
    }
    std::vector<unsigned char> output(kMagic.begin(), kMagic.end());
    append_u32(output, kVersion);
    append_u32(output, static_cast<std::uint32_t>(ordered.size()));
    append_u64(output, checksum(payload));
    output.insert(output.end(), payload.begin(), payload.end());
    return output;
}

}  // namespace

UserFrequencyIoResult UserFrequencyStore::load(const std::filesystem::path& path) {
    path_ = path;
    counts_.clear();
    contextual_counts_.clear();
    language_context_counts_.clear();
    user_lexicon_.clear();
    if (!std::filesystem::exists(path)) return {true, false, {}};
    if (decode(path, counts_, contextual_counts_, language_context_counts_)) {
        rebuild_user_lexicon();
        return {true, false, {}};
    }
    auto backup = path; backup += L".bak";
    if (decode(backup, counts_, contextual_counts_, language_context_counts_)) {
        rebuild_user_lexicon();
        return {true, true, {}};
    }
    return {false, false, "user frequency and backup are invalid"};
}

void UserFrequencyStore::rebuild_user_lexicon() {
    user_lexicon_.clear();
    for (const auto& [key, count] : contextual_counts_) {
        const auto separator = key.find('\0');
        if (separator == std::string::npos || separator == 0 ||
            separator + 1 >= key.size())
            continue;
        user_lexicon_[compact_input_key(
                          std::string_view(key).substr(0, separator))]
                     [key.substr(separator + 1)] = count;
    }
}

void UserFrequencyStore::record(const std::string_view input, const std::string_view text,
                                const std::uint32_t amount) {
    record(text, amount);
    if (input.empty() || text.empty() || amount == 0) return;
    std::string key(input);
    key.push_back('\0');
    key += text;
    std::string group(input);
    group.push_back('\0');
    const auto decayed = record_bounded(
        contextual_counts_, key, amount, kInputCountCeiling,
        [&group](const std::string_view candidate) {
            return candidate.starts_with(group);
        });
    if (decayed)
        rebuild_user_lexicon();
    else
        user_lexicon_[compact_input_key(input)][std::string(text)] =
            contextual_counts_[key];
}

void UserFrequencyStore::record(const std::string_view context, const std::string_view input,
                                const std::string_view text, const std::uint32_t amount) {
    record(input, text, amount);
    if (context.empty() || input.empty() || text.empty() || amount == 0) return;
    for (const auto suffix : context_suffixes_for_record(context)) {
        std::string group(suffix.text);
        group.push_back('\0');
        group += input;
        group.push_back('\0');
        auto key = group;
        key += text;
        record_bounded(
            language_context_counts_, key, amount,
            kLanguageContextCountCeiling,
            [&group](const std::string_view candidate) {
                return candidate.starts_with(group);
            });
    }
}

void UserFrequencyStore::set_sensitivity(const std::uint32_t sensitivity) noexcept {
    sensitivity_ = std::clamp(sensitivity, 1U, 10U);
}

void UserFrequencyStore::record(const std::string_view text, const std::uint32_t amount) {
    if (text.empty() || amount == 0) return;
    const std::string key(text);
    record_bounded(counts_, key, amount, kGlobalCountCeiling,
                   [](const std::string_view) { return true; });
}

std::uint32_t UserFrequencyStore::count(const std::string_view text) const {
    const auto found = counts_.find(std::string(text));
    return found == counts_.end() ? 0 : found->second;
}

std::uint32_t UserFrequencyStore::contextual_count(const std::string_view input,
                                                   const std::string_view text) const {
    std::string key(input);
    key.push_back('\0');
    key += text;
    const auto found = contextual_counts_.find(key);
    return found == contextual_counts_.end() ? 0 : found->second;
}

std::int64_t UserFrequencyStore::score(const std::string_view text) const {
    const auto gain = 250LL + static_cast<std::int64_t>(sensitivity_ - 1U) * 250LL;
    return bounded_learning_score(count(text), gain, 32);
}

std::int64_t UserFrequencyStore::contextual_score(const std::string_view input,
                                                  const std::string_view text) const {
    const auto gain = 500LL + static_cast<std::int64_t>(sensitivity_ - 1U) * 500LL;
    return bounded_learning_score(contextual_count(input, text), gain, 16);
}

std::int64_t UserFrequencyStore::language_context_score(
    const std::string_view context, const std::string_view input,
    const std::string_view text) const {
    const auto gain = 1'000LL + static_cast<std::int64_t>(sensitivity_ - 1U) * 1'000LL;
    std::int64_t best = 0;
    for (const auto suffix : context_suffixes_for_record(context)) {
        std::string key(suffix.text);
        key.push_back('\0');
        key += input;
        key.push_back('\0');
        key += text;
        const auto found = language_context_counts_.find(key);
        if (found == language_context_counts_.end()) continue;
        const auto score = bounded_learning_score(found->second, gain, 8);
        const auto weight = (std::min<std::size_t>)(
            100, 25 + suffix.character_count * 15);
        best = (std::max)(best, score * static_cast<std::int64_t>(weight) / 100);
    }
    return best;
}

std::vector<UserLexiconEntry> UserFrequencyStore::lookup(
    const std::string_view input, const std::size_t limit) const {
    if (input.empty() || limit == 0) return {};
    const auto found = user_lexicon_.find(compact_input_key(input));
    if (found == user_lexicon_.end()) return {};
    std::vector<UserLexiconEntry> entries;
    entries.reserve((std::min)(limit, found->second.size()));
    for (const auto& [text, count] : found->second)
        entries.push_back({text, count, found->first});
    const auto better = [](const UserLexiconEntry& left,
                           const UserLexiconEntry& right) {
        if (left.count != right.count) return left.count > right.count;
        return left.text < right.text;
    };
    if (entries.size() > limit) {
        std::nth_element(entries.begin(), entries.begin() + limit,
                         entries.end(), better);
        entries.resize(limit);
    }
    std::sort(entries.begin(), entries.end(), better);
    return entries;
}

std::vector<UserLexiconEntry> UserFrequencyStore::lookup_prefix(
    const std::string_view input, const std::size_t limit) const {
    const auto compact = compact_input_key(input);
    if (compact.size() < 2 || limit == 0) return {};
    std::vector<UserLexiconEntry> entries;
    for (const auto& [learned_input, words] : user_lexicon_) {
        if (learned_input.size() <= compact.size() ||
            !std::string_view(learned_input).starts_with(compact))
            continue;
        for (const auto& [text, count] : words)
            entries.push_back({text, count, learned_input});
    }
    const auto better = [](const UserLexiconEntry& left,
                           const UserLexiconEntry& right) {
        if (left.input.size() != right.input.size())
            return left.input.size() < right.input.size();
        if (left.count != right.count) return left.count > right.count;
        if (left.input != right.input) return left.input < right.input;
        return left.text < right.text;
    };
    if (entries.size() > limit) {
        std::nth_element(entries.begin(), entries.begin() + limit,
                         entries.end(), better);
        entries.resize(limit);
    }
    std::sort(entries.begin(), entries.end(), better);
    return entries;
}

UserFrequencyIoResult UserFrequencyStore::flush() const {
    if (path_.empty()) return {false, false, "user frequency path is not set"};
    const auto bytes = encode(counts_, contextual_counts_, language_context_counts_);
    if (bytes.size() > kMaximumFileBytes) return {false, false, "user frequency exceeds size limit"};
    auto temporary = path_; temporary += L".tmp";
    auto backup = path_; backup += L".bak";
    { std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
      if (!output) return {false, false, "cannot open temporary file"};
      output.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
      output.flush();
      if (!output) return {false, false, "cannot write temporary file"}; }
#ifdef _WIN32
    if (std::filesystem::exists(path_) && !CopyFileW(path_.c_str(), backup.c_str(), FALSE))
        return {false, false, "cannot update backup"};
    if (!MoveFileExW(temporary.c_str(), path_.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
        return {false, false, "cannot atomically replace user frequency"};
#else
    std::error_code error;
    if (std::filesystem::exists(path_)) std::filesystem::copy_file(path_, backup, std::filesystem::copy_options::overwrite_existing, error);
    std::filesystem::rename(temporary, path_, error);
    if (error) return {false, false, "cannot replace user frequency"};
#endif
    return {true, false, {}};
}

}  // namespace owo::engine

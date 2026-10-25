#include "owo/engine/user_frequency.h"
#include <algorithm>
#include <set>
#include <unordered_set>

namespace owo::engine {
namespace {
std::string compact(std::string_view input) {
    std::string result;
    for (auto c : input) {
        if (c == '\'')
            continue;
        if (c >= 'A' && c <= 'Z')
            c = static_cast<char>(c - 'A' + 'a');
        result += c;
    }
    return result;
}
std::string key(std::string_view input, std::string_view text) {
    auto result = compact(input);
    result.push_back('\0');
    result += text;
    return result;
}
bool valid_text(std::string_view text) {
    for (std::size_t i = 0; i < text.size();) {
        const auto c = static_cast<unsigned char>(text[i++]);
        if (c < 32 || c == 127)
            return false;
        if (c < 128)
            continue;
        unsigned n = 0;
        std::uint32_t value = 0, minimum = 0;
        if (c >= 0xc2 && c <= 0xdf) {
            n = 1;
            value = c & 31;
            minimum = 128;
        } else if (c >= 0xe0 && c <= 0xef) {
            n = 2;
            value = c & 15;
            minimum = 2048;
        } else if (c >= 0xf0 && c <= 0xf4) {
            n = 3;
            value = c & 7;
            minimum = 65536;
        } else
            return false;
        if (i + n > text.size())
            return false;
        while (n--) {
            const auto next = static_cast<unsigned char>(text[i++]);
            if ((next & 0xc0) != 0x80)
                return false;
            value = (value << 6) | (next & 63);
        }
        if (value < minimum || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff))
            return false;
    }
    return true;
}
} // namespace
std::string UserFrequencyStore::validate_word(const UserLexiconEntry &entry) {
    if (entry.input.empty() || entry.input.size() > 256 || compact(entry.input).empty())
        return "编码长度必须在 1–256 字节之间。";
    for (const auto c : entry.input)
        if (c != '\'' && !(c >= 'a' && c <= 'z') && !(c >= 'A' && c <= 'Z'))
            return "编码只支持英文字母和拼音分隔符。";
    if (entry.text.empty() || entry.text.size() > 1024 || !valid_text(entry.text))
        return "词语必须为有效 UTF-8，不能包含换行或控制字符，且不超过 1024 字节。";
    if (entry.count < 1 || entry.count > 128)
        return "权重必须在 1–128 之间。";
    return {};
}
UserLexiconPage UserFrequencyStore::list_words(std::string_view query, std::size_t offset,
                                               std::size_t limit) const {
    UserLexiconPage result;
    const auto search = compact(query);
    for (const auto &[input, words] : user_lexicon_)
        for (const auto &[text, count] : words) {
            if (!query.empty() && input.find(search) == input.npos && text.find(query) == text.npos)
                continue;
            const auto times = user_lexicon_times_.find(input);
            result.entries.push_back(
                {text, count, input,
                 times == user_lexicon_times_.end() || preferred_observation(text, times->second)});
        }
    std::sort(result.entries.begin(), result.entries.end(), [](const auto &a, const auto &b) {
        return a.input != b.input ? a.input < b.input : a.text < b.text;
    });
    result.total = result.entries.size();
    const auto begin = std::min(offset, result.total);
    const auto end = begin + std::min(limit, result.total - begin);
    result.entries = {result.entries.begin() + static_cast<std::ptrdiff_t>(begin),
                      result.entries.begin() + static_cast<std::ptrdiff_t>(end)};
    return result;
}
void UserFrequencyStore::clear_learning() {
    counts_.clear();
    contextual_counts_.clear();
    language_context_counts_.clear();
    singleton_times_.clear();
    contextual_singleton_times_.clear();
    language_singleton_times_.clear();
    rebuild_user_lexicon();
}
std::string UserFrequencyStore::edit_words(const std::vector<UserLexiconEntry> &entries,
                                           std::string_view old_input, std::string_view old_text,
                                           bool replace) {
    std::unordered_set<std::string> keys;
    std::unordered_set<std::string> affected;
    for (const auto &entry : entries) {
        auto error = validate_word(entry);
        if (!error.empty())
            return error;
        if (!keys.insert(key(entry.input, entry.text)).second)
            return "导入包含重复的编码和词语。";
        affected.insert(entry.text);
    }
    if (!old_input.empty() || !old_text.empty()) {
        auto error = validate_word({std::string(old_text), 1, std::string(old_input)});
        if (!error.empty())
            return error;
        keys.insert(key(old_input, old_text));
        affected.insert(std::string(old_text));
    }
    if (replace)
        clear_learning();
    const auto matches = [&keys](std::string_view stored, bool language) {
        if (language) {
            const auto context = stored.find('\0');
            if (context == stored.npos)
                return false;
            stored.remove_prefix(context + 1);
        }
        const auto split = stored.find('\0');
        return split != stored.npos &&
               keys.contains(key(stored.substr(0, split), stored.substr(split + 1)));
    };
    std::erase_if(contextual_counts_, [&](const auto &item) { return matches(item.first, false); });
    std::erase_if(contextual_singleton_times_,
                  [&](const auto &item) { return matches(item.first, false); });
    std::erase_if(language_context_counts_,
                  [&](const auto &item) { return matches(item.first, true); });
    std::erase_if(language_singleton_times_,
                  [&](const auto &item) { return matches(item.first, true); });
    for (const auto &entry : entries)
        contextual_counts_[key(entry.input, entry.text)] = entry.count;
    std::unordered_map<std::string, std::uint32_t> totals;
    for (const auto &[stored, count] : contextual_counts_) {
        const auto text = stored.substr(stored.find('\0') + 1);
        if (affected.contains(text))
            totals[text] = std::min(256U, totals[text] + count);
    }
    for (const auto &text : affected) {
        if (totals.contains(text))
            counts_[text] = totals[text];
        else
            counts_.erase(text);
        singleton_times_.erase(text);
    }
    rebuild_user_lexicon();
    return {};
}
} // namespace owo::engine

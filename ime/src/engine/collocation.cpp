#include "owo/engine/collocation.h"

#include <algorithm>
#include <array>
#include <vector>

namespace owo::engine {
namespace {

// Terminal punctuation ends a sentence, so nothing is predicted after it.
constexpr std::array<std::string_view, 9> kSentenceEnds{
    "。", "！", "？", "…", "!", "?", "；", ";", "\n"};

// A follower seen only once may be a slip of the tongue; it never drives a
// prediction.
constexpr std::uint32_t kMinimumFollowerCount = 2;

bool is_sentence_end(const std::string_view text) noexcept {
    return std::find(kSentenceEnds.begin(), kSentenceEnds.end(), text) !=
           kSentenceEnds.end();
}

bool ends_sentence(const std::string_view history) noexcept {
    if (history.empty()) return true;
    for (const auto end : kSentenceEnds) {
        if (history.ends_with(end)) return true;
    }
    return false;
}

std::vector<std::string_view> code_points(const std::string_view text) {
    std::vector<std::string_view> points;
    points.reserve(text.size());
    for (std::size_t index = 0; index < text.size();) {
        const auto lead = static_cast<unsigned char>(text[index]);
        std::size_t size = 1;
        if ((lead & 0xe0U) == 0xc0U) size = 2;
        else if ((lead & 0xf0U) == 0xe0U) size = 3;
        else if ((lead & 0xf8U) == 0xf0U) size = 4;
        size = (std::min)(size, text.size() - index);
        points.push_back(text.substr(index, size));
        index += size;
    }
    return points;
}

std::size_t code_point_count(const std::string_view text) noexcept {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](const char value) {
            return (static_cast<unsigned char>(value) & 0xc0U) != 0x80U;
        }));
}

}  // namespace

bool UserCollocationModel::best_follower(const std::string_view history,
                                         std::string& follower) const {
    const auto points = code_points(history);
    if (points.empty()) return false;
    // Prefer the longest context that has evidence. A shorter context may
    // predict a different word, so stop at the first hit instead of falling
    // back and contradicting the user's established phrasing.
    for (std::size_t length = std::min<std::size_t>(3, points.size()); length >= 1;
         --length) {
        std::string context;
        for (std::size_t index = points.size() - length; index < points.size(); ++index)
            context += points[index];
        const auto found = contexts_.find(context);
        if (found == contexts_.end()) return false;
        const auto& next = found->second.next;
        if (next.empty()) continue;
        const auto chosen = std::max_element(
            next.begin(), next.end(), [](const auto& left, const auto& right) {
                if (left.second != right.second) return left.second < right.second;
                return left.first > right.first;
            });
        if (chosen->second < kMinimumFollowerCount) return false;
        follower = chosen->first;
        return true;
    }
    return false;
}


void UserCollocationModel::observe(const std::string_view sentence) {
    const auto points = code_points(sentence);
    if (points.size() < 3) return;
    ++sentences_;
    for (std::size_t position = 0; position + 1 < points.size(); ++position) {
        // The follower stops at the sentence end so a continuation never
        // carries the closing punctuation.
        const auto follower = points[position + 1];
        if (is_sentence_end(follower)) continue;
        std::string context;
        for (std::size_t length = 1; length <= 3 && length <= position + 1; ++length) {
            context.insert(0, points[position + 1 - length]);
            auto found = contexts_.find(context);
            if (found == contexts_.end()) {
                if (contexts_.size() >= kMaximumCollocationContexts) {
                    // The index is full; stop learning new contexts rather
                    // than evicting evidence the user already produced.
                    return;
                }
                found = contexts_.emplace(context, Followers{}).first;
            }
            auto& entry = found->second;
            const auto existing = entry.next.find(std::string(follower));
            if (existing == entry.next.end() &&
                entry.next.size() >= kMaximumCollocationFollowers)
                continue;
            auto& count = entry.next[std::string(follower)];
            count = (std::min)(count + 1, kMaximumCollocationFollowerCount);
            entry.total = (std::min)(entry.total + 1, kMaximumCollocationFollowerCount);
        }
    }
}

void UserCollocationModel::clear() noexcept {
    contexts_.clear();
    sentences_ = 0;
}

std::string UserCollocationModel::continuation(const std::string_view history,
                                               const std::size_t maximum_bytes) const {
    if (contexts_.empty() || history.empty() || maximum_bytes == 0) return {};
    if (ends_sentence(history)) return {};
    std::string result;
    std::string context(history);
    while (result.size() < maximum_bytes) {
        std::string follower;
        if (!best_follower(context, follower)) break;
        if (result.size() + follower.size() > maximum_bytes) break;
        result += follower;
        context += follower;
    }
    // One character is not a prediction; the user already knows the next key.
    if (code_point_count(result) < 2) return {};
    return result;
}

UserCollocationModel build_user_collocation_model(
    const std::span<const UserCorpusEntry> entries) {
    UserCollocationModel model;
    // The store keeps entries in append order, so the tail is the newest
    // phrasing the user produced.
    const auto begin = entries.size() > kMaximumCollocationCorpusEntries
                           ? entries.size() - kMaximumCollocationCorpusEntries
                           : std::size_t{0};
    for (std::size_t index = begin; index < entries.size(); ++index)
        model.observe(entries[index].text);
    return model;
}

}  // namespace owo::engine

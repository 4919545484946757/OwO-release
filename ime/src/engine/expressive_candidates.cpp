#include "owo/engine/expressive_candidates.h"

#include <algorithm>
#include <array>
#include <unordered_set>

namespace owo::engine {
namespace {

struct ExpressiveKeyEntry {
    std::string_view key;
    ExpressiveAssetKind kind;
    std::string_view text;
    std::string_view annotation;
};

// Every entry is local-only. Keys are matched against the complete preedit so
// adding a key can never rewrite an unrelated word that merely starts with it.
constexpr std::array<ExpressiveKeyEntry, 53> kExpressiveKeys{{
    {"leisi", ExpressiveAssetKind::emoji, "😩", "表情：疲惫"},
    {"leisi", ExpressiveAssetKind::kaomoji, "(￣o￣) zZ", "颜文字：困倦"},
    {"leisile", ExpressiveAssetKind::emoji, "🥱", "表情：累坏了"},
    {"tired", ExpressiveAssetKind::emoji, "😩", "表情：疲惫"},
    {"haoxiao", ExpressiveAssetKind::emoji, "😂", "表情：大笑"},
    {"haoxiao", ExpressiveAssetKind::kaomoji, "(≧▽≦)", "颜文字：开心"},
    {"xiaosi", ExpressiveAssetKind::emoji, "🤣", "表情：笑翻"},
    {"lol", ExpressiveAssetKind::emoji, "😂", "表情：大笑"},
    {"kaixin", ExpressiveAssetKind::emoji, "😄", "表情：开心"},
    {"happy", ExpressiveAssetKind::emoji, "😊", "表情：愉快"},
    {"wuyu", ExpressiveAssetKind::emoji, "🙄", "表情：无语"},
    {"无语", ExpressiveAssetKind::emoji, "😑", "表情：无奈"},
    {"shengqi", ExpressiveAssetKind::emoji, "😠", "表情：生气"},
    {"angry", ExpressiveAssetKind::emoji, "💢", "表情：怒气"},
    {"zan", ExpressiveAssetKind::emoji, "👍", "表情：点赞"},
    {"good", ExpressiveAssetKind::emoji, "👌", "表情：不错"},
    {"xihuan", ExpressiveAssetKind::emoji, "❤", "表情：喜欢"},
    {"love", ExpressiveAssetKind::emoji, "🥰", "表情：爱意"},
    {"gongxi", ExpressiveAssetKind::emoji, "🎉", "表情：祝贺"},
    {"congrats", ExpressiveAssetKind::emoji, "🎊", "表情：庆祝"},
    {"jiayou", ExpressiveAssetKind::emoji, "💪", "表情：加油"},
    {"fighting", ExpressiveAssetKind::emoji, "🔥", "表情：斗志"},
    {"nanguo", ExpressiveAssetKind::emoji, "😢", "表情：难过"},
    {"sad", ExpressiveAssetKind::emoji, "😔", "表情：失落"},
    {"kun", ExpressiveAssetKind::emoji, "😴", "表情：困"},
    {"shuijiao", ExpressiveAssetKind::emoji, "💤", "表情：睡眠"},
    {"ok", ExpressiveAssetKind::emoji, "👌", "表情：可以"},
    {"xiexie", ExpressiveAssetKind::emoji, "🙏", "表情：感谢"},
    {"xiexie", ExpressiveAssetKind::kaomoji, "m(_ _)m", "颜文字：拜托"},
    {"thanks", ExpressiveAssetKind::emoji, "🙏", "表情：感谢"},
    {"buhaoyisi", ExpressiveAssetKind::emoji, "🙇", "表情：抱歉"},
    {"sorry", ExpressiveAssetKind::emoji, "😅", "表情：歉意"},
    {"ganbei", ExpressiveAssetKind::emoji, "🍻", "表情：干杯"},
    {"ganbei", ExpressiveAssetKind::kaomoji, "( ゜- ゜)つロ", "颜文字：干杯"},
    {"shengrikuaile", ExpressiveAssetKind::emoji, "🎂", "表情：生日"},
    {"birthday", ExpressiveAssetKind::emoji, "🎂", "表情：生日"},
    {"wanan", ExpressiveAssetKind::emoji, "🌙", "表情：晚安"},
    {"goodnight", ExpressiveAssetKind::emoji, "🌙", "表情：晚安"},
    {"zaoan", ExpressiveAssetKind::emoji, "☀", "表情：早安"},
    {"moyu", ExpressiveAssetKind::emoji, "🐟", "表情：摸鱼"},
    {"bailan", ExpressiveAssetKind::emoji, "🛌", "表情：摆烂"},
    {"pofang", ExpressiveAssetKind::emoji, "💔", "表情：破防"},
    {"liekai", ExpressiveAssetKind::emoji, "🤯", "表情：裂开"},
    {"baobao", ExpressiveAssetKind::emoji, "🤗", "表情：抱抱"},
    {"qinqin", ExpressiveAssetKind::emoji, "😘", "表情：亲亲"},
    {"jingya", ExpressiveAssetKind::emoji, "😱", "表情：震惊"},
    {"ganga", ExpressiveAssetKind::emoji, "😬", "表情：尴尬"},
    {"haixiu", ExpressiveAssetKind::emoji, "😳", "表情：害羞"},
    {"bixin", ExpressiveAssetKind::emoji, "💗", "表情：比心"},
    {"dacall", ExpressiveAssetKind::emoji, "📣", "表情：打call"},
    {"qiudai", ExpressiveAssetKind::emoji, "😣", "表情：求"},
    {"hungry", ExpressiveAssetKind::emoji, "🍚", "表情：饿"},
    {"kuqi", ExpressiveAssetKind::emoji, "😭", "表情：大哭"},
}};

class BuiltinExpressiveSource final : public ExpressiveAssetSource {
public:
    [[nodiscard]] std::vector<ExpressiveAsset> lookup(
        const std::string_view normalized_key,
        const std::size_t limit) const override {
        std::vector<ExpressiveAsset> assets;
        if (normalized_key.empty() || limit == 0) return assets;
        for (const auto& entry : kExpressiveKeys) {
            if (assets.size() >= limit) break;
            if (entry.key != normalized_key) continue;
            ExpressiveAsset asset;
            asset.kind = entry.kind;
            asset.text = std::string(entry.text);
            asset.annotation = std::string(entry.annotation);
            asset.local_only = true;
            assets.push_back(std::move(asset));
        }
        return assets;
    }
};

std::int64_t expressive_score(const ExpressiveAssetKind kind) {
    switch (kind) {
        case ExpressiveAssetKind::emoji: return kExpressiveEmojiScore;
        case ExpressiveAssetKind::kaomoji: return kExpressiveKaomojiScore;
        case ExpressiveAssetKind::image:
        case ExpressiveAssetKind::animation:
        case ExpressiveAssetKind::audio: return 0;
    }
    return 0;
}

}  // namespace

std::string expressive_lookup_key(const std::string_view raw) {
    std::string key;
    key.reserve(raw.size());
    for (const char character : raw) {
        if (character == ' ' || character == '\t' || character == '\r' ||
            character == '\n')
            continue;
        key.push_back(character >= 'A' && character <= 'Z'
                          ? static_cast<char>(character - 'A' + 'a')
                          : character);
    }
    return key;
}

const ExpressiveAssetSource& builtin_expressive_source() {
    static const BuiltinExpressiveSource source;
    return source;
}

std::vector<Candidate> generate_expressive_candidates(
    const std::string_view raw, const ExpressiveAssetSource& source,
    const std::size_t limit) {
    std::vector<Candidate> candidates;
    if (raw.empty() || limit == 0) return candidates;
    const auto assets = source.lookup(expressive_lookup_key(raw),
                                      std::min(limit, kExpressiveMaximumAssets));
    std::unordered_set<std::string> seen;
    for (const auto& asset : assets) {
        if (candidates.size() >= limit) break;
        // Media assets cannot be committed through the text-only candidate
        // path yet; they remain available to a host through lookup().
        const auto score = expressive_score(asset.kind);
        if (asset.text.empty() || score == 0) continue;
        if (!seen.insert(asset.text).second) continue;
        Candidate candidate;
        candidate.text = asset.text;
        candidate.score = score;
        candidate.match_kind = InputMatchKind::exact;
        candidate.consumed_input_bytes = raw.size();
        candidate.segment_count = 1;
        candidates.push_back(std::move(candidate));
    }
    return candidates;
}

std::vector<Candidate> generate_combination_candidates(
    const std::string_view text, const std::string_view expressive_text,
    const std::size_t consumed_bytes) {
    std::vector<Candidate> candidates;
    if (text.empty() || expressive_text.empty()) return candidates;
    Candidate candidate;
    candidate.text = std::string(text) + std::string(expressive_text);
    candidate.score = kExpressiveCombinationScore;
    candidate.match_kind = InputMatchKind::exact;
    candidate.consumed_input_bytes = consumed_bytes;
    candidate.segment_count = 1;
    candidates.push_back(std::move(candidate));
    return candidates;
}

}  // namespace owo::engine

#include "owo/engine/style_profile.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

namespace owo::engine {
namespace {

struct SceneNames {
    StyleScene scene;
    std::string_view id;
    std::string_view name;
};

constexpr std::array<SceneNames, 3> kSceneNames{{
    {StyleScene::chat, "chat", "聊天"},
    {StyleScene::work, "work", "工作"},
    {StyleScene::general, "general", "其他"},
}};

// Executable names are compared case-insensitively; only the file name is
// stored in the corpus, so these lists stay short and auditable.
constexpr std::array<std::string_view, 10> kChatApplications{
    "wechat.exe", "weixin.exe", "wechatappex.exe", "qq.exe", "qqnt.exe",
    "tim.exe", "telegram.exe", "discord.exe", "whatsapp.exe", "line.exe"};
constexpr std::array<std::string_view, 14> kWorkApplications{
    "winword.exe", "excel.exe", "powerpnt.exe", "outlook.exe", "onenote.exe",
    "wps.exe", "wpp.exe", "et.exe", "feishu.exe", "lark.exe", "dingtalk.exe",
    "teams.exe", "ms-teams.exe", "acrobat.exe"};

// Colloquial markers are matched as plain substrings. That can over-count a
// character that also appears inside an unrelated word (for example 吧 in
// 酒吧), so only markers seen at least twice are reported and the count is a
// habit indicator rather than a linguistic annotation.
constexpr std::array<std::string_view, 17> kColloquialMarkers{
    "哈哈", "嗯", "啊", "呀", "吧", "呢", "啦", "哦", "嘛",
    "诶", "唉", "咋", "啥", "其实", "感觉", "反正", "挺"};
// Words that are typical of written work communication; they push the score up.
constexpr std::array<std::string_view, 18> kFormalMarkers{
    "您", "敬请", "烦请", "感谢", "汇报", "方案", "纪要", "同步", "确认",
    "建议", "安排", "推进", "复盘", "对齐", "周报", "评审", "总结", "计划"};
constexpr std::uint32_t kMinimumMarkerCount = 2;

bool ascii_lower_equals(const std::string_view left, const std::string_view right) noexcept {
    if (left.size() != right.size()) return false;
    return std::equal(left.begin(), left.end(), right.begin(), [](const char l, const char r) {
        const auto lower = [](const char c) {
            return c >= 'A' && c <= 'Z' ? static_cast<char>(c - 'A' + 'a') : c;
        };
        return lower(l) == lower(r);
    });
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

std::string_view trim(const std::string_view text) noexcept {
    const auto blank = [](const char value) {
        return value == ' ' || value == '\t' || value == '\r' || value == '\n';
    };
    std::string_view result = text;
    while (!result.empty() && blank(result.front())) result.remove_prefix(1);
    while (!result.empty() && blank(result.back())) result.remove_suffix(1);
    return result;
}

bool terminal_punctuation(const std::string_view point) noexcept {
    return point == "。" || point == "！" || point == "？" || point == "…" ||
           point == "!" || point == "?";
}

std::size_t count_occurrences(const std::string_view text,
                              const std::string_view needle) noexcept {
    if (needle.empty()) return 0;
    std::size_t count = 0;
    for (auto found = text.find(needle); found != std::string_view::npos;
         found = text.find(needle, found + needle.size()))
        ++count;
    return count;
}

std::size_t count_points(const std::vector<std::string_view>& points,
                         const std::string_view needle) noexcept {
    return static_cast<std::size_t>(std::count(points.begin(), points.end(), needle));
}

int clamp_delta(const double value, const int low, const int high) noexcept {
    const auto rounded = static_cast<int>(std::lround(value));
    return (std::max)(low, (std::min)(high, rounded));
}

// Integer formatting keeps the report identical regardless of the C locale,
// and the settings center compares these strings.
std::string one_decimal(const double value) {
    const auto scaled = static_cast<long long>(std::llround(value * 10.0));
    const auto whole = scaled / 10;
    const auto fraction = scaled < 0 ? -scaled % 10 : scaled % 10;
    return std::to_string(whole) + "." + std::to_string(fraction);
}

std::string percent(const double ratio) {
    return std::to_string(static_cast<int>(std::lround(ratio * 100.0))) + "%";
}

bool is_valid_utf8(const std::string_view text) noexcept {
    std::size_t index = 0;
    while (index < text.size()) {
        const auto lead = static_cast<unsigned char>(text[index]);
        std::size_t length = 0;
        std::uint32_t scalar = 0;
        if (lead < 0x80U) { ++index; continue; }
        if ((lead & 0xe0U) == 0xc0U) { length = 2; scalar = lead & 0x1fU; }
        else if ((lead & 0xf0U) == 0xe0U) { length = 3; scalar = lead & 0x0fU; }
        else if ((lead & 0xf8U) == 0xf0U) { length = 4; scalar = lead & 0x07U; }
        else return false;
        if (index + length > text.size()) return false;
        for (std::size_t offset = 1; offset < length; ++offset) {
            const auto next = static_cast<unsigned char>(text[index + offset]);
            if ((next & 0xc0U) != 0x80U) return false;
            scalar = (scalar << 6U) | (next & 0x3fU);
        }
        if ((length == 2 && scalar < 0x80U) || (length == 3 && scalar < 0x800U) ||
            (length == 4 && (scalar < 0x10000U || scalar > 0x10ffffU)) ||
            (scalar >= 0xd800U && scalar <= 0xdfffU)) return false;
        index += length;
    }
    return true;
}

struct SceneAccumulator {
    std::size_t sentences{};
    std::size_t characters{};
    std::size_t short_sentences{};
    std::size_t terminal_endings{};
    std::size_t exclamations{};
    std::size_t questions{};
    std::size_t ellipses{};
    std::size_t commas{};
    std::size_t colloquial{};
    std::size_t formal{};
    std::unordered_map<std::string, std::uint32_t> markers;
    std::unordered_map<std::string, std::uint32_t> applications;
};

void observe(const UserCorpusEntry& entry, SceneAccumulator& accumulator) {
    const auto text = trim(entry.text);
    if (text.empty() || !is_valid_utf8(text)) return;
    const auto points = code_points(text);
    if (points.empty()) return;
    ++accumulator.sentences;
    accumulator.characters += points.size();
    if (points.size() <= 12) ++accumulator.short_sentences;
    if (terminal_punctuation(points.back())) ++accumulator.terminal_endings;
    accumulator.exclamations += count_points(points, "！") + count_points(points, "!");
    accumulator.questions += count_points(points, "？") + count_points(points, "?");
    accumulator.ellipses += count_points(points, "…");
    accumulator.commas += count_points(points, "，") + count_points(points, ",");
    for (const auto marker : kColloquialMarkers) {
        const auto count = count_occurrences(text, marker);
        if (count == 0) continue;
        accumulator.colloquial += count;
        accumulator.markers[std::string(marker)] += static_cast<std::uint32_t>(count);
    }
    for (const auto marker : kFormalMarkers) accumulator.formal += count_occurrences(text, marker);
    if (!entry.application.empty())
        accumulator.applications[entry.application] += 1;
}

void add_signal(StyleSceneProfile& profile, std::string label, const int delta) {
    if (delta == 0) return;
    profile.formality_signals.push_back({std::move(label), delta});
}

StyleSceneProfile finish(const SceneNames& names, const SceneAccumulator& accumulator) {
    StyleSceneProfile profile;
    profile.scene = names.scene;
    profile.sentence_count = accumulator.sentences;
    if (accumulator.sentences == 0) return profile;
    const auto sentences = static_cast<double>(accumulator.sentences);
    profile.average_characters = static_cast<double>(accumulator.characters) / sentences;
    profile.short_sentence_ratio =
        static_cast<double>(accumulator.short_sentences) / sentences;
    profile.terminal_punctuation_ratio =
        static_cast<double>(accumulator.terminal_endings) / sentences;
    profile.exclamation_per_sentence = static_cast<double>(accumulator.exclamations) / sentences;
    profile.question_per_sentence = static_cast<double>(accumulator.questions) / sentences;
    profile.ellipsis_per_sentence = static_cast<double>(accumulator.ellipses) / sentences;
    profile.comma_per_sentence = static_cast<double>(accumulator.commas) / sentences;

    std::vector<StyleMarker> markers;
    for (const auto& [text, count] : accumulator.markers) {
        if (count < kMinimumMarkerCount) continue;
        markers.push_back({text, count});
    }
    std::sort(markers.begin(), markers.end(), [](const auto& left, const auto& right) {
        if (left.count != right.count) return left.count > right.count;
        return left.text < right.text;
    });
    if (markers.size() > kMaximumStyleMarkers)
        markers.resize(kMaximumStyleMarkers);
    profile.markers = std::move(markers);

    std::vector<std::pair<std::string, std::uint32_t>> applications(
        accumulator.applications.begin(), accumulator.applications.end());
    std::sort(applications.begin(), applications.end(),
              [](const auto& left, const auto& right) {
                  if (left.second != right.second) return left.second > right.second;
                  return left.first < right.first;
              });
    for (std::size_t index = 0; index < applications.size() && index < 4; ++index)
        profile.applications.push_back(applications[index].first);

    if (accumulator.sentences < kMinimumStyleSampleSentences) return profile;

    // Every contribution is a small, named delta around a neutral base, so
    // the reported score can be re-derived by hand from the same numbers.
    int score = 50;
    const auto punctuation = clamp_delta(
        (profile.terminal_punctuation_ratio - 0.6) * 25.0, -10, 10);
    score += punctuation;
    add_signal(profile, "句末带标点 " + percent(profile.terminal_punctuation_ratio),
               punctuation);
    const auto exclamations = -clamp_delta(profile.exclamation_per_sentence * 12.0, 0, 12);
    score += exclamations;
    add_signal(profile, "每句感叹号 " + one_decimal(profile.exclamation_per_sentence),
               exclamations);
    const auto ellipses = -clamp_delta(profile.ellipsis_per_sentence * 8.0, 0, 8);
    score += ellipses;
    add_signal(profile, "每句省略号 " + one_decimal(profile.ellipsis_per_sentence), ellipses);
    const auto commas = clamp_delta((profile.comma_per_sentence - 0.8) * 5.0, -4, 6);
    score += commas;
    add_signal(profile, "每句逗号 " + one_decimal(profile.comma_per_sentence), commas);
    int length_delta = 0;
    if (profile.average_characters >= 24.0) length_delta = 8;
    else if (profile.average_characters >= 16.0) length_delta = 4;
    else if (profile.average_characters < 8.0) length_delta = -6;
    else if (profile.average_characters < 12.0) length_delta = -2;
    score += length_delta;
    add_signal(profile, "平均句长 " + one_decimal(profile.average_characters) + " 字",
               length_delta);
    const auto colloquial = -clamp_delta(
        static_cast<double>(accumulator.colloquial) / sentences * 5.0, 0, 10);
    score += colloquial;
    add_signal(profile, "每句口头语 " +
                   one_decimal(static_cast<double>(accumulator.colloquial) / sentences) + " 处",
               colloquial);
    const auto formal = clamp_delta(
        static_cast<double>(accumulator.formal) / sentences * 6.0, 0, 8);
    score += formal;
    add_signal(profile, "每句工作用语 " +
                   one_decimal(static_cast<double>(accumulator.formal) / sentences) + " 处",
               formal);
    profile.formality = (std::max)(0, (std::min)(100, score));
    return profile;
}

}  // namespace

StyleScene style_scene_for_application(const std::string_view executable) noexcept {
    for (const auto application : kChatApplications)
        if (ascii_lower_equals(executable, application)) return StyleScene::chat;
    for (const auto application : kWorkApplications)
        if (ascii_lower_equals(executable, application)) return StyleScene::work;
    return StyleScene::general;
}

std::string_view style_scene_id(const StyleScene scene) noexcept {
    for (const auto& names : kSceneNames)
        if (names.scene == scene) return names.id;
    return {};
}

std::string_view style_scene_name(const StyleScene scene) noexcept {
    for (const auto& names : kSceneNames)
        if (names.scene == scene) return names.name;
    return {};
}

std::uint32_t style_scene_bit(const StyleScene scene) noexcept {
    return std::uint32_t{1} << static_cast<std::uint32_t>(scene);
}

StyleProfile build_style_profile(const std::span<const UserCorpusEntry> entries,
                                 const std::uint32_t scene_mask) {
    StyleProfile profile;
    profile.total_sentences = entries.size();
    std::array<SceneAccumulator, 3> accumulators{};
    const auto begin = entries.size() > kMaximumStyleProfileCorpusEntries
                           ? entries.size() - kMaximumStyleProfileCorpusEntries
                           : std::size_t{0};
    for (std::size_t index = begin; index < entries.size(); ++index) {
        const auto& entry = entries[index];
        const auto scene = static_cast<std::size_t>(style_scene_for_application(entry.application));
        if ((scene_mask & style_scene_bit(static_cast<StyleScene>(scene))) == 0) continue;
        observe(entry, accumulators[scene]);
    }
    for (const auto& names : kSceneNames) {
        const auto& accumulator = accumulators[static_cast<std::size_t>(names.scene)];
        if (accumulator.sentences == 0) continue;
        profile.scenes.push_back(finish(names, accumulator));
    }
    return profile;
}

std::string describe_style_scene(const StyleSceneProfile& scene) {
    std::string text;
    text += "【";
    text += style_scene_name(scene.scene);
    text += "】";
    if (!scene.applications.empty()) {
        text += " ";
        for (std::size_t index = 0; index < scene.applications.size(); ++index) {
            if (index != 0) text += "、";
            text += scene.applications[index];
        }
        text += " 等";
    }
    text += "\n- 样本：";
    text += std::to_string(scene.sentence_count);
    text += " 句\n- 平均句长：";
    text += one_decimal(scene.average_characters);
    text += " 字；短句（≤12 字）占 ";
    text += percent(scene.short_sentence_ratio);
    text += "\n- 句末带标点：";
    text += percent(scene.terminal_punctuation_ratio);
    text += "\n- 每句：感叹号 ";
    text += one_decimal(scene.exclamation_per_sentence);
    text += "、问号 ";
    text += one_decimal(scene.question_per_sentence);
    text += "、省略号 ";
    text += one_decimal(scene.ellipsis_per_sentence);
    text += "、逗号 ";
    text += one_decimal(scene.comma_per_sentence);
    text += "\n- 常用口头语：";
    if (scene.markers.empty()) {
        text += "暂无明显重复";
    } else {
        for (std::size_t index = 0; index < scene.markers.size(); ++index) {
            if (index != 0) text += "、";
            text += scene.markers[index].text;
            text += "（" + std::to_string(scene.markers[index].count) + "）";
        }
    }
    text += "\n- 正式程度：";
    if (scene.formality < 0) {
        text += "样本不足（至少 ";
        text += std::to_string(kMinimumStyleSampleSentences);
        text += " 句）";
    } else {
        text += std::to_string(scene.formality);
        text += "/100";
        if (scene.formality <= 35) text += "（偏口语）";
        else if (scene.formality >= 65) text += "（偏正式）";
        else text += "（居中）";
        for (const auto& signal : scene.formality_signals) {
            text += "\n  - ";
            text += signal.label;
            text += signal.delta > 0 ? "（+" : "（";
            text += std::to_string(signal.delta);
            text += "）";
        }
    }
    text += "\n";
    return text;
}

std::string describe_style_profile(const StyleProfile& profile) {
    if (profile.total_sentences == 0) {
        return "还没有可用的语料。打开授权语料学习并正常输入后，这里会按场景给出写作风格。\n";
    }
    if (profile.scenes.empty()) {
        return "语料不为空，但选中的场景里没有句子。可在设置中打开其他场景，或继续输入。\n";
    }
    std::string text;
    for (const auto& scene : profile.scenes) text += describe_style_scene(scene);
    return text;
}

}  // namespace owo::engine

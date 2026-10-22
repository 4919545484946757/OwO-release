#include "owo/engine/tone_draft.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <string>
#include <unordered_set>
#include <utility>
#include <vector>

namespace owo::engine {
namespace {

std::string_view trim(const std::string_view text) noexcept {
    const auto blank = [](const char value) {
        return value == ' ' || value == '\t' || value == '\r' || value == '\n';
    };
    std::string_view result = text;
    while (!result.empty() && blank(result.front())) result.remove_prefix(1);
    while (!result.empty() && blank(result.back())) result.remove_suffix(1);
    return result;
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

std::vector<std::string> code_points(const std::string_view text) {
    std::vector<std::string> points;
    points.reserve(text.size());
    for (std::size_t index = 0; index < text.size();) {
        const auto lead = static_cast<unsigned char>(text[index]);
        std::size_t size = 1;
        if ((lead & 0xe0U) == 0xc0U) size = 2;
        else if ((lead & 0xf0U) == 0xe0U) size = 3;
        else if ((lead & 0xf8U) == 0xf0U) size = 4;
        size = (std::min)(size, text.size() - index);
        points.emplace_back(text.substr(index, size));
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

// Integer formatting keeps the prompt identical across locales.
std::string one_decimal(const double value) {
    const auto scaled = static_cast<long long>(std::llround(value * 10.0));
    const auto whole = scaled / 10;
    const auto fraction = scaled % 10 < 0 ? -(scaled % 10) : scaled % 10;
    return std::to_string(whole) + "." + std::to_string(fraction);
}

std::string percent(const double ratio) {
    return std::to_string(static_cast<int>(std::lround(ratio * 100.0))) + "%";
}

// The prompt is assembled from the user's own text. Section brackets inside it
// are flattened so a message cannot forge a section header; this is a
// formatting guard, not a substitute for the fact check.
std::string prompt_safe(const std::string_view text) {
    std::string result(text);
    std::size_t offset = 0;
    while ((offset = result.find("\xE3\x80\x90", offset)) != std::string::npos) {
        result.replace(offset, 3, "[");
        offset += 1;
    }
    offset = 0;
    while ((offset = result.find("\xE3\x80\x91", offset)) != std::string::npos) {
        result.replace(offset, 3, "]");
        offset += 1;
    }
    return result;
}

std::string digest_for(const StyleSceneProfile& scene) {
    std::string digest = "平均 " + one_decimal(scene.average_characters) + " 字；短句（≤12 字）占 " +
        percent(scene.short_sentence_ratio) + "；句末带标点 " +
        percent(scene.terminal_punctuation_ratio) + "；每句感叹号 " +
        one_decimal(scene.exclamation_per_sentence) + "、问号 " +
        one_decimal(scene.question_per_sentence) + "、省略号 " +
        one_decimal(scene.ellipsis_per_sentence) + "、逗号 " +
        one_decimal(scene.comma_per_sentence);
    if (scene.formality >= 0) {
        digest += "；正式程度 " + std::to_string(scene.formality) + "/100（";
        digest += scene.formality <= 35 ? "偏口语"
                : scene.formality >= 65 ? "偏正式" : "居中";
        digest += "）";
    } else {
        digest += "；正式程度：样本不足";
    }
    if (!scene.markers.empty()) {
        digest += "；常用口头语：";
        for (std::size_t index = 0; index < scene.markers.size(); ++index) {
            if (index != 0) digest += "、";
            digest += scene.markers[index].text;
        }
    }
    return digest;
}

std::vector<std::string> message_units(const std::vector<std::string>& points) {
    std::vector<std::string> units;
    if (points.size() < 2) return units;
    units.reserve(points.size() - 1);
    for (std::size_t index = 0; index + 1 < points.size(); ++index)
        units.push_back(points[index] + points[index + 1]);
    std::sort(units.begin(), units.end());
    units.erase(std::unique(units.begin(), units.end()), units.end());
    return units;
}

struct FactTokens {
    std::unordered_set<std::string> numbers;
    std::unordered_set<std::string> words;
};

bool ascii_digit(const char value) noexcept { return value >= '0' && value <= '9'; }
bool ascii_letter(const char value) noexcept {
    return (value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z');
}

FactTokens fact_tokens(const std::string_view text) {
    FactTokens tokens;
    for (std::size_t index = 0; index < text.size();) {
        const char value = text[index];
        if (ascii_digit(value)) {
            std::size_t end = index;
            while (end < text.size() && ascii_digit(text[end])) ++end;
            tokens.numbers.emplace(text.substr(index, end - index));
            index = end;
            continue;
        }
        if (ascii_letter(value)) {
            std::size_t end = index;
            while (end < text.size() && ascii_letter(text[end])) ++end;
            std::string word(text.substr(index, end - index));
            std::transform(word.begin(), word.end(), word.begin(), [](const char c) {
                return c >= 'A' && c <= 'Z' ? static_cast<char>(c - 'A' + 'a') : c;
            });
            if (word.size() >= 2) tokens.words.insert(std::move(word));
            index = end;
            continue;
        }
        ++index;
    }
    return tokens;
}

// Longest common substring of two code-point sequences, in code points. The
// run itself is returned through `shared`.
std::size_t longest_shared_run(const std::vector<std::string>& left,
                               const std::vector<std::string>& right,
                               std::string& shared) {
    std::size_t best = 0;
    std::size_t best_left = 0;
    std::vector<std::size_t> previous(right.size() + 1, 0);
    std::vector<std::size_t> current(right.size() + 1, 0);
    for (std::size_t i = 1; i <= left.size(); ++i) {
        for (std::size_t j = 1; j <= right.size(); ++j) {
            current[j] = left[i - 1] == right[j - 1] ? previous[j - 1] + 1 : 0;
            if (current[j] > best) {
                best = current[j];
                best_left = i;
            }
        }
        std::swap(previous, current);
        std::fill(current.begin(), current.end(), 0);
    }
    shared.clear();
    if (best == 0) return 0;
    for (std::size_t index = best_left - best; index < best_left; ++index)
        shared += left[index];
    return best;
}

// Models often number their lines or wrap them in quotes despite the
// instruction. Removing that presentation keeps the inserted text exactly what
// the user asked for; the fact check runs on the normalized text.
std::string normalize_draft(const std::string_view raw) {
    std::string_view value = trim(raw);
    const auto drop = [&value](const std::size_t count) {
        value = trim(value.substr((std::min)(count, value.size())));
    };
    std::size_t digits = 0;
    while (digits < value.size() && ascii_digit(value[digits])) ++digits;
    if (digits != 0 && digits <= 2 && digits < value.size()) {
        // The marker after the number may be ASCII or a full-width CJK mark,
        // so it is matched by length rather than by a one-byte comparison.
        std::size_t marker = 0;
        const auto next = value.substr(digits);
        if (next.starts_with(".") || next.starts_with(")") || next.starts_with("]"))
            marker = 1;
        else if (next.starts_with("\xEF\xBC\x89") || next.starts_with("\xE3\x80\x81") ||
                 next.starts_with("\xEF\xBC\x8E") || next.starts_with("\xEF\xBC\x9A"))
            marker = 3;
        if (marker != 0) drop(digits + marker);
    } else if (value.size() >= 3 && value[0] == '(' && value[2] == ')') {
        drop(3);
    } else if (value.starts_with("\xEF\xBC\x88") && value.size() >= 6 &&
               value.substr(3, 3) == "\xEF\xBC\x89") {
        drop(6);
    } else if (value.size() >= 2 && value[0] == '-' && value[1] == ' ') {
        drop(2);
    } else if (value.starts_with("\xE2\x80\xA2 ")) {
        drop(4);
    }
    const auto wrapped = [&value](const std::string_view open, const std::string_view close) {
        return value.size() > open.size() + close.size() && value.starts_with(open) &&
               value.ends_with(close);
    };
    if (wrapped("\"", "\"")) value = trim(value.substr(1, value.size() - 2));
    else if (wrapped("\xE2\x80\x9C", "\xE2\x80\x9D"))
        value = trim(value.substr(3, value.size() - 6));
    else if (wrapped("\xE3\x80\x8C", "\xE3\x80\x8D"))
        value = trim(value.substr(3, value.size() - 6));
    else if (wrapped("\xE3\x80\x8E", "\xE3\x80\x8F"))
        value = trim(value.substr(3, value.size() - 6));
    return std::string(value);
}

std::string_view message_section_header(const ToneDraftMode mode) noexcept {
    return mode == ToneDraftMode::continuation
               ? "【用户已经写下的开头（请接着写完）】"
               : "【当前要回复的消息】";
}

}  // namespace

ToneDraftPlan build_tone_draft_plan(const std::span<const UserCorpusEntry> entries,
                                    const std::string_view message,
                                    const std::string_view executable,
                                    const std::uint32_t scene_mask,
                                    const std::string_view preferences,
                                    const ToneDraftMode mode,
                                    const std::size_t maximum_examples) {
    return build_tone_draft_plan(entries, message, executable, scene_mask,
                                 preferences, mode, maximum_examples, {});
}

ToneDraftPlan build_tone_draft_plan(const std::span<const UserCorpusEntry> entries,
                                    const std::string_view message,
                                    const std::string_view executable,
                                    const std::uint32_t scene_mask,
                                    const std::string_view preferences,
                                    const ToneDraftMode mode,
                                    const std::size_t maximum_examples,
                                    const std::span<const std::string> feedback_hints) {
    ToneDraftPlan plan;
    plan.scene = style_scene_for_application(executable);
    plan.mode = mode;
    const auto trimmed = trim(message);
    if (trimmed.empty()) {
        plan.diagnostic = mode == ToneDraftMode::continuation
                              ? "请先写下要续写的开头。"
                              : "请先选中或输入要回复的消息。";
        return plan;
    }
    if (!is_valid_utf8(trimmed)) {
        plan.diagnostic = "当前消息不是有效的 UTF-8 文本。";
        return plan;
    }
    if (trimmed.size() > kMaximumToneMessageBytes) {
        plan.diagnostic = "当前消息超过 " + std::to_string(kMaximumToneMessageBytes) +
                          " 字节，请先缩短。";
        return plan;
    }
    if ((scene_mask & style_scene_bit(plan.scene)) == 0) {
        plan.diagnostic = "该场景已在设置中关闭，不会取用它的例句。";
        return plan;
    }
    if (!preferences.empty() &&
        (preferences.size() > kMaximumTonePreferencesBytes || !is_valid_utf8(preferences))) {
        plan.diagnostic = "手写偏好无效，无法组成本次请求。";
        return plan;
    }

    // The profile reads the newest entries only, matching the viewer and the
    // collocation model so all three describe the same part of the corpus.
    const auto begin = entries.size() > kMaximumStyleProfileCorpusEntries
                           ? entries.size() - kMaximumStyleProfileCorpusEntries
                           : std::size_t{0};
    const auto profile = build_style_profile(entries, scene_mask);
    const StyleSceneProfile* scene_profile = nullptr;
    for (const auto& value : profile.scenes)
        if (value.scene == plan.scene) scene_profile = &value;
    if (scene_profile == nullptr || scene_profile->sentence_count == 0) {
        plan.diagnostic = "该场景还没有可用句子。打开授权语料学习并正常输入后，这里才能按语气起草。";
        return plan;
    }
    plan.style_digest = digest_for(*scene_profile);

    const auto message_text = std::string(trimmed);
    const auto message_points = code_points(message_text);
    const auto units = message_units(message_points);
    std::unordered_set<std::string> message_units_set(units.begin(), units.end());
    std::vector<ToneExample> candidates;
    std::unordered_set<std::string> seen;
    for (std::size_t index = entries.size(); index > begin; --index) {
        const auto& entry = entries[index - 1];
        if (style_scene_for_application(entry.application) != plan.scene) continue;
        if (!is_valid_utf8(entry.text)) continue;
        const auto text = trim(entry.text);
        if (code_point_count(text) < 2) continue;
        if (text == trimmed) continue;
        if (!seen.insert(std::string(text)).second) continue;
        std::uint32_t shared = 0;
        if (!units.empty()) {
            const auto points = code_points(text);
            std::unordered_set<std::string> entry_units;
            for (std::size_t point = 0; point + 1 < points.size(); ++point) {
                const auto unit = points[point] + points[point + 1];
                if (entry_units.insert(unit).second && message_units_set.contains(unit)) ++shared;
            }
        }
        candidates.push_back({std::string(text), shared, entry.id});
    }
    std::sort(candidates.begin(), candidates.end(),
              [](const ToneExample& left, const ToneExample& right) {
                  if (left.shared_units != right.shared_units)
                      return left.shared_units > right.shared_units;
                  if (left.entry_id != right.entry_id) return left.entry_id > right.entry_id;
                  return left.text < right.text;
              });
    const auto wanted = std::clamp<std::size_t>(maximum_examples, 1, kMaximumToneExamples);
    if (candidates.size() > wanted) candidates.resize(wanted);
    plan.examples = std::move(candidates);
    plan.ready = true;

    plan.prompt = "请按用户本人的语气起草一条中文回复。\n\n【场景】";
    plan.prompt.append(style_scene_name(plan.scene));
    plan.prompt += "\n【这位用户的写作习惯】\n";
    plan.prompt += plan.style_digest;
    plan.prompt += "\n";
    if (!preferences.empty()) {
        plan.prompt += "【用户自己写下的偏好】\n";
        plan.prompt += prompt_safe(preferences);
        plan.prompt += "\n";
    }
    if (!plan.examples.empty()) {
        plan.prompt += "【用户以前在同类场景写过的句子（只看语气，不是事实来源）】\n";
        for (const auto& example : plan.examples) {
            plan.prompt += "- ";
            plan.prompt += prompt_safe(example.text);
            plan.prompt += "\n";
        }
    }
    if (!feedback_hints.empty()) {
        plan.prompt += "【用户对此前草稿的修改（请照做）】\n";
        for (const auto& hint : feedback_hints) {
            plan.prompt += "- ";
            plan.prompt += prompt_safe(hint);
            plan.prompt += "\n";
        }
    }
    plan.prompt += message_section_header(mode);
    plan.prompt += "\n";
    plan.prompt += prompt_safe(message_text);
    plan.prompt += "\n【要求】\n"
                   "1. 生成 2–3 条可以直接发出去的中文回复，每条单独一行；不要编号、引号、解释或前后缀。\n"
                   "2. 事实（时间、地点、人名、数字、链接、金额等）只能来自当前消息，"
                   "不能编造，也不能从例句里搬。\n";
    if (mode == ToneDraftMode::continuation) {
        plan.prompt += "3. 每条回复都必须从用户已经写下的开头原样开始，只往后接，不要改写开头。\n";
    } else {
        plan.prompt += "3. 句长、标点、口头语要向这位用户的习惯和偏好靠拢，意思必须清楚。\n";
    }
    plan.prompt += mode == ToneDraftMode::continuation
                       ? "4. 只用中文；完整的一句或几句，不超过 80 字。\n"
                       : "4. 只用中文回答；每条不超过 80 字。\n";
    if (plan.prompt.size() > kMaximumTonePromptBytes) {
        plan.ready = false;
        plan.examples.clear();
        plan.prompt.clear();
        plan.diagnostic = "组装的请求超过大小上限，请缩短消息或手写偏好。";
    }
    return plan;
}

ToneDraftCheck check_tone_draft(const std::string_view draft,
                                const std::string_view message,
                                const std::span<const std::string> examples,
                                const ToneDraftMode mode) {
    ToneDraftCheck check;
    const auto value = trim(draft);
    if (value.empty()) {
        check.reason = "空回复";
        return check;
    }
    if (!is_valid_utf8(value)) {
        check.reason = "不是有效的 UTF-8 文本";
        return check;
    }
    if (value.size() > kMaximumToneDraftBytes) {
        check.reason = "回复过长";
        return check;
    }
    if (value.find_first_of("\r\n") != std::string_view::npos) {
        check.reason = "回复不是单独一行";
        return check;
    }
    const auto message_value = trim(message);
    if (mode == ToneDraftMode::continuation) {
        // The draft replaces the composition, so it must extend the opening
        // rather than rewrite it.
        if (!value.starts_with(message_value) || value.size() == message_value.size()) {
            check.reason = "没有接着用户写下的开头";
            return check;
        }
    } else if (value == message_value) {
        check.reason = "与当前消息完全相同";
        return check;
    }

    const auto message_tokens = fact_tokens(message_value);
    const auto draft_tokens = fact_tokens(value);
    // Numbers are the highest-risk invented fact, so they must appear in the
    // message itself. A Latin word the user already uses in their examples is
    // part of their vocabulary rather than a new fact.
    for (const auto& number : draft_tokens.numbers)
        if (!message_tokens.numbers.contains(number)) check.unsupported_facts.push_back(number);
    std::unordered_set<std::string> example_words;
    for (const auto& example : examples) {
        const auto tokens = fact_tokens(example);
        example_words.insert(tokens.words.begin(), tokens.words.end());
    }
    for (const auto& word : draft_tokens.words)
        if (!message_tokens.words.contains(word) && !example_words.contains(word))
            check.unsupported_facts.push_back(word);
    std::sort(check.unsupported_facts.begin(), check.unsupported_facts.end());
    check.unsupported_facts.erase(
        std::unique(check.unsupported_facts.begin(), check.unsupported_facts.end()),
        check.unsupported_facts.end());

    const auto draft_points = code_points(value);
    for (const auto& example : examples) {
        std::string shared;
        const auto length = longest_shared_run(draft_points, code_points(example), shared);
        if (length < kMinimumCopiedExampleCodes) continue;
        if (message_value.find(shared) != std::string_view::npos) continue;
        check.copied_example_codes = (std::max)(check.copied_example_codes, length);
    }
    if (!check.unsupported_facts.empty()) {
        check.reason = "含当前消息里没有的数字或英文（" +
                       std::to_string(check.unsupported_facts.size()) + " 处）";
    } else if (check.copied_example_codes != 0) {
        check.reason = "照搬了例句里的内容";
    }
    check.acceptable = check.unsupported_facts.empty() && check.copied_example_codes == 0;
    return check;
}

ToneDraftSelection filter_tone_drafts(const std::span<const std::string> drafts,
                                      const std::string_view message,
                                      const std::span<const std::string> examples,
                                      const ToneDraftMode mode,
                                      const std::size_t maximum) {
    ToneDraftSelection selection;
    const auto limit = std::clamp<std::size_t>(maximum, 1, kMaximumToneDrafts);
    std::unordered_set<std::string> seen;
    std::size_t index = 0;
    for (const auto& raw : drafts) {
        // A provider may return all replies in one string; each line is one
        // candidate, and an empty line carries nothing.
        std::size_t begin = 0;
        while (begin <= raw.size()) {
            const auto end = raw.find('\n', begin);
            const auto line = std::string_view(raw).substr(
                begin, end == std::string::npos ? raw.size() - begin : end - begin);
            ++index;
            const auto value = normalize_draft(line);
            if (!value.empty()) {
                const auto check = check_tone_draft(value, message, examples, mode);
                if (!check.acceptable) {
                    selection.rejected.push_back("第 " + std::to_string(index) + " 条：" +
                                                 check.reason);
                } else if (!seen.insert(value).second) {
                    selection.rejected.push_back("第 " + std::to_string(index) +
                                                 " 条：与前面的候选重复");
                } else if (selection.candidates.size() >= limit) {
                    selection.rejected.push_back("第 " + std::to_string(index) + " 条：超出 " +
                                                 std::to_string(limit) + " 条上限");
                } else {
                    selection.candidates.push_back({value});
                }
            }
            if (end == std::string::npos) break;
            begin = end + 1;
        }
    }
    return selection;
}

}  // namespace owo::engine

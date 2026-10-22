#include "owo/config/config_store.h"
#include "owo/engine/style_profile.h"

#include <algorithm>
#include <cmath>
#include <iostream>
#include <string>
#include <vector>

// The settings file and the engine must agree on how many scenes exist and
// which bit each one uses.
static_assert(owo::config::kStyleSceneAll == owo::engine::kStyleSceneAll);
static_assert(static_cast<std::uint32_t>(owo::engine::StyleScene::chat) == 0);
static_assert(static_cast<std::uint32_t>(owo::engine::StyleScene::work) == 1);
static_assert(static_cast<std::uint32_t>(owo::engine::StyleScene::general) == 2);

namespace {

using owo::engine::StyleProfile;
using owo::engine::StyleScene;
using owo::engine::UserCorpusEntry;

owo::engine::StyleSceneProfile scene(const StyleProfile& profile, const StyleScene scene) {
    for (const auto& value : profile.scenes)
        if (value.scene == scene) return value;
    return {};
}

double close(const double left, const double right, const double epsilon = 1e-9) {
    return std::abs(left - right) <= epsilon;
}

}  // namespace

int main() {
    int failures = 0;
    // Applications map to scenes by executable name, ignoring case.
    {
        if (owo::engine::style_scene_for_application("WeChat.exe") != StyleScene::chat ||
            owo::engine::style_scene_for_application("weixin.exe") != StyleScene::chat ||
            owo::engine::style_scene_for_application("WINWORD.EXE") != StyleScene::work ||
            owo::engine::style_scene_for_application("notepad.exe") != StyleScene::general ||
            owo::engine::style_scene_for_application("") != StyleScene::general) {
            std::cerr << "scene mapping failed\n";
            ++failures;
        }
        if (owo::engine::style_scene_id(StyleScene::chat) != "chat" ||
            owo::engine::style_scene_name(StyleScene::work) != "工作" ||
            owo::engine::style_scene_bit(StyleScene::general) != 4 ||
            owo::engine::kStyleSceneAll != 7) {
            std::cerr << "scene identifiers failed\n";
            ++failures;
        }
    }
    // An empty corpus produces an empty profile rather than a fabricated one.
    {
        const auto profile = owo::engine::build_style_profile({});
        if (!profile.scenes.empty() || profile.total_sentences != 0) {
            std::cerr << "empty corpus produced a profile\n";
            ++failures;
        }
        if (owo::engine::describe_style_profile(profile).find("还没有可用") == std::string::npos) {
            std::cerr << "empty profile was not explained\n";
            ++failures;
        }
    }
    // A colloquial writer and a formal writer differ in the expected direction.
    {
        std::vector<UserCorpusEntry> entries{
            {1, 0, "WeChat.exe", "哈哈好呀"},
            {2, 0, "WeChat.exe", "行吧"},
            {3, 0, "WeChat.exe", "感觉挺不错啊"},
            {4, 0, "WeChat.exe", "咋回事？"},
            {5, 0, "WeChat.exe", "嗯嗯"},
            {6, 0, "WeChat.exe", "那就这样吧"},
            {7, 0, "WINWORD.EXE", "您好，烦请确认本周周报的评审安排。"},
            {8, 0, "WINWORD.EXE", "建议先同步方案，再推进后续总结计划。"},
            {9, 0, "WINWORD.EXE", "感谢各位参与本次会议纪要的评审。"},
            {10, 0, "WINWORD.EXE", "我会在周报里确认进度。"},
            {11, 0, "WINWORD.EXE", "请确认方案后同步给大家。"},
            {12, 0, "WINWORD.EXE", "本周计划推进评审总结。"}};
        const auto profile = owo::engine::build_style_profile(entries);
        if (profile.total_sentences != 12 || profile.scenes.size() != 2) {
            std::cerr << "scene split failed: " << profile.scenes.size() << "\n";
            ++failures;
        }
        const auto chat = scene(profile, StyleScene::chat);
        const auto work = scene(profile, StyleScene::work);
        if (chat.sentence_count != 6 || work.sentence_count != 6) {
            std::cerr << "scene counts failed: " << chat.sentence_count << "/"
                      << work.sentence_count << "\n";
            ++failures;
        }
        if (chat.formality < 0 || work.formality < 0 ||
            chat.formality + 5 >= work.formality) {
            std::cerr << "formality did not separate scenes: " << chat.formality << " vs "
                      << work.formality << "\n";
            ++failures;
        }
        // The score is reconstructible: base 50 plus the reported signals.
        for (const auto& value : profile.scenes) {
            int sum = 50;
            for (const auto& signal : value.formality_signals) sum += signal.delta;
            sum = (std::max)(0, (std::min)(100, sum));
            if (sum != value.formality) {
                std::cerr << "formality signals do not add up: " << sum << " vs "
                          << value.formality << "\n";
                ++failures;
            }
        }
        if (chat.markers.empty() || chat.markers.front().text != "吧" ||
            chat.markers.front().count != 2) {
            std::cerr << "recurring markers were not summarized\n";
            ++failures;
        }
        if (chat.short_sentence_ratio < 0.5) {
            std::cerr << "short sentences were not counted\n";
            ++failures;
        }
        if (work.exclamation_per_sentence != 0.0 ||
            !close(chat.terminal_punctuation_ratio, 1.0 / 6.0)) {
            std::cerr << "punctuation statistics failed\n";
            ++failures;
        }
        if (chat.applications.size() != 1 || chat.applications.front() != "WeChat.exe") {
            std::cerr << "application summary failed\n";
            ++failures;
        }
        const auto report = owo::engine::describe_style_profile(profile);
        if (report.find("聊天") == std::string::npos ||
            report.find("工作") == std::string::npos ||
            report.find("正式程度") == std::string::npos) {
            std::cerr << "report did not describe both scenes\n";
            ++failures;
        }
        // The mask lets the setting hide a scene.
        const auto chat_only = owo::engine::build_style_profile(
            entries, owo::engine::style_scene_bit(StyleScene::chat));
        if (chat_only.scenes.size() != 1 || chat_only.scenes.front().scene != StyleScene::chat) {
            std::cerr << "scene mask failed\n";
            ++failures;
        }
    }
    // A small sample still reports the measured numbers but refuses a score.
    {
        std::vector<UserCorpusEntry> entries{
            {1, 0, "WeChat.exe", "好呀"},
            {2, 0, "WeChat.exe", "明天见。"}};
        const auto profile = owo::engine::build_style_profile(entries);
        if (profile.scenes.size() != 1 || profile.scenes.front().sentence_count != 2 ||
            profile.scenes.front().formality != -1 ||
            profile.scenes.front().average_characters <= 0.0) {
            std::cerr << "small sample handling failed\n";
            ++failures;
        }
        const auto report = owo::engine::describe_style_profile(profile);
        if (report.find("样本不足") == std::string::npos) {
            std::cerr << "small sample was not explained\n";
            ++failures;
        }
    }
    // Markers seen once are not habits; they are dropped from the report.
    {
        std::vector<UserCorpusEntry> entries{
            {1, 0, "WeChat.exe", "这就很神奇了"},
            {2, 0, "WeChat.exe", "明天开会"},
            {3, 0, "WeChat.exe", "收到"},
            {4, 0, "WeChat.exe", "好的"},
            {5, 0, "WeChat.exe", "没问题"}};
        const auto profile = owo::engine::build_style_profile(entries);
        if (!profile.scenes.empty() && !profile.scenes.front().markers.empty()) {
            std::cerr << "single marker was promoted\n";
            ++failures;
        }
    }
    // Only the newest entries count, and invalid UTF-8 is skipped rather than
    // crashing the viewer.
    {
        std::vector<UserCorpusEntry> entries(
            owo::engine::kMaximumStyleProfileCorpusEntries + 4);
        for (std::size_t index = 0; index < entries.size(); ++index) {
            entries[index] = {index + 1, 0, "WeChat.exe",
                              index < 4 ? "旧的句子不该参与统计。" : "新的句子参与统计。"};
        }
        const auto profile = owo::engine::build_style_profile(entries);
        if (profile.scenes.size() != 1 ||
            profile.scenes.front().sentence_count !=
                owo::engine::kMaximumStyleProfileCorpusEntries) {
            std::cerr << "profile entry bound failed\n";
            ++failures;
        }
        std::vector<UserCorpusEntry> invalid{{1, 0, "WeChat.exe", "\xff\xfe"}};
        const auto skipped = owo::engine::build_style_profile(invalid);
        if (!skipped.scenes.empty()) {
            std::cerr << "invalid UTF-8 was counted\n";
            ++failures;
        }
    }
    if (failures == 0) std::cout << "style profile tests passed\n";
    return failures == 0 ? 0 : 1;
}

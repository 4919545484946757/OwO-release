#include "owo/config/config_store.h"
#include <algorithm>
#include <iostream>

namespace {
std::string legacy(owo::config::AppConfig value) {
    value.expanded_previous_page_shortcut_enabled = false;
    value.expanded_next_page_shortcut_enabled = false;
    auto text = owo::config::serialize_config(value);
    text.replace(0, std::string("schema_version=21").size(), "schema_version=16");
    for (const auto key : {"expanded_previous_page_shortcut_enabled=", "expanded_previous_page_shortcut=",
                            "expanded_next_page_shortcut_enabled=", "expanded_next_page_shortcut="}) {
        const auto begin = text.find(key);
        if (begin != std::string::npos) text.erase(begin, text.find('\n', begin) - begin + 1);
    }
    for (const auto key : {"smart_candidates_enabled=","expressive_candidates_enabled=","concept_candidates_enabled=","corpus_learning_enabled=","corpus_learning_paused=","corpus_learning_allowed_apps=","corpus_learning_excluded_apps=","corpus_retention_days=","corpus_prediction_enabled=","style_scene_mask=","style_preferences=","tone_reply_enabled=","tone_continuation_enabled=","tone_cloud_model_enabled="}) {
        const auto begin = text.find(key);
        if (begin != std::string::npos) text.erase(begin, text.find('\n', begin) - begin + 1);
    }
    return text;
}
}
int main() {
    using namespace owo::config;
    AppConfig value;
    if (value.previous_page_shortcuts != std::vector<std::string>{"["} ||
        value.expanded_previous_page_shortcuts != std::vector<std::string>{"Up"} ||
        value.expanded_next_page_shortcuts != std::vector<std::string>{"Down"}) return 1;
    value.previous_page_shortcuts = {"[", "Up"};
    value.next_page_shortcuts = {"]", "Down"};
    auto parsed = parse_config(legacy(value));
    if (!parsed.ok || parsed.value.previous_page_shortcuts != std::vector<std::string>{"["} ||
        parsed.value.next_page_shortcuts != std::vector<std::string>{"]"} ||
        !parsed.value.expanded_previous_page_shortcut_enabled || !parsed.value.expanded_next_page_shortcut_enabled) return 2;
    value.previous_page_shortcuts = {"Up"};
    value.previous_page_shortcut_enabled = false;
    parsed = parse_config(legacy(value));
    if (!parsed.ok || parsed.value.previous_page_shortcut_enabled ||
        parsed.value.expanded_previous_page_shortcut_enabled || !parsed.value.previous_page_shortcuts.empty()) return 3;
    value = AppConfig{};
    value.raw_input_shortcuts = {"Up"};
    parsed = parse_config(legacy(value));
    if (!parsed.ok || parsed.value.raw_input_shortcuts != std::vector<std::string>{"Up"} ||
        parsed.value.expanded_previous_page_shortcut_enabled || !parsed.value.expanded_next_page_shortcut_enabled) return 4;
    value = AppConfig{};
    value.expanded_previous_page_shortcuts = {"Ctrl+F7", "Alt+Up"};
    value.expanded_next_page_shortcuts = {"Ctrl+F8"};
    parsed = parse_config(serialize_config(value));
    if (!parsed.ok || parsed.value != value) return 5;
    value.expanded_next_page_shortcuts = value.expanded_previous_page_shortcuts;
    if (validate_config(value).ok) return 6;
    value.expanded_next_page_shortcut_enabled = false;
    if (!validate_config(value).ok) return 7;
    value.expanded_previous_page_shortcuts.clear();
    if (validate_config(value).ok) return 8;
    std::cout << "Expanded page shortcut defaults, legacy migration, persistence and conflict checks passed.\n";
}

#pragma once

#include "owo/config/config_store.h"
#include <algorithm>
#include <array>
#include <map>
#include <optional>

namespace owo::config {

inline constexpr std::string_view kSchemeShortcutHeader = "OWO_SCHEME_SHORTCUTS_V1";
struct SchemeShortcutAction {
    std::string_view id;
    bool AppConfig::*enabled;
    std::vector<std::string> AppConfig::*keys;
};
inline constexpr std::array kSchemeShortcutActions{
    SchemeShortcutAction{"raw", &AppConfig::raw_input_shortcut_enabled, &AppConfig::raw_input_shortcuts},
    SchemeShortcutAction{"cursor_left", &AppConfig::cursor_left_shortcut_enabled, &AppConfig::cursor_left_shortcuts},
    SchemeShortcutAction{"cursor_right", &AppConfig::cursor_right_shortcut_enabled, &AppConfig::cursor_right_shortcuts},
    SchemeShortcutAction{"previous_page", &AppConfig::previous_page_shortcut_enabled, &AppConfig::previous_page_shortcuts},
    SchemeShortcutAction{"next_page", &AppConfig::next_page_shortcut_enabled, &AppConfig::next_page_shortcuts},
    SchemeShortcutAction{"expanded_previous_page", &AppConfig::expanded_previous_page_shortcut_enabled, &AppConfig::expanded_previous_page_shortcuts},
    SchemeShortcutAction{"expanded_next_page", &AppConfig::expanded_next_page_shortcut_enabled, &AppConfig::expanded_next_page_shortcuts}
};
using SchemeShortcuts = std::map<std::string, std::vector<std::string>, std::less<>>;

inline std::optional<SchemeShortcuts> parse_scheme_shortcuts(std::string_view text) {
    if (text.size() > 4096) return std::nullopt;
    const auto line = [&] {
        auto next = text.substr(0, text.find('\n'));
        text.remove_prefix(next.size() + (next.size() < text.size()));
        if (next.ends_with('\r')) next.remove_suffix(1);
        return next;
    };
    if (line() != kSchemeShortcutHeader) return std::nullopt;
    SchemeShortcuts result;
    while (!text.empty()) {
        const auto row = line();
        const auto separator = row.find('=');
        if (separator == row.npos) return std::nullopt;
        const auto action = row.substr(0, separator);
        if (std::none_of(kSchemeShortcutActions.begin(), kSchemeShortcutActions.end(),
            [&](const auto& item) { return item.id == action; })) return std::nullopt;
        auto [item, unique] = result.try_emplace(std::string(action));
        if (!unique) return std::nullopt;
        auto keys = row.substr(separator + 1);
        if (keys.ends_with(';')) return std::nullopt;
        while (!keys.empty()) {
            const auto key = keys.substr(0, keys.find(';'));
            if (key.empty() || key.size() > 64 || item->second.size() >= 16) return std::nullopt;
            item->second.emplace_back(key);
            keys.remove_prefix(key.size() + (key.size() < keys.size()));
        }
    }
    return result;
}

inline bool apply_scheme_shortcuts(AppConfig& config, const SchemeShortcuts& defaults,
                                  const SchemeShortcuts& overrides) {
    auto scoped = config;
    scoped.correction_shortcut_enabled = scoped.separator_shortcut_enabled = false;
    scoped.punctuation_shortcut_enabled = scoped.text_navigation_shortcut_enabled = false;
    for (const auto& [action, keys] : overrides)
        if (!defaults.contains(action)) return false;
    for (const auto& action : kSchemeShortcutActions) {
        const auto custom = overrides.find(action.id), packaged = defaults.find(action.id);
        auto& keys = scoped.*action.keys;
        keys = custom != overrides.end() ? custom->second : packaged != defaults.end() ? packaged->second : std::vector<std::string>{};
        for (const auto& key : keys)
            if (key == "Ctrl+C" || key == "Ctrl+V" || key == "Ctrl+X" || key == "Ctrl+Z" ||
                key == "Ctrl+Y" || key == "Ctrl+A" || key == "Ctrl+S" || key == "Ctrl+F") return false;
        scoped.*action.enabled = !keys.empty();
    }
    auto order = std::move(scoped.input_scheme_order); // live scheme need not be the default
    if (!validate_config(scoped).ok) return false; // includes global scheme-switch collision
    scoped.input_scheme_order = std::move(order);
    config = std::move(scoped);
    return true;
}

}  // namespace owo::config

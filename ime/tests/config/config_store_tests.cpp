#include "owo/config/config_store.h"

#include <filesystem>
#include <fstream>
#include <string>

namespace {

void write(const std::filesystem::path& path, const std::string_view text) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(text.data(), static_cast<std::streamsize>(text.size()));
}

}  // namespace

int main(const int argc, char** argv) {
    if (argc != 2) return 1;
    const std::filesystem::path root(argv[1]);
    // Per-key preferences survive persistence, while older schemas preserve all conversions.
    auto punctuation = owo::config::AppConfig{};
    for (const char source : owo::config::kChinesePunctuationSources)
        if (!owo::config::punctuation_selected(punctuation.chinese_punctuation_mask, source)) return 50;
    if (owo::config::punctuation_selected(punctuation.chinese_punctuation_mask, '/')) return 51;
    punctuation.chinese_punctuation_mask = 1;
    const auto punctuation_roundtrip = owo::config::parse_config(owo::config::serialize_config(punctuation));
    if (!punctuation_roundtrip.ok || punctuation_roundtrip.value != punctuation ||
        !owo::config::punctuation_selected(1, ',') || owo::config::punctuation_selected(1, '.')) return 52;
    punctuation.chinese_punctuation_mask = 0;
    if (!owo::config::parse_config(owo::config::serialize_config(punctuation)).ok) return 53;
    punctuation.chinese_punctuation_mask = 0x80000000;
    if (owo::config::validate_config(punctuation).ok) return 54;
    auto version_fourteen = owo::config::serialize_config(owo::config::AppConfig{});
    const auto erase_field = [](std::string& text, const std::string_view key) {
        const auto begin = text.find(key);
        if (begin == std::string::npos) return false;
        text.erase(begin, text.find('\n', begin) - begin + 1);
        return true;
    };
    version_fourteen.replace(0, std::string("schema_version=16").size(), "schema_version=14");
    const auto mask_begin = version_fourteen.find("chinese_punctuation_mask=");
    version_fourteen.erase(mask_begin, version_fourteen.find('\n', mask_begin) - mask_begin + 1);
    // v14 predates the assistive candidate switches.
    for (const auto key : {"smart_candidates_enabled=",
                           "expressive_candidates_enabled=",
                           "concept_candidates_enabled="})
        if (!erase_field(version_fourteen, key)) return 56;
    const auto migrated = owo::config::parse_config(version_fourteen);
    if (!migrated.ok || migrated.value.chinese_punctuation_mask != owo::config::kChinesePunctuationAll) return 55;
    if (!migrated.value.smart_candidates_enabled ||
        !migrated.value.expressive_candidates_enabled ||
        !migrated.value.concept_candidates_enabled) return 56;
    // v15 predates the assistive switches as well; they must default to on.
    auto version_fifteen = version_fourteen;
    version_fifteen.replace(version_fifteen.find("schema_version=14"),
                            std::string("schema_version=14").size(),
                            "schema_version=15");
    version_fifteen += "chinese_punctuation_mask=" +
                       std::to_string(owo::config::kChinesePunctuationAll) + "\n";
    const auto version_fifteen_result = owo::config::parse_config(version_fifteen);
    if (!version_fifteen_result.ok ||
        !version_fifteen_result.value.smart_candidates_enabled ||
        !version_fifteen_result.value.expressive_candidates_enabled ||
        !version_fifteen_result.value.concept_candidates_enabled)
        return 57;
    auto disabled = owo::config::AppConfig{};
    disabled.smart_candidates_enabled = false;
    disabled.expressive_candidates_enabled = false;
    disabled.concept_candidates_enabled = false;
    const auto disabled_roundtrip =
        owo::config::parse_config(owo::config::serialize_config(disabled));
    if (!disabled_roundtrip.ok || disabled_roundtrip.value != disabled)
        return 58;
    auto invalid_assistive = owo::config::serialize_config(owo::config::AppConfig{});
    invalid_assistive.replace(
        invalid_assistive.find("smart_candidates_enabled=true"),
        std::string("smart_candidates_enabled=true").size(),
        "smart_candidates_enabled=maybe");
    if (owo::config::parse_config(invalid_assistive).ok) return 59;
    auto missing_assistive = owo::config::serialize_config(owo::config::AppConfig{});
    if (!erase_field(missing_assistive, "concept_candidates_enabled=")) return 60;
    if (owo::config::parse_config(missing_assistive).ok) return 61;
    std::error_code error;
    std::filesystem::remove_all(root, error);
    std::filesystem::create_directories(root, error);
    if (error) return 2;
    const auto path = root / "owo.conf";

    owo::config::ConfigStore store;
    const auto defaults = store.load(path);
    if (!defaults.success || !defaults.used_defaults || defaults.generation != 1 ||
        store.snapshot() != owo::config::AppConfig{}) return 3;

    auto changed = store.snapshot();
    changed.input_scheme = "exact_code";
    changed.candidate_page_size = 7;
    changed.candidate_wrap_length = 18;
    changed.candidate_partition_enabled = false;
    changed.candidate_immediate_commit_enabled = false;
    changed.smart_candidates_enabled = false;
    changed.expressive_candidates_enabled = false;
    changed.concept_candidates_enabled = false;
    changed.focus_loss_commit_enabled = true;
    changed.user_learning_enabled = false;
    changed.user_learning_sensitivity = 9;
    changed.user_lexicon_completion_enabled = false;
    changed.user_lexicon_completion_max_syllables = 4;
    changed.model_ranking_enabled = true;
    changed.model_timeout_ms = 80;
    changed.correction_shortcuts = {"Ctrl+Alt+C", "F8"};
    changed.language_shortcuts = {"Ctrl+Shift+Space"};
    changed.raw_input_shortcut_enabled = false;
    changed.cursor_left_shortcuts = {"Ctrl+Shift+Left"};
    changed.cursor_right_shortcuts = {"Ctrl+Shift+Right"};
    changed.previous_page_shortcuts = {"[", "Ctrl+Shift+Up"};
    changed.next_page_shortcuts = {"]", "Ctrl+Shift+Down"};
    changed.chinese_punctuation_enabled = false;
    changed.chinese_punctuation_mask = 7;
    changed.alt_code_preview_enabled = false;
    changed.alt_code_preview_page_size = 6;
    changed.alt_code_preview_wheel_enabled = false;
    changed.punctuation_shortcut_enabled = true;
    changed.punctuation_shortcuts = {"Ctrl+Period"};
    const auto first_save = store.save(changed);
    if (!first_save.success || !first_save.changed || first_save.generation != 2) return 4;
    changed.candidate_page_size = 8;
    if (!store.save(changed).success) return 5;
    if (std::filesystem::exists(path.wstring() + L".tmp") ||
        std::filesystem::exists(path.wstring() + L".bak.tmp")) return 6;

    owo::config::ConfigStore loaded;
    if (!loaded.load(path).success || loaded.snapshot() != changed) return 7;
    auto hot = changed;
    hot.model_timeout_ms = 90;
    write(path, owo::config::serialize_config(hot));
    const auto hot_reload = loaded.reload();
    if (!hot_reload.success || !hot_reload.changed || hot_reload.generation != 2 ||
        loaded.snapshot() != hot) return 8;
    const auto unchanged_reload = loaded.reload();
    if (!unchanged_reload.success || unchanged_reload.changed || unchanged_reload.generation != 2)
        return 9;
    write(path, "broken");
    const auto invalid_reload = loaded.reload();
    if (invalid_reload.success || invalid_reload.changed || loaded.snapshot() != hot ||
        loaded.generation() != 2) return 10;

    owo::config::ConfigStore recovered;
    const auto recovery = recovered.load(path);
    auto backup_value = changed;
    backup_value.candidate_page_size = 7;
    if (!recovery.success || !recovery.recovered_from_backup ||
        recovered.snapshot() != backup_value) return 11;

    write(path.wstring() + L".bak", "also broken");
    owo::config::ConfigStore fallback;
    const auto fallback_result = fallback.load(path);
    if (!fallback_result.success || !fallback_result.used_defaults ||
        fallback.snapshot() != owo::config::AppConfig{}) return 12;

    auto invalid_value = fallback.snapshot();
    invalid_value.candidate_page_size = 0;
    if (fallback.save(invalid_value).success || fallback.snapshot() != owo::config::AppConfig{} ||
        fallback.generation() != 1) return 13;
    invalid_value = fallback.snapshot();
    invalid_value.candidate_wrap_length = 3;
    if (fallback.save(invalid_value).success) return 23;
    invalid_value = fallback.snapshot();
    invalid_value.user_learning_sensitivity = 11;
    if (fallback.save(invalid_value).success) return 29;
    invalid_value = fallback.snapshot();
    invalid_value.user_lexicon_completion_max_syllables = 9;
    if (fallback.save(invalid_value).success) return 37;
    invalid_value = fallback.snapshot();
    invalid_value.alt_code_preview_page_size = 1;
    if (fallback.save(invalid_value).success) return 33;
    invalid_value = fallback.snapshot();
    invalid_value.input_scheme = "unknown";
    if (fallback.save(invalid_value).success) return 36;

    if (owo::config::parse_config("schema_version=2\ncandidate_page_size=5\n"
            "user_learning_enabled=true\nmodel_ranking_enabled=false\nmodel_timeout_ms=50\n").ok) return 14;
    if (owo::config::parse_config("schema_version=1\ncandidate_page_size=10\n"
            "user_learning_enabled=true\nmodel_ranking_enabled=false\nmodel_timeout_ms=50\n").ok) return 15;
    if (owo::config::parse_config("schema_version=1\ncandidate_page_size=5\n"
            "user_learning_enabled=yes\nmodel_ranking_enabled=false\nmodel_timeout_ms=50\n").ok) return 16;
    if (owo::config::parse_config("schema_version=1\ncandidate_page_size=5\n"
            "user_learning_enabled=true\nmodel_ranking_enabled=false\nmodel_timeout_ms=50\nunknown=x\n").ok) return 17;
    if (owo::config::parse_config("\xef\xbb\xbfschema_version=1\n").ok) return 18;

    const auto legacy = owo::config::parse_config(
        "schema_version=1\ncandidate_page_size=6\nuser_learning_enabled=false\n"
        "model_ranking_enabled=true\nmodel_timeout_ms=75\n");    if (!legacy.ok || legacy.value.candidate_page_size != 6 ||
        legacy.value.correction_shortcuts != std::vector<std::string>{"Ctrl+Q"} ||
        legacy.value.language_shortcuts != std::vector<std::string>{"Ctrl+Space"} ||
        legacy.value.raw_input_shortcuts != std::vector<std::string>{"Enter"} ||
        legacy.value.cursor_left_shortcuts != std::vector<std::string>{"Left"} ||
        legacy.value.next_page_shortcuts != std::vector<std::string>{"]", "Down"} ||
        legacy.value.candidate_wrap_length != 12 ||
        !legacy.value.candidate_partition_enabled ||
        !legacy.value.candidate_immediate_commit_enabled ||
        legacy.value.focus_loss_commit_enabled ||
        legacy.value.user_learning_sensitivity != 10 ||
        !legacy.value.chinese_punctuation_enabled ||
        !legacy.value.alt_code_preview_enabled ||
        legacy.value.alt_code_preview_page_size != 4 ||
        !legacy.value.alt_code_preview_wheel_enabled ||
        legacy.value.punctuation_shortcut_enabled ||
        !legacy.value.punctuation_shortcuts.empty()) return 20;

    auto version_thirteen_text = version_fourteen;
    version_thirteen_text.replace(version_thirteen_text.find("schema_version=14"),
                                  std::string("schema_version=14").size(),
                                  "schema_version=13");
    for (const auto key : {"user_lexicon_completion_enabled=",
                           "user_lexicon_completion_max_syllables="}) {
        const auto begin = version_thirteen_text.find(key);
        if (begin == std::string::npos) return 38;
        version_thirteen_text.erase(begin,
            version_thirteen_text.find('\n', begin) - begin + 1);
    }
    const auto version_thirteen = owo::config::parse_config(version_thirteen_text);
    if (!version_thirteen.ok ||
        !version_thirteen.value.user_lexicon_completion_enabled ||
        version_thirteen.value.user_lexicon_completion_max_syllables != 0)
        return 39;
    auto version_ten_text = version_thirteen_text;
    version_ten_text.replace(version_ten_text.find("schema_version=13"),
                             std::string("schema_version=13").size(),
                             "schema_version=10");
    const auto input_scheme_begin = version_ten_text.find("input_scheme=");
    if (input_scheme_begin == std::string::npos) return 36;
    version_ten_text.erase(input_scheme_begin,
        version_ten_text.find('\n', input_scheme_begin) - input_scheme_begin + 1);
    const auto text_navigation_begin = version_ten_text.find(
        "text_navigation_shortcut_enabled=");
    if (text_navigation_begin == std::string::npos) return 35;
    version_ten_text.erase(text_navigation_begin);
    version_ten_text.replace(version_ten_text.find("cursor_left_shortcut=Left"),
                             std::string("cursor_left_shortcut=Left").size(),
                             "cursor_left_shortcut=Shift+Left");
    version_ten_text.replace(version_ten_text.find("cursor_right_shortcut=Right"),
                             std::string("cursor_right_shortcut=Right").size(),
                             "cursor_right_shortcut=Shift+Right");
    version_ten_text.replace(version_ten_text.find("previous_page_shortcut=[;Up"),
                             std::string("previous_page_shortcut=[;Up").size(),
                             "previous_page_shortcut=[;Shift+Up");
    version_ten_text.replace(version_ten_text.find("next_page_shortcut=];Down"),
                             std::string("next_page_shortcut=];Down").size(),
                             "next_page_shortcut=];Shift+Down");
    const auto version_ten = owo::config::parse_config(version_ten_text);
    if (!version_ten.ok ||
        version_ten.value.cursor_left_shortcuts != std::vector<std::string>{"Left"} ||
        version_ten.value.cursor_right_shortcuts != std::vector<std::string>{"Right"} ||
        version_ten.value.previous_page_shortcuts !=
            std::vector<std::string>{"[", "Up"} ||
        version_ten.value.next_page_shortcuts !=
            std::vector<std::string>{"]", "Down"}) return 34;

    const auto version_two = owo::config::parse_config(
        "schema_version=2\ncandidate_page_size=5\nuser_learning_enabled=true\n"
        "model_ranking_enabled=false\nmodel_timeout_ms=50\n"
        "correction_shortcut_enabled=true\ncorrection_shortcut=Alt\n"
        "language_shortcut_enabled=true\nlanguage_shortcut=Ctrl+Space\n"
        "raw_input_shortcut_enabled=true\nraw_input_shortcut=Enter\n");
    if (!version_two.ok || version_two.value.candidate_wrap_length != 12 ||
        !version_two.value.candidate_partition_enabled ||
        !version_two.value.candidate_immediate_commit_enabled ||
        version_two.value.focus_loss_commit_enabled ||
        version_two.value.user_learning_sensitivity != 10 ||
        !version_two.value.user_lexicon_completion_enabled ||
        version_two.value.user_lexicon_completion_max_syllables != 0) return 24;

    auto duplicate_shortcuts = changed;
    duplicate_shortcuts.raw_input_shortcut_enabled = true;
    duplicate_shortcuts.raw_input_shortcuts = duplicate_shortcuts.language_shortcuts;
    if (owo::config::validate_config(duplicate_shortcuts).ok) return 21;
    auto noncanonical_shortcut = changed;
    noncanonical_shortcut.correction_shortcuts = {"Alt+Ctrl+C"};
    if (owo::config::validate_config(noncanonical_shortcut).ok) return 22;
    noncanonical_shortcut = changed;
    noncanonical_shortcut.correction_shortcuts = {"Ctrl"};
    if (owo::config::validate_config(noncanonical_shortcut).ok) return 25;
    noncanonical_shortcut.correction_shortcuts = {"Shift"};
    if (owo::config::validate_config(noncanonical_shortcut).ok) return 26;
    noncanonical_shortcut.correction_shortcuts = {"Ctrl+Alt"};
    if (!owo::config::validate_config(noncanonical_shortcut).ok) return 27;
    noncanonical_shortcut.correction_shortcuts = {"Alt"};
    if (!owo::config::validate_config(noncanonical_shortcut).ok) return 28;

    auto duplicate_navigation = changed;
    duplicate_navigation.cursor_right_shortcuts =
        duplicate_navigation.cursor_left_shortcuts;
    if (owo::config::validate_config(duplicate_navigation).ok) return 30;

    auto missing_punctuation_binding = changed;
    missing_punctuation_binding.punctuation_shortcuts.clear();
    if (owo::config::validate_config(missing_punctuation_binding).ok) return 31;
    missing_punctuation_binding.punctuation_shortcut_enabled = false;
    if (!owo::config::validate_config(missing_punctuation_binding).ok) return 32;

    const auto stable = owo::config::serialize_config(changed);
    const auto round_trip = owo::config::parse_config(stable);
    if (!round_trip.ok || round_trip.value != changed ||
        stable != owo::config::serialize_config(round_trip.value)) return 19;

    std::filesystem::remove_all(root, error);
    auto plugin_scheme = changed;
    plugin_scheme.input_scheme = "plugin:owo.input-method-example/table";
    if (!owo::config::validate_config(plugin_scheme).ok) return 70;
    const auto scheme_roundtrip = owo::config::parse_config(owo::config::serialize_config(plugin_scheme));
    if (!scheme_roundtrip.ok || scheme_roundtrip.value != plugin_scheme) return 71;
    for (const auto bad : {"plugin:", "plugin:../table", "plugin:owo.test/../table", "plugin:owo.test/", "plugin:owo.test/a/b"}) {
        plugin_scheme.input_scheme = bad;
        if (owo::config::validate_config(plugin_scheme).ok) return 72;
    }
    return 0;
}

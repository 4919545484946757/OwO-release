#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace owo::config {

inline constexpr std::uint32_t kConfigSchemaVersion = 20;

// Style scenes (learning layer 3). The order and bits mirror
// engine::StyleScene in owo/engine/style_profile.h; the settings and the
// profile viewer must agree on them, so an engine test keeps the two equal.
inline constexpr std::uint32_t kStyleSceneAll = (std::uint32_t{1} << 3) - 1;
inline constexpr std::size_t kMaximumStylePreferencesBytes = 512;

// Append-only bit order: each physical ASCII punctuation has its own preference.
inline constexpr std::string_view kChinesePunctuationSources = ",.<>?;:[]{}\\|-_+=`~'\"()!$^";
inline constexpr std::uint32_t kChinesePunctuationAll =
    (std::uint32_t{1} << kChinesePunctuationSources.size()) - 1;
[[nodiscard]] inline bool punctuation_selected(std::uint32_t mask, char source) noexcept {
    const auto index = kChinesePunctuationSources.find(source);
    return index != std::string_view::npos && (mask & (std::uint32_t{1} << index)) != 0;
}

// Persisted reference only, never a grant or filesystem path.
[[nodiscard]] bool is_plugin_input_scheme(std::string_view) noexcept;

inline constexpr std::size_t kMaximumCorpusApplications = 64;
/// Executable file name such as `WeChat.exe`; compared case-insensitively.
[[nodiscard]] bool valid_corpus_application(std::string_view) noexcept;

struct AppConfig {
    std::string input_scheme{"full_pinyin"};
    std::uint32_t candidate_page_size{7};
    std::uint32_t candidate_wrap_length{12};
    bool candidate_partition_enabled{true};
    bool candidate_immediate_commit_enabled{true};
    // Assistive candidate families. The modeless unified input itself (mixed
    // preedit, tone digits, literal fallback) is not gated by these switches.
    bool smart_candidates_enabled{true};
    bool expressive_candidates_enabled{true};
    bool concept_candidates_enabled{true};
    bool focus_loss_commit_enabled{false};
    bool user_learning_enabled{true};
    std::uint32_t user_learning_sensitivity{10};
    bool user_lexicon_completion_enabled{true};
    std::uint32_t user_lexicon_completion_max_syllables{0};
    bool chinese_punctuation_enabled{true};
    std::uint32_t chinese_punctuation_mask{kChinesePunctuationAll};
    bool alt_code_preview_enabled{true};
    std::uint32_t alt_code_preview_page_size{4};
    bool alt_code_preview_wheel_enabled{true};
    bool punctuation_shortcut_enabled{false};
    std::vector<std::string> punctuation_shortcuts{};
    bool model_ranking_enabled{false};
    std::uint32_t model_timeout_ms{50};
    bool correction_shortcut_enabled{true};
    std::vector<std::string> correction_shortcuts{"Ctrl+Q"};
    bool language_shortcut_enabled{true};
    std::vector<std::string> language_shortcuts{"Ctrl+Space"};
    bool raw_input_shortcut_enabled{true};
    std::vector<std::string> raw_input_shortcuts{"Enter"};
    bool cursor_left_shortcut_enabled{true};
    std::vector<std::string> cursor_left_shortcuts{"Left"};
    bool cursor_right_shortcut_enabled{true};
    std::vector<std::string> cursor_right_shortcuts{"Right"};
    bool previous_page_shortcut_enabled{true};
    std::vector<std::string> previous_page_shortcuts{"[", "Up"};
    bool next_page_shortcut_enabled{true};
    std::vector<std::string> next_page_shortcuts{"]", "Down"};
    bool text_navigation_shortcut_enabled{true};
    std::vector<std::string> text_navigation_shortcuts{
        "Shift+Left", "Shift+Right", "Shift+Up", "Shift+Down"};
    bool text_navigation_select_enabled{false};
    // Authorized corpus learning (schema 17). Off until the user opts in; an
    // empty allow list means every application except the excluded ones.
    bool corpus_learning_enabled{false};
    bool corpus_learning_paused{false};
    std::vector<std::string> corpus_learning_allowed_apps{};
    std::vector<std::string> corpus_learning_excluded_apps{
        "KeePass.exe", "KeePassXC.exe", "1Password.exe", "Bitwarden.exe"};
    // 0 keeps sentences until the user deletes them.
    std::uint32_t corpus_retention_days{180};
    // Local next-phrase suggestions derived from the corpus (schema 18). It
    // needs an enabled corpus but is otherwise independent of recording, so a
    // user can keep a corpus for tone drafting and refuse suggestions.
    bool corpus_prediction_enabled{true};
    // Style profile (schema 19). The mask selects which inferred scenes the
    // profile viewer reports and, later, which examples tone drafting may use.
    std::uint32_t style_scene_mask{kStyleSceneAll};
    // The user's own words about how they want to write ("简短、少用感叹号").
    // They are stored as-is and are never learned or overwritten automatically.
    std::string style_preferences;
    // Reply drafting (schema 20). Both switches default off: drafting reads the
    // corpus and may send the current message to a model, so it is an explicit
    // opt-in like corpus learning itself.
    bool tone_reply_enabled{false};
    // The continuation switch covers "write an opening and let the model finish
    // it"; it is independent of reply drafting.
    bool tone_continuation_enabled{false};
    // Without this, drafting can still assemble the local request but the caller
    // must not send it to a cloud model. Local-only operation stays unchanged.
    bool tone_cloud_model_enabled{false};
    bool operator==(const AppConfig&) const = default;
};

struct ConfigValidationResult {
    bool ok{};
    std::string diagnostic;
};

struct ConfigParseResult {
    bool ok{};
    AppConfig value;
    std::string diagnostic;
};

struct ConfigIoResult {
    bool success{};
    bool recovered_from_backup{};
    bool used_defaults{};
    bool changed{};
    std::uint64_t generation{};
    std::string diagnostic;
};

[[nodiscard]] ConfigValidationResult validate_config(const AppConfig&);
/// Whether a committed fragment from `executable` may enter the user corpus.
[[nodiscard]] bool corpus_learning_allows(const AppConfig&, std::string_view executable) noexcept;
[[nodiscard]] ConfigParseResult parse_config(std::string_view utf8);
[[nodiscard]] std::string serialize_config(const AppConfig&);

class ConfigStore final {
public:
    /// 首次加载允许从备份恢复；主文件和备份都不可用时返回默认配置及诊断。
    [[nodiscard]] ConfigIoResult load(const std::filesystem::path& path);
    /// 热加载只接受有效主文件；失败时保留当前不可变快照和 generation。
    [[nodiscard]] ConfigIoResult reload();
    /// 严格校验后原子保存；只用有效旧主文件更新备份。
    [[nodiscard]] ConfigIoResult save(const AppConfig& value);
    [[nodiscard]] const AppConfig& snapshot() const noexcept { return current_; }
    [[nodiscard]] std::uint64_t generation() const noexcept { return generation_; }

private:
    std::filesystem::path path_;
    AppConfig current_;
    std::uint64_t generation_{};
};

}  // namespace owo::config

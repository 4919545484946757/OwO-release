#include "owo/engine/symbol_composition.h"
#include "owo/config/config_store.h"
#include <iostream>

int main() {
    using namespace owo::engine;
    int failures = 0;
    auto check = [&](bool value, const char* name) {
        if (!value) { std::cerr << name << '\n'; ++failures; }
    };
    const auto composition = split_symbol_composition("ni'hao，shi'jie。", true);
    check(composition.valid && composition.segments.size() == 2, "strong segments");
    const auto& first = composition.segments.front();
    check(first.query == "ni'hao" && first.symbols.empty(), "default manual apostrophe");
    const std::vector<std::string> nihao{"ni", "hao"};
    auto projected = project_symbol_candidate(first, "你好", nihao, first.query.size());
    check(projected.text == "你好，" && projected.consumed == std::string("ni'hao，").size(), "FIFO consume delimiter");
    const auto partial = project_symbol_candidate(first, "你", std::vector<std::string>{"ni"}, 2);
    check(partial.text == "你" && partial.consumed == 2, "partial preserves remaining segment");
    check(symbol_segment_preview(first, nihao) == "ni'hao，", "source preview");
    const auto quoted = split_symbol_composition("wo“nihao”", true);
    const auto& quote = quoted.segments.front();
    check(quote.query == "wo'nihao" && quote.weak_spans.size() == 1, "weak quote boundaries");
    const std::vector<std::string> full{"wo", "ni", "hao"};
    check(symbol_segment_preview(quote, full) == "wo“ni'hao”", "quotes retained in preview");
    projected = project_symbol_candidate(quote, "我你好", full, quote.query.size());
    check(projected.text == "我“你好”" && projected.consumed == quoted.source.size(), "quotes in committed candidate");
    projected = project_symbol_candidate(quote, "我", std::vector<std::string>{"wo"}, 2);
    check(projected.text == "我" && projected.consumed == 2, "opening quote belongs to next word");
    const auto ascii = split_symbol_composition("wo'nihao'", false);
    check(ascii.valid && ascii.segments.front().weak_spans.size() == 1, "unbound apostrophe is weak");
    check(project_symbol_candidate(ascii.segments.front(), "我你好", full,
        ascii.segments.front().query.size()).text == "我'你好'", "literal ASCII quotes preserved");
    const std::vector<std::uint64_t> manual{2};
    const auto remapped = split_symbol_composition("ni'hao", false, manual);
    check(remapped.valid && remapped.segments.front().symbols.empty(), "custom shortcut inserts syntax");
    check(project_symbol_candidate(remapped.segments.front(), "你好", nihao, 6).text == "你好", "syntax never committed");
    const auto literal = split_symbol_composition("ni🙂+hao", true);
    check(project_symbol_candidate(literal.segments.front(), "你好", nihao,
        literal.segments.front().query.size()).text == "你🙂+好", "other symbols retained");
    check(!split_symbol_composition(std::string("\xc0\xaf", 2), true).valid, "reject overlong UTF8");
    check(!split_symbol_composition(std::string("\xed\xa0\x80", 3), true).valid, "reject surrogate UTF8");
    check(!split_symbol_composition("nihao", false, manual).valid, "reject invalid break offset");
    const auto leading = split_symbol_composition("，；nihao", true);
    check(leading.segments.size() == 2 && leading.segments[0].query.empty() &&
        leading.segments[0].delimiter == "，；", "leading/repeated strong symbols");
    Candidate base{"我你好", full, 100};
    base.source_segments = full; base.consumed_input_bytes = quote.query.size();
    std::vector<Candidate> choices{base};
    refine_weak_symbol_candidates(quote, choices, [&](std::string_view query) {
        check(query == "nihao", "local quote query only");
        Candidate local{"拟好", nihao, 200};
        local.source_segments = nihao; local.consumed_input_bytes = 5;
        return std::vector<Candidate>{local};
    }, 2);
    check(choices.size() == 2 && choices[1].text == "我拟好", "replace quoted span only");
    auto config = owo::config::AppConfig{};
    check(config.pinyin_symbol_compatibility_enabled && config.separator_shortcuts == std::vector<std::string>{"Quote"}, "symbol compatibility enabled by default");
    config.pinyin_symbol_compatibility_enabled = false;
    config.separator_shortcuts = {"Ctrl+F7", "Alt+F8"};
    const auto roundtrip = owo::config::parse_config(owo::config::serialize_config(config));
    check(roundtrip.ok && roundtrip.value == config, "multi binding config roundtrip");
    config.separator_shortcuts = {"Ctrl+Space"};
    check(!owo::config::validate_config(config).ok, "shortcut collision rejected");
    config.separator_shortcut_enabled = false;
    config.separator_shortcuts.clear();
    check(owo::config::validate_config(config).ok, "disabled empty bindings allowed");
    config.raw_input_shortcuts = {"Quote"};
    auto legacy = owo::config::serialize_config(config);
    legacy.replace(0, std::string("schema_version=18").size(), "schema_version=17");
    for (const auto key : {"pinyin_symbol_compatibility_enabled=", "separator_shortcut_enabled=", "separator_shortcut="}) {
        const auto at = legacy.find(key);
        legacy.erase(at, legacy.find('\n', at) - at + 1);
    }
    const auto migrated = owo::config::parse_config(legacy);
    check(migrated.ok && !migrated.value.separator_shortcut_enabled &&
        migrated.value.raw_input_shortcuts == std::vector<std::string>{"Quote"}, "migration preserves occupied apostrophe");
    return failures == 0 ? 0 : 1;
}

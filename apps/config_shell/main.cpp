#include "owo/config/config_monitor.h"
#include "owo/config/config_store.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/managed_dictionary.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"
#include <Windows.h>

#include <charconv>
#include <chrono>
#include <cstdint>
#include <iostream>
#include <algorithm>
#include <map>
#include <set>
#include <string>
#include <string_view>
#include <vector>

namespace {

bool parse_u32(const std::wstring_view text, std::uint32_t& output) {
    std::string ascii;
    ascii.reserve(text.size());
    for (const auto character : text) {
        if (character > 0x7f) return false;
        ascii.push_back(static_cast<char>(character));
    }
    const auto result = std::from_chars(ascii.data(), ascii.data() + ascii.size(), output);
    return result.ec == std::errc{} && result.ptr == ascii.data() + ascii.size();
}

bool parse_ascii(const std::wstring_view text, std::string& output) {
    output.clear();
    output.reserve(text.size());
    for (const auto character : text) {
        if (character > 0x7f) return false;
        output.push_back(static_cast<char>(character));
    }
    return !output.empty();
}

bool parse_shortcuts(const std::wstring_view text, std::vector<std::string>& output) {
    if (text.empty()) { output.clear(); return true; }
    std::string ascii;
    if (!parse_ascii(text, ascii)) return false;
    output.clear();
    std::size_t offset = 0;
    while (offset < ascii.size()) {
        const auto separator = ascii.find(';', offset);
        const auto item = ascii.substr(offset, separator == std::string::npos
            ? ascii.size() - offset : separator - offset);
        if (item.empty()) return false;
        output.push_back(item);
        if (separator == std::string::npos) break;
        offset = separator + 1;
    }
    return !output.empty();
}

bool apply(owo::config::AppConfig& config, const std::wstring_view field,
           const std::wstring_view value) {
    if (field == L"input_scheme")
        return parse_ascii(value, config.input_scheme);
    if (field == L"chinese_punctuation_mask")
        return parse_u32(value, config.chinese_punctuation_mask);
    if (field == L"candidate_page_size")
        return parse_u32(value, config.candidate_page_size);
    if (field == L"candidate_wrap_length")
        return parse_u32(value, config.candidate_wrap_length);
    if (field == L"alt_code_preview_page_size")
        return parse_u32(value, config.alt_code_preview_page_size);
    if (field == L"user_learning_sensitivity")
        return parse_u32(value, config.user_learning_sensitivity);
    if (field == L"user_lexicon_completion_max_syllables")
        return parse_u32(value, config.user_lexicon_completion_max_syllables);
    if (field == L"model_timeout_ms") return parse_u32(value, config.model_timeout_ms);
    if (field == L"correction_shortcut")
        return parse_shortcuts(value, config.correction_shortcuts);
    if (field == L"language_shortcut")
        return parse_shortcuts(value, config.language_shortcuts);
    if (field == L"raw_input_shortcut")
        return parse_shortcuts(value, config.raw_input_shortcuts);
    if (field == L"cursor_left_shortcut")
        return parse_shortcuts(value, config.cursor_left_shortcuts);
    if (field == L"cursor_right_shortcut")
        return parse_shortcuts(value, config.cursor_right_shortcuts);
    if (field == L"previous_page_shortcut")
        return parse_shortcuts(value, config.previous_page_shortcuts);
    if (field == L"next_page_shortcut")
        return parse_shortcuts(value, config.next_page_shortcuts);
    if (field == L"expanded_previous_page_shortcut")
        return parse_shortcuts(value, config.expanded_previous_page_shortcuts);
    if (field == L"expanded_next_page_shortcut")
        return parse_shortcuts(value, config.expanded_next_page_shortcuts);
    if (field == L"text_navigation_shortcut")
        return parse_shortcuts(value, config.text_navigation_shortcuts);
    if (field == L"punctuation_shortcut")
        return parse_shortcuts(value, config.punctuation_shortcuts);
    if (field == L"separator_shortcut")
        return parse_shortcuts(value, config.separator_shortcuts);
    if (field == L"pinyin_preview_location") {
        if (value == L"candidate") config.pinyin_preview_location = "candidate";
        else if (value == L"input") config.pinyin_preview_location = "input";
        else return false;
        return true;
    }
    bool parsed{};
    if (value == L"true") parsed = true;
    else if (value != L"false") return false;
    if (field == L"user_learning_enabled") config.user_learning_enabled = parsed;
    else if (field == L"pinyin_symbol_compatibility_enabled") config.pinyin_symbol_compatibility_enabled = parsed;
    else if (field == L"separator_shortcut_enabled") config.separator_shortcut_enabled = parsed;
    else if (field == L"user_lexicon_completion_enabled")
        config.user_lexicon_completion_enabled = parsed;
    else if (field == L"chinese_punctuation_enabled")
        config.chinese_punctuation_enabled = parsed;
    else if (field == L"punctuation_shortcut_enabled")
        config.punctuation_shortcut_enabled = parsed;
    else if (field == L"candidate_partition_enabled")
        config.candidate_partition_enabled = parsed;
    else if (field == L"candidate_single_character_first_enabled")
        config.candidate_single_character_first_enabled = parsed;
    else if (field == L"candidate_immediate_commit_enabled")
        config.candidate_immediate_commit_enabled = parsed;
    else if (field == L"focus_loss_commit_enabled")
        config.focus_loss_commit_enabled = parsed;
    else if (field == L"alt_code_preview_enabled")
        config.alt_code_preview_enabled = parsed;
    else if (field == L"alt_code_preview_wheel_enabled")
        config.alt_code_preview_wheel_enabled = parsed;
    else if (field == L"model_ranking_enabled") config.model_ranking_enabled = parsed;
    else if (field == L"correction_shortcut_enabled")
        config.correction_shortcut_enabled = parsed;
    else if (field == L"language_shortcut_enabled")
        config.language_shortcut_enabled = parsed;
    else if (field == L"raw_input_shortcut_enabled")
        config.raw_input_shortcut_enabled = parsed;
    else if (field == L"cursor_left_shortcut_enabled")
        config.cursor_left_shortcut_enabled = parsed;
    else if (field == L"cursor_right_shortcut_enabled")
        config.cursor_right_shortcut_enabled = parsed;
    else if (field == L"previous_page_shortcut_enabled")
        config.previous_page_shortcut_enabled = parsed;
    else if (field == L"next_page_shortcut_enabled")
        config.next_page_shortcut_enabled = parsed;
    else if (field == L"expanded_previous_page_shortcut_enabled")
        config.expanded_previous_page_shortcut_enabled = parsed;
    else if (field == L"expanded_next_page_shortcut_enabled")
        config.expanded_next_page_shortcut_enabled = parsed;
    else if (field == L"text_navigation_shortcut_enabled")
        config.text_navigation_shortcut_enabled = parsed;
    else if (field == L"text_navigation_select_enabled")
        config.text_navigation_select_enabled = parsed;
    else return false;
    return true;
}

void usage() {
    std::cerr << "usage: owo_config_shell <path> show | defaults | repair | set <field> <value> | "
                 "set-all <page-size> <learning> <ranking> <timeout-ms> "
                 "[<correction-enabled> <correction-key> <language-enabled> <language-key> "
                 "<raw-enabled> <raw-key> [<candidate-wrap-length> "
                 "[<learning-sensitivity> [<cursor-left-enabled> <cursor-left-key> "
                 "<cursor-right-enabled> <cursor-right-key> <previous-page-enabled> "
                 "<previous-page-key> <next-page-enabled> <next-page-key> "
                 "[<candidate-partition-enabled> <chinese-punctuation-enabled> "
                 "<punctuation-shortcut-enabled> <punctuation-shortcut> "
                 "[<alt-code-preview-enabled> <alt-code-preview-page-size> "
                 "<alt-code-preview-wheel-enabled> "
                 "[<candidate-immediate-commit-enabled> "
                 "<focus-loss-commit-enabled> [<input-scheme> "
                 "[<user-completion-enabled> <user-completion-max-syllables> "
                 "[<chinese-punctuation-mask> [<single-character-first-enabled> "
                 "[<expanded-previous-enabled> <expanded-previous-keys> "
                 "<expanded-next-enabled> <expanded-next-keys> "
                 "[<pinyin-symbol-compatibility-enabled> <separator-shortcut-enabled> <separator-shortcuts>]]]]]]]]]]]]] | "
                 "watch <timeout-ms>\n";
}

}  // namespace

int wmain(const int argc, wchar_t** argv) {
    if (argc == 3 && std::wstring_view(argv[1]) == L"--dictionary-compile") {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
        try {
            const auto result = owo::engine::prepare_managed_dictionary_snapshot(std::filesystem::path(argv[2]));
            if (!result.success) { std::cerr << result.error << '\n'; return 3; }
            std::cout << "OWO_DICTIONARY_RUNTIME_V1\n";
            return 0;
        } catch (const std::exception&) { std::cerr << "dictionary compilation failed\n"; return 3; }
    }
    if (argc == 3 && std::wstring_view(argv[1]) == L"--dictionary-readings") {
        const std::filesystem::path source(argv[2]);
        const bool installed_only = std::wstring_view(argv[2]) == L"--installed-only";
        auto extension = source.extension().wstring();
        for (auto& value : extension) if (value >= L'A' && value <= L'Z') value += L'a' - L'A';
        if (!installed_only && (!source.is_absolute() || extension != L".owolx")) {
            std::cerr << "请选择本机 .owolx 参考词典。\n"; return 2;
        }
        std::string payload;
        for (char value{}; std::cin.get(value);) {
            if (payload.size() >= 2 * 1024 * 1024) { std::cerr << "注音请求超过 2 MiB，请减小批量。\n"; return 2; }
            payload += value;
        }
        constexpr std::string_view header = "OWO_DICTIONARY_READINGS_V1\n";
        if (!payload.starts_with(header)) { std::cerr << "注音请求格式无效。\n"; return 2; }
        std::vector<std::string_view> words;
        struct Reference { std::string_view text, code; std::uint32_t frequency; };
        std::vector<Reference> references;
        for (std::size_t offset = header.size(); offset < payload.size();) {
            auto end = payload.find('\n', offset);
            if (end == std::string::npos) end = payload.size();
            const auto line = std::string_view(payload).substr(offset, end - offset);
            const auto a = line.find('\t'), b = a == line.npos ? a : line.find('\t', a + 1);
            const auto text = line.substr(0, a);
            if (!owo::engine::UserFrequencyStore::validate_word({std::string(text), 1, "a", false}).empty()) {
                std::cerr << "注音请求词语无效或超过 32768 项，请减小批量。\n"; return 2;
            }
            if (a == line.npos) {
                if (words.size() >= 32768) return 2;
                words.push_back(text);
            } else {
                std::uint32_t frequency{};
                if (b == line.npos || line.find('\t', b + 1) != line.npos || references.size() >= 32768) return 2;
                const auto value = line.substr(b + 1);
                const auto parsed = std::from_chars(value.data(), value.data() + value.size(), frequency);
                const auto code = line.substr(a + 1, b - a - 1);
                if (parsed.ec != std::errc{} || parsed.ptr != value.data() + value.size() ||
                    !owo::engine::UserFrequencyStore::validate_word({std::string(text), frequency, std::string(code), false}).empty()) return 2;
                references.push_back({text, code, frequency});
            }
            offset = end + 1;
        }
        owo::engine::BinaryLexicon lexicon;
        if (!installed_only) {
            const auto loaded = lexicon.load(source);
            if (!loaded.success) { std::cerr << "参考词典加载失败：" << loaded.error << '\n'; return 3; }
        }
        struct Readings { std::map<std::string, std::uint32_t> codes; bool uncertain{}; };
        std::map<std::string_view, Readings> collected;
        for (auto text : words) collected.try_emplace(text);
        const auto add = [&](std::string_view text, std::string_view code, std::uint32_t frequency, bool use_evidence = false) {
            auto found = collected.find(text);
            if (found == collected.end()) return;
            const auto characters = std::count_if(text.begin(), text.end(), [](unsigned char value) { return (value & 0xc0) != 0x80; });
            const auto readings = owo::engine::FullPinyinSchema::dictionary_readings(code, static_cast<std::size_t>(characters));
            if (use_evidence && code.find('\'') == code.npos && readings.size() > 1) {
                for (const auto& [known, weight] : found->second.codes) {
                    std::string compact;
                    for (auto value : known) if (value != '\'') compact += value;
                    if (compact == code) return; // precise word evidence already supplies the boundary
                }
            }
            if (readings.size() >= 4) found->second.uncertain = true;
            for (const auto& reading : readings) {
                std::string normalized;
                for (const auto& syllable : reading) { if (!normalized.empty()) normalized += '\''; normalized += syllable; }
                auto& values = found->second;
                if (auto existing = values.codes.find(normalized); existing != values.codes.end()) existing->second = (std::max)(existing->second, frequency);
                else if (values.codes.size() < 8) values.codes.emplace(std::move(normalized), frequency);
                else values.uncertain = true;
            }
        };
        std::map<std::string_view, std::size_t> base_counts;
        for (const auto& word : lexicon.lookup_readings(words)) {
            std::string code;
            for (const auto& syllable : word.syllables) { if (!code.empty()) code += '\''; code += syllable; }
            // Use request-owned keys: the temporary materialized row is gone
            // after this iteration. A saturated batch is never "unique" proof.
            const auto found = collected.find(word.text);
            if (found == collected.end() || word.frequency == 0) continue;
            if (++base_counts[found->first] >= 8) found->second.uncertain = true;
            add(found->first, code, word.frequency);
        }
        std::stable_sort(references.begin(), references.end(), [](const auto& a, const auto& b) {
            return (a.code.find('\'') != a.code.npos) > (b.code.find('\'') != b.code.npos);
        });
        for (const auto& reference : references) add(reference.text, reference.code, reference.frequency, true);
        std::cout << header;
        for (const auto& [text, values] : collected)
            for (const auto& [code, frequency] : values.codes)
                std::cout << text << '\t' << code << '\t' << frequency << '\t' << (values.uncertain ? 1 : 0) << '\n';
        return 0;
    }
    if (argc == 2 && std::wstring_view(argv[1]) == L"--user-lexicon") {
        std::string payload;
        char c{};
        while (std::cin.get(c)) {
            if (payload.size() >= 512 * 1024) { std::cerr << "词库请求过大。\n"; return 2; }
            payload += c;
        }
        owo::protocol::Message request;
        request.type = owo::protocol::MessageType::user_lexicon_request;
        request.request_id = GetTickCount64();
        request.context_generation = (std::uint64_t{GetCurrentProcessId()} << 32) | 0x4c455831;
        request.text = std::move(payload);
        const auto encoded = owo::protocol::encode_message(request);
        if (encoded.empty() || encoded.size() > owo::protocol::kMaximumPayloadBytes) {
            std::cerr << "词库请求编码后过大，请拆分导入。\n"; return 2;
        }
        const auto reply = owo::ipc::exchange(owo::ipc::kCorePipeName,
            encoded, std::chrono::seconds(10));
        if (!reply.status) { std::cerr << "无法连接词库服务，请先启动最新版 OwO Core。\n"; return 4; }
        const auto decoded = owo::protocol::decode_message(reply.response);
        if (!decoded.validation || decoded.message.request_id != request.request_id ||
            decoded.message.context_generation != request.context_generation) {
            std::cerr << "词库响应无效或版本不匹配。\n"; return 4;
        }
        if (decoded.message.type != owo::protocol::MessageType::user_lexicon_response) {
            std::cerr << decoded.message.text << '\n'; return 5;
        }
        std::cout << decoded.message.text;
        return 0;
    }
    if (argc < 3) {
        usage();
        return 2;
    }
    const std::filesystem::path path(argv[1]);
    const std::wstring_view command(argv[2]);
    if (command == L"defaults" && argc == 3) {
        std::cout << owo::config::serialize_config(owo::config::AppConfig{});
        return 0;
    }
    if (command == L"show" && argc == 3) {
        owo::config::ConfigStore store;
        const auto loaded = store.load(path);
        if (!loaded.success) {
            std::cerr << loaded.diagnostic << '\n';
            return 3;
        }
        std::cout << owo::config::serialize_config(store.snapshot());
        if (!loaded.diagnostic.empty()) std::cerr << loaded.diagnostic << '\n';
        return 0;
    }
    if (command == L"repair" && argc == 3) {
        owo::config::ConfigStore store;
        const auto loaded = store.load(path);
        if (!loaded.success) return 3;
        const auto saved = store.save(store.snapshot());
        if (!saved.success) {
            std::cerr << saved.diagnostic << '\n';
            return 5;
        }
        std::cout << (loaded.recovered_from_backup ? "repaired_from_backup" :
                      loaded.used_defaults ? "repaired_with_defaults" : "already_valid") << '\n';
        return 0;
    }
    if (command == L"set" && argc == 5) {
        owo::config::ConfigStore store;
        const auto loaded = store.load(path);
        if (!loaded.success) return 3;
        auto value = store.snapshot();
        if (!apply(value, argv[3], argv[4])) {
            std::cerr << "unknown field or invalid value type\n";
            return 4;
        }
        const auto saved = store.save(value);
        if (!saved.success) {
            std::cerr << saved.diagnostic << '\n';
            return 5;
        }
        std::cout << "saved generation=" << saved.generation << '\n';
        return 0;
    }
    if (command == L"set-all" &&
        (argc == 7 || argc == 13 || argc == 14 || argc == 15 || argc == 23 ||
         argc == 24 || argc == 27 || argc == 30 || argc == 32 || argc == 35 ||
         argc == 36 || argc == 38 || argc == 39 || argc == 40 || argc == 44 || argc == 47 || argc == 48)) {
        owo::config::ConfigStore store;
        const auto loaded = store.load(path);
        if (!loaded.success) return 3;
        auto value = store.snapshot();
        if (!apply(value, L"candidate_page_size", argv[3]) ||
            !apply(value, L"user_learning_enabled", argv[4]) ||
            !apply(value, L"model_ranking_enabled", argv[5]) ||
            !apply(value, L"model_timeout_ms", argv[6]) ||
            (argc >= 13 &&
             (!apply(value, L"correction_shortcut_enabled", argv[7]) ||
              !apply(value, L"correction_shortcut", argv[8]) ||
              !apply(value, L"language_shortcut_enabled", argv[9]) ||
              !apply(value, L"language_shortcut", argv[10]) ||
              !apply(value, L"raw_input_shortcut_enabled", argv[11]) ||
              !apply(value, L"raw_input_shortcut", argv[12]))) ||
            (argc >= 14 && !apply(value, L"candidate_wrap_length", argv[13])) ||
            (argc >= 15 && !apply(value, L"user_learning_sensitivity", argv[14])) ||
            (argc >= 23 &&
             (!apply(value, L"cursor_left_shortcut_enabled", argv[15]) ||
              !apply(value, L"cursor_left_shortcut", argv[16]) ||
              !apply(value, L"cursor_right_shortcut_enabled", argv[17]) ||
              !apply(value, L"cursor_right_shortcut", argv[18]) ||
              !apply(value, L"previous_page_shortcut_enabled", argv[19]) ||
              !apply(value, L"previous_page_shortcut", argv[20]) ||
              !apply(value, L"next_page_shortcut_enabled", argv[21]) ||
              !apply(value, L"next_page_shortcut", argv[22]))) ||
            (argc >= 24 &&
             !apply(value, L"candidate_partition_enabled", argv[23])) ||
            (argc >= 27 &&
             (!apply(value, L"chinese_punctuation_enabled", argv[24]) ||
              !apply(value, L"punctuation_shortcut_enabled", argv[25]) ||
              !apply(value, L"punctuation_shortcut", argv[26]))) ||
            (argc >= 30 &&
             (!apply(value, L"alt_code_preview_enabled", argv[27]) ||
              !apply(value, L"alt_code_preview_page_size", argv[28]) ||
              !apply(value, L"alt_code_preview_wheel_enabled", argv[29]))) ||
            (argc >= 32 &&
             (!apply(value, L"candidate_immediate_commit_enabled", argv[30]) ||
              !apply(value, L"focus_loss_commit_enabled", argv[31]))) ||
            (argc >= 35 &&
             (!apply(value, L"text_navigation_shortcut_enabled", argv[32]) ||
              !apply(value, L"text_navigation_shortcut", argv[33]) ||
              !apply(value, L"text_navigation_select_enabled", argv[34]))) ||
            (argc >= 36 && !apply(value, L"input_scheme", argv[35])) ||
            (argc >= 38 &&
             (!apply(value, L"user_lexicon_completion_enabled", argv[36]) ||
              !apply(value, L"user_lexicon_completion_max_syllables", argv[37]))) ||
            (argc >= 39 && !apply(value, L"chinese_punctuation_mask", argv[38])) ||
            (argc >= 40 && !apply(value, L"candidate_single_character_first_enabled", argv[39])) ||
            (argc >= 44 &&
             (!apply(value, L"expanded_previous_page_shortcut_enabled", argv[40]) ||
              !apply(value, L"expanded_previous_page_shortcut", argv[41]) ||
              !apply(value, L"expanded_next_page_shortcut_enabled", argv[42]) ||
              !apply(value, L"expanded_next_page_shortcut", argv[43]))) ||
            (argc >= 47 && (!apply(value, L"pinyin_symbol_compatibility_enabled", argv[44]) ||
             !apply(value, L"separator_shortcut_enabled", argv[45]) ||
             !apply(value, L"separator_shortcut", argv[46]))) ||
            (argc >= 48 && !apply(value, L"pinyin_preview_location", argv[47]))) {
            std::cerr << "invalid configuration value type\n";
            return 4;
        }
        const auto saved = store.save(value);
        if (!saved.success) {
            std::cerr << saved.diagnostic << '\n';
            return 5;
        }
        std::cout << "saved generation=" << saved.generation << '\n';
        return 0;
    }
    if (command == L"watch" && argc == 4) {
        std::uint32_t timeout_ms{};
        if (!parse_u32(argv[3], timeout_ms) || timeout_ms < 10 || timeout_ms > 60'000) return 4;
        owo::config::ConfigMonitor monitor;
        const auto started = monitor.start(path, std::chrono::milliseconds(20));
        if (!started.success) return 3;
        const auto initial_generation = monitor.generation();
        std::cout << "ready generation=" << initial_generation << '\n' << std::flush;
        if (!monitor.wait_for_generation(initial_generation, std::chrono::milliseconds(timeout_ms))) {
            std::cerr << "watch timeout\n";
            return 6;
        }
        std::cout << "changed generation=" << monitor.generation() << '\n'
                  << owo::config::serialize_config(*monitor.snapshot());
        return 0;
    }
    usage();
    return 2;
}

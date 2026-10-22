#include "owo/config/config_paths.h"
#include "owo/config/config_store.h"
#include "owo/engine/style_profile.h"
#include "owo/engine/tone_draft.h"
#include "owo/engine/tone_feedback.h"
#include "owo/engine/user_corpus.h"

#include <Windows.h>

#include <charconv>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace {

std::string utf8(const std::wstring_view value) {
    if (value.empty()) return {};
    const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
                                         static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    if (size <= 0) return {};
    std::string result(static_cast<std::size_t>(size), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()),
                            result.data(), size, nullptr, nullptr) != size) return {};
    return result;
}

bool parse_u64(const std::wstring_view text, std::uint64_t& output) {
    const auto ascii = utf8(text);
    const auto result = std::from_chars(ascii.data(), ascii.data() + ascii.size(), output);
    return !ascii.empty() && result.ec == std::errc{} && result.ptr == ascii.data() + ascii.size();
}

// One entry per line: id, created_at_ms, application, text; tab, newline and
// backslash are escaped so every field stays on its line.
std::string escape(const std::string_view value) {
    std::string result;
    result.reserve(value.size());
    for (const char c : value) {
        if (c == '\\') result += "\\\\";
        else if (c == '\t') result += "\\t";
        else if (c == '\n') result += "\\n";
        else if (c == '\r') result += "\\r";
        else result.push_back(c);
    }
    return result;
}

std::uint64_t now_ms() {
    return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count());
}

void usage() {
    std::cerr << "usage: owo_corpus_shell [--corpus <path>] [--config <path>] "
                 "[--app <executable>] [--feedback <path>] "
                 "list | stats | style | delete <id> | clear-app <executable> | clear-all | "
                 "tone [--continue] <message> | "
                 "tone-check [--continue] <message> <drafts-file> | "
                 "feedback list | feedback metadata | feedback add <adopted|edited|rejected|undone> "
                 "<draft> [<final>] | feedback delete <id> | feedback clear | "
                 "feedback profile | feedback summary\n";
}

// The scene mask and the tone switches live in the configuration, so the
// viewer and drafting honour the same switches the settings center edits.
// Without a readable configuration the defaults apply, which keep drafting off.
std::optional<owo::config::AppConfig> load_config(const std::filesystem::path& config_path) {
    const auto path = config_path.empty() ? owo::config::default_config_path() : config_path;
    if (path.empty()) return std::nullopt;
    owo::config::ConfigStore store;
    if (!store.load(path).success) return std::nullopt;
    return store.snapshot();
}

int report(const owo::engine::UserCorpusIoResult& result, const std::size_t removed) {
    if (!result.success) {
        std::cerr << result.error << '\n';
        return 5;
    }
    std::cout << "removed=" << removed << '\n';
    return 0;
}

}  // namespace

int wmain(int argc, wchar_t** argv) {
    SetConsoleOutputCP(CP_UTF8);
    int index = 1;
    std::filesystem::path path;
    std::filesystem::path config_path;
    std::filesystem::path feedback_path;
    std::string application;
    while (index + 1 < argc) {
        const std::wstring_view option(argv[index]);
        if (option == L"--corpus") path = argv[index + 1];
        else if (option == L"--config") config_path = argv[index + 1];
        else if (option == L"--app") application = utf8(argv[index + 1]);
        else if (option == L"--feedback") feedback_path = argv[index + 1];
        else break;
        index += 2;
    }
    const auto root = owo::config::local_data_root();
    if (path.empty()) {
        if (root.empty()) return 3;
        path = root / L"data" / L"user-corpus.owuc";
    }
    if (feedback_path.empty() && !root.empty())
        feedback_path = root / L"data" / L"tone-feedback.owtf";
    if (index >= argc) {
        usage();
        return 2;
    }
    const std::wstring_view command(argv[index]);
    const int remaining = argc - index - 1;
    if ((command == L"list" || command == L"stats" || command == L"style") &&
        remaining == 0) {
        owo::engine::UserCorpusStore store;
        const auto loaded = store.load(path);
        if (!loaded.success) {
            std::cerr << loaded.error << '\n';
            return 3;
        }
        if (command == L"stats") {
            std::cout << "entries=" << store.entries().size() << '\n';
            return 0;
        }
        if (command == L"style") {
            const auto config = load_config(config_path);
            std::cout << owo::engine::describe_style_profile(
                owo::engine::build_style_profile(
                    store.entries(),
                    config ? config->style_scene_mask : owo::config::kStyleSceneAll));
            return 0;
        }
        for (const auto& entry : store.entries())
            std::cout << entry.id << '\t' << entry.created_at_ms << '\t' << escape(entry.application)
                      << '\t' << escape(entry.text) << '\n';
        return 0;
    }
    // Tone drafting (L4). `tone` prints the exact local request a model would
    // receive; `tone-check` runs the deterministic post-filter over a file of
    // model output. Neither command sends anything anywhere.
    if (command == L"tone" || command == L"tone-check") {
        int argument = index + 1;
        auto mode = owo::engine::ToneDraftMode::reply;
        if (argument < argc && std::wstring_view(argv[argument]) == L"--continue") {
            mode = owo::engine::ToneDraftMode::continuation;
            ++argument;
        }
        const int needed = command == L"tone" ? 1 : 2;
        if (argc - argument != needed) {
            usage();
            return 2;
        }
        const auto config = load_config(config_path);
        // Drafting is consent-gated: a missing or unreadable configuration has
        // never recorded an opt-in, so it is treated as switched off. This is
        // deliberately stricter than `style`, which only reads aggregates.
        const bool enabled = config &&
            (mode == owo::engine::ToneDraftMode::continuation
                 ? config->tone_continuation_enabled
                 : config->tone_reply_enabled);
        if (!enabled) {
            std::cerr << (mode == owo::engine::ToneDraftMode::continuation
                              ? "整句续写在设置中没有打开\n"
                              : "语气回复在设置中没有打开\n");
            return 7;
        }
        if (!config->corpus_learning_enabled) {
            std::cerr << "授权语料学习在设置中没有打开\n";
            return 7;
        }
        owo::engine::UserCorpusStore store;
        const auto loaded = store.load(path);
        if (!loaded.success) {
            std::cerr << loaded.error << '\n';
            return 3;
        }
        const auto message = utf8(argv[argument]);
        // Feedback hints are local records of earlier drafts; they join the
        // request exactly like the hand-written preferences do.
        std::vector<std::string> hints;
        if (!feedback_path.empty()) {
            owo::engine::ToneFeedbackStore feedback;
            if (feedback.load(feedback_path).success) {
                const auto scene = owo::engine::style_scene_for_application(application);
                hints = owo::engine::describe_tone_feedback_hints(
                    owo::engine::build_tone_feedback_profile(
                        feedback.entries(), scene,
                        config ? config->style_scene_mask : owo::config::kStyleSceneAll));
            }
        }
        // `--app` selects the scene exactly as the IME would; without it the
        // scene is the general one, matching an application that is not listed.
        const auto plan = owo::engine::build_tone_draft_plan(
            store.entries(), message, application,
            config ? config->style_scene_mask : owo::config::kStyleSceneAll,
            config ? config->style_preferences : std::string{}, mode,
            owo::engine::kDefaultToneExamples, hints);
        std::cout << "scene=" << owo::engine::style_scene_id(plan.scene) << '\n'
                  << "mode="
                  << (mode == owo::engine::ToneDraftMode::continuation ? "continuation"
                                                                      : "reply")
                  << '\n'
                  << "ready=" << (plan.ready ? "true" : "false") << '\n'
                  << "examples=" << plan.examples.size() << '\n'
                  << "cloud_model="
                  << (config && config->tone_cloud_model_enabled ? "allowed"
                                                                 : "local_only")
                  << '\n';
        if (!plan.ready) {
            std::cerr << plan.diagnostic << '\n';
            return 6;
        }
        std::cout << "style=" << plan.style_digest << '\n';
        for (const auto& hint : hints) std::cout << "hint=" << escape(hint) << '\n';
        for (const auto& example : plan.examples)
            std::cout << "example=" << example.shared_units << '\t' << escape(example.text)
                      << '\n';
        if (command == L"tone") {
            std::cout << "--- prompt ---\n" << plan.prompt;
            return 0;
        }
        std::ifstream input(std::filesystem::path(argv[argument + 1]),
                            std::ios::binary);
        if (!input) {
            std::cerr << "cannot read drafts file\n";
            return 4;
        }
        const std::string drafts_text((std::istreambuf_iterator<char>(input)),
                                      std::istreambuf_iterator<char>());
        std::vector<std::string> examples;
        for (const auto& example : plan.examples) examples.push_back(example.text);
        const std::vector<std::string> drafts{drafts_text};
        auto selection = owo::engine::filter_tone_drafts(drafts, message, examples, mode);
        // Feedback only refines the order; the model's ranking survives ties.
        if (!feedback_path.empty() && !selection.candidates.empty()) {
            owo::engine::ToneFeedbackStore feedback;
            if (feedback.load(feedback_path).success) {
                const auto scene = owo::engine::style_scene_for_application(application);
                selection = owo::engine::apply_tone_feedback(
                    std::move(selection),
                    owo::engine::build_tone_feedback_profile(
                        feedback.entries(), scene,
                        config ? config->style_scene_mask : owo::config::kStyleSceneAll));
            }
        }
        std::cout << "accepted=" << selection.candidates.size() << '\n';
        for (std::size_t candidate = 0; candidate < selection.candidates.size();
             ++candidate)
            std::cout << "candidate=" << candidate + 1 << '\t'
                      << escape(selection.candidates[candidate].text) << '\n';
        std::cout << "rejected=" << selection.rejected.size() << '\n';
        for (const auto& reason : selection.rejected) std::cout << "reason=" << reason << '\n';
        return 0;
    }
    // Feedback records (L5). They never touch the corpus: recording what the
    // user did with a draft is separate from what they typed.
    if (command == L"feedback") {
        if (feedback_path.empty()) {
            std::cerr << "feedback path is unavailable\n";
            return 3;
        }
        const std::wstring_view action = remaining >= 1 ? std::wstring_view(argv[index + 1])
                                                        : std::wstring_view{};
        if ((action == L"list" || action == L"metadata") && remaining == 1) {
            owo::engine::ToneFeedbackStore store;
            const auto loaded = store.load(feedback_path);
            if (!loaded.success) {
                std::cerr << loaded.error << '\n';
                return 3;
            }
            std::cout << "entries=" << store.entries().size() << '\n';
            for (const auto& entry : store.entries()) {
                std::cout << entry.id << '\t' << entry.created_at_ms << '\t'
                          << owo::engine::tone_feedback_kind_id(entry.kind) << '\t'
                          << escape(entry.application);
                if (action == L"list")
                    std::cout << '\t' << escape(entry.draft) << '\t' << escape(entry.final_text);
                std::cout << '\n';
            }
            return 0;
        }
        if (action == L"add" && (remaining == 3 || remaining == 4)) {
            const auto config = load_config(config_path);
            if (!config || !config->corpus_learning_enabled || config->corpus_learning_paused ||
                (!config->tone_reply_enabled && !config->tone_continuation_enabled) ||
                (config->style_scene_mask & owo::engine::style_scene_bit(
                    owo::engine::style_scene_for_application(application))) == 0) {
                std::cerr << "反馈学习未授权、已暂停或该场景已关闭\n";
                return 7;
            }
            owo::engine::ToneFeedbackKind kind{};
            if (!owo::engine::parse_tone_feedback_kind(utf8(argv[index + 2]), kind)) {
                std::cerr << "unknown feedback kind\n";
                return 4;
            }
            const auto draft = utf8(argv[index + 3]);
            const auto final_text = remaining == 4 ? utf8(argv[index + 4]) : std::string{};
            // Adoption needs the committed text; an edit needs the result.
            if (kind == owo::engine::ToneFeedbackKind::adopted && final_text.empty())
                return 4;
            std::uint64_t id{};
            const auto saved = owo::engine::ToneFeedbackStore::modify(
                feedback_path, [&](auto& store) {
                    id = store.append(kind, application, draft, final_text, now_ms());
                });
            if (!saved.success) {
                std::cerr << saved.error << '\n';
                return 5;
            }
            if (id == 0) {
                std::cerr << "feedback record was rejected\n";
                return 4;
            }
            std::cout << "id=" << id << '\n';
            return 0;
        }
        if (action == L"delete" && remaining == 2) {
            std::uint64_t id{};
            if (!parse_u64(argv[index + 2], id) || id == 0) return 4;
            std::size_t removed = 0;
            const auto saved = owo::engine::ToneFeedbackStore::modify(
                feedback_path, [&](auto& store) { removed = store.erase(id) ? 1 : 0; });
            if (!saved.success) {
                std::cerr << saved.error << '\n';
                return 5;
            }
            std::cout << "removed=" << removed << '\n';
            return 0;
        }
        if (action == L"clear" && remaining == 1) {
            std::size_t removed = 0;
            const auto saved = owo::engine::ToneFeedbackStore::modify(
                feedback_path, [&](auto& store) { removed = store.clear_all(); }, true);
            if (!saved.success) {
                std::cerr << saved.error << '\n';
                return 5;
            }
            std::cout << "removed=" << removed << '\n';
            return 0;
        }
        if (action == L"profile" && remaining == 1) {
            owo::engine::ToneFeedbackStore store;
            const auto loaded = store.load(feedback_path);
            if (!loaded.success) {
                std::cerr << loaded.error << '\n';
                return 3;
            }
            const auto config = load_config(config_path);
            const auto scene = owo::engine::style_scene_for_application(application);
            const auto profile = owo::engine::build_tone_feedback_profile(
                store.entries(), scene,
                config ? config->style_scene_mask : owo::config::kStyleSceneAll);
            std::cout << "scene=" << owo::engine::style_scene_id(scene) << '\n'
                      << "records=" << profile.entry_count << '\n'
                      << "preferences=" << profile.preferences.size() << '\n';
            for (const auto& preference : profile.preferences)
                std::cout << "preference=" << preference.weight << '\t'
                          << preference.observations << '\t' << escape(preference.from)
                          << '\t' << escape(preference.to) << '\n';
            for (const auto& hint : owo::engine::describe_tone_feedback_hints(profile))
                std::cout << "hint=" << escape(hint) << '\n';
            return 0;
        }
        // Counts across every scene, so a settings surface can show what the
        // feedback file currently holds without listing the draft texts.
        if (action == L"summary" && remaining == 1) {
            owo::engine::ToneFeedbackStore store;
            const auto loaded = store.load(feedback_path);
            if (!loaded.success) {
                std::cerr << loaded.error << '\n';
                return 3;
            }
            const auto config = load_config(config_path);
            const auto mask = config ? config->style_scene_mask : owo::config::kStyleSceneAll;
            std::cout << "records=" << store.entries().size() << '\n'
                      << "adopted=" << store.count(owo::engine::ToneFeedbackKind::adopted)
                      << '\n'
                      << "edited=" << store.count(owo::engine::ToneFeedbackKind::edited) << '\n'
                      << "rejected=" << store.count(owo::engine::ToneFeedbackKind::rejected)
                      << '\n'
                      << "undone=" << store.count(owo::engine::ToneFeedbackKind::undone) << '\n';
            for (const auto scene : {owo::engine::StyleScene::chat,
                                     owo::engine::StyleScene::work,
                                     owo::engine::StyleScene::general}) {
                const auto profile = owo::engine::build_tone_feedback_profile(
                    store.entries(), scene, mask);
                std::cout << "scene=" << owo::engine::style_scene_id(scene) << '\t'
                          << profile.entry_count << '\t' << profile.preferences.size()
                          << '\t' << (mask & owo::engine::style_scene_bit(scene) ? "on" : "off")
                          << '\n';
            }
            return 0;
        }
        usage();
        return 2;
    }
    if (command == L"delete" && remaining == 1) {
        std::uint64_t id{};
        if (!parse_u64(argv[index + 1], id) || id == 0) return 4;
        std::size_t removed = 0;
        const auto saved = owo::engine::UserCorpusStore::modify(path, [&](auto& store) {
            removed = store.erase(id) ? 1 : 0;
        });
        return report(saved, removed);
    }
    if (command == L"clear-app" && remaining == 1) {
        const auto target = utf8(argv[index + 1]);
        if (target.empty()) return 4;
        std::size_t removed = 0;
        const auto saved = owo::engine::UserCorpusStore::modify(path, [&](auto& store) {
            removed = store.clear_application(target, now_ms());
        });
        return report(saved, removed);
    }
    if (command == L"clear-all" && remaining == 0) {
        std::size_t removed = 0;
        const auto saved = owo::engine::UserCorpusStore::modify(path, [&](auto& store) {
            removed = store.clear_all(now_ms());
        }, true);
        if (saved.success) {
            // The backup still holds the previous sentences; remove it as well.
            auto backup = path; backup += L".bak";
            std::error_code ignored;
            std::filesystem::remove(backup, ignored);
        }
        return report(saved, removed);
    }
    usage();
    return 2;
}

#include "owo/config/config_store.h"
#include "owo/engine/tone_feedback.h"

#include <filesystem>
#include <iostream>
#include <string>
#include <vector>

namespace {

using owo::engine::StyleScene;
using owo::engine::ToneFeedbackEntry;
using owo::engine::ToneFeedbackKind;
using owo::engine::ToneFeedbackStore;

int failures = 0;

void expect(const bool condition, const char* message) {
    if (condition) return;
    std::cerr << message << "\n";
    ++failures;
}

ToneFeedbackEntry record(const std::uint64_t id, const ToneFeedbackKind kind,
                         const std::string& draft, const std::string& final_text = {},
                         const std::string& application = "WeChat.exe") {
    return {id, 1000 + id, kind, application, draft, final_text};
}

}  // namespace

int main(const int argc, char** argv) {
    if (argc != 2) return 1;
    const std::filesystem::path root(argv[1]);
    std::error_code error;
    std::filesystem::remove_all(root, error);
    std::filesystem::create_directories(root, error);
    if (error) return 2;
    const auto path = root / L"tone-feedback.owtf";

    // Kinds round-trip through their stable ids and nothing else parses.
    {
        for (const auto kind : {ToneFeedbackKind::adopted, ToneFeedbackKind::edited,
                                ToneFeedbackKind::rejected, ToneFeedbackKind::undone}) {
            ToneFeedbackKind parsed{};
            expect(owo::engine::parse_tone_feedback_kind(
                       owo::engine::tone_feedback_kind_id(kind), parsed) && parsed == kind,
                   "kind id did not round-trip");
        }
        ToneFeedbackKind parsed{};
        expect(!owo::engine::parse_tone_feedback_kind("accepted", parsed),
               "unknown kind id was accepted");
    }
    // A missing file loads as empty, and records persist with their kind.
    {
        ToneFeedbackStore store;
        const auto loaded = store.load(path);
        expect(loaded.success && store.entries().empty(), "missing file did not load empty");
        expect(store.append(ToneFeedbackKind::adopted, "WeChat.exe", "好的", "好的", 10) == 1,
               "adopted record was rejected");
        expect(store.append(ToneFeedbackKind::edited, "WeChat.exe", "好的", "好呀", 11) == 2,
               "edited record was rejected");
        expect(store.append(ToneFeedbackKind::rejected, "WeChat.exe", "行吧", {}, 12) == 3,
               "rejected record was rejected");
        expect(store.append(ToneFeedbackKind::undone, "WeChat.exe", "算了", {}, 13) == 4,
               "undone record was rejected");
        expect(store.save().success, "feedback save failed");
        ToneFeedbackStore reloaded;
        expect(reloaded.load(path).success && reloaded.entries().size() == 4,
               "feedback did not round-trip");
        expect(reloaded.count(ToneFeedbackKind::edited) == 1 &&
                   reloaded.count(ToneFeedbackKind::adopted) == 1,
               "kind counts were wrong");
        // Invalid records are refused rather than stored half-formed.
        expect(reloaded.append(ToneFeedbackKind::adopted, "WeChat.exe", "", {}, 14) == 0,
               "empty draft was stored");
        expect(reloaded.append(ToneFeedbackKind::edited, "WeChat.exe", "好的", "好的", 15) == 0,
               "unchanged edit was stored");
        expect(reloaded.append(ToneFeedbackKind::rejected, "WeChat.exe", "行吧", "多余", 16) == 0,
               "rejection with a final text was stored");
        expect(reloaded.append(ToneFeedbackKind::adopted, "WeChat.exe", "好的", "", 17) == 0,
               "adoption without a final text was stored");
        expect(reloaded.append(ToneFeedbackKind::adopted, "WeChat.exe",
                               std::string(owo::engine::kMaximumToneFeedbackTextBytes + 1, 'a'),
                               "x", 18) == 0, "oversized draft was stored");
        expect(reloaded.append(ToneFeedbackKind::adopted, "WeChat.exe", "\xff\xfe", "x", 19) == 0,
               "invalid UTF-8 was stored");
        expect(reloaded.append(static_cast<ToneFeedbackKind>(99), "WeChat.exe", "好的", {}, 20) == 0,
               "unknown enum kind was stored");
        expect(reloaded.append(ToneFeedbackKind::adopted, "WeChat.exe", "好的", "好呀", 21) == 0,
               "modified adoption was stored as unchanged");
        expect(reloaded.erase(2) && !reloaded.erase(2), "erase did not remove exactly once");
        expect(reloaded.clear_all() == 3 && reloaded.entries().empty(),
               "clear_all did not remove every record");
    }
    // The file is encrypted at rest on Windows: the raw bytes must not contain
    // the draft, and a corrupted main file recovers from the backup.
    {
        ToneFeedbackStore store;
        expect(store.load(path).success, "feedback reload failed");
        store.clear_all();
        (void)store.append(ToneFeedbackKind::adopted, "WeChat.exe", "只在本机出现的句子", "只在本机出现的句子", 1);
        expect(store.save().success, "save failed");
        {
            std::FILE* file = nullptr;
            expect(fopen_s(&file, path.string().c_str(), "rb") == 0 && file != nullptr,
                   "cannot reopen feedback file");
            std::string bytes;
            if (file != nullptr) {
                char buffer[4096];
                std::size_t read = 0;
                while ((read = std::fread(buffer, 1, sizeof(buffer), file)) != 0)
                    bytes.append(buffer, read);
                std::fclose(file);
            }
            expect(bytes.find("只在本机出现的句子") == std::string::npos,
                   "feedback draft was stored in clear text");
        }
        // A second save updates the backup with the previous valid file.
        (void)store.append(ToneFeedbackKind::rejected, "WeChat.exe", "不要这句", {}, 2);
        expect(store.save().success, "second save failed");
        {
            std::FILE* file = nullptr;
            if (fopen_s(&file, path.string().c_str(), "wb") == 0 && file != nullptr) {
                std::fputs("broken", file);
                std::fclose(file);
            }
        }
        ToneFeedbackStore recovered;
        const auto result = recovered.load(path);
        expect(result.success && result.recovered_from_backup &&
                   recovered.entries().size() == 1,
               "corrupted feedback did not recover from its backup");
    }
    // The cross-process modify helper round-trips a change.
    {
        const auto modified = ToneFeedbackStore::modify(path, [](ToneFeedbackStore& store) {
            store.clear_all();
            (void)store.append(ToneFeedbackKind::adopted, "WINWORD.EXE", "收到", "收到", 5);
        });
        expect(modified.success, "modify failed");
        ToneFeedbackStore store;
        expect(store.load(path).success && store.entries().size() == 1 &&
                   store.entries().front().application == "WINWORD.EXE",
               "modify did not persist");
    }
    // Deletion must also purge the backup, otherwise corruption can revive a
    // record the user explicitly rolled back.
    {
        const auto deleted = ToneFeedbackStore::modify(path, [](ToneFeedbackStore& store) {
            store.clear_all();
        });
        expect(deleted.success, "rollback save failed");
        auto backup = path; backup += L".bak";
        expect(!std::filesystem::exists(backup), "rollback retained old backup");
        ToneFeedbackStore reloaded;
        expect(reloaded.load(path).success && reloaded.entries().empty(),
               "rolled-back records survived reload");
    }
    // A local edit names what the user wanted; pure insertions and deletions
    // and long rewrites do not.
    {
        std::string from, to;
        expect(owo::engine::tone_feedback_correction("好的，我明天去", "好呀，我明天去", from, to) &&
                   from == "好的" && to == "好呀", "local edit was not extracted");
        expect(owo::engine::tone_feedback_correction("我明天下午三点到", "我明天四点左右到", from, to) &&
                   from == "下午三点" && to == "四点左右", "middle edit was not extracted");
        expect(!owo::engine::tone_feedback_correction("好的", "好的", from, to),
               "identical text produced a correction");
        expect(!owo::engine::tone_feedback_correction("好的", "好的呀", from, to),
               "pure insertion produced a correction");
        expect(!owo::engine::tone_feedback_correction("好的呀", "好的", from, to),
               "pure deletion produced a correction");
        expect(!owo::engine::tone_feedback_correction(
                   std::string(40, 'a') + "X",
                   std::string(40, 'b') + "X", from, to),
               "oversized rewrite produced a correction");
        expect(!owo::engine::tone_feedback_correction("", "好呀", from, to),
               "empty draft produced a correction");
    }
    // Records of one scene become readable preferences; another scene and a
    // scene switched off in the mask contribute nothing.
    {
        const std::vector<ToneFeedbackEntry> entries{
            record(1, ToneFeedbackKind::edited, "好的，我明天去", "好呀，我明天去"),
            record(2, ToneFeedbackKind::edited, "好的，我明天去", "好呀，我明天去"),
            record(3, ToneFeedbackKind::rejected, "行吧"),
            record(4, ToneFeedbackKind::undone, "算了吧"),
            record(5, ToneFeedbackKind::adopted, "这个我可以看看"),
            record(6, ToneFeedbackKind::adopted, "您好，烦请确认", {}, "WINWORD.EXE")};
        const auto profile = owo::engine::build_tone_feedback_profile(
            entries, StyleScene::chat);
        expect(profile.entry_count == 5, "chat record count");
        expect(!profile.preferences.empty(), "chat preferences were empty");
        const auto find = [&profile](const std::string& from, const std::string& to) {
            for (const auto& value : profile.preferences)
                if (value.from == from && value.to == to) return value;
            return owo::engine::TonePhrasePreference{};
        };
        const auto correction = find("好的", "好呀");
        expect(correction.weight == 4 && correction.observations == 2,
               "repeated edits did not accumulate");
        expect(find("行吧", {}).weight == -1, "rejection did not lower a phrasing");
        expect(find("算了吧", {}).weight == -2, "undo did not weigh more than a rejection");
        expect(find("这个我可以看看", {}).weight == 1, "adoption did not reinforce a phrasing");
        expect(profile.preferences.front().from == "好的",
               "strongest preference was not first");
        const auto work = owo::engine::build_tone_feedback_profile(entries, StyleScene::work);
        expect(work.entry_count == 1 && work.preferences.size() == 1,
               "work scene records leaked into chat");
        const auto disabled = owo::engine::build_tone_feedback_profile(
            entries, StyleScene::chat, 0);
        expect(disabled.entry_count == 0 && disabled.preferences.empty(),
               "scene mask was ignored");
        const auto hints = owo::engine::describe_tone_feedback_hints(profile);
        expect(!hints.empty() && hints.size() <= owo::engine::kMaximumTonePromptHints,
               "hints were empty or unbounded");
        expect(hints.front().find("好呀") != std::string::npos,
               "the strongest correction was not hinted");
        expect(owo::engine::describe_tone_feedback_hints({}).empty(),
               "empty profile produced hints");
    }
    // Whole sentences only count as exposure, so style does not move for them.
    {
        const std::vector<ToneFeedbackEntry> entries{
            record(1, ToneFeedbackKind::rejected,
                   "这是一条很长很长的完整句子根本不可能在别的回复里出现"),
            record(3, ToneFeedbackKind::adopted,
                   "这也是一条很长很长的完整句子根本不可能在别的回复里出现")};
        const auto profile = owo::engine::build_tone_feedback_profile(
            entries, StyleScene::chat);
        expect(profile.entry_count == 2 && profile.preferences.empty(),
               "whole sentences moved the style profile");
    }
    // The bound holds: repeated evidence saturates and then ages instead of
    // growing without limit.
    {
        std::vector<ToneFeedbackEntry> entries;
        for (std::uint64_t id = 1; id <= 80; ++id)
            entries.push_back(record(id, ToneFeedbackKind::rejected, "行吧"));
        const auto profile = owo::engine::build_tone_feedback_profile(
            entries, StyleScene::chat);
        bool bounded = true;
        for (const auto& value : profile.preferences)
            bounded = bounded && value.weight >= -owo::engine::kMaximumTonePreferenceWeight;
        expect(bounded && !profile.preferences.empty(),
               "preference weight grew past its bound");
    }
    // Ranking follows the learned preferences and stays stable on ties.
    {
        const std::vector<ToneFeedbackEntry> entries{
            record(1, ToneFeedbackKind::rejected, "行吧"),
            record(2, ToneFeedbackKind::rejected, "行吧"),
            record(3, ToneFeedbackKind::edited, "好的，我看看", "好呀，我看看")};
        const auto profile = owo::engine::build_tone_feedback_profile(
            entries, StyleScene::chat);
        owo::engine::ToneDraftSelection selection;
        selection.candidates = {{"行吧，明天见"}, {"好呀，明天见"}, {"收到"}};
        const auto ranked = owo::engine::apply_tone_feedback(selection, profile);
        expect(ranked.candidates.front().text == "好呀，明天见",
               "preferred phrasing was not ranked first");
        expect(ranked.candidates.back().text == "行吧，明天见",
               "avoided phrasing was not ranked last");
        // With no profile the order is untouched.
        owo::engine::ToneDraftSelection plain;
        plain.candidates = {{"行吧"}, {"好呀"}};
        const auto unchanged = owo::engine::apply_tone_feedback(plain, {});
        expect(unchanged.candidates.front().text == "行吧",
               "empty profile reordered candidates");
        // Ties keep the model's order.
        owo::engine::ToneDraftSelection tied;
        tied.candidates = {{"甲"}, {"乙"}};
        const auto kept = owo::engine::apply_tone_feedback(tied, profile);
        expect(kept.candidates.front().text == "甲", "tie was reordered");
        expect(owo::engine::tone_feedback_adjustment("好呀", profile) > 0,
               "preferred phrasing did not raise the score");
        expect(owo::engine::tone_feedback_adjustment("行吧", profile) < 0,
               "avoided phrasing did not lower the score");
    }
    // Immediate undo retires the exact originating record. A deleted or stale
    // receipt cannot teach negative feedback about an unrelated draft.
    {
        const auto undo_path = root / L"undo.owtf";
        std::uint64_t id = 0, undone = 0;
        expect(ToneFeedbackStore::modify(undo_path, [&](auto& store) {
            id = store.append(ToneFeedbackKind::edited, "WeChat.exe", "好的", "好呀", 100);
        }).success, "seed edit receipt");
        expect(ToneFeedbackStore::modify(undo_path, [&](auto& store) {
            expect(store.undo(id, "WINWORD.EXE", "好的", "好呀", 101) == 0, "wrong application undo denied");
            expect(store.undo(id, "WeChat.exe", "好的", "收到", 101) == 0, "wrong final text undo denied");
            undone = store.undo(id, "WeChat.exe", "好的", "好呀", 101);
        }).success && undone > id, "edit replaced by undo");
        ToneFeedbackStore snapshot;
        expect(snapshot.load(undo_path).success && snapshot.entries().size() == 1 &&
            snapshot.entries()[0].kind == ToneFeedbackKind::undone &&
            snapshot.entries()[0].draft == "好呀", "undo uses actual committed text");
        auto backup = undo_path; backup += L".bak";
        expect(!std::filesystem::exists(backup), "retired adoption cannot return from backup");
        expect(snapshot.undo(id, "WeChat.exe", "好的", "好呀", 102) == 0, "receipt used once");
        const auto adopted = snapshot.append(ToneFeedbackKind::adopted, "WeChat.exe", "收到", "收到", 103);
        snapshot.erase(adopted);
        expect(snapshot.undo(adopted, "WeChat.exe", "收到", "收到", 104) == 0,
            "deleted feedback is not resurrected");
        const auto padded = snapshot.append(ToneFeedbackKind::edited, "WeChat.exe", "好的", " 好呀 ", 105);
        expect(padded != 0 && snapshot.undo(padded, "WeChat.exe", "好的", " 好呀 ", 106) > padded,
            "receipt uses the same whitespace normalization as persistence");
        expect(snapshot.entries().back().kind == ToneFeedbackKind::undone &&
            snapshot.entries().back().draft == "好呀", "padded edit loses its positive record");
    }
    // The engine bound and the settings bound describe the same field.
    static_assert(owo::engine::kMaximumToneHintCodes == 12);
    std::filesystem::remove_all(root, error);
    if (failures == 0) std::cout << "tone feedback tests passed\n";
    return failures == 0 ? 0 : 1;
}

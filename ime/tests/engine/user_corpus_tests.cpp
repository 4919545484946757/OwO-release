#include "owo/engine/user_corpus.h"

#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>

namespace {

using owo::engine::UserCorpusCollector;
using owo::engine::UserCorpusStore;

std::filesystem::path scratch_directory() {
    const auto root = std::filesystem::temp_directory_path() /
        ("owo-user-corpus-tests-" + std::to_string(
            std::chrono::steady_clock::now().time_since_epoch().count()));
    std::filesystem::create_directories(root);
    return root;
}

std::uint64_t now_ms() {
    return static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::milliseconds>(
        std::chrono::system_clock::now().time_since_epoch()).count());
}

bool contains(const std::vector<owo::engine::UserCorpusEntry>& entries, const std::string& text) {
    return std::any_of(entries.begin(), entries.end(),
                       [&text](const auto& entry) { return entry.text == text; });
}

}  // namespace

int main() {
    int failures = 0;
    const auto directory = scratch_directory();
    const auto file = directory / "user-corpus.owuc";

    // A missing file loads as an empty corpus; every mutation round-trips
    // through an encrypted file.
    {
        UserCorpusStore store;
        const auto loaded = store.load(file);
        const auto initially_empty = store.entries().empty();
        const auto first = store.append("WeChat.exe", "明天组会几点？", now_ms());
        const auto second = store.append("WINWORD.EXE", "把跑通的流程写成报告。", now_ms());
        const auto saved = store.save();
        if (!loaded.success || !initially_empty || first == 0 || second == 0 ||
            !saved.success) {
            std::cerr << "corpus append/save failed: ok=" << loaded.success
                      << " err=" << loaded.error << " first=" << first
                      << " second=" << second << " save=" << saved.error << "\n";
            ++failures;
        }
        std::ifstream raw(file, std::ios::binary);
        std::string bytes((std::istreambuf_iterator<char>(raw)), {});
        if (bytes.find("组会") != std::string::npos) {
            std::cerr << "corpus file stored plaintext\n";
            ++failures;
        }
    }
    {
        UserCorpusStore store;
        const auto loaded = store.load(file);
        if (!loaded.success || store.entries().size() != 2 ||
            !contains(store.entries(), "明天组会几点？")) {
            std::cerr << "corpus round trip failed: ok=" << loaded.success
                      << " err=" << loaded.error
                      << " backup=" << loaded.recovered_from_backup
                      << " n=" << store.entries().size() << "\n";
            ++failures;
        }
        // Saving again turns the current file into the backup.
        store.append("WeChat.exe", "留给备份的一句", now_ms());
        if (!store.save().success) {
            std::cerr << "corpus backup save failed\n";
            ++failures;
        }
    }
    // A corrupt primary file falls back to the backup.
    {
        {
            std::ofstream corrupt(file, std::ios::binary | std::ios::trunc);
            corrupt << "not a corpus";
        }
        UserCorpusStore store;
        const auto loaded = store.load(file);
        if (!loaded.success || !loaded.recovered_from_backup || store.entries().size() != 2) {
            std::cerr << "corpus backup recovery failed\n";
            ++failures;
        }
    }
    // Rejected sentences: empty, single character, codes, overlong text.
    {
        UserCorpusStore store;
        (void)store.load(directory / "rejects.owuc");
        if (store.append("WeChat.exe", "", now_ms()) != 0 ||
            store.append("WeChat.exe", "好", now_ms()) != 0 ||
            store.append("WeChat.exe", "验证码 123456", now_ms()) != 0 ||
            store.append("WeChat.exe", std::string(600, 'a'), now_ms()) != 0 ||
            store.append("", "有效句子", now_ms()) != 0) {
            std::cerr << "unacceptable sentence was stored\n";
            ++failures;
        }
    }
    // Deleting one entry, clearing an application and clearing everything are
    // remembered so buffered sentences cannot reappear.
    {
        UserCorpusStore store;
        (void)store.load(file);
        // Current file: one WeChat sentence, one Word sentence and one Word
        // sentence that replaced a deleted entry.
        const auto word = std::find_if(store.entries().begin(), store.entries().end(),
            [](const auto& entry) { return entry.application == "WINWORD.EXE"; });
        if (word == store.entries().end()) {
            std::cerr << "corpus erase failed\n";
            ++failures;
        } else {
            (void)store.erase(word->id);
        }
        store.append("WINWORD.EXE", "删除后重新写入的报告", now_ms());
        const auto clear_at = now_ms();
        if (store.clear_application("wechat.exe", clear_at) != 1 ||
            store.append("WeChat.exe", "清空前的旧句", clear_at - 1000) != 0 ||
            store.append("WeChat.exe", "清空时的边界句", clear_at) != 0) {
            std::cerr << "clear by application did not stick\n";
            ++failures;
        }
        if (store.append("WeChat.exe", "清空后的新句", clear_at + 1) == 0) {
            std::cerr << "post-clear sentence was rejected\n";
            ++failures;
        }
        const auto clear_all = now_ms();
        if (store.clear_all(clear_all) != 2 ||
            store.append("WINWORD.EXE", "清空后仍被写入", clear_all - 1) != 0) {
            std::cerr << "clear all did not stick\n";
            ++failures;
        }
        if (!store.save().success) {
            std::cerr << "corpus clear save failed\n";
            ++failures;
        }
    }
    // Retention prunes old sentences only.
    {
        UserCorpusStore store;
        (void)store.load(directory / "retention.owuc");
        const auto now = now_ms();
        const std::uint64_t day = 24ULL * 60ULL * 60ULL * 1000ULL;
        store.append("WeChat.exe", "很久以前说过的话", now - 40 * day);
        store.append("WeChat.exe", "昨天说过的话", now - day);
        if (store.prune(now, 30) != 1 || store.entries().size() != 1) {
            std::cerr << "retention prune failed\n";
            ++failures;
        }
    }
    // Locked modification is visible to the next load.
    {
        const auto saved = UserCorpusStore::modify(file, [](UserCorpusStore& store) {
            const auto cleared = now_ms();
            store.clear_all(cleared);
            store.append("WeChat.exe", "跨进程写入的句子", cleared + 1);
        }, true);
        UserCorpusStore store;
        const auto reloaded = store.load(file);
        if (!saved.success || !reloaded.success || store.entries().size() != 1) {
            std::cerr << "modify round trip failed: save=" << saved.success
                      << " load=" << reloaded.success << " n=" << store.entries().size() << "\n";
            ++failures;
        }
    }

    // Collector assembles fragments per context and splits on punctuation.
    {
        UserCorpusCollector collector;
        collector.observe("ctx-1", "WeChat.exe", "老师我们这周", 1000);
        collector.observe("ctx-1", "WeChat.exe", "把流程跑通了。然后", 2000);
        collector.observe("ctx-2", "WeChat.exe", "另一个上下文", 3000);
        auto sentences = collector.take_completed();
        if (sentences.size() != 1 || sentences[0].text != "老师我们这周把流程跑通了。") {
            std::cerr << "collector did not split at a sentence boundary\n";
            ++failures;
        }
        // Both open contexts go idle: the unabridged tail of ctx-1 and ctx-2.
        collector.finish_idle(3000 + UserCorpusCollector::kIdleBoundaryMs);
        collector.finish_all();
        sentences = collector.take_completed();
        if (sentences.size() != 2 || sentences[0].text != "然后" ||
            sentences[1].text != "另一个上下文") {
            std::cerr << "collector idle boundary failed\n";
            ++failures;
        }
        // Too long without punctuation splits at the size limit: 512 + 288.
        collector.observe("ctx-3", "WeChat.exe", std::string(400, 'x'), 5000);
        collector.observe("ctx-3", "WeChat.exe", std::string(400, 'y'), 5100);
        collector.finish_all();
        sentences = collector.take_completed();
        if (sentences.size() != 2 || sentences[0].text.size() != 512 ||
            sentences[1].text.size() != 288) {
            std::cerr << "collector size boundary failed\n";
            ++failures;
        }
        // Switching application in the same context starts a new sentence.
        collector.observe("ctx-4", "WeChat.exe", "第一句在这里", 6000);
        collector.observe("ctx-4", "WINWORD.EXE", "换了应用", 6100);
        collector.finish_all();
        sentences = collector.take_completed();
        if (sentences.size() != 2 || sentences[0].application != "WeChat.exe" ||
            sentences[1].application != "WINWORD.EXE") {
            std::cerr << "collector application switch failed\n";
            ++failures;
        }
        collector.discard();
        collector.observe("ctx-5", "WeChat.exe", "丢弃前的半句", 7000);
        collector.discard();
        collector.finish_all();
        if (collector.completed_count() != 0) {
            std::cerr << "collector discard failed\n";
            ++failures;
        }
    }

    std::error_code ignored;
    std::filesystem::remove_all(directory, ignored);
    if (failures == 0) std::cout << "user corpus tests passed\n";
    return failures == 0 ? 0 : 1;
}

#include "owo/engine/tone_session.h"
#include <Windows.h>
#include <filesystem>
#include <iostream>
#include <thread>
int wmain(int argc, wchar_t** argv) {
    if (argc != 3) return 2;
    const std::filesystem::path root(argv[1]);
    const std::wstring mode(argv[2]);
    const auto config_path = root / L"owo.conf";
    owo::config::ConfigStore store;
    if (!store.load(config_path).success) return 3;
    auto config = store.snapshot();
    config.corpus_learning_enabled = config.tone_reply_enabled = true;
    config.corpus_learning_paused = mode == L"paused";
    if (!store.save(config).success) return 4;
    owo::engine::UserCorpusStore corpus;
    if (!corpus.load(root / L"data" / L"user-corpus.owuc").success) return 5;
    corpus.append("WeChat.exe", "好呀到时候见", 1000);
    if (!corpus.save().success) return 6;
    std::stop_source stop;
    std::jthread cancel;
    if (mode == L"cancel") cancel = std::jthread([&](std::stop_token token) {
        for (int n = 0; n < 100 && !token.stop_requested(); ++n) Sleep(10);
        if (!token.stop_requested()) stop.request_stop();
    });
    if (mode == L"revoke") cancel = std::jthread([&](std::stop_token token) {
        for (int n = 0; n < 80 && !token.stop_requested(); ++n) Sleep(10);
        if (!token.stop_requested()) { config.corpus_learning_enabled = false; (void)store.save(config); }
    });
    const auto result = owo::engine::generate_tone_drafts(root, config_path, "WeChat.exe", "明天有空吗",
        owo::engine::ToneDraftMode::reply, stop.get_token());
    if (cancel.joinable()) { cancel.request_stop(); cancel.join(); }
    if (mode == L"ok") {
        if (result.selection.candidates.size() != 2 || result.selection.candidates[0].text != "好的明天见" ||
            result.selection.candidates[1].text != "好呀有空") return 10;
    } else if (!result.selection.candidates.empty() || result.diagnostic.empty()) return 11;
    std::cout << "transport_mode_passed\n";
    return 0;
}

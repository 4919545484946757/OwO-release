#include "owo/engine/user_corpus.h"

#include <Windows.h>

#include <filesystem>
#include <fstream>
#include <string>

namespace {

// Runs a command line and captures its combined output.
bool run(const std::wstring& command, std::string& output) {
    SECURITY_ATTRIBUTES attributes{sizeof(attributes), nullptr, TRUE};
    HANDLE read_end{};
    HANDLE write_end{};
    if (!CreatePipe(&read_end, &write_end, &attributes, 0)) return false;
    SetHandleInformation(read_end, HANDLE_FLAG_INHERIT, 0);
    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdOutput = write_end;
    startup.hStdError = write_end;
    startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
    std::wstring mutable_command = command;
    PROCESS_INFORMATION process{};
    const bool started = CreateProcessW(nullptr, mutable_command.data(), nullptr, nullptr,
                                        TRUE, CREATE_NO_WINDOW, nullptr, nullptr,
                                        &startup, &process) != FALSE;
    CloseHandle(write_end);
    output.clear();
    if (!started) {
        CloseHandle(read_end);
        return false;
    }
    char buffer[4096];
    DWORD read = 0;
    while (ReadFile(read_end, buffer, sizeof(buffer), &read, nullptr) && read != 0)
        output.append(buffer, read);
    WaitForSingleObject(process.hProcess, 10'000);
    DWORD exit_code = 1;
    GetExitCodeProcess(process.hProcess, &exit_code);
    CloseHandle(process.hProcess);
    CloseHandle(process.hThread);
    CloseHandle(read_end);
    return exit_code == 0;
}

std::wstring quote(const std::wstring_view value) {
    return L"\"" + std::wstring(value) + L"\"";
}

}  // namespace

// Seeds an encrypted corpus, then checks that the corpus shell renders the
// style profile from aggregates only and honours the per-scene switches.
int wmain(const int argc, wchar_t** argv) {
    if (argc != 4) return 1;
    const std::filesystem::path root(argv[2]);
    std::error_code error;
    std::filesystem::remove_all(root, error);
    std::filesystem::create_directories(root, error);
    if (error) return 2;
    const auto corpus = root / L"user-corpus.owuc";
    const auto config = root / L"owo.conf";
    const auto seeded = owo::engine::UserCorpusStore::modify(
        corpus, [](owo::engine::UserCorpusStore& store) {
            const struct {
                const char* application;
                const char* text;
            } sentences[]{
                {"WeChat.exe", "哈哈好呀"}, {"WeChat.exe", "行吧"},
                {"WeChat.exe", "感觉挺不错啊"}, {"WeChat.exe", "咋回事？"},
                {"WeChat.exe", "嗯嗯"}, {"WeChat.exe", "那就这样吧"},
                {"WINWORD.EXE", "您好，烦请确认本周周报的评审安排。"},
                {"WINWORD.EXE", "建议先同步方案，再推进后续总结计划。"},
                {"WINWORD.EXE", "感谢各位参与本次会议纪要的评审。"},
                {"WINWORD.EXE", "我会在周报里确认进度。"},
                {"WINWORD.EXE", "请确认方案后同步给大家。"},
                {"WINWORD.EXE", "本周计划推进评审总结。"},
            };
            std::uint64_t at = 1000;
            for (const auto& sentence : sentences)
                (void)store.append(sentence.application, sentence.text, at++);
        });
    if (!seeded.success) return 3;

    const std::wstring shell = quote(argv[1]);
    const std::wstring config_shell = quote(argv[3]);
    const std::wstring corpus_argument = L" --corpus " + quote(corpus.wstring());
    const std::wstring config_argument = L" --config " + quote(config.wstring());
    std::string output;

    // A missing configuration means all scenes are reported.
    if (!run(shell + corpus_argument + config_argument + L" style", output)) return 4;
    if (output.find("【聊天】") == std::string::npos ||
        output.find("【工作】") == std::string::npos ||
        output.find("正式程度") == std::string::npos ||
        output.find("样本：6 句") == std::string::npos ||
        output.find("吧（2）") == std::string::npos) {
        return 5;
    }
    // Only aggregates are printed; the raw sentences stay in the corpus.
    if (output.find("烦请确认本周周报") != std::string::npos ||
        output.find("咋回事") != std::string::npos) return 6;
    // A word seen once is not reported as a habit.
    if (output.find("哈哈") != std::string::npos) return 7;

    // A valid configuration that switches the work scene off hides it. The
    // first save writes the defaults plus the requested mask.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set style_scene_mask 1", output)) return 9;
    if (!run(shell + corpus_argument + config_argument + L" style", output)) return 10;
    if (output.find("【聊天】") == std::string::npos ||
        output.find("【工作】") != std::string::npos) return 11;

    // With every scene off the viewer explains itself instead of guessing.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set style_scene_mask 0", output)) return 12;
    if (!run(shell + corpus_argument + config_argument + L" style", output)) return 13;
    if (output.find("没有句子") == std::string::npos) return 14;

    std::filesystem::remove_all(root, error);
    return 0;
}

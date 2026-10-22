#include "owo/config/config_store.h"
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

bool write(const std::filesystem::path& path, const std::string& text) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(text.data(), static_cast<std::streamsize>(text.size()));
    return output.good();
}

}  // namespace

// Seeds an encrypted corpus and a configuration, then checks the local tone
// drafting surface: the consent gate, the exact request text and the
// deterministic post-filter. Nothing here contacts a model.
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

    // Without a configuration there is no recorded consent, so drafting is off.
    if (run(shell + corpus_argument + config_argument + L" tone 明天有空吗", output))
        return 4;
    if (output.find("没有打开") == std::string::npos) return 5;

    // A stored configuration still has every tone switch off by default.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set style_preferences 简短", output))
        return 6;
    if (run(shell + corpus_argument + config_argument + L" tone 明天有空吗", output))
        return 7;

    // Only opening reply drafting leaves continuation off.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set tone_reply_enabled true", output))
        return 8;
    if (run(shell + corpus_argument + config_argument + L" tone --continue 我觉得",
            output))
        return 9;
    if (output.find("整句续写") == std::string::npos) return 10;
    // An old tone switch cannot bypass a revoked corpus authorization.
    if (run(shell + corpus_argument + config_argument +
            L" --app WeChat.exe tone 明天有空吗", output)) return 53;
    if (output.find("授权语料学习") == std::string::npos) return 54;
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set corpus_learning_enabled true", output)) return 55;

    // Reply drafting in the chat scene prints the exact request and the
    // examples chosen, and never the other scene's sentences.
    if (!run(shell + corpus_argument + config_argument +
             L" --app WeChat.exe tone 明天有空吗", output))
        return 11;
    if (output.find("scene=chat") == std::string::npos ||
        output.find("ready=true") == std::string::npos ||
        output.find("cloud_model=local_only") == std::string::npos ||
        output.find("--- prompt ---") == std::string::npos ||
        output.find("简短") == std::string::npos ||
        output.find("明天有空吗") == std::string::npos ||
        output.find("不能编造") == std::string::npos ||
        output.find("【当前要回复的消息】") == std::string::npos)
        return 12;
    if (output.find("烦请确认本周周报") != std::string::npos) return 13;

    // A scene switched off in settings is never read for drafting. Mask 6
    // keeps work and general, so the chat scene that holds the examples is off.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set style_scene_mask 6", output))
        return 14;
    if (run(shell + corpus_argument + config_argument +
            L" --app WeChat.exe tone 明天有空吗", output))
        return 15;
    if (output.find("关闭") == std::string::npos) return 16;
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set style_scene_mask 7", output))
        return 17;

    // The cloud switch only changes the stated scope; the request is built
    // the same way and nothing is sent by this tool.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set tone_cloud_model_enabled true", output))
        return 18;
    if (!run(shell + corpus_argument + config_argument +
             L" --app WeChat.exe tone 明天有空吗", output))
        return 19;
    if (output.find("cloud_model=allowed") == std::string::npos) return 20;

    // Model output is filtered: an invented number is dropped with a reason,
    // an ordinary line is accepted, and the message's own words are allowed.
    const auto drafts = root / L"drafts.txt";
    if (!write(drafts, "3 点见\n好的明天见\n")) return 21;
    if (!run(shell + corpus_argument + config_argument +
             L" --app WeChat.exe tone-check 明天有空吗 " + quote(drafts.wstring()),
             output))
        return 22;
    if (output.find("accepted=1") == std::string::npos ||
        output.find("candidate=1\t好的明天见") == std::string::npos ||
        output.find("rejected=1") == std::string::npos ||
        output.find("数字或英文") == std::string::npos)
        return 23;

    // Continuation drafting requires the opening to be extended.
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set tone_continuation_enabled true", output))
        return 24;
    if (!write(drafts, "我觉得还行\n明天再说\n")) return 25;
    if (!run(shell + corpus_argument + config_argument +
             L" --app WeChat.exe tone-check --continue 我觉得 " +
             quote(drafts.wstring()), output))
        return 26;
    if (output.find("accepted=1") == std::string::npos ||
        output.find("candidate=1\t我觉得还行") == std::string::npos ||
        output.find("没有接着") == std::string::npos)
        return 27;

    // Feedback records steer later drafting and ranking. Adoption needs the
    // committed text, an edit needs a real change, and the kind is validated.
    const auto feedback = root / L"tone-feedback.owtf";
    const std::wstring feedback_argument = L" --feedback " + quote(feedback.wstring());
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe feedback add rejected \"行吧\"", output))
        return 30;
    if (output.find("id=1") == std::string::npos) return 31;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe feedback add edited \"好的，明天见\" \"好呀，明天见\"", output))
        return 32;
    if (run(shell + corpus_argument + config_argument + feedback_argument +
            L" --app WeChat.exe feedback add adopted \"没写正文\"", output))
        return 33;
    if (run(shell + corpus_argument + config_argument + feedback_argument +
            L" --app WeChat.exe feedback add edited \"一样的\" \"一样的\"", output))
        return 34;
    if (run(shell + corpus_argument + config_argument + feedback_argument +
            L" --app WeChat.exe feedback add unknown \"随便\"", output))
        return 35;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe feedback profile", output))
        return 36;
    if (output.find("scene=chat") == std::string::npos ||
        output.find("records=2") == std::string::npos ||
        output.find("好呀") == std::string::npos ||
        output.find("hint=") == std::string::npos)
        return 37;
    // The hints join the request, and the ranking follows what was learned.
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe tone 明天有空吗", output))
        return 38;
    if (output.find("hint=") == std::string::npos ||
        output.find("用户对此前草稿的修改") == std::string::npos ||
        output.find("把“好的”写成“好呀”") == std::string::npos)
        return 39;
    if (!write(drafts, "行吧，明天见\n好呀，明天见\n")) return 40;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe tone-check 明天有空吗 " + quote(drafts.wstring()),
             output))
        return 41;
    const auto first = output.find("candidate=1\t");
    if (first == std::string::npos ||
        output.find("candidate=1\t好呀，明天见") == std::string::npos)
        return 42;
    // The summary counts records across scenes without printing the drafts.
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" feedback summary", output))
        return 51;
    if (output.find("records=2") == std::string::npos ||
        output.find("edited=1") == std::string::npos ||
        output.find("rejected=1") == std::string::npos ||
        output.find("scene=chat\t2\t2\ton") == std::string::npos ||
        output.find("明天见") != std::string::npos)
        return 52;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" feedback metadata", output)) return 56;
    if (output.find("WeChat.exe") == std::string::npos ||
        output.find("明天见") != std::string::npos ||
        output.find("行吧") != std::string::npos) return 57;
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set corpus_learning_paused true", output)) return 58;
    if (run(shell + corpus_argument + config_argument + feedback_argument +
            L" --app WeChat.exe feedback add rejected 行吧", output)) return 59;
    if (!run(config_shell + L" " + quote(config.wstring()) +
             L" set corpus_learning_paused false", output)) return 60;
    // Listing, deleting one record and clearing everything are explicit.
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" feedback list", output))
        return 43;
    if (output.find("entries=2") == std::string::npos ||
        output.find("rejected") == std::string::npos)
        return 44;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" feedback delete 1", output))
        return 45;
    if (output.find("removed=1") == std::string::npos) return 46;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" feedback clear", output))
        return 47;
    if (output.find("removed=1") == std::string::npos) return 48;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" feedback list", output))
        return 49;
    if (output.find("entries=0") == std::string::npos) return 50;
    auto backup = feedback; backup += L".bak";
    if (std::filesystem::exists(backup)) return 61;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe tone 明天有空吗", output)) return 62;
    if (output.find("hint=") != std::string::npos) return 63;
    if (!run(shell + corpus_argument + config_argument + feedback_argument +
             L" --app WeChat.exe tone-check 明天有空吗 " + quote(drafts.wstring()), output)) return 64;
    if (output.find("candidate=1\t行吧，明天见") == std::string::npos) return 65;

    // Usage errors stay distinguishable from a policy refusal.
    if (run(shell + corpus_argument + config_argument + L" tone", output)) return 28;
    if (run(shell + corpus_argument + config_argument +
            L" --app WeChat.exe tone-check 明天有空吗 " +
            quote((root / L"missing.txt").wstring()), output))
        return 29;

    std::filesystem::remove_all(root, error);
    return 0;
}

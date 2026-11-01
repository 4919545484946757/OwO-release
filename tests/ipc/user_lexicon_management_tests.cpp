#include "owo/ipc/user_lexicon_management.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"
#include "owo/engine/lexicon.h"
#include "owo/engine/binary_lexicon.h"
#define NOMINMAX
#include <Windows.h>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>

namespace {
std::string request(std::string action, std::string revision = "-", std::string entries = {},
                    std::string old = "\t", std::string mode = "merge") {
    return action + '\t' + revision + "\t0\t50\t" + mode + "\n\n" + old + "\n" + entries;
}
std::string bytes(const std::filesystem::path &path) {
    std::ifstream input(path, std::ios::binary);
    return {std::istreambuf_iterator<char>(input), {}};
}
} // namespace
int main() {
    const auto nonce = std::to_string(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto root = std::filesystem::temp_directory_path() / ("owo-lexicon-management-" + nonce);
    std::filesystem::create_directories(root);
    const auto path = root / "frequency.owuf";
    owo::engine::UserFrequencyStore store;
    if (!store.load(path).success)
        return 1;
    store.record("私密上下文", "nihao", "你好", 2);
    store.record("global-only", 3);
    if (!store.flush().success)
        return 2;
    std::uint64_t revision = 0;
    const auto initial = owo::ipc::manage_user_lexicon(store, request("list"), revision);
    if (!initial.ok || initial.text.find("nihao\t你好\t2") == initial.text.npos ||
        initial.text.find("私密上下文") != initial.text.npos)
        return 3;
    const auto before = bytes(path);
    const auto invalid = owo::ipc::manage_user_lexicon(
        store, request("import", "0", "nihao\t你号\t2\nbad\t错误\t0\n"), revision);
    if (invalid.ok || bytes(path) != before || revision != 0)
        return 4;
    const auto duplicate = owo::ipc::manage_user_lexicon(
        store, request("import", "0", "NI'HAO\t你好\t2\nnihao\t你好\t3\n"), revision);
    if (duplicate.ok || bytes(path) != before)
        return 5;
    const auto preview =
        owo::ipc::manage_user_lexicon(store, request("preview", "0", "nihao\t你号\t2\n"), revision);
    if (!preview.ok || preview.mutated || revision != 0 || bytes(path) != before)
        return 6;
    store.record("unflushed-selection");
    const auto imported =
        owo::ipc::manage_user_lexicon(store, request("import", "0", "nihao\t你号\t2\n"), revision);
    if (!imported.ok || !imported.mutated || revision != 1 ||
        store.contextual_count("nihao", "你号") != 2)
        return 7;
    owo::engine::UserFrequencyStore backup;
    if (!backup.load(path.wstring() + L".bak").success ||
        backup.count("unflushed-selection") != 1 || backup.contextual_count("nihao", "你号") != 0)
        return 8;
    if (owo::ipc::manage_user_lexicon(store, request("clear", "0", {}, "\t", "clear-confirmed"),
                                      revision)
            .ok)
        return 9;
    const auto edited = owo::ipc::manage_user_lexicon(
        store, request("save", "1", "shijie\t世界\t1\n", "nihao\t你好"), revision);
    if (!edited.ok || store.contextual_count("nihao", "你好") != 0 || store.count("你好") != 0 ||
        store.language_context_score("私密上下文", "nihao", "你好") != 0 ||
        !store.lookup("shijie", 1).front().preferred)
        return 10;
    if (!owo::ipc::manage_user_lexicon(store, request("delete", "2", {}, "nihao\t你号"), revision)
             .ok ||
        store.count("你号") != 0 || store.count("global-only") != 3)
        return 11;
    if (owo::ipc::manage_user_lexicon(store, request("clear", "3"), revision).ok)
        return 12;
    if (!owo::ipc::manage_user_lexicon(
             store, request("import", "3", "ceshi\t测试\t2\n", "\t", "replace-confirmed"), revision)
             .ok ||
        store.count("global-only") != 0 || store.list_words({}, 0, 50).total != 1)
        return 13;
    if (!owo::ipc::manage_user_lexicon(store, request("clear", "4", {}, "\t", "clear-confirmed"),
                                       revision)
             .ok ||
        store.list_words({}, 0, 50).total != 0 || store.count("测试") != 0)
        return 14;
    owo::engine::UserFrequencyStore unavailable;
    if (!unavailable.load(root / "missing-parent" / "frequency.owuf").success)
        return 15;
    unavailable.record("nihao", "你好", 2);
    std::uint64_t failed_revision = 0;
    if (owo::ipc::manage_user_lexicon(
            unavailable, request("clear", "0", {}, "\t", "clear-confirmed"), failed_revision)
            .ok ||
        failed_revision != 0 || unavailable.count("你好") != 2)
        return 16;

    // Real private Core: mutation is serialized with feedback and invalidates
    // cached candidates; no production pipe, config or user store is touched.
    const auto suffix = std::wstring(nonce.begin(), nonce.end());
    const auto pipe = LR"(\\.\pipe\OwO.Test.UserLexicon.)" + suffix;
    const auto base_path = root / "base.owolx";
    if (!owo::engine::write_binary_lexicon(base_path,
        {{{"ni", "hao"}, "你好", 50000}, {{"ni", "hao"}, "你号", 1}, {{"ni"}, "你", 1000}}).success) return 18;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(base_path).success) return 18;
    store.record("nihao", "你号", 4);
    std::thread core(
        [&] { static_cast<void>(owo::ipc::run_core_server(pipe.c_str(), lexicon, &store)); });
    const auto send = [&](owo::protocol::MessageType type, std::string text) {
        static std::uint64_t id = 0;
        owo::protocol::Message message{type, ++id, 9876, std::move(text)};
        const auto reply = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(message),
                                              std::chrono::seconds(2));
        return owo::protocol::decode_message(reply.response);
    };
    const auto first = send(owo::protocol::MessageType::candidate_request, "nihao");
    const auto export_source = send(owo::protocol::MessageType::user_lexicon_request, "base-file\n");
    const auto exported_path = base_path.u8string();
    const bool export_ok = export_source.validation &&
        export_source.message.type == owo::protocol::MessageType::user_lexicon_response &&
        export_source.message.text == std::string(reinterpret_cast<const char*>(exported_path.data()), exported_path.size());
    // Publish independent source switches while reusing exactly the same
    // candidate cache key. Each snapshot must invalidate old source results.
    int snapshot = 0;
    const auto publish_sources = [&](bool base, bool user, bool imported = false) {
        const auto id = std::string(31, '0') + char('1' + snapshot++);
        const auto directory = root / "dictionaries" / "snapshots" / id;
        std::filesystem::create_directories(directory);
        { std::ofstream file(directory / "active.tsv", std::ios::binary); file << "OWO_DICTIONARIES_V1\n";
          if (imported) file << "nihao\t导入字典\t30\n"; }
        { std::ofstream file(directory / "sources.tsv", std::ios::binary); file << "OWO_DICTIONARY_SOURCES_V1\nbase\t" << base << "\nuser\t" << user << '\n'; }
        { std::ofstream file(root / "dictionaries" / "current"); file << id; }
        std::this_thread::sleep_for(std::chrono::milliseconds(270));
        return send(owo::protocol::MessageType::candidate_request, "nihao");
    };
    const auto learned_only = publish_sources(false, true);
    const auto none = publish_sources(false, false);
    const auto imported_only = publish_sources(false, false, true);
    const auto base_only = publish_sources(true, false);
    const auto restored = publish_sources(true, true);
    const bool sources_ok = learned_only.validation && learned_only.message.text == "你号" &&
        none.validation && none.message.candidates.empty() && imported_only.validation &&
        imported_only.message.text == "导入字典" && base_only.validation && base_only.message.text == "你好" &&
        restored.validation && restored.message.text == "你号" && store.contextual_count("nihao", "你号") == 4;
    HANDLE process_token{}, low_token{};
    bool low_denied = false;
    if (OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE, &process_token) &&
        DuplicateTokenEx(process_token, TOKEN_ALL_ACCESS, nullptr, SecurityImpersonation,
                         TokenImpersonation, &low_token)) {
        BYTE sid[SECURITY_MAX_SID_SIZE]{};
        DWORD size = sizeof(sid);
        if (CreateWellKnownSid(WinLowLabelSid, nullptr, sid, &size)) {
            TOKEN_MANDATORY_LABEL label{};
            label.Label.Sid = sid;
            label.Label.Attributes = SE_GROUP_INTEGRITY;
            if (SetTokenInformation(low_token, TokenIntegrityLevel, &label, sizeof(label) + size) &&
                ImpersonateLoggedOnUser(low_token)) {
                const auto denied =
                    send(owo::protocol::MessageType::user_lexicon_request, request("list"));
                low_denied = denied.validation &&
                             denied.message.type == owo::protocol::MessageType::error_response;
                const auto base_denied = send(owo::protocol::MessageType::user_lexicon_request, "base-file\n");
                low_denied = low_denied && base_denied.validation &&
                    base_denied.message.type == owo::protocol::MessageType::error_response;
                if (!RevertToSelf())
                    TerminateProcess(GetCurrentProcess(), 99);
            }
        }
    }
    if (low_token)
        CloseHandle(low_token);
    if (process_token)
        CloseHandle(process_token);
    const auto listing = send(owo::protocol::MessageType::user_lexicon_request, request("list"));
    const auto rpc_revision = listing.message.text.substr(0, listing.message.text.find('\t'));
    const auto cleared = send(owo::protocol::MessageType::user_lexicon_request,
                              request("clear", rpc_revision, {}, "\t", "clear-confirmed"));
    const auto next = send(owo::protocol::MessageType::candidate_request, "nihao");
    const auto stale = send(owo::protocol::MessageType::user_lexicon_request,
                            request("save", rpc_revision, "nihao\t你号\t20\n"));
    static_cast<void>(send(owo::protocol::MessageType::shutdown_request, {}));
    core.join();
    const bool ok = export_ok && sources_ok && low_denied && first.validation && first.message.text == "你号" &&
                    listing.validation &&
                    listing.message.type == owo::protocol::MessageType::user_lexicon_response &&
                    cleared.validation &&
                    cleared.message.type == owo::protocol::MessageType::user_lexicon_response &&
                    next.validation && next.message.text == "你好" && stale.validation &&
                    stale.message.type == owo::protocol::MessageType::error_response;
    std::filesystem::remove_all(root);
    if (!ok) {
        std::cerr << "Core lexicon transaction/cache/permission contract failed\n";
        return 17;
    }
    std::cout
        << "User lexicon edit/delete/clear/import/preview/backup/revision/cache contracts passed\n";
}

#ifndef OWO_DEVELOPER_TOOLS
#error This program is available only in an explicit developer-tools build.
#endif
#include "owo/agent/agent_protocol.h"
#include "owo/model/model_backend.h"
#include "owo/config/config_monitor.h"
#include "owo/config/config_paths.h"
#include "owo/core/input_method_runtime.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/full_pinyin_schema.h"
#include "owo/engine/user_frequency.h"
#include "owo/ipc/named_pipe.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "owo/plugin/plugin_host.h"
#include "owo/plugin/plugin_installer.h"
#include "owo/plugin/plugin_store.h"
#include "owo/plugin/candidate_theme.h"
#include "owo/plugin/unicode_palette.h"
#include "owo/protocol/messages.h"
#include <Windows.h>
#include <algorithm>
#include <charconv>
#include <condition_variable>
#include <fstream>
#include <iostream>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>
#include <thread>
#include <iomanip>
#include <random>

namespace {
using namespace owo;
using namespace std::chrono_literals;
namespace fs = std::filesystem;
struct Error : std::runtime_error {
    int code;
    bool reported;
    Error(int c, std::string s, bool r = false)
        : runtime_error(std::move(s)), code(c), reported(r) {}
};
void require(bool ok, std::string reason, int code = 2) {
    if (!ok)
        throw Error(code, std::move(reason));
}
std::string utf8(std::wstring_view value) {
    if (value.empty())
        return {};
    const auto n =
        WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
                            static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    require(n > 0, "invalid Unicode argument");
    std::string s(n, '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()),
                        s.data(), n, nullptr, nullptr);
    return s;
}
std::wstring wide(std::string_view value) {
    if (value.empty())
        return {};
    auto n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
                                 static_cast<int>(value.size()), nullptr, 0);
    require(n > 0, "invalid UTF-8");
    std::wstring s(n, L'\0');
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()),
                        s.data(), n);
    return s;
}
std::string json(std::string_view value) {
    std::string out = "\"";
    constexpr char hex[] = "0123456789abcdef";
    for (unsigned char c : value) {
        if (c == '"' || c == '\\') {
            out += '\\';
            out += static_cast<char>(c);
        } else if (c < 32) {
            out += "\\u00";
            out += hex[c >> 4];
            out += hex[c & 15];
        } else
            out += static_cast<char>(c);
    }
    return out + '"';
}
std::string hex_bytes(std::string_view value) {
    constexpr char hex[] = "0123456789abcdef";
    std::string out;
    out.reserve(value.size() * 2);
    for (unsigned char c : value) {
        out += hex[c >> 4];
        out += hex[c & 15];
    }
    return out;
}
bool valid_utf8(std::string_view value) {
    return value.empty() || MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
                                                static_cast<int>(value.size()), nullptr, 0) > 0;
}
std::string read(const fs::path &path, std::size_t limit = 1024 * 1024, bool text = true) {
    require(fs::is_regular_file(path), "file not found");
    require(fs::file_size(path) <= limit, "file exceeds size limit");
    std::ifstream in(path, std::ios::binary);
    require(static_cast<bool>(in), "cannot read file");
    std::string s{std::istreambuf_iterator<char>(in), {}};
    require(s.size() <= limit, "file changed beyond size limit");
    if (text) {
        if (s.starts_with("\xef\xbb\xbf"))
            s.erase(0, 3);
        static_cast<void>(wide(s));
    }
    return s;
}
struct Args {
    std::vector<std::string> commands;
    std::map<std::string, std::string> options;
    std::set<std::string> used;
    Args(int argc, wchar_t **argv) {
        const std::set<std::string> flags{"dev",
                                          "expanded",
                                          "trace",
                                          "run-plugin",
                                          "user-action",
                                          "accept-untrusted",
                                          "allow-full-trust",
                                          "learn",
                                          "sensitive",
                                          "help",
                                          "run-model"};
        for (int i = 1; i < argc; ++i) {
            auto word = utf8(argv[i]);
            if (!word.starts_with("--")) {
                commands.push_back(word);
                continue;
            }
            auto key = word.substr(2);
            require(!options.contains(key), "duplicate option: " + key);
            if (flags.contains(key))
                options[key] = "true";
            else {
                require(i + 1 < argc && !std::wstring_view(argv[i + 1]).starts_with(L"--"),
                        "missing option value: " + key);
                options[key] = utf8(argv[++i]);
            }
        }
    }
    std::string get(std::string key, std::string fallback = {}) {
        used.insert(key);
        auto it = options.find(key);
        return it == options.end() ? fallback : it->second;
    }
    bool flag(std::string key) { return get(key) == "true"; }
    unsigned number(std::string key, unsigned fallback, unsigned maximum = 30000) {
        auto s = get(key, std::to_string(fallback));
        unsigned n{};
        auto r = std::from_chars(s.data(), s.data() + s.size(), n);
        require(r.ec == std::errc{} && r.ptr == s.data() + s.size() && n <= maximum,
                "invalid numeric option: " + key);
        return n;
    }
    void finish() {
        for (auto &[key, value] : options)
            require(used.contains(key), "unsupported option: " + key);
    }
};
bool prefix(const fs::path &path, const fs::path &parent) {
    auto p = path.begin();
    for (auto part : parent) {
        if (p == path.end() || _wcsicmp(p->c_str(), part.c_str()))
            return false;
        ++p;
    }
    return true;
}
void safe_chain(const fs::path &path) {
    for (fs::path p = path; !p.empty() && p != p.parent_path(); p = p.parent_path()) {
        auto attr = GetFileAttributesW(p.c_str());
        require(attr == INVALID_FILE_ATTRIBUTES || !(attr & FILE_ATTRIBUTE_REPARSE_POINT),
                "reparse points are forbidden in developer storage");
    }
}
fs::path session_root(Args &args, bool init) {
    require(args.flag("dev"), "explicit --dev is required", 3);
    auto raw = args.get("root");
    require(!raw.empty(), "--root must name a dedicated empty developer directory", 3);
    if (init)
        args.finish();
    fs::path path(wide(raw));
    require(path.is_absolute(), "developer root must be absolute", 3);
    path = path.lexically_normal();
    require(path.root_name().wstring().size() == 2 && path != path.root_path() &&
                std::distance(path.begin(), path.end()) >= 4,
            "developer root is too broad", 3);
    safe_chain(path);
    const auto local = config::local_data_root();
    require(!local.empty(), "cannot resolve daily OwO directory for isolation", 3);
    require(!prefix(path, local) && !prefix(local, path),
            "daily OwO data or an ancestor cannot be a developer root", 3);
    const auto installed = local.parent_path().parent_path() / L"Programs" / L"OwO";
    require(!prefix(path, installed) && !prefix(installed, path),
            "installed OwO directory or an ancestor is forbidden", 3);
    for (auto part : path) {
        if (part == path.root_name() || part == path.root_directory())
            continue;
        const auto name = part.wstring();
        require(!name.empty() && name.back() != L'.' && name.back() != L' ' &&
                    name.find(L':') == name.npos,
                "ambiguous Windows developer path", 3);
        for (const auto protected_name : {L".git", L".codex", L".agents", L".aws", L".ssh"})
            require(_wcsicmp(name.c_str(), protected_name) != 0, "protected developer root", 3);
    }
    const auto marker = path / L".owo-dev-session";
    if (init) {
        require(!fs::exists(path) || fs::is_empty(path), "init requires a new or empty directory",
                3);
        fs::create_directories(path);
        safe_chain(path);
        HANDLE file = CreateFileW(marker.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                                  FILE_ATTRIBUTE_NORMAL, nullptr);
        require(file != INVALID_HANDLE_VALUE, "cannot create developer marker", 3);
        constexpr char text[] = "OwO.Developer.Session.v1\n";
        DWORD written{};
        const bool ok = WriteFile(file, text, sizeof(text) - 1, &written, nullptr) &&
                        written == sizeof(text) - 1;
        CloseHandle(file);
        require(ok, "cannot write developer marker", 3);
        config::ConfigStore config;
        require(config.load(path / L"owo.conf").success, "cannot initialize isolated config");
        require(config.save(config::AppConfig{}).success, "cannot save isolated config");
        require(plugin::initialize_plugin_store(path / L"plugins").ok,
                "cannot initialize isolated plugin store");
    } else
        require(read(marker, 64) == "OwO.Developer.Session.v1\n",
                "invalid developer session marker", 3);
    // No existing descendants may redirect later managed writes/launches.
    if (fs::exists(path))
        for (const auto &item : fs::recursive_directory_iterator(path)) {
            require(!(GetFileAttributesW(item.path().c_str()) & FILE_ATTRIBUTE_REPARSE_POINT),
                    "developer session contains a reparse point", 3);
        }
    return path;
}
void ok(std::string_view command, std::string_view extra = {}) {
    std::cout << "{\"schema_version\":1,\"ok\":true,\"command\":" << json(command);
    if (!extra.empty())
        std::cout << ',' << extra;
    std::cout << "}\n";
}
void require_developer_plugin_id(std::string_view id) {
    require(!id.empty() && id != "." && id != ".." && id.size() <= 128 &&
                std::all_of(id.begin(), id.end(),
                            [](unsigned char c) {
                                return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
                                       (c >= '0' && c <= '9') || c == '.' || c == '-' || c == '_';
                            }),
            "invalid developer plugin id");
    // AppContainer profile names are global per plugin ID, not per store. Even a
    // malformed/inactive daily record must prevent a developer uninstall from
    // deleting the daily profile. Never write to or repair the daily store.
    const auto daily = config::local_data_root();
    require(!daily.empty(), "cannot resolve daily plugin store for collision protection", 3);
    const auto store = daily / L"plugins";
    safe_chain(store);
    for (const auto &candidate :
         {store / L"versions" / wide(id), store / L"data" / wide(id), store / L"records" / wide(id),
          store / L"active" / wide(std::string(id) + ".record"),
          store / L"authorizations" / wide(id)}) {
        std::error_code error;
        const bool exists = fs::exists(candidate, error);
        require(!error, "cannot check daily plugin id collision", 3);
        require(!exists, "plugin id belongs to the daily store; use a unique developer id", 3);
    }
}
struct Cancellation {
    std::stop_source source;
    std::mutex mutex;
    std::condition_variable_any changed;
    std::jthread timer;
    explicit Cancellation(unsigned after) {
        if (after)
            timer = std::jthread([this, after](std::stop_token stop) {
                std::unique_lock lock(mutex);
                if (!changed.wait_for(lock, stop, std::chrono::milliseconds(after),
                                      [] { return false; }))
                    if (!stop.stop_requested())
                        source.request_stop();
            });
    }
};
class RootLock {
    HANDLE file_{INVALID_HANDLE_VALUE};

  public:
    explicit RootLock(const fs::path &root) {
        file_ =
            CreateFileW((root / L".owo-dev-lock").c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                        OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        require(file_ != INVALID_HANDLE_VALUE, "developer session is busy or cannot be locked", 3);
    }
    ~RootLock() {
        if (file_ != INVALID_HANDLE_VALUE)
            CloseHandle(file_);
    }
};

class InputSession {
    engine::BinaryLexicon disk_;
    engine::MemoryLexicon fallback_{{{{"ni", "hao"}, "你好", 1000},
                                     {{"ni", "hao"}, "你号", 50},
                                     {{"shi", "jie"}, "世界", 900},
                                     {{"ni"}, "你", 800},
                                     {{"hao"}, "好", 800},
                                     {{"shi"}, "是", 800},
                                     {{"jie"}, "界", 800},
                                     {{"xi", "an"}, "西安", 900}}};
    engine::UserFrequencyStore frequency_;
    config::ConfigMonitor config_;
    std::wstring pipe_;
    std::thread server_;
    ipc::PersistentPipeClient client_;
    std::uint64_t id_{};
    unsigned timeout_;

  public:
    InputSession(const fs::path &root, std::string lexicon, unsigned timeout)
        : pipe_(L"\\\\.\\pipe\\OwO.Developer.Core." + std::to_wstring(GetCurrentProcessId()) +
                L"." + std::to_wstring(GetTickCount64())),
          client_(pipe_), timeout_(timeout) {
        const engine::Lexicon *selected = &fallback_;
        if (!lexicon.empty()) {
            auto result = disk_.load(wide(lexicon));
            require(result.success, result.error, 4);
            selected = &disk_;
        }
        require(frequency_.load(root / L"user-frequency.owuf").success,
                "cannot load isolated user frequency");
        require(config_.start(root / L"owo.conf").success, "cannot load isolated config");
        server_ = std::thread([this, selected] {
            static_cast<void>(
                ipc::run_core_server(pipe_.c_str(), *selected, &frequency_, nullptr, &config_));
        });
    }
    ~InputSession() {
        if (!server_.joinable())
            return;
        client_.reset();
        protocol::Message stop;
        stop.type = protocol::MessageType::shutdown_request;
        static_cast<void>(ipc::exchange(pipe_.c_str(), protocol::encode_message(stop), 5s));
        server_.join();
    }
    protocol::Message send(protocol::Message message) {
        message.request_id = ++id_;
        message.context_generation = (std::uint64_t{GetCurrentProcessId()} << 32) | 0x44455631;
        auto result = client_.exchange(protocol::encode_message(message),
                                       std::chrono::milliseconds(timeout_));
        require(static_cast<bool>(result.status), result.status.message, 4);
        auto decoded = protocol::decode_message(result.response);
        require(static_cast<bool>(decoded.validation) &&
                    decoded.message.request_id == message.request_id &&
                    decoded.message.context_generation == message.context_generation,
                "invalid/stale private Core response", 4);
        require(decoded.message.type != protocol::MessageType::error_response, decoded.message.text,
                4);
        return decoded.message;
    }
};

void input(Args &args, const fs::path &root, std::string command) {
    require(command == "parse" || command == "query" || command == "replay" || command == "repl" ||
                command == "bench",
            "unknown input command");
    auto text = args.get("text");
    auto correction = args.get("correction", "on");
    require(correction == "on" || correction == "off", "correction must be on/off");
    require(text.size() <= 256, "built-in input is limited to 256 bytes");
    if (command == "parse") {
        auto paths = args.number("paths", 32, 128);
        require(paths > 0, "paths must be positive");
        args.finish();
        engine::FullPinyinParseMetrics metrics;
        auto parsed = engine::FullPinyinSchema{}.parse(text, paths, correction == "on", &metrics);
        std::cout << "{\"schema_version\":1,\"ok\":" << (parsed.valid ? "true" : "false")
                  << ",\"input\":" << json(parsed.normalized_input) << ",\"paths\":[";
        for (std::size_t i = 0; i < parsed.paths.size(); ++i) {
            if (i)
                std::cout << ',';
            auto &path = parsed.paths[i];
            std::cout << "{\"kind\":" << static_cast<unsigned>(path.match_kind)
                      << ",\"edits\":" << path.edit_count << ",\"syllables\":[";
            for (std::size_t j = 0; j < path.syllables.size(); ++j) {
                if (j)
                    std::cout << ',';
                auto &s = path.syllables[j];
                std::cout << "{\"text\":" << json(s.text) << ",\"begin\":" << s.begin
                          << ",\"end\":" << s.end
                          << ",\"complete\":" << (s.complete ? "true" : "false") << '}';
            }
            std::cout << "]}";
        }
        std::cout << "],\"timing_us\":{\"segment\":" << metrics.segmentation_us
                  << ",\"correction\":" << metrics.correction_us
                  << ",\"assistance\":" << metrics.assistance_us << "}}\n";
        if (!parsed.valid)
            throw Error(1, "input has no valid parse", true);
        return;
    }
    auto lexicon = args.get("lexicon");
    auto timeout = args.number("timeout-ms", 2500);
    require(timeout > 0, "timeout must be positive");
    auto file = command == "replay" ? args.get("file") : std::string{};
    auto context = args.get("context");
    auto previous = args.get("previous");
    bool learn = (command == "replay" || command == "repl") ? args.flag("learn") : false;
    auto samples = command == "bench" ? args.number("samples", 20, 1000) : 0;
    protocol::Message query;
    query.type = protocol::MessageType::candidate_request;
    query.text = text;
    query.correction_enabled = correction == "on";
    query.page = args.number("page", 0, 100);
    query.expanded = args.flag("expanded");
    query.input = previous;
    query.context = context;
    require(!(query.expanded && query.page), "expanded query must start at page zero");
    args.finish();
    InputSession session(root, lexicon, timeout);
    if (command == "query") {
        std::cout << protocol::encode_message(session.send(query)) << '\n';
        return;
    }
    if (command == "bench") {
        require(samples >= 2, "bench needs at least two samples");
        std::vector<double> warm;
        double first{};
        for (unsigned i = 0; i < samples; ++i) {
            auto start = std::chrono::steady_clock::now();
            static_cast<void>(session.send(query));
            auto elapsed =
                std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start)
                    .count();
            if (i == 0)
                first = elapsed;
            else
                warm.push_back(elapsed);
        }
        std::sort(warm.begin(), warm.end());
        ok("input.bench", "\"samples\":" + std::to_string(samples) +
                              ",\"first_roundtrip_ms\":" + std::to_string(first) +
                              ",\"cached_p50_ms\":" + std::to_string(warm[(warm.size() - 1) / 2]));
        return;
    }
    require(command == "replay" || command == "repl", "unknown input command");
    std::istringstream script;
    if (command == "replay") {
        require(!file.empty(), "replay requires --file");
        script.str(read(wide(file)));
    }
    auto &in = command == "replay" ? static_cast<std::istream &>(script) : std::cin;
    std::string line, committed;
    protocol::Message response;
    std::size_t assertions = 0, steps = 0;
    const auto refresh = [&] {
        require(query.text.size() <= 256, "built-in input is limited to 256 bytes");
        if (query.text.empty()) {
            response = {};
            return;
        }
        response = session.send(query);
    };
    while (std::getline(in, line)) {
        if (line.ends_with('\r'))
            line.pop_back();
        if (line.empty() || line.starts_with('#'))
            continue;
        require(++steps <= 10000 && line.size() <= 16384, "replay limit exceeded");
        auto split = line.find(' ');
        auto op = line.substr(0, split);
        auto value = split == line.npos ? std::string{} : line.substr(split + 1);
        if (op == "quit")
            break;
        if (op == "query") {
            query.text = value;
            query.page = 0;
            query.expanded = false;
            refresh();
        } else if (op == "type") {
            query.text += value;
            query.page = 0;
            query.expanded = false;
            refresh();
        } else if (op == "backspace") {
            if (!query.text.empty()) {
                auto end = query.text.size() - 1;
                while (end && (static_cast<unsigned char>(query.text[end]) & 0xc0) == 0x80)
                    --end;
                query.text.resize(end);
            }
            query.page = 0;
            refresh();
        } else if (op == "clear") {
            query.text.clear();
            response = {};
            query.page = 0;
            query.expanded = false;
        } else if (op == "correct") {
            require(value == "on" || value == "off", "invalid correction action");
            query.correction_enabled = value == "on";
            refresh();
        } else if (op == "page" || op == "select") {
            unsigned n{};
            auto r = std::from_chars(value.data(), value.data() + value.size(), n);
            require(r.ec == std::errc{} && r.ptr == value.data() + value.size(),
                    "invalid replay index");
            if (op == "page") {
                require(n <= 100, "page too large");
                query.page = n;
                query.expanded = false;
                refresh();
            } else {
                require(n > 0 && n <= response.candidates.size(),
                        "candidate selection is out of range", 1);
                auto selected = response.candidates[n - 1];
                if (learn) {
                    protocol::Message feedback;
                    feedback.type = protocol::MessageType::candidate_committed;
                    feedback.text = selected;
                    feedback.input = query.text.substr(0, response.candidate_consumed[n - 1]);
                    feedback.context = query.context;
                    static_cast<void>(session.send(feedback));
                }
                committed += selected;
                query.text.erase(0, static_cast<std::size_t>(response.candidate_consumed[n - 1]));
                query.page = 0;
                query.expanded = false;
                refresh();
            }
        } else if (op == "expand") {
            query.expanded = true;
            query.page = 0;
            refresh();
        } else if (op == "expect" || op == "expect-input" || op == "expect-commit") {
            auto actual =
                op == "expect"
                    ? (response.candidates.empty() ? std::string{} : response.candidates.front())
                : op == "expect-input" ? query.text
                                       : committed;
            require(actual == value, "replay assertion failed at step " + std::to_string(steps), 1);
            ++assertions;
        } else
            throw Error(2, "unknown replay action");
        std::cout << "{\"schema_version\":1,\"ok\":true,\"step\":" << steps
                  << ",\"input\":" << json(query.text) << ",\"committed\":" << json(committed)
                  << ",\"candidates\":[";
        for (std::size_t i = 0; i < response.candidates.size(); ++i) {
            if (i)
                std::cout << ',';
            std::cout << json(response.candidates[i]);
        }
        std::cout << "]}\n";
    }
    ok(command,
       "\"steps\":" + std::to_string(steps) + ",\"assertions\":" + std::to_string(assertions));
}

void plugins(Args &args, const fs::path &root, std::string command) {
    const auto store = root / L"plugins";
    auto id = args.get("id");
    auto version = args.get("version");
    if (command == "inspect" || command == "install") {
        require(id.empty() && version.empty(),
                "inspect/install derive identity from the source manifest");
        auto source = args.get("source");
        require(!source.empty(), "--source is required");
        auto inventory = args.get("inventory");
        auto risk = args.get("accept-risk");
        auto permissions = args.get("permissions");
        bool untrusted = args.flag("accept-untrusted"), full = args.flag("allow-full-trust");
        args.finish();
        auto preview = plugin::inspect_plugin_install(wide(source));
        require(preview.ok, preview.diagnostic, 5);
        if (command == "inspect") {
            std::cout << "{\"schema_version\":1,\"ok\":true,\"id\":" << json(preview.manifest.id)
                      << ",\"version\":" << json(preview.manifest.version)
                      << ",\"inventory\":" << json(preview.inventory_sha256)
                      << ",\"requires_risk_consent\":"
                      << (preview.requires_risk_consent ? "true" : "false")
                      << ",\"requires_full_trust\":"
                      << (preview.requires_full_trust ? "true" : "false") << ",\"permissions\":[";
            for (std::size_t i = 0; i < preview.manifest.permissions.size(); ++i) {
                if (i)
                    std::cout << ',';
                std::cout << json(preview.manifest.permissions[i]);
            }
            std::cout << "]}\n";
            return;
        }
        require_developer_plugin_id(preview.manifest.id);
        plugin::PluginInstallResult result;
        if (preview.requires_risk_consent || !risk.empty() || !inventory.empty() ||
            !permissions.empty() || untrusted || full) {
            require(risk == "I_ACCEPT_PLUGIN_RISK_V1" && inventory == preview.inventory_sha256,
                    "explicit risk disclaimer and exact inspected inventory are required", 3);
            plugin::PluginInstallConsent consent;
            consent.inventory_sha256 = inventory;
            consent.disclaimer_version = 1;
            consent.accept_untrusted_publisher = untrusted;
            consent.accept_full_trust = full;
            std::istringstream list(permissions);
            std::string permission;
            while (std::getline(list, permission, ','))
                consent.granted_permissions.push_back(permission);
            result = plugin::install_plugin_package(wide(source), store, consent);
        } else
            result = plugin::install_plugin_package(wide(source), store);
        require(result.ok, result.diagnostic, 5);
        ok(command, "\"id\":" + json(result.manifest.id) +
                        ",\"version\":" + json(result.manifest.version) +
                        ",\"activated\":" + (result.activated ? "true" : "false"));
        return;
    }
    if (command == "list") {
        require(id.empty() && version.empty(), "list does not accept plugin identity options");
        args.finish();
        auto list = plugin::list_installed_plugins(store);
        require(list.ok, list.diagnostic, 5);
        std::cout << "{\"schema_version\":1,\"ok\":true,\"plugins\":[";
        for (std::size_t i = 0; i < list.versions.size(); ++i) {
            if (i)
                std::cout << ',';
            auto &item = list.versions[i];
            std::cout << "{\"id\":" << json(item.manifest.id)
                      << ",\"version\":" << json(item.manifest.version)
                      << ",\"active\":" << (item.active ? "true" : "false") << '}';
        }
        std::cout << "]}\n";
        return;
    }
    if (command == "audit") {
        args.finish();
        auto r = plugin::scan_plugin_store_recovery(store);
        require(r.ok, r.diagnostic, 5);
        std::cout << "{\"schema_version\":1,\"ok\":true,\"items\":[";
        for (std::size_t i = 0; i < r.items.size(); ++i) {
            if (i)
                std::cout << ',';
            auto &v = r.items[i];
            std::cout << "{\"id\":" << json(v.plugin_id) << ",\"version\":" << json(v.version)
                      << ",\"kind\":" << static_cast<unsigned>(v.kind)
                      << ",\"diagnostic\":" << json(v.diagnostic) << '}';
        }
        std::cout << "]}\n";
        return;
    }
    require(!id.empty(), "--id is required");
    if (command == "schema" || command == "settings-read") {
        require(version.empty(), "this command reads the active plugin");
        args.finish();
        auto installed = plugin::query_active_plugin_version(store, id);
        require(installed.ok, installed.diagnostic, 5);
        fs::path path =
            command == "schema"
                ? installed.installed_path / fs::path(wide(installed.manifest.config_schema))
                : store / L"data" / fs::path(wide(id)) / L"settings.json";
        require(command != "schema" || !installed.manifest.config_schema.empty(),
                "plugin has no settings schema");
        auto contents = (command == "settings-read" && !fs::exists(path)) ? std::string("{}")
                                                                          : read(path, 256 * 1024);
        ok(command, "\"contents\":" + json(contents));
        return;
    }
    if (command == "invoke") {
        require(version.empty(),
                "invoke uses the active exact binding; activate the desired version first");
        auto service = args.get("service");
        auto payload = args.get("payload-file");
        auto timeout = args.number("timeout-ms", 1000);
        auto after = args.number("cancel-after-ms", 0);
        bool run = args.flag("run-plugin"), user = args.flag("user-action");
        args.finish();
        require(run, "--run-plugin is required: plugin code may perform external operations", 3);
        require(timeout > 0, "timeout must be positive");
        require(!service.empty(), "--service is required");
        auto data = payload.empty()
                        ? std::string{}
                        : read(wide(payload), plugin::kMaximumPluginPayloadBytes, false);
        require_developer_plugin_id(id);
        auto launched = plugin::launch_active_plugin(store, id, std::chrono::milliseconds(timeout));
        require(static_cast<bool>(launched), launched.diagnostic, 5);
        Cancellation cancellation(after);
        plugin::PluginInvokeRequest request;
        request.service = service;
        request.payload = data;
        request.timeout = std::chrono::milliseconds(timeout);
        request.cancellation = cancellation.source.get_token();
        request.user_initiated = user;
        auto result = launched.session.invoke(std::move(request));
        std::cout << "{\"schema_version\":1,\"ok\":" << (result.ok ? "true" : "false")
                  << ",\"status\":" << static_cast<unsigned>(result.status)
                  << ",\"forced_termination\":" << (result.forced_termination ? "true" : "false")
                  << ",\"payload\":" << (valid_utf8(result.payload) ? json(result.payload) : "null")
                  << ",\"payload_hex\":" << json(hex_bytes(result.payload))
                  << ",\"diagnostic\":" << json(result.diagnostic) << "}\n";
        if (!result.ok)
            throw Error(5, result.diagnostic, true);
        return;
    }
    auto privacy = args.get("accept-agent-privacy");
    args.finish();
    require(!version.empty(), "--version is required");
    require_developer_plugin_id(id);
    if (command == "activate") {
        auto installed = plugin::query_installed_plugin_version(store, id, version);
        require(installed.ok, installed.diagnostic, 5);
        auto &p = installed.manifest.permissions;
        if (std::find(p.begin(), p.end(), "agent.ipc") != p.end())
            require(privacy == "I_ACCEPT_AGENT_IPC_PRIVACY_V1",
                    "Agent activation requires explicit privacy acknowledgement", 3);
        if (std::find(p.begin(), p.end(), "system.full_trust") != p.end()) {
            auto auth = plugin::load_plugin_authorization(store, id, version);
            require(auth.ok && std::all_of(p.begin(), p.end(),
                                           [&](const auto &permission) {
                                               return plugin::is_plugin_permission_granted(
                                                   auth.value, installed.manifest,
                                                   installed.inventory_sha256,
                                                   installed.publisher_certificate_sha256,
                                                   permission);
                                           }),
                    "full-trust authorization is incomplete", 3);
        }
        auto r = plugin::activate_installed_plugin_version(store, id, version);
        require(r.ok, r.diagnostic, 5);
    } else if (command == "deactivate") {
        auto r = plugin::deactivate_plugin(store, id, version);
        require(r.ok, r.diagnostic, 5);
    } else if (command == "revoke") {
        auto existing = plugin::load_plugin_authorization(store, id, version);
        auto installed = plugin::query_installed_plugin_version(store, id, version);
        require(existing.ok && installed.ok, "installed authorization not found", 5);
        auto revoked = plugin::make_plugin_authorization(
            installed.manifest, installed.inventory_sha256, installed.publisher_certificate_sha256,
            {}, existing.value.context);
        require(revoked.ok, revoked.diagnostic, 5);
        auto saved = plugin::save_plugin_authorization(store, revoked.value);
        require(saved.ok, saved.diagnostic, 5);
    } else if (command == "uninstall") {
        auto r = plugin::uninstall_plugin_version(store, id, version);
        require(r.ok, r.diagnostic, 5);
    } else
        throw Error(2, "unknown plugin command");
    ok(command);
}

void scheme(Args &args, const fs::path &root, std::string command) {
    auto id = args.get("id"), scheme = args.get("scheme"), text = args.get("text");
    bool run = args.flag("run-plugin"), sensitive = args.flag("sensitive");
    auto page = args.number("page", 0, 100), size = args.number("page-size", 7, 256),
         select = args.number("select", 0, 256),
         caret = args.number("caret-byte", static_cast<unsigned>(text.size()), 16384);
    auto after = args.number("cancel-after-ms", 0);
    args.finish();
    require(run, "--run-plugin is required for scheme discovery/execution", 3);
    require_developer_plugin_id(id);
    if (command == "list") {
        auto found = core::discover_plugin_input_methods(root / L"plugins", id);
        require(found.validation.ok, found.validation.diagnostic, 5);
        std::cout << "{\"schema_version\":1,\"ok\":true,\"schemes\":[";
        for (std::size_t i = 0; i < found.descriptors.size(); ++i) {
            if (i)
                std::cout << ',';
            auto &d = found.descriptors[i];
            std::cout << "{\"id\":" << json(d.id) << ",\"name\":" << json(d.name)
                      << ",\"max_input_bytes\":" << d.max_input_bytes
                      << ",\"max_candidates\":" << d.max_candidates << '}';
        }
        std::cout << "]}\n";
        return;
    }
    require(command == "query", "unknown scheme command");
    require(size > 0, "page size must be positive");
    core::InputMethodSessionRouter router(root / L"plugins");
    const std::string session = "developer-session";
    const auto owner = static_cast<std::uint64_t>(GetCurrentProcessId());
    auto opened = router.open(owner, session, id, scheme);
    require(opened.ok, opened.diagnostic, 5);
    engine::InputMethodRequest request;
    request.request_id = 1;
    request.session_id = session;
    request.scheme_id = scheme;
    request.input = text;
    request.caret_byte = caret;
    request.page = page;
    request.page_size = size;
    auto future = router.submit(owner, request, sensitive);
    if (after && future.wait_for(std::chrono::milliseconds(after)) != std::future_status::ready)
        static_cast<void>(router.close(owner, session));
    auto result = future.get();
    if (select) {
        require(select <= result.candidates.size(), "selection out of range", 1);
        request.request_id++;
        request.action = engine::InputMethodAction::select;
        request.selected_candidate_id = result.candidates[select - 1].id;
        result = router.submit(owner, request, sensitive).get();
    }
    static_cast<void>(router.close(owner, session));
    std::cout << "{\"schema_version\":1,\"ok\":"
              << (result.status == engine::InputMethodStatus::success ? "true" : "false")
              << ",\"status\":" << static_cast<unsigned>(result.status)
              << ",\"preview\":" << json(result.preview) << ",\"page\":" << result.page
              << ",\"has_more\":" << (result.has_more ? "true" : "false") << ",\"candidates\":[";
    for (std::size_t i = 0; i < result.candidates.size(); ++i) {
        if (i)
            std::cout << ',';
        auto &c = result.candidates[i];
        std::cout << "{\"id\":" << json(c.id) << ",\"text\":" << json(c.text) << ",\"ranges\":[";
        for (std::size_t j = 0; j < c.consumed_ranges.size(); ++j) {
            if (j)
                std::cout << ',';
            std::cout << '[' << c.consumed_ranges[j].begin << ',' << c.consumed_ranges[j].end
                      << ']';
        }
        std::cout << "]}";
    }
    std::cout << "]}\n";
    if (result.status != engine::InputMethodStatus::success)
        throw Error(5, result.diagnostic, true);
}
void rank_model(Args &args) {
    auto backend = args.get("backend", "mock"), file = args.get("candidates-file"),
         bridge = args.get("bridge"), model_file = args.get("model");
    model::ModelRequest request;
    request.request_id = 1;
    request.input = args.get("text");
    request.context = args.get("context");
    auto timeout = args.number("timeout-ms", 100), cancel = args.number("cancel-after-ms", 0),
         latency = args.number("mock-latency-ms", 0);
    bool execute = args.flag("run-model");
    args.finish();
    require(timeout > 0, "timeout must be positive");
    std::istringstream source(read(wide(file), 256 * 1024));
    std::string line;
    while (std::getline(source, line)) {
        if (line.ends_with('\r'))
            line.pop_back();
        if (!line.empty()) {
            require(line.size() <= 4096 && request.candidates.size() < 256,
                    "candidate file limit exceeded");
            request.candidates.push_back(line);
        }
    }
    require(!request.candidates.empty(), "candidate file is empty");
    std::unique_ptr<model::IModelBackend> backend_impl;
    if (backend == "mock") {
        require(bridge.empty() && model_file.empty() && !execute, "mock does not load model files");
        backend_impl = std::make_unique<model::MockModelBackend>(
            model::MockBackendOptions{std::chrono::milliseconds(latency), false});
    } else if (backend == "libime") {
        require(execute, "--run-model is required to load native model bridge code", 3);
        require(latency == 0, "mock latency is not a libime option");
        auto loaded = model::load_libime_backend(bridge, model_file);
        require(static_cast<bool>(loaded), loaded.diagnostic, 5);
        backend_impl = std::move(loaded.backend);
    } else
        throw Error(2, "unsupported model backend");
    Cancellation cancellation(cancel);
    request.model_id = std::string(backend_impl->id());
    request.timeout = std::chrono::milliseconds(timeout);
    auto start = std::chrono::steady_clock::now();
    auto result = backend_impl->rank(request, cancellation.source.get_token());
    std::cout << "{\"schema_version\":1,\"ok\":"
              << (result.status == model::ModelStatus::success ? "true" : "false")
              << ",\"status\":" << static_cast<unsigned>(result.status) << ",\"elapsed_ms\":"
              << std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start)
                     .count()
              << ",\"candidates\":[";
    for (std::size_t i = 0; i < result.candidates.size(); ++i) {
        if (i)
            std::cout << ',';
        std::cout << json(result.candidates[i]);
    }
    std::cout << "]}\n";
    if (result.status != model::ModelStatus::success)
        throw Error(5, result.diagnostic, true);
}
void agent_tools(Args &args, std::string command) {
    if (command == "filter") {
        auto file = args.get("file");
        args.finish();
        auto r = agent::filter_sensitive_text(read(wide(file), 128 * 1024), 128 * 1024);
        require(r.ok, r.diagnostic, 1);
        ok("agent.filter", "\"redacted\":" + std::string(r.redacted ? "true" : "false") +
                               ",\"text\":" + json(r.value));
        return;
    }
    require(command == "sample-request", "unknown agent command");
    auto text = args.get("text"), pinyin = args.get("pinyin");
    args.finish();
    auto filtered = agent::filter_sensitive_text(text, 4096);
    require(filtered.ok, filtered.diagnostic, 1);
    std::random_device random;
    std::ostringstream nonce;
    nonce << std::hex << std::setfill('0');
    for (unsigned i = 0; i < 4; ++i)
        nonce << std::setw(8) << random();
    agent::Request request;
    request.session_id = nonce.str();
    request.user_input = filtered.value;
    request.application = {GetCurrentProcessId(),
                           GetCurrentThreadId(),
                           "owo_dev_cli.exe",
                           "DeveloperSynthetic",
                           "DeveloperSynthetic",
                           "ctx-cli-synthetic",
                           false};
    request.input_details = {pinyin, pinyin, "", pinyin, filtered.value, "agent", false};
    request.request_id = request.session_id + ".sample-1";
    request.idempotency_key = request.request_id;
    request.capabilities = {"protocol.negotiation", "input.structured", "commands.risk"};
    auto encoded = agent::encode_request(request);
    require(!encoded.empty(), "sample request validation failed");
    std::cout << encoded << '\n';
}
void help() {
    std::cout
        << "OwO developer CLI (isolated; no TSF registration, application writes or live Core "
           "control)\n"
           "owo-dev build | test --suite cli|engine|plugin|scheme|agent\n"
           "owo-dev --dev --root <absolute empty session> init\n"
           "owo-dev --dev --root <session> doctor\n"
           "input parse|query|repl|replay|bench --text <input> [--file script] [--lexicon file]\n"
           "  [--correction on|off] [--paths 32] [--page 0] [--expanded] [--context text] "
           "[--previous input] [--learn]\n"
           "config show|import --file <owo.conf>\n"
           "plugin inspect|install --source <folder/zip/owopkg>\n"
           "  install: --inventory <sha256> --accept-risk I_ACCEPT_PLUGIN_RISK_V1\n"
           "  --accept-untrusted [--allow-full-trust] --permissions <comma-separated explicit "
           "grants>\n"
           "plugin list|audit|schema|settings-read|activate|deactivate|revoke|uninstall --id <id> "
           "--version <version>\n"
           "plugin invoke --id <id> --service <service.v1> --run-plugin [--payload-file file]\n"
           "  [--user-action] [--timeout-ms 1000] [--cancel-after-ms 50]\n"
           "scheme list|query --id <plugin> --run-plugin [--scheme id] [--text input]\n"
           "  [--page 0] [--page-size 7] [--caret-byte N] [--select N] [--sensitive] "
           "[--cancel-after-ms N]\n"
           "protocol validate --kind core|plugin|agent-request|agent-response --file <UTF-8 file>\n"
           "resource validate --kind theme|palette --file <resource> [--settings file]\n"
           "agent sample-request --text <synthetic instruction> [--pinyin code] | agent filter "
           "--file file\n"
           "model rank --backend mock|libime --text input --candidates-file file [--context text]\n"
           "  libime: --run-model --bridge <dll> --model <lm>; mock: --mock-latency-ms N\n"
           "All runtime operations need --dev and a session root. JSON v1 stdout; diagnostics "
           "stderr.\n";
}
class QuietLog {
    struct NullBuffer : std::streambuf {
        int_type overflow(int_type c) override { return traits_type::not_eof(c); }
    } sink_;
    std::streambuf *old_;

  public:
    explicit QuietLog(bool trace) : old_(std::clog.rdbuf()) {
        if (!trace)
            std::clog.rdbuf(&sink_);
    }
    ~QuietLog() { std::clog.rdbuf(old_); }
};
} // namespace
int wmain(int argc, wchar_t **argv) {
    SetConsoleOutputCP(CP_UTF8);
    SetConsoleCP(CP_UTF8);
    try {
        Args args(argc, argv);
        if (args.commands.empty() || args.commands[0] == "help" || args.flag("help")) {
            help();
            return 0;
        }
        if (args.commands[0] == "version") {
            args.finish();
            ok("version", "\"developer_build\":true,\"protocol\":" +
                              std::to_string(protocol::kProtocolVersion));
            return 0;
        }
        const auto command = args.commands[0];
        bool trace = args.flag("trace");
        QuietLog quiet(trace);
        if (command == "init" || command == "doctor")
            require(args.commands.size() == 1, "unexpected subcommand");
        auto root = session_root(args, command == "init");
        RootLock lock(root);
        if (command == "init") {
            args.finish();
            ok(command,
               "\"developer_mode\":true,\"isolated\":true,\"root\":" + json(utf8(root.wstring())));
        } else if (command == "doctor") {
            args.finish();
            auto parsed = config::parse_config(read(root / L"owo.conf", 256 * 1024));
            require(parsed.ok, parsed.diagnostic, 2);
            auto installed = plugin::list_installed_plugins(root / L"plugins");
            require(installed.ok, installed.diagnostic, 5);
            SYSTEM_INFO system{};
            GetNativeSystemInfo(&system);
            ok(command,
               "\"developer_mode\":true,\"isolated\":true,\"root\":" + json(utf8(root.wstring())) +
                   ",\"process_bits\":" + std::to_string(sizeof(void *) * 8) +
                   ",\"native_architecture\":" + std::to_string(system.wProcessorArchitecture) +
                   ",\"protocol_version\":" + std::to_string(protocol::kProtocolVersion) +
                   ",\"config_schema\":" + std::to_string(config::kConfigSchemaVersion) +
                   ",\"candidate_page_size\":" + std::to_string(parsed.value.candidate_page_size) +
                   ",\"plugin_versions\":" + std::to_string(installed.versions.size()));
        } else {
            require(args.commands.size() == 2, "command requires one subcommand");
            auto sub = args.commands[1];
            if (command == "input")
                input(args, root, sub);
            else if (command == "plugin")
                plugins(args, root, sub);
            else if (command == "scheme")
                scheme(args, root, sub);
            else if (command == "model") {
                require(sub == "rank", "unknown model command");
                rank_model(args);
            } else if (command == "agent") {
                agent_tools(args, sub);
            } else if (command == "config") {
                auto file = args.get("file");
                args.finish();
                config::ConfigStore store;
                require(store.load(root / L"owo.conf").success, "cannot load session config");
                if (sub == "import") {
                    auto parsed = config::parse_config(read(wide(file), 256 * 1024));
                    require(parsed.ok, parsed.diagnostic);
                    require(store.save(parsed.value).success, "cannot save session config");
                } else
                    require(sub == "show", "unknown config command");
                ok("config", "\"contents\":" + json(config::serialize_config(store.snapshot())));
            } else if (command == "protocol") {
                require(sub == "validate", "unknown protocol command");
                auto kind = args.get("kind"), file = args.get("file");
                args.finish();
                auto bytes = read(wide(file), 1024 * 1024, kind != "plugin");
                bool valid = false;
                if (kind == "core")
                    valid = static_cast<bool>(protocol::decode_message(bytes).validation);
                else if (kind == "plugin")
                    valid = static_cast<bool>(plugin::decode_plugin_message(bytes).validation);
                else if (kind == "agent-request")
                    valid = agent::decode_request(bytes).ok;
                else if (kind == "agent-response")
                    valid = agent::decode_response(bytes).ok;
                else
                    throw Error(2, "unknown protocol kind");
                require(valid, "protocol validation failed", 1);
                ok("protocol.validate");
            } else if (command == "resource") {
                require(sub == "validate", "unknown resource command");
                auto kind = args.get("kind"), file = args.get("file"),
                     settings = args.get("settings");
                args.finish();
                if (kind == "theme") {
                    auto r = plugin::load_candidate_theme_resource(wide(file), wide(settings));
                    require(r.active, r.diagnostic, 1);
                    ok("resource.theme",
                       "\"background_opacity\":" + std::to_string(r.theme.background.opacity));
                } else if (kind == "palette") {
                    require(settings.empty(), "palette does not support --settings");
                    auto r = plugin::load_unicode_palette_resource(wide(file));
                    require(r.active, r.diagnostic, 1);
                    ok("resource.palette",
                       "\"categories\":" + std::to_string(r.palette.categories.size()));
                } else
                    throw Error(2, "unknown resource kind");
            } else
                throw Error(2, "unknown command");
        }
        return 0;
    } catch (const Error &e) {
        if (!e.reported)
            std::cout << "{\"schema_version\":1,\"ok\":false,\"exit_code\":" << e.code
                      << ",\"error\":" << json(e.what()) << "}\n";
        return e.code;
    } catch (const std::exception &) {
        std::cout << "{\"schema_version\":1,\"ok\":false,\"exit_code\":6,\"error\":\"developer "
                     "tool failed\"}\n";
        return 6;
    }
}

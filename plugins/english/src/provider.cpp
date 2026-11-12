#include "provider.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <bcrypt.h>

#include <algorithm>
#include <array>
#include <charconv>
#include <fstream>
#include <map>
#include <optional>
#include <queue>
#include <stdexcept>
#include <utility>

namespace owo::english {
namespace {
constexpr std::size_t max_word = 64;
constexpr std::size_t max_matches = 128;
constexpr std::size_t max_learned = 4096;
constexpr auto alphabet = "abcdefghijklmnopqrstuvwxyz";
using Counts = std::map<std::string, int, std::less<>>;
using Path = std::filesystem::path;

bool letter(char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }
char lower(char c) { return c >= 'A' && c <= 'Z' ? static_cast<char>(c + ('a' - 'A')) : c; }
char upper(char c) { return c >= 'a' && c <= 'z' ? static_cast<char>(c - ('a' - 'A')) : c; }
std::string normalized(std::string_view text) {
    std::string result(text);
    std::ranges::transform(result, result.begin(), lower);
    return result;
}
bool word(std::string_view text) {
    if (text.empty() || text.size() > max_word || !letter(text.front()) || !letter(text.back())) return false;
    for (std::size_t i = 1; i < text.size(); ++i)
        if (!letter(text[i]) && ((text[i] != '\'' && text[i] != '-') || !letter(text[i - 1]))) return false;
    return true;
}
std::string styled(std::string text, std::string_view typed, bool prefix) {
    const auto upper_count = std::ranges::count_if(typed, [](char c) { return c >= 'A' && c <= 'Z'; });
    const auto letter_count = std::ranges::count_if(typed, letter);
    if (letter_count > 1 && upper_count == letter_count)
        std::ranges::transform(text, text.begin(), upper);
    else if (upper_count == 1 && typed.front() >= 'A' && typed.front() <= 'Z')
        text.front() = upper(text.front());
    else if (prefix && upper_count != 0) // preserve the user's mixed-case prefix
        text.replace(0, typed.size(), typed);
    return text;
}

std::string read_file(const Path& path, std::size_t limit) {
    const auto flags = GetFileAttributesW(path.c_str());
    if (flags == INVALID_FILE_ATTRIBUTES || flags & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) return {};
    std::error_code error;
    const auto size = std::filesystem::file_size(path, error);
    if (error || size == 0 || size > limit) return {};
    std::string bytes(static_cast<std::size_t>(size), '\0');
    std::ifstream file(path, std::ios::binary);
    if (!file.read(bytes.data(), static_cast<std::streamsize>(size)) || file.peek() != std::char_traits<char>::eof()) return {};
    return bytes;
}

// Minimal, bounded reader for the unchanged wordfreq v1 cBpack (array of word arrays).
struct Cbpack {
    std::string_view bytes;
    std::size_t take(unsigned width) {
        if (width > bytes.size()) throw std::runtime_error("Truncated wordfreq data");
        std::size_t n{};
        while (width--) { n = (n << 8) | static_cast<unsigned char>(bytes.front()); bytes.remove_prefix(1); }
        return n;
    }
    std::size_t array() {
        const auto op = take(1);
        if ((op & 0xf0) == 0x90) return op & 15;
        if (op == 0xdc || op == 0xdd) return take(op == 0xdc ? 2 : 4);
        throw std::runtime_error("Invalid wordfreq array");
    }
    std::string_view string() {
        const auto op = take(1);
        std::size_t n{};
        if ((op & 0xe0) == 0xa0) n = op & 31;
        else if (op >= 0xd9 && op <= 0xdb) n = take(1U << (op - 0xd9));
        else throw std::runtime_error("Invalid wordfreq token");
        if (n > bytes.size()) throw std::runtime_error("Truncated wordfreq token");
        const auto value = bytes.substr(0, n);
        bytes.remove_prefix(n);
        return value;
    }
};
using WordList = std::vector<std::pair<std::string, int>>;
WordList read_wordfreq(const Path& path) {
    const auto data = read_file(path, 8 * 1024 * 1024);
    Cbpack pack{data};
    const auto size = pack.array();
    if (size < 2 || size > 1001 || pack.take(1) != 0x82 ||
        pack.string() != "format" || pack.string() != "cB" ||
        pack.string() != "version" || pack.take(1) != 1) throw std::runtime_error("Invalid wordfreq format");
    WordList rows;
    rows.reserve(300000);
    for (std::size_t bin = 1; bin < size; ++bin) {
        const auto n = pack.array();
        if (n > 500000 || n > pack.bytes.size()) throw std::runtime_error("Invalid wordfreq bucket");
        for (std::size_t i = 0; i < n; ++i) {
            const auto value = pack.string();
            if (word(value)) rows.emplace_back(normalized(value), 1001 - static_cast<int>(bin));
        }
    }
    if (!pack.bytes.empty() || rows.empty()) throw std::runtime_error("Invalid wordfreq data");
    std::ranges::sort(rows, [](const auto& a, const auto& b) {
        return a.first == b.first ? a.second > b.second : a.first < b.first;
    });
    rows.erase(std::unique(rows.begin(), rows.end(), [](const auto& a, const auto& b) {
        return a.first == b.first;
    }), rows.end());
    return rows;
}

bool integer(std::string_view text, int& result) {
    if (text.empty()) return false;
    const auto end = text.data() + text.size();
    const auto parsed = std::from_chars(text.data(), end, result);
    return parsed.ec == std::errc{} && parsed.ptr == end;
}
Counts read_words(const Path& file, bool hunspell = false, bool learning = false) {
    const auto data = read_file(file, learning ? 512 * 1024 : 16 * 1024 * 1024);
    std::string_view rest(data);
    if (rest.starts_with("\xef\xbb\xbf")) rest.remove_prefix(3);
    Counts result;
    const auto limit = learning ? max_learned : 200000;
    for (bool first = true; !rest.empty() && result.size() < limit; first = false) {
        const auto pos = rest.find('\n');
        auto line = rest.substr(0, pos);
        rest.remove_prefix(pos == rest.npos ? rest.size() : pos + 1);
        if (line.ends_with('\r')) line.remove_suffix(1);
        int value = 1;
        auto token = line;
        if (hunspell) {
            if (first) { if (!integer(line, value) || value < 1) return {}; continue; }
            token = line.substr(0, line.find_first_of("/\t "));
        } else if (const auto split = line.find('\t'); split != line.npos) {
            token = line.substr(0, split);
            if (!integer(line.substr(split + 1), value) || value < 1) continue;
        }
        if (!word(token)) continue;
        auto& score = result[normalized(token)];
        score = std::max(score, hunspell ? 100 : std::min(value, 1000));
    }
    return result;
}

std::string nonce() {
    unsigned char bytes[16];
    if (BCryptGenRandom(nullptr, bytes, sizeof(bytes), BCRYPT_USE_SYSTEM_PREFERRED_RNG) != 0) return {};
    std::string token;
    constexpr auto hex = "0123456789abcdef";
    for (const auto byte : bytes) { token += hex[byte >> 4]; token += hex[byte & 15]; }
    return token;
}
bool learn(const Path& path, std::string_view token, const std::string& value) {
    if (path.empty()) return false;
    std::error_code error;
    std::filesystem::create_directories(path.parent_path(), error);
    if (error) return false;
    const auto flags = GetFileAttributesW(path.parent_path().c_str());
    if (flags == INVALID_FILE_ATTRIBUTES || flags & FILE_ATTRIBUTE_REPARSE_POINT) return false;
    auto lock_path = path; lock_path += L".lock";
    const auto lock = CreateFileW(lock_path.c_str(), GENERIC_WRITE, 0, nullptr, OPEN_ALWAYS,
        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    if (lock == INVALID_HANDLE_VALUE) return false;
    struct Close { HANDLE value; ~Close() { CloseHandle(value); } } close{lock};
    auto counts = read_words(path, false, true);
    if (counts.size() >= max_learned && !counts.contains(value)) {
        const auto lowest = std::ranges::min_element(counts, {}, [](const auto& row) { return row.second; });
        counts.erase(lowest);
    }
    auto& count = counts[value]; count = std::min(count + 1, 1000);
    std::string text;
    for (const auto& [entry, frequency] : counts) text += entry + '\t' + std::to_string(frequency) + '\n';
    auto temp = path; temp += ".pending."; temp += token;
    const auto file = CreateFileW(temp.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return false;
    DWORD written{};
    const auto bytes = static_cast<DWORD>(text.size());
    const bool saved = WriteFile(file, text.data(), bytes, &written, nullptr) && written == bytes && FlushFileBuffers(file);
    CloseHandle(file);
    if (saved && MoveFileExW(temp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) return true;
    DeleteFileW(temp.c_str());
    return false;
}
bool enabled(std::string_view json, std::string_view field) {
    const auto at = json.find(field);
    if (at == json.npos) return true;
    auto tail = json.substr(at + field.size());
    const auto colon = tail.find_first_not_of(" \t\r\n");
    if (colon == tail.npos || tail[colon] != ':') return true;
    tail.remove_prefix(colon + 1);
    const auto first = tail.find_first_not_of(" \t\r\n");
    return first == tail.npos || !tail.substr(first).starts_with("false");
}

struct Stamp {
    std::uintmax_t size{};
    std::filesystem::file_time_type time{};
    bool operator==(const Stamp&) const = default;
};
Stamp stamp(const Path& path) {
    std::error_code error;
    const auto bytes = std::filesystem::file_size(path, error);
    if (error) return {};
    const auto time = std::filesystem::last_write_time(path, error);
    return error ? Stamp{} : Stamp{bytes, time};
}
struct Ranked {
    std::string value;
    int score{};
    bool better_than(const Ranked& other) const {
        if (score != other.score) return score > other.score;
        if (value.size() != other.value.size()) return value.size() < other.value.size();
        return value < other.value;
    }
};
} // namespace

engine::InputMethodDescriptor descriptor() {
    engine::InputMethodDescriptor d;
    d.plugin_id = "owo.plugin.english"; d.id = d.plugin_id + "/english";
    d.name = "英文输入"; d.language_tag = "en"; d.kind = engine::InputMethodKind::phonetic;
    d.alphabet = U"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ'-";
    d.max_input_bytes = max_word; d.max_candidates = max_matches; d.commit_confirmation = true;
    return d;
}

struct Provider::State {
    struct Selection {
        std::string token, session, input, value;
        std::size_t page{};
        bool matches(const engine::InputMethodRequest& r) const {
            return token == r.selected_candidate_id && session == r.session_id && input == r.input && page == r.page;
        }
    };
    explicit State(const Path& source, Path directory) : base(read_wordfreq(source)), root(std::move(directory)) {}
    const WordList base;
    const Path root;
    std::array<Stamp, 5> stamps{};
    Counts custom, learned;
    bool completions{true}, spelling{true}, learning{true};
    std::mutex mutex;
    std::string session, input, cached_input;
    std::size_t page{}, page_size{};
    std::uint64_t sequence{};
    std::optional<Selection> prepared, acknowledged;
    std::vector<engine::InputMethodCandidate> issued, cached;

    void refresh() {
        if (root.empty()) return;
        const std::array files{root.parent_path().parent_path() / L"settings.json", root / L"user-lexicon.tsv",
            root / L"dictionaries" / L"libreoffice-en-us.tsv", root / L"dictionaries" / L"libreoffice-en-gb.tsv",
            root / L"learned.tsv"};
        auto next = stamps;
        std::ranges::transform(files, next.begin(), stamp);
        if (next == stamps) return;
        if (next[0] != stamps[0]) {
            const auto json = read_file(files[0], 65536);
            completions = enabled(json, "\"show_completions\"");
            spelling = enabled(json, "\"show_spelling_suggestions\"");
            learning = enabled(json, "\"learn_selected_words\"");
        }
        if (next[1] != stamps[1] || next[2] != stamps[2] || next[3] != stamps[3]) {
            custom.clear();
            const auto merge = [&](Counts rows, int bonus) {
                for (const auto& [entry, value] : rows) {
                    auto& score = custom[entry]; score = std::max(score, bonus + value);
                }
            };
            merge(read_words(files[1]), 350);
            const auto attributes = GetFileAttributesW(files[2].parent_path().c_str());
            if (attributes != INVALID_FILE_ATTRIBUTES && !(attributes & FILE_ATTRIBUTE_REPARSE_POINT)) {
                merge(read_words(files[2], true), 0);
                merge(read_words(files[3], true), 0);
            }
        }
        if (next[4] != stamps[4]) learned = read_words(files[4], false, true);
        stamps = next;
        cached_input.clear();
    }
    int base_score(std::string_view key) const {
        const auto it = std::ranges::lower_bound(base, key, {}, &WordList::value_type::first);
        return it == base.end() || it->first != key ? 0 : it->second;
    }
    static int find(const Counts& rows, std::string_view key) {
        const auto it = rows.find(key); return it == rows.end() ? 0 : it->second;
    }
    int score(std::string_view key, int frequency, int extra, int selections) const {
        return std::max({frequency, extra, selections ? 300 : 0}) + (learning ? std::min(selections, 30) * 14 : 0)
            - static_cast<int>(key.size());
    }
    std::vector<engine::InputMethodCandidate> candidates(std::string_view typed, std::stop_token stop) const {
        std::vector<engine::InputMethodCandidate> result;
        const auto add = [&](std::string id, std::string text, std::string annotation) {
            result.push_back({std::move(id), std::move(text), std::move(annotation), {{0, typed.size()}}});
        };
        const auto key = normalized(typed);
        add("literal", std::string(typed), "原样");
        std::vector<Ranked> prefixes;
        if (completions && letter(key.front())) {
            const auto compare = [](const Ranked& a, const Ranked& b) { return a.better_than(b); };
            std::priority_queue<Ranked, std::vector<Ranked>, decltype(compare)> best(compare);
            const auto push = [&](const std::string& value, int frequency, int extra, int selections) {
                if (value == key) return;
                const auto weight = score(value, frequency, extra, selections);
                if (best.size() < max_matches - 1 || weight >= best.top().score) {
                    Ranked row{value, weight};
                    if (best.size() < max_matches - 1) best.push(std::move(row));
                    else if (row.better_than(best.top())) { best.pop(); best.push(std::move(row)); }
                }
            };
            for (auto it = std::ranges::lower_bound(base, key, {}, &WordList::value_type::first);
                 it != base.end() && it->first.starts_with(key); ++it) {
                if (stop.stop_requested()) return {};
                push(it->first, it->second, find(custom, it->first), find(learned, it->first));
            }
            for (auto it = custom.lower_bound(key); it != custom.end() && it->first.starts_with(key); ++it) {
                if (stop.stop_requested()) return {};
                if (!base_score(it->first)) push(it->first, 0, it->second, find(learned, it->first));
            }
            if (learning) for (auto it = learned.lower_bound(key); it != learned.end() && it->first.starts_with(key); ++it) {
                if (stop.stop_requested()) return {};
                if (!base_score(it->first) && !custom.contains(it->first)) push(it->first, 0, 0, it->second);
            }
            while (!best.empty()) { prefixes.push_back(best.top()); best.pop(); }
            std::ranges::reverse(prefixes);
        }
        const auto add_prefix = [&](const Ranked& row) {
            if (result.size() < max_matches) add("word:" + row.value, styled(row.value, typed, true), "补全");
        };
        // Frequency corpora contain rare misspellings too; a rare literal does not
        // hide a substantially more common one-edit correction.
        constexpr int common = 400;
        const auto strong = std::ranges::find_if(prefixes, [](const Ranked& row) { return row.score < common; });
        for (auto it = prefixes.begin(); it != strong; ++it) add_prefix(*it);
        const auto exact = [&](std::string_view value) {
            return base_score(value) >= common || custom.contains(value) || (learning && learned.contains(value));
        };
        if (!spelling || key.size() < 4 || key.size() > 20 || !word(key) || strong - prefixes.begin() >= 3 || exact(key)) {
            for (auto it = strong; it != prefixes.end(); ++it) add_prefix(*it);
            return result;
        }

        Counts alternatives;
        const auto consider = [&](const std::string& edit) {
            if (edit.starts_with(key) && completions) return; // already in prefix candidates
            if (edit != key && exact(edit)) alternatives.try_emplace(edit,
                score(edit, base_score(edit), find(custom, edit), find(learned, edit)));
        };
        const std::string_view letters(alphabet);
        for (std::size_t i = 0; i <= key.size(); ++i) {
            if (stop.stop_requested()) return {};
            auto edit = key;
            if (i < key.size()) {
                edit.erase(i, 1); consider(edit);
                edit = key;
                if (i + 1 < key.size()) { std::swap(edit[i], edit[i + 1]); consider(edit); }
                for (char c : letters) { edit = key; edit[i] = c; consider(edit); }
            }
            edit = key; edit.insert(i, 1, 'a');
            for (char c : letters) { edit[i] = c; consider(edit); }
        }
        std::vector<Ranked> ranked;
        for (const auto& [value, weight] : alternatives) ranked.push_back({value, weight});
        std::ranges::sort(ranked, [](const auto& a, const auto& b) { return a.better_than(b); });
        for (std::size_t i = 0; i < ranked.size() && i < 5; ++i)
            add("word:" + ranked[i].value, styled(ranked[i].value, typed, false), "拼写建议");
        for (auto it = strong; it != prefixes.end(); ++it) add_prefix(*it);
        return result;
    }
};

Provider::Provider(const Path& wordfreq, Path data) : state_(std::make_unique<State>(wordfreq, std::move(data))) {}
Provider::~Provider() = default;

engine::InputMethodResponse Provider::handle(const engine::InputMethodRequest& r, std::stop_token stop) {
    using namespace engine;
    auto& s = *state_;
    const std::lock_guard lock(s.mutex);
    InputMethodResponse out;
    out.request_id = r.request_id; out.session_id = r.session_id; out.scheme_id = r.scheme_id;
    out.page = r.page; out.preview = r.input;
    const auto error = [&](std::string diagnostic) {
        out.status = InputMethodStatus::error; out.diagnostic = std::move(diagnostic); return out;
    };
    static const auto spec = descriptor();
    if (!validate_input_method_request(spec, r).ok) return error("Invalid English scheme request");
    if (stop.stop_requested()) { out.status = InputMethodStatus::cancelled; return out; }
    if (r.action == InputMethodAction::committed) {
        if (s.acknowledged && s.acknowledged->matches(r)) return out;
        if (!s.prepared || !s.prepared->matches(r)) return error("Unknown commit transaction");
        const auto& value = s.prepared->value;
        if (s.learning && !s.root.empty() && word(value)) {
            if (!learn(s.root / L"learned.tsv", s.prepared->token, normalized(value))) return error("English learning storage unavailable");
            s.learned = read_words(s.root / L"learned.tsv", false, true);
            s.stamps[4] = stamp(s.root / L"learned.tsv");
            s.cached_input.clear();
        }
        s.acknowledged = std::exchange(s.prepared, std::nullopt);
        s.sequence = r.request_id;
        return out;
    }
    if (s.session == r.session_id && r.request_id <= s.sequence) return error("Stale English scheme request");
    if (r.action == InputMethodAction::select) {
        if (s.session != r.session_id || s.input != r.input || s.page != r.page || s.page_size != r.page_size)
            return error("Stale English candidate page");
        const auto found = std::ranges::find(s.issued, r.selected_candidate_id, &InputMethodCandidate::id);
        if (found == s.issued.end()) return error("Unknown English candidate");
        auto token = nonce(); if (token.empty()) return error("Commit identity unavailable");
        s.prepared = State::Selection{token, r.session_id, r.input, found->text, r.page};
        out.commit_id = std::move(token); out.candidates.push_back(*found);
        s.sequence = r.request_id; s.issued.clear();
        return out;
    }
    if (s.session != r.session_id || s.input != r.input) s.refresh();
    s.session = r.session_id; s.input = r.input; s.page = r.page; s.page_size = r.page_size;
    s.sequence = r.request_id; s.prepared.reset(); s.issued.clear();
    if (r.action == InputMethodAction::cancel) { out.status = InputMethodStatus::cancelled; return out; }
    if (r.input.empty()) return out;
    if (s.cached_input != r.input) {
        auto next = s.candidates(r.input, stop);
        if (stop.stop_requested()) { out.status = InputMethodStatus::cancelled; return out; }
        s.cached = std::move(next); s.cached_input = r.input;
    }
    const auto begin = std::min(r.page * r.page_size, s.cached.size());
    const auto end = std::min(begin + r.page_size, s.cached.size());
    out.candidates.assign(s.cached.begin() + begin, s.cached.begin() + end);
    out.has_more = end < s.cached.size();
    s.issued = out.candidates;
    return out;
}
} // namespace owo::english

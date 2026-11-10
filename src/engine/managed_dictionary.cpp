#include "owo/engine/managed_dictionary.h"
#include "owo/engine/full_pinyin_schema.h"
#include <algorithm>
#include <atomic>
#include <charconv>
#include <fstream>
#include <iostream>
#include <new>
#include <set>
#include <stdexcept>
#include <unordered_map>
#ifdef _WIN32
#include <Windows.h>
#endif

namespace owo::engine {
namespace {
std::string compact(std::string_view input) {
    std::string result;
    for (char c : input) if (c != '\'') result += c >= 'A' && c <= 'Z' ? char(c + 'a' - 'A') : c;
    return result;
}
bool safe_directory(const std::filesystem::path& path) {
    std::error_code error;
    const auto status = std::filesystem::symlink_status(path, error);
    if (error || !std::filesystem::is_directory(status) || std::filesystem::is_symlink(status)) return false;
#ifdef _WIN32
    const auto attributes = GetFileAttributesW(path.c_str());
    if (attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_REPARSE_POINT)) return false;
#endif
    return true;
}
bool safe_file(const std::filesystem::path& path, std::uintmax_t maximum) {
    std::error_code error;
    const auto status = std::filesystem::symlink_status(path, error);
    if (error || !std::filesystem::is_regular_file(status) || std::filesystem::is_symlink(status)) return false;
#ifdef _WIN32
    const auto attributes = GetFileAttributesW(path.c_str());
    if (attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_REPARSE_POINT)) return false;
#endif
    const auto size = std::filesystem::file_size(path, error);
    return !error && size <= maximum;
}
LexiconIoResult read_rows(const std::filesystem::path& path, std::vector<UserLexiconEntry>& staged) {
    const auto fail = [](const char* error) { return LexiconIoResult{false, error}; };
    if (!safe_file(path, 256 * 1024 * 1024))
        return fail("dictionary snapshot is missing, linked or too large");
    std::ifstream file(path, std::ios::binary);
    std::string line;
    if (!std::getline(file, line) || line != "OWO_DICTIONARIES_V1") return fail("dictionary snapshot header is invalid");
    while (std::getline(file, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        const auto a = line.find('\t'), b = a == std::string::npos ? a : line.find('\t', a + 1);
        if (a == std::string::npos || b == std::string::npos || line.find('\t', b + 1) != std::string::npos || staged.size() >= 500000)
            return fail("dictionary snapshot row is invalid or exceeds limits");
        std::uint32_t weight{};
        const auto parsed = std::from_chars(line.data() + b + 1, line.data() + line.size(), weight);
        // Preserve upstream syllable boundaries until the phonetic index has
        // been built. Old compact snapshots remain readable.
        UserLexiconEntry entry{line.substr(a + 1, b - a - 1), weight, line.substr(0, a), false};
        if (parsed.ec != std::errc{} || parsed.ptr != line.data() + line.size() || !UserFrequencyStore::validate_word(entry).empty())
            return fail("dictionary snapshot entry failed validation");
        staged.push_back(std::move(entry));
    }
    if (!file.eof()) return fail("dictionary snapshot read failed");
    return {true, {}};
}
}
LexiconIoResult prepare_managed_dictionary_snapshot(const std::filesystem::path& directory) {
    const auto fail = [](const char* error) { return LexiconIoResult{false, error}; };
    if (!safe_directory(directory)) return fail("linked dictionary directory rejected");
    const auto destination = directory / "runtime-v1";
    std::error_code error;
    if (std::filesystem::exists(destination, error)) {
        if (!safe_directory(destination)) return fail("linked dictionary runtime rejected");
        if (!safe_file(destination / "phonetic.owolx", 1024ULL * 1024 * 1024)) return fail("invalid compiled dictionary file");
        BinaryLexicon check;
        const auto loaded = check.load(destination / "phonetic.owolx");
        if (!loaded.success) return loaded;
        std::vector<UserLexiconEntry> aliases;
        return read_rows(destination / "aliases.tsv", aliases);
    }
    if (error) return fail("dictionary runtime unavailable");
    std::vector<UserLexiconEntry> staged;
    const auto read = read_rows(directory / "active.tsv", staged);
    if (!read.success) return read;
    IndexedLexicon phonetic;
    std::vector<bool> is_phonetic(staged.size());
    std::unordered_map<std::string, std::vector<std::vector<std::string>>> reading_cache;
    // Generate indices once per snapshot, not for each keystroke. Non-Chinese
    // expansions and shortcut-only codes remain exact aliases, not guessed
    // pronunciations. Do not borrow disabled base/user data to infer readings.
    try {
    for (std::size_t index = 0; index < staged.size(); ++index) {
        const auto& entry = staged[index];
        std::size_t characters = 0;
        bool chinese = true;
        for (std::size_t offset = 0; offset < entry.text.size();) {
            const auto lead = static_cast<unsigned char>(entry.text[offset]);
            const std::size_t bytes = lead < 0x80 ? 1 : lead < 0xe0 ? 2 : lead < 0xf0 ? 3 : 4;
            std::uint32_t value = lead & (bytes == 1 ? 0x7f : bytes == 2 ? 0x1f : bytes == 3 ? 0x0f : 0x07);
            if (offset + bytes > entry.text.size()) { chinese = false; break; }
            for (std::size_t i = 1; i < bytes; ++i) value = (value << 6) | (static_cast<unsigned char>(entry.text[offset + i]) & 0x3f);
            if (!((value >= 0x3400 && value <= 0x9fff) || (value >= 0xf900 && value <= 0xfaff) || (value >= 0x20000 && value <= 0x323af))) { chinese = false; break; }
            ++characters; offset += bytes;
        }
        if (!chinese || characters > entry.input.size()) continue;
        const auto cache_key = entry.input + '\t' + std::to_string(characters);
        auto cached = reading_cache.find(cache_key);
        std::vector<std::vector<std::string>> scratch;
        if (cached == reading_cache.end()) {
            scratch = FullPinyinSchema::dictionary_readings(entry.input, characters);
            if (reading_cache.size() < 4096) cached = reading_cache.emplace(cache_key, std::move(scratch)).first;
        }
        const auto& readings = cached == reading_cache.end() ? scratch : cached->second;
        for (const auto& reading : readings) phonetic.add(reading, entry.text, entry.count);
        is_phonetic[index] = !readings.empty();
    }
    phonetic.finish();
    } catch (const std::length_error&) {
        return fail("imported phonetic index exceeds limits");
    } catch (const std::bad_alloc&) {
        return fail("insufficient memory for imported phonetic index");
    }
    // Keep only shortcuts in the UserFrequencyModel surface. Re-injecting
    // normal words there would lose syllables, bypass correction/boundary
    // validation and incorrectly mark a dictionary phrase as a learned alias.
    std::vector<UserLexiconEntry> aliases;
    for (std::size_t index = 0; index < staged.size(); ++index) {
        if (is_phonetic[index]) continue;
        staged[index].input = compact(staged[index].input); aliases.push_back(std::move(staged[index]));
    }
    std::sort(aliases.begin(), aliases.end(), [](const auto& a, const auto& b) {
        if (a.input != b.input) return a.input < b.input;
        if (a.count != b.count) return a.count > b.count;
        return a.text < b.text;
    });
    // Both files become visible together. Never overwrite a mapped runtime;
    // a concurrent compiler may publish the same immutable snapshot first.
    static std::atomic<std::uint64_t> sequence{};
    const auto temporary = directory / ("runtime-v1-" + std::to_string(
        std::chrono::steady_clock::now().time_since_epoch().count()) + "-" + std::to_string(++sequence) + ".tmp");
    if (!std::filesystem::create_directory(temporary, error) || error) return fail("cannot stage dictionary runtime");
    struct Cleanup {
        std::filesystem::path directory;
        ~Cleanup() {
            std::error_code ignored;
            std::filesystem::remove(directory / "phonetic.owolx", ignored);
            std::filesystem::remove(directory / "aliases.tsv", ignored);
            std::filesystem::remove(directory, ignored);
        }
    } cleanup{temporary};
    const auto written = write_binary_lexicon(temporary / "phonetic.owolx", phonetic.materialize_entries(), LexiconIndexOrder::Imported);
    if (!written.success) return written;
    {
        std::ofstream file(temporary / "aliases.tsv", std::ios::binary);
        file << "OWO_DICTIONARIES_V1\n";
        for (const auto& entry : aliases) file << entry.input << '\t' << entry.text << '\t' << entry.count << '\n';
        file.flush();
        if (!file) return fail("cannot write dictionary aliases");
    }
    // Validate the actual persisted payload before it can be published.
    { BinaryLexicon check; const auto loaded = check.load(temporary / "phonetic.owolx"); if (!loaded.success) return loaded; }
    std::filesystem::rename(temporary, destination, error);
    if (error && !safe_directory(destination)) return fail("cannot publish dictionary runtime");
    return {true, {}};
}
namespace {
ManagedDictionarySnapshot load_snapshot(const std::filesystem::path& root, const std::string& id) {
    const auto started = std::chrono::steady_clock::now();
    ManagedDictionarySnapshot result;
    const auto fail = [&](std::string error) { result.error = std::move(error); result.phonetic.reset(); result.aliases.clear(); return result; };
    if (id.empty()) return result;
    if (id.size() != 32 || id.find_first_not_of("0123456789abcdef") != std::string::npos)
        return fail("invalid dictionary snapshot id");
    const auto directory = root / "snapshots" / id;
    if (!safe_directory(root) || !safe_directory(root / "snapshots") || !safe_directory(directory))
        return fail("linked dictionary directory rejected");
    try {
        const auto sources_path = directory / "sources.tsv";
        std::error_code error;
        const bool has_sources = std::filesystem::exists(sources_path, error);
        if (error) return fail("dictionary sources metadata unavailable");
        if (has_sources) {
            if (!safe_file(sources_path, 128)) return fail("invalid dictionary sources file");
            std::ifstream sources(sources_path, std::ios::binary);
            std::string header, base, user, extra;
            if (!std::getline(sources, header) || header != "OWO_DICTIONARY_SOURCES_V1" ||
                !std::getline(sources, base) || (base != "base\t0" && base != "base\t1") ||
                !std::getline(sources, user) || (user != "user\t0" && user != "user\t1") ||
                std::getline(sources, extra) || !sources.eof()) return fail("invalid dictionary sources metadata");
            result.base_enabled = base.back() == '1'; result.user_enabled = user.back() == '1';
        }
        // Legacy TSV snapshots are upgraded on this worker, never on a key
        // request. The ready-directory rename also makes subsequent starts cheap.
        const auto runtime = directory / "runtime-v1";
        if (!std::filesystem::exists(runtime, error)) {
            if (error) return fail("dictionary runtime unavailable");
            const auto prepared = prepare_managed_dictionary_snapshot(directory);
            if (!prepared.success) return fail(prepared.error);
            result.migrated = true;
        }
        if (!safe_directory(runtime) || !safe_file(runtime / "phonetic.owolx", 1024ULL * 1024 * 1024))
            return fail("invalid compiled dictionary file");
        result.phonetic = std::make_shared<BinaryLexicon>();
        result.phonetic->set_mixed_lookup_work_limit(16384);
        const auto loaded = result.phonetic->load(runtime / "phonetic.owolx");
        if (!loaded.success) return fail(loaded.error);
        const auto aliases = read_rows(runtime / "aliases.tsv", result.aliases);
        if (!aliases.success) return fail(aliases.error);
        if (!std::is_sorted(result.aliases.begin(), result.aliases.end(), [](const auto& a, const auto& b) {
            if (a.input != b.input) return a.input < b.input;
            if (a.count != b.count) return a.count > b.count;
            return a.text < b.text;
        })) return fail("dictionary aliases are not sorted");
        result.prepare_us = std::chrono::duration_cast<std::chrono::microseconds>(std::chrono::steady_clock::now() - started).count();
        return result;
    } catch (const std::exception&) { return fail("dictionary runtime preparation failed"); }
}
}
bool ManagedDictionaryModel::apply(ManagedDictionarySnapshot snapshot) {
    diagnostic_ = std::move(snapshot.error);
    if (diagnostic_.empty()) {
        base_enabled_ = snapshot.base_enabled; user_enabled_ = snapshot.user_enabled;
    }
    entries_ = std::move(snapshot.aliases); phonetic_ = std::move(snapshot.phonetic);
    std::clog << "{\"process\":\"core_service\",\"module\":\"dictionary\",\"event_id\":\"dictionary_runtime_ready\",\"prepare_us\":"
        << snapshot.prepare_us << ",\"migrated\":" << (snapshot.migrated ? "true" : "false")
        << ",\"phonetic_entries\":" << (phonetic_ ? phonetic_->size() : 0)
        << ",\"alias_entries\":" << entries_.size() << ",\"success\":" << (diagnostic_.empty() ? "true" : "false") << "}\n";
    return true;
}
bool ManagedDictionaryModel::refresh(bool force) {
    const auto now = std::chrono::steady_clock::now();
    const bool ready = pending_.valid() && pending_.wait_for(std::chrono::seconds(0)) == std::future_status::ready;
    if (root_.empty() || (!force && !ready && now < next_check_)) return false;
    next_check_ = now + std::chrono::milliseconds(250);
    std::string id;
    if (safe_directory(root_) && safe_file(root_ / "current", 33)) {
        std::ifstream pointer(root_ / "current", std::ios::binary);
        char bytes[34]{}; pointer.read(bytes, 33); id.assign(bytes, static_cast<std::size_t>(pointer.gcount()));
        if (id.empty()) id = "invalid";
    } else {
        std::error_code error;
        if (std::filesystem::exists(root_ / "current", error) || error) id = "invalid";
    }
    // Force is an explicit offline/test barrier, never used by the Core's
    // candidate loop. Ordinary refresh has at most one background preparation.
    if (force) {
        if (pending_.valid()) { pending_.wait(); pending_.get(); }
        const bool changed = id != snapshot_ || !id.empty() || !entries_.empty() || phonetic_;
        snapshot_ = id;
        apply(load_snapshot(root_, id)); return changed;
    }
    bool changed = false;
    if (ready) {
        auto prepared = pending_.get();
        if (pending_id_ == id) { changed = apply(std::move(prepared)); snapshot_ = id; }
    }
    if (!pending_.valid() && id != snapshot_) {
        pending_id_ = id;
        try {
        pending_ = std::async(std::launch::async, [root = root_, id] {
#ifdef _WIN32
            // Lower CPU competition without lowering the memory/IO priority
            // of mapped pages that the foreground candidate thread will use.
            const auto previous_priority = GetThreadPriority(GetCurrentThread());
            const bool background = previous_priority != THREAD_PRIORITY_ERROR_RETURN &&
                SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) != FALSE;
            auto prepared = load_snapshot(root, id);
            if (background) SetThreadPriority(GetCurrentThread(), previous_priority);
            return prepared;
#else
            return load_snapshot(root, id);
#endif
        });
        } catch (const std::exception&) {
            // A low-memory machine must not crash Core if a worker cannot be
            // created. Preserve the last good snapshot and retry next poll.
            diagnostic_ = "cannot schedule dictionary preparation";
        }
    }
    return changed;
}
std::int64_t ManagedDictionaryModel::score(std::string_view text) const { return learned_ && user_enabled_ ? learned_->score(text) : 0; }
bool ManagedDictionaryModel::preferred(std::string_view input, std::string_view text) const { return learned_ && user_enabled_ && learned_->preferred(input, text); }
std::int64_t ManagedDictionaryModel::contextual_score(std::string_view input, std::string_view text) const { return learned_ && user_enabled_ ? learned_->contextual_score(input, text) : 0; }
std::int64_t ManagedDictionaryModel::language_context_score(std::string_view context, std::string_view input, std::string_view text) const {
    return learned_ && user_enabled_ ? learned_->language_context_score(context, input, text) : 0;
}
std::vector<UserLexiconEntry> ManagedDictionaryModel::lookup(std::string_view input, std::size_t limit) const { return combined(input, limit, false); }
std::vector<UserLexiconEntry> ManagedDictionaryModel::lookup_prefix(std::string_view input, std::size_t limit) const { return combined(input, limit, true); }
std::vector<UserLexiconEntry> ManagedDictionaryModel::lookup_related(std::string_view input, std::size_t limit) const {
    return learned_ && user_enabled_ ? learned_->lookup_related(input, limit) : std::vector<UserLexiconEntry>{};
}
std::vector<UserLexiconEntry> ManagedDictionaryModel::lookup_explicit(std::string_view input, std::size_t limit) const {
    std::vector<UserLexiconEntry> result;
    if (!enabled_ || !limit) return result;
    const auto key = compact(input);
    auto begin = std::lower_bound(entries_.begin(), entries_.end(), key, [](const auto& entry, const auto& value) { return entry.input < value; });
    for (auto it = begin; it != entries_.end() && it->input == key && result.size() < limit; ++it) result.push_back(*it);
    return result;
}
std::vector<UserLexiconEntry> ManagedDictionaryModel::combined(std::string_view input, std::size_t limit, bool prefix) const {
    auto result = learned_ && user_enabled_ ? (prefix ? learned_->lookup_prefix(input, limit) : learned_->lookup(input, limit)) : std::vector<UserLexiconEntry>{};
    const auto key = compact(input);
    if (!enabled_ || key.empty() || !limit || (prefix && key.size() < 2)) return result;
    auto begin = std::lower_bound(entries_.begin(), entries_.end(), key, [](const auto& entry, const auto& value) { return entry.input < value; });
    std::size_t work = 0;
    std::set<std::pair<std::string, std::string>> seen;
    for (const auto& entry : result) seen.emplace(entry.input, entry.text);
    for (auto it = begin; it != entries_.end() && work++ < 2048; ++it) {
        if (prefix ? !std::string_view(it->input).starts_with(key) : it->input != key) break;
        if (prefix && it->input.size() <= key.size()) continue;
        if (seen.emplace(it->input, it->text).second) result.push_back(*it);
        if (!prefix && result.size() >= limit) break;
    }
    std::stable_sort(result.begin(), result.end(), [prefix](const auto& a, const auto& b) {
        if (prefix && a.input.size() != b.input.size()) return a.input.size() < b.input.size();
        if (a.preferred != b.preferred) return a.preferred;
        if (a.count != b.count) return a.count > b.count;
        if (a.input != b.input) return a.input < b.input;
        return a.text < b.text;
    });
    if (result.size() > limit) result.resize(limit);
    return result;
}
}

#include "owo/engine/managed_dictionary.h"
#include <algorithm>
#include <charconv>
#include <fstream>
#include <set>

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
    return !error && std::filesystem::is_directory(status) && !std::filesystem::is_symlink(status);
}
}
bool ManagedDictionaryModel::refresh(bool force) {
    const auto now = std::chrono::steady_clock::now();
    if (root_.empty() || (!force && now < next_check_)) return false;
    next_check_ = now + std::chrono::milliseconds(250);
    std::ifstream pointer(root_ / "current", std::ios::binary);
    std::string id;
    if (pointer) { char bytes[34]{}; pointer.read(bytes, 33); id.assign(bytes, static_cast<std::size_t>(pointer.gcount())); }
    pointer.close();
    if (id == snapshot_ && !force) return false;
    const auto previous = snapshot_;
    snapshot_ = id;
    auto fail = [&](std::string message) {
        const bool changed = !entries_.empty() || previous != id;
        entries_.clear(); diagnostic_ = std::move(message); return changed;
    };
    if (id.empty()) { base_enabled_ = user_enabled_ = true; return fail({}); }
    if (id.size() != 32 || id.find_first_not_of("0123456789abcdef") != std::string::npos)
        return fail("invalid dictionary snapshot id");
    const auto directory = root_ / "snapshots" / id;
    if (!safe_directory(root_) || !safe_directory(root_ / "snapshots") || !safe_directory(directory))
        return fail("linked dictionary directory rejected");
    const auto path = directory / "active.tsv";
    std::error_code error;
    const auto length = std::filesystem::file_size(path, error);
    if (error || length > 24 * 1024 * 1024 || std::filesystem::is_symlink(std::filesystem::symlink_status(path, error)))
        return fail("dictionary snapshot is missing, linked or too large");
    std::ifstream file(path, std::ios::binary);
    std::string line;
    if (!std::getline(file, line) || line != "OWO_DICTIONARIES_V1") return fail("dictionary snapshot header is invalid");
    std::vector<UserLexiconEntry> staged;
    while (std::getline(file, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        const auto a = line.find('\t'), b = a == std::string::npos ? a : line.find('\t', a + 1);
        if (a == std::string::npos || b == std::string::npos || line.find('\t', b + 1) != std::string::npos || staged.size() >= 100000)
            return fail("dictionary snapshot row is invalid or exceeds limits");
        std::uint32_t weight{};
        const auto parsed = std::from_chars(line.data() + b + 1, line.data() + line.size(), weight);
        UserLexiconEntry entry{line.substr(a + 1, b - a - 1), weight, compact(line.substr(0, a)), false};
        if (parsed.ec != std::errc{} || parsed.ptr != line.data() + line.size() || !UserFrequencyStore::validate_word(entry).empty())
            return fail("dictionary snapshot entry failed validation");
        staged.push_back(std::move(entry));
    }
    if (!file.eof()) return fail("dictionary snapshot read failed");
    bool base_enabled = true, user_enabled = true;
    const auto sources_path = directory / "sources.tsv";
    const bool has_sources = std::filesystem::exists(sources_path, error);
    if (error) return fail("dictionary sources metadata unavailable");
    if (has_sources) {
        const auto size = std::filesystem::file_size(sources_path, error);
        if (error || size > 128 || std::filesystem::is_symlink(std::filesystem::symlink_status(sources_path, error)))
            return fail("dictionary sources metadata is linked or too large");
        std::ifstream sources(sources_path, std::ios::binary);
        std::string header, base, user, extra;
        if (!std::getline(sources, header) || header != "OWO_DICTIONARY_SOURCES_V1" ||
            !std::getline(sources, base) || (base != "base\t0" && base != "base\t1") ||
            !std::getline(sources, user) || (user != "user\t0" && user != "user\t1") ||
            std::getline(sources, extra) || !sources.eof()) return fail("invalid dictionary sources metadata");
        base_enabled = base.back() == '1'; user_enabled = user.back() == '1';
    }
    std::sort(staged.begin(), staged.end(), [](const auto& a, const auto& b) {
        if (a.input != b.input) return a.input < b.input;
        if (a.count != b.count) return a.count > b.count;
        return a.text < b.text;
    });
    entries_ = std::move(staged); base_enabled_ = base_enabled; user_enabled_ = user_enabled;
    diagnostic_.clear(); return true;
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

#include "owo/engine/tone_feedback.h"

#ifdef _WIN32
#include <Windows.h>
#include <dpapi.h>
#endif

#include <algorithm>
#include <array>
#include <fstream>
#include <iterator>
#include <unordered_map>
#include <utility>
#include <vector>

namespace owo::engine {
namespace {

constexpr std::array<unsigned char, 4> kMagic{'O', 'W', 'T', 'F'};
constexpr std::uint32_t kVersion = 1;
constexpr std::uint32_t kPlainEncoding = 0;
constexpr std::uint32_t kDpapiEncoding = 1;
constexpr std::size_t kHeaderSize = 16;
constexpr std::size_t kMaximumFileBytes = 8U * 1024U * 1024U;
constexpr std::uint32_t kDecayNumerator = 3;
constexpr std::uint32_t kDecayDenominator = 4;
constexpr std::uint32_t kMaximumToneFeedbackKinds = 4;

void append_u32(std::vector<unsigned char>& out, const std::uint32_t value) {
    for (unsigned shift = 0; shift < 32; shift += 8)
        out.push_back(static_cast<unsigned char>(value >> shift));
}
void append_u64(std::vector<unsigned char>& out, const std::uint64_t value) {
    for (unsigned shift = 0; shift < 64; shift += 8)
        out.push_back(static_cast<unsigned char>(value >> shift));
}
void append_bytes(std::vector<unsigned char>& out, const std::string_view value) {
    append_u32(out, static_cast<std::uint32_t>(value.size()));
    out.insert(out.end(), value.begin(), value.end());
}
std::uint64_t checksum(const std::span<const unsigned char> bytes) {
    std::uint64_t value = 14695981039346656037ULL;
    for (const auto byte : bytes) { value ^= byte; value *= 1099511628211ULL; }
    return value;
}
bool read_u32(const std::span<const unsigned char> bytes, std::size_t& offset,
              std::uint32_t& value) {
    if (offset + 4 > bytes.size()) return false;
    value = 0;
    for (unsigned index = 0; index < 4; ++index)
        value |= static_cast<std::uint32_t>(bytes[offset + index]) << (index * 8U);
    offset += 4;
    return true;
}
bool read_u64(const std::span<const unsigned char> bytes, std::size_t& offset,
              std::uint64_t& value) {
    if (offset + 8 > bytes.size()) return false;
    value = 0;
    for (unsigned index = 0; index < 8; ++index)
        value |= static_cast<std::uint64_t>(bytes[offset + index]) << (index * 8U);
    offset += 8;
    return true;
}
bool read_bytes(const std::span<const unsigned char> bytes, std::size_t& offset,
                const std::size_t maximum, std::string& value) {
    std::uint32_t size{};
    if (!read_u32(bytes, offset, size) || size > maximum || offset + size > bytes.size())
        return false;
    value.assign(reinterpret_cast<const char*>(bytes.data() + offset), size);
    offset += size;
    return true;
}

bool valid_utf8(const std::string_view text) noexcept {
    std::size_t index = 0;
    while (index < text.size()) {
        const auto lead = static_cast<unsigned char>(text[index]);
        std::size_t length = 0;
        std::uint32_t scalar = 0;
        if (lead < 0x80U) { ++index; continue; }
        if ((lead & 0xe0U) == 0xc0U) { length = 2; scalar = lead & 0x1fU; }
        else if ((lead & 0xf0U) == 0xe0U) { length = 3; scalar = lead & 0x0fU; }
        else if ((lead & 0xf8U) == 0xf0U) { length = 4; scalar = lead & 0x07U; }
        else return false;
        if (index + length > text.size()) return false;
        for (std::size_t offset = 1; offset < length; ++offset) {
            const auto next = static_cast<unsigned char>(text[index + offset]);
            if ((next & 0xc0U) != 0x80U) return false;
            scalar = (scalar << 6U) | (next & 0x3fU);
        }
        if ((length == 2 && scalar < 0x80U) || (length == 3 && scalar < 0x800U) ||
            (length == 4 && (scalar < 0x10000U || scalar > 0x10ffffU)) ||
            (scalar >= 0xd800U && scalar <= 0xdfffU)) return false;
        index += length;
    }
    return true;
}

std::string_view trim(const std::string_view text) noexcept {
    const auto blank = [](const char value) {
        return value == ' ' || value == '\t' || value == '\r' || value == '\n';
    };
    std::string_view result = text;
    while (!result.empty() && blank(result.front())) result.remove_prefix(1);
    while (!result.empty() && blank(result.back())) result.remove_suffix(1);
    return result;
}

std::vector<std::string> code_points(const std::string_view text) {
    std::vector<std::string> points;
    points.reserve(text.size());
    for (std::size_t index = 0; index < text.size();) {
        const auto lead = static_cast<unsigned char>(text[index]);
        std::size_t size = 1;
        if ((lead & 0xe0U) == 0xc0U) size = 2;
        else if ((lead & 0xf0U) == 0xe0U) size = 3;
        else if ((lead & 0xf8U) == 0xf0U) size = 4;
        size = (std::min)(size, text.size() - index);
        points.emplace_back(text.substr(index, size));
        index += size;
    }
    return points;
}

std::size_t code_point_count(const std::string_view text) noexcept {
    return static_cast<std::size_t>(std::count_if(
        text.begin(), text.end(), [](const char value) {
            return (static_cast<unsigned char>(value) & 0xc0U) != 0x80U;
        }));
}

std::string join(const std::vector<std::string>& points, const std::size_t begin,
                 const std::size_t end) {
    std::string result;
    for (std::size_t index = begin; index < end; ++index) result += points[index];
    return result;
}

std::vector<unsigned char> encode(const std::uint64_t next_id,
                                  const std::vector<ToneFeedbackEntry>& entries) {
    std::vector<unsigned char> payload;
    append_u64(payload, next_id);
    append_u32(payload, static_cast<std::uint32_t>(entries.size()));
    for (const auto& entry : entries) {
        append_u64(payload, entry.id);
        append_u64(payload, entry.created_at_ms);
        append_u32(payload, static_cast<std::uint32_t>(entry.kind));
        append_bytes(payload, entry.application);
        append_bytes(payload, entry.draft);
        append_bytes(payload, entry.final_text);
    }
    std::vector<unsigned char> plain;
    plain.reserve(payload.size() + 8);
    append_u64(plain, checksum(payload));
    plain.insert(plain.end(), payload.begin(), payload.end());
    return plain;
}

#ifdef _WIN32
bool dpapi(const std::vector<unsigned char>& input, std::vector<unsigned char>& output,
           const bool protect) {
    DATA_BLOB in{static_cast<DWORD>(input.size()), const_cast<BYTE*>(input.data())};
    DATA_BLOB out{};
    static constexpr wchar_t kDescription[] = L"OwO tone feedback";
    const BOOL ok = protect
        ? CryptProtectData(&in, kDescription, nullptr, nullptr, nullptr,
                           CRYPTPROTECT_UI_FORBIDDEN, &out)
        : CryptUnprotectData(&in, nullptr, nullptr, nullptr, nullptr,
                             CRYPTPROTECT_UI_FORBIDDEN, &out);
    if (!ok) return false;
    output.assign(out.pbData, out.pbData + out.cbData);
    SecureZeroMemory(out.pbData, out.cbData);
    LocalFree(out.pbData);
    return true;
}
#endif

struct Decoded {
    std::uint64_t next_id{1};
    std::vector<ToneFeedbackEntry> entries;
};

enum class DecodeStatus { ok, missing, invalid };

DecodeStatus decode(const std::filesystem::path& path, Decoded& output) {
    std::error_code error;
    if (!std::filesystem::exists(path, error)) return DecodeStatus::missing;
    const auto size = std::filesystem::file_size(path, error);
    if (error || size < kHeaderSize || size > kMaximumFileBytes) return DecodeStatus::invalid;
    std::vector<unsigned char> bytes(static_cast<std::size_t>(size));
    {
        std::ifstream input(path, std::ios::binary);
        if (!input.read(reinterpret_cast<char*>(bytes.data()),
                        static_cast<std::streamsize>(bytes.size())))
            return DecodeStatus::invalid;
    }
    const std::span<const unsigned char> all(bytes);
    if (!std::equal(kMagic.begin(), kMagic.end(), bytes.begin())) return DecodeStatus::invalid;
    std::size_t offset = 4;
    std::uint32_t version{}, encoding{}, length{};
    if (!read_u32(all, offset, version) || version != kVersion ||
        !read_u32(all, offset, encoding) || !read_u32(all, offset, length) ||
        offset + length != bytes.size()) return DecodeStatus::invalid;
    std::vector<unsigned char> plain(bytes.begin() + static_cast<std::ptrdiff_t>(offset),
                                     bytes.end());
    if (encoding == kDpapiEncoding) {
#ifdef _WIN32
        std::vector<unsigned char> decrypted;
        if (!dpapi(plain, decrypted, false)) return DecodeStatus::invalid;
        plain = std::move(decrypted);
#else
        return DecodeStatus::invalid;
#endif
    } else if (encoding != kPlainEncoding) {
        return DecodeStatus::invalid;
    }
    const std::span<const unsigned char> view(plain);
    std::size_t cursor = 0;
    std::uint64_t expected{};
    if (!read_u64(view, cursor, expected) || checksum(view.subspan(cursor)) != expected)
        return DecodeStatus::invalid;
    Decoded result;
    std::uint32_t count{};
    if (!read_u64(view, cursor, result.next_id) || result.next_id == 0 ||
        !read_u32(view, cursor, count) || count > kMaximumToneFeedbackEntries)
        return DecodeStatus::invalid;
    result.entries.reserve(count);
    std::uint64_t previous_id = 0;
    for (std::uint32_t index = 0; index < count; ++index) {
        ToneFeedbackEntry entry;
        std::uint64_t id{};
        std::uint32_t kind{};
        if (!read_u64(view, cursor, id) || id <= previous_id || id >= result.next_id ||
            !read_u64(view, cursor, entry.created_at_ms) ||
            !read_u32(view, cursor, kind) || kind >= kMaximumToneFeedbackKinds)
            return DecodeStatus::invalid;
        entry.id = id;
        entry.kind = static_cast<ToneFeedbackKind>(kind);
        if (!read_bytes(view, cursor, kMaximumUserCorpusApplicationBytes, entry.application) ||
            !read_bytes(view, cursor, kMaximumToneFeedbackTextBytes, entry.draft) ||
            !read_bytes(view, cursor, kMaximumToneFeedbackTextBytes, entry.final_text))
            return DecodeStatus::invalid;
        if (entry.draft.empty() || !valid_utf8(entry.draft) ||
            !valid_utf8(entry.application) || !valid_utf8(entry.final_text))
            return DecodeStatus::invalid;
        previous_id = id;
        result.entries.push_back(std::move(entry));
    }
    if (cursor != view.size()) return DecodeStatus::invalid;
    output = std::move(result);
    return DecodeStatus::ok;
}

bool write_file(const std::filesystem::path& path,
                const std::vector<unsigned char>& bytes) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    if (!output) return false;
    output.write(reinterpret_cast<const char*>(bytes.data()),
                 static_cast<std::streamsize>(bytes.size()));
    output.flush();
    return static_cast<bool>(output);
}

#ifdef _WIN32
class FeedbackLock final {
public:
    FeedbackLock()
        : mutex_(CreateMutexW(nullptr, FALSE, L"Local\\OwO.InputMethod.ToneFeedback.v1")) {
        if (mutex_ == nullptr) return;
        const auto waited = WaitForSingleObject(mutex_, 5000);
        locked_ = waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED;
    }
    ~FeedbackLock() {
        if (locked_) ReleaseMutex(mutex_);
        if (mutex_ != nullptr) CloseHandle(mutex_);
    }
    FeedbackLock(const FeedbackLock&) = delete;
    FeedbackLock& operator=(const FeedbackLock&) = delete;
    explicit operator bool() const noexcept { return locked_; }
private:
    HANDLE mutex_{};
    bool locked_{};
};
#else
class FeedbackLock final {
public:
    explicit operator bool() const noexcept { return true; }
};
#endif

struct Relation {
    std::string from;
    std::string to;
    int delta{};
};

// A relation worth learning from one record, or nothing when the text is too
// long to name a phrasing. Rejected and undone drafts are the phrasing to
// avoid; adopted ones reinforce what worked. A whole sentence is too specific
// to reappear in a later draft, so it only counts as exposure: the record is
// kept, the style does not move.
bool relation_for(const ToneFeedbackEntry& entry, Relation& relation) {
    const auto draft = trim(entry.draft);
    if (entry.kind == ToneFeedbackKind::edited) {
        std::string from, to;
        if (!tone_feedback_correction(draft, trim(entry.final_text), from, to)) return false;
        relation = {std::move(from), std::move(to), 2};
        return true;
    }
    if (code_point_count(draft) > kMaximumToneHintCodes) return false;
    if (entry.kind == ToneFeedbackKind::adopted) {
        relation = {std::string(draft), {}, 1};
        return true;
    }
    relation = {std::string(draft), {},
                entry.kind == ToneFeedbackKind::undone ? -2 : -1};
    return true;
}

void decay_weights(std::vector<TonePhrasePreference>& preferences) {
    for (auto& preference : preferences) {
        const auto magnitude = preference.weight < 0 ? -preference.weight : preference.weight;
        const auto reduced = static_cast<int>(static_cast<std::uint64_t>(magnitude) *
                                              kDecayNumerator / kDecayDenominator);
        preference.weight = preference.weight < 0 ? -reduced : reduced;
    }
    std::erase_if(preferences, [](const TonePhrasePreference& value) {
        return value.weight == 0;
    });
}

// Scans records oldest first and ages the accumulated weights when the
// magnitude bound is reached, so no single phrasing accumulates forever.
std::vector<TonePhrasePreference> accumulate(
    const std::vector<const ToneFeedbackEntry*>& ordered) {
    std::vector<TonePhrasePreference> preferences;
    const auto find = [&preferences](const std::string& from, const std::string& to) {
        return std::find_if(preferences.begin(), preferences.end(),
                            [&](const TonePhrasePreference& value) {
            return value.from == from && value.to == to;
        });
    };
    for (const auto* entry : ordered) {
        Relation relation;
        if (!relation_for(*entry, relation)) continue;
        auto found = find(relation.from, relation.to);
        if (found == preferences.end()) {
            if (preferences.size() >= kMaximumToneFeedbackPreferences * 4)
                decay_weights(preferences);
            TonePhrasePreference value;
            value.from = relation.from;
            value.to = relation.to;
            value.observations = 1;
            value.newest_id = entry->id;
            value.weight = relation.delta;
            preferences.push_back(std::move(value));
            continue;
        }
        if (found->weight + relation.delta > kMaximumTonePreferenceWeight ||
            found->weight + relation.delta < -kMaximumTonePreferenceWeight) {
            decay_weights(preferences);
            found = find(relation.from, relation.to);
            if (found == preferences.end()) continue;
        }
        found->weight = (std::max)(-kMaximumTonePreferenceWeight,
                                   (std::min)(kMaximumTonePreferenceWeight,
                                              found->weight + relation.delta));
        ++found->observations;
        found->newest_id = entry->id;
    }
    std::erase_if(preferences, [](const TonePhrasePreference& value) {
        return value.weight == 0;
    });
    return preferences;
}

}  // namespace

std::string_view tone_feedback_kind_id(const ToneFeedbackKind kind) noexcept {
    switch (kind) {
    case ToneFeedbackKind::adopted: return "adopted";
    case ToneFeedbackKind::edited: return "edited";
    case ToneFeedbackKind::rejected: return "rejected";
    case ToneFeedbackKind::undone: return "undone";
    }
    return "unknown";
}

bool parse_tone_feedback_kind(const std::string_view id,
                              ToneFeedbackKind& kind) noexcept {
    if (id == "adopted") { kind = ToneFeedbackKind::adopted; return true; }
    if (id == "edited") { kind = ToneFeedbackKind::edited; return true; }
    if (id == "rejected") { kind = ToneFeedbackKind::rejected; return true; }
    if (id == "undone") { kind = ToneFeedbackKind::undone; return true; }
    return false;
}

ToneFeedbackIoResult ToneFeedbackStore::load(const std::filesystem::path& path) {
    path_ = path;
    next_id_ = 1;
    entries_.clear();
    Decoded decoded;
    const auto status = decode(path_, decoded);
    auto backup = path_; backup += L".bak";
    if (status == DecodeStatus::ok) {
        next_id_ = decoded.next_id;
        entries_ = std::move(decoded.entries);
        return {true, false, {}};
    }
    const auto backup_status = decode(backup, decoded);
    if (backup_status == DecodeStatus::ok) {
        next_id_ = decoded.next_id;
        entries_ = std::move(decoded.entries);
        return {true, true, {}};
    }
    if (status == DecodeStatus::missing && backup_status == DecodeStatus::missing)
        return {true, false, {}};
    return {false, false, "tone feedback and backup are invalid"};
}

ToneFeedbackIoResult ToneFeedbackStore::save() const {
    if (path_.empty()) return {false, false, "tone feedback path is not set"};
    auto plain = encode(next_id_, entries_);
    std::uint32_t encoding = kPlainEncoding;
#ifdef _WIN32
    std::vector<unsigned char> protected_bytes;
    const bool encrypted = dpapi(plain, protected_bytes, true);
    SecureZeroMemory(plain.data(), plain.size());
    if (!encrypted) return {false, false, "cannot encrypt tone feedback"};
    plain = std::move(protected_bytes);
    encoding = kDpapiEncoding;
#endif
    std::vector<unsigned char> bytes(kMagic.begin(), kMagic.end());
    append_u32(bytes, kVersion);
    append_u32(bytes, encoding);
    append_u32(bytes, static_cast<std::uint32_t>(plain.size()));
    bytes.insert(bytes.end(), plain.begin(), plain.end());
    if (bytes.size() > kMaximumFileBytes)
        return {false, false, "tone feedback exceeds size limit"};
    std::error_code error;
    if (!path_.parent_path().empty())
        std::filesystem::create_directories(path_.parent_path(), error);
    auto temporary = path_; temporary += L".tmp";
    auto backup = path_; backup += L".bak";
    if (!write_file(temporary, bytes)) {
        std::filesystem::remove(temporary, error);
        return {false, false, "cannot write temporary tone feedback"};
    }
#ifdef _WIN32
    Decoded previous;
    if (decode(path_, previous) == DecodeStatus::ok &&
        !CopyFileW(path_.c_str(), backup.c_str(), FALSE))
        return {false, false, "cannot update tone feedback backup"};
    if (!MoveFileExW(temporary.c_str(), path_.c_str(),
                     MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
        return {false, false, "cannot atomically replace tone feedback"};
#else
    if (std::filesystem::exists(path_))
        std::filesystem::copy_file(path_, backup,
                                   std::filesystem::copy_options::overwrite_existing, error);
    std::filesystem::rename(temporary, path_, error);
    if (error) return {false, false, "cannot replace tone feedback"};
#endif
    return {true, false, {}};
}

std::uint64_t ToneFeedbackStore::append(const ToneFeedbackKind kind,
                                        const std::string_view application,
                                        const std::string_view draft,
                                        const std::string_view final_text,
                                        const std::uint64_t created_at_ms) {
    const auto trimmed_draft = trim(draft);
    const auto trimmed_final = trim(final_text);
    if (static_cast<unsigned>(kind) >= kMaximumToneFeedbackKinds ||
        trimmed_draft.empty() || trimmed_draft.size() > kMaximumToneFeedbackTextBytes ||
        trimmed_final.size() > kMaximumToneFeedbackTextBytes ||
        !valid_utf8(trimmed_draft) || !valid_utf8(trimmed_final) ||
        !valid_utf8(application) ||
        application.size() > kMaximumUserCorpusApplicationBytes) return 0;
    // The kind decides which text matters: an edit without its result says
    // nothing about what the user wanted.
    if (kind == ToneFeedbackKind::edited) {
        if (trimmed_final.empty() || trimmed_final == trimmed_draft) return 0;
    } else if (!trimmed_final.empty() && kind != ToneFeedbackKind::adopted) {
        return 0;
    }
    if (kind == ToneFeedbackKind::adopted && trimmed_final != trimmed_draft) return 0;
    if (entries_.size() >= kMaximumToneFeedbackEntries)
        entries_.erase(entries_.begin(), entries_.begin() + static_cast<std::ptrdiff_t>(
            entries_.size() - kMaximumToneFeedbackEntries + 1));
    const auto id = next_id_++;
    ToneFeedbackEntry entry;
    entry.id = id;
    entry.created_at_ms = created_at_ms;
    entry.kind = kind;
    entry.application = std::string(application);
    entry.draft = std::string(trimmed_draft);
    entry.final_text = std::string(trimmed_final);
    entries_.push_back(std::move(entry));
    return id;
}

std::uint64_t ToneFeedbackStore::undo(const std::uint64_t id,
    const std::string_view application, const std::string_view draft,
    const std::string_view final_text, const std::uint64_t created_at_ms) {
    // Match the same normalized identity that append persisted.
    const auto normalized_draft = trim(draft);
    const auto normalized_final = trim(final_text);
    const auto found = std::find_if(entries_.begin(), entries_.end(), [&](const auto& entry) {
        return entry.id == id && entry.application == application && entry.draft == normalized_draft &&
            entry.final_text == normalized_final && (entry.kind == ToneFeedbackKind::adopted ||
                                              entry.kind == ToneFeedbackKind::edited);
    });
    if (found == entries_.end()) return 0;
    // Copy before append can invalidate the vector. Validate the new entry
    // before erasing the original, so failure never loses recorded evidence.
    const auto text = found->final_text;
    const auto new_id = append(ToneFeedbackKind::undone, application, text, {}, created_at_ms);
    if (new_id) erase(id);
    return new_id;
}

bool ToneFeedbackStore::erase(const std::uint64_t id) {
    const auto found = std::find_if(entries_.begin(), entries_.end(),
                                    [id](const auto& entry) { return entry.id == id; });
    if (found == entries_.end()) return false;
    entries_.erase(found);
    return true;
}

std::size_t ToneFeedbackStore::clear_all() {
    const auto removed = entries_.size();
    entries_.clear();
    return removed;
}

std::size_t ToneFeedbackStore::count(const ToneFeedbackKind kind) const noexcept {
    return static_cast<std::size_t>(std::count_if(
        entries_.begin(), entries_.end(),
        [kind](const auto& entry) { return entry.kind == kind; }));
}

ToneFeedbackIoResult ToneFeedbackStore::modify(
    const std::filesystem::path& path,
    const std::function<void(ToneFeedbackStore&)>& change, const bool reset_invalid) {
    const FeedbackLock lock;
    if (!lock) return {false, false, "tone feedback is locked"};
    ToneFeedbackStore store;
    auto loaded = store.load(path);
    if (!loaded.success) {
        if (!reset_invalid) return loaded;
        store = ToneFeedbackStore{};
        store.path_ = path;
    }
    const auto previous = store.entries_;
    change(store);
    auto saved = store.save();
    // Deleted records must not be resurrected by backup recovery. Purge the
    // backup under the writer lock so a concurrent append cannot race it.
    const bool removed = reset_invalid || std::any_of(
        previous.begin(), previous.end(), [&store](const auto& entry) {
            return std::none_of(store.entries_.begin(), store.entries_.end(),
                                [&entry](const auto& kept) { return kept.id == entry.id; });
        });
    if (saved.success && removed) {
        auto backup = path; backup += L".bak";
        std::error_code error;
        std::filesystem::remove(backup, error);
        if (error) return {false, loaded.recovered_from_backup,
                           "cannot remove deleted feedback from backup"};
    }
    saved.recovered_from_backup = loaded.recovered_from_backup;
    return saved;
}

bool tone_feedback_correction(const std::string_view draft,
                              const std::string_view final_text, std::string& from,
                              std::string& to) {
    from.clear();
    to.clear();
    const auto left = code_points(trim(draft));
    const auto right = code_points(trim(final_text));
    if (left.empty() || right.empty()) return false;
    std::size_t prefix = 0;
    while (prefix < left.size() && prefix < right.size() && left[prefix] == right[prefix])
        ++prefix;
    std::size_t suffix = 0;
    while (suffix < left.size() - prefix && suffix < right.size() - prefix &&
           left[left.size() - 1 - suffix] == right[right.size() - 1 - suffix])
        ++suffix;
    auto left_end = left.size() - suffix;
    auto right_end = right.size() - suffix;
    if (left_end == prefix && right_end == prefix) return false;
    // A pure insertion or deletion names no replacement, so there is nothing
    // to learn about the user's wording.
    if (left_end == prefix || right_end == prefix) return false;
    // A one-character swap (的→呀) is too small to be a phrasing on its own,
    // so the neighbouring character is included: 好的→好呀. The hint then
    // names something a later draft could actually contain.
    const auto widen = prefix > 0 ? prefix - 1 : prefix;
    if ((left_end - prefix == 1 || right_end - prefix == 1) && widen != prefix) {
        prefix = widen;
    } else if ((left_end - prefix == 1 || right_end - prefix == 1) && suffix > 0) {
        left_end = (std::min)(left.size(), left_end + 1);
        right_end = (std::min)(right.size(), right_end + 1);
    }
    if (left_end - prefix > kMaximumToneCorrectionCodes ||
        right_end - prefix > kMaximumToneCorrectionCodes) return false;
    from = join(left, prefix, left_end);
    to = join(right, prefix, right_end);
    if (from.size() > kMaximumTonePhraseBytes || to.size() > kMaximumTonePhraseBytes)
        return false;
    return true;
}

ToneFeedbackProfile build_tone_feedback_profile(
    const std::span<const ToneFeedbackEntry> entries, const StyleScene scene,
    const std::uint32_t scene_mask) {
    ToneFeedbackProfile profile;
    if ((scene_mask & style_scene_bit(scene)) == 0) return profile;
    std::vector<const ToneFeedbackEntry*> ordered;
    ordered.reserve(entries.size());
    for (const auto& entry : entries) {
        if (style_scene_for_application(entry.application) != scene) continue;
        if (!valid_utf8(entry.draft) || !valid_utf8(entry.final_text)) continue;
        ordered.push_back(&entry);
    }
    std::sort(ordered.begin(), ordered.end(), [](const auto* left, const auto* right) {
        return left->id < right->id;
    });
    profile.entry_count = ordered.size();

    auto preferences = accumulate(ordered);
    std::sort(preferences.begin(), preferences.end(), [](const auto& left, const auto& right) {
        const auto left_magnitude = left.weight < 0 ? -left.weight : left.weight;
        const auto right_magnitude = right.weight < 0 ? -right.weight : right.weight;
        if (left_magnitude != right_magnitude) return left_magnitude > right_magnitude;
        if (left.newest_id != right.newest_id) return left.newest_id > right.newest_id;
        if (left.from != right.from) return left.from < right.from;
        return left.to < right.to;
    });
    if (preferences.size() > kMaximumToneFeedbackPreferences)
        preferences.resize(kMaximumToneFeedbackPreferences);
    profile.preferences = std::move(preferences);
    return profile;
}

std::vector<std::string> describe_tone_feedback_hints(
    const ToneFeedbackProfile& profile) {
    std::vector<std::string> hints;
    for (const auto& preference : profile.preferences) {
        if (hints.size() >= kMaximumTonePromptHints) break;
        if (preference.from.empty()) continue;
        if (code_point_count(preference.from) > kMaximumToneHintCodes) continue;
        if (!preference.to.empty()) {
            if (preference.weight <= 0) continue;
            if (code_point_count(preference.to) > kMaximumToneHintCodes) continue;
            hints.push_back("把“" + preference.from + "”写成“" + preference.to + "”");
        } else if (preference.weight > 0) {
            hints.push_back("用户自己常这样写：“" + preference.from + "”");
        } else {
            hints.push_back("避免类似“" + preference.from + "”的说法");
        }
    }
    return hints;
}

int tone_feedback_adjustment(const std::string_view draft,
                             const ToneFeedbackProfile& profile) {
    int adjustment = 0;
    for (const auto& preference : profile.preferences) {
        if (preference.from.empty()) continue;
        if (preference.to.empty()) {
            if (draft.find(preference.from) != std::string_view::npos)
                adjustment += preference.weight;
            continue;
        }
        // A correction says "write `to`, not `from`". Seeing `from` fails to
        // follow it; seeing `to` follows it.
        if (draft.find(preference.from) != std::string_view::npos) {
            adjustment -= preference.weight;
            continue;
        }
        if (draft.find(preference.to) != std::string_view::npos)
            adjustment += preference.weight;
    }
    return adjustment;
}

ToneDraftSelection apply_tone_feedback(ToneDraftSelection selection,
                                       const ToneFeedbackProfile& profile) {
    if (profile.preferences.empty()) return selection;
    // Stable: equal adjustments keep the model's order.
    std::stable_sort(selection.candidates.begin(), selection.candidates.end(),
                     [&profile](const ToneDraftCandidate& left,
                                const ToneDraftCandidate& right) {
        return tone_feedback_adjustment(left.text, profile) >
               tone_feedback_adjustment(right.text, profile);
    });
    return selection;
}

}  // namespace owo::engine

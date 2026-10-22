#include "owo/engine/user_corpus.h"

#ifdef _WIN32
#include <Windows.h>
#include <dpapi.h>
#endif

#include <algorithm>
#include <array>
#include <fstream>
#include <iterator>

namespace owo::engine {
namespace {

constexpr std::array<unsigned char, 4> kMagic{'O', 'W', 'U', 'C'};
constexpr std::uint32_t kVersion = 1;
constexpr std::uint32_t kPlainEncoding = 0;
constexpr std::uint32_t kDpapiEncoding = 1;
constexpr std::size_t kHeaderSize = 16;
constexpr std::size_t kMaximumFileBytes = 32U * 1024U * 1024U;
constexpr std::size_t kMaximumClearedApplications = 256;
constexpr std::uint64_t kMillisecondsPerDay = 24ULL * 60ULL * 60ULL * 1000ULL;

void append_u32(std::vector<unsigned char>& out, const std::uint32_t value) {
    for (unsigned shift = 0; shift < 32; shift += 8) out.push_back(static_cast<unsigned char>(value >> shift));
}
void append_u64(std::vector<unsigned char>& out, const std::uint64_t value) {
    for (unsigned shift = 0; shift < 64; shift += 8) out.push_back(static_cast<unsigned char>(value >> shift));
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
bool read_u32(const std::span<const unsigned char> bytes, std::size_t& offset, std::uint32_t& value) {
    if (offset + 4 > bytes.size()) return false;
    value = 0;
    for (unsigned index = 0; index < 4; ++index) value |= static_cast<std::uint32_t>(bytes[offset + index]) << (index * 8U);
    offset += 4;
    return true;
}
bool read_u64(const std::span<const unsigned char> bytes, std::size_t& offset, std::uint64_t& value) {
    if (offset + 8 > bytes.size()) return false;
    value = 0;
    for (unsigned index = 0; index < 8; ++index) value |= static_cast<std::uint64_t>(bytes[offset + index]) << (index * 8U);
    offset += 8;
    return true;
}
bool read_bytes(const std::span<const unsigned char> bytes, std::size_t& offset,
                const std::size_t maximum, std::string& value) {
    std::uint32_t size{};
    if (!read_u32(bytes, offset, size) || size > maximum || offset + size > bytes.size()) return false;
    value.assign(reinterpret_cast<const char*>(bytes.data() + offset), size);
    offset += size;
    return true;
}

char ascii_lower(const char value) noexcept {
    return value >= 'A' && value <= 'Z' ? static_cast<char>(value - 'A' + 'a') : value;
}
std::string application_key(const std::string_view application) {
    std::string result;
    result.reserve(application.size());
    std::transform(application.begin(), application.end(), std::back_inserter(result), ascii_lower);
    return result;
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

std::size_t code_point_count(const std::string_view text) noexcept {
    return static_cast<std::size_t>(std::count_if(text.begin(), text.end(), [](const char value) {
        return (static_cast<unsigned char>(value) & 0xc0U) != 0x80U;
    }));
}

std::string_view trim(std::string_view text) noexcept {
    const auto blank = [](const char value) {
        return value == ' ' || value == '\t' || value == '\r' || value == '\n';
    };
    while (!text.empty() && blank(text.front())) text.remove_prefix(1);
    while (!text.empty() && blank(text.back())) text.remove_suffix(1);
    return text;
}

// Truncate on a UTF-8 boundary.
std::string_view utf8_prefix(std::string_view text, const std::size_t maximum_bytes) noexcept {
    if (text.size() <= maximum_bytes) return text;
    std::size_t size = maximum_bytes;
    while (size != 0 && (static_cast<unsigned char>(text[size]) & 0xc0U) == 0x80U) --size;
    return text.substr(0, size);
}

constexpr std::array<std::string_view, 8> kSentenceBoundaries{
    "。", "！", "？", "…", "!", "?", "\n", "\r"};

// Returns the byte offset just after the first sentence boundary, or npos.
std::size_t first_boundary_end(const std::string_view text) noexcept {
    std::size_t best = std::string_view::npos;
    for (const auto boundary : kSentenceBoundaries) {
        const auto found = text.find(boundary);
        if (found != std::string_view::npos)
            best = (std::min)(best, found + boundary.size());
    }
    return best;
}

std::vector<unsigned char> encode(const std::uint64_t next_id, const std::uint64_t cleared_all,
                                  const std::unordered_map<std::string, std::uint64_t>& cleared,
                                  const std::vector<UserCorpusEntry>& entries) {
    std::vector<unsigned char> payload;
    append_u64(payload, next_id);
    append_u64(payload, cleared_all);
    std::vector<std::pair<std::string, std::uint64_t>> ordered(cleared.begin(), cleared.end());
    std::sort(ordered.begin(), ordered.end());
    append_u32(payload, static_cast<std::uint32_t>(ordered.size()));
    for (const auto& [application, at] : ordered) {
        append_bytes(payload, application);
        append_u64(payload, at);
    }
    append_u32(payload, static_cast<std::uint32_t>(entries.size()));
    for (const auto& entry : entries) {
        append_u64(payload, entry.id);
        append_u64(payload, entry.created_at_ms);
        append_bytes(payload, entry.application);
        append_bytes(payload, entry.text);
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
    static constexpr wchar_t kDescription[] = L"OwO user corpus";
    const BOOL ok = protect
        ? CryptProtectData(&in, kDescription, nullptr, nullptr, nullptr, CRYPTPROTECT_UI_FORBIDDEN, &out)
        : CryptUnprotectData(&in, nullptr, nullptr, nullptr, nullptr, CRYPTPROTECT_UI_FORBIDDEN, &out);
    if (!ok) return false;
    output.assign(out.pbData, out.pbData + out.cbData);
    SecureZeroMemory(out.pbData, out.cbData);
    LocalFree(out.pbData);
    return true;
}
#endif

struct Decoded {
    std::uint64_t next_id{1};
    std::uint64_t cleared_all{};
    std::unordered_map<std::string, std::uint64_t> cleared;
    std::vector<UserCorpusEntry> entries;
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
        if (!input.read(reinterpret_cast<char*>(bytes.data()), static_cast<std::streamsize>(bytes.size())))
            return DecodeStatus::invalid;
    }
    const std::span<const unsigned char> all(bytes);
    if (!std::equal(kMagic.begin(), kMagic.end(), bytes.begin())) return DecodeStatus::invalid;
    std::size_t offset = 4;
    std::uint32_t version{}, encoding{}, length{};
    if (!read_u32(all, offset, version) || version != kVersion ||
        !read_u32(all, offset, encoding) || !read_u32(all, offset, length) ||
        offset + length != bytes.size()) return DecodeStatus::invalid;
    std::vector<unsigned char> plain(bytes.begin() + static_cast<std::ptrdiff_t>(offset), bytes.end());
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
    std::uint32_t cleared_count{}, entry_count{};
    if (!read_u64(view, cursor, result.next_id) || result.next_id == 0 ||
        !read_u64(view, cursor, result.cleared_all) ||
        !read_u32(view, cursor, cleared_count) || cleared_count > kMaximumClearedApplications)
        return DecodeStatus::invalid;
    for (std::uint32_t index = 0; index < cleared_count; ++index) {
        std::string application;
        std::uint64_t at{};
        if (!read_bytes(view, cursor, kMaximumUserCorpusApplicationBytes, application) ||
            application.empty() || !read_u64(view, cursor, at) ||
            !result.cleared.emplace(std::move(application), at).second)
            return DecodeStatus::invalid;
    }
    if (!read_u32(view, cursor, entry_count) || entry_count > kMaximumUserCorpusEntries)
        return DecodeStatus::invalid;
    result.entries.reserve(entry_count);
    std::uint64_t previous_id = 0;
    for (std::uint32_t index = 0; index < entry_count; ++index) {
        UserCorpusEntry entry;
        if (!read_u64(view, cursor, entry.id) || entry.id <= previous_id || entry.id >= result.next_id ||
            !read_u64(view, cursor, entry.created_at_ms) ||
            !read_bytes(view, cursor, kMaximumUserCorpusApplicationBytes, entry.application) ||
            !read_bytes(view, cursor, kMaximumUserCorpusSentenceBytes, entry.text) ||
            entry.text.empty() || !valid_utf8(entry.text) || !valid_utf8(entry.application))
            return DecodeStatus::invalid;
        previous_id = entry.id;
        result.entries.push_back(std::move(entry));
    }
    if (cursor != view.size()) return DecodeStatus::invalid;
    output = std::move(result);
    return DecodeStatus::ok;
}

bool write_file(const std::filesystem::path& path, const std::vector<unsigned char>& bytes) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    if (!output) return false;
    output.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
    output.flush();
    return static_cast<bool>(output);
}

#ifdef _WIN32
// Serializes load-modify-save across Core and the management tools.
class CorpusLock final {
public:
    CorpusLock() : mutex_(CreateMutexW(nullptr, FALSE, L"Local\\OwO.InputMethod.UserCorpus.v1")) {
        if (mutex_ == nullptr) return;
        const auto waited = WaitForSingleObject(mutex_, 5000);
        locked_ = waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED;
    }
    ~CorpusLock() {
        if (locked_) ReleaseMutex(mutex_);
        if (mutex_ != nullptr) CloseHandle(mutex_);
    }
    CorpusLock(const CorpusLock&) = delete;
    CorpusLock& operator=(const CorpusLock&) = delete;
    explicit operator bool() const noexcept { return locked_; }
private:
    HANDLE mutex_{};
    bool locked_{};
};
#else
class CorpusLock final {
public:
    explicit operator bool() const noexcept { return true; }
};
#endif

}  // namespace

bool user_corpus_sentence_acceptable(const std::string_view text) noexcept {
    const auto trimmed = trim(text);
    if (trimmed.empty() || trimmed.size() > kMaximumUserCorpusSentenceBytes ||
        !valid_utf8(trimmed) || code_point_count(trimmed) < 2) return false;
    std::size_t digits = 0;
    for (const char value : trimmed) {
        digits = value >= '0' && value <= '9' ? digits + 1 : 0;
        if (digits >= 6) return false;
    }
    return true;
}

UserCorpusIoResult UserCorpusStore::load(const std::filesystem::path& path) {
    path_ = path;
    next_id_ = 1;
    cleared_all_at_ms_ = 0;
    cleared_application_at_ms_.clear();
    entries_.clear();
    Decoded decoded;
    const auto status = decode(path_, decoded);
    auto backup = path_; backup += L".bak";
    if (status == DecodeStatus::ok) {
        next_id_ = decoded.next_id;
        cleared_all_at_ms_ = decoded.cleared_all;
        cleared_application_at_ms_ = std::move(decoded.cleared);
        entries_ = std::move(decoded.entries);
        return {true, false, {}};
    }
    const auto backup_status = decode(backup, decoded);
    if (backup_status == DecodeStatus::ok) {
        next_id_ = decoded.next_id;
        cleared_all_at_ms_ = decoded.cleared_all;
        cleared_application_at_ms_ = std::move(decoded.cleared);
        entries_ = std::move(decoded.entries);
        return {true, true, {}};
    }
    if (status == DecodeStatus::missing && backup_status == DecodeStatus::missing)
        return {true, false, {}};
    return {false, false, "user corpus and backup are invalid"};
}

UserCorpusIoResult UserCorpusStore::save() const {
    if (path_.empty()) return {false, false, "user corpus path is not set"};
    auto plain = encode(next_id_, cleared_all_at_ms_, cleared_application_at_ms_, entries_);
    std::uint32_t encoding = kPlainEncoding;
#ifdef _WIN32
    std::vector<unsigned char> protected_bytes;
    const bool encrypted = dpapi(plain, protected_bytes, true);
    SecureZeroMemory(plain.data(), plain.size());
    if (!encrypted) return {false, false, "cannot encrypt user corpus"};
    plain = std::move(protected_bytes);
    encoding = kDpapiEncoding;
#endif
    std::vector<unsigned char> bytes(kMagic.begin(), kMagic.end());
    append_u32(bytes, kVersion);
    append_u32(bytes, encoding);
    append_u32(bytes, static_cast<std::uint32_t>(plain.size()));
    bytes.insert(bytes.end(), plain.begin(), plain.end());
    if (bytes.size() > kMaximumFileBytes) return {false, false, "user corpus exceeds size limit"};
    std::error_code error;
    if (!path_.parent_path().empty()) std::filesystem::create_directories(path_.parent_path(), error);
    auto temporary = path_; temporary += L".tmp";
    auto backup = path_; backup += L".bak";
    if (!write_file(temporary, bytes)) {
        std::filesystem::remove(temporary, error);
        return {false, false, "cannot write temporary user corpus"};
    }
#ifdef _WIN32
    if (std::filesystem::exists(path_) && !CopyFileW(path_.c_str(), backup.c_str(), FALSE))
        return {false, false, "cannot update user corpus backup"};
    if (!MoveFileExW(temporary.c_str(), path_.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
        return {false, false, "cannot atomically replace user corpus"};
#else
    if (std::filesystem::exists(path_)) std::filesystem::copy_file(path_, backup, std::filesystem::copy_options::overwrite_existing, error);
    std::filesystem::rename(temporary, path_, error);
    if (error) return {false, false, "cannot replace user corpus"};
#endif
    return {true, false, {}};
}

bool UserCorpusStore::cleared_after(const std::string_view application,
                                    const std::uint64_t created_at_ms) const {
    if (created_at_ms <= cleared_all_at_ms_) return true;
    const auto found = cleared_application_at_ms_.find(application_key(application));
    return found != cleared_application_at_ms_.end() && created_at_ms <= found->second;
}

std::uint64_t UserCorpusStore::append(const std::string_view application,
                                      const std::string_view text,
                                      const std::uint64_t created_at_ms) {
    const auto trimmed = trim(text);
    if (!user_corpus_sentence_acceptable(trimmed) || application.empty() ||
        application.size() > kMaximumUserCorpusApplicationBytes || !valid_utf8(application) ||
        cleared_after(application, created_at_ms)) return 0;
    if (entries_.size() >= kMaximumUserCorpusEntries)
        entries_.erase(entries_.begin(), entries_.begin() +
            static_cast<std::ptrdiff_t>(entries_.size() - kMaximumUserCorpusEntries + 1));
    const auto id = next_id_++;
    entries_.push_back({id, created_at_ms, std::string(application), std::string(trimmed)});
    return id;
}

bool UserCorpusStore::erase(const std::uint64_t id) {
    const auto found = std::find_if(entries_.begin(), entries_.end(),
                                    [id](const auto& entry) { return entry.id == id; });
    if (found == entries_.end()) return false;
    entries_.erase(found);
    return true;
}

std::size_t UserCorpusStore::clear_application(const std::string_view application,
                                               const std::uint64_t now_ms) {
    const auto key = application_key(application);
    if (key.empty() || key.size() > kMaximumUserCorpusApplicationBytes) return 0;
    const auto before = entries_.size();
    std::erase_if(entries_, [&](const auto& entry) { return application_key(entry.application) == key; });
    // Remember the clear so sentences still buffered in Core cannot reappear.
    if (cleared_application_at_ms_.size() >= kMaximumClearedApplications &&
        !cleared_application_at_ms_.contains(key)) {
        cleared_all_at_ms_ = (std::max)(cleared_all_at_ms_, std::min_element(
            cleared_application_at_ms_.begin(), cleared_application_at_ms_.end(),
            [](const auto& left, const auto& right) { return left.second < right.second; })->second);
        std::erase_if(cleared_application_at_ms_, [this](const auto& value) {
            return value.second <= cleared_all_at_ms_;
        });
    }
    auto& at = cleared_application_at_ms_[key];
    at = (std::max)(at, now_ms);
    return before - entries_.size();
}

std::size_t UserCorpusStore::clear_all(const std::uint64_t now_ms) {
    const auto removed = entries_.size();
    entries_.clear();
    cleared_application_at_ms_.clear();
    cleared_all_at_ms_ = (std::max)(cleared_all_at_ms_, now_ms);
    return removed;
}

std::size_t UserCorpusStore::prune(const std::uint64_t now_ms, const std::uint32_t retention_days) {
    if (retention_days == 0) return 0;
    const auto window = static_cast<std::uint64_t>(retention_days) * kMillisecondsPerDay;
    if (now_ms <= window) return 0;
    const auto cutoff = now_ms - window;
    const auto before = entries_.size();
    std::erase_if(entries_, [cutoff](const auto& entry) { return entry.created_at_ms < cutoff; });
    return before - entries_.size();
}

UserCorpusIoResult UserCorpusStore::modify(const std::filesystem::path& path,
                                           const std::function<void(UserCorpusStore&)>& change,
                                           const bool reset_invalid) {
    const CorpusLock lock;
    if (!lock) return {false, false, "user corpus is locked"};
    UserCorpusStore store;
    auto loaded = store.load(path);
    if (!loaded.success) {
        if (!reset_invalid) return loaded;
        store = UserCorpusStore{};
        store.path_ = path;
    }
    change(store);
    auto saved = store.save();
    saved.recovered_from_backup = loaded.recovered_from_backup;
    return saved;
}

void UserCorpusCollector::observe(const std::string_view context_id,
                                  const std::string_view application,
                                  std::string_view fragment, const std::uint64_t now_ms) {
    if (context_id.empty() || application.empty() || fragment.empty()) return;
    finish_idle(now_ms);
    auto found = std::find_if(open_.begin(), open_.end(), [&](const auto& sentence) {
        return sentence.context_id == context_id;
    });
    if (found != open_.end() && found->application != application) {
        finish(*found);
        open_.erase(found);
        found = open_.end();
    }
    if (found == open_.end()) {
        if (open_.size() >= kMaximumOpenContexts) {
            const auto oldest = std::min_element(open_.begin(), open_.end(), [](const auto& l, const auto& r) {
                return l.updated_at_ms < r.updated_at_ms;
            });
            finish(*oldest);
            open_.erase(oldest);
        }
        open_.push_back({std::string(context_id), std::string(application), {}, now_ms, now_ms});
        found = std::prev(open_.end());
    }
    while (!fragment.empty()) {
        if (found->text.empty()) found->started_at_ms = now_ms;
        const auto boundary = first_boundary_end(fragment);
        auto piece = boundary == std::string_view::npos ? fragment : fragment.substr(0, boundary);
        // Never exceed the limit: cut the remainder into a new sentence.
        if (found->text.size() + piece.size() > kMaximumUserCorpusSentenceBytes) {
            piece = utf8_prefix(fragment, kMaximumUserCorpusSentenceBytes - found->text.size());
        }
        found->text.append(piece);
        found->updated_at_ms = now_ms;
        fragment.remove_prefix(piece.size());
        if (boundary != std::string_view::npos ||
            found->text.size() >= kMaximumUserCorpusSentenceBytes) finish(*found);
    }
}

void UserCorpusCollector::finish(OpenSentence& sentence) {
    auto text = utf8_prefix(trim(sentence.text), kMaximumUserCorpusSentenceBytes);
    if (user_corpus_sentence_acceptable(text))
        completed_.push_back({0, sentence.started_at_ms, sentence.application, std::string(text)});
    sentence.text.clear();
}

void UserCorpusCollector::finish_idle(const std::uint64_t now_ms) {
    for (auto iterator = open_.begin(); iterator != open_.end();) {
        if (now_ms >= iterator->updated_at_ms + kIdleBoundaryMs) {
            finish(*iterator);
            iterator = open_.erase(iterator);
        } else {
            ++iterator;
        }
    }
}

void UserCorpusCollector::finish_all() {
    for (auto& sentence : open_) finish(sentence);
    open_.clear();
}

void UserCorpusCollector::discard() noexcept {
    open_.clear();
    completed_.clear();
}

std::vector<UserCorpusEntry> UserCorpusCollector::take_completed() {
    return std::exchange(completed_, {});
}

std::uint64_t UserCorpusCollector::oldest_completed_ms() const noexcept {
    if (completed_.empty()) return 0;
    return std::min_element(completed_.begin(), completed_.end(), [](const auto& l, const auto& r) {
        return l.created_at_ms < r.created_at_ms;
    })->created_at_ms;
}

}  // namespace owo::engine

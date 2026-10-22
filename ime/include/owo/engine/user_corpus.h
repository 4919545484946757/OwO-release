#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <functional>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace owo::engine {

// Authorized personal corpus (learning layer 3). Only sentences the user typed
// through this IME in an allowed, non-sensitive field ever reach this store.
inline constexpr std::size_t kMaximumUserCorpusEntries = 20'000;
inline constexpr std::size_t kMaximumUserCorpusSentenceBytes = 512;
inline constexpr std::size_t kMaximumUserCorpusApplicationBytes = 260;

struct UserCorpusEntry {
    std::uint64_t id{};
    std::uint64_t created_at_ms{};
    std::string application;
    std::string text;
    bool operator==(const UserCorpusEntry&) const = default;
};

struct UserCorpusIoResult {
    bool success{};
    bool recovered_from_backup{};
    std::string error;
};

/// Snapshot of the corpus file. Persisted with a checksum and, on Windows,
/// encrypted for the current user with DPAPI.
class UserCorpusStore final {
public:
    /// A missing file and missing backup load as an empty corpus.
    [[nodiscard]] UserCorpusIoResult load(const std::filesystem::path& path);
    [[nodiscard]] UserCorpusIoResult save() const;

    /// Returns the assigned id, or 0 when the entry was rejected (empty, too
    /// long, or older than a clear operation for its application).
    std::uint64_t append(std::string_view application, std::string_view text,
                         std::uint64_t created_at_ms);
    bool erase(std::uint64_t id);
    std::size_t clear_application(std::string_view application, std::uint64_t now_ms);
    std::size_t clear_all(std::uint64_t now_ms);
    std::size_t prune(std::uint64_t now_ms, std::uint32_t retention_days);

    [[nodiscard]] const std::vector<UserCorpusEntry>& entries() const noexcept {
        return entries_;
    }
    [[nodiscard]] std::uint64_t cleared_all_at_ms() const noexcept { return cleared_all_at_ms_; }

    /// Load, modify and save under a cross-process lock so Core capture and
    /// user management (view/delete/clear) never lose each other's changes.
    /// When `reset_invalid` is true an unreadable corpus is replaced by an
    /// empty one instead of failing (used by explicit "clear all").
    [[nodiscard]] static UserCorpusIoResult modify(
        const std::filesystem::path& path,
        const std::function<void(UserCorpusStore&)>& change,
        bool reset_invalid = false);

private:
    [[nodiscard]] bool cleared_after(std::string_view application,
                                     std::uint64_t created_at_ms) const;

    std::filesystem::path path_;
    std::uint64_t next_id_{1};
    std::uint64_t cleared_all_at_ms_{};
    std::unordered_map<std::string, std::uint64_t> cleared_application_at_ms_;
    std::vector<UserCorpusEntry> entries_;
};

/// Assembles committed fragments into sentences per input context. A sentence
/// ends at terminal punctuation or a line break, when the context goes idle,
/// or when it reaches the size limit.
class UserCorpusCollector final {
public:
    static constexpr std::uint64_t kIdleBoundaryMs = 10'000;
    static constexpr std::size_t kMaximumOpenContexts = 4;

    void observe(std::string_view context_id, std::string_view application,
                 std::string_view fragment, std::uint64_t now_ms);
    void finish_idle(std::uint64_t now_ms);
    void finish_all();
    void discard() noexcept;
    [[nodiscard]] std::vector<UserCorpusEntry> take_completed();
    [[nodiscard]] std::size_t completed_count() const noexcept { return completed_.size(); }
    [[nodiscard]] std::uint64_t oldest_completed_ms() const noexcept;

private:
    struct OpenSentence {
        std::string context_id;
        std::string application;
        std::string text;
        std::uint64_t started_at_ms{};
        std::uint64_t updated_at_ms{};
    };
    void finish(OpenSentence& sentence);

    std::vector<OpenSentence> open_;
    std::vector<UserCorpusEntry> completed_;
};

/// True for sentences worth keeping: at least two characters and no long
/// digit runs (verification codes, phone and card numbers).
[[nodiscard]] bool user_corpus_sentence_acceptable(std::string_view text) noexcept;

}  // namespace owo::engine

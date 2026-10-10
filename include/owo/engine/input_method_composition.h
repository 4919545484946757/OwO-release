#pragma once
#include "owo/engine/input_method_provider.h"
#include <optional>

namespace owo::engine {
struct InputMethodCommit {
    std::string text;
    std::vector<std::string> consumed_input;
};
struct InputMethodApplyResult {
    bool accepted{};
    std::optional<InputMethodCommit> commit;
};
// UI-thread-owned adapter. No IPC, direct application writes or learning here.
// Requests use UTF-8 byte positions; Windows caret positions use UTF-16 units.
class InputMethodComposition final {
public:
    [[nodiscard]] bool begin(InputMethodDescriptor, std::string session_id);
    void close();
    [[nodiscard]] bool insert(std::string_view utf8);
    [[nodiscard]] bool backspace();
    [[nodiscard]] bool erase_forward();
    [[nodiscard]] bool move_caret(int scalar_delta);
    [[nodiscard]] bool set_caret_utf16(std::size_t units);
    [[nodiscard]] std::size_t caret_utf16() const;
    [[nodiscard]] std::size_t caret_byte() const { return caret_; }
    [[nodiscard]] const std::string& input() const { return input_; }
    // Retained visual snapshots are not selectable after edits or a new request.
    [[nodiscard]] const std::optional<InputMethodResponse>& display() const { return display_; }
    [[nodiscard]] bool selectable() const { return selectable_; }
    [[nodiscard]] std::optional<InputMethodRequest> compose_request(
        std::uint64_t id, std::size_t page, std::size_t page_size);
    [[nodiscard]] std::optional<InputMethodRequest> select_request(
        std::uint64_t id, std::string_view candidate_id);
    // Prepare commit only on matching successful selection acknowledgement.
    [[nodiscard]] InputMethodApplyResult apply(const InputMethodResponse&);
    // Call after the host application write/staging succeeds or fails. While prepared,
    // edits/requests are rejected; TSF must defer such keys, never silently discard them.
    [[nodiscard]] bool finish_commit(bool committed);
private:
    struct Pending {
        InputMethodRequest request;
        std::uint64_t revision{};
        std::optional<InputMethodCandidate> selected;
    };
    void edited();
    std::optional<InputMethodDescriptor> descriptor_;
    std::string session_, input_;
    std::size_t caret_{}, displayed_page_size_{};
    std::uint64_t revision_{}, last_id_{};
    std::optional<Pending> pending_;
    std::optional<InputMethodCandidate> prepared_;
    std::optional<InputMethodResponse> display_;
    bool selectable_{};
};
} // namespace owo::engine

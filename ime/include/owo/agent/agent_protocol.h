#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace owo::agent {

inline constexpr std::uint32_t kAgentProtocolVersion = 3;
inline constexpr std::uint32_t kMinimumAgentProtocolVersion = 2;
inline constexpr std::size_t kMaximumAgentPayloadBytes = 256U * 1024U;
inline constexpr std::size_t kMaximumAgentCommands = 64;
// v4 in-place write-back. Commands may carry an effect only when both sides list it.
inline constexpr const char* kAgentCapabilityOutputEffects = "output.effects";
inline constexpr const char* kAgentCapabilityInputSelection = "input.selection";
inline constexpr std::size_t kMaximumAgentEffectTextCodePoints = 16384;
inline constexpr std::size_t kMaximumAgentContextEntries = 32;
inline constexpr std::size_t kMaximumAgentContextEntryBytes = 32U * 1024U;
inline constexpr std::size_t kMaximumAgentContextBytes = 128U * 1024U;
inline constexpr std::size_t kMaximumAgentTaskSlots = 16;
inline constexpr std::size_t kMaximumAgentTaskRanges = 32;
inline constexpr std::size_t kMaximumAgentSlotUpdates = 8;
inline constexpr std::string_view kAgentPluginId = "org.owo.agent-ipc";
inline constexpr std::string_view kAgentPluginService = "owo.agent.exchange.v1";

enum class Action { submit, select, page, cancel, poll };
enum class Status {
    connecting,
    agent_mode,
    submitting,
    thinking,
    waiting_for_user,
    executing,
    waiting_for_confirmation,
    cancelling,
    cancelled,
    completed,
    disconnected,
    timeout,
    error,
};

struct InputPayload {
    std::string raw_pinyin;
    std::string segmented_pinyin;
    std::string selected_text;
    std::string pending_pinyin;
    std::string natural_language;
    std::string input_mode{"agent"};
    bool correction_enabled{};
    bool operator==(const InputPayload&) const = default;
};

struct ApplicationContext {
    std::uint32_t process_id{};
    std::uint32_t thread_id{};
    std::string executable;
    std::string window_class;
    std::string focus_window_class;
    std::string context_id;
    bool sensitive_input{};
    bool operator==(const ApplicationContext&) const = default;
};

struct ContextEntry {
    std::string context_id;
    ApplicationContext application;
    std::string text;
    std::uint64_t sequence{};
    std::string context_type{"committed_text"};
    std::string source{"owo_commit"};
    std::uint64_t created_at_ms{};
    std::uint32_t relevance_milli{};
    bool is_current{};
    bool filtered{};
    bool redacted{};
    bool truncated{};
    bool operator==(const ContextEntry&) const = default;
};

// Offsets refer to the original ASCII pinyin after the leading Agent marker `v`
// has been removed. Ranges are zero-based and half-open: [start, end).
struct PinyinRange {
    std::uint32_t start{};
    std::uint32_t end{};
    bool operator==(const PinyinRange&) const = default;
};

struct TaskSlot {
    std::string id;
    std::string label;
    std::string value;
    std::vector<PinyinRange> source_ranges;
    bool locked{};
    std::uint32_t confidence_milli{};
    bool operator==(const TaskSlot&) const = default;
};

struct TaskDraft {
    std::string intent;
    std::vector<PinyinRange> intent_ranges;
    std::vector<TaskSlot> slots;
    std::vector<PinyinRange> unconsumed_ranges;
    std::uint64_t revision{};
    bool operator==(const TaskDraft&) const = default;
};

struct SlotUpdate {
    std::string slot_id;
    std::string value;
    std::vector<PinyinRange> consumed_ranges;
    bool lock{true};
    bool operator==(const SlotUpdate&) const = default;
};

// Selected document text is distinct from input_details.selected_text (the command).
struct SelectionSummary {
    std::string text;
    std::string sha256;
    bool redacted{};
    bool operator==(const SelectionSummary&) const = default;
};

struct Request {
    Action action{Action::submit};
    std::string session_id;
    std::string user_input;
    ApplicationContext application;
    std::string session_context;
    std::vector<ContextEntry> context_entries;
    std::optional<SelectionSummary> selection;
    std::string command_id;
    std::uint32_t page{};
    InputPayload input_details;
    std::string request_id;
    std::string parent_request_id;
    std::string idempotency_key;
    std::vector<std::string> capabilities;
    std::uint32_t protocol_min{kMinimumAgentProtocolVersion};
    std::uint32_t protocol_max{kAgentProtocolVersion};
    std::vector<std::string> required_features;
    std::uint32_t schema_version{kAgentProtocolVersion};
    std::uint64_t task_revision{};
    std::vector<SlotUpdate> slot_updates;
    bool operator==(const Request&) const = default;
};

enum class EffectKind { commit_text, replace_selection, copy_text };

// Written locally by the IME after it rechecks the target; never executed by the Agent.
struct CommandEffect {
    EffectKind kind{EffectKind::commit_text};
    std::string text;
    // application.context_id of the originating request; required for replace_selection.
    std::string target_context_id;
    // Lowercase hex SHA-256 of the UTF-8 selection text; replace_selection only.
    std::string selection_sha256;
    bool operator==(const CommandEffect&) const = default;
};

struct Command {
    std::string id;
    std::string label;
    bool high_risk{};
    std::string description;
    std::string category;
    std::string risk_level{"none"};
    bool requires_confirmation{};
    std::string preview;
    bool enabled{true};
    std::string disabled_reason;
    std::uint64_t task_revision{};
    std::vector<SlotUpdate> slot_updates;
    bool commit_task{};
    std::optional<CommandEffect> effect;
    bool operator==(const Command&) const = default;
};

struct Response {
    std::string session_id;
    std::string message;
    std::vector<Command> commands;
    std::string executing_command;
    Status status{Status::error};
    std::uint32_t page{};
    bool has_more{};
    std::string error_code;
    std::string request_id;
    std::uint64_t state_revision{};
    std::uint32_t progress{};
    std::uint32_t retry_after_ms{};
    bool can_cancel{};
    bool can_continue_input{true};
    std::uint64_t expires_at_ms{};
    std::string error_message;
    bool retryable{};
    std::vector<std::string> capabilities;
    std::uint32_t schema_version{kAgentProtocolVersion};
    TaskDraft task;
    bool operator==(const Response&) const = default;
};

struct RequestDecodeResult {
    bool ok{};
    Request value;
    std::string diagnostic;
};

struct ResponseDecodeResult {
    bool ok{};
    Response value;
    std::string diagnostic;
};

struct FilterResult {
    bool ok{};
    std::string value;
    bool redacted{};
    std::string diagnostic;
};

// Status requests retain the origin binding without resending task/context payloads.
[[nodiscard]] Request make_poll_request(const Request& origin, std::uint64_t sequence);
[[nodiscard]] std::string encode_request(const Request& request);
[[nodiscard]] std::string encode_request_v2(const Request& request);
[[nodiscard]] RequestDecodeResult decode_request(std::string_view json);
[[nodiscard]] std::string encode_response(const Response& response);
[[nodiscard]] std::string encode_response_v2(const Response& response);
[[nodiscard]] ResponseDecodeResult decode_response(std::string_view json);
[[nodiscard]] FilterResult filter_sensitive_text(std::string_view utf8,
                                                 std::size_t maximum_bytes = 4096);
// Cross-application context is off by default: an entry may be forwarded only when it was
// recorded at the current input position or in the same executable. Sensitive targets
// receive nothing.
[[nodiscard]] bool context_entry_shareable(const ContextEntry& entry,
                                           const ApplicationContext& current) noexcept;
// Facts gathered inside the TSF read/write edit session right before an effect is written.
struct EffectTargetFacts {
    bool capability_negotiated{};
    bool same_document{};
    bool context_allowed{};
    std::string current_context_id;
    std::string selection_sha256;
};

enum class EffectDecision { write, preview_only };

// Every mismatch falls back to a preview; copy_text never touches the document.
[[nodiscard]] EffectDecision decide_effect(const CommandEffect& effect,
                                           const EffectTargetFacts& facts) noexcept;
[[nodiscard]] const char* effect_kind_name(EffectKind kind) noexcept;
[[nodiscard]] const char* action_name(Action action) noexcept;
[[nodiscard]] const char* status_name(Status status) noexcept;

}  // namespace owo::agent

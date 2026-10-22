#pragma once

#include "owo/protocol/envelope.h"

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace owo::protocol {

inline constexpr std::size_t kMaximumCandidateCount = 256;

enum class CandidateGroup : std::uint64_t {
    sentence = 0,
    dictionary = 1,
    single_character = 2,
    // A whole-sentence continuation learned from the user's authorized corpus.
    // It follows the page's normal candidates, consumes the full preedit, and
    // is accepted only by explicit choice, so the typed pinyin keeps its
    // Space/number key selection.
    continuation = 3,
};

enum class MessageType {
    candidate_request,
    candidate_response,
    candidate_update_request,
    candidate_update_response,
    candidate_committed,
    agent_context_recorded,
    agent_request,
    agent_response,
    acknowledgement,
    shutdown_request,
    error_response,
};

struct Message {
    MessageType type{MessageType::error_response};
    std::uint64_t request_id{};
    std::uint64_t context_generation{};
    std::string text;
    std::vector<std::string> candidates;
    std::uint64_t page{};
    bool has_more{};
    bool model_pending{};
    bool defer_expensive_candidates{};
    bool candidate_refinement_pending{};
    std::vector<std::string> syllables;
    std::vector<std::uint64_t> candidate_consumed;
    std::vector<std::uint64_t> candidate_groups;
    bool expanded{};
    std::uint64_t page_size{};
    bool correction_enabled{true};
    std::string input;
    std::string context;
    std::string application_context_id;
    std::uint64_t application_process_id{};
    std::uint64_t application_thread_id{};
    std::string application_executable;
    std::string application_window_class;
    std::string application_focus_window_class;
    bool application_sensitive_input{};
    // Only agent_context_recorded may set this: the fragment was typed by the
    // user (not written back by an Agent) and may enter the authorized corpus.
    bool corpus_eligible{};
};

struct DecodeResult {
    Message message;
    ValidationResult validation;
};

/// 将内部消息编码为 UTF-8 JSON。该表示尚不是稳定公共协议。
[[nodiscard]] std::string encode_message(const Message& message);

/// 严格解析内部消息；拒绝未知字段、重复字段和非法转义。
[[nodiscard]] DecodeResult decode_message(std::string_view json);

}  // namespace owo::protocol

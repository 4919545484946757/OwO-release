#include "owo/protocol/messages.h"

#include <iostream>

int main() {
    using namespace owo::protocol;
    int failures = 0;
    for (const auto type : {MessageType::user_lexicon_request, MessageType::user_lexicon_response}) {
        Message management{type, 77, 42, "list\t-\t0\t50\tmerge\n\n\t\n"};
        const auto result = decode_message(encode_message(management));
        if (!result.validation || result.message.type != type || result.message.text != management.text)
            ++failures;
    }
    Message original{MessageType::candidate_response, 9, 4, "你好",
                     {"你好", "你号", "a\\\"b\n中文"}, 2, true};
    original.syllables = {"ni", "hao"};
    original.candidate_consumed = {5, 5, 5};
    original.candidate_groups = {
        static_cast<std::uint64_t>(CandidateGroup::sentence),
        static_cast<std::uint64_t>(CandidateGroup::dictionary),
        static_cast<std::uint64_t>(CandidateGroup::single_character)};
    original.expanded = true;
    original.page_size = 5;
    original.correction_enabled = false;
    const auto encoded = encode_message(original);
    const auto decoded = decode_message(encoded);
    if (!decoded.validation || decoded.message.type != original.type ||
        decoded.message.request_id != original.request_id ||
        decoded.message.context_generation != original.context_generation ||
        decoded.message.text != original.text ||
        decoded.message.candidates != original.candidates ||
        decoded.message.page != original.page ||
        decoded.message.has_more != original.has_more ||
        decoded.message.model_pending != original.model_pending ||
        decoded.message.defer_expensive_candidates !=
            original.defer_expensive_candidates ||
        decoded.message.candidate_refinement_pending !=
            original.candidate_refinement_pending ||
        decoded.message.syllables != original.syllables ||
        decoded.message.candidate_consumed != original.candidate_consumed ||
        decoded.message.candidate_groups != original.candidate_groups ||
        decoded.message.expanded != original.expanded ||
        decoded.message.page_size != original.page_size ||
        decoded.message.correction_enabled != original.correction_enabled) {
        std::cerr << "message round trip failed\n";
        ++failures;
    }
    Message feedback{MessageType::candidate_committed, 10, 5, "你好"};
    feedback.input = "Ni'Hao";
    feedback.context = "我说";
    const auto decoded_feedback = decode_message(encode_message(feedback));
    if (!decoded_feedback.validation || decoded_feedback.message.input != feedback.input ||
        decoded_feedback.message.context != feedback.context) {
        std::cerr << "commit feedback input round trip failed\n";
        ++failures;
    }
    feedback.input = "ni hao";
    if (!encode_message(feedback).empty()) {
        std::cerr << "invalid commit feedback input was encoded\n";
        ++failures;
    }
    Message contextual_request{MessageType::candidate_request, 11, 6, "wang"};
    contextual_request.input = "kuang";
    contextual_request.context = "狂";
    contextual_request.defer_expensive_candidates = true;
    const auto decoded_contextual = decode_message(encode_message(contextual_request));
    if (!decoded_contextual.validation ||
        decoded_contextual.message.input != contextual_request.input ||
        decoded_contextual.message.context != contextual_request.context ||
        !decoded_contextual.message.defer_expensive_candidates) {
        std::cerr << "candidate context input round trip failed\n";
        ++failures;
    }
    contextual_request.input = "kuang wang";
    if (!encode_message(contextual_request).empty()) {
        std::cerr << "invalid candidate context input was encoded\n";
        ++failures;
    }
    auto invalid_deferred_response = original;
    invalid_deferred_response.defer_expensive_candidates = true;
    if (!encode_message(invalid_deferred_response).empty()) {
        std::cerr << "response accepted request-only correction deferral\n";
        ++failures;
    }
    Message context_event{MessageType::agent_context_recorded, 13, 5, "前序上下文"};
    context_event.application_context_id = "ctx-editor-1";
    context_event.application_process_id = 42;
    context_event.application_thread_id = 84;
    context_event.application_executable = "editor.exe";
    context_event.application_window_class = "EditorWindow";
    context_event.application_focus_window_class = "TextBox";
    const auto decoded_context_event = decode_message(encode_message(context_event));
    if (!decoded_context_event.validation ||
        decoded_context_event.message.application_context_id !=
            context_event.application_context_id ||
        decoded_context_event.message.application_thread_id != 84 ||
        decoded_context_event.message.application_focus_window_class != "TextBox") {
        std::cerr << "Agent context event round trip failed\n";
        ++failures;
    }
    context_event.application_context_id = "invalid context";
    if (!encode_message(context_event).empty()) {
        std::cerr << "invalid Agent context identifier was encoded\n";
        ++failures;
    }
    Message agent_request{MessageType::agent_request, 12, 7,
        R"({"schema_version":1,"action":"submit"})"};
    const auto decoded_agent = decode_message(encode_message(agent_request));
    if (!decoded_agent.validation || decoded_agent.message.type != MessageType::agent_request ||
        decoded_agent.message.text != agent_request.text) {
        std::cerr << "agent request envelope round trip failed\n";
        ++failures;
    }
    Message agent_response{MessageType::agent_response, 12, 7,
        R"({"schema_version":1,"status":"thinking"})"};
    const auto decoded_agent_response = decode_message(encode_message(agent_response));
    if (!decoded_agent_response.validation ||
        decoded_agent_response.message.type != MessageType::agent_response) {
        std::cerr << "agent response envelope round trip failed\n";
        ++failures;
    }
    if (decode_message("{}").validation.error != ErrorCode::invalid_payload) {
        std::cerr << "invalid schema was accepted\n";
        ++failures;
    }
    auto wrong_version = encoded;
    const auto version_text = "\"protocol_version\":" + std::to_string(kProtocolVersion);
    const auto version_offset = wrong_version.find(version_text);
    if (version_offset == std::string::npos) {
        std::cerr << "encoded protocol version missing\n";
        ++failures;
    } else {
        wrong_version.replace(version_offset, version_text.size(),
                              "\"protocol_version\":999");
    }
    if (decode_message(wrong_version).validation.error != ErrorCode::unsupported_protocol) {
        std::cerr << "wrong version was accepted\n";
        ++failures;
    }
    auto forged_plugin_call = encoded;
    const auto response_type = std::string("\"type\":\"candidate_response\"");
    const auto type_offset = forged_plugin_call.find(response_type);
    if (type_offset == std::string::npos) {
        std::cerr << "encoded message type missing\n";
        ++failures;
    } else {
        forged_plugin_call.replace(type_offset, response_type.size(),
                                   "\"type\":\"plugin_invoke\"");
    }
    if (decode_message(forged_plugin_call).validation.error != ErrorCode::invalid_payload) {
        std::cerr << "forged plugin invocation type was accepted\n";
        ++failures;
    }
    auto invalid_syllable = original;
    invalid_syllable.syllables = {"ni", "Hao"};
    if (!encode_message(invalid_syllable).empty()) {
        std::cerr << "invalid syllable was encoded\n";
        ++failures;
    }
    auto invalid_consumption = original;
    invalid_consumption.candidate_consumed.pop_back();
    if (!encode_message(invalid_consumption).empty()) {
        std::cerr << "misaligned candidate consumption was encoded\n";
        ++failures;
    }
    auto expanded_capacity = original;
    expanded_capacity.candidates.assign(kMaximumCandidateCount, "候选");
    expanded_capacity.candidate_consumed.assign(kMaximumCandidateCount, 5);
    expanded_capacity.candidate_groups.assign(
        kMaximumCandidateCount,
        static_cast<std::uint64_t>(CandidateGroup::dictionary));
    if (encode_message(expanded_capacity).empty()) {
        std::cerr << "expanded candidate capacity was rejected\n";
        ++failures;
    }
    // Learned continuations are a distinct, first-page group.
    auto continuation_group = original;
    continuation_group.candidate_groups.back() =
        static_cast<std::uint64_t>(CandidateGroup::continuation);
    const auto decoded_continuation =
        decode_message(encode_message(continuation_group));
    if (!decoded_continuation.validation ||
        decoded_continuation.message.candidate_groups.back() !=
            static_cast<std::uint64_t>(CandidateGroup::continuation)) {
        std::cerr << "continuation candidate group round trip failed\n";
        ++failures;
    }
    auto unknown_group = original;
    unknown_group.candidate_groups.back() = 4;
    if (!encode_message(unknown_group).empty()) {
        std::cerr << "unknown candidate group was encoded\n";
        ++failures;
    }
    expanded_capacity.candidates.push_back("越界");
    expanded_capacity.candidate_consumed.push_back(5);
    expanded_capacity.candidate_groups.push_back(
        static_cast<std::uint64_t>(CandidateGroup::dictionary));
    if (!encode_message(expanded_capacity).empty()) {
        std::cerr << "oversized expanded candidate response was encoded\n";
        ++failures;
    }
    auto invalid_page_size = original;
    invalid_page_size.page_size = 10;
    if (!encode_message(invalid_page_size).empty()) {
        std::cerr << "invalid candidate page size was encoded\n";
        ++failures;
    }
    // Corpus eligibility may only be claimed by a user-authored context record.
    Message context_entry{MessageType::agent_context_recorded, 13, 8, "今天开会"};
    context_entry.application_context_id = "ctx-13-8";
    context_entry.corpus_eligible = true;
    const auto decoded_entry = decode_message(encode_message(context_entry));
    if (!decoded_entry.validation || !decoded_entry.message.corpus_eligible) {
        std::cerr << "corpus eligible context record round trip failed\n";
        ++failures;
    }
    auto sensitive_entry = context_entry;
    sensitive_entry.application_sensitive_input = true;
    if (!encode_message(sensitive_entry).empty()) {
        std::cerr << "sensitive context record claimed corpus eligibility\n";
        ++failures;
    }
    auto forged_eligibility = context_entry;
    forged_eligibility.type = MessageType::candidate_committed;
    forged_eligibility.text = "你好";
    if (!encode_message(forged_eligibility).empty()) {
        std::cerr << "commit feedback claimed corpus eligibility\n";
        ++failures;
    }
    auto minted_eligibility = encoded;
    const auto eligibility_offset = minted_eligibility.find("\"corpus_eligible\":false");
    if (eligibility_offset == std::string::npos) {
        std::cerr << "encoded corpus eligibility missing\n";
        ++failures;
    } else {
        minted_eligibility.replace(eligibility_offset,
                                   std::string("\"corpus_eligible\":false").size(),
                                   "\"corpus_eligible\":true");
        if (decode_message(minted_eligibility).validation.error != ErrorCode::invalid_payload) {
            std::cerr << "minted corpus eligibility was accepted\n";
            ++failures;
        }
    }
    return failures == 0 ? 0 : 1;
}

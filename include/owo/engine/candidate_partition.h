#pragma once

#include "owo/engine/candidate_generator.h"

#include <span>

namespace owo::engine {

[[nodiscard]] bool is_non_single_candidate(const Candidate& candidate) noexcept;
[[nodiscard]] bool requires_incomplete_candidate_limit(
    const ParseResult& parsed, std::span<const Candidate> candidates) noexcept;
// For incomplete input, all non-single candidates share a quota, including
// dictionary/user words, partial prefixes and the hidden model tail.
void limit_incomplete_candidates(std::vector<Candidate>& candidates, std::size_t page_size,
                                bool incomplete_input, std::size_t reserved_slots = 0);

// Within a character area, exact whole-input readings precede heads that
// consume only a shorter prefix. Preserve each reading tier's existing order.
void prioritize_complete_characters(std::vector<Candidate>& candidates,
                                    std::size_t full_input_bytes);

// Apply once to a fresh generation pool before coherence/model copies. Only
// equal-length, exact, whole-input compositions compete with exact whole words;
// confirmed user preferences and provisional/hidden dictionary anchors are exempt.
void apply_exact_dictionary_sentence_penalty(std::span<Candidate> candidates,
                                              std::size_t full_input_bytes);

// Unify comparable word/sentence candidates in their existing multi-character
// slots. Single-character quotas, consumed ranges and hidden-tail order stay intact.
void merge_similar_length_partitions(std::vector<Candidate> &candidates,
                                     std::size_t full_input_bytes);

} // namespace owo::engine

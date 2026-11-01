#pragma once

#include "owo/engine/candidate_generator.h"

#include <span>

namespace owo::engine {

[[nodiscard]] bool is_incomplete_composition(const Candidate& candidate) noexcept;
// One shared quota across visible, partial and hidden generated compositions.
// Dictionary/user words and complete/corrected readings are not charged.
void limit_incomplete_compositions(std::vector<Candidate>& candidates, std::size_t page_size);

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

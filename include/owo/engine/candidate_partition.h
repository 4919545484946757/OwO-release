#pragma once

#include "owo/engine/candidate_generator.h"

#include <span>

namespace owo::engine {

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

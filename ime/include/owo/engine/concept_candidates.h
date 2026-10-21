#pragma once

#include "owo/engine/candidate_generator.h"

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace owo::engine {

// Concept candidates answer "I remember the description, not the term". The
// lookup is a local glossary: every entry maps pinyin cue readings (the
// reading of a short description) to one canonical term. Nothing is inferred
// from the document, nothing is generated, and nothing leaves the machine.
struct ConceptEntry {
    // The term itself, for example 幂等.
    std::string term;
    // Pinyin of the term, kept for display and diagnostics.
    std::string term_reading;
    // Normalized cue readings: lowercase ASCII letters only.
    std::vector<std::string> cue_readings;
    // Optional human-readable description carried from the glossary file.
    std::string cue_text;
};

class ConceptTable {
public:
    ConceptTable() = default;
    explicit ConceptTable(std::vector<ConceptEntry> entries);

    [[nodiscard]] const std::vector<ConceptEntry>& entries() const noexcept {
        return entries_;
    }
    [[nodiscard]] bool empty() const noexcept { return entries_.empty(); }

    // Exact whole-preedit match after normalization. Matching is deliberately
    // exact: a partial cue must never decorate ordinary typing.
    [[nodiscard]] std::vector<const ConceptEntry*> lookup(
        std::string_view normalized_key) const;

    // Progressive match for typing a description: a cue that starts with the
    // key counts once the key is at least `minimum_prefix` letters long, so the
    // term appears while the description is still being typed. Callers should
    // only use this when no exact match exists.
    [[nodiscard]] std::vector<const ConceptEntry*> lookup_prefix(
        std::string_view normalized_key,
        std::size_t minimum_prefix) const;

private:
    // Cue lookup indexes are built once per table; project glossaries can hold
    // thousands of entries and the lookup runs on every candidate request.
    void rebuild_index();

    std::vector<ConceptEntry> entries_;
    std::unordered_map<std::string, std::vector<std::size_t>> exact_;
    // Sorted (cue, entry) pairs for prefix ranges; deterministic order keeps
    // candidate ordering stable across runs.
    std::vector<std::pair<std::string, std::size_t>> sorted_cues_;
};

struct ConceptTableLoadResult {
    std::optional<ConceptTable> table;
    std::string error;
};

// Loads `term<TAB>reading<TAB>cue1,cue2[,cue3]` lines (UTF-8, '#' comments and
// blank lines ignored). Malformed input yields an error instead of a partial
// table so a broken glossary cannot silently degrade candidate quality.
[[nodiscard]] ConceptTableLoadResult load_concept_table(
    const std::filesystem::path& path);

// Small starter glossary compiled into the engine so the feature works before a
// project glossary exists. Replace it with a local TSV for real work.
[[nodiscard]] const ConceptTable& builtin_concept_table();

// Normalizes a preedit the way cue readings are matched: lowercase ASCII
// letters only.
[[nodiscard]] std::string concept_lookup_key(std::string_view raw);

inline constexpr std::int64_t kConceptCandidateScore = 3'400;
inline constexpr std::int64_t kConceptPrefixScore = 3'300;
// Short keys stay exact: only a description-length prefix may hint a term.
inline constexpr std::size_t kConceptPrefixMinimumLetters = 8;
inline constexpr std::size_t kConceptMaximumCandidates = 3;

// Whole-preedit concept candidates. Exact cue matches win; when there is no
// exact match and the key is long enough, cue prefixes are offered at a
// slightly lower score. Returned candidates consume the full input and carry no
// source segmentation, so they behave exactly like dictionary candidates when
// committed.
[[nodiscard]] std::vector<Candidate> generate_concept_candidates(
    std::string_view raw,
    const ConceptTable& table = builtin_concept_table(), std::size_t limit = 2);

}  // namespace owo::engine

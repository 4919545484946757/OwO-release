#include "owo/engine/binary_lexicon.h"
#include "owo/engine/candidate_scheduler.h"
#include "owo/engine/lexicon_candidate_provider.h"

#include <algorithm>
#include <chrono>
#include <filesystem>
#include <iostream>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_set>
#include <vector>

namespace {

class VectorProvider final : public owo::engine::CandidateProvider {
public:
    VectorProvider(std::string id, std::vector<owo::engine::Candidate> candidates)
        : id_(std::move(id)), candidates_(std::move(candidates)) {}

    std::string_view id() const noexcept override { return id_; }

    owo::engine::CandidateBatch request(
        const owo::engine::CandidateRequestContext& context,
        const owo::engine::GenerationBudget budget,
        const owo::engine::ProviderContinuation* continuation) override {
        std::size_t offset = continuation == nullptr
            ? 0 : static_cast<std::size_t>(continuation->opaque_state.front());
        owo::engine::CandidateBatch batch;
        while (offset < candidates_.size() &&
               batch.candidates.size() < budget.max_results &&
               batch.work_units_consumed < budget.max_work_units) {
            batch.candidates.push_back(candidates_[offset++]);
            ++batch.work_units_consumed;
        }
        batch.exhausted = offset == candidates_.size();
        if (!batch.exhausted) {
            batch.next_score_upper_bound = candidates_[offset].score;
            batch.continuation = owo::engine::ProviderContinuation{
                id_, context.input_fingerprint, context.lexicon_generation,
                context.parse_generation, {offset}};
        }
        return batch;
    }

private:
    std::string id_;
    std::vector<owo::engine::Candidate> candidates_;
};

owo::engine::Candidate make_candidate(std::string text, const std::int64_t score) {
    owo::engine::Candidate candidate;
    candidate.text = std::move(text);
    candidate.score = score;
    return candidate;
}

double microseconds(const std::chrono::steady_clock::duration elapsed) {
    return std::chrono::duration<double, std::micro>(elapsed).count();
}

double percentile(std::vector<double> values, const double fraction) {
    std::sort(values.begin(), values.end());
    const auto index = static_cast<std::size_t>(
        fraction * static_cast<double>(values.size() - 1));
    return values[index];
}

bool is_single_utf8_character(const std::string_view text) {
    if (text.empty()) return false;
    const auto lead = static_cast<unsigned char>(text.front());
    const std::size_t bytes = lead < 0x80 ? 1 : lead < 0xE0 ? 2 :
                              lead < 0xF0 ? 3 : lead < 0xF8 ? 4 : 0;
    if (bytes == 0 || text.size() != bytes) return false;
    for (std::size_t index = 1; index < bytes; ++index) {
        if ((static_cast<unsigned char>(text[index]) & 0xC0) != 0x80) return false;
    }
    return true;
}

}  // namespace

int main(int argc, char** argv) {
    constexpr std::size_t kEntries = 4096;
    constexpr std::size_t kVisibleCandidates = 7;
    constexpr std::size_t kLegacyMaterialization = 256;
    constexpr std::size_t kSamplesPerInitial = 1'000;

    if (argc > 2) return 2;
    const std::optional<std::filesystem::path> external_path = argc == 2
        ? std::optional<std::filesystem::path>(argv[1]) : std::nullopt;
    const std::string_view initials = external_path
        ? std::string_view("bcdfghjklmnpqrstvwxyz") : std::string_view("d");

    const auto path = external_path.value_or(
        std::filesystem::temp_directory_path() /
        "owo-candidate-budget-benchmark.owolx");
    if (!external_path) {
        std::vector<owo::engine::LexiconEntry> entries;
        entries.reserve(kEntries);
        for (std::size_t index = 0; index < kEntries; ++index) {
            entries.push_back({{"d" + std::to_string(index)},
                               "candidate-" + std::to_string(index),
                               static_cast<std::uint32_t>(kEntries - index)});
        }
        const auto written = owo::engine::write_binary_lexicon(path, std::move(entries));
        if (!written.success) return 2;
    }
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(path).success) return 2;

    owo::engine::InitialLexiconCandidateProvider provider(
        lexicon, external_path ? owo::engine::InitialCandidateFilter::single_character
                               : owo::engine::InitialCandidateFilter::none);
    const std::vector<owo::engine::ProviderAllocation> providers{{&provider, 1}};
    const owo::engine::CandidateScheduler scheduler;

    std::size_t legacy_work = 0;
    std::size_t scheduled_work = 0;
    std::size_t equivalent_initials = 0;
    std::size_t equivalent_three_page_initials = 0;
    std::size_t legacy_three_page_work = 0;
    std::size_t scheduled_three_page_work = 0;
    std::vector<double> legacy_samples;
    std::vector<double> scheduled_samples;
    legacy_samples.reserve(initials.size() * kSamplesPerInitial);
    scheduled_samples.reserve(initials.size() * kSamplesPerInitial);
    for (const char initial : initials) {
        std::vector<std::string> legacy_visible;
        std::vector<std::string> scheduled_visible;
        const owo::engine::CandidateRequestContext context{
            std::string(1, initial), static_cast<std::uint64_t>(initial), 1, 1};
        for (std::size_t sample = 0; sample < kSamplesPerInitial; ++sample) {
            const auto legacy_started = std::chrono::steady_clock::now();
            auto legacy = lexicon.lookup_initial(initial, kLegacyMaterialization);
            legacy_samples.push_back(microseconds(
                std::chrono::steady_clock::now() - legacy_started));
            legacy_work += legacy.size();
            if (sample == 0) {
                std::unordered_set<std::string> seen;
                for (const auto& entry : legacy) {
                    if (external_path && (entry.syllables.size() != 1 ||
                                          !is_single_utf8_character(entry.text)))
                        continue;
                    if (seen.insert(entry.text).second)
                        legacy_visible.push_back(entry.text);
                    if (legacy_visible.size() == kVisibleCandidates) break;
                }
            }

            const auto scheduled_started = std::chrono::steady_clock::now();
            const auto scheduled = scheduler.generate(
                context, providers, {kVisibleCandidates, kLegacyMaterialization});
            scheduled_samples.push_back(microseconds(
                std::chrono::steady_clock::now() - scheduled_started));
            scheduled_work += scheduled.metrics.work_units_consumed;
            if (sample == 0) {
                for (const auto& candidate : scheduled.candidates)
                    scheduled_visible.push_back(candidate.text);
            }
        }
        if (legacy_visible == scheduled_visible) ++equivalent_initials;

        auto legacy_three_page_entries =
            lexicon.lookup_initial(initial, kLegacyMaterialization);
        legacy_three_page_work += legacy_three_page_entries.size();
        std::vector<std::string> legacy_three_pages;
        std::unordered_set<std::string> legacy_seen;
        for (const auto& entry : legacy_three_page_entries) {
            if (external_path && (entry.syllables.size() != 1 ||
                                  !is_single_utf8_character(entry.text)))
                continue;
            if (legacy_seen.insert(entry.text).second)
                legacy_three_pages.push_back(entry.text);
            if (legacy_three_pages.size() == kVisibleCandidates * 3) break;
        }
        std::vector<std::string> scheduled_three_pages;
        std::optional<owo::engine::CandidateScheduleContinuation> continuation;
        for (std::size_t page_index = 0; page_index < 3; ++page_index) {
            auto page = scheduler.generate(
                context, providers, {kVisibleCandidates, kLegacyMaterialization},
                continuation ? &*continuation : nullptr);
            scheduled_three_page_work += page.metrics.work_units_consumed;
            for (const auto& candidate : page.candidates)
                scheduled_three_pages.push_back(candidate.text);
            continuation = std::move(page.continuation);
            if (!continuation) break;
        }
        if (legacy_three_pages == scheduled_three_pages)
            ++equivalent_three_page_initials;
    }

    std::error_code ignored;
    if (!external_path) std::filesystem::remove(path, ignored);
    const auto requests = initials.size() * kSamplesPerInitial;
    const bool equivalent = equivalent_initials == initials.size();
    const auto saved_percent = 100.0 *
        (1.0 - static_cast<double>(scheduled_work) / static_cast<double>(legacy_work));

    VectorProvider sparse_dictionary("sparse-dictionary", {
        make_candidate("dictionary", 100)});
    std::vector<owo::engine::Candidate> character_candidates;
    for (std::size_t index = 0; index < 12; ++index)
        character_candidates.push_back(
            make_candidate("character-" + std::to_string(index), 90 - index));
    VectorProvider characters("characters", std::move(character_candidates));
    const std::vector<owo::engine::ProviderAllocation> mixed_providers{
        {&sparse_dictionary, 1}, {&characters, 1}};
    const auto mixed = scheduler.generate(
        {"mixed", 2, 1, 1}, mixed_providers,
        {kVisibleCandidates, kLegacyMaterialization});
    std::cout << "{\"benchmark\":\"owo.engine.candidate_budget_ab\""
              << ",\"source\":\"" << (external_path ? "external" : "synthetic") << "\""
              << ",\"requests\":" << requests
              << ",\"initials\":" << initials.size()
              << ",\"visible_candidates\":" << kVisibleCandidates
              << ",\"legacy_work_per_request\":"
              << static_cast<double>(legacy_work) / static_cast<double>(requests)
              << ",\"scheduled_work_per_request\":"
              << static_cast<double>(scheduled_work) / static_cast<double>(requests)
              << ",\"work_saved_percent\":" << saved_percent
              << ",\"legacy_latency_us\":{\"p50\":"
              << percentile(legacy_samples, 0.50) << ",\"p95\":"
              << percentile(legacy_samples, 0.95) << ",\"p99\":"
              << percentile(legacy_samples, 0.99) << "}"
              << ",\"scheduled_latency_us\":{\"p50\":"
              << percentile(scheduled_samples, 0.50) << ",\"p95\":"
              << percentile(scheduled_samples, 0.95) << ",\"p99\":"
              << percentile(scheduled_samples, 0.99) << "}"
              << ",\"equivalent_initials\":" << equivalent_initials
              << ",\"three_page\":{\"equivalent_initials\":"
              << equivalent_three_page_initials
              << ",\"legacy_work_per_initial\":"
              << static_cast<double>(legacy_three_page_work) /
                     static_cast<double>(initials.size())
              << ",\"scheduled_work_per_initial\":"
              << static_cast<double>(scheduled_three_page_work) /
                     static_cast<double>(initials.size()) << "}"
              << ",\"multi_source\":{\"results\":" << mixed.candidates.size()
              << ",\"provider_requests\":" << mixed.metrics.provider_requests
              << ",\"continuation_requests\":"
              << mixed.metrics.continuation_requests
              << ",\"result_slots_reclaimed\":"
              << mixed.metrics.result_slots_reclaimed
              << ",\"reallocated_result_budget\":"
              << mixed.metrics.reallocated_result_budget
              << ",\"candidates_after_reallocation\":"
              << mixed.metrics.candidates_generated_after_reallocation
              << ",\"work_consumed\":" << mixed.metrics.work_units_consumed
              << ",\"work_reclaimed\":" << mixed.metrics.work_units_reclaimed
              << ",\"reallocated_work_budget\":"
              << mixed.metrics.reallocated_work_budget << "}"
              << ",\"visible_output_equivalent\":"
              << (equivalent ? "true" : "false") << "}\n";
    return equivalent && equivalent_three_page_initials == initials.size() &&
                   mixed.candidates.size() == kVisibleCandidates &&
                   mixed.metrics.continuation_requests != 0 &&
                   mixed.metrics.result_slots_reclaimed != 0 &&
                   mixed.metrics.candidates_generated_after_reallocation != 0 &&
                   mixed.metrics.work_units_reclaimed != 0
        ? 0 : 3;
}

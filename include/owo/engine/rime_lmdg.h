#pragma once

#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <string_view>

namespace owo::engine {

// Read-only subset of Rime::Grammar/1.0, for a user-supplied local model.
// No dependency on librime, dictionary replacement, or run-time code loading.
class RimeLmdg final {
public:
    RimeLmdg() = default;
    RimeLmdg(const RimeLmdg&) = delete;
    RimeLmdg& operator=(const RimeLmdg&) = delete;
    ~RimeLmdg();

    // An empty result means success. A failed (re)load leaves the reader disabled.
    [[nodiscard]] std::string_view load(const std::filesystem::path& path);
    [[nodiscard]] bool ready() const noexcept { return view_ != nullptr; }
    // Exact 2–4 Han characters per side, at most 6 combined. Nonmatches are neutral.
    [[nodiscard]] std::int64_t boundary_bonus(std::string_view left,
                                              std::string_view right) const noexcept;
    static constexpr std::int64_t kMaximumBonus = 6'000;
    static constexpr std::int64_t kMaximumPathBonus = 9'000;
    static constexpr std::int64_t kConfirmedSentenceBonus = 3'500;
    static constexpr std::uint32_t kMaximumQueries = 4'096;

private:
    void reset() noexcept;
    const unsigned char* view_{};
    const unsigned char* table_{};
    std::size_t unit_count_{};
    void* file_{};
};

} // namespace owo::engine

#pragma once

#include "owo/tsf/input_method_context.h"
#include <atomic>
#include <cstdint>
#include <memory>
#include <string>
#include <string_view>
#include <thread>
#include <wrl/client.h>

namespace owo::tsf {
class TextService;
inline constexpr UINT_PTR kZeroCandidateTimer = 0x4f574f30;

class ZeroCandidate final {
public:
    static constexpr bool kAutomaticGenerationPaused = true;
    explicit ZeroCandidate(TextService& owner) : owner_(owner) {}
    void observe();
    void poll();
    void cancel() noexcept;
    [[nodiscard]] std::wstring_view text() const noexcept;
    [[nodiscard]] bool handles(WPARAM) const noexcept;
    HRESULT choose(ITfContext* expected = nullptr);
private:
#ifdef OWO_TSF_TEST_ACCESS
    friend struct ExpandedCandidateTestAccess;
#endif
    bool eligible() const noexcept;
    bool read_position(bool first, std::wstring* surroundings = nullptr);
    bool refresh_mode(bool force = false);
    void invalidate();
    struct Job {
        std::uint64_t generation{};
        bool succeeded{};
        std::wstring result;
        std::atomic<bool> finished{false};
    };
    TextService& owner_;
    std::string mode_;
    std::wstring input_, application_, text_;
    Microsoft::WRL::ComPtr<ITfContext> context_;
    Microsoft::WRL::ComPtr<ITfRange> anchor_;
    EditWindowIdentity window_{};
    std::uint64_t generation_{}, composition_generation_{}, next_mode_check_{}, deadline_{}, next_request_{}, hide_at_{};
    bool submitted_{};
    std::shared_ptr<Job> job_;
    std::jthread worker_;
};
}

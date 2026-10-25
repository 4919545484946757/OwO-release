#pragma once

#include <optional>
#include <string>
#include <string_view>
#include <utility>

namespace owo::tsf {

struct CompositionLearningRecord {
    std::wstring text;
    std::wstring input;
    std::wstring context;
};

// One chain belongs to one preedit composition, not to a document or application.
// Only successful commits are appended. A finished chain is consumed exactly once.
class CompositionLearningChain {
  public:
    [[nodiscard]] bool empty() const noexcept { return record_.text.empty(); }
    [[nodiscard]] std::wstring_view text() const noexcept { return record_.text; }
    void clear() { record_ = {}; }

    [[nodiscard]] std::optional<CompositionLearningRecord> append(std::wstring_view text,
                                                                  std::wstring_view input,
                                                                  std::wstring_view context,
                                                                  bool finished) {
        if (empty())
            record_.context = context;
        record_.text.append(text);
        for (const auto character : input)
            if (character != L'\'')
                record_.input.push_back(character);
        if (!finished)
            return std::nullopt;
        auto completed = std::move(record_);
        clear();
        if (completed.text.empty() || completed.input.empty())
            return std::nullopt;
        return completed;
    }

  private:
    CompositionLearningRecord record_;
};

} // namespace owo::tsf

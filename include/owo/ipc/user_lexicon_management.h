#pragma once
#include "owo/engine/user_frequency.h"
#include <string>
#include <string_view>
namespace owo::ipc {
struct UserLexiconManagementResult {
    bool ok{};
    bool mutated{};
    std::string text;
};
// Called exclusively by the Core owner thread; all writes are staged and flushed.
[[nodiscard]] UserLexiconManagementResult manage_user_lexicon(engine::UserFrequencyStore &store,
                                                              std::string_view request,
                                                              std::uint64_t &revision);
} // namespace owo::ipc

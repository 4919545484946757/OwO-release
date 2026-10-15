#include "owo/engine/language_model.h"

#include <utility>

namespace owo::engine {
namespace {

std::string key(const std::string_view previous, const std::string_view current) {
    std::string value(previous);
    value.push_back('\0');
    value.append(current);
    return value;
}

}  // namespace

void MemoryBigramModel::set(std::string previous, std::string current, const std::int64_t score) {
    scores_.insert_or_assign(key(previous, current), score);
}

std::int64_t MemoryBigramModel::score(const std::string_view previous,
                                      const std::string_view current) const {
    const auto found = scores_.find(key(previous, current));
    return found == scores_.end() ? 0 : found->second;
}

}  // namespace owo::engine

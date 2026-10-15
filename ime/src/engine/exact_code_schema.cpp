#include "owo/engine/exact_code_schema.h"

#include <algorithm>
#include <utility>

namespace owo::engine {

ParseResult ExactCodeSchema::parse(const std::string_view input,
                                   const std::size_t max_paths) const {
    ParseResult result;
    result.scheme = InputSchemeKind::exact_code;
    result.phonetic_assistance = false;
    if (input.empty() || max_paths == 0 || input.front() == '\'' ||
        input.back() == '\'')
        return result;

    result.normalized_input.reserve(input.size());
    for (const auto value : input) {
        const bool lower = value >= 'a' && value <= 'z';
        const bool upper = value >= 'A' && value <= 'Z';
        if (value != '\'' && !lower && !upper) return result;
        result.normalized_input.push_back(value == '\'' || lower
            ? value : static_cast<char>(value - 'A' + 'a'));
    }

    ParsePath path;
    std::size_t begin = 0;
    while (begin < result.normalized_input.size()) {
        const auto end = result.normalized_input.find('\'', begin);
        const auto segment_end = end == std::string::npos
                                     ? result.normalized_input.size() : end;
        if (segment_end == begin) {
            result.paths.clear();
            return result;
        }
        path.syllables.push_back({
            result.normalized_input.substr(begin, segment_end - begin),
            begin, segment_end, true});
        if (end == std::string::npos) break;
        begin = end + 1;
    }
    path.match_kind = InputMatchKind::exact;
    result.paths.push_back(std::move(path));
    result.valid = true;
    return result;
}

}  // namespace owo::engine

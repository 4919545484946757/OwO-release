#pragma once
#include <algorithm>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace owo::engine {
// Bounded request-local index of dictionary-backed word-pair evidence.
// Packed text storage avoids a pair of std::strings for every cached record.
class CompactCollocations {
    struct Record { std::uint32_t offset; std::uint16_t left, right; std::int16_t score; };
    std::string text_;
    std::vector<Record> records_;
public:
    template<class Evidence> std::int64_t score(std::string_view left, std::string_view right, Evidence evidence) {
        if (left.empty() || right.empty() || left.size() > 48 || right.size() > 48) return 0;
        const auto key = std::pair{left, right};
        const auto view = [&](const Record& value) {
            return std::pair{std::string_view(text_).substr(value.offset, value.left),
                std::string_view(text_).substr(value.offset + value.left, value.right)};
        };
        const auto found = std::lower_bound(records_.begin(), records_.end(), key,
            [&](const auto& record, const auto& sought) { return view(record) < sought; });
        if (found != records_.end() && view(*found) == key) return found->score;
        if (records_.size() == 256) return 0;
        const auto position = static_cast<std::size_t>(found - records_.begin());
        const auto value = static_cast<std::int16_t>(evidence() ? 1000 : 0);
        const Record record{static_cast<std::uint32_t>(text_.size()),
            static_cast<std::uint16_t>(left.size()), static_cast<std::uint16_t>(right.size()), value};
        text_.append(left); text_.append(right);
        records_.insert(records_.begin() + position, record);
        return value;
    }
};
} // namespace owo::engine

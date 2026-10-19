#pragma once

#include <cstdint>
#include <string>
#include <string_view>
#include <unordered_map>

namespace owo::engine {

class BigramModel {
public:
    virtual ~BigramModel() = default;
    [[nodiscard]] virtual std::int64_t score(std::string_view previous,
                                             std::string_view current) const = 0;
};

class MemoryBigramModel final : public BigramModel {
public:
    void set(std::string previous, std::string current, std::int64_t score);
    [[nodiscard]] std::int64_t score(std::string_view previous,
                                     std::string_view current) const override;

private:
    std::unordered_map<std::string, std::int64_t> scores_;
};

}  // namespace owo::engine

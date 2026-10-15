#pragma once

#include "owo/engine/input_schema.h"

namespace owo::engine {

// A scheme-neutral, exact segmented-code parser. It deliberately performs no
// phonetic segmentation, completion, abbreviation expansion or correction.
// The active lexicon decides what each apostrophe-separated code means.
class ExactCodeSchema final : public InputSchema {
public:
    [[nodiscard]] ParseResult parse(std::string_view input,
                                    std::size_t max_paths = 32) const override;
};

}  // namespace owo::engine

#include "owo/engine/expressive_candidates.h"

#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

bool is_emoji_candidate(const std::string_view text) {
    return text.size() >= 4 &&
           static_cast<unsigned char>(text.front()) >= 0xF0;
}

}  // namespace

int main() {
    const auto& source = owo::engine::builtin_expressive_source();

    if (owo::engine::expressive_lookup_key(" Lei  Si\t") != "leisi")
        return fail("expressive lookup key normalization failed");

    {
        const auto assets = source.lookup("leisi", 1);
        if (assets.size() != 1 ||
            assets.front().kind != owo::engine::ExpressiveAssetKind::emoji ||
            assets.front().text.empty() || assets.front().annotation.empty() ||
            !assets.front().local_only)
            return fail("builtin expressive lookup returned an unexpected asset");
    }
    if (!source.lookup("leisi", 0).empty())
        return fail("zero expressive limit still returned assets");
    if (!source.lookup("nihao", 3).empty())
        return fail("unrelated pinyin matched an expressive key");

    {
        const auto candidates =
            owo::engine::generate_expressive_candidates("leisi");
        if (candidates.size() != 2)
            return fail("expressive candidate count is wrong");
        if (!is_emoji_candidate(candidates.front().text))
            return fail("emoji candidate must lead the expressive group");
        if (candidates.front().score <= candidates.back().score)
            return fail("emoji and kaomoji scores are not ranked");
        for (const auto& candidate : candidates)
            if (candidate.consumed_input_bytes != 5 ||
                candidate.model_only || candidate.text.empty())
                return fail("expressive candidate fields are wrong");
    }
    {
        const auto candidates =
            owo::engine::generate_expressive_candidates("Lei Si");
        if (candidates.size() != 2)
            return fail("spaced/cased expressive key was not normalized");
    }
    if (!owo::engine::generate_expressive_candidates("nihao").empty())
        return fail("ordinary pinyin produced expressive candidates");
    {
        const auto candidates =
            owo::engine::generate_expressive_candidates("ganbei");
        if (candidates.size() != 2 ||
            candidates.front().text != "🍻" ||
            candidates.back().text != "( ゜- ゜)つロ")
            return fail("expanded expressive table entry is wrong");
    }
    if (owo::engine::generate_expressive_candidates("shengrikuaile").empty())
        return fail("birthday key did not resolve");
    if (!owo::engine::generate_expressive_candidates("zzzznotakey").empty())
        return fail("unknown key produced expressive candidates");
    {
        const auto candidates =
            owo::engine::generate_expressive_candidates("leisi", source, 1);
        if (candidates.size() != 1)
            return fail("expressive limit was not applied");
    }

    std::cout << "expressive candidate tests passed\n";
    return 0;
}

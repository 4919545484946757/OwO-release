#pragma once

#include "owo/engine/input_method_provider.h"

#include <array>
#include <string_view>

namespace owo::example {
inline engine::InputMethodDescriptor table_descriptor(std::string plugin_id) {
    engine::InputMethodDescriptor d;
    d.plugin_id = std::move(plugin_id);
    d.id = d.plugin_id + "/table";
    d.name = "Experimental table example";
    d.language_tag = "zh-Hans";
    d.alphabet = U"abcdefghijklmnopqrstuvwxyz";
    return d;
}
inline engine::InputMethodResponse table_response(const engine::InputMethodRequest& r) {
    engine::InputMethodResponse v;
    v.request_id = r.request_id; v.session_id = r.session_id; v.scheme_id = r.scheme_id;
    v.page = r.page; v.preview = r.input;
    if (r.action == engine::InputMethodAction::cancel) {
        v.status = engine::InputMethodStatus::cancelled;
        return v;
    }
    // A tiny illustrative table, not a complete Cangjie/Wubi dictionary.
    constexpr std::array<std::pair<std::string_view, std::string_view>, 4> table{{
        {"a", "日"}, {"aa", "昌"}, {"ab", "明"}, {"b", "月"}
    }};
    if (r.input.empty()) return v;
    std::size_t index = 0;
    const auto start = r.page * r.page_size;
    for (const auto& [code, text] : table) {
        if (!code.starts_with(r.input)) continue;
        if (index++ < start) continue;
        if (v.candidates.size() == r.page_size) { v.has_more = true; break; }
        v.candidates.push_back({std::string(code), std::string(text), std::string(code), {{0, r.input.size()}}});
    }
    return v;
}
} // namespace owo::example

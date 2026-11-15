#pragma once

#include "owo/engine/ranking_context.h"
#include "owo/tsf/input_method_context.h"

#include <optional>
#include <wrl/client.h>

namespace owo::tsf {

// Inspect at most the scalar immediately before a collapsed caret, and only in
// a verified context. Never return application text to the ranking pipeline.
inline std::optional<bool> preceding_numeric_symbol(ITfContext* context, TfEditCookie cookie,
                                                     InputMethodContextSafety safety) {
    if (!context || safety != InputMethodContextSafety::allowed) return std::nullopt;
    TF_SELECTION selected{};
    ULONG picked{};
    const auto result = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selected, &picked);
    Microsoft::WRL::ComPtr<ITfRange> selection;
    selection.Attach(selected.range);
    BOOL empty{};
    if (FAILED(result) || picked != 1 || !selection || FAILED(selection->IsEmpty(cookie, &empty)))
        return std::nullopt;
    if (!empty) return false;

    Microsoft::WRL::ComPtr<ITfRange> before;
    LONG moved{};
    if (FAILED(selection->Clone(&before)) || !before ||
        FAILED(before->ShiftStart(cookie, -2, &moved, nullptr))) return std::nullopt;
    if (moved == 0) return false;
    wchar_t units[4]{}; // Space for two surrogate pairs if this host shifts by scalar.
    ULONG count{};
    if (FAILED(before->GetText(cookie, 0, units, 4, &count)) || count == 0 || count > 4)
        return std::nullopt;
    return engine::ranking_context_ends_in_number(std::wstring_view(units, count));
}

} // namespace owo::tsf

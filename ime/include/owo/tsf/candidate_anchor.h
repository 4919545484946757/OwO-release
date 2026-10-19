#pragma once
#include <windows.h>

namespace owo::tsf {
// Some hosts return S_OK with a zero-height sentinel at the screen edge.
// A caret may be zero-width, and negative coordinates are valid on other monitors.
inline bool candidate_text_extent_usable(const RECT& bounds) noexcept {
    return bounds.right >= bounds.left && bounds.bottom > bounds.top;
}
} // namespace owo::tsf

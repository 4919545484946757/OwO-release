#include "owo/tsf/unicode_palette_scroll.h"

int main() {
    using owo::tsf::unicode_category_scroll_offset;
    // The visible palette is narrower than the retained anti-flicker surface.
    constexpr float content = 424, surface = 1100, categories = 900, padding = 12;
    if (unicode_category_scroll_offset(0, 48, categories, content, padding) != 48) return 1;
    if (unicode_category_scroll_offset(0, 48, categories, surface, padding) != 0) return 2;
    if (unicode_category_scroll_offset(0, 5000, categories, content, padding) != 500) return 3;
    if (unicode_category_scroll_offset(500, -48, categories, content, padding) != 452) return 4;
    if (unicode_category_scroll_offset(30, -100, categories, content, padding) != 0) return 5;
    // Partial wheel deltas stay continuous, and resizing clamps stale offsets.
    if (unicode_category_scroll_offset(48, 12, categories, content, padding) != 60) return 6;
    if (unicode_category_scroll_offset(500, 0, categories, 800, padding) != 124) return 7;
    if (unicode_category_scroll_offset(500, 0, 100, content, padding) != 0) return 8;
    // DPI conversion must yield the same logical viewport.
    if (unicode_category_scroll_offset(0, 48, categories, 636.0F / 1.5F, padding) != 48) return 9;
    return 0;
}

#include "owo/tsf/expanded_candidate_pages.h"

#include <climits>
#include <iostream>
#include <limits>

int main() {
    using owo::tsf::ExpandedCandidatePages;
    const std::vector<std::uint64_t> sparse{0, 0, 1, 1, 2, 0, 0};
    const auto boundaries = owo::protocol::candidate_page_boundaries(sparse.size(), 9, sparse);
    if (boundaries != std::vector<std::size_t>{0, 5, 7}) return 11;
    ExpandedCandidatePages sparse_pages;
    sparse_pages.set_layout(sparse.size(), 9, sparse);
    sparse_pages.move(1, sparse.size(), 9, 5);
    if (sparse_pages.selected != 1 || sparse_pages.index(0, sparse.size(), 9) != 5 ||
        sparse_pages.index(1, sparse.size(), 9) != 6 || sparse_pages.index(2, sparse.size(), 9) ||
        sparse_pages.begin(1, sparse.size(), 9) != 5 || sparse_pages.end(1, sparse.size(), 9) != 7 ||
        sparse_pages.page_for(6, sparse.size(), 9) != 1) return 12;
    sparse_pages.scroll(-1, sparse.size(), 9, 1);
    if (sparse_pages.selected != 0 || sparse_pages.first_visible != 0) return 13;
    for (const auto size : {4U, 5U, 7U, 9U}) {
        const std::vector<std::uint64_t> mixed{0, 0, 1, 2, 2, 0, 0, 1, 2, 0, 1};
        const auto pages = owo::protocol::candidate_page_boundaries(mixed.size(), size, mixed);
        for (std::size_t page = 0; page + 1 < pages.size(); ++page) {
            if (pages[page + 1] - pages[page] > size ||
                std::count(mixed.begin() + pages[page], mixed.begin() + pages[page + 1], 0) > 2) return 14;
        }
    }
    for (std::size_t size = 1; size <= 9; ++size) {
        for (std::size_t count = 0; count <= 256; ++count) {
            ExpandedCandidatePages state;
            const auto pages = state.page_count(count, size);
            for (std::size_t n = 0; n < pages + 3; ++n) {
                state.move(1, count, size, 5);
                if (state.selected >= (pages ? pages : 1) || state.selected < state.first_visible ||
                    state.selected - state.first_visible >= 5) return 1;
                for (std::size_t slot = 0; slot < 10; ++slot) {
                    const auto index = state.index(slot, count, size);
                    const bool valid = slot < size && state.selected * size + slot < count;
                    if (index.has_value() != valid || (index && *index != state.selected * size + slot)) return 2;
                }
            }
            state.move(-INT_MAX, count, size, 5);
            if (state.selected != 0 || state.first_visible != 0) return 3;
            state.scroll(INT_MAX, count, size, 5);
            if (state.selected < state.first_visible || state.selected - state.first_visible >= 5) return 4;
            state.scroll(INT_MIN, count, size, 5);
            if (state.first_visible != 0 || state.selected >= 5) return 5;
        }
    }
    ExpandedCandidatePages pending;
    if (pending.block_size(256, 7) != 252 || pending.block_size(4, 7) != 4 ||
        pending.block_size(1, 9) != 1 || pending.block_size(256, 1) != 256) return 10;
    pending.reset(3); // 从折叠第 4 页展开，等待全列表期间继续向下两次。
    pending.move_requested(2);
    pending.ensure_visible(63, 7, 5);
    if (pending.selected != 5 || pending.index(1, 63, 7) != 36) return 6;
    pending.ensure_visible(15, 7, 5); // 新结果缩短，尾页只剩 1 个候选。
    if (pending.selected != 2 || pending.index(0, 15, 7) != 14 || pending.index(1, 15, 7)) return 7;
    pending.ensure_visible(0, 7, 5);
    if (pending.selected || pending.first_visible || pending.index(0, 0, 7)) return 8;
    pending.reset(std::numeric_limits<std::size_t>::max());
    pending.move_requested(INT_MAX);
    pending.ensure_visible(256, 1, 1);
    if (pending.selected != 255 || pending.first_visible != 255) return 9;
    std::cout << "Expanded page selection, viewport, deferred target and numeric mapping passed.\n";
}

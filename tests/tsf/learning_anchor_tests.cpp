#include "owo/tsf/learning_anchor.h"
#include "owo/tsf/composition_learning.h"

#include <algorithm>
#include <iostream>

namespace {
struct Range {
    std::wstring *document{};
    LONG begin{}, end{};
    bool owned{}, clone_fails{}, shift_fails{}, read_fails{};
    static inline int clones{};
    HRESULT CompareStart(TfEditCookie, Range *other, TfAnchor anchor, LONG *result) {
        if (document != other->document)
            return E_INVALIDARG;
        *result = begin - (anchor == TF_ANCHOR_START ? other->begin : other->end);
        return S_OK;
    }
    HRESULT CompareEnd(TfEditCookie, Range *other, TfAnchor anchor, LONG *result) {
        if (document != other->document)
            return E_INVALIDARG;
        *result = end - (anchor == TF_ANCHOR_START ? other->begin : other->end);
        return S_OK;
    }
    HRESULT Clone(Range **output) {
        *output = nullptr;
        if (clone_fails)
            return E_FAIL;
        *output = new Range(*this);
        (*output)->owned = true;
        ++clones;
        return S_OK;
    }
    HRESULT ShiftStart(TfEditCookie, LONG requested, LONG *moved, const void *) {
        if (shift_fails)
            return E_FAIL;
        const auto next = std::clamp(begin + requested, 0L, end);
        *moved = next - begin;
        begin = next;
        return S_OK;
    }
    HRESULT GetText(TfEditCookie, DWORD, WCHAR *output, ULONG maximum, ULONG *read) {
        if (read_fails)
            return E_FAIL;
        *read = std::min(maximum, static_cast<ULONG>(end - begin));
        std::copy_n(document->data() + begin, *read, output);
        return S_OK;
    }
    void Release() {
        if (owned) {
            --clones;
            delete this;
        }
    }
};
} // namespace

int main() {
    using owo::tsf::learning_range_is_contiguous;
    std::wstring document = L"已有上下文董柯";
    const auto end = static_cast<LONG>(document.size());
    Range caret{&document, end, end}, anchor = caret;
    int errors{};
    const auto check = [&](bool value, const char *label) {
        if (!value) {
            ++errors;
            std::cerr << label << '\n';
        }
        if (Range::clones) {
            ++errors;
            std::cerr << "Leaked range clone\n";
        }
    };
    check(learning_range_is_contiguous(&caret, &anchor, 0, L"董柯"),
          "Adjacent own prefix rejected");
    auto moved = caret;
    --moved.begin;
    --moved.end;
    check(!learning_range_is_contiguous(&moved, &anchor, 0, L"董柯"),
          "Moved caret retained old chain");
    auto selected = caret;
    --selected.begin;
    check(!learning_range_is_contiguous(&selected, &anchor, 0, L"董柯"),
          "Replacement selection retained old chain");
    std::wstring other = document;
    Range other_caret{&other, end, end};
    check(!learning_range_is_contiguous(&other_caret, &anchor, 0, L"董柯"),
          "Different document retained old chain");
    document.back() = L'可';
    check(!learning_range_is_contiguous(&caret, &anchor, 0, L"董柯"),
          "Modified committed text retained old chain");
    document.back() = L'柯';
    auto inserted = caret;
    document += L"外部粘贴";
    inserted.begin = inserted.end = static_cast<LONG>(document.size());
    check(!learning_range_is_contiguous(&inserted, &anchor, 0, L"董柯"),
          "External paste retained old chain");
    std::wstring deleted = L"已有上下文";
    Range after_delete{&deleted, static_cast<LONG>(deleted.size()),
                       static_cast<LONG>(deleted.size())};
    check(!learning_range_is_contiguous(&after_delete, &after_delete, 0, L"董柯"),
          "Deleted own prefix retained old chain");
    for (int failure = 0; failure < 3; ++failure) {
        auto unavailable = anchor;
        unavailable.clone_fails = failure == 0;
        unavailable.shift_fails = failure == 1;
        unavailable.read_fails = failure == 2;
        check(!learning_range_is_contiguous(&caret, &unavailable, 0, L"董柯"),
              "Unavailable anchor did not fail closed");
    }
    check(!learning_range_is_contiguous(&caret, static_cast<Range *>(nullptr), 0, L"董柯"),
          "Missing anchor authorized joining");
    check(!learning_range_is_contiguous(&caret, &anchor, 0, std::wstring(1025, L'字')),
          "Unbounded prefix read authorized");

    // Exercise the same decision that TSF applies before adding the next commit.
    for (const bool continuous : {true, false}) {
        owo::tsf::CompositionLearningChain chain;
        static_cast<void>(chain.append(L"董柯", L"dongke", L"外部上下文", false));
        if (!continuous)
            chain.clear();
        const auto result = chain.append(L"伊", L"yi", L"另一个位置的上下文", true);
        check(result && result->text == (continuous ? L"董柯伊" : L"伊") &&
                  result->input == (continuous ? L"dongkeyi" : L"yi") &&
                  result->context == (continuous ? L"外部上下文" : L"另一个位置的上下文"),
              "Learning text/input/context boundaries mixed");
    }
    std::cout << "Learning anchor continuity, edited text and fail-closed checks complete\n";
    return errors ? 1 : 0;
}

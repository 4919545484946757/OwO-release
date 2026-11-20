#pragma once

#include <Windows.h>
#include <msctf.h>

#include <memory>
#include <string>
#include <string_view>

namespace owo::tsf {

// Range is ITfRange in production. A small range adapter permits deterministic
// tests of the same code without reading a user's document or driving a GUI.
template <typename Range>
[[nodiscard]] bool learning_range_is_contiguous(Range *selection, Range *previous_end,
                                                TfEditCookie cookie,
                                                std::wstring_view expected_prefix) {
    if (!selection || !previous_end || expected_prefix.empty() || expected_prefix.size() > 1024)
        return false;
    LONG start{}, end{};
    if (FAILED(selection->CompareStart(cookie, previous_end, TF_ANCHOR_START, &start)) ||
        FAILED(selection->CompareEnd(cookie, previous_end, TF_ANCHOR_END, &end)) || start || end)
        return false;
    Range *copy = nullptr;
    if (FAILED(previous_end->Clone(&copy)) || !copy)
        return false;
    const auto release = [](Range *range) { range->Release(); };
    std::unique_ptr<Range, decltype(release)> prefix(copy, release);
    LONG moved{};
    const auto length = static_cast<LONG>(expected_prefix.size());
    if (FAILED(prefix->ShiftStart(cookie, -length, &moved, nullptr)) || moved != -length)
        return false;
    std::wstring actual(expected_prefix.size() + 1, L'\0');
    ULONG read{};
    if (FAILED(
            prefix->GetText(cookie, 0, actual.data(), static_cast<ULONG>(actual.size()), &read)) ||
        read != expected_prefix.size())
        return false;
    actual.resize(read);
    return actual == expected_prefix;
}

} // namespace owo::tsf

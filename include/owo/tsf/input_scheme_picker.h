#pragma once

#include <cstddef>
#include <optional>

namespace owo::tsf {

// Transport-independent tap state. The hook, normal TSF events and preserved keys
// may report the same physical press; only the first report moves the highlight.
class InputSchemePicker {
public:
    bool tap(std::size_t count, std::optional<std::size_t> active) noexcept {
        if (latched_ || count == 0) return false;
        latched_ = true;
        if (!selected_) selected_ = active ? (*active + 1) % count : 0;
        else selected_ = (*selected_ + 1) % count;
        return true;
    }
    void release_space() noexcept { latched_ = false; }
    bool showing() const noexcept { return selected_.has_value(); }
    bool space_latched() const noexcept { return latched_; }
    std::optional<std::size_t> selection() const noexcept { return selected_; }
    void close() noexcept { selected_.reset(); }
    void reset() noexcept { close(); release_space(); }
private:
    std::optional<std::size_t> selected_;
    bool latched_{};
};

} // namespace owo::tsf

#pragma once
#include "owo/engine/input_method_composition.h"
#include "owo/ipc/input_method_client.h"

namespace owo::tsf {
// UI-thread lifecycle gate. Generation comes from trusted TSF document events, not IPC.
// Does not retain COM pointers or read text; binding to actual TSF events is a later step.
class InputMethodDocumentSession final {
public:
    [[nodiscard]] bool enter(std::uint64_t generation) {
        if (!generation || generation <= last_generation_) return false;
        composition_.close(); bound_ = false; generation_ = last_generation_ = generation; return true;
    }
    void invalidate() { generation_ = 0; bound_ = false; composition_.close(); }
    [[nodiscard]] bool bind(const ipc::InputMethodClientResult& result, std::string session) {
        if (!generation_ || bound_ || result.document_generation != generation_ || !result.status ||
            result.operation != ipc::InputMethodClientOperation::open ||
            result.descriptors.size() != 1 || !composition_.begin(result.descriptors.front(), std::move(session))) return false;
        bound_ = true; return true;
    }
    [[nodiscard]] engine::InputMethodApplyResult apply(const ipc::InputMethodClientResult& result) {
        if (!generation_ || result.document_generation != generation_ || !result.status || !result.response ||
            result.operation != ipc::InputMethodClientOperation::submit) return {};
        return composition_.apply(*result.response);
    }
    [[nodiscard]] bool finish_commit(std::uint64_t generation, bool committed) {
        return generation_ && generation == generation_ && composition_.finish_commit(committed);
    }
    [[nodiscard]] engine::InputMethodComposition& composition() { return composition_; }
    [[nodiscard]] std::uint64_t generation() const { return generation_; }
private:
    std::uint64_t generation_{}, last_generation_{};
    bool bound_{};
    engine::InputMethodComposition composition_;
};
} // namespace owo::tsf

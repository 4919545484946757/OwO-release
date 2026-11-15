#pragma once

#include "owo/tsf/input_method_context.h"

#include <array>
#include <optional>
#include <string_view>
#include <wrl/client.h>

namespace owo::tsf {

// The application owns Undo. These bounded fingerprints are checked in TSF
// read locks on both sides of the application's command; they never write.
class CandidateUndoEvidence {
    using Range = Microsoft::WRL::ComPtr<ITfRange>;
    static constexpr LONG max_span = 1024;
    static constexpr LONG flank = 12;
    struct Fingerprint {
        ULONG length{};
        std::uint64_t hash{14695981039346656037ULL};
        bool operator==(const Fingerprint&) const = default;
    };
    Range start_;
    EditWindowIdentity window_;
    Fingerprint previous_, committed_, left_, right_;
    std::optional<LONG> offset_, committed_document_end_;

    static Fingerprint digest(std::wstring_view value) {
        Fingerprint result{static_cast<ULONG>(value.size())};
        for (const auto unit : value)
            result.hash = (result.hash ^ static_cast<std::uint16_t>(unit)) * 1099511628211ULL;
        return result;
    }
    static std::optional<Fingerprint> read(ITfRange* range, TfEditCookie cookie, LONG limit) {
        std::array<wchar_t, max_span + 1> buffer{};
        ULONG count{};
        if (FAILED(range->GetText(cookie, 0, buffer.data(), static_cast<ULONG>(limit+1), &count)) ||
            count > static_cast<ULONG>(limit)) return {};
        return digest({buffer.data(), count});
    }
    static std::optional<LONG> position(ITfRange* range) {
        Microsoft::WRL::ComPtr<ITfRangeACP> acp;
        LONG offset{}, length{};
        if (FAILED(range->QueryInterface(IID_PPV_ARGS(&acp))) ||
            FAILED(acp->GetExtent(&offset, &length))) return {};
        return offset;
    }
    static std::optional<LONG> document_end(ITfContext* context, TfEditCookie cookie) {
        Range end;
        if (FAILED(context->GetEnd(cookie, &end)) || !end) return {};
        return position(end.Get());
    }
    static std::optional<Fingerprint> side(ITfRange* range, TfEditCookie cookie, bool before) {
        Range part;
        LONG moved{};
        if (FAILED(range->Clone(&part)) ||
            FAILED(part->Collapse(cookie, before ? TF_ANCHOR_START : TF_ANCHOR_END))) return {};
        const auto status = before ? part->ShiftStart(cookie, -flank, &moved, nullptr)
                                   : part->ShiftEnd(cookie, flank, &moved, nullptr);
        return SUCCEEDED(status) ? read(part.Get(), cookie, flank) : std::nullopt;
    }
    Range span(TfEditCookie cookie, ULONG length) const {
        Range range;
        LONG moved{};
        if (FAILED(start_->Clone(&range)) || FAILED(range->Collapse(cookie, TF_ANCHOR_START)) ||
            (length && (FAILED(range->ShiftEnd(cookie, static_cast<LONG>(length), &moved, nullptr)) ||
                        moved != static_cast<LONG>(length)))) return {};
        return range;
    }
    static bool matches(ITfRange* a, TfEditCookie cookie, TfAnchor edge, ITfRange* b, TfAnchor b_edge) {
        LONG compare{};
        const auto status = edge == TF_ANCHOR_START ? a->CompareStart(cookie, b, b_edge, &compare)
                                                   : a->CompareEnd(cookie, b, b_edge, &compare);
        return SUCCEEDED(status) && compare == 0;
    }
public:
    static std::optional<CandidateUndoEvidence> capture(
        ITfRange* replaced, TfEditCookie cookie, std::wstring_view committed,
        const InputContextResolution& resolution, bool inline_preedit) {
        if (!resolution.focus || !resolution.view || committed.empty() || committed.size() > max_span) return {};
        CandidateUndoEvidence proof;
        const auto previous = inline_preedit ? std::optional{Fingerprint{}} : read(replaced, cookie, max_span);
        const auto left = side(replaced, cookie, true);
        const auto right = side(replaced, cookie, false);
        if (!previous || !left || !right || FAILED(replaced->Clone(&proof.start_)) ||
            FAILED(proof.start_->Collapse(cookie, TF_ANCHOR_START)) ||
            FAILED(proof.start_->SetGravity(cookie, TF_GRAVITY_BACKWARD, TF_GRAVITY_BACKWARD))) return {};
        proof.window_ = edit_window_identity(resolution);
        proof.offset_ = position(proof.start_.Get());
        proof.previous_ = *previous;
        proof.committed_ = digest(committed);
        proof.left_ = *left;
        proof.right_ = *right;
        return proof;
    }
    void finish(ITfContext* context, TfEditCookie cookie) {
        committed_document_end_ = document_end(context, cookie);
    }
    bool matches_document(ITfContext* context, TfEditCookie cookie, bool undone) const {
        const auto resolution = resolve_input_context(context, cookie);
        if (!resolution.ordinary_input || window_ != edit_window_identity(resolution) ||
            (offset_ && position(start_.Get()) != offset_)) return false;
        const auto& fingerprint = undone ? previous_ : committed_;
        if (committed_document_end_) {
            const auto expected = static_cast<std::int64_t>(*committed_document_end_) +
                fingerprint.length - committed_.length;
            const auto actual = document_end(context, cookie);
            if (!actual || *actual != expected) return false;
        }
        const auto range = span(cookie, fingerprint.length);
        if (!range || read(range.Get(), cookie, static_cast<LONG>(fingerprint.length)) != fingerprint ||
            side(range.Get(), cookie, true) != left_ || side(range.Get(), cookie, false) != right_) return false;
        TF_SELECTION current{};
        ULONG count{};
        const auto result = context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &current, &count);
        Range selection;
        selection.Attach(current.range);
        // If Undo restores an originally selected span, require the application
        // to select it again; otherwise re-choosing would insert a duplicate.
        return SUCCEEDED(result) && count == 1 && selection && !current.style.fInterimChar &&
            matches(selection.Get(), cookie, TF_ANCHOR_START, range.Get(), undone ? TF_ANCHOR_START : TF_ANCHOR_END) &&
            matches(selection.Get(), cookie, TF_ANCHOR_END, range.Get(), TF_ANCHOR_END);
    }
};
} // namespace owo::tsf

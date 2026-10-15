#include "owo/engine/input_method_provider.h"

#include <algorithm>
#include <exception>
#include <unordered_set>

namespace owo::engine {
namespace {
InputMethodValidation fail(std::string message) { return {false, std::move(message)}; }

bool valid_utf8(std::string_view value, const std::u32string* alphabet = nullptr) {
    for (std::size_t i = 0; i < value.size();) {
        const auto first = static_cast<unsigned char>(value[i++]);
        if (first < 0x80) {
            if (first == 0 || (alphabet && alphabet->find(first) == std::u32string::npos)) return false;
            continue;
        }
        unsigned count{}, scalar{}, minimum{};
        if (first >= 0xc2 && first <= 0xdf) { count = 1; scalar = first & 31; minimum = 0x80; }
        else if (first >= 0xe0 && first <= 0xef) { count = 2; scalar = first & 15; minimum = 0x800; }
        else if (first >= 0xf0 && first <= 0xf4) { count = 3; scalar = first & 7; minimum = 0x10000; }
        else return false;
        if (count > value.size() - i) return false;
        for (unsigned n = 0; n < count; ++n) {
            const auto next = static_cast<unsigned char>(value[i++]);
            if ((next & 0xc0) != 0x80) return false;
            scalar = (scalar << 6) | (next & 63);
        }
        if (scalar < minimum || scalar > 0x10ffff || (scalar >= 0xd800 && scalar <= 0xdfff)) return false;
        if (alphabet && alphabet->find(static_cast<char32_t>(scalar)) == std::u32string::npos) return false;
    }
    return true;
}

bool boundary(std::string_view text, std::size_t offset) {
    return offset <= text.size() && (offset == text.size() ||
        (static_cast<unsigned char>(text[offset]) & 0xc0) != 0x80);
}

bool token(std::string_view value, bool dots = false) {
    return !value.empty() && value.size() <= 128 &&
        std::all_of(value.begin(), value.end(), [dots](char c) {
            return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') ||
                c == '-' || c == '_' || (dots && c == '.');
        });
}

InputMethodResponse error_response(const InputMethodRequest& request,
                                   InputMethodStatus status, std::string diagnostic) {
    InputMethodResponse response;
    response.request_id = request.request_id;
    response.session_id = request.session_id;
    response.scheme_id = request.scheme_id;
    response.page = request.page;
    response.status = status;
    response.diagnostic = std::move(diagnostic);
    return response;
}
} // namespace

InputMethodValidation validate_input_method_descriptor(const InputMethodDescriptor& value) {
    if (value.api_version != kInputMethodApiVersion) return fail("Unsupported input method API version");
    if (!token(value.plugin_id, true)) return fail("Invalid plugin ID");
    const auto prefix = value.plugin_id + "/";
    if (!value.id.starts_with(prefix) || !token(std::string_view(value.id).substr(prefix.size())))
        return fail("Scheme ID must belong to its plugin namespace");
    if (value.name.empty() || value.name.size() > 256 || !valid_utf8(value.name)) return fail("Invalid scheme name");
    if (value.language_tag.empty() || value.language_tag.size() > 64 ||
        !std::all_of(value.language_tag.begin(), value.language_tag.end(), [](char c) {
            return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
                (c >= '0' && c <= '9') || c == '-';
        })) return fail("Invalid language tag");
    if (value.kind != InputMethodKind::phonetic && value.kind != InputMethodKind::table &&
        value.kind != InputMethodKind::transliteration) return fail("Invalid scheme kind");
    if (value.max_input_bytes == 0 || value.max_input_bytes > 16384 ||
        value.max_candidates == 0 || value.max_candidates > 256) return fail("Invalid scheme limits");
    if (value.alphabet.empty() || value.alphabet.size() > 1024) return fail("Invalid scheme alphabet");
    std::unordered_set<char32_t> seen;
    for (auto c : value.alphabet)
        if (c == 0 || c > 0x10ffff || (c >= 0xd800 && c <= 0xdfff) || !seen.insert(c).second)
            return fail("Alphabet contains invalid or duplicate characters");
    return {true, {}};
}

InputMethodValidation validate_input_method_request(const InputMethodDescriptor& descriptor,
                                                    const InputMethodRequest& request) {
    if (request.scheme_id != descriptor.id || request.request_id == 0 ||
        request.session_id.empty() || request.session_id.size() > 128 || !valid_utf8(request.session_id))
        return fail("Invalid request identity");
    if (request.input.size() > descriptor.max_input_bytes || !valid_utf8(request.input, &descriptor.alphabet) ||
        !boundary(request.input, request.caret_byte)) return fail("Invalid input or caret");
    if (request.page_size == 0 || request.page_size > descriptor.max_candidates || request.page > 1000000)
        return fail("Invalid candidate page");
    if (request.action != InputMethodAction::compose && request.action != InputMethodAction::select &&
        request.action != InputMethodAction::cancel) return fail("Invalid request action");
    if ((request.action == InputMethodAction::select) != !request.selected_candidate_id.empty() ||
        request.selected_candidate_id.size() > 128 || !valid_utf8(request.selected_candidate_id))
        return fail("Invalid candidate selection");
    return {true, {}};
}

InputMethodValidation validate_input_method_response(const InputMethodDescriptor& descriptor,
                                                     const InputMethodRequest& request,
                                                     const InputMethodResponse& response) {
    if (response.request_id != request.request_id || response.session_id != request.session_id ||
        response.scheme_id != request.scheme_id || response.page != request.page)
        return fail("Response identity or page mismatch");
    if (response.status != InputMethodStatus::success && response.status != InputMethodStatus::cancelled &&
        response.status != InputMethodStatus::unavailable && response.status != InputMethodStatus::error)
        return fail("Invalid response status");
    if (response.preview.size() > 16384 || !valid_utf8(response.preview) ||
        response.diagnostic.size() > 1024 || !valid_utf8(response.diagnostic)) return fail("Invalid response text");
    if (response.candidates.size() > request.page_size || response.candidates.size() > descriptor.max_candidates)
        return fail("Too many candidates");
    if ((response.status != InputMethodStatus::success || request.action == InputMethodAction::cancel) &&
        (!response.candidates.empty() || response.has_more)) return fail("Candidates on non-success response");
    std::unordered_set<std::string> ids;
    std::size_t bytes = response.preview.size() + response.diagnostic.size();
    for (const auto& candidate : response.candidates) {
        if (candidate.id.empty() || candidate.id.size() > 128 || !valid_utf8(candidate.id) ||
            !ids.insert(candidate.id).second || candidate.text.empty() || candidate.text.size() > 4096 ||
            !valid_utf8(candidate.text) || candidate.annotation.size() > 1024 || !valid_utf8(candidate.annotation))
            return fail("Invalid candidate text or ID");
        if (candidate.consumed_ranges.empty() || candidate.consumed_ranges.size() > 64)
            return fail("Missing or excessive consumed ranges");
        std::size_t previous_end = 0;
        for (const auto range : candidate.consumed_ranges) {
            if (range.begin >= range.end || range.begin < previous_end ||
                !boundary(request.input, range.begin) || !boundary(request.input, range.end))
                return fail("Invalid consumed input range");
            previous_end = range.end;
        }
        bytes += candidate.id.size() + candidate.text.size() + candidate.annotation.size() +
            candidate.consumed_ranges.size() * 16;
        if (bytes > 128 * 1024) return fail("Response payload limit exceeded");
    }
    return {true, {}};
}

InputMethodValidation InputMethodRegistry::add(InputMethodDescriptor descriptor,
                                             std::shared_ptr<InputMethodProvider> provider) {
    auto validation = validate_input_method_descriptor(descriptor);
    if (!validation.ok) return validation;
    if (!provider) return fail("Missing input method provider");
    std::lock_guard lock(mutex_);
    const auto id = descriptor.id;
    if (entries_.contains(id)) return fail("Scheme already registered");
    entries_.emplace(id, Entry{std::move(descriptor), std::move(provider), next_generation_++});
    return {true, {}};
}

void InputMethodRegistry::remove_plugin(std::string_view plugin_id) {
    std::lock_guard lock(mutex_);
    std::erase_if(entries_, [plugin_id](const auto& entry) { return entry.second.descriptor.plugin_id == plugin_id; });
}

std::vector<InputMethodDescriptor> InputMethodRegistry::descriptors() const {
    std::lock_guard lock(mutex_);
    std::vector<InputMethodDescriptor> values;
    for (const auto& [id, entry] : entries_) { (void)id; values.push_back(entry.descriptor); }
    std::sort(values.begin(), values.end(), [](const auto& a, const auto& b) { return a.id < b.id; });
    return values;
}

InputMethodResponse InputMethodRegistry::dispatch(const InputMethodRequest& request, std::stop_token stop) const {
    Entry entry;
    {
        std::lock_guard lock(mutex_);
        const auto found = entries_.find(request.scheme_id);
        if (found == entries_.end()) return error_response(request, InputMethodStatus::unavailable, "Scheme unavailable");
        entry = found->second;
    }
    const auto validation = validate_input_method_request(entry.descriptor, request);
    if (!validation.ok) return error_response(request, InputMethodStatus::error, validation.diagnostic);
    if (stop.stop_requested()) return error_response(request, InputMethodStatus::cancelled, {});
    try {
        auto response = entry.provider->handle(request, stop);
        if (stop.stop_requested()) return error_response(request, InputMethodStatus::cancelled, {});
        {
            std::lock_guard lock(mutex_);
            const auto found = entries_.find(request.scheme_id);
            if (found == entries_.end() || found->second.generation != entry.generation)
                return error_response(request, InputMethodStatus::unavailable, "Scheme registration changed");
        }
        const auto checked = validate_input_method_response(entry.descriptor, request, response);
        if (!checked.ok) return error_response(request, InputMethodStatus::error, checked.diagnostic);
        return response;
    } catch (...) {
        return error_response(request, InputMethodStatus::error, "Input method provider failed");
    }
}
} // namespace owo::engine

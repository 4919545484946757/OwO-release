#include "owo/plugin/input_method_protocol.h"

#include <utility>
#include <unordered_set>

namespace owo::plugin {
namespace {
using namespace engine;
struct Writer {
    std::string bytes;
    explicit Writer(std::string_view magic) : bytes(magic) {}
    void integer(std::uint64_t value, unsigned width) {
        for (unsigned i = 0; i < width; ++i) bytes.push_back(static_cast<char>(value >> (i * 8)));
    }
    void text(std::string_view value) { integer(value.size(), 4); bytes.append(value); }
};
struct Reader {
    std::string_view bytes;
    std::size_t offset{};
    bool integer(std::uint64_t& value, unsigned width) {
        if (width > bytes.size() - offset) return false;
        value = 0;
        for (unsigned i = 0; i < width; ++i)
            value |= static_cast<std::uint64_t>(static_cast<unsigned char>(bytes[offset++])) << (i * 8);
        return true;
    }
    bool size(std::size_t& value) {
        std::uint64_t raw{};
        if (!integer(raw, 4)) return false;
        value = static_cast<std::size_t>(raw);
        return true;
    }
    bool text(std::string& value, std::size_t maximum) {
        std::size_t size{};
        if (!this->size(size) || size > maximum || size > bytes.size() - offset) return false;
        value.assign(bytes.substr(offset, size));
        offset += size;
        return true;
    }
    bool header(std::string_view magic) {
        if (bytes.size() > kMaximumInputMethodWireBytes || !bytes.starts_with(magic)) return false;
        offset = magic.size();
        std::uint64_t version{};
        return integer(version, 4) && version == kInputMethodApiVersion;
    }
    bool done() const { return offset == bytes.size(); }
};
InputMethodValidation invalid() { return {false, "Invalid experimental input method payload"}; }
bool identity(Reader& reader, std::uint64_t& id, std::string& session, std::string& scheme) {
    return reader.integer(id, 8) && reader.text(session, 128) && reader.text(scheme, 257);
}
void identity(Writer& writer, std::uint64_t id, std::string_view session, std::string_view scheme) {
    writer.integer(kInputMethodApiVersion, 4);
    writer.integer(id, 8);
    writer.text(session);
    writer.text(scheme);
}
} // namespace

std::string encode_input_method_request(const engine::InputMethodDescriptor& d,
                                       const engine::InputMethodRequest& r) {
    if (!engine::validate_input_method_descriptor(d).ok ||
        !engine::validate_input_method_request(d, r).ok) return {};
    Writer writer("OWIQ");
    identity(writer, r.request_id, r.session_id, r.scheme_id);
    writer.integer(static_cast<std::uint8_t>(r.action), 1);
    writer.text(r.input);
    writer.integer(r.caret_byte, 4);
    writer.integer(r.page, 4);
    writer.integer(r.page_size, 4);
    writer.text(r.selected_candidate_id);
    return std::move(writer.bytes);
}

engine::InputMethodValidation decode_input_method_request(const engine::InputMethodDescriptor& d,
                                                          std::string_view payload,
                                                          engine::InputMethodRequest& output) {
    output = {};
    if (!engine::validate_input_method_descriptor(d).ok) return invalid();
    Reader reader{payload};
    engine::InputMethodRequest value;
    std::uint64_t action{};
    if (!reader.header("OWIQ") || !identity(reader, value.request_id, value.session_id, value.scheme_id) ||
        !reader.integer(action, 1) || !reader.text(value.input, d.max_input_bytes) ||
        !reader.size(value.caret_byte) || !reader.size(value.page) || !reader.size(value.page_size) ||
        !reader.text(value.selected_candidate_id, 128) || !reader.done()) return invalid();
    value.action = static_cast<engine::InputMethodAction>(action);
    const auto validation = engine::validate_input_method_request(d, value);
    if (validation.ok) output = std::move(value);
    return validation;
}

std::string encode_input_method_response(const engine::InputMethodDescriptor& d,
                                        const engine::InputMethodRequest& r,
                                        const engine::InputMethodResponse& v) {
    if (!engine::validate_input_method_descriptor(d).ok ||
        !engine::validate_input_method_request(d, r).ok ||
        !engine::validate_input_method_response(d, r, v).ok) return {};
    Writer writer("OWIR");
    identity(writer, v.request_id, v.session_id, v.scheme_id);
    writer.integer(static_cast<std::uint8_t>(v.status), 1);
    writer.text(v.preview);
    writer.integer(v.page, 4);
    writer.integer(v.has_more ? 1 : 0, 1);
    writer.text(v.diagnostic);
    writer.integer(v.candidates.size(), 4);
    for (const auto& c : v.candidates) {
        writer.text(c.id); writer.text(c.text); writer.text(c.annotation);
        writer.integer(c.consumed_ranges.size(), 4);
        for (const auto range : c.consumed_ranges) {
            writer.integer(range.begin, 4); writer.integer(range.end, 4);
        }
    }
    if (writer.bytes.size() > kMaximumInputMethodWireBytes) return {};
    return std::move(writer.bytes);
}

engine::InputMethodValidation decode_input_method_response(const engine::InputMethodDescriptor& d,
                                                           const engine::InputMethodRequest& r,
                                                           std::string_view payload,
                                                           engine::InputMethodResponse& output) {
    output = {};
    if (!engine::validate_input_method_descriptor(d).ok ||
        !engine::validate_input_method_request(d, r).ok) return invalid();
    Reader reader{payload};
    engine::InputMethodResponse value;
    std::uint64_t status{}, more{};
    std::size_t count{};
    if (!reader.header("OWIR") || !identity(reader, value.request_id, value.session_id, value.scheme_id) ||
        !reader.integer(status, 1) || !reader.text(value.preview, 16384) || !reader.size(value.page) ||
        !reader.integer(more, 1) || more > 1 || !reader.text(value.diagnostic, 1024) ||
        !reader.size(count) || count > r.page_size || count > d.max_candidates) return invalid();
    value.status = static_cast<engine::InputMethodStatus>(status);
    value.has_more = more != 0;
    for (std::size_t i = 0; i < count; ++i) {
        engine::InputMethodCandidate c;
        std::size_t ranges{};
        if (!reader.text(c.id, 128) || !reader.text(c.text, 4096) || !reader.text(c.annotation, 1024) ||
            !reader.size(ranges) || ranges == 0 || ranges > 64) return invalid();
        for (std::size_t j = 0; j < ranges; ++j) {
            engine::InputMethodRange range;
            if (!reader.size(range.begin) || !reader.size(range.end)) return invalid();
            c.consumed_ranges.push_back(range);
        }
        value.candidates.push_back(std::move(c));
    }
    if (!reader.done()) return invalid();
    const auto validation = engine::validate_input_method_response(d, r, value);
    if (validation.ok) output = std::move(value);
    return validation;
}
std::string encode_input_method_descriptors(std::string_view plugin_id,
    const std::vector<engine::InputMethodDescriptor>& descriptors) {
    if (descriptors.empty() || descriptors.size() > kMaximumPluginInputSchemes) return {};
    std::unordered_set<std::string> ids;
    Writer writer("OWID");
    writer.integer(engine::kInputMethodApiVersion, 4);
    writer.integer(descriptors.size(), 4);
    for (const auto& d : descriptors) {
        if (d.plugin_id != plugin_id || !engine::validate_input_method_descriptor(d).ok ||
            !ids.insert(d.id).second) return {};
        writer.text(d.id); writer.text(d.plugin_id); writer.text(d.name); writer.text(d.language_tag);
        writer.integer(static_cast<std::uint8_t>(d.kind), 1);
        writer.integer(d.max_input_bytes, 4); writer.integer(d.max_candidates, 4);
        writer.integer(d.alphabet.size(), 4);
        for (auto scalar : d.alphabet) writer.integer(scalar, 4);
    }
    if (writer.bytes.size() > kMaximumInputMethodWireBytes) return {};
    return std::move(writer.bytes);
}

engine::InputMethodValidation decode_input_method_descriptors(std::string_view plugin_id,
    std::string_view payload, std::vector<engine::InputMethodDescriptor>& output) {
    output.clear();
    Reader reader{payload};
    std::size_t count{};
    if (!reader.header("OWID") || !reader.size(count) || count == 0 || count > kMaximumPluginInputSchemes)
        return invalid();
    std::vector<engine::InputMethodDescriptor> values;
    std::unordered_set<std::string> ids;
    for (std::size_t i = 0; i < count; ++i) {
        engine::InputMethodDescriptor d;
        std::uint64_t kind{};
        std::size_t alphabet_count{};
        if (!reader.text(d.id, 257) || !reader.text(d.plugin_id, 128) || !reader.text(d.name, 256) ||
            !reader.text(d.language_tag, 64) || !reader.integer(kind, 1) || !reader.size(d.max_input_bytes) ||
            !reader.size(d.max_candidates) || !reader.size(alphabet_count) || alphabet_count == 0 ||
            alphabet_count > 1024) return invalid();
        d.kind = static_cast<engine::InputMethodKind>(kind);
        for (std::size_t j = 0; j < alphabet_count; ++j) {
            std::uint64_t scalar{};
            if (!reader.integer(scalar, 4)) return invalid();
            d.alphabet.push_back(static_cast<char32_t>(scalar));
        }
        if (d.plugin_id != plugin_id || !engine::validate_input_method_descriptor(d).ok ||
            !ids.insert(d.id).second) return invalid();
        values.push_back(std::move(d));
    }
    if (!reader.done()) return invalid();
    output = std::move(values);
    return {true, {}};
}
} // namespace owo::plugin

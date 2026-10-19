#include "owo/agent/agent_protocol.h"

#include <algorithm>
#include <array>
#include <charconv>
#include <cctype>
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <variant>

namespace owo::agent {
namespace {

struct JsonValue;
using Object = std::unordered_map<std::string, JsonValue>;
using Array = std::vector<JsonValue>;
struct JsonValue : std::variant<std::string, std::uint64_t, bool, Object, Array> {
    using variant::variant;
};

class Parser {
public:
    explicit Parser(const std::string_view input) : input_(input) {}
    bool parse(JsonValue& output) {
        space();
        if (!value(output, 0)) return false;
        space();
        return offset_ == input_.size() || fail("trailing JSON data");
    }
    const std::string& diagnostic() const { return diagnostic_; }

private:
    bool value(JsonValue& output, const unsigned depth) {
        if (depth > 7 || offset_ >= input_.size()) return fail("invalid JSON value");
        if (input_[offset_] == '"') {
            std::string text;
            if (!string(text)) return false;
            output = std::move(text);
            return true;
        }
        if (input_[offset_] == '{') {
            Object object;
            if (!object_value(object, depth + 1)) return false;
            output = std::move(object);
            return true;
        }
        if (input_[offset_] == '[') {
            Array array;
            if (!array_value(array, depth + 1)) return false;
            output = std::move(array);
            return true;
        }
        if (take("true")) { output = true; return true; }
        if (take("false")) { output = false; return true; }
        std::uint64_t number{};
        const auto begin = input_.data() + offset_;
        const auto parsed = std::from_chars(begin, input_.data() + input_.size(), number);
        if (parsed.ec != std::errc{} || parsed.ptr == begin) return fail("expected JSON value");
        offset_ = static_cast<std::size_t>(parsed.ptr - input_.data());
        output = number;
        return true;
    }
    bool object_value(Object& output, const unsigned depth) {
        if (!character('{')) return false;
        space();
        if (character('}')) return true;
        while (true) {
            std::string key;
            if (!string(key)) return false;
            space();
            if (!character(':')) return fail("expected ':'");
            space();
            JsonValue item;
            if (!value(item, depth)) return false;
            if (!output.emplace(std::move(key), std::move(item)).second)
                return fail("duplicate JSON field");
            space();
            if (character('}')) return true;
            if (!character(',')) return fail("expected ',' or '}'");
            space();
        }
    }
    bool array_value(Array& output, const unsigned depth) {
        if (!character('[')) return false;
        space();
        if (character(']')) return true;
        while (true) {
            if (output.size() >= kMaximumAgentCommands) return fail("array exceeds limit");
            JsonValue item;
            if (!value(item, depth)) return false;
            output.push_back(std::move(item));
            space();
            if (character(']')) return true;
            if (!character(',')) return fail("expected ',' or ']'");
            space();
        }
    }
    bool string(std::string& output) {
        if (!character('"')) return fail("expected JSON string");
        while (offset_ < input_.size()) {
            const unsigned char byte = static_cast<unsigned char>(input_[offset_++]);
            if (byte == '"') return true;
            if (byte < 0x20) return fail("control character in JSON string");
            if (byte != '\\') { output.push_back(static_cast<char>(byte)); continue; }
            if (offset_ >= input_.size()) return fail("truncated JSON escape");
            switch (input_[offset_++]) {
            case '"': output.push_back('"'); break;
            case '\\': output.push_back('\\'); break;
            case '/': output.push_back('/'); break;
            case 'b': output.push_back('\b'); break;
            case 'f': output.push_back('\f'); break;
            case 'n': output.push_back('\n'); break;
            case 'r': output.push_back('\r'); break;
            case 't': output.push_back('\t'); break;
            case 'u': {
                std::uint32_t scalar{};
                if (!hex_quad(scalar)) return false;
                if (scalar >= 0xd800 && scalar <= 0xdbff) {
                    if (offset_ + 2 > input_.size() || input_[offset_] != '\\' ||
                        input_[offset_ + 1] != 'u') return fail("missing low surrogate");
                    offset_ += 2;
                    std::uint32_t low{};
                    if (!hex_quad(low) || low < 0xdc00 || low > 0xdfff)
                        return fail("invalid low surrogate");
                    scalar = 0x10000 + ((scalar - 0xd800) << 10) + (low - 0xdc00);
                } else if (scalar >= 0xdc00 && scalar <= 0xdfff) {
                    return fail("unexpected low surrogate");
                }
                append_utf8(output, scalar);
                break;
            }
            default: return fail("unsupported JSON escape");
            }
        }
        return fail("unterminated JSON string");
    }
    bool hex_quad(std::uint32_t& output) {
        if (offset_ + 4 > input_.size()) return fail("truncated Unicode escape");
        output = 0;
        for (unsigned index = 0; index < 4; ++index) {
            const unsigned char byte = static_cast<unsigned char>(input_[offset_++]);
            output <<= 4;
            if (byte >= '0' && byte <= '9') output |= byte - '0';
            else if (byte >= 'a' && byte <= 'f') output |= byte - 'a' + 10;
            else if (byte >= 'A' && byte <= 'F') output |= byte - 'A' + 10;
            else return fail("invalid Unicode escape");
        }
        return true;
    }
    static void append_utf8(std::string& output, const std::uint32_t scalar) {
        if (scalar <= 0x7f) output.push_back(static_cast<char>(scalar));
        else if (scalar <= 0x7ff) {
            output.push_back(static_cast<char>(0xc0 | (scalar >> 6)));
            output.push_back(static_cast<char>(0x80 | (scalar & 0x3f)));
        } else if (scalar <= 0xffff) {
            output.push_back(static_cast<char>(0xe0 | (scalar >> 12)));
            output.push_back(static_cast<char>(0x80 | ((scalar >> 6) & 0x3f)));
            output.push_back(static_cast<char>(0x80 | (scalar & 0x3f)));
        } else {
            output.push_back(static_cast<char>(0xf0 | (scalar >> 18)));
            output.push_back(static_cast<char>(0x80 | ((scalar >> 12) & 0x3f)));
            output.push_back(static_cast<char>(0x80 | ((scalar >> 6) & 0x3f)));
            output.push_back(static_cast<char>(0x80 | (scalar & 0x3f)));
        }
    }
    bool character(const char expected) {
        if (offset_ >= input_.size() || input_[offset_] != expected) return false;
        ++offset_;
        return true;
    }
    bool take(const std::string_view token) {
        if (input_.substr(offset_, token.size()) != token) return false;
        offset_ += token.size();
        return true;
    }
    void space() {
        while (offset_ < input_.size() &&
               (input_[offset_] == ' ' || input_[offset_] == '\t' ||
                input_[offset_] == '\r' || input_[offset_] == '\n')) ++offset_;
    }
    bool fail(std::string value) {
        if (diagnostic_.empty()) diagnostic_ = std::move(value);
        return false;
    }
    std::string_view input_;
    std::size_t offset_{};
    std::string diagnostic_;
};

bool valid_utf8(const std::string_view text) {
    std::size_t offset = 0;
    while (offset < text.size()) {
        const auto first = static_cast<unsigned char>(text[offset]);
        std::size_t count = 0;
        std::uint32_t scalar = 0;
        if (first <= 0x7f) { count = 1; scalar = first; }
        else if (first >= 0xc2 && first <= 0xdf) { count = 2; scalar = first & 0x1f; }
        else if (first >= 0xe0 && first <= 0xef) { count = 3; scalar = first & 0x0f; }
        else if (first >= 0xf0 && first <= 0xf4) { count = 4; scalar = first & 7; }
        else return false;
        if (offset + count > text.size()) return false;
        for (std::size_t index = 1; index < count; ++index) {
            const auto next = static_cast<unsigned char>(text[offset + index]);
            if ((next & 0xc0) != 0x80) return false;
            scalar = (scalar << 6) | (next & 0x3f);
        }
        if ((count == 2 && scalar < 0x80) || (count == 3 && scalar < 0x800) ||
            (count == 4 && scalar < 0x10000) ||
            (scalar >= 0xd800 && scalar <= 0xdfff) || scalar > 0x10ffff) return false;
        offset += count;
    }
    return true;
}

bool token(const std::string_view value, const std::size_t maximum = 128) {
    return !value.empty() && value.size() <= maximum &&
        std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
            return std::isalnum(byte) != 0 || byte == '-' || byte == '_' || byte == '.';
        });
}

bool session_id(const std::string_view value) {
    return value.size() >= 16 && value.size() <= 64 &&
        std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
            return std::isxdigit(byte) != 0 || byte == '-';
        });
}

std::string escape(std::string_view value);
template <typename T>
const T* field(const Object& object, const char* name);

bool valid_application(const ApplicationContext& application) {
    return application.executable.size() <= 260 &&
        application.window_class.size() <= 128 &&
        application.focus_window_class.size() <= 128 &&
        (application.context_id.empty() || token(application.context_id)) &&
        valid_utf8(application.executable) && valid_utf8(application.window_class) &&
        valid_utf8(application.focus_window_class);
}

std::string encode_application(const ApplicationContext& application) {
    if (!valid_application(application)) return {};
    return "{\"process_id\":" + std::to_string(application.process_id) +
        ",\"thread_id\":" + std::to_string(application.thread_id) +
        ",\"executable\":\"" + escape(application.executable) +
        "\",\"window_class\":\"" + escape(application.window_class) +
        "\",\"focus_window_class\":\"" + escape(application.focus_window_class) +
        "\",\"context_id\":\"" + escape(application.context_id) +
        "\",\"sensitive_input\":" +
        (application.sensitive_input ? "true" : "false") + "}";
}

std::optional<ApplicationContext> decode_application(const Object& object) {
    if (object.size() != 7) return std::nullopt;
    const auto* pid = field<std::uint64_t>(object, "process_id");
    const auto* tid = field<std::uint64_t>(object, "thread_id");
    const auto* executable = field<std::string>(object, "executable");
    const auto* window_class = field<std::string>(object, "window_class");
    const auto* focus_class = field<std::string>(object, "focus_window_class");
    const auto* context_id = field<std::string>(object, "context_id");
    const auto* sensitive = field<bool>(object, "sensitive_input");
    if (pid == nullptr || *pid > 0xffffffffULL || tid == nullptr ||
        *tid > 0xffffffffULL || executable == nullptr || window_class == nullptr ||
        focus_class == nullptr || context_id == nullptr || sensitive == nullptr)
        return std::nullopt;
    ApplicationContext application;
    application.process_id = static_cast<std::uint32_t>(*pid);
    application.thread_id = static_cast<std::uint32_t>(*tid);
    application.executable = *executable;
    application.window_class = *window_class;
    application.focus_window_class = *focus_class;
    application.context_id = *context_id;
    application.sensitive_input = *sensitive;
    if (!valid_application(application)) return std::nullopt;
    return application;
}

std::string escape(const std::string_view value) {
    std::string output;
    output.reserve(value.size() + 8);
    for (const unsigned char byte : value) {
        switch (byte) {
        case '"': output += "\\\""; break;
        case '\\': output += "\\\\"; break;
        case '\b': output += "\\b"; break;
        case '\f': output += "\\f"; break;
        case '\n': output += "\\n"; break;
        case '\r': output += "\\r"; break;
        case '\t': output += "\\t"; break;
        default: if (byte < 0x20) return {}; output.push_back(static_cast<char>(byte));
        }
    }
    return output;
}

template <typename T>
const T* field(const Object& object, const char* name) {
    const auto found = object.find(name);
    return found == object.end() ? nullptr : std::get_if<T>(&found->second);
}

bool valid_optional_token(const std::string_view value, const std::size_t maximum = 128) {
    return value.empty() || token(value, maximum);
}

bool valid_text(const std::string_view value, const std::size_t maximum) {
    return value.size() <= maximum && valid_utf8(value);
}

std::optional<EffectKind> parse_effect_kind(const std::string_view value) {
    if (value == "commit_text") return EffectKind::commit_text;
    if (value == "replace_selection") return EffectKind::replace_selection;
    if (value == "copy_text") return EffectKind::copy_text;
    return std::nullopt;
}

bool lowercase_sha256(const std::string_view value) {
    return value.size() == 64 && std::all_of(value.begin(), value.end(), [](const char c) {
        return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
    });
}

bool valid_effect(const CommandEffect& effect) {
    const auto code_points = std::count_if(effect.text.begin(), effect.text.end(), [](const char c) {
        return (static_cast<unsigned char>(c) & 0xc0U) != 0x80U;
    });
    if (effect.text.empty() || !valid_utf8(effect.text) ||
        static_cast<std::size_t>(code_points) > kMaximumAgentEffectTextCodePoints ||
        !valid_optional_token(effect.target_context_id))
        return false;
    if (effect.kind == EffectKind::replace_selection)
        return !effect.target_context_id.empty() && lowercase_sha256(effect.selection_sha256);
    return effect.selection_sha256.empty();
}

bool valid_risk_level(const std::string_view value) {
    return value == "none" || value == "low" || value == "medium" ||
           value == "high" || value == "critical";
}

std::string encode_string_array(const std::vector<std::string>& values) {
    std::string output = "[";
    std::unordered_set<std::string> unique;
    for (std::size_t index = 0; index < values.size(); ++index) {
        if (!token(values[index], 96) || !unique.insert(values[index]).second) return {};
        if (index != 0) output += ',';
        output += "\"" + escape(values[index]) + "\"";
    }
    output += ']';
    return output;
}

std::optional<std::vector<std::string>> decode_string_array(const Array& values) {
    std::vector<std::string> output;
    std::unordered_set<std::string> unique;
    for (const auto& value : values) {
        const auto* text = std::get_if<std::string>(&value);
        if (text == nullptr || !token(*text, 96) || !unique.insert(*text).second)
            return std::nullopt;
        output.push_back(*text);
    }
    return output;
}

bool valid_input_payload(const InputPayload& input) {
    return valid_text(input.raw_pinyin, 4096) &&
        valid_text(input.segmented_pinyin, 8192) &&
        valid_text(input.selected_text, 16384) &&
        valid_text(input.pending_pinyin, 4096) &&
        valid_text(input.natural_language, 16384) && token(input.input_mode, 32);
}

std::string encode_input_payload(const InputPayload& input) {
    if (!valid_input_payload(input)) return {};
    return "{\"raw_pinyin\":\"" + escape(input.raw_pinyin) +
        "\",\"segmented_pinyin\":\"" + escape(input.segmented_pinyin) +
        "\",\"selected_text\":\"" + escape(input.selected_text) +
        "\",\"pending_pinyin\":\"" + escape(input.pending_pinyin) +
        "\",\"natural_language\":\"" + escape(input.natural_language) +
        "\",\"input_mode\":\"" + escape(input.input_mode) +
        "\",\"correction_enabled\":" +
        (input.correction_enabled ? "true" : "false") + "}";
}

std::optional<InputPayload> decode_input_payload(const Object& object) {
    if (object.size() != 7) return std::nullopt;
    const auto* raw = field<std::string>(object, "raw_pinyin");
    const auto* segmented = field<std::string>(object, "segmented_pinyin");
    const auto* selected = field<std::string>(object, "selected_text");
    const auto* pending = field<std::string>(object, "pending_pinyin");
    const auto* natural = field<std::string>(object, "natural_language");
    const auto* mode = field<std::string>(object, "input_mode");
    const auto* correction = field<bool>(object, "correction_enabled");
    if (raw == nullptr || segmented == nullptr || selected == nullptr || pending == nullptr ||
        natural == nullptr || mode == nullptr || correction == nullptr) return std::nullopt;
    InputPayload output{*raw, *segmented, *selected, *pending, *natural, *mode, *correction};
    return valid_input_payload(output) ? std::optional<InputPayload>(std::move(output))
                                       : std::nullopt;
}

bool valid_range(const PinyinRange& range) {
    return range.start < range.end && range.end <= 4096;
}

bool valid_ranges(const std::vector<PinyinRange>& ranges,
                  const std::size_t maximum = kMaximumAgentTaskRanges) {
    if (ranges.size() > maximum) return false;
    auto sorted = ranges;
    for (const auto& range : sorted)
        if (!valid_range(range)) return false;
    std::sort(sorted.begin(), sorted.end(), [](const auto& left, const auto& right) {
        return left.start < right.start ||
               (left.start == right.start && left.end < right.end);
    });
    for (std::size_t index = 1; index < sorted.size(); ++index)
        if (sorted[index].start < sorted[index - 1].end) return false;
    return true;
}

std::string encode_ranges(const std::vector<PinyinRange>& ranges) {
    if (!valid_ranges(ranges)) return {};
    std::string output = "[";
    for (std::size_t index = 0; index < ranges.size(); ++index) {
        if (index != 0) output += ',';
        output += "{\"start\":" + std::to_string(ranges[index].start) +
            ",\"end\":" + std::to_string(ranges[index].end) + "}";
    }
    output += ']';
    return output;
}

std::optional<std::vector<PinyinRange>> decode_ranges(const Array& values) {
    if (values.size() > kMaximumAgentTaskRanges) return std::nullopt;
    std::vector<PinyinRange> output;
    for (const auto& value : values) {
        const auto* item = std::get_if<Object>(&value);
        if (item == nullptr || item->size() != 2) return std::nullopt;
        const auto* start = field<std::uint64_t>(*item, "start");
        const auto* end = field<std::uint64_t>(*item, "end");
        if (start == nullptr || end == nullptr || *start > 4096 || *end > 4096)
            return std::nullopt;
        output.push_back({static_cast<std::uint32_t>(*start),
                          static_cast<std::uint32_t>(*end)});
    }
    return valid_ranges(output) ? std::optional(std::move(output)) : std::nullopt;
}

bool valid_slot_update(const SlotUpdate& update) {
    return token(update.slot_id, 64) && valid_text(update.value, 4096) &&
           !update.consumed_ranges.empty() && valid_ranges(update.consumed_ranges);
}

std::string encode_slot_updates(const std::vector<SlotUpdate>& updates) {
    if (updates.size() > kMaximumAgentSlotUpdates) return {};
    std::unordered_set<std::string> ids;
    std::string output = "[";
    for (std::size_t index = 0; index < updates.size(); ++index) {
        const auto& update = updates[index];
        const auto ranges = encode_ranges(update.consumed_ranges);
        if (!valid_slot_update(update) || ranges.empty() ||
            !ids.insert(update.slot_id).second) return {};
        if (index != 0) output += ',';
        output += "{\"slot_id\":\"" + escape(update.slot_id) +
            "\",\"value\":\"" + escape(update.value) +
            "\",\"consumed_ranges\":" + ranges + ",\"lock\":" +
            (update.lock ? "true" : "false") + "}";
    }
    output += ']';
    return output;
}

std::optional<std::vector<SlotUpdate>> decode_slot_updates(const Array& values) {
    if (values.size() > kMaximumAgentSlotUpdates) return std::nullopt;
    std::vector<SlotUpdate> output;
    std::unordered_set<std::string> ids;
    for (const auto& value : values) {
        const auto* item = std::get_if<Object>(&value);
        if (item == nullptr || item->size() != 4) return std::nullopt;
        const auto* id = field<std::string>(*item, "slot_id");
        const auto* text = field<std::string>(*item, "value");
        const auto* ranges = field<Array>(*item, "consumed_ranges");
        const auto* lock = field<bool>(*item, "lock");
        if (id == nullptr || text == nullptr || ranges == nullptr || lock == nullptr ||
            !ids.insert(*id).second) return std::nullopt;
        const auto parsed_ranges = decode_ranges(*ranges);
        if (!parsed_ranges) return std::nullopt;
        SlotUpdate update{*id, *text, *parsed_ranges, *lock};
        if (!valid_slot_update(update)) return std::nullopt;
        output.push_back(std::move(update));
    }
    return output;
}

bool valid_task(const TaskDraft& task) {
    if (task.intent.empty())
        return task.intent_ranges.empty() && task.slots.empty() &&
               task.unconsumed_ranges.empty() && task.revision == 0;
    if (!token(task.intent, 96) || task.slots.size() > kMaximumAgentTaskSlots ||
        !valid_ranges(task.intent_ranges) || !valid_ranges(task.unconsumed_ranges)) return false;
    std::unordered_set<std::string> ids;
    std::vector<PinyinRange> mapped = task.intent_ranges;
    for (const auto& slot : task.slots) {
        if (!token(slot.id, 64) || !ids.insert(slot.id).second ||
            !valid_text(slot.label, 256) || !valid_text(slot.value, 4096) ||
            slot.confidence_milli > 1000 || !valid_ranges(slot.source_ranges)) return false;
        mapped.insert(mapped.end(), slot.source_ranges.begin(), slot.source_ranges.end());
    }
    mapped.insert(mapped.end(), task.unconsumed_ranges.begin(), task.unconsumed_ranges.end());
    return valid_ranges(mapped, kMaximumAgentTaskRanges * 2);
}

std::string encode_task(const TaskDraft& task) {
    if (!valid_task(task)) return {};
    const auto intent_ranges = encode_ranges(task.intent_ranges);
    const auto unconsumed = encode_ranges(task.unconsumed_ranges);
    if (intent_ranges.empty() || unconsumed.empty()) return {};
    std::string slots = "[";
    for (std::size_t index = 0; index < task.slots.size(); ++index) {
        const auto& slot = task.slots[index];
        const auto ranges = encode_ranges(slot.source_ranges);
        if (ranges.empty()) return {};
        if (index != 0) slots += ',';
        slots += "{\"id\":\"" + escape(slot.id) + "\",\"label\":\"" +
            escape(slot.label) + "\",\"value\":\"" + escape(slot.value) +
            "\",\"source_ranges\":" + ranges + ",\"locked\":" +
            (slot.locked ? "true" : "false") + ",\"confidence_milli\":" +
            std::to_string(slot.confidence_milli) + "}";
    }
    slots += ']';
    return "{\"intent\":\"" + escape(task.intent) + "\",\"intent_ranges\":" +
        intent_ranges + ",\"slots\":" + slots + ",\"unconsumed_ranges\":" +
        unconsumed + ",\"revision\":" + std::to_string(task.revision) + "}";
}

std::optional<TaskDraft> decode_task(const Object& object) {
    if (object.size() != 5) return std::nullopt;
    const auto* intent = field<std::string>(object, "intent");
    const auto* intent_ranges = field<Array>(object, "intent_ranges");
    const auto* slots = field<Array>(object, "slots");
    const auto* unconsumed = field<Array>(object, "unconsumed_ranges");
    const auto* revision = field<std::uint64_t>(object, "revision");
    if (intent == nullptr || intent_ranges == nullptr || slots == nullptr ||
        unconsumed == nullptr || revision == nullptr ||
        slots->size() > kMaximumAgentTaskSlots) return std::nullopt;
    const auto parsed_intent_ranges = decode_ranges(*intent_ranges);
    const auto parsed_unconsumed = decode_ranges(*unconsumed);
    if (!parsed_intent_ranges || !parsed_unconsumed) return std::nullopt;
    TaskDraft task;
    task.intent = *intent;
    task.intent_ranges = *parsed_intent_ranges;
    task.unconsumed_ranges = *parsed_unconsumed;
    task.revision = *revision;
    for (const auto& value : *slots) {
        const auto* item = std::get_if<Object>(&value);
        if (item == nullptr || item->size() != 6) return std::nullopt;
        const auto* id = field<std::string>(*item, "id");
        const auto* label = field<std::string>(*item, "label");
        const auto* text = field<std::string>(*item, "value");
        const auto* ranges = field<Array>(*item, "source_ranges");
        const auto* locked = field<bool>(*item, "locked");
        const auto* confidence = field<std::uint64_t>(*item, "confidence_milli");
        if (id == nullptr || label == nullptr || text == nullptr || ranges == nullptr ||
            locked == nullptr || confidence == nullptr || *confidence > 1000)
            return std::nullopt;
        const auto parsed_ranges = decode_ranges(*ranges);
        if (!parsed_ranges) return std::nullopt;
        task.slots.push_back({*id, *label, *text, *parsed_ranges, *locked,
                              static_cast<std::uint32_t>(*confidence)});
    }
    return valid_task(task) ? std::optional(std::move(task)) : std::nullopt;
}

std::optional<Action> parse_action(const std::string_view value) {
    for (const auto action : {Action::submit, Action::select, Action::page,
                              Action::cancel, Action::poll})
        if (value == action_name(action)) return action;
    return std::nullopt;
}

std::optional<Status> parse_status(const std::string_view value) {
    for (const auto status : {Status::connecting, Status::agent_mode, Status::submitting,
                              Status::thinking, Status::waiting_for_user, Status::executing,
                              Status::waiting_for_confirmation, Status::cancelling,
                              Status::cancelled, Status::completed, Status::disconnected,
                              Status::timeout, Status::error})
        if (value == status_name(status)) return status;
    return std::nullopt;
}

bool luhn_number(const std::string_view digits) {
    if (digits.size() < 13 || digits.size() > 19) return false;
    unsigned sum = 0;
    bool double_digit = false;
    for (auto index = digits.rbegin(); index != digits.rend(); ++index) {
        if (*index < '0' || *index > '9') return false;
        unsigned value = static_cast<unsigned>(*index - '0');
        if (double_digit && (value *= 2) > 9) value -= 9;
        sum += value;
        double_digit = !double_digit;
    }
    return sum % 10 == 0;
}

bool ascii_digit(const char value) noexcept { return value >= '0' && value <= '9'; }

// GB 11643 checksum for 18-character resident identity numbers.
bool resident_id_number(const std::string_view value) {
    if (value.size() != 18) return false;
    constexpr std::array<int, 17> weights{7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2};
    constexpr std::string_view checks = "10X98765432";
    int sum = 0;
    for (std::size_t index = 0; index < 17; ++index) {
        if (!ascii_digit(value[index])) return false;
        sum += (value[index] - '0') * weights[index];
    }
    const char last = value[17] == 'x' ? 'X' : value[17];
    return checks[static_cast<std::size_t>(sum % 11)] == last;
}

bool mainland_mobile_number(const std::string_view value) {
    return value.size() == 11 && value[0] == '1' && value[1] >= '3' && value[1] <= '9' &&
           std::all_of(value.begin(), value.end(), ascii_digit);
}

bool email_local_byte(const unsigned char value) noexcept {
    return std::isalnum(value) || value == '.' || value == '_' || value == '%' ||
           value == '+' || value == '-';
}

bool email_domain_byte(const unsigned char value) noexcept {
    return std::isalnum(value) || value == '.' || value == '-';
}

// Chinese secret labels are followed by an optional separator and the value. The value
// ends at whitespace, ASCII separators or common full-width punctuation.
void redact_labeled_secrets(std::string& output, bool& redacted) {
    constexpr std::array<std::string_view, 11> labels{
        "支付密码", "登录密码", "密码", "口令", "验证码", "校验码", "动态码",
        "卡号", "身份证号", "身份证", "PIN"};
    constexpr std::array<std::string_view, 7> separators{
        "：", ":", "=", "是", "为", "＝", " "};
    constexpr std::array<std::string_view, 6> terminators{
        "，", "。", "；", "、", "）", "　"};
    for (const auto label : labels) {
        std::size_t found = 0;
        while ((found = output.find(label, found)) != std::string::npos) {
            auto begin = found + label.size();
            bool advanced = true;
            while (advanced && begin < output.size()) {
                advanced = false;
                for (const auto separator : separators) {
                    if (output.compare(begin, separator.size(), separator) == 0) {
                        begin += separator.size();
                        advanced = true;
                        break;
                    }
                }
            }
            // Without a separator only an immediate digit counts as a value, so prose such as
            // "忘记密码怎么办" stays intact while "验证码123456" is still redacted.
            const bool separated = begin != found + label.size();
            if (!separated && (begin >= output.size() || !ascii_digit(output[begin]))) {
                found = begin;
                continue;
            }
            auto end = begin;
            while (end < output.size()) {
                const auto byte = static_cast<unsigned char>(output[end]);
                if (std::isspace(byte) || byte == ',' || byte == ';' || byte == '&' ||
                    byte == '.' || byte == ')') break;
                bool stop = false;
                for (const auto terminator : terminators)
                    if (output.compare(end, terminator.size(), terminator) == 0) stop = true;
                if (stop) break;
                ++end;
            }
            // Values already replaced by a typed marker such as [ID_REDACTED] stay as they are.
            if (end > begin && output[begin] != '[') {
                output.replace(begin, end - begin, "[REDACTED]");
                redacted = true;
                found = begin + 10;
            } else {
                found = begin;
            }
        }
    }
}

void redact_identifiers(std::string& output, bool& redacted) {
    for (std::size_t begin = 0; begin < output.size();) {
        if (!ascii_digit(output[begin]) || (begin != 0 && ascii_digit(output[begin - 1]))) {
            ++begin;
            continue;
        }
        auto end = begin;
        while (end < output.size() && ascii_digit(output[end])) ++end;
        if (end - begin == 17 && end < output.size() && (output[end] == 'X' || output[end] == 'x'))
            ++end;
        const auto run = std::string_view(output).substr(begin, end - begin);
        const bool bounded = end == output.size() ||
            !std::isalnum(static_cast<unsigned char>(output[end]));
        std::string_view replacement;
        if (bounded && resident_id_number(run)) replacement = "[ID_REDACTED]";
        else if (bounded && mainland_mobile_number(run)) replacement = "[PHONE_REDACTED]";
        else if (luhn_number(run)) replacement = "[PAYMENT_REDACTED]";
        if (!replacement.empty()) {
            output.replace(begin, run.size(), replacement);
            redacted = true;
            begin += replacement.size();
        } else {
            begin = end;
        }
    }
    for (std::size_t at = output.find('@'); at != std::string::npos; at = output.find('@', at + 1)) {
        auto begin = at;
        while (begin != 0 && email_local_byte(static_cast<unsigned char>(output[begin - 1]))) --begin;
        auto end = at + 1;
        while (end < output.size() && email_domain_byte(static_cast<unsigned char>(output[end]))) ++end;
        while (end > at + 1 && (output[end - 1] == '.' || output[end - 1] == '-')) --end;
        const auto domain = std::string_view(output).substr(at + 1, end - at - 1);
        const auto dot = domain.rfind('.');
        if (begin == at || dot == std::string_view::npos || dot == 0 || domain.size() - dot < 3)
            continue;
        output.replace(begin, end - begin, "[EMAIL_REDACTED]");
        redacted = true;
        at = begin + 15;
    }
}

}  // namespace

const char* action_name(const Action action) noexcept {
    switch (action) {
    case Action::submit: return "submit";
    case Action::select: return "select";
    case Action::page: return "page";
    case Action::cancel: return "cancel";
    case Action::poll: return "poll";
    }
    return "submit";
}

const char* status_name(const Status status) noexcept {
    switch (status) {
    case Status::connecting: return "connecting";
    case Status::agent_mode: return "agent_mode";
    case Status::submitting: return "submitting";
    case Status::thinking: return "thinking";
    case Status::waiting_for_user: return "waiting_for_user";
    case Status::executing: return "executing";
    case Status::waiting_for_confirmation: return "waiting_for_confirmation";
    case Status::cancelling: return "cancelling";
    case Status::cancelled: return "cancelled";
    case Status::completed: return "completed";
    case Status::disconnected: return "disconnected";
    case Status::timeout: return "timeout";
    case Status::error: return "error";
    }
    return "error";
}

std::string encode_request_v2(const Request& request) {
    if (request.selection) return {};
    if (!session_id(request.session_id) || request.user_input.size() > 4096 ||
        request.session_context.size() > kMaximumAgentContextEntryBytes ||
        request.context_entries.size() > kMaximumAgentContextEntries ||
        request.command_id.size() > 128 || !valid_application(request.application) ||
        !valid_utf8(request.user_input) || !valid_utf8(request.session_context) ||
        (!request.command_id.empty() && !token(request.command_id)) ||
        request.slot_updates.size() > kMaximumAgentSlotUpdates) return {};
    if (request.action == Action::submit && request.user_input.empty()) return {};
    if (request.action == Action::select && request.command_id.empty()) return {};
    const auto input = escape(request.user_input);
    const auto context = escape(request.session_context);
    const auto command = escape(request.command_id);
    const auto application = encode_application(request.application);
    std::string entries = "[";
    std::size_t context_bytes = 0;
    for (std::size_t index = 0; index < request.context_entries.size(); ++index) {
        const auto& entry = request.context_entries[index];
        if (!token(entry.context_id) || entry.application.context_id != entry.context_id ||
            entry.text.empty() || entry.text.size() > kMaximumAgentContextEntryBytes ||
            !valid_utf8(entry.text) || !valid_application(entry.application) ||
            context_bytes > kMaximumAgentContextBytes - entry.text.size()) return {};
        context_bytes += entry.text.size();
        if (index != 0) entries += ',';
        entries += "{\"context_id\":\"" + escape(entry.context_id) +
            "\",\"application\":" + encode_application(entry.application) +
            ",\"text\":\"" + escape(entry.text) + "\",\"sequence\":" +
            std::to_string(entry.sequence) + "}";
    }
    entries += ']';
    auto output = std::string("{\"schema_version\":") +
        std::to_string(kMinimumAgentProtocolVersion) + ",\"action\":\"" +
        action_name(request.action) + "\",\"session_id\":\"" + request.session_id +
        "\",\"user_input\":\"" + input + "\",\"application\":" + application +
        ",\"session_context\":\"" + context + "\",\"context_entries\":" + entries +
        ",\"command_id\":\"" + command +
        "\",\"page\":" + std::to_string(request.page) + "}";
    return output.size() <= kMaximumAgentPayloadBytes ? output : std::string{};
}

std::string encode_request(const Request& request) {
    if (!session_id(request.session_id) || request.user_input.size() > 4096 ||
        request.session_context.size() > kMaximumAgentContextEntryBytes ||
        request.context_entries.size() > kMaximumAgentContextEntries ||
        request.capabilities.size() > 32 || request.required_features.size() > 16 ||
        request.protocol_min < kMinimumAgentProtocolVersion ||
        request.protocol_max > kAgentProtocolVersion ||
        request.protocol_min > request.protocol_max || request.command_id.size() > 128 ||
        !valid_application(request.application) || !valid_utf8(request.user_input) ||
        !valid_utf8(request.session_context) || !valid_input_payload(request.input_details) ||
        !token(request.request_id) ||
        !valid_optional_token(request.parent_request_id) ||
        !token(request.idempotency_key) ||
        (!request.command_id.empty() && !token(request.command_id))) return {};
    std::string selection_json;
    if (request.selection) {
        const auto& selected = *request.selection;
        const auto points = std::count_if(selected.text.begin(), selected.text.end(),
            [](unsigned char c) { return (c & 0xc0U) != 0x80U; });
        if (selected.text.empty() || selected.text.size() > 4096 || points > 1000 ||
            !valid_utf8(selected.text) || !lowercase_sha256(selected.sha256) ||
            selected.text.find_first_not_of(" \t\r\n") == std::string::npos ||
            request.application.sensitive_input || request.application.context_id.empty() ||
            std::find(request.capabilities.begin(), request.capabilities.end(),
                      kAgentCapabilityInputSelection) == request.capabilities.end()) return {};
        selection_json = ",\"selection\":{\"text\":\"" + escape(selected.text) +
            "\",\"sha256\":\"" + selected.sha256 + "\",\"redacted\":" +
            (selected.redacted ? "true" : "false") + "}";
    }
    if (request.action == Action::submit && request.user_input.empty()) return {};
    if (request.action == Action::select && request.command_id.empty()) return {};
    if (!request.slot_updates.empty() && request.task_revision == 0) return {};
    const auto capabilities = encode_string_array(request.capabilities);
    const auto required_features = encode_string_array(request.required_features);
    const auto input_details = encode_input_payload(request.input_details);
    const auto slot_updates = encode_slot_updates(request.slot_updates);
    if (capabilities.empty() || required_features.empty() || input_details.empty() ||
        slot_updates.empty()) return {};
    if (!std::all_of(request.required_features.begin(), request.required_features.end(),
        [&](const auto& feature) {
            return std::find(request.capabilities.begin(), request.capabilities.end(), feature) !=
                   request.capabilities.end();
        })) return {};
    std::string entries = "[";
    std::size_t context_bytes = 0;
    for (std::size_t index = 0; index < request.context_entries.size(); ++index) {
        const auto& entry = request.context_entries[index];
        if (!token(entry.context_id) || entry.application.context_id != entry.context_id ||
            entry.text.empty() || entry.text.size() > kMaximumAgentContextEntryBytes ||
            !valid_utf8(entry.text) || !valid_application(entry.application) ||
            !token(entry.context_type, 48) || !token(entry.source, 48) ||
            entry.relevance_milli > 1000 ||
            context_bytes > kMaximumAgentContextBytes - entry.text.size()) return {};
        context_bytes += entry.text.size();
        if (index != 0) entries += ',';
        entries += "{\"context_id\":\"" + escape(entry.context_id) +
            "\",\"application\":" + encode_application(entry.application) +
            ",\"text\":\"" + escape(entry.text) + "\",\"sequence\":" +
            std::to_string(entry.sequence) + ",\"context_type\":\"" +
            escape(entry.context_type) + "\",\"source\":\"" + escape(entry.source) +
            "\",\"created_at_ms\":" + std::to_string(entry.created_at_ms) +
            ",\"relevance_milli\":" + std::to_string(entry.relevance_milli) +
            ",\"is_current\":" + (entry.is_current ? "true" : "false") +
            ",\"privacy\":{\"filtered\":" + (entry.filtered ? "true" : "false") +
            ",\"redacted\":" + (entry.redacted ? "true" : "false") +
            ",\"truncated\":" + (entry.truncated ? "true" : "false") + "}}";
    }
    entries += ']';
    auto output = std::string("{\"schema_version\":3,\"action\":\"") +
        action_name(request.action) + "\",\"session_id\":\"" + request.session_id +
        "\",\"request_id\":\"" + escape(request.request_id) +
        "\",\"parent_request_id\":\"" + escape(request.parent_request_id) +
        "\",\"idempotency_key\":\"" + escape(request.idempotency_key) +
        "\",\"capabilities\":" + capabilities +
        ",\"protocol_min\":" + std::to_string(request.protocol_min) +
        ",\"protocol_max\":" + std::to_string(request.protocol_max) +
        ",\"required_features\":" + required_features +
        ",\"user_input\":\"" + escape(request.user_input) +
        "\",\"input\":" + input_details +
        ",\"application\":" + encode_application(request.application) +
        ",\"session_context\":\"" + escape(request.session_context) +
        "\",\"context_entries\":" + entries +
        ",\"command_id\":\"" + escape(request.command_id) +
        "\",\"page\":" + std::to_string(request.page) +
        ",\"task_revision\":" + std::to_string(request.task_revision) +
        ",\"slot_updates\":" + slot_updates + selection_json + "}";
    return output.size() <= kMaximumAgentPayloadBytes ? output : std::string{};
}

RequestDecodeResult decode_request(const std::string_view json) {
    RequestDecodeResult result;
    if (json.empty() || json.size() > kMaximumAgentPayloadBytes)
        return {false, {}, "agent request size is invalid"};
    JsonValue root;
    Parser parser(json);
    if (!parser.parse(root)) return {false, {}, parser.diagnostic()};
    const auto* object = std::get_if<Object>(&root);
    if (object == nullptr) return {false, {}, "agent request fields mismatch"};
    const auto* version = field<std::uint64_t>(*object, "schema_version");
    if (version == nullptr || (*version != 2 && *version != 3))
        return {false, {}, "unsupported agent request version"};
    const bool v3 = *version == 3;
    const bool has_selection = object->find("selection") != object->end();
    if (object->size() != (v3 ? 19U + (has_selection ? 1U : 0U) : 9U))
        return {false, {}, "agent request fields mismatch"};
    const auto* action = field<std::string>(*object, "action");
    const auto* id = field<std::string>(*object, "session_id");
    const auto* input = field<std::string>(*object, "user_input");
    const auto* application = field<Object>(*object, "application");
    const auto* context = field<std::string>(*object, "session_context");
    const auto* context_entries = field<Array>(*object, "context_entries");
    const auto* command = field<std::string>(*object, "command_id");
    const auto* page = field<std::uint64_t>(*object, "page");
    if (action == nullptr || id == nullptr || input == nullptr || application == nullptr ||
        context == nullptr || command == nullptr ||
        context_entries == nullptr || page == nullptr || *page > 1000)
        return {false, {}, "agent request field type is invalid"};
    const auto parsed_action = parse_action(*action);
    const auto parsed_application = decode_application(*application);
    if (!parsed_action || !session_id(*id) || !parsed_application)
        return {false, {}, "agent request identity is invalid"};
    result.value.action = *parsed_action;
    result.value.session_id = *id;
    result.value.user_input = *input;
    result.value.application = *parsed_application;
    result.value.session_context = *context;
    result.value.command_id = *command;
    result.value.page = static_cast<std::uint32_t>(*page);
    result.value.schema_version = static_cast<std::uint32_t>(*version);
    if (v3) {
        const auto* request_id = field<std::string>(*object, "request_id");
        const auto* parent_id = field<std::string>(*object, "parent_request_id");
        const auto* idempotency = field<std::string>(*object, "idempotency_key");
        const auto* capabilities = field<Array>(*object, "capabilities");
        const auto* protocol_min = field<std::uint64_t>(*object, "protocol_min");
        const auto* protocol_max = field<std::uint64_t>(*object, "protocol_max");
        const auto* required_features = field<Array>(*object, "required_features");
        const auto* details = field<Object>(*object, "input");
        const auto* task_revision = field<std::uint64_t>(*object, "task_revision");
        const auto* slot_updates = field<Array>(*object, "slot_updates");
        if (request_id == nullptr || parent_id == nullptr || idempotency == nullptr ||
            capabilities == nullptr || protocol_min == nullptr || protocol_max == nullptr ||
            required_features == nullptr || details == nullptr || task_revision == nullptr ||
            slot_updates == nullptr ||
            *protocol_min < kMinimumAgentProtocolVersion ||
            *protocol_max > kAgentProtocolVersion || *protocol_min > *protocol_max)
            return {false, {}, "agent request v3 fields are invalid"};
        const auto parsed_capabilities = decode_string_array(*capabilities);
        const auto parsed_required = decode_string_array(*required_features);
        const auto parsed_details = decode_input_payload(*details);
        const auto parsed_updates = decode_slot_updates(*slot_updates);
        if (!parsed_capabilities || !parsed_required || !parsed_details || !parsed_updates ||
            !token(*request_id) ||
            !valid_optional_token(*parent_id) || !token(*idempotency))
            return {false, {}, "agent request v3 metadata is invalid"};
        result.value.request_id = *request_id;
        result.value.parent_request_id = *parent_id;
        result.value.idempotency_key = *idempotency;
        result.value.capabilities = *parsed_capabilities;
        result.value.protocol_min = static_cast<std::uint32_t>(*protocol_min);
        result.value.protocol_max = static_cast<std::uint32_t>(*protocol_max);
        result.value.required_features = *parsed_required;
        result.value.input_details = *parsed_details;
        result.value.task_revision = *task_revision;
        result.value.slot_updates = *parsed_updates;
        if (has_selection) {
            const auto* selected = field<Object>(*object, "selection");
            if (!selected || selected->size() != 3)
                return {false, {}, "selection fields are invalid"};
            const auto* text = field<std::string>(*selected, "text");
            const auto* digest = field<std::string>(*selected, "sha256");
            const auto* redacted = field<bool>(*selected, "redacted");
            if (!text || !digest || !redacted) return {false, {}, "selection type is invalid"};
            result.value.selection = SelectionSummary{*text, *digest, *redacted};
        }
    } else {
        result.value.input_details.raw_pinyin = result.value.user_input;
        result.value.input_details.natural_language = result.value.user_input;
    }
    for (const auto& value : *context_entries) {
        const auto* item = std::get_if<Object>(&value);
        if (item == nullptr || item->size() != (v3 ? 10U : 4U))
            return {false, {}, "context entry is invalid"};
        const auto* context_id = field<std::string>(*item, "context_id");
        const auto* entry_application = field<Object>(*item, "application");
        const auto* text = field<std::string>(*item, "text");
        const auto* sequence = field<std::uint64_t>(*item, "sequence");
        if (context_id == nullptr || entry_application == nullptr || text == nullptr ||
            sequence == nullptr) return {false, {}, "context entry field is invalid"};
        const auto parsed_entry_application = decode_application(*entry_application);
        if (!parsed_entry_application || parsed_entry_application->context_id != *context_id)
            return {false, {}, "context entry application mismatch"};
        ContextEntry entry{*context_id, *parsed_entry_application, *text, *sequence};
        if (v3) {
            const auto* type = field<std::string>(*item, "context_type");
            const auto* source = field<std::string>(*item, "source");
            const auto* created = field<std::uint64_t>(*item, "created_at_ms");
            const auto* relevance = field<std::uint64_t>(*item, "relevance_milli");
            const auto* current = field<bool>(*item, "is_current");
            const auto* privacy = field<Object>(*item, "privacy");
            if (type == nullptr || source == nullptr || created == nullptr || relevance == nullptr ||
                *relevance > 1000 || current == nullptr || privacy == nullptr ||
                privacy->size() != 3) return {false, {}, "context metadata is invalid"};
            const auto* filtered = field<bool>(*privacy, "filtered");
            const auto* redacted = field<bool>(*privacy, "redacted");
            const auto* truncated = field<bool>(*privacy, "truncated");
            if (filtered == nullptr || redacted == nullptr || truncated == nullptr)
                return {false, {}, "context privacy metadata is invalid"};
            entry.context_type = *type;
            entry.source = *source;
            entry.created_at_ms = *created;
            entry.relevance_milli = static_cast<std::uint32_t>(*relevance);
            entry.is_current = *current;
            entry.filtered = *filtered;
            entry.redacted = *redacted;
            entry.truncated = *truncated;
        }
        result.value.context_entries.push_back(std::move(entry));
    }
    if ((v3 ? encode_request(result.value) : encode_request_v2(result.value)).empty())
        return {false, {}, "agent request bounds are invalid"};
    result.ok = true;
    return result;
}

std::string encode_response_v2(const Response& response) {
    if (!session_id(response.session_id) || response.message.size() > 16384 ||
        response.executing_command.size() > 4096 || response.error_code.size() > 64 ||
        response.commands.size() > kMaximumAgentCommands || !valid_utf8(response.message) ||
        !valid_utf8(response.executing_command) ||
        (!response.error_code.empty() && !token(response.error_code, 64))) return {};
    std::string commands = "[";
    for (std::size_t index = 0; index < response.commands.size(); ++index) {
        const auto& command = response.commands[index];
        if (!token(command.id) || command.label.empty() || command.label.size() > 512 ||
            !valid_utf8(command.label)) return {};
        if (index != 0) commands += ',';
        commands += "{\"id\":\"" + escape(command.id) + "\",\"label\":\"" +
            escape(command.label) + "\",\"high_risk\":" +
            (command.high_risk ? "true" : "false") + "}";
    }
    commands += ']';
    auto output = std::string("{\"schema_version\":") +
        std::to_string(kMinimumAgentProtocolVersion) + ",\"session_id\":\"" +
        response.session_id + "\",\"message\":\"" + escape(response.message) +
        "\",\"commands\":" + commands + ",\"executing_command\":\"" +
        escape(response.executing_command) + "\",\"status\":\"" +
        status_name(response.status) + "\",\"page\":" + std::to_string(response.page) +
        ",\"has_more\":" + (response.has_more ? "true" : "false") +
        ",\"error_code\":\"" + escape(response.error_code) + "\"}";
    return output.size() <= kMaximumAgentPayloadBytes ? output : std::string{};
}

std::string encode_response(const Response& response) {
    if (!session_id(response.session_id) || response.message.size() > 16384 ||
        response.executing_command.size() > 4096 || response.error_code.size() > 64 ||
        response.error_message.size() > 4096 || response.commands.size() > kMaximumAgentCommands ||
        response.capabilities.size() > 32 || response.progress > 100 ||
        !valid_utf8(response.message) || !valid_utf8(response.executing_command) ||
        !valid_utf8(response.error_message) || !token(response.request_id) ||
        (!response.error_code.empty() && !token(response.error_code, 64)) ||
        !valid_task(response.task)) return {};
    const auto capabilities = encode_string_array(response.capabilities);
    const auto task = encode_task(response.task);
    if (capabilities.empty() || task.empty()) return {};
    const bool effects_negotiated =
        std::find(response.capabilities.begin(), response.capabilities.end(),
                  kAgentCapabilityOutputEffects) != response.capabilities.end();
    std::string commands = "[";
    for (std::size_t index = 0; index < response.commands.size(); ++index) {
        const auto& command = response.commands[index];
        auto risk = command.risk_level;
        if (command.high_risk && risk == "none") risk = "high";
        const auto slot_updates = encode_slot_updates(command.slot_updates);
        if (!token(command.id) || command.label.empty() || !valid_text(command.label, 512) ||
            !valid_text(command.description, 2048) || !valid_optional_token(command.category, 96) ||
            !valid_risk_level(risk) || !valid_text(command.preview, 2048) ||
            !valid_text(command.disabled_reason, 512) ||
            ((!command.enabled) && command.disabled_reason.empty()) || slot_updates.empty() ||
            ((!command.slot_updates.empty() || command.commit_task) &&
             command.task_revision == 0) ||
            ((risk == "high" || risk == "critical") && !command.high_risk) ||
            (command.effect && (!effects_negotiated || !valid_effect(*command.effect))))
            return {};
        if (index != 0) commands += ',';
        commands += "{\"id\":\"" + escape(command.id) + "\",\"label\":\"" +
            escape(command.label) + "\",\"high_risk\":" +
            (command.high_risk ? "true" : "false") + ",\"description\":\"" +
            escape(command.description) + "\",\"category\":\"" + escape(command.category) +
            "\",\"risk_level\":\"" + escape(risk) +
            "\",\"requires_confirmation\":" +
            ((command.requires_confirmation || command.high_risk) ? "true" : "false") +
            ",\"preview\":\"" + escape(command.preview) + "\",\"enabled\":" +
            (command.enabled ? "true" : "false") + ",\"disabled_reason\":\"" +
            escape(command.disabled_reason) + "\",\"task_revision\":" +
            std::to_string(command.task_revision) + ",\"slot_updates\":" + slot_updates +
            ",\"commit_task\":" + (command.commit_task ? "true" : "false");
        if (command.effect) {
            const auto& effect = *command.effect;
            commands += std::string(",\"effect\":{\"kind\":\"") +
                effect_kind_name(effect.kind) + "\",\"text\":\"" + escape(effect.text) +
                "\",\"target_context_id\":\"" + escape(effect.target_context_id) +
                "\",\"selection_sha256\":\"" + escape(effect.selection_sha256) + "\"}";
        }
        commands += '}';
    }
    commands += ']';
    auto output = std::string("{\"schema_version\":3,\"session_id\":\"") +
        response.session_id + "\",\"request_id\":\"" + escape(response.request_id) +
        "\",\"message\":\"" + escape(response.message) + "\",\"commands\":" + commands +
        ",\"executing_command\":\"" + escape(response.executing_command) +
        "\",\"status\":\"" + status_name(response.status) +
        "\",\"page\":" + std::to_string(response.page) +
        ",\"has_more\":" + (response.has_more ? "true" : "false") +
        ",\"error_code\":\"" + escape(response.error_code) +
        "\",\"state_revision\":" + std::to_string(response.state_revision) +
        ",\"progress\":" + std::to_string(response.progress) +
        ",\"retry_after_ms\":" + std::to_string(response.retry_after_ms) +
        ",\"can_cancel\":" + (response.can_cancel ? "true" : "false") +
        ",\"can_continue_input\":" + (response.can_continue_input ? "true" : "false") +
        ",\"expires_at_ms\":" + std::to_string(response.expires_at_ms) +
        ",\"error_message\":\"" + escape(response.error_message) +
        "\",\"retryable\":" + (response.retryable ? "true" : "false") +
        ",\"capabilities\":" + capabilities + ",\"task\":" + task + "}";
    return output.size() <= kMaximumAgentPayloadBytes ? output : std::string{};
}

ResponseDecodeResult decode_response(const std::string_view json) {
    ResponseDecodeResult result;
    if (json.empty() || json.size() > kMaximumAgentPayloadBytes)
        return {false, {}, "agent response size is invalid"};
    JsonValue root;
    Parser parser(json);
    if (!parser.parse(root)) return {false, {}, parser.diagnostic()};
    const auto* object = std::get_if<Object>(&root);
    if (object == nullptr) return {false, {}, "agent response fields mismatch"};
    const auto* version = field<std::uint64_t>(*object, "schema_version");
    if (version == nullptr || (*version != 2 && *version != 3))
        return {false, {}, "unsupported agent response version"};
    const bool v3 = *version == 3;
    if (object->size() != (v3 ? 20U : 9U))
        return {false, {}, "agent response fields mismatch"};
    const auto* id = field<std::string>(*object, "session_id");
    const auto* message = field<std::string>(*object, "message");
    const auto* commands = field<Array>(*object, "commands");
    const auto* executing = field<std::string>(*object, "executing_command");
    const auto* status = field<std::string>(*object, "status");
    const auto* page = field<std::uint64_t>(*object, "page");
    const auto* more = field<bool>(*object, "has_more");
    const auto* error = field<std::string>(*object, "error_code");
    if (id == nullptr || message == nullptr || commands == nullptr || executing == nullptr ||
        status == nullptr || page == nullptr ||
        *page > 1000 || more == nullptr || error == nullptr)
        return {false, {}, "agent response field type is invalid"};
    const auto parsed_status = parse_status(*status);
    if (!parsed_status || !session_id(*id)) return {false, {}, "agent response identity is invalid"};
    result.value.session_id = *id;
    result.value.message = *message;
    result.value.executing_command = *executing;
    result.value.status = *parsed_status;
    result.value.page = static_cast<std::uint32_t>(*page);
    result.value.has_more = *more;
    result.value.error_code = *error;
    result.value.schema_version = static_cast<std::uint32_t>(*version);
    if (v3) {
        const auto* request_id = field<std::string>(*object, "request_id");
        const auto* revision = field<std::uint64_t>(*object, "state_revision");
        const auto* progress = field<std::uint64_t>(*object, "progress");
        const auto* retry_after = field<std::uint64_t>(*object, "retry_after_ms");
        const auto* can_cancel = field<bool>(*object, "can_cancel");
        const auto* can_continue = field<bool>(*object, "can_continue_input");
        const auto* expires = field<std::uint64_t>(*object, "expires_at_ms");
        const auto* error_message = field<std::string>(*object, "error_message");
        const auto* retryable = field<bool>(*object, "retryable");
        const auto* capabilities = field<Array>(*object, "capabilities");
        const auto* task = field<Object>(*object, "task");
        if (request_id == nullptr || revision == nullptr || progress == nullptr || *progress > 100 ||
            retry_after == nullptr || *retry_after > 300000 || can_cancel == nullptr ||
            can_continue == nullptr || expires == nullptr || error_message == nullptr ||
            retryable == nullptr || capabilities == nullptr || task == nullptr)
            return {false, {}, "agent response v3 fields are invalid"};
        const auto parsed_capabilities = decode_string_array(*capabilities);
        const auto parsed_task = decode_task(*task);
        if (!parsed_capabilities || !parsed_task || !token(*request_id))
            return {false, {}, "agent response v3 metadata is invalid"};
        result.value.request_id = *request_id;
        result.value.state_revision = *revision;
        result.value.progress = static_cast<std::uint32_t>(*progress);
        result.value.retry_after_ms = static_cast<std::uint32_t>(*retry_after);
        result.value.can_cancel = *can_cancel;
        result.value.can_continue_input = *can_continue;
        result.value.expires_at_ms = *expires;
        result.value.error_message = *error_message;
        result.value.retryable = *retryable;
        result.value.capabilities = *parsed_capabilities;
        result.value.task = *parsed_task;
    }
    std::unordered_set<std::string> command_ids;
    for (const auto& value : *commands) {
        const auto* item = std::get_if<Object>(&value);
        const bool has_effect = v3 && item != nullptr && item->contains("effect");
        if (item == nullptr || item->size() != (v3 ? (has_effect ? 14U : 13U) : 3U))
            return {false, {}, "invalid agent command"};
        const auto* command_id = field<std::string>(*item, "id");
        const auto* label = field<std::string>(*item, "label");
        const auto* high_risk = field<bool>(*item, "high_risk");
        if (command_id == nullptr || label == nullptr || high_risk == nullptr)
            return {false, {}, "invalid agent command fields"};
        if (!command_ids.insert(*command_id).second)
            return {false, {}, "duplicate agent command id"};
        Command parsed{*command_id, *label, *high_risk};
        if (v3) {
            const auto* description = field<std::string>(*item, "description");
            const auto* category = field<std::string>(*item, "category");
            const auto* risk = field<std::string>(*item, "risk_level");
            const auto* confirmation = field<bool>(*item, "requires_confirmation");
            const auto* preview = field<std::string>(*item, "preview");
            const auto* enabled = field<bool>(*item, "enabled");
            const auto* disabled = field<std::string>(*item, "disabled_reason");
            const auto* task_revision = field<std::uint64_t>(*item, "task_revision");
            const auto* slot_updates = field<Array>(*item, "slot_updates");
            const auto* commit_task = field<bool>(*item, "commit_task");
            if (description == nullptr || category == nullptr || risk == nullptr ||
                confirmation == nullptr || preview == nullptr || enabled == nullptr ||
                disabled == nullptr || task_revision == nullptr || slot_updates == nullptr ||
                commit_task == nullptr)
                return {false, {}, "invalid agent command metadata"};
            parsed.description = *description;
            parsed.category = *category;
            parsed.risk_level = *risk;
            parsed.requires_confirmation = *confirmation;
            parsed.preview = *preview;
            parsed.enabled = *enabled;
            parsed.disabled_reason = *disabled;
            const auto parsed_updates = decode_slot_updates(*slot_updates);
            if (!parsed_updates) return {false, {}, "invalid agent command slot updates"};
            parsed.task_revision = *task_revision;
            parsed.slot_updates = *parsed_updates;
            parsed.commit_task = *commit_task;
            if (has_effect) {
                const auto* effect = field<Object>(*item, "effect");
                if (effect == nullptr || effect->size() != 4U)
                    return {false, {}, "invalid agent command effect"};
                const auto* kind = field<std::string>(*effect, "kind");
                const auto* text = field<std::string>(*effect, "text");
                const auto* target = field<std::string>(*effect, "target_context_id");
                const auto* digest = field<std::string>(*effect, "selection_sha256");
                const auto parsed_kind = kind ? parse_effect_kind(*kind) : std::nullopt;
                if (!parsed_kind || text == nullptr || target == nullptr || digest == nullptr)
                    return {false, {}, "invalid agent command effect"};
                parsed.effect = CommandEffect{*parsed_kind, *text, *target, *digest};
            }
            if (!valid_risk_level(parsed.risk_level) ||
                ((parsed.risk_level == "high" || parsed.risk_level == "critical") &&
                 (!parsed.high_risk || !parsed.requires_confirmation)) ||
                (!parsed.enabled && parsed.disabled_reason.empty()))
                return {false, {}, "invalid agent command risk metadata"};
        }
        result.value.commands.push_back(std::move(parsed));
    }
    if ((v3 ? encode_response(result.value) : encode_response_v2(result.value)).empty())
        return {false, {}, "agent response bounds are invalid"};
    result.ok = true;
    return result;
}

const char* effect_kind_name(const EffectKind kind) noexcept {
    switch (kind) {
    case EffectKind::commit_text: return "commit_text";
    case EffectKind::replace_selection: return "replace_selection";
    case EffectKind::copy_text: return "copy_text";
    }
    return "commit_text";
}

EffectDecision decide_effect(const CommandEffect& effect,
                             const EffectTargetFacts& facts) noexcept {
    if (!facts.capability_negotiated || !valid_effect(effect)) return EffectDecision::preview_only;
    if (effect.kind == EffectKind::copy_text) return EffectDecision::write;
    // Document writes require a binding to the originating input position.
    if (!facts.same_document || !facts.context_allowed || effect.target_context_id.empty() ||
        effect.target_context_id != facts.current_context_id)
        return EffectDecision::preview_only;
    if (effect.kind == EffectKind::replace_selection &&
        effect.selection_sha256 != facts.selection_sha256)
        return EffectDecision::preview_only;
    return EffectDecision::write;
}

bool context_entry_shareable(const ContextEntry& entry,
                             const ApplicationContext& current) noexcept {
    if (current.sensitive_input || entry.application.sensitive_input) return false;
    if (!current.context_id.empty() && entry.context_id == current.context_id) return true;
    return !current.executable.empty() && entry.application.executable == current.executable;
}

FilterResult filter_sensitive_text(const std::string_view utf8,
                                   const std::size_t maximum_bytes) {
    if (!valid_utf8(utf8)) return {false, {}, false, "sensitive filter received invalid UTF-8"};
    std::size_t output_size = std::min(utf8.size(), maximum_bytes);
    if (output_size < utf8.size()) {
        while (output_size != 0 &&
               (static_cast<unsigned char>(utf8[output_size]) & 0xc0U) == 0x80U)
            --output_size;
    }
    std::string output(utf8.substr(0, output_size));
    bool redacted = output.size() != utf8.size();
    std::string lower = output;
    std::transform(lower.begin(), lower.end(), lower.begin(), [](const unsigned char byte) {
        return static_cast<char>(std::tolower(byte));
    });
    constexpr std::array<std::string_view, 10> keys{
        "password", "passwd", "pwd", "token", "secret", "api_key", "apikey",
        "authorization", "bearer", "credential"};
    for (const auto key : keys) {
        std::size_t found = 0;
        while ((found = lower.find(key, found)) != std::string::npos) {
            auto begin = lower.find_first_of(":=", found + key.size());
            if (begin == std::string::npos || begin - found > 32) { found += key.size(); continue; }
            ++begin;
            while (begin < output.size() && std::isspace(static_cast<unsigned char>(output[begin])))
                ++begin;
            auto end = begin;
            while (end < output.size() && !std::isspace(static_cast<unsigned char>(output[end])) &&
                   output[end] != '&' && output[end] != ';' && output[end] != ',') ++end;
            if (end > begin) {
                output.replace(begin, end - begin, "[REDACTED]");
                lower.replace(begin, end - begin, "[redacted]");
                redacted = true;
                found = begin + 10;
            } else found = begin;
        }
    }
    redact_identifiers(output, redacted);
    redact_labeled_secrets(output, redacted);
    return {true, std::move(output), redacted, {}};
}

Request make_poll_request(const Request& origin, const std::uint64_t sequence) {
    Request poll;
    poll.action = Action::poll;
    poll.session_id = origin.session_id;
    poll.request_id = origin.request_id + ".ui-poll." + std::to_string(sequence);
    poll.parent_request_id = origin.request_id;
    poll.idempotency_key = poll.request_id;
    poll.capabilities = origin.capabilities;
    poll.application = origin.application;
    poll.selection = origin.selection;
    return poll;
}

}  // namespace owo::agent

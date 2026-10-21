#include "owo/engine/smart_candidates.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <optional>
#include <sstream>
#include <string>
#include <string_view>
#include <unordered_set>

namespace owo::engine {
namespace {

bool is_digit(const char value) { return value >= '0' && value <= '9'; }

bool is_ascii_letter(const char value) {
    return (value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z');
}

bool is_ascii_lower(const char value) { return value >= 'a' && value <= 'z'; }

bool is_ascii_upper(const char value) { return value >= 'A' && value <= 'Z'; }

bool is_word_character(const char value) {
    return is_ascii_letter(value) || is_digit(value) || value == '_';
}

bool is_space(const char value) {
    return value == ' ' || value == '\t' || value == '\r' || value == '\n';
}

Candidate whole_input_candidate(std::string text, const std::int64_t score,
                                const std::size_t consumed) {
    Candidate candidate;
    candidate.text = std::move(text);
    candidate.score = score;
    candidate.match_kind = InputMatchKind::exact;
    candidate.consumed_input_bytes = consumed;
    candidate.segment_count = 1;
    return candidate;
}

std::string format_number(const double value) {
    if (std::isfinite(value) && std::floor(value) == value &&
        std::abs(value) < 1e15) {
        return std::to_string(static_cast<long long>(std::llround(value)));
    }
    std::ostringstream out;
    out.precision(12);
    out << value;
    return out.str();
}

// Minimal recursive-descent evaluator for one ASCII arithmetic expression.
// Operators are limited to the ones a keyboard exposes, and failures are
// silent: a malformed expression simply produces no candidate.
class ArithmeticParser {
public:
    explicit ArithmeticParser(const std::string_view text) : text_(text) {}

    [[nodiscard]] bool evaluate(double& value) {
        value = parse_expression();
        skip_spaces();
        return !failed_ && position_ == text_.size();
    }

    [[nodiscard]] bool has_operator() const { return operator_count_ != 0; }

private:
    void skip_spaces() {
        while (position_ < text_.size() && is_space(text_[position_])) ++position_;
    }

    double parse_expression() {
        double value = parse_term();
        for (;;) {
            skip_spaces();
            if (position_ >= text_.size()) break;
            const char operation = text_[position_];
            if (operation != '+' && operation != '-') break;
            ++position_;
            ++operator_count_;
            const double right = parse_term();
            value = operation == '+' ? value + right : value - right;
        }
        return value;
    }

    double parse_term() {
        double value = parse_power();
        for (;;) {
            skip_spaces();
            if (position_ >= text_.size()) break;
            const char operation = text_[position_];
            if (operation != '*' && operation != '/' && operation != '%') break;
            ++position_;
            ++operator_count_;
            const double right = parse_power();
            if (operation == '*') {
                value *= right;
            } else if (operation == '/') {
                if (right == 0.0) { failed_ = true; return 0.0; }
                value /= right;
            } else {
                if (right == 0.0) { failed_ = true; return 0.0; }
                value = std::fmod(value, right);
            }
        }
        return value;
    }

    double parse_power() {
        const double value = parse_unary();
        skip_spaces();
        if (position_ < text_.size() && text_[position_] == '^') {
            ++position_;
            ++operator_count_;
            const double exponent = parse_power();
            return std::pow(value, exponent);
        }
        return value;
    }

    double parse_unary() {
        skip_spaces();
        if (position_ < text_.size() &&
            (text_[position_] == '-' || text_[position_] == '+')) {
            const char sign = text_[position_++];
            const double value = parse_unary();
            return sign == '-' ? -value : value;
        }
        return parse_primary();
    }

    double parse_primary() {
        skip_spaces();
        if (position_ >= text_.size()) { failed_ = true; return 0.0; }
        if (text_[position_] == '(') {
            ++position_;
            const double value = parse_expression();
            skip_spaces();
            if (position_ >= text_.size() || text_[position_] != ')') {
                failed_ = true;
                return 0.0;
            }
            ++position_;
            return value;
        }
        return parse_number();
    }

    double parse_number() {
        skip_spaces();
        const auto begin = position_;
        while (position_ < text_.size() && is_digit(text_[position_])) ++position_;
        if (position_ == begin) { failed_ = true; return 0.0; }
        if (position_ + 1 < text_.size() && text_[position_] == '.' &&
            is_digit(text_[position_ + 1])) {
            ++position_;
            while (position_ < text_.size() && is_digit(text_[position_])) ++position_;
        }
        try {
            return std::stod(std::string(text_.substr(begin, position_ - begin)));
        } catch (...) {
            failed_ = true;
            return 0.0;
        }
    }

    std::string_view text_;
    std::size_t position_{};
    std::size_t operator_count_{};
    bool failed_{};
};

int days_in_month(const int year, const int month) {
    constexpr std::array<int, 12> kDays{31, 28, 31, 30, 31, 30,
                                        31, 31, 30, 31, 30, 31};
    if (month < 1 || month > 12) return 0;
    if (month != 2) return kDays[static_cast<std::size_t>(month - 1)];
    const bool leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    return leap ? 29 : 28;
}

struct ParsedDate {
    std::size_t begin{};
    std::size_t end{};
    int year{};
    int month{};
    int day{};
    bool cjk{};
    bool valid{true};

    // Ordering ignores the surface separator: only the calendar value matters.
    [[nodiscard]] bool before(const ParsedDate& other) const noexcept {
        if (year != other.year) return year < other.year;
        if (month != other.month) return month < other.month;
        return day < other.day;
    }
};

std::optional<int> take_digits(const std::string_view text, std::size_t& position,
                               const std::size_t minimum,
                               const std::size_t maximum) {
    const auto begin = position;
    while (position < text.size() && is_digit(text[position]) &&
           position - begin < maximum)
        ++position;
    if (position - begin < minimum) return std::nullopt;
    int value = 0;
    for (auto index = begin; index < position; ++index)
        value = value * 10 + (text[index] - '0');
    return value;
}

std::optional<ParsedDate> parse_compact_date(const std::string_view raw) {
    if (raw.size() != 8) return std::nullopt;
    for (const char character : raw)
        if (!is_digit(character)) return std::nullopt;
    const auto digits = [raw](const std::size_t begin, const std::size_t count) {
        int value = 0;
        for (std::size_t index = 0; index < count; ++index)
            value = value * 10 + (raw[begin + index] - '0');
        return value;
    };
    const int year = digits(0, 4);
    const int month = digits(4, 2);
    const int day = digits(6, 2);
    if (year < 1900 || year > 2200) return std::nullopt;
    if (month < 1 || month > 12) return std::nullopt;
    if (day < 1 || day > days_in_month(year, month)) return std::nullopt;
    ParsedDate date;
    date.begin = 0;
    date.end = raw.size();
    date.year = year;
    date.month = month;
    date.day = day;
    date.valid = true;
    return date;
}

std::vector<ParsedDate> collect_calendar_dates(const std::string_view text) {
    std::vector<ParsedDate> dates;
    for (std::size_t begin = 0; begin + 6 < text.size(); ++begin) {
        if (!is_digit(text[begin])) continue;
        if (begin > 0 && is_digit(text[begin - 1])) continue;
        std::size_t position = begin;
        const auto year = take_digits(text, position, 4, 4);
        if (!year || *year < 1900 || *year > 2200) continue;
        std::string_view separator;
        if (position < text.size() && text.substr(position).starts_with("-"))
            separator = "-";
        else if (position < text.size() && text.substr(position).starts_with("/"))
            separator = "/";
        else if (position < text.size() && text.substr(position).starts_with("."))
            separator = ".";
        else if (position < text.size() && text.substr(position).starts_with("年"))
            separator = "年";
        else
            continue;
        position += separator.size();
        const auto month = take_digits(text, position, 1, 2);
        if (!month) continue;
        std::string_view month_separator;
        if (separator == "年") {
            if (!text.substr(position).starts_with("月")) continue;
            month_separator = "月";
        } else {
            if (!text.substr(position).starts_with(separator)) continue;
            month_separator = separator;
        }
        position += month_separator.size();
        const auto day = take_digits(text, position, 1, 2);
        if (!day) continue;
        if (separator == "年") {
            if (!text.substr(position).starts_with("日")) continue;
            position += std::string_view("日").size();
        }
        ParsedDate date;
        date.begin = begin;
        date.end = position;
        date.year = *year;
        date.month = *month;
        date.day = *day;
        date.cjk = separator == "年";
        date.valid = date.month >= 1 && date.month <= 12 && date.day >= 1 &&
                     date.day <= days_in_month(date.year, date.month);
        dates.push_back(date);
        // Jump past this date so a later invalid one is still discovered.
        begin = position - 1;
    }
    return dates;
}

std::string corrected_date_text(const ParsedDate& date) {
    int month = date.month;
    int year = date.year;
    while (month > 12) { month -= 12; ++year; }
    if (month < 1) month = 1;
    const int maximum_day = days_in_month(year, month);
    int day = date.day;
    if (day < 1) day = 1;
    if (day > maximum_day) day = maximum_day;
    std::ostringstream out;
    out.width(4);
    out.fill('0');
    out << year;
    out.fill('0');
    if (date.cjk) {
        out.width(2);
        out << month << "月";
        out.width(2);
        out << day << "日";
    } else {
        out << '-';
        out.width(2);
        out << month;
        // The caller substitutes the original separator family.
        out << '-';
        out.width(2);
        out << day;
    }
    return out.str();
}

// A date range distinguishes "2026-03-05到2026-01-01" from two unrelated
// dates: only connector characters (and whitespace) may sit between them.
bool range_connector_only(const std::string_view text) {
    constexpr std::array<std::string_view, 6> kConnectors{"到", "至", "~", "～",
                                                          "—", "-"};
    bool found = false;
    std::size_t position = 0;
    while (position < text.size()) {
        bool matched = false;
        for (const auto connector : kConnectors) {
            if (!text.substr(position).starts_with(connector)) continue;
            found = true;
            position += connector.size();
            matched = true;
            break;
        }
        if (matched) continue;
        if (is_space(text[position])) {
            ++position;
            continue;
        }
        return false;
    }
    return found;
}

std::optional<std::string> swapped_date_range(const std::string_view text,
                                              const ParsedDate& first,
                                              const ParsedDate& second) {
    if (!range_connector_only(text.substr(first.end, second.begin - first.end)))
        return std::nullopt;
    if (!second.before(first)) return std::nullopt;
    std::string swapped;
    swapped.reserve(text.size());
    swapped.append(text.substr(0, first.begin));
    swapped.append(text.substr(second.begin, second.end - second.begin));
    swapped.append(text.substr(first.end, second.begin - first.end));
    swapped.append(text.substr(first.begin, first.end - first.begin));
    swapped.append(text.substr(second.end));
    return swapped;
}

std::size_t utf8_unit_size(const unsigned char lead) {
    if ((lead & 0x80U) == 0) return 1;
    if ((lead & 0xE0U) == 0xC0U) return 2;
    if ((lead & 0xF0U) == 0xE0U) return 3;
    if ((lead & 0xF8U) == 0xF0U) return 4;
    return 1;
}

struct Utf8Unit {
    std::uint32_t codepoint{};
    std::size_t begin{};
    std::size_t end{};
};

std::vector<Utf8Unit> utf8_units(const std::string_view text) {
    std::vector<Utf8Unit> units;
    units.reserve(text.size());
    for (std::size_t position = 0; position < text.size();) {
        const auto lead = static_cast<unsigned char>(text[position]);
        const auto size = (std::min)(utf8_unit_size(lead), text.size() - position);
        const auto byte = [&text](const std::size_t index) {
            return static_cast<std::uint32_t>(
                static_cast<unsigned char>(text[index]));
        };
        std::uint32_t codepoint = byte(position);
        if (size == 2)
            codepoint = ((byte(position) & 0x1FU) << 6) | (byte(position + 1) & 0x3FU);
        else if (size == 3)
            codepoint = ((byte(position) & 0x0FU) << 12) |
                        ((byte(position + 1) & 0x3FU) << 6) |
                        (byte(position + 2) & 0x3FU);
        else if (size == 4)
            codepoint = ((byte(position) & 0x07U) << 18) |
                        ((byte(position + 1) & 0x3FU) << 12) |
                        ((byte(position + 2) & 0x3FU) << 6) |
                        (byte(position + 3) & 0x3FU);
        units.push_back({codepoint, position, position + size});
        position += size;
    }
    return units;
}

// Ideographs and kana. CJK punctuation does not count, so 你好，world stays as
// typed and never gains a second space.
bool is_cjk_ideograph(const std::uint32_t codepoint) {
    return (codepoint >= 0x3040 && codepoint <= 0x30FF) ||
           (codepoint >= 0x31F0 && codepoint <= 0x31FF) ||
           (codepoint >= 0x3400 && codepoint <= 0x4DBF) ||
           (codepoint >= 0x4E00 && codepoint <= 0x9FFF) ||
           (codepoint >= 0xF900 && codepoint <= 0xFAFF) ||
           (codepoint >= 0x20000 && codepoint <= 0x3FFFF);
}

bool spacing_ascii_alnum(const std::uint32_t codepoint) {
    return (codepoint >= '0' && codepoint <= '9') ||
           (codepoint >= 'a' && codepoint <= 'z') ||
           (codepoint >= 'A' && codepoint <= 'Z');
}

// 2026年1月5日 is a fixed date format; the CJK/Latin spacing rule must not
// rewrite it into 2026 年 1 月 5 日. The same applies to clock units.
bool spacing_date_unit(const std::uint32_t codepoint) {
    switch (codepoint) {
        case 0x5E74U:  // 年
        case 0x6708U:  // 月
        case 0x65E5U:  // 日
        case 0x53F7U:  // 号
        case 0x65F6U:  // 时
        case 0x5206U:  // 分
        case 0x79D2U:  // 秒
            return true;
        default:
            return false;
    }
}

std::string insert_cjk_latin_spaces(const std::string_view text) {
    const auto units = utf8_units(text);
    std::string spaced;
    spaced.reserve(text.size() + units.size() / 4);
    for (std::size_t index = 0; index < units.size(); ++index) {
        if (index > 0) {
            const auto previous = units[index - 1].codepoint;
            const auto current = units[index].codepoint;
            const bool date_unit =
                spacing_date_unit(previous) || spacing_date_unit(current);
            const bool boundary = !date_unit &&
                                  ((is_cjk_ideograph(previous) && spacing_ascii_alnum(current)) ||
                                   (spacing_ascii_alnum(previous) && is_cjk_ideograph(current)));
            if (boundary && !spaced.empty() && spaced.back() != ' ') spaced.push_back(' ');
        }
        spaced.append(
            text.substr(units[index].begin, units[index].end - units[index].begin));
    }
    return spaced;
}

// ASCII punctuation between Chinese text becomes the full-width form. The rule
// only fires when the preceding character is an ideograph, so "hello, world"
// and "1,000" are untouched.
std::string normalize_punctuation(const std::string_view text) {
    const auto units = utf8_units(text);
    std::string normalized;
    normalized.reserve(text.size());
    std::size_t quote_index = 0;
    for (std::size_t index = 0; index < units.size(); ++index) {
        const auto codepoint = units[index].codepoint;
        if (codepoint > 0x7FU) {
            normalized.append(
                text.substr(units[index].begin, units[index].end - units[index].begin));
            continue;
        }
        const bool previous_ideograph =
            index > 0 && is_cjk_ideograph(units[index - 1].codepoint);
        const bool next_ideograph =
            index + 1 < units.size() && is_cjk_ideograph(units[index + 1].codepoint);
        std::string_view replacement;
        switch (static_cast<char>(codepoint)) {
            case ',': replacement = "，"; break;
            case '.': replacement = "。"; break;
            case '?': replacement = "？"; break;
            case '!': replacement = "！"; break;
            case ':': replacement = "："; break;
            case ';': replacement = "；"; break;
            case '(': replacement = "（"; break;
            case ')': replacement = "）"; break;
            case '"': replacement = next_ideograph || previous_ideograph
                                        ? (quote_index++ % 2 == 0 ? "“" : "”")
                                        : std::string_view{};
                break;
            default: break;
        }
        if (replacement.empty()) {
            normalized.push_back(static_cast<char>(codepoint));
            continue;
        }
        // Opening brackets look forward, closing ones look back; the rest use
        // the preceding ideograph so numbers and URLs keep their ASCII form.
        const bool applies = codepoint == '(' ? next_ideograph
                             : codepoint == ')' ? previous_ideograph
                             : codepoint == '"' ? true
                                                : previous_ideograph;
        if (applies)
            normalized.append(replacement);
        else
            normalized.push_back(static_cast<char>(codepoint));
    }
    return normalized;
}

// Full-width ASCII forms and the ideographic space collapse to ASCII. This is
// the "same text, different width" half of unified encoding.
// Only code-shaped full-width forms collapse to ASCII. Full-width Chinese
// punctuation such as ，。！？（） must keep its width: it is punctuation, not a
// mistyped ASCII character.
bool width_normalizable(const std::uint32_t codepoint) {
    if (codepoint >= 0xFF10U && codepoint <= 0xFF19U) return true;  // 0-9
    if (codepoint >= 0xFF21U && codepoint <= 0xFF3AU) return true;  // A-Z
    if (codepoint >= 0xFF41U && codepoint <= 0xFF5AU) return true;  // a-z
    switch (codepoint) {
        case 0xFF03U:  // #
        case 0xFF04U:  // $
        case 0xFF05U:  // %
        case 0xFF06U:  // &
        case 0xFF0AU:  // *
        case 0xFF0BU:  // +
        case 0xFF0DU:  // -
        case 0xFF0FU:  // /
        case 0xFF1CU:  // <
        case 0xFF1DU:  // =
        case 0xFF1EU:  // >
        case 0xFF20U:  // @
        case 0xFF3BU:  // [
        case 0xFF3DU:  // ]
        case 0xFF3EU:  // ^
        case 0xFF3FU:  // _
        case 0xFF5BU:  // {
        case 0xFF5CU:  // |
        case 0xFF5DU:  // }
        case 0xFF5EU:  // ~
            return true;
        default:
            return false;
    }
}

std::string normalize_full_width(const std::string_view text) {
    const auto units = utf8_units(text);
    std::string normalized;
    normalized.reserve(text.size());
    for (const auto& unit : units) {
        if (unit.codepoint == 0x3000U) {
            normalized.push_back(' ');
            continue;
        }
        if (width_normalizable(unit.codepoint)) {
            normalized.push_back(static_cast<char>(unit.codepoint - 0xFEE0U));
            continue;
        }
        normalized.append(text.substr(unit.begin, unit.end - unit.begin));
    }
    return normalized;
}

std::size_t count_token(const std::string_view text, const std::string_view token) {
    std::size_t count = 0;
    for (std::size_t position = 0;
         (position = text.find(token, position)) != std::string_view::npos;
         position += token.size())
        ++count;
    return count;
}

std::string collapse_repeated_punctuation(const std::string_view text) {
    constexpr std::array<std::string_view, 6> kRepeatable{{
        "。", "！", "？", "，", "、", "；",
    }};
    std::string collapsed;
    collapsed.reserve(text.size());
    for (std::size_t position = 0; position < text.size();) {
        bool handled = false;
        for (const auto token : kRepeatable) {
            if (!text.substr(position).starts_with(token)) continue;
            collapsed.append(token);
            position += token.size();
            while (text.substr(position).starts_with(token)) position += token.size();
            handled = true;
            break;
        }
        if (handled) continue;
        if ((text[position] == '!' || text[position] == '?') &&
            position + 1 < text.size() && text[position + 1] == text[position]) {
            const auto repeated = text[position];
            collapsed.push_back(repeated);
            position += 2;
            while (position < text.size() && text[position] == repeated) ++position;
            continue;
        }
        collapsed.push_back(text[position]);
        ++position;
    }
    return collapsed;
}

std::string close_balanced_pairs(const std::string_view text) {
    std::string closed(text);
    const auto open_round = count_token(text, "(");
    const auto close_round = count_token(text, ")");
    if (open_round > close_round) closed.append(open_round - close_round, ')');
    if (count_token(text, "\"") % 2 == 1) closed.push_back('"');
    const auto left_quote = count_token(text, "“");
    const auto right_quote = count_token(text, "”");
    if (left_quote > right_quote)
        for (std::size_t index = 0; index < left_quote - right_quote; ++index)
            closed.append("”");
    const auto left_bracket = count_token(text, "【");
    const auto right_bracket = count_token(text, "】");
    if (left_bracket > right_bracket)
        for (std::size_t index = 0; index < left_bracket - right_bracket; ++index)
            closed.append("】");
    const auto left_book = count_token(text, "《");
    const auto right_book = count_token(text, "》");
    if (left_book > right_book)
        for (std::size_t index = 0; index < left_book - right_book; ++index)
            closed.append("》");
    return closed;
}

std::optional<std::string> split_camel_or_separator(const std::string_view text) {
    if (text.empty() || text.size() > kSmartMaximumInputBytes) return std::nullopt;
    if (text.find(' ') != std::string_view::npos ||
        text.find('\t') != std::string_view::npos)
        return std::nullopt;
    // Separator splitting only makes sense for word-shaped input; a date or a
    // version number should not become a spaced list of digits.
    if (std::none_of(text.begin(), text.end(), is_ascii_letter)) return std::nullopt;
    bool separator_found = false;
    bool camel_found = false;
    std::string split;
    split.reserve(text.size() + 8);
    for (std::size_t index = 0; index < text.size(); ++index) {
        const char character = text[index];
        if (character == '_' || character == '-') {
            separator_found = true;
            if (!split.empty() && split.back() != ' ') split.push_back(' ');
            while (index + 1 < text.size() &&
                   (text[index + 1] == '_' || text[index + 1] == '-'))
                ++index;
            continue;
        }
        if (is_ascii_upper(character) && !split.empty() && split.back() != ' ') {
            const auto previous = text[index - 1];
            const bool previous_lower_or_digit = is_ascii_lower(previous) || is_digit(previous);
            const bool next_lower =
                index + 1 < text.size() && is_ascii_lower(text[index + 1]);
            if (previous_lower_or_digit || next_lower) {
                split.push_back(' ');
                camel_found = true;
            }
        }
        split.push_back(character);
    }
    if ((!separator_found && !camel_found) || split == text) return std::nullopt;
    return split;
}

std::string_view digit_text(const int digit, const bool formal) {
    static const std::array<std::string_view, 10> kLower{"零", "一", "二", "三",
                                                         "四", "五", "六", "七",
                                                         "八", "九"};
    static const std::array<std::string_view, 10> kFormal{"零", "壹", "贰", "叁",
                                                          "肆", "伍", "陆", "柒",
                                                          "捌", "玖"};
    const auto index = static_cast<std::size_t>(digit);
    return formal ? kFormal[index] : kLower[index];
}

// Chinese reading of a non-negative integer, grouped by 万/亿. Amounts are
// bounded by kSmartMaximumNumberDigits so the four group units always suffice.
std::string chinese_integer(unsigned long long value, const bool formal) {
    if (value == 0) return std::string(digit_text(0, formal));
    std::vector<unsigned int> groups;
    while (value != 0) {
        groups.push_back(static_cast<unsigned int>(value % 10000));
        value /= 10000;
    }
    constexpr std::array<std::string_view, 4> kLowerUnits{"", "十", "百", "千"};
    constexpr std::array<std::string_view, 4> kFormalUnits{"", "拾", "佰", "仟"};
    constexpr std::array<std::string_view, 4> kGroupUnits{"", "万", "亿", "万亿"};
    constexpr std::array<unsigned int, 4> kPlaceDivisors{1000, 100, 10, 1};
    std::string out;
    bool pending_zero = false;
    for (std::size_t index = groups.size(); index-- > 0;) {
        const auto group = groups[index];
        if (group == 0) {
            if (!out.empty()) pending_zero = true;
            continue;
        }
        if (!out.empty() && group < 1000) pending_zero = true;
        bool started = false;
        for (int place = 3; place >= 0; --place) {
            const auto divisor = kPlaceDivisors[static_cast<std::size_t>(3 - place)];
            const auto digit = (group / divisor) % 10;
            if (digit == 0) {
                if (started) pending_zero = true;
                continue;
            }
            if (pending_zero) {
                out += digit_text(0, formal);
                pending_zero = false;
            }
            const auto position = static_cast<std::size_t>(place);
            const bool ten_shortcut = !formal && place == 1 && digit == 1 &&
                                      out.empty() &&
                                      index == groups.size() - 1;
            if (ten_shortcut) {
                out += kLowerUnits[1];
            } else {
                out += digit_text(digit, formal);
                out += formal ? kFormalUnits[position] : kLowerUnits[position];
            }
            started = true;
        }
        out += kGroupUnits[index];
    }
    return out;
}

struct NumberShape {
    std::string integer_digits;
    std::string fraction_digits;
    bool percent{};
};

std::optional<NumberShape> parse_number(const std::string_view raw) {
    if (raw.empty()) return std::nullopt;
    auto text = raw;
    NumberShape shape;
    if (text.back() == '%') {
        shape.percent = true;
        text.remove_suffix(1);
    }
    if (text.empty()) return std::nullopt;
    bool seen_point = false;
    for (const char character : text) {
        if (character >= '0' && character <= '9') continue;
        if (character == '.' && !seen_point) {
            seen_point = true;
            continue;
        }
        return std::nullopt;
    }
    const auto point = text.find('.');
    shape.integer_digits = std::string(
        text.substr(0, point == std::string_view::npos ? text.size() : point));
    if (point != std::string_view::npos)
        shape.fraction_digits = std::string(text.substr(point + 1));
    if (shape.integer_digits.empty() ||
        (seen_point && shape.fraction_digits.empty()))
        return std::nullopt;
    return shape;
}

std::vector<std::string> split_tokens(const std::string_view text, const char separator) {    std::vector<std::string> tokens;
    std::size_t begin = 0;
    while (begin <= text.size()) {
        const auto end = text.find(separator, begin);
        const auto stop = end == std::string_view::npos ? text.size() : end;
        tokens.push_back(std::string(text.substr(begin, stop - begin)));
        if (end == std::string_view::npos) break;
        begin = stop + 1;
    }
    return tokens;
}

bool all_word_tokens(const std::vector<std::string>& tokens) {
    return std::all_of(tokens.begin(), tokens.end(), [](const std::string& token) {
        return !token.empty() &&
               std::all_of(token.begin(), token.end(), is_word_character);
    });
}

}  // namespace

std::vector<Candidate> generate_math_result_candidates(const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;
    // A whole-input date such as 2026-10-19 is also a subtraction expression;
    // dates must stay dates instead of yielding 1997 at the top of the bar.
    const auto dates = collect_calendar_dates(raw);
    if (!dates.empty() && dates.front().begin == 0 &&
        dates.front().end == raw.size())
        return candidates;
    auto expression = raw;
    bool equality_prefix = false;
    if (!expression.empty() && expression.front() == '=') {
        // Spreadsheet habit: "=1+2" is a formula, not a comparison.
        equality_prefix = true;
        expression.remove_prefix(1);
    }
    bool equality_suffix = false;
    if (!expression.empty() && expression.back() == '=') {
        equality_suffix = true;
        expression.remove_suffix(1);
    }
    if (expression.empty()) return candidates;
    constexpr std::string_view kAllowed = "+-*/%^().";
    for (const char character : expression) {
        if (is_space(character) || is_digit(character) ||
            kAllowed.find(character) != std::string_view::npos)
            continue;
        return candidates;
    }
    ArithmeticParser parser(expression);
    double value = 0.0;
    if (!parser.evaluate(value) || !parser.has_operator() || !std::isfinite(value))
        return candidates;
    auto text = format_number(value);
    if (equality_suffix)
        text = std::string(raw) + text;
    else if (equality_prefix)
        text = std::string(raw) + "=" + text;
    candidates.push_back(whole_input_candidate(std::move(text),
                                               kSmartMathResultScore, raw.size()));
    return candidates;
}

std::vector<Candidate> generate_text_lint_candidates(const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;
    const auto dates = collect_calendar_dates(raw);
    std::string corrected(raw);
    bool changed = false;
    std::optional<std::string> reordered;
    const auto invalid = std::find_if(
        dates.begin(), dates.end(),
        [](const ParsedDate& date) { return !date.valid; });
    if (invalid != dates.end()) {
        auto replacement = corrected_date_text(*invalid);
        if (!invalid->cjk) {
            // corrected_date_text always uses '-'; substitute the original
            // separator so 2026/13/05 stays in the slash family.
            const auto separator = raw[invalid->begin + 4];
            std::replace(replacement.begin(), replacement.end(), '-', separator);
        }
        corrected.replace(invalid->begin, invalid->end - invalid->begin, replacement);
        changed = true;
    } else if (dates.size() >= 2) {
        // Both dates are valid; only a reversed range is a contradiction.
        reordered = swapped_date_range(raw, dates[0], dates[1]);
    }
    const auto collapsed = collapse_repeated_punctuation(corrected);
    if (collapsed != corrected) {
        corrected = collapsed;
        changed = true;
    }
    const auto balanced = close_balanced_pairs(corrected);
    if (balanced != corrected) {
        corrected = balanced;
        changed = true;
    }
    const auto punctuated = normalize_punctuation(corrected);
    if (punctuated != corrected) {
        corrected = punctuated;
        changed = true;
    }
    if (changed && corrected != raw)
        candidates.push_back(whole_input_candidate(
            std::move(corrected), kSmartCorrectionScore, raw.size()));
    if (reordered && *reordered != raw)
        candidates.push_back(whole_input_candidate(
            std::move(*reordered), kSmartOrderScore, raw.size()));
    auto spaced = insert_cjk_latin_spaces(raw);
    if (spaced != raw)
        candidates.push_back(whole_input_candidate(
            std::move(spaced), kSmartSpacingScore, raw.size()));
    auto width_normalized = normalize_full_width(raw);
    if (width_normalized != raw)
        candidates.push_back(whole_input_candidate(
            std::move(width_normalized), kSmartWidthScore, raw.size()));
    return candidates;
}

std::vector<Candidate> generate_text_shape_candidates(const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;

    if (const auto split = split_camel_or_separator(raw)) {
        candidates.push_back(whole_input_candidate(*split, kSmartShapeScore,
                                                   raw.size()));
    }

    if (raw.find(' ') != std::string_view::npos && raw.find('\t') == std::string_view::npos) {
        const auto tokens = split_tokens(raw, ' ');
        if (tokens.size() >= 2 && all_word_tokens(tokens)) {
            std::string compact;
            std::string snake;
            for (const auto& token : tokens) {
                compact += token;
                if (!snake.empty()) snake.push_back('_');
                snake += token;
            }
            if (compact != raw)
                candidates.push_back(whole_input_candidate(std::move(compact),
                                                           kSmartShapeScore, raw.size()));
            if (snake != raw)
                candidates.push_back(whole_input_candidate(std::move(snake),
                                                           kSmartShapeScore - 100,
                                                           raw.size()));
        }
    }

    if (raw.find(',') != std::string_view::npos &&
        raw.find(' ') == std::string_view::npos) {
        const auto tokens = split_tokens(raw, ',');
        if (tokens.size() >= 3 && all_word_tokens(tokens)) {
            std::string reversed;
            for (auto iterator = tokens.rbegin(); iterator != tokens.rend(); ++iterator) {
                if (!reversed.empty()) reversed.push_back(',');
                reversed += *iterator;
            }
            if (reversed != raw)
                candidates.push_back(whole_input_candidate(std::move(reversed),
                                                           kSmartShapeScore - 200,
                                                           raw.size()));
        }
    }

    std::size_t gap = raw.size();
    while (gap > 0 && raw[gap - 1] == '_') --gap;
    if (raw.size() - gap >= 2) {
        std::string filled(raw.substr(0, gap));
        filled += "【待补】";
        candidates.push_back(whole_input_candidate(std::move(filled),
                                                   kSmartShapeScore + 300,
                                                   raw.size()));
    }
    return candidates;
}

std::vector<Candidate> generate_number_reading_candidates(
    const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;
    // An 8-digit compact date (20261019) belongs to the date rule, not to the
    // number reading ("二千零二十六万一千零一十九").
    if (parse_compact_date(raw)) return candidates;
    const auto shape = parse_number(raw);
    if (!shape) return candidates;
    const auto significant = shape->integer_digits.find_first_not_of('0');
    const auto integer_part = significant == std::string::npos
                                  ? std::string("0")
                                  : shape->integer_digits.substr(significant);
    if (integer_part.size() > kSmartMaximumNumberDigits) return candidates;
    // A single-digit value is not worth a reading; it would decorate ordinary
    // numeric typing without adding information.
    if (!shape->percent && shape->fraction_digits.empty() &&
        integer_part.size() == 1)
        return candidates;
    unsigned long long value = 0;
    for (const char character : integer_part)
        value = value * 10 + static_cast<unsigned long long>(character - '0');
    const auto lowercase = chinese_integer(value, false);
    std::string reading = shape->percent ? "百分之" + lowercase : lowercase;
    if (!shape->fraction_digits.empty()) {
        reading += "点";
        for (const char character : shape->fraction_digits)
            reading += digit_text(character - '0', false);
    }
    candidates.push_back(whole_input_candidate(reading, kSmartNumberScore,
                                               raw.size()));
    if (!shape->percent && shape->fraction_digits.empty() && value != 0)
        candidates.push_back(whole_input_candidate(chinese_integer(value, true),
                                                   kSmartNumberFormalScore,
                                                   raw.size()));
    return candidates;
}

// Days since 1970-01-01 (Howard Hinnant's civil-date algorithm).
long long days_since_epoch(const int year, const int month, const int day) {
    int y = year;
    const unsigned m = static_cast<unsigned>(month);
    const unsigned d = static_cast<unsigned>(day);
    y -= m <= 2U ? 1 : 0;
    const int era = (y >= 0 ? y : y - 399) / 400;
    const unsigned yoe = static_cast<unsigned>(y - era * 400);
    const unsigned shifted = static_cast<unsigned>(static_cast<int>(m) + (m > 2U ? -3 : 9));
    const unsigned doy = (153U * shifted + 2U) / 5U + d - 1U;
    const unsigned doe = yoe * 365U + yoe / 4U - yoe / 100U + doy;
    return static_cast<long long>(era) * 146097LL +
           static_cast<long long>(doe) - 719468LL;
}

std::string_view weekday_name(const int year, const int month, const int day) {
    constexpr std::array<std::string_view, 7> kWeekdays{
        "星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"};
    const auto days = days_since_epoch(year, month, day);
    // 0 = Sunday; 1970-01-01 was a Thursday.
    const auto index = static_cast<std::size_t>(((days % 7) + 7 + 4) % 7);
    return kWeekdays[index];
}

std::string chinese_date(const int year, const int month, const int day) {
    return std::to_string(year) + "年" + std::to_string(month) + "月" +
           std::to_string(day) + "日";
}

std::string iso_date(const int year, const int month, const int day) {
    std::ostringstream out;
    out.width(4);
    out.fill('0');
    out << year << '-';
    out.width(2);
    out << month << '-';
    out.width(2);
    out << day;
    return out.str();
}

std::vector<Candidate> generate_date_structure_candidates(
    const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;
    const auto dates = collect_calendar_dates(raw);
    if (dates.size() == 1 && dates.front().valid && dates.front().begin == 0 &&
        dates.front().end == raw.size()) {
        const auto& date = dates.front();
        if (date.cjk) {
            candidates.push_back(whole_input_candidate(
                iso_date(date.year, date.month, date.day), kSmartStructureScore,
                raw.size()));
            // A Chinese date is often typed to ask "which weekday is that?".
            const auto localized = chinese_date(date.year, date.month, date.day);
            candidates.push_back(whole_input_candidate(
                localized + " " + std::string(weekday_name(date.year, date.month,
                                                          date.day)),
                kSmartWeekdayScore, raw.size()));
        } else {
            const auto localized = chinese_date(date.year, date.month, date.day);
            candidates.push_back(whole_input_candidate(
                localized, kSmartStructureScore, raw.size()));
            candidates.push_back(whole_input_candidate(
                localized + " " + std::string(weekday_name(date.year, date.month,
                                                          date.day)),
                kSmartWeekdayScore, raw.size()));
        }
    }
    if (const auto compact = parse_compact_date(raw)) {
        candidates.push_back(whole_input_candidate(
            iso_date(compact->year, compact->month, compact->day),
            kSmartStructureScore, raw.size()));
        const auto localized =
            chinese_date(compact->year, compact->month, compact->day);
        candidates.push_back(whole_input_candidate(
            localized + " " +
                std::string(weekday_name(compact->year, compact->month,
                                         compact->day)),
            kSmartWeekdayScore, raw.size()));
    }
    if (dates.size() >= 2) {
        // A range that covers the whole input can be localized as one unit.
        // Partial ranges are left alone: replacing the whole input with only
        // the dates would drop the surrounding text.
        const auto& first = dates[0];
        const auto& second = dates[1];
        if (first.valid && second.valid && first.begin == 0 &&
            second.end == raw.size() &&
            range_connector_only(raw.substr(first.end, second.begin - first.end))) {
            const auto text =
                chinese_date(first.year, first.month, first.day) + " 至 " +
                chinese_date(second.year, second.month, second.day);
            if (text != raw)
                candidates.push_back(whole_input_candidate(
                    text, kSmartStructureScore - 50, raw.size()));
        }
    }
    const auto colon = raw.find(':');
    if (colon != std::string_view::npos &&
        raw.find(':', colon + 1) == std::string_view::npos) {
        const auto hours_text = raw.substr(0, colon);
        const auto minutes_text = raw.substr(colon + 1);
        const auto all_digits = [](const std::string_view text) {
            return !text.empty() &&
                   (std::all_of(text.begin(), text.end(), is_digit));
        };
        if (hours_text.size() <= 2 && minutes_text.size() <= 2 &&
            all_digits(hours_text) && all_digits(minutes_text)) {
            const int hours = std::stoi(std::string(hours_text));
            const int minutes = std::stoi(std::string(minutes_text));
            if (hours <= 23 && minutes <= 59) {
                std::ostringstream padded;
                padded.width(2);
                padded.fill('0');
                padded << hours << ':';
                padded.width(2);
                padded << minutes;
                if (padded.str() != raw)
                    candidates.push_back(whole_input_candidate(
                        padded.str(), kSmartStructureScore, raw.size()));
            }
        }
    }
    return candidates;
}

std::vector<Candidate> generate_symbol_candidates(const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;
    struct Replacement {
        std::string_view from;
        std::string_view to;
    };
    // Longest first so "->" never eats part of a longer arrow.
    constexpr std::array<Replacement, 8> kReplacements{{
        {"-->", "→"}, {"->", "→"}, {"<-", "←"}, {"=>", "⇒"},
        {"<=", "≤"},  {">=", "≥"}, {"!=", "≠"}, {"...", "…"},
    }};
    std::string replaced;
    replaced.reserve(raw.size() + 4);
    bool changed = false;
    for (std::size_t position = 0; position < raw.size();) {
        bool matched = false;
        for (const auto& replacement : kReplacements) {
            if (!raw.substr(position).starts_with(replacement.from)) continue;
            replaced.append(replacement.to);
            position += replacement.from.size();
            changed = true;
            matched = true;
            break;
        }
        if (matched) continue;
        replaced.push_back(raw[position]);
        ++position;
    }
    if (!changed || replaced == raw) return candidates;
    candidates.push_back(whole_input_candidate(std::move(replaced),
                                               kSmartSymbolScore, raw.size()));
    return candidates;
}

namespace {

struct DurationParts {
    long long hours{};
    long long minutes{};
    long long seconds{};
};

std::string duration_text(const DurationParts& parts) {
    std::string text;
    if (parts.hours > 0) text += std::to_string(parts.hours) + "小时";
    if (parts.minutes > 0) text += std::to_string(parts.minutes) + "分";
    if (parts.seconds > 0) text += std::to_string(parts.seconds) + "秒";
    return text;
}

std::optional<DurationParts> split_duration(double total_seconds) {
    if (!std::isfinite(total_seconds) || total_seconds < 0.0 ||
        total_seconds > 1'000'000.0)
        return std::nullopt;
    auto remaining = total_seconds;
    DurationParts parts;
    parts.hours = static_cast<long long>(remaining / 3600.0);
    remaining -= static_cast<double>(parts.hours) * 3600.0;
    parts.minutes = static_cast<long long>(remaining / 60.0);
    remaining -= static_cast<double>(parts.minutes) * 60.0;
    // Round the remainder to whole seconds; notes do not need fractions.
    parts.seconds = static_cast<long long>(remaining + 0.5);
    if (parts.seconds == 60) {
        parts.seconds = 0;
        ++parts.minutes;
    }
    if (parts.minutes == 60) {
        parts.minutes = 0;
        ++parts.hours;
    }
    return parts;
}

std::optional<double> parse_decimal(const std::string_view text) {
    if (text.empty()) return std::nullopt;
    bool seen_digit = false;
    bool seen_point = false;
    for (const char character : text) {
        if (is_digit(character)) {
            seen_digit = true;
            continue;
        }
        if (character == '.' && !seen_point) {
            seen_point = true;
            continue;
        }
        return std::nullopt;
    }
    if (!seen_digit) return std::nullopt;
    try {
        return std::stod(std::string(text));
    } catch (...) {
        return std::nullopt;
    }
}

}  // namespace

std::vector<Candidate> generate_duration_candidates(const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes) return candidates;
    struct UnitSuffix {
        std::string_view suffix;
        double seconds_per_unit;
    };
    constexpr std::array<UnitSuffix, 3> kUnits{{
        {"h", 3600.0}, {"min", 60.0}, {"s", 1.0},
    }};
    for (const auto& unit : kUnits) {
        if (!raw.ends_with(unit.suffix)) continue;
        const auto number_text =
            raw.substr(0, raw.size() - unit.suffix.size());
        const auto value = parse_decimal(number_text);
        if (!value) return candidates;
        const auto parts = split_duration(*value * unit.seconds_per_unit);
        if (!parts) return candidates;
        const auto text = duration_text(*parts);
        if (text.empty() || text == raw) return candidates;
        candidates.push_back(whole_input_candidate(text, kSmartDurationScore,
                                                   raw.size()));
        return candidates;
    }
    return candidates;
}

namespace {

std::optional<int> chinese_digit_value(const std::uint32_t codepoint) {
    switch (codepoint) {
        case 0x96F6U:  // 零
        case 0x3007U:  // 〇
            return 0;
        case 0x4E00U:  // 一
        case 0x58F9U:  // 壹
            return 1;
        case 0x4E8CU:  // 二
        case 0x8D30U:  // 贰
        case 0x4E24U:  // 两
            return 2;
        case 0x4E09U:  // 三
        case 0x53C1U:  // 叁
            return 3;
        case 0x56DBU:  // 四
        case 0x8086U:  // 肆
            return 4;
        case 0x4E94U:  // 五
        case 0x4F0DU:  // 伍
            return 5;
        case 0x516DU:  // 六
        case 0x9646U:  // 陆
            return 6;
        case 0x4E03U:  // 七
        case 0x67D2U:  // 柒
            return 7;
        case 0x516BU:  // 八
        case 0x634CU:  // 捌
            return 8;
        case 0x4E5DU:  // 九
        case 0x7396U:  // 玖
            return 9;
        default:
            return std::nullopt;
    }
}

std::optional<long long> chinese_unit_value(const std::uint32_t codepoint) {
    switch (codepoint) {
        case 0x5341U:  // 十
        case 0x62FEU:  // 拾
            return 10;
        case 0x767EU:  // 百
        case 0x4F70U:  // 佰
            return 100;
        case 0x5343U:  // 千
        case 0x4EDFU:  // 仟
            return 1000;
        case 0x4E07U:  // 万
        case 0x842CU:  // 萬
            return 10000;
        case 0x4EBFU:  // 亿
        case 0x5104U:  // 億
            return 100000000;
        default:
            return std::nullopt;
    }
}

bool chinese_currency_suffix(const std::uint32_t codepoint) {
    return codepoint == 0x5143U ||  // 元
           codepoint == 0x5706U ||  // 圆
           codepoint == 0x5713U ||  // 圓
           codepoint == 0x6574U ||  // 整
           codepoint == 0x6B63U;    // 正
}

std::optional<std::string> parse_chinese_number(const std::string_view raw) {
    const auto units = utf8_units(raw);
    auto end = units.size();
    while (end > 0 && chinese_currency_suffix(units[end - 1].codepoint)) --end;
    if (end == 0) return std::nullopt;
    long long total = 0;
    long long section = 0;
    long long number = 0;
    bool digit_pending = false;
    bool last_was_unit = false;
    bool in_fraction = false;
    std::string fraction;
    for (std::size_t index = 0; index < end; ++index) {
        const auto codepoint = units[index].codepoint;
        if (codepoint == 0x70B9U || codepoint == 0x9EDEU) {  // 点 / 點
            if (in_fraction || index == 0 || index + 1 >= end)
                return std::nullopt;
            in_fraction = true;
            continue;
        }
        if (const auto digit = chinese_digit_value(codepoint)) {
            if (in_fraction) {
                fraction.push_back(static_cast<char>('0' + *digit));
                continue;
            }
            // Consecutive digits are not a number (一二三, 一零).
            if (digit_pending) return std::nullopt;
            number = *digit;
            digit_pending = *digit != 0;
            last_was_unit = false;
            continue;
        }
        if (in_fraction) return std::nullopt;
        const auto unit = chinese_unit_value(codepoint);
        if (!unit) return std::nullopt;
        if (*unit >= 10000) {
            if (number == 0 && section == 0 && total == 0) return std::nullopt;
            section += number;
            number = 0;
            digit_pending = false;
            if (*unit == 100000000) {
                if (total + section > 100000000LL) return std::nullopt;
                total = (total + section) * 100000000LL;
                section = 0;
            } else {
                if (section > 1000000000000LL) return std::nullopt;
                total += section * 10000LL;
                section = 0;
            }
        } else {
            if (number == 0) {
                // 十二 / 十 mean 一十二 / 一十, but two units in a row do not.
                if (last_was_unit) return std::nullopt;
                number = 1;
            }
            section += number * *unit;
            number = 0;
            digit_pending = false;
        }
        last_was_unit = true;
    }
    if (in_fraction && fraction.empty()) return std::nullopt;
    const auto value = total + section + number;
    if (value < 0 || value > 10000000000000000LL) return std::nullopt;
    std::string text = std::to_string(value);
    if (!fraction.empty()) text += "." + fraction;
    return text;
}

}  // namespace

std::vector<Candidate> generate_arabic_number_candidates(const std::string_view raw) {
    std::vector<Candidate> candidates;
    if (raw.empty() || raw.size() > kSmartMaximumInputBytes * 3) return candidates;
    const auto text = parse_chinese_number(raw);
    if (!text) return candidates;
    candidates.push_back(whole_input_candidate(*text, kSmartArabicNumberScore,
                                               raw.size()));
    return candidates;
}

std::vector<Candidate> generate_smart_candidates(const std::string_view raw,
                                                 const std::size_t limit) {
    std::vector<Candidate> merged;
    if (raw.empty() || limit == 0) return merged;
    const auto append = [&merged, limit](std::vector<Candidate>&& source) {
        for (auto& candidate : source) {
            if (merged.size() >= limit) return;
            const bool duplicate = std::any_of(
                merged.begin(), merged.end(), [&candidate](const Candidate& existing) {
                    return existing.text == candidate.text;
                });
            if (!duplicate) merged.push_back(std::move(candidate));
        }
    };
    append(generate_math_result_candidates(raw));
    append(generate_number_reading_candidates(raw));
    append(generate_arabic_number_candidates(raw));
    append(generate_date_structure_candidates(raw));
    append(generate_symbol_candidates(raw));
    append(generate_duration_candidates(raw));
    append(generate_text_lint_candidates(raw));
    append(generate_text_shape_candidates(raw));
    std::stable_sort(merged.begin(), merged.end(),
                     [](const Candidate& left, const Candidate& right) {
                         return left.score > right.score;
                     });
    if (merged.size() > std::min(limit, kSmartMaximumCandidates))
        merged.resize(std::min(limit, kSmartMaximumCandidates));
    return merged;
}

}  // namespace owo::engine

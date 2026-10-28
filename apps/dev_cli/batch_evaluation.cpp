#include "batch_evaluation.h"
#include "owo/engine/full_pinyin_schema.h"

#include <Windows.h>
#include <algorithm>
#include <charconv>
#include <cmath>
#include <fstream>
#include <iomanip>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>
#include <variant>

namespace owo::dev {
namespace {
void check(bool value, std::string message) { if (!value) throw std::invalid_argument(std::move(message)); }
std::string quote(std::string_view value) {
    std::string out = "\"";
    constexpr char hex[] = "0123456789abcdef";
    for (unsigned char c : value) {
        if (c == '"' || c == '\\') { out += '\\'; out += char(c); }
        else if (c < 32) { out += "\\u00"; out += hex[c >> 4]; out += hex[c & 15]; }
        else out += char(c);
    }
    return out + '"';
}
std::string strings(const std::vector<std::string>& values) {
    std::string out = "[";
    for (const auto& value : values) { if (out.size() > 1) out += ','; out += quote(value); }
    return out + ']';
}
std::string case_spec(const EvaluationCase& test, bool correction) {
    return "{\"id\":" + quote(test.id) + ",\"input\":" + quote(test.input) + ",\"expected\":" + strings(test.expected) + ",\"segmentation\":" + quote(test.segmentation) + ",\"context\":" + quote(test.context) + ",\"previous\":" + quote(test.previous) + ",\"correction\":" + (correction ? "true" : "false") + ",\"valid\":" + (test.valid ? "true" : "false") + ",\"tags\":" + strings(test.tags) + (test.consumed ? ",\"consumed\":" + std::to_string(*test.consumed) : "") + '}';
}
std::string decimal(double value) { std::ostringstream out; out << std::fixed << std::setprecision(6) << value; return out.str(); }
std::string boolean(bool value) { return value ? "true" : "false"; }
std::string rate(std::size_t hits, std::size_t count) { return count ? decimal(double(hits) / double(count)) : "null"; }
std::vector<std::string> split(std::string_view text, char delimiter) {
    std::vector<std::string> result;
    std::size_t begin = 0;
    for (;;) { const auto end = text.find(delimiter, begin); result.emplace_back(text.substr(begin, end - begin)); if (end == text.npos) return result; begin = end + 1; }
}
void utf8_codepoint(std::string& out, unsigned codepoint) {
    if (codepoint < 128) out += char(codepoint);
    else if (codepoint < 2048) { out += char(0xc0 | (codepoint >> 6)); out += char(0x80 | (codepoint & 63)); }
    else if (codepoint < 65536) { out += char(0xe0 | (codepoint >> 12)); out += char(0x80 | ((codepoint >> 6) & 63)); out += char(0x80 | (codepoint & 63)); }
    else { out += char(0xf0 | (codepoint >> 18)); out += char(0x80 | ((codepoint >> 12) & 63)); out += char(0x80 | ((codepoint >> 6) & 63)); out += char(0x80 | (codepoint & 63)); }
}
using Field = std::variant<std::string, std::vector<std::string>, bool, std::size_t>;
// Deliberately a fixed-schema JSON object reader: no nested objects, coercion,
// duplicate keys, unknown fields, trailing commas or executable interpolation.
class JsonRow {
    std::string_view source_; std::size_t position_{};
    void space() { while (position_ < source_.size() && (source_[position_] == ' ' || source_[position_] == '\t' || source_[position_] == '\r')) ++position_; }
    bool take(char c) { space(); if (position_ < source_.size() && source_[position_] == c) { ++position_; return true; } return false; }
    unsigned hex() {
        unsigned value = 0;
        for (int i = 0; i < 4; ++i) {
            check(position_ < source_.size(), "truncated Unicode escape");
            char c = source_[position_++]; unsigned digit = c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : 16;
            check(digit < 16, "invalid Unicode escape"); value = value * 16 + digit;
        }
        return value;
    }
    std::string string() {
        check(take('"'), "expected JSON string"); std::string value;
        while (position_ < source_.size()) {
            unsigned char c = source_[position_++];
            if (c == '"') return value;
            check(c >= 32, "control character in JSON string");
            if (c != '\\') { value += char(c); continue; }
            check(position_ < source_.size(), "truncated JSON escape"); c = source_[position_++];
            switch (c) {
            case '"': case '\\': case '/': value += char(c); break;
            case 'b': value += '\b'; break; case 'f': value += '\f'; break;
            case 'n': value += '\n'; break; case 'r': value += '\r'; break; case 't': value += '\t'; break;
            case 'u': {
                auto code = hex();
                if (code >= 0xd800 && code <= 0xdbff) { check(source_.substr(position_, 2) == "\\u", "missing low surrogate"); position_ += 2; auto low = hex(); check(low >= 0xdc00 && low <= 0xdfff, "invalid low surrogate"); code = 0x10000 + ((code - 0xd800) << 10) + low - 0xdc00; }
                else check(code < 0xdc00 || code > 0xdfff, "unpaired low surrogate");
                utf8_codepoint(value, code); break;
            }
            default: check(false, "invalid JSON escape");
            }
        }
        throw std::invalid_argument("unterminated JSON string");
    }
    Field field() {
        space(); check(position_ < source_.size(), "missing JSON value");
        if (source_[position_] == '"') return string();
        if (take('[')) {
            std::vector<std::string> values;
            if (take(']')) return values;
            do { check(values.size() < 64, "too many list values"); values.push_back(string()); if (take(']')) return values; } while (take(','));
            throw std::invalid_argument("invalid string array");
        }
        for (auto token : {std::string_view("true"), std::string_view("false")})
            if (source_.substr(position_, token.size()) == token) { position_ += token.size(); return token == "true"; }
        const auto start = position_;
        while (position_ < source_.size() && source_[position_] >= '0' && source_[position_] <= '9') ++position_;
        check(position_ > start && (position_ - start == 1 || source_[start] != '0'), "invalid JSON value");
        std::size_t value{}; const auto parsed = std::from_chars(source_.data() + start, source_.data() + position_, value);
        check(parsed.ec == std::errc{}, "integer overflow"); return value;
    }
public:
    explicit JsonRow(std::string_view source) : source_(source) {}
    std::map<std::string, Field> parse() {
        check(take('{'), "expected JSON object"); std::map<std::string, Field> values;
        if (!take('}')) for (;;) {
            auto key = string(); check(take(':'), "missing JSON colon"); auto value = field();
            check(values.emplace(std::move(key), std::move(value)).second, "duplicate JSON field");
            if (take('}')) break; check(take(','), "missing JSON comma");
        }
        space(); check(position_ == source_.size(), "trailing JSON data"); return values;
    }
};
EvaluationCase row(const std::map<std::string, Field>& fields) {
    EvaluationCase result;
    for (const auto& [key, value] : fields) {
        if (key == "id" || key == "input" || key == "segmentation" || key == "context" || key == "previous") {
            check(std::holds_alternative<std::string>(value), key + " must be a string"); auto text = std::get<std::string>(value);
            if (key == "id") result.id = text; else if (key == "input") result.input = text;
            else if (key == "segmentation") result.segmentation = text; else if (key == "context") result.context = text; else result.previous = text;
        } else if (key == "expected" || key == "tags") {
            check(std::holds_alternative<std::vector<std::string>>(value), key + " must be a string array");
            (key == "expected" ? result.expected : result.tags) = std::get<std::vector<std::string>>(value);
        } else if (key == "valid" || key == "correction") {
            check(std::holds_alternative<bool>(value), key + " must be boolean");
            if (key == "valid") result.valid = std::get<bool>(value); else result.correction = std::get<bool>(value);
        } else if (key == "consumed") {
            check(std::holds_alternative<std::size_t>(value), "consumed must be an integer"); result.consumed = std::get<std::size_t>(value);
        } else throw std::invalid_argument("unknown dataset field: " + key);
    }
    return result;
}
std::string path_segmentation(const engine::ParsePath& path) {
    std::string value; for (const auto& part : path.syllables) { if (!value.empty()) value += '\''; value += part.text; } return value;
}
std::string preview(const protocol::Message& response) {
    std::string result; for (const auto& part : response.syllables) { if (!result.empty()) result += '\''; result += part; } return result;
}
struct Item { std::string text; std::size_t consumed{}, page{}, index{}; std::uint64_t group{3}; bool operator==(const Item&) const = default; };
std::vector<Item> items(const protocol::Message& response) {
    check(response.candidates.size() == response.candidate_consumed.size(), "candidate consumption mapping missing");
    check(response.candidate_groups.empty() || response.candidates.size() == response.candidate_groups.size(), "candidate group mapping missing");
    std::vector<Item> result;
    for (std::size_t i = 0; i < response.candidates.size(); ++i) result.push_back({response.candidates[i], std::size_t(response.candidate_consumed[i]), std::size_t(response.page), i + 1, response.candidate_groups.empty() ? 3 : response.candidate_groups[i]});
    return result;
}
std::string item_json(const std::vector<Item>& values) {
    std::string result = "[";
    for (const auto& value : values) { if (result.size() > 1) result += ','; result += "{\"text\":" + quote(value.text) + ",\"consumed\":" + std::to_string(value.consumed) + ",\"page\":" + std::to_string(value.page) + ",\"index\":" + std::to_string(value.index) + ",\"group\":" + std::to_string(value.group) + '}'; }
    return result + ']';
}
std::size_t rank(const std::vector<Item>& values, const EvaluationCase& test) {
    auto consumed = test.consumed.value_or(test.input.size());
    for (std::size_t i = 0; i < values.size(); ++i)
        if (values[i].consumed == consumed && std::find(test.expected.begin(), test.expected.end(), values[i].text) != test.expected.end()) return i + 1;
    return 0;
}
struct Counts {
    std::size_t cases{}, failures{}, errors{}, labeled{}, base_top1{}, base_first{}, topk{}, final_top1{}, final_first{}, valid{}, segments{}, preview_hits{}, path_hits{}, model_requested{}, model_applied{}, model_failed{}, expansion_checked{}, expansion_failed{};
    double reciprocal{};
    void candidate(std::size_t base_rank, std::size_t final_rank, std::size_t first_count, std::size_t final_count, unsigned k) {
        ++labeled; base_top1 += base_rank == 1; base_first += base_rank != 0 && base_rank <= first_count;
        topk += base_rank != 0 && base_rank <= k; if (base_rank && base_rank <= k) reciprocal += 1.0 / double(base_rank);
        final_top1 += final_rank == 1; final_first += final_rank != 0 && final_rank <= final_count;
    }
    std::string json() const {
        return "{\"cases\":" + std::to_string(cases) + ",\"failures\":" + std::to_string(failures) + ",\"errors\":" + std::to_string(errors) + ",\"labeled\":" + std::to_string(labeled) +
            ",\"base_top1\":" + rate(base_top1, labeled) + ",\"base_first_page\":" + rate(base_first, labeled) + ",\"base_topk\":" + rate(topk, labeled) + ",\"base_mrr_at_k\":" + (labeled ? decimal(reciprocal / double(labeled)) : "null") +
            ",\"final_top1\":" + rate(final_top1, labeled) + ",\"final_first_page\":" + rate(final_first, labeled) +
            ",\"parse_validity_accuracy\":" + rate(valid, cases) + ",\"segmentation_labeled\":" + std::to_string(segments) + ",\"preview_accuracy\":" + rate(preview_hits, segments) + ",\"parse_path_coverage\":" + rate(path_hits, segments) +
            ",\"model_requested\":" + std::to_string(model_requested) + ",\"model_applied\":" + std::to_string(model_applied) + ",\"model_failed\":" + std::to_string(model_failed) + ",\"expansion_checked\":" + std::to_string(expansion_checked) + ",\"expansion_failed\":" + std::to_string(expansion_failed) + '}';
    }
};
double percentile(std::vector<double> values, double fraction) { if (values.empty()) return 0; std::sort(values.begin(), values.end()); return values[std::size_t(std::ceil(fraction * double(values.size()))) - 1]; }
std::string timings(const std::vector<double>& values) {
    if (values.empty()) return "null";
    return "{\"samples\":" + std::to_string(values.size()) + ",\"p50\":" + decimal(percentile(values, .5)) + ",\"p95\":" + decimal(percentile(values, .95)) + ",\"max\":" + decimal(*std::max_element(values.begin(), values.end())) + '}';
}
void write_file(const std::filesystem::path& path, std::string_view value) {
    std::ofstream file(path, std::ios::binary); file.write(value.data(), std::streamsize(value.size())); file.flush(); check(bool(file), "cannot write evaluation report");
}
} // namespace

std::vector<EvaluationCase> read_evaluation_cases(const std::filesystem::path& path, std::string format) {
    check(std::filesystem::is_regular_file(path) && std::filesystem::file_size(path) <= 16 * 1024 * 1024, "dataset is missing or exceeds 16 MiB");
    std::ifstream file(path, std::ios::binary); check(bool(file), "cannot open dataset");
    std::string bytes; char buffer[8192];
    while (file.read(buffer, sizeof(buffer)) || file.gcount()) { bytes.append(buffer, std::size_t(file.gcount())); check(bytes.size() <= 16 * 1024 * 1024, "dataset grew beyond limit"); }
    check(file.eof(), "dataset read failed");
    if (bytes.starts_with("\xef\xbb\xbf")) bytes.erase(0, 3);
    check(!bytes.empty() && MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(), int(bytes.size()), nullptr, 0) > 0, "dataset must be UTF-8");
    if (format == "auto") format = path.extension() == L".tsv" ? "tsv" : "jsonl";
    check(format == "jsonl" || format == "tsv", "dataset format must be auto/jsonl/tsv");
    std::istringstream input(bytes); std::string line; std::vector<EvaluationCase> result; std::set<std::string> ids; std::vector<std::string> columns; std::size_t number = 0;
    while (std::getline(input, line)) {
        ++number; if (line.ends_with('\r')) line.pop_back(); if (line.empty() || (format == "tsv" && line.starts_with('#'))) continue;
        try {
            check(line.size() <= 65536, "dataset line exceeds 64 KiB"); EvaluationCase test;
            if (format == "jsonl") test = row(JsonRow(line).parse());
            else {
                auto cells = split(line, '\t');
                if (columns.empty()) {
                    columns = cells; std::map<std::string, Field> header;
                    for (auto& column : columns) { check(!column.empty() && !header.contains(column), "empty/duplicate TSV column"); header[column] = (column == "expected" || column == "tags") ? Field(std::vector<std::string>{}) : column == "valid" || column == "correction" ? Field(true) : column == "consumed" ? Field(std::size_t{0}) : Field(std::string{}); }
                    static_cast<void>(row(header)); check(header.contains("input"), "TSV requires input column"); continue;
                }
                check(cells.size() == columns.size(), "TSV column count mismatch"); std::map<std::string, Field> fields;
                for (std::size_t i = 0; i < cells.size(); ++i) {
                    if (cells[i].empty() && columns[i] != "input") continue;
                    if (columns[i] == "expected" || columns[i] == "tags") {
                        if (cells[i].starts_with('[')) fields[columns[i]] = JsonRow("{" + quote(columns[i]) + ":" + cells[i] + "}").parse().at(columns[i]);
                        else fields[columns[i]] = split(cells[i], '|');
                    }
                    else if (columns[i] == "valid" || columns[i] == "correction") { check(cells[i] == "true" || cells[i] == "false", "TSV booleans must be true/false"); fields[columns[i]] = cells[i] == "true"; }
                    else if (columns[i] == "consumed") { std::size_t value{}; auto parsed = std::from_chars(cells[i].data(), cells[i].data() + cells[i].size(), value); check(parsed.ec == std::errc{} && parsed.ptr == cells[i].data() + cells[i].size(), "invalid consumed range"); fields[columns[i]] = value; }
                    else fields[columns[i]] = cells[i];
                }
                test = row(fields);
            }
            if (test.id.empty()) test.id = "line-" + std::to_string(number);
            auto clean = [](std::string_view value) { return std::none_of(value.begin(), value.end(), [](unsigned char c) { return c < 32 || c == 127; }); };
            check(test.id.size() <= 128 && clean(test.id) && ids.insert(test.id).second, "invalid/duplicate case id");
            check(!test.input.empty() && test.input.size() <= 256 && std::all_of(test.input.begin(), test.input.end(), [](unsigned char c) { return c >= 32 && c < 127; }), "input must be 1..256 printable ASCII bytes");
            check(test.context.size() <= 256 && test.previous.size() <= 256 && clean(test.context) && clean(test.previous), "context/previous exceeds 256 bytes or contains controls");
            check(test.segmentation.size() <= 512 && clean(test.segmentation), "invalid segmentation");
            check(!test.consumed || (*test.consumed > 0 && *test.consumed <= test.input.size()), "consumed exceeds input range");
            check(test.valid || (test.expected.empty() && test.segmentation.empty()), "invalid-input cases cannot expect candidates/segmentation");
            check(test.expected.size() <= 64 && test.tags.size() <= 64, "too many expected words/tags");
            for (const auto& word : test.expected) check(!word.empty() && word.size() <= 4096 && clean(word), "invalid expected word");
            for (const auto& tag : test.tags) check(!tag.empty() && tag.size() <= 64 && clean(tag), "invalid tag");
            check(result.size() < 10000, "dataset exceeds 10000 cases"); result.push_back(std::move(test));
        } catch (const std::exception& error) { throw std::invalid_argument("dataset line " + std::to_string(number) + ": " + error.what()); }
    }
    check(!result.empty(), "dataset contains no cases"); return result;
}

int evaluate_cases(const std::vector<EvaluationCase>& cases, const EvaluationOptions& options,
                   const EvaluationQuery& query, const std::filesystem::path& report_directory,
                   std::string_view manifest_fields, std::ostream& output) {
    auto report = report_directory / L"cases.jsonl.partial";
    std::ofstream records(report, std::ios::binary); check(bool(records), "cannot create case report");
    Counts total; std::map<std::string, Counts> tags; std::vector<double> latency, settled_latency, parse_latency, typing_latency;
    const auto run_started = std::chrono::steady_clock::now();
    std::string failed_cases, dataset;
    for (const auto& test : cases) dataset += case_spec(test, test.correction.value_or(options.correction)) + '\n';
    write_file(report_directory / L"dataset.jsonl", dataset);
    std::string failures = "id\tinput\texpected\tsegmentation\tcontext\tprevious\tcorrection\tconsumed\tvalid\ttags\n";
    auto emit = [&](const std::string& text) { output << text << '\n'; records << text << '\n'; records.flush(); check(bool(records) && bool(output), "report output failed"); };
    auto threshold = [](std::optional<double> value) { return value ? decimal(*value) : "null"; };
    emit("{\"schema_version\":1,\"record\":\"manifest\",\"command\":\"input.batch\",\"model\":" + quote(options.model) + ",\"mode\":" + quote(options.mode) + ",\"typing\":" + quote(options.typing) + ",\"top_k\":" + std::to_string(options.top_k) + ",\"max_pages\":" + std::to_string(options.max_pages) + ",\"repeat\":" + std::to_string(options.repeat) + ",\"check_expanded\":" + boolean(options.check_expanded) + ",\"thresholds\":{\"min_top1\":" + threshold(options.min_top1) + ",\"min_first_page\":" + threshold(options.min_first_page) + ",\"min_base_topk\":" + threshold(options.min_topk) + ",\"max_base_p95_ms\":" + threshold(options.max_p95_ms) + "}," + std::string(manifest_fields) + '}');
    for (const auto& test : cases) {
        bool pass = true, runtime_error = false, parse_ok = false, preview_ok = false, path_ok = false, model_requested = false, model_applied = false, model_failed = false, expansion_checked = false, expansion_ok = true, truncated = false;
        std::string error, actual_preview; std::vector<std::string> paths; std::vector<Item> base_items, final_items;
        std::size_t base_rank = 0, final_rank = 0, first_count = 0, final_count = 0; double first_ms = 0, settled_ms = 0, parse_ms = 0;
        std::vector<double> repeats;
        const bool correction = test.correction.value_or(options.correction);
        try {
            const auto started = std::chrono::steady_clock::now();
            const auto parsed = engine::FullPinyinSchema{}.parse(test.input, options.paths, correction);
            parse_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - started).count();
            parse_latency.push_back(parse_ms); parse_ok = parsed.valid == test.valid; pass &= parse_ok;
            for (const auto& path : parsed.paths) paths.push_back(path_segmentation(path));
            if (!paths.empty()) actual_preview = paths.front();
            path_ok = test.segmentation.empty() || std::find(paths.begin(), paths.end(), test.segmentation) != paths.end();
            if (options.mode != "parse" && parsed.valid) {
                protocol::Message request; request.type = protocol::MessageType::candidate_request; request.text = test.input; request.context = test.context; request.input = test.previous; request.correction_enabled = correction;
                if (options.typing == "incremental") for (std::size_t length = 1; length < test.input.size(); ++length) {
                    auto prefix = request; prefix.text.resize(length); auto answer = query(prefix); typing_latency.push_back(answer.base_ms);
                }
                const auto first = query(request); first_ms = first.base_ms; settled_ms = first.settled_ms;
                base_items = items(first.base); final_items = items(first.final); first_count = base_items.size(); final_count = final_items.size();
                actual_preview = preview(first.base); model_requested = first.model_requested; model_applied = first.model_applied; model_failed = first.model_failed;
                check(!first.base.candidate_refinement_pending, "candidate refinement did not settle");
                auto has_more = first.base.has_more;
                for (unsigned page = 1; has_more && page < options.max_pages; ++page) {
                    request.page = page; auto next = query(request); auto page_items = items(next.base);
                    base_items.insert(base_items.end(), page_items.begin(), page_items.end()); has_more = next.base.has_more;
                }
                truncated = has_more;
                if (options.check_expanded) {
                    request.page = 0; request.expanded = true; auto expanded = items(query(request).base); expansion_checked = true;
                    const auto count = std::min(expanded.size(), base_items.size());
                    for (std::size_t i = 0; i < count; ++i) expansion_ok &= expanded[i].text == base_items[i].text && expanded[i].consumed == base_items[i].consumed && expanded[i].group == base_items[i].group;
                    const auto scanned = std::min(base_items.size(), protocol::kMaximumCandidateCount);
                    if (expanded.size() < scanned || (!has_more && expanded.size() != scanned)) expansion_ok = false;
                    pass &= expansion_ok;
                }
                request.page = 0; request.expanded = false;
                for (unsigned sample = 1; sample < options.repeat; ++sample) repeats.push_back(query(request).base_ms);
                base_rank = rank(base_items, test); final_rank = rank(final_items, test);
                latency.push_back(first_ms); settled_latency.push_back(settled_ms);
                pass &= !model_failed;
                if (!test.expected.empty()) pass &= options.model == "base" ? base_rank != 0 && base_rank <= options.top_k : final_rank != 0;
            } else if (options.mode != "parse" && !test.expected.empty()) pass = false;
            preview_ok = test.segmentation.empty() || actual_preview == test.segmentation;
            pass &= preview_ok && path_ok;
        } catch (const std::exception& failure) { error = failure.what(); runtime_error = true; pass = false; }
        auto accumulate = [&](Counts& count) {
            ++count.cases; count.failures += !pass; count.errors += runtime_error; count.valid += parse_ok;
            if (!test.segmentation.empty()) { ++count.segments; count.preview_hits += preview_ok; count.path_hits += path_ok; }
            if (options.mode != "parse" && !test.expected.empty()) count.candidate(base_rank, final_rank, first_count, final_count, options.top_k);
            count.model_requested += model_requested; count.model_applied += model_applied; count.model_failed += model_failed;
            count.expansion_checked += expansion_checked; count.expansion_failed += expansion_checked && !expansion_ok;
        };
        accumulate(total); for (const auto& tag : std::set<std::string>(test.tags.begin(), test.tags.end())) accumulate(tags[tag]);
        if (!pass) {
            failed_cases += case_spec(test, correction) + '\n';
            failures += test.id + '\t' + test.input + '\t' + strings(test.expected) + '\t' + test.segmentation + '\t' + test.context + '\t' + test.previous + '\t' + boolean(correction) + '\t' + (test.consumed ? std::to_string(*test.consumed) : "") + '\t' + boolean(test.valid) + '\t' + strings(test.tags) + '\n';
        }
        emit("{\"schema_version\":1,\"record\":\"case\",\"id\":" + quote(test.id) + ",\"input\":" + quote(test.input) + ",\"expected\":" + strings(test.expected) + ",\"tags\":" + strings(test.tags) + ",\"test\":" + case_spec(test, correction) + ",\"pass\":" + boolean(pass) + ",\"runtime_error\":" + boolean(runtime_error) + ",\"error\":" + quote(error) +
            ",\"parse_validity_pass\":" + boolean(parse_ok) + ",\"paths\":" + strings(paths) + ",\"preview\":" + quote(actual_preview) + ",\"expected_segmentation\":" + quote(test.segmentation) + ",\"base_rank\":" + (base_rank ? std::to_string(base_rank) : "null") + ",\"final_first_page_rank\":" + (final_rank ? std::to_string(final_rank) : "null") + ",\"base_scan_truncated\":" + boolean(truncated) + ",\"base_candidates\":" + item_json(base_items) + ",\"final_first_page\":" + item_json(final_items) + ",\"model_requested\":" + boolean(model_requested) + ",\"model_applied\":" + boolean(model_applied) + ",\"model_failed\":" + boolean(model_failed) + ",\"expansion_checked\":" + boolean(expansion_checked) + ",\"expansion_pass\":" + boolean(expansion_ok) + ",\"timing_ms\":{\"parse\":" + decimal(parse_ms) + ",\"base_request\":" + decimal(first_ms) + ",\"settled_request\":" + decimal(settled_ms) + ",\"repeat_base\":" + timings(repeats) + "}}");
    }
    std::vector<std::string> gates;
    if (!options.allow_misses && total.failures) gates.push_back("case_assertions");
    auto gate = [&](std::optional<double> minimum, std::size_t hits, std::string name) { if (minimum && (!total.labeled || double(hits) / double(total.labeled) < *minimum)) gates.push_back(name); };
    gate(options.min_top1, options.model == "base" ? total.base_top1 : total.final_top1, "min_top1");
    gate(options.min_first_page, options.model == "base" ? total.base_first : total.final_first, "min_first_page");
    gate(options.min_topk, total.topk, "min_base_topk");
    if (options.max_p95_ms && (latency.empty() || percentile(latency, .95) > *options.max_p95_ms)) gates.push_back("max_base_p95_ms");
    if (total.model_failed) gates.push_back("model_failed");
    const int exit_code = total.errors ? 4 : gates.empty() ? 0 : 1;
    std::string breakdown = "{"; for (const auto& [tag, counts] : tags) { if (breakdown.size() > 1) breakdown += ','; breakdown += quote(tag) + ':' + counts.json(); } breakdown += '}';
    const auto summary = "{\"schema_version\":1,\"record\":\"summary\",\"command\":\"input.batch\",\"complete\":true,\"ok\":" + boolean(exit_code == 0) + ",\"exit_code\":" + std::to_string(exit_code) + ",\"allow_misses\":" + boolean(options.allow_misses) + ",\"failed_gates\":" + strings(gates) + ",\"metrics\":" + total.json() + ",\"by_tag\":" + breakdown + ",\"duration_ms\":" + decimal(std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - run_started).count()) + ",\"timing_ms\":{\"base_request\":" + timings(latency) + ",\"settled_request\":" + timings(settled_latency) + ",\"parse\":" + timings(parse_latency) + ",\"incremental_prefix_request\":" + timings(typing_latency) + "}}";
    emit(summary); records.close(); check(bool(records), "cannot close evaluation report");
    write_file(report_directory / L"failures.tsv", failures); write_file(report_directory / L"failures.jsonl", failed_cases);
    write_file(report_directory / L"summary.md", "# OwO CLI candidate evaluation\n\nCases: " + std::to_string(total.cases) + "; failures: " + std::to_string(total.failures) + "; runtime errors: " + std::to_string(total.errors) + ".\n\nBase top-1: " + rate(total.base_top1, total.labeled) + "; base first-page: " + rate(total.base_first, total.labeled) + "; base Top-K: " + rate(total.topk, total.labeled) + "; final first-page top-1: " + rate(total.final_top1, total.labeled) + ".\n\nRates are 0..1; null means no eligible labels. See summary.json and cases.jsonl for limits, input consumption and model status. No GUI/TSF latency is measured.\n");
    std::filesystem::rename(report, report_directory / L"cases.jsonl");
    write_file(report_directory / L"summary.json", summary + '\n');
    return exit_code;
}
} // namespace owo::dev

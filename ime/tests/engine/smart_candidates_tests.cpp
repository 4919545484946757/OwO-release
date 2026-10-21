#include "owo/engine/smart_candidates.h"

#include <algorithm>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

int fail(const std::string_view message) {
    std::cerr << message << '\n';
    return 1;
}

bool contains(const std::vector<owo::engine::Candidate>& candidates,
              const std::string_view text) {
    return std::any_of(candidates.begin(), candidates.end(),
                       [text](const owo::engine::Candidate& candidate) {
                           return candidate.text == text;
                       });
}

}  // namespace

int main() {
    {
        const auto candidates =
            owo::engine::generate_math_result_candidates("1+2*3");
        if (candidates.size() != 1 || candidates.front().text != "7" ||
            candidates.front().consumed_input_bytes != 5)
            return fail("basic arithmetic result candidate failed");
    }
    {
        const auto candidates =
            owo::engine::generate_math_result_candidates("(1+2)*3=");
        if (candidates.size() != 1 || candidates.front().text != "(1+2)*3=9")
            return fail("equality-suffixed arithmetic candidate failed");
    }
    {
        const auto candidates =
            owo::engine::generate_math_result_candidates("=1+2");
        if (candidates.size() != 1 || candidates.front().text != "=1+2=3")
            return fail("spreadsheet-style formula candidate failed");
    }
    if (!owo::engine::generate_math_result_candidates("=").empty())
        return fail("bare equals sign produced a candidate");
    {
        const auto candidates = owo::engine::generate_math_result_candidates(
            "2^10");
        if (candidates.size() != 1 || candidates.front().text != "1024")
            return fail("power arithmetic candidate failed");
    }
    if (!owo::engine::generate_math_result_candidates("1/0").empty())
        return fail("division by zero produced a candidate");
    if (!owo::engine::generate_math_result_candidates("123").empty())
        return fail("plain digits produced a structure candidate");
    if (!owo::engine::generate_math_result_candidates("2026-10-19").empty())
        return fail("date-shaped input was evaluated as subtraction");
    if (!owo::engine::generate_math_result_candidates("1+2;3").empty())
        return fail("non-arithmetic input produced a candidate");

    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("2026-02-30");
        if (candidates.size() != 1 || candidates.front().text != "2026-02-28")
            return fail("impossible day was not clamped");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("2026-13-05");
        if (candidates.size() != 1 || candidates.front().text != "2027-01-05")
            return fail("out-of-range month was not carried into the year");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("2026/13/05");
        if (candidates.size() != 1 || candidates.front().text != "2027/01/05")
            return fail("slash date family was not preserved");
    }
    if (!owo::engine::generate_text_lint_candidates("2026-02-28").empty())
        return fail("valid date produced a correction");
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("你好。。");
        if (candidates.size() != 1 || candidates.front().text != "你好。")
            return fail("repeated punctuation was not collapsed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("(abc");
        if (candidates.size() != 1 || candidates.front().text != "(abc)")
            return fail("unbalanced parenthesis was not closed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("\"abc");
        if (candidates.size() != 1 || candidates.front().text != "\"abc\"")
            return fail("unbalanced quote was not closed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("2026-03-05到2026-01-01");
        if (!contains(candidates, "2026-01-01到2026-03-05"))
            return fail("reversed date range was not reordered");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("2026-03-05~2026-01-01");
        if (!contains(candidates, "2026-01-01~2026-03-05"))
            return fail("tilde date range was not reordered");
    }
    {
        const auto candidates = owo::engine::generate_text_lint_candidates(
            "2026-01-01到2026-03-05");
        for (const auto& candidate : candidates)
            if (candidate.text.starts_with("2026-03-05"))
                return fail("ordered date range was reordered");
    }
    if (!owo::engine::generate_text_lint_candidates(
             "2026-03-05 会议 2026-01-01").empty())
        return fail("unrelated dates were treated as a range");
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("你好world");
        if (!contains(candidates, "你好 world"))
            return fail("missing CJK/Latin space was not inserted");
    }
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("测试3个");
        if (!contains(candidates, "测试 3 个"))
            return fail("missing CJK/digit spacing was not inserted");
    }
    if (contains(owo::engine::generate_text_lint_candidates("2026年1月5日"),
                 "2026 年 1 月 5 日"))
        return fail("date units were treated as CJK/Latin spacing boundaries");
    if (contains(owo::engine::generate_text_lint_candidates("9时30分"),
                 "9 时 30 分"))
        return fail("clock units were treated as CJK/Latin spacing boundaries");
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("hi你好");
        if (!contains(candidates, "hi 你好"))
            return fail("Latin/CJK spacing was not inserted");
    }
    if (!owo::engine::generate_text_lint_candidates("你好，world").empty())
        return fail("CJK punctuation triggered spacing");
    if (!owo::engine::generate_text_lint_candidates("hello world").empty())
        return fail("plain ASCII text produced a lint candidate");
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("你好,世界");
        if (!contains(candidates, "你好，世界"))
            return fail("ASCII comma between Chinese text was not normalized");
    }
    if (!contains(owo::engine::generate_text_lint_candidates("真的?"), "真的？"))
        return fail("ASCII question mark was not normalized");
    if (!contains(owo::engine::generate_text_lint_candidates("注意!")
                  , "注意！"))
        return fail("ASCII exclamation mark was not normalized");
    if (!contains(owo::engine::generate_text_lint_candidates("说明:测试"),
                  "说明：测试"))
        return fail("ASCII colon was not normalized");
    if (!contains(owo::engine::generate_text_lint_candidates("(你好)"),
                  "（你好）"))
        return fail("ASCII brackets around Chinese text were not normalized");
    if (!contains(owo::engine::generate_text_lint_candidates("\"你好\""),
                  "“你好”"))
        return fail("ASCII quotes around Chinese text were not normalized");
    if (!owo::engine::generate_text_lint_candidates("hello, world").empty())
        return fail("ASCII-only punctuation was normalized");
    if (!owo::engine::generate_text_lint_candidates("1,000").empty())
        return fail("digit grouping comma was normalized");
    if (!owo::engine::generate_text_lint_candidates("3.14").empty())
        return fail("decimal point was normalized");
    if (!owo::engine::generate_text_lint_candidates("www.qq.com").empty())
        return fail("URL punctuation was normalized");
    {
        const auto candidates =
            owo::engine::generate_text_lint_candidates("ＡＢＣ１２３");
        if (!contains(candidates, "ABC123"))
            return fail("full-width ASCII was not collapsed to half-width");
    }
    if (!contains(owo::engine::generate_text_lint_candidates("１＋２＝３"),
                  "1+2=3"))
        return fail("full-width expression was not normalized");
    if (!contains(owo::engine::generate_text_lint_candidates("你好\u3000世界"),
                  "你好 世界"))
        return fail("ideographic space was not normalized");
    if (!owo::engine::generate_text_lint_candidates("ABC123").empty())
        return fail("half-width ASCII produced a width candidate");

    if (!owo::engine::generate_text_shape_candidates("hello").empty())
        return fail("plain lowercase word produced a shape candidate");    {
        const auto candidates =
            owo::engine::generate_text_shape_candidates("helloWorld");
        if (!contains(candidates, "hello World"))
            return fail("camelCase split candidate failed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_shape_candidates("hello_world");
        if (!contains(candidates, "hello world"))
            return fail("separator split candidate failed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_shape_candidates("hello world");
        if (!contains(candidates, "helloworld") ||
            !contains(candidates, "hello_world"))
            return fail("word join candidates failed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_shape_candidates("a,b,c");
        if (!contains(candidates, "c,b,a"))
            return fail("list reorder candidate failed");
    }
    {
        const auto candidates =
            owo::engine::generate_text_shape_candidates("mingtian__");
        if (!contains(candidates, "mingtian【待补】"))
            return fail("gap placeholder candidate failed");
    }

    {
        const auto candidates =
            owo::engine::generate_number_reading_candidates("1234");
        if (!contains(candidates, "一千二百三十四") ||
            !contains(candidates, "壹仟贰佰叁拾肆"))
            return fail("integer number readings failed");
    }
    {
        const auto candidates =
            owo::engine::generate_number_reading_candidates("10");
        if (!contains(candidates, "十") || !contains(candidates, "壹拾"))
            return fail("ten shortcut reading failed");
    }
    if (!contains(owo::engine::generate_number_reading_candidates("10001"),
                  "一万零一"))
        return fail("ten-thousand grouping reading failed");
    if (!contains(owo::engine::generate_number_reading_candidates("12.5"),
                  "十二点五"))
        return fail("decimal reading failed");
    if (!contains(owo::engine::generate_number_reading_candidates("12%"),
                  "百分之十二"))
        return fail("percent reading failed");
    if (!owo::engine::generate_number_reading_candidates("5").empty())
        return fail("single digit produced a reading");
    if (!owo::engine::generate_number_reading_candidates("1+2").empty())
        return fail("arithmetic expression produced a number reading");
    if (!owo::engine::generate_number_reading_candidates("12345678901234567")
             .empty())
        return fail("oversized number produced a reading");

    if (!contains(owo::engine::generate_arabic_number_candidates("十二"), "12"))
        return fail("implicit ten was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("二十"), "20"))
        return fail("twenty was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("一百零五"),
                  "105"))
        return fail("zero placeholder was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("一千零十二"),
                  "1012"))
        return fail("zero plus implicit ten was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("一万零一"),
                  "10001"))
        return fail("ten-thousand grouping was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("一亿二千万"),
                  "120000000"))
        return fail("hundred-million grouping was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("十二点五"),
                  "12.5"))
        return fail("Chinese decimal was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("壹仟贰佰叁拾肆元整"),
                  "1234"))
        return fail("formal amount with currency suffix was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("两百"), "200"))
        return fail("colloquial two hundred was not parsed");
    if (!contains(owo::engine::generate_arabic_number_candidates("十"), "10"))
        return fail("bare ten was not parsed");
    if (!owo::engine::generate_arabic_number_candidates("一二三").empty())
        return fail("consecutive Chinese digits produced a number");
    if (!owo::engine::generate_arabic_number_candidates("十十").empty())
        return fail("repeated unit produced a number");
    if (!owo::engine::generate_arabic_number_candidates("十点").empty())
        return fail("empty Chinese fraction produced a number");
    if (!owo::engine::generate_arabic_number_candidates("abc").empty())
        return fail("plain text produced a Chinese numeral candidate");
    if (!owo::engine::generate_arabic_number_candidates("1234").empty())
        return fail("ASCII digits produced a Chinese numeral candidate");

    {
        const auto candidates =
            owo::engine::generate_date_structure_candidates("2026-10-19");
        if (!contains(candidates, "2026年10月19日") ||
            !contains(candidates, "2026年10月19日 星期一"))
            return fail("localized date/weekday candidate failed");
    }
    if (!contains(owo::engine::generate_date_structure_candidates("2026年1月5日"),
                  "2026-01-05"))
        return fail("Chinese date did not offer the ISO form");
    if (!contains(owo::engine::generate_date_structure_candidates("2026年1月5日"),
                  "2026年1月5日 星期一"))
        return fail("Chinese date did not offer the weekday");
    if (!contains(owo::engine::generate_date_structure_candidates("9:5"),
                  "09:05"))
        return fail("loose clock time was not zero-padded");
    if (!owo::engine::generate_date_structure_candidates("25:00").empty())
        return fail("invalid clock time produced a candidate");
    if (!owo::engine::generate_date_structure_candidates("2026-02-30").empty())
        return fail("invalid date produced a structure candidate");
    if (!owo::engine::generate_date_structure_candidates("2026-10-19 10:00")
             .empty())
        return fail("date plus time produced a single-date structure candidate");
    {
        const auto candidates = owo::engine::generate_date_structure_candidates(
            "2026-10-19~2026-10-25");
        if (!contains(candidates, "2026年10月19日 至 2026年10月25日"))
            return fail("date range was not localized");
    }
    {
        const auto candidates = owo::engine::generate_date_structure_candidates(
            "2026-10-19到2026-10-25");
        if (!contains(candidates, "2026年10月19日 至 2026年10月25日"))
            return fail("Chinese-connector date range was not localized");
    }
    {
        const auto candidates = owo::engine::generate_date_structure_candidates(
            "2026-10-19~2026-10-25 会议");
        if (contains(candidates, "2026年10月19日 至 2026年10月25日"))
            return fail("partial date range was localized");
    }
    if (!owo::engine::generate_date_structure_candidates(
             "2026-02-30~2026-10-25")
             .empty())
        return fail("invalid date range was localized");
    {
        const auto candidates =
            owo::engine::generate_date_structure_candidates("20261019");
        if (!contains(candidates, "2026-10-19") ||
            !contains(candidates, "2026年10月19日 星期一"))
            return fail("compact date was not expanded");
    }
    if (!owo::engine::generate_date_structure_candidates("20261319").empty())
        return fail("invalid compact date produced a candidate");
    if (!owo::engine::generate_date_structure_candidates("2026101").empty())
        return fail("seven-digit input produced a compact date");
    if (!owo::engine::generate_number_reading_candidates("20261019").empty())
        return fail("compact date produced a number reading");

    if (!contains(owo::engine::generate_symbol_candidates("x!=y"), "x≠y"))
        return fail("not-equal symbol was not converted");
    if (!contains(owo::engine::generate_symbol_candidates("a<=b"), "a≤b"))
        return fail("less-or-equal symbol was not converted");
    if (!contains(owo::engine::generate_symbol_candidates("a->b"), "a→b"))
        return fail("arrow symbol was not converted");
    if (!contains(owo::engine::generate_symbol_candidates("a-->b"), "a→b"))
        return fail("long arrow was not matched before the short one");
    if (!contains(owo::engine::generate_symbol_candidates("a...b"), "a…b"))
        return fail("ellipsis was not converted");
    if (!owo::engine::generate_symbol_candidates("--help").empty())
        return fail("command-line flag was converted");
    if (!owo::engine::generate_symbol_candidates("hello").empty())
        return fail("plain text produced a symbol candidate");

    if (!contains(owo::engine::generate_duration_candidates("90min"),
                  "1小时30分"))
        return fail("minute duration over an hour was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("2h"), "2小时"))
        return fail("whole-hour duration was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("1.5h"),
                  "1小时30分"))
        return fail("fractional-hour duration was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("45s"), "45秒"))
        return fail("second duration was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("75s"),
                  "1分15秒"))
        return fail("second duration over a minute was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("3661s"),
                  "1小时1分1秒"))
        return fail("second duration over an hour was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("120min"),
                  "2小时"))
        return fail("exact-hour minute duration was not normalized");
    if (!contains(owo::engine::generate_duration_candidates("0.5h"), "30分"))
        return fail("half-hour duration was not normalized");
    if (!owo::engine::generate_duration_candidates("2hours").empty())
        return fail("long unit name produced a duration candidate");
    if (!owo::engine::generate_duration_candidates("2H").empty())
        return fail("uppercase unit produced a duration candidate");
    if (!owo::engine::generate_duration_candidates("1.2.3h").empty())
        return fail("malformed duration produced a candidate");
    if (!owo::engine::generate_duration_candidates("2000000s").empty())
        return fail("oversized duration produced a candidate");

    {
        const auto candidates =
            owo::engine::generate_smart_candidates("2026-02-30__", 4);
        if (candidates.size() != 2 ||
            !contains(candidates, "2026-02-28__") ||
            !contains(candidates, "2026-02-30【待补】"))
            return fail("merged smart candidates lost a rule family");
        for (const auto& candidate : candidates)
            if (candidate.consumed_input_bytes != 12)
                return fail("smart candidate consumption is wrong");
    }
    {
        const auto candidates = owo::engine::generate_smart_candidates("hello", 0);
        if (!candidates.empty())
            return fail("zero limit still produced candidates");
    }

    std::cout << "smart candidate tests passed\n";
    return 0;
}

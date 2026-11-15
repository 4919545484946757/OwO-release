#include "owo/tsf/language_context.h"

#include <string>

int main() {
    std::wstring context;
    owo::tsf::append_bounded_language_context(context, L"你好");
    owo::tsf::append_bounded_language_context(context, L"世界");
    if (context != L"你好世界") return 1;

    owo::tsf::append_bounded_language_context(
        context, L"123456789012345678901234567890");
    if (context != L"789012345678901234567890" ||
        context.size() != owo::tsf::kDefaultLanguageContextUtf16Units) return 2;

    context.assign({static_cast<wchar_t>(0xd83d), static_cast<wchar_t>(0xde00)});
    context.append(23, L'x');
    owo::tsf::append_bounded_language_context(context, L"");
    if (context.size() != 23 || context.front() != L'x') return 3;

    owo::tsf::append_bounded_language_context(context, L"ignored", 0);
    if (!context.empty()) return 4;

    owo::tsf::append_ranking_language_context(context, L"前缀１２３");
    owo::tsf::append_ranking_language_context(context, L"④\U0001d7db");
    if (context != L"前缀0" || !owo::engine::ranking_context_ends_in_number(std::wstring_view(context))) return 5;
    owo::tsf::append_ranking_language_context(context, L"，一起5.64号");
    if (context != L"前缀0，一起0.0号" || owo::engine::ranking_context_ends_in_number(std::wstring_view(context))) return 6;
    std::string encoded = "订单１２3④\U0001d7db；一起²Ⅵ";
    if (!owo::engine::ranking_numbers_need_redaction(encoded)) return 7;
    owo::engine::redact_ranking_numbers(encoded);
    if (encoded != "订单0；一起0" || owo::engine::ranking_numbers_need_redaction(encoded) ||
        !owo::engine::ranking_context_ends_in_number(std::string_view(encoded))) return 8;
    return 0;
}

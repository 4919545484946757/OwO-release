#include "owo/ipc/user_lexicon_management.h"
#include <charconv>
#include <limits>
#include <set>
#include <sstream>

namespace owo::ipc {
namespace {
std::vector<std::string> split(std::string_view value, char separator) {
    std::vector<std::string> result;
    for (;;) {
        const auto at = value.find(separator);
        result.emplace_back(value.substr(0, at));
        if (at == value.npos)
            return result;
        value.remove_prefix(at + 1);
    }
}
bool number(std::string_view text, std::uint64_t &value) {
    const auto r = std::from_chars(text.data(), text.data() + text.size(), value);
    return r.ec == std::errc{} && r.ptr == text.data() + text.size();
}
std::string word_key(const engine::UserLexiconEntry &entry) {
    std::string key;
    for (auto c : entry.input)
        if (c != '\'')
            key += c >= 'A' && c <= 'Z' ? c - 'A' + 'a' : c;
    key.push_back('\0');
    key += entry.text;
    return key;
}
} // namespace
UserLexiconManagementResult manage_user_lexicon(engine::UserFrequencyStore &store,
                                                std::string_view request, std::uint64_t &revision) {
    const auto fail = [](std::string error) {
        return UserLexiconManagementResult{false, false, std::move(error)};
    };
    if (request.size() > 512 * 1024)
        return fail("词库请求过大。每次最多导入 5000 条、512 KiB 数据。");
    auto lines = split(request, '\n');
    if (lines.size() < 3)
        return fail("词库请求格式错误。");
    const auto header = split(lines[0], '\t');
    const auto old = split(lines[2], '\t');
    if (header.size() != 5 || old.size() != 2 || lines[1].size() > 256)
        return fail("词库请求字段错误。");
    const auto &action = header[0];
    if (action != "list" && action != "get" && action != "save" && action != "delete" &&
        action != "clear" && action != "preview" && action != "import")
        return fail("不支持的词库操作。");
    std::uint64_t expected{}, offset{}, limit{};
    if (!number(header[2], offset) || !number(header[3], limit) || offset > 1000000 || limit < 1 ||
        limit > 200)
        return fail("词库分页参数无效。");
    if (header[1] == "-") {
        if (action != "list" && action != "get")
            return fail("修改词库需要有效版本。");
    } else if (!number(header[1], expected) || expected != revision)
        return fail("词库已被学习或其他窗口更新，请刷新后重试。");
    if (header[4] != "merge" && header[4] != "replace-confirmed" && header[4] != "clear-confirmed")
        return fail("导入或清空确认参数无效。");
    std::vector<engine::UserLexiconEntry> entries;
    std::set<std::string> keys;
    for (std::size_t i = 3; i < lines.size(); ++i) {
        if (lines[i].empty() && i + 1 == lines.size())
            continue;
        const auto fields = split(lines[i], '\t');
        std::uint64_t count{};
        if (fields.size() != 3 || !number(fields[2], count) || count > 128)
            return fail("词条字段或权重无效。");
        engine::UserLexiconEntry entry{fields[1], static_cast<std::uint32_t>(count), fields[0]};
        auto error = engine::UserFrequencyStore::validate_word(entry);
        if (!error.empty())
            return fail(error);
        if (!keys.insert(word_key(entry)).second)
            return fail("导入文件有重复的编码和词语。");
        entries.push_back(std::move(entry));
        if (entries.size() > 5000)
            return fail("每次最多导入 5000 条。");
    }
    if ((action == "list" || action == "get" || action == "delete" || action == "clear") &&
        !entries.empty())
        return fail("此操作不接受词条数据。");
    if (action == "save" && entries.size() != 1)
        return fail("保存必须提供一条词语。");
    if ((action == "import" || action == "preview") && entries.empty())
        return fail("导入文件为空。");
    if ((action == "delete" || action == "get") && (old[0].empty() || old[1].empty()))
        return fail("请选择词条。");
    if (action == "clear" && header[4] != "clear-confirmed")
        return fail("清空操作需要单独确认。");
    if (action != "save" && action != "delete" && action != "get" &&
        (!old[0].empty() || !old[1].empty()))
        return fail("此操作不接受原词条。");
    std::size_t added = 0, updated = 0;
    if (!entries.empty()) {
        std::set<std::string> existing;
        for (const auto &entry :
             store.list_words({}, 0, (std::numeric_limits<std::size_t>::max)()).entries)
            existing.insert(word_key(entry));
        for (const auto &entry : entries)
            existing.contains(word_key(entry)) ? ++updated : ++added;
    }
    bool mutated = false;
    if (action != "list" && action != "get" && action != "preview") {
        auto staged = store;
        if (action == "clear")
            staged.clear_learning();
        else {
            auto error = staged.edit_words(entries, old[0], old[1],
                                           action == "import" && header[4] == "replace-confirmed");
            if (!error.empty())
                return fail(error);
        }
        // Persist in-memory selections before the replacement creates its .bak.
        const auto before = store.flush();
        if (!before.success)
            return fail("无法备份当前词库：" + before.error);
        const auto written = staged.flush();
        if (!written.success)
            return fail("词库未修改：" + written.error);
        store = std::move(staged);
        ++revision;
        mutated = true;
    }
    auto page = store.list_words(lines[1], static_cast<std::size_t>(offset),
                                 static_cast<std::size_t>(limit));
    if (action == "get") {
        const auto error = engine::UserFrequencyStore::validate_word({old[1], 1, old[0]});
        if (!error.empty())
            return fail(error);
        page.entries.clear();
        const auto target = word_key({old[1], 1, old[0]});
        for (const auto &entry :
             store.list_words({}, 0, (std::numeric_limits<std::size_t>::max)()).entries)
            if (word_key(entry) == target)
                page.entries.push_back(entry);
        page.total = page.entries.size();
    }
    std::string response = std::to_string(revision) + '\t' + std::to_string(page.total) + '\t' +
                           std::to_string(added) + '\t' + std::to_string(updated) + '\n';
    for (const auto &entry : page.entries)
        response += entry.input + '\t' + entry.text + '\t' + std::to_string(entry.count) + '\t' +
                    (entry.preferred ? "1\n" : "0\n");
    return {true, mutated, std::move(response)};
}
} // namespace owo::ipc

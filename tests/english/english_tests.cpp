#include "../../plugins/english/src/provider.h"
#include "owo/core/input_method_runtime.h"
#include "owo/ipc/input_method_client.h"
#include "owo/ipc/input_method_server.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "owo/plugin/plugin_store.h"

#include <Windows.h>
#include <algorithm>
#include <fstream>
#include <iostream>
#include <stdexcept>

#define CHECK(condition) do { if (!(condition)) throw std::runtime_error(#condition); } while (false)
namespace {
using namespace owo;
using namespace std::chrono_literals;
using namespace engine;
namespace fs = std::filesystem;

void write(const fs::path& path, std::string_view data) {
    fs::create_directories(path.parent_path());
    std::ofstream file(path, std::ios::binary);
    file.write(data.data(), static_cast<std::streamsize>(data.size()));
    CHECK(file.good());
}
std::string read(const fs::path& path) {
    std::ifstream file(path, std::ios::binary);
    return {std::istreambuf_iterator<char>(file), {}};
}
const auto scheme = english::descriptor();
InputMethodRequest request(std::string input, std::uint64_t sequence, std::size_t page = 0, std::size_t size = 7) {
    InputMethodRequest r;
    r.request_id = sequence; r.session_id = "test"; r.scheme_id = scheme.id; r.input = std::move(input);
    r.caret_byte = r.input.size(); r.page = page; r.page_size = size;
    return r;
}
bool has(const InputMethodResponse& out, std::string_view value) {
    return std::ranges::any_of(out.candidates, [&](const auto& row) { return row.text == value; });
}
void test_provider(const fs::path& package, const fs::path& store) {
    const auto dir = store / "input-schemes" / "english";
    write(dir / "user-lexicon.tsv", "\xef\xbb\xbf" "owotestalpha\t1000\r\nowotestbeta\t1000\r\ninvalid row\t7\r\n");
    write(dir / "dictionaries/libreoffice-en-us.tsv", "2\nownhunspell/SM\tpo:noun\n123bad/X\n");
    const auto asset = package / "assets/wordfreq-en.msgpack";
    english::Provider provider(asset, dir);
    std::uint64_t sequence = 0;
    const auto compose = [&](std::string text, std::size_t page = 0, std::size_t size = 7) {
        const auto r = request(std::move(text), ++sequence, page, size);
        const auto out = provider.handle(r, {});
        CHECK(validate_input_method_response(scheme, r, out).ok);
        CHECK(out.status == InputMethodStatus::success);
        return out;
    };
    auto out = compose("prog");
    CHECK(out.candidates.front().text == "prog" && has(out, "program") && out.has_more);
    CHECK(has(compose("Prog"), "Program"));
    CHECK(has(compose("PROG"), "PROGRAM"));
    CHECK(has(compose("pROg"), "pROgram"));
    CHECK(has(compose("don'"), "don't"));
    CHECK(has(compose("recieve"), "receive"));
    CHECK(has(compose("ownhun"), "ownhunspell"));
    CHECK(compose("qqqqqqqq").candidates.front().text == "qqqqqqqq");
    const auto page0 = compose("prog", 0, 4);
    const auto page1 = compose("prog", 1, 4);
    const auto expanded = compose("prog", 0, 8);
    CHECK(expanded.candidates.size() == 8);
    for (std::size_t i = 0; i < 4; ++i) {
        CHECK(expanded.candidates[i].id == page0.candidates[i].id);
        CHECK(expanded.candidates[i + 4].id == page1.candidates[i].id);
    }
    auto select = request("owotest", ++sequence);
    CHECK(compose("owotest").candidates[1].text == "owotestbeta"); // equal weight; shorter spelling wins
    select.request_id = ++sequence;
    select.action = InputMethodAction::select; select.selected_candidate_id = "word:owotestalpha";
    const auto prepared = provider.handle(select, {});
    CHECK(prepared.commit_id.size() == 32 && prepared.candidates.front().text == "owotestalpha");
    CHECK(!fs::exists(dir / "learned.tsv"));
    auto ack = select; ack.action = InputMethodAction::committed; ack.selected_candidate_id = prepared.commit_id;
    auto bad = ack; bad.page = 1; bad.request_id = ++sequence;
    CHECK(provider.handle(bad, {}).status == InputMethodStatus::error);
    ack.request_id = ++sequence;
    CHECK(provider.handle(ack, {}).status == InputMethodStatus::success);
    ack.request_id = ++sequence;
    CHECK(provider.handle(ack, {}).status == InputMethodStatus::success);
    CHECK(read(dir / "learned.tsv") == "owotestalpha\t1\n");
    CHECK(compose("owotest").candidates[1].text == "owotestalpha");
    english::Provider reopened(asset, dir);
    CHECK(reopened.handle(request("owotest", 1), {}).candidates[1].text == "owotestalpha");
    CHECK(fs::remove(dir / "learned.tsv"));
    CHECK(compose("owotes").candidates[1].text == "owotestbeta");
    write(store / "settings.json", R"({"show_completions":false,"show_spelling_suggestions":false,"learn_selected_words":false})");
    CHECK(compose("prog").candidates.size() == 1);
    CHECK(compose("recieve").candidates.size() == 1);
    std::stop_source stop; stop.request_stop();
    CHECK(provider.handle(request("a", ++sequence), stop.get_token()).status == InputMethodStatus::cancelled);
}

void test_process(const fs::path& package, const fs::path& store) {
    CHECK(plugin::initialize_plugin_store(store).ok);
    const auto staging = store / "staging/.english";
    fs::copy(package, staging, fs::copy_options::recursive);
    CHECK(plugin::publish_staged_plugin(store, staging, std::string(64, 'a'), std::string(64, 'b')).ok);
    const auto active = plugin::query_active_plugin_version(store, scheme.plugin_id);
    CHECK(active.ok);
    const auto pipe = std::wstring(LR"(\\.\pipe\OwO.Test.English.)") + std::to_wstring(GetCurrentProcessId());
    ipc::InputMethodPipeServer server(store, pipe, [](std::uint32_t) { return ipc::InputMethodFocus{1, false}; });
    CHECK(server.started());
    ipc::InputMethodAsyncClient client(pipe);
    const auto wait = [](std::future<ipc::InputMethodClientResult> result) {
        CHECK(result.wait_for(6s) == std::future_status::ready);
        return result.get();
    };
    CHECK(!wait(client.open(1, scheme.plugin_id, scheme.id, "denied", false)).status);
    const auto grant = plugin::make_plugin_authorization(active.manifest, active.inventory_sha256,
        active.publisher_certificate_sha256, {"input.scheme"});
    CHECK(grant.ok && plugin::save_plugin_authorization(store, grant.value).ok);
    const auto discovered = core::discover_plugin_input_methods(store, scheme.plugin_id, 5s);
    if (!discovered.validation.ok) std::cerr << discovered.validation.diagnostic << '\n';
    CHECK(discovered.validation.ok);
    const auto started = std::chrono::steady_clock::now();
    const auto open = wait(client.open(2, scheme.plugin_id, scheme.id, "test", false));
    if (!open.status) std::cerr << open.status.message << '\n';
    CHECK(open.status && open.descriptors.size() == 1 && open.descriptors[0].id == scheme.id);
    auto r = request("Prog", 1);
    auto result = wait(client.submit(2, r, false));
    if (result.response && result.response->status != InputMethodStatus::success)
        std::cerr << result.response->diagnostic << '\n';
    CHECK(result.status && result.response && has(*result.response, "Program"));
    std::cout << "cold open and candidates: "
              << std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - started).count()
              << " ms\n";
    r.action = InputMethodAction::select; r.selected_candidate_id = "word:program"; ++r.request_id;
    result = wait(client.submit(2, r, false));
    CHECK(result.status && result.response && result.response->commit_id.size() == 32 && has(*result.response, "Program"));
    const auto learned = store / "data" / scheme.plugin_id / "input-schemes/english/learned.tsv";
    CHECK(!fs::exists(learned));
    r.action = InputMethodAction::committed; r.selected_candidate_id = result.response->commit_id; ++r.request_id;
    result = wait(client.submit(2, r, false));
    CHECK(result.status && result.response && result.response->status == InputMethodStatus::success);
    CHECK(read(learned) == "program\t1\n");
    const auto warm = std::chrono::steady_clock::now();
    result = wait(client.submit(2, request("PROG", 4), false));
    CHECK(result.status && result.response && has(*result.response, "PROGRAM"));
    std::cout << "next word in same session: "
              << std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - warm).count()
              << " ms\n";
    CHECK(wait(client.close()).status);
}
} // namespace

int main(int argc, char** argv) {
    try {
        CHECK(argc == 4);
        const auto store = fs::absolute(fs::path(argv[3]) /
            (std::to_string(GetCurrentProcessId()) + "-" + std::to_string(GetTickCount64())));
        CHECK(!fs::exists(store));
        fs::create_directories(store.parent_path());
        if (std::string_view(argv[1]) == "provider") test_provider(argv[2], store);
        else if (std::string_view(argv[1]) == "process") test_process(argv[2], store);
        else return 2;
    } catch (const std::exception& error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}

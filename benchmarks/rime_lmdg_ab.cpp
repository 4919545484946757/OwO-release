// Standalone evaluation harness. Build against the official librime C header;
// no Rime dependency is added to the OwO product. Takes only test codes on stdin.
#include <Windows.h>
#include <rime_api.h>

#include <chrono>
#include <filesystem>
#include <iostream>
#include <string>
#include <string_view>

namespace {
using Clock = std::chrono::steady_clock;
std::string utf8_path(const std::filesystem::path& path) {
    const auto utf8 = path.u8string();
    return {utf8.begin(), utf8.end()};
}
double elapsed_ms(const Clock::time_point start) {
    return std::chrono::duration<double, std::milli>(Clock::now() - start).count();
}
void string_json(std::string_view text) {
    std::cout << '"';
    for (const unsigned char c : text) {
        if (c == '"' || c == '\\') std::cout << '\\' << c;
        else if (c < 32) {
            constexpr char hex[] = "0123456789abcdef";
            std::cout << "\\u00" << hex[c >> 4] << hex[c & 15];
        } else std::cout << c;
    }
    std::cout << '"';
}
}

int wmain(int argc, wchar_t** argv) {
    if (argc != 6) {
        std::cerr << "rime_lmdg_ab <rime.dll> <shared-fixture-dir> <user-fixture-dir> <schema> <expected-grammar-or-dash>\n";
        return 2;
    }
    const auto dll = std::filesystem::absolute(argv[1]);
    const auto shared = utf8_path(std::filesystem::absolute(argv[2]));
    const auto user = utf8_path(std::filesystem::absolute(argv[3]));
    const auto schema = utf8_path(argv[4]);
    const auto expected_grammar = utf8_path(argv[5]);
    if (shared == user || !std::filesystem::is_directory(argv[2]) ||
        !std::filesystem::is_directory(argv[3])) return 2;
    const auto module = LoadLibraryExW(dll.c_str(), nullptr,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!module) { std::cerr << "librime load failed: " << GetLastError() << '\n'; return 3; }
    const auto get_api = reinterpret_cast<decltype(&rime_get_api)>(GetProcAddress(module, "rime_get_api"));
    const auto* api = get_api ? get_api() : nullptr;
    if (!RIME_PROVIDED(api, set_input) || !RIME_PROVIDED(api, candidate_list_next)) return 4;
    RIME_STRUCT(RimeTraits, traits);
    traits.shared_data_dir = shared.c_str();
    traits.user_data_dir = user.c_str();
    traits.log_dir = user.c_str();
    traits.app_name = "rime.owo-ab";
    api->setup(&traits);
    api->initialize(&traits);
    struct Finalize { const RimeApi* api; ~Finalize() { api->finalize(); } } finalize{api};
    const auto started = Clock::now();
    if (api->start_maintenance(true)) api->join_maintenance_thread();
    const auto session = api->create_session();
    if (!session || !api->select_schema(session, schema.c_str())) { std::cerr << "test schema load failed\n"; return 5; }
    api->set_option(session, "ascii_mode", false);
    RimeConfig config{};
    if (!api->schema_open(schema.c_str(), &config)) return 5;
    char grammar[128]{};
    const bool has_grammar = api->config_get_string(&config, "grammar/language", grammar, sizeof grammar);
    api->config_close(&config);
    const bool octagram = api->find_module("octagram") != nullptr;
    if ((expected_grammar == "-" && has_grammar) || (expected_grammar != "-" &&
        (!has_grammar || !octagram || expected_grammar != grammar))) {
        std::cerr << "unexpected deployed grammar or octagram unavailable: " << grammar << '\n';
        return 6;
    }
    std::cout << "{\"init_deploy_ms\":" << elapsed_ms(started) << ",\"version\":";
    string_json(api->get_version());
    std::cout << ",\"octagram\":" << (octagram ? "true" : "false") << ",\"grammar\":";
    string_json(grammar);
    std::cout << "}\n";
    std::string input;
    while (std::getline(std::cin, input)) {
        if (!input.empty() && input.back() == '\r') input.pop_back();
        if (input.empty()) continue;
        if (input.size() > 128 || input.find_first_not_of("abcdefghijklmnopqrstuvwxyz'") != std::string::npos) return 7;
        api->clear_composition(session);
        RIME_STRUCT(RimeContext, context);
        const auto query_started = Clock::now();
        if (!api->set_input(session, input.c_str()) || !api->get_context(session, &context)) return 8;
        const auto first_page_ms = elapsed_ms(query_started);
        std::cout << "{\"input\":";
        string_json(input);
        std::cout << ",\"first_page_ms\":" << first_page_ms << ",\"first\":[";
        for (int i = 0; i < context.menu.num_candidates; ++i) {
            if (i) std::cout << ',';
            string_json(context.menu.candidates[i].text);
        }
        api->free_context(&context);
        std::cout << "],\"top32\":[";
        RimeCandidateListIterator iterator{};
        if (api->candidate_list_begin(session, &iterator)) {
            for (int i = 0; i < 32 && api->candidate_list_next(&iterator); ++i) {
                if (i) std::cout << ',';
                string_json(iterator.candidate.text);
            }
            api->candidate_list_end(&iterator);
        }
        std::cout << "]}\n";
    }
    api->destroy_session(session);
    return 0;
}

#include "owo/ipc/named_pipe.h"
#include "owo/ipc/input_method_server.h"
#include "owo/config/config_monitor.h"
#include "owo/config/config_paths.h"
#include "owo/core/plugin_executor.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/engine/concept_candidates.h"
#include "owo/engine/user_frequency.h"

#include <iostream>
#include <memory>
#include <chrono>
#include <cstdlib>
#include <optional>
#include <string>
#include <string_view>
#include <utility>

int wmain(int argc, wchar_t** argv) {
    const auto process_started = std::chrono::steady_clock::now();
    const auto startup_log = [](const std::string_view phase,
                                const std::chrono::steady_clock::time_point started) {
        const auto duration = std::chrono::duration_cast<std::chrono::microseconds>(
            std::chrono::steady_clock::now() - started).count();
        std::clog << R"({"process":"core_service","module":"startup","level":"info","event_id":")"
                  << phase << R"(","duration_us":)" << duration << "}\n";
    };
    const wchar_t* lexicon_path = nullptr;
    const wchar_t* user_frequency_path = nullptr;
    const wchar_t* model_pipe_name = nullptr;
    const wchar_t* config_path = nullptr;
    const wchar_t* plugin_store_path = nullptr;
    const wchar_t* concept_glossary_path = nullptr;
    bool config_disabled = false;
    // Listener alone grants nothing. Every open still requires explicit package-bound authorization.
    bool experimental_input_schemes = true;
    for (int index = 1; index < argc;) {
        const std::wstring_view option(argv[index]);
        if (option == L"--experimental-input-schemes") {
            experimental_input_schemes = true;
            ++index;
        } else if (option == L"--model-host") {
            model_pipe_name = owo::ipc::kModelHostPipeName;
            ++index;
        } else if (option == L"--no-config") {
            config_disabled = true;
            ++index;
        } else if (index + 1 >= argc) {
            return 2;
        } else if (option == L"--lexicon") {
            lexicon_path = argv[index + 1];
            index += 2;
        } else if (option == L"--user-frequency") {
            user_frequency_path = argv[index + 1];
            index += 2;
        } else if (option == L"--config") {
            config_path = argv[index + 1];
            index += 2;
        } else if (option == L"--plugin-store") {
            plugin_store_path = argv[index + 1];
            index += 2;
        } else if (option == L"--concept-glossary") {
            concept_glossary_path = argv[index + 1];
            index += 2;
        }
        else return 2;
    }
    if (config_disabled && config_path != nullptr) return 2;
    const auto data_root = owo::config::local_data_root();
    const auto default_plugin_store = data_root.empty()
        ? std::filesystem::path{} : data_root / L"plugins";
    std::filesystem::path effective_plugin_store = plugin_store_path != nullptr
        ? std::filesystem::path(plugin_store_path) : default_plugin_store;
    effective_plugin_store = effective_plugin_store.lexically_normal();
    if (effective_plugin_store.empty()) {
        std::cerr << "default_plugin_store_unavailable\n";
        return 3;
    }
    if (!effective_plugin_store.is_absolute() ||
        effective_plugin_store.root_name().native().size() != 2 ||
        effective_plugin_store.root_name().native()[1] != L':' ||
        effective_plugin_store == effective_plugin_store.root_path()) {
        std::cerr << "plugin_store_must_be_local_absolute_child\n";
        return 2;
    }
    owo::core::PluginExecutor plugin_executor(effective_plugin_store);
    std::unique_ptr<owo::ipc::InputMethodPipeServer> input_schemes;
    if (experimental_input_schemes) {
        input_schemes = std::make_unique<owo::ipc::InputMethodPipeServer>(effective_plugin_store);
        if (!input_schemes->started()) {
            std::cerr << "experimental_input_scheme_pipe_unavailable\n";
            input_schemes.reset(); // basic candidates remain available
        }
    }
    std::clog << R"({"process":"core_service","module":"plugin","level":"info","event_id":"plugin_worker_ready"})"
              << '\n';
    const auto default_config_path = owo::config::default_config_path();
    if (!config_disabled && config_path == nullptr) {
        if (default_config_path.empty()) {
            std::cerr << "default_config_path_unavailable\n";
            return 3;
        }
        config_path = default_config_path.c_str();
    }
    owo::config::ConfigMonitor config_monitor;
    const owo::config::ConfigMonitor* config_monitor_ptr = nullptr;
    if (config_path != nullptr) {
        const auto phase_started = std::chrono::steady_clock::now();
        const auto loaded = config_monitor.start(config_path);
        if (!loaded.success) {
            std::cerr << "config_start_failed: " << loaded.diagnostic << '\n';
            return 3;
        }
        if (!loaded.diagnostic.empty())
            std::clog << "config_diagnostic: " << loaded.diagnostic << '\n';
        config_monitor_ptr = &config_monitor;
        startup_log("config_loaded", phase_started);
    }
    // Glossary resolution order: explicit flag, environment, then the
    // well-known data file. A missing file simply keeps the built-in starter
    // glossary, so the input method always starts.
    std::wstring concept_glossary_storage;
    if (concept_glossary_path == nullptr) {
        std::size_t required = 0;
        if (_wgetenv_s(&required, nullptr, 0, L"OWO_CONCEPT_GLOSSARY") == 0 &&
            required > 1) {
            std::wstring value(required, L'\0');
            if (_wgetenv_s(&required, value.data(), value.size(),
                           L"OWO_CONCEPT_GLOSSARY") == 0) {
                value.resize(required > 0 ? required - 1 : 0);
                concept_glossary_storage = std::move(value);
                concept_glossary_path = concept_glossary_storage.c_str();
            }
        }
    }
    if (concept_glossary_path == nullptr && !data_root.empty()) {
        const auto candidate = data_root / L"data" / L"concept-glossary.tsv";
        std::error_code ignored;
        if (std::filesystem::is_regular_file(candidate, ignored)) {
            concept_glossary_storage = candidate.wstring();
            concept_glossary_path = concept_glossary_storage.c_str();
        }
    }
    // A project glossary replaces the built-in starter table; a broken file is
    // reported and ignored so the input method always starts.
    std::optional<owo::engine::ConceptTable> concept_table;
    if (concept_glossary_path != nullptr) {
        const auto phase_started = std::chrono::steady_clock::now();
        const auto loaded = owo::engine::load_concept_table(concept_glossary_path);
        if (!loaded.table) {
            std::cerr << "concept_glossary_load_failed: " << loaded.error << '\n';
            std::clog << R"({"process":"core_service","module":"concept","level":"warn","event_id":"concept_glossary_rejected"})"
                      << '\n';
        } else {
            concept_table = std::move(*loaded.table);
            startup_log("concept_glossary_loaded", phase_started);
            std::clog << R"({"process":"core_service","module":"concept","level":"info","event_id":"concept_glossary_ready","entries":)"
                      << concept_table->entries().size() << "}\n";
        }
    }
    const auto concept_table_ptr =
        concept_table ? &*concept_table : nullptr;
    owo::engine::UserFrequencyStore user_frequency;
    owo::engine::UserFrequencyStore* user_frequency_ptr = nullptr;
    if (user_frequency_path != nullptr) {
        const auto phase_started = std::chrono::steady_clock::now();
        const auto loaded = user_frequency.load(user_frequency_path);
        if (!loaded.success) {
            std::cerr << "user_frequency_load_failed: " << loaded.error << '\n';
            return 3;
        }
        user_frequency_ptr = &user_frequency;
        startup_log("user_frequency_loaded", phase_started);
    }
    if (lexicon_path != nullptr) {
        owo::engine::BinaryLexicon lexicon;
        const auto phase_started = std::chrono::steady_clock::now();
        const auto loaded = lexicon.load(lexicon_path);
        if (!loaded.success) {
            std::cerr << "lexicon_load_failed: " << loaded.error << '\n';
            return 3;
        }
        startup_log("lexicon_mapped", phase_started);
        startup_log("core_initialized", process_started);
        return owo::ipc::run_core_server(owo::ipc::kCorePipeName, lexicon,
                                         user_frequency_ptr, model_pipe_name,
                                         config_monitor_ptr, &plugin_executor,
                                         concept_table_ptr);
    }
    if (user_frequency_ptr != nullptr) {
        const owo::engine::MemoryLexicon fallback({
            {{"ni", "hao"}, "你好", 1000}, {{"ni", "hao"}, "你号", 50},
            {{"xian"}, "先", 800}, {{"xian"}, "线", 700}, {{"xi", "an"}, "西安", 900}});
        return owo::ipc::run_core_server(owo::ipc::kCorePipeName, fallback,
                                         user_frequency_ptr, model_pipe_name,
                                         config_monitor_ptr, &plugin_executor,
                                         concept_table_ptr);
    }
    if (model_pipe_name != nullptr) {
        const owo::engine::MemoryLexicon fallback({
            {{"ni", "hao"}, "你好", 1000}, {{"ni", "hao"}, "你号", 50},
            {{"xian"}, "先", 800}, {{"xian"}, "线", 700}, {{"xi", "an"}, "西安", 900}});
        return owo::ipc::run_core_server(owo::ipc::kCorePipeName, fallback, nullptr,
                                         model_pipe_name, config_monitor_ptr,
                                         &plugin_executor, concept_table_ptr);
    }
    if (config_monitor_ptr != nullptr) {
        const owo::engine::MemoryLexicon fallback({
            {{"ni", "hao"}, "你好", 1000}, {{"ni", "hao"}, "你号", 50},
            {{"xian"}, "先", 800}, {{"xian"}, "线", 700}, {{"xi", "an"}, "西安", 900}});
        return owo::ipc::run_core_server(owo::ipc::kCorePipeName, fallback, nullptr,
                                         nullptr, config_monitor_ptr,
                                         &plugin_executor, concept_table_ptr);
    }
    const owo::engine::MemoryLexicon fallback({
        {{"ni", "hao"}, "你好", 1000}, {{"ni", "hao"}, "你号", 50},
        {{"xian"}, "先", 800}, {{"xian"}, "线", 700}, {{"xi", "an"}, "西安", 900}});
    return owo::ipc::run_core_server(owo::ipc::kCorePipeName, fallback, nullptr,
                                     nullptr, nullptr, &plugin_executor,
                                     concept_table_ptr);
}

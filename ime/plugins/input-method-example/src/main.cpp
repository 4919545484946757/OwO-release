#include "scheme.h"
#include "owo/plugin/input_method_protocol.h"
#include "owo/plugin/plugin_pipe.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>

#include <algorithm>
#ifdef OWO_INPUT_METHOD_TEST_FAULTS
#include <filesystem>
#include <fstream>
#endif

int wmain(int argc, wchar_t** argv) {
    using namespace owo;
    if (argc != 7 || std::wstring_view(argv[1]) != L"--owo-plugin-pipe" ||
        std::wstring_view(argv[3]) != L"--owo-plugin-id" ||
        std::wstring_view(argv[5]) != L"--owo-plugin-data") return 1;
    const std::wstring_view wide_id(argv[4]);
    if (wide_id.empty() || std::any_of(wide_id.begin(), wide_id.end(), [](wchar_t c) { return c > 127; })) return 2;
    std::string plugin_id(wide_id.size(), '\0');
    std::transform(wide_id.begin(), wide_id.end(), plugin_id.begin(),
        [](wchar_t c) { return static_cast<char>(c); });
    HANDLE token{};
    DWORD appcontainer{}, returned{};
    const bool sandboxed = OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token) &&
        GetTokenInformation(token, TokenIsAppContainer, &appcontainer, sizeof(appcontainer), &returned) && appcontainer;
    if (token) CloseHandle(token);
    BOOL in_job{};
    if (!sandboxed || !IsProcessInJob(GetCurrentProcess(), nullptr, &in_job) || !in_job) return 3;
#ifdef OWO_INPUT_METHOD_TEST_FAULTS
    // Real AppContainer resource and budget checks, only in the test probe.
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
    if (!QueryInformationJobObject(nullptr, JobObjectExtendedLimitInformation, &limits, sizeof(limits), nullptr) ||
        limits.ProcessMemoryLimit != 512ULL * 1024ULL * 1024ULL) return 30;
    std::ifstream dictionary("assets/dictionary-test.txt", std::ios::binary);
    std::string word; dictionary >> word;
    if (word != "packaged-dictionary") return 31;
    dictionary.close();
    std::ofstream forbidden("assets/should-not-write.txt", std::ios::binary);
    if (forbidden) return 32;
    const auto private_file = std::filesystem::path(argv[6]) / "private-learning-test.txt";
    { std::ofstream data(private_file, std::ios::binary); data << "private-data"; if (!data) return 33; }
    std::ifstream data(private_file, std::ios::binary); data >> word;
    if (word != "private-data") return 34;
#endif
    auto connection = plugin::connect_plugin_pipe_client(argv[2], std::chrono::seconds(5));
    if (!connection) return 4;
    plugin::PluginMessage hello;
    hello.type = plugin::PluginMessageType::hello_request;
    hello.status = plugin::PluginStatus::success;
    hello.request_id = 1; hello.plugin_id = plugin_id;
    if (!plugin::send_plugin_pipe_message(connection.pipe, hello, std::chrono::seconds(5)).ok) return 5;
    const auto reply = plugin::receive_plugin_pipe_message(connection.pipe, std::chrono::seconds(5));
    if (!reply.status.ok || reply.message.type != plugin::PluginMessageType::hello_response ||
        reply.message.request_id != hello.request_id || reply.message.plugin_id != plugin_id ||
        reply.message.capabilities != std::vector<std::string>{"cancel.v1", "invoke.v1"}) return 6;
    const auto descriptor = example::table_descriptor(plugin_id);
    for (;;) {
        const auto incoming = plugin::receive_plugin_pipe_message(connection.pipe, std::chrono::hours(24));
        if (!incoming.status.ok || incoming.message.plugin_id != plugin_id) return 7;
        const auto& call = incoming.message;
        plugin::PluginMessage result;
        result.plugin_id = plugin_id; result.request_id = call.request_id;
        result.status = plugin::PluginStatus::success;
        if (call.type == plugin::PluginMessageType::shutdown_request || call.type == plugin::PluginMessageType::cancel_request) {
            result.type = plugin::PluginMessageType::acknowledgement;
            if (!plugin::send_plugin_pipe_message(connection.pipe, result, std::chrono::seconds(5)).ok) return 8;
            if (call.type == plugin::PluginMessageType::shutdown_request) return 0;
            continue;
        }
        if (call.type != plugin::PluginMessageType::invoke_request) return 9;
        result.type = plugin::PluginMessageType::invoke_response;
        if (call.service == plugin::kInputMethodDescribeService && call.payload.empty()) {
#ifdef OWO_INPUT_METHOD_TEST_FAULTS
            std::ofstream marker(std::filesystem::path(argv[6]) / "describe-active.txt", std::ios::binary);
            marker << call.request_id;
            marker.close();
            Sleep(100);
#endif
            result.payload = plugin::encode_input_method_descriptors(plugin_id, {descriptor});
            if (!plugin::send_plugin_pipe_message(connection.pipe, result, std::chrono::seconds(5)).ok) return 10;
            continue;
        }
        engine::InputMethodRequest request;
        if (call.service != plugin::kInputMethodService ||
            !plugin::decode_input_method_request(descriptor, call.payload, request).ok) {
            result.status = plugin::PluginStatus::invalid_request;
            result.diagnostic = "Invalid experimental scheme request";
        } else {
#ifdef OWO_INPUT_METHOD_TEST_FAULTS
            // Only compiled into the test probe, never into the example plugin.
            if (request.input == "hang" || request.input == "slow") {
                std::ofstream marker(std::filesystem::path(argv[6]) / "input-active.txt", std::ios::binary);
                marker << request.request_id;
                marker.close();
                Sleep(request.input == "hang" ? 10000 : 250);
            }
            if (request.input == "crash") return 77;
#endif
            result.payload = plugin::encode_input_method_response(descriptor, request, example::table_response(request));
#ifdef OWO_INPUT_METHOD_TEST_FAULTS
            if (request.input == "bad") result.payload += "trailing";
#endif
        }
        if (!plugin::send_plugin_pipe_message(connection.pipe, result, std::chrono::seconds(5)).ok) return 10;
    }
}

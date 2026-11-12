#include "provider.h"
#include "owo/plugin/input_method_protocol.h"
#include "owo/plugin/plugin_pipe.h"

#include <Windows.h>

int wmain(int argc, wchar_t** argv) {
    using namespace owo;
    using namespace std::chrono_literals;
    const auto descriptor = english::descriptor();
    const std::wstring expected_id(descriptor.plugin_id.begin(), descriptor.plugin_id.end());
    if (argc != 7 || std::wstring_view(argv[1]) != L"--owo-plugin-pipe" ||
        std::wstring_view(argv[3]) != L"--owo-plugin-id" || argv[4] != expected_id ||
        std::wstring_view(argv[5]) != L"--owo-plugin-data") return 1;
    HANDLE token{};
    DWORD appcontainer{}, size{};
    const bool sandboxed = OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token) &&
        GetTokenInformation(token, TokenIsAppContainer, &appcontainer, sizeof(appcontainer), &size) && appcontainer;
    if (token) CloseHandle(token);
    BOOL in_job{};
    if (!sandboxed || !IsProcessInJob(GetCurrentProcess(), nullptr, &in_job) || !in_job) return 2;
    auto connection = plugin::connect_plugin_pipe_client(argv[2], 5s);
    if (!connection) return 3;

    plugin::PluginMessage hello;
    hello.type = plugin::PluginMessageType::hello_request;
    hello.status = plugin::PluginStatus::success;
    hello.request_id = 1; hello.plugin_id = descriptor.plugin_id;
    if (!plugin::send_plugin_pipe_message(connection.pipe, hello, 5s).ok) return 4;
    const auto reply = plugin::receive_plugin_pipe_message(connection.pipe, 5s);
    if (!reply.status.ok || reply.message.type != plugin::PluginMessageType::hello_response ||
        reply.message.request_id != hello.request_id || reply.message.plugin_id != descriptor.plugin_id ||
        reply.message.capabilities != std::vector<std::string>{"cancel.v1", "invoke.v1"}) return 5;
    std::unique_ptr<english::Provider> provider;
    for (;;) {
        const auto incoming = plugin::receive_plugin_pipe_message(connection.pipe, 24h);
        if (!incoming.status.ok || incoming.message.plugin_id != descriptor.plugin_id) return 6;
        const auto& call = incoming.message;
        plugin::PluginMessage result;
        result.plugin_id = descriptor.plugin_id; result.request_id = call.request_id;
        result.status = plugin::PluginStatus::success;
        if (call.type == plugin::PluginMessageType::shutdown_request || call.type == plugin::PluginMessageType::cancel_request) {
            result.type = plugin::PluginMessageType::acknowledgement;
        } else if (call.type == plugin::PluginMessageType::invoke_request) {
            result.type = plugin::PluginMessageType::invoke_response;
            if (call.service == plugin::kInputMethodDescribeService && call.payload.empty()) {
                result.payload = plugin::encode_input_method_descriptors(descriptor.plugin_id, {descriptor});
            } else if (engine::InputMethodRequest request; call.service == plugin::kInputMethodService &&
                plugin::decode_input_method_request(descriptor, call.payload, request).ok) {
                try {
                    if (!provider) provider = std::make_unique<english::Provider>(L"assets/wordfreq-en.msgpack",
                        std::filesystem::path(argv[6]) / L"input-schemes" / L"english");
                    result.payload = plugin::encode_input_method_response(descriptor, request, provider->handle(request, {}));
                    if (result.payload.empty()) return 7;
                } catch (const std::exception&) { return 7; }
            } else {
                result.status = plugin::PluginStatus::invalid_request;
                result.diagnostic = "Invalid English scheme request";
            }
        } else return 8;
        if (!plugin::send_plugin_pipe_message(connection.pipe, result, 5s).ok) return 9;
        if (call.type == plugin::PluginMessageType::shutdown_request) return 0;
    }
}

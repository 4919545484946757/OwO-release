// Exercise the connector boundary before any pipe/network request can occur.
#define wmain owo_connector_entry_for_test
#include "../../plugins/agent-ipc/src/main.cpp"
#undef wmain

int main() {
    owo::agent::Request request;
    request.session_id = "0123456789abcdef0123456789abcdef";
    request.request_id = request.idempotency_key = "connector-boundary";
    request.user_input = "synthetic-only";
    request.application.context_id = "synthetic-input";
    request.application.sensitive_input = true;
    request.capabilities = {"protocol.negotiation", "errors.structured"};
    owo::plugin::PluginMessage invocation;
    invocation.service = std::string(owo::agent::kAgentPluginService);
    invocation.payload = owo::agent::encode_request(request);
    Settings settings;
    settings.pipe_name = LR"(\\.\pipe\OwO.NoServer.ConnectorBoundaryTest)";
    settings.timeout = std::chrono::milliseconds(500);
    settings.send_application_identity = false;
    const auto result = process_request(invocation, settings);
    const auto decoded = owo::agent::decode_response(result.payload);
    if (!decoded.ok || decoded.value.error_code != "sensitive_input") return 1;
    request.application.sensitive_input = false;
    invocation.payload = owo::agent::encode_request(request);
    settings.sensitive_filter_enabled = false;
    const auto disabled = owo::agent::decode_response(process_request(invocation, settings).payload);
    if (!disabled.ok || disabled.value.error_code != "filter_disabled") return 2;
    return 0;
}

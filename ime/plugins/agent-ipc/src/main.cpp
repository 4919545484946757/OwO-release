#include "owo/agent/agent_pipe.h"
#include "owo/agent/agent_auth.h"
#include "owo/agent/agent_protocol.h"
#include "owo/plugin/plugin_pipe.h"
#include "owo/plugin/plugin_protocol.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <charconv>
#include <cctype>
#include <filesystem>
#include <fstream>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

namespace {

struct Arguments {
    std::wstring pipe;
    std::string plugin_id;
    std::filesystem::path data;
};

struct Settings {
    std::wstring pipe_name{LR"(\\.\pipe\OwO.Agent.External.v1)"};
    std::chrono::milliseconds timeout{10000};
    std::size_t context_limit{32768};
    std::size_t context_entry_limit{8192};
    std::size_t context_entry_count{16};
    bool send_application_identity{true};
    bool send_session_context{true};
    bool sensitive_filter_enabled{true};
    bool require_pairing{false};
    std::filesystem::path pairing_credential_file;
};

std::string utf8(const std::wstring_view value) {
    if (value.empty()) return {};
    const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
        static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    if (size <= 0) return {};
    std::string result(static_cast<std::size_t>(size), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
        static_cast<int>(value.size()), result.data(), size, nullptr, nullptr) != size) return {};
    return result;
}

std::wstring wide(const std::string_view value) {
    if (value.empty()) return {};
    const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
        static_cast<int>(value.size()), nullptr, 0);
    if (size <= 0) return {};
    std::wstring result(static_cast<std::size_t>(size), L'\0');
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, value.data(),
        static_cast<int>(value.size()), result.data(), size) != size) return {};
    return result;
}

bool parse_arguments(const int argc, wchar_t** argv, Arguments& output) {
    if (argc != 7) return false;
    for (int index = 1; index + 1 < argc; index += 2) {
        const std::wstring_view key(argv[index]);
        if (key == L"--owo-plugin-pipe") output.pipe = argv[index + 1];
        else if (key == L"--owo-plugin-id") output.plugin_id = utf8(argv[index + 1]);
        else if (key == L"--owo-plugin-data") output.data = argv[index + 1];
        else return false;
    }
    return !output.pipe.empty() && output.plugin_id == owo::agent::kAgentPluginId &&
           output.data.is_absolute();
}

std::optional<std::string> read_file(const std::filesystem::path& path) {
    std::error_code error;
    const auto size = std::filesystem::file_size(path, error);
    if (error || size > 256 * 1024) return std::nullopt;
    std::ifstream input(path, std::ios::binary);
    if (!input) return std::nullopt;
    std::string result(static_cast<std::size_t>(size), '\0');
    input.read(result.data(), static_cast<std::streamsize>(result.size()));
    return input || result.empty() ? std::optional<std::string>(std::move(result))
                                   : std::nullopt;
}

std::optional<std::string> json_string(const std::string_view json,
                                       const std::string_view field) {
    const auto key = "\"" + std::string(field) + "\"";
    auto offset = json.find(key);
    if (offset == std::string_view::npos) return std::nullopt;
    offset = json.find(':', offset + key.size());
    if (offset == std::string_view::npos) return std::nullopt;
    offset = json.find('"', offset + 1);
    if (offset == std::string_view::npos) return std::nullopt;
    std::string output;
    for (++offset; offset < json.size(); ++offset) {
        const char byte = json[offset];
        if (byte == '"') return output;
        if (byte == '\\') {
            if (++offset >= json.size()) return std::nullopt;
            const char escaped = json[offset];
            if (escaped == '"' || escaped == '\\' || escaped == '/') output.push_back(escaped);
            else return std::nullopt;
        } else if (static_cast<unsigned char>(byte) < 0x20) return std::nullopt;
        else output.push_back(byte);
    }
    return std::nullopt;
}

std::optional<std::uint32_t> json_uint(const std::string_view json,
                                       const std::string_view field) {
    const auto key = "\"" + std::string(field) + "\"";
    auto offset = json.find(key);
    if (offset == std::string_view::npos ||
        (offset = json.find(':', offset + key.size())) == std::string_view::npos)
        return std::nullopt;
    ++offset;
    while (offset < json.size() && std::isspace(static_cast<unsigned char>(json[offset]))) ++offset;
    std::uint32_t result{};
    const auto parsed = std::from_chars(json.data() + offset, json.data() + json.size(), result);
    return parsed.ec == std::errc{} && parsed.ptr != json.data() + offset
        ? std::optional<std::uint32_t>(result) : std::nullopt;
}

std::optional<bool> json_bool(const std::string_view json, const std::string_view field) {
    const auto key = "\"" + std::string(field) + "\"";
    auto offset = json.find(key);
    if (offset == std::string_view::npos ||
        (offset = json.find(':', offset + key.size())) == std::string_view::npos)
        return std::nullopt;
    ++offset;
    while (offset < json.size() && std::isspace(static_cast<unsigned char>(json[offset]))) ++offset;
    if (json.substr(offset, 4) == "true") return true;
    if (json.substr(offset, 5) == "false") return false;
    return std::nullopt;
}

Settings load_settings(const std::filesystem::path& data) {
    Settings result;
    const auto json = read_file(data / L"settings.json");
    if (!json) return result;
    if (const auto value = json_string(*json, "pipe_name")) {
        const auto converted = wide(*value);
        if (owo::agent::valid_agent_pipe_name(converted)) result.pipe_name = converted;
    }
    if (const auto value = json_uint(*json, "timeout_ms"); value && *value >= 500 && *value <= 30000)
        result.timeout = std::chrono::milliseconds(*value);
    if (const auto value = json_uint(*json, "context_limit"); value && *value <= 131072)
        result.context_limit = *value;
    if (const auto value = json_uint(*json, "context_entry_limit");
        value && *value >= 256 && *value <= 32768)
        result.context_entry_limit = *value;
    if (const auto value = json_uint(*json, "context_entry_count");
        value && *value >= 1 && *value <= owo::agent::kMaximumAgentContextEntries)
        result.context_entry_count = *value;
    if (const auto value = json_bool(*json, "send_application_identity"))
        result.send_application_identity = *value;
    if (const auto value = json_bool(*json, "send_session_context"))
        result.send_session_context = *value;
    if (const auto value = json_bool(*json, "sensitive_filter_enabled"))
        result.sensitive_filter_enabled = *value;
    if (const auto value = json_bool(*json, "require_pairing")) result.require_pairing = *value;
    if (const auto value = json_string(*json, "pairing_credential_file"); value && !value->empty()) {
        result.require_pairing = true;
        result.pairing_credential_file = std::filesystem::path(wide(*value));
    }
    return result;
}

owo::agent::PipeExchangeResult exchange_configured_agent(const Settings& settings,
        const std::string_view payload, const std::chrono::milliseconds timeout) {
    if (!settings.require_pairing) return owo::agent::exchange_agent_pipe(settings.pipe_name, payload, timeout);
    const auto proven = owo::agent::authenticate_agent_request(payload, settings.pairing_credential_file);
    if (!proven.ok) return {false, {}, "pairing unavailable"};
    return owo::agent::exchange_agent_pipe(settings.pipe_name, proven.payload, timeout);
}

owo::agent::Response error_response(const owo::agent::Request& request,
                                    owo::agent::Status status,
                                    std::string code, std::string message) {
    owo::agent::Response response{
        request.session_id, message, {}, {}, status, 0, false, code};
    response.request_id = request.request_id;
    response.error_message = std::move(message);
    response.retryable = status == owo::agent::Status::disconnected ||
                         status == owo::agent::Status::timeout;
    response.can_continue_input = true;
    response.capabilities = {"protocol.negotiation", "input.structured",
                             "context.entries", "context.privacy",
                             "commands.risk", "errors.structured", "state.progress",
                             "tasks.slots", "tasks.incremental"};
    return response;
}

owo::plugin::PluginMessage invoke_response(const owo::plugin::PluginMessage& request,
                                           const owo::plugin::PluginStatus status,
                                           std::string payload, std::string diagnostic = {}) {
    owo::plugin::PluginMessage output;
    output.type = owo::plugin::PluginMessageType::invoke_response;
    output.status = status;
    output.request_id = request.request_id;
    output.plugin_id = request.plugin_id;
    output.payload = std::move(payload);
    output.diagnostic = std::move(diagnostic);
    return output;
}

owo::plugin::PluginMessage process_request(const owo::plugin::PluginMessage& invocation,
                                           const Settings& settings) {
    if (invocation.service != owo::agent::kAgentPluginService)
        return invoke_response(invocation, owo::plugin::PluginStatus::invalid_request, {},
                               "unsupported Agent service");
    const auto decoded = owo::agent::decode_request(invocation.payload);
    if (!decoded.ok)
        return invoke_response(invocation, owo::plugin::PluginStatus::invalid_request, {},
                               "invalid structured Agent request");
    auto request = decoded.value;
    if (request.application.sensitive_input && request.action != owo::agent::Action::cancel) {
        const auto response = error_response(request, owo::agent::Status::error,
            "sensitive_input", "敏感或未知输入框不可调用 Agent");
        return invoke_response(invocation, owo::plugin::PluginStatus::success,
                               owo::agent::encode_response(response));
    }
    if (!settings.sensitive_filter_enabled) {
        const auto response = error_response(request, owo::agent::Status::error,
            "filter_disabled", "敏感信息过滤已关闭，插件拒绝发送请求");
        return invoke_response(invocation, owo::plugin::PluginStatus::success,
                               owo::agent::encode_response(response));
    }
    const auto filtered_input = owo::agent::filter_sensitive_text(request.user_input, 4096);
    if (!filtered_input.ok) {
        const auto response = error_response(request, owo::agent::Status::error,
            "filter_failed", "敏感信息过滤失败，请求未发送");
        return invoke_response(invocation, owo::plugin::PluginStatus::success,
                               owo::agent::encode_response(response));
    }
    request.user_input = filtered_input.value;
    const auto filter_input_field = [&](std::string& value, const std::size_t limit) {
        const auto filtered = owo::agent::filter_sensitive_text(value, limit);
        if (!filtered.ok) return false;
        value = filtered.value;
        return true;
    };
    if (!filter_input_field(request.input_details.raw_pinyin, 4096) ||
        !filter_input_field(request.input_details.segmented_pinyin, 8192) ||
        !filter_input_field(request.input_details.selected_text, 16384) ||
        !filter_input_field(request.input_details.pending_pinyin, 4096) ||
        !filter_input_field(request.input_details.natural_language, 16384)) {
        const auto response = error_response(request, owo::agent::Status::error,
            "filter_failed", "结构化输入过滤失败，请求未发送");
        return invoke_response(invocation, owo::plugin::PluginStatus::success,
                               owo::agent::encode_response(response));
    }
    if (request.selection) {
        const auto filtered = owo::agent::filter_sensitive_text(
            request.selection->text, request.selection->text.size());
        if (!filtered.ok) {
            const auto response = error_response(request, owo::agent::Status::error,
                "filter_failed", "选区过滤失败，请求未发送");
            return invoke_response(invocation, owo::plugin::PluginStatus::success,
                                   owo::agent::encode_response(response));
        }
        request.selection->text = filtered.value;
        request.selection->redacted = request.selection->redacted || filtered.redacted;
    }
    for (auto& update : request.slot_updates) {
        if (!filter_input_field(update.value, 4096)) {
            const auto response = error_response(request, owo::agent::Status::error,
                "filter_failed", "任务槽位过滤失败，请求未发送");
            return invoke_response(invocation, owo::plugin::PluginStatus::success,
                                   owo::agent::encode_response(response));
        }
    }
    if (request.application.sensitive_input || !settings.send_session_context) {
        request.session_context.clear();
        request.context_entries.clear();
    } else {
        const auto context = owo::agent::filter_sensitive_text(
            request.session_context,
            std::min(settings.context_entry_limit, settings.context_limit));
        if (!context.ok) {
            const auto response = error_response(request, owo::agent::Status::error,
                "filter_failed", "局部上下文过滤失败，请求未发送");
            return invoke_response(invocation, owo::plugin::PluginStatus::success,
                                   owo::agent::encode_response(response));
        }
        request.session_context = context.value;
        std::vector<owo::agent::ContextEntry> eligible_entries;
        for (const auto& entry : request.context_entries) {
            if (entry.application.sensitive_input) continue;
            if (!request.application.context_id.empty() &&
                entry.context_id == request.application.context_id) continue;
            const auto filtered = owo::agent::filter_sensitive_text(
                entry.text, settings.context_entry_limit);
            if (!filtered.ok) {
                const auto response = error_response(
                    request, owo::agent::Status::error,
                    "filter_failed", "分段上下文过滤失败，请求未发送");
                return invoke_response(invocation, owo::plugin::PluginStatus::success,
                                       owo::agent::encode_response(response));
            }
            if (filtered.value.empty()) continue;
            auto retained = entry;
            retained.text = filtered.value;
            retained.filtered = true;
            retained.redacted = filtered.redacted;
            retained.truncated = entry.text.size() > settings.context_entry_limit;
            eligible_entries.push_back(std::move(retained));
        }
        std::stable_sort(eligible_entries.begin(), eligible_entries.end(),
            [](const auto& left, const auto& right) {
                if (left.relevance_milli != right.relevance_milli)
                    return left.relevance_milli > right.relevance_milli;
                return left.sequence > right.sequence;
            });
        std::vector<owo::agent::ContextEntry> selected_entries;
        std::size_t total_bytes = request.session_context.size();
        for (auto& entry : eligible_entries) {
            if (selected_entries.size() >= settings.context_entry_count) break;
            if (total_bytes + entry.text.size() > settings.context_limit) continue;
            const bool duplicate = std::any_of(
                selected_entries.begin(), selected_entries.end(), [&](const auto& selected) {
                    return selected.text == entry.text ||
                           selected.text.find(entry.text) != std::string::npos ||
                           entry.text.find(selected.text) != std::string::npos;
                });
            if (duplicate) continue;
            total_bytes += entry.text.size();
            selected_entries.push_back(std::move(entry));
        }
        std::stable_sort(selected_entries.begin(), selected_entries.end(),
            [](const auto& left, const auto& right) { return left.sequence < right.sequence; });
        request.context_entries = std::move(selected_entries);
    }
    if (!settings.send_application_identity) {
        const auto current_id = request.application.context_id;
        request.application = {};
        request.application.context_id = current_id;
        for (auto& entry : request.context_entries) {
            const auto context_id = entry.context_id;
            entry.application = {};
            entry.application.context_id = context_id;
        }
    }
    if (request.application.sensitive_input) {
        request.application.executable.clear();
        request.application.window_class.clear();
        request.application.focus_window_class.clear();
    }
    const auto payload = request.schema_version == 2
        ? owo::agent::encode_request_v2(request)
        : owo::agent::encode_request(request);
    if (payload.empty())
        return invoke_response(invocation, owo::plugin::PluginStatus::invalid_request, {},
                               "filtered Agent request is invalid");
    const auto exchanged = exchange_configured_agent(settings, payload, settings.timeout);
    if (!exchanged.ok) {
        if (exchanged.diagnostic == "pairing unavailable") {
            const auto response = error_response(request, owo::agent::Status::error,
                "pairing_unavailable", "Agent 配对不可用，请重新配对");
            return invoke_response(invocation, owo::plugin::PluginStatus::success,
                                   owo::agent::encode_response(response));
        }
        const auto response = error_response(request,
            owo::agent::Status::disconnected, "agent_disconnected", "第三方 Agent 未连接");
        return invoke_response(invocation, owo::plugin::PluginStatus::success,
                               owo::agent::encode_response(response));
    }
    const auto response = owo::agent::decode_response(exchanged.response);
    if (!response.ok || response.value.session_id != request.session_id ||
        (response.value.schema_version >= 3 && !response.value.request_id.empty() &&
         response.value.request_id != request.request_id))
        return invoke_response(invocation, owo::plugin::PluginStatus::plugin_error, {},
                               "third-party Agent returned invalid or mismatched data");
    if (request.schema_version >= 3) {
        const bool unsupported = std::any_of(
            response.value.capabilities.begin(), response.value.capabilities.end(),
            [&](const auto& capability) {
                return std::find(request.capabilities.begin(), request.capabilities.end(),
                                 capability) == request.capabilities.end();
            });
        const bool missing_required = std::any_of(
            request.required_features.begin(), request.required_features.end(),
            [&](const auto& required) {
                return std::find(response.value.capabilities.begin(),
                                 response.value.capabilities.end(), required) ==
                       response.value.capabilities.end();
            });
        if (unsupported || missing_required)
            return invoke_response(invocation, owo::plugin::PluginStatus::plugin_error, {},
                                   "third-party Agent capability negotiation failed");
    }
    return invoke_response(invocation, owo::plugin::PluginStatus::success,
                           owo::agent::encode_response(response.value));
}

int run(const Arguments& arguments) {
    wchar_t full_trust[8]{};
    if (GetEnvironmentVariableW(L"OWO_PLUGIN_FULL_TRUST", full_trust,
                                static_cast<DWORD>(std::size(full_trust))) != 1 ||
        full_trust[0] != L'1') return 10;
    auto connected = owo::plugin::connect_plugin_pipe_client(
        arguments.pipe, std::chrono::seconds(5));
    if (!connected) return 20;
    owo::plugin::PluginMessage hello;
    hello.type = owo::plugin::PluginMessageType::hello_request;
    hello.status = owo::plugin::PluginStatus::success;
    hello.request_id = 1;
    hello.plugin_id = arguments.plugin_id;
    if (!owo::plugin::send_plugin_pipe_message(connected.pipe, hello,
                                                std::chrono::seconds(5)).ok) return 21;
    const auto hello_response = owo::plugin::receive_plugin_pipe_message(
        connected.pipe, std::chrono::seconds(5));
    if (!hello_response.status.ok ||
        hello_response.message.type != owo::plugin::PluginMessageType::hello_response ||
        hello_response.message.request_id != 1) return 22;

    std::mutex send_mutex;
    std::jthread worker;
    std::atomic_bool worker_finished{false};
    std::uint64_t invocation_id = 0;
    std::optional<owo::agent::Request> active_agent_request;
    const auto send = [&](const owo::plugin::PluginMessage& message) {
        std::lock_guard lock(send_mutex);
        return owo::plugin::send_plugin_pipe_message(
            connected.pipe, message, std::chrono::seconds(5)).ok;
    };
    const auto reap = [&] {
        if (!worker_finished.load(std::memory_order_acquire)) return;
        if (worker.joinable()) worker.join();
        worker_finished.store(false, std::memory_order_release);
        invocation_id = 0;
        active_agent_request.reset();
    };
    for (;;) {
        reap();
        const auto received = owo::plugin::receive_plugin_pipe_message(
            connected.pipe, std::chrono::hours(24));
        if (!received.status.ok) return 23;
        const auto& message = received.message;
        if (message.plugin_id != arguments.plugin_id) return 24;
        if (message.type == owo::plugin::PluginMessageType::shutdown_request) {
            owo::plugin::PluginMessage acknowledgement;
            acknowledgement.type = owo::plugin::PluginMessageType::acknowledgement;
            acknowledgement.status = owo::plugin::PluginStatus::success;
            acknowledgement.request_id = message.request_id;
            acknowledgement.plugin_id = arguments.plugin_id;
            send(acknowledgement);
            return 0;
        }
        if (message.type == owo::plugin::PluginMessageType::cancel_request) {
            if (message.target_request_id != invocation_id) return 25;
            owo::plugin::PluginMessage acknowledgement;
            acknowledgement.type = owo::plugin::PluginMessageType::acknowledgement;
            acknowledgement.status = owo::plugin::PluginStatus::success;
            acknowledgement.request_id = message.request_id;
            acknowledgement.plugin_id = arguments.plugin_id;
            if (!send(acknowledgement)) return 26;
            if (active_agent_request) {
                auto cancel = *active_agent_request;
                cancel.action = owo::agent::Action::cancel;
                cancel.user_input.clear();
                cancel.input_details = {};
                cancel.selection.reset();
                cancel.session_context.clear();
                cancel.context_entries.clear();
                cancel.slot_updates.clear();
                cancel.command_id.clear();
                cancel.parent_request_id = cancel.request_id;
                cancel.request_id += ".cancel";
                cancel.idempotency_key = cancel.request_id;
                const auto settings = load_settings(arguments.data);
                const auto payload = cancel.schema_version == 2
                    ? owo::agent::encode_request_v2(cancel)
                    : owo::agent::encode_request(cancel);
                if (!payload.empty())
                    static_cast<void>(exchange_configured_agent(settings, payload, std::chrono::milliseconds(500)));
            }
            continue;
        }
        if (message.type != owo::plugin::PluginMessageType::invoke_request) return 27;
        reap();
        if (invocation_id != 0) {
            if (!send(invoke_response(message, owo::plugin::PluginStatus::plugin_error, {},
                                      "Agent invocation is already active"))) return 28;
            continue;
        }
        const auto decoded = owo::agent::decode_request(message.payload);
        if (decoded.ok) active_agent_request = decoded.value;
        invocation_id = message.request_id;
        const auto invocation = message;
        const auto settings = load_settings(arguments.data);
        worker_finished.store(false, std::memory_order_release);
        worker = std::jthread([&, invocation, settings] {
            send(process_request(invocation, settings));
            worker_finished.store(true, std::memory_order_release);
        });
    }
}

}  // namespace

int wmain(const int argc, wchar_t** argv) {
    Arguments arguments;
    if (!parse_arguments(argc, argv, arguments)) return 1;
    return run(arguments);
}

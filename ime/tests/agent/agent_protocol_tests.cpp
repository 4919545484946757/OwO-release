#include "owo/agent/agent_pipe.h"
#include "owo/agent/agent_protocol.h"

#include <string>

int main() {
    const std::string session = "0123456789abcdef0123456789abcdef";
    owo::agent::Request request;
    request.action = owo::agent::Action::submit;
    request.session_id = session;
    request.user_input = "bangwozhaowenjian";
    request.application = {42, 84, "notepad.exe", "Notepad", "Edit",
                           "ctx-notepad-1", false};
    request.session_context = "必要上下文";
    request.input_details = {"ceshi", "ce'shi", "测试", "", "测试", "agent", true};
    request.request_id = "req-0001";
    request.idempotency_key = "idem-0001";
    request.capabilities = {"context.entries", "input.structured", "commands.risk"};
    request.required_features = {"input.structured", "commands.risk"};
    request.context_entries = {
        {"ctx-editor-1",
         {43, 85, "editor.exe", "EditorWindow", "TextBox", "ctx-editor-1", false},
         "前一个应用中的上下文", 1},
        {"ctx-notepad-1", request.application, "当前输入位置的上下文", 2},
    };
    const auto encoded_request = owo::agent::encode_request(request);
    const auto decoded_request = owo::agent::decode_request(encoded_request);
    if (!decoded_request.ok || decoded_request.value != request) return 1;
    auto missing_request_id = request;
    missing_request_id.request_id.clear();
    if (!owo::agent::encode_request(missing_request_id).empty()) return 18;
    auto duplicate_capability = request;
    duplicate_capability.capabilities.push_back("context.entries");
    if (!owo::agent::encode_request(duplicate_capability).empty()) return 19;
    auto missing_required_capability = request;
    missing_required_capability.required_features.push_back("errors.structured");
    if (!owo::agent::encode_request(missing_required_capability).empty()) return 21;
    const auto decoded_v2_request = owo::agent::decode_request(
        owo::agent::encode_request_v2(request));
    if (!decoded_v2_request.ok || decoded_v2_request.value.schema_version != 2 ||
        decoded_v2_request.value.user_input != request.user_input) return 16;
    if (owo::agent::decode_request(encoded_request.substr(0, encoded_request.size() - 1)).ok)
        return 2;
    auto unknown = encoded_request;
    unknown.insert(unknown.size() - 1, ",\"unknown\":false");
    if (owo::agent::decode_request(unknown).ok) return 3;
    auto mismatched = request;
    mismatched.context_entries.front().application.context_id = "ctx-other";
    if (!owo::agent::encode_request(mismatched).empty()) return 13;

    owo::agent::Response response;
    response.session_id = session;
    response.message = "请选择操作";
    response.commands = {
        {"open-folder", "打开文件夹", false, "显示结果所在目录", "file.open",
         "low", false, "不会修改文件", true, ""},
        {"delete-file", "删除文件", true, "删除选定文件", "file.delete",
         "high", true, "需要在第三方 Agent 中确认", true, ""}};
    response.status = owo::agent::Status::agent_mode;
    response.page = 2;
    response.has_more = true;
    response.request_id = request.request_id;
    response.state_revision = 2;
    response.progress = 20;
    response.can_cancel = true;
    response.capabilities = {"context.entries", "commands.risk"};
    response.task.intent = "create_reminder";
    response.task.intent_ranges = {{2, 3}};
    response.task.slots = {
        {"time", "时间", "三点", {{0, 2}}, false, 700},
    };
    response.task.unconsumed_ranges = {{3, 5}};
    response.task.revision = 4;
    response.commands.front().task_revision = 4;
    response.commands.front().slot_updates = {
        {"time", "今天三点", {{0, 2}}, true},
    };
    const auto encoded_response = owo::agent::encode_response(response);
    const auto decoded_response = owo::agent::decode_response(encoded_response);
    if (!decoded_response.ok || decoded_response.value != response) return 4;
    auto selection = request;
    selection.action = owo::agent::Action::select;
    selection.command_id = "open-folder";
    selection.task_revision = 4;
    selection.slot_updates = response.commands.front().slot_updates;
    selection.request_id = "req-0002";
    selection.parent_request_id = request.request_id;
    selection.idempotency_key = "idem-0002";
    const auto decoded_selection = owo::agent::decode_request(
        owo::agent::encode_request(selection));
    if (!decoded_selection.ok || decoded_selection.value != selection) return 22;
    auto overlapping_task = response;
    overlapping_task.task.unconsumed_ranges = {{1, 5}};
    if (!owo::agent::encode_response(overlapping_task).empty()) return 23;
    auto duplicate_slot_patch = selection;
    duplicate_slot_patch.slot_updates.push_back(selection.slot_updates.front());
    if (!owo::agent::encode_request(duplicate_slot_patch).empty()) return 24;
    auto unsafe_command = response;
    unsafe_command.commands.front().risk_level = "high";
    if (!owo::agent::encode_response(unsafe_command).empty()) return 20;
    const auto decoded_v2_response = owo::agent::decode_response(
        owo::agent::encode_response_v2(response));
    if (!decoded_v2_response.ok || decoded_v2_response.value.schema_version != 2 ||
        decoded_v2_response.value.commands.size() != 2) return 17;
    if (owo::agent::decode_response("{}").ok) return 5;
    response.commands.push_back({"open-folder", "重复", false});
    if (owo::agent::decode_response(owo::agent::encode_response(response)).ok) return 12;
    response.commands.pop_back();
    auto escaped_response = encoded_response;
    const auto message_begin = escaped_response.find("请选择操作");
    if (message_begin == std::string::npos) return 9;
    escaped_response.replace(message_begin, std::string("请选择操作").size(),
                             "\\u8bf7\\u9009\\u62e9\\u64cd\\u4f5c");
    const auto decoded_escaped = owo::agent::decode_response(escaped_response);
    if (!decoded_escaped.ok || decoded_escaped.value.message != "请选择操作") return 10;
    auto invalid_surrogate = escaped_response;
    const auto unicode = invalid_surrogate.find("\\u8bf7");
    invalid_surrogate.replace(unicode, 6, "\\ud800");
    if (owo::agent::decode_response(invalid_surrogate).ok) return 11;

    const auto filtered = owo::agent::filter_sensitive_text(
        "password=hunter2 token:abc123 card=4111111111111111", 4096);
    if (!filtered.ok || !filtered.redacted ||
        filtered.value.find("hunter2") != std::string::npos ||
        filtered.value.find("abc123") != std::string::npos ||
        filtered.value.find("4111111111111111") != std::string::npos) return 6;
    if (owo::agent::filter_sensitive_text(std::string("\xff", 1)).ok) return 7;
    const auto utf8_boundary = owo::agent::filter_sensitive_text("中文", 2);
    if (!utf8_boundary.ok || !utf8_boundary.redacted || !utf8_boundary.value.empty()) return 14;
    auto too_many_contexts = request;
    too_many_contexts.context_entries.clear();
    for (std::size_t index = 0;
         index <= owo::agent::kMaximumAgentContextEntries; ++index) {
        const auto context_id = "ctx-limit-" + std::to_string(index);
        auto application = request.application;
        application.context_id = context_id;
        too_many_contexts.context_entries.push_back(
            {context_id, application, "x", static_cast<std::uint64_t>(index)});
    }
    if (!owo::agent::encode_request(too_many_contexts).empty()) return 15;
    if (!owo::agent::valid_agent_pipe_name(LR"(\\.\pipe\OwO.Agent.External.v1)") ||
        owo::agent::valid_agent_pipe_name(LR"(\\server\pipe\agent)") ||
        owo::agent::valid_agent_pipe_name(LR"(\\.\pipe\..\agent)")) return 8;
    return 0;
}

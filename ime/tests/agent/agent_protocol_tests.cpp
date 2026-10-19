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
    auto document_selection = request;
    document_selection.capabilities.push_back(owo::agent::kAgentCapabilityInputSelection);
    document_selection.selection = owo::agent::SelectionSummary{"合成选区", std::string(64, 'a'), false};
    const auto selection_wire = owo::agent::encode_request(document_selection);
    const auto selection_roundtrip = owo::agent::decode_request(selection_wire);
    if (!selection_roundtrip.ok || selection_roundtrip.value != document_selection) return 80;
    if (!owo::agent::encode_request_v2(document_selection).empty()) return 81;
    const auto poll = owo::agent::make_poll_request(document_selection, 1);
    const auto polled = owo::agent::decode_request(owo::agent::encode_request(poll));
    if (!polled.ok || polled.value.application != document_selection.application ||
        polled.value.selection != document_selection.selection ||
        polled.value.capabilities != document_selection.capabilities ||
        polled.value.action != owo::agent::Action::poll ||
        polled.value.parent_request_id != document_selection.request_id ||
        polled.value.request_id == document_selection.request_id ||
        !polled.value.user_input.empty() || !polled.value.session_context.empty() ||
        !polled.value.context_entries.empty()) return 90;
    if (owo::agent::make_poll_request(document_selection, 2).idempotency_key ==
        poll.idempotency_key) return 91;

    document_selection.application.sensitive_input = true;
    if (!owo::agent::encode_request(document_selection).empty()) return 82;
    document_selection.application.sensitive_input = false;
    document_selection.selection->text = std::string(1001, 'a');
    if (!owo::agent::encode_request(document_selection).empty()) return 83;
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
    const auto chinese = owo::agent::filter_sensitive_text(
        "我的密码：Abc#123，验证码654321。手机13812345678，"
        "身份证11010519491231002X，邮箱 someone.name@example.com 卡号 6222020200112233446", 4096);
    if (!chinese.ok || !chinese.redacted ||
        chinese.value.find("Abc#123") != std::string::npos ||
        chinese.value.find("654321") != std::string::npos ||
        chinese.value.find("13812345678") != std::string::npos ||
        chinese.value.find("11010519491231002X") != std::string::npos ||
        chinese.value.find("someone.name@example.com") != std::string::npos ||
        chinese.value.find("6222020200112233446") != std::string::npos ||
        chinese.value.find("[PHONE_REDACTED]") == std::string::npos ||
        chinese.value.find("[ID_REDACTED]") == std::string::npos ||
        chinese.value.find("[EMAIL_REDACTED]") == std::string::npos) return 22;
    const auto prose = owo::agent::filter_sensitive_text(
        "忘记密码怎么办？订单号 20261016123456789 和版本 1.2.3 不应被改写", 4096);
    if (!prose.ok || prose.redacted ||
        prose.value != "忘记密码怎么办？订单号 20261016123456789 和版本 1.2.3 不应被改写") return 23;
    const auto bad_checksum = owo::agent::filter_sensitive_text("编号110105194912310021", 4096);
    if (!bad_checksum.ok || bad_checksum.value.find("[ID_REDACTED]") != std::string::npos) return 24;
    {
        owo::agent::ApplicationContext current{1, 2, "notepad.exe", "Notepad", "Edit", "ctx-a", false};
        owo::agent::ContextEntry same_position{"ctx-a", current, "x", 1};
        owo::agent::ContextEntry same_app{"ctx-b", current, "x", 2};
        same_app.application.context_id = "ctx-b";
        owo::agent::ContextEntry other_app{"ctx-c", {3, 4, "chat.exe", "Chat", "Edit", "ctx-c", false}, "x", 3};
        if (!owo::agent::context_entry_shareable(same_position, current) ||
            !owo::agent::context_entry_shareable(same_app, current) ||
            owo::agent::context_entry_shareable(other_app, current)) return 25;
        auto sensitive = current;
        sensitive.sensitive_input = true;
        if (owo::agent::context_entry_shareable(same_position, sensitive)) return 26;
        auto anonymous = current;
        anonymous.executable.clear();
        anonymous.context_id = "ctx-z";
        if (owo::agent::context_entry_shareable(same_app, anonymous)) return 27;
    }
    {
        // v4 command.effect: present only when output.effects is negotiated.
        auto effects = response;
        effects.capabilities.push_back(owo::agent::kAgentCapabilityOutputEffects);
        effects.commands.front().effect = owo::agent::CommandEffect{
            owo::agent::EffectKind::commit_text, "Hello \"world\"", "ctx-1", ""};
        const auto encoded = owo::agent::encode_response(effects);
        const auto decoded = owo::agent::decode_response(encoded);
        if (!decoded.ok || decoded.value != effects) return 30;
        if (owo::agent::encode_response(response).find("\"effect\"") != std::string::npos)
            return 31;
        auto unnegotiated = effects;
        unnegotiated.capabilities.pop_back();
        if (!owo::agent::encode_response(unnegotiated).empty()) return 32;
        auto replace = effects;
        replace.commands.front().effect->kind = owo::agent::EffectKind::replace_selection;
        if (!owo::agent::encode_response(replace).empty()) return 33;
        replace.commands.front().effect->selection_sha256 = std::string(64, 'a');
        if (owo::agent::encode_response(replace).empty()) return 33;
        replace.commands.front().effect->target_context_id.clear();
        if (!owo::agent::encode_response(replace).empty()) return 33;
        auto extra = encoded;
        const auto digest = extra.find("\"selection_sha256\":\"\"");
        if (digest == std::string::npos) return 34;
        extra.insert(digest, "\"extra\":1,");
        if (owo::agent::decode_response(extra).ok) return 34;
        auto bad_kind = encoded;
        const auto kind = bad_kind.find("commit_text");
        bad_kind.replace(kind, 11, "run_program");
        if (owo::agent::decode_response(bad_kind).ok) return 34;

        using owo::agent::EffectDecision;
        const auto& commit = *effects.commands.front().effect;
        owo::agent::EffectTargetFacts facts{true, true, true, "ctx-1", ""};
        if (owo::agent::decide_effect(commit, facts) != EffectDecision::write) return 35;
        auto moved = facts;
        moved.current_context_id = "ctx-2";
        if (owo::agent::decide_effect(commit, moved) != EffectDecision::preview_only) return 35;
        auto unsafe = facts;
        unsafe.context_allowed = false;
        if (owo::agent::decide_effect(commit, unsafe) != EffectDecision::preview_only) return 35;
        auto other_document = facts;
        other_document.same_document = false;
        if (owo::agent::decide_effect(commit, other_document) != EffectDecision::preview_only)
            return 35;
        auto unbound = commit;
        unbound.target_context_id.clear();
        auto blank = facts;
        blank.current_context_id.clear();
        if (owo::agent::decide_effect(unbound, blank) != EffectDecision::preview_only) return 35;
        auto old_client = facts;
        old_client.capability_negotiated = false;
        if (owo::agent::decide_effect(commit, old_client) != EffectDecision::preview_only)
            return 35;
        owo::agent::CommandEffect selection_effect{
            owo::agent::EffectKind::replace_selection, "x", "ctx-1", std::string(64, 'b')};
        auto selected = facts;
        selected.selection_sha256 = std::string(64, 'b');
        if (owo::agent::decide_effect(selection_effect, selected) != EffectDecision::write)
            return 36;
        selected.selection_sha256 = std::string(64, 'c');
        if (owo::agent::decide_effect(selection_effect, selected) != EffectDecision::preview_only)
            return 36;
        owo::agent::CommandEffect copy{owo::agent::EffectKind::copy_text, "x", "", ""};
        if (owo::agent::decide_effect(copy, other_document) != EffectDecision::write) return 36;
    }
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

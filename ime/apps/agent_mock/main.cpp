#include "owo/agent/agent_protocol.h"
#include "owo/agent/agent_pipe.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <sddl.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <iostream>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace {

std::atomic_bool running{true};

BOOL WINAPI console_handler(const DWORD event) {
    if (event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT || event == CTRL_CLOSE_EVENT) {
        running.store(false, std::memory_order_release);
        return TRUE;
    }
    return FALSE;
}

class CurrentUserSecurity final {
public:
    CurrentUserSecurity() {
        HANDLE token = nullptr;
        if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return;
        DWORD size = 0;
        GetTokenInformation(token, TokenUser, nullptr, 0, &size);
        std::vector<std::byte> storage(size);
        if (size == 0 || !GetTokenInformation(token, TokenUser, storage.data(), size, &size)) {
            CloseHandle(token);
            return;
        }
        const auto* user = reinterpret_cast<const TOKEN_USER*>(storage.data());
        LPWSTR sid = nullptr;
        if (!ConvertSidToStringSidW(user->User.Sid, &sid)) {
            CloseHandle(token);
            return;
        }
        CloseHandle(token);
        const std::wstring sddl = L"D:P(A;;GA;;;" + std::wstring(sid) +
            L")(A;;GA;;;SY)(A;;GA;;;BA)S:(ML;;NW;;;LW)";
        LocalFree(sid);
        if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.c_str(), SDDL_REVISION_1, &descriptor_, nullptr)) return;
        attributes_ = {sizeof(attributes_), descriptor_, FALSE};
    }
    ~CurrentUserSecurity() { if (descriptor_ != nullptr) LocalFree(descriptor_); }
    CurrentUserSecurity(const CurrentUserSecurity&) = delete;
    CurrentUserSecurity& operator=(const CurrentUserSecurity&) = delete;
    explicit operator bool() const noexcept { return descriptor_ != nullptr; }
    SECURITY_ATTRIBUTES* get() noexcept { return descriptor_ ? &attributes_ : nullptr; }

private:
    PSECURITY_DESCRIPTOR descriptor_{};
    SECURITY_ATTRIBUTES attributes_{};
};

bool transfer_exact(const HANDLE pipe, void* data, const DWORD size, const bool write) {
    DWORD transferred = 0;
    const BOOL ok = write ? WriteFile(pipe, data, size, &transferred, nullptr)
                          : ReadFile(pipe, data, size, &transferred, nullptr);
    return ok != FALSE && transferred == size;
}

bool read_frame(const HANDLE pipe, std::string& output) {
    std::array<unsigned char, 4> header{};
    if (!transfer_exact(pipe, header.data(), static_cast<DWORD>(header.size()), false))
        return false;
    const auto size = static_cast<std::uint32_t>(header[0]) |
        (static_cast<std::uint32_t>(header[1]) << 8) |
        (static_cast<std::uint32_t>(header[2]) << 16) |
        (static_cast<std::uint32_t>(header[3]) << 24);
    if (size == 0 || size > owo::agent::kMaximumAgentPayloadBytes) return false;
    output.resize(size);
    return transfer_exact(pipe, output.data(), size, false);
}

bool write_frame(const HANDLE pipe, const std::string_view payload) {
    if (payload.empty() || payload.size() > owo::agent::kMaximumAgentPayloadBytes)
        return false;
    std::array<unsigned char, 4> header{
        static_cast<unsigned char>(payload.size()),
        static_cast<unsigned char>(payload.size() >> 8),
        static_cast<unsigned char>(payload.size() >> 16),
        static_cast<unsigned char>(payload.size() >> 24)};
    return transfer_exact(pipe, header.data(), static_cast<DWORD>(header.size()), true) &&
        transfer_exact(pipe, const_cast<char*>(payload.data()),
                       static_cast<DWORD>(payload.size()), true);
}

struct Session {
    owo::agent::Request last_request;
    bool executing{};
    std::uint64_t revision{};
    owo::agent::TaskDraft task;
};

std::optional<owo::agent::PinyinRange> pinyin_range(
    const std::string_view input, const std::string_view token) {
    const auto offset = input.find(token);
    if (offset == std::string_view::npos) return std::nullopt;
    return owo::agent::PinyinRange{static_cast<std::uint32_t>(offset),
        static_cast<std::uint32_t>(offset + token.size())};
}

owo::agent::Command slot_command(std::string id, std::string label,
                                 const std::uint64_t revision,
                                 owo::agent::SlotUpdate update) {
    owo::agent::Command command;
    command.id = std::move(id);
    command.label = std::move(label);
    command.description = "只更新对应任务槽位，不重新解析整条指令";
    command.category = "task.slot.update";
    command.risk_level = "none";
    command.preview = "更新任务草稿，不直接执行系统操作";
    command.task_revision = revision;
    command.slot_updates.push_back(std::move(update));
    return command;
}

owo::agent::Response page_response(const owo::agent::Request& request) {
    owo::agent::Response response;
    response.session_id = request.session_id;
    response.message = "模拟 Agent 已收到：" + request.user_input;
    response.status = owo::agent::Status::agent_mode;
    response.request_id = request.request_id;
    response.state_revision = 1;
    response.progress = 100;
    response.can_cancel = true;
    response.can_continue_input = true;
    response.capabilities = {"protocol.negotiation", "input.structured",
                             "context.entries", "context.privacy",
                             "commands.risk", "errors.structured", "state.progress",
                             "tasks.slots", "tasks.incremental"};
    response.page = request.page;
    const auto& raw = request.input_details.raw_pinyin;
    const auto time = pinyin_range(raw, "sanzhong");
    const auto intent = pinyin_range(raw, "tixingwo");
    const auto content = pinyin_range(raw, "kaihui");
    if (time && intent && content) {
        response.message = "已识别提醒任务，请确定时间";
        response.task.intent = "create_reminder";
        response.task.intent_ranges = {*intent};
        response.task.slots = {
            {"time", "时间", "三点钟", {*time}, false, 650},
            {"content", "事项", "开会", {*content}, true, 980},
        };
        response.task.revision = 1;
        response.commands = {
            slot_command("time-today-3", "今天3点提醒我开会", 1,
                         {"time", "今天3点", {*time}, true}),
            slot_command("time-tomorrow-3", "明天3点提醒我开会", 1,
                         {"time", "明天3点", {*time}, true}),
            slot_command("time-daily-3", "每天3点提醒我开会", 1,
                         {"time", "每天3点", {*time}, true}),
        };
        return response;
    }
    if (request.page == 0) {
        response.commands = {
            {"echo", "回显本次请求", false, "返回 Agent 实际收到的自然语言输入",
             "debug.echo", "none", false, "不会执行系统操作", true, ""},
            {"inspect-context", "显示收到的应用上下文", false,
             "显示分段上下文及其应用标识", "debug.context", "low", false,
             "只返回本次请求内已有信息", true, ""},
            {"simulate-execution", "模拟执行中状态", false,
             "模拟一个可取消的异步任务", "debug.execution", "low", false,
             "不会执行真实操作", true, ""},
            {"delete-demo", "模拟删除文件（高风险测试）", true,
             "验证高风险命令必须转到 Agent 可信界面确认", "file.delete", "high",
             true, "不会实际删除文件", true, ""},
        };
        response.has_more = true;
        const bool effects = std::find(request.capabilities.begin(), request.capabilities.end(),
                                       owo::agent::kAgentCapabilityOutputEffects) !=
                             request.capabilities.end();
        if (effects) {
            // v4 acceptance: write-back candidates bound to the requesting input position.
            response.capabilities.push_back(owo::agent::kAgentCapabilityOutputEffects);
            const auto effect_command = [&](std::string id, std::string label,
                                            owo::agent::EffectKind kind, std::string text,
                                            std::string target) {
                owo::agent::Command command;
                command.id = std::move(id);
                command.label = std::move(label);
                command.description = "v4 原位回写验收";
                command.category = "text.insert";
                command.risk_level = "low";
                command.preview = "选中后由输入法复核目标再写入";
                command.effect = owo::agent::CommandEffect{kind, std::move(text),
                                                           std::move(target), ""};
                return command;
            };
            const auto& current = request.application.context_id;
            response.commands.insert(response.commands.begin(), {
                effect_command("effect-commit", "回写验收：上屏测试文字",
                               owo::agent::EffectKind::commit_text,
                               "【OwO 回写验收】" + request.user_input, current),
                effect_command("effect-wrong-target", "回写验收：错投目标（应只预览）",
                               owo::agent::EffectKind::commit_text,
                               "【不应出现在文档里】", "ctx-wrong-target"),
                effect_command("effect-copy", "回写验收：复制到剪贴板",
                               owo::agent::EffectKind::copy_text,
                               "【OwO 剪贴板验收】" + request.user_input, ""),
            });
        }
    } else {
        response.commands = {
            {"finish", "模拟完成任务", false, "返回完成状态", "debug.finish",
             "none", false, "不会执行系统操作", true, ""},
            {"error-demo", "模拟 Agent 错误", false, "返回可识别的结构化错误",
             "debug.error", "none", false, "不会执行系统操作", true, ""},
        };
    }
    return response;
}

owo::agent::Response process(const owo::agent::Request& request,
                             std::unordered_map<std::string, Session>& sessions) {
    if (request.action == owo::agent::Action::cancel) {
        sessions.erase(request.session_id);
        return {request.session_id, "模拟会话已取消", {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.action == owo::agent::Action::submit) {
        auto response = page_response(request);
        sessions[request.session_id] = {request, false, 0, response.task};
        return response;
    }
    const auto found = sessions.find(request.session_id);
    if (found == sessions.end())
        return {request.session_id, "模拟 Agent 找不到该会话", {}, {},
                owo::agent::Status::error, 0, false, "session_not_found"};
    if (request.action == owo::agent::Action::page) {
        auto paged = found->second.last_request;
        paged.page = request.page;
        return page_response(paged);
    }
    if (request.action == owo::agent::Action::poll) {
        if (found->second.executing) {
            found->second.executing = false;
            return {request.session_id, "模拟操作已经完成", {}, {},
                    owo::agent::Status::completed, request.page, false, {}};
        }
        return {request.session_id, "模拟 Agent 当前没有执行中的任务", {}, {},
                owo::agent::Status::agent_mode, request.page, false, {}};
    }
    if (!request.slot_updates.empty()) {
        if (request.task_revision != found->second.task.revision) {
            auto response = page_response(found->second.last_request);
            response.status = owo::agent::Status::error;
            response.message = "任务版本不匹配，请刷新候选";
            response.error_code = "task_revision_mismatch";
            response.task = found->second.task;
            return response;
        }
        for (const auto& update : request.slot_updates) {
            auto slot = std::find_if(found->second.task.slots.begin(),
                found->second.task.slots.end(), [&](const auto& item) {
                    return item.id == update.slot_id;
                });
            if (slot == found->second.task.slots.end()) continue;
            if (slot->locked && slot->value != update.value) {
                auto response = page_response(found->second.last_request);
                response.status = owo::agent::Status::error;
                response.message = "任务槽位已经锁定";
                response.error_code = "slot_locked";
                response.task = found->second.task;
                return response;
            }
            slot->value = update.value;
            slot->source_ranges = update.consumed_ranges;
            slot->locked = update.lock;
            slot->confidence_milli = 1000;
        }
        ++found->second.task.revision;
        owo::agent::Response response;
        response.session_id = request.session_id;
        response.message = "时间槽位已锁定；任务草稿保持在输入法组合态";
        response.status = owo::agent::Status::agent_mode;
        response.task = found->second.task;
        response.capabilities = {"protocol.negotiation", "input.structured",
                                 "context.entries", "context.privacy", "commands.risk",
                                 "errors.structured", "state.progress", "tasks.slots",
                                 "tasks.incremental"};
        owo::agent::Command execute;
        execute.id = "execute-reminder";
        execute.label = "确认创建提醒";
        execute.description = "提交当前已锁定的任务草稿";
        execute.category = "reminder.create";
        execute.risk_level = "low";
        execute.preview = "由第三方 Agent 按自身权限规则继续确认或执行";
        execute.task_revision = found->second.task.revision;
        execute.commit_task = true;
        response.commands = {std::move(execute)};
        return response;
    }
    if (request.command_id == "execute-reminder") {
        return {request.session_id, "模拟提醒任务已完成", {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "echo") {
        return {request.session_id,
                "回显：" + found->second.last_request.user_input, {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "inspect-context") {
        const auto& source = found->second.last_request;
        const std::string message = "应用=" + source.application.executable +
            "，窗口类=" + source.application.window_class +
            "，焦点控件=" + source.application.focus_window_class +
            "，上下文标识=" + source.application.context_id +
            "，分段数=" + std::to_string(source.context_entries.size()) +
            "，局部上下文=" + source.session_context;
        return {request.session_id, message, {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "simulate-execution") {
        found->second.executing = true;
        return {request.session_id, "模拟 Agent 正在处理", {}, "模拟等待 1 次轮询",
                owo::agent::Status::executing, 0, false, {}};
    }
    if (request.command_id == "finish") {
        return {request.session_id, "模拟任务完成", {}, {},
                owo::agent::Status::completed, 0, false, {}};
    }
    if (request.command_id == "error-demo") {
        return {request.session_id, "这是模拟错误，不会执行任何操作", {}, {},
                owo::agent::Status::error, 0, false, "mock_error"};
    }
    return {request.session_id, "模拟 Agent 收到未知指令 ID", {}, {},
            owo::agent::Status::error, 0, false, "unknown_command"};
}

}  // namespace

int wmain(const int argc, wchar_t** argv) {
    std::wstring pipe_name = LR"(\\.\pipe\OwO.Agent.External.v1)";
    if (argc == 3 && std::wstring_view(argv[1]) == L"--pipe") pipe_name = argv[2];
    else if (argc != 1) {
        std::cerr << "usage: owo_agent_mock [--pipe \\\\.\\pipe\\Name]\n";
        return 1;
    }
    if (!owo::agent::valid_agent_pipe_name(pipe_name)) {
        std::cerr << "invalid local Agent pipe name\n";
        return 2;
    }
    SetConsoleOutputCP(CP_UTF8);
    SetConsoleCtrlHandler(console_handler, TRUE);
    CurrentUserSecurity security;
    if (!security) {
        std::cerr << "cannot create current-user pipe security descriptor\n";
        return 3;
    }
    std::unordered_map<std::string, Session> sessions;
    std::unordered_map<std::string, owo::agent::Response> idempotent_responses;
    std::cout << "OwO 模拟第三方 Agent 已启动。\n"
              << "监听本机管道；按 Ctrl+C 停止。\n\n";
    while (running.load(std::memory_order_acquire)) {
        const HANDLE pipe = CreateNamedPipeW(
            pipe_name.c_str(), PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT, 1,
            owo::agent::kMaximumAgentPayloadBytes + 4,
            owo::agent::kMaximumAgentPayloadBytes + 4, 1000, security.get());
        if (pipe == INVALID_HANDLE_VALUE) return 4;
        const bool connected = ConnectNamedPipe(pipe, nullptr) != FALSE ||
                               GetLastError() == ERROR_PIPE_CONNECTED;
        if (connected) {
            std::string payload;
            if (read_frame(pipe, payload)) {
                const auto decoded = owo::agent::decode_request(payload);
                if (decoded.ok) {
                    const auto& request = decoded.value;
                    std::cout << "[接收] action=" << owo::agent::action_name(request.action)
                              << " session=" << request.session_id
                              << " page=" << request.page << '\n'
                              << "       input=" << request.user_input << '\n'
                              << "       request_id=" << request.request_id
                              << " parent_request_id=" << request.parent_request_id
                              << " idempotency_key=" << request.idempotency_key << '\n'
                              << "       raw_pinyin=" << request.input_details.raw_pinyin
                              << " segmented_pinyin=" << request.input_details.segmented_pinyin
                              << " selected_text=" << request.input_details.selected_text
                              << " pending_pinyin=" << request.input_details.pending_pinyin << '\n'
                              << "       app=" << request.application.executable
                              << " pid=" << request.application.process_id
                              << " tid=" << request.application.thread_id
                              << " class=" << request.application.window_class
                              << " focus_class=" << request.application.focus_window_class
                              << " context_id=" << request.application.context_id << '\n'
                              << "       context=" << request.session_context
                              << " command_id=" << request.command_id
                              << " task_revision=" << request.task_revision << '\n'
                              << "       output.effects=" <<
                                     (std::find(request.capabilities.begin(),
                                                request.capabilities.end(),
                                                owo::agent::kAgentCapabilityOutputEffects) !=
                                              request.capabilities.end() ? "yes" : "no")
                              << " sensitive=" << request.application.sensitive_input << '\n';
                    for (const auto& update : request.slot_updates) {
                        std::cout << "       slot_update=" << update.slot_id
                                  << " value=" << update.value
                                  << " lock=" << update.lock << " ranges=";
                        for (const auto& range : update.consumed_ranges)
                            std::cout << '[' << range.start << ',' << range.end << ')';
                        std::cout << '\n';
                    }
                    for (const auto& entry : request.context_entries) {
                        std::cout << "       context_entry=" << entry.context_id
                                  << " sequence=" << entry.sequence
                                  << " type=" << entry.context_type
                                  << " source=" << entry.source
                                  << " relevance=" << entry.relevance_milli
                                  << " filtered=" << entry.filtered
                                  << " redacted=" << entry.redacted
                                  << " truncated=" << entry.truncated
                                  << " app=" << entry.application.executable
                                  << " pid=" << entry.application.process_id
                                  << " tid=" << entry.application.thread_id
                                  << " class=" << entry.application.window_class
                                  << " focus_class=" <<
                                         entry.application.focus_window_class << '\n'
                                  << "         text=" << entry.text << '\n';
                    }
                    const auto idempotency_scope = request.session_id + ":" +
                        request.idempotency_key;
                    auto cached = request.idempotency_key.empty()
                        ? idempotent_responses.end()
                        : idempotent_responses.find(idempotency_scope);
                    auto response = cached == idempotent_responses.end()
                        ? process(request, sessions) : cached->second;
                    response.request_id = request.request_id;
                    response.schema_version = request.schema_version;
                    response.capabilities = {"protocol.negotiation", "input.structured",
                                             "context.entries", "context.privacy",
                                             "commands.risk", "errors.structured",
                                             "state.progress", "tasks.slots",
                                             "tasks.incremental"};
                    response.can_continue_input = true;
                    if (response.status == owo::agent::Status::thinking ||
                        response.status == owo::agent::Status::executing) {
                        response.can_cancel = true;
                        response.progress = 50;
                        response.retry_after_ms = 150;
                    } else if (response.status == owo::agent::Status::completed) {
                        response.progress = 100;
                    }
                    if (!response.error_code.empty()) {
                        response.error_message = response.message;
                        response.retryable = response.error_code == "session_not_found";
                    }
                    if (cached == idempotent_responses.end() &&
                        !request.idempotency_key.empty()) {
                        const auto session = sessions.find(request.session_id);
                        response.state_revision = session == sessions.end()
                            ? 1 : ++session->second.revision;
                        idempotent_responses.emplace(idempotency_scope, response);
                    }
                    const auto encoded = request.schema_version == 2
                        ? owo::agent::encode_response_v2(response)
                        : owo::agent::encode_response(response);
                    std::cout << "[发送] status=" << owo::agent::status_name(response.status)
                              << " commands=" << response.commands.size()
                              << " task=" << response.task.intent
                              << " task_revision=" << response.task.revision
                              << " message=" << response.message << "\n\n";
                    write_frame(pipe, encoded);
                } else {
                    if (payload.find("\"schema_version\":1") != std::string::npos) {
                        std::cerr << "[拒绝] 收到 Agent 协议 v1 请求；请升级并重新授权 "
                                     "OwO Agent IPC 连接器 1.3.0，然后重启 OwO。\n\n";
                    } else {
                        std::cerr << "[拒绝] " << decoded.diagnostic << "\n\n";
                    }
                }
            }
            FlushFileBuffers(pipe);
            DisconnectNamedPipe(pipe);
        }
        CloseHandle(pipe);
    }
    return 0;
}

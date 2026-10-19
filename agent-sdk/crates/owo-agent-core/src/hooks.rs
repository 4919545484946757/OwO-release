//! Hooks 生命周期扩展点（A2-1，差距文档批次 4；对齐 Claude Code / Codex 的
//! PreToolUse / PostToolUse / UserPromptSubmit / Stop / PreCompact 钩子）。
//!
//! - 配置：`settings.json` 的 `hooks: [{ event, matcher?, command }]`；
//! - 执行：命令经系统 shell 运行，事件上下文以 JSON 写入 **stdin**；
//! - 阻断语义：**exit code 2 = 阻断**，stderr 原样回喂模型（PreToolUse 拦下
//!   该次工具调用、UserPromptSubmit 拒绝该 prompt）；其余非零码仅记审计；
//! - 超时：单 hook 10s，超时视为失败（不阻断）——hooks 必须快而稳；
//! - 匹配：`matcher` 为工具名通配（`*` = 全部、`desktop_*` 前缀通配；缺省 = 全部），
//!   对无工具上下文的事件（Stop / PreCompact / UserPromptSubmit）matcher 恒匹配。
//!
//! 安全边界：hooks 是用户在自身配置里显式声明的命令，信任级别等同用户手动
//! 执行；**不**经过 SandboxManager 沙箱（沙箱门卫只约束模型触发的进程）。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;

/// 单 hook 执行超时：hooks 必须毫秒级，10s 已是宽容上限。
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// 生命周期事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    /// 工具执行前：可阻断（exit 2）。
    PreToolUse,
    /// 工具执行后：通知（stderr 仅记审计，不改变工具结果）。
    PostToolUse,
    /// 用户 prompt 提交时：可阻断（exit 2 = 拒绝本回合）。
    UserPromptSubmit,
    /// 回合结束：通知。
    Stop,
    /// 历史压缩前：通知（payload 含将压缩的消息条数）。
    PreCompact,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "pre_tool_use",
            HookEvent::PostToolUse => "post_tool_use",
            HookEvent::UserPromptSubmit => "user_prompt_submit",
            HookEvent::Stop => "stop",
            HookEvent::PreCompact => "pre_compact",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pre_tool_use" => Some(HookEvent::PreToolUse),
            "post_tool_use" => Some(HookEvent::PostToolUse),
            "user_prompt_submit" => Some(HookEvent::UserPromptSubmit),
            "stop" => Some(HookEvent::Stop),
            "pre_compact" => Some(HookEvent::PreCompact),
            _ => None,
        }
    }
}

/// 单条 hook 配置（settings.json 的 `hooks` 数组元素）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookConfig {
    /// 事件名（[`HookEvent::as_str`] 形态；解析失败时该条被忽略）。
    pub event: String,
    /// 工具名通配（`*` / `desktop_*`；缺省 = 全部工具）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// 经系统 shell 执行的命令；事件上下文 JSON 从 stdin 传入。
    pub command: String,
    /// 失败模式：`open`（缺省，兼容旧配置——启动失败/超时/非 0、2 退出不阻断）
    /// 或 `closed`（把 hook 当安全闸：任一失败都阻断并说明原因，审查 N1 第 6 条）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fail_mode: Option<String>,
}

impl HookConfig {
    /// 失败时是否阻断（`fail_mode=closed`；缺省 open 保持既有行为）。
    pub fn fail_closed(&self) -> bool {
        self.fail_mode
            .as_deref()
            .map(|mode| mode.trim().eq_ignore_ascii_case("closed"))
            .unwrap_or(false)
    }
}

/// hooks 执行结果。
#[derive(Debug, Clone, PartialEq)]
pub enum HookOutcome {
    /// 全部通过（或未配置任何匹配 hook）。
    Proceed,
    /// 存在 exit code 2 的 hook：stderr 原样回喂模型。
    Blocked(String),
}

#[derive(Debug, Default, Clone)]
pub struct HookManager {
    hooks: Vec<HookConfig>,
}

impl HookManager {
    /// 从配置构建；事件名非法的条目被忽略（解析失败宁可少执行，不误阻断）。
    pub fn from_configs(configs: &[HookConfig]) -> Self {
        let hooks = configs
            .iter()
            .filter(|config| HookEvent::parse(&config.event).is_some())
            .cloned()
            .collect();
        Self { hooks }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// 运行某事件的全部匹配 hooks。串行执行（数量少、毫秒级；并行反而放大抖动）。
    pub async fn run(&self, event: HookEvent, payload: &Value) -> HookOutcome {
        let configs: Vec<&HookConfig> = self
            .hooks
            .iter()
            .filter(|config| HookEvent::parse(&config.event) == Some(event))
            .filter(|config| hook_matches(&config.matcher, payload))
            .collect();
        let mut blocked_messages: Vec<String> = Vec::new();
        for config in configs {
            match run_one(config, payload).await {
                HookOutcome::Proceed => {}
                HookOutcome::Blocked(stderr) => {
                    blocked_messages.push(stderr);
                }
            }
        }
        if blocked_messages.is_empty() {
            HookOutcome::Proceed
        } else {
            HookOutcome::Blocked(blocked_messages.join("\n"))
        }
    }
}

/// matcher 匹配：payload 里的 `tool` 字段（无则恒匹配）。
fn hook_matches(matcher: &Option<String>, payload: &Value) -> bool {
    let Some(pattern) = matcher.as_deref().map(str::trim).filter(|p| !p.is_empty()) else {
        return true;
    };
    if pattern == "*" {
        return true;
    }
    let tool = payload.get("tool").and_then(Value::as_str).unwrap_or("");
    glob_suffix_match(pattern, tool)
}

/// 极简通配：仅支持 `*` 后缀通配（`desktop_*`）；其余按前缀精确匹配。
fn glob_suffix_match(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

/// 失败处理：`fail_mode=closed` 的 hook 进入阻断（stderr 回喂模型），
/// 缺省 open 只记日志（兼容旧配置，避免把通知类 hook 的偶发失败变成硬阻断）。
fn on_hook_failure(config: &HookConfig, reason: String) -> HookOutcome {
    tracing::warn!("{reason}（{}）", config.command);
    if config.fail_closed() {
        HookOutcome::Blocked(format!("{reason}（hook fail_mode=closed）"))
    } else {
        HookOutcome::Proceed
    }
}

/// 执行单条 hook：shell 包装、stdin 传 JSON、10s 超时、exit 2 = 阻断。
async fn run_one(config: &HookConfig, payload: &Value) -> HookOutcome {
    let body = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = tokio::process::Command::new("cmd");
        command.arg("/C").arg(&config.command);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = {
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg(&config.command);
        command
    };
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return on_hook_failure(config, format!("hook 启动失败：{error}"));
        }
    };
    // stdin 先写完再等结果，避免命令读 stdin 与我们等退出互锁。
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(body.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let output = match tokio::time::timeout(HOOK_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return on_hook_failure(config, format!("hook 执行失败：{error}"));
        }
        Err(_) => {
            return on_hook_failure(config, format!("hook 超时（>{HOOK_TIMEOUT:?}）"));
        }
    };
    let stderr = crate::tools::decode_process_output(&output.stderr)
        .trim()
        .to_string();
    match output.status.code() {
        Some(0) => HookOutcome::Proceed,
        Some(2) => HookOutcome::Blocked(if stderr.is_empty() {
            format!("hook 阻断（exit 2）：{}", config.command)
        } else {
            stderr
        }),
        code => {
            let reason = format!("hook 非零退出（code={code:?}）：{stderr}");
            if config.fail_closed() {
                on_hook_failure(config, reason)
            } else {
                tracing::warn!("{}（{}）", reason, config.command);
                HookOutcome::Proceed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(event: &str, matcher: Option<&str>, command: &str) -> HookConfig {
        HookConfig {
            event: event.to_string(),
            matcher: matcher.map(str::to_string),
            command: command.to_string(),
            fail_mode: None,
        }
    }

    fn closed_config(event: &str, command: &str) -> HookConfig {
        HookConfig {
            event: event.to_string(),
            matcher: None,
            command: command.to_string(),
            fail_mode: Some("closed".to_string()),
        }
    }

    #[test]
    fn invalid_event_names_are_ignored() {
        let manager = HookManager::from_configs(&[config("bogus_event", None, "echo hi")]);
        assert!(manager.is_empty());
        let manager = HookManager::from_configs(&[config("pre_tool_use", None, "echo hi")]);
        assert!(!manager.is_empty());
    }

    #[tokio::test]
    async fn no_matching_hooks_proceed() {
        let manager =
            HookManager::from_configs(&[config("pre_tool_use", Some("desktop_*"), "exit 2")]);
        let outcome = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "write_file" }))
            .await;
        assert_eq!(outcome, HookOutcome::Proceed);
    }

    #[tokio::test]
    async fn exit_two_blocks_with_stderr() {
        // Windows cmd /C 的退出码透传：exit 2 + stderr 输出。
        let command = if cfg!(windows) {
            "echo 自定义拒绝原因 1>&2 & exit /b 2"
        } else {
            "echo 自定义拒绝原因 1>&2; exit 2"
        };
        let manager = HookManager::from_configs(&[config("pre_tool_use", None, command)]);
        let outcome = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "run_command" }))
            .await;
        match outcome {
            HookOutcome::Blocked(message) => {
                assert!(message.contains("自定义拒绝原因"), "{message}")
            }
            other => panic!("应阻断：{other:?}"),
        }
    }

    #[tokio::test]
    async fn exit_zero_proceeds_and_nonblocking_codes_are_ignored() {
        let pass = if cfg!(windows) { "exit /b 0" } else { "exit 0" };
        let warn = if cfg!(windows) { "exit /b 1" } else { "exit 1" };
        let manager = HookManager::from_configs(&[
            config("pre_tool_use", None, pass),
            config("pre_tool_use", None, warn),
        ]);
        let outcome = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "read_file" }))
            .await;
        assert_eq!(outcome, HookOutcome::Proceed);
    }

    #[tokio::test]
    async fn fail_mode_closed_turns_failures_into_blocks() {
        // 审查 N1 第 6 条：hook 作为安全闸时必须可 fail-closed（缺省 open 保持兼容）。
        let warn = if cfg!(windows) { "exit /b 1" } else { "exit 1" };
        let manager = HookManager::from_configs(&[closed_config("pre_tool_use", warn)]);
        let outcome = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "run_command" }))
            .await;
        match outcome {
            HookOutcome::Blocked(message) => {
                assert!(message.contains("fail_mode=closed"), "{message}");
            }
            other => panic!("fail_mode=closed 下非零退出必须阻断：{other:?}"),
        }
        // exit 2 语义不因 fail_mode 改变。
        let block = if cfg!(windows) { "exit /b 2" } else { "exit 2" };
        let manager = HookManager::from_configs(&[closed_config("pre_tool_use", block)]);
        let outcome = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "read_file" }))
            .await;
        assert!(matches!(outcome, HookOutcome::Blocked(_)));
    }

    #[tokio::test]
    async fn stdin_receives_payload_json() {
        // hook 读 stdin 并校验内容命中时输出到 stderr 且 exit 2。
        let command = if cfg!(windows) {
            "findstr /C:\"run_command\" & if not errorlevel 1 (exit /b 2)"
        } else {
            "grep -q run_command && exit 2"
        };
        let manager = HookManager::from_configs(&[config("pre_tool_use", None, command)]);
        let blocked = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "run_command" }))
            .await;
        let passed = manager
            .run(HookEvent::PreToolUse, &json!({ "tool": "read_file" }))
            .await;
        assert!(matches!(blocked, HookOutcome::Blocked(_)));
        assert_eq!(passed, HookOutcome::Proceed);
    }
}

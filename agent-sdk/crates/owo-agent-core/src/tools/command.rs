use super::{
    decode_process_output, resolve_session_path, strip_verbatim_prefix, tool_sandbox_policy, Tool,
    ToolContext, ToolSpec,
};
use crate::external_tools;
use async_trait::async_trait;
use owo_agent_kernel::required_string;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const PROCESS_CLEANUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Dropping an in-flight command future must terminate its process tree as well.
struct CommandProcessGuard {
    handle: crate::sandbox::SandboxHandle,
    armed: bool,
}

impl CommandProcessGuard {
    fn new(handle: crate::sandbox::SandboxHandle) -> Self {
        Self {
            handle,
            armed: true,
        }
    }

    fn terminate(&self) -> Result<(), crate::sandbox::SandboxError> {
        let manager = crate::sandbox::default_manager();
        let mut manager = manager.lock().unwrap_or_else(|error| error.into_inner());
        manager.kill(&self.handle)
    }
}

impl Drop for CommandProcessGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.terminate();
        }
    }
}

async fn wait_for_command_abort(abort: Option<&std::sync::atomic::AtomicBool>) {
    let Some(abort) = abort else {
        std::future::pending::<()>().await;
        return;
    };
    while !abort.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// 读取日志尾部：返回 `(字节, 文件总长度)`。
///
/// 同时给出总长度，调用方才能机器可读地判断"这是全部输出"还是"只是尾部窗口"——
/// 只看尾部却不说明，模型会把 32KB 尾巴当成完整日志（本仓库其它读取类工具都带
/// `truncated`/`total_*` 字段）。
fn read_log_tail(path: &std::path::Path, max_bytes: u64) -> Result<(Vec<u8>, u64), std::io::Error> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(max_bytes)))?;
    let mut bytes = Vec::with_capacity(length.min(max_bytes) as usize);
    file.take(max_bytes).read_to_end(&mut bytes)?;
    Ok((bytes, length))
}

/// 后台 shell 记录（`run_command background=true` 产生）。
#[derive(Clone)]
struct BackgroundShell {
    handle: crate::sandbox::SandboxHandle,
    log_path: PathBuf,
    done: Arc<std::sync::atomic::AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
}

fn background_shells() -> &'static Mutex<HashMap<String, BackgroundShell>> {
    static SHELLS: std::sync::OnceLock<Mutex<HashMap<String, BackgroundShell>>> =
        std::sync::OnceLock::new();
    SHELLS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `shell_output`：读取后台 shell 的累积输出与状态。
pub(super) struct ShellOutputTool;

/// 后台 shell 输出只回读最后这么多字节（避免长跑日志吃满上下文）；
/// 响应里的 `truncated`/`total_bytes` 会说明是否还有更早的输出。
pub(super) const TAIL_WINDOW_BYTES: u64 = 32 * 1024;

#[async_trait]
impl Tool for ShellOutputTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "shell_output".into(),
            description:
                "查看后台 shell 的输出与状态（shell_id 来自 run_command background=true）；\
                          日志很长时只返回最后 32KB 并标记 truncated"
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": { "shell_id": { "type": "string" } },
                "required": ["shell_id"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let shell_id = required_string(&args, "shell_id")?;
        let shell = background_shells()
            .lock()
            .map_err(|_| "后台 shell 注册表中毒".to_string())?
            .get(&shell_id)
            .cloned()
            .ok_or_else(|| format!("未知 shell_id：{shell_id}"))?;
        let tail = read_log_tail(&shell.log_path, TAIL_WINDOW_BYTES)
            .map_err(|error| format!("executor/log_read_failed: {error}"))?;
        let done = shell.done.load(std::sync::atomic::Ordering::Relaxed);
        let exit_code = *shell
            .exit_code
            .lock()
            .map_err(|_| "后台 shell 状态锁中毒".to_string())?;
        let (bytes, total_bytes) = tail;
        Ok(json!({
            "shell_id": shell_id,
            "running": !done,
            "exit_code": exit_code,
            "log_path": shell.log_path.display().to_string(),
            "output": decode_process_output(&bytes),
            // 尾部窗口语义要显式：total_bytes > returned_bytes 时模型才知道前面还有输出。
            "returned_bytes": bytes.len(),
            "total_bytes": total_bytes,
            "tail_window_bytes": TAIL_WINDOW_BYTES,
            "truncated": total_bytes > bytes.len() as u64,
        }))
    }
}

/// `kill_shell`：终止后台 shell（仅限本 Agent 启动的 shell）。
pub(super) struct KillShellTool;

#[async_trait]
impl Tool for KillShellTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "kill_shell".into(),
            description: "终止后台 shell（仅限 run_command background=true 启动的 shell）".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "shell_id": { "type": "string" } },
                "required": ["shell_id"]
            }),
            effect: None,
        }
    }

    async fn run(&self, _ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let shell_id = required_string(&args, "shell_id")?;
        let shell = background_shells()
            .lock()
            .map_err(|_| "后台 shell 注册表中毒".to_string())?
            .get(&shell_id)
            .cloned()
            .ok_or_else(|| format!("未知 shell_id：{shell_id}"))?;
        if shell.done.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(json!({ "shell_id": shell_id, "killed": false, "already_finished": true }));
        }
        let manager = crate::sandbox::default_manager();
        let termination = {
            let mut manager = manager.lock().unwrap_or_else(|error| error.into_inner());
            manager.kill(&shell.handle)
        };
        let stopped = tokio::time::timeout(PROCESS_CLEANUP_TIMEOUT, async {
            while !shell.done.load(std::sync::atomic::Ordering::Acquire) {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .is_ok();
        if !stopped {
            return Err(format!(
                "executor/cleanup_failed: shell did not exit; termination={termination:?}"
            ));
        }
        Ok(json!({
            "shell_id": shell_id, "killed": termination.is_ok(),
            "already_finished": termination.is_err(), "running": false
        }))
    }
}

pub(super) struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "run_command".into(),
            description: "在工作区内执行 shell 命令（需审批，60 秒超时）".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "background": { "type": "boolean", "description": "后台运行（返回 shell_id，用 shell_output/kill_shell 管理）" }
                },
                "required": ["command"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let command = required_string(&args, "command")?;
        let timeout_ms = args
            .get("_host_timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(60_000)
            .clamp(1, 60_000);
        let cwd = args
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string)
            .map(|path| resolve_session_path(ctx, &path))
            .transpose()?
            .unwrap_or_else(|| ctx.workspace.to_path_buf());

        // 沙箱门卫：run_command 统一经 SandboxManager 执行（X01）。
        // 策略：工作区作用域 + 危险片段 deny + Job 级隔离（允许显式降级，审计记录）。
        let mut policy = tool_sandbox_policy(ctx, "run_command");
        policy.cpu_ms = Some(60_000);
        policy.mem_mb = Some(1024);
        // `cmd /C <外部命令>` 至少占 2 个 Job 进程（cmd + 子进程）；默认 limit=1 会
        // 直接报 "Not enough quota"。放宽到 16，仍能兜住进程炸弹。
        policy.active_process_limit = Some(16);
        // 命令文本（cmd /C <command> 的命令体）走结构化危险命令匹配（审查 N4）：
        // 覆盖 `rm -r -f`、`git  push`、`git -c … push`、`r^d /s`、`powershell -enc`
        // 等旧子串黑名单的绕过变体；自定义 deny 片段与沙箱 deny_programs 一并生效。
        let mut deny_fragments = ctx.policy.deny_fragments();
        deny_fragments.extend(policy.deny_programs.iter().cloned());
        if let Some(pattern) =
            crate::command_guard::dangerous_command_match(&command, &deny_fragments)
        {
            return Err(format!("命令命中危险模式：{pattern}"));
        }
        let mut sandbox_command = crate::sandbox::SandboxCommand::new("cmd", policy.clone())
            .with_args(vec!["/C".to_string(), command.to_string()])
            .with_cwd(cwd.clone());
        if args
            .get("background")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            // 后台 shell：输出重定向到日志文件，句柄入注册表，watcher 任务等待退出。
            // 日志必须落在**工作区内**：沙箱文件作用域只允许工作区，写到 %TEMP% 会被拒。
            let shell_id = uuid::Uuid::new_v4().to_string();
            // 去 verbatim 前缀：`\\?\C:\…` 在沙箱（受限令牌）下做 cmd 重定向会失败。
            let workspace_plain = strip_verbatim_prefix(ctx.workspace);
            let log_dir = workspace_plain.join(".owo").join("shells");
            tokio::fs::create_dir_all(&log_dir)
                .await
                .map_err(|error| format!("创建后台日志目录失败：{error}"))?;
            let log_path = log_dir.join(format!("{shell_id}.log"));
            // 用包装脚本而不是 `cmd /C "<cmd> > "<log>" 2>&1"`：嵌套引号会被沙箱的
            // 参数引用破坏（cmd 提前截断 → exit 1、日志不生成）。
            let script_path = log_dir.join(format!("{shell_id}.cmd"));
            let script = format!(
                "@echo off\r\n{} > \"{}\" 2>&1\r\nexit /b %ERRORLEVEL%\r\n",
                command,
                log_path.display()
            );
            tokio::fs::write(&script_path, script)
                .await
                .map_err(|error| format!("写入后台脚本失败：{error}"))?;
            let mut background_command = crate::sandbox::SandboxCommand::new("cmd", policy.clone())
                .with_args(vec![
                    "/C".to_string(),
                    script_path.to_string_lossy().into_owned(),
                ])
                .with_cwd(cwd.clone());
            if let Some(path) = external_tools::path_with_bundled_tools() {
                background_command.env.push(("PATH".to_string(), path));
            }
            let manager = crate::sandbox::default_manager();
            let process = {
                let mut manager = manager
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                manager
                    .spawn(&background_command)
                    .map_err(|error| format!("沙箱拒绝执行（{command}）：{error}"))?
            };
            let handle = process.handle.clone();
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let exit_code: Arc<Mutex<Option<i32>>> = Arc::new(Mutex::new(None));
            let done_task = Arc::clone(&done);
            let code_task = Arc::clone(&exit_code);
            tokio::task::spawn_blocking(move || {
                let mut process = process;
                let result = process.wait_output();
                drop(process);
                if let Ok(info) = result {
                    if let Ok(mut guard) = code_task.lock() {
                        *guard = Some(info.exit_code);
                    }
                    done_task.store(true, std::sync::atomic::Ordering::Release);
                }
            });
            background_shells()
                .lock()
                .map_err(|_| "后台 shell 注册表中毒".to_string())?
                .insert(
                    shell_id.clone(),
                    BackgroundShell {
                        handle,
                        log_path: log_path.clone(),
                        done,
                        exit_code,
                    },
                );
            return Ok(json!({
                "shell_id": shell_id,
                "background": true,
                "log_path": log_path.display().to_string(),
                "hint": "用 shell_output 查看输出，kill_shell 终止",
            }));
        }
        if let Some(path) = external_tools::path_with_bundled_tools() {
            sandbox_command.env.push(("PATH".to_string(), path));
        }

        let manager = crate::sandbox::default_manager();
        let process = {
            let mut manager = manager
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            manager
                .spawn(&sandbox_command)
                .map_err(|error| format!("沙箱拒绝执行（{}）：{error}", command))?
        };

        // Waiting is blocking, but cancellation owns an independent Job handle.
        let process_handle = process.handle.clone();
        let mut process_guard = CommandProcessGuard::new(process_handle);
        let command_started = std::time::Instant::now();
        let mut wait_task = tokio::task::spawn_blocking(move || {
            let mut process = process;
            process.wait_output()
        });
        let stopped = tokio::select! {
            joined = &mut wait_task => {
                process_guard.armed = false;
                Some(joined
                    .map_err(|error| format!("executor/wait_failed: {error}"))?
                    .map_err(|error| format!("executor/process_failed: {error}"))?)
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)) => None,
            _ = wait_for_command_abort(ctx.abort) => None,
        };
        let output = if let Some(output) = stopped {
            output
        } else {
            let cancelled = ctx
                .abort
                .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire));
            let termination = process_guard.terminate();
            let cleanup = tokio::time::timeout(PROCESS_CLEANUP_TIMEOUT, &mut wait_task).await;
            if matches!(&cleanup, Ok(Ok(Ok(_)))) {
                process_guard.armed = false;
                let code = if cancelled {
                    "executor/cancelled"
                } else {
                    "executor/process_timeout"
                };
                return Err(format!(
                    "{code}: process tree stopped; task budget={timeout_ms}ms"
                ));
            }
            return Err(format!(
                "executor/cleanup_failed: process termination={termination:?}; wait={cleanup:?}"
            ));
        };

        Ok(json!({
            "command": command,
            "exit_code": output.exit_code,
            "duration_ms": command_started.elapsed().as_millis() as u64,
            "stdout": decode_process_output(&output.stdout),
            "stderr": decode_process_output(&output.stderr),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::read_log_tail;
    use std::io::Write;

    /// 尾窗读取必须同时给出总长度，调用方才能机器可读地判断是否被截断。
    #[test]
    fn log_tail_reports_total_length_for_truncation_awareness() {
        let dir = std::env::temp_dir().join(format!("owo-log-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shell.log");

        std::fs::write(&path, b"short").unwrap();
        let (bytes, total) = read_log_tail(&path, 32 * 1024).unwrap();
        assert_eq!(bytes, b"short");
        assert_eq!(
            total, 5,
            "小日志：total == returned，调用方判 truncated=false"
        );

        let big: Vec<u8> = (0..40_000u32).map(|index| (index % 251) as u8).collect();
        {
            let mut file = std::fs::File::create(&path).unwrap();
            file.write_all(&big).unwrap();
        }
        let (bytes, total) = read_log_tail(&path, 32 * 1024).unwrap();
        assert_eq!(total, big.len() as u64, "total 必须是文件真实长度");
        assert_eq!(bytes.len(), 32 * 1024, "只回读尾部窗口");
        assert!(total > bytes.len() as u64, "长日志必须能被判定为已截断");
        assert_eq!(&bytes[..], &big[big.len() - 32 * 1024..], "读的是最后 32KB");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

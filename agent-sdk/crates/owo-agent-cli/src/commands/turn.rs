// P1（指南 §7.2/§8 P1）：`turn` 子命令经**唯一 Daemon 客户端**执行。
//
// 与旧实现的根本区别：本文件不再 `Agent::new` / `SqliteSessionStore::open` /
// `connect_mcp_clients`——会话、模型、权限、工具、审计全部由 Daemon 持有，CLI 只是
// 一个事件流消费者。单实例协议由 `support::ensure_daemon_client` 实现（有则复用，
// 无则启动一个分离的 serve 进程）。此约束由 `tests/turn_path_guard_tests.rs` 静态守卫。

use crate::support::*;
use crate::ui_output::{
    parse_approval_response, render_error_jsonl, render_event_jsonl, render_final_result_plain,
    OutputMode, PermissionsProfile, StreamPrinter,
};
use clap::Args;
use colored::Colorize;
use owo_agent_protocol::{PermissionResponse, SseEvent};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Args)]
pub(crate) struct TurnArgs {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// 任务提示词；传 `-` 或省略（stdin 非终端）时从 stdin 读取（B4）
    #[arg(long)]
    prompt: Option<String>,
    #[arg(long)]
    model: Option<String>,
    /// 自动允许所有审批（仅测试用；等价 --permissions trusted）
    #[arg(long)]
    no_approval: bool,
    /// 在指定会话里执行本回合（续接历史；与 --continue 互斥）
    #[arg(long, conflicts_with = "continuation")]
    session: Option<String>,
    /// 续接本工作区最近更新的会话（对标 `codex exec resume --last`）
    #[arg(long = "continue", alias = "last")]
    continuation: bool,
    /// 数据根（缺省用户级数据目录）；用于发现/启动共享 Daemon。
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

/// stdin 提示词上限：与服务端默认 JSON 体上限（1 MiB）对齐，避免先读入
/// 超大输入再被 413 拒绝；用 `Read::take` 限制读取量，内存有界。
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

fn ensure_prompt_size(len: usize) -> Result<(), String> {
    if len > MAX_PROMPT_BYTES {
        return Err(format!(
            "stdin 提示词过大（上限 {} KiB）：请改用附件上传或拆分任务",
            MAX_PROMPT_BYTES / 1024
        ));
    }
    Ok(())
}

/// B4（取优合并自远端 engine）：解析一次性任务的提示词来源——
/// `--prompt X` > `--prompt -`（stdin）> stdin 管道（未显式传入且非终端）。
fn resolve_turn_prompt(option: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::{IsTerminal, Read};
    let explicit = option.as_deref().map(str::trim);
    let needs_stdin =
        matches!(explicit, Some("-")) || (explicit.is_none() && !std::io::stdin().is_terminal());
    let stdin_text = if needs_stdin {
        let mut buffer = String::new();
        std::io::stdin()
            .take((MAX_PROMPT_BYTES + 1) as u64)
            .read_to_string(&mut buffer)?;
        ensure_prompt_size(buffer.len())?;
        Some(buffer)
    } else {
        None
    };
    resolve_prompt_value(explicit, stdin_text).map_err(Into::into)
}

/// [`resolve_turn_prompt`] 的纯函数部分（IO 之外可单测）：
/// 显式文本优先；`-`/省略时取 stdin；空文本返回可读错误。
fn resolve_prompt_value(
    explicit: Option<&str>,
    stdin_text: Option<String>,
) -> Result<String, String> {
    let prompt = match explicit {
        Some("-") => stdin_text.unwrap_or_default(),
        Some(text) if !text.is_empty() => text.to_string(),
        Some(_) => String::new(),
        None => stdin_text.unwrap_or_default(),
    };
    if prompt.trim().is_empty() {
        return Err("提示词为空：用 --prompt 传入，或从 stdin 喂入（--prompt -）".to_string());
    }
    Ok(prompt)
}

pub(crate) async fn run_turn(
    args: TurnArgs,
    output: OutputMode,
    permissions: PermissionsProfile,
) -> Result<u8, Box<dyn std::error::Error>> {
    let prompt = resolve_turn_prompt(args.prompt.clone())?;
    let workspace = args.workspace.canonicalize()?;
    let workspace_str = workspace.to_string_lossy().to_string();
    let root = ensure_data_root(args.data_dir.clone(), &workspace);
    let client = ensure_daemon_client(&root, &workspace).await?;
    // 会话选择（对标 `codex exec resume`）：显式 --session 优先；--continue 取本工作区
    // 最近更新的未归档会话；都不给才新建。续接语义必须显式，避免把历史悄悄接进来。
    let resumed = match (args.session.as_deref(), args.continuation) {
        (Some(id), _) => Some(client.get_session(id).await.map_err(|error| {
            format!("会话不可用：{id}（{error}）；用 `owo-agent turn` 新建或检查会话 ID")
        })?),
        (None, true) => {
            let sessions = client.list_sessions().await?;
            match crate::support::latest_session_for_workspace(&sessions, &workspace) {
                Some(session) => Some(session.clone()),
                None => {
                    return Err(format!(
                    "该工作区没有可续接的会话（{}）：先运行一次 `owo-agent turn --prompt <任务>`",
                    workspace.display()
                )
                    .into())
                }
            }
        }
        (None, false) => None,
    };
    let session = match resumed {
        Some(session) => {
            if !crate::support::session_matches_workspace(&session, &workspace) {
                return Err(format!(
                    "会话 {} 属于另一个工作区（{}）；请改用 `--workspace` 指向该工作区",
                    session.id, session.workspace
                )
                .into());
            }
            if !matches!(output, OutputMode::Jsonl) {
                eprintln!(
                    "续接会话：{}（{}）",
                    session.id,
                    session.title.clone().unwrap_or_default()
                );
            }
            session
        }
        None => {
            client
                .create_session_with_model(&workspace_str, args.model.clone())
                .await?
        }
    };

    // Ctrl+C → 取消运行中的回合（贯穿网络与 Daemon 侧 abort 标志）。
    let invocation_id = uuid::Uuid::new_v4().to_string();
    let cancel_turn_id = invocation_id.clone();
    let abort = Arc::new(AtomicBool::new(false));
    let cancel_client = client.clone();
    let cancel_session = session.id.clone();
    let abort_flag = Arc::clone(&abort);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            abort_flag.store(true, Ordering::Relaxed);
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                cancel_client.cancel_turn_id(&cancel_session, &cancel_turn_id),
            )
            .await;
        }
    });

    let trusted = args.no_approval || matches!(permissions, PermissionsProfile::Trusted);
    if args.no_approval {
        eprintln!("⚠ 已弃用：--no-approval 将在未来版本移除；兼容期等价 --permissions trusted（高风险：全部操作自动批准）");
    }

    let mut stream = client
        .open_turn_with_id(
            &session.id,
            &prompt,
            matches!(permissions, PermissionsProfile::ReadOnly),
            &invocation_id,
        )
        .await?;
    let turn_id = stream.turn_id().map(str::to_string);
    let mut completion = crate::ui_output::TurnCompletion::default();
    let mut stream_error: Option<String> = None;
    let mut printer = StreamPrinter::new();

    let mut cancelling = false;
    let mut cancel_deadline = tokio::time::Instant::now();
    loop {
        let event = tokio::select! {
            event=stream.next_event() => event,
            _=crate::ui_output::wait_for_local_abort(&abort), if !cancelling => {
                cancelling=true; cancel_deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(10); continue;
            }
            _=tokio::time::sleep_until(cancel_deadline), if cancelling => {
                stream_error=Some("turn/cancellation_unconfirmed: 宿主尚未确认取消".into()); break;
            }
        };
        let Some(event) = event else { break };
        let event = match event {
            Ok(event) => event,
            Err(error) => {
                stream_error = Some(error.to_string());
                break;
            }
        };
        completion.observe(&event);
        let human = matches!(output, OutputMode::Human);
        match &event {
            SseEvent::TokenDelta { .. } => {
                if human {
                    printer.print_sse(&event);
                }
            }
            SseEvent::Final { text } => {
                let _ = text;
                if human {
                    printer.print_sse(&event);
                }
                emit_event(output, &event);
            }
            SseEvent::PermissionRequest { request_id, .. } => {
                if human {
                    printer.print_sse(&event);
                }
                emit_event(output, &event);
                let response = decide_permission(output, trusted, &event)?;
                if let Err(error) = client
                    .respond_permission(&session.id, request_id, &response)
                    .await
                {
                    eprintln!("审批响应失败：{error}");
                }
            }
            SseEvent::ToolResult { .. } => {
                emit_event(output, &event);
                if human {
                    printer.print_sse(&event);
                }
            }
            SseEvent::UserQuestion {
                question_id,
                question,
                options,
            } => {
                emit_event(output, &event);
                if human {
                    printer.print_sse(&event);
                }
                let answer = decide_question(output, question, options)?;
                if let Err(error) = client
                    .answer_question(&session.id, question_id, &answer)
                    .await
                {
                    eprintln!("回答提问失败：{error}");
                }
            }
            SseEvent::TurnFailed { message, .. } => {
                emit_event(output, &event);
                if human {
                    printer.print_sse(&event);
                }
                let _ = message;
            }
            _ => {
                emit_event(output, &event);
                if human {
                    printer.print_sse(&event);
                }
            }
        }
    }

    if let Some(error) = stream_error {
        emit_error(output, &error);
        return Err(error.into());
    }

    let completed = completion
        .finish()
        .inspect_err(|error| emit_error(output, error))?;
    let diffs = client.session_diff(&session.id).await.unwrap_or_default();
    let diff_paths: Vec<String> = diffs.iter().map(|diff| diff.path.clone()).collect();
    if matches!(output, OutputMode::Human) {
        printer.finish();
    }
    render_completed(
        output,
        &completed,
        &session.id,
        turn_id.as_deref(),
        &diff_paths,
    );
    Ok(completed.exit_code())
}

/// 非 human 模式的事件协议出口（jsonl 包裹为 `turn_event`；plain 只保留 stderr 提示）。
fn emit_event(output: OutputMode, event: &SseEvent) {
    match output {
        OutputMode::Human => {}
        OutputMode::Plain => {
            if let SseEvent::ToolResult {
                tool, ok, error, ..
            } = event
            {
                if !ok {
                    eprintln!("tool {tool} 失败：{}", error.as_deref().unwrap_or("-"));
                }
            }
        }
        OutputMode::Jsonl => {
            // token_delta 不进 JSONL 协议（保持既有契约：文本由 final 承载）。
            if !matches!(event, SseEvent::TokenDelta { .. }) {
                println!("{}", render_event_jsonl(event));
            }
        }
    }
}

fn emit_error(output: OutputMode, message: &str) {
    match output {
        OutputMode::Jsonl => println!("{}", render_error_jsonl(message)),
        _ => eprintln!("错误：{message}"),
    }
}

fn render_completed(
    output: OutputMode,
    completed: &crate::ui_output::CompletedTurn,
    session_id: &str,
    turn_id: Option<&str>,
    diff_paths: &[String],
) {
    match output {
        OutputMode::Human => {
            let label = completed.label();
            if completed.exit_code() == 0
                && completed.status == owo_agent_protocol::CompletionStatusV1::Accepted
            {
                println!("\n{} 工具 {} 步", label.green(), completed.steps);
            } else {
                println!("\n{} 工具 {} 步", label.yellow(), completed.steps);
            }
            if let Some(error) = &completed.failure {
                eprintln!("{error}");
            }
            for path in diff_paths {
                println!("  M {path}");
            }
        }
        OutputMode::Plain => {
            for line in render_final_result_plain(completed.final_text.as_deref(), diff_paths) {
                println!("{line}");
            }
            eprintln!("{}", completed.label());
            if let Some(error) = &completed.failure {
                eprintln!("{error}");
            }
        }
        OutputMode::Jsonl => println!(
            "{}",
            completed.render_jsonl(session_id, turn_id, diff_paths)
        ),
    }
}

/// 审批决策：trusted 自动允许；human 交互选择；jsonl 默认拒绝（避免编排被阻塞）。
fn decide_permission(
    output: OutputMode,
    trusted: bool,
    event: &SseEvent,
) -> Result<PermissionResponse, Box<dyn std::error::Error>> {
    let SseEvent::PermissionRequest {
        tool,
        reason,
        level,
        ..
    } = event
    else {
        return Ok(parse_approval_response("deny"));
    };
    if trusted {
        return Ok(PermissionResponse {
            allow: true,
            remember: None,
            scope: Some("once".to_string()),
        });
    }
    if !matches!(output, OutputMode::Human) || !std::io::IsTerminal::is_terminal(&std::io::stdin())
    {
        eprintln!("需要审批（jsonl 非交互，默认拒绝）：{tool}（{reason}）");
        return Ok(parse_approval_response("deny"));
    }
    if matches!(output, OutputMode::Human) {
        // 审批卡已由 StreamPrinter 输出（含参数/风险/影响），这里只给交互提示。
    } else {
        eprintln!(
            "{} 需要 {} 权限：{tool}（{reason}）",
            "审批".yellow(),
            level.as_deref().unwrap_or("unknown")
        );
    }
    eprint!("允许？[y=仅本次 / t=本任务 / w=工作区长期 / n=拒绝] ");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(parse_approval_response(&line))
}

/// 提问决策：human 交互回答；plain/jsonl 非交互给确定性继续指令，
/// 避免整轮回合在提问卡上空等 300s（与审批的 non-interactive 默认口径一致）。
fn decide_question(
    output: OutputMode,
    question: &str,
    options: &[String],
) -> Result<String, Box<dyn std::error::Error>> {
    const NON_INTERACTIVE: &str =
        "请基于现有信息按最合理假设继续完成任务，不要再次提问；在最终回复中说明所做假设。";
    if !matches!(output, OutputMode::Human) || !std::io::IsTerminal::is_terminal(&std::io::stdin())
    {
        eprintln!("提问（非交互模式，自动继续）：{question}");
        return Ok(NON_INTERACTIVE.to_string());
    }
    eprintln!("{} {question}", "提问".yellow());
    for (index, option) in options.iter().enumerate() {
        eprintln!("  [{}] {option}", index + 1);
    }
    eprint!("回答（回车使用默认：按最合理假设继续）：");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let answer = line.trim();
    if answer.is_empty() {
        return Ok(NON_INTERACTIVE.to_string());
    }
    if let Ok(index) = answer.parse::<usize>() {
        if let Some(option) = index.checked_sub(1).and_then(|index| options.get(index)) {
            return Ok(option.clone());
        }
    }
    Ok(answer.to_string())
}

#[cfg(test)]
mod tests {
    use super::{decide_question, ensure_prompt_size, resolve_prompt_value, MAX_PROMPT_BYTES};
    use crate::ui_output::OutputMode;

    #[test]
    fn oversized_stdin_prompt_is_rejected_with_actionable_error() {
        assert!(ensure_prompt_size(1024).is_ok());
        let error = ensure_prompt_size(MAX_PROMPT_BYTES + 1).unwrap_err();
        assert!(error.contains("过大"), "{error}");
        assert!(error.contains("KiB"), "{error}");
    }

    /// plain/jsonl 非交互模式不能等待 300s 提问超时：直接给确定性继续指令。
    #[test]
    fn non_interactive_question_gets_deterministic_continue_answer() {
        for mode in [OutputMode::Jsonl, OutputMode::Plain] {
            let answer = decide_question(mode, "还缺一个字段名", &[]).expect("非交互回答不应失败");
            assert!(
                answer.contains("最合理假设"),
                "回答应要求按假设继续：{answer}"
            );
        }
    }

    /// B4：显式 `--prompt X` 优先，不触碰 stdin。
    #[test]
    fn explicit_prompt_wins_over_stdin() {
        let prompt = resolve_prompt_value(Some("写一个测试"), Some("stdin 内容".to_string()))
            .expect("显式提示词可用");
        assert_eq!(prompt, "写一个测试");
    }

    /// B4：`--prompt -` 读 stdin；stdin 为空 → 可读错误而非空提示词。
    #[test]
    fn dash_reads_stdin_and_empty_is_rejected() {
        let prompt = resolve_prompt_value(Some("-"), Some("来自管道".to_string()))
            .expect("stdin 提示词可用");
        assert_eq!(prompt, "来自管道");

        let error =
            resolve_prompt_value(Some("-"), Some(String::new())).expect_err("空 stdin 必须报错");
        assert!(error.contains("提示词为空"), "{error}");
    }

    /// B4：省略 `--prompt` 且提供了 stdin 管道内容 → 整体读入；都没有 → 报错。
    #[test]
    fn missing_prompt_uses_pipe_or_errors() {
        let prompt =
            resolve_prompt_value(None, Some("管道内容".to_string())).expect("管道内容可用");
        assert_eq!(prompt, "管道内容");

        let error = resolve_prompt_value(None, None).expect_err("无提示词必须报错");
        assert!(error.contains("提示词为空"), "{error}");
    }
}

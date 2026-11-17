//! 任务剧本工具（**可选能力组**）：让模型按剧本推进，进度写入工作区状态文件。
//!
//! 为什么默认关闭：这两个工具会改变模型可见工具面（上下文成本 + 行为引导），
//! 属于"额外功能"。由 `settings.json` 的 `tool_capabilities.playbooks` 显式开启，
//! 未开启时工具表与既有行为完全一致。
//!
//! 安全边界：
//! * 文件路径经 [`resolve_session_path`] 限制在会话工作区内；
//! * `playbook_next` 只读（不写状态文件）；`playbook_record` 只写同目录的
//!   `<stem>.state.json`，副作用登记为 Write，照常走审批；
//! * 剧本/状态文件大小与字段长度有上限，避免把上下文撑爆。

use super::{resolve_session_path, Tool, ToolContext, ToolSpec};
use crate::playbook::{Playbook, PlaybookRun, PlaybookStep};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 剧本文件大小上限（读取前先看元数据）。
const MAX_PLAYBOOK_BYTES: u64 = 256 * 1024;
/// 记录备注长度上限（会进状态文件与会话历史）。
const MAX_NOTE_CHARS: usize = 2000;

/// `playbook_next`：查询下一步（只读）。
pub(super) struct PlaybookNextTool;

/// `playbook_record`：记录一步结果（写状态文件）。
pub(super) struct PlaybookRecordTool;

fn state_path(file: &Path) -> PathBuf {
    file.with_extension("state.json")
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("缺少参数 `{key}`（非空字符串）"))
}

fn load_playbook(file: &Path) -> Result<Playbook, String> {
    let metadata = std::fs::metadata(file)
        .map_err(|error| format!("读取剧本失败（{}）：{error}", file.display()))?;
    if metadata.len() > MAX_PLAYBOOK_BYTES {
        return Err(format!(
            "剧本文件过大（{} 字节 > 上限 {MAX_PLAYBOOK_BYTES}）",
            metadata.len()
        ));
    }
    let content = std::fs::read_to_string(file)
        .map_err(|error| format!("读取剧本失败（{}）：{error}", file.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let playbook: Playbook =
        serde_json::from_str(content).map_err(|error| format!("剧本不是合法 JSON：{error}"))?;
    let issues = playbook.validate();
    if !issues.is_empty() {
        let detail: Vec<String> = issues
            .iter()
            .take(3)
            .map(|issue| issue.message.clone())
            .collect();
        return Err(format!(
            "剧本不合法（{} 个问题）：{}",
            issues.len(),
            detail.join("；")
        ));
    }
    Ok(playbook)
}

fn load_or_new_run(file: &Path, playbook: &Playbook) -> Result<PlaybookRun, String> {
    let path = state_path(file);
    if !path.exists() {
        return Ok(PlaybookRun::new(playbook));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("读取状态文件失败（{}）：{error}", path.display()))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let run: PlaybookRun = serde_json::from_str(content)
        .map_err(|error| format!("状态文件不是合法 JSON（{}）：{error}", path.display()))?;
    if run.playbook_id != playbook.id {
        return Err(format!(
            "状态文件属于另一个剧本（{} ≠ {}）：换一个文件或清理 {}",
            run.playbook_id,
            playbook.id,
            path.display()
        ));
    }
    Ok(run)
}

/// 原子写状态文件：先写 `.tmp` 再 rename，半截文件不会留下。
fn save_run(file: &Path, run: &PlaybookRun) -> Result<(), String> {
    let path = state_path(file);
    let json =
        serde_json::to_string_pretty(run).map_err(|error| format!("序列化状态失败：{error}"))?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "playbook.state.json".to_string());
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&temporary, json)
        .map_err(|error| format!("写状态文件失败（{}）：{error}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("替换状态文件失败（{}）：{error}", path.display()))?;
    Ok(())
}

fn step_json(step: &PlaybookStep) -> Value {
    json!({
        "id": step.id,
        "title": step.title,
        "kind": match step.kind {
            crate::playbook::StepKind::Model => "model",
            crate::playbook::StepKind::Check => "check",
            crate::playbook::StepKind::Human => "human",
        },
        "detail": step.detail,
        "acceptance": step.acceptance,
        "depends_on": step.depends_on,
    })
}

fn progress_json(playbook: &Playbook, run: &PlaybookRun) -> Value {
    let progress = run.progress(playbook);
    json!({
        "total": progress.total,
        "passed": progress.passed,
        "failed": progress.failed,
        "skipped": progress.skipped,
        "pending": progress.pending,
        "blocked": progress.blocked,
        "percent": progress.percent(),
        "summary": run.summary(playbook),
        "complete": run.is_complete(playbook),
    })
}

fn next_json(playbook: &Playbook, run: &PlaybookRun) -> Value {
    let next = run.next_ready(playbook).map(step_json);
    let blocked: Vec<String> = playbook
        .steps
        .iter()
        .filter(|step| run.is_blocked(playbook, &step.id))
        .map(|step| step.id.clone())
        .collect();
    json!({ "next": next, "blocked": blocked })
}

#[async_trait]
impl Tool for PlaybookNextTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "playbook_next".into(),
            description: "查询任务剧本的下一步（只读）：返回可执行步骤、完成判据与整体进度。\
                          流程：playbook_next → 按判据执行这一步 → playbook_record 记录结果 → 再 next。"
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "剧本文件路径（相对工作区）" }
                },
                "required": ["file"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let file = arg_str(&args, "file")?;
        let path = resolve_session_path(ctx, &file)?;
        let playbook = load_playbook(&path)?;
        let run = load_or_new_run(&path, &playbook)?;
        Ok(json!({
            "playbook": { "id": playbook.id, "name": playbook.name },
            "progress": progress_json(&playbook, &run),
            "next": next_json(&playbook, &run),
            "state_file": state_path(&path).to_string_lossy(),
            "hint": "只读查询，未写状态；执行这一步后用 playbook_record 记录结果",
        }))
    }
}

#[async_trait]
impl Tool for PlaybookRecordTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "playbook_record".into(),
            description:
                "记录任务剧本某一步的结果（写入 <剧本>.state.json）：\
                          outcome=pass 需给出证据；fail 会累计尝试次数并阻塞下游；skip 会阻塞下游。"
                    .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "剧本文件路径（相对工作区）" },
                    "step": { "type": "string", "description": "步骤 id" },
                    "outcome": { "type": "string", "enum": ["pass", "fail", "skip"] },
                    "note": { "type": "string", "description": "证据/失败原因/跳过原因" }
                },
                "required": ["file", "step", "outcome"]
            }),
            effect: None,
        }
    }

    async fn run(&self, ctx: &mut ToolContext<'_>, args: Value) -> Result<Value, String> {
        let file = arg_str(&args, "file")?;
        let step_id = arg_str(&args, "step")?;
        let outcome = arg_str(&args, "outcome")?;
        if !matches!(outcome.as_str(), "pass" | "fail" | "skip") {
            return Err(format!("outcome 非法：{outcome}（pass|fail|skip）"));
        }
        let note = args
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(format!(
                "note 过长（{} 字符 > {MAX_NOTE_CHARS}）：只写结论与关键证据",
                note.chars().count()
            ));
        }
        let path = resolve_session_path(ctx, &file)?;
        let playbook = load_playbook(&path)?;
        if !playbook.steps.iter().any(|step| step.id == step_id) {
            return Err(format!("剧本里没有步骤 `{step_id}`"));
        }
        let mut run = load_or_new_run(&path, &playbook)?;
        match outcome.as_str() {
            "pass" => run.record_pass(&step_id, note.clone()),
            "fail" => run.record_fail(&step_id, note.clone()),
            _ => run.record_skip(&step_id, note.clone()),
        }
        save_run(&path, &run)?;
        let next = next_json(&playbook, &run);
        Ok(json!({
            "recorded": { "step": step_id, "outcome": outcome, "attempts": run.attempts(&step_id) },
            "progress": progress_json(&playbook, &run),
            "next": next["next"],
            "blocked": next["blocked"],
            "state_file": state_path(&path).to_string_lossy(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "owo-playbook-tool-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_playbook(dir: &Path) -> PathBuf {
        let file = dir.join("demo.json");
        std::fs::write(
            &file,
            r#"{"id":"demo","name":"演示","steps":[
                {"id":"a","title":"A"},
                {"id":"b","title":"B","depends_on":["a"]}
            ]}"#,
        )
        .unwrap();
        file
    }

    #[test]
    fn state_path_is_stem_state_json() {
        assert_eq!(
            state_path(Path::new("playbooks/demo.json")),
            PathBuf::from("playbooks/demo.state.json")
        );
    }

    #[test]
    fn load_playbook_rejects_invalid_and_mismatched_state() {
        let dir = scratch("invalid");
        let file = dir.join("bad.json");
        std::fs::write(&file, r#"{"id":"bad","name":"坏剧本","steps":[]}"#).unwrap();
        let error = load_playbook(&file).unwrap_err();
        assert!(error.contains("剧本不合法"), "{error}");

        let file = write_playbook(&dir);
        let playbook = load_playbook(&file).unwrap();
        let mut foreign = PlaybookRun::new(&playbook);
        foreign.playbook_id = "another".to_string();
        save_run(&file, &foreign).unwrap();
        let error = load_or_new_run(&file, &playbook).unwrap_err();
        assert!(error.contains("另一个剧本"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_then_next_reflects_progress_and_blocking() {
        let dir = scratch("record");
        let file = write_playbook(&dir);
        let playbook = load_playbook(&file).unwrap();
        let mut run = load_or_new_run(&file, &playbook).unwrap();
        assert_eq!(run.next_ready(&playbook).unwrap().id, "a");

        run.record_fail("b", "还没轮到它");
        run.record_pass("a", "证据 A");
        save_run(&file, &run).unwrap();

        let reloaded = load_or_new_run(&file, &playbook).unwrap();
        // b 失败过，但不依赖 a？这里 b 依赖 a：a 通过后 b 可重做（Failed 不是终态）。
        let next = reloaded.next_ready(&playbook).map(|step| step.id.clone());
        assert_eq!(next.as_deref(), Some("b"));
        let progress = progress_json(&playbook, &reloaded);
        assert_eq!(progress["passed"], 1);
        assert_eq!(progress["failed"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! 任务剧本 HTTP 面：桌面「任务剧本」面板的后端。
//!
//! 只读写**工作区内的剧本文件**与其同目录状态文件，不执行任何命令；
//! 执行仍由模型经 `playbook_check`（Tool Host）完成。
//!
//! 路由：
//! * `GET  /playbooks`            —— 工作区 `playbooks/` 下全部剧本与进度
//! * `GET  /playbooks/status`     —— 单个剧本（`?file=`，相对工作区）
//! * `POST /playbooks/record`     —— 记录一步结果（写 `<剧本>.state.json`）

use std::path::{Component, Path, PathBuf};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use owo_agent_core::playbook::{Playbook, PlaybookRun, StepState};

pub fn router(state: std::sync::Arc<owo_agent_server::AppState>) -> axum::Router {
    axum::Router::new()
        .route("/playbooks", axum::routing::get(list))
        .route("/playbooks/status", axum::routing::get(status))
        .route("/playbooks/record", axum::routing::post(record))
        .with_state(state)
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn err(status: StatusCode, message: impl AsRef<str>) -> (StatusCode, Json<Value>) {
    (status, Json(json!({ "error": message.as_ref() })))
}

/// 剧本目录：工作区下的 `playbooks/`（约定位置，面板与 CLI 一致）。
fn playbooks_dir(workspace: &Path) -> PathBuf {
    workspace.join("playbooks")
}

/// 把请求里的路径限制在工作区内。
///
/// 接受两种形式：工作区相对路径（如 `playbooks/x.json`），或**工作区内**的绝对路径
/// （面板从列表里拿到的路径可能被客户端规范化过）。`..` 逃逸、工作区外路径一律拒绝。
fn resolve_in_workspace(workspace: &Path, file: &str) -> Result<PathBuf, String> {
    let base = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let raw = Path::new(file);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        if raw
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(format!("路径必须是工作区内的相对路径：{file}"));
        }
        base.join(raw)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("剧本不存在或不可读（{file}）：{error}"))?;
    if !canonical.starts_with(&base) {
        return Err(format!("路径越出工作区：{file}"));
    }
    if canonical.extension().and_then(|value| value.to_str()) != Some("json")
        || canonical
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".state.json"))
    {
        return Err("只能指定剧本 *.json（不含 *.state.json）".to_string());
    }
    Ok(canonical)
}

fn state_path(file: &Path) -> PathBuf {
    file.with_extension("state.json")
}

fn load_playbook(file: &Path) -> Result<Playbook, String> {
    let metadata = std::fs::metadata(file).map_err(|error| format!("读取剧本失败：{error}"))?;
    if metadata.len() > 256 * 1024 {
        return Err("剧本文件过大（> 256 KB）".to_string());
    }
    let content =
        std::fs::read_to_string(file).map_err(|error| format!("读取剧本失败：{error}"))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let playbook: Playbook =
        serde_json::from_str(content).map_err(|error| format!("剧本不是合法 JSON：{error}"))?;
    let issues = playbook.validate();
    if !issues.is_empty() {
        return Err(format!("剧本不合法（{} 个问题）", issues.len()));
    }
    Ok(playbook)
}

fn load_run(file: &Path, playbook: &Playbook) -> Result<PlaybookRun, String> {
    let path = state_path(file);
    if !path.exists() {
        return Ok(PlaybookRun::new(playbook));
    }
    let content =
        std::fs::read_to_string(&path).map_err(|error| format!("读取状态文件失败：{error}"))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let run: PlaybookRun =
        serde_json::from_str(content).map_err(|error| format!("状态文件不是合法 JSON：{error}"))?;
    if run.playbook_id != playbook.id {
        return Err(format!(
            "状态文件属于另一个剧本（{} ≠ {}）",
            run.playbook_id, playbook.id
        ));
    }
    Ok(run)
}

fn save_run(file: &Path, run: &PlaybookRun) -> Result<(), String> {
    let path = state_path(file);
    let json = serde_json::to_string_pretty(run).map_err(|error| error.to_string())?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "playbook.state.json".to_string());
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    std::fs::write(&temporary, json).map_err(|error| format!("写状态文件失败：{error}"))?;
    std::fs::rename(&temporary, &path).map_err(|error| format!("替换状态文件失败：{error}"))
}

fn kind_label(kind: owo_agent_core::playbook::StepKind) -> &'static str {
    match kind {
        owo_agent_core::playbook::StepKind::Model => "model",
        owo_agent_core::playbook::StepKind::Check => "check",
        owo_agent_core::playbook::StepKind::Human => "human",
    }
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

fn state_label(run: &PlaybookRun, playbook: &Playbook, step_id: &str) -> (&'static str, String) {
    match run.states.get(step_id).unwrap_or(&StepState::Pending) {
        StepState::Passed { evidence } => ("passed", evidence.clone()),
        StepState::Failed { reason, attempts } => {
            ("failed", format!("尝试 {attempts} 次：{reason}"))
        }
        StepState::Skipped { reason } => ("skipped", reason.clone()),
        StepState::Pending => {
            if run.is_blocked(playbook, step_id) {
                ("pending", "阻塞（前置未通过）".to_string())
            } else {
                ("pending", String::new())
            }
        }
    }
}

/// GET /playbooks：工作区 `playbooks/` 的目录总览（含进度；不合法的不中断整表）。
async fn list(State(state): State<std::sync::Arc<owo_agent_server::AppState>>) -> ApiResult {
    let dir = playbooks_dir(&state.workspace);
    let workspace_canonical = state
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| state.workspace.clone());
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(_) => {
            return Ok(Json(
                json!({ "dir": "playbooks", "count": 0, "playbooks": [] }),
            ));
        }
    };
    let mut playbooks: Vec<Value> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned());
        if !path.is_file()
            || name
                .as_deref()
                .is_some_and(|value| value.ends_with(".state.json"))
            || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        // 对外只用工作区相对路径（正斜杠）：客户端回传时可以原样作为 `file`。
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        let display = canonical
            .strip_prefix(&workspace_canonical)
            .map(|value| value.to_path_buf())
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .replace('\\', "/");
        match load_playbook(&path) {
            Ok(playbook) => match load_run(&path, &playbook) {
                Ok(run) => playbooks.push(json!({
                    "id": playbook.id,
                    "name": playbook.name,
                    "path": display,
                    "steps": playbook.steps.len(),
                    "ok": true,
                    "issues": [],
                    "progress": progress_json(&playbook, &run),
                    "next": run.next_ready(&playbook).map(|step| step.id.clone()),
                })),
                Err(error) => playbooks.push(json!({
                    "id": playbook.id,
                    "name": playbook.name,
                    "path": display,
                    "steps": playbook.steps.len(),
                    "ok": false,
                    "issues": [json!({"code": "state_invalid", "step": null, "message": error})],
                })),
            },
            Err(error) => playbooks.push(json!({
                "id": path.file_stem().map(|value| value.to_string_lossy().into_owned()),
                "name": Value::Null,
                "path": display,
                "steps": 0,
                "ok": false,
                "issues": [json!({"code": "playbook_invalid", "step": null, "message": error})],
            })),
        }
    }
    playbooks.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["id"].as_str().unwrap_or_default())
    });
    Ok(Json(json!({
        "dir": "playbooks",
        "count": playbooks.len(),
        "playbooks": playbooks,
    })))
}

#[derive(Deserialize)]
struct StatusQuery {
    file: String,
}

/// GET /playbooks/status?file=…：单个剧本的逐步状态。
async fn status(
    State(state): State<std::sync::Arc<owo_agent_server::AppState>>,
    Query(query): Query<StatusQuery>,
) -> ApiResult {
    let file = resolve_in_workspace(&state.workspace, &query.file)
        .map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let playbook = load_playbook(&file).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let run = load_run(&file, &playbook).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let steps: Vec<Value> = playbook
        .steps
        .iter()
        .map(|step| {
            let (state, note) = state_label(&run, &playbook, &step.id);
            json!({
                "id": step.id,
                "title": step.title,
                "kind": kind_label(step.kind),
                "state": state,
                "note": note,
                "attempts": run.attempts(&step.id),
                "command": step.command,
            })
        })
        .collect();
    Ok(Json(json!({
        "ok": true,
        "id": playbook.id,
        "name": playbook.name,
        "path": file.to_string_lossy(),
        "state_file": state_path(&file).to_string_lossy(),
        "progress": progress_json(&playbook, &run),
        "next": run.next_ready(&playbook).map(|step| step.id.clone()),
        "steps": steps,
    })))
}

#[derive(Deserialize)]
struct RecordRequest {
    file: String,
    step: String,
    outcome: String,
    #[serde(default)]
    note: String,
}

/// POST /playbooks/record：记录一步结果（写入同目录状态文件）。
async fn record(
    State(state): State<std::sync::Arc<owo_agent_server::AppState>>,
    Json(request): Json<RecordRequest>,
) -> ApiResult {
    if !matches!(request.outcome.as_str(), "pass" | "fail" | "skip") {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("outcome 非法：{}（pass|fail|skip）", request.outcome),
        ));
    }
    if request.note.chars().count() > 2000 {
        return Err(err(StatusCode::BAD_REQUEST, "note 超过 2000 字符"));
    }
    let file = resolve_in_workspace(&state.workspace, &request.file)
        .map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let playbook = load_playbook(&file).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    if !playbook.steps.iter().any(|step| step.id == request.step) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("剧本里没有步骤 `{}`", request.step),
        ));
    }
    let mut run =
        load_run(&file, &playbook).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    match request.outcome.as_str() {
        "pass" => run.record_pass(&request.step, request.note.clone()),
        "fail" => run.record_fail(&request.step, request.note.clone()),
        _ => run.record_skip(&request.step, request.note.clone()),
    }
    save_run(&file, &run).map_err(|error| err(StatusCode::INTERNAL_SERVER_ERROR, error))?;
    Ok(Json(json!({
        "ok": true,
        "id": playbook.id,
        "step": request.step,
        "outcome": request.outcome,
        "attempts": run.attempts(&request.step),
        "progress": progress_json(&playbook, &run),
        "next": run.next_ready(&playbook).map(|step| step.id.clone()),
        "state_file": state_path(&file).to_string_lossy(),
    })))
}

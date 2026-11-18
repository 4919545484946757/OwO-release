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
        .route("/playbooks/history", axum::routing::get(history))
        .route("/playbooks/record", axum::routing::post(record))
        .route("/playbooks/archive", axum::routing::post(archive))
        .route("/playbooks/init", axum::routing::post(init))
        .route("/playbooks/reset", axum::routing::post(reset))
        .with_state(state)
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

/// 单次列表/归档读取上限（避免超大目录造成读放大）。
const MAX_PLAYBOOK_FILES: usize = 500;

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
        StepState::Failed {
            reason, attempts, ..
        } => ("failed", format!("尝试 {attempts} 次：{reason}")),
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
    // 先过滤候选、按文件名排序再截断：截断子集确定，"前 500 个"与目录顺序无关。
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.ends_with(".state.json"))
        })
        .collect();
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let truncated = files.len() > MAX_PLAYBOOK_FILES;
    files.truncate(MAX_PLAYBOOK_FILES);
    for path in files {
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
                    "description": playbook.description,
                    "warnings": playbook.risk_warnings().len(),
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
                    "description": playbook.description,
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
        "truncated": truncated,
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
            let warnings: Vec<String> = playbook
                .risk_warnings()
                .into_iter()
                .filter(|warning| warning.step.as_deref() == Some(step.id.as_str()))
                .map(|warning| warning.message)
                .collect();
            json!({
                "id": step.id,
                "title": step.title,
                "kind": kind_label(step.kind),
                "state": state,
                "note": note,
                "attempts": run.attempts(&step.id),
                "failure_kind": run.failure_kind(&step.id),
                "max_attempts": step.max_attempts,
                "command": step.command,
                "warnings": warnings,
            })
        })
        .collect();
    Ok(Json(json!({
        "ok": true,
        "id": playbook.id,
        "name": playbook.name,
        "description": playbook.description,
        "path": file.to_string_lossy(),
        "state_file": state_path(&file).to_string_lossy(),
        "progress": progress_json(&playbook, &run),
        "next": run.next_ready(&playbook).map(|step| step.id.clone()),
        "steps": steps,
    })))
}

/// GET /playbooks/history?file=…：归档列表（含每步状态，UI 可本地对比两次运行）。
async fn history(
    State(state): State<std::sync::Arc<owo_agent_server::AppState>>,
    Query(query): Query<StatusQuery>,
) -> ApiResult {
    let file = resolve_in_workspace(&state.workspace, &query.file)
        .map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let playbook = load_playbook(&file).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let dir = file.with_extension("runs");
    let mut runs: Vec<Value> = Vec::new();
    let mut truncated = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        // 与列表一致：先过滤、按文件名排序、再截断，保证保留子集确定。
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect();
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        truncated = files.len() > MAX_PLAYBOOK_FILES;
        files.truncate(MAX_PLAYBOOK_FILES);
        for path in files {
            let marker = path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(run) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|content| {
                    // 容忍 UTF-8 BOM（Windows 编辑器保存的归档）。
                    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
                    serde_json::from_str::<PlaybookRun>(content).ok()
                })
                .filter(|run| run.playbook_id == playbook.id)
            else {
                continue;
            };
            let mut steps = serde_json::Map::new();
            let mut step_kinds = serde_json::Map::new();
            for step in &playbook.steps {
                let (label, _note) = state_label(&run, &playbook, &step.id);
                steps.insert(step.id.clone(), json!(label));
                if let Some(kind) = run.failure_kind(&step.id) {
                    step_kinds.insert(step.id.clone(), json!(kind));
                }
            }
            runs.push(json!({
                "marker": marker,
                "progress": progress_json(&playbook, &run),
                "complete": run.is_complete(&playbook),
                "steps": steps,
                "step_kinds": step_kinds,
            }));
        }
    }
    runs.sort_by(|left, right| {
        right["marker"]
            .as_str()
            .unwrap_or_default()
            .cmp(left["marker"].as_str().unwrap_or_default())
    });
    Ok(Json(json!({
        "ok": true,
        "id": playbook.id,
        "name": playbook.name,
        "count": runs.len(),
        "truncated": truncated,
        "runs": runs,
    })))
}

#[derive(Deserialize)]
struct ArchiveRequest {
    file: String,
    #[serde(default)]
    marker: Option<String>,
}

fn validate_marker(marker: &str) -> Result<(), String> {
    if marker.is_empty() || marker.chars().count() > 64 {
        return Err("归档标记长度必须在 1–64 之间".to_string());
    }
    if !marker
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err("归档标记只允许字母、数字与 . - _".to_string());
    }
    if marker.starts_with('.') || marker.contains("..") {
        return Err("归档标记不能以 . 开头或包含 ..".to_string());
    }
    Ok(())
}

/// POST /playbooks/archive {file, marker?}：把当前状态快照到 `<剧本>.runs/`。
///
/// 面板"保存快照"用；默认标记 `ui-<epoch 毫秒>`。只复制状态文件，不改进度。
async fn archive(
    State(state): State<std::sync::Arc<owo_agent_server::AppState>>,
    Json(request): Json<ArchiveRequest>,
) -> ApiResult {
    let file = resolve_in_workspace(&state.workspace, &request.file)
        .map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let playbook = load_playbook(&file).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let run = load_run(&file, &playbook).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let _ = run;
    let state_file = state_path(&file);
    if !state_file.exists() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "还没有运行状态：先记录一步再归档",
        ));
    }
    let marker = request.marker.unwrap_or_else(|| {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        format!("ui-{millis}")
    });
    validate_marker(&marker).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let dir = file.with_extension("runs");
    std::fs::create_dir_all(&dir).map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建归档目录失败：{error}"),
        )
    })?;
    let target = dir.join(format!("{marker}.json"));
    if target.exists() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("归档 `{marker}` 已存在"),
        ));
    }
    std::fs::copy(&state_file, &target).map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入归档失败：{error}"),
        )
    })?;
    Ok(Json(json!({
        "ok": true,
        "id": playbook.id,
        "marker": marker,
        "archive": target.to_string_lossy(),
    })))
}

#[derive(Deserialize)]
struct InitRequest {
    id: String,
}

fn validate_new_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.chars().count() > 64 {
        return Err("剧本 id 长度必须在 1–64 之间".to_string());
    }
    if !id.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err("剧本 id 只允许小写字母、数字与连字符".to_string());
    }
    let first = id.chars().next().unwrap_or(' ');
    let last = id.chars().last().unwrap_or(' ');
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err("剧本 id 必须以字母或数字开头/结尾".to_string());
    }
    Ok(())
}

/// POST /playbooks/init {id}：在工作区 `playbooks/` 新建三步骨架。
async fn init(
    State(state): State<std::sync::Arc<owo_agent_server::AppState>>,
    Json(request): Json<InitRequest>,
) -> ApiResult {
    validate_new_id(request.id.trim()).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let id = request.id.trim();
    let dir = playbooks_dir(&state.workspace);
    std::fs::create_dir_all(&dir).map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建目录失败：{error}"),
        )
    })?;
    let path = dir.join(format!("{id}.json"));
    if path.exists() {
        return Err(err(StatusCode::BAD_REQUEST, format!("剧本 `{id}` 已存在")));
    }
    let skeleton = json!({
        "id": id,
        "name": id,
        "description": "TODO：说明这类任务的输入、产出与边界",
        "steps": [
            {
                "id": "plan",
                "title": "确认任务范围",
                "kind": "model",
                "detail": "读输入、确认产出与不做的事。",
                "acceptance": "列出输入、产出与不做的事"
            },
            {
                "id": "check",
                "title": "可自动校验的检查",
                "kind": "check",
                "detail": "能写成一条「退出码即结论」的命令就填 command；留空则人工判定。",
                "acceptance": "TODO：写可判定的判据",
                "depends_on": ["plan"]
            },
            {
                "id": "deliver",
                "title": "评审与交付",
                "kind": "human",
                "detail": "汇总证据，交给人工确认。",
                "acceptance": "人工确认结果",
                "depends_on": ["check"]
            }
        ]
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&skeleton).map_err(|error| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("序列化失败：{error}"),
            )
        })?,
    )
    .map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入失败：{error}"),
        )
    })?;
    Ok(Json(json!({
        "ok": true,
        "id": id,
        "path": format!("playbooks/{id}.json"),
        "steps": 3,
    })))
}

#[derive(Deserialize)]
struct ResetRequest {
    file: String,
}

/// POST /playbooks/reset {file}：重置进度。
///
/// 破坏性操作的安全网：删除前先把手头状态快照到 `<剧本>.runs/reset-<时间戳>.json`，
/// 同时清除派发日志（否则"结果未知"会一直挡着重跑）。
async fn reset(
    State(state): State<std::sync::Arc<owo_agent_server::AppState>>,
    Json(request): Json<ResetRequest>,
) -> ApiResult {
    let file = resolve_in_workspace(&state.workspace, &request.file)
        .map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let playbook = load_playbook(&file).map_err(|error| err(StatusCode::BAD_REQUEST, error))?;
    let state_file = state_path(&file);
    if !state_file.exists() {
        return Err(err(StatusCode::BAD_REQUEST, "还没有运行状态，无需重置"));
    }
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let marker = format!("reset-{millis}");
    let dir = file.with_extension("runs");
    std::fs::create_dir_all(&dir).map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("创建归档目录失败：{error}"),
        )
    })?;
    let snapshot = dir.join(format!("{marker}.json"));
    std::fs::copy(&state_file, &snapshot).map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("保存快照失败：{error}"),
        )
    })?;
    std::fs::remove_file(&state_file).map_err(|error| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("删除状态失败：{error}"),
        )
    })?;
    let journal = file.with_extension("journal.jsonl");
    if journal.exists() {
        let _ = std::fs::remove_file(&journal);
    }
    Ok(Json(json!({
        "ok": true,
        "id": playbook.id,
        "snapshot": marker,
        "state_file": state_file.to_string_lossy(),
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

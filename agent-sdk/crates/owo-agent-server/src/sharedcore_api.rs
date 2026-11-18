//! SharedCore 桌面看板数据面（只读）。
//!
//! `GET /sharedcore/state` 读取工作区 `.sharedcore/` 下的项目上下文、任务、契约与最近事件，
//! 并计算依赖批次与写范围冲突。**不写任何文件、不执行命令**；状态由插件工具维护。

use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::Arc;

/// 单个状态文件读取上限（project/tasks/contracts 都是小 JSON）。
const MAX_STATE_FILE_BYTES: u64 = 512 * 1024;
const MAX_EVENTS: usize = 30;
const MAX_TASKS: usize = 500;

pub fn router(state: Arc<owo_agent_server::AppState>) -> axum::Router {
    axum::Router::new()
        .route("/sharedcore/state", get(state_handler))
        .with_state(state)
}

async fn state_handler(State(state): State<Arc<owo_agent_server::AppState>>) -> Json<Value> {
    Json(build_state(&state.workspace))
}

fn read_json_file(path: &Path) -> Option<Value> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_STATE_FILE_BYTES {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    serde_json::from_str(content).ok()
}

fn read_events(path: &Path, limit: usize) -> Vec<Value> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(limit);
    lines[start..]
        .iter()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

fn build_state(workspace: &Path) -> Value {
    let dir = workspace.join(".sharedcore");
    let Some(project) = read_json_file(&dir.join("project.json")) else {
        return json!({
            "ok": true,
            "exists": false,
            "dir": ".sharedcore",
            "hint": "启用 owo-sharedcore 插件后，让 Agent 调用 sharedcore_init 建立项目上下文",
        });
    };
    let tasks_payload = read_json_file(&dir.join("tasks.json")).unwrap_or_else(|| json!({}));
    let tasks = tasks_payload
        .get("tasks")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let contracts = read_json_file(&dir.join("contracts.json")).unwrap_or_else(|| json!({}));
    let contracts = contracts
        .get("contracts")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let events = read_events(&dir.join("events.jsonl"), MAX_EVENTS);
    let plan = plan_for(&tasks);
    json!({
        "ok": true,
        "exists": true,
        "project": project,
        "tasks": tasks,
        "plan": plan,
        "contracts": contracts,
        "events": events,
    })
}

/// 依赖满足 + 写范围不冲突的并行批次（与插件的 `sharedcore_plan` 语义一致）。
fn plan_for(tasks: &Value) -> Value {
    let Some(map) = tasks.as_object() else {
        return json!({ "ready": [], "batches": [], "blocked": {}, "conflicts": [] });
    };
    let done: std::collections::HashSet<String> = map
        .iter()
        .filter(|(_, task)| task.get("status").and_then(Value::as_str) == Some("done"))
        .map(|(id, _)| id.clone())
        .collect();
    let mut ready: Vec<(String, Value)> = Vec::new();
    let mut blocked = Map::new();
    for (id, task) in map {
        let status = task.get("status").and_then(Value::as_str).unwrap_or("");
        if matches!(
            status,
            "done" | "skipped" | "failed" | "cancelled" | "running"
        ) {
            continue;
        }
        let missing: Vec<String> = task
            .get("depends_on")
            .and_then(Value::as_array)
            .map(|dependencies| {
                dependencies
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|dependency| !done.contains(*dependency))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if missing.is_empty() {
            ready.push((id.clone(), task.clone()));
        } else {
            blocked.insert(id.clone(), json!(missing));
        }
    }
    ready.sort_by(|(left_id, left), (right_id, right)| {
        let priority = |task: &Value| task.get("priority").and_then(Value::as_i64).unwrap_or(0);
        priority(right)
            .cmp(&priority(left))
            .then(left_id.cmp(right_id))
    });
    let ready_ids: Vec<String> = ready.iter().map(|(id, _)| id.clone()).collect();
    let ready: Vec<(String, Value)> = ready.into_iter().take(MAX_TASKS).collect();
    let mut batches: Vec<Vec<String>> = Vec::new();
    let mut conflicts: Vec<Value> = Vec::new();
    let mut remaining = ready;
    while !remaining.is_empty() {
        let mut batch: Vec<(String, Value)> = Vec::new();
        let mut deferred: Vec<(String, Value)> = Vec::new();
        for (id, task) in remaining.drain(..) {
            let scopes = scopes_of(&task);
            let overlaps: Vec<String> = batch
                .iter()
                .filter(|(_, chosen)| scopes_overlap(&scopes, &scopes_of(chosen)))
                .map(|(chosen_id, _)| chosen_id.clone())
                .collect();
            if overlaps.is_empty() {
                batch.push((id, task));
            } else {
                conflicts.push(json!({ "task": id, "overlaps": overlaps }));
                deferred.push((id, task));
            }
        }
        if batch.is_empty() {
            // 全部互相冲突：逐批串行，避免无进展。
            let (id, task) = deferred.remove(0);
            batch.push((id, task));
        }
        batches.push(batch.into_iter().map(|(id, _)| id).collect());
        remaining = deferred;
    }
    json!({
        "ready": ready_ids,
        "ready_shown": batches.len(),
        "batches": batches,
        "blocked": blocked,
        "conflicts": conflicts,
    })
}

fn scopes_of(task: &Value) -> Vec<String> {
    task.get("write_scope")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(normalize_scope)
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_scope(scope: &str) -> String {
    scope
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn scopes_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|first| {
        right.iter().any(|second| {
            !first.is_empty()
                && !second.is_empty()
                && (first == second
                    || first.starts_with(&format!("{second}/"))
                    || second.starts_with(&format!("{first}/")))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_batches_parallel_tasks_and_reports_scope_conflicts() {
        let tasks = json!({
            "T-001": { "id": "T-001", "status": "done", "depends_on": [] },
            "T-002": { "id": "T-002", "status": "ready", "depends_on": ["T-001"], "write_scope": ["src/backend/"] },
            "T-003": { "id": "T-003", "status": "ready", "depends_on": ["T-001"], "write_scope": ["src/frontend/"] },
            "T-004": { "id": "T-004", "status": "ready", "depends_on": ["T-002"], "write_scope": ["docs/"] },
            "T-005": { "id": "T-005", "status": "ready", "depends_on": [], "write_scope": ["src/backend/api/"] }
        });
        let plan = plan_for(&tasks);
        assert_eq!(plan["batches"][0], json!(["T-002", "T-003"]));
        assert!(plan["blocked"]["T-004"].is_array());
        assert!(plan["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["task"] == "T-005"));
    }
}

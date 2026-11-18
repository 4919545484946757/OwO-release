//! 执行工作区 HTTP 面（Agent V2 Phase 6）。
//!
//! 路由（bearer 保护面，前缀 `/workspaces`）：
//!
//! - `GET    /workspaces`                 列出工作区租约（含状态）
//! - `POST   /workspaces`                 创建租约（`git_worktree` 推荐）
//! - `GET    /workspaces/{id}`            租约详情
//! - `DELETE /workspaces/{id}`            释放（删除 worktree，不碰用户原工作区）
//! - `GET    /workspaces/{id}/diff`       unified diff 与文件清单
//! - `POST   /workspaces/{id}/apply`      把 Diff 应用到用户原工作区
//! - `POST   /workspaces/{id}/reject`     丢弃改动（已应用时先回滚）
//! - `POST   /workspaces/{id}/rollback`   反向应用已应用的 Diff
//!
//! 隔离本身在 `owo_agent_core::workspace_provider`；这里只做租约登记与 HTTP 映射。
//! 所有 Git 操作都在 `spawn_blocking` 里执行，避免阻塞 async 运行时。创建租约后把
//! `lease.path` 作为任务的 `workspace_ref` 提交，任务就会在该隔离目录里执行。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use owo_agent_core::workspace_provider::{
    CurrentWorkspaceProvider, GitWorktreeProvider, PrepareWorkspaceRequest,
    UnsupportedWorkspaceProvider, WorkspaceDiff, WorkspaceError, WorkspaceKind, WorkspaceLease,
    WorkspaceProvider, CODE_WORKSPACE_APPLY_CONFLICT, CODE_WORKSPACE_INVALID_STATE,
    CODE_WORKSPACE_NOT_FOUND, CODE_WORKSPACE_UNSUPPORTED,
};
use owo_agent_server::AppState;

type ApiResult<T> = Result<T, (StatusCode, Json<Value>)>;

fn workspace_error(error: WorkspaceError) -> (StatusCode, Json<Value>) {
    let status = match error.code {
        CODE_WORKSPACE_NOT_FOUND => StatusCode::NOT_FOUND,
        CODE_WORKSPACE_UNSUPPORTED => StatusCode::NOT_IMPLEMENTED,
        CODE_WORKSPACE_INVALID_STATE | CODE_WORKSPACE_APPLY_CONFLICT => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        Json(json!({ "ok": false, "code": error.code, "error": error.message })),
    )
}

fn join_error(error: tokio::task::JoinError) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(
            json!({ "ok": false, "code": "STORE_ERROR", "error": format!("工作区任务失败：{error}") }),
        ),
    )
}

/// 工作区租约登记表：id → 租约；提供者按 kind 现场构造（无状态）。
pub struct WorkspaceRegistry {
    worktree_root: PathBuf,
    workspace_root: PathBuf,
    leases: Mutex<std::collections::HashMap<String, WorkspaceLease>>,
}

impl WorkspaceRegistry {
    pub fn new(worktree_root: PathBuf, workspace_root: PathBuf) -> Self {
        Self {
            worktree_root,
            workspace_root,
            leases: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn provider(&self, kind: WorkspaceKind) -> Box<dyn WorkspaceProvider> {
        match kind {
            WorkspaceKind::CurrentWorkspace => {
                Box::new(CurrentWorkspaceProvider::new(self.workspace_root.clone()))
            }
            WorkspaceKind::GitWorktree => {
                Box::new(GitWorktreeProvider::new(self.worktree_root.clone()))
            }
            WorkspaceKind::LocalSandbox => Box::new(UnsupportedWorkspaceProvider::local_sandbox()),
            WorkspaceKind::RemoteWorkspace => {
                Box::new(UnsupportedWorkspaceProvider::remote_workspace())
            }
        }
    }

    fn leases(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, WorkspaceLease>> {
        self.leases
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn create(
        &self,
        kind: WorkspaceKind,
        task_id: Option<String>,
    ) -> Result<WorkspaceLease, WorkspaceError> {
        let workspace_id = format!("ws-{}", uuid::Uuid::new_v4());
        let provider = self.provider(kind);
        let lease = provider.prepare(&PrepareWorkspaceRequest {
            workspace_id: workspace_id.clone(),
            task_id,
            kind,
            base_root: self.workspace_root.clone(),
        })?;
        self.leases().insert(workspace_id, lease.clone());
        Ok(lease)
    }

    pub fn list(&self) -> Vec<WorkspaceLease> {
        let mut leases: Vec<WorkspaceLease> = self.leases().values().cloned().collect();
        leases.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        leases
    }

    pub fn get(&self, id: &str) -> Result<WorkspaceLease, WorkspaceError> {
        self.leases().get(id).cloned().ok_or_else(|| {
            WorkspaceError::new(CODE_WORKSPACE_NOT_FOUND, format!("工作区 {id} 不存在"))
        })
    }

    fn with_lease<T>(
        &self,
        id: &str,
        action: impl FnOnce(&dyn WorkspaceProvider, &mut WorkspaceLease) -> Result<T, WorkspaceError>,
    ) -> Result<T, WorkspaceError> {
        let mut leases = self.leases();
        let lease = leases.get_mut(id).ok_or_else(|| {
            WorkspaceError::new(CODE_WORKSPACE_NOT_FOUND, format!("工作区 {id} 不存在"))
        })?;
        let provider = self.provider(lease.kind);
        action(provider.as_ref(), lease)
    }

    pub fn diff(&self, id: &str) -> Result<WorkspaceDiff, WorkspaceError> {
        self.with_lease(id, |provider, lease| provider.diff(lease))
    }

    pub fn apply(&self, id: &str) -> Result<WorkspaceDiff, WorkspaceError> {
        self.with_lease(id, |provider, lease| provider.apply(lease))
    }

    pub fn reject(&self, id: &str) -> Result<WorkspaceLease, WorkspaceError> {
        self.with_lease(id, |provider, lease| {
            provider.reject(lease)?;
            Ok(lease.clone())
        })
    }

    pub fn rollback(&self, id: &str) -> Result<WorkspaceLease, WorkspaceError> {
        self.with_lease(id, |provider, lease| {
            provider.rollback(lease)?;
            Ok(lease.clone())
        })
    }

    pub fn release(&self, id: &str) -> Result<WorkspaceLease, WorkspaceError> {
        self.with_lease(id, |provider, lease| {
            provider.release(lease)?;
            Ok(lease.clone())
        })
    }
}

#[derive(Deserialize)]
struct CreateWorkspaceBody {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
}

async fn create_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateWorkspaceBody>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let kind_text = body.kind.as_deref().unwrap_or("git_worktree");
    let kind = WorkspaceKind::parse(kind_text).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "code": "INVALID_REQUEST",
                "error": format!("未知工作区类型：{kind_text}（可选 current_workspace / git_worktree / local_sandbox / remote_workspace）"),
            })),
        )
    })?;
    let registry = Arc::clone(&state.workspaces);
    let task_id = body.task_id.filter(|value| !value.trim().is_empty());
    let lease = tokio::task::spawn_blocking(move || registry.create(kind, task_id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "workspace": lease })),
    ))
}

async fn list_workspaces(State(state): State<Arc<AppState>>) -> Json<Value> {
    let leases = state.workspaces.list();
    Json(json!({ "ok": true, "count": leases.len(), "workspaces": leases }))
}

async fn get_workspace(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let registry = Arc::clone(&state.workspaces);
    let lease = tokio::task::spawn_blocking(move || registry.get(&id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok(Json(json!({ "ok": true, "workspace": lease })))
}

async fn workspace_diff(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let registry = Arc::clone(&state.workspaces);
    let diff = tokio::task::spawn_blocking(move || registry.diff(&id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok(Json(
        json!({ "ok": true, "diff": diff, "empty": diff.is_empty() }),
    ))
}

async fn apply_workspace(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let registry = Arc::clone(&state.workspaces);
    let diff = tokio::task::spawn_blocking(move || registry.apply(&id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok(Json(json!({ "ok": true, "applied": true, "diff": diff })))
}

async fn reject_workspace(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let registry = Arc::clone(&state.workspaces);
    let lease = tokio::task::spawn_blocking(move || registry.reject(&id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok(Json(json!({ "ok": true, "workspace": lease })))
}

async fn rollback_workspace(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let registry = Arc::clone(&state.workspaces);
    let lease = tokio::task::spawn_blocking(move || registry.rollback(&id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok(Json(json!({ "ok": true, "workspace": lease })))
}

async fn release_workspace(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<Value>> {
    let registry = Arc::clone(&state.workspaces);
    let lease = tokio::task::spawn_blocking(move || registry.release(&id))
        .await
        .map_err(join_error)?
        .map_err(workspace_error)?;
    Ok(Json(json!({ "ok": true, "workspace": lease })))
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/workspaces", get(list_workspaces).post(create_workspace))
        .route(
            "/workspaces/{id}",
            get(get_workspace).delete(release_workspace),
        )
        .route("/workspaces/{id}/diff", get(workspace_diff))
        .route("/workspaces/{id}/apply", post(apply_workspace))
        .route("/workspaces/{id}/reject", post(reject_workspace))
        .route("/workspaces/{id}/rollback", post(rollback_workspace))
        .with_state(state)
}

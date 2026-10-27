//! Artifact 评审闭环 HTTP 路由（V1 四期 · 第三路）。
//!
//! 路由面：
//! - `POST /artifacts/{id}/review`：提交评审（approve / request_changes / reject）；
//! - `GET  /artifacts/{id}/history`：评审历史 + 版本链 + approved head；
//! - `GET  /artifacts/{id}/content`：产物正文交付（JSON 信封 / `?raw=true` 原始下载流）；
//! - `GET  /artifacts/{id}/metadata`：交付元数据（格式/哈希/校验/证据链/交接）；
//! - `GET  /projects/{id}/delivery-manifest`：最终交付清单（每 kind 最新版本）。
//!
//! 契约要点：
//! - `expected_version` 乐观并发：与当前版本不符 → 409（旧页面提交被拒）；
//! - `idempotency_key` 幂等：同键重放零副作用（返回既有记录，`replayed: true`）；
//! - 生产者未经 Human 策略（`human_policy == "self_review_allowed"`）自批 → 403；
//! - 被新版本取代（superseded）的产物不可再批准 → 409；
//! - 幂等键已用于其他产物 → 409；
//! - 未知产物 / 未知团队 → 404。
//!
//! 存储：复用 WorkSwarm 的 `space.db`（独立 SQLite 连接 + busy_timeout，
//! 与 TeamCoordinator 连接共存；评审为低频操作，锁竞争在 2s 超时内自旋消化）。

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_core::project_space_store::{
    apply_artifact_review, self_approve_allowed, ArtifactReviewError, ArtifactReviewInput,
    ProjectSpaceStoreBackend, SqliteProjectSpaceStore,
};
use owo_agent_protocol::ArtifactReviewDecision;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use owo_agent_server::AppState;

// 返工与最终交付物（V1 五期 · 第二路）：独立文件，经本模块 router 合并挂载，
// 不改动 lib.rs 装配（子模块可访问本模块私有项：store 连接与错误映射）。
#[path = "artifact_rework_api.rs"]
pub(crate) mod artifact_rework;

// Artifact 校验、证据链与下载交付（七期 · 第三路）：独立文件，经本模块 router
// 合并挂载，不改动 lib.rs 装配（子模块可访问本模块私有项：store 连接与错误映射）。
#[path = "artifact_delivery_api.rs"]
pub(crate) mod artifact_delivery;

// 统一 Human Inbox（八期 · 第三路）：四类待办统一列表 + 领取/释放/直接处理。
// 独立文件经本模块 router 合并挂载（lib.rs 装配零改动）；子模块可访问本模块
// 私有项（store 连接）；resolve 分派复用既有领域能力（评审/人节点/retry）。
#[path = "human_inbox_api/mod.rs"]
pub(crate) mod human_inbox;
#[path = "human_inbox_store.rs"]
pub(crate) mod human_inbox_store;

#[path = "artifact_media.rs"]
pub(crate) mod artifact_media;

#[path = "input_assets.rs"]
pub(crate) mod input_assets;

// ---------------------------------------------------------------------------
// 状态（进程内单例连接，懒初始化）
// ---------------------------------------------------------------------------

/// 评审闭环存储状态：`data_root/workswarm/space.db` 的独立连接。
#[derive(Clone)]
pub struct ArtifactReviewState {
    db_path: PathBuf,
    store: Arc<OnceLock<Arc<SqliteProjectSpaceStore>>>,
}

impl ArtifactReviewState {
    pub fn new(db_path: PathBuf) -> Self {
        Self {
            db_path,
            store: Arc::new(OnceLock::new()),
        }
    }

    /// 获取（并懒初始化）存储连接。
    fn store(&self) -> Result<Arc<SqliteProjectSpaceStore>, (StatusCode, Json<Value>)> {
        if let Some(s) = self.store.get() {
            return Ok(Arc::clone(s));
        }
        if let Some(parent) = self.db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let store = SqliteProjectSpaceStore::open(&self.db_path).map_err(store_error)?;
        // 并发首用：set 失败者取先到者（幂等）。
        let _ = self.store.set(Arc::new(store));
        self.store
            .get()
            .map(Arc::clone)
            .ok_or_else(|| store_error_msg("评审存储初始化失败".to_string()))
    }
}

fn store_error(
    e: owo_agent_core::project_space_store::ProjectSpaceStoreError,
) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": format!("评审存储错误：{e}") })),
    )
}

fn store_error_msg(msg: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": msg })),
    )
}

// ---------------------------------------------------------------------------
// 路由
// ---------------------------------------------------------------------------

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/artifacts/{id}/review", post(submit_review))
        .route("/artifacts/{id}/history", get(artifact_history))
        .route(
            "/artifacts/{id}/rework",
            post(artifact_rework::submit_rework),
        )
        .route(
            "/projects/{id}/deliverables",
            get(artifact_rework::project_deliverables),
        )
        .route(
            "/artifacts/{id}/content",
            get(artifact_delivery::artifact_content),
        )
        .route(
            "/artifacts/{id}/metadata",
            get(artifact_delivery::artifact_metadata),
        )
        .route(
            "/projects/{id}/delivery-manifest",
            get(artifact_delivery::project_delivery_manifest),
        )
        .merge(human_inbox::router())
        .with_state(state)
}

// ---------------- handlers ----------------

/// POST /artifacts/{id}/review 请求体。
#[derive(Debug, Deserialize)]
struct ArtifactReviewHttpRequest {
    team_id: String,
    /// approve / request_changes / reject（snake_case）。
    decision: String,
    reviewer: String,
    #[serde(default)]
    comment: String,
    /// 乐观并发目标版本；缺省跳过版本校验。
    #[serde(default)]
    expected_version: Option<u32>,
    idempotency_key: String,
}

/// POST /artifacts/{id}/review：提交评审。
async fn submit_review(
    State(state): State<Arc<AppState>>,
    AxumPath(artifact_id): AxumPath<String>,
    Json(req): Json<ArtifactReviewHttpRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let store = state.artifact_review.store()?;

    // 决定枚举（400：未知决定）。
    let decision = match req.decision.as_str() {
        "approve" => ArtifactReviewDecision::Approve,
        "request_changes" => ArtifactReviewDecision::RequestChanges,
        "reject" => ArtifactReviewDecision::Reject,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!(
                    "未知 decision：{other}（支持 approve / request_changes / reject）"
                ) })),
            ));
        }
    };
    if req.team_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "team_id 不能为空" })),
        ));
    }

    // Human 策略：生产者自批需团队显式 self_review_allowed 授权。
    let self_authorized = match store.get_team_run(req.team_id.trim()).await {
        Ok(team) => self_approve_allowed(team.human_policy.as_deref()),
        Err(owo_agent_core::project_space_store::ProjectSpaceStoreError::NotFound(_)) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("团队不存在：{}", req.team_id.trim()) })),
            ));
        }
        Err(e) => return Err(store_error(e)),
    };

    let input = ArtifactReviewInput {
        artifact_id: artifact_id.clone(),
        team_id: req.team_id.trim().to_string(),
        decision,
        reviewer: req.reviewer.clone(),
        comment: req.comment,
        expected_version: req.expected_version,
        idempotency_key: req.idempotency_key,
        self_approve_authorized: self_authorized,
    };

    match apply_artifact_review(store.as_ref(), &input).await {
        Ok(outcome) => {
            let code = if outcome.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            Ok((
                code,
                Json(json!({
                    "replayed": outcome.replayed,
                    "review": outcome.review,
                    "artifact": outcome.artifact,
                    "approved_head": outcome.approved_head,
                })),
            ))
        }
        Err(ArtifactReviewError::ArtifactNotFound(id)) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("产物不存在：{id}") })),
        )),
        Err(ArtifactReviewError::VersionConflict { current, expected }) => Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("版本冲突：产物当前为 v{current}，提交基于 v{expected}"),
                "current_version": current,
                "expected_version": expected,
            })),
        )),
        Err(ArtifactReviewError::Forbidden(msg)) => {
            Err((StatusCode::FORBIDDEN, Json(json!({ "error": msg }))))
        }
        Err(ArtifactReviewError::Superseded(id)) => Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": format!("产物已被新版本取代，不能批准旧版：{id}") })),
        )),
        Err(ArtifactReviewError::IdempotencyConflict(other)) => Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("幂等键冲突：该键已用于其他产物（{other}）"),
            })),
        )),
        Err(ArtifactReviewError::Validation(msg)) => {
            Err((StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))))
        }
        Err(ArtifactReviewError::Store(e)) => Err(store_error(e)),
    }
}

/// GET /artifacts/{id}/history：评审历史 + 版本链两端 + approved head。
async fn artifact_history(
    State(state): State<Arc<AppState>>,
    AxumPath(artifact_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let store = state.artifact_review.store()?;
    let artifact = store
        .get_artifact(&artifact_id)
        .await
        .map_err(|e| match e {
            owo_agent_core::project_space_store::ProjectSpaceStoreError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("产物不存在：{artifact_id}") })),
            ),
            other => store_error(other),
        })?;
    let reviews = store
        .list_artifact_reviews(&artifact_id)
        .await
        .map_err(store_error)?;
    let project_id = store
        .get_artifact_project(&artifact_id)
        .await
        .map_err(store_error)?;

    // 版本链解析：向后（本版取代谁）取字段；向前（谁取代本版）扫同项目同 kind。
    let mut superseded_by_resolved: Option<String> = None;
    if let Ok(siblings) = store.list_artifacts_by_project(&project_id).await {
        for s in siblings {
            if s.supersedes_artifact_id.as_deref() == Some(artifact_id.as_str()) {
                superseded_by_resolved = Some(s.artifact_id);
                break;
            }
        }
    }
    let approved_head = store
        .get_approved_head(&project_id, &artifact.kind)
        .await
        .map_err(store_error)?;

    Ok(Json(json!({
        "artifact_id": artifact.artifact_id,
        "kind": artifact.kind,
        "version": artifact.version,
        "producer": artifact.producer,
        "review_state": artifact.review_state,
        "supersedes_artifact_id": artifact.supersedes_artifact_id,
        "superseded_by": superseded_by_resolved,
        "reviews": reviews,
        "approved_head": approved_head,
    })))
}

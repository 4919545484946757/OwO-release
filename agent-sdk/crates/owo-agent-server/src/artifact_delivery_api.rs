//! Artifact 下载交付 HTTP 路由（七期 · 第三路：Artifact 校验、证据链与下载交付）。
//!
//! 路由面（经 [`crate::artifact_review_api::router`] 合并挂载）：
//! - `GET /artifacts/{id}/content`：产物正文交付。默认 JSON 信封（冻结契约：
//!   `artifact_id/format/sha256/size_bytes/content`）；`?raw=true` 时返回原始字节
//!   下载流（`Content-Type: <media_type>`; textual bodies retain UTF-8 charset +
//!   `Content-Disposition: attachment; filename="<file_name>"`）；
//! - `GET /artifacts/{id}/metadata`：交付元数据（冻结契约：`artifact_id/team_id/
//!   kind/format/version/sha256/size_bytes/validation/evidence_refs`，可选 `handoff`
//!   ——Worker 交接原文，仅提供时出现）；
//! - `GET /projects/{id}/delivery-manifest`：最终交付清单（冻结契约：
//!   `project_id/generated_at/manifest[{artifact_id,kind,format,version,sha256,
//!   size_bytes,approved,content_url}]`；每 kind 取最高版本，评审结论（kind=review）
//!   为过程产物不入清单；`content_url` 为 content 端点相对路径）。
//!
//! 契约要点：
//! - 哈希/字节数解析：七期 additive 字段优先（`sha256`/`size_bytes`），legacy 记录
//!   回退 `content_ref`（`cas://sha256:{hash}`）与 CAS 实际字节数；
//! - metadata `team_id`：additive 字段优先，legacy 记录回退 artifact_id 首段
//!   （artifact_id 形如 `{team_id}:{role}:v{n}`）；
//! - 校验结果 `validation`（`{format, valid, reason?}`）与证据引用 `evidence_refs[]`
//!   原样透出（登记前门控产物，七期第三路 core 侧产出）；
//! - 未知产物 / 未知项目 / CAS 内容缺失 → 404。
//!
//! 存储：复用 WorkSwarm 的 `space.db` 与 `cas/`（与 TeamCoordinator 同一物理目录）。

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use owo_agent_core::project_space_store::{ProjectSpaceStoreBackend, ProjectSpaceStoreError};
use owo_agent_core::CasStore;
use owo_agent_protocol::{Artifact, ReviewState};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use super::{store_error, store_error_msg};
use crate::AppState;

// ---------------------------------------------------------------------------
// 公共辅助
// ---------------------------------------------------------------------------

/// CAS 内容库目录（与 TeamCoordinator 同一物理目录：`data_root/workswarm/cas`）。
fn cas_store(state: &AppState) -> Result<CasStore, (StatusCode, Json<Value>)> {
    CasStore::new(state.data_root.join("workswarm").join("cas"))
        .map_err(|e| store_error_msg(format!("CAS 内容库不可用：{e}")))
}

/// 产物 CAS 哈希（七期 additive 字段优先；legacy 记录回退 content_ref 引用解析）。
fn artifact_hash(artifact: &Artifact) -> String {
    if !artifact.sha256.is_empty() {
        artifact.sha256.clone()
    } else {
        artifact
            .content_ref
            .strip_prefix("cas://sha256:")
            .map(str::to_string)
            .unwrap_or_default()
    }
}

/// 产物字节数（七期 additive 字段优先；legacy 记录由 CAS 实际内容回退计算）。
fn artifact_size(artifact: &Artifact, cas: &CasStore, hash: &str) -> u64 {
    if artifact.size_bytes > 0 {
        return artifact.size_bytes;
    }
    cas.get(hash).map(|b| b.len() as u64).unwrap_or(0)
}

/// 产物 lookup + 404 映射（三个端点共用）。
async fn load_artifact(
    store: &owo_agent_core::project_space_store::SqliteProjectSpaceStore,
    artifact_id: &str,
) -> Result<Artifact, (StatusCode, Json<Value>)> {
    store.get_artifact(artifact_id).await.map_err(|e| match e {
        ProjectSpaceStoreError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("产物不存在：{artifact_id}") })),
        ),
        other => store_error(other),
    })
}

// ---------------------------------------------------------------------------
// GET /artifacts/{id}/content
// ---------------------------------------------------------------------------

/// content 端点查询参数。
/// 四路集成注：经 artifact_review_api::router 以 `get(artifact_delivery::artifact_content)`
/// 挂载，Handler 签名中的 Query 类型必须跨模块可见，故 pub(crate)（否则 `type is private` 编译错）。
#[derive(Debug, Deserialize)]
pub(crate) struct ContentQuery {
    /// `true` 时返回原始字节下载流（正确文件名 + media type）。
    #[serde(default)]
    raw: bool,
}

/// GET /artifacts/{id}/content：产物正文交付（JSON 信封；`?raw=true` 原始下载流）。
pub(crate) async fn artifact_content(
    State(state): State<Arc<AppState>>,
    AxumPath(artifact_id): AxumPath<String>,
    Query(query): Query<ContentQuery>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let store = state.artifact_review.store()?;
    let artifact = load_artifact(store.as_ref(), &artifact_id).await?;

    let hash = artifact_hash(&artifact);
    if hash.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!(
                "产物内容引用缺失（artifact={artifact_id}，content_ref={}）",
                artifact.content_ref
            ) })),
        ));
    }
    let cas = cas_store(&state)?;
    let bytes = cas.get(&hash).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("产物内容缺失（CAS 对象 {hash} 不存在）") })),
        )
    })?;
    let size_bytes = artifact_size(&artifact, &cas, &hash);
    let media_type = if artifact.media_type.is_empty() {
        "text/plain"
    } else {
        artifact.media_type.as_str()
    };
    let file_name = if artifact.file_name.is_empty() {
        owo_agent_core::artifact_pipeline::file_name_of(&artifact.kind, &artifact.format)
    } else {
        artifact.file_name.clone()
    };

    if query.raw {
        return Ok(raw_response(
            bytes,
            media_type,
            &file_name,
            headers.get(header::RANGE),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(json!({
            // 冻结契约（七期四路冻结，字段名固定）
            "artifact_id": artifact.artifact_id,
            "format": artifact.format,
            "sha256": hash,
            "size_bytes": size_bytes,
            "content": String::from_utf8_lossy(&bytes),
            // additive：交付元数据与证据链（UI 宽容读取）
            "kind": artifact.kind,
            "version": artifact.version,
            "media_type": artifact.media_type,
            "file_name": file_name,
            "evidence_refs": artifact.evidence_refs,
            "open_issues": artifact.open_issues,
            "validation": artifact.validation,
            "handoff": artifact.handoff,
        })),
    )
        .into_response())
}

// A single range is sufficient for browser media seeking; ambiguous multipart
// requests fail closed instead of returning incorrect bytes or full responses.
fn byte_range(value: &str, len: usize) -> Option<(usize, usize)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    if len == 0 || end.contains(',') {
        return None;
    }
    if start.is_empty() {
        let suffix = end.parse::<usize>().ok()?;
        return (suffix > 0).then(|| (len.saturating_sub(suffix), len - 1));
    }
    let start = start.parse::<usize>().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<usize>().ok()?.min(len - 1)
    };
    (start < len && start <= end).then_some((start, end))
}

fn raw_response(bytes: Vec<u8>, mime: &str, name: &str, range: Option<&HeaderValue>) -> Response {
    let len = bytes.len();
    let selection = range.map(|value| value.to_str().ok().and_then(|value| byte_range(value, len)));
    if selection == Some(None) {
        let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes */{len}")).unwrap(),
        );
        response
            .headers_mut()
            .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
        return response;
    }
    let selected = selection.flatten();
    let status = if selected.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let body = match selected {
        Some((start, end)) => bytes[start..=end].to_vec(),
        None => bytes,
    };
    let body_len = body.len();
    let mut response = (status, body).into_response();
    let textual = mime.starts_with("text/") || mime.eq_ignore_ascii_case("application/json");
    let content_type = if textual && !mime.contains("charset=") {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };
    let content_type = HeaderValue::from_str(&content_type)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let safe_name = name.replace(['"', '\r', '\n', '\\'], "_");
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{safe_name}\""))
        .unwrap_or(HeaderValue::from_static("attachment"));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::CONTENT_DISPOSITION, disposition);
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body_len.to_string()).unwrap(),
    );
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if let Some((start, end)) = selected {
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{len}")).unwrap(),
        );
    }
    response
}

// ---------------------------------------------------------------------------
// GET /artifacts/{id}/metadata
// ---------------------------------------------------------------------------

/// GET /artifacts/{id}/metadata：交付元数据（格式/哈希/字节数/校验/证据链/交接）。
pub(crate) async fn artifact_metadata(
    State(state): State<Arc<AppState>>,
    AxumPath(artifact_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let store = state.artifact_review.store()?;
    let artifact = load_artifact(store.as_ref(), &artifact_id).await?;
    let cas = cas_store(&state)?;
    let hash = artifact_hash(&artifact);
    let size_bytes = artifact_size(&artifact, &cas, &hash);

    // team_id：七期 additive 字段优先；legacy 记录回退 artifact_id 首段解析。
    let team_id = if artifact.team_id.is_empty() {
        artifact
            .artifact_id
            .split(':')
            .next()
            .unwrap_or_default()
            .to_string()
    } else {
        artifact.team_id.clone()
    };

    // 冻结字段 + additive 元数据；handoff 仅在 Worker 提供交接说明时出现（可选键）。
    let mut body = json!({
        "artifact_id": artifact.artifact_id,
        "team_id": team_id,
        "task_id": artifact.task_id,
        "attempt_id": artifact.attempt_id,
        "kind": artifact.kind,
        "format": artifact.format,
        "version": artifact.version,
        "sha256": hash,
        "size_bytes": size_bytes,
        "validation": artifact.validation,
        "evidence_refs": artifact.evidence_refs,
        // additive
        "media_type": artifact.media_type,
        "file_name": artifact.file_name,
        "content_ref": artifact.content_ref,
        "open_issues": artifact.open_issues,
        "review_state": artifact.review_state,
        "supersedes_artifact_id": artifact.supersedes_artifact_id,
        "created_at": artifact.created_at,
    });
    if let Some(handoff) = artifact.handoff {
        body["handoff"] = json!(handoff);
    }
    Ok(Json(body))
}

// ---------------------------------------------------------------------------
// GET /projects/{id}/delivery-manifest
// ---------------------------------------------------------------------------

/// GET /projects/{id}/delivery-manifest：最终交付清单（每 kind 最新版本）。
pub(crate) async fn project_delivery_manifest(
    State(state): State<Arc<AppState>>,
    AxumPath(project_id): AxumPath<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let store = state.artifact_review.store()?;
    store
        .get_project_space(&project_id)
        .await
        .map_err(|e| match e {
            ProjectSpaceStoreError::NotFound(_) => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("项目空间不存在：{project_id}") })),
            ),
            other => store_error(other),
        })?;
    let artifacts = store
        .list_artifacts_by_project(&project_id)
        .await
        .map_err(store_error)?;

    // 每 kind 取最高版本（版本链 head；被取代的旧版本不出现在清单）；
    // critic 评审结论（kind=review）是过程产物，不属于交付物。
    let mut latest: HashMap<String, &Artifact> = HashMap::new();
    for artifact in &artifacts {
        if artifact.kind == "review" {
            continue;
        }
        match latest.get(&artifact.kind) {
            Some(current) if current.version >= artifact.version => {}
            _ => {
                latest.insert(artifact.kind.clone(), artifact);
            }
        }
    }
    let mut heads: Vec<&Artifact> = latest.values().copied().collect();
    heads.sort_by(|a, b| a.artifact_id.cmp(&b.artifact_id));

    let cas = cas_store(&state)?;
    let manifest: Vec<Value> = heads
        .iter()
        .map(|artifact| {
            let hash = artifact_hash(artifact);
            let size_bytes = artifact_size(artifact, &cas, &hash);
            json!({
                // 冻结契约（七期四路冻结，字段名固定）
                "artifact_id": artifact.artifact_id,
                "kind": artifact.kind,
                "format": artifact.format,
                "version": artifact.version,
                "sha256": hash,
                "size_bytes": size_bytes,
                "approved": artifact.review_state == ReviewState::Approved,
                "content_url": format!("/artifacts/{}/content", artifact.artifact_id),
                // additive：校验结果与证据引用（验收项：清单含版本/哈希/验证/证据）
                "validation": artifact.validation,
                "evidence_refs": artifact.evidence_refs,
            })
        })
        .collect();

    Ok(Json(json!({
        "project_id": project_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "manifest": manifest,
    })))
}

#[cfg(test)]
mod media_range_tests {
    use super::*;
    #[tokio::test]
    async fn binary_mime_and_inclusive_open_suffix_ranges() {
        for (range, expected) in [
            ("bytes=1-3", vec![1, 2, 3]),
            ("bytes=3-", vec![3, 4]),
            ("bytes=-2", vec![3, 4]),
            ("bytes=0-99", vec![0, 1, 2, 3, 4]),
        ] {
            let header = HeaderValue::from_str(range).unwrap();
            let response = raw_response(
                vec![0, 1, 2, 3, 4],
                "audio/mpeg",
                "voice.mp3",
                Some(&header),
            );
            assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/mpeg");
            let body = axum::body::to_bytes(response.into_body(), 16)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), expected.as_slice());
        }
        for range in ["bytes=10-", "bytes=4-1", "bytes=-0", "bytes=0-1,3-4", "bad"] {
            let response = raw_response(
                vec![0, 1, 2, 3, 4],
                "image/png",
                "image.png",
                Some(&HeaderValue::from_str(range).unwrap()),
            );
            assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
            assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */5");
        }
        assert!(byte_range("bytes=0-", 0).is_none());
    }
}

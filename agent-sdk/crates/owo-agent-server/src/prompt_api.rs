//! 用户自定义提示词库（Hermes 风格的 persona 预设）。
//!
//! 设计取舍：
//! - **用户级、跨工作区**：提示词是"我怎么跟模型说话"的偏好，不属于某个项目，
//!   因此落在 `data_root/prompt_presets.json`，而不是 `settings.json` 的工作区字段。
//! - **纯文件 + 原子写**：几十条文本不值得为它做一次 SQLite 迁移；写入走
//!   `tmp → rename`，避免半截文件让整个接口 500。
//! - **应用是会话属性**：套用提示词只改 `Session.system_prompt`（已有持久化列），
//!   下一回合组装系统提示词时生效；不改写历史消息。
//!
//! 安全边界：只接收文本，不解释为路径/命令；长度与条数有上限，避免无限膨胀。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::AppState;

/// 单用户提示词库的容量上限（够用且不会把设置文件撑大）。
const MAX_PRESETS: usize = 50;
const MAX_NAME_CHARS: usize = 60;
const MAX_CONTENT_CHARS: usize = 20_000;
const FILE_NAME: &str = "prompt_presets.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPreset {
    pub id: String,
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PromptLibrary {
    #[serde(default)]
    presets: Vec<PromptPreset>,
}

fn library_path(state: &AppState) -> std::path::PathBuf {
    state.data_root.join(FILE_NAME)
}

fn load(state: &AppState) -> Result<PromptLibrary, (StatusCode, String)> {
    load_from(&library_path(state))
}

fn save(state: &AppState, library: &PromptLibrary) -> Result<(), (StatusCode, String)> {
    save_to(&library_path(state), library)
}

fn load_from(path: &std::path::Path) -> Result<PromptLibrary, (StatusCode, String)> {
    if !path.exists() {
        return Ok(PromptLibrary::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("读取提示词库失败：{error}"),
        )
    })?;
    if raw.trim().is_empty() {
        return Ok(PromptLibrary::default());
    }
    // 文件损坏时明确报错，不静默丢用户数据。
    serde_json::from_str(&raw).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("提示词库格式损坏（{}）：{error}", path.display()),
        )
    })
}

fn save_to(path: &std::path::Path, library: &PromptLibrary) -> Result<(), (StatusCode, String)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建数据目录失败：{error}"),
            )
        })?;
    }
    let body = serde_json::to_string_pretty(library)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入提示词库失败：{error}"),
        )
    })?;
    std::fs::rename(&tmp, path).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("提交提示词库失败：{error}"),
        )
    })
}

fn validate_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let name = raw.trim();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "提示词名称不能为空".into()));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("提示词名称过长（最多 {MAX_NAME_CHARS} 字）"),
        ));
    }
    Ok(name.to_string())
}

fn validate_content(raw: &str) -> Result<String, (StatusCode, String)> {
    let content = raw.trim();
    if content.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "提示词内容不能为空".into()));
    }
    if content.chars().count() > MAX_CONTENT_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("提示词内容过长（最多 {MAX_CONTENT_CHARS} 字）"),
        ));
    }
    Ok(content.to_string())
}

pub(super) async fn prompts_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let library = load(&state)?;
    Ok(Json(json!({
        "prompts": library.presets,
        "limits": {
            "max_presets": MAX_PRESETS,
            "max_name_chars": MAX_NAME_CHARS,
            "max_content_chars": MAX_CONTENT_CHARS,
        },
    })))
}

#[derive(Debug, Deserialize)]
pub(super) struct SavePromptRequest {
    /// 缺省 = 新建；给了已存在的 id = 更新。
    #[serde(default)]
    id: Option<String>,
    name: String,
    content: String,
}

/// 新建或更新一条提示词（同名不合并，id 是唯一键）。
pub(super) async fn prompts_save(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SavePromptRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let name = validate_name(&request.name)?;
    let content = validate_content(&request.content)?;
    let mut library = load(&state)?;
    let id = request
        .id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let preset = match id {
        Some(id) => {
            let existing = library
                .presets
                .iter_mut()
                .find(|preset| preset.id == id)
                .ok_or_else(|| (StatusCode::NOT_FOUND, format!("提示词不存在：{id}")))?;
            existing.name = name;
            existing.content = content;
            existing.updated_at = chrono::Utc::now().to_rfc3339();
            existing.clone()
        }
        None => {
            if library.presets.len() >= MAX_PRESETS {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("提示词数量已达上限（{MAX_PRESETS} 条），请先删除不用的"),
                ));
            }
            let preset = PromptPreset {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                content,
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            library.presets.push(preset.clone());
            preset
        }
    };
    save(&state, &library)?;
    Ok(Json(
        json!({ "prompt": preset, "prompts": library.presets }),
    ))
}

pub(super) async fn prompts_delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut library = load(&state)?;
    let before = library.presets.len();
    library.presets.retain(|preset| preset.id != id);
    if library.presets.len() == before {
        return Err((StatusCode::NOT_FOUND, format!("提示词不存在：{id}")));
    }
    save(&state, &library)?;
    Ok(Json(json!({ "deleted": id, "prompts": library.presets })))
}

#[derive(Debug, Deserialize)]
pub(super) struct SetSessionPromptRequest {
    /// `null`/空串 = 清除，回到默认系统提示词。
    #[serde(default)]
    prompt: Option<String>,
}

/// 把自定义提示词应用到会话：只影响后续回合，不改写历史。
pub(super) async fn session_set_system_prompt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(request): Json<SetSessionPromptRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 与创建时同一上限：避免会话库被巨型 persona 撑大；空串是"清除"而非错误。
    if let Some(prompt) = request
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        validate_content(prompt)?;
    }
    let _guard = owo_agent_server::acquire_session_lock(&state, &id).await?;
    let mut session = state
        .store
        .load(&id)
        .map_err(|_error| (StatusCode::NOT_FOUND, format!("会话不存在：{id}")))?;
    session.set_system_prompt(request.prompt);
    state
        .store
        .save(&session)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    state
        .sessions
        .lock()
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "state poisoned".to_string(),
            )
        })?
        .insert(session.id.clone(), session.clone());
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Sessions);
    Ok(Json(json!({
        "id": session.id,
        "system_prompt": session.system_prompt,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("owo-prompt-api-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_is_an_empty_library() {
        let dir = temp_dir("missing");
        let library = load_from(&dir.join(FILE_NAME)).unwrap();
        assert!(library.presets.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_reports_path_instead_of_dropping_data() {
        let dir = temp_dir("corrupt");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, "{not json").unwrap();
        let error = load_from(&path).unwrap_err();
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(error.1.contains("提示词库格式损坏"), "{}", error.1);
        assert!(error.1.contains(FILE_NAME), "{}", error.1);
        // 损坏文件不能被静默清空：原内容仍在。
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_roundtrips_and_replaces_atomically() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE_NAME);
        let mut library = PromptLibrary::default();
        library.presets.push(PromptPreset {
            id: "p1".into(),
            name: "严谨评审".into(),
            content: "先给结论".into(),
            updated_at: String::new(),
        });
        save_to(&path, &library).unwrap();
        // 临时文件不应残留。
        assert!(!dir.join("prompt_presets.json.tmp").exists());
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.presets.len(), 1);
        assert_eq!(loaded.presets[0].name, "严谨评审");

        // 第二次保存覆盖旧内容（Windows 上 rename 需替换已存在文件）。
        library.presets[0].content = "改过的内容".into();
        save_to(&path, &library).unwrap();
        assert_eq!(load_from(&path).unwrap().presets[0].content, "改过的内容");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn limits_match_frontend_and_reject_oversize() {
        assert_eq!(MAX_PRESETS, 50);
        assert_eq!(MAX_NAME_CHARS, 60);
        assert_eq!(MAX_CONTENT_CHARS, 20_000);
        assert!(validate_name(&"名".repeat(MAX_NAME_CHARS + 1)).is_err());
        let error = validate_content(&"长".repeat(MAX_CONTENT_CHARS + 1)).unwrap_err();
        assert!(error.1.contains("过长"), "{}", error.1);
        // 中文按字符数而非字节数计数。
        assert!(validate_content(&"长".repeat(MAX_CONTENT_CHARS)).is_ok());
    }
}

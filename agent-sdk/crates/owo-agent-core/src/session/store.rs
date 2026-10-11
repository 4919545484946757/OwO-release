use crate::error::AgentError;
use owo_agent_protocol::SseEvent;
use owo_agent_protocol::TurnEventRecord;
use std::path::{Path, PathBuf};

use super::model::*;

pub(super) fn relative_display(workspace: &Path, path: &Path) -> String {
    let normalize = |value: &Path| -> String {
        let raw = value.to_string_lossy().replace('\\', "/");
        raw.strip_prefix("//?/").unwrap_or(&raw).to_string()
    };
    let workspace = normalize(workspace);
    let path = normalize(path);
    path.strip_prefix(&workspace)
        .map(|relative| relative.trim_start_matches('/').to_string())
        .unwrap_or(path)
}

pub trait SessionStore: Send + Sync {
    fn create(
        &self,
        workspace: &Path,
        model: &str,
        system_prompt: Option<&str>,
    ) -> Result<Session, AgentError>;
    fn load(&self, id: &str) -> Result<Session, AgentError>;
    fn save(&self, session: &Session) -> Result<(), AgentError>;
    /// Check session presence without loading/sorting the entire session index.
    fn exists(&self, id: &str) -> Result<bool, AgentError> {
        Ok(self.list().iter().any(|candidate| candidate == id))
    }
    /// Persist one turn event and allocate the next sequence number for its session.
    /// Non-durable stores must opt in explicitly rather than pretending to support replay.
    fn append_turn_event(
        &self,
        session_id: &str,
        turn_id: &str,
        payload: &SseEvent,
    ) -> Result<TurnEventRecord, AgentError> {
        let _ = (session_id, turn_id, payload);
        Err(AgentError::Session("当前存储不支持持久化回合事件".into()))
    }
    /// Read a bounded page of persisted events after a session-scoped sequence cursor.
    fn turn_events_after(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<TurnEventRecord>, AgentError> {
        let _ = (session_id, turn_id, after_seq, limit);
        Err(AgentError::Session("当前存储不支持回合事件回放".into()))
    }
    /// Durable facts from the host; unsupported stores explicitly return no history.
    fn turn_history(
        &self,
        _session_id: &str,
    ) -> Result<Option<owo_agent_protocol::SessionTurnHistoryV1>, AgentError> {
        Ok(None)
    }
    /// 列出全部会话 ID（按更新时间倒序）。
    fn list(&self) -> Vec<String> {
        Vec::new()
    }
    /// 删除一个会话及其存储产物。
    ///
    /// 默认返回"不支持"而不是静默成功：调用方（`DELETE /session/{id}`）必须能区分
    /// "删掉了"和"这个后端根本没实现删除"，否则界面会显示删除成功而会话仍在。
    fn remove(&self, _id: &str) -> Result<(), AgentError> {
        Err(AgentError::Session("当前会话存储后端不支持删除会话".into()))
    }
    /// 持久化审计记录（默认 no-op；SQLite 存储落库）。
    fn append_audit(&self, entries: &[crate::audit::AuditEntry]) -> Result<(), AgentError> {
        let _ = entries;
        Ok(())
    }
    /// 最近 N 条审计记录（默认空；SQLite 存储返回持久化记录）。
    fn recent_audit(&self, limit: usize) -> Vec<crate::audit::AuditEntry> {
        let _ = limit;
        Vec::new()
    }

    /// 分页/过滤/搜索审计（默认退化为 recent_audit；SQLite 存储提供完整实现）。
    fn query_audit(
        &self,
        query: &crate::sqlite_store::AuditQuery,
    ) -> (Vec<crate::audit::AuditEntry>, usize) {
        let entries = self.recent_audit(query.limit.max(1));
        let total = entries.len();
        (entries, total)
    }

    /// 清空会话与审计（R8 存储运维；默认不支持，SQLite 存储提供实现）。
    fn clear(&self) -> Result<(), AgentError> {
        Err(AgentError::Session("当前存储不支持清空".into()))
    }

    /// 存储是否处于只读降级（R8：迁移失败后的安全状态；默认否）。
    fn is_read_only(&self) -> bool {
        false
    }

    /// 只读降级原因/迁移警告（R8；默认无）。
    fn migration_warning(&self) -> Option<String> {
        None
    }
}

/// M1 会话存储：JSON 文件（后续迁移 SQLite）。
/// R9：可选加密模式——落盘经 storage_crypto 文件信封加密（`<id>.json.owo-crypt`），
/// 读取优先解密；明文 `.json` 保持兼容（既有存储不受影响）。
pub struct JsonSessionStore {
    root: PathBuf,
    encrypted: bool,
}

impl JsonSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            encrypted: false,
        }
    }

    /// 加密模式：会话落盘经 DPAPI 信封加密（非 Windows 下 save 会显式失败）。
    pub fn new_encrypted(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            encrypted: true,
        }
    }

    pub(super) fn plain_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn encrypted_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json.owo-crypt"))
    }

    fn path(&self, id: &str) -> PathBuf {
        if self.encrypted {
            self.encrypted_path(id)
        } else {
            self.plain_path(id)
        }
    }
}

impl SessionStore for JsonSessionStore {
    /// 删除会话文件（明文与加密两种形态都尝试）。
    fn remove(&self, id: &str) -> Result<(), AgentError> {
        let mut removed = false;
        for path in [self.plain_path(id), self.encrypted_path(id)] {
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|error| AgentError::Session(format!("删除会话文件失败：{error}")))?;
                removed = true;
            }
        }
        if removed {
            Ok(())
        } else {
            Err(AgentError::Session(format!("会话不存在：{id}")))
        }
    }

    fn create(
        &self,
        workspace: &Path,
        model: &str,
        system_prompt: Option<&str>,
    ) -> Result<Session, AgentError> {
        let session = Session::new(workspace, model, system_prompt.map(str::to_string));
        self.save(&session)?;
        Ok(session)
    }

    fn load(&self, id: &str) -> Result<Session, AgentError> {
        // 加密形态优先，明文兜底（读取解密；加密文件损坏 → 显式错误，不静默回退明文）。
        let encrypted_path = self.encrypted_path(id);
        if encrypted_path.exists() {
            let content = crate::storage_crypto::decrypt_file_envelope(&encrypted_path)
                .map_err(|error| AgentError::Session(format!("会话 {id} 解密失败：{error}")))?;
            return Ok(serde_json::from_slice(&content)?);
        }
        let content = std::fs::read_to_string(self.plain_path(id))
            .map_err(|e| AgentError::Session(format!("会话 {id} 读取失败：{e}")))?;
        Ok(serde_json::from_str(&content)?)
    }

    fn exists(&self, id: &str) -> Result<bool, AgentError> {
        Ok(self.plain_path(id).exists() || self.encrypted_path(id).exists())
    }

    fn save(&self, session: &Session) -> Result<(), AgentError> {
        std::fs::create_dir_all(&self.root)?;
        let target = self.path(&session.id);
        let content = serde_json::to_vec_pretty(session)?;
        if self.encrypted {
            crate::storage_crypto::encrypt_file_envelope(&target, &content).map_err(|error| {
                AgentError::Session(format!("会话 {} 落盘加密失败：{error}", session.id))
            })?;
            return Ok(());
        }
        let tmp = self.root.join(format!("{}.tmp", session.id));
        std::fs::write(&tmp, content)?;
        if let Err(rename_error) = std::fs::rename(&tmp, &target) {
            // Windows 不允许 rename 覆盖已有文件；保留临时文件写入语义，
            // 在目标存在时执行一次兼容替换。
            if !target.exists() {
                return Err(rename_error.into());
            }
            std::fs::remove_file(&target)?;
            std::fs::rename(&tmp, &target)?;
        }
        Ok(())
    }

    fn list(&self) -> Vec<String> {
        let mut sessions = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(id) = name.strip_suffix(".json.owo-crypt") {
                    sessions.push(id.to_string());
                } else if let Some(id) = name.strip_suffix(".json") {
                    sessions.push(id.to_string());
                }
            }
        }
        sessions.sort_by(|a, b| {
            let ta = self
                .load(a)
                .map(|s| s.updated_at.clone())
                .unwrap_or_default();
            let tb = self
                .load(b)
                .map(|s| s.updated_at.clone())
                .unwrap_or_default();
            tb.cmp(&ta)
        });
        sessions
    }
}

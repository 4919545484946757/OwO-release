use super::store::*;
use crate::error::AgentError;
use crate::gateway::ChatMessage;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use owo_agent_protocol::FileDiff;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 文件写前快照条目（diff/revert 底座）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEntry {
    /// None 表示文件原本不存在（回滚时删除）。
    #[serde(default)]
    pub original_b64: Option<String>,
    /// Agent 最近一次成功写入后的 SHA-256；撤销前必须匹配，防止覆盖用户后续修改。
    #[serde(default)]
    pub expected_after_sha256: Option<String>,
    /// 回合归属：记录快照时该回合用户消息的下标（= 写文件那一刻的 messages.len()）。
    /// 供 `/rewind` 只回滚被截断段落的写操作；旧数据回退为 0（视作最早回合）。
    #[serde(default)]
    pub turn: usize,
}

/// 普通 Agent 文件写入的最小变更集收据。
///
/// 内容本身仍由 `SnapshotEntry` 保存；这里保存本次执行窗口的身份、基线/结果哈希和
/// 状态，使 diff/revert 可以按收据消费，而不是只能按整个会话的路径集合猜测。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    pub receipt_id: String,
    pub tool: String,
    pub turn_id: String,
    pub changed_files: Vec<String>,
    /// 展示相对路径 → Session snapshot 的 canonical 绝对键；只用于内部恢复定位。
    #[serde(default)]
    pub snapshot_keys: HashMap<String, String>,
    pub before_hashes: HashMap<String, Option<String>>,
    pub after_hashes: HashMap<String, Option<String>>,
    pub diff_sha256: String,
    pub created_at: String,
    #[serde(default)]
    pub status: String,
    /// Host validation receipt that accepted this exact file snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_receipt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub workspace: PathBuf,
    pub model: String,
    pub system_prompt: Option<String>,
    pub messages: Vec<ChatMessage>,
    pub snapshots: HashMap<String, SnapshotEntry>,
    /// ToolHost 成功写入产生的收据；旧会话没有此字段时按空列表加载。
    #[serde(default)]
    pub execution_receipts: Vec<ExecutionReceipt>,
    /// Host-produced behavior validation evidence for ordinary Agent work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_receipts: Vec<crate::plan::ValidationReceiptV1>,
    /// Latest host completion decision, bound to this turn and its candidate snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_record: Option<owo_agent_protocol::TaskCompletionRecordV1>,
    /// Durable Single review findings and their receipt-bound repair closure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub single_review_issues: Vec<crate::goal::DeliveryIssueV1>,
    /// Model-proposed host-registered acceptance checks for the active task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_verification_plan: Option<crate::plan::VerificationPlanV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_verification_plan_input_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub single_verification_plan_turn_id: Option<String>,
    /// Ephemeral host-resolved task identity shared by Single execution and acceptance tools.
    #[serde(skip)]
    pub(crate) active_task_context: Option<crate::task_context::ResolvedTaskContext>,
    /// Ephemeral model-request observations for building an error trace before a TurnOutcome exists.
    #[serde(skip)]
    pub(crate) transient_model_calls: Vec<crate::agent::ModelCallRecord>,
    pub created_at: String,
    pub updated_at: String,
    /// 父会话（由 fork 产生时）。
    #[serde(default)]
    pub parent_id: Option<String>,
    /// fork 时的消息下标（含）。
    #[serde(default)]
    pub fork_point: Option<usize>,
    /// 被 /rewind 截断的历史，可 /redo 恢复。
    #[serde(default)]
    pub redo_stack: Vec<Vec<ChatMessage>>,
    /// 被 /undo-msg 移除的消息，可 /redo-msg 恢复。
    #[serde(default)]
    pub message_redo_stack: Vec<Vec<ChatMessage>>,
    /// 用户可编辑的会话标题（None 时按首条用户消息自动显示）。
    #[serde(default)]
    pub title: Option<String>,
    /// 归档标记（默认列表可隐藏）。
    #[serde(default)]
    pub archived: bool,
    /// 置顶标记（列表优先）。
    #[serde(default)]
    pub pinned: bool,
    /// M4.2 会话级模型覆盖：`Some(model)` 固定请求模型（回合/压缩外的一切调用
    /// 都走该模型）；`None` 表示按 Provider 解析链（OPENAI_MODEL 热切换 → 启动
    /// 配置 → 内置默认）。`model` 字段仅为展示值，路由以本字段为准。
    #[serde(default)]
    pub model_override: Option<String>,
    /// 会话级任务清单（`todo` 工具维护；整表替换语义）。
    #[serde(default)]
    pub todos: Vec<TodoItem>,
    /// 会话级目标模式状态（`/goal`）；状态与迁移见 [`super::goal`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<super::SessionGoal>,
}

/// 任务清单条目（`todo` 工具写入，CLI `/todo` 渲染）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItem {
    pub content: String,
    /// `pending` | `in_progress` | `completed`。
    pub status: String,
}

impl Session {
    pub fn new(
        workspace: impl Into<PathBuf>,
        model: impl Into<String>,
        system_prompt: Option<String>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            workspace: workspace.into(),
            model: model.into(),
            system_prompt,
            messages: Vec::new(),
            snapshots: HashMap::new(),
            execution_receipts: Vec::new(),
            validation_receipts: Vec::new(),
            completion_record: None,
            single_review_issues: Vec::new(),
            single_verification_plan: None,
            single_verification_plan_input_sha256: None,
            single_verification_plan_turn_id: None,
            active_task_context: None,
            transient_model_calls: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            parent_id: None,
            fork_point: None,
            redo_stack: Vec::new(),
            message_redo_stack: Vec::new(),
            title: None,
            archived: false,
            pinned: false,
            model_override: None,
            todos: Vec::new(),
            goal: None,
        }
    }

    /// 模型覆盖归一（M4.2）：trim；空串与 `"default"` 哨兵 = None（自动）。
    fn normalize_model_override(model: Option<String>) -> Option<String> {
        model
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && value != crate::gateway::MODEL_DEFAULT_SENTINEL)
    }

    /// 设置会话级模型覆盖（M4.2）；空串/纯空白/`"default"` 哨兵视为清除覆盖
    /// （回退 Provider 链）；哨兵不落库。
    pub fn with_model_override(mut self, model: Option<String>) -> Self {
        self.model_override = Self::normalize_model_override(model);
        self
    }

    /// 会话级模型路由设置（M4.2）：非空字符串固定请求模型；`None`、空串与
    /// `"default"` 哨兵一律清除覆盖（回退 Provider 解析链，哨兵不落库、不进请求体）。
    /// 固定时展示模型同步为固定值；清除时展示模型保持现状。
    pub fn set_model_override(&mut self, model: Option<String>) {
        self.model_override = Self::normalize_model_override(model);
        if let Some(value) = self.model_override.clone() {
            self.model = value;
        }
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// 展示标题：优先自定义标题，否则取首条用户消息，最后回退为会话短 ID。
    pub fn display_title(&self) -> String {
        if let Some(title) = &self.title {
            let trimmed = title.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(first) = self
            .messages
            .iter()
            .find(|message| message.role == "user")
            .and_then(|message| message.content.as_deref())
        {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return trimmed.chars().take(40).collect();
            }
        }
        let short_id: String = self.id.chars().take(8).collect();
        format!("会话 {short_id}")
    }

    pub fn rename(&mut self, title: String) {
        self.title = Some(title);
        self.updated_at = Utc::now().to_rfc3339();
    }

    pub fn set_archived(&mut self, archived: bool) {
        self.archived = archived;
        self.updated_at = Utc::now().to_rfc3339();
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
        self.updated_at = Utc::now().to_rfc3339();
    }

    pub fn push(&mut self, message: ChatMessage) {
        self.messages.push(message);
        self.redo_stack.clear();
        self.message_redo_stack.clear();
        self.updated_at = Utc::now().to_rfc3339();
    }

    /// 当前会话改动 diff（相对工作区路径）。
    pub fn diff(&self) -> Vec<FileDiff> {
        let mut diffs = Vec::new();
        for (path, snapshot) in &self.snapshots {
            let original = snapshot.original_b64.as_ref().and_then(|encoded| {
                BASE64
                    .decode(encoded)
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            });
            let current = std::fs::read(path)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
            if original == current {
                continue;
            }
            diffs.push(FileDiff {
                path: relative_display(&self.workspace, Path::new(path)),
                before: original,
                after: current,
            });
        }
        diffs
    }

    /// 在工具成功写入后登记一个单文件执行收据。
    ///
    /// 返回 `None` 表示内容未发生变化；读取/解码错误则 fail closed，不生成可撤销收据。
    pub fn record_file_execution(
        &mut self,
        tool: &str,
        turn_id: &str,
        absolute_path: &Path,
    ) -> Result<Option<ExecutionReceipt>, AgentError> {
        let key = absolute_path.to_string_lossy().replace('\\', "/");
        let Some(snapshot) = self.snapshots.get(&key) else {
            return Ok(None);
        };
        let before = snapshot
            .original_b64
            .as_deref()
            .map(|encoded| {
                BASE64
                    .decode(encoded)
                    .map_err(|error| AgentError::Session(format!("快照解码失败：{error}")))
            })
            .transpose()?;
        let after = match std::fs::read(absolute_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(AgentError::Io(error)),
        };
        let before_hash = before.as_deref().map(crate::CasStore::hash_of);
        let after_hash = after.as_deref().map(crate::CasStore::hash_of);
        if before_hash == after_hash {
            return Ok(None);
        }
        let relative = relative_display(&self.workspace, absolute_path);
        let diff_sha256 = crate::CasStore::hash_of(
            serde_json::json!({
                "path": relative,
                "before": before_hash,
                "after": after_hash,
            })
            .to_string()
            .as_bytes(),
        );
        let receipt = ExecutionReceipt {
            receipt_id: format!("exec-{}", uuid::Uuid::new_v4()),
            tool: tool.to_string(),
            turn_id: turn_id.to_string(),
            changed_files: vec![relative.clone()],
            snapshot_keys: HashMap::from([(relative.clone(), key)]),
            before_hashes: HashMap::from([(relative.clone(), before_hash)]),
            after_hashes: HashMap::from([(relative, after_hash)]),
            diff_sha256,
            created_at: Utc::now().to_rfc3339(),
            status: "executed".to_string(),
            validation_receipt_id: None,
        };
        self.execution_receipts.push(receipt.clone());
        self.updated_at = Utc::now().to_rfc3339();
        Ok(Some(receipt))
    }

    /// 按执行收据撤销；不传 ID 时消费最近一张尚未撤销的收据。
    /// 没有新式收据的旧会话回退到兼容的全快照撤销逻辑。
    pub async fn revert_receipt(
        &mut self,
        receipt_id: Option<&str>,
    ) -> Result<Vec<String>, AgentError> {
        let index = match receipt_id {
            Some(id) => self
                .execution_receipts
                .iter()
                .position(|receipt| receipt.receipt_id == id)
                .ok_or_else(|| AgentError::Session(format!("执行收据不存在：{id}")))?,
            None => match self
                .execution_receipts
                .iter()
                .rposition(|receipt| receipt.status != "reverted")
            {
                Some(index) => index,
                None => return self.revert_legacy().await,
            },
        };
        let receipt = self.execution_receipts[index].clone();
        if receipt.status == "reverted" {
            return Ok(Vec::new());
        }

        let mut plan = Vec::new();
        let mut conflicts = Vec::new();
        for relative in &receipt.changed_files {
            let key = receipt
                .snapshot_keys
                .get(relative)
                .cloned()
                .unwrap_or_else(|| {
                    self.workspace
                        .canonicalize()
                        .unwrap_or_else(|_| self.workspace.clone())
                        .join(relative)
                        .to_string_lossy()
                        .replace('\\', "/")
                });
            let target = PathBuf::from(&key);
            let Some(snapshot) = self.snapshots.get(&key) else {
                conflicts.push(relative.clone());
                continue;
            };
            let current = match std::fs::read(&target) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(AgentError::Io(error)),
            };
            let current_hash = current.as_deref().map(crate::CasStore::hash_of);
            let before_hash = receipt.before_hashes.get(relative).cloned().flatten();
            let after_hash = receipt.after_hashes.get(relative).cloned().flatten();
            if current_hash == before_hash {
                continue;
            }
            if current_hash != after_hash {
                conflicts.push(relative.clone());
                continue;
            }
            let original = snapshot
                .original_b64
                .as_deref()
                .map(|encoded| {
                    BASE64
                        .decode(encoded)
                        .map_err(|error| AgentError::Session(format!("快照解码失败：{error}")))
                })
                .transpose()?;
            plan.push((relative.clone(), target, key, original));
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            conflicts.dedup();
            return Err(AgentError::RevertConflict { paths: conflicts });
        }

        let mut restored = Vec::with_capacity(plan.len());
        for (relative, target, key, original) in plan {
            match original {
                Some(bytes) => {
                    if let Some(parent) = target.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&target, bytes).await?;
                }
                None => match tokio::fs::remove_file(&target).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(AgentError::Io(error)),
                },
            }
            self.snapshots.remove(&key);
            restored.push(relative);
        }
        self.execution_receipts[index].status = "reverted".to_string();
        self.updated_at = Utc::now().to_rfc3339();
        Ok(restored)
    }

    /// 回滚全部已快照的写操作，返回被恢复的路径。
    pub async fn revert(&mut self) -> Result<Vec<String>, AgentError> {
        if !self.execution_receipts.is_empty() {
            return self.revert_receipt(None).await;
        }
        self.revert_legacy().await
    }

    /// 旧快照格式的兼容撤销路径；新写入优先走 `revert_receipt`。
    async fn revert_legacy(&mut self) -> Result<Vec<String>, AgentError> {
        self.revert_legacy_from(None).await
    }

    /// 只回滚「回合归属 >= keep」的写操作（配合 rewind：撤销被截断段落的文件改动），
    /// 更早回合的快照保留：`/diff` 与后续 `/revert` 依旧能看到、回滚它们。
    ///
    /// 取优合并（远端 engine）：receipt 路径（`execution_receipts`）尚未带回合归属，
    /// 存在 receipt 时保守退化为全量回滚。
    pub async fn revert_from(&mut self, keep: usize) -> Result<Vec<String>, AgentError> {
        if !self.execution_receipts.is_empty() {
            return self.revert().await;
        }
        self.revert_legacy_from(Some(keep)).await
    }

    /// `keep = Some(k)`：只回滚 `turn >= k` 的快照，并把更早的快照留在表里。
    async fn revert_legacy_from(&mut self, keep: Option<usize>) -> Result<Vec<String>, AgentError> {
        // 先完整预检，再开始写盘：任何文件被用户/外部进程改过时，整批撤销零副作用。
        let mut restore_plan = Vec::new();
        let mut conflicts = Vec::new();
        for (path, snapshot) in &self.snapshots {
            // keep = Some(k)：只处理被截断段落（turn >= k）的快照。
            if keep.is_some_and(|keep| snapshot.turn < keep) {
                continue;
            }
            let target = PathBuf::from(path);
            let original = match &snapshot.original_b64 {
                Some(encoded) => Some(
                    BASE64
                        .decode(encoded)
                        .map_err(|e| AgentError::Session(format!("快照解码失败：{e}")))?,
                ),
                None => None,
            };
            let current = match std::fs::read(&target) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(AgentError::Io(error)),
            };
            if current == original {
                continue;
            }
            let matches_agent_write = match (
                snapshot.expected_after_sha256.as_deref(),
                current.as_deref(),
            ) {
                (Some(expected), Some(bytes)) => crate::CasStore::hash_of(bytes) == expected,
                _ => false,
            };
            if matches_agent_write {
                restore_plan.push((target, original));
            } else {
                conflicts.push(relative_display(&self.workspace, &target));
            }
        }
        if !conflicts.is_empty() {
            conflicts.sort();
            conflicts.dedup();
            return Err(AgentError::RevertConflict { paths: conflicts });
        }

        let mut restored = Vec::with_capacity(restore_plan.len());
        for (target, original) in restore_plan {
            match original {
                Some(bytes) => {
                    if let Some(parent) = target.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&target, bytes).await?;
                }
                None => match tokio::fs::remove_file(&target).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(AgentError::Io(error)),
                },
            }
            restored.push(relative_display(&self.workspace, &target));
        }
        match keep {
            // rewind 场景：更早回合的快照保留（/diff 与后续 /revert 仍可见）。
            Some(keep) => self.snapshots.retain(|_, snapshot| snapshot.turn < keep),
            None => self.snapshots.clear(),
        }
        self.execution_receipts.clear();
        self.updated_at = Utc::now().to_rfc3339();
        Ok(restored)
    }

    /// 在指定消息处派生一个子会话（继承历史，快照与 redo 栈清空）。
    pub fn fork(&self, message_index: usize) -> Session {
        let messages = if self.messages.is_empty() {
            Vec::new()
        } else {
            let end = message_index.min(self.messages.len() - 1);
            self.messages[..=end].to_vec()
        };
        let now = Utc::now().to_rfc3339();
        Session {
            id: uuid::Uuid::new_v4().to_string(),
            workspace: self.workspace.clone(),
            model: self.model.clone(),
            system_prompt: self.system_prompt.clone(),
            messages,
            snapshots: HashMap::new(),
            execution_receipts: Vec::new(),
            validation_receipts: Vec::new(),
            completion_record: None,
            single_review_issues: Vec::new(),
            single_verification_plan: self.single_verification_plan.clone(),
            single_verification_plan_input_sha256: self
                .single_verification_plan_input_sha256
                .clone(),
            single_verification_plan_turn_id: self.single_verification_plan_turn_id.clone(),
            active_task_context: None,
            transient_model_calls: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            parent_id: Some(self.id.clone()),
            fork_point: Some(message_index),
            redo_stack: Vec::new(),
            message_redo_stack: Vec::new(),
            title: None,
            archived: false,
            pinned: false,
            // fork 继承父会话的模型覆盖（路由语义随历史一起派生）。
            model_override: self.model_override.clone(),
            // 任务清单随 fork 继承（继续同一任务）。
            todos: self.todos.clone(),
            // 分支继承父会话的当前目标与进度（继续推进同一目标）。
            goal: self.goal.clone(),
        }
    }

    /// 回退到仅保留前 `keep` 条消息，同时清空文件快照；返回被移除的历史。
    pub fn rewind(&mut self, keep: usize) -> Vec<ChatMessage> {
        if keep >= self.messages.len() {
            return Vec::new();
        }
        let removed = self.messages.split_off(keep);
        self.redo_stack.push(removed.clone());
        // 取优合并（远端 engine）：保留被截断段落之前的快照，仅清掉被截断段落的
        // 写操作归属（文件回滚由调用方 `revert_from(keep)` 执行）。
        self.snapshots.retain(|_, snapshot| snapshot.turn < keep);
        self.execution_receipts.clear();
        self.updated_at = Utc::now().to_rfc3339();
        removed
    }

    /// 恢复最近一次 rewind 移除的历史。
    pub fn redo(&mut self) -> Option<Vec<ChatMessage>> {
        let tail = self.redo_stack.pop()?;
        self.messages.extend(tail.iter().cloned());
        self.updated_at = Utc::now().to_rfc3339();
        Some(tail)
    }

    /// 移除最近 `count` 条消息（消息级撤销），压入可恢复栈。
    pub fn undo_message(&mut self, count: usize) -> Option<Vec<ChatMessage>> {
        let count = count.min(self.messages.len());
        if count == 0 {
            return None;
        }
        let split_at = self.messages.len() - count;
        let removed = self.messages.split_off(split_at);
        self.message_redo_stack.push(removed.clone());
        self.updated_at = Utc::now().to_rfc3339();
        Some(removed)
    }

    /// 恢复最近一次消息级撤销。
    pub fn redo_message(&mut self) -> Option<Vec<ChatMessage>> {
        let tail = self.message_redo_stack.pop()?;
        self.messages.extend(tail.iter().cloned());
        self.updated_at = Utc::now().to_rfc3339();
        Some(tail)
    }
}

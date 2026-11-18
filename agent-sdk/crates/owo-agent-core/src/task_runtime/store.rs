//! Task Store：任务、事件、检查点、审批、产物与幂等键的 SQLite 持久化。
//!
//! 与 `sqlite_store::SqliteSessionStore` 分开建库（默认 `<data_root>/agent_tasks.db`）：
//! Task 是独立的用户委托单元，不塞进 Session 表；两者共用同一套 SQLite 技术栈与
//! busy_timeout 约定。
//!
//! 关键要求（Agent V2 §2.5/§7）：
//! - 状态与事件写入在同一事务内完成，事件序号严格单调递增；
//! - 大对象（截图、模型原始响应、文件内容）只存引用，不入事件表；
//! - 秘密值（API Key、密码、Cookie、验证码）不得写入任何 Task 表。

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};

use super::model::*;
use super::state_machine::ensure_transition;

const SCHEMA: &str = r#"
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS tasks (
    task_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    user_goal TEXT NOT NULL,
    state TEXT NOT NULL,
    execution_strategy TEXT NOT NULL,
    current_attempt INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_checkpoint_id TEXT,
    terminal_summary TEXT,
    budget_json TEXT NOT NULL,
    policy_profile TEXT NOT NULL,
    workspace_ref TEXT,
    session_refs_json TEXT NOT NULL DEFAULT '[]',
    schema_version INTEGER NOT NULL,
    payload_json TEXT
);

CREATE TABLE IF NOT EXISTS task_attempts (
    task_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    outcome TEXT,
    PRIMARY KEY (task_id, attempt)
);

CREATE TABLE IF NOT EXISTS task_events (
    task_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    correlation_id TEXT,
    PRIMARY KEY (task_id, sequence),
    UNIQUE (event_id)
);
CREATE INDEX IF NOT EXISTS idx_task_events_type
    ON task_events (task_id, event_type, sequence);

CREATE TABLE IF NOT EXISTS task_checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    phase TEXT NOT NULL,
    state_snapshot_ref TEXT NOT NULL,
    workspace_revision TEXT,
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    metadata_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_task_checkpoints_task
    ON task_checkpoints (task_id, created_at DESC);

CREATE TABLE IF NOT EXISTS task_approvals (
    approval_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    action TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    expires_at TEXT,
    decided_at TEXT,
    decision TEXT,
    decided_by TEXT
);
CREATE INDEX IF NOT EXISTS idx_task_approvals_pending
    ON task_approvals (task_id, decision);

CREATE TABLE IF NOT EXISTS task_artifacts (
    artifact_id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    kind TEXT NOT NULL,
    uri_or_path TEXT NOT NULL,
    sha256 TEXT,
    size_bytes INTEGER,
    created_at TEXT NOT NULL,
    verification_status TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_artifacts_task
    ON task_artifacts (task_id, created_at);

CREATE TABLE IF NOT EXISTS task_idempotency_keys (
    key TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);
"#;

/// SQLite 支撑的 Task Store。
pub struct TaskStore {
    conn: Mutex<Connection>,
}

impl TaskStore {
    /// 打开（或创建）任务库。会创建父目录并执行幂等建表。
    pub fn open(path: &Path) -> TaskResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(TaskError::store)?;
        }
        let conn = Connection::open(path).map_err(TaskError::store)?;
        Self::from_connection(conn)
    }

    /// 内存库（测试与无法落盘时的降级）。
    pub fn open_in_memory() -> TaskResult<Self> {
        let conn = Connection::open_in_memory().map_err(TaskError::store)?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> TaskResult<Self> {
        conn.busy_timeout(Duration::from_millis(5_000))
            .map_err(TaskError::store)?;
        conn.execute_batch(SCHEMA).map_err(TaskError::store)?;
        // v1 → v2：旧库没有 `payload_json`；`CREATE TABLE IF NOT EXISTS` 不会补列，
        // 这里按 table_info 探测后幂等追加，已有任务按 NULL（无后端专属输入）加载。
        ensure_column(&conn, "tasks", "payload_json", "TEXT")?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(TaskError::store)?;
        if version < i64::from(TASK_SCHEMA_VERSION) {
            conn.pragma_update(None, "user_version", i64::from(TASK_SCHEMA_VERSION))
                .map_err(TaskError::store)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> TaskResult<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| TaskError::store("任务库连接锁中毒"))
    }

    // ------------------------------------------------------------------
    // tasks
    // ------------------------------------------------------------------

    pub fn create_task(&self, record: &TaskRecord) -> TaskResult<()> {
        self.create_task_with_claim(record, None).map(|_| ())
    }

    /// The task, its initial events and admission key commit together.
    pub fn create_task_idempotent(
        &self,
        record: &TaskRecord,
        key: &str,
        fingerprint: &Value,
    ) -> TaskResult<(TaskRecord, bool)> {
        self.create_task_with_claim(record, Some((key, fingerprint)))
    }

    fn create_task_with_claim(
        &self,
        record: &TaskRecord,
        claim: Option<(&str, &Value)>,
    ) -> TaskResult<(TaskRecord, bool)> {
        let mut conn = self.lock()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(TaskError::store)?;
        if let Some((key, fingerprint)) = claim {
            let existing = tx
                .query_row(
                    "SELECT task_id, payload_json FROM task_idempotency_keys WHERE key = ?1",
                    params![key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(TaskError::store)?;
            if let Some((task_id, payload)) = existing {
                let stored: Value = serde_json::from_str(&payload).map_err(TaskError::store)?;
                if stored != *fingerprint {
                    return Err(TaskError::new(
                        codes::IDEMPOTENCY_CONFLICT,
                        "Admission key was already used for different content",
                    ));
                }
                let task = load_task(&tx, &task_id)?
                    .ok_or_else(|| TaskError::store("Admission key refers to a missing task"))?;
                return Ok((task, true));
            }
        }
        tx.execute(
            "INSERT INTO tasks (
                task_id, title, user_goal, state, execution_strategy, current_attempt,
                created_at, updated_at, last_checkpoint_id, terminal_summary,
                budget_json, policy_profile, workspace_ref, session_refs_json, schema_version,
                payload_json
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                record.task_id.as_str(),
                record.title,
                record.user_goal,
                record.state.as_str(),
                record.execution_strategy.as_str(),
                record.current_attempt,
                record.created_at,
                record.updated_at,
                record.last_checkpoint_id,
                record.terminal_summary,
                serde_json::to_string(&record.budget).map_err(TaskError::store)?,
                record.policy_profile,
                record.workspace_ref,
                serde_json::to_string(&record.session_refs).map_err(TaskError::store)?,
                record.schema_version,
                record
                    .payload
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(TaskError::store)?,
            ],
        )
        .map_err(TaskError::store)?;
        append_event_tx(
            &tx,
            record.task_id.as_str(),
            event_type::TASK_CREATED,
            record.current_attempt,
            json!({
                "title": record.title,
                "user_goal": record.user_goal,
                "execution_strategy": record.execution_strategy.as_str(),
                "policy_profile": record.policy_profile,
            }),
            None,
        )?;
        if let Some((key, fingerprint)) = claim {
            append_event_tx(
                &tx,
                record.task_id.as_str(),
                event_type::TASK_QUEUED,
                0,
                json!({"execution_strategy": record.execution_strategy.as_str()}),
                None,
            )?;
            tx.execute(
                "INSERT INTO task_idempotency_keys (key, task_id, created_at, payload_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    key,
                    record.task_id.as_str(),
                    now(),
                    serde_json::to_string(fingerprint).map_err(TaskError::store)?
                ],
            )
            .map_err(TaskError::store)?;
        }
        tx.commit().map_err(TaskError::store)?;
        Ok((record.clone(), false))
    }

    /// Latest committed event cursor, without replaying the full history.
    pub fn last_event_sequence(&self, task_id: &str) -> TaskResult<u64> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM task_events WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .map_err(TaskError::store)
    }

    pub fn get_task(&self, task_id: &str) -> TaskResult<Option<TaskRecord>> {
        let conn = self.lock()?;
        load_task(&conn, task_id)
    }

    pub fn list_tasks(
        &self,
        state: Option<TaskState>,
        limit: usize,
        offset: usize,
    ) -> TaskResult<Vec<TaskRecord>> {
        let conn = self.lock()?;
        let limit = limit.clamp(1, 500) as i64;
        let offset = offset as i64;
        let mut rows: Vec<TaskRecord> = Vec::new();
        if let Some(state) = state {
            let mut statement = conn
                .prepare(
                    "SELECT * FROM tasks WHERE state = ?1
                     ORDER BY updated_at DESC LIMIT ?2 OFFSET ?3",
                )
                .map_err(TaskError::store)?;
            let mapped = statement
                .query_map(params![state.as_str(), limit, offset], row_to_task)
                .map_err(TaskError::store)?;
            for row in mapped {
                rows.push(row.map_err(TaskError::store)?);
            }
        } else {
            let mut statement = conn
                .prepare("SELECT * FROM tasks ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2")
                .map_err(TaskError::store)?;
            let mapped = statement
                .query_map(params![limit, offset], row_to_task)
                .map_err(TaskError::store)?;
            for row in mapped {
                rows.push(row.map_err(TaskError::store)?);
            }
        }
        Ok(rows)
    }

    /// 状态迁移 + 事件写入（同一事务）。终态不可被普通更新覆盖。
    pub fn transition(
        &self,
        task_id: &str,
        to: TaskState,
        reason: Option<&str>,
        summary: Option<&str>,
    ) -> TaskResult<TaskRecord> {
        self.transition_if_current(task_id, to, reason, summary, None)
    }

    pub(super) fn transition_if_current(
        &self,
        task_id: &str,
        to: TaskState,
        reason: Option<&str>,
        summary: Option<&str>,
        expected: Option<(TaskState, u32)>,
    ) -> TaskResult<TaskRecord> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        let current = load_task_tx(&tx, task_id)?.ok_or_else(|| TaskError::not_found(task_id))?;
        if expected.is_some_and(|expected| expected != (current.state, current.current_attempt)) {
            return Err(TaskError::new(
                codes::INVALID_REQUEST,
                "task changed before resume",
            ));
        }
        ensure_transition(current.state, to)?;
        let now = now();
        tx.execute(
            "UPDATE tasks SET state = ?1, updated_at = ?2,
                terminal_summary = COALESCE(?3, terminal_summary)
             WHERE task_id = ?4",
            params![to.as_str(), now, summary, task_id],
        )
        .map_err(TaskError::store)?;
        let event_type = event_type_for_state(to);
        append_event_tx(
            &tx,
            task_id,
            event_type,
            current.current_attempt,
            json!({
                "from": current.state.as_str(),
                "to": to.as_str(),
                "reason": reason,
            }),
            None,
        )?;
        tx.commit().map_err(TaskError::store)?;
        // 释放连接锁再读回：Mutex 不可重入，`self.get_task` 会再次 lock。
        drop(conn);
        self.get_task(task_id)?
            .ok_or_else(|| TaskError::not_found(task_id))
    }

    pub fn bump_attempt(&self, task_id: &str) -> TaskResult<u32> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        let current = load_task_tx(&tx, task_id)?.ok_or_else(|| TaskError::not_found(task_id))?;
        if current.state.is_terminal() {
            return Err(TaskError::new(
                codes::INVALID_TRANSITION,
                format!("任务 {task_id} 已处于终态 {}", current.state),
            ));
        }
        let attempt = current.current_attempt.saturating_add(1);
        let now = now();
        tx.execute(
            "UPDATE tasks SET current_attempt = ?1, updated_at = ?2 WHERE task_id = ?3",
            params![attempt, now, task_id],
        )
        .map_err(TaskError::store)?;
        tx.execute(
            "INSERT INTO task_attempts (task_id, attempt, started_at) VALUES (?1, ?2, ?3)",
            params![task_id, attempt, now],
        )
        .map_err(TaskError::store)?;
        tx.commit().map_err(TaskError::store)?;
        Ok(attempt)
    }

    /// 所有非终态任务（启动恢复扫描用）。
    pub fn non_terminal_tasks(&self) -> TaskResult<Vec<TaskRecord>> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT * FROM tasks
                 WHERE state NOT IN ('succeeded','failed','cancelled')
                 ORDER BY updated_at ASC",
            )
            .map_err(TaskError::store)?;
        let mapped = statement
            .query_map([], row_to_task)
            .map_err(TaskError::store)?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(TaskError::store)?);
        }
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // events
    // ------------------------------------------------------------------

    pub fn append_event(
        &self,
        task_id: &str,
        event_type: &str,
        attempt: u32,
        payload: Value,
        correlation_id: Option<&str>,
    ) -> TaskResult<TaskEvent> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        let event = append_event_tx(&tx, task_id, event_type, attempt, payload, correlation_id)?;
        tx.commit().map_err(TaskError::store)?;
        Ok(event)
    }

    pub fn list_events(
        &self,
        task_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> TaskResult<Vec<TaskEvent>> {
        let conn = self.lock()?;
        let limit = limit.clamp(1, 1_000) as i64;
        let mut statement = conn
            .prepare(
                "SELECT task_id, sequence, event_id, event_type, occurred_at, attempt,
                        payload_json, correlation_id
                 FROM task_events WHERE task_id = ?1 AND sequence > ?2
                 ORDER BY sequence ASC LIMIT ?3",
            )
            .map_err(TaskError::store)?;
        let mapped = statement
            .query_map(params![task_id, after_sequence as i64, limit], row_to_event)
            .map_err(TaskError::store)?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(TaskError::store)?);
        }
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // checkpoints
    // ------------------------------------------------------------------

    pub fn create_checkpoint(&self, checkpoint: &TaskCheckpoint) -> TaskResult<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        tx.execute(
            "INSERT INTO task_checkpoints (
                checkpoint_id, task_id, attempt, created_at, phase,
                state_snapshot_ref, workspace_revision, artifact_refs_json, metadata_json
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                checkpoint.checkpoint_id,
                checkpoint.task_id.as_str(),
                checkpoint.attempt,
                checkpoint.created_at,
                checkpoint.phase,
                checkpoint.state_snapshot_ref,
                checkpoint.workspace_revision,
                serde_json::to_string(&checkpoint.artifact_refs).map_err(TaskError::store)?,
                serde_json::to_string(&checkpoint.metadata).map_err(TaskError::store)?,
            ],
        )
        .map_err(TaskError::store)?;
        tx.execute(
            "UPDATE tasks SET last_checkpoint_id = ?1, updated_at = ?2 WHERE task_id = ?3",
            params![checkpoint.checkpoint_id, now(), checkpoint.task_id.as_str()],
        )
        .map_err(TaskError::store)?;
        append_event_tx(
            &tx,
            checkpoint.task_id.as_str(),
            event_type::TASK_CHECKPOINT_CREATED,
            checkpoint.attempt,
            json!({
                "checkpoint_id": checkpoint.checkpoint_id,
                "phase": checkpoint.phase,
                "workspace_revision": checkpoint.workspace_revision,
            }),
            None,
        )?;
        tx.commit().map_err(TaskError::store)?;
        Ok(())
    }

    pub fn latest_checkpoint(&self, task_id: &str) -> TaskResult<Option<TaskCheckpoint>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT checkpoint_id, task_id, attempt, created_at, phase,
                    state_snapshot_ref, workspace_revision, artifact_refs_json, metadata_json
             FROM task_checkpoints WHERE task_id = ?1
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![task_id],
            row_to_checkpoint,
        )
        .optional()
        .map_err(TaskError::store)
    }

    // ------------------------------------------------------------------
    // artifacts
    // ------------------------------------------------------------------

    pub fn add_artifact(&self, artifact: &TaskArtifact) -> TaskResult<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        tx.execute(
            "INSERT INTO task_artifacts (
                artifact_id, task_id, attempt, kind, uri_or_path, sha256,
                size_bytes, created_at, verification_status
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                artifact.artifact_id,
                artifact.task_id.as_str(),
                artifact.attempt,
                artifact.kind,
                artifact.uri_or_path,
                artifact.sha256,
                artifact.size_bytes,
                artifact.created_at,
                verification_status_str(artifact.verification_status),
            ],
        )
        .map_err(TaskError::store)?;
        append_event_tx(
            &tx,
            artifact.task_id.as_str(),
            event_type::TASK_ARTIFACT_CREATED,
            artifact.attempt,
            json!({
                "artifact_id": artifact.artifact_id,
                "kind": artifact.kind,
                "uri_or_path": artifact.uri_or_path,
                "sha256": artifact.sha256,
            }),
            None,
        )?;
        tx.commit().map_err(TaskError::store)?;
        Ok(())
    }

    pub fn list_artifacts(&self, task_id: &str) -> TaskResult<Vec<TaskArtifact>> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT artifact_id, task_id, attempt, kind, uri_or_path, sha256,
                        size_bytes, created_at, verification_status
                 FROM task_artifacts WHERE task_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(TaskError::store)?;
        let mapped = statement
            .query_map(params![task_id], row_to_artifact)
            .map_err(TaskError::store)?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(TaskError::store)?);
        }
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // approvals
    // ------------------------------------------------------------------

    pub fn request_approval(&self, approval: &TaskApproval) -> TaskResult<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        let task = load_task_tx(&tx, approval.task_id.as_str())?
            .ok_or_else(|| TaskError::not_found(approval.task_id.as_str()))?;
        if task.state.is_terminal() {
            return Err(TaskError::new(
                codes::INVALID_TRANSITION,
                format!("任务 {} 已结束，不能再请求审批", approval.task_id),
            ));
        }
        ensure_transition(task.state, TaskState::AwaitingApproval)?;
        tx.execute(
            "INSERT INTO task_approvals (
                approval_id, task_id, action, scope_json, requested_at,
                expires_at, decided_at, decision, decided_by
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                approval.approval_id,
                approval.task_id.as_str(),
                approval.action,
                serde_json::to_string(&approval.scope).map_err(TaskError::store)?,
                approval.requested_at,
                approval.expires_at,
                approval.decided_at,
                approval.decision.map(decision_str),
                approval.decided_by,
            ],
        )
        .map_err(TaskError::store)?;
        tx.execute(
            "UPDATE tasks SET state = 'awaiting_approval', updated_at = ?1 WHERE task_id = ?2",
            params![now(), approval.task_id.as_str()],
        )
        .map_err(TaskError::store)?;
        append_event_tx(
            &tx,
            approval.task_id.as_str(),
            event_type::TASK_APPROVAL_REQUESTED,
            task.current_attempt,
            json!({
                "approval_id": approval.approval_id,
                "action": approval.action,
                "scope": approval.scope,
                "expires_at": approval.expires_at,
            }),
            None,
        )?;
        tx.commit().map_err(TaskError::store)?;
        Ok(())
    }

    pub fn decide_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
        decided_by: Option<&str>,
    ) -> TaskResult<TaskApproval> {
        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(TaskError::store)?;
        let approval = tx
            .query_row(
                "SELECT approval_id, task_id, action, scope_json, requested_at,
                        expires_at, decided_at, decision, decided_by
                 FROM task_approvals WHERE approval_id = ?1",
                params![approval_id],
                row_to_approval,
            )
            .optional()
            .map_err(TaskError::store)?
            .ok_or_else(|| {
                TaskError::new(codes::TASK_NOT_FOUND, format!("审批 {approval_id} 不存在"))
            })?;
        if approval.decision.is_some() {
            return Err(TaskError::new(
                codes::INVALID_TRANSITION,
                format!("审批 {approval_id} 已决策，不能重复写入"),
            ));
        }
        let now = now();
        let task = load_task_tx(&tx, approval.task_id.as_str())?
            .ok_or_else(|| TaskError::not_found(approval.task_id.as_str()))?;
        let next_state = match decision {
            ApprovalDecision::Approved => TaskState::Running,
            ApprovalDecision::Rejected => TaskState::Cancelled,
        };
        ensure_transition(task.state, next_state)?;
        tx.execute(
            "UPDATE task_approvals SET decided_at = ?1, decision = ?2, decided_by = ?3
             WHERE approval_id = ?4",
            params![now, decision_str(decision), decided_by, approval_id],
        )
        .map_err(TaskError::store)?;
        tx.execute(
            "UPDATE tasks SET state = ?1, updated_at = ?2 WHERE task_id = ?3",
            params![next_state.as_str(), now, approval.task_id.as_str()],
        )
        .map_err(TaskError::store)?;
        append_event_tx(
            &tx,
            approval.task_id.as_str(),
            event_type::TASK_APPROVAL_DECIDED,
            task.current_attempt,
            json!({
                "approval_id": approval_id,
                "decision": decision_str(decision),
                "decided_by": decided_by,
                "task_state": next_state.as_str(),
            }),
            None,
        )?;
        tx.commit().map_err(TaskError::store)?;
        // 同上：先释放连接锁，避免 get_approval 重入死锁。
        drop(conn);
        self.get_approval(approval_id)?.ok_or_else(|| {
            TaskError::new(codes::TASK_NOT_FOUND, format!("审批 {approval_id} 不存在"))
        })
    }

    pub fn get_approval(&self, approval_id: &str) -> TaskResult<Option<TaskApproval>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT approval_id, task_id, action, scope_json, requested_at,
                    expires_at, decided_at, decision, decided_by
             FROM task_approvals WHERE approval_id = ?1",
            params![approval_id],
            row_to_approval,
        )
        .optional()
        .map_err(TaskError::store)
    }

    pub fn list_approvals(
        &self,
        task_id: &str,
        pending_only: bool,
    ) -> TaskResult<Vec<TaskApproval>> {
        let conn = self.lock()?;
        let sql = if pending_only {
            "SELECT approval_id, task_id, action, scope_json, requested_at,
                    expires_at, decided_at, decision, decided_by
             FROM task_approvals WHERE task_id = ?1 AND decision IS NULL
             ORDER BY requested_at ASC"
        } else {
            "SELECT approval_id, task_id, action, scope_json, requested_at,
                    expires_at, decided_at, decision, decided_by
             FROM task_approvals WHERE task_id = ?1 ORDER BY requested_at ASC"
        };
        let mut statement = conn.prepare(sql).map_err(TaskError::store)?;
        let mapped = statement
            .query_map(params![task_id], row_to_approval)
            .map_err(TaskError::store)?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(TaskError::store)?);
        }
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // idempotency
    // ------------------------------------------------------------------

    /// 记录幂等键。重复键返回 `IDEMPOTENCY_CONFLICT`（调用方先查再写）。
    pub fn put_idempotency(&self, key: &str, task_id: &str, payload: &Value) -> TaskResult<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO task_idempotency_keys (key, task_id, created_at, payload_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                key,
                task_id,
                now(),
                serde_json::to_string(payload).map_err(TaskError::store)?
            ],
        )
        .map_err(|error| {
            if matches!(
                error,
                rusqlite::Error::SqliteFailure(ref inner, _)
                    if inner.code == rusqlite::ErrorCode::ConstraintViolation
            ) {
                TaskError::new(codes::IDEMPOTENCY_CONFLICT, format!("幂等键 {key} 已存在"))
            } else {
                TaskError::store(error)
            }
        })?;
        Ok(())
    }

    pub fn get_idempotency(&self, key: &str) -> TaskResult<Option<(String, Value)>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT task_id, payload_json FROM task_idempotency_keys WHERE key = ?1",
                params![key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(TaskError::store)?;
        row.map(|(task_id, payload)| {
            let value: Value = serde_json::from_str(&payload).map_err(TaskError::store)?;
            Ok((task_id, value))
        })
        .transpose()
    }
}

// ----------------------------------------------------------------------
// helpers
// ----------------------------------------------------------------------

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn event_type_for_state(state: TaskState) -> &'static str {
    match state {
        TaskState::Succeeded => event_type::TASK_SUCCEEDED,
        TaskState::Failed => event_type::TASK_FAILED,
        TaskState::Cancelled => event_type::TASK_CANCELLED,
        TaskState::Interrupted => event_type::TASK_INTERRUPTED,
        TaskState::Paused => event_type::TASK_PAUSED,
        TaskState::CancelRequested => event_type::TASK_CANCEL_REQUESTED,
        TaskState::Verifying => event_type::TASK_VERIFICATION_STARTED,
        TaskState::Repairing => event_type::TASK_REPAIR_STARTED,
        TaskState::Planning => event_type::TASK_PLANNING_STARTED,
        TaskState::Running => event_type::TASK_EXECUTION_STARTED,
        _ => event_type::TASK_STATE_CHANGED,
    }
}

fn decision_str(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Approved => "approved",
        ApprovalDecision::Rejected => "rejected",
    }
}

fn verification_status_str(status: ArtifactVerification) -> &'static str {
    match status {
        ArtifactVerification::Pending => "pending",
        ArtifactVerification::Verified => "verified",
        ArtifactVerification::Failed => "failed",
        ArtifactVerification::NotApplicable => "not_applicable",
    }
}

fn verification_status_parse(value: &str) -> ArtifactVerification {
    match value {
        "verified" => ArtifactVerification::Verified,
        "failed" => ArtifactVerification::Failed,
        "not_applicable" => ArtifactVerification::NotApplicable,
        _ => ArtifactVerification::Pending,
    }
}

fn append_event_tx(
    tx: &rusqlite::Transaction<'_>,
    task_id: &str,
    event_type: &str,
    attempt: u32,
    payload: Value,
    correlation_id: Option<&str>,
) -> TaskResult<TaskEvent> {
    let sequence: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM task_events WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .map_err(TaskError::store)?;
    let event = TaskEvent {
        task_id: TaskId::from(task_id),
        sequence: sequence as u64,
        event_id: uuid::Uuid::new_v4().to_string(),
        event_type: event_type.to_string(),
        occurred_at: now(),
        attempt,
        payload,
        correlation_id: correlation_id.map(str::to_string),
    };
    tx.execute(
        "INSERT INTO task_events (
            task_id, sequence, event_id, event_type, occurred_at,
            attempt, payload_json, correlation_id
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            task_id,
            sequence,
            event.event_id,
            event.event_type,
            event.occurred_at,
            event.attempt,
            serde_json::to_string(&event.payload).map_err(TaskError::store)?,
            event.correlation_id,
        ],
    )
    .map_err(TaskError::store)?;
    Ok(event)
}

fn load_task(conn: &Connection, task_id: &str) -> TaskResult<Option<TaskRecord>> {
    conn.query_row(
        "SELECT task_id, title, user_goal, state, execution_strategy, current_attempt,
                created_at, updated_at, last_checkpoint_id, terminal_summary,
                budget_json, policy_profile, workspace_ref, session_refs_json, schema_version,
                payload_json
         FROM tasks WHERE task_id = ?1",
        params![task_id],
        row_to_task,
    )
    .optional()
    .map_err(TaskError::store)
}

fn load_task_tx(tx: &rusqlite::Transaction<'_>, task_id: &str) -> TaskResult<Option<TaskRecord>> {
    tx.query_row(
        "SELECT task_id, title, user_goal, state, execution_strategy, current_attempt,
                created_at, updated_at, last_checkpoint_id, terminal_summary,
                budget_json, policy_profile, workspace_ref, session_refs_json, schema_version,
                payload_json
         FROM tasks WHERE task_id = ?1",
        params![task_id],
        row_to_task,
    )
    .optional()
    .map_err(TaskError::store)
}

/// 幂等补列（sqlite 没有 `ADD COLUMN IF NOT EXISTS`）。
fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    declaration: &str,
) -> TaskResult<()> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(TaskError::store)?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(TaskError::store)?;
    let mut exists = false;
    for name in names {
        if name.map_err(TaskError::store)? == column {
            exists = true;
            break;
        }
    }
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
        ))
        .map_err(TaskError::store)?;
    }
    Ok(())
}

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let state_text: String = row.get(3)?;
    let strategy_text: String = row.get(4)?;
    let budget_text: String = row.get(10)?;
    let session_refs_text: String = row.get(13)?;
    Ok(TaskRecord {
        task_id: TaskId::from(row.get::<_, String>(0)?),
        title: row.get(1)?,
        user_goal: row.get(2)?,
        state: TaskState::parse(&state_text).unwrap_or(TaskState::Failed),
        execution_strategy: ExecutionStrategy::parse(&strategy_text)
            .unwrap_or(ExecutionStrategy::SingleAgent),
        current_attempt: row.get::<_, i64>(5)? as u32,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        last_checkpoint_id: row.get(8)?,
        terminal_summary: row.get(9)?,
        budget: serde_json::from_str(&budget_text).unwrap_or_default(),
        policy_profile: row.get(11)?,
        workspace_ref: row.get(12)?,
        session_refs: serde_json::from_str(&session_refs_text).unwrap_or_default(),
        // 按列名取：新库与 ALTER 迁移库的列序可能不同。
        payload: row
            .get::<_, Option<String>>("payload_json")?
            .and_then(|text| serde_json::from_str(&text).ok()),
        schema_version: row.get::<_, i64>(14)? as u32,
    })
}

fn row_to_event(row: &Row<'_>) -> rusqlite::Result<TaskEvent> {
    let payload_text: String = row.get(6)?;
    Ok(TaskEvent {
        task_id: TaskId::from(row.get::<_, String>(0)?),
        sequence: row.get::<_, i64>(1)? as u64,
        event_id: row.get(2)?,
        event_type: row.get(3)?,
        occurred_at: row.get(4)?,
        attempt: row.get::<_, i64>(5)? as u32,
        payload: serde_json::from_str(&payload_text).unwrap_or(Value::Null),
        correlation_id: row.get(7)?,
    })
}

fn row_to_checkpoint(row: &Row<'_>) -> rusqlite::Result<TaskCheckpoint> {
    let artifact_refs_text: String = row.get(7)?;
    let metadata_text: String = row.get(8)?;
    Ok(TaskCheckpoint {
        checkpoint_id: row.get(0)?,
        task_id: TaskId::from(row.get::<_, String>(1)?),
        attempt: row.get::<_, i64>(2)? as u32,
        created_at: row.get(3)?,
        phase: row.get(4)?,
        state_snapshot_ref: row.get(5)?,
        workspace_revision: row.get(6)?,
        artifact_refs: serde_json::from_str(&artifact_refs_text).unwrap_or_default(),
        metadata: serde_json::from_str(&metadata_text).unwrap_or(Value::Null),
    })
}

fn row_to_artifact(row: &Row<'_>) -> rusqlite::Result<TaskArtifact> {
    let status_text: String = row.get(8)?;
    Ok(TaskArtifact {
        artifact_id: row.get(0)?,
        task_id: TaskId::from(row.get::<_, String>(1)?),
        attempt: row.get::<_, i64>(2)? as u32,
        kind: row.get(3)?,
        uri_or_path: row.get(4)?,
        sha256: row.get(5)?,
        size_bytes: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
        created_at: row.get(7)?,
        verification_status: verification_status_parse(&status_text),
    })
}

fn row_to_approval(row: &Row<'_>) -> rusqlite::Result<TaskApproval> {
    let scope_text: String = row.get(3)?;
    let decision_text: Option<String> = row.get(7)?;
    Ok(TaskApproval {
        approval_id: row.get(0)?,
        task_id: TaskId::from(row.get::<_, String>(1)?),
        action: row.get(2)?,
        scope: serde_json::from_str(&scope_text).unwrap_or(Value::Null),
        requested_at: row.get(4)?,
        expires_at: row.get(5)?,
        decided_at: row.get(6)?,
        decision: decision_text.as_deref().and_then(ApprovalDecision::parse),
        decided_by: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> TaskStore {
        TaskStore::open_in_memory().unwrap()
    }

    fn record(goal: &str) -> TaskRecord {
        TaskRecord::new(
            goal,
            goal,
            ExecutionStrategy::SingleAgent,
            TaskBudget::default(),
            "workspace",
        )
    }

    #[test]
    fn create_and_read_roundtrip() {
        let store = store();
        let task = record("写一个测试");
        store.create_task(&task).unwrap();
        let loaded = store.get_task(task.task_id.as_str()).unwrap().unwrap();
        assert_eq!(loaded.user_goal, "写一个测试");
        assert_eq!(loaded.state, TaskState::Draft);
        let events = store.list_events(task.task_id.as_str(), 0, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, event_type::TASK_CREATED);
        assert_eq!(events[0].sequence, 1);
    }

    #[test]
    fn transitions_are_validated_and_events_are_monotonic() {
        let store = store();
        let task = record("状态机");
        store.create_task(&task).unwrap();
        assert!(store
            .transition(task.task_id.as_str(), TaskState::Succeeded, None, None)
            .is_err());
        store
            .transition(task.task_id.as_str(), TaskState::Queued, None, None)
            .unwrap();
        store
            .transition(task.task_id.as_str(), TaskState::Running, None, None)
            .unwrap();
        let paused = store
            .transition(
                task.task_id.as_str(),
                TaskState::Paused,
                Some("用户暂停"),
                None,
            )
            .unwrap();
        assert_eq!(paused.state, TaskState::Paused);
        let events = store.list_events(task.task_id.as_str(), 0, 100).unwrap();
        let sequences: Vec<u64> = events.iter().map(|event| event.sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3, 4]);
    }

    #[test]
    fn payload_roundtrips_and_stays_optional() {
        let store = store();
        let mut with_payload = record("computer-use 任务");
        with_payload.execution_strategy = ExecutionStrategy::ComputerUse;
        with_payload.payload = Some(json!({
            "target_app": "notepad",
            "goals": [{"anchor_text": "发送", "action": "click", "value": ""}]
        }));
        store.create_task(&with_payload).unwrap();
        let loaded = store
            .get_task(with_payload.task_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.payload, with_payload.payload);
        assert_eq!(loaded.execution_strategy, ExecutionStrategy::ComputerUse);

        let plain = record("single agent 任务");
        store.create_task(&plain).unwrap();
        assert!(store
            .get_task(plain.task_id.as_str())
            .unwrap()
            .unwrap()
            .payload
            .is_none());
    }

    #[test]
    fn opening_v1_database_adds_payload_column_and_keeps_old_tasks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent_tasks.db");
        {
            // v1 建表语句：没有 payload_json（2026-11 之前的任务库）。
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE tasks (
                    task_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    user_goal TEXT NOT NULL,
                    state TEXT NOT NULL,
                    execution_strategy TEXT NOT NULL,
                    current_attempt INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    last_checkpoint_id TEXT,
                    terminal_summary TEXT,
                    budget_json TEXT NOT NULL,
                    policy_profile TEXT NOT NULL,
                    workspace_ref TEXT,
                    session_refs_json TEXT NOT NULL DEFAULT '[]',
                    schema_version INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tasks (task_id, title, user_goal, state, execution_strategy,
                    current_attempt, created_at, updated_at, budget_json, policy_profile,
                    session_refs_json, schema_version)
                 VALUES ('legacy', '旧任务', '旧任务', 'queued', 'single_agent', 0,
                    '2026-10-01T00:00:00Z', '2026-10-01T00:00:00Z', ?1, 'workspace', '[]', 1)",
                params![json!(TaskBudget::default()).to_string()],
            )
            .unwrap();
        }
        let store = TaskStore::open(&path).unwrap();
        let loaded = store.get_task("legacy").unwrap().unwrap();
        assert_eq!(loaded.user_goal, "旧任务");
        assert!(loaded.payload.is_none(), "老任务按 NULL 加载");
        // 迁移后新任务可以带 payload 写入并读回。
        let mut task = record("迁移后新任务");
        task.payload = Some(json!({"target_app": "notepad"}));
        store.create_task(&task).unwrap();
        assert_eq!(
            store
                .get_task(task.task_id.as_str())
                .unwrap()
                .unwrap()
                .payload,
            task.payload
        );
    }

    #[test]
    fn attempts_are_monotonic() {
        let store = store();
        let task = record("attempts");
        store.create_task(&task).unwrap();
        assert_eq!(store.bump_attempt(task.task_id.as_str()).unwrap(), 1);
        assert_eq!(store.bump_attempt(task.task_id.as_str()).unwrap(), 2);
        let loaded = store.get_task(task.task_id.as_str()).unwrap().unwrap();
        assert_eq!(loaded.current_attempt, 2);
    }

    #[test]
    fn checkpoint_and_artifact_are_persisted() {
        let store = store();
        let task = record("checkpoint");
        store.create_task(&task).unwrap();
        let checkpoint = TaskCheckpoint {
            checkpoint_id: "cp-1".into(),
            task_id: task.task_id.clone(),
            attempt: 0,
            created_at: now(),
            phase: "running".into(),
            state_snapshot_ref: "snapshots/cp-1.json".into(),
            workspace_revision: Some("abc123".into()),
            artifact_refs: vec!["art-1".into()],
            metadata: json!({"next": "step-2"}),
        };
        store.create_checkpoint(&checkpoint).unwrap();
        let loaded = store
            .latest_checkpoint(task.task_id.as_str())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.checkpoint_id, "cp-1");
        assert_eq!(loaded.artifact_refs, vec!["art-1"]);

        let artifact = TaskArtifact {
            artifact_id: "art-1".into(),
            task_id: task.task_id.clone(),
            attempt: 0,
            kind: "report".into(),
            uri_or_path: "artifacts/report.md".into(),
            sha256: Some("deadbeef".into()),
            size_bytes: Some(42),
            created_at: now(),
            verification_status: ArtifactVerification::Verified,
        };
        store.add_artifact(&artifact).unwrap();
        assert_eq!(
            store.list_artifacts(task.task_id.as_str()).unwrap().len(),
            1
        );
    }

    #[test]
    fn approval_moves_task_state() {
        let store = store();
        let task = record("审批");
        store.create_task(&task).unwrap();
        store
            .transition(task.task_id.as_str(), TaskState::Queued, None, None)
            .unwrap();
        store
            .transition(task.task_id.as_str(), TaskState::Planning, None, None)
            .unwrap();
        let approval = TaskApproval {
            approval_id: "ap-1".into(),
            task_id: task.task_id.clone(),
            action: "desktop_click".into(),
            scope: json!({"app": "notepad"}),
            requested_at: now(),
            expires_at: None,
            decided_at: None,
            decision: None,
            decided_by: None,
        };
        store.request_approval(&approval).unwrap();
        assert_eq!(
            store
                .get_task(task.task_id.as_str())
                .unwrap()
                .unwrap()
                .state,
            TaskState::AwaitingApproval
        );
        let decided = store
            .decide_approval("ap-1", ApprovalDecision::Approved, Some("user"))
            .unwrap();
        assert_eq!(decided.decision, Some(ApprovalDecision::Approved));
        assert_eq!(
            store
                .get_task(task.task_id.as_str())
                .unwrap()
                .unwrap()
                .state,
            TaskState::Running
        );
        // 重复决策被拒绝。
        assert!(store
            .decide_approval("ap-1", ApprovalDecision::Rejected, None)
            .is_err());
    }

    #[test]
    fn idempotency_conflict_is_reported() {
        let store = store();
        store.put_idempotency("k1", "t1", &json!({"x": 1})).unwrap();
        let error = store
            .put_idempotency("k1", "t1", &json!({"x": 2}))
            .unwrap_err();
        assert_eq!(error.code, codes::IDEMPOTENCY_CONFLICT);
        assert_eq!(store.get_idempotency("k1").unwrap().unwrap().0, "t1");
    }
}

//! SQLite 会话存储：`<appdata>/index.db`（对应技术文档 5.7）。
//!
//! R8：迁移框架——`PRAGMA user_version` + 顺序迁移表（`MIGRATIONS`），
//! `open` 时自动迁移；迁移失败降级只读并记录提示，禁止运行时隐式 ALTER。

use crate::audit::AuditEntry;
use crate::error::AgentError;
use crate::session::{Session, SessionStore, SnapshotEntry, TurnEventRecord};
use crate::storage_crypto::{decrypt_file_envelope, encrypt_file_envelope, StorageCryptoError};
use owo_agent_protocol::SseEvent;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// 库文件信封加密（R9）：整库拷贝加密（备份/导出面），运行时 SQLite 保持原语义。
/// 加密文件后缀（`.owo-crypt` 由 storage_crypto 信封格式承载）。
pub fn encrypt_db_copy(source: &Path, target: &Path) -> Result<(), StorageCryptoError> {
    let bytes = std::fs::read(source)?;
    encrypt_file_envelope(target, &bytes)
}

/// 库文件信封解密：`<target>.owo-crypt` → 解密写入 target（覆盖）。
pub fn decrypt_db_copy(source: &Path, target: &Path) -> Result<(), StorageCryptoError> {
    let plain = decrypt_file_envelope(source)?;
    std::fs::write(target, plain)?;
    Ok(())
}

/// 打开加密库文件的只读快照：解密到临时文件并 open（调用方负责清理临时文件）。
pub fn open_encrypted_snapshot(
    encrypted_path: &Path,
    tmp_dir: &Path,
) -> Result<(SqliteSessionStore, std::path::PathBuf), AgentError> {
    let tmp = tmp_dir.join(format!("owo-db-dec-{}", uuid::Uuid::new_v4()));
    decrypt_db_copy(encrypted_path, &tmp)
        .map_err(|error| AgentError::Session(format!("加密库解密失败：{error}")))?;
    let store = SqliteSessionStore::open(&tmp)?;
    Ok((store, tmp))
}

mod migrations;

use migrations::{base_schema, run_migrations, user_version};
pub use migrations::{Migration, MIGRATIONS};

/// 迁移运行状态（供健康/状态面板展示；只读降级时 last_error 给出原因）。
#[derive(Debug, Clone, Default)]
pub struct MigrationStatus {
    /// 当前 schema 版本（PRAGMA user_version）。
    pub schema_version: i64,
    /// 本次打开已应用的迁移（"vN: name"）。
    pub applied: Vec<String>,
    /// 未应用的迁移（version > schema_version）。
    pub pending: Vec<String>,
    /// 迁移失败后降级只读（拒绝写入，数据不损坏）。
    pub read_only: bool,
    /// 迁移失败原因（read_only 时的提示）。
    pub last_error: Option<String>,
}

pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
    status: MigrationStatus,
}

/// 审计查询条件（v0.5 生产加固：分页 + 精确过滤 + 模糊搜索）。
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// 返回条数；0 时取默认 100，上限 1000。
    pub limit: usize,
    pub offset: usize,
    pub event: Option<String>,
    pub tool: Option<String>,
    pub approved: Option<bool>,
    /// 对 detail/event/tool/session_id 做 LIKE 模糊搜索（%/_ 自动转义）。
    pub q: Option<String>,
}

fn like_escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

impl SqliteSessionStore {
    pub fn open(path: &Path) -> Result<Self, AgentError> {
        Self::open_with_migrations(path, MIGRATIONS)
    }

    /// 打开并自动迁移（MIGRATIONS 或测试注入的迁移表）；迁移失败降级只读。
    fn open_with_migrations(path: &Path, migrations: &[Migration]) -> Result<Self, AgentError> {
        let mut conn = Connection::open(path).map_err(sqlite_error)?;
        // 多连接（会话库 + 审计 + 并发回合）共存：写锁竞争在 busy_timeout 内自旋等待，
        // 避免瞬时 SQLITE_BUSY 被当成永久失败。
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(sqlite_error)?;
        conn.execute_batch(base_schema()).map_err(sqlite_error)?;
        let mut status = MigrationStatus {
            schema_version: user_version(&conn)?,
            ..MigrationStatus::default()
        };
        match run_migrations(&mut conn, migrations) {
            Ok(applied) => {
                status.applied = applied;
                status.schema_version = user_version(&conn)?;
            }
            Err(error) => {
                // 迁移失败：降级只读并提示，拒绝继续写入（数据保持原状）。
                let read_only_conn =
                    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                        .map_err(sqlite_error)?;
                let _ = read_only_conn.busy_timeout(std::time::Duration::from_millis(5000));
                status.read_only = true;
                status.last_error = Some(error.to_string());
                return Ok(Self {
                    conn: Mutex::new(read_only_conn),
                    status,
                });
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
            status,
        })
    }

    /// 迁移状态（含未应用清单；供 /server/status 与面板展示）。
    pub fn migration_status(&self) -> MigrationStatus {
        let mut status = self.status.clone();
        status.pending = MIGRATIONS
            .iter()
            .filter(|migration| migration.version > status.schema_version)
            .map(|migration| format!("v{}: {}", migration.version, migration.name))
            .collect();
        status
    }

    pub fn is_read_only(&self) -> bool {
        self.status.read_only
    }

    /// 清空会话与审计，返回 (会话数, 审计数)。
    pub fn clear_all(&self) -> Result<(usize, usize), AgentError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        let transaction = conn.transaction().map_err(sqlite_error)?;
        let sessions = transaction
            .execute("DELETE FROM sessions", [])
            .map_err(sqlite_error)?;
        let audit = transaction
            .execute("DELETE FROM audit", [])
            .map_err(sqlite_error)?;
        transaction
            .execute("DELETE FROM turn_events", [])
            .map_err(sqlite_error)?;
        transaction
            .execute("DELETE FROM turn_event_cursors", [])
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok((sessions, audit))
    }

    /// PRAGMA integrity_check 结果（"ok" 表示完整）。
    pub fn integrity_check(&self) -> Result<String, AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(sqlite_error)
    }

    /// 当前 (会话数, 审计数)。
    pub fn counts(&self) -> (usize, usize) {
        let conn = match self.conn.lock() {
            Ok(conn) => conn,
            Err(_) => return (0, 0),
        };
        let sessions = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or_default();
        let audit = conn
            .query_row("SELECT COUNT(*) FROM audit", [], |row| row.get::<_, i64>(0))
            .unwrap_or_default();
        (sessions as usize, audit as usize)
    }

    fn save_locked(conn: &Connection, session: &Session) -> Result<(), AgentError> {
        conn.execute(
            "INSERT INTO sessions (
                 id, workspace, model, system_prompt, messages_json, snapshots_json,
                 execution_receipts_json, created_at, updated_at, parent_id, fork_point,
                 redo_json, message_redo_json, title, archived, pinned, model_override,
                 validation_receipts_json, single_verification_plan_json,
                 single_verification_plan_input_sha256, single_verification_plan_turn_id,
                 single_review_issues_json, completion_record_json, goal_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
             ON CONFLICT(id) DO UPDATE SET
                 workspace=excluded.workspace,
                 model=excluded.model,
                 system_prompt=excluded.system_prompt,
                 messages_json=excluded.messages_json,
                 snapshots_json=excluded.snapshots_json,
                 execution_receipts_json=excluded.execution_receipts_json,
                 updated_at=excluded.updated_at,
                 parent_id=excluded.parent_id,
                 fork_point=excluded.fork_point,
                 redo_json=excluded.redo_json,
                 message_redo_json=excluded.message_redo_json,
                 title=excluded.title,
                 archived=excluded.archived,
                 pinned=excluded.pinned,
                 model_override=excluded.model_override,
                 validation_receipts_json=excluded.validation_receipts_json,
                 single_verification_plan_json=excluded.single_verification_plan_json,
                 single_verification_plan_input_sha256=excluded.single_verification_plan_input_sha256,
                 single_verification_plan_turn_id=excluded.single_verification_plan_turn_id,
                 single_review_issues_json=excluded.single_review_issues_json,
                 completion_record_json=excluded.completion_record_json,
                 goal_json=excluded.goal_json",
            params![
                session.id,
                session.workspace.to_string_lossy(),
                session.model,
                session.system_prompt,
                serde_json::to_string(&session.messages).map_err(json_error)?,
                serde_json::to_string(&session.snapshots).map_err(json_error)?,
                serde_json::to_string(&session.execution_receipts).map_err(json_error)?,
                session.created_at,
                session.updated_at,
                session.parent_id,
                session.fork_point.map(|point| point as i64),
                serde_json::to_string(&session.redo_stack).map_err(json_error)?,
                serde_json::to_string(&session.message_redo_stack).map_err(json_error)?,
                session.title,
                i64::from(session.archived),
                i64::from(session.pinned),
                session.model_override,
                serde_json::to_string(&session.validation_receipts).map_err(json_error)?,
                session.single_verification_plan.as_ref().map(serde_json::to_string).transpose().map_err(json_error)?,
                session.single_verification_plan_input_sha256,
                session.single_verification_plan_turn_id,
                serde_json::to_string(&session.single_review_issues).map_err(json_error)?,
                session.completion_record.as_ref().map(serde_json::to_string).transpose().map_err(json_error)?,
                session.goal.as_ref().map(serde_json::to_string).transpose().map_err(json_error)?,
            ],
        )
        .map_err(sqlite_error)?;
        Ok(())
    }

    fn load_locked(conn: &Connection, id: &str) -> Result<Session, AgentError> {
        let row = conn
            .query_row(
                "SELECT id, workspace, model, system_prompt, messages_json, snapshots_json,
                        execution_receipts_json, created_at, updated_at, parent_id, fork_point,
                        redo_json, message_redo_json, title, archived, pinned, model_override,
                        validation_receipts_json, single_verification_plan_json,
                        single_verification_plan_input_sha256, single_verification_plan_turn_id,
                        single_review_issues_json, completion_record_json, goal_json
                 FROM sessions WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, bool>(14)?,
                        row.get::<_, bool>(15)?,
                        row.get::<_, Option<String>>(16)?,
                        row.get::<_, String>(17)?,
                        row.get::<_, Option<String>>(18)?,
                        row.get::<_, Option<String>>(19)?,
                        row.get::<_, Option<String>>(20)?,
                        row.get::<_, String>(21)?,
                        row.get::<_, Option<String>>(22)?,
                        row.get::<_, Option<String>>(23)?,
                    ))
                },
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => {
                    AgentError::Session(format!("会话不存在：{id}"))
                }
                other => sqlite_error(other),
            })?;
        Ok(Session {
            id: row.0,
            workspace: row.1.into(),
            model: row.2,
            system_prompt: row.3,
            messages: serde_json::from_str(&row.4).map_err(json_error)?,
            snapshots: serde_json::from_str::<HashMap<String, SnapshotEntry>>(&row.5)
                .map_err(json_error)?,
            execution_receipts: serde_json::from_str(&row.6).map_err(json_error)?,
            validation_receipts: serde_json::from_str(&row.17).map_err(json_error)?,
            single_verification_plan: row
                .18
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(json_error)?,
            single_verification_plan_input_sha256: row.19,
            single_verification_plan_turn_id: row.20,
            single_review_issues: serde_json::from_str(&row.21).map_err(json_error)?,
            completion_record: row
                .22
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(json_error)?,
            active_task_context: None,
            transient_model_calls: Vec::new(),
            created_at: row.7,
            updated_at: row.8,
            parent_id: row.9,
            fork_point: row.10.map(|point| point as usize),
            redo_stack: serde_json::from_str(&row.11).map_err(json_error)?,
            message_redo_stack: serde_json::from_str(&row.12).map_err(json_error)?,
            title: row.13,
            archived: row.14,
            pinned: row.15,
            model_override: row.16,
            // 任务清单暂不落库（会话内存态）；需要跨重启保留时再补列迁移。
            todos: Vec::new(),
            // 目标模式状态落库（migration v10）：重开进程 /resume 后目标与进度不丢。
            goal: row
                .23
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(json_error)?,
        })
    }
}

impl SessionStore for SqliteSessionStore {
    /// 删除会话及其回合事件/游标。
    fn remove(&self, id: &str) -> Result<(), AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        // 先确认存在：不能"删了 0 行也回报成功"，否则界面显示删除成功而会话还在。
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .map_err(|error| AgentError::Session(format!("查询会话失败：{error}")))?;
        if count == 0 {
            return Err(AgentError::Session(format!("会话不存在：{id}")));
        }
        // 事件与游标按 session_id 关联，一并清理；否则会留下永远读不到的孤儿记录。
        for statement in [
            "DELETE FROM turn_events WHERE session_id = ?1",
            "DELETE FROM turn_event_cursors WHERE session_id = ?1",
            "DELETE FROM sessions WHERE id = ?1",
        ] {
            conn.execute(statement, [id])
                .map_err(|error| AgentError::Session(format!("删除会话失败：{error}")))?;
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), AgentError> {
        self.clear_all().map(|_| ())
    }

    fn is_read_only(&self) -> bool {
        self.is_read_only()
    }

    fn migration_warning(&self) -> Option<String> {
        let status = self.migration_status();
        if status.read_only {
            return Some(
                status
                    .last_error
                    .unwrap_or_else(|| "迁移失败，已降级只读".into()),
            );
        }
        if !status.pending.is_empty() {
            return Some(format!(
                "存在未应用迁移：{}（重启服务自动应用）",
                status.pending.join("；")
            ));
        }
        None
    }
    fn create(
        &self,
        workspace: &Path,
        model: &str,
        system_prompt: Option<&str>,
    ) -> Result<Session, AgentError> {
        let session = Session::new(workspace, model, system_prompt.map(str::to_string));
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        Self::save_locked(&conn, &session)?;
        Ok(session)
    }

    fn load(&self, id: &str) -> Result<Session, AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        Self::load_locked(&conn, id)
    }

    fn save(&self, session: &Session) -> Result<(), AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        Self::save_locked(&conn, session)
    }

    fn exists(&self, id: &str) -> Result<bool, AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        conn.query_row("SELECT 1 FROM sessions WHERE id = ?1", [id], |_| Ok(()))
            .optional()
            .map(|value| value.is_some())
            .map_err(sqlite_error)
    }

    fn append_turn_event(
        &self,
        session_id: &str,
        turn_id: &str,
        payload: &SseEvent,
    ) -> Result<TurnEventRecord, AgentError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let last_seq = transaction
            .query_row(
                "SELECT last_seq FROM turn_event_cursors WHERE session_id = ?1",
                [session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(sqlite_error)?
            .unwrap_or(0);
        let seq = last_seq
            .checked_add(1)
            .ok_or_else(|| AgentError::Session("回合事件 seq 已耗尽".into()))?;
        let created_at = chrono::Utc::now().to_rfc3339();
        let payload_json = serde_json::to_string(payload)
            .map_err(|error| AgentError::Session(format!("回合事件编码失败：{error}")))?;
        transaction
            .execute(
                "INSERT INTO turn_event_cursors(session_id, last_seq) VALUES (?1, ?2)
                 ON CONFLICT(session_id) DO UPDATE SET last_seq = excluded.last_seq",
                params![session_id, seq],
            )
            .map_err(sqlite_error)?;
        transaction
            .execute(
                "INSERT INTO turn_events(session_id, seq, turn_id, created_at, payload_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![session_id, seq, turn_id, created_at, payload_json],
            )
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        Ok(TurnEventRecord {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            seq: seq as u64,
            created_at,
            payload: payload.clone(),
        })
    }

    fn turn_events_after(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<TurnEventRecord>, AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        let after_seq = i64::try_from(after_seq)
            .map_err(|_| AgentError::Session("回合事件 seq 超出 SQLite 范围".into()))?;
        let limit = limit.clamp(1, 1000) as i64;
        let mut statement = conn
            .prepare(
                "SELECT session_id, turn_id, seq, created_at, payload_json
                 FROM turn_events
                 WHERE session_id = ?1 AND seq > ?2 AND (?3 IS NULL OR turn_id = ?3)
                 ORDER BY seq ASC LIMIT ?4",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map(params![session_id, after_seq, turn_id, limit], |row| {
                let payload_json: String = row.get(4)?;
                let payload = serde_json::from_str(&payload_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        payload_json.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                let seq: i64 = row.get(2)?;
                Ok(TurnEventRecord {
                    session_id: row.get(0)?,
                    turn_id: row.get(1)?,
                    seq: seq as u64,
                    created_at: row.get(3)?,
                    payload,
                })
            })
            .map_err(sqlite_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sqlite_error)
    }

    fn turn_history(
        &self,
        session_id: &str,
    ) -> Result<Option<owo_agent_protocol::SessionTurnHistoryV1>, AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite lock poisoned".into()))?;
        let latest: Option<(String, i64, String, String)> = conn.query_row(
            "SELECT turn_id, seq, created_at, payload_json FROM turn_events WHERE session_id = ?1 ORDER BY seq DESC LIMIT 1",
            [session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(sqlite_error)?;
        let Some((last_turn_id, latest_seq, last_event_at, payload)) = latest else {
            return Ok(None);
        };
        let latest_payload: serde_json::Value =
            serde_json::from_str(&payload).map_err(|e| AgentError::Session(e.to_string()))?;
        let terminal: Option<(String, i64, String, String)> = conn.query_row(
            "SELECT turn_id, seq, created_at, payload_json FROM turn_events
             WHERE session_id = ?1 AND json_extract(payload_json, '$.type') IN ('turn_stats', 'turn_failed')
             ORDER BY seq DESC LIMIT 1",
            [session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(sqlite_error)?;
        let last_terminal = terminal
            .map(|(turn_id, seq, created_at, payload)| {
                serde_json::from_str::<SseEvent>(&payload)
                    .map(|payload| TurnEventRecord {
                        session_id: session_id.to_string(),
                        turn_id,
                        seq: seq as u64,
                        created_at,
                        payload,
                    })
                    .map_err(|e| AgentError::Session(e.to_string()))
            })
            .transpose()?;
        let last_stats: Option<(String, i64, String, String)> = conn.query_row(
            "SELECT turn_id, seq, created_at, payload_json FROM turn_events WHERE session_id = ?1 AND json_extract(payload_json, '$.type') = 'turn_stats' ORDER BY seq DESC LIMIT 1",
            [session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional().map_err(sqlite_error)?;
        let last_stats = last_stats
            .map(|(turn_id, seq, created_at, payload)| {
                serde_json::from_str::<SseEvent>(&payload)
                    .map(|payload| TurnEventRecord {
                        session_id: session_id.to_string(),
                        turn_id,
                        seq: seq as u64,
                        created_at,
                        payload,
                    })
                    .map_err(|e| AgentError::Session(e.to_string()))
            })
            .transpose()?;
        Ok(Some(owo_agent_protocol::SessionTurnHistoryV1 {
            last_turn_id,
            latest_seq: latest_seq as u64,
            last_event_at,
            last_event_type: latest_payload["type"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            last_terminal,
            last_stats,
        }))
    }

    fn list(&self) -> Vec<String> {
        let conn = match self.conn.lock() {
            Ok(conn) => conn,
            Err(_) => return Vec::new(),
        };
        let mut statement = match conn.prepare("SELECT id FROM sessions ORDER BY updated_at DESC") {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    fn append_audit(&self, entries: &[AuditEntry]) -> Result<(), AgentError> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| AgentError::Session("SQLite 锁中毒".into()))?;
        let transaction = conn.transaction().map_err(sqlite_error)?;
        for entry in entries {
            transaction
                .execute(
                    "INSERT INTO audit (ts, session_id, event, tool, approved, detail)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        entry.ts,
                        entry.session_id,
                        entry.event,
                        entry.tool,
                        entry.approved.map(|approved| approved as i64),
                        entry.detail,
                    ],
                )
                .map_err(sqlite_error)?;
        }
        transaction.commit().map_err(sqlite_error)?;
        Ok(())
    }

    fn recent_audit(&self, limit: usize) -> Vec<AuditEntry> {
        let conn = match self.conn.lock() {
            Ok(conn) => conn,
            Err(_) => return Vec::new(),
        };
        let mut statement = match conn.prepare(
            "SELECT ts, session_id, event, tool, approved, detail
             FROM audit ORDER BY id DESC LIMIT ?1",
        ) {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let rows = match statement.query_map([limit as i64], |row| {
            Ok(AuditEntry {
                ts: row.get(0)?,
                session_id: row.get(1)?,
                event: row.get(2)?,
                tool: row.get(3)?,
                approved: row.get::<_, Option<i64>>(4)?.map(|value| value != 0),
                detail: row.get(5)?,
            })
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(Result::ok).collect()
    }

    /// 分页查询审计：返回 (当前页条目, 满足条件总数)。
    fn query_audit(&self, query: &AuditQuery) -> (Vec<AuditEntry>, usize) {
        let conn = match self.conn.lock() {
            Ok(conn) => conn,
            Err(error) => {
                eprintln!("AUDIT LOCK ERROR: {error}");
                return (Vec::new(), 0);
            }
        };
        let mut conditions: Vec<String> = Vec::new();
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(event) = &query.event {
            conditions.push("event = ?".to_string());
            args.push(event.clone().into());
        }
        if let Some(tool) = &query.tool {
            conditions.push("tool = ?".to_string());
            args.push(tool.clone().into());
        }
        if let Some(approved) = query.approved {
            conditions.push("approved = ?".to_string());
            args.push(i64::from(approved).into());
        }
        if let Some(q) = &query.q {
            let pattern = format!("%{}%", like_escape(q));
            conditions.push(
                "(detail LIKE ? ESCAPE '\\' OR event LIKE ? ESCAPE '\\' \
                 OR tool LIKE ? ESCAPE '\\' OR session_id LIKE ? ESCAPE '\\')"
                    .to_string(),
            );
            for _ in 0..4 {
                args.push(pattern.clone().into());
            }
        }
        let where_sql = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        let total_sql = format!("SELECT COUNT(*) FROM audit{where_sql}");
        let total = conn
            .query_row(&total_sql, rusqlite::params_from_iter(args.iter()), |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or_default();
        let limit = if query.limit == 0 {
            100
        } else {
            query.limit.min(1000)
        };
        args.push((limit as i64).into());
        args.push((query.offset as i64).into());
        let sql = format!(
            "SELECT ts, session_id, event, tool, approved, detail
             FROM audit{where_sql} ORDER BY id DESC LIMIT ? OFFSET ?"
        );
        let mut statement = match conn.prepare(&sql) {
            Ok(statement) => statement,
            Err(_) => return (Vec::new(), total as usize),
        };
        let rows = match statement.query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(AuditEntry {
                ts: row.get(0)?,
                session_id: row.get(1)?,
                event: row.get(2)?,
                tool: row.get(3)?,
                approved: row.get::<_, Option<i64>>(4)?.map(|value| value != 0),
                detail: row.get(5)?,
            })
        }) {
            Ok(rows) => rows,
            Err(_) => return (Vec::new(), total as usize),
        };
        (rows.filter_map(Result::ok).collect(), total as usize)
    }
}

fn sqlite_error(error: rusqlite::Error) -> AgentError {
    AgentError::Session(format!("SQLite 错误：{error}"))
}

fn json_error(error: serde_json::Error) -> AgentError {
    AgentError::Json(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::ChatMessage;
    use rusqlite::Connection;

    #[test]
    fn sqlite_session_round_trip_and_list() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-test-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        let mut session = store.create(Path::new("."), "mock", None).unwrap();
        session.push(ChatMessage::user("你好".to_string()));
        session.push(ChatMessage::assistant_text("收到".to_string()));
        session.rename("SQLite 会话".to_string());
        session.set_pinned(true);
        session.set_archived(true);
        session.single_verification_plan = Some(crate::plan::VerificationPlanV1 {
            plan_id: "single-plan-test".to_string(),
            requirements: vec![crate::plan::VerificationRequirementV1 {
                requirement_id: "feature-visible".to_string(),
                covers_requirement_ids: vec!["user-request:feature-visible".to_string()],
                validator_id: "workspace-command-success-v1".to_string(),
                validator_version: Some("1".to_string()),
                scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                    relative_paths: vec!["src/lib.rs".to_string()],
                },
                arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
                required: true,
                resources: crate::plan::VerificationResourcesV1 {
                    cpu_slots: 1,
                    memory_mb: 16,
                    exclusive_workspace: false,
                    timeout_ms: 20_000,
                },
            }],
        });
        session.single_verification_plan_input_sha256 = Some("input-hash".to_string());
        session.single_verification_plan_turn_id = Some("turn-1".to_string());
        session
            .single_review_issues
            .push(crate::goal::DeliveryIssueV1 {
                issue_id: "single-issue-1".to_string(),
                source_review_artifact_id: "single-review-1".to_string(),
                source_review_sha256: "review-sha".to_string(),
                finding_sha256: "finding-sha".to_string(),
                severity: "major".to_string(),
                detail: "Repair this behavior".to_string(),
                requirement_id: Some("feature-visible".to_string()),
                target_task_id: "task-1".to_string(),
                target_attempt_id: "turn-1".to_string(),
                target_artifact_id: None,
                owner_step_id: "single:session".to_string(),
                status: crate::goal::DeliveryIssueStatusV1::RepairDispatched,
                repair_attempt: 1,
                resolution_review_artifact_id: None,
                resolution_review_sha256: None,
                resolution_attempt_id: None,
                opened_at: "2026-10-04T00:00:00Z".to_string(),
                updated_at: "2026-10-04T00:00:01Z".to_string(),
            });
        session
            .validation_receipts
            .push(crate::plan::ValidationReceiptV1 {
                receipt_id: "single-validation-test".to_string(),
                task_id: session.id.clone(),
                attempt_id: "turn-1".to_string(),
                epoch: 1,
                requirement_id: "single-behavior-validation".to_string(),
                validator_id: "workspace-command-success-v1".to_string(),
                validator_version: "1".to_string(),
                arguments_sha256: "command-hash".to_string(),
                input_sha256: "input-hash".to_string(),
                environment_id: "environment-hash".to_string(),
                changeset_sha256: Some("changeset-hash".to_string()),
                detail: Some("host test passed".to_string()),
                subject_sha256: std::collections::HashMap::from([(
                    "workspace-path:src/lib.rs".to_string(),
                    "source-hash".to_string(),
                )]),
                verdict: crate::plan::ValidationVerdictV1::Passed,
                evidence_refs: vec!["command-result:sha256:result-hash".to_string()],
                review_result: None,
                started_at: "2026-10-04T00:00:00Z".to_string(),
                completed_at: "2026-10-04T00:00:01Z".to_string(),
            });
        session.completion_record = Some(crate::completion::build_completion_record(
            &session.id,
            "turn-1",
            owo_agent_protocol::CompletionStatusV1::Accepted,
            vec!["single-validation-test".to_string()],
            Some("single-candidate-sha".to_string()),
        ));
        store.save(&session).unwrap();
        let child = session.fork(1);
        store.save(&child).unwrap();

        let loaded = store.load(&session.id).unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].content.as_deref(), Some("你好"));
        assert_eq!(loaded.title.as_deref(), Some("SQLite 会话"));
        assert!(loaded.pinned);
        assert!(loaded.archived);
        assert_eq!(loaded.validation_receipts, session.validation_receipts);
        assert_eq!(loaded.single_review_issues, session.single_review_issues);
        assert_eq!(loaded.completion_record, session.completion_record);
        assert_eq!(
            loaded.single_verification_plan,
            session.single_verification_plan
        );
        assert_eq!(
            loaded.single_verification_plan_input_sha256.as_deref(),
            Some("input-hash")
        );
        assert_eq!(
            loaded.single_verification_plan_turn_id.as_deref(),
            Some("turn-1")
        );
        let loaded_child = store.load(&child.id).unwrap();
        assert_eq!(loaded_child.parent_id.as_deref(), Some(session.id.as_str()));
        assert_eq!(loaded_child.fork_point, Some(1));
        assert!(loaded_child.single_review_issues.is_empty());
        assert!(loaded_child.completion_record.is_none());
        assert_eq!(
            loaded_child.single_verification_plan,
            session.single_verification_plan
        );
        assert_eq!(
            loaded_child
                .single_verification_plan_input_sha256
                .as_deref(),
            Some("input-hash")
        );
        assert_eq!(
            loaded_child.single_verification_plan_turn_id.as_deref(),
            Some("turn-1")
        );
        assert!(loaded_child.title.is_none());
        assert!(!loaded_child.pinned);
        assert!(!loaded_child.archived);
        assert_eq!(store.list().len(), 2);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn migrates_legacy_database_without_message_redo_column() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-legacy-{}.db", uuid::Uuid::new_v4()));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 workspace TEXT NOT NULL,
                 model TEXT NOT NULL,
                 system_prompt TEXT,
                 messages_json TEXT NOT NULL,
                 snapshots_json TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 parent_id TEXT,
                 fork_point INTEGER,
                 redo_json TEXT NOT NULL
             );
             INSERT INTO sessions VALUES (
                 'legacy', '.', 'mock', NULL, '[]', '{}',
                 '2026-08-11T00:00:00Z', '2026-08-11T00:00:00Z', NULL, NULL, '[]'
             );",
        )
        .unwrap();
        drop(conn);

        let store = SqliteSessionStore::open(&path).unwrap();
        let loaded = store.load("legacy").unwrap();
        assert!(loaded.message_redo_stack.is_empty());
        assert_eq!(loaded.model, "mock");
        assert!(loaded.title.is_none());
        assert!(!loaded.pinned);
        assert!(!loaded.archived);
        assert_eq!(loaded.model_override, None, "v2 迁移后旧行覆盖列为 NULL");
        let mut session = store.create(Path::new("."), "mock", None).unwrap();
        session.rename("迁移后新会话".to_string());
        store.save(&session).unwrap();
        assert_eq!(
            store.load(&session.id).unwrap().title.as_deref(),
            Some("迁移后新会话")
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// 目标模式状态（migration v10）往返：持久化后重开库仍在，旧库旧行按 None 加载。
    #[test]
    fn goal_roundtrip_survives_reopen_and_legacy_rows() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-goal-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        let mut session = store.create(Path::new("."), "mock", None).unwrap();
        assert!(session.goal.is_none(), "新会话默认无目标");

        session.set_goal("把目标持久化到会话");
        session.advance_goal();
        store.save(&session).unwrap();
        drop(store);

        let reopened = SqliteSessionStore::open(&path).unwrap();
        let loaded = reopened.load(&session.id).unwrap();
        let goal = loaded.goal.as_ref().expect("目标应随会话落库");
        assert_eq!(goal.objective, "把目标持久化到会话");
        assert_eq!(goal.iterations, 1);
        assert!(!goal.done);

        // 完成后再次往返：done 与轮次都保留。
        let mut loaded = loaded;
        loaded.complete_goal();
        reopened.save(&loaded).unwrap();
        let after = reopened.load(&session.id).unwrap();
        assert!(after.goal.as_ref().unwrap().done);

        // 清除后落 NULL：重开库不复活旧目标。
        let mut cleared = after;
        cleared.clear_goal();
        reopened.save(&cleared).unwrap();
        assert!(reopened.load(&session.id).unwrap().goal.is_none());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// M4.2：`model_override` 列往返（含重开库）；覆盖不改变展示模型。
    #[test]
    fn model_override_roundtrip_survives_reopen() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-model-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        let mut session = store.create(Path::new("."), "glm-5.3-flash", None).unwrap();
        assert_eq!(session.model_override, None);
        session.model_override = Some("vision-pro".to_string());
        store.save(&session).unwrap();
        drop(store);
        let reopened = SqliteSessionStore::open(&path).unwrap();
        let loaded = reopened.load(&session.id).unwrap();
        assert_eq!(loaded.model_override.as_deref(), Some("vision-pro"));
        assert_eq!(loaded.model, "glm-5.3-flash");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn appends_audit_entries_to_sqlite() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-audit-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        let entry = AuditEntry {
            ts: "2026-08-11T00:00:00Z".to_string(),
            session_id: "s1".to_string(),
            event: "tool_call".to_string(),
            tool: Some("read_file".to_string()),
            approved: None,
            detail: "ok".to_string(),
        };
        store.append_audit(&[entry]).unwrap();
        let recent = store.recent_audit(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].event, "tool_call");
        assert_eq!(recent[0].session_id, "s1");
        assert_eq!(recent[0].approved, None);
        let conn = Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn migrations_advance_user_version_and_are_idempotent() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-migrate-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        assert_eq!(store.migration_status().schema_version, 10);
        assert!(store.migration_status().pending.is_empty());
        assert!(!store.is_read_only());
        drop(store);
        // 再次打开：无新迁移应用，schema_version 保持。
        let reopened = SqliteSessionStore::open(&path).unwrap();
        assert_eq!(reopened.migration_status().schema_version, 10);
        assert!(reopened.migration_status().applied.is_empty());
        assert!(!reopened.is_read_only());
        let conn = Connection::open(&path).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 10);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn legacy_schema_gets_migrated_by_v1() {
        // 旧库（user_version=0、缺 4 列）由注册迁移 v1 补齐，而不是隐式 ALTER。
        let path = std::env::temp_dir().join(format!("owo-sqlite-v1-{}.db", uuid::Uuid::new_v4()));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY,
                 workspace TEXT NOT NULL,
                 model TEXT NOT NULL,
                 system_prompt TEXT,
                 messages_json TEXT NOT NULL,
                 snapshots_json TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 parent_id TEXT,
                 fork_point INTEGER,
                 redo_json TEXT NOT NULL
             );",
        )
        .unwrap();
        drop(conn);
        let store = SqliteSessionStore::open(&path).unwrap();
        assert!(!store.is_read_only());
        assert_eq!(
            store.migration_status().applied,
            vec![
                "v1: sessions 列补齐（message_redo_json/title/archived/pinned）",
                "v2: sessions 列补齐（model_override：M4.2 会话级模型路由）",
                "v3: session turn events 持久化与单调 seq",
                "v4: session execution receipts 持久化",
                "v5: session behavior validation receipts 持久化",
                "v6: session Single VerificationPlan 持久化",
                "v7: session VerificationPlan 回合身份绑定",
                "v8: session Single review issues 持久化",
                "v9: session Single completion record 持久化",
                "v10: session 目标模式状态持久化（/goal）",
            ]
        );
        let mut session = store.create(Path::new("."), "mock", None).unwrap();
        session.rename("v1 迁移会话".to_string());
        store.save(&session).unwrap();
        assert_eq!(
            store.load(&session.id).unwrap().title.as_deref(),
            Some("v1 迁移会话")
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn migration_failure_degrades_to_read_only_with_hint() {
        let path = std::env::temp_dir().join(format!("owo-sqlite-ro-{}.db", uuid::Uuid::new_v4()));
        let failing = Migration {
            version: 99,
            name: "boom",
            run: |_| Err(AgentError::Session("boom".into())),
        };
        let store = SqliteSessionStore::open_with_migrations(&path, &[failing]).unwrap();
        assert!(store.is_read_only(), "迁移失败必须降级只读，不得静默写入");
        let status = store.migration_status();
        assert!(status.last_error.is_some(), "降级必须携带原因提示");
        assert!(
            store.create(Path::new("."), "mock", None).is_err(),
            "只读库必须拒绝写入"
        );
        assert_eq!(store.list().len(), 0, "读取仍可用");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn turn_events_have_session_monotonic_seq_and_survive_reopen() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-events-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        let session_a = store.create(Path::new("."), "mock", None).unwrap();
        let session_b = store.create(Path::new("."), "mock", None).unwrap();

        let first = store
            .append_turn_event(
                &session_a.id,
                "turn-a",
                &SseEvent::TokenDelta {
                    delta: "hel".to_string(),
                },
            )
            .unwrap();
        let second = store
            .append_turn_event(
                &session_a.id,
                "turn-a",
                &SseEvent::TokenDelta {
                    delta: "lo".to_string(),
                },
            )
            .unwrap();
        let independent = store
            .append_turn_event(
                &session_b.id,
                "turn-b",
                &SseEvent::Final {
                    text: "other session".to_string(),
                },
            )
            .unwrap();
        assert_eq!((first.seq, second.seq, independent.seq), (1, 2, 1));
        drop(store);

        let reopened = SqliteSessionStore::open(&path).unwrap();
        let replay = reopened
            .turn_events_after(&session_a.id, Some("turn-a"), 1, 10)
            .unwrap();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].seq, 2);
        assert_eq!(replay[0].turn_id, "turn-a");
        assert!(matches!(
            replay[0].payload,
            SseEvent::TokenDelta { ref delta } if delta == "lo"
        ));
        let next_turn = reopened
            .append_turn_event(
                &session_a.id,
                "turn-next",
                &SseEvent::Progress {
                    message: "next turn".to_string(),
                },
            )
            .unwrap();
        assert_eq!(
            next_turn.seq, 3,
            "sequence continues across turns in a session"
        );
        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[test]
    fn clear_wipes_sessions_and_audit_and_integrity_ok() {
        let path =
            std::env::temp_dir().join(format!("owo-sqlite-clear-{}.db", uuid::Uuid::new_v4()));
        let store = SqliteSessionStore::open(&path).unwrap();
        let session = store.create(Path::new("."), "mock", None).unwrap();
        let entry = AuditEntry {
            ts: "2026-08-11T00:00:00Z".to_string(),
            session_id: "s1".to_string(),
            event: "test".to_string(),
            tool: None,
            approved: None,
            detail: "d".to_string(),
        };
        store.append_audit(&[entry]).unwrap();
        store
            .append_turn_event(
                &session.id,
                "turn-clear",
                &SseEvent::Final {
                    text: "clear me".to_string(),
                },
            )
            .unwrap();
        assert_eq!(store.counts(), (1, 1));
        assert_eq!(store.integrity_check().unwrap(), "ok");
        assert_eq!(store.clear_all().unwrap(), (1, 1));
        assert_eq!(store.counts(), (0, 0));
        let conn = store.conn.lock().unwrap();
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM turn_events", [], |row| row.get(0))
            .unwrap();
        let cursor_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM turn_event_cursors", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!((event_count, cursor_count), (0, 0));
        drop(conn);
        assert_eq!(
            store.integrity_check().unwrap(),
            "ok",
            "清空后完整性校验必须通过"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}

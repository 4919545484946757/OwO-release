//! R8 迁移框架：注册迁移表与顺序迁移执行（从 sqlite_store.rs 拆出）。
//!
//! `MigrationStatus` 与存储结构留在 sqlite_store/mod.rs（属存储状态，不属迁移框架）。

use super::*;
use rusqlite::Connection;

/// 一条注册迁移：version 必须严格递增，run 在单个事务内执行并同步 user_version。
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub run: fn(&Connection) -> Result<(), AgentError>,
}

/// 顺序迁移表：任何 schema 变更都必须以新条目显式注册（禁止隐式 ALTER）。
/// v1：sessions 列补齐（此前为运行时逐列探测的隐式 ALTER，R8 收敛为注册迁移）。
/// v2：M4.2 会话级模型路由（`model_override` 列）。
/// v3：持久化 turn event 及每会话单调序号。
/// v4：普通 ToolHost 文件执行收据（基线/结果哈希与撤销状态）。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "sessions 列补齐（message_redo_json/title/archived/pinned）",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            for (column, definition) in [
                ("message_redo_json", "TEXT NOT NULL DEFAULT '[]'"),
                ("title", "TEXT"),
                ("archived", "INTEGER NOT NULL DEFAULT 0"),
                ("pinned", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                if !columns.iter().any(|existing| existing == column) {
                    conn.execute_batch(&format!(
                        "ALTER TABLE sessions ADD COLUMN {column} {definition}"
                    ))
                    .map_err(sqlite_error)?;
                }
            }
            Ok(())
        },
    },
    Migration {
        version: 2,
        name: "sessions 列补齐（model_override：M4.2 会话级模型路由）",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns.iter().any(|existing| existing == "model_override") {
                conn.execute_batch("ALTER TABLE sessions ADD COLUMN model_override TEXT")
                    .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
    Migration {
        version: 3,
        name: "session turn events 持久化与单调 seq",
        run: |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS turn_event_cursors (
                     session_id TEXT PRIMARY KEY,
                     last_seq INTEGER NOT NULL CHECK(last_seq >= 0)
                 );
                 CREATE TABLE IF NOT EXISTS turn_events (
                     session_id TEXT NOT NULL,
                     seq INTEGER NOT NULL CHECK(seq > 0),
                     turn_id TEXT NOT NULL,
                     created_at TEXT NOT NULL,
                     payload_json TEXT NOT NULL,
                     PRIMARY KEY(session_id, seq)
                 );
                 CREATE INDEX IF NOT EXISTS idx_turn_events_turn
                     ON turn_events(session_id, turn_id, seq);",
            )
            .map_err(sqlite_error)?;
            Ok(())
        },
    },
    Migration {
        version: 4,
        name: "session execution receipts 持久化",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns
                .iter()
                .any(|existing| existing == "execution_receipts_json")
            {
                conn.execute_batch(
                    "ALTER TABLE sessions ADD COLUMN execution_receipts_json TEXT NOT NULL DEFAULT '[]'",
                )
                .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
    Migration {
        version: 5,
        name: "session behavior validation receipts 持久化",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns
                .iter()
                .any(|existing| existing == "validation_receipts_json")
            {
                conn.execute_batch(
                    "ALTER TABLE sessions ADD COLUMN validation_receipts_json TEXT NOT NULL DEFAULT '[]'",
                )
                .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
    Migration {
        version: 6,
        name: "session Single VerificationPlan 持久化",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            for (column, definition) in [
                ("single_verification_plan_json", "TEXT"),
                ("single_verification_plan_input_sha256", "TEXT"),
            ] {
                if !columns.iter().any(|existing| existing == column) {
                    conn.execute_batch(&format!(
                        "ALTER TABLE sessions ADD COLUMN {column} {definition}"
                    ))
                    .map_err(sqlite_error)?;
                }
            }
            Ok(())
        },
    },
    Migration {
        version: 7,
        name: "session VerificationPlan 回合身份绑定",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns
                .iter()
                .any(|existing| existing == "single_verification_plan_turn_id")
            {
                conn.execute_batch(
                    "ALTER TABLE sessions ADD COLUMN single_verification_plan_turn_id TEXT",
                )
                .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
    Migration {
        version: 8,
        name: "session Single review issues 持久化",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns
                .iter()
                .any(|existing| existing == "single_review_issues_json")
            {
                conn.execute_batch(
                    "ALTER TABLE sessions ADD COLUMN single_review_issues_json TEXT NOT NULL DEFAULT '[]'",
                )
                .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
    Migration {
        version: 9,
        name: "session Single completion record 持久化",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns
                .iter()
                .any(|existing| existing == "completion_record_json")
            {
                conn.execute_batch("ALTER TABLE sessions ADD COLUMN completion_record_json TEXT")
                    .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
    Migration {
        version: 10,
        name: "session 目标模式状态持久化（/goal）",
        run: |conn| {
            let columns = table_columns(conn, "sessions")?;
            if !columns.iter().any(|existing| existing == "goal_json") {
                conn.execute_batch("ALTER TABLE sessions ADD COLUMN goal_json TEXT")
                    .map_err(sqlite_error)?;
            }
            Ok(())
        },
    },
];

/// 基表结构（CREATE TABLE IF NOT EXISTS，幂等）。
pub(super) fn base_schema() -> &'static str {
    "PRAGMA journal_mode=WAL;
     CREATE TABLE IF NOT EXISTS sessions (
         id TEXT PRIMARY KEY,
         workspace TEXT NOT NULL,
         model TEXT NOT NULL,
         system_prompt TEXT,
         messages_json TEXT NOT NULL,
         snapshots_json TEXT NOT NULL,
         execution_receipts_json TEXT NOT NULL DEFAULT '[]',
         created_at TEXT NOT NULL,
         updated_at TEXT NOT NULL,
         parent_id TEXT,
         fork_point INTEGER,
         redo_json TEXT NOT NULL,
         message_redo_json TEXT NOT NULL DEFAULT '[]',
         title TEXT,
         archived INTEGER NOT NULL DEFAULT 0,
         pinned INTEGER NOT NULL DEFAULT 0,
         model_override TEXT,
         validation_receipts_json TEXT NOT NULL DEFAULT '[]',
         single_verification_plan_json TEXT,
         single_verification_plan_input_sha256 TEXT,
         single_verification_plan_turn_id TEXT,
         single_review_issues_json TEXT NOT NULL DEFAULT '[]',
         completion_record_json TEXT,
         goal_json TEXT
     );
     CREATE TABLE IF NOT EXISTS audit (
         id INTEGER PRIMARY KEY AUTOINCREMENT,
         ts TEXT NOT NULL,
         session_id TEXT NOT NULL,
         event TEXT NOT NULL,
         tool TEXT,
         approved INTEGER,
         detail TEXT NOT NULL
     );"
}

pub(super) fn user_version(conn: &Connection) -> Result<i64, AgentError> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(sqlite_error)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, AgentError> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sqlite_error)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sqlite_error)?
        .filter_map(Result::ok)
        .collect();
    Ok(columns)
}

/// 顺序执行 version > user_version 的迁移；每条在独立事务内提交并推进 user_version。
pub(super) fn run_migrations(
    conn: &mut Connection,
    migrations: &[Migration],
) -> Result<Vec<String>, AgentError> {
    let mut applied = Vec::new();
    let mut current = user_version(conn)?;
    for migration in migrations {
        if migration.version <= current {
            continue;
        }
        let transaction = conn.transaction().map_err(sqlite_error)?;
        (migration.run)(&transaction)?;
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(sqlite_error)?;
        transaction.commit().map_err(sqlite_error)?;
        applied.push(format!("v{}: {}", migration.version, migration.name));
        current = migration.version;
    }
    Ok(applied)
}

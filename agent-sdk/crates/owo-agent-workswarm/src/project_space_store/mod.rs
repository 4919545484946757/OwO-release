//! ProjectSpaceStore：WorkSwarm 项目空间持久化（§6.6）。
//!
//! 提供 [`ProjectSpaceStore`] trait 与 [`SqliteProjectSpaceStore`] 实现，
//! 管理 [`ProjectSpace`]、[`Artifact`]、[`DecisionRecord`]、[`HandoffRecord`]
//! 和 [`TeamRun`] 的 CRUD 操作。
//!
//! 设计原则：
//! - 所有实体以 JSON 序列化存储，保留完整结构；
//! - 索引列（project_id, team_id, artifact_id 等）用于快速查询；
//! - 不自动级联删除：移除 ProjectSpace 时只删自身记录，关联实体由调用方显式清理；
//! - 线程安全：内部使用 `Mutex<Connection>`，与既有 `SqliteSessionStore` 一致。

use async_trait::async_trait;
use owo_agent_protocol::{
    Artifact, ArtifactReviewDecision, ArtifactReviewRecord, ArtifactReworkTask, DecisionRecord,
    HandoffRecord, ProjectSpace, SharedContextFact, SharedContextSnapshot, TeamRun,
};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

/// 存储层错误。
#[derive(Debug, thiserror::Error)]
pub enum ProjectSpaceStoreError {
    #[error("SQLite 错误：{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("序列化错误：{0}")]
    Serialization(String),
    #[error("未找到：{0}")]
    NotFound(String),
    #[error("交付状态不一致：{0}")]
    InvalidState(String),
}

impl From<serde_json::Error> for ProjectSpaceStoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}

type Result<T> = std::result::Result<T, ProjectSpaceStoreError>;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// 项目空间存储抽象。
///
/// 当前同步实现（rusqlite 本身是同步的），使用 async_trait 为未来
/// 异步后端（如远程存储）预留接口。
#[async_trait]
pub trait ProjectSpaceStoreBackend: Send + Sync {
    // -- ProjectSpace --
    async fn save_project_space(&self, space: &ProjectSpace) -> Result<()>;
    async fn get_project_space(&self, project_id: &str) -> Result<ProjectSpace>;
    async fn list_project_spaces(&self) -> Result<Vec<ProjectSpace>>;
    async fn delete_project_space(&self, project_id: &str) -> Result<()>;
    async fn get_team_context(&self, team_id: &str) -> Result<SharedContextSnapshot>;
    async fn compare_and_swap_team_context(
        &self,
        team_id: &str,
        expected_revision: u64,
        fact: &SharedContextFact,
    ) -> Result<bool>;

    // -- Artifact --
    /// 保存产物；`project_id` 落真实关联列（`list_artifacts_by_project` 按索引过滤）。
    async fn save_artifact(&self, artifact: &Artifact, project_id: &str) -> Result<()>;
    async fn get_artifact(&self, artifact_id: &str) -> Result<Artifact>;
    async fn list_artifacts_by_project(&self, project_id: &str) -> Result<Vec<Artifact>>;
    async fn delete_artifact(&self, artifact_id: &str) -> Result<()>;

    // -- DecisionRecord --
    async fn save_decision(&self, decision: &DecisionRecord, project_id: &str) -> Result<()>;
    async fn get_decision(&self, decision_id: &str) -> Result<DecisionRecord>;
    async fn list_decisions_by_project(&self, project_id: &str) -> Result<Vec<DecisionRecord>>;

    // -- HandoffRecord --
    async fn save_handoff(&self, handoff: &HandoffRecord, project_id: &str) -> Result<()>;
    async fn get_handoff(&self, handoff_id: &str) -> Result<HandoffRecord>;
    async fn list_handoffs_by_project(&self, project_id: &str) -> Result<Vec<HandoffRecord>>;

    // -- TeamRun --
    async fn save_team_run(&self, team_run: &TeamRun) -> Result<()>;
    /// Atomically publish a succeeded TeamRun and its completed ProjectSpace delivery pointer.
    async fn commit_team_delivery(&self, team_run: &TeamRun, space: &ProjectSpace) -> Result<()>;
    async fn get_team_run(&self, team_id: &str) -> Result<TeamRun>;
    async fn list_team_runs(&self) -> Result<Vec<TeamRun>>;
    async fn delete_team_run(&self, team_id: &str) -> Result<()>;

    // -- ArtifactReview（评审闭环，V1-R2；记录只增不改） --
    /// 追加一条评审记录（`idempotency_key` 唯一约束，重复插入报错由调用方幂等处理）。
    async fn save_artifact_review(&self, review: &ArtifactReviewRecord) -> Result<()>;
    /// 按产物取全部评审记录（created_at 升序）。
    async fn list_artifact_reviews(&self, artifact_id: &str) -> Result<Vec<ArtifactReviewRecord>>;
    /// 按幂等键查既有记录（幂等回放依据）。
    async fn get_artifact_review_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<ArtifactReviewRecord>>;
    /// 产物所属 project_id（索引列读取；评审头表定位用）。
    async fn get_artifact_project(&self, artifact_id: &str) -> Result<String>;
    /// 设置 (project, kind) 的 approved head（真实存在且已批准的版本，调用方保证）。
    async fn set_approved_head(
        &self,
        project_id: &str,
        kind: &str,
        artifact_id: &str,
        approved_at: &str,
    ) -> Result<()>;
    /// 读取 approved head（不存在返回 None；head 指向的产物缺失/未批准时返回 None——
    /// 自愈语义：脏 head 不阻塞读取，等待下次 approve 覆盖）。
    async fn get_approved_head(&self, project_id: &str, kind: &str) -> Result<Option<Artifact>>;
}

// ---------------------------------------------------------------------------
// SQLite 实现
// ---------------------------------------------------------------------------

/// WorkSwarm 表结构（幂等 CREATE TABLE IF NOT EXISTS）。
fn worksarm_schema() -> &'static str {
    "CREATE TABLE IF NOT EXISTS project_spaces (
         project_id TEXT PRIMARY KEY,
         data_json TEXT NOT NULL,
         created_at TEXT NOT NULL,
         updated_at TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS team_contexts (
         team_id TEXT PRIMARY KEY,
         revision INTEGER NOT NULL,
         facts_json TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS artifacts (
         artifact_id TEXT PRIMARY KEY,
         project_id TEXT NOT NULL,
         data_json TEXT NOT NULL,
         created_at TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_artifacts_project ON artifacts(project_id);
     CREATE TABLE IF NOT EXISTS decisions (
         decision_id TEXT PRIMARY KEY,
         project_id TEXT NOT NULL,
         data_json TEXT NOT NULL,
         created_at TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_decisions_project ON decisions(project_id);
     CREATE TABLE IF NOT EXISTS handoffs (
         handoff_id TEXT PRIMARY KEY,
         project_id TEXT NOT NULL,
         data_json TEXT NOT NULL,
         created_at TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_handoffs_project ON handoffs(project_id);
     CREATE TABLE IF NOT EXISTS team_runs (
         team_id TEXT PRIMARY KEY,
         data_json TEXT NOT NULL,
         created_at TEXT NOT NULL,
         updated_at TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS artifact_reviews (
         review_id TEXT PRIMARY KEY,
         artifact_id TEXT NOT NULL,
         artifact_version INTEGER NOT NULL,
         team_id TEXT NOT NULL,
         decision TEXT NOT NULL,
         reviewer TEXT NOT NULL,
         comment TEXT NOT NULL DEFAULT '',
         idempotency_key TEXT NOT NULL UNIQUE,
         content_ref TEXT NOT NULL DEFAULT '',
         created_at TEXT NOT NULL
     );
     CREATE INDEX IF NOT EXISTS idx_artifact_reviews_artifact ON artifact_reviews(artifact_id);
     CREATE TABLE IF NOT EXISTS artifact_approved_heads (
         project_id TEXT NOT NULL,
         kind TEXT NOT NULL,
         artifact_id TEXT NOT NULL,
         approved_at TEXT NOT NULL,
         PRIMARY KEY (project_id, kind)
     );"
}

/// 评审记录步骤关联列迁移（V1 五期 · 第二路）：已存在的旧库按需补列。
///
/// `CREATE TABLE IF NOT EXISTS` 不会为旧表加列，这里按 pragma 检查后 ALTER。
fn migrate_artifact_reviews_columns(conn: &Connection) -> Result<()> {
    let has_column = |name: &str| -> Result<bool> {
        let mut stmt = conn.prepare("PRAGMA table_info(artifact_reviews)")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            if row.get::<_, String>(1)? == name {
                return Ok(true);
            }
        }
        Ok(false)
    };
    if !has_column("step_id")? {
        conn.execute_batch(
            "ALTER TABLE artifact_reviews ADD COLUMN step_id TEXT NOT NULL DEFAULT ''",
        )?;
    }
    if !has_column("producer_member_id")? {
        conn.execute_batch(
            "ALTER TABLE artifact_reviews ADD COLUMN producer_member_id TEXT NOT NULL DEFAULT ''",
        )?;
    }
    Ok(())
}

/// SQLite 后端的项目空间存储。
pub struct SqliteProjectSpaceStore {
    conn: Mutex<Connection>,
}

impl SqliteProjectSpaceStore {
    /// 打开或创建存储（自动建表）。
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        // 多连接共存（协调器 + 评审 API 各持一条连接）：写锁竞争在 busy_timeout 内自旋等待。
        conn.busy_timeout(std::time::Duration::from_millis(2000))?;
        conn.execute_batch(worksarm_schema())?;
        migrate_artifact_reviews_columns(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// 使用已有连接（测试注入）。
    pub fn from_connection(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_millis(2000))?;
        conn.execute_batch(worksarm_schema())?;
        migrate_artifact_reviews_columns(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn serialize<T: serde::Serialize>(value: &T) -> Result<String> {
        serde_json::to_string(value).map_err(Into::into)
    }

    fn deserialize<T: serde::de::DeserializeOwned>(json: &str) -> Result<T> {
        serde_json::from_str(json).map_err(Into::into)
    }
}

#[async_trait]
impl ProjectSpaceStoreBackend for SqliteProjectSpaceStore {
    // -- ProjectSpace --

    async fn save_project_space(&self, space: &ProjectSpace) -> Result<()> {
        let json = Self::serialize(space)?;
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO project_spaces (project_id, data_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id) DO UPDATE SET data_json = excluded.data_json, updated_at = excluded.updated_at",
            params![space.project_id, json, space.created_at, space.updated_at],
        )?;
        Ok(())
    }

    async fn get_team_context(&self, team_id: &str) -> Result<SharedContextSnapshot> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let row = conn.query_row(
            "SELECT revision, facts_json FROM team_contexts WHERE team_id = ?1",
            params![team_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        );
        match row {
            Ok((revision, json)) => {
                let facts: Vec<SharedContextFact> = Self::deserialize(&json)?;
                Ok(SharedContextSnapshot { revision, facts })
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(SharedContextSnapshot::default()),
            Err(error) => Err(error.into()),
        }
    }

    async fn compare_and_swap_team_context(
        &self,
        team_id: &str,
        expected_revision: u64,
        fact: &SharedContextFact,
    ) -> Result<bool> {
        let next_revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= i64::MAX as u64);
        if next_revision != Some(fact.revision) {
            return Err(ProjectSpaceStoreError::Serialization(
                "共享上下文 revision 必须递增 1 且不超过 SQLite 整数上限".to_string(),
            ));
        }
        let mut conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row = tx.query_row(
            "SELECT revision, facts_json FROM team_contexts WHERE team_id = ?1",
            params![team_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        );
        let (current_revision, mut facts) = match row {
            Ok((revision, json)) => (
                revision,
                Self::deserialize::<Vec<SharedContextFact>>(&json)?,
            ),
            Err(rusqlite::Error::QueryReturnedNoRows) => (0, Vec::new()),
            Err(error) => return Err(error.into()),
        };
        if current_revision != expected_revision {
            return Ok(false);
        }
        if facts.len() >= 128 {
            return Err(ProjectSpaceStoreError::Serialization(
                "团队共享事实最多保存 128 个版本".to_string(),
            ));
        }
        facts.push(fact.clone());
        let encoded = Self::serialize(&facts)?;
        tx.execute(
            "INSERT INTO team_contexts (team_id, revision, facts_json) VALUES (?1, ?2, ?3)
             ON CONFLICT(team_id) DO UPDATE SET revision = excluded.revision, facts_json = excluded.facts_json",
            params![team_id, fact.revision, encoded],
        )?;
        tx.commit()?;
        Ok(true)
    }

    async fn get_project_space(&self, project_id: &str) -> Result<ProjectSpace> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let json: String = conn
            .query_row(
                "SELECT data_json FROM project_spaces WHERE project_id = ?1",
                params![project_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ProjectSpaceStoreError::NotFound(format!("ProjectSpace {project_id}"))
                }
                other => ProjectSpaceStoreError::Sqlite(other),
            })?;
        Self::deserialize(&json)
    }

    async fn list_project_spaces(&self) -> Result<Vec<ProjectSpace>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare("SELECT data_json FROM project_spaces ORDER BY created_at")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            let json = row?;
            result.push(Self::deserialize(&json)?);
        }
        Ok(result)
    }

    async fn delete_project_space(&self, project_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "DELETE FROM project_spaces WHERE project_id = ?1",
            params![project_id],
        )?;
        Ok(())
    }

    // -- Artifact --

    async fn save_artifact(&self, artifact: &Artifact, project_id: &str) -> Result<()> {
        // 版本链回填（V1 五期）：登记更高版本且未显式声明取代关系时，
        // 自动指向同 (project, kind) 的当前最高版本（追溯链；不改任何 review_state——
        // 旧版进入 Superseded 由评审 approve 路径统一处理，保证 approved head 恒有效）。
        let mut artifact = artifact.clone();
        if artifact.supersedes_artifact_id.is_none() {
            let latest: Option<Artifact> = {
                let conn = self
                    .conn
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut stmt =
                    conn.prepare("SELECT data_json FROM artifacts WHERE project_id = ?1")?;
                let mut rows = stmt.query(params![project_id])?;
                let mut latest: Option<Artifact> = None;
                while let Some(row) = rows.next()? {
                    let json: String = row.get(0)?;
                    let candidate: Artifact = Self::deserialize(&json)?;
                    if candidate.kind != artifact.kind
                        || candidate.artifact_id == artifact.artifact_id
                    {
                        continue;
                    }
                    if latest
                        .as_ref()
                        .map(|l| candidate.version > l.version)
                        .unwrap_or(true)
                    {
                        latest = Some(candidate);
                    }
                }
                latest
            };
            if let Some(latest) = latest {
                if artifact.version > latest.version {
                    artifact.supersedes_artifact_id = Some(latest.artifact_id);
                }
            }
        }
        let json = Self::serialize(&artifact)?;
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO artifacts (artifact_id, project_id, data_json, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(artifact_id) DO UPDATE SET data_json = excluded.data_json",
            params![artifact.artifact_id, project_id, json, artifact.created_at],
        )?;
        Ok(())
    }

    async fn get_artifact(&self, artifact_id: &str) -> Result<Artifact> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let json: String = conn
            .query_row(
                "SELECT data_json FROM artifacts WHERE artifact_id = ?1",
                params![artifact_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ProjectSpaceStoreError::NotFound(format!("Artifact {artifact_id}"))
                }
                other => ProjectSpaceStoreError::Sqlite(other),
            })?;
        Self::deserialize(&json)
    }

    async fn list_artifacts_by_project(&self, project_id: &str) -> Result<Vec<Artifact>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare(
            "SELECT data_json FROM artifacts
             WHERE project_id = ?1
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![project_id], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            let json = row?;
            result.push(Self::deserialize(&json)?);
        }
        Ok(result)
    }

    async fn delete_artifact(&self, artifact_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "DELETE FROM artifacts WHERE artifact_id = ?1",
            params![artifact_id],
        )?;
        Ok(())
    }

    // -- DecisionRecord --

    async fn save_decision(&self, decision: &DecisionRecord, project_id: &str) -> Result<()> {
        let json = Self::serialize(decision)?;
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO decisions (decision_id, project_id, data_json, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(decision_id) DO UPDATE SET data_json = excluded.data_json",
            params![decision.decision_id, project_id, json, decision.created_at],
        )?;
        Ok(())
    }

    async fn get_decision(&self, decision_id: &str) -> Result<DecisionRecord> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let json: String = conn
            .query_row(
                "SELECT data_json FROM decisions WHERE decision_id = ?1",
                params![decision_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ProjectSpaceStoreError::NotFound(format!("DecisionRecord {decision_id}"))
                }
                other => ProjectSpaceStoreError::Sqlite(other),
            })?;
        Self::deserialize(&json)
    }

    async fn list_decisions_by_project(&self, project_id: &str) -> Result<Vec<DecisionRecord>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare(
            "SELECT data_json FROM decisions
             WHERE project_id = ?1
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![project_id], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            let json = row?;
            result.push(Self::deserialize(&json)?);
        }
        Ok(result)
    }

    // -- HandoffRecord --

    async fn save_handoff(&self, handoff: &HandoffRecord, project_id: &str) -> Result<()> {
        let json = Self::serialize(handoff)?;
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO handoffs (handoff_id, project_id, data_json, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(handoff_id) DO UPDATE SET data_json = excluded.data_json",
            params![handoff.handoff_id, project_id, json, handoff.created_at],
        )?;
        Ok(())
    }

    async fn get_handoff(&self, handoff_id: &str) -> Result<HandoffRecord> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let json: String = conn
            .query_row(
                "SELECT data_json FROM handoffs WHERE handoff_id = ?1",
                params![handoff_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ProjectSpaceStoreError::NotFound(format!("HandoffRecord {handoff_id}"))
                }
                other => ProjectSpaceStoreError::Sqlite(other),
            })?;
        Self::deserialize(&json)
    }

    async fn list_handoffs_by_project(&self, project_id: &str) -> Result<Vec<HandoffRecord>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare(
            "SELECT data_json FROM handoffs
             WHERE project_id = ?1
             ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![project_id], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            let json = row?;
            result.push(Self::deserialize(&json)?);
        }
        Ok(result)
    }

    // -- TeamRun --

    async fn save_team_run(&self, team_run: &TeamRun) -> Result<()> {
        let json = Self::serialize(team_run)?;
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO team_runs (team_id, data_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(team_id) DO UPDATE SET data_json = excluded.data_json, updated_at = excluded.updated_at",
            params![team_run.team_id, json, team_run.created_at, team_run.updated_at],
        )?;
        Ok(())
    }

    async fn commit_team_delivery(&self, team_run: &TeamRun, space: &ProjectSpace) -> Result<()> {
        if team_run.status != owo_agent_protocol::TeamRunStatus::Succeeded
            || space.status != owo_agent_protocol::ProjectSpaceStatus::Completed
            || team_run.project_space_id.as_deref() != Some(space.project_id.as_str())
            || space.team_id.as_deref() != Some(team_run.team_id.as_str())
        {
            return Err(ProjectSpaceStoreError::InvalidState(format!(
                "TeamRun {} 与 ProjectSpace {} 未形成匹配的成功交付",
                team_run.team_id, space.project_id
            )));
        }
        let team_json = Self::serialize(team_run)?;
        let space_json = Self::serialize(space)?;
        let mut conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO team_runs (team_id, data_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(team_id) DO UPDATE SET data_json = excluded.data_json, updated_at = excluded.updated_at",
            params![team_run.team_id, team_json, team_run.created_at, team_run.updated_at],
        )?;
        tx.execute(
            "INSERT INTO project_spaces (project_id, data_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id) DO UPDATE SET data_json = excluded.data_json, updated_at = excluded.updated_at",
            params![space.project_id, space_json, space.created_at, space.updated_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    async fn get_team_run(&self, team_id: &str) -> Result<TeamRun> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let json: String = conn
            .query_row(
                "SELECT data_json FROM team_runs WHERE team_id = ?1",
                params![team_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ProjectSpaceStoreError::NotFound(format!("TeamRun {team_id}"))
                }
                other => ProjectSpaceStoreError::Sqlite(other),
            })?;
        Self::deserialize(&json)
    }

    async fn list_team_runs(&self) -> Result<Vec<TeamRun>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare("SELECT data_json FROM team_runs ORDER BY created_at")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut result = Vec::new();
        for row in rows {
            let json = row?;
            result.push(Self::deserialize(&json)?);
        }
        Ok(result)
    }

    async fn delete_team_run(&self, team_id: &str) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute("DELETE FROM team_runs WHERE team_id = ?1", params![team_id])?;
        Ok(())
    }

    // -- ArtifactReview --

    async fn save_artifact_review(&self, review: &ArtifactReviewRecord) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO artifact_reviews (
                 review_id, artifact_id, artifact_version, team_id, decision,
                 reviewer, comment, idempotency_key, content_ref, created_at,
                 step_id, producer_member_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                review.review_id,
                review.artifact_id,
                review.artifact_version,
                review.team_id,
                serde_json::to_string(&review.decision)
                    .map_err(|e| ProjectSpaceStoreError::Serialization(e.to_string()))?,
                review.reviewer,
                review.comment,
                review.idempotency_key,
                review.content_ref,
                review.created_at,
                review.step_id,
                review.producer_member_id,
            ],
        )?;
        Ok(())
    }

    async fn list_artifact_reviews(&self, artifact_id: &str) -> Result<Vec<ArtifactReviewRecord>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare(
            "SELECT review_id, artifact_id, artifact_version, team_id, decision,
                    reviewer, comment, idempotency_key, content_ref, created_at,
                    step_id, producer_member_id
             FROM artifact_reviews WHERE artifact_id = ?1 ORDER BY created_at, review_id",
        )?;
        let rows = stmt.query_map(params![artifact_id], review_from_row)?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    async fn get_artifact_review_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<ArtifactReviewRecord>> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut stmt = conn.prepare(
            "SELECT review_id, artifact_id, artifact_version, team_id, decision,
                    reviewer, comment, idempotency_key, content_ref, created_at,
                    step_id, producer_member_id
             FROM artifact_reviews WHERE idempotency_key = ?1",
        )?;
        let mut rows = stmt.query_map(params![idempotency_key], review_from_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    async fn get_artifact_project(&self, artifact_id: &str) -> Result<String> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let project_id: String = conn
            .query_row(
                "SELECT project_id FROM artifacts WHERE artifact_id = ?1",
                params![artifact_id],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    ProjectSpaceStoreError::NotFound(format!("Artifact {artifact_id}"))
                }
                other => ProjectSpaceStoreError::Sqlite(other),
            })?;
        Ok(project_id)
    }

    async fn set_approved_head(
        &self,
        project_id: &str,
        kind: &str,
        artifact_id: &str,
        approved_at: &str,
    ) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        conn.execute(
            "INSERT INTO artifact_approved_heads (project_id, kind, artifact_id, approved_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id, kind) DO UPDATE SET
                 artifact_id = excluded.artifact_id, approved_at = excluded.approved_at",
            params![project_id, kind, artifact_id, approved_at],
        )?;
        Ok(())
    }

    async fn get_approved_head(&self, project_id: &str, kind: &str) -> Result<Option<Artifact>> {
        let artifact_id: Option<String> = {
            let conn = self
                .conn
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            conn.query_row(
                "SELECT artifact_id FROM artifact_approved_heads
                 WHERE project_id = ?1 AND kind = ?2",
                params![project_id, kind],
                |row| row.get::<_, String>(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(ProjectSpaceStoreError::Sqlite(other)),
            })?
        };
        let Some(artifact_id) = artifact_id else {
            return Ok(None);
        };
        // 自愈：head 必须指向真实存在且已批准的版本；否则视为无有效 head。
        match self.get_artifact(&artifact_id).await {
            Ok(a) if a.review_state == owo_agent_protocol::ReviewState::Approved => Ok(Some(a)),
            _ => Ok(None),
        }
    }
}

/// artifact_reviews 行 → 评审记录（decision 以 JSON 文本存 snake_case）。
fn review_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactReviewRecord> {
    let decision_json: String = row.get(4)?;
    Ok(ArtifactReviewRecord {
        review_id: row.get(0)?,
        artifact_id: row.get(1)?,
        artifact_version: row.get(2)?,
        team_id: row.get(3)?,
        decision: serde_json::from_str(&decision_json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        reviewer: row.get(5)?,
        comment: row.get(6)?,
        idempotency_key: row.get(7)?,
        content_ref: row.get(8)?,
        created_at: row.get(9)?,
        step_id: row.get(10)?,
        producer_member_id: row.get(11)?,
    })
}

/// 生产者 member → 生产步骤 id（WorkSwarm 计划约定：`m-{role}` → `s-{role}`）。
///
/// 非 `m-` 前缀成员（如 `human:u1`）返回空串（该来源无对应团队步骤）。
fn producer_step_id(producer_member: &str) -> String {
    producer_member
        .strip_prefix("m-")
        .map(|role| format!("s-{role}"))
        .unwrap_or_default()
}

mod review;

pub use review::{
    apply_artifact_review, self_approve_allowed, ArtifactReviewError, ArtifactReviewInput,
    ArtifactReviewOutcome,
};

// ---------------------------------------------------------------------------
// Artifact 返工任务（V1 五期 · 第二路）：随 ProjectSpace JSON 持久化
// ---------------------------------------------------------------------------

/// 保存（插入或更新）一条返工任务；同一 rework_id 覆盖写（状态回填用）。
pub async fn save_artifact_rework_task(
    store: &dyn ProjectSpaceStoreBackend,
    task: &ArtifactReworkTask,
) -> Result<()> {
    let mut space = store.get_project_space(&task.project_id).await?;
    space.rework_tasks.retain(|t| t.rework_id != task.rework_id);
    space.rework_tasks.push(task.clone());
    space.version += 1;
    space.updated_at = chrono::Utc::now().to_rfc3339();
    store.save_project_space(&space).await
}

/// 列出项目的全部返工任务（created_at 升序）。
pub async fn list_artifact_rework_tasks(
    store: &dyn ProjectSpaceStoreBackend,
    project_id: &str,
) -> Result<Vec<ArtifactReworkTask>> {
    let mut tasks = store.get_project_space(project_id).await?.rework_tasks;
    tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(tasks)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_protocol::{
        ArtifactClassification, MemberHealth, ReviewState, RuntimeBinding, TeamMember, TeamMode,
        TeamRunStatus,
    };

    fn test_store() -> SqliteProjectSpaceStore {
        let conn = Connection::open_in_memory().unwrap();
        SqliteProjectSpaceStore::from_connection(conn).unwrap()
    }

    fn sample_project_space(id: &str) -> ProjectSpace {
        let now = chrono::Utc::now().to_rfc3339();
        ProjectSpace {
            project_id: id.to_string(),
            goal_id: Some("goal-1".to_string()),
            team_id: Some("team-1".to_string()),
            tasks: vec!["task-a".to_string(), "task-b".to_string()],
            artifacts: vec!["art-1".to_string()],
            decisions: vec![],
            approvals: vec![],
            discussions: vec![],
            activity_stream: vec![],
            delivery_manifest_ref: None,
            rework_tasks: vec![],
            version: 1,
            status: owo_agent_protocol::ProjectSpaceStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn sample_artifact(id: &str) -> Artifact {
        Artifact {
            artifact_id: id.to_string(),
            kind: "document".to_string(),
            version: 1,
            producer: "member-planner".to_string(),
            content_ref: "cas://sha256:abc123".to_string(),
            schema_ref: None,
            source_refs: vec!["trace-1".to_string()],
            classification: ArtifactClassification::Private,
            review_state: ReviewState::Draft,
            supersedes_artifact_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            // 七期（第三路）交付扩展字段：测试样例取缺省值。
            team_id: String::new(),
            task_id: None,
            attempt_id: None,
            format: "text".to_string(),
            media_type: "text/plain".to_string(),
            file_name: String::new(),
            sha256: String::new(),
            size_bytes: 0,
            evidence_refs: vec![],
            open_issues: vec![],
            validation: None,
            handoff: None,
        }
    }

    fn sample_team_run(id: &str) -> TeamRun {
        let now = chrono::Utc::now().to_rfc3339();
        TeamRun {
            team_id: id.to_string(),
            goal_id: Some("goal-1".to_string()),
            mode: TeamMode::Team,
            members: vec![TeamMember {
                member_id: "m-planner".to_string(),
                role: "planner".to_string(),
                runtime_binding: RuntimeBinding::Agent {
                    agent_id: "agent-1".to_string(),
                },
                capabilities: vec!["plan".to_string()],
                tool_scope: vec![],
                read_scope: vec!["*".to_string()],
                write_scope: vec!["artifacts".to_string()],
                budget: serde_json::json!({"max_turns": 10}),
                handoff_contract: Some("Plan artifact with steps".to_string()),
                health: MemberHealth::Active,
            }],
            task_graph_ref: Some("graph-1".to_string()),
            project_space_id: Some("proj-1".to_string()),
            template_id: None,
            shared_context_refs: vec![],
            budget: serde_json::json!({"max_duration_secs": 300}),
            human_policy: Some("human_approval_required".to_string()),
            strategy_decision: None,
            status: TeamRunStatus::Created,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn project_space_crud_roundtrip() {
        let store = test_store();
        let space = sample_project_space("proj-crud");

        store.save_project_space(&space).await.unwrap();
        let loaded = store.get_project_space("proj-crud").await.unwrap();
        assert_eq!(loaded.project_id, "proj-crud");
        assert_eq!(loaded.goal_id, Some("goal-1".to_string()));
        assert_eq!(loaded.tasks.len(), 2);
        assert_eq!(loaded.version, 1);

        // Update
        let mut updated = loaded.clone();
        updated.version = 2;
        updated.tasks.push("task-c".to_string());
        store.save_project_space(&updated).await.unwrap();
        let reloaded = store.get_project_space("proj-crud").await.unwrap();
        assert_eq!(reloaded.version, 2);
        assert_eq!(reloaded.tasks.len(), 3);

        // List
        let all = store.list_project_spaces().await.unwrap();
        assert_eq!(all.len(), 1);

        // Delete
        store.delete_project_space("proj-crud").await.unwrap();
        let err = store.get_project_space("proj-crud").await;
        assert!(matches!(err, Err(ProjectSpaceStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn team_delivery_commit_publishes_both_terminal_records() {
        let store = test_store();
        let mut team = sample_team_run("team-1");
        team.status = TeamRunStatus::Succeeded;
        let mut space = sample_project_space("proj-1");
        space.status = owo_agent_protocol::ProjectSpaceStatus::Completed;
        space.delivery_manifest_ref = Some("cas://sha256:manifest".to_string());

        store.commit_team_delivery(&team, &space).await.unwrap();
        assert_eq!(
            store.get_team_run("team-1").await.unwrap().status,
            TeamRunStatus::Succeeded
        );
        let committed_space = store.get_project_space("proj-1").await.unwrap();
        assert_eq!(
            committed_space.status,
            owo_agent_protocol::ProjectSpaceStatus::Completed
        );
        assert_eq!(
            committed_space.delivery_manifest_ref.as_deref(),
            Some("cas://sha256:manifest")
        );
    }

    #[tokio::test]
    async fn team_delivery_commit_rejects_mismatched_records() {
        let store = test_store();
        let mut team = sample_team_run("team-1");
        team.status = TeamRunStatus::Succeeded;
        let mut space = sample_project_space("proj-1");
        space.status = owo_agent_protocol::ProjectSpaceStatus::Completed;
        space.team_id = Some("another-team".to_string());

        assert!(matches!(
            store.commit_team_delivery(&team, &space).await,
            Err(ProjectSpaceStoreError::InvalidState(_))
        ));
        assert!(matches!(
            store.get_team_run("team-1").await,
            Err(ProjectSpaceStoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn team_delivery_commit_rolls_back_if_project_space_write_fails() {
        let store = test_store();
        store
            .conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_project_space_delivery
             BEFORE INSERT ON project_spaces
             BEGIN SELECT RAISE(ABORT, 'injected project space failure'); END;",
            )
            .unwrap();
        let mut team = sample_team_run("team-1");
        team.status = TeamRunStatus::Succeeded;
        let mut space = sample_project_space("proj-1");
        space.status = owo_agent_protocol::ProjectSpaceStatus::Completed;

        assert!(store.commit_team_delivery(&team, &space).await.is_err());
        assert!(matches!(
            store.get_team_run("team-1").await,
            Err(ProjectSpaceStoreError::NotFound(_))
        ));
        assert!(matches!(
            store.get_project_space("proj-1").await,
            Err(ProjectSpaceStoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn artifact_crud_roundtrip() {
        let store = test_store();
        let art = sample_artifact("art-crud");

        store.save_artifact(&art, "proj-1").await.unwrap();
        let loaded = store.get_artifact("art-crud").await.unwrap();
        assert_eq!(loaded.kind, "document");
        assert_eq!(loaded.classification, ArtifactClassification::Private);
        assert_eq!(loaded.review_state, ReviewState::Draft);

        // project_id 真实落库：按 project 过滤命中。
        let by_project = store.list_artifacts_by_project("proj-1").await.unwrap();
        assert_eq!(by_project.len(), 1);
        let other = store.list_artifacts_by_project("proj-2").await.unwrap();
        assert!(other.is_empty(), "其他 project 不应看到该 artifact");

        store.delete_artifact("art-crud").await.unwrap();
        let err = store.get_artifact("art-crud").await;
        assert!(matches!(err, Err(ProjectSpaceStoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn team_run_crud_roundtrip() {
        let store = test_store();
        let tr = sample_team_run("team-crud");

        store.save_team_run(&tr).await.unwrap();
        let loaded = store.get_team_run("team-crud").await.unwrap();
        assert_eq!(loaded.mode, TeamMode::Team);
        assert_eq!(loaded.members.len(), 1);
        assert_eq!(loaded.members[0].role, "planner");
        assert!(matches!(
            loaded.members[0].runtime_binding,
            RuntimeBinding::Agent { .. }
        ));

        // Status transition
        let mut updated = loaded;
        updated.status = TeamRunStatus::Running;
        store.save_team_run(&updated).await.unwrap();
        let reloaded = store.get_team_run("team-crud").await.unwrap();
        assert_eq!(reloaded.status, TeamRunStatus::Running);

        // List & delete
        let all = store.list_team_runs().await.unwrap();
        assert_eq!(all.len(), 1);
        store.delete_team_run("team-crud").await.unwrap();
        assert!(store.list_team_runs().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn decision_and_handoff_roundtrip() {
        let store = test_store();
        let now = chrono::Utc::now().to_rfc3339();

        let decision = DecisionRecord {
            decision_id: "dec-1".to_string(),
            proposer: "m-leader".to_string(),
            choice: "Use template A for browser tasks".to_string(),
            rationale: "Template A has higher grounding accuracy in S1 benchmarks.".to_string(),
            affected_refs: vec!["task-b".to_string()],
            created_at: now.clone(),
        };
        store.save_decision(&decision, "proj-1").await.unwrap();
        let loaded_dec = store.get_decision("dec-1").await.unwrap();
        assert_eq!(loaded_dec.choice, "Use template A for browser tasks");

        let handoff = HandoffRecord {
            handoff_id: "ho-1".to_string(),
            from_member: "m-planner".to_string(),
            to_member: "m-builder".to_string(),
            completed_summary: "Plan generated with 5 steps".to_string(),
            open_issues: vec!["Step 3 needs clarification".to_string()],
            output_artifact_refs: vec!["art-plan-v1".to_string()],
            evidence_refs: vec!["trace-plan".to_string()],
            suggested_next_actions: vec!["Execute step 1-2".to_string()],
            known_risks: vec!["Step 4 may timeout".to_string()],
            created_at: now.clone(),
            // 七期（第三路）：Worker 交接说明（测试样例不涉及，保持 None）。
            handoff_note: None,
        };
        store.save_handoff(&handoff, "proj-1").await.unwrap();
        let loaded_ho = store.get_handoff("ho-1").await.unwrap();
        assert_eq!(loaded_ho.from_member, "m-planner");
        assert_eq!(loaded_ho.open_issues.len(), 1);

        // project_id 真实落库：decisions/handoffs 按 project 过滤命中。
        let decs = store.list_decisions_by_project("proj-1").await.unwrap();
        assert_eq!(decs.len(), 1);
        let hos = store.list_handoffs_by_project("proj-1").await.unwrap();
        assert_eq!(hos.len(), 1);
    }

    #[tokio::test]
    async fn dto_serialization_stability() {
        // Verify key enums serialize to expected snake_case strings
        let mode: TeamMode = TeamMode::Swarmflow;
        assert_eq!(serde_json::to_string(&mode).unwrap(), "\"swarmflow\"");

        let binding = RuntimeBinding::Human {
            user_id: "u1".to_string(),
        };
        let json = serde_json::to_value(&binding).unwrap();
        assert_eq!(json["kind"], "human");
        assert_eq!(json["user_id"], "u1");

        let status = TeamRunStatus::AwaitingHuman;
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            "\"awaiting_human\""
        );

        let health = MemberHealth::Fused;
        assert_eq!(serde_json::to_string(&health).unwrap(), "\"fused\"");

        let cls = ArtifactClassification::Sensitive;
        assert_eq!(serde_json::to_string(&cls).unwrap(), "\"sensitive\"");

        let rs = ReviewState::PendingReview;
        assert_eq!(serde_json::to_string(&rs).unwrap(), "\"pending_review\"");
    }
}

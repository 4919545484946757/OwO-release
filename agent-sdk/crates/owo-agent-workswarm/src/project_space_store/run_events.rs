//! One indexed run journal in the existing ProjectSpace database.
//! Domain writers append inside their own transaction. No model/runtime dependency.
use super::*;
use owo_agent_protocol::{RunEvent, RunEventPage};
use rusqlite::{OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_RUN_EVENT_BYTES: usize = 16 * 1024;
pub const MAX_RUN_EVENT_PAGE: usize = 256;

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS run_events(
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 run_id TEXT NOT NULL,
 source_key TEXT NOT NULL,
 event_type TEXT NOT NULL,
 timestamp TEXT NOT NULL,
 data_json TEXT NOT NULL,
 UNIQUE(run_id,source_key)
);
CREATE INDEX IF NOT EXISTS idx_run_events_run_seq ON run_events(run_id,seq);
CREATE INDEX IF NOT EXISTS idx_run_events_run_type ON run_events(run_id,event_type);
";

/// Host-only event candidate. source_key binds a durable source identity for idempotent recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRunEvent {
    pub run_id: String,
    pub source_key: String,
    pub event_type: String,
    pub node_id: Option<String>,
    pub task_id: Option<String>,
    pub attempt: Option<String>,
    pub causation_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub artifact_refs: Vec<String>,
    pub data: Value,
}
impl NewRunEvent {
    /// Validate a candidate before its canonical checkpoint can commit.
    pub fn check_limits(&self) -> Result<()> {
        validate(self).map(|_| ())
    }
    pub fn new(run_id: &str, source_key: String, event_type: &str, data: Value) -> Self {
        Self {
            run_id: run_id.into(),
            source_key,
            event_type: event_type.into(),
            node_id: None,
            task_id: None,
            attempt: None,
            causation_id: None,
            tool_call_id: None,
            artifact_refs: Vec::new(),
            data,
        }
    }
}
fn invalid(reason: impl Into<String>) -> ProjectSpaceStoreError {
    ProjectSpaceStoreError::InvalidState(reason.into())
}
fn validate(input: &NewRunEvent) -> Result<String> {
    for value in [&input.run_id, &input.source_key, &input.event_type]
        .into_iter()
        .chain(
            [
                &input.node_id,
                &input.task_id,
                &input.attempt,
                &input.causation_id,
                &input.tool_call_id,
            ]
            .into_iter()
            .filter_map(|s| s.as_ref()),
        )
    {
        if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
            return Err(invalid("invalid run event identity"));
        }
    }
    if input.artifact_refs.len() > 128 || input.artifact_refs.iter().any(|r| r.len() > 1024) {
        return Err(invalid("run event reference limit exceeded"));
    }
    let raw = serde_json::to_string(input)?;
    if raw.len() > MAX_RUN_EVENT_BYTES {
        return Err(invalid("run event exceeds 16 KiB"));
    }
    Ok(raw)
}
pub(super) fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunEvent> {
    let seq: i64 = row.get(0)?;
    let run_id: String = row.get(1)?;
    let raw: String = row.get(3)?;
    let input: NewRunEvent = serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    if seq <= 0 || input.run_id != run_id {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(RunEvent {
        schema_version: 1,
        seq: seq as u64,
        event_id: format!("run:{run_id}:{seq}"),
        run_id,
        timestamp: row.get(2)?,
        event_type: input.event_type,
        node_id: input.node_id,
        task_id: input.task_id,
        attempt: input.attempt,
        causation_id: input.causation_id,
        tool_call_id: input.tool_call_id,
        artifact_refs: input.artifact_refs,
        data: input.data,
    })
}
pub(super) const SELECT: &str = "SELECT seq,run_id,timestamp,data_json FROM run_events";

/// Must be called under a write transaction (including by domain persistence).
pub(super) fn append(conn: &Connection, input: &NewRunEvent) -> Result<RunEvent> {
    let raw = validate(input)?;
    if let Some((seq, previous)) = conn
        .query_row(
            "SELECT seq,data_json FROM run_events WHERE run_id=?1 AND source_key=?2",
            params![input.run_id, input.source_key],
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        if previous != raw {
            return Err(invalid("run event source identity changed"));
        }
        return Ok(conn.query_row(&format!("{SELECT} WHERE seq=?1"), [seq], decode)?);
    }
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM team_runs WHERE team_id=?1)",
        [&input.run_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(ProjectSpaceStoreError::NotFound(format!(
            "TeamRun {}",
            input.run_id
        )));
    }
    if let Some(cause) = input.causation_id.as_deref() {
        let Some(prefix) = cause.strip_prefix(&format!("run:{}:", input.run_id)) else {
            return Err(invalid("run event cause belongs to another run"));
        };
        let seq = prefix
            .parse::<u64>()
            .map_err(|_| invalid("invalid run event cause"))?;
        if seq == 0 || seq > i64::MAX as u64 || cause != format!("run:{}:{seq}", input.run_id) {
            return Err(invalid("invalid run event cause"));
        }
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1 AND seq=?2)",
            params![input.run_id, seq],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(invalid("run event cause does not exist"));
        }
    }
    conn.execute("INSERT INTO run_events(run_id,source_key,event_type,timestamp,data_json) VALUES(?1,?2,?3,?4,?5)",
        params![input.run_id,input.source_key,input.event_type,chrono::Utc::now().to_rfc3339(),raw])?;
    Ok(conn.query_row(
        &format!("{SELECT} WHERE seq=?1"),
        [conn.last_insert_rowid()],
        decode,
    )?)
}

pub(super) fn team_transition(
    conn: &Connection,
    team: &TeamRun,
    previous: Option<&str>,
) -> Result<()> {
    let previous = previous.map(serde_json::from_str::<TeamRun>).transpose()?;
    if previous
        .as_ref()
        .is_some_and(|old| old.status == team.status)
    {
        return Ok(());
    }
    let kind = if previous.is_none() {
        "team.created"
    } else {
        "team.status_changed"
    };
    append(
        conn,
        &NewRunEvent::new(
            &team.team_id,
            uuid::Uuid::new_v4().to_string(),
            kind,
            json!({"status":team.status,"previous_status":previous.map(|old|old.status)}),
        ),
    )?;
    Ok(())
}
fn project_run(conn: &Connection, project_id: &str) -> Result<Option<String>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT data_json FROM project_spaces WHERE project_id=?1",
            [project_id],
            |r| r.get(0),
        )
        .optional()?;
    let space = raw
        .map(|raw| serde_json::from_str::<ProjectSpace>(&raw))
        .transpose()?;
    Ok(space.and_then(|p| p.team_id))
}
fn known_run(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM team_runs WHERE team_id=?1)",
        [id],
        |r| r.get(0),
    )?)
}
pub(super) fn artifact_saved(
    conn: &Connection,
    artifact: &Artifact,
    _raw: &str,
    project: &str,
) -> Result<()> {
    let run = if artifact.team_id.is_empty() {
        project_run(conn, project)?
    } else {
        Some(artifact.team_id.clone())
    };
    let Some(run) = run else { return Ok(()) };
    // Generic non-Team artifacts and legacy orphan references keep their old storage contract.
    if !known_run(conn, &run)? {
        return Ok(());
    }
    let mut event = NewRunEvent::new(
        &run,
        uuid::Uuid::new_v4().to_string(),
        "artifact.saved",
        json!({"artifact_id":artifact.artifact_id,"version":artifact.version,
        "review_state":artifact.review_state,"content_ref":artifact.content_ref,
        "sha256":artifact.sha256,"media_type":artifact.media_type}),
    );
    event.task_id = artifact.task_id.clone();
    event.attempt = artifact.attempt_id.clone();
    event.artifact_refs = vec![artifact.artifact_id.clone()];
    append(conn, &event)?;
    Ok(())
}
pub(super) fn handoff_saved(
    conn: &Connection,
    handoff: &HandoffRecord,
    _raw: &str,
    project: &str,
) -> Result<()> {
    let Some(run) = project_run(conn, project)? else {
        return Ok(());
    };
    if !known_run(conn, &run)? {
        return Ok(());
    }
    let mut event = NewRunEvent::new(
        &run,
        uuid::Uuid::new_v4().to_string(),
        "handoff.saved",
        json!({"handoff_id":handoff.handoff_id,
        "from_member":handoff.from_member,"to_member":handoff.to_member}),
    );
    event.artifact_refs = handoff.output_artifact_refs.clone();
    append(conn, &event)?;
    Ok(())
}

impl SqliteProjectSpaceStore {
    pub(super) fn run_event_by_source_sql(
        &self,
        run_id: &str,
        source: &str,
    ) -> Result<Option<RunEvent>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        Ok(conn
            .query_row(
                &format!("{SELECT} WHERE run_id=?1 AND source_key=?2"),
                params![run_id, source],
                decode,
            )
            .optional()?)
    }

    pub(super) fn append_run_event_sql(&self, input: &NewRunEvent) -> Result<RunEvent> {
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = append(&tx, input)?;
        tx.commit()?;
        Ok(result)
    }
    pub(super) fn run_events_after_sql(
        &self,
        run_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<RunEventPage> {
        self.run_event_page_sql(run_id, after, limit, false)
    }

    pub(super) fn run_events_before_sql(
        &self,
        run_id: &str,
        before: u64,
        limit: usize,
    ) -> Result<owo_agent_protocol::RunEventHistoryPage> {
        let page = self.run_event_page_sql(run_id, before, limit, true)?;
        Ok(owo_agent_protocol::RunEventHistoryPage {
            next_before: page.events.first().map(|event| event.seq),
            run_id: page.run_id,
            events: page.events,
            high_watermark: page.high_watermark,
            has_more: page.has_more,
            history_complete: page.history_complete,
            task_history_complete: page.task_history_complete,
        })
    }

    fn run_event_page_sql(
        &self,
        run_id: &str,
        after: u64,
        limit: usize,
        backwards: bool,
    ) -> Result<RunEventPage> {
        if !(1..=MAX_RUN_EVENT_PAGE).contains(&limit) || after > i64::MAX as u64 {
            return Err(invalid("invalid run event page limit or cursor"));
        }
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction()?; // one read snapshot for cursor, high-water mark and rows
        if !known_run(&tx, run_id)? {
            return Err(ProjectSpaceStoreError::NotFound(format!(
                "TeamRun {run_id}"
            )));
        }
        if after != 0 {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1 AND seq=?2)",
                params![run_id, after],
                |r| r.get(0),
            )?;
            if !exists {
                return Err(invalid(
                    "run event cursor is unknown or belongs to another run",
                ));
            }
        }
        let high_watermark: u64 = tx.query_row(
            "SELECT COALESCE(MAX(seq),0) FROM run_events WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )?;
        let history_complete: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1 AND event_type='team.created')",
            [run_id],
            |r| r.get(0),
        )?;
        let mut events = {
            let sql = if backwards {
                let comparison = if after == 0 { "<=" } else { "<" };
                format!("{SELECT} WHERE run_id=?1 AND seq{comparison}?2 ORDER BY seq DESC LIMIT ?3")
            } else {
                format!("{SELECT} WHERE run_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")
            };
            let mut stmt = tx.prepare(&sql)?;
            let cursor = if backwards && after == 0 {
                high_watermark
            } else {
                after
            };
            let rows = stmt.query_map(params![run_id, cursor, limit as u64 + 1], decode)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let has_more = events.len() > limit;
        events.truncate(limit);
        if backwards {
            events.reverse();
        }
        let next_after = events.last().map(|e| e.seq).unwrap_or(after);
        tx.commit()?;
        Ok(RunEventPage {
            run_id: run_id.into(),
            events,
            next_after,
            high_watermark,
            has_more,
            history_complete,
            task_history_complete: None,
        })
    }
}

#[cfg(test)]
#[path = "run_events_tests.rs"]
mod tests;

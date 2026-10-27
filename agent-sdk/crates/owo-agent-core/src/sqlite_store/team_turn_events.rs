//! Transactional outbox of references to canonical public Team conversation records.
use super::*;
use crate::session::{TeamTurnBinding, TeamTurnDelivery};

pub(super) const SCHEMA: &str = "
CREATE TABLE team_turn_delivery(
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 run_id TEXT NOT NULL, session_id TEXT NOT NULL, session_seq INTEGER NOT NULL,
 turn_id TEXT NOT NULL, created_at TEXT NOT NULL, binding_json TEXT NOT NULL,
 event_type TEXT NOT NULL, tool_call_id TEXT, cause_seq INTEGER,
 artifact_refs_json TEXT NOT NULL, artifact_refs_complete INTEGER NOT NULL,
 delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN (0,1)),
 UNIQUE(session_id, session_seq)
);
CREATE INDEX idx_team_turn_pending ON team_turn_delivery(run_id,delivered,id);
CREATE INDEX idx_team_turn_tool ON team_turn_delivery(run_id,session_id,tool_call_id,event_type,session_seq);
";

fn error(message: &str) -> AgentError {
    AgentError::Session(message.into())
}
fn check_binding(binding: &TeamTurnBinding, session: &str, turn: &str) -> Result<(), AgentError> {
    let fields = [
        Some(&binding.run_id),
        Some(&binding.task_id),
        binding.node_id.as_ref(),
        binding.attempt.as_ref(),
    ];
    for value in fields.into_iter().flatten() {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(error("invalid Team event binding"));
        }
    }
    if !session.starts_with(&format!("{}-worker-", binding.run_id))
        || session.len() > 384
        || session.chars().any(char::is_control)
        || turn.is_empty()
        || turn.len() > 256
        || turn.chars().any(char::is_control)
    {
        return Err(error("Team event session does not match host binding"));
    }
    Ok(())
}
fn event_identity(payload: &SseEvent) -> Result<(&'static str, Option<&str>), AgentError> {
    match payload {
        SseEvent::TokenDelta { .. } => Ok(("message.delta", None)),
        SseEvent::Final { .. } => Ok(("message.final", None)),
        SseEvent::Progress { .. } => Ok(("member.progress", None)),
        SseEvent::PlanUpdate { .. } => Ok(("plan.updated", None)),
        SseEvent::ToolUse { id, .. } => Ok(("tool.started", Some(id))),
        SseEvent::ToolResult { id, .. } => Ok(("tool.finished", Some(id))),
        _ => Err(error("event is not a public Team worker observation")),
    }
}
impl SqliteSessionStore {
    pub(super) fn append_team_event_sql(
        &self,
        binding: &TeamTurnBinding,
        session: &str,
        turn: &str,
        payload: &SseEvent,
    ) -> Result<TurnEventRecord, AgentError> {
        check_binding(binding, session, turn)?;
        let (kind, tool) = event_identity(payload)?;
        if tool
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err(error("invalid tool call identity"));
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| error("SQLite lock poisoned"))?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let record = append_event_tx(&tx, session, turn, payload)?;
        let cause: Option<u64> = if kind == "tool.finished" {
            tx.query_row(
                "SELECT session_seq FROM team_turn_delivery
                WHERE run_id=?1 AND session_id=?2 AND tool_call_id=?3 AND event_type='tool.started'
                AND session_seq<?4 ORDER BY session_seq DESC LIMIT 1",
                params![binding.run_id, session, tool, record.seq],
                |r| r.get(0),
            )
            .optional()
            .map_err(sqlite_error)?
        } else {
            None
        };
        let mut refs = Vec::new();
        let mut ref_bytes = 0;
        let mut refs_complete = true;
        if let SseEvent::ToolResult { outputs, .. } = payload {
            for output in outputs {
                let id = &output.artifact_id;
                let cost = serde_json::to_string(id)
                    .map_err(|_| error("invalid artifact id"))?
                    .len()
                    + 1;
                if refs.len() >= 128
                    || id.is_empty()
                    || id.len() > 256
                    || id.chars().any(char::is_control)
                    || ref_bytes + cost > 8190
                {
                    refs_complete = false;
                    continue;
                }
                refs.push(id.clone());
                ref_bytes += cost;
            }
        }
        let refs_json =
            serde_json::to_string(&refs).map_err(|_| error("invalid artifact references"))?;
        let binding_json =
            serde_json::to_string(binding).map_err(|_| error("invalid binding JSON"))?;
        tx.execute("INSERT INTO team_turn_delivery(run_id,session_id,session_seq,turn_id,created_at,binding_json,event_type,tool_call_id,cause_seq,artifact_refs_json,artifact_refs_complete)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![binding.run_id,session,record.seq,turn,record.created_at,binding_json,kind,tool,cause,refs_json,refs_complete]).map_err(sqlite_error)?;
        tx.commit().map_err(sqlite_error)?;
        Ok(record)
    }
    pub(super) fn pending_team_events_sql(
        &self,
        run: &str,
        limit: usize,
    ) -> Result<Vec<TeamTurnDelivery>, AgentError> {
        if !(1..=257).contains(&limit) {
            return Err(error("invalid delivery page size"));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| error("SQLite lock poisoned"))?;
        let mut stmt=conn.prepare("SELECT id,binding_json,session_id,session_seq,turn_id,created_at,event_type,tool_call_id,cause_seq,artifact_refs_json,artifact_refs_complete
            FROM team_turn_delivery WHERE run_id=?1 AND delivered=0 ORDER BY id LIMIT ?2").map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![run, limit], |row| {
                let raw: String = row.get(1)?;
                let binding: TeamTurnBinding = serde_json::from_str(&raw).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                if binding.run_id != run {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                Ok(TeamTurnDelivery {
                    delivery_id: row.get(0)?,
                    binding,
                    session_id: row.get(2)?,
                    session_seq: row.get(3)?,
                    turn_id: row.get(4)?,
                    created_at: row.get(5)?,
                    event_type: row.get(6)?,
                    tool_call_id: row.get(7)?,
                    cause_seq: row.get(8)?,
                    artifact_refs: serde_json::from_str(&row.get::<_, String>(9)?).map_err(
                        |e| {
                            rusqlite::Error::FromSqlConversionFailure(
                                9,
                                rusqlite::types::Type::Text,
                                Box::new(e),
                            )
                        },
                    )?,
                    artifact_refs_complete: row.get(10)?,
                })
            })
            .map_err(sqlite_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sqlite_error)
    }
    pub(super) fn acknowledge_team_event_sql(&self, run: &str, id: i64) -> Result<(), AgentError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| error("SQLite lock poisoned"))?;
        let updated = conn
            .execute(
                "UPDATE team_turn_delivery SET delivered=1 WHERE id=?1 AND run_id=?2",
                params![id, run],
            )
            .map_err(sqlite_error)?;
        if updated != 1 {
            return Err(error("Team delivery acknowledgement identity is missing"));
        }
        Ok(())
    }
    pub(super) fn turn_event_json_bounded_sql(
        &self,
        session: &str,
        seq: u64,
        max: usize,
    ) -> Result<Option<String>, AgentError> {
        if seq == 0 || seq > i64::MAX as u64 || !(1..=65536).contains(&max) {
            return Err(error("invalid bounded event read"));
        }
        let conn = self
            .conn
            .lock()
            .map_err(|_| error("SQLite lock poisoned"))?;
        conn.query_row("SELECT CASE WHEN length(CAST(payload_json AS BLOB))<=?3 THEN payload_json ELSE NULL END
            FROM turn_events WHERE session_id=?1 AND seq=?2",params![session,seq,max],
            |r|r.get::<_,Option<String>>(0)).optional().map(Option::flatten).map_err(sqlite_error)
    }
}

pub(super) fn append_event_tx(
    transaction: &rusqlite::Transaction<'_>,
    session_id: &str,
    turn_id: &str,
    payload: &SseEvent,
) -> Result<TurnEventRecord, AgentError> {
    let last_seq = transaction
        .query_row(
            "SELECT last_seq FROM turn_event_cursors WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(sqlite_error)?
        .unwrap_or(0);
    // 思维链合并：模型一次回合可吐数万条 reasoning_delta，逐条入库会造成
    // ~95% 的写放大（实测单会话 7.8 万行）。连续 reasoning 增量并入上一行，
    // 单行超过 64KiB 才另起一行，既有 seq 语义与断点续传都不受影响。
    if let SseEvent::ReasoningDelta { delta } = payload {
        let previous: Option<(i64, String)> = transaction
            .query_row(
                "SELECT seq, payload_json FROM turn_events
                     WHERE session_id = ?1 ORDER BY seq DESC LIMIT 1",
                [session_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        if let Some((previous_seq, previous_json)) = previous {
            if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&previous_json) {
                if value.get("type").and_then(|kind| kind.as_str()) == Some("reasoning_delta") {
                    let merged = format!(
                        "{}{}",
                        value
                            .get("delta")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default(),
                        delta
                    );
                    if merged.len() <= 64 * 1024 {
                        value["delta"] = serde_json::Value::String(merged.clone());
                        let merged_json = serde_json::to_string(&value).map_err(|error| {
                            AgentError::Session(format!("回合事件编码失败：{error}"))
                        })?;
                        transaction
                            .execute(
                                "UPDATE turn_events SET payload_json = ?1
                                     WHERE session_id = ?2 AND seq = ?3",
                                params![merged_json, session_id, previous_seq],
                            )
                            .map_err(sqlite_error)?;
                        return Ok(TurnEventRecord {
                            session_id: session_id.to_string(),
                            turn_id: turn_id.to_string(),
                            seq: previous_seq as u64,
                            created_at: chrono::Utc::now().to_rfc3339(),
                            payload: SseEvent::ReasoningDelta { delta: merged },
                        });
                    }
                }
            }
        }
    }
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
    Ok(TurnEventRecord {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        seq: seq as u64,
        created_at,
        payload: payload.clone(),
    })
}

impl SqliteSessionStore {
    pub(super) fn pending_conversation_sessions_sql(
        &self,
        run_id: &str,
        cursors: &std::collections::BTreeMap<String, u64>,
        after_session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, AgentError> {
        if run_id.is_empty()
            || run_id.len() > 256
            || run_id.chars().any(char::is_control)
            || !(1..=64).contains(&limit)
        {
            return Err(error("invalid conversation session query"));
        }
        let prefix = format!("{run_id}-worker-");
        // All host Team session IDs share this literal prefix; no LIKE wildcards.
        let upper = format!("{run_id}-worker.");
        if cursors.iter().any(|(id, seq)| {
            !id.starts_with(&prefix)
                || id.len() > 384
                || id.chars().any(char::is_control)
                || *seq > i64::MAX as u64
        }) {
            return Err(error("invalid or foreign conversation cursor"));
        }
        if after_session.is_some_and(|id| {
            !id.starts_with(&prefix) || id.len() > 384 || id.chars().any(char::is_control)
        }) {
            return Err(error("invalid conversation session rotation cursor"));
        }
        let json =
            serde_json::to_string(cursors).map_err(|_| error("invalid conversation cursors"))?;
        let conn = self
            .conn
            .lock()
            .map_err(|_| error("SQLite lock poisoned"))?;
        let mut statement = conn
            .prepare(
                "SELECT c.session_id FROM turn_event_cursors c
             WHERE c.session_id>=?1 AND c.session_id<?2
               AND c.session_id>?5
               AND c.last_seq>COALESCE(json_extract(?3, '$.' || json_quote(c.session_id)),0)
               AND EXISTS(SELECT 1 FROM sessions s WHERE s.id=c.session_id)
             ORDER BY c.session_id LIMIT ?4",
            )
            .map_err(sqlite_error)?;
        let mut read = |after: &str| -> Result<Vec<String>, AgentError> {
            statement
                .query_map(params![prefix, upper, json, limit as i64, after], |row| {
                    row.get(0)
                })
                .map_err(sqlite_error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_error)
        };
        let page = read(after_session.unwrap_or(""))?;
        if page.is_empty() && after_session.is_some() {
            read("")
        } else {
            Ok(page)
        }
    }
}

#[cfg(test)]
#[path = "team_turn_events_tests.rs"]
mod tests;

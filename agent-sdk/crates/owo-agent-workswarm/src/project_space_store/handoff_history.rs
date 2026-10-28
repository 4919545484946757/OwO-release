//! Indexed reverse keyset reads of canonical handoffs; no second history store.
use super::*;

impl SqliteProjectSpaceStore {
    pub(super) fn handoffs_before_sql(
        &self,
        project: &str,
        before: Option<&str>,
        limit: usize,
    ) -> Result<owo_agent_protocol::HandoffPage> {
        if !(1..=128).contains(&limit) {
            return Err(ProjectSpaceStoreError::InvalidState(
                "handoff limit must be 1..=128".into(),
            ));
        }
        if before
            .is_some_and(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err(ProjectSpaceStoreError::InvalidState(
                "invalid handoff cursor".into(),
            ));
        }
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let tx = conn.transaction()?;
        let boundary: Option<(String, String)> = match before {
            Some(id) => Some(tx.query_row(
                "SELECT created_at, handoff_id FROM handoffs WHERE project_id=?1 AND handoff_id=?2",
                params![project, id], |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?.ok_or_else(|| ProjectSpaceStoreError::InvalidState("unknown or foreign handoff cursor".into()))?),
            None => None,
        };
        let sql = if boundary.is_some() {
            "SELECT handoff_id, data_json FROM handoffs WHERE project_id=?1
             AND (created_at, handoff_id)<(?2, ?3)
             ORDER BY created_at DESC, handoff_id DESC LIMIT ?4"
        } else {
            "SELECT handoff_id, data_json FROM handoffs WHERE project_id=?1
             ORDER BY created_at DESC, handoff_id DESC LIMIT ?2"
        };
        let mut statement = tx.prepare(sql)?;
        let read = |r: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String)> {
            Ok((r.get(0)?, r.get(1)?))
        };
        let mut rows: Vec<(String, String)> = if let Some((time, id)) = boundary {
            statement
                .query_map(params![project, time, id, (limit + 1) as i64], read)?
                .collect::<std::result::Result<_, _>>()?
        } else {
            statement
                .query_map(params![project, (limit + 1) as i64], read)?
                .collect::<std::result::Result<_, _>>()?
        };
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        rows.reverse();
        let next_before = rows.first().map(|(id, _)| id.clone());
        let handoffs = rows
            .into_iter()
            .map(|(_, json)| Self::deserialize(&json))
            .collect::<Result<Vec<_>>>()?;
        drop(statement);
        tx.commit()?;
        Ok(owo_agent_protocol::HandoffPage {
            handoffs,
            next_before,
            has_more,
        })
    }
}

#[cfg(test)]
mod tests;

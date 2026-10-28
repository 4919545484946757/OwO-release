//! Persisted, bounded run history. v1 /events remains available during client migration.
use super::*;
use owo_agent_protocol::RunEventPage;

const STREAM_PAGE: usize = 64;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Deserialize)]
pub(super) struct TimelineQuery {
    after: Option<u64>,
    before: Option<u64>,
    limit: Option<usize>,
    format: Option<String>,
}

fn bad_request(message: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error":message,"code":"workswarm/invalid_cursor"})),
    )
}
fn resume_cursor(
    id: &str,
    after: Option<u64>,
    headers: &axum::http::HeaderMap,
) -> Result<u64, (StatusCode, Json<Value>)> {
    let Some(header) = headers.get("last-event-id") else {
        return Ok(after.unwrap_or(0));
    };
    let header = header
        .to_str()
        .map_err(|_| bad_request("invalid Last-Event-ID"))?;
    let prefix = format!("run:{id}:");
    let suffix = header
        .strip_prefix(&prefix)
        .ok_or_else(|| bad_request("Last-Event-ID belongs to another run"))?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad_request("invalid Last-Event-ID"));
    }
    let seq = suffix
        .parse::<u64>()
        .map_err(|_| bad_request("invalid Last-Event-ID"))?;
    if seq == 0 || seq > i64::MAX as u64 || suffix != seq.to_string() {
        return Err(bad_request("invalid Last-Event-ID"));
    }
    if after.is_some_and(|after| after != seq) {
        return Err(bad_request("after and Last-Event-ID disagree"));
    }
    Ok(seq)
}

pub(super) async fn team_timeline(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<TimelineQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|e| error_response(&e))?;
    // Resource lookup before opening any stream; the existing router enforces authentication.
    coordinator
        .get_team_run(&id)
        .await
        .map_err(|e| error_response(&e))?;
    let sse = match query.format.as_deref() {
        Some("sse") => true,
        None | Some("json") => false,
        _ => return Err(bad_request("format must be json or sse")),
    };
    if query.before.is_some()
        && (sse || query.after.is_some() || headers.contains_key("last-event-id"))
    {
        return Err(bad_request(
            "before is JSON-only and cannot be combined with after or Last-Event-ID",
        ));
    }
    let after = resume_cursor(&id, query.after, &headers)?;
    let limit = query.limit.unwrap_or(STREAM_PAGE);
    if !(1..=256).contains(&limit) {
        return Err(bad_request("limit must be between 1 and 256"));
    }
    let limit = if sse { limit.min(STREAM_PAGE) } else { limit };
    if let Some(before) = query.before {
        let mut page = super::super::team_event_delivery::history_page(
            &coordinator,
            state.store.as_ref(),
            &id,
            before,
            limit,
        )
        .map_err(|e| error_response(&e))?;
        for record in &mut page.events {
            record.data = workswarm_metrics::sanitize_value(&record.data);
        }
        return Ok(Json(page).into_response());
    }

    let page = super::super::team_event_delivery::page(
        &coordinator,
        state.store.as_ref(),
        &id,
        after,
        limit,
    )
    .map_err(|e| error_response(&e))?;
    if !sse {
        let mut page = page;
        for record in &mut page.events {
            record.data = workswarm_metrics::sanitize_value(&record.data);
        }
        return Ok(Json(page).into_response());
    }
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    tokio::spawn(stream(
        coordinator,
        Arc::clone(&state.store),
        id,
        page,
        after,
        limit,
        tx,
    ));
    Ok(Sse::new(ReceiverStream::new(rx))
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}

async fn send(
    tx: &tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
    event: Event,
    timeout: Duration,
) -> bool {
    matches!(
        tokio::time::timeout(timeout, tx.send(Ok(event))).await,
        Ok(Ok(()))
    )
}

async fn stream(
    coordinator: Arc<TeamCoordinator>,
    store: Arc<dyn owo_agent_core::SessionStore>,
    id: String,
    mut page: RunEventPage,
    mut after: u64,
    limit: usize,
    tx: tokio::sync::mpsc::Sender<Result<Event, Infallible>>,
) {
    let meta = json!({"run_id":id,"schema_version":1,"high_watermark":page.high_watermark,
        "history_complete":page.history_complete,"task_history_complete":page.task_history_complete});
    if !send(
        &tx,
        Event::default().event("timeline").data(meta.to_string()),
        SEND_TIMEOUT,
    )
    .await
    {
        return;
    }
    loop {
        for record in &page.events {
            let mut public = json!(record);
            // Keep protocol identities stable. Only public event data is free-form.
            public["data"] = workswarm_metrics::sanitize_value(&record.data);
            let event = Event::default()
                .event("run_event")
                .id(&record.event_id)
                .data(public.to_string());
            if !send(&tx, event, SEND_TIMEOUT).await {
                return;
            }
            after = record.seq;
        }
        if !page.has_more {
            let Ok(team) = coordinator.get_team_run(&id).await else {
                return;
            };
            if team.status.is_terminal() {
                // Status and its event commit together. Re-read after observing terminal status
                // so a terminal transition committed after the previous page cannot be lost.
                match super::super::team_event_delivery::page(
                    &coordinator,
                    store.as_ref(),
                    &id,
                    after,
                    limit,
                ) {
                    Ok(next) if next.events.is_empty() => return,
                    Ok(next) => {
                        page = next;
                        continue;
                    }
                    Err(_) => break,
                }
            }
            tokio::select! {
                _=tx.closed()=>return,
                _=tokio::time::sleep(Duration::from_millis(500))=>{},
            }
        }
        match super::super::team_event_delivery::page(
            &coordinator,
            store.as_ref(),
            &id,
            after,
            limit,
        ) {
            Ok(next) => page = next,
            Err(_) => break,
        }
    }
    // Do not fabricate progress or advance the cursor when durable history cannot be read.
    let _ = send(
        &tx,
        Event::default()
            .event("timeline_error")
            .data(r#"{"code":"workswarm/timeline_unavailable"}"#),
        SEND_TIMEOUT,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_rejects_foreign_ambiguous_and_noncanonical_identities() {
        let mut headers = axum::http::HeaderMap::new();
        for bad in [
            "run:other:12",
            "run:team:0",
            "run:team:012",
            "run:team:+12",
            "run:team:18446744073709551615",
        ] {
            headers.insert("last-event-id", bad.parse().unwrap());
            assert!(resume_cursor("team", None, &headers).is_err(), "{bad}");
        }
        headers.insert("last-event-id", "run:team:12".parse().unwrap());
        assert_eq!(resume_cursor("team", Some(12), &headers).unwrap(), 12);
        assert!(resume_cursor("team", Some(11), &headers).is_err());
    }
    #[tokio::test]
    async fn disconnected_and_stalled_consumers_release_producer() {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tx.send(Ok(Event::default())).await.unwrap();
        assert!(!send(&tx, Event::default(), Duration::from_millis(20)).await);
        drop(rx);
        assert!(!send(&tx, Event::default(), Duration::from_secs(1)).await);
    }
}

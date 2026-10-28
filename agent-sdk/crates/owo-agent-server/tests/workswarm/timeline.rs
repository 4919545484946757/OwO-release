use super::*;

async fn completed_team(state: &Arc<owo_agent_server::AppState>, app: &axum::Router) -> String {
    let create = json!({"strategy":"team","objective":"timeline local integration","roles":echo_relay_roles()});
    let (status, created) = call(state, app, "POST", "/teams", Some(&create.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap().to_owned();
    let detail = poll_team_status(
        state,
        app,
        &id,
        &["succeeded", "failed"],
        std::time::Duration::from_secs(30),
    )
    .await;
    assert_eq!(detail["team"]["status"], "succeeded", "{detail}");
    id
}

#[tokio::test]
async fn paged_timeline_survives_coordinator_restart_and_resumes_without_gaps() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let id = completed_team(&state, &app).await;
    let mut after = 0;
    let mut events = Vec::new();
    loop {
        let (status, page) = call(
            &state,
            &app,
            "GET",
            &format!("/teams/{id}/timeline?after={after}&limit=2"),
            None,
        )
        .await;
        assert_eq!(status, 200, "{page}");
        assert_eq!(page["history_complete"], true);
        assert_eq!(page["task_history_complete"], true);
        let records = page["events"].as_array().unwrap();
        assert!(records.len() <= 2);
        events.extend(records.iter().cloned());
        after = page["next_after"].as_u64().unwrap();
        if page["has_more"] == false {
            assert_eq!(page["high_watermark"], after);
            break;
        }
    }
    assert_eq!(events.first().unwrap()["event_type"], "team.created");
    assert_eq!(events.last().unwrap()["data"]["status"], "succeeded");
    assert!(events
        .windows(2)
        .all(|w| w[0]["seq"].as_u64() < w[1]["seq"].as_u64()));
    let artifact = events
        .iter()
        .find(|e| e["event_type"] == "artifact.saved")
        .unwrap();
    let artifact_id = artifact["artifact_refs"][0].as_str().unwrap();
    let (status, record) = call(
        &state,
        &app,
        "GET",
        &format!("/artifacts/{artifact_id}/metadata"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{record}");
    assert_eq!(record["artifact_id"], artifact_id);
    assert_eq!(record["version"], artifact["data"]["version"]);
    assert!(events.iter().any(|e| e["event_type"] == "handoff.saved"));
    let tasks = events
        .iter()
        .filter(|e| e["event_type"].as_str().unwrap().starts_with("task."))
        .collect::<Vec<_>>();
    for kind in ["task.created", "task.running", "task.succeeded"] {
        assert!(
            tasks.iter().any(|e| e["event_type"] == kind),
            "{kind}: {tasks:?}"
        );
    }
    for task in &tasks {
        if let Some(cause) = task["causation_id"].as_str() {
            let prior = tasks
                .iter()
                .find(|e| e["event_id"] == cause)
                .expect("task cause exists");
            assert_eq!(prior["task_id"], task["task_id"]);
            assert!(prior["seq"].as_u64() < task["seq"].as_u64());
        }
    }

    let cut = events[1]["seq"].as_u64().unwrap();
    drop(app);
    drop(state);

    // New AppState, coordinator, audit buffer and SQLite connections on the same persisted root.
    let workspace = temp.path().join("ws");
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        SqliteSessionStore::open(&workspace.join("index.db")).unwrap(),
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    let app = build_router(Arc::clone(&state));
    let (status, page) = call(
        &state,
        &app,
        "GET",
        &format!("/teams/{id}/timeline?after={cut}&limit=256"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["events"], json!(events[2..]));
    assert_eq!(page["next_after"], after);
    drop(app);
    drop(state);
    drop(temp);
}

#[tokio::test]
async fn timeline_rejects_unknown_foreign_and_ambiguous_cursors_before_streaming() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let id = completed_team(&state, &app).await;
    let other = completed_team(&state, &app).await;
    let (_, page) = call(
        &state,
        &app,
        "GET",
        &format!("/teams/{id}/timeline?limit=1"),
        None,
    )
    .await;
    let first = page["events"][0]["seq"].as_u64().unwrap();
    let event_id = page["events"][0]["event_id"].as_str().unwrap();
    for path in [
        format!("/teams/{other}/timeline?after={first}"),
        format!("/teams/{id}/timeline?after=9223372036854775807&format=sse"),
        format!("/teams/{id}/timeline?limit=0"),
        format!("/teams/{id}/timeline?limit=257"),
        format!("/teams/{id}/timeline?format=other"),
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, "GET", &path, None))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 400, "{path}");
        assert!(!response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("text/event-stream"));
    }
    let mut req = request(
        &state,
        "GET",
        &format!("/teams/{id}/timeline?format=sse&after=0"),
        None,
    );
    req.headers_mut()
        .insert("last-event-id", event_id.parse().unwrap());
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status().as_u16(),
        400
    );
    let (status, _) = call(
        &state,
        &app,
        "GET",
        "/teams/absent/timeline?format=sse",
        None,
    )
    .await;
    assert_eq!(status, 404);
    let req = axum::http::Request::builder()
        .uri(format!("/teams/{id}/timeline"))
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status().as_u16(),
        401
    );
    drop(app);
    drop(state);
    drop(temp);
}

fn frames(text: &str) -> Vec<Value> {
    text.split("\n\n")
        .filter_map(|frame| {
            if !frame.lines().any(|line| line == "event: run_event") {
                return None;
            }
            let id = frame
                .lines()
                .find_map(|line| line.strip_prefix("id: "))
                .expect("stable SSE id");
            let data = frame
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .expect("SSE JSON");
            let record: Value = serde_json::from_str(data).unwrap();
            assert_eq!(record["event_id"], id);
            Some(record)
        })
        .collect()
}

#[tokio::test]
async fn live_sse_drains_terminal_event_and_header_resume_is_exclusive() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let create = json!({"strategy":"team","objective":"live timeline","roles":[
        {"role":"runner","assignee":"agent","worker":"sleep","extra_input":{"ms":800},"verify":"non_empty"}
    ]});
    let (status, created) = call(&state, &app, "POST", "/teams", Some(&create.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/teams/{id}/timeline?format=sse&limit=1"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let records = frames(&text);
    assert!(text.contains("event: timeline\n"));
    assert_eq!(records.first().unwrap()["event_type"], "team.created");
    assert_eq!(records.last().unwrap()["data"]["status"], "succeeded");
    let (_, page) = call(
        &state,
        &app,
        "GET",
        &format!("/teams/{id}/timeline?limit=256"),
        None,
    )
    .await;
    assert_eq!(page["events"], json!(records));

    let mut req = request(
        &state,
        "GET",
        &format!("/teams/{id}/timeline?format=sse"),
        None,
    );
    req.headers_mut().insert(
        "last-event-id",
        records[0]["event_id"].as_str().unwrap().parse().unwrap(),
    );
    let response = app.clone().oneshot(req).await.unwrap();
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(frames(std::str::from_utf8(&bytes).unwrap()), records[1..]);
    drop(app);
    drop(state);
    drop(temp);
}

struct EventModel(std::sync::atomic::AtomicUsize);
impl EventModel {
    fn next(&self) -> owo_agent_core::gateway::ModelOutput {
        use owo_agent_core::gateway::{ModelOutput, ToolCall};
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            ModelOutput::ToolCalls(vec![ToolCall {
                id: "read-1".into(),
                name: "read_file".into(),
                arguments: json!({"path":"source.txt"}),
            }])
        } else {
            ModelOutput::Text(
                json!({"status":"done","summary":"Read local evidence",
                "artifact":{"kind":"document","format":"markdown","content":"local evidence"},
                "evidence":[],"open_issues":[]})
                .to_string(),
            )
        }
    }
}
#[async_trait::async_trait]
impl ModelProvider for EventModel {
    async fn complete(
        &self,
        _: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Ok(self.next())
    }
    async fn complete_stream_with_model(
        &self,
        _: Option<&str>,
        _: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        let output = self.next();
        if let owo_agent_core::ModelOutput::Text(text) = &output {
            on_delta(text.clone());
        }
        Ok(output)
    }
}

#[tokio::test]
async fn agent_worker_streams_real_tool_and_message_references_into_timeline() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("source.txt"), "local evidence").unwrap();
    let agent = Agent::new(
        Arc::new(EventModel(std::sync::atomic::AtomicUsize::new(0))),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        SqliteSessionStore::open(&workspace.join("index.db")).unwrap(),
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    let app = build_router(Arc::clone(&state));
    let create = json!({"strategy":"team","objective":"read source and return document",
        "roles":[{"role":"reader","node_id":"node-reader","assignee":"agent","worker":"agent",
        "tools":["read_file"],"verify":"non_empty"}]});
    let (status, created) = call(&state, &app, "POST", "/teams", Some(&create.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap();
    let detail = poll_team_status(
        &state,
        &app,
        id,
        &["succeeded", "failed"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["team"]["status"], "succeeded", "{detail}");
    let (status, page) = call(
        &state,
        &app,
        "GET",
        &format!("/teams/{id}/timeline?limit=256"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{page}");
    let records = page["events"].as_array().unwrap();
    let start = records
        .iter()
        .find(|e| e["event_type"] == "tool.started")
        .expect("actual tool start");
    let finish = records
        .iter()
        .find(|e| e["event_type"] == "tool.finished")
        .expect("actual tool result");
    assert_eq!(start["tool_call_id"], "read-1");
    assert_eq!(start["node_id"], "node-reader");
    assert!(start["task_id"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(start["attempt"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(finish["causation_id"], start["event_id"]);
    assert_eq!(finish["data"]["payload_preview"]["ok"], true, "{finish}");
    assert!(
        finish["data"]["payload_preview"]
            .to_string()
            .contains("local evidence"),
        "{finish}"
    );
    let final_message = records
        .iter()
        .find(|e| e["event_type"] == "message.final")
        .unwrap();
    assert!(final_message["data"]["record_ref"]["seq"]
        .as_u64()
        .is_some());
    assert!(records.iter().any(|e| e["event_type"] == "message.delta"));
    assert!(state
        .store
        .pending_team_turn_events(id, 256)
        .unwrap()
        .is_empty());
    drop(app);
    drop(state);
    drop(temp);
}

fn existing_state(temp: &std::path::Path) -> Arc<owo_agent_server::AppState> {
    let workspace = temp.join("ws");
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    Arc::new(owo_agent_server::AppState::new(
        agent,
        SqliteSessionStore::open(&workspace.join("index.db")).unwrap(),
        workspace.join("traces"),
        temp.to_path_buf(),
        workspace,
    ))
}

#[tokio::test]
async fn pending_public_messages_recover_after_restart_in_bounded_pages_and_keep_private_source_out(
) {
    use owo_agent_core::session::TeamTurnBinding;
    use owo_agent_protocol::SseEvent;
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let id = completed_team(&state, &app).await;
    let binding = TeamTurnBinding {
        run_id: id.clone(),
        node_id: Some("fixture-node".into()),
        task_id: "fixture-task".into(),
        attempt: Some("fixture-attempt".into()),
    };
    let session = format!("{id}-worker-recovery");
    for i in 0..130 {
        state
            .store
            .append_team_turn_event(
                &binding,
                &session,
                "fixture-role",
                &SseEvent::TokenDelta {
                    delta: format!("chunk {i}"),
                },
            )
            .unwrap();
    }
    state
        .store
        .append_turn_event(
            &session,
            "fixture-role",
            &SseEvent::ReasoningDelta {
                delta: "PRIVATE_NOT_A_PUBLIC_EVENT".into(),
            },
        )
        .unwrap();
    let large = state
        .store
        .append_team_turn_event(
            &binding,
            &session,
            "fixture-role",
            &SseEvent::Final {
                text: "图".repeat(5000),
            },
        )
        .unwrap();
    drop(app);
    drop(state);
    let state = existing_state(temp.path());
    let app = build_router(Arc::clone(&state));
    let mut after = 0;
    let mut messages = Vec::new();
    loop {
        let (status, page) = call(
            &state,
            &app,
            "GET",
            &format!("/teams/{id}/timeline?after={after}&limit=64"),
            None,
        )
        .await;
        assert_eq!(status, 200, "{page}");
        assert!(!page.to_string().contains("PRIVATE_NOT_A_PUBLIC_EVENT"));
        let events = page["events"].as_array().unwrap();
        assert!(events.len() <= 64);
        messages.extend(
            events
                .iter()
                .filter(|e| e["event_type"].as_str().unwrap().starts_with("message."))
                .cloned(),
        );
        let next = page["next_after"].as_u64().unwrap();
        assert!(next >= after);
        after = next;
        if page["has_more"] == false {
            break;
        }
    }
    assert_eq!(messages.len(), 131);
    assert_eq!(messages[0]["data"]["payload_preview"]["delta"], "chunk 0");
    assert_eq!(messages.last().unwrap()["data"]["payload_complete"], false);
    assert_eq!(
        messages.last().unwrap()["data"]["payload_unavailable"],
        true
    );
    assert_eq!(
        messages.last().unwrap()["data"]["record_ref"]["seq"],
        large.seq
    );
    assert!(state
        .store
        .turn_event_json_bounded(&session, large.seq, 20000)
        .unwrap()
        .is_some());
    assert!(state
        .store
        .pending_team_turn_events(&id, 256)
        .unwrap()
        .is_empty());
    drop(app);
    drop(state);
    drop(temp);
}

struct DeliveryFaultStore {
    inner: Arc<dyn owo_agent_core::SessionStore>,
    reject_append: bool,
    reject_ack: bool,
}
impl owo_agent_core::SessionStore for DeliveryFaultStore {
    fn create(
        &self,
        w: &std::path::Path,
        m: &str,
        p: Option<&str>,
    ) -> Result<owo_agent_core::Session, owo_agent_core::AgentError> {
        self.inner.create(w, m, p)
    }
    fn load(&self, id: &str) -> Result<owo_agent_core::Session, owo_agent_core::AgentError> {
        self.inner.load(id)
    }
    fn save(&self, s: &owo_agent_core::Session) -> Result<(), owo_agent_core::AgentError> {
        self.inner.save(s)
    }
    fn list(&self) -> Vec<String> {
        self.inner.list()
    }
    fn exists(&self, id: &str) -> Result<bool, owo_agent_core::AgentError> {
        self.inner.exists(id)
    }
    fn append_team_turn_event(
        &self,
        b: &owo_agent_core::session::TeamTurnBinding,
        s: &str,
        t: &str,
        p: &owo_agent_protocol::SseEvent,
    ) -> Result<owo_agent_protocol::TurnEventRecord, owo_agent_core::AgentError> {
        if self.reject_append {
            return Err(owo_agent_core::AgentError::Session(
                "injected canonical write failure".into(),
            ));
        }
        self.inner.append_team_turn_event(b, s, t, p)
    }
    fn pending_team_turn_events(
        &self,
        r: &str,
        n: usize,
    ) -> Result<Vec<owo_agent_core::session::TeamTurnDelivery>, owo_agent_core::AgentError> {
        self.inner.pending_team_turn_events(r, n)
    }
    fn acknowledge_team_turn_event(
        &self,
        r: &str,
        id: i64,
    ) -> Result<(), owo_agent_core::AgentError> {
        if self.reject_ack {
            return Err(owo_agent_core::AgentError::Session(
                "injected acknowledgement failure".into(),
            ));
        }
        self.inner.acknowledge_team_turn_event(r, id)
    }
    fn turn_event_json_bounded(
        &self,
        s: &str,
        seq: u64,
        n: usize,
    ) -> Result<Option<String>, owo_agent_core::AgentError> {
        self.inner.turn_event_json_bounded(s, seq, n)
    }
    fn append_turn_event(
        &self,
        s: &str,
        t: &str,
        p: &owo_agent_protocol::SseEvent,
    ) -> Result<owo_agent_protocol::TurnEventRecord, owo_agent_core::AgentError> {
        self.inner.append_turn_event(s, t, p)
    }
    fn turn_events_after(
        &self,
        s: &str,
        t: Option<&str>,
        seq: u64,
        n: usize,
    ) -> Result<Vec<owo_agent_protocol::TurnEventRecord>, owo_agent_core::AgentError> {
        self.inner.turn_events_after(s, t, seq, n)
    }
}

#[tokio::test]
async fn journal_commit_before_ack_failure_recovers_once_after_restart() {
    use owo_agent_core::session::TeamTurnBinding;
    let (mut state, temp) = test_state().await;
    let inner = Arc::clone(&state.store);
    Arc::get_mut(&mut state).unwrap().store = Arc::new(DeliveryFaultStore {
        inner,
        reject_append: false,
        reject_ack: true,
    });
    let app = build_router(Arc::clone(&state));
    let id = completed_team(&state, &app).await;
    let binding = TeamTurnBinding {
        run_id: id.clone(),
        node_id: None,
        task_id: "task".into(),
        attempt: Some("attempt".into()),
    };
    state
        .store
        .append_team_turn_event(
            &binding,
            &format!("{id}-worker-ack"),
            "role",
            &owo_agent_protocol::SseEvent::Final {
                text: "recover me".into(),
            },
        )
        .unwrap();
    let (status, _) = call(&state, &app, "GET", &format!("/teams/{id}/timeline"), None).await;
    assert_eq!(status, 500);
    assert_eq!(
        state.store.pending_team_turn_events(&id, 10).unwrap().len(),
        1
    );
    let page = state
        .workswarm
        .coordinator()
        .unwrap()
        .run_events_after(&id, 0, 256)
        .unwrap();
    let before = page
        .events
        .iter()
        .find(|e| e.event_type == "message.final")
        .unwrap()
        .event_id
        .clone();
    drop(app);
    drop(state);
    let state = existing_state(temp.path());
    let app = build_router(Arc::clone(&state));
    let (status, page) = call(
        &state,
        &app,
        "GET",
        &format!("/teams/{id}/timeline?limit=256"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{page}");
    let messages = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event_type"] == "message.final")
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["event_id"], before);
    assert_eq!(messages[0]["data"]["payload_preview"]["text"], "recover me");
    assert!(state
        .store
        .pending_team_turn_events(&id, 10)
        .unwrap()
        .is_empty());
    drop(app);
    drop(state);
    drop(temp);
}

#[tokio::test]
async fn failed_canonical_worker_replay_cannot_publish_success() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("source.txt"), "local evidence").unwrap();
    let agent = Agent::new(
        Arc::new(EventModel(std::sync::atomic::AtomicUsize::new(0))),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let mut state = Arc::new(owo_agent_server::AppState::new(
        agent,
        SqliteSessionStore::open(&workspace.join("index.db")).unwrap(),
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    let inner = Arc::clone(&state.store);
    Arc::get_mut(&mut state).unwrap().store = Arc::new(DeliveryFaultStore {
        inner,
        reject_append: true,
        reject_ack: false,
    });
    let app = build_router(Arc::clone(&state));
    let create = json!({"strategy":"team","objective":"do not hide replay failure",
        "roles":[{"role":"reader","assignee":"agent","worker":"agent","tools":["read_file"],"verify":"non_empty"}]});
    let (status, created) = call(&state, &app, "POST", "/teams", Some(&create.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap();
    let detail = poll_team_status(
        &state,
        &app,
        id,
        &["failed", "succeeded"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["team"]["status"], "failed", "{detail}");
    assert!(
        detail
            .to_string()
            .contains("injected canonical write failure"),
        "{detail}"
    );
    drop(app);
    drop(state);
    drop(temp);
}

struct HeldFinalModel {
    model: EventModel,
    release: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl ModelProvider for HeldFinalModel {
    async fn complete(
        &self,
        _: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        let output = self.model.next();
        if matches!(output, owo_agent_core::ModelOutput::Text(_)) {
            self.release.notified().await;
        }
        Ok(output)
    }
    async fn complete_stream_with_model(
        &self,
        _: Option<&str>,
        messages: &[owo_agent_core::ChatMessage],
        tools: &[owo_agent_core::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        let output = self.complete(messages, tools).await?;
        if let owo_agent_core::ModelOutput::Text(text) = &output {
            on_delta(text.clone());
        }
        Ok(output)
    }
}

#[tokio::test]
async fn agent_tool_result_arrives_over_live_sse_before_worker_completion() {
    use futures_util::StreamExt;
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::write(workspace.join("source.txt"), "local evidence").unwrap();
    let release = Arc::new(tokio::sync::Notify::new());
    let agent = Agent::new(
        Arc::new(HeldFinalModel {
            model: EventModel(std::sync::atomic::AtomicUsize::new(0)),
            release: Arc::clone(&release),
        }),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        SqliteSessionStore::open(&workspace.join("index.db")).unwrap(),
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    let app = build_router(Arc::clone(&state));
    let create = json!({"strategy":"team","objective":"observe the actual tool before final output",
        "roles":[{"role":"reader","node_id":"live-reader","assignee":"agent","worker":"agent",
            "tools":["read_file"],"verify":"non_empty"}]});
    let (status, created) = call(&state, &app, "POST", "/teams", Some(&create.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/teams/{id}/timeline?format=sse"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut stream = response.into_body().into_data_stream();
    let mut bytes = Vec::new();
    let observed = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
            let text = std::str::from_utf8(&bytes).unwrap();
            if text.ends_with("\n\n")
                && frames(text)
                    .iter()
                    .any(|e| e["event_type"] == "tool.finished")
            {
                return frames(text);
            }
        }
        panic!("stream closed before an actual tool result");
    })
    .await;
    // Always release the worker before propagating a timeout assertion.
    release.notify_one();
    let early = observed.expect("actual tool result must arrive while final model output is held");
    assert!(!early.iter().any(|e| e["event_type"] == "message.final"));
    assert!(!early
        .iter()
        .any(|e| e["event_type"] == "team.status" && e["data"]["status"] == "succeeded"));
    let tool = early
        .iter()
        .find(|e| e["event_type"] == "tool.finished")
        .unwrap();
    assert_eq!(tool["node_id"], "live-reader");
    assert_eq!(tool["data"]["payload_preview"]["ok"], true);
    assert!(tool["data"]["payload_preview"]["preview"]
        .as_str()
        .unwrap()
        .contains("local evidence"));
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.unwrap());
        }
    })
    .await
    .expect("terminal history must drain and close");
    let records = frames(std::str::from_utf8(&bytes).unwrap());
    assert!(records.iter().any(|e| e["event_type"] == "message.final"));
    assert_eq!(records.last().unwrap()["data"]["status"], "succeeded");
    let ids = records
        .iter()
        .map(|e| e["event_id"].as_str().unwrap())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), records.len());
    let (status, page) = call(
        &state,
        &app,
        "GET",
        &format!("/teams/{id}/timeline?limit=256"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(page["events"], json!(records));
    drop(stream);
    drop(app);
    drop(state);
    drop(temp);
}

async fn task_page(state: &Arc<owo_agent_server::AppState>, app: &axum::Router, id: &str) -> Value {
    let (status, page) = call(
        state,
        app,
        "GET",
        &format!("/teams/{id}/timeline?limit=256"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{page}");
    assert_eq!(page["task_history_complete"], true);
    assert_eq!(page["has_more"], false);
    page
}

#[tokio::test]
async fn task_timeline_reports_human_dependencies_and_committed_result() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let body = json!({"strategy":"team","objective":"human observation","roles":[
        {"role":"planner","node_id":"plan-node","assignee":"agent","worker":"echo","verify":"non_empty"},
        {"role":"approver","node_id":"human-node","assignee":"human","worker":"u-1","depends_on":["planner"],"verify":"non_empty"}
    ]});
    let (status, created) = call(&state, &app, "POST", "/teams", Some(&body.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap();
    let detail = poll_team_status(
        &state,
        &app,
        id,
        &["awaiting_human", "failed"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["team"]["status"], "awaiting_human", "{detail}");
    let page = task_page(&state, &app, id).await;
    let tasks = page["events"].as_array().unwrap();
    let initial = tasks
        .iter()
        .find(|e| e["task_id"] == "s-approver" && e["event_type"] == "task.created")
        .unwrap();
    assert_eq!(initial["data"]["task"]["waiting_reason"], "dependencies");
    assert_eq!(initial["data"]["task"]["waiting_for"], json!(["s-planner"]));
    let waiting = tasks
        .iter()
        .rev()
        .find(|e| e["task_id"] == "s-approver")
        .unwrap();
    assert_eq!(waiting["node_id"], "human-node");
    assert_eq!(waiting["data"]["task"]["waiting_reason"], "human_input");
    assert_eq!(waiting["data"]["task"]["assignee"], "human");
    let (status, result) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{id}/steer"),
        Some(
            &json!({"command":"steer","step_id":"s-approver",
            "new_input":{"assigned_task_id":"approval-reassigned"},"note":"reassign task binding"})
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    let rebound_page = task_page(&state, &app, id).await;
    let rebound = rebound_page["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["task_id"] == "approval-reassigned")
        .expect("changed task binding remains observable");
    assert_eq!(rebound["causation_id"], waiting["event_id"]);
    assert_eq!(rebound["data"]["previous_task_id"], "s-approver");

    let (status, result) = call(
        &state,
        &app,
        "POST",
        "/tasks/s-approver/human-result",
        Some(&json!({"team_id":id,"result":"approved"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    let detail = poll_team_status(
        &state,
        &app,
        id,
        &["succeeded", "failed"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["team"]["status"], "succeeded", "{detail}");
    let final_page = task_page(&state, &app, id).await;
    let tasks = final_page["events"].as_array().unwrap();
    let success = tasks
        .iter()
        .find(|e| e["task_id"] == "approval-reassigned" && e["event_type"] == "task.succeeded")
        .unwrap();
    assert_eq!(success["node_id"], "human-node");
    let admitted = tasks
        .iter()
        .find(|e| e["event_id"] == success["causation_id"])
        .expect("human result retains its committed attempt admission");
    assert_eq!(admitted["event_type"], "task.attempt_changed");
    assert_eq!(admitted["task_id"], "approval-reassigned");
    assert_eq!(admitted["causation_id"], rebound["event_id"]);
    assert_eq!(admitted["attempt"], success["attempt"]);
    assert!(success["data"]["task"]["waiting_reason"].is_null());
    drop(app);
    drop(state);
    drop(temp);
}

#[tokio::test]
async fn task_timeline_keeps_cancel_reset_and_new_attempt_in_one_real_causal_chain() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let body = json!({"strategy":"team","objective":"cancel retry observation","roles":[
        {"role":"runner","node_id":"run-node","assignee":"agent","worker":"sleep","extra_input":{"ms":2500},"verify":"non_empty"}
    ]});
    let (status, created) = call(&state, &app, "POST", "/teams", Some(&body.to_string())).await;
    assert_eq!(status, 202, "{created}");
    let id = created["team_id"].as_str().unwrap();
    let first_attempt = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let page = task_page(&state, &app, id).await;
            if let Some(event) = page["events"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["event_type"] == "task.running")
            {
                break event["attempt"]
                    .as_str()
                    .expect("host-owned attempt")
                    .to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("admitted running task before cancellation");
    let (status, result) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{id}/steer"),
        Some(&json!({"command":"cancel","note":"cancel test"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    let detail = poll_team_status(
        &state,
        &app,
        id,
        &["cancelled", "failed"],
        std::time::Duration::from_secs(20),
    )
    .await;
    assert_eq!(detail["team"]["status"], "cancelled", "{detail}");
    let page = task_page(&state, &app, id).await;
    let aborted = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event_type"] == "task.aborted")
        .expect("committed cancellation");
    assert_eq!(aborted["attempt"], first_attempt);
    let coordinator = state.workswarm.coordinator().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while coordinator.is_run_active(id) || coordinator.is_loop_alive(id) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("cancelled execution and its loop must release ownership before retry");

    let (status, result) = call(
        &state,
        &app,
        "POST",
        &format!("/teams/{id}/steer"),
        Some(&json!({"command":"retry","step_id":"s-runner","note":"retry test"}).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{result}");
    let detail = poll_team_status(
        &state,
        &app,
        id,
        &["succeeded", "failed"],
        std::time::Duration::from_secs(25),
    )
    .await;
    assert_eq!(detail["team"]["status"], "succeeded", "{detail}");
    let page = task_page(&state, &app, id).await;
    let tasks = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event_type"].as_str().unwrap().starts_with("task."))
        .collect::<Vec<_>>();
    let reset = tasks
        .iter()
        .find(|e| e["event_type"] == "task.pending" && e["causation_id"] == aborted["event_id"])
        .expect("retry reset retains previous event cause");
    assert!(reset["attempt"].is_null());
    let running = tasks
        .iter()
        .filter(|e| e["event_type"] == "task.running")
        .collect::<Vec<_>>();
    assert_eq!(running.len(), 2, "{tasks:?}");
    assert_ne!(running[0]["attempt"], running[1]["attempt"]);
    assert_eq!(running[1]["node_id"], "run-node");
    assert!(tasks
        .iter()
        .any(|e| e["event_type"] == "task.succeeded" && e["attempt"] == running[1]["attempt"]));
    for task in &tasks {
        if let Some(cause) = task["causation_id"].as_str() {
            let prior = tasks
                .iter()
                .find(|e| e["event_id"] == cause)
                .expect("same task's real preceding event");
            assert_eq!(prior["task_id"], task["task_id"]);
            assert!(prior["seq"].as_u64() < task["seq"].as_u64());
        }
    }
    drop(app);
    drop(state);
    drop(temp);
}

#[tokio::test]
async fn reverse_timeline_pages_reject_ambiguous_inputs_and_match_forward_history() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let id = completed_team(&state, &app).await;
    let mut forward = Vec::new();
    let mut after = 0;
    loop {
        let (status, page) = call(
            &state,
            &app,
            "GET",
            &format!("/teams/{id}/timeline?after={after}&limit=3"),
            None,
        )
        .await;
        assert_eq!(status, 200, "{page}");
        forward.extend(page["events"].as_array().unwrap().iter().cloned());
        after = page["next_after"].as_u64().unwrap();
        if page["has_more"] == false {
            break;
        }
    }
    let mut before = 0;
    let mut reverse = Vec::new();
    loop {
        let (status, page) = call(
            &state,
            &app,
            "GET",
            &format!("/teams/{id}/timeline?before={before}&limit=3"),
            None,
        )
        .await;
        assert_eq!(status, 200, "{page}");
        assert!(page.get("next_after").is_none());
        assert_eq!(page["task_history_complete"], true);
        let rows = page["events"].as_array().unwrap();
        assert!(rows.len() <= 3);
        assert!(rows
            .windows(2)
            .all(|pair| pair[0]["seq"].as_u64() < pair[1]["seq"].as_u64()));
        reverse.push(rows.clone());
        before = page["next_before"].as_u64().unwrap();
        if page["has_more"] == false {
            break;
        }
    }
    reverse.reverse();
    assert_eq!(reverse.into_iter().flatten().collect::<Vec<_>>(), forward);
    for query in [
        "before=0&after=0",
        "before=0&format=sse",
        "before=18446744073709551615",
        "before=999999999",
        "before=0&limit=257",
    ] {
        let (status, body) = call(
            &state,
            &app,
            "GET",
            &format!("/teams/{id}/timeline?{query}"),
            None,
        )
        .await;
        assert_eq!(status, 400, "{query}: {body}");
    }
}

//! 会话与任务状态机测试（E1.3 验收）。
//!
//! 覆盖协议最小接入验证第 12~15 条与 mock `process()` 的关键语义：
//! 幂等缓存、槽位补丁（revision/锁定）、cancel 清理、state_revision 单调性。

use std::collections::HashMap;
use std::time::Duration;

use owo_intent_gateway::commands;
use owo_intent_gateway::protocol::*;
use owo_intent_gateway::state::{CommandAction, ImeState, StateAction};

const SESSION: &str = "0123456789abcdef0123456789abcdef";

fn request(action: Action, request_id: &str) -> AgentIpcRequest {
    AgentIpcRequest {
        schema_version: AGENT_PROTOCOL_VERSION,
        action,
        session_id: SESSION.to_string(),
        request_id: request_id.to_string(),
        parent_request_id: String::new(),
        idempotency_key: request_id.to_string(),
        capabilities: SUPPORTED_CAPABILITIES
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
        protocol_min: 2,
        protocol_max: 3,
        required_features: vec![
            "protocol.negotiation".to_string(),
            "input.structured".to_string(),
            "commands.risk".to_string(),
            "tasks.slots".to_string(),
        ],
        user_input: "bangwozhaowenjian".to_string(),
        input: InputView {
            selected_text: "改写你好".into(),
            ..Default::default()
        },
        application: ApplicationView::default(),
        session_context: String::new(),
        context_entries: Vec::new(),
        selection: None,
        command_id: String::new(),
        page: 0,
        task_revision: 0,
        slot_updates: Vec::new(),
    }
}

fn expect_respond(action: StateAction) -> AgentIpcResponse {
    match action {
        StateAction::Respond(response) => response,
        _ => panic!("期望 Respond，得到其它 StateAction"),
    }
}

fn expect_start_turn(
    state: &ImeState,
    request: &AgentIpcRequest,
) -> (
    AgentIpcResponse,
    owo_intent_gateway::state::StartTurnRequest,
) {
    let draft = expect_respond(state.dispatch(request));
    let mut commit = request.clone();
    commit.action = Action::Select;
    commit.command_id = "task-commit".into();
    commit.task_revision = draft.task.revision;
    commit.idempotency_key = format!("{}-commit", request.idempotency_key);
    match state.dispatch(&commit) {
        StateAction::StartTurn { immediate, turn } => (immediate, *turn),
        _ => panic!("期望 StartTurn，得到其它 StateAction"),
    }
}

fn slot_draft() -> TaskDraft {
    TaskDraft {
        intent: "create_reminder".to_string(),
        intent_ranges: vec![PinyinRange { start: 8, end: 16 }],
        slots: vec![
            TaskSlot {
                id: "time".to_string(),
                label: "时间".to_string(),
                value: "三点钟".to_string(),
                source_ranges: vec![PinyinRange { start: 0, end: 8 }],
                locked: false,
                confidence_milli: 650,
            },
            TaskSlot {
                id: "content".to_string(),
                label: "事项".to_string(),
                value: "开会".to_string(),
                source_ranges: vec![PinyinRange { start: 16, end: 22 }],
                locked: true,
                confidence_milli: 980,
            },
        ],
        unconsumed_ranges: Vec::new(),
        revision: 1,
    }
}

#[test]
fn submit_creates_session_and_starts_turn() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (immediate, turn) = expect_start_turn(&state, &req);

    assert_eq!(immediate.status, Status::Thinking);
    assert_eq!(immediate.state_revision, 2);
    assert!(immediate.can_cancel);
    assert!(immediate.retry_after_ms > 0);
    assert_eq!(immediate.capabilities, req.negotiated_capabilities());
    assert_eq!(turn.session_id, SESSION);
    assert!(state.has_session(SESSION));
}

#[test]
fn duplicate_submit_while_pending_does_not_restart() {
    let state = ImeState::new(Duration::from_secs(60));
    let first = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &first);

    // 不同幂等键、同会话的再次 submit：应返回观察态而非新 turn。
    let mut second = request(Action::Submit, "req-2");
    second.idempotency_key = "req-2".to_string();
    match state.dispatch(&second) {
        StateAction::Respond(response) => {
            assert_eq!(response.status, Status::Thinking);
            assert_eq!(response.state_revision, 2, "观察态不得推进状态版本");
        }
        _ => panic!("挂起中的重复 submit 不得启动新 turn"),
    }
}

#[test]
fn completed_expression_then_poll_returns_final() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);

    let mut final_response = commands::base_response(&req, Status::AgentMode);
    final_response.message = "已找到 3 个文件".to_string();
    let completed = state
        .complete_expression(SESSION, &_turn.slot, final_response, HashMap::new())
        .expect("会话存在时必须写回结果");
    assert_eq!(completed.state_revision, 3, "完成回调推进状态版本");

    let poll = request(Action::Poll, "req-3");
    let response = expect_respond(state.dispatch(&poll));
    assert_eq!(response.status, Status::AgentMode);
    assert_eq!(response.message, "已找到 3 个文件");
    assert_eq!(response.request_id, "req-3", "request_id 必须回填当前请求");
    assert_eq!(response.state_revision, 3);

    // 结果只取一次：再 poll 回到空闲态。
    let poll_again = request(Action::Poll, "req-4");
    let idle = expect_respond(state.dispatch(&poll_again));
    assert_eq!(idle.status, Status::AgentMode);
    assert_eq!(idle.message, "当前没有执行中的任务");
}

#[test]
fn idempotency_cache_returns_same_response() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);
    let final_response = commands::base_response(&req, Status::Completed);
    state
        .complete_expression(SESSION, &_turn.slot, final_response, HashMap::new())
        .unwrap();

    // 相同 idempotency_key 重发（模拟连接器重试）：命中缓存。
    let mut retry = req.clone();
    retry.action = Action::Select;
    retry.command_id = "task-commit".into();
    retry.task_revision = 1;
    retry.request_id = "req-99".into();
    retry.idempotency_key = "req-1-commit".to_string();
    let response = expect_respond(state.dispatch(&retry));
    assert_eq!(response.status, Status::Completed);
    assert_eq!(
        response.request_id, "req-99",
        "缓存响应也要回填当前 request_id"
    );
}

#[tokio::test]
async fn cancel_signals_pending_and_removes_session() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, turn) = expect_start_turn(&state, &req);

    let cancel = request(Action::Cancel, "req-2");
    let response = expect_respond(state.dispatch(&cancel));
    assert_eq!(response.status, Status::Cancelled);
    assert_eq!(response.state_revision, 1);
    assert!(!state.has_session(SESSION), "cancel 后会话必须清理");

    // bridge 侧必须收到取消信号（permit 已存储，即使稍后才等待）。
    tokio::time::timeout(Duration::from_millis(200), turn.slot.cancelled())
        .await
        .expect("取消信号不得丢失");
}

#[test]
fn cancel_without_session_is_lenient() {
    let state = ImeState::new(Duration::from_secs(60));
    let response = expect_respond(state.dispatch(&request(Action::Cancel, "req-1")));
    assert_eq!(response.status, Status::Cancelled);
}

#[test]
fn slot_patch_applies_and_bumps_task_revision() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);
    state.set_task(SESSION, slot_draft());

    let mut select = request(Action::Select, "req-2");
    select.command_id = String::new();
    select.task_revision = 1;
    select.slot_updates = vec![SlotUpdate {
        slot_id: "time".to_string(),
        value: "明天3点".to_string(),
        consumed_ranges: vec![PinyinRange { start: 0, end: 8 }],
        lock: true,
    }];
    let response = expect_respond(state.dispatch(&select));
    assert_eq!(response.status, Status::AgentMode);
    assert_eq!(response.task.revision, 2);
    let time_slot = response
        .task
        .slots
        .iter()
        .find(|slot| slot.id == "time")
        .unwrap();
    assert_eq!(time_slot.value, "明天3点");
    assert!(time_slot.locked);
    assert_eq!(time_slot.confidence_milli, 1000);
    assert_eq!(response.commands.len(), 1);
    assert!(response.commands[0].commit_task);
}

#[test]
fn slot_patch_revision_mismatch_rejected() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);
    state.set_task(SESSION, slot_draft());

    let mut select = request(Action::Select, "req-2");
    select.task_revision = 99;
    select.slot_updates = vec![SlotUpdate {
        slot_id: "time".to_string(),
        value: "明天3点".to_string(),
        consumed_ranges: vec![PinyinRange { start: 0, end: 8 }],
        lock: true,
    }];
    let response = expect_respond(state.dispatch(&select));
    assert_eq!(response.status, Status::Error);
    assert_eq!(response.error_code, "task_revision_mismatch");
    assert_eq!(response.task.revision, 1, "错误响应必须回带当前草稿");
}

#[test]
fn slot_patch_locked_conflict_rejected() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);
    state.set_task(SESSION, slot_draft());

    let mut select = request(Action::Select, "req-2");
    select.task_revision = 1;
    select.slot_updates = vec![SlotUpdate {
        slot_id: "content".to_string(),
        value: "看电影".to_string(),
        consumed_ranges: vec![PinyinRange { start: 16, end: 22 }],
        lock: true,
    }];
    let response = expect_respond(state.dispatch(&select));
    assert_eq!(response.error_code, "slot_locked");
}

#[test]
fn select_unknown_command_rejected() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);

    let mut select = request(Action::Select, "req-2");
    select.command_id = "no-such-command".to_string();
    let response = expect_respond(state.dispatch(&select));
    assert_eq!(response.error_code, "unknown_command");
}

#[test]
fn page_keeps_session_and_updates_page() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);

    let mut page = request(Action::Page, "req-2");
    page.page = 2;
    let response = expect_respond(state.dispatch(&page));
    assert_eq!(response.page, 2);
    assert!(state.has_session(SESSION), "翻页不得结束会话");
}

#[test]
fn select_insert_reply_completes_with_text() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);
    state.set_commands(
        SESSION,
        vec![commands::command(
            "insert-reply",
            "回显",
            "text.insert",
            RiskLevel::Low,
        )],
        HashMap::from([(
            "insert-reply".to_string(),
            CommandAction::InsertReply {
                text: "你好，我是 OwO".to_string(),
            },
        )]),
    );

    let mut select = request(Action::Select, "req-2");
    select.command_id = "insert-reply".to_string();
    let response = expect_respond(state.dispatch(&select));
    assert_eq!(response.status, Status::Completed);
    assert_eq!(response.message, "你好，我是 OwO");
}

#[test]
fn select_revert_delegates_to_handler() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(&state, &req);
    state.set_commands(
        SESSION,
        vec![commands::revert_command()],
        HashMap::from([("revert-all".to_string(), CommandAction::RevertAll)]),
    );

    let mut select = request(Action::Select, "req-2");
    select.command_id = "revert-all".to_string();
    match state.dispatch(&select) {
        StateAction::RunCommand { session_id, action } => {
            assert_eq!(session_id, SESSION);
            assert_eq!(action, CommandAction::RevertAll);
        }
        _ => panic!("回滚必须委托 handler 执行"),
    }
}

#[test]
fn unknown_session_returns_not_found() {
    let state = ImeState::new(Duration::from_secs(60));
    let mut poll = request(Action::Poll, "req-1");
    poll.session_id = "ffffffffffffffffffffffffffffffff".to_string();
    let response = expect_respond(state.dispatch(&poll));
    assert_eq!(response.error_code, "session_not_found");
    assert!(response.retryable);
}

#[tokio::test]
async fn gc_removes_expired_sessions() {
    let state = ImeState::new(Duration::from_millis(10));
    let req = request(Action::Submit, "req-1");
    let (_, turn) = expect_start_turn(&state, &req);
    assert_eq!(state.session_count(), 1);

    tokio::time::sleep(Duration::from_millis(30)).await;
    state.gc();
    assert_eq!(state.session_count(), 0, "过期会话必须被 GC");
    // Cancellation must retain its permit even when the HTTP task has not
    // started waiting yet; removing the map entry alone leaves that task alive.
    tokio::time::timeout(Duration::from_millis(100), turn.slot.cancelled())
        .await
        .expect("expired expression must receive cancellation");
    let late = commands::base_response(&req, Status::AgentMode);
    assert!(state
        .complete_expression(SESSION, &turn.slot, late, HashMap::new())
        .is_none());
}

#[test]
fn resubmit_after_cancel_starts_new_turn() {
    let state = ImeState::new(Duration::from_secs(60));
    let (_, _turn) = expect_start_turn(&state, &request(Action::Submit, "req-1"));
    let _ = expect_respond(state.dispatch(&request(Action::Cancel, "req-2")));

    let mut again = request(Action::Submit, "req-3");
    again.idempotency_key = "req-3".to_string();
    let (immediate, _turn) = expect_start_turn(&state, &again);
    assert_eq!(immediate.status, Status::Thinking);
    assert!(state.has_session(SESSION));
}

#[test]
fn sensitive_request_cannot_replay_cached_results() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, turn) = expect_start_turn(&state, &req);
    let mut sensitive = req.clone();
    sensitive.application.sensitive_input = true;
    let response = expect_respond(state.dispatch(&sensitive));
    assert_eq!(response.error_code, "sensitive_input");
    assert!(response.commands.is_empty());
    assert!(!state.has_session(SESSION));
    let mut late = commands::base_response(&req, Status::AgentMode);
    late.message = "late".into();
    assert!(state
        .complete_expression(SESSION, &turn.slot, late, HashMap::new())
        .is_none());
}

#[test]
fn stale_pending_handle_cannot_overwrite_reused_session() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, old) = expect_start_turn(&state, &req);
    let _ = state.dispatch(&request(Action::Cancel, "cancel"));
    let newer = request(Action::Submit, "req-new");
    let (_, new) = expect_start_turn(&state, &newer);
    let late = commands::base_response(&req, Status::Completed);
    assert!(state
        .complete_expression(SESSION, &old.slot, late, HashMap::new())
        .is_none());
    assert!(!new.slot.lock().done);
}

#[test]
fn selection_origin_survives_commit_and_rejects_rebinding_before_cache() {
    let state = ImeState::new(Duration::from_secs(60));
    let mut req = request(Action::Submit, "selected-origin");
    req.input.selected_text = "翻译成英文".into();
    req.application.context_id = "origin-selection".into();
    req.selection = Some(SelectionSummary {
        text: "你好".into(),
        sha256: "a".repeat(64),
        redacted: false,
    });
    let (_, turn) = expect_start_turn(&state, &req);
    assert_eq!(turn.request.selection, req.selection);
    assert_eq!(turn.request.input.selected_text, "翻译成英文");
    assert!(turn.prompt.unwrap().contains("你好"));
    let mut rebound = req.clone();
    rebound.application.context_id = "other-document".into();
    let result = expect_respond(state.dispatch(&rebound));
    assert_eq!(result.error_code, "target_changed");
    assert!(!state.has_session(SESSION));
}

#[test]
fn selected_text_and_style_are_separate_and_produce_replace_effect() {
    let mut req = request(Action::Submit, "selected-style");
    req.input.selected_text = "改写正式一点".into();
    req.application.context_id = "origin-selection".into();
    req.selection = Some(SelectionSummary {
        text: "合成原文".into(),
        sha256: "b".repeat(64),
        redacted: true,
    });
    let task = owo_intent_gateway::intent::parse(&req).unwrap().task;
    let prompt = owo_intent_gateway::intent::expression_prompt(&task).unwrap();
    assert!(prompt.contains("<<<TEXT\n合成原文\nTEXT>>>"));
    assert!(prompt.contains("正式一点"));
    let mut command = commands::insert_reply_command("改写结果");
    commands::attach_commit_effect(&mut command, &req, "改写结果");
    let effect = command.effect.unwrap();
    assert_eq!(effect.kind, EffectKind::ReplaceSelection);
    assert_eq!(effect.selection_sha256, "b".repeat(64));
    assert_eq!(effect.target_context_id, "origin-selection");
}

#[test]
fn cancel_discards_cached_results_before_session_reuse() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "reuse-cancel-cache");
    let draft = expect_respond(state.dispatch(&req));
    let mut cancel = req.clone();
    cancel.action = Action::Cancel;
    cancel.idempotency_key = "cancel-cache".into();
    assert_eq!(
        expect_respond(state.dispatch(&cancel)).status,
        Status::Cancelled
    );
    let mut reused = req.clone();
    reused.input.selected_text = "找文件".into();
    let result = expect_respond(state.dispatch(&reused));
    assert_ne!(result.commands, draft.commands);
    assert_eq!(result.commands[0].id, "open-agent");
}

#[tokio::test]
async fn changed_submission_supersedes_pending_or_unread_expression_and_old_cache() {
    for completed in [false, true] {
        let state = ImeState::new(Duration::from_secs(60));
        let old_req = request(Action::Submit, "old-input");
        let (_, old) = expect_start_turn(&state, &old_req);
        if completed {
            let mut response = commands::base_response(&old_req, Status::AgentMode);
            response.message = "old expression result".into();
            response.commands = vec![commands::insert_reply_command("old expression result")];
            assert!(state
                .complete_expression(SESSION, &old.slot, response, HashMap::new())
                .is_some());
        }
        let mut changed = request(Action::Submit, "new-input");
        changed.user_input = "翻译你好".into();
        changed.input.selected_text = "翻译你好".into();
        let fresh = expect_respond(state.dispatch(&changed));
        assert_eq!(fresh.task.intent, "text.translate");
        assert_eq!(fresh.commands.len(), 1);
        assert_eq!(fresh.commands[0].id, "task-commit");
        assert!(fresh.commands[0].effect.is_none());
        tokio::time::timeout(Duration::from_millis(100), old.slot.cancelled())
            .await
            .expect("superseded expression must be cancelled");
        let late = commands::base_response(&old_req, Status::AgentMode);
        assert!(state
            .complete_expression(SESSION, &old.slot, late, HashMap::new())
            .is_none());
        let mut replay = request(Action::Select, "old-commit-replay");
        replay.idempotency_key = format!("{}-commit", old_req.idempotency_key);
        replay.command_id = "task-commit".into();
        replay.task_revision = 1;
        let rejected = expect_respond(state.dispatch(&replay));
        assert_eq!(rejected.error_code, "task_revision_mismatch");
        assert!(rejected.commands.is_empty());
    }
}

#[test]
fn non_expression_handoff_removes_cached_expression_preview() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "before-handoff");
    let (_, turn) = expect_start_turn(&state, &req);
    let mut response = commands::base_response(&req, Status::AgentMode);
    response.message = "old preview".into();
    response.commands = vec![commands::insert_reply_command("old preview")];
    assert!(state
        .complete_expression(SESSION, &turn.slot, response, HashMap::new())
        .is_some());
    let mut changed = request(Action::Submit, "handoff");
    changed.input.selected_text = "帮我找文件".into();
    let handoff = expect_respond(state.dispatch(&changed));
    assert_eq!(handoff.commands.len(), 1);
    assert_eq!(handoff.commands[0].id, "open-agent");
    let mut replay = request(Action::Select, "old-result-replay");
    replay.idempotency_key = format!("{}-commit", req.idempotency_key);
    replay.command_id = "task-commit".into();
    replay.task_revision = 1;
    let rejected = expect_respond(state.dispatch(&replay));
    assert_eq!(rejected.error_code, "unknown_command");
    assert!(rejected.commands.is_empty());
}

#[test]
fn reused_execution_key_rejects_changed_request_without_revoking_original() {
    for completed in [false, true] {
        let state = ImeState::new(Duration::from_secs(60));
        let req = request(Action::Submit, "binding");
        let (_, turn) = expect_start_turn(&state, &req);
        let mut commit = req.clone();
        commit.action = Action::Select;
        commit.command_id = "task-commit".into();
        commit.task_revision = 1;
        commit.idempotency_key = "binding-commit".into();
        if completed {
            state
                .complete_expression(
                    SESSION,
                    &turn.slot,
                    commands::base_response(&req, Status::Completed),
                    HashMap::new(),
                )
                .unwrap();
        }
        for variant in 0..6 {
            let mut changed = commit.clone();
            changed.request_id = format!("conflict-{variant}");
            match variant {
                0 => changed.input.selected_text = "translate another synthetic text".into(),
                1 => changed.action = Action::Submit,
                2 => changed.command_id = "other-command".into(),
                3 => changed.task_revision += 1,
                4 => changed.session_context = "different synthetic context".into(),
                _ => changed.parent_request_id = "different-parent".into(),
            }
            let rejected = expect_respond(state.dispatch(&changed));
            assert_eq!(
                rejected.error_code, "idempotency_conflict",
                "completed={completed}, variant={variant}"
            );
            assert!(rejected.commands.is_empty());
        }
        let mut retry = commit.clone();
        retry.request_id = "exact-retry".into();
        let replay = expect_respond(state.dispatch(&retry));
        assert_eq!(
            replay.status,
            if completed {
                Status::Completed
            } else {
                Status::Thinking
            }
        );
        assert_eq!(replay.request_id, "exact-retry");
        assert!(state
            .complete_expression(
                SESSION,
                &turn.slot,
                commands::base_response(&req, Status::Completed),
                HashMap::new()
            )
            .is_some());
    }
}

#[test]
fn command_completion_cache_is_bound_to_request_parameters() {
    let state = ImeState::new(Duration::from_secs(60));
    let submit = request(Action::Submit, "command-draft");
    expect_respond(state.dispatch(&submit));
    let mut command = request(Action::Select, "command-key");
    command.command_id = "open-agent".into();
    let completed = commands::base_response(&command, Status::Completed);
    state.complete_command(SESSION, &command, completed);
    let mut changed = command.clone();
    changed.command_id = "insert-reply".into();
    assert_eq!(
        expect_respond(state.dispatch(&changed)).error_code,
        "idempotency_conflict"
    );
    command.request_id = "command-retry".into();
    let replay = expect_respond(state.dispatch(&command));
    assert_eq!(replay.status, Status::Completed);
    assert_eq!(replay.request_id, "command-retry");
}

#[test]
fn draft_retry_preserves_revision_and_original_confirmation() {
    let state = ImeState::new(Duration::from_secs(60));
    let submit = request(Action::Submit, "draft-retry");
    let draft = expect_respond(state.dispatch(&submit));
    let mut retry = submit.clone();
    retry.request_id = "draft-second-delivery".into();
    let repeated = expect_respond(state.dispatch(&retry));
    assert_eq!(repeated.task, draft.task);
    assert_eq!(repeated.commands, draft.commands);
    assert_eq!(repeated.state_revision, draft.state_revision);
    assert_eq!(repeated.request_id, retry.request_id);
    let mut commit = submit.clone();
    commit.action = Action::Select;
    commit.command_id = "task-commit".into();
    commit.task_revision = draft.task.revision;
    commit.idempotency_key = "draft-confirm".into();
    assert!(matches!(
        state.dispatch(&commit),
        StateAction::StartTurn { .. }
    ));
    let after_commit = expect_respond(state.dispatch(&retry));
    assert_eq!(after_commit.status, Status::Thinking);
    assert!(after_commit.commands.is_empty());
}

#[test]
fn draft_key_conflict_preserves_original_draft() {
    let state = ImeState::new(Duration::from_secs(60));
    let submit = request(Action::Submit, "draft-key");
    let draft = expect_respond(state.dispatch(&submit));
    let mut conflicting = submit.clone();
    conflicting.request_id = "draft-conflict".into();
    conflicting.input.selected_text = "different synthetic text".into();
    let rejected = expect_respond(state.dispatch(&conflicting));
    assert_eq!(rejected.error_code, "idempotency_conflict");
    assert!(rejected.commands.is_empty());
    let repeated = expect_respond(state.dispatch(&submit));
    assert_eq!(repeated.task, draft.task);
    assert_eq!(repeated.state_revision, draft.state_revision);
}

#[test]
fn rejected_multi_slot_patch_preserves_entire_draft_and_confirmation() {
    for duplicate_slot in [false, true] {
        let state = ImeState::new(Duration::from_secs(60));
        let submit = request(Action::Submit, "atomic-draft");
        expect_respond(state.dispatch(&submit));
        let original = slot_draft();
        state.set_task(SESSION, original.clone());
        let before = expect_respond(state.dispatch(&request(Action::Poll, "atomic-before")));
        let mut patch = request(Action::Select, "atomic-patch");
        patch.task_revision = original.revision;
        patch.slot_updates = vec![
            SlotUpdate {
                slot_id: "time".into(),
                value: "first synthetic change".into(),
                consumed_ranges: vec![PinyinRange { start: 0, end: 2 }],
                lock: true,
            },
            SlotUpdate {
                slot_id: if duplicate_slot { "time" } else { "content" }.into(),
                value: "second conflicting change".into(),
                consumed_ranges: vec![],
                lock: false,
            },
        ];
        let rejected = expect_respond(state.dispatch(&patch));
        assert_eq!(rejected.error_code, "slot_locked");
        assert_eq!(
            rejected.task, original,
            "failed patch must not expose partial edits"
        );
        let after = expect_respond(state.dispatch(&request(Action::Poll, "atomic-after")));
        assert_eq!(
            after.task, original,
            "failed patch must leave stored draft unchanged"
        );
        assert_eq!(after.commands, before.commands);
    }
}

#[test]
fn accepted_patch_retry_preserves_revision_and_rejects_changed_payload() {
    let state = ImeState::new(Duration::from_secs(60));
    let submit = request(Action::Submit, "edit-submit");
    expect_respond(state.dispatch(&submit));
    state.set_task(SESSION, slot_draft());
    let mut patch = request(Action::Select, "edit-key");
    patch.task_revision = 1;
    patch.slot_updates = vec![SlotUpdate {
        slot_id: "time".into(),
        value: "revised synthetic time".into(),
        consumed_ranges: vec![],
        lock: true,
    }];
    let accepted = expect_respond(state.dispatch(&patch));
    assert_eq!(accepted.task.revision, 2);
    let mut retry = patch.clone();
    retry.request_id = "edit-redelivery".into();
    let replay = expect_respond(state.dispatch(&retry));
    assert!(
        replay.error_code.is_empty(),
        "exact retry must not be revision mismatch"
    );
    assert_eq!(replay.task, accepted.task);
    assert_eq!(replay.commands, accepted.commands);
    assert_eq!(replay.state_revision, accepted.state_revision);
    assert_eq!(replay.request_id, retry.request_id);
    retry.slot_updates[0].value = "different change under same key".into();
    let rejected = expect_respond(state.dispatch(&retry));
    assert_eq!(rejected.error_code, "idempotency_conflict");
    let after = expect_respond(state.dispatch(&request(Action::Poll, "edit-observe")));
    assert_eq!(after.task, accepted.task);
}

#[tokio::test]
async fn accepted_edit_revokes_pending_unread_and_cached_old_expression() {
    for phase in 0..3 {
        let state = ImeState::new(Duration::from_secs(60));
        let submit = request(Action::Submit, "edit-old");
        let (_, old) = expect_start_turn(&state, &submit);
        if phase > 0 {
            let mut result = commands::base_response(&submit, Status::AgentMode);
            result.commands = vec![commands::insert_reply_command("old synthetic result")];
            state
                .complete_expression(SESSION, &old.slot, result, HashMap::new())
                .unwrap();
        }
        if phase == 2 {
            expect_respond(state.dispatch(&request(Action::Poll, "consume-old")));
        }
        let mut patch = request(Action::Select, "edit-revoke");
        patch.task_revision = 1;
        patch.slot_updates = vec![
            SlotUpdate {
                slot_id: "text".into(),
                value: "你好".into(),
                consumed_ranges: vec![],
                lock: false,
            },
            SlotUpdate {
                slot_id: "text".into(),
                value: "new synthetic draft".into(),
                consumed_ranges: vec![],
                lock: true,
            },
        ];
        let edited = expect_respond(state.dispatch(&patch));
        assert!(edited.error_code.is_empty());
        assert_eq!(edited.task.revision, 2);
        assert!(
            state.pending_of(SESSION).is_none(),
            "old pending retained in phase {phase}"
        );
        if phase < 2 {
            tokio::time::timeout(Duration::from_millis(100), old.slot.cancelled())
                .await
                .unwrap();
        }
        assert!(state
            .complete_expression(
                SESSION,
                &old.slot,
                commands::base_response(&submit, Status::AgentMode),
                HashMap::new()
            )
            .is_none());
        let observed = expect_respond(state.dispatch(&request(Action::Poll, "observe-edit")));
        assert_eq!(observed.task, edited.task);
        assert_eq!(observed.commands.len(), 1);
        assert_eq!(observed.commands[0].id, "task-commit");
        assert!(observed.commands[0].effect.is_none());
        let mut old_commit = submit.clone();
        old_commit.action = Action::Select;
        old_commit.command_id = "task-commit".into();
        old_commit.task_revision = 1;
        old_commit.idempotency_key = "edit-old-commit".into();
        let rejected = expect_respond(state.dispatch(&old_commit));
        assert_eq!(rejected.error_code, "task_revision_mismatch");
        assert!(rejected.commands.is_empty());
    }
}

#[test]
fn rejected_edit_keeps_running_expression_and_accepts_its_completion() {
    let state = ImeState::new(Duration::from_secs(60));
    let submit = request(Action::Submit, "reject-keep");
    let (_, original) = expect_start_turn(&state, &submit);
    let mut patch = request(Action::Select, "reject-keep-patch");
    patch.task_revision = 1;
    patch.slot_updates = vec![SlotUpdate {
        slot_id: "text".into(),
        value: "cannot overwrite locked text".into(),
        consumed_ranges: vec![],
        lock: true,
    }];
    assert_eq!(
        expect_respond(state.dispatch(&patch)).error_code,
        "slot_locked"
    );
    assert!(std::sync::Arc::ptr_eq(
        &state.pending_of(SESSION).unwrap(),
        &original.slot
    ));
    let mut result = commands::base_response(&submit, Status::AgentMode);
    result.message = "original accepted result".into();
    result.commands = vec![commands::insert_reply_command("original accepted result")];
    assert!(state
        .complete_expression(SESSION, &original.slot, result, HashMap::new())
        .is_some());
    let observed = expect_respond(state.dispatch(&request(Action::Poll, "reject-keep-poll")));
    assert_eq!(observed.message, "original accepted result");
    assert_eq!(observed.commands.len(), 1);
    assert_eq!(observed.commands[0].id, "insert-reply");
}

#[test]
fn same_input_new_draft_revokes_consumed_result_confirmation_cache() {
    let state = ImeState::new(Duration::from_secs(60));
    let submit = request(Action::Submit, "consumed-original");
    let (_, old) = expect_start_turn(&state, &submit);
    let mut result = commands::base_response(&submit, Status::AgentMode);
    result.commands = vec![commands::insert_reply_command("consumed old result")];
    state
        .complete_expression(SESSION, &old.slot, result, HashMap::new())
        .unwrap();
    expect_respond(state.dispatch(&request(Action::Poll, "consume-result")));
    let mut fresh_submit = submit.clone();
    fresh_submit.request_id = "fresh-draft".into();
    fresh_submit.idempotency_key = "fresh-draft".into();
    let draft = expect_respond(state.dispatch(&fresh_submit));
    assert_eq!(draft.task.revision, 2);
    assert_eq!(draft.commands[0].id, "task-commit");
    let mut replay = submit;
    replay.action = Action::Select;
    replay.command_id = "task-commit".into();
    replay.task_revision = 1;
    replay.request_id = "replay-consumed-confirm".into();
    replay.idempotency_key = "consumed-original-commit".into();
    let rejected = expect_respond(state.dispatch(&replay));
    assert_eq!(rejected.error_code, "task_revision_mismatch");
    assert!(rejected.commands.is_empty());
    let observed = expect_respond(state.dispatch(&request(Action::Poll, "observe-fresh")));
    assert_eq!(observed.task, draft.task);
    assert_eq!(observed.commands, draft.commands);
}

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
        input: InputView::default(),
        application: ApplicationView::default(),
        session_context: String::new(),
        context_entries: Vec::new(),
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
    action: StateAction,
) -> (AgentIpcResponse, owo_intent_gateway::state::StartTurnRequest) {
    match action {
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
    let (immediate, turn) = expect_start_turn(state.dispatch(&req));

    assert_eq!(immediate.status, Status::Thinking);
    assert_eq!(immediate.state_revision, 1);
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
    let (_, _turn) = expect_start_turn(state.dispatch(&first));

    // 不同幂等键、同会话的再次 submit：应返回观察态而非新 turn。
    let mut second = request(Action::Submit, "req-2");
    second.idempotency_key = "req-2".to_string();
    match state.dispatch(&second) {
        StateAction::Respond(response) => {
            assert_eq!(response.status, Status::Thinking);
            assert_eq!(response.state_revision, 1, "观察态不得推进状态版本");
        }
        _ => panic!("挂起中的重复 submit 不得启动新 turn"),
    }
}

#[test]
fn complete_turn_then_poll_returns_final() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(state.dispatch(&req));

    let mut final_response = commands::base_response(&req, Status::AgentMode);
    final_response.message = "已找到 3 个文件".to_string();
    let completed = state
        .complete_turn(SESSION, final_response)
        .expect("会话存在时必须写回结果");
    assert_eq!(completed.state_revision, 2, "完成回调推进状态版本");

    let poll = request(Action::Poll, "req-3");
    let response = expect_respond(state.dispatch(&poll));
    assert_eq!(response.status, Status::AgentMode);
    assert_eq!(response.message, "已找到 3 个文件");
    assert_eq!(response.request_id, "req-3", "request_id 必须回填当前请求");
    assert_eq!(response.state_revision, 2);

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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
    let final_response = commands::base_response(&req, Status::Completed);
    state.complete_turn(SESSION, final_response).unwrap();

    // 相同 idempotency_key 重发（模拟连接器重试）：命中缓存。
    let mut retry = request(Action::Submit, "req-99");
    retry.idempotency_key = "req-1".to_string();
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
    let (_, turn) = expect_start_turn(state.dispatch(&req));

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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));

    let mut select = request(Action::Select, "req-2");
    select.command_id = "no-such-command".to_string();
    let response = expect_respond(state.dispatch(&select));
    assert_eq!(response.error_code, "unknown_command");
}

#[test]
fn page_keeps_session_and_updates_page() {
    let state = ImeState::new(Duration::from_secs(60));
    let req = request(Action::Submit, "req-1");
    let (_, _turn) = expect_start_turn(state.dispatch(&req));

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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
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
    let (_, _turn) = expect_start_turn(state.dispatch(&req));
    assert_eq!(state.session_count(), 1);

    tokio::time::sleep(Duration::from_millis(30)).await;
    state.gc();
    assert_eq!(state.session_count(), 0, "过期会话必须被 GC");
}

#[test]
fn resubmit_after_cancel_starts_new_turn() {
    let state = ImeState::new(Duration::from_secs(60));
    let (_, _turn) = expect_start_turn(state.dispatch(&request(Action::Submit, "req-1")));
    let _ = expect_respond(state.dispatch(&request(Action::Cancel, "req-2")));

    let mut again = request(Action::Submit, "req-3");
    again.idempotency_key = "req-3".to_string();
    let (immediate, _turn) = expect_start_turn(state.dispatch(&again));
    assert_eq!(immediate.status, Status::Thinking);
    assert!(state.has_session(SESSION));
}

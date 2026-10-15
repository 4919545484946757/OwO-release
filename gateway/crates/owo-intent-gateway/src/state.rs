//! 会话与任务状态机（对齐官方参考实现 `agent_mock/main.cpp` 的 `process()` 语义）。
//!
//! 分层设计：本模块是**纯同步决策核心**（无 IO、无 async），
//! 需要网络/子进程侧写的动作（启动 turn、回滚文件等）以 [`StateAction`] 委托给
//! handler/bridge 层执行，完成后回调 `complete_turn` / `complete_command`。
//!
//! 关键语义（与 mock 对齐）：
//! - 幂等：`idempotency_key` 命中缓存 → 原样返回缓存响应（仅更新 `request_id`）；
//! - `state_revision` 单调递增（非缓存响应每次 +1；会话不存在时从 1 起）；
//! - 槽位补丁校验 `task_revision` 与锁定状态（`task_revision_mismatch` / `slot_locked`）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::commands;
use crate::protocol::{
    Action, AgentIpcRequest, AgentIpcResponse, Command, SlotUpdate, Status, TaskDraft,
};

/// 默认会话过期时间。
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(30 * 60);

/// 审批等待通知（bridge 从 SSE `permission_request` 事件写入，poll 读取展示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionNotice {
    pub request_id: String,
    pub tool: String,
    pub reason: String,
}

/// 挂起 turn 的完成槽位（bridge 任务写入，状态机读取）。
#[derive(Debug, Default, Clone)]
pub struct PendingSlot {
    pub done: bool,
    pub response: Option<AgentIpcResponse>,
    pub error_message: Option<String>,
    /// 正在等待用户在可信界面审批的工具（poll 据此返回等待确认候选）。
    pub permission_waiting: Option<PermissionNotice>,
    /// 已执行的工具调用数（进度估算）。
    pub tools_used: usize,
}

/// 挂起 turn 的句柄：完成槽位 + 取消通知。
#[derive(Debug)]
pub struct PendingHandle {
    slot: Mutex<PendingSlot>,
    cancel: Notify,
}

impl PendingHandle {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            slot: Mutex::new(PendingSlot::default()),
            cancel: Notify::new(),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, PendingSlot> {
        lock(&self.slot)
    }

    /// 请求取消（存储 permit；bridge 的 turn 任务收到后执行 abort）。
    ///
    /// 用 `notify_one` 而非 `notify_waiters`：若取消发生在 bridge 进入
    /// `cancelled()` 等待之前，permit 会被保留，避免通知丢失。
    pub fn request_cancel(&self) {
        self.cancel.notify_one();
    }

    /// bridge 侧等待取消信号（与 SSE 帧流 `select!` 竞争）。
    pub async fn cancelled(&self) {
        self.cancel.notified().await;
    }
}

/// select 触发的命令动作（真实 IO 由 handler/bridge 执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    /// 把文本作为回复候选（是否上屏由连接器决定）。
    InsertReply { text: String },
    /// 打开 Agent 可信界面确认（审批）。
    OpenConfirmUi { request_id: String },
    /// 打开 Agent 工作台（diff 审阅等只读展示）。
    OpenWorkbench,
    /// 回滚全部改动。
    RevertAll,
    /// 提交任务草稿。
    CommitTask,
}

/// 启动 turn 的委托数据。
pub struct StartTurnRequest {
    pub session_id: String,
    pub request: AgentIpcRequest,
    pub slot: Arc<PendingHandle>,
}

/// 状态机对请求的处理结果。
pub enum StateAction {
    /// 立即响应（已含 state_revision）。
    Respond(AgentIpcResponse),
    /// 启动 turn：先发回 `immediate` 响应，bridge 用 `turn.request` 跑任务，
    /// 完成后调用 [`ImeState::complete_turn`] 写回最终响应。
    StartTurn {
        immediate: AgentIpcResponse,
        turn: Box<StartTurnRequest>,
    },
    /// 执行命令动作；handler 完成后调用 [`ImeState::complete_command`] 写回响应。
    RunCommand {
        session_id: String,
        action: CommandAction,
    },
}

struct ImeSession {
    last_request: AgentIpcRequest,
    task: TaskDraft,
    state_revision: u64,
    pending: Option<Arc<PendingHandle>>,
    commands: Vec<Command>,
    actions: HashMap<String, CommandAction>,
    expires_at: Instant,
}

/// 输入法会话状态机。
pub struct ImeState {
    sessions: Mutex<HashMap<String, ImeSession>>,
    idempotency: Mutex<HashMap<String, AgentIpcResponse>>,
    session_ttl: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 状态机不 panic 优先：锁中毒后继续提供服务。
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ImeState {
    pub fn new(session_ttl: Duration) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            idempotency: Mutex::new(HashMap::new()),
            session_ttl,
        }
    }

    /// 当前活跃会话数（测试/诊断用）。
    pub fn session_count(&self) -> usize {
        lock(&self.sessions).len()
    }

    /// 会话是否存活（诊断用）。
    pub fn has_session(&self, session_id: &str) -> bool {
        lock(&self.sessions).contains_key(session_id)
    }

    /// 惰性 GC：移除过期会话与过期幂等缓存。
    pub fn gc(&self) {
        let now = Instant::now();
        lock(&self.sessions).retain(|_, session| session.expires_at > now);
        // 幂等缓存随会话生命周期：仅保留仍活跃会话的键前缀。
        let sessions = lock(&self.sessions);
        lock(&self.idempotency).retain(|key, _| {
            key.split_once(':')
                .is_some_and(|(session_id, _)| sessions.contains_key(session_id))
        });
    }

    /// 处理一个请求（纯状态决策）。
    pub fn dispatch(&self, request: &AgentIpcRequest) -> StateAction {
        self.gc();

        // 幂等：同 (session_id, idempotency_key) 命中缓存 → 原样返回。
        if !request.idempotency_key.is_empty() {
            let scoped = format!("{}:{}", request.session_id, request.idempotency_key);
            if let Some(mut cached) = lock(&self.idempotency).get(&scoped).cloned() {
                cached.request_id = request.request_id.clone();
                return StateAction::Respond(cached);
            }
        }

        match request.action {
            Action::Submit => self.handle_submit(request),
            Action::Select => self.handle_select(request),
            Action::Page => self.handle_page(request),
            Action::Cancel => self.handle_cancel(request),
            Action::Poll => self.handle_poll(request),
        }
    }

    // ────────────────────────── 各动作处理 ──────────────────────────

    fn handle_submit(&self, request: &AgentIpcRequest) -> StateAction {
        let slot = PendingHandle::new();
        let mut sessions = lock(&self.sessions);

        let state_revision = match sessions.get_mut(&request.session_id) {
            Some(session) => {
                session.expires_at = Instant::now() + self.session_ttl;
                if let Some(pending) = session.pending.clone() {
                    let mut slot_guard = pending.lock();
                    if !slot_guard.done {
                        // 挂起中：不重复启动 turn，返回观察态（不改变状态版本）。
                        drop(slot_guard);
                        let mut thinking = commands::thinking_response(request, 10, 1500);
                        thinking.state_revision = session.state_revision;
                        return StateAction::Respond(thinking);
                    }
                    // turn 已完成：先取出最终结果（等价于一次立即 poll）。
                    let finished = slot_guard.response.take();
                    slot_guard.done = false;
                    drop(slot_guard);
                    session.pending = None;
                    if let Some(mut response) = finished {
                        response.request_id = request.request_id.clone();
                        return StateAction::Respond(response);
                    }
                    // done 但无结果（异常态）：按新 submit 继续。
                }
                session.last_request = request.clone();
                session.pending = Some(Arc::clone(&slot));
                bump_revision(session)
            }
            None => {
                let mut session = ImeSession {
                    last_request: request.clone(),
                    task: TaskDraft::default(),
                    state_revision: 0,
                    pending: Some(Arc::clone(&slot)),
                    commands: Vec::new(),
                    actions: HashMap::new(),
                    expires_at: Instant::now() + self.session_ttl,
                };
                let revision = bump_revision(&mut session);
                sessions.insert(request.session_id.clone(), session);
                revision
            }
        };

        let mut immediate = commands::thinking_response(request, 10, 1500);
        immediate.state_revision = state_revision;
        StateAction::StartTurn {
            immediate,
            turn: Box::new(StartTurnRequest {
                session_id: request.session_id.clone(),
                request: request.clone(),
                slot,
            }),
        }
    }

    fn handle_select(&self, request: &AgentIpcRequest) -> StateAction {
        let mut sessions = lock(&self.sessions);
        let Some(session) = sessions.get_mut(&request.session_id) else {
            return StateAction::Respond(commands::error_response(
                request,
                "session_not_found",
                "找不到该会话，请重新发起请求",
            ));
        };
        session.expires_at = Instant::now() + self.session_ttl;

        // 槽位补丁（task.slots 增量语义）。
        if !request.slot_updates.is_empty() {
            return StateAction::Respond(self.apply_slot_patch(request, session));
        }

        // 命令选择。
        if !request.command_id.is_empty() {
            let Some(action) = session.actions.get(&request.command_id).cloned() else {
                let response = commands::error_response(
                    request,
                    "unknown_command",
                    "候选已过期或不存在，请刷新候选",
                );
                return StateAction::Respond(self.finalize(session, response));
            };
            // 纯展示类动作立即响应；其余委托 handler。
            match &action {
                CommandAction::InsertReply { text } => {
                    let mut response = commands::base_response(request, Status::Completed);
                    response.message = text.clone();
                    return StateAction::Respond(self.finalize(session, response));
                }
                // 打开界面 / 回滚 / 任务提交均需要 handler 侧真实 IO。
                CommandAction::OpenConfirmUi { .. }
                | CommandAction::OpenWorkbench
                | CommandAction::RevertAll
                | CommandAction::CommitTask => {
                    return StateAction::RunCommand {
                        session_id: request.session_id.clone(),
                        action,
                    };
                }
            }
        }

        // 无 command_id 的 select：按当前草稿重放（宽容处理，照 mock 语义走向确认）。
        let mut response = commands::base_response(request, Status::AgentMode);
        response.message = "请选择一个候选或补充输入".to_string();
        response.task = session.task.clone();
        response.commands = session.commands.clone();
        StateAction::Respond(self.finalize(session, response))
    }

    fn handle_page(&self, request: &AgentIpcRequest) -> StateAction {
        let mut sessions = lock(&self.sessions);
        let Some(session) = sessions.get_mut(&request.session_id) else {
            return StateAction::Respond(commands::error_response(
                request,
                "session_not_found",
                "找不到该会话，请重新发起请求",
            ));
        };
        session.expires_at = Instant::now() + self.session_ttl;
        let mut response = commands::base_response(request, Status::AgentMode);
        response.message = "已切换候选页".to_string();
        response.task = session.task.clone();
        response.commands = session.commands.clone();
        response.page = request.page;
        StateAction::Respond(self.finalize(session, response))
    }

    fn handle_cancel(&self, request: &AgentIpcRequest) -> StateAction {
        let mut sessions = lock(&self.sessions);
        if let Some(session) = sessions.remove(&request.session_id) {
            if let Some(pending) = &session.pending {
                let slot = pending.lock();
                if !slot.done {
                    drop(slot);
                    pending.request_cancel(); // bridge 收到后执行 HTTP abort
                }
            }
        }
        let mut response = commands::base_response(request, Status::Cancelled);
        response.message = "会话已取消".to_string();
        response.state_revision = 1;
        StateAction::Respond(response)
    }

    fn handle_poll(&self, request: &AgentIpcRequest) -> StateAction {
        let mut sessions = lock(&self.sessions);
        let Some(session) = sessions.get_mut(&request.session_id) else {
            return StateAction::Respond(commands::error_response(
                request,
                "session_not_found",
                "找不到该会话，请重新发起请求",
            ));
        };
        session.expires_at = Instant::now() + self.session_ttl;

        if let Some(pending) = session.pending.clone() {
            let mut slot = pending.lock();
            if slot.done {
                let response = slot.response.take().unwrap_or_else(|| {
                    commands::error_response(request, "internal", "任务已完成但无结果")
                });
                slot.done = false; // 结果只取一次；后续以幂等缓存/空闲态兜底
                drop(slot);
                session.pending = None;
                let mut response = response;
                response.request_id = request.request_id.clone();
                return StateAction::Respond(response);
            }
            let notice = slot.permission_waiting.clone();
            let tools_used = slot.tools_used;
            drop(slot);

            // 审批等待：返回等待确认候选（走可信界面，候选框不直接授权）。
            if let Some(notice) = notice {
                let mut response = commands::base_response(request, Status::WaitingForConfirmation);
                response.message =
                    format!("等待确认：{}（请在 Agent 可信界面中处理）", notice.tool);
                response.can_cancel = true;
                response.commands = vec![commands::confirm_ui_command(
                    &notice.request_id,
                    &notice.tool,
                )];
                session.commands = response.commands.clone();
                session.actions = HashMap::from([(
                    format!("confirm-{}", notice.request_id),
                    CommandAction::OpenConfirmUi {
                        request_id: notice.request_id.clone(),
                    },
                )]);
                response.state_revision = session.state_revision;
                return StateAction::Respond(response);
            }

            let progress = (20 + (tools_used as u32).saturating_mul(15)).min(90);
            return StateAction::Respond(commands::thinking_response(request, progress, 1500));
        }

        let mut response = commands::base_response(request, Status::AgentMode);
        response.message = "当前没有执行中的任务".to_string();
        response.task = session.task.clone();
        response.commands = session.commands.clone();
        StateAction::Respond(self.finalize(session, response))
    }

    // ────────────────────────── 槽位补丁 ──────────────────────────

    fn apply_slot_patch(
        &self,
        request: &AgentIpcRequest,
        session: &mut ImeSession,
    ) -> AgentIpcResponse {
        if request.task_revision != session.task.revision {
            let mut response = commands::error_response(
                request,
                "task_revision_mismatch",
                "任务版本不匹配，请刷新候选",
            );
            response.task = session.task.clone();
            return self.finalize(session, response);
        }
        for update in &request.slot_updates {
            let Some(slot) = session
                .task
                .slots
                .iter_mut()
                .find(|slot| slot.id == update.slot_id)
            else {
                continue; // 未知槽位忽略（mock 语义）
            };
            if slot.locked && slot.value != update.value {
                let mut response =
                    commands::error_response(request, "slot_locked", "任务槽位已经锁定，不能覆盖");
                response.task = session.task.clone();
                return self.finalize(session, response);
            }
            apply_update(slot, update);
        }
        session.task.revision += 1;
        session.commands = vec![commands::task_commit_command(session.task.revision)];
        session.actions = HashMap::from([("task-commit".to_string(), CommandAction::CommitTask)]);

        let mut response = commands::base_response(request, Status::AgentMode);
        response.message = "任务草稿已更新，请确认提交".to_string();
        response.task = session.task.clone();
        response.commands = session.commands.clone();
        self.finalize(session, response)
    }

    // ────────────────────────── 完成回调 ──────────────────────────

    /// bridge 的 turn 任务完成后调用：写回槽位、递增 revision、写幂等缓存。
    ///
    /// 会话已被取消/GC 时返回 `None`（结果被丢弃）。
    pub fn complete_turn(
        &self,
        session_id: &str,
        mut response: AgentIpcResponse,
    ) -> Option<AgentIpcResponse> {
        let mut sessions = lock(&self.sessions);
        let session = sessions.get_mut(session_id)?;
        response.state_revision = bump_revision(session);
        if let Some(pending) = session.pending.clone() {
            let mut slot = pending.lock();
            slot.done = true;
            slot.response = Some(response.clone());
        }
        // 幂等缓存（key = 原 submit 的 idempotency_key）。
        if !session.last_request.idempotency_key.is_empty() {
            let scoped = format!("{}:{}", session_id, session.last_request.idempotency_key);
            lock(&self.idempotency).insert(scoped, response.clone());
        }
        Some(response)
    }

    /// 命令动作（回滚/提交等）完成后调用：递增 revision 并写缓存。
    pub fn complete_command(
        &self,
        session_id: &str,
        request: &AgentIpcRequest,
        response: AgentIpcResponse,
    ) -> AgentIpcResponse {
        let mut response = response;
        let mut sessions = lock(&self.sessions);
        match sessions.get_mut(session_id) {
            Some(session) => response.state_revision = bump_revision(session),
            None => response.state_revision = 1,
        }
        if !request.idempotency_key.is_empty() {
            let scoped = format!("{}:{}", session_id, request.idempotency_key);
            lock(&self.idempotency).insert(scoped, response.clone());
        }
        response
    }

    /// 注册当前会话的候选命令（E1.5 turn 结果映射使用）。
    pub fn set_commands(
        &self,
        session_id: &str,
        commands: Vec<Command>,
        actions: HashMap<String, CommandAction>,
    ) {
        let mut sessions = lock(&self.sessions);
        if let Some(session) = sessions.get_mut(session_id) {
            session.commands = commands;
            session.actions = actions;
        }
    }

    /// 读取指定会话的挂起句柄（bridge 使用）。
    pub fn pending_of(&self, session_id: &str) -> Option<Arc<PendingHandle>> {
        lock(&self.sessions)
            .get(session_id)
            .and_then(|session| session.pending.clone())
    }

    /// 设置会话的任务草稿（E1.5 使用）。
    pub fn set_task(&self, session_id: &str, task: TaskDraft) {
        let mut sessions = lock(&self.sessions);
        if let Some(session) = sessions.get_mut(session_id) {
            session.task = task;
        }
    }

    fn finalize(
        &self,
        session: &mut ImeSession,
        mut response: AgentIpcResponse,
    ) -> AgentIpcResponse {
        response.state_revision = bump_revision(session);
        response
    }
}

fn bump_revision(session: &mut ImeSession) -> u64 {
    session.state_revision += 1;
    session.state_revision
}

fn apply_update(slot: &mut crate::protocol::TaskSlot, update: &SlotUpdate) {
    slot.value = update.value.clone();
    slot.source_ranges = update.consumed_ranges.clone();
    slot.locked = update.lock;
    slot.confidence_milli = 1000;
}

use std::collections::VecDeque;
use std::sync::{Mutex, Weak};

use axum::http::StatusCode;
use owo_agent_protocol::SseEvent;

pub(crate) const TURN_EVENT_QUEUE_CAPACITY: usize = 128;
pub(crate) const TURN_EVENT_QUEUE_MAX_BYTES: usize = 1024 * 1024;
pub(crate) const TURN_EVENT_COALESCE_AT: usize = TURN_EVENT_QUEUE_CAPACITY / 2;

pub(crate) struct TurnEventQueue {
    pub(crate) state: Mutex<TurnEventQueueState>,
    pub(crate) notify: tokio::sync::Notify,
}

pub(crate) struct TurnEventQueueState {
    pub(crate) events: VecDeque<QueuedTurnEvent>,
    pub(crate) buffered_bytes: usize,
    pub(crate) closed: bool,
    pub(crate) overflowed: bool,
    pub(crate) overflow_notice_sent: bool,
    pub(crate) disconnect_recorded: bool,
}

#[derive(Debug)]
pub(crate) struct QueuedTurnEvent {
    pub(crate) seq: Option<u64>,
    pub(crate) event: SseEvent,
}

#[derive(Debug)]
pub(crate) enum TurnEventPushError {
    ConsumerGone,
    Full,
}

#[derive(Debug)]
pub(crate) enum TurnEventPop {
    Event(Box<QueuedTurnEvent>),
    Empty,
    Closed,
}

impl TurnEventQueue {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(TurnEventQueueState {
                events: VecDeque::with_capacity(TURN_EVENT_QUEUE_CAPACITY),
                buffered_bytes: 0,
                closed: false,
                overflowed: false,
                overflow_notice_sent: false,
                disconnect_recorded: false,
            }),
            notify: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn push(
        &self,
        seq: Option<u64>,
        event: SseEvent,
        receiver_alive: &Weak<()>,
    ) -> Result<(), TurnEventPushError> {
        if receiver_alive.upgrade().is_none() {
            if let Ok(mut state) = self.state.lock() {
                if !state.disconnect_recorded {
                    state.disconnect_recorded = true;
                    crate::observability_api::record_turn_sse_disconnect();
                }
            }
            return Err(TurnEventPushError::ConsumerGone);
        }

        let Ok(mut state) = self.state.lock() else {
            return Err(TurnEventPushError::Full);
        };
        if state.closed || state.overflowed {
            return Err(TurnEventPushError::Full);
        }

        if state.events.len() >= TURN_EVENT_COALESCE_AT {
            // 正文与思考都是纯文本增量：拥塞时把相邻同类事件并成一条。思考尤其需要
            // 合并——一次回合可吐数万条 reasoning_delta，逐条占队列会让慢消费者在
            // 几十秒内就撞上容量上限，被迫中断 SSE 再走断线重放。
            let merged_event = match (state.events.back(), &event) {
                (
                    Some(QueuedTurnEvent {
                        event: SseEvent::TokenDelta { delta: previous },
                        ..
                    }),
                    SseEvent::TokenDelta { delta },
                ) => Some(SseEvent::TokenDelta {
                    delta: join_delta(previous, delta),
                }),
                (
                    Some(QueuedTurnEvent {
                        event: SseEvent::ReasoningDelta { delta: previous },
                        ..
                    }),
                    SseEvent::ReasoningDelta { delta },
                ) => Some(SseEvent::ReasoningDelta {
                    delta: join_delta(previous, delta),
                }),
                _ => None,
            };
            if let Some(merged_event) = merged_event {
                let old_bytes = state
                    .events
                    .back()
                    .map(|last| event_wire_size(&last.event))
                    .unwrap_or(0);
                let new_bytes = event_wire_size(&merged_event);
                let next_bytes = state
                    .buffered_bytes
                    .saturating_sub(old_bytes)
                    .saturating_add(new_bytes);
                if next_bytes > TURN_EVENT_QUEUE_MAX_BYTES {
                    state.overflowed = true;
                    crate::observability_api::record_turn_sse_slow_consumer();
                    drop(state);
                    self.notify.notify_one();
                    return Err(TurnEventPushError::Full);
                }
                if let Some(last) = state.events.back_mut() {
                    last.seq = seq;
                    last.event = merged_event;
                    state.buffered_bytes = next_bytes;
                }
                drop(state);
                self.notify.notify_one();
                return Ok(());
            }
        }

        let event_bytes = event_wire_size(&event);
        if state.events.len() >= TURN_EVENT_QUEUE_CAPACITY
            || state.buffered_bytes.saturating_add(event_bytes) > TURN_EVENT_QUEUE_MAX_BYTES
        {
            state.overflowed = true;
            crate::observability_api::record_turn_sse_slow_consumer();
            drop(state);
            self.notify.notify_one();
            return Err(TurnEventPushError::Full);
        }

        state.buffered_bytes += event_bytes;
        state.events.push_back(QueuedTurnEvent { seq, event });
        drop(state);
        self.notify.notify_one();
        Ok(())
    }

    pub(crate) fn pop(&self) -> TurnEventPop {
        let Ok(mut state) = self.state.lock() else {
            return TurnEventPop::Closed;
        };
        if let Some(event) = state.events.pop_front() {
            state.buffered_bytes = state
                .buffered_bytes
                .saturating_sub(event_wire_size(&event.event));
            return TurnEventPop::Event(Box::new(event));
        }
        if state.closed {
            if state.overflowed && !state.overflow_notice_sent {
                state.overflow_notice_sent = true;
                return TurnEventPop::Event(Box::new(QueuedTurnEvent {
                    seq: None,
                    event: SseEvent::Progress {
                        message:
                        "[turn/sse_slow_consumer] 回合因 SSE 客户端处理过慢而取消；本次流不支持续传，请从会话中重新发起"
                            .to_string(),
                    },
                }));
            }
            return TurnEventPop::Closed;
        }
        TurnEventPop::Empty
    }

    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.notify.notify_one();
    }
}

/// 拼接两段增量文本。预留容量避免长思考反复扩容（单条可到 64KiB）。
fn join_delta(previous: &str, delta: &str) -> String {
    let mut merged = String::with_capacity(previous.len() + delta.len());
    merged.push_str(previous);
    merged.push_str(delta);
    merged
}

pub(crate) fn event_wire_size(event: &SseEvent) -> usize {
    serde_json::to_vec(event)
        .map(|value| value.len())
        .unwrap_or(usize::MAX)
}

pub(crate) fn persist_and_queue_event(
    store: &dyn owo_agent_core::SessionStore,
    session_id: &str,
    turn_id: &str,
    queue: &TurnEventQueue,
    receiver_alive: &Weak<()>,
    event: SseEvent,
) -> Result<(), String> {
    let record = store
        .append_turn_event(session_id, turn_id, &event)
        .map_err(|error| error.to_string())?;
    queue
        .push(Some(record.seq), event, receiver_alive)
        .map_err(|_| "回合 SSE 客户端已断开或队列已满".to_string())
}

pub(crate) fn poison<T>(_error: std::sync::PoisonError<T>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, "状态锁中毒".to_string())
}

use super::handlers::turn_replay_state;
use super::queue::*;
use owo_agent_protocol::{CompletionStatusV1, SseEvent, TurnEventRecord, TurnReplayState};
use std::sync::{Arc, Weak};

fn queue_and_receiver() -> (TurnEventQueue, Arc<()>, Weak<()>) {
    let receiver = Arc::new(());
    let receiver_weak = Arc::downgrade(&receiver);
    (TurnEventQueue::new(), receiver, receiver_weak)
}

#[test]
fn coalesces_adjacent_token_deltas_without_changing_text_order() {
    let (queue, _receiver, receiver_weak) = queue_and_receiver();
    let expected = "x".repeat(TURN_EVENT_COALESCE_AT + 17);
    for seq in 1..=(TURN_EVENT_COALESCE_AT + 17) {
        queue
            .push(
                Some(seq as u64),
                SseEvent::TokenDelta {
                    delta: "x".to_string(),
                },
                &receiver_weak,
            )
            .expect("queue has room and a live receiver");
    }
    {
        let state = queue.state.lock().expect("queue lock");
        assert_eq!(state.events.len(), TURN_EVENT_COALESCE_AT);
        assert!(state.buffered_bytes <= TURN_EVENT_QUEUE_MAX_BYTES);
    }

    queue.close();
    let mut actual = String::new();
    loop {
        match queue.pop() {
            TurnEventPop::Event(event) => match event.event {
                SseEvent::TokenDelta { delta } => actual.push_str(&delta),
                unexpected => panic!("unexpected event: {unexpected:?}"),
            },
            TurnEventPop::Closed => break,
            other => panic!("unexpected queue result: {other:?}"),
        }
    }
    assert_eq!(actual, expected);
}

#[test]
fn queue_capacity_is_hard_bounded_and_overflow_is_reported_after_drain() {
    let slow_before = crate::observability_api::turn_sse_counts_for_test().0;
    let (queue, _receiver, receiver_weak) = queue_and_receiver();
    for index in 0..TURN_EVENT_QUEUE_CAPACITY {
        queue
            .push(
                Some(index as u64 + 1),
                SseEvent::Progress {
                    message: format!("event-{index}"),
                },
                &receiver_weak,
            )
            .expect("the queue accepts up to its configured capacity");
    }
    assert!(matches!(
        queue.push(
            Some(TURN_EVENT_QUEUE_CAPACITY as u64 + 1),
            SseEvent::Progress {
                message: "overflow".to_string(),
            },
            &receiver_weak,
        ),
        Err(TurnEventPushError::Full)
    ));
    {
        let state = queue.state.lock().expect("queue lock");
        assert_eq!(state.events.len(), TURN_EVENT_QUEUE_CAPACITY);
        assert!(state.buffered_bytes <= TURN_EVENT_QUEUE_MAX_BYTES);
        assert!(state.overflowed);
    }

    queue.close();
    for _ in 0..TURN_EVENT_QUEUE_CAPACITY {
        assert!(matches!(queue.pop(), TurnEventPop::Event(_)));
    }
    match queue.pop() {
        TurnEventPop::Event(event) => match event.event {
            SseEvent::Progress { message } => {
                assert!(message.starts_with("[turn/sse_slow_consumer]"));
            }
            unexpected => panic!("expected progress event, got {unexpected:?}"),
        },
        other => panic!("expected slow-consumer terminal event, got {other:?}"),
    }
    assert!(matches!(queue.pop(), TurnEventPop::Closed));
    assert!(
        crate::observability_api::turn_sse_counts_for_test().0 > slow_before,
        "bounded queue overflow should increment the slow-consumer counter"
    );
}

#[test]
fn oversized_event_is_rejected_without_exceeding_byte_limit() {
    let (queue, _receiver, receiver_weak) = queue_and_receiver();
    assert!(matches!(
        queue.push(
            Some(1),
            SseEvent::Progress {
                message: "x".repeat(TURN_EVENT_QUEUE_MAX_BYTES + 1),
            },
            &receiver_weak,
        ),
        Err(TurnEventPushError::Full)
    ));
    let state = queue.state.lock().expect("queue lock");
    assert!(state.events.is_empty());
    assert_eq!(state.buffered_bytes, 0);
    assert!(state.overflowed);
}

#[test]
fn dropped_receiver_stops_accepting_events() {
    let disconnects_before = crate::observability_api::turn_sse_counts_for_test().1;
    let queue = TurnEventQueue::new();
    let receiver = Arc::new(());
    let receiver_weak = Arc::downgrade(&receiver);
    drop(receiver);

    assert!(matches!(
        queue.push(
            Some(1),
            SseEvent::TokenDelta {
                delta: "not delivered".to_string(),
            },
            &receiver_weak,
        ),
        Err(TurnEventPushError::ConsumerGone)
    ));
    assert!(matches!(queue.pop(), TurnEventPop::Empty));
    assert!(matches!(
        queue.push(
            Some(2),
            SseEvent::TokenDelta {
                delta: "still gone".to_string(),
            },
            &receiver_weak,
        ),
        Err(TurnEventPushError::ConsumerGone)
    ));
    assert_eq!(
        crate::observability_api::turn_sse_counts_for_test().1,
        disconnects_before + 1,
        "repeated producer callbacks should count one disconnect per turn stream"
    );
}

fn replay_record(seq: u64, payload: SseEvent) -> TurnEventRecord {
    TurnEventRecord {
        session_id: "session".to_string(),
        turn_id: "turn".to_string(),
        seq,
        created_at: "2026-10-04T00:00:00Z".to_string(),
        payload,
    }
}

fn replay_stats() -> SseEvent {
    SseEvent::TurnStats {
        steps: 1,
        duration_ms: 1,
        prompt_tokens: 1,
        completion_tokens: 1,
        total_tokens: 2,
        cost_usd: 0.0,
        completion_status: CompletionStatusV1::Accepted,
        completion_record: None,
        model_calls: Vec::new(),
    }
}

#[test]
fn replay_page_with_lookahead_remains_active_until_final_stats_page() {
    let first_page = vec![replay_record(
        7,
        SseEvent::Final {
            text: "done".to_string(),
        },
    )];
    assert_eq!(
        turn_replay_state(&first_page, true, false),
        TurnReplayState::Active
    );

    let terminal_marker_with_more = vec![replay_record(8, replay_stats())];
    assert_eq!(
        turn_replay_state(&terminal_marker_with_more, true, false),
        TurnReplayState::Active,
        "a later page may contain a failure tail"
    );

    let terminal_page = vec![replay_record(8, replay_stats())];
    assert_eq!(
        turn_replay_state(&terminal_page, false, false),
        TurnReplayState::Completed
    );
}

#[test]
fn replay_page_without_more_data_reports_interrupted_and_failures_are_terminal() {
    assert_eq!(
        turn_replay_state(&[], false, false),
        TurnReplayState::Interrupted
    );

    let failed = vec![replay_record(
        9,
        SseEvent::TurnFailed {
            message: "save failed".to_string(),
            completion_status: CompletionStatusV1::Unverified,
            completion_record: None,
        },
    )];
    assert_eq!(
        turn_replay_state(&failed, false, false),
        TurnReplayState::Failed
    );
}

/// 思考增量与正文一样是纯文本流：拥塞时同样合并。一次回合可吐数万条
/// reasoning_delta，逐条占队列会让慢消费者很快撞上容量上限、被断开重连。
#[test]
fn coalesces_adjacent_reasoning_deltas_and_tracks_latest_seq() {
    let (queue, _receiver, receiver_weak) = queue_and_receiver();
    let total = TURN_EVENT_COALESCE_AT + 9;
    for seq in 1..=total {
        queue
            .push(
                Some(seq as u64),
                SseEvent::ReasoningDelta {
                    delta: "想".to_string(),
                },
                &receiver_weak,
            )
            .expect("queue has room and a live receiver");
    }
    {
        let state = queue.state.lock().expect("queue lock");
        assert_eq!(state.events.len(), TURN_EVENT_COALESCE_AT);
        let last = state.events.back().expect("queue is not empty");
        assert_eq!(
            last.seq,
            Some(total as u64),
            "合并后的思考事件必须携带最新 seq，前端游标才能前进"
        );
    }

    queue.close();
    let mut actual = String::new();
    loop {
        match queue.pop() {
            TurnEventPop::Event(event) => match event.event {
                SseEvent::ReasoningDelta { delta } => actual.push_str(&delta),
                unexpected => panic!("unexpected event: {unexpected:?}"),
            },
            TurnEventPop::Closed => break,
            other => panic!("unexpected queue result: {other:?}"),
        }
    }
    assert_eq!(
        actual.chars().count(),
        total,
        "合并只减少事件条数，不丢字符"
    );
}

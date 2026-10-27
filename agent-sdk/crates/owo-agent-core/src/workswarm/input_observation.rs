//! The host's prepared Worker receipt follows the actual execution future.
//! It is not encoded in user JSON, and is never inherited by an unrelated task.
use owo_agent_protocol::RunEvent;
use std::future::Future;

tokio::task_local! {
    static WORKER_INPUT_EVENT: RunEvent;
}

/// Current host-persisted input identity. Missing outside RoleWorker execution.
pub fn current_worker_input_event() -> Option<RunEvent> {
    WORKER_INPUT_EVENT.try_with(Clone::clone).ok()
}

pub(super) async fn scope_worker_input<F: Future>(event: RunEvent, execution: F) -> F::Output {
    WORKER_INPUT_EVENT.scope(event, execution).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(run: &str) -> RunEvent {
        RunEvent {
            schema_version: 1,
            seq: 1,
            event_id: format!("run:{run}:1"),
            run_id: run.into(),
            event_type: "worker.input.prepared".into(),
            timestamp: "fixture".into(),
            node_id: None,
            task_id: Some("step".into()),
            attempt: Some("attempt".into()),
            causation_id: None,
            tool_call_id: None,
            artifact_refs: Vec::new(),
            data: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn parallel_worker_scopes_keep_exact_receipts_and_do_not_escape() {
        assert!(current_worker_input_event().is_none());
        let first = event("first");
        let second = event("second");
        let check = |expected: RunEvent| async move {
            tokio::task::yield_now().await;
            assert_eq!(current_worker_input_event(), Some(expected));
        };
        tokio::join!(
            scope_worker_input(first.clone(), check(first)),
            scope_worker_input(second.clone(), check(second))
        );
        assert!(current_worker_input_event().is_none());
    }

    #[tokio::test]
    async fn cancelled_scope_cannot_leak_its_receipt_to_the_next_execution() {
        let mut pending = Box::pin(scope_worker_input(event("cancelled"), async {
            assert_eq!(current_worker_input_event().unwrap().run_id, "cancelled");
            std::future::pending::<()>().await;
        }));
        tokio::select! {
            biased;
            _ = &mut pending => panic!("pending worker unexpectedly returned"),
            _ = tokio::task::yield_now() => {}
        }
        drop(pending);
        assert!(current_worker_input_event().is_none());
        scope_worker_input(event("next"), async {
            assert_eq!(current_worker_input_event().unwrap().run_id, "next");
        })
        .await;
        assert!(current_worker_input_event().is_none());
    }
}

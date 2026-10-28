//! Small native target for the exact Provider observer and admission implementation.
//! Source-path modules avoid linking the entire Server unit-test collection.
//! No copied observer, budget logic, registry, or mock persistence layer.
#[path = "../src/workswarm_api/provider_observation.rs"]
mod provider_observation;
#[allow(dead_code)]
#[path = "../src/workswarm_metrics/util.rs"]
mod util;
#[allow(dead_code)]
#[path = "../src/workswarm_metrics/request_budget.rs"]
mod workswarm_metrics;

#[test]
fn measured_preflight_does_not_reserve_budget_or_publish_usage() {
    use owo_agent_core::gateway::{custom_model_provider, ModelProvider};
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };
    use workswarm_metrics::{
        MeasuredProvider, RequestReservationJournal, RequestUsageCollector, TeamModelRequestBudget,
    };
    let dir = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let connection = serde_json::from_value(serde_json::json!({
        "model":"selected","base_url":base,
        "input_capabilities":{"model":"selected","base_url":base,"image_input":true}
    }))
    .unwrap();
    let inner = custom_model_provider(&connection).unwrap();
    let budget = Arc::new(
        TeamModelRequestBudget::new(1, RequestReservationJournal::for_team(dir.path(), "team"))
            .unwrap(),
    );
    let calls = Arc::new(AtomicU64::new(0));
    let usage = Arc::new(RequestUsageCollector::default());
    let provider = MeasuredProvider::new_with_scoped_request_budget(
        inner,
        calls.clone(),
        usage.clone(),
        "step".into(),
        Some(budget.clone()),
        Some(1),
    );
    assert!(provider.validate_image_input(None, true).is_ok());
    assert!(provider.validate_image_input(Some("other"), false).is_ok());
    assert!(provider
        .validate_image_input(Some("other"), true)
        .unwrap_err()
        .starts_with("input/capability_binding_mismatch"));
    assert_eq!(budget.used(), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(usage.take("step").is_empty());
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
}

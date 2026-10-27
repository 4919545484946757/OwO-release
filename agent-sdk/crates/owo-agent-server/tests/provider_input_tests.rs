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

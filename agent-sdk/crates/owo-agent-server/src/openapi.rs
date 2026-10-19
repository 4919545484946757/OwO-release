//! Embedded OpenAPI contract. Keeping the data in JSON avoids compiling a large
//! json! expansion. The TS snapshot remains independent and is checked by route tests.
use crate::OWO_API_VERSION;
use axum::Json;
use serde_json::Value;
use std::sync::OnceLock;

pub(crate) async fn openapi_spec() -> Json<Value> {
    static SPEC: OnceLock<Value> = OnceLock::new();
    let spec = SPEC.get_or_init(|| {
        let mut spec: Value = serde_json::from_str(include_str!("openapi.json"))
            .expect("embedded OpenAPI JSON must be valid (covered by route contract tests)");
        spec["info"]["version"] = Value::String(env!("CARGO_PKG_VERSION").into());
        spec["x-owo-api-version"] = Value::String(OWO_API_VERSION.into());
        spec
    });
    Json(spec.clone())
}

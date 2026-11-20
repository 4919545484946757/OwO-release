//! `/plugins` 列表的签名状态（`signed`）契约：规范包看 `plugin.sig.json` 是否存在，
//! 历史格式看清单 `signature` 字段；列表不重算摘要。

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    use owo_agent_core::agent::Agent;
    use owo_agent_core::gateway::ModelProvider;
    use owo_agent_core::permissions::Policy;
    use owo_agent_core::sqlite_store::SqliteSessionStore;
    use owo_agent_core::tools::ToolRegistry;

    struct IdleProvider;
    #[async_trait::async_trait]
    impl ModelProvider for IdleProvider {
        async fn complete(
            &self,
            _messages: &[owo_agent_core::ChatMessage],
            _tools: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            Err("IdleProvider 不应被调用".to_string())
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    (state, temp)
}

fn write_spec_plugin(root: &std::path::Path, name: &str, signed: bool) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("skills/demo")).unwrap();
    std::fs::write(
        dir.join("plugin.json"),
        serde_json::json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": name,
            "version": "1.0.0"
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: 演示\n---\n",
    )
    .unwrap();
    if signed {
        // 列表只判断"有无"；摘要校验发生在安装/更新/check。
        std::fs::write(
            dir.join("plugin.sig.json"),
            r#"{"algorithm":"ed25519","public_key":"","signature":""}"#,
        )
        .unwrap();
    }
}

fn write_legacy_plugin(root: &std::path::Path, name: &str, signed: bool) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let mut manifest = serde_json::json!({
        "id": name,
        "name": name,
        "version": "1.0.0"
    });
    if signed {
        // 只验证"列表报有无"；签名本身的有效性由安装/更新/check 负责。
        manifest["signature"] = serde_json::json!({
            "algorithm": "ed25519",
            "public_key_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                [0u8; 32],
            ),
            "signature_b64": base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                [0u8; 64],
            )
        });
    }
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn plugin_list_reports_signed_state() {
    let (state, _temp) = test_state();
    write_spec_plugin(&state.workspace.join("plugins"), "plain", false);
    write_spec_plugin(&state.workspace.join("plugins"), "signed", true);
    write_legacy_plugin(&state.workspace.join("plugins"), "legacy-signed", true);
    write_legacy_plugin(&state.workspace.join("plugins"), "legacy-plain", false);

    let request = Request::builder()
        .method("GET")
        .uri("/plugins")
        .header(header::HOST, "127.0.0.1:4096")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        )
        .body(Body::empty())
        .unwrap();
    let response = owo_agent_server::build_router(Arc::clone(&state))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let plugins = body["plugins"].as_array().unwrap();
    let signed = plugins
        .iter()
        .find(|plugin| plugin["id"] == "signed")
        .expect("signed 应在列表");
    let plain = plugins
        .iter()
        .find(|plugin| plugin["id"] == "plain")
        .expect("plain 应在列表");
    assert_eq!(signed["signed"], true, "{body}");
    assert_eq!(plain["signed"], false, "{body}");
    let legacy_signed = plugins
        .iter()
        .find(|plugin| plugin["id"] == "legacy-signed")
        .expect("legacy-signed 应在列表");
    let legacy_plain = plugins
        .iter()
        .find(|plugin| plugin["id"] == "legacy-plain")
        .expect("legacy-plain 应在列表");
    assert_eq!(legacy_signed["signed"], true, "{body}");
    assert_eq!(legacy_plain["signed"], false, "{body}");
}

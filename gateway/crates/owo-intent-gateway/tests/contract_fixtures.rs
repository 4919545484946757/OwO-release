//! 读取 `contracts/agent-ipc/` 黄金样例：valid 必须可解码且往返相等，invalid 必须被拒绝。
//! C++ 端 `owo_agent_contract_tests` 读取同一份 manifest。

use std::path::PathBuf;

use owo_intent_gateway::protocol::{AgentIpcRequest, AgentIpcResponse};
use serde_json::Value;

fn contracts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../contracts/agent-ipc")
}

#[test]
fn golden_fixtures_match_expectations() {
    let dir = contracts_dir();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).expect("读取 manifest"))
            .expect("manifest 必须是 JSON");
    let fixtures = manifest["fixtures"].as_array().expect("fixtures 数组");
    assert!(!fixtures.is_empty());
    for entry in fixtures {
        let file = entry["file"].as_str().unwrap();
        let bytes = std::fs::read(dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let valid = entry["expect"] == "valid";
        match entry["kind"].as_str().unwrap() {
            "request" => {
                let decoded = AgentIpcRequest::decode(&bytes);
                if valid {
                    let request = decoded.unwrap_or_else(|e| panic!("{file} 应可解码：{e}"));
                    let again = AgentIpcRequest::decode(&request.encode().unwrap()).unwrap();
                    assert_eq!(request, again, "{file} 往返不等");
                } else {
                    assert!(decoded.is_err(), "{file} 应被拒绝");
                }
            }
            "response" => {
                let decoded = AgentIpcResponse::decode(&bytes);
                if valid {
                    let response = decoded.unwrap_or_else(|e| panic!("{file} 应可解码：{e}"));
                    let again = AgentIpcResponse::decode(&response.encode().unwrap()).unwrap();
                    assert_eq!(response, again, "{file} 往返不等");
                } else {
                    assert!(decoded.is_err(), "{file} 应被拒绝");
                }
            }
            other => panic!("{file}: 未知 kind {other}"),
        }
    }
}

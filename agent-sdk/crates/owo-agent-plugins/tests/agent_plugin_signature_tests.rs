use ed25519_dalek::{Signer, SigningKey};
use owo_agent_plugins::agent_plugins::{
    agent_plugin_package_digest, load_agent_plugin_signature, verify_agent_plugin_signature,
    AgentPluginPackageSignature, AGENT_PLUGIN_SIGNATURE_FILE,
};
use owo_agent_plugins::plugin::PluginManager;
use serde_json::json;
use std::path::{Path, PathBuf};

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "owo-agent-plugin-sig-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_package(root: &Path) {
    std::fs::create_dir_all(root.join("skills/demo")).unwrap();
    std::fs::write(
        root.join("plugin.json"),
        serde_json::to_string_pretty(&json!({
            "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
            "name": "signed-demo",
            "version": "1.0.0"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: 演示\n---\n\n# Demo\n",
    )
    .unwrap();
}

fn sign_package(root: &Path, secret: [u8; 32]) {
    let digest = agent_plugin_package_digest(root).unwrap();
    let signing_key = SigningKey::from_bytes(&secret);
    let signature = signing_key.sign(&digest);
    let payload = AgentPluginPackageSignature {
        algorithm: "ed25519".to_string(),
        public_key: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            signing_key.verifying_key().to_bytes(),
        ),
        signature: base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            signature.to_bytes(),
        ),
    };
    std::fs::write(
        root.join(AGENT_PLUGIN_SIGNATURE_FILE),
        serde_json::to_string_pretty(&payload).unwrap(),
    )
    .unwrap();
}

#[test]
fn package_signature_covers_every_file() {
    let dir = scratch("ok");
    write_package(&dir);
    // 未签名：读取返回 None。
    assert!(load_agent_plugin_signature(&dir).unwrap().is_none());

    sign_package(&dir, [7u8; 32]);
    let signature = load_agent_plugin_signature(&dir)
        .unwrap()
        .expect("应有签名");
    verify_agent_plugin_signature(&dir, &signature).expect("签名应通过");

    // 篡改任意文件 → 摘要变化 → 验签失败。
    std::fs::write(dir.join("skills/demo/SKILL.md"), "tampered").unwrap();
    assert!(verify_agent_plugin_signature(&dir, &signature).is_err());

    // 算法不支持 → 明确报错。
    let mut bad = signature.clone();
    bad.algorithm = "rsa".to_string();
    assert!(verify_agent_plugin_signature(&dir, &bad)
        .unwrap_err()
        .contains("不支持的签名算法"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signature_file_must_be_valid_json() {
    let dir = scratch("bad");
    write_package(&dir);
    std::fs::write(dir.join(AGENT_PLUGIN_SIGNATURE_FILE), "{not json").unwrap();
    assert!(load_agent_plugin_signature(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn manager_accepts_signed_package_and_rejects_tampering() {
    let data = scratch("data");
    let pkg = scratch("pkg");
    write_package(&pkg);
    sign_package(&pkg, [7u8; 32]);
    let manager = PluginManager::new(data, "0.5.0".to_string());
    let report = manager.verify_plugin_dir(&pkg).expect("签名包应通过校验");
    assert!(
        report
            .audit
            .iter()
            .any(|line| line.contains("签名校验通过")),
        "{:?}",
        report.audit
    );

    std::fs::write(pkg.join("skills/demo/SKILL.md"), "tampered").unwrap();
    assert!(manager.verify_plugin_dir(&pkg).is_err(), "篡改后应拒绝");
    let _ = std::fs::remove_dir_all(&pkg);
}

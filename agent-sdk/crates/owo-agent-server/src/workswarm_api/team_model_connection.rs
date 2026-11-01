//! 团队运行的模型连接 sidecar（`<run_dir>/<team_id>-model.json`）。
//!
//! 为什么需要它：桌面端保存的自定义模型只把密钥放进核心进程环境（`OWO_CUSTOM_MODEL_SECRET_<ref>`），
//! 核心默认 provider（`DeferredProvider`）只读 `OPENAI_*`，因此在只配了桌面模型的机器上
//! 所有团队节点都会以 `provider/not_configured` 失败。单回合（`/session/{id}/turn`）和
//! 工作流草稿已经接受请求级 `model_connection`，团队运行同样需要——但团队是后台运行、
//! 可恢复（cancel/retry/resume 后按迭代重建注册表），所以连接必须落盘。
//!
//! 安全：只持久化**引用**（`api_key_env = OWO_CUSTOM_MODEL_SECRET_<32 hex>`），
//! 明文密钥一律拒绝写入运行记录（仓库红线：凭据只经环境变量注入）。

use owo_agent_protocol::CustomModelConnection;
use serde_json::json;
use std::path::{Path, PathBuf};

/// sidecar 文件路径。
pub(super) fn sidecar_path(run_dir: &Path, team_id: &str) -> PathBuf {
    run_dir.join(format!("{team_id}-model.json"))
}

/// 请求里的连接是否可用于持久化：允许无密钥（本机端点）或环境引用，拒绝明文密钥。
pub(super) fn reject_plaintext_key(connection: &CustomModelConnection) -> Result<(), String> {
    if connection
        .api_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        return Err(
            "model_connection/api_key_env_required：团队运行会持久化模型连接并支持恢复，             明文密钥不能写入运行记录；请改用 api_key_env（OWO_CUSTOM_MODEL_SECRET_<32 位十六进制>）"
                .to_string(),
        );
    }
    Ok(())
}

/// 保存团队模型连接（只存引用；`api_key` 恒为空）。
pub(super) fn save(
    run_dir: &Path,
    team_id: &str,
    connection: &CustomModelConnection,
) -> Result<(), String> {
    reject_plaintext_key(connection)?;
    let mut stored = connection.clone();
    stored.api_key = None;
    let path = sidecar_path(run_dir, team_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建运行目录失败：{error}"))?;
    }
    let text = serde_json::to_string_pretty(&json!({
        "team_id": team_id,
        "connection": stored,
    }))
    .map_err(|error| error.to_string())?;
    std::fs::write(&path, text).map_err(|error| format!("写团队模型连接失败：{error}"))
}

/// 读取团队模型连接。缺失 = 未配置（走核心默认 provider）；格式错误/含密钥 = 硬错误。
pub(super) fn load(run_dir: &Path, team_id: &str) -> Result<Option<CustomModelConnection>, String> {
    let path = sidecar_path(run_dir, team_id);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("团队模型连接不可读：{error}")),
    };
    #[derive(serde::Deserialize)]
    struct Stored {
        #[allow(dead_code)]
        team_id: String,
        connection: CustomModelConnection,
    }
    let stored: Stored =
        serde_json::from_str(&raw).map_err(|error| format!("团队模型连接格式错误：{error}"))?;
    reject_plaintext_key(&stored.connection)?;
    Ok(Some(stored.connection))
}

#[cfg(test)]
mod tests {
    use super::{load, save, sidecar_path};
    use owo_agent_protocol::CustomModelConnection;

    fn connection(api_key: Option<&str>, api_key_env: Option<&str>) -> CustomModelConnection {
        CustomModelConnection {
            model: "fixture-model".to_string(),
            base_url: "http://127.0.0.1:4999/v1".to_string(),
            api_format: "openai".to_string(),
            use_full_url: false,
            api_key: api_key.map(str::to_string),
            api_key_env: api_key_env.map(str::to_string),
            temperature: None,
            timeout_secs: None,
            input_capabilities: None,
        }
    }

    #[test]
    fn plaintext_keys_never_reach_the_run_record() {
        let dir = tempfile::tempdir().unwrap();
        let env = "OWO_CUSTOM_MODEL_SECRET_0123456789abcdef0123456789abcdef";
        let error = save(
            dir.path(),
            "team-a",
            &connection(Some("synthetic-key"), Some(env)),
        )
        .expect_err("明文密钥必须被拒绝");
        assert!(error.contains("api_key_env_required"), "{error}");
        assert!(!sidecar_path(dir.path(), "team-a").exists());
    }

    #[test]
    fn env_reference_roundtrips_and_missing_sidecar_means_default_provider() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), "team-a").unwrap().is_none());
        let env = "OWO_CUSTOM_MODEL_SECRET_0123456789abcdef0123456789abcdef";
        save(dir.path(), "team-a", &connection(None, Some(env))).unwrap();
        let stored = load(dir.path(), "team-a").unwrap().expect("sidecar 应可读");
        assert_eq!(stored.model, "fixture-model");
        assert_eq!(stored.api_key_env.as_deref(), Some(env));
        assert!(stored.api_key.is_none(), "运行记录不得含密钥");
        let raw = std::fs::read_to_string(sidecar_path(dir.path(), "team-a")).unwrap();
        assert!(!raw.contains("synthetic-key"));
        assert!(!raw.contains("api_key\""));
    }
}

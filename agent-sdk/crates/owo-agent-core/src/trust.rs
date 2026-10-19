//! 工作区信任（Workspace Trust，审查 N1）。
//!
//! ## 为什么需要
//!
//! Daemon/CLI 会从**工作区目录**读取并执行一批"配置即代码"的内容：
//! `settings.json` 的 hooks（以系统 shell 执行命令）、工作区插件/`mcp_servers`
//! （启动即拉起子进程）、`permission_profile`/`permission_spec`（可放宽权限）。
//! 用户 `git clone` 一个陌生仓库并把它设为工作区时，这些内容**未经确认就被执行**；
//! VS Code 的 Restricted Mode、Codex/Claude Code 的"信任此目录"都是为同一攻击面。
//!
//! ## 语义
//!
//! - 信任记录放**数据根**（`OWO_AGENT_DATA` 或 `%LOCALAPPDATA%\OwO\Agent`）的
//!   `trust.json`，不在工作区内——仓库自带文件不能给自己授权；
//! - 工作区**有**危险配置（hooks / 工作区插件 / mcp_servers / 放宽档位 / 白名单）
//!   且没有信任记录 → 视为未信任（fail-closed）；
//! - 工作区**没有**危险配置 → 不进入信任门（保持既有行为，避免给普通仓库添噪音）；
//! - 显式撤销（`trusted=false` 记录）后必须显式重新信任；
//! - 配置内容变化（settings/hooks/插件 manifest 的 sha256 指纹变化）→ 记录失效，
//!   需要重新确认（"信任的是这份内容"，不是目录名）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 默认数据根：`OWO_AGENT_DATA` → `%LOCALAPPDATA%\OwO\Agent`（Windows）→ `data/agent`。
pub fn default_data_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("OWO_AGENT_DATA") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("OwO").join("Agent");
    }
    PathBuf::from("data").join("agent")
}

/// 单条信任记录（以 canonical 工作区路径为键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustRecord {
    pub trusted: bool,
    #[serde(default)]
    pub trusted_at: String,
    /// 记录时的配置指纹；内容变化即失效。
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TrustFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    workspaces: BTreeMap<String, TrustRecord>,
}

/// 工作区信任状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStatus {
    /// canonical 工作区路径（作为记录键）。
    pub workspace: String,
    /// 是否已信任这份配置。
    pub trusted: bool,
    /// 当前配置指纹。
    pub fingerprint: String,
    /// 记录里的指纹（未信任过为 null）。
    pub recorded_fingerprint: Option<String>,
    /// 已信任但配置内容已变化（需要重新确认）。
    pub changed: bool,
    /// 未信任时是否应进入受限模式（有危险配置或显式撤销）。
    pub gated: bool,
    /// 检测到的危险配置条目（供 UI 展示"将加载什么"）。
    pub reasons: Vec<String>,
}

/// 工作区信任存储（数据根下 `trust.json`）。
#[derive(Debug, Clone)]
pub struct TrustStore {
    root: PathBuf,
}

impl TrustStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn from_default_root() -> Self {
        Self::new(default_data_root())
    }

    fn path(&self) -> PathBuf {
        self.root.join("trust.json")
    }

    fn load_file(&self) -> TrustFile {
        let Ok(text) = std::fs::read_to_string(self.path()) else {
            return TrustFile::default();
        };
        serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_default()
    }

    fn save_file(&self, file: &TrustFile) -> Result<(), String> {
        if let Some(parent) = self.path().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("创建数据目录失败：{error}"))?;
        }
        let text = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
        let tmp = self.path().with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|error| format!("写入信任记录失败：{error}"))?;
        std::fs::rename(&tmp, self.path()).map_err(|error| format!("替换信任记录失败：{error}"))
    }

    fn key(workspace: &Path) -> String {
        // Windows canonicalize 可能返回 verbatim 形式（`\\?\T:\...`），server 侧
        // workspace 常是普通形式；两侧必须同键，否则 CLI 信任后桌面端读不到记录。
        let canonical = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        let text = canonical.to_string_lossy();
        match text.strip_prefix(r"\\?\") {
            Some(stripped) => stripped.to_string(),
            None => text.into_owned(),
        }
    }

    /// 当前状态（含危险配置扫描与指纹）。
    pub fn status(&self, workspace: &Path) -> TrustStatus {
        let key = Self::key(workspace);
        let file = self.load_file();
        let record = file.workspaces.get(&key).cloned();
        let fingerprint = fingerprint(workspace);
        let reasons = risky_entries(workspace);
        let recorded_fingerprint = record.as_ref().map(|record| record.fingerprint.clone());
        let explicitly_revoked = record.as_ref().is_some_and(|record| !record.trusted);
        let fingerprint_changed = record
            .as_ref()
            .map(|record| record.trusted && record.fingerprint != fingerprint)
            .unwrap_or(false);
        let trusted = record
            .as_ref()
            .map(|record| record.trusted && !fingerprint_changed)
            .unwrap_or(false);
        TrustStatus {
            workspace: key,
            trusted,
            fingerprint: fingerprint.clone(),
            recorded_fingerprint,
            changed: fingerprint_changed,
            gated: !trusted && (explicitly_revoked || !reasons.is_empty()),
            reasons,
        }
    }

    /// 信任当前配置内容（记录当下指纹）。
    pub fn trust(&self, workspace: &Path) -> Result<TrustStatus, String> {
        let key = Self::key(workspace);
        let mut file = self.load_file();
        file.version = 1;
        file.workspaces.insert(
            key,
            TrustRecord {
                trusted: true,
                trusted_at: chrono::Utc::now().to_rfc3339(),
                fingerprint: fingerprint(workspace),
            },
        );
        self.save_file(&file)?;
        Ok(self.status(workspace))
    }

    /// 撤销信任（显式撤销后必须显式重新信任）。
    pub fn revoke(&self, workspace: &Path) -> Result<TrustStatus, String> {
        let key = Self::key(workspace);
        let mut file = self.load_file();
        file.version = 1;
        file.workspaces.insert(
            key,
            TrustRecord {
                trusted: false,
                trusted_at: chrono::Utc::now().to_rfc3339(),
                fingerprint: fingerprint(workspace),
            },
        );
        self.save_file(&file)?;
        Ok(self.status(workspace))
    }
}

/// 工作区里会被自动执行的"配置即代码"清单；空 = 不需要信任门。
fn risky_entries(workspace: &Path) -> Vec<String> {
    let mut reasons = Vec::new();
    let settings = crate::Settings::load(workspace);
    if !settings.hooks.is_empty() {
        reasons.push(format!(
            "settings.hooks：{} 条（启动/回合事件执行命令）",
            settings.hooks.len()
        ));
    }
    if !settings.mcp_servers.is_empty() {
        reasons.push(format!(
            "settings.mcp_servers：{} 个（启动即连接）",
            settings.mcp_servers.len()
        ));
    }
    if let Some(profile) = settings.permission_profile.as_deref() {
        let profile = profile.trim().to_ascii_lowercase();
        if matches!(
            profile.as_str(),
            "full_access" | "unrestricted" | "danger_full_access"
        ) {
            reasons.push(format!(
                "settings.permission_profile：{profile}（可放宽权限）"
            ));
        }
    }
    if settings.permission_spec.is_some() {
        reasons.push("settings.permission_spec：已配置（可放宽权限维度）".to_string());
    }
    if !settings.whitelist.is_empty() {
        reasons.push(format!(
            "settings.whitelist：{} 条（扩大文件/应用范围）",
            settings.whitelist.len()
        ));
    }
    if let Ok(entries) = std::fs::read_dir(workspace.join("plugins")) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.path().join("manifest.json").is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        names.sort();
        if !names.is_empty() {
            let shown = names.iter().take(8).cloned().collect::<Vec<_>>().join("、");
            let suffix = if names.len() > 8 { " 等" } else { "" };
            reasons.push(format!(
                "工作区插件：{shown}{suffix}（可注册工具/拉起 MCP）"
            ));
        }
    }
    reasons
}

/// 配置指纹：settings/hooks 所在文件 + 项目规则 + 插件 manifest 的内容哈希。
///
/// 只做 sha256，不解析内容；文件集合有界（插件 manifest 最多 64 个）。
fn fingerprint(workspace: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for name in ["settings.json", "AGENTS.md", "CLAUDE.md"] {
        let path = workspace.join(name);
        if path.is_file() {
            files.push(path);
        }
    }
    if let Ok(entries) = std::fs::read_dir(workspace.join("plugins")) {
        let mut manifests: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path().join("manifest.json"))
            .filter(|path| path.is_file())
            .collect();
        manifests.sort();
        files.extend(manifests.into_iter().take(64));
    }
    for file in files {
        if let Ok(bytes) = std::fs::read(&file) {
            hasher.update(bytes);
        }
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_workspace_is_not_gated_and_needs_no_confirmation() {
        let dir = std::env::temp_dir().join(format!("owo-trust-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = TrustStore::new(dir.join("data"));
        let status = store.status(&dir);
        assert!(!status.gated, "没有危险配置的仓库不应进入信任门");
        assert!(!status.trusted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hooks_and_plugins_gate_until_trusted_and_content_change() {
        let dir = std::env::temp_dir().join(format!("owo-trust-risky-{}", std::process::id()));
        let plugin = dir.join("plugins").join("demo");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("manifest.json"), "{\"id\":\"demo\"}").unwrap();
        std::fs::write(
            dir.join("settings.json"),
            serde_json::json!({
                "hooks": [{"event": "UserPromptSubmit", "command": "echo hi"}],
            })
            .to_string(),
        )
        .unwrap();
        let store = TrustStore::new(dir.join("data"));
        let status = store.status(&dir);
        assert!(status.gated, "有 hooks/插件的工作区必须先进信任门");
        assert!(!status.trusted);

        let trusted = store.trust(&dir).unwrap();
        assert!(trusted.trusted && !trusted.gated);

        // 内容变化 → 记录失效，回到 gated（信任的是内容而非目录名）。
        std::fs::write(plugin.join("manifest.json"), "{\"id\":\"demo\",\"v\":2}").unwrap();
        let changed = store.status(&dir);
        assert!(changed.changed && changed.gated && !changed.trusted);

        // 显式撤销后即使内容没变也必须重新信任。
        store.trust(&dir).unwrap();
        let revoked = store.revoke(&dir).unwrap();
        assert!(revoked.gated && !revoked.trusted);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

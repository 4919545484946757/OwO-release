//! 社区工作流的导入溯源（谁在什么时候导入了哪个 sha256 的包）。
//!
//! 落盘在 `<data>/workswarm/community-imports.json`（**放在 templates/ 之外**，
//! 避免被模板注册表的 `*.json` 扫描当成模板）。纯 IO + 结构，无业务判定：
//! 导入/删除由调用方决定何时写、何时清。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 单个社区工作流的导入溯源。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommunityProvenance {
    /// 导入包的 sha256（完整性凭据；不落配置正文）。
    pub sha256: String,
    /// 导入时间（RFC3339）。
    pub imported_at: String,
    /// 导入时的 warn 级发现数。
    #[serde(default)]
    pub warnings: usize,
    /// 导入时的 findings 总数。
    #[serde(default)]
    pub findings: usize,
}

/// 溯源文件路径。
pub fn community_imports_path(data_root: &Path) -> PathBuf {
    data_root.join("workswarm").join("community-imports.json")
}

/// 读取溯源表（文件缺失/损坏返回空表，不阻塞列表）。
pub fn load_community_imports(data_root: &Path) -> BTreeMap<String, CommunityProvenance> {
    let path = community_imports_path(data_root);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// 写溯源表（父目录不存在时创建）。
pub fn save_community_imports(
    data_root: &Path,
    imports: &BTreeMap<String, CommunityProvenance>,
) -> std::io::Result<()> {
    let path = community_imports_path(data_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(imports)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    std::fs::write(path, text)
}

/// 来源清单版本（新增字段向后兼容；不兼容变更时提升）。
pub const SOURCES_SCHEMA_VERSION: u32 = 1;

/// 一条"我信任的来源"记录（导出清单里只放身份字段，不放配置）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceEntry {
    pub template_id: String,
    /// 已知良好版本的 sha256。
    pub sha256: String,
    /// 首次导入时间（清单导出时照抄本地记录）。
    #[serde(default)]
    pub imported_at: String,
}

/// 可分享的来源清单（团队内核对"本地社区工作流是否来自可信来源"）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourcesManifest {
    pub schema_version: u32,
    pub kind: String,
    pub exported_at: String,
    pub sources: Vec<SourceEntry>,
}

impl SourcesManifest {
    /// 由本地溯源表生成清单。
    pub fn from_imports(imports: &BTreeMap<String, CommunityProvenance>) -> Self {
        let sources = imports
            .iter()
            .map(|(template_id, provenance)| SourceEntry {
                template_id: template_id.clone(),
                sha256: provenance.sha256.clone(),
                imported_at: provenance.imported_at.clone(),
            })
            .collect();
        Self {
            schema_version: SOURCES_SCHEMA_VERSION,
            kind: "owo-workflow-sources".to_string(),
            exported_at: chrono::Utc::now().to_rfc3339(),
            sources,
        }
    }

    /// 解析清单（kind/schema 校验）。
    pub fn parse(text: &str) -> Result<Self, String> {
        let manifest: Self =
            serde_json::from_str(text).map_err(|error| format!("清单解析失败：{error}"))?;
        if manifest.kind != "owo-workflow-sources" {
            return Err(format!(
                "清单 kind 不是 owo-workflow-sources：{}",
                manifest.kind
            ));
        }
        if manifest.schema_version != SOURCES_SCHEMA_VERSION {
            return Err(format!(
                "清单 schema_version 不兼容：{}（本机支持 {}）",
                manifest.schema_version, SOURCES_SCHEMA_VERSION
            ));
        }
        Ok(manifest)
    }
}

/// 核对结果（清单 vs 本地溯源）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ProvenanceCheck {
    /// 模板 id 与 sha256 都一致。
    pub matched: Vec<String>,
    /// 清单里有、本地没有（未导入或已被删除）。
    pub missing: Vec<String>,
    /// 本地有但 sha256 不一致（本地是另一个版本/来源被替换过）。
    pub mismatched: Vec<(String, String, String)>,
    /// 本地有、清单里没列（本地额外来源，不算失败，但要可见）。
    pub extra: Vec<String>,
}

impl ProvenanceCheck {
    /// 是否存在"不达标"项（缺失或 sha256 不一致）。
    pub fn is_clean(&self) -> bool {
        self.missing.is_empty() && self.mismatched.is_empty()
    }
}

/// 对照清单与本地溯源（按 template_id 匹配，比 sha256）。
pub fn check_against_manifest(
    imports: &BTreeMap<String, CommunityProvenance>,
    manifest: &SourcesManifest,
) -> ProvenanceCheck {
    let mut check = ProvenanceCheck::default();
    for entry in &manifest.sources {
        match imports.get(&entry.template_id) {
            None => check.missing.push(entry.template_id.clone()),
            Some(local) => {
                if local.sha256.eq_ignore_ascii_case(entry.sha256.trim()) {
                    check.matched.push(entry.template_id.clone());
                } else {
                    check.mismatched.push((
                        entry.template_id.clone(),
                        entry.sha256.clone(),
                        local.sha256.clone(),
                    ));
                }
            }
        }
    }
    for template_id in imports.keys() {
        if !manifest
            .sources
            .iter()
            .any(|entry| &entry.template_id == template_id)
        {
            check.extra.push(template_id.clone());
        }
    }
    check
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("owo-prov-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn manifest_export_parse_and_check() {
        let mut imports = BTreeMap::new();
        imports.insert(
            "good".to_string(),
            CommunityProvenance {
                sha256: "aaaa".to_string(),
                imported_at: "2026-10-19T00:00:00+00:00".to_string(),
                warnings: 1,
                findings: 2,
            },
        );
        imports.insert(
            "changed".to_string(),
            CommunityProvenance {
                sha256: "bbbb".to_string(),
                imported_at: "2026-10-19T00:00:00+00:00".to_string(),
                warnings: 0,
                findings: 1,
            },
        );
        let manifest = SourcesManifest::from_imports(&imports);
        assert_eq!(manifest.kind, "owo-workflow-sources");
        assert_eq!(manifest.sources.len(), 2);

        // 清单可解析；坏清单被拒（kind / schema）。
        let text = serde_json::to_string(&manifest).unwrap();
        assert_eq!(SourcesManifest::parse(&text).unwrap().sources.len(), 2);
        let mut wrong_kind: serde_json::Value = serde_json::from_str(&text).unwrap();
        wrong_kind["kind"] = serde_json::json!("owo-workflow");
        assert!(SourcesManifest::parse(&wrong_kind.to_string()).is_err());
        let mut wrong_schema: serde_json::Value = serde_json::from_str(&text).unwrap();
        wrong_schema["schema_version"] = serde_json::json!(99);
        assert!(SourcesManifest::parse(&wrong_schema.to_string()).is_err());

        // 核对：good 一致；changed 的本地 sha 换成别的 → mismatched；补一条清单里有但本地没有的。
        let mut local = imports.clone();
        local.insert(
            "changed".to_string(),
            CommunityProvenance {
                sha256: "cccc".to_string(),
                imported_at: "2026-10-19T00:00:00+00:00".to_string(),
                warnings: 0,
                findings: 1,
            },
        );
        local.insert(
            "extra-local".to_string(),
            CommunityProvenance {
                sha256: "dddd".to_string(),
                imported_at: "2026-10-19T00:00:00+00:00".to_string(),
                warnings: 0,
                findings: 0,
            },
        );
        let mut manifest_with_missing = manifest.clone();
        manifest_with_missing.sources.push(SourceEntry {
            template_id: "not-imported".to_string(),
            sha256: "eeee".to_string(),
            imported_at: String::new(),
        });
        let check = check_against_manifest(&local, &manifest_with_missing);
        assert_eq!(check.matched, vec!["good".to_string()]);
        assert_eq!(check.mismatched.len(), 1);
        assert_eq!(check.mismatched[0].0, "changed");
        assert_eq!(check.missing, vec!["not-imported".to_string()]);
        assert_eq!(check.extra, vec!["extra-local".to_string()]);
        assert!(!check.is_clean());

        // 空清单 + 空本地 → 干净。
        let empty = SourcesManifest::from_imports(&BTreeMap::new());
        assert!(check_against_manifest(&BTreeMap::new(), &empty).is_clean());
    }

    #[test]
    fn round_trips_and_tolerates_missing_or_corrupt_file() {
        let root = temp_root("round-trip");
        assert!(load_community_imports(&root).is_empty(), "缺失时为空的表");

        let mut imports = BTreeMap::new();
        imports.insert(
            "video-recreate".to_string(),
            CommunityProvenance {
                sha256: "abc123".to_string(),
                imported_at: "2026-10-19T00:00:00+00:00".to_string(),
                warnings: 2,
                findings: 3,
            },
        );
        save_community_imports(&root, &imports).unwrap();
        let loaded = load_community_imports(&root);
        assert_eq!(loaded.get("video-recreate"), imports.get("video-recreate"));
        assert!(community_imports_path(&root).exists());

        // 损坏文件按空表处理（不阻塞工作流列表）。
        std::fs::write(community_imports_path(&root), "{ not json").unwrap();
        assert!(load_community_imports(&root).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}

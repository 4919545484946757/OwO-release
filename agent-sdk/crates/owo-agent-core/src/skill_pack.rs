//! 技能包契约（v0.4 D18）：`SKILL.md + manifest.json + assets/ + tests/`。
//!
//! 与插件系统统一：技能 = 指令 + 资源 + 可选工具；权限在 manifest 声明，核心强制。
//! 内置技能包随包分发，安装到 `<data>/skills/<name>` 后与用户流程技能包同构。

use crate::skill::parse_frontmatter;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinSkillManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub permissions: Vec<String>,
    #[serde(default)]
    pub min_app_version: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkillPackageInfo {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
    pub permissions: Vec<String>,
    pub has_tests: bool,
    /// manifest 版本（诊断/展示用）。
    pub version: String,
    /// manifest 声明的最低宿主版本（空串 = 不限制）。
    pub min_app_version: String,
}

/// 一次内置技能安装的结果：装了几份、跳过了哪些（附原因）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillInstallReport {
    pub installed: usize,
    /// `(技能名或目录名, 跳过原因)`——单份坏包不再中断整批安装。
    pub skipped: Vec<(String, String)>,
}

impl SkillInstallReport {
    pub fn is_empty(&self) -> bool {
        self.installed == 0 && self.skipped.is_empty()
    }
}

/// 技能名 → 安全目录名：安装目标固定在 `<data>/skills/<name>`，而 name 来自
/// SKILL.md frontmatter / manifest（用户可直接提供的技能包），因此必须拒绝
/// 路径穿越、盘符、控制字符与 Windows 保留名，避免写到 skills 目录之外或写失败。
pub fn safe_skill_dir_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 {
        return None;
    }
    if trimmed.starts_with('.') || trimmed.ends_with('.') || trimmed.ends_with(' ') {
        return None;
    }
    if trimmed.contains("..") {
        return None;
    }
    for character in trimmed.chars() {
        // Windows 非法字符 + 路径分隔符 + 控制字符。
        if character.is_control() || r#"\/:*?"<>|"#.contains(character) {
            return None;
        }
    }
    let base = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&base.as_str()) {
        return None;
    }
    Some(trimmed.to_string())
}

/// 校验技能包目录结构（契约：SKILL.md frontmatter + manifest.json + tests/）。
pub fn validate_skill_package(dir: &Path) -> Result<SkillPackageInfo, String> {
    let skill_file = dir.join("SKILL.md");
    let content =
        std::fs::read_to_string(&skill_file).map_err(|error| format!("SKILL.md：{error}"))?;
    let (name, description, _) = parse_frontmatter(&content);
    let manifest_path = dir.join("manifest.json");
    let manifest: BuiltinSkillManifest = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .map_err(|error| format!("manifest.json：{error}"))?,
    )
    .map_err(|error| format!("manifest.json 解析失败：{error}"))?;
    if manifest.id.is_empty() || manifest.name.is_empty() || manifest.version.is_empty() {
        return Err("manifest 缺少 id/name/version".to_string());
    }
    if manifest.permissions.is_empty() {
        return Err("manifest.permissions 不能为空（默认 deny）".to_string());
    }
    // 安装目标 = <data>/skills/<name>：frontmatter 名（若声明）与 manifest 名都必须安全。
    let install_name = match name {
        Some(name) => safe_skill_dir_name(&name)
            .ok_or_else(|| format!("SKILL.md name 不是安全目录名：{name}"))?,
        None => manifest.name.clone(),
    };
    if safe_skill_dir_name(&manifest.name).is_none() {
        return Err(format!("manifest.name 不是安全目录名：{}", manifest.name));
    }
    let has_tests = dir.join("tests").is_dir();
    if !has_tests {
        return Err("缺少 tests/ 契约测试目录".to_string());
    }
    Ok(SkillPackageInfo {
        name: install_name,
        path: dir.to_path_buf(),
        description: if description.is_empty() {
            manifest.description.clone()
        } else {
            description
        },
        permissions: manifest.permissions,
        has_tests,
        version: manifest.version,
        min_app_version: manifest.min_app_version,
    })
}

/// 在 [`validate_skill_package`] 之上追加 `min_app_version` 门禁（与插件安装同一语义）：
/// 宿主版本低于技能要求时不得装载，避免半兼容的指令/工具面进入上下文。
pub fn validate_skill_package_for_app(
    dir: &Path,
    app_version: &str,
) -> Result<SkillPackageInfo, String> {
    let info = validate_skill_package(dir)?;
    if !skill_min_app_version_met(&info.min_app_version, app_version) {
        return Err(format!(
            "技能要求宿主 ≥ {}，当前 {}（请升级后再安装）",
            info.min_app_version.trim(),
            app_version.trim()
        ));
    }
    Ok(info)
}

/// `min_app_version` 兼容判定：未声明即兼容。
pub fn skill_min_app_version_met(min_app_version: &str, app_version: &str) -> bool {
    min_app_version.trim().is_empty() || crate::plugin::version_gte(app_version, min_app_version)
}

/// 发现目录下合法的内置技能包（按 `app_version` 过滤 `min_app_version` 不满足者）。
pub fn discover_builtin_packages(root: &Path, app_version: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut packages = Vec::new();
    for entry in entries.flatten() {
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        if is_dir
            && entry.path().join("SKILL.md").exists()
            && entry.path().join("manifest.json").exists()
            && validate_skill_package_for_app(&entry.path(), app_version).is_ok()
        {
            packages.push(entry.path());
        }
    }
    packages.sort();
    packages
}

/// 把内置技能包安装到 `<data_root>/skills/`（已存在则跳过）。
///
/// 单份包不合格（结构/名字/版本不满足）只记入 `skipped`，不中断整批安装——
/// 一份坏包不该让其余技能都装不上。
pub fn install_builtin_packages(
    builtin_root: &Path,
    data_root: &Path,
    app_version: &str,
) -> Result<SkillInstallReport, String> {
    let mut report = SkillInstallReport::default();
    let entries = std::fs::read_dir(builtin_root).map_err(|error| error.to_string())?;
    let mut directories: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect();
    directories.sort();
    for package in directories {
        let label = package
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| package.display().to_string());
        if !package.join("SKILL.md").exists() || !package.join("manifest.json").exists() {
            continue; // 非技能包目录（asset/临时文件）不算失败。
        }
        let info = match validate_skill_package_for_app(&package, app_version) {
            Ok(info) => info,
            Err(reason) => {
                report.skipped.push((label, reason));
                continue;
            }
        };
        let target = data_root.join("skills").join(&info.name);
        if target.exists() {
            continue;
        }
        if let Err(error) = copy_dir(&package, &target) {
            report
                .skipped
                .push((info.name, format!("复制失败：{error}")));
            continue;
        }
        report.installed += 1;
    }
    Ok(report)
}

fn copy_dir(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|error| error.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&from, &to)?;
        } else if file_type.is_file() {
            std::fs::copy(&from, &to).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_package(root: &Path, name: &str) -> PathBuf {
        write_package_with(root, name, "1.0.0", "0.4.0")
    }

    fn write_package_with(
        root: &Path,
        name: &str,
        version: &str,
        min_app_version: &str,
    ) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: 测试技能\n---\n执行步骤。"),
        )
        .unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&BuiltinSkillManifest {
                id: format!("com.example.{name}"),
                name: name.to_string(),
                version: version.to_string(),
                description: "测试技能".to_string(),
                permissions: vec!["files:read".to_string()],
                min_app_version: min_app_version.to_string(),
                tools: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("tests").join("case-1.md"), "# 用例 1").unwrap();
        dir
    }

    #[test]
    fn validates_package_contract() {
        let root = std::env::temp_dir().join(format!("owo-pack-{}", uuid::Uuid::new_v4()));
        let dir = write_package(&root, "documents");
        let info = validate_skill_package(&dir).unwrap();
        assert_eq!(info.name, "documents");
        assert!(info.has_tests);
        assert_eq!(info.permissions, vec!["files:read"]);
        assert_eq!(info.version, "1.0.0");
        assert_eq!(info.min_app_version, "0.4.0");

        std::fs::remove_file(dir.join("manifest.json")).unwrap();
        assert!(validate_skill_package(&dir).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unsafe_directory_names_are_rejected() {
        // 合法名（含中文与点号分隔）通过。
        assert_eq!(
            safe_skill_dir_name("documents").as_deref(),
            Some("documents")
        );
        assert_eq!(
            safe_skill_dir_name(" 数据-整理 ").as_deref(),
            Some("数据-整理")
        );
        assert_eq!(
            safe_skill_dir_name("com.owo.skill").as_deref(),
            Some("com.owo.skill")
        );
        // 首尾空白按 trim 归一（落到磁盘上的名字不含尾空格）。
        assert_eq!(
            safe_skill_dir_name(" trailing ").as_deref(),
            Some("trailing")
        );
        // 路径穿越 / 分隔符 / 盘符 / 保留名 / 隐藏名 / 控制字符 / 过长。
        for bad in [
            "../escape",
            "..\\escape",
            "a/b",
            "a\\b",
            "C:evil",
            "con",
            "NUL.md",
            ".hidden",
            "trailing.",
            "with\0nul",
            &"x".repeat(65),
            "",
            "   ",
        ] {
            assert!(
                safe_skill_dir_name(bad).is_none(),
                "应拒绝不安全目录名：{bad:?}"
            );
        }
    }

    #[test]
    fn min_app_version_gate_blocks_install_only_when_required() {
        assert!(skill_min_app_version_met("", "1.0.0"));
        assert!(skill_min_app_version_met("0.5.0", "0.5.0"));
        assert!(skill_min_app_version_met("0.5.0", "1.2.0"));
        assert!(!skill_min_app_version_met("9.0.0", "0.5.8"));

        let root = std::env::temp_dir().join(format!("owo-pack-gate-{}", uuid::Uuid::new_v4()));
        let dir = write_package_with(&root, "future", "2.0.0", "9.0.0");
        assert!(validate_skill_package(&dir).is_ok());
        let error = validate_skill_package_for_app(&dir, "0.5.8").unwrap_err();
        assert!(error.contains("技能要求宿主 ≥ 9.0.0"), "{error}");
        assert!(validate_skill_package_for_app(&dir, "9.1.0").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn traversal_skill_name_cannot_escape_or_install() {
        let root = std::env::temp_dir().join(format!("owo-pack-evil-{}", uuid::Uuid::new_v4()));
        // 包目录本身在 `pkgs/` 内，但 manifest/frontmatter 声明的名字试图穿越到 `pkgs/..`。
        let packages = root.join("pkgs");
        let dir = packages.join("evil");
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: ../escape\ndescription: 越权\n---\n执行步骤。",
        )
        .unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            r#"{"id":"com.example.evil","name":"../escape","version":"1.0.0","description":"x","permissions":["files:read"]}"#,
        )
        .unwrap();
        let error = validate_skill_package(&dir).unwrap_err();
        assert!(error.contains("不是安全目录名"), "{error}");

        let data = root.join("data");
        let report = install_builtin_packages(&packages, &data, "1.0.0").unwrap();
        assert_eq!(report.installed, 0);
        assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
        assert!(report.skipped[0].1.contains("不是安全目录名"));
        // skills 目录内不得出现任何落盘，穿越目标（pkgs/../escape = root/escape）也不得出现。
        let installed: Vec<_> = std::fs::read_dir(data.join("skills"))
            .map(|entries| entries.flatten().collect())
            .unwrap_or_default();
        assert!(installed.is_empty(), "{installed:?}");
        assert!(!root.join("escape").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discovers_and_installs_builtin_packages() {
        let root = std::env::temp_dir().join(format!("owo-pack-root-{}", uuid::Uuid::new_v4()));
        write_package(&root, "documents");
        write_package(&root, "pdf");
        assert_eq!(discover_builtin_packages(&root, "1.0.0").len(), 2);

        let data = root.join("data");
        let report = install_builtin_packages(&root, &data, "1.0.0").unwrap();
        assert_eq!(report.installed, 2);
        assert!(report.skipped.is_empty());
        assert!(data.join("skills/documents/SKILL.md").exists());
        assert!(data.join("skills/pdf/manifest.json").exists());
        assert_eq!(
            install_builtin_packages(&root, &data, "1.0.0")
                .unwrap()
                .installed,
            0
        ); // 已存在跳过
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn one_bad_package_does_not_block_the_others() {
        let root = std::env::temp_dir().join(format!("owo-pack-mixed-{}", uuid::Uuid::new_v4()));
        write_package(&root, "good");
        write_package_with(&root, "future", "2.0.0", "9.0.0"); // 版本不满足
        let broken = root.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("SKILL.md"), "---\nname: broken\n---\n正文").unwrap();
        std::fs::write(broken.join("manifest.json"), "{ not json").unwrap();

        assert_eq!(discover_builtin_packages(&root, "1.0.0").len(), 1);
        let data = root.join("data");
        let report = install_builtin_packages(&root, &data, "1.0.0").unwrap();
        assert_eq!(report.installed, 1);
        assert!(data.join("skills/good/SKILL.md").exists());
        assert!(!data.join("skills/future").exists());
        assert!(!data.join("skills/broken").exists());
        let reasons = report
            .skipped
            .iter()
            .map(|(name, reason)| format!("{name}:{reason}"))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(reasons.contains("future"), "{reasons}");
        assert!(reasons.contains("broken"), "{reasons}");
        // 高版本宿主可以装上 future 包。
        let report = install_builtin_packages(&root, &data, "9.2.0").unwrap();
        assert_eq!(report.installed, 1);
        assert!(data.join("skills/future/manifest.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

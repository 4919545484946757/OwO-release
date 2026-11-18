//! 测试替身 MockBackend：内存/文件动作后端（写文件、追加、感知、通知记录）。

use super::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// 测试替身：MockBackend
// ---------------------------------------------------------------------------

/// 内存/文件动作后端（契约测试用）：写文件、追加、感知文件列表、通知记录。
pub struct MockBackend {
    pub root: PathBuf,
    pub log: Vec<String>,
    pub sense_results: HashMap<String, serde_json::Value>,
    pub locate_results: HashMap<String, serde_json::Value>,
    pub skill_results: HashMap<String, Result<serde_json::Value, String>>,
    pub mcp_results: HashMap<String, Result<serde_json::Value, String>>,
    pub notifications: Vec<String>,
    pub fail_acts: Vec<String>,
}

impl MockBackend {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            log: Vec::new(),
            sense_results: HashMap::new(),
            locate_results: HashMap::new(),
            skill_results: HashMap::new(),
            mcp_results: HashMap::new(),
            notifications: Vec::new(),
            fail_acts: Vec::new(),
        }
    }

    pub fn file_content(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }
}

#[async_trait::async_trait]
impl ActionBackend for MockBackend {
    async fn sense(&mut self, spec: &SenseSpec) -> Result<serde_json::Value, String> {
        self.log.push(format!("sense:{}", spec.target));
        if let Some(value) = self.sense_results.get(&spec.target) {
            return Ok(value.clone());
        }
        if spec.target == "files" {
            let mut files = Vec::new();
            let root = self.root.clone();
            walk_files(&root, &root, &mut files)?;
            return Ok(serde_json::json!({ "files": files }));
        }
        if spec.target == "clipboard" {
            return Ok(
                serde_json::json!({ "text": self.file_content("clipboard.txt").unwrap_or_default() }),
            );
        }
        Err(format!("未配置的感知目标：{}", spec.target))
    }

    async fn locate(&mut self, spec: &LocateSpec) -> Result<serde_json::Value, String> {
        self.log.push(format!("locate:{}", spec.target));
        self.locate_results
            .get(&spec.target)
            .cloned()
            .ok_or_else(|| format!("定位失败：{}", spec.target))
    }

    async fn act(&mut self, spec: &ActSpec) -> Result<serde_json::Value, String> {
        self.log
            .push(format!("act:{}:{}", spec.action, spec.target));
        if self.fail_acts.iter().any(|a| a == &spec.action) {
            return Err(format!("动作失败（测试注入）：{}", spec.action));
        }
        match spec.action.as_str() {
            "write_file" => {
                let path = self.root.join(&spec.target);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, spec.value.clone().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "written": spec.target }))
            }
            "append_file" => {
                let path = self.root.join(&spec.target);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut content = std::fs::read_to_string(&path).unwrap_or_default();
                content.push_str(&spec.value.clone().unwrap_or_default());
                std::fs::write(&path, content).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "appended": spec.target }))
            }
            "send_message" => {
                let mut content =
                    std::fs::read_to_string(self.root.join("messages.log")).unwrap_or_default();
                content.push_str(&format!(
                    "{}: {}\n",
                    spec.target,
                    spec.value.clone().unwrap_or_default()
                ));
                std::fs::write(self.root.join("messages.log"), content)
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "sent": spec.target }))
            }
            other => Err(format!("未知动作：{other}")),
        }
    }

    async fn invoke_skill(
        &mut self,
        skill: &str,
        args: &BTreeMap<String, String>,
    ) -> Result<serde_json::Value, String> {
        self.log.push(format!("skill:{}", skill));
        self.skill_results
            .get(skill)
            .cloned()
            .unwrap_or_else(|| Ok(serde_json::json!({ "skill": skill, "args": args })))
    }

    async fn invoke_mcp(
        &mut self,
        server: &str,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, (String, bool)> {
        self.log.push(format!("mcp:{}/{}", server, tool));
        let key = format!("{server}/{tool}");
        self.mcp_results
            .get(&key)
            .cloned()
            .unwrap_or_else(|| {
                Ok(serde_json::json!({ "server": server, "tool": tool, "args": args }))
            })
            .map_err(|error| (error, false))
    }

    async fn notify(&mut self, message: &str) -> Result<(), String> {
        self.notifications.push(message.to_string());
        self.log.push(format!("notify:{message}"));
        Ok(())
    }
}

fn walk_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|_| "路径越界".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if path
            .file_name()
            .map(|n| n == ".wf-checkpoints")
            .unwrap_or(false)
        {
            continue;
        }
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            walk_files(root, &path, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

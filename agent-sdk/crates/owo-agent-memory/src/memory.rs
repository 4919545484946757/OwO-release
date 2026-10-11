//! 语义记忆三层（v0.5 M-C，对应技术文档 5.8.3）。
//!
//! 把情景观察（`observe::Observation`）沉淀为带结果判定（`Outcome`）与归一化动作序列的
//! 记忆条目，提供 `recall` 语义检索（本地无外部依赖实现，M-C 可升级为本地 embedding）。

use crate::observe::Observation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// 结果判定：操作是否成功。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Failure,
    #[default]
    Unknown,
}

/// 记忆条目：观察 + 结果 + 归一化动作序列。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub ts: String,
    pub app_id: String,
    pub summary: String,
    pub detail: serde_json::Value,
    #[serde(default)]
    pub outcome: Outcome,
    #[serde(default)]
    pub normalized: Vec<String>,
    #[serde(default = "default_memory_confidence")]
    pub confidence: f64,
}

fn default_memory_confidence() -> f64 {
    0.5
}

/// 原子落盘：同目录临时文件 + rename 覆盖，避免崩溃留下半写 JSON。
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("memory"),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, bytes).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        error.to_string()
    })?;
    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        error.to_string()
    })
}

impl MemoryEntry {
    pub fn from_observation(observation: &Observation) -> Self {
        let normalized = normalize_summary(&observation.summary);
        Self {
            ts: observation.ts.clone(),
            app_id: observation.app_id.clone(),
            summary: observation.summary.clone(),
            detail: observation.detail.clone(),
            outcome: Outcome::Unknown,
            normalized,
            confidence: 0.5,
        }
    }
}

/// 本地语义记忆：条目 + 词项倒排索引（无外部依赖的轻量召回）。
#[derive(Debug, Clone, Default)]
pub struct SemanticMemory {
    entries: Vec<MemoryEntry>,
    index: HashMap<String, Vec<usize>>,
}

impl SemanticMemory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, entry: MemoryEntry) {
        let idx = self.entries.len();
        for token in entry.normalized.iter() {
            self.index.entry(token.clone()).or_default().push(idx);
        }
        self.entries.push(entry);
    }

    pub fn add_observation(&mut self, observation: &Observation) {
        self.add(MemoryEntry::from_observation(observation));
    }

    /// 按 ts+app_id+summary 标记结果；命中返回 true。
    pub fn mark_outcome(
        &mut self,
        ts: &str,
        app_id: &str,
        summary: &str,
        outcome: Outcome,
    ) -> bool {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.ts == ts && entry.app_id == app_id && entry.summary == summary)
        else {
            return false;
        };
        entry.outcome = outcome;
        true
    }

    /// 容量上限：淘汰最旧条目并重建索引。
    pub fn prune(&mut self, max_entries: usize) {
        if max_entries == 0 || self.entries.len() <= max_entries {
            return;
        }
        let overflow = self.entries.len() - max_entries;
        self.entries.drain(0..overflow);
        self.rebuild_index();
    }

    /// 持久化全部条目（JSON 数组）。原子落盘：崩溃不会留下半写文件导致整库丢失。
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(&self.entries).map_err(|error| error.to_string())?;
        atomic_write(path, json.as_bytes())
    }

    /// 从持久化文件加载并重建索引；文件缺失/损坏时静默为空。
    /// 兼容 JSON 数组与 JSONL 两种格式。
    pub fn load_from(&mut self, path: &Path) {
        let Ok(content) = std::fs::read_to_string(path) else {
            return;
        };
        let entries: Vec<MemoryEntry> = serde_json::from_str(&content)
            .ok()
            .or_else(|| {
                Some(
                    content
                        .lines()
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect(),
                )
            })
            .unwrap_or_default();
        self.entries = entries;
        self.rebuild_index();
    }

    pub fn recall(&self, query: &str, top_k: usize) -> Vec<MemoryEntry> {
        let tokens = tokenize(query);
        let mut scores: HashMap<usize, f64> = HashMap::new();
        for token in tokens {
            if let Some(ids) = self.index.get(&token) {
                for &id in ids {
                    *scores.entry(id).or_insert(0.0) += 1.0;
                }
            }
        }
        let mut ranked: Vec<(usize, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
            .into_iter()
            .take(top_k)
            .filter_map(|(id, _)| self.entries.get(id).cloned())
            .collect()
    }

    pub fn entries(&self) -> &[MemoryEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for (idx, entry) in self.entries.iter().enumerate() {
            let tokens = if entry.normalized.is_empty() {
                normalize_summary(&entry.summary)
            } else {
                entry.normalized.clone()
            };
            for token in tokens {
                self.index.entry(token).or_default().push(idx);
            }
        }
    }
}

/// 归一化：摘要拆词 + 数字/内容掩码占位符统一。
pub fn normalize_summary(summary: &str) -> Vec<String> {
    tokenize(summary)
}

fn tokenize(text: &str) -> Vec<String> {
    let tokens = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|token| !token.is_empty())
        .map(|token| {
            if token.chars().all(|c| c.is_ascii_digit()) {
                "{num}".to_string()
            } else {
                token.to_lowercase()
            }
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    for token in tokens {
        // 中文无空格分词：补充二元字符组，保证 recall 可按子串命中。
        let chars: Vec<char> = token.chars().collect();
        if chars.len() > 1 && chars.iter().any(|c| c.is_alphabetic() && !c.is_ascii()) {
            for pair in chars.windows(2) {
                output.push(pair.iter().collect());
            }
        } else {
            output.push(token);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(summary: &str) -> Observation {
        Observation {
            ts: chrono::Utc::now().to_rfc3339(),
            app_id: "qq".to_string(),
            kind: "action".to_string(),
            summary: summary.to_string(),
            detail: serde_json::json!({}),
            state_hash: 0,
        }
    }

    #[test]
    fn recall_finds_similar_observations() {
        let mut memory = SemanticMemory::new();
        memory.add_observation(&observation("点击发送按钮"));
        memory.add_observation(&observation("输入消息内容"));
        let hits = memory.recall("发送按钮", 2);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].summary.contains("发送"));
    }

    #[test]
    fn numbers_normalize_to_placeholder() {
        assert_eq!(
            normalize_summary("金额 123 元"),
            vec!["金额", "{num}", "元"]
        );
    }

    #[test]
    fn save_to_is_atomic_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("owo-memory-{}", uuid::Uuid::new_v4()));
        let path = dir.join("semantic.json");
        let mut memory = SemanticMemory::new();
        memory.add_observation(&observation("点击发送按钮"));
        memory.save_to(&path).unwrap();
        let mut loaded = SemanticMemory::new();
        loaded.load_from(&path);
        assert_eq!(loaded.recall("发送按钮", 2).len(), 1);

        // 覆盖写不残留临时文件。
        memory.save_to(&path).unwrap();
        let residue: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(residue.is_empty(), "不应残留临时文件：{residue:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

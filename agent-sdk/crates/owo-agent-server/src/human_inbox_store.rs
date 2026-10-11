//! Human Inbox 持久化存储（八期 · 第三路：统一 Human Inbox 后端与直接处理；
//! 九期 · 第二路：主键升级为 `kind:team_id:target_id:occurrence`）。
//!
//! 统一四类待办的**覆盖层（overlay）记录**：领域对象本身继续由各自存储持有
//! （团队运行/任务视图 = space.db + run_dir 状态文件；产物评审 = space.db；
//! ChangeSet = 八期二路 store），本存储只持久化「领取/解决」的人工协作状态：
//! - 同一待办只能被一个用户领取（进程内 Mutex CAS + 共享单例注册表）；
//! - 服务重启后待办协作状态仍能恢复（JSON 文件 + 原子写 tmp→rename）；
//! - 已解决待办不会重新出现（resolved 记录按主键幂等保留）；
//! - resolve 采用「CAS 占位 → 分派 → 终态」三段式，重放同键返回缓存结果、
//!   不产生重复 Artifact / 重复写入 / 重复 retry（分派前占位是幂等的关键）。
//!
//! 主键（九期）：`kind:team_id:target_id:occurrence`——team 入键修复「两个团队
//! 使用相同 step_id 时互相覆盖」；occurrence 为发生版本，修复「重试轮次被旧
//! resolved 记录永久吞掉」（retry 会把 attempts 重置为 0，故 resolved 占用首选
//! 版本时由 [`HumanInboxStore::ensure_item`] 自动分配下一空闲版本）。八期旧键
//! `kind:target_id` 在加载时按记录自身字段兼容迁移（保留协作状态）。
//!
//! 存储规模：人工待办为低频小数据（数十条），JSON 文件足够；并发正确性由
//! 进程内共享单例（[`open_shared`]）+ Mutex 保证（单服务进程部署形态）。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// 常量：kind 与 status（与 wire 契约一致的小写下划线字符串）
// ---------------------------------------------------------------------------

pub const KIND_HUMAN_RESULT: &str = "human_result";
pub const KIND_ARTIFACT_REVIEW: &str = "artifact_review";
pub const KIND_CHANGE_SET: &str = "change_set";
pub const KIND_STEP_RETRY: &str = "step_retry";

pub const STATUS_OPEN: &str = "open";
pub const STATUS_CLAIMED: &str = "claimed";
pub const STATUS_RESOLVING: &str = "resolving";
pub const STATUS_RESOLVED: &str = "resolved";

pub fn is_valid_kind(kind: &str) -> bool {
    matches!(
        kind,
        KIND_HUMAN_RESULT | KIND_ARTIFACT_REVIEW | KIND_CHANGE_SET | KIND_STEP_RETRY
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 统一 Human 待办（wire 形状；八期冻结字段 + 协作字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanWorkItem {
    /// `"<kind>:<team_id>:<target_id>:<occurrence>"`（九期主键；路径段安全：不含 `/`）。
    pub item_id: String,
    pub kind: String,
    pub team_id: String,
    /// 产物/变更类待办的项目空间 id；团队任务类可为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// 领域对象 id（step_id / artifact_id / change_set_id）。
    pub target_id: String,
    /// open | claimed | resolving | resolved。
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub summary: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,
    /// resolve 幂等键（缺省由 API 层派生）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// resolve 成功结果缓存（同键重放返回，不再分派）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// 最近一次分派失败原因（分派失败回滚到原状态时记录）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// 扫描源产生的待办草稿（live 数据；不含协作状态）。
#[derive(Debug, Clone)]
pub struct InboxItemDraft {
    pub kind: String,
    pub team_id: String,
    pub project_id: Option<String>,
    pub target_id: String,
    /// 发生版本（九期）：同 `(kind, team_id, target_id)` 的第几次出现。
    /// step_retry / human_result 首选 attempts；artifact_review / change_set 的
    /// target_id 全局唯一，恒为 "1"。resolved 占用首选版本时自动递增分配。
    pub occurrence: String,
    pub summary: String,
}

impl InboxItemDraft {
    /// 主键基座（不含发生版本）：`kind:team_id:target_id`。
    pub fn base_key(&self) -> String {
        format!("{}:{}:{}", self.kind, self.team_id, self.target_id)
    }

    /// 主键（九期）：`kind:team_id:target_id:occurrence`。
    pub fn item_id(&self) -> String {
        format!("{}:{}", self.base_key(), self.occurrence)
    }
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// claim/release 失败语义。
#[derive(Debug)]
pub enum ClaimError {
    NotFound,
    /// 已被其他用户领取（或释放者不是领取者）。
    Conflict {
        by: Option<String>,
    },
}

/// begin_resolve 失败/幂等语义。
#[derive(Debug)]
pub enum ResolveLeaseError {
    NotFound,
    /// 已按相同幂等键解决 → 返回缓存结果（重放，不分派）。
    AlreadyDone {
        result: Option<Value>,
    },
    /// 已按不同幂等键解决 → 拒绝（同一待办不允许两种解决）。
    KeyConflict {
        existing: Option<String>,
    },
    /// 正在解决中（并发重放窗口）。
    InProgress,
}

// ---------------------------------------------------------------------------
// 存储
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct InboxFile {
    #[serde(default)]
    items: HashMap<String, HumanWorkItem>,
}

/// 八期旧主键兼容迁移（九期）：`item_id == "{kind}:{target_id}"`（无 team/occurrence）
/// 的记录按其自身字段重键为 `kind:team_id:target_id:1`——claimed/resolved/幂等结果
/// 全部保留，ensure_item 的首选版本恰好命中迁移键，历史状态无缝延续。新键已被
/// 占用时保留新记录（由新扫描产生，是权威）；team 缺失无法定位的旧记录原样保留。
fn migrate_legacy_keys(file: &mut InboxFile) {
    let legacy: Vec<String> = file
        .items
        .iter()
        .filter(|(_, item)| {
            !item.team_id.is_empty() && item.item_id == format!("{}:{}", item.kind, item.target_id)
        })
        .map(|(key, _)| key.clone())
        .collect();
    for old_id in legacy {
        let Some(mut item) = file.items.remove(&old_id) else {
            continue;
        };
        let new_id = format!("{}:{}:{}:1", item.kind, item.team_id, item.target_id);
        item.item_id = new_id.clone();
        if file.items.contains_key(&new_id) {
            continue;
        }
        file.items.insert(new_id, item);
    }
}

pub struct HumanInboxStore {
    path: PathBuf,
    inner: Mutex<InboxFile>,
}

impl HumanInboxStore {
    /// 打开（不加载）——一般经 [`open_shared`] 取进程内单例。
    fn new(path: PathBuf) -> Self {
        let initial = Self::load_file(&path);
        Self {
            path,
            inner: Mutex::new(initial),
        }
    }

    fn load_file(path: &Path) -> InboxFile {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return InboxFile::default();
        };
        // 损坏文件 → 备份旁置并从空开始（人工待办可由 live 扫描重建，不阻塞服务）。
        match serde_json::from_str::<InboxFile>(&raw) {
            Ok(mut file) => {
                migrate_legacy_keys(&mut file);
                file
            }
            Err(e) => {
                let backup = path.with_extension("json.corrupt");
                let _ = std::fs::rename(path, &backup);
                eprintln!(
                    "[human-inbox] 状态文件损坏（已备份至 {}）从空重建：{e}",
                    backup.display()
                );
                InboxFile::default()
            }
        }
    }

    /// 原子写（tmp → rename）。
    fn persist(&self, file: &InboxFile) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = self.path.with_extension("json.tmp");
        match serde_json::to_string_pretty(file) {
            Ok(json) => {
                if std::fs::write(&tmp, json).is_ok() {
                    let _ = std::fs::rename(&tmp, &self.path);
                }
            }
            Err(_) => { /* 序列化失败不落盘：内存态仍正确，下次变更重试 */ }
        }
    }

    /// 幂等登记：不存在则创建（open）；已存在则仅刷新 summary（绝不复活已解决项）。
    /// 无变化时不落盘（列表扫描每次调用本方法，避免无谓写放大）。
    ///
    /// 发生版本分配（九期）：首选 occurrence 键未被占用或未解决 → 直接复用；
    /// 已被 resolved 记录占用 → 递增分配下一个空闲版本。由此同一目标的每个
    /// 「出现」（重试轮次）都得到独立待办，不被旧 resolved 记录永久吞掉；
    /// 首选版本仍是 open/claimed 时复用原记录，不产生重复条目。
    pub fn ensure_item(&self, draft: &InboxItemDraft) -> HumanWorkItem {
        let mut file = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let base = draft.base_key();
        let mut item_id = draft.item_id();
        if file
            .items
            .get(&item_id)
            .is_some_and(|existing| existing.status == STATUS_RESOLVED)
        {
            let mut n = draft.occurrence.parse::<u64>().unwrap_or(1);
            loop {
                n += 1;
                item_id = format!("{base}:{n}");
                if !file
                    .items
                    .get(&item_id)
                    .is_some_and(|existing| existing.status == STATUS_RESOLVED)
                {
                    break;
                }
            }
        }
        if let Some(existing) = file.items.get_mut(&item_id) {
            let needs_backfill = (existing.team_id.is_empty() && !draft.team_id.is_empty())
                || (existing.project_id.is_none() && draft.project_id.is_some());
            let unchanged = existing.summary == draft.summary && !needs_backfill;
            existing.summary = draft.summary.clone();
            if existing.team_id.is_empty() {
                existing.team_id = draft.team_id.clone();
            }
            if existing.project_id.is_none() {
                existing.project_id = draft.project_id.clone();
            }
            if unchanged {
                return existing.clone(); // 无变化 → 跳过落盘
            }
            let snapshot = existing.clone();
            self.persist(&file);
            return snapshot;
        }
        let item = HumanWorkItem {
            item_id: item_id.clone(),
            kind: draft.kind.clone(),
            team_id: draft.team_id.clone(),
            project_id: draft.project_id.clone(),
            target_id: draft.target_id.clone(),
            status: STATUS_OPEN.to_string(),
            assignee: None,
            summary: draft.summary.clone(),
            created_at: now_ms(),
            claimed_at: None,
            resolved_at: None,
            idempotency_key: None,
            result: None,
            last_error: None,
        };
        file.items.insert(item_id, item.clone());
        self.persist(&file);
        item
    }

    /// 领取：open → claimed（CAS）；同人重复领取幂等；他人已领取 → Conflict。
    pub fn claim(&self, item_id: &str, user: &str) -> Result<HumanWorkItem, ClaimError> {
        let mut file = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let item = file.items.get_mut(item_id).ok_or(ClaimError::NotFound)?;
        match item.status.as_str() {
            STATUS_RESOLVED | STATUS_RESOLVING => {
                return Err(ClaimError::Conflict {
                    by: item.assignee.clone(),
                })
            }
            STATUS_CLAIMED => {
                if item.assignee.as_deref() == Some(user) {
                    let snapshot = item.clone();
                    return Ok(snapshot); // 同人重复领取：幂等
                }
                return Err(ClaimError::Conflict {
                    by: item.assignee.clone(),
                });
            }
            _ => {}
        }
        item.status = STATUS_CLAIMED.to_string();
        item.assignee = Some(user.to_string());
        item.claimed_at = Some(now_ms());
        let snapshot = item.clone();
        self.persist(&file);
        Ok(snapshot)
    }

    /// 释放：claimed → open（仅领取者可释放）。
    pub fn release(&self, item_id: &str, user: &str) -> Result<HumanWorkItem, ClaimError> {
        let mut file = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let item = file.items.get_mut(item_id).ok_or(ClaimError::NotFound)?;
        if item.status != STATUS_CLAIMED {
            return Err(ClaimError::Conflict {
                by: item.assignee.clone(),
            });
        }
        if item.assignee.as_deref() != Some(user) {
            return Err(ClaimError::Conflict {
                by: item.assignee.clone(),
            });
        }
        item.status = STATUS_OPEN.to_string();
        item.assignee = None;
        item.claimed_at = None;
        let snapshot = item.clone();
        self.persist(&file);
        Ok(snapshot)
    }

    /// resolve 第一步（CAS 占位）：open/claimed → resolving。
    /// - 已 resolved 且同键 → `AlreadyDone`（重放：返回缓存结果，调用方不得再分派）；
    /// - 已 resolved 且异键 → `KeyConflict`；
    /// - resolving → `InProgress`（并发窗口）。
    pub fn begin_resolve(
        &self,
        item_id: &str,
        idempotency_key: &str,
    ) -> Result<String, ResolveLeaseError> {
        let mut file = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let item = file
            .items
            .get_mut(item_id)
            .ok_or(ResolveLeaseError::NotFound)?;
        match item.status.as_str() {
            STATUS_RESOLVED => {
                if item.idempotency_key.as_deref() == Some(idempotency_key) {
                    return Err(ResolveLeaseError::AlreadyDone {
                        result: item.result.clone(),
                    });
                }
                return Err(ResolveLeaseError::KeyConflict {
                    existing: item.idempotency_key.clone(),
                });
            }
            STATUS_RESOLVING => return Err(ResolveLeaseError::InProgress),
            _ => {}
        }
        let prev = item.status.clone();
        item.status = STATUS_RESOLVING.to_string();
        item.idempotency_key = Some(idempotency_key.to_string());
        self.persist(&file);
        Ok(prev)
    }

    /// resolve 成功终态：resolving → resolved（缓存结果）。
    pub fn finish_resolve(&self, item_id: &str, result: Value) -> Option<HumanWorkItem> {
        let mut file = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let item = file.items.get_mut(item_id)?;
        if item.status != STATUS_RESOLVING {
            return None;
        }
        item.status = STATUS_RESOLVED.to_string();
        item.resolved_at = Some(now_ms());
        item.result = Some(result);
        let snapshot = item.clone();
        self.persist(&file);
        Some(snapshot)
    }

    /// resolve 分派失败：resolving → 原状态（open/claimed），记录错误可重试。
    pub fn abort_resolve(&self, item_id: &str, prev_status: &str, error: &str) {
        let mut file = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(item) = file.items.get_mut(item_id) {
            if item.status == STATUS_RESOLVING {
                item.status = prev_status.to_string();
                item.last_error = Some(error.to_string());
                self.persist(&file);
            }
        }
    }

    pub fn get(&self, item_id: &str) -> Option<HumanWorkItem> {
        self.inner
            .lock()
            .expect("human inbox lock")
            .items
            .get(item_id)
            .cloned()
    }

    /// resolved 记录数（自检/统计；当前仅测试消费，保留为公开自省 API）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .items
            .len()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// 进程内共享单例（并发 CAS 正确性的关键：同一路径全进程一个实例）
// ---------------------------------------------------------------------------

fn shared_registry() -> &'static Mutex<HashMap<PathBuf, Arc<HumanInboxStore>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<HumanInboxStore>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 取（并懒建）路径对应的进程内共享存储实例。
pub fn open_shared(path: &Path) -> Arc<HumanInboxStore> {
    let mut registry = shared_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(HumanInboxStore::new(path.to_path_buf())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn draft(target: &str) -> InboxItemDraft {
        InboxItemDraft {
            kind: KIND_STEP_RETRY.to_string(),
            team_id: "team-x".to_string(),
            project_id: Some("ps-x".to_string()),
            target_id: target.to_string(),
            occurrence: "1".to_string(),
            summary: format!("失败步骤待重试：{target}"),
        }
    }

    /// 与 draft("s-1") 同目标但更高发生版本的草稿。
    fn draft_occ(target: &str, occurrence: &str) -> InboxItemDraft {
        InboxItemDraft {
            occurrence: occurrence.to_string(),
            ..draft(target)
        }
    }

    #[test]
    fn ensure_item_is_idempotent_and_never_resurrects_resolved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("human-inbox.json");
        let store = HumanInboxStore::new(path.clone());

        let first = store.ensure_item(&draft("s-1"));
        assert_eq!(first.status, STATUS_OPEN);
        assert_eq!(first.item_id, "step_retry:team-x:s-1:1");
        let again = store.ensure_item(&draft("s-1"));
        assert_eq!(again.item_id, first.item_id);
        assert_eq!(store.len(), 1, "重复登记不产生第二条");

        // claim → resolve → 再 ensure：已解决记录本身不复活（九期起在其上分配
        // 新发生版本，见 ensure_item_allocates_next_occurrence_after_resolve）。
        store.claim("step_retry:team-x:s-1:1", "alice").unwrap();
        store
            .begin_resolve("step_retry:team-x:s-1:1", "k1")
            .unwrap();
        store.finish_resolve("step_retry:team-x:s-1:1", json!({"ok": true}));
        let next = store.ensure_item(&draft("s-1"));
        assert_eq!(next.item_id, "step_retry:team-x:s-1:2");
        assert_eq!(next.status, STATUS_OPEN);
        let old = store.get("step_retry:team-x:s-1:1").unwrap();
        assert_eq!(
            old.status, STATUS_RESOLVED,
            "旧记录保持 resolved（不复活也不丢失）"
        );
    }

    /// 九期核心守卫：首选发生版本被 resolved 占用 → 自动分配下一空闲版本，
    /// 重试轮次得到全新待办（修复八期「重试后被旧 resolved 项永久吞掉」）。
    #[test]
    fn ensure_item_allocates_next_occurrence_after_resolve() {
        let temp = tempfile::tempdir().unwrap();
        let store = HumanInboxStore::new(temp.path().join("human-inbox.json"));

        let first = store.ensure_item(&draft("s-1"));
        assert_eq!(first.item_id, "step_retry:team-x:s-1:1");
        store.claim(first.item_id.as_str(), "alice").unwrap();
        store.begin_resolve(&first.item_id, "k1").unwrap();
        store.finish_resolve(&first.item_id, json!({"ok": true}));

        // 重试后再次失败（attempts 又回到 1）→ 分配 :2，旧 resolved 不吞新待办。
        let second = store.ensure_item(&draft("s-1"));
        assert_eq!(second.item_id, "step_retry:team-x:s-1:2");
        assert_eq!(second.status, STATUS_OPEN);
        assert_eq!(store.len(), 2, "旧 resolved 保留为历史");

        // 再次解决 :2 → 下一轮分配 :3（递推可用）。
        store.claim(&second.item_id, "bob").unwrap();
        store.begin_resolve(&second.item_id, "k2").unwrap();
        store.finish_resolve(&second.item_id, json!({"ok": true}));
        let third = store.ensure_item(&draft("s-1"));
        assert_eq!(third.item_id, "step_retry:team-x:s-1:3");
    }

    /// 首选版本仍是 open/claimed → 复用原记录，不产生重复条目。
    #[test]
    fn ensure_item_reuses_unresolved_occurrence() {
        let temp = tempfile::tempdir().unwrap();
        let store = HumanInboxStore::new(temp.path().join("human-inbox.json"));
        let first = store.ensure_item(&draft("s-1"));
        let again = store.ensure_item(&draft_occ("s-1", "1"));
        assert_eq!(again.item_id, first.item_id);
        assert_eq!(store.len(), 1);

        store.claim(&first.item_id, "alice").unwrap();
        let claimed = store.ensure_item(&draft("s-1"));
        assert_eq!(claimed.item_id, first.item_id);
        assert_eq!(claimed.status, STATUS_CLAIMED, "复用不重置协作状态");
        assert_eq!(store.len(), 1);
    }

    /// 八期旧键迁移：claimed/resolved 状态与幂等结果完整保留到新键；
    /// 含冒号的 target_id（artifact id 形如 team:role:v2）也能正确迁移。
    #[test]
    fn legacy_keys_migrated_with_state_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("human-inbox.json");
        std::fs::write(
            &path,
            r#"{
  "items": {
    "step_retry:s-1": {
      "item_id": "step_retry:s-1", "kind": "step_retry", "team_id": "team-x",
      "target_id": "s-1", "status": "claimed", "assignee": "alice",
      "summary": "失败步骤待重试：s-1", "created_at": 1, "claimed_at": 2
    },
    "artifact_review:team-x:impl:v2": {
      "item_id": "artifact_review:team-x:impl:v2", "kind": "artifact_review",
      "team_id": "team-x", "target_id": "team-x:impl:v2", "status": "resolved",
      "summary": "待评审：document v2", "created_at": 1, "resolved_at": 3,
      "idempotency_key": "old-key", "result": {"approved": true}
    },
    "human_result:no-team": {
      "item_id": "human_result:no-team", "kind": "human_result", "team_id": "",
      "target_id": "no-team", "status": "open", "summary": "无 team 的旧记录",
      "created_at": 1
    }
  }
}"#,
        )
        .unwrap();

        let store = HumanInboxStore::new(path);
        // claimed → 迁移到 kind:team:target:1，领取人保留。
        let claimed = store
            .get("step_retry:team-x:s-1:1")
            .expect("迁移后新键在场");
        assert_eq!(claimed.status, STATUS_CLAIMED);
        assert_eq!(claimed.assignee.as_deref(), Some("alice"));
        assert!(store.get("step_retry:s-1").is_none(), "旧键已移除");
        // resolved + 幂等结果保留（含冒号 target）。
        let resolved = store
            .get("artifact_review:team-x:team-x:impl:v2:1")
            .expect("冒号 target 迁移后新键在场");
        assert_eq!(resolved.status, STATUS_RESOLVED);
        assert_eq!(resolved.idempotency_key.as_deref(), Some("old-key"));
        assert_eq!(resolved.result, Some(json!({"approved": true})));
        // 重放语义在新键上继续成立。
        assert!(matches!(
            store.begin_resolve("artifact_review:team-x:team-x:impl:v2:1", "old-key"),
            Err(ResolveLeaseError::AlreadyDone { .. })
        ));
        // team 缺失的旧记录无法定位 → 原样保留（不阻塞服务）。
        assert!(store.get("human_result:no-team").is_some());

        // 迁移后 ensure：s-1 已 claimed → 复用；artifact 已 resolved → 分配 :2。
        let reused = store.ensure_item(&draft("s-1"));
        assert_eq!(reused.item_id, "step_retry:team-x:s-1:1");
        let next = store.ensure_item(&InboxItemDraft {
            kind: KIND_ARTIFACT_REVIEW.to_string(),
            team_id: "team-x".to_string(),
            project_id: None,
            target_id: "team-x:impl:v2".to_string(),
            occurrence: "1".to_string(),
            summary: "待评审：document v2".to_string(),
        });
        assert_eq!(next.item_id, "artifact_review:team-x:team-x:impl:v2:2");
    }

    #[test]
    fn claim_cas_exclusive_and_release_guards() {
        let temp = tempfile::tempdir().unwrap();
        let store = HumanInboxStore::new(temp.path().join("human-inbox.json"));
        store.ensure_item(&draft("s-1"));

        store.claim("step_retry:team-x:s-1:1", "alice").unwrap();
        let err = store.claim("step_retry:team-x:s-1:1", "bob").unwrap_err();
        assert!(matches!(err, ClaimError::Conflict { .. }));
        // 同人幂等。
        store.claim("step_retry:team-x:s-1:1", "alice").unwrap();
        // 他人释放 → Conflict；领取者释放 → open。
        assert!(matches!(
            store.release("step_retry:team-x:s-1:1", "bob"),
            Err(ClaimError::Conflict { .. })
        ));
        let released = store.release("step_retry:team-x:s-1:1", "alice").unwrap();
        assert_eq!(released.status, STATUS_OPEN);
        // 重复释放 → Conflict（已不是 claimed）。
        assert!(matches!(
            store.release("step_retry:team-x:s-1:1", "alice"),
            Err(ClaimError::Conflict { .. })
        ));
    }

    #[test]
    fn resolve_lease_same_key_replays_and_conflicts_on_other_key() {
        let temp = tempfile::tempdir().unwrap();
        let store = HumanInboxStore::new(temp.path().join("human-inbox.json"));
        store.ensure_item(&draft("s-1"));

        let prev = store
            .begin_resolve("step_retry:team-x:s-1:1", "k1")
            .unwrap();
        assert_eq!(prev, STATUS_OPEN);
        // resolving 期间重入 → InProgress。
        assert!(matches!(
            store.begin_resolve("step_retry:team-x:s-1:1", "k1"),
            Err(ResolveLeaseError::InProgress)
        ));
        store.finish_resolve("step_retry:team-x:s-1:1", json!({"done": 1}));
        // 同键重放 → AlreadyDone + 缓存结果。
        assert!(matches!(
            store.begin_resolve("step_retry:team-x:s-1:1", "k1"),
            Err(ResolveLeaseError::AlreadyDone { .. })
        ));
        // 异键 → KeyConflict。
        assert!(matches!(
            store.begin_resolve("step_retry:team-x:s-1:1", "k2"),
            Err(ResolveLeaseError::KeyConflict { .. })
        ));
    }

    #[test]
    fn abort_resolve_rolls_back_and_persists_error() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("human-inbox.json");
        let store = HumanInboxStore::new(path.clone());
        store.ensure_item(&draft("s-1"));
        store.claim("step_retry:team-x:s-1:1", "alice").unwrap();

        let prev = store
            .begin_resolve("step_retry:team-x:s-1:1", "k1")
            .unwrap();
        assert_eq!(prev, STATUS_CLAIMED);
        store.abort_resolve("step_retry:team-x:s-1:1", &prev, "领域校验未通过");
        let item = store.get("step_retry:team-x:s-1:1").unwrap();
        assert_eq!(item.status, STATUS_CLAIMED, "回滚到原状态（可重试）");
        assert_eq!(item.last_error.as_deref(), Some("领域校验未通过"));
    }

    /// 服务重启恢复：同一路径重新加载文件，协作状态完整保留。
    #[test]
    fn state_survives_store_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("human-inbox.json");
        {
            let store = HumanInboxStore::new(path.clone());
            store.ensure_item(&draft("s-1"));
            store.ensure_item(&draft("s-2"));
            store.claim("step_retry:team-x:s-1:1", "alice").unwrap();
            store
                .begin_resolve("step_retry:team-x:s-2:1", "k2")
                .unwrap();
            store.finish_resolve("step_retry:team-x:s-2:1", json!({"ok": true}));
        }
        // 「重启」：全新实例从同一文件加载。
        let reopened = HumanInboxStore::new(path.clone());
        let claimed = reopened.get("step_retry:team-x:s-1:1").unwrap();
        assert_eq!(claimed.status, STATUS_CLAIMED);
        assert_eq!(claimed.assignee.as_deref(), Some("alice"));
        let resolved = reopened.get("step_retry:team-x:s-2:1").unwrap();
        assert_eq!(resolved.status, STATUS_RESOLVED);
        assert_eq!(resolved.result, Some(json!({"ok": true})));
        // 重开实例上幂等语义继续成立。
        assert!(matches!(
            reopened.begin_resolve("step_retry:team-x:s-2:1", "k2"),
            Err(ResolveLeaseError::AlreadyDone { .. })
        ));
    }

    #[test]
    fn corrupt_file_backs_up_and_rebuilds() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("human-inbox.json");
        std::fs::write(&path, "{ not json ]").unwrap();
        let store = HumanInboxStore::new(path.clone());
        assert!(store.is_empty(), "损坏文件从空重建");
        assert!(!path.exists(), "原损坏文件被旁置");
        assert!(
            path.with_extension("json.corrupt").exists(),
            "损坏文件备份在场"
        );
        // 重建后可正常使用。
        store.ensure_item(&draft("s-1"));
        assert_eq!(store.len(), 1);
    }
}

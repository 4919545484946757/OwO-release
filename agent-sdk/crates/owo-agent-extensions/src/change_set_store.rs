//! ChangeSet 存储（八期 · 二路）：`<run_dir>/<team_id>-change-sets.json`
//! （JSON 数组，按 change_set_id upsert；重启可读）。
//!
//! 状态机（`apply_decision`）：
//! - `pending_review / conflicted` --accept--> `accepted`（保留文件现状）；
//! - `pending_review / conflicted` --reject/revert--> `rejected / reverted`
//!   （文件恢复由调用方先执行 [`crate::change_set::restore_change_set`]，恢复成功
//!   才落状态）；
//! - 同动作幂等键重放 → `replayed: true` 零副作用；已决定后跨动作 → `Conflict`
//!   （409）；`accepted / rejected / reverted` 为终态。
//!
//! 批准门控：[`ChangeSetStore::approval_block_reason`] —— 存在 `pending_review /
//! conflicted` 的 ChangeSet 时，该团队的代码 Artifact 可评审但不得成为最终
//! approved head（评审/Inbox 分派处接线）。

use owo_agent_protocol::{ChangeSet, ChangeSetDecision};
use std::path::{Path, PathBuf};

// 便捷 re-export：评审/Inbox 分派处直接 `change_set_store::ChangeSetStatus` 引用。
pub use owo_agent_protocol::ChangeSetStatus;

/// ChangeSet 存储错误（路由层映射：NotFound→404、Conflict→409、Storage→500）。
#[derive(Debug, Clone)]
pub enum ChangeSetStoreError {
    NotFound,
    Conflict(String),
    Storage(String),
}

impl std::fmt::Display for ChangeSetStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeSetStoreError::NotFound => write!(formatter, "ChangeSet 不存在"),
            ChangeSetStoreError::Conflict(message) => write!(formatter, "{message}"),
            ChangeSetStoreError::Storage(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ChangeSetStoreError {}

/// 决定动作（与路由一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeSetAction {
    Accept,
    Reject,
    Revert,
}

impl ChangeSetAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeSetAction::Accept => "accept",
            ChangeSetAction::Reject => "reject",
            ChangeSetAction::Revert => "revert",
        }
    }

    fn status(&self) -> ChangeSetStatus {
        match self {
            ChangeSetAction::Accept => ChangeSetStatus::Accepted,
            ChangeSetAction::Reject => ChangeSetStatus::Rejected,
            ChangeSetAction::Revert => ChangeSetStatus::Reverted,
        }
    }
}

/// 决定结果（`replayed = true` 表示幂等重放，零副作用）。
#[derive(Debug, Clone)]
pub struct DecisionOutcome {
    pub change_set: ChangeSet,
    pub replayed: bool,
}

/// ChangeSet 存储（Clone 共享 run_dir）。
#[derive(Debug, Clone)]
pub struct ChangeSetStore {
    run_dir: PathBuf,
}

/// 进程内团队级串行化锁（十期 · 四路 R3）：并发/重复请求对同一团队 sidecar 的
/// 读-改-写（upsert/apply_decision/mark_conflicted）必须串行，避免丢失更新与
/// 写撕裂；跨进程由原子落盘（tmp + rename）+ 崩溃语义兜底。
fn team_mutex(team_id: &str) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().expect("团队锁表 poisoned");
    guard
        .entry(team_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn normalize_superseded(records: &mut [ChangeSet]) {
    use std::collections::HashMap;

    let mut latest_by_step: HashMap<&str, usize> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        if record.status != ChangeSetStatus::PendingReview {
            continue;
        }
        match latest_by_step.get(record.step_id.as_str()).copied() {
            Some(current)
                if records[current].created_at > record.created_at
                    || (records[current].created_at == record.created_at && current > index) => {}
            Some(_) => {
                latest_by_step.insert(record.step_id.as_str(), index);
            }
            None => {
                latest_by_step.insert(record.step_id.as_str(), index);
            }
        }
    }

    let superseded_by: Vec<Option<(String, String)>> = records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let latest = latest_by_step.get(record.step_id.as_str()).copied()?;
            if latest == index {
                return None;
            }
            // Multiple ChangeSets can legitimately belong to one attempt (for
            // example, separate deliverables). Only a reworked attempt replaces
            // an older pending record; same-attempt records must remain reviewable.
            if record.attempt_id == records[latest].attempt_id {
                return None;
            }
            Some((
                records[latest].change_set_id.clone(),
                records[latest].created_at.clone(),
            ))
        })
        .collect();

    for (index, record) in records.iter_mut().enumerate() {
        let Some((latest_id, decided_at)) = superseded_by[index].clone() else {
            continue;
        };
        record.status = ChangeSetStatus::Superseded;
        record.decision = Some(ChangeSetDecision {
            action: "supersede".to_string(),
            idempotency_key: format!("superseded-by:{latest_id}"),
            decided_at,
            note: Some(format!("superseded by newer attempt {latest_id}")),
        });
        record.conflicts.clear();
    }
}

impl ChangeSetStore {
    pub fn new(run_dir: &Path) -> Self {
        Self {
            run_dir: run_dir.to_path_buf(),
        }
    }

    fn team_path(&self, team_id: &str) -> PathBuf {
        self.run_dir.join(format!("{team_id}-change-sets.json"))
    }

    fn load_team(&self, team_id: &str) -> Result<Vec<ChangeSet>, ChangeSetStoreError> {
        let path = self.team_path(team_id);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ChangeSetStoreError::Storage(format!(
                    "读取 ChangeSet 文件失败（{}）：{error}",
                    path.display()
                )))
            }
        };
        let mut records: Vec<ChangeSet> = serde_json::from_slice(&bytes).map_err(|error| {
            ChangeSetStoreError::Storage(format!(
                "ChangeSet 文件损坏（{}）：{error}",
                path.display()
            ))
        })?;
        normalize_superseded(&mut records);
        Ok(records)
    }

    /// 原子落盘（十期 · 四路 R3）：先写同目录 `<file>.tmp` 再 rename 覆盖——
    /// 崩溃任一点都只会有「旧完整文件」或「新完整文件」，绝不出现截断/半写
    /// 的 sidecar（避免崩溃后状态不可信、重复副作用无法判定）。
    fn save_team(&self, team_id: &str, records: &[ChangeSet]) -> Result<(), ChangeSetStoreError> {
        if let Some(parent) = self.run_dir.parent().or(Some(self.run_dir.as_path())) {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::create_dir_all(&self.run_dir);
        let bytes = serde_json::to_vec_pretty(records).map_err(|error| {
            ChangeSetStoreError::Storage(format!("ChangeSet 序列化失败：{error}"))
        })?;
        let path = self.team_path(team_id);
        let tmp = self.run_dir.join(format!(
            "{team_id}-change-sets.json.tmp-{}",
            std::process::id()
        ));
        std::fs::write(&tmp, &bytes).map_err(|error| {
            ChangeSetStoreError::Storage(format!("ChangeSet 临时落盘失败：{error}"))
        })?;
        std::fs::rename(&tmp, &path).map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            ChangeSetStoreError::Storage(format!(
                "ChangeSet 原子覆盖失败（{} → {}）：{error}",
                tmp.display(),
                path.display()
            ))
        })
    }

    /// 团队级读-改-写串行执行体：持团队锁完成「load → mutate → atomic save」，
    /// 避免并发请求的丢失更新；突变闭包可返回自己的业务错误（NotFound/Conflict），
    /// 该错误原样透传且**不落盘**（零副作用）。
    fn with_team_mut<R>(
        &self,
        team_id: &str,
        mutate: impl FnOnce(&mut Vec<ChangeSet>) -> Result<R, ChangeSetStoreError>,
    ) -> Result<R, ChangeSetStoreError> {
        let team_mutex = team_mutex(team_id);
        let _guard = team_mutex
            .lock()
            .map_err(|_| ChangeSetStoreError::Storage("团队锁 poisoned".to_string()))?;
        let mut records = self.load_team(team_id)?;
        let result = mutate(&mut records)?;
        self.save_team(team_id, &records)?;
        Ok(result)
    }

    /// 按 id upsert（存在同 id → 原位替换；否则追加）。
    pub fn save_upsert(&self, change_set: &ChangeSet) -> Result<(), ChangeSetStoreError> {
        let change_set = change_set.clone();
        self.with_team_mut(&change_set.team_id, |records| {
            // A reworked attempt replaces the prior pending review for the same
            // task. Keep the old record for audit, but make it terminal so the
            // delivery gate points at the current attempt instead of an obsolete
            // ChangeSet. Accepted history is never rewritten.
            if change_set.status == ChangeSetStatus::PendingReview {
                if let Some(attempt) = change_set.attempt_id.as_deref() {
                    for record in records.iter_mut().filter(|record| {
                        record.change_set_id != change_set.change_set_id
                            && record.step_id == change_set.step_id
                            && record.status == ChangeSetStatus::PendingReview
                            && record.attempt_id.as_deref() != Some(attempt)
                            && record.created_at <= change_set.created_at
                    }) {
                        record.status = ChangeSetStatus::Superseded;
                        record.decision = Some(ChangeSetDecision {
                            action: "supersede".to_string(),
                            idempotency_key: format!("superseded-by:{}", change_set.change_set_id),
                            decided_at: change_set.created_at.clone(),
                            note: Some(format!(
                                "superseded by newer attempt {}",
                                change_set.change_set_id
                            )),
                        });
                        record.conflicts.clear();
                    }
                }
            }
            match records
                .iter_mut()
                .find(|record| record.change_set_id == change_set.change_set_id)
            {
                Some(existing) => *existing = change_set.clone(),
                None => records.push(change_set.clone()),
            }
            Ok(())
        })
    }

    /// 团队全部 ChangeSet（缺失 → 空数组）。
    pub fn list_for_team(&self, team_id: &str) -> Result<Vec<ChangeSet>, ChangeSetStoreError> {
        self.load_team(team_id)
    }

    /// 跨团队全量（扫描 `*-change-sets.json`；规模 = 有变更的团队数，小）。
    pub fn list_all(&self) -> Result<Vec<ChangeSet>, ChangeSetStoreError> {
        let mut all = Vec::new();
        // run_dir 不存在 = 还没有任何团队跑过，是正常空状态，不是存储故障。
        //
        // 以前这里无条件 read_dir 并把它当 Storage 错误上抛，导致
        // `GET /change-sets/{id}` 在任何 id 下都返回 **500**（应为 404），
        // 排查时极易误判成"改动集存储坏了"。
        if !self.run_dir.is_dir() {
            return Ok(all);
        }
        let entries = std::fs::read_dir(&self.run_dir)
            .map_err(|error| ChangeSetStoreError::Storage(format!("扫描 run_dir 失败：{error}")))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(team_id) = name.strip_suffix("-change-sets.json") else {
                continue;
            };
            all.extend(self.load_team(team_id)?);
        }
        Ok(all)
    }

    /// 按 id 查找。
    pub fn find(&self, change_set_id: &str) -> Result<Option<ChangeSet>, ChangeSetStoreError> {
        Ok(self
            .list_all()?
            .into_iter()
            .find(|record| record.change_set_id == change_set_id))
    }

    /// 按 id 定位记录及其团队（九期修复：决策写路径只允许落在该团队自己的
    /// sidecar 上——`list_all` 是跨团队只读视图，绝不能整体写回单团队文件）。
    fn locate(&self, change_set_id: &str) -> Result<(String, ChangeSet), ChangeSetStoreError> {
        for record in self.list_all()? {
            if record.change_set_id == change_set_id {
                return Ok((record.team_id.clone(), record));
            }
        }
        Err(ChangeSetStoreError::NotFound)
    }

    /// 标记冲突（revert/reject 检测到用户改动时调用；已决定 → Conflict）。
    pub fn mark_conflicted(
        &self,
        change_set_id: &str,
        conflicts: &[String],
    ) -> Result<ChangeSet, ChangeSetStoreError> {
        let (team_id, _) = self.locate(change_set_id)?;
        let conflicts = conflicts.to_vec();
        self.with_team_mut(&team_id, |records| {
            let record = records
                .iter_mut()
                .find(|record| record.change_set_id == change_set_id)
                .ok_or(ChangeSetStoreError::NotFound)?;
            if record.decision.is_some() {
                return Err(ChangeSetStoreError::Conflict(format!(
                    "ChangeSet {change_set_id} 已决定（{}），不能再标记冲突",
                    record.status_label()
                )));
            }
            record.status = ChangeSetStatus::Conflicted;
            record.conflicts = conflicts.clone();
            Ok(record.clone())
        })
    }

    /// 落决定（accept/reject/revert 共用；幂等键 + 终态校验在此内聚）。
    /// 九期修复：只读写目标团队自己的 sidecar——旧实现把 `list_all()` 的跨团队
    /// 全量列表写回单团队文件，会把其他团队的 ChangeSet 错误搬进本团队 sidecar
    /// 并产生同 id 重复记录（多团队交替决定时 GET 可能读到旧状态）。
    /// 十期·四路 R3：`locate` 定位团队后，**判定与落盘全部在团队锁内**执行——
    /// 并发重复请求对同一 ChangeSet 只允许一个执行者落决定，另一个重放/冲突，
    /// 绝不双写。
    pub fn apply_decision(
        &self,
        change_set_id: &str,
        action: ChangeSetAction,
        idempotency_key: &str,
        note: Option<&str>,
    ) -> Result<DecisionOutcome, ChangeSetStoreError> {
        let (team_id, _) = self.locate(change_set_id)?;
        self.with_team_mut(&team_id, |records| {
            let record = records
                .iter_mut()
                .find(|record| record.change_set_id == change_set_id)
                .ok_or(ChangeSetStoreError::NotFound)?;
            // 幂等重放先判：同动作 → 零副作用返回现状。
            if let Some(existing) = &record.decision {
                if existing.action == action.as_str() {
                    return Ok(DecisionOutcome {
                        change_set: record.clone(),
                        replayed: true,
                    });
                }
                return Err(ChangeSetStoreError::Conflict(format!(
                    "ChangeSet {change_set_id} 已 {}，不能再 {}",
                    existing.action,
                    action.as_str()
                )));
            }
            if !matches!(
                record.status,
                ChangeSetStatus::PendingReview | ChangeSetStatus::Conflicted
            ) {
                return Err(ChangeSetStoreError::Conflict(format!(
                    "ChangeSet {change_set_id} 状态为 {}，不接受 {}",
                    record.status_label(),
                    action.as_str()
                )));
            }
            record.status = action.status();
            record.decision = Some(ChangeSetDecision {
                action: action.as_str().to_string(),
                idempotency_key: idempotency_key.to_string(),
                decided_at: chrono::Utc::now().to_rfc3339(),
                note: note
                    .filter(|note| !note.trim().is_empty())
                    .map(str::to_string),
            });
            record.conflicts.clear();
            Ok(DecisionOutcome {
                change_set: record.clone(),
                replayed: false,
            })
        })
    }

    /// 批准门控（纯函数）：存在 `pending_review / conflicted` 的 ChangeSet → 阻断
    /// 最终 approved head（`accepted / rejected / reverted` 不阻断——前者的修改已
    /// 接受，后两者的修改已撤销、由重试版本的新 ChangeSet 接力）。
    pub fn approval_block_reason(records: &[ChangeSet]) -> Option<String> {
        records
            .iter()
            .find(|record| {
                matches!(
                    record.status,
                    ChangeSetStatus::PendingReview | ChangeSetStatus::Conflicted
                )
            })
            .map(|record| {
                format!(
                    "存在未接受 ChangeSet：{}（{}，步骤 {}，角色 {}）——代码 Artifact 可评审，但不能成为最终 approved head",
                    record.change_set_id,
                    record.status_label(),
                    record.step_id,
                    record.role
                )
            })
    }

    /// 团队级批准门控（评审/Inbox 分派处接线的入口）。
    pub fn approval_block_for_team(
        &self,
        team_id: &str,
    ) -> Result<Option<String>, ChangeSetStoreError> {
        Ok(Self::approval_block_reason(&self.list_for_team(team_id)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_protocol::ChangeSetFileHash;

    fn sample_change_set(id: &str, team_id: &str) -> ChangeSet {
        ChangeSet {
            change_set_id: id.to_string(),
            team_id: team_id.to_string(),
            step_id: "s-implementer".to_string(),
            attempt_id: None,
            role: "implementer".to_string(),
            base_hashes: vec![ChangeSetFileHash {
                path: "src/a.rs".to_string(),
                sha256: Some("base".to_string()),
                content_available: true,
            }],
            result_hashes: vec![ChangeSetFileHash {
                path: "src/a.rs".to_string(),
                sha256: Some("result".to_string()),
                content_available: false,
            }],
            changed_files: vec!["src/a.rs".to_string()],
            diff_ref: None,
            status: ChangeSetStatus::PendingReview,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            decision: None,
            conflicts: Vec::new(),
        }
    }

    fn unique_run_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "owo-change-set-store-{}-{}-{}",
            tag,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loading_sidecar_supersedes_older_pending_same_step() {
        let run_dir = unique_run_dir("load-backfill");
        let mut old = sample_change_set("cs-old", "team-backfill");
        old.attempt_id = Some("1".to_string());
        old.created_at = "2026-01-01T00:00:00+00:00".to_string();
        let mut new = sample_change_set("cs-new", "team-backfill");
        new.attempt_id = Some("2".to_string());
        new.created_at = "2026-01-01T00:01:00+00:00".to_string();
        std::fs::write(
            run_dir.join("team-backfill-change-sets.json"),
            serde_json::to_vec_pretty(&vec![old, new]).unwrap(),
        )
        .unwrap();

        let store = ChangeSetStore::new(&run_dir);
        let records = store.list_for_team("team-backfill").unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, ChangeSetStatus::Superseded);
        assert_eq!(records[0].status_label(), "已被新版本取代");
        assert_eq!(records[1].status, ChangeSetStatus::PendingReview);
        assert!(ChangeSetStore::approval_block_reason(&records)
            .unwrap()
            .contains("cs-new"));
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn save_list_find_roundtrip_and_cross_team_isolation() {
        let run_dir = unique_run_dir("roundtrip");
        let store = ChangeSetStore::new(&run_dir);
        let cs_a = sample_change_set("cs-team-a-s1-1", "team-a");
        let cs_b = sample_change_set("cs-team-b-s1-2", "team-b");
        store.save_upsert(&cs_a).unwrap();
        store.save_upsert(&cs_b).unwrap();
        assert_eq!(store.list_for_team("team-a").unwrap().len(), 1);
        assert_eq!(store.list_for_team("team-b").unwrap().len(), 1);
        assert!(store.list_for_team("team-a").unwrap()[0].change_set_id == "cs-team-a-s1-1");
        assert_eq!(
            store.find("cs-team-b-s1-2").unwrap().unwrap().team_id,
            "team-b"
        );
        assert!(store.find("cs-team-x-s9-9").unwrap().is_none());
        // upsert：同 id 原位替换（不重复）。
        let mut updated = cs_a.clone();
        updated.status = ChangeSetStatus::Accepted;
        store.save_upsert(&updated).unwrap();
        assert_eq!(store.list_for_team("team-a").unwrap().len(), 1);
        assert_eq!(
            store.list_for_team("team-a").unwrap()[0].status,
            ChangeSetStatus::Accepted
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn apply_decision_idempotent_replay_and_cross_action_conflict() {
        let run_dir = unique_run_dir("decision");
        let store = ChangeSetStore::new(&run_dir);
        let cs = sample_change_set("cs-team-a-s1-1", "team-a");
        store.save_upsert(&cs).unwrap();

        // 首次 accept → accepted。
        let outcome = store
            .apply_decision("cs-team-a-s1-1", ChangeSetAction::Accept, "k1", None)
            .unwrap();
        assert!(!outcome.replayed);
        assert_eq!(outcome.change_set.status, ChangeSetStatus::Accepted);
        assert_eq!(
            outcome.change_set.decision.as_ref().unwrap().action,
            "accept"
        );

        // 同动作重放（任意键）→ 零副作用。
        let replay = store
            .apply_decision("cs-team-a-s1-1", ChangeSetAction::Accept, "k1", None)
            .unwrap();
        assert!(replay.replayed);
        let replay2 = store
            .apply_decision("cs-team-a-s1-1", ChangeSetAction::Accept, "k2", None)
            .unwrap();
        assert!(replay2.replayed, "同动作不同键也只重放，不产生第二次副作用");

        // 跨动作（reject after accept）→ Conflict。
        assert!(matches!(
            store.apply_decision("cs-team-a-s1-1", ChangeSetAction::Reject, "k3", None),
            Err(ChangeSetStoreError::Conflict(_))
        ));
        assert!(matches!(
            store.apply_decision("cs-team-a-s1-1", ChangeSetAction::Revert, "k4", None),
            Err(ChangeSetStoreError::Conflict(_))
        ));
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    /// 九期回归守卫：apply_decision 只写目标团队自己的 sidecar——多团队交替决定
    /// 不得把其他团队的记录搬进本团队文件（旧实现 list_all 整体写回的腐蚀）。
    #[test]
    fn apply_decision_scopes_writes_to_own_team_sidecar() {
        let run_dir = unique_run_dir("scope");
        let store = ChangeSetStore::new(&run_dir);
        store
            .save_upsert(&sample_change_set("cs-team-a-s1-1", "team-a"))
            .unwrap();
        store
            .save_upsert(&sample_change_set("cs-team-b-s1-1", "team-b"))
            .unwrap();

        store
            .apply_decision("cs-team-a-s1-1", ChangeSetAction::Accept, "k1", None)
            .unwrap();

        // team-a 文件只含自己的记录；team-b 文件原样未动。
        let a_records = store.list_for_team("team-a").unwrap();
        assert_eq!(a_records.len(), 1, "team-a sidecar 不得混入 team-b 记录");
        assert_eq!(a_records[0].change_set_id, "cs-team-a-s1-1");
        assert_eq!(a_records[0].status, ChangeSetStatus::Accepted);

        // 交替决定 team-b → team-a 的状态与 sidecar 仍不受影响。
        store
            .apply_decision("cs-team-b-s1-1", ChangeSetAction::Reject, "k2", None)
            .unwrap();
        assert_eq!(store.list_for_team("team-a").unwrap().len(), 1);
        let b_records = store.list_for_team("team-b").unwrap();
        assert_eq!(b_records.len(), 1, "team-b sidecar 不得出现同 id 重复记录");
        assert_eq!(b_records[0].status, ChangeSetStatus::Rejected);

        // 全量视图恰好两条、id 唯一；find 读到的是各自最新状态。
        let all = store.list_all().unwrap();
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!(
            store.find("cs-team-a-s1-1").unwrap().unwrap().status,
            ChangeSetStatus::Accepted
        );
        assert_eq!(
            store.find("cs-team-b-s1-1").unwrap().unwrap().status,
            ChangeSetStatus::Rejected
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn conflicted_can_be_decided_afterwards() {
        let run_dir = unique_run_dir("conflicted");
        let store = ChangeSetStore::new(&run_dir);
        store
            .save_upsert(&sample_change_set("cs-team-a-s1-1", "team-a"))
            .unwrap();
        // revert 撞上用户改动 → conflicted。
        let conflicted = store
            .mark_conflicted("cs-team-a-s1-1", &["src/a.rs".to_string()])
            .unwrap();
        assert_eq!(conflicted.status, ChangeSetStatus::Conflicted);
        assert_eq!(conflicted.conflicts, vec!["src/a.rs".to_string()]);
        // 用户处理后（文件恢复到结果哈希状态）→ 仍可落决定。
        let outcome = store
            .apply_decision(
                "cs-team-a-s1-1",
                ChangeSetAction::Reject,
                "k1",
                Some("用户已处理"),
            )
            .unwrap();
        assert!(!outcome.replayed);
        assert_eq!(outcome.change_set.status, ChangeSetStatus::Rejected);
        assert!(
            outcome.change_set.conflicts.is_empty(),
            "落决定后冲突清单清空"
        );
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn newer_attempt_supersedes_prior_pending_review() {
        let run_dir = unique_run_dir("supersede");
        let store = ChangeSetStore::new(&run_dir);
        let mut old = sample_change_set("cs-t-s1-old", "t");
        old.attempt_id = Some("attempt-old".into());
        old.created_at = "2026-01-01T00:00:00+00:00".into();
        store.save_upsert(&old).unwrap();

        let mut new = sample_change_set("cs-t-s1-new", "t");
        new.attempt_id = Some("attempt-new".into());
        new.created_at = "2026-01-01T00:01:00+00:00".into();
        store.save_upsert(&new).unwrap();

        let records = store.list_for_team("t").unwrap();
        let old = records
            .iter()
            .find(|r| r.change_set_id == "cs-t-s1-old")
            .unwrap();
        let new = records
            .iter()
            .find(|r| r.change_set_id == "cs-t-s1-new")
            .unwrap();
        assert_eq!(old.status, ChangeSetStatus::Superseded);
        assert_eq!(new.status, ChangeSetStatus::PendingReview);
        let reason = ChangeSetStore::approval_block_reason(&records).unwrap();
        assert!(reason.contains("cs-t-s1-new"), "{reason}");
        assert!(!reason.contains("cs-t-s1-old"), "{reason}");
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn approval_block_reason_semantics() {
        let mut pending = sample_change_set("cs-t-s1-1", "t");
        assert!(ChangeSetStore::approval_block_reason(&[pending.clone()]).is_some());
        pending.status = ChangeSetStatus::Accepted;
        assert!(
            ChangeSetStore::approval_block_reason(std::slice::from_ref(&pending)).is_none(),
            "accepted 不阻断"
        );
        let mut rejected = sample_change_set("cs-t-s1-2", "t");
        rejected.status = ChangeSetStatus::Rejected;
        assert!(
            ChangeSetStore::approval_block_reason(&[pending, rejected]).is_none(),
            "rejected/reverted（修改已撤销）不阻断，由重试版本的新 ChangeSet 接力"
        );
        let mut conflicted = sample_change_set("cs-t-s1-3", "t");
        conflicted.status = ChangeSetStatus::Conflicted;
        assert!(ChangeSetStore::approval_block_reason(&[conflicted]).is_some());
    }
}

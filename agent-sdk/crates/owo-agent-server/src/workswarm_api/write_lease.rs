//! 范围写租约：按共享工作区内的实际写路径进行并发仲裁。
//!
//! - 未声明写范围（工作区级）与所有写者互斥，作为无法证明路径隔离时的保守默认；
//! - 声明写范围（角色 write_paths 或 TaskGraph 任务子范围）后，不重叠路径可并发，
//!   范围重叠的写者排队。
//!
//! 租约覆盖窗口：前快照 → 执行 → 后快照 → 变更登记；释放后唤醒等待者。实现使用
//! watch 代次与进程内活跃/已释放范围列表，等待者在检查冲突前标记代次，释放方推进
//! 代次，避免丢失唤醒且不需要超时轮询。
//!
//! 已释放范围保留有界历史（HISTORY_LIMIT）：变更窗口归属过滤需要识别其他活动写者
//! 并排除其写入路径，避免误判越界或把变更记入错误的 ChangeSet。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::workspace_change_tracker::{now_ms, path_in_allowed, simplify_path};

/// 已释放范围历史上限（按释放时间保留最近若干条，覆盖并发写窗口归属查询）。
const HISTORY_LIMIT: usize = 64;

/// 写范围（绝对路径键；空 = 工作区级/全局）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WriteScope {
    keys: Vec<PathBuf>,
}

impl WriteScope {
    /// 工作区级（全局）写范围：与所有范围冲突。
    pub(crate) fn global() -> Self {
        Self { keys: Vec::new() }
    }

    /// 由写白名单绝对路径构造（去 verbatim 前缀、排序去重保证可比）。
    pub(crate) fn from_paths(paths: &[PathBuf]) -> Self {
        let mut keys: Vec<PathBuf> = paths.iter().map(|path| simplify_path(path)).collect();
        keys.sort();
        keys.dedup();
        Self { keys }
    }

    pub(crate) fn is_global(&self) -> bool {
        self.keys.is_empty()
    }

    /// 相对路径是否落在本范围内（全局范围不参与归属过滤 → 恒 false）。
    pub(crate) fn covers(&self, root: &Path, relative: &str) -> bool {
        !self.is_global() && path_in_allowed(relative, root, &self.keys)
    }

    /// 范围冲突判定：任一侧为全局 → 冲突；否则任一路径对互为前缀 → 冲突。
    ///
    /// `Path::starts_with` 按组件比较，`src/a` 与 `src/ab` 不冲突（正确）；
    /// 与工具/审批层的绝对路径前缀口径一致（同经 `simplify_path` 去 verbatim）。
    fn conflicts(&self, other: &Self) -> bool {
        if self.is_global() || other.is_global() {
            return true;
        }
        self.keys.iter().any(|a| {
            other
                .keys
                .iter()
                .any(|b| a.starts_with(b) || b.starts_with(a))
        })
    }
}

struct ActiveEntry {
    scope: WriteScope,
    started_at_ms: u64,
}

struct ClosedEntry {
    scope: WriteScope,
    started_at_ms: u64,
    released_at_ms: u64,
}

#[derive(Default)]
struct LeaseState {
    active: Vec<ActiveEntry>,
    closed: Vec<ClosedEntry>,
}

/// 范围租约管理器（同一进程内按工作区共享，覆盖跨 TeamRun 与调度阶段的注册表重建）。
pub(crate) struct WriteLeaseManager {
    state: Mutex<LeaseState>,
    /// 代次：每次释放 +1；等待者以代次变化为唤醒信号。
    generation: tokio::sync::watch::Sender<u64>,
}

static WORKSPACE_MANAGERS: OnceLock<
    Mutex<std::collections::HashMap<PathBuf, Arc<WriteLeaseManager>>>,
> = OnceLock::new();

/// 获取同一工作区共享的租约仲裁器，避免不同团队或阶段重建注册表后各自放行冲突写入。
pub(crate) fn manager_for_workspace(workspace: &Path) -> Arc<WriteLeaseManager> {
    let key = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let managers = WORKSPACE_MANAGERS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut managers = managers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(managers.entry(key).or_insert_with(WriteLeaseManager::new))
}

impl WriteLeaseManager {
    pub(crate) fn new() -> Arc<Self> {
        let (generation, _) = tokio::sync::watch::channel(0u64);
        Arc::new(Self {
            state: Mutex::new(LeaseState::default()),
            generation,
        })
    }

    fn release(&self, scope: &WriteScope, started_at_ms: u64) {
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(index) = state.active.iter().position(|entry| &entry.scope == scope) {
                let entry = state.active.remove(index);
                state.closed.push(ClosedEntry {
                    scope: entry.scope,
                    started_at_ms,
                    released_at_ms: now_ms(),
                });
                if state.closed.len() > HISTORY_LIMIT {
                    let overflow = state.closed.len() - HISTORY_LIMIT;
                    state.closed.drain(..overflow);
                }
            }
        }
        self.generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }

    /// 与本窗口 `[start_ms, end_ms]` 有重叠的其他写范围（活跃 + 已释放历史）。
    fn overlapping_foreign_scopes(
        &self,
        own: &WriteScope,
        start_ms: u64,
        end_ms: u64,
    ) -> Vec<WriteScope> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state
            .active
            .iter()
            .filter(|entry| &entry.scope != own && entry.started_at_ms <= end_ms)
            .map(|entry| entry.scope.clone())
            .chain(
                state
                    .closed
                    .iter()
                    .filter(|entry| {
                        &entry.scope != own
                            && entry.started_at_ms <= end_ms
                            && entry.released_at_ms >= start_ms
                    })
                    .map(|entry| entry.scope.clone()),
            )
            .collect()
    }
}

/// 单个写角色的范围租约。
pub(crate) struct WriteLease {
    manager: Arc<WriteLeaseManager>,
    scope: WriteScope,
}

impl WriteLease {
    pub(crate) fn new(manager: Arc<WriteLeaseManager>, scope: WriteScope) -> Self {
        Self { manager, scope }
    }

    /// Narrow a role lease to one already-authorized task scope.
    pub(crate) fn for_paths(&self, paths: &[PathBuf]) -> Result<Self, String> {
        let scope = WriteScope::from_paths(paths);
        if !self.scope.is_global()
            && scope.keys.iter().any(|path| {
                !self
                    .scope
                    .keys
                    .iter()
                    .any(|allowed| path.starts_with(allowed))
            })
        {
            return Err("任务写租约超出角色预授权范围".to_string());
        }
        Ok(Self {
            manager: Arc::clone(&self.manager),
            scope,
        })
    }

    /// 获取租约：无冲突立即返回；有冲突等待任一释放后重试。
    pub(crate) async fn acquire(&self) -> WriteLeaseGuard {
        let mut generation = self.manager.generation.subscribe();
        loop {
            // 先标记当前代次再检查——释放发生在检查之后也必然推进代次，
            // `changed()` 会立即返回（无 lost-wakeup）。
            generation.borrow_and_update();
            {
                let mut state = self
                    .manager
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if !state
                    .active
                    .iter()
                    .any(|entry| entry.scope.conflicts(&self.scope))
                {
                    let started_at_ms = now_ms();
                    state.active.push(ActiveEntry {
                        scope: self.scope.clone(),
                        started_at_ms,
                    });
                    return WriteLeaseGuard {
                        manager: Arc::clone(&self.manager),
                        scope: self.scope.clone(),
                        started_at_ms,
                    };
                }
            }
            // 发送端由 manager 持有、本租约存活则 manager 存活 → 不会 Err。
            let _ = generation.changed().await;
        }
    }
}

/// 租约守卫：Drop 即释放并唤醒等待者（覆盖成功/失败/取消/panic 展开路径）。
pub(crate) struct WriteLeaseGuard {
    manager: Arc<WriteLeaseManager>,
    scope: WriteScope,
    started_at_ms: u64,
}

impl WriteLeaseGuard {
    /// 与本窗口 `[start_ms, end_ms]` 重叠的其他写范围（用于路径归属过滤）。
    pub(crate) fn foreign_scopes_in_window(&self, start_ms: u64, end_ms: u64) -> Vec<WriteScope> {
        self.manager
            .overlapping_foreign_scopes(&self.scope, start_ms, end_ms)
    }
}

impl Drop for WriteLeaseGuard {
    fn drop(&mut self) {
        self.manager.release(&self.scope, self.started_at_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[tokio::test]
    async fn workspace_managers_share_conflicts_across_registry_rebuilds() {
        let root = std::env::temp_dir().join(format!("owo-write-lease-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let first = manager_for_workspace(&root);
        let rebuilt = manager_for_workspace(&root);
        assert!(Arc::ptr_eq(&first, &rebuilt));
        let scope = WriteScope::from_paths(&[root.join("src/shared.rs")]);
        let held = WriteLease::new(first, scope.clone()).acquire().await;
        let waiting_lease = WriteLease::new(rebuilt, scope);
        let waiting = waiting_lease.acquire();
        assert!(tokio::time::timeout(Duration::from_millis(20), waiting)
            .await
            .is_err());
        drop(held);
        let reacquired = tokio::time::timeout(
            Duration::from_secs(1),
            WriteLease::new(
                manager_for_workspace(&root),
                WriteScope::from_paths(&[root.join("src/shared.rs")]),
            )
            .acquire(),
        )
        .await
        .expect("lease should resume after prior registry releases it");
        drop(reacquired);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 范围互不重叠 → 并发获取不阻塞；重叠/全局 → 排队。
    #[tokio::test]
    async fn disjoint_scopes_acquire_concurrently_overlapping_serialize() {
        let manager = WriteLeaseManager::new();
        let a = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src/a"])),
        );
        let b = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src/b"])),
        );
        let a_again = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src/a"])),
        );
        let global = WriteLease::new(Arc::clone(&manager), WriteScope::global());

        let guard_a = tokio::time::timeout(Duration::from_secs(1), a.acquire())
            .await
            .expect("disjoint a 应立即获取");
        let guard_b = tokio::time::timeout(Duration::from_secs(1), b.acquire())
            .await
            .expect("disjoint b 应立即获取");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), a_again.acquire())
                .await
                .is_err(),
            "重叠范围必须排队"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), global.acquire())
                .await
                .is_err(),
            "全局范围与任一范围冲突"
        );
        drop(guard_a);
        let guard_a2 = tokio::time::timeout(Duration::from_secs(1), a_again.acquire())
            .await
            .expect("释放后重叠范围应可获取");
        // 归属过滤：a2 窗口内 B 仍在活动 → 返回 B 的范围（排除自身 a）。
        let foreign = guard_a2.foreign_scopes_in_window(now_ms() - 1000, now_ms());
        assert!(
            foreign.iter().any(|scope| !scope.is_global()),
            "{foreign:?}"
        );
        drop(guard_a2);
        drop(guard_b);
        let _global = tokio::time::timeout(Duration::from_secs(1), global.acquire())
            .await
            .expect("全部释放后全局范围应可获取");
    }

    #[tokio::test]
    async fn role_lease_can_narrow_to_disjoint_task_scopes() {
        let manager = WriteLeaseManager::new();
        let role = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src"])),
        );
        let task_a = role.for_paths(&paths(&["/w/src/a.rs"])).unwrap();
        let task_b = role.for_paths(&paths(&["/w/src/b.rs"])).unwrap();
        let nested_a = role.for_paths(&paths(&["/w/src/a.rs/child.rs"])).unwrap();
        assert!(role.for_paths(&paths(&["/w/private.rs"])).is_err());

        let _a = task_a.acquire().await;
        let _b = tokio::time::timeout(Duration::from_secs(1), task_b.acquire())
            .await
            .expect("disjoint task scope should run concurrently");
        assert!(
            tokio::time::timeout(Duration::from_millis(30), nested_a.acquire())
                .await
                .is_err()
        );
    }

    /// 前缀重叠（目录 vs 文件）同样冲突。
    #[tokio::test]
    async fn prefix_overlap_conflicts() {
        let manager = WriteLeaseManager::new();
        let dir = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src"])),
        );
        let file = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src/a.rs"])),
        );
        let sibling = WriteLease::new(
            Arc::clone(&manager),
            WriteScope::from_paths(&paths(&["/w/src2"])),
        );
        let _dir = dir.acquire().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(50), file.acquire())
                .await
                .is_err(),
            "目录与其内文件冲突"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), sibling.acquire())
                .await
                .is_ok(),
            "同前缀字面量（src vs src2）按组件比较不冲突"
        );
    }
}

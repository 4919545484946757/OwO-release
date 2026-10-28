use owo_agent_core::project_space_store::SqliteProjectSpaceStore;
use owo_agent_core::workswarm::{TeamCoordinator, TeamTemplateRegistry, WorkSwarmError};
use std::path::PathBuf;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// 状态（进程内单例协调器，懒初始化；目录 = data_root/workswarm）
// ---------------------------------------------------------------------------

/// WorkSwarm 服务端状态：`data_root/workswarm/{space.db, cas, templates, runs}`。
#[derive(Clone)]
pub struct WorkSwarmState {
    inner: Arc<Inner>,
}

pub(super) struct Inner {
    base_dir: PathBuf,
    coordinator: std::sync::OnceLock<Arc<TeamCoordinator>>,
}

impl WorkSwarmState {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                base_dir,
                coordinator: std::sync::OnceLock::new(),
            }),
        }
    }

    /// 获取（并懒初始化）协调器。
    pub fn coordinator(&self) -> Result<Arc<TeamCoordinator>, WorkSwarmError> {
        if let Some(c) = self.inner.coordinator.get() {
            return Ok(c.clone());
        }
        let base = self.inner.base_dir.clone();
        std::fs::create_dir_all(&base)
            .map_err(|e| WorkSwarmError::Io(format!("创建 workswarm 目录失败：{e}")))?;
        let store_path = base.join("space.db");
        let store = SqliteProjectSpaceStore::open(&store_path).map_err(WorkSwarmError::Store)?;
        let cas = owo_agent_core::cas_store::CasStore::new(base.join("cas"))
            .map_err(|e| WorkSwarmError::Io(format!("CAS 初始化失败：{e}")))?;
        let templates = Arc::new(TeamTemplateRegistry::for_workswarm_root(&base));
        let audit = Arc::new(std::sync::Mutex::new(
            owo_agent_core::audit::AuditLog::default(),
        ));
        let mut coordinator = TeamCoordinator::new(
            Arc::new(store)
                as Arc<dyn owo_agent_core::project_space_store::ProjectSpaceStoreBackend>,
            templates,
            cas,
            base.join("runs"),
        );
        coordinator.attach_audit(audit);
        let coordinator = Arc::new(coordinator);
        // 并发首用：set 失败者取先到者（幂等）。
        let _ = self.inner.coordinator.set(coordinator.clone());
        // R2 重启恢复：启动时全量中断扫描（幂等；只落「interrupted」识别标记并
        // 把磁盘遗留的 Running 步骤转为可恢复状态——绝不在启动时静默重放任何写操作；
        // 恢复必须经用户显式 continue / retry 发起）。
        {
            let boot_coordinator = Arc::clone(&coordinator);
            tokio::spawn(async move {
                match boot_coordinator.detect_interrupted().await {
                    Ok(scan) if !scan.interrupted.is_empty() => tracing::info!(
                        teams = ?scan
                            .interrupted
                            .iter()
                            .map(|r| r.team_id.as_str())
                            .collect::<Vec<_>>(),
                        "workswarm 启动扫描：识别到中断运行（等待显式 continue/retry 恢复）"
                    ),
                    Ok(scan) if !scan.unreadable_states.is_empty() => tracing::warn!(
                        teams = ?scan.unreadable_states,
                        "workswarm 启动扫描：存在损坏的运行状态文件（已原样保留，未被覆盖；需人工修复）"
                    ),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(%e, "workswarm 启动中断扫描失败"),
                }
            });
        }
        self.inner
            .coordinator
            .get()
            .cloned()
            .ok_or_else(|| WorkSwarmError::Run("协调器初始化失败".to_string()))
    }
}

// ---------------------------------------------------------------------------
// 内层 worker（agent = 模型驱动；echo/sleep/fail = 内置演示/测试）
// ---------------------------------------------------------------------------

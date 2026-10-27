use super::*;
impl TeamCoordinator {
    pub fn new(
        store: Arc<dyn ProjectSpaceStoreBackend>,
        templates: Arc<TeamTemplateRegistry>,
        cas: CasStore,
        run_dir: PathBuf,
    ) -> Self {
        Self {
            store,
            templates,
            cas,
            bus: AgentBus::new(),
            audit: None,
            run_dir,
            max_agent_members: 5,
            team_locks: Arc::new(Mutex::new(HashMap::new())),
            run_flags: Arc::new(Mutex::new(HashMap::new())),
            loop_alive: Arc::new(Mutex::new(HashMap::new())),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            phase_epochs: Arc::new(Mutex::new(HashMap::new())),
            phase_claims: Arc::new(Mutex::new(HashMap::new())),
            progress_seqs: Arc::new(Mutex::new(HashMap::new())),
            supervisor_states: Arc::new(Mutex::new(HashMap::new())),
            progress_watchers: Arc::new(Mutex::new(HashMap::new())),
            verification_workspaces: Arc::new(Mutex::new(HashMap::new())),
            context_assembly_timings: Arc::new(Mutex::new(
                context_timing::ContextAssemblyTimingStore::default(),
            )),
            artifact_preview_cache: Arc::new(Mutex::new(
                artifact_read::SharedArtifactPreviewCache::default(),
            )),
            phase_context_snapshots: Arc::new(Mutex::new(
                context_snapshot::PhaseContextSnapshotCache::default(),
            )),
        }
    }

    pub fn attach_audit(&mut self, log: Arc<Mutex<AuditLog>>) {
        self.audit = Some(log);
    }

    pub fn bus(&self) -> &AgentBus {
        &self.bus
    }

    pub fn templates(&self) -> &Arc<TeamTemplateRegistry> {
        &self.templates
    }

    pub fn cas(&self) -> &CasStore {
        &self.cas
    }
    /// 项目空间存储（七期 · 第三路：交付端点/诊断用只读访问器）。
    pub fn store(&self) -> &Arc<dyn ProjectSpaceStoreBackend> {
        &self.store
    }

    pub fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub fn bind_verification_workspace(
        &self,
        team_id: &str,
        root: impl AsRef<Path>,
    ) -> WorkSwarmResult<()> {
        let root = root
            .as_ref()
            .canonicalize()
            .map_err(|error| WorkSwarmError::Io(format!("验证工作区不可用：{error}")))?;
        if !root.is_dir() {
            return Err(WorkSwarmError::Validation(
                "验证工作区必须是目录".to_string(),
            ));
        }
        self.verification_workspaces
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(team_id.to_string(), root);
        Ok(())
    }

    pub(crate) fn verification_workspace(&self, team_id: &str) -> Option<PathBuf> {
        self.verification_workspaces
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(team_id)
            .cloned()
    }

    // -- 内部基础 --

    pub(crate) fn team_lock(&self, team_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut map = self.team_locks.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(team_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub(crate) fn run_flag(&self, team_id: &str) -> Arc<AtomicBool> {
        let mut map = self.run_flags.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(team_id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    pub fn cancel_token(&self, team_id: &str) -> Arc<CancelToken> {
        let mut map = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(team_id.to_string())
            .or_insert_with(|| Arc::new(CancelToken::new()))
            .clone()
    }

    /// 重新武装取消令牌（R2/R3）：显式恢复（continue/retry）前调用——
    /// 历史取消不得粘滞到下一轮执行（否则 continue 重启循环的瞬间又被旧取消打断，
    /// 团队永远停在 cancelled，取消链不可收敛）。原子替换：在飞阶段持有的旧 Arc
    /// 不受影响，仅新阶段取到全新令牌。
    pub fn reset_cancel_token(&self, team_id: &str) {
        let mut map = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(team_id.to_string(), Arc::new(CancelToken::new()));
    }

    pub(crate) fn set_run_active(&self, team_id: &str, active: bool) {
        self.run_flag(team_id).store(active, Ordering::SeqCst);
    }

    pub fn is_run_active(&self, team_id: &str) -> bool {
        self.run_flag(team_id).load(Ordering::SeqCst)
    }

    pub(crate) fn loop_flag(&self, team_id: &str) -> Arc<AtomicBool> {
        let mut map = self.loop_alive.lock().unwrap_or_else(|e| e.into_inner());
        map.entry(team_id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone()
    }

    /// 声明/撤销进程内运行循环存活（server 运行循环入口调用；重启后自然为空）。
    pub fn set_loop_alive(&self, team_id: &str, alive: bool) {
        self.loop_flag(team_id).store(alive, Ordering::SeqCst);
    }

    /// 原子声明运行循环所有权：仅当当前无人持有时置真并返回 true。
    ///
    /// `submit_rework` 的「已有循环则跳过」检查与 `run_team_loop` 的启动之间
    /// 存在窗口：两个并发返工/恢复请求都可能通过检查各自 spawn 一个循环，
    /// 两个循环并发 `run_phase` 会把对方的 Claimed/Running 步骤当成死锁并终止
    /// 团队。循环入口必须用本方法竞争所有权，落选者直接返回。
    pub fn try_claim_loop(&self, team_id: &str) -> bool {
        self.loop_flag(team_id)
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// 该团队在当前进程中是否有存活的运行循环（磁盘 Running 但此值为假 → 中断候选）。
    pub fn is_loop_alive(&self, team_id: &str) -> bool {
        self.loop_flag(team_id).load(Ordering::SeqCst)
    }

    // -- 阶段代次 / 实时进度（长任务响应性支撑） --

    /// 登记阶段领取（claim 后调用；进度视图据此外显 current_steps）。
    pub(crate) fn note_phase_claim(&self, team_id: &str, claim: PhaseClaim) {
        self.phase_claims
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(team_id.to_string(), claim);
    }

    /// Worker 包装层真正进入执行时，把实时阶段状态从 Claimed 推进到 Running。
    pub fn mark_phase_step_running(&self, team_id: &str, step_id: &str) -> bool {
        let epoch = self.phase_epoch(team_id);
        let changed = {
            let mut claims = self.phase_claims.lock().unwrap_or_else(|e| e.into_inner());
            let Some(claim) = claims.get_mut(team_id) else {
                return false;
            };
            if claim.epoch != epoch {
                return false;
            }
            let Some(step) = claim.steps.iter_mut().find(|step| step.step_id == step_id) else {
                return false;
            };
            if step.status == "Running" {
                false
            } else {
                step.status = "Running".to_string();
                step.started_at = now_ts();
                true
            }
        };
        if changed {
            self.advance_progress(team_id);
        }
        changed
    }

    /// 清除阶段领取（仅当代次仍匹配；防误清新阶段的领取）。
    pub(crate) fn clear_phase_claim(&self, team_id: &str, epoch: u64) {
        let mut map = self.phase_claims.lock().unwrap_or_else(|e| e.into_inner());
        if map.get(team_id).map(|c| c.epoch) == Some(epoch) {
            map.remove(team_id);
        }
    }

    /// 进度序号 +1（每次状态转移调用；返回新序号）。
    pub(crate) fn advance_progress(&self, team_id: &str) -> u64 {
        let seq = {
            let mut map = self.progress_seqs.lock().unwrap_or_else(|e| e.into_inner());
            let seq = map.entry(team_id.to_string()).or_insert(0);
            *seq = seq.wrapping_add(1);
            *seq
        };
        let watchers = self
            .progress_watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = watchers.get(team_id) {
            // Concurrent transitions may reach this lock in a different order than their
            // sequence increments; publish the latest sequence rather than regressing a watch.
            sender.send_replace(self.progress_seq(team_id));
        }
        seq
    }

    /// 订阅团队状态变化。watch 保留最新序号，订阅前后发生的并发更新不会丢失。
    pub fn subscribe_progress(&self, team_id: &str) -> tokio::sync::watch::Receiver<u64> {
        let mut watchers = self
            .progress_watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = watchers.get(team_id) {
            return sender.subscribe();
        }
        let initial = self.progress_seq(team_id);
        let (sender, receiver) = tokio::sync::watch::channel(initial);
        watchers.insert(team_id.to_string(), sender);
        receiver
    }

    /// 当前进度序号（只读；订阅方据此判断是否有新进度）。
    pub fn progress_seq(&self, team_id: &str) -> u64 {
        self.progress_seqs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(team_id)
            .copied()
            .unwrap_or(0)
    }

    /// 实时进度快照（**不取 team_lock**：长 Worker 执行期间随时可安全调用）。
    ///
    /// - `current_steps` 来自阶段领取记录（代次匹配时）；
    /// - 计数：pending = Pending+Ready，running = Running，failed = Failed+Aborted。
    pub async fn progress_snapshot(&self, team_id: &str) -> WorkSwarmResult<TeamProgress> {
        let team = self
            .store
            .get_team_run(team_id)
            .await
            .map_err(|e| match e {
                ProjectSpaceStoreError::NotFound(_) => {
                    WorkSwarmError::NotFound(format!("团队 {team_id} 不存在"))
                }
                other => WorkSwarmError::Store(other),
            })?;
        let state = self.load_goal_state(team_id)?;
        let mut counts = ProgressCounts::default();
        let mut persisted_running = 0u32;
        for record in state.records.values() {
            match record.status {
                StepStatus::Pending | StepStatus::Ready => counts.pending += 1,
                StepStatus::Running => persisted_running += 1,
                StepStatus::Succeeded => counts.succeeded += 1,
                StepStatus::Failed => counts.failed += 1,
                StepStatus::Aborted => counts.aborted += 1,
            }
        }
        let epoch = self.phase_epoch(team_id);
        let claim = self
            .phase_claims
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(team_id)
            .filter(|claim| claim.epoch == epoch)
            .cloned();
        let current_steps = claim
            .as_ref()
            .map(|claim| claim.steps.clone())
            .unwrap_or_default();
        if let Some(claim) = claim {
            counts.claimed = claim
                .steps
                .iter()
                .filter(|step| step.status == "Claimed")
                .count() as u32;
            counts.running = claim
                .steps
                .iter()
                .filter(|step| step.status == "Running")
                .count() as u32;
        } else {
            // After a restart, persisted Running steps are recoverable claims, not live execution.
            counts.claimed = persisted_running;
        }
        let status = format!("{:?}", team.status);
        let active = self.is_run_active(team_id);
        // Only consult the interrupted marker for a persisted Running team with no
        // live loop; normal polling must not add a filesystem read on every tick.
        let interrupted =
            !active && team.status == TeamRunStatus::Running && self.is_interrupted(team_id);
        let supervisor = supervise_progress(
            &status,
            active,
            interrupted,
            &counts,
            &current_steps,
            chrono::Utc::now(),
            stall_after_secs(),
        );
        // Progress polling is the only supervisor clock: emit an audit event once per
        // health transition, never once per poll tick. The detail is content-free.
        let changed = {
            let mut states = self
                .supervisor_states
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if states.len() >= 4096 && !states.contains_key(team_id) {
                states.clear();
            }
            let previous = states.insert(team_id.to_string(), supervisor.state.clone());
            previous.as_deref() != Some(supervisor.state.as_str())
        };
        if changed {
            self.record_runtime_event(
                team_id,
                "team_supervisor",
                format!(
                    "state={} active={} interrupted={} stalled_steps={} threshold_secs={}",
                    supervisor.state,
                    supervisor.active,
                    supervisor.interrupted,
                    supervisor.stalled_steps.join(","),
                    supervisor.threshold_secs
                ),
            );
            // Supervisor transitions are part of the public progress contract. Advance
            // the shared sequence so SSE clients and watch subscribers receive the new
            // supervisor snapshot even when no step record changed (for example, a
            // running step crosses the stall threshold).
            self.advance_progress(team_id);
        }
        Ok(TeamProgress {
            seq: self.progress_seq(team_id),
            team_id: team_id.to_string(),
            status,
            active,
            current_steps,
            counts,
            updated_at: now_ts(),
            supervisor,
        })
    }

    /// Record a bounded, content-free execution diagnostic in the team audit stream.
    /// Callers must pass identifiers and measurements only, never prompts or tool arguments.
    pub fn record_runtime_event(&self, team_id: &str, event: &str, detail: String) {
        self.audit(team_id, event, detail);
    }

    pub(crate) fn runtime_event_details(&self, team_id: &str, event: &str) -> Vec<String> {
        let Some(log) = &self.audit else {
            return Vec::new();
        };
        let Ok(log) = log.lock() else {
            return Vec::new();
        };
        log.entries
            .iter()
            .filter(|entry| entry.session_id == team_id && entry.event == event)
            .map(|entry| entry.detail.clone())
            .collect()
    }

    pub(crate) fn audit(&self, team_id: &str, event: &str, detail: String) {
        if let Some(log) = &self.audit {
            if let Ok(mut log) = log.lock() {
                log.record(
                    team_id,
                    event,
                    Some(format!("workswarm/{team_id}")),
                    None,
                    detail,
                );
            }
        }
    }
}

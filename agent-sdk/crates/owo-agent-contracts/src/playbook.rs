//! 任务剧本（playbook）：按域定义「这类任务怎么做」，并记录一次运行的进度。
//!
//! ## 定位
//!
//! 仓库里已有两类编排资产，本模块**不替代它们**：
//! * `TeamTemplate` / `workflow`：多 Agent 团队的角色 DAG（谁做、写哪里）；
//! * `goal` / `GoalRunner`：一次性目标的计划与验收。
//!
//! 缺的是**可复用的单 Agent 任务剧本**：把"代码评审怎么做""发布前检查什么"这类
//! 固定套路写成文件，反复执行、按步骤记录进度。所以本模块只做两件事：
//!
//! 1. **定义与校验**（[`Playbook`] / [`PlaybookStep`]）：结构 + 依赖 + 环检测；
//! 2. **运行状态机**（[`PlaybookRun`]）：什么能做、做到哪、卡在哪。
//!
//! 它**不执行任何命令、不读写工作区**：`Check` 步骤的命令由宿主经 Tool Host 执行
//! （权限、审批、审计一个都不能绕），这里只记录"过了/没过"。
//!
//! 这也是「工作流是额外功能」的落点：新增模块与新增命令，不改变任何既有入口。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 单个剧本的步骤上限（防御性上限：执行时每步都可能是一次模型回合）。
pub const MAX_PLAYBOOK_STEPS: usize = 64;

/// 一份任务剧本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playbook {
    /// 稳定 id（如 `code-review`）。
    pub id: String,
    /// 人读名称。
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<PlaybookStep>,
}

/// 剧本里的一步。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybookStep {
    /// 剧本内唯一 id。
    pub id: String,
    /// 人读标题（会出现在进度里）。
    pub title: String,
    /// 展开说明（给执行者的上下文）。
    #[serde(default)]
    pub detail: String,
    /// 完成判据：人话或可执行断言（`Check` 步骤的判据由宿主执行）。
    #[serde(default)]
    pub acceptance: String,
    /// `Check` 步骤可声明一条由宿主执行的命令（0 = 通过）。
    ///
    /// 本模块**不执行**它：执行经宿主 Tool Host（`playbook_check` 工具内部复用
    /// `run_command` 的沙箱与审计），这里只存声明。空 = 人工判定/由模型记录。
    #[serde(default)]
    pub command: String,
    /// 前置步骤 id：必须都已通过，本步才可开始。
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// 步骤类型。
    #[serde(default)]
    pub kind: StepKind,
}

/// 步骤类型：决定"谁来判定这一步完成"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// 需要模型判断/生成（默认）。
    #[default]
    Model,
    /// 确定性校验：由宿主执行命令/断言，退出码/断言结果即结论。
    /// 这类步骤**不需要模型回合**，是"高效"的主要来源。
    Check,
    /// 必须由人确认（审批、提供信息、主观验收）。
    Human,
}

/// 一条校验问题（`code` 稳定，供测试与前端断言）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybookIssue {
    pub code: &'static str,
    /// 出问题的步骤 id（全局问题为 None）。
    pub step: Option<String>,
    pub message: String,
}

impl PlaybookIssue {
    fn new(code: &'static str, step: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            code,
            step: step.map(str::to_string),
            message: message.into(),
        }
    }
}

impl Playbook {
    /// 校验结构：必填、id 唯一、依赖存在、无环、步骤数上限。
    pub fn validate(&self) -> Vec<PlaybookIssue> {
        let mut issues = Vec::new();
        if self.id.trim().is_empty() {
            issues.push(PlaybookIssue::new(
                "playbook_id_empty",
                None,
                "剧本 id 不能为空",
            ));
        }
        if self.name.trim().is_empty() {
            issues.push(PlaybookIssue::new(
                "playbook_name_empty",
                None,
                "剧本 name 不能为空",
            ));
        }
        if self.steps.is_empty() {
            issues.push(PlaybookIssue::new(
                "playbook_empty",
                None,
                "剧本至少要有一步",
            ));
            return issues;
        }
        if self.steps.len() > MAX_PLAYBOOK_STEPS {
            issues.push(PlaybookIssue::new(
                "playbook_too_large",
                None,
                format!("步骤数 {} 超过上限 {MAX_PLAYBOOK_STEPS}", self.steps.len()),
            ));
        }

        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for step in &self.steps {
            if step.id.trim().is_empty() {
                issues.push(PlaybookIssue::new(
                    "step_id_empty",
                    None,
                    "步骤 id 不能为空",
                ));
                continue;
            }
            if !seen.insert(step.id.as_str()) {
                issues.push(PlaybookIssue::new(
                    "step_id_duplicate",
                    Some(&step.id),
                    format!("步骤 id `{}` 重复", step.id),
                ));
            }
            if step.title.trim().is_empty() {
                issues.push(PlaybookIssue::new(
                    "step_title_empty",
                    Some(&step.id),
                    format!("步骤 `{}` 缺少 title", step.id),
                ));
            }
            if step.kind == StepKind::Check && step.acceptance.trim().is_empty() {
                issues.push(PlaybookIssue::new(
                    "check_without_acceptance",
                    Some(&step.id),
                    format!(
                        "Check 步骤 `{}` 必须声明 acceptance（否则无从判定）",
                        step.id
                    ),
                ));
            }
        }

        for step in &self.steps {
            for dependency in &step.depends_on {
                if dependency == &step.id {
                    issues.push(PlaybookIssue::new(
                        "step_self_dependency",
                        Some(&step.id),
                        format!("步骤 `{}` 依赖了自己", step.id),
                    ));
                } else if !seen.contains(dependency.as_str()) {
                    issues.push(PlaybookIssue::new(
                        "step_unknown_dependency",
                        Some(&step.id),
                        format!("步骤 `{}` 依赖了不存在的 `{dependency}`", step.id),
                    ));
                }
            }
        }

        if let Err(cycle) = self.topological_order() {
            issues.push(PlaybookIssue::new(
                "playbook_cycle",
                None,
                format!("依赖成环：{}", cycle.join(" → ")),
            ));
        }
        issues
    }

    /// 拓扑序（同层保持定义顺序）。成环时返回环上的 id。
    ///
    /// 未知依赖不参与成环判定：它已经由 `step_unknown_dependency` 单独报告，
    /// 否则「依赖了不存在的步骤」会被误报成环，诊断信息自相矛盾。
    pub fn topological_order(&self) -> Result<Vec<&str>, Vec<String>> {
        let known: BTreeSet<&str> = self.steps.iter().map(|step| step.id.as_str()).collect();
        let mut remaining: Vec<&PlaybookStep> = self.steps.iter().collect();
        let mut done: BTreeSet<&str> = BTreeSet::new();
        let mut order: Vec<&str> = Vec::new();
        loop {
            let mut progressed = false;
            let mut still: Vec<&PlaybookStep> = Vec::with_capacity(remaining.len());
            for step in remaining {
                // 只把"依赖已全部完成"的步骤取出来；未知依赖视为不阻塞。
                let ready = step.depends_on.iter().all(|dependency| {
                    !known.contains(dependency.as_str()) || done.contains(dependency.as_str())
                });
                if ready {
                    done.insert(step.id.as_str());
                    order.push(step.id.as_str());
                    progressed = true;
                } else {
                    still.push(step);
                }
            }
            remaining = still;
            if remaining.is_empty() {
                return Ok(order);
            }
            if !progressed {
                let cycle: Vec<String> = remaining.iter().map(|step| step.id.clone()).collect();
                return Err(cycle);
            }
        }
    }
}

/// 单步状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum StepState {
    /// 尚未开始（可能因依赖未满足而暂不可做）。
    Pending,
    /// 已通过（带证据：命令输出摘要/评审结论/人工确认）。
    Passed { evidence: String },
    /// 失败过（记录尝试次数与最近原因）。
    Failed { reason: String, attempts: u32 },
    /// 主动跳过（记录原因；依赖它的步骤会被阻塞）。
    Skipped { reason: String },
}

impl StepState {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Passed { .. } | Self::Skipped { .. })
    }

    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed { .. })
    }
}

/// 一次运行的进度快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybookProgress {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub pending: usize,
    /// 因前置失败/跳过而永远做不了的步骤（不是当前 active 的）。
    pub blocked: usize,
}

impl PlaybookProgress {
    /// 完成百分比（passed + skipped 视为已了结；blocked 不算完成）。
    pub fn percent(&self) -> u32 {
        if self.total == 0 {
            return 100;
        }
        (((self.passed + self.skipped) * 100) / self.total) as u32
    }
}

/// 一次剧本运行的状态（可序列化，调用方负责持久化）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybookRun {
    pub playbook_id: String,
    pub states: BTreeMap<String, StepState>,
    /// 调用方写入的时间戳/批次号（本模块不生成时间，保持纯函数）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
}

impl PlaybookRun {
    /// 按剧本初始化：所有步骤 Pending。
    pub fn new(playbook: &Playbook) -> Self {
        Self {
            playbook_id: playbook.id.clone(),
            states: playbook
                .steps
                .iter()
                .map(|step| (step.id.clone(), StepState::Pending))
                .collect(),
            marker: None,
        }
    }

    /// 该步骤是否因前置失败/跳过而永久阻塞。
    ///
    /// 语义：依赖链上出现 Failed 或 Skipped，本步就做不了（需要人处理前置）。
    pub fn is_blocked(&self, playbook: &Playbook, step_id: &str) -> bool {
        let Some(step) = playbook.steps.iter().find(|step| step.id == step_id) else {
            return true;
        };
        if !matches!(
            self.states.get(step_id).unwrap_or(&StepState::Pending),
            StepState::Pending
        ) {
            return false;
        }
        step.depends_on.iter().any(|dependency| {
            matches!(
                self.states.get(dependency).unwrap_or(&StepState::Pending),
                StepState::Failed { .. } | StepState::Skipped { .. }
            ) || self.is_blocked(playbook, dependency)
        })
    }

    /// 下一个可执行步骤：`Pending` 或 `Failed`（可重试）且依赖全部已通过，
    /// 按定义顺序取第一个。
    ///
    /// 失败步骤**保留在可执行集里**：`Failed` 是"上次没过"，不是终态；重试次数由
    /// `attempts` 累计，是否继续重试由调用方（模型/推进策略）决定。
    pub fn next_ready<'a>(&self, playbook: &'a Playbook) -> Option<&'a PlaybookStep> {
        playbook.steps.iter().find(|step| {
            matches!(
                self.states.get(&step.id).unwrap_or(&StepState::Pending),
                StepState::Pending | StepState::Failed { .. }
            ) && step
                .depends_on
                .iter()
                .all(|dependency| self.is_passed(dependency))
        })
    }

    fn is_passed(&self, step_id: &str) -> bool {
        matches!(self.states.get(step_id), Some(StepState::Passed { .. }))
    }

    /// 记录通过。
    pub fn record_pass(&mut self, step_id: &str, evidence: impl Into<String>) {
        self.states.insert(
            step_id.to_string(),
            StepState::Passed {
                evidence: evidence.into(),
            },
        );
    }

    /// 记录失败（保留尝试次数，供有界重试）。
    pub fn record_fail(&mut self, step_id: &str, reason: impl Into<String>) {
        let attempts = match self.states.get(step_id) {
            Some(StepState::Failed { attempts, .. }) => attempts + 1,
            _ => 1,
        };
        self.states.insert(
            step_id.to_string(),
            StepState::Failed {
                reason: reason.into(),
                attempts,
            },
        );
    }

    /// 记录跳过。
    pub fn record_skip(&mut self, step_id: &str, reason: impl Into<String>) {
        self.states.insert(
            step_id.to_string(),
            StepState::Skipped {
                reason: reason.into(),
            },
        );
    }

    /// 某一步的尝试次数（未失败过 = 0）。
    pub fn attempts(&self, step_id: &str) -> u32 {
        match self.states.get(step_id) {
            Some(StepState::Failed { attempts, .. }) => *attempts,
            _ => 0,
        }
    }

    /// 进度快照。
    pub fn progress(&self, playbook: &Playbook) -> PlaybookProgress {
        let mut progress = PlaybookProgress {
            total: playbook.steps.len(),
            passed: 0,
            failed: 0,
            skipped: 0,
            pending: 0,
            blocked: 0,
        };
        for step in &playbook.steps {
            match self.states.get(&step.id).unwrap_or(&StepState::Pending) {
                StepState::Passed { .. } => progress.passed += 1,
                StepState::Skipped { .. } => progress.skipped += 1,
                StepState::Failed { .. } => progress.failed += 1,
                StepState::Pending => {
                    if self.is_blocked(playbook, &step.id) {
                        progress.blocked += 1;
                    } else {
                        progress.pending += 1;
                    }
                }
            }
        }
        progress
    }

    /// 全部步骤都已了结（passed/skipped）；失败与阻塞不算完成。
    pub fn is_complete(&self, playbook: &Playbook) -> bool {
        let progress = self.progress(playbook);
        progress.passed + progress.skipped == progress.total
    }

    /// 一行进度摘要（CLI/日志用）。
    pub fn summary(&self, playbook: &Playbook) -> String {
        let progress = self.progress(playbook);
        let mut parts = vec![format!(
            "{}/{}（{}%）",
            progress.passed,
            progress.total,
            progress.percent()
        )];
        if progress.failed > 0 {
            parts.push(format!("失败 {}", progress.failed));
        }
        if progress.blocked > 0 {
            parts.push(format!("阻塞 {}", progress.blocked));
        }
        if progress.skipped > 0 {
            parts.push(format!("跳过 {}", progress.skipped));
        }
        parts.join("，")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: &str, depends_on: &[&str], kind: StepKind) -> PlaybookStep {
        PlaybookStep {
            id: id.to_string(),
            title: format!("步骤 {id}"),
            detail: String::new(),
            acceptance: if kind == StepKind::Check {
                "exit 0".to_string()
            } else {
                String::new()
            },
            command: String::new(),
            depends_on: depends_on.iter().map(|value| value.to_string()).collect(),
            kind,
        }
    }

    fn playbook() -> Playbook {
        Playbook {
            id: "demo".to_string(),
            name: "演示剧本".to_string(),
            description: String::new(),
            steps: vec![
                step("plan", &[], StepKind::Model),
                step("check", &["plan"], StepKind::Check),
                step("review", &["check"], StepKind::Human),
                step("ship", &["review"], StepKind::Model),
            ],
        }
    }

    #[test]
    fn valid_playbook_passes_and_orders_steps() {
        let playbook = playbook();
        assert!(playbook.validate().is_empty(), "{:?}", playbook.validate());
        assert_eq!(
            playbook.topological_order().unwrap(),
            vec!["plan", "check", "review", "ship"]
        );
    }

    #[test]
    fn duplicate_ids_unknown_deps_and_self_deps_are_reported() {
        let mut playbook = playbook();
        playbook.steps.push(step("plan", &[], StepKind::Model));
        playbook
            .steps
            .push(step("orphan", &["nope", "orphan"], StepKind::Model));
        let codes: Vec<&str> = playbook.validate().iter().map(|issue| issue.code).collect();
        assert!(codes.contains(&"step_id_duplicate"), "{codes:?}");
        assert!(codes.contains(&"step_unknown_dependency"), "{codes:?}");
        assert!(codes.contains(&"step_self_dependency"), "{codes:?}");
    }

    #[test]
    fn unknown_dependency_is_not_misreported_as_cycle() {
        let playbook = Playbook {
            id: "unknown-dep".to_string(),
            name: "未知依赖".to_string(),
            description: String::new(),
            steps: vec![
                step("a", &[], StepKind::Model),
                step("c", &["missing"], StepKind::Model),
            ],
        };
        let codes: Vec<&str> = playbook.validate().iter().map(|issue| issue.code).collect();
        assert!(codes.contains(&"step_unknown_dependency"), "{codes:?}");
        assert!(!codes.contains(&"playbook_cycle"), "{codes:?}");
        // 排序仍能把合法步骤排出来（未知依赖不阻塞）。
        assert_eq!(playbook.topological_order().unwrap(), vec!["a", "c"]);
    }

    #[test]
    fn cycles_are_detected() {
        let playbook = Playbook {
            id: "cycle".to_string(),
            name: "环".to_string(),
            description: String::new(),
            steps: vec![
                step("a", &["b"], StepKind::Model),
                step("b", &["a"], StepKind::Model),
            ],
        };
        let codes: Vec<&str> = playbook.validate().iter().map(|issue| issue.code).collect();
        assert!(codes.contains(&"playbook_cycle"), "{codes:?}");
        assert!(playbook.topological_order().is_err());
    }

    #[test]
    fn check_steps_require_acceptance_and_size_is_capped() {
        let mut playbook = playbook();
        playbook.steps[1].acceptance.clear();
        let codes: Vec<&str> = playbook.validate().iter().map(|issue| issue.code).collect();
        assert!(codes.contains(&"check_without_acceptance"), "{codes:?}");

        let playbook = Playbook {
            id: "big".to_string(),
            name: "过大".to_string(),
            description: String::new(),
            steps: (0..(MAX_PLAYBOOK_STEPS + 1))
                .map(|index| step(&format!("s{index}"), &[], StepKind::Model))
                .collect(),
        };
        let codes: Vec<&str> = playbook.validate().iter().map(|issue| issue.code).collect();
        assert!(codes.contains(&"playbook_too_large"), "{codes:?}");
    }

    #[test]
    fn check_command_is_optional_and_round_trips() {
        let playbook: Playbook = serde_json::from_str(
            r#"{"id":"c","name":"C","steps":[
                {"id":"a","title":"A","kind":"check","acceptance":"exit 0","command":"cargo check"},
                {"id":"b","title":"B","kind":"check","acceptance":"人工确认"}
            ]}"#,
        )
        .unwrap();
        assert!(playbook.validate().is_empty(), "{:?}", playbook.validate());
        assert_eq!(playbook.steps[0].command, "cargo check");
        assert_eq!(playbook.steps[1].command, "");
        let json = serde_json::to_string(&playbook).unwrap();
        let restored: Playbook = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, playbook);
    }

    #[test]
    fn next_ready_respects_dependencies_and_definition_order() {
        let playbook = playbook();
        let mut run = PlaybookRun::new(&playbook);
        assert_eq!(
            run.next_ready(&playbook).map(|step| step.id.as_str()),
            Some("plan")
        );
        run.record_pass("plan", "计划已确认");
        assert_eq!(
            run.next_ready(&playbook).map(|step| step.id.as_str()),
            Some("check")
        );
        // 跳步不生效：ship 的依赖链未满足。
        assert!(run
            .next_ready(&playbook)
            .map(|step| step.id != "ship")
            .unwrap_or(false));
    }

    #[test]
    fn failed_step_stays_retryable() {
        let playbook = playbook();
        let mut run = PlaybookRun::new(&playbook);
        run.record_pass("plan", "ok");
        run.record_fail("check", "第一次 exit 1");
        // 失败不是终态：next_ready 会把这一步再次给出（attempts 记录次数）。
        assert_eq!(
            run.next_ready(&playbook).map(|step| step.id.as_str()),
            Some("check")
        );
        assert_eq!(run.attempts("check"), 1);
        // 重试通过后继续往下走。
        run.record_pass("check", "exit 0");
        assert_eq!(
            run.next_ready(&playbook).map(|step| step.id.as_str()),
            Some("review")
        );
    }

    #[test]
    fn failed_dependency_blocks_downstream_steps() {
        let playbook = playbook();
        let mut run = PlaybookRun::new(&playbook);
        run.record_pass("plan", "ok");
        run.record_fail("check", "exit 1");
        let progress = run.progress(&playbook);
        assert_eq!(progress.failed, 1);
        // review 与 ship 都被阻塞（依赖链上有失败）。
        assert_eq!(progress.blocked, 2);
        assert_eq!(progress.pending, 0);
        assert!(!run.is_complete(&playbook));
        assert!(run.is_blocked(&playbook, "review"));
        assert!(run.is_blocked(&playbook, "ship"));
        // 重试次数累计。
        run.record_fail("check", "still failing");
        assert_eq!(run.attempts("check"), 2);
    }

    #[test]
    fn skipped_dependency_blocks_downstream() {
        let playbook = playbook();
        let mut run = PlaybookRun::new(&playbook);
        run.record_pass("plan", "ok");
        run.record_skip("check", "环境不可用");
        let progress = run.progress(&playbook);
        assert_eq!(progress.skipped, 1);
        assert_eq!(progress.blocked, 2);
    }

    #[test]
    fn completion_and_percent_track_terminal_states() {
        let playbook = playbook();
        let mut run = PlaybookRun::new(&playbook);
        assert_eq!(run.progress(&playbook).percent(), 0);
        run.record_pass("plan", "ok");
        run.record_pass("check", "exit 0");
        assert_eq!(run.progress(&playbook).percent(), 50);
        run.record_pass("review", "人工确认");
        run.record_pass("ship", "已发布");
        assert!(run.is_complete(&playbook));
        assert_eq!(run.progress(&playbook).percent(), 100);
        assert_eq!(run.summary(&playbook), "4/4（100%）");
    }

    #[test]
    fn run_state_is_serializable_for_persistence() {
        let playbook = playbook();
        let mut run = PlaybookRun::new(&playbook);
        run.record_pass("plan", "ok");
        run.record_fail("check", "boom");
        run.marker = Some("2026-11-18T02:40:00+08:00".to_string());
        let json = serde_json::to_string(&run).unwrap();
        let restored: PlaybookRun = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, run);
        assert_eq!(restored.attempts("check"), 1);
    }

    #[test]
    fn playbook_round_trips_through_json() {
        let playbook = playbook();
        let json = serde_json::to_string_pretty(&playbook).unwrap();
        let restored: Playbook = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, playbook);
        // 默认 kind = model，缺省字段可省略。
        let minimal: Playbook =
            serde_json::from_str(r#"{"id":"x","name":"X","steps":[{"id":"a","title":"A"}]}"#)
                .unwrap();
        assert_eq!(minimal.steps[0].kind, StepKind::Model);
        assert!(minimal.steps[0].depends_on.is_empty());
    }
}

//! Plan：目标拆解为步骤依赖图（DAG），§12 Goal/Plan 编排层的数据模型。
//!
//! - 步骤含：前置依赖（DAG 边）、可并行标记、worker 规格（由 [`crate::goal::WorkerRegistry`] 派发）、
//!   验证断言（verify）、重试策略（预算内）。
//! - 非法环检测 + 缺失依赖校验；拓扑排序（wave 分层）供调度器使用。
//! - 序列化持久化（`<dir>/<plan_id>.json`），重启可恢复。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// 步骤执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepStatus {
    /// 已创建，等待依赖就绪。
    Pending,
    /// 依赖已全部成功，可被调度。
    Ready,
    /// 执行中。
    Running,
    /// 执行 + 验证通过。
    Succeeded,
    /// 重试耗尽或验证失败。
    Failed,
    /// abort/replan 时被中止。
    Aborted,
}

impl StepStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            StepStatus::Succeeded | StepStatus::Failed | StepStatus::Aborted
        )
    }

    pub fn can_resume(self) -> bool {
        matches!(
            self,
            StepStatus::Pending | StepStatus::Ready | StepStatus::Failed
        )
    }
}

/// 验证断言（步骤输出 / 目标验收条件共用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationSpec {
    /// 输出包含子串。
    OutputContains(String),
    /// 输出与期望完全相等。
    OutputEquals(String),
    /// 输出非空。
    OutputNonEmpty,
    /// 保留扩展（自定义校验器名称）；宿主未注册校验器时保持未验证。
    Custom(String),
}

/// 对 worker 输出做验证断言（纯函数，可测）。
pub fn verify_output(spec: &VerificationSpec, output: &str) -> Result<(), String> {
    match spec {
        VerificationSpec::OutputContains(needle) => {
            if output.contains(needle.as_str()) {
                Ok(())
            } else {
                Err(format!(
                    "验证失败：输出缺少「{needle}」（实际：{}）",
                    preview(output)
                ))
            }
        }
        VerificationSpec::OutputEquals(expected) => {
            if output == expected.as_str() {
                Ok(())
            } else {
                Err(format!(
                    "验证失败：输出不等于「{expected}」（实际：{}）",
                    preview(output)
                ))
            }
        }
        VerificationSpec::OutputNonEmpty => {
            if output.trim().is_empty() {
                Err("验证失败：输出为空".to_string())
            } else {
                Ok(())
            }
        }
        VerificationSpec::Custom(name) => {
            Err(format!("验证器「{name}」未注册；自定义验收保持未验证"))
        }
    }
}

/// Scope bound to a registered validator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VerificationScopeV1 {
    StepOutput,
    ArtifactRefs { artifact_ids: Vec<String> },
    WorkspacePaths { relative_paths: Vec<String> },
    Manual,
}

/// Resource budget reserved before a validator starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationResourcesV1 {
    #[serde(default = "default_validator_cpu_slots")]
    pub cpu_slots: u16,
    #[serde(default)]
    pub memory_mb: u32,
    #[serde(default)]
    pub exclusive_workspace: bool,
    #[serde(default)]
    pub timeout_ms: u64,
}

fn default_validator_cpu_slots() -> u16 {
    1
}

impl Default for VerificationResourcesV1 {
    fn default() -> Self {
        Self {
            cpu_slots: 1,
            memory_mb: 0,
            exclusive_workspace: false,
            timeout_ms: 0,
        }
    }
}

/// One host-resolved verification obligation. The executor must resolve its
/// validator_id against an immutable host registry before execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationRequirementV1 {
    pub requirement_id: String,
    #[serde(default)]
    pub covers_requirement_ids: Vec<String>,
    pub validator_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator_version: Option<String>,
    pub scope: VerificationScopeV1,
    #[serde(default = "empty_validator_arguments")]
    pub arguments: serde_json::Value,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub resources: VerificationResourcesV1,
}

fn empty_validator_arguments() -> serde_json::Value {
    serde_json::json!({})
}
fn default_true() -> bool {
    true
}

/// Verification obligations attached to a task, independent of worker prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationPlanV1 {
    pub plan_id: String,
    #[serde(default)]
    pub requirements: Vec<VerificationRequirementV1>,
}

impl VerificationPlanV1 {
    /// Structural safety only; validator support is checked by the host executor.
    pub fn validate(&self) -> Result<(), String> {
        if self.plan_id.trim().is_empty() {
            return Err("验证计划缺少 plan_id".into());
        }
        if self.requirements.is_empty() {
            return Err("验证计划没有任何验收要求".into());
        }
        if !self
            .requirements
            .iter()
            .any(|requirement| requirement.required)
        {
            return Err("验证计划至少需要一个必需验收要求".into());
        }
        let mut ids = HashSet::new();
        for req in &self.requirements {
            if req.requirement_id.trim().is_empty() {
                return Err("验证要求缺少 requirement_id".into());
            }
            if !ids.insert(req.requirement_id.as_str()) {
                return Err(format!("验证要求 id 重复：{}", req.requirement_id));
            }
            if req.validator_id.trim().is_empty() {
                return Err(format!("验证要求 {} 缺少 validator_id", req.requirement_id));
            }
            if req.required
                && !matches!(&req.scope, VerificationScopeV1::Manual)
                && req
                    .validator_version
                    .as_deref()
                    .map(str::trim)
                    .filter(|version| !version.is_empty())
                    .is_none()
            {
                return Err(format!(
                    "必需验证要求 {} 必须固定 validator_version",
                    req.requirement_id
                ));
            }
            if !req.arguments.is_object() {
                return Err(format!(
                    "验证要求 {} 的 arguments 必须是 JSON object",
                    req.requirement_id
                ));
            }
            if req
                .covers_requirement_ids
                .iter()
                .any(|id| id.trim().is_empty())
            {
                return Err(format!(
                    "验证要求 {} 包含空的覆盖 requirement id",
                    req.requirement_id
                ));
            }
            match &req.scope {
                VerificationScopeV1::ArtifactRefs { artifact_ids }
                    if artifact_ids.is_empty()
                        || artifact_ids.iter().any(|id| id.trim().is_empty()) =>
                {
                    return Err(format!(
                        "验证要求 {} 的 artifact scope 为空或含空引用",
                        req.requirement_id
                    ));
                }
                VerificationScopeV1::WorkspacePaths { relative_paths }
                    if relative_paths.is_empty()
                        || relative_paths.iter().any(|raw| {
                            let path = std::path::Path::new(raw);
                            path.as_os_str().is_empty()
                                || path.is_absolute()
                                || path.components().any(|part| {
                                    matches!(
                                        part,
                                        std::path::Component::ParentDir
                                            | std::path::Component::Prefix(_)
                                    )
                                })
                        }) =>
                {
                    return Err(format!(
                        "验证要求 {} 的 workspace scope 为空或越界",
                        req.requirement_id
                    ));
                }
                _ => {}
            }
            if req.required
                && !matches!(&req.scope, VerificationScopeV1::Manual)
                && (req.resources.cpu_slots == 0 || req.resources.timeout_ms == 0)
            {
                return Err(format!(
                    "必需的自动验证要求 {} 必须声明正数 CPU 与超时预算",
                    req.requirement_id
                ));
            }
        }
        Ok(())
    }
}

/// Host-produced evidence receipt. Consumers must recheck these hashes against
/// the accepted snapshot; model output is never a source for this structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReceiptV1 {
    pub receipt_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub epoch: u64,
    pub requirement_id: String,
    pub validator_id: String,
    pub validator_version: String,
    #[serde(default)]
    pub arguments_sha256: String,
    pub input_sha256: String,
    #[serde(default)]
    pub environment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changeset_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Validated structured reviewer output, retained so receipt hashes are auditable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_result: Option<serde_json::Value>,
    #[serde(default)]
    pub subject_sha256: HashMap<String, String>,
    pub verdict: ValidationVerdictV1,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub started_at: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationVerdictV1 {
    Passed,
    Failed,
    Unsupported,
    Unverified,
    Stale,
    ManualAccepted,
}

fn preview(text: &str) -> String {
    let preview: String = text.chars().take(60).collect();
    if text.chars().count() > 60 {
        format!("{preview}…")
    } else {
        preview
    }
}

/// 计划步骤规格。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSpec {
    /// 步骤唯一 id。
    pub id: String,
    /// 前置依赖步骤 id（DAG 边）。
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// 可并行标记：与依赖无冲突的步骤允许并行（调度器还受 max_parallel 限制）。
    #[serde(default)]
    pub parallel: bool,
    /// worker 名称（由 WorkerRegistry 按名派发）。
    pub worker: String,
    /// 传给 worker 的输入规格（任意 JSON）。
    #[serde(default)]
    pub input: serde_json::Value,
    /// 兼容旧计划的输出断言；不代表行为验收。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerificationSpec>,
    /// 任务级验证计划；执行时必须由宿主注册表解析 validator_id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_plan: Option<VerificationPlanV1>,
    /// 重试次数（预算内；失败/验证失败重试）。
    #[serde(default)]
    pub retries: u32,
}

impl StepSpec {
    pub fn new(id: impl Into<String>, worker: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            depends_on: Vec::new(),
            parallel: false,
            worker: worker.into(),
            input: serde_json::Value::Null,
            verify: None,
            verification_plan: None,
            retries: 0,
        }
    }
}

/// 计划：步骤 DAG + 元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    /// 所属目标 id。
    pub goal_id: String,
    /// 计划描述（人读）。
    pub description: String,
    pub steps: Vec<StepSpec>,
    pub created_at: String,
}

impl Plan {
    pub fn new(id: impl Into<String>, goal_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            goal_id: goal_id.into(),
            description: String::new(),
            steps: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn add_step(&mut self, step: StepSpec) {
        self.steps.push(step);
    }

    pub fn step(&self, id: &str) -> Option<&StepSpec> {
        self.steps.iter().find(|s| s.id == id)
    }

    pub fn step_mut(&mut self, id: &str) -> Option<&mut StepSpec> {
        self.steps.iter_mut().find(|s| s.id == id)
    }

    /// 校验：步骤 id 唯一、依赖存在、无环（DFS 三色标记）。
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        for step in &self.steps {
            if !seen.insert(step.id.as_str()) {
                return Err(format!("步骤 id 重复：{}", step.id));
            }
            if let Some(verification_plan) = &step.verification_plan {
                verification_plan
                    .validate()
                    .map_err(|error| format!("步骤 {} 验证计划非法：{error}", step.id))?;
            }
            for dep in &step.depends_on {
                if dep == &step.id {
                    return Err(format!("步骤 {} 依赖自身", step.id));
                }
                if !self.step(dep).is_some() {
                    return Err(format!("步骤 {} 依赖不存在的步骤 {}", step.id, dep));
                }
            }
        }
        if let Some(cycle) = self.find_cycle() {
            return Err(format!("计划存在环：{}", cycle.join(" → ")));
        }
        Ok(())
    }

    /// 环检测（DFS 三色），返回构成环的步骤路径；无环返回 None。
    pub fn find_cycle(&self) -> Option<Vec<String>> {
        #[derive(Clone, Copy, PartialEq)]
        enum Color {
            White,
            Gray,
            Black,
        }
        let ids: Vec<String> = self.steps.iter().map(|s| s.id.clone()).collect();
        let mut colors: HashMap<&str, Color> =
            ids.iter().map(|id| (id.as_str(), Color::White)).collect();
        let mut stack: Vec<String> = Vec::new();

        fn dfs<'a>(
            plan: &'a Plan,
            id: &'a str,
            colors: &mut HashMap<&'a str, Color>,
            stack: &mut Vec<String>,
        ) -> Option<Vec<String>> {
            colors.insert(id, Color::Gray);
            stack.push(id.to_string());
            let step = plan.step(id)?;
            for dep in &step.depends_on {
                match colors.get(dep.as_str()) {
                    Some(Color::Gray) => {
                        // 找到环：从栈中 dep 位置截取。
                        let pos = stack.iter().position(|s| s == dep)?;
                        let mut cycle = stack[pos..].to_vec();
                        cycle.push(dep.clone());
                        return Some(cycle);
                    }
                    Some(Color::White) => {
                        if let Some(cycle) = dfs(plan, dep, colors, stack) {
                            return Some(cycle);
                        }
                    }
                    _ => {}
                }
            }
            stack.pop();
            colors.insert(id, Color::Black);
            None
        }

        for id in &ids {
            if colors.get(id.as_str()) == Some(&Color::White) {
                if let Some(cycle) = dfs(self, id, &mut colors, &mut stack) {
                    return Some(cycle);
                }
            }
        }
        None
    }

    /// 拓扑排序（wave 分层）：第 i 层 = 依赖都在 <i 层且不依赖同层/后层的步骤。
    /// 返回每层的步骤 id 列表（层内可并行；层间串行屏障）。
    pub fn topological_waves(&self) -> Result<Vec<Vec<String>>, String> {
        self.validate()?;
        let ids: Vec<&StepSpec> = self.steps.iter().collect();
        let mut wave_of: HashMap<&str, usize> = HashMap::new();
        // 迭代到不动点：wave = 1 + max(dep waves)；无依赖 = 1。
        loop {
            let mut changed = false;
            for step in &ids {
                let mut wave = 1usize;
                for dep in &step.depends_on {
                    if let Some(dep_wave) = wave_of.get(dep.as_str()) {
                        wave = wave.max(dep_wave + 1);
                    } else {
                        wave = usize::MAX; // 依赖尚未定层（有环时会卡住）
                    }
                }
                if wave != usize::MAX {
                    let current = wave_of.get(step.id.as_str()).copied();
                    if current != Some(wave) {
                        wave_of.insert(step.id.as_str(), wave);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        // 校验全部定层（否则有环，但 validate 已拦）。
        if wave_of.len() != ids.len() {
            return Err("拓扑排序失败：存在环或缺失依赖".to_string());
        }
        let max_wave = wave_of.values().copied().max().unwrap_or(0);
        let mut waves = vec![Vec::new(); max_wave];
        for step in &ids {
            let wave = wave_of[step.id.as_str()];
            waves[wave - 1].push(step.id.clone());
        }
        Ok(waves)
    }

    /// 序列化持久化：`<dir>/<plan_id>.json`（原子写：崩溃不留下半截计划）。
    pub fn persist(&self, dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建计划目录失败：{e}"))?;
        let path = dir.join(format!("{}.json", self.id));
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("计划序列化失败：{e}"))?;
        let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
        std::fs::write(&tmp, json).map_err(|e| format!("计划写入失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            format!("计划原子覆盖失败：{error}")
        })?;
        Ok(path)
    }

    /// 从磁盘加载计划。
    pub fn load(dir: &Path, plan_id: &str) -> Result<Plan, String> {
        let path = dir.join(format!("{plan_id}.json"));
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("计划 {plan_id} 读取失败：{e}（{path:?}）"))?;
        serde_json::from_str(&json).map_err(|e| format!("计划 {plan_id} 解析失败：{e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> Plan {
        let mut plan = Plan::new("p1", "g1");
        plan.add_step(StepSpec::new("a", "w1"));
        let mut b = StepSpec::new("b", "w2");
        b.depends_on = vec!["a".into()];
        b.verify = Some(VerificationSpec::OutputNonEmpty);
        plan.add_step(b);
        plan
    }

    #[test]
    fn plan_validate_ok() {
        assert!(sample_plan().validate().is_ok());
    }

    #[test]
    fn plan_validate_detects_cycle() {
        let mut plan = sample_plan();
        plan.add_step(StepSpec::new("c", "w3"));
        let mut d = StepSpec::new("d", "w4");
        d.depends_on = vec!["c".into()];
        plan.add_step(d);
        plan.step_mut("c").unwrap().depends_on = vec!["d".into()];
        let error = plan.validate().unwrap_err();
        assert!(error.contains("环"), "{error}");
        let cycle = plan.find_cycle().unwrap();
        assert!(cycle.len() >= 2, "{cycle:?}");
    }

    #[test]
    fn plan_validate_missing_dependency() {
        let mut plan = sample_plan();
        plan.step_mut("b").unwrap().depends_on = vec!["missing".into()];
        let error = plan.validate().unwrap_err();
        assert!(error.contains("不存在的步骤"));
    }

    #[test]
    fn plan_validate_duplicate_id() {
        let mut plan = sample_plan();
        plan.add_step(StepSpec::new("a", "w9"));
        let error = plan.validate().unwrap_err();
        assert!(error.contains("重复"));
    }

    #[test]
    fn topological_waves_three_parallel_join() {
        let mut plan = Plan::new("p2", "g1");
        for id in ["a", "b", "c"] {
            plan.add_step(StepSpec::new(id, "w1"));
        }
        let mut join = StepSpec::new("join", "w2");
        join.depends_on = vec!["a".into(), "b".into(), "c".into()];
        join.parallel = true;
        plan.add_step(join);
        let waves = plan.topological_waves().unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].len(), 3, "前 3 步应同层（可并行）：{waves:?}");
        assert_eq!(waves[1], vec!["join"]);
    }

    #[test]
    fn topological_waves_chain_is_serial() {
        let mut plan = Plan::new("p3", "g1");
        let mut prev: Option<String> = None;
        for i in 0..4 {
            let mut step = StepSpec::new(format!("s{i}"), "w1");
            if let Some(p) = &prev {
                step.depends_on = vec![p.clone()];
            }
            prev = Some(step.id.clone());
            plan.add_step(step);
        }
        let waves = plan.topological_waves().unwrap();
        assert_eq!(waves.len(), 4, "链式依赖应逐层：{waves:?}");
        for (i, wave) in waves.iter().enumerate() {
            assert_eq!(wave, &vec![format!("s{i}")]);
        }
    }

    #[test]
    fn plan_serde_roundtrip() {
        let plan = sample_plan();
        let json = serde_json::to_string(&plan).unwrap();
        let restored: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, plan.id);
        assert_eq!(restored.steps.len(), plan.steps.len());
        assert_eq!(
            restored.steps[1].verify,
            Some(VerificationSpec::OutputNonEmpty)
        );
    }

    #[test]
    fn plan_persist_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("owo-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plan = sample_plan();
        plan.persist(&dir).unwrap();
        let restored = Plan::load(&dir, "p1").unwrap();
        assert_eq!(restored.steps.len(), plan.steps.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verification_plan_rejects_duplicate_ids_and_unbounded_workspace_scope() {
        let requirement = VerificationRequirementV1 {
            requirement_id: "tests".into(),
            covers_requirement_ids: vec!["REQ-1".into()],
            validator_id: "cargo-test-v1".into(),
            validator_version: Some("1".into()),
            scope: VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["crates/app".into()],
            },
            arguments: serde_json::json!({"target": "app"}),
            required: true,
            resources: VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 1024,
                exclusive_workspace: true,
                timeout_ms: 60_000,
            },
        };
        let mut plan = VerificationPlanV1 {
            plan_id: "verify-1".into(),
            requirements: vec![requirement.clone()],
        };
        assert!(plan.validate().is_ok());
        plan.requirements.push(requirement.clone());
        assert!(plan.validate().unwrap_err().contains("重复"));

        plan.requirements.truncate(1);
        plan.requirements[0].scope = VerificationScopeV1::WorkspacePaths {
            relative_paths: vec!["../outside".into()],
        };
        assert!(plan.validate().unwrap_err().contains("越界"));
    }

    #[test]
    fn verification_plan_requires_bounded_resources_for_required_automated_check() {
        let mut plan = VerificationPlanV1 {
            plan_id: "verify-2".into(),
            requirements: vec![VerificationRequirementV1 {
                requirement_id: "REQ-1".into(),
                covers_requirement_ids: vec!["REQ-1".into()],
                validator_id: "cargo-test-v1".into(),
                validator_version: Some("1".into()),
                scope: VerificationScopeV1::StepOutput,
                arguments: serde_json::json!({}),
                required: true,
                resources: VerificationResourcesV1::default(),
            }],
        };
        assert!(plan.validate().unwrap_err().contains("超时预算"));
        plan.requirements[0].scope = VerificationScopeV1::Manual;
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn validation_receipt_roundtrips_with_explicit_non_passing_verdict() {
        let receipt = ValidationReceiptV1 {
            receipt_id: "receipt-1".into(),
            task_id: "task-1".into(),
            attempt_id: "attempt-2".into(),
            epoch: 3,
            requirement_id: "REQ-1".into(),
            validator_id: "cargo-test-v1".into(),
            validator_version: "1".into(),
            arguments_sha256: "b".repeat(64),
            input_sha256: "a".repeat(64),
            environment_id: "test".into(),
            changeset_sha256: None,
            detail: Some("not registered".into()),
            subject_sha256: HashMap::new(),
            verdict: ValidationVerdictV1::Unsupported,
            evidence_refs: vec![],
            review_result: None,
            started_at: "start".into(),
            completed_at: "end".into(),
        };
        let encoded = serde_json::to_vec(&receipt).unwrap();
        let decoded: ValidationReceiptV1 = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, receipt);
        assert_ne!(decoded.verdict, ValidationVerdictV1::Passed);
        let mut legacy = serde_json::to_value(&receipt).unwrap();
        legacy.as_object_mut().unwrap().remove("review_result");
        let legacy: ValidationReceiptV1 = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.review_result, None);
    }

    #[test]
    fn verify_output_semantics() {
        assert!(verify_output(
            &VerificationSpec::OutputContains("ok".into()),
            "everything ok"
        )
        .is_ok());
        assert!(verify_output(&VerificationSpec::OutputContains("ok".into()), "nope").is_err());
        assert!(verify_output(&VerificationSpec::OutputEquals("x".into()), "x").is_ok());
        assert!(verify_output(&VerificationSpec::OutputEquals("x".into()), "y").is_err());
        assert!(verify_output(&VerificationSpec::OutputNonEmpty, "  ").is_err());
        assert!(verify_output(&VerificationSpec::OutputNonEmpty, "data").is_ok());
        let custom = verify_output(&VerificationSpec::Custom("x".into()), "data");
        assert!(custom.is_err());
        assert!(custom.unwrap_err().contains("未注册"));
    }
}

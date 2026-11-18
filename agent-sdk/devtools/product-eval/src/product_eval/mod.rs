//! ProductEval（V1-R1）：固定真实任务集 × 重复次数的产品级评测底座。
//!
//! 与 [`crate::eval`] 的分工：`eval` 面向工程回归（内置 demo 单轮套件），
//! 本模块面向**产品成功率**——每个任务固定输入 fixture、允许读写范围、
//! 预期 Artifact、自动检查器与超时/预算/权限；单 Agent 与多 Agent 在
//! **完全相同的任务、输入、权限、预算与检查器**下对照执行；矩阵支持
//! 中断续跑（不重复已完成 case），失败运行永久进入报告（禁止删除后重算成功率）。
//!
//! 执行模式：
//! - `dry`（参考回放执行器）：零模型调用、本地确定性——把任务内置
//!   `reference_outputs` 写入沙盒后交检查器判定，自检检查器可达性；
//! - `live`（生成式执行器）：经 [`ModelProvider`] 网关真实生成文件块，
//!   受控落盘后过检，产出真实的墙钟/调用次数/token 统计。

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use owo_agent_core::gateway::{
    ChatMessage, ModelOutput, ModelProvider, OpenAiCompatibleConfig, TokenUsage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

mod paired_report;
pub use paired_report::{
    build_paired_report_json, split_paired_mode_reports, PairedReportOptions,
    PAIRED_REPORT_SCHEMA_VERSION,
};
#[cfg(test)]
use paired_report::{paired_cell_deltas_json, paired_snapshot_json};

mod freeze;
pub use freeze::{
    build_freeze_json, parse_freeze, permissions_hash, verify_freeze, FREEZE_SCHEMA_VERSION,
};

mod matrix_runner;
pub use matrix_runner::{filter_cases, MatrixRunner, RunOptions};

/// 当前产品评测任务 schema 版本。
pub const PRODUCT_EVAL_SCHEMA_VERSION: u32 = 1;

/// 单文件草稿/评审提示词的最大保留字符数（防止超长上下文）。
const PROMPT_TEXT_CAP: usize = 1600;
/// Successful runs retain only registered artifacts, with fixed disk bounds.
const ARTIFACT_SNAPSHOT_MAX_FILES: usize = 128;
const ARTIFACT_SNAPSHOT_MAX_PATH_BYTES: usize = 4096;
const ARTIFACT_SNAPSHOT_MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const ARTIFACT_SNAPSHOT_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

// ---------------------------------------------------------------------------
// 任务定义类型（suite.json / tasks/*.json 的 schema）
// ---------------------------------------------------------------------------

/// 任务领域分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvalCategory {
    Code,
    Research,
    Document,
    Desktop,
    #[serde(rename = "long_horizon")]
    LongHorizon,
}

impl EvalCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            EvalCategory::Code => "code",
            EvalCategory::Research => "research",
            EvalCategory::Document => "document",
            EvalCategory::Desktop => "desktop",
            EvalCategory::LongHorizon => "long_horizon",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "code" => Ok(EvalCategory::Code),
            "research" => Ok(EvalCategory::Research),
            "document" => Ok(EvalCategory::Document),
            "desktop" => Ok(EvalCategory::Desktop),
            "long_horizon" => Ok(EvalCategory::LongHorizon),
            other => Err(format!(
                "未知分类「{other}」（可选 code/research/document/desktop/long_horizon）"
            )),
        }
    }
}

impl fmt::Display for EvalCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 对照拓扑：同一任务定义在两种模式下使用相同的输入/权限/预算/检查器。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentMode {
    Single,
    Multi,
}

impl AgentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentMode::Single => "single",
            AgentMode::Multi => "multi",
        }
    }

    /// 矩阵默认拓扑：单/多对照都跑。
    pub fn all() -> [AgentMode; 2] {
        [AgentMode::Single, AgentMode::Multi]
    }

    pub fn parse_list(spec: &str) -> Result<Vec<AgentMode>, String> {
        let mut modes = Vec::new();
        for part in spec.split(',') {
            match part.trim() {
                "single" => modes.push(AgentMode::Single),
                "multi" => modes.push(AgentMode::Multi),
                other => return Err(format!("未知 Agent 拓扑「{other}」（可选 single/multi）")),
            }
        }
        if modes.is_empty() {
            return Err("Agent 拓扑列表为空".to_string());
        }
        Ok(modes)
    }
}

impl fmt::Display for AgentMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 套件级默认值（任务级字段可逐项覆盖）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteDefaults {
    /// 默认重复次数（V1 目标：每任务 ×20）。
    #[serde(default = "default_repetitions")]
    pub repetitions: u32,
    /// 默认单次运行超时（秒）。
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// 默认单次运行模型调用预算。
    #[serde(default = "default_max_model_calls")]
    pub max_model_calls: u32,
}

impl Default for SuiteDefaults {
    fn default() -> Self {
        Self {
            repetitions: default_repetitions(),
            timeout_secs: default_timeout_secs(),
            max_model_calls: default_max_model_calls(),
        }
    }
}

fn default_repetitions() -> u32 {
    20
}
fn default_timeout_secs() -> u64 {
    180
}
fn default_max_model_calls() -> u32 {
    6
}

/// Opt-in for wall-clock benchmark runs: the outer timeout, not an arbitrary call
/// count, ends the run. Normal product-eval suites retain their explicit budgets.
pub(crate) fn unbounded_benchmark_calls() -> bool {
    std::env::var("OWO_PRODUCT_EVAL_UNBOUNDED_CALLS")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

/// 输入 fixture：执行前写入沙盒工作区的文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputFixture {
    pub path: String,
    pub content: String,
}

/// 自动检查器：对一个已完成运行的沙盒产物做判定。
mod checkers;

pub use checkers::*;
use checkers::{evaluate_all, MapSource};

/// 长程任务的故障注入声明（V3 产品基准：restart / interrupt / requirement-change /
/// transient-error / approval-wait）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultInjection {
    /// 注入类型；静态校验只接受 [`FAULT_INJECTION_KINDS`] 中的五种。
    pub kind: String,
    /// 在第几步之后注入（可选，仅在声明时用于执行器定位）。
    #[serde(default)]
    pub after_step: Option<u32>,
    /// 注入说明/预期用户可见行为。
    #[serde(default)]
    pub note: String,
}

/// 允许的故障注入类型。
pub const FAULT_INJECTION_KINDS: [&str; 5] = [
    "restart",
    "interrupt",
    "requirement-change",
    "transient-error",
    "approval-wait",
];

/// 一个固定产品评测任务（tasks/*.json 的顶层结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductEvalCase {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 全局唯一 ID（kebab-case）。
    pub id: String,
    pub category: EvalCategory,
    pub title: String,
    /// 下发给执行代理的任务说明（live 模式的用户提示词主体）。
    pub instruction: String,
    /// 输入 fixture：执行前写入沙盒。
    #[serde(default)]
    pub inputs: Vec<InputFixture>,
    /// 允许读取范围（相对路径 glob 片段）；inputs 必须全部落在其中。
    #[serde(default)]
    pub allow_read: Vec<String>,
    /// 允许写入范围；expected_artifacts 必须全部落在其中。默认 deny。
    #[serde(default)]
    pub allow_write: Vec<String>,
    /// 预期 Artifact（相对路径），必须在 allow_write 内。
    pub expected_artifacts: Vec<String>,
    /// 自动检查器集合。
    pub checkers: Vec<ArtifactChecker>,
    /// 参考输出：dry 回放模式写入这些内容用于自检检查器可达性。
    #[serde(default)]
    pub reference_outputs: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_model_calls: Option<u32>,
    #[serde(default)]
    pub repetitions: Option<u32>,
    /// 命令白名单（run_command 受控执行）：整条命令需精确或前缀命中其一。
    /// 空列表 = 本任务禁止执行任何命令（默认 deny）。
    #[serde(default)]
    pub allow_commands: Vec<String>,
    /// 失败分类标签（如 wrong-output / missing-artifact / permission-denied /
    /// timeout / unapproved-side-effect）。V3 起要求非空；旧套件缺省为空。
    #[serde(default)]
    pub failure_classes: Vec<String>,
    /// 允许的副作用范围（如 write:out/**、command:python tests/check.py、
    /// read:sources/**）。V3 起要求非空；旧套件缺省为空。
    #[serde(default)]
    pub expected_side_effects: Vec<String>,
    /// 长程任务的故障注入声明；普通任务可省略。
    #[serde(default)]
    pub fault_injection: Option<FaultInjection>,
}

fn default_schema_version() -> u32 {
    PRODUCT_EVAL_SCHEMA_VERSION
}

impl ProductEvalCase {
    /// 生效重复次数：CLI 覆盖 > 任务级 > 套件默认。
    pub fn effective_repetitions(
        &self,
        defaults: &SuiteDefaults,
        cli_override: Option<u32>,
    ) -> u32 {
        cli_override
            .or(self.repetitions)
            .unwrap_or(defaults.repetitions)
            .clamp(1, 100)
    }

    pub fn effective_timeout_secs(&self, defaults: &SuiteDefaults) -> u64 {
        self.timeout_secs.unwrap_or(defaults.timeout_secs).max(1)
    }

    pub fn effective_max_model_calls(&self, defaults: &SuiteDefaults) -> u32 {
        let calls = self
            .max_model_calls
            .unwrap_or(defaults.max_model_calls)
            .max(1);
        if unbounded_benchmark_calls() {
            calls
        } else {
            calls.min(64)
        }
    }
}

/// 套件（suite.json）。tasks 为相对本文件的 JSON 任务路径。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductEvalSuite {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub defaults: SuiteDefaults,
    pub tasks: Vec<String>,
}

/// 套件与其所在目录（解析 tasks 相对路径的锚点）。
#[derive(Debug, Clone)]
pub struct SuiteBundle {
    pub dir: PathBuf,
    pub suite: ProductEvalSuite,
    pub cases: Vec<ProductEvalCase>,
}

// ---------------------------------------------------------------------------
// 运行记录 / 报告 / 指标
// ---------------------------------------------------------------------------

/// 单次重复运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Passed,
    Failed,
    Error,
    Timeout,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Passed => "passed",
            RunStatus::Failed => "failed",
            RunStatus::Error => "error",
            RunStatus::Timeout => "timeout",
            RunStatus::Cancelled => "cancelled",
        }
    }
}

/// 矩阵中的一个单元格：`(case_id, agent_mode, repetition)`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MatrixKey {
    pub case_id: String,
    pub agent_mode: AgentMode,
    pub repetition: u32,
}

impl MatrixKey {
    pub fn new(case_id: impl Into<String>, agent_mode: AgentMode, repetition: u32) -> Self {
        Self {
            case_id: case_id.into(),
            agent_mode,
            repetition,
        }
    }

    pub fn slug(&self) -> String {
        format!(
            "{}#{}#{}",
            self.case_id,
            self.agent_mode.as_str(),
            self.repetition
        )
    }
}

impl fmt::Display for MatrixKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.slug())
    }
}

/// 一次运行的完整记录（journal 的最小单元；失败记录同样永久保留）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductEvalRun {
    pub key: MatrixKey,
    pub category: EvalCategory,
    pub status: RunStatus,
    pub wall_ms: u64,
    /// Wall time inside CaseExecutor::execute. For Team, this includes inline host checks
    /// and DeliveryGate; for Single, the outer checker pass is measured separately.
    #[serde(default)]
    pub executor_wall_ms: u64,
    /// Time spent in host-side product checkers, including retries and fallback checks.
    /// This can overlap executor_wall_ms for Team and must not be added to total wall time.
    #[serde(default)]
    pub validation_wall_ms: u64,
    /// Time spent committing the Team DeliveryGate (zero for Single); nested in executor wall.
    #[serde(default)]
    pub delivery_gate_wall_ms: u64,
    pub model_calls: u32,
    /// Exact tool invocation count when reported by the executor; absent for legacy/unknown runs.
    #[serde(default)]
    pub tool_calls: Option<u32>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    /// 失败步骤（检查器描述 / 执行器阶段名）。
    pub failed_steps: Vec<String>,
    pub retries: u32,
    pub cancellations: u32,
    /// 最终 Artifact 引用（沙盒内相对路径）。
    pub artifact_refs: Vec<String>,
    /// 真实工具调用轨迹（单 Agent 执行器填写：工具 + 实参摘要 + 结果；旧记录缺省为空）。
    #[serde(default)]
    pub tool_log: Vec<String>,
    /// 通过/总检查器数（质量分 = passed/total；0/0 = 无质量维度）。
    #[serde(default)]
    pub checker_passed: u32,
    #[serde(default)]
    pub checker_total: u32,
    /// 沙盒/失败留档目录相对 out 根的位置（成功运行清理原沙盒，故为 None）。
    #[serde(default)]
    pub sandbox_rel: Option<String>,
    /// 成功运行的登记产物快照目录（相对 out 根）；旧记录缺省为空。
    #[serde(default)]
    pub artifact_snapshot_rel: Option<String>,
    pub model: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub error: Option<String>,
    /// 绑定矩阵单元的外部驱动证据；历史记录缺省为空，正式计分一律拒绝。
    #[serde(default)]
    pub external_evidence: Option<serde_json::Value>,
}

/// 聚合指标。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProductEvalMetrics {
    pub runs_total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: usize,
    pub timeouts: usize,
    pub cancelled: usize,
    /// 成功率分母为全部已尝试运行（失败/错误/超时一律计入，禁止剔除重算）。
    pub success_rate: f64,
    pub mean_wall_ms: f64,
    #[serde(default)]
    pub mean_executor_wall_ms: f64,
    #[serde(default)]
    pub mean_validation_wall_ms: f64,
    #[serde(default)]
    pub mean_delivery_gate_wall_ms: f64,
    pub total_model_calls: u64,
    /// None unless every included run has an exact tool-call count.
    #[serde(default)]
    pub total_tool_calls: Option<u64>,
    /// 仅在**全部**运行都有 usage 时给出总和；None 表示实际消耗未知（不是 0）。
    pub total_tokens: Option<u64>,
    /// 已知 usage 的已执行运行数 / 缺失 usage 的运行数（含 fail-closed 的 error 格）。
    #[serde(default)]
    pub usage_known_runs: usize,
    #[serde(default)]
    pub usage_unknown_runs: usize,
    /// 已执行格的已知 token 合计：存在缺失 usage 时仍给出**下界**，
    /// 便于报告区分「没有数据」与「部分缺失」。
    #[serde(default)]
    pub known_total_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

/// 按 (case_id, mode) 分组的细分统计。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseModeMetrics {
    pub case_id: String,
    pub category: EvalCategory,
    pub agent_mode: AgentMode,
    pub runs_total: usize,
    pub passed: usize,
    pub success_rate: f64,
    pub mean_wall_ms: f64,
    pub mean_model_calls: f64,
    /// None when any run in the group has unknown tool-call telemetry.
    #[serde(default)]
    pub mean_tool_calls: Option<f64>,
    pub total_tokens: Option<u64>,
    /// 检查器通过率均值（质量代理；无检查器计数的旧记录缺省为 None）。
    #[serde(default)]
    pub quality: Option<f64>,
}

/// 完整报告：聚合**全部** journal 记录（含失败）+ 未完成单元格清单。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductEvalReport {
    pub schema_version: u32,
    pub suite_name: String,
    pub suite_hash: String,
    pub execution: String,
    pub model: Option<String>,
    /// 批次标签（正式验收批次隔离用，同目录只允许同一批次）。
    #[serde(default)]
    pub batch_label: Option<String>,
    /// 附加标签（CLI --tag 可重复；进报告供溯源）。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 验证/修复组允许的追加修复次数（0 = 单次执行，旧报告缺省视为 0）。
    #[serde(default)]
    pub repair_attempts: u32,
    /// Fingerprint of effective task, permission, checker and budget configuration.
    /// Older reports lack this and cannot be used as paired benefit evidence.
    #[serde(default)]
    pub run_contract_sha256: Option<String>,
    /// SHA-256 of the executable that produced the report.
    #[serde(default)]
    pub evaluator_binary_sha256: Option<String>,
    /// SHA-256 of the configured provider endpoint; the URL itself is never stored.
    #[serde(default)]
    pub provider_endpoint_sha256: Option<String>,
    pub generated_at: String,
    pub runs: Vec<ProductEvalRun>,
    /// 计划中但尚未完成的矩阵单元格（中断续跑的目标集）。
    pub pending: Vec<MatrixKey>,
    pub metrics: ProductEvalMetrics,
    pub per_case: Vec<CaseModeMetrics>,
    /// Shared automatic Team enablement verdict; older reports may omit it.
    #[serde(default)]
    pub comparison: Option<ModeComparison>,
}

/// out 目录元信息（续跑兼容性校验）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunDirMeta {
    schema_version: u32,
    suite_name: String,
    suite_hash: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    execution: Option<String>,
    #[serde(default)]
    batch_label: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    /// Missing fingerprints make old run directories non-resumable without --fresh.
    #[serde(default)]
    run_contract_sha256: Option<String>,
    #[serde(default)]
    evaluator_binary_sha256: Option<String>,
    #[serde(default)]
    provider_endpoint_sha256: Option<String>,
    /// 外部驱动注册身份；续跑必须一致，但不改变历史 run_contract_sha256。
    #[serde(default)]
    external_driver_identity_sha256: Option<String>,
    created_at: String,
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// ProductEval 顶层错误（消息已含上下文，直接面向 CLI 展示）。
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProductEvalError(pub String);

fn err<T>(msg: impl Into<String>) -> Result<T, ProductEvalError> {
    Err(ProductEvalError(msg.into()))
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn current_executable_sha256() -> Option<String> {
    let executable = std::env::current_exe().ok()?;
    let mut file = std::fs::File::open(executable).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn truncate_for_log(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out.replace('\n', "\\n")
}

// ---------------------------------------------------------------------------
// 路径工具 / 权限范围
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 套件加载 / 校验
// ---------------------------------------------------------------------------

/// 解析 --suite 参数：None 时从 cwd 向上查找 `agent-sdk/evals/v1/suite.json`。
pub fn resolve_suite_input(spec: Option<&Path>) -> Result<PathBuf, ProductEvalError> {
    if let Some(path) = spec {
        return Ok(path.to_path_buf());
    }
    let mut current =
        std::env::current_dir().map_err(|e| ProductEvalError(format!("无法获取当前目录：{e}")))?;
    for _ in 0..8 {
        let candidate = current
            .join("agent-sdk")
            .join("evals")
            .join("v1")
            .join("suite.json");
        if candidate.is_file() {
            return Ok(candidate);
        }
        let nested = current.join("evals").join("v1").join("suite.json");
        if nested.is_file() {
            return Ok(nested);
        }
        if !current.pop() {
            break;
        }
    }
    err("未找到套件：请用 --suite 指定 suite.json（缺省查找 agent-sdk/evals/v1/suite.json）")
}

/// 从 suite.json 加载套件及全部任务定义。
pub fn load_suite(suite_spec: &Path) -> Result<SuiteBundle, ProductEvalError> {
    let suite_file = if suite_spec.is_dir() {
        suite_spec.join("suite.json")
    } else {
        suite_spec.to_path_buf()
    };
    if !suite_file.is_file() {
        return err(format!("suite.json 不存在：{}", suite_file.display()));
    }
    let dir = suite_file
        .parent()
        .ok_or_else(|| ProductEvalError("suite.json 缺少父目录".into()))?
        .to_path_buf();
    let text = std::fs::read_to_string(&suite_file)
        .map_err(|e| ProductEvalError(format!("读取 {} 失败：{e}", suite_file.display())))?;
    let suite: ProductEvalSuite = serde_json::from_str(&text).map_err(|e| {
        ProductEvalError(format!(
            "解析 {} 失败（schema 不符或字段拼写错误）：{e}",
            suite_file.display()
        ))
    })?;
    if suite.schema_version != PRODUCT_EVAL_SCHEMA_VERSION {
        return err(format!(
            "套件 schema 版本不匹配：期望 {PRODUCT_EVAL_SCHEMA_VERSION}，实际 {}",
            suite.schema_version
        ));
    }
    if suite.name.trim().is_empty() {
        return err("套件 name 不能为空");
    }
    if suite.tasks.is_empty() {
        return err("套件 tasks 为空");
    }
    let mut cases = Vec::new();
    for rel in &suite.tasks {
        let task_path = dir.join(rel);
        let task_text = std::fs::read_to_string(&task_path)
            .map_err(|e| ProductEvalError(format!("读取任务 {rel} 失败：{e}")))?;
        let case: ProductEvalCase = serde_json::from_str(&task_text)
            .map_err(|e| ProductEvalError(format!("解析任务 {rel} 失败（schema 不符）：{e}")))?;
        cases.push(case);
    }
    Ok(SuiteBundle { dir, suite, cases })
}

/// 单任务校验；`seen_ids` 用于跨任务查重。
pub fn validate_case(
    case: &ProductEvalCase,
    seen_ids: &mut std::collections::BTreeSet<String>,
) -> Vec<String> {
    let mut issues = Vec::new();

    if case.schema_version != PRODUCT_EVAL_SCHEMA_VERSION {
        issues.push(format!(
            "schema_version 期望 {PRODUCT_EVAL_SCHEMA_VERSION}，实际 {}",
            case.schema_version
        ));
    }
    let id_ok = !case.id.is_empty()
        && case
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_ok {
        issues.push(format!("id「{}」必须是非空 kebab-case", case.id));
    }
    if !seen_ids.insert(case.id.clone()) {
        issues.push(format!("id「{}」在套件内重复", case.id));
    }
    if case.title.trim().is_empty() {
        issues.push("title 为空".to_string());
    }
    if case.instruction.trim().is_empty() {
        issues.push("instruction 为空".to_string());
    }

    // 规格一致性：检查器要求的 Markdown 标题必须能从题目推断——写在 instruction
    // 里，或原样出现在输入材料中（合并/转写类任务）。无修复组的模型无法猜出未
    // 声明的标题格式，整类失败会被误读成编码能力问题。
    let input_text: String = case
        .inputs
        .iter()
        .map(|input| input.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for checker in &case.checkers {
        if let ArtifactChecker::Contains { text, .. } = checker {
            let heading = text.trim_start();
            if heading.starts_with('#')
                && !case.instruction.contains(text.as_str())
                && !input_text.contains(text.as_str())
            {
                issues.push(format!(
                    "检查器要求的标题「{text}」既不在 instruction 也不在输入材料中"
                ));
            }
        }
    }

    // 产物与写入范围。
    if case.expected_artifacts.is_empty() {
        issues.push("expected_artifacts 为空：至少声明一个预期 Artifact".to_string());
    }
    let mut normalized_artifacts = Vec::new();
    for artifact in &case.expected_artifacts {
        match sanitize_rel_path(artifact) {
            Ok(norm) => {
                if in_scope(&case.allow_write, &norm) {
                    if !normalized_artifacts.contains(&norm) {
                        normalized_artifacts.push(norm);
                    }
                } else {
                    issues.push(format!(
                        "预期 Artifact「{artifact}」不在 allow_write 范围内（权限默认 deny）"
                    ));
                }
            }
            Err(e) => issues.push(format!("预期 Artifact「{artifact}」非法：{e}")),
        }
    }
    if case.allow_write.is_empty() {
        issues.push("allow_write 为空：未授予任何写范围".to_string());
    }

    // 命令白名单：条目必须非空且不得内嵌 shell 链式元字符（执行器同规则逐条强制）。
    for command in &case.allow_commands {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            issues.push("allow_commands 存在空白条目".to_string());
        } else if single_agent::command_has_shell_metachars(trimmed) {
            issues.push(format!(
                "allow_commands 条目「{trimmed}」含链式/重定向元字符（白名单按单命令收口）"
            ));
        }
    }

    // 故障注入：只做轻量静态校验（类型必须属于受支持的五种），不要求旧用例填写。
    if let Some(fault) = &case.fault_injection {
        let kind = fault.kind.trim();
        if !FAULT_INJECTION_KINDS.contains(&kind) {
            issues.push(format!(
                "fault_injection.kind「{}」不受支持（可选 restart/interrupt/requirement-change/transient-error/approval-wait）",
                fault.kind
            ));
        }
    }

    // 输入与读取范围。
    for input in &case.inputs {
        match sanitize_rel_path(&input.path) {
            Ok(norm) => {
                if !in_scope(&case.allow_read, &norm) {
                    issues.push(format!(
                        "输入 fixture「{}」不在 allow_read 范围内",
                        input.path
                    ));
                }
            }
            Err(e) => issues.push(format!("输入 fixture「{}」非法：{e}", input.path)),
        }
    }

    // 检查器。
    if case.checkers.is_empty() {
        issues.push("checkers 为空：至少需要一个自动检查器".to_string());
    }
    // 检查器目标可达性：预期产物、输入 fixture 或参考输出均可（行为/内容检查可指向
    // 修复后的源文件——如代码任务验证 src/ 的修改结果）。
    let mut known_files: std::collections::BTreeSet<String> =
        normalized_artifacts.iter().cloned().collect();
    for fixture in &case.inputs {
        if let Ok(norm) = sanitize_rel_path(&fixture.path) {
            known_files.insert(norm);
        }
    }
    for key in case.reference_outputs.keys() {
        if let Ok(norm) = sanitize_rel_path(key) {
            known_files.insert(norm);
        }
    }
    for checker in &case.checkers {
        issues.extend(checker.static_issues());
        let path = match checker {
            ArtifactChecker::Exists { path }
            | ArtifactChecker::Contains { path, .. }
            | ArtifactChecker::NotContains { path, .. }
            | ArtifactChecker::Regex { path, .. }
            | ArtifactChecker::LineCountMin { path, .. }
            | ArtifactChecker::JsonFieldEquals { path, .. }
            | ArtifactChecker::JsonKeysExact { path, .. } => Some(path.clone()),
            ArtifactChecker::CommandCheck { .. } => None,
        };
        if let Some(path) = path {
            if let Ok(norm) = sanitize_rel_path(&path) {
                if !known_files.contains(&norm) {
                    issues.push(format!(
                        "检查器目标「{path}」不在 expected_artifacts / 输入 fixture / 参考输出 中"
                    ));
                }
            }
        }
    }

    // 参考输出覆盖所有预期产物，且参考输出必须通过全部检查器（dry 可达性自检）。
    for artifact in &normalized_artifacts {
        if !case.reference_outputs.contains_key(artifact) {
            issues.push(format!("reference_outputs 缺少预期产物「{artifact}」"));
        }
    }
    let mut refs = case.reference_outputs.clone();
    for fixture in &case.inputs {
        if let Ok(norm) = sanitize_rel_path(&fixture.path) {
            // 参考输出优先（代表"修复后/完成后"状态）；fixture 只在参考未覆盖时补位。
            refs.entry(norm).or_insert_with(|| fixture.content.clone());
        }
    }
    let source = MapSource(&refs);
    let (_, failed) = evaluate_all(&case.checkers, &source);
    for reason in failed {
        issues.push(format!("参考输出未通过检查器：{reason}"));
    }

    // 数值范围。
    if let Some(reps) = case.repetitions {
        if !(1..=100).contains(&reps) {
            issues.push(format!("repetitions={reps} 超出 [1,100]"));
        }
    }
    if let Some(secs) = case.timeout_secs {
        if secs < 5 {
            issues.push(format!("timeout_secs={secs} 过小（≥5）"));
        }
    }
    if let Some(calls) = case.max_model_calls {
        if calls == 0 || (calls > 64 && !unbounded_benchmark_calls()) {
            issues.push(format!(
                "max_model_calls={calls} 超出允许范围（普通评测 1..=64；计时基准可用完整 u32 范围）"
            ));
        }
    }

    issues
}

/// 全套件校验（含跨任务查重）。
pub fn validate_suite(bundle: &SuiteBundle) -> SuiteValidation {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut tasks = Vec::new();
    for case in &bundle.cases {
        let issues = validate_case(case, &mut seen);
        let ok = issues.is_empty();
        tasks.push(TaskValidation {
            id: case.id.clone(),
            ok,
            issues,
        });
    }
    let ok_count = tasks.iter().filter(|task| task.ok).count();
    SuiteValidation {
        suite_name: bundle.suite.name.clone(),
        task_count: tasks.len(),
        ok_count,
        all_ok: ok_count == tasks.len(),
        tasks,
    }
}

/// 单任务校验结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskValidation {
    pub id: String,
    pub ok: bool,
    pub issues: Vec<String>,
}

/// 整套校验结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuiteValidation {
    pub suite_name: String,
    pub task_count: usize,
    pub ok_count: usize,
    pub all_ok: bool,
    pub tasks: Vec<TaskValidation>,
}

/// 套件指纹：对全部任务定义做规范序列化摘要，作为续跑兼容性凭据。
pub fn suite_hash(bundle: &SuiteBundle) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bundle.suite.name.as_bytes());
    for case in &bundle.cases {
        let canonical = serde_json::to_vec(case).unwrap_or_else(|_| case.id.as_bytes().to_vec());
        hasher.update(&canonical);
    }
    format!("{:x}", hasher.finalize())
}

/// 校验结果的人类可读清单（validate 子命令输出）。
pub fn format_validation(validation: &SuiteValidation) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "套件「{}」任务校验：{}/{} 通过\n",
        validation.suite_name, validation.ok_count, validation.task_count
    ));
    for task in &validation.tasks {
        let mark = if task.ok { "PASS" } else { "FAIL" };
        out.push_str(&format!("  [{mark}] {}\n", task.id));
        for issue in &task.issues {
            out.push_str(&format!("        - {issue}\n"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 执行器契约
// ---------------------------------------------------------------------------

/// 运行中可读取的原子遥测，供墙钟截止时保留已发生的工作量。
#[derive(Debug, Default)]
pub(crate) struct ExecProgress {
    model_calls: AtomicU32,
    prompt_tokens: AtomicU64,
    completion_tokens: AtomicU64,
    total_tokens: AtomicU64,
    usage_known: AtomicBool,
    usage_unknown: AtomicBool,
    team_workspace: Mutex<Option<PathBuf>>,
    prevalidated_checkers: Mutex<Option<(u32, u32)>>,
    validation_wall_ms: AtomicU64,
    delivery_gate_wall_ms: AtomicU64,
}

impl ExecProgress {
    pub(crate) fn record_model_call(&self) {
        self.model_calls.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_usage(&self, usage: TokenUsage) {
        self.prompt_tokens
            .fetch_add(usage.prompt_tokens, Ordering::Relaxed);
        self.completion_tokens
            .fetch_add(usage.completion_tokens, Ordering::Relaxed);
        self.total_tokens
            .fetch_add(usage.total_tokens, Ordering::Relaxed);
        self.usage_known.store(true, Ordering::Relaxed);
    }

    pub(crate) fn record_usage_unknown(&self) {
        self.usage_unknown.store(true, Ordering::Relaxed);
    }

    pub(crate) fn set_team_workspace(&self, path: PathBuf) {
        if let Ok(mut workspace) = self.team_workspace.lock() {
            *workspace = Some(path);
        }
    }

    pub(crate) fn record_validation_wall_ms(&self, elapsed_ms: u64) {
        self.validation_wall_ms
            .fetch_add(elapsed_ms, Ordering::Relaxed);
    }

    pub(crate) fn record_delivery_gate_wall_ms(&self, elapsed_ms: u64) {
        self.delivery_gate_wall_ms
            .fetch_add(elapsed_ms, Ordering::Relaxed);
    }

    pub(crate) fn mark_checkers_prevalidated(&self, passed: u32, total: u32) {
        if passed == total {
            if let Ok(mut result) = self.prevalidated_checkers.lock() {
                *result = Some((passed, total));
            }
        }
    }

    pub(crate) fn prevalidated_checkers(&self) -> Option<(u32, u32)> {
        self.prevalidated_checkers
            .lock()
            .ok()
            .and_then(|result| *result)
    }

    fn snapshot(&self) -> RawExecOutcome {
        let usage_known =
            self.usage_known.load(Ordering::Relaxed) && !self.usage_unknown.load(Ordering::Relaxed);
        RawExecOutcome {
            model_calls: self.model_calls.load(Ordering::Relaxed),
            usage: TokenUsage {
                prompt_tokens: self.prompt_tokens.load(Ordering::Relaxed),
                completion_tokens: self.completion_tokens.load(Ordering::Relaxed),
                total_tokens: self.total_tokens.load(Ordering::Relaxed),
            },
            usage_known,
            validation_wall_ms: self.validation_wall_ms.load(Ordering::Relaxed),
            delivery_gate_wall_ms: self.delivery_gate_wall_ms.load(Ordering::Relaxed),
            ..RawExecOutcome::default()
        }
    }

    fn copy_expected_artifacts(
        &self,
        case: &ProductEvalCase,
        destination: &Path,
    ) -> Result<Vec<String>, String> {
        let source = self
            .team_workspace
            .lock()
            .map_err(|_| "团队工作区快照锁中毒".to_string())?
            .clone();
        let Some(source) = source else {
            return Ok(Vec::new());
        };
        let mut copied = Vec::new();
        for raw in &case.expected_artifacts {
            let rel = sanitize_rel_path(raw)?;
            if !in_scope(&case.allow_write, &rel) {
                continue;
            }
            let from = source.join(&rel);
            if !from.is_file() {
                continue;
            }
            let to = destination.join(&rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("创建截止快照目录失败：{error}"))?;
            }
            let source_bytes = std::fs::read(&from)
                .map_err(|error| format!("读取截止快照 {} 失败：{error}", rel))?;
            if to.is_file() {
                let target_bytes = std::fs::read(&to)
                    .map_err(|error| format!("读取基线文件 {} 失败：{error}", rel))?;
                if Sha256::digest(&source_bytes)[..] == Sha256::digest(&target_bytes)[..] {
                    continue;
                }
            }
            std::fs::write(&to, source_bytes)
                .map_err(|error| format!("复制截止快照 {} 失败：{error}", rel))?;
            copied.push(rel.replace('\\', "/"));
        }
        Ok(copied)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactSnapshotEntry {
    path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ArtifactSnapshotManifest {
    schema_version: u32,
    run_key: String,
    files: Vec<ArtifactSnapshotEntry>,
}

fn sanitize_artifact_snapshot_path(raw: &str) -> Result<String, String> {
    if raw.len() > ARTIFACT_SNAPSHOT_MAX_PATH_BYTES {
        return Err("登记产物路径超过快照上限".to_string());
    }
    let rel = sanitize_rel_path(raw)?;
    // Windows trims trailing dots/spaces from path components. Reject those forms
    // when they would turn into a traversal segment on the snapshot destination.
    if rel
        .split('/')
        .any(|segment| segment.contains('.') && segment.chars().all(|ch| ch == '.' || ch == ' '))
    {
        return Err("登记产物路径包含 Windows 规范化后的越界段".to_string());
    }
    Ok(rel)
}

/// Preserve only explicitly registered deliverables from a successful sandbox.
/// The snapshot is published with a same-volume rename and refuses path escapes,
/// symlink escapes, oversized files, and destination collisions.
fn preserve_artifact_snapshot(
    sandbox: &Path,
    out_dir: &Path,
    key: &MatrixKey,
    artifact_refs: &[String],
) -> Result<Option<String>, String> {
    if artifact_refs.is_empty() {
        return Ok(None);
    }
    if artifact_refs.len() > ARTIFACT_SNAPSHOT_MAX_FILES {
        return Err(format!(
            "登记产物数量 {} 超出快照上限 {}",
            artifact_refs.len(),
            ARTIFACT_SNAPSHOT_MAX_FILES
        ));
    }

    let sandbox_root =
        std::fs::canonicalize(sandbox).map_err(|error| format!("解析运行沙盒失败：{error}"))?;
    let out_root =
        std::fs::canonicalize(out_dir).map_err(|error| format!("解析评测输出目录失败：{error}"))?;
    let artifacts_root = out_dir.join("artifacts");
    match std::fs::symlink_metadata(&artifacts_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("产物快照根目录不是普通目录".to_string());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&artifacts_root)
                .map_err(|create_error| format!("创建产物快照目录失败：{create_error}"))?;
        }
        Err(error) => return Err(format!("检查产物快照目录失败：{error}")),
    }
    let artifacts_root = std::fs::canonicalize(&artifacts_root)
        .map_err(|error| format!("解析产物快照目录失败：{error}"))?;
    if !artifacts_root.starts_with(&out_root) {
        return Err("产物快照目录逃逸评测输出根目录".to_string());
    }

    let key_sha = format!("{:x}", Sha256::digest(key.slug().as_bytes()));
    // A unique path means a crash after publishing but before journaling cannot
    // block a retry of this still-incomplete matrix cell.
    let snapshot_id = format!("{key_sha}-{}", uuid::Uuid::new_v4().simple());
    let destination = artifacts_root.join(&snapshot_id);
    let staging = artifacts_root.join(format!(".{snapshot_id}-{}.tmp", std::process::id()));
    std::fs::create_dir(&staging).map_err(|error| format!("创建产物快照暂存目录失败：{error}"))?;

    let result = (|| {
        let mut entries = Vec::with_capacity(artifact_refs.len());
        let mut total_bytes = 0u64;
        let mut seen = std::collections::BTreeSet::new();
        for raw in artifact_refs {
            let rel = sanitize_artifact_snapshot_path(raw)?;
            if !seen.insert(rel.clone()) {
                continue;
            }
            let source = sandbox_root.join(&rel);
            let canonical_source = std::fs::canonicalize(&source)
                .map_err(|error| format!("登记产物 {rel} 不可读取：{error}"))?;
            if !canonical_source.starts_with(&sandbox_root) {
                return Err(format!("登记产物 {rel} 通过链接逃逸运行沙盒"));
            }
            let metadata = std::fs::metadata(&canonical_source)
                .map_err(|error| format!("读取登记产物 {rel} 元数据失败：{error}"))?;
            if !metadata.is_file() {
                return Err(format!("登记产物 {rel} 不是普通文件"));
            }
            let size_bytes = metadata.len();
            if size_bytes > ARTIFACT_SNAPSHOT_MAX_FILE_BYTES {
                return Err(format!("登记产物 {rel} 超出单文件快照上限"));
            }
            total_bytes = total_bytes
                .checked_add(size_bytes)
                .ok_or_else(|| "产物快照总字节数溢出".to_string())?;
            if total_bytes > ARTIFACT_SNAPSHOT_MAX_TOTAL_BYTES {
                return Err("登记产物超出单次运行快照总字节上限".to_string());
            }
            let file = std::fs::File::open(&canonical_source)
                .map_err(|error| format!("打开登记产物 {rel} 失败：{error}"))?;
            let mut bytes = Vec::with_capacity(size_bytes as usize);
            file.take(ARTIFACT_SNAPSHOT_MAX_FILE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| format!("读取登记产物 {rel} 失败：{error}"))?;
            if bytes.len() as u64 > ARTIFACT_SNAPSHOT_MAX_FILE_BYTES {
                return Err(format!("登记产物 {rel} 在快照期间超过单文件上限"));
            }
            if bytes.len() as u64 != size_bytes {
                return Err(format!("登记产物 {rel} 在快照期间发生变化"));
            }
            let target_rel = sanitize_artifact_snapshot_path(&rel)?;
            let target = staging.join(&target_rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("创建快照子目录失败：{error}"))?;
            }
            std::fs::write(&target, &bytes)
                .map_err(|error| format!("写入产物快照 {rel} 失败：{error}"))?;
            entries.push(ArtifactSnapshotEntry {
                path: rel,
                size_bytes,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
            });
        }
        let manifest = ArtifactSnapshotManifest {
            schema_version: 1,
            run_key: key.slug(),
            files: entries,
        };
        let manifest_text = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("序列化产物快照清单失败：{error}"))?;
        std::fs::write(staging.join("manifest.json"), manifest_text)
            .map_err(|error| format!("写入产物快照清单失败：{error}"))?;
        std::fs::rename(&staging, &destination)
            .map_err(|error| format!("发布产物快照失败：{error}"))?;
        Ok(format!("artifacts/{snapshot_id}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result.map(Some)
}

/// The matrix cell and attempt that owns external execution evidence.
#[derive(Debug, Clone, Copy)]
pub struct MatrixAttempt<'a> {
    pub key: &'a MatrixKey,
    pub execution: &'a str,
    pub batch_label: Option<&'a str>,
    /// Zero for the initial execution, increasing for each repair.
    pub repair_index: u32,
    pub repair_limit: u32,
}

/// 执行器可见的单次运行上下文。写入统一走 [`ExecContext::write_file`]，
/// 强制 allow_write 范围（权限默认 deny），并自动登记 Artifact 引用与失败步骤。
pub struct ExecContext<'ctx> {
    pub case: &'ctx ProductEvalCase,
    pub sandbox: &'ctx Path,
    pub mode: AgentMode,
    pub timeout_secs: u64,
    pub max_model_calls: u32,
    /// 外部取消令牌（Ctrl-C 等）；执行器应在合适时机检查并尽快返回 aborted。
    pub cancel: Arc<AtomicBool>,
    progress: Arc<ExecProgress>,
    artifact_refs: Vec<String>,
    failed_steps: Vec<String>,
    /// 验证/修复组的检查器失败反馈（第二次及以后的尝试由 Runner 注入）。
    repair_feedback: Option<String>,
    matrix_attempt: Option<MatrixAttempt<'ctx>>,
    /// True only when the runner has an explicitly registered external-driver
    /// registry. Built-in executors then defer real desktop/fault execution to
    /// the runner; without a registry they remain default-deny.
    external_drivers_registered: bool,
}

impl<'ctx> ExecContext<'ctx> {
    pub(crate) fn new(
        case: &'ctx ProductEvalCase,
        sandbox: &'ctx Path,
        mode: AgentMode,
        timeout_secs: u64,
        max_model_calls: u32,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            case,
            sandbox,
            mode,
            timeout_secs,
            max_model_calls,
            cancel,
            progress: Arc::new(ExecProgress::default()),
            artifact_refs: Vec::new(),
            failed_steps: Vec::new(),
            repair_feedback: None,
            matrix_attempt: None,
            external_drivers_registered: false,
        }
    }

    pub(crate) fn set_external_drivers_registered(&mut self, registered: bool) {
        self.external_drivers_registered = registered;
    }

    /// Whether a registered external driver owns this gated attempt. Only the
    /// runner may set this; unregistered executors stay default-deny.
    pub fn external_drivers_registered(&self) -> bool {
        self.external_drivers_registered
    }

    /// Write a real driver-produced artifact through the same allow_write gate
    /// and artifact registry as the built-in Agent tools.
    pub fn write_driver_artifact(&mut self, rel: &str, content: &str) -> Result<(), String> {
        self.write_file(rel, content)
    }

    pub fn matrix_attempt(&self) -> Option<MatrixAttempt<'ctx>> {
        self.matrix_attempt
    }

    /// 注入上一轮检查器失败信息；执行器把它追加到用户提示词，形成验证/修复闭环。
    pub fn set_repair_feedback(&mut self, feedback: impl Into<String>) {
        self.repair_feedback = Some(feedback.into());
    }

    /// 当前尝试的修复反馈（首次执行时为 None）。
    pub fn repair_feedback(&self) -> Option<&str> {
        self.repair_feedback.as_deref()
    }

    /// Test-only bridge for a prevalidated checker result; the runner's
    /// external-evidence gate must still reject gated cases.
    #[doc(hidden)]
    pub fn mark_checkers_prevalidated_for_test(&self, passed: u32, total: u32) {
        self.progress.mark_checkers_prevalidated(passed, total);
    }

    #[doc(hidden)]
    pub(crate) fn test_progress(&self) -> Arc<ExecProgress> {
        Arc::clone(&self.progress)
    }

    pub(crate) fn progress(&self) -> Arc<ExecProgress> {
        self.test_progress()
    }

    pub fn record_failed_step(&mut self, step: impl Into<String>) {
        self.failed_steps.push(step.into());
    }

    /// 直接登记一个已落盘的产物引用（执行器内 Agent 经工具写入磁盘后，
    /// 由执行器清点沙盒并调用本方法补录；与 write_file 的登记口径一致）。
    pub(crate) fn record_artifact(&mut self, rel: impl Into<String>) {
        let rel = rel.into();
        if !self.artifact_refs.contains(&rel) {
            self.artifact_refs.push(rel);
        }
    }

    /// 取消命中检测。
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// 沙盒内受控写入：路径合法性 + allow_write 范围强制。
    /// 失败时自动登记失败步骤并返回 Err。
    pub fn write_file(&mut self, raw_path: &str, content: &str) -> Result<(), String> {
        let rel = sanitize_rel_path(raw_path)?;
        if !in_scope(&self.case.allow_write, &rel) {
            let desc = format!("write_denied:{raw_path}（超出 allow_write 范围）");
            self.failed_steps.push(desc.clone());
            return Err(desc);
        }
        let target = self.sandbox.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录 {} 失败：{e}", parent.display()))?;
        }
        std::fs::write(&target, content)
            .map_err(|e| format!("写入 {} 失败：{e}", target.display()))?;
        if !self.artifact_refs.contains(&rel) {
            self.artifact_refs.push(rel);
        }
        Ok(())
    }

    fn take_records(&mut self) -> (Vec<String>, Vec<String>) {
        (
            std::mem::take(&mut self.artifact_refs),
            std::mem::take(&mut self.failed_steps),
        )
    }
}

/// 执行器原始产出；状态由 Runner 与检查器共同合成。
#[derive(Debug, Clone, Default)]
pub struct RawExecOutcome {
    /// 执行器观察到取消令牌提前退出。
    pub aborted: bool,
    /// 执行器层面的错误（None = 执行流程完成，成败交由检查器裁决）。
    pub error: Option<String>,
    pub model_calls: u32,
    /// Exact tool calls; None means the executor could not observe them.
    pub tool_calls: Option<u32>,
    pub usage: TokenUsage,
    /// Provider 是否回报了可用用量（否则 token 字段落盘为 null）。
    pub usage_known: bool,
    /// Host-side validation wall time accumulated by the executor.
    pub validation_wall_ms: u64,
    /// Team DeliveryGate commit wall time; zero for executors without a Team gate.
    pub delivery_gate_wall_ms: u64,
    /// 内部重试次数（如多 Agent 复审返工）。
    pub retries: u32,
    /// 真实工具调用轨迹（单 Agent 执行器填写；其余执行器为空）。
    pub tool_log: Vec<String>,
    /// 必须由已注册驱动在本次 MatrixAttempt 内生成；伪造/过期/无绑定一律失败。
    pub external_evidence: Option<serde_json::Value>,
}

/// The kind of registered external driver required for a formal matrix case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalDriverKind {
    RealDesktop,
    FaultInjection,
}

impl ExternalDriverKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ExternalDriverKind::RealDesktop => "real_desktop",
            ExternalDriverKind::FaultInjection => "fault_injection",
        }
    }

    pub fn required_error(self) -> &'static str {
        match self {
            ExternalDriverKind::RealDesktop => "EVAL_REAL_DESKTOP_DRIVER_REQUIRED",
            ExternalDriverKind::FaultInjection => "EVAL_FAULT_INJECTION_DRIVER_REQUIRED",
        }
    }
}

/// The built-in model executors only expose sandbox tools. These cases need a
/// driver that observes real desktop actions or injects a fault at runtime.
pub(crate) fn external_execution_required(case: &ProductEvalCase) -> Option<&'static str> {
    crate::external_drivers::required_external_driver(case).map(ExternalDriverKind::required_error)
}

/// When a real external driver is registered, that process owns the gated
/// case. The built-in executor returns without consuming model/tool budget; the
/// runner then invokes the driver and validates its bound evidence. Without
/// registration the default deny remains absolute.
pub fn registered_external_driver_outcome(ctx: &ExecContext<'_>) -> Option<RawExecOutcome> {
    if ctx.external_drivers_registered() && external_execution_required(ctx.case).is_some() {
        Some(RawExecOutcome {
            tool_calls: Some(0),
            ..RawExecOutcome::default()
        })
    } else {
        None
    }
}
/// 用例执行器契约。dry 参考回放 / live 网关生成共用此契约，
/// 因此单/多 Agent 天然共享同一份任务、输入、权限、预算与检查器。
#[async_trait]
pub trait CaseExecutor: Send + Sync {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome;
}

// ---------------------------------------------------------------------------
// dry 执行器：参考输出回放（零模型调用，本地确定性可重复）
// ---------------------------------------------------------------------------

/// 参考输出回放执行器：把任务内置的 reference_outputs 写入沙盒后交检查器判定。
/// 若某个预期产物没有参考内容，判执行器错误——这是 harness 自检信号。
pub struct ReferenceDryExecutor;

#[async_trait]
impl CaseExecutor for ReferenceDryExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        let mut outcome = RawExecOutcome {
            tool_calls: Some(0),
            ..RawExecOutcome::default()
        };
        // 回放**全部**参考输出（不只预期产物）：代码任务的"修复后源文件"也在其中，
        // 行为 command_check 才能对参考状态真正执行（dry = 完整 harness 自检）。
        for (path, content) in &ctx.case.reference_outputs {
            if let Err(write_err) = ctx.write_file(path, content) {
                outcome.error = Some(format!("dry 回放写入失败：{write_err}"));
                return outcome;
            }
        }
        // 参考输出必须覆盖全部预期产物（否则检查器无可判定对象）。
        for artifact in &ctx.case.expected_artifacts {
            if !ctx.case.reference_outputs.contains_key(artifact) {
                outcome.error = Some(format!(
                    "dry 回放缺少参考输出：{artifact}（harness 自检失败）"
                ));
                return outcome;
            }
        }
        outcome
    }
}

// ---------------------------------------------------------------------------
// live 执行器：经网关生成的文件块协议
// ---------------------------------------------------------------------------

const OUTPUT_FORMAT_CONTRACT: &str = "=== FILE: 相对路径 ===\n<文件内容，可多行>\n=== END FILE ===";

const SYSTEM_FILE_WRITER: &str = "你是产品评测中的执行代理。你的全部回复只能是若干个文件块，格式严格如下（不要 markdown 围栏、不要解释文字）：\n=== FILE: 相对路径 ===\n<文件内容>\n=== END FILE ===\n可以连续输出多个文件块。只允许写出任务声明的预期产物路径。";

const SYSTEM_PLANNER: &str =
    "你是多 Agent 流水线中的规划者。针对给定任务输出一份不超过 10 条要点的执行计划，不要输出文件内容本身。";

const SYSTEM_REVIEWER: &str = "你是多 Agent 流水线中的评审者。审阅执行者给出的文件草稿：若需返工，回复以 REVISE: 开头并附具体修改意见；若无问题，只回复 APPROVED。不要输出其他内容。";

/// 从 Provider 文本回复中解析 FILE 块；容忍 markdown 围栏包裹。
/// 返回 (blocks, warnings)；warning 描述被丢弃的残块。
pub fn parse_file_blocks(text: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut blocks = Vec::new();
    let mut warnings = Vec::new();
    let mut current_path: Option<String> = None;
    let mut buffer: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("===") {
            // 完整围栏形如 `=== FILE: 路径 ===` / `=== END FILE ===`：去掉行尾的 "===" 再比对。
            let inner = rest.trim();
            let inner = inner.strip_suffix("===").map(str::trim).unwrap_or(inner);
            if let Some(path_part) = inner.strip_prefix("FILE:") {
                if let Some(open) = &current_path {
                    warnings.push(format!(
                        "嵌套 FILE 开始标记（丢弃残块 {}）",
                        truncate_for_log(open, 40)
                    ));
                }
                current_path = Some(path_part.trim().trim_matches('`').trim().to_string());
                buffer.clear();
                continue;
            }
            if inner == "END FILE" {
                if let Some(path) = current_path.take() {
                    blocks.push((path, buffer.join("\n")));
                    buffer.clear();
                } else {
                    warnings.push("孤立的 END FILE 标记".to_string());
                }
                continue;
            }
        }
        if current_path.is_some() {
            buffer.push(line.to_string());
        }
    }
    if let Some(path) = current_path {
        warnings.push(format!(
            "未闭合的文件块（{}），内容已丢弃",
            truncate_for_log(&path, 40)
        ));
    }
    (blocks, warnings)
}

fn cap_text(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push_str("\n…（截断）");
    }
    out
}

fn messages_for_generation(
    case: &ProductEvalCase,
    extra_role_context: Option<&str>,
) -> Vec<ChatMessage> {
    let mut user = String::new();
    user.push_str("# 任务\n");
    user.push_str(&format!("[{}] {}\n", case.category.as_str(), case.title));
    user.push_str(case.instruction.trim());
    user.push('\n');
    user.push_str("\n# 必须产出的 Artifact（相对路径）\n");
    for artifact in &case.expected_artifacts {
        user.push_str(&format!("- {artifact}\n"));
    }
    if !case.inputs.is_empty() {
        user.push_str("\n# 输入文件（已就绪，仅供阅读）\n");
        for input in &case.inputs {
            user.push_str(&format!(
                "--- {} ---\n{}\n--- 结束 {} ---\n",
                input.path,
                cap_text(&input.content, PROMPT_TEXT_CAP),
                input.path
            ));
        }
    }
    if let Some(context) = extra_role_context {
        user.push_str("\n# 协作者上下文\n");
        user.push_str(context);
        user.push('\n');
    }
    user.push_str(&format!("\n# 输出格式\n{OUTPUT_FORMAT_CONTRACT}\n"));
    vec![
        ChatMessage::system(SYSTEM_FILE_WRITER.to_string()),
        ChatMessage::user(user),
    ]
}

async fn chat_text(
    provider: &dyn ModelProvider,
    model: &str,
    messages: &[ChatMessage],
) -> Result<String, String> {
    match provider.complete(messages, &[]).await? {
        ModelOutput::Text(text) => {
            if text.trim().is_empty() {
                Err(format!("模型 {model} 返回空文本"))
            } else {
                Ok(text)
            }
        }
        ModelOutput::ToolCalls(_) => Err("评测生成器不接受工具调用回复".to_string()),
    }
}

/// 单次 live 运行的内部状态。
struct LiveState {
    model_calls: u32,
    session_usage: TokenUsage,
    usage_before: TokenUsage,
    plan: String,
    draft: BTreeMap<String, String>,
}

/// live 生成式执行器：经网关产出文件块 → 受控落盘 → 检查器判定。
///
/// - single：一次调用直出全部文件块；
/// - multi：规划者 → 执行者 → 评审者（必要时一轮返工）流水线；
/// - 预算：超过 max_model_calls 即报错终止；
/// - 取消：每次模型调用之间检查取消令牌。
pub struct GenerativeExecutor {
    pub provider: Arc<dyn ModelProvider>,
    pub model: String,
}

impl GenerativeExecutor {
    async fn call_counted(
        &self,
        state: &mut LiveState,
        budget: u32,
        messages: &[ChatMessage],
    ) -> Result<String, String> {
        if state.model_calls >= budget {
            return Err(format!("模型调用预算耗尽：已达 max_model_calls={budget}"));
        }
        state.model_calls += 1;
        let output = chat_text(self.provider.as_ref(), &self.model, messages).await?;
        let snapshot = self.provider.usage_snapshot();
        let delta = TokenUsage {
            prompt_tokens: snapshot
                .prompt_tokens
                .saturating_sub(state.usage_before.prompt_tokens),
            completion_tokens: snapshot
                .completion_tokens
                .saturating_sub(state.usage_before.completion_tokens),
            total_tokens: snapshot
                .total_tokens
                .saturating_sub(state.usage_before.total_tokens),
        };
        state.session_usage.add(&delta);
        Ok(output)
    }

    async fn generate_files(
        &self,
        ctx: &mut ExecContext<'_>,
        state: &mut LiveState,
        messages: Vec<ChatMessage>,
    ) -> Result<(), String> {
        let text = self
            .call_counted(state, ctx.max_model_calls, &messages)
            .await?;
        let (blocks, warnings) = parse_file_blocks(&text);
        for warning in warnings {
            ctx.record_failed_step(format!("parse_warning:{warning}"));
        }
        if blocks.is_empty() {
            return Err("模型回复不含任何 FILE 块".to_string());
        }
        state.draft.clear();
        for (path, content) in blocks {
            state.draft.insert(path, content);
        }
        Ok(())
    }

    fn flush_draft_to_sandbox(
        &self,
        ctx: &mut ExecContext<'_>,
        draft: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let mut first_error = None;
        for (path, content) in draft {
            if let Err(write_err) = ctx.write_file(path, content) {
                first_error.get_or_insert(write_err);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn draft_for_prompt(draft: &BTreeMap<String, String>) -> String {
        let mut text = String::new();
        for (path, content) in draft {
            text.push_str(&format!(
                "--- {} ---\n{}\n",
                path,
                cap_text(content, PROMPT_TEXT_CAP)
            ));
        }
        text
    }

    async fn ensure_not_cancelled(ctx: &ExecContext<'_>) -> Result<(), String> {
        if ctx.cancelled() {
            Err("__cancelled__".to_string())
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl CaseExecutor for GenerativeExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        if ctx.cancelled() {
            return RawExecOutcome {
                aborted: true,
                tool_calls: Some(0),
                ..RawExecOutcome::default()
            };
        }
        if let Some(outcome) = registered_external_driver_outcome(ctx) {
            return outcome;
        }
        if !ctx.external_drivers_registered() {
            if let Some(error) = external_execution_required(ctx.case) {
                return RawExecOutcome {
                    error: Some(error.to_string()),
                    tool_calls: Some(0),
                    ..RawExecOutcome::default()
                };
            }
        }
        let usage_before = self.provider.usage_snapshot();
        let mut state = LiveState {
            model_calls: 0,
            session_usage: TokenUsage::default(),
            usage_before,
            plan: String::new(),
            draft: BTreeMap::new(),
        };
        let mut retries = 0u32;

        let pipeline_result: Result<(), String> = async {
            if ctx.mode == AgentMode::Multi {
                // ① 规划者
                Self::ensure_not_cancelled(ctx).await?;
                let mut planner_user = String::new();
                planner_user.push_str(&format!(
                    "# 任务\n[{}] {}\n{}\n",
                    ctx.case.category.as_str(),
                    ctx.case.title,
                    ctx.case.instruction.trim()
                ));
                planner_user.push_str("\n# 必须产出的 Artifact\n");
                for artifact in &ctx.case.expected_artifacts {
                    planner_user.push_str(&format!("- {artifact}\n"));
                }
                let plan = self
                    .call_counted(
                        &mut state,
                        ctx.max_model_calls,
                        &[
                            ChatMessage::system(SYSTEM_PLANNER.to_string()),
                            ChatMessage::user(planner_user),
                        ],
                    )
                    .await?;
                state.plan = plan;
            }

            // ② 执行者
            Self::ensure_not_cancelled(ctx).await?;
            let worker_context = if ctx.mode == AgentMode::Multi {
                Some(format!("规划者建议：\n{}", cap_text(&state.plan, PROMPT_TEXT_CAP)))
            } else {
                None
            };
            self.generate_files(ctx, &mut state, messages_for_generation(ctx.case, worker_context.as_deref()))
                .await?;

            // ③ 评审者（仅 multi）
            if ctx.mode == AgentMode::Multi {
                Self::ensure_not_cancelled(ctx).await?;
                let mut reviewer_user = String::new();
                reviewer_user.push_str(&format!(
                    "# 任务\n[{}] {}\n\n# 执行者草稿\n{}",
                    ctx.case.category.as_str(),
                    ctx.case.title,
                    Self::draft_for_prompt(&state.draft)
                ));
                reviewer_user.push_str("\n# 必须产出的 Artifact\n");
                for artifact in &ctx.case.expected_artifacts {
                    reviewer_user.push_str(&format!("- {artifact}\n"));
                }
                let verdict = self
                    .call_counted(
                        &mut state,
                        ctx.max_model_calls,
                        &[
                            ChatMessage::system(SYSTEM_REVIEWER.to_string()),
                            ChatMessage::user(reviewer_user),
                        ],
                    )
                    .await?;
                if verdict.trim().to_uppercase().starts_with("REVISE") {
                    // ④ 返工一轮
                    retries += 1;
                    Self::ensure_not_cancelled(ctx).await?;
                    let revise_context = format!(
                        "规划者建议：\n{}\n\n你的上一版草稿：\n{}\n\n评审意见：\n{}\n\n请输出修订后的完整文件块（全部产物，不是补丁）。",
                        cap_text(&state.plan, PROMPT_TEXT_CAP),
                        Self::draft_for_prompt(&state.draft),
                        cap_text(&verdict, PROMPT_TEXT_CAP)
                    );
                    self.generate_files(ctx, &mut state, messages_for_generation(ctx.case, Some(&revise_context)))
                        .await?;
                }
            }

            // ⑤ 受控落盘（写入范围强制在 write_file 内）
            let draft = state.draft.clone();
            self.flush_draft_to_sandbox(ctx, &draft)
        }
        .await;

        match pipeline_result {
            Ok(()) => RawExecOutcome {
                aborted: false,
                error: None,
                model_calls: state.model_calls,
                tool_calls: Some(0),
                usage: state.session_usage,
                usage_known: state.session_usage.total_tokens > 0,
                validation_wall_ms: 0,
                delivery_gate_wall_ms: 0,
                retries,
                tool_log: Vec::new(),
                external_evidence: None,
            },
            Err(error) if error == "__cancelled__" => RawExecOutcome {
                aborted: true,
                error: None,
                model_calls: state.model_calls,
                tool_calls: Some(0),
                usage: state.session_usage,
                usage_known: state.session_usage.total_tokens > 0,
                validation_wall_ms: 0,
                delivery_gate_wall_ms: 0,
                retries,
                tool_log: Vec::new(),
                external_evidence: None,
            },
            Err(error) => RawExecOutcome {
                aborted: false,
                error: Some(error),
                model_calls: state.model_calls,
                tool_calls: Some(0),
                usage: state.session_usage,
                usage_known: state.session_usage.total_tokens > 0,
                validation_wall_ms: 0,
                delivery_gate_wall_ms: 0,
                retries,
                tool_log: Vec::new(),
                external_evidence: None,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// 矩阵执行器（journal + 续跑 + 报告）
/// 真实单 Agent 执行器（第一路）：经既有 Agent/工具/权限/Session 全链路执行任务。
pub mod single_agent;
// M1：原 core 用 `#[path = "product_eval/workswarm_executor.rs"]` 把它挂在 crate 根
// （模块名 product_eval_workswarm）。迁移后成为 product_eval 的正式子模块。
pub mod workswarm_executor;

pub use single_agent::{
    ProductEvalApprover, ProductEvalScope, SingleAgentExecutor, TelemetrySnapshot,
};

/// 依据环境变量单价估算成本（未配置则 None）。
fn estimate_cost_from_env(usage: &TokenUsage) -> Option<f64> {
    let input = std::env::var("OWO_EVAL_PRICE_IN_PER_MTOK")
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    let output = std::env::var("OWO_EVAL_PRICE_OUT_PER_MTOK")
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    Some(usage.cost_estimate_usd(input, output))
}

/// 运行检查器质量分（0..1）：通过检查器数 / 总检查器数的均值。
/// 无任何带检查器计数的运行（如旧 journal）时返回 None。
pub fn quality_of(runs: &[ProductEvalRun]) -> Option<f64> {
    let mut sum = 0.0;
    let mut counted = 0usize;
    for run in runs {
        if run.checker_total > 0 {
            sum += run.checker_passed as f64 / run.checker_total as f64;
            counted += 1;
        }
    }
    if counted == 0 {
        None
    } else {
        Some(sum / counted as f64)
    }
}

fn complete_optional_u64_sum(values: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    let mut count = 0usize;
    let mut total = 0u64;
    for value in values {
        total = total.checked_add(value?)?;
        count += 1;
    }
    (count > 0).then_some(total)
}

/// 已知值的合计：允许部分缺失，只累加 Some（溢出时返回 None）。
fn known_optional_u64_sum(values: impl IntoIterator<Item = Option<u64>>) -> Option<u64> {
    let mut count = 0usize;
    let mut total = 0u64;
    for value in values {
        let Some(value) = value else { continue };
        total = total.checked_add(value)?;
        count += 1;
    }
    (count > 0).then_some(total)
}

fn complete_optional_cost_sum(values: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let mut count = 0usize;
    let mut total = 0.0f64;
    for value in values {
        let value = value?;
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        total += value;
        if !total.is_finite() {
            return None;
        }
        count += 1;
    }
    (count > 0).then_some(total)
}

/// 聚合指标：全部已尝试运行一律计入分母。
pub fn aggregate_metrics(runs: &[ProductEvalRun]) -> ProductEvalMetrics {
    let total = runs.len();
    let passed = runs
        .iter()
        .filter(|r| r.status == RunStatus::Passed)
        .count();
    let failed = runs
        .iter()
        .filter(|r| r.status == RunStatus::Failed)
        .count();
    let errors = runs.iter().filter(|r| r.status == RunStatus::Error).count();
    let timeouts = runs
        .iter()
        .filter(|r| r.status == RunStatus::Timeout)
        .count();
    let cancelled = runs
        .iter()
        .filter(|r| r.status == RunStatus::Cancelled)
        .count();
    let wall_sum: u64 = runs.iter().map(|r| r.wall_ms).sum();
    let executor_wall_sum: u64 = runs.iter().map(|r| r.executor_wall_ms).sum();
    let validation_wall_sum: u64 = runs.iter().map(|r| r.validation_wall_ms).sum();
    let delivery_gate_wall_sum: u64 = runs.iter().map(|r| r.delivery_gate_wall_ms).sum();
    let model_calls: u64 = runs.iter().map(|r| r.model_calls as u64).sum();
    let tool_calls_known = total > 0 && runs.iter().all(|r| r.tool_calls.is_some());
    let total_tool_calls = tool_calls_known.then(|| {
        runs.iter()
            .filter_map(|r| r.tool_calls)
            .map(u64::from)
            .sum()
    });
    let total_tokens = complete_optional_u64_sum(runs.iter().map(|run| run.total_tokens));
    let estimated_cost_usd = complete_optional_cost_sum(runs.iter().map(|run| run.cost_usd));
    let usage_known_runs = runs.iter().filter(|run| run.total_tokens.is_some()).count();
    // 只统计真正发起过模型调用的运行：fail-closed（外部驱动缺失）没有调用过模型，
    // 不能算成“usage 缺失”。
    let usage_unknown_runs = runs
        .iter()
        .filter(|run| run.model_calls > 0 && run.total_tokens.is_none())
        .count();
    let known_total_tokens = known_optional_u64_sum(runs.iter().map(|run| run.total_tokens));
    ProductEvalMetrics {
        runs_total: total,
        passed,
        failed,
        errors,
        timeouts,
        cancelled,
        success_rate: if total == 0 {
            0.0
        } else {
            passed as f64 / total as f64
        },
        mean_wall_ms: if total == 0 {
            0.0
        } else {
            wall_sum as f64 / total as f64
        },
        mean_executor_wall_ms: if total == 0 {
            0.0
        } else {
            executor_wall_sum as f64 / total as f64
        },
        mean_validation_wall_ms: if total == 0 {
            0.0
        } else {
            validation_wall_sum as f64 / total as f64
        },
        mean_delivery_gate_wall_ms: if total == 0 {
            0.0
        } else {
            delivery_gate_wall_sum as f64 / total as f64
        },
        total_model_calls: model_calls,
        total_tool_calls,
        total_tokens,
        usage_known_runs,
        usage_unknown_runs,
        known_total_tokens,
        estimated_cost_usd,
    }
}

/// 按 (case_id, mode) 分组统计（按 id+mode 排序，输出稳定）。
pub fn aggregate_per_case(runs: &[ProductEvalRun]) -> Vec<CaseModeMetrics> {
    let mut groups: BTreeMap<(String, AgentMode), Vec<&ProductEvalRun>> = BTreeMap::new();
    for run in runs {
        groups
            .entry((run.key.case_id.clone(), run.key.agent_mode))
            .or_default()
            .push(run);
    }
    groups
        .into_iter()
        .map(|((case_id, agent_mode), group)| {
            let owned: Vec<ProductEvalRun> = group.iter().map(|run| (*run).clone()).collect();
            let metrics = aggregate_metrics(&owned);
            let mean_calls = if group.is_empty() {
                0.0
            } else {
                group.iter().map(|r| r.model_calls as f64).sum::<f64>() / group.len() as f64
            };
            let mean_tool_calls =
                if !group.is_empty() && group.iter().all(|run| run.tool_calls.is_some()) {
                    Some(
                        group
                            .iter()
                            .filter_map(|run| run.tool_calls)
                            .map(f64::from)
                            .sum::<f64>()
                            / group.len() as f64,
                    )
                } else {
                    None
                };
            CaseModeMetrics {
                case_id,
                category: group
                    .first()
                    .map(|r| r.category)
                    .unwrap_or(EvalCategory::Code),
                agent_mode,
                runs_total: metrics.runs_total,
                passed: metrics.passed,
                success_rate: metrics.success_rate,
                mean_wall_ms: metrics.mean_wall_ms,
                mean_model_calls: mean_calls,
                mean_tool_calls,
                total_tokens: metrics.total_tokens,
                quality: quality_of(&owned),
            }
        })
        .collect()
}

/// 从磁盘加载报告（compare 用）。
pub fn load_report(path: &Path) -> Result<ProductEvalReport, ProductEvalError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ProductEvalError(format!("读取 {} 失败：{e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| ProductEvalError(format!("解析报告 {} 失败：{e}", path.display())))
}

fn write_report(out_dir: &Path, report: &ProductEvalReport) -> Result<(), ProductEvalError> {
    let text = serde_json::to_string_pretty(report)
        .map_err(|e| ProductEvalError(format!("序列化报告失败：{e}")))?;
    std::fs::write(out_dir.join("report.json"), text)
        .map_err(|e| ProductEvalError(format!("写入报告失败：{e}")))
}

/// 报告的人类可读摘要（run 结束时输出）。
pub fn format_report_summary(report: &ProductEvalReport) -> String {
    let m = &report.metrics;
    let mut out = String::new();
    out.push_str(&format!(
        "套件「{}」（execution={}，model={}）\n",
        report.suite_name,
        report.execution,
        report.model.as_deref().unwrap_or("-")
    ));
    if let Some(label) = &report.batch_label {
        out.push_str(&format!("批次：{label}\n"));
    }
    if !report.tags.is_empty() {
        out.push_str(&format!("标签：{}\n", report.tags.join("、")));
    }
    out.push_str(&format!(
        "运行 {}/{}（passed={} failed={} error={} timeout={} cancelled={}）成功率 {:.1}%\n",
        m.runs_total,
        m.runs_total + report.pending.len(),
        m.passed,
        m.failed,
        m.errors,
        m.timeouts,
        m.cancelled,
        m.success_rate * 100.0
    ));
    out.push_str(&format!(
        "平均墙钟 {:.0}ms；模型调用 {} 次；工具调用 {:?} 次；tokens {:?}；成本 {:?}\n",
        m.mean_wall_ms,
        m.total_model_calls,
        m.total_tool_calls,
        m.total_tokens,
        m.estimated_cost_usd
    ));
    for run in &report.runs {
        if let Some(snapshot_rel) = &run.artifact_snapshot_rel {
            out.push_str(&format!("  成功产物快照 {}：{}\n", run.key, snapshot_rel));
        }
    }
    if !report.pending.is_empty() {
        out.push_str(&format!(
            "未完成单元格 {} 个（重跑同一命令即可续跑，不重复已完成 case）\n",
            report.pending.len()
        ));
    }
    for row in &report.per_case {
        out.push_str(&format!(
            "  {:<34} {:<7} {:>2}/{}（{:.0}%）mean_wall={:.0}ms mean_calls={:.1} mean_tools={:?} quality={}\n",
            row.case_id,
            row.agent_mode.as_str(),
            row.passed,
            row.runs_total,
            row.success_rate * 100.0,
            row.mean_wall_ms,
            row.mean_model_calls,
            row.mean_tool_calls,
            row.quality
                .map(|q| format!("{:.2}", q))
                .unwrap_or_else(|| "-".to_string())
        ));
    }
    out
}

/// 对照表中一个单元格：A/B 两侧的分组统计（缺侧为 None）。
type CompareRow<'a> = (Option<&'a CaseModeMetrics>, Option<&'a CaseModeMetrics>);

/// 对照两份报告（相同任务集），输出文本表或 JSON；失败运行不参与任何"剔除"。
pub fn compare_reports(a: &ProductEvalReport, b: &ProductEvalReport, as_json: bool) -> String {
    let mut rows: BTreeMap<(String, AgentMode), CompareRow<'_>> = BTreeMap::new();
    for row in &a.per_case {
        rows.insert((row.case_id.clone(), row.agent_mode), (Some(row), None));
    }
    for row in &b.per_case {
        rows.entry((row.case_id.clone(), row.agent_mode))
            .and_modify(|(_, slot_b)| *slot_b = Some(row))
            .or_insert_with(|| (None, Some(row)));
    }
    if as_json {
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|((case_id, mode), (ra, rb))| {
                serde_json::json!({
                    "case_id": case_id,
                    "agent_mode": mode.as_str(),
                    "a": ra,
                    "b": rb,
                    "delta_success_rate": match (ra, rb) {
                        (Some(ra), Some(rb)) => Some(rb.success_rate - ra.success_rate),
                        _ => None,
                    },
                })
            })
            .collect();
        return serde_json::to_string_pretty(&serde_json::json!({
            "a": a,
            "b": b,
            "rows": json_rows,
        }))
        .unwrap_or_else(|_| "{}".to_string());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "A：{}（{}，model={}，generated={}）\n",
        a.suite_name,
        a.execution,
        a.model.as_deref().unwrap_or("-"),
        a.generated_at
    ));
    out.push_str(&format!(
        "B：{}（{}，model={}，generated={}）\n\n",
        b.suite_name,
        b.execution,
        b.model.as_deref().unwrap_or("-"),
        b.generated_at
    ));
    out.push_str(&format!(
        "总体成功率：A {:.1}%（{}/{}）→ B {:.1}%（{}/{}）\n",
        a.metrics.success_rate * 100.0,
        a.metrics.passed,
        a.metrics.runs_total,
        b.metrics.success_rate * 100.0,
        b.metrics.passed,
        b.metrics.runs_total
    ));
    out.push_str(&format!(
        "平均墙钟：A {:.0}ms → B {:.0}ms；tokens：A {:?} → B {:?}\n\n",
        a.metrics.mean_wall_ms,
        b.metrics.mean_wall_ms,
        a.metrics.total_tokens,
        b.metrics.total_tokens
    ));
    out.push_str(&format!(
        "{:<32} {:<7} {:>12} {:>12} {:>8}\n",
        "case", "mode", "A", "B", "Δrate"
    ));
    let mut regressions = 0usize;
    let mut improvements = 0usize;
    for ((case_id, mode), (ra, rb)) in &rows {
        let cell_a = ra
            .map(|r| {
                format!(
                    "{}/{}（{:.0}%）",
                    r.passed,
                    r.runs_total,
                    r.success_rate * 100.0
                )
            })
            .unwrap_or_else(|| "-".to_string());
        let cell_b = rb
            .map(|r| {
                format!(
                    "{}/{}（{:.0}%）",
                    r.passed,
                    r.runs_total,
                    r.success_rate * 100.0
                )
            })
            .unwrap_or_else(|| "-".to_string());
        let delta = match (ra, rb) {
            (Some(ra), Some(rb)) => {
                let d = rb.success_rate - ra.success_rate;
                if d < -1e-9 {
                    regressions += 1;
                    format!("{d:+.2}↓")
                } else if d > 1e-9 {
                    improvements += 1;
                    format!("{d:+.2}↑")
                } else {
                    format!("{d:+.2}")
                }
            }
            _ => "-".to_string(),
        };
        out.push_str(&format!(
            "{:<32} {:<7} {:>12} {:>12} {:>8}\n",
            truncate_for_log(case_id, 32),
            mode.as_str(),
            cell_a,
            cell_b,
            delta
        ));
    }
    out.push_str(&format!(
        "\n回归 {regressions} 组，提升 {improvements} 组（Δ 仅作对照，不重算任何历史成功率）\n"
    ));
    if a.execution != b.execution {
        out.push_str("⚠️ 两份报告 execution 不同（如 dry vs live），成功率不具备直接可比性\n");
    }
    if a.suite_hash != b.suite_hash {
        out.push_str("⚠️ 两份报告 suite_hash 不同：任务定义已变更，对照仅具参考意义\n");
    }
    out
}

/// 构建 live 生成式执行器所需的 Provider（GLM/任意 OpenAI 兼容端点）。
/// 模型解析顺序：显式覆盖 > OPENAI_MODEL > 内置默认（GLM）。
pub fn build_live_provider(
    model_override: Option<&str>,
) -> Result<(Arc<dyn ModelProvider>, String, String), ProductEvalError> {
    let mut config = OpenAiCompatibleConfig::from_env().map_err(ProductEvalError)?;
    if let Some(model) = model_override {
        config.model = model.to_string();
    }
    let model = config.model.clone();
    let endpoint_sha256 = format!("{:x}", Sha256::digest(config.base_url.as_bytes()));
    let provider = owo_agent_core::gateway::ResilientProvider::from_config(config)
        .map_err(ProductEvalError)?;
    Ok((Arc::new(provider), model, endpoint_sha256))
}

// ---------------------------------------------------------------------------
// R1 live 基线统计：Wilson 置信区间 / p50-p95 分位数 / 单多对照与启用条件
// ---------------------------------------------------------------------------

/// 95% 置信区间的 z 值（正态近似）。
pub const CI95_Z: f64 = 1.96;

/// 成功率的 Wilson score 置信区间（小样本稳健；z 传 [`CI95_Z`] 即 95%）。
/// total = 0（无样本）时返回 (0.0, 0.0)。
pub fn wilson_interval(successes: usize, total: usize, z: f64) -> (f64, f64) {
    if total == 0 {
        return (0.0, 0.0);
    }
    let n = total as f64;
    let p = successes as f64 / n;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = (p + z2 / (2.0 * n)) / denom;
    let half = (z / denom) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    (
        (center - half).clamp(0.0, 1.0),
        (center + half).clamp(0.0, 1.0),
    )
}

/// 线性插值分位数（p ∈ [0,100]；空集返回 None；单元素直接返回该值）。
pub fn percentile(values: &[u64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.iter().map(|value| *value as f64).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if sorted.len() == 1 {
        return Some(sorted[0]);
    }
    let rank = (p.clamp(0.0, 100.0) / 100.0) * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(sorted[lo] * (1.0 - frac) + sorted[hi.min(sorted.len() - 1)] * frac)
}

/// 判定样本量是否足以支撑统计结论（经验阈值：n ≥ 30）。
pub const SUFFICIENT_SAMPLE_SIZE: usize = 30;

/// 单一拓扑（single/multi）的统计快照（分母一律含失败/错误/超时，不剔除）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeStats {
    pub mode: String,
    pub runs_total: usize,
    pub passed: usize,
    pub success_rate: f64,
    pub ci95_low: f64,
    pub ci95_high: f64,
    pub p50_wall_ms: Option<f64>,
    pub p95_wall_ms: Option<f64>,
    pub mean_wall_ms: f64,
    #[serde(default)]
    pub mean_executor_wall_ms: f64,
    #[serde(default)]
    pub mean_validation_wall_ms: f64,
    #[serde(default)]
    pub mean_delivery_gate_wall_ms: f64,
    pub mean_model_calls: f64,
    #[serde(default)]
    pub mean_tool_calls: Option<f64>,
    pub total_tokens: Option<u64>,
    pub total_cost_usd: Option<f64>,
    /// 检查器通过率均值（质量代理；无检查器计数的旧记录缺省为 None）。
    #[serde(default)]
    pub quality: Option<f64>,
    /// 样本量是否达到可判定阈值（n ≥ 30）。
    pub sample_sufficient: bool,
}

/// 计算一个拓扑的统计快照（无样本时 success_rate=0、区间 (0,0)、分位数 None）。
pub fn mode_statistics(runs: &[ProductEvalRun], mode: AgentMode) -> ModeStats {
    let group: Vec<&ProductEvalRun> = runs
        .iter()
        .filter(|run| run.key.agent_mode == mode)
        .collect();
    let total = group.len();
    let passed = group
        .iter()
        .filter(|run| run.status == RunStatus::Passed)
        .count();
    let (low, high) = wilson_interval(passed, total, CI95_Z);
    let walls: Vec<u64> = group.iter().map(|run| run.wall_ms).collect();
    let mean_wall = if total == 0 {
        0.0
    } else {
        walls.iter().sum::<u64>() as f64 / total as f64
    };
    let mean_calls = if total == 0 {
        0.0
    } else {
        group.iter().map(|run| run.model_calls as f64).sum::<f64>() / total as f64
    };
    let mean_tool_calls = if total > 0 && group.iter().all(|run| run.tool_calls.is_some()) {
        Some(
            group
                .iter()
                .filter_map(|run| run.tool_calls)
                .map(f64::from)
                .sum::<f64>()
                / total as f64,
        )
    } else {
        None
    };
    let mean_executor_wall = if total == 0 {
        0.0
    } else {
        group
            .iter()
            .map(|run| run.executor_wall_ms as f64)
            .sum::<f64>()
            / total as f64
    };
    let mean_validation_wall = if total == 0 {
        0.0
    } else {
        group
            .iter()
            .map(|run| run.validation_wall_ms as f64)
            .sum::<f64>()
            / total as f64
    };
    let mean_delivery_gate_wall = if total == 0 {
        0.0
    } else {
        group
            .iter()
            .map(|run| run.delivery_gate_wall_ms as f64)
            .sum::<f64>()
            / total as f64
    };
    let tokens = complete_optional_u64_sum(group.iter().map(|run| run.total_tokens));
    let cost = complete_optional_cost_sum(group.iter().map(|run| run.cost_usd));
    let owned: Vec<ProductEvalRun> = group.iter().map(|run| (*run).clone()).collect();
    let quality = quality_of(&owned);
    ModeStats {
        mode: mode.as_str().to_string(),
        runs_total: total,
        passed,
        success_rate: if total == 0 {
            0.0
        } else {
            passed as f64 / total as f64
        },
        ci95_low: low,
        ci95_high: high,
        p50_wall_ms: percentile(&walls, 50.0),
        p95_wall_ms: percentile(&walls, 95.0),
        mean_wall_ms: mean_wall,
        mean_executor_wall_ms: mean_executor_wall,
        mean_validation_wall_ms: mean_validation_wall,
        mean_delivery_gate_wall_ms: mean_delivery_gate_wall,
        mean_model_calls: mean_calls,
        mean_tool_calls,
        total_tokens: tokens,
        total_cost_usd: cost,
        quality,
        sample_sufficient: total >= SUFFICIENT_SAMPLE_SIZE,
    }
}

/// 一条主收益候选或质量/资源护栏判定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnablementRule {
    pub name: String,
    pub satisfied: bool,
    pub detail: String,
}

/// 单/多 Agent 对照差异与启用条件判定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeComparison {
    pub multi_success_rate_diff: f64,
    pub multi_wall_rel_change: Option<f64>,
    pub multi_calls_rel_change: Option<f64>,
    #[serde(default)]
    pub multi_tool_calls_rel_change: Option<f64>,
    pub multi_tokens_rel_change: Option<f64>,
    pub multi_cost_rel_change: Option<f64>,
    pub rules: Vec<EnablementRule>,
    #[serde(default)]
    pub resource_guardrails: Vec<EnablementRule>,
    #[serde(default)]
    pub quality_guardrails: Vec<EnablementRule>,
    #[serde(default)]
    pub alignment_guardrails: Vec<EnablementRule>,
    /// 资源护栏、质量、样本量与至少一项收益同时满足才启用.
    pub enabled: bool,
    /// 样本量是否足以支撑对照结论（两组都 ≥ 30 才算充分）。
    pub sample_sufficient: bool,
}

fn rel_change(baseline: f64, candidate: f64) -> Option<f64> {
    if baseline == 0.0 {
        None
    } else {
        Some((candidate - baseline) / baseline)
    }
}

/// 对照判定（multi 相对 single）。通过条件分两层：交付成功率不低于 Single，
/// Single/Team 每任务 token、模型调用、工具调用都完整可测且未突破共享上限；在此
/// 基础上，成功率、检查器质量、耗时三种主收益至少命中一项。成本有双侧价格时也受限。
pub fn compare_mode_statistics(single: &ModeStats, multi: &ModeStats) -> ModeComparison {
    let diff = multi.success_rate - single.success_rate;
    let wall_rel = rel_change(single.mean_wall_ms, multi.mean_wall_ms);
    let calls_rel = rel_change(single.mean_model_calls, multi.mean_model_calls);
    let tool_calls_rel = match (single.mean_tool_calls, multi.mean_tool_calls) {
        (Some(a), Some(b)) => rel_change(a, b),
        _ => None,
    };
    let tokens_rel = match (single.total_tokens, multi.total_tokens) {
        (Some(a), Some(b)) => rel_change(a as f64, b as f64),
        _ => None,
    };
    let cost_rel = match (single.total_cost_usd, multi.total_cost_usd) {
        (Some(a), Some(b)) => rel_change(a, b),
        _ => None,
    };
    let rate_rel = rel_change(single.success_rate, multi.success_rate);
    let per_run = |total: Option<u64>, runs: usize| {
        if runs == 0 {
            None
        } else {
            total.map(|value| value as f64 / runs as f64)
        }
    };
    let per_run_cost = |total: Option<f64>, runs: usize| {
        if runs == 0 {
            None
        } else {
            total.map(|value| value / runs as f64)
        }
    };
    let policy_thresholds = owo_agent_workswarm::team_benefit::BenefitThresholds::default();
    let resource_checks = owo_agent_workswarm::team_benefit::compare_resource_envelopes(
        &owo_agent_workswarm::team_benefit::ResourceEnvelope {
            mean_model_calls: Some(single.mean_model_calls),
            mean_tool_calls: single.mean_tool_calls,
            mean_tokens: per_run(single.total_tokens, single.runs_total),
            mean_cost_usd: per_run_cost(single.total_cost_usd, single.runs_total),
        },
        &owo_agent_workswarm::team_benefit::ResourceEnvelope {
            mean_model_calls: Some(multi.mean_model_calls),
            mean_tool_calls: multi.mean_tool_calls,
            mean_tokens: per_run(multi.total_tokens, multi.runs_total),
            mean_cost_usd: per_run_cost(multi.total_cost_usd, multi.runs_total),
        },
        &policy_thresholds,
    )
    .into_iter()
    .map(|rule| EnablementRule {
        name: rule.name,
        satisfied: rule.satisfied,
        detail: rule.detail,
    })
    .collect::<Vec<_>>();
    let mut rules = vec![EnablementRule {
        name: "成功率 +5%".to_string(),
        satisfied: diff >= 0.05,
        detail: format!("multi−single = {:+.1}pp（门槛 ≥ +5.0pp）", diff * 100.0),
    }];
    let (quality_gain, quality_detail) = match (single.quality, multi.quality) {
        (Some(base), Some(candidate)) if base > 0.0 => {
            let relative = candidate / base - 1.0;
            (
                relative >= 0.10,
                format!(
                    "检查器通过率相对变化 {:+.1}%（门槛 ≥ +10%）",
                    relative * 100.0
                ),
            )
        }
        (Some(0.0), Some(_)) => (
            false,
            "Single 检查器质量基线为 0，无法计算相对质量收益".to_string(),
        ),
        (None, None) => (
            rate_rel.is_some_and(|value| value >= 0.10),
            format!(
                "双方无独立检查器质量值，以成功率作代理：相对变化 {:+.1}%（门槛 ≥ +10%）",
                rate_rel.map(|value| value * 100.0).unwrap_or(f64::NAN)
            ),
        ),
        _ => (
            false,
            "Single/Team 独立检查器质量覆盖不完整，质量收益不可比".to_string(),
        ),
    };
    rules.push(EnablementRule {
        name: "质量 +10%".to_string(),
        satisfied: quality_gain,
        detail: quality_detail,
    });
    let quality_guardrails = match (single.quality, multi.quality) {
        (Some(base), Some(candidate)) => vec![EnablementRule {
            name: "检查器质量不退化".to_string(),
            satisfied: base.is_finite() && candidate.is_finite() && candidate >= base,
            detail: format!("Single={base:.3}，Team={candidate:.3}"),
        }],
        (None, None) => vec![EnablementRule {
            name: "检查器质量不退化".to_string(),
            satisfied: true,
            detail: "双方均无独立检查器质量；交付成功率仍需不退化".to_string(),
        }],
        _ => vec![EnablementRule {
            name: "检查器质量不退化".to_string(),
            satisfied: false,
            detail: "Single/Team 独立检查器质量覆盖不对称，默认不自动放行".to_string(),
        }],
    };
    rules.push(EnablementRule {
        name: "耗时 -30%".to_string(),
        satisfied: wall_rel.is_some_and(|value| value <= -0.30),
        detail: format!(
            "平均墙钟相对变化 {:+.1}%（门槛 ≤ −30.0%）",
            wall_rel.map(|value| value * 100.0).unwrap_or(f64::NAN)
        ),
    });
    ModeComparison {
        multi_success_rate_diff: diff,
        multi_wall_rel_change: wall_rel,
        multi_calls_rel_change: calls_rel,
        multi_tool_calls_rel_change: tool_calls_rel,
        multi_tokens_rel_change: tokens_rel,
        multi_cost_rel_change: cost_rel,
        enabled: single.sample_sufficient
            && multi.sample_sufficient
            && diff >= 0.0
            && quality_guardrails.iter().all(|rule| rule.satisfied)
            && resource_checks.iter().all(|rule| rule.satisfied)
            && rules.iter().any(|rule| rule.satisfied),
        rules,
        quality_guardrails,
        resource_guardrails: resource_checks,
        alignment_guardrails: Vec::new(),
        sample_sufficient: single.sample_sufficient && multi.sample_sufficient,
    }
}

/// 报告级对照判定：除统计收益外，必须完整配对每个任务/重复轮次，且没有待执行矩阵项。
pub fn compare_report_statistics(runs: &[ProductEvalRun], pending: &[MatrixKey]) -> ModeComparison {
    let mut comparison = compare_mode_statistics(
        &mode_statistics(runs, AgentMode::Single),
        &mode_statistics(runs, AgentMode::Multi),
    );
    let alignment = report_pairing_guardrail(runs, pending);
    comparison.enabled &= alignment.satisfied;
    comparison.alignment_guardrails = vec![alignment];
    comparison
}

/// 保证每个 (case_id, repetition) 恰好有一条 Single 与一条 Team 运行记录。
pub fn report_pairing_guardrail(runs: &[ProductEvalRun], pending: &[MatrixKey]) -> EnablementRule {
    use std::collections::BTreeMap;
    let mut cells: BTreeMap<(String, u32), (usize, usize)> = BTreeMap::new();
    for run in runs {
        let counts = cells
            .entry((run.key.case_id.clone(), run.key.repetition))
            .or_default();
        match run.key.agent_mode {
            AgentMode::Single => counts.0 += 1,
            AgentMode::Multi => counts.1 += 1,
        }
    }
    let malformed = cells
        .values()
        .any(|(single, team)| *single != 1 || *team != 1);
    let satisfied = !runs.is_empty() && pending.is_empty() && !malformed;
    let detail = if pending.is_empty() && !malformed && !runs.is_empty() {
        format!(
            "{} 个任务/重复单元均有且仅有 Single 与 Team 记录",
            cells.len()
        )
    } else if !pending.is_empty() {
        format!(
            "仍有 {} 个待执行矩阵单元，不能作自动启用判定",
            pending.len()
        )
    } else if runs.is_empty() {
        "没有可配对的运行记录".to_string()
    } else {
        "任务或重复轮次缺少一侧，或存在重复记录".to_string()
    };
    EnablementRule {
        name: "任务配对完整".to_string(),
        satisfied,
        detail,
    }
}

/// 单/多两拓扑的完整统计（报告 JSON 契约形状：供 server/TS 同步）。
pub fn report_statistics(runs: &[ProductEvalRun]) -> serde_json::Value {
    let single = mode_statistics(runs, AgentMode::Single);
    let multi = mode_statistics(runs, AgentMode::Multi);
    let comparison = compare_report_statistics(runs, &[]);
    serde_json::json!({
        "modes": [serde_json::to_value(&single).unwrap_or_default(),
                  serde_json::to_value(&multi).unwrap_or_default()],
        "comparison": serde_json::to_value(&comparison).unwrap_or_default(),
    })
}

/// 人类可读的统计段落（run 结束摘要追加 / live 基线报告引用）。
pub fn format_mode_statistics(runs: &[ProductEvalRun]) -> String {
    let single = mode_statistics(runs, AgentMode::Single);
    let multi = mode_statistics(runs, AgentMode::Multi);
    let mut out = String::from("—— 统计（95% Wilson 置信区间；分母含失败，不剔除）——\n");
    for stats in [&single, &multi] {
        if stats.runs_total == 0 {
            continue;
        }
        out.push_str(&format!(
            "  {:<7} 成功率 {:.1}% CI95 [{:.1}%, {:.1}%]（{}/{}） p50={:.0}ms p95={:.0}ms executor={:.0}ms validate={:.0}ms gate={:.0}ms mean_calls={:.1} tools={:?} tokens={:?} cost={:?}{}\n",
            stats.mode,
            stats.success_rate * 100.0,
            stats.ci95_low * 100.0,
            stats.ci95_high * 100.0,
            stats.passed,
            stats.runs_total,
            stats.p50_wall_ms.unwrap_or(0.0),
            stats.p95_wall_ms.unwrap_or(0.0),
            stats.mean_executor_wall_ms,
            stats.mean_validation_wall_ms,
            stats.mean_delivery_gate_wall_ms,
            stats.mean_model_calls,
            stats.mean_tool_calls,
            stats.total_tokens,
            stats.total_cost_usd,
            if stats.sample_sufficient { "" } else { "（样本不足 n<30，区间仅供参考）" },
        ));
    }
    if single.runs_total > 0 || multi.runs_total > 0 {
        out.push_str("  说明：Team 的 validate/gate 计入 executor；Single 的外层检查在 executor 之后计时，这些阶段值不可相加替代总 wall_ms。\n");
    }
    if single.runs_total > 0 && multi.runs_total > 0 {
        let comparison = compare_report_statistics(runs, &[]);
        out.push_str(&format_mode_comparison(&comparison));
    }
    out
}

/// 启用条件判定的人类可读段落。
pub fn format_mode_comparison(comparison: &ModeComparison) -> String {
    let mut out = String::from("  —— 多 Agent 启用条件（样本、质量与资源护栏为硬门槛）——\n");
    for rule in &comparison.alignment_guardrails {
        let mark = if rule.satisfied { "✅" } else { "⬜" };
        out.push_str(&format!(
            "  {} 配对护栏 {}：{}\n",
            mark, rule.name, rule.detail
        ));
    }
    for rule in &comparison.quality_guardrails {
        let mark = if rule.satisfied { "✅" } else { "⬜" };
        out.push_str(&format!(
            "  {} 质量护栏 {}：{}\n",
            mark, rule.name, rule.detail
        ));
    }
    for rule in &comparison.resource_guardrails {
        let mark = if rule.satisfied { "✅" } else { "⬜" };
        out.push_str(&format!(
            "  {} 资源护栏 {}：{}\n",
            mark, rule.name, rule.detail
        ));
    }
    for rule in &comparison.rules {
        let mark = if rule.satisfied { "✅" } else { "⬜" };
        out.push_str(&format!("  {} {}：{}\n", mark, rule.name, rule.detail));
    }
    if let Some(tool_delta) = comparison.multi_tool_calls_rel_change {
        out.push_str(&format!(
            "  工具调用相对变化：{:+.1}%（Team 相对 Single）\n",
            tool_delta * 100.0
        ));
    }
    if comparison.multi_success_rate_diff < 0.0 {
        out.push_str(&format!(
            "  质量守卫：未通过，multi 成功率低于 single {:.1}pp；速度收益不抵消质量退化\n",
            comparison.multi_success_rate_diff.abs() * 100.0
        ));
    } else if comparison
        .quality_guardrails
        .iter()
        .any(|rule| !rule.satisfied)
    {
        out.push_str(
            "  质量守卫：未通过，检查器质量证据退化或覆盖不完整；速度收益不抵消质量退化\n",
        );
    } else {
        out.push_str("  质量守卫：通过，multi 成功率不低于 single\n");
    }
    if comparison.enabled {
        out.push_str("  结论：建议启用多 Agent（样本充分、质量守卫通过且至少一条优势条件满足）\n");
    } else if comparison
        .alignment_guardrails
        .iter()
        .any(|rule| !rule.satisfied)
    {
        out.push_str("  结论：暂不建议启用多 Agent（任务未完整配对或仍有待执行单元）\n");
    } else if !comparison.sample_sufficient {
        out.push_str("  结论：暂不建议启用多 Agent（single 与 multi 均需至少 30 个样本）\n");
    } else if comparison.multi_success_rate_diff < 0.0 {
        out.push_str("  结论：暂不建议启用多 Agent（质量守卫未通过）\n");
    } else if comparison
        .resource_guardrails
        .iter()
        .any(|rule| !rule.satisfied)
    {
        out.push_str("  结论：暂不建议启用多 Agent（关键用量缺失或资源增幅超过上限）\n");
    } else {
        out.push_str("  结论：暂不建议启用多 Agent（未达到任何优势条件）\n");
    }
    if !comparison.sample_sufficient {
        out.push_str("  ⚠️ 样本不足（n<30）：以上对照结论仅具方向性参考\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn successful_artifact_snapshot_is_bounded_and_hash_manifest_is_durable() {
        let root = std::env::temp_dir().join(format!(
            "product-eval-artifact-snapshot-{}",
            uuid::Uuid::new_v4()
        ));
        let sandbox = root.join("sandbox");
        let out = root.join("out");
        std::fs::create_dir_all(sandbox.join("out")).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let content = b"deliverable body";
        std::fs::write(sandbox.join("out/report.md"), content).unwrap();

        let key = MatrixKey::new("snapshot-case", AgentMode::Single, 0);
        let snapshot_rel =
            preserve_artifact_snapshot(&sandbox, &out, &key, &["out/report.md".to_string()])
                .unwrap()
                .unwrap();
        let snapshot = out.join(&snapshot_rel);
        assert_eq!(
            std::fs::read(snapshot.join("out/report.md")).unwrap(),
            content
        );
        let manifest: ArtifactSnapshotManifest =
            serde_json::from_slice(&std::fs::read(snapshot.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest.run_key, key.slug());
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].path, "out/report.md");
        assert_eq!(manifest.files[0].size_bytes, content.len() as u64);
        assert_eq!(
            manifest.files[0].sha256,
            format!("{:x}", Sha256::digest(content))
        );

        let too_many = vec!["out/report.md".to_string(); ARTIFACT_SNAPSHOT_MAX_FILES + 1];
        assert!(preserve_artifact_snapshot(&sandbox, &out, &key, &too_many).is_err());
        let large = std::fs::File::create(sandbox.join("out/large.bin")).unwrap();
        large.set_len(ARTIFACT_SNAPSHOT_MAX_FILE_BYTES + 1).unwrap();
        drop(large);
        let large_key = MatrixKey::new("large-snapshot-case", AgentMode::Single, 0);
        assert!(preserve_artifact_snapshot(
            &sandbox,
            &out,
            &large_key,
            &["out/large.bin".to_string()]
        )
        .is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_artifact_snapshot_rejects_missing_and_escaping_paths() {
        let root = std::env::temp_dir().join(format!(
            "product-eval-artifact-path-{}",
            uuid::Uuid::new_v4()
        ));
        let sandbox = root.join("sandbox");
        let out = root.join("out");
        std::fs::create_dir_all(&sandbox).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        let key = MatrixKey::new("snapshot-path-case", AgentMode::Single, 0);
        assert!(preserve_artifact_snapshot(&sandbox, &out, &key, &["missing.md".into()]).is_err());
        assert!(
            preserve_artifact_snapshot(&sandbox, &out, &key, &["../outside.md".into()]).is_err()
        );
        assert!(
            preserve_artifact_snapshot(&sandbox, &out, &key, &[".. /outside.md".into()]).is_err()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn execution_progress_accumulates_validation_and_delivery_gate_time() {
        let progress = ExecProgress::default();
        progress.record_validation_wall_ms(12);
        progress.record_validation_wall_ms(8);
        progress.record_delivery_gate_wall_ms(5);

        let snapshot = progress.snapshot();
        assert_eq!(snapshot.validation_wall_ms, 20);
        assert_eq!(snapshot.delivery_gate_wall_ms, 5);
    }

    /// 契约变更任务的映射片段检查器必须容忍格式化差异（反引号/空格/箭头变体）。
    /// 回归依据：2026-08-31 正式批次 code-contract-change 3 个失败均为"语义正确、
    /// 仅字面短语被反引号/ASCII 箭头打断"（rep 2/6/7 的产物内容原样收录）。
    #[test]
    fn contract_change_mapping_checker_tolerates_formatting() {
        let task_text = include_str!("../../../../evals/v1/tasks/code-contract-change.json");
        let task: serde_json::Value =
            serde_json::from_str(task_text).expect("任务 JSON 必须可解析");
        let mapping = task["checkers"]
            .as_array()
            .expect("checkers 数组")
            .iter()
            .find(|c| {
                c.get("path")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|p| p == "out/contract-change.md")
                    && c.get("pattern")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|p| p.contains("fetch_user_display_name"))
            })
            .expect("必须存在映射 Regex 检查器");
        assert_eq!(
            mapping["type"].as_str(),
            Some("regex"),
            "检查器应升级为 Regex（Contains 已被证伪为脆弱字面匹配）"
        );
        let checker: ArtifactChecker =
            serde_json::from_value(mapping.clone()).expect("checker 可反序列化");

        // 2026-08-31 正式批次真实失败产物片段（语义正确，仅格式不同）+ 参考回放格式。
        let positives = [
            "## 变更概述\n\n- 公开函数 `get_user_name` 更名为 `fetch_user_display_name`，并为其 `user_id` 参数增加显式 `str` 类型标注。",
            "## 迁移说明\n\n1. 所有调用点执行机械替换：`get_user_name` → `fetch_user_display_name`。",
            "映射关系：get_user_name → fetch_user_display_name；旧名保留一个版本的 DeprecationWarning 后移除。",
            "公开函数 get_user_name 更名为 fetch_user_display_name。",
        ];
        for (i, content) in positives.iter().enumerate() {
            let mut map = BTreeMap::new();
            map.insert("out/contract-change.md".to_string(), content.to_string());
            assert!(
                evaluate_checker_on_map(&checker, &map).is_ok(),
                "正向样例 {i} 应通过：{content:?}"
            );
        }

        // 反例：只并列提及新旧名、无映射关系语义 → 必须失败。
        let mut negative = BTreeMap::new();
        negative.insert(
            "out/contract-change.md".to_string(),
            "文档同时提及 get_user_name 与 fetch_user_display_name，但没有任何映射表述。"
                .to_string(),
        );
        assert!(
            evaluate_checker_on_map(&checker, &negative).is_err(),
            "反例（无映射语义）必须失败"
        );
    }
}

#[cfg(test)]
mod paired_case_cluster_bootstrap_tests {
    use super::paired_report::paired_case_cluster_bootstrap_ci;

    #[test]
    fn bootstrap_is_reproducible_and_requires_three_independent_cases() {
        let insufficient = paired_case_cluster_bootstrap_ci(&[0.2, 0.4], true, true);
        assert_eq!(insufficient["available"], false);
        assert_eq!(
            insufficient["reason"],
            "requires_at_least_3_independent_cases"
        );

        let values = [-0.1, 0.2, 0.4, 0.0];
        let first = paired_case_cluster_bootstrap_ci(&values, true, true);
        let second = paired_case_cluster_bootstrap_ci(&values, true, true);
        assert_eq!(first, second);
        assert_eq!(first["available"], true);
        assert_eq!(first["resamples"], 5000);
        assert!(first["lower"].as_f64().unwrap() <= first["upper"].as_f64().unwrap());
    }

    #[test]
    fn configuration_mismatch_never_emits_an_interval() {
        let result = paired_case_cluster_bootstrap_ci(&[0.1, 0.2, 0.3], true, false);
        assert_eq!(result["available"], false);
        assert_eq!(result["reason"], "configuration_mismatch");
    }

    #[test]
    fn incomplete_pairing_never_emits_an_interval() {
        let result = paired_case_cluster_bootstrap_ci(&[0.1, 0.2, 0.3], false, true);
        assert_eq!(result["available"], false);
        assert_eq!(result["reason"], "incomplete_or_duplicate_pairing");
    }
}

#[cfg(test)]
mod telemetry_completeness_tests {
    use super::*;

    fn run(case: &str, mode: AgentMode, repetition: u32) -> ProductEvalRun {
        ProductEvalRun {
            key: MatrixKey::new(case, mode, repetition),
            category: EvalCategory::Code,
            status: RunStatus::Passed,
            wall_ms: 100,
            executor_wall_ms: 90,
            validation_wall_ms: 10,
            delivery_gate_wall_ms: 0,
            model_calls: 1,
            tool_calls: Some(1),
            prompt_tokens: Some(0),
            completion_tokens: Some(0),
            total_tokens: Some(0),
            cost_usd: Some(0.0),
            failed_steps: Vec::new(),
            retries: 0,
            cancellations: 0,
            artifact_refs: Vec::new(),
            tool_log: Vec::new(),
            checker_passed: 1,
            checker_total: 1,
            sandbox_rel: None,
            artifact_snapshot_rel: None,
            model: Some("test-model".to_string()),
            started_at: "2026-10-05T00:00:00Z".to_string(),
            finished_at: "2026-10-05T00:00:01Z".to_string(),
            error: None,
            external_evidence: None,
        }
    }

    #[test]
    fn aggregate_token_and_cost_totals_require_complete_usage_coverage() {
        let mut known = run("case", AgentMode::Single, 0);
        known.total_tokens = Some(120);
        known.cost_usd = Some(0.12);
        let mut unknown = run("case", AgentMode::Single, 1);
        unknown.total_tokens = None;
        unknown.cost_usd = None;
        let runs = vec![known.clone(), unknown.clone()];

        let aggregate = aggregate_metrics(&runs);
        assert_eq!(aggregate.total_tokens, None);
        assert_eq!(aggregate.estimated_cost_usd, None);
        assert_eq!(aggregate.usage_known_runs, 1);
        assert_eq!(aggregate.usage_unknown_runs, 1);
        assert_eq!(aggregate.known_total_tokens, Some(120));
        assert_eq!(aggregate_per_case(&runs)[0].total_tokens, None);
        let mode = mode_statistics(&runs, AgentMode::Single);
        assert_eq!(mode.total_tokens, None);
        assert_eq!(mode.total_cost_usd, None);

        let complete = vec![known, run("case", AgentMode::Single, 1)];
        assert_eq!(aggregate_metrics(&complete).total_tokens, Some(120));
        assert_eq!(aggregate_metrics(&complete).estimated_cost_usd, Some(0.12));
    }

    #[test]
    fn paired_snapshots_and_deltas_withhold_partial_token_and_cost_summaries() {
        let mut single_known = run("case-a", AgentMode::Single, 0);
        single_known.total_tokens = Some(100);
        single_known.cost_usd = Some(0.1);
        let mut team_known = run("case-a", AgentMode::Multi, 0);
        team_known.total_tokens = Some(150);
        team_known.cost_usd = Some(0.15);
        let mut single_missing = run("case-b", AgentMode::Single, 0);
        single_missing.total_tokens = None;
        single_missing.cost_usd = None;
        let mut team_missing = run("case-b", AgentMode::Multi, 0);
        team_missing.total_tokens = None;
        team_missing.cost_usd = None;

        let single_refs = [&single_known, &single_missing];
        let team_refs = [&team_known, &team_missing];
        let snapshot = paired_snapshot_json(AgentMode::Single, &single_refs);
        assert_eq!(snapshot["total_tokens"], serde_json::Value::Null);
        assert_eq!(snapshot["total_cost_usd"], serde_json::Value::Null);

        let paired = paired_cell_deltas_json(&single_refs, &team_refs, true, true);
        assert_eq!(paired["token_usage_coverage"]["known_pairs"], 1);
        assert_eq!(paired["token_usage_coverage"]["paired_cells"], 2);
        assert_eq!(paired["token_usage_coverage"]["complete"], false);
        assert_eq!(paired["cost_coverage"]["complete"], false);
        assert_eq!(
            paired["team_minus_single_total_tokens"]["mean"],
            serde_json::Value::Null
        );
        assert_eq!(
            paired["team_minus_single_cost_usd"]["mean"],
            serde_json::Value::Null
        );
    }
}

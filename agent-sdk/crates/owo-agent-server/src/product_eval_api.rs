//! ProductEval HTTP API（V1 三日 · 第四路）。
//!
//! 冻结契约见 `AGENTS-COORD.md` 留言区「第四路（三期开工）」。要点：
//! - 四路由（全部挂 bearer 保护面）：`POST /product-eval/runs`（202 异步受理）、
//!   `GET /product-eval/runs`（列表）、`GET /product-eval/runs/{id}`（详情+报告）、
//!   `POST /product-eval/runs/{id}/cancel`（幂等）。
//! - `ProductEvalHub` 管理后台矩阵任务（core `MatrixRunner` 顺序执行 + journal 断点续跑）、
//!   取消令牌（`Arc<AtomicBool>`，单元格边界生效）与持久化（`hub.json` + core `report.json`）。
//! - suite 仅允许注册名（`v1` → `workspace/evals/v1/suite.json`）；客户端路径一律 400。
//! - 运行目录固定 `data_root/product_eval/runs/{run_id}`。
//! - 重启语义：启动扫描把 queued/running 标记为 `interrupted`（不自动重跑、不调模型）。
//! - `reference` 执行免模型（ReferenceDryExecutor 回放 + 检查器）；`live` 走注入的执行器
//!   工厂（第一/二路执行器接线；工厂失败 → 运行 `failed`，不调模型、不伪造结果）。
//! - 请求校验：结构错误（缺字段/类型错）→ 422；语义错误（未知 suite/execution/mode、
//!   repetitions 越界）→ 400；成功受理 → 202 `{run_id, status:"queued"}`。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_eval_facade::product_eval::{
    self, CaseExecutor, ExecContext, MatrixRunner, RawExecOutcome, ReferenceDryExecutor, RunOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 注册的 suite 名（仅此名单可被 API 触发；客户端本地路径一律拒绝）。
const REGISTERED_SUITES: &[&str] = &["v1"];

/// 运行状态（冻结六态）。
pub const ST_QUEUED: &str = "queued";
pub const ST_RUNNING: &str = "running";
pub const ST_CANCELLED: &str = "cancelled";
pub const ST_COMPLETED: &str = "completed";
pub const ST_FAILED: &str = "failed";
pub const ST_INTERRUPTED: &str = "interrupted";

/// live 执行器工厂：由 lib.rs 接线（reference 模式不经过此工厂）。
/// 工厂失败（如执行器未接线/Provider 不可用）→ 运行进入 `failed`，不调模型。
pub type LiveExecutorFactory = Arc<dyn Fn() -> Result<Arc<dyn CaseExecutor>, String> + Send + Sync>;

/// hub 级运行元数据（持久化为运行目录内 `hub.json`）。
/// 与 core 的 `report.json` 解耦：hub.json 表达受理参数与运行态，报告按需读取。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRunRecord {
    pub run_id: String,
    pub suite: String,
    /// reference | live（原样回显请求字面量）。
    pub execution: String,
    /// 请求字面量：single | workswarm（核心报告 wire 里为 single/multi，workswarm ≡ multi）。
    pub modes: Vec<String>,
    pub repetitions: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// queued | running | cancelled | completed | failed | interrupted
    pub status: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// 计划单元格总数（modes × cases × repetitions）。
    pub planned_total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl EvalRunRecord {
    fn terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            ST_CANCELLED | ST_COMPLETED | ST_FAILED | ST_INTERRUPTED
        )
    }
}

struct RunHandle {
    record: Mutex<EvalRunRecord>,
    cancel: Arc<AtomicBool>,
}

/// 已解析的创建参数（handlers 校验后传入）。
struct CreateParams {
    suite: String,
    execution: String,
    modes: Vec<owo_agent_eval_facade::product_eval::AgentMode>,
    mode_literals: Vec<String>,
    repetitions: u32,
    category: Option<owo_agent_eval_facade::product_eval::EvalCategory>,
    only: Option<String>,
}

/// ProductEval 评测运行中心。
pub struct ProductEvalHub {
    runs_root: PathBuf,
    /// suite 注册根目录（workspace/evals；运行时拼 `/{suite}/suite.json`）。
    suite_root: PathBuf,
    live_factory: LiveExecutorFactory,
    /// A single active matrix keeps paired Single/Team measurements free from cross-run load.
    run_slot: Arc<Semaphore>,
    handles: Mutex<HashMap<String, Arc<RunHandle>>>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl ProductEvalHub {
    pub fn new(runs_root: PathBuf, suite_root: PathBuf, live_factory: LiveExecutorFactory) -> Self {
        let hub = Self {
            runs_root,
            suite_root,
            live_factory,
            run_slot: Arc::new(Semaphore::new(1)),
            handles: Mutex::new(HashMap::new()),
        };
        hub.recover_interrupted();
        hub
    }

    /// 服务重启扫描：queued/running → interrupted（不自动重跑、不调模型）；
    /// completed/cancelled/failed/interrupted 原样保留（报告继续可查）。
    fn recover_interrupted(&self) {
        let Ok(entries) = std::fs::read_dir(&self.runs_root) else {
            return;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let meta_path = dir.join("hub.json");
            let Ok(text) = std::fs::read_to_string(&meta_path) else {
                continue;
            };
            let Ok(mut record) = serde_json::from_str::<EvalRunRecord>(&text) else {
                continue;
            };
            if record.status == ST_QUEUED || record.status == ST_RUNNING {
                record.status = ST_INTERRUPTED.to_string();
                record.finished_at = Some(now_rfc3339());
                record.error =
                    Some("服务重启：运行被中断（不自动重跑；报告含已完成单元格）".to_string());
                let _ = std::fs::write(
                    &meta_path,
                    serde_json::to_string_pretty(&record).unwrap_or_default(),
                );
                tracing::warn!(run_id = %record.run_id, "product-eval 运行标记为 interrupted");
            }
        }
    }

    fn suite_path(&self, suite: &str) -> PathBuf {
        self.suite_root.join(suite).join("suite.json")
    }

    /// 受理一次评测运行（校验后的入口）：
    /// 先同步加载 suite 与过滤（错误 → 400），再落盘 hub.json（queued）并后台执行矩阵。
    fn create(&self, params: CreateParams) -> Result<EvalRunRecord, (StatusCode, String)> {
        let run_permit = Arc::clone(&self.run_slot)
            .try_acquire_owned()
            .map_err(|_| {
                (
                    StatusCode::CONFLICT,
                    "已有评测矩阵正在执行或收尾；请等待结束后再启动，避免两组测量互相干扰"
                        .to_string(),
                )
            })?;
        let bundle = product_eval::load_suite(&self.suite_path(&params.suite))
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("suite 加载失败：{}", e.0)))?;
        let opts = RunOptions {
            modes: params.modes.clone(),
            reps_override: Some(params.repetitions),
            only: params.only.clone(),
            category: params.category,
            fresh: false,
            batch_label: None,
            provider_endpoint_sha256: None,
            tags: Vec::new(),
        };
        let cases = product_eval::filter_cases(&bundle, &opts);
        if cases.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "过滤条件下没有可执行的任务（检查 category/only）".to_string(),
            ));
        }
        let planned_total: usize = cases
            .iter()
            .map(|case| {
                case.effective_repetitions(&bundle.suite.defaults, Some(params.repetitions))
                    as usize
                    * params.modes.len()
            })
            .sum();

        let run_id = format!("eval-{}", uuid::Uuid::new_v4().simple());
        let dir = self.runs_root.join(&run_id);
        std::fs::create_dir_all(&dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建运行目录失败：{e}"),
            )
        })?;
        let record = EvalRunRecord {
            run_id: run_id.clone(),
            suite: params.suite.clone(),
            execution: params.execution.clone(),
            modes: params.mode_literals.clone(),
            repetitions: params.repetitions,
            category: params.category.as_ref().map(|c| c.as_str().to_string()),
            only: params.only.clone(),
            model: std::env::var("OWO_AGENT_MODEL")
                .ok()
                .filter(|m| !m.trim().is_empty()),
            status: ST_QUEUED.to_string(),
            created_at: now_rfc3339(),
            started_at: None,
            finished_at: None,
            planned_total,
            error: None,
        };
        std::fs::write(
            dir.join("hub.json"),
            serde_json::to_string_pretty(&record).unwrap_or_default(),
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("写入 hub.json 失败：{e}"),
            )
        })?;

        let handle = Arc::new(RunHandle {
            record: Mutex::new(record.clone()),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.handles
            .lock()
            .expect("product-eval handles 锁")
            .insert(run_id, Arc::clone(&handle));

        let suite_path = self.suite_path(&params.suite);
        let live_factory = Arc::clone(&self.live_factory);
        tokio::spawn(run_job(
            suite_path,
            live_factory,
            handle,
            params,
            dir,
            run_permit,
        ));
        Ok(record)
    }

    /// 全量运行列表（磁盘为底、内存句柄覆盖最新态），created_at 倒序。
    fn list(&self) -> Vec<EvalRunRecord> {
        let mut by_id: HashMap<String, EvalRunRecord> = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&self.runs_root) {
            for entry in entries.flatten() {
                let path = entry.path().join("hub.json");
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Ok(record) = serde_json::from_str::<EvalRunRecord>(&text) {
                    by_id.insert(record.run_id.clone(), record);
                }
            }
        }
        for handle in self
            .handles
            .lock()
            .expect("product-eval handles 锁")
            .values()
        {
            let record = handle
                .record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            by_id.insert(record.run_id.clone(), record);
        }
        let mut records: Vec<EvalRunRecord> = by_id.into_values().collect();
        records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        records
    }

    fn load_record(
        &self,
        run_id: &str,
    ) -> Option<(Option<Arc<RunHandle>>, EvalRunRecord, PathBuf)> {
        let dir = self.runs_root.join(run_id);
        if !dir.is_dir() {
            return None;
        }
        let in_memory = self
            .handles
            .lock()
            .expect("product-eval handles 锁")
            .get(run_id)
            .cloned();
        let record = match &in_memory {
            Some(handle) => handle
                .record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            None => {
                let text = std::fs::read_to_string(dir.join("hub.json")).ok()?;
                serde_json::from_str::<EvalRunRecord>(&text).ok()?
            }
        };
        Some((in_memory, record, dir))
    }
}

fn persist_record(dir: &std::path::Path, record: &EvalRunRecord) {
    let _ = std::fs::write(
        dir.join("hub.json"),
        serde_json::to_string_pretty(record).unwrap_or_default(),
    );
}

/// 已完成单元格计数：journal 逐行落盘（追加+flush），行数即进度。
fn journal_done(dir: &std::path::Path) -> usize {
    std::fs::read_to_string(dir.join("state.jsonl"))
        .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// 后台矩阵执行：状态机 queued → running → completed/cancelled/failed。
/// 取消在单元格边界生效（core runner 检查令牌）；hub 状态在 cancel 请求时立即置 cancelled。
async fn run_job(
    suite_path: PathBuf,
    live_factory: LiveExecutorFactory,
    handle: Arc<RunHandle>,
    params: CreateParams,
    dir: PathBuf,
    _run_permit: OwnedSemaphorePermit,
) {
    // queued → running（若已被取消则跳过；最终态不回退）。
    {
        let mut record = handle
            .record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if record.status == ST_QUEUED {
            record.status = ST_RUNNING.to_string();
            record.started_at = Some(now_rfc3339());
            persist_record(&dir, &record);
        }
    }

    // 执行器选择：reference 免模型；live 走工厂（失败 → failed，不调模型）。
    let executor: Result<Arc<dyn CaseExecutor>, String> = match params.execution.as_str() {
        "reference" => Ok(Arc::new(ReferenceDryExecutor) as Arc<dyn CaseExecutor>),
        "live" => (live_factory)(),
        other => Err(format!("未知 execution：{other}")),
    };
    let executor = match executor {
        Ok(executor) => executor,
        Err(message) => {
            let mut record = handle
                .record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !record.terminal() {
                record.status = ST_FAILED.to_string();
                record.finished_at = Some(now_rfc3339());
                record.error = Some(message);
                persist_record(&dir, &record);
            }
            return;
        }
    };

    let bundle = match product_eval::load_suite(&suite_path) {
        Ok(bundle) => bundle,
        Err(e) => {
            let mut record = handle
                .record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !record.terminal() {
                record.status = ST_FAILED.to_string();
                record.finished_at = Some(now_rfc3339());
                record.error = Some(format!("suite 加载失败：{}", e.0));
                persist_record(&dir, &record);
            }
            return;
        }
    };

    let runner = MatrixRunner::new(bundle, &dir);
    let opts = RunOptions {
        modes: params.modes.clone(),
        reps_override: Some(params.repetitions),
        only: params.only.clone(),
        category: params.category,
        fresh: false,
        batch_label: None,
        provider_endpoint_sha256: None,
        tags: Vec::new(),
    };
    let model = handle
        .record
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .model
        .clone();
    let cancel = Arc::clone(&handle.cancel);
    let result = runner
        .run(executor, &params.execution, model, &opts, cancel)
        .await;

    let mut record = handle
        .record
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match result {
        Ok(report) => {
            if record.status == ST_CANCELLED {
                // 取消请求已即时落账：保留 cancelled，报告（含已完成单元格）已在磁盘。
            } else if handle.cancel.load(Ordering::Relaxed) && !report.pending.is_empty() {
                record.status = ST_CANCELLED.to_string();
            } else {
                record.status = ST_COMPLETED.to_string();
            }
            record.finished_at = Some(now_rfc3339());
        }
        Err(e) => {
            if record.status != ST_CANCELLED {
                record.status = ST_FAILED.to_string();
                record.error = Some(e.0);
            }
            record.finished_at = Some(now_rfc3339());
        }
    }
    persist_record(&dir, &record);
}

// ---------------------------------------------------------------------------
// live 模式分派执行器（lib.rs 接线用）：按 ExecContext.mode 路由到对应执行器。
// ---------------------------------------------------------------------------

/// live 模式分派：Single → 单 Agent 执行器（第一路），Multi → WorkSwarm 执行器（第二路）。
/// 未接线的拓扑返回执行器级错误（该单元格记 Error，不 panic、不静默成功）。
pub struct ModeDispatchExecutor {
    pub single: Option<Arc<dyn CaseExecutor>>,
    pub multi: Option<Arc<dyn CaseExecutor>>,
}

impl ModeDispatchExecutor {
    pub fn new(
        single: Option<Arc<dyn CaseExecutor>>,
        multi: Option<Arc<dyn CaseExecutor>>,
    ) -> Self {
        Self { single, multi }
    }
}

#[async_trait::async_trait]
impl CaseExecutor for ModeDispatchExecutor {
    async fn execute<'ctx>(&self, ctx: &mut ExecContext<'ctx>) -> RawExecOutcome {
        use owo_agent_eval_facade::product_eval::AgentMode as M;
        let target = match ctx.mode {
            M::Single => self.single.as_ref(),
            M::Multi => self.multi.as_ref(),
        };
        match target {
            Some(executor) => executor.execute(ctx).await,
            None => RawExecOutcome {
                error: Some(format!(
                    "live 模式缺少 {} 执行器接线（等待第一/二路交付）",
                    ctx.mode.as_str()
                )),
                ..RawExecOutcome::default()
            },
        }
    }
}

// ---------------------------------------------------------------------------
// 请求校验：结构错误 → 422；语义错误 → 400。
// ---------------------------------------------------------------------------

fn parse_request(body: &Value) -> Result<CreateParams, (StatusCode, String)> {
    let obj = body.as_object().ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "请求体必须是 JSON 对象".to_string(),
        )
    })?;

    let suite = obj.get("suite").and_then(Value::as_str).ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "缺少字段 suite（string）".to_string(),
        )
    })?;
    if !REGISTERED_SUITES.contains(&suite) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "未知 suite「{suite}」（仅允许注册名：{}；不接受本地路径）",
                REGISTERED_SUITES.join(", ")
            ),
        ));
    }

    let execution = obj
        .get("execution")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "缺少字段 execution（string：reference|live）".to_string(),
            )
        })?;
    if execution != "reference" && execution != "live" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("未知 execution「{execution}」（可选 reference/live）"),
        ));
    }

    let modes_value = obj.get("modes").ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "缺少字段 modes（array：single/workswarm 子集）".to_string(),
        )
    })?;
    let modes_arr = modes_value.as_array().ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "字段 modes 必须是字符串数组".to_string(),
        )
    })?;
    if modes_arr.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "modes 不能为空（至少一个拓扑）".to_string(),
        ));
    }
    let mut modes = Vec::new();
    let mut literals = Vec::new();
    for m in modes_arr {
        let literal = m.as_str().ok_or_else(|| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "字段 modes 必须是字符串数组".to_string(),
            )
        })?;
        let mode = match literal {
            "single" => owo_agent_eval_facade::product_eval::AgentMode::Single,
            "workswarm" => owo_agent_eval_facade::product_eval::AgentMode::Multi,
            other => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("未知 mode「{other}」（可选 single/workswarm）"),
                ));
            }
        };
        if !literals.iter().any(|l: &String| l == literal) {
            literals.push(literal.to_string());
            modes.push(mode);
        }
    }

    let repetitions_value = obj.get("repetitions").ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "缺少字段 repetitions（integer 1..=20）".to_string(),
        )
    })?;
    let repetitions = repetitions_value.as_u64().ok_or_else(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            "字段 repetitions 必须是正整数".to_string(),
        )
    })?;
    if !(1..=20).contains(&repetitions) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("repetitions 超出范围（1..=20）：{repetitions}"),
        ));
    }

    let category = match obj.get("category") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => match owo_agent_eval_facade::product_eval::EvalCategory::parse(s)
        {
            Ok(c) => Some(c),
            Err(m) => return Err((StatusCode::BAD_REQUEST, m)),
        },
        Some(_) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "字段 category 必须是 string 或 null".to_string(),
            ));
        }
    };

    let only = match obj.get("only") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "only 为空字符串时应传 null".to_string(),
                ));
            }
            Some(trimmed.to_string())
        }
        Some(_) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "字段 only 必须是 string 或 null".to_string(),
            ));
        }
    };

    Ok(CreateParams {
        suite: suite.to_string(),
        execution: execution.to_string(),
        modes,
        mode_literals: literals,
        repetitions: repetitions as u32,
        category,
        only,
    })
}

// ---------------------------------------------------------------------------
// handlers
// ---------------------------------------------------------------------------

async fn create_run(
    State(hub): State<Arc<ProductEvalHub>>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let params = parse_request(&body)
        .map_err(|(code, message)| (code, Json(json!({ "error": message }))))?;
    let record = hub
        .create(params)
        .map_err(|(status, message)| (status, Json(json!({ "error": message }))))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "run_id": record.run_id, "status": record.status })),
    ))
}

fn record_to_summary(record: &EvalRunRecord, dir: &std::path::Path) -> Value {
    let done = journal_done(dir).min(record.planned_total);
    json!({
        "run_id": record.run_id,
        "suite": record.suite,
        "execution": record.execution,
        "modes": record.modes,
        "repetitions": record.repetitions,
        "category": record.category,
        "only": record.only,
        "model": record.model,
        "status": record.status,
        "created_at": record.created_at,
        "started_at": record.started_at,
        "finished_at": record.finished_at,
        "planned_total": record.planned_total,
        "progress": { "done": done, "total": record.planned_total },
        "error": record.error,
    })
}

async fn list_runs(State(hub): State<Arc<ProductEvalHub>>) -> Json<Value> {
    let runs: Vec<Value> = hub
        .list()
        .iter()
        .map(|record| record_to_summary(record, &hub.runs_root.join(&record.run_id)))
        .collect();
    Json(json!({ "runs": runs }))
}

async fn get_run(
    State(hub): State<Arc<ProductEvalHub>>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (_, record, dir) = hub.load_record(&run_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("评测运行 {run_id} 不存在") })),
        )
    })?;
    let mut body = record_to_summary(&record, &dir);
    // 报告：runner 每完成一个单元格即刷新 report.json；中途读取容忍缺席/损坏。
    body["report"] = match product_eval::load_report(&dir.join("report.json")) {
        Ok(report) => serde_json::to_value(&report).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };
    Ok(Json(body))
}

async fn cancel_run(
    State(hub): State<Arc<ProductEvalHub>>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (in_memory, mut record, dir) = hub.load_record(&run_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("评测运行 {run_id} 不存在") })),
        )
    })?;
    if !record.terminal() {
        // 非终态运行必有内存句柄（queued/running 只在本进程存活期内出现）。
        if let Some(handle) = in_memory.as_ref() {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        record.status = ST_CANCELLED.to_string();
        record.finished_at = Some(now_rfc3339());
        if let Some(handle) = in_memory.as_ref() {
            *handle
                .record
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = record.clone();
        }
        persist_record(&dir, &record);
        tracing::info!(run_id = %run_id, "product-eval 运行已请求取消（协作令牌已置位）");
    }
    // 幂等：终态（含已取消）原样返回当前状态，零副作用。
    Ok(Json(
        json!({ "run_id": record.run_id, "status": record.status }),
    ))
}

// ---------------------------------------------------------------------------
// 路由
// ---------------------------------------------------------------------------

pub fn router(state: Arc<crate::AppState>) -> Router {
    router_with_hub(Arc::clone(&state.product_eval))
}

pub fn router_with_hub(hub: Arc<ProductEvalHub>) -> Router {
    Router::new()
        .route("/product-eval/runs", post(create_run).get(list_runs))
        .route("/product-eval/runs/{id}", get(get_run))
        .route("/product-eval/runs/{id}/cancel", post(cancel_run))
        .with_state(hub)
}

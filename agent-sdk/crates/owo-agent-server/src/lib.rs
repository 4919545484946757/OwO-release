#![recursion_limit = "1024"]

//! OwO Agent SDK HTTP 服务（M1 + v0.4）：session/turn/permission/diff/revert/abort + SSE，
//! 以及 v0.4 接口：context.snapshot / perception.subscribe / learn.* / skill.verify /
//! proactive.suggest / whitelist.manage。
//!
//! 第四轮核心模块 HTTP/UI 集成：notes_api / plugin_market_api / workflow_api / goal_api /
//! sse（云端任务进度 SSE 集线器）四个模块路由并入 build_router；cloud_task_submit 的
//! ProgressSink 接 sse::sink(task_id) 使 /cloud/tasks/{id}/events 收到真实进度。
//!
//! 第五轮（R5）：workflow_api/goal_api 扩展（真实执行后端 + 人审 + run SSE + Agent Worker，
//! 子模块 workflow_backend/agent_worker 在各自模块内声明）；plugin_market_api 扩展
//! （远端市场 refresh/install-remote）+ team_api（团队技能包共享）+ market_client；
//! eval_gate（eval 护栏）+ observability_api（/metrics 可观测性）；memory_graph_api
//! （记忆图谱）+ intent_api（统一命令入口）。
//!
//! 第七轮（R7）：本地 API 安全边界（X03）——bearer token 鉴权（auth_token.rs，token 文件
//! 用户级 ACL + 发布桌面 `/auth/token` 进程配对引导）、CORS 显式 origin 白名单（webview + localhost）、
//! 全局/每会话/敏感端点双令牌桶限流（rate_limit.rs，429 + Retry-After + 审计）；SSE
//! 资源型路径（/…/events）因 EventSource 无法携带自定义头而豁免鉴权（只读遥测）。
//!
//! 第八轮（R8）：存储运维（backup.rs：/storage/backup|restore|export|clear，恢复前自动备份、
//! zip-slip 防护、清空二次确认 + 完整性校验）与服务端韧性（shutdown.rs：全局并发 turn 上限、
//! 优雅关闭 POST /server/shutdown + GET /server/status、CLI serve 强杀恢复 pid 文件）。

/// V1 四期（第三路）：Artifact 评审闭环路由（review / history）。
mod activity_api;
pub mod artifact_review_api;
mod assist_api;
mod audit_api;
mod auth_token;
pub mod turn_control;

// §12：flush_audit 移入 audit_api.rs，根部 re-export 保全 CLI 对
// `owo_agent_server::flush_audit` 的既有依赖（外部契约不变）。
pub use audit_api::flush_audit;
pub mod backup;
pub mod capabilities;
mod cloud_api;
mod computer_api;
mod desktop_api;
mod desktop_model_credentials;
mod desktop_world_api;
pub mod discovery;
pub mod error_codes;
mod eval_api;
mod eval_gate;
/// 可靠事件流集线器（§3.1：公开给契约测试发布续传事件用；路由在 build_router 挂载）。
pub mod event_stream;
mod expression_api;
mod fleet_api;
mod fs_api;
mod goal_api;
mod idempotency;
mod intent_api;
mod learn_api;
mod locate_api;
pub mod logging;
mod market_client;
mod mcp_api;
mod memory_api;
mod memory_graph_api;
mod notes_api;
mod observability_api;
mod openapi;
mod ops_api;
mod perception_api;
mod plugin_api;
mod plugin_market_api;
pub mod product_eval_api;
mod project_api;
mod rate_limit;
mod routes;
pub use routes::build_router;
/// R3（§8.1）：安全请求 ledger（六字段白名单，/diagnostics/requests 消费）。
/// `pub` 仅为契约测试可用 `reset_for_test` 取得干净窗口（同 event_stream 先例）。
pub mod request_ledger_api;
mod schemas_api;
mod session_api;
mod session_goal_api;
mod session_hunks_api;
mod settings_api;
pub mod shutdown;
mod skills_api;
mod slo;
#[doc(hidden)]
pub mod sse;
mod subagent_api;
mod symbols_api;
mod team_api;
mod team_template_catalog_api;
mod traces_api;
mod turn_api;
mod usage;
mod user_workflows_api;
mod whitelist_api;
mod workflow_api;
mod workflow_draft_api;
mod workflow_stats_api;
mod workflows_share_api;
mod workspace_trust_api;
mod workswarm_api;

/// R1 测试装配面：集成测试需要用独立 TempDir 数据根构造 DesktopWorld 运行态
/// （进程级单例仅服务生产 build_router；导出构造函数不新增 AppState 字段、不改路由面）。
pub use desktop_world_api::{router_with_hub, DesktopWorldHub};
/// A2 接线面：goal 显式 `fleet_node` 目标复用进程级控制面——绑定任务经真实
/// `/fleet/*` 节点协议被已注册节点领取与回传，不伪造远程执行。
pub use fleet_api::{fleet_hub, router_with_hub as fleet_router_with_hub, FleetHub};

/// 协议约束：新模块（notes_api 等）一律写全限定名 `owo_agent_server::AppState`，
/// 以便测试以 `#[path = "../src/xxx.rs"] mod` 独立编译；此处建立 crate 自别名，
/// 使该路径在库内（含子模块）同样可解析。
extern crate self as owo_agent_server;

use axum::extract::DefaultBodyLimit;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use owo_agent_core::automation::{AutomationAction, AutomationRun, AutomationStore};
use owo_agent_core::learn::{
    ActionType, LearnPipeline, LearnState, ProactiveEngine, RecordedAction, SemanticAnchor,
};
use owo_agent_core::perception::SituationStore;
use owo_agent_core::permissions::{Decision, PermissionRequest};
use owo_agent_core::session::{Session, SessionStore};
use owo_agent_core::whitelist::Whitelist;
use owo_agent_core::Agent;
use owo_agent_protocol::{BuildInfo, HealthResponse};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

/// §5.4 待审批请求：request_id → (oneshot，PermissionRequest 副本)。
/// 副本用于响应时按 scope 生成临时授权（Grant）。
pub type PendingApproval = (tokio::sync::oneshot::Sender<Decision>, PermissionRequest);

/// A8-3（取优合并自远端 engine）：桌宠显隐的期望/实际状态。
#[derive(Debug, Default)]
pub struct PetState {
    /// 工作台开关写入的期望值。
    pub desired: Option<bool>,
    pub desired_at: Option<String>,
    /// 桌面端心跳上报的实际值。
    pub actual: Option<bool>,
    pub actual_at: Option<String>,
    /// 最近一次心跳时刻（判断桌面端是否在线）。
    pub actual_seen: Option<std::time::Instant>,
}

pub struct AppState {
    pub agent: Arc<Agent>,
    pub store: Arc<dyn SessionStore>,
    pub sessions: Arc<Mutex<HashMap<String, Session>>>,
    pub pending_approvals: Arc<Mutex<HashMap<String, PendingApproval>>>,
    pub pending_approval_sessions: Arc<Mutex<HashMap<String, String>>>,
    /// §5.4 授权记忆（server 全局一份；Agent.Policy 注入同一引用）。
    pub grants: Arc<owo_agent_core::grant_store::GrantStore>,
    pub turn_controls: Arc<Mutex<turn_control::TurnControls>>,
    /// 每个会话一个运行锁，避免并发回合覆盖消息、快照和审计状态。
    pub turn_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    pub traces_dir: PathBuf,
    pub perception: Arc<Mutex<SituationStore>>,
    pub whitelist: Arc<Mutex<Whitelist>>,
    pub pipeline: Arc<Mutex<LearnPipeline>>,
    pub proactive: Arc<Mutex<ProactiveEngine>>,
    pub stt: Arc<Mutex<owo_agent_core::LocalStt>>,
    pub automations: Arc<Mutex<AutomationStore>>,
    pub memory: Arc<Mutex<owo_agent_core::MemoryStore>>,
    pub audit_flushed: Arc<Mutex<usize>>,
    pub workspace: PathBuf,
    pub data_root: PathBuf,
    pub elements: Arc<Mutex<owo_agent_core::ElementRegistry>>,
    /// 插件启用状态（进程级热卸载的持久化基础）。
    pub plugin_state: Arc<Mutex<owo_agent_core::plugin::PluginStateStore>>,
    /// 持久场景图（跨请求保持模板命中率/历史命中先验；元素每请求从注册表刷新）。
    pub scene: Arc<Mutex<owo_agent_core::scene::SceneGraph>>,
    /// computer-use 任务注册表（任务级审批 + 熔断，m4d 前奏）。
    pub computer_tasks: Arc<owo_agent_core::ComputerTaskRegistry>,
    /// 云端执行队列（/cloud/* 路由；懒初始化，传输由环境变量决定）。
    /// tokio Mutex：异步 handler 内跨 await 持锁（std MutexGuard 非 Send）。
    pub cloud_queue: Arc<tokio::sync::Mutex<Option<owo_agent_core::cloud_exec::CloudTaskQueue>>>,
    /// 短时提交事务锁：分配顺序 task_id 并持久化记录，不与 transport 执行锁竞争。
    pub cloud_submit_lock: Arc<tokio::sync::Mutex<()>>,
    /// 云端执行期间可绕过队列锁发送取消请求。
    pub cloud_cancel_signals:
        Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>>,
    /// 本地 API bearer token（X03：启动生成/复用 + 用户级 ACL 文件）。
    pub auth_token: Arc<auth_token::AuthToken>,
    /// 全局/每会话/敏感端点 双令牌桶限流（X03）。
    pub rate_limiter: Arc<rate_limit::RateLimiter>,
    /// R8 服务端韧性：全局并发 turn 上限 + 优雅关闭信号（CLI serve 接线退出）。
    pub shutdown_gate: Arc<shutdown::ShutdownGate>,
    /// R13 WorkSwarm S0：团队协同状态（协调器懒初始化；/teams、/projects、/tasks 路由）。
    pub workswarm: Arc<workswarm_api::WorkSwarmState>,
    /// V1 三日：ProductEval 评测中心（后台矩阵任务 + 取消令牌 + 持久化报告；/product-eval/*）。
    pub product_eval: Arc<product_eval_api::ProductEvalHub>,
    /// V1 四期（第三路）：Artifact 评审闭环存储（独立 SQLite 连接，
    /// 与 TeamCoordinator 的连接共存于 `data_root/workswarm/space.db`）。
    pub artifact_review: artifact_review_api::ArtifactReviewState,
    /// A8-2（取优合并自远端 engine）：活跃回合快照（`/activity`）——
    /// session_id → {phase, tool, started_at…}，由 turn handler 事件回调维护。
    pub activities: Arc<Mutex<HashMap<String, serde_json::Value>>>,
    /// A8-3：桌面挂件（桌宠）显隐中转——工作台写期望值，桌面端心跳上报实际值。
    pub pet_state: Arc<Mutex<PetState>>,
    /// ask_user（取优合并自远端 engine）：等待用户回答的提问
    /// （question_id → 一次性应答通道）。
    pub pending_questions: Arc<
        Mutex<
            HashMap<String, tokio::sync::oneshot::Sender<owo_agent_core::question::QuestionAnswer>>,
        >,
    >,
    /// 提问归属会话（应答路由校验用）。
    pub pending_question_sessions: Arc<Mutex<HashMap<String, String>>>,
    /// 工作区信任存储（审查 N1；数据根 `trust.json`，不经工作区文件自授权）。
    pub trust_store: Arc<owo_agent_core::trust::TrustStore>,
    /// 启动时是否因未信任而进入受限模式（跳过 hooks/权限放宽/白名单）。
    pub workspace_trust_gated: bool,
}

impl AppState {
    /// §5.4 授权记忆匹配用的工作区标识（与 Policy::workspace_id 一致）。
    pub fn workspace_id(&self) -> String {
        self.workspace
            .canonicalize()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| self.workspace.to_string_lossy().into_owned())
    }

    pub fn new(
        agent: Agent,
        store: impl SessionStore + 'static,
        traces_dir: PathBuf,
        data_root: PathBuf,
        workspace: PathBuf,
    ) -> Self {
        // V1 四期（第三路）：评审闭环库路径（在 data_root 被 struct 字面量 move 前取好）。
        let workswarm_db_root = data_root.join("workswarm");
        // 审查 N1：工作区信任门。工作区里有 hooks / 工作区插件 / mcp_servers /
        // 放宽权限档位 / 白名单，且没有信任记录时，一律不加载这些"配置即代码"
        // 内容（受限模式）；用户经 GET/POST /workspace/trust 或保存设置页显式确认。
        let trust_store = Arc::new(owo_agent_core::trust::TrustStore::new(data_root.clone()));
        let trust_status = trust_store.status(&workspace);
        let workspace_trust_gated = !trust_status.trusted && trust_status.gated;
        let mut settings = owo_agent_core::Settings::load(&workspace);
        if workspace_trust_gated {
            settings.permission_profile = None;
            settings.permission_spec = None;
            settings.hooks.clear();
            settings.whitelist.clear();
            tracing::warn!(
                "工作区未信任，已跳过工作区配置（hooks/插件 MCP/权限放宽/白名单）：{}",
                trust_status.reasons.join("；")
            );
        }
        settings.apply_usage_env();
        settings.apply_reasoning_env();
        // R8：用量预算接线（Agent 4 交付 usage）——单价/预算从环境变量注入，turn 入口硬熔断。
        {
            let usage_store = usage::global();
            if let Some(price) = std::env::var("OWO_MODEL_INPUT_PRICE_PER_MTOK")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
            {
                let output_price = std::env::var("OWO_MODEL_OUTPUT_PRICE_PER_MTOK")
                    .ok()
                    .and_then(|value| value.parse::<f64>().ok())
                    .unwrap_or(price);
                usage_store.set_price_per_mtok(price.max(output_price));
            }
            if let Some(budget) = std::env::var("OWO_USAGE_COST_BUDGET_USD")
                .ok()
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| *value > 0.0)
            {
                usage_store.set_budget(usage::UsageDimension::Session, budget);
            }
        }
        let mut whitelist = Whitelist::default();
        for entry in settings.whitelist.clone() {
            whitelist.upsert(entry);
        }
        let elements = Arc::new(Mutex::new(owo_agent_core::ElementRegistry::new()));
        // §5.4 授权记忆 —— server 全局一份，Agent 的 Policy 共享引用，
        // 审批卡选项写入的 Grant 在下一请求即命中；profile 从 settings 恢复。
        // §4.5.2「工作区长期」必须跨重启有效，所以长期授权落 `<data_root>/grants.json`
        // （tmp→rename 原子写；坏文件改名 *.json.bad 保留现场）。有期限的授权不写盘。
        let grants = Arc::new(owo_agent_core::grant_store::GrantStore::persisting(
            data_root.join("grants.json"),
        ));
        let mut agent = agent;
        agent.set_elements(elements.clone());
        agent.set_grants(Arc::clone(&grants));
        if let Some(profile) = settings
            .permission_profile
            .as_deref()
            .and_then(owo_agent_core::PermissionProfile::parse)
        {
            agent.set_permission_profile(profile);
        }
        // §4.5.3 结构化配置必须在档位**之后**恢复：`set_spec` 会把档位同步为 spec 的
        // 最近不放宽投影，顺序反过来等于用 settings 里的旧档位盖掉本次的收紧决定。
        if let Some(spec) = settings.permission_spec.clone() {
            agent.policy().set_spec(spec);
        }
        // A2-1：启动即装载 settings.json 的 hooks（设置页保存后热重灌）。
        agent.set_hooks(owo_agent_core::hooks::HookManager::from_configs(
            &settings.hooks,
        ));
        // 审查 N1：受限模式进入审计（谁在何时因何被跳过，之后可查询/追溯）。
        if workspace_trust_gated {
            if let Ok(mut audit) = agent.audit_log().lock() {
                audit.record(
                    "workspace_trust",
                    "gated",
                    None,
                    Some(false),
                    format!(
                        "未信任工作区：跳过 hooks/插件 MCP/权限放宽/白名单（{}）",
                        trust_status.reasons.join("；")
                    ),
                );
            }
        }
        // X03/R3（§8.2）：本地 API bearer token **每次启动换发**并覆盖写盘（+ ACL）；
        // 写盘失败降级为内存 token。旧代际 bearer 因此在新进程上必然 401。
        let auth_token = Arc::new(auth_token::AuthToken::mint_for_boot(&data_root));
        let rate_limiter = Arc::new(rate_limit::RateLimiter::from_env());
        let shutdown_gate = Arc::new(shutdown::ShutdownGate::from_env());
        // V1 三日（第四路）：ProductEval suite 注册根目录（v1 → workspace/evals/v1/suite.json）。
        let product_eval_suite_root = workspace.join("evals");
        Self {
            agent: Arc::new(agent),
            store: Arc::new(store),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
            pending_approval_sessions: Arc::new(Mutex::new(HashMap::new())),
            grants: Arc::clone(&grants),
            turn_controls: Arc::new(Mutex::new(turn_control::TurnControls::default())),
            turn_locks: Arc::new(Mutex::new(HashMap::new())),
            traces_dir,
            perception: Arc::new(Mutex::new(SituationStore::new())),
            whitelist: Arc::new(Mutex::new(whitelist)),
            pipeline: Arc::new(Mutex::new(LearnPipeline::new(
                data_root.join("skills").join("user"),
            ))),
            proactive: Arc::new(Mutex::new(ProactiveEngine::new(settings.proactive.clone()))),
            stt: Arc::new(Mutex::new(owo_agent_core::LocalStt::new(
                &settings.stt,
                &data_root,
            ))),
            automations: Arc::new(Mutex::new(AutomationStore::new(data_root.clone()))),
            memory: Arc::new(Mutex::new(owo_agent_core::MemoryStore::new(
                data_root.join("memory.jsonl"),
            ))),
            audit_flushed: Arc::new(Mutex::new(0)),
            workspace,
            plugin_state: Arc::new(Mutex::new(owo_agent_core::plugin::PluginStateStore::new(
                Some(data_root.join("plugin_state.json")),
            ))),
            scene: Arc::new(Mutex::new(owo_agent_core::scene::SceneGraph::new())),
            computer_tasks: Arc::new(owo_agent_core::ComputerTaskRegistry::new()),
            cloud_queue: Arc::new(tokio::sync::Mutex::new(None)),
            cloud_submit_lock: Arc::new(tokio::sync::Mutex::new(())),
            cloud_cancel_signals: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            // R13 WorkSwarm：数据目录 data_root/workswarm（协调器首次使用懒初始化）。
            workswarm: Arc::new(workswarm_api::WorkSwarmState::new(
                data_root.join("workswarm"),
            )),
            // V1 三日（第四路）：ProductEval 评测中心。运行目录 data_root/product_eval/runs；
            // suite 注册表固定 v1 → workspace/evals/v1/suite.json。
            // live 执行器工厂：Provider 按 core 统一入口构建（OPENAI_API_KEY 等环境变量），
            // single → 第一路 SingleAgentExecutor；multi（workswarm）→ 第二路 WorkSwarmExecutor
            // （等 core `pub mod workswarm_executor` 登记后接入；未接线前 live 运行 failed，不伪造结果）。
            product_eval: Arc::new(product_eval_api::ProductEvalHub::new(
                data_root.join("product_eval").join("runs"),
                product_eval_suite_root,
                {
                    let workswarm_work_root = data_root.join("product_eval").join("workswarm");
                    Arc::new(move || {
                        let (provider, model, _endpoint_sha256) =
                            owo_agent_eval_facade::product_eval::build_live_provider(None)
                                .map_err(|e| {
                                    format!(
                                        "live Provider 不可用（检查 OPENAI_API_KEY 等）：{}",
                                        e.0
                                    )
                                })?;
                        let single: Arc<dyn owo_agent_eval_facade::product_eval::CaseExecutor> =
                            Arc::new(
                                owo_agent_eval_facade::product_eval::SingleAgentExecutor::new(
                                    Arc::clone(&provider),
                                    model.clone(),
                                ),
                            );
                        let multi: Option<
                            Arc<dyn owo_agent_eval_facade::product_eval::CaseExecutor>,
                        > = Some(Arc::new(
                            // 第二路适配器以 crate 根模块名注册（物理文件 product_eval/workswarm_executor.rs，
                            // #[path] 技巧见 core lib.rs——避免与第一路双写 product_eval.rs）。
                            owo_agent_eval_facade::WorkSwarmExecutor::new(
                                Arc::clone(&provider),
                                model.clone(),
                                workswarm_work_root.clone(),
                            ),
                        ));
                        Ok(Arc::new(product_eval_api::ModeDispatchExecutor::new(
                            Some(single),
                            multi,
                        )))
                    })
                },
            )),
            data_root,
            elements,
            auth_token,
            rate_limiter,
            shutdown_gate,
            // V1 四期（第三路）：评审闭环复用 WorkSwarm 的 space.db（独立连接 + busy_timeout）。
            artifact_review: artifact_review_api::ArtifactReviewState::new(
                workswarm_db_root.join("space.db"),
            ),
            // A8-2 / A8-3（取优合并自远端 engine）。
            activities: Arc::new(Mutex::new(HashMap::new())),
            pet_state: Arc::new(Mutex::new(PetState::default())),
            // ask_user（取优合并自远端 engine）。
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            pending_question_sessions: Arc::new(Mutex::new(HashMap::new())),
            trust_store,
            workspace_trust_gated,
        }
    }
}

/// 全局 `Cache-Control: no-store`（含静态资源；SSE 不受影响）。
async fn no_store_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store, no-cache, must-revalidate"),
    );
    response
}

/// N11：`Host` 头校验（DNS rebinding 防护）。
///
/// 服务只监听 127.0.0.1；恶意网页可用「攻击者域名」经 DNS rebinding 把浏览器请求
/// 打到本机 API（拿到 `/auth/token` 后即可控制 Daemon）。这里只放行回环主机名
/// （`localhost` / `127.0.0.1` / `[::1]`，任意端口）与桌面壳的 `tauri.localhost`；
/// 其余返回 421。缺 Host 头（服务端直连测试 / HTTP/1.0）不在本层拦截——
/// 各端点仍有 bearer 鉴权与配对密钥。
async fn host_guard_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some(host) = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
    {
        if !host_is_allowed(host) {
            return axum::response::IntoResponse::into_response((
                axum::http::StatusCode::MISDIRECTED_REQUEST,
                axum::Json(serde_json::json!({
                    "error": "Host 不受信任（仅允许回环地址）",
                    "code": "http/untrusted_host",
                })),
            ));
        }
    }
    next.run(request).await
}

/// Host 白名单判定：去端口（含 IPv6 `[::1]:port`）后整词比较。
fn host_is_allowed(host: &str) -> bool {
    let host = host.trim();
    let hostname = if let Some(stripped) = host.strip_prefix('[') {
        match stripped.split_once(']') {
            Some((name, _)) => name,
            None => return false,
        }
    } else {
        host.split(':').next().unwrap_or(host)
    };
    matches!(
        hostname.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1" | "tauri.localhost"
    )
}

/// R8/R9：trace_id 请求贯穿——从 `X-Trace-Id` 头继承（不合法则生成），回填响应头，
/// 设置全局 trace 上下文（Agent 4 logging：后台任务/SSE/指标可继承），
/// 并落一条结构化访问日志（脱敏不落消息体）。
///
/// R3（§8.1）：同一最外层切面把请求写入进程内安全 ledger（`request_ledger_api`）。
/// 只记录路由模板（axum `MatchedPath`），未匹配路由（fallback 静态资产服务）
/// 不进 ledger；访问日志同口径改用 `route_template` 字段，避免真实资源 id
/// 进入日志面。
async fn trace_id_middleware(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let inherited = request
        .headers()
        .get(logging::TRACE_HEADER)
        .and_then(|value| value.to_str().ok());
    let trace_id = logging::TraceId::from_header(inherited);
    logging::set_current_trace_id(Some(trace_id.as_str()));
    let method = request.method().to_string();
    // 路由模板优先（隐私：参数段恒为 {name}）；fallback 无模板 → None，不记录。
    let route_template = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|matched| matched.as_str().to_string());
    let path = request.uri().path().to_string();
    let source = request_ledger_api::sanitize_source(
        request
            .headers()
            .get(request_ledger_api::CLIENT_HEADER)
            .and_then(|value| value.to_str().ok()),
    );
    let started_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let started = std::time::Instant::now();
    let mut response = next.run(request).await;
    if let Ok(value) = trace_id
        .to_header_value()
        .parse::<axum::http::HeaderValue>()
    {
        response.headers_mut().insert(
            axum::http::HeaderName::from_static(logging::TRACE_HEADER),
            value,
        );
    }
    let duration_ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    if let Some(template) = route_template {
        request_ledger_api::record(
            &state.data_root,
            request_ledger_api::RequestRecord {
                method: method.clone(),
                route_template: template.clone(),
                started_at: started_at.clone(),
                duration_ms,
                status,
                source: source.clone(),
            },
        );
        let _ = path; // 原始 path 只存在于局部，既不落日志也不落 ledger。
        logging::emit(
            logging::Level::Info,
            "http",
            Some(trace_id.as_str()),
            "request",
            &[
                ("method", serde_json::json!(method)),
                ("route_template", serde_json::json!(template)),
                ("status", serde_json::json!(status)),
                ("duration_ms", serde_json::json!(duration_ms)),
                ("source", serde_json::json!(source)),
            ],
        );
    }
    logging::set_current_trace_id(None);
    response
}

/// CORS：发布桌面版只接受 Tauri origin；开发模式额外允许 loopback 调试页。
/// 跨源预检由浏览器强制；服务器侧仍以 bearer token 鉴权为准。
fn cors_layer() -> CorsLayer {
    use axum::http::Method;
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _parts| {
            origin_allowed(origin.as_bytes())
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::ACCEPT,
            axum::http::HeaderName::from_static(auth_token::DESKTOP_PAIRING_HEADER),
            // R3（§8.1）：ledger 来源标签与 trace 贯穿头（跨源预检需显式放行）。
            axum::http::HeaderName::from_static(request_ledger_api::CLIENT_HEADER),
            axum::http::HeaderName::from_static(logging::TRACE_HEADER),
        ])
        .expose_headers([axum::http::HeaderName::from_static("x-owo-turn-id")])
        .max_age(std::time::Duration::from_secs(600))
}

/// `OWO_DESKTOP_RELEASE=1` 由 Tauri 子进程注入，避免发布版继承开发浏览器边界。
fn origin_allowed(origin: &[u8]) -> bool {
    let Ok(origin) = std::str::from_utf8(origin) else {
        return false;
    };
    let mut parts = origin.split("://");
    let Some(scheme) = parts.next() else {
        return false;
    };
    let Some(host_port) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let host = host_port.split(':').next().unwrap_or(host_port);
    let desktop_release = std::env::var("OWO_DESKTOP_RELEASE")
        .map(|value| value == "1")
        .unwrap_or(false);
    if desktop_release {
        return (scheme == "tauri" && host == "localhost")
            || ((scheme == "http" || scheme == "https") && host == "tauri.localhost");
    }
    ((scheme == "http" || scheme == "https") && (host == "localhost" || host == "127.0.0.1"))
        || (scheme == "tauri" && host == "localhost")
        || ((scheme == "http" || scheme == "https") && host == "tauri.localhost")
}

/// 开发环境下的桌面工作台静态目录：`<repo>/agent-sdk/desktop/web`。
/// ADR-003：打包形态下 web 工作台随包携带在 `resources/web`，由桌面壳经
/// `OWO_WEB_UI_DIR` 指过来（`main.js` 在 `app.isPackaged` 时注入），
/// 避免回落到 `CARGO_MANIFEST_DIR` 这个编译期源码树路径（打包后不存在）。
/// 与 `pet_ui_dir()` 同一套约定：目录里必须有 index.html 才采纳。
fn desktop_web_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OWO_WEB_UI_DIR") {
        let path = PathBuf::from(dir);
        if path.join("index.html").is_file() {
            return path;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|parent| parent.parent())
        .map(|root| root.join("desktop").join("web"))
        .unwrap_or_else(|| PathBuf::from("desktop/web"))
}

/// 桌宠前端静态目录：环境变量 `OWO_PET_UI_DIR`（桌面端 overlay 启动引擎时
/// 传入其 `ui/pet` 目录）优先——目录里必须有 index.html 才采纳；
/// 否则回落 `desktop/web/pet`（仓库内兜底目录，可为占位页）。
fn pet_ui_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OWO_PET_UI_DIR") {
        let path = PathBuf::from(dir);
        if path.join("index.html").is_file() {
            return path;
        }
    }
    desktop_web_dir().join("pet")
}

/// 桌宠皮肤资产目录：`OWO_PET_ASSETS_DIR`（桌面端 overlay 的 `ui/assets`）
/// 优先，回落 `desktop/web/pet/assets`。
fn pet_assets_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OWO_PET_ASSETS_DIR") {
        let path = PathBuf::from(dir);
        if path.join("skins").is_dir() {
            return path;
        }
    }
    desktop_web_dir().join("pet").join("assets")
}

// （§12：to_session_info/load_session 与会话元数据处理器已外移至 session_api.rs）

async fn acquire_session_lock(
    state: &AppState,
    id: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>, (StatusCode, String)> {
    let lock = {
        let mut locks = state.turn_locks.lock().map_err(poison)?;
        locks
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    Ok(lock.lock_owned().await)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        api_version: OWO_API_VERSION.to_string(),
        auto_approve: turn_api::auto_approve_enabled(),
        build: load_build_info().clone(),
        // §4.2 实例握手：非秘密身份字段。桌面壳凭 instance_id/pid 核对
        // "这个服务是我启动的子进程"，不再盲复用同端口旧服务。
        instance_id: auth_token::desktop_instance_id(),
        pid: std::process::id(),
        stage: "ready".to_string(),
        build_id: load_build_info()
            .as_ref()
            .map(|info| info.commit.clone())
            .unwrap_or_else(|| "unknown".to_string()),
    })
}

static BUILD_INFO: OnceLock<Option<BuildInfo>> = OnceLock::new();

/// 构建身份解析（§5.1/§6.1.2/§7.1）——委托 `owo_build_info::identity()`
/// 单一链（① OWO_BUILD_INFO 覆写文件 → ② 编译期烧录 → ③ cwd 遗留文件 →
/// 不可用）。server 不再自持回退链副本；旧实现里「覆写文件损坏直接返回
/// None」的陷阱一并消除（损坏即跳过覆写、回落编译期事实，/health 身份
/// 永不为空）。Unavailable 时保持 build=None（等价旧行为的仅版本形态）。
fn load_build_info() -> &'static Option<BuildInfo> {
    BUILD_INFO.get_or_init(|| {
        let identity = owo_build_info::identity();
        if identity.source == owo_build_info::IdentitySource::Unavailable {
            None
        } else {
            Some(BuildInfo {
                commit: identity.commit,
                dirty: identity.dirty,
                built_at: identity.built_at,
            })
        }
    })
}

// （§7.1 收口：build-info.json 解析链已迁入 owo_build_info::identity()
//   单一实现，本文件不再持有副本。）

// （§12：GET /usage 的 usage_summary 已外移至 usage.rs::usage_model_summary，
//   经 usage_router 并入 build_router，路由面零变化）

// （§12：flush_audit 与 audit_list 已外移至 audit_api.rs）

// （§12：server_status/shutdown 已外移至 ops_api.rs）

// （§12：usage_topup 与 UsageTopupRequest 已外移至 usage.rs）

// （§12：list_sessions 已外移至 session_api.rs）
// （§12：list_skills/skill_detail/skill_edit/skill_enabled 已外移至 skills_api.rs）
// （§12：create_session/get_session 已外移至 session_api.rs）

// （§12：turn 编排器已外移至 turn_api.rs）

// （§12：附件三助手两处理器已外移至 session_api.rs·第二刀）

// （§12：respond_permission 已外移至 turn_api.rs）

// （§12：abort/diff/revert/fork/rewind/redo/export 已外移至 session_api.rs·第二刀）

// （§12：session_rename/archive/pin/children 与三请求结构已外移至 session_api.rs）
// （§12：export_session 已外移至 session_api.rs·第二刀）

// （§12：run_eval 已外移至 eval_api.rs；context_snapshot 已外移至 session_api.rs）

// （§12：desktop/vision 十四处理器与九请求结构 + gate_desktop_action/ensure_real_desktop 已外移至 desktop_api.rs）

// （§12：learn 域十六处理器与五请求结构、parse_sensitivity/ui_action_source 已外移至 learn_api.rs）

// （§12：proactive 三处理器 + stt_transcribe + automations 六处理器及请求结构
//   已外移至 assist_api.rs）

/// §13 批次三（R10 持久化组接线）：启动恢复——从 data_root 重放用量快照
/// （records + budgets + 硬熔断状态，崩溃后续接）。返回恢复的记录条数。
pub fn restore_usage_snapshot(data_root: &std::path::Path) -> usize {
    match usage::load_from(data_root) {
        Ok(restored) => restored,
        Err(error) => {
            tracing::warn!("用量快照恢复失败（跳过，继续启动）：{error}");
            0
        }
    }
}

/// §13 批次三：立即落盘一次用量快照（优雅关闭路径调用）。返回快照路径。
pub fn persist_usage_snapshot(data_root: &std::path::Path) -> Option<std::path::PathBuf> {
    match usage::persist_to(data_root) {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::warn!("用量快照落盘失败：{error}");
            None
        }
    }
}

/// §13 批次六（遥测接线）：应用 settings.json 的遥测开关（默认关；
/// 仅聚合功能计数/错误码分布/性能分位，数据字典经 /metrics/telemetry/status 暴露）。
pub fn apply_telemetry_setting(enabled: bool) {
    observability_api::set_telemetry_enabled(enabled);
}

/// §13 批次八（R10 文件日志接线）：初始化轮转文件日志
/// `<data_root>/logs/server.jsonl`（8MB × 5 份；落盘内容经统一脱敏——
/// 用户供给字段按字段名走 Redactor 策略，未知字段保守哈希）。
pub fn init_server_file_logging(data_root: &std::path::Path) {
    let dir = data_root.join("logs");
    let path = dir.join("server.jsonl");
    match logging::init_file_logging(&path, 8 * 1024 * 1024, 5) {
        Ok(()) => logging::info(
            "server",
            None,
            "文件日志已启用（8MB×5 轮转，内容经统一脱敏）",
        ),
        Err(error) => tracing::warn!("文件日志初始化失败（仅落 stderr）：{error}"),
    }
}

/// §13 批次八：关闭文件日志（优雅关闭序列，与 init 配对）。
pub fn close_server_file_logging() {
    logging::close_file_logging();
}

/// §13 批次八：生命周期审计事件（结构化日志面；detail 强制脱敏）。
pub fn logging_lifecycle_audit(action: &str, detail: &str) {
    logging::audit_event(action, None, detail);
}

/// §13 批次三：用量定时落盘常驻循环（每小时一次；首 tick 即时落盘一次，
/// 保证启动恢复后快照与内存态同步）。与 start_automation_loop 同批派生。
pub async fn start_usage_persistence_loop(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        interval.tick().await;
        let data_root = state.data_root.clone();
        let result = tokio::task::spawn_blocking(move || usage::persist_to(&data_root)).await;
        match result {
            Ok(Ok(path)) => tracing::debug!("用量快照已定时落盘：{}", path.display()),
            Ok(Err(error)) => tracing::warn!("用量快照定时落盘失败：{error}"),
            Err(error) => tracing::warn!("用量快照落盘任务 Join 失败：{error}"),
        }
    }
}

/// 自动化常驻循环：每秒检查到期任务——提醒走提醒列表，「跑任务」交给 Agent 执行。
const MAX_CONCURRENT_AUTOMATION_PROMPTS: usize = 2;

fn reserve_automation_prompt_slot(
    slots: &Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::TryAcquireError> {
    Arc::clone(slots).try_acquire_owned()
}

/// 自动化常驻循环：到期提醒即时入列；无人值守 Agent 任务最多并发 2 个。
pub async fn start_automation_loop(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let prompt_slots = Arc::new(tokio::sync::Semaphore::new(
        MAX_CONCURRENT_AUTOMATION_PROMPTS,
    ));
    loop {
        interval.tick().await;
        // Agent 执行始终在锁外；每个运行任务持有一个 permit，完成后自动释放。
        let mut pending_prompts: Vec<(
            String,
            String,
            String,
            String,
            tokio::sync::OwnedSemaphorePermit,
        )> = Vec::new();
        let fired = {
            let mut automations = state
                .automations
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let now = chrono::Utc::now();
            let mut fired = Vec::new();
            for id in automations.due_tasks(now) {
                let (task_name, action) = match automations.get(&id) {
                    Some(task) => (task.name.clone(), task.action.clone()),
                    None => continue,
                };
                let at = now.to_rfc3339();
                match action {
                    AutomationAction::RunPrompt { prompt } => {
                        let slot = reserve_automation_prompt_slot(&prompt_slots);
                        match automations.fire(&id, now) {
                            Ok(_) => match slot {
                                Ok(permit) => {
                                    pending_prompts.push((id, task_name, at, prompt, permit))
                                }
                                Err(_) => {
                                    let _ = automations.record_run(AutomationRun {
                                        task_id: id,
                                        task_name,
                                        at,
                                        status: "skipped".to_string(),
                                        output: Some(format!(
                                            "已跳过本次触发：自动化 Agent 并发上限为 {MAX_CONCURRENT_AUTOMATION_PROMPTS}，当前已满"
                                        )),
                                    });
                                }
                            },
                            Err(error) => {
                                let _ = automations.record_run(AutomationRun {
                                    task_id: id,
                                    task_name,
                                    at,
                                    status: "failed".to_string(),
                                    output: Some(error),
                                });
                            }
                        }
                    }
                    AutomationAction::Reminder { .. } => match automations.fire(&id, now) {
                        Ok(payload) => {
                            let _ = automations.record_run(AutomationRun {
                                task_id: id,
                                task_name,
                                at,
                                status: "ok".to_string(),
                                output: Some(payload.clone()),
                            });
                            fired.push(payload);
                        }
                        Err(error) => {
                            let _ = automations.record_run(AutomationRun {
                                task_id: id,
                                task_name,
                                at,
                                status: "failed".to_string(),
                                output: Some(error),
                            });
                        }
                    },
                }
            }
            fired
        };
        for (task_id, task_name, at, prompt, permit) in pending_prompts {
            tokio::spawn(run_automation_prompt(
                state.clone(),
                task_id,
                task_name,
                at,
                prompt,
                permit,
            ));
        }
        if !fired.is_empty() {
            if let Ok(mut audit) = state.agent.audit_log().lock() {
                audit.record("automation", "fire", None, Some(true), fired.join(" | "));
            }
            logging::audit_event("automation_fire", None, &fired.join(" | "));
        }
    }
}

#[cfg(test)]
mod automation_scheduler_tests {
    use super::{reserve_automation_prompt_slot, MAX_CONCURRENT_AUTOMATION_PROMPTS};
    use std::sync::Arc;

    #[test]
    fn automation_prompt_slots_are_bounded_and_released() {
        let slots = Arc::new(tokio::sync::Semaphore::new(
            MAX_CONCURRENT_AUTOMATION_PROMPTS,
        ));
        let held = (0..MAX_CONCURRENT_AUTOMATION_PROMPTS)
            .map(|_| reserve_automation_prompt_slot(&slots).expect("slot available"))
            .collect::<Vec<_>>();
        assert!(reserve_automation_prompt_slot(&slots).is_err());
        drop(held);
        assert!(reserve_automation_prompt_slot(&slots).is_ok());
    }
}

/// 执行一条「定时跑任务」：只读子代理跑提示词，结果写入执行记录。
///
/// 两个刻意的约束：
/// * **只读**：定时任务无人值守，没有可以把审批卡送达的客户端。放行写/执行的话，
///   工具调用只会阻塞在一个永远不会有人点的审批上，直到 300s 超时被拒——
///   任务"跑了"却什么也没做，且每次白等五分钟。
/// * **复用 `AgentWorker`**：与 goal/plan 的后台 worker 走同一条执行路径
///   （凭据检查、模型解析、审批器都在那里），避免出现第二套无人值守语义。
async fn run_automation_prompt(
    state: Arc<AppState>,
    task_id: String,
    task_name: String,
    at: String,
    prompt: String,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    // `_permit` 的生命周期覆盖整个 Agent 执行，完成/异常退出后自动归还并发槽。
    // `agent_worker` 声明在 `goal_api` 之下（goal_api/mod.rs: `pub mod agent_worker;`），
    // 根作用域里没有这个名字，必须写全路径。
    let worker =
        goal_api::agent_worker::AgentWorker::new(Arc::clone(&state.agent), state.workspace.clone());
    // `AgentWorker::run` 由 `Worker` trait 提供，作用域内必须引入（局部 use 最小污染）。
    use owo_agent_core::goal::Worker as _;
    let outcome = worker
        .run(&serde_json::json!({ "prompt": prompt, "read_only": true }))
        .await;
    let (status, output) = match outcome {
        Ok(text) => ("ok".to_string(), text),
        Err(error) => ("failed".to_string(), format!("执行失败：{error}")),
    };
    // 截断到 4000 字符：runs 是给人看的执行台账，不该被一次长回答撑爆。
    let output: String = output.chars().take(4000).collect();
    if let Ok(mut automations) = state.automations.lock() {
        let _ = automations.record_run(AutomationRun {
            task_id,
            task_name: task_name.clone(),
            at,
            status: status.clone(),
            output: Some(output),
        });
    }
    logging::audit_event(
        "automation_run",
        None,
        &format!("定时跑任务「{task_name}」{status}"),
    );
}

/// 静默观察器：每 2s 采样桌面状态（前台应用/标题哈希/剪贴板序列，受 L0 授权门控），
/// 并在模拟面下额外拉取模拟窗口日志；动作摘要（内容掩码）写入情景记忆。
pub async fn start_memory_observer(state: Arc<AppState>) {
    let mut sim_seen = 0usize;
    let mut desktop_prev: Option<owo_agent_core::DesktopSnapshot> = None;
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let l0_enabled = state
            .perception
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_enabled(owo_agent_core::PerceptionLayer::L0Event);
        if l0_enabled {
            let snapshot = owo_agent_core::sample_desktop();
            if let Some(prev) = &desktop_prev {
                if let Some(observation) = owo_agent_core::desktop_observation(prev, &snapshot) {
                    let mut memory = state
                        .memory
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let _ = memory.append(observation);
                }
            }
            desktop_prev = Some(snapshot);
        }
        let Some(base) = std::env::var("OWO_SIM_QQ_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let url = format!("{}/log", base.trim_end_matches('/'));
        let Ok(response) = reqwest::get(&url).await else {
            continue;
        };
        let Ok(value) = response.json::<Value>().await else {
            continue;
        };
        let Some(entries) = value.get("entries").and_then(Value::as_array) else {
            continue;
        };
        if entries.len() < sim_seen {
            // 模拟场景被 /reset 清空：从头重新计数。
            sim_seen = 0;
        }
        if entries.len() <= sim_seen {
            continue;
        }
        let mut memory = state
            .memory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for entry in &entries[sim_seen..] {
            if let Some(observation) = owo_agent_core::observation_from_sim_event(entry) {
                let _ = memory.append(observation);
            }
        }
        sim_seen = entries.len();
    }
}

// ---------- 设置与诊断 ----------

// （§12：settings_get/effective_runtime_config 已外移至 settings_api.rs）

// （§12：settings_egress 与 EgressRequest 已外移至 settings_api.rs）

// （§12：permissions_status/set_profile 与 grants_list/revoke 已外移至 settings_api.rs）

// （§12：whitelist_list/manage 已外移至 whitelist_api.rs）

// （§12：ChannelApprover/auto_approve_enabled/to_sse 已外移至 turn_api.rs）
// （§12：to_event 已外移至 turn_api.rs）
fn poison<T>(_error: std::sync::PoisonError<T>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, "状态锁中毒".to_string())
}

/// P3 录制自动观察：录制中每 2s 采样前台应用/剪贴板事件（掩码）进入样本。
/// 前台应用变化只记一次，剪贴板变化按序列号去重。
pub async fn start_observer(state: Arc<AppState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut last_app: Option<(String, String)> = None;
    let mut last_clipboard: u32 = 0;
    loop {
        interval.tick().await;
        let (foreground, clipboard_changed) = {
            let mut perception = state
                .perception
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let _ = perception.refresh_from_platform();
            let sequence = owo_agent_core::clipboard_sequence();
            let changed = sequence != 0 && sequence != last_clipboard;
            perception.refresh_clipboard(sequence);
            let _ = perception.refresh_from_uia(2, 32);
            let snapshot = perception.snapshot();
            (snapshot.foreground_app.clone(), changed)
        };
        let mut pipeline = state
            .pipeline
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pipeline.recorder.state() != LearnState::Recording {
            continue;
        }
        if let Some(app) = &foreground {
            let key = (app.id.clone(), app.title.clone());
            if last_app.as_ref() != Some(&key) {
                last_app = Some(key);
                let _ = pipeline.recorder.record(RecordedAction {
                    app_id: app.id.clone(),
                    anchor: SemanticAnchor {
                        app_id: Some(app.id.clone()),
                        role: None,
                        name: app.title.clone(),
                        parent: None,
                        element_id: None,
                    },
                    action_type: ActionType::Shortcut,
                    value_masked: true,
                    sensitive: false,
                    at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
        if clipboard_changed {
            last_clipboard = owo_agent_core::clipboard_sequence();
            if let Some(app) = &foreground {
                let _ = pipeline.recorder.record(RecordedAction {
                    app_id: app.id.clone(),
                    anchor: SemanticAnchor {
                        app_id: Some(app.id.clone()),
                        role: None,
                        name: "剪贴板".to_string(),
                        parent: None,
                        element_id: None,
                    },
                    action_type: ActionType::Inject,
                    value_masked: true,
                    sensitive: false,
                    at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }
    }
}

// ===========================================================================
// R10 契约治理：API 版本 / 弃用策略 / 错误码表 / JSON Schema 发布
// ===========================================================================

/// API 版本（`x-owo-api-version`）。破坏性变更递增 minor；弃用期 ≥2 个 minor。
pub const OWO_API_VERSION: &str = owo_build_info::API_VERSION;

/// 已弃用路由登记：(路径前缀, since, until, 替代建议)。
/// 命中时响应携带 `Deprecation` 头；当前无已弃用路由，破坏性变更前在此登记。
///
/// 路由/事件契约变更 RFC 登记（弃用策略落地：变更前登记 → 弃用期 ≥2 minor → 移除）：
/// - 2026-08-17（R10）：SSE 事件 data 统一携带 `v` 字段（v=1；旧客户端帧缺 v 视为 v=0）。
/// - 2026-08-17（R10）：新增 /schemas/{kind}/{version} 静态 JSON Schema 版本化发布。
/// - 2026-08-17（R10）：错误响应统一为 {error:{code,message,retry_after_ms,domain,reason,retryable}}。
const DEPRECATED_ROUTES: &[(&str, &str, &str, &str)] = &[];

/// 统一错误响应（R10 错误码表接入 HTTP 层）：(status, {error:{code,message,retry_after_ms,...}})。
/// `pub`：usage.rs 等 #[path] 独立编译的域模块经 crate 名引用（后代可见性不跨 crate 边界）。
pub fn api_error_response(
    code: &error_codes::ErrorCode,
    message: impl std::fmt::Display,
) -> (StatusCode, Json<Value>) {
    // §13 批次六：遥测错误码分布（默认关时零开销早退；仅 code 计数，无消息内容）。
    observability_api::record_telemetry_error(&format!("{}/{}", code.domain, code.reason));
    (
        StatusCode::from_u16(code.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(json!({
            "error": {
                "code": format!(
                    "{}/{}/{}",
                    code.domain,
                    code.reason,
                    if code.retryable { "retryable" } else { "not_retryable" }
                ),
                "message": message.to_string(),
                "domain": code.domain,
                "reason": code.reason,
                "retryable": code.retryable,
                "retry_after_ms": code.retry_after_ms,
            }
        })),
    )
}

/// 计算给定路径应附加的 `Deprecation` 头值（未命中返回 None）。
/// 独立为纯函数供契约测试直接覆盖命中/未命中与头格式（R12 收尾）。
pub fn deprecation_header_value_for(
    routes: &[(&str, &str, &str, &str)],
    path: &str,
) -> Option<String> {
    for (route, since, until, alternative) in routes {
        if path.starts_with(route) {
            return Some(format!(
                "{route}: since {since}, until {until} (use {alternative})"
            ));
        }
    }
    None
}

/// Deprecation 中间件：命中 DEPRECATED_ROUTES 的请求附加 `Deprecation` 响应头。
async fn deprecation_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    if let Some(value) = deprecation_header_value_for(DEPRECATED_ROUTES, &path) {
        if let Ok(value) = value.parse::<axum::http::HeaderValue>() {
            response.headers_mut().insert("Deprecation", value);
        }
    }
    response
}

// （§12：schemas_list/schema_get 与三份 schema 常量已外移至 schemas_api.rs）

#[cfg(test)]
mod tests {
    use crate::session_api::{rewind_session, sanitize_attachment_name};
    use crate::turn_api::auto_approve_enabled;
    use crate::AppState;
    use async_trait::async_trait;
    use base64::Engine;
    use owo_agent_core::permissions::Policy;
    use owo_agent_core::{
        Agent, AgentConfig, ChatMessage, ModelOutput, ModelProvider, Session, ToolRegistry,
        ToolSpec,
    };
    use owo_agent_protocol::RewindRequest;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn sanitizes_attachment_names() {
        assert_eq!(
            sanitize_attachment_name("report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            sanitize_attachment_name("a/b/c.txt").as_deref(),
            Some("c.txt")
        );
        assert_eq!(
            sanitize_attachment_name("..\\evil.txt").as_deref(),
            Some("evil.txt")
        );
        assert_eq!(
            sanitize_attachment_name("a:b*c?.txt").as_deref(),
            Some("bc.txt")
        );
        assert!(sanitize_attachment_name("").is_none());
        assert!(sanitize_attachment_name("   ").is_none());
        assert!(sanitize_attachment_name("x".repeat(201).as_str()).is_none());
    }

    #[test]
    fn auto_approve_env_detection() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::env::remove_var("OWO_AUTO_APPROVE");
        assert!(!auto_approve_enabled());
        std::env::set_var("OWO_AUTO_APPROVE", "1");
        assert!(auto_approve_enabled());
        std::env::set_var("OWO_AUTO_APPROVE", "TRUE");
        assert!(auto_approve_enabled());
        std::env::set_var("OWO_AUTO_APPROVE", "0");
        assert!(!auto_approve_enabled());
        std::env::remove_var("OWO_AUTO_APPROVE");
    }

    struct IdleProvider;

    #[async_trait]
    impl ModelProvider for IdleProvider {
        async fn complete(
            &self,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
        ) -> Result<ModelOutput, String> {
            Err("测试 Provider 不应被调用".to_string())
        }
    }

    #[tokio::test]
    async fn rewind_endpoint_restores_files_before_saving_session() {
        let root = std::env::temp_dir().join(format!("owo-server-rewind-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let data_root = root.join("data");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&data_root).unwrap();
        let path = workspace.join("changed.txt");
        std::fs::write(&path, "after").unwrap();

        let agent = Agent::new(
            Arc::new(IdleProvider),
            ToolRegistry::new(),
            Policy::new(&workspace),
            AgentConfig::default(),
        );
        let store_root = root.join("sessions");
        let state = Arc::new(AppState::new(
            agent,
            owo_agent_core::JsonSessionStore::new(&store_root),
            data_root.join("traces"),
            data_root,
            workspace.clone(),
        ));
        let mut session = Session::new(&workspace, "mock", None);
        session.push(ChatMessage::user("first".to_string()));
        session.push(ChatMessage::assistant_text("reply".to_string()));
        session.snapshots.insert(
            path.to_string_lossy().replace('\\', "/"),
            owo_agent_core::session::SnapshotEntry {
                original_b64: Some(base64::engine::general_purpose::STANDARD.encode("before")),
                expected_after_sha256: Some(owo_agent_core::CasStore::hash_of(b"after")),
                // turn 归属 = 写入时已存在的消息数：该写入发生在第一个用户回合内
                // （user 已入历史、assistant 未入），故 turn=1；keep=1 截掉该回合，
                // 快照必须被回滚（turn=0 表示"回合开始前"，语义上不属于被截段落）。
                turn: 1,
            },
        );
        state.store.save(&session).unwrap();
        state
            .sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());

        let id = session.id.clone();
        let response = rewind_session(
            axum::extract::State(Arc::clone(&state)),
            axum::extract::Path(id.clone()),
            axum::Json(RewindRequest { keep: 1 }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(response["ok"], json!(true));
        assert_eq!(response["removed"], json!(1));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before");
        assert_eq!(state.store.load(&id).unwrap().messages.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }
}

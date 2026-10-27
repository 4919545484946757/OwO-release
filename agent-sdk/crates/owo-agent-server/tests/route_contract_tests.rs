//! 路由面契约测试：验证 v0.5/v0.6 HTTP 接口全部注册且可达（防回归）。
//!
//! 背景（2026-08-14）：/locate/query、/memory/recall、/skills/health、/plugins、
//! /traces、/subagent/run、/project/rules、/mcp、/session/{id}/context 曾在服务端回归
//! 丢失（404），而桌面端仍在调用。
//!
//! 本文件以 `clients/ts/openapi.json`（2026-08-13 权威契约快照）为基准：
//! 1. 断言每个契约路径 + 方法可到达（非 404/405，合法输入 2xx、最小非法输入 400/422 亦可）。
//! 2. 断言 `/openapi.json` 登记了契约快照全部路径与 lib.rs 实际注册的全部路由。
//! 3. 真实 HTTP 服务启动 smoke（防 Router 构建 panic）。

use base64::Engine as _;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_protocol::SseEvent;
use owo_agent_server::build_router;
use std::sync::Arc;
use tower::ServiceExt;

/// 契约快照（权威）：agent-sdk/clients/ts/openapi.json。
const CONTRACT_SNAPSHOT: &str = include_str!("../../../clients/ts/openapi.json");

/// lib.rs / routes.rs 源码（用于提取实际注册路由，验证 openapi_spec 无漏登）。
/// 路由装配表在 routes.rs；lib.rs 保留公开再导出与其他装配。
const LIB_RS: &str = include_str!("../src/lib.rs");
const ROUTES_RS: &str = include_str!("../src/routes.rs");

/// 返回契约快照中的全部 (path, [method...])。
fn contract_endpoints() -> Vec<(String, Vec<String>)> {
    let snapshot = CONTRACT_SNAPSHOT.trim_start_matches('\u{feff}');
    let json: serde_json::Value = serde_json::from_str(snapshot).unwrap();
    json["paths"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, methods)| {
            let list = methods
                .as_object()
                .unwrap()
                .keys()
                .map(|m| m.to_uppercase())
                .collect();
            (path.clone(), list)
        })
        .collect()
}

#[test]
fn input_preflight_route_and_schema_are_synchronized() {
    let spec: serde_json::Value = serde_json::from_str(CONTRACT_SNAPSHOT).unwrap();
    let source: serde_json::Value =
        serde_json::from_str(include_str!("../src/openapi.json")).unwrap();
    assert!(registered_routes().contains(&"/input/preflight".to_string()));
    assert_eq!(
        source["paths"]["/input/preflight"],
        spec["paths"]["/input/preflight"]
    );
    assert_eq!(
        source["components"]["schemas"]["InputPreflightRequest"],
        spec["components"]["schemas"]["InputPreflightRequest"]
    );
    assert_eq!(
        spec["paths"]["/input/preflight"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/InputPreflightRequest"
    );
    assert!(
        serde_json::from_value::<owo_agent_protocol::InputPreflightRequest>(
            serde_json::json!({"verified":true})
        )
        .is_err()
    );
}

/// 从 lib.rs 与 routes.rs 提取实际注册的路由路径。
fn registered_routes() -> Vec<String> {
    let mut routes = Vec::new();
    for source in [LIB_RS, ROUTES_RS] {
        for line in source.lines() {
            if let Some(start) = line.find(".route(") {
                let rest = &line[start + ".route(".len()..];
                if let Some(quote) = rest.find('"') {
                    let after = &rest[quote + 1..];
                    if let Some(end) = after.find('"') {
                        routes.push(after[..end].to_string());
                    }
                }
            }
        }
    }
    routes.sort();
    routes.dedup();
    routes
}

/// 从全部 server 源文件（含模块路由）提取注册路由，防“模块内路由漏登记 OpenAPI”。
fn all_source_routes() -> Vec<String> {
    let mut routes: Vec<String> = Vec::new();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in source.lines() {
                if let Some(start) = line.find(".route(") {
                    let rest = &line[start + ".route(".len()..];
                    if let Some(quote) = rest.find('"') {
                        let after = &rest[quote + 1..];
                        if let Some(end) = after.find('"') {
                            let route = after[..end].to_string();
                            if !routes.contains(&route) {
                                routes.push(route);
                            }
                        }
                    }
                }
            }
        }
    }
    routes.sort();
    routes
}

/// 无外部依赖的最小模型 Provider（任何模型调用即失败）。
struct IdleProvider;

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

/// 可让 HTTP 流消费者落后的确定性 Provider；每个 delta 后让出执行权，确保取消分支可运行。
struct BurstProvider {
    emitted: Arc<std::sync::atomic::AtomicUsize>,
    limit: usize,
}

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for BurstProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Ok(owo_agent_core::ModelOutput::Text("complete".to_string()))
    }

    async fn complete_stream(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        for _ in 0..self.limit {
            on_delta("x".repeat(8 * 1024));
            self.emitted
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tokio::task::yield_now().await;
        }
        Ok(owo_agent_core::ModelOutput::Text("complete".to_string()))
    }
}

/// 在已发送一个增量后模拟 Provider 中途断流，用于验证实际 turn/SSE/持久回放故障路径。
struct DisconnectAfterDeltaProvider;

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for DisconnectAfterDeltaProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("非流式路径不应被调用".to_string())
    }

    async fn complete_stream(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        on_delta("partial-before-disconnect".to_string());
        Err("synthetic provider disconnect".to_string())
    }
}

/// 在慢客户端的有界队列溢出边界附近完成一批大增量，再返回 Provider 断流错误。
/// 不在循环中 yield：故障注入本身不依赖调度时序；事件仍逐个持久化，供测试验证回放。
struct BurstThenDisconnectProvider {
    emitted: Arc<std::sync::atomic::AtomicUsize>,
    completed: Arc<std::sync::atomic::AtomicBool>,
    delta_count: usize,
    delta_bytes: usize,
}

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for BurstThenDisconnectProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("非流式路径不应被调用".to_string())
    }

    async fn complete_stream(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
        on_delta: &mut (dyn FnMut(String) + Send),
    ) -> Result<owo_agent_core::ModelOutput, String> {
        for _ in 0..self.delta_count {
            on_delta("x".repeat(self.delta_bytes));
            self.emitted
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        self.completed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Err("synthetic provider disconnect after burst".to_string())
    }
}

/// 首轮请求一个受审批保护的文件写入，第二轮结束对话。
struct ApprovedWriteThenFinalProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for ApprovedWriteThenFinalProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        use std::sync::atomic::Ordering;

        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(owo_agent_core::ModelOutput::ToolCalls(vec![
                owo_agent_core::gateway::ToolCall {
                    id: "write-1".to_string(),
                    name: "write_file".to_string(),
                    arguments: serde_json::json!({
                        "path": "approved-output.txt",
                        "content": "approved content\n"
                    }),
                },
            ]))
        } else {
            Ok(owo_agent_core::ModelOutput::Text(
                "write complete".to_string(),
            ))
        }
    }
}

/// 环境变量是进程级共享（OWO_API_RPM_GLOBAL 等），而 AppState::new 构建时读取它们；
/// 并行测试若一方 set_var、另一方同时 new state，就会把限流配置泄漏给对方。
/// 因此所有「env 读取敏感」的 state 构建统一经此锁串行化。
static STATE_ENV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let _guard = STATE_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    // 限流用例可能已把进程级 RPM 调小；恢复默认（600）再构建，避免污染其他用例。
    std::env::remove_var("OWO_API_RPM_GLOBAL");
    build_state_inner().await
}

/// 无锁的 state 构建（调用方负责持有 STATE_ENV_LOCK 并管理 OWO_API_* 环境变量）。
async fn build_state_inner() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(IdleProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace,
    ));
    (state, temp)
}

/// 构造请求（R7：自动附带本 state 的 bearer token）。
fn request(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(header::HOST, "127.0.0.1:4096")
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

/// 不带 token 的请求（鉴权负例）。
fn anonymous_request(
    method: &str,
    path: &str,
    body: Option<&str>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{header, Method, Request};
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path);
    if let Some(b) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        return builder.body(axum::body::Body::from(b.to_string())).unwrap();
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

/// 契约路径的最小合法/非法请求体（缺省 {}，避免 route 存在性判断被 body 校验干扰）。
fn sample_body(path: &str) -> Option<&'static str> {
    match path {
        "/session" => Some(r#"{"workspace":".","model":"idle"}"#),
        "/session/{id}/turn" => Some(r#"{"prompt":"hi"}"#),
        // 自定义提示词：用合法短文本，可达性测试要 200。
        "/session/{id}/system-prompt" => Some(r#"{"prompt":"契约样例提示词"}"#),
        "/prompts" => Some(r#"{"name":"契约样例","content":"契约样例提示词"}"#),
        "/permissions" => Some(r#"{"profile":"workspace"}"#),
        // §4.5.3 结构化配置：给一份**合法且收紧**的 body（可达性测试要 200，不是 400）。
        "/permissions/spec" => Some(
            r#"{"spec":{"filesystem":"workspace_read","command":"allowlisted","network":"deny","persistence":"once","scopes":[]}}"#,
        ),
        "/permissions/grants/revoke" => Some(r#"{"grant_id":"contract-no-such"}"#),
        "/session/{id}/permission/{request_id}" => Some(r#"{"allow":true}"#),
        "/plugins/{id}/enabled" => Some(r#"{"enabled":false}"#),
        "/subagent/run" => Some(r#"{"prompt":"hi","read_only":true}"#),
        "/project/rules" => Some(r#"{"content":"rules"}"#),
        "/mcp/add" => {
            Some(r#"{"name":"__missing__","transport":"http","url":"http://127.0.0.1:1"}"#)
        }
        "/mcp/remove" => Some(r#"{"name":"__missing__"}"#),
        "/mcp/reconnect" => Some(r#"{"name":"__missing__"}"#),
        "/mcp/enabled" => Some(r#"{"name":"__missing__","enabled":false}"#),
        "/locate/query" => Some(r#"{}"#),
        // 「测试连接」无请求体（端点取自配置），但要走同一个 sample_body 查表面，
        // 缺这一条会被 reachable 测试当成"没有样例体的新路由"而跳过断言。
        "/settings/provider-test" => Some(r#"{}"#),
        "/workspace/trust" => Some(r#"{"trusted":true}"#),
        "/memory/mine-skill" => {
            Some(r#"{"name":"t","target_apps":[],"sensitivity":"low","description":"d"}"#)
        }
        "/skills/{name}/enabled" => Some(r#"{"enabled":false}"#),
        // `/goal` 会话级目标状态：默认 action=set 且 objective 必填（可达性测试要 200）。
        "/session/{id}/goal" => Some(r#"{"action":"set","objective":"契约测试目标"}"#),
        // 会话工作模式：agent / chat；未知值 400，可达性测试给合法值。
        "/session/{id}/mode" => Some(r#"{"mode":"chat"}"#),
        // 逐 hunk 审阅的应用：缺 accept/reject 会 422，可达性测试只要求路由存在与请求体可解析。
        "/session/{id}/hunks/apply" => Some(r#"{"path":"fixture.txt","accept":[0]}"#),
        // 用户自定义工作流：非法配置会 422（可达性测试接受 4xx，这里给"能解析"的最小体）。
        "/workflows/user" => Some(
            r#"{"template_id":"contract-test","name":"契约测试","mode":"team","roles":[],"applicability":"契约","created_at":""}"#,
        ),
        "/workflows/user/validate" => Some(
            r#"{"template_id":"contract-test","name":"契约测试","mode":"team","roles":[],"applicability":"契约","created_at":""}"#,
        ),
        // 草稿生成会调用模型（契约测试 provider 返回错误 → 502；可达性只要求非 404/405）。
        "/workflows/draft" => Some(r#"{"description":"契约测试工作流"}"#),
        // 收敛接口同样会调用模型（契约测试 provider 报错 → 502，可达性只要求非 404/405）。
        "/workflows/draft/converge" => Some(r#"{"description":"契约测试收敛","max_rounds":2}"#),
        "/workflows/draft/{id}/verify" => Some(r#"{"team_id":"team-missing"}"#),
        // 社区共享：导出（未知 id → 404）、导入/评审（非法 base64 → 422）。
        "/workflows/user/{id}/export" => Some(r#"{}"#),
        "/workflows/import/review" => Some(r#"{"package_b64":"not-base64"}"#),
        "/workflows/import" => Some(r#"{"package_b64":"not-base64"}"#),
        "/computer-use/task" => Some(r#"{"target_app":"notepad","description":"d"}"#),
        "/computer-use/task/{id}/{action}" => Some(r#"{"reason":"r"}"#),
        "/computer-use/sensitive-check" => Some(r#"{"name":"PasswordBox"}"#),
        "/notes" => Some(r#"{"title":"契约测试笔记"}"#),
        "/notes/import" => Some(r##"{"title":"t","markdown":"# hi"}"##),
        "/notes/{id}/blocks" => Some(r#"{"kind":"paragraph","text":"t"}"#),
        "/notes/{id}/blocks/move" => Some(r#"{"block_id":"no-such-block"}"#),
        "/goal" => Some(r#"{"objective":"契约测试目标"}"#),
        "/goal/{id}/plan" => Some(r#"{"steps":[{"id":"a","worker":"echo"}]}"#),
        "/goal/{id}/run" => Some(r#"{}"#),
        "/goal/{id}/abort" => Some(r#"{}"#),
        "/plugins/market/seed" => Some(r#"{"entries":[]}"#),
        "/plugins/market/verify" => Some(r#"{"dir":"."}"#),
        "/plugins/market/install" => Some(r#"{"dir":"."}"#),
        "/plugins/market/update" => Some(r#"{"id":"x","dir":"."}"#),
        "/plugins/market/uninstall" => Some(r#"{"id":"x"}"#),
        "/workflow/validate" => Some(
            r#"{"id":"ct","name":"ct-flow","version":1,"triggers":[{"id":"t1","kind":{"kind":"manual"}}],"permissions":[{"scope":"fs.write","mode":"allow"}],"preconditions":[],"rollback_points":[],"max_steps":100,"subflow_depth_limit":5,"steps":[{"kind":"notify","id":"n1","message":"ok"}]}"#,
        ),
        "/workflow/{name}/run" => Some(r#"{}"#),
        "/workflow/run/{run_id}/abort" => Some(r#"{}"#),
        "/workflow/run/{run_id}/approval" => Some(r#"{"decision":"approve"}"#),
        "/plugins/market/refresh" => Some(r#"{}"#),
        "/plugins/market/install-remote" => Some(r#"{"id":"x"}"#),
        "/team/export" => Some(r#"{"type":"flow","id":"x"}"#),
        "/team/review" => Some(r#"{"package_b64":"aGVsbG8="}"#),
        "/team/import" => Some(r#"{"package_b64":"aGVsbG8="}"#),
        // 故意给不存在的套件路径：无论是否配置 OPENAI_API_KEY 都快速失败（400），
        // 避免契约测试在真实凭据环境下触发真实模型 eval（分钟级挂起）。
        "/eval/gate/run" => Some(r#"{"suite":"__contract_missing_suite__"}"#),
        "/memory/graph/link" => Some(r#"{"a":"x","b":"y","relation":"r"}"#),
        "/intent/parse" => Some(r#"{"text":"hi"}"#),
        "/command/run" => Some(r#"{"mode":"text","text":"hi"}"#),
        // R12 /fleet/*（Agent 2 控制面）：合法最小输入应 200。
        "/fleet/nodes/register" => {
            Some(r#"{"node_id":"ct-node","card":{"worker":"ct-node","actions":["shell"]}}"#)
        }
        "/fleet/tasks/submit" => Some(r#"{"task_id":"ct-task","worker":"ct-node","input":{}}"#),
        "/fleet/approvals/{id}/respond" => Some(r#"{"decision":"reject","approved_by":"ct"}"#),
        // V1 三日 /product-eval/*（第四路）：reference 免模型最小合法体（202 受理；
        // test_state 的 workspace 不含 evals/v1 时则 400「suite 加载失败」，两种都证明路由可达）。
        "/product-eval/runs" => Some(
            r#"{"suite":"v1","execution":"reference","modes":["single"],"repetitions":1,"category":null,"only":null}"#,
        ),
        // R13 WorkSwarm（§8.5）：空 objective 快速 400，避免契约测试触发真实模型运行。
        // 十一期（二路）additive：roles[].model / write_paths + 团队统一 model /
        // parallel / max_agent_members / budget.max_parallel 进入请求体解析面。
        "/teams" => Some(
            r#"{"objective":"","roles":[{"role":"w1","assignee":"agent","model":"glm-5.3-flash","write_paths":["src/a"]}],"model":"glm-5.3-flashx","parallel":true,"max_agent_members":4,"budget":{"max_parallel":2}}"#,
        ),
        // 五期（第一路）：组队策略（auto 缺省）；空 objective 快速 400 语义不变。
        // "/teams" 的 strategy 字段随 auto 缺省可省略；此处维持最小体。
        "/teams/{id}/steer" => Some(r#"{"command":"cancel"}"#),
        // 五期（第二路）：返工请求冻结契约；占位 team/review 不存在 → 404（资源缺失）。
        "/artifacts/{id}/rework" => Some(
            r#"{"team_id":"no-such-team","review_id":"no-such-review","instruction":"契约测试返工指令"}"#,
        ),
        // 六期（第二路）：工作区绑定冻结契约；占位 project 不存在 → 404（资源缺失）。
        "/projects/{id}/workspace" => {
            Some(r#"{"root":".","read_only":true,"write_allowed_paths":[],"tree_depth":2}"#)
        }
        // 六期（第三路）：模板目录安装（幂等）；占位 id 不存在 → 404（资源缺失）。
        "/teams/templates/catalog/{id}/install" => Some(r#"{}"#),
        "/tasks/{id}/handoff" => {
            Some(r#"{"team_id":"no-such-team","from_member":"m-x","completed_summary":"done"}"#)
        }
        "/tasks/{id}/human-result" => Some(r#"{"team_id":"no-such-team","result":"ok"}"#),
        "/teams/templates/proposals/{proposal_id}/adopt" => Some(r#"{}"#),
        "/teams/templates/proposals/{proposal_id}/reject" => Some(r#"{}"#),
        // R1 DesktopWorld/WorldModel 训练闭环（§5.11/§5.12）：合法最小输入；
        // 资源型路径用占位 {id}（不存在资源 → 404 由资源缺失产生，见 resource_404_ok）。
        "/desktop-envs" => Some(r#"{"task":{"task_id":"ct-dw","app":"chat","seed":1}}"#),
        "/desktop-envs/{id}/reset" => Some(
            r#"{"task":{"task_id":"ct-dw","app":"chat","seed":1},"lease":{"owner":"ct","token":"ct","epoch":0}}"#,
        ),
        "/desktop-envs/{id}/lease" => Some(r#"{"op":"acquire"}"#),
        "/desktop-envs/{id}/step" => Some(
            r#"{"action":{"action_id":"ct-a1","kind":"wait","semantic_intent":"等待"},"lease":{"owner":"ct","token":"ct","epoch":0}}"#,
        ),
        "/desktop-envs/{id}/snapshot" => Some(r#"{}"#),
        "/desktop-envs/{id}/restore" => {
            Some(r#"{"snapshot":"no-such-snapshot","lease":{"owner":"ct","token":"ct","epoch":0}}"#)
        }
        "/desktop-envs/{id}/judge" => Some(r#"{"success":{"name":"ct","assertions":[]}}"#),
        "/desktop-envs/{id}/inject-fault" => Some(
            r#"{"fault":{"type":"sluggish_steps","steps":1},"lease":{"owner":"ct","token":"ct","epoch":0}}"#,
        ),
        "/world-model/predict" => Some(
            r#"{"env_id":"no-such-env","action":{"action_id":"ct-a1","kind":"wait","semantic_intent":"等待"}}"#,
        ),
        "/transitions/{id}" => Some(r#"{}"#),
        "/datasets/build" => Some(r#"{}"#),
        "/datasets/{id}/manifest" => Some(r#"{}"#),
        "/model-candidates" => Some(r#"{"model_id":"ct-model","model_version":"0.0.1"}"#),
        "/model-candidates/{id}/promote" => Some(r#"{"ack":true,"reason":"契约测试晋升"}"#),
        // 八期（第二路）：ChangeSet 动作——idempotency_key 必填；占位 id → 404（资源缺失）。
        "/change-sets/{id}/accept" => Some(r#"{"idempotency_key":"contract-ct"}"#),
        "/change-sets/{id}/reject" => Some(r#"{"idempotency_key":"contract-ct"}"#),
        "/change-sets/{id}/revert" => Some(r#"{"idempotency_key":"contract-ct"}"#),
        // 八期（第三路）：Human Inbox 领取/释放/处理——user 必填；占位 id → 404（资源缺失）。
        "/human/inbox/{id}/claim" => Some(r#"{"user":"contract-ct"}"#),
        "/human/inbox/{id}/release" => Some(r#"{"user":"contract-ct"}"#),
        "/human/inbox/{id}/resolve" => Some(r#"{"user":"contract-ct"}"#),
        _ => Some(r#"{}"#),
    }
}

/// 占位路径参数（资源不存在时应返回 400/422/500，而不是 404——404 意味着路由缺失）。
fn sample_path(path: &str, session_id: &str) -> String {
    path.replace("{id}", session_id)
        .replace("{request_id}", "no-such-request")
        .replace("{name}", "no-such-name")
        .replace("{index}", "0")
        .replace("{app_id}", "no-such-app")
        .replace("{format}", "md")
        .replace("{action}", "cancel")
        .replace("{block_id}", "no-such-block")
        .replace("{run_id}", "no-such-run")
        .replace("{node_id}", "no-such-node")
        // R10：/schemas/{kind}/{version} 契约路径（合法值应 200）。
        .replace("{kind}", "owflow")
        .replace("{version}", "v1")
        // R13：/teams/templates/proposals/{proposal_id}/adopt
        .replace("{proposal_id}", "no-such-proposal")
}

/// 资源型 404 白名单：路由已注册且方法匹配，但目标资源不存在时 handler 正确地返回 404。
/// 此类路径的契约断言为「非 405」+（非 404 或 404 由资源缺失产生）。
fn resource_404_ok(path: &str) -> bool {
    matches!(
        path,
        "/skills/{name}"
            | "/skills/{name}/enabled"
            | "/learn/packages/{name}"
            | "/learn/export/{name}"
            | "/traces/{index}"
            | "/mcp/remove"
            | "/mcp/reconnect"
            | "/mcp/enabled"
            | "/automations/{id}/toggle"
            | "/perception/template/{app_id}"
            | "/computer-use/task/{id}/run"
            | "/cloud/tasks/{id}"
            | "/cloud/tasks/{id}/result"
            | "/prompts/{id}"
            | "/notes/{id}"
            | "/notes/{id}/export/{format}"
            | "/notes/{id}/blocks"
            | "/notes/{id}/blocks/move"
            | "/notes/{id}/blocks/{block_id}"
            | "/notes/{id}/reindex"
            | "/goal/{id}"
            | "/goal/{id}/plan"
            | "/goal/{id}/run"
            | "/goal/{id}/status"
            | "/goal/{id}/abort"
            | "/goal/{id}/audit"
            | "/goal/{id}/runs"
            | "/workflow/{name}"
            | "/workflow/{name}/run"
            // 用户自定义工作流：删除不存在的模板按契约 404。
            | "/workflows/user/{id}"
            | "/workflows/user/{id}/revisions"
            | "/workflows/draft/{id}"
            | "/workflows/draft/{id}/verify"
            | "/workflows/draft/{id}/adopt"
            | "/workflows/user/{id}/export"
            | "/workflow/{name}/runs"
            | "/workflow/run/{run_id}"
            | "/workflow/run/{run_id}/abort"
            | "/workflow/run/{run_id}/audit"
            | "/workflow/run/{run_id}/approval"
            | "/workflow/run/{run_id}/events"
            | "/plugins/market/uninstall"
            | "/eval/gate/report"
            | "/team/export"
            | "/eval/run"
            | "/session/{id}/permission/{request_id}"
            | "/session/{id}/hunks/apply"
            // R12 /fleet/*（任务/审批资源不存在 → 404 由资源缺失产生，非路由缺失）。
            | "/fleet/nodes/{node_id}/heartbeat"
            | "/fleet/nodes/{node_id}/tasks"
            | "/fleet/tasks/{id}"
            | "/fleet/tasks/{id}/claim"
            | "/fleet/tasks/{id}/progress"
            | "/fleet/tasks/{id}/result"
            | "/fleet/tasks/{id}/cancel-ack"
            | "/fleet/tasks/{id}/cancel"
            | "/fleet/tasks/{id}/events"
            | "/fleet/approvals/{id}/respond"
            // R13 WorkSwarm（§8.5）：占位 id 指向不存在资源 → 404 由资源缺失产生，非路由缺失。
            | "/teams/{id}"
            | "/teams/{id}/tasks"
            | "/teams/{id}/conversation"
            | "/teams/{id}/steer"
            | "/teams/{id}/events"
            | "/teams/{id}/timeline"
            | "/projects/{id}"
            | "/projects/{id}/artifacts"
            | "/tasks/{id}/handoff"
            | "/tasks/{id}/human-result"
            | "/teams/templates/proposals/{proposal_id}/adopt"
            | "/teams/templates/proposals/{proposal_id}/reject"
            // R1 DesktopWorld/WorldModel（§5.11/§5.12）：占位 id 指向不存在资源 → 404 由资源缺失产生，非路由缺失。
            | "/desktop-envs/{id}/reset"
            | "/desktop-envs/{id}/lease"
            | "/desktop-envs/{id}/observe"
            | "/desktop-envs/{id}/step"
            | "/desktop-envs/{id}/snapshot"
            | "/desktop-envs/{id}/restore"
            | "/desktop-envs/{id}/judge"
            | "/desktop-envs/{id}/inject-fault"
            | "/world-model/predict"
            | "/transitions/{id}"
            | "/datasets/{id}/manifest"
            | "/model-candidates/{id}/promote"
            // V1 三日 ProductEval（第四路）：占位 id 指向不存在运行 → 404 由资源缺失产生，非路由缺失。
            | "/product-eval/runs/{id}"
            | "/product-eval/runs/{id}/cancel"
            // V1 四期（第三路）：评审闭环——占位 id 指向不存在产物 → 404 由资源缺失产生，非路由缺失。
            | "/artifacts/{id}/review"
            | "/artifacts/{id}/history"
            // 五期：返工/交付物/团队指标/脱敏诊断——占位 id 指向不存在资源 → 404 非路由缺失。
            | "/artifacts/{id}/rework"
            | "/projects/{id}/deliverables"
            | "/teams/{id}/metrics"
            | "/teams/{id}/context"
            | "/teams/{id}/diagnostic"
            // 六期：工作区绑定四路由 + 模板目录安装——占位 id 指向不存在资源 → 404 非路由缺失。
            | "/projects/{id}/workspace"
            | "/projects/{id}/workspace/tree"
            | "/projects/{id}/workspace/git-status"
            | "/teams/templates/catalog/{id}/install"
            // 七期（第二路）：Worker 代码变更追踪——占位 id 指向不存在项目/团队 → 404 非路由缺失。
            | "/projects/{id}/workspace/changes"
            // 七期（第三路）：交付三路由——接线前路由未注册（fallback 404），接线后 404 由资源缺失产生；均非路由缺失。
            | "/artifacts/{id}/content"
            | "/artifacts/{id}/metadata"
            | "/projects/{id}/delivery-manifest"
            // 八期（第二路）：ChangeSet 审批闭环——占位 id 指向不存在资源 → 404 非路由缺失。
            | "/teams/{id}/change-sets"
            | "/change-sets/{id}"
            | "/change-sets/{id}/accept"
            | "/change-sets/{id}/reject"
            | "/change-sets/{id}/revert"
            // 八期（第三路）：统一 Human Inbox——占位 id 指向不存在待办 → 404 非路由缺失。
            | "/human/inbox/{id}"
            | "/human/inbox/{id}/claim"
            | "/human/inbox/{id}/release"
            | "/human/inbox/{id}/resolve"
            // §5.4 授权记忆撤销——占位 grant_id 指向不存在授权 → 404 非路由缺失。
            | "/permissions/grants/revoke"
    )
}

#[tokio::test]
async fn all_contract_endpoints_are_reachable() {
    // §3.2：本测试会触发 mutation 领域失效发布；与矩阵测试共用发布锁，
    // 避免并行测试把“恰好一次”断言污染成不稳定结果。
    let _guard = invalidate_guard().await;
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 创建真实会话，使 /session/{id}/* 系列走真实资源路径（强断言非 404）。
    let create_resp = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":".","model":"idle"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(create_resp.status().as_u16(), 200, "POST /session 应 200");
    let create_bytes = axum::body::to_bytes(create_resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let session_id = serde_json::from_slice::<serde_json::Value>(&create_bytes)
        .unwrap()
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("create_session 响应应含 id")
        .to_string();

    let mut failed: Vec<String> = Vec::new();
    for (path, methods) in contract_endpoints() {
        // /fs/pick-directory 会拉起系统原生「选择文件夹」对话框（阻塞至多 180s），
        // 契约测试只校验路由登记与 OpenAPI 一致性，不实际调起 GUI（取优合并自远端
        // engine 的同名跳过逻辑；漏掉会让本测试在桌面上真实弹窗直至 60s 超时）。
        if path == "/fs/pick-directory" {
            continue;
        }
        for method in methods {
            let is_get_like = matches!(method.as_str(), "GET" | "DELETE");
            let body = if is_get_like {
                None
            } else {
                Some(sample_body(&path).unwrap_or("{}"))
            };
            // DELETE must not consume the fixture used by the remaining routes.
            let route_session_id = if path == "/session/{id}" && method == "DELETE" {
                let disposable = state.store.create(&state.workspace, "idle", None).unwrap();
                state
                    .sessions
                    .lock()
                    .unwrap()
                    .insert(disposable.id.clone(), disposable.clone());
                disposable.id
            } else {
                session_id.clone()
            };
            let uri = sample_path(&path, &route_session_id);
            // 每请求加超时：SSE/慢路径不应拖垮契约测试（挂起即视为失败并指明路径）。
            let response = match tokio::time::timeout(
                std::time::Duration::from_secs(60),
                app.clone().oneshot(request(&state, &method, &uri, body)),
            )
            .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    failed.push(format!("{method} {path} → 请求错误 {error}"));
                    continue;
                }
                Err(_) => {
                    failed.push(format!("{method} {path} → 超时（60s 未返回）"));
                    continue;
                }
            };
            let status = response.status().as_u16();
            let ok = if status == 405 {
                false
            } else if status == 404 {
                resource_404_ok(&path)
            } else {
                true
            };
            eprintln!("contract {method} {uri} → {status}");
            if !ok {
                failed.push(format!("{method} {path} → {status}"));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "契约路径不可达（404/405）：\n{}",
        failed.join("\n")
    );
}

#[tokio::test]
async fn turn_event_replay_filters_by_turn_and_resumes_after_session_seq() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let session = state.store.create(&state.workspace, "idle", None).unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), session.clone());
    state
        .store
        .append_turn_event(
            &session.id,
            "turn-one",
            &SseEvent::TokenDelta {
                delta: "first".to_string(),
            },
        )
        .unwrap();
    state
        .store
        .append_turn_event(
            &session.id,
            "turn-one",
            &SseEvent::Final {
                text: "complete".to_string(),
            },
        )
        .unwrap();
    state
        .store
        .append_turn_event(
            &session.id,
            "turn-two",
            &SseEvent::TokenDelta {
                delta: "other turn".to_string(),
            },
        )
        .unwrap();

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!(
                "/session/{}/turn/events?turn_id=turn-one&after_seq=1&limit=10",
                session.id
            ),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["events"].as_array().unwrap().len(), 1);
    assert_eq!(body["events"][0]["seq"], serde_json::json!(2));
    assert_eq!(body["events"][0]["turn_id"], "turn-one");
    assert_eq!(body["events"][0]["payload"]["type"], "final");
    assert_eq!(body["active"], false);
    // A streamed Final frame is not a durable completion marker by itself.
    // The host writes TurnStats only after successful session persistence.
    assert_eq!(body["state"], "interrupted");
    assert_eq!(body["next_after_seq"], 2);

    state
        .turn_controls
        .lock()
        .unwrap()
        .register(&session.id, "turn-live")
        .unwrap();
    let live_response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!(
                "/session/{}/turn/events?turn_id=turn-live&after_seq=0&limit=10",
                session.id
            ),
            None,
        ))
        .await
        .unwrap();
    let live_bytes = axum::body::to_bytes(live_response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let live: serde_json::Value = serde_json::from_slice(&live_bytes).unwrap();
    assert_eq!(live["active"], true);
    assert_eq!(live["state"], "active");

    state
        .turn_controls
        .lock()
        .unwrap()
        .finish(&session.id, "turn-live");
    let interrupted_response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!(
                "/session/{}/turn/events?turn_id=turn-live&after_seq=0&limit=10",
                session.id
            ),
            None,
        ))
        .await
        .unwrap();
    let interrupted_bytes = axum::body::to_bytes(interrupted_response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let interrupted: serde_json::Value = serde_json::from_slice(&interrupted_bytes).unwrap();
    assert_eq!(interrupted["active"], false);
    assert_eq!(interrupted["state"], "interrupted");

    state
        .store
        .append_turn_event(
            &session.id,
            "turn-failed",
            &SseEvent::Progress {
                message: "turn failed: provider disconnected".to_string(),
            },
        )
        .unwrap();
    let failed_response = app
        .oneshot(request(
            &state,
            "GET",
            &format!(
                "/session/{}/turn/events?turn_id=turn-failed&after_seq=0&limit=10",
                session.id
            ),
            None,
        ))
        .await
        .unwrap();
    let failed_bytes = axum::body::to_bytes(failed_response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let failed: serde_json::Value = serde_json::from_slice(&failed_bytes).unwrap();
    assert_eq!(failed["active"], false);
    assert_eq!(failed["state"], "failed");
}

#[tokio::test]
async fn session_revert_conflict_returns_structured_409_and_preserves_user_edit() {
    let (state, _temp) = test_state().await;
    let path = state.workspace.join("user-edited.txt");
    std::fs::write(&path, "user's later edit").unwrap();
    let mut session = state.store.create(&state.workspace, "idle", None).unwrap();
    session.snapshots.insert(
        path.to_string_lossy().replace('\\', "/"),
        owo_agent_core::session::SnapshotEntry {
            original_b64: Some(base64::engine::general_purpose::STANDARD.encode("before")),
            expected_after_sha256: Some(owo_agent_core::CasStore::hash_of(b"agent version")),
            turn: 0,
        },
    );
    state.store.save(&session).unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), session.clone());

    let response = build_router(Arc::clone(&state))
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/revert", session.id),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["error"]["code"],
        "storage/revert_conflict/not_retryable"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "user's later edit",
        "拒绝冲突撤销时不得覆盖用户新内容"
    );
}

#[tokio::test]
async fn slow_http_turn_client_triggers_bounded_queue_overflow_and_cancels_provider() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let emitted = Arc::new(AtomicUsize::new(0));
    let provider = Arc::new(BurstProvider {
        emitted: Arc::clone(&emitted),
        limit: 1024,
    });
    let agent = Agent::new(
        provider,
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let session = state.store.create(&workspace, "idle", None).unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), session.clone());

    let app = build_router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let token = state.auth_token.token().to_string();
    let before = client
        .get(format!("{base}/metrics/runtime"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["turn_sse"]["slow_consumers_total"]
        .as_u64()
        .unwrap();

    // 收到 headers 后故意不读取 response body，模拟网络可连但消费端停滞。
    let response = client
        .post(format!("{base}/session/{}/turn", session.id))
        .bearer_auth(&token)
        .json(&serde_json::json!({"prompt":"bounded slow-client probe"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let turn_id = response
        .headers()
        .get("x-owo-turn-id")
        .expect("turn response must expose replay identity")
        .to_str()
        .unwrap()
        .to_string();

    let after = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value = client
                .get(format!("{base}/metrics/runtime"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            let slow = value["turn_sse"]["slow_consumers_total"].as_u64().unwrap();
            if slow > before {
                break slow;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("不读取 HTTP SSE body 应触发有界队列慢消费者指标");
    assert!(after > before);

    tokio::time::timeout(Duration::from_secs(2), async {
        while emitted.load(Ordering::Relaxed) >= 1024 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("队列过载后 Agent 应取消 Provider 流，而不是消费完整个 burst");
    assert!(emitted.load(Ordering::Relaxed) < 1024);

    drop(response);
    let recovered = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = client
                .get(format!(
                    "{base}/session/{}/turn/events?turn_id={turn_id}&after_seq=0&limit=512",
                    session.id
                ))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            if page["state"] != "active" {
                break page;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("慢消费者触发取消后应持久化可补拉的回合终态");
    assert_eq!(recovered["state"], "failed");
    assert!(
        recovered["events"].as_array().unwrap().iter().any(|event| {
            event["payload"]["type"] == "token_delta"
                && event["payload"]["delta"]
                    .as_str()
                    .is_some_and(|delta| !delta.is_empty())
        }),
        "断线补拉至少应恢复一个已产生的部分增量"
    );
    server.abort();
}

#[tokio::test]
async fn concurrent_slow_http_turn_clients_each_replay_failed_state() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    const CONNECTIONS: usize = 4;
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let emitted = Arc::new(AtomicUsize::new(0));
    let agent = Agent::new(
        Arc::new(BurstProvider {
            emitted: Arc::clone(&emitted),
            limit: 1024,
        }),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let mut session_ids = Vec::with_capacity(CONNECTIONS);
    for index in 0..CONNECTIONS {
        let session = state
            .store
            .create(&workspace, &format!("multi-slow-{index}"), None)
            .unwrap();
        state
            .sessions
            .lock()
            .unwrap()
            .insert(session.id.clone(), session.clone());
        session_ids.push(session.id);
    }

    let app = build_router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let token = state.auth_token.token().to_string();
    let before = client
        .get(format!("{base}/metrics/runtime"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["turn_sse"]["slow_consumers_total"]
        .as_u64()
        .unwrap();

    // Each request reads headers but intentionally retains the body, so all four
    // connections are simultaneously live slow consumers.
    let mut requests = tokio::task::JoinSet::new();
    for session_id in &session_ids {
        let client = client.clone();
        let base = base.clone();
        let token = token.clone();
        let session_id = session_id.clone();
        requests.spawn(async move {
            let response = client
                .post(format!("{base}/session/{session_id}/turn"))
                .bearer_auth(token)
                .json(&serde_json::json!({"prompt":"four concurrent slow clients"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            let turn_id = response
                .headers()
                .get("x-owo-turn-id")
                .expect("turn response must expose replay identity")
                .to_str()
                .unwrap()
                .to_string();
            (session_id, turn_id, response)
        });
    }
    let mut responses = Vec::with_capacity(CONNECTIONS);
    while let Some(result) = requests.join_next().await {
        responses.push(result.unwrap());
    }
    assert_eq!(responses.len(), CONNECTIONS);

    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let value = client
                .get(format!("{base}/metrics/runtime"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            let slow = value["turn_sse"]["slow_consumers_total"].as_u64().unwrap();
            if slow >= before + CONNECTIONS as u64 {
                break slow;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("四个并发慢读连接都应触发有界队列背压");
    assert!(emitted.load(Ordering::Relaxed) < CONNECTIONS * 1024);

    let replay_keys: Vec<(String, String)> = responses
        .iter()
        .map(|(session_id, turn_id, _)| (session_id.clone(), turn_id.clone()))
        .collect();
    drop(responses);
    for (session_id, turn_id) in replay_keys {
        let replay = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let page = client
                    .get(format!(
                        "{base}/session/{session_id}/turn/events?turn_id={turn_id}&after_seq=0&limit=512"
                    ))
                    .bearer_auth(&token)
                    .send()
                    .await
                    .unwrap()
                    .json::<serde_json::Value>()
                    .await
                    .unwrap();
                if page["state"] != "active" {
                    break page;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("每个并发慢读回合都应持久化终态");
        assert_eq!(replay["state"], "failed");
        assert!(replay["events"].as_array().unwrap().iter().any(|event| {
            event["payload"]["type"] == "token_delta"
                && event["payload"]["delta"]
                    .as_str()
                    .is_some_and(|delta| !delta.is_empty())
        }));
    }
    server.abort();
}

#[tokio::test]
async fn provider_disconnect_after_delta_is_persisted_as_failed_and_replayable() {
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let agent = Agent::new(
        Arc::new(DisconnectAfterDeltaProvider),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let session = state
        .store
        .create(&workspace, "disconnect-test", None)
        .unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), session.clone());

    let app = build_router(Arc::clone(&state));
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/turn", session.id),
            Some(r#"{"prompt":"exercise provider disconnect"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let turn_id = response
        .headers()
        .get("x-owo-turn-id")
        .expect("turn response must expose replay identity")
        .to_str()
        .unwrap()
        .to_string();
    let body = tokio::time::timeout(
        Duration::from_secs(5),
        axum::body::to_bytes(response.into_body(), 1024 * 1024),
    )
    .await
    .expect("provider failure must close the SSE stream")
    .unwrap();
    let sse = String::from_utf8_lossy(&body);
    assert!(
        sse.contains("event: token_delta"),
        "partial delta must reach SSE: {sse}"
    );
    assert!(
        sse.contains("partial-before-disconnect"),
        "the already-produced text must be preserved: {sse}"
    );
    assert!(
        sse.contains("event: turn_failed"),
        "provider failure must be surfaced before stream close: {sse}"
    );

    let replay = app
        .oneshot(request(
            &state,
            "GET",
            &format!(
                "/session/{}/turn/events?turn_id={turn_id}&after_seq=0&limit=20",
                session.id
            ),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(replay.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(replay.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let replay: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(replay["active"], false);
    assert_eq!(replay["state"], "failed");
    let events = replay["events"].as_array().expect("events page");
    assert!(events.iter().any(|event| {
        event["payload"]["type"] == "token_delta"
            && event["payload"]["delta"] == "partial-before-disconnect"
    }));
    assert!(events.iter().any(|event| {
        event["payload"]["type"] == "progress"
            && event["payload"]["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("turn failed:"))
            || event["payload"]["type"] == "turn_failed"
    }));
}

#[tokio::test]
async fn slow_http_consumer_and_provider_disconnect_preserve_failed_turn_replay() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let emitted = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicBool::new(false));
    let agent = Agent::new(
        Arc::new(BurstThenDisconnectProvider {
            emitted: Arc::clone(&emitted),
            completed: Arc::clone(&completed),
            delta_count: 160,
            delta_bytes: 8 * 1024,
        }),
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let session = state
        .store
        .create(&workspace, "combined-fault", None)
        .unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), session.clone());

    let app = build_router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let token = state.auth_token.token().to_string();
    let before = client
        .get(format!("{base}/metrics/runtime"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["turn_sse"]["slow_consumers_total"]
        .as_u64()
        .unwrap();

    // Read headers, then leave the SSE body untouched while the provider crosses the
    // queue byte bound and returns its own disconnect error.
    let response = client
        .post(format!("{base}/session/{}/turn", session.id))
        .bearer_auth(&token)
        .json(&serde_json::json!({"prompt":"combined slow-client and provider fault"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let turn_id = response
        .headers()
        .get("x-owo-turn-id")
        .expect("turn response must expose replay identity")
        .to_str()
        .unwrap()
        .to_string();

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let value = client
                .get(format!("{base}/metrics/runtime"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            if value["turn_sse"]["slow_consumers_total"]
                .as_u64()
                .is_some_and(|slow| slow > before)
                && completed.load(Ordering::SeqCst)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("慢消费者队列溢出和 Provider 断流均应实际发生");
    assert_eq!(emitted.load(Ordering::SeqCst), 160);

    drop(response);
    let replay = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = client
                .get(format!(
                    "{base}/session/{}/turn/events?turn_id={turn_id}&after_seq=0&limit=256",
                    session.id
                ))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            if page["state"] != "active" {
                break page;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("组合故障后 turn 应持久化终态");
    assert_eq!(replay["state"], "failed");
    let events = replay["events"]
        .as_array()
        .expect("persisted replay events");
    assert!(events.iter().any(|event| {
        event["payload"]["type"] == "token_delta"
            && event["payload"]["delta"]
                .as_str()
                .is_some_and(|delta| !delta.is_empty())
    }));
    assert!(events.iter().any(|event| {
        event["payload"]["type"] == "progress"
            && event["payload"]["message"]
                .as_str()
                .is_some_and(|message| message.starts_with("turn failed:"))
            || event["payload"]["type"] == "turn_failed"
    }));
    server.abort();
}

#[tokio::test]
async fn approved_write_diff_and_revert_close_the_server_golden_path() {
    use http_body_util::BodyExt;
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let policy = Policy::new(&workspace);
    policy.set_profile(owo_agent_core::permissions::PermissionProfile::AutoReview);
    let agent = Agent::new(
        Arc::new(ApprovedWriteThenFinalProvider {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }),
        ToolRegistry::new(),
        policy,
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let session = state.store.create(&workspace, "golden-path", None).unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), session.clone());

    let app = build_router(Arc::clone(&state));
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/turn", session.id),
            Some(r#"{"prompt":"write a file"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let mut body = response.into_body();
    let mut received_sse = String::new();
    let mut pending_sse = String::new();
    let mut permission_request_id = None;
    while permission_request_id.is_none() {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("write turn should request approval")
            .unwrap_or_else(|| {
                panic!("SSE closed before approval request; received so far: {received_sse}")
            })
            .unwrap();
        if let Ok(data) = frame.into_data() {
            let chunk = String::from_utf8_lossy(&data);
            received_sse.push_str(&chunk);
            pending_sse.push_str(&chunk);
            while let Some(end) = pending_sse.find("\n\n") {
                let event = pending_sse[..end].to_string();
                pending_sse.drain(..end + 2);
                for line in event.lines() {
                    let Some(data) = line.strip_prefix("data: ") else {
                        continue;
                    };
                    let Ok(payload) = serde_json::from_str::<serde_json::Value>(data) else {
                        continue;
                    };
                    if payload["type"] == "permission_request" {
                        permission_request_id = payload["request_id"].as_str().map(str::to_owned);
                        break;
                    }
                }
                if permission_request_id.is_some() {
                    break;
                }
            }
        }
    }
    let permission_request_id = permission_request_id.unwrap();
    let approval = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/permission/{permission_request_id}", session.id),
            Some(r#"{"allow":true,"scope":"once"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(approval.status(), axum::http::StatusCode::OK);

    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = body.frame().await {
            let frame = frame.expect("turn SSE must not fail after approval");
            if let Ok(data) = frame.into_data() {
                received_sse.push_str(&String::from_utf8_lossy(&data));
            }
        }
    })
    .await
    .expect("approved tool turn should complete");
    assert!(received_sse.contains("permission_request"));
    assert!(received_sse.contains("tool_use"));
    assert!(received_sse.contains("tool_result"));
    assert!(received_sse.contains("write complete"));
    assert_eq!(
        std::fs::read_to_string(workspace.join("approved-output.txt")).unwrap(),
        "approved content\n"
    );

    let diff = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/session/{}/diff", session.id),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(diff.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(diff.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let diff: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(diff.as_array().unwrap().iter().any(|entry| {
        entry["path"] == "approved-output.txt"
            && entry["after"] == "approved content\n"
            && entry["before"].is_null()
    }));

    let persisted = state.store.load(&session.id).unwrap();
    let receipt_id = persisted
        .execution_receipts
        .last()
        .map(|receipt| receipt.receipt_id.clone())
        .expect("approved write should persist an execution receipt");
    let receipt_body = format!(r#"{{"receipt_id":"{receipt_id}"}}"#);

    // 用户在 Agent 写入后继续编辑：撤销必须报告冲突且保留用户内容。
    std::fs::write(workspace.join("approved-output.txt"), "user's later edit\n").unwrap();
    let conflict = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/revert", session.id),
            Some(&receipt_body),
        ))
        .await
        .unwrap();
    assert_eq!(conflict.status(), axum::http::StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(conflict.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let conflict: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        conflict["error"]["code"],
        "storage/revert_conflict/not_retryable"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("approved-output.txt")).unwrap(),
        "user's later edit\n"
    );

    // 将文件恢复到 Agent 写入版本后，原操作可安全重试。
    std::fs::write(workspace.join("approved-output.txt"), "approved content\n").unwrap();
    let reverted = app
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/revert", session.id),
            Some(&receipt_body),
        ))
        .await
        .unwrap();
    assert_eq!(reverted.status(), axum::http::StatusCode::OK);
    assert!(!workspace.join("approved-output.txt").exists());
}

#[tokio::test]
async fn turn_replay_openapi_documents_identity_and_terminal_state() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(CONTRACT_SNAPSHOT).unwrap();
    assert_eq!(
        spec["paths"]["/session/{id}/turn"]["post"]["responses"]["200"]["headers"]["x-owo-turn-id"]
            ["schema"]["type"],
        "string"
    );
    assert_eq!(
        spec["paths"]["/session/{id}/turn/events"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/TurnEventReplayPage"
    );
    assert_eq!(
        spec["components"]["schemas"]["TurnEventReplayPage"]["properties"]["state"]["enum"],
        serde_json::json!(["active", "completed", "failed", "interrupted"])
    );
    assert_eq!(
        snapshot["paths"]["/session/{id}/turn"]["post"]["responses"]["200"]["headers"]
            ["x-owo-turn-id"],
        spec["paths"]["/session/{id}/turn"]["post"]["responses"]["200"]["headers"]["x-owo-turn-id"]
    );
    assert_eq!(
        snapshot["paths"]["/session/{id}/turn/events"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"],
        spec["paths"]["/session/{id}/turn/events"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]
    );
    assert_eq!(
        snapshot["components"]["schemas"]["TurnEventReplayPage"],
        spec["components"]["schemas"]["TurnEventReplayPage"]
    );
}

#[tokio::test]
async fn openapi_documents_chat_mode_and_optional_workspace() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let served: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // 纯对话可以不选工作区：文档不能继续把 workspace 标成必填。
    let create = &served["components"]["schemas"]["CreateSessionRequest"];
    assert_eq!(create["properties"]["workspace"]["type"], "string");
    assert!(
        create["properties"]["workspace"]["description"]
            .as_str()
            .unwrap()
            .contains("chat 模式可省略"),
        "workspace 必须说明纯对话可省略"
    );
    assert!(
        !create["required"]
            .as_array()
            .map(|fields| fields.iter().any(|field| field == "workspace"))
            .unwrap_or(false),
        "workspace 不应继续出现在 required"
    );
    assert_eq!(
        create["properties"]["mode"]["enum"],
        serde_json::json!(["agent", "chat"])
    );

    // 模式切换与详情也必须是可枚举的，客户端才能生成联合类型。
    assert_eq!(
        served["paths"]["/session/{id}/mode"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["properties"]["mode"]["enum"],
        serde_json::json!(["agent", "chat"])
    );
    for schema in ["SessionInfo", "SessionDetail"] {
        assert_eq!(
            served["components"]["schemas"][schema]["properties"]["mode"]["enum"],
            serde_json::json!(["agent", "chat"]),
            "{schema} 必须暴露 mode 枚举"
        );
    }
}

#[tokio::test]
async fn openapi_json_covers_snapshot_and_registered_routes() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "/openapi.json 应可访问");
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let served: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(CONTRACT_SNAPSHOT).unwrap();
    for field in [
        "model",
        "parallel",
        "max_agent_members",
        "acknowledge_risk",
        "allow_scoped_writes",
    ] {
        let served_property = &served["paths"]["/teams"]["post"]["requestBody"]["content"]
            ["application/json"]["schema"]["properties"][field];
        let snapshot_property = &snapshot["paths"]["/teams"]["post"]["requestBody"]["content"]
            ["application/json"]["schema"]["properties"][field];
        assert!(
            !served_property.is_null(),
            "POST /teams field {field} must be documented"
        );
        assert_eq!(
            served_property, snapshot_property,
            "POST /teams field {field} drifted"
        );
    }
    for schema in ["ProductEvalRun", "ProductEvalMetrics", "CaseModeMetrics"] {
        assert_eq!(
            served["components"]["schemas"][schema], snapshot["components"]["schemas"][schema],
            "ProductEval schema {schema} 与 TypeScript OpenAPI 快照漂移"
        );
    }
    let served_paths: Vec<&str> = served["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();

    // 1) 契约快照全部路径已登记
    let mut missing: Vec<String> = Vec::new();
    for (path, _) in contract_endpoints() {
        if !served_paths.contains(&path.as_str()) {
            missing.push(path.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "契约快照路径漏登记 /openapi.json：{missing:?}"
    );

    // 2) lib.rs 实际注册的路由全部已登记
    let mut missing_routes: Vec<String> = Vec::new();
    for route in registered_routes() {
        if !served_paths.contains(&route.as_str()) {
            missing_routes.push(route);
        }
    }
    assert!(
        missing_routes.is_empty(),
        "已注册路由漏登记 /openapi.json：{missing_routes:?}"
    );

    // 3) 全部源文件（含模块内 router）注册的路由全部已登记
    let mut missing_module_routes: Vec<String> = Vec::new();
    for route in all_source_routes() {
        if !served_paths.contains(&route.as_str()) {
            missing_module_routes.push(route);
        }
    }
    assert!(
        missing_module_routes.is_empty(),
        "模块路由漏登记 /openapi.json：{missing_module_routes:?}"
    );

    // 4) OpenAPI 快照与 served spec 路径集合双向一致（防快照过期/超前漂移）
    let endpoints = contract_endpoints();
    let snapshot_paths: Vec<&str> = endpoints.iter().map(|(path, _)| path.as_str()).collect();
    let mut snapshot_extra: Vec<String> = Vec::new();
    for path in &snapshot_paths {
        if !served_paths.contains(path) {
            snapshot_extra.push(path.to_string());
        }
    }
    assert!(
        snapshot_extra.is_empty(),
        "快照路径未在 served /openapi.json 中：{snapshot_extra:?}"
    );
    let mut served_extra: Vec<String> = Vec::new();
    for path in &served_paths {
        if !snapshot_paths.contains(path) {
            served_extra.push(path.to_string());
        }
    }
    assert!(
        served_extra.is_empty(),
        "served /openapi.json 存在快照未登记路径（快照需同步）：{served_extra:?}"
    );
}

/// R2 冻结契约（第三路实现、第一路接线同步）：
/// `POST /teams/{id}/steer` 的 `{"command":"retry","step_id":"…","note":"…"}`。
/// 本用例只锁 HTTP 契约形状，不依赖第三路业务实现细节：
/// - retry 缺失/空/空白 step_id → 400（校验先于团队存在性，无需已存在团队）；
/// - 合法 retry + 未知团队 → 404（路由在、资源不在，证明 retry 形状已被契约接受）；
/// - 未知 command → 400。
#[tokio::test]
async fn steer_retry_contract_shape_is_frozen() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    for body in [
        r#"{"command":"retry","note":"契约"}"#,
        r#"{"command":"retry","step_id":"","note":"契约"}"#,
        r#"{"command":"retry","step_id":"   ","note":"契约"}"#,
    ] {
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                "/teams/ct-retry-team/steer",
                Some(body),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            400,
            "retry 缺 step_id 必须 400：{body}"
        );
    }

    let unknown_team = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/teams/ct-retry-team/steer",
            Some(r#"{"command":"retry","step_id":"builder","note":"契约"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(
        unknown_team.status().as_u16(),
        404,
        "合法 retry + 未知团队应 404（路由在、资源不在）"
    );

    let unknown_command = app
        .oneshot(request(
            &state,
            "POST",
            "/teams/ct-retry-team/steer",
            Some(r#"{"command":"no-such-command"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(
        unknown_command.status().as_u16(),
        400,
        "未知 steer command 应 400"
    );
}

/// V1 三日冻结契约（第四路实现并接线，见 AGENTS-COORD.md 留言区「第四路（三期开工）」）：
/// ProductEval 四路由的校验形状——只锁 HTTP 契约，不依赖后台执行：
/// - 缺字段/类型错 → 422；未知 suite（含客户端路径）/execution/mode、repetitions 越界 → 400；
/// - 未知运行：GET /product-eval/runs/{id} 与 POST …/cancel → 404（路由在、资源不在）。
#[tokio::test]
async fn product_eval_contract_shapes_are_frozen() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 结构错误 → 422
    for (body, why) in [
        (
            r#"{"execution":"reference","modes":["single"],"repetitions":1}"#,
            "缺 suite",
        ),
        (
            r#"{"suite":"v1","modes":["single"],"repetitions":1}"#,
            "缺 execution",
        ),
        (
            r#"{"suite":"v1","execution":"reference","repetitions":1}"#,
            "缺 modes",
        ),
        (
            r#"{"suite":"v1","execution":"reference","modes":"single","repetitions":1}"#,
            "modes 非数组",
        ),
        (
            r#"{"suite":"v1","execution":"reference","modes":["single"],"repetitions":"1"}"#,
            "repetitions 非整数",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, "POST", "/product-eval/runs", Some(body)))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 422, "{why} 必须 422：{body}");
    }

    // 语义错误 → 400
    for (body, why) in [
        (
            r#"{"suite":"../evil","execution":"reference","modes":["single"],"repetitions":1}"#,
            "客户端路径拒绝",
        ),
        (
            r#"{"suite":"nope","execution":"reference","modes":["single"],"repetitions":1}"#,
            "未知 suite 名",
        ),
        (
            r#"{"suite":"v1","execution":"dry","modes":["single"],"repetitions":1}"#,
            "未知 execution",
        ),
        (
            r#"{"suite":"v1","execution":"reference","modes":["team"],"repetitions":1}"#,
            "未知 mode",
        ),
        (
            r#"{"suite":"v1","execution":"reference","modes":[],"repetitions":1}"#,
            "modes 空",
        ),
        (
            r#"{"suite":"v1","execution":"reference","modes":["single"],"repetitions":0}"#,
            "repetitions 0",
        ),
        (
            r#"{"suite":"v1","execution":"reference","modes":["single"],"repetitions":21}"#,
            "repetitions 21",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, "POST", "/product-eval/runs", Some(body)))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 400, "{why} 必须 400：{body}");
    }

    // 未知运行 → 404（GET 详情 + POST 取消）
    let detail = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            "/product-eval/runs/eval-ct-unknown",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(detail.status().as_u16(), 404, "未知运行详情应 404");
    let cancel = app
        .oneshot(request(
            &state,
            "POST",
            "/product-eval/runs/eval-ct-unknown/cancel",
            Some("{}"),
        ))
        .await
        .unwrap();
    assert_eq!(cancel.status().as_u16(), 404, "未知运行取消应 404");
}

#[tokio::test]
async fn v05_routes_are_registered_not_404_via_real_http() {
    // §3.2：包含会发布领域失效的 mutation；与矩阵测试串行。
    let _guard = invalidate_guard().await;
    let (state, _temp) = test_state().await;
    let token = state.auth_token.token().to_string();
    let app = build_router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let base = format!("http://{addr}");

    let client = reqwest::Client::new();
    let get_cases = [
        "/health",
        "/usage",
        "/skills/health",
        "/plugins",
        "/project/rules",
        "/mcp",
        "/mcp/health",
        "/capabilities",
        "/traces",
        "/memory/observations",
        "/memory/recall?q=t&top_k=3",
        "/symbols?query=main",
        "/workflows/user",
        "/workflows/draft",
        "/workflows/provenance",
        "/workflows/stats",
        "/workflows/user/contract-test/last-run",
        "/openapi.json",
        "/auth/token",
    ];
    for path in get_cases {
        let status = client
            .get(format!("{base}{path}"))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16();
        assert!(
            status != 404 && status != 405,
            "GET {path} → {status}（路由缺失）"
        );
    }

    let post_cases: &[(&str, &str)] = &[
        ("/subagent/run", r#"{"prompt":"hi","read_only":true}"#),
        ("/locate/query", "{}"),
        (
            "/memory/mine-skill",
            r#"{"name":"t","target_apps":[],"sensitivity":"low","description":"d"}"#,
        ),
        ("/memory/clear", "{}"),
        (
            "/computer-use/task",
            r#"{"target_app":"notepad","description":"d","allowed_actions":[],"max_duration_ms":60000}"#,
        ),
        ("/project/rules", r#"{"content":"project-rules"}"#),
    ];
    for (path, body) in post_cases {
        let status = client
            .post(format!("{base}{path}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap()
            .status()
            .as_u16();
        assert!(
            status != 404 && status != 405,
            "POST {path} → {status}（路由缺失）"
        );
    }

    handle.abort();
}

// ---------- R7 X03：本地 API 安全边界契约 ----------

/// 无 token 的 API 请求一律 401；公开端点与 SSE 资源型路径豁免。
#[tokio::test]
async fn unauthorized_requests_get_401() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    for (method, path, body) in [
        ("GET", "/usage", None),
        ("GET", "/sessions", None),
        ("GET", "/session/x/diff", None),
        ("POST", "/goal", Some(r#"{"objective":"t"}"#)),
        ("GET", "/audit", None),
        // §3.4 引导页「测试连接」：它会回显掩码端点与连通结论，属受保护面——
        // 无 token 必须 401，否则任何本机进程都能拿它探测（并可能探测内网）。
        ("POST", "/settings/provider-test", Some("{}")),
        // 提供商预设表同属受保护面：无 token 必须 401。
        ("GET", "/providers/presets", None),
    ] {
        let response = app
            .clone()
            .oneshot(anonymous_request(method, path, body))
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            401,
            "{method} {path} 无 token 应 401"
        );
    }
}

/// 公开端点：health/openapi/token 引导无 token 可访问；错误 token 401。
#[tokio::test]
async fn public_endpoints_are_token_free_and_bad_token_rejected() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    for path in ["/health", "/openapi.json", "/auth/token"] {
        let response = app
            .clone()
            .oneshot(anonymous_request("GET", path, None))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200, "GET {path} 应公开可达");
    }
    // 引导端点返回真实 token。
    let response = app
        .clone()
        .oneshot(anonymous_request("GET", "/auth/token", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let served: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        served["token"].as_str().unwrap(),
        state.auth_token.token(),
        "引导端点应返回同一 token"
    );

    // 错误 token → 401。
    use axum::http::{header, Method, Request};
    let bad = Request::builder()
        .method(Method::GET)
        .uri("/sessions")
        .header(header::AUTHORIZATION, "Bearer wrong-token")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(bad).await.unwrap();
    assert_eq!(response.status().as_u16(), 401);
}

/// `/auth/token` 的发布桌面进程配对必须在 OpenAPI 与 CORS 预检中可用，
/// 否则 WebView 会在真正发送请求前拦截带自定义证明头的请求。
#[tokio::test]
async fn auth_token_desktop_pairing_is_documented_and_cors_allowed() {
    use axum::http::{header, Method, Request};

    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        spec["paths"]["/auth/token"]["get"]["responses"]["403"].is_object(),
        "/auth/token 的 OpenAPI 应登记发布桌面配对失败的 403 响应"
    );

    let preflight = Request::builder()
        .method(Method::OPTIONS)
        .uri("/auth/token")
        .header(header::ORIGIN, "tauri://localhost")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            "x-owo-desktop-pairing",
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(preflight).await.unwrap();
    assert!(response.status().is_success(), "配对头预检应成功");
    let allowed_headers = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        allowed_headers.contains("x-owo-desktop-pairing"),
        "CORS 必须放行桌面配对证明头，实际为 {allowed_headers:?}"
    );
}

/// 桌面 WebView 必须能读取回合响应头中的恢复 ID；仅由 CORS 放行请求仍不够。
#[tokio::test]
async fn cors_exposes_turn_id_for_stream_recovery() {
    use axum::http::{header, Method, Request};

    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header(header::ORIGIN, "tauri://localhost")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let exposed = response
        .headers()
        .get(header::ACCESS_CONTROL_EXPOSE_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        exposed.contains("x-owo-turn-id"),
        "CORS must expose x-owo-turn-id to the desktop webview; got {exposed:?}"
    );
}

/// §3.1：事件流不再匿名——`/events/stream` 与全部资源型事件流均要求 Bearer。
/// 匿名 → 401（正常 JSON 错误体）；错误 token → 401；有效 token → 进入处理器
/// （/events/stream 直接 200 SSE；资源型路径对不存在 id 为 404，但绝不是 401）。
#[tokio::test]
async fn event_streams_require_bearer_auth() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let sse_paths = [
        "/events/stream",
        "/cloud/tasks/x/events",
        "/workflow/run/x/events",
        "/teams/x/events",
        "/fleet/tasks/x/events",
    ];
    for path in sse_paths {
        let response = app
            .clone()
            .oneshot(anonymous_request("GET", path, None))
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            401,
            "GET {path} 匿名应 401（事件流不再豁免）"
        );
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            parsed["code"], "auth/unauthorized/not_retryable",
            "鉴权失败必须返回正常 JSON 错误（SSE 建立前拦截）"
        );
    }
    for path in sse_paths {
        let bad = axum::http::Request::builder()
            .method(axum::http::Method::GET)
            .uri(path)
            .header(axum::http::header::AUTHORIZATION, "Bearer wrong-token")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.clone().oneshot(bad).await.unwrap();
        assert_eq!(
            response.status().as_u16(),
            401,
            "GET {path} 错误 token 应 401"
        );
    }
    for path in sse_paths {
        let response = app
            .clone()
            .oneshot(request(&state, "GET", path, None))
            .await
            .unwrap();
        let status = response.status().as_u16();
        assert!(
            status == 200 || status == 404,
            "GET {path} 有效 token 不应 401，实际 {status}"
        );
    }
    let response = app
        .oneshot(request(&state, "GET", "/events/stream", None))
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "/events/stream 有效 token 应 200"
    );
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.contains("text/event-stream"),
        "应为 SSE 响应，实际 {content_type}"
    );
}

/// §3.2：领域失效发布测试串行锁——hub 单例 + 域版本是进程级共享状态，
/// 并行 mutation 测试会把“恰好一次”断言污染成不稳定结果。
static INVALIDATE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 带上限的发布锁获取：若持有者被某个卡死的请求拖住，等待方在 90s 后
/// 快速失败（带诊断信息），而不是让整个测试二进制静默挂起。
async fn invalidate_guard() -> tokio::sync::MutexGuard<'static, ()> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(90),
        INVALIDATE_TEST_LOCK.lock(),
    )
    .await
    {
        Ok(guard) => guard,
        Err(_) => {
            panic!("INVALIDATE_TEST_LOCK 90s 未获得：另一领域失效测试疑似卡死（快速失败诊断）")
        }
    }
}

/// §3.1 续传成功：`Last-Event-ID` 请求头指定续传起点，服务端从下一 seq 重放。
#[tokio::test]
async fn events_stream_resumes_from_last_event_id_header() {
    use http_body_util::BodyExt;

    let _guard = invalidate_guard().await;
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let hub = owo_agent_server::event_stream::hub();
    let before = hub.last_seq();
    let seq = hub.publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Projects);
    assert!(seq > before, "发布应分配新 seq");
    let req = axum::http::Request::builder()
        .method(axum::http::Method::GET)
        .uri("/events/stream")
        .header(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        )
        .header("Last-Event-ID", before.to_string())
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
        .await
        .expect("首帧应立即重放（5s 超时）")
        .expect("流不应报错")
        .unwrap();
    let bytes = frame.into_data().expect("首帧应为 data 帧");
    let text = String::from_utf8_lossy(&bytes);
    // §3.1 契约（R0 收口）：断言必须按真实帧格式解析两层 JSON——
    // SSE data 行是外层 StreamEvent JSON，其 data 字段是内层失效载荷的
    // JSON 字符串。直接搜未转义的 domain 会把"帧格式变更"误判为"续传失败"。
    let data_line = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap_or_else(|| panic!("SSE 帧缺少 data 行，实际：{text}"));
    let outer: serde_json::Value = serde_json::from_str(data_line)
        .unwrap_or_else(|e| panic!("外层 StreamEvent JSON 解析失败（{e}），实际：{data_line}"));
    assert_eq!(outer["seq"].as_u64(), Some(seq), "续传首帧 seq 应为发布点");
    assert_eq!(
        outer["kind"].as_str(),
        Some("invalidate"),
        "首帧应为失效事件"
    );
    let inner_str = outer["data"]
        .as_str()
        .unwrap_or_else(|| panic!("外层 data 字段应为 JSON 字符串，实际：{outer}"));
    let payload: serde_json::Value = serde_json::from_str(inner_str)
        .unwrap_or_else(|e| panic!("内层失效载荷 JSON 解析失败（{e}），实际：{inner_str}"));
    assert_eq!(
        payload["domain"].as_str(),
        Some("projects"),
        "失效领域应为 projects，实际：{payload}"
    );
    // id 行与外层 seq 一致（Last-Event-ID 续传游标同源）。
    let id_line = text
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .and_then(|v| v.parse::<u64>().ok());
    assert_eq!(id_line, Some(seq), "SSE id 行必须等于外层 seq");
}

/// R3（§8.2）：新订阅（无 `Last-Event-ID`、无 `?last_event_id=`）**不得重放历史**。
/// 真实桌面冷启动的首屏请求风暴根因即旧语义「缺省从头重放」：WebView 先水合、
/// 随后连事件流，整环 invalidate 历史一次性下发 → 每个领域各刷一次
/// （实测首屏业务请求 19 条，超 §8.2 的 ≤5 预算）。断线续传语义不受影响
/// （见 `events_stream_resumes_from_last_event_id_header`）。
#[tokio::test]
async fn events_stream_fresh_subscription_does_not_replay_history() {
    use http_body_util::BodyExt;
    use owo_agent_server::event_stream::{hub, InvalidateDomain};

    let _guard = invalidate_guard().await;
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    // 连接前已存在的历史事件：新订阅必须看不到它。
    let stale = hub().publish_invalidate(InvalidateDomain::Projects);
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/events/stream", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    // 连接之后再发布：这才是新订阅应收到的事件。
    let live = hub().publish_invalidate(InvalidateDomain::Skills);
    assert!(live > stale, "seq 单调");
    let mut body = response.into_body();
    let mut seen: Vec<u64> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        let budget = deadline.saturating_duration_since(std::time::Instant::now());
        if budget.is_zero() {
            break;
        }
        // `timeout(..).await` 的层序是 Result<**Option**<Result<Frame, Error>>, Elapsed>
        // （帧迭代器先给"流是否还有下一帧"，再给"该帧是否出错"）。
        let frame = match tokio::time::timeout(budget, body.frame()).await {
            Ok(Some(Ok(frame))) => frame,
            _ => break, // 超时 / 流结束 / 帧错误：都停止收集，由下方断言判定
        };
        // `Frame::into_data()` 返回 Result（心跳注释帧不是 data 帧 → 跳过）。
        let Ok(data) = frame.into_data() else {
            continue;
        };
        let text = String::from_utf8_lossy(&data).to_string();
        let Some(line) = text.lines().find_map(|l| l.strip_prefix("data: ")) else {
            continue; // 心跳注释帧无 data 行
        };
        let Ok(outer) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(seq) = outer["seq"].as_u64() {
            seen.push(seq);
            if seq == live {
                break;
            }
        }
    }
    assert!(
        seen.contains(&live),
        "新订阅必须收到连接后的实时事件（已收 {seen:?}）"
    );
    assert!(
        !seen.contains(&stale),
        "新订阅不得重放连接前的历史事件 seq={stale}（首屏请求风暴根因），实际收到 {seen:?}"
    );
}

/// R3（§8.2）配套：`?last_event_id=0` 是脚本/调试用的**显式**全量重放开关，
/// 缺省语义变更不得把这个逃生口一起带走。
#[tokio::test]
async fn events_stream_explicit_zero_replays_history() {
    use http_body_util::BodyExt;
    use owo_agent_server::event_stream::{hub, InvalidateDomain};

    let _guard = invalidate_guard().await;
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let stale = hub().publish_invalidate(InvalidateDomain::Whitelist);
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            "/events/stream?last_event_id=0",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
        .await
        .expect("显式 last_event_id=0 应立即重放历史")
        .expect("流不应报错")
        .unwrap();
    let first_frame = frame.into_data().expect("显式重放的首帧应为 data 帧");
    let text = String::from_utf8_lossy(&first_frame);
    let data_line = text
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap_or_else(|| panic!("SSE 帧缺少 data 行，实际：{text}"));
    let outer: serde_json::Value = serde_json::from_str(data_line).expect("外层 JSON");
    assert!(
        outer["seq"].as_u64().unwrap_or(u64::MAX) <= stale,
        "显式全量重放的首帧应是历史事件（≤{stale}），实际：{outer}"
    );
}

/// §3.2 路由-事件矩阵：每个改变领域状态的 mutation 在成功后恰好发布一次
/// 对应领域失效；失败路径（4xx/5xx）不得发布。新增 mutation 路由时必须
/// 在本表登记 (method, path, body, domain)，防止漏接领域事件。
#[tokio::test]
async fn mutation_routes_publish_exactly_one_domain_invalidate() {
    use owo_agent_server::event_stream::{hub, InvalidateDomain};

    let _guard = invalidate_guard().await;
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // settings 回读体：全字段合法，避免手写缺字段导致 400。
    // 前置同样限时：任何路由卡死都在此处显式失败，而非把矩阵拖成挂起。
    let settings_response = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        app.clone()
            .oneshot(request(&state, "GET", "/settings", None)),
    )
    .await
    {
        Ok(Ok(response)) => response,
        other => panic!("GET /settings 契约前置异常（15s 超时或错误）：{other:?}"),
    };
    assert_eq!(
        settings_response.status().as_u16(),
        200,
        "GET /settings 契约前置失败"
    );
    let settings_bytes = axum::body::to_bytes(settings_response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let settings_json: serde_json::Value = serde_json::from_slice(&settings_bytes).unwrap();
    assert_eq!(
        settings_json["tool_capabilities"],
        serde_json::json!({
            "desktop_observation": false,
            "desktop_control": false,
            "browser": false,
        }),
        "旧/默认工作区不得隐式启用可选工具能力"
    );
    let active_tools = settings_json["runtime"]["active_tool_names"]
        .as_array()
        .expect("runtime 必须报告当前真实工具面");
    for required in ["read_file", "write_file", "run_command", "use_skill"] {
        assert!(
            active_tools.iter().any(|name| name == required),
            "最小工具面缺少基础工具 {required}"
        );
    }
    for optional in ["screen_ocr", "desktop_click", "browser_navigate"] {
        assert!(
            !active_tools.iter().any(|name| name == optional),
            "可选工具 {optional} 不应在默认 Agent 暴露"
        );
    }

    let matrix: Vec<(&str, &str, serde_json::Value, InvalidateDomain)> = vec![
        (
            "POST",
            "/automations",
            serde_json::json!({"name":"ct-inv","schedule":{"kind":"daily","time":"09:00"},"reminder":"hi"}),
            InvalidateDomain::Automations,
        ),
        (
            "POST",
            "/settings",
            settings_json,
            InvalidateDomain::Settings,
        ),
        (
            // §4.5.3 提交结构化权限配置会写 settings.json（档位/只读位/spec 三处），
            // 因此它属于 Settings 领域失效；不另发一个 Permissions 领域，
            // 那需要前端 INVALIDATE_HANDLERS 同步消费（尚未接入，先不制造无人消费的域）。
            "POST",
            "/permissions/spec",
            serde_json::json!({"spec":{"filesystem":"workspace_read","command":"allowlisted","network":"deny","persistence":"once","scopes":[]}}),
            InvalidateDomain::Settings,
        ),
        (
            "POST",
            "/whitelist/manage",
            serde_json::json!({"action":"list"}),
            InvalidateDomain::Whitelist,
        ),
        (
            "POST",
            "/mcp/add",
            serde_json::json!({"name":"__ct_invalid__","transport":"http","url":"http://127.0.0.1:1"}),
            InvalidateDomain::Mcp,
        ),
        (
            "POST",
            "/learn/clear",
            serde_json::json!({}),
            InvalidateDomain::Learn,
        ),
        // 任务 2 尾·全量复核：learn 状态变更族补发布（成功恰一次 / 失败零发布）。
        // learn/record 行缺 app_id/anchor/action_type/at → 422 → 断言失败路径零发布。
        (
            "POST",
            "/learn/start",
            serde_json::json!({}),
            InvalidateDomain::Learn,
        ),
        (
            "POST",
            "/learn/record",
            serde_json::json!({"action":"click"}),
            InvalidateDomain::Learn,
        ),
        (
            "POST",
            "/learn/stop",
            serde_json::json!({}),
            InvalidateDomain::Learn,
        ),
        (
            "POST",
            "/learn/sink",
            serde_json::json!({"name":"ct-inv-pkg","target_apps":[],"sensitivity":"low","description":"契约矩阵"}),
            InvalidateDomain::Packages,
        ),
        (
            "POST",
            "/plugins/__ct_missing__/enabled",
            serde_json::json!({"enabled":false}),
            InvalidateDomain::Plugins,
        ),
        (
            "POST",
            "/project/rules",
            serde_json::json!({"content":"契约矩阵项目规则"}),
            InvalidateDomain::Projects,
        ),
        (
            "POST",
            "/computer-use/task",
            serde_json::json!({"target_app":"notepad","description":"契约矩阵","allowed_actions":[],"max_duration_ms":60000}),
            InvalidateDomain::Computer,
        ),
        // 任务 2：mutation 失效发布补全（成功恰一次 / 失败零发布）。
        (
            "POST",
            "/memory/clear",
            serde_json::json!({}),
            InvalidateDomain::Memory,
        ),
        (
            "POST",
            "/memory/mine-skill",
            // 空记忆 → 400：断言失败路径零发布。
            serde_json::json!({"name":"ct-inv-skill","target_apps":[],"sensitivity":"low","description":"d"}),
            InvalidateDomain::Memory,
        ),
        (
            "POST",
            "/skills/__ct_missing__/enabled",
            serde_json::json!({"enabled":false}),
            InvalidateDomain::Skills,
        ),
        (
            "POST",
            "/usage/topup",
            serde_json::json!({"amount":0.0}),
            InvalidateDomain::Usage,
        ),
        (
            "POST",
            "/session",
            serde_json::json!({"workspace":".","model":"idle"}),
            InvalidateDomain::Sessions,
        ),
        (
            "POST",
            "/session/__ct_missing__/fork",
            serde_json::json!({}),
            InvalidateDomain::Sessions,
        ),
        // M4.2：会话级模型路由 mutation（404 失败路径 → 零发布）。
        (
            "POST",
            "/session/__ct_missing__/model",
            serde_json::json!({"model":null}),
            InvalidateDomain::Sessions,
        ),
        (
            "POST",
            "/mcp/enabled",
            serde_json::json!({"name":"__ct_missing__","enabled":false}),
            InvalidateDomain::Mcp,
        ),
        (
            "POST",
            "/mcp/reconnect",
            serde_json::json!({"name":"__ct_missing__"}),
            InvalidateDomain::Mcp,
        ),
    ];

    let mut failures: Vec<String> = Vec::new();
    for (method, path, body, domain) in matrix {
        let before = hub().domain_version(domain.as_str());
        let body_text = serde_json::to_string(&body).unwrap();
        // 行级 15s 上限：单行卡死只计为该行失败；最坏 9×15s 全套仍有界。
        let response = match tokio::time::timeout(
            std::time::Duration::from_secs(15),
            app.clone()
                .oneshot(request(&state, method, path, Some(&body_text))),
        )
        .await
        {
            Ok(Ok(response)) => response,
            other => {
                failures.push(format!("{method} {path} 请求异常：{other:?}"));
                continue;
            }
        };
        let status = response.status().as_u16();
        let after = hub().domain_version(domain.as_str());
        if status == 200 {
            if after - before != 1 {
                failures.push(format!(
                    "{method} {path} 成功应恰好发布一次 {domain:?} 失效（delta={}）",
                    after - before
                ));
            }
        } else if after != before {
            failures.push(format!(
                "{method} {path} 失败路径（{status}）不得发布 {domain:?} 失效"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "路由-事件矩阵失败：\n{}",
        failures.join("\n")
    );
}

/// §6.1.3：/health 必须携带完整构建身份——commit/dirty/built_at 三项语义。
/// 编译期身份来自 owo-build-info 的 build.rs（§6.1.2 单一实现）；测试机构建
/// 必能解析 git，故 commit 非 "unknown"、built_at 非空且为 RFC3339。
#[tokio::test]
async fn health_build_identity_has_full_semantics() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/health", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let health: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let build = &health["build"];
    assert!(
        build.is_object(),
        "/health 必须携带 build 身份对象：{health}"
    );
    let commit = build["commit"].as_str().unwrap_or_default();
    assert!(!commit.is_empty(), "commit 必须非空（§6.1.3）");
    assert_ne!(
        commit, "unknown",
        "编译期身份来自 owo-build-info 的 build.rs，测试机构建必能解析 git（§6.1.3）"
    );
    assert!(
        matches!(build["dirty"], serde_json::Value::Bool(_)),
        "dirty 必须是布尔（§6.1.3）"
    );
    let built_at = build["built_at"].as_str().unwrap_or_default();
    assert!(
        !built_at.is_empty(),
        "built_at 必须非空——epoch 为编译期烧录，不得再读运行时变量（§6.1.3）"
    );
    chrono::DateTime::parse_from_rfc3339(built_at)
        .unwrap_or_else(|error| panic!("built_at 必须是 RFC3339（实际 {built_at}）：{error}"));
}

/// §7.1 单一事实源：/health 的 api_version 与 build 三元组必须逐字段等于
/// `owo_build_info::identity()`（server 不得再有独立解析链/独立常量）。
#[tokio::test]
async fn health_identity_is_single_source_with_build_info() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "GET", "/health", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let health: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let identity = owo_build_info::identity();
    assert_eq!(
        health["api_version"].as_str(),
        Some(identity.api_version),
        "api_version 必须与 owo-build-info 同源"
    );
    assert_eq!(
        health["build"]["commit"].as_str(),
        Some(identity.commit.as_str())
    );
    assert_eq!(health["build"]["dirty"], serde_json::json!(identity.dirty));
    assert_eq!(
        health["build"]["built_at"].as_str(),
        Some(identity.built_at.as_str())
    );
    // build_id（壳握手字段）与 build.commit 同源，不得出现第二种解析。
    assert_eq!(health["build_id"].as_str(), Some(identity.commit.as_str()));
}

/// CORS：不允许的跨源请求无 Access-Control-Allow-Origin（浏览器侧拒绝）；
/// webview/localhost 白名单放行并回显 ACAO。
#[tokio::test]
async fn cors_whitelist_enforces_origins() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    fn preflight(origin: &str) -> axum::http::Request<axum::body::Body> {
        use axum::http::{header, Method, Request};
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/session")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn acao(response: &axum::http::Response<axum::body::Body>) -> String {
        response
            .headers()
            .get(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }

    // 恶意跨源 → 预检不回显 ACAO（浏览器拒绝跨源读取）。
    for evil in [
        "https://evil.example",
        "https://attacker.com",
        "http://192.168.0.1:8080",
    ] {
        let response = app.clone().oneshot(preflight(evil)).await.unwrap();
        assert_eq!(
            acao(&response),
            "",
            "Origin {evil} 预检不应回显 Access-Control-Allow-Origin"
        );
    }

    // webview / localhost 白名单 → 预检放行且回显 ACAO。
    for allowed in [
        "tauri://localhost",
        "http://tauri.localhost",
        "http://localhost:1420",
        "http://127.0.0.1:4096",
    ] {
        let response = app.clone().oneshot(preflight(allowed)).await.unwrap();
        assert_eq!(
            response.status().as_u16(),
            200,
            "Origin {allowed} 预检应放行"
        );
        assert!(
            !acao(&response).is_empty(),
            "Origin {allowed} 应回显 Access-Control-Allow-Origin"
        );
    }

    // 真实请求（非预检）同样不回显 ACAO 给恶意 origin。
    use axum::http::{header, Method, Request};
    let actual = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .header(header::ORIGIN, "https://evil.example")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(actual).await.unwrap();
    assert_eq!(acao(&response), "", "恶意 origin 的真实请求不应回显 ACAO");
}

/// 限流：超全局 RPM 后 429 + Retry-After + 审计记录。
#[tokio::test]
async fn rate_limit_returns_429_with_retry_after() {
    // 环境变量进程级共享：与 test_state 共用 STATE_ENV_LOCK 串行化，
    // 防止 set_var("...","5") 泄漏进并行用例正在构建的 AppState（ tokio Mutex 跨 await 安全）。
    let _guard = STATE_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    std::env::set_var("OWO_API_RPM_GLOBAL", "5");
    let (state, _temp) = build_state_inner().await;
    std::env::remove_var("OWO_API_RPM_GLOBAL");
    let app = build_router(Arc::clone(&state));

    let mut statuses: Vec<u16> = Vec::new();
    for _ in 0..10 {
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                "/goal",
                Some(r#"{"objective":"rl"}"#),
            ))
            .await
            .unwrap();
        statuses.push(response.status().as_u16());
    }
    let ok_count = statuses.iter().filter(|s| **s == 201).count();
    let limited = statuses.iter().filter(|s| **s == 429).count();
    assert!((1..=5).contains(&ok_count), "前 5 个应放行：{statuses:?}");
    assert!(limited >= 1, "超限应有 429：{statuses:?}");

    // Retry-After 头存在。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/goal",
            Some(r#"{"objective":"rl"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 429);
    assert!(
        response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .is_some(),
        "429 应携带 Retry-After"
    );

    // 限流拒绝已写审计。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/audit?limit=50", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["event"].as_str() == Some("rate_limited")),
        "限流拒绝应产生审计记录（event=rate_limited）"
    );
}

// ---------- R12 契约治理：/schemas/* 版本化发布 + Deprecation 头机制 ----------

/// /schemas/* 语义契约：列表含三份 schema 与 api_version；各版本 JSON Schema 可解析且 draft-07；
/// 未知 kind/version 404；api_version 与 OpenAPI x-owo-api-version 一致。
#[tokio::test]
async fn schemas_endpoints_serve_versioned_json_schema() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 列表：200 + 三份 schema + api_version。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/schemas", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "GET /schemas 应 200");
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        list["api_version"].as_str(),
        Some(owo_agent_server::OWO_API_VERSION),
        "/schemas api_version 应等于 OWO_API_VERSION"
    );
    let kinds = list["schemas"].as_object().unwrap();
    for kind in ["plugin-manifest", "owskill", "owflow"] {
        assert!(kinds.contains_key(kind), "/schemas 应登记 {kind}");
    }

    // 每份 schema：200 + draft-07 + 合法 JSON object。
    for kind in ["plugin-manifest", "owskill", "owflow"] {
        let uri = format!("/schemas/{kind}/v1");
        let response = app
            .clone()
            .oneshot(request(&state, "GET", &uri, None))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200, "GET {uri} 应 200");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let schema: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            schema["$schema"].as_str(),
            Some("http://json-schema.org/draft-07/schema#"),
            "{uri} 应为 draft-07"
        );
        assert_eq!(schema["type"].as_str(), Some("object"), "{uri} 应为 object");
    }

    // 未知 kind/version → 404（路由存在，资源缺失语义）。
    for uri in ["/schemas/bogus/v1", "/schemas/owflow/v9"] {
        let response = app
            .clone()
            .oneshot(request(&state, "GET", uri, None))
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            404,
            "GET {uri} 应 404（未知 schema）"
        );
    }

    // api_version 与 OpenAPI x-owo-api-version 一致。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let openapi: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        openapi["x-owo-api-version"].as_str(),
        Some(owo_agent_server::OWO_API_VERSION),
        "OpenAPI x-owo-api-version 应等于 OWO_API_VERSION"
    );
}

/// Deprecation 头机制纯函数契约：命中生成正确头值、未命中 None（无需真实弃用路由即可覆盖生成逻辑）。
#[test]
fn deprecation_header_value_contract() {
    let routes: &[(&str, &str, &str, &str)] = &[("/old", "0.5", "0.7", "/new")];

    // 命中（前缀匹配）：头值格式 = "{route}: since {since}, until {until} (use {alternative})"。
    assert_eq!(
        owo_agent_server::deprecation_header_value_for(routes, "/old/session").as_deref(),
        Some("/old: since 0.5, until 0.7 (use /new)")
    );
    // 未命中（前缀不匹配）：None。
    assert!(owo_agent_server::deprecation_header_value_for(routes, "/new/session").is_none());
    assert!(
        owo_agent_server::deprecation_header_value_for(routes, "/ol").is_none(),
        "前缀边界：/ol 不应命中 /old"
    );
}

/// HTTP 负例：当前 DEPRECATED_ROUTES 为空，正常路由不附加 Deprecation 头（中间件已接线且无误报）。
#[tokio::test]
async fn deprecation_header_absent_on_current_routes() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    for (method, path) in [
        ("GET", "/health"),
        ("GET", "/usage"),
        ("GET", "/sessions"),
        ("GET", "/schemas"),
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, method, path, None))
            .await
            .unwrap();
        assert!(
            response.headers().get("Deprecation").is_none(),
            "{method} {path} 不应携带 Deprecation 头（当前无弃用路由）"
        );
    }
}

/// V1 四期（第三路）：Artifact 评审闭环契约冻结——路由形状 + 错误语义面
/// （404 资源缺失 / 400 未知 decision / 鉴权外的路由可达性）。
#[tokio::test]
async fn artifact_review_contract_shape_is_frozen() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 1) POST /artifacts/{id}/review：未知产物 → 404 + 结构化错误（路由已注册，非 405）。
    let body = r#"{
        "team_id": "team-nope",
        "decision": "approve",
        "reviewer": "critic",
        "comment": "契约探针",
        "expected_version": 1,
        "idempotency_key": "contract-probe-approve"
    }"#;
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/artifacts/contract-probe:planner:v1/review",
            Some(body),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        404,
        "未知产物应 404（路由已注册）"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // handler 先查团队再查产物：探针两者都不存在，命中团队缺失分支（产物缺失分支由 API 集成测试覆盖）。
    assert!(
        value["error"].as_str().unwrap_or("").contains("团队不存在"),
        "404 错误应结构化：{value}"
    );

    // 2) 未知 decision → 400（handler 先校验决定枚举，再查团队/产物）。
    let bad_decision = r#"{
        "team_id": "team-nope",
        "decision": "maybe",
        "reviewer": "critic",
        "idempotency_key": "contract-probe-bad-decision"
    }"#;
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/artifacts/contract-probe:planner:v1/review",
            Some(bad_decision),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400, "未知 decision 应 400");

    // 3) GET /artifacts/{id}/history：未知产物 → 404 + 结构化错误。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            "/artifacts/contract-probe:planner:v1/history",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        value["error"].as_str().is_some(),
        "404 应带 error 字段：{value}"
    );

    // 4) /openapi.json 登记评审路由与 operationId（契约面自描述）。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        spec["paths"]["/artifacts/{id}/review"]["post"]["operationId"],
        serde_json::json!("artifactSubmitReview")
    );
    assert_eq!(
        spec["paths"]["/artifacts/{id}/history"]["get"]["operationId"],
        serde_json::json!("artifactReviewHistory")
    );
    assert!(
        spec["components"]["schemas"]["ArtifactReviewRecord"].is_object(),
        "ArtifactReviewRecord 组件应登记"
    );
}

/// 十期一路：/health 契约——healthy/version/api_version/auto_approve 必备；build 为 additive
/// 可缺省字段（build-info.json 缺失时不序列化），在场时 commit/dirty/built_at 齐备；
/// /openapi.json 同步登记 HealthResponse/BuildInfo 组件与 200 schema 引用。
#[tokio::test]
async fn health_contract_version_and_optional_build() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // ① /health 响应面。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/health", None))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["healthy"], serde_json::json!(true));
    assert_eq!(
        v["api_version"],
        serde_json::json!(owo_agent_server::OWO_API_VERSION)
    );
    assert!(v["version"].is_string(), "version 必须是字符串");
    assert!(
        !v["version"].as_str().unwrap().is_empty(),
        "version 不得为空"
    );
    assert!(v["auto_approve"].is_boolean());
    // additive：build 可缺省（无 build-info.json 时不序列化）；在场则三字段齐备。
    if let Some(build) = v.get("build") {
        assert!(build.is_object(), "build 在场必须是对象");
        assert!(build["commit"].is_string(), "build.commit 必须是字符串");
        assert!(build["dirty"].is_boolean(), "build.dirty 必须是布尔");
        assert!(build["built_at"].is_string(), "build.built_at 必须是字符串");
    }

    // §4.2 实例握手字段（additive）：测试进程未注入实例身份 → instance_id 不序列化；
    // pid/stage/build_id 恒在场（serde default 字段）。
    assert!(
        v.get("instance_id").is_none(),
        "未注入 OWO_DESKTOP_INSTANCE_ID 时 /health 不得出现 instance_id 键：{v}"
    );
    assert!(
        v["pid"].as_u64().unwrap_or(0) > 0,
        "/health.pid 应为正数：{v}"
    );
    assert_eq!(v["stage"], "ready", "stage 恒为 ready");
    assert!(v["build_id"].is_string(), "build_id 必须是字符串：{v}");

    // ② /openapi.json 契约面：200 引用 HealthResponse，组件登记两个 schema。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/openapi.json", None))
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        spec["paths"]["/health"]["get"]["operationId"],
        serde_json::json!("health")
    );
    assert_eq!(
        spec["paths"]["/health"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        serde_json::json!("#/components/schemas/HealthResponse")
    );
    let schemas = &spec["components"]["schemas"];
    assert!(
        schemas["HealthResponse"].is_object(),
        "HealthResponse 组件应登记"
    );
    assert!(schemas["BuildInfo"].is_object(), "BuildInfo 组件应登记");
    assert_eq!(
        schemas["HealthResponse"]["required"],
        serde_json::json!(["healthy", "version", "api_version", "auto_approve"]),
        "build 为 additive：不得进入 required"
    );
    assert_eq!(
        schemas["BuildInfo"]["required"],
        serde_json::json!(["commit", "dirty", "built_at"])
    );
}

/// R3（§8.1）冷启动诊断 ledger 契约：
/// 1. `/diagnostics/requests` 受 bearer 保护（匿名 401），不进入公开面；
/// 2. 每条记录**恰好**六字段（method/route_template/started_at/duration_ms/status/source）；
/// 3. 动态路由记录为模板（真实资源 id 不落 ledger），查询串不落；
/// 4. Authorization/token 值绝不出现在报告任何字节里；
/// 5. `x-owo-client` 头按白名单消毒（异常值 → other）；
/// 6. fallback 静态资产不进 ledger（首屏 ≤5 口径只算 API）。
#[tokio::test]
async fn diagnostics_requests_ledger_is_protected_and_privacy_safe() {
    use axum::http::{header, Method, Request};
    use owo_agent_server::request_ledger_api;

    /// 附带 bearer 与可选 `x-owo-client` 来源头的请求。
    fn sourced(
        state: &Arc<owo_agent_server::AppState>,
        path: &str,
        client: Option<&str>,
    ) -> axum::http::Request<axum::body::Body> {
        let mut builder = Request::builder().method(Method::GET).uri(path).header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
        if let Some(value) = client {
            builder = builder.header("x-owo-client", value);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    /// 发一次请求并解析 JSON 响应体（返回状态码与值）。
    async fn body_json(
        app: axum::Router,
        req: axum::http::Request<axum::body::Body>,
    ) -> (u16, serde_json::Value) {
        let response = app.oneshot(req).await.unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    let (state, _temp) = test_state().await;
    request_ledger_api::reset_for_test(&state.data_root);
    let app = build_router(Arc::clone(&state));

    // 1) 匿名必须 401（诊断面不得进入公开面）。
    let (status, _) = body_json(
        app.clone(),
        anonymous_request("GET", "/diagnostics/requests", None),
    )
    .await;
    assert_eq!(status, 401, "/diagnostics/requests 匿名应 401");

    // 2) 制造流量：公开 health、动态资源路由（真实 id + 查询串）、
    //    异常来源头的业务请求，以及一次 fallback 静态资产请求。
    let marker_id = "ct-ghost-session-8f2a";
    let traffic: Vec<axum::http::Request<axum::body::Body>> = vec![
        sourced(&state, "/health", Some("web")),
        sourced(
            &state,
            &format!("/session/{marker_id}/diff?deep=1&hint=LEAKME"),
            Some("shell"),
        ),
        sourced(&state, "/sessions", Some("Bearer super-secret-value")),
        // ServeDir fallback：不计入 ledger（首屏 ≤5 只算 API）。
        sourced(&state, "/index.html", Some("web")),
    ];
    for req in traffic {
        let (status, _) = body_json(app.clone(), req).await;
        assert_ne!(status, 401, "合法 token 请求不应 401");
    }

    // 3) 读取报告：窗口 = 三条已匹配 API + 报告自身（自计）。
    let (status, report) = body_json(
        app.clone(),
        request(&state, "GET", "/diagnostics/requests?limit=100", None),
    )
    .await;
    assert_eq!(status, 200, "/diagnostics/requests 带 token 应 200");
    let report_text = report.to_string();
    assert_eq!(
        report["cap"].as_u64(),
        Some(request_ledger_api::LEDGER_CAP as u64)
    );
    let records = report["records"].as_array().unwrap();
    let window = report["returned"].as_u64().unwrap() as usize;
    assert_eq!(window, records.len(), "returned 必须等于窗口长度");
    assert!(
        records.len() >= 3,
        "至少记录 health / session-diff / sessions：{report}"
    );
    assert!(
        report["total"].as_u64().unwrap() >= window as u64,
        "total 单调，不少于窗口长度"
    );

    // 每条记录恰好六字段白名单，且时间戳/时长/状态类型正确。
    for record in records {
        let obj = record.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "duration_ms",
                "method",
                "route_template",
                "source",
                "started_at",
                "status"
            ],
            "ledger 记录必须恰好六字段：{record}"
        );
        let started_at = record["started_at"].as_str().unwrap();
        assert!(
            chrono::DateTime::parse_from_rfc3339(started_at).is_ok(),
            "started_at 必须是 RFC3339：{started_at}"
        );
        assert!(record["duration_ms"].is_number(), "duration_ms 必须是数字");
        assert!(record["status"].is_number(), "status 必须是数字");
    }

    // 4) 动态路由记录为模板；真实 id、查询串、凭据原文一律不落。
    let templates: Vec<&str> = records
        .iter()
        .map(|r| r["route_template"].as_str().unwrap())
        .collect();
    assert!(
        templates.contains(&"/session/{id}/diff"),
        "应记录为路由模板：{templates:?}"
    );
    assert!(
        !report_text.contains(marker_id),
        "真实资源 id 不得进入 ledger"
    );
    assert!(
        !report_text.contains("LEAKME") && !report_text.contains("deep=1"),
        "查询串不得进入 ledger"
    );
    assert!(
        !report_text.contains(state.auth_token.token()),
        "bearer token 值不得出现在诊断报告里"
    );
    assert!(
        !report_text.contains("super-secret-value"),
        "异常 x-owo-client 原文必须被消毒"
    );
    assert!(
        !report_text.contains("index.html"),
        "fallback 静态资产不得进 ledger：{templates:?}"
    );

    // 5) source 消毒：合规值保留，异常值归 other。
    let sources: Vec<&str> = records
        .iter()
        .map(|r| r["source"].as_str().unwrap())
        .collect();
    assert!(sources.contains(&"web"), "合规 source 应保留：{sources:?}");
    assert!(sources.contains(&"shell"), "壳标签应保留：{sources:?}");
    assert!(
        sources.contains(&"other"),
        "异常 source 应消毒为 other：{sources:?}"
    );

    // 6) 聚合口径与窗口自洽：health + auth_token + business == returned。
    let aggregates = &report["aggregates"];
    let health = aggregates["health"].as_u64().unwrap();
    let auth = aggregates["auth_token"].as_u64().unwrap();
    let business = aggregates["business"].as_u64().unwrap();
    assert_eq!(health, 1, "/health 恰有一条（本轮唯一 health）");
    assert_eq!(auth, 0, "本轮无 token 引导请求");
    assert_eq!(
        health + auth + business,
        window as u64,
        "三桶必须恰好覆盖窗口：{aggregates}"
    );
    assert!(business >= 2, "业务桶至少含 sessions 与 session-diff");

    // 7) OpenAPI 契约：六字段 required 必须登记（TS SDK 与验收脚本共同基准）。
    let (status, spec) =
        body_json(app.clone(), request(&state, "GET", "/openapi.json", None)).await;
    assert_eq!(status, 200);
    let entry = &spec["paths"]["/diagnostics/requests"]["get"];
    assert_eq!(
        entry["operationId"],
        serde_json::json!("diagnosticsRequests")
    );
    let item = &entry["responses"]["200"]["content"]["application/json"]["schema"]["properties"]
        ["records"]["items"];
    let mut required: Vec<&str> = item["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    required.sort_unstable();
    assert_eq!(
        required,
        vec![
            "duration_ms",
            "method",
            "route_template",
            "source",
            "started_at",
            "status"
        ]
    );

    // 8) CORS：ledger 来源标签与 trace 头必须预检放行（否则 WebView 发不出）。
    let preflight = Request::builder()
        .method(Method::OPTIONS)
        .uri("/health")
        .header(header::ORIGIN, "tauri://localhost")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(
            header::ACCESS_CONTROL_REQUEST_HEADERS,
            "x-owo-client,x-trace-id",
        )
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(preflight).await.unwrap();
    let allowed = response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        allowed.contains("x-owo-client") && allowed.contains("x-trace-id"),
        "CORS 必须放行 ledger 标签与 trace 头，实际为 {allowed:?}"
    );
}

async fn read_json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// ask_user 应答路由（取优合并自远端 engine）：答案回填挂起中的回合；
/// 未知/已回答的 question_id 一律 404（重复提交零副作用）。
#[tokio::test]
async fn answer_question_routes_to_pending_sender() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let (tx, rx) = tokio::sync::oneshot::channel();
    state
        .pending_questions
        .lock()
        .unwrap()
        .insert("q-1".to_string(), tx);
    state
        .pending_question_sessions
        .lock()
        .unwrap()
        .insert("q-1".to_string(), "session-x".to_string());

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session/session-x/answer/q-1",
            Some(r#"{"question_id":"q-1","answer":"选 A"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "应答应送达");
    let answer = rx.await.expect("挂起回合应收到答案");
    assert_eq!(answer.answer, "选 A");
    assert_eq!(answer.question_id, "q-1");

    // 重复提交（或未知提问）→ 404。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session/session-x/answer/q-1",
            Some(r#"{"question_id":"q-1","answer":"再来一次"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404, "已回答的提问应 404");
}

/// `/fs/open`（取优合并自远端 engine）：路由可达 + 工作区越界 403 + 空路径 400。
/// 不实际调起外部程序（避免测试机弹窗）：只锁安全边界契约。
#[tokio::test]
async fn fs_open_enforces_workspace_scope_and_rejects_empty_path() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // 空路径 → 400（路由可达，参数校验先于任何本机动作）。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/fs/open",
            Some(r#"{"path":"  "}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400, "空路径应 400");

    // 越界路径 → 403（本机动作只允许工作区内）。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/fs/open",
            Some(r#"{"path":"../escape.txt"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 403, "越界路径应 403");
}

/// A8（取优合并自远端 engine）：活跃回合快照 / 桌宠显隐中转 / 跨会话待审批 /
/// 自动化执行记录——路由可达 + 状态机语义（期望/实际心跳双向）。
#[tokio::test]
async fn a8_activity_pet_approvals_and_automation_runs_contract() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    // GET /activity：无活跃回合 → 空列表 + 0 待审批。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/activity", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "/activity 应可达");
    let body = read_json_body(response).await;
    assert_eq!(body["active"].as_array().map(Vec::len), Some(0), "{body}");
    assert_eq!(body["pending_approvals"].as_u64(), Some(0), "{body}");

    // GET /desktop/pet：初始无期望/实际值，桌面端不在线。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/desktop/pet", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "/desktop/pet 应可达");
    let body = read_json_body(response).await;
    assert_eq!(body["desired"], serde_json::Value::Null, "{body}");
    assert_eq!(body["overlay_online"], serde_json::json!(false), "{body}");

    // POST /desktop/pet：工作台写期望值。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/desktop/pet",
            Some(r#"{"visible":true}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = read_json_body(response).await;
    assert_eq!(body["desired"], serde_json::json!(true), "{body}");

    // POST /desktop/pet/report：桌面端心跳上报实际值，回传期望值（差异化同步依据）。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/desktop/pet/report",
            Some(r#"{"visible":false}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = read_json_body(response).await;
    assert_eq!(body["desired"], serde_json::json!(true), "{body}");

    // 心跳后 overlay_online=true，且 GET 返回 actual=false。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/desktop/pet", None))
        .await
        .unwrap();
    let body = read_json_body(response).await;
    assert_eq!(body["overlay_online"], serde_json::json!(true), "{body}");
    assert_eq!(body["actual"], serde_json::json!(false), "{body}");

    // GET /approvals/pending：跨会话待审批列表（无等待 → count=0）。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/approvals/pending", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = read_json_body(response).await;
    assert_eq!(body["count"].as_u64(), Some(0), "{body}");

    // GET /automations/runs：执行记录（未触发过 → 空数组）。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/automations/runs", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let body = read_json_body(response).await;
    assert_eq!(body.as_array().map(Vec::len), Some(0), "{body}");
}

/// 桌宠静态面（远端 be6298f 取优）：`/pet` 与 `/pet-assets` 分目录挂载、
/// 磁盘直读、全局 `Cache-Control: no-store`（改前端刷新即生效，免重新构建 overlay）。
/// 环境变量是进程级共享：与 test_state 共用 STATE_ENV_LOCK 串行化，测试尾部必须清理。
#[tokio::test]
async fn pet_static_routes_are_mounted_and_no_store() {
    let _guard = STATE_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    let pet_root = tempfile::tempdir().unwrap();
    let ui = pet_root.path().join("ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(ui.join("index.html"), "<html>owo-pet-ui</html>").unwrap();
    let assets = pet_root.path().join("assets");
    std::fs::create_dir_all(assets.join("skins").join("default")).unwrap();
    std::fs::write(
        assets.join("skins").join("default").join("skin.json"),
        r#"{"id":"default"}"#,
    )
    .unwrap();

    std::env::set_var("OWO_PET_UI_DIR", &ui);
    std::env::set_var("OWO_PET_ASSETS_DIR", &assets);
    let (state, _temp) = build_state_inner().await;
    let app = build_router(Arc::clone(&state));
    std::env::remove_var("OWO_PET_UI_DIR");
    std::env::remove_var("OWO_PET_ASSETS_DIR");

    // /pet：公开面（无需 bearer），直接读到 index.html，且 no-store 覆盖静态面。
    let response = app
        .clone()
        .oneshot(anonymous_request("GET", "/pet/index.html", None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "/pet 应挂载并磁盘直读");
    let cache_control = response
        .headers()
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        cache_control.contains("no-store"),
        "静态资源必须 no-store（实际 {cache_control:?}）"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains("owo-pet-ui"),
        "应返回磁盘上的桌宠页面正文"
    );

    // /pet-assets：皮肤资产从 OWO_PET_ASSETS_DIR 单一来源直读（与 /pet 分离挂载）。
    let response = app
        .clone()
        .oneshot(anonymous_request(
            "GET",
            "/pet-assets/skins/default/skin.json",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "/pet-assets 应挂载并直读皮肤资产"
    );
}

#[tokio::test]
async fn team_context_api_persists_facts_and_rejects_stale_revision() {
    let (state, _temp) = test_state().await;
    let coordinator = state.workswarm.coordinator().unwrap();
    let team = coordinator
        .create_team_run(&owo_agent_core::CreateTeamRequest::new(
            "验证共享上下文 API",
            owo_agent_protocol::TeamMode::Single,
        ))
        .await
        .unwrap();
    let app = build_router(Arc::clone(&state));
    let path = format!("/teams/{}/context", team.team_id);
    let first = serde_json::json!({
        "expected_revision": 0,
        "key": "api.contract",
        "value": "GET /items returns ItemList",
        "producer": "user",
        "source_refs": ["manual-check"]
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(request(&state, "POST", &path, Some(&first)))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["fact"]["revision"], 1);
    assert_eq!(parsed["fact"]["status"], "candidate");

    let read = app
        .clone()
        .oneshot(request(&state, "GET", &path, None))
        .await
        .unwrap();
    assert_eq!(read.status().as_u16(), 200);
    let body = axum::body::to_bytes(read.into_body(), 128 * 1024)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["revision"], 1);
    assert_eq!(parsed["facts"][0]["value"], "GET /items returns ItemList");

    let forged = serde_json::json!({
        "expected_revision": 1,
        "key": "forged",
        "value": "cannot claim another worker identity",
        "producer": "not-a-team-member"
    })
    .to_string();
    let response = app
        .clone()
        .oneshot(request(&state, "POST", &path, Some(&forged)))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    let stale = serde_json::json!({
        "expected_revision": 0,
        "key": "stale",
        "value": "must not commit",
        "producer": "user"
    })
    .to_string();
    let response = app
        .oneshot(request(&state, "POST", &path, Some(&stale)))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 409);
}

#[tokio::test]
async fn team_create_snapshots_only_same_workspace_source_session_constraints() {
    let (state, _temp) = test_state().await;
    let mut session = owo_agent_core::session::Session::new(
        state.workspace.clone(),
        "test-model",
        Some("系统约束：保留现有协议，不要扩大写范围。".to_string()),
    );
    session.messages.push(owo_agent_core::ChatMessage::user(
        "验收要求：CLI 与 Daemon 的会话要贯通。".to_string(),
    ));
    state.store.save(&session).unwrap();
    let body = serde_json::json!({
        "objective": "验证父会话约束进入团队 Worker",
        "mode": "single",
        "parent_session_id": session.id,
    })
    .to_string();
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "POST", "/teams", Some(&body)))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 202);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let team_id = created["team_id"].as_str().unwrap();
    let coordinator = state.workswarm.coordinator().unwrap();
    let run = coordinator.get_team_run(team_id).await.unwrap();
    assert_eq!(run.shared_context_refs.len(), 1);
    let step = coordinator.load_run_state(team_id).unwrap().plan.steps[0]
        .id
        .clone();
    let slice = coordinator
        .assemble_context_slice(team_id, "m-runner", &step)
        .await
        .unwrap();
    assert!(slice["core_spec"].to_string().contains("保留现有协议"));
    assert!(slice["core_spec"].to_string().contains("会话要贯通"));
}

#[tokio::test]
async fn team_create_rejects_parent_session_from_another_workspace() {
    let (state, temp) = test_state().await;
    let other_workspace = temp.path().join("other-workspace");
    std::fs::create_dir_all(&other_workspace).unwrap();
    let session = owo_agent_core::session::Session::new(
        other_workspace,
        "test-model",
        Some("foreign constraints".to_string()),
    );
    state.store.save(&session).unwrap();
    let body = serde_json::json!({
        "objective": "不得注入其他工作区的会话约束",
        "mode": "single",
        "parent_session_id": session.id,
    })
    .to_string();
    let app = build_router(Arc::clone(&state));
    let response = app
        .oneshot(request(&state, "POST", "/teams", Some(&body)))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
}

// Keep these cases in the complete route suite as well as the bounded focused target.
#[path = "major_repair_http_tests.rs"]
mod major_repair_http;

#[test]
fn desktop_credentials_and_durable_session_facts_are_documented() {
    let spec: serde_json::Value = serde_json::from_str(CONTRACT_SNAPSHOT).unwrap();
    assert!(spec["paths"]["/desktop/model-credentials"]["post"].is_object());
    assert_eq!(
        spec["components"]["schemas"]["DesktopModelCredentialBinding"]["properties"]["api_key"]
            ["writeOnly"],
        true
    );
    assert!(
        spec["components"]["schemas"]["SessionDetail"]["properties"]["turn_history"].is_object()
    );
}

#[path = "desktop_six_item_contract_tests.rs"]
mod desktop_six_item_contracts;

struct ExpressionProbe(String);
#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for ExpressionProbe {
    async fn complete(
        &self,
        _: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        panic!("expression must never use generic completion or Agent loop");
    }
    async fn complete_expression(
        &self,
        messages: &[owo_agent_core::ChatMessage],
    ) -> Result<String, String> {
        assert_eq!(messages.len(), 2, "no workspace or conversation context");
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[1].role, "user");
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn expression_is_authenticated_stateless_and_bounded() {
    let _guard = STATE_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    std::env::remove_var("OWO_API_RPM_GLOBAL");
    let temp = tempfile::tempdir().unwrap();
    let ws = temp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let agent = Agent::new(
        Arc::new(ExpressionProbe("改写结果".into())),
        ToolRegistry::new(),
        Policy::new(&ws),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&ws.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        ws.join("traces"),
        temp.path().into(),
        ws,
    ));
    let app = build_router(state.clone());
    let response = app
        .clone()
        .oneshot(anonymous_request(
            "POST",
            "/expression",
            Some(r#"{"prompt":"改写文字"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    for (body, expected) in [
        (r#"{"prompt":" "}"#, 400),
        (r#"{"prompt":"改写文字","tools":["read_file"]}"#, 422),
        (r#"{"prompt":"改写文字","workspace":"."}"#, 422),
        (r#"{"prompt":"改写文字"}"#, 200),
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, "POST", "/expression", Some(body)))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        if expected == 200 {
            let bytes = axum::body::to_bytes(response.into_body(), 32768)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["text"],
                "改写结果"
            );
        }
    }
    // Preserve desktop callers and all accepted loopback Host forms.
    for host in ["localhost:4096", "127.0.0.1:4096", "[::1]:4096"] {
        let mut req = request(
            &state,
            "POST",
            "/expression",
            Some(r#"{"prompt":"synthetic"}"#),
        );
        req.headers_mut().insert("host", host.parse().unwrap());
        req.headers_mut()
            .insert("origin", "tauri://localhost".parse().unwrap());
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status().as_u16(),
            200
        );
    }
    let body = serde_json::json!({"prompt":"中".repeat(8193)}).to_string();
    assert_eq!(
        app.oneshot(request(&state, "POST", "/expression", Some(&body)))
            .await
            .unwrap()
            .status()
            .as_u16(),
        400
    );
    assert!(
        state.sessions.lock().unwrap().is_empty(),
        "expression must not create a session"
    );
    assert_eq!(state.shutdown_gate.active_turns(), 0);
}

#[tokio::test]
async fn expression_rejects_oversized_provider_output() {
    let temp = tempfile::tempdir().unwrap();
    let ws = temp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let provider = ExpressionProbe("中".repeat(4097));
    let agent = Agent::new(
        Arc::new(provider),
        ToolRegistry::new(),
        Policy::new(&ws),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&ws.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        ws.join("traces"),
        temp.path().into(),
        ws,
    ));
    let response = build_router(state.clone())
        .oneshot(request(
            &state,
            "POST",
            "/expression",
            Some(r#"{"prompt":"hi"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 502);
    assert_eq!(state.shutdown_gate.active_turns(), 0);
}

#[tokio::test]
async fn expression_rejects_invalid_host_or_origin_before_provider() {
    let (state, _temp) = test_state().await;
    let app = build_router(state.clone());
    // IdleProvider fails expression by default; any accepted source would reach 502.
    for (host, origin, expected_status) in [
        (None, None, 403),
        (Some("evil.example"), None, 421),
        (Some("localhost:80@evil.example"), None, 403),
        (Some("localhost:0"), None, 403),
        (Some("localhost/path"), None, 421),
        (Some("127.0.0.1:4096"), Some("https://evil.example"), 403),
        (
            Some("127.0.0.1:4096"),
            Some("http://localhost:80@evil.example"),
            403,
        ),
        (Some("127.0.0.1:4096"), Some("http://localhost/path"), 403),
        (
            Some("127.0.0.1:4096"),
            Some("http://localhost?secret=synthetic"),
            403,
        ),
        (Some("127.0.0.1:4096"), Some("null"), 403),
    ] {
        let mut req = request(
            &state,
            "POST",
            "/expression",
            Some(r#"{"prompt":"synthetic"}"#),
        );
        req.headers_mut().remove("host");
        if let Some(host) = host {
            req.headers_mut().insert("host", host.parse().unwrap());
        }
        if let Some(origin) = origin {
            req.headers_mut().insert("origin", origin.parse().unwrap());
        }
        // A claimed forwarding chain must not authorize a remote Host.
        req.headers_mut()
            .insert("x-forwarded-host", "localhost".parse().unwrap());
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status().as_u16(),
            expected_status,
            "host={host:?}, origin={origin:?}"
        );
        let body = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        if expected_status == 421 {
            let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["code"], "http/untrusted_host");
        } else {
            assert_eq!(body.as_ref(), b"expression/invalid_source");
        }
    }
    for header in ["host", "origin"] {
        let mut req = request(
            &state,
            "POST",
            "/expression",
            Some(r#"{"prompt":"synthetic"}"#),
        );
        if header == "origin" {
            req.headers_mut()
                .insert("origin", "http://localhost".parse().unwrap());
        }
        req.headers_mut()
            .append(header, "http://localhost".parse().unwrap());
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status().as_u16(),
            403
        );
    }
    assert_eq!(state.shutdown_gate.active_turns(), 0);
    assert!(state.sessions.lock().unwrap().is_empty());
}

/// 纯对话模式：创建时指定 -> 切回 agent -> 读回一致 -> 非法值 400。
///
/// 走真实 Router + 真 SQLite，避免只断言"函数存在"。
#[tokio::test]
async fn chat_session_mode_roundtrip_and_validation() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":".","model":"idle","mode":"chat"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created["mode"], "chat", "创建时应返回 chat 模式");
    let id = created["id"].as_str().unwrap().to_string();

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{id}/mode"),
            Some(r#"{"mode":"agent"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let switched: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(switched["mode"], "agent");

    // 读回会话确认落库（不是只改了内存副本）。
    let response = app
        .clone()
        .oneshot(request(&state, "GET", &format!("/session/{id}"), None))
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let detail: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(detail["mode"], "agent");

    // 未知模式必须 400：不能静默降级成 agent 或 chat。
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{id}/mode"),
            Some(r#"{"mode":"telepathy"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

/// 纯对话可以不选工作区（ChatGPT 式直接开聊）；Agent 模式仍然必须有工作区，
/// 显式写错的路径在两种模式下都必须是 400，不能静默落到默认目录。
#[tokio::test]
async fn chat_session_creates_isolated_workspace_when_none_is_given() {
    let (state, temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":"   ","model":"idle","mode":"chat"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created["mode"], "chat");
    let workspace = created["workspace"].as_str().unwrap();
    assert!(
        !workspace.trim().is_empty(),
        "纯对话会话也必须返回真实工作区"
    );
    assert!(
        std::path::Path::new(workspace).is_dir(),
        "返回的工作区应当已存在：{workspace}"
    );
    assert_eq!(
        std::path::Path::new(workspace),
        temp.path().join("chat"),
        "纯对话目录应隔离在数据根下，而不是用户项目"
    );

    // 会话详情读回同一路径，后续回合才能落到同一个沙箱。
    let detail = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/session/{}", created["id"].as_str().unwrap()),
            None,
        ))
        .await
        .unwrap();
    let body = axum::body::to_bytes(detail.into_body(), usize::MAX)
        .await
        .unwrap();
    let detail: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(detail["workspace"], workspace);

    // Agent 模式缺少工作区必须拒绝。
    let denied = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":"","model":"idle"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), axum::http::StatusCode::BAD_REQUEST);

    // 显式写错的路径不能被默认目录兜底。
    let typo = app
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":"Z:/definitely-missing-owo","model":"idle","mode":"chat"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(typo.status(), axum::http::StatusCode::BAD_REQUEST);
}
#[tokio::test]
async fn prompt_presets_crud_and_session_apply_contract() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));

    let create = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/session",
            Some(r#"{"workspace":".","model":"idle"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(create.status().as_u16(), 200, "POST /session 应 200");
    let bytes = axum::body::to_bytes(create.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let session_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // 初始为空库（测试数据目录是独立的临时目录）。
    let list = app
        .clone()
        .oneshot(request(&state, "GET", "/prompts", None))
        .await
        .unwrap();
    assert_eq!(list.status().as_u16(), 200);
    let body = axum::body::to_bytes(list.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["prompts"].as_array().unwrap().len(), 0);

    // 创建
    let saved = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/prompts",
            Some(r#"{"name":"严谨评审","content":"先给结论，再列证据。"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(saved.status().as_u16(), 200, "创建提示词应 200");
    let saved = axum::body::to_bytes(saved.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    let preset_id = saved["prompt"]["id"].as_str().unwrap().to_string();
    assert_eq!(saved["prompt"]["name"], "严谨评审");

    // 名称非法：明确 400，不落库
    let bad = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/prompts",
            Some(r#"{"name":"   ","content":"x"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 400, "空名称应 400");

    // 带 id 保存 = 更新同一条（前端导入/编辑都依赖这个语义）
    let updated = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/prompts",
            Some(&format!(
                r#"{{"id":"{preset_id}","name":"严谨评审 v2","content":"先给结论，再列证据，最后给适用边界。"}}"#
            )),
        ))
        .await
        .unwrap();
    assert_eq!(updated.status().as_u16(), 200, "更新提示词应 200");
    let updated = axum::body::to_bytes(updated.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let updated: serde_json::Value = serde_json::from_slice(&updated).unwrap();
    assert_eq!(updated["prompt"]["id"].as_str().unwrap(), preset_id);
    assert_eq!(updated["prompt"]["name"], "严谨评审 v2");
    assert_eq!(
        updated["prompts"].as_array().unwrap().len(),
        1,
        "更新不能新增第二条"
    );

    // 不存在的 id 更新：404 而不是悄悄变成新建
    let missing = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/prompts",
            Some(r#"{"id":"no-such-id","name":"x","content":"y"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404, "未知 id 应 404");

    // 内容超限：400，且不落库
    let oversize = "长".repeat(20_001);
    let too_long = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/prompts",
            Some(&format!(r#"{{"name":"超长","content":"{oversize}"}}"#)),
        ))
        .await
        .unwrap();
    assert_eq!(too_long.status().as_u16(), 400, "超长内容应 400");
    let still = app
        .clone()
        .oneshot(request(&state, "GET", "/prompts", None))
        .await
        .unwrap();
    let still = axum::body::to_bytes(still.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let still: serde_json::Value = serde_json::from_slice(&still).unwrap();
    assert_eq!(still["prompts"].as_array().unwrap().len(), 1);

    // 应用到会话
    let applied = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{session_id}/system-prompt"),
            Some(r#"{"prompt":"先给结论，再列证据。"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(applied.status().as_u16(), 200, "应用提示词应 200");
    let applied = axum::body::to_bytes(applied.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let applied: serde_json::Value = serde_json::from_slice(&applied).unwrap();
    assert_eq!(applied["system_prompt"], "先给结论，再列证据。");

    // 会话详情读回（重启恢复也走同一字段）
    let detail = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/session/{session_id}"),
            None,
        ))
        .await
        .unwrap();
    let detail = axum::body::to_bytes(detail.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let detail: serde_json::Value = serde_json::from_slice(&detail).unwrap();
    assert_eq!(detail["system_prompt"], "先给结论，再列证据。");

    // 空串 = 清除
    let cleared = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{session_id}/system-prompt"),
            Some(r#"{"prompt":"  "}"#),
        ))
        .await
        .unwrap();
    assert_eq!(cleared.status().as_u16(), 200);
    let cleared = axum::body::to_bytes(cleared.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let cleared: serde_json::Value = serde_json::from_slice(&cleared).unwrap();
    assert!(cleared["system_prompt"].is_null(), "空串应清除提示词");

    // 删除
    let deleted = app
        .clone()
        .oneshot(request(
            &state,
            "DELETE",
            &format!("/prompts/{preset_id}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(deleted.status().as_u16(), 200, "删除应 200");
    let again = app
        .clone()
        .oneshot(request(
            &state,
            "DELETE",
            &format!("/prompts/{preset_id}"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(again.status().as_u16(), 404, "重复删除应 404");
}

#[tokio::test]
async fn media_delivery_contract_is_authenticated_binary_and_range_aware() {
    use owo_agent_core::project_space_store::ProjectSpaceStoreBackend;
    let (state, temp) = test_state().await;
    let root = temp.path().join("workswarm");
    let cas = owo_agent_core::CasStore::new(root.join("cas")).unwrap();
    let content = b"ID3\0\x01\x02\x03\x04";
    let hash = cas.put(content).unwrap();
    let store = owo_agent_core::SqliteProjectSpaceStore::open(&root.join("space.db")).unwrap();
    let artifact: owo_agent_protocol::Artifact = serde_json::from_value(serde_json::json!({
        "artifact_id":"media-route", "kind":"audio", "version":1, "producer":"session-1",
        "content_ref":format!("cas://sha256:{hash}"), "classification":"private", "review_state":"draft",
        "created_at":"2026-10-25T00:00:00Z", "format":"binary", "media_type":"audio/mpeg",
        "sha256":hash, "size_bytes":content.len(), "file_name":"voice.mp3"
    })).unwrap();
    store
        .save_artifact(&artifact, "session:session-1")
        .await
        .unwrap();
    let app = build_router(state.clone());
    let path = "/artifacts/media-route/content?raw=true";
    let mut req = request(&state, "GET", path, None);
    req.headers_mut()
        .insert("Range", "bytes=1-3".parse().unwrap());
    let response = app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-type"], "audio/mpeg");
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes 1-3/{}", content.len())
    );
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 100)
            .await
            .unwrap()
            .as_ref(),
        &content[1..=3]
    );
    let mut req = request(&state, "GET", path, None);
    req.headers_mut()
        .insert("Range", "bytes=100-".parse().unwrap());
    assert_eq!(app.clone().oneshot(req).await.unwrap().status(), 416);
    let req = axum::http::Request::builder()
        .uri(path)
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(req).await.unwrap().status(), 401);
    for source in [CONTRACT_SNAPSHOT, include_str!("../src/openapi.json")] {
        let spec: serde_json::Value = serde_json::from_str(source).unwrap();
        let route = &spec["paths"]["/artifacts/{id}/content"]["get"];
        assert!(route["responses"]["206"].is_object());
        assert!(route["responses"]["416"].is_object());
        assert_eq!(
            spec["components"]["schemas"]["ToolOutputArtifact"]["properties"]["artifact_id"]
                ["type"],
            "string"
        );
    }
}

#[tokio::test]
async fn workflow_catalog_contract_matches_draft_preflight_and_rejects_before_team_persistence() {
    let (state, _temp) = test_state().await;
    let app = build_router(state.clone());
    let anonymous = anonymous_request("GET", "/workflows/tools", None);
    assert_eq!(app.clone().oneshot(anonymous).await.unwrap().status(), 401);
    let response = app
        .clone()
        .oneshot(request(&state, "GET", "/workflows/tools", None))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let catalog: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let tools = catalog["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "generate_image" && tool["worker_supported"] == true));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "read_image" && tool["status"] == "unsupported"));
    assert_eq!(catalog["service_verification"], "not_performed");
    assert!(!String::from_utf8_lossy(&body).contains("api_key"));
    let template = serde_json::json!({"template_id":"offline-catalog","name":"离线连接验证","mode":"team","applicability":"offline","created_at":"","roles":[{"role":"media creator","assignee":"agent","tools":["mcp__offline__draw"],"verify":"non_empty"}]});
    let body = template.to_string();
    let saved = app
        .clone()
        .oneshot(request(&state, "POST", "/workflows/user", Some(&body)))
        .await
        .unwrap();
    assert_eq!(
        saved.status(),
        200,
        "unconfigured templates remain editable and saveable"
    );
    let checked = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/workflows/user/validate",
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(checked.status(), 200);
    let checked = axum::body::to_bytes(checked.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let checked: serde_json::Value = serde_json::from_slice(&checked).unwrap();
    assert_eq!(
        checked["ok"], true,
        "structural validation is distinct from runtime availability"
    );
    assert_eq!(checked["preflight"]["ok"], false);
    assert_eq!(checked["preflight"]["issues"][0]["code"], "not_connected");
    let start = r#"{"objective":"offline","mode":"team","template_id":"offline-catalog"}"#;
    let rejected = app
        .clone()
        .oneshot(request(&state, "POST", "/teams", Some(start)))
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    let rejected = axum::body::to_bytes(rejected.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&rejected).contains("workflow preflight"));
    let teams = app
        .clone()
        .oneshot(request(&state, "GET", "/teams", None))
        .await
        .unwrap();
    let teams = axum::body::to_bytes(teams.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let teams: serde_json::Value = serde_json::from_slice(&teams).unwrap();
    assert!(
        teams["teams"].as_array().unwrap().is_empty(),
        "rejection must not leave a team run behind"
    );
    let invalid_draft = r#"{"description":"offline","tools":["mcp__offline__draw"]}"#;
    assert_eq!(
        app.oneshot(request(
            &state,
            "POST",
            "/workflows/draft",
            Some(invalid_draft)
        ))
        .await
        .unwrap()
        .status(),
        422,
        "must reject before IdleProvider is called"
    );
    for source in [CONTRACT_SNAPSHOT, include_str!("../src/openapi.json")] {
        let doc: serde_json::Value = serde_json::from_str(source).unwrap();
        assert!(
            doc["paths"]["/workflows/tools"]["get"]["responses"]["200"]["content"]
                ["application/json"]["schema"]
                .is_object()
        );
        assert!(
            doc["components"]["schemas"]["WorkflowToolPreflight"]["properties"]["issues"]
                .is_object()
        );
    }
}

#[tokio::test]
async fn workflow_graph_document_contract_validates_and_preserves_portable_layout() {
    async fn json_body(response: axum::response::Response) -> serde_json::Value {
        serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap(),
        )
        .unwrap()
    }
    let (state, _temp) = test_state().await;
    let app = build_router(state.clone());
    let template = serde_json::json!({"template_id":"graph-contract","name":"Graph fixture","applicability":"graph","roles":[{"node_id":"node-a","role":"First","assignee":"human"},{"node_id":"node-b","role":"Second","assignee":"human","depends_on":["First"]}],"editor":{"version":1,"nodes":[{"id":"node-a","position":{"x":10.5,"y":220}}]}});
    let saved = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/workflows/user",
            Some(&template.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let listed = json_body(
        app.clone()
            .oneshot(request(&state, "GET", "/workflows/user", None))
            .await
            .unwrap(),
    )
    .await;
    // Positions use f64: JSON 220 and 220.0 describe the same coordinate.
    // Compare the complete editor payload after the public DTO's numeric normalization.
    let expected_editor: owo_agent_protocol::WorkflowEditorMetadata =
        serde_json::from_value(template["editor"].clone()).unwrap();
    assert_eq!(
        listed["templates"][0]["template"]["editor"],
        serde_json::to_value(expected_editor).unwrap()
    );
    let mut invalid = template.clone();
    invalid["roles"][1]["node_id"] = serde_json::json!("node-a");
    let rejected = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/workflows/user",
            Some(&invalid.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), 422);
    let retained = json_body(
        app.clone()
            .oneshot(request(&state, "GET", "/workflows/user", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        retained["templates"][0]["template"]["roles"][1]["node_id"],
        "node-b"
    );
    for source in [CONTRACT_SNAPSHOT, include_str!("../src/openapi.json")] {
        let schema: serde_json::Value = serde_json::from_str(source).unwrap();
        assert_eq!(
            schema["components"]["schemas"]["WorkflowTemplate"]["properties"]["editor"]["$ref"],
            "#/components/schemas/WorkflowEditorMetadata"
        );
        assert!(
            schema["paths"]["/workflows/draft/{id}/adopt"]["post"]["requestBody"]["content"]
                ["application/json"]["schema"]["properties"]["template"]
                .is_object()
        );
    }
}

#[tokio::test]
async fn proposal_routes_enforce_frozen_revision_and_publish_committed_identity() {
    use serde_json::json;
    let (state, _temp) = test_state().await;
    let app = build_router(state.clone());
    let registry = owo_agent_core::workswarm::TeamTemplateRegistry::for_workswarm_root(
        &state.data_root.join("workswarm"),
    );
    let proposal:owo_agent_protocol::TeamTemplateProposal=serde_json::from_value(json!({
        "proposal_id":"contract-proposal","source_team_id":"fixture","created_at":"fixture",
        "template":{"template_id":"contract-proposal-flow","name":"Reviewed candidate","origin":"user","applicability":"fixture",
        "roles":[{"role":"author","assignee":"agent","worker":"echo","tools":["read_file"]}]}
    })).unwrap();
    registry.save_proposal(&proposal).unwrap();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/teams/templates/proposals/contract-proposal/adopt",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["revision"], 1);
    assert_eq!(body["content_hash"].as_str().unwrap().len(), 64);
    let mut stale = proposal.clone();
    stale.proposal_id = "stale-proposal".into();
    registry.save_proposal(&stale).unwrap();
    let mut newer = registry.get_template("contract-proposal-flow").unwrap();
    newer.name = "Concurrent edit".into();
    registry.save_user_template(&newer, Some(1)).unwrap();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/teams/templates/proposals/stale-proposal/adopt",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        registry
            .get_template("contract-proposal-flow")
            .unwrap()
            .name,
        "Concurrent edit"
    );
    assert_eq!(
        registry
            .get_proposal_record("stale-proposal")
            .unwrap()
            .unwrap()
            .base_revision,
        1
    );
    assert_eq!(
        registry.get_proposal("stale-proposal").unwrap().status,
        owo_agent_protocol::TeamTemplateProposalStatus::Proposed
    );
    let mut invalid = proposal.clone();
    invalid.proposal_id = "invalid-proposal".into();
    invalid.template.template_id = "invalid-proposal-flow".into();
    invalid.template.roles[0].depends_on = vec!["missing".into()];
    registry.save_proposal(&invalid).unwrap();
    let response = app
        .oneshot(request(
            &state,
            "POST",
            "/teams/templates/proposals/invalid-proposal/adopt",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert!(registry
        .get_revision("invalid-proposal-flow")
        .unwrap()
        .is_none());
    for source in [CONTRACT_SNAPSHOT, include_str!("../src/openapi.json")] {
        let spec: serde_json::Value = serde_json::from_str(source).unwrap();
        let install = &spec["paths"]["/teams/templates/catalog/{id}/install"]["post"];
        assert!(install["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["in"] == "query" && p["name"] == "expected_revision"));
        assert!(install["responses"]["409"].is_object());
        assert!(
            spec["paths"]["/teams/templates/proposals/{proposal_id}/adopt"]["post"]["responses"]
                ["409"]
                .is_object()
        );
    }
}

#[tokio::test]
async fn team_detail_reports_frozen_template_identity_and_keeps_legacy_unknown() {
    use serde_json::json;
    let (state, _temp) = test_state().await;
    let app = build_router(state.clone());
    let coordinator = state.workswarm.coordinator().unwrap();
    let registry = coordinator.templates();
    let mut template: owo_agent_protocol::TeamTemplate = serde_json::from_value(json!({
        "template_id":"contract-source", "name":"Original", "mode":"swarmflow",
        "applicability":"contract-source", "origin":"user",
        "roles":[{"node_id":"source-node","role":"human-author","assignee":"human","worker":"user"}]
    }))
    .unwrap();
    let saved = registry.save_template_checked(&template, Some(0)).unwrap();
    let mut create = owo_agent_core::CreateTeamRequest::new(
        "contract-source",
        owo_agent_protocol::TeamMode::Swarmflow,
    );
    create.template_id = Some(template.template_id.clone());
    let team = coordinator.create_team_run(&create).await.unwrap();
    template.name = "Edited after team creation".into();
    template.roles[0].role = "different-role".into();
    registry.save_template_checked(&template, Some(1)).unwrap();
    let endpoint = format!("/teams/{}", team.team_id);
    let response = app
        .clone()
        .oneshot(request(&state, "GET", &endpoint, None))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["template_source"]["revision"], 1);
    assert_eq!(
        body["template_source"]["content_hash"],
        saved.content_hash.unwrap()
    );
    assert_eq!(body["template_source"]["template_id"], "contract-source");
    assert_eq!(body["template_source"]["roles_overridden"], false);
    assert_eq!(body["team"]["members"][0]["role"], "human-author");
    let path = coordinator
        .run_dir()
        .join(format!("{}-meta.json", team.team_id));
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("template_source");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let response = app
        .clone()
        .oneshot(request(&state, "GET", &endpoint, None))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let legacy_body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(legacy_body["template_source"].is_null());
    std::fs::write(&path, b"{corrupt metadata").unwrap();
    let response = app
        .oneshot(request(&state, "GET", &endpoint, None))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        500,
        "corrupt metadata must remain a visible diagnostic error"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{corrupt metadata");
    for source in [CONTRACT_SNAPSHOT, include_str!("../src/openapi.json")] {
        let spec: serde_json::Value = serde_json::from_str(source).unwrap();
        let field = &spec["paths"]["/teams/{id}"]["get"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["properties"]["template_source"];
        assert_eq!(field["nullable"], true);
        assert_eq!(
            field["allOf"][0]["$ref"],
            "#/components/schemas/RunTemplateSource"
        );
        assert_eq!(
            spec["components"]["schemas"]["RunTemplateSource"]["properties"]["revision"]["minimum"],
            1
        );
    }
}

#[test]
fn workflow_draft_contract_exposes_transaction_conflicts_and_storage_failures() {
    for source in [CONTRACT_SNAPSHOT, include_str!("../src/openapi.json")] {
        let spec: serde_json::Value = serde_json::from_str(source).unwrap();
        for (path, methods) in [
            ("/workflows/draft", vec!["get", "post"]),
            ("/workflows/draft/converge", vec!["post"]),
            ("/workflows/draft/{id}", vec!["get", "delete"]),
            ("/workflows/draft/{id}/adopt", vec!["post"]),
            ("/workflows/draft/{id}/verify", vec!["post"]),
        ] {
            for method in methods {
                let responses = &spec["paths"][path][method]["responses"];
                assert!(responses["500"].is_object(), "{path} {method}");
                if method == "post" || method == "delete" {
                    assert!(responses["409"].is_object(), "{path} {method}");
                }
            }
        }
    }
}

#[test]
fn run_timeline_contract_matches_durable_cursor_and_page_bounds() {
    for raw in [
        include_str!("../src/openapi.json"),
        include_str!("../../../clients/ts/openapi.json"),
    ] {
        let spec: serde_json::Value = serde_json::from_str(raw).unwrap();
        let route = &spec["paths"]["/teams/{id}/timeline"]["get"];
        for code in ["200", "400", "401", "404", "500"] {
            assert!(route["responses"].get(code).is_some(), "{code}");
        }
        assert_eq!(
            route["responses"]["200"]["content"]["application/json"]["schema"]["oneOf"],
            serde_json::json!([{"$ref":"#/components/schemas/RunEventPage"},{"$ref":"#/components/schemas/RunEventHistoryPage"}])
        );
        assert!(route["description"]
            .as_str()
            .unwrap()
            .contains("pending delivery"));
        let before = route["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "before")
            .unwrap();
        assert!(before["description"]
            .as_str()
            .unwrap()
            .contains("Mutually exclusive"));
        let history = &spec["components"]["schemas"]["RunEventHistoryPage"]["properties"];
        assert_eq!(history["next_before"]["nullable"], true);
        assert!(history["has_more"]["description"]
            .as_str()
            .unwrap()
            .contains("older committed"));
        let data = &spec["components"]["schemas"]["RunEvent"]["properties"]["data"];
        for field in [
            "record_ref",
            "source_timestamp",
            "payload_preview",
            "payload_complete",
            "payload_unavailable",
            "artifact_refs_complete",
        ] {
            assert!(data["properties"].get(field).is_some(), "{field}");
        }
        assert!(
            spec["components"]["schemas"]["RunEventPage"]["properties"]["has_more"]["description"]
                .as_str()
                .unwrap()
                .contains("pending Worker")
        );
        let page = &spec["components"]["schemas"]["RunEventPage"]["properties"];
        assert_eq!(page["task_history_complete"]["type"], "boolean");
        assert!(page["task_history_complete"]["description"]
            .as_str()
            .unwrap()
            .contains("legacy"));
        assert!(page["has_more"]["description"]
            .as_str()
            .unwrap()
            .contains("task checkpoint"));
        for field in [
            "task",
            "source_seq",
            "step_id",
            "previous_status",
            "removed",
        ] {
            assert!(data["properties"].get(field).is_some(), "{field}");
        }
        assert_eq!(
            data["properties"]["task"]["properties"]["waiting_for"]["maxItems"],
            64
        );
        let parameters = route["parameters"].as_array().unwrap();
        assert!(parameters
            .iter()
            .any(|p| p["name"] == "Last-Event-ID" && p["in"] == "header"));
        assert!(parameters
            .iter()
            .any(|p| p["name"] == "limit" && p["schema"]["maximum"] == 256));
        for field in [
            "seq",
            "run_id",
            "node_id",
            "task_id",
            "attempt",
            "event_type",
            "causation_id",
            "tool_call_id",
            "artifact_refs",
            "timestamp",
        ] {
            assert!(
                spec["components"]["schemas"]["RunEvent"]["properties"]
                    .get(field)
                    .is_some(),
                "{field}"
            );
        }
    }
}

#[test]
fn conversation_contract_declares_bounded_handoff_history_and_distinct_cursors() {
    let spec: serde_json::Value = serde_json::from_str(CONTRACT_SNAPSHOT).unwrap();
    let op = &spec["paths"]["/teams/{id}/conversation"]["get"];
    assert!(op["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "handoff_before"));
    assert_eq!(
        op["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/TeamConversationPage"
    );
    let page = &spec["components"]["schemas"]["TeamConversationPage"]["properties"];
    assert_eq!(page["handoffs"]["maxItems"], 64);
    assert!(page["next_handoff_before"]["type"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("null")));
    assert!(page.get("has_more").is_some());
    assert!(page.get("handoffs_has_more").is_some());
}

struct InputImageContractProvider {
    messages: Arc<std::sync::Mutex<Vec<Vec<owo_agent_core::ChatMessage>>>>,
}
#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for InputImageContractProvider {
    fn validate_image_input(&self, _model: Option<&str>, _has_images: bool) -> Result<(), String> {
        Ok(())
    }

    async fn complete(
        &self,
        messages: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.messages.lock().unwrap().push(messages.to_vec());
        Ok(owo_agent_core::ModelOutput::Text(serde_json::json!({"status":"done","summary":"image fixture observed","artifact":{"kind":"document","format":"text","content":"image fixture observed"},"evidence":[],"open_issues":[]}).to_string()))
    }
}

#[tokio::test]
async fn immutable_input_contract_upload_turn_and_authenticated_delivery_preserve_exact_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = InputImageContractProvider {
        messages: Arc::clone(&observed),
    };
    let state = Arc::new(owo_agent_server::AppState::new(
        Agent::new(
            Arc::new(provider),
            ToolRegistry::empty(),
            Policy::new(&workspace),
            Default::default(),
        ),
        SqliteSessionStore::open(&workspace.join("sessions.db")).unwrap(),
        workspace.join("traces"),
        temp.path().to_path_buf(),
        workspace.clone(),
    ));
    let session = owo_agent_core::session::Session::new(workspace.clone(), "fixture", None);
    let other = owo_agent_core::session::Session::new(workspace, "fixture", None);
    state.store.save(&session).unwrap();
    state.store.save(&other).unwrap();
    let app = build_router(Arc::clone(&state));
    // Valid tiny PNG; tests prove byte transport, not vision interpretation.
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    let mut uploads = Vec::new();
    for bytes in [
        &png[..],
        b"\x89PNG\r\n\x1a\nsecond-header-only-fixture".as_slice(),
    ] {
        let body = serde_json::json!({"name":"same.png","mime":"image/png","data_b64":base64::engine::general_purpose::STANDARD.encode(bytes)}).to_string();
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/attachments", session.id),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        uploads.push(
            serde_json::from_slice::<serde_json::Value>(
                &axum::body::to_bytes(response.into_body(), 64 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap(),
        );
    }
    assert_ne!(uploads[0]["id"], uploads[1]["id"]);
    assert_ne!(uploads[0]["asset"]["sha256"], uploads[1]["asset"]["sha256"]);
    let content_url = uploads[0]["content_url"].as_str().unwrap();
    let response = app
        .clone()
        .oneshot(anonymous_request("GET", content_url, None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 401);
    let response = app
        .clone()
        .oneshot(request(&state, "GET", content_url, None))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .as_ref(),
        png.as_slice()
    );
    for (part, owner, expected) in [
        (uploads[0]["content_part"].clone(), other.id.as_str(), 403),
        (
            serde_json::json!({"type":"image","asset":{"artifact_id":uploads[0]["id"],"sha256":"0".repeat(64)}}),
            session.id.as_str(),
            400,
        ),
        (
            serde_json::json!({"type":"file","asset":uploads[0]["asset"]}),
            session.id.as_str(),
            400,
        ),
    ] {
        let body = serde_json::json!({"prompt":"inspect","content_parts":[part]}).to_string();
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{owner}/turn"),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
    }
    assert!(observed.lock().unwrap().is_empty());
    let body=serde_json::json!({"prompt":"inspect original","content_parts":[uploads[0]["content_part"]]}).to_string();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/turn", session.id),
            Some(&body),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let stream = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&stream).contains("image fixture observed"));
    {
        let messages = observed.lock().unwrap();
        assert!(!messages.is_empty());
        let user = messages[0].iter().rev().find(|m| m.role == "user").unwrap();
        assert_eq!(user.images.len(), 1);
        assert_eq!(
            user.images[0].url,
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&png)
            )
        );
    }
    let persisted = state.store.load(&session.id).unwrap();
    assert!(persisted
        .messages
        .iter()
        .any(|m| m.role == "user" && m.images.len() == 1));
    let list = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/session/{}/attachments", session.id),
            None,
        ))
        .await
        .unwrap();
    let list: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
    let original_user = persisted
        .messages
        .iter()
        .find(|m| m.role == "user")
        .unwrap();
    let input = original_user
        .input
        .as_ref()
        .expect("persist original input refs");
    assert_eq!(input.prompt, "inspect original");
    assert_eq!(
        serde_json::to_value(&input.content_parts).unwrap(),
        serde_json::json!([uploads[0]["content_part"]])
    );
    // An excluded, damaged later input must not block an earlier fork.
    let mut with_tail = persisted.clone();
    let mut tail = owo_agent_core::ChatMessage::user("later image".to_string());
    tail.input = Some(owo_agent_protocol::StoredUserInput {
        prompt: "later image".into(),
        content_parts: vec![serde_json::from_value(uploads[1]["content_part"].clone()).unwrap()],
        legacy_attachments: Vec::new(),
    });
    with_tail.messages.push(tail);
    state.store.save(&with_tail).unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), with_tail);
    let tail_cas = temp
        .path()
        .join("workswarm")
        .join("cas")
        .join(uploads[1]["asset"]["sha256"].as_str().unwrap());
    std::fs::write(&tail_cas, b"tampered excluded tail").unwrap();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{}/fork", session.id),
            Some("{\"message_index\":0}"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let fork: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    let fork_id = fork["id"].as_str().unwrap();
    let forked = state.store.load(fork_id).unwrap();
    let fork_input = forked
        .messages
        .iter()
        .find(|m| m.role == "user")
        .unwrap()
        .input
        .as_ref()
        .unwrap();
    let fork_parts = serde_json::to_value(&fork_input.content_parts).unwrap();
    assert_ne!(fork_parts[0]["asset"]["artifact_id"], uploads[0]["id"]);
    assert_eq!(
        fork_parts[0]["asset"]["sha256"],
        uploads[0]["asset"]["sha256"]
    );
    {
        use owo_agent_core::project_space_store::{
            ProjectSpaceStoreBackend, SqliteProjectSpaceStore,
        };
        let asset_store =
            SqliteProjectSpaceStore::open(&temp.path().join("workswarm").join("space.db")).unwrap();
        let copied = asset_store
            .get_artifact(fork_parts[0]["asset"]["artifact_id"].as_str().unwrap())
            .await
            .unwrap();
        assert_eq!(copied.producer, fork_id);
        assert!(copied.source_refs.contains(&"input:fork".to_string()));
        assert!(copied
            .source_refs
            .contains(&format!("artifact:{}", uploads[0]["id"].as_str().unwrap())));
        assert!(copied
            .source_refs
            .contains(&format!("session:{}", session.id)));
    }
    state.store.save(&persisted).unwrap();
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session.id.clone(), persisted.clone());
    std::fs::write(tail_cas, b"\x89PNG\r\n\x1a\nsecond-header-only-fixture").unwrap();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{fork_id}/turn"),
            Some("{\"prompt\":\"continue from original image\"}"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let _ = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    observed.lock().unwrap().clear();
    let body = serde_json::json!({"objective":"Inspect the uploaded image fixture","strategy":"team","mode":"team",
        "parent_session_id":session.id,"content_parts":[uploads[0]["content_part"]],
        "roles":[{"role":"visual-reader","assignee":"agent","tools":[],"write_paths":[]}],"budget":{"max_steps":4,"max_model_calls":4}}).to_string();
    let response = app
        .clone()
        .oneshot(request(&state, "POST", "/teams", Some(&body)))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    assert_eq!(status.as_u16(), 202, "{}", String::from_utf8_lossy(&bytes));
    let team: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let team_id = team["team_id"].as_str().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(12), async {
        loop {
            let response = app
                .clone()
                .oneshot(request(&state, "GET", &format!("/teams/{team_id}"), None))
                .await
                .unwrap();
            let detail: serde_json::Value = serde_json::from_slice(
                &axum::body::to_bytes(response.into_body(), 1024 * 1024)
                    .await
                    .unwrap(),
            )
            .unwrap();
            let status: owo_agent_protocol::TeamRunStatus =
                serde_json::from_value(detail["team"]["status"].clone()).unwrap();
            if status.is_terminal() {
                assert_eq!(
                    status,
                    owo_agent_protocol::TeamRunStatus::Succeeded,
                    "{detail}"
                );
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("fixture team must finish");
    // The role omitted verify: the host must compile the default output
    // obligation rather than defer a missing plan to the delivery gate.
    let run = state
        .workswarm
        .coordinator()
        .unwrap()
        .load_run_state(team_id)
        .unwrap();
    let step = run
        .plan
        .steps
        .iter()
        .find(|step| step.id == "s-visual-reader")
        .unwrap();
    let requirement = &step
        .verification_plan
        .as_ref()
        .expect("default role verification plan")
        .requirements[0];
    assert_eq!(requirement.validator_id, "artifact-output-non-empty-v1");
    assert!(requirement.required);
    let worker_input = {
        let calls = observed.lock().unwrap();
        assert!(!calls.is_empty());
        let user = calls[0].iter().rev().find(|m| m.role == "user").unwrap();
        assert_eq!(user.images.len(), 1);
        assert_eq!(
            user.images[0].url,
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&png)
            )
        );
        let worker_input = user
            .input
            .as_ref()
            .expect("Team provider receives persisted structured input")
            .clone();
        assert_eq!(worker_input.content_parts.len(), 1);
        assert_ne!(
            serde_json::to_value(&worker_input.content_parts).unwrap()[0]["asset"]["artifact_id"],
            uploads[0]["id"]
        );
        assert_eq!(
            serde_json::to_value(&worker_input.content_parts).unwrap()[0]["asset"]["sha256"],
            uploads[0]["asset"]["sha256"]
        );
        worker_input
    };
    let journal = state
        .workswarm
        .coordinator()
        .unwrap()
        .store()
        .run_events_after(team_id, 0, 256)
        .unwrap()
        .events;
    let prepared = journal
        .iter()
        .find(|e| e.event_type == "model.input.prepared")
        .expect("actual role execution must record provider input");
    let parent = journal
        .iter()
        .find(|e| Some(&e.event_id) == prepared.causation_id.as_ref())
        .expect("provider input has a persisted causal parent");
    assert_eq!(parent.event_type, "worker.input.prepared");
    assert_eq!(parent.run_id, prepared.run_id);
    assert_eq!(prepared.data["image_count"], 1);
    assert_eq!(
        prepared.data["last_user_input"]["parts"][0]["asset"],
        serde_json::to_value(&worker_input.content_parts).unwrap()[0]["asset"]
    );
    let result = journal
        .iter()
        .find(|e| {
            e.event_type == "model.request.completed"
                && e.causation_id.as_ref() == Some(&prepared.event_id)
        })
        .unwrap();
    assert_eq!(result.data["provider_succeeded"], true);
    assert_eq!(result.data["model_delivery"], "not_proven");
    // Follow the actual public Team index to the persisted worker session.
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/teams/{team_id}/conversation"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let conversation: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    let worker_id = conversation["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["role"] == "visual-reader")
        .unwrap()["session_id"]
        .as_str()
        .unwrap();
    let reopened = SqliteSessionStore::open(&state.workspace.join("sessions.db")).unwrap();
    use owo_agent_core::session::SessionStore as _;
    let worker_session = reopened.load(worker_id).unwrap();
    assert_eq!(
        worker_session.parent_id.as_deref(),
        Some(session.id.as_str())
    );
    let saved = worker_session
        .messages
        .iter()
        .find(|m| m.role == "user")
        .unwrap();
    assert_eq!(saved.input.as_ref(), Some(&worker_input));
    assert_eq!(
        saved.images[0].url,
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&png)
        )
    );
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "GET",
            &format!("/session/{worker_id}/attachments"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let child_assets: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(child_assets.as_array().unwrap().len(), 1);
    assert_eq!(
        child_assets[0]["content_part"],
        serde_json::to_value(&worker_input.content_parts[0]).unwrap()
    );
    // Child custody never grants reuse of the original parent identity.
    let foreign=serde_json::json!({"prompt":"wrong parent reference","content_parts":[uploads[0]["content_part"]]}).to_string();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            &format!("/session/{worker_id}/turn"),
            Some(&foreign),
        ))
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 403);
}

#[tokio::test]
async fn immutable_input_contract_documents_shared_parts_and_accurate_upload_limits() {
    let (state, _temp) = test_state().await;
    let response = build_router(Arc::clone(&state))
        .oneshot(request(&state, "GET", "/capabilities", None))
        .await
        .unwrap();
    let capability: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(capability["constraints"]["structured_input_parts"], true);
    for (name, flag) in [
        ("image_generation", "media_image_generation"),
        ("text_to_speech", "media_speech"),
    ] {
        let view = &capability["media_connections"][name];
        assert_eq!(capability["constraints"][flag], view["configured"]);
        assert!(view["connected"].is_null());
        assert!(view["verified"].is_null());
        assert!(view["last_verified_at"].is_null());
        assert!(view.get("api_key").is_none());
    }

    let document: serde_json::Value =
        serde_json::from_str(CONTRACT_SNAPSHOT.trim_start_matches('﻿')).unwrap();
    assert_eq!(
        document["components"]["schemas"]["TurnRequest"]["properties"]["content_parts"]["maxItems"],
        32
    );
    assert_eq!(
        document["components"]["schemas"]["ContentPart"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(
        document["paths"]["/session/{id}/attachments"]["post"]["description"]
            .as_str()
            .unwrap()
            .contains("5 MiB")
    );
    assert_eq!(
        document["paths"]["/session/{id}/attachments"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/UploadedAttachment"
    );
}

#[tokio::test]
async fn immutable_input_contract_rejects_corrupt_cas_and_unsupported_audio_before_model() {
    let (state, _temp) = test_state().await;
    let session = owo_agent_core::session::Session::new(state.workspace.clone(), "fixture", None);
    state.store.save(&session).unwrap();
    let app = build_router(Arc::clone(&state));
    for (name, mime, bytes, expected) in [
        ("bad.png", "image/png", b"not png".as_slice(), 400),
        ("photo.heic", "image/heic", b"fixture".as_slice(), 400),
        ("sound.wav", "audio/wav", b"RIFFfixture".as_slice(), 200),
        (
            "image.png",
            "image/png",
            b"\x89PNG\r\n\x1a\nfixture".as_slice(),
            200,
        ),
    ] {
        let body=serde_json::json!({"name":name,"mime":mime,"data_b64":base64::engine::general_purpose::STANDARD.encode(bytes)}).to_string();
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/attachments", session.id),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected);
        if expected != 200 {
            continue;
        }
        let item: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        if mime == "image/png" {
            std::fs::write(
                state
                    .data_root
                    .join("workswarm/cas")
                    .join(item["asset"]["sha256"].as_str().unwrap()),
                b"tampered",
            )
            .unwrap();
        }
        let body = serde_json::json!({"prompt":"inspect","content_parts":[item["content_part"]]})
            .to_string();
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/turn", session.id),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            if mime == "audio/wav" { 422 } else { 400 }
        );
    }
}

#[tokio::test]
async fn media_profiles_host_config_is_authenticated_versioned_and_used_by_capabilities_and_workers(
) {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let unauthorized = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/settings/media-connections")
                .header("host", "127.0.0.1:4096")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), 401);
    let initial = app
        .clone()
        .oneshot(request(&state, "GET", "/settings/media-connections", None))
        .await
        .unwrap();
    assert_eq!(initial.status(), 200);
    let initial: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(initial.into_body(), 512 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(initial["catalog"]["revision"], 0);
    assert!(!state.data_root.join("media-connections.sqlite3").exists());
    let mut proposal = serde_json::json!({"schema_version":1,"revision":0,"profiles":[{"id":"speech-local","name":"Local speech","base_url":"http://127.0.0.1:9999/v1","model":"local-profile-speech","credential_env":null,"services":["text_to_speech"]}],"assignments":{"text_to_speech":"speech-local"}});
    let saved = app
        .clone()
        .oneshot(request(
            &state,
            "PUT",
            "/settings/media-connections",
            Some(&proposal.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let saved: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(saved.into_body(), 512 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["catalog"]["revision"], 1);
    assert_eq!(
        saved["services"]["text_to_speech"]["profile_id"],
        "speech-local"
    );
    assert!(saved["services"]["text_to_speech"]["connected"].is_null());
    assert!(saved["services"]["text_to_speech"]["verified"].is_null());
    let conflict = app
        .clone()
        .oneshot(request(
            &state,
            "PUT",
            "/settings/media-connections",
            Some(&proposal.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(conflict.status(), 409);
    let capability = app
        .clone()
        .oneshot(request(&state, "GET", "/capabilities", None))
        .await
        .unwrap();
    let capability: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(capability.into_body(), 512 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        capability["media_connections"]["text_to_speech"]["model"],
        "local-profile-speech"
    );
    assert_eq!(
        capability["media_connections"]["text_to_speech"]["configuration_source"],
        "named_profile"
    );
    assert!(state
        .agent
        .workflow_tool_capabilities()
        .unwrap()
        .iter()
        .any(|tool| tool.name == "text_to_speech" && tool.status == "registered"));
    proposal["revision"] = 1.into();
    proposal["profiles"][0]["credential_env"] =
        format!("OWO_PROFILE_CONTRACT_{}", uuid::Uuid::new_v4().simple()).into();
    let saved = app
        .clone()
        .oneshot(request(
            &state,
            "PUT",
            "/settings/media-connections",
            Some(&proposal.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(
        saved.status(),
        200,
        "Missing credentials may be saved; runtime must reject"
    );
    assert!(state
        .agent
        .workflow_tool_capabilities()
        .unwrap()
        .iter()
        .any(|tool| tool.name == "text_to_speech" && tool.status == "not_configured"));
    proposal["revision"] = 2.into();
    proposal["profiles"][0]["api_key"] = "fixture-inline-secret".into();
    let rejected = app
        .clone()
        .oneshot(request(
            &state,
            "PUT",
            "/settings/media-connections",
            Some(&proposal.to_string()),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), 422);
    let loaded = owo_agent_core::gateway::media_profiles::MediaProfileStore::new(&state.data_root)
        .load()
        .unwrap();
    assert_eq!(loaded.revision, 2);
    assert!(!serde_json::to_string(&loaded)
        .unwrap()
        .contains("fixture-inline-secret"));
}

#[tokio::test]
async fn media_profile_tcp_check_is_authenticated_persisted_and_revision_scoped() {
    let (state, _temp) = test_state().await;
    let app = build_router(Arc::clone(&state));
    let unauth = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/settings/media-connections/check")
                .header("host", "127.0.0.1:4096")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    r#"{"expected_revision":0,"profile_id":"local","service":"text_to_speech"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let store = owo_agent_core::gateway::media_profiles::MediaProfileStore::new(&state.data_root);
    let proposal = serde_json::json!({"schema_version":1,"revision":0,"profiles":[{"id":"local","name":"local","base_url":format!("http://{}/v1",listener.local_addr().unwrap()),"model":"fixture","credential_env":null,"services":["text_to_speech"]}],"assignments":{"text_to_speech":"local"}});
    let saved = store
        .save(serde_json::from_value(proposal).unwrap())
        .unwrap();
    let request_body=serde_json::json!({"expected_revision":saved.revision,"profile_id":"local","service":"text_to_speech"}).to_string();
    let response = app
        .clone()
        .oneshot(request(
            &state,
            "POST",
            "/settings/media-connections/check",
            Some(&request_body),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let observed: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(observed["connected"], true);
    assert_eq!(observed["kind"], "tcp");
    assert!(observed["checked_at_ms"].as_u64().unwrap() > 0);
    assert!(observed.get("verified").is_none());
    for (path, key) in [
        ("/settings/media-connections", "services"),
        ("/capabilities", "media_connections"),
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, "GET", path, None))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let value: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 512 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(value[key]["text_to_speech"]["connected"], true);
        assert_eq!(
            value[key]["text_to_speech"]["last_checked_at_ms"],
            observed["checked_at_ms"]
        );
        assert!(value[key]["text_to_speech"]["verified"].is_null());
    }
    let mut updated = saved;
    updated.profiles[0].model = "new-model".into();
    let updated = store.save(updated).unwrap();
    assert!(store.checks(updated.revision).unwrap().is_empty());
    let stale = app
        .oneshot(request(
            &state,
            "POST",
            "/settings/media-connections/check",
            Some(&request_body),
        ))
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    assert!(
        store.status(owo_agent_core::gateway::media_config::MediaService::TextToSpeech)
            ["connected"]
            .is_null()
    );
}

struct PendingCheckpointContractProvider {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    fail: bool,
}
#[async_trait::async_trait]
impl owo_agent_core::gateway::ModelProvider for PendingCheckpointContractProvider {
    async fn complete(
        &self,
        _: &[owo_agent_core::ChatMessage],
        _: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        self.entered.notify_one();
        self.release.notified().await;
        if self.fail {
            Err("fixture checkpoint provider failure".into())
        } else {
            Ok(owo_agent_core::ModelOutput::Text(
                "checkpoint completed".into(),
            ))
        }
    }
}
#[tokio::test]
async fn immutable_input_checkpoint_is_readable_while_provider_is_pending() {
    use owo_agent_core::SessionStore;
    for fail in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let db = workspace.join("sessions.db");
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let state = Arc::new(owo_agent_server::AppState::new(
            Agent::new(
                Arc::new(PendingCheckpointContractProvider {
                    entered: Arc::clone(&entered),
                    release: Arc::clone(&release),
                    fail,
                }),
                ToolRegistry::empty(),
                Policy::new(&workspace),
                owo_agent_core::AgentConfig {
                    max_turns: 2,
                    compaction_enabled: false,
                    ..Default::default()
                },
            ),
            SqliteSessionStore::open(&db).unwrap(),
            workspace.join("traces"),
            temp.path().to_path_buf(),
            workspace.clone(),
        ));
        let mut session = owo_agent_core::session::Session::new(&workspace, "fixture", None);
        session.set_mode(owo_agent_core::session::SessionMode::Chat);
        state.store.save(&session).unwrap();
        let app = build_router(Arc::clone(&state));
        let part = serde_json::json!({"type":"text","text":"attached textual input"});
        let body =
            serde_json::json!({"prompt":"original HTTP request","content_parts":[part.clone()]})
                .to_string();
        let response = app
            .oneshot(request(
                &state,
                "POST",
                &format!("/session/{}/turn", session.id),
                Some(&body),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        let reopened = SqliteSessionStore::open(&db).unwrap();
        let during = reopened.load(&session.id).unwrap();
        assert_eq!(
            during.messages.iter().filter(|m| m.role == "user").count(),
            1
        );
        let original = during.messages[0]
            .input
            .as_ref()
            .expect("input committed before Provider completes");
        assert_eq!(original.prompt, "original HTTP request");
        assert_eq!(
            serde_json::to_value(&original.content_parts).unwrap(),
            serde_json::json!([part])
        );
        assert!(!during.messages.iter().any(|m| m.role == "assistant"));
        release.notify_one();
        let stream = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            axum::body::to_bytes(response.into_body(), 1024 * 1024),
        )
        .await
        .unwrap()
        .unwrap();
        let stream = String::from_utf8_lossy(&stream);
        assert!(
            stream.contains(if fail {
                "fixture checkpoint provider failure"
            } else {
                "checkpoint completed"
            }),
            "{stream}"
        );
        let after = reopened.load(&session.id).unwrap();
        assert_eq!(
            after.messages.iter().filter(|m| m.role == "user").count(),
            1
        );
        assert_eq!(after.messages[0].input.as_ref(), Some(original));
    }
}

#[test]
fn image_input_capability_wire_contract_is_bound_and_not_verified() {
    let connection: owo_agent_protocol::CustomModelConnection = serde_json::from_value(serde_json::json!({
        "model":"vision", "base_url":"http://127.0.0.1:4999/v1",
        "input_capabilities":{"model":"vision","base_url":"http://127.0.0.1:4999/v1","image_input":true}
    })).unwrap();
    assert!(connection.input_capabilities.as_ref().unwrap().image_input);
    let mut value = serde_json::to_value(&connection).unwrap();
    value["input_capabilities"]["verified"] = serde_json::json!(true);
    assert!(serde_json::from_value::<owo_agent_protocol::CustomModelConnection>(value).is_err());
    let spec: serde_json::Value = serde_json::from_str(CONTRACT_SNAPSHOT).unwrap();
    assert_eq!(
        spec["components"]["schemas"]["CustomModelConnection"]["properties"]["input_capabilities"]
            ["$ref"],
        "#/components/schemas/ModelInputCapabilityBinding"
    );
    assert_eq!(
        spec["components"]["schemas"]["ModelInputCapabilityBinding"]["additionalProperties"],
        false
    );
    let source: serde_json::Value =
        serde_json::from_str(include_str!("../src/openapi.json")).unwrap();
    assert_eq!(
        source["components"]["schemas"]["ModelInputCapabilityBinding"],
        spec["components"]["schemas"]["ModelInputCapabilityBinding"]
    );
}

#[tokio::test]
async fn direct_subagent_flags_cannot_authorize_side_effects_without_approval_channel() {
    struct CountedProvider(Arc<std::sync::atomic::AtomicUsize>);
    #[async_trait::async_trait]
    impl owo_agent_core::gateway::ModelProvider for CountedProvider {
        async fn complete(
            &self,
            _messages: &[owo_agent_core::ChatMessage],
            _tools: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(owo_agent_core::ModelOutput::Text(
                r#"{"status":"done","summary":"synthetic read-only result","review_result":{"verdict":"approved","findings":[]},"evidence":[],"open_issues":[]}"#.into(),
            ))
        }
    }
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let state = {
        let _guard = STATE_ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let agent = Agent::new(
            Arc::new(CountedProvider(calls.clone())),
            ToolRegistry::new(),
            Policy::new(&workspace),
            Default::default(),
        );
        let store = SqliteSessionStore::open(&temp.path().join("index.db")).unwrap();
        Arc::new(owo_agent_server::AppState::new(
            agent,
            store,
            temp.path().join("traces"),
            temp.path().to_path_buf(),
            workspace,
        ))
    };
    let app = build_router(Arc::clone(&state));
    for body in [
        r#"{"prompt":"synthetic boundary check","read_only":false}"#,
        r#"{"prompt":"synthetic boundary check","read_only":false,"allow_commands":true}"#,
        r#"{"prompt":"synthetic boundary check","read_only":true,"allow_commands":true}"#,
    ] {
        let response = app
            .clone()
            .oneshot(request(&state, "POST", "/subagent/run", Some(body)))
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            403,
            "request flags must not substitute for action approval"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"subagent/approval_channel_required");
    }
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "denied flags must not call the model"
    );
    let response = app
        .oneshot(request(
            &state,
            "POST",
            "/subagent/run",
            Some(r#"{"prompt":"synthetic read-only check"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        200,
        "read-only exploration stays available"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn signed_expression_origin_is_bound_fresh_and_at_most_once() {
    use hmac::{Hmac, Mac};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl owo_agent_core::gateway::ModelProvider for Counted {
        async fn complete(
            &self,
            _: &[owo_agent_core::ChatMessage],
            _: &[owo_agent_core::ToolSpec],
        ) -> Result<owo_agent_core::ModelOutput, String> {
            panic!("no generic Agent/tools path");
        }
        async fn complete_expression(
            &self,
            messages: &[owo_agent_core::ChatMessage],
        ) -> Result<String, String> {
            assert_eq!(messages.len(), 2);
            self.0.fetch_add(1, Ordering::SeqCst);
            if messages[1].content.as_deref() == Some("cancel") {
                return std::future::pending::<Result<String, String>>().await;
            }
            if messages[1].content.as_deref() == Some("fail") {
                Err("synthetic ambiguous provider failure".into())
            } else {
                Ok("synthetic output".into())
            }
        }
    }
    fn sign(capsule: &mut owo_agent_protocol::ExpressionOriginCapsule) {
        let mut mac = Hmac::<Sha256>::new_from_slice(&[7; 32]).unwrap();
        mac.update(owo_agent_protocol::EXPRESSION_ORIGIN_PURPOSE);
        mac.update(&capsule.payload.canonical_bytes().unwrap());
        capsule.signature.value = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
    }
    fn body(capsule: &owo_agent_protocol::ExpressionOriginCapsule, prompt: &str) -> String {
        serde_json::json!({"prompt":prompt,"origin_capsule":capsule}).to_string()
    }
    let _guard = STATE_ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    struct EnvRestore(Option<std::ffi::OsString>);
    impl Drop for EnvRestore {
        fn drop(&mut self) {
            if let Some(v) = &self.0 {
                std::env::set_var("OWO_EXPRESSION_ORIGIN_KEY", v);
            } else {
                std::env::remove_var("OWO_EXPRESSION_ORIGIN_KEY");
            }
        }
    }
    let _restore = EnvRestore(std::env::var_os("OWO_EXPRESSION_ORIGIN_KEY"));
    std::env::set_var("OWO_EXPRESSION_ORIGIN_KEY", "07".repeat(32));
    let calls = Arc::new(AtomicUsize::new(0));
    let temp = tempfile::tempdir().unwrap();
    let ws = temp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let make_state = || {
        let agent = Agent::new(
            Arc::new(Counted(calls.clone())),
            ToolRegistry::new(),
            Policy::new(&ws),
            Default::default(),
        );
        let store = SqliteSessionStore::open(&ws.join("index.db")).unwrap();
        Arc::new(owo_agent_server::AppState::new(
            agent,
            store,
            ws.join("traces"),
            temp.path().into(),
            ws.clone(),
        ))
    };
    let state = make_state();
    let app = build_router(state.clone());
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../contracts/expression-origin/synthetic-golden.json"
    ))
    .unwrap();
    let mut capsule: owo_agent_protocol::ExpressionOriginCapsule =
        serde_json::from_value(golden["capsule"].clone()).unwrap();
    assert_eq!(
        capsule.payload.canonical_bytes().unwrap(),
        golden["canonical_payload"].as_str().unwrap().as_bytes()
    );
    let expected = capsule.signature.value.clone();
    sign(&mut capsule);
    assert_eq!(
        capsule.signature.value, expected,
        "independent Python golden HMAC"
    );
    capsule.payload.freshness.issued_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    sign(&mut capsule);
    for pointer in [
        "/binding/request_id",
        "/binding/session_id",
        "/binding/task_revision",
        "/origin/connector_pid",
        "/origin/process_created_at_hex",
        "/origin/executable_sha256",
        "/origin/context_id",
        "/origin_request_sha256",
        "/scope/mode",
        "/audience",
    ] {
        let mut value = serde_json::to_value(&capsule).unwrap();
        let path = format!("/payload{pointer}");
        let target = value.pointer_mut(&path).unwrap();
        *target = if target.is_number() {
            serde_json::json!(999)
        } else {
            serde_json::json!("tampered")
        };
        let payload =
            serde_json::json!({"prompt":"synthetic rewrite","origin_capsule":value}).to_string();
        assert_eq!(
            app.clone()
                .oneshot(request(&state, "POST", "/expression", Some(&payload)))
                .await
                .unwrap()
                .status()
                .as_u16(),
            403,
            "{pointer}"
        );
    }
    assert_eq!(
        app.clone()
            .oneshot(request(
                &state,
                "POST",
                "/expression",
                Some(&body(&capsule, "different prompt"))
            ))
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(request(
                &state,
                "POST",
                "/expression",
                Some(&body(&capsule, "synthetic rewrite")),
            ))
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 32768)
            .await
            .unwrap();
        let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result["origin_verified"], true);
        assert_eq!(result["text"], "synthetic output");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1, "replay must use receipt");
    let mut conflict = capsule.clone();
    conflict.payload.binding.request_id = "other-request".into();
    sign(&mut conflict);
    assert_eq!(
        app.clone()
            .oneshot(request(
                &state,
                "POST",
                "/expression",
                Some(&body(&conflict, "synthetic rewrite"))
            ))
            .await
            .unwrap()
            .status()
            .as_u16(),
        409
    );
    for future in [false, true] {
        let mut expired = capsule.clone();
        expired.payload.nonce = "55".repeat(32);
        if future {
            expired.payload.freshness.issued_at_ms += 60000;
        } else {
            expired.payload.freshness.issued_at_ms -= 180000;
        }
        sign(&mut expired);
        assert_eq!(
            app.clone()
                .oneshot(request(
                    &state,
                    "POST",
                    "/expression",
                    Some(&body(&expired, "synthetic rewrite"))
                ))
                .await
                .unwrap()
                .status()
                .as_u16(),
            403
        );
    }
    let mut failed = capsule.clone();
    failed.payload.nonce = "66".repeat(32);
    failed.payload.prompt_sha256 = format!("{:x}", Sha256::digest(b"fail"));
    sign(&mut failed);
    for expected in [502, 409] {
        assert_eq!(
            app.clone()
                .oneshot(request(
                    &state,
                    "POST",
                    "/expression",
                    Some(&body(&failed, "fail"))
                ))
                .await
                .unwrap()
                .status()
                .as_u16(),
            expected
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "ambiguous provider error must never reissue"
    );
    let mut cancelled = capsule.clone();
    cancelled.payload.nonce = "77".repeat(32);
    cancelled.payload.prompt_sha256 = format!("{:x}", Sha256::digest(b"cancel"));
    sign(&mut cancelled);
    let cancelled_body = body(&cancelled, "cancel");
    assert!(tokio::time::timeout(
        std::time::Duration::from_millis(50),
        app.clone().oneshot(request(
            &state,
            "POST",
            "/expression",
            Some(&cancelled_body)
        ))
    )
    .await
    .is_err());
    assert_eq!(
        app.clone()
            .oneshot(request(
                &state,
                "POST",
                "/expression",
                Some(&cancelled_body)
            ))
            .await
            .unwrap()
            .status()
            .as_u16(),
        409
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "cancelled model call must not be reissued"
    );
    // Strict DTO parsing rejects duplicate and unknown fields before provider execution.
    let duplicate = body(&capsule, "synthetic rewrite")
        .replace("\"ic_version\":1", "\"ic_version\":1,\"ic_version\":1");
    assert_eq!(
        app.clone()
            .oneshot(request(&state, "POST", "/expression", Some(&duplicate)))
            .await
            .unwrap()
            .status()
            .as_u16(),
        422
    );
    std::env::remove_var("OWO_EXPRESSION_ORIGIN_KEY");
    let unregistered = make_state();
    assert_eq!(
        build_router(unregistered.clone())
            .oneshot(request(
                &unregistered,
                "POST",
                "/expression",
                Some(&body(&capsule, "synthetic rewrite"))
            ))
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(state.sessions.lock().unwrap().is_empty());
}

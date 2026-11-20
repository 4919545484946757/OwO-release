//! OwO's local gateway to the managed collaboration process.

pub(crate) mod delegation;
mod delivery;
mod runtime;

use crate::{auth_token::AuthToken, AppState};
use axum::body::{to_bytes, Body};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use owo_agent_protocol::{CustomModelConnection, TurnRequest};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const COOKIE: &str = "owo_collaboration";
const WORKER_PROOF: &str = "x-owo-owobot-worker";
const TURN_PROOF: &str = "x-owo-owobot-turn";

/// worker 回合的会话归属（服务端权威标记）：侧栏据此把角色会话收进任务分组，
/// 而不是平铺进用户聊天历史。落盘一份，daemon 重启后不丢归属。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerSessionLink {
    pub group_id: String,
    pub thread_id: String,
    pub role: String,
    pub workspace: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerRole {
    Lead,
    Worker,
    Reviewer,
}

impl WorkerRole {
    pub(super) const ALL: [Self; 3] = [Self::Lead, Self::Worker, Self::Reviewer];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Lead => "组长",
            Self::Worker => "执行员",
            Self::Reviewer => "评审员",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Worker => "worker",
            Self::Reviewer => "reviewer",
        }
    }

    pub(crate) fn instructions(self) -> &'static str {
        match self {
            Self::Lead => "理解任务，查阅当前项目信息。在可用时使用 owobot_delegate 交给执行员，收到结果后请评审员独立核对，最后汇总。",
            Self::Worker => "查阅当前项目材料。需要交付文件时，使用 owobot_stage_delivery 暂存完整文件集，回报修订版本与验收结果。",
            Self::Reviewer => "独立核对任务、项目材料和执行结果。有暂存文件时，用 owobot_read_delivery 读取确切版本，用 owobot_review_delivery 记录结论。",
        }
    }

    pub(crate) fn readers(self) -> &'static [&'static str] {
        match self {
            Self::Lead => &["read_file", "list_dir", "search_files"],
            Self::Worker => &[
                "read_file",
                "list_dir",
                "search_files",
                "grep",
                "find_symbol",
            ],
            Self::Reviewer => &["read_file", "list_dir", "search_files", "grep"],
        }
    }
}

pub(crate) struct WorkerTurn {
    pub(crate) role: WorkerRole,
    pub(crate) model: Option<String>,
    dispatcher: Option<delegation::Delegate>,
    delivery: delivery::Store,
    scope: CollaborationTurn,
    readonly: bool,
}

impl WorkerTurn {
    pub(crate) async fn project_tools(
        self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Vec<Arc<dyn owo_agent_core::tools::Tool>>, (StatusCode, String)> {
        let actor = delivery::Actor {
            role: self.role.key().to_owned(),
            bot_id: self.scope.bot_id,
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            assignment_id: self.scope.assignment_id,
        };
        let thread = self.scope.thread_id;
        let store = self.delivery;
        let mut tools = tokio::task::spawn_blocking(move || {
            store.bind(&thread, actor.clone())?;
            Ok::<_, delivery::DeliveryError>(delivery::tools::for_role(
                &store,
                &thread,
                &actor,
                self.role,
                self.readonly,
            ))
        })
        .await
        .map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "owobot/delivery_unavailable".into(),
            )
        })?
        .map_err(|error| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("owobot/{}", error.code()),
            )
        })?;
        if let Some(dispatcher) = self.dispatcher {
            tools.push(Arc::new(dispatcher));
        }
        Ok(tools)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CollaborationTurn {
    bot_id: String,
    group_id: String,
    thread_id: String,
    depth: u32,
    assignment_id: Option<String>,
    #[serde(default)]
    custom_bot: bool,
    #[serde(default)]
    collaborators: Vec<Collaborator>,
}

#[derive(Deserialize)]
struct Collaborator {
    id: String,
    name: String,
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelSelection {
    model: Option<String>,
    connection: Option<CustomModelConnection>,
}

impl ModelSelection {
    fn validate(mut self) -> GatewayResult<Self> {
        self.model = self.model.take().and_then(|model| {
            let model = model.trim();
            (!model.is_empty() && model != "default").then(|| model.to_owned())
        });
        if self
            .model
            .as_ref()
            .is_some_and(|model| model.len() > 256 || model.chars().any(char::is_control))
        {
            return Err(GatewayError::Refused(
                StatusCode::BAD_REQUEST,
                "OwOBot 模型名称无效",
            ));
        }
        if let Some(connection) = &self.connection {
            if connection.api_key.is_some() {
                return Err(GatewayError::Refused(
                    StatusCode::BAD_REQUEST,
                    "OwOBot 模型连接需使用桌面端凭据引用",
                ));
            }
            if self.model.as_deref() != Some(connection.model.trim()) {
                return Err(GatewayError::Refused(
                    StatusCode::BAD_REQUEST,
                    "OwOBot 模型与连接不匹配",
                ));
            }
            owo_agent_core::gateway::custom_model_provider(connection).map_err(|_| {
                GatewayError::Refused(
                    StatusCode::BAD_REQUEST,
                    "OwOBot 模型连接不可用，请检查桌面端模型设置",
                )
            })?;
        }
        Ok(self)
    }
}

type GatewayResult<T> = Result<T, GatewayError>;

enum GatewayError {
    Unavailable(&'static str),
    Refused(StatusCode, &'static str),
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unavailable(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason),
            Self::Refused(status, reason) => (status, reason),
        };
        (status, Json(json!({"error": message}))).into_response()
    }
}

pub(crate) struct Collaboration {
    home: Option<PathBuf>,
    client: reqwest::Client,
    browser_session: AuthToken,
    live: Mutex<HashMap<PathBuf, runtime::Running>>,
    starting: Mutex<()>,
    delivery_gate: Arc<std::sync::Mutex<()>>,
    worker_links: std::sync::Mutex<HashMap<String, WorkerSessionLink>>,
    worker_links_path: PathBuf,
}

impl Collaboration {
    pub(crate) fn new(data_root: PathBuf) -> Self {
        let worker_links_path = data_root.join("owobot-worker-sessions.json");
        let worker_links = std::fs::read_to_string(&worker_links_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Self {
            home: std::env::var_os("OWOBOT_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute()),
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(3))
                .build()
                .expect("loopback collaboration HTTP client"),
            browser_session: AuthToken::generate(),
            live: Mutex::new(HashMap::new()),
            starting: Mutex::new(()),
            delivery_gate: Arc::new(std::sync::Mutex::new(())),
            worker_links: std::sync::Mutex::new(worker_links),
            worker_links_path,
        }
    }

    /// 每次 worker 回合绑定成功后记录归属；标题取该会话首回合的目标文本，
    /// 任务级标题在读取侧按线程内最早会话推导（组长先绑定）。
    pub(crate) fn record_worker_turn(
        &self,
        session_id: &str,
        worker: &WorkerTurn,
        workspace: &FsPath,
        prompt: &str,
        now: &str,
    ) {
        let Ok(mut links) = self.worker_links.lock() else {
            return;
        };
        let link = links
            .entry(session_id.to_owned())
            .or_insert_with(|| WorkerSessionLink {
                group_id: worker.scope.group_id.clone(),
                thread_id: worker.scope.thread_id.clone(),
                role: worker.role.name().to_owned(),
                workspace: workspace.to_string_lossy().into_owned(),
                title: prompt.chars().take(200).collect(),
                created_at: now.to_owned(),
                updated_at: now.to_owned(),
            });
        link.role = worker.role.name().to_owned();
        link.updated_at = now.to_owned();
        let snapshot = links.clone();
        drop(links);
        // 归属是展示层索引：写盘失败不阻断回合，下一回合会再记一次。
        let _ = std::fs::write(
            &self.worker_links_path,
            serde_json::to_vec(&snapshot).unwrap_or_default(),
        );
    }

    pub(crate) fn worker_sessions_wire(&self) -> serde_json::Value {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Task {
            thread_id: String,
            group_id: String,
            workspace: String,
            title: String,
            updated_at: String,
            session_ids: Vec<String>,
        }
        let links = self
            .worker_links
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let mut by_thread: HashMap<String, Task> = HashMap::new();
        let mut title_stamp: HashMap<String, String> = HashMap::new();
        for (session_id, link) in &links {
            let task = by_thread
                .entry(link.thread_id.clone())
                .or_insert_with(|| Task {
                    thread_id: link.thread_id.clone(),
                    group_id: link.group_id.clone(),
                    workspace: link.workspace.clone(),
                    title: link.title.clone(),
                    updated_at: link.updated_at.clone(),
                    session_ids: Vec::new(),
                });
            task.session_ids.push(session_id.clone());
            if link.updated_at > task.updated_at {
                task.updated_at = link.updated_at.clone();
            }
            let stamp = title_stamp
                .entry(link.thread_id.clone())
                .or_insert_with(|| link.created_at.clone());
            if link.created_at < *stamp {
                *stamp = link.created_at.clone();
                task.title = link.title.clone();
                task.workspace = link.workspace.clone();
            }
        }
        let mut tasks: Vec<Task> = by_thread.into_values().collect();
        for task in &mut tasks {
            task.session_ids.sort();
        }
        tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        json!({ "sessions": links, "tasks": tasks })
    }

    pub(crate) fn web_dir(&self) -> PathBuf {
        self.home
            .as_ref()
            .map(|home| home.join("dist"))
            .unwrap_or_else(|| crate::desktop_web_dir().join("owobot"))
    }

    pub(crate) async fn shutdown(&self) {
        let _starting = self.starting.lock().await;
        let running = std::mem::take(&mut *self.live.lock().await);
        let mut stopping = tokio::task::JoinSet::new();
        for project in running.into_values() {
            stopping.spawn(project.stop());
        }
        while stopping.join_next().await.is_some() {}
    }

    pub(crate) async fn bind_worker_turn(
        &self,
        headers: &HeaderMap,
        workspace: &FsPath,
        request: &mut TurnRequest,
    ) -> Result<Option<WorkerTurn>, (StatusCode, String)> {
        let denied = || (StatusCode::FORBIDDEN, "owobot/worker_scope".to_string());
        let Some(proof) = headers.get(WORKER_PROOF) else {
            return if headers.contains_key(TURN_PROOF) {
                Err(denied())
            } else {
                Ok(None)
            };
        };
        let proof = proof.to_str().map_err(|_| denied())?;
        let turn = headers
            .get(TURN_PROOF)
            .and_then(|value| value.to_str().ok())
            .filter(|token| token.len() == 48 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(denied)?;
        let mut live = self.live.lock().await;
        let active = live
            .values_mut()
            .find(|project| project.worker_proof.verify(proof))
            .ok_or_else(denied)?;
        if !active.is_alive() {
            return Err(denied());
        }
        if workspace != active.workspace
            && workspace.canonicalize().ok().as_ref() != Some(&active.workspace)
        {
            return Err(denied());
        }
        if request.read_only == Some(false)
            || request.model_connection.is_some()
            || !request.attachments.is_empty()
            || !request.content_parts.is_empty()
        {
            return Err(denied());
        }

        let scope: CollaborationTurn = self
            .client
            .get(format!("{}/api/internal/owobot-scope", active.base_url))
            .bearer_auth(turn)
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| denied())?
            .json()
            .await
            .map_err(|_| denied())?;
        if scope.group_id != active.group || scope.depth > 3 || scope.thread_id.is_empty() {
            return Err(denied());
        }
        let role = active
            .role_for(&scope.bot_id)
            .or(scope.custom_bot.then_some(WorkerRole::Worker))
            .ok_or_else(denied)?;
        request.model_connection = active.model.connection.clone();
        let dispatcher = ((role == WorkerRole::Lead || scope.custom_bot)
            && scope.depth < 3
            && request.read_only != Some(true))
        .then(|| {
            let mut recipients = vec![
                ("worker".into(), active.member(WorkerRole::Worker).into()),
                (
                    "reviewer".into(),
                    active.member(WorkerRole::Reviewer).into(),
                ),
            ];
            if scope.custom_bot {
                recipients.push(("lead".into(), active.member(WorkerRole::Lead).into()));
            }
            recipients.extend(
                scope
                    .collaborators
                    .iter()
                    .filter(|member| member.id != scope.bot_id)
                    .map(|member| (member.name.clone(), member.id.clone())),
            );
            delegation::Delegate::new(
                self.client.clone(),
                active.base_url.clone(),
                turn.to_owned(),
                active.group.clone(),
                recipients,
            )
        });
        Ok(Some(WorkerTurn {
            role,
            model: active.model.model.clone(),
            dispatcher,
            delivery: active.delivery.clone(),
            scope,
            readonly: request.read_only == Some(true),
        }))
    }

    fn accepts(&self, headers: &HeaderMap) -> bool {
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|part| part.trim().split_once('='))
            .any(|(name, value)| name == COOKIE && self.browser_session.verify(value))
    }
}

/// GET /owobot/worker-sessions：侧栏聚合 OwOBot 任务、收起角色会话的归属索引。
pub(crate) async fn worker_sessions(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    Json(state.owobot.worker_sessions_wire())
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenRequest {
    workspace: Option<PathBuf>,
    model: Option<String>,
    connection: Option<CustomModelConnection>,
}

pub(crate) async fn open(
    State(state): State<Arc<AppState>>,
    request: Option<Json<OpenRequest>>,
) -> Response {
    open_desktop(
        &state,
        request.map(|Json(request)| request).unwrap_or_default(),
    )
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

async fn open_desktop(state: &AppState, request: OpenRequest) -> GatewayResult<Response> {
    let workspace = request.workspace.as_deref().unwrap_or(&state.workspace);
    let workspace = canonical_project(workspace).await?;
    let model = if request.model.is_some() || request.connection.is_some() {
        Some(
            ModelSelection {
                model: request.model,
                connection: request.connection,
            }
            .validate()?,
        )
    } else {
        None
    };
    let collaboration = &state.owobot;
    let home = collaboration
        .home
        .as_deref()
        .filter(|path| {
            path.join("node.exe").is_file() && path.join("dist-server/index.js").is_file()
        })
        .filter(|_| collaboration.web_dir().join("index.html").is_file())
        .ok_or(GatewayError::Unavailable("当前安装缺少 OwOBot 运行组件"))?;
    let _starting = collaboration.starting.lock().await;
    let mut live = collaboration.live.lock().await;
    let needs_start = match live.get_mut(&workspace) {
        Some(running) => !running.is_alive(),
        None => true,
    };
    if needs_start {
        live.remove(&workspace);
        drop(live);
        let running = runtime::start(
            &collaboration.client,
            home,
            &state.data_root,
            &workspace,
            Arc::clone(&collaboration.delivery_gate),
        )
        .await?;
        live = collaboration.live.lock().await;
        live.insert(workspace.clone(), running);
    }
    let active = live.get_mut(&workspace).expect("sidecar started");
    if let Some(model) = model {
        active.model = model;
    }
    let group = &active.group;
    let cookie = format!(
        "{COOKIE}={}; Path=/owobot/api/; HttpOnly; SameSite=Strict",
        collaboration.browser_session.token(),
    );
    let mut response = Json(json!({"url": "/owobot/", "groupId": group})).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("generated cookie is ASCII"),
    );
    Ok(response)
}

async fn canonical_project(workspace: &FsPath) -> GatewayResult<PathBuf> {
    let invalid =
        || GatewayError::Refused(StatusCode::BAD_REQUEST, "OwOBot 需要一个存在的项目目录");
    if !workspace.is_absolute() {
        return Err(invalid());
    }
    let path = tokio::fs::canonicalize(workspace)
        .await
        .map_err(|_| invalid())?;
    if !tokio::fs::metadata(&path)
        .await
        .map_err(|_| invalid())?
        .is_dir()
    {
        return Err(invalid());
    }
    Ok(path)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserProject {
    owo_project: Option<String>,
}

async fn browser_project<T>(
    state: &AppState,
    method: &Method,
    headers: &HeaderMap,
    group: Option<&str>,
    select: impl FnOnce(&runtime::Running) -> GatewayResult<T>,
) -> GatewayResult<T> {
    let collaboration = &state.owobot;
    if !collaboration.accepts(headers) {
        return Err(GatewayError::Refused(
            StatusCode::UNAUTHORIZED,
            "请从 OwO 打开协作工作台",
        ));
    }
    if !same_origin(method, headers) {
        return Err(GatewayError::Refused(
            StatusCode::FORBIDDEN,
            "OwOBot 仅接受桌面工作台请求",
        ));
    }
    let default_workspace = if group.is_none() {
        Some(canonical_project(&state.workspace).await?)
    } else {
        None
    };
    let mut live = collaboration.live.lock().await;
    let running = match default_workspace {
        Some(workspace) => live.get_mut(&workspace),
        None => live
            .values_mut()
            .find(|project| group == Some(project.group.as_str())),
    }
    .ok_or(GatewayError::Unavailable("请重新打开 OwOBot"))?;
    if !running.is_alive() {
        return Err(GatewayError::Unavailable("OwOBot 已停止，请重新打开"));
    }
    select(running)
}

fn delivery_error(error: delivery::DeliveryError) -> Response {
    let status = match error {
        delivery::DeliveryError::Invalid(_) => StatusCode::BAD_REQUEST,
        delivery::DeliveryError::Conflict(_) => StatusCode::CONFLICT,
        delivery::DeliveryError::Missing => StatusCode::NOT_FOUND,
        delivery::DeliveryError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (status, Json(json!({"error": error.code()}))).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeliveryLookup {
    revision: Option<u64>,
    owo_project: Option<String>,
}

pub(crate) async fn read_delivery(
    State(state): State<Arc<AppState>>,
    Path(thread): Path<String>,
    Query(lookup): Query<DeliveryLookup>,
    headers: HeaderMap,
) -> Response {
    let store = match browser_project(
        &state,
        &Method::GET,
        &headers,
        lookup.owo_project.as_deref(),
        |running| Ok(running.delivery.clone()),
    )
    .await
    {
        Ok(store) => store,
        Err(error) => return error.into_response(),
    };
    match tokio::task::spawn_blocking(move || store.inspect(&thread, lookup.revision)).await {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(error)) => delivery_error(error),
        Err(_) => delivery_error(delivery::DeliveryError::Unavailable),
    }
}

pub(crate) async fn decide_delivery(
    State(state): State<Arc<AppState>>,
    Path(thread): Path<String>,
    Query(project): Query<BrowserProject>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let store = match browser_project(
        &state,
        &parts.method,
        &parts.headers,
        project.owo_project.as_deref(),
        |running| Ok(running.delivery.clone()),
    )
    .await
    {
        Ok(store) => store,
        Err(error) => return error.into_response(),
    };
    let Ok(bytes) = to_bytes(body, 4096).await else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Ok(decision) = serde_json::from_slice::<delivery::DecisionRequest>(&bytes) else {
        return delivery_error(delivery::DeliveryError::Invalid(
            "invalid_delivery_decision",
        ));
    };
    let revision = decision.revision;
    let action = decision.action;
    let task_id = format!("owobot-{thread}");
    let lease = crate::workswarm_api::acquire_workspace_delivery_lease(&store.workspace).await;
    let result = tokio::task::spawn_blocking(move || {
        let _lease = lease;
        let result = store.decide(&thread, decision);
        if !matches!(&result, Ok(view) if view.replayed) {
            if let Ok(mut log) = state.agent.audit_log().lock() {
                log.record(
                    &task_id,
                    "owobot_delivery_decision",
                    None,
                    Some(result.is_ok()),
                    json!({"revision":revision,"action":action,
                        "error":result.as_ref().err().map(delivery::DeliveryError::code)})
                    .to_string(),
                );
            }
            crate::flush_audit(&state);
        }
        result
    })
    .await;
    match result {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(error)) => delivery_error(error),
        Err(_) => delivery_error(delivery::DeliveryError::Unavailable),
    }
}

pub(crate) async fn proxy(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    Query(project): Query<BrowserProject>,
    request: Request,
) -> Response {
    forward(&state, &path, project.owo_project.as_deref(), request)
        .await
        .unwrap_or_else(IntoResponse::into_response)
}

async fn forward(
    state: &AppState,
    path: &str,
    group: Option<&str>,
    request: Request,
) -> GatewayResult<Response> {
    let (parts, body) = request.into_parts();
    let (base_url, session) =
        browser_project(state, &parts.method, &parts.headers, group, |running| {
            if !route_allowed(&parts.method, path, &running.group) {
                return Err(GatewayError::Refused(
                    StatusCode::FORBIDDEN,
                    "该操作不属于 OwOBot 协作入口",
                ));
            }
            Ok((running.base_url.clone(), running.session.clone()))
        })
        .await?;
    let query = parts
        .uri
        .query()
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    let mut upstream = state
        .owobot
        .client
        .request(
            parts.method.clone(),
            format!("{base_url}/api/{path}{query}"),
        )
        .bearer_auth(&session);
    if let Some(value) = parts.headers.get("last-event-id") {
        upstream = upstream.header("last-event-id", value);
    }
    if parts.method == Method::POST {
        let bytes = to_bytes(body, 1024 * 1024)
            .await
            .map_err(|_| GatewayError::Refused(StatusCode::PAYLOAD_TOO_LARGE, "OwOBot 请求过大"))?;
        upstream = upstream
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
    }
    const HEAD_LIMIT: Duration = Duration::from_secs(16);
    let response = tokio::time::timeout(HEAD_LIMIT, upstream.send())
        .await
        .ok()
        .and_then(Result::ok)
        .ok_or(GatewayError::Unavailable("OwOBot 本机服务未响应"))?;
    let status = response.status();
    if status.is_redirection() {
        return Err(GatewayError::Unavailable("OwOBot 返回了非预期跳转"));
    }
    let content_type = if path == "events" && status.is_success() {
        "text/event-stream"
    } else {
        "application/json"
    };
    let mut reply = Response::new(Body::from_stream(response.bytes_stream()));
    *reply.status_mut() = status;
    reply
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    reply.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(reply)
}

fn same_origin(method: &Method, headers: &HeaderMap) -> bool {
    if let Some(site) = headers.get("sec-fetch-site") {
        if site != "same-origin" {
            return false;
        }
    }
    match headers.get(header::ORIGIN) {
        None => method == Method::GET,
        Some(origin) => headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .is_some_and(|host| origin == format!("http://{host}").as_str()),
    }
}

fn route_allowed(method: &Method, path: &str, group: &str) -> bool {
    let segments: Vec<_> = path.split('/').collect();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || segment.len() > 128
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return false;
    }

    match (method, segments.as_slice()) {
        (&Method::GET, ["bots" | "events" | "instances" | "search"]) => true,
        (&Method::GET, ["threads", _, "messages"]) => true,
        (&Method::GET, ["groups", id]) => *id == group,
        (&Method::POST, ["groups", id, "messages" | "read" | "interrupt" | "tasks" | "bots"]) => {
            *id == group
        }
        (&Method::POST, ["groups", id, "tasks" | "bots", _]) => *id == group,
        _ => false,
    }
}

/// 协作角色额外可见的 MCP 工具名。
///
/// 只取宿主已按「server + tool + schema hash」声明为可信只读的工具（`effect = Read` 且
/// `host_verified_readonly`）；MCP 自报的 `readOnlyHint` 不算，写工具仍走暂存与审批。
/// 服务器白名单来自 `OWO_OWOBOT_MCP_SERVERS`（逗号分隔，`*` 表示全部）；未配置时不注入。
pub(crate) fn mcp_reader_names(specs: &[owo_agent_core::tools::ToolSpec]) -> Vec<String> {
    let configured = std::env::var("OWO_OWOBOT_MCP_SERVERS").unwrap_or_default();
    let servers: Vec<&str> = configured
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    mcp_reader_names_for(specs, &servers)
}

fn mcp_reader_names_for(
    specs: &[owo_agent_core::tools::ToolSpec],
    servers: &[&str],
) -> Vec<String> {
    if servers.is_empty() {
        return Vec::new();
    }
    let every_server = servers.contains(&"*");
    specs
        .iter()
        .filter(|spec| {
            let Some(rest) = spec.name.strip_prefix("mcp__") else {
                return false;
            };
            let Some((server, _tool)) = rest.split_once("__") else {
                return false;
            };
            if !every_server && !servers.contains(&server) {
                return false;
            }
            spec.effect.as_ref().is_some_and(|effect| {
                effect.class == owo_agent_core::EffectClass::Read && effect.host_verified_readonly
            })
        })
        .map(|spec| spec.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_gateway_only_exposes_project_chat_surface() {
        assert!(route_allowed(
            &Method::POST,
            "groups/current/messages",
            "current"
        ));
        assert!(route_allowed(&Method::GET, "events", "current"));
        assert!(route_allowed(
            &Method::GET,
            "threads/abc/messages",
            "current"
        ));
        assert!(!route_allowed(
            &Method::POST,
            "groups/other/messages",
            "current"
        ));
        assert!(!route_allowed(&Method::GET, "groups/other", "current"));
        assert!(!route_allowed(&Method::POST, "auth/pairing", "current"));
        assert!(!route_allowed(&Method::GET, "config", "current"));
        assert!(!route_allowed(
            &Method::GET,
            "threads/../messages",
            "current"
        ));
        assert!(!route_allowed(
            &Method::GET,
            "threads/%2e%2e/messages",
            "current"
        ));
    }

    #[test]
    fn cookies_and_origin_are_bound_to_the_collaboration_gateway() {
        let temp = tempfile::TempDir::new().unwrap();
        let collaboration = Collaboration::new(temp.path().to_path_buf());
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:4096"));
        assert!(!collaboration.accepts(&headers));
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "another=123; {COOKIE}={}",
                collaboration.browser_session.token(),
            ))
            .unwrap(),
        );
        assert!(collaboration.accepts(&headers));
        assert!(same_origin(&Method::GET, &headers));
        assert!(!same_origin(&Method::POST, &headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4097"),
        );
        assert!(!same_origin(&Method::GET, &headers));
        assert!(!same_origin(&Method::POST, &headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:4096"),
        );
        assert!(same_origin(&Method::POST, &headers));
    }

    fn mcp_spec(
        name: &str,
        class: owo_agent_core::EffectClass,
        verified: bool,
    ) -> owo_agent_core::tools::ToolSpec {
        owo_agent_core::tools::ToolSpec {
            name: name.to_string(),
            description: String::new(),
            input_schema: serde_json::json!({ "type": "object" }),
            effect: Some(owo_agent_core::ToolEffect {
                tool: name.to_string(),
                class,
                source: "mcp:github".to_string(),
                risk_note: None,
                annotations: None,
                host_verified_readonly: verified,
            }),
        }
    }

    #[test]
    fn collaboration_roles_only_see_declared_readonly_mcp_tools() {
        let specs = vec![
            mcp_spec(
                "mcp__github__search_issues",
                owo_agent_core::EffectClass::Read,
                true,
            ),
            mcp_spec(
                "mcp__github__create_issue",
                owo_agent_core::EffectClass::Write,
                false,
            ),
            mcp_spec(
                "mcp__github__list_repos",
                owo_agent_core::EffectClass::Read,
                false,
            ),
            mcp_spec(
                "mcp__slack__search",
                owo_agent_core::EffectClass::Read,
                true,
            ),
            mcp_spec("read_file", owo_agent_core::EffectClass::Read, true),
        ];
        assert_eq!(
            mcp_reader_names_for(&specs, &["github"]),
            vec!["mcp__github__search_issues".to_string()]
        );
        assert!(mcp_reader_names_for(&specs, &[]).is_empty());
        assert_eq!(mcp_reader_names_for(&specs, &["*"]).len(), 2);
    }

    #[test]
    fn worker_links_aggregate_tasks_and_survive_reload() {
        let temp = tempfile::TempDir::new().unwrap();
        let collaboration = Collaboration::new(temp.path().to_path_buf());
        let make_turn = |role: WorkerRole, bot_id: &str, depth: u32| WorkerTurn {
            role,
            model: None,
            dispatcher: None,
            delivery: delivery::Store::new(
                temp.path(),
                PathBuf::from("D:/proj"),
                "group-1".to_owned(),
                Arc::new(std::sync::Mutex::new(())),
            )
            .unwrap(),
            scope: CollaborationTurn {
                bot_id: bot_id.to_owned(),
                group_id: "group-1".to_owned(),
                thread_id: "thread-1".to_owned(),
                depth,
                assignment_id: None,
                custom_bot: false,
                collaborators: Vec::new(),
            },
            readonly: false,
        };
        let lead = make_turn(WorkerRole::Lead, "bot-lead", 0);
        let worker = make_turn(WorkerRole::Worker, "bot-worker", 1);
        collaboration.record_worker_turn(
            "session-lead",
            &lead,
            FsPath::new("D:/proj"),
            "整理本周交付清单",
            "2026-11-17T10:00:00+00:00",
        );
        collaboration.record_worker_turn(
            "session-worker",
            &worker,
            FsPath::new("D:/proj"),
            "子任务：核对文件",
            "2026-11-17T10:01:00+00:00",
        );
        let wire = collaboration.worker_sessions_wire();
        assert_eq!(wire["sessions"].as_object().unwrap().len(), 2);
        let tasks = wire["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1, "同线程的角色会话必须聚合成一个任务");
        assert_eq!(tasks[0]["threadId"], "thread-1");
        assert_eq!(
            tasks[0]["title"], "整理本周交付清单",
            "任务标题取线程内最早会话的目标文本，而不是角色指令"
        );
        assert_eq!(tasks[0]["updatedAt"], "2026-11-17T10:01:00+00:00");
        assert_eq!(
            tasks[0]["sessionIds"],
            serde_json::json!(["session-lead", "session-worker"])
        );
        drop(collaboration);
        let reloaded = Collaboration::new(temp.path().to_path_buf());
        let wire = reloaded.worker_sessions_wire();
        assert_eq!(
            wire["sessions"].as_object().unwrap().len(),
            2,
            "归属落盘后重启不丢"
        );
        assert_eq!(wire["tasks"].as_array().unwrap().len(), 1);
    }
}

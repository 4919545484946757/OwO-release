//! AI 生成工作流草稿的 HTTP 集成测试。
//!
//! 用可编排的假 provider（返回预定 JSON）驱动真实路由，**不调用云端模型**：
//! 覆盖 生成→列表→详情→采纳→丢弃 全链路，以及三类失败：
//! 无 JSON、JSON 结构不符、校验不过（采纳被拒且不落盘）。

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::{Agent, ChatMessage, ModelOutput, ToolSpec};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

/// 假 provider：按顺序返回预设回复（用完后重复最后一条）。
struct ScriptedProvider {
    replies: Mutex<Vec<String>>,
    /// 最后一次用户消息（断言修复提示词带上校验错误）。
    last_user: Mutex<String>,
}

impl ScriptedProvider {
    fn new(replies: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            last_user: Mutex::new(String::new()),
        })
    }
}

#[async_trait::async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolSpec],
    ) -> Result<ModelOutput, String> {
        if let Some(user) = messages.iter().rev().find(|message| message.role == "user") {
            *self.last_user.lock().unwrap() = user.content.clone().unwrap_or_default();
        }
        let mut replies = self.replies.lock().unwrap();
        let reply = if replies.len() > 1 {
            replies.remove(0)
        } else {
            replies.first().cloned().unwrap_or_default()
        };
        Ok(ModelOutput::Text(reply))
    }
}

struct Fixture {
    state: Arc<owo_agent_server::AppState>,
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    data_root: std::path::PathBuf,
}

fn fixture(replies: Vec<String>) -> Fixture {
    fixture_with_provider(ScriptedProvider::new(replies))
}

fn fixture_with_provider(provider: Arc<ScriptedProvider>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("ws");
    let data_root = temp.path().join("data");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&data_root).unwrap();
    let agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn ModelProvider>,
        ToolRegistry::new(),
        Policy::new(&workspace),
        Default::default(),
    );
    let store = SqliteSessionStore::open(&workspace.join("index.db")).unwrap();
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        workspace.join("traces"),
        data_root.clone(),
        workspace,
    ));
    Fixture {
        state,
        temp,
        data_root,
    }
}

async fn call(
    state: &Arc<owo_agent_server::AppState>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, Value) {
    use axum::http::{header, Method, Request};
    let app = owo_agent_server::build_router(Arc::clone(state));
    let mut builder = Request::builder()
        .method(Method::from_bytes(method.as_bytes()).unwrap())
        .uri(path)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", state.auth_token.token()),
        );
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let request = match body {
        Some(body) => builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(axum::body::Body::empty()).unwrap(),
    };
    let resp = app.oneshot(request).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        json!(null)
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!(null))
    };
    (status, value)
}

/// 模型"生成"的一份合法工作流（游戏关卡生成流水线，含人工验收节点）。
fn good_reply() -> String {
    let template = json!({
        "template_id": "level-design-pipeline",
        "name": "游戏关卡生成流水线",
        "mode": "team",
        "applicability": "关卡 生成 玩法 数值",
        "description": "从玩法意图到可玩关卡：设计 → 生成 → 可玩性校验 → 人工验收",
        "version": "1.0.0",
        "tags": ["game", "level"],
        "roles": [
            {
                "role": "designer",
                "assignee": "agent",
                "depends_on": [],
                "handoff_contract": "关卡设计稿：目标玩法、难度曲线、元素清单",
                "verify": "non_empty",
                "tools": ["read_file", "write_file", "search_files"],
                "write_paths": ["design/"]
            },
            {
                "role": "builder",
                "assignee": "agent",
                "depends_on": ["designer"],
                "handoff_contract": "可运行关卡脚本 + 资源清单",
                "tools": ["read_file", "write_file", "run_command"],
                "write_paths": ["levels/"]
            },
            {
                "role": "playtest",
                "assignee": "agent",
                "depends_on": ["builder"],
                "verify": "contains:可玩性",
                "capabilities": ["review"],
                "tools": ["read_file"]
            },
            {
                "role": "acceptance",
                "assignee": "human",
                "depends_on": ["playtest"],
                "handoff_contract": "人工验收结论（通过/需返工 + 理由）"
            }
        ]
    });
    format!(
        "好的，我给出如下配置：\n```json\n{}\n```\n如需调整请告诉我。",
        serde_json::to_string_pretty(&template).unwrap()
    )
}

#[tokio::test]
async fn draft_list_adopt_discard_round_trip() {
    let fixture = fixture(vec![good_reply()]);
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"做一个游戏关卡生成流水线，最后要人工验收","constraints":"必须有人工节点"}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true), "{body}");
    assert_eq!(
        body["template"]["template_id"],
        json!("level-design-pipeline")
    );
    assert_eq!(body["preview"]["roles"].as_array().unwrap().len(), 4);
    assert_eq!(body["preview"]["edges"].as_array().unwrap().len(), 3);
    // 生成结果带健康面：该夹具含 run_command 与人工节点 → risk=high、人工节点 1。
    assert_eq!(body["health"]["risk_level"], json!("high"), "{body}");
    assert_eq!(body["health"]["human_roles"], json!(1), "{body}");
    assert!(
        body["health"]["elevated_tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool == "run_command"),
        "{body}"
    );
    let draft_id = body["draft"]["draft_id"].as_str().unwrap().to_string();
    // 草稿已落盘（但不等于启用）。
    let draft_file = fixture
        .data_root
        .join("workswarm")
        .join("drafts")
        .join(format!("{draft_id}.json"));
    assert!(draft_file.exists(), "草稿应落盘：{draft_file:?}");

    // 列表可见且有效。
    let (status, listed) = call(&fixture.state, "GET", "/workflows/draft", None).await;
    assert_eq!(status, 200, "{listed}");
    assert_eq!(listed["count"], json!(1));
    assert_eq!(listed["drafts"][0]["valid"], json!(true));

    // 详情带模板与预览。
    let (status, detail) = call(
        &fixture.state,
        "GET",
        &format!("/workflows/draft/{draft_id}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(
        detail["preview"]["edges"][0],
        json!({"from": "designer", "to": "builder"})
    );

    // 采纳 → 进入用户模板列表。
    let (status, adopted) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/adopt"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{adopted}");
    assert_eq!(adopted["template"]["origin"], json!("user"));
    let (_, user_workflows) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(user_workflows["count"], json!(1));
    assert_eq!(
        user_workflows["templates"][0]["template"]["template_id"],
        json!("level-design-pipeline")
    );

    // 丢弃草稿。
    let (status, deleted) = call(
        &fixture.state,
        "DELETE",
        &format!("/workflows/draft/{draft_id}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{deleted}");
    let (status, _) = call(
        &fixture.state,
        "DELETE",
        &format!("/workflows/draft/{draft_id}"),
        None,
    )
    .await;
    assert_eq!(status, 404);
}

/// 从 good_reply 的 ```json 围栏里取出模板 JSON（夹具复用）。
fn good_template_value() -> Value {
    let text = good_reply();
    let start = text.find('{').expect("good_reply 应含 JSON");
    let end = text.rfind('}').expect("good_reply 应含 JSON");
    serde_json::from_str(&text[start..=end]).expect("good_reply 模板应可解析")
}

/// 第一版故意非法（未知工具），修复版为合法模板。
fn bad_then_good_replies() -> Arc<ScriptedProvider> {
    let mut broken = good_template_value();
    broken["roles"][0]["tools"] = json!(["not_a_tool"]);
    broken["template_id"] = json!("bad-draft");
    let broken_reply = format!("```json\n{}\n```", broken);
    ScriptedProvider::new(vec![broken_reply, good_reply()])
}

#[tokio::test]
async fn invalid_first_draft_is_repaired_once_with_host_errors() {
    let provider = bad_then_good_replies();
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"做一条视频流水线"}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true), "修复后应收敛到合法草稿：{body}");
    assert_eq!(body["attempts"], json!(2));
    assert_eq!(body["repaired"], json!(true));
    // 修复提示词必须真的带上宿主校验错误与上一版草稿。
    let last_user = provider.last_user.lock().unwrap().clone();
    assert!(last_user.contains("宿主校验错误"), "{last_user}");
    assert!(last_user.contains("not_a_tool"), "{last_user}");
    assert!(last_user.contains("bad-draft"), "{last_user}");
}

#[tokio::test]
async fn repair_is_bounded_to_one_attempt_when_still_invalid() {
    let mut broken = good_template_value();
    broken["roles"][0]["tools"] = json!(["not_a_tool"]);
    let broken_reply = format!("```json\n{}\n```", broken);
    let provider = ScriptedProvider::new(vec![broken_reply.clone(), broken_reply]);
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"坏配置"}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["attempts"], json!(2), "最多修复一次：{body}");
    assert_eq!(body["repaired"], json!(false));
}

/// 逐轮变好的回复序列：首轮 2 个错误、次轮 1 个错误、末轮合法。
fn improving_replies() -> Arc<ScriptedProvider> {
    let good = good_template_value();
    // 轮 1：未知工具 + 依赖不存在的角色（2 个错误）。
    let mut first = good.clone();
    first["template_id"] = json!("converge-flow");
    first["roles"][0]["tools"] = json!(["not_a_tool"]);
    first["roles"][1]["depends_on"] = json!(["ghost"]);
    // 轮 2：只剩未知工具（1 个错误）。
    let mut second = good.clone();
    second["template_id"] = json!("converge-flow");
    second["roles"][0]["tools"] = json!(["not_a_tool"]);
    // 轮 3：合法。
    let mut third = good.clone();
    third["template_id"] = json!("converge-flow");
    ScriptedProvider::new(vec![
        format!("```json\n{first}\n```"),
        format!("```json\n{second}\n```"),
        format!("```json\n{third}\n```"),
    ])
}

#[tokio::test]
async fn based_on_injects_real_run_evidence_and_rejects_conflicts() {
    let provider = ScriptedProvider::new(vec![good_reply()]);
    let fixture = fixture_with_provider(Arc::clone(&provider));
    // 先保存一条用户工作流（内容取自同一个 good_reply，校验通过）。
    let saved: Value = serde_json::from_str(
        owo_agent_core::workflow_draft::extract_json_object(&good_reply()).unwrap(),
    )
    .unwrap();
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&saved.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let template_id = body["template"]["template_id"]
        .as_str()
        .expect("保存后应有 id")
        .to_string();

    // 未知工作流 → 404（早失败，不烧模型）。
    let (status, missing) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"改进它","based_on":"no-such-flow"}"#),
    )
    .await;
    assert_eq!(status, 404, "{missing}");

    // based_on 与 refine_of 同时给 → 422（语义冲突）。
    let (status, conflict) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(
            &json!({
                "description": "改进它",
                "based_on": template_id,
                "refine_of": "draft-x"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 422, "{conflict}");

    // 尚未运行：不注入证据（也不编造成功率），但仍回灌现有配置。
    let (status, fresh) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(&json!({ "description": "改进它", "based_on": template_id }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{fresh}");
    assert_eq!(fresh["based_on"], json!(template_id));
    assert!(fresh["run_evidence"].is_null(), "{fresh}");
    let prompt = provider.last_user.lock().unwrap().clone();
    assert!(!prompt.contains("## 运行证据"), "{prompt}");
    // 没有证据时沿用原标签（不假装是在按证据改）。
    assert!(prompt.contains("## 现有草稿"), "{prompt}");

    // 跑一次（离线必失败）→ 证据进提示词，也在响应里透明返回。
    let (status, created) = call(
        &fixture.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "复现失败",
                "mode": "team",
                "template_id": template_id
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "{created}");
    let team_id = created["team_id"]
        .as_str()
        .or_else(|| created["team"]["team_id"].as_str())
        .unwrap_or_default()
        .to_string();
    let mut final_status = String::new();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, detail) = call(&fixture.state, "GET", &format!("/teams/{team_id}"), None).await;
        final_status = detail["team"]["status"].as_str().unwrap_or("").to_string();
        if matches!(final_status.as_str(), "failed" | "succeeded" | "cancelled") {
            break;
        }
    }
    assert_eq!(final_status, "failed", "离线环境应快速失败");

    let (status, with_evidence) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(&json!({ "description": "这条总失败，帮我改", "based_on": template_id }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{with_evidence}");
    let evidence = with_evidence["run_evidence"].as_str().unwrap_or_default();
    assert!(evidence.contains("运行 1 次"), "{with_evidence}");
    assert!(evidence.contains("从未成功"), "{with_evidence}");
    assert!(evidence.contains("最近一次失败"), "{with_evidence}");
    let prompt = provider.last_user.lock().unwrap().clone();
    assert!(prompt.contains("## 运行证据"), "{prompt}");
    assert!(prompt.contains("从未成功"), "{prompt}");

    // 收敛接口同样支持（多轮按证据改进），并在审计里留下"凭什么改"。
    let (status, converged) = call(
        &fixture.state,
        "POST",
        "/workflows/draft/converge",
        Some(
            &json!({
                "description": "按证据多轮改进",
                "based_on": template_id,
                "max_rounds": 2
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 200, "{converged}");
    assert_eq!(converged["based_on"], json!(template_id));
    assert!(
        converged["run_evidence"]
            .as_str()
            .unwrap_or("")
            .contains("运行 1 次"),
        "{converged}"
    );
    let entries: Vec<String> = fixture
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| {
            entry.event == "workflow_draft_generate" || entry.event == "workflow_draft_converge"
        })
        .map(|entry| entry.detail.clone())
        .collect();
    assert!(
        entries.iter().any(|detail| detail.contains("evidence=true")
            && detail.contains(&format!("based_on={template_id}"))),
        "{entries:?}"
    );
    // 未运行前的那次生成应记为 evidence=false（不冒充有证据）。
    assert!(
        entries
            .iter()
            .any(|detail| detail.contains("evidence=false")),
        "{entries:?}"
    );
}

#[tokio::test]
async fn adopt_returns_verify_plan_with_baseline_objective() {
    let provider = ScriptedProvider::new(vec![good_reply()]);
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let saved: Value = serde_json::from_str(
        owo_agent_core::workflow_draft::extract_json_object(&good_reply()).unwrap(),
    )
    .unwrap();
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&saved.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let template_id = body["template"]["template_id"]
        .as_str()
        .expect("保存后应有 id")
        .to_string();

    // 先跑一次（离线必失败）→ 形成基线。
    let (status, created) = call(
        &fixture.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "关卡流水线：先跑通",
                "mode": "team",
                "template_id": template_id
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "{created}");
    let team_id = created["team_id"]
        .as_str()
        .or_else(|| created["team"]["team_id"].as_str())
        .unwrap_or_default()
        .to_string();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, detail) = call(&fixture.state, "GET", &format!("/teams/{team_id}"), None).await;
        if matches!(
            detail["team"]["status"].as_str().unwrap_or(""),
            "failed" | "succeeded" | "cancelled"
        ) {
            break;
        }
    }

    // 不带 based_on 的草稿：采纳后没有基线 → verify 为 null（不编造对比）。
    let (status, plain) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"全新一条"}"#),
    )
    .await;
    assert_eq!(status, 200, "{plain}");
    let plain_id = plain["draft"]["draft_id"].as_str().unwrap().to_string();
    let (status, adopted_plain) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{plain_id}/adopt"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{adopted_plain}");
    assert!(adopted_plain["verify"].is_null(), "{adopted_plain}");

    // 带 based_on 的草稿：采纳后给出基线（目标/状态/需要修复）供验证对比。
    let (status, with_evidence) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(&json!({ "description": "按证据改进", "based_on": template_id }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{with_evidence}");
    let draft_id = with_evidence["draft"]["draft_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, adopted) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/adopt"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{adopted}");
    assert_eq!(adopted["based_on"], json!(template_id));
    let plan = adopted["verify"].as_object().expect("应有验证计划");
    assert_eq!(plan["previous_template_id"], json!(template_id));
    assert_eq!(plan["previous_status"], json!("failed"));
    assert_eq!(plan["objective"], json!("关卡流水线：先跑通"));
    assert_eq!(plan["same_template_id"], json!(true));
    assert_eq!(plan["adopted_template_id"], json!(template_id));
    let attention: Vec<&str> = plan["previous_attention"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(
        attention.iter().any(|reason| reason.contains("从未成功")),
        "{attention:?}"
    );

    // 审计记录"基于谁 + 是否待验证"。
    let details: Vec<String> = fixture
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.event == "workflow_draft_adopt")
        .map(|entry| entry.detail.clone())
        .collect();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("待验证=true") && detail.contains("基于=")),
        "{details:?}"
    );
}

#[tokio::test]
async fn verify_outcome_is_written_back_and_fed_into_next_evidence() {
    let provider = ScriptedProvider::new(vec![good_reply()]);
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let saved: Value = serde_json::from_str(
        owo_agent_core::workflow_draft::extract_json_object(&good_reply()).unwrap(),
    )
    .unwrap();
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/user",
        Some(&saved.to_string()),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let template_id = body["template"]["template_id"]
        .as_str()
        .expect("保存后应有 id")
        .to_string();

    // 基线运行（离线必失败）。
    let (status, created) = call(
        &fixture.state,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "基线目标",
                "mode": "team",
                "template_id": template_id
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "{created}");
    let baseline_team = created["team_id"]
        .as_str()
        .or_else(|| created["team"]["team_id"].as_str())
        .unwrap_or_default()
        .to_string();
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let (_, detail) = call(
            &fixture.state,
            "GET",
            &format!("/teams/{baseline_team}"),
            None,
        )
        .await;
        if matches!(
            detail["team"]["status"].as_str().unwrap_or(""),
            "failed" | "succeeded" | "cancelled"
        ) {
            break;
        }
    }

    // 基于证据生成 → 采纳 → 造一次"验证运行"（同目标）→ 回写结果。
    let (status, with_evidence) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(&json!({ "description": "按证据改进", "based_on": template_id }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{with_evidence}");
    let draft_id = with_evidence["draft"]["draft_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, adopted) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/adopt"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 200, "{adopted}");

    let (status, verified) = call(
        &fixture.state,
        "POST",
        &format!("/teams"),
        Some(
            &json!({
                "objective": "基线目标",
                "mode": "team",
                "template_id": template_id
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "{verified}");
    let verify_team = verified["team_id"]
        .as_str()
        .or_else(|| verified["team"]["team_id"].as_str())
        .unwrap_or_default()
        .to_string();

    // 缺 team_id → 400；未知 team → 404（不编造结果）。
    let (status, bad) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/verify"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 400, "{bad}");
    let (status, missing) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/verify"),
        Some(r#"{"team_id":"team-nope"}"#),
    )
    .await;
    assert_eq!(status, 404, "{missing}");

    let (status, written) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/verify"),
        Some(
            &json!({
                "team_id": verify_team,
                "objective": "基线目标",
                "compared_status": "failed"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 200, "{written}");
    assert_eq!(
        written["verify_outcome"]["compared_status"],
        json!("failed"),
        "{written}"
    );
    assert_eq!(
        written["draft"]["verify_outcome"]["team_id"],
        json!(verify_team)
    );
    // 未显式给 status → 从团队记录读取（不编造）。
    assert!(written["verify_outcome"]["status"].is_string(), "{written}");

    let (status, listed) = call(&fixture.state, "GET", "/workflows/draft", None).await;
    assert_eq!(status, 200, "{listed}");
    let item = listed["drafts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["draft_id"] == json!(draft_id))
        .expect("草稿仍在列表");
    assert_eq!(item["verify_outcome"]["compared_status"], json!("failed"));

    // 下一轮 based_on：证据里应带上"上次按证据改进 → 验证"的反馈。
    let (status, next) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(&json!({ "description": "再改一轮", "based_on": template_id }).to_string()),
    )
    .await;
    assert_eq!(status, 200, "{next}");
    let evidence = next["run_evidence"].as_str().unwrap_or_default();
    assert!(
        evidence.contains("上次按证据改进后用同一目标验证"),
        "{next}"
    );
    let prompt = provider.last_user.lock().unwrap().clone();
    assert!(
        prompt.contains("上次按证据改进后用同一目标验证"),
        "{prompt}"
    );

    let events: Vec<String> = fixture
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.event == "workflow_draft_verify")
        .map(|entry| entry.detail.clone())
        .collect();
    assert_eq!(events.len(), 1, "回写应审计一条：{events:?}");
    assert!(events[0].contains("基线=failed"), "{events:?}");
}

#[tokio::test]
async fn converge_accepts_only_improving_rounds_and_stops_when_valid() {
    let provider = improving_replies();
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft/converge",
        Some(r#"{"description":"做一条关卡流水线","max_rounds":5}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true), "末轮应合法：{body}");
    let trajectory = body["trajectory"].as_array().unwrap();
    assert_eq!(trajectory.len(), 3, "应在第三轮通过后停止：{body}");
    assert_eq!(trajectory[0]["errors"], json!(2));
    assert_eq!(trajectory[1]["errors"], json!(1));
    assert_eq!(trajectory[2]["valid"], json!(true));
    assert_eq!(body["rounds"], json!(3));
    // 每轮都带风险等级（"通过但风险不低"必须可见）。
    assert!(
        trajectory.iter().all(|step| step["risk_level"].is_string()),
        "{body}"
    );
    // 每轮都带风险等级、"通过但风险不低"可见：夹具含 run_command + 人工节点 → high。
    assert_eq!(body["health"]["risk_level"], json!("high"), "{body}");
    assert_eq!(body["health"]["human_roles"], json!(1), "{body}");
    // 落盘草稿与轨迹一致。
    let draft_id = body["draft"]["draft_id"].as_str().unwrap().to_string();
    let (status, detail) = call(
        &fixture.state,
        "GET",
        &format!("/workflows/draft/{draft_id}"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["errors"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn converge_audit_records_rounds_errors_and_risk() {
    let provider = improving_replies();
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let (status, _) = call(
        &fixture.state,
        "POST",
        "/workflows/draft/converge",
        Some(r#"{"description":"做一条关卡流水线","max_rounds":5}"#),
    )
    .await;
    assert_eq!(status, 200);
    let entries: Vec<(Option<bool>, String)> = fixture
        .state
        .agent
        .audit_log()
        .lock()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.event == "workflow_draft_converge")
        .map(|entry| (entry.approved, entry.detail.clone()))
        .collect();
    assert_eq!(entries.len(), 1, "应收敛审计一条");
    assert_eq!(entries[0].0, Some(true));
    assert!(entries[0].1.contains("轮次=3"), "{entries:?}");
    assert!(entries[0].1.contains("风险="), "{entries:?}");
}

#[tokio::test]
async fn converge_stops_early_when_a_round_does_not_improve() {
    // 每轮都同样错（2 个错误）→ 第 2 轮"未改善"即停，保留第一版。
    let good = good_template_value();
    let mut broken = good.clone();
    broken["template_id"] = json!("stuck-flow");
    broken["roles"][0]["tools"] = json!(["not_a_tool"]);
    broken["roles"][1]["depends_on"] = json!(["ghost"]);
    let reply = format!("```json\n{broken}\n```");
    let provider = ScriptedProvider::new(vec![reply.clone(), reply.clone(), reply]);
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft/converge",
        Some(r#"{"description":"卡住的配置","max_rounds":5}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(false));
    let trajectory = body["trajectory"].as_array().unwrap();
    assert_eq!(
        trajectory.len(),
        2,
        "未改善应立刻停止（省模型调用）：{body}"
    );
    assert_eq!(trajectory[1]["accepted"], json!(false));
    assert_eq!(body["errors"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn converge_requires_description_and_clamps_rounds() {
    let provider = ScriptedProvider::new(vec![good_reply()]);
    let fixture = fixture_with_provider(Arc::clone(&provider));
    let (status, _) = call(
        &fixture.state,
        "POST",
        "/workflows/draft/converge",
        Some(r#"{"description":"   "}"#),
    )
    .await;
    assert_eq!(status, 400);

    // max_rounds 超上限按硬顶处理（合法配置第一轮就结束）。
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft/converge",
        Some(r#"{"description":"一次就过","max_rounds":99}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["rounds"], json!(1));
}

#[tokio::test]
async fn reply_without_json_is_rejected_honestly() {
    let fixture = fixture(vec!["我现在还不确定，你能再说明一下吗？".to_string()]);
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"随便做个什么"}"#),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("没有 JSON 对象"),
        "{body}"
    );
    let (_, listed) = call(&fixture.state, "GET", "/workflows/draft", None).await;
    assert_eq!(listed["count"], json!(0), "失败的生成不应留下草稿");
}

#[tokio::test]
async fn structurally_invalid_json_is_rejected_with_excerpt() {
    let fixture = fixture(vec!["```json\n{\"template_id\": 42}\n```".to_string()]);
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"结构不对的返回"}"#),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    let message = body["error"].as_str().unwrap();
    assert!(message.contains("不符合 TeamTemplate 结构"), "{message}");
    assert!(message.contains("原文片段"), "{message}");
}

#[tokio::test]
async fn draft_with_validation_errors_can_be_stored_but_not_adopted() {
    let mut template: Value = serde_json::from_str(
        &serde_json::to_string(&json!({
            "template_id": "bad-flow",
            "name": "有问题的流水线",
            "mode": "team",
            "applicability": "测试",
            "description": "角色依赖成环 + 未知工具",
            "roles": [
                {"role": "a", "assignee": "agent", "depends_on": ["b"], "tools": ["not_a_tool"]},
                {"role": "b", "assignee": "agent", "depends_on": ["a"]}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    template["created_at"] = json!("");
    let fixture = fixture(vec![format!("```json\n{}\n```", template)]);
    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"会成环的配置"}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(false), "校验应报错");
    let errors = body["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|item| item.as_str().unwrap().contains("环")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|item| item.as_str().unwrap().contains("未知工具")),
        "{errors:?}"
    );
    let draft_id = body["draft"]["draft_id"].as_str().unwrap().to_string();

    // 采纳必须被拒（422），且不写入用户模板。
    let (status, rejected) = call(
        &fixture.state,
        "POST",
        &format!("/workflows/draft/{draft_id}/adopt"),
        Some("{}"),
    )
    .await;
    assert_eq!(status, 422, "{rejected}");
    let (_, user_workflows) = call(&fixture.state, "GET", "/workflows/user", None).await;
    assert_eq!(user_workflows["count"], json!(0));
}

#[tokio::test]
async fn refine_reuses_previous_draft_and_missing_description_is_400() {
    let fixture = fixture(vec![good_reply()]);
    let (_, first) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"关卡流水线"}"#),
    )
    .await;
    let first_id = first["draft"]["draft_id"].as_str().unwrap().to_string();

    let (status, refined) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(&format!(
            r#"{{"description":"把校验角色加强","refine_of":"{first_id}"}}"#
        )),
    )
    .await;
    assert_eq!(status, 200, "{refined}");
    assert_eq!(refined["draft"]["refine_of"], json!(first_id));
    let (_, listed) = call(&fixture.state, "GET", "/workflows/draft", None).await;
    assert_eq!(listed["count"], json!(2));

    let (status, body) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"   "}"#),
    )
    .await;
    assert_eq!(status, 400, "{body}");

    // 引用不存在的草稿 → 404。
    let (status, _) = call(
        &fixture.state,
        "POST",
        "/workflows/draft",
        Some(r#"{"description":"x","refine_of":"no-such-draft"}"#),
    )
    .await;
    assert_eq!(status, 404);
}

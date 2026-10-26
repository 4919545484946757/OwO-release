//! 内置团队模板目录集成测试（六期 · 第三路）。
//!
//! 断面：① 目录只展示五类候选（未安装不进入注册表）；② 安装幂等且非破坏
//! （重复安装不覆盖用户定制）；③ **自动模式只匹配已安装模板**（安装前不命中、
//! 安装后 applicability 关键词命中即自动建队）；④ 模板建队后角色数/成员/预算与
//! 模板一致（DAG 持久化保真在 core 模块单测覆盖）；⑤ 安装只新增模板文件，
//! 不触碰任何权限/设置面（不自动扩权）。
//!
//! 目录路由以 `#[path] mod` 独立编译（usage.rs 先例；正式 lib.rs 接线归第四路），
//! 建队/查询走 `build_router` 真实路由。全部使用内置执行路径，不依赖模型凭据。

#[path = "../src/team_template_catalog_api.rs"]
mod team_template_catalog_api;

use owo_agent_core::gateway::ModelProvider;
use owo_agent_core::permissions::Policy;
use owo_agent_core::sqlite_store::SqliteSessionStore;
use owo_agent_core::tools::ToolRegistry;
use owo_agent_core::Agent;
use owo_agent_server::build_router;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

/// 无外部依赖的最小模型 Provider（模板角色为 agent 驱动，测试进程无凭据即快速失败，
/// 断言只针对创建响应与目录面，不受运行结局影响）。
struct IdleProvider;

#[async_trait::async_trait]
impl ModelProvider for IdleProvider {
    async fn complete(
        &self,
        _messages: &[owo_agent_core::ChatMessage],
        _tools: &[owo_agent_core::ToolSpec],
    ) -> Result<owo_agent_core::ModelOutput, String> {
        Err("IdleProvider 不应被调用".to_string())
    }
}

async fn test_state() -> (Arc<owo_agent_server::AppState>, tempfile::TempDir) {
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

async fn call(
    state: &Arc<owo_agent_server::AppState>,
    app: &axum::Router,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (u16, Value) {
    let resp = app
        .clone()
        .oneshot(request(state, method, path, body))
        .await
        .unwrap();
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

fn dir_entries(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn catalog_lists_six_builtins_as_candidates() {
    let (state, _temp) = test_state().await;
    let catalog_router = team_template_catalog_api::team_template_catalog_router(state.clone());
    let (status, body) = call(
        &state,
        &catalog_router,
        "GET",
        "/teams/templates/catalog",
        None,
    )
    .await;
    assert_eq!(status, 200, "目录应 200：{body}");
    let entries = body["catalog"].as_array().expect("catalog 数组");
    assert_eq!(entries.len(), 6, "六类内置模板：{entries:?}");
    let ids: Vec<&str> = entries
        .iter()
        .map(|e| e["template"]["template_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "code-change-v1",
            "code-change-v2",
            "fullstack-web-v1",
            "research-brief-v1",
            "document-delivery-v1",
            "structured-extract-v1"
        ]
    );
    assert_eq!(
        body["installed_count"],
        json!(0),
        "全新数据目录：全部未安装"
    );
    for e in entries {
        assert_eq!(e["installed"], json!(false));
        let roles = e["template"]["roles"].as_array().unwrap();
        assert!(!roles.is_empty());
        // 预算/完成条件/工具范围/关键词齐全（UI 预览面）。
        assert!(e["budget"]["max_steps"].as_u64().unwrap_or(0) >= roles.len() as u64);
        assert!(!e["completion_criteria"].as_array().unwrap().is_empty());
        assert!(!e["tool_scope"].as_str().unwrap_or("").is_empty());
        assert!(!e["auto_match_keywords"].as_array().unwrap().is_empty());
        assert_eq!(
            e["budget_calls_per_role"].as_array().unwrap().len(),
            roles.len(),
            "每角色都有调用预算"
        );
        // DAG 闭合：依赖都在角色集内。
        let role_names: Vec<&str> = roles.iter().map(|r| r["role"].as_str().unwrap()).collect();
        for r in roles {
            for dep in r["depends_on"].as_array().unwrap() {
                assert!(
                    role_names.contains(&dep.as_str().unwrap()),
                    "依赖缺失：{} → {dep}",
                    r["role"]
                );
            }
        }
    }
}

#[tokio::test]
async fn install_is_idempotent_and_enables_auto_match_only_after_install() {
    let (state, temp) = test_state().await;
    let app = build_router(state.clone());
    let catalog_router = team_template_catalog_api::team_template_catalog_router(state.clone());

    // ① 安装前：自动模式不命中任何内置模板（目录只是候选）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "重构登录模块的代码",
                "mode": "team",
                "strategy": "team"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "建队应 202：{body}");
    assert_eq!(
        body["template_id"],
        json!(null),
        "安装前自动匹配不得命中内置模板"
    );

    // ② 手动安装（幂等性第一段：首次 installed=true）。
    let (status, body) = call(
        &state,
        &catalog_router,
        "POST",
        "/teams/templates/catalog/code-change-v1/install",
        None,
    )
    .await;
    assert_eq!(status, 200, "安装应 200：{body}");
    assert_eq!(body["installed"], json!(true));
    assert_eq!(body["already_installed"], json!(false));
    assert_eq!(body["template"]["template_id"], json!("code-change-v1"));
    // 注册表根与协调器一致（workswarm/templates 下再嵌 templates/）。
    let tpl_path = temp
        .path()
        .join("workswarm/templates/workflow-templates.sqlite3");
    assert!(tpl_path.exists(), "安装应落盘注册表模板文件：{tpl_path:?}");

    // ③ 重复安装：幂等且非破坏（不覆盖，返回现状）。
    let (status, body) = call(
        &state,
        &catalog_router,
        "POST",
        "/teams/templates/catalog/code-change-v1/install",
        None,
    )
    .await;
    assert_eq!(status, 200, "重复安装应 200：{body}");
    assert_eq!(body["installed"], json!(false));
    assert_eq!(body["already_installed"], json!(true));
    assert_eq!(
        body["template"]["template_id"],
        json!("code-change-v1"),
        "重复安装返回注册表现状"
    );
    let registry = owo_agent_core::workswarm::TeamTemplateRegistry::for_workswarm_root(
        &temp.path().join("workswarm"),
    );
    let current = registry.get_revision("code-change-v1").unwrap().unwrap();
    assert_eq!(current.revision, 1, "重复安装不得创建额外修订");
    assert_eq!(current.template.unwrap().template_id, "code-change-v1");
    assert_eq!(
        registry
            .revision_history("code-change-v1", None, 10)
            .unwrap()
            .len(),
        1,
        "重启读取规范存储时仍只有初次安装修订"
    );

    // ④ 目录反映安装态。
    let (status, body) = call(
        &state,
        &catalog_router,
        "GET",
        "/teams/templates/catalog",
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["installed_count"], json!(1));
    let entry = body["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["template"]["template_id"] == json!("code-change-v1"))
        .unwrap();
    assert_eq!(entry["installed"], json!(true));

    // ⑤ 安装后：自动模式按 applicability 关键词命中已安装模板。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "重构支付模块的代码",
                "mode": "team",
                "strategy": "team"
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "建队应 202：{body}");
    assert_eq!(
        body["template_id"],
        json!("code-change-v1"),
        "安装后自动匹配应命中 code-change-v1"
    );
    // 八期（一路自适应 DAG）：简单代码任务先保留 reviewer（变更状态未知时不得
    // 提前跳过独立评审），运行期由宿主观测到"无实际变更"后才裁剪；建队时仍为
    // analyzer + implementer + reviewer 三角色。
    assert_eq!(
        body["members"].as_array().unwrap().len(),
        3,
        "模板角色数（reviewer 延迟到宿主观测后再裁剪）"
    );
}

#[tokio::test]
async fn install_unknown_template_404() {
    let (state, _temp) = test_state().await;
    let catalog_router = team_template_catalog_api::team_template_catalog_router(state.clone());
    let (status, body) = call(
        &state,
        &catalog_router,
        "POST",
        "/teams/templates/catalog/no-such-template/install",
        None,
    )
    .await;
    assert_eq!(status, 404, "未知模板应 404：{body}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("候选目录"),
        "错误文案应指向候选目录：{body}"
    );
}

#[tokio::test]
async fn team_from_template_matches_roles_and_budget() {
    let (state, _temp) = test_state().await;
    let app = build_router(state.clone());
    let catalog_router = team_template_catalog_api::team_template_catalog_router(state.clone());

    // 安装结构化抽取模板并取目录预算。
    let (status, _) = call(
        &state,
        &catalog_router,
        "POST",
        "/teams/templates/catalog/structured-extract-v1/install",
        None,
    )
    .await;
    assert_eq!(status, 200);
    let (status, body) = call(
        &state,
        &catalog_router,
        "GET",
        "/teams/templates/catalog",
        None,
    )
    .await;
    assert_eq!(status, 200);
    let entry = body["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["template"]["template_id"] == json!("structured-extract-v1"))
        .expect("已安装模板应在目录中");
    let template_budget = entry["budget"].clone();
    // The launcher omits this opt-in field; the persisted budget must default to false.
    let mut expected_budget = template_budget.clone();
    expected_budget["allow_scoped_writes"] = json!(false);
    let expected_roles: Vec<String> = entry["template"]["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["role"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        expected_roles,
        vec!["extractor", "schema_validator", "artifact_formatter"]
    );

    // 显式 template_id 建队，预算随目录值传入（launcher/UI 预填口径）。
    let (status, body) = call(
        &state,
        &app,
        "POST",
        "/teams",
        Some(
            &json!({
                "objective": "从表单截图抽取字段并清洗",
                "mode": "team",
                "template_id": "structured-extract-v1",
                "budget": template_budget
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(status, 202, "模板建队应 202：{body}");
    assert_eq!(body["template_id"], json!("structured-extract-v1"));
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), expected_roles.len(), "角色数与模板一致");
    let mut member_roles: Vec<&str> = members
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    member_roles.sort();
    let mut want_roles = expected_roles.clone();
    want_roles.sort();
    assert_eq!(member_roles, want_roles, "成员角色与模板一致");

    // 预算与模板一致（GET /teams/{id} 透传 TeamRun.budget）。
    let team_id = body["team_id"].as_str().unwrap().to_string();
    let (status, body) = call(&state, &app, "GET", &format!("/teams/{team_id}"), None).await;
    assert_eq!(status, 200);
    assert_eq!(
        body["team"]["template_id"],
        json!("structured-extract-v1"),
        "团队应记录来源模板"
    );
    assert_eq!(
        body["team"]["budget"], expected_budget,
        "预算与模板目录值逐字段一致"
    );

    // 任务视图：每角色一个步骤（角色 → 步骤映射保真）。
    let tasks = body["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), expected_roles.len());
    for role in &expected_roles {
        assert!(
            tasks
                .iter()
                .any(|t| t["task_id"] == json!(format!("s-{role}"))),
            "缺少步骤 s-{role}：{tasks:?}"
        );
    }
}

#[tokio::test]
async fn install_touches_no_permission_surfaces() {
    let (state, temp) = test_state().await;
    let catalog_router = team_template_catalog_api::team_template_catalog_router(state.clone());

    // 目录浏览只准备注册表目录结构（注册表根 workswarm/templates 下自建
    // templates/ + proposals/），无其他副作用（协调器未被初始化：
    // 无 space.db/cas/runs）。
    let (status, _) = call(
        &state,
        &catalog_router,
        "GET",
        "/teams/templates/catalog",
        None,
    )
    .await;
    assert_eq!(status, 200);
    let swarm_dir = temp.path().join("workswarm");
    assert_eq!(
        dir_entries(&swarm_dir),
        vec!["templates"],
        "目录浏览不得触碰权限/设置面（协调器未初始化）"
    );
    let registry_root = swarm_dir.join("templates");
    assert_eq!(
        dir_entries(&registry_root),
        vec!["proposals", "templates", "workflow-templates.sqlite3"],
        "注册表根只含注册表自身目录与修订数据库"
    );

    // 安装只新增一个模板修订，且不扩大任何权限面。
    let (status, _) = call(
        &state,
        &catalog_router,
        "POST",
        "/teams/templates/catalog/document-delivery-v1/install",
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        dir_entries(&swarm_dir),
        vec!["templates"],
        "安装不得新增权限/设置文件"
    );
    assert_eq!(
        dir_entries(&registry_root.join("templates")),
        Vec::<String>::new(),
        "新模板保存在事务注册表，不增加旧版 JSON 文件"
    );
    let registry = owo_agent_core::workswarm::TeamTemplateRegistry::new(registry_root.clone());
    let stored =
        serde_json::to_value(registry.get_template("document-delivery-v1").unwrap()).unwrap();
    assert_eq!(
        registry
            .get_revision("document-delivery-v1")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(stored["template_id"], json!("document-delivery-v1"));
    // 模板角色不携带任何工具白名单/写权限（权限仍走运行时审批策略）。
    for role in stored["roles"].as_array().unwrap() {
        assert!(
            role.get("tool_scope").is_none() && role.get("write_scope").is_none(),
            "模板角色不得内嵌权限授权：{role}"
        );
    }
}

#[tokio::test]
async fn install_requires_the_reviewed_tombstone_revision_and_never_overwrites_a_customization() {
    let (state, temp) = test_state().await;
    let app = team_template_catalog_api::team_template_catalog_router(state.clone());
    let path = "/teams/templates/catalog/code-change-v1/install";
    let (status, first) = call(&state, &app, "POST", path, None).await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["revision"], 1);
    let registry = owo_agent_core::workswarm::TeamTemplateRegistry::for_workswarm_root(
        &temp.path().join("workswarm"),
    );
    registry
        .delete_template_checked("code-change-v1", Some(1))
        .unwrap();
    let (status, conflict) = call(&state, &app, "POST", path, None).await;
    assert_eq!(status, 409, "{conflict}");
    assert_eq!(conflict["current_revision"], 2);
    assert!(registry
        .get_revision("code-change-v1")
        .unwrap()
        .unwrap()
        .template
        .is_none());
    let (_, catalog) = call(&state, &app, "GET", "/teams/templates/catalog", None).await;
    let entry = catalog["catalog"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["template"]["template_id"] == "code-change-v1")
        .unwrap();
    assert_eq!(entry["revision"], 2);
    assert_eq!(entry["installed"], false);
    let (status, installed) = call(
        &state,
        &app,
        "POST",
        &format!("{path}?expected_revision=2"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{installed}");
    assert_eq!(installed["revision"], 3);
    let mut custom = registry.get_template("code-change-v1").unwrap();
    custom.name = "User customization".into();
    custom.origin = Some("user".into());
    registry.save_template_checked(&custom, Some(3)).unwrap();
    let (status, replay) = call(
        &state,
        &app,
        "POST",
        &format!("{path}?expected_revision=3"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{replay}");
    assert_eq!(replay["already_installed"], true);
    assert_eq!(replay["template"]["name"], "User customization");
    assert_eq!(replay["revision"], 4);
    assert_eq!(
        registry
            .revision_history("code-change-v1", None, 100)
            .unwrap()
            .len(),
        4
    );
}

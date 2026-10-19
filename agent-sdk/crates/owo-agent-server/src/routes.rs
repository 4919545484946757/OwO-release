//! HTTP 路由装配表（自 `lib.rs` 机械提取，单一职责：路由注册）。
//!
//! 契约测试按行扫描本文件与 `lib.rs` 的 route 声明；每条路由保持
//! 单行声明（`/session/{id}` 的 GET+DELETE 用变量绕开折行）。修改路由必须同步
//! `openapi.rs`、`tests/route_contract_tests.rs` 与 `clients/ts/openapi.json`。
use super::desktop_model_credentials::register as register_model_credential;
use super::*;

pub fn build_router(state: Arc<AppState>) -> Router {
    // R7：SSE→可观测性指标桥接（Agent 4 钩子）：/events/stream 的采样样本
    // 转发到 observability_api（/metrics/runtime 呈现真实运行期数值）；
    // SLO 报告探针注册（/metrics/slo 反映全局 SLO 状态）。幂等：重复调用仅替换。
    event_stream::set_metrics_observer(Box::new(|sample| {
        observability_api::ingest_metrics_sample(&sample.to_json());
    }));
    observability_api::register_slo_report_probe(std::sync::Arc::new(slo::report_global));
    // R12（Agent 4 交付，主控接线）：用量/SLO 告警/SLO 周期报告探针注册，
    // 使 /metrics/prometheus 用量指标、/metrics/slo/alerts、/metrics/slo/report 返回真实数据
    // （此前仅注册 slo_report_probe，其余探针为未注册空 stub）。
    observability_api::register_usage_probe(std::sync::Arc::new(|| usage::global().summary()));
    observability_api::register_slo_alerts_probe(std::sync::Arc::new(|| slo::alerts_json(50)));
    observability_api::register_slo_period_probe(std::sync::Arc::new(slo::report_period_global));
    // R9 主控接线收尾：SLO 告警监听器转发到可靠事件流（/events/stream 收到 alert 事件）。
    // 触发源为 `slo::check_alerts_global`（数据面）；未评估时不产生事件，无副作用。
    slo::set_alert_listener(Box::new(|event| {
        let trace_id = event.trace_id.clone();
        let data = serde_json::to_string(event).unwrap_or_default();
        event_stream::hub().publish_alert(data, trace_id);
    }));
    // 公开面：健康检查 / OpenAPI / token 引导（发布桌面模式下 token handler 额外验证配对证明）。
    let public = Router::new()
        .route("/health", get(health))
        .route("/openapi.json", get(openapi::openapi_spec))
        .route("/auth/token", get(auth_token::auth_token_bootstrap))
        .with_state(state.clone());
    // 保护面：全部业务 API（bearer token 鉴权 + 双令牌桶限流）。
    //
    // 注意 `/session/{id}` 的 GET+DELETE：两个方法链在一起会超出 rustfmt 行宽而被折成
    // 多行，而契约测试 route_contract_tests.rs 是**按行**扫描路由声明的——
    // 折行会让该路径从注册清单里消失、断言失败。所以先构造 MethodRouter 变量，
    // 让路径与 route 调用保持单行。（这段注释本身也不能出现形如 route 加引号的字面量，
    // 否则会被那个按行扫描的提取器当成一条真实路由，实测让契约测试报出假漏登记。）
    let session_detail = get(session_api::get_session).delete(session_api::delete_session);
    let protected = Router::new()
        .route("/expression", post(expression_api::expression))
        .route("/audit", get(audit_api::audit_list))
        .route("/session", post(session_api::create_session))
        .route("/session/{id}", session_detail)
        .route("/session/{id}/turn", post(turn_api::turn))
        .route("/session/{id}/turn/events", get(turn_api::turn_events))
        .route(
            "/session/{id}/attachments",
            get(session_api::attachments_list),
        )
        .route(
            "/session/{id}/attachments",
            post(session_api::attachment_upload).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route(
            "/session/{id}/permission/{request_id}",
            post(turn_api::respond_permission),
        )
        // ask_user 应答（取优合并自远端 engine）：把答案送回挂起中的回合。
        .route(
            "/session/{id}/answer/{question_id}",
            post(session_api::respond_question),
        )
        // 全权限模式（运行时开关）：写开关文件 + 读当前状态。界面在输入框下面切换，
        // 不需要重启核心（见 turn_api::auto_approve_enabled 的注释）。
        .route(
            "/approval/mode",
            get(turn_api::approval_mode_get).post(turn_api::approval_mode_set),
        )
        .route("/session/{id}/abort", post(session_api::abort_turn))
        .route("/session/{id}/diff", get(session_api::diff))
        .route("/session/{id}/hunks", get(session_hunks_api::session_hunks))
        .route(
            "/session/{id}/hunks/apply",
            post(session_hunks_api::session_hunks_apply),
        )
        .route("/session/{id}/revert", post(session_api::revert))
        .route("/session/{id}/fork", post(session_api::fork_session))
        .route("/session/{id}/rewind", post(session_api::rewind_session))
        .route("/session/{id}/redo", post(session_api::redo_session))
        .route("/session/{id}/rename", post(session_api::session_rename))
        .route("/session/{id}/archive", post(session_api::session_archive))
        .route("/session/{id}/pin", post(session_api::session_pin))
        .route("/session/{id}/model", post(session_api::session_set_model))
        .route("/session/{id}/children", get(session_api::children))
        .route(
            "/session/{id}/export/{format}",
            get(session_api::export_session),
        )
        .route("/sessions", get(session_api::list_sessions))
        .route("/skills", get(skills_api::list_skills))
        .route(
            "/skills/{name}",
            get(skills_api::skill_detail).post(skills_api::skill_edit),
        )
        .route("/skills/{name}/enabled", post(skills_api::skill_enabled))
        .route("/eval/run", post(eval_api::run_eval))
        .route("/context/snapshot", get(session_api::context_snapshot))
        .route("/perception/events", get(perception_api::perception_events))
        .route(
            "/perception/capture",
            post(perception_api::perception_capture),
        )
        .route(
            "/perception/layers",
            post(perception_api::perception_layers),
        )
        .route("/perception/tree", post(perception_api::perception_tree))
        .route(
            "/perception/template/build",
            post(perception_api::perception_template_build),
        )
        .route(
            "/perception/template/build-ocr",
            post(perception_api::perception_template_build_ocr),
        )
        .route(
            "/perception/template/detect",
            post(perception_api::perception_template_detect),
        )
        .route(
            "/perception/template/detect-ocr",
            post(perception_api::perception_template_detect_ocr),
        )
        .route(
            "/perception/elements",
            post(perception_api::perception_elements),
        )
        .route(
            "/perception/template/{app_id}",
            get(perception_api::perception_template_get),
        )
        .route("/perception/ocr", post(perception_api::perception_ocr))
        .route(
            "/perception/ocr/bytes",
            post(perception_api::perception_ocr_bytes),
        )
        .route("/perception/ocr/status", get(perception_api::ocr_status))
        .route(
            "/perception/ocr/region",
            post(perception_api::perception_ocr_region),
        )
        .route(
            "/perception/window",
            post(perception_api::perception_window),
        )
        .route("/desktop/foreground", get(desktop_api::desktop_foreground))
        .route("/desktop/windows", get(desktop_api::desktop_windows))
        .route("/desktop/activate", post(desktop_api::desktop_activate))
        .route("/desktop/click", post(desktop_api::desktop_click))
        .route("/desktop/type", post(desktop_api::desktop_type))
        .route("/desktop/key", post(desktop_api::desktop_key))
        .route("/desktop/shortcut", post(desktop_api::desktop_shortcut))
        .route("/desktop/launch", post(desktop_api::desktop_launch))
        .route("/desktop/scroll", post(desktop_api::desktop_scroll))
        .route("/desktop/wait", post(desktop_api::desktop_wait))
        .route("/vision/status", get(desktop_api::vision_status))
        .route("/vision/describe", post(desktop_api::vision_describe))
        .route("/vision/verify", post(desktop_api::vision_verify))
        .route("/vision/ground", post(desktop_api::vision_ground))
        .route("/memory/observations", get(memory_api::memory_observations))
        .route("/memory/clear", post(memory_api::memory_clear))
        .route("/memory/mine-skill", post(memory_api::memory_mine_skill))
        .route("/learn/start", post(learn_api::learn_start))
        .route("/learn/record", post(learn_api::learn_record))
        .route("/learn/pause", post(learn_api::learn_pause))
        .route("/learn/resume", post(learn_api::learn_resume))
        .route("/learn/stop", post(learn_api::learn_stop))
        .route("/learn/clear", post(learn_api::learn_clear))
        .route("/learn/status", get(learn_api::learn_status))
        .route("/learn/execute", post(learn_api::learn_execute))
        .route("/learn/packages", get(learn_api::learn_packages))
        .route(
            "/learn/packages/{name}",
            get(learn_api::learn_package_detail).delete(learn_api::learn_package_delete),
        )
        .route("/learn/sink", post(learn_api::learn_sink))
        .route(
            "/learn/execute-package",
            post(learn_api::learn_execute_package),
        )
        .route("/learn/export/{name}", get(learn_api::learn_export))
        .route(
            "/learn/import",
            post(learn_api::learn_import).layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route("/skill/verify", post(learn_api::skill_verify))
        .route("/proactive/observe", post(assist_api::proactive_observe))
        .route("/proactive/decide", post(assist_api::proactive_decide))
        .route(
            "/proactive/suggestions",
            get(assist_api::proactive_suggestions),
        )
        .route(
            "/stt/transcribe",
            post(assist_api::stt_transcribe).layer(DefaultBodyLimit::max(25 * 1024 * 1024)),
        )
        .route("/automations", get(assist_api::automations_list))
        .route("/automations", post(assist_api::automations_create))
        // A8-1（取优合并自远端 engine）：执行记录查询。
        .route("/automations/runs", get(assist_api::automations_runs))
        .route(
            "/automations/{id}/toggle",
            post(assist_api::automations_toggle),
        )
        .route(
            "/automations/{id}",
            axum::routing::delete(assist_api::automations_delete),
        )
        .route(
            "/automations/reminders",
            get(assist_api::automations_reminders),
        )
        .route(
            "/automations/reminders/clear",
            post(assist_api::automations_clear_reminders),
        )
        // A8-2 / A8-3（取优合并自远端 engine）：活跃回合快照 / 桌宠显隐 / 跨会话待审批。
        .route("/activity", get(activity_api::activity_list))
        .route(
            "/desktop/pet",
            get(activity_api::pet_state_get).post(activity_api::pet_state_set),
        )
        .route("/desktop/pet/report", post(activity_api::pet_state_report))
        .route(
            "/desktop/model-credentials",
            post(register_model_credential),
        )
        .route(
            "/approvals/pending",
            get(activity_api::pending_approvals_list),
        )
        // 本机文件系统动作（取优合并自远端 engine）：选择文件夹 / 用外部程序打开。
        .route("/fs/pick-directory", post(fs_api::fs_pick_directory))
        .route("/fs/open", post(fs_api::fs_open))
        .route(
            "/settings",
            get(settings_api::settings_get).post(settings_api::settings_update),
        )
        .route("/settings/egress", post(settings_api::settings_egress))
        .route(
            "/settings/provider-test",
            post(settings_api::settings_provider_test),
        )
        // 审查 N1：工作区信任（受限模式状态 + 显式信任/撤销）。
        .route(
            "/workspace/trust",
            get(workspace_trust_api::workspace_trust_get)
                .post(workspace_trust_api::workspace_trust_set),
        )
        // 符号级检索（G14）：与 find_symbol 工具 / CLI `owo-agent symbols` 同内核，只读。
        .route("/symbols", get(symbols_api::symbols))
        // §5.3/§5.4 权限档位与授权记忆管理（UI/CLI 统一入口）。
        .route(
            "/permissions",
            get(settings_api::permissions_status).post(settings_api::permissions_set_profile),
        )
        .route("/permissions/grants", get(settings_api::grants_list))
        .route(
            "/permissions/grants/revoke",
            post(settings_api::grants_revoke),
        )
        // §4.5 权限中心：服务端聚合总览 + 结构化配置写入（前端不自行推导范围）。
        .route(
            "/permissions/overview",
            get(settings_api::permissions_overview),
        )
        .route(
            "/permissions/spec",
            post(settings_api::permissions_set_spec),
        )
        .route("/whitelist", get(whitelist_api::whitelist_list))
        .route("/whitelist/manage", post(whitelist_api::whitelist_manage))
        .route("/session/{id}/context", get(session_api::session_context))
        .route("/session/{id}/compact", post(session_api::compact_session))
        .route("/session/{id}/goal", post(session_goal_api::session_goal))
        .route("/skills/health", get(skills_api::skills_health))
        .route(
            "/skills/health/{name}/reset",
            post(skills_api::skill_health_reset),
        )
        .route("/plugins", get(plugin_api::plugins_list))
        .route("/plugins/{id}/enabled", post(plugin_api::plugin_enabled))
        .route("/subagent/run", post(subagent_api::subagent_run))
        .route(
            "/project/rules",
            get(project_api::project_rules_get).post(project_api::project_rules_post),
        )
        .route(
            "/project/rules/template",
            post(project_api::project_rules_template),
        )
        .route("/mcp", get(mcp_api::mcp_list))
        .route("/mcp/health", get(mcp_api::mcp_health_snapshot))
        .route("/capabilities", get(capabilities::capabilities_list))
        .route("/mcp/add", post(mcp_api::mcp_add))
        .route("/mcp/remove", post(mcp_api::mcp_remove))
        .route("/mcp/reconnect", post(mcp_api::mcp_reconnect))
        .route("/mcp/enabled", post(mcp_api::mcp_enabled))
        .route("/locate/query", post(locate_api::locate_query))
        .route("/traces", get(traces_api::traces_list))
        .route("/traces/{index}", get(traces_api::trace_show))
        .route("/memory/recall", get(memory_api::memory_recall))
        .route(
            "/computer-use/tasks",
            get(computer_api::computer_tasks_list),
        )
        .route(
            "/computer-use/task",
            post(computer_api::computer_task_create),
        )
        .route(
            "/computer-use/task/{id}/{action}",
            post(computer_api::computer_task_transition),
        )
        .route(
            "/computer-use/task/{id}/check/{action}",
            get(computer_api::computer_task_check),
        )
        .route(
            "/computer-use/sensitive-check",
            post(computer_api::computer_sensitive_check),
        )
        .route(
            "/computer-use/task/{id}/run",
            post(computer_api::computer_task_run),
        )
        .route("/cloud/tasks", post(cloud_api::cloud_task_submit))
        .route("/cloud/tasks/{id}", get(cloud_api::cloud_task_status))
        .route(
            "/cloud/tasks/{id}/result",
            get(cloud_api::cloud_task_result),
        )
        .route(
            "/cloud/tasks/{id}/cancel",
            post(cloud_api::cloud_task_cancel),
        )
        // R8 服务端韧性（并发上限/状态/优雅关闭）。
        .route("/server/status", get(ops_api::server_status))
        .route("/server/shutdown", post(ops_api::server_shutdown))
        // R8 用量预算：加额恢复（硬熔断后 request_topup 解除停轮）。
        .route("/usage/topup", post(usage::usage_topup))
        // 与 R6 同款对齐：先 with_state 定 S，再 merge 模块 router（Router<()> 经 From 转换）。
        .with_state(state.clone())
        .merge(notes_api::router(state.clone()))
        .merge(plugin_market_api::router(state.clone()))
        .merge(workflow_api::router(state.clone()))
        .merge(goal_api::router(state.clone()))
        .merge(sse::router(state.clone()))
        // R5 第五轮：eval 护栏 / 团队共享 / 可观测性 / 记忆图谱 / 统一命令入口。
        // Agent 1 的审批（/workflow/run/{run_id}/approval）与 run SSE
        // （/workflow/run/{run_id}/events）已自含在 workflow_api::router 内，无需新 merge。
        .merge(team_api::router(state.clone()))
        .merge(workswarm_api::router(state.clone()))
        // V1 四期（第三路）：Artifact 评审闭环。
        .merge(artifact_review_api::router(state.clone()))
        // 六期（第三路）：内置团队模板目录（候选展示 + 手动安装，幂等）。
        .merge(team_template_catalog_api::team_template_catalog_router(
            state.clone(),
        ))
        // 用户自定义工作流（团队模板的用户/社区通道：校验 + CRUD）。
        .merge(user_workflows_api::user_workflows_router(state.clone()))
        // AI 生成工作流草稿（自然语言 → 候选配置 → 校验 → 人工采纳）。
        .merge(workflow_draft_api::workflow_draft_router(state.clone()))
        // 工作流社区共享（导出/导入前评审/导入门禁）。
        .merge(workflows_share_api::workflows_share_router(state.clone()))
        // 工作流使用统计（只读聚合：运行次数与成败）。
        .merge(workflow_stats_api::workflow_stats_router(state.clone()))
        // R1（§8.5）：DesktopWorld/WorldModel 闭环 /desktop-envs/*、/world-model/*、
        // /transitions/*、/datasets/*、/model-candidates/*（desktop_world_api 模块内
        // DesktopWorldHub 单例 + ControllerLease token+epoch 围栏；与 /desktop/* 计算机
        // 操作路由不同区）。
        .merge(desktop_world_api::router(state.clone()))
        .merge(eval_gate::router(state.clone()))
        // V1 三日（第四路）：ProductEval 评测中心（bearer 保护面）。
        .merge(product_eval_api::router(state.clone()))
        .merge(observability_api::router(state.clone()))
        .merge(memory_graph_api::router(state.clone()))
        .merge(intent_api::router(state.clone()))
        // R8 存储运维（备份/恢复/导出/清空）。
        .merge(backup::router(state.clone()))
        // R3（§8.1）：冷启动诊断 ledger（GET /diagnostics/requests，仅六字段）。
        .merge(request_ledger_api::router(state.clone()))
        // R8 用量与成本归集（Agent 4 交付：usage_router 四维用量 + 预算硬熔断）。
        .merge(usage::usage_router(state.clone()))
        // R10 契约治理：JSON Schema 版本化发布（/schemas/*）+ 契约变更 RFC 登记见本文件契约区。
        .route("/schemas", get(schemas_api::schemas_list))
        .route("/schemas/{kind}/{version}", get(schemas_api::schema_get))
        // R12（Agent 2 交付，主控挂载）：P2 双节点网格控制面 /fleet/*（节点注册/列表、
        // 任务提交/查询/取消/SSE 事件、审批响应；模块内 FleetHub 单例，不占用 AppState）。
        .merge(fleet_api::router(state.clone()))
        // R6（Wave 1，Agent 4 交付）：可靠事件流 /events/stream（SSE 续传 + 背压）。
        .merge(event_stream::router(state.clone()))
        // 默认 JSON 请求只允许 1 MiB；音频、附件、技能包在各自路由上单独放宽。
        .layer(DefaultBodyLimit::max(1024 * 1024))
        // 鉴权在最外层：未授权请求不进入限流，也不消耗令牌。
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_token::require_auth,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit::enforce_rate_limit,
        ));
    // 公开面（含静态 fallback）与保护面合并：两者均为 Router<Arc<AppState>>。
    // R8/R9：trace_id 贯穿置于最外层（public + protected + fallback 全覆盖）。
    public
        .merge(protected)
        // 桌宠 UI（overlay 壳加载）：`OWO_PET_UI_DIR` 指向桌面端仓库的
        // `apps/overlay/ui/pet`（index.html/JS/CSS 与 assets/skins 皮肤资产），
        // 未设置或目录无效时回落 `desktop/web/pet`。磁盘直读 + 全局 no-store
        // → 改桌宠前端只需刷新窗口，无需重新构建 overlay。
        .nest_service("/pet", ServeDir::new(pet_ui_dir()))
        // 桌宠皮肤资产（spritesheet/静态图）：与 `/pet` 分离挂载——
        // 桌面端 overlay 的皮肤目录 `ui/assets/skins` 单一来源，
        // 前端以根相对路径 `/pet-assets/skins/<id>/<file>` 取图。
        .nest_service("/pet-assets", ServeDir::new(pet_assets_dir()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            trace_id_middleware,
        ))
        // N11：Host 白名单（DNS rebinding 防护）——在任何业务处理之前拒绝。
        .layer(axum::middleware::from_fn(host_guard_middleware))
        // R10：弃用策略——命中 DEPRECATED_ROUTES 附加 Deprecation 头。
        .layer(axum::middleware::from_fn(deprecation_middleware))
        .fallback_service(ServeDir::new(desktop_web_dir()))
        .layer(cors_layer())
        // 本地工具：API 与静态资源一律禁用浏览器缓存——工作台由磁盘直读，
        // 启发式缓存会让「代码改了页面却没变」（已实测 app.js 被缓存拿旧逻辑）。
        .layer(axum::middleware::from_fn(no_store_middleware))
}

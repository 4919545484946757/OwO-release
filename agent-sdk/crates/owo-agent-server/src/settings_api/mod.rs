//! 设置与权限档位 HTTP API（§12：从 lib.rs 机械外移的 settings/permissions/grants 域）。
//!
//! 路由面（`GET/POST /settings`、`POST /settings/egress`、`GET/POST /permissions`、
//! `GET /permissions/grants`、`POST /permissions/grants/revoke`）与 /openapi.json
//! 登记保持不变，零行为变化。
//! `auto_approve_enabled` 仍留根（health 与内联测试共用），不在本模块。
//!
//! 独立编译约束（沿 notes_api 惯例）：不使用 `crate::`/`super::`；引用 server
//! 类型一律写全限定名 `owo_agent_server::AppState`。

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use owo_agent_core::whitelist::Whitelist;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

pub(super) async fn settings_get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = owo_agent_core::Settings::load(&state.workspace);
    let mut value = serde_json::to_value(&settings)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let active_tools = state
        .agent
        .visible_tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    if let Some(object) = value.as_object_mut() {
        let mut runtime = effective_runtime_config(&settings);
        if let Some(runtime) = runtime.as_object_mut() {
            runtime.insert("active_tool_names".to_string(), json!(active_tools));
        }
        // provider_ready 从上面这份 runtime 取（它就是 effective_runtime_config 的产物）。
        // **不能回头读 `value.get("runtime")`**：此刻 `object` 是 value 的可变借用，
        // 再不可变借一次会触发 E0502。
        let ready = runtime
            .get("provider_ready")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        // provider 段投影成「存在性 + 掩码」形状回前端：设置页要用它回填表单，
        // 但明文密钥绝不能过网（仓库红线：凭据只经环境变量 / 本机信封）。
        // 这里只给 api_key_set 布尔与 base_url；前端据此显示"已保存（加密）"。
        let provider_view = json!({
            "base_url": settings
                .provider
                .base_url
                .clone()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    std::env::var("OPENAI_BASE_URL")
                        .ok()
                        .filter(|value| !value.trim().is_empty())
                }),
            "api_key_set": settings.provider.has_api_key()
                || std::env::var_os("OPENAI_API_KEY")
                    .map(|value| !value.to_string_lossy().trim().is_empty())
                    .unwrap_or(false),
            "api_key_env": settings.provider.api_key_env.clone(),
        });
        object.insert("provider".to_string(), provider_view);
        // 顶层便捷字段：工作台首启门直接读它（历史上只读 settings.runtime，
        // 缺 provider_ready 时恒判未就绪 → 发送按钮永远禁用）。
        object.insert("provider_ready".to_string(), json!(ready));
        object.insert("runtime".to_string(), runtime);
    }
    Ok(Json(value))
}

/// 只读有效运行配置：前端不得用历史表单默认值冒充当前 provider/model。
/// 凭据只暴露来源，不暴露内容。
pub(super) fn effective_runtime_config(settings: &owo_agent_core::Settings) -> Value {
    // 优先级：环境变量（已注入/已保存即时生效） > settings.json 的 provider 段。
    // 两者都缺省才回落内置默认端点。**不能只看环境变量**：浏览器开发模式下没有壳注入，
    // 用户在设置页填的端点只落在 settings.json。
    let base_url = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            settings
                .provider
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "https://open.bigmodel.cn/api/paas/v4".to_string());
    let model = std::env::var("OPENAI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| settings.model.clone())
        .unwrap_or_else(|| "glm-5.3-flash".to_string());
    let lower = base_url.to_ascii_lowercase();
    let provider = if lower.contains("bigmodel") || lower.contains("zhipu") {
        "bigmodel"
    } else if lower.contains("aliyuncs") || lower.contains("dashscope") {
        "qwen"
    } else if lower.contains("deepseek") {
        "deepseek"
    } else if lower.contains("ollama") || lower.contains("127.0.0.1") || lower.contains("localhost")
    {
        "ollama"
    } else {
        "openai-compatible"
    };
    let local = provider == "ollama";
    let cloud_enabled = std::env::var("OWO_CLOUD_ENABLED")
        .ok()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(settings.egress.cloud_enabled);
    // 凭据来源：环境变量优先，其次 settings.json 的 provider 段（浏览器开发模式）。
    let key_present = std::env::var_os("OPENAI_API_KEY")
        .map(|value| !value.to_string_lossy().trim().is_empty())
        .unwrap_or(false)
        || settings.provider.has_api_key();
    json!({
        "provider": provider,
        "model": model,
        "endpoint_kind": if local { "local" } else { "cloud" },
        "credential_source": if local {
            "not_required"
        } else if key_present {
            "environment"
        } else {
            "missing"
        },
        "cloud_enabled": cloud_enabled,
        "available_models": [model],
        // 前端首启门与设置页回填用；服务端从不回显明文密钥。
        "provider_ready": local || key_present,
        "base_url_set": settings.provider.has_base_url()
            || std::env::var("OPENAI_BASE_URL").is_ok_and(|v| !v.trim().is_empty()),
        "api_key_set": key_present,
    })
}

#[derive(Deserialize)]
pub(super) struct EgressRequest {
    cloud_enabled: bool,
}

pub(super) async fn settings_egress(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EgressRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut settings = owo_agent_core::Settings::load(&state.workspace);
    settings.egress.cloud_enabled = request.cloud_enabled;
    settings
        .save(&state.workspace)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    std::env::set_var(
        "OWO_CLOUD_ENABLED",
        if request.cloud_enabled {
            "true"
        } else {
            "false"
        },
    );
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "settings",
            "egress",
            None,
            Some(request.cloud_enabled),
            format!("数据出境开关：cloud_enabled={}", request.cloud_enabled),
        );
    }
    Ok(Json(json!({
        "cloud_enabled": request.cloud_enabled,
        "note": "已写入 settings.json 并即时生效",
    })))
}

/// §5.3/§5.4 权限状态：当前档位 + 生效中的授权记忆（脱敏：不含参数原文）。
pub(super) async fn permissions_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let profile = state.agent.permission_profile();
    let grants = grant_rows(&state.grants);
    Ok(Json(json!({
        "profile": profile.label(),
        "read_only": state.agent.permission_profile() == owo_agent_core::PermissionProfile::ReadOnly,
        "grants": grants,
        "grant_count": grants.len(),
    })))
}

#[derive(Deserialize)]
pub(super) struct ProfileRequest {
    profile: String,
}

/// §5.3 切换权限档位：写 settings.json（重启恢复）+ 运行时即时生效。
pub(super) async fn permissions_set_profile(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProfileRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let profile = owo_agent_core::PermissionProfile::parse(&request.profile).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            format!(
                "未知档位：{}（可选 read_only / workspace / auto_review / full_access / custom）",
                request.profile
            ),
        )
    })?;
    let mut settings = owo_agent_core::Settings::load(&state.workspace);
    if profile == owo_agent_core::PermissionProfile::ReadOnly {
        settings.read_only = true;
    } else {
        settings.read_only = false;
        settings.permission_profile = Some(request.profile.clone());
    }
    settings
        .save(&state.workspace)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    state.agent.set_permission_profile(profile);
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "permissions",
            "profile",
            None,
            Some(true),
            format!("权限档位切换：{}", request.profile),
        );
    }
    Ok(Json(json!({
        "ok": true,
        "profile": profile.label(),
        "note": "已写入 settings.json 并即时生效",
    })))
}

/// §5.4 授权记忆列表（与 permissions_status / overview 同构；独立路由便于前端独立刷新）。
pub(super) async fn grants_list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let grants = grant_rows(&state.grants);
    Ok(Json(
        json!({ "grants": grants, "grant_count": grants.len() }),
    ))
}

#[derive(Deserialize, Default)]
pub(super) struct GrantRevokeRequest {
    #[serde(default)]
    grant_id: Option<String>,
    #[serde(default)]
    tool_id: Option<String>,
    #[serde(default)]
    all: bool,
}

/// §5.4 / §4.5.2 撤销授权记忆，三种粒度（**级联撤销**）：
///
/// - `{grant_id}`：单条（既有契约，字面不变；不存在仍 404）；
/// - `{tool_id}`：该工具的全部授权（工具下线/换实现时用）；
/// - `{all:true}`：当前工作区全部长期授权（权限中心"撤销本工作区"）。
///
/// 三者互斥按上表优先级取第一个出现的字段；都不给 → 400（拒绝"空请求撤销全部"
/// 这类误解，宁可比照失败也不放大作用面）。
pub(super) async fn grants_revoke(
    State(state): State<Arc<AppState>>,
    Json(request): Json<GrantRevokeRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let workspace_id = state.workspace_id();
    let (revoked, scope) = if let Some(grant_id) = request.grant_id.as_deref() {
        if !state.grants.revoke(grant_id) {
            return Err((StatusCode::NOT_FOUND, format!("授权记忆不存在：{grant_id}")));
        }
        (1usize, "grant")
    } else if let Some(tool_id) = request.tool_id.as_deref() {
        (state.grants.revoke_tool(tool_id), "tool")
    } else if request.all {
        (state.grants.revoke_workspace(&workspace_id), "workspace")
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "撤销需要指定 grant_id、tool_id 或 all=true 之一".to_string(),
        ));
    };
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "permissions",
            "grant_revoke",
            None,
            Some(true),
            format!("撤销授权记忆：{scope} 共 {revoked} 条"),
        );
    }
    // `revoked` 语义从"被撤销的 grant_id 字符串"改为"条数"：前端要的是
    // "撤销后还剩几条"的可断言数字，单条场景两者信息等价（grant_id 另原样回显）。
    Ok(Json(json!({
        "ok": true,
        "revoked": revoked,
        "scope": scope,
        "grant_id": request.grant_id,
        "remaining": state.grants.list().len(),
    })))
}

/// 授权记忆行（`/permissions`、`/permissions/grants`、`/permissions/overview` 共用，
/// 保证三处形状一致——同一事实不能有两个版本）。
fn grant_rows(store: &owo_agent_core::grant_store::GrantStore) -> Vec<Value> {
    store
        .list()
        .into_iter()
        .map(|grant| {
            json!({
                "grant_id": grant.grant_id,
                "tool_id": grant.tool_id,
                "workspace_id": grant.workspace_id,
                "scope": grant.scope,
                "path_scope": grant.path_scope.map(|path| path.to_string_lossy().into_owned()),
                "host_scope": grant.host_scope,
                "has_fingerprint": grant.argument_fingerprint.is_some(),
                "created_at": grant.created_at.to_rfc3339(),
                "expires_at": grant.expires_at.map(|at| at.to_rfc3339()),
                "remaining_uses": grant.remaining_uses,
                "long_lived": grant.expires_at.is_none() && grant.remaining_uses.is_none(),
                // 旧版本曾给非只读工具生成 Grant；列表标出失效项，用户仍可撤销。
                "effective": owo_agent_core::tool_effects::effect_class_for(&grant.tool_id)
                    == owo_agent_core::tool_effects::EffectClass::Read,
            })
        })
        .collect()
}

/// §4.5 权限中心总览：档位 + 结构化配置 + 四维生效判定 + 待审批 + 授权记忆 + 近期决定。
///
/// 为什么这个聚合必须在服务端：§4.5.1 要页面显示"三个维度的实际范围"，而前端只有
/// 档位名。让前端从 `workspace` 去猜"文件可写、命令询问"就是**在权限页面上编造范围**，
/// 用户会照着它做授权决定。这里由 [`PermissionSpec::expand`] 给出唯一口径，
/// 前端只做渲染（见 §4.8 的 render 层零判定）。
pub(super) async fn permissions_overview(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    use owo_agent_core::permission_spec::PermissionSpec;
    let profile = state.agent.permission_profile();
    let read_only = state.agent.policy().is_read_only();
    let stored = state.agent.policy().spec();
    // 没有显式配置时按档位**如实投影**（投影不出来的落 custom），
    // 而不是硬编码一套"看起来合理"的维度值。
    let effective = stored
        .clone()
        .unwrap_or_else(|| PermissionSpec::from_profile(profile, !read_only));
    let source = if stored.is_some() { "spec" } else { "profile" };
    let rules = effective.expand();
    let summary_of = |key: &str| {
        rules
            .iter()
            .find(|rule| rule.id == key)
            .map(|rule| rule.reason.clone())
            .unwrap_or_default()
    };
    let dimensions = json!([
        {
            "key": "filesystem",
            "label": "文件系统",
            "effective": effective.filesystem.as_str(),
            "source": source,
            "configurable": true,
            "summary": summary_of("filesystem"),
        },
        {
            "key": "command",
            "label": "命令执行",
            "effective": effective.command.as_str(),
            "source": source,
            "configurable": true,
            "summary": summary_of("command"),
        },
        {
            "key": "network",
            "label": "网络访问",
            "effective": effective.network.as_str(),
            "source": source,
            "configurable": true,
            "summary": summary_of("network"),
        },
        {
            "key": "persistence",
            "label": "授权有效期",
            "effective": effective.persistence.as_str(),
            "source": source,
            "configurable": true,
            "summary": summary_of("persistence"),
        },
    ]);
    // 待审批动作：全局一份（此前只有会话 SSE 能看见，断线即失联）。
    // 两个 map 分别取快照、不同时持两把锁——避免与 respond_permission 的加锁顺序互相卡住。
    let session_of: std::collections::HashMap<String, String> = state
        .pending_approval_sessions
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    let waiting: Vec<(String, String, owo_agent_core::PermissionRequest)> = state
        .pending_approvals
        .lock()
        .map(|guard| {
            guard
                .iter()
                .map(|(id, (_sender, request))| (id.clone(), request.tool.clone(), request.clone()))
                .collect()
        })
        .unwrap_or_default();
    let pending: Vec<Value> = waiting
        .into_iter()
        .map(|(request_id, _tool, request)| {
            let explain = owo_agent_core::permissions::describe_request(&request);
            json!({
                "request_id": request_id.clone(),
                "session_id": session_of.get(&request_id).cloned(),
                "tool": request.tool,
                "level": level_label(&request),
                "reason": request.reason,
                "explain": explain,
                "redacted_args": request.redacted_args.clone()
                    .unwrap_or_else(|| owo_agent_core::permissions::redact_args(&request.args)),
                "risk_note": request.risk_note,
                // 「工作区长期」只对非破坏性动作开放（§5.4）；前端据此禁用第三/第四个选项。
                "destructive": request.is_destructive(),
            })
        })
        .collect();
    let grants = grant_rows(&state.grants);
    let recent_decisions: Vec<Value> = state
        .agent
        .audit_log()
        .lock()
        .map(|audit| {
            audit
                .entries
                .iter()
                .filter(|entry| entry.event.contains("permission"))
                .rev()
                .take(20)
                .map(|entry| {
                    json!({
                        "ts": entry.ts,
                        "session_id": entry.session_id,
                        "tool": entry.tool,
                        "approved": entry.approved,
                        "detail": entry.detail,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let full_access_preview =
        PermissionSpec::from_profile(owo_agent_core::PermissionProfile::FullAccess, true);
    Ok(Json(json!({
        "profile": profile.label(),
        "read_only": read_only,
        "spec": stored,
        "effective_spec": effective,
        "dimensions": dimensions,
        "expanded": rules,
        "pending": pending,
        "pending_count": pending.len(),
        "grants": grants,
        "grant_count": grants.len(),
        // §4.5.2 落盘事实：不落盘就没有"工作区长期"，界面上打了勾也是假的。
        "grants_persisted": state.grants.persist_path().is_some(),
        "recent_decisions": recent_decisions,
        "full_access": {
            "active": profile == owo_agent_core::PermissionProfile::FullAccess,
            "requires_confirm": true,
            "risk_notes": full_access_preview.risk_notes(),
        },
        "scope_literals": {
            "filesystem": ["none", "workspace_read", "workspace_write", "custom"],
            "command": ["deny", "allowlisted", "unrestricted"],
            "network": ["deny", "allowlisted", "unrestricted"],
            "persistence": ["once", "task", "workspace"],
            "approval": ["once", "task", "workspace"]
        },
    })))
}

/// 审批请求的动作级别标签（小写稳定字面量，前端与测试都按它断言）。
fn level_label(request: &owo_agent_core::PermissionRequest) -> &'static str {
    match request.level {
        owo_agent_core::permissions::Level::Read => "read",
        owo_agent_core::permissions::Level::Write => "write",
        owo_agent_core::permissions::Level::Execute => "execute",
        owo_agent_core::permissions::Level::Inject => "inject",
    }
}

#[derive(Deserialize)]
pub(super) struct SpecRequest {
    spec: owo_agent_core::permission_spec::PermissionSpec,
    #[serde(default)]
    confirm: bool,
    /// 完全访问的**时长**（秒）。指南要求"范围 + 时长 + 风险"三要素齐了才允许提交。
    #[serde(default)]
    duration_secs: Option<i64>,
}

/// 完全访问允许的最长时长（8 小时）：到期后由人重新确认，不接受"永久完全访问"。
const FULL_ACCESS_MAX_SECS: i64 = 8 * 3600;

/// §4.5.3 提交结构化权限配置（权限中心唯一写入口）。
///
/// 三道闸：
/// 1. **字面量校验**：serde 直接拒未知值（`validation/failed`），不做静默降级；
/// 2. **只收紧不放宽**：spec 只能把档位往严的方向同步（`Policy::set_spec` 里
///    只读档是上界），因此这里显式拒绝"在只读模式下开放写入"；
/// 3. **完全访问三要素**：命令或网络任一 `unrestricted` 时，必须带 `confirm=true`
///    与 `duration_secs`（1..=8h），并把风险清单回给前端做二次确认展示。
pub(super) async fn permissions_set_spec(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SpecRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let bad_request = |code: &str, message: String| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": { "code": code, "message": message },
            })),
        )
    };
    let spec = request.spec;
    if spec.filesystem == owo_agent_core::permission_spec::FilesystemScope::Custom
        && spec.scopes.is_empty()
    {
        return Err(bad_request(
            "validation/failed",
            "filesystem=custom 必须给出至少一条 path: 范围；空范围等于没配置".to_string(),
        ));
    }
    for scope in &spec.scopes {
        let known = scope.starts_with("path:")
            || scope.starts_with("host:")
            || scope.starts_with("command:");
        if !known {
            return Err(bad_request(
                "validation/failed",
                format!("未知范围前缀：{scope}（应为 path: / host: / command:）"),
            ));
        }
        if scope.contains("..") || std::path::Path::new(scope).is_absolute() {
            return Err(bad_request(
                "validation/failed",
                format!("范围必须是工作区相对字面量，不得含 .. 或绝对路径：{scope}"),
            ));
        }
    }
    let unrestricted = spec.command == owo_agent_core::permission_spec::RuleScope::Unrestricted
        || spec.network == owo_agent_core::permission_spec::RuleScope::Unrestricted;
    if unrestricted {
        if !request.confirm {
            return Err(bad_request(
                "confirmation/required",
                "命令或网络设为 unrestricted 属完全访问，必须显式 confirm=true 并给出时长"
                    .to_string(),
            ));
        }
        match request.duration_secs {
            Some(secs) if (60..=FULL_ACCESS_MAX_SECS).contains(&secs) => {}
            Some(secs) => {
                return Err(bad_request(
                    "validation/failed",
                    format!("完全访问时长需在 60..{FULL_ACCESS_MAX_SECS} 秒之间，收到 {secs}"),
                ))
            }
            None => {
                return Err(bad_request(
                    "confirmation/required",
                    "完全访问必须带 duration_secs（时长上限 8 小时，到期后重新确认）".to_string(),
                ))
            }
        }
    }
    let nearest = spec.nearest_profile();
    // 两件事必须分开判，混成一个条件就会把闸门写死：
    // - **上界**：只读档（Plan 模式）下提交任何"非只读等价"的 spec 一律拒绝。
    //   运行时确实不会立刻放宽（`Policy::set_spec` 守得住档位），但这份配置会
    //   落进 settings.json，用户一退出只读模式就自动生效——那是他没确认过的授权。
    // - **收紧**：spec 等价只读时，把档位与 settings.read_only 一起推到只读，
    //   让"界面写着 workspace、实际什么都干不了"这种分裂真相不出现。
    if state.agent.policy().is_read_only() && nearest != owo_agent_core::PermissionProfile::ReadOnly
    {
        return Err(bad_request(
            "conflict/read_only",
            "当前为只读模式，无法保存更宽的结构化配置；请先退出只读档位再提交（只读等价的收紧配置可以直接存）"
                .to_string(),
        ));
    }
    let forces_read_only = nearest == owo_agent_core::PermissionProfile::ReadOnly;
    let effective_profile = if forces_read_only {
        owo_agent_core::PermissionProfile::ReadOnly
    } else {
        state.agent.permission_profile()
    };
    let mut settings = owo_agent_core::Settings::load(&state.workspace);
    settings.read_only = forces_read_only;
    if forces_read_only {
        settings.permission_profile = None;
    } else {
        settings.permission_profile = Some(effective_profile.label().to_string());
    }
    settings.permission_spec = Some(spec.clone());
    if let Err(error) = settings.save(&state.workspace) {
        return Err(bad_request("storage/not_writable", error));
    }
    state.agent.policy().set_spec(spec.clone());
    let expanded = spec.expand();
    let denials_added = expanded.iter().filter(|rule| rule.effect == "deny").count();
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "permissions",
            "spec",
            None,
            Some(true),
            format!(
                "权限中心提交结构化配置：filesystem={} command={} network={} persistence={} scopes={}（档位同步为 {}）",
                spec.filesystem.as_str(),
                spec.command.as_str(),
                spec.network.as_str(),
                spec.persistence.as_str(),
                spec.scopes.len(),
                effective_profile.label(),
            ),
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Settings);
    Ok(Json(json!({
        "ok": true,
        "spec": spec,
        "expanded": expanded,
        "profile": state.agent.permission_profile().label(),
        "denials_added": denials_added,
        "duration_secs": if unrestricted { request.duration_secs } else { None },
        "note": "已写入 settings.json 并即时生效；只收紧维度已接入判定链",
    })))
}

/// 通用设置保存：写入 settings.json 并应用运行时设置（数据出境、STT、主动建议、白名单）。
pub(super) async fn settings_update(
    State(state): State<Arc<AppState>>,
    Json(settings): Json<owo_agent_core::Settings>,
) -> Result<Json<Value>, (StatusCode, String)> {
    settings
        .save(&state.workspace)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    // 审查 N1：用户经设置页显式保存 = 对该工作区配置的显式信任。否则"保存 hooks
    // 成功、重启后却不加载"会变成无法解释的行为（受限模式）。
    if let Err(error) = state.trust_store.trust(&state.workspace) {
        tracing::warn!("写入工作区信任记录失败（本次设置仍已保存）：{error}");
    }
    settings.apply_usage_env();
    settings.apply_reasoning_env();
    // §13 批次六：遥测开关即时生效（默认关；与 cli serve 启动应用同一出口）。
    owo_agent_server::apply_telemetry_setting(settings.telemetry_enabled == Some(true));
    state
        .agent
        .apply_policy_settings(settings.read_only, &settings.deny_commands);
    // A2-1：hooks 随设置热重灌（空数组 = 清空，避免「文件删了进程还在跑」）。
    state
        .agent
        .set_hooks(owo_agent_core::hooks::HookManager::from_configs(
            &settings.hooks,
        ));
    // §5.3 档位从 settings 恢复（read_only=true 时以只读档为准）。
    if !settings.read_only {
        if let Some(profile) = settings
            .permission_profile
            .as_deref()
            .and_then(owo_agent_core::PermissionProfile::parse)
        {
            state.agent.set_permission_profile(profile);
        }
    }
    // §4.5.3 结构化配置跟着 settings 一起恢复；缺省时清除运行时 spec，
    // 避免"文件里已删掉、进程里还在收紧"的两套真相。
    match settings.permission_spec.clone() {
        Some(spec) => state.agent.policy().set_spec(spec),
        None => state.agent.policy().clear_spec(),
    }
    if let Some(model) = &settings.model {
        if !model.trim().is_empty() {
            std::env::set_var("OPENAI_MODEL", model);
        }
    }
    // Provider 段（设置页「模型」面板）写回环境变量：provider_ready() 只认
    // OPENAI_BASE_URL / OPENAI_API_KEY，不看 settings.json。此前这里不写，导致
    // 用户保存端点与密钥后首启门仍判定未就绪、发送按钮一直禁用。
    // 留空 = 不覆盖（保留进程启动时继承的环境变量）。
    let provider = &settings.provider;
    if provider.has_base_url() {
        if let Some(base_url) = provider.base_url.as_deref() {
            std::env::set_var("OPENAI_BASE_URL", base_url.trim());
        }
    }
    if provider.has_api_key() {
        if let Some(api_key) = provider.api_key.as_deref() {
            std::env::set_var("OPENAI_API_KEY", api_key.trim());
        }
    }
    if let Some(env_name) = provider.api_key_env.as_deref() {
        let name = env_name.trim();
        if !name.is_empty() {
            std::env::set_var("OWO_PROVIDER_API_KEY_ENV", name);
        }
    }
    std::env::set_var(
        "OWO_CLOUD_ENABLED",
        settings.egress.cloud_enabled.to_string(),
    );
    if let Ok(mut stt) = state.stt.lock() {
        stt.apply_settings(&settings.stt);
    }
    if let Ok(mut proactive) = state.proactive.lock() {
        proactive.apply_settings(settings.proactive.clone());
    }
    if let Ok(mut whitelist) = state.whitelist.lock() {
        let mut merged = Whitelist::default();
        for entry in settings.whitelist.clone() {
            merged.upsert(entry);
        }
        *whitelist = merged;
    }
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "settings",
            "update",
            None,
            Some(true),
            "设置页保存（settings.json）",
        );
    }
    owo_agent_server::event_stream::hub()
        .publish_invalidate(owo_agent_server::event_stream::InvalidateDomain::Settings);
    Ok(Json(json!({
        "ok": true,
        "note": "已写入 settings.json 并应用运行时设置；模型对新回合即时生效，tool_capabilities 在重启 Daemon 后生效",
        "runtime": effective_runtime_config(&settings),
    })))
}

/// R3-B（§3.4 动作「测试连接」）：提供商自诊断端点。
///
/// 契约（§2.4 统一错误模型）：响应恒携带稳定码，UI 按 `code` 渲染动作，不匹配中文——
/// - `provider/not_configured`：云端路径缺凭据（**先判凭据，不发起任何网络**）；
/// - `provider/endpoint_reachable` / `provider/endpoint_unreachable`：仅 TCP 层探测
///   （3s 超时），不发真实补全请求、不消耗 token；
/// - 端点展示一律脱敏（scheme+host[:port]，无路径/查询/ userinfo）。
pub(super) async fn settings_provider_test(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let settings = owo_agent_core::Settings::load(&state.workspace);
    let runtime = effective_runtime_config(&settings);
    let base_url = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://open.bigmodel.cn/api/paas/v4".to_string());
    let is_local = runtime["endpoint_kind"].as_str() == Some("local");
    let key_present = std::env::var_os("OPENAI_API_KEY")
        .map(|value| !value.to_string_lossy().trim().is_empty())
        .unwrap_or(false);
    let credential = if is_local {
        "not_required"
    } else if key_present {
        "environment"
    } else {
        "missing"
    };
    let (code, ok, message, latency_ms) = if !is_local && !key_present {
        (
            "provider/not_configured",
            false,
            "模型提供商未配置：请在设置中选择云端（并配置 OPENAI_API_KEY 环境变量）或本地 Ollama"
                .to_string(),
            Value::Null,
        )
    } else if !is_local
        && !std::env::var("OWO_CLOUD_ENABLED")
            .ok()
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(true)
    {
        // 数据出境开关（AGENTS.md 辅助变量 `OWO_CLOUD_ENABLED=false`，与 gateway.rs
        // 的云端放行判定同一语义）：**测试连接本身也是一条出网路径**，必须在发起
        // 连接之前就拒绝。否则用户显式关掉了云，点一下引导页的「测试连接」仍然会
        // 打公网端点——那既是出境泄漏，也让 cargo test 顺带探测公网。
        //
        // 这里返回的是**本地判定结论**，不是网络结论，所以另起稳定码，不复用
        // endpoint_unreachable：把"我们拒绝出网"标成"端点不可达"会把用户推向
        // 查防火墙这条完全错误的路上（§2.5 错误模型：归因必须指向可执行的下一步）。
        (
            "network/cloud_disabled",
            false,
            "已拒绝云端访问（OWO_CLOUD_ENABLED=false）：请改用本地 Ollama，或确认需要出网后显式放开该开关".to_string(),
            Value::Null,
        )
    } else {
        match probe_endpoint(&base_url).await {
            Ok(ms) => (
                "provider/endpoint_reachable",
                true,
                "端点可达（TCP 层；模型答复需真实调用验证）".to_string(),
                json!(ms),
            ),
            Err(error) => (
                "provider/endpoint_unreachable",
                false,
                format!("端点不可达：{error}"),
                Value::Null,
            ),
        }
    };
    if let Ok(mut audit) = state.agent.audit_log().lock() {
        audit.record(
            "settings",
            "provider_test",
            None,
            Some(ok),
            format!("提供商连接测试：code={code}"),
        );
    }
    Ok(Json(json!({
        "ok": ok,
        "code": code,
        "endpoint": mask_endpoint(&base_url),
        "model": runtime["model"],
        "credential": credential,
        "latency_ms": latency_ms,
        "message": message,
    })))
}

mod provider_test;

use provider_test::*;

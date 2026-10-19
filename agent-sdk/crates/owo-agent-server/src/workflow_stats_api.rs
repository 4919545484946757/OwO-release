//! 工作流使用统计（`GET /workflows/stats`）：让用户看到"这条工作流被用过几次、成败如何"。
//!
//! 数据来源是团队运行记录（`TeamRun.template_id` 是组队来源的溯源字段）：
//! 只做聚合与只读展示，不改变任何运行状态。用于社区工作流治理（"常用的/总是失败的"）。
//!
//! 独立编译约束（沿 session_api 惯例）：不引 `crate::`/`super::`，全限定
//! `owo_agent_server::AppState`。

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use owo_agent_core::workswarm::TeamTemplateRegistry;
use owo_agent_protocol::TeamRunStatus;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use owo_agent_server::AppState;

/// 最近一次失败运行的定位信息（**取自当前进程的审计日志**；跨进程历史不保证）。
#[derive(Debug, Clone)]
pub(crate) struct FailureTrace {
    pub(crate) team_id: String,
    pub(crate) at: String,
    pub(crate) event: String,
    pub(crate) detail: String,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct TemplateStats {
    pub(crate) runs: usize,
    pub(crate) created: usize,
    pub(crate) running: usize,
    pub(crate) awaiting_human: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: usize,
    pub(crate) interrupted: usize,
    pub(crate) last_created_at: String,
    /// 最近一次运行时间（用于判定 last_run_failed 的口径）。
    pub(crate) last_run_at: String,
    /// 最近一次失败运行的 team_id（用于取审计定位原因）。
    last_failed_team: Option<(String, String)>,
    pub(crate) last_failure: Option<FailureTrace>,
    /// 最近一次运行（时间最大）是否失败。
    pub(crate) last_run_failed: bool,
}

impl TemplateStats {
    pub(crate) fn note_status(&mut self, status: TeamRunStatus) {
        match status {
            TeamRunStatus::Created => self.created += 1,
            TeamRunStatus::Running => self.running += 1,
            TeamRunStatus::AwaitingHuman => self.awaiting_human += 1,
            TeamRunStatus::Succeeded => self.succeeded += 1,
            TeamRunStatus::Failed => self.failed += 1,
            TeamRunStatus::Cancelled => self.cancelled += 1,
        }
    }

    /// 成功率（只有终态参与分母；没有终态时为 null——不编造 0%）。
    pub(crate) fn success_rate(&self) -> Option<f64> {
        let terminal = self.succeeded + self.failed + self.cancelled;
        if terminal == 0 {
            return None;
        }
        Some(self.succeeded as f64 / terminal as f64)
    }
}

/// 统计查询参数（排序与条数；缺省按运行次数倒序）。
#[derive(Debug, Default, Deserialize)]
struct StatsQuery {
    /// `runs`（默认）| `failed` | `success_rate` | `last`
    #[serde(default)]
    sort: Option<String>,
    /// `desc`（默认）| `asc`
    #[serde(default)]
    order: Option<String>,
    /// 最多返回多少条（缺省全部，硬顶 200）
    #[serde(default)]
    limit: Option<usize>,
}

/// 按模板聚合团队运行情况（`/workflows/stats` 与 `/workflows/user` 列表共用）。
///
/// 只统计 `template_id` 存在的运行（动态组队不参与工作流统计）。
pub(crate) async fn collect_template_stats(
    state: &AppState,
) -> Result<std::collections::BTreeMap<String, TemplateStats>, String> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|error| error.to_string())?;
    let runs = coordinator
        .list_team_runs()
        .await
        .map_err(|error| error.to_string())?;
    let mut stats: std::collections::BTreeMap<String, TemplateStats> =
        std::collections::BTreeMap::new();
    for run in runs {
        let Some(template_id) = run.template_id.clone() else {
            continue;
        };
        let created_at = run.created_at.clone();
        let status = run.status;
        let team_id = run.team_id.clone();
        let entry = stats.entry(template_id).or_default();
        entry.runs += 1;
        entry.note_status(status);
        if created_at > entry.last_created_at {
            entry.last_created_at = created_at.clone();
        }
        if created_at >= entry.last_run_at {
            entry.last_run_at = created_at.clone();
            entry.last_run_failed = status == TeamRunStatus::Failed;
        }
        if status == TeamRunStatus::Failed {
            let newer = entry
                .last_failed_team
                .as_ref()
                .map(|(_, at)| created_at > *at)
                .unwrap_or(true);
            if newer {
                entry.last_failed_team = Some((team_id, created_at));
            }
        }
    }
    // 失败原因：取最近失败运行的审计尾部里第一条"失败类"事件（同源的 audit_tail）。
    // 放在聚合里，统计接口/列表/AI 证据三方共用同一口径。
    for entry in stats.values_mut() {
        let Some((team_id, at)) = entry.last_failed_team.clone() else {
            continue;
        };
        let tail = owo_agent_server::workswarm_api::audit_tail(&coordinator, &team_id);
        let reason = tail.iter().find(|item| {
            let event = item["event"].as_str().unwrap_or_default();
            let detail = item["detail"].as_str().unwrap_or_default();
            event.contains("failed")
                || event.contains("budget_exhausted")
                || event.contains("interrupted")
                || event.contains("invalid")
                || detail.contains("失败")
        });
        entry.last_failure = reason.map(|item| FailureTrace {
            team_id: team_id.clone(),
            at: at.clone(),
            event: item["event"].as_str().unwrap_or("team.failed").to_string(),
            detail: item["detail"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect(),
        });
    }
    Ok(stats)
}

/// 运行期"需要修复"提示（薄封装：让列表与统计用同一口径）。
pub(crate) fn attention_for(stats: &TemplateStats) -> Vec<String> {
    owo_agent_core::workflow_attention(&owo_agent_core::WorkflowRunSummary {
        runs: stats.runs,
        succeeded: stats.succeeded,
        failed: stats.failed,
        cancelled: stats.cancelled,
        last_run_failed: stats.last_run_failed,
    })
}

/// 某工作流最近一次运行（用于"沿用上次目标再跑一次"与单条目诊断）。
///
/// 只读协调器已有记录：objective 取自 runs 里的 `GoalRunState.goal.objective`，
/// 缺快照（迁移/清盘）时降级为 None，不编造目标。
pub(crate) async fn latest_run_for_template(
    state: &AppState,
    template_id: &str,
) -> Result<Option<Value>, String> {
    let coordinator = state
        .workswarm
        .coordinator()
        .map_err(|error| error.to_string())?;
    let runs = coordinator
        .list_team_runs()
        .await
        .map_err(|error| error.to_string())?;
    let latest = runs
        .into_iter()
        .filter(|run| run.template_id.as_deref() == Some(template_id))
        .max_by(|left, right| left.created_at.cmp(&right.created_at));
    let Some(run) = latest else {
        return Ok(None);
    };
    let objective = coordinator
        .load_run_state(&run.team_id)
        .ok()
        .map(|state| state.goal.objective)
        .filter(|objective| !objective.trim().is_empty())
        .unwrap_or_else(|| run.team_id.clone());
    Ok(Some(json!({
        "team_id": run.team_id,
        "objective": objective,
        "status": match run.status {
            TeamRunStatus::Created => "created",
            TeamRunStatus::Running => "running",
            TeamRunStatus::AwaitingHuman => "awaiting_human",
            TeamRunStatus::Succeeded => "succeeded",
            TeamRunStatus::Failed => "failed",
            TeamRunStatus::Cancelled => "cancelled",
        },
        "created_at": run.created_at,
        "updated_at": run.updated_at,
    })))
}

/// `GET /workflows/user/{id}/last-run`：最近一次运行（含可复用的 objective）。
async fn workflow_last_run(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(template_id): axum::extract::Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let last = latest_run_for_template(&state, &template_id)
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))?;
    Ok(Json(json!({
        "template_id": template_id,
        "last_run": last,
    })))
}

/// `GET /workflows/stats`：按工作流聚合团队运行情况（含模板名/来源/风险）。
///
/// 查询参数：`sort=runs|failed|success_rate|last`、`order=desc|asc`、`limit=N`。
async fn workflow_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<StatsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let stats = collect_template_stats(&state)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let registry = TeamTemplateRegistry::new(state.data_root.join("workswarm").join("templates"));

    let mut items: Vec<Value> = stats
        .into_iter()
        .map(|(template_id, stats)| {
            let template = registry.get_template(&template_id);
            json!({
                "template_id": template_id,
                "name": template.as_ref().map(|t| t.name.clone()),
                "origin": template
                    .as_ref()
                    .and_then(|t| t.origin.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                "risk_level": template.as_ref().map(|t| {
                    owo_agent_core::workflow_health(t).risk_level
                }),
                "runs": stats.runs,
                "created": stats.created,
                "running": stats.running,
                "awaiting_human": stats.awaiting_human,
                "succeeded": stats.succeeded,
                "failed": stats.failed,
                "cancelled": stats.cancelled,
                "interrupted": stats.interrupted,
                "success_rate": stats.success_rate(),
                "last_run_at": stats.last_created_at,
                // 运行期"需要修复"提示（从未成功/高失败率/长期低成功率/最近失败）。
                "attention": attention_for(&stats),
                "last_run_failed": stats.last_run_failed,
                // 失败定位（本进程审计；跨进程历史可能为空）。
                "last_failure": stats.last_failure.as_ref().map(|trace| json!({
                    "team_id": trace.team_id,
                    "at": trace.at,
                    "event": trace.event,
                    "detail": trace.detail,
                })),
            })
        })
        .collect();
    // 排序：runs（默认）/ failed / success_rate / last；同值按 id 稳定排序。
    let sort = params
        .sort
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "runs".to_string());
    let ascending = params
        .order
        .as_deref()
        .map(|value| value.trim().eq_ignore_ascii_case("asc"))
        .unwrap_or(false);
    let numeric_key = |item: &Value| -> f64 {
        match sort.as_str() {
            "failed" => item["failed"].as_u64().unwrap_or(0) as f64,
            "success_rate" => item["success_rate"].as_f64().unwrap_or(-1.0),
            _ => item["runs"].as_u64().unwrap_or(0) as f64,
        }
    };
    items.sort_by(|left, right| {
        let ordering = if sort == "last" {
            // 时间戳是 RFC3339：字典序即时间序。
            let left_at = left["last_run_at"].as_str().unwrap_or("");
            let right_at = right["last_run_at"].as_str().unwrap_or("");
            if ascending {
                left_at.cmp(right_at)
            } else {
                right_at.cmp(left_at)
            }
        } else {
            let ordering = numeric_key(left)
                .partial_cmp(&numeric_key(right))
                .unwrap_or(std::cmp::Ordering::Equal);
            if ascending {
                ordering
            } else {
                ordering.reverse()
            }
        };
        ordering.then_with(|| {
            left["template_id"]
                .as_str()
                .cmp(&right["template_id"].as_str())
        })
    });
    let total = items.len();
    // summary 覆盖**全部**工作流（分页只影响 templates；否则 limit=1 时合计会被误读）。
    let summary = json!({
        "total_runs": items
            .iter()
            .map(|item| item["runs"].as_u64().unwrap_or(0))
            .sum::<u64>(),
        "succeeded": items
            .iter()
            .map(|item| item["succeeded"].as_u64().unwrap_or(0))
            .sum::<u64>(),
        "failed": items
            .iter()
            .map(|item| item["failed"].as_u64().unwrap_or(0))
            .sum::<u64>(),
    });
    let limit = params.limit.unwrap_or(total).clamp(1, 200);
    let truncated = items.len() > limit;
    items.truncate(limit);
    Ok(Json(json!({
        "templates": items,
        "count": items.len(),
        "total": total,
        "sort": sort,
        "order": if ascending { "asc" } else { "desc" },
        "truncated": truncated,
        "summary": summary,
    })))
}

/// 工作流统计路由（由 build_router 合并）。
pub fn workflow_stats_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/workflows/stats", get(workflow_stats))
        .route("/workflows/user/{id}/last-run", get(workflow_last_run))
        .with_state(state)
}

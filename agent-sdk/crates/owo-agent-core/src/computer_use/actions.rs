use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::executor;
use owo_agent_kernel::task_surface::WindowIdentity;

use super::sim::*;

/// Agent V2 稳定错误码（§5 任务 1.2）。对外错误字符串以 `CODE: 说明` 形式返回，
/// 调用方与前端可以按前缀分类，不必解析中文文案。
pub mod codes {
    pub const EMPTY_GOALS: &str = "EMPTY_GOALS";
    pub const INVALID_ACTION: &str = "INVALID_ACTION";
    pub const STALE_WINDOW_IDENTITY: &str = "STALE_WINDOW_IDENTITY";
    pub const FOREGROUND_CHANGED: &str = "FOREGROUND_CHANGED";
    pub const ANCHOR_AMBIGUOUS: &str = "ANCHOR_AMBIGUOUS";
    pub const TARGET_NOT_FOUND: &str = "TARGET_NOT_FOUND";
    pub const ACTION_NOT_AUTHORIZED: &str = "ACTION_NOT_AUTHORIZED";
    pub const POST_ACTION_VERIFY_FAILED: &str = "POST_ACTION_VERIFY_FAILED";
    pub const ACTION_RECEIPT_STORE_FAILED: &str = "ACTION_RECEIPT_STORE_FAILED";
    pub const BUDGET_EXCEEDED: &str = "BUDGET_EXCEEDED";
}

/// 组装带稳定错误码的错误串。
pub fn coded_error(code: &str, message: impl std::fmt::Display) -> String {
    format!("{code}: {message}")
}

/// 一次有副作用动作的执行回执（§5 任务 1.3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionExecutionReceipt {
    pub action_id: String,
    pub action: String,
    pub observed_window: Option<WindowIdentity>,
    pub started_at: String,
    pub finished_at: String,
    pub outcome: ActionOutcome,
    /// 动作前观察引用（OCR 输出摘要哈希或调用方提供的引用）。
    pub before_observation_ref: Option<String>,
    /// 动作后观察引用。
    pub after_observation_ref: Option<String>,
    /// 指定 UIA 控件的 ValuePattern 逐字读回结果；其他动作始终为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_readback: Option<String>,
    pub verification: ActionVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionOutcome {
    Executed,
    Rejected { reason: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionVerification {
    /// 有明确证据证明目标状态出现。
    Verified,
    /// 没有足够证据（不代表失败）。
    NotVerified,
    /// 观察到明确失败。
    Failed { reason: String },
    /// 按策略跳过，必须带原因。
    Skipped { reason: String },
}

/// 动作前窗口身份复核（§5 任务 1.1）。
///
/// `expected` 是任务开始时授权到的窗口；`current` 是动作前刚刚重新采集的快照。
/// 任一不一致都返回带稳定错误码的 `Err`，调用方必须暂停任务，不得继续发送输入。
pub fn revalidate_window_identity(
    expected: &WindowIdentity,
    current: Option<&WindowIdentity>,
) -> Result<WindowIdentity, String> {
    let Some(current) = current else {
        return Err(coded_error(
            codes::STALE_WINDOW_IDENTITY,
            "无法重新读取当前窗口身份（窗口可能已关闭）",
        ));
    };
    if !expected.same_window(current) {
        return Err(coded_error(
            codes::STALE_WINDOW_IDENTITY,
            format!(
                "窗口身份已变化：授权时 {}/pid {} /hwnd {}，当前 {}/pid {} /hwnd {}",
                expected.app, expected.pid, expected.hwnd, current.app, current.pid, current.hwnd
            ),
        ));
    }
    if !current.foreground {
        return Err(coded_error(
            codes::FOREGROUND_CHANGED,
            format!("目标窗口 {} 已不在前台，停止向它发送输入", current.app),
        ));
    }
    Ok(current.clone())
}

/// 观察内容的稳定引用（用于回执里关联前后观察，不把整屏 OCR 塞进事件表）。
pub fn observation_ref(ocr: &Value) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(ocr).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    format!("sha256:{:x}", digest)
}
pub fn desktop_click(x: i32, y: i32) -> Result<(), String> {
    executor::click_at_screen(x, y)
}

/// 公开同步入口：注入 UTF-16 文本。
pub fn desktop_type(text: &str) -> Result<(), String> {
    executor::send_unicode(text)
}

/// 公开同步入口：发送单个按键（enter/tab 等）。
pub fn desktop_key(key: &str) -> Result<(), String> {
    executor::send_shortcut(key)
}

/// 公开同步入口：发送组合键。
pub fn desktop_shortcut(combo: &str) -> Result<(), String> {
    executor::send_shortcut(combo)
}

/// 公开同步入口：启动应用/URL。
pub fn desktop_launch(target: &str) -> Result<(), String> {
    executor::launch_target(target)
}

/// 公开同步入口：屏幕坐标处滚轮（正上负下）。
pub fn desktop_scroll(x: i32, y: i32, delta: i32) -> Result<(), String> {
    executor::scroll_at_screen(x, y, delta)
}

// ---------- m4d 任务级门禁与审批闭环 ----------

/// 任务动作门禁：任何 desktop_* 动作执行前必须通过本校验（m4d 审批版）。
///
/// 校验顺序：任务存在 → 状态可执行（含超时）→ 动作在允许集 → 目标应用匹配 →
/// 敏感 UI 熔断 → 动作预算。任一失败返回 Err（**不执行动作**），并落审计
/// `permission/deny`（approved=false）；全部通过时记审计 `permission/grant`。
///
/// `sensitive_sample` 为当前界面的敏感检测样本 `(name, role, ocr_text)`；
/// 命中密码/支付/验证码等关键词时任务置 Fused 并要求人工接管。
pub fn task_gate_check(
    registry: &crate::computer_task::ComputerTaskRegistry,
    mut audit: Option<&mut crate::audit::AuditLog>,
    session_id: &str,
    task_id: &str,
    action: &str,
    actual_app: &str,
    sensitive_sample: Option<(&str, &str, &str)>,
) -> Result<(), String> {
    let deny = |audit: &mut crate::audit::AuditLog, detail: String| {
        audit.record(
            session_id,
            "permission",
            Some(action.to_string()),
            Some(false),
            detail,
        );
    };
    // 1. 任务存在 + 状态可执行（含超时自动暂停）。
    if let Err(e) = registry.check_can_execute(task_id) {
        if let Some(a) = audit.as_mut() {
            deny(a, format!("computer-use 门禁拒绝：{e}"));
        }
        return Err(e);
    }
    // 2. 动作允许集。
    if let Err(e) = registry.check_action_allowed(task_id, action) {
        if let Some(a) = audit.as_mut() {
            deny(a, format!("computer-use 门禁拒绝：{e}"));
        }
        return Err(e);
    }
    // 3. 目标应用匹配。
    match registry.target_matches(task_id, actual_app) {
        Ok(true) => {}
        Ok(false) => {
            let detail = format!(
                "computer-use 门禁拒绝：任务 {task_id} 目标应用不匹配（任务声明与当前 {actual_app}）"
            );
            if let Some(a) = audit.as_mut() {
                deny(a, detail);
            }
            return Err(format!(
                "目标应用 {actual_app} 不在任务 {task_id} 授权范围内"
            ));
        }
        Err(e) => {
            if let Some(a) = audit.as_mut() {
                deny(a, format!("computer-use 门禁拒绝：{e}"));
            }
            return Err(e);
        }
    }
    // 4. 敏感 UI 熔断。
    if let Some((name, role, ocr_text)) = sensitive_sample {
        if let Some(reason) = crate::computer_task::sensitive_ui_hit(name, role, ocr_text) {
            let detail = format!("computer-use 敏感熔断：{reason}，任务 {task_id} 置 Fused");
            let _ = registry.fuse(task_id, &detail);
            if let Some(a) = audit.as_mut() {
                deny(a, detail);
            }
            return Err(format!(
                "敏感 UI 熔断：{reason}；任务 {task_id} 已暂停，需人工接管后 resume"
            ));
        }
    }
    // 5. 动作预算。
    if let Err(e) = registry.check_action_budget(task_id) {
        if let Some(a) = audit.as_mut() {
            deny(a, format!("computer-use 门禁拒绝：{e}"));
        }
        return Err(e);
    }
    if let Some(a) = audit.as_mut() {
        a.record(
            session_id,
            "permission",
            Some(action.to_string()),
            Some(true),
            format!("computer-use 任务 {task_id} 动作 {action} 已获授权（目标 {actual_app}）"),
        );
    }
    Ok(())
}

/// 对 OCR lines 做整屏敏感扫描，返回首个命中说明（无命中返回 None）。
pub fn scan_ui_sensitive(ocr: &Value) -> Option<String> {
    let lines = ocr.get("lines")?.as_array()?;
    for line in lines {
        let text = line.get("text").and_then(Value::as_str).unwrap_or("");
        let role = line.get("role_hint").and_then(Value::as_str).unwrap_or("");
        if let Some(reason) = crate::computer_task::sensitive_ui_hit(text, role, "") {
            return Some(reason);
        }
    }
    None
}

/// 门禁后屏幕单击（sim/真实统一走各自实现）。
#[allow(clippy::too_many_arguments)]
pub async fn desktop_click_gated(
    registry: &crate::computer_task::ComputerTaskRegistry,
    mut audit: Option<&mut crate::audit::AuditLog>,
    session_id: &str,
    task_id: &str,
    app: &str,
    x: i32,
    y: i32,
    sensitive_sample: Option<(&str, &str, &str)>,
) -> Result<Value, String> {
    task_gate_check(
        registry,
        audit.as_deref_mut(),
        session_id,
        task_id,
        "desktop_click",
        app,
        sensitive_sample,
    )?;
    let result = if on_sim_surface() {
        sim_post("click", json!({ "x": x, "y": y })).await?
    } else {
        executor::click_at_screen(x, y)?;
        json!({ "clicked": [x, y] })
    };
    registry.record_action(task_id);
    if let Some(a) = audit.as_mut() {
        a.record(
            session_id,
            "tool_call",
            Some("desktop_click".to_string()),
            Some(true),
            format!("任务 {task_id} 点击 ({x},{y})"),
        );
    }
    Ok(result)
}

/// 门禁后注入文本。
#[allow(clippy::too_many_arguments)]
pub async fn desktop_type_gated(
    registry: &crate::computer_task::ComputerTaskRegistry,
    mut audit: Option<&mut crate::audit::AuditLog>,
    session_id: &str,
    task_id: &str,
    app: &str,
    text: &str,
    sensitive_sample: Option<(&str, &str, &str)>,
) -> Result<Value, String> {
    task_gate_check(
        registry,
        audit.as_deref_mut(),
        session_id,
        task_id,
        "desktop_type",
        app,
        sensitive_sample,
    )?;
    let result = if on_sim_surface() {
        sim_post("type", json!({ "text": text })).await?
    } else {
        executor::send_unicode(text)?;
        json!({ "typed_chars": text.chars().count() })
    };
    registry.record_action(task_id);
    if let Some(a) = audit.as_mut() {
        a.record(
            session_id,
            "tool_call",
            Some("desktop_type".to_string()),
            Some(true),
            format!("任务 {task_id} 输入 {} 字符", text.chars().count()),
        );
    }
    Ok(result)
}

/// 门禁后发送按键（enter/tab/backspace 等）。
#[allow(clippy::too_many_arguments)]
pub async fn desktop_key_gated(
    registry: &crate::computer_task::ComputerTaskRegistry,
    mut audit: Option<&mut crate::audit::AuditLog>,
    session_id: &str,
    task_id: &str,
    app: &str,
    key: &str,
    sensitive_sample: Option<(&str, &str, &str)>,
) -> Result<Value, String> {
    task_gate_check(
        registry,
        audit.as_deref_mut(),
        session_id,
        task_id,
        "desktop_key",
        app,
        sensitive_sample,
    )?;
    let result = if on_sim_surface() {
        sim_post("key", json!({ "key": key })).await?
    } else {
        executor::send_shortcut(key)?;
        json!({ "key": key })
    };
    registry.record_action(task_id);
    if let Some(a) = audit.as_mut() {
        a.record(
            session_id,
            "tool_call",
            Some("desktop_key".to_string()),
            Some(true),
            format!("任务 {task_id} 按键 {key}"),
        );
    }
    Ok(result)
}

/// 门禁后滚轮。
#[allow(clippy::too_many_arguments)]
pub async fn desktop_scroll_gated(
    registry: &crate::computer_task::ComputerTaskRegistry,
    mut audit: Option<&mut crate::audit::AuditLog>,
    session_id: &str,
    task_id: &str,
    app: &str,
    x: i32,
    y: i32,
    delta: i32,
    sensitive_sample: Option<(&str, &str, &str)>,
) -> Result<Value, String> {
    task_gate_check(
        registry,
        audit.as_deref_mut(),
        session_id,
        task_id,
        "desktop_scroll",
        app,
        sensitive_sample,
    )?;
    let result = if on_sim_surface() {
        sim_post("scroll", json!({ "x": x, "y": y, "delta": delta })).await?
    } else {
        executor::scroll_at_screen(x, y, delta)?;
        json!({ "scrolled": [x, y, delta] })
    };
    registry.record_action(task_id);
    if let Some(a) = audit.as_mut() {
        a.record(
            session_id,
            "tool_call",
            Some("desktop_scroll".to_string()),
            Some(true),
            format!("任务 {task_id} 滚轮 ({x},{y},{delta})"),
        );
    }
    Ok(result)
}

/// 门禁后启动应用/URL（任务已批准且目标应用匹配时才允许）。
#[allow(clippy::too_many_arguments)]
pub async fn desktop_launch_gated(
    registry: &crate::computer_task::ComputerTaskRegistry,
    mut audit: Option<&mut crate::audit::AuditLog>,
    session_id: &str,
    task_id: &str,
    app: &str,
    target: &str,
) -> Result<Value, String> {
    task_gate_check(
        registry,
        audit.as_deref_mut(),
        session_id,
        task_id,
        "desktop_launch",
        app,
        None,
    )?;
    let result = if on_sim_surface() {
        json!({ "launched": target, "surface": "sim" })
    } else {
        executor::launch_target(target)?;
        json!({ "launched": target })
    };
    registry.record_action(task_id);
    if let Some(a) = audit.as_mut() {
        a.record(
            session_id,
            "tool_call",
            Some("desktop_launch".to_string()),
            Some(true),
            format!("任务 {task_id} 启动 {target}"),
        );
    }
    Ok(result)
}

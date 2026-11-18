use chrono::Utc;
use serde_json::{json, Value};
use std::time::Duration;

use crate::executor;
use owo_agent_kernel::task_surface::{UiaEditIdentity, WindowIdentity};

use super::actions::*;
use super::sim::*;

/// 闭环单步目标：感知到 `anchor_text` 后执行动作，并用 `verify_text` 验证。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TaskGoal {
    /// 定位锚点（OCR 文本，大小写不敏感子串匹配）。
    pub anchor_text: String,
    /// 动作类型：click / type / key。
    pub action: String,
    /// 动作载荷：type 的文本或 key 名（click 忽略）。
    pub value: String,
    /// 动作后的验证文本（可选；须出现在与动作前不同的 OCR 观察中）。
    pub verify_text: Option<String>,
}

impl TaskGoal {
    /// 单个目标的静态校验（§5 任务 1.2）。
    pub fn validate(&self) -> Result<(), String> {
        if self.anchor_text.trim().is_empty() {
            return Err(coded_error(
                codes::INVALID_ACTION,
                "anchor_text 为空，无法定位可验证的语义锚点",
            ));
        }
        match self.action.as_str() {
            "click" => Ok(()),
            "type" | "set_value" | "key" | "launch" => {
                if self.value.trim().is_empty() {
                    return Err(coded_error(
                        codes::INVALID_ACTION,
                        format!("动作 {} 需要非空 value", self.action),
                    ));
                }
                Ok(())
            }
            other => Err(coded_error(
                codes::INVALID_ACTION,
                format!("闭环不支持的动作：{other}"),
            )),
        }
    }
}

/// 目标集合校验：空目标永远不能进入成功路径（§5 任务 1.2）。
pub fn validate_goals(goals: &[TaskGoal]) -> Result<(), String> {
    if goals.is_empty() {
        return Err(coded_error(
            codes::EMPTY_GOALS,
            "目标列表为空，拒绝执行（空目标不能因为没有动作失败而被当作成功）",
        ));
    }
    for (index, goal) in goals.iter().enumerate() {
        goal.validate()
            .map_err(|error| format!("第 {} 步无效：{error}", index + 1))?;
    }
    Ok(())
}

/// 闭环动作名 → 门禁动作名。
pub fn gate_action_name(action: &str) -> Result<&'static str, String> {
    match action {
        "click" => Ok("desktop_click"),
        "type" | "set_value" => Ok("desktop_type"),
        "key" => Ok("desktop_key"),
        "launch" => Ok("desktop_launch"),
        other => Err(coded_error(
            codes::INVALID_ACTION,
            format!("闭环不支持的动作：{other}"),
        )),
    }
}

/// 闭环执行报告。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskReport {
    pub task_id: String,
    pub steps: usize,
    pub state: crate::computer_task::TaskState,
    pub detail: String,
    /// 每个有副作用动作的执行回执（§5 任务 1.3）。
    pub receipts: Vec<ActionExecutionReceipt>,
}

// `TaskSurface` 契约已下沉到内核（M5 依赖倒置）：desktop_env 外迁需要它，
// 而 desktop_env 若依赖 computer_use 就会成环。实现体仍在本模块。
pub use owo_agent_kernel::TaskSurface;

/// owo-sim-qq HTTP 模拟面（`OWO_SIM_QQ_URL` 指向模拟窗口）。
#[derive(Debug)]
pub struct SimTaskSurface {
    base: String,
}

impl SimTaskSurface {
    pub fn new() -> Result<Self, String> {
        let base = sim_base_url().ok_or("模拟环境未配置 OWO_SIM_QQ_URL")?;
        Ok(Self { base })
    }
}

/// 真实桌面面：OCR 走本地引擎（Media.Ocr / PP-OCRv6 / 本地 ONNX），
/// 动作走 executor（SendInput / UIA / 启动）。用于已授权任务在真实桌面的闭环。
#[derive(Debug, Default)]
pub struct RealTaskSurface;

#[async_trait::async_trait]
impl TaskSurface for RealTaskSurface {
    fn app(&self) -> String {
        crate::platform::poll_foreground_app()
            .map(|(app_id, _)| app_id)
            .unwrap_or_default()
    }

    fn window_identity(&self) -> Option<WindowIdentity> {
        crate::platform::foreground_window_identity()
    }

    async fn ocr(&mut self) -> Result<Value, String> {
        ocr_screen(0).await
    }

    async fn ocr_window(&mut self, hwnd: i64) -> Result<Value, String> {
        super::tools::ocr_window_at_screen_coordinates(hwnd as isize).await
    }

    async fn accessible_anchors(&mut self, hwnd: i64, name: &str) -> Result<Vec<Value>, String> {
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || {
            crate::ui_tree_for_hwnd(hwnd as isize, 8, 160)
                .unwrap_or_default()
                .into_iter()
                .filter(|node| node.name.trim() == name && node.width > 0 && node.height > 0)
                .map(|node| json!({"x":node.x,"y":node.y,"width":node.width,"height":node.height}))
                .collect()
        })
        .await
        .map_err(|error| error.to_string())
    }

    async fn accessible_observation(
        &mut self,
        hwnd: i64,
        name: &str,
    ) -> Result<Option<Value>, String> {
        let hwnd = hwnd as isize;
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || crate::accessible_anchor_observation(hwnd, name.trim()))
            .await
            .map_err(|error| error.to_string())
    }

    async fn click(&mut self, x: i32, y: i32) -> Result<(), String> {
        executor::click_at_screen(x, y)
    }

    async fn type_text(&mut self, text: &str) -> Result<(), String> {
        executor::send_unicode(text)
    }

    #[cfg(target_os = "windows")]
    async fn set_value(
        &mut self,
        identity: &UiaEditIdentity,
        value: &str,
    ) -> Result<String, String> {
        let identity = identity.clone();
        let value = value.to_owned();
        tokio::task::spawn_blocking(move || {
            executor::WindowsUiaSource::set_named_edit_value(&identity, &value)
        })
        .await
        .map_err(|error| error.to_string())?
    }

    async fn key(&mut self, key: &str) -> Result<(), String> {
        executor::send_shortcut(key)
    }

    async fn launch(&mut self, target: &str) -> Result<(), String> {
        executor::launch_target(target)
    }
}

#[async_trait::async_trait]
impl TaskSurface for SimTaskSurface {
    fn app(&self) -> String {
        "owo-sim-qq".to_string()
    }

    /// 模拟面没有真实句柄，但给出稳定的合成身份，使"动作前复核"路径同样被执行。
    fn window_identity(&self) -> Option<WindowIdentity> {
        Some(WindowIdentity {
            app: "owo-sim-qq".to_string(),
            pid: 0,
            hwnd: 1,
            title: "OwO 模拟QQ".to_string(),
            foreground: true,
            observed_at: Utc::now().to_rfc3339(),
        })
    }

    async fn ocr(&mut self) -> Result<Value, String> {
        let base = self.base.trim_end_matches('/');
        let response = reqwest::get(format!("{base}/ocr"))
            .await
            .map_err(|e| format!("模拟面 OCR 失败：{e}"))?;
        let value: Value = response
            .json()
            .await
            .map_err(|e| format!("模拟面 OCR 响应解析失败：{e}"))?;
        let has_lines = value
            .get("lines")
            .and_then(Value::as_array)
            .map(|lines| !lines.is_empty())
            .unwrap_or(false);
        if has_lines {
            Ok(value)
        } else {
            Err("模拟面 OCR 不可用".to_string())
        }
    }

    async fn click(&mut self, x: i32, y: i32) -> Result<(), String> {
        sim_post_at(&self.base, "click", json!({ "x": x, "y": y })).await?;
        Ok(())
    }

    async fn type_text(&mut self, text: &str) -> Result<(), String> {
        sim_post_at(&self.base, "type", json!({ "text": text })).await?;
        Ok(())
    }

    async fn key(&mut self, key: &str) -> Result<(), String> {
        sim_post_at(&self.base, "key", json!({ "key": key })).await?;
        Ok(())
    }

    async fn launch(&mut self, _target: &str) -> Result<(), String> {
        // 模拟面无独立启动端点；启动语义由真实面/后续接线承载。
        Ok(())
    }
}

/// 感知闭环（指定 surface）：截图/OCR 感知 → 定位锚点 → 门禁动作 → 验证 → 下一步/完成，每步审计。
///
/// 前置：任务已 Approved 或 Running（Pending 返回 Err，需用户先批准）。
/// 敏感 UI（密码/支付/验证码）在每步感知后扫描，命中即 Fused 熔断并要求人工接管。
/// 任一动作被门禁拒绝（未批准/越界应用/超预算/超时）立即停止并返回错误。
pub async fn run_approved_task_on(
    registry: &crate::computer_task::ComputerTaskRegistry,
    audit: &mut crate::audit::AuditLog,
    session_id: &str,
    task_id: &str,
    goals: &[TaskGoal],
    surface: &mut dyn TaskSurface,
) -> Result<TaskReport, String> {
    let mut discard = |_: ActionReceiptStage, _: &ActionExecutionReceipt| Ok(());
    run_approved_task_on_with_receipts(
        registry,
        audit,
        session_id,
        task_id,
        goals,
        surface,
        &mut discard,
    )
    .await
}

/// 执行返回时先报告事实；观察完成后再报告验证状态。不把“执行”当作“验证通过”。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionReceiptStage {
    Execution,
    Observation,
}

/// 带逐步回执接收器的闭环。接收器应同步持久化；失败立即暂停，禁止继续输入。
///
/// 执行事实在任何动作后观察的 await 之前发布，因此后续验证错误或超时不会
/// 丢失已完成动作。原入口仍保留，旧调用者继续自行负责持久化。
pub async fn run_approved_task_on_with_receipts(
    registry: &crate::computer_task::ComputerTaskRegistry,
    audit: &mut crate::audit::AuditLog,
    session_id: &str,
    task_id: &str,
    goals: &[TaskGoal],
    surface: &mut dyn TaskSurface,
    on_receipt: &mut (dyn FnMut(ActionReceiptStage, &ActionExecutionReceipt) -> Result<(), String>
              + Send),
) -> Result<TaskReport, String> {
    validate_goals(goals)?;
    let result = execute_approved_task_on_with_receipts(
        registry, audit, session_id, task_id, goals, surface, on_receipt,
    )
    .await;
    if let Err(reason) = &result {
        // 原子检查可执行状态，避免覆盖并发取消或敏感熔断。
        let _ = registry.pause_if_executable(task_id, reason);
    }
    result
}

async fn execute_approved_task_on_with_receipts(
    registry: &crate::computer_task::ComputerTaskRegistry,
    audit: &mut crate::audit::AuditLog,
    session_id: &str,
    task_id: &str,
    goals: &[TaskGoal],
    surface: &mut dyn TaskSurface,
    on_receipt: &mut (dyn FnMut(ActionReceiptStage, &ActionExecutionReceipt) -> Result<(), String>
              + Send),
) -> Result<TaskReport, String> {
    // 0. 空目标/无效目标在触碰任何桌面状态之前拒绝（§5 任务 1.2）。
    validate_goals(goals)?;
    // 前置门禁：以首个动作做整体授权检查（任务须已批准）。
    let task = registry
        .get(task_id)
        .ok_or_else(|| format!("任务 {task_id} 不存在"))?;
    if task.state != crate::computer_task::TaskState::Approved
        && task.state != crate::computer_task::TaskState::Running
    {
        return Err(format!(
            "任务 {task_id} 状态 {:?}，需用户先批准（Pending→approve）",
            task.state
        ));
    }
    // 每个动作都重新读取前台应用（不再沿用任务开始时的快照）。
    let first_action = gate_action_name(&goals[0].action)?;
    task_gate_check(
        registry,
        Some(&mut *audit),
        session_id,
        task_id,
        first_action,
        &surface.app(),
        None,
    )?;
    // 授权时的窗口身份基线；真实桌面面必须提供，模拟/Mock 允许为 None。
    let expected_identity = surface.window_identity();
    if registry.get(task_id).unwrap().state != crate::computer_task::TaskState::Running {
        registry.start(task_id)?;
    }
    audit.record(
        session_id,
        "computer_task",
        Some("start".to_string()),
        Some(true),
        format!("任务 {task_id} 感知闭环启动（{} 步目标）", goals.len()),
    );

    let mut receipts: Vec<ActionExecutionReceipt> = Vec::new();
    let mut steps = 0usize;
    for goal in goals {
        // 1. 动作前窗口身份复核（§5 任务 1.1）。
        let observed_window = match &expected_identity {
            Some(expected) => {
                match revalidate_window_identity(expected, surface.window_identity().as_ref()) {
                    Ok(current) => Some(current),
                    Err(reason) => {
                        let detail = format!("动作前窗口复核失败（第 {} 步）：{reason}", steps + 1);
                        let _ = registry.pause(task_id, &detail);
                        audit.record(
                            session_id,
                            "computer_task",
                            Some("window_changed".to_string()),
                            Some(false),
                            detail,
                        );
                        return Err(reason);
                    }
                }
            }
            None => surface.window_identity(),
        };
        // 2. 感知：普通动作仍要求整屏 OCR；仅 `set_value` 在已绑定窗口、
        // 具名 UIA 控件有效时允许 OCR 为空，用 UIA 树作为受控观察。
        let uia_name = goal.anchor_text.strip_prefix("uia:").map(str::trim);
        let uia_set_value = goal.action == "set_value" && uia_name.is_some();
        let mut after_ocr = None;
        let (ocr, before_observation_ref, uia_observation) = if uia_set_value {
            let window = observed_window.as_ref().ok_or_else(|| {
                coded_error(codes::STALE_WINDOW_IDENTITY, "无法读取无障碍目标窗口")
            })?;
            let name = uia_name.unwrap_or_default();
            let observation = surface
                .accessible_observation(window.hwnd, name)
                .await?
                .ok_or_else(|| {
                    coded_error(
                        codes::TARGET_NOT_FOUND,
                        format!("第 {} 步未找到安全的可写 UIA 编辑控件「{name}」", steps + 1),
                    )
                })?;
            let uia_ref = observation
                .get("observation_ref")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !uia_ref.starts_with("sha256:") {
                return Err(coded_error(
                    codes::STALE_WINDOW_IDENTITY,
                    "UIA 观察缺少窗口绑定的稳定引用",
                ));
            }
            let mut ocr = surface.ocr().await.unwrap_or_else(|_| observation.clone());
            if let Value::Object(map) = &mut ocr {
                map.insert("uia_observation_ref".into(), Value::String(uia_ref.clone()));
            }
            // 写值绑定必须来自已绑定窗口的 UIA 观察；整屏 OCR 只用于敏感扫描，
            // 其文本行不含控件身份，不能当作可写控件候选。
            (ocr, Some(uia_ref), Some(observation))
        } else {
            let ocr = surface.ocr().await.inspect_err(|error| {
                let _ = registry.pause(task_id, error);
            })?;
            let reference = observation_ref(&ocr);
            (ocr, Some(reference), None)
        };
        // 3. 敏感扫描（整屏 OCR）→ 熔断；UIA-only 观察不含自由文本，另由控件身份守卫。
        if let Some(reason) = scan_ui_sensitive(&ocr) {
            let detail = format!("敏感熔断（第 {} 步感知）：{reason}", steps + 1);
            let _ = registry.fuse(task_id, &detail);
            audit.record(
                session_id,
                "computer_task",
                Some("fuse".to_string()),
                Some(false),
                detail.clone(),
            );
            return Err(detail);
        }
        // 4. 定位锚点：多个候选不得静默选第一个（§5 任务 1.4）。
        let candidates = if let Some(name) = uia_name {
            let window = observed_window.as_ref().ok_or_else(|| {
                coded_error(codes::STALE_WINDOW_IDENTITY, "无法读取无障碍目标窗口")
            })?;
            if uia_set_value {
                uia_observation
                    .as_ref()
                    .and_then(|observation| observation.get("lines"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|line| {
                        line.get("text").and_then(Value::as_str) == Some(name)
                            && line.get("hwnd").and_then(Value::as_i64) == Some(window.hwnd)
                            && line.get("has_value_pattern").and_then(Value::as_bool) == Some(true)
                    })
                    .collect()
            } else {
                surface.accessible_anchors(window.hwnd, name).await?
            }
        } else {
            find_ocr_lines(&ocr, &goal.anchor_text, "")
        };
        if candidates.is_empty() {
            return Err(coded_error(
                codes::TARGET_NOT_FOUND,
                format!("第 {} 步未找到锚点「{}」", steps + 1, goal.anchor_text),
            ));
        }
        if candidates.len() > 1 {
            return Err(coded_error(
                codes::ANCHOR_AMBIGUOUS,
                format!(
                    "第 {} 步锚点「{}」命中 {} 个候选，请补充更精确的锚点或角色过滤",
                    steps + 1,
                    goal.anchor_text,
                    candidates.len()
                ),
            ));
        }
        let line = &candidates[0];
        let x = line.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
        let y = line.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
        let w = line.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
        let h = line.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
        let (cx, cy) = (x + w / 2, y + h / 2);
        // 5. 门禁动作（状态/允许集/目标应用/敏感/预算，拒绝即停）。
        let action_name = gate_action_name(&goal.action)?;
        if let Err(error) = task_gate_check(
            registry,
            Some(&mut *audit),
            session_id,
            task_id,
            action_name,
            &surface.app(),
            None,
        ) {
            // A foreground switch can happen while OCR is in flight, before
            // the final identity check below. Preserve its stable window code
            // rather than leaving a live ComputerTask behind a generic error.
            let reason = expected_identity
                .as_ref()
                .and_then(|expected| {
                    revalidate_window_identity(expected, surface.window_identity().as_ref()).err()
                })
                .unwrap_or(error);
            let _ = registry.pause(task_id, &reason);
            return Err(reason);
        }
        // 6. 动作执行前最后再确认一次前台窗口没有切换。
        if let Some(expected) = &expected_identity {
            if let Err(reason) =
                revalidate_window_identity(expected, surface.window_identity().as_ref())
            {
                let detail = format!("动作执行前窗口复核失败（第 {} 步）：{reason}", steps + 1);
                let _ = registry.pause(task_id, &detail);
                audit.record(
                    session_id,
                    "computer_task",
                    Some("window_changed".to_string()),
                    Some(false),
                    detail,
                );
                return Err(reason);
            }
        }
        let action_id = format!("{task_id}:{}:{}", steps + 1, uuid::Uuid::new_v4());
        let started_at = Utc::now().to_rfc3339();
        let mut value_readback = None;
        let mut readback_mismatch = None;
        let outcome = match goal.action.as_str() {
            "click" => surface.click(cx, cy).await,
            "type" => surface.type_text(&goal.value).await,
            "set_value" => match (&observed_window, uia_name, candidates.first()) {
                (Some(window), Some(name), Some(bound)) => {
                    let identity = UiaEditIdentity {
                        hwnd: window.hwnd,
                        pid: window.pid,
                        name: name.to_string(),
                        automation_id: bound
                            .get("automation_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        native_window_handle: bound
                            .get("native_window_handle")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    };
                    let result = surface.set_value(&identity, &goal.value).await;
                    if let Ok(readback) = &result {
                        if readback != &goal.value {
                            readback_mismatch =
                                Some(format!("UIA ValuePattern 读回与请求值不一致：{readback:?}"));
                        }
                        value_readback = Some(readback.clone());
                    }
                    result.map(|_| ())
                }
                _ => Err(coded_error(
                    codes::STALE_WINDOW_IDENTITY,
                    "缺少受控 UIA 目标",
                )),
            },
            "key" => surface.key(&goal.value).await,
            "launch" => surface.launch(&goal.value).await,
            _ => unreachable!("validate_goals 已拒绝未知动作"),
        };
        let finished_at = Utc::now().to_rfc3339();
        if let Err(error) = outcome {
            let detail = format!("第 {} 步动作执行失败：{error}", steps + 1);
            // 持久化/执行边界的稳定错误码不能被上下文文本吞掉。
            let code = error.split_once(':').map(|(code, _)| code).filter(|code| {
                !code.is_empty() && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
            });
            let reason = code.map_or_else(|| detail.clone(), |code| coded_error(code, &detail));
            let receipt = ActionExecutionReceipt {
                action_id,
                action: goal.action.clone(),
                observed_window,
                started_at,
                finished_at,
                outcome: ActionOutcome::Failed {
                    reason: error.clone(),
                },
                before_observation_ref,
                after_observation_ref: None,
                value_readback: None,
                verification: ActionVerification::Failed {
                    reason: error.clone(),
                },
            };
            publish_action_receipt(
                registry,
                task_id,
                &receipt,
                ActionReceiptStage::Execution,
                on_receipt,
            )?;
            receipts.push(receipt);
            let _ = registry.pause(task_id, &reason);
            audit.record(
                session_id,
                "computer_task",
                Some("action_failed".to_string()),
                Some(false),
                reason.clone(),
            );
            return Err(reason);
        }
        if let Some(reason) = readback_mismatch {
            let _ = registry.pause(task_id, &reason);
            return Err(coded_error(codes::POST_ACTION_VERIFY_FAILED, reason));
        }
        registry.record_action(task_id);
        steps += 1;
        let mut receipt = ActionExecutionReceipt {
            action_id,
            action: goal.action.clone(),
            observed_window,
            started_at,
            finished_at,
            outcome: ActionOutcome::Executed,
            before_observation_ref: before_observation_ref.clone(),
            after_observation_ref: None,
            value_readback,
            verification: ActionVerification::NotVerified,
        };
        publish_action_receipt(
            registry,
            task_id,
            &receipt,
            ActionReceiptStage::Execution,
            on_receipt,
        )?;
        // 7. 动作后重新观察 + 验证（§5 任务 1.3）。set_value 由执行器逐字读回，
        // 不把空 OCR 伪装成验证证据；未声明 verify_text 时按既有语义跳过。
        if !uia_set_value {
            after_ocr = surface.ocr().await.ok();
        }
        let verification = if uia_set_value {
            ActionVerification::Verified
        } else {
            match &goal.verify_text {
                None => ActionVerification::Skipped {
                    reason: "目标未声明 verify_text".to_string(),
                },
                Some(expected) => {
                    // Preexisting text in an unchanged frame cannot prove the action worked.
                    // Reobserve without replaying the action until both the text and changed
                    // observation are present. Invisible effects require independent verification
                    // (omit verify_text so this backend returns a candidate instead of accepting).
                    let mut verified = after_ocr.as_ref().is_some_and(|ocr| {
                        changed_observation_contains_text(
                            ocr,
                            expected,
                            before_observation_ref.as_deref(),
                        )
                    });
                    if !verified {
                        if let Some(observation) = observe_text_appears_on(
                            surface,
                            expected,
                            before_observation_ref.as_deref(),
                        )
                        .await?
                        {
                            after_ocr = Some(observation);
                            verified = true;
                        }
                    }
                    if verified {
                        ActionVerification::Verified
                    } else {
                        ActionVerification::NotVerified
                    }
                }
            }
        };
        let after_observation_ref = if uia_set_value {
            before_observation_ref.clone()
        } else {
            after_ocr.as_ref().map(observation_ref)
        };
        let verified = !matches!(verification, ActionVerification::NotVerified);
        receipt.after_observation_ref = after_observation_ref;
        receipt.verification = verification;
        publish_action_receipt(
            registry,
            task_id,
            &receipt,
            ActionReceiptStage::Observation,
            on_receipt,
        )?;
        receipts.push(receipt);
        audit.record(
            session_id,
            "computer_task",
            Some("step".to_string()),
            Some(verified),
            format!("任务 {task_id} 第 {steps} 步完成（{action_name}）@({cx},{cy})"),
        );
        if !verified {
            let expected = goal.verify_text.clone().unwrap_or_default();
            let detail =
                format!("验证失败（第 {steps} 步）：缺少与动作前不同且包含「{expected}」的观察");
            let _ = registry.pause(task_id, &detail);
            return Err(coded_error(codes::POST_ACTION_VERIFY_FAILED, detail));
        }
        // 8. 超时/预算兜底（超时自动暂停并报错）。
        registry.check_can_execute(task_id)?;
    }
    let _ = registry.complete(task_id);
    audit.record(
        session_id,
        "computer_task",
        Some("complete".to_string()),
        Some(true),
        format!("任务 {task_id} 闭环完成，共 {steps} 步"),
    );
    Ok(TaskReport {
        task_id: task_id.to_string(),
        steps,
        state: crate::computer_task::TaskState::Completed,
        detail: "全部目标完成".to_string(),
        receipts,
    })
}

/// 感知闭环（模拟/沙箱面便捷入口）：`OWO_SIM_QQ_URL` 指向 owo-sim-qq，
/// 在沙箱应用内跑通"打开应用→输入→保存/发送→验证"最小闭环。
///
/// 真实桌面需显式授权（本轮不提供）；逻辑与 [`run_approved_task_on`] 完全一致。
pub async fn run_approved_task(
    registry: &crate::computer_task::ComputerTaskRegistry,
    audit: &mut crate::audit::AuditLog,
    session_id: &str,
    task_id: &str,
    goals: &[TaskGoal],
) -> Result<TaskReport, String> {
    let mut surface = SimTaskSurface::new()?;
    run_approved_task_on(registry, audit, session_id, task_id, goals, &mut surface).await
}

fn publish_action_receipt(
    registry: &crate::computer_task::ComputerTaskRegistry,
    task_id: &str,
    receipt: &ActionExecutionReceipt,
    stage: ActionReceiptStage,
    sink: &mut (dyn FnMut(ActionReceiptStage, &ActionExecutionReceipt) -> Result<(), String>
              + Send),
) -> Result<(), String> {
    sink(stage, receipt).map_err(|error| {
        let reason = coded_error(codes::ACTION_RECEIPT_STORE_FAILED, error);
        let _ = registry.pause(task_id, &reason);
        reason
    })
}

fn changed_observation_contains_text(
    ocr: &Value,
    expected: &str,
    before_ref: Option<&str>,
) -> bool {
    let after_ref = observation_ref(ocr);
    before_ref != Some(after_ref.as_str()) && find_ocr_line(ocr, expected, "").is_some()
}

/// 返回发生变化且包含 `needle` 的 OCR 观察（重试 5 次，间隔 50ms），不重放动作。
async fn observe_text_appears_on(
    surface: &mut dyn TaskSurface,
    needle: &str,
    before_ref: Option<&str>,
) -> Result<Option<Value>, String> {
    for _ in 0..5 {
        if let Ok(ocr) = surface.ocr().await {
            if changed_observation_contains_text(&ocr, needle, before_ref) {
                return Ok(Some(ocr));
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(None)
}

//! M4d computer-use 审批版闭环契约测试。
//!
//! 覆盖：动作门禁（未批准/应用不匹配/超时/预算/敏感熔断）、resume 恢复、
//! 感知闭环（Mock 面全流程）、每步审计。全部无网络/无真实桌面依赖。

use owo_agent_core::audit::AuditLog;
use owo_agent_core::computer_task::{
    sensitive_ui_hit, ComputerTask, ComputerTaskRegistry, TaskState,
};
use owo_agent_core::computer_use::{
    run_approved_task_on, scan_ui_sensitive, task_gate_check, TaskGoal, TaskSurface,
};
use serde_json::{json, Value};
use std::time::Duration;

fn sample_task(id: &str) -> ComputerTask {
    ComputerTask {
        id: id.to_string(),
        target_app: "owo-sim-qq".to_string(),
        description: "模拟 QQ 受控发送".to_string(),
        allowed_actions: vec![
            "desktop_click".to_string(),
            "desktop_type".to_string(),
            "desktop_key".to_string(),
        ],
        max_duration_ms: 60_000,
        state: TaskState::Pending,
        created_at: "2026-08-14T00:00:00Z".to_string(),
        fuse_reason: None,
    }
}

fn line(text: &str, x: i32, y: i32, role: &str) -> Value {
    json!({
        "text": text,
        "x": x,
        "y": y,
        "width": 200,
        "height": 30,
        "role_hint": role,
    })
}

/// 内存模拟桌面面：聊天窗口版面 + 输入框/发送按钮/消息上屏。
struct MockTaskSurface {
    app: String,
    lines: Vec<Value>,
    input: String,
    sent: Vec<String>,
    clicks: Vec<(i32, i32)>,
    typed: Vec<String>,
    keys: Vec<String>,
    /// 输入包含该关键词时下一帧弹出敏感对话框（密码框）。
    sensitive_after_typing: Option<String>,
    /// 是否提供真实窗口身份（模拟 Agent V2 的动作前复核）。
    identity_enabled: bool,
    /// 第一次点击后模拟用户切走窗口。
    switch_after_first_click: bool,
}

impl MockTaskSurface {
    fn chat() -> Self {
        Self {
            app: "owo-sim-qq".to_string(),
            lines: vec![
                line("会话：张子豪", 10, 10, "header"),
                line("输入消息...", 10, 500, "input"),
                line("发送", 600, 500, "button"),
            ],
            input: String::new(),
            sent: Vec::new(),
            clicks: Vec::new(),
            typed: Vec::new(),
            keys: Vec::new(),
            sensitive_after_typing: None,
            identity_enabled: false,
            switch_after_first_click: false,
        }
    }

    fn sent_count(&self) -> usize {
        self.sent.len()
    }
}

#[async_trait::async_trait]
impl TaskSurface for MockTaskSurface {
    fn app(&self) -> String {
        self.app.clone()
    }

    fn window_identity(&self) -> Option<owo_agent_core::task_surface::WindowIdentity> {
        if !self.identity_enabled {
            return None;
        }
        Some(owo_agent_core::task_surface::WindowIdentity {
            app: self.app.clone(),
            pid: 42,
            hwnd: 7,
            title: "模拟窗口".to_string(),
            foreground: true,
            observed_at: "2026-11-03T00:00:00Z".to_string(),
        })
    }

    async fn ocr(&mut self) -> Result<Value, String> {
        Ok(json!({ "lines": self.lines, "surface": "mock" }))
    }

    async fn click(&mut self, x: i32, y: i32) -> Result<(), String> {
        self.clicks.push((x, y));
        if self.switch_after_first_click && self.clicks.len() == 1 {
            // 模拟用户切走窗口：应用与窗口身份都变了。
            self.app = "notepad".to_string();
        }
        // 点击输入框行 → 聚焦（无副作用即可）。
        Ok(())
    }

    async fn type_text(&mut self, text: &str) -> Result<(), String> {
        self.typed.push(text.to_string());
        self.input.push_str(text);
        if let Some(keyword) = &self.sensitive_after_typing {
            if text.contains(keyword) {
                self.lines.push(line("请输入支付密码", 300, 300, "edit"));
            }
        }
        Ok(())
    }

    async fn key(&mut self, key: &str) -> Result<(), String> {
        self.keys.push(key.to_string());
        if key == "enter" {
            let message = self.input.clone();
            self.input.clear();
            if !message.is_empty() {
                self.sent.push(message.clone());
                // 消息上屏，供 verify_text 命中。
                self.lines
                    .push(line(&format!("我：{message}"), 10, 80, "message"));
            }
        }
        Ok(())
    }

    async fn launch(&mut self, _target: &str) -> Result<(), String> {
        Ok(())
    }
}

fn closed_loop_goals(value: &str) -> Vec<TaskGoal> {
    vec![
        TaskGoal {
            anchor_text: "输入消息".to_string(),
            action: "click".to_string(),
            value: String::new(),
            verify_text: None,
        },
        TaskGoal {
            anchor_text: "输入消息".to_string(),
            action: "type".to_string(),
            value: value.to_string(),
            verify_text: None,
        },
        TaskGoal {
            anchor_text: "发送".to_string(),
            action: "click".to_string(),
            value: String::new(),
            verify_text: None,
        },
        TaskGoal {
            anchor_text: "发送".to_string(),
            action: "key".to_string(),
            value: "enter".to_string(),
            verify_text: Some(value.to_string()),
        },
    ]
}

fn approved_running(registry: &ComputerTaskRegistry, id: &str) {
    registry.approve(id).unwrap();
    registry.start(id).unwrap();
}

#[tokio::test]
async fn unapproved_action_denied_before_any_execution() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t1")).unwrap();
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t1",
        &closed_loop_goals("未批准-001"),
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.contains("需用户先批准"));
    assert_eq!(surface.sent_count(), 0);
    assert!(surface.clicks.is_empty());
    assert!(surface.typed.is_empty());
    assert!(registry.get("t1").unwrap().state == TaskState::Pending);
}

#[tokio::test]
async fn approved_task_runs_full_closed_loop_on_mock_surface() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t2")).unwrap();
    approved_running(&registry, "t2");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    let report = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t2",
        &closed_loop_goals("审批版闭环-001"),
        &mut surface,
    )
    .await
    .unwrap();
    assert_eq!(report.steps, 4);
    assert_eq!(report.state, TaskState::Completed);
    assert_eq!(surface.sent_count(), 1);
    assert_eq!(surface.sent[0], "审批版闭环-001");
    assert_eq!(registry.get("t2").unwrap().state, TaskState::Completed);
    // 动作预算计数=4。
    assert_eq!(registry.actions_taken("t2"), 4);
}

#[tokio::test]
async fn target_app_mismatch_denied_before_execution() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t3")).unwrap();
    approved_running(&registry, "t3");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();
    surface.app = "qq".to_string();

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t3",
        &closed_loop_goals("越界-001"),
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.contains("目标应用"));
    assert!(surface.clicks.is_empty());
    assert!(surface.typed.is_empty());
}

#[tokio::test]
async fn sensitive_ui_fuses_and_blocks_until_resume() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t4")).unwrap();
    approved_running(&registry, "t4");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();
    surface.sensitive_after_typing = Some("密码".to_string());

    let goals = vec![
        TaskGoal {
            anchor_text: "输入消息".to_string(),
            action: "click".to_string(),
            value: String::new(),
            verify_text: None,
        },
        TaskGoal {
            anchor_text: "输入消息".to_string(),
            action: "type".to_string(),
            value: "我的支付密码是xxx".to_string(),
            verify_text: None,
        },
        TaskGoal {
            anchor_text: "发送".to_string(),
            action: "click".to_string(),
            value: String::new(),
            verify_text: None,
        },
    ];
    let error = run_approved_task_on(&registry, &mut audit, "s", "t4", &goals, &mut surface)
        .await
        .unwrap_err();
    assert!(error.contains("敏感熔断"));
    assert_eq!(registry.get("t4").unwrap().state, TaskState::Fused);
    assert!(registry.get("t4").unwrap().fuse_reason.is_some());
    // 熔断后：下一步动作未执行（第 3 步 click 未发生；只有前两步动作）。
    assert_eq!(surface.clicks.len(), 1);
    assert_eq!(surface.typed.len(), 1);

    // 熔断后所有动作被拒，直到人工接管 resume。
    let denied = task_gate_check(
        &registry,
        Some(&mut audit),
        "s",
        "t4",
        "desktop_click",
        "owo-sim-qq",
        None,
    );
    assert!(denied.is_err());
    assert!(denied.unwrap_err().contains("不可执行"));

    // 人工接管（resume）后可继续执行。
    assert_eq!(registry.resume("t4").unwrap(), TaskState::Approved);
    assert!(registry.check_can_execute("t4").is_ok());
    // 敏感界面已消失后，重新跑闭环可以完成。
    surface
        .lines
        .retain(|l| l.get("text").and_then(Value::as_str) != Some("请输入支付密码"));
    surface.sensitive_after_typing = None;
    let report = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t4",
        &[
            TaskGoal {
                anchor_text: "输入消息".to_string(),
                action: "type".to_string(),
                value: "恢复后继续-002".to_string(),
                verify_text: None,
            },
            TaskGoal {
                anchor_text: "发送".to_string(),
                action: "key".to_string(),
                value: "enter".to_string(),
                verify_text: Some("恢复后继续-002".to_string()),
            },
        ],
        &mut surface,
    )
    .await
    .unwrap();
    assert_eq!(report.state, TaskState::Completed);
    assert!(surface.sent.iter().any(|s| s.contains("恢复后继续-002")));
}

#[tokio::test]
async fn timeout_pauses_loop_and_denies() {
    let registry = ComputerTaskRegistry::new();
    let mut task = sample_task("t5");
    task.max_duration_ms = 10;
    registry.create(task).unwrap();
    approved_running(&registry, "t5");
    std::thread::sleep(Duration::from_millis(20));
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t5",
        &closed_loop_goals("超时-001"),
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.contains("超时"));
    assert_eq!(registry.get("t5").unwrap().state, TaskState::Paused);
    assert!(surface.clicks.is_empty());
}

#[tokio::test]
async fn action_budget_exhausted_pauses_and_denies() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t6")).unwrap();
    registry.set_action_budget("t6", 2).unwrap();
    approved_running(&registry, "t6");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    // 预算 2：前两步（click/type）放行，第三步 click 应被拒并自动暂停。
    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t6",
        &closed_loop_goals("预算-001"),
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.contains("预算"));
    assert_eq!(registry.get("t6").unwrap().state, TaskState::Paused);
    assert_eq!(surface.clicks.len(), 1); // 只有第一步 click 执行
    assert_eq!(surface.typed.len(), 1);
    assert_eq!(surface.keys.len(), 0);
    assert_eq!(registry.actions_taken("t6"), 2);
}

#[tokio::test]
async fn audit_records_denied_and_allowed_entries() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t7")).unwrap();
    let mut audit = AuditLog::default();

    // 未批准直接调用门禁 → 拒绝条目（approved=false）。
    let err = task_gate_check(
        &registry,
        Some(&mut audit),
        "s",
        "t7",
        "desktop_click",
        "owo-sim-qq",
        None,
    )
    .unwrap_err();
    assert!(err.contains("Pending"));

    approved_running(&registry, "t7");
    task_gate_check(
        &registry,
        Some(&mut audit),
        "s",
        "t7",
        "desktop_click",
        "owo-sim-qq",
        None,
    )
    .unwrap();
    registry.record_action("t7");

    let entries = audit.entries.clone();
    let denied = entries.iter().find(|e| e.approved == Some(false));
    assert!(denied.is_some(), "应有拒绝审计条目");
    assert!(denied.unwrap().detail.contains("门禁拒绝"));
    let allowed = entries.iter().find(|e| e.approved == Some(true));
    assert!(allowed.is_some(), "应有放行审计条目");
}

#[tokio::test]
async fn closed_loop_audits_start_step_and_complete() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("t8")).unwrap();
    approved_running(&registry, "t8");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    let _ = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "t8",
        &closed_loop_goals("审计-001"),
        &mut surface,
    )
    .await
    .unwrap();

    let events: Vec<String> = audit.entries.iter().map(|e| e.event.clone()).collect();
    assert!(events.iter().any(|e| e == "computer_task"));
    let tools: Vec<Option<String>> = audit.entries.iter().map(|e| e.tool.clone()).collect();
    assert!(tools.iter().any(|t| t.as_deref() == Some("start")));
    assert!(tools.iter().any(|t| t.as_deref() == Some("step")));
    assert!(tools.iter().any(|t| t.as_deref() == Some("complete")));
    // 门禁放行条目（permission/approved=true）。
    assert!(audit
        .entries
        .iter()
        .any(|e| e.event == "permission" && e.approved == Some(true)));
}

#[test]
fn gate_and_sensitive_pure_function_semantics() {
    // sensitive_ui_hit 关键词覆盖（密码/支付/验证码）。
    assert!(sensitive_ui_hit("PasswordBox", "Edit", "").is_some());
    assert!(sensitive_ui_hit("", "", "请输入验证码").is_some());
    assert!(sensitive_ui_hit("输入消息", "Edit", "").is_none());
    // scan_ui_sensitive 对整屏 lines 扫描。
    let clean =
        json!({ "lines": [line("输入消息...", 0, 0, "input"), line("发送", 0, 0, "button")] });
    assert!(scan_ui_sensitive(&clean).is_none());
    let dirty = json!({ "lines": [line("请输入支付密码", 0, 0, "edit")] });
    assert!(scan_ui_sensitive(&dirty).is_some());
}

#[tokio::test]
async fn empty_goals_are_rejected_without_touching_the_desktop() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("e1")).unwrap();
    approved_running(&registry, "e1");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    let error = run_approved_task_on(&registry, &mut audit, "s", "e1", &[], &mut surface)
        .await
        .unwrap_err();
    assert!(error.starts_with("EMPTY_GOALS"), "{error}");
    assert!(surface.clicks.is_empty());
    assert!(surface.typed.is_empty());
    // 空目标不能把任务带到终态。
    assert_eq!(registry.get("e1").unwrap().state, TaskState::Running);
}

#[tokio::test]
async fn invalid_goal_payload_is_rejected() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("e2")).unwrap();
    approved_running(&registry, "e2");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();
    let goals = vec![TaskGoal {
        anchor_text: "发送".to_string(),
        action: "type".to_string(),
        value: String::new(),
        verify_text: None,
    }];

    let error = run_approved_task_on(&registry, &mut audit, "s", "e2", &goals, &mut surface)
        .await
        .unwrap_err();
    assert!(error.contains("INVALID_ACTION"), "{error}");
    assert!(surface.typed.is_empty());
}

#[tokio::test]
async fn ambiguous_anchor_is_rejected_instead_of_picking_the_first() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("e3")).unwrap();
    approved_running(&registry, "e3");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();
    surface.lines.push(line("发送给张子豪", 400, 300, "button"));
    let goals = vec![TaskGoal {
        anchor_text: "发送".to_string(),
        action: "click".to_string(),
        value: String::new(),
        verify_text: None,
    }];

    let error = run_approved_task_on(&registry, &mut audit, "s", "e3", &goals, &mut surface)
        .await
        .unwrap_err();
    assert!(error.starts_with("ANCHOR_AMBIGUOUS"), "{error}");
    assert!(surface.clicks.is_empty());
}

#[tokio::test]
async fn foreground_window_switch_pauses_before_the_next_action() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("e4")).unwrap();
    approved_running(&registry, "e4");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();
    surface.identity_enabled = true;
    surface.switch_after_first_click = true;

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "e4",
        &closed_loop_goals("切换窗口-001"),
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("STALE_WINDOW_IDENTITY"), "{error}");
    // 只执行了第一次点击，之后的输入没有发到新窗口。
    assert_eq!(surface.clicks.len(), 1);
    assert!(surface.typed.is_empty());
    assert_eq!(registry.get("e4").unwrap().state, TaskState::Paused);
}

#[tokio::test]
async fn every_action_produces_a_receipt() {
    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("e5")).unwrap();
    approved_running(&registry, "e5");
    let mut audit = AuditLog::default();
    let mut surface = MockTaskSurface::chat();

    let report = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "e5",
        &closed_loop_goals("回执-001"),
        &mut surface,
    )
    .await
    .unwrap();
    assert_eq!(report.receipts.len(), 4);
    assert!(report
        .receipts
        .iter()
        .all(|receipt| receipt.observed_window.is_some() || !surface.identity_enabled));
    assert!(matches!(
        report.receipts.last().unwrap().verification,
        owo_agent_core::ActionVerification::Verified
    ));
}

/// OCR 为空但窗口绑定 UIA 控件有效时，`set_value` 可以使用 UIA 观察，
/// 且动作执行回执仍带前后观察和窗口身份。
struct UiaValueSurface {
    identity: owo_agent_core::task_surface::WindowIdentity,
    name: String,
    hwnd: i64,
    value: String,
    readback_override: Option<String>,
    duplicate_uia_control: bool,
    missing_uia_control: bool,
    write_count: usize,
    observed_hwnds: Vec<i64>,
    set_names: Vec<String>,
}

impl UiaValueSurface {
    fn new(name: &str, hwnd: i64, identity_hwnd: i64) -> Self {
        Self {
            identity: owo_agent_core::task_surface::WindowIdentity {
                app: "owo-sim-qq".to_string(),
                pid: 42,
                hwnd: identity_hwnd,
                title: "打开".to_string(),
                foreground: true,
                observed_at: "2026-11-07T00:00:00Z".to_string(),
            },
            name: name.to_string(),
            hwnd,
            value: String::new(),
            readback_override: None,
            duplicate_uia_control: false,
            missing_uia_control: false,
            write_count: 0,
            observed_hwnds: Vec::new(),
            set_names: Vec::new(),
        }
    }

    fn observation(&self) -> Value {
        let control = json!({
            "text": self.name,
            "x": 10,
            "y": 20,
            "width": 120,
            "height": 24,
            "role_hint": "edit",
            "control_type": 50004,
            "has_value_pattern": true,
            "has_keyboard_focus": true,
            "automation_id": "File name",
            "process_id": 42,
            "native_window_handle": 0,
            "hwnd": self.hwnd,
        });
        let lines = if self.duplicate_uia_control {
            vec![control.clone(), control]
        } else if self.missing_uia_control {
            Vec::new()
        } else {
            vec![control]
        };
        json!({
            "surface": "uia",
            "text": self.name,
            "observation_ref": "sha256:bound-uia-observation",
            "lines": lines,
        })
    }
}

#[async_trait::async_trait]
impl TaskSurface for UiaValueSurface {
    fn app(&self) -> String {
        self.identity.app.clone()
    }

    fn window_identity(&self) -> Option<owo_agent_core::task_surface::WindowIdentity> {
        Some(self.identity.clone())
    }

    async fn ocr(&mut self) -> Result<Value, String> {
        Err("识别结果为空".to_string())
    }

    async fn accessible_observation(
        &mut self,
        hwnd: i64,
        name: &str,
    ) -> Result<Option<Value>, String> {
        self.observed_hwnds.push(hwnd);
        Ok((hwnd == self.identity.hwnd && name == self.name).then(|| self.observation()))
    }

    async fn accessible_anchors(&mut self, _hwnd: i64, _name: &str) -> Result<Vec<Value>, String> {
        Ok(Vec::new())
    }

    async fn click(&mut self, _x: i32, _y: i32) -> Result<(), String> {
        Ok(())
    }

    async fn type_text(&mut self, _text: &str) -> Result<(), String> {
        Ok(())
    }

    async fn set_value(
        &mut self,
        identity: &owo_agent_core::task_surface::UiaEditIdentity,
        value: &str,
    ) -> Result<String, String> {
        self.set_names.push(identity.name.clone());
        if identity.hwnd != self.identity.hwnd
            || identity.pid != self.identity.pid
            || identity.name != self.name
            || identity.automation_id != "File name"
        {
            return Err("错误窗口或控件，拒绝写值".to_string());
        }
        self.value = value.to_string();
        self.write_count += 1;
        Ok(self
            .readback_override
            .clone()
            .unwrap_or_else(|| self.value.clone()))
    }

    async fn key(&mut self, _key: &str) -> Result<(), String> {
        Ok(())
    }

    async fn launch(&mut self, _target: &str) -> Result<(), String> {
        Ok(())
    }
}

fn uia_set_value_goal(name: &str) -> Vec<TaskGoal> {
    vec![TaskGoal {
        anchor_text: format!("uia:{name}"),
        action: "set_value".to_string(),
        value: "D:/OwO/acceptance/notice.txt".to_string(),
        verify_text: None,
    }]
}

fn approved_uia_task(registry: &ComputerTaskRegistry, id: &str) {
    let mut task = sample_task(id);
    task.allowed_actions = vec!["desktop_type".to_string()];
    registry.create(task).unwrap();
    approved_running(registry, id);
}

#[tokio::test]
async fn ordinary_actions_still_require_ocr() {
    let registry = ComputerTaskRegistry::new();
    approved_uia_task(&registry, "uia-ocr-required");
    let mut audit = AuditLog::default();
    let mut surface = UiaValueSurface::new("文件名", 7, 7);
    let goals = vec![TaskGoal {
        anchor_text: "文件名".to_string(),
        action: "type".to_string(),
        value: "text".to_string(),
        verify_text: None,
    }];

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "uia-ocr-required",
        &goals,
        &mut surface,
    )
    .await
    .unwrap_err();
    assert_eq!(error, "识别结果为空");
    assert_eq!(surface.write_count, 0);
}

#[tokio::test]
async fn empty_ocr_with_valid_bound_uia_control_sets_value_and_records_receipt() {
    let registry = ComputerTaskRegistry::new();
    approved_uia_task(&registry, "uia-ok");
    let mut audit = AuditLog::default();
    let mut surface = UiaValueSurface::new("文件名", 7, 7);
    let goals = uia_set_value_goal("文件名");

    let report = run_approved_task_on(&registry, &mut audit, "s", "uia-ok", &goals, &mut surface)
        .await
        .unwrap();
    assert_eq!(report.steps, 1);
    assert_eq!(surface.write_count, 1);
    assert_eq!(surface.set_names, vec!["文件名".to_string()]);
    assert_eq!(surface.observed_hwnds, vec![7]);
    assert_eq!(surface.value, goals[0].value);
    assert_eq!(report.receipts.len(), 1);
    let receipt = &report.receipts[0];
    assert!(matches!(
        receipt.outcome,
        owo_agent_core::ActionOutcome::Executed
    ));
    assert_eq!(
        receipt.before_observation_ref.as_deref(),
        Some("sha256:bound-uia-observation")
    );
    assert_eq!(receipt.observed_window.as_ref().unwrap().hwnd, 7);
    assert_eq!(
        receipt.value_readback.as_deref(),
        Some(goals[0].value.as_str())
    );
    assert_eq!(
        receipt.after_observation_ref.as_deref(),
        Some("sha256:bound-uia-observation")
    );
    assert!(matches!(
        receipt.verification,
        owo_agent_core::ActionVerification::Verified
    ));
}

#[tokio::test]
async fn uia_set_value_refuses_a_wrong_window_before_any_write() {
    let registry = ComputerTaskRegistry::new();
    approved_uia_task(&registry, "uia-wrong-window");
    let mut audit = AuditLog::default();
    let mut surface = UiaValueSurface::new("文件名", 9, 7);
    // The approved window is HWND 7; a control from HWND 9 must never be used.
    let goals = uia_set_value_goal("文件名");

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "uia-wrong-window",
        &goals,
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("TARGET_NOT_FOUND"), "{error}");
    assert_eq!(surface.write_count, 0);
    assert_eq!(
        registry.get("uia-wrong-window").unwrap().state,
        TaskState::Paused
    );
}

#[tokio::test]
async fn duplicate_uia_candidates_refuse_set_value() {
    let registry = ComputerTaskRegistry::new();
    approved_uia_task(&registry, "uia-ambiguous");
    let mut audit = AuditLog::default();
    let mut surface = UiaValueSurface::new("文件名", 7, 7);
    surface.duplicate_uia_control = true;
    let goals = uia_set_value_goal("文件名");

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "uia-ambiguous",
        &goals,
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("ANCHOR_AMBIGUOUS"), "{error}");
    assert_eq!(surface.write_count, 0);
}

#[tokio::test]
async fn uia_observation_requires_bound_identity() {
    let empty_ocr = json!({"lines": []});
    let missing_identity = json!({"lines": [{
        "control_type": 50004,
        "has_value_pattern": true,
        "hwnd": 7,
    }]});
    assert!(!owo_agent_core::uia_observation_is_safe(
        &empty_ocr,
        &missing_identity
    ));
    let bound = json!({"lines": [{
        "control_type": 50004,
        "has_value_pattern": true,
        "hwnd": 7,
        "automation_id": "File name",
        "process_id": 42,
    }]});
    assert!(owo_agent_core::uia_observation_is_safe(&empty_ocr, &bound));
}

#[tokio::test]
async fn empty_uia_observation_refuses_set_value() {
    let registry = ComputerTaskRegistry::new();
    approved_uia_task(&registry, "uia-empty-observation");
    let mut audit = AuditLog::default();
    let mut surface = UiaValueSurface::new("文件名", 7, 7);
    surface.missing_uia_control = true;
    let goals = uia_set_value_goal("文件名");

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "uia-empty-observation",
        &goals,
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("TARGET_NOT_FOUND"), "{error}");
    assert_eq!(surface.write_count, 0);
}

#[tokio::test]
async fn uia_set_value_readback_mismatch_is_refused() {
    let registry = ComputerTaskRegistry::new();
    approved_uia_task(&registry, "uia-readback-mismatch");
    let mut audit = AuditLog::default();
    let mut surface = UiaValueSurface::new("文件名", 7, 7);
    surface.readback_override = Some("unexpected".to_string());
    let goals = uia_set_value_goal("文件名");

    let error = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "uia-readback-mismatch",
        &goals,
        &mut surface,
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("POST_ACTION_VERIFY_FAILED"), "{error}");
    assert_eq!(
        registry.get("uia-readback-mismatch").unwrap().state,
        TaskState::Paused
    );
}

#[tokio::test]
async fn password_edit_is_not_a_valid_uia_observation() {
    let node = owo_agent_core::UiNode {
        name: "密码".to_string(),
        control_type: 50004,
        class: "Edit".to_string(),
        depth: 2,
        x: 10,
        y: 20,
        width: 120,
        height: 24,
        value_safety: Some(owo_agent_core::UiValueSafety {
            is_password: true,
            is_enabled: true,
            is_read_only: false,
            is_offscreen: false,
            has_keyboard_focus: true,
            has_value_pattern: true,
            automation_id: "Password".into(),
            process_id: 42,
            native_window_handle: 0,
        }),
    };
    assert!(!owo_agent_core::safe_value_target(&node, "密码", false));
    assert!(owo_agent_core::scan_ui_sensitive(&json!({
        "lines": [{"text": "密码", "role_hint": "edit"}]
    }))
    .is_some());
}

#[tokio::test]
async fn sim_surface_requires_env_or_returns_clear_error() {
    // 无 OWO_SIM_QQ_URL 时 SimTaskSurface::new 返回明确错误（含配置提示）。
    if std::env::var("OWO_SIM_QQ_URL").is_err() {
        let error = owo_agent_core::computer_use::SimTaskSurface::new().unwrap_err();
        assert!(error.contains("OWO_SIM_QQ_URL"));
    }
}

#[tokio::test]
async fn run_approved_task_on_live_sim_when_configured() {
    // 可选：OWO_SIM_QQ_URL 指向 owo-sim-qq 时跑真实模拟面闭环（与 e2e 脚本同路径）。
    let Ok(base) = std::env::var("OWO_SIM_QQ_URL") else {
        eprintln!("[skip] 未配置 OWO_SIM_QQ_URL，跳过 live-sim 闭环测试");
        return;
    };
    let client = reqwest::Client::new();
    let _ = client
        .post(format!("{}/reset", base.trim_end_matches('/')))
        .json(&json!({}))
        .send()
        .await
        .expect("模拟服务不可达");

    let registry = ComputerTaskRegistry::new();
    registry.create(sample_task("live1")).unwrap();
    registry.approve("live1").unwrap();
    registry.start("live1").unwrap();
    let mut audit = AuditLog::default();
    let mut surface = owo_agent_core::computer_use::SimTaskSurface::new().unwrap();

    let report = run_approved_task_on(
        &registry,
        &mut audit,
        "s",
        "live1",
        &closed_loop_goals("模拟面闭环-001"),
        &mut surface,
    )
    .await
    .unwrap();
    assert_eq!(report.state, TaskState::Completed);
    assert_eq!(report.steps, 4);
    assert_eq!(registry.get("live1").unwrap().state, TaskState::Completed);
    eprintln!("[live-sim] 闭环完成，4 步全过");
}

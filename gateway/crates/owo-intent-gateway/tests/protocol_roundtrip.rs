//! 协议 v3 契约测试（E1.1 验收）。
//!
//! 基准数据来自 `OwO-release/docs/plugins/agent-ipc-integration.md` 的官方示例
//! （请求/响应完整 JSON），以及协议最小接入验证中的关键反例。

use owo_intent_gateway::protocol::*;

/// integration.md「请求结构」节的官方示例。
const OFFICIAL_REQUEST: &str = r#"{
  "schema_version": 3,
  "action": "submit",
  "session_id": "0123456789abcdef0123456789abcdef",
  "request_id": "req-120",
  "parent_request_id": "",
  "idempotency_key": "req-120",
  "capabilities": ["protocol.negotiation", "input.structured", "context.entries", "commands.risk", "tasks.slots", "tasks.incremental"],
  "protocol_min": 2,
  "protocol_max": 3,
  "required_features": ["protocol.negotiation", "input.structured", "commands.risk", "tasks.slots"],
  "user_input": "bangwozhaowenjian",
  "input": {
    "raw_pinyin": "bangwozhaowenjian",
    "segmented_pinyin": "bang'wo'zhao'wen'jian",
    "selected_text": "帮我找文件",
    "pending_pinyin": "",
    "natural_language": "帮我找文件",
    "input_mode": "agent",
    "correction_enabled": true
  },
  "application": {
    "process_id": 4242,
    "thread_id": 8120,
    "executable": "notepad.exe",
    "window_class": "Notepad",
    "focus_window_class": "RichEditD2DPT",
    "context_id": "ctx-current-01234567",
    "sensitive_input": false
  },
  "session_context": "当前输入位置的近期上下文",
  "context_entries": [
    {
      "context_id": "ctx-editor-89abcdef",
      "application": {
        "process_id": 3000,
        "thread_id": 3001,
        "executable": "editor.exe",
        "window_class": "EditorWindow",
        "focus_window_class": "TextBox",
        "context_id": "ctx-editor-89abcdef",
        "sensitive_input": false
      },
      "text": "上一输入位置由 OwO 上屏的有限片段",
      "sequence": 17,
      "context_type": "committed_text",
      "source": "owo_commit",
      "created_at_ms": 1787830000000,
      "relevance_milli": 800,
      "is_current": false,
      "privacy": {"filtered": true, "redacted": false, "truncated": false}
    }
  ],
  "command_id": "",
  "page": 0,
  "task_revision": 0,
  "slot_updates": []
}"#;

/// integration.md「响应结构」节的官方示例。
const OFFICIAL_RESPONSE: &str = r#"{
  "schema_version": 3,
  "session_id": "0123456789abcdef0123456789abcdef",
  "request_id": "req-120",
  "message": "我找到了几个可选操作",
  "commands": [
    {
      "id": "show-results", "label": "显示搜索结果", "high_risk": false,
      "description": "显示已经找到的文件", "category": "file.search",
      "risk_level": "low", "requires_confirmation": false,
      "preview": "不会修改文件", "enabled": true, "disabled_reason": "",
      "task_revision": 0, "slot_updates": [], "commit_task": false
    },
    {
      "id": "delete-files", "label": "删除这些文件", "high_risk": true,
      "description": "删除选定文件", "category": "file.delete",
      "risk_level": "high", "requires_confirmation": true,
      "preview": "必须在 Agent 可信界面确认", "enabled": true, "disabled_reason": "",
      "task_revision": 0, "slot_updates": [], "commit_task": false
    }
  ],
  "executing_command": "",
  "status": "agent_mode",
  "page": 0,
  "has_more": true,
  "error_code": "",
  "state_revision": 4,
  "progress": 65,
  "retry_after_ms": 150,
  "can_cancel": true,
  "can_continue_input": true,
  "expires_at_ms": 1787830060000,
  "error_message": "",
  "retryable": false,
  "capabilities": ["protocol.negotiation", "input.structured", "context.entries", "commands.risk", "tasks.slots", "tasks.incremental"],
  "task": {"intent": "", "intent_ranges": [], "slots": [], "unconsumed_ranges": [], "revision": 0}
}"#;

#[test]
fn official_request_decodes() {
    let request =
        AgentIpcRequest::decode(OFFICIAL_REQUEST.as_bytes()).expect("官方请求示例必须可解码");
    assert_eq!(request.schema_version, 3);
    assert_eq!(request.action, Action::Submit);
    assert_eq!(request.session_id, "0123456789abcdef0123456789abcdef");
    assert_eq!(request.request_id, "req-120");
    assert_eq!(request.user_input, "bangwozhaowenjian");
    assert_eq!(request.input.segmented_pinyin, "bang'wo'zhao'wen'jian");
    assert_eq!(request.input.input_mode, "agent");
    assert_eq!(request.application.executable, "notepad.exe");
    assert_eq!(request.context_entries.len(), 1);
    assert_eq!(request.context_entries[0].sequence, 17);
    assert!(request.context_entries[0].privacy.filtered);
    assert_eq!(request.page, 0);
    assert!(request.slot_updates.is_empty());
}

#[test]
fn official_request_roundtrip() {
    let request = AgentIpcRequest::decode(OFFICIAL_REQUEST.as_bytes()).unwrap();
    let encoded = request.encode().expect("编码必须通过校验");
    let decoded = AgentIpcRequest::decode(&encoded).unwrap();
    assert_eq!(request, decoded, "往返后必须完全相等");
}

#[test]
fn official_response_decodes() {
    let response =
        AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).expect("官方响应示例必须可解码");
    assert_eq!(response.status, Status::AgentMode);
    assert_eq!(response.commands.len(), 2);
    assert_eq!(response.commands[0].risk_level, RiskLevel::Low);
    assert_eq!(response.commands[1].risk_level, RiskLevel::High);
    assert!(response.commands[1].high_risk);
    assert!(response.commands[1].requires_confirmation);
    assert_eq!(response.progress, 65);
    assert_eq!(response.retry_after_ms, 150);
    assert!(response.commands[0].slot_updates.is_empty());
}

#[test]
fn official_response_roundtrip() {
    let response = AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap();
    let encoded = response.encode().expect("编码必须通过校验");
    let decoded = AgentIpcResponse::decode(&encoded).unwrap();
    assert_eq!(response, decoded, "往返后必须完全相等");
}

#[test]
fn unknown_field_rejected() {
    let json = OFFICIAL_REQUEST.replace(r#""page": 0,"#, r#""page": 0, "unknown_field": 1,"#);
    let error = AgentIpcRequest::decode(json.as_bytes()).expect_err("未知字段必须拒绝");
    assert!(
        matches!(error, ProtocolError::Json(_)),
        "应为 JSON 解码错误"
    );
}

#[test]
fn missing_field_rejected() {
    let json = OFFICIAL_REQUEST.replace(r#""page": 0,"#, "");
    let error = AgentIpcRequest::decode(json.as_bytes()).expect_err("缺字段必须拒绝");
    assert!(matches!(error, ProtocolError::Json(_)));
}

#[test]
fn schema_version_must_be_3() {
    let json = OFFICIAL_REQUEST.replace(r#""schema_version": 3"#, r#""schema_version": 2"#);
    let error = AgentIpcRequest::decode(json.as_bytes()).expect_err("v2 结构应被拒绝");
    assert!(
        matches!(&error, ProtocolError::InvalidField { field, .. } if *field == "schema_version"),
        "应报 schema_version 非法：{error}"
    );
}

#[test]
fn session_id_too_short_rejected() {
    let json = OFFICIAL_REQUEST.replace("0123456789abcdef0123456789abcdef", "short");
    let error = AgentIpcRequest::decode(json.as_bytes()).expect_err("短 session_id 必须拒绝");
    assert!(matches!(&error, ProtocolError::InvalidField { field, .. } if *field == "session_id"));
}

#[test]
fn request_id_character_set_enforced() {
    let json = OFFICIAL_REQUEST.replace(r#""request_id": "req-120""#, r#""request_id": "req 120""#);
    let error = AgentIpcRequest::decode(json.as_bytes()).expect_err("非法字符必须拒绝");
    assert!(matches!(&error, ProtocolError::InvalidField { field, .. } if *field == "request_id"));
}

#[test]
fn high_risk_invariant_enforced() {
    // 协议最小接入验证第 15 条：risk_level=high 但未同时设置两个布尔位 → 拒绝。
    let json = OFFICIAL_RESPONSE.replace(
        r#""risk_level": "high", "requires_confirmation": true,"#,
        r#""risk_level": "high", "requires_confirmation": false,"#,
    );
    let error = AgentIpcResponse::decode(json.as_bytes()).expect_err("风险字段不变量必须生效");
    assert!(
        matches!(&error, ProtocolError::InvalidField { field, .. } if *field == "command.risk_level")
    );
}

#[test]
fn duplicate_command_ids_rejected() {
    let response = AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap();
    let mut broken = response.clone();
    broken.commands[1].id = broken.commands[0].id.clone();
    let error = broken.encode().expect_err("重复命令 ID 必须拒绝");
    assert!(matches!(&error, ProtocolError::InvalidField { field, .. } if *field == "commands.id"));
}

#[test]
fn pinyin_range_bounds_enforced() {
    let mut request = AgentIpcRequest::decode(OFFICIAL_REQUEST.as_bytes()).unwrap();
    request.slot_updates = vec![SlotUpdate {
        slot_id: "time".to_string(),
        value: "明天3点".to_string(),
        consumed_ranges: vec![PinyinRange { start: 10, end: 10 }],
        lock: true,
    }];
    let error = request.encode().expect_err("start >= end 的区间必须拒绝");
    assert!(matches!(
        &error,
        ProtocolError::InvalidField { field, .. } if *field == "slot_updates.consumed_ranges"
    ));
}

#[test]
fn payload_limit_enforced() {
    let oversized = vec![b'{'; MAXIMUM_AGENT_PAYLOAD_BYTES + 1];
    let error = AgentIpcRequest::decode(&oversized).expect_err("超限载荷必须拒绝");
    assert!(matches!(error, ProtocolError::PayloadTooLarge { .. }));
}

#[test]
fn negotiated_capabilities_intersection() {
    let request = AgentIpcRequest::decode(OFFICIAL_REQUEST.as_bytes()).unwrap();
    let negotiated = request.negotiated_capabilities();
    // 请求声明 6 项，全部在支持列表内 → 交集为 6 项且保序（按支持列表顺序）。
    assert_eq!(negotiated.len(), 6);
    assert!(negotiated.contains(&"tasks.slots".to_string()));

    let mut minimal = request.clone();
    minimal.capabilities = vec!["context.privacy".to_string(), "unknown.feature".to_string()];
    let negotiated = minimal.negotiated_capabilities();
    assert_eq!(negotiated, vec!["context.privacy".to_string()]);
}

#[test]
fn slot_patch_request_decodes() {
    // integration.md 槽位补丁示例（select 只携带 command_id/task_revision/slot_updates）。
    let json = OFFICIAL_REQUEST
        .replace(r#""action": "submit""#, r#""action": "select""#)
        .replace(r#""command_id": """#, r#""command_id": "time-tomorrow-3""#)
        .replace(r#""task_revision": 0,"#, r#""task_revision": 1,"#)
        .replace(
            r#""slot_updates": []"#,
            r#""slot_updates": [{"slot_id": "time", "value": "明天3点", "consumed_ranges": [{"start": 0, "end": 8}], "lock": true}]"#,
        );
    let request = AgentIpcRequest::decode(json.as_bytes()).expect("槽位补丁请求必须可解码");
    assert_eq!(request.action, Action::Select);
    assert_eq!(request.command_id, "time-tomorrow-3");
    assert_eq!(request.task_revision, 1);
    assert_eq!(request.slot_updates.len(), 1);
    assert_eq!(request.slot_updates[0].consumed_ranges[0].end, 8);
}

#[test]
fn cancel_action_decodes() {
    let json = OFFICIAL_REQUEST
        .replace(r#""action": "submit""#, r#""action": "cancel""#)
        .replace(
            r#""user_input": "bangwozhaowenjian","#,
            r#""user_input": "","#,
        );
    let request = AgentIpcRequest::decode(json.as_bytes()).expect("cancel 请求必须可解码");
    assert_eq!(request.action, Action::Cancel);
}

#[test]
fn error_response_with_code_decodes() {
    let response = AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap();
    let mut error = response.clone();
    error.status = Status::Error;
    error.error_code = "session_not_found".to_string();
    error.error_message = "找不到会话".to_string();
    error.commands.clear();
    error.retryable = false;
    let encoded = error.encode().unwrap();
    let decoded = AgentIpcResponse::decode(&encoded).unwrap();
    assert_eq!(decoded.status, Status::Error);
    assert_eq!(decoded.error_code, "session_not_found");
}

// ─────────────── v4 command.effect（output.effects）───────────────

fn with_effects_capability(mut response: AgentIpcResponse) -> AgentIpcResponse {
    response
        .capabilities
        .push(CAPABILITY_OUTPUT_EFFECTS.to_string());
    response
}

fn commit_effect() -> CommandEffect {
    CommandEffect {
        kind: EffectKind::CommitText,
        text: "Hello".to_string(),
        target_context_id: "ctx-current-01234567".to_string(),
        selection_sha256: String::new(),
    }
}

#[test]
fn command_without_effect_keeps_v3_shape() {
    let response = AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap();
    assert!(response.commands.iter().all(|c| c.effect.is_none()));
    let encoded = String::from_utf8(response.encode().unwrap()).unwrap();
    assert!(
        !encoded.contains("\"effect\""),
        "无 effect 时不得输出该字段"
    );
}

#[test]
fn effect_roundtrip_when_negotiated() {
    let mut response =
        with_effects_capability(AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap());
    response.commands[0].effect = Some(commit_effect());
    let bytes = response.encode().expect("已协商时允许 effect");
    let decoded = AgentIpcResponse::decode(&bytes).unwrap();
    assert_eq!(decoded, response);
}

#[test]
fn effect_requires_negotiated_capability() {
    let mut response = AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap();
    response.commands[0].effect = Some(commit_effect());
    let error = response.encode().expect_err("未协商不得携带 effect");
    assert!(
        matches!(&error, ProtocolError::InvalidField { field, .. } if *field == "commands.effect")
    );
}

#[test]
fn replace_selection_requires_binding() {
    let mut response =
        with_effects_capability(AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap());
    let mut effect = commit_effect();
    effect.kind = EffectKind::ReplaceSelection;
    response.commands[0].effect = Some(effect.clone());
    assert!(response.encode().is_err(), "缺选区摘要必须拒绝");

    effect.selection_sha256 = "A".repeat(64);
    response.commands[0].effect = Some(effect.clone());
    assert!(response.encode().is_err(), "摘要必须是小写十六进制");

    effect.selection_sha256 = "a".repeat(64);
    effect.target_context_id.clear();
    response.commands[0].effect = Some(effect.clone());
    assert!(response.encode().is_err(), "必须绑定上下文");

    effect.target_context_id = "ctx-current-01234567".to_string();
    response.commands[0].effect = Some(effect);
    response.encode().expect("完整绑定应通过");
}

#[test]
fn effect_text_and_digest_bounds() {
    let mut response =
        with_effects_capability(AgentIpcResponse::decode(OFFICIAL_RESPONSE.as_bytes()).unwrap());
    let mut effect = commit_effect();
    effect.text.clear();
    response.commands[0].effect = Some(effect.clone());
    assert!(response.encode().is_err(), "空文本必须拒绝");
    effect.text = "x".to_string();
    effect.selection_sha256 = "a".repeat(64);
    response.commands[0].effect = Some(effect);
    assert!(response.encode().is_err(), "commit_text 不得携带摘要");
}

#[test]
fn effect_unknown_field_rejected() {
    let json = OFFICIAL_RESPONSE
        .replace(
            r#""capabilities": ["protocol.negotiation","#,
            r#""capabilities": ["output.effects", "protocol.negotiation","#,
        )
        .replacen(
            r#""commit_task": false"#,
            r#""commit_task": false, "effect": {"kind": "copy_text", "text": "x", "target_context_id": "", "selection_sha256": "", "extra": 1}"#,
            1,
        );
    assert!(AgentIpcResponse::decode(json.as_bytes()).is_err());
}

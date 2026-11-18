//! Trusted tool capability issuance, execution, and receipt auditing.

use super::{resolve_session_path, AuditLog, ToolContext, ToolRegistry, ToolSpec};
use crate::permissions::{Decision, PermissionRequest};
use crate::tool_effects::EffectClass;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

/// ToolHost 签发 capability 时必须绑定的运行上下文。
///
/// 该类型只在 core 内部构造；调用方不能只凭工具名/参数伪造一个脱离
/// 当前会话、工作区和回合的执行能力。
#[derive(Debug, Clone)]
pub(crate) struct ToolCapabilityContext {
    pub workspace: String,
    pub session_id: String,
    pub turn_id: String,
    pub scope: String,
    pub max_command_timeout_ms: Option<u64>,
}

/// Policy 放行后的类型化凭证。
///
/// `ToolHostService` 不接受裸 `bool` 作为批准证明；凭证只能由
/// `PermissionRequest + Decision::Allow` 构造，并绑定工具名和参数摘要。
#[derive(Debug, Clone)]
pub(crate) struct ToolApprovalGrant {
    tool: String,
    args_sha256: String,
    request_id: String,
}

impl ToolApprovalGrant {
    pub(crate) fn from_decision(
        request: &PermissionRequest,
        decision: Decision,
    ) -> Result<Self, String> {
        if decision != Decision::Allow {
            return Err(format!("permission not granted: {}", request.tool));
        }
        Ok(Self {
            tool: request.tool.clone(),
            args_sha256: crate::CasStore::hash_of(request.args.to_string().as_bytes()),
            request_id: request.request_id.clone(),
        })
    }
}

impl ToolCapabilityContext {
    pub(crate) fn for_workspace(
        workspace: &Path,
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
    ) -> Self {
        Self {
            workspace: workspace.to_string_lossy().to_string(),
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            scope: format!("workspace:{}", workspace.to_string_lossy()),
            max_command_timeout_ms: None,
        }
    }

    pub(crate) fn with_command_timeout(mut self, timeout_ms: Option<u64>) -> Self {
        self.max_command_timeout_ms = timeout_ms;
        if let Some(timeout_ms) = timeout_ms {
            self.scope
                .push_str(&format!(";command_timeout_ms={timeout_ms}"));
        }
        self
    }
}

/// 受信执行能力：只能由 `ToolHostService::issue` 创建，携带一次性调用参数。
///
/// Agent loop 不再直接从 `ToolRegistry` 取出工具并执行；它必须先经过这个
/// capability 门面。参数摘要进入收据，避免把原始参数写入审计日志。
#[derive(Debug, Clone)]
pub(crate) struct ToolCapability {
    tool: String,
    pub(super) tool_version: String,
    args: Value,
    args_sha256: String,
    workspace: String,
    session_id: String,
    turn_id: String,
    effect: EffectClass,
    scope: String,
    max_command_timeout_ms: Option<u64>,
    /// 短时效能力：审批结果不能被无限期重放。
    pub(super) expires_at_unix: u64,
    /// 每次签发的不可预测调用标识，写入收据用于关联但不写原始参数。
    pub(super) nonce: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolReceipt {
    pub tool: String,
    pub tool_version: String,
    pub session_id: String,
    pub turn_id: String,
    pub approved: bool,
    pub ok: bool,
    pub effect: EffectClass,
    pub scope_sha256: String,
    pub args_sha256: String,
    pub result_sha256: Option<String>,
    pub execution_receipt_id: Option<String>,
    pub changed_files: Vec<String>,
    pub diff_sha256: Option<String>,
    pub duration_ms: u64,
    pub expires_at_unix: u64,
    pub nonce: String,
}

pub(crate) trait ToolReceiptSink: Send + Sync {
    fn record(&self, receipt: ToolReceipt);
}

struct AuditReceiptSink {
    audit: Arc<Mutex<AuditLog>>,
}

impl ToolReceiptSink for AuditReceiptSink {
    fn record(&self, receipt: ToolReceipt) {
        let detail = json!({
            "tool_version": receipt.tool_version,
            "turn_id": receipt.turn_id,
            "approved": receipt.approved,
            "ok": receipt.ok,
            "effect": receipt.effect.label(),
            "scope_sha256": receipt.scope_sha256,
            "args_sha256": receipt.args_sha256,
            "result_sha256": receipt.result_sha256,
            "execution_receipt_id": receipt.execution_receipt_id,
            "changed_files": receipt.changed_files,
            "diff_sha256": receipt.diff_sha256,
            "duration_ms": receipt.duration_ms,
            "expires_at_unix": receipt.expires_at_unix,
            "nonce": receipt.nonce,
        })
        .to_string();
        if let Ok(mut audit) = self.audit.lock() {
            audit.record(
                &receipt.session_id,
                "tool_receipt",
                Some(receipt.tool),
                Some(receipt.approved),
                detail,
            );
        }
    }
}

fn tool_spec_fingerprint(spec: &ToolSpec) -> String {
    crate::CasStore::hash_of(
        json!({
            "name": spec.name,
            "input_schema": spec.input_schema,
            "effect": spec.effect,
        })
        .to_string()
        .as_bytes(),
    )
}

/// Agent 工具执行的唯一受信门面。
///
/// `ToolRegistry::get` 仍只在 core crate 内可见，但执行也必须通过这里完成：
/// 先签发带参数摘要的 capability，再由门面查找并运行工具，最后无论成功或
/// 失败都向 receipt sink 写入结构化收据。后续可在不改变 Agent loop 的情况下
/// 将 sink 替换为持久化/变更集收据实现。
#[derive(Clone)]
pub(crate) struct ToolHostService {
    registry: Arc<RwLock<ToolRegistry>>,
    receipt_sink: Arc<dyn ToolReceiptSink>,
}

impl ToolHostService {
    pub(crate) fn new(registry: Arc<RwLock<ToolRegistry>>, audit: Arc<Mutex<AuditLog>>) -> Self {
        Self {
            registry,
            receipt_sink: Arc::new(AuditReceiptSink { audit }),
        }
    }

    /// 只有最终通过审批的调用才能取得执行 capability。
    pub(crate) fn issue(
        &self,
        tool: &str,
        args: Value,
        approval: ToolApprovalGrant,
        context: ToolCapabilityContext,
    ) -> Result<ToolCapability, String> {
        let args_sha256 = crate::CasStore::hash_of(args.to_string().as_bytes());
        if approval.tool != tool {
            return Err(format!("approval tool mismatch: {tool}"));
        }
        if approval.args_sha256 != args_sha256 {
            return Err(format!("approval args mismatch: {tool}"));
        }
        if context.workspace.is_empty()
            || context.session_id.is_empty()
            || context.turn_id.is_empty()
            || context.scope.is_empty()
        {
            return Err(format!("capability context missing: {tool}"));
        }
        let spec = self
            .registry
            .read()
            .map_err(|_| "工具注册表锁中毒".to_string())?
            .get(tool)
            .map(|registered| registered.spec())
            .ok_or_else(|| format!("未知工具：{tool}"))?;
        let tool_version = tool_spec_fingerprint(&spec);
        let effect = spec
            .effect
            .as_ref()
            .map(|metadata| metadata.class)
            .unwrap_or_else(|| crate::tool_effects::effect_class_for(&spec.name));
        let issued_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时钟早于 Unix epoch".to_string())?
            .as_secs();
        Ok(ToolCapability {
            tool: tool.to_string(),
            tool_version,
            args,
            args_sha256,
            workspace: context.workspace,
            session_id: context.session_id,
            turn_id: context.turn_id,
            effect,
            scope: context.scope,
            max_command_timeout_ms: context.max_command_timeout_ms,
            expires_at_unix: issued_at_unix.saturating_add(60),
            nonce: format!("{}:{}", approval.request_id, uuid::Uuid::new_v4()),
        })
    }

    pub(crate) async fn execute(
        &self,
        capability: ToolCapability,
        ctx: &mut ToolContext<'_>,
    ) -> Result<Value, String> {
        let (tool, current_tool_version, current_effect) = self
            .registry
            .read()
            .map_err(|_| "工具注册表锁中毒".to_string())?
            .get(&capability.tool)
            .map(|registered| {
                let spec = registered.spec();
                (Some(registered), tool_spec_fingerprint(&spec), spec.effect)
            })
            .unwrap_or((None, String::new(), None));
        let live_request = ctx.policy.evaluate_with_effect(
            &capability.tool,
            current_effect.as_ref(),
            &capability.args,
        );
        let live_denial = ctx.policy.execution_denial(&live_request);
        let started = std::time::Instant::now();
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(u64::MAX);
        let current_workspace = ctx.workspace.to_string_lossy();
        let write_path = (capability.effect == EffectClass::Write)
            .then(|| capability.args.get("path").and_then(Value::as_str))
            .flatten()
            .and_then(|path| resolve_session_path(ctx, path).ok());
        let mut outcome = if capability.session_id != ctx.session.id {
            Err(format!("capability session mismatch: {}", capability.tool))
        } else if capability.workspace != current_workspace {
            Err(format!(
                "capability workspace mismatch: {}",
                capability.tool
            ))
        } else if capability.tool_version != current_tool_version {
            Err(format!(
                "capability tool version mismatch: {}",
                capability.tool
            ))
        } else if ctx
            .abort
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
        {
            Err(format!("executor/cancelled: {}", capability.tool))
        } else if let Some(reason) = live_denial {
            Err(format!("permission/revoked: {reason}"))
        } else if ctx.policy.is_read_only()
            && !current_effect.as_ref().is_some_and(|effect| {
                effect.class == EffectClass::Read && effect.host_verified_readonly
            })
        {
            Err(format!(
                "permission/read_only: {} is not host-verified readonly",
                capability.tool
            ))
        } else if now_unix >= capability.expires_at_unix {
            Err(format!("approval expired: {}", capability.tool))
        } else {
            let mut tool_args = capability.args.clone();
            if capability.tool == "run_command" {
                if let (Some(timeout_ms), Some(arguments)) =
                    (capability.max_command_timeout_ms, tool_args.as_object_mut())
                {
                    arguments.insert("_host_timeout_ms".to_string(), json!(timeout_ms));
                }
            }
            match tool {
                Some(tool) => tool.run(ctx, tool_args).await,
                None => Err(format!("未知工具：{}", capability.tool)),
            }
        };
        let mut write_paths = Vec::new();
        if let Some(path) = write_path.as_deref() {
            write_paths.push(path.to_path_buf());
        } else if capability.tool == "apply_patch" {
            if let Some(files) = outcome
                .as_ref()
                .ok()
                .and_then(|value| value.get("files"))
                .and_then(Value::as_array)
            {
                for path in files
                    .iter()
                    .filter_map(|file| file.get("path").and_then(Value::as_str))
                {
                    if let Ok(path) = resolve_session_path(ctx, path) {
                        write_paths.push(path);
                    }
                }
            }
        }
        let mut execution_receipts = Vec::new();
        if outcome.is_ok() {
            for path in write_paths {
                match ctx.session.record_file_execution(
                    &capability.tool,
                    &capability.turn_id,
                    &path,
                ) {
                    Ok(Some(receipt)) => execution_receipts.push(receipt),
                    Ok(None) => {}
                    Err(error) => {
                        outcome = Err(format!("写入收据失败：{error}"));
                        execution_receipts.clear();
                        break;
                    }
                }
            }
        }
        if let (Ok(Value::Object(result)), receipts) = (&mut outcome, &execution_receipts) {
            if !receipts.is_empty() {
                result.insert(
                    "execution_receipt_ids".to_string(),
                    Value::Array(
                        receipts
                            .iter()
                            .map(|receipt| Value::String(receipt.receipt_id.clone()))
                            .collect(),
                    ),
                );
                if receipts.len() == 1 {
                    result.insert(
                        "execution_receipt_id".to_string(),
                        Value::String(receipts[0].receipt_id.clone()),
                    );
                }
                let changed_files = receipts
                    .iter()
                    .flat_map(|receipt| receipt.changed_files.iter().cloned())
                    .collect::<Vec<_>>();
                result.insert(
                    "changed_files".to_string(),
                    Value::Array(changed_files.into_iter().map(Value::String).collect()),
                );
                result.insert(
                    "diff_sha256".to_string(),
                    Value::String(crate::CasStore::hash_of(
                        serde_json::to_vec(
                            &receipts
                                .iter()
                                .map(|receipt| &receipt.diff_sha256)
                                .collect::<Vec<_>>(),
                        )
                        .unwrap_or_default()
                        .as_slice(),
                    )),
                );
            }
        }
        let result_sha256 = outcome
            .as_ref()
            .ok()
            .map(|value| crate::CasStore::hash_of(value.to_string().as_bytes()));
        let execution_ok = super::execution_error(&capability.tool, &outcome).is_none();
        self.receipt_sink.record(ToolReceipt {
            tool: capability.tool,
            tool_version: capability.tool_version,
            session_id: ctx.session.id.clone(),
            turn_id: capability.turn_id,
            approved: true,
            ok: execution_ok,
            effect: capability.effect,
            scope_sha256: crate::CasStore::hash_of(capability.scope.as_bytes()),
            args_sha256: capability.args_sha256,
            result_sha256,
            execution_receipt_id: execution_receipts
                .first()
                .map(|receipt| receipt.receipt_id.clone()),
            changed_files: execution_receipts
                .iter()
                .flat_map(|receipt| receipt.changed_files.iter().cloned())
                .collect(),
            diff_sha256: (!execution_receipts.is_empty()).then(|| {
                crate::CasStore::hash_of(
                    serde_json::to_vec(
                        &execution_receipts
                            .iter()
                            .map(|receipt| &receipt.diff_sha256)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_default()
                    .as_slice(),
                )
            }),
            duration_ms: started.elapsed().as_millis() as u64,
            expires_at_unix: capability.expires_at_unix,
            nonce: capability.nonce,
        });
        outcome
    }
}

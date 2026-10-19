use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Read,
    Write,
    Execute,
    Inject,
}

impl Level {
    pub fn label(&self) -> &'static str {
        match self {
            Level::Read => "read",
            Level::Write => "write",
            Level::Execute => "execute",
            Level::Inject => "inject",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionProfile {
    /// 只允许宿主验证的只读操作。
    ReadOnly,
    /// 工作区内只读默认允许；写入首次审批，执行、联网、UI 控制、越界和破坏性操作询问。
    Workspace,
    /// 审批 Agent 可以收紧或代批「可代批」操作，但不能突破 OS 沙箱与 deny 规则。
    AutoReview,
    /// 减少询问，但不绕过审计、秘密脱敏和不可恢复操作确认。
    FullAccess,
    /// 完全权限（高风险，显式 opt-in）：允许**越界**文件访问与命令执行（等价 Codex
    /// danger-full-access）；仍不绕过 deny 规则、审计、秘密脱敏与注入类确认。
    Unrestricted,
    /// 用户按工具、路径、主机和时效组合规则。
    Custom,
}

impl PermissionProfile {
    pub fn label(self) -> &'static str {
        match self {
            PermissionProfile::ReadOnly => "read_only",
            PermissionProfile::Workspace => "workspace",
            PermissionProfile::AutoReview => "auto_review",
            PermissionProfile::FullAccess => "full_access",
            PermissionProfile::Unrestricted => "unrestricted",
            PermissionProfile::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read_only" => Some(Self::ReadOnly),
            "workspace" => Some(Self::Workspace),
            "auto_review" => Some(Self::AutoReview),
            "full_access" => Some(Self::FullAccess),
            "unrestricted" | "danger_full_access" => Some(Self::Unrestricted),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub request_id: String,
    pub tool: String,
    pub args: Value,
    pub level: Level,
    pub reason: String,
    /// §7.2/§7.4：风险说明（审批卡展示）。工具未声明风险信息时给出统一提示；
    /// 已知内置工具无额外说明时省略该字段（保持既有 JSON 契约形状）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_note: Option<String>,
    /// §5.5：脱敏后的参数视图（秘密字段只显示类型与长度），审批卡默认展示；
    /// 原始 args 仅在开发者详情中展开。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_args: Option<Value>,
}

impl PermissionRequest {
    /// 统一构造入口：业务模块必须经此创建权限请求，禁止直接拼装 request_id。
    /// request_id 由构造函数生成，保证全局唯一，业务调用点无需关心。
    pub fn new(
        tool: impl Into<String>,
        args: Value,
        level: Level,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            tool: tool.into(),
            args,
            level,
            reason: reason.into(),
            risk_note: None,
            redacted_args: None,
        }
    }

    /// 串联风险说明；`None` 表示省略该字段（保持既有 JSON 契约形状）。
    pub fn with_risk_note(mut self, note: Option<String>) -> Self {
        self.risk_note = note;
        self
    }

    /// §5.5：attach 脱敏参数视图（见 [`redact_args`]）。
    pub fn with_redacted_args(mut self, redacted: Option<Value>) -> Self {
        self.redacted_args = redacted;
        self
    }

    /// 是否为非只读动作。授权记忆只接受 Read 级且工具注册表也声明为 Read 的请求；
    /// 同时用于权限中心风险标记和拒绝危险级别的 Grant。
    pub fn is_destructive(&self) -> bool {
        self.level != Level::Read
            || crate::tool_effects::effect_class_for(&self.tool)
                != crate::tool_effects::EffectClass::Read
    }
}

/// §5.5 秘密字段识别词根：值为 string 且键匹配任一词根 → 只暴露类型与长度。
const SECRET_KEY_HINTS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "cookie",
    "authorization",
    "api_key",
    "apikey",
    "x-api",
    "key",
    "credential",
    "header",
];

/// §5.5 脱敏：递归遍历，string 值且键命中秘密词根 → `{"type":"string","length":N}`；
/// 其余原样。命令参数不整段抹掉（审批卡需要看做了什么），但 url/command 之外的
/// 高危键（如 file 内容、data）单独成对处理：data 只保留长度。
pub fn redact_args(args: &Value) -> Value {
    redact_inner(args)
}

/// 递归脱敏：string/array 值且键命中秘密词根 → 只暴露类型与长度；其余原样。
fn redact_inner(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let hint = SECRET_KEY_HINTS
                    .iter()
                    .any(|hint| key.to_lowercase().contains(hint));
                let value = if hint {
                    match child {
                        Value::String(text) => {
                            let length = text.chars().count();
                            out.insert(
                                key.clone(),
                                json!({ "type": "string", "length": length, "redacted": true }),
                            );
                            continue;
                        }
                        Value::Array(items) if !items.is_empty() => {
                            let total: usize = items
                                .iter()
                                .filter_map(|item| item.as_str())
                                .map(|text| text.chars().count())
                                .sum();
                            out.insert(
                                key.clone(),
                                json!({ "type": "array", "length": total, "redacted": true }),
                            );
                            continue;
                        }
                        _ => redact_inner(child),
                    }
                } else {
                    redact_inner(child)
                };
                out.insert(key.clone(), value);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact_inner).collect()),
        other => other.clone(),
    }
}

/// §5.5 可解释展示：一行为用户总结「将做什么 / 影响哪里 / 能否撤销」。
/// 返回 { action, target, undoable }（UI 与 CLI 共用；无敏感载荷）。
pub fn describe_request(request: &PermissionRequest) -> serde_json::Value {
    let mut action = request.tool.clone();
    let mut target = String::new();
    let mut undoable = false;
    let args = &request.args;
    match request.tool.as_str() {
        "read_file" | "write_file" | "edit_file" | "multi_edit" | "apply_patch" => {
            if let Some(path) = args.get("path").and_then(Value::as_str) {
                target = path.to_string();
            }
            undoable = request.tool != "read_file";
            action = match request.tool.as_str() {
                "read_file" => "读取文件".to_string(),
                "write_file" => "写入文件".to_string(),
                "apply_patch" => "应用补丁".to_string(),
                _ => "修改文件".to_string(),
            };
        }
        "run_command" => {
            if let Some(command) = args.get("command").and_then(Value::as_str) {
                let parts: Vec<&str> = command.split_whitespace().collect();
                if let Some(program) = parts.first() {
                    action = format!("执行命令：{program}");
                }
                target = parts
                    .iter()
                    .skip(1)
                    .take(6)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
            }
        }
        _ => {
            if let Some(url) = args.get("url").and_then(Value::as_str) {
                let domain = url
                    .strip_prefix("https://")
                    .or_else(|| url.strip_prefix("http://"))
                    .and_then(|rest| rest.split(['/', '?', '#']).next())
                    .unwrap_or(url);
                action = format!("联网访问：{domain}");
                target = url.to_string();
            }
        }
    }
    if !request.reason.starts_with("拒绝") {
        // 文件写与命令执行一般可通过 diff/revert 撤销（会话快照）。注入不可逆。
        undoable = undoable || request.tool == "run_command";
    }
    json!({
        "action": action,
        "target": target,
        "undoable": undoable,
        "risk": request.risk_note.clone().unwrap_or_default(),
    })
}

#[async_trait]
pub trait Approver: Send + Sync {
    async fn decide(&self, request: &PermissionRequest) -> Decision;

    /// Interactive transports register their responder before publishing the notification.
    async fn decide_with_notification(
        &self,
        request: &PermissionRequest,
        notify: &mut (dyn FnMut() + Send),
    ) -> ApprovalOutcome {
        notify();
        ApprovalOutcome {
            decision: self.decide(request).await,
            reason: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApprovalOutcome {
    pub decision: Decision,
    pub reason: Option<String>,
}

/// 测试/自动化用：统一放行或拒绝。
pub struct AutoApprover {
    pub allow: bool,
}

#[async_trait]
impl Approver for AutoApprover {
    async fn decide(&self, _request: &PermissionRequest) -> Decision {
        if self.allow {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}

/// 无交互审批通道（后台/嵌套子代理）用的工作区范围审批器。
///
/// 背景：直呼子代理（goal/plan 的 `AgentWorker`、`POST /subagent`）没有把审批卡
/// 回传给用户的通道；此前用 `AutoApprover { allow: read_only }` 兜底，导致
/// **producer 角色的写/执行一律被拒**——子代理"跑完了但什么都没改"，或者
/// 在等待一个永远不会到来的审批（直到超时）。
///
/// 语义（与 WorkSwarm 的 `WorkspaceScopeApprover` 同口径）：
/// - `Read` 恒放行（文件/命令路径已被 [`Policy`] 限制在工作区内）；
/// - `Write` 仅当 `allow_writes` 且目标路径在工作区内（绝对/相对路径都支持，
///   `\\?\` 前缀已剥离）——越界写一律拒绝；
/// - `Execute` / `Inject` 仅当 `allow_writes` **且 `allow_execute`**（显式 opt-in，
///   见 [`WorkspaceApprover::allow_execute`]）。
///
/// 注意：本审批器**不放宽** [`Policy`] 的判定——它只处理策略层已经判为"需要询问"
/// 的请求，且永远不放行工作区外的写入。
pub struct WorkspaceApprover {
    pub workspace: PathBuf,
    /// 是否允许工作区内的写（`false` = 只读子代理）。
    pub allow_writes: bool,
    /// 是否允许工作区内的命令执行/注入（**显式 opt-in，默认应设 false**）。
    ///
    /// 无交互审批通道的后台子代理里，读到的仓库文件/网页输出都可能带注入——
    /// 若默认放行 Execute，等于把"命令执行"交给不可信内容诱导的模型。因此
    /// 只有调用方明确声明"本次运行的用户已同意执行命令"时才置 true；
    /// 有审批通道的正常回合不经过本审批器。
    pub allow_execute: bool,
}

impl WorkspaceApprover {
    /// 工作区内的写路径判定：绝对路径直接用，相对路径相对 workspace 解析。
    fn write_path_allowed(&self, raw: &str) -> bool {
        let root = strip_verbatim_prefix(&self.workspace);
        let candidate = if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            root.join(raw)
        };
        // 不要求路径已存在（新建文件），逐级向上找到最近的存在祖先再比对。
        let resolved = nearest_existing_ancestor(&candidate);
        resolved.starts_with(&root)
    }
}

/// 最近的存在祖先（canonicalize 仅对存在路径有效；新建文件的父目录通常已存在）。
fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(&current) {
            return strip_verbatim_prefix(&canonical);
        }
        match current.parent() {
            Some(parent) if parent.as_os_str().is_empty() => {
                return strip_verbatim_prefix(&current);
            }
            Some(parent) => current = parent.to_path_buf(),
            None => return strip_verbatim_prefix(&current),
        }
    }
}

/// 剥离 Windows canonicalize 产物前缀 `\\?\`（`\\?\C:\a` → `C:\a`）。
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

#[async_trait]
impl Approver for WorkspaceApprover {
    async fn decide(&self, request: &PermissionRequest) -> Decision {
        match request.level {
            Level::Read => Decision::Allow,
            Level::Write => {
                if !self.allow_writes {
                    return Decision::Deny;
                }
                let path = request
                    .args
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                // 无 path 参数的写级请求（如 apply_patch 的多文件补丁）保守拒绝：
                // 多目标补丁无法在此逐条校验，交由带交互通道的路径处理。
                if path.is_empty() || !self.write_path_allowed(path) {
                    Decision::Deny
                } else {
                    Decision::Allow
                }
            }
            Level::Execute | Level::Inject => {
                if self.allow_writes && self.allow_execute {
                    Decision::Allow
                } else {
                    Decision::Deny
                }
            }
        }
    }
}

/// 权限策略：deny 优先，其次 profile 档位与授权记忆（grant），最后 ask。
/// 作用域：所有文件/命令路径必须位于 workspace 内。
pub struct Policy {
    workspace: PathBuf,
    deny_command_fragments: Vec<String>,
    /// 运行时追加的危险命令片段（热生效，与基础列表合并判断）。
    runtime_deny: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    read_only: Arc<AtomicBool>,
    request_read_only: bool,
    /// §5.3 权限档位（默认 Workspace：工作区内只读默认允许，首次写入/执行/联网/
    /// UI 控制/越界/破坏性操作询问；审批后由有作用域 Grant 记忆）。
    profile: std::sync::Arc<std::sync::Mutex<PermissionProfile>>,
    /// §5.4 有作用域、可撤销的授权记忆（用户审批选项生成；命中即放行）。
    grants:
        std::sync::Arc<std::sync::RwLock<Option<std::sync::Arc<crate::grant_store::GrantStore>>>>,
    /// §4.5.3 结构化 profile（权限中心提交）。`None` = 没有显式结构化配置，
    /// 判定完全按档位走（保持既有行为，老调用点零改动）。
    ///
    /// 这个维度层**只收紧不放宽**：`extra_denial` 只会给出 Deny；
    /// 而且优先于 Read 放行与 grant 命中——显式拒绝维度不能被授权记忆绕过。
    spec: std::sync::Arc<std::sync::Mutex<Option<crate::permission_spec::PermissionSpec>>>,
}

impl Policy {
    /// Bind path and grant identity to a session while sharing the host's live
    /// restrictions. This does not grant any capability or relax the policy.
    pub fn for_workspace(&self, workspace: impl Into<PathBuf>) -> Result<Self, String> {
        let _grants = self
            .grants
            .read()
            .map_err(|_| "permission grant registry poisoned".to_string())?;
        Ok(Self {
            workspace: workspace.into(),
            deny_command_fragments: self.deny_command_fragments.clone(),
            runtime_deny: Arc::clone(&self.runtime_deny),
            read_only: Arc::clone(&self.read_only),
            request_read_only: self.request_read_only,
            profile: Arc::clone(&self.profile),
            grants: Arc::clone(&self.grants),
            spec: Arc::clone(&self.spec),
        })
    }

    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            deny_command_fragments: vec![
                "rm -rf".to_string(),
                "sudo".to_string(),
                "shutdown".to_string(),
                "format c:".to_string(),
                "rd /s".to_string(),
                "remove-item -recurse".to_string(),
                "del /s".to_string(),
                "git push".to_string(),
                "git reset --hard".to_string(),
            ],
            runtime_deny: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            read_only: Arc::new(AtomicBool::new(false)),
            request_read_only: false,
            profile: std::sync::Arc::new(std::sync::Mutex::new(PermissionProfile::Workspace)),
            grants: Arc::new(std::sync::RwLock::new(None)),
            spec: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// 只读策略（Plan 模式）：写/执行/注入一律拒绝（档位=ReadOnly）。
    pub fn read_only(workspace: impl Into<PathBuf>) -> Self {
        let policy = Self::new(workspace);
        policy.read_only.store(true, Ordering::Relaxed);
        if let Ok(mut profile) = policy.profile.lock() {
            *profile = PermissionProfile::ReadOnly;
        }
        policy
    }

    /// §5.3 注入授权记忆存储（审批卡选项持久化；重复注入覆盖引用）。
    pub fn with_grants(self, grants: std::sync::Arc<crate::grant_store::GrantStore>) -> Self {
        self.set_grants(grants);
        self
    }

    /// §5.4 运行时注入授权记忆（Agent 已被引用共享时使用；覆盖既有引用）。
    pub fn set_grants(&self, grants: std::sync::Arc<crate::grant_store::GrantStore>) {
        if let Ok(mut current) = self.grants.write() {
            *current = Some(grants);
        }
    }

    /// §11 `/permissions` 视图：读取授权记忆存储（未注入则 None）。
    pub fn grant_store(&self) -> Option<std::sync::Arc<crate::grant_store::GrantStore>> {
        self.grants.read().ok().and_then(|guard| guard.clone())
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only.store(read_only, Ordering::Relaxed);
        if let Ok(mut profile) = self.profile.lock() {
            *profile = if read_only {
                PermissionProfile::ReadOnly
            } else {
                PermissionProfile::Workspace
            };
        }
    }

    /// 运行时切换只读模式（M12 提权为 `pub`：唯一调用方 `agent::set_read_only` 已在
    /// core，`pub(crate)` 跨 crate 后不可达——就是 M8 记档的可见性收缩）。
    pub fn set_read_only_runtime(&self, read_only: bool) {
        self.read_only.store(read_only, Ordering::Relaxed);
        if let Ok(mut profile) = self.profile.lock() {
            *profile = if read_only {
                PermissionProfile::ReadOnly
            } else {
                PermissionProfile::Workspace
            };
        }
    }

    /// §5.3 运行时切换权限档位（UI/CLI/HTTP 统一入口；进程重启后由 settings 恢复）。
    pub fn set_profile(&self, profile: PermissionProfile) {
        if let Ok(mut current) = self.profile.lock() {
            *current = profile;
        }
        self.read_only
            .store(profile == PermissionProfile::ReadOnly, Ordering::Relaxed);
    }

    pub fn profile(&self) -> PermissionProfile {
        if self.request_read_only {
            return PermissionProfile::ReadOnly;
        }
        self.profile
            .lock()
            .map(|guard| *guard)
            .unwrap_or(PermissionProfile::ReadOnly)
    }

    /// §4.5.3 提交结构化 profile（权限中心唯一写入口）。
    ///
    /// **档位只会被推到只读，永远不会被这次提交放宽**：
    /// - spec 三维全关（等价只读）→ 档位同步为 `ReadOnly`，让 settings 与运行时
    ///   说的是同一件事，不留"档位写着 workspace、实际什么都干不了"的分裂真相；
    /// - 其余情况**不动档位**。收紧由 [`decision`](Self::decision) 里的
    ///   `extra_denial` 层直接生效——它是纯减法，能把 FullAccess 下的命令关掉，
    ///   却不需要（也不应该）把用户的档位旋钮拧松。
    ///
    /// 为什么不做"取最近档位"的自动同步：`AutoReview` 在某些面比 `Workspace` 更严
    /// （工作区内写也要过审批链），任何"按 spec 反推档位"都会把这类更严的档位降级，
    /// 那正好违反"只收紧不放宽"。反推函数 [`crate::permission_spec::PermissionSpec::nearest_profile`]
    /// 因此只用于"是否等价只读"这一个判断。
    pub fn set_spec(&self, spec: crate::permission_spec::PermissionSpec) {
        let forces_read_only = spec.nearest_profile() == PermissionProfile::ReadOnly
            || self.profile() == PermissionProfile::ReadOnly;
        if let Ok(mut current) = self.spec.lock() {
            *current = Some(spec);
        }
        if forces_read_only {
            self.set_profile(PermissionProfile::ReadOnly);
        }
    }

    /// §4.5.3 当前结构化 profile（`None` = 未显式配置，判定只按档位走）。
    pub fn spec(&self) -> Option<crate::permission_spec::PermissionSpec> {
        self.spec.lock().ok().and_then(|guard| guard.clone())
    }

    /// 清除结构化 profile（回到纯档位驱动；权限中心"恢复档位默认"用）。
    pub fn clear_spec(&self) {
        if let Ok(mut current) = self.spec.lock() {
            *current = None;
        }
    }

    /// 追加额外危险命令片段（deny 优先；写入基础列表，构造时静态）。
    pub fn add_deny_command(&mut self, fragment: impl Into<String>) {
        let fragment = fragment.into().to_lowercase();
        if !self.deny_command_fragments.contains(&fragment) {
            self.deny_command_fragments.push(fragment);
        }
    }

    /// 运行时追加危险命令片段（热生效，不重建 Policy；进程重启后由 settings 恢复）。
    pub fn add_runtime_deny(&self, fragment: impl Into<String>) {
        let fragment = fragment.into().to_lowercase();
        if let Ok(mut runtime) = self.runtime_deny.lock() {
            if !runtime.contains(&fragment) {
                runtime.push(fragment);
            }
        }
    }

    /// 整体替换运行时 deny 片段（同样因调用方在 core 而提权为 `pub`）。
    pub fn replace_runtime_deny(&self, fragments: &[String]) {
        if let Ok(mut runtime) = self.runtime_deny.lock() {
            runtime.clear();
            for fragment in fragments {
                let fragment = fragment.to_lowercase();
                if !fragment.is_empty() && !runtime.contains(&fragment) {
                    runtime.push(fragment);
                }
            }
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.request_read_only || self.read_only.load(Ordering::Acquire)
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// 全部危险命令片段（基础 + 运行时，去重；诊断/可视化用只读访问器）。
    pub fn deny_fragments(&self) -> Vec<String> {
        let runtime = self
            .runtime_deny
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        let mut all = self.deny_command_fragments.clone();
        for fragment in runtime {
            if !all.contains(&fragment) {
                all.push(fragment);
            }
        }
        all
    }

    /// 工具 → 权限级别矩阵（内置映射，诊断面板展示）。
    pub fn tool_levels() -> Vec<(String, Level)> {
        [
            ("read_file", Level::Read),
            ("list_dir", Level::Read),
            ("search_files", Level::Read),
            ("git_status", Level::Read),
            ("git_diff", Level::Read),
            ("git_log", Level::Read),
            ("ask_user", Level::Read),
            ("screen_ocr", Level::Read),
            ("desktop_window_ocr", Level::Read),
            ("ocr_region", Level::Read),
            ("desktop_foreground", Level::Read),
            ("desktop_window_list", Level::Read),
            ("desktop_wait", Level::Read),
            ("desktop_wait_until", Level::Read),
            ("browser_snapshot", Level::Read),
            ("screen_vision", Level::Read),
            ("vision_verify", Level::Read),
            ("vision_ground", Level::Read),
            ("write_file", Level::Write),
            ("edit_file", Level::Write),
            ("multi_edit", Level::Write),
            ("apply_patch", Level::Write),
            ("browser_screenshot", Level::Write),
            ("browser_download_image", Level::Write),
            ("run_command", Level::Execute),
            ("browser_navigate", Level::Execute),
            ("browser_search", Level::Execute),
            ("browser_click", Level::Execute),
            ("browser_type", Level::Execute),
            ("browser_press", Level::Execute),
            ("browser_close", Level::Execute),
            ("text.inject", Level::Inject),
            ("clipboard", Level::Inject),
            ("desktop_click", Level::Inject),
            ("desktop_type", Level::Inject),
            ("desktop_key", Level::Inject),
            ("desktop_shortcut", Level::Inject),
            ("desktop_activate", Level::Inject),
            ("desktop_launch", Level::Inject),
            ("desktop_scroll", Level::Inject),
        ]
        .iter()
        .map(|(tool, level)| (tool.to_string(), *level))
        .collect()
    }

    /// §7.2：工具 → 权限级别由 ToolEffect 元数据驱动（注册表 → 内置矩阵 → Execute 兜底）。
    pub fn level_for(tool: &str) -> Level {
        crate::tool_effects::effect_class_for(tool).into()
    }

    /// 解析并校验路径位于 workspace 内（文件可尚不存在，校验父级）。
    pub fn resolve_within_workspace(&self, path: &str) -> Result<PathBuf, String> {
        resolve_within(&self.workspace, path)
    }

    pub fn evaluate(&self, tool: &str, args: &Value) -> PermissionRequest {
        self.evaluate_with_effect(tool, None, args)
    }

    /// §5.1：接收调用方已从 ToolSpec 解析出的 effect（唯一事实源），
    /// 不再回查进程级全局注册表；`effect=None` 时兜底按未登记高风险处理。
    pub fn evaluate_with_effect(
        &self,
        tool: &str,
        effect: Option<&crate::tool_effects::ToolEffect>,
        args: &Value,
    ) -> PermissionRequest {
        let level: Level = effect
            .map(|effect| effect.class)
            .unwrap_or_else(|| crate::tool_effects::effect_class_for(tool))
            .into();
        let reason = match tool {
            // 审查 N6：路径边界检查不能只覆盖 read_file/write_file——edit_file/
            // multi_edit 同样是单 path 参数的写入工具，漏检会让"越界在 evaluate
            // 阶段拒绝"的承诺失效。apply_patch 的多目标在工具内逐条解析校验。
            "read_file" | "write_file" | "edit_file" | "multi_edit" => {
                let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
                match self.resolve_within_workspace(path) {
                    Ok(_) => format!("{level} 文件操作（工作区内）", level = level.label()),
                    Err(_) if self.profile() == PermissionProfile::Unrestricted => {
                        format!(
                            "{level} 文件操作（工作区外，unrestricted）",
                            level = level.label()
                        )
                    }
                    Err(e) => format!("拒绝：{e}"),
                }
            }
            "list_dir" | "search_files" => "目录/搜索操作".to_string(),
            "run_command" => {
                let command = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match crate::command_guard::dangerous_command_match(command, &self.deny_fragments())
                {
                    Some(pattern) => format!("拒绝：命令命中危险模式（{pattern}）"),
                    None => format!("执行命令：{command}"),
                }
            }
            _ => format!("工具 {tool} 需要审批"),
        };
        let risk_note = effect
            .and_then(|effect| effect.risk_note.clone())
            .or_else(|| {
                crate::tool_effects::effect_for(tool)
                    .and_then(|effect| effect.risk_note)
                    .or(Some(crate::tool_effects::UNDECLARED_RISK_NOTE.to_string()))
            });
        PermissionRequest::new(tool, args.clone(), level, reason)
            .with_risk_note(risk_note)
            .with_redacted_args(Some(redact_args(args)))
    }

    /// Apply an immutable request restriction without changing the shared host policy.
    pub fn read_only_scope(mut self) -> Self {
        self.request_read_only = true;
        self
    }

    /// Recheck only denials before executing an already approved capability.
    /// This does not consume a reusable grant or replace a required approval.
    pub fn execution_denial(&self, request: &PermissionRequest) -> Option<String> {
        if request.reason.starts_with("拒绝") {
            return Some(request.reason.clone());
        }
        if self
            .spec()
            .is_some_and(|spec| spec.extra_denial(request).is_some())
        {
            return Some("拒绝：当前权限维度禁止此操作".to_string());
        }
        if request.level != Level::Read
            && (self.is_read_only() || self.profile() == PermissionProfile::ReadOnly)
        {
            return Some("拒绝：本次回合或宿主处于只读模式".to_string());
        }
        None
    }

    /// 工具执行前的最终判定（拒绝原因通过 request.reason 表达）。
    ///
    /// §5.3/§5.4 判定顺序：deny 优先 → §4.5.3 维度收紧层 → 等级档位（profile）
    /// → 授权记忆（grant）→ ask。
    /// grant 命中放行与 profile 档位叠加，但绝不越过 deny 规则（reason 以"拒绝"开头
    /// 恒为 Deny，false 优先）。
    pub fn decision(&self, request: &PermissionRequest) -> Decision {
        if self.execution_denial(request).is_some() {
            return Decision::Deny;
        }
        if request.level == Level::Read {
            return Decision::Allow;
        }
        if self.is_read_only() || self.profile() == PermissionProfile::ReadOnly {
            return Decision::Deny;
        }
        // §5.4 授权记忆命中（同一工作区 + 同一参数指纹）即放行。
        if let Some(grants) = self.grants.read().ok().and_then(|guard| guard.clone()) {
            if grants.consume(request, &self.workspace_id()).is_some() {
                return Decision::Allow;
            }
        }
        match self.profile() {
            // Workspace：工作区内写入也必须首次审批；审批卡生成的有作用域 Grant
            // 已在上方命中后直接放行。执行/注入/联网/UI 控制仍询问；越界已在
            // evaluate 阶段拒绝。
            PermissionProfile::Workspace => Decision::Ask,
            // AutoReview：审批 Agent 可收紧或代批「可代批」操作——工作区内写
            // 不自动放行，一律进入审批链（reviewer 代批 / 人工审批），否则收紧
            // 无从生效；执行/注入照旧询问。
            PermissionProfile::AutoReview => match request.level {
                Level::Write | Level::Execute | Level::Inject => Decision::Ask,
                Level::Read => Decision::Allow,
            },
            // FullAccess：减少询问——Write 与 Execute 放行，Inject 仍询问
            // （注入/UI 控制不可逆，即使全权模式也不绕过确认）。
            PermissionProfile::FullAccess => match request.level {
                Level::Write | Level::Execute => Decision::Allow,
                Level::Inject => Decision::Ask,
                Level::Read => Decision::Allow,
            },
            // Unrestricted：完全权限（显式 opt-in）——读/写/执行放行，越界已在 evaluate
            // 阶段放行；Inject 仍 Ask；deny 规则与维度收紧层已在上方生效。
            PermissionProfile::Unrestricted => match request.level {
                Level::Read | Level::Write | Level::Execute => Decision::Allow,
                Level::Inject => Decision::Ask,
            },
            PermissionProfile::Custom => match request.level {
                Level::Write if request.reason.contains("工作区内") => Decision::Allow,
                Level::Execute | Level::Inject => Decision::Ask,
                Level::Read => Decision::Allow,
                Level::Write => Decision::Ask,
            },
            PermissionProfile::ReadOnly => Decision::Deny, // 上方已拦截，防御性分支
        }
    }

    /// 授权记忆匹配用的工作区标识（canonical 后字符串；不可读取时回退原始路径）。
    pub fn workspace_id(&self) -> String {
        self.workspace
            .canonicalize()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| self.workspace.to_string_lossy().into_owned())
    }
}

pub fn resolve_within(workspace: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        workspace.join(raw)
    };
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|e| format!("工作区不可访问：{e}"))?;
    let canonical_candidate =
        canonicalize_existing_parent(&candidate).map_err(|e| format!("路径校验失败：{e}"))?;
    if !canonical_candidate.starts_with(&canonical_workspace) {
        return Err(format!("路径位于工作区之外：{path}"));
    }
    Ok(candidate)
}

fn canonicalize_existing_parent(path: &Path) -> std::io::Result<PathBuf> {
    if path.exists() {
        return path.canonicalize();
    }
    let mut current = path;
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if current.exists() {
            let mut base = current.canonicalize()?;
            for part in suffix.iter().rev() {
                base.push(part);
            }
            return Ok(base);
        }
        match current.parent() {
            Some(parent) => {
                if let Some(name) = current.file_name() {
                    suffix.push(name.to_os_string());
                }
                current = parent;
            }
            None => return Ok(path.to_path_buf()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // WorkspaceApprover：无交互通道（后台/嵌套子代理）的工作区范围审批
    // -----------------------------------------------------------------------

    fn workspace_approver(
        workspace: &Path,
        allow_writes: bool,
        allow_execute: bool,
    ) -> WorkspaceApprover {
        WorkspaceApprover {
            workspace: workspace.to_path_buf(),
            allow_writes,
            allow_execute,
        }
    }

    fn request(level: Level, args: serde_json::Value) -> PermissionRequest {
        PermissionRequest::new("tool", args, level, "测试")
    }

    #[tokio::test]
    async fn workspace_approver_allows_reads_and_in_workspace_writes() {
        let workspace =
            std::env::temp_dir().join(format!("owo-ws-approver-{}", std::process::id()));
        std::fs::create_dir_all(&workspace).unwrap();
        let approver = workspace_approver(&workspace, true, true);
        assert_eq!(
            approver
                .decide(&request(Level::Read, json!({ "path": "a.txt" })))
                .await,
            Decision::Allow,
            "读恒放行（路径已由 Policy 限制在工作区内）"
        );
        assert_eq!(
            approver
                .decide(&request(Level::Write, json!({ "path": "sub/new.txt" })))
                .await,
            Decision::Allow,
            "工作区内相对路径写应放行（新建文件的父目录无需存在）"
        );
        assert_eq!(
            approver
                .decide(&request(
                    Level::Write,
                    json!({ "path": workspace.join("abs.txt").to_string_lossy() })
                ))
                .await,
            Decision::Allow,
            "工作区内绝对路径写应放行"
        );
        assert_eq!(
            approver
                .decide(&request(Level::Execute, json!({ "command": "echo hi" })))
                .await,
            Decision::Allow,
            "显式 opt-in 执行时放行（危险片段仍由 Policy 黑名单拦截）"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn workspace_approver_denies_execute_without_explicit_optin() {
        let workspace =
            std::env::temp_dir().join(format!("owo-ws-approver-exec-{}", std::process::id()));
        std::fs::create_dir_all(&workspace).unwrap();
        // 允许写但未显式允许执行：文件写放行，命令执行拒绝（防注入诱导执行）。
        let approver = workspace_approver(&workspace, true, false);
        assert_eq!(
            approver
                .decide(&request(Level::Write, json!({ "path": "a.txt" })))
                .await,
            Decision::Allow,
            "工作区内写不受执行开关影响"
        );
        assert_eq!(
            approver
                .decide(&request(Level::Execute, json!({ "command": "echo hi" })))
                .await,
            Decision::Deny,
            "未显式 opt-in 时执行必须拒绝"
        );
        assert_eq!(
            approver
                .decide(&request(Level::Inject, json!({ "command": "echo hi" })))
                .await,
            Decision::Deny,
            "注入级同样 fail-closed"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[tokio::test]
    async fn workspace_approver_denies_out_of_workspace_and_readonly() {
        let workspace =
            std::env::temp_dir().join(format!("owo-ws-approver-deny-{}", std::process::id()));
        std::fs::create_dir_all(&workspace).unwrap();
        let approver = workspace_approver(&workspace, true, true);
        let outside = workspace.parent().unwrap().join("owo-outside.txt");
        assert_eq!(
            approver
                .decide(&request(
                    Level::Write,
                    json!({ "path": outside.to_string_lossy() })
                ))
                .await,
            Decision::Deny,
            "越界写必须拒绝（无交互通道也不能放开工作区外）"
        );
        assert_eq!(
            approver
                .decide(&request(Level::Write, json!({ "path": "../escape.txt" })))
                .await,
            Decision::Deny,
            "相对路径越界同样拒绝"
        );
        assert_eq!(
            approver
                .decide(&request(Level::Write, json!({ "path": "" })))
                .await,
            Decision::Deny,
            "无 path 参数的写级请求（多文件补丁）保守拒绝"
        );

        let read_only = workspace_approver(&workspace, false, false);
        assert_eq!(
            read_only
                .decide(&request(Level::Write, json!({ "path": "a.txt" })))
                .await,
            Decision::Deny,
            "只读模式拒绝工作区内写"
        );
        assert_eq!(
            read_only
                .decide(&request(Level::Execute, json!({ "command": "echo hi" })))
                .await,
            Decision::Deny,
            "只读模式拒绝执行"
        );
        assert_eq!(
            read_only
                .decide(&request(Level::Read, json!({ "path": "a.txt" })))
                .await,
            Decision::Allow,
            "只读模式仍允许读"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }
    #[test]
    fn read_only_policy_denies_writes() {
        let policy = Policy::read_only(".");
        let request = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&request), Decision::Deny);
    }

    #[test]
    fn read_only_policy_allows_reads() {
        let policy = Policy::read_only(".");
        let request = policy.evaluate("read_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&request), Decision::Allow);
    }

    #[test]
    fn session_workspace_view_rebases_paths_and_keeps_parent_restrictions_live() {
        let root = std::env::temp_dir().join(format!("owo-policy-view-{}", uuid::Uuid::new_v4()));
        let launch = root.join("launch");
        let session = root.join("session");
        std::fs::create_dir_all(&launch).unwrap();
        std::fs::create_dir_all(&session).unwrap();
        let file = session.join("note.txt");
        std::fs::write(&file, "fixture").unwrap();

        let mut parent = Policy::new(&launch);
        parent.add_deny_command("parent-denied".to_string());
        let scoped = parent.for_workspace(&session).unwrap();
        assert!(Arc::ptr_eq(&parent.grants, &scoped.grants));
        let absolute = json!({ "path": file.to_string_lossy() });
        assert_eq!(
            parent.decision(&parent.evaluate("read_file", &absolute)),
            Decision::Deny
        );
        assert_eq!(
            scoped.decision(&scoped.evaluate("read_file", &absolute)),
            Decision::Allow
        );
        assert_eq!(
            scoped.decision(&scoped.evaluate(
                "write_file",
                &json!({ "path": "new.txt", "content": "fixture" })
            )),
            Decision::Ask
        );
        assert_eq!(
            scoped.decision(&scoped.evaluate(
                "read_file",
                &json!({ "path": launch.join("outside.txt").to_string_lossy() })
            )),
            Decision::Deny
        );
        assert_eq!(
            scoped
                .decision(&scoped.evaluate("run_command", &json!({ "command": "parent-denied" }))),
            Decision::Deny
        );
        assert_ne!(parent.workspace_id(), scoped.workspace_id());
        parent.set_profile(PermissionProfile::ReadOnly);
        assert_eq!(
            scoped.decision(&scoped.evaluate("write_file", &json!({ "path": "new.txt" }))),
            Decision::Deny
        );

        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(session).unwrap();
        std::fs::remove_dir(launch).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn request_read_only_is_immutable_and_does_not_change_the_host_profile() {
        let parent = Policy::new(".");
        let scoped = parent.for_workspace(".").unwrap().read_only_scope();
        assert_eq!(parent.profile(), PermissionProfile::Workspace);
        assert_eq!(scoped.profile(), PermissionProfile::ReadOnly);
        assert_eq!(
            scoped.decision(&scoped.evaluate("run_command", &json!({ "command": "echo fixture" }))),
            Decision::Deny
        );
        parent.set_read_only_runtime(false);
        assert!(scoped.is_read_only());
        assert_eq!(
            scoped.decision(&scoped.evaluate("write_file", &json!({ "path": "fixture.txt" }))),
            Decision::Deny
        );
        assert_eq!(
            parent.decision(&parent.evaluate("write_file", &json!({ "path": "fixture.txt" }))),
            Decision::Ask
        );
        let inherited = scoped.for_workspace(".").unwrap();
        assert!(inherited.is_read_only());
    }

    #[test]
    fn live_denial_rechecks_runtime_rules_without_consuming_approval() {
        let parent = Policy::new(".");
        let scoped = parent.for_workspace(".").unwrap();
        let args = json!({ "command": "echo fixture" });
        assert!(scoped
            .execution_denial(&scoped.evaluate("run_command", &args))
            .is_none());
        parent.add_runtime_deny("echo fixture");
        assert!(scoped
            .execution_denial(&scoped.evaluate("run_command", &args))
            .is_some());
        parent.set_profile(PermissionProfile::ReadOnly);
        assert!(scoped
            .execution_denial(
                &scoped.evaluate("run_command", &json!({ "command": "another fixture" }))
            )
            .is_some());
    }

    #[test]
    fn default_workspace_profile_asks_for_workspace_writes() {
        // 指南 §4.5：工作区内写入首次必须审批；后续由 Grant 命中放行。
        let policy = Policy::new(".");
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&write), Decision::Ask);
    }

    #[test]
    fn workspace_profile_asks_for_execute_and_out_of_workspace() {
        let policy = Policy::new(".");
        let command = policy.evaluate("run_command", &json!({ "command": "ls" }));
        assert_eq!(policy.decision(&command), Decision::Ask, "执行始终询问");
        let out_of_scope = policy.evaluate("write_file", &json!({ "path": "../escape.txt" }));
        assert_eq!(
            policy.decision(&out_of_scope),
            Decision::Deny,
            "越界写入在 evaluate 阶段拒绝"
        );
    }

    #[test]
    fn unrestricted_profile_allows_out_of_workspace_but_keeps_danger_deny() {
        // 显式 opt-in 的完全权限：越界文件放行，但 deny 黑名单仍然拒绝。
        let policy = Policy::new(".");
        policy.set_profile(PermissionProfile::Unrestricted);

        let outside = if cfg!(windows) {
            "C:/owo-outside.txt"
        } else {
            "/tmp/owo-outside.txt"
        };
        let read = policy.evaluate("read_file", &json!({ "path": outside }));
        assert!(
            !read.reason.starts_with("拒绝"),
            "unrestricted 下越界不应拒绝：{}",
            read.reason
        );
        assert_eq!(policy.decision(&read), Decision::Allow);
        let write = policy.evaluate("write_file", &json!({ "path": outside }));
        assert_eq!(policy.decision(&write), Decision::Allow);

        let danger = policy.evaluate("run_command", &json!({ "command": "shutdown /s" }));
        assert!(
            danger.reason.starts_with("拒绝"),
            "危险命令仍应拒绝：{}",
            danger.reason
        );
        assert_eq!(policy.decision(&danger), Decision::Deny);

        // 回到 workspace 档位：越界恢复拒绝。
        policy.set_profile(PermissionProfile::Workspace);
        let blocked = policy.evaluate("read_file", &json!({ "path": outside }));
        assert!(
            blocked.reason.starts_with("拒绝"),
            "workspace 档位应拒绝越界：{}",
            blocked.reason
        );
        assert_eq!(policy.decision(&blocked), Decision::Deny);
    }

    #[test]
    fn full_access_profile_reduces_questions_but_keeps_inject_ask() {
        // §5.3 FullAccess：减少询问，但不绕过注入/不可逆操作确认。
        let policy = Policy::new(".");
        policy.set_profile(PermissionProfile::FullAccess);
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&write), Decision::Allow);
        let command = policy.evaluate("run_command", &json!({ "command": "ls" }));
        assert_eq!(policy.decision(&command), Decision::Allow, "执行也减少询问");
        let inject = policy.evaluate("desktop_type", &json!({ "text": "hello" }));
        assert_eq!(
            policy.decision(&inject),
            Decision::Ask,
            "注入/UI 控制即使全权模式也确认"
        );
    }

    #[test]
    fn read_only_profile_via_set_profile_denies_writes() {
        let policy = Policy::new(".");
        policy.set_profile(PermissionProfile::ReadOnly);
        assert!(policy.is_read_only());
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&write), Decision::Deny);
    }

    #[test]
    fn destructive_requests_ignore_legacy_grants() {
        use crate::grant_store::{Grant, GrantScope, GrantStore};
        let store = std::sync::Arc::new(GrantStore::new());
        let policy = Policy::new(".").with_grants(store.clone());
        let request = policy.evaluate("run_command", &json!({ "command": "ls -la" }));
        let workspace_id = policy.workspace_id();
        assert_eq!(policy.decision(&request), Decision::Ask);
        assert!(store
            .grant_from_scope(&request, &workspace_id, GrantScope::Task)
            .is_none());

        // 即使旧版或导入的数据里已有执行工具授权，匹配阶段也必须拒绝复用。
        store.insert(Grant::test("run_command", &workspace_id, None, None));
        assert_eq!(policy.decision(&request), Decision::Ask);
    }
    #[test]
    fn custom_deny_command_fragment_is_enforced() {
        let mut policy = Policy::new(".");
        policy.add_deny_command("danger-command");
        let request = policy.evaluate(
            "run_command",
            &json!({ "command": "danger-command --force" }),
        );
        assert_eq!(policy.decision(&request), Decision::Deny);
    }

    #[test]
    fn runtime_policy_settings_take_effect_without_rebuilding() {
        let policy = Policy::new(".");
        // 默认 Workspace：工作区内写首次询问。切 ReadOnly 后拒绝，切回恢复询问。
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&write), Decision::Ask);

        policy.set_read_only_runtime(true);
        assert_eq!(policy.decision(&write), Decision::Deny);
        policy.set_read_only_runtime(false);
        assert_eq!(policy.decision(&write), Decision::Ask);

        policy.replace_runtime_deny(&["danger-now".to_string()]);
        let denied = policy.evaluate("run_command", &json!({ "command": "danger-now" }));
        assert_eq!(policy.decision(&denied), Decision::Deny);
        policy.replace_runtime_deny(&[]);
        let allowed_to_ask = policy.evaluate("run_command", &json!({ "command": "danger-now" }));
        assert_eq!(policy.decision(&allowed_to_ask), Decision::Ask);
    }

    #[test]
    fn redact_hides_secret_fields_but_keeps_paths() {
        let args = json!({
            "path": "a/b.txt",
            "content": "秘密正文",
            "url": "https://example.com/api",
            "headers": { "Authorization": "Bearer sk-12345", "X-Key": "sec" },
            "command": "run-task --flag"
        });
        let redacted = redact_args(&args);
        assert_eq!(redacted["path"], json!("a/b.txt"), "路径不脱敏");
        assert_eq!(
            redacted["content"],
            json!("秘密正文"),
            "正文按原样保留（UI 看做什么）"
        );
        assert_eq!(
            redacted["url"],
            json!("https://example.com/api"),
            "URL 保留域名便于判断"
        );
        assert_eq!(
            redacted["command"],
            json!("run-task --flag"),
            "命令保留便于判断"
        );
        let header = &redacted["headers"]["Authorization"];
        assert_eq!(header["type"], json!("string"));
        assert_eq!(header["length"], json!(15), "Bearer sk-12345 长度");
        assert_eq!(header["redacted"], json!(true), "秘密字段标记 redacted");
        assert!(
            redacted["headers"]["Authorization"].get("text").is_none(),
            "不携带原文"
        );
    }

    /// multi_edit（远端 engine 取优）：与 edit_file 同为 Write 级，审批卡展示目标与可撤销。
    #[test]
    fn multi_edit_is_write_level_and_described_with_target() {
        assert_eq!(Policy::level_for("multi_edit"), Level::Write);
        let request = Policy::new(".").evaluate(
            "multi_edit",
            &json!({ "path": "a.txt", "edits": [{ "old_string": "x", "new_string": "y" }] }),
        );
        let summary = describe_request(&request);
        assert_eq!(summary["action"], json!("修改文件"));
        assert_eq!(summary["target"], json!("a.txt"));
        assert_eq!(summary["undoable"], json!(true));
    }

    #[test]
    fn describe_write_shows_target_and_undoable() {
        let request = Policy::new(".").evaluate("write_file", &json!({ "path": "a.txt" }));
        let summary = describe_request(&request);
        assert_eq!(summary["action"], json!("写入文件"));
        assert_eq!(summary["target"], json!("a.txt"));
        assert_eq!(summary["undoable"], json!(true));
    }

    #[test]
    fn inject_tool_is_destructive_and_read_file_is_not() {
        let write = Policy::new(".").evaluate("write_file", &json!({ "path": "a.txt" }));
        assert!(write.is_destructive(), "写操作不提供始终允许只读");
        let read = Policy::new(".").evaluate("read_file", &json!({ "path": "a.txt" }));
        assert!(!read.is_destructive(), "只读操作可始终允许");
        let inject = Policy::new(".").evaluate("desktop_type", &json!({ "text": "hi" }));
        assert!(inject.is_destructive(), "注入不可逆，不允许授权记忆");
        let misclassified = PermissionRequest::new(
            "write_file",
            json!({ "path": "a.txt" }),
            Level::Read,
            "错误等级夹具",
        );
        assert!(
            misclassified.is_destructive(),
            "工具注册表为 Write 时，不得被错误的 Read 标签降级"
        );
    }

    /// §4.5.3 测试夹具：四个维度独立可设，scopes 留空。
    fn spec(
        filesystem: crate::permission_spec::FilesystemScope,
        command: crate::permission_spec::RuleScope,
        network: crate::permission_spec::RuleScope,
    ) -> crate::permission_spec::PermissionSpec {
        crate::permission_spec::PermissionSpec {
            filesystem,
            command,
            network,
            persistence: crate::permission_spec::PersistenceScope::Once,
            scopes: Vec::new(),
        }
    }

    #[test]
    fn filesystem_deny_beats_read_default() {
        use crate::permission_spec::{FilesystemScope, RuleScope};
        let policy = Policy::new(".");
        let request = policy.evaluate("read_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&request), Decision::Allow);
        policy.set_spec(spec(
            FilesystemScope::None,
            RuleScope::Deny,
            RuleScope::Deny,
        ));
        assert_eq!(policy.decision(&request), Decision::Deny);
        assert_eq!(policy.profile(), PermissionProfile::ReadOnly);
    }
    #[test]
    fn filesystem_none_denies_reads_too() {
        // Read 级默认放行是既有语义；filesystem:none 是唯一能把「读」也关掉的面板，
        // 所以收紧层必须排在 Read 放行之前。
        use crate::permission_spec::{FilesystemScope, RuleScope};
        let policy = Policy::new(".");
        let read = policy.evaluate("read_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&read), Decision::Allow);
        policy.set_spec(spec(
            FilesystemScope::None,
            RuleScope::Deny,
            RuleScope::Deny,
        ));
        assert_eq!(
            policy.decision(&read),
            Decision::Deny,
            "文件系统=none 时连读取默认放行也要让位"
        );
        assert_eq!(
            policy.profile(),
            PermissionProfile::ReadOnly,
            "三维全关时档位同步为只读（单一真相，不留分裂）"
        );
    }

    #[test]
    fn absent_spec_leaves_profile_semantics_untouched() {
        // 回归护栏：没有结构化配置时仍遵循默认档位；工作区写入首次询问，
        // 只读默认放行，执行与注入询问。
        let policy = Policy::new(".");
        assert!(policy.spec().is_none(), "默认无结构化配置");
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        let exec = policy.evaluate("run_command", &json!({ "command": "ls" }));
        let inject = policy.evaluate("desktop_type", &json!({ "text": "hi" }));
        assert_eq!(policy.decision(&write), Decision::Ask);
        assert_eq!(policy.decision(&exec), Decision::Ask);
        assert_eq!(policy.decision(&inject), Decision::Ask);

        policy.clear_spec();
        assert_eq!(policy.decision(&write), Decision::Ask);
        assert_eq!(policy.decision(&exec), Decision::Ask);
        assert_eq!(
            policy.decision(&inject),
            Decision::Ask,
            "clear_spec 不改变档位判定"
        );
    }

    #[test]
    fn read_only_is_an_upper_bound_spec_cannot_lift() {
        // 只读模式必须是上界：提交一张"文件可写 + 命令不限"的结构化配置，
        // 判定仍然是拒绝；档位也不会被抬回可写。这是本次改动唯一的放宽风险面。
        use crate::permission_spec::{
            FilesystemScope, PermissionSpec, PersistenceScope, RuleScope,
        };
        let policy = Policy::read_only(".");
        let write = policy.evaluate("write_file", &json!({ "path": "a.txt" }));
        assert_eq!(policy.decision(&write), Decision::Deny);
        policy.set_spec(PermissionSpec {
            filesystem: FilesystemScope::WorkspaceWrite,
            command: RuleScope::Unrestricted,
            network: RuleScope::Unrestricted,
            persistence: PersistenceScope::Workspace,
            scopes: vec![],
        });
        assert_eq!(
            policy.profile(),
            PermissionProfile::ReadOnly,
            "只读档不能因为 spec 更宽而被同步抬升"
        );
        assert!(policy.is_read_only(), "read_only 开关必须保持为真");
        assert_eq!(
            policy.decision(&write),
            Decision::Deny,
            "更宽的结构化配置不得让写入通过"
        );
    }

    #[test]
    fn spec_tightens_without_moving_the_profile_dial() {
        // 收紧靠 extra_denial，不靠改档位：FullAccess 下关掉命令维度后，
        // 判定立刻拒绝，但档位旋钮仍停留在人亲手选的位置（不会被这次提交拧松）。
        use crate::permission_spec::{
            FilesystemScope, PermissionSpec, PersistenceScope, RuleScope,
        };
        let policy = Policy::new(".");
        policy.set_profile(PermissionProfile::FullAccess);
        let exec = policy.evaluate("run_command", &json!({ "command": "ls" }));
        assert_eq!(policy.decision(&exec), Decision::Allow, "全权档下执行放行");
        policy.set_spec(PermissionSpec {
            filesystem: FilesystemScope::WorkspaceWrite,
            command: RuleScope::Deny,
            network: RuleScope::Unrestricted,
            persistence: PersistenceScope::Task,
            scopes: vec![],
        });
        assert_eq!(
            policy.decision(&exec),
            Decision::Deny,
            "命令维度关掉后必须拒绝"
        );
        assert_eq!(
            policy.profile(),
            PermissionProfile::FullAccess,
            "档位不被这次提交改动（AutoReview 等更严档位同理，不会被降级）"
        );
    }
}

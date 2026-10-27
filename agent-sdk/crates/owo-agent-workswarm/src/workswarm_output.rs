//! Worker 结构化输出契约（V1）：`WorkerOutputV1`。
//!
//! 背景（R1 live 基线 + R3 冒烟）：真实模型 Worker 偶发输出自由文本冒充交付物、
//! 整份非法 JSON（document-structured-extract 曾 0/3）、或 Critic 评审文本被
//! 误选为最终交付物。本契约让每个 Agent Worker 返回**可机器校验**的结构化结果：
//!
//! - Producer 类角色（producer/builder/researcher/writer/leader）必须提交
//!   `artifact { kind, format, content }`——交付物正文只取 `artifact.content`，
//!   不再拿整段自由文本当产物；
//! - review capability **不得**提交 artifact；必须提交结构化 `review_result`，宿主绑定评审范围，
//!   其产物登记为 `review` 类且永不参与最终交付选择；
//! - 解析失败由调用方执行**恰好一次**定向修复（见执行器 `repair_output_once`），
//!   仍失败以 `output_contract_invalid` 定位上报，禁止无限重试。
//!
//! 兼容性：非契约路径（服务端 echo worker / 未启用契约提示的旧流程）的纯文本
//! 输出走 Legacy 语义，登记行为不变。

use serde::{Deserialize, Serialize};

/// 契约标识（调试/取证字段；解析不强制要求）。
pub const WORKER_OUTPUT_CONTRACT: &str = "worker-output-v1";

/// Worker 终态声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutputStatus {
    /// 正常完成（producer 需附 artifact）。
    Done,
    /// 自身无法完成（原因必须写入 summary/open_issues）。
    Failed,
    /// 被阻塞（等待上游/权限/预算，原因必须写入 summary）。
    Blocked,
}

impl WorkerOutputStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }
}

/// 交付物/产物描述（只允许 Producer 类角色提交）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerArtifactV1 {
    /// 产物分类（document/research/plan/json/csv/…；应与团队目标一致）。
    pub kind: String,
    /// 内容格式：text | markdown | json | csv。
    pub format: String,
    /// 产物正文本体（登记进版本化 Artifact 的就是这段内容）。
    pub content: String,
}

/// 证据引用（研究/审计链；自由文本来源描述 + 可选要点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEvidenceV1 {
    /// 来源描述（文件路径 / URL / 引用名）。
    pub source: String,
    /// 可选说明（摘录、相关性、可信度）。
    #[serde(default)]
    pub note: Option<String>,
}

/// 独立评审结论。被审查的产物身份与哈希由宿主从当前上下文绑定，不能由模型声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerReviewResultV1 {
    pub verdict: WorkerReviewVerdict,
    /// Exact host-provided requirement IDs the reviewer considered.
    #[serde(default)]
    pub reviewed_requirement_ids: Vec<String>,
    #[serde(default)]
    pub findings: Vec<WorkerReviewFindingV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerReviewVerdict {
    Approved,
    ChangesRequested,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerReviewFindingV1 {
    pub severity: WorkerReviewSeverity,
    pub detail: String,
    #[serde(default)]
    pub requirement_id: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    /// 仅为派发建议；宿主必须校验其为原生产者且不扩大写范围。
    #[serde(default)]
    pub suggested_owner: Option<String>,
    /// 精确指向宿主绑定的被审任务；同一 Worker 承担多个任务时必须提供。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_task_id: Option<String>,
    /// 可选的被审产物身份；若 task 与 artifact 都提供，二者必须指向同一快照。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_artifact_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerReviewSeverity {
    Blocker,
    Major,
    Minor,
    Note,
}

impl WorkerReviewSeverity {
    /// Blocker and major findings always prevent approval across all delivery paths.
    pub fn blocks_approval(self) -> bool {
        matches!(self, Self::Blocker | Self::Major)
    }
}

/// Worker 结构化输出（V1 契约本体）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutputV1 {
    /// 终态声明。
    pub status: WorkerOutputStatus,
    /// 一句话结论（reviewer 的简要说明；结构化结论使用 review_result）。
    pub summary: String,
    /// 交付物（Producer 类角色必填；critic 禁止提交）。
    #[serde(default)]
    pub artifact: Option<WorkerArtifactV1>,
    /// 证据引用链。
    #[serde(default)]
    pub evidence: Vec<WorkerEvidenceV1>,
    /// 未解决问题 / 风险（如实上报，不做无据通过）。
    #[serde(default)]
    pub open_issues: Vec<String>,
    /// 给下游角色的交接说明（可选）。
    #[serde(default)]
    pub handoff: Option<String>,
    /// review capability 的结构化结论；被审查快照由宿主绑定。
    #[serde(default)]
    pub review_result: Option<WorkerReviewResultV1>,
}

impl WorkerOutputV1 {
    /// Producer 类角色的合法输出校验：status=done 时必须携带非空 artifact.content。
    pub fn validate(&self) -> Result<(), String> {
        match self.status {
            WorkerOutputStatus::Done => {}
            WorkerOutputStatus::Failed | WorkerOutputStatus::Blocked => {
                if self.summary.trim().is_empty() && self.open_issues.is_empty() {
                    return Err(format!(
                        "status={} 必须在 summary 或 open_issues 中说明原因",
                        self.status.as_str()
                    ));
                }
                return Ok(());
            }
        }
        if self.summary.trim().is_empty() {
            return Err("summary 不能为空".to_string());
        }
        if !self.open_issues.is_empty() {
            return Err(format!(
                "status=done 仍有未解决问题，不能提交为候选完成：{}",
                self.open_issues.join("；")
            ));
        }
        match &self.artifact {
            Some(artifact) => {
                if artifact.content.trim().is_empty() {
                    return Err("artifact.content 不能为空（交付物正文缺失）".to_string());
                }
                if artifact.kind.trim().is_empty() {
                    return Err("artifact.kind 不能为空".to_string());
                }
                let format = artifact.format.trim().to_ascii_lowercase();
                if !matches!(format.as_str(), "text" | "markdown" | "json" | "csv") {
                    return Err(format!(
                        "artifact.format「{format}」不在 text|markdown|json|csv 内"
                    ));
                }
                Ok(())
            }
            None => Err("producer 必须提交 artifact（结构化交付物）".to_string()),
        }
    }

    /// Normalize the common reviewer mistake of putting a JSON scorecard in
    /// `artifact` (the critic contract forbids it). The scorecard is kept as
    /// evidence in summary, the artifact is dropped, and a decision/verdict field
    /// is converted into `review_result`. Ambiguous payloads still fail
    /// `validate_critic`, so this cannot turn a non-review into an approval.
    pub fn normalize_critic_artifact(&mut self) {
        let Some(artifact) = self.artifact.take() else {
            return;
        };
        let content = artifact.content;
        if self.summary.trim().is_empty() {
            self.summary = content.clone();
        }
        if self.review_result.is_some() {
            return;
        }
        let Some(start) = content.find('{') else {
            return;
        };
        let Some(end) = content.rfind('}') else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content[start..=end]) else {
            return;
        };
        if let Some(result) = value.get("review_result") {
            if let Ok(result) = serde_json::from_value(result.clone()) {
                self.review_result = Some(result);
                return;
            }
        }
        let Some(raw) = value
            .get("decision")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.get("verdict").and_then(serde_json::Value::as_str))
        else {
            return;
        };
        let verdict = match raw.trim().to_ascii_lowercase().as_str() {
            "ship" | "approved" | "approve" | "pass" | "通过" => WorkerReviewVerdict::Approved,
            "revise" | "changes_requested" | "changes-requested" | "changes requested" => {
                WorkerReviewVerdict::ChangesRequested
            }
            "block" | "rejected" | "reject" | "blocked" => WorkerReviewVerdict::Rejected,
            _ => return,
        };
        let mut findings = Vec::new();
        for (key, severity) in [
            ("blockers", WorkerReviewSeverity::Blocker),
            ("major", WorkerReviewSeverity::Major),
        ] {
            for detail in value
                .get(key)
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .filter(|detail| !detail.trim().is_empty())
            {
                findings.push(WorkerReviewFindingV1 {
                    severity,
                    detail: detail.to_string(),
                    requirement_id: None,
                    evidence_refs: Vec::new(),
                    suggested_owner: None,
                    target_task_id: None,
                    target_artifact_id: None,
                });
            }
        }
        self.review_result = Some(WorkerReviewResultV1 {
            verdict,
            reviewed_requirement_ids: Vec::new(),
            findings,
        });
    }

    /// Critic 输出校验：只提交评审结论，禁止携带 artifact。
    /// Reviewers also sometimes mark an approved review as status=blocked.
    /// The verdict is the structured decision; promote an explicit approval to
    /// done, but never infer approval from a negative/ambiguous summary.
    pub fn normalize_reviewer_verdict(&mut self) {
        if self.review_result.is_some() {
            self.status = WorkerOutputStatus::Done;
            return;
        }
        if self.status == WorkerOutputStatus::Done {
            return;
        }
        let summary = self.summary.to_ascii_lowercase();
        let negative = summary.contains("not approved")
            || summary.contains("changes_requested")
            || summary.contains("changes requested")
            || summary.contains("rejected")
            || summary.contains("blocked");
        let approved = !negative
            && (summary.contains("approved")
                || summary.contains("ship")
                || summary.contains("通过"));
        if approved {
            self.review_result = Some(WorkerReviewResultV1 {
                verdict: WorkerReviewVerdict::Approved,
                reviewed_requirement_ids: Vec::new(),
                findings: Vec::new(),
            });
            self.status = WorkerOutputStatus::Done;
        }
    }

    pub fn validate_critic(&self) -> Result<(), String> {
        if self.artifact.is_some() {
            return Err(
                "critic 不得提交 artifact（评审结论放 summary；交付物归 producer 链）".to_string(),
            );
        }
        if self.summary.trim().is_empty() {
            return Err("critic summary（评审结论）不能为空".to_string());
        }
        if self.status == WorkerOutputStatus::Done {
            let review = self.review_result.as_ref().ok_or_else(|| {
                "status=done 的 reviewer 必须提交结构化 review_result".to_string()
            })?;
            for finding in &review.findings {
                if finding.detail.trim().is_empty() {
                    return Err("review finding.detail 不能为空".to_string());
                }
            }
            if review.verdict == WorkerReviewVerdict::Approved
                && review
                    .findings
                    .iter()
                    .any(|finding| finding.severity.blocks_approval())
            {
                return Err("存在 blocker/major finding 时 verdict 不得为 approved".to_string());
            }
        }
        Ok(())
    }
}

/// 契约解析结果三态。
///
/// `Parsed` 装箱：`WorkerOutputV1` 有 200+ 字节，直接内联会让每次
/// `parse_worker_output` 的返回与匹配都搬动整个结构（clippy::large_enum_variant）。
#[derive(Debug, Clone)]
pub enum WorkerOutputParse {
    /// 完全符合契约。
    Parsed(Box<WorkerOutputV1>),
    /// 非契约输出（纯文本 legacy 语义，登记行为不变）。
    Legacy,
    /// 看起来想按契约输出但解析/校验失败（触发一次定向修复）。
    Invalid { error: String },
}

/// 解析 worker 原始输出为契约三态。
///
/// 判定规则：JSON 对象且含 `status` 字段 → 走契约（结构解析失败是
/// `Invalid`，可修复）；其余一律 `Legacy`（服务端旧流程不受影响）。
/// **角色规则不在此处判定**——producer 必须带 artifact / critic 禁止带
/// artifact 由调用方按角色调用 [`WorkerOutputV1::validate`] /
/// [`WorkerOutputV1::validate_critic`]（否则 critic 的合法输出会被误判）。
pub fn parse_worker_output(text: &str) -> WorkerOutputParse {
    // 模型常把合法 JSON 包在 ```json 围栏中。围栏不改变契约语义，解析前先做
    // 确定性归一化，避免触发一次昂贵的模型修复调用。
    let normalized = strip_code_fences(text);
    let trimmed = normalized.trim();
    // 快速排除：非 `{` 开头不可能是契约 JSON（自由文本直接 Legacy）。
    if !trimmed.starts_with('{') {
        return WorkerOutputParse::Legacy;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        // 以 { 开头但非法 JSON：多半是模型想按契约输出但格式坏了。
        return WorkerOutputParse::Invalid {
            error: "输出以 { 开头但不是合法 JSON".to_string(),
        };
    };
    if value.get("status").is_none() {
        // 有结构但不是本契约（其他 JSON 用途）→ Legacy。
        return WorkerOutputParse::Legacy;
    }
    // producer 常把 `review_result` 写成一句自检说明字符串。该字段只对 reviewer
    // 有意义，非对象值不应把一份合法交付物判成契约非法（会白白浪费一次定点修复
    // 调用，长任务里实测让 implementer 整步失败）。此处剥离畸形值，角色校验再决定
    // 是否需要结构化 review_result（critic 缺失会被 validate_critic 明确拒绝）。
    let mut value = value;
    if value
        .get("review_result")
        .is_some_and(|review| !review.is_object() && !review.is_null())
    {
        if let Some(object) = value.as_object_mut() {
            object.remove("review_result");
        }
    }
    // `evidence` / `open_issues` 是可选字段；模型常把 evidence 写成一段说明字符串，
    // 或把 open_issues 写成单个字符串。可选字段的常见简写不应让整份交付物失败
    // （真实团队 lead 实测：TaskGraph 内容正确，仅 evidence 是字符串，被契约修复
    // 一次后仍失败）。保留语义做确定性归一化，必需字段与角色规则仍由后续校验把关。
    normalize_optional_contract_fields(&mut value);
    normalize_review_verdict_aliases(&mut value);
    match serde_json::from_value::<WorkerOutputV1>(value) {
        Ok(output) => WorkerOutputParse::Parsed(Box::new(output)),
        Err(error) => WorkerOutputParse::Invalid {
            error: format!("WorkerOutputV1 字段校验失败：{error}"),
        },
    }
}

/// 归一化可选字段的常见简写，避免模型的小偏差触发整份契约失败。
///
/// - `evidence: "..."` → `[{"source":"..."}]`；空字符串 → `[]`
/// - `evidence: [ "...", {...} ]` → 字符串项转成 `{source}`
/// - `open_issues: "..."` → `["..."]`；空字符串 → `[]`
///
/// 只处理明确的简写；未知对象/数组结构仍交给 serde 报错，避免掩盖真正不合法的输出。
fn normalize_optional_contract_fields(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    // WorkerOutputV1 has a deliberately small terminal vocabulary. Models
    // sometimes use workflow-review words such as needs_revision. Map those to
    // blocked (the producer path still accepts a blocked output with an artifact
    // and records its notes), while keeping unknown statuses rejected.
    let raw_status = object
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    if let Some(status) = raw_status {
        let normalized = match status.trim().to_ascii_lowercase().as_str() {
            "needs_revision" | "needs_revisions" | "revision" | "changes_requested" => {
                Some("blocked")
            }
            "completed" | "complete" | "success" | "succeeded" => Some("done"),
            "error" => Some("failed"),
            _ => None,
        };
        if let Some(normalized) = normalized {
            object.insert(
                "status".to_string(),
                serde_json::Value::String(normalized.to_string()),
            );
        }
    }
    // Some models return `artifact.content` as the TaskGraph object instead of the
    // required JSON string. The semantic content is unchanged; serialize it once so
    // a valid plan is not lost to a type-only contract failure.
    if let Some(artifact) = object
        .get_mut("artifact")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(content) = artifact.get_mut("content") {
            if content.is_object() || content.is_array() {
                if let Ok(serialized) = serde_json::to_string(content) {
                    *content = serde_json::Value::String(serialized);
                }
            }
        }
    }
    // A TaskGraph is a planning artifact. Models often put unresolved planning
    // questions in open_issues while still returning a complete graph; those are
    // notes for downstream workers, not a failed deliverable. Preserve them in
    // handoff so the team can proceed without weakening open_issues for code or
    // test artifacts.
    let task_graph_artifact = object
        .get("artifact")
        .and_then(serde_json::Value::as_object)
        .and_then(|artifact| artifact.get("content"))
        .and_then(serde_json::Value::as_str)
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .is_some_and(|graph| {
            graph
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .is_some()
        });
    if task_graph_artifact
        && object.get("status").and_then(serde_json::Value::as_str) == Some("done")
    {
        if let Some(issues) = object.remove("open_issues") {
            let notes = match issues {
                serde_json::Value::String(text) => text,
                serde_json::Value::Array(items) => items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join("；"),
                other => other.to_string(),
            };
            if !notes.trim().is_empty() {
                let previous = object
                    .remove("handoff")
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default();
                let separator = if previous.trim().is_empty() { "" } else { "\n" };
                object.insert(
                    "handoff".to_string(),
                    serde_json::Value::String(format!(
                        "{previous}{separator}架构/计划阶段待确认事项（非阻塞）：{notes}"
                    )),
                );
            }
        }
    }
    // A producer that still has open issues is not a completed candidate. Keep
    // the artifact and issues but downgrade the terminal status to blocked; the
    // role layer can then preserve the candidate and surface the notes instead
    // of discarding the whole plan on a wording/contract mismatch.
    let has_issues = object
        .get("open_issues")
        .is_some_and(|issues| match issues {
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::String(text) => !text.trim().is_empty(),
            serde_json::Value::Null => false,
            _ => true,
        });
    if object.get("status").and_then(serde_json::Value::as_str) == Some("done")
        && has_issues
        && object.get("artifact").is_some()
    {
        object.insert(
            "status".to_string(),
            serde_json::Value::String("blocked".to_string()),
        );
    }
    // Some models put a brief or structured handoff object in a string field.
    // Keep the information but satisfy the wire contract.
    for key in ["summary", "handoff"] {
        if let Some(field) = object.get_mut(key) {
            if field.is_object() || field.is_array() {
                if let Ok(serialized) = serde_json::to_string(field) {
                    *field = serde_json::Value::String(serialized);
                }
            }
        }
    }
    if let Some(evidence) = object.remove("evidence") {
        let normalized = match evidence {
            serde_json::Value::String(source) => {
                let source = source.trim().to_string();
                if source.is_empty() {
                    serde_json::json!([])
                } else {
                    serde_json::json!([{ "source": source }])
                }
            }
            serde_json::Value::Null => serde_json::json!([]),
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.into_iter().map(normalize_evidence_item).collect())
            }
            serde_json::Value::Object(item) => {
                serde_json::Value::Array(vec![normalize_evidence_item(serde_json::Value::Object(
                    item,
                ))])
            }
            other => other,
        };
        object.insert("evidence".to_string(), normalized);
    }
    if let Some(open_issues) = object.remove("open_issues") {
        let normalized = match open_issues {
            serde_json::Value::String(issue) => {
                let issue = issue.trim().to_string();
                if issue.is_empty() {
                    serde_json::json!([])
                } else {
                    serde_json::json!([issue])
                }
            }
            serde_json::Value::Null => serde_json::json!([]),
            other => other,
        };
        object.insert("open_issues".to_string(), normalized);
    }
}

/// Evidence objects must carry a `source`. Small models often emit
/// `{"path":"hello.py","note":"..."}` or `{"type":"file","description":"..."}`;
/// derive `source` from a known locator field and keep the original fields.
fn normalize_evidence_item(item: serde_json::Value) -> serde_json::Value {
    match item {
        serde_json::Value::String(source) => serde_json::json!({ "source": source }),
        serde_json::Value::Object(mut object) => {
            if object.get("source").is_none() {
                let source = [
                    "path",
                    "file",
                    "ref",
                    "uri",
                    "description",
                    "detail",
                    "note",
                ]
                .iter()
                .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| serde_json::Value::Object(object.clone()).to_string());
                object.insert("source".to_string(), serde_json::Value::String(source));
            }
            serde_json::Value::Object(object)
        }
        other => other,
    }
}

/// `artifact.format` 合法枚举（九期一路冻结口径：契约提示与修复提示必须逐字传达）。
pub const ARTIFACT_FORMAT_WHITELIST: &str = "text|markdown|json|csv";

/// 契约的系统提示片段（执行器拼进 Worker system prompt；schema 即机器可校验规则）。
pub fn contract_system_prompt(is_critic: bool) -> String {
    let mut prompt = String::from(
        "## 输出契约（必须严格遵守）\n你的最终回复**必须且只能是**一个合法 JSON 对象（WorkerOutputV1），\
不要输出任何解释、代码围栏或契约之外的内容。字段：\n\
{\n  \"status\": \"done\" | \"failed\" | \"blocked\",\n  \"summary\": \"一句话结论\",\n\
  \"artifact\": {\"kind\": \"产物分类\", \"format\": \"text|markdown|json|csv\", \"content\": \"产物正文本体\"},\n\
  \"evidence\": [{\"source\": \"来源\", \"note\": \"说明\"}],\n\
  \"open_issues\": [\"未解决问题\"],\n  \"handoff\": \"给下游的交接说明（可省略）\"\n}\n\
review_result（reviewer必填）：对象含 verdict（approved/changes_requested/rejected）、reviewed_requirement_ids（从宿主评审清单逐项原样复制且不得重复）与 findings 列表；finding 含 severity（blocker/major/minor/note）、detail、requirement_id、evidence_refs、suggested_owner、target_task_id、target_artifact_id；同一 owner 有多个被审任务时必须从上游上下文的宿主 Artifact 身份复制 target_task_id 或 target_artifact_id 精确定位；其他角色省略。\nartifact.format 只能取 text|markdown|json|csv 之一（大小写敏感，用小写）。\n",
    );
    if is_critic {
        prompt.push_str(
            "你是评审角色（review capability）：**禁止**提交 artifact；status=done 时必须提交 review_result，\
包含 verdict、reviewed_requirement_ids（从宿主清单逐项原样复制且不得重复）和 findings（severity/detail/requirement_id/evidence_refs/suggested_owner/target_task_id/target_artifact_id）；owner 有多个被审任务时必须精确定位。\
只报告有证据的问题；approved 不得包含 blocker。summary 可写简要结论。被审查产物由宿主绑定，\
不要自行编造哈希或身份。交付物归 producer 链，评审无权覆盖。\n\
评审维度（每项都必须给出有证据的结论；缺证据视为未通过）：①需求覆盖；②行为正确性；③验证命令/测试证据；④写入范围与安全边界；⑤交付完整性。approved 前确认五个维度均无 blocker/major。\n\
发现缺陷时用 status=done + review_result.verdict=changes_requested 表达“需要返修”，**禁止**用 status=blocked/failed 表达评审不通过；只有完全无法读取被审产物时才用 blocked。\n\
findings 每项必须是包含 **severity** 与 **detail** 的对象（缺 severity 会直接判契约失败），最小示例：{\"severity\":\"major\",\"detail\":\"test_hello.py 未导入 greet\",\"requirement_id\":\"<宿主清单中的 id>\",\"evidence_refs\":[\"read_file: test_hello.py\"]}。\n",
        );
    } else {
        prompt.push_str(
            "你是交付角色：**不得输出 review_result**（该字段仅供 reviewer 使用），\
status=done 时**必须**提交 artifact（content=交付物正文本体，\
不是你的过程描述）。代码分析与补丁/变更报告的 artifact.format 一律用 \"markdown\"、\
kind 分别用 \"analysis\"/\"code\"。交付物内容必须**逐字满足**任务指令的字面要求（如指定必须出现的\
映射行、签名行、字段值与文件路径）；若无法完成，用 status=failed/blocked 并在 \
summary/open_issues 说明原因。\n\
回合预算纪律：最后一个回合（只剩 1 次调用时）**禁止再调用任何工具**，必须直接输出完整契约 JSON——\
把已取得的发现写进 artifact.content/summary，宁可内容不完美也不要因超预算失去输出机会。\n",
        );
    }
    prompt
}

/// 剥掉模型输出常见的代码围栏（``` 包裹），只处理首尾成对的一对（七期一路自
/// `product_eval::workswarm_executor` 私有函数移入，成为两路共享工具）。
/// Normalize common review-verdict spellings before serde enum parsing.
///
/// Small local models frequently emit `"verdict":"pass"` or `"lgtm"` even
/// after a repair prompt. These are semantically unambiguous review decisions;
/// rejecting the whole artifact for a spelling variant wastes a model call and
/// turns a valid review into a failed step.
fn normalize_review_verdict_aliases(value: &mut serde_json::Value) {
    let Some(verdict) = value
        .get_mut("review_result")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|review| review.get_mut("verdict"))
        .and_then(|verdict| verdict.as_str())
        .map(str::to_string)
    else {
        return;
    };
    let normalized = match verdict
        .trim()
        .to_ascii_lowercase()
        .replace(['-', ' '], "_")
        .as_str()
    {
        "approved" | "approve" | "pass" | "passed" | "ok" | "lgtm" | "accept" | "accepted"
        | "yes" | "通过" | "合格" | "同意" | "批准" => "approved",
        "changes_requested" | "change_requested" | "request_changes" | "needs_changes"
        | "needs_revision" | "needs_fix" | "must_fix" | "revise" | "rework" | "blocker"
        | "blocked" | "blocking" | "not_approved" | "修改" | "返修" | "需修改" | "阻断" => {
            "changes_requested"
        }
        "rejected" | "reject" | "deny" | "denied" | "fail" | "failed" | "不通过" | "拒绝" => {
            "rejected"
        }
        _ => return,
    };
    if let Some(review) = value
        .get_mut("review_result")
        .and_then(serde_json::Value::as_object_mut)
    {
        review.insert(
            "verdict".to_string(),
            serde_json::Value::String(normalized.to_string()),
        );
    }
}

pub fn strip_code_fences(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    // 跳过语言标记行（```json / ```JSON5 等）。
    let body = match rest.find('\n') {
        Some(idx) => &rest[idx + 1..],
        None => rest,
    };
    body.trim()
        .strip_suffix("```")
        .map(str::trim)
        .map(str::to_string)
        .unwrap_or_else(|| body.trim().to_string())
}

/// 输出契约定向修复提示词（七期一路）：角色规则 + 具体违例原因 + “只输出 JSON 本体”要求。
///
/// 九期（一路）：修复提示必须**逐字传达** `artifact.format` 白名单（text|markdown|json|csv）、
/// 代码分析/补丁报告默认 markdown、critic/reviewer 禁止输出最终 Artifact——八期冒烟中
/// 修复提示只回显了违例原因，模型第二次仍输出白名单外格式（如 "md"/"Markdown"）。
pub fn contract_repair_prompt(is_critic: bool, violation: &str, broken: &str) -> String {
    let role_rule = if is_critic {
        "你是 review capability：禁止 artifact；status=done 必须提交 review_result={verdict,reviewed_requirement_ids,findings}，reviewed_requirement_ids 必须逐项复制宿主清单且不得重复，findings 每项必须含 severity（blocker/major/minor/note）和 detail，只报告有证据的问题；同一 owner 多任务时从上游上下文的宿主 Artifact 身份原样复制 target_task_id 或 target_artifact_id 精确定位；宿主绑定评审快照"
    } else {
        "你是交付角色：不得输出 review_result；status=done 必须携带 artifact（content=交付物正文本体；artifact.format 只能取 text|markdown|json|csv 之一，代码分析与补丁/变更报告一律用 \"markdown\"，kind 用 \"analysis\"/\"code\"）"
    };
    format!(
        "你的上一次回复不符合 WorkerOutputV1 输出契约（必须是合法 JSON：status/summary/artifact{{kind,format,content}}/evidence/open_issues/handoff/review_result）。\
artifact.format 的合法枚举只有 text|markdown|json|csv——不要输出 \"md\"、\"Markdown\"、\"plaintext\" 等白名单外写法；\
评审/审查角色（critic/reviewer）禁止输出最终 Artifact；同一 owner 多任务时 finding 必须从上游上下文宿主 Artifact 身份复制 target_task_id 或 target_artifact_id。具体违例：{violation}。{role_rule}。请修正后**只输出**契约合规的 JSON 本体。\n原输出：\n{broken}\n\n请重新输出契约合规的 JSON："
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn producer_json(content: &str) -> String {
        format!(
            r#"{{"status":"done","summary":"完成","artifact":{{"kind":"document","format":"markdown","content":"{content}"}},"evidence":[],"open_issues":[]}}"#
        )
    }

    #[test]
    fn parses_valid_producer_output() {
        let parsed = parse_worker_output(&producer_json("# 交付 正文"));
        match parsed {
            WorkerOutputParse::Parsed(output) => {
                assert_eq!(output.status, WorkerOutputStatus::Done);
                assert_eq!(output.artifact.as_ref().unwrap().format, "markdown");
                assert!(output.validate().is_ok());
            }
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn parses_valid_producer_output_inside_markdown_fence_without_repair() {
        let fenced = format!("```json\n{}\n```", producer_json("# 交付 正文"));
        match parse_worker_output(&fenced) {
            WorkerOutputParse::Parsed(output) => assert!(output.validate().is_ok()),
            other => panic!("围栏内的合法契约应直接解析，不触发模型修复：{other:?}"),
        }
    }

    #[test]
    fn producer_stray_review_result_string_is_ignored_not_invalid() {
        // 长任务实测：implementer 把 `review_result` 写成一句自检说明字符串，
        // 旧行为 serde 直接判整份交付物非法并触发一次浪费的定点修复；现在剥离
        // 畸形值，由角色校验决定是否需要结构化评审结果。
        let output = serde_json::json!({
            "status": "done",
            "summary": "修复完成",
            "artifact": {"kind": "code", "format": "markdown", "content": "# 补丁"},
            "evidence": [],
            "open_issues": [],
            "review_result": "自检通过（这不是结构化对象）",
        });
        match parse_worker_output(&output.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert!(
                    parsed.review_result.is_none(),
                    "畸形 review_result 必须被剥离"
                );
                assert!(parsed.validate().is_ok(), "合法交付物不应因无关字段失败");
            }
            other => panic!("应为 Parsed：{other:?}"),
        }
        // critic 的字符串 review_result 同样先被剥离，随后由 validate_critic 明确拒绝。
        let critic = serde_json::json!({
            "status": "done",
            "summary": "评审",
            "evidence": [],
            "open_issues": [],
            "review_result": "approved",
        });
        match parse_worker_output(&critic.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert!(parsed.review_result.is_none());
                assert!(parsed.validate_critic().is_err(), "缺少结构化评审必须拒绝");
            }
            other => panic!("应为 Parsed（由角色校验拒绝）：{other:?}"),
        }
    }

    #[test]
    fn artifact_content_object_is_serialized_to_the_required_string_shape() {
        let output = parse_worker_output(
            r#"{"status":"done","summary":"TaskGraph","artifact":{"kind":"task-graph","format":"json","content":{"version":1,"tasks":[]}},"evidence":[],"open_issues":[]}"#,
        );
        match output {
            WorkerOutputParse::Parsed(parsed) => {
                let content = parsed.artifact.as_ref().unwrap().content.as_str();
                let parsed_content: serde_json::Value =
                    serde_json::from_str(content).expect("content must be valid JSON text");
                assert_eq!(parsed_content["version"], 1);
                assert_eq!(parsed_content["tasks"], serde_json::json!([]));
            }
            _ => panic!("expected parsed output"),
        }
    }

    #[test]
    fn done_with_open_issues_and_artifact_becomes_blocked_candidate() {
        let output = parse_worker_output(
            r#"{"status":"done","summary":"???","artifact":{"kind":"code","format":"text","content":"partial"},"evidence":[],"open_issues":["????"]}"#,
        );
        match output {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(parsed.status, WorkerOutputStatus::Blocked);
                assert!(!parsed.open_issues.is_empty());
            }
            _ => panic!("expected parsed output"),
        }
    }

    #[test]
    fn needs_revision_status_is_normalized_to_blocked() {
        let output = parse_worker_output(
            r#"{"status":"needs_revision","summary":"????","artifact":{"kind":"task-graph","format":"json","content":"{}"},"evidence":[],"open_issues":[]}"#,
        );
        match output {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(parsed.status, WorkerOutputStatus::Blocked);
            }
            _ => panic!("expected parsed output"),
        }
    }

    #[test]
    fn structured_summary_is_serialized_to_string() {
        let output = parse_worker_output(
            r#"{"status":"done","summary":{"brief":"坦克大战"},"artifact":{"kind":"task-graph","format":"json","content":"{}"},"evidence":[],"open_issues":[]}"#,
        );
        match output {
            WorkerOutputParse::Parsed(parsed) => {
                assert!(parsed.summary.contains("坦克大战"), "{}", parsed.summary);
                assert!(parsed.validate().is_ok());
            }
            _ => panic!("expected parsed output"),
        }
    }

    #[test]
    fn task_graph_open_issues_become_non_blocking_handoff_notes() {
        let output = parse_worker_output(
            r#"{"status":"done","summary":"TaskGraph","artifact":{"kind":"task-graph","format":"json","content":{"version":1,"tasks":[]}},"evidence":[],"open_issues":["???????????"]}"#,
        );
        match output {
            WorkerOutputParse::Parsed(parsed) => {
                assert!(parsed.open_issues.is_empty());
                assert!(parsed.handoff.as_deref().unwrap_or("").contains("???"));
                assert!(parsed.validate().is_ok());
            }
            _ => panic!("expected parsed output"),
        }
    }

    #[test]
    fn optional_evidence_and_open_issues_strings_are_normalized() {
        let output = serde_json::json!({
            "status": "done",
            "summary": "完成",
            "artifact": {"kind": "json", "format": "json", "content": "{\"version\":1,\"tasks\":[]}"},
            "evidence": "read_file: src/main.js",
            "open_issues": "",
        });
        match parse_worker_output(&output.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(parsed.evidence.len(), 1);
                assert_eq!(parsed.evidence[0].source, "read_file: src/main.js");
                assert!(parsed.open_issues.is_empty());
                assert!(parsed.validate().is_ok());
            }
            other => panic!("字符串简写应归一化为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn non_empty_open_issues_string_downgrades_done_to_blocked() {
        let output = serde_json::json!({
            "status": "done",
            "summary": "完成",
            "artifact": {"kind": "text", "format": "text", "content": "交付"},
            "evidence": [],
            "open_issues": "仍需人工确认",
        });
        match parse_worker_output(&output.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(parsed.open_issues, vec!["仍需人工确认"]);
                assert_eq!(
                    parsed.status,
                    WorkerOutputStatus::Blocked,
                    "带未解决问题的 done 必须降级为 blocked，不能被当成候选完成"
                );
                assert!(parsed.validate().is_ok());
            }
            other => panic!("字符串 open_issues 应归一化为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn optional_evidence_array_strings_are_normalized() {
        let output = serde_json::json!({
            "status": "done",
            "summary": "完成",
            "artifact": {"kind": "text", "format": "text", "content": "交付"},
            "evidence": ["文件 A", {"source": "文件 B", "note": "证据"}],
            "open_issues": [],
        });
        match parse_worker_output(&output.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(parsed.evidence.len(), 2);
                assert_eq!(parsed.evidence[0].source, "文件 A");
                assert_eq!(parsed.evidence[1].note.as_deref(), Some("证据"));
            }
            other => panic!("数组字符串项应归一化为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn evidence_objects_without_source_are_normalized() {
        let output = serde_json::json!({
            "status": "done",
            "summary": "read",
            "artifact": {"kind": "text", "format": "text", "content": "greet"},
            "evidence": [
                {"path": "hello.py", "note": "read function"},
                {"type": "file", "description": "test_hello.py"}
            ],
            "open_issues": [],
        });
        match parse_worker_output(&output.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(parsed.evidence.len(), 2);
                assert_eq!(parsed.evidence[0].source, "hello.py");
                assert_eq!(parsed.evidence[1].source, "test_hello.py");
            }
            other => panic!("缺 source 的 evidence 对象应派生 source：{other:?}"),
        }
    }

    #[test]
    fn review_verdict_aliases_are_normalized_before_parsing() {
        let pass = serde_json::json!({
            "status": "done",
            "summary": "looks good",
            "review_result": {"verdict": "pass", "reviewed_requirement_ids": [], "findings": []},
        });
        match parse_worker_output(&pass.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(
                    parsed.review_result.as_ref().unwrap().verdict,
                    WorkerReviewVerdict::Approved
                );
            }
            other => panic!("verdict=pass 应归一化为 approved：{other:?}"),
        }

        let revise = serde_json::json!({
            "status": "done",
            "summary": "needs work",
            "review_result": {
                "verdict": "changes-requested",
                "reviewed_requirement_ids": [],
                "findings": [{"severity": "major", "detail": "x"}]
            },
        });
        match parse_worker_output(&revise.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(
                    parsed.review_result.as_ref().unwrap().verdict,
                    WorkerReviewVerdict::ChangesRequested
                );
            }
            other => panic!("changes-requested 应归一化：{other:?}"),
        }

        let blocker = serde_json::json!({
            "status": "done",
            "summary": "blocked",
            "review_result": {
                "verdict": "blocker",
                "reviewed_requirement_ids": [],
                "findings": [{"severity": "blocker", "detail": "x"}]
            },
        });
        match parse_worker_output(&blocker.to_string()) {
            WorkerOutputParse::Parsed(parsed) => {
                assert_eq!(
                    parsed.review_result.as_ref().unwrap().verdict,
                    WorkerReviewVerdict::ChangesRequested
                );
            }
            other => panic!("verdict=blocker 应归一化为 changes_requested：{other:?}"),
        }
    }

    #[test]
    fn plain_text_is_legacy() {
        assert!(matches!(
            parse_worker_output("自由文本交付物"),
            WorkerOutputParse::Legacy
        ));
        // 非契约 JSON（其他用途）也是 Legacy。
        assert!(matches!(
            parse_worker_output(r#"{"approved":true,"score":88}"#),
            WorkerOutputParse::Legacy
        ));
    }

    #[test]
    fn broken_json_is_invalid_not_legacy() {
        match parse_worker_output("{\"status\":\"done\",\"summary\":\"x\", artifact:}") {
            WorkerOutputParse::Invalid { error } => assert!(error.contains("合法 JSON")),
            other => panic!("应为 Invalid：{other:?}"),
        }
    }

    #[test]
    fn producer_without_artifact_fails_role_validation() {
        // 结构解析通过（Parsed）；producer 角色规则由 validate() 判定。
        let parsed = parse_worker_output(r#"{"status":"done","summary":"只有文本"}"#);
        match parsed {
            WorkerOutputParse::Parsed(output) => {
                let error = output.validate().unwrap_err();
                assert!(error.contains("artifact"), "{error}");
            }
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn empty_artifact_content_fails_validation() {
        let parsed = parse_worker_output(
            r#"{"status":"done","summary":"x","artifact":{"kind":"document","format":"text","content":"  "}}"#,
        );
        match parsed {
            WorkerOutputParse::Parsed(output) => assert!(output.validate().is_err()),
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn bad_format_fails_validation() {
        let parsed = parse_worker_output(
            r#"{"status":"done","summary":"x","artifact":{"kind":"document","format":"xml","content":"y"}}"#,
        );
        match parsed {
            WorkerOutputParse::Parsed(output) => assert!(output.validate().is_err()),
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn critic_with_artifact_is_rejected() {
        let parsed = parse_worker_output(
            r#"{"status":"done","summary":"评审","artifact":{"kind":"review","format":"text","content":"越权交付"}}"#,
        );
        match parsed {
            WorkerOutputParse::Parsed(output) => {
                let error = output.validate_critic().unwrap_err();
                assert!(error.contains("不得提交 artifact"));
            }
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn reviewer_done_requires_structured_review_result() {
        let output = WorkerOutputV1 {
            status: WorkerOutputStatus::Done,
            summary: "看起来通过".to_string(),
            artifact: None,
            evidence: Vec::new(),
            open_issues: Vec::new(),
            handoff: None,
            review_result: None,
        };
        assert!(output
            .validate_critic()
            .unwrap_err()
            .contains("review_result"));
    }

    #[test]
    fn review_finding_accepts_exact_task_and_artifact_identity() {
        let output = parse_worker_output(
            r#"{"status":"done","summary":"发现缺陷","review_result":{"verdict":"changes_requested","findings":[{"severity":"major","detail":"边界条件错误","suggested_owner":"m-w1","target_task_id":"step-a","target_artifact_id":"artifact-a"}]}}"#,
        );
        let WorkerOutputParse::Parsed(output) = output else {
            panic!("包含宿主引用字段的 review 输出应能解析");
        };
        let review_result = output.review_result.unwrap();
        let finding = &review_result.findings[0];
        assert_eq!(finding.target_task_id.as_deref(), Some("step-a"));
        assert_eq!(finding.target_artifact_id.as_deref(), Some("artifact-a"));
    }

    #[test]
    fn blocker_and_major_severities_share_one_approval_barrier() {
        assert!(WorkerReviewSeverity::Blocker.blocks_approval());
        assert!(WorkerReviewSeverity::Major.blocks_approval());
        assert!(!WorkerReviewSeverity::Minor.blocks_approval());
        assert!(!WorkerReviewSeverity::Note.blocks_approval());
    }

    #[test]
    fn reviewer_cannot_approve_with_blocker_finding() {
        let output = WorkerOutputV1 {
            status: WorkerOutputStatus::Done,
            summary: "发现阻断问题".to_string(),
            artifact: None,
            evidence: Vec::new(),
            open_issues: Vec::new(),
            handoff: None,
            review_result: Some(WorkerReviewResultV1 {
                verdict: WorkerReviewVerdict::Approved,
                reviewed_requirement_ids: Vec::new(),
                findings: vec![WorkerReviewFindingV1 {
                    severity: WorkerReviewSeverity::Blocker,
                    detail: "行为验收缺失".to_string(),
                    requirement_id: None,
                    evidence_refs: Vec::new(),
                    suggested_owner: None,
                    target_task_id: None,
                    target_artifact_id: None,
                }],
            }),
        };
        assert!(output.validate_critic().unwrap_err().contains("blocker"));
    }

    #[test]
    fn reviewer_cannot_approve_with_major_finding() {
        let output = WorkerOutputV1 {
            status: WorkerOutputStatus::Done,
            summary: "发现主要问题".to_string(),
            artifact: None,
            evidence: Vec::new(),
            open_issues: Vec::new(),
            handoff: None,
            review_result: Some(WorkerReviewResultV1 {
                verdict: WorkerReviewVerdict::Approved,
                reviewed_requirement_ids: Vec::new(),
                findings: vec![WorkerReviewFindingV1 {
                    severity: WorkerReviewSeverity::Major,
                    detail: "核心验收行为没有覆盖".to_string(),
                    requirement_id: Some("pagination-boundary".to_string()),
                    evidence_refs: vec!["src/pagination.rs".to_string()],
                    suggested_owner: None,
                    target_task_id: None,
                    target_artifact_id: None,
                }],
            }),
        };
        assert!(output.validate_critic().unwrap_err().contains("major"));
    }

    #[test]
    fn critic_failure_is_preserved_as_failure() {
        let parsed = parse_worker_output(
            r#"{"status":"blocked","summary":"缺少被测版本的可读快照","open_issues":["目标产物不可访问"]}"#,
        );
        match parsed {
            WorkerOutputParse::Parsed(output) => {
                output
                    .validate_critic()
                    .expect("失败状态可作为候选失败结果提交");
                assert_eq!(output.status, WorkerOutputStatus::Blocked);
            }
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn failed_status_needs_reason() {
        // 角色规则在 validate()：failed/blocked 必须在 summary 或 open_issues 给出原因。
        let parsed = parse_worker_output(r#"{"status":"failed","summary":"","open_issues":[]}"#);
        match parsed {
            WorkerOutputParse::Parsed(output) => assert!(output.validate().is_err()),
            other => panic!("应为 Parsed：{other:?}"),
        }
        let parsed =
            parse_worker_output(r#"{"status":"failed","summary":"","open_issues":["上游缺失"]}"#);
        match parsed {
            WorkerOutputParse::Parsed(output) => assert!(output.validate().is_ok()),
            other => panic!("应为 Parsed：{other:?}"),
        }
    }

    #[test]
    fn contract_prompt_covers_role_rules() {
        let producer = contract_system_prompt(false);
        assert!(producer.contains("必须") && producer.contains("artifact"));
        let critic = contract_system_prompt(true);
        assert!(critic.contains("禁止") && critic.contains("summary"));
        for dimension in [
            "需求覆盖",
            "行为正确性",
            "测试证据",
            "安全边界",
            "交付完整性",
        ] {
            assert!(
                critic.contains(dimension),
                "评审契约缺维度 {dimension}: {critic}"
            );
        }
    }

    // 九期（一路）：契约提示与修复提示必须传达 format 白名单/默认值/critic 禁令/
    // 末回合禁工具——八期冒烟失败模式（analyzer 白名单外 format、修复提示未传达
    // 枚举）的回归锁。
    #[test]
    fn system_prompt_states_format_whitelist_and_defaults() {
        let producer = contract_system_prompt(false);
        assert!(producer.contains("text|markdown|json|csv"));
        assert!(
            producer.contains("markdown"),
            "代码分析/补丁报告默认 markdown"
        );
        assert!(producer.contains("最后一个回合"), "末回合禁工具纪律");
        assert!(
            !producer.contains("禁止**提交最终 Artifact"),
            "producer 允许提交 artifact"
        );
        let critic = contract_system_prompt(true);
        assert!(critic.contains("禁止**提交 artifact"));
        assert!(critic.contains("review capability"));
        assert!(
            critic.contains("status=done + review_result.verdict=changes_requested"),
            "评审不通过必须走 changes_requested，而不是 blocked/failed"
        );
        assert!(critic.contains("severity"), "finding 必须声明 severity");
        assert!(
            critic.contains("\"severity\":\"major\""),
            "需要最小 finding 示例"
        );
    }

    #[test]
    fn repair_prompt_states_whitelist_default_and_critic_rule() {
        let producer = contract_repair_prompt(false, "format 不合法", "{\"status\":\"done\"}");
        assert!(producer.contains("text|markdown|json|csv"));
        assert!(producer.contains("markdown"));
        assert!(producer.contains("\"md\""), "点名白名单外常见写法");
        let critic = contract_repair_prompt(true, "携带 artifact", "{\"status\":\"done\"}");
        assert!(critic.contains("禁止 artifact"));
        assert!(critic.contains("review capability"));
        assert!(
            critic.contains("severity"),
            "修复提示必须点名 missing field severity"
        );
    }
    #[test]
    fn done_output_with_open_issues_is_not_accepted() {
        let output = WorkerOutputV1 {
            status: WorkerOutputStatus::Done,
            summary: "实现完成".to_string(),
            artifact: Some(WorkerArtifactV1 {
                kind: "code".to_string(),
                format: "text".to_string(),
                content: "候选交付".to_string(),
            }),
            evidence: Vec::new(),
            open_issues: vec!["验收失败".to_string()],
            handoff: None,
            review_result: None,
        };
        assert!(output.validate().unwrap_err().contains("未解决问题"));
    }
    #[test]
    fn blocked_reviewer_with_explicit_approval_becomes_done() {
        let mut output = WorkerOutputV1 {
            status: WorkerOutputStatus::Blocked,
            summary: "conclusion approved; no blocker/major".to_string(),
            artifact: None,
            evidence: Vec::new(),
            open_issues: Vec::new(),
            handoff: None,
            review_result: None,
        };
        output.normalize_reviewer_verdict();
        assert_eq!(output.status, WorkerOutputStatus::Done);
        assert_eq!(
            output.review_result.as_ref().map(|review| review.verdict),
            Some(WorkerReviewVerdict::Approved)
        );
        assert!(output.validate_critic().is_ok());
    }
}

//! Artifact 校验与交付管线（七期 · 第三路：Artifact 校验、证据链与下载交付）。
//!
//! 职责：
//! - **格式门控**（登记前执行）：`json` 必须可解析且不得裹 Markdown 围栏；
//!   `csv` 需表头 + 全行列数一致；`research` 需至少一条有效证据引用
//!   （文件引用或 http(s) URL）；`markdown` 拒绝 TBD/空模板占位交付；
//!   校验未通过的产物**不登记**（不进 CAS、不进版本链、不进 PendingReview）。
//! - **交付元数据**：format/media_type/file_name/sha256/size_bytes，随 Artifact 落盘，
//!   服务端据此提供下载（正确文件名 + media type）与交付清单。
//! - **证据链**：Worker 的 `evidence` 来源列表 → `Artifact.evidence_refs`，
//!   研究类交付可溯源；与 HandoffRecord 的证据引用链同源。
//!
//! 格式口径与 `workswarm_output`（第一路冻结契约）的衔接：
//! 契约枚举为 `text|markdown|json|csv`（不含 "research"），研究类交付由
//! 产物 `kind == "research"` 识别——声明格式为 text/markdown 时按 research 规则
//! （证据链）校验，见 [`effective_format`]。

use owo_agent_protocol::ArtifactValidation;

use crate::workswarm_output::WorkerEvidenceV1;

/// 校验结果（登记前门控判定，随 Artifact.validation 落盘）。
///
/// 直接返回协议类型 [`ArtifactValidation`]，服务端 metadata 端点原样透出。
pub fn validate_artifact_content(
    format: &str,
    content: &str,
    evidence: &[WorkerEvidenceV1],
) -> ArtifactValidation {
    let trimmed = content.trim();
    let norm = format.trim().to_ascii_lowercase();
    let result = match norm.as_str() {
        "json" => validate_json(trimmed),
        "csv" => validate_csv(trimmed),
        "research" => validate_research(trimmed, evidence),
        "markdown" | "text" => validate_markdown_or_text(trimmed, norm.as_str() == "markdown"),
        // 未知格式（旧记录/自由声明）：只要求非空，保持兼容。
        _ => {
            if trimmed.is_empty() {
                Err("产物内容为空".to_string())
            } else {
                Ok(())
            }
        }
    };
    match result {
        Ok(()) => ArtifactValidation {
            format: norm,
            valid: true,
            reason: None,
        },
        Err(reason) => ArtifactValidation {
            format: norm,
            valid: false,
            reason: Some(reason),
        },
    }
}

/// 有效校验格式（契约枚举与研究类交付的口径对齐）：
/// 声明 `research` 直接按 research 规则；声明 `text|markdown` 且产物
/// `kind == "research"`（研究类交付物）时改按 research 证据链规则；
/// 其余（含 kind 非 research 的 json/csv 声明）保持声明格式规则。
pub fn effective_format(declared_format: &str, kind: &str) -> String {
    let norm = declared_format.trim().to_ascii_lowercase();
    if norm == "research" || (kind == "research" && matches!(norm.as_str(), "text" | "markdown")) {
        return "research".to_string();
    }
    norm
}

/// Worker 证据 → `Artifact.evidence_refs`（带 note 时以「来源 (note)」呈现）。
pub fn evidence_refs_of(evidence: &[WorkerEvidenceV1]) -> Vec<String> {
    evidence
        .iter()
        .map(
            |e| match e.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                Some(note) => format!("{} ({})", e.source.trim(), note),
                None => e.source.trim().to_string(),
            },
        )
        .collect()
}

/// 判断证据引用是否有效（文件引用或 URL）：非空且为 http(s) URL、
/// 或含路径分隔符、或带扩展名标记（含 `.`）。
pub fn is_valid_evidence_ref(source: &str) -> bool {
    let s = source.trim();
    if s.is_empty() {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return true;
    }
    s.contains('/') || s.contains('\\') || s.contains('.')
}

/// 交付元数据：格式/媒体类型/文件名 + CAS 内容哈希与字节数 + 校验结果。
#[derive(Debug, Clone)]
pub struct DeliveryMeta {
    pub format: String,
    pub media_type: String,
    pub file_name: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub validation: ArtifactValidation,
}

/// 下载交付 media type 口径（按有效格式）。
pub fn media_type_of(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "json" => "application/json",
        "csv" => "text/csv",
        "markdown" | "research" => "text/markdown",
        _ => "text/plain",
    }
}

/// 下载交付扩展名（按有效格式）。
pub fn extension_of(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "json" => "json",
        "csv" => "csv",
        "markdown" | "research" => "md",
        _ => "txt",
    }
}

/// 交付文件名：由产物 kind 归一为安全文件名基 + 格式扩展名；
/// kind 归一后为空时回退 `artifact`。
pub fn file_name_of(kind: &str, format: &str) -> String {
    let base: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let base = if base.is_empty() {
        "artifact"
    } else {
        base.as_str()
    };
    format!("{base}.{}", extension_of(format))
}

/// 构建交付元数据（登记前门控 + 元数据一次性产出）。
///
/// `format` 传 [`effective_format`] 的结果；`kind` 用于推导文件名；
/// `evidence` 参与 research 证据链判定；sha256 走 CAS 同款 [`crate::cas_store::CasStore::hash_of`]。
pub fn build_delivery_meta(
    format: &str,
    kind: &str,
    content: &str,
    evidence: &[WorkerEvidenceV1],
) -> DeliveryMeta {
    let validation = validate_artifact_content(format, content, evidence);
    let bytes = content.as_bytes();
    DeliveryMeta {
        format: format.to_string(),
        media_type: media_type_of(format).to_string(),
        file_name: file_name_of(kind, format),
        sha256: crate::cas_store::CasStore::hash_of(bytes),
        size_bytes: bytes.len() as u64,
        validation,
    }
}

// ---------------------------------------------------------------------------
// 各格式校验规则
// ---------------------------------------------------------------------------

/// JSON：拒绝 Markdown 围栏包裹（围栏剥离属输出契约层的事，交付层要求干净 JSON 体），
/// 必须可解析为任意 JSON 值。
fn validate_json(trimmed: &str) -> Result<(), String> {
    if trimmed.is_empty() {
        return Err("JSON 产物内容不能为空".to_string());
    }
    if trimmed.starts_with("```") {
        return Err("JSON 产物不得裹 Markdown 代码围栏（直接输出 JSON 本体）".to_string());
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(_) => Ok(()),
        Err(strict) => match crate::workswarm_output::parse_first_json_value(trimmed) {
            // 模型常在 JSON 本体后追加一段说明（serde 报 "trailing characters"）。
            // 登记门只要求开头是一个可解析的 JSON 值；产物是否满足任务要求仍由
            // 任务检查器判定，这里不替它放行内容。
            Ok(_) => Ok(()),
            Err(_) => Err(format!("JSON 产物无法解析：{strict}")),
        },
    }
}

/// CSV：非空；首行为表头，其后每个非空行列数与表头一致（引号内逗号不计列）。
/// 至少一行数据行（表头 + 1 行数据）——仅表头视为占位交付。
fn validate_csv(trimmed: &str) -> Result<(), String> {
    let lines: Vec<&str> = trimmed
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Err("CSV 产物内容不能为空".to_string());
    }
    if lines.len() < 2 {
        return Err("CSV 产物需至少表头加一行数据（仅表头视为占位交付）".to_string());
    }
    let header = lines[0];
    let header_cols = csv_column_count(header);
    if header_cols < 2 {
        return Err("CSV 表头需至少两列".to_string());
    }
    for (idx, line) in lines.iter().enumerate().skip(1) {
        let cols = csv_column_count(line);
        if cols != header_cols {
            return Err(format!(
                "CSV 列数不一致：第 {} 行为 {cols} 列，表头为 {header_cols} 列",
                idx + 1
            ));
        }
    }
    Ok(())
}

/// CSV 列数（引号感知：双引号内逗号不计列）。
fn csv_column_count(line: &str) -> usize {
    let mut count = 1;
    let mut in_quotes = false;
    for c in line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => count += 1,
            _ => {}
        }
    }
    count
}

/// research：内容非空，且至少一条有效证据引用（文件引用或 URL）。
fn validate_research(trimmed: &str, evidence: &[WorkerEvidenceV1]) -> Result<(), String> {
    if trimmed.is_empty() {
        return Err("研究产物内容不能为空".to_string());
    }
    if !evidence.iter().any(|e| is_valid_evidence_ref(&e.source)) {
        return Err("研究产物需至少一条有效证据引用（文件引用或 http(s) URL）".to_string());
    }
    Ok(())
}

/// markdown/text：内容非空；markdown 额外拒绝占位交付
/// （整体为 TBD/待定 类占位词，或除标题/分隔线外无任何正文的空模板骨架）。
fn validate_markdown_or_text(trimmed: &str, is_markdown: bool) -> Result<(), String> {
    if trimmed.is_empty() {
        return Err(if is_markdown {
            "Markdown 产物内容不能为空".to_string()
        } else {
            "文本产物内容不能为空".to_string()
        });
    }
    if !is_markdown {
        return Ok(());
    }
    if is_placeholder_markdown(trimmed) {
        return Err("Markdown 产物为占位交付（TBD/空模板骨架），不满足交付标准".to_string());
    }
    Ok(())
}

/// 占位判定：整体等于占位词（小写比较），或无正文内容的空模板骨架
/// （仅剩标题行/分隔线）。
fn is_placeholder_markdown(trimmed: &str) -> bool {
    const PLACEHOLDERS: &[&str] = &[
        "tbd",
        "tbd.",
        "tbd?",
        "todo",
        "placeholder",
        "tbd: 待定",
        "待定",
        "待补充",
        "待完善",
        "xxx",
        "n/a",
        "…",
    ];
    let lower = trimmed.to_ascii_lowercase();
    if PLACEHOLDERS.iter().any(|p| lower == *p) {
        return true;
    }
    let has_body = trimmed.lines().any(|line| {
        let s = line.trim();
        !s.is_empty() && !s.starts_with('#') && !matches!(s, "---" | "***" | "___")
    });
    !has_body
}

// ---------------------------------------------------------------------------
// 单元测试：四种格式各一条样例（通过 + 拒绝用例）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(source: &str, note: Option<&str>) -> WorkerEvidenceV1 {
        WorkerEvidenceV1 {
            source: source.to_string(),
            note: note.map(str::to_string),
        }
    }

    #[test]
    fn json_sample_valid_and_invalid() {
        // 通过样例：合法 JSON（对象/数组均可）。
        let valid = validate_artifact_content("json", r#"{"result": 42, "items": [1, 2, 3]}"#, &[]);
        assert!(valid.valid, "合法 JSON 应通过：{:?}", valid.reason);

        // 拒绝：裹 Markdown 围栏。
        let fenced = validate_artifact_content("json", "```json\n{}\n```", &[]);
        assert!(!fenced.valid, "围栏包裹的 JSON 应被拒");

        // 拒绝：不可解析。
        let broken = validate_artifact_content("json", "{not json", &[]);
        assert!(!broken.valid, "坏 JSON 应被拒");

        // 接受：JSON 本体后追加说明文字（serde 报 "trailing characters"）。
        // 登记门不替检查器判定内容，但也不该因一句补充说明整步失败。
        let trailing = validate_artifact_content(
            "json",
            "{\"result\": 42}\n\n补充说明：以上为抽取结果。",
            &[],
        );
        assert!(
            trailing.valid,
            "JSON 后有说明文字应可登记：{:?}",
            trailing.reason
        );

        // 拒绝：空内容。
        assert!(!validate_artifact_content("json", "  \n", &[]).valid);
    }

    #[test]
    fn csv_sample_valid_and_invalid() {
        // 通过样例：表头 + 两行数据，列数一致（含引号内逗号）。
        let valid = validate_artifact_content(
            "csv",
            "name,note,weight\nalpha,\"x, y\",1.0\nbeta,plain,2.5\n",
            &[],
        );
        assert!(valid.valid, "列数一致的 CSV 应通过：{:?}", valid.reason);

        // 拒绝：列数不一致。
        let ragged = validate_artifact_content("csv", "a,b,c\n1,2\n3,4,5\n", &[]);
        assert!(!ragged.valid, "列数不一致的 CSV 应被拒");

        // 拒绝：仅表头（占位交付）。
        assert!(!validate_artifact_content("csv", "a,b,c\n", &[]).valid);

        // 拒绝：空内容。
        assert!(!validate_artifact_content("csv", "\n  \n", &[]).valid);
    }

    #[test]
    fn research_sample_valid_and_invalid() {
        // 通过样例：内容 + URL 证据（研究交付证据链）。
        let with_url = validate_artifact_content(
            "research",
            "# 调研\n\n结论：方案 A 更优。",
            &[ev("https://example.com/report", Some("来源报告"))],
        );
        assert!(
            with_url.valid,
            "带 URL 证据的研究交付应通过：{:?}",
            with_url.reason
        );

        // 通过样例：文件引用证据。
        let with_file =
            validate_artifact_content("research", "摘要……", &[ev("out/research-notes.md", None)]);
        assert!(with_file.valid, "文件引用证据应有效");

        // 拒绝：无证据（研究交付必须可溯源）。
        let no_evidence = validate_artifact_content("research", "结论：方案 A。", &[]);
        assert!(!no_evidence.valid, "无证据的研究交付应被拒");

        // 拒绝：证据来源全为无效空串。
        let bad_evidence = validate_artifact_content("research", "结论：方案 A。", &[ev("", None)]);
        assert!(!bad_evidence.valid);

        // 拒绝：空内容。
        assert!(!validate_artifact_content("research", "  ", &[ev("a/b.md", None)]).valid);
    }

    #[test]
    fn markdown_sample_valid_and_invalid() {
        // 通过样例：正常报告（标题 + 正文）。
        let valid = validate_artifact_content("markdown", "# 交付报告\n\n正文内容……", &[]);
        assert!(valid.valid, "正常 Markdown 应通过：{:?}", valid.reason);

        // 拒绝：TBD 占位交付。
        let tbd = validate_artifact_content("markdown", "TBD", &[]);
        assert!(!tbd.valid, "TBD 占位应被拒");

        // 拒绝：空模板骨架（仅标题与分隔线，无正文）。
        let skeleton = validate_artifact_content("markdown", "# 标题\n\n---\n\n## 小节\n", &[]);
        assert!(!skeleton.valid, "空模板骨架应被拒");

        // 拒绝：空内容。
        assert!(!validate_artifact_content("markdown", "", &[]).valid);

        // text：非空即通过（不做占位门控，兼容既有自由文本口径）。
        assert!(validate_artifact_content("text", "ok", &[]).valid);
        assert!(!validate_artifact_content("text", "  ", &[]).valid);
    }

    #[test]
    fn effective_format_reconciles_research_kind() {
        // 契约枚举不含 "research"：kind=research 的 text/markdown 声明按证据链规则。
        assert_eq!(effective_format("markdown", "research"), "research");
        assert_eq!(effective_format("text", "research"), "research");
        assert_eq!(effective_format("research", "document"), "research");
        // 非研究类保持声明格式；json/csv 声明不受 kind 影响。
        assert_eq!(effective_format("markdown", "document"), "markdown");
        assert_eq!(effective_format("json", "research"), "json");
        assert_eq!(effective_format("csv", "research"), "csv");
    }

    #[test]
    fn delivery_meta_fields_and_evidence_refs() {
        let meta = build_delivery_meta(
            "json",
            "research",
            r#"{"a": 1}"#,
            &[
                ev("https://example.com/x", Some("note")),
                ev("docs/notes.md", None),
            ],
        );
        assert_eq!(meta.format, "json");
        assert_eq!(meta.media_type, "application/json");
        assert_eq!(meta.file_name, "research.json");
        assert_eq!(meta.size_bytes, 8);
        assert_eq!(meta.sha256.len(), 64); // sha256 十六进制
        assert!(meta.validation.valid);

        let refs = evidence_refs_of(&[
            ev("https://example.com/x", Some("note")),
            ev("docs/notes.md", None),
        ]);
        assert_eq!(refs, vec!["https://example.com/x (note)", "docs/notes.md"]);

        // media type / 扩展名口径。
        assert_eq!(media_type_of("csv"), "text/csv");
        assert_eq!(file_name_of("document", "csv"), "document.csv");
        assert_eq!(file_name_of("review", "markdown"), "review.md");
        assert_eq!(file_name_of("", "text"), "artifact.txt");
        assert_eq!(media_type_of("unknown"), "text/plain");

        // 证据引用有效性判定。
        assert!(is_valid_evidence_ref("https://a.b/c"));
        assert!(is_valid_evidence_ref("out/notes.md"));
        assert!(is_valid_evidence_ref("report.pdf"));
        assert!(!is_valid_evidence_ref(""));
        assert!(!is_valid_evidence_ref("plain"));
    }
}

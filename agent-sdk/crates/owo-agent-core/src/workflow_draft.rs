//! AI 生成"用户自定义工作流"的提示词与结果抽取（服务端 `/workflows/draft` 使用）。
//!
//! 借鉴（研究资料库已下载的本地论文卡与项目快照）：
//! - **AFlow**（2410.10762）：工作流可由"搜索 + 执行反馈"自动生成/优化——本项目把
//!   生成结果经过**确定性校验器**（`user_workflow::validate_workflow_template`）与
//!   人工采纳两道关，先保证"可调度、不越权"，再谈优化；
//! - **ADAS**（ShengranHu/ADAS）：元 Agent 产出候选设计、候选需评估后归档——这里对应
//!   "草稿 = 候选"，采纳才进注册表；
//! - **dify / n8n / langgraph**（项目快照）：节点/边式工作流 + 人机在环 + 版本化分享——
//!   对应本项目 `TeamTemplate`（角色 + `depends_on` 边）与社区包导出导入。
//!
//! 本模块是纯函数：提示词构造 + 从模型回复里抽取 JSON 对象。

use owo_agent_protocol::TeamTemplate;

/// 生成提示词里允许的最大描述长度（防超长输入把上下文挤爆）。
pub const MAX_DRAFT_DESCRIPTION_CHARS: usize = 4_000;
/// 已有草稿做"迭代"时，回灌的上一版 JSON 长度上限。
pub const MAX_PREVIOUS_JSON_CHARS: usize = 12_000;

/// 生成工作流草稿的系统提示词（结构化输出契约 + 硬约束）。
///
/// 约束与 [`crate::user_workflow::validate_workflow_template`] 一一对应，避免"模型
/// 生成→校验必然报错"的来回：模型先按契约产出，宿主再校验并逐条回报。
pub fn draft_system_prompt(max_roles: usize, available_tools: &[String]) -> String {
    let tools_hint = if available_tools.is_empty() {
        "可用工具名：内置工具（如 read_file/write_file/edit_file/multi_edit/apply_patch/list_dir/search_files/grep/find_symbol/read_artifact/run_command/shell_output/web_fetch/web_search/todo/ask_user/git_status/git_diff/git_log…）或 MCP 命名空间形式 mcp__{server}__{tool}（server 段不含 __）。".to_string()
    } else {
        format!(
            "可用工具名（只能从这里选，或使用 mcp__{{server}}__{{tool}} 形式）：{}",
            available_tools.join(", ")
        )
    };
    format!(
        r#"你在为一个"用户自定义多 Agent 工作流"系统生成配置。输出必须是一个 JSON 对象（不要输出解释、不要加代码围栏以外的文字）。

JSON 结构（TeamTemplate）：
{{
  "template_id": "小写 ASCII 字母/数字/-_.，不以数字开头，唯一",
  "name": "人类可读名称",
  "mode": "single" | "team" | "swarmflow",
  "applicability": "适用条件关键词（空格分隔，用于自动匹配；必须非空）",
  "description": "一两句话说明这条工作流做什么、产出什么",
  "version": "1.0.0",
  "tags": ["分类标签"],
  "roles": [
    {{
      "role": "角色名（唯一）",
      "assignee": "agent" | "human" | "worker",
      "worker": "assignee=worker 时必填（内置 worker 名）",
      "depends_on": ["上游角色名"],
      "handoff_contract": "下游期望的交付格式（可选但推荐）",
      "verify": "non_empty" | "contains:<文本>" | "equals:<文本>"（可选）,
      "tools": ["角色可用工具名"],
      "write_paths": ["该角色允许写入的工作区相对目录（可空 = 工作区内可写）"],
      "model": "可选：该角色的模型名",
      "capabilities": ["可选能力标签，如 review"]
    }}
  ]
}}

硬约束（违反会被宿主的确定性校验器拒绝）：
1. 角色数 ≤ {max_roles}；角色名唯一；`depends_on` 只能引用已存在的角色，**不得成环**；
2. `mode=single` 时只能有 1 个角色；
3. `write_paths` 必须是工作区内相对路径，禁止绝对路径、盘符与 `..`；
4. `tools` 只能是内置工具名或 `mcp__{{server}}__{{tool}}`；{tools_hint}
5. `verify` 只支持 non_empty / contains:<文本> / equals:<文本>；
6. 至少一个角色负责产出最终交付物；只读分析角色不要给写工具。

设计要求：
- 让边界清晰的角色承担不同阶段（分析 → 实现/生成 → 校验/评审），把交付契约写清楚；
- 若任务需要人类确认或提供素材，用 assignee=human 的角色；
- 面向"任意特定功能"：请按用户描述定制角色/步骤/工具，而不是套用通用模板。

只输出 JSON。"#
    )
}

/// 一条工作流的真实运行证据（统计口径，见 `/workflows/stats`）。
///
/// 借鉴 AFlow（2410.10762）"执行反馈驱动工作流优化"：让模型**基于真实失败/取消记录**
/// 改进配置，而不是凭描述重写一遍。证据全部来自宿主记录，模型不得臆造。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkflowRunEvidence {
    pub template_id: String,
    pub runs: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: usize,
    /// 成功率（无终态为 None，不编造 0%）。
    pub success_rate: Option<f64>,
    /// "需要修复"原因（从未成功/高失败率/长期低成功率/最近失败）。
    pub attention: Vec<String>,
    /// 最近一次失败的定位（`team.failed · 失败：…`）。
    pub last_failure: Option<String>,
    /// 上一次「按证据改进 → 验证」的结果（迭代反馈；未验证为 None）。
    pub previous_attempt: Option<String>,
}

/// 把运行证据渲染成提示词段落（无运行记录 → 空串，调用方据此决定是否附上）。
pub fn render_run_evidence(evidence: &WorkflowRunEvidence) -> String {
    if evidence.runs == 0 {
        return String::new();
    }
    let mut block = String::from("## 运行证据（真实团队运行，请据此定位问题）\n");
    block.push_str(&format!(
        "- 运行 {} 次：成功 {} · 失败 {} · 取消 {}\n",
        evidence.runs, evidence.succeeded, evidence.failed, evidence.cancelled
    ));
    match evidence.success_rate {
        Some(rate) => block.push_str(&format!("- 成功率：{rate:.0}%\n")),
        None => block.push_str("- 成功率：暂无终态运行，无法计算\n"),
    }
    if !evidence.attention.is_empty() {
        block.push_str(&format!("- 需要修复：{}\n", evidence.attention.join("；")));
    }
    if let Some(failure) = evidence
        .last_failure
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        block.push_str(&format!("- 最近一次失败：{failure}\n"));
    }
    if let Some(attempt) = evidence
        .previous_attempt
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        block.push_str(&format!("- {attempt}\n"));
    }
    block.push_str(
        "只针对上述证据能解释的环节调整配置（角色划分/工具白名单/写范围/依赖顺序/交付契约/验证断言）；\n         保持仍然合理的角色名与依赖不变，不要为了改而重写整条流程。\n",
    );
    block
}

/// 生成草稿的用户消息（描述 + 约束 + 可选上一版草稿用于迭代）。
pub fn draft_user_prompt(
    description: &str,
    constraints: Option<&str>,
    previous: Option<&TeamTemplate>,
) -> String {
    draft_user_prompt_with_evidence(description, constraints, previous, None)
}

/// 带运行证据的生成/迭代提示词（`based_on` 工作流时使用）。
pub fn draft_user_prompt_with_evidence(
    description: &str,
    constraints: Option<&str>,
    previous: Option<&TeamTemplate>,
    evidence: Option<&WorkflowRunEvidence>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("## 目标\n");
    prompt.push_str(description.trim());
    if let Some(constraints) = constraints.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str("\n\n## 附加约束\n");
        prompt.push_str(constraints);
    }
    let evidence_block = evidence.map(render_run_evidence).unwrap_or_default();
    if !evidence_block.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(&evidence_block);
    }
    if let Some(previous) = previous {
        let json = serde_json::to_string_pretty(previous).unwrap_or_default();
        let truncated: String = json.chars().take(MAX_PREVIOUS_JSON_CHARS).collect();
        let label = if evidence_block.is_empty() {
            "## 现有草稿（请在其基础上改进；保留合理的角色划分）"
        } else {
            "## 现有配置（请在其基础上改进；保留仍然合理的角色与依赖）"
        };
        prompt.push_str(&format!("\n\n{label}\n```json\n"));
        prompt.push_str(&truncated);
        prompt.push_str("\n```");
    }
    prompt.push_str("\n\n请输出 JSON。");
    prompt
}

/// 修复提示词：把上一版草稿与宿主校验错误一起回灌，要求**只改错误相关字段**。
///
/// 这是"生成 → 宿主校验 → 带着错误重生成"的一次闭环（不无限重试；调用方限制次数）。
pub fn draft_repair_prompt(
    description: &str,
    constraints: Option<&str>,
    previous: &TeamTemplate,
    errors: &[String],
) -> String {
    let json = serde_json::to_string_pretty(previous).unwrap_or_default();
    let truncated: String = json.chars().take(MAX_PREVIOUS_JSON_CHARS).collect();
    let mut prompt = String::new();
    prompt.push_str("## 目标\n");
    prompt.push_str(description.trim());
    if let Some(constraints) = constraints.map(str::trim).filter(|value| !value.is_empty()) {
        prompt.push_str("\n\n## 附加约束\n");
        prompt.push_str(constraints);
    }
    prompt.push_str("\n\n## 上一版草稿（宿主校验未通过）\n```json\n");
    prompt.push_str(&truncated);
    prompt.push_str("\n```\n\n## 宿主校验错误（必须逐条修正）\n");
    for error in errors {
        prompt.push_str("- ");
        prompt.push_str(error);
        prompt.push('\n');
    }
    prompt.push_str(
        "\n只修正与上述错误相关的字段；不要扩大权限（工具/写范围）也不要删掉已有角色，除非错误涉及它们。\n只输出修正后的完整 JSON。",
    );
    prompt
}

/// 从模型回复里抽取第一个 JSON 对象（容忍 ```json 围栏与前后解释文字）。
///
/// 花括号配对按字符串感知扫描（跳过字符串内的括号与转义），因此
/// `{"a": "}"}` 这类内容不会被误判。
pub fn extract_json_object(response: &str) -> Option<&str> {
    let bytes = response.as_bytes();
    let mut start = None;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'{' {
            start = Some(index);
            break;
        }
    }
    let start = start?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match *byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = start + offset + 1;
                    return response.get(start..end);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use owo_agent_protocol::{TeamMode, TeamTemplateRole};

    fn sample_template() -> TeamTemplate {
        TeamTemplate {
            template_id: "video-recreate".to_string(),
            name: "视频复刻".to_string(),
            mode: TeamMode::Team,
            roles: vec![TeamTemplateRole {
                role: "script".to_string(),
                assignee: "agent".to_string(),
                ..Default::default()
            }],
            applicability: "视频复刻 分镜".to_string(),
            source_team_id: None,
            created_at: "2026-10-19T00:00:00Z".to_string(),
            origin: None,
            description: Some("从素材到成片".to_string()),
            version: Some("1.0.0".to_string()),
            tags: vec!["video".to_string()],
        }
    }

    #[test]
    fn system_prompt_states_schema_and_hard_constraints() {
        let prompt = draft_system_prompt(12, &[]);
        assert!(prompt.contains("template_id"));
        assert!(prompt.contains("depends_on"));
        assert!(prompt.contains("不得成环"));
        assert!(prompt.contains("角色数 ≤ 12"));
        assert!(prompt.contains("mcp__{server}__{tool}"));
        assert!(prompt.contains("只输出 JSON"));
        // 指定可用工具时按白名单提示。
        let restricted =
            draft_system_prompt(4, &["read_file".to_string(), "write_file".to_string()]);
        assert!(restricted.contains("read_file, write_file"));
        assert!(restricted.contains("角色数 ≤ 4"));
    }

    #[test]
    fn user_prompt_includes_description_constraints_and_previous_draft() {
        let prompt = draft_user_prompt("做一个游戏关卡生成流水线", Some("必须有人工验收"), None);
        assert!(prompt.contains("游戏关卡生成流水线"));
        assert!(prompt.contains("必须有人工验收"));
        assert!(!prompt.contains("现有草稿"));

        let refined = draft_user_prompt("改进它", None, Some(&sample_template()));
        assert!(refined.contains("现有草稿"));
        assert!(refined.contains("\"template_id\": \"video-recreate\""));
        assert!(refined.contains("请输出 JSON"));
    }

    #[test]
    fn run_evidence_is_rendered_only_when_present() {
        let evidence = WorkflowRunEvidence {
            template_id: "video-recreate".to_string(),
            runs: 3,
            succeeded: 0,
            failed: 3,
            cancelled: 0,
            success_rate: Some(0.0),
            attention: vec![
                "从未成功（失败 3 次）".to_string(),
                "最近一次运行失败".to_string(),
            ],
            last_failure: Some("team.failed · 失败：步骤失败且 replan 未启用".to_string()),
            // 上一次按证据改过，但验证还是失败（与基线相同）。
            previous_attempt: Some(
                "上次按证据改进后用同一目标验证：failed（未康复，与基线相同）".to_string(),
            ),
        };
        let block = render_run_evidence(&evidence);
        assert!(block.contains("运行 3 次"), "{block}");
        assert!(block.contains("成功率：0%"), "{block}");
        assert!(block.contains("从未成功（失败 3 次）"), "{block}");
        assert!(block.contains("最近一次失败"), "{block}");
        assert!(block.contains("上次按证据改进后用同一目标验证"), "{block}");
        // 未验证时不出现“上次”行（不假装有反馈）。
        let no_attempt = WorkflowRunEvidence {
            runs: 1,
            failed: 1,
            previous_attempt: None,
            ..Default::default()
        };
        assert!(!render_run_evidence(&no_attempt).contains("上次按证据改进"));

        let prompt = draft_user_prompt_with_evidence("让它别再失败", None, None, Some(&evidence));
        assert!(prompt.contains("## 运行证据"), "{prompt}");
        assert!(prompt.contains("只针对上述证据"), "{prompt}");

        // 从未运行 → 不注入证据段（也不编造 0% 成功率）。
        let empty = WorkflowRunEvidence {
            template_id: "fresh".to_string(),
            ..Default::default()
        };
        assert!(render_run_evidence(&empty).is_empty());
        let prompt = draft_user_prompt_with_evidence("新建一条", None, None, Some(&empty));
        assert!(!prompt.contains("## 运行证据"), "{prompt}");
        assert!(prompt.contains("现有草稿") == false, "{prompt}");

        // 无终态：给出"暂无终态"而不是 0%。
        let cancelled_only = WorkflowRunEvidence {
            template_id: "c".to_string(),
            runs: 2,
            succeeded: 0,
            failed: 0,
            cancelled: 2,
            success_rate: None,
            attention: vec![],
            last_failure: None,
            previous_attempt: None,
        };
        let block = render_run_evidence(&cancelled_only);
        assert!(block.contains("暂无终态运行"), "{block}");
        assert!(!block.contains("成功率：0%"), "{block}");

        // 有证据且带既有配置 → 标签切换为"现有配置"，模型知道要迭代而不是重写。
        let prompt = draft_user_prompt_with_evidence(
            "改进它",
            None,
            Some(&sample_template()),
            Some(&evidence),
        );
        assert!(prompt.contains("## 现有配置"), "{prompt}");
        assert!(!prompt.contains("## 现有草稿"), "{prompt}");
    }

    #[test]
    fn repair_prompt_carries_previous_draft_and_every_error() {
        let previous = sample_template();
        let errors = vec![
            "角色 script 的 tools 非法：未知工具：not_a_tool".to_string(),
            "角色依赖存在环：a → b → a".to_string(),
        ];
        let prompt =
            draft_repair_prompt("做一条视频流水线", Some("必须人工验收"), &previous, &errors);
        assert!(prompt.contains("宿主校验未通过"));
        assert!(prompt.contains("\"template_id\": \"video-recreate\""));
        assert!(prompt.contains("未知工具：not_a_tool"));
        assert!(prompt.contains("存在环"));
        assert!(prompt.contains("必须人工验收"));
        assert!(prompt.contains("不要扩大权限"));
        assert!(prompt.contains("只输出修正后的完整 JSON"));
    }

    #[test]
    fn extracts_json_from_fences_and_prose_including_braces_in_strings() {
        assert_eq!(
            extract_json_object("```json\n{\"a\":1}\n```"),
            Some("{\"a\":1}")
        );
        assert_eq!(
            extract_json_object("先说明一下。\n{\"a\": {\"b\": 2}}\n以上。"),
            Some("{\"a\": {\"b\": 2}}")
        );
        // 字符串里的花括号与转义不影响配对。
        assert_eq!(
            extract_json_object("{\"text\": \"}\" }"),
            Some("{\"text\": \"}\" }")
        );
        assert_eq!(
            extract_json_object("{\"text\": \"\\\"}\" }"),
            Some("{\"text\": \"\\\"}\" }")
        );
        assert_eq!(extract_json_object("没有 JSON"), None);
        assert_eq!(extract_json_object("{ 未闭合"), None);
    }
}

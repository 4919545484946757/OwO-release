//! Deterministic extraction of explicit acceptance checklists shared by Single and Team.

use crate::plan::VerificationPlanV1;

/// Return explicit checklist items. Checkbox lines are always explicit; ordinary
/// Markdown list items are included only beneath an acceptance/criteria heading.
pub fn explicit_acceptance_items(request: &str) -> Vec<String> {
    let mut fenced = false;
    let mut acceptance_heading_level = None;
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in request.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            let title = trimmed[level..].trim().to_lowercase();
            let is_acceptance_heading = ["验收", "完成标准", "验收标准", "acceptance", "criteria"]
                .iter()
                .any(|needle| title.contains(needle));
            if is_acceptance_heading {
                acceptance_heading_level = Some(level);
            } else if acceptance_heading_level.is_some_and(|active| active == 0 || level <= active)
            {
                acceptance_heading_level = None;
            }
            continue;
        }
        let plain_heading = trimmed.trim_end_matches([':', '：']).trim().to_lowercase();
        if [
            "验收",
            "验收标准",
            "验收条件",
            "验收点",
            "完成标准",
            "acceptance criteria",
            "acceptance requirements",
            "criteria",
        ]
        .contains(&plain_heading.as_str())
        {
            acceptance_heading_level = Some(0);
            continue;
        }

        let checkbox_item = strip_checkbox_marker(trimmed);
        let body = checkbox_item
            .or_else(|| acceptance_heading_level.and_then(|_| strip_list_marker(trimmed)));
        let Some(body) = body else {
            continue;
        };
        let item = normalize_whitespace(body);
        if !item.is_empty() && item.len() <= 2_048 && seen.insert(item.clone()) {
            items.push(item);
        }
    }
    items
}

/// Require each explicit checklist item to have an exact user-request citation
/// on at least one required host validation obligation.
pub fn validate_exact_user_request_quotes(quotes: &[String], request: &str) -> Result<(), String> {
    if quotes.is_empty() {
        return Err("任务没有引用任何用户原文要求".to_string());
    }
    let normalized_request = normalize_whitespace(request);
    let mut seen = std::collections::HashSet::new();
    for quote in quotes {
        let normalized_quote = normalize_whitespace(quote);
        if normalized_quote.is_empty() || normalized_quote.len() > 2_048 {
            return Err("用户原文引用不能为空且不得超过 2048 字节".to_string());
        }
        if !seen.insert(normalized_quote.clone()) {
            return Err("任务重复引用同一条用户原文要求".to_string());
        }
        if !normalized_request.contains(&normalized_quote) {
            // 真实模型实测：用户原文常带 Markdown 反引号（如 `创建 \`src/a.js\``），
            // 模型引用时往往会去掉反引号。此时只要去掉反引号后仍逐字出现，就视为有效
            // 引用——不放松"必须真实存在于用户原文"的约束，只容忍反引号差异。
            let request_without_ticks = normalized_request.replace('`', "");
            let quote_without_ticks = normalized_quote.replace('`', "");
            if quote_without_ticks.is_empty()
                || !request_without_ticks.contains(&quote_without_ticks)
            {
                return Err(format!("任务引用不属于当前用户目标原文：{quote}"));
            }
        }
    }
    Ok(())
}

pub fn validate_plan_covers_explicit_acceptance(
    plan: &VerificationPlanV1,
    request: &str,
) -> Result<(), String> {
    let citations = plan
        .requirements
        .iter()
        .filter(|requirement| requirement.required)
        .flat_map(|requirement| requirement.covers_requirement_ids.iter())
        .filter_map(|id| id.strip_prefix("user-request:"))
        .map(normalize_whitespace)
        .collect::<std::collections::HashSet<_>>();

    for item in explicit_acceptance_items(request) {
        if !citations.contains(&item) {
            return Err(format!("显式验收清单项未绑定到必需验证要求：{item}"));
        }
    }
    Ok(())
}

fn strip_checkbox_marker(line: &str) -> Option<&str> {
    for marker in [
        "- [ ]", "- [x]", "- [X]", "* [ ]", "* [x]", "* [X]", "+ [ ]", "+ [x]", "+ [X]",
    ] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim_start());
        }
    }
    None
}

fn strip_list_marker(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    let digits = line.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits > 0 {
        let rest = &line[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(rest.trim());
        }
    }
    None
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{VerificationRequirementV1, VerificationScopeV1};

    fn plan(citations: &[&str]) -> VerificationPlanV1 {
        VerificationPlanV1 {
            plan_id: "checklist-plan".to_string(),
            requirements: vec![VerificationRequirementV1 {
                requirement_id: "checklist".to_string(),
                covers_requirement_ids: citations
                    .iter()
                    .map(|quote| format!("user-request:{quote}"))
                    .collect(),
                validator_id: "workspace-command-success-v1".to_string(),
                validator_version: Some("1".to_string()),
                scope: VerificationScopeV1::WorkspacePaths {
                    relative_paths: vec!["src/lib.rs".to_string()],
                },
                arguments: serde_json::json!({"command":"cargo test -p owo-agent-core"}),
                required: true,
                resources: crate::plan::VerificationResourcesV1 {
                    cpu_slots: 1,
                    memory_mb: 16,
                    exclusive_workspace: false,
                    timeout_ms: 10_000,
                },
            }],
        }
    }

    #[test]
    fn explicit_checklist_items_are_extracted_outside_code_fences() {
        let request = "## 验收标准\n- 默认页码为 1\n  1. 越界返回空列表\n\n示例：\n```md\n- 不是要求\n```\n- [ ] 写入审计日志";
        assert_eq!(
            explicit_acceptance_items(request),
            vec!["默认页码为 1", "越界返回空列表", "写入审计日志",]
        );
    }

    #[test]
    fn plan_must_cover_every_explicit_acceptance_item() {
        let request = "验收标准\n- 默认页码为 1\n- 越界返回空列表\n普通说明";
        let complete = plan(&["默认页码为 1", "越界返回空列表"]);
        assert!(validate_plan_covers_explicit_acceptance(&complete, request).is_ok());

        let incomplete = plan(&["默认页码为 1"]);
        let error = validate_plan_covers_explicit_acceptance(&incomplete, request).unwrap_err();
        assert!(error.contains("越界返回空列表"));
    }

    #[test]
    fn ordinary_lists_are_not_promoted_to_acceptance_requirements() {
        let request = "执行顺序\n1. 先检查代码\n2. 再修改实现";
        assert!(explicit_acceptance_items(request).is_empty());
    }

    #[test]
    fn quote_matches_user_request_with_markdown_backticks_removed() {
        // 用户原文带反引号，模型引用时去掉反引号仍应视为有效（真实模型实测形态）。
        let request = "第 1 步：创建 `tank-game/index.html` 与 `tank-game/game.js`";
        assert!(validate_exact_user_request_quotes(
            &["创建 tank-game/index.html 与 tank-game/game.js".to_string()],
            request
        )
        .is_ok());
        // 去掉反引号后仍不存在的内容照旧拒绝，不放松真实性约束。
        let error =
            validate_exact_user_request_quotes(&["创建 tank-game/other.js".to_string()], request)
                .unwrap_err();
        assert!(error.contains("不属于当前用户目标原文"), "{error}");
    }
}

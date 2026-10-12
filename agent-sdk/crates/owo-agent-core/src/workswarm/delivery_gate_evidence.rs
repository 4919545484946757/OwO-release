use super::*;

pub(super) fn is_code_artifact_kind(kind: &str) -> bool {
    matches!(
        kind.trim().to_ascii_lowercase().as_str(),
        "code" | "source" | "patch" | "implementation" | "frontend" | "backend" | "integrated"
    )
}

/// Classification only strengthens acceptance; model labels never grant execution.
pub(super) fn requires_source_delivery(
    kind: &str,
    format: &str,
    file_name: &str,
    content: &str,
) -> bool {
    if kind.eq_ignore_ascii_case("review") {
        return false;
    }
    if is_code_artifact_kind(kind) || (!file_name.is_empty() && is_source_code_path(file_name)) {
        return true;
    }
    if matches!(
        format.trim().to_ascii_lowercase().as_str(),
        "html" | "javascript" | "js" | "typescript" | "python" | "rust" | "source" | "code"
    ) {
        return true;
    }
    let body = content
        .trim_start_matches(|ch: char| ch == '\u{feff}' || ch.is_whitespace())
        .to_ascii_lowercase();
    body.starts_with("<!doctype html") || body.starts_with("<html") || body.starts_with("```html")
}

/// Classify source paths from host-observed writes; model-declared Artifact.kind is not trusted.
pub(super) fn is_source_code_path(raw: &str) -> bool {
    let normalized = raw.replace('\\', "/");
    let path = std::path::Path::new(&normalized);
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let source_extensions = [
        "rs", "c", "h", "cc", "hh", "cpp", "hpp", "cxx", "hxx", "cs", "fs", "vb", "java", "kt",
        "kts", "scala", "go", "py", "pyi", "js", "jsx", "mjs", "cjs", "ts", "tsx", "vue", "svelte",
        "html", "css", "scss", "sass", "less", "sql", "sh", "bash", "ps1", "psm1", "bat", "cmd",
        "lua", "rb", "php", "swift", "dart", "ex", "exs", "hs", "lhs", "clj", "cljs", "cljc",
        "proto", "graphql", "gql", "m", "mm", "pl", "r", "jl", "tf", "tfvars", "nix", "astro",
        "mdx", "csproj", "fsproj", "vbproj", "sln", "cmake", "gradle", "cabal",
    ];
    if source_extensions.contains(&extension.as_str()) {
        return true;
    }
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        file_name.as_str(),
        "cargo.toml"
            | "go.mod"
            | "go.sum"
            | "package.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "package-lock.json"
            | "tsconfig.json"
            | "pyproject.toml"
            | "requirements.txt"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "makefile"
            | "justfile"
            | "cmakelists.txt"
            | "dockerfile"
    ) {
        return true;
    }
    false
}

pub(super) fn attempt_changeset_contains_code(
    change_sets: &[owo_agent_protocol::ChangeSet],
    team_id: &str,
    step_id: &str,
    attempt_id: &str,
) -> bool {
    change_sets.iter().any(|change_set| {
        change_set.team_id == team_id
            && change_set.step_id == step_id
            && change_set.attempt_id.as_deref() == Some(attempt_id)
            && change_set
                .changed_files
                .iter()
                .chain(change_set.result_hashes.iter().map(|file| &file.path))
                .any(|path| is_source_code_path(path))
    })
}

pub(super) fn uncovered_source_paths(
    change_sets: &[owo_agent_protocol::ChangeSet],
    team_id: &str,
    step_id: &str,
    attempt_id: &str,
    plan: &crate::plan::VerificationPlanV1,
) -> Vec<String> {
    let source_paths = change_sets
        .iter()
        .filter(|change_set| {
            change_set.team_id == team_id
                && change_set.step_id == step_id
                && change_set.attempt_id.as_deref() == Some(attempt_id)
        })
        .flat_map(|change_set| {
            change_set
                .changed_files
                .iter()
                .chain(change_set.result_hashes.iter().map(|file| &file.path))
        })
        .filter(|path| is_source_code_path(path))
        .map(|path| path.replace('\\', "/"))
        .collect::<std::collections::BTreeSet<_>>();
    let behavior_scope = plan
        .requirements
        .iter()
        .filter(|requirement| {
            requirement.required && requirement.validator_id == "workspace-command-success-v1"
        })
        .filter_map(|requirement| match &requirement.scope {
            crate::plan::VerificationScopeV1::WorkspacePaths { relative_paths } => {
                Some(relative_paths)
            }
            _ => None,
        })
        .flatten()
        .map(|path| path.replace('\\', "/"))
        .collect::<std::collections::BTreeSet<_>>();
    source_paths.difference(&behavior_scope).cloned().collect()
}

/// Host-generated review binding for the exact ChangeSet lineage and workspace bytes
/// visible when a reviewer context is assembled. ChangeSet decision status is omitted:
/// accepting a reviewed change must not invalidate the source snapshot.
///
/// Production callers share one batch fence through
/// [`review_source_snapshot_with_batch`]; this convenience wrapper exists for tests.
#[cfg(test)]
pub(super) fn review_source_snapshot(
    team_id: &str,
    step_id: &str,
    attempt_id: &str,
    change_sets: &[owo_agent_protocol::ChangeSet],
    workspace: Option<&std::path::Path>,
) -> Value {
    let mut snapshot = crate::workspace_snapshot::WorkspaceSnapshotBatch::new(workspace);
    review_source_snapshot_with_batch(team_id, step_id, attempt_id, change_sets, &mut snapshot)
}

/// Callers building multiple upstream bindings share only this operation's fence.
pub(super) fn review_source_snapshot_with_batch(
    team_id: &str,
    step_id: &str,
    attempt_id: &str,
    change_sets: &[owo_agent_protocol::ChangeSet],
    snapshot: &mut crate::workspace_snapshot::WorkspaceSnapshotBatch,
) -> Value {
    let mut matching = change_sets
        .iter()
        .filter(|change_set| {
            change_set.team_id == team_id
                && change_set.step_id == step_id
                && change_set.attempt_id.as_deref() == Some(attempt_id)
        })
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| left.change_set_id.cmp(&right.change_set_id));

    let change_set_ids = matching
        .iter()
        .map(|change_set| change_set.change_set_id.clone())
        .collect::<Vec<_>>();
    let stable_change_sets = matching
        .iter()
        .map(|change_set| {
            let mut changed_files = change_set
                .changed_files
                .iter()
                .map(|path| path.replace('\\', "/"))
                .collect::<Vec<_>>();
            changed_files.sort();
            let mut result_hashes = change_set.result_hashes.clone();
            result_hashes.sort_by(|left, right| left.path.cmp(&right.path));
            json!({
                "change_set_id": change_set.change_set_id,
                "step_id": change_set.step_id,
                "attempt_id": change_set.attempt_id,
                "changed_files": changed_files,
                "result_hashes": result_hashes,
            })
        })
        .collect::<Vec<_>>();
    let change_set_sha256 =
        CasStore::hash_of(&serde_json::to_vec(&stable_change_sets).unwrap_or_default());

    let mut paths = std::collections::BTreeSet::new();
    for change_set in &matching {
        paths.extend(
            change_set
                .changed_files
                .iter()
                .map(|path| path.replace('\\', "/")),
        );
        paths.extend(
            change_set
                .result_hashes
                .iter()
                .map(|file| file.path.replace('\\', "/")),
        );
    }
    let mut source_hashes = std::collections::BTreeMap::new();
    for path in &paths {
        let observation = match snapshot.read(path) {
            Some(Some(hash)) => json!({"observed":true,"exists":true,"sha256":hash}),
            Some(None) => json!({"observed":true,"exists":false,"sha256":Value::Null}),
            None => json!({"observed":false,"exists":Value::Null,"sha256":Value::Null}),
        };
        source_hashes.insert(path.clone(), observation);
    }
    let mut expected_hashes = std::collections::BTreeMap::new();
    let mut changeset_source_consistent = true;
    for change_set in &matching {
        for path in &change_set.changed_files {
            if !change_set
                .result_hashes
                .iter()
                .any(|file| file.path.replace('\\', "/") == path.replace('\\', "/"))
            {
                changeset_source_consistent = false;
            }
        }
        for file in &change_set.result_hashes {
            let path = file.path.replace('\\', "/");
            if expected_hashes
                .insert(path, file.sha256.clone())
                .is_some_and(|previous| previous != file.sha256)
            {
                changeset_source_consistent = false;
            }
        }
    }
    for (path, expected_hash) in &expected_hashes {
        let Some(observation) = source_hashes.get(path) else {
            changeset_source_consistent = false;
            continue;
        };
        if observation.get("observed").and_then(Value::as_bool) != Some(true) {
            changeset_source_consistent = false;
            continue;
        }
        let actual_hash = observation
            .get("sha256")
            .and_then(Value::as_str)
            .map(str::to_string);
        let exists = observation.get("exists").and_then(Value::as_bool);
        match expected_hash {
            Some(expected) if exists == Some(true) && actual_hash.as_deref() == Some(expected) => {}
            None if exists == Some(false) && actual_hash.is_none() => {}
            _ => changeset_source_consistent = false,
        }
    }
    json!({
        "step_id": step_id,
        "attempt_id": attempt_id,
        "change_set_ids": change_set_ids,
        "change_set_sha256": change_set_sha256,
        "source_hashes": source_hashes,
        "workspace_observed": snapshot.is_bound(),
        "changeset_source_consistent": changeset_source_consistent,
        "contains_source_code": paths.iter().any(|path| is_source_code_path(path)),
    })
}

pub(super) fn validate_review_artifact_kind(is_reviewer: bool, kind: &str) -> Result<(), String> {
    match (is_reviewer, kind == "review") {
        (true, true) | (false, false) => Ok(()),
        (true, false) => Err("review capability 必须提交 review 类型的结构化评审产物".to_string()),
        (false, true) => Err("只有独立 reviewer 才能提交 review 类型产物".to_string()),
    }
}

/// Host-authored review obligations derived from the exact upstream step contract.
pub(super) fn review_requirements_for_step(
    step: &crate::plan::StepSpec,
    goal_objective: &str,
) -> Vec<Value> {
    let mut requirements = Vec::new();
    let goal_objective = goal_objective.trim();
    if !goal_objective.is_empty() {
        requirements.push(serde_json::json!({
            "requirement_id": format!("{}:goal-objective", step.id),
            "kind": "goal_objective",
            "description": goal_objective,
        }));
    }
    if let Some(task) = step.input.get("assigned_task").and_then(Value::as_str) {
        let task = task.trim();
        if !task.is_empty() {
            requirements.push(serde_json::json!({
                "requirement_id": format!("{}:task-objective", step.id),
                "kind": "task_objective",
                "description": task,
            }));
        }
    }
    if let Some(quotes) = step
        .input
        .get("assigned_user_requirement_quotes")
        .and_then(Value::as_array)
    {
        for (index, quote) in quotes.iter().filter_map(Value::as_str).enumerate() {
            let quote = quote.trim();
            if quote.is_empty() {
                continue;
            }
            requirements.push(serde_json::json!({
                "requirement_id": format!("{}:user-request-quote:{index}", step.id),
                "kind": "user_request_quote",
                "description": quote,
            }));
        }
    }
    if let Some(acceptance) = step
        .input
        .get("assigned_acceptance")
        .and_then(Value::as_str)
    {
        let acceptance = acceptance.trim();
        if !acceptance.is_empty() {
            requirements.push(serde_json::json!({
                "requirement_id": format!("{}:task-acceptance", step.id),
                "kind": "task_acceptance",
                "description": acceptance,
            }));
        }
    }
    let mut used_requirement_ids = requirements
        .iter()
        .filter_map(|item| item.get("requirement_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(plan) = &step.verification_plan {
        used_requirement_ids.extend(
            plan.requirements
                .iter()
                .map(|item| format!("{}:{}", step.id, item.requirement_id)),
        );
    }
    let mut add_explicit_items = |source: &str, prefix: &str| {
        for (index, item) in crate::request_requirements::explicit_acceptance_items(source)
            .into_iter()
            .enumerate()
        {
            let mut requirement_id = format!("{}:{prefix}:{index}", step.id);
            while !used_requirement_ids.insert(requirement_id.clone()) {
                requirement_id.push_str(":host");
            }
            requirements.push(serde_json::json!({
                "requirement_id": requirement_id,
                "kind": "explicit_acceptance_item",
                "description": item,
            }));
        }
    };
    add_explicit_items(goal_objective, "goal-checklist");
    if let Some(task) = step.input.get("assigned_task").and_then(Value::as_str) {
        add_explicit_items(task, "task-checklist");
    }
    if let Some(acceptance) = step
        .input
        .get("assigned_acceptance")
        .and_then(Value::as_str)
    {
        add_explicit_items(acceptance, "acceptance-checklist");
    }
    if let Some(plan) = &step.verification_plan {
        for requirement in plan.requirements.iter().filter(|item| item.required) {
            requirements.push(serde_json::json!({
                "requirement_id": format!("{}:{}", step.id, requirement.requirement_id),
                "kind": "verification",
                "source_requirement_id": requirement.requirement_id,
                "validator_id": requirement.validator_id,
                "validator_version": requirement.validator_version,
                "scope": requirement.scope,
                "arguments": requirement.arguments,
                "covers_requirement_ids": requirement.covers_requirement_ids,
            }));
        }
    }
    requirements.sort_by(|left, right| {
        left.get("requirement_id")
            .and_then(Value::as_str)
            .cmp(&right.get("requirement_id").and_then(Value::as_str))
    });
    requirements
}

pub(super) fn validate_review_requirement_coverage(
    result: &owo_agent_workswarm::WorkerReviewResultV1,
    expected_ids: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    let reported = result
        .reviewed_requirement_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if reported.len() != result.reviewed_requirement_ids.len() {
        return Err("ReviewResult 重复声明 reviewed_requirement_ids".to_string());
    }
    if &reported != expected_ids {
        return Err(format!(
            "ReviewResult 未精确覆盖宿主验收要求：expected={expected_ids:?}, reported={reported:?}"
        ));
    }
    if let Some(unexpected) = result
        .findings
        .iter()
        .filter_map(|finding| {
            finding
                .requirement_id
                .as_deref()
                .filter(|id| !expected_ids.contains(*id))
        })
        .next()
    {
        return Err(format!(
            "ReviewResult finding 引用了未知验收要求：{unexpected}"
        ));
    }
    Ok(())
}

pub(super) fn validate_review_approval(result: &Value) -> Result<(), String> {
    let result: owo_agent_workswarm::WorkerReviewResultV1 = serde_json::from_value(result.clone())
        .map_err(|error| format!("ReviewResult 结构非法：{error}"))?;
    if result.verdict != owo_agent_workswarm::WorkerReviewVerdict::Approved {
        return Err("ReviewResult 未批准交付，需先完成 Issue 修复与复验".to_string());
    }
    for finding in &result.findings {
        if finding.detail.trim().is_empty() {
            return Err("ReviewResult 存在缺少 detail 的 finding".to_string());
        }
    }
    if result
        .findings
        .iter()
        .any(|finding| finding.severity.blocks_approval())
    {
        return Err("ReviewResult 含 blocker/major finding，不能批准交付".to_string());
    }
    Ok(())
}

pub(super) fn is_user_delivery_artifact(kind: &str) -> bool {
    kind != "review"
}

pub(super) fn changeset_delivery_error(
    status: owo_agent_protocol::ChangeSetStatus,
) -> Option<&'static str> {
    match status {
        owo_agent_protocol::ChangeSetStatus::Accepted => None,
        owo_agent_protocol::ChangeSetStatus::PendingReview => Some("仍待人工接受"),
        owo_agent_protocol::ChangeSetStatus::Conflicted => Some("存在未解决冲突"),
        owo_agent_protocol::ChangeSetStatus::Rejected => Some("已被拒绝"),
        owo_agent_protocol::ChangeSetStatus::Reverted => Some("已撤销"),
    }
}

pub(super) fn collect_attempt_changeset_evidence(
    team_id: &str,
    step_id: &str,
    attempt_id: &str,
    change_sets: &[owo_agent_protocol::ChangeSet],
) -> WorkSwarmResult<(Option<String>, Vec<String>)> {
    let mut matching: Vec<_> = change_sets
        .iter()
        .filter(|change_set| {
            change_set.step_id == step_id && change_set.attempt_id.as_deref() == Some(attempt_id)
        })
        .collect();
    for change_set in &matching {
        if change_set.team_id != team_id {
            return Err(WorkSwarmError::Conflict(format!(
                "任务 {step_id} 的 ChangeSet {} 团队归属不匹配",
                change_set.change_set_id
            )));
        }
        if let Some(reason) = changeset_delivery_error(change_set.status) {
            let detail = format!(
                "任务 {step_id} 的 ChangeSet {} {}，不能进入已验收交付",
                change_set.change_set_id, reason
            );
            return Err(match change_set.status {
                owo_agent_protocol::ChangeSetStatus::PendingReview
                | owo_agent_protocol::ChangeSetStatus::Conflicted => {
                    WorkSwarmError::DeliveryPending(detail)
                }
                _ => WorkSwarmError::Conflict(detail),
            });
        }
        if change_set.status == owo_agent_protocol::ChangeSetStatus::Accepted
            && change_set
                .decision
                .as_ref()
                .map(|decision| decision.action.as_str())
                != Some("accept")
        {
            return Err(WorkSwarmError::Conflict(format!(
                "任务 {step_id} 的 ChangeSet {} 缺少宿主接受决定",
                change_set.change_set_id
            )));
        }
    }
    matching.sort_by(|left, right| left.change_set_id.cmp(&right.change_set_id));
    let snapshot: Vec<_> = matching
        .iter()
        .map(|change_set| {
            json!({
                "change_set_id": &change_set.change_set_id,
                "team_id": &change_set.team_id,
                "step_id": &change_set.step_id,
                "attempt_id": &change_set.attempt_id,
                "role": &change_set.role,
                "base_hashes": &change_set.base_hashes,
                "result_hashes": &change_set.result_hashes,
                "changed_files": &change_set.changed_files,
                "diff_ref": &change_set.diff_ref,
                "status": &change_set.status,
                "decision": &change_set.decision,
            })
        })
        .collect();
    let digest = if snapshot.is_empty() {
        None
    } else {
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|error| WorkSwarmError::Serialization(error.to_string()))?;
        Some(CasStore::hash_of(&bytes))
    };
    let refs = matching
        .iter()
        .map(|change_set| format!("changeset://{}", change_set.change_set_id))
        .collect();
    Ok((digest, refs))
}

/// Ensure workspace validation evidence for changed files describes the exact
/// accepted ChangeSet result snapshot. This prevents a validator run before a
/// later edit from being reused as proof for different delivered bytes.
/// 模板角色（verify 只声明 non_empty）的宿主兜底：当前 attempt 是否存在真实、
/// 成功且覆盖全部已修改源码的行为命令回执。
///
/// 与 [`evaluate_workspace_command_receipt`] 的区别：不依赖模型/模板声明的命令文本，
/// 只信任宿主 `team.command.executed` 事件里由 ToolHost 生成的
/// `CommandExecutionReceipt`（`validator_id=workspace-command-success-v1`、
/// `exit_code=0`、工作区哈希完整），并要求回执快照与当前 ChangeSet 的最终源码哈希
/// 逐一一致——写完源码后必须真的运行过行为命令，且之后不得再改写。
pub(super) fn attempt_behavior_receipt_evidence(
    team_id: &str,
    event_details: &[String],
    step_id: &str,
    attempt_id: &str,
    change_sets: &[owo_agent_protocol::ChangeSet],
) -> Option<String> {
    let mut source_hashes = std::collections::BTreeMap::new();
    for change_set in change_sets.iter().filter(|change_set| {
        change_set.team_id == team_id
            && change_set.step_id == step_id
            && change_set.attempt_id.as_deref() == Some(attempt_id)
    }) {
        for file in &change_set.result_hashes {
            let path = file.path.replace('\\', "/");
            if is_source_code_path(&path) {
                source_hashes.insert(path, file.sha256.clone());
            }
        }
        for path in &change_set.changed_files {
            let path = path.replace('\\', "/");
            if is_source_code_path(&path) {
                source_hashes.entry(path).or_insert(None);
            }
        }
    }
    if source_hashes.is_empty() {
        return None;
    }
    event_details.iter().rev().find_map(|detail| {
        let event: Value = serde_json::from_str(detail).ok()?;
        if event.get("step_id").and_then(Value::as_str) != Some(step_id)
            || event.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
        {
            return None;
        }
        let receipt: crate::CommandExecutionReceipt =
            serde_json::from_value(event.get("receipt")?.clone()).ok()?;
        if receipt.exit_code != 0
            || receipt.validator_id.as_deref() != Some("workspace-command-success-v1")
            || !receipt.workspace_hashes_complete
        {
            return None;
        }
        let covers = source_hashes.iter().all(|(path, expected)| {
            receipt
                .workspace_hashes
                .get(path)
                .is_some_and(|actual| actual == expected)
        });
        covers.then(|| format!("command-result:sha256:{}", receipt.result_sha256))
    })
}

pub(super) fn evaluate_workspace_command_receipt(
    team_id: &str,
    requirement: &crate::plan::VerificationRequirementV1,
    event_details: &[String],
    step_id: &str,
    attempt_id: &str,
    change_sets: &[owo_agent_protocol::ChangeSet],
) -> (
    crate::plan::ValidationVerdictV1,
    Option<String>,
    std::collections::BTreeMap<String, String>,
    Option<String>,
) {
    use crate::plan::{ValidationVerdictV1, VerificationScopeV1};

    let unsupported = |detail: String| {
        (
            ValidationVerdictV1::Unsupported,
            Some(detail),
            std::collections::BTreeMap::new(),
            None,
        )
    };
    let VerificationScopeV1::WorkspacePaths { relative_paths } = &requirement.scope else {
        return unsupported("行为检查要求绑定 WorkspacePaths 文件集合".to_string());
    };
    let Some(command) = requirement.arguments.get("command").and_then(Value::as_str) else {
        return unsupported("行为检查缺少 command 参数".to_string());
    };
    if !crate::verification::is_registered_behavior_command(command) {
        return unsupported("行为检查命令不在宿主登记的测试命令集合内".to_string());
    }
    let command_sha256 = CasStore::hash_of(command.trim().as_bytes());
    let mut matching = event_details.iter().filter_map(|detail| {
        let event: Value = serde_json::from_str(detail).ok()?;
        if event.get("step_id").and_then(Value::as_str) != Some(step_id)
            || event.get("attempt_id").and_then(Value::as_str) != Some(attempt_id)
        {
            return None;
        }
        let receipt = event.get("receipt")?;
        if receipt.get("command_sha256").and_then(Value::as_str) != Some(command_sha256.as_str()) {
            return None;
        }
        Some(receipt.clone())
    });
    let Some(receipt_value) = matching.next_back() else {
        return (
            ValidationVerdictV1::Unverified,
            Some("当前 task/attempt 没有匹配的宿主命令执行回执".to_string()),
            std::collections::BTreeMap::new(),
            None,
        );
    };
    let receipt: crate::CommandExecutionReceipt = match serde_json::from_value(receipt_value) {
        Ok(receipt) => receipt,
        Err(error) => return unsupported(format!("宿主命令回执结构无效：{error}")),
    };
    if let Err((verdict, detail)) =
        crate::command_evidence::validate_command_receipt(requirement, &receipt)
    {
        return (
            verdict,
            Some(detail),
            std::collections::BTreeMap::new(),
            Some(format!("command-result:sha256:{}", receipt.result_sha256)),
        );
    }
    let mut subject_hashes = std::collections::BTreeMap::new();
    for raw_path in relative_paths {
        let path = raw_path.replace('\\', "/");
        let Some(hash) = receipt.workspace_hashes.get(&path) else {
            return (
                ValidationVerdictV1::Unverified,
                Some(format!("命令执行时没有宿主快照证据：{path}")),
                subject_hashes,
                Some(format!("command-result:sha256:{}", receipt.result_sha256)),
            );
        };
        let subject_hash = match hash {
            Some(hash) => hash.clone(),
            None => {
                let is_recorded_deletion = receipt.workspace_hashes_complete
                    && change_sets.iter().any(|change_set| {
                        change_set.team_id == team_id
                            && change_set.step_id == step_id
                            && change_set.attempt_id.as_deref() == Some(attempt_id)
                            && change_set
                                .changed_files
                                .iter()
                                .any(|changed| changed.replace('\\', "/") == path)
                            && change_set.result_hashes.iter().any(|file| {
                                file.path.replace('\\', "/") == path && file.sha256.is_none()
                            })
                    });
                if !is_recorded_deletion {
                    return (
                        ValidationVerdictV1::Failed,
                        Some(format!("命令执行时工作区文件不存在或不可读：{path}")),
                        subject_hashes,
                        Some(format!("command-result:sha256:{}", receipt.result_sha256)),
                    );
                }
                crate::verification::workspace_path_absence_sha256()
            }
        };
        subject_hashes.insert(format!("workspace-path:{raw_path}"), subject_hash);
    }
    if let Err(reason) = validate_workspace_receipt_snapshot(
        team_id,
        step_id,
        attempt_id,
        relative_paths,
        &subject_hashes,
        change_sets,
    ) {
        return (
            ValidationVerdictV1::Failed,
            Some(reason),
            subject_hashes,
            Some(format!("command-result:sha256:{}", receipt.result_sha256)),
        );
    }
    let evidence_ref = Some(format!("command-result:sha256:{}", receipt.result_sha256));
    if receipt.exit_code != 0 {
        return (
            ValidationVerdictV1::Failed,
            Some(format!("宿主命令退出码为 {}", receipt.exit_code)),
            subject_hashes,
            evidence_ref,
        );
    }
    (
        ValidationVerdictV1::Passed,
        Some(format!(
            "宿主登记命令成功，exit_code=0 command_sha256={command_sha256}"
        )),
        subject_hashes,
        evidence_ref,
    )
}

pub(super) fn validate_workspace_receipt_snapshot(
    team_id: &str,
    step_id: &str,
    attempt_id: &str,
    relative_paths: &[String],
    subject_hashes: &std::collections::BTreeMap<String, String>,
    change_sets: &[owo_agent_protocol::ChangeSet],
) -> Result<(), String> {
    let matching: Vec<_> = change_sets
        .iter()
        .filter(|change_set| {
            change_set.team_id == team_id
                && change_set.step_id == step_id
                && change_set.attempt_id.as_deref() == Some(attempt_id)
        })
        .collect();
    if matching.is_empty() {
        return Ok(());
    }

    let mut expected = std::collections::BTreeMap::new();
    let mut changed = std::collections::BTreeSet::new();
    for change_set in matching {
        changed.extend(
            change_set
                .changed_files
                .iter()
                .map(|path| path.replace('\\', "/")),
        );
        for file in &change_set.result_hashes {
            let path = file.path.replace('\\', "/");
            if expected
                .insert(path.clone(), file.sha256.clone())
                .is_some_and(|previous| previous != file.sha256)
            {
                return Err(format!("ChangeSet 对同一文件 {path} 包含冲突结果哈希"));
            }
        }
    }
    for raw_path in relative_paths {
        let path = raw_path.replace('\\', "/");
        if changed.contains(&path) && !expected.contains_key(&path) {
            return Err(format!("ChangeSet 缺少已变更文件 {path} 的结果哈希"));
        }
        let Some(expected_hash) = expected.get(&path) else {
            continue;
        };
        let evidence_key = format!("workspace-path:{raw_path}");
        let actual_hash = subject_hashes
            .get(&evidence_key)
            .ok_or_else(|| format!("workspace 验证未为 ChangeSet 文件 {path} 产生最终源码哈希"))?;
        let matches_result = match expected_hash {
            Some(expected_hash) => actual_hash == expected_hash,
            None if changed.contains(&path) => {
                actual_hash == &crate::verification::workspace_path_absence_sha256()
            }
            None => false,
        };
        if !matches_result {
            return Err(format!(
                "workspace 验证哈希与接受的 ChangeSet 快照不一致：{path}"
            ));
        }
    }
    Ok(())
}

pub(super) fn store_validation_receipt(
    state: &mut GoalRunState,
    step_id: &str,
    receipt: &crate::plan::ValidationReceiptV1,
) {
    let Some(record) = state.records.get_mut(step_id) else {
        return;
    };
    if let Some(existing) = record
        .validation_receipts
        .iter_mut()
        .find(|existing| existing.receipt_id == receipt.receipt_id)
    {
        *existing = receipt.clone();
    } else {
        record.validation_receipts.push(receipt.clone());
    }
}

pub(super) struct ValidationReceiptInput<'a> {
    pub(super) team_id: &'a str,
    pub(super) step_id: &'a str,
    pub(super) attempts: u32,
    pub(super) attempt_id: &'a str,
    pub(super) epoch: u64,
    pub(super) requirement_id: &'a str,
    pub(super) scope: &'a crate::plan::VerificationScopeV1,
    pub(super) validator_id: &'a str,
    pub(super) validator_version: &'a str,
    pub(super) arguments_sha256: &'a str,
    pub(super) input_sha256: &'a str,
    pub(super) artifact_id: &'a str,
    pub(super) artifact_sha256: &'a str,
    pub(super) changeset_sha256: Option<String>,
    pub(super) changeset_refs: Vec<String>,
    pub(super) evidence_ref: &'a str,
    pub(super) started_at: &'a str,
    pub(super) verdict: crate::plan::ValidationVerdictV1,
    pub(super) detail: Option<String>,
    pub(super) subject_hashes: std::collections::BTreeMap<String, String>,
    pub(super) additional_evidence_refs: Vec<String>,
}

pub(super) fn make_validation_receipt(
    input: ValidationReceiptInput<'_>,
) -> crate::plan::ValidationReceiptV1 {
    let receipt_identity = json!({
        "team_id": input.team_id,
        "step_id": input.step_id,
        "attempts": input.attempts,
        "attempt_id": input.attempt_id,
        "epoch": input.epoch,
        "requirement_id": input.requirement_id,
        "scope": input.scope,
        "validator_id": input.validator_id,
        "validator_version": input.validator_version,
        "arguments_sha256": input.arguments_sha256,
        "input_sha256": input.input_sha256,
        "artifact_id": input.artifact_id,
        "artifact_sha256": input.artifact_sha256,
        "changeset_sha256": input.changeset_sha256,
        "changeset_refs": input.changeset_refs,
        "evidence_ref": input.evidence_ref,
        "subject_hashes": &input.subject_hashes,
        "additional_evidence_refs": &input.additional_evidence_refs,
    })
    .to_string();
    let mut subject_sha256 = std::collections::HashMap::from([(
        input.artifact_id.to_string(),
        input.artifact_sha256.to_string(),
    )]);
    subject_sha256.extend(input.subject_hashes);
    let evidence_refs = std::iter::once(input.evidence_ref.to_string())
        .chain(input.changeset_refs)
        .chain(input.additional_evidence_refs)
        .collect();
    crate::plan::ValidationReceiptV1 {
        receipt_id: CasStore::hash_of(receipt_identity.as_bytes()),
        task_id: input.step_id.to_string(),
        attempt_id: input.attempt_id.to_string(),
        epoch: input.epoch,
        requirement_id: input.requirement_id.to_string(),
        validator_id: input.validator_id.to_string(),
        validator_version: input.validator_version.to_string(),
        arguments_sha256: input.arguments_sha256.to_string(),
        input_sha256: input.input_sha256.to_string(),
        environment_id: format!("owo-agent-core/{}", env!("CARGO_PKG_VERSION")),
        changeset_sha256: input.changeset_sha256,
        detail: input.detail,
        subject_sha256,
        verdict: input.verdict,
        evidence_refs,
        review_result: None,
        started_at: input.started_at.to_string(),
        completed_at: now_ts(),
    }
}

#[cfg(test)]
mod review_requirement_tests {
    use super::*;

    #[test]
    fn review_requirement_coverage_requires_exact_unique_host_ids() {
        let expected = std::collections::BTreeSet::from([
            "step-a:goal-objective".to_string(),
            "step-a:verify-behavior".to_string(),
        ]);
        let valid: owo_agent_workswarm::WorkerReviewResultV1 =
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "reviewed_requirement_ids": [
                    "step-a:goal-objective",
                    "step-a:verify-behavior"
                ],
                "findings": []
            }))
            .unwrap();
        assert!(validate_review_requirement_coverage(&valid, &expected).is_ok());

        let missing: owo_agent_workswarm::WorkerReviewResultV1 =
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "reviewed_requirement_ids": ["step-a:goal-objective"],
                "findings": []
            }))
            .unwrap();
        assert!(validate_review_requirement_coverage(&missing, &expected).is_err());

        let extra: owo_agent_workswarm::WorkerReviewResultV1 =
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "reviewed_requirement_ids": [
                    "step-a:goal-objective",
                    "step-a:verify-behavior",
                    "invented-requirement"
                ],
                "findings": []
            }))
            .unwrap();
        assert!(validate_review_requirement_coverage(&extra, &expected).is_err());

        let duplicate: owo_agent_workswarm::WorkerReviewResultV1 =
            serde_json::from_value(serde_json::json!({
                "verdict": "approved",
                "reviewed_requirement_ids": [
                    "step-a:goal-objective",
                    "step-a:goal-objective",
                    "step-a:verify-behavior"
                ],
                "findings": []
            }))
            .unwrap();
        assert!(validate_review_requirement_coverage(&duplicate, &expected).is_err());
    }

    #[test]
    fn review_requirements_include_goal_task_acceptance_and_required_verification() {
        let mut step = crate::plan::StepSpec::new("step-a", "m-builder");
        step.input = serde_json::json!({
            "assigned_task": "implement the API",
            "assigned_acceptance": "Acceptance criteria\n- returns the saved record",
            "assigned_user_requirement_quotes": ["return the saved record"]
        });
        step.verification_plan = Some(crate::plan::VerificationPlanV1 {
            plan_id: "verify-step-a".to_string(),
            requirements: vec![
                crate::plan::VerificationRequirementV1 {
                    requirement_id: "behavior".to_string(),
                    covers_requirement_ids: Vec::new(),
                    validator_id: "workspace-command-success-v1".to_string(),
                    validator_version: Some("1".to_string()),
                    scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                        relative_paths: vec!["src/api.rs".to_string()],
                    },
                    arguments: serde_json::json!({"command":"cargo test"}),
                    required: true,
                    resources: Default::default(),
                },
                crate::plan::VerificationRequirementV1 {
                    requirement_id: "goal-checklist:0".to_string(),
                    covers_requirement_ids: Vec::new(),
                    validator_id: "workspace-command-success-v1".to_string(),
                    validator_version: Some("1".to_string()),
                    scope: crate::plan::VerificationScopeV1::WorkspacePaths {
                        relative_paths: vec!["src/api.rs".to_string()],
                    },
                    arguments: serde_json::json!({"command":"cargo test"}),
                    required: true,
                    resources: Default::default(),
                },
            ],
        });
        let requirements = review_requirements_for_step(
            &step,
            "## Acceptance criteria\n- complete the feature behavior",
        );
        let ids = requirements
            .iter()
            .filter_map(|item| item.get("requirement_id").and_then(Value::as_str))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(ids.contains("step-a:goal-objective"));
        assert!(ids.contains("step-a:goal-checklist:0:host"));
        assert!(ids.contains("step-a:task-objective"));
        assert!(ids.contains("step-a:task-acceptance"));
        assert!(ids.contains("step-a:user-request-quote:0"));
        assert!(ids.contains("step-a:acceptance-checklist:0"));
        assert!(ids.contains("step-a:behavior"));
    }
}

#[cfg(test)]
mod behavior_receipt_tests {
    use super::*;

    fn change_set() -> owo_agent_protocol::ChangeSet {
        owo_agent_protocol::ChangeSet {
            change_set_id: "cs-1".to_string(),
            team_id: "team-1".to_string(),
            step_id: "s-implementer".to_string(),
            attempt_id: Some("attempt-1".to_string()),
            role: "implementer".to_string(),
            base_hashes: Vec::new(),
            result_hashes: vec![owo_agent_protocol::ChangeSetFileHash {
                path: "src/calc.py".to_string(),
                sha256: Some("abc123".to_string()),
                content_available: true,
            }],
            changed_files: vec!["src/calc.py".to_string()],
            diff_ref: None,
            status: owo_agent_protocol::ChangeSetStatus::PendingReview,
            created_at: "2026-10-09T00:00:00Z".to_string(),
            decision: None,
            conflicts: Vec::new(),
        }
    }

    fn event_detail(exit_code: i32, validator: Option<&str>, complete: bool) -> String {
        serde_json::json!({
            "step_id": "s-implementer",
            "task_id": "s-implementer",
            "attempt_id": "attempt-1",
            "receipt": {
                "command_sha256": "cmd-sha",
                "exit_code": exit_code,
                "result_sha256": "result-sha",
                "workspace_hashes_complete": complete,
                "validator_id": validator,
                "validator_version": "1",
                "workspace_hashes": {"src/calc.py": "abc123"},
                "workspace_hashes_before": {"src/calc.py": "old-sha"},
            }
        })
        .to_string()
    }

    #[test]
    fn template_code_change_is_accepted_with_host_behavior_receipt() {
        let change_sets = vec![change_set()];
        // 成功 + 注册行为命令 + 快照覆盖最终源码哈希 → 接受并带证据引用。
        let evidence = attempt_behavior_receipt_evidence(
            "team-1",
            &[event_detail(0, Some("workspace-command-success-v1"), true)],
            "s-implementer",
            "attempt-1",
            &change_sets,
        );
        assert_eq!(
            evidence.as_deref(),
            Some("command-result:sha256:result-sha")
        );
    }

    #[test]
    fn template_code_change_rejects_untrusted_or_stale_receipts() {
        let change_sets = vec![change_set()];
        for detail in [
            // 非零退出码。
            event_detail(1, Some("workspace-command-success-v1"), true),
            // 未注册行为命令（普通命令不算行为证据）。
            event_detail(0, None, true),
            // 快照不完整。
            event_detail(0, Some("workspace-command-success-v1"), false),
        ] {
            assert!(
                attempt_behavior_receipt_evidence(
                    "team-1",
                    &[detail],
                    "s-implementer",
                    "attempt-1",
                    &change_sets,
                )
                .is_none(),
                "不可信回执不得作为行为证据"
            );
        }
        // 命令执行后源码又被改写（回执哈希 ≠ 最终 ChangeSet 哈希）→ 不接受。
        let stale = serde_json::json!({
            "step_id": "s-implementer",
            "attempt_id": "attempt-1",
            "receipt": {
                "command_sha256": "cmd-sha",
                "exit_code": 0,
                "result_sha256": "result-sha",
                "workspace_hashes_complete": true,
                "validator_id": "workspace-command-success-v1",
                "validator_version": "1",
                "workspace_hashes": {"src/calc.py": "older-sha"},
                "workspace_hashes_before": {"src/calc.py": "old-sha"},
            }
        })
        .to_string();
        assert!(attempt_behavior_receipt_evidence(
            "team-1",
            &[stale],
            "s-implementer",
            "attempt-1",
            &change_sets,
        )
        .is_none());
    }
}

#[cfg(test)]
mod runnable_artifact_regressions {
    use super::*;
    #[test]
    fn implementer_html_requires_source_delivery_even_without_declared_code_kind() {
        assert!(requires_source_delivery(
            "implementer",
            "html",
            "result.txt",
            "<!doctype html><html></html>"
        ));
        assert!(requires_source_delivery(
            "text",
            "text",
            "result.txt",
            " \u{feff}<!doctype html><html></html>"
        ));
        assert!(requires_source_delivery(
            "text",
            "text",
            "page.html",
            "body"
        ));
        assert!(requires_source_delivery(
            "text",
            "text",
            "result.txt",
            "```html\n<html></html>\n```"
        ));
        assert!(!requires_source_delivery(
            "implementer",
            "markdown",
            "notes.md",
            "A normal summary"
        ));
        assert!(!requires_source_delivery(
            "review",
            "json",
            "review.json",
            "{\"finding\":\"<html>\"}"
        ));
    }
}

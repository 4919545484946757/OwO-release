use crate::plan::{
    verify_output, ValidationVerdictV1, VerificationPlanV1, VerificationRequirementV1,
    VerificationResourcesV1, VerificationScopeV1, VerificationSpec,
};
use serde_json::Value;

#[path = "verification_workspace.rs"]
mod workspace;
pub(crate) use workspace::WorkspaceValidationBatch;
pub use workspace::{
    execute_workspace_requirement, is_registered_workspace_validator,
    workspace_validator_arguments_supported, workspace_validator_contracts,
    WorkspaceValidatorArgumentKind, WorkspaceValidatorContract, MAX_WORKSPACE_VALIDATION_PATHS,
    MIN_WORKSPACE_VALIDATION_PATHS,
};

/// Domain-separated digest used in receipt subject_sha256 when a changed workspace path is absent.
pub(crate) fn workspace_path_absence_sha256() -> String {
    crate::CasStore::hash_of(b"owo-agent:workspace-path-absence:v1")
}

use std::collections::BTreeMap;
use std::path::Path;

/// Compile the legacy output assertion into the same host-owned validator contract
/// used by Team delivery checks. Legacy names never create executable validators.
pub fn requirement_for_spec(
    requirement_id: &str,
    spec: &VerificationSpec,
) -> VerificationRequirementV1 {
    let (validator_id, arguments) = match spec {
        VerificationSpec::OutputNonEmpty => ("artifact-output-non-empty-v1", serde_json::json!({})),
        VerificationSpec::OutputContains(value) => (
            "artifact-output-contains-v1",
            serde_json::json!({"value": value}),
        ),
        VerificationSpec::OutputEquals(value) => (
            "artifact-output-equals-v1",
            serde_json::json!({"value": value}),
        ),
        VerificationSpec::Custom(name) => (name.as_str(), serde_json::json!({})),
    };
    VerificationRequirementV1 {
        requirement_id: requirement_id.to_string(),
        covers_requirement_ids: Vec::new(),
        validator_id: validator_id.to_string(),
        validator_version: Some("1".to_string()),
        scope: VerificationScopeV1::StepOutput,
        arguments,
        required: true,
        resources: VerificationResourcesV1 {
            cpu_slots: 1,
            memory_mb: 1,
            exclusive_workspace: false,
            timeout_ms: 5_000,
        },
    }
}

/// Turn compatibility assertions into an explicit plan. Empty input remains an
/// absent plan at the caller; this helper always creates one obligation per item.
pub fn plan_for_specs(plan_id: &str, specs: &[VerificationSpec]) -> VerificationPlanV1 {
    VerificationPlanV1 {
        plan_id: plan_id.to_string(),
        requirements: specs
            .iter()
            .enumerate()
            .map(|(index, spec)| {
                requirement_for_spec(&format!("{plan_id}:requirement:{index}"), spec)
            })
            .collect(),
    }
}

/// Execute a registered text-output validator. Unknown IDs, versions, scopes, or
/// malformed arguments remain Unsupported and cannot pass as non-empty output.
pub fn execute_requirement(
    requirement: &VerificationRequirementV1,
    content: &str,
) -> (ValidationVerdictV1, Option<String>) {
    if requirement.validator_version.as_deref() != Some("1") {
        return (
            ValidationVerdictV1::Unsupported,
            Some(format!(
                "validator {} 版本未注册，当前为 unsupported/unverified",
                requirement.validator_id
            )),
        );
    }
    if requirement.scope != VerificationScopeV1::StepOutput {
        return (
            ValidationVerdictV1::Unsupported,
            Some(format!(
                "validator {} 的 scope 未注册，当前为 unsupported/unverified",
                requirement.validator_id
            )),
        );
    }
    let spec = match requirement.validator_id.as_str() {
        "artifact-output-non-empty-v1" => {
            if requirement
                .arguments
                .as_object()
                .is_some_and(|arguments| arguments.is_empty())
            {
                VerificationSpec::OutputNonEmpty
            } else {
                return (
                    ValidationVerdictV1::Unsupported,
                    Some("artifact-output-non-empty-v1 不接受参数".to_string()),
                );
            }
        }
        "artifact-output-contains-v1" => {
            match exact_string_argument(&requirement.arguments, "value") {
                Some(value) => VerificationSpec::OutputContains(value.to_string()),
                None => {
                    return (
                        ValidationVerdictV1::Unsupported,
                        Some(
                            "artifact-output-contains-v1 要求且仅接受字符串参数 value".to_string(),
                        ),
                    );
                }
            }
        }
        "artifact-output-equals-v1" => {
            match exact_string_argument(&requirement.arguments, "value") {
                Some(value) => VerificationSpec::OutputEquals(value.to_string()),
                None => {
                    return (
                        ValidationVerdictV1::Unsupported,
                        Some("artifact-output-equals-v1 要求且仅接受字符串参数 value".to_string()),
                    );
                }
            }
        }
        unknown => {
            return (
                ValidationVerdictV1::Unsupported,
                Some(format!(
                    "validator「{unknown}」未注册，当前为 unsupported/unverified"
                )),
            );
        }
    };
    match verify_output(&spec, content) {
        Ok(()) => (ValidationVerdictV1::Passed, None),
        Err(detail) => (ValidationVerdictV1::Failed, Some(detail)),
    }
}

/// Dispatch a requirement through the single host-owned validator registry used by
/// GoalRunner and WorkSwarm. Dynamic model output can select only registered
/// validators; it never supplies an executable command.
pub fn execute_registered_requirement(
    requirement: &VerificationRequirementV1,
    content: &str,
    workspace_root: Option<&Path>,
) -> (
    ValidationVerdictV1,
    Option<String>,
    BTreeMap<String, String>,
) {
    match &requirement.scope {
        VerificationScopeV1::StepOutput => {
            let (verdict, detail) = execute_requirement(requirement, content);
            (verdict, detail, BTreeMap::new())
        }
        VerificationScopeV1::WorkspacePaths { .. } => match workspace_root {
            Some(root) => execute_workspace_requirement(requirement, root),
            None => (
                ValidationVerdictV1::Unverified,
                Some("未绑定宿主工作区，WorkspacePaths 验证未执行".to_string()),
                BTreeMap::new(),
            ),
        },
        _ => (
            ValidationVerdictV1::Unsupported,
            Some(format!(
                "validator {} 的 scope 未注册，当前为 unsupported/unverified",
                requirement.validator_id
            )),
            BTreeMap::new(),
        ),
    }
}

pub fn is_registered_behavior_command(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty()
        || command.len() > 512
        || !command.is_ascii()
        || command.chars().any(|ch| {
            matches!(
                ch as u32,
                38 | 124 | 60 | 62 | 94 | 37 | 40 | 41 | 59 | 96 | 34 | 39 | 33 | 10 | 13
            )
        })
    {
        return false;
    }
    let tokens: Vec<_> = command.split_whitespace().collect();
    let bypass_flags = [
        "--no-run",
        "--list",
        "--help",
        "-h",
        "/?",
        "/help",
        "--dry-run",
        "--collect-only",
        "--co",
        "--if-present",
        "--passwithnotests",
        "--skip",
        "--test-only",
        "--test-name-pattern",
        "--exclude-task",
        "-x",
        "-dskiptests",
        "-dmaven.test.skip=true",
    ];
    if tokens.iter().any(|token| {
        let normalized = token.to_ascii_lowercase();
        bypass_flags.contains(&normalized.as_str())
            || normalized.starts_with("--test-name-pattern=")
            || normalized.starts_with("--test-only=")
            || normalized.starts_with("--no-run=")
            || normalized.starts_with("--collect-only=")
            || normalized.starts_with("-dmaven.test.skip=")
            || normalized == "--"
                && tokens.iter().any(|candidate| {
                    matches!(
                        candidate.to_ascii_lowercase().as_str(),
                        "--list" | "--help" | "-h" | "--collect-only" | "--co"
                    )
                })
    }) {
        return false;
    }
    if tokens.first() == Some(&"node") {
        return tokens.len() >= 3
            && tokens[1] == "--test"
            && tokens[2..].iter().all(|file| {
                !file.starts_with('-')
                    && !file.contains('*')
                    && !file.contains('?')
                    && [".js", ".cjs", ".mjs"]
                        .iter()
                        .any(|extension| file.ends_with(extension))
            });
    }
    matches!(
        tokens.as_slice(),
        ["cargo", "test", ..]
            | ["npm", "test"]
            | ["npm", "run", "test", ..]
            | ["pnpm", "test", ..]
            | ["yarn", "test", ..]
            | ["bun", "test", ..]
            | ["pytest", ..]
            | ["python", "-m", "pytest", ..]
            | ["dotnet", "test", ..]
            | ["go", "test", ..]
            | ["mvn", "test", ..]
            | ["gradle", "test", ..]
            | ["ctest", ..]
    )
}

fn exact_string_argument<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    let object = arguments.as_object()?;
    if object.len() != 1 {
        return None;
    }
    object.get(key)?.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn legacy_assertion_compiles_to_registered_requirement() {
        let requirement = requirement_for_spec(
            "step-1",
            &VerificationSpec::OutputContains("ready".to_string()),
        );
        assert_eq!(requirement.requirement_id, "step-1");
        assert_eq!(requirement.validator_id, "artifact-output-contains-v1");
        assert_eq!(requirement.validator_version.as_deref(), Some("1"));
        assert_eq!(
            execute_requirement(&requirement, "service ready").0,
            ValidationVerdictV1::Passed
        );
        assert_eq!(
            execute_requirement(&requirement, "service pending"),
            (
                ValidationVerdictV1::Failed,
                Some("验证失败：输出缺少「ready」（实际：service pending）".to_string())
            )
        );
    }

    #[test]
    fn shared_dispatch_fails_closed_for_unbound_or_unsupported_scopes() {
        let workspace_requirement = VerificationRequirementV1 {
            requirement_id: "task:workspace".to_string(),
            covers_requirement_ids: Vec::new(),
            validator_id: "workspace-file-exists-v1".to_string(),
            validator_version: Some("1".to_string()),
            scope: VerificationScopeV1::WorkspacePaths {
                relative_paths: vec!["README.md".to_string()],
            },
            arguments: serde_json::json!({}),
            required: true,
            resources: VerificationResourcesV1 {
                cpu_slots: 1,
                memory_mb: 8,
                exclusive_workspace: false,
                timeout_ms: 1_000,
            },
        };
        let (verdict, detail, subjects) =
            execute_registered_requirement(&workspace_requirement, "output", None);
        assert_eq!(verdict, ValidationVerdictV1::Unverified);
        assert!(detail.unwrap().contains("未绑定宿主工作区"));
        assert!(subjects.is_empty());

        let artifact_scope_requirement = VerificationRequirementV1 {
            scope: VerificationScopeV1::ArtifactRefs {
                artifact_ids: vec!["artifact-1".to_string()],
            },
            ..workspace_requirement
        };
        let (verdict, detail, subjects) =
            execute_registered_requirement(&artifact_scope_requirement, "output", None);
        assert_eq!(verdict, ValidationVerdictV1::Unsupported);
        assert!(detail.unwrap().contains("scope 未注册"));
        assert!(subjects.is_empty());
    }

    #[test]
    fn workspace_validators_are_read_only_path_scoped_and_hash_bound() {
        let root = std::env::temp_dir().join(format!(
            "owo-workspace-verification-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/app.js");
        std::fs::write(&file, "export const state = 'accepted';").unwrap();
        let make_requirement =
            |validator_id: &str, arguments: Value, path: &str| VerificationRequirementV1 {
                requirement_id: "task-verify".to_string(),
                covers_requirement_ids: Vec::new(),
                validator_id: validator_id.to_string(),
                validator_version: Some("1".to_string()),
                scope: VerificationScopeV1::WorkspacePaths {
                    relative_paths: vec![path.to_string()],
                },
                arguments,
                required: true,
                resources: VerificationResourcesV1 {
                    cpu_slots: 1,
                    memory_mb: 8,
                    exclusive_workspace: false,
                    timeout_ms: 1_000,
                },
            };
        let contains = make_requirement(
            "workspace-file-contains-v1",
            serde_json::json!({"text":"accepted"}),
            "src/app.js",
        );
        let (verdict, detail, hashes) = execute_workspace_requirement(&contains, &root);
        assert_eq!(verdict, ValidationVerdictV1::Passed, "{detail:?}");
        let initial_hash = format!("{:x}", Sha256::digest(b"export const state = 'accepted';"));
        assert_eq!(
            hashes.get("workspace-path:src/app.js"),
            Some(&initial_hash),
            "receipt subject must bind the exact checked file version"
        );

        let json_file = root.join("src/state.json");
        std::fs::write(&json_file, r#"{"status":"ready"}"#).unwrap();
        let json_requirement = make_requirement(
            "workspace-json-field-equals-v1",
            serde_json::json!({"field":"status","expected":"ready"}),
            "src/state.json",
        );
        assert_eq!(
            execute_workspace_requirement(&json_requirement, &root).0,
            ValidationVerdictV1::Passed
        );

        std::fs::write(&file, "export const state = 'pending';").unwrap();
        let (verdict, _, hashes) = execute_workspace_requirement(&contains, &root);
        assert_eq!(verdict, ValidationVerdictV1::Failed);
        assert_ne!(hashes.get("workspace-path:src/app.js"), Some(&initial_hash));

        let traversal = make_requirement(
            "workspace-file-exists-v1",
            serde_json::json!({}),
            "../outside.txt",
        );
        assert_eq!(
            execute_workspace_requirement(&traversal, &root).0,
            ValidationVerdictV1::Unsupported
        );
        let unknown = make_requirement("shell-command-v1", serde_json::json!({}), "src/app.js");
        assert_eq!(
            execute_workspace_requirement(&unknown, &root).0,
            ValidationVerdictV1::Unsupported
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_validator_version_scope_and_arguments_fail_closed() {
        let mut requirement = requirement_for_spec("step-1", &VerificationSpec::OutputNonEmpty);
        requirement.validator_id = "custom-check".to_string();
        assert_eq!(
            execute_requirement(&requirement, "non-empty").0,
            ValidationVerdictV1::Unsupported
        );

        requirement.validator_id = "artifact-output-non-empty-v1".to_string();
        requirement.validator_version = Some("2".to_string());
        assert_eq!(
            execute_requirement(&requirement, "non-empty").0,
            ValidationVerdictV1::Unsupported
        );

        requirement.validator_version = Some("1".to_string());
        requirement.scope = VerificationScopeV1::Manual;
        assert_eq!(
            execute_requirement(&requirement, "non-empty").0,
            ValidationVerdictV1::Unsupported
        );

        requirement.scope = VerificationScopeV1::StepOutput;
        requirement.validator_id = "artifact-output-contains-v1".to_string();
        assert_eq!(
            execute_requirement(&requirement, "non-empty").0,
            ValidationVerdictV1::Unsupported
        );
        requirement.arguments = serde_json::json!({"value": "ok", "ignored": true});
        assert_eq!(
            execute_requirement(&requirement, "ok").0,
            ValidationVerdictV1::Unsupported
        );

        requirement.validator_id = "artifact-output-non-empty-v1".to_string();
        requirement.arguments = serde_json::json!({"ignored": true});
        assert_eq!(
            execute_requirement(&requirement, "non-empty").0,
            ValidationVerdictV1::Unsupported
        );
    }

    #[test]
    fn behavior_commands_are_registered_test_runners_without_shell_chaining() {
        assert!(is_registered_behavior_command(
            "cargo test -p owo-agent-core"
        ));
        assert!(is_registered_behavior_command("npm test"));
        assert!(is_registered_behavior_command(
            "python -m pytest tests/test_api.py"
        ));
        assert!(!is_registered_behavior_command("echo passed"));
        assert!(!is_registered_behavior_command("cargo test --no-run"));
        assert!(!is_registered_behavior_command("cargo test -- --list"));
        assert!(!is_registered_behavior_command("npm test --if-present"));
        assert!(!is_registered_behavior_command("pytest --collect-only"));
        assert!(!is_registered_behavior_command("npm test && echo passed"));
        assert!(!is_registered_behavior_command("cargo check"));
    }

    #[test]
    fn workspace_subject_snapshot_recheck_detects_mutation_and_escape() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        let path = root.path().join("src/lib.rs");
        std::fs::write(&path, "pub fn ready() {}\n").unwrap();
        let subjects = std::collections::HashMap::from([(
            "workspace-path:src/lib.rs".to_string(),
            crate::CasStore::hash_of(b"pub fn ready() {}\n"),
        )]);
        let matches = |root: &std::path::Path,
                       subjects: &std::collections::HashMap<String, String>| {
            crate::workspace_snapshot::WorkspaceSnapshotBatch::new(Some(root))
                .subjects_match(subjects)
        };
        assert!(matches(root.path(), &subjects));

        std::fs::write(&path, "pub fn broken() {}\n").unwrap();
        assert!(!matches(root.path(), &subjects));

        std::fs::remove_file(&path).unwrap();
        let absent = std::collections::HashMap::from([(
            "workspace-path:src/lib.rs".to_string(),
            workspace_path_absence_sha256(),
        )]);
        assert!(matches(root.path(), &absent));
        std::fs::write(&path, "recreated").unwrap();
        assert!(!matches(root.path(), &absent));

        let escaped = std::collections::HashMap::from([(
            "workspace-path:../outside".to_string(),
            "any-hash".to_string(),
        )]);
        assert!(!matches(root.path(), &escaped));
    }
}

#[cfg(test)]
mod node_behavior_regressions {
    use super::is_registered_behavior_command;
    #[test]
    fn node_test_entry_is_registered_but_skip_and_shell_bypass_are_not() {
        assert!(is_registered_behavior_command(
            "node --test outputs/page.test.cjs"
        ));
        for command in [
            "node --eval malicious",
            "node --test",
            "node --test --test-skip-pattern=.* outputs/page.test.cjs",
            "node --test --test-shard=2/2 outputs/page.test.cjs",
            "node --test --test-only outputs/page.test.cjs",
            "node --test --test-name-pattern=nonexistent outputs/page.test.cjs",
            "node --test outputs/page.test.cjs && echo passed",
        ] {
            assert!(!is_registered_behavior_command(command), "{command}");
        }
    }
}

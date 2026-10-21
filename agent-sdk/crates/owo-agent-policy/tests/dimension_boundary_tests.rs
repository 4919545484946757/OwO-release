//! Exercise live policy decisions, not only the dimension-name helper.
use owo_agent_policy::{
    Decision, FilesystemScope, Level, PermissionProfile, PermissionRequest, PermissionSpec,
    PersistenceScope, Policy, RuleScope,
};
use serde_json::json;
fn policy(filesystem: FilesystemScope, command: RuleScope, network: RuleScope) -> Policy {
    let policy = Policy::new(std::env::temp_dir());
    policy.set_profile(PermissionProfile::Unrestricted);
    policy.set_spec(PermissionSpec {
        filesystem,
        command,
        network,
        persistence: PersistenceScope::Once,
        scopes: vec![],
    });
    policy
}
#[test]
fn network_denial_cannot_be_bypassed_with_shell_mcp_or_unknown_tools() {
    let policy = policy(
        FilesystemScope::Unrestricted,
        RuleScope::Unrestricted,
        RuleScope::Deny,
    );
    for tool in [
        "run_command",
        "mcp__synthetic__fetch",
        "foreign_synthetic_tool",
    ] {
        let request = PermissionRequest::new(
            tool,
            json!({"command":"echo synthetic"}),
            Level::Execute,
            "synthetic dimension probe",
        );
        assert_eq!(
            policy.decision(&request),
            Decision::Deny,
            "network refusal bypass by {tool}"
        );
        assert!(
            policy.execution_denial(&request).is_some(),
            "execution-time recheck must also deny {tool}"
        );
    }
}
#[test]
fn readonly_filesystem_cannot_be_bypassed_with_process_or_external_tools() {
    let policy = policy(
        FilesystemScope::WorkspaceRead,
        RuleScope::Unrestricted,
        RuleScope::Unrestricted,
    );
    for tool in [
        "run_command",
        "mcp__synthetic__write",
        "foreign_synthetic_tool",
    ] {
        let request = PermissionRequest::new(
            tool,
            json!({}),
            Level::Execute,
            "synthetic non-read capability",
        );
        assert_eq!(
            policy.decision(&request),
            Decision::Deny,
            "readonly filesystem bypass by {tool}"
        );
    }
    let read = policy.evaluate(
        "read_file",
        &json!({"path":std::env::temp_dir().to_string_lossy()}),
    );
    assert_eq!(
        policy.decision(&read),
        Decision::Allow,
        "known filesystem reads remain available"
    );
}
#[test]
fn dynamic_readonly_hint_cannot_hide_network_or_command_capability() {
    let server = format!("dimension-{}", uuid::Uuid::new_v4().simple());
    let effect = owo_agent_policy::register_mcp_effect(
        &server,
        "synthetic",
        Some(&json!({"readOnlyHint":true})),
        true,
    );
    let policy = policy(
        FilesystemScope::Unrestricted,
        RuleScope::Deny,
        RuleScope::Deny,
    );
    let request = PermissionRequest::new(
        &effect.tool,
        json!({}),
        Level::Read,
        "host-verified readonly still has external capability",
    );
    assert_eq!(policy.decision(&request), Decision::Deny);
    owo_agent_policy::tool_effects::remove_prefix(&owo_agent_policy::mcp_tool_prefix(&server));
}

#[test]
fn external_read_hint_is_not_a_workspace_or_endpoint_scope() {
    for policy in [
        policy(
            FilesystemScope::WorkspaceRead,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
        ),
        policy(
            FilesystemScope::WorkspaceWrite,
            RuleScope::Unrestricted,
            RuleScope::Unrestricted,
        ),
        policy(
            FilesystemScope::Unrestricted,
            RuleScope::Allowlisted,
            RuleScope::Unrestricted,
        ),
        policy(
            FilesystemScope::Unrestricted,
            RuleScope::Unrestricted,
            RuleScope::Allowlisted,
        ),
    ] {
        let request = PermissionRequest::new(
            "mcp__synthetic__read",
            json!({}),
            Level::Read,
            "synthetic unbounded external read",
        );
        assert_eq!(policy.decision(&request), Decision::Deny);
    }
}

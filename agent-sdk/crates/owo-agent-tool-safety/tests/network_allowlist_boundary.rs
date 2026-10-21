//! Exact destination claims must never be expanded to broad AppContainer SIDs.
use owo_agent_tool_safety::sandbox::*;
fn policy(host: &str) -> SandboxPolicy {
    let mut p =
        SandboxPolicy::for_workspace("synthetic-destination-boundary", std::env::temp_dir());
    p.network_policy = NetworkPolicy::AllowList;
    p.allow_hosts = vec![host.into()];
    p
}
#[test]
fn exact_host_policy_requires_a_real_destination_enforcing_broker() {
    for host in ["example.invalid:443", "127.0.0.1:45678"] {
        assert!(matches!(policy(host).validate(),Err(SandboxError::Unsupported(_))),"exact destination {host} is not enforced by an InternetClient/PrivateNetworkClientServer capability SID");
    }
}
#[test]
fn exact_host_never_receives_broad_network_capabilities() {
    for host in ["example.invalid:443", "127.0.0.1:45678"] {
        let policy = policy(host);
        assert!(
            app_container_network_capabilities(&policy).is_empty(),
            "an exact allowlist must not gain an entire network class"
        );
        for sid in [internet_client_sid(), private_network_client_server_sid()] {
            assert!(
                validate_app_container_network(&policy, &[sid]).is_err(),
                "broad network capability is not a destination allowlist"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn real_manager_refuses_before_starting_a_child_or_allowing_degradation() {
    let manager = default_manager();
    for degraded in [false, true] {
        let mut policy = policy("example.invalid:443");
        policy.allow_degraded = degraded;
        let command =
            SandboxCommand::new(std::env::current_exe().unwrap().to_string_lossy(), policy);
        let result = manager.lock().unwrap().spawn(&command);
        assert!(matches!(result,Err(SandboxError::Unsupported(_))),"exact-host policy must fail at the policy gate, never degrade or launch a broad-network child");
    }
}

#![cfg(windows)]
//! Real Windows API and bounded synthetic AppContainer process acceptance.
//! Creates a private ephemeral profile; grants no workspace ACLs.
use owo_agent_tool_safety::sandbox::probe_platform_support;
#[test]
fn modern_windows_userenv_sid_support_is_not_misread_as_boolean_failure() {
    let support = probe_platform_support();
    assert!(support.app_container,"independent userenv derivation returned S_OK/non-null SID on this supported Windows host, but Rust reports: {}",support.reason);
}

#[test]
fn appcontainer_process_must_capture_output_without_degraded_fallback() {
    use owo_agent_tool_safety::sandbox::*;
    let system = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let mut policy = SandboxPolicy::for_workspace("synthetic-appcontainer-process", system.clone());
    policy.file_scope = FileScope::WorkspacePlusReadonlySystem;
    policy.require_isolation = IsolationLevel::AppContainerJob;
    policy.allow_degraded = false;
    policy.network_policy = NetworkPolicy::None;
    policy.mem_mb = Some(128);
    policy.active_process_limit = Some(2);
    let command = SandboxCommand::new(system.join("cmd.exe").to_string_lossy(), policy)
        .with_cwd(system)
        .with_args(vec![
            "/D".into(),
            "/C".into(),
            "echo owo-synthetic-appcontainer".into(),
        ]);
    let manager = default_manager();
    let mut child =
        manager.lock().unwrap().spawn(&command).expect(
            "AppContainer process acceptance requires a real child, never a Job-only fallback",
        );
    let result = child
        .wait_output()
        .expect("capture actual AppContainer child output");
    assert_eq!(
        result.exit_code,
        0,
        "synthetic stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("owo-synthetic-appcontainer"),
        "AppContainer output must not silently disappear"
    );
}

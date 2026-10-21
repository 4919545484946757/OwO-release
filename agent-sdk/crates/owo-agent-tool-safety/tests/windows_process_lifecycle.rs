#![cfg(windows)]

use owo_agent_tool_safety::sandbox::*;
use std::io::Write;
use std::time::{Duration, Instant};

fn manager() -> std::sync::Arc<std::sync::Mutex<SandboxManager>> {
    let support = probe_platform_support();
    assert!(
        support.job_object,
        "Windows lifecycle tests require a real Job Object"
    );
    default_manager()
}

fn child(manager: &std::sync::Arc<std::sync::Mutex<SandboxManager>>, mode: &str) -> SandboxProcess {
    child_with_env(manager, mode, Vec::new())
}

fn child_with_env(
    manager: &std::sync::Arc<std::sync::Mutex<SandboxManager>>,
    mode: &str,
    env: Vec<(String, String)>,
) -> SandboxProcess {
    let workspace = std::env::current_dir().unwrap();
    let mut policy = SandboxPolicy::for_workspace("lifecycle-test", workspace.clone());
    policy.require_isolation = IsolationLevel::JobOnly;
    policy.allow_degraded = false;
    policy.cpu_ms = Some(30_000);
    policy.mem_mb = Some(256);
    policy.active_process_limit = Some(4);
    let mut command =
        SandboxCommand::new(std::env::current_exe().unwrap().to_string_lossy(), policy)
            .with_cwd(workspace)
            .with_args(vec![
                "--ignored".into(),
                "--exact".into(),
                "sandbox_child_payload".into(),
                "--nocapture".into(),
            ]);
    command
        .env
        .push(("OWO_LIFECYCLE_CHILD".into(), mode.into()));
    command.env.extend(env);
    manager
        .lock()
        .unwrap()
        .spawn(&command)
        .expect("spawn controlled test child")
}

fn wait_bounded(
    manager: &std::sync::Arc<std::sync::Mutex<SandboxManager>>,
    mut process: SandboxProcess,
) -> SandboxWaitInfo {
    let handle = process.handle.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = process.wait_output();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(result) => {
            worker.join().unwrap();
            result.unwrap()
        }
        Err(error) => {
            let _ = manager.lock().unwrap().kill(&handle);
            let _ = rx.recv_timeout(Duration::from_secs(5));
            panic!("controlled child failed to finish within the lifecycle budget: {error}");
        }
    }
}

#[test]
#[ignore = "only invoked as a controlled child by the lifecycle tests"]
fn sandbox_child_payload() {
    match std::env::var("OWO_LIFECYCLE_CHILD").as_deref() {
        Ok("stderr") => {
            std::io::stderr()
                .write_all(&vec![b'e'; 1024 * 1024])
                .unwrap();
            println!("stdout-after-stderr");
        }
        Ok("large") => {
            for _ in 0..2048 {
                std::io::stderr().write_all(&[b'e'; 8192]).unwrap();
            }
            println!("drained-large-stderr");
        }
        Ok("environment") => {
            let private_name = std::env::var("OWO_TEST_PRIVATE_NAME").unwrap();
            assert!(std::env::var_os(private_name).is_none());
            assert_eq!(
                std::env::var("OWO_TEST_EXPLICIT").unwrap(),
                "synthetic-host-value"
            );
            println!("worker-environment-boundary-ok");
        }
        Ok("tree") => {
            let mut grandchild = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "sandbox_child_payload",
                    "--nocapture",
                ])
                .env("OWO_LIFECYCLE_CHILD", "wait")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            std::fs::write(
                std::env::var("OWO_LIFECYCLE_PID_FILE").unwrap(),
                grandchild.id().to_string(),
            )
            .unwrap();
            std::thread::sleep(Duration::from_secs(30));
            let _ = grandchild.wait();
        }
        Ok("wait") => std::thread::sleep(Duration::from_secs(30)),
        _ => panic!("child payload requires a test-owned mode"),
    }
}

#[test]
fn job_only_drains_stderr_while_stdout_is_open() {
    let manager = manager();
    let process = child(&manager, "stderr");
    let result = wait_bounded(&manager, process);
    assert_eq!(result.exit_code, 0);
    assert!(String::from_utf8_lossy(&result.stdout).contains("stdout-after-stderr"));
    assert!(result.stderr.len() >= 1024 * 1024);
}

#[test]
fn manager_kill_terminates_the_owned_job() {
    let manager = manager();
    let process = child(&manager, "wait");
    let started = Instant::now();
    manager
        .lock()
        .unwrap()
        .kill(&process.handle)
        .expect("manager must really terminate its Job");
    assert_ne!(wait_bounded(&manager, process).exit_code, 0);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn capture_remains_bounded_but_keeps_draining() {
    let manager = manager();
    let process = child(&manager, "large");
    let result = wait_bounded(&manager, process);
    assert_eq!(result.exit_code, 0);
    assert!(result.stderr.len() <= 8 * 1024 * 1024 + 128);
    assert!(String::from_utf8_lossy(&result.stderr).contains("output truncated"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("drained-large-stderr"));
}

#[test]
fn foreign_and_expired_handles_do_not_authorize_pid_kills() {
    let manager = manager();
    let process = child(&manager, "stderr");
    let handle = process.handle.clone();
    wait_bounded(&manager, process);
    assert!(manager.lock().unwrap().kill(&handle).is_err());
    let mut foreign = handle;
    foreign.id.push_str("-foreign");
    assert!(manager.lock().unwrap().kill(&foreign).is_err());
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}
struct ProcessWatch(*mut std::ffi::c_void);
impl Drop for ProcessWatch {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[test]
fn manager_kill_terminates_the_descendant_process_tree() {
    let manager = manager();
    let pid_file =
        std::env::temp_dir().join(format!("owo-lifecycle-pid-{}.txt", uuid::Uuid::new_v4()));
    let process = child_with_env(
        &manager,
        "tree",
        vec![(
            "OWO_LIFECYCLE_PID_FILE".into(),
            pid_file.to_string_lossy().into_owned(),
        )],
    );
    let ready = Instant::now();
    let pid = loop {
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.parse::<u32>().ok())
        {
            break pid;
        }
        if ready.elapsed() >= Duration::from_secs(5) {
            let _ = manager.lock().unwrap().kill(&process.handle);
            let result = wait_bounded(&manager, process);
            panic!(
                "controlled descendant did not start; synthetic child stdout={:?} stderr={:?}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // Retain a handle before termination so PID reuse cannot affect the observation.
    let handle = unsafe { OpenProcess(0x0010_0000, 0, pid) };
    assert!(
        !handle.is_null(),
        "cannot observe the controlled descendant"
    );
    let watch = ProcessWatch(handle);
    manager.lock().unwrap().kill(&process.handle).unwrap();
    assert_ne!(wait_bounded(&manager, process).exit_code, 0);
    assert_eq!(
        unsafe { WaitForSingleObject(watch.0, 5000) },
        0,
        "descendant survived its owned Job termination"
    );
    std::fs::remove_file(pid_file).unwrap();
}

#[test]
fn real_worker_excludes_ambient_private_values_and_accepts_host_overrides() {
    let manager = manager();
    let name = format!("OWO_SYNTHETIC_PRIVATE_{}", uuid::Uuid::new_v4().simple());
    struct RemoveEnv(String);
    impl Drop for RemoveEnv {
        fn drop(&mut self) {
            std::env::remove_var(&self.0);
        }
    }
    std::env::set_var(&name, "synthetic-private-value");
    let _cleanup = RemoveEnv(name.clone());
    let process = child_with_env(
        &manager,
        "environment",
        vec![
            ("OWO_TEST_PRIVATE_NAME".into(), name),
            ("OWO_TEST_EXPLICIT".into(), "synthetic-host-value".into()),
        ],
    );
    let result = wait_bounded(&manager, process);
    assert_eq!(result.exit_code, 0);
    assert!(String::from_utf8_lossy(&result.stdout).contains("worker-environment-boundary-ok"));
}

//! Entry point for the Windows-only simulated QQ desktop surface.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
#[path = "../windows/sim_qq.rs"]
mod windows;

#[cfg(target_os = "windows")]
fn main() {
    windows::main();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("owo-sim-qq requires Windows");
    std::process::exit(1);
}

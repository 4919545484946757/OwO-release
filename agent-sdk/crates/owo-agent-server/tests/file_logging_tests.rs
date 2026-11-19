//! §13 批次八回归：`init_file_logging` 必须自建缺失的父目录。
//!
//! 现场故障：核心的 data_root `<...>/Agent/data` 下没有 `logs/`，而 init 直接
//! `OpenOptions::open` 该路径 → Windows 返回 ERROR_PATH_NOT_FOUND（os error 3），
//! 文件日志静默降级为「仅落 stderr」，`logs/server.jsonl` 长期不再产出。
//!
//! 文件日志是进程级全局状态，本文件内两个用例共用同一把锁串行执行。

use owo_agent_server::logging::{self, init_file_logging};
use std::sync::Mutex;

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const BACKUPS: u32 = 5;

/// 串行化：FILE_LOG 是 `static Mutex<Option<FileLog>>`，用例并发会互相顶掉句柄。
static SERIAL: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn init_creates_missing_parent_dir_and_writes() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("logs").join("server.jsonl");
    assert!(
        !path.parent().expect("parent").exists(),
        "前置条件：父目录不存在"
    );

    init_file_logging(&path, MAX_BYTES, BACKUPS).expect("父目录缺失时不应失败");
    logging::info("test", None, "落盘探针");
    logging::close_file_logging();

    assert!(path.exists(), "父目录应被自建且文件已打开");
    let content = std::fs::read_to_string(&path).expect("读回日志");
    assert!(content.contains("落盘探针"), "写入内容应落盘：{content}");
}

#[test]
fn init_rotates_into_numbered_backups() {
    let _guard = lock();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("logs").join("server.jsonl");

    // 阈值压到下限（1024 字节）后连写超量内容，触发轮转。
    init_file_logging(&path, 1, BACKUPS).expect("init");
    let chunk = "x".repeat(400);
    for _ in 0..6 {
        logging::info("test", None, &chunk);
    }
    logging::close_file_logging();

    assert!(
        path.with_file_name("server.jsonl.1").exists(),
        "轮转产物 .1 应存在"
    );
}

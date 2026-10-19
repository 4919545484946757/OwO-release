// §12.3 CLI 拆分批次二：doctor 子命令域（自 main.rs 机械外移，零行为变化）。
// 环境健康诊断：数据目录/凭据来源/模型/端点/服务可达性。

use std::path::PathBuf;

use clap::Args;

/// `owo-agent doctor`：环境健康诊断（数据目录/凭据/模型/端点/服务）。
#[derive(Args)]
pub(crate) struct DoctorArgs {
    /// 数据目录（缺省 OWO_AGENT_DATA 或 %LOCALAPPDATA%\OwO\Agent，与 serve 同源；
    /// 该目录不存在时回退诊断 <workspace>/.owo-data）
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// 目标工作区（缺省当前目录）
    #[arg(long)]
    workspace: Option<PathBuf>,
}
/// 诊断项级别：warn 只提示（不影响退出码），fail 才计入失败。
#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckLevel {
    Ok,
    Warn,
    Fail,
}

/// `owo-agent doctor`：逐项环境诊断，输出 [ok]/[warn]/[fail] 清单；任一 fail 非零退出。
pub(crate) async fn run_doctor_cmd(args: DoctorArgs) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args
        .workspace
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let data_root = if let Some(dir) = args.data_dir {
        dir
    } else if let Ok(env_dir) = std::env::var("OWO_AGENT_DATA") {
        PathBuf::from(env_dir)
    } else {
        // 与 serve 同源：缺省数据根 = %LOCALAPPDATA%\OwO\Agent（serve::run_serve
        // 经 ensure_data_root 解析的正是该目录）。此前缺省诊断 <workspace>/.owo-data
        // 与运行面脱节（运行时冒烟发现：doctor 报"index.db 未初始化"而真实存储
        // 在 LOCALAPPDATA 已有数据）。.owo-data 仅作为该目录不存在时的历史遗留
        // 回退保留诊断能力。
        let preferred = crate::support::data_root(None);
        if preferred.is_dir() {
            preferred
        } else {
            workspace.join(".owo-data")
        }
    };
    let mut failures = 0usize;
    let mut checks: Vec<(&'static str, CheckLevel, String)> = Vec::new();

    // 0) 构建身份（§7.1：与 /health、--version、release manifest 同构，
    //    单一来源 owo_build_info::identity()）。正常编译必然可解析；报
    //    [fail] 意味着该二进制来自无 git 环境编译或身份覆写指向了过期
    //    产物——正是发布链要在现场抓的错误。
    let identity = owo_build_info::identity();
    let identity_ok = identity.commit != "unknown" && !identity.commit.is_empty();
    checks.push((
        "构建身份",
        if identity_ok {
            CheckLevel::Ok
        } else {
            CheckLevel::Fail
        },
        identity.oneline(),
    ));

    // 1) 数据目录与关键存储文件。
    let storage_ok = data_root.is_dir() || std::fs::create_dir_all(&data_root).is_ok();
    checks.push((
        "数据目录",
        if storage_ok {
            CheckLevel::Ok
        } else {
            CheckLevel::Fail
        },
        data_root.display().to_string(),
    ));
    let index_db = data_root.join("index.db");
    if index_db.exists() {
        let openable = owo_agent_core::sqlite_store::SqliteSessionStore::open(&index_db).is_ok();
        checks.push((
            "SQLite index.db",
            if openable {
                CheckLevel::Ok
            } else {
                CheckLevel::Fail
            },
            index_db.display().to_string(),
        ));
    } else {
        checks.push((
            "SQLite index.db",
            CheckLevel::Ok,
            "未初始化（首次运行自动创建）".to_string(),
        ));
    }

    // 2) 模型凭据与端点（缺省不视为失败，仅提示）。
    match std::env::var("OPENAI_API_KEY") {
        Ok(value) if !value.trim().is_empty() => {
            checks.push(("OPENAI_API_KEY", CheckLevel::Ok, "已配置".to_string()));
        }
        _ => {
            let base_url = std::env::var("OPENAI_BASE_URL").unwrap_or_default();
            let local = base_url.contains("127.0.0.1") || base_url.contains("localhost");
            checks.push((
                "OPENAI_API_KEY",
                if local {
                    CheckLevel::Ok
                } else {
                    CheckLevel::Fail
                },
                if local {
                    "本地端点免凭据".to_string()
                } else {
                    "未配置（云端模型不可用）".to_string()
                },
            ));
        }
    }

    // 3) 模型网关韧性配置（R9：熔断/重试参数）。
    let mut gateway_note = String::new();
    for (name, default) in [
        ("OWO_MODEL_RETRY_MAX", "3"),
        ("OWO_MODEL_CIRCUIT_THRESHOLD", "5"),
    ] {
        match std::env::var(name) {
            Ok(value) => gateway_note.push_str(&format!("{name}={value} ")),
            Err(_) => gateway_note.push_str(&format!("{name}={default}（默认） ")),
        }
    }
    checks.push((
        "模型网关韧性",
        CheckLevel::Ok,
        gateway_note.trim().to_string(),
    ));

    // 4) 服务健康（serve 冒烟：可选项）。
    match std::env::var("OWO_DOCTOR_SERVE_PORT") {
        Ok(port) => {
            let url = format!("http://127.0.0.1:{port}/health");
            let token = std::env::var("OWO_DOCTOR_SERVE_TOKEN").unwrap_or_default();
            let response = reqwest::Client::new()
                .get(&url)
                .header("authorization", format!("Bearer {token}"))
                .timeout(std::time::Duration::from_secs(3))
                .send()
                .await;
            match response {
                Ok(resp) => {
                    checks.push((
                        "本地服务 /health",
                        if resp.status().is_success() {
                            CheckLevel::Ok
                        } else {
                            CheckLevel::Fail
                        },
                        url,
                    ));
                }
                Err(error) => {
                    checks.push((
                        "本地服务 /health",
                        CheckLevel::Fail,
                        format!("{url}（{error}）"),
                    ));
                }
            }
        }
        Err(_) => {
            checks.push((
                "本地服务 /health",
                CheckLevel::Ok,
                "跳过（设置 OWO_DOCTOR_SERVE_PORT 启用）".to_string(),
            ));
        }
    }

    // 5) 备份目录可写（release 产物路径）。
    let backups = data_root.join("backups");
    let backups_ok = backups.is_dir() || std::fs::create_dir_all(&backups).is_ok();
    checks.push((
        "备份目录",
        if backups_ok {
            CheckLevel::Ok
        } else {
            CheckLevel::Fail
        },
        backups.display().to_string(),
    ));

    // 6) 工作区信任（审查 N1）：有 hooks/插件等"配置即代码"且未信任 → 受限模式。
    //    受限是安全的显式状态而非故障：记 warn，不影响退出码。
    let trust = owo_agent_core::trust::TrustStore::new(data_root.clone()).status(&workspace);
    let fingerprint = &trust.fingerprint[..trust.fingerprint.len().min(16)];
    let (trust_level, trust_note) = if trust.trusted {
        (CheckLevel::Ok, format!("已信任（指纹 {fingerprint}）"))
    } else if trust.gated {
        (
            CheckLevel::Warn,
            format!(
                "未信任（受限模式，hooks/插件/权限放宽不加载）：{}；用 `owo-agent trust` 或桌面端信任",
                trust.reasons.join("；")
            ),
        )
    } else {
        (
            CheckLevel::Ok,
            "未信任（该工作区没有会自动执行的配置，无需确认）".to_string(),
        )
    };
    checks.push(("工作区信任", trust_level, trust_note));

    for (name, level, note) in &checks {
        let mark = match level {
            CheckLevel::Ok => "[ok]  ",
            CheckLevel::Warn => "[warn]",
            CheckLevel::Fail => "[fail]",
        };
        println!("{mark} {name}：{note}");
        if *level == CheckLevel::Fail {
            failures += 1;
        }
    }
    if failures > 0 {
        println!("诊断完成：{} 项失败", failures);
        std::process::exit(1);
    }
    println!("诊断完成：全部通过");
    Ok(())
}

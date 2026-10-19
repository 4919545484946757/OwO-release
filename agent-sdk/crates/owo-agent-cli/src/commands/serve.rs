// §12.3 CLI 拆分批次 4b：serve/init 子命令域（自 main.rs 机械外移，零行为变化）。
// serve：服务生命周期（恢复/遥测/文件日志/路由/实例握手/优雅关闭）；init：项目脚手架。

use crate::support::*;
use clap::Args;
use colored::Colorize;
use owo_agent_core::{install_builtin_packages, Settings, SkillRegistry, SqliteSessionStore};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Args)]
pub(crate) struct ServeArgs {
    /// 监听端口（0 = 由系统分配临时端口；实际端口经 stdout 的 core_ready 行上报）
    #[arg(long, default_value_t = 4096)]
    port: u16,
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
}

/// R3-B（§3.4 契约）：启动期致命错误以恰好一行 `core_fatal` JSON 上报到 stdout
/// （桌面壳解析并映射为稳定错误码，UI 呈现对应恢复动作），随后非零退出。
/// 消息**必须脱敏**：只带目录路径与 OS 错误文本，绝不携带凭据/环境变量值。
fn emit_core_fatal(code: &str, message: &str) -> ! {
    println!(
        "{}",
        serde_json::json!({ "event": "core_fatal", "code": code, "message": message })
    );
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    std::process::exit(3);
}

pub(crate) async fn run_serve(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args.workspace.canonicalize()?;
    let settings = Settings::load(&workspace);
    apply_egress_setting(&settings);
    settings.apply_usage_env();
    settings.apply_reasoning_env();
    let model = resolve_model(None, settings.model.as_deref());
    // DeferredProvider（取优合并自远端 engine）每次调用前重读 OPENAI_MODEL；把启动
    // 解析出的模型写回进程环境，使保存的 settings.model 在重启后也生效（设置页保存
    // 时同样写该变量，模型对新回合即时生效）。
    std::env::set_var("OPENAI_MODEL", &model);
    // R3-B（§3.4 `storage/not_writable`）：桌面壳上下文（实例身份由壳注入）绝不允许
    // 静默把数据根迁移到工作区 `.owo-agent`——用户必须看到存储错误并"更换数据目录"。
    // 非桌面 CLI 保持既有回退行为不变。
    let desktop_ctx = std::env::var_os("OWO_DESKTOP_INSTANCE_ID").is_some();
    let root = if desktop_ctx {
        match ensure_data_root_checked(None) {
            Ok(root) => root,
            Err(reason) => emit_core_fatal("storage/not_writable", &reason),
        }
    } else {
        ensure_data_root(None, &workspace)
    };
    let plugin_state = owo_agent_core::PluginStateStore::new(Some(root.join("plugin_state.json")));
    // 审查 N1：工作区信任门。未信任的工作区不加载插件（插件 manifest 可自带命令/
    // MCP，会在启动时拉起子进程）——信任经桌面端弹窗/`POST /workspace/trust`/设置页保存。
    let trust = owo_agent_core::trust::TrustStore::new(root.clone()).status(&workspace);
    let trust_gated = !trust.trusted && trust.gated;
    if trust_gated {
        tracing::warn!(
            "工作区未信任，跳过工作区插件加载：{}",
            trust.reasons.join("；")
        );
    }
    let plugins = if trust_gated {
        Vec::new()
    } else {
        owo_agent_core::plugin::discover_enabled_plugins(&workspace, &root, &plugin_state)
    };
    let mut mcp_configs = load_mcp_configs(&root);
    merge_plugin_mcp(&plugins, &mut mcp_configs);
    // MCP 是可选扩展，握手不能占用 Daemon 的 ready 关键路径。
    // 成功后通过 Agent 的线程安全注册表热挂工具；服务仍先对本地客户端可用。
    match install_builtin_packages(&builtin_skills_root(), &root, env!("CARGO_PKG_VERSION")) {
        Ok(report) => {
            for (name, reason) in &report.skipped {
                tracing::warn!("内置技能 {name} 未安装：{reason}");
            }
        }
        Err(error) => tracing::warn!("内置技能安装失败：{error}"),
    }
    let mut skills = SkillRegistry::discover(&workspace, &root);
    apply_disabled_skills(&mut skills, &settings);
    let agent = build_agent_with_mcp_serve(
        &workspace,
        &model,
        settings.read_only,
        &[],
        &skills,
        &settings.deny_commands,
    )?;
    let store = match SqliteSessionStore::open(&root.join("index.db")) {
        Ok(store) => store,
        Err(error) => {
            if desktop_ctx {
                emit_core_fatal(
                    "storage/not_writable",
                    &format!("会话库不可打开：{}（{error}）", display_path(&root)),
                )
            }
            return Err(error.into());
        }
    };
    let state = Arc::new(owo_agent_server::AppState::new(
        agent,
        store,
        root.join("traces"),
        root.clone(),
        workspace.clone(),
    ));
    // R8：强杀恢复——陈旧 pid 文件清理；检测到存活实例则显式拒绝双开。
    //
    // R10（2026-09-22 桌面端"连不上"根因）：桌面壳换工作区/换提供商时会先请旧核心
    // 优雅退出、再立刻拉起新核心，两者必然有毫秒到数秒的重叠；旧实例在途回合
    // drain 上限是 30s，期间 pid 文件一直存在。此前这里**一次判死**，于是壳拉起的
    // 新核心秒退，日志留下 Error: 检测到运行中的服务（pid=…），用户看到的是
    // "服务明明在跑、界面却连不上"。桌面上下文下改为有界等待上一代退出再继续：
    // 这不是放松双开保护（CLI 语义不变），而是把"上一代正在退"识别成等待而非冲突。
    let mut recovery = owo_agent_server::shutdown::recover_force_kill(&root);
    if desktop_ctx {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(40);
        while let Err(ref reason) = recovery {
            if reason.contains("检测到运行中的服务") && std::time::Instant::now() < deadline
            {
                tracing::warn!("上一代核心仍在退出（{reason}），等待其退出后继续启动");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                recovery = owo_agent_server::shutdown::recover_force_kill(&root);
                continue;
            }
            if reason.contains("检测到运行中的服务") {
                tracing::error!("等待上一代核心退出超时（40s），按双开冲突拒绝启动");
            }
            break;
        }
    }
    if let Some(recovery) = recovery? {
        tracing::warn!(
            "检测到强杀残留（pid={:?}），已清理 pid 文件并恢复干净状态",
            recovery.stale_pid
        );
    }
    let _pid_file = owo_agent_server::shutdown::PidFile::create(&root)?;
    // §13 批次三（R10 持久化组接线）：启动恢复用量快照（records/budgets/硬熔断）。
    let restored_usage = owo_agent_server::restore_usage_snapshot(&root);
    if restored_usage > 0 {
        tracing::info!("用量快照已恢复：{restored_usage} 条记录（崩溃后续接）");
    }
    // §13 批次六（遥测接线）：应用 settings.json 遥测开关（默认关）。
    owo_agent_server::apply_telemetry_setting(settings.telemetry_enabled == Some(true));
    // §13 批次八（R10 文件日志接线）：轮转落盘 + 生命周期审计事件。
    owo_agent_server::init_server_file_logging(&root);
    owo_agent_server::logging_lifecycle_audit("server_start", "服务启动完成（恢复/开关应用后）");
    // R8：优雅关闭接线——停止接收 → 完成在途 → flush 审计 → 用量落盘 → 清理 pid → 退出。
    let shutdown_state = Arc::clone(&state);
    let shutdown_root = root.clone();
    tokio::spawn(async move {
        let gate = Arc::clone(&shutdown_state.shutdown_gate);
        gate.wait_shutdown_request().await;
        tracing::info!("收到关闭请求：等待在途回合完成（上限 30s）");
        let remaining = gate.await_drain(std::time::Duration::from_secs(30)).await;
        if remaining > 0 {
            tracing::warn!("仍有 {remaining} 个在途回合超时未完成，强制执行退出");
        }
        owo_agent_server::flush_audit(&shutdown_state);
        if let Some(path) = owo_agent_server::persist_usage_snapshot(&shutdown_root) {
            tracing::info!("用量快照已落盘：{}", path.display());
        }
        owo_agent_server::logging_lifecycle_audit("server_stop", "服务优雅关闭（审计/用量落盘后）");
        owo_agent_server::close_server_file_logging();
        // process::exit 不执行 Drop，显式清理 pid 文件（强杀残留仍由 recover_force_kill 兜底）。
        let _ = std::fs::remove_file(shutdown_root.join("server.pid"));
        // §2.3：同样显式清理发现文件，避免留下"指向已退出实例"的陈旧发现。
        let _ = std::fs::remove_file(owo_agent_server::discovery::descriptor_path(&shutdown_root));
        tracing::info!("审计已 flush，服务退出");
        std::process::exit(0);
    });
    // 启动时同步插件禁用状态到 Agent 工具前缀（热卸载重启后仍生效）。
    if let Ok(plugin_state) = state.plugin_state.lock() {
        for id in plugin_state.disabled_ids() {
            let prefix = owo_agent_core::tools::mcp_tool_prefix(&id);
            state.agent.set_tool_prefix_enabled(&prefix, false);
        }
    }
    let observer_state = Arc::clone(&state);
    tokio::spawn(async move {
        owo_agent_server::start_observer(observer_state).await;
    });
    let automation_state = Arc::clone(&state);
    tokio::spawn(async move {
        owo_agent_server::start_automation_loop(automation_state).await;
    });
    let memory_state = Arc::clone(&state);
    tokio::spawn(async move {
        owo_agent_server::start_memory_observer(memory_state).await;
    });
    // §13 批次三：用量定时落盘循环（每小时；与 R10 恢复/关闭落盘三面闭环）。
    let usage_persist_state = Arc::clone(&state);
    tokio::spawn(async move {
        owo_agent_server::start_usage_persistence_loop(usage_persist_state).await;
    });
    let app = owo_agent_server::build_router(Arc::clone(&state));
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], args.port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    tracing::info!("owo-agent server listening on http://{bound}");
    // §4.2 实例握手：路由/数据库/凭据初始化完成且端口已绑定后，向 stdout 打印
    // 恰好一行 core_ready JSON。桌面壳以该行（而非"TCP 能连"）判定服务就绪，
    // 并据此取得实际端口（--port 0 时由系统分配）。stdout 其他内容不受影响。
    println!(
        "{}",
        serde_json::json!({
            "event": "core_ready",
            "pid": std::process::id(),
            "port": bound.port(),
            "api_version": owo_agent_server::OWO_API_VERSION,
            "build_id": resolve_build_id(),
            "instance_id": std::env::var("OWO_DESKTOP_INSTANCE_ID")
                .unwrap_or_default()
                .trim()
                .to_string(),
        })
    );
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    // §2.3：端口绑定后原子写入 `<data_root>/runtime/daemon.json`，让 CLI/TUI/桌面壳
    // 能发现并复用**同一** Daemon（而不是各自另起实例）。句柄存活到服务退出；
    // 优雅关闭路径另行显式删除（process::exit 不跑 Drop）。
    let descriptor = owo_agent_protocol::DaemonDescriptor {
        pid: std::process::id(),
        port: bound.port(),
        instance_id: std::env::var("OWO_DESKTOP_INSTANCE_ID")
            .unwrap_or_default()
            .trim()
            .to_string(),
        api_version: owo_agent_server::OWO_API_VERSION.to_string(),
        build_id: resolve_build_id(),
        started_at: owo_agent_server::discovery::now_rfc3339(),
        data_root: owo_agent_server::discovery::mask_data_root(&root),
    };
    let _discovery_file = match owo_agent_server::discovery::write_descriptor(&root, &descriptor) {
        Ok(file) => Some(file),
        Err(error) => {
            tracing::warn!("写入发现文件失败（客户端将无法自动发现本实例）：{error}");
            None
        }
    };
    let mcp_initialization = spawn_mcp_connections(
        Arc::clone(&state.agent),
        mcp_configs,
        Arc::clone(&state.plugin_state),
    );
    let result = axum::serve(listener, app).await;
    // 服务退出时取消尚未完成的 MCP 握手，再终止已连接的 stdio 子进程。
    if let Some(task) = mcp_initialization {
        task.abort();
        let _ = task.await;
    }
    // 服务退出：终止全部 MCP stdio 子进程，不留孤儿进程。
    let shutdown_errors = state.agent.shutdown_all_mcp().await;
    for (name, error) in shutdown_errors {
        tracing::warn!("MCP 服务器 {name} 关闭失败：{error}");
    }
    result?;
    Ok(())
}

#[derive(Args)]
pub(crate) struct InitArgs {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// 已存在时覆盖
    #[arg(long)]
    force: bool,
}

pub(crate) fn run_init(args: InitArgs) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = args.workspace.canonicalize()?;
    let target = workspace.join("AGENTS.md");
    if target.exists() && !args.force {
        println!("{} {}", "AGENTS.md 已存在".yellow(), target.display());
        println!("如需覆盖请使用 --force");
        return Ok(());
    }
    std::fs::write(&target, AGENTS_TEMPLATE)?;
    println!("{} {}", "已生成".green(), target.display());
    Ok(())
}

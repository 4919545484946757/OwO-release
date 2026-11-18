//! `owo-agent product-eval`（V1-R1）：产品级评测底座 CLI。
//!
//! - `validate`：10 个任务定义 schema/参考输出/权限范围全量校验；
//! - `preflight`：live 基线环境预检（Provider/凭据存在性/suite/输出目录/磁盘/ORT），
//!   只输出"已配置/未配置"，绝不输出凭据内容；
//! - `run`：执行评测矩阵（dry 参考回放 = 本地确定性；live = 网关生成文件块；
//!   agent = 真实单 Agent：独立 Session + 最小受控工具集 + 任务级权限审批），
//!   journal 断点续跑、失败运行永久进报告；
//! - `compare`：对照两份报告（单/多 Agent、不同批次），失败记录不参与任何剔除。

use clap::{Args, Subcommand};
use owo_agent_eval_facade::external_drivers::ExternalDriverRegistry;
use owo_agent_eval_facade::product_eval::{
    build_freeze_json, build_live_provider, build_paired_report_json, compare_reports,
    format_mode_statistics, format_report_summary, format_validation, load_report, load_suite,
    resolve_suite_input, verify_freeze, AgentMode, ArtifactChecker, EvalCategory,
    GenerativeExecutor, MatrixRunner, PairedReportOptions, ReferenceDryExecutor, RunOptions,
    SingleAgentExecutor,
};
use owo_agent_eval_facade::product_eval_workswarm::WorkSwarmExecutor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[derive(Args)]
pub struct ProductEvalArgs {
    #[command(subcommand)]
    pub action: ProductEvalAction,
}

#[derive(Subcommand)]
pub enum ProductEvalAction {
    /// 校验套件与全部任务定义（schema / 参考输出可达 / 权限范围）
    Validate {
        /// suite.json 路径（缺省向上查找 agent-sdk/evals/v1/suite.json）
        #[arg(long)]
        suite: Option<PathBuf>,
        /// 同时校验与 freeze.json 的冻结一致性（任务输入/检查器/权限/预算/模型/版本哈希）
        #[arg(long)]
        freeze: Option<PathBuf>,
    },
    /// 冻结当前套件为 freeze.json（正式验收前必须冻结；版本哈希取自 git）
    Freeze {
        /// suite.json 路径（缺省向上查找 agent-sdk/evals/v1/suite.json）
        #[arg(long)]
        suite: Option<PathBuf>,
        /// 输出路径（缺省 <suite 目录>/freeze.json）
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// 执行评测矩阵（断点续跑；--exec dry 为本地确定性回放）
    Run {
        /// suite.json 路径（缺省向上查找 agent-sdk/evals/v1/suite.json）
        #[arg(long)]
        suite: Option<PathBuf>,
        /// 执行模式：dry（参考回放，零模型调用）| live（网关生成文件块）
        /// | agent（真实单 Agent）| workswarm（真实 WorkSwarm TeamRun）
        #[arg(long, default_value = "dry")]
        exec: String,
        /// 对照拓扑：single,multi
        #[arg(long, default_value = "single,multi")]
        agents: String,
        /// 覆盖重复次数（缺省用任务/套件定义，V1 目标 20）
        #[arg(long)]
        reps: Option<u32>,
        /// 只跑 id 包含该子串的任务
        #[arg(long)]
        only: Option<String>,
        /// 只跑指定分类：code|research|document
        #[arg(long)]
        category: Option<String>,
        /// 输出目录（缺省 <agent-sdk>/scratch-eval-runs/product-eval）
        #[arg(long)]
        out: Option<PathBuf>,
        /// 模型覆盖（缺省 OPENAI_MODEL / 内置 GLM 默认）
        #[arg(long)]
        model: Option<String>,
        /// 组队模式（仅 exec=workswarm 生效）：single|team|auto，缺省 auto
        #[arg(long)]
        team_mode: Option<String>,
        /// 批次标签：写入 meta/报告；同一 out 目录只允许同一批次（防跨批次混算）
        #[arg(long)]
        label: Option<String>,
        /// 附加溯源标签（可重复出现多次）
        #[arg(long)]
        tag: Vec<String>,
        /// 清空输出目录重跑（唯一允许的归零入口）
        #[arg(long)]
        fresh: bool,
        /// 验证/修复组：检查器未通过时最多追加的修复次数（0 = 单次执行）
        #[arg(long, default_value_t = 0)]
        repair_attempts: u32,
        /// 显式注册真实桌面驱动；未注册或运行失败绝不生成正式通过结果。
        #[arg(long)]
        desktop_driver: Option<PathBuf>,
        /// 显式注册真实故障注入驱动；未注册或运行失败绝不生成正式通过结果。
        #[arg(long)]
        fault_driver: Option<PathBuf>,
        /// 外部驱动配置和证据根（证据写入该文件的父目录）。
        #[arg(long)]
        driver_config: Option<PathBuf>,
    },
    /// live 基线环境预检（不泄露凭据内容；exit 0=就绪，2=存在阻塞项）
    Preflight {
        /// suite.json 路径（缺省向上查找 agent-sdk/evals/v1/suite.json）
        #[arg(long)]
        suite: Option<PathBuf>,
        /// 输出目录（检查可写性；缺省 <agent-sdk>/scratch-eval-runs/product-eval）
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// 对照两份报告
    Compare {
        report_a: PathBuf,
        report_b: PathBuf,
        /// 输出格式：text|json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// 由单/多两份报告生成「配对对照报告」（三路收益判定可直接读取的 PairedStats JSON）
    Paired {
        /// 单 Agent 报告 report.json
        single: PathBuf,
        /// 多 Agent 报告 report.json
        multi: PathBuf,
        /// 输出 paired JSON 路径
        #[arg(long)]
        out: PathBuf,
        /// 模型绑定（缺省取 single 报告的 model）
        #[arg(long)]
        model: Option<String>,
        /// 模板绑定（多 Agent 实际组队模板；由三路冻结）
        #[arg(long)]
        template: Option<String>,
        /// 任务集绑定（缺省取 single 报告的 suite_name）
        #[arg(long)]
        task_set: Option<String>,
        /// 策略版本（与 team-policy.json 的 strategy_version 一致，由三路冻结）
        #[arg(long, default_value = "ten-5-independent-case-samples")]
        strategy_version: String,
    },
}

/// 入口（由 main.rs 的 Commands::ProductEval 分发）。
pub async fn run(args: ProductEvalArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.action {
        ProductEvalAction::Validate { suite, freeze } => run_validate(suite, freeze),
        ProductEvalAction::Freeze { suite, out } => run_freeze(suite, out),
        ProductEvalAction::Preflight { suite, out } => run_preflight(suite, out),
        ProductEvalAction::Run {
            suite,
            exec,
            agents,
            reps,
            only,
            category,
            out,
            model,
            team_mode,
            label,
            tag,
            fresh,
            repair_attempts,
            desktop_driver,
            fault_driver,
            driver_config,
        } => {
            let selection = match team_mode.as_deref() {
                None => owo_agent_core::team_strategy::TeamSelectionMode::default(),
                Some(raw) => owo_agent_core::team_strategy::TeamSelectionMode::parse(raw)?,
            };
            run_matrix_cmd(
                suite,
                exec,
                agents,
                reps,
                only,
                category,
                out,
                model,
                selection,
                label,
                tag,
                fresh,
                repair_attempts,
                desktop_driver,
                fault_driver,
                driver_config,
            )
            .await
        }
        ProductEvalAction::Compare {
            report_a,
            report_b,
            format,
        } => run_compare(report_a, report_b, format),
        ProductEvalAction::Paired {
            single,
            multi,
            out,
            model,
            template,
            task_set,
            strategy_version,
        } => run_paired(
            single,
            multi,
            out,
            model,
            template,
            task_set,
            strategy_version,
        ),
    }
}

fn run_validate(
    suite: Option<PathBuf>,
    freeze: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let suite_path = resolve_suite_input(suite.as_deref())?;
    let bundle = load_suite(&suite_path)?;
    println!("套件：{}（{}）", bundle.suite.name, suite_path.display());
    let validation = owo_agent_eval_facade::product_eval::validate_suite(&bundle);
    print!("{}", format_validation(&validation));
    let mut all_ok = validation.all_ok;
    // 冻结一致性校验：正式验收前必须通过（任务输入/检查器/权限/预算/模型/版本哈希未漂移）。
    if let Some(freeze_path) = &freeze {
        let text = match std::fs::read_to_string(freeze_path) {
            Ok(text) => text,
            Err(error) => {
                println!("读取 freeze.json {} 失败：{error}", freeze_path.display());
                std::process::exit(1);
            }
        };
        let freeze_json = owo_agent_eval_facade::product_eval::parse_freeze(&text)?;
        let default_model = crate::support::resolve_model(None, None);
        let issues = verify_freeze(&bundle, &freeze_json, Some(&default_model));
        if issues.is_empty() {
            println!("freeze 一致性：通过（{}）", freeze_path.display());
        } else {
            all_ok = false;
            println!("freeze 一致性：失败（冻结被破坏或版本漂移）");
            for issue in &issues {
                println!("    - {issue}");
            }
        }
    }
    if !all_ok {
        std::process::exit(1);
    }
    Ok(())
}

/// 冻结：写出 freeze.json（任务文件哈希 + 生效预算 + 权限哈希 + 模型配置 + git 版本）。
fn run_freeze(
    suite: Option<PathBuf>,
    out: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let suite_path = resolve_suite_input(suite.as_deref())?;
    let bundle = load_suite(&suite_path)?;
    let validation = owo_agent_eval_facade::product_eval::validate_suite(&bundle);
    if !validation.all_ok {
        print!("{}", format_validation(&validation));
        return Err("套件校验未通过：禁止冻结存在问题的任务集".into());
    }
    let model = crate::support::resolve_model(None, None);
    let base_url = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| owo_agent_core::gateway::DEFAULT_MODEL_BASE_URL.to_string());
    // git 版本哈希（从套件目录向上找仓库根）。
    let git_dir = repo_root_of(&suite_path).unwrap_or_else(|| suite_path.to_path_buf());
    let git_commit = run_git(&git_dir, &["rev-parse", "HEAD"]).ok();
    let git_dirty = run_git(&git_dir, &["status", "--porcelain"])
        .map(|output| !output.trim().is_empty())
        .unwrap_or(true);
    let freeze_json = build_freeze_json(
        &bundle,
        Some(&model),
        Some(&base_url),
        git_commit.as_deref(),
        Some(git_dirty),
        None,
    )?;
    let out_path = match out {
        Some(path) => path,
        None => suite_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("freeze.json"),
    };
    let text = serde_json::to_string_pretty(&freeze_json)
        .map_err(|e| format!("序列化 freeze.json 失败：{e}"))?;
    std::fs::write(&out_path, text + "\n")
        .map_err(|e| format!("写入 {} 失败：{e}", out_path.display()))?;
    println!(
        "已冻结：{}（{} 任务；模型 {model}；git={} dirty={git_dirty}）",
        out_path.display(),
        bundle.cases.len(),
        git_commit.as_deref().unwrap_or("未知")
    );
    println!("请复核 freeze.json 后提交；正式验收前用 `product-eval validate --freeze <路径>` 校验一致性。");
    Ok(())
}

fn repo_root_of(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    for _ in 0..10 {
        if current.join(".git").exists() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git 无法执行：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} 失败（stderr: {}）",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// 生成配对对照报告（三路收益判定入口；PairedStats 兼容形状）。
fn run_paired(
    single: PathBuf,
    multi: PathBuf,
    out: PathBuf,
    model: Option<String>,
    template: Option<String>,
    task_set: Option<String>,
    strategy_version: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let single_report = load_report(&single)?;
    let multi_report = load_report(&multi)?;
    let model = model.or_else(|| single_report.model.clone());
    let task_set = task_set.or_else(|| Some(single_report.suite_name.clone()));
    if single_report.suite_hash != multi_report.suite_hash {
        println!(
            "⚠️ 两份报告 suite_hash 不一致（{} vs {}）：任务定义已变更，配对对照仅具参考意义",
            single_report.suite_hash, multi_report.suite_hash
        );
    }
    let opts = PairedReportOptions {
        model,
        template,
        task_set,
        strategy_version,
    };
    let paired = build_paired_report_json(&single_report, &multi_report, &opts, None);
    let text =
        serde_json::to_string_pretty(&paired).map_err(|e| format!("序列化配对报告失败：{e}"))?;
    std::fs::write(&out, text + "\n").map_err(|e| format!("写入 {} 失败：{e}", out.display()))?;
    println!("配对对照报告：{}", out.display());
    let alignment = paired.get("run_alignment");
    let aligned = alignment
        .and_then(|value| value.get("configuration_aligned"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if aligned {
        println!("真实执行器、任务/权限/检查器/预算指纹、评测器二进制、模型与批次对齐；Git 源码版本和模型端点仍需从 freeze 元数据核对。");
    } else {
        let reasons = alignment
            .and_then(|value| value.get("reasons"))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join("；")
            })
            .unwrap_or_else(|| "缺少对齐信息".to_string());
        println!("配对对照不可用于收益放行：{reasons}");
    }
    println!(
        "包含 {} 个任务组（overall/分类/每 case）；请三路按 PairedStats 契约读取。",
        paired
            .get("pairs")
            .and_then(|p| p.as_array())
            .map(|p| p.len())
            .unwrap_or(0)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// preflight：live 基线环境预检（只报"已配置/未配置"，绝不输出凭据内容）
// ---------------------------------------------------------------------------

/// 在仓库本地缓存中探测 onnxruntime.lib（ort crate 链接期需要 ORT_LIB_PATH）。
/// 只探测不写入：返回第一个命中目录。
fn probe_ort_lib_dir() -> Option<PathBuf> {
    let sdk_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)?;
    let prebuilt = sdk_root.join("target").join("sherpa-onnx-prebuilt");
    let mut hits: Vec<PathBuf> = Vec::new();
    collect_lib_hits(&prebuilt, 0, &mut hits);
    hits.into_iter().next()
}

fn collect_lib_hits(dir: &Path, depth: usize, hits: &mut Vec<PathBuf>) {
    if depth > 4 || !hits.is_empty() {
        return;
    }
    let Ok(reader) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in reader.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_lib_hits(&path, depth + 1, hits);
        } else if path
            .file_name()
            .is_some_and(|name| name == "onnxruntime.lib")
        {
            if let Some(parent) = path.parent() {
                hits.push(parent.to_path_buf());
            }
        }
        if !hits.is_empty() {
            break;
        }
    }
}

/// Windows 下查询卷剩余空间（字节）；非 Windows 或失败返回 None。
#[cfg(windows)]
fn disk_free_bytes(path: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free: u64 = 0;
    let mut _total: u64 = 0;
    let mut _total_free: u64 = 0;
    // SAFETY：缓冲区与指针均为本函数局部有效分配，系统调用只读路径写入计数。
    unsafe {
        if GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut _total, &mut _total_free) != 0 {
            Some(free)
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn disk_free_bytes(_path: &Path) -> Option<u64> {
    None
}

fn run_preflight(
    suite: Option<PathBuf>,
    out: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut blocked = 0usize;
    println!("== product-eval preflight（只报已配置/未配置，绝不输出凭据内容）==");

    // 1) Provider 端点与模型（非敏感，可直接展示）。
    let base_url = std::env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| owo_agent_core::gateway::DEFAULT_MODEL_BASE_URL.to_string());
    let model = std::env::var("OPENAI_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| owo_agent_core::gateway::DEFAULT_MODEL_ID.to_string());
    println!("① Provider URL：已配置（{base_url}）");
    println!("② 模型名：已配置（{model}）");

    // 2) 凭据存在性（只看存在与长度）。
    let key_len = std::env::var("OPENAI_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().count());
    match key_len {
        Some(len) => println!("③ OPENAI_API_KEY：已配置（长度 {len}，内容不展示）"),
        None => {
            println!("③ OPENAI_API_KEY：未配置 → live/agent/workswarm 阻塞（dry 不受影响）");
            blocked += 1;
        }
    }

    // 3) suite 与任务定义。
    let suite_loaded = resolve_suite_input(suite.as_deref())
        .and_then(|suite_path| load_suite(&suite_path).map(|bundle| (suite_path, bundle)));
    let mut needs_command_check = false;
    match &suite_loaded {
        Ok((suite_path, bundle)) => {
            let validation = owo_agent_eval_facade::product_eval::validate_suite(bundle);
            println!(
                "④ 套件：已加载（{}，{} 任务，校验 {}；{}）",
                bundle.suite.name,
                bundle.cases.len(),
                if validation.all_ok {
                    "全部通过"
                } else {
                    "存在问题"
                },
                suite_path.display()
            );
            if !validation.all_ok {
                blocked += 1;
            }
            needs_command_check = bundle.cases.iter().any(|case| {
                case.checkers
                    .iter()
                    .any(|checker| matches!(checker, ArtifactChecker::CommandCheck { .. }))
            });
        }
        Err(error) => {
            println!("④ 套件：加载失败（{error}；如为 suite 定位失败请用 --suite 指定）");
            blocked += 1;
        }
    }

    // 3.5) 行为检查器依赖：command_check 需要可执行的 python 运行时。
    if needs_command_check {
        let python_ok = std::process::Command::new("python")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        if python_ok {
            println!("⑥ 行为检查器依赖：python 运行时可用（command_check 可执行）");
        } else {
            println!(
                "⑥ 行为检查器依赖：python 运行时不可用 → command_check 类任务阻塞（agent 模式需 python 解释器）"
            );
            blocked += 1;
        }
    }

    // 4) 输出目录可写性 + 磁盘剩余空间。
    let out_dir = out.unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(|sdk| sdk.join("scratch-eval-runs").join("product-eval"))
            .unwrap_or_else(|| std::env::temp_dir().join("owo-product-eval"))
    });
    match std::fs::create_dir_all(&out_dir) {
        Ok(()) => {
            let probe = out_dir.join(".preflight-probe");
            let writable = std::fs::write(&probe, b"ok").is_ok();
            let _ = std::fs::remove_file(&probe);
            if writable {
                let free = disk_free_bytes(&out_dir);
                match free {
                    Some(bytes) => println!(
                        "⑤ 输出目录：可写（{}）磁盘剩余 {:.1} GB",
                        out_dir.display(),
                        bytes as f64 / 1024.0 / 1024.0 / 1024.0
                    ),
                    None => println!(
                        "⑤ 输出目录：可写（{}）磁盘剩余空间：查询不可用",
                        out_dir.display()
                    ),
                }
            } else {
                println!("⑤ 输出目录：不可写（{}）", out_dir.display());
                blocked += 1;
            }
        }
        Err(error) => {
            println!("⑤ 输出目录：创建失败（{}）：{error}", out_dir.display());
            blocked += 1;
        }
    }

    // 5) ORT 链接环境（Rust 测试在 ort crate 静态链接时需要 ORT_LIB_PATH）。
    match std::env::var("ORT_LIB_PATH") {
        Ok(value) if !value.trim().is_empty() => {
            let exists = Path::new(value.trim()).join("onnxruntime.lib").exists();
            println!(
                "⑦ ORT_LIB_PATH：已配置（{}）",
                if exists { "目录含 onnxruntime.lib" } else { "⚠️ 目录中未见 onnxruntime.lib" }
            );
        }
        _ => match probe_ort_lib_dir() {
            Some(found) => println!(
                "⑦ ORT_LIB_PATH：未设置（仓库缓存自动探测可用：{}；跑 check-v1-r0.ps1 会自动注入当前进程）",
                found.display()
            ),
            None => {
                println!("⑦ ORT_LIB_PATH：未设置且仓库缓存中未找到 onnxruntime.lib → Rust 测试链接可能失败");
                blocked += 1;
            }
        },
    }

    if blocked == 0 {
        println!("preflight：全部就绪（exit 0）");
        Ok(())
    } else {
        println!("preflight：{blocked} 项阻塞（exit 2）——live 基线在阻塞解除前不会执行，也不会用 reference 结果代替");
        std::process::exit(2);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_matrix_cmd(
    suite: Option<PathBuf>,
    exec: String,
    agents: String,
    reps: Option<u32>,
    only: Option<String>,
    category: Option<String>,
    out: Option<PathBuf>,
    model: Option<String>,
    selection: owo_agent_core::team_strategy::TeamSelectionMode,
    label: Option<String>,
    tag: Vec<String>,
    fresh: bool,
    repair_attempts: u32,
    desktop_driver: Option<PathBuf>,
    fault_driver: Option<PathBuf>,
    driver_config: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let suite_path = resolve_suite_input(suite.as_deref())?;
    let bundle = load_suite(&suite_path)?;
    println!("套件：{}（{}）", bundle.suite.name, suite_path.display());

    let modes = AgentMode::parse_list(&agents)?;
    let category = match category.as_deref() {
        None => None,
        Some(raw) => Some(EvalCategory::parse(raw)?),
    };
    if !matches!(exec.as_str(), "dry" | "live" | "agent" | "workswarm") {
        return Err(format!("未知执行模式「{exec}」（可选 dry|live|agent|workswarm）").into());
    }
    if exec == "dry"
        && (desktop_driver.is_some() || fault_driver.is_some() || driver_config.is_some())
    {
        return Err("dry-reference 不得注册外部驱动；正式驱动结果只能由 live 执行产生".into());
    }
    let external_drivers = ExternalDriverRegistry::new(
        desktop_driver.clone(),
        fault_driver.clone(),
        driver_config.clone(),
    )?;

    // 缺省输出目录：<agent-sdk>/scratch-eval-runs/product-eval（scratch-* 已被 gitignore）。
    let out_dir = out.unwrap_or_else(|| {
        bundle
            .dir
            .parent()
            .and_then(|evals| evals.parent())
            .map(|agent_sdk| agent_sdk.join("scratch-eval-runs").join("product-eval"))
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join("scratch-eval-runs")
                    .join("product-eval")
            })
    });
    println!("输出目录：{}（{}）", out_dir.display(), exec);

    let (executor, execution, model_label, provider_endpoint_sha256): (
        Arc<dyn owo_agent_eval_facade::product_eval::CaseExecutor>,
        String,
        Option<String>,
        Option<String>,
    ) = if exec == "live" {
        let (provider, resolved_model, endpoint_sha256) = build_live_provider(model.as_deref())?;
        (
            Arc::new(GenerativeExecutor {
                provider,
                model: resolved_model.clone(),
            }),
            "live-generative".to_string(),
            Some(resolved_model),
            Some(endpoint_sha256),
        )
    } else if exec == "agent" {
        let (provider, resolved_model, endpoint_sha256) = build_live_provider(model.as_deref())?;
        (
            Arc::new(SingleAgentExecutor::new(provider, resolved_model.clone())),
            "live-agent".to_string(),
            Some(resolved_model),
            Some(endpoint_sha256),
        )
    } else if exec == "workswarm" {
        let (provider, resolved_model, endpoint_sha256) = build_live_provider(model.as_deref())?;
        // 每个单元格独立 TeamRun 工作目录（CAS/sqlite/状态互不串扰）。
        let work_root = out_dir.join("workswarm-teams");
        let mut ws_executor = WorkSwarmExecutor::new(provider, resolved_model.clone(), work_root);
        ws_executor.config.selection = selection;
        println!(
            "组队模式：{}（auto=自适应判定：简单任务单 Agent，多来源合并/评审才组队）",
            selection.as_str()
        );
        (
            Arc::new(ws_executor),
            "live-workswarm".to_string(),
            Some(resolved_model),
            Some(endpoint_sha256),
        )
    } else {
        (
            Arc::new(ReferenceDryExecutor),
            "dry-reference".to_string(),
            Some(crate::support::resolve_model(model, None)),
            None,
        )
    };

    let runner = MatrixRunner::new(bundle, out_dir.clone()).with_external_drivers(external_drivers);
    let opts = RunOptions {
        modes,
        reps_override: reps,
        only,
        category,
        fresh,
        batch_label: label,
        provider_endpoint_sha256,
        tags: tag,
        repair_attempts,
        desktop_driver,
        fault_driver,
        driver_config,
    };
    let cancel = Arc::new(AtomicBool::new(false));

    let run_future = runner.run(
        executor,
        &execution,
        model_label.clone(),
        &opts,
        Arc::clone(&cancel),
    );
    tokio::pin!(run_future);
    let report = tokio::select! {
        result = &mut run_future => result?,
        _ = tokio::signal::ctrl_c() => {
            println!("\n收到 Ctrl-C：停止调度后续单元格，保留已完成记录（重跑同一命令即可续跑）…");
            cancel.store(true, Ordering::Relaxed);
            run_future.await?
        }
    };

    print!("{}", format_report_summary(&report));
    // R1 live 基线统计：95% Wilson 置信区间 / p50-p95 / 单多对照与启用条件。
    print!("{}", format_mode_statistics(&report.runs));
    println!("报告：{}", out_dir.join("report.json").display());
    println!("journal：{}", out_dir.join("state.jsonl").display());

    // dry 是校验性运行：未全绿按失败退出，供 CI 门禁使用；live 是测量性运行，恒 0。
    if execution == "dry-reference" && report.metrics.success_rate < 1.0 {
        std::process::exit(1);
    }
    Ok(())
}

fn run_compare(
    report_a: PathBuf,
    report_b: PathBuf,
    format: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let a = load_report(&report_a)?;
    let b = load_report(&report_b)?;
    let as_json = match format.as_str() {
        "json" => true,
        "text" => false,
        other => return Err(format!("未知输出格式「{other}」（可选 text|json）").into()),
    };
    println!("{}", compare_reports(&a, &b, as_json));
    Ok(())
}

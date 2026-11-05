#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use vproxy::api::AppState;
use vproxy::collector::Collector;
use vproxy::db::Database;
#[cfg(feature = "gui")]
use vproxy::gui::{setup_chinese_fonts, VpnApp};
use vproxy::kernel::KernelManager;
use vproxy::proxy::ProxyManager;
use vproxy::scheduler::Scheduler;
use vproxy::speedtest::SpeedTester;
use vproxy::tasks::TaskTracker;
use vproxy::*;
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "vproxy",
    version,
    about = "VProxy: A modern cross-platform proxy workstation with desktop GUI, multi-core compilers and HTTP API"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// HTTP API listen address
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,

    /// SQLite database path。不写就等于 `default_db_path()`：Windows 仍是程序目录下的
    /// `data/vproxy.db`（或平滑沿用历史 `data/vpn-parser.db`），Linux 走 XDG 数据目录。
    #[arg(long)]
    db: Option<std::path::PathBuf>,

    /// Headless mode: only run HTTP API (no GUI)
    #[arg(long, default_value_t = false)]
    headless: bool,

    /// Bearer token required by the HTTP API; can also be set with VPROXY_API_TOKEN (or legacy VPN_PARSER_API_TOKEN)
    #[arg(long)]
    api_token: Option<String>,

    /// base64url 的字母表里有 `-`，编码结果可能以 `-` 开头；不放行的话 clap 会把值
    /// 当成未知选项，提权实例在写日志之前就 exit(2)。
    #[arg(long, hide = true, allow_hyphen_values = true)]
    elevated_proxy_config: Option<String>,

    /// 提权交接配置文件路径（推荐：解决超长规则列表导致 Windows 命令行 32KB 限制溢出崩溃）
    #[arg(long, hide = true)]
    elevated_proxy_config_file: Option<std::path::PathBuf>,

    #[arg(long, hide = true)]
    start_proxy_node: Option<String>,

    /// macOS LaunchAgent entry; resolve the saved node from the private DB.
    #[arg(long, hide = true, default_value_t = false, conflicts_with_all = ["headless", "start_proxy_node"])]
    login_startup: bool,

    /// 无头高权限内核守护进程模式
    #[arg(long, hide = true, default_value_t = false)]
    run_elevated_kernel: bool,

    /// 「以管理员方式启动软件」的提权重启标记：新实例等待旧实例释放互斥量后接管
    /// （与 TUN 提权交接同一套 3 秒轮询，否则新实例会撞上旧实例还没退出的互斥量直接自杀）
    #[arg(long, hide = true, default_value_t = false)]
    elevated_relaunch: bool,

    #[arg(long, hide = true)]
    parent_pid: Option<u32>,

    #[arg(long, hide = true)]
    kernel: Option<String>,

    #[arg(long, hide = true)]
    executable: Option<std::path::PathBuf>,

    #[arg(long, hide = true)]
    data_root: Option<std::path::PathBuf>,

    #[arg(long, hide = true)]
    config: Option<std::path::PathBuf>,

    #[arg(long, hide = true)]
    log_file: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Subcommand)]
enum Commands {
    /// 解析订阅链接或本地文件，以 JSON 或格式化表格输出节点列表
    Parse {
        /// 订阅链接 (URL)、单节点 URI (ss/vmess/vless/...) 或本地文件路径
        target: String,

        /// 输出完整 JSON 格式（默认输出简明表格）
        #[arg(long)]
        json: bool,
    },
    /// 将节点配置编译为目标内核支持的配置文件 (Mihomo / Sing-box / VP-Core / Xray / Shoes / Meow)
    Compile {
        /// 目标内核名称: mihomo, sing-box, vp-core, xray, shoes, meow
        #[arg(long, default_value = "mihomo")]
        kernel: String,

        /// 节点链接或订阅文件路径
        #[arg(short, long)]
        input: String,

        /// 导出文件路径（不指定则打印至 stdout）
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,

        /// 运行模式: rule (规则分流), global (全局代理), direct (全直连)
        #[arg(long, default_value = "rule")]
        mode: String,
    },
    /// 对目标服务器或节点进行快速 TCP / TLS 握手连通性与延迟测试
    Ping {
        /// 目标节点 URI 或主机名:端口 (如 1.1.1.1:443)
        target: String,

        /// 超时时间（毫秒）
        #[arg(long, default_value_t = 3000)]
        timeout: u32,
    },
    /// 将订阅或节点导出转换为指定格式 (clash, base64, uris, json)
    Export {
        /// 节点链接、本地文件路径或订阅 URL
        #[arg(short, long)]
        input: String,

        /// 导出格式: clash (Clash 代理列表 YAML), base64 (标准 Base64 订阅), uris (单行 URI 列表), json (节点完整 JSON)
        #[arg(short, long, default_value = "clash")]
        format: String,

        /// 输出文件路径（不指定则打印至 stdout）
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
}

fn handle_command(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::Parse { target, json } => {
            let content = if target.starts_with("http://") || target.starts_with("https://") {
                eprintln!("正在获取订阅内容: {target} ...");
                let rt = Runtime::new()?;
                rt.block_on(async {
                    let resp = reqwest::get(&target).await?;
                    resp.text().await.map_err(anyhow::Error::from)
                })?
            } else if std::path::Path::new(&target).is_file() {
                std::fs::read_to_string(&target)?
            } else {
                target
            };

            let nodes = vproxy::parser::parse_subscription(&content, None);
            if json {
                println!("{}", serde_json::to_string_pretty(&nodes)?);
            } else {
                println!("┌──────┬──────────────┬──────────────────────────────┬──────────────────────────────┐");
                println!("│ 序号 │ 协议         │ 节点名称                     │ 服务器地址:端口              │");
                println!("├──────┼──────────────┼──────────────────────────────┼──────────────────────────────┤");
                for (idx, node) in nodes.iter().enumerate() {
                    let mut name = node.name.clone();
                    if name.chars().count() > 24 {
                        name = format!("{}…", name.chars().take(23).collect::<String>());
                    }
                    println!(
                        "│ {:4} │ {:12} │ {:28} │ {:24}:{:5} │",
                        idx + 1,
                        node.protocol,
                        name,
                        node.server,
                        node.port
                    );
                }
                println!("└──────┴──────────────┴──────────────────────────────┴──────────────────────────────┘");
                println!("共解析出 {} 个有效节点", nodes.len());
            }
        }
        Commands::Compile {
            kernel,
            input,
            output,
            mode,
        } => {
            let content = if std::path::Path::new(&input).is_file() {
                std::fs::read_to_string(&input)?
            } else {
                input
            };
            let nodes = vproxy::parser::parse_subscription(&content, None);
            if nodes.is_empty() {
                eprintln!("错误: 未在输入中解析出任何有效节点");
                std::process::exit(1);
            }
            let routing_mode = match mode.to_ascii_lowercase().as_str() {
                "global" => vproxy::proxy::RoutingMode::Global,
                "direct" => vproxy::proxy::RoutingMode::Direct,
                _ => vproxy::proxy::RoutingMode::Rule,
            };
            let mut options = vproxy::proxy::MihomoConfigOptions::default();
            options.routing_mode = routing_mode;
            let plan = vproxy::proxy::ir::lower_options_to_ir(&options, Some(&nodes[0]));
            use vproxy::proxy::compiler::{
                BackendCompiler, CompiledArtifact, MeowCompiler, MihomoCompiler, ShoesCompiler,
                SingBoxCompiler, VpCoreCompiler, XrayCompiler,
            };
            let artifact = match kernel.to_ascii_lowercase().as_str() {
                "mihomo" | "clash" => MihomoCompiler::default().compile(&plan)?,
                "sing-box" | "singbox" => SingBoxCompiler::default().compile(&plan)?,
                "vp-core" | "vpcore" | "vp_core" => VpCoreCompiler::default().compile(&plan)?,
                "xray" => XrayCompiler::default().compile(&plan)?,
                "shoes" => ShoesCompiler::default().compile(&plan)?,
                "meow" => MeowCompiler::default().compile(&plan)?,
                other => {
                    eprintln!("错误: 不支持的内核类型 '{other}'，可选: mihomo, sing-box, vp-core, xray, shoes, meow");
                    std::process::exit(1);
                }
            };
            let config_text = match artifact {
                CompiledArtifact::Yaml(yaml) => yaml,
                CompiledArtifact::Json(json) => serde_json::to_string_pretty(&json)?,
            };
            if let Some(out_path) = output {
                std::fs::write(&out_path, &config_text)?;
                println!("配置已成功写入至: {}", out_path.display());
            } else {
                println!("{config_text}");
            }
        }
        Commands::Ping { target, timeout } => {
            let (host, port) = if target.starts_with("ss://")
                || target.starts_with("vmess://")
                || target.starts_with("vless://")
                || target.starts_with("trojan://")
                || target.starts_with("hysteria2://")
            {
                let nodes = vproxy::parser::parse_subscription(&target, None);
                if let Some(node) = nodes.first() {
                    (node.server.clone(), node.port)
                } else {
                    eprintln!("错误: 无法解析节点 URI");
                    std::process::exit(1);
                }
            } else if let Some((h, p)) = target.split_once(':') {
                let port: u16 = p.parse().unwrap_or(80);
                (h.to_string(), port)
            } else {
                (target, 80)
            };
            eprintln!("正在探测目标连通性: {host}:{port} (超时: {timeout}ms)...");
            let c_host = std::ffi::CString::new(host.clone())?;
            let latency =
                unsafe { vproxy::ffi::vpn_test_tcp_latency(c_host.as_ptr(), port, timeout) };
            if latency >= 0 {
                println!("✓ 握手成功! 延迟: {latency} ms");
            } else {
                println!("✗ 握手失败: 连接超时或无法解析主机");
                std::process::exit(1);
            }
        }
        Commands::Export {
            input,
            format,
            output,
        } => {
            let content = if input.starts_with("http://") || input.starts_with("https://") {
                eprintln!("正在获取输入订阅内容: {input} ...");
                let rt = Runtime::new()?;
                rt.block_on(async {
                    let resp = reqwest::get(&input).await?;
                    resp.text().await.map_err(anyhow::Error::from)
                })?
            } else if std::path::Path::new(&input).is_file() {
                std::fs::read_to_string(&input)?
            } else {
                input
            };
            let nodes = vproxy::parser::parse_subscription(&content, None);
            if nodes.is_empty() {
                eprintln!("错误: 未在输入中解析出任何有效节点");
                std::process::exit(1);
            }
            let exported_text = match format.to_ascii_lowercase().as_str() {
                "clash" | "yaml" => {
                    let proxies = vproxy::export::nodes_to_clash_proxies(&nodes);
                    let mut map = serde_yaml::Mapping::new();
                    let yaml_val: serde_yaml::Value =
                        serde_json::from_value(serde_json::Value::Array(proxies))?;
                    map.insert(serde_yaml::Value::String("proxies".into()), yaml_val);
                    serde_yaml::to_string(&map)?
                }
                "base64" | "b64" => {
                    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
                    let uris = vproxy::export::nodes_to_share_uris(&nodes);
                    let combined = uris.join("\n");
                    B64.encode(combined)
                }
                "uris" | "uri" | "links" => {
                    let uris = vproxy::export::nodes_to_share_uris(&nodes);
                    uris.join("\n")
                }
                "json" => serde_json::to_string_pretty(&nodes)?,
                other => {
                    eprintln!("错误: 不支持的导出格式 '{other}'，可选: clash, base64, uris, json");
                    std::process::exit(1);
                }
            };
            if let Some(out_path) = output {
                std::fs::write(&out_path, &exported_text)?;
                eprintln!("导出已成功写入至: {}", out_path.display());
            } else {
                println!("{exported_text}");
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    // launchd must enter the daemon before GUI logging, HOME discovery or DB access.
    #[cfg(target_os = "macos")]
    {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        if args.first().map(String::as_str) == Some("--macos-tun-helper") {
            anyhow::ensure!(args.len() == 3, "后台服务参数错误");
            return platform::macos_helper::run_helper(args[1].parse()?, args[2].parse()?);
        }
        if args.first().map(String::as_str) == Some("--macos-tun-helper-check") {
            anyhow::ensure!(args.len() == 2, "后台服务校验参数错误");
            return platform::macos_helper::check_installation(args[1].parse()?);
        }
        #[cfg(feature = "macos-tun-probe")]
        if args.first().map(String::as_str) == Some("--macos-tun-self-test") {
            return platform::macos_probe::run();
        }
        #[cfg(feature = "macos-tun-probe")]
        if args.first().map(String::as_str) == Some("--macos-tun-abandon-probe") {
            anyhow::ensure!(args.len() == 2, "测试参数错误");
            return platform::macos_probe::abandon(std::path::Path::new(&args[1]));
        }
    }
    platform::register_crash_rollback_hook();
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        #[cfg(windows)]
        {
            system_proxy::direct_disable_wininet_proxy_atomic();
            let _ = platform::restore_all_stale_loopback_dns();
        }
        let _ = std::fs::write("panic.log", &msg);
        startup_log(&msg);
    }));

    // 诊断壳先切 cwd，再写第一行日志：这样 startup.log / panic.log / `data/`（含
    // `--db` 的默认相对路径和分片库）全落在 %TEMP% 的独立目录里，一个字都不碰
    // D 盘那份真实数据。
    let diag = diag::enabled();
    if diag {
        match diag::prepare_home() {
            Ok(home) => {
                let _ = std::env::set_current_dir(&home);
                startup_log(&format!(
                    "=== Application starting in DIAG mode, workdir={} ===",
                    home.display()
                ));
            }
            Err(error) => {
                startup_log(&format!("DIAG workdir 创建失败，退回普通模式: {error}"));
            }
        }
    }

    if !diag {
        startup_log("=== Application starting ===");
    }
    if let Ok(exe_path) = std::env::current_exe() {
        startup_log(&format!("exe_path: {}", exe_path.display()));
        // 诊断模式已经切到自己的临时目录了，别再跟去安装目录（那会重新用上 D 盘的 data）。
        if !diag {
            #[cfg(windows)]
            if let Some(exe_dir) = exe_path.parent() {
                let _ = std::env::set_current_dir(exe_dir);
                startup_log(&format!("set_current_dir: {}", exe_dir.display()));
            }
            #[cfg(not(windows))]
            if let Some(exe_dir) = exe_path.parent() {
                let dir_str = exe_dir.to_string_lossy();
                // Linux/macOS: 系统目录以及 macOS 的 .app Bundle 内部严禁切换 CWD（只读且会导致工作区混乱）
                if !dir_str.starts_with("/usr")
                    && !dir_str.starts_with("/bin")
                    && !dir_str.starts_with("/sbin")
                    && !dir_str.contains(".app/Contents/MacOS")
                {
                    let _ = std::env::set_current_dir(exe_dir);
                    startup_log(&format!("set_current_dir: {}", exe_dir.display()));
                }
            }
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if let Some(cmd) = cli.command {
        return handle_command(cmd);
    }
    if cli.run_elevated_kernel {
        let parent_pid = cli.parent_pid.unwrap_or(0);
        let kernel_str = cli.kernel.unwrap_or_else(|| "mihomo".to_string());
        let kernel = match kernel_str.to_ascii_lowercase().as_str() {
            "sing-box" | "singbox" => vproxy::kernel::KernelKind::SingBox,
            "xray" => vproxy::kernel::KernelKind::Xray,
            "shoes" => vproxy::kernel::KernelKind::Shoes,
            "meow" => vproxy::kernel::KernelKind::Meow,
            "vpcore" | "vp-core" | "vp_core" => vproxy::kernel::KernelKind::VpCore,
            _ => vproxy::kernel::KernelKind::Mihomo,
        };
        let executable = cli.executable.unwrap_or_default();
        let data_root = cli.data_root.unwrap_or_default();
        let config = cli.config.unwrap_or_default();
        let log_file = cli.log_file.unwrap_or_default();

        let args = platform::ElevatedKernelArgs {
            parent_pid,
            kernel,
            executable,
            data_root,
            config,
            log_file,
        };
        return platform::run_elevated_kernel_supervisor(args);
    }

    platform::ensure_job_initialized();
    startup_log(&format!(
        "CLI parsed: headless={}, db={:?}",
        cli.headless, cli.db
    ));
    // 本地 API 能改分流、启停代理、导出全部节点凭据，同机进程和浏览器里的网页都摸得到
    // 它，所以没有令牌就等于敞开：没配置时随机生成一个（GUI「外部接口」页会显示）。
    let api_token = cli
        .api_token
        .clone()
        .or_else(|| std::env::var("VPROXY_API_TOKEN").ok())
        .or_else(|| std::env::var("VPN_PARSER_API_TOKEN").ok())
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let elevated_handoff = cli.elevated_proxy_config.is_some()
        || cli.elevated_proxy_config_file.is_some()
        || cli.elevated_relaunch;
    let _single_instance = if diag {
        // 诊断壳不抢互斥量，也不唤醒/杀死任何别的实例：这台机器上用户实例可能正在跑。
        startup_log("DIAG: 跳过单实例互斥量与重复实例清理");
        None
    } else if !cli.headless {
        startup_log("Checking single instance mutex...");
        if elevated_handoff {
            // 提权交接启动：父进程正在准备退出并释放互斥量。
            // 轮询等待最多 3 秒，拿到互斥量并接管，确保系统中永远由唯一的活动实例持有互斥量。
            let mut guard = None;
            for attempt in 1..=30 {
                match platform::acquire_single_instance() {
                    Ok(Some(g)) => {
                        startup_log(&format!(
                            "Acquired single instance mutex on attempt {attempt}."
                        ));
                        guard = Some(g);
                        break;
                    }
                    Ok(None) | Err(_) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
            if guard.is_none() {
                startup_log("Parent process still holds mutex after 3s; killing duplicate parent/zombie instances...");
                let killed = platform::kill_duplicate_instances();
                startup_log(&format!(
                    "Killed {killed} duplicate instances; retrying mutex acquisition..."
                ));
                guard = platform::acquire_single_instance().ok().flatten();
            }
            guard
        } else {
            match platform::acquire_single_instance() {
                Ok(Some(guard)) => {
                    startup_log("Acquired single instance mutex successfully.");
                    Some(guard)
                }
                Ok(None) => {
                    startup_log(
                        "Single instance mutex exists! Woke up existing instance and exiting.",
                    );
                    return Ok(());
                }
                Err(error) => {
                    startup_log(&format!(
                        "Warning: failed to acquire single instance lock: {error}. Proceeding anyway."
                    ));
                    None
                }
            }
        }
    } else {
        None
    };

    // 清残留内核**必须排在单实例检查之后**：`kill_orphan_proxy_cores` 是按 exe 名满系统杀的，
    // 排在前面时"第二个实例发现已有人在跑、唤醒并退出"这条路会先把**那个正在跑的实例**挂着的
    // mihomo 干掉（2026-10-16 实测：他从命令行跑了一下自己的 exe，出口瞬断 8 秒）。
    // 诊断壳同样不许动任何进程。
    let cleaned_cores = if diag {
        startup_log("DIAG: 跳过 kill_orphan_proxy_cores（不动任何进程）");
        0
    } else {
        platform::kill_orphan_proxy_cores()
    };
    if cleaned_cores > 0 {
        startup_log(&format!(
            "Cleaned up {cleaned_cores} orphan proxy core processes."
        ));
    }

    if !diag {
        platform::self_heal_stale_system_proxy(9980);
        platform::self_heal_stale_dns();
    }

    if _single_instance.is_some() {
        let killed_zombies = platform::kill_duplicate_instances();
        if killed_zombies > 0 {
            startup_log(&format!(
                "Cleaned up {killed_zombies} orphan vproxy zombie processes."
            ));
        }
    }
    let rt = Arc::new(Runtime::new()?);
    let _rt_guard = rt.enter();
    let handoff_proxy_options = if let Some(config_file) = cli.elevated_proxy_config_file.as_deref() {
        match vproxy::proxy::read_elevated_proxy_config_file(config_file) {
            Ok(options) => {
                options.validate()?;
                Some(options)
            }
            Err(error) => {
                startup_log(&format!("读取提权交接配置文件失败: {error}"));
                return Err(error);
            }
        }
    } else if let Some(encoded) = cli.elevated_proxy_config.as_deref() {
        let options = vproxy::proxy::decode_elevated_proxy_config(encoded)?;
        options.validate()?;
        Some(options)
    } else {
        None
    };
    if elevated_handoff {
        std::thread::sleep(std::time::Duration::from_millis(750));
    }

    let db_path = cli
        .db
        .unwrap_or_else(default_db_path)
        .to_string_lossy()
        .replace('\\', "/");
    info!(db = %db_path, "opening database");

    let kernel_manager = KernelManager;
    let (db, collector, speedtester, tasks) = rt.block_on(async {
        for kernel in kernel_manager.detect_all().await {
            info!(
                kernel = kernel.kind.display_name(),
                available = kernel.available,
                path = kernel.path.as_ref().map(|path| path.display().to_string()),
                version = kernel.version,
                message = kernel.message,
                "kernel detection"
            );
        }
        let db = Database::connect(&db_path).await?;
        // 这里不种任何内置订阅源：源一律由用户在「订阅源」页自己加。以前首次运行会静默
        // 塞八个免费源并且全部启用，一轮定时拉取就往库里灌上万条别人分享的节点。
        // Scrub previously mis-tested loopback/private nodes — 后台执行：
        // 全量扫描 35+ 个节点库要好几秒，不能挡住窗口出现。
        {
            let scrub_db = db.clone();
            rt.spawn(async move {
                match scrub_db.scrub_invalid_hosts().await {
                    Ok(n) if n > 0 => info!(count = n, "scrubbed invalid/private host nodes"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "scrub invalid hosts failed"),
                }
                match scrub_db.fix_misclassified_tr_nodes().await {
                    Ok(n) if n > 0 => info!(count = n, "auto-corrected misclassified TR nodes"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "fix misclassified TR nodes failed"),
                }
            });
        }
        // 新安装如果缺少 Geo 分流规则库，在后台异步预先拉取（Fastly CDN 2秒就绪）
        {
            let geo_dir = vproxy::geo::mihomo_dir();
            if !vproxy::geo::geo_ready(&geo_dir) {
                rt.spawn(async move {
                    tracing::info!("新安装首次运行，自动下载 GeoIP / GeoSite 规则库...");
                    let _ = vproxy::geo::provision(&geo_dir, None, false).await;
                });
            }
        }
        let tasks = TaskTracker::default();
        let collector = Collector::new(db.clone(), tasks.clone())?;
        let speedtester = SpeedTester::new(db.clone(), tasks.clone());
        Ok::<_, anyhow::Error>((db, collector, speedtester, tasks))
    })?;
    let proxy_options = if let Some(options) = handoff_proxy_options {
        rt.block_on(db.update_proxy_config(&options))?;
        options
    } else {
        rt.block_on(db.get_proxy_config())?
    };
    let app_settings = match rt.block_on(db.get_settings()) {
        Ok(settings) => settings,
        // Do not mistake a transient DB read failure for the user disabling
        // login startup. launchd can retry this nonzero exit after its throttle.
        Err(error) if cli.login_startup => return Err(error),
        Err(error) => {
            tracing::warn!(%error, "读取应用设置失败，本次使用默认设置");
            Default::default()
        }
    };
    // A still-loaded job can restart after the user disables next-login startup.
    // Exit successfully so launchd does not keep restarting that obsolete entry.
    if cli.login_startup && (!cfg!(target_os = "macos") || !app_settings.launch_at_startup || diag) {
        startup_log("登录自动启动已关闭，本次登录入口正常退出");
        return Ok(());
    }
    #[cfg(feature = "gui")]
    let login_proxy_node = login_proxy_node_id(
        cli.login_startup,
        app_settings.launch_at_startup,
        proxy_options.selected_node_id.as_deref(),
    )
    .map(str::to_owned);
    let has_override_groups = proxy_options
        .override_yaml
        .as_deref()
        .map(|t| t.contains("proxy-groups:") || t.contains("proxy-groups :"))
        .unwrap_or(false);
    let (startup_url_test, startup_url_test_size) =
        (proxy_options.url_test, proxy_options.url_test_size);
    let proxy_manager = ProxyManager::new(proxy_options.clone());
    #[allow(unused_variables)]
    let initial_proxy_node = if let Some(node_id) = cli.start_proxy_node.as_deref() {
        rt.block_on(async {
            match db.get_node(node_id).await? {
                Some(node) => {
                    // 命令行直启也要注入 url-test 候选：候选不填，配置会静默退化成
                    // 单节点 PROXY 组，看起来"自动选路没生效"。
                    if startup_url_test || has_override_groups {
                        match db
                            .top_verified_nodes(startup_url_test_size as usize, Some(&node.id))
                            .await
                        {
                            Ok(candidates) => {
                                let taken = candidates.len();
                                let exported =
                                    proxy_manager.set_url_test_candidates(candidates).await;
                                tracing::info!(
                                    taken,
                                    exported,
                                    "url-test 候选注入完成（exported <= 1 时内核侧会退回单节点组）"
                                );
                            }
                            Err(error) => {
                                tracing::warn!(%error, "url-test 候选读取失败，本次退回单节点组")
                            }
                        }
                    }
                    let selected = Some((node.id.clone(), node.name.clone()));
                    proxy_manager.start(&node).await?;
                    // 持久化当前启动节点至代理配置，保证后续重启记忆
                    let mut options_to_save = proxy_options.clone();
                    options_to_save.selected_node_id = Some(node.id.clone());
                    options_to_save.selected_node_name = Some(node.name.clone());
                    let _ = db.update_proxy_config(&options_to_save).await;
                    Ok::<_, anyhow::Error>(selected)
                }
                None => anyhow::bail!("提权启动节点不存在: {node_id}"),
            }
        })?
    } else if let Some(ref node_id) = proxy_options.selected_node_id {
        // 从持久化代理配置中恢复上次选中的节点记忆
        rt.block_on(async {
            match db.get_node(node_id).await {
                Ok(Some(node)) => {
                    tracing::info!(id = %node.id, name = %node.name, "已恢复上次选中的代理节点记忆");
                    Some((node.id, node.name))
                }
                Ok(None) => {
                    tracing::warn!(node_id, "上次记忆的代理节点在库中已不存在，回退到未选");
                    None
                }
                Err(error) => {
                    tracing::warn!(%error, "查询上次记忆的代理节点失败");
                    None
                }
            }
        })
    } else {
        None
    };

    // Background scheduler: auto-fetch / auto-speedtest
    {
        let scheduler = Scheduler::new(db.clone(), collector.clone(), speedtester.clone());
        rt.spawn(async move {
            scheduler.run_loop().await;
        });
    }

    // Create ApiServerController for dynamic start/stop
    let api_controller = vproxy::api::ApiServerController::new(
        AppState {
            db: db.clone(),
            collector: collector.clone(),
            speedtester: speedtester.clone(),
            tasks: tasks.clone(),
            kernel_manager: kernel_manager.clone(),
            proxy_manager: proxy_manager.clone(),
        },
        cli.listen.clone(),
        Some(api_token.clone()),
    );

    // Start HTTP API in background if headless or enabled in settings
    // Keep the per-user startup entry aligned with the current executable.
    // Diagnostics never change the user's registry or LaunchAgents directory.
    if !diag && !platform::sync_launch_at_startup(app_settings.launch_at_startup) {
        startup_log("警告：登录自动启动同步失败，请检查系统设置和 startup.log");
    }

    // 「以管理员方式启动软件」必须在**创建任何窗口之前**决策：提权成功则本实例直接退出，
    // 整个过程没有窗口出现过，用户只看到提权实例那一个窗口——否则普通实例先把窗口画出来、
    // 提权后再闪第二个，就是"启动闪一次"的来源。UAC 被取消则本实例以普通权限继续（不再重试）。
    if cfg!(windows) && app_settings.launch_as_admin
        && !diag
        && !cli.headless
        && !cli.run_elevated_kernel
        && !elevated_handoff
        && !platform::is_elevated()
    {
        startup_log(
            "管理员启动：设置开启且本实例非提权，在窗口创建前请求 runas 接管",
        );
        match platform::request_elevation(&["--elevated-relaunch".to_string()]) {
            Ok(()) => {
                startup_log("管理员启动：提权已批准，本实例未显示窗口即退出，由提权实例接管");
                return Ok(());
            }
            Err(error) => {
                platform::mark_admin_relaunch_cancelled();
                startup_log(&format!(
                    "管理员启动：提权被取消或拒绝（{error}），本次以普通权限继续启动"
                ));
            }
        }
    }

    // 提权子进程只做代理，不接管 HTTP API：否则等于把（刚加上鉴权的）接口
    // 以管理员身份再监听一次。
    if diag {
        // 诊断壳不起 HTTP API：默认监听 127.0.0.1:8080，会和用户实例的监听端口撞车，
        // 而且那个接口能启停代理、导出全部节点凭据，不该由一个临时实例多开一份。
        startup_log("DIAG: 跳过 HTTP API 监听");
    } else if (cli.headless || app_settings.http_server_enabled) && !elevated_handoff {
        let controller = api_controller.clone();
        rt.spawn(async move {
            if let Err(e) = controller.start().await {
                tracing::error!(error = %e, "HTTP API server failed to start");
            }
        });
    }

    if cli.headless || !cfg!(feature = "gui") {
        info!("running in headless mode, press Ctrl+C to exit");
        // 无界面时没有别的地方能看到自动生成的令牌，只在它确实是随机生成的那次打印一次。
        if cli.api_token.is_none()
            && std::env::var("VPROXY_API_TOKEN").is_err()
            && std::env::var("VPN_PARSER_API_TOKEN").is_err()
        {
            println!("HTTP API 访问令牌（本次启动随机生成）: {api_token}");
        }
        rt.block_on(async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm = signal(SignalKind::terminate()).ok();
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        info!("received SIGINT (Ctrl+C), shutting down");
                    }
                    _ = async {
                        if let Some(sig) = &mut sigterm {
                            sig.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("received SIGTERM, shutting down");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c().await.ok();
            }
            if let Err(error) = proxy_manager.stop().await {
                tracing::error!(%error, "restore system proxy on headless exit failed");
            }
        });
        return Ok(());
    }

    #[cfg(feature = "gui")]
    let app_icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/app-icon.png"))
        .map_err(|error| anyhow::anyhow!("load application icon: {error}"))?;

    // 每个后端都要一份全新的应用工厂：run_native 会把它消费掉，重试时不能复用。
    #[cfg(feature = "gui")]
    let gui_handles = (
        rt.clone(),
        db.clone(),
        collector.clone(),
        speedtester.clone(),
        tasks.clone(),
        kernel_manager.clone(),
        proxy_manager.clone(),
        api_controller.clone(),
        initial_proxy_node.clone(),
    );
    // The renderer may fall back to a second GUI factory. Start the saved proxy
    // only once, after app construction, rather than blocking window creation.
    #[cfg(feature = "gui")]
    let login_connect_started = Arc::new(std::sync::atomic::AtomicBool::new(false));

    #[cfg(feature = "gui")]
    let run_gui = |label: &str, renderer: eframe::Renderer| -> eframe::Result<()> {
        let (
            rt,
            db,
            collector,
            speedtester,
            tasks,
            kernel_manager,
            proxy_manager,
            api_controller,
            initial_proxy_node,
        ) = gui_handles.clone();
        let login_proxy_node = login_proxy_node.clone();
        let login_connect_started = login_connect_started.clone();
        // 窗口大小配置已移除（2026-10-24）：固定中屏 1280x800
        let (window_w, window_h) = (1280.0, 800.0);
        let mut viewport = egui::ViewportBuilder::default()
            // Linux / Wayland 下合成器按 app_id 关联 .desktop 与图标
            .with_app_id("vproxy")
            .with_inner_size([window_w, window_h])
            .with_min_inner_size([820.0, 520.0])
            .with_title("VProxy")
            .with_icon(app_icon.clone())
            .with_visible(true)
            .with_active(!diag::enabled());

        #[cfg(target_os = "macos")]
        {
            // macOS 下如果直接使用 with_decorations(false)，AppKit 会将窗口标记为 NSWindowStyleMaskBorderless（无 Titled 掩码），
            // 导致 Cocoa 拒绝响应 miniaturize:，点击黄色按钮无法最小化到 Dock。
            // 现代 Mac 做法：保留 decorations(true) 以获得完整的 Titled/Closable/Miniaturizable 掩码，
            // 同时隐藏原生标题与原生按钮，并开启 fullsize_content_view，由自定义标题栏呈现无缝界面。
            viewport = viewport
                .with_decorations(true)
                .with_title_shown(false)
                .with_titlebar_buttons_shown(false)
                .with_titlebar_shown(false)
                .with_fullsize_content_view(true);
        }

        #[cfg(not(target_os = "macos"))]
        {
            // 系统标题栏去掉，换成 gui::VpnApp::ui_title_bar 自绘的一条
            // （最小化 / 最大化 / 关闭 + 拖动 + 8 向缩放手）。
            viewport = viewport.with_decorations(false);
        }

        let options = eframe::NativeOptions {
            viewport,
            centered: true,
            persist_window: false,
            renderer,
            ..Default::default()
        };
        let options = if diag::enabled()
            && std::env::var_os("VPROXY_DIAG_ONSCREEN").is_none()
            && std::env::var_os("VPN_PARSER_DIAG_ONSCREEN").is_none()
        {
            // 诊断壳摆到屏幕外、也不抢焦点：验收要靠 PrintWindow 抓图和 PostMessage
            // 合成点击，都不需要它出现在用户眼前或抢走键盘。
            startup_log("DIAG: 窗口移到屏幕外且不激活，避免打扰当前桌面");
            eframe::NativeOptions {
                viewport: options
                    .viewport
                    .with_position(egui::pos2(-30_000.0, -30_000.0)),
                // `centered` 会把上面的 position 直接覆盖掉（实测窗口落在 615,320 屏幕正中，
                // 等于诊断实例把窗口摆到了用户桌面上），所以这里必须自己关掉居中。
                centered: false,
                ..options
            }
        } else {
            options
        };
        startup_log(&format!("Calling eframe::run_native with {label}..."));
        eframe::run_native(
            vproxy::constants::brand::APP_NAME,
            options,
            Box::new(move |cc| {
                startup_log("Inside eframe setup closure, configuring fonts...");
                setup_chinese_fonts(&cc.egui_ctx);
                startup_log("Fonts configured.");
                startup_log("Instantiating VpnApp...");
                let app = VpnApp::new(
                    rt.clone(),
                    db.clone(),
                    collector.clone(),
                    speedtester.clone(),
                    tasks.clone(),
                    kernel_manager.clone(),
                    proxy_manager.clone(),
                    api_controller,
                    initial_proxy_node,
                );
                startup_log("VpnApp instantiated successfully!");
                if let Some(node_id) = login_proxy_node {
                    if !login_connect_started.swap(true, std::sync::atomic::Ordering::AcqRel) {
                        rt.spawn(async move {
                            // No network/authorization work is awaited on the UI
                            // thread. A temporary startup failure leaves the GUI
                            // available and manager recovery retains the intent.
                            if let Err(error) = connect_saved_proxy_at_login(&db, &proxy_manager, &node_id).await {
                                tracing::warn!(%error, "登录自动连接未完成，保留界面以便恢复或手动启动");
                            }
                        });
                    }
                }
                Ok(Box::new(app))
            }),
        )
    };

    #[cfg(feature = "gui")]
    {
        startup_log("Entering GUI startup block");
        // 渲染后端选择策略：
        // 跨平台默认统一首选 Glow（OpenGL），由系统合成器（Windows DWM）稳定呈现，
        // 彻底规避 wgpu 30 在 Windows DirectX 12 下因即时模式突发重绘触发的显示器 VRR / G-SYNC 物理背光频闪。
        // 若环境缺少可用 OpenGL（如未开启 3D 加速的虚拟机），下方自动降级回退至 Wgpu（D3D12 / WARP）。
        // 仍支持 VPROXY_RENDERER（或历史兼容的 VPN_PARSER_RENDERER）环境变量覆盖（如设置 VPROXY_RENDERER=wgpu 强制使用 Wgpu）。
        let force_wgpu = std::env::var("VPROXY_RENDERER")
            .or_else(|_| std::env::var("VPN_PARSER_RENDERER"))
            .map(|v| v.eq_ignore_ascii_case("wgpu"))
            .unwrap_or(false);
        let preferred = if force_wgpu {
            (
                eframe::Renderer::Wgpu,
                "wgpu",
                eframe::Renderer::Glow,
                "glow",
            )
        } else {
            (
                eframe::Renderer::Glow,
                "glow",
                eframe::Renderer::Wgpu,
                "wgpu",
            )
        };
        let mut gui_res = run_gui(preferred.1, preferred.0);
        startup_log(&format!("{} run_native result: {gui_res:?}", preferred.1));
        if gui_res.is_err() {
            // 虚拟机/老驱动常常只有 OpenGL 1.1（Windows 自带软件实现），glow 会直接
            // 报 "egui_glow requires opengl 2.0+"。wgpu 在 Windows 上走 D3D12，
            // 没有硬件时会落到 WARP 软件光栅，仍能出图。
            tracing::warn!("{} 渲染器不可用，改用 {} 重试", preferred.1, preferred.3);
            gui_res = run_gui(preferred.3, preferred.2);
            startup_log(&format!("{} run_native result: {gui_res:?}", preferred.3));
        }
        if let Err(error) = gui_res {
            startup_log(&format!("GUI failed permanently: {error}"));
            vproxy::platform::show_error(
                "VProxy 启动失败",
                &format!(
                    "界面渲染器初始化失败：{error}\n\n\
                     这台机器可能没有可用的 OpenGL/D3D（常见于未开启 3D 加速的虚拟机或显卡驱动过旧）。\n\n\
                     可以试试：\n\
                     1. 开启虚拟机 3D 加速 / 更新显卡驱动\n\
                     2. 以无界面模式运行：vproxy.exe --headless\n\
                     3. 查看启动日志：startup.log"
                ),
            );
            return Err(anyhow::anyhow!("GUI error: {error}"));
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
async fn connect_saved_proxy_at_login(
    db: &Database,
    manager: &ProxyManager,
    node_id: &str,
) -> Result<()> {
    let settings = db.get_settings().await?;
    let saved = db.get_proxy_config().await?;
    // Recheck after the GUI has appeared: an intervening manual preference or
    // selection change wins over the original login request.
    if login_proxy_node_id(
        true,
        settings.launch_at_startup,
        saved.selected_node_id.as_deref(),
    ) != Some(node_id)
    {
        return Ok(());
    }
    let node = db
        .get_node(node_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("上次节点已不存在，请在界面选择节点"))?;
    let mut options = manager.config().await;
    if options.routing_mode == vproxy::proxy::RoutingMode::Subscription {
        if let Some(source_id) = options.subscription_rules_source_id.as_deref() {
            if let Some(source) = db.get_source(source_id).await? {
                options.subscription_rules_yaml = source.rules_yaml;
                options.subscription_groups_yaml = source.groups_yaml;
                manager.update_config(options.clone()).await?;
            }
        }
    }
    if options.url_test
        || options
            .override_yaml
            .as_deref()
            .is_some_and(|text| text.contains("proxy-groups:") || text.contains("proxy-groups :"))
    {
        if let Ok(nodes) = db
            .top_verified_nodes(options.url_test_size as usize, Some(&node.id))
            .await
        {
            manager.set_url_test_candidates(nodes).await;
        }
    }
    let mut mappings = Vec::new();
    for mapping in options
        .port_mappings
        .iter()
        .filter(|mapping| mapping.enabled)
    {
        if let Some(node) = db.get_node(&mapping.node_id).await? {
            mappings.push((mapping.port, node));
        }
    }
    manager.set_port_mapping_nodes(mappings).await;
    if options.chain_proxy_enabled || options.routing_mode == vproxy::proxy::RoutingMode::Chain {
        if let (Some(entry), Some(exit)) = (
            options.chain_entry_node_id.as_deref(),
            options.chain_exit_node_id.as_deref(),
        ) {
            if entry != exit {
                if let (Some(entry), Some(exit)) =
                    (db.get_node(entry).await?, db.get_node(exit).await?)
                {
                    manager.set_chain_nodes(Some((entry, exit))).await;
                }
            }
        }
    }
    let settings = db.get_settings().await?;
    let saved = db.get_proxy_config().await?;
    if login_proxy_node_id(
        true,
        settings.launch_at_startup,
        saved.selected_node_id.as_deref(),
    ) != Some(node_id)
    {
        return Ok(());
    }
    manager.start_automatically(&node).await
}

/// Manual opens remember selection without connecting. Only the enabled login
/// entry reconnects, and an absent selection simply opens the GUI for selection.
#[cfg(any(feature = "gui", test))]
fn login_proxy_node_id<'a>(
    login_entry: bool,
    enabled: bool,
    selected: Option<&'a str>,
) -> Option<&'a str> {
    if login_entry && enabled {
        selected.filter(|id| !id.is_empty())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{login_proxy_node_id, Cli};
    use clap::Parser;

    #[test]
    fn login_entry_restores_only_when_enabled_and_selected() {
        assert_eq!(login_proxy_node_id(true, true, Some("saved-node")), Some("saved-node"));
        assert_eq!(login_proxy_node_id(false, true, Some("saved-node")), None);
        assert_eq!(login_proxy_node_id(true, false, Some("saved-node")), None);
        assert_eq!(login_proxy_node_id(true, true, None), None);
        assert_eq!(login_proxy_node_id(true, true, Some("")), None);
    }

    #[test]
    fn login_entry_cannot_enter_blocking_explicit_node_or_headless_branch() {
        let cli = Cli::try_parse_from(["vproxy", "--login-startup"]).unwrap();
        assert!(cli.login_startup);
        assert!(cli.start_proxy_node.is_none());
        assert!(Cli::try_parse_from(["vproxy", "--login-startup", "--headless"]).is_err());
        assert!(Cli::try_parse_from(["vproxy", "--login-startup", "--start-proxy-node", "node"]).is_err());
    }

    /// 提权交接会把当前进程的命令行原样带上，再追加新的 `--start-proxy-node`，于是同一个
    /// flag 出现两次 —— clap 对此报 ArgumentConflict 并 exit(2)，提权实例在写下第一行日志
    /// 之前就死掉（现象：TUN 模式一闪就没）。这里按真实调用链测：组装参数 → 解析。
    #[test]
    fn elevated_handoff_command_line_parses() {
        let current = ["--start-proxy-node", "db022bb2"]
            .map(String::from)
            .to_vec();
        let extra = [
            "--elevated-proxy-config".to_string(),
            "eyJtb2RlIjoidHVuIn0".to_string(),
            "--start-proxy-node".to_string(),
            "db022bb2".to_string(),
        ];
        let assembled = vproxy::platform::elevated_arguments(&current, &extra);
        let mut argv = vec!["vproxy".to_string()];
        argv.extend(assembled);
        let parsed = Cli::try_parse_from(&argv);
        assert!(parsed.is_ok(), "解析失败: {:?}", parsed.err());
        let cli = parsed.unwrap();
        assert_eq!(cli.start_proxy_node.as_deref(), Some("db022bb2"));
        assert_eq!(
            cli.elevated_proxy_config.as_deref(),
            Some("eyJtb2RlIjoidHVuIn0")
        );
    }

    /// 提权交接使用配置文件时命令行更短，且能正确解析路径。
    #[test]
    fn elevated_handoff_command_line_parses_with_config_file() {
        let current = ["--start-proxy-node", "old-node"]
            .map(String::from)
            .to_vec();
        let extra = [
            "--elevated-proxy-config-file".to_string(),
            "D:\\data\\run\\elevated.json".to_string(),
            "--start-proxy-node".to_string(),
            "new-node".to_string(),
        ];
        let assembled = vproxy::platform::elevated_arguments(&current, &extra);
        let mut argv = vec!["vproxy".to_string()];
        argv.extend(assembled);
        let parsed = Cli::try_parse_from(&argv);
        assert!(parsed.is_ok(), "解析失败: {:?}", parsed.err());
        let cli = parsed.unwrap();
        assert_eq!(cli.start_proxy_node.as_deref(), Some("new-node"));
        assert_eq!(
            cli.elevated_proxy_config_file.as_deref(),
            Some(std::path::Path::new("D:\\data\\run\\elevated.json"))
        );
    }

    /// base64url 字母表含 `-`/`_`，编码结果可能以 `-` 开头，clap 默认会把它当成选项。
    #[test]
    fn elevated_proxy_config_accepts_value_starting_with_hyphen() {
        let parsed = Cli::try_parse_from([
            "vproxy",
            "--elevated-proxy-config",
            "-8f2_aQ",
            "--start-proxy-node",
            "db022bb2",
        ]);
        assert!(parsed.is_ok(), "解析失败: {:?}", parsed.err());
        assert_eq!(
            parsed.unwrap().elevated_proxy_config.as_deref(),
            Some("-8f2_aQ")
        );
    }

    #[test]
    fn subcommand_parse_parses_properly() {
        use super::Commands;
        let parsed = Cli::try_parse_from(["vproxy", "parse", "ss://example", "--json"]);
        assert!(parsed.is_ok());
        let cli = parsed.unwrap();
        match cli.command {
            Some(Commands::Parse { target, json }) => {
                assert_eq!(target, "ss://example");
                assert!(json);
            }
            other => panic!("预期 Commands::Parse，实际: {:?}", other),
        }
    }

    #[test]
    fn subcommand_compile_parses_properly() {
        use super::Commands;
        let parsed = Cli::try_parse_from([
            "vproxy",
            "compile",
            "--kernel",
            "sing-box",
            "-i",
            "ss://test",
            "--mode",
            "global",
        ]);
        assert!(parsed.is_ok());
        let cli = parsed.unwrap();
        match cli.command {
            Some(Commands::Compile {
                kernel,
                input,
                output,
                mode,
            }) => {
                assert_eq!(kernel, "sing-box");
                assert_eq!(input, "ss://test");
                assert!(output.is_none());
                assert_eq!(mode, "global");
            }
            other => panic!("预期 Commands::Compile，实际: {:?}", other),
        }
    }

    #[test]
    fn subcommand_ping_parses_properly() {
        use super::Commands;
        let parsed = Cli::try_parse_from([
            "vproxy",
            "ping",
            "1.1.1.1:443",
            "--timeout",
            "1500",
        ]);
        assert!(parsed.is_ok());
        let cli = parsed.unwrap();
        match cli.command {
            Some(Commands::Ping { target, timeout }) => {
                assert_eq!(target, "1.1.1.1:443");
                assert_eq!(timeout, 1500);
            }
            other => panic!("预期 Commands::Ping，实际: {:?}", other),
        }
    }

    #[test]
    fn subcommand_export_parses_properly() {
        use super::Commands;
        let parsed = Cli::try_parse_from([
            "vproxy",
            "export",
            "-i",
            "ss://test",
            "--format",
            "base64",
        ]);
        assert!(parsed.is_ok());
        let cli = parsed.unwrap();
        match cli.command {
            Some(Commands::Export {
                input,
                format,
                output,
            }) => {
                assert_eq!(input, "ss://test");
                assert_eq!(format, "base64");
                assert!(output.is_none());
            }
            other => panic!("预期 Commands::Export，实际: {:?}", other),
        }
    }
}

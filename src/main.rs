#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app_update;
mod collector;
mod db;
mod diag;
mod export;
mod export_quality;
mod geo;
#[cfg(feature = "gui")]
mod gui;
mod hostutil;
mod i18n;
mod ip_risk;
mod kernel;
mod models;
mod parser;
mod platform;
mod proxy;
mod proxy_probe;
mod router;
mod scheduler;
mod speedtest;
mod system_proxy;
mod tasks;

use crate::api::AppState;
use crate::collector::Collector;
use crate::db::Database;
#[cfg(feature = "gui")]
use crate::gui::VpnApp;
use crate::kernel::KernelManager;
use crate::proxy::ProxyManager;
use crate::scheduler::Scheduler;
use crate::speedtest::SpeedTester;
use crate::tasks::TaskTracker;
use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "vpn-parser",
    about = "VPN/proxy subscription collector with desktop GUI and HTTP API"
)]
struct Cli {
    /// HTTP API listen address
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: String,

    /// SQLite database path。不写就等于 `default_db_path()`：Windows 仍是程序目录下的
    /// `data/vpn-parser.db`（装机就是那个位置，一个字都没改），Linux 走 XDG 数据目录。
    #[arg(long)]
    db: Option<std::path::PathBuf>,

    /// Headless mode: only run HTTP API (no GUI)
    #[arg(long, default_value_t = false)]
    headless: bool,

    /// Bearer token required by the HTTP API; can also be set with VPN_PARSER_API_TOKEN
    #[arg(long)]
    api_token: Option<String>,

    /// base64url 的字母表里有 `-`，编码结果可能以 `-` 开头；不放行的话 clap 会把值
    /// 当成未知选项，提权实例在写日志之前就 exit(2)。
    #[arg(long, hide = true, allow_hyphen_values = true)]
    elevated_proxy_config: Option<String>,

    #[arg(long, hide = true)]
    start_proxy_node: Option<String>,

    /// 无头高权限内核守护进程模式
    #[arg(long, hide = true, default_value_t = false)]
    run_elevated_kernel: bool,

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

/// 单份 startup.log 的上限。超过就整体挪到 `.1` 另开一份 —— 留一份历史，别让
/// 启动阶段那几行（预备状态、内核探测）被后面几小时的运行日志挤掉。
const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// 判据单独拎出来，好测。
fn should_rotate(len: u64) -> bool {
    len > MAX_LOG_BYTES
}

pub(crate) fn startup_log(msg: &str) {
    use std::io::Write;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("[{now}] {msg}");
    // 程序目录写不进去（装在 Program Files 又没提权）就退到 %TEMP%：预备状态那些
    // 步骤要是因为权限看不见，就等于没打日志。
    for candidate in [
        std::path::PathBuf::from("startup.log"),
        std::env::temp_dir().join("vpn-parser-startup.log"),
    ] {
        if should_rotate(
            std::fs::metadata(&candidate)
                .map(|meta| meta.len())
                .unwrap_or(0),
        ) {
            let archived = std::path::PathBuf::from(format!("{}.1", candidate.display()));
            let _ = std::fs::remove_file(&archived);
            let _ = std::fs::rename(&candidate, &archived);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&candidate)
        {
            let _ = writeln!(f, "{line}");
            return;
        }
    }
}

fn main() -> Result<()> {
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
    if cli.run_elevated_kernel {
        let parent_pid = cli.parent_pid.unwrap_or(0);
        let kernel_str = cli.kernel.unwrap_or_else(|| "mihomo".to_string());
        let kernel = match kernel_str.to_ascii_lowercase().as_str() {
            "sing-box" | "singbox" => crate::kernel::KernelKind::SingBox,
            "xray" => crate::kernel::KernelKind::Xray,
            "shoes" => crate::kernel::KernelKind::Shoes,
            "meow" => crate::kernel::KernelKind::Meow,
            _ => crate::kernel::KernelKind::Mihomo,
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
        .or_else(|| std::env::var("VPN_PARSER_API_TOKEN").ok())
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let elevated_handoff = cli.elevated_proxy_config.is_some();
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
                let killed = platform::kill_duplicate_vpn_parser_instances();
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
        let killed_zombies = platform::kill_duplicate_vpn_parser_instances();
        if killed_zombies > 0 {
            startup_log(&format!(
                "Cleaned up {killed_zombies} orphan vpn-parser zombie processes."
            ));
        }
    }
    let rt = Arc::new(Runtime::new()?);
    let _rt_guard = rt.enter();
    let handoff_proxy_options = match cli.elevated_proxy_config.as_deref() {
        Some(encoded) => {
            let options = crate::proxy::decode_elevated_proxy_config(encoded)?;
            options.validate()?;
            Some(options)
        }
        None => None,
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
            });
        }
        // 新安装如果缺少 Geo 分流规则库，在后台异步预先拉取（Fastly CDN 2秒就绪）
        {
            let geo_dir = crate::geo::mihomo_dir();
            if !crate::geo::geo_ready(&geo_dir) {
                rt.spawn(async move {
                    tracing::info!("新安装首次运行，自动下载 GeoIP / GeoSite 规则库...");
                    let _ = crate::geo::provision(&geo_dir, None, false).await;
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
    let (startup_url_test, startup_url_test_size) =
        (proxy_options.url_test, proxy_options.url_test_size);
    let proxy_manager = ProxyManager::new(proxy_options);
    #[allow(unused_variables)]
    let initial_proxy_node = if let Some(node_id) = cli.start_proxy_node.as_deref() {
        rt.block_on(async {
            match db.get_node(node_id).await? {
                Some(node) => {
                    // 命令行直启也要注入 url-test 候选：候选不填，配置会静默退化成
                    // 单节点 PROXY 组，看起来"自动选路没生效"。
                    if startup_url_test {
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
                    Ok::<_, anyhow::Error>(selected)
                }
                None => anyhow::bail!("提权启动节点不存在: {node_id}"),
            }
        })?
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
    let api_controller = crate::api::ApiServerController::new(
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
    let app_settings = rt.block_on(db.get_settings()).unwrap_or_default();
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
        if cli.api_token.is_none() && std::env::var("VPN_PARSER_API_TOKEN").is_err() {
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
        // 窗口大小配置已移除（2026-10-24）：固定中屏 1280x800
        let (window_w, window_h) = (1280.0, 800.0);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                // Linux / Wayland 下合成器按 app_id 关联 .desktop 与图标
                .with_app_id("vpn-parser")
                // 系统标题栏去掉，换成 gui::VpnApp::ui_title_bar 自绘的一条
                // （最小化 / 最大化 / 关闭 + 拖动 + 8 向缩放手）。
                .with_decorations(false)
                // 窗口默认尺寸按数据库持久化设置：小屏 1024x680 / 中屏 1280x800 / 大屏 1440x880
                .with_inner_size([window_w, window_h])
                .with_min_inner_size([820.0, 520.0])
                .with_title("VPN Parser")
                .with_icon(app_icon.clone())
                .with_visible(true)
                .with_active(!diag::enabled()),
            centered: true,
            persist_window: false,
            renderer,
            ..Default::default()
        };
        let options = if diag::enabled() {
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
            "VPN Parser",
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
                Ok(Box::new(app))
            }),
        )
    };

    #[cfg(feature = "gui")]
    {
        startup_log("Entering GUI startup block");
        // VPN_PARSER_RENDERER=wgpu 可强制首选后端，方便在无 OpenGL 的机器上复现/验证。
        let preferred = std::env::var("VPN_PARSER_RENDERER")
            .ok()
            .filter(|v| v.eq_ignore_ascii_case("wgpu"))
            .map(|_| {
                (
                    eframe::Renderer::Wgpu,
                    "wgpu",
                    eframe::Renderer::Glow,
                    "glow",
                )
            })
            .unwrap_or((
                eframe::Renderer::Glow,
                "glow",
                eframe::Renderer::Wgpu,
                "wgpu",
            ));
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
            crate::platform::show_error(
                "VPN Parser 启动失败",
                &format!(
                    "界面渲染器初始化失败：{error}\n\n\
                     这台机器可能没有可用的 OpenGL/D3D（常见于未开启 3D 加速的虚拟机或显卡驱动过旧）。\n\n\
                     可以试试：\n\
                     1. 开启虚拟机 3D 加速 / 更新显卡驱动\n\
                     2. 以无界面模式运行：vpn-parser.exe --headless\n\
                     3. 查看启动日志：startup.log"
                ),
            );
            return Err(anyhow::anyhow!("GUI error: {error}"));
        }
    }
    Ok(())
}

/// 文件名像不像一个中日韩字体。macOS/Linux 上要靠它从字体目录里挑；Windows 用固定文件名。
#[cfg(feature = "gui")]
#[allow(dead_code)]
fn looks_like_cjk_font(file_name_lower: &str) -> bool {
    [
        "pingfang",
        "hiragino",
        "stheiti",
        "songti",
        "kaiti",
        "yuanti",
        "arial unicode",
        "heiti",
        "applegothic",
        "applesdgothicneo",
        "msyh",
        "simhei",
        "simsun",
        "msjh",
        "notosanscjk",
        "notosanssc",
        "notoserifcjk",
        "sourcehansans",
        "sourcehanserif",
        "wqy-microhei",
        "wqy-zenhei",
        "uming",
        "ukai",
        "droidsansfallback",
        "fireflysung",
    ]
    .iter()
    .any(|needle| file_name_lower.contains(needle))
}

/// 在字体目录里递归找中日韩字体。macOS/Linux 的字体装在随发行版变化的子目录下，固定路径
/// 猜不中就走这里；深度封顶，不跟着符号链接往下转。
#[cfg(feature = "gui")]
#[allow(dead_code)]
fn scan_cjk_fonts(dirs: &[std::path::PathBuf], depth: u8) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack: Vec<(std::path::PathBuf, u8)> = dirs.iter().cloned().map(|d| (d, 0)).collect();
    while let Some((current, level)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue; // 目录不存在或读不了：跳过，这不是错误
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if level + 1 <= depth {
                    stack.push((path, level + 1));
                }
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let font_file =
                name.ends_with(".ttf") || name.ends_with(".ttc") || name.ends_with(".otf");
            if font_file && looks_like_cjk_font(&name) {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(feature = "gui")]
/// 本机可用的中日韩字体候选，按“先用哪个”排序。
fn cjk_font_candidates() -> Vec<std::path::PathBuf> {
    #[cfg(windows)]
    {
        [
            std::path::PathBuf::from(r"C:\Windows\Fonts\msyh.ttc"),
            std::path::PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttc"),
            std::path::PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
            std::path::PathBuf::from(r"C:\Windows\Fonts\simsun.ttc"),
            std::path::PathBuf::from(r"C:\Windows\Fonts\msjh.ttc"),
            std::path::PathBuf::from(r"C:\Windows\Fonts\NotoSansSC-Regular.otf"),
        ]
        .to_vec()
    }
    #[cfg(target_os = "macos")]
    {
        let fixed = [
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/Supplemental/Hiragino Sans GB.ttc",
            "/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Kaiti.ttc",
            "/System/Library/Fonts/Supplemental/Yuanti.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        ];
        let mut roots = vec![
            std::path::PathBuf::from("/System/Library/Fonts"),
            std::path::PathBuf::from("/System/Library/Fonts/Supplemental"),
            std::path::PathBuf::from("/Library/Fonts"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(std::path::Path::new(&home).join("Library/Fonts"));
        }
        let mut out: Vec<_> = fixed.iter().map(std::path::PathBuf::from).collect();
        out.extend(scan_cjk_fonts(&roots, 2));
        out
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let fixed = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        ];
        let mut roots = vec![
            std::path::PathBuf::from("/usr/share/fonts"),
            std::path::PathBuf::from("/usr/local/share/fonts"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(std::path::Path::new(&home).join(".local/share/fonts"));
        }
        let mut out: Vec<_> = fixed.iter().map(std::path::PathBuf::from).collect();
        out.extend(scan_cjk_fonts(&roots, 4));
        out
    }
}

#[cfg(feature = "gui")]
/// 启动时校准墨迹垂直位置用的参照串与字号（单位 point）。
const INK_CALIBRATION_LABEL: &str = "启动代理";
#[cfg(feature = "gui")]
const INK_CALIBRATION_SIZE: f32 = 14.0;
#[cfg(feature = "gui")]
/// 偏不到这个数就不动它：实测 Windows 的雅黑 14pt 正好 0.00pt，不该被改。
const INK_MIN_OFFSET: f32 = 1.0;
#[cfg(feature = "gui")]
/// 封顶，再偏也只补这么多（em 的比例）。
/// 依据：mac CI 那台机器实测冬青黑（Hiragino Sans GB）14pt 原始偏 -3.50pt ⇒ 需要 0.25，
/// 这个帽留了一档余量，同时挡住"字体指标病态到把字挤出按钮框"的情况。
const INK_MAX_OFFSET_FACTOR: f32 = 0.30;

#[cfg(feature = "gui")]
/// 由「实测偏移」算出该给这个字体补的 `y_offset_factor`（正 = 往下挪）。
/// 拆成纯函数是为了让测试能拿同一份规则复算，而不是在产品路径旁边再抄一遍。
fn ink_calibration_factor(raw_offset: f32) -> f32 {
    if raw_offset.abs() < INK_MIN_OFFSET {
        return 0.0;
    }
    (-raw_offset / INK_CALIBRATION_SIZE).clamp(-INK_MAX_OFFSET_FACTOR, INK_MAX_OFFSET_FACTOR)
}

#[cfg(feature = "gui")]
/// 墨迹中心相对布局盒中心的偏移，单位 point；负 = 字偏上。
///
/// 按钮画的是「布局盒居中 + 上下等距 padding」，所以这个差就是他肉眼看到的偏差。
/// 中日韩字体的 ascent/descent 普遍不对称（同一份 14pt 文本，行高盒比实际字形高出一截，
/// 多出来的量几乎全落在基线以下），egui 按行高居中 → 字看着贴顶。
/// 只在启动装载字体时跑一次，不进每帧。
fn ink_offset(font_name: &str, data: &std::sync::Arc<egui::FontData>) -> f32 {
    let ctx = egui::Context::default();
    let mut definitions = egui::FontDefinitions::empty();
    definitions
        .font_data
        .insert(font_name.to_owned(), data.clone());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.to_owned());
    }
    ctx.set_fonts(definitions);
    let mut input = egui::RawInput::default();
    input.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(400.0, 200.0),
    ));
    let text = INK_CALIBRATION_LABEL.to_owned();
    let mut offset = 0.0_f32;
    let _ = ctx.run(input, |ctx| {
        let galley = ctx.fonts(|fonts| {
            fonts.layout_no_wrap(
                text.clone(),
                egui::FontId::proportional(INK_CALIBRATION_SIZE),
                egui::Color32::WHITE,
            )
        });
        offset = galley.mesh_bounds.center().y - galley.rect.center().y;
    });
    offset
}

#[cfg(feature = "gui")]
/// 加载中文字体。没有它整屏中文都是方框 —— egui 自带字体不含 CJK 字形。
fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut loaded_names = Vec::new();

    for path in cjk_font_candidates() {
        if let Ok(bytes) = std::fs::read(&path) {
            let font_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("cjk_{}", loaded_names.len()));

            info!(path = %path.display(), font = %font_name, "loaded CJK font for GUI");

            let data = std::sync::Arc::new(egui::FontData::from_owned(bytes));
            // 以前这里按字体名钉死一个 `-0.18`，且只认 "pingfang" —— 而 mac 上第一个能读到的
            // 通常是 Hiragino Sans GB，那段等于从来没跑过（用户 2026-10-20：mac 按钮的字偏上）。
            // 改成装载时拿真正会用到的那份字体量一次再补，Windows 量到 0.00pt 就原样不动。
            let raw_offset = ink_offset(&font_name, &data);
            let factor = ink_calibration_factor(raw_offset);
            let font_data = if factor == 0.0 {
                data
            } else {
                let mut tweaked = std::sync::Arc::unwrap_or_clone(data);
                tweaked.tweak.y_offset_factor = factor;
                std::sync::Arc::new(tweaked)
            };
            info!(
                font = %font_name,
                ink_offset_pt = raw_offset,
                y_offset_factor = factor,
                "calibrated CJK font vertical centering"
            );

            fonts.font_data.insert(font_name.clone(), font_data);
            loaded_names.push(font_name);

            // 仅加载 1 个最优先的可用 CJK 字体。常规中文字体（如微软雅黑、苹方、文泉驿、Noto）
            // 单个字库就包含完备的简体/繁体/日韩字形，避免同时载入多个动辄十数兆的字体文件
            // 造成堆内存与纹理缓存膨胀（每个字体保留整份原始字节与字形表）。
            if loaded_names.len() >= 1 {
                break;
            }
        }
    }

    if loaded_names.is_empty() {
        tracing::warn!("no CJK font found; Chinese text may not render");
        return;
    }

    // 将加载到的 CJK 字体按顺序插入 proportional 与 monospace 字体族首部
    for (i, name) in loaded_names.iter().enumerate() {
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(i, name.clone());
        }
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.insert(i, name.clone());
        }
    }

    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

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
        let assembled = crate::platform::elevated_arguments(&current, &extra);
        let mut argv = vec!["vpn-parser".to_string()];
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

    /// 日志滚动的判据：正好到上限不动，超一点才挪档（挪档是 rename，频繁触发会丢历史）。
    #[test]
    fn log_rotation_triggers_only_above_the_cap() {
        use super::should_rotate;
        assert!(!should_rotate(0));
        assert!(!should_rotate(super::MAX_LOG_BYTES));
        assert!(should_rotate(super::MAX_LOG_BYTES + 1));
    }

    /// base64url 字母表含 `-`/`_`，编码结果可能以 `-` 开头，clap 默认会把它当成选项。
    #[test]
    fn elevated_proxy_config_accepts_value_starting_with_hyphen() {
        let parsed = Cli::try_parse_from([
            "vpn-parser",
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
}
#[cfg(all(test, feature = "gui"))]
mod cjk_font_tests {
    use super::{looks_like_cjk_font, scan_cjk_fonts};
    use std::path::PathBuf;

    /// 判据要认得出各发行版真实的文件名，同时**不许**把这台机器上确实存在的
    /// 拉丁字体（Ubuntu / DejaVu）当成中文 —— 认错了就等于加载一个画不出汉字的字体，
    /// 现象还是方框，但日志会说"已加载中文字体"，比现在更难查。
    #[test]
    fn the_cjk_name_test_accepts_real_packages_and_rejects_latin_fonts() {
        for name in [
            "pingfang.ttc",
            "hiraginosansgb.ttc",
            "stheiti-light.ttc",
            "songti.ttc",
            "notosanscjk-regular.ttc",
            "notosanscjksc-regular.otf",
            "wqy-microhei.ttc",
            "wqy-zenhei.ttc",
            "sourcehansanssc-regular.otf",
            "msyh.ttc",
            "simhei.ttf",
            "droidsansfallback.ttf",
        ] {
            assert!(looks_like_cjk_font(name), "{name} 该被认成中文字体");
        }
        for name in [
            "dejavusans.ttf",
            "dejavusansmono.ttf",
            "ubuntu[wdth,wght].ttf",
            "ubuntumono[wght].ttf",
            "liberationMono-Regular.ttf",
            "",
        ] {
            assert!(!looks_like_cjk_font(name), "{name} 不该被认成中文字体");
        }
    }

    /// 扫描要能钻进发行版那些不一样的子目录（opentype/noto、truetype/wqy），
    /// 并且只收字体后缀：同目录下的 .txt、以及不相关的 .ttf 都不能进来。
    #[test]
    fn the_font_scan_finds_nested_packages_and_skips_other_files() {
        let root = std::env::temp_dir().join(format!("vpncjk-{}", std::process::id()));
        let nested = root.join("opentype/noto");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("NotoSansCJK-Regular.ttc"), b"fake").unwrap();
        let other = root.join("truetype/dejavu");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("DejaVuSans.ttf"), b"fake").unwrap();
        std::fs::write(other.join("README.txt"), b"fake").unwrap();

        let found = scan_cjk_fonts(&[root.clone()], 4);
        assert_eq!(found.len(), 1, "只该找到那个 CJK 字体，实际 {found:?}");
        assert!(found[0]
            .to_string_lossy()
            .ends_with("NotoSansCJK-Regular.ttc"));

        // 反向对照：深度给 0 时，第二层目录不该被打开。
        let shallow = scan_cjk_fonts(&[PathBuf::from(&root)], 0);
        assert!(shallow.is_empty(), "深度 0 还找到了 {shallow:?}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
/// 这个实例的数据根目录：DB、分片库、内核都挂在它下面。
///
/// 三条规则按重要性排：
/// ① 诊断壳永远跟着当前工作目录 —— 它已经切进自己的临时隔离目录，落到用户数据里
///    就等于这个开关存在的意义没了（`VPN_PARSER_DIAG` 的第一条承诺就是不动真数据）。
/// ② Windows 用工作目录下的 `data/`。装机目录就是程序目录（启动时会 chdir 过去），
///    从 1.0 到现在都是这个位置，一个字没改。
/// ③ Linux 用 XDG 数据目录。FHS 布局下可执行文件在 `/usr/bin`，而 `main` 会把工作目录
///    切到 exe 旁边 —— 相对路径就变成往 `/usr/bin/data` 写，普通用户必然失败，
///    而且失败发生在"窗口已经建好之后"，看起来像程序坏了。
pub fn data_root() -> std::path::PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let xdg = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let root = data_root_from(
        crate::diag::enabled(),
        &cwd,
        xdg.as_deref(),
        home.as_deref(),
    );
    let _ = std::fs::create_dir_all(&root);
    root
}

/// 纯函数版，判据在这里测得到（Windows 上也能测 Linux 那两行，否则只能等 Linux CI）。
fn data_root_from(
    diag: bool,
    cwd: &std::path::Path,
    xdg_data_home: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> std::path::PathBuf {
    if diag {
        return cwd.join("data");
    }
    #[cfg(windows)]
    {
        let _ = (xdg_data_home, home);
        cwd.join("data")
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        {
            if let Some(base) = home {
                return base.join("Library/Application Support/vpn-parser");
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(base) = xdg_data_home.filter(|p| p.is_absolute()) {
                return base.join("vpn-parser");
            }
            if let Some(base) = home {
                return base.join(".local/share/vpn-parser");
            }
        }
        cwd.join("data")
    }
}

/// `--db` 不写时的默认库路径。
fn default_db_path() -> std::path::PathBuf {
    data_root().join("vpn-parser.db")
}

#[cfg(test)]
mod data_root_tests {
    use super::data_root_from;
    use std::path::Path;

    /// 诊断壳必须永远落在工作目录里，哪怕这台机器配了 XDG。
    /// 反向对照：这条要是不成立，`VPN_PARSER_DIAG=1` 就会往真数据目录写东西。
    #[test]
    fn the_diag_instance_never_uses_the_user_data_dir() {
        let got = data_root_from(
            true,
            Path::new("/tmp/vpn-parser-diag-1"),
            Some(Path::new("/home/me/.local/share")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(got, Path::new("/tmp/vpn-parser-diag-1/data").to_path_buf());
    }

    /// XDG 优先，其次 HOME 下的常见位置，两个都没有才退回工作目录（而不是猜一个绝对路径）。
    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[test]
    fn the_linux_data_root_follows_xdg_then_home() {
        let got = data_root_from(
            false,
            Path::new("/usr/bin"),
            Some(Path::new("/data/x")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(got, Path::new("/data/x/vpn-parser").to_path_buf());

        let got = data_root_from(
            false,
            Path::new("/usr/bin"),
            None,
            Some(Path::new("/home/me")),
        );
        assert_eq!(
            got,
            Path::new("/home/me/.local/share/vpn-parser").to_path_buf()
        );

        // 相对值不当 XDG 配置：那样会拼出一个跟着工作目录跑的假绝对路径。
        let got = data_root_from(
            false,
            Path::new("/usr/bin"),
            Some(Path::new("relative")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(
            got,
            Path::new("/home/me/.local/share/vpn-parser").to_path_buf()
        );

        let got = data_root_from(false, Path::new("/usr/bin"), None, None);
        assert_eq!(got, Path::new("/usr/bin/data").to_path_buf());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_data_root_follows_application_support() {
        let got = data_root_from(
            false,
            Path::new("/Applications"),
            Some(Path::new("/data/x")),
            Some(Path::new("/Users/me")),
        );
        assert_eq!(
            got,
            Path::new("/Users/me/Library/Application Support/vpn-parser").to_path_buf()
        );
    }

    /// Windows 那份没变：工作目录下的 `data/`，和 1.0 以来的装机位置一致。
    #[cfg(windows)]
    #[test]
    fn the_windows_data_root_stays_next_to_the_program() {
        let got = data_root_from(false, Path::new(r"C:\Apps\VPN Parser"), None, None);
        assert_eq!(got, Path::new(r"C:\Apps\VPN Parser\data").to_path_buf());
    }
}

#[cfg(all(test, feature = "gui"))]
mod font_center_probe {
    use super::*;

    /// 当前平台第一个能被读到的 CJK 字体（`setup_chinese_fonts` 只装这一个）。
    fn active_font() -> String {
        cjk_font_candidates()
            .iter()
            .find(|path| path.is_file())
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .unwrap_or_else(|| "<没有 CJK 字体>".to_string())
    }

    /// 逐台机器上真实存在的每个候选字体跑一遍「量 → 补 → 再量」。
    /// 这条断言在 mac CI 上量的就是那台机器上的真字体，不是我抄的表。
    /// 一个 CJK 字体都读不到时明说跳过（ubuntu CI 镜像就这样），不把"没字体"当成校准失败。
    #[test]
    fn every_available_cjk_font_ends_up_vertically_centered() {
        let mut report = Vec::new();
        let mut over = Vec::new();
        let mut checked = 0_usize;
        for path in cjk_font_candidates() {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let data = std::sync::Arc::new(egui::FontData::from_owned(bytes));
            let started = std::time::Instant::now();
            let raw = ink_offset(&name, &data);
            let factor = ink_calibration_factor(raw);
            let mut tweaked = std::sync::Arc::unwrap_or_clone(data);
            tweaked.tweak.y_offset_factor = factor;
            let residual = ink_offset(&name, &std::sync::Arc::new(tweaked));
            let cost_ms = started.elapsed().as_secs_f64() * 1000.0;
            checked += 1;
            report.push(format!(
                "{name}: 原始{raw:+.2}pt 补{factor:+.3} 残差{residual:+.2}pt 量两次{cost_ms:.1}ms"
            ));
            if residual.abs() > 0.75 {
                over.push(format!("{name} 残差 {residual:+.2}pt"));
            }
        }
        // 这台机器上一个 CJK 字体都读不到 —— ubuntu CI 镜像就是这种情况（顺带说明：
        // 那种环境下界面里的中文只能是方框，产品路径本来也没字体可校准）。
        // 这里明说跳过，而不是把"没字体"当成校准失败。
        if checked == 0 {
            println!(
                "FONT_PROBE SKIPPED：候选路径上一个字体都没读到（{:#?}）",
                active_font()
            );
            return;
        }
        println!(
            "FONT_ACTIVE={} 校准了 {checked} 个字体：{:#?}",
            active_font(),
            report
        );
        assert!(
            over.is_empty(),
            "这些字体校准后仍然偏：{over:#?}\n全部：{report:#?}"
        );
    }

    /// 反向对照：门槛以下不许动它 —— Windows 的雅黑本来就正中，不能被"修"出新偏差。
    #[test]
    fn tiny_offsets_are_left_alone() {
        assert_eq!(ink_calibration_factor(0.0), 0.0);
        assert_eq!(ink_calibration_factor(-0.9), 0.0);
        assert_eq!(ink_calibration_factor(0.9), 0.0);
        // 负 = 偏上 ⇒ 要往下补（正数），别把符号搞反
        assert!(ink_calibration_factor(-2.8) > 0.0);
        assert!(ink_calibration_factor(2.8) < 0.0);
        // 封顶
        assert!(ink_calibration_factor(-90.0).abs() <= INK_MAX_OFFSET_FACTOR + f32::EPSILON);
    }
}

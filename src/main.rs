#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app_update;
mod collector;
mod db;
mod diag;
mod export;
mod geo;
mod export_quality;
#[cfg(feature = "gui")]
mod gui;
mod hostutil;
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
use crate::proxy::{MihomoConfigOptions, ProxyManager};
use crate::scheduler::Scheduler;
use crate::speedtest::SpeedTester;
use crate::tasks::TaskTracker;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
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

    /// SQLite database path
    #[arg(long, default_value = "data/vpn-parser.db")]
    db: std::path::PathBuf,

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
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
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
            if let Some(exe_dir) = exe_path.parent() {
                let _ = std::env::set_current_dir(exe_dir);
                startup_log(&format!("set_current_dir: {}", exe_dir.display()));
            }
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    platform::ensure_job_initialized();
    // 诊断壳不许杀内核：这台机器上用户实例正挂着 mihomo，`kill_orphan_proxy_cores`
    // 是按 exe 名满系统杀的，一杀他的网就断。
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

    let cli = Cli::parse();
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
                match platform::acquire_single_instance()? {
                    Some(g) => {
                        startup_log(&format!("Acquired single instance mutex on attempt {attempt}."));
                        guard = Some(g);
                        break;
                    }
                    None => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
            if guard.is_none() {
                startup_log("Parent process still holds mutex after 3s; killing duplicate parent/zombie instances...");
                let killed = platform::kill_duplicate_vpn_parser_instances();
                startup_log(&format!("Killed {killed} duplicate instances; retrying mutex acquisition..."));
                guard = platform::acquire_single_instance()?;
            }
            guard
        } else {
            match platform::acquire_single_instance()? {
                Some(guard) => {
                    startup_log("Acquired single instance mutex successfully.");
                    Some(guard)
                }
                None => {
                    startup_log("Single instance mutex exists! Woke up existing instance and exiting.");
                    return Ok(());
                }
            }
        }
    } else {
        None
    };

    if _single_instance.is_some() {
        let killed_zombies = platform::kill_duplicate_vpn_parser_instances();
        if killed_zombies > 0 {
            startup_log(&format!("Cleaned up {killed_zombies} orphan vpn-parser zombie processes."));
        }
    }
    let rt = Arc::new(Runtime::new()?);
    let handoff_proxy_options = match cli.elevated_proxy_config.as_deref() {
        Some(encoded) => {
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .context("读取提权代理配置失败")?;
            let options: MihomoConfigOptions =
                serde_json::from_slice(&bytes).context("解析提权代理配置失败")?;
            options.validate()?;
            Some(options)
        }
        None => None,
    };
    if elevated_handoff {
        std::thread::sleep(std::time::Duration::from_millis(750));
    }

    let db_path = cli.db.to_string_lossy().replace('\\', "/");
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
            tokio::signal::ctrl_c().await.ok();
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
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                // 系统标题栏去掉，换成 gui::VpnApp::ui_title_bar 自绘的一条
                // （最小化 / 最大化 / 关闭 + 拖动 + 8 向缩放手）。
                .with_decorations(false)
                // 1280 → 1330：设置页表头那排右对齐按钮（保存设置）在 1280 宽时
                // 会被窗口右边缘裁掉，用户 2026-10-07 要求加宽 50px。
                .with_inner_size([1330.0, 800.0])
                .with_min_inner_size([960.0, 600.0])
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

#[cfg(feature = "gui")]
/// Load a Windows CJK font so Chinese UI text renders correctly.
fn setup_chinese_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",   // Microsoft YaHei
        r"C:\Windows\Fonts\msyhbd.ttc", // Microsoft YaHei Bold
        r"C:\Windows\Fonts\simhei.ttf", // SimHei
        r"C:\Windows\Fonts\simsun.ttc", // SimSun
        r"C:\Windows\Fonts\msjh.ttc",   // Microsoft JhengHei
        r"C:\Windows\Fonts\NotoSansSC-Regular.otf",
    ];

    let mut font_bytes: Option<Vec<u8>> = None;
    let mut font_name = "cjk".to_string();
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            info!(path, "loaded CJK font for GUI");
            font_bytes = Some(bytes);
            font_name = path.rsplit('\\').next().unwrap_or("cjk").to_string();
            break;
        }
    }

    let Some(bytes) = font_bytes else {
        tracing::warn!("no CJK font found; Chinese text may not render");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        font_name.clone(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );

    // Prefer CJK font first for both proportional and monospace families.
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, font_name.clone());
    }
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        family.insert(0, font_name);
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

use crate::kernel::{
    find_kernel_executable, managed_kernel_root, select_proxy_kernel_preference_for_node,
    KernelKind, KernelPreference,
};
use crate::models::Node;
use crate::proxy::adapter::kernel_adapter;
use crate::proxy::types::{
    MihomoConfigOptions, ProxyMode, ProxyRuntimeStatus, ProxyState, RoutingMode,
};
use crate::proxy_probe::{ProxyProbeProgress, ProxyProbeResult, PROXY_CONNECTIVITY_TEST_URL};
use crate::system_proxy::{platform_system_proxy, SystemProxyControl, SystemProxySnapshot};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use uuid::Uuid;

pub const STARTUP_PROXY_TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
pub const STARTUP_PROXY_VERIFY_TIMEOUT: Duration = Duration::from_millis(1_500);
#[allow(dead_code)]
pub const STARTUP_SYSTEM_PROXY_RESERVE: Duration = Duration::from_millis(250);
pub const STARTUP_PORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(60);
pub const VERIFY_CONNECTIVITY_ON_START: bool = true;
pub const STARTUP_PROXY_VERIFY_URLS: [&str; 1] = [PROXY_CONNECTIVITY_TEST_URL];
const RECOVERY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(15),
];

#[derive(Debug)]
pub struct ProxyWorkDir {
    root: PathBuf,
    data_root: PathBuf,
    kernel: KernelKind,
}

impl ProxyWorkDir {
    pub async fn create(kernel: KernelKind) -> Result<Self> {
        let base_dir = crate::data_root().join("run");
        let root = if tokio::fs::create_dir_all(&base_dir).await.is_ok() {
            base_dir.join(format!("proxy-{}", Uuid::new_v4()))
        } else {
            std::env::temp_dir().join(format!("vpn-parser-proxy-{}", Uuid::new_v4()))
        };
        tokio::fs::create_dir_all(&root).await?;
        let data_root = match kernel {
            KernelKind::Mihomo | KernelKind::Meow | KernelKind::VpCore => {
                managed_kernel_root().join(kernel.directory_name())
            }
            KernelKind::SingBox | KernelKind::Xray | KernelKind::Shoes => root.clone(),
        };
        tokio::fs::create_dir_all(&data_root).await?;
        if kernel == KernelKind::VpCore {
            crate::geo::populate_vpcore_work_dir(&data_root, &root);
        }
        Ok(Self {
            root,
            data_root,
            kernel,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(match self.kernel {
            KernelKind::Mihomo | KernelKind::Shoes | KernelKind::Meow | KernelKind::VpCore => "config.yaml",
            KernelKind::SingBox | KernelKind::Xray => "config.json",
        })
    }

    pub fn log_path(&self) -> PathBuf {
        self.root.join(match self.kernel {
            KernelKind::Mihomo => "mihomo.log",
            KernelKind::SingBox => "sing-box.log",
            KernelKind::Xray => "xray.log",
            KernelKind::Shoes => "shoes.log",
            KernelKind::Meow => "meow.log",
            KernelKind::VpCore => "vpcore.log",
        })
    }

    pub async fn cleanup(self) -> Result<()> {
        if self.root.exists() {
            tokio::fs::remove_dir_all(self.root).await?;
        }
        Ok(())
    }
}

pub(crate) enum ProxyChild {
    Standard(Child),
    Elevated(crate::platform::ElevatedChild),
}

impl ProxyChild {
    #[allow(dead_code)]
    pub fn id(&self) -> Option<u32> {
        match self {
            Self::Standard(child) => child.id(),
            Self::Elevated(child) => child.id(),
        }
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self {
            Self::Standard(child) => child.try_wait(),
            Self::Elevated(child) => child.try_wait(),
        }
    }

    pub async fn kill(&mut self) -> std::io::Result<()> {
        match self {
            Self::Standard(child) => child.kill().await,
            Self::Elevated(child) => child.kill().await,
        }
    }

    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self {
            Self::Standard(child) => child.wait().await,
            Self::Elevated(child) => child.wait().await,
        }
    }
}

pub(crate) struct ProxyRuntime {
    pub(crate) status: ProxyRuntimeStatus,
    pub(crate) options: MihomoConfigOptions,
    pending_options: Option<MihomoConfigOptions>,
    pub(crate) child: Option<ProxyChild>,
    pub(crate) work_dir: Option<ProxyWorkDir>,
    pub(crate) system_proxy_snapshot: Option<SystemProxySnapshot>,
    pub(crate) current_node: Option<Node>,
    pub(crate) generation: u64,
    pub(crate) recovery_target: Option<(Node, Option<KernelPreference>)>,
    pub(crate) recovery_pending: bool,
    pub(crate) recovery_attempts: usize,
    pub(crate) recovery_due: Option<tokio::time::Instant>,
    pub(crate) stable_since: Option<tokio::time::Instant>,
    last_route_available: Option<bool>,
    recovery_waiting_for_route: bool,
}

impl ProxyRuntime {
    fn cancel_recovery(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.recovery_target = None;
        self.recovery_pending = false;
        self.recovery_due = None;
        self.recovery_waiting_for_route = false;
    }

    fn request_start(&mut self, node: &Node, preference: Option<KernelPreference>) {
        self.generation = self.generation.wrapping_add(1);
        self.recovery_target = Some((node.clone(), preference));
        self.recovery_pending = false;
        self.recovery_attempts = 0;
        self.recovery_due = None;
        self.stable_since = None;
        self.recovery_waiting_for_route = false;
    }

    fn observe_route(&mut self, available: bool) {
        let returned = self.last_route_available == Some(false) && available;
        self.last_route_available = Some(available);
        if returned && self.child.is_none() && self.recovery_waiting_for_route
            && self.recovery_target.is_some()
        {
            self.recovery_pending = true;
            self.recovery_attempts = 0;
            self.recovery_due = None;
            self.recovery_waiting_for_route = false;
        }
    }

    fn apply_connectivity_result(&mut self, generation: u64, error: Option<String>) {
        if self.generation != generation || !self.status.local_proxy_ready {
            return;
        }
        match error {
            Some(message) if self.status.state == ProxyState::Running => {
                self.status.state = ProxyState::Degraded;
                self.status.last_error = Some(message.clone());
                self.status.message = message;
            }
            None if self.status.state == ProxyState::Degraded
                && self.status.last_error.as_deref().is_some_and(
                    crate::proxy::types::is_node_connectivity_failure,
                ) =>
            {
                self.status.state = ProxyState::Running;
                self.status.last_error = None;
                self.status.message = "代理连通性已恢复，内核继续运行".into();
            }
            _ => {}
        }
    }

    fn recovery_to_attempt(&mut self, now: tokio::time::Instant) -> Option<(Node, Option<KernelPreference>)> {
        if self.child.is_some() {
            if self.stable_since.is_some_and(|since| now.saturating_duration_since(since) >= Duration::from_secs(60)) {
                self.recovery_attempts = 0;
            }
            return None;
        }
        if !self.recovery_pending || self.recovery_target.is_none() {
            return None;
        }
        let attempt = self.recovery_attempts;
        if attempt >= RECOVERY_DELAYS.len() {
            self.recovery_pending = false;
            self.status.message = "内核自动恢复已尝试 3 次，请检查错误后手动启动".into();
            return None;
        }
        let due = *self.recovery_due.get_or_insert(now + RECOVERY_DELAYS[attempt]);
        if now < due {
            return None;
        }
        self.recovery_attempts += 1;
        self.recovery_due = None;
        self.recovery_target.clone()
    }
    /// 界面传进来的配置里，一次性控制器密钥和 url-test 候选都是空的（两者都不持久
    /// 化）。直接覆盖会让鉴权失效、自动选路丢候选，所以空值一律沿用运行时当前的。
    fn merged_options(&self, mut options: MihomoConfigOptions) -> MihomoConfigOptions {
        if options.controller_secret.is_empty() {
            options.controller_secret = self.options.controller_secret.clone();
        }
        if options.url_test_nodes.is_empty() {
            options.url_test_nodes = self.options.url_test_nodes.clone();
        }
        if options.port_mapping_nodes.is_empty() {
            options.port_mapping_nodes = self.options.port_mapping_nodes.clone();
        }
        if options.chain_nodes.is_none() {
            options.chain_nodes = self.options.chain_nodes.clone();
        }
        options
    }

    fn defer_options(&mut self, options: MihomoConfigOptions) {
        self.pending_options = Some(self.merged_options(options));
    }

    pub(crate) fn apply_options(&mut self, options: MihomoConfigOptions) {
        self.options = self.merged_options(options);
        self.pending_options = None;
    }
}

/// 给控制器请求补上 Bearer 密钥（密钥为空时保持原样，兼容未启用鉴权的场景）。
pub fn with_controller_auth(
    request: reqwest::RequestBuilder,
    secret: &str,
) -> reqwest::RequestBuilder {
    if secret.is_empty() {
        request
    } else {
        request.bearer_auth(secret)
    }
}

fn controller_reload_payload(path: &Path, content: &str) -> Value {
    // The macOS service keeps its initial config in a private root-owned directory.
    // Send the compiled YAML inline so reload never depends on a GUI-owned path.
    if cfg!(target_os = "macos") {
        serde_json::json!({ "payload": content })
    } else {
        serde_json::json!({ "path": path.to_string_lossy(), "payload": content })
    }
}

/// Keep the active file intact until the controller accepts the staged config.
/// A rejected request therefore cannot silently change either the next reload or
/// the manager's effective options. The temporary file has the same privacy as
/// other kernel configuration files and is removed on every normal error path.
async fn reload_controller_config(
    controller_port: u16,
    secret: &str,
    config_path: &Path,
    config_text: &str,
) -> Result<()> {
    let previous = tokio::fs::read_to_string(config_path).await
        .context("读取当前内核配置失败")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build().context("创建热重载 HTTP 客户端失败")?;
    let staged = config_path.with_file_name(format!(".reload-{}.yaml", Uuid::new_v4()));
    let mut file_options = tokio::fs::OpenOptions::new();
    file_options.write(true).create_new(true);
    #[cfg(unix)]
    file_options.mode(0o600);
    let result = async {
        use tokio::io::AsyncWriteExt;
        let mut file = file_options.open(&staged).await.context("创建待应用内核配置失败")?;
        file.write_all(config_text.as_bytes()).await.context("写入待应用内核配置失败")?;
        file.flush().await.context("刷新待应用内核配置失败")?;
        drop(file);
        let url = format!("http://127.0.0.1:{controller_port}/configs?force=true");
        let payload = controller_reload_payload(&staged, config_text);
        let response = match with_controller_auth(client.put(&url).json(&payload), secret)
            .send().await
        {
            Ok(response) => response,
            Err(error) => {
                // A lost/late reply does not prove that the core rejected the
                // config. Reapply the previous payload before reporting failure.
                let payload = controller_reload_payload(config_path, &previous);
                let restored = with_controller_auth(client.put(&url).json(&payload), secret)
                    .timeout(Duration::from_secs(5))
                    .send().await.is_ok_and(|response| response.status().is_success());
                return Err(anyhow!(error).context(if restored {
                    "请求内核热重载 API 失败，已恢复原配置"
                } else {
                    "请求内核热重载 API 失败，未确认恢复原配置，请检查当前连接"
                }));
            }
        };
        if !response.status().is_success() {
            return Err(anyhow!("内核热重载 API 返回 {}，当前生效配置未更新", response.status()));
        }
        if let Err(error) = tokio::fs::rename(&staged, config_path).await {
            // The controller accepted the new config, but storage could not commit.
            // Restore the previous core config while its original file is intact.
            let payload = controller_reload_payload(config_path, &previous);
            let restored = with_controller_auth(client.put(&url).json(&payload), secret)
                .timeout(Duration::from_secs(5))
                .send().await.is_ok_and(|response| response.status().is_success());
            if restored {
                return Err(anyhow!(error).context("提交内核配置文件失败，已恢复原配置"));
            }
            return Err(anyhow!(error).context("提交内核配置文件失败，恢复原配置亦失败，请检查当前连接"));
        }
        Ok(())
    }.await;
    let _ = tokio::fs::remove_file(staged).await;
    result
}

#[derive(Clone)]
pub(crate) struct ProxyConfigSnapshot {
    active: MihomoConfigOptions,
    pending: Option<MihomoConfigOptions>,
}

#[derive(Clone)]
pub struct ProxyManager {
    pub(crate) runtime: Arc<Mutex<ProxyRuntime>>,
    operation: Arc<Mutex<()>>,
    configuration_save: Arc<Mutex<()>>,
    system_proxy: Arc<dyn SystemProxyControl>,
}

impl ProxyManager {
    pub fn new(options: MihomoConfigOptions) -> Self {
        Self::with_system_proxy(options, platform_system_proxy())
    }

    pub fn with_system_proxy(
        mut options: MihomoConfigOptions,
        system_proxy: Arc<dyn SystemProxyControl>,
    ) -> Self {
        // 控制端口默认无鉴权：同机任意进程都能 PUT /configs 热改规则/DNS/节点。
        // 每次进程启动生成一次性密钥，GUI 侧的控制器调用统一带 Bearer。
        if options.controller_secret.is_empty() {
            options.controller_secret = Uuid::new_v4().simple().to_string();
        }
        Self {
            runtime: Arc::new(Mutex::new(ProxyRuntime {
                status: ProxyRuntimeStatus::stopped(options.clone()),
                options,
                pending_options: None,
                child: None,
                work_dir: None,
                system_proxy_snapshot: None,
                current_node: None,
                generation: 0,
                recovery_target: None,
                recovery_pending: false,
                recovery_attempts: 0,
                recovery_due: None,
                stable_since: None,
                last_route_available: None,
                recovery_waiting_for_route: false,
            })),
            operation: Arc::new(Mutex::new(())),
            configuration_save: Arc::new(Mutex::new(())),
            system_proxy,
        }
    }

    /// 系统代理被外部改掉之后，把它写回本程序的地址。只在设置里那条「自动写回」开着时才会被调用
    /// —— 它动的是用户 HKCU，默认不许动。返回的话给界面/日志用，失败不能静默。
    pub async fn rearm_system_proxy(&self) -> Result<String> {
        let _operation = self.operation.lock().await;
        let mut runtime = self.runtime.lock().await;
        if !runtime.options.mode.has_system() {
            return Err(anyhow::anyhow!("当前未启用系统代理，不写注册表"));
        }
        let server = format!("127.0.0.1:{}", runtime.options.mixed_port);
        let terminal_port = runtime
            .options
            .terminal_proxy
            .then_some(runtime.options.mixed_port);
        let snapshot =
            activate_system_proxy(self.system_proxy.clone(), server.clone(), terminal_port).await?;
        runtime.system_proxy_snapshot = Some(snapshot);
        Ok(format!("已把系统代理写回 {server}"))
    }

    /// 运行中免停热切换系统代理开关（修改 Windows 注册表，无需重启内核进程、不断开已有连接）
    pub async fn hot_toggle_system_proxy(&self, enable: bool) -> Result<String> {
        let _operation = self.operation.lock().await;
        let mut runtime = self.runtime.lock().await;
        let port = runtime.options.mixed_port;
        if enable {
            let server = format!("127.0.0.1:{port}");
            let terminal_port = runtime
                .options
                .terminal_proxy
                .then_some(port);
            let snapshot =
                activate_system_proxy(self.system_proxy.clone(), server.clone(), terminal_port).await?;
            runtime.system_proxy_snapshot = Some(snapshot);
            runtime.status.system_proxy_enabled = true;
            let tun_on = runtime.options.mode.has_tun();
            runtime.options.mode = if tun_on { ProxyMode::Both } else { ProxyMode::System };
            runtime.status.mode = runtime.options.mode;
            let mode = runtime.options.mode;
            if let Some(pending) = runtime.pending_options.as_mut() {
                pending.mode = mode;
            }
            tracing::info!(port, "系统代理免停热开启成功");
            Ok(format!("系统代理已即时开启 (指向 {server})，未重启内核"))
        } else {
            if let Some(snapshot) = runtime.system_proxy_snapshot.take() {
                let _ = restore_system_proxy(self.system_proxy.clone(), snapshot, false).await;
            } else {
                let _ = crate::system_proxy::direct_disable_wininet_proxy_atomic();
            }
            runtime.status.system_proxy_enabled = false;
            let tun_on = runtime.options.mode.has_tun();
            runtime.options.mode = if tun_on { ProxyMode::Tun } else { ProxyMode::None };
            runtime.status.mode = runtime.options.mode;
            let mode = runtime.options.mode;
            if let Some(pending) = runtime.pending_options.as_mut() {
                pending.mode = mode;
            }
            tracing::info!("系统代理免停热关闭成功");
            Ok("系统代理已即时关闭，未重启内核".to_string())
        }
    }

    pub async fn status(&self) -> ProxyRuntimeStatus {
        let cleanup = {
            let mut runtime = self.runtime.lock().await;
            let exit_status = match runtime.child.as_mut().map(ProxyChild::try_wait) {
                Some(Ok(status)) => {
                    if status.is_none()
                        && runtime.status.state == ProxyState::Degraded
                        && runtime.status.local_proxy_ready
                        && runtime.status.last_error.as_deref().is_some_and(|message| {
                            message.starts_with("读取内核或 TUN 后台服务状态失败:")
                        })
                    {
                        runtime.status.state = ProxyState::Running;
                        runtime.status.last_error = None;
                        runtime.status.message = "TUN 后台服务状态查询已恢复".into();
                    }
                    status
                }
                Some(Err(error)) => {
                    runtime.status.state = ProxyState::Degraded;
                    runtime.status.last_error = Some(format!("读取内核或 TUN 后台服务状态失败: {error}"));
                    runtime.status.message = runtime.status.last_error.clone().unwrap();
                    // Keep the handle/work directory so Stop can still retry cleanup.
                    None
                }
                None => None,
            };
            if let Some(exit_status) = exit_status {
                let kernel_name = runtime
                    .status
                    .kernel
                    .map(KernelKind::display_name)
                    .unwrap_or("代理内核");
                runtime.child = None;
                runtime.generation = runtime.generation.wrapping_add(1);
                runtime.recovery_pending = runtime.recovery_target.is_some();
                runtime.recovery_waiting_for_route = runtime.recovery_pending;
                runtime.stable_since = None;
                let message = format!("{kernel_name} 已退出: {exit_status}");
                runtime.status.state = if runtime.system_proxy_snapshot.is_some() {
                    ProxyState::Degraded
                } else {
                    ProxyState::Failed
                };
                runtime.status.local_proxy_ready = false;
                runtime.status.system_proxy_enabled = runtime.system_proxy_snapshot.is_some();
                runtime.status.last_error = Some(message.clone());
                runtime.status.message = message;
                runtime.work_dir.take()
            } else {
                None
            }
        };
        if let Some(work_dir) = cleanup {
            let _ = work_dir.cleanup().await;
        }
        self.runtime.lock().await.status.clone()
    }

    pub async fn config(&self) -> MihomoConfigOptions {
        let runtime = self.runtime.lock().await;
        runtime.pending_options.as_ref().unwrap_or(&runtime.options).clone()
    }

    pub(crate) async fn config_snapshot(&self) -> ProxyConfigSnapshot {
        let runtime = self.runtime.lock().await;
        ProxyConfigSnapshot {
            active: runtime.options.clone(),
            pending: runtime.pending_options.clone(),
        }
    }

    /// Roll back the config that was actually running, then restore the saved
    /// reconnect request independently. Reloading only the desired pending config
    /// could otherwise leave a newer active config behind after a DB failure.
    pub(crate) async fn restore_config_snapshot(&self, previous: ProxyConfigSnapshot) -> Result<bool> {
        let result = self.reload_config(previous.active).await;
        self.runtime.lock().await.pending_options = previous.pending;
        result
    }

    /// Serialize the whole GUI save transaction, including DB commit/rollback.
    /// This is separate from `operation`, which reload/start already acquire.
    pub(crate) async fn lock_configuration_save(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.configuration_save.clone().lock_owned().await
    }

    /// Local process supervision is independent of node failover. A disconnected
    /// Wi-Fi or an HTTP timeout never restarts a live kernel or changes its node.
    pub async fn maintain(&self) -> Option<ProxyRuntimeStatus> {
        let _operation = self.operation.try_lock().ok()?;
        self.status().await;
        self.runtime.lock().await.observe_route(local_route_available());
        let target = self.runtime.lock().await.recovery_to_attempt(tokio::time::Instant::now());
        if let Some((node, preference)) = target {
            // The existing helper may be restarting under launchd. Automatic
            // recovery must never install software or ask for administrator auth.
            let result = self.restart_inner(&node, preference, false).await;
            if let Err(error) = result {
                tracing::warn!(error = %error, "内核自动恢复失败");
            }
        } else {
            let should_recheck = {
                let runtime = self.runtime.lock().await;
                runtime.child.is_some()
                    && runtime.status.state == ProxyState::Degraded
                    && runtime.status.last_error.as_deref().is_some_and(
                        crate::proxy::types::is_node_connectivity_failure,
                    )
            };
            if should_recheck {
                self.probe_health(Duration::from_secs(3)).await;
            }
        }
        Some(self.status().await)
    }

    /// 自动切换用的轻量健康探测：经**正在运行的** mixed 端口取一次 generate_204，
    /// 返回 (是否连通, 端到端耗时 ms)。不新起进程、不重启内核，所以可以高频调用。
    pub async fn probe_health(&self, timeout: Duration) -> (bool, u64) {
        let (port, generation) = {
            let runtime = self.runtime.lock().await;
            (runtime.options.mixed_port, runtime.generation)
        };
        let started = tokio::time::Instant::now();
        let connect_timeout = timeout.min(Duration::from_millis(3_500)).max(Duration::from_millis(1_500));
        let built = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .and_then(|proxy| {
                reqwest::Client::builder()
                    .proxy(proxy)
                    .connect_timeout(connect_timeout)
                    .timeout(timeout)
                    .build()
            })
            .map_err(|error| error.to_string());
        let ok = match built {
            Ok(client) => {
                let google_ok = client
                    .get(crate::proxy_probe::PROXY_CONNECTIVITY_TEST_URL)
                    .send()
                    .await
                    .map(|response| response.status().is_success())
                    .unwrap_or(false);
                if google_ok {
                    true
                } else {
                    client
                        .get(crate::proxy_probe::PROXY_CONNECTIVITY_FALLBACK_URL)
                        .send()
                        .await
                        .map(|response| response.status().is_success())
                        .unwrap_or(false)
                }
            }
            Err(_) => false,
        };
        if ok {
            self.runtime.lock().await.apply_connectivity_result(generation, None);
        }
        (ok, started.elapsed().as_millis() as u64)
    }

    /// 启动前注入 url-test 候选节点（不持久化，只影响本次生成的配置）。返回真正
    /// 能导出成 clash proxy 的个数：内核侧只有 >1 才会生成「自动选择」组。
    pub async fn set_url_test_candidates(&self, nodes: Vec<Node>) -> usize {
        let proxies = crate::export::nodes_to_clash_proxies(&nodes);
        let count = proxies.len();
        let mut runtime = self.runtime.lock().await;
        if let Some(pending) = runtime.pending_options.as_mut() {
            pending.url_test_nodes = proxies.clone();
        }
        runtime.options.url_test_nodes = proxies;
        count
    }

    /// 启动前或热重载前注入一节点一端口映射目标节点（不持久化，只影响本次生成的配置）。
    pub async fn set_port_mapping_nodes(&self, items: Vec<(u16, Node)>) {
        let mut mapping_nodes = Vec::with_capacity(items.len());
        for (port, node) in items {
            let clash = crate::export::nodes_to_clash_proxies(&[node.clone()]);
            if let Some(proxy_obj) = clash.into_iter().next() {
                mapping_nodes.push((port, node.name, proxy_obj));
            }
        }
        let mut runtime = self.runtime.lock().await;
        if let Some(pending) = runtime.pending_options.as_mut() {
            pending.port_mapping_nodes = mapping_nodes.clone();
        }
        runtime.options.port_mapping_nodes = mapping_nodes;
    }

    /// 启动前或热重载前注入链式代理中继与落地节点实体（不持久化，只影响本次生成的配置）。
    pub async fn set_chain_nodes(&self, nodes: Option<(Node, Node)>) {
        let mut runtime = self.runtime.lock().await;
        let nodes = nodes.map(Box::new);
        if let Some(pending) = runtime.pending_options.as_mut() {
            pending.chain_nodes = nodes.clone();
        }
        runtime.options.chain_nodes = nodes;
    }

    /// Candidate priming is local preparation, not an applied config change.
    /// Restore only those transient fields when compilation/storage rejects a save.
    pub(crate) async fn restore_config_candidates(&self, previous: &ProxyConfigSnapshot) {
        let mut runtime = self.runtime.lock().await;
        runtime.options.port_mapping_nodes = previous.active.port_mapping_nodes.clone();
        runtime.options.chain_nodes = previous.active.chain_nodes.clone();
        if let (Some(pending), Some(saved)) = (runtime.pending_options.as_mut(), previous.pending.as_ref()) {
            pending.port_mapping_nodes = saved.port_mapping_nodes.clone();
            pending.chain_nodes = saved.chain_nodes.clone();
        }
    }

    pub async fn update_config(&self, options: MihomoConfigOptions) -> Result<()> {
        options.validate()?;
        let _operation = self.operation.lock().await;
        let mut runtime = self.runtime.lock().await;
        if !runtime.status.config_editable() {
            return Err(anyhow!("请先关闭内核再修改运行配置"));
        }
        runtime.apply_options(options.clone());
        runtime.status.mode = options.mode;
        runtime.status.routing_mode = options.routing_mode;
        runtime.status.mixed_port = options.mixed_port;
        runtime.status.controller_port = options.controller_port;
        runtime.status.controller_secret = options.controller_secret.clone();
        Ok(())
    }

    /// Ok(true): applied now (or stored while stopped); Ok(false): reconnect required.
    /// Compilation, storage and controller failures are errors, never deferred success.
    pub async fn reload_config(&self, options: MihomoConfigOptions) -> Result<bool> {
        options.validate()?;
        let _operation = self.operation.lock().await;

        let (kernel, config_path, controller_port, controller_secret, node, effective_options) = {
            let mut runtime = self.runtime.lock().await;
            if !matches!(runtime.status.state, ProxyState::Running | ProxyState::Degraded) {
                runtime.apply_options(options);
                let options = runtime.options.clone();
                runtime.status.mode = options.mode;
                runtime.status.routing_mode = options.routing_mode;
                runtime.status.mixed_port = options.mixed_port;
                runtime.status.controller_port = options.controller_port;
                runtime.status.controller_secret = options.controller_secret;
                return Ok(true);
            }
            // A user-owned process cannot acquire utun permissions by reloading YAML.
            #[cfg(target_os = "macos")]
            if options.mode.has_tun()
                && !crate::platform::is_elevated()
                && !matches!(runtime.child.as_ref(), Some(ProxyChild::Elevated(_)))
            {
                runtime.defer_options(options);
                return Ok(false);
            }
            let kernel = match runtime.status.kernel {
                Some(k @ (KernelKind::Mihomo | KernelKind::VpCore)) => k,
                _ => {
                    runtime.defer_options(options);
                    return Ok(false);
                }
            };
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().context("读取运行内核状态失败")?.is_none(),
                None => false,
            };
            if !alive {
                return Err(anyhow!("当前内核已退出，请重新连接后保存配置"));
            }
            let config_path = runtime.work_dir.as_ref()
                .ok_or_else(|| anyhow!("运行内核缺少配置目录"))?.config_path();
            let node = runtime.current_node.clone()
                .ok_or_else(|| anyhow!("运行内核缺少当前节点"))?;
            let effective = runtime.merged_options(options.clone());
            // Listener/authentication changes need a reconnect; issue reload requests
            // only to the controller belonging to the currently running process.
            if effective.controller_port != runtime.options.controller_port
                || effective.controller_secret != runtime.options.controller_secret
                || effective.mixed_port != runtime.options.mixed_port
            {
                runtime.defer_options(options);
                return Ok(false);
            }
            (
                kernel,
                config_path,
                runtime.options.controller_port,
                runtime.options.controller_secret.clone(),
                node,
                effective,
            )
        };

        let started = tokio::time::Instant::now();
        let config = build_kernel_config(kernel, &node, effective_options)
            .context("构建热重载配置失败")?;
        let config_text = serialize_kernel_config(kernel, &config)
            .context("序列化热重载配置失败")?;
        reload_controller_config(controller_port, &controller_secret, &config_path, &config_text).await?;

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut runtime = self.runtime.lock().await;
        runtime.apply_options(options);
        let routing_mode = runtime.options.routing_mode;
        runtime.status.routing_mode = routing_mode;
        let kernel_name = match kernel {
            KernelKind::Mihomo => "Mihomo",
            KernelKind::VpCore => "VP-Core",
            _ => "内核",
        };
        runtime.status.message = format!("{kernel_name} 运行中（配置已热重载生效，耗时 {elapsed_ms} ms）");
        tracing::info!(elapsed_ms, "{kernel_name} 配置热重载生效成功");
        Ok(true)
    }

    /// TUN 开关热切换：代理运行中改 TUN 启停不重启内核进程、不断已建立连接。
    /// - Mihomo：PATCH /configs 只提交 tun 段差量（内核即时建/删 Wintun 网卡与路由）；
    /// - VP-Core：重新编译全量配置后 PUT /configs 热重载（内核按 tun.enable 差量同步入站）；
    /// - 其余内核（SingBox / Xray / Shoes / Meow）没有运行时 TUN API：返回 Ok(false)，
    ///   由调用方回退整进程重启。
    /// 返回 Ok(true) = 热切换已生效；Ok(false) = 内核不支持或热切换失败（可安全回退重启）。
    pub async fn try_hot_toggle_tun(&self, mut options: MihomoConfigOptions) -> Result<bool> {
        options.validate()?;
        let started = tokio::time::Instant::now();
        let tun_on = options.mode.has_tun();

        let (kernel, config_path, controller_port, controller_secret, options, node) = {
            let mut runtime = self.runtime.lock().await;
            if !matches!(
                runtime.status.state,
                ProxyState::Running | ProxyState::Degraded
            ) {
                return Ok(false);
            }
            #[cfg(target_os = "macos")]
            if tun_on
                && !crate::platform::is_elevated()
                && !matches!(runtime.child.as_ref(), Some(ProxyChild::Elevated(_)))
            {
                runtime.defer_options(options);
                return Ok(false);
            }
            options = runtime.merged_options(options);
            if options.controller_port != runtime.options.controller_port
                || options.controller_secret != runtime.options.controller_secret
                || options.mixed_port != runtime.options.mixed_port
            {
                runtime.defer_options(options);
                return Ok(false);
            }
            let Some(kernel) = runtime.status.kernel else {
                return Ok(false);
            };
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().is_ok_and(|status| status.is_none()),
                None => false,
            };
            if !alive {
                return Ok(false);
            }
            let Some(work_dir) = &runtime.work_dir else {
                return Ok(false);
            };
            // 一次性字段（密钥 / 候选节点 / 端口映射）不持久化：为空时沿用运行时当前值，
            // 否则热重载出的配置会丢鉴权、丢 url-test 候选（与 apply_options 同一套规则）。
            if options.controller_secret.is_empty() {
                options.controller_secret = runtime.options.controller_secret.clone();
            }
            if options.url_test_nodes.is_empty() {
                options.url_test_nodes = runtime.options.url_test_nodes.clone();
            }
            if options.port_mapping_nodes.is_empty() {
                options.port_mapping_nodes = runtime.options.port_mapping_nodes.clone();
            }
            (
                kernel,
                work_dir.config_path(),
                runtime.options.controller_port,
                runtime.options.controller_secret.clone(),
                options,
                runtime.current_node.clone(),
            )
        };

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_millis(2500))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("创建 TUN 热切换 HTTP 客户端失败: {error}");
                return Ok(false);
            }
        };

        let hot_applied = match kernel {
            KernelKind::Mihomo => {
                // PATCH 只提交 tun 段差量，字段与 mihomo 编译器保持一致
                let mut dns_hijack = options.tun.dns_hijack.clone();
                if !dns_hijack.iter().any(|h| h.contains("tcp://")) {
                    dns_hijack.push("tcp://any:53".into());
                }
                let payload = serde_json::json!({
                    "tun": {
                        "enable": tun_on,
                        "stack": options.tun.stack.as_mihomo_value(),
                        "dns-hijack": dns_hijack,
                        "auto-route": options.tun.auto_route,
                        "auto-detect-interface": options.tun.auto_detect_interface,
                        "strict-route": true,
                        "endpoint-independent-nat": true,
                    }
                });
                let url = format!("http://127.0.0.1:{controller_port}/configs");
                let request = with_controller_auth(client.patch(&url).json(&payload), &controller_secret);
                match request.send().await {
                    Ok(response) if response.status().is_success() => true,
                    Ok(response) => {
                        tracing::warn!(status = %response.status(), "Mihomo TUN 热切换 PATCH 失败");
                        false
                    }
                    Err(error) => {
                        tracing::warn!("请求 Mihomo TUN 热切换失败: {error}");
                        false
                    }
                }
            }
            KernelKind::VpCore => {
                let Some(node) = node else {
                    tracing::warn!("VP-Core TUN 热切换缺当前节点，回退重启");
                    return Ok(false);
                };
                let config = match build_kernel_config(KernelKind::VpCore, &node, options.clone())
                {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::warn!("构建 VP-Core TUN 热切换配置失败: {error}");
                        return Ok(false);
                    }
                };
                let config_text = match serialize_kernel_config(KernelKind::VpCore, &config) {
                    Ok(text) => text,
                    Err(error) => {
                        tracing::warn!("序列化 VP-Core TUN 热切换配置失败: {error}");
                        return Ok(false);
                    }
                };
                if let Err(error) = tokio::fs::write(&config_path, &config_text).await {
                    tracing::warn!("写入 VP-Core 热切换配置文件失败: {error}");
                    return Ok(false);
                }
                let url = format!("http://127.0.0.1:{controller_port}/configs?force=true");
                let payload = serde_json::json!({
                    "path": config_path.to_string_lossy(),
                });
                let request = with_controller_auth(client.put(&url).json(&payload), &controller_secret);
                match request.send().await {
                    Ok(response) if response.status().is_success() => true,
                    Ok(response) => {
                        tracing::warn!(status = %response.status(), "VP-Core TUN 热切换 PUT 失败");
                        false
                    }
                    Err(error) => {
                        tracing::warn!("请求 VP-Core TUN 热切换失败: {error}");
                        false
                    }
                }
            }
            _ => false,
        };

        if !hot_applied {
            return Ok(false);
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut runtime = self.runtime.lock().await;

        // 同步系统代理状态：避免开关 TUN 时丢失原先并行的系统代理状态
        if options.mode.has_system() && runtime.system_proxy_snapshot.is_none() {
            let server = format!("127.0.0.1:{}", runtime.options.mixed_port);
            let terminal_port = runtime.options.terminal_proxy.then_some(runtime.options.mixed_port);
            if let Ok(snapshot) = activate_system_proxy(self.system_proxy.clone(), server, terminal_port).await {
                runtime.system_proxy_snapshot = Some(snapshot);
                runtime.status.system_proxy_enabled = true;
            }
        } else if !options.mode.has_system() && runtime.system_proxy_snapshot.is_some() {
            if let Some(snapshot) = runtime.system_proxy_snapshot.take() {
                let _ = restore_system_proxy(self.system_proxy.clone(), snapshot, false).await;
            } else {
                let _ = crate::system_proxy::direct_disable_wininet_proxy_atomic();
            }
            runtime.status.system_proxy_enabled = false;
        }

        runtime.apply_options(options.clone());
        runtime.status.mode = options.mode;
        runtime.status.routing_mode = options.routing_mode;
        runtime.status.last_error = None;
        runtime.status.message = if tun_on {
            format!("{} TUN 已热开启（{elapsed_ms} ms，未重启进程）", kernel.display_name())
        } else {
            format!("{} TUN 已热关闭（{elapsed_ms} ms，未重启进程）", kernel.display_name())
        };
        tracing::info!(kernel = %kernel.display_name(), tun_on, elapsed_ms, "TUN 热切换生效");
        Ok(true)
    }

    /// 分流出站模式热切换：代理运行中动态切换「规则 / 全局 / 直连」等模式，
    /// 无论配合 Mihomo 还是 VP-Core 均通过内核 REST 控制器 `PATCH /configs {"mode": "..."}`
    /// 实现微秒/毫秒级无感热切，不重启进程、不断开已有 TCP/UDP 连接。
    /// 若内核不支持或热切换失败返回 Ok(false)。
    pub async fn try_hot_switch_routing_mode(&self, routing_mode: RoutingMode) -> Result<bool> {
        let started = tokio::time::Instant::now();
        let (kernel, config_path, controller_port, controller_secret) = {
            let mut runtime = self.runtime.lock().await;
            if !matches!(
                runtime.status.state,
                ProxyState::Running | ProxyState::Degraded
            ) {
                runtime.options.routing_mode = routing_mode;
                runtime.status.routing_mode = routing_mode;
                return Ok(true);
            }
            let Some(kernel) = runtime.status.kernel else {
                return Ok(false);
            };
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().is_ok_and(|status| status.is_none()),
                None => false,
            };
            if !alive {
                return Ok(false);
            }
            let Some(work_dir) = &runtime.work_dir else {
                return Ok(false);
            };
            (
                kernel,
                work_dir.config_path(),
                runtime.options.controller_port,
                runtime.options.controller_secret.clone(),
            )
        };

        let mode_str = routing_mode.as_controller_mode();

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_millis(2500))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("创建分流模式热切 HTTP 客户端失败: {error}");
                return Ok(false);
            }
        };

        let hot_applied = match kernel {
            KernelKind::Mihomo | KernelKind::VpCore | KernelKind::Meow => {
                let payload = serde_json::json!({
                    "mode": mode_str
                });
                let url = format!("http://127.0.0.1:{controller_port}/configs");
                let request = with_controller_auth(client.patch(&url).json(&payload), &controller_secret);
                match request.send().await {
                    Ok(response) if response.status().is_success() => {
                        // 同步持久化修改工作目录内的 config.yaml
                        if let Ok(content) = tokio::fs::read_to_string(&config_path).await {
                            if let Ok(re) = regex::Regex::new(r"(?m)^(\s*mode\s*:\s*).*$") {
                                let new_content = re.replace(&content, format!("${{1}}{mode_str}")).to_string();
                                let _ = tokio::fs::write(&config_path, &new_content).await;
                            }
                        }
                        true
                    }
                    Ok(response) => {
                        tracing::warn!(
                            status = %response.status(),
                            kernel = %kernel.display_name(),
                            "内核 PATCH /configs 模式热切失败，尝试回退热重载"
                        );
                        // VP-Core 优雅兜底：若运行的旧二进制无 PATCH 路由，自动重写配置并走 PUT /configs
                        if kernel == KernelKind::VpCore {
                            if let Ok(content) = tokio::fs::read_to_string(&config_path).await {
                                if let Ok(re) = regex::Regex::new(r"(?m)^(\s*mode\s*:\s*).*$") {
                                    let new_content = re.replace(&content, format!("${{1}}{mode_str}")).to_string();
                                    let _ = tokio::fs::write(&config_path, &new_content).await;
                                    let put_url = format!("http://127.0.0.1:{controller_port}/configs?force=true");
                                    let put_payload = controller_reload_payload(&config_path, &new_content);
                                    let put_req = with_controller_auth(client.put(&put_url).json(&put_payload), &controller_secret);
                                    if let Ok(resp) = put_req.send().await {
                                        resp.status().is_success()
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    }
                    Err(error) => {
                        tracing::warn!("请求模式热切失败: {error}");
                        false
                    }
                }
            }
            _ => false,
        };

        if !hot_applied {
            return Ok(false);
        }

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut runtime = self.runtime.lock().await;
        runtime.options.routing_mode = routing_mode;
        if let Some(pending) = runtime.pending_options.as_mut() {
            pending.routing_mode = routing_mode;
        }
        runtime.status.routing_mode = routing_mode;
        runtime.status.last_error = None;
        runtime.status.message = format!(
            "{} 已免停热切至「{}」模式（{} ms）",
            kernel.display_name(),
            routing_mode.short_label(),
            elapsed_ms
        );
        tracing::info!(kernel = %kernel.display_name(), ?routing_mode, elapsed_ms, "分流模式热切生效");
        Ok(true)
    }

    pub async fn start(&self, node: &Node) -> Result<()> {
        self.connect(node, None).await
    }

    pub async fn start_with_options(&self, node: &Node, options: MihomoConfigOptions) -> Result<()> {
        options.validate()?;
        let _operation = self.operation.lock().await;
        {
            let mut runtime = self.runtime.lock().await;
            runtime.request_start(node, Some(options.kernel_preference));
            runtime.apply_options(options.clone());
            runtime.status.mode = options.mode;
            runtime.status.routing_mode = options.routing_mode;
            runtime.status.mixed_port = options.mixed_port;
            runtime.status.controller_port = options.controller_port;
            runtime.status.controller_secret = options.controller_secret.clone();
        }
        let running = matches!(
            self.runtime.lock().await.status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        if running {
            self.restart_inner(node, Some(options.kernel_preference), true).await
        } else {
            self.start_inner_with_preference(node, None, Some(options.kernel_preference), true).await
        }
    }

    /// Login recovery reuses the previously authorized service. Any temporary
    /// service-start race is retried by maintain(), without an authorization loop.
    pub async fn start_automatically(&self, node: &Node) -> Result<()> {
        let _operation = self.operation.lock().await;
        {
            let mut runtime = self.runtime.lock().await;
            if runtime.generation != 0
                || !matches!(runtime.status.state, ProxyState::Stopped | ProxyState::Failed)
            {
                return Ok(());
            }
            runtime.request_start(node, None);
        }
        self.start_inner_with_preference(node, None, None, false).await
    }

    pub fn connect<'a>(
        &'a self,
        node: &'a Node,
        kernel_override: Option<KernelPreference>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let _operation = self.operation.lock().await;
            self.runtime.lock().await.request_start(node, kernel_override);
            let running = matches!(
                self.runtime.lock().await.status.state,
                ProxyState::Running | ProxyState::Degraded
            );
            if running {
                self.restart_inner(node, kernel_override, true).await
            } else {
                self.start_inner_with_preference(node, None, kernel_override, true)
                    .await
            }
        })
    }

    async fn start_inner_with_preference(
        &self,
        node: &Node,
        preserved_system_proxy_snapshot: Option<SystemProxySnapshot>,
        kernel_override: Option<KernelPreference>,
        allow_install: bool,
    ) -> Result<()> {
        #[cfg(not(target_os = "macos"))]
        let _ = allow_install;
        let options = {
            let mut runtime = self.runtime.lock().await;
            if let Some(pending) = runtime.pending_options.take() {
                runtime.apply_options(pending);
            }
            runtime.options.clone()
        };
        let preference = kernel_override.unwrap_or(options.kernel_preference);
        let (kernel, fallback_notice) = match select_proxy_kernel_preference_for_node(
            node,
            preference,
            options.mode.has_system(),
            options.mode.has_tun(),
        ) {
            Ok(res) => res,
            Err(error) => {
                self.set_failed(error.to_string()).await;
                return Err(error);
            }
        };
        let Some(path) = find_kernel_executable(kernel) else {
            let message = format!("未找到 {} 可执行文件", kernel.display_name());
            self.set_failed(message.clone()).await;
            return Err(anyhow!(message));
        };
        self.start_with_path_inner(node, kernel, &path, preserved_system_proxy_snapshot, allow_install)
            .await?;
        if let Some(notice) = fallback_notice {
            let mut rt = self.runtime.lock().await;
            rt.status.message = format!("{}（{}）", rt.status.message, notice);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn start_with_path(&self, node: &Node, mihomo_path: &Path) -> Result<()> {
        self.start_with_kernel_path(node, KernelKind::Mihomo, mihomo_path)
            .await
    }

    #[cfg(test)]
    pub(crate) async fn start_with_kernel_path(
        &self,
        node: &Node,
        kernel: KernelKind,
        executable_path: &Path,
    ) -> Result<()> {
        let _operation = self.operation.lock().await;
        self.start_with_path_inner(node, kernel, executable_path, None, true)
            .await
    }

    async fn start_with_path_inner(
        &self,
        node: &Node,
        kernel: KernelKind,
        executable_path: &Path,
        mut preserved_system_proxy_snapshot: Option<SystemProxySnapshot>,
        allow_install: bool,
    ) -> Result<()> {
        let mut runtime = self.runtime.lock().await;
        if matches!(
            runtime.status.state,
            ProxyState::Starting | ProxyState::Running | ProxyState::Stopping
        ) {
            return Err(anyhow!("代理正在运行或切换状态"));
        }
        #[cfg(windows)]
        if runtime.options.mode.has_tun() && !crate::platform::is_elevated() {
            return Err(anyhow!("TUN 模式需要管理员权限"));
        }
        #[cfg(target_os = "linux")]
        if runtime.options.mode.has_tun() && !crate::platform::kernel_has_tun_capability(executable_path) {
            let cmd = crate::platform::tun_capability_command(executable_path);
            let message = format!(
                "TUN 模式启动失败：内核进程缺少 CAP_NET_ADMIN 权限。\n\
                 请在终端执行一次授权命令（无需使用 sudo 启动 GUI）：\n\
                 {cmd}"
            );
            mark_failed(&mut runtime, message.clone());
            return Err(anyhow!(message));
        }
        #[cfg(target_os = "macos")]
        if runtime.options.mode.has_tun() && kernel != KernelKind::Mihomo {
            let message = "macOS TUN 服务当前支持 Mihomo，请将代理内核设为 Mihomo".to_string();
            mark_failed(&mut runtime, message.clone());
            return Err(anyhow!(message));
        }
        // 分流数据库没备好就别急着起内核：半截库会让内核直接拒绝启动，而且报出来的
        // 是一句要人复制才能看懂的长错误。这里缺就补（镜像优先，其次系统代理）。
        // 不限 Mihomo：shoes 的 geosite/geoip 规则同样引用这份库（shared mihomo 目录）。
        if runtime.options.uses_geo() {
            let dir = crate::geo::mihomo_dir();
            if !crate::geo::geo_ready(&dir) {
                let system = crate::system_proxy::current_system_proxy_url();
                if let Err(error) = crate::geo::provision(&dir, system.as_deref(), false).await {
                    tracing::warn!(%error, "分流数据库准备未完全完成，尝试继续启动");
                }
            }
        }
        if kernel == KernelKind::Meow {
            // meow 的 GEOSITE/GEOIP 读 home 目录的 dat 与 Country.mmdb；
            // mmdb 缺失时 meow 会自己走 GitHub 下载，境内大概率卡住启动，这里抢先备好。
            let system = crate::system_proxy::current_system_proxy_url();
            let meow_home = managed_kernel_root().join(KernelKind::Meow.directory_name());
            if let Err(error) = crate::geo::ensure_meow_home_geo(&meow_home, system.as_deref()).await
            {
                tracing::warn!(%error, "meow geo 准备未完全完成，尝试继续启动");
            }
        }
        if kernel == KernelKind::VpCore {
            let system = crate::system_proxy::current_system_proxy_url();
            let vpcore_home = managed_kernel_root().join(KernelKind::VpCore.directory_name());
            if let Err(error) = crate::geo::ensure_vpcore_geo(&vpcore_home, system.as_deref()).await
            {
                tracing::warn!(%error, "vp_core geo 准备未完全完成，尝试继续启动");
            }
        }

        let startup_started = tokio::time::Instant::now();
        let mut startup_deadline = startup_started + STARTUP_PROXY_TOTAL_TIMEOUT;

        runtime.generation = runtime.generation.wrapping_add(1);
        let generation = runtime.generation;
        runtime.status.state = ProxyState::Starting;
        runtime.status.kernel = Some(kernel);
        runtime.status.node_id = Some(node.id.clone());
        runtime.current_node = Some(node.clone());
        runtime.status.message = format!("正在启动 {}", kernel.display_name());
        runtime.status.local_proxy_ready = false;
        runtime.status.last_error = None;

        let port_preflight = startup_port_preflight(
            cfg!(target_os = "macos"),
            crate::platform::is_elevated(),
            runtime.options.mode,
        );
        for port in required_startup_ports(kernel, &runtime.options) {
            if let Err(error) = ensure_startup_port_available(port, port_preflight).await {
                mark_failed(&mut runtime, error.to_string());
                return Err(error);
            }
        }

        let config = match build_kernel_config(kernel, node, runtime.options.clone()) {
            Ok(config) => config,
            Err(error) => {
                mark_failed(&mut runtime, error.to_string());
                return Err(error);
            }
        };
        let config_text = match serialize_kernel_config(kernel, &config) {
            Ok(text) => text,
            Err(error) => {
                let message = format!("生成 {} 配置失败: {error}", kernel.display_name());
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
        };
        let work_dir = match ProxyWorkDir::create(kernel).await {
            Ok(work_dir) => work_dir,
            Err(error) => {
                let message = format!("创建 {} 工作目录失败: {error}", kernel.display_name());
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
        };
        if let Err(error) = tokio::fs::write(work_dir.config_path(), config_text).await {
            let _ = work_dir.cleanup().await;
            mark_failed(
                &mut runtime,
                format!("写入 {} 配置失败: {error}", kernel.display_name()),
            );
            return Err(error.into());
        }

        // We skip dry-run validation here to avoid spawning a heavy process and saving 300-800ms.
        // If config is invalid, the kernel will fail fast on launch and wait_for_port reads the diagnostic log.

        let log = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(work_dir.log_path())
        {
            Ok(log) => log,
            Err(error) => {
                let _ = work_dir.cleanup().await;
                let message = format!("创建 {} 日志失败: {error}", kernel.display_name());
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
        };
        let stderr = match log.try_clone() {
            Ok(stderr) => stderr,
            Err(error) => {
                let _ = work_dir.cleanup().await;
                let message = format!("打开 {} 日志失败: {error}", kernel.display_name());
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
        };
        let requires_elevation = cfg!(any(windows, target_os = "macos"))
            && runtime.options.mode.has_tun()
            && !crate::platform::is_elevated();

        let mut child = if requires_elevation {
            #[cfg(any(windows, target_os = "macos"))]
            {
                tracing::info!(
                    kernel = kernel.display_name(),
                    "TUN 模式需管理员权限，通过无头提权 Supervisor 静默拉起内核..."
                );
                let executable = executable_path.to_owned();
                let data = work_dir.data_root().to_owned();
                let config = work_dir.config_path();
                let log_path = work_dir.log_path();
                // The OS authorization dialog must not block Tokio's worker threads.
                let elevated = tokio::task::spawn_blocking(move || {
                    #[cfg(target_os = "macos")]
                    if !allow_install {
                        return crate::platform::macos_helper::start_existing(
                            kernel, &executable, &data, &config, &log_path,
                        );
                    }
                    crate::platform::start_elevated_kernel_runner(
                        kernel, &executable, &data, &config, &log_path,
                    )
                }).await.unwrap_or_else(|error| Err(anyhow!("启动 TUN 后台服务失败: {error}")));
                // First installation may take longer than the normal startup deadline.
                startup_deadline = tokio::time::Instant::now() + STARTUP_PROXY_TOTAL_TIMEOUT;
                match elevated {
                    Ok(elevated) => {
                        runtime.status.kernel_pid = elevated.id();
                        ProxyChild::Elevated(elevated)
                    }
                    Err(error) => {
                        let _ = work_dir.cleanup().await;
                        let message = format!("无头提权启动 {} 失败: {error}", kernel.display_name());
                        mark_failed(&mut runtime, message);
                        return Err(error);
                    }
                }
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            {
                unreachable!()
            }
        } else {
            let mut command = Command::new(executable_path);
            configure_kernel_command(&mut command, kernel, &work_dir);
            command
                .kill_on_drop(true)
                .stdin(Stdio::null())
                .stdout(Stdio::from(log))
                .stderr(Stdio::from(stderr));
            #[cfg(windows)]
            {
                command.creation_flags(0x0800_0000);
            }

            match command.spawn() {
                Ok(child) => {
                    if let Some(pid) = child.id() {
                        crate::platform::assign_child_process_id(pid);
                        runtime.status.kernel_pid = Some(pid);
                    }
                    ProxyChild::Standard(child)
                }
                Err(error) => {
                    let _ = work_dir.cleanup().await;
                    let message = format!("启动 {} 失败: {error}", kernel.display_name());
                    mark_failed(&mut runtime, message.clone());
                    return Err(anyhow!(message));
                }
            }
        };

        tracing::info!(
            kernel = kernel.display_name(),
            stage = "process_spawned",
            elapsed_ms = startup_started.elapsed().as_millis() as u64,
            "代理启动阶段完成"
        );
        for port in required_startup_ports(kernel, &runtime.options) {
            let startup_timeout =
                startup_deadline.saturating_duration_since(tokio::time::Instant::now());
            let readiness = wait_for_port(&mut child, port, startup_timeout).await;
            if !matches!(readiness, Ok(true)) {
                let readiness_error = readiness.err();
                // 先探测内核进程在超时前是否已自行退出
                let exit_status = child.try_wait().ok().flatten();
                if exit_status.is_none() {
                    let _ = child.kill().await;
                }
                let _ = child.wait().await;
                // 留 50ms 缓冲让操作系统完成文件写入缓冲区刷盘
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

                let detail = tokio::fs::read_to_string(work_dir.log_path())
                    .await
                    .unwrap_or_default();
                let _ = work_dir.cleanup().await;
                let detail_trimmed = detail.trim();

                let fatal_line = detail_trimmed
                    .lines()
                    .rev()
                    .find(|l| {
                        let lower = l.to_lowercase();
                        if lower.contains("[warning]")
                            || lower.contains("[info]")
                            || lower.contains("level=warn")
                            || lower.contains("level=info")
                        {
                            return false;
                        }
                        lower.contains("level=fatal")
                            || lower.contains("level=error")
                            || lower.contains("fatal:")
                            || lower.contains("panic:")
                            || lower.contains("error:")
                            || lower.contains("bind:")
                            || lower.contains("failed")
                    });

                let is_not_found = detail.contains("not found in geosite.dat")
                    || detail.contains("not found in geoip.dat")
                    || detail.contains("not found in")
                    || (detail.contains("category") && detail.contains("not found"));

                let message = if let Some(error) = readiness_error {
                    format!("{} 启动失败: {error:#}", kernel.display_name())
                } else if is_not_found {
                    format!(
                        "{} 启动失败：分流规则中引用了不存在的分类（已保留规则库，无需重新下载）。请检查自定义规则或切换默认推荐规则。",
                        kernel.display_name()
                    )
                } else if (detail.contains("failed to decode geodata file")
                    || detail.contains("Geosite.dat invalid")
                    || detail.contains("GeoIP.dat invalid")
                    || (detail.contains("cannot parse") && detail.contains("geo")))
                    && !is_not_found
                {
                    crate::geo::purge_corrupt_geo_files(&crate::geo::mihomo_dir());
                    "Mihomo 分流规则文件 (GeoSite/GeoIP) 损坏，已自动清理损坏文件。请在「规则设置」点击「更新规则文件」重新同步完整库后再启动内核。".to_string()
                } else if let Some(err) = fatal_line {
                    format!("{} 启动失败: {}", kernel.display_name(), err.trim())
                } else if !detail_trimmed.is_empty() {
                    if let Some(status) = exit_status {
                        format!(
                            "{} 启动失败 (退出码 {:?}): {}",
                            kernel.display_name(),
                            status.code(),
                            last_nonempty_line(detail_trimmed)
                        )
                    } else {
                        format!(
                            "{} 启动失败: {}",
                            kernel.display_name(),
                            last_nonempty_line(detail_trimmed)
                        )
                    }
                } else if let Some(status) = exit_status {
                    format!(
                        "{} 启动失败：内核进程在就绪前异常退出 (退出码 {:?})",
                        kernel.display_name(),
                        status.code()
                    )
                } else {
                    format!(
                        "{} 启动后端口 {port} 未就绪（启动超时 {} 秒）",
                        kernel.display_name(),
                        STARTUP_PROXY_TOTAL_TIMEOUT.as_secs()
                    )
                };
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
        }

        tracing::info!(
            kernel = kernel.display_name(),
            stage = "port_ready",
            elapsed_ms = startup_started.elapsed().as_millis() as u64,
            "代理启动阶段完成"
        );

        if runtime.options.mode.has_tun() {
            // 给内核轻微缓冲时间尝试完成 TUN 网卡设备创建
            tokio::time::sleep(Duration::from_millis(100)).await;

            // 1. 扫描内核日志中是否有明确的 TUN 权限/创建致命错误
            let log_detail = tokio::fs::read_to_string(work_dir.log_path())
                .await
                .unwrap_or_default();
            if port_preflight == StartupPortPreflight::ListenerOnly {
                if let Some(detail) = startup_tcp_listener_bind_error(
                    &log_detail,
                    &required_startup_ports(kernel, &runtime.options),
                ) {
                    // A different process can win the port after preflight. A
                    // reachable port plus a live core does not prove ownership:
                    // Mihomo logs listener bind errors while continuing to run.
                    let message = format!("{} 监听端口绑定失败: {detail}", kernel.display_name());
                    runtime.child = Some(child);
                    runtime.work_dir = Some(work_dir);
                    drop(runtime);
                    if let Err(error) = self.stop_inner_for_restart().await {
                        return Err(anyhow!("{message}；停止失败，已保留会话供重试: {error:#}"));
                    }
                    self.set_failed(message.clone()).await;
                    return Err(anyhow!(message));
                }
            }
            let lower_log = log_detail.to_lowercase();
            if lower_log.contains("tunsetiff")
                || lower_log.contains("start tun error")
                || lower_log.contains("start tun listening error")
                || lower_log.contains("create tun interface error")
                || (lower_log.contains("tun") && lower_log.contains("operation not permitted"))
                || (lower_log.contains("tun") && lower_log.contains("permission denied"))
                || lower_log.contains("failed to start tun")
                || lower_log.contains("create tun adapter failed")
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = work_dir.cleanup().await;
                #[cfg(target_os = "macos")]
                let message = format!(
                    "TUN 虚拟网卡创建失败，请检查 macOS 后台服务与内核日志：{}",
                    last_nonempty_line(&log_detail)
                );
                #[cfg(not(target_os = "macos"))]
                let cmd = crate::platform::tun_capability_command(executable_path);
                #[cfg(not(target_os = "macos"))]
                let message = format!(
                    "TUN 模式启动失败：内核未能创建虚拟网卡（缺少系统网络权限）。\n\
                     请在 Linux 终端执行一次授权命令为内核授权：\n\
                     {cmd}\n\
                     （若在 Windows 上，请以管理员身份运行本程序）"
                );
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }

            // 2. Linux 平台：真实网络接口存在性校验
            #[cfg(target_os = "linux")]
            if !crate::platform::check_linux_tun_interface_active() {
                // 如果 /sys/class/net 中暂未找到 Meta/tun0/utun，复核一次日志
                tokio::time::sleep(Duration::from_millis(150)).await;
                if !crate::platform::check_linux_tun_interface_active() {
                    let fresh_log = tokio::fs::read_to_string(work_dir.log_path())
                        .await
                        .unwrap_or_default();
                    if fresh_log.contains("level=error") || fresh_log.contains("level=fatal") {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = work_dir.cleanup().await;
                        let cmd = crate::platform::tun_capability_command(executable_path);
                        let message = format!(
                            "TUN 模式启动失败：系统虚拟网卡未成功创建。\n\
                             请确认已通过终端命令为内核授权：\n\
                             {cmd}"
                        );
                        mark_failed(&mut runtime, message.clone());
                        return Err(anyhow!(message));
                    }
                }
            }
        }

        if runtime.options.mode.has_system() && self.system_proxy.supported() {
            if tokio::time::Instant::now() >= startup_deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = work_dir.cleanup().await;
                let message = format!(
                    "{} 启动超过 {} 秒，系统代理未启用",
                    kernel.display_name(),
                    STARTUP_PROXY_TOTAL_TIMEOUT.as_secs()
                );
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
            if let Some(snapshot) = preserved_system_proxy_snapshot.take() {
                runtime.system_proxy_snapshot = Some(snapshot);
            } else {
                let server = format!("127.0.0.1:{}", runtime.options.mixed_port);
                let terminal_port = runtime
                    .options
                    .terminal_proxy
                    .then_some(runtime.options.mixed_port);
                match activate_system_proxy(self.system_proxy.clone(), server, terminal_port).await
                {
                    Ok(snapshot) => runtime.system_proxy_snapshot = Some(snapshot),
                    Err(error) => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        let _ = work_dir.cleanup().await;
                        let message = format!("启用系统代理失败: {error}");
                        mark_failed(&mut runtime, message.clone());
                        return Err(anyhow!(message));
                    }
                }
            }
            tracing::info!(
                kernel = kernel.display_name(),
                stage = "system_proxy_enabled",
                elapsed_ms = startup_started.elapsed().as_millis() as u64,
                "代理启动阶段完成"
            );
        }

        if runtime.options.terminal_proxy && (!runtime.options.mode.has_system() || !self.system_proxy.supported()) {
            let port = runtime.options.mixed_port;
            tokio::task::spawn_blocking(move || {
                match crate::system_proxy::terminal::enable(port) {
                    Ok(report) => tracing::info!(summary = report.summary(), "终端代理已启用"),
                    Err(error) => tracing::warn!(%error, "终端代理启用失败"),
                }
            });
        }

        let elapsed_ms = startup_started.elapsed().as_millis() as u64;
        runtime.status.state = ProxyState::Running;
        runtime.status.routing_mode = runtime.options.routing_mode;
        // 这个字段的意思是"本地 mixed 端口已经就绪"，必须等端口检查通过后才置真，
        // 不是"系统代理归我们"。归属由 `system_proxy_enabled` 表达，别在这儿混用。
        runtime.status.local_proxy_ready = true;
        runtime.status.system_proxy_enabled = runtime.system_proxy_snapshot.is_some();
        runtime.status.last_error = None;
        runtime.recovery_pending = false;
        runtime.recovery_due = None;
        runtime.stable_since = Some(tokio::time::Instant::now());
        runtime.recovery_waiting_for_route = false;
        runtime.status.message = start_success_message(
            kernel.display_name(),
            elapsed_ms,
            runtime.options.mode,
            runtime.system_proxy_snapshot.is_some(),
            self.system_proxy.supported(),
            runtime.options.mixed_port,
        );

        if VERIFY_CONNECTIVITY_ON_START {
            let runtime_clone = self.runtime.clone();
            let mixed_port = runtime.options.mixed_port;
            let kernel_name = kernel.display_name().to_string();
            let log_path = work_dir.log_path().to_path_buf();
            let manager_clone = self.clone();
            let current_node = node.clone();
            tokio::spawn(async move {
                // 等待内核加载并与远程服务器建立初始连接
                tokio::time::sleep(Duration::from_millis(250)).await;
                if let Err(error) = verify_local_proxy(mixed_port, STARTUP_PROXY_VERIFY_TIMEOUT.max(Duration::from_secs(6))).await {
                    let detail = tokio::fs::read_to_string(&log_path)
                        .await
                        .unwrap_or_default();
                    let diagnostic = format_kernel_diagnostic(&detail);
                    if diagnostic.contains("规则加载中")
                        || diagnostic.contains("自动建立连接")
                        || diagnostic.contains("start initial configuration")
                    {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        if verify_local_proxy(mixed_port, Duration::from_secs(3)).await.is_ok() {
                            tracing::info!(kernel = %kernel_name, "内核初始握手完成，连通性验证通过");
                            return;
                        }
                    }

                    // 如果当前是 Meow 发生连通性超时，触发自动熔断保护，平滑切换到工业级 Mihomo 接管
                    if kernel == KernelKind::Meow && find_kernel_executable(KernelKind::Mihomo).is_some() {
                        let _operation = manager_clone.operation.lock().await;
                        if runtime_clone.lock().await.generation != generation {
                            return;
                        }
                        tracing::warn!(
                            kernel = %kernel_name,
                            %error,
                            "Meow 内核网络连接全部超时，触发熔断保护，自动平滑切换至 Mihomo"
                        );
                        if let Ok(()) = manager_clone.restart_inner(&current_node, Some(KernelPreference::Mihomo), false).await {
                            let mut rt = runtime_clone.lock().await;
                            if rt.status.state == ProxyState::Running {
                                rt.status.message = "Meow 连通超时，已自动触发熔断切换至 Mihomo 接管网络".to_string();
                            }
                            return;
                        }
                    }

                    let mut rt = runtime_clone.lock().await;
                    if rt.generation == generation && rt.status.state == ProxyState::Running {
                        let message = if detail.trim().is_empty() {
                            format!("节点连通性验证失败: {error}")
                        } else {
                            format!("节点连通性验证失败: {error}; {kernel_name}: {diagnostic}")
                        };
                        tracing::warn!(kernel = %kernel_name, %message, "后台连通性验证未通过，置为降级运行");
                        rt.apply_connectivity_result(generation, Some(message));
                    }
                } else {
                    runtime_clone.lock().await.apply_connectivity_result(generation, None);
                    tracing::info!(
                        kernel = %kernel_name,
                        stage = "connectivity_verified",
                        "后台连通性验证通过"
                    );
                }
            });
        }

        runtime.child = Some(child);
        runtime.work_dir = Some(work_dir);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.runtime.lock().await.cancel_recovery();
        let _operation = self.operation.lock().await;
        // A start queued ahead of Stop may have recreated its intent while we
        // waited. Stop must remain the last word once it owns the operation lock.
        self.runtime.lock().await.cancel_recovery();
        self.stop_inner().await
    }

    async fn stop_inner(&self) -> Result<()> {
        self.stop_inner_with_system_proxy_restore(true)
            .await
            .map(|_| ())
    }

    pub(crate) async fn stop_inner_for_restart(&self) -> Result<Option<SystemProxySnapshot>> {
        self.stop_inner_with_system_proxy_restore(false).await
    }

    async fn stop_inner_with_system_proxy_restore(
        &self,
        restore_proxy: bool,
    ) -> Result<Option<SystemProxySnapshot>> {
        let (mut child, work_dir, snapshot, options, kernel) = {
            let mut runtime = self.runtime.lock().await;
            runtime.generation = runtime.generation.wrapping_add(1);
            let kernel = runtime.status.kernel;
            runtime.status.state = ProxyState::Stopping;
            runtime.status.message = format!(
                "正在停止 {}",
                kernel.map(KernelKind::display_name).unwrap_or("代理内核")
            );
            (
                runtime.child.take(),
                runtime.work_dir.take(),
                runtime.system_proxy_snapshot.take(),
                runtime.options.clone(),
                kernel,
            )
        };
        let kernel_name = kernel.map(KernelKind::display_name).unwrap_or("代理内核");

        let mut errors = Vec::new();
        if let Some(process) = child.as_mut() {
            let exited = matches!(process.try_wait(), Ok(Some(_)));
            if !exited {
                // A missed helper status RPC does not mean Stop should be skipped.
                match tokio::time::timeout(Duration::from_secs(3), process.kill()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::warn!(%error, "停止内核请求失败，继续确认退出状态"),
                    Err(_) => tracing::warn!("停止内核请求超时，继续确认退出状态"),
                }
            }
            let stopped = tokio::time::timeout(Duration::from_secs(3), process.wait()).await;
            let stop_error = match stopped {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(format!("等待 {kernel_name} 退出失败: {error}")),
                Err(_) => Some(format!("等待 {kernel_name} 退出超时")),
            };
            if let Some(message) = stop_error {
                // Never discard an unconfirmed session: the root helper may still
                // own a live TUN. Keep all handles/configs so the next Stop retries.
                let mut runtime = self.runtime.lock().await;
                runtime.child = child;
                runtime.work_dir = work_dir;
                runtime.system_proxy_snapshot = snapshot;
                runtime.status.state = ProxyState::Degraded;
                runtime.status.last_error = Some(message.clone());
                runtime.status.message = format!("停止尚未确认，已保留会话供重试: {message}");
                return Err(anyhow!(message));
            }
        }
        if let Some(work_dir) = work_dir {
            if let Err(error) = work_dir.cleanup().await {
                errors.push(format!("清理 {kernel_name} 工作目录失败: {error}"));
            }
        }
        let preserved_snapshot = if restore_proxy {
            let had_snapshot = snapshot.is_some();
            if let Some(snapshot) = snapshot {
                if let Err(error) = restore_system_proxy(
                    self.system_proxy.clone(),
                    snapshot,
                    options.terminal_proxy,
                )
                .await
                {
                    errors.push(format!("恢复系统代理失败: {error}"));
                }
            }
            #[cfg(windows)]
            if options.mode.has_tun() {
                tokio::task::spawn_blocking(crate::platform::self_heal_stale_dns);
            }
            if !had_snapshot && options.terminal_proxy {
                tokio::task::spawn_blocking(crate::system_proxy::terminal::disable);
            }
            None
        } else {
            snapshot
        };

        if !errors.is_empty() {
            if let Some(snapshot) = preserved_snapshot.as_ref() {
                if let Err(error) = restore_system_proxy(
                    self.system_proxy.clone(),
                    snapshot.clone(),
                    options.terminal_proxy,
                )
                .await
                {
                    errors.push(format!("停止失败后恢复系统代理失败: {error}"));
                }
            }
            let message = errors.join("；");
            let mut runtime = self.runtime.lock().await;
            mark_failed(&mut runtime, message.clone());
            return Err(anyhow!(message));
        }

        let mut runtime = self.runtime.lock().await;
        runtime.status = ProxyRuntimeStatus::stopped(options);
        runtime.current_node = None;
        Ok(preserved_snapshot)
    }

    pub async fn switch_node(&self, node: &Node) -> Result<()> {
        let _operation = self.operation.lock().await;
        self.runtime.lock().await.request_start(node, None);
        if self.try_hot_reload_mihomo(node).await? {
            return Ok(());
        }
        self.restart_inner(node, None, true).await
    }

    pub(crate) async fn try_hot_reload_mihomo(&self, node: &Node) -> Result<bool> {
        let (config_path, controller_port, controller_secret, options) = {
            let mut runtime = self.runtime.lock().await;
            if !matches!(
                runtime.status.state,
                ProxyState::Running | ProxyState::Degraded
            ) {
                return Ok(false);
            }
            if runtime.status.kernel != Some(KernelKind::Mihomo) {
                return Ok(false);
            }
            #[cfg(target_os = "macos")]
            if runtime.options.mode.has_tun()
                && !crate::platform::is_elevated()
                && !matches!(runtime.child.as_ref(), Some(ProxyChild::Elevated(_)))
            {
                return Ok(false);
            }
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().is_ok_and(|status| status.is_none()),
                None => false,
            };
            if !alive {
                return Ok(false);
            }
            let Some(work_dir) = &runtime.work_dir else {
                return Ok(false);
            };
            (
                work_dir.config_path(),
                runtime.options.controller_port,
                runtime.options.controller_secret.clone(),
                runtime.options.clone(),
            )
        };

        let started = tokio::time::Instant::now();
        let config = match build_kernel_config(KernelKind::Mihomo, node, options) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("构建热重载配置失败: {error}");
                return Ok(false);
            }
        };
        let config_text = match serialize_kernel_config(KernelKind::Mihomo, &config) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!("序列化热重载配置失败: {error}");
                return Ok(false);
            }
        };

        if let Err(error) = tokio::fs::write(&config_path, &config_text).await {
            tracing::warn!("写入热重载配置文件失败: {error}");
            return Ok(false);
        }

        let client = match reqwest::Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("创建热重载 HTTP 客户端失败: {error}");
                return Ok(false);
            }
        };

        let url = format!("http://127.0.0.1:{controller_port}/configs?force=true");
        let payload = controller_reload_payload(&config_path, &config_text);
        let request = with_controller_auth(client.put(&url).json(&payload), &controller_secret);
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let mut runtime = self.runtime.lock().await;
                runtime.status.node_id = Some(node.id.clone());
                runtime.current_node = Some(node.clone());
                runtime.status.state = ProxyState::Running;
                runtime.status.routing_mode = runtime.options.routing_mode;
                runtime.status.local_proxy_ready = true;
                runtime.status.last_error = None;
                runtime.status.message = format!(
                    "Mihomo 运行中（平滑切换至「{}」，耗时 {elapsed_ms} ms）",
                    node.name
                );
                tracing::info!(
                    node_id = %node.id,
                    node_name = %node.name,
                    elapsed_ms,
                    "Mihomo 热重载平滑切换节点成功"
                );
                Ok(true)
            }
            Ok(response) => {
                tracing::warn!(
                    status = %response.status(),
                    "Mihomo 热重载 API 返回失败状态，回退进程重启"
                );
                Ok(false)
            }
            Err(error) => {
                tracing::warn!("请求 Mihomo 热重载 API 失败: {error}，回退进程重启");
                Ok(false)
            }
        }
    }

    fn restart_inner<'a>(
        &'a self,
        node: &'a Node,
        kernel_override: Option<KernelPreference>,
        allow_install: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
        let restart_started = tokio::time::Instant::now();
        let snapshot = self.stop_inner_for_restart().await?;
        tracing::info!(
            stage = "old_kernel_stopped",
            elapsed_ms = restart_started.elapsed().as_millis() as u64,
            "代理重启阶段完成"
        );
        match self
            .start_inner_with_preference(node, snapshot.clone(), kernel_override, allow_install)
            .await
        {
            Ok(()) => {
                let elapsed_ms = restart_started.elapsed().as_millis() as u64;
                let mut runtime = self.runtime.lock().await;
                if let Some(kernel) = runtime.status.kernel {
                    runtime.status.message = format!(
                        "{} 运行中（切换耗时 {elapsed_ms} ms）",
                        kernel.display_name()
                    );
                }
                Ok(())
            }
            Err(error) => {
                if let Some(snapshot) = snapshot {
                    let mut runtime = self.runtime.lock().await;
                    runtime.system_proxy_snapshot = Some(snapshot);
                    runtime.status.state = ProxyState::Degraded;
                    runtime.status.local_proxy_ready = false;
                    runtime.status.system_proxy_enabled = true;
                    runtime.status.last_error = Some(error.to_string());
                    runtime.status.message = format!("切换失败，系统代理保持开启: {error}");
                }
                Err(error)
            }
        }
        })
    }

    pub async fn recent_log(&self, max_chars: usize) -> Result<String> {
        let log_path = self
            .runtime
            .lock()
            .await
            .work_dir
            .as_ref()
            .map(ProxyWorkDir::log_path);
        let Some(log_path) = log_path else {
            return Ok(String::new());
        };
        let text = tokio::fs::read_to_string(log_path).await?;
        let text = strip_ansi(&text);
        if text.chars().count() <= max_chars {
            return Ok(text);
        }
        Ok(text
            .chars()
            .rev()
            .take(max_chars)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect())
    }

    async fn set_failed(&self, message: String) {
        let mut runtime = self.runtime.lock().await;
        mark_failed(&mut runtime, message);
    }
}

pub fn build_kernel_config(
    kernel: KernelKind,
    node: &Node,
    options: MihomoConfigOptions,
) -> Result<Value> {
    kernel_adapter(kernel).build_config(node, options)
}

pub fn serialize_kernel_config(kernel: KernelKind, config: &Value) -> Result<String> {
    kernel_adapter(kernel).serialize_config(config)
}

pub fn configure_kernel_command(command: &mut Command, kernel: KernelKind, work_dir: &ProxyWorkDir) {
    kernel_adapter(kernel).configure_command(command, work_dir);
}

pub(crate) async fn wait_for_port(child: &mut ProxyChild, port: u16, timeout: Duration) -> Result<bool> {
    wait_for_port_with_status(|| child.try_wait(), port, timeout).await
}

async fn wait_for_port_with_status(
    mut try_wait: impl FnMut() -> std::io::Result<Option<std::process::ExitStatus>>,
    port: u16,
    timeout: Duration,
) -> Result<bool> {
    let started = tokio::time::Instant::now();
    while started.elapsed() < timeout {
        if try_wait().context("读取内核或 TUN 后台服务状态失败")?.is_some() {
            return Ok(false);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        let connect_timeout = remaining.min(STARTUP_PORT_CONNECT_TIMEOUT);
        if tokio::time::timeout(connect_timeout, TcpStream::connect(("127.0.0.1", port)))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(false)
}

#[cfg(test)]
mod port_readiness_tests {
    use super::*;

    #[tokio::test]
    async fn reports_status_errors_even_when_the_port_accepts_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let error = wait_for_port_with_status(
            || Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "helper disconnected")),
            port,
            Duration::from_secs(1),
        ).await.unwrap_err();

        let message = format!("{error:#}");
        assert!(message.contains("读取内核或 TUN 后台服务状态失败"), "{message}");
        assert!(message.contains("helper disconnected"), "{message}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_exited_child_does_not_claim_an_open_port() {
        use std::os::unix::process::ExitStatusExt;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let ready = wait_for_port_with_status(
            || Ok(Some(std::process::ExitStatus::from_raw(7 << 8))),
            port,
            Duration::from_secs(1),
        ).await.unwrap();

        assert!(!ready);
    }

    #[tokio::test]
    async fn a_live_child_needs_an_open_port_before_it_is_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(wait_for_port_with_status(|| Ok(None), port, Duration::from_secs(1))
            .await.unwrap());

        drop(listener);
        assert!(!wait_for_port_with_status(|| Ok(None), port, Duration::from_millis(30))
            .await.unwrap());
    }

    #[test]
    fn listener_only_preflight_is_limited_to_unprivileged_macos_tun() {
        for macos in [false, true] {
            for elevated in [false, true] {
                for mode in [ProxyMode::None, ProxyMode::System, ProxyMode::Tun, ProxyMode::Both] {
                    let expected = if macos && !elevated && mode.has_tun() {
                        StartupPortPreflight::ListenerOnly
                    } else {
                        StartupPortPreflight::Bind
                    };
                    assert_eq!(startup_port_preflight(macos, elevated, mode), expected);
                }
            }
        }
    }

    #[tokio::test]
    async fn elevated_macos_preflight_rejects_a_real_listener_and_accepts_refusal() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let preflight = startup_port_preflight(true, false, ProxyMode::Tun);
        let error = ensure_startup_port_available(port, preflight).await.unwrap_err();
        assert!(error.to_string().contains("已有 TCP 服务正在监听"));
        drop(listener);
        ensure_startup_port_available(port, preflight).await.unwrap();
    }

    #[tokio::test]
    async fn cross_uid_bind_conflict_does_not_block_a_root_core_without_a_listener() {
        let bind_called = std::cell::Cell::new(false);
        let result = ensure_startup_port_available_with_probes(
            9980,
            startup_port_preflight(true, false, ProxyMode::Both),
            async {
                bind_called.set(true);
                Err(anyhow!(std::io::Error::from(std::io::ErrorKind::AddrInUse)))
            },
            async { Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)) },
            Duration::from_secs(1),
        ).await;
        assert!(result.is_ok());
        assert!(!bind_called.get(), "the GUI must not bind or run orphan cleanup on this path");
    }

    #[tokio::test]
    async fn other_startup_paths_still_use_the_bind_result() {
        let listener_called = std::cell::Cell::new(false);
        let error = ensure_startup_port_available_with_probes(
            9980,
            startup_port_preflight(true, false, ProxyMode::System),
            async { Err(anyhow!("isolated bind conflict")) },
            async {
                listener_called.set(true);
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            },
            Duration::from_secs(1),
        ).await.unwrap_err();
        assert_eq!(error.to_string(), "isolated bind conflict");
        assert!(!listener_called.get());
    }

    #[tokio::test]
    async fn uncertain_listener_probes_never_report_the_port_as_available() {
        let preflight = startup_port_preflight(true, false, ProxyMode::Tun);
        let denied = ensure_startup_port_available_with_probes(
            9980,
            preflight,
            async { panic!("bind probe must not run") },
            async { Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)) },
            Duration::from_secs(1),
        ).await.unwrap_err();
        assert!(denied.to_string().contains("无法确认代理端口 9980 是否空闲"));
        let timed_out = ensure_startup_port_available_with_probes(
            9980,
            preflight,
            async { panic!("bind probe must not run") },
            std::future::pending::<std::io::Result<()>>(),
            Duration::from_millis(5),
        ).await.unwrap_err();
        assert!(timed_out.to_string().contains("探测超时"));
    }

    #[test]
    fn startup_bind_diagnostics_match_real_mihomo_listener_errors_only() {
        // Captured from the installed Mihomo 1.19.33 using isolated, occupied
        // random loopback ports, with TUN/DNS disabled. The core remained alive.
        let mixed = "time=\"fixture\" level=error msg=\"Start Mixed(http+socks) server error: listen tcp 127.0.0.1:62300: bind: address already in use\"";
        let controller = "time=\"fixture\" level=error msg=\"External controller listen error: listen tcp 127.0.0.1:62302: bind: address already in use\"";
        assert_eq!(startup_tcp_listener_bind_error(mixed, &[62300]), Some(mixed.into()));
        assert_eq!(startup_tcp_listener_bind_error(controller, &[62302]), Some(controller.into()));
        assert!(startup_tcp_listener_bind_error(mixed, &[62301]).is_none());
        assert!(startup_tcp_listener_bind_error(&mixed.replace("level=error", "level=warning"), &[62300]).is_none());
        for line in [
            "time=\"fixture\" level=error msg=\"[TCP] dial node.example:62300 error: bind: address already in use\"",
            "time=\"fixture\" level=error msg=\"node example:62300 failed health check\"",
            "time=\"fixture\" level=info msg=\"Mixed(http+socks) proxy listening at: 127.0.0.1:62300\"",
        ] {
            assert!(startup_tcp_listener_bind_error(line, &[62300]).is_none(), "{line}");
        }
    }

    #[tokio::test]
    async fn a_foreign_open_port_does_not_hide_the_new_cores_bind_failure() {
        let foreign = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = foreign.local_addr().unwrap().port();
        // This reproduces why socket readiness alone cannot establish ownership.
        assert!(wait_for_port_with_status(|| Ok(None), port, Duration::from_secs(1)).await.unwrap());
        let log = format!("time=\"fixture\" level=error msg=\"Start Mixed(http+socks) server error: listen tcp 127.0.0.1:{port}: bind: address already in use\"");
        assert!(startup_tcp_listener_bind_error(&log, &[port]).is_some());
    }
}

pub async fn verify_local_proxy(port: u16, timeout: Duration) -> Result<()> {
    let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_errors = Vec::new();

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let attempt_timeout = remaining.min(Duration::from_millis(3500));
        let client = match reqwest::Client::builder()
            .proxy(proxy.clone())
            .connect_timeout(attempt_timeout)
            .timeout(attempt_timeout)
            .build()
        {
            Ok(c) => c,
            Err(e) => return Err(e.into()),
        };

        let mut tasks = JoinSet::new();
        for url in STARTUP_PROXY_VERIFY_URLS {
            let client = client.clone();
            tasks.spawn(async move {
                let response = client.get(url).send().await?;
                if response.status() != reqwest::StatusCode::NO_CONTENT {
                    return Err(anyhow!("{url} 返回 HTTP {}，预期 204", response.status()));
                }
                Ok::<_, anyhow::Error>(())
            });
        }

        let mut attempt_errors = Vec::new();
        let mut succeeded = false;
        let attempt_deadline = tokio::time::Instant::now() + attempt_timeout;
        while let Ok(Some(res)) = tokio::time::timeout_at(attempt_deadline, tasks.join_next()).await
        {
            match res {
                Ok(Ok(())) => {
                    tasks.abort_all();
                    succeeded = true;
                    break;
                }
                Ok(Err(error)) => attempt_errors.push(error.to_string()),
                Err(error) => attempt_errors.push(format!("验证任务失败: {error}")),
            }
        }
        if succeeded {
            return Ok(());
        }
        last_errors = attempt_errors;
        let sleep_dur = Duration::from_millis(40)
            .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        if sleep_dur.is_zero() {
            break;
        }
        tokio::time::sleep(sleep_dur).await;
    }

    if last_errors.is_empty() {
        last_errors.push(format!("验证超过 {} ms", timeout.as_millis()));
    }
    Err(anyhow!(
        "本地代理无法访问测试地址: {}",
        last_errors.join("；")
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupPortPreflight {
    Bind,
    ListenerOnly,
}

fn startup_port_preflight(macos: bool, elevated: bool, mode: ProxyMode) -> StartupPortPreflight {
    if macos && !elevated && mode.has_tun() {
        StartupPortPreflight::ListenerOnly
    } else {
        StartupPortPreflight::Bind
    }
}

async fn ensure_startup_port_available(port: u16, preflight: StartupPortPreflight) -> Result<()> {
    ensure_startup_port_available_with_probes(
        port,
        preflight,
        ensure_port_available(port),
        async {
            TcpStream::connect(("127.0.0.1", port)).await.map(drop)
        },
        Duration::from_millis(250),
    ).await
}

async fn ensure_startup_port_available_with_probes(
    port: u16,
    preflight: StartupPortPreflight,
    bind_probe: impl std::future::Future<Output = Result<()>>,
    listener_probe: impl std::future::Future<Output = std::io::Result<()>>,
    timeout: Duration,
) -> Result<()> {
    if preflight == StartupPortPreflight::Bind {
        return bind_probe.await;
    }
    // Darwin rejects a non-root bind against another UID's surviving TCP PCB
    // even with SO_REUSEADDR. The root TUN core can still bind that port, so the
    // GUI checks for a real listener and leaves binding to the privileged core.
    match tokio::time::timeout(timeout, listener_probe).await {
        Ok(Ok(())) => Err(anyhow!("代理端口 {port} 已被占用：已有 TCP 服务正在监听")),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => Ok(()),
        Ok(Err(error)) => Err(anyhow!(error).context(format!("无法确认代理端口 {port} 是否空闲"))),
        Err(_) => Err(anyhow!("无法确认代理端口 {port} 是否空闲：TCP 监听探测超时")),
    }
}

fn startup_tcp_listener_bind_error(log: &str, ports: &[u16]) -> Option<String> {
    log.lines().find_map(|line| {
        if !line.contains(" level=error ")
            || !(line.contains("msg=\"Start Mixed(http+socks) server error: listen tcp ")
                || line.contains("msg=\"External controller listen error: listen tcp "))
        {
            return None;
        }
        let (_, listen) = line.split_once("listen tcp ")?;
        let (address, _) = listen.split_once(": bind:")?;
        ports.iter().any(|port| address.ends_with(&format!(":{port}")))
            .then(|| line.to_owned())
    })
}

pub async fn ensure_port_available(port: u16) -> Result<()> {
    let mut last_err = None;
    let max_attempts = if cfg!(test) { 3 } else { 8 };
    for attempt in 0..max_attempts {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                drop(listener);
                return Ok(());
            }
            Err(error) => {
                last_err = Some(error);
                if attempt == 0 {
                    #[cfg(not(test))]
                    {
                        let cleaned = crate::platform::kill_orphan_proxy_cores();
                        if cleaned > 0 {
                            tracing::info!("清理了 {cleaned} 个残留代理内核进程以释放端口 {port}");
                        }
                    }
                }
                #[cfg(test)]
                {
                    tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
                }
                #[cfg(not(test))]
                {
                    let delay_ms = match attempt {
                        0 => 50,
                        1 => 100,
                        2 => 150,
                        3 => 200,
                        _ => 250,
                    };
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }
    let error = last_err
        .map(|e| e.to_string())
        .unwrap_or_else(|| "未知错误".into());

    #[cfg(windows)]
    let occupant_hint = crate::platform::query_port_occupant_hint(port);
    #[cfg(not(windows))]
    let occupant_hint: Option<String> = None;

    if let Some(hint) = occupant_hint {
        Err(anyhow!("代理端口 {port} 已被占用 ({hint}): {error}"))
    } else {
        Err(anyhow!("代理端口 {port} 已被占用: {error}"))
    }
}

pub fn required_startup_ports(kernel: KernelKind, options: &MihomoConfigOptions) -> Vec<u16> {
    kernel_adapter(kernel).required_ports(options)
}

pub async fn verify_nodes_with_kernel_progress(
    kernel: KernelKind,
    nodes: Vec<Node>,
    timeout: Duration,
    progress: Option<&(dyn Fn(ProxyProbeProgress) + Send + Sync)>,
    network_limit: Arc<Semaphore>,
    batch_size: usize,
    conditional_isolation: bool,
) -> Result<Vec<ProxyProbeResult>> {
    kernel_adapter(kernel)
        .verify_nodes(
            nodes,
            timeout,
            progress,
            network_limit,
            batch_size,
            conditional_isolation,
        )
        .await
}

pub(crate) fn mark_failed(runtime: &mut ProxyRuntime, message: String) {
    runtime.recovery_pending = runtime.recovery_target.is_some()
        && retryable_local_start_failure(&message);
    runtime.recovery_waiting_for_route = runtime.recovery_pending;
    runtime.stable_since = None;
    runtime.status.state = ProxyState::Failed;
    runtime.status.local_proxy_ready = false;
    runtime.status.last_error = Some(message.clone());
    runtime.status.message = message;
}

fn retryable_local_start_failure(message: &str) -> bool {
    message.contains("TUN 后台服务暂未就绪")
        || message.contains("端口") && message.contains("已被占用")
        || message.contains("启动超时")
        || message.contains("内核进程在就绪前异常退出")
        || ["network is unreachable", "no route to host", "no available interface",
            "no default route", "failed to get default interface", "no such network interface"]
            .iter().any(|text| message.to_ascii_lowercase().contains(text))
}

fn local_route_available() -> bool {
    // connect() on UDP selects a local route without sending a packet or relying
    // on DNS/the Internet. IPv6-only local networks are supported as well.
    [("0.0.0.0:0", "192.0.2.1:9"), ("[::]:0", "[2001:db8::1]:9")]
        .iter().any(|(bind, remote)| {
            std::net::UdpSocket::bind(bind).is_ok_and(|socket| socket.connect(remote).is_ok())
        })
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    fn selected_node(id: &str) -> Node {
        let now = chrono::Utc::now();
        Node {
            id: id.into(), source_id: None, protocol: "ss".into(), name: "local-test".into(),
            server: "127.0.0.1".into(), port: 8388, raw: String::new(),
            config: serde_json::json!({"server":"127.0.0.1", "port":8388,
                "cipher":"aes-128-gcm", "password":"test-only"}),
            country: None, latency_ms: None, latency_ok: None, download_kbps: None,
            latency_kernel: None, latency_test_kind: None, latency_tested_at: None,
            created_at: now, updated_at: now,
        }
    }

    fn manager() -> ProxyManager {
        ProxyManager::new(MihomoConfigOptions { mode: ProxyMode::None, ..Default::default() })
    }

    #[tokio::test]
    async fn stale_probe_failure_does_not_degrade_a_restarted_session() {
        let manager = manager();
        let mut runtime = manager.runtime.lock().await;
        runtime.request_start(&selected_node("old"), None);
        let old = runtime.generation;
        runtime.request_start(&selected_node("new"), None);
        runtime.status.state = ProxyState::Running;
        runtime.status.local_proxy_ready = true;
        runtime.apply_connectivity_result(old, Some("节点连通性验证失败: old timeout".into()));
        assert_eq!(runtime.status.state, ProxyState::Running);
        assert!(runtime.status.last_error.is_none());
    }

    #[tokio::test]
    async fn recovered_network_clears_only_the_current_connectivity_error() {
        let manager = manager();
        let mut runtime = manager.runtime.lock().await;
        runtime.request_start(&selected_node("active"), None);
        let generation = runtime.generation;
        runtime.status.state = ProxyState::Running;
        runtime.status.local_proxy_ready = true;
        runtime.status.kernel_pid = Some(12345);
        runtime.apply_connectivity_result(generation, Some("节点连通性验证失败: offline".into()));
        assert_eq!(runtime.status.state, ProxyState::Degraded);
        runtime.apply_connectivity_result(generation.wrapping_sub(1), None);
        assert_eq!(runtime.status.state, ProxyState::Degraded, "old success is also stale");
        runtime.apply_connectivity_result(generation, None);
        assert_eq!(runtime.status.state, ProxyState::Running);
        assert_eq!(runtime.status.kernel_pid, Some(12345));
        runtime.status.state = ProxyState::Degraded;
        runtime.status.last_error = Some("恢复系统代理失败".into());
        runtime.apply_connectivity_result(generation, None);
        assert_eq!(runtime.status.state, ProxyState::Degraded);
        assert_eq!(runtime.status.last_error.as_deref(), Some("恢复系统代理失败"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn confirmed_kernel_exit_retains_the_node_and_stop_cancels_recovery() {
        let manager = manager();
        let mut child = Command::new("/bin/sh").args(["-c", "exit 7"])
            .kill_on_drop(true).spawn().unwrap();
        child.wait().await.unwrap();
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.request_start(&selected_node("original"), Some(KernelPreference::Mihomo));
            runtime.status.state = ProxyState::Running;
            runtime.status.local_proxy_ready = true;
            runtime.child = Some(ProxyChild::Standard(child));
        }
        let status = manager.status().await;
        assert_eq!(status.state, ProxyState::Failed);
        assert!(!status.local_proxy_ready);
        {
            let runtime = manager.runtime.lock().await;
            assert!(runtime.recovery_pending);
            assert_eq!(runtime.recovery_target.as_ref().unwrap().0.id, "original");
        }
        manager.maintain().await;
        manager.stop().await.unwrap();
        manager.maintain().await;
        let runtime = manager.runtime.lock().await;
        assert_eq!(runtime.status.state, ProxyState::Stopped);
        assert!(runtime.recovery_target.is_none());
        assert!(!runtime.recovery_pending);
    }

    #[tokio::test]
    async fn failed_start_retries_are_bounded_until_a_local_route_returns() {
        let manager = manager();
        let mut runtime = manager.runtime.lock().await;
        runtime.request_start(&selected_node("offline-login"), None);
        mark_failed(&mut runtime, "TUN 后台服务暂未就绪".into());
        runtime.observe_route(false);
        let mut now = tokio::time::Instant::now();
        for delay in RECOVERY_DELAYS {
            assert!(runtime.recovery_to_attempt(now).is_none(), "backoff must wait");
            now += delay;
            assert_eq!(runtime.recovery_to_attempt(now).unwrap().0.id, "offline-login");
        }
        assert!(runtime.recovery_to_attempt(now + Duration::from_secs(600)).is_none());
        assert!(!runtime.recovery_pending, "no endless retry loop");
        runtime.observe_route(false);
        assert!(!runtime.recovery_pending);
        runtime.observe_route(true);
        assert!(runtime.recovery_pending);
        assert_eq!(runtime.recovery_attempts, 0);
        runtime.recovery_attempts = 2;
        runtime.observe_route(true);
        assert_eq!(runtime.recovery_attempts, 2, "unchanged routes do not reset the budget");
    }

    #[tokio::test]
    async fn invalid_configuration_and_update_authorization_are_not_retried() {
        let manager = manager();
        let mut runtime = manager.runtime.lock().await;
        runtime.request_start(&selected_node("configured"), None);
        for message in ["未找到 Mihomo 可执行文件", "配置无效", "内核或程序已更新，请重新授权安装 TUN 后台服务"] {
            mark_failed(&mut runtime, message.into());
            runtime.observe_route(false);
            runtime.observe_route(true);
            assert!(!runtime.recovery_pending, "{message}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_live_kernel_is_not_restarted_for_network_timeouts() {
        let manager = manager();
        let child = Command::new("/bin/sleep").arg("30").kill_on_drop(true).spawn().unwrap();
        let pid = child.id();
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.request_start(&selected_node("same-node"), None);
            runtime.child = Some(ProxyChild::Standard(child));
            runtime.status.state = ProxyState::Degraded;
            runtime.recovery_pending = true;
            runtime.recovery_waiting_for_route = true;
            runtime.observe_route(false);
            runtime.observe_route(true);
            assert!(runtime.recovery_to_attempt(tokio::time::Instant::now() + Duration::from_secs(60)).is_none());
            assert_eq!(runtime.child.as_ref().unwrap().id(), pid);
            assert_eq!(runtime.recovery_attempts, 0);
        }
        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn login_autostart_cannot_override_an_earlier_manual_stop() {
        let manager = manager();
        manager.stop().await.unwrap();
        manager.start_automatically(&selected_node("saved-login-node")).await.unwrap();
        let runtime = manager.runtime.lock().await;
        assert_eq!(runtime.status.state, ProxyState::Stopped);
        assert!(runtime.recovery_target.is_none());
        assert!(runtime.child.is_none());
    }

    #[tokio::test]
    async fn login_autostart_cannot_override_a_new_manual_target() {
        let manager = manager();
        manager.runtime.lock().await.request_start(&selected_node("new-user-choice"), None);
        manager.start_automatically(&selected_node("saved-login-node")).await.unwrap();
        let runtime = manager.runtime.lock().await;
        assert_eq!(runtime.recovery_target.as_ref().unwrap().0.id, "new-user-choice");
        assert!(runtime.child.is_none());
    }

    #[tokio::test]
    async fn a_stop_queued_after_start_also_cancels_the_new_start_intent() {
        let manager = manager();
        let operation = manager.operation.lock().await;
        let starting = manager.clone();
        let start = tokio::spawn(async move {
            let _operation = starting.operation.lock().await;
            starting.runtime.lock().await.request_start(&selected_node("queued-start"), None);
        });
        tokio::task::yield_now().await;
        let stopping = manager.clone();
        let stop = tokio::spawn(async move { stopping.stop().await });
        tokio::task::yield_now().await;
        drop(operation);
        start.await.unwrap();
        stop.await.unwrap().unwrap();
        let runtime = manager.runtime.lock().await;
        assert_eq!(runtime.status.state, ProxyState::Stopped);
        assert!(runtime.recovery_target.is_none());
        assert!(!runtime.recovery_pending);
    }
}

pub fn start_success_message(
    kernel: &str,
    elapsed_ms: u64,
    mode: ProxyMode,
    owns_system_proxy: bool,
    platform_supports_system_proxy: bool,
    mixed_port: u16,
) -> String {
    let head = format!("{kernel} 运行中（启动耗时 {elapsed_ms} ms）");
    let needs_port_hint =
        mode != ProxyMode::Tun && !owns_system_proxy && !platform_supports_system_proxy;
    if needs_port_hint {
        return format!("{head}· 本平台没有系统代理设置，请把应用指向 127.0.0.1:{mixed_port}");
    }
    head
}

pub async fn activate_system_proxy(
    controller: Arc<dyn SystemProxyControl>,
    server: String,
    terminal_proxy_port: Option<u16>,
) -> Result<SystemProxySnapshot> {
    let snapshot = tokio::task::spawn_blocking(move || {
        let snapshot = sanitize_self_referential(controller.capture()?, &server);
        if let Err(error) = controller.enable(&server) {
            let _ = controller.restore(&snapshot);
            return Err(error);
        }
        Ok(snapshot)
    })
    .await
    .context("系统代理任务执行失败")??;
    if let Some(port) = terminal_proxy_port {
        tokio::task::spawn_blocking(move || {
            match crate::system_proxy::terminal::enable(port) {
                Ok(report) => tracing::info!(summary = report.summary(), "终端代理已启用"),
                Err(error) => tracing::warn!(%error, "终端代理启用失败"),
            }
        });
    }
    Ok(snapshot)
}

pub fn sanitize_self_referential(snapshot: SystemProxySnapshot, server: &str) -> SystemProxySnapshot {
    let points_at_us = snapshot.enabled
        && snapshot
            .server
            .as_deref()
            .is_some_and(|current| current.contains(server.trim()));
    if points_at_us {
        tracing::warn!(
            server,
            "系统代理已指向我方端口（上次未正常还原），还原目标改为关闭系统代理"
        );
        SystemProxySnapshot {
            enabled: false,
            server: None,
            override_list: None,
        }
    } else {
        snapshot
    }
}

pub async fn restore_system_proxy(
    controller: Arc<dyn SystemProxyControl>,
    snapshot: SystemProxySnapshot,
    clear_terminal_proxy: bool,
) -> Result<()> {
    tokio::task::spawn_blocking(move || controller.restore(&snapshot))
        .await
        .context("系统代理恢复任务执行失败")??;
    if clear_terminal_proxy {
        tokio::task::spawn_blocking(|| {
            match crate::system_proxy::terminal::disable() {
                Ok(report) => tracing::info!(summary = report.summary(), "终端代理已清除"),
                Err(error) => tracing::warn!(%error, "终端代理清除失败"),
            }
        });
    }
    Ok(())
}

pub fn last_nonempty_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or("未知错误")
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn format_kernel_diagnostic(raw: &str) -> String {
    let cleaned = strip_ansi(raw);
    let lower = cleaned.to_lowercase();
    if lower.contains("forcibly closed") || lower.contains("connection reset") {
        "远程服务器强制重置/关闭连接 (目标节点已不可用)".into()
    } else if lower.contains("timeout") || lower.contains("deadline exceeded") {
        "连接远程服务器超时 (节点无响应)".into()
    } else if lower.contains("connection refused") {
        "远程服务器拒绝连接 (端口未开放或节点已失效)".into()
    } else if lower.contains("certificate") || lower.contains("tls: handshake") {
        "TLS 证书验证或握手失败".into()
    } else if lower.contains("websocket") {
        "WebSocket 握手失败 (协议或路径配置有误)".into()
    } else if lower.contains("start initial configuration") || lower.contains("in progress") {
        "内核初次启动规则加载中，正在自动建立连接".into()
    } else if lower.contains("outbound connection to") {
        "节点连接超时或无响应 (目标服务器无法连通)".into()
    } else if let Some(msg) = cleaned
        .split("msg=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
    {
        msg.to_string()
    } else {
        last_nonempty_line(&cleaned).to_string()
    }
}

#[cfg(all(test, unix))]
mod config_reload_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn running_manager(response_status: u16) -> (
        ProxyManager, PathBuf, tokio::task::JoinHandle<(String, Value)>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let controller_port = listener.local_addr().unwrap().port();
        let manager = ProxyManager::new(MihomoConfigOptions {
            mode: ProxyMode::None,
            routing_mode: RoutingMode::Direct,
            controller_port,
            ..Default::default()
        });
        let secret = manager.config().await.controller_secret;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0, "controller request ended early");
                received.extend_from_slice(&chunk[..count]);
                if let Some(end) = received.windows(4).position(|s| s == b"\r\n\r\n") {
                    break end;
                }
            };
            let headers = String::from_utf8(received[..header_end].to_vec()).unwrap();
            let length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            }).unwrap();
            while received.len() < header_end + 4 + length {
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                received.extend_from_slice(&chunk[..count]);
            }
            assert!(headers.lines().any(|line|
                line.eq_ignore_ascii_case(&format!("authorization: Bearer {secret}"))),
                "the saved config has no secret; reload must inherit the active one");
            let payload: Value = serde_json::from_slice(&received[header_end + 4..]).unwrap();
            let response = format!("HTTP/1.1 {response_status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            stream.write_all(response.as_bytes()).await.unwrap();
            (headers, payload)
        });
        let root = std::env::temp_dir().join(format!("vproxy-reload-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let config_path = root.join("config.yaml");
        tokio::fs::write(&config_path, "old-effective-config").await.unwrap();
        let now = chrono::Utc::now();
        let node = Node {
            id: "isolated-reload-test".into(), source_id: None,
            protocol: "ss".into(), name: "local-only".into(),
            server: "127.0.0.1".into(), port: 8388, raw: String::new(),
            config: serde_json::json!({"server":"127.0.0.1", "port":8388,
                "cipher":"aes-128-gcm", "password":"test-only"}),
            country: None, latency_ms: None, latency_ok: None, download_kbps: None,
            latency_kernel: None, latency_test_kind: None, latency_tested_at: None,
            created_at: now, updated_at: now,
        };
        let child = Command::new("/bin/sleep").arg("30").kill_on_drop(true).spawn().unwrap();
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.status.state = ProxyState::Running;
            runtime.status.kernel = Some(KernelKind::Mihomo);
            runtime.status.local_proxy_ready = true;
            runtime.child = Some(ProxyChild::Standard(child));
            runtime.current_node = Some(node);
            runtime.work_dir = Some(ProxyWorkDir { root: root.clone(), data_root: root, kernel: KernelKind::Mihomo });
        }
        (manager, config_path, server)
    }

    #[tokio::test]
    async fn rejected_reload_keeps_the_active_options_and_file() {
        let (manager, path, server) = running_manager(400).await;
        let previous = manager.config().await;
        let mut requested = previous.clone();
        requested.controller_secret.clear(); // Persisted/GUI config deliberately omits it.
        requested.block_trackers = !previous.block_trackers;
        let error = manager.reload_config(requested).await.unwrap_err();
        assert!(error.to_string().contains("400"));
        assert_eq!(manager.config().await, previous);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "old-effective-config");
        let (_, payload) = server.await.unwrap();
        assert!(payload["payload"].as_str().unwrap().contains("mixed-port"));
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1,
            "rejected temporary config must be removed");
        manager.stop_inner_for_restart().await.unwrap();
    }

    #[tokio::test]
    async fn accepted_reload_commits_options_and_file_with_the_active_secret() {
        let (manager, path, server) = running_manager(204).await;
        let previous = manager.config().await;
        let mut requested = previous.clone();
        requested.controller_secret.clear();
        requested.block_trackers = !previous.block_trackers;
        assert!(manager.reload_config(requested.clone()).await.unwrap());
        requested.controller_secret = previous.controller_secret;
        assert_eq!(manager.config().await, requested);
        let (_, payload) = server.await.unwrap();
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), payload["payload"].as_str().unwrap());
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        manager.stop_inner_for_restart().await.unwrap();
    }

    #[tokio::test]
    async fn compiler_failure_does_not_commit_options_or_touch_the_controller() {
        let (manager, path, server) = running_manager(204).await;
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.current_node.as_mut().unwrap().protocol = "unsupported-test-only".into();
        }
        let previous = manager.config().await;
        let mut requested = previous.clone();
        requested.block_trackers = !previous.block_trackers;
        let error = manager.reload_config(requested).await.unwrap_err();
        assert!(format!("{error:#}").contains("构建热重载配置失败"));
        assert_eq!(manager.config().await, previous);
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "old-effective-config");
        assert!(!server.is_finished(), "compilation must finish before contacting the controller");
        server.abort();
        manager.stop_inner_for_restart().await.unwrap();
    }

    #[tokio::test]
    async fn deferred_listener_settings_never_replace_the_active_endpoint() {
        let (manager, path, server) = running_manager(204).await;
        let active = manager.config().await;
        let mut pending = active.clone();
        pending.controller_port = if active.controller_port == u16::MAX {
            active.controller_port - 1
        } else {
            active.controller_port + 1
        };
        if pending.controller_port == active.mixed_port {
            pending.controller_port = active.controller_port - 1;
        }
        pending.controller_secret = "next-controller-secret".into();
        pending.mixed_port = active.mixed_port + 1;
        if pending.mixed_port == pending.controller_port {
            pending.mixed_port += 1;
        }
        assert!(!manager.reload_config(pending.clone()).await.unwrap());
        pending.block_trackers = !pending.block_trackers;
        assert!(!manager.reload_config(pending.clone()).await.unwrap());
        {
            let runtime = manager.runtime.lock().await;
            assert_eq!(runtime.options, active, "pending listeners/auth must stay inactive");
            assert_eq!(runtime.pending_options.as_ref(), Some(&pending));
        }
        assert_eq!(manager.config().await, pending, "the UI retains the saved settings");
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "old-effective-config");
        assert!(!server.is_finished(), "a deferred save must not contact any controller");
        server.abort();
        manager.stop_inner_for_restart().await.unwrap();
    }

    #[tokio::test]
    async fn rollback_restores_the_active_config_and_the_previous_pending_request() {
        let (manager, _path, server) = running_manager(204).await;
        let active = manager.config().await;
        let mut pending = active.clone();
        pending.controller_secret = "saved-for-next-connection".into();
        assert!(!manager.reload_config(pending.clone()).await.unwrap());
        let snapshot = manager.config_snapshot().await;
        {
            // Simulate a later successful live change whose DB commit failed.
            let mut runtime = manager.runtime.lock().await;
            let mut newer = active.clone();
            newer.block_trackers = !newer.block_trackers;
            runtime.apply_options(newer);
        }
        assert!(manager.restore_config_snapshot(snapshot).await.unwrap());
        server.await.unwrap();
        {
            let runtime = manager.runtime.lock().await;
            assert_eq!(runtime.options, active, "rollback must restore the active config");
            assert_eq!(runtime.pending_options.as_ref(), Some(&pending));
        }
        assert_eq!(manager.config().await, pending);
        manager.stop_inner_for_restart().await.unwrap();
    }

    #[tokio::test]
    async fn unsupported_reload_is_saved_for_reconnect_without_claiming_to_apply() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.status.state = ProxyState::Running;
            runtime.status.kernel = Some(KernelKind::Xray);
        }
        let previous_status = manager.runtime.lock().await.status.routing_mode;
        let mut requested = manager.config().await;
        requested.routing_mode = RoutingMode::Direct;
        assert!(!manager.reload_config(requested).await.unwrap());
        let runtime = manager.runtime.lock().await;
        assert_eq!(runtime.options.routing_mode, previous_status);
        assert_eq!(runtime.pending_options.as_ref().unwrap().routing_mode, RoutingMode::Direct);
        assert_eq!(runtime.status.routing_mode, previous_status,
            "a deferred save must not report the new routing mode as active");
    }

    #[tokio::test]
    async fn invalid_reload_is_an_error_without_changing_effective_options() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        let previous = manager.config().await;
        let mut invalid = previous.clone();
        invalid.mixed_port = invalid.controller_port;
        assert!(manager.reload_config(invalid).await.is_err());
        assert_eq!(manager.config().await, previous);
    }

    #[tokio::test]
    async fn timed_out_reload_restores_previous_payload_before_reporting_failure() {
        async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, Value) {
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                received.extend_from_slice(&chunk[..count]);
                if let Some(end) = received.windows(4).position(|s| s == b"\r\n\r\n") {
                    break end;
                }
            };
            let headers = String::from_utf8(received[..header_end].to_vec()).unwrap();
            let length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            }).unwrap();
            while received.len() < header_end + 4 + length {
                let count = stream.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0);
                received.extend_from_slice(&chunk[..count]);
            }
            let payload = serde_json::from_slice(&received[header_end + 4..header_end + 4 + length]).unwrap();
            (headers, payload)
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let controller_port = listener.local_addr().unwrap().port();
        let root = std::env::temp_dir().join(format!("vproxy-reload-timeout-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let config_path = root.join("config.yaml");
        tokio::fs::write(&config_path, "old-effective-config").await.unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let (first_headers, first_payload) = read_request(&mut first).await;
            assert!(first_headers.to_ascii_lowercase().contains("authorization: bearer isolated-timeout-secret"));
            assert_eq!(first_payload["payload"], "new-isolated-config");
            // The core has applied the request, but its acknowledgement is lost.
            // Keep the first connection open and accept the corrective reload.
            let (mut rollback, _) = listener.accept().await.unwrap();
            let (rollback_headers, rollback_payload) = read_request(&mut rollback).await;
            assert!(rollback_headers.to_ascii_lowercase().contains("authorization: bearer isolated-timeout-secret"));
            assert_eq!(rollback_payload["payload"], "old-effective-config");
            rollback.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
            drop(first);
        });
        let error = reload_controller_config(
            controller_port,
            "isolated-timeout-secret",
            &config_path,
            "new-isolated-config",
        ).await.unwrap_err();
        assert!(format!("{error:#}").contains("已恢复"));
        tokio::time::timeout(Duration::from_secs(8), server).await.unwrap().unwrap();
        assert_eq!(tokio::fs::read_to_string(&config_path).await.unwrap(), "old-effective-config");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1,
            "ambiguous reload must remove its staged config after recovery");
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

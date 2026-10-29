use crate::kernel::{
    find_kernel_executable, managed_kernel_root, select_proxy_kernel_preference_for_node,
    KernelKind, KernelPreference,
};
use crate::models::Node;
use crate::proxy::adapter::kernel_adapter;
use crate::proxy::types::{MihomoConfigOptions, ProxyMode, ProxyRuntimeStatus, ProxyState};
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
    pub(crate) child: Option<ProxyChild>,
    pub(crate) work_dir: Option<ProxyWorkDir>,
    pub(crate) system_proxy_snapshot: Option<SystemProxySnapshot>,
    pub(crate) current_node: Option<Node>,
}

impl ProxyRuntime {
    /// 界面传进来的配置里，一次性控制器密钥和 url-test 候选都是空的（两者都不持久
    /// 化）。直接覆盖会让鉴权失效、自动选路丢候选，所以空值一律沿用运行时当前的。
    pub(crate) fn apply_options(&mut self, mut options: MihomoConfigOptions) {
        if options.controller_secret.is_empty() {
            options.controller_secret = self.options.controller_secret.clone();
        }
        if options.url_test_nodes.is_empty() {
            options.url_test_nodes = self.options.url_test_nodes.clone();
        }
        if options.port_mapping_nodes.is_empty() {
            options.port_mapping_nodes = self.options.port_mapping_nodes.clone();
        }
        self.options = options;
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

#[derive(Clone)]
pub struct ProxyManager {
    pub(crate) runtime: Arc<Mutex<ProxyRuntime>>,
    operation: Arc<Mutex<()>>,
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
                child: None,
                work_dir: None,
                system_proxy_snapshot: None,
                current_node: None,
            })),
            operation: Arc::new(Mutex::new(())),
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

    pub async fn status(&self) -> ProxyRuntimeStatus {
        let cleanup = {
            let mut runtime = self.runtime.lock().await;
            let exit_status = runtime
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten());
            if let Some(exit_status) = exit_status {
                let kernel_name = runtime
                    .status
                    .kernel
                    .map(KernelKind::display_name)
                    .unwrap_or("代理内核");
                runtime.child = None;
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
        self.runtime.lock().await.options.clone()
    }

    /// 自动切换用的轻量健康探测：经**正在运行的** mixed 端口取一次 generate_204，
    /// 返回 (是否连通, 端到端耗时 ms)。不新起进程、不重启内核，所以可以高频调用。
    pub async fn probe_health(&self, timeout: Duration) -> (bool, u64) {
        let port = self.runtime.lock().await.options.mixed_port;
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
        (ok, started.elapsed().as_millis() as u64)
    }

    /// 启动前注入 url-test 候选节点（不持久化，只影响本次生成的配置）。返回真正
    /// 能导出成 clash proxy 的个数：内核侧只有 >1 才会生成「自动选择」组。
    pub async fn set_url_test_candidates(&self, nodes: Vec<Node>) -> usize {
        let proxies = crate::export::nodes_to_clash_proxies(&nodes);
        let count = proxies.len();
        let mut runtime = self.runtime.lock().await;
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
        runtime.options.port_mapping_nodes = mapping_nodes;
    }

    pub async fn update_config(&self, options: MihomoConfigOptions) -> Result<()> {
        options.validate()?;
        let _operation = self.operation.lock().await;
        let mut runtime = self.runtime.lock().await;
        if !runtime.status.config_editable() {
            return Err(anyhow!("请先停止代理再修改运行配置"));
        }
        runtime.apply_options(options.clone());
        runtime.status.mode = options.mode;
        runtime.status.mixed_port = options.mixed_port;
        runtime.status.controller_port = options.controller_port;
        runtime.status.controller_secret = options.controller_secret.clone();
        Ok(())
    }

    pub async fn reload_config(&self, options: MihomoConfigOptions) -> Result<bool> {
        options.validate()?;
        let _operation = self.operation.lock().await;

        let (config_path, controller_port, node) = {
            let mut runtime = self.runtime.lock().await;
            if !matches!(
                runtime.status.state,
                ProxyState::Running | ProxyState::Degraded
            ) {
                runtime.apply_options(options.clone());
                runtime.status.mode = options.mode;
                runtime.status.mixed_port = options.mixed_port;
                runtime.status.controller_port = options.controller_port;
                runtime.status.controller_secret = options.controller_secret.clone();
                return Ok(true);
            }
            if runtime.status.kernel != Some(KernelKind::Mihomo) {
                runtime.apply_options(options.clone());
                return Ok(false);
            }
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().ok().flatten().is_none(),
                None => false,
            };
            if !alive {
                runtime.apply_options(options.clone());
                return Ok(false);
            }
            let Some(work_dir) = &runtime.work_dir else {
                runtime.apply_options(options.clone());
                return Ok(false);
            };
            let Some(node) = runtime.current_node.clone() else {
                runtime.apply_options(options.clone());
                return Ok(false);
            };
            (
                work_dir.config_path(),
                runtime.options.controller_port,
                node,
            )
        };

        let started = tokio::time::Instant::now();
        let config = match build_kernel_config(KernelKind::Mihomo, &node, options.clone()) {
            Ok(config) => config,
            Err(error) => {
                tracing::warn!("构建热重载分流规则配置失败: {error}");
                return Ok(false);
            }
        };
        let config_text = match serialize_kernel_config(KernelKind::Mihomo, &config) {
            Ok(text) => text,
            Err(error) => {
                tracing::warn!("序列化热重载分流规则配置失败: {error}");
                return Ok(false);
            }
        };

        if let Err(error) = tokio::fs::write(&config_path, &config_text).await {
            tracing::warn!("写入热重载分流规则配置文件失败: {error}");
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

        let controller_secret = options.controller_secret.clone();
        let url = format!("http://127.0.0.1:{controller_port}/configs?force=true");
        let payload = serde_json::json!({
            "path": config_path.to_string_lossy(),
            "payload": &config_text,
        });
        let request = with_controller_auth(client.put(&url).json(&payload), &controller_secret);
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let mut runtime = self.runtime.lock().await;
                runtime.apply_options(options);
                runtime.status.message =
                    format!("Mihomo 运行中（分流规则已热重载生效，耗时 {elapsed_ms} ms）");
                tracing::info!(elapsed_ms, "Mihomo 分流规则热重载生效成功");
                Ok(true)
            }
            Ok(response) => {
                tracing::warn!(status = %response.status(), "Mihomo 热重载 API 返回失败");
                Ok(false)
            }
            Err(error) => {
                tracing::warn!("请求 Mihomo 热重载 API 失败: {error}");
                Ok(false)
            }
        }
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

        let (kernel, config_path, controller_port, controller_secret, mut options, node) = {
            let mut runtime = self.runtime.lock().await;
            if !matches!(
                runtime.status.state,
                ProxyState::Running | ProxyState::Degraded
            ) {
                return Ok(false);
            }
            let Some(kernel) = runtime.status.kernel else {
                return Ok(false);
            };
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().ok().flatten().is_none(),
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
        runtime.apply_options(options.clone());
        runtime.status.mode = options.mode;
        runtime.status.last_error = None;
        runtime.status.message = if tun_on {
            format!("{} TUN 已热开启（{elapsed_ms} ms，未重启进程）", kernel.display_name())
        } else {
            format!("{} TUN 已热关闭（{elapsed_ms} ms，未重启进程）", kernel.display_name())
        };
        tracing::info!(kernel = %kernel.display_name(), tun_on, elapsed_ms, "TUN 热切换生效");
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
            runtime.apply_options(options.clone());
            runtime.status.mode = options.mode;
            runtime.status.mixed_port = options.mixed_port;
            runtime.status.controller_port = options.controller_port;
            runtime.status.controller_secret = options.controller_secret.clone();
        }
        let running = matches!(
            self.runtime.lock().await.status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        if running {
            self.restart_inner(node, Some(options.kernel_preference)).await
        } else {
            self.start_inner_with_preference(node, None, Some(options.kernel_preference)).await
        }
    }

    pub fn connect<'a>(
        &'a self,
        node: &'a Node,
        kernel_override: Option<KernelPreference>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let _operation = self.operation.lock().await;
            let running = matches!(
                self.runtime.lock().await.status.state,
                ProxyState::Running | ProxyState::Degraded
            );
            if running {
                self.restart_inner(node, kernel_override).await
            } else {
                self.start_inner_with_preference(node, None, kernel_override)
                    .await
            }
        })
    }

    async fn start_inner_with_preference(
        &self,
        node: &Node,
        preserved_system_proxy_snapshot: Option<SystemProxySnapshot>,
        kernel_override: Option<KernelPreference>,
    ) -> Result<()> {
        let options = self.runtime.lock().await.options.clone();
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
        self.start_with_path_inner(node, kernel, &path, preserved_system_proxy_snapshot)
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
        self.start_with_path_inner(node, kernel, executable_path, None)
            .await
    }

    async fn start_with_path_inner(
        &self,
        node: &Node,
        kernel: KernelKind,
        executable_path: &Path,
        mut preserved_system_proxy_snapshot: Option<SystemProxySnapshot>,
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
        let startup_deadline = startup_started + STARTUP_PROXY_TOTAL_TIMEOUT;

        runtime.status.state = ProxyState::Starting;
        runtime.status.kernel = Some(kernel);
        runtime.status.node_id = Some(node.id.clone());
        runtime.current_node = Some(node.clone());
        runtime.status.message = format!("正在启动 {}", kernel.display_name());
        runtime.status.local_proxy_ready = false;
        runtime.status.last_error = None;

        for port in required_startup_ports(kernel, &runtime.options) {
            if let Err(error) = ensure_port_available(port).await {
                mark_failed(&mut runtime, error.to_string());
                return Err(error);
            }
        }

        let config = match build_kernel_config(kernel, node, runtime.options.clone()) {
            Ok(config) => config,
            Err(error) => {
                runtime.status.state = ProxyState::Failed;
                runtime.status.message = error.to_string();
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
        let requires_elevation = cfg!(windows)
            && runtime.options.mode.has_tun()
            && !crate::platform::is_elevated();

        let mut child = if requires_elevation {
            #[cfg(windows)]
            {
                tracing::info!(
                    kernel = kernel.display_name(),
                    "TUN 模式需管理员权限，通过无头提权 Supervisor 静默拉起内核..."
                );
                match crate::platform::start_elevated_kernel_runner(
                    kernel,
                    executable_path,
                    work_dir.data_root(),
                    &work_dir.config_path(),
                    &work_dir.log_path(),
                ) {
                    Ok(elevated) => ProxyChild::Elevated(elevated),
                    Err(error) => {
                        let _ = work_dir.cleanup().await;
                        let message = format!("无头提权启动 {} 失败: {error}", kernel.display_name());
                        runtime.status.state = ProxyState::Failed;
                        runtime.status.message = message.clone();
                        return Err(error);
                    }
                }
            }
            #[cfg(not(windows))]
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
                    runtime.status.state = ProxyState::Failed;
                    runtime.status.message = message.clone();
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
        runtime.status.local_proxy_ready = true;

        for port in required_startup_ports(kernel, &runtime.options) {
            let startup_timeout =
                startup_deadline.saturating_duration_since(tokio::time::Instant::now());
            if !wait_for_port(&mut child, port, startup_timeout).await {
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

                let message = if detail.contains("failed to decode geodata file")
                    || detail.contains("Geosite.dat invalid")
                    || detail.contains("GeoIP.dat invalid")
                    || (detail.contains("cannot parse") && detail.contains("geo"))
                {
                    crate::geo::purge_corrupt_geo_files(&crate::geo::mihomo_dir());
                    "Mihomo 分流规则文件 (GeoSite/GeoIP) 损坏，已自动清理损坏文件。请在「规则设置」点击「更新规则文件」重新同步完整库后再启动代理。".to_string()
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
                runtime.status.state = ProxyState::Failed;
                runtime.status.message = message.clone();
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
            let lower_log = log_detail.to_lowercase();
            if lower_log.contains("tunsetiff")
                || lower_log.contains("start tun error")
                || lower_log.contains("create tun interface error")
                || (lower_log.contains("tun") && lower_log.contains("operation not permitted"))
                || (lower_log.contains("tun") && lower_log.contains("permission denied"))
                || lower_log.contains("failed to start tun")
                || lower_log.contains("create tun adapter failed")
            {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = work_dir.cleanup().await;
                let cmd = crate::platform::tun_capability_command(executable_path);
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
        // 这个字段的意思是"本地 mixed 端口已经就绪"（刚 spawn 就置真的那几处是同一个意思），
        // 不是"系统代理归我们"。归属由 `system_proxy_enabled` 表达，别在这儿混用。
        runtime.status.local_proxy_ready = true;
        runtime.status.system_proxy_enabled = runtime.system_proxy_snapshot.is_some();
        runtime.status.last_error = None;
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
                if let Err(error) = verify_local_proxy(mixed_port, STARTUP_PROXY_VERIFY_TIMEOUT.max(Duration::from_secs(3))).await {
                    let detail = tokio::fs::read_to_string(&log_path)
                        .await
                        .unwrap_or_default();
                    let diagnostic = format_kernel_diagnostic(&detail);
                    if diagnostic.contains("规则加载中")
                        || diagnostic.contains("自动建立连接")
                        || diagnostic.contains("start initial configuration")
                    {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        if verify_local_proxy(mixed_port, Duration::from_secs(2)).await.is_ok() {
                            tracing::info!(kernel = %kernel_name, "内核初始握手完成，连通性验证通过");
                            return;
                        }
                    }

                    // 如果当前是 Meow 发生连通性超时，触发自动熔断保护，平滑切换到工业级 Mihomo 接管
                    if kernel == KernelKind::Meow && find_kernel_executable(KernelKind::Mihomo).is_some() {
                        tracing::warn!(
                            kernel = %kernel_name,
                            %error,
                            "Meow 内核网络连接全部超时，触发熔断保护，自动平滑切换至 Mihomo"
                        );
                        if let Ok(()) = manager_clone.connect(&current_node, Some(KernelPreference::Mihomo)).await {
                            let mut rt = runtime_clone.lock().await;
                            if rt.status.state == ProxyState::Running {
                                rt.status.message = "Meow 连通超时，已自动触发熔断切换至 Mihomo 接管网络".to_string();
                            }
                            return;
                        }
                    }

                    let mut rt = runtime_clone.lock().await;
                    if rt.status.state == ProxyState::Running && rt.options.mixed_port == mixed_port {
                        let message = if detail.trim().is_empty() {
                            format!("节点连通性验证失败: {error}")
                        } else {
                            format!("节点连通性验证失败: {error}; {kernel_name}: {diagnostic}")
                        };
                        tracing::warn!(kernel = %kernel_name, %message, "后台连通性验证未通过，置为降级运行");
                        rt.status.state = ProxyState::Degraded;
                        rt.status.last_error = Some(message.clone());
                        rt.status.message = format!("{kernel_name} 降级运行: {message}");
                    }
                } else {
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
        let _operation = self.operation.lock().await;
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
        if let Some(child) = child.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    if let Err(error) = child.kill().await {
                        errors.push(format!("停止 {kernel_name} 失败: {error}"));
                    }
                }
                Ok(Some(_)) => {}
                Err(error) => errors.push(format!("读取 {kernel_name} 状态失败: {error}")),
            }
            if let Err(error) = child.wait().await {
                errors.push(format!("等待 {kernel_name} 退出失败: {error}"));
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
        if self.try_hot_reload_mihomo(node).await? {
            return Ok(());
        }
        self.restart_inner(node, None).await
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
            let alive = match runtime.child.as_mut() {
                Some(child) => child.try_wait().ok().flatten().is_none(),
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
        let payload = serde_json::json!({
            "path": config_path.to_string_lossy(),
            "payload": &config_text,
        });
        let request = with_controller_auth(client.put(&url).json(&payload), &controller_secret);
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let mut runtime = self.runtime.lock().await;
                runtime.status.node_id = Some(node.id.clone());
                runtime.current_node = Some(node.clone());
                runtime.status.state = ProxyState::Running;
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

    async fn restart_inner(
        &self,
        node: &Node,
        kernel_override: Option<KernelPreference>,
    ) -> Result<()> {
        let restart_started = tokio::time::Instant::now();
        let snapshot = self.stop_inner_for_restart().await?;
        tracing::info!(
            stage = "old_kernel_stopped",
            elapsed_ms = restart_started.elapsed().as_millis() as u64,
            "代理重启阶段完成"
        );
        match self
            .start_inner_with_preference(node, snapshot.clone(), kernel_override)
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
        runtime.status.state = ProxyState::Failed;
        runtime.status.message = message;
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

pub async fn wait_for_port(child: &mut ProxyChild, port: u16, timeout: Duration) -> bool {
    let started = tokio::time::Instant::now();
    while started.elapsed() < timeout {
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        let connect_timeout = remaining.min(STARTUP_PORT_CONNECT_TIMEOUT);
        if tokio::time::timeout(connect_timeout, TcpStream::connect(("127.0.0.1", port)))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
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

pub async fn ensure_port_available(port: u16) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..4 {
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
                    tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
                } else if attempt == 1 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                } else if attempt == 2 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(75)).await;
                }
            }
        }
    }
    let error = last_err
        .map(|e| e.to_string())
        .unwrap_or_else(|| "未知错误".into());
    Err(anyhow!("代理端口 {port} 已被占用: {error}"))
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
    runtime.status.state = ProxyState::Failed;
    runtime.status.local_proxy_ready = false;
    runtime.status.last_error = Some(message.clone());
    runtime.status.message = message;
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

use crate::export::nodes_to_clash_proxies;
use crate::kernel::{
    find_kernel_executable, managed_kernel_root, select_proxy_kernel_preference, KernelKind,
    KernelPreference,
};
use crate::models::Node;
use crate::proxy_probe::{ProxyProbeProgress, ProxyProbeResult, PROXY_CONNECTIVITY_TEST_URL};
use crate::system_proxy::{platform_system_proxy, SystemProxyControl, SystemProxySnapshot};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use uuid::Uuid;

mod config;
mod sing_box;
mod xray;
pub use config::*;
pub use sing_box::build_sing_box_config;
pub use xray::build_xray_config;

const STARTUP_PROXY_TOTAL_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_PROXY_VERIFY_TIMEOUT: Duration = Duration::from_millis(1_500);
const STARTUP_SYSTEM_PROXY_RESERVE: Duration = Duration::from_millis(250);
const STARTUP_PORT_CONNECT_TIMEOUT: Duration = Duration::from_millis(60);
const VERIFY_CONNECTIVITY_ON_START: bool = true;
const STARTUP_PROXY_VERIFY_URLS: [&str; 1] = [PROXY_CONNECTIVITY_TEST_URL];

pub fn is_node_connectivity_failure(message: &str) -> bool {
    message.contains("节点连通性验证失败")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    System,
    Tun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyState {
    Stopped,
    Starting,
    Running,
    Degraded,
    Stopping,
    Failed,
}

// 不派生 Eq：url-test 候选里带着 Node（含浮点字段），只能做近似比较。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MihomoConfigOptions {
    pub mode: ProxyMode,
    pub mixed_port: u16,
    pub controller_port: u16,
    pub allow_lan: bool,
    pub tun: TunConfig,
    pub dns: DnsConfig,
    #[serde(default)]
    pub routing_mode: RoutingMode,
    pub rules: Vec<TrafficRule>,
    #[serde(default)]
    pub kernel_preference: KernelPreference,
    /// mihomo `external-controller` 的一次性密钥，不落盘、不跨进程共享。
    #[serde(default, skip)]
    pub controller_secret: String,
    /// 内核内自动选路：把测速前 N 名一起写进配置，交给 mihomo 的 url-test 组按延迟
    /// 自己切换。切换就不再需要重写配置 + 热重载（那会掐断所有已建立的连接）。
    /// 仅 Mihomo 内核支持；sing-box / xray 仍按单节点生成。
    #[serde(default)]
    pub url_test: bool,
    /// 写进 url-test 组的节点上限（含当前节点）。
    #[serde(default = "default_url_test_size")]
    pub url_test_size: u16,
    /// 终端代理：启动时把 HTTP_PROXY / HTTPS_PROXY / ALL_PROXY / NO_PROXY 写进
    /// HKCU\Environment（Git、curl、npm、pip 只认这些，不认 WinINet 系统代理）。
    #[serde(default)]
    pub terminal_proxy: bool,
    /// 候选节点（已转成 clash proxy 对象）：启动前由界面按最新测速结果填入，
    /// 不持久化。
    #[serde(default, skip)]
    pub url_test_nodes: Vec<Value>,
}

fn default_url_test_size() -> u16 {
    20
}

impl Default for MihomoConfigOptions {
    fn default() -> Self {
        Self {
            mode: ProxyMode::System,
            mixed_port: 9980,
            controller_port: 9090,
            allow_lan: false,
            tun: TunConfig::default(),
            dns: DnsConfig::default(),
            routing_mode: RoutingMode::Rule,
            rules: Vec::new(),
            kernel_preference: KernelPreference::Auto,
            controller_secret: String::new(),
            url_test: false,
            url_test_size: default_url_test_size(),
            terminal_proxy: false,
            url_test_nodes: Vec::new(),
        }
    }
}

impl MihomoConfigOptions {
    pub fn validate(&self) -> Result<()> {
        if self.mixed_port == self.controller_port {
            return Err(anyhow!("代理端口和控制端口不能相同"));
        }
        self.dns.validate()?;
        for rule in &self.rules {
            rule.to_mihomo_rule()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRuntimeStatus {
    pub state: ProxyState,
    pub kernel: Option<KernelKind>,
    pub mode: ProxyMode,
    pub node_id: Option<String>,
    pub mixed_port: u16,
    pub controller_port: u16,
    /// 与 `MihomoConfigOptions::controller_secret` 同步，供 GUI 调用控制器时使用。
    #[serde(default, skip)]
    pub controller_secret: String,
    pub message: String,
    pub local_proxy_ready: bool,
    pub system_proxy_enabled: bool,
    pub last_error: Option<String>,
}

impl ProxyRuntimeStatus {
    pub fn stopped(options: MihomoConfigOptions) -> Self {
        Self {
            state: ProxyState::Stopped,
            kernel: None,
            mode: options.mode,
            node_id: None,
            mixed_port: options.mixed_port,
            controller_port: options.controller_port,
            controller_secret: options.controller_secret.clone(),
            message: "代理未启动".into(),
            local_proxy_ready: false,
            system_proxy_enabled: false,
            last_error: None,
        }
    }

    pub fn local_proxy_active(&self) -> bool {
        self.local_proxy_ready && matches!(self.state, ProxyState::Running | ProxyState::Degraded)
    }

    pub fn config_editable(&self) -> bool {
        matches!(self.state, ProxyState::Stopped | ProxyState::Failed)
            || (self.state == ProxyState::Degraded && !self.local_proxy_ready)
    }

    pub fn kernel_in_use(&self, kind: KernelKind) -> bool {
        self.kernel == Some(kind)
            && (matches!(
                self.state,
                ProxyState::Starting | ProxyState::Running | ProxyState::Stopping
            ) || (self.state == ProxyState::Degraded && self.local_proxy_ready))
    }
}

/// 内核内自动选路用的两个组名。
pub const URL_TEST_GROUP: &str = "自动选择";

/// url-test 的判活节奏。原来是 `interval: 300`：一台节点从坏掉到被避开最长要等 5 分钟 ——
/// 2026-10-05 实测那台 SG 节点连续 17 分钟 502，内核每 300 秒才重测它一次。
/// 调到 60/3000/30 的代价：每 60 秒对每个候选各发一次 HEAD 请求（几十 KB 级，CPU 可忽略）；
/// 收益：坏节点 1 分钟内被重测，tolerance 30 让"换到明显更快的那台"更少犹豫。
const URL_TEST_INTERVAL_SECS: u64 = 60;
const URL_TEST_TOLERANCE_MS: u64 = 30;
const URL_TEST_TIMEOUT_MS: u64 = 3000;
pub const FINAL_GROUP: &str = "漏网之鱼";

pub fn build_mihomo_config(node: &Node, options: MihomoConfigOptions) -> Result<Value> {
    options.validate()?;
    let Some(proxy) = nodes_to_clash_proxies(std::slice::from_ref(node)).pop() else {
        return Err(anyhow!("Mihomo 不支持协议: {}", node.protocol));
    };
    let proxy_name = proxy
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Mihomo 代理配置缺少名称"))?;

    let selected_rules = match options.routing_mode {
        RoutingMode::Rule => optimize_traffic_rules(&options.rules),
        RoutingMode::ChinaDirect => recommended_traffic_rules(),
        RoutingMode::GfwList => gfwlist_traffic_rules(),
        RoutingMode::Global | RoutingMode::Direct => Vec::new(),
    };
    // 境内 DoH 污染出的国内 IP 会让 GEOIP,CN 误判直连，先按域名定性（仅 mihomo）。
    let mut selected_rules = selected_rules;
    apply_mihomo_dns_poison_overrides(&mut selected_rules);
    let mut rules = selected_rules
        .iter()
        .map(TrafficRule::to_mihomo_rule)
        .collect::<Result<Vec<_>>>()?;
    // 候选节点：当前节点排第一，其余来自界面按最新测速填进来的 url_test_nodes。
    // clash 要求代理名唯一，而不同订阅里同名节点很常见，所以撞名就加 #2/#3 后缀。
    let mut candidate_proxies = vec![proxy.clone()];
    let mut used_names: std::collections::HashSet<String> =
        std::collections::HashSet::from([proxy_name.to_owned()]);
    if options.url_test {
        let limit = options.url_test_size.max(2) as usize;
        for candidate in &options.url_test_nodes {
            if candidate_proxies.len() >= limit {
                break;
            }
            let Some(mut object) = candidate.as_object().cloned() else {
                continue;
            };
            let raw = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if raw.is_empty() {
                continue;
            }
            let mut name = raw.clone();
            let mut index = 2;
            while !used_names.insert(name.clone()) {
                name = format!("{raw} #{index}");
                index += 1;
            }
            object.insert("name".into(), Value::String(name));
            candidate_proxies.push(Value::Object(object));
        }
    }
    let candidate_names: Vec<String> = candidate_proxies
        .iter()
        .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let url_test_active = candidate_names.len() > 1;
    if options.url_test && !url_test_active {
        tracing::warn!("url-test 已开启但候选不足（只有当前节点），本次配置退回单节点 PROXY 组");
    }

    let default_target = match options.routing_mode {
        RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::Global => "PROXY",
        RoutingMode::GfwList | RoutingMode::Direct => "DIRECT",
    };
    let match_target = if url_test_active && default_target == "PROXY" {
        FINAL_GROUP
    } else {
        default_target
    };
    // 内核下载 geo 库用的是它自己的规则引擎；不钉直连就会被下面的 MATCH 丢进代理节点，
    // 实测 10 秒 TLS 握手超时（同一 URL 本机直连 1.4 秒）。
    for host in crate::geo::MIRROR_DIRECT_HOSTS {
        rules.push(format!("DOMAIN-SUFFIX,{host},DIRECT"));
    }
    rules.push(format!("MATCH,{match_target}"));

    let proxy_groups = if url_test_active {
        // PROXY 的第一个成员是「自动选择」：默认就让内核按延迟自己挑，界面上选中
        // 某个具体节点即等价于 PUT /proxies/PROXY，不需要重写配置。
        let mut proxy_members = vec![URL_TEST_GROUP.to_owned()];
        proxy_members.extend(candidate_names.iter().cloned());
        json!([
            { "name": "PROXY", "type": "select", "proxies": proxy_members },
            {
                "name": URL_TEST_GROUP,
                "type": "url-test",
                "url": crate::proxy_probe::PROXY_CONNECTIVITY_TEST_URL,
                "interval": URL_TEST_INTERVAL_SECS,
                "tolerance": URL_TEST_TOLERANCE_MS,
                "timeout": URL_TEST_TIMEOUT_MS,
                "max-failed": 3,
                "lazy": true,
                "proxies": candidate_names,
            },
            {
                "name": FINAL_GROUP,
                "type": "select",
                "proxies": ["PROXY", URL_TEST_GROUP, "DIRECT", "REJECT"],
            }
        ])
    } else {
        json!([{ "name": "PROXY", "type": "select", "proxies": candidate_names }])
    };

    let mut config = json!({
        "mixed-port": options.mixed_port,
        "external-controller": format!("127.0.0.1:{}", options.controller_port),
        "secret": options.controller_secret,
        "allow-lan": options.allow_lan,
        "mode": "rule",
        "log-level": "info",
        "proxies": candidate_proxies,
        "proxy-groups": proxy_groups,
        "dns": {
            "enable": options.dns.enabled,
            "ipv6": options.dns.ipv6,
            "enhanced-mode": options.dns.enhanced_mode.as_mihomo_value(),
            "fake-ip-range": "198.18.0.1/16",
            "nameserver": options.dns.nameservers
        },
        "rules": rules
    });
    // 节点自身域名用独立的 DNS 列表解析，避免经 ISP 明文查询暴露代理用途。
    if !options.dns.proxy_server_nameserver.is_empty() {
        config["dns"]["proxy-server-nameserver"] = json!(options.dns.proxy_server_nameserver);
    }

    let mut rule_sets = std::collections::HashSet::new();
    for r in &selected_rules {
        if let crate::proxy::RuleMatcher::RuleSet(name) = &r.matcher {
            rule_sets.insert(name.clone());
        }
    }
    if !rule_sets.is_empty() {
        let mut providers = serde_json::Map::new();
        for name in rule_sets {
            let (url, behavior) = match name.as_str() {
                "reject" | "reject-list" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/reject-list.txt",
                    "domain",
                ),
                "direct" | "direct-list" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/direct-list.txt",
                    "domain",
                ),
                "proxy" | "proxy-list" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/proxy-list.txt",
                    "domain",
                ),
                "gfw" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/gfw.txt",
                    "domain",
                ),
                _ => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/proxy-list.txt",
                    "domain",
                ),
            };
            providers.insert(
                name.clone(),
                json!({
                    "type": "http",
                    "behavior": behavior,
                    "url": url,
                    "path": format!("./ruleset/{name}.yaml"),
                    "interval": 86400
                }),
            );
        }
        config["rule-providers"] = serde_json::Value::Object(providers);
    }

    if selected_rules.iter().any(TrafficRule::uses_geo) {
        config["geodata-mode"] = json!(true);
        // 关死内核侧自动更新：它只会直连 GitHub（实测 0.116 MB/s），在启动窗口内下不完就
        // 留下半截库 → `list cn not found` → 内核起不来。库改由 `crate::geo` 负责：
        // 镜像优先 + 校验大小 + 先写 .part 再原子替换。
        config["geo-auto-update"] = json!(false);
        config["geo-update-interval"] = json!(24);
        config["geox-url"] = json!({
            "geoip": "https://gh-proxy.com/https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geoip.dat",
            "geosite": "https://gh-proxy.com/https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/geosite.dat"
        });
    }
    if options.mode == ProxyMode::Tun {
        config["tun"] = json!({
            "enable": true,
            "stack": options.tun.stack.as_mihomo_value(),
            "dns-hijack": options.tun.dns_hijack,
            "auto-route": options.tun.auto_route,
            "auto-detect-interface": options.tun.auto_detect_interface
        });
    }

    Ok(config)
}

#[derive(Debug)]
pub struct ProxyWorkDir {
    root: PathBuf,
    data_root: PathBuf,
    kernel: KernelKind,
}

impl ProxyWorkDir {
    pub async fn create(kernel: KernelKind) -> Result<Self> {
        let root = std::env::temp_dir().join(format!("vpn-parser-proxy-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await?;
        let data_root = match kernel {
            KernelKind::Mihomo => managed_kernel_root().join(kernel.directory_name()),
            KernelKind::SingBox | KernelKind::Xray => root.clone(),
        };
        tokio::fs::create_dir_all(&data_root).await?;
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
            KernelKind::Mihomo => "config.yaml",
            KernelKind::SingBox | KernelKind::Xray => "config.json",
        })
    }

    pub fn log_path(&self) -> PathBuf {
        self.root.join(match self.kernel {
            KernelKind::Mihomo => "mihomo.log",
            KernelKind::SingBox => "sing-box.log",
            KernelKind::Xray => "xray.log",
        })
    }

    pub async fn cleanup(self) -> Result<()> {
        if self.root.exists() {
            tokio::fs::remove_dir_all(self.root).await?;
        }
        Ok(())
    }
}

impl ProxyRuntime {
    /// 界面传进来的配置里，一次性控制器密钥和 url-test 候选都是空的（两者都不持久
    /// 化）。直接覆盖会让鉴权失效、自动选路丢候选，所以空值一律沿用运行时当前的。
    fn apply_options(&mut self, mut options: MihomoConfigOptions) {
        if options.controller_secret.is_empty() {
            options.controller_secret = self.options.controller_secret.clone();
        }
        if options.url_test_nodes.is_empty() {
            options.url_test_nodes = self.options.url_test_nodes.clone();
        }
        self.options = options;
    }
}

struct ProxyRuntime {
    status: ProxyRuntimeStatus,
    options: MihomoConfigOptions,
    child: Option<Child>,
    work_dir: Option<ProxyWorkDir>,
    system_proxy_snapshot: Option<SystemProxySnapshot>,
    current_node: Option<Node>,
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
    runtime: Arc<Mutex<ProxyRuntime>>,
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
        if runtime.options.mode != ProxyMode::System {
            return Err(anyhow::anyhow!("当前不是系统代理模式，不写注册表"));
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
        let built = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))
            .and_then(|proxy| {
                reqwest::Client::builder()
                    .proxy(proxy)
                    .connect_timeout(timeout.min(Duration::from_millis(1_500)))
                    .timeout(timeout)
                    .build()
            })
            .map_err(|error| error.to_string());
        let ok = match built {
            Ok(client) => client
                .get(crate::proxy_probe::PROXY_CONNECTIVITY_TEST_URL)
                .send()
                .await
                .map(|response| response.status().is_success())
                .unwrap_or(false),
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

    pub async fn start(&self, node: &Node) -> Result<()> {
        self.connect(node, None).await
    }

    pub async fn connect(
        &self,
        node: &Node,
        kernel_override: Option<KernelPreference>,
    ) -> Result<()> {
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
    }

    async fn start_inner(&self, node: &Node) -> Result<()> {
        self.start_inner_with_preference(node, None, None).await
    }

    async fn start_inner_with_snapshot(
        &self,
        node: &Node,
        preserved_system_proxy_snapshot: Option<SystemProxySnapshot>,
    ) -> Result<()> {
        self.start_inner_with_preference(node, preserved_system_proxy_snapshot, None)
            .await
    }

    async fn start_inner_with_preference(
        &self,
        node: &Node,
        preserved_system_proxy_snapshot: Option<SystemProxySnapshot>,
        kernel_override: Option<KernelPreference>,
    ) -> Result<()> {
        let options = self.runtime.lock().await.options.clone();
        let preference = kernel_override.unwrap_or(options.kernel_preference);
        let kernel = match select_proxy_kernel_preference(
            &node.protocol,
            preference,
            options.mode == ProxyMode::System,
        ) {
            Ok(kernel) => kernel,
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
            .await
    }

    #[cfg(test)]
    async fn start_with_path(&self, node: &Node, mihomo_path: &Path) -> Result<()> {
        self.start_with_kernel_path(node, KernelKind::Mihomo, mihomo_path)
            .await
    }

    #[cfg(test)]
    async fn start_with_kernel_path(
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
        let startup_started = tokio::time::Instant::now();
        let startup_deadline = startup_started + STARTUP_PROXY_TOTAL_TIMEOUT;
        let mut runtime = self.runtime.lock().await;
        if matches!(
            runtime.status.state,
            ProxyState::Starting | ProxyState::Running | ProxyState::Stopping
        ) {
            return Err(anyhow!("代理正在运行或切换状态"));
        }
        if runtime.options.mode == ProxyMode::Tun && !crate::platform::is_elevated() {
            return Err(anyhow!("TUN 模式需要管理员权限"));
        }
        // 分流数据库没备好就别急着起内核：半截库会让内核直接拒绝启动，而且报出来的
        // 是一句要人复制才能看懂的长错误。这里缺就补（镜像优先，其次系统代理）。
        if kernel == KernelKind::Mihomo
            && runtime.options.rules.iter().any(TrafficRule::uses_geo)
        {
            let dir = crate::geo::mihomo_dir();
            if !crate::geo::geo_ready(&dir) {
                let system = crate::system_proxy::current_system_proxy_url();
                if let Err(error) =
                    crate::geo::provision(&dir, system.as_deref(), false).await
                {
                    return Err(error);
                }
            }
        }

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

        let mut child = match command.spawn() {
            Ok(child) => {
                if let Some(pid) = child.id() {
                    crate::platform::assign_child_process_id(pid);
                }
                child
            }
            Err(error) => {
                let _ = work_dir.cleanup().await;
                let message = format!("启动 {} 失败: {error}", kernel.display_name());
                runtime.status.state = ProxyState::Failed;
                runtime.status.message = message.clone();
                return Err(anyhow!(message));
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
                let _ = child.kill().await;
                let _ = child.wait().await;
                let detail = tokio::fs::read_to_string(work_dir.log_path())
                    .await
                    .unwrap_or_default();
                let _ = work_dir.cleanup().await;
                let message = if detail.trim().is_empty() {
                    format!("{} 启动后端口 {port} 未就绪", kernel.display_name())
                } else {
                    format!(
                        "{} 启动失败: {}",
                        kernel.display_name(),
                        last_nonempty_line(&detail)
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

        if VERIFY_CONNECTIVITY_ON_START {
            let remaining = startup_deadline.saturating_duration_since(tokio::time::Instant::now());
            let system_proxy_reserve = if runtime.options.mode == ProxyMode::System
                && preserved_system_proxy_snapshot.is_none()
            {
                STARTUP_SYSTEM_PROXY_RESERVE
            } else {
                Duration::ZERO
            };
            let verify_timeout = remaining
                .saturating_sub(system_proxy_reserve)
                .min(STARTUP_PROXY_VERIFY_TIMEOUT);
            if verify_timeout.is_zero() {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = work_dir.cleanup().await;
                let message = format!(
                    "{} 启动超过 {} 秒，节点验证未执行",
                    kernel.display_name(),
                    STARTUP_PROXY_TOTAL_TIMEOUT.as_secs()
                );
                mark_failed(&mut runtime, message.clone());
                return Err(anyhow!(message));
            }
            if let Err(error) = verify_local_proxy(runtime.options.mixed_port, verify_timeout).await
            {
                let detail = tokio::fs::read_to_string(work_dir.log_path())
                    .await
                    .unwrap_or_default();
                let diagnostic = format_kernel_diagnostic(&detail);
                if diagnostic.contains("规则加载中")
                    || diagnostic.contains("自动建立连接")
                    || diagnostic.contains("start initial configuration")
                {
                    if runtime.options.mode == ProxyMode::System {
                        if let Some(snapshot) = preserved_system_proxy_snapshot.take() {
                            runtime.system_proxy_snapshot = Some(snapshot);
                        } else if runtime.system_proxy_snapshot.is_none() {
                            let server = format!("127.0.0.1:{}", runtime.options.mixed_port);
                            let terminal_port = runtime
                                .options
                                .terminal_proxy
                                .then_some(runtime.options.mixed_port);
                            runtime.system_proxy_snapshot = Some(
                                activate_system_proxy(
                                    self.system_proxy.clone(),
                                    server,
                                    terminal_port,
                                )
                                .await?,
                            );
                        }
                    }
                    runtime.status.state = ProxyState::Running;
                    runtime.status.local_proxy_ready = true;
                    runtime.status.system_proxy_enabled = runtime.system_proxy_snapshot.is_some();
                    runtime.status.last_error = None;
                    runtime.status.message =
                        format!("{} 运行中（初次连接自动建立中）", kernel.display_name());
                    runtime.child = Some(child);
                    runtime.work_dir = Some(work_dir);
                    return Ok(());
                }
                let message = if detail.trim().is_empty() {
                    format!("节点连通性验证失败: {error}")
                } else {
                    format!(
                        "节点连通性验证失败: {error}; {}: {diagnostic}",
                        kernel.display_name()
                    )
                };
                if runtime.options.mode == ProxyMode::System {
                    if let Some(snapshot) = preserved_system_proxy_snapshot.take() {
                        runtime.system_proxy_snapshot = Some(snapshot);
                    } else if runtime.system_proxy_snapshot.is_none() {
                        let server = format!("127.0.0.1:{}", runtime.options.mixed_port);
                        let terminal_port = runtime
                            .options
                            .terminal_proxy
                            .then_some(runtime.options.mixed_port);
                        runtime.system_proxy_snapshot = Some(
                            activate_system_proxy(self.system_proxy.clone(), server, terminal_port)
                                .await?,
                        );
                    }
                }
                runtime.status.state = ProxyState::Degraded;
                runtime.status.system_proxy_enabled = runtime.system_proxy_snapshot.is_some();
                runtime.status.last_error = Some(message.clone());
                runtime.status.message = format!("{} 降级运行: {message}", kernel.display_name());
                runtime.child = Some(child);
                runtime.work_dir = Some(work_dir);
                return Ok(());
            }

            tracing::info!(
                kernel = kernel.display_name(),
                stage = "connectivity_verified",
                elapsed_ms = startup_started.elapsed().as_millis() as u64,
                "代理启动阶段完成"
            );
        }

        if runtime.options.mode == ProxyMode::System {
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

        let elapsed_ms = startup_started.elapsed().as_millis() as u64;
        runtime.status.state = ProxyState::Running;
        runtime.status.local_proxy_ready = true;
        runtime.status.system_proxy_enabled = runtime.system_proxy_snapshot.is_some();
        runtime.status.last_error = None;
        runtime.status.message = format!(
            "{} 运行中（启动耗时 {elapsed_ms} ms）",
            kernel.display_name()
        );
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

    async fn stop_inner_for_restart(&self) -> Result<Option<SystemProxySnapshot>> {
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

    async fn try_hot_reload_mihomo(&self, node: &Node) -> Result<bool> {
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

fn build_kernel_config(
    kernel: KernelKind,
    node: &Node,
    options: MihomoConfigOptions,
) -> Result<Value> {
    kernel_adapter(kernel).build_config(node, options)
}

fn serialize_kernel_config(kernel: KernelKind, config: &Value) -> Result<String> {
    kernel_adapter(kernel).serialize_config(config)
}

fn configure_kernel_command(command: &mut Command, kernel: KernelKind, work_dir: &ProxyWorkDir) {
    kernel_adapter(kernel).configure_command(command, work_dir);
}

async fn validate_kernel_config(
    kernel: KernelKind,
    executable: &Path,
    work_dir: &ProxyWorkDir,
) -> Result<()> {
    let adapter = kernel_adapter(kernel);
    let Some(args) = adapter.validation_args(work_dir) else {
        return Ok(());
    };
    let mut command = Command::new(executable);
    command.args(args).stdin(Stdio::null());
    if kernel == KernelKind::Xray {
        command
            .current_dir(managed_kernel_root().join(KernelKind::Xray.directory_name()))
            .env(
                "XRAY_LOCATION_ASSET",
                managed_kernel_root().join(KernelKind::Xray.directory_name()),
            );
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = command.output().await?;
    if output.status.success() {
        return Ok(());
    }
    let detail = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Err(anyhow!("{}", last_nonempty_line(&detail)))
}

async fn wait_for_port(child: &mut Child, port: u16, timeout: Duration) -> bool {
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
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

async fn verify_local_proxy(port: u16, timeout: Duration) -> Result<()> {
    let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?;
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_errors = Vec::new();

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let attempt_timeout = remaining.min(Duration::from_millis(900));
        let client = match reqwest::Client::builder()
            .proxy(proxy.clone())
            .connect_timeout(Duration::from_millis(600))
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

async fn ensure_port_available(port: u16) -> Result<()> {
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
                    tokio::time::sleep(tokio::time::Duration::from_millis(60)).await;
                } else if attempt == 1 {
                    #[cfg(not(test))]
                    {
                        let cleaned = crate::platform::kill_orphan_proxy_cores();
                        if cleaned > 0 {
                            tracing::info!("清理了 {cleaned} 个残留代理内核进程以释放端口 {port}");
                        }
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(120)).await;
                } else if attempt == 2 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    }
    let error = last_err
        .map(|e| e.to_string())
        .unwrap_or_else(|| "未知错误".into());
    Err(anyhow!("代理端口 {port} 已被占用: {error}"))
}

fn required_startup_ports(kernel: KernelKind, options: &MihomoConfigOptions) -> Vec<u16> {
    kernel_adapter(kernel).required_ports(options)
}

trait KernelAdapter: Send + Sync {
    fn kind(&self) -> KernelKind;
    fn supports_protocol(&self, protocol: &str) -> bool {
        self.kind().supports_proxy_protocol(protocol)
    }
    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value>;
    fn serialize_config(&self, config: &Value) -> Result<String>;
    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir);
    fn validation_args(&self, _work_dir: &ProxyWorkDir) -> Option<Vec<std::ffi::OsString>> {
        None
    }
    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16;
    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16>;
    fn batch_size(&self) -> usize;
    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        batch_size: usize,
        conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>>;
}

struct MihomoAdapter;
struct SingBoxAdapter;
struct XrayAdapter;

static MIHOMO_ADAPTER: MihomoAdapter = MihomoAdapter;
static SING_BOX_ADAPTER: SingBoxAdapter = SingBoxAdapter;
static XRAY_ADAPTER: XrayAdapter = XrayAdapter;

fn kernel_adapter(kernel: KernelKind) -> &'static dyn KernelAdapter {
    match kernel {
        KernelKind::Mihomo => &MIHOMO_ADAPTER,
        KernelKind::SingBox => &SING_BOX_ADAPTER,
        KernelKind::Xray => &XRAY_ADAPTER,
    }
}

impl KernelAdapter for MihomoAdapter {
    fn kind(&self) -> KernelKind {
        KernelKind::Mihomo
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value> {
        build_mihomo_config(node, options)
    }

    fn serialize_config(&self, config: &Value) -> Result<String> {
        Ok(serde_yaml::to_string(config)?)
    }

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir) {
        command
            .arg("-d")
            .arg(work_dir.data_root())
            .arg("-f")
            .arg(work_dir.config_path());
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16 {
        options.controller_port
    }

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16> {
        vec![options.mixed_port, options.controller_port]
    }

    fn batch_size(&self) -> usize {
        256
    }

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        batch_size: usize,
        conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>> {
        Box::pin(async move {
            crate::proxy_probe::verify_nodes_with_mihomo_progress_limited(
                nodes,
                timeout,
                progress.map(|callback| move |state| callback(state)),
                network_limit,
                batch_size,
                conditional_isolation,
            )
            .await
        })
    }
}

impl KernelAdapter for SingBoxAdapter {
    fn kind(&self) -> KernelKind {
        KernelKind::SingBox
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value> {
        build_sing_box_config(node, options)
    }

    fn serialize_config(&self, config: &Value) -> Result<String> {
        Ok(serde_json::to_string_pretty(config)?)
    }

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir) {
        command
            .arg("run")
            .arg("-D")
            .arg(work_dir.root())
            .arg("-c")
            .arg(work_dir.config_path());
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16 {
        options.mixed_port
    }

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16> {
        vec![options.mixed_port]
    }

    fn batch_size(&self) -> usize {
        64
    }

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        _batch_size: usize,
        _conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>> {
        Box::pin(async move {
            sing_box::verify_nodes_with_sing_box_progress_limited(
                nodes,
                timeout,
                progress.map(|callback| move |state| callback(state)),
                network_limit,
            )
            .await
        })
    }
}

impl KernelAdapter for XrayAdapter {
    fn kind(&self) -> KernelKind {
        KernelKind::Xray
    }

    fn build_config(&self, node: &Node, options: MihomoConfigOptions) -> Result<Value> {
        build_xray_config(node, options)
    }

    fn serialize_config(&self, config: &Value) -> Result<String> {
        Ok(serde_json::to_string_pretty(config)?)
    }

    fn configure_command(&self, command: &mut Command, work_dir: &ProxyWorkDir) {
        let asset_dir = managed_kernel_root().join(KernelKind::Xray.directory_name());
        command
            .current_dir(&asset_dir)
            .env("XRAY_LOCATION_ASSET", asset_dir)
            .arg("run")
            .arg("-c")
            .arg(work_dir.config_path());
    }

    fn validation_args(&self, work_dir: &ProxyWorkDir) -> Option<Vec<std::ffi::OsString>> {
        Some(vec![
            "run".into(),
            "-test".into(),
            "-c".into(),
            work_dir.config_path().as_os_str().to_owned(),
        ])
    }

    fn readiness_port(&self, options: &MihomoConfigOptions) -> u16 {
        options.mixed_port
    }

    fn required_ports(&self, options: &MihomoConfigOptions) -> Vec<u16> {
        vec![options.mixed_port]
    }

    fn batch_size(&self) -> usize {
        64
    }

    fn verify_nodes<'a>(
        &'a self,
        nodes: Vec<Node>,
        timeout: Duration,
        progress: Option<&'a (dyn Fn(ProxyProbeProgress) + Send + Sync)>,
        network_limit: Arc<Semaphore>,
        _batch_size: usize,
        _conditional_isolation: bool,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<ProxyProbeResult>>> + Send + 'a>> {
        Box::pin(async move {
            xray::verify_nodes_with_xray_progress_limited(
                nodes,
                timeout,
                progress.map(|callback| move |state| callback(state)),
                network_limit,
            )
            .await
        })
    }
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

fn mark_failed(runtime: &mut ProxyRuntime, message: String) {
    runtime.status.state = ProxyState::Failed;
    runtime.status.local_proxy_ready = false;
    runtime.status.last_error = Some(message.clone());
    runtime.status.message = message;
}

async fn activate_system_proxy(
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
    // 终端代理变量与系统代理同生同死：Git / curl / npm / pip 只认环境变量，不写它们
    // 就是"系统代理开了，终端里还是连不上"。
    if let Some(port) = terminal_proxy_port {
        match crate::system_proxy::terminal::enable(port) {
            Ok(report) => tracing::info!(summary = report.summary(), "终端代理已启用"),
            Err(error) => tracing::warn!(%error, "终端代理启用失败"),
        }
    }
    Ok(snapshot)
}

/// 上一次会话被强杀后，注册表里留下的就是我们自己的端口。把它当成"要还原的原值"，
/// 停止代理时就会把系统代理指回已经死掉的端口（浏览器直接断网），
/// 所以这种情况的还原目标改为"关闭系统代理"。
fn sanitize_self_referential(snapshot: SystemProxySnapshot, server: &str) -> SystemProxySnapshot {
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

async fn restore_system_proxy(
    controller: Arc<dyn SystemProxyControl>,
    snapshot: SystemProxySnapshot,
    clear_terminal_proxy: bool,
) -> Result<()> {
    tokio::task::spawn_blocking(move || controller.restore(&snapshot))
        .await
        .context("系统代理恢复任务执行失败")??;
    if clear_terminal_proxy {
        match crate::system_proxy::terminal::disable() {
            Ok(report) => tracing::info!(summary = report.summary(), "终端代理已清除"),
            Err(error) => tracing::warn!(%error, "终端代理清除失败"),
        }
    }
    Ok(())
}

fn last_nonempty_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or("未知错误")
}

fn strip_ansi(s: &str) -> String {
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

fn format_kernel_diagnostic(raw: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Node;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn self_referential_system_proxy_snapshot_is_reset_to_off() {
        // 上次会话残留的"就是我们自己端口"的系统代理，不能当成还原目标。
        let leftover = SystemProxySnapshot {
            enabled: true,
            server: Some("127.0.0.1:9980".into()),
            override_list: Some("<local>;localhost".into()),
        };
        let cleaned = sanitize_self_referential(leftover, "127.0.0.1:9980");
        assert!(!cleaned.enabled);
        assert_eq!(cleaned.server, None);
        assert_eq!(cleaned.override_list, None);

        // 用户原本的代理要原样保留。
        let users = SystemProxySnapshot {
            enabled: true,
            server: Some("127.0.0.1:10808".into()),
            override_list: None,
        };
        let kept = sanitize_self_referential(users.clone(), "127.0.0.1:9980");
        assert_eq!(kept, users);

        // 协议分组形式（http=...;https=...）里指向我方端口，同样要清掉。
        let grouped = SystemProxySnapshot {
            enabled: true,
            server: Some("http=127.0.0.1:9980;https=127.0.0.1:9980".into()),
            override_list: None,
        };
        assert!(!sanitize_self_referential(grouped, "127.0.0.1:9980").enabled);

        // 本来就未启用的快照保持原样。
        let off = SystemProxySnapshot {
            enabled: false,
            server: Some("127.0.0.1:9980".into()),
            override_list: None,
        };
        assert!(!sanitize_self_referential(off.clone(), "127.0.0.1:9980").enabled);
        assert_eq!(
            sanitize_self_referential(off.clone(), "127.0.0.1:9980").server,
            off.server
        );
    }

    #[derive(Default)]
    struct FakeSystemProxy {
        enabled: StdMutex<Vec<String>>,
        restored: StdMutex<Vec<SystemProxySnapshot>>,
    }

    impl SystemProxyControl for FakeSystemProxy {
        fn capture(&self) -> Result<SystemProxySnapshot> {
            Ok(SystemProxySnapshot {
                enabled: false,
                server: Some("old-proxy:8080".into()),
                override_list: Some("<local>".into()),
            })
        }

        fn enable(&self, server: &str) -> Result<()> {
            self.enabled.lock().unwrap().push(server.into());
            Ok(())
        }

        fn restore(&self, snapshot: &SystemProxySnapshot) -> Result<()> {
            self.restored.lock().unwrap().push(snapshot.clone());
            Ok(())
        }
    }

    fn node(protocol: &str, config: serde_json::Value) -> Node {
        let now = Utc::now();
        Node {
            id: format!("{protocol}-id"),
            source_id: None,
            protocol: protocol.into(),
            name: "test-node".into(),
            server: config["server"].as_str().unwrap().into(),
            port: config["port"].as_u64().unwrap() as u16,
            raw: String::new(),
            config,
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn stopped_status_uses_default_ports() {
        let status = ProxyRuntimeStatus::stopped(MihomoConfigOptions::default());

        assert_eq!(status.state, ProxyState::Stopped);
        assert_eq!(status.mode, ProxyMode::System);
        assert_eq!(status.mixed_port, 9980);
        assert_eq!(status.controller_port, 9090);
        assert!(status.node_id.is_none());
        assert!(!status.local_proxy_ready);
        assert!(!status.system_proxy_enabled);
        assert!(status.last_error.is_none());
    }

    #[test]
    fn startup_proxy_verification_uses_five_second_total_budget() {
        assert_eq!(STARTUP_PROXY_TOTAL_TIMEOUT, Duration::from_secs(5));
        assert_eq!(STARTUP_PROXY_VERIFY_TIMEOUT, Duration::from_millis(1_500));
        assert_eq!(STARTUP_SYSTEM_PROXY_RESERVE, Duration::from_millis(250));
        assert_eq!(STARTUP_PROXY_VERIFY_URLS, [PROXY_CONNECTIVITY_TEST_URL]);
    }

    #[test]
    fn startup_requires_google_connectivity_verification() {
        assert!(VERIFY_CONNECTIVITY_ON_START);
    }

    #[test]
    fn mihomo_requires_both_proxy_and_controller_ports() {
        let options = MihomoConfigOptions {
            mixed_port: 17890,
            controller_port: 19090,
            ..MihomoConfigOptions::default()
        };

        assert_eq!(
            required_startup_ports(KernelKind::Mihomo, &options),
            [17890, 19090]
        );
        assert_eq!(
            required_startup_ports(KernelKind::SingBox, &options),
            [17890]
        );
    }

    #[test]
    fn kernel_adapters_expose_runtime_and_batch_contracts() {
        let options = MihomoConfigOptions::default();
        let cases = [
            (KernelKind::Mihomo, 256, options.controller_port, 2),
            (KernelKind::SingBox, 64, options.mixed_port, 1),
            (KernelKind::Xray, 64, options.mixed_port, 1),
        ];

        for (kind, batch_size, ready_port, port_count) in cases {
            let adapter = kernel_adapter(kind);
            assert_eq!(adapter.kind(), kind);
            assert_eq!(adapter.batch_size(), batch_size);
            assert_eq!(adapter.readiness_port(&options), ready_port);
            assert_eq!(adapter.required_ports(&options).len(), port_count);
        }
        assert!(kernel_adapter(KernelKind::Xray).supports_protocol("vless"));
        assert!(!kernel_adapter(KernelKind::Xray).supports_protocol("hysteria2"));
    }

    #[test]
    fn classifies_only_node_connectivity_startup_failures() {
        assert!(is_node_connectivity_failure(
            "节点连通性验证失败: 本地代理无法访问测试地址"
        ));
        assert!(!is_node_connectivity_failure("Mihomo 配置生成失败"));
        assert!(!is_node_connectivity_failure("代理端口 7890 已被占用"));
    }

    #[test]
    fn builds_minimal_mihomo_config_for_selected_node() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );

        let config = build_mihomo_config(&selected, MihomoConfigOptions::default()).unwrap();

        assert_eq!(config["mixed-port"], 9980);
        assert_eq!(config["external-controller"], "127.0.0.1:9090");
        assert_eq!(config["proxies"].as_array().unwrap().len(), 1);
        assert_eq!(config["proxy-groups"][0]["name"], "PROXY");
        // 没有用户规则时，只剩两条"镜像钉直连"+ 兜底 MATCH。
        let rules = config["rules"].as_array().unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
    }

    #[test]
    fn recommended_routing_preserves_order_enables_geo_and_keeps_proxy_fallback_last() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let options = MihomoConfigOptions {
            rules: recommended_traffic_rules(),
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&selected, options).unwrap();
        let rules = config["rules"].as_array().unwrap();

        assert_eq!(config["geodata-mode"], true);
        // 18 条用户规则 + 两条"镜像钉直连"，MATCH 仍是最后一条。
        assert_eq!(rules.len(), 20);
        assert_eq!(rules[11], "GEOSITE,category-ads-all,REJECT");
        assert_eq!(rules[12], "GEOSITE,cn,DIRECT");
        assert_eq!(rules[13], "GEOSITE,google,PROXY");
        assert_eq!(rules[14], "GEOSITE,youtube,PROXY");
        assert_eq!(rules[15], "GEOIP,CN,DIRECT,no-resolve");
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
    }

    #[test]
    fn url_test_mode_emits_three_groups_and_routes_match_to_final_group() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let candidate = node(
            "vless",
            json!({"server": "5.6.7.8", "port": 443, "uuid": "u2", "network": "ws", "security": "tls"}),
        );
        let mut options = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            url_test: true,
            url_test_size: 3,
            ..MihomoConfigOptions::default()
        };
        options.url_test_nodes =
            crate::export::nodes_to_clash_proxies(std::slice::from_ref(&candidate));

        let config = build_mihomo_config(&current, options).unwrap();
        let groups = config["proxy-groups"].as_array().unwrap();
        let names: Vec<&str> = groups
            .iter()
            .map(|group| group["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, vec!["PROXY", URL_TEST_GROUP, FINAL_GROUP]);
        assert_eq!(groups[1]["type"], "url-test");
        // PROXY 的第一个成员必须是「自动选择」：默认让内核自己挑，锁定单节点才只是
        // 一次 PUT /proxies/PROXY。
        assert_eq!(groups[0]["proxies"][0], URL_TEST_GROUP);
        assert_eq!(
            config["rules"].as_array().unwrap().last().unwrap(),
            &Value::String(format!("MATCH,{FINAL_GROUP}"))
        );
        assert_eq!(config["proxies"].as_array().unwrap().len(), 2);
    }

    /// 候选注入 → runtime.options → 配置生成 这条链以前是静默断的（SQL 拼参报错被
    /// 吞掉），所以把三段接起来测一次。
    #[tokio::test]
    async fn set_url_test_candidates_reaches_generated_config() {
        let candidate_a = node(
            "vless",
            json!({"server": "5.6.7.8", "port": 443, "uuid": "ua", "network": "ws", "security": "tls"}),
        );
        let candidate_b = node(
            "vless",
            json!({"server": "9.9.9.9", "port": 443, "uuid": "ub", "network": "ws", "security": "tls"}),
        );
        let manager = ProxyManager::new(MihomoConfigOptions {
            url_test: true,
            url_test_size: 20,
            ..MihomoConfigOptions::default()
        });
        assert_eq!(
            manager
                .set_url_test_candidates(vec![candidate_a, candidate_b])
                .await,
            2
        );

        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let config = build_mihomo_config(&current, manager.config().await).unwrap();
        let groups = config["proxy-groups"].as_array().unwrap();
        assert_eq!(groups.len(), 3, "候选应该生成 PROXY / 自动选择 / 漏网之鱼");
        assert_eq!(groups[1]["type"], "url-test");
        assert_eq!(groups[1]["proxies"].as_array().unwrap().len(), 3);
        // 判活节奏必须真的写进配置：这台客户端"坏节点长时间不被避开"就是栽在 300 秒一轮。
        assert_eq!(groups[1]["interval"], json!(URL_TEST_INTERVAL_SECS));
        assert_eq!(groups[1]["tolerance"], json!(URL_TEST_TOLERANCE_MS));
        assert_eq!(groups[1]["timeout"], json!(URL_TEST_TIMEOUT_MS));
        assert_eq!(
            URL_TEST_INTERVAL_SECS, 60,
            "url-test 间隔被改回分钟级，坏节点又要几分钟才被避开"
        );
    }

    #[test]
    fn url_test_off_keeps_single_select_group_and_proxy_match() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let options = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&current, options).unwrap();
        assert_eq!(config["proxy-groups"].as_array().unwrap().len(), 1);
        assert_eq!(config["proxies"].as_array().unwrap().len(), 1);
        assert_eq!(
            config["rules"].as_array().unwrap().last().unwrap(),
            &Value::String("MATCH,PROXY".into())
        );
    }

    #[test]
    fn china_direct_mode_places_dns_poison_overrides_before_geoip() {
        // 境内 DoH 把 googlevideo 污染成国内 IP 时，只有先按域名定性才不会误判直连。
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let options = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&selected, options).unwrap();
        let rules = config["rules"].as_array().unwrap();
        let geoip = rules
            .iter()
            .position(|rule| rule == "GEOIP,CN,DIRECT,no-resolve")
            .expect("GEOIP,CN,DIRECT,no-resolve");
        assert_eq!(&rules[geoip - 2], "GEOSITE,google,PROXY");
        assert_eq!(&rules[geoip - 1], "GEOSITE,youtube,PROXY");
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
        assert!(config["dns"].get("fallback").is_none());
    }

    #[test]
    fn routing_presets_generate_expected_mihomo_fallbacks() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let cases = [
            (RoutingMode::ChinaDirect, "MATCH,PROXY"),
            (RoutingMode::GfwList, "MATCH,DIRECT"),
            (RoutingMode::Global, "MATCH,PROXY"),
            (RoutingMode::Direct, "MATCH,DIRECT"),
        ];

        for (routing_mode, fallback) in cases {
            let config = build_mihomo_config(
                &selected,
                MihomoConfigOptions {
                    routing_mode,
                    ..MihomoConfigOptions::default()
                },
            )
            .unwrap();
            assert_eq!(
                config["rules"].as_array().unwrap().last().unwrap(),
                fallback
            );
        }
    }

    #[test]
    fn online_mihomo_presets_enable_geo_updates() {
        let selected = node(
            "trojan",
            json!({"server":"1.2.3.4","port":443,"password":"p"}),
        );
        let config = build_mihomo_config(
            &selected,
            MihomoConfigOptions {
                routing_mode: RoutingMode::GfwList,
                ..MihomoConfigOptions::default()
            },
        )
        .unwrap();

        assert_eq!(config["geo-auto-update"], false);
        assert!(config["geox-url"]["geosite"]
            .as_str()
            .unwrap()
            .contains("Loyalsoldier"));
        assert_eq!(config["rules"][1], "GEOSITE,gfw,PROXY");
    }

    #[test]
    fn builds_tun_dns_and_ordered_routing_config() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let options = MihomoConfigOptions {
            mode: ProxyMode::Tun,
            rules: vec![
                TrafficRule {
                    matcher: RuleMatcher::GeoSite("cn".into()),
                    action: RuleAction::Direct,
                },
                TrafficRule {
                    matcher: RuleMatcher::GeoIp("CN".into()),
                    action: RuleAction::Direct,
                },
                TrafficRule {
                    matcher: RuleMatcher::Quic,
                    action: RuleAction::Reject,
                },
            ],
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&selected, options).unwrap();

        assert_eq!(config["tun"]["enable"], true);
        assert_eq!(config["tun"]["stack"], "mixed");
        assert_eq!(config["tun"]["dns-hijack"][0], "any:53");
        assert_eq!(config["dns"]["enhanced-mode"], "fake-ip");
        assert_eq!(config["dns"]["nameserver"][0], DEFAULT_DNS_PRIMARY);
        assert_eq!(config["dns"]["nameserver"][1], DEFAULT_DNS_SECONDARY);
        assert_eq!(config["geodata-mode"], true);
        assert_eq!(config["rules"][0], "GEOSITE,cn,DIRECT");
        // 生成配置时污染免疫规则被插在 GEOIP,CN 之前，用户规则表本身不变。
        assert_eq!(config["rules"][1], "GEOSITE,google,PROXY");
        assert_eq!(config["rules"][2], "GEOSITE,youtube,PROXY");
        assert_eq!(config["rules"][3], "GEOIP,CN,DIRECT,no-resolve");
        assert_eq!(
            config["rules"][4],
            "AND,((NETWORK,udp),(DST-PORT,443)),REJECT"
        );
        assert_eq!(config["rules"][5], "DOMAIN-SUFFIX,gh-proxy.com,DIRECT");
        assert_eq!(config["rules"][6], "DOMAIN-SUFFIX,ghfast.top,DIRECT");
        assert_eq!(config["rules"][7], "MATCH,PROXY");
    }

    #[test]
    fn system_mode_does_not_emit_tun_config() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111"
            }),
        );

        let config = build_mihomo_config(&selected, MihomoConfigOptions::default()).unwrap();

        assert!(config.get("tun").is_none());
    }

    #[test]
    fn rejects_node_unsupported_by_mihomo_export() {
        let selected = node(
            "mieru",
            json!({"server": "1.2.3.4", "port": 443, "password": "secret"}),
        );

        let error = build_mihomo_config(&selected, MihomoConfigOptions::default()).unwrap_err();

        assert!(error.to_string().contains("不支持"));
    }

    #[tokio::test]
    async fn work_dir_uses_kernel_specific_paths_and_cleans_up() {
        for (kernel, config_name, log_name) in [
            (KernelKind::Mihomo, "config.yaml", "mihomo.log"),
            (KernelKind::SingBox, "config.json", "sing-box.log"),
        ] {
            let work_dir = ProxyWorkDir::create(kernel).await.unwrap();
            let root = work_dir.root().to_path_buf();

            assert!(root.is_dir());
            assert_eq!(work_dir.config_path(), root.join(config_name));
            assert_eq!(work_dir.log_path(), root.join(log_name));
            match kernel {
                KernelKind::Mihomo => assert_eq!(
                    work_dir.data_root(),
                    crate::kernel::managed_kernel_root().join("mihomo")
                ),
                KernelKind::SingBox => assert_eq!(work_dir.data_root(), root),
                KernelKind::Xray => unreachable!(),
            }

            work_dir.cleanup().await.unwrap();
            assert!(!root.exists());
        }
    }

    #[tokio::test]
    async fn manager_starts_in_stopped_state() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());

        let status = manager.status().await;

        assert_eq!(status.state, ProxyState::Stopped);
        assert!(status.node_id.is_none());
    }

    #[test]
    fn degraded_status_distinguishes_ready_and_waiting_states() {
        let mut status = ProxyRuntimeStatus::stopped(MihomoConfigOptions::default());
        status.state = ProxyState::Degraded;
        status.system_proxy_enabled = true;
        status.last_error = Some("验证超时".into());

        status.local_proxy_ready = true;
        assert!(status.local_proxy_active());
        assert!(!status.config_editable());

        status.local_proxy_ready = false;
        assert!(!status.local_proxy_active());
        assert!(status.config_editable());
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["system_proxy_enabled"], true);
        assert_eq!(json["last_error"], "验证超时");
    }

    #[tokio::test]
    async fn manager_updates_valid_config_while_stopped() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        let mut options = manager.config().await;
        options.mode = ProxyMode::Tun;
        options.rules.push(TrafficRule {
            matcher: RuleMatcher::GeoSite("cn".into()),
            action: RuleAction::Direct,
        });

        manager.update_config(options.clone()).await.unwrap();

        assert_eq!(manager.config().await, options);
        assert_eq!(manager.status().await.mode, ProxyMode::Tun);
    }

    #[tokio::test]
    async fn manager_rejects_config_changes_while_running() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        manager.runtime.lock().await.status.state = ProxyState::Running;

        let error = manager
            .update_config(MihomoConfigOptions::default())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("停止代理"));
    }

    #[tokio::test]
    async fn missing_mihomo_executable_sets_failed_status() {
        let controller = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let controller_port = controller.local_addr().unwrap().port();
        drop(controller);
        let mixed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mixed_port = mixed.local_addr().unwrap().port();
        drop(mixed);
        let manager = ProxyManager::new(MihomoConfigOptions {
            mixed_port,
            controller_port,
            ..MihomoConfigOptions::default()
        });
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let missing = std::env::temp_dir().join(format!(
            "vpn-parser-missing-mihomo-{}.exe",
            uuid::Uuid::new_v4()
        ));

        let error = manager
            .start_with_path(&selected, &missing)
            .await
            .unwrap_err();
        let status = manager.status().await;

        assert!(error.to_string().contains("启动"));
        assert_eq!(status.state, ProxyState::Failed);
        assert!(status.message.contains("启动"));
    }

    #[tokio::test]
    async fn missing_sing_box_executable_keeps_selected_kernel_in_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mixed_port = listener.local_addr().unwrap().port();
        drop(listener);
        let manager = ProxyManager::new(MihomoConfigOptions {
            mixed_port,
            ..MihomoConfigOptions::default()
        });
        let selected = node(
            "anytls",
            json!({
                "server": "anytls.example.com",
                "port": 443,
                "password": "secret"
            }),
        );
        let missing = std::env::temp_dir().join(format!(
            "vpn-parser-missing-sing-box-{}.exe",
            uuid::Uuid::new_v4()
        ));

        let error = manager
            .start_with_kernel_path(&selected, KernelKind::SingBox, &missing)
            .await
            .unwrap_err();
        let status = manager.status().await;

        assert!(error.to_string().contains("Sing-box"));
        assert_eq!(status.state, ProxyState::Failed);
        assert_eq!(status.kernel, Some(KernelKind::SingBox));
        assert!(status.message.contains("Sing-box"));
    }

    #[tokio::test]
    async fn occupied_controller_port_is_rejected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let error = ensure_port_available(port).await.unwrap_err();

        assert!(error.to_string().contains("已被占用"));
    }

    #[tokio::test]
    async fn startup_probe_rejects_proxy_that_cannot_reach_test_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let started = tokio::time::Instant::now();
        let error = verify_local_proxy(port, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("本地代理无法访问测试地址"));
        assert!(started.elapsed() < Duration::from_millis(1_500));
    }

    #[tokio::test]
    async fn system_proxy_activation_returns_snapshot_for_restore() {
        let controller = Arc::new(FakeSystemProxy::default());

        let snapshot = activate_system_proxy(controller.clone(), "127.0.0.1:7890".into(), None)
            .await
            .unwrap();
        restore_system_proxy(controller.clone(), snapshot.clone(), false)
            .await
            .unwrap();

        assert_eq!(
            controller.enabled.lock().unwrap().as_slice(),
            ["127.0.0.1:7890"]
        );
        assert_eq!(controller.restored.lock().unwrap().as_slice(), [snapshot]);
    }

    #[tokio::test]
    async fn restart_stop_preserves_system_proxy_without_restoring_it() {
        let controller = Arc::new(FakeSystemProxy::default());
        let manager =
            ProxyManager::with_system_proxy(MihomoConfigOptions::default(), controller.clone());
        let snapshot = SystemProxySnapshot {
            enabled: false,
            server: Some("old-proxy:8080".into()),
            override_list: Some("<local>".into()),
        };
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.status.state = ProxyState::Running;
            runtime.system_proxy_snapshot = Some(snapshot.clone());
        }

        let preserved = manager.stop_inner_for_restart().await.unwrap();

        assert_eq!(preserved, Some(snapshot));
        assert!(controller.restored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn switch_node_falls_back_when_not_running() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        let test_node = node(
            "ss",
            json!({
                "server": "1.2.3.4",
                "port": 8388,
                "cipher": "aes-128-gcm",
                "password": "pass"
            }),
        );
        let res = manager.try_hot_reload_mihomo(&test_node).await.unwrap();
        assert!(!res);
    }

    #[test]
    fn format_kernel_diagnostic_cleans_raw_errors() {
        let raw1 = r#"time="2026-09-04T13:00:45.748268800+08:00" level=warning msg="[TCP] dial PROXY (match Match/) 127.0.0.1:53655 --> www.google.com:443 error: failed to dial WebSocket: read tcp 192.168.0.103:53662->104.18.42.163:2087: wsarecv: An existing connection was forcibly closed by the remote host.""#;
        let formatted1 = format_kernel_diagnostic(raw1);
        assert!(formatted1.contains("远程服务器强制重置/关闭连接 (目标节点已不可用)"));

        let raw2 = r#"error: dial tcp: i/o timeout"#;
        let formatted2 = format_kernel_diagnostic(raw2);
        assert!(formatted2.contains("连接远程服务器超时"));

        let raw3 = "\u{1b}[36mINFO\u{1b}[0m [\u{1b}[38;5;40m1600451608\u{1b}[0m 0ms] outbound/tuic[proxy]: outbound connection to www.google.com:443";
        let formatted3 = format_kernel_diagnostic(raw3);
        assert!(formatted3.contains("目标服务器无法连通") || formatted3.contains("节点连接超时"));
        assert!(!formatted3.contains("\u{1b}"));
    }
}

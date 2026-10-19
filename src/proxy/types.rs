use crate::kernel::{KernelKind, KernelPreference};
use crate::proxy::config::{DnsConfig, RoutingMode, TrafficRule, TunConfig};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn default_hop_ports() -> String {
    "20000-40000".to_string()
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoRoutingStrategy {
    #[default]
    UrlTest,
    Fallback,
    LoadBalance,
}

impl AutoRoutingStrategy {
    #[allow(dead_code)]
    pub const ALL: [Self; 3] = [Self::UrlTest, Self::Fallback, Self::LoadBalance];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::UrlTest => "⚡ 最低延迟 (Auto)",
            Self::Fallback => "🛡️ 故障转移 (Fallback)",
            Self::LoadBalance => "⚖️ 负载均衡 (LoadBalance)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::UrlTest => "周期并发探活，将流量自动调度至实时延迟最低的健康节点",
            Self::Fallback => "按健康顺位连接，当前主节点失活时立即无感重定向至下一存活节点",
            Self::LoadBalance => "多节点并发流量哈希调度，分散网络负载并大幅提升突发并发吞吐",
        }
    }
}

/// 一节点一端口：多端口独立入站映射规则
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMapping {
    pub id: String,
    pub port: u16,
    pub node_id: String,
    pub node_name: String,
    #[serde(default = "default_port_mapping_enabled")]
    pub enabled: bool,
}

fn default_port_mapping_enabled() -> bool {
    true
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
    /// 内核内自动选路：把测速前 N 名一起写进配置，交给 mihomo 的 url-test / fallback / load-balance 组按延迟
    /// 自己切换。切换就不再需要重写配置 + 热重载（那会掐断所有已建立的连接）。
    /// 仅 Mihomo 内核支持；sing-box / xray 仍按单节点生成。
    #[serde(default)]
    pub url_test: bool,
    /// 自动选路策略（最低延迟 / 故障转移 / 负载均衡）
    #[serde(default)]
    pub auto_strategy: AutoRoutingStrategy,
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
    /// 智能分流策略组体系（AI分组 / 美国节点 / 多线并发下载 / 普通代理）
    #[serde(default)]
    pub smart_routing: bool,
    /// 用户自定义配置覆写（YAML 或 脚本扩展片段）
    #[serde(default)]
    pub override_yaml: Option<String>,
    /// 一节点一端口多端口转发规则列表
    #[serde(default)]
    pub port_mappings: Vec<PortMapping>,
    /// 端口映射的目标节点对象（已转为 clash proxy 对象）：启动前注入，不落盘。
    #[serde(default, skip)]
    pub port_mapping_nodes: Vec<(u16, String, Value)>,
    /// 全局 uTLS 客户端浏览器指纹自动伪装（如 Some("chrome")、Some("safari")、None）
    #[serde(default)]
    pub client_fingerprint: Option<String>,
    /// WebRTC 真实 IP 泄漏防御（拦截 STUN / TURN 协议探测）
    #[serde(default)]
    pub block_webrtc: bool,
    /// 隐私与行为追踪广告规则集过滤
    #[serde(default)]
    pub block_trackers: bool,
    /// UDP 端口跳跃与防 QoS 丢包混淆
    #[serde(default)]
    pub udp_port_hopping: bool,
    /// UDP 端口跳跃范围（默认 20000-40000）
    #[serde(default = "default_hop_ports")]
    pub hop_ports: String,
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
            auto_strategy: AutoRoutingStrategy::UrlTest,
            url_test_size: default_url_test_size(),
            terminal_proxy: false,
            url_test_nodes: Vec::new(),
            smart_routing: false,
            override_yaml: None,
            port_mappings: Vec::new(),
            port_mapping_nodes: Vec::new(),
            client_fingerprint: None,
            block_webrtc: false,
            block_trackers: false,
            udp_port_hopping: false,
            hop_ports: default_hop_ports(),
        }
    }
}

impl MihomoConfigOptions {
    /// 判断当前配置是否会激活 GeoSite / GeoIP 分流数据库。
    /// 包含：隐私防追踪广告过滤、自定义规则、国内直连/GFW预设、或覆写片段中的 GEOSITE/GEOIP 规则。
    pub fn uses_geo(&self) -> bool {
        self.block_trackers
            || self.rules.iter().any(TrafficRule::uses_geo)
            || matches!(self.routing_mode, RoutingMode::ChinaDirect | RoutingMode::GfwList)
            || self
                .override_yaml
                .as_deref()
                .is_some_and(|y| y.contains("GEOSITE") || y.contains("GEOIP"))
    }

    pub fn validate(&self) -> Result<()> {
        if self.mixed_port == self.controller_port {
            return Err(anyhow!("代理端口和控制端口不能相同"));
        }
        let mut mapped_ports = std::collections::HashSet::new();
        for mapping in &self.port_mappings {
            if mapping.port < 1024 {
                return Err(anyhow!("映射端口 {} 低于 1024 保留端口", mapping.port));
            }
            if mapping.port == self.mixed_port {
                return Err(anyhow!("映射端口 {} 与主代理端口冲突", mapping.port));
            }
            if mapping.port == self.controller_port {
                return Err(anyhow!("映射端口 {} 与控制台端口冲突", mapping.port));
            }
            if !mapped_ports.insert(mapping.port) {
                return Err(anyhow!("存在重复的映射端口: {}", mapping.port));
            }
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

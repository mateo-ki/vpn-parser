use crate::models::Node;
use crate::proxy::{
    AutoRoutingStrategy, DnsEnhancedMode, NetworkProtocol,
    PortMapping, ProxyMode, RoutingMode, TunConfig,
};
use serde::{Deserialize, Serialize};

/// 统一路由匹配原语 (Intermediate Representation Matcher)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IRMatcher {
    Domain(String),
    DomainSuffix(String),
    DomainKeyword(String),
    DomainRegex(String),
    IpCidr(String),
    Port(u16),
    PortRange(u16, u16),
    GeoSite(String),
    GeoIp(String),
    ProcessName(String),
    Network(NetworkProtocol),
    Inbound(String),
    RuleSet(String),
    Quic,
}

/// 统一路由动作 (Intermediate Representation Action)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IRAction {
    Direct,
    /// None 表示走默认代理出口；Some(name) 表示指定出站节点名或策略组名
    Proxy(Option<String>),
    Reject,
    HijackDns,
}

/// 规则来源标识
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleSource {
    System,
    Inbound,
    Privacy,
    SmartRouting,
    Preset,
    User,
    Override,
    Fallback,
}

/// 统一单条路由规则 (Intermediate Representation Rule)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IRRule {
    pub matchers: Vec<IRMatcher>,
    pub action: IRAction,
    pub no_resolve: bool,
    pub source: RuleSource,
}

impl IRRule {
    #[allow(dead_code)]
    pub fn new(matcher: IRMatcher, action: IRAction, source: RuleSource) -> Self {
        Self {
            matchers: vec![matcher],
            action,
            no_resolve: false,
            source,
        }
    }

    #[allow(dead_code)]
    pub fn with_no_resolve(mut self, no_resolve: bool) -> Self {
        self.no_resolve = no_resolve;
        self
    }
}

/// 统一入站计划
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InboundPlan {
    pub mixed_port: u16,
    pub controller_port: u16,
    pub controller_secret: String,
    pub allow_lan: bool,
    pub mode: ProxyMode,
    pub tun: TunConfig,
    pub port_mappings: Vec<PortMapping>,
}

/// 统一出站计划
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboundPlan {
    pub primary_node: Option<Node>,
    pub url_test: bool,
    pub auto_strategy: AutoRoutingStrategy,
    pub url_test_size: u16,
    pub url_test_nodes: Vec<serde_json::Value>,
    pub smart_routing: bool,
    pub override_yaml: Option<String>,
    pub subscription_rules_yaml: Option<String>,
    pub subscription_groups_yaml: Option<String>,
    pub port_mapping_nodes: Vec<(u16, String, serde_json::Value)>,
    pub client_fingerprint: Option<String>,
    pub udp_port_hopping: bool,
    pub hop_ports: String,
}

/// 统一 DNS 分流计划
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DnsPlan {
    pub enhanced_mode: DnsEnhancedMode,
    pub nameservers: Vec<String>,
    pub default_nameservers: Vec<String>,
    pub china_nameservers: Vec<String>,
    pub overseas_nameservers: Vec<String>,
    pub proxy_server_nameservers: Vec<String>,
    pub fake_ip_range: String,
    pub fake_ip_filter: Vec<String>,
    pub respect_rules: bool,
}

/// 统一路由编译上下文 (Routing Plan)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingPlan {
    pub inbounds: InboundPlan,
    pub outbounds: OutboundPlan,
    pub dns: DnsPlan,
    pub rules: Vec<IRRule>,
    pub default_action: IRAction,
    #[serde(default)]
    pub routing_mode: RoutingMode,
}

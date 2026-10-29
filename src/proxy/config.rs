use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// 默认 DNS 走 DoH：明文 UDP（8.8.8.8/114）在境内会被透明拦截，属于 DNS 泄露。
pub const DEFAULT_DNS_PRIMARY: &str = "https://doh.pub/dns-query";
pub const DEFAULT_DNS_SECONDARY: &str = "https://dns.alidns.com/dns-query";
/// 旧版本的明文默认值；保存过的配置命中它们时迁移到上面的 DoH。
const LEGACY_DEFAULT_PLAINTEXT: &[&str] = &["8.8.8.8", "114.114.114.114"];
const LEGACY_DEFAULT_DOH: &str = "https://1.1.1.1/dns-query";
const PREVIOUS_DEFAULT_DOH: &str = "https://223.5.5.5/dns-query";
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    #[default]
    Rule,
    ChinaDirect,
    GfwList,
    Global,
    Direct,
    Subscription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunStack {
    System,
    Gvisor,
    Mixed,
}

impl TunStack {
    pub fn as_mihomo_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Gvisor => "gvisor",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunConfig {
    pub stack: TunStack,
    pub dns_hijack: Vec<String>,
    pub auto_route: bool,
    pub auto_detect_interface: bool,
}

impl Default for TunConfig {
    fn default() -> Self {
        Self {
            stack: TunStack::Mixed,
            dns_hijack: vec!["any:53".into()],
            auto_route: true,
            auto_detect_interface: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsEnhancedMode {
    FakeIp,
    RedirHost,
}

impl DnsEnhancedMode {
    pub fn as_mihomo_value(self) -> &'static str {
        match self {
            Self::FakeIp => "fake-ip",
            Self::RedirHost => "redir-host",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnsConfig {
    pub enabled: bool,
    pub ipv6: bool,
    pub enhanced_mode: DnsEnhancedMode,
    pub nameservers: Vec<String>,
    /// 解析代理节点自身域名所用 DNS。节点域名如果走 ISP 明文解析会暴露"在使用
    /// 代理"，mihomo 用这个独立列表解析它们（默认与主 DNS 同为 DoH）。
    #[serde(default = "default_proxy_server_nameserver")]
    pub proxy_server_nameserver: Vec<String>,
}

fn default_proxy_server_nameserver() -> Vec<String> {
    vec![
        "223.5.5.5".into(),
        "119.29.29.29".into(),
        DEFAULT_DNS_PRIMARY.into(),
        DEFAULT_DNS_SECONDARY.into(),
    ]
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ipv6: false,
            enhanced_mode: DnsEnhancedMode::FakeIp,
            nameservers: vec![DEFAULT_DNS_PRIMARY.into(), DEFAULT_DNS_SECONDARY.into()],
            proxy_server_nameserver: default_proxy_server_nameserver(),
        }
    }
}

impl DnsConfig {
    pub fn migrate_legacy_defaults(&mut self) -> bool {
        let mut migrated = false;
        let legacy_sets: [&[&str]; 3] = [
            LEGACY_DEFAULT_PLAINTEXT,
            &[LEGACY_DEFAULT_DOH],
            &[PREVIOUS_DEFAULT_DOH],
        ];
        if legacy_sets
            .iter()
            .any(|legacy| self.nameservers.as_slice() == *legacy)
        {
            self.nameservers = vec![DEFAULT_DNS_PRIMARY.into(), DEFAULT_DNS_SECONDARY.into()];
            migrated = true;
        }
        if self.proxy_server_nameserver.is_empty()
            || self.proxy_server_nameserver == [DEFAULT_DNS_PRIMARY, DEFAULT_DNS_SECONDARY]
        {
            self.proxy_server_nameserver = default_proxy_server_nameserver();
            migrated = true;
        }
        migrated
    }

    pub fn validate(&self) -> Result<()> {
        if self.enabled && self.nameservers.is_empty() {
            return Err(anyhow!("启用 DNS 时至少需要一个 DNS 地址"));
        }
        if let Some(server) = self.nameservers.iter().find(|server| {
            server.parse::<std::net::IpAddr>().is_err() && !server.starts_with("https://")
        }) {
            return Err(anyhow!("DNS 地址必须是 IP 或 HTTPS URL: {server}"));
        }
        if let Some(server) = self.proxy_server_nameserver.iter().find(|server| {
            server.parse::<std::net::IpAddr>().is_err() && !server.starts_with("https://")
        }) {
            return Err(anyhow!(
                "proxy-server-nameserver 地址必须是 IP 或 HTTPS URL: {server}"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleAction {
    Direct,
    Proxy,
    Reject,
}

impl RuleAction {
    pub fn as_target(self) -> &'static str {
        match self {
            Self::Direct => "DIRECT",
            Self::Proxy => "PROXY",
            Self::Reject => "REJECT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    Tcp,
    Udp,
}

impl NetworkProtocol {
    fn as_mihomo_value(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuleMatcher {
    Domain(String),
    DomainSuffix(String),
    DomainKeyword(String),
    GeoSite(String),
    GeoIp(String),
    IpCidr(String),
    DestinationPort(String),
    ProcessName(String),
    RuleSet(String),
    Network(NetworkProtocol),
    Quic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrafficRule {
    pub matcher: RuleMatcher,
    pub action: RuleAction,
}

/// 保留用户规则顺序，只移除完全重复项，避免多个内核重复匹配同一条规则。
pub fn optimize_traffic_rules(rules: &[TrafficRule]) -> Vec<TrafficRule> {
    if rules.len() > 600 {
        // 大规模规则使用快速去重，杜绝 O(N^2) 导致 UI 冻结
        let mut seen = std::collections::HashSet::with_capacity(rules.len());
        let mut out = Vec::with_capacity(rules.len());
        for rule in rules {
            let key = (&rule.matcher, &rule.action);
            if seen.insert(key) {
                out.push(rule.clone());
            }
        }
        return out;
    }
    let mut optimized: Vec<TrafficRule> = Vec::with_capacity(rules.len());
    for rule in rules {
        // 完全重复，或被前面同动作的规则覆盖（域名后缀包含、或已被某个 GEOSITE 分类
        // 的父域覆盖）的规则，匹配不到任何东西，只会让规则表越来越长。
        if optimized
            .iter()
            .any(|existing| shadows(existing, rule) && existing.action == rule.action)
        {
            continue;
        }
        optimized.push(rule.clone());
    }
    optimized
}

#[inline]
fn is_subdomain(child: &str, parent: &str) -> bool {
    child == parent
        || (child.ends_with(parent)
            && child.len() > parent.len()
            && child.as_bytes()[child.len() - parent.len() - 1] == b'.')
}

/// `earlier` 是否已经覆盖 `later`（mihomo 从上到下匹配，先命中的规则生效）。
fn shadows(earlier: &TrafficRule, later: &TrafficRule) -> bool {
    use RuleMatcher::*;
    match (&earlier.matcher, &later.matcher) {
        (Domain(a), Domain(b))
        | (DomainKeyword(a), DomainKeyword(b))
        | (GeoSite(a), GeoSite(b))
        | (IpCidr(a), IpCidr(b)) => a == b,
        (DomainSuffix(a), DomainSuffix(b)) => is_subdomain(b, a),
        (DomainSuffix(a), Domain(b)) => is_subdomain(b, a),
        (DomainKeyword(a), Domain(b)) | (DomainKeyword(a), DomainSuffix(b)) => b.contains(a),
        (GeoIp(a), GeoIp(b)) => a.eq_ignore_ascii_case(b),
        (DestinationPort(a), DestinationPort(b)) => a == b,
        (ProcessName(a), ProcessName(b)) => a == b,
        (RuleSet(a), RuleSet(b)) => a == b,
        (Quic, Quic) => true,
        (Network(a), Network(b)) => a == b,
        _ => false,
    }
}

impl TrafficRule {
    pub fn to_mihomo_rule(&self) -> Result<String> {
        let target = self.action.as_target();
        let rule = match &self.matcher {
            RuleMatcher::Domain(value) => render_value_rule("DOMAIN", value, target)?,
            RuleMatcher::DomainSuffix(value) => render_value_rule("DOMAIN-SUFFIX", value, target)?,
            RuleMatcher::DomainKeyword(value) => render_value_rule("DOMAIN-KEYWORD", value, target)?,
            RuleMatcher::GeoSite(value) => render_value_rule("GEOSITE", value, target)?,
            RuleMatcher::GeoIp(value) => {
                let rendered = render_value_rule("GEOIP", value, target)?;
                // no-resolve：域名连接不参与 GEOIP 判定。否则 fake-ip 下每条没被前面
                // 规则命中的域名都要先真解析一次，而境内 DoH 常把境外域名污染成国内
                // IP（实测 googlevideo 解析到上海/北京），于是被误判成"国内直连"。
                format!("{rendered},no-resolve")
            }
            RuleMatcher::IpCidr(value) => render_value_rule("IP-CIDR", value, target)?,
            RuleMatcher::DestinationPort(value) => render_value_rule("DST-PORT", value, target)?,
            RuleMatcher::ProcessName(value) => render_value_rule("PROCESS-NAME", value, target)?,
            RuleMatcher::RuleSet(value) => render_value_rule("RULE-SET", value, target)?,
            RuleMatcher::Network(protocol) => {
                format!("NETWORK,{},{}", protocol.as_mihomo_value(), target)
            }
            RuleMatcher::Quic => {
                format!("AND,((NETWORK,udp),(DST-PORT,443)),{target}")
            }
        };
        Ok(rule)
    }

    pub fn uses_geo(&self) -> bool {
        matches!(
            self.matcher,
            RuleMatcher::GeoSite(_) | RuleMatcher::GeoIp(_)
        )
    }
}

/// 仅走 Mihomo 编译管线的内核（Mihomo / VP-Core / Meow）使用的域名优先规则：
/// fake-ip 模式下 `GEOIP,CN` 会拿真实解析结果参与判定，而境内 DoH 间歇性把
/// google/youtube 的 CDN 域名污染成国内 IP（实测 googlevideo 解析到上海/北京），
/// 于是被判成"国内直连"再 i/o timeout。插在第一条 GEOIP,CN 之前按域名定性，
/// 就不需要依赖任何境外 DNS。VP-Core 与 Meow 的编译器复用 Mihomo 编译器作为基础，
/// 注入随管线自动生效（vp-core 原生支持 GEOSITE 与 geosite.dat 自动下载）。
/// sing-box 侧没有对应分类的 rule-set，不注入。
pub fn mihomo_dns_poison_overrides() -> Vec<TrafficRule> {
    ["google", "youtube"]
        .into_iter()
        .map(|category| TrafficRule {
            matcher: RuleMatcher::GeoSite(category.into()),
            action: RuleAction::Proxy,
        })
        .collect()
}

/// 把污染免疫规则插到第一条 `GEOIP,CN` 之前；规则表里没有 GEOIP,CN 时无需注入。
pub fn apply_mihomo_dns_poison_overrides(rules: &mut Vec<TrafficRule>) {
    let Some(position) = rules.iter().position(|rule| {
        matches!(
            &rule.matcher,
            RuleMatcher::GeoIp(value) if value.eq_ignore_ascii_case("CN")
        )
    }) else {
        return;
    };
    let already = rules[..position]
        .iter()
        .any(|rule| matches!(&rule.matcher, RuleMatcher::GeoSite(value) if value == "youtube"));
    if already {
        return;
    }
    rules.splice(position..position, mihomo_dns_poison_overrides());
}

pub fn recommended_traffic_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::Domain("localhost".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("lan".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("home.arpa".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("127.0.0.0/8".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("10.0.0.0/8".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("172.16.0.0/12".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("192.168.0.0/16".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("169.254.0.0/16".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("100.64.0.0/10".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("192.0.0.0/24".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("198.18.0.0/15".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::GeoSite("category-ads-all".into()),
            action: RuleAction::Reject,
        },
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
    ]
}

pub fn gfwlist_traffic_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::GeoSite("category-ads-all".into()),
            action: RuleAction::Reject,
        },
        TrafficRule {
            matcher: RuleMatcher::GeoSite("gfw".into()),
            action: RuleAction::Proxy,
        },
    ]
}

pub fn ai_service_rules() -> Vec<TrafficRule> {
    vec![
        // 实测这份 GeoSite.dat 里有 openai / anthropic 分类（没有 chatgpt/claude/gemini/
        // copilot），用分类比手写域名覆盖更全、也更少。
        TrafficRule {
            matcher: RuleMatcher::GeoSite("openai".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::GeoSite("anthropic".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("copilot.microsoft.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("ai.google.dev".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("gemini.google.com".into()),
            action: RuleAction::Proxy,
        },
    ]
}

pub fn streaming_media_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("youtube.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("googlevideo.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("ytimg.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("netflix.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("nflxvideo.net".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("disneyplus.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("spotify.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("twitch.tv".into()),
            action: RuleAction::Proxy,
        },
    ]
}

pub fn social_telegram_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("telegram.org".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("t.me".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("telegra.ph".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("91.108.4.0/22".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("91.108.56.0/22".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::IpCidr("149.154.160.0/20".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("discord.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("discordapp.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("twitter.com".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("x.com".into()),
            action: RuleAction::Proxy,
        },
    ]
}

pub fn dev_mirrors_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("npmmirror.com".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("cnpmjs.org".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("tsinghua.edu.cn".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("ustc.edu.cn".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("aliyun.com".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("gitee.com".into()),
            action: RuleAction::Direct,
        },
    ]
}

pub fn game_platforms_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("steamcontent.com".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("steambroadcast.akamaized.net".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("epicgames.com".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::DomainSuffix("battlenet.com.cn".into()),
            action: RuleAction::Direct,
        },
    ]
}

pub fn adblock_quic_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::GeoSite("category-ads-all".into()),
            action: RuleAction::Reject,
        },
        TrafficRule {
            matcher: RuleMatcher::Quic,
            action: RuleAction::Reject,
        },
    ]
}

fn render_value_rule(kind: &str, value: &str, target: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("{kind} 规则值不能为空"));
    }
    if value.contains(',') || value.contains('\n') || value.contains('\r') {
        return Err(anyhow!("{kind} 规则值包含非法分隔符"));
    }
    Ok(format!("{kind},{value},{target}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tun_defaults_enable_route_and_dns_hijack() {
        let tun = TunConfig::default();

        assert_eq!(tun.stack, TunStack::Mixed);
        assert!(tun.auto_route);
        assert!(tun.auto_detect_interface);
        assert_eq!(tun.dns_hijack, vec!["any:53"]);
    }

    #[test]
    fn dns_defaults_use_doh_with_fake_ip() {
        let dns = DnsConfig::default();

        assert!(dns.enabled);
        assert_eq!(dns.enhanced_mode, DnsEnhancedMode::FakeIp);
        assert_eq!(
            dns.nameservers,
            vec![
                "https://doh.pub/dns-query",
                "https://dns.alidns.com/dns-query"
            ]
        );
        assert!(!dns.proxy_server_nameserver.is_empty());
        assert!(dns.validate().is_ok());
    }

    #[test]
    fn dns_migrates_legacy_cloudflare_default() {
        let mut dns = DnsConfig {
            nameservers: vec!["https://1.1.1.1/dns-query".into()],
            ..DnsConfig::default()
        };

        assert!(dns.migrate_legacy_defaults());
        assert_eq!(
            dns.nameservers,
            vec![
                "https://doh.pub/dns-query",
                "https://dns.alidns.com/dns-query"
            ]
        );
    }

    #[test]
    fn dns_migrates_legacy_plaintext_default() {
        // 旧默认 8.8.8.8/114 明文 UDP 属于 DNS 泄露，保存过的配置要自动迁移。
        let mut dns = DnsConfig {
            nameservers: vec!["8.8.8.8".into(), "114.114.114.114".into()],
            proxy_server_nameserver: Vec::new(),
            ..DnsConfig::default()
        };

        assert!(dns.migrate_legacy_defaults());
        assert_eq!(
            dns.nameservers,
            vec![
                "https://doh.pub/dns-query",
                "https://dns.alidns.com/dns-query"
            ]
        );
        assert!(!dns.proxy_server_nameserver.is_empty());
    }

    #[test]
    fn dns_accepts_ip_nameservers() {
        let dns = DnsConfig {
            nameservers: vec!["8.8.8.8".into(), "114.114.114.114".into()],
            ..DnsConfig::default()
        };

        assert!(dns.validate().is_ok());
    }

    #[test]
    fn dns_rejects_invalid_nameserver() {
        let dns = DnsConfig {
            nameservers: vec!["not-a-dns-server".into()],
            ..DnsConfig::default()
        };

        assert!(dns
            .validate()
            .unwrap_err()
            .to_string()
            .contains("IP 或 HTTPS"));
    }

    #[test]
    fn traffic_rules_render_all_supported_matchers() {
        let cases = [
            (
                RuleMatcher::Domain("example.com".into()),
                "DOMAIN,example.com,PROXY",
            ),
            (
                RuleMatcher::DomainSuffix("example.com".into()),
                "DOMAIN-SUFFIX,example.com,PROXY",
            ),
            (RuleMatcher::GeoSite("cn".into()), "GEOSITE,cn,PROXY"),
            (RuleMatcher::GeoIp("CN".into()), "GEOIP,CN,PROXY,no-resolve"),
            (
                RuleMatcher::IpCidr("10.0.0.0/8".into()),
                "IP-CIDR,10.0.0.0/8,PROXY",
            ),
            (
                RuleMatcher::DestinationPort("443".into()),
                "DST-PORT,443,PROXY",
            ),
            (
                RuleMatcher::ProcessName("curl.exe".into()),
                "PROCESS-NAME,curl.exe,PROXY",
            ),
            (
                RuleMatcher::Network(NetworkProtocol::Tcp),
                "NETWORK,tcp,PROXY",
            ),
            (
                RuleMatcher::Quic,
                "AND,((NETWORK,udp),(DST-PORT,443)),PROXY",
            ),
        ];

        for (matcher, expected) in cases {
            let rule = TrafficRule {
                matcher,
                action: RuleAction::Proxy,
            };
            assert_eq!(rule.to_mihomo_rule().unwrap(), expected);
        }
    }

    #[test]
    fn rule_actions_map_to_mihomo_targets() {
        assert_eq!(RuleAction::Direct.as_target(), "DIRECT");
        assert_eq!(RuleAction::Proxy.as_target(), "PROXY");
        assert_eq!(RuleAction::Reject.as_target(), "REJECT");
    }

    #[test]
    fn recommended_rules_cover_lan_ads_cn_and_quic_in_order() {
        let rules = recommended_traffic_rules()
            .iter()
            .map(TrafficRule::to_mihomo_rule)
            .collect::<Result<Vec<_>>>()
            .unwrap();

        assert_eq!(
            rules,
            vec![
                "DOMAIN,localhost,DIRECT",
                "DOMAIN-SUFFIX,lan,DIRECT",
                "DOMAIN-SUFFIX,home.arpa,DIRECT",
                "IP-CIDR,127.0.0.0/8,DIRECT",
                "IP-CIDR,10.0.0.0/8,DIRECT",
                "IP-CIDR,172.16.0.0/12,DIRECT",
                "IP-CIDR,192.168.0.0/16,DIRECT",
                "IP-CIDR,169.254.0.0/16,DIRECT",
                "IP-CIDR,100.64.0.0/10,DIRECT",
                "IP-CIDR,192.0.0.0/24,DIRECT",
                "IP-CIDR,198.18.0.0/15,DIRECT",
                "GEOSITE,category-ads-all,REJECT",
                "GEOSITE,cn,DIRECT",
                "GEOIP,CN,DIRECT,no-resolve",
                "AND,((NETWORK,udp),(DST-PORT,443)),REJECT",
            ]
        );
    }

    #[test]
    fn gfwlist_rules_proxy_matches_and_reject_ads() {
        let rules = gfwlist_traffic_rules()
            .iter()
            .map(TrafficRule::to_mihomo_rule)
            .collect::<Result<Vec<_>>>()
            .unwrap();

        assert_eq!(
            rules,
            vec!["GEOSITE,category-ads-all,REJECT", "GEOSITE,gfw,PROXY"]
        );
    }

    #[test]
    fn optimize_rules_removes_duplicates_and_preserves_order() {
        let first = TrafficRule {
            matcher: RuleMatcher::DomainSuffix("example.com".into()),
            action: RuleAction::Proxy,
        };
        let second = TrafficRule {
            matcher: RuleMatcher::Domain("localhost".into()),
            action: RuleAction::Direct,
        };
        let optimized = optimize_traffic_rules(&[first.clone(), second.clone(), first.clone()]);
        assert_eq!(optimized, vec![first, second]);
    }

    #[test]
    fn routing_mode_defaults_when_old_config_has_no_field() {
        #[derive(Deserialize)]
        struct LegacyCompatible {
            #[serde(default)]
            routing_mode: RoutingMode,
        }

        let value: LegacyCompatible = serde_json::from_str("{}").unwrap();
        assert_eq!(value.routing_mode, RoutingMode::Rule);
    }
}

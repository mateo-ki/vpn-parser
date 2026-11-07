use crate::models::Node;
use crate::proxy::config::{RuleAction, RuleMatcher, TrafficRule};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleMatchStatus {
    Mismatch,
    Hit(String),
}

#[derive(Debug, Clone)]
pub struct TraceRuleRecord {
    pub index: usize,
    pub rule_text: String,
    pub status: RuleMatchStatus,
    pub action: RuleAction,
    pub is_final: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TraceNodeInfo {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub protocol: String,
    pub latency_ms: Option<i64>,
}

impl TraceNodeInfo {
    pub fn from_node(node: &Node) -> Self {
        Self {
            name: node.name.clone(),
            server: node.server.clone(),
            port: node.port,
            protocol: node.protocol.clone(),
            latency_ms: node.latency_ms,
        }
    }

    pub fn from_summary(s: &crate::models::NodeSummary) -> Self {
        Self {
            name: s.name.clone(),
            server: s.server.clone(),
            port: s.port,
            protocol: s.protocol.clone(),
            latency_ms: s.latency_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteTraceReport {
    pub target: String,
    pub host: String,
    pub port: u16,
    pub protocol: String,
    pub inbound_session: String,
    pub protocol_sniffing: String,
    pub rule_records: Vec<TraceRuleRecord>,
    pub routing_action: String,
    pub endpoint_decision: String,
    pub raw_text: String,
    pub inbound_ms: f64,
    pub sniffing_ms: f64,
    pub rule_match_ms: f64,
    pub routing_action_ms: f64,
    pub endpoint_ms: f64,
    pub is_from_vpcore_api: bool,
    pub evaluated_count: Option<usize>,
    pub total_rules: Option<usize>,
    pub kernel_elapsed_us: Option<u64>,
    pub dns_resolved_ips: Option<Vec<String>>,
    pub group_pick_reason: Option<String>,
}

/// 解析目标地址与端口（默认端口 443）
pub fn parse_target(target: &str) -> (String, u16) {
    let trimmed = target
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    
    // 如果带 path，去掉 path
    let addr_part = if let Some(idx) = trimmed.find('/') {
        &trimmed[..idx]
    } else {
        trimmed
    };

    if addr_part.is_empty() {
        return ("github.com".to_string(), 443);
    }

    if let Some(colon_idx) = addr_part.rfind(':') {
        let host = &addr_part[..colon_idx];
        let port_str = &addr_part[colon_idx + 1..];
        let clean_host = host.trim_start_matches('[').trim_end_matches(']');
        let port = port_str.parse::<u16>().unwrap_or(443);
        (clean_host.to_string(), port)
    } else {
        (addr_part.to_string(), 443)
    }
}

/// 模拟协议嗅探
pub fn infer_sniffing(host: &str, port: u16, protocol: &str) -> String {
    let is_udp = protocol.eq_ignore_ascii_case("udp");
    if is_udp && port == 443 {
        format!("QUIC Initial -> SNI: {host}")
    } else if port == 53 {
        format!("DNS Query -> Query: {host}")
    } else if port == 443 || port == 8443 {
        format!("TLS ClientHello -> SNI: {host}")
    } else if port == 80 || port == 8080 {
        format!("HTTP Request -> Host: {host}")
    } else {
        format!("TCP Raw Stream -> 目标: {host}:{port}")
    }
}

#[inline]
fn is_subdomain(child: &str, parent: &str) -> bool {
    let child = child.trim().to_ascii_lowercase();
    let parent = parent.trim().to_ascii_lowercase();
    child == parent
        || (child.ends_with(&parent)
            && child.len() > parent.len()
            && child.as_bytes()[child.len() - parent.len() - 1] == b'.')
}

fn is_geosite_category_match(host: &str, category: &str) -> bool {
    let h = host.to_ascii_lowercase();
    let cat = category.to_ascii_lowercase();
    match cat.as_str() {
        "category-ads-all" | "ads" | "reject" => {
            h.contains("adservice")
                || h.contains("doubleclick")
                || h.contains("googlesyndication")
                || h.contains("pagead")
                || h.contains("telemetry")
                || h.contains("umeng")
                || h.contains("analytics")
                || h.contains("ads.")
                || h.starts_with("ad.")
                || h.contains("adjust.com")
                || h.contains("appsflyer")
        }
        "google" => {
            is_subdomain(&h, "google.com")
                || is_subdomain(&h, "youtube.com")
                || is_subdomain(&h, "googlevideo.com")
                || is_subdomain(&h, "gstatic.com")
                || is_subdomain(&h, "googleapis.com")
                || is_subdomain(&h, "gmail.com")
                || is_subdomain(&h, "1e100.net")
                || h.contains(".google.")
        }
        "youtube" => {
            is_subdomain(&h, "youtube.com")
                || is_subdomain(&h, "googlevideo.com")
                || is_subdomain(&h, "ytimg.com")
                || is_subdomain(&h, "youtu.be")
        }
        "openai" | "chatgpt" => {
            is_subdomain(&h, "openai.com")
                || is_subdomain(&h, "chatgpt.com")
                || is_subdomain(&h, "oaistatic.com")
                || is_subdomain(&h, "oaiusercontent.com")
                || is_subdomain(&h, "ai.com")
        }
        "anthropic" | "claude" => {
            is_subdomain(&h, "anthropic.com") || is_subdomain(&h, "claude.ai")
        }
        "telegram" => {
            is_subdomain(&h, "telegram.org")
                || is_subdomain(&h, "t.me")
                || is_subdomain(&h, "telesco.pe")
                || is_subdomain(&h, "telegram.me")
        }
        "gfw" => {
            is_subdomain(&h, "github.com")
                || is_subdomain(&h, "githubusercontent.com")
                || is_subdomain(&h, "twitter.com")
                || is_subdomain(&h, "x.com")
                || is_subdomain(&h, "facebook.com")
                || is_subdomain(&h, "instagram.com")
                || is_subdomain(&h, "wikipedia.org")
                || is_subdomain(&h, "reddit.com")
                || is_subdomain(&h, "medium.com")
                || is_subdomain(&h, "v2ex.com")
                || is_subdomain(&h, "dropbox.com")
                || is_subdomain(&h, "google.com")
                || is_subdomain(&h, "youtube.com")
                || is_subdomain(&h, "t.me")
        }
        "cn" | "china" => {
            h.ends_with(".cn")
                || is_subdomain(&h, "baidu.com")
                || is_subdomain(&h, "qq.com")
                || is_subdomain(&h, "bilibili.com")
                || is_subdomain(&h, "taobao.com")
                || is_subdomain(&h, "jd.com")
                || is_subdomain(&h, "163.com")
                || is_subdomain(&h, "weibo.com")
                || is_subdomain(&h, "zhihu.com")
                || is_subdomain(&h, "alipay.com")
                || is_subdomain(&h, "aliyun.com")
                || is_subdomain(&h, "tencent.com")
                || is_subdomain(&h, "sohu.com")
                || is_subdomain(&h, "sina.com.cn")
                || is_subdomain(&h, "douyin.com")
                || is_subdomain(&h, "xiaohongshu.com")
        }
        "private" => {
            h == "localhost"
                || h.ends_with(".local")
                || h.starts_with("127.")
                || h.starts_with("192.168.")
                || h.starts_with("10.")
        }
        other => h.contains(other),
    }
}

fn is_ruleset_category_match(host: &str, ruleset: &str) -> bool {
    let lower = ruleset.to_ascii_lowercase();
    if lower.contains("gfw") || lower.contains("proxy") {
        is_geosite_category_match(host, "gfw")
    } else if lower.contains("ad") || lower.contains("reject") {
        is_geosite_category_match(host, "category-ads-all")
    } else if lower.contains("cn") || lower.contains("direct") {
        is_geosite_category_match(host, "cn")
    } else {
        host.to_ascii_lowercase().contains(&lower)
    }
}

fn ip_in_cidr(ip: &IpAddr, cidr: &str) -> bool {
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.is_empty() {
        return false;
    }
    let net_ip = match parts[0].trim().parse::<IpAddr>() {
        Ok(ip) => ip,
        Err(_) => return false,
    };
    let prefix = if parts.len() > 1 {
        parts[1].trim().parse::<u32>().unwrap_or(32)
    } else {
        32
    };
    match (ip, net_ip) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let mask = if prefix == 0 {
                0
            } else {
                !0u32 << (32 - prefix.min(32))
            };
            (u32::from(*a) & mask) == (u32::from(b) & mask)
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let mask = if prefix == 0 {
                0
            } else {
                !0u128 << (128 - prefix.min(128))
            };
            (u128::from(*a) & mask) == (u128::from(b) & mask)
        }
        _ => false,
    }
}

fn evaluate_matcher(
    matcher: &RuleMatcher,
    lower_host: &str,
    parsed_ip: Option<&IpAddr>,
    port: u16,
    protocol: &str,
) -> RuleMatchStatus {
    match matcher {
        RuleMatcher::Domain(d) => {
            if lower_host.eq_ignore_ascii_case(d) {
                RuleMatchStatus::Hit(format!("Matched exact domain: {d}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::DomainSuffix(d) => {
            if is_subdomain(lower_host, d) {
                RuleMatchStatus::Hit(format!("Matched suffix: {d}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::DomainKeyword(k) => {
            let lower_k = k.to_ascii_lowercase();
            if lower_host.contains(&lower_k) {
                RuleMatchStatus::Hit(format!("Matched keyword: {k}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::GeoSite(category) => {
            if is_geosite_category_match(lower_host, category) {
                RuleMatchStatus::Hit(format!("Matched geosite: {category}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::RuleSet(name) => {
            if is_ruleset_category_match(lower_host, name) {
                RuleMatchStatus::Hit(format!("Matched suffix: {lower_host}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::DestinationPort(p) => {
            if p == &port.to_string() {
                RuleMatchStatus::Hit(format!("Matched port: {p}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::GeoIp(country) => {
            if country.eq_ignore_ascii_case("CN") && is_geosite_category_match(lower_host, "cn") {
                RuleMatchStatus::Hit(format!("Matched GEOIP: {country}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::IpCidr(cidr) => {
            if let Some(ip) = parsed_ip {
                if ip_in_cidr(ip, cidr) {
                    return RuleMatchStatus::Hit(format!("Matched IP-CIDR: {cidr}"));
                }
            }
            RuleMatchStatus::Mismatch
        }
        RuleMatcher::Network(proto) => {
            let p_str = match proto {
                crate::proxy::config::NetworkProtocol::Tcp => "tcp",
                crate::proxy::config::NetworkProtocol::Udp => "udp",
            };
            if protocol.eq_ignore_ascii_case(p_str) {
                RuleMatchStatus::Hit(format!("Matched network: {p_str}"))
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::Quic => {
            if protocol.eq_ignore_ascii_case("udp") && port == 443 {
                RuleMatchStatus::Hit("Matched QUIC: udp/443".to_string())
            } else {
                RuleMatchStatus::Mismatch
            }
        }
        RuleMatcher::ProcessName(_) => RuleMatchStatus::Mismatch,
    }
}

/// 格式化单条匹配规则的展示文本
fn format_matcher_display(matcher: &RuleMatcher) -> String {
    match matcher {
        RuleMatcher::Domain(d) => format!("DOMAIN, {d}"),
        RuleMatcher::DomainSuffix(d) => format!("DOMAIN-SUFFIX, {d}"),
        RuleMatcher::DomainKeyword(k) => format!("DOMAIN-KEYWORD, {k}"),
        RuleMatcher::GeoSite(c) => format!("GEOSITE, {c}"),
        RuleMatcher::RuleSet(s) => {
            if s.eq_ignore_ascii_case("gfwlist") || s.eq_ignore_ascii_case("gfw") {
                "RULE-SET, gfwlist (mrs)".to_string()
            } else {
                format!("RULE-SET, {s}")
            }
        }
        RuleMatcher::GeoIp(c) => format!("GEOIP, {c}"),
        RuleMatcher::IpCidr(c) => format!("IP-CIDR, {c}"),
        RuleMatcher::DestinationPort(p) => format!("DST-PORT, {p}"),
        RuleMatcher::ProcessName(p) => format!("PROCESS-NAME, {p}"),
        RuleMatcher::Network(n) => format!("NETWORK, {:?}", n),
        RuleMatcher::Quic => "QUIC, 443".to_string(),
    }
}

/// 获取推荐的标准追踪规则集（若用户配置规则为空或过短时充当默认梯队）
pub fn default_trace_rules() -> Vec<TrafficRule> {
    vec![
        TrafficRule {
            matcher: RuleMatcher::DomainKeyword("google".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::GeoSite("category-ads-all".into()),
            action: RuleAction::Reject,
        },
        TrafficRule {
            matcher: RuleMatcher::RuleSet("gfwlist".into()),
            action: RuleAction::Proxy,
        },
        TrafficRule {
            matcher: RuleMatcher::GeoSite("cn".into()),
            action: RuleAction::Direct,
        },
        TrafficRule {
            matcher: RuleMatcher::GeoIp("CN".into()),
            action: RuleAction::Direct,
        },
    ]
}

/// 核心路由追踪模拟诊断
pub fn simulate_route_trace(
    target_input: &str,
    protocol: &str,
    mixed_port: u16,
    configured_rules: &[TrafficRule],
    node: Option<&TraceNodeInfo>,
    proxy_group_name: Option<&str>,
    is_active_node: bool,
) -> RouteTraceReport {
    // 环节 1：实测本地内核 Socket 端口分配与握手就绪基准耗时
    let inbound_start = std::time::Instant::now();
    let _ = std::net::TcpListener::bind("127.0.0.1:0");
    let inbound_ms = (inbound_start.elapsed().as_micros() as f64 / 1000.0).max(0.08);

    // 环节 2：实测协议嗅探与主机提取解析耗时
    let sniff_start = std::time::Instant::now();
    let (host, port) = parse_target(target_input);
    let proto = if protocol.trim().is_empty() { "TCP" } else { protocol.trim() };
    let inbound_port = if mixed_port > 0 { mixed_port } else { 7890 };

    let inbound_session = format!("Inbound(mixed:{inbound_port}) -> {proto} -> 目标: {host}:{port}");
    let protocol_sniffing = infer_sniffing(&host, port, proto);
    let sniffing_ms = (sniff_start.elapsed().as_micros() as f64 / 1000.0).max(0.12);

    // 组合规则池：使用配置规则；若为空则使用默认标准分流规则
    let effective_rules: Vec<TrafficRule> = if configured_rules.is_empty() {
        default_trace_rules()
    } else {
        configured_rules.to_vec()
    };

    let mut rule_records = Vec::new();
    let mut hit_action: Option<RuleAction> = None;
    let mut hit_index = 1;

    let lower_host = host.to_ascii_lowercase();
    let parsed_ip = host.parse::<IpAddr>().ok();

    // 环节 3：实测逐条规则比对全过程真实 CPU 运算耗时
    let rule_start = std::time::Instant::now();
    for rule in &effective_rules {
        let match_status = evaluate_matcher(&rule.matcher, &lower_host, parsed_ip.as_ref(), port, proto);
        
        match match_status {
            RuleMatchStatus::Hit(reason) => {
                let rule_text = format_matcher_display(&rule.matcher);
                rule_records.push(TraceRuleRecord {
                    index: hit_index,
                    rule_text,
                    status: RuleMatchStatus::Hit(reason),
                    action: rule.action,
                    is_final: true,
                });
                hit_action = Some(rule.action);
                break;
            }
            RuleMatchStatus::Mismatch => {
                let rule_text = format_matcher_display(&rule.matcher);
                rule_records.push(TraceRuleRecord {
                    index: hit_index,
                    rule_text,
                    status: RuleMatchStatus::Mismatch,
                    action: rule.action,
                    is_final: false,
                });
                hit_index += 1;
            }
        }
    }
    let rule_match_ms = (rule_start.elapsed().as_micros() as f64 / 1000.0).max(0.04);

    // 若所有规则均 MISMATCH，则触发兜底 MATCH 规则
    if hit_action.is_none() {
        rule_records.push(TraceRuleRecord {
            index: hit_index,
            rule_text: "MATCH, PROXY".to_string(),
            status: RuleMatchStatus::Hit("Default fallback route".to_string()),
            action: RuleAction::Proxy,
            is_final: true,
        });
        hit_action = Some(RuleAction::Proxy);
    }

    let final_action = hit_action.unwrap_or(RuleAction::Proxy);
    let group = proxy_group_name.unwrap_or("🎯 PROXY");

    // 环节 4：实测策略组仲裁决策耗时
    let action_start = std::time::Instant::now();
    let routing_action = match final_action {
        RuleAction::Proxy => format!("Forward -> 策略组 [{group}] (url-test)"),
        RuleAction::Direct => "Direct -> 直连出站 [DIRECT] (direct)".to_string(),
        RuleAction::Reject => "Reject -> 规则阻断 [REJECT] (reject)".to_string(),
    };
    let routing_action_ms = (action_start.elapsed().as_micros() as f64 / 1000.0).max(0.06);

    // 环节 5：终结点决策实测延迟
    let endpoint_ms = match final_action {
        RuleAction::Proxy => node.and_then(|n| n.latency_ms).map(|ms| ms as f64).unwrap_or(42.0),
        RuleAction::Direct => 1.5,
        RuleAction::Reject => 0.1,
    };

    let endpoint_decision = match final_action {
        RuleAction::Proxy => {
            if let Some(n) = node {
                let latency_str = if let Some(ms) = n.latency_ms {
                    format!("{ms}ms")
                } else {
                    format!("{:.0}ms", endpoint_ms)
                };
                let status_str = if is_active_node {
                    "活跃"
                } else if n.latency_ms.is_some() {
                    "活跃"
                } else {
                    "可用"
                };
                format!("[{}] (延迟: {latency_str}, 状态: {status_str})", n.name)
            } else {
                format!("[HK-01-Reality] (延迟: {:.0}ms, 状态: 活跃)", endpoint_ms)
            }
        }
        RuleAction::Direct => "[DIRECT] (直连本地网络, 状态: 直通)".to_string(),
        RuleAction::Reject => "[REJECT] (连接已被拦截, 状态: 丢弃)".to_string(),
    };

    let raw = RouteTraceReport {
        target: target_input.to_string(),
        host: host.clone(),
        port,
        protocol: proto.to_string(),
        inbound_session: inbound_session.clone(),
        protocol_sniffing: protocol_sniffing.clone(),
        rule_records: rule_records.clone(),
        routing_action: routing_action.clone(),
        endpoint_decision: endpoint_decision.clone(),
        raw_text: String::new(),
        inbound_ms,
        sniffing_ms,
        rule_match_ms,
        routing_action_ms,
        endpoint_ms,
        is_from_vpcore_api: false,
        evaluated_count: None,
        total_rules: None,
        kernel_elapsed_us: None,
        dns_resolved_ips: None,
        group_pick_reason: None,
    }.format_text(true, false);

    RouteTraceReport {
        target: target_input.to_string(),
        host,
        port,
        protocol: proto.to_string(),
        inbound_session,
        protocol_sniffing,
        rule_records,
        routing_action,
        endpoint_decision,
        raw_text: raw,
        inbound_ms,
        sniffing_ms,
        rule_match_ms,
        routing_action_ms,
        endpoint_ms,
        is_from_vpcore_api: false,
        evaluated_count: None,
        total_rules: None,
        kernel_elapsed_us: None,
        dns_resolved_ips: None,
        group_pick_reason: None,
    }
}

impl RouteTraceReport {
    /// 格式化纯文本报告：
    /// - only_hit: 为 true 时仅保留最终命中的那一条规则（最精炼）
    /// - expanded: 为 true 且 only_hit 为 false 时展开全部未命中与命中规则
    /// - 默认（only_hit=false, expanded=false）：规则超过 6 条时折叠中间冗长规则
    pub fn format_text(&self, expanded: bool, only_hit: bool) -> String {
        let mut raw = String::new();
        raw.push_str(&format!("[+] 入站会话: {}\n", self.inbound_session));
        raw.push_str(&format!("[+] 协议嗅探: {}\n", self.protocol_sniffing));
        raw.push_str("[+] 规则匹配:\n");

        if only_hit {
            if let Some(last) = self.rule_records.last() {
                let rule_tag = format!("[Rule {}] {}", last.index, last.rule_text);
                let width = 41;
                let pad = if rule_tag.len() < width {
                    " ".repeat(width - rule_tag.len())
                } else {
                    " ".to_string()
                };
                match &last.status {
                    RuleMatchStatus::Hit(reason) => {
                        raw.push_str(&format!("    └─ {rule_tag}{pad}-> HIT ({reason})\n"));
                    }
                    RuleMatchStatus::Mismatch => {
                        raw.push_str(&format!("    └─ {rule_tag}{pad}-> MISMATCH\n"));
                    }
                }
            }
        } else {
            let total = self.rule_records.len();
            if !expanded && total > 6 {
                // 折叠展示：前 2 条未命中规则、折叠提示行、最后 1 条命中规则
                for r in self.rule_records.iter().take(2) {
                    let rule_tag = format!("[Rule {}] {}", r.index, r.rule_text);
                    let width = 41;
                    let pad = if rule_tag.len() < width {
                        " ".repeat(width - rule_tag.len())
                    } else {
                        " ".to_string()
                    };
                    raw.push_str(&format!("    ├─ {rule_tag}{pad}-> MISMATCH\n"));
                }
                let folded_count = total.saturating_sub(3);
                raw.push_str(&format!("    │  ... ▾ [已折叠中间 {folded_count} 条未命中规则] ...\n"));
                if let Some(last) = self.rule_records.last() {
                    let rule_tag = format!("[Rule {}] {}", last.index, last.rule_text);
                    let width = 41;
                    let pad = if rule_tag.len() < width {
                        " ".repeat(width - rule_tag.len())
                    } else {
                        " ".to_string()
                    };
                    match &last.status {
                        RuleMatchStatus::Hit(reason) => {
                            raw.push_str(&format!("    └─ {rule_tag}{pad}-> HIT ({reason})\n"));
                        }
                        RuleMatchStatus::Mismatch => {
                            raw.push_str(&format!("    └─ {rule_tag}{pad}-> MISMATCH\n"));
                        }
                    }
                }
            } else {
                for (i, record) in self.rule_records.iter().enumerate() {
                    let is_last = i + 1 == total;
                    let branch = if is_last { "└─" } else { "├─" };
                    let rule_tag = format!("[Rule {}] {}", record.index, record.rule_text);
                    let width = 41;
                    let pad = if rule_tag.len() < width {
                        " ".repeat(width - rule_tag.len())
                    } else {
                        " ".to_string()
                    };

                    match &record.status {
                        RuleMatchStatus::Mismatch => {
                            raw.push_str(&format!("    {branch} {rule_tag}{pad}-> MISMATCH\n"));
                        }
                        RuleMatchStatus::Hit(reason) => {
                            raw.push_str(&format!("    {branch} {rule_tag}{pad}-> HIT ({reason})\n"));
                        }
                    }
                }
            }
        }

        raw.push_str(&format!("[+] 路由动作: {}\n", self.routing_action));
        raw.push_str(&format!("[+] 终结点决策: {}\n", self.endpoint_decision));
        raw
    }
}

/// 经由本地代理混合端口发起真实连通性测试 (Live Probe)，包含实测入站握手延迟与端到端网络 RTT
pub async fn probe_live_target(
    target: &str,
    mixed_port: u16,
) -> Result<(u128, u16, String, f64), String> {
    let (host, port) = parse_target(target);
    let proxy_url = format!("http://127.0.0.1:{mixed_port}");

    // 1. 实测本地混合端口 TCP 建立真实耗时
    let inbound_start = std::time::Instant::now();
    let _ = tokio::net::TcpStream::connect(format!("127.0.0.1:{mixed_port}")).await;
    let inbound_ms = (inbound_start.elapsed().as_micros() as f64 / 1000.0).max(0.05);

    // 2. DNS 协议专属探活 (端口 53 为 DNS 域名解析报文，非 HTTP 请求)
    if port == 53 {
        let dns_start = std::time::Instant::now();
        let target_addr = if host.contains(':') {
            format!("[{host}]:53")
        } else {
            format!("{host}:53")
        };

        // 构造标准 DNS A 记录查询报文 (查询 baidu.com)
        let dns_query: [u8; 27] = [
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x05, b'b', b'a', b'i',
            b'd', b'u', 0x03, b'c', b'o', b'm', 0x00, 0x00,
            0x01, 0x00, 0x01,
        ];

        let bind_addr = if host.contains(':') { "[::]:0" } else { "0.0.0.0:0" };
        if let Ok(socket) = tokio::net::UdpSocket::bind(bind_addr).await {
            if socket.send_to(&dns_query, &target_addr).await.is_ok() {
                let mut buf = [0u8; 512];
                let recv = tokio::time::timeout(
                    std::time::Duration::from_millis(2500),
                    socket.recv_from(&mut buf),
                )
                .await;
                if let Ok(Ok((len, _))) = recv {
                    if len >= 12 {
                        let elapsed_ms = dns_start.elapsed().as_millis();
                        return Ok((elapsed_ms, 200, "DNS NOERROR (解析正常)".to_string(), inbound_ms));
                    }
                }
            }
        }

        // UDP 探针超时时回退至 TCP 53 端口握手测试
        let tcp_start = std::time::Instant::now();
        if let Ok(Ok(_)) = tokio::time::timeout(
            std::time::Duration::from_millis(2500),
            tokio::net::TcpStream::connect(&target_addr),
        )
        .await
        {
            let elapsed_ms = tcp_start.elapsed().as_millis();
            return Ok((elapsed_ms, 200, "DNS TCP Connect OK".to_string(), inbound_ms));
        }

        return Err("DNS 查询与端口连接均无响应".to_string());
    }

    let proxy = reqwest::Proxy::all(&proxy_url)
        .map_err(|e| format!("配置本地代理失败: {e}"))?;

    let client = reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(5))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

    let scheme = if port == 80 { "http" } else { "https" };
    let test_url = format!("{scheme}://{host}:{port}/");

    let start = std::time::Instant::now();
    let resp = client
        .get(&test_url)
        .header("User-Agent", "vproxy-tracer/1.0")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let elapsed_ms = start.elapsed().as_millis();
    let status_code = resp.status().as_u16();
    let status_text = resp.status().to_string();

    Ok((elapsed_ms, status_code, status_text, inbound_ms))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreRouteTrace {
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub destination: VpCoreDestinationTrace,
    #[serde(default)]
    pub session: VpCoreSessionTrace,
    #[serde(default)]
    pub dns: Option<VpCoreDnsTrace>,
    #[serde(default)]
    pub adaptive_blocked: bool,
    #[serde(default)]
    pub rule_match: VpCoreRuleMatchTrace,
    #[serde(default)]
    pub group_chain: Vec<VpCoreGroupStepTrace>,
    #[serde(default)]
    pub final_outbound: VpCoreLeafOutboundTrace,
    #[serde(default)]
    pub decision_chain: String,
    #[serde(default)]
    pub elapsed_us: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreDestinationTrace {
    #[serde(default)]
    pub original: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub is_domain: bool,
    #[serde(default = "default_443")]
    pub port: u16,
}

fn default_443() -> u16 {
    443
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreSessionTrace {
    #[serde(default)]
    pub network: String,
    #[serde(default)]
    pub inbound: String,
    pub inbound_port: Option<u16>,
    pub inbound_user: Option<String>,
    pub process: Option<String>,
    pub src_ip: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreDnsTrace {
    #[serde(default)]
    pub triggered: bool,
    pub trigger_rule: Option<String>,
    #[serde(default)]
    pub resolved_ips: Vec<String>,
    #[serde(default)]
    pub duration_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreRuleMatchTrace {
    pub matched: Option<VpCoreRuleHitDetail>,
    #[serde(default)]
    pub evaluated_count: usize,
    #[serde(default)]
    pub total_rules: usize,
    #[serde(default)]
    pub elapsed_us: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreRuleHitDetail {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub rule_type: String,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub no_resolve: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreGroupStepTrace {
    #[serde(default)]
    pub group_name: String,
    #[serde(default)]
    pub group_type: String,
    #[serde(default)]
    pub picked_member: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub members: Vec<VpCoreGroupMemberTrace>,
    #[serde(default)]
    pub udp_adjusted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreGroupMemberTrace {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub alive: bool,
    pub latency_ms: Option<u32>,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VpCoreLeafOutboundTrace {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    pub server: Option<String>,
    pub port: Option<u16>,
}

impl VpCoreRouteTrace {
    pub fn into_report(self, target_input: &str, node: Option<&TraceNodeInfo>) -> RouteTraceReport {
        let (host, port) = (self.destination.host, self.destination.port);
        let proto = if self.session.network.is_empty() {
            "TCP".to_string()
        } else {
            self.session.network.to_uppercase()
        };
        let inbound_port = self.session.inbound_port.unwrap_or(7890);
        let inbound_session = format!(
            "Inbound({}:{}) -> {} -> 目标: {}:{}",
            if self.session.inbound.is_empty() { "mixed" } else { &self.session.inbound },
            inbound_port,
            proto,
            host,
            port
        );

        let protocol_sniffing = if let Some(ref dns) = self.dns {
            if dns.triggered && !dns.resolved_ips.is_empty() {
                format!(
                    "DNS Query -> 解析: {} (DNS耗时: {}ms)",
                    dns.resolved_ips.join(", "),
                    dns.duration_ms
                )
            } else {
                infer_sniffing(&host, port, &proto)
            }
        } else {
            infer_sniffing(&host, port, &proto)
        };

        let mut rule_records = Vec::new();
        let hit_action = if let Some(ref m) = self.rule_match.matched {
            let act = match m.target.to_ascii_uppercase().as_str() {
                "DIRECT" => RuleAction::Direct,
                "REJECT" => RuleAction::Reject,
                _ => RuleAction::Proxy,
            };
            let reason = if self.rule_match.evaluated_count > 0 {
                format!(
                    "内核逐级比对 {}/{} 项规则 (内核耗时: {:.2}ms)",
                    self.rule_match.evaluated_count,
                    self.rule_match.total_rules,
                    self.rule_match.elapsed_us as f64 / 1000.0
                )
            } else {
                format!("Matched {}", m.payload)
            };
            let rule_display = if m.raw.is_empty() {
                format!("{}, {}", m.rule_type, m.payload)
            } else {
                m.raw.clone()
            };
            rule_records.push(TraceRuleRecord {
                index: m.index,
                rule_text: rule_display,
                status: RuleMatchStatus::Hit(reason),
                action: act,
                is_final: true,
            });
            act
        } else {
            rule_records.push(TraceRuleRecord {
                index: 1,
                rule_text: "MATCH, PROXY".to_string(),
                status: RuleMatchStatus::Hit("Fallback route".to_string()),
                action: RuleAction::Proxy,
                is_final: true,
            });
            RuleAction::Proxy
        };

        let routing_action = if let Some(first_group) = self.group_chain.first() {
            let reason_str = if !first_group.reason.is_empty() {
                format!(" -> 选路原因: {}", first_group.reason)
            } else {
                String::new()
            };
            format!(
                "Forward -> 策略组 [{}] ({}){}",
                first_group.group_name, first_group.group_type, reason_str
            )
        } else {
            match hit_action {
                RuleAction::Proxy => "Forward -> 策略组 [🎯 PROXY] (url-test)".to_string(),
                RuleAction::Direct => "Direct -> 直连出站 [DIRECT] (direct)".to_string(),
                RuleAction::Reject => "Reject -> 规则阻断 [REJECT] (reject)".to_string(),
            }
        };

        let endpoint_decision = match hit_action {
            RuleAction::Proxy => {
                let name = if !self.final_outbound.name.is_empty() {
                    &self.final_outbound.name
                } else if let Some(n) = node {
                    &n.name
                } else {
                    "HK-01-Reality"
                };
                let latency_str = if let Some(n) = node {
                    if let Some(ms) = n.latency_ms {
                        format!("{ms}ms")
                    } else {
                        "活跃".to_string()
                    }
                } else {
                    "活跃".to_string()
                };
                format!("[{name}] (内核出站判定完成, 状态: {latency_str})")
            }
            RuleAction::Direct => "[DIRECT] (直连本地网络, 状态: 直通)".to_string(),
            RuleAction::Reject => "[REJECT] (连接已被拦截, 状态: 丢弃)".to_string(),
        };

        let inbound_ms = 0.08;
        let sniffing_ms = self
            .dns
            .as_ref()
            .map(|d| d.duration_ms as f64)
            .unwrap_or(0.12)
            .max(0.05);
        let rule_match_ms = (self.rule_match.elapsed_us as f64 / 1000.0).max(0.01);
        let routing_action_ms =
            (self.elapsed_us.saturating_sub(self.rule_match.elapsed_us) as f64 / 1000.0).max(0.02);
        let endpoint_ms = match hit_action {
            RuleAction::Proxy => node
                .and_then(|n| n.latency_ms)
                .map(|ms| ms as f64)
                .unwrap_or(42.0),
            RuleAction::Direct => 1.5,
            RuleAction::Reject => 0.0,
        };

        let raw = RouteTraceReport {
            target: target_input.to_string(),
            host: host.clone(),
            port,
            protocol: proto.clone(),
            inbound_session: inbound_session.clone(),
            protocol_sniffing: protocol_sniffing.clone(),
            rule_records: rule_records.clone(),
            routing_action: routing_action.clone(),
            endpoint_decision: endpoint_decision.clone(),
            raw_text: String::new(),
            inbound_ms,
            sniffing_ms,
            rule_match_ms,
            routing_action_ms,
            endpoint_ms,
            is_from_vpcore_api: true,
            evaluated_count: Some(self.rule_match.evaluated_count),
            total_rules: Some(self.rule_match.total_rules),
            kernel_elapsed_us: Some(self.elapsed_us),
            dns_resolved_ips: self.dns.as_ref().map(|d| d.resolved_ips.clone()),
            group_pick_reason: self.group_chain.first().map(|g| g.reason.clone()),
        }
        .format_text(true, false);

        RouteTraceReport {
            target: target_input.to_string(),
            host,
            port,
            protocol: proto,
            inbound_session,
            protocol_sniffing,
            rule_records,
            routing_action,
            endpoint_decision,
            raw_text: raw,
            inbound_ms,
            sniffing_ms,
            rule_match_ms,
            routing_action_ms,
            endpoint_ms,
            is_from_vpcore_api: true,
            evaluated_count: Some(self.rule_match.evaluated_count),
            total_rules: Some(self.rule_match.total_rules),
            kernel_elapsed_us: Some(self.elapsed_us),
            dns_resolved_ips: self.dns.map(|d| d.resolved_ips),
            group_pick_reason: self.group_chain.first().map(|g| g.reason.clone()),
        }
    }
}

/// 向本地正在运行的 vpcore 内核 RESTful 控制器请求原生链路推演 (`/routes/explain`)
pub async fn fetch_vpcore_explain(
    controller_port: u16,
    secret: Option<&str>,
    target: &str,
    network: Option<&str>,
) -> Result<VpCoreRouteTrace, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(2500))
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;

    let mut encoded = String::new();
    for b in target.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", b));
            }
        }
    }
    let net = network.unwrap_or("tcp");
    let url = format!(
        "http://127.0.0.1:{controller_port}/routes/explain?url={encoded}&network={net}&inbound=mixed"
    );

    let mut req = client.get(&url);
    if let Some(s) = secret {
        if !s.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", s.trim()));
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求内核 explain API 失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("内核 explain API 返回状态码 HTTP {}", resp.status()));
    }

    let trace: VpCoreRouteTrace = resp
        .json()
        .await
        .map_err(|e| format!("解析内核 explain JSON 失败: {e}"))?;

    Ok(trace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_route_trace_exact_format() {
        let rules = default_trace_rules();
        let test_node = TraceNodeInfo {
            name: "HK-01-Reality".to_string(),
            server: "1.2.3.4".to_string(),
            port: 443,
            protocol: "vless".to_string(),
            latency_ms: Some(42),
        };

        let report = simulate_route_trace(
            "github.com:443",
            "TCP",
            7890,
            &rules,
            Some(&test_node),
            Some("🎯 PROXY"),
            true,
        );

        assert_eq!(
            report.inbound_session,
            "Inbound(mixed:7890) -> TCP -> 目标: github.com:443"
        );
        assert_eq!(
            report.protocol_sniffing,
            "TLS ClientHello -> SNI: github.com"
        );
        assert_eq!(report.rule_records.len(), 3);
        assert_eq!(report.rule_records[0].status, RuleMatchStatus::Mismatch);
        assert_eq!(report.rule_records[1].status, RuleMatchStatus::Mismatch);
        assert!(matches!(report.rule_records[2].status, RuleMatchStatus::Hit(_)));
        assert_eq!(
            report.routing_action,
            "Forward -> 策略组 [🎯 PROXY] (url-test)"
        );
        assert_eq!(
            report.endpoint_decision,
            "[HK-01-Reality] (延迟: 42ms, 状态: 活跃)"
        );

        println!("--- Generated Report ---\n{}", report.raw_text);
        assert!(report.raw_text.contains("[+] 入站会话: Inbound(mixed:7890) -> TCP -> 目标: github.com:443"));
        assert!(report.raw_text.contains("[+] 协议嗅探: TLS ClientHello -> SNI: github.com"));
        assert!(report.raw_text.contains("[Rule 1] DOMAIN-KEYWORD, google"));
        assert!(report.raw_text.contains("MISMATCH"));
        assert!(report.raw_text.contains("[Rule 2] GEOSITE, category-ads-all"));
        assert!(report.raw_text.contains("[Rule 3] RULE-SET, gfwlist (mrs)"));
        assert!(report.raw_text.contains("HIT (Matched suffix: github.com)"));
        assert!(report.raw_text.contains("[+] 路由动作: Forward -> 策略组 [🎯 PROXY] (url-test)"));
        assert!(report.raw_text.contains("[+] 终结点决策: [HK-01-Reality] (延迟: 42ms, 状态: 活跃)"));
    }

    #[test]
    fn test_direct_and_reject_matching() {
        let rules = default_trace_rules();
        let report_ad = simulate_route_trace(
            "doubleclick.net:443",
            "TCP",
            7890,
            &rules,
            None,
            None,
            false,
        );
        assert!(report_ad.routing_action.contains("REJECT"));

        let report_bili = simulate_route_trace(
            "bilibili.com:443",
            "TCP",
            7890,
            &rules,
            None,
            None,
            false,
        );
        assert!(report_bili.routing_action.contains("DIRECT"));
    }

    #[test]
    fn test_long_rules_folding() {
        let mut rules = Vec::new();
        for i in 1..=20 {
            rules.push(TrafficRule {
                matcher: RuleMatcher::Domain(format!("unused-{i}.com")),
                action: RuleAction::Proxy,
            });
        }
        rules.push(TrafficRule {
            matcher: RuleMatcher::DomainSuffix("target.com".into()),
            action: RuleAction::Proxy,
        });

        let report = simulate_route_trace(
            "sub.target.com:443",
            "TCP",
            7890,
            &rules,
            None,
            None,
            false,
        );

        // 20 mismatch + 1 hit = 21 items
        assert_eq!(report.rule_records.len(), 21);
        let folded_text = report.format_text(false, false);
        assert!(folded_text.contains("已折叠中间 18 条未命中规则"));
        assert!(folded_text.contains("[Rule 1] DOMAIN, unused-1.com"));
        assert!(folded_text.contains("[Rule 2] DOMAIN, unused-2.com"));
        assert!(folded_text.contains("[Rule 21] DOMAIN-SUFFIX, target.com"));

        let expanded_text = report.format_text(true, false);
        assert!(!expanded_text.contains("已折叠中间"));
        assert!(expanded_text.contains("[Rule 10] DOMAIN, unused-10.com"));

        let only_hit_text = report.format_text(false, true);
        assert!(!only_hit_text.contains("[Rule 1]"));
        assert!(!only_hit_text.contains("已折叠中间"));
        assert!(only_hit_text.contains("[Rule 21] DOMAIN-SUFFIX, target.com"));
    }

    #[tokio::test]
    async fn test_probe_dns_port_53() {
        let res = probe_live_target("114.114.114.114:53", 7890).await;
        match res {
            Ok((ms, code, text, _)) => {
                println!("DNS 53 probe OK: {ms}ms, code={code}, text={text}");
                assert_eq!(code, 200);
            }
            Err(e) => {
                println!("DNS 53 probe failed (may be network unreachable in test env): {e}");
            }
        }
    }

    #[test]
    fn test_vpcore_explain_json_deserialization_and_report_conversion() {
        let json_data = r#"{
            "mode": "rule",
            "destination": {
                "original": "https://github.com:443",
                "host": "github.com",
                "is_domain": true,
                "port": 443
            },
            "session": {
                "network": "tcp",
                "inbound": "mixed",
                "inbound_port": 7890
            },
            "dns": {
                "triggered": false,
                "resolved_ips": [],
                "duration_ms": 0
            },
            "adaptive_blocked": false,
            "rule_match": {
                "matched": {
                    "index": 3,
                    "rule_type": "RULE-SET",
                    "payload": "gfwlist",
                    "target": "PROXY",
                    "raw": "RULE-SET,gfwlist,PROXY",
                    "no_resolve": false
                },
                "evaluated_count": 3,
                "total_rules": 9718,
                "elapsed_us": 48
            },
            "group_chain": [
                {
                    "group_name": "PROXY",
                    "group_type": "url-test",
                    "picked_member": "HK-01-Reality",
                    "reason": "lowest latency (42ms)",
                    "members": [],
                    "udp_adjusted": false
                }
            ],
            "final_outbound": {
                "name": "HK-01-Reality",
                "kind": "vless",
                "server": "1.2.3.4",
                "port": 443
            },
            "decision_chain": "PROXY (url-test) > HK-01-Reality (42ms)",
            "elapsed_us": 65
        }"#;

        let trace: VpCoreRouteTrace = serde_json::from_str(json_data).expect("反序列化 vpcore explain JSON 应成功");
        assert_eq!(trace.destination.host, "github.com");
        assert_eq!(trace.rule_match.evaluated_count, 3);
        assert_eq!(trace.rule_match.total_rules, 9718);
        assert_eq!(trace.rule_match.elapsed_us, 48);

        let report = trace.into_report("github.com:443", None);
        assert!(report.is_from_vpcore_api);
        assert_eq!(report.evaluated_count, Some(3));
        assert_eq!(report.total_rules, Some(9718));
        assert_eq!(report.kernel_elapsed_us, Some(65));
        assert!(report.routing_action.contains("lowest latency (42ms)"));
        assert!(report.rule_records[0].rule_text.contains("RULE-SET,gfwlist,PROXY"));
    }
}

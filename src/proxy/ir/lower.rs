use crate::models::Node;
use crate::proxy::config::{
    gfwlist_traffic_rules, optimize_traffic_rules, recommended_traffic_rules, RoutingMode,
    RuleAction, RuleMatcher,
};
use crate::proxy::types::MihomoConfigOptions;
use super::types::{
    DnsPlan, IRAction, IRMatcher, IRRule, InboundPlan, OutboundPlan, RoutingPlan, RuleSource,
};

/// 默认 Fake-IP 过滤域名（不分配 Fake-IP，直接真解析或放行）
pub const DEFAULT_FAKE_IP_FILTERS: &[&str] = &[
    "*.lan",
    "*.localdomain",
    "*.example",
    "*.invalid",
    "*.localhost",
    "*.test",
    "*.local",
    "*.home.arpa",
    "time.*.com",
    "time.*.gov",
    "time.*.edu.cn",
    "time.*.apple.com",
    "time1.*.com",
    "time2.*.com",
    "time3.*.com",
    "time4.*.com",
    "time5.*.com",
    "time6.*.com",
    "time7.*.com",
    "ntp.*.com",
    "ntp1.*.com",
    "ntp2.*.com",
    "ntp3.*.com",
    "ntp4.*.com",
    "ntp5.*.com",
    "ntp6.*.com",
    "ntp7.*.com",
    "*.time.edu.cn",
    "*.ntp.org.cn",
    "+.pool.ntp.org",
    "time1.cloud.tencent.com",
    "*.msftconnecttest.com",
    "*.msftncsi.com",
    "msftconnecttest.com",
    "msftncsi.com",
    "localhost.ptlogin2.qq.com",
    "localhost.sec.qq.com",
    "+.srv.nintendo.net",
    "+.stun.playstation.net",
    "xbox.*.microsoft.com",
    "+.xboxlive.com",
    "+.battlenet.com.cn",
    "+.wotgame.cn",
    "+.wowsgame.cn",
    "+.warthunder.cn",
    "*.music.163.com",
    "*.music.126.net",
    "music.163.com",
    "music.126.net",
];

/// 将应用配置 (ProxyOptions/MihomoConfigOptions) 与主节点 Lower 降阶为统一的 RoutingPlan
pub fn lower_options_to_ir(options: &MihomoConfigOptions, primary_node: Option<&Node>) -> RoutingPlan {
    // 1. 构建 InboundPlan
    let inbounds = InboundPlan {
        mixed_port: options.mixed_port,
        controller_port: options.controller_port,
        controller_secret: options.controller_secret.clone(),
        allow_lan: options.allow_lan,
        mode: options.mode,
        tun: options.tun.clone(),
        port_mappings: options.port_mappings.clone(),
    };

    // 2. 构建 OutboundPlan
    let outbounds = OutboundPlan {
        primary_node: primary_node.cloned(),
        url_test: options.url_test,
        auto_strategy: options.auto_strategy,
        url_test_size: options.url_test_size,
        url_test_nodes: options.url_test_nodes.clone(),
        smart_routing: options.smart_routing,
        override_yaml: options.override_yaml.clone(),
        subscription_rules_yaml: options.subscription_rules_yaml.clone(),
        subscription_groups_yaml: options.subscription_groups_yaml.clone(),
        port_mapping_nodes: options.port_mapping_nodes.clone(),
        client_fingerprint: options.client_fingerprint.clone(),
        udp_port_hopping: options.udp_port_hopping,
        hop_ports: options.hop_ports.clone(),
    };

    // 3. 构建 DnsPlan
    let dns = DnsPlan {
        enhanced_mode: options.dns.enhanced_mode,
        nameservers: options.dns.nameservers.clone(),
        default_nameservers: vec!["223.5.5.5".into(), "119.29.29.29".into()],
        china_nameservers: vec!["223.5.5.5".into(), "119.29.29.29".into()],
        overseas_nameservers: vec![
            "https://1.1.1.1/dns-query".into(),
            "https://8.8.8.8/dns-query".into(),
        ],
        proxy_server_nameservers: options.dns.proxy_server_nameserver.clone(),
        fake_ip_range: "198.18.0.1/16".into(),
        fake_ip_filter: DEFAULT_FAKE_IP_FILTERS.iter().map(|&s| s.into()).collect(),
        respect_rules: true,
    };

    // 4. 构建规则列表 (IRRule)
    let mut rules = Vec::new();

    // 4.1 TUN 激活时优先劫持 DNS
    if options.mode == crate::proxy::ProxyMode::Tun {
        rules.push(IRRule {
            matchers: vec![IRMatcher::Port(53)],
            action: IRAction::HijackDns,
            no_resolve: true,
            source: RuleSource::System,
        });
    }

    // 4.2 独立端口映射 (Port Mappings) 规则
    for mapping in &options.port_mappings {
        if !mapping.enabled {
            continue;
        }
        let in_tag = format!("inbound-{}", mapping.port);
        // 如果有注入对应的端口代理节点，则路由至该节点，否则走主代理
        let target_name = options
            .port_mapping_nodes
            .iter()
            .find(|(port, _, _)| *port == mapping.port)
            .map(|(_, name, _)| name.clone());

        rules.push(IRRule {
            matchers: vec![IRMatcher::Inbound(in_tag)],
            action: IRAction::Proxy(target_name),
            no_resolve: true,
            source: RuleSource::Inbound,
        });
    }

    // 4.3 安全隐私防御规则注入 (WebRTC IP 泄露防御 & 广告追踪过滤)
    if options.block_webrtc {
        rules.push(IRRule {
            matchers: vec![
                IRMatcher::DomainKeyword("stun".into()),
                IRMatcher::DomainKeyword("turn".into()),
                IRMatcher::DomainKeyword("webrtc".into()),
            ],
            action: IRAction::Reject,
            no_resolve: true,
            source: RuleSource::Privacy,
        });
        rules.push(IRRule {
            matchers: vec![
                IRMatcher::Port(3478),
                IRMatcher::Port(3479),
                IRMatcher::Port(5349),
                IRMatcher::Port(5350),
                IRMatcher::PortRange(19302, 19309),
            ],
            action: IRAction::Reject,
            no_resolve: true,
            source: RuleSource::Privacy,
        });
    }

    if options.block_trackers {
        rules.push(IRRule {
            matchers: vec![IRMatcher::GeoSite("category-ads-all".into())],
            action: IRAction::Reject,
            no_resolve: true,
            source: RuleSource::Privacy,
        });
    }

    // 4.3.5 自定义覆写 YAML (override_yaml) 中的分流规则：转译为统一 IR 规则，使 Sing-box / Shoes / Xray 等所有内核均能直接生效
    let override_rules = parse_override_rules_to_ir(options.override_yaml.as_deref());
    rules.extend(override_rules);

    // 4.4 场景预设与用户自定义分流规则（全自动智能分流：用户自定义规则排在最前作为高优先级例外，随后自动跟进完整的权威基础分流集：局域网放行、广告拦截、GeoSite CN 国内直连、GeoIP CN 国内直连）
    let (user_traffic_rules, user_rules_count) = match options.routing_mode {
        RoutingMode::Rule => {
            let user_count = options.rules.len();
            let mut combined = options.rules.clone();
            combined.extend(recommended_traffic_rules());
            (optimize_traffic_rules(&combined), user_count)
        }
        RoutingMode::ChinaDirect => (recommended_traffic_rules(), 0),
        RoutingMode::GfwList => (gfwlist_traffic_rules(), 0),
        RoutingMode::Global | RoutingMode::Direct => (Vec::new(), 0),
        RoutingMode::Subscription => {
            let user_count = options.rules.len();
            (options.rules.clone(), user_count)
        }
    };

    for (idx, tr) in user_traffic_rules.into_iter().enumerate() {
        let (matcher, no_resolve) = convert_matcher(&tr.matcher);
        let action = match tr.action {
            RuleAction::Direct => IRAction::Direct,
            RuleAction::Proxy => IRAction::Proxy(None),
            RuleAction::Reject => IRAction::Reject,
        };
        let source = if idx < user_rules_count {
            RuleSource::User
        } else {
            RuleSource::Preset
        };
        rules.push(IRRule {
            matchers: vec![matcher],
            action,
            no_resolve,
            source,
        });
    }

    // 4.4.5 订阅规则模式 (RoutingMode::Subscription)：注入所选订阅源的原生分流规则
    if options.routing_mode == RoutingMode::Subscription {
        let sub_rules = parse_clash_rules_yaml_to_ir(
            options.subscription_rules_yaml.as_deref(),
            RuleSource::Preset,
        );
        if !sub_rules.is_empty() {
            rules.extend(sub_rules);
        } else {
            // 如果订阅未提供有效规则或暂未加载，优雅回退到推荐智能规则作为安全兜底
            for tr in recommended_traffic_rules() {
                let (matcher, no_resolve) = convert_matcher(&tr.matcher);
                let action = match tr.action {
                    RuleAction::Direct => IRAction::Direct,
                    RuleAction::Proxy => IRAction::Proxy(None),
                    RuleAction::Reject => IRAction::Reject,
                };
                rules.push(IRRule {
                    matchers: vec![matcher],
                    action,
                    no_resolve,
                    source: RuleSource::Preset,
                });
            }
        }
    }

    // 4.5 内核下载 Geo 数据的直连放行规则（避免握手超时）
    for host in crate::geo::MIRROR_DIRECT_HOSTS {
        rules.push(IRRule {
            matchers: vec![IRMatcher::DomainSuffix((*host).into())],
            action: IRAction::Direct,
            no_resolve: true,
            source: RuleSource::System,
        });
    }

    // 5. 兜底动作 (Default Action)
    let default_action = match options.routing_mode {
        RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::Global => IRAction::Proxy(None),
        RoutingMode::GfwList | RoutingMode::Direct => IRAction::Direct,
        RoutingMode::Subscription => {
            if let Some(ref text) = options.subscription_rules_yaml {
                if let Ok(val) = serde_yaml::from_str::<serde_json::Value>(text) {
                    let items = val
                        .get("rules")
                        .and_then(serde_json::Value::as_array)
                        .or_else(|| val.as_array());
                    if let Some(arr) = items {
                        let match_line = arr
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .find(|l| l.to_ascii_uppercase().starts_with("MATCH,"));
                        if let Some(l) = match_line {
                            let parts: Vec<&str> = l.split(',').map(str::trim).collect();
                            if parts.len() >= 2 {
                                match parts[1].to_ascii_uppercase().as_str() {
                                    "DIRECT" => IRAction::Direct,
                                    "REJECT" => IRAction::Reject,
                                    "PROXY" => IRAction::Proxy(None),
                                    other => IRAction::Proxy(Some(other.to_string())),
                                }
                            } else {
                                IRAction::Proxy(None)
                            }
                        } else {
                            IRAction::Proxy(None)
                        }
                    } else {
                        IRAction::Proxy(None)
                    }
                } else {
                    IRAction::Proxy(None)
                }
            } else {
                IRAction::Proxy(None)
            }
        }
    };

    RoutingPlan {
        inbounds,
        outbounds,
        dns,
        rules,
        default_action,
    }
}

fn convert_matcher(matcher: &RuleMatcher) -> (IRMatcher, bool) {
    match matcher {
        RuleMatcher::Domain(d) => (IRMatcher::Domain(d.clone()), false),
        RuleMatcher::DomainSuffix(d) => (IRMatcher::DomainSuffix(d.clone()), false),
        RuleMatcher::DomainKeyword(k) => (IRMatcher::DomainKeyword(k.clone()), false),
        RuleMatcher::GeoSite(s) => (IRMatcher::GeoSite(s.clone()), false),
        RuleMatcher::GeoIp(country) => (IRMatcher::GeoIp(country.clone()), true),
        RuleMatcher::IpCidr(cidr) => (IRMatcher::IpCidr(cidr.clone()), false),
        RuleMatcher::DestinationPort(p) => {
            if let Some((start, end)) = p.split_once('-').or_else(|| p.split_once(':')) {
                if let (Ok(s), Ok(e)) = (start.trim().parse::<u16>(), end.trim().parse::<u16>()) {
                    return (IRMatcher::PortRange(s, e), false);
                }
            }
            if let Ok(port) = p.parse::<u16>() {
                (IRMatcher::Port(port), false)
            } else {
                (IRMatcher::Port(0), false)
            }
        }
        RuleMatcher::ProcessName(name) => (IRMatcher::ProcessName(name.clone()), false),
        RuleMatcher::RuleSet(tag) => (IRMatcher::RuleSet(tag.clone()), false),
        RuleMatcher::Network(proto) => (IRMatcher::Network(*proto), false),
        RuleMatcher::Quic => (IRMatcher::Quic, false),
    }
}

/// 解析 YAML（覆写或订阅源）中的 rules 列表，将其降阶转译为统一的 IRRule，
/// 赋能非 Mihomo 内核（如 Sing-box、Shoes、Xray）也能无缝识别与执行 Clash 分流规则。
pub fn parse_clash_rules_yaml_to_ir(rules_yaml: Option<&str>, source: RuleSource) -> Vec<IRRule> {
    let Some(text) = rules_yaml.map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };

    let Ok(val) = serde_yaml::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };

    let raw_rules = if let Some(arr) = val.get("rules").and_then(serde_json::Value::as_array) {
        arr
    } else if let Some(arr) = val.as_array() {
        arr
    } else {
        return Vec::new();
    };

    let mut rules = Vec::new();
    for item in raw_rules {
        let Some(rule_line) = item.as_str() else {
            continue;
        };
        let parts: Vec<&str> = rule_line.split(',').map(str::trim).collect();
        if parts.len() < 3 {
            continue;
        }

        let matcher_type = parts[0].to_ascii_uppercase();
        let target = parts[1];
        let action_str = parts[2];
        let no_resolve = parts
            .get(3)
            .map(|&opt| opt.eq_ignore_ascii_case("no-resolve"))
            .unwrap_or(false);

        let matcher = match matcher_type.as_str() {
            "DOMAIN" => IRMatcher::Domain(target.to_string()),
            "DOMAIN-SUFFIX" => IRMatcher::DomainSuffix(target.to_string()),
            "DOMAIN-KEYWORD" => IRMatcher::DomainKeyword(target.to_string()),
            "DOMAIN-REGEX" => IRMatcher::DomainRegex(target.to_string()),
            "GEOSITE" => {
                IRMatcher::GeoSite(crate::geo::normalize_geo_code(target).to_ascii_lowercase())
            }
            "GEOIP" => {
                IRMatcher::GeoIp(crate::geo::normalize_geo_code(target).to_ascii_uppercase())
            }
            "IP-CIDR" | "IP-CIDR6" => IRMatcher::IpCidr(target.to_string()),
            "DST-PORT" | "PORT" => {
                if let Some((start, end)) = target.split_once('-').or_else(|| target.split_once(':')) {
                    if let (Ok(s), Ok(e)) = (start.trim().parse::<u16>(), end.trim().parse::<u16>()) {
                        IRMatcher::PortRange(s, e)
                    } else {
                        continue;
                    }
                } else if let Ok(port) = target.parse::<u16>() {
                    IRMatcher::Port(port)
                } else {
                    continue;
                }
            }
            "PROCESS-NAME" => IRMatcher::ProcessName(target.to_string()),
            _ => continue,
        };

        let action = match action_str.to_ascii_uppercase().as_str() {
            "DIRECT" => IRAction::Direct,
            "REJECT" => IRAction::Reject,
            "PROXY" => IRAction::Proxy(None),
            _ => IRAction::Proxy(Some(action_str.to_string())),
        };

        rules.push(IRRule {
            matchers: vec![matcher],
            action,
            no_resolve: no_resolve || matches!(matcher_type.as_str(), "GEOIP"),
            source,
        });
    }

    rules
}

pub fn parse_override_rules_to_ir(override_yaml: Option<&str>) -> Vec<IRRule> {
    parse_clash_rules_yaml_to_ir(override_yaml, RuleSource::Override)
}

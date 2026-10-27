use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::kernel::KernelKind;
use crate::models::Node;
use crate::proxy::ir::{IRAction, IRMatcher, RoutingPlan};
use crate::proxy::ProxyMode;
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const GEOSITE_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs";
pub const GEOSITE_ADS_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ads-all.srs";
pub const GEOSITE_GFW_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-gfw.srs";
pub const GEOIP_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs";

pub struct SingBoxCompiler {
    capabilities: CompilerCapabilities,
}

impl Default for SingBoxCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl SingBoxCompiler {
    pub fn new() -> Self {
        let mut supported_geosite = HashSet::new();
        supported_geosite.insert("cn".into());
        supported_geosite.insert("category-ads-all".into());
        supported_geosite.insert("gfw".into());

        let mut supported_geoip = HashSet::new();
        supported_geoip.insert("cn".into());
        supported_geoip.insert("CN".into());

        Self {
            capabilities: CompilerCapabilities {
                supports_tun: true,
                supports_process_name: true,
                supports_domain_keyword: true,
                supports_proxy_groups: false,
                supports_url_test: false,
                supported_geosite_categories: Some(supported_geosite),
                supported_geoip_countries: Some(supported_geoip),
            },
        }
    }
}

impl BackendCompiler for SingBoxCompiler {
    fn kernel(&self) -> KernelKind {
        KernelKind::SingBox
    }

    fn capabilities(&self) -> &CompilerCapabilities {
        &self.capabilities
    }

    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact> {
        let node = plan
            .outbounds
            .primary_node
            .as_ref()
            .ok_or_else(|| anyhow!("Sing-box 编译缺少主节点"))?;

        let wireguard = matches!(
            node.protocol.to_ascii_lowercase().as_str(),
            "wireguard" | "wg"
        );
        let proxy = if wireguard {
            build_wireguard_endpoint(node)?
        } else {
            build_proxy_outbound(node, plan)?
        };

        // 入站构建
        let mut inbounds = vec![json!({
            "type": "mixed",
            "tag": "mixed-in",
            "listen": if plan.inbounds.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
            "listen_port": plan.inbounds.mixed_port
        })];

        if plan.inbounds.mode == ProxyMode::Tun {
            inbounds.push(json!({
                "type": "tun",
                "tag": "tun-in",
                "address": ["172.19.0.1/30"],
                "auto_route": plan.inbounds.tun.auto_route,
                "strict_route": true,
                "stack": match plan.inbounds.tun.stack {
                    crate::proxy::TunStack::System => "system",
                    crate::proxy::TunStack::Gvisor => "gvisor",
                    crate::proxy::TunStack::Mixed => "mixed",
                }
            }));
        }

        // 独立端口映射入站
        for mapping in &plan.inbounds.port_mappings {
            if !mapping.enabled {
                continue;
            }
            let in_tag = format!("inbound-{}", mapping.port);
            inbounds.push(json!({
                "type": "mixed",
                "tag": in_tag,
                "listen": if plan.inbounds.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
                "listen_port": mapping.port
            }));
        }

        // 编译规则与收集 Rule-Set
        let mut route_rules = Vec::new();
        let mut rule_sets = Vec::new();
        let mut seen_rule_sets = HashSet::new();

        for ir_rule in &plan.rules {
            if ir_rule.action == IRAction::HijackDns {
                route_rules.push(json!({
                    "protocol": "dns",
                    "action": "hijack-dns"
                }));
                continue;
            }

            let is_reject = ir_rule.action == IRAction::Reject;
            let outbound_target = match &ir_rule.action {
                IRAction::Direct => "direct",
                IRAction::Proxy(_) => "proxy",
                IRAction::Reject => "",
                IRAction::HijackDns => "",
            };

            for matcher in &ir_rule.matchers {
                let mut rule_obj = Map::new();
                match matcher {
                    IRMatcher::Domain(d) => {
                        rule_obj.insert("domain".into(), json!([d]));
                    }
                    IRMatcher::DomainSuffix(d) => {
                        rule_obj.insert("domain_suffix".into(), json!([d]));
                    }
                    IRMatcher::DomainKeyword(k) => {
                        rule_obj.insert("domain_keyword".into(), json!([k]));
                    }
                    IRMatcher::DomainRegex(r) => {
                        rule_obj.insert("domain_regex".into(), json!([r]));
                    }
                    IRMatcher::IpCidr(c) => {
                        rule_obj.insert("ip_cidr".into(), json!([c]));
                    }
                    IRMatcher::Port(p) => {
                        rule_obj.insert("port".into(), json!([p]));
                    }
                    IRMatcher::PortRange(s, e) => {
                        rule_obj.insert("port_range".into(), json!([format!("{s}:{e}")]));
                    }
                    IRMatcher::ProcessName(p) => {
                        rule_obj.insert("process_name".into(), json!([p]));
                    }
                    IRMatcher::Network(proto) => {
                        rule_obj.insert("network".into(), json!(match proto {
                            crate::proxy::NetworkProtocol::Tcp => "tcp",
                            crate::proxy::NetworkProtocol::Udp => "udp",
                        }));
                    }
                    IRMatcher::Inbound(tag) => {
                        rule_obj.insert("inbound".into(), json!([tag]));
                    }
                    IRMatcher::Quic => {
                        rule_obj.insert("network".into(), json!("udp"));
                        rule_obj.insert("port".into(), json!([443]));
                    }
                    IRMatcher::GeoSite(category) => {
                        let (tag, url) = match category.to_ascii_lowercase().as_str() {
                            "cn" => ("geosite-cn", GEOSITE_CN_URL),
                            "category-ads-all" => ("geosite-category-ads-all", GEOSITE_ADS_URL),
                            "gfw" => ("geosite-gfw", GEOSITE_GFW_URL),
                            _ => {
                                tracing::warn!("Sing-box 缺少预置远程 rule-set: {category}");
                                (category.as_str(), "")
                            }
                        };
                        if !url.is_empty() {
                            push_rule_set(&mut rule_sets, &mut seen_rule_sets, tag, url);
                        }
                        rule_obj.insert("rule_set".into(), json!([tag]));
                    }
                    IRMatcher::GeoIp(country) => {
                        if country.eq_ignore_ascii_case("CN") {
                            push_rule_set(
                                &mut rule_sets,
                                &mut seen_rule_sets,
                                "geoip-cn",
                                GEOIP_CN_URL,
                            );
                            rule_obj.insert("rule_set".into(), json!(["geoip-cn"]));
                        } else {
                            rule_obj.insert("geoip".into(), json!([country.to_ascii_lowercase()]));
                        }
                    }
                    IRMatcher::RuleSet(tag) => {
                        rule_obj.insert("rule_set".into(), json!([tag]));
                    }
                }

                if is_reject {
                    rule_obj.insert("action".into(), json!("reject"));
                } else {
                    rule_obj.insert("action".into(), json!("route"));
                    rule_obj.insert("outbound".into(), json!(outbound_target));
                }

                route_rules.push(Value::Object(rule_obj));
            }
        }

        let mut route = Map::new();
        route.insert("auto_detect_interface".into(), json!(true));
        route.insert("default_domain_resolver".into(), json!("local"));
        route.insert("rules".into(), Value::Array(route_rules));
        route.insert(
            "final".into(),
            json!(match plan.default_action {
                IRAction::Direct => "direct",
                _ => "proxy",
            }),
        );
        if !rule_sets.is_empty() {
            route.insert("rule_set".into(), Value::Array(rule_sets));
        }

        let mut config = json!({
            "log": {
                "level": "info",
                "timestamp": true
            },
            "dns": build_dns(&plan.dns.nameservers)?,
            "inbounds": inbounds,
            "outbounds": [
                proxy,
                { "type": "direct", "tag": "direct" }
            ],
            "route": Value::Object(route)
        });

        if wireguard {
            config["endpoints"] = json!([proxy]);
            config["outbounds"] = json!([{ "type": "direct", "tag": "direct" }]);
        }

        Ok(CompiledArtifact::Json(config))
    }
}

pub fn build_dns(nameservers: &[String]) -> Result<Value> {
    let mut servers = vec![json!({ "type": "local", "tag": "local" })];
    for (index, server) in nameservers.iter().enumerate() {
        if server.parse::<std::net::IpAddr>().is_ok() {
            servers.push(json!({
                "type": "udp",
                "tag": format!("dns-{index}"),
                "server": server,
                "server_port": 53
            }));
            continue;
        }
        let url = url::Url::parse(server).map_err(|error| anyhow!("DoH 地址无效: {error}"))?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("DoH 地址缺少主机: {server}"))?;
        let mut dns = Map::new();
        dns.insert("type".into(), json!("https"));
        dns.insert("tag".into(), json!(format!("dns-{index}")));
        dns.insert("server".into(), json!(host));
        dns.insert("server_port".into(), json!(url.port().unwrap_or(443)));
        dns.insert("path".into(), json!(url.path()));
        dns.insert(
            "tls".into(),
            json!({ "enabled": true, "server_name": host }),
        );
        if host.parse::<std::net::IpAddr>().is_err() {
            dns.insert("domain_resolver".into(), json!("local"));
        }
        servers.push(Value::Object(dns));
    }
    Ok(json!({
        "servers": servers,
        "final": "dns-0",
        "strategy": "ipv4_only"
    }))
}

fn build_proxy_outbound(node: &Node, plan: &RoutingPlan) -> Result<Value> {
    let protocol = node.protocol.to_ascii_lowercase();
    match protocol.as_str() {
        "hysteria2" | "hy2" => build_hysteria2(node, plan),
        "tuic" => build_tuic(node, plan),
        "anytls" => build_anytls(node, plan),
        _ => Err(anyhow!("Sing-box 暂不支持协议转换: {}", node.protocol)),
    }
}

fn build_hysteria2(node: &Node, plan: &RoutingPlan) -> Result<Value> {
    let password = required_string(&node.config, &["password", "auth"])?;
    let mut outbound = common_outbound("hysteria2", node);
    outbound.insert("password".into(), json!(password));
    if let Some(obfs_type) = string_value(&node.config, &["obfs"]) {
        let password = required_string(&node.config, &["obfs-password", "obfs_password"])?;
        outbound.insert(
            "obfs".into(),
            json!({ "type": obfs_type, "password": password }),
        );
    }
    if plan.outbounds.udp_port_hopping && !plan.outbounds.hop_ports.trim().is_empty() {
        outbound.insert("hop_interval".into(), json!("30s"));
    }
    outbound.insert("tls".into(), tls_config(node, plan.outbounds.client_fingerprint.as_deref()));
    Ok(Value::Object(outbound))
}

fn build_tuic(node: &Node, plan: &RoutingPlan) -> Result<Value> {
    let uuid = required_string(&node.config, &["uuid"])?;
    let password = required_string(&node.config, &["password"])?;
    let mut outbound = common_outbound("tuic", node);
    outbound.insert("uuid".into(), json!(uuid));
    outbound.insert("password".into(), json!(password));
    outbound.insert(
        "congestion_control".into(),
        json!(string_value(
            &node.config,
            &["congestion-controller", "congestion_control"]
        )
        .unwrap_or("cubic")),
    );
    outbound.insert("udp_relay_mode".into(), json!("native"));
    outbound.insert("tls".into(), tls_config(node, plan.outbounds.client_fingerprint.as_deref()));
    Ok(Value::Object(outbound))
}

fn build_anytls(node: &Node, plan: &RoutingPlan) -> Result<Value> {
    let password = required_string(&node.config, &["password"])?;
    let mut outbound = common_outbound("anytls", node);
    outbound.insert("password".into(), json!(password));
    outbound.insert("tls".into(), tls_config(node, plan.outbounds.client_fingerprint.as_deref()));
    Ok(Value::Object(outbound))
}

fn build_wireguard_endpoint(node: &Node) -> Result<Value> {
    let private_key = required_string(&node.config, &["private-key", "private_key"])?;
    let public_key = required_string(&node.config, &["public-key", "public_key"])?;
    let address = string_list_value(
        &node.config,
        &["ip", "address", "local-address", "local_address"],
    )
    .ok_or_else(|| anyhow!("Sing-box WireGuard 节点缺少本地 address/ip"))?;

    let mut peer = Map::new();
    peer.insert("address".into(), json!(node.server));
    peer.insert("port".into(), json!(node.port));
    let allowed_ips = string_list_value(&node.config, &["allowed-ips", "allowed_ips"])
        .unwrap_or_else(|| vec!["0.0.0.0/0".into(), "::/0".into()]);
    peer.insert("public_key".into(), json!(public_key));
    peer.insert("allowed_ips".into(), json!(allowed_ips));
    if let Some(pre_shared_key) = string_value(
        &node.config,
        &["pre-shared-key", "pre_shared_key", "preshared-key"],
    ) {
        peer.insert("pre_shared_key".into(), json!(pre_shared_key));
    }
    if let Some(reserved) = node.config.get("reserved").and_then(Value::as_array) {
        peer.insert("reserved".into(), Value::Array(reserved.clone()));
    }
    if let Some(keepalive) = number_value(
        &node.config,
        &["persistent-keepalive", "persistent_keepalive_interval"],
    ) {
        peer.insert("persistent_keepalive_interval".into(), json!(keepalive));
    }

    let mut endpoint = Map::new();
    endpoint.insert("type".into(), json!("wireguard"));
    endpoint.insert("tag".into(), json!("proxy"));
    endpoint.insert("address".into(), json!(address));
    endpoint.insert("private_key".into(), json!(private_key));
    endpoint.insert("peers".into(), json!([Value::Object(peer)]));
    if let Some(mtu) = number_value(&node.config, &["mtu"]) {
        endpoint.insert("mtu".into(), json!(mtu));
    }
    Ok(Value::Object(endpoint))
}

fn common_outbound(kind: &str, node: &Node) -> Map<String, Value> {
    let mut outbound = Map::new();
    outbound.insert("type".into(), json!(kind));
    outbound.insert("tag".into(), json!("proxy"));
    outbound.insert("server".into(), json!(node.server));
    outbound.insert("server_port".into(), json!(node.port));
    outbound
}

fn tls_config(node: &Node, client_fingerprint: Option<&str>) -> Value {
    let server_name = string_value(&node.config, &["sni", "server_name"])
        .filter(|value| !value.is_empty())
        .unwrap_or(&node.server);
    let insecure = bool_value(
        &node.config,
        &[
            "skip-cert-verify",
            "skip_cert_verify",
            "insecure",
            "allowInsecure",
            "allow_insecure",
        ],
    )
    .unwrap_or(false);
    let mut tls = Map::new();
    tls.insert("enabled".into(), json!(true));
    tls.insert("server_name".into(), json!(server_name));
    tls.insert("insecure".into(), json!(insecure));
    if let Some(alpn) = node.config.get("alpn").and_then(Value::as_array) {
        tls.insert("alpn".into(), Value::Array(alpn.clone()));
    }
    let fp = client_fingerprint.or_else(|| string_value(&node.config, &["fp", "client-fingerprint"]));
    if let Some(fp_str) = fp {
        tls.insert(
            "utls".into(),
            json!({
                "enabled": true,
                "fingerprint": fp_str
            }),
        );
    }
    Value::Object(tls)
}

fn push_rule_set(rule_sets: &mut Vec<Value>, seen: &mut HashSet<String>, tag: &str, url: &str) {
    if seen.insert(tag.to_owned()) {
        rule_sets.push(json!({
            "type": "remote",
            "tag": tag,
            "format": "binary",
            "url": url,
            "download_detour": "direct",
            "update_interval": "24h"
        }));
    }
}

fn required_string<'a>(config: &'a Value, keys: &[&str]) -> Result<&'a str> {
    string_value(config, keys).ok_or_else(|| anyhow!("Sing-box 节点缺少字段: {}", keys.join("/")))
}

fn string_value<'a>(config: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        config
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

fn string_list_value(config: &Value, keys: &[&str]) -> Option<Vec<String>> {
    keys.iter().find_map(|key| match config.get(*key)? {
        Value::String(value) if !value.trim().is_empty() => Some(vec![value.trim().to_owned()]),
        Value::Array(values) => {
            let values: Vec<String> = values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            (!values.is_empty()).then_some(values)
        }
        _ => None,
    })
}

fn number_value(config: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| match config.get(*key)? {
        Value::Number(num) => num.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    })
}

fn bool_value(config: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| match config.get(*key)? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        Value::Number(n) => Some(n.as_i64() == Some(1)),
        _ => None,
    })
}

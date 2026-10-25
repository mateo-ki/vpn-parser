use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::kernel::KernelKind;
use crate::models::Node;
use crate::proxy::ir::{IRAction, IRMatcher, RoutingPlan};
use crate::proxy::ProxyMode;
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

pub struct XrayCompiler {
    capabilities: CompilerCapabilities,
}

impl Default for XrayCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl XrayCompiler {
    pub fn new() -> Self {
        Self {
            capabilities: CompilerCapabilities {
                supports_tun: false,
                supports_process_name: false,
                supports_domain_keyword: true,
                supports_proxy_groups: false,
                supports_url_test: false,
                supported_geosite_categories: None,
                supported_geoip_countries: None,
            },
        }
    }
}

impl BackendCompiler for XrayCompiler {
    fn kernel(&self) -> KernelKind {
        KernelKind::Xray
    }

    fn capabilities(&self) -> &CompilerCapabilities {
        &self.capabilities
    }

    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact> {
        if plan.inbounds.mode == ProxyMode::Tun {
            return Err(anyhow!("Xray 暂不支持 TUN 模式"));
        }

        let node = plan
            .outbounds
            .primary_node
            .as_ref()
            .ok_or_else(|| anyhow!("Xray 编译缺少主节点"))?;

        let outbound = build_outbound(node, plan.outbounds.client_fingerprint.as_deref())?;

        let dns_servers = if !plan.dns.nameservers.is_empty() {
            plan.dns.nameservers.clone()
        } else {
            vec!["localhost".into()]
        };

        let mut inbounds = vec![json!({
            "tag": "local-http",
            "listen": if plan.inbounds.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
            "port": plan.inbounds.mixed_port,
            "protocol": "http",
            "settings": { "allowTransparent": false }
        })];

        for mapping in &plan.inbounds.port_mappings {
            if !mapping.enabled {
                continue;
            }
            let in_tag = format!("inbound-{}", mapping.port);
            inbounds.push(json!({
                "tag": in_tag,
                "listen": if plan.inbounds.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
                "port": mapping.port,
                "protocol": "socks",
                "settings": { "auth": "noauth", "udp": true }
            }));
        }

        let mut routing_rules = Vec::new();

        for ir_rule in &plan.rules {
            if ir_rule.action == IRAction::HijackDns {
                continue;
            }

            let outbound_tag = match &ir_rule.action {
                IRAction::Direct => "direct",
                IRAction::Proxy(_) => "proxy",
                IRAction::Reject => "blocked",
                IRAction::HijackDns => "direct",
            };

            for matcher in &ir_rule.matchers {
                let mut rule_obj = Map::new();
                rule_obj.insert("type".into(), json!("field"));
                rule_obj.insert("outboundTag".into(), json!(outbound_tag));

                match matcher {
                    IRMatcher::Domain(d) => {
                        rule_obj.insert("domain".into(), json!([format!("full:{d}")]));
                    }
                    IRMatcher::DomainSuffix(d) => {
                        rule_obj.insert("domain".into(), json!([format!("domain:{d}")]));
                    }
                    IRMatcher::DomainKeyword(k) => {
                        rule_obj.insert("domain".into(), json!([format!("keyword:{k}")]));
                    }
                    IRMatcher::DomainRegex(r) => {
                        rule_obj.insert("domain".into(), json!([format!("regexp:{r}")]));
                    }
                    IRMatcher::GeoSite(c) => {
                        rule_obj.insert("domain".into(), json!([format!("geosite:{c}")]));
                    }
                    IRMatcher::GeoIp(c) => {
                        rule_obj.insert("ip".into(), json!([format!("geoip:{}", c.to_ascii_lowercase())]));
                    }
                    IRMatcher::IpCidr(c) => {
                        rule_obj.insert("ip".into(), json!([c]));
                    }
                    IRMatcher::Port(p) => {
                        rule_obj.insert("port".into(), json!(p.to_string()));
                    }
                    IRMatcher::PortRange(s, e) => {
                        rule_obj.insert("port".into(), json!(format!("{s}-{e}")));
                    }
                    IRMatcher::ProcessName(name) => {
                        if ir_rule.source == crate::proxy::ir::RuleSource::Override {
                            tracing::warn!("Xray 不支持 YAML 覆写中的进程名分流规则: {name}，已自动跳过");
                            continue;
                        } else {
                            return Err(anyhow!("Xray 暂不支持进程名分流规则: {name}"));
                        }
                    }
                    IRMatcher::Network(proto) => {
                        rule_obj.insert("network".into(), json!(match proto {
                            crate::proxy::NetworkProtocol::Tcp => "tcp",
                            crate::proxy::NetworkProtocol::Udp => "udp",
                        }));
                    }
                    IRMatcher::Inbound(tag) => {
                        rule_obj.insert("inboundTag".into(), json!([tag]));
                    }
                    IRMatcher::Quic => {
                        rule_obj.insert("network".into(), json!("udp"));
                        rule_obj.insert("port".into(), json!("443"));
                    }
                    IRMatcher::RuleSet(tag) => {
                        rule_obj.insert("domain".into(), json!([format!("geosite:{tag}")]));
                    }
                }

                routing_rules.push(Value::Object(rule_obj));
            }
        }

        let fallback_tag = match plan.default_action {
            IRAction::Direct => "direct",
            _ => "proxy",
        };

        routing_rules.push(json!({
            "type": "field",
            "inboundTag": ["local-http"],
            "outboundTag": fallback_tag
        }));

        let config = json!({
            "log": { "loglevel": "warning" },
            "dns": {
                "servers": dns_servers,
                "queryStrategy": "UseIPv4"
            },
            "inbounds": inbounds,
            "outbounds": [
                outbound,
                { "tag": "direct", "protocol": "freedom", "settings": {} },
                { "tag": "blocked", "protocol": "blackhole", "settings": {} }
            ],
            "routing": {
                "domainStrategy": "AsIs",
                "rules": routing_rules
            }
        });

        Ok(CompiledArtifact::Json(config))
    }
}

fn build_outbound(node: &Node, client_fingerprint: Option<&str>) -> Result<Value> {
    let cfg = node.config.as_object().cloned().unwrap_or_default();
    let protocol = node.protocol.to_ascii_lowercase();
    let settings = match protocol.as_str() {
        "ss" | "shadowsocks" => json!({"servers": [{
            "address": node.server,
            "port": node.port,
            "method": required(&cfg, &["cipher", "method"])?,
            "password": required(&cfg, &["password"])?,
            "uot": false
        }]}),
        "vmess" => json!({"vnext": [{
            "address": node.server,
            "port": node.port,
            "users": [{
                "id": required(&cfg, &["uuid", "id"])?,
                "alterId": number(&cfg, &["alterId", "alter-id"]).unwrap_or(0),
                "security": string(&cfg, &["cipher", "security"]).unwrap_or("auto")
            }]
        }]}),
        "vless" => json!({"vnext": [{
            "address": node.server,
            "port": node.port,
            "users": [{
                "id": required(&cfg, &["uuid", "id"])?,
                "encryption": "none",
                "flow": string(&cfg, &["flow"]).unwrap_or("")
            }]
        }]}),
        "trojan" => json!({"servers": [{
            "address": node.server,
            "port": node.port,
            "password": required(&cfg, &["password"])?
        }]}),
        "http" | "https" => json!({"servers": [server_with_users(node, &cfg)]}),
        "socks" | "socks5" => json!({"servers": [server_with_users(node, &cfg)]}),
        _ => return Err(anyhow!("Xray 不支持协议: {}", node.protocol)),
    };
    let xray_protocol = match protocol.as_str() {
        "ss" | "shadowsocks" => "shadowsocks",
        "socks5" => "socks",
        "https" => "http",
        value => value,
    };
    let mut outbound = json!({
        "tag": "proxy",
        "protocol": xray_protocol,
        "settings": settings
    });
    if matches!(protocol.as_str(), "vmess" | "vless" | "trojan") {
        outbound["streamSettings"] = build_stream_settings(&cfg, client_fingerprint);
    }
    Ok(outbound)
}

fn server_with_users(node: &Node, cfg: &Map<String, Value>) -> Value {
    let mut server = json!({"address": node.server, "port": node.port});
    let user = string(cfg, &["username", "user"]);
    let pass = string(cfg, &["password", "pass"]);
    if user.is_some() || pass.is_some() {
        server["users"] = json!([{
            "user": user.unwrap_or(""),
            "pass": pass.unwrap_or("")
        }]);
    }
    server
}

fn build_stream_settings(cfg: &Map<String, Value>, client_fingerprint: Option<&str>) -> Value {
    let network = string(cfg, &["network", "type"]).unwrap_or("tcp");
    let security = string(cfg, &["security"])
        .filter(|value| matches!(*value, "tls" | "reality"))
        .unwrap_or("none");
    let mut stream = json!({"network": network, "security": security});
    if network == "ws" {
        let host = string(cfg, &["host"]);
        stream["wsSettings"] = json!({
            "path": string(cfg, &["path"]).unwrap_or("/"),
            "headers": host.map(|value| json!({"Host": value})).unwrap_or_else(|| json!({}))
        });
    } else if network == "grpc" {
        stream["grpcSettings"] = json!({
            "serviceName": string(cfg, &["serviceName", "service-name"]).unwrap_or("")
        });
    }
    let fp = client_fingerprint.or_else(|| string(cfg, &["client-fingerprint", "fingerprint", "fp"]));
    if security == "tls" {
        let mut tls = json!({
            "serverName": string(cfg, &["sni", "servername", "server-name"]).unwrap_or(""),
            "allowInsecure": false
        });
        if let Some(fp_str) = fp {
            tls["fingerprint"] = json!(fp_str);
        }
        stream["tlsSettings"] = tls;
    } else if security == "reality" {
        let fp_str = fp.unwrap_or("chrome");
        stream["realitySettings"] = json!({
            "serverName": string(cfg, &["sni", "servername", "server-name"]).unwrap_or(""),
            "publicKey": string(cfg, &["public-key", "publicKey", "pbk"]).unwrap_or(""),
            "shortId": string(cfg, &["short-id", "shortId", "sid"]).unwrap_or(""),
            "fingerprint": fp_str
        });
    }
    stream
}

fn required<'a>(cfg: &'a Map<String, Value>, keys: &[&str]) -> Result<&'a str> {
    string(cfg, keys).ok_or_else(|| anyhow!("Xray 配置缺少字段: {}", keys.join("/")))
}

fn string<'a>(cfg: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| cfg.get(*key).and_then(Value::as_str))
}

fn number(cfg: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| cfg.get(*key).and_then(Value::as_u64))
}

use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::kernel::KernelKind;
use crate::models::Node;
use crate::proxy::ir::{IRAction, IRMatcher, RoutingPlan};
use crate::proxy::ProxyMode;
use anyhow::{anyhow, Result};
use serde_yaml::{Mapping, Value};

pub struct ShoesCompiler {
    capabilities: CompilerCapabilities,
}

impl Default for ShoesCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl ShoesCompiler {
    pub fn new() -> Self {
        Self {
            capabilities: CompilerCapabilities {
                supports_tun: false,
                supports_process_name: false,
                supports_domain_keyword: true,
                supports_proxy_groups: true,
                supports_url_test: false,
                supported_geosite_categories: None,
                supported_geoip_countries: None,
            },
        }
    }
}

impl BackendCompiler for ShoesCompiler {
    fn kernel(&self) -> KernelKind {
        KernelKind::Shoes
    }

    fn capabilities(&self) -> &CompilerCapabilities {
        &self.capabilities
    }

    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact> {
        if plan.inbounds.mode == ProxyMode::Tun {
            return Err(anyhow!("Shoes 暂不支持 TUN 模式"));
        }

        let primary_node = plan
            .outbounds
            .primary_node
            .as_ref()
            .ok_or_else(|| anyhow!("Shoes 编译缺少主节点"))?;

        let mut doc = Vec::new();

        // 1. Clash API 控制器配置
        if plan.inbounds.controller_port > 0 {
            let mut clash_api = Mapping::new();
            clash_api.insert(
                Value::String("clash_api_address".into()),
                Value::String(format!("127.0.0.1:{}", plan.inbounds.controller_port)),
            );
            let secret = if plan.inbounds.controller_secret.is_empty() {
                "shoes-internal-secret".to_string()
            } else {
                plan.inbounds.controller_secret.clone()
            };
            clash_api.insert(
                Value::String("secret".into()),
                Value::String(secret),
            );
            clash_api.insert(
                Value::String("cors_origins".into()),
                Value::Sequence(vec![Value::String("*".into())]),
            );
            doc.push(Value::Mapping(clash_api));
        }

        // 2. 主节点出站代理组 (Client Group: outbound)
        let primary_proxy = build_shoes_client_proxy(
            "proxy",
            primary_node,
            plan.outbounds.client_fingerprint.as_deref(),
        )?;

        let mut client_proxies = vec![primary_proxy];

        // 附加直接连接选项
        let mut direct_client = Mapping::new();
        direct_client.insert(
            Value::String("name".into()),
            Value::String("direct".into()),
        );
        let mut direct_proto = Mapping::new();
        direct_proto.insert(
            Value::String("type".into()),
            Value::String("direct".into()),
        );
        direct_client.insert(
            Value::String("protocol".into()),
            Value::Mapping(direct_proto),
        );
        client_proxies.push(Value::Mapping(direct_client));

        let mut outbound_group = Mapping::new();
        outbound_group.insert(
            Value::String("client_group".into()),
            Value::String("PROXY".into()),
        );
        outbound_group.insert(
            Value::String("strategy".into()),
            Value::String("selector".into()),
        );
        outbound_group.insert(
            Value::String("default".into()),
            Value::String("proxy".into()),
        );
        outbound_group.insert(
            Value::String("client_proxies".into()),
            Value::Sequence(client_proxies),
        );
        doc.push(Value::Mapping(outbound_group));

        // 3. 构建本地 Mixed 协议服务与路由规则
        let mut local_server = Mapping::new();
        local_server.insert(
            Value::String("name".into()),
            Value::String("local-mixed".into()),
        );
        local_server.insert(
            Value::String("address".into()),
            Value::String(format!(
                "{}:{}",
                if plan.inbounds.allow_lan {
                    "0.0.0.0"
                } else {
                    "127.0.0.1"
                },
                plan.inbounds.mixed_port
            )),
        );
        let mut mixed_proto = Mapping::new();
        mixed_proto.insert(
            Value::String("type".into()),
            Value::String("mixed".into()),
        );
        mixed_proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));
        local_server.insert(
            Value::String("protocol".into()),
            Value::Mapping(mixed_proto),
        );

        let mut rules = Vec::new();

        // 局域网与回环直连规则（必须设置 no_resolve 防止本地 DNS 污染抢先将境外域名解析为保留/伪造 IP）
        let mut local_rule = Mapping::new();
        local_rule.insert(
            Value::String("masks".into()),
            Value::Sequence(vec![
                Value::String("127.0.0.0/8".into()),
                Value::String("10.0.0.0/8".into()),
                Value::String("172.16.0.0/12".into()),
                Value::String("192.168.0.0/16".into()),
                Value::String("::1/128".into()),
                Value::String("fc00::/7".into()),
                Value::String("fe80::/10".into()),
            ]),
        );
        local_rule.insert(
            Value::String("no_resolve".into()),
            Value::Bool(true),
        );
        local_rule.insert(
            Value::String("action".into()),
            Value::String("allow".into()),
        );
        let mut direct_chain = Mapping::new();
        let mut chain_proto = Mapping::new();
        chain_proto.insert(
            Value::String("type".into()),
            Value::String("direct".into()),
        );
        direct_chain.insert(Value::String("protocol".into()), Value::Mapping(chain_proto));
        local_rule.insert(
            Value::String("client_chain".into()),
            Value::Mapping(direct_chain),
        );
        rules.push(Value::Mapping(local_rule));

        // 将统一 IR 规则转换为 Shoes 规则
        // （translate_ir_rule 会把多谓词规则拆成多条同动作规则，等价 OR 语义）
        for ir_rule in &plan.rules {
            rules.extend(translate_ir_rule(ir_rule));
        }

        // 默认兜底动作
        let fallback_rule = match plan.default_action {
            IRAction::Direct => {
                let mut rule = Mapping::new();
                rule.insert(
                    Value::String("masks".into()),
                    Value::String("0.0.0.0/0".into()),
                );
                rule.insert(
                    Value::String("action".into()),
                    Value::String("allow".into()),
                );
                let mut direct_chain = Mapping::new();
                let mut chain_proto = Mapping::new();
                chain_proto.insert(
                    Value::String("type".into()),
                    Value::String("direct".into()),
                );
                direct_chain.insert(Value::String("protocol".into()), Value::Mapping(chain_proto));
                rule.insert(
                    Value::String("client_chain".into()),
                    Value::Mapping(direct_chain),
                );
                Value::Mapping(rule)
            }
            IRAction::Reject => {
                let mut rule = Mapping::new();
                rule.insert(
                    Value::String("masks".into()),
                    Value::String("0.0.0.0/0".into()),
                );
                rule.insert(
                    Value::String("action".into()),
                    Value::String("block".into()),
                );
                Value::Mapping(rule)
            }
            _ => {
                let mut rule = Mapping::new();
                rule.insert(
                    Value::String("masks".into()),
                    Value::String("0.0.0.0/0".into()),
                );
                rule.insert(
                    Value::String("action".into()),
                    Value::String("allow".into()),
                );
                rule.insert(
                    Value::String("client_chain".into()),
                    Value::String("PROXY".into()),
                );
                Value::Mapping(rule)
            }
        };
        rules.push(fallback_rule);

        local_server.insert(Value::String("rules".into()), Value::Sequence(rules));
        doc.push(Value::Mapping(local_server));

        // 4. 独立端口映射 (Port Mappings)
        for mapping in &plan.inbounds.port_mappings {
            if !mapping.enabled {
                continue;
            }
            let server_name = format!("inbound-{}", mapping.port);
            let mut port_server = Mapping::new();
            port_server.insert(Value::String("name".into()), Value::String(server_name));
            port_server.insert(
                Value::String("address".into()),
                Value::String(format!(
                    "{}:{}",
                    if plan.inbounds.allow_lan {
                        "0.0.0.0"
                    } else {
                        "127.0.0.1"
                    },
                    mapping.port
                )),
            );
            let mut proto = Mapping::new();
            proto.insert(
                Value::String("type".into()),
                Value::String("mixed".into()),
            );
            proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));
            port_server.insert(Value::String("protocol".into()), Value::Mapping(proto));

            let mut port_rule = Mapping::new();
            port_rule.insert(
                Value::String("masks".into()),
                Value::String("0.0.0.0/0".into()),
            );
            port_rule.insert(
                Value::String("action".into()),
                Value::String("allow".into()),
            );
            port_rule.insert(
                Value::String("client_chain".into()),
                Value::String("PROXY".into()),
            );
            port_server.insert(
                Value::String("rules".into()),
                Value::Sequence(vec![Value::Mapping(port_rule)]),
            );
            doc.push(Value::Mapping(port_server));
        }

        let yaml = serde_yaml::to_string(&doc)?;
        Ok(CompiledArtifact::Yaml(yaml))
    }
}

pub(crate) fn find_geo_file(name: &str) -> Option<std::path::PathBuf> {
    let candidate_dirs = [
        crate::geo::mihomo_dir(),
        crate::kernel::managed_kernel_root().join("xray"),
        crate::kernel::managed_kernel_root().join("shoes"),
        std::path::PathBuf::from("data/kernels/mihomo"),
        std::path::PathBuf::from("data/kernels/xray"),
    ];
    for dir in candidate_dirs {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
        let cap_name = if name == "geosite.dat" {
            "GeoSite.dat"
        } else if name == "geoip.dat" {
            "GeoIP.dat"
        } else {
            name
        };
        let p_cap = dir.join(cap_name);
        if p_cap.exists() {
            return Some(p_cap);
        }
    }
    None
}

pub(crate) fn has_geosite_category(code: &str) -> bool {
    if let Some(p) = find_geo_file("geosite.dat") {
        let normalized = crate::geo::normalize_geo_code(code);
        crate::geo::dat_has_category(&p, normalized)
    } else {
        false
    }
}

/// 将统一 IR 规则转为 Shoes 规则映射列表。
///
/// 多谓词（例如 STUN 端口列表 + 端口段）必须拆成多条同动作规则：
/// Shoes 0.3.4 的 `masks: {any: [...]}` 组合 `no_resolve: true` 时存在匹配管线
/// 缺陷——其后所有未被更早规则按域名命中的目标都会被静默 Block（连接被直接关闭，
/// 浏览器表现为 ERR_EMPTY_RESPONSE）。拆分后逐条 first-match，等价保留 OR 语义。
fn translate_ir_rule(rule: &crate::proxy::ir::IRRule) -> Vec<Value> {
    let mut masks_map = Mapping::new();
    let mut legacy_masks: Vec<Value> = Vec::new();
    let mut geo_predicates: Vec<Value> = Vec::new();

    for matcher in &rule.matchers {
        match matcher {
            IRMatcher::Domain(d) => {
                let entry = masks_map
                    .entry(Value::String("domain".into()))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if let Value::Sequence(seq) = entry {
                    seq.push(Value::String(d.clone()));
                }
            }
            IRMatcher::DomainSuffix(ds) => {
                let entry = masks_map
                    .entry(Value::String("domain_suffix".into()))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if let Value::Sequence(seq) = entry {
                    seq.push(Value::String(ds.clone()));
                }
            }
            IRMatcher::DomainKeyword(dk) => {
                let entry = masks_map
                    .entry(Value::String("domain_keyword".into()))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if let Value::Sequence(seq) = entry {
                    seq.push(Value::String(dk.clone()));
                }
            }
            IRMatcher::DomainRegex(dr) => {
                let entry = masks_map
                    .entry(Value::String("domain_regex".into()))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if let Value::Sequence(seq) = entry {
                    seq.push(Value::String(dr.clone()));
                }
            }
            IRMatcher::IpCidr(cidr) => {
                legacy_masks.push(Value::String(cidr.clone()));
            }
            IRMatcher::Port(p) => {
                let entry = masks_map
                    .entry(Value::String("destination_port".into()))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if let Value::Sequence(seq) = entry {
                    seq.push(Value::Number((*p).into()));
                }
            }
            IRMatcher::PortRange(start, end) => {
                let entry = masks_map
                    .entry(Value::String("destination_port_range".into()))
                    .or_insert_with(|| Value::Sequence(Vec::new()));
                if let Value::Sequence(seq) = entry {
                    seq.push(Value::String(format!("{start}:{end}")));
                }
            }
            // QUIC 拦截的语义是"仅拦 UDP 443"（mihomo 侧译成
            // AND,((NETWORK,udp),(DST-PORT,443))）。Shoes 的 masks 没有
            // TCP/UDP 协议维度，裸写 destination_port: 443 会把所有未命中
            // 前置域名规则的 TCP 443（即普通 HTTPS）一起静默拦截，
            // 浏览器表现为 ERR_EMPTY_RESPONSE —— 只能整体跳过这条规则。
            IRMatcher::Quic => {}
            IRMatcher::GeoSite(site) => {
                if let Some(p) = find_geo_file("geosite.dat") {
                    // 订阅惯用别名（LAN→private）先归一；内核对"类别不存在"是
                    // 致命错误（整个配置拒启），再查一遍存在性，查不到的规则
                    // 直接丢弃 —— 该规则本来也永远匹配不上。
                    let code = crate::geo::normalize_geo_code(site);
                    if crate::geo::dat_has_category(&p, code) {
                        let mut m = Mapping::new();
                        m.insert(
                            Value::String("path".into()),
                            Value::String(p.to_string_lossy().replace('\\', "/")),
                        );
                        m.insert(
                            Value::String("format".into()),
                            Value::String("geosite_dat".into()),
                        );
                        m.insert(Value::String("code".into()), Value::String(code.into()));
                        geo_predicates.push(Value::Mapping(m));
                    } else {
                        tracing::warn!(code = %code, "geosite.dat 中不存在该类别，规则已跳过");
                    }
                }
            }
            IRMatcher::GeoIp(country) => {
                if let Some(p) = find_geo_file("geoip.dat") {
                    let code = crate::geo::normalize_geo_code(country);
                    if crate::geo::dat_has_category(&p, code) {
                        let mut m = Mapping::new();
                        m.insert(
                            Value::String("path".into()),
                            Value::String(p.to_string_lossy().replace('\\', "/")),
                        );
                        m.insert(
                            Value::String("format".into()),
                            Value::String("geoip_dat".into()),
                        );
                        m.insert(Value::String("code".into()), Value::String(code.into()));
                        geo_predicates.push(Value::Mapping(m));
                    } else {
                        tracing::warn!(code = %code, "geoip.dat 中不存在该类别，规则已跳过");
                    }
                }
            }
            _ => {}
        }
    }

    let mut predicates = Vec::new();
    predicates.extend(geo_predicates);
    if !legacy_masks.is_empty() {
        if legacy_masks.len() == 1 && masks_map.is_empty() {
            // 单一 IP-CIDR 可以直接作为字面量 mask
            predicates.push(legacy_masks.pop().unwrap());
        } else {
            let mut m = Mapping::new();
            m.insert(
                Value::String("ip_cidr".into()),
                Value::Sequence(legacy_masks),
            );
            predicates.push(Value::Mapping(m));
        }
    }
    for (k, v) in masks_map {
        let mut m = Mapping::new();
        m.insert(k, v);
        predicates.push(Value::Mapping(m));
    }

    let masks_values: Vec<Value> = if predicates.is_empty() {
        Vec::new()
    } else {
        predicates
    };

    if masks_values.is_empty() {
        return Vec::new();
    }

    let build_rule = |masks_val: Value| {
        let mut shoes_rule = Mapping::new();
        shoes_rule.insert(Value::String("masks".into()), masks_val);
        if rule.no_resolve {
            shoes_rule.insert(Value::String("no_resolve".into()), Value::Bool(true));
        }

        match &rule.action {
            IRAction::Reject => {
                shoes_rule.insert(
                    Value::String("action".into()),
                    Value::String("block".into()),
                );
            }
            IRAction::Direct => {
                shoes_rule.insert(
                    Value::String("action".into()),
                    Value::String("allow".into()),
                );
                let mut direct_chain = Mapping::new();
                let mut chain_proto = Mapping::new();
                chain_proto.insert(
                    Value::String("type".into()),
                    Value::String("direct".into()),
                );
                direct_chain.insert(Value::String("protocol".into()), Value::Mapping(chain_proto));
                shoes_rule.insert(
                    Value::String("client_chain".into()),
                    Value::Mapping(direct_chain),
                );
            }
            _ => {
                shoes_rule.insert(
                    Value::String("action".into()),
                    Value::String("allow".into()),
                );
                shoes_rule.insert(
                    Value::String("client_chain".into()),
                    Value::String("PROXY".into()),
                );
            }
        }
        Value::Mapping(shoes_rule)
    };

    masks_values.into_iter().map(build_rule).collect()
}

/// 将 Node 构造为 Shoes client_proxy 配置
pub fn build_shoes_client_proxy(
    name: &str,
    node: &Node,
    client_fingerprint: Option<&str>,
) -> Result<Value> {
    let mut map = Mapping::new();
    map.insert(Value::String("name".into()), Value::String(name.into()));
    map.insert(
        Value::String("address".into()),
        Value::String(format!("{}:{}", node.server, node.port)),
    );

    let protocol_value = build_protocol_layer(node, client_fingerprint)?;
    map.insert(Value::String("protocol".into()), protocol_value);

    Ok(Value::Mapping(map))
}

fn build_protocol_layer(node: &Node, client_fingerprint: Option<&str>) -> Result<Value> {
    let cfg = node.config.as_object().cloned().unwrap_or_default();
    let protocol = node.protocol.to_ascii_lowercase();

    match protocol.as_str() {
        "ss" | "shadowsocks" => {
            let cipher = string_from_cfg(&cfg, &["cipher", "method"])
                .unwrap_or_else(|| "aes-128-gcm".into());
            let password = string_from_cfg(&cfg, &["password"])
                .ok_or_else(|| anyhow!("Shadowsocks 缺少 password"))?;
            let mut proto = Mapping::new();
            proto.insert(
                Value::String("type".into()),
                Value::String("shadowsocks".into()),
            );
            proto.insert(Value::String("cipher".into()), Value::String(cipher));
            proto.insert(Value::String("password".into()), Value::String(password));
            proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));
            Ok(Value::Mapping(proto))
        }
        "snell" => {
            let cipher = string_from_cfg(&cfg, &["cipher"])
                .unwrap_or_else(|| "aes-128-gcm".into());
            let password = string_from_cfg(&cfg, &["password"])
                .ok_or_else(|| anyhow!("Snell 缺少 password"))?;
            let mut proto = Mapping::new();
            proto.insert(
                Value::String("type".into()),
                Value::String("snell".into()),
            );
            proto.insert(Value::String("cipher".into()), Value::String(cipher));
            proto.insert(Value::String("password".into()), Value::String(password));
            proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));
            Ok(Value::Mapping(proto))
        }
        "vmess" => {
            let user_id = string_from_cfg(&cfg, &["uuid", "id"])
                .ok_or_else(|| anyhow!("VMess 缺少 uuid"))?;
            let raw_cipher = string_from_cfg(&cfg, &["cipher", "security"])
                .unwrap_or_else(|| "auto".into());
            let cipher = if raw_cipher == "auto" || raw_cipher.is_empty() {
                "aes-128-gcm"
            } else {
                raw_cipher.as_str()
            };

            let mut vmess_proto = Mapping::new();
            vmess_proto.insert(
                Value::String("type".into()),
                Value::String("vmess".into()),
            );
            vmess_proto.insert(
                Value::String("cipher".into()),
                Value::String(cipher.into()),
            );
            vmess_proto.insert(Value::String("user_id".into()), Value::String(user_id));
            vmess_proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));

            wrap_transport(vmess_proto, &cfg, &node.server, client_fingerprint)
        }
        "vless" => {
            let user_id = string_from_cfg(&cfg, &["uuid", "id"])
                .ok_or_else(|| anyhow!("VLESS 缺少 uuid"))?;

            let mut vless_proto = Mapping::new();
            vless_proto.insert(
                Value::String("type".into()),
                Value::String("vless".into()),
            );
            vless_proto.insert(Value::String("user_id".into()), Value::String(user_id));
            vless_proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));

            wrap_transport(vless_proto, &cfg, &node.server, client_fingerprint)
        }
        "trojan" => {
            let password = string_from_cfg(&cfg, &["password"])
                .ok_or_else(|| anyhow!("Trojan 缺少 password"))?;

            let mut trojan_proto = Mapping::new();
            trojan_proto.insert(
                Value::String("type".into()),
                Value::String("trojan".into()),
            );
            trojan_proto.insert(Value::String("password".into()), Value::String(password));

            let security = string_from_cfg(&cfg, &["security"]).unwrap_or_else(|| "tls".into());
            let mut cfg_with_sec = cfg.clone();
            if security != "none" {
                cfg_with_sec.insert("security".into(), serde_json::Value::String("tls".into()));
            }
            wrap_transport(trojan_proto, &cfg_with_sec, &node.server, client_fingerprint)
        }
        "http" | "https" => {
            let mut http_proto = Mapping::new();
            http_proto.insert(
                Value::String("type".into()),
                Value::String("http".into()),
            );
            if let Some(user) = string_from_cfg(&cfg, &["username", "user"]) {
                http_proto.insert(Value::String("username".into()), Value::String(user));
            }
            if let Some(pass) = string_from_cfg(&cfg, &["password", "pass"]) {
                http_proto.insert(Value::String("password".into()), Value::String(pass));
            }

            if protocol == "https" {
                let mut tls_proto = Mapping::new();
                tls_proto.insert(
                    Value::String("type".into()),
                    Value::String("tls".into()),
                );
                let sni = string_from_cfg(&cfg, &["sni", "servername"])
                    .unwrap_or_else(|| node.server.clone());
                tls_proto.insert(Value::String("sni_hostname".into()), Value::String(sni));
                tls_proto.insert(
                    Value::String("protocol".into()),
                    Value::Mapping(http_proto),
                );
                Ok(Value::Mapping(tls_proto))
            } else {
                Ok(Value::Mapping(http_proto))
            }
        }
        "socks" | "socks5" => {
            let mut socks_proto = Mapping::new();
            socks_proto.insert(
                Value::String("type".into()),
                Value::String("socks".into()),
            );
            if let Some(user) = string_from_cfg(&cfg, &["username", "user"]) {
                socks_proto.insert(Value::String("username".into()), Value::String(user));
            }
            if let Some(pass) = string_from_cfg(&cfg, &["password", "pass"]) {
                socks_proto.insert(Value::String("password".into()), Value::String(pass));
            }
            Ok(Value::Mapping(socks_proto))
        }
        "anytls" => {
            let password = string_from_cfg(&cfg, &["password"])
                .ok_or_else(|| anyhow!("AnyTLS 缺少 password"))?;
            let mut proto = Mapping::new();
            proto.insert(
                Value::String("type".into()),
                Value::String("anytls".into()),
            );
            proto.insert(Value::String("password".into()), Value::String(password));
            proto.insert(Value::String("udp_enabled".into()), Value::Bool(true));
            Ok(Value::Mapping(proto))
        }
        "naiveproxy" => {
            let username = string_from_cfg(&cfg, &["username", "user"])
                .ok_or_else(|| anyhow!("NaiveProxy 缺少 username"))?;
            let password = string_from_cfg(&cfg, &["password", "pass"])
                .ok_or_else(|| anyhow!("NaiveProxy 缺少 password"))?;
            let sni = string_from_cfg(&cfg, &["sni", "servername"])
                .unwrap_or_else(|| node.server.clone());

            let mut naive_proto = Mapping::new();
            naive_proto.insert(
                Value::String("type".into()),
                Value::String("naiveproxy".into()),
            );
            naive_proto.insert(Value::String("username".into()), Value::String(username));
            naive_proto.insert(Value::String("password".into()), Value::String(password));
            naive_proto.insert(Value::String("padding".into()), Value::Bool(true));

            let mut tls_proto = Mapping::new();
            tls_proto.insert(
                Value::String("type".into()),
                Value::String("tls".into()),
            );
            tls_proto.insert(Value::String("sni_hostname".into()), Value::String(sni));
            tls_proto.insert(
                Value::String("protocol".into()),
                Value::Mapping(naive_proto),
            );
            Ok(Value::Mapping(tls_proto))
        }
        _ => Err(anyhow!("Shoes 暂不支持协议: {}", node.protocol)),
    }
}

fn wrap_transport(
    inner: Mapping,
    cfg: &serde_json::Map<String, serde_json::Value>,
    server: &str,
    _client_fingerprint: Option<&str>,
) -> Result<Value> {
    let network = string_from_cfg(cfg, &["network", "type"]).unwrap_or_else(|| "tcp".into());

    let reality_opts_pbk = cfg
        .get("reality-opts")
        .or_else(|| cfg.get("reality_opts"))
        .and_then(|opts| {
            opts.get("public-key")
                .or_else(|| opts.get("publicKey"))
                .or_else(|| opts.get("pbk"))
        })
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let direct_pbk = string_from_cfg(cfg, &["pbk", "public-key", "publicKey"]);

    let has_reality = string_from_cfg(cfg, &["security"]).as_deref() == Some("reality")
        || reality_opts_pbk.is_some()
        || direct_pbk.is_some();

    let security = if has_reality {
        "reality".to_string()
    } else if cfg.get("tls").and_then(|v| v.as_bool()).unwrap_or(false)
        || string_from_cfg(cfg, &["security"]).as_deref() == Some("tls")
    {
        "tls".to_string()
    } else {
        "none".to_string()
    };
    let sni = string_from_cfg(cfg, &["sni", "servername", "server-name"])
        .or_else(|| {
            cfg.get("reality-opts")
                .or_else(|| cfg.get("reality_opts"))
                .and_then(|opts| opts.get("sni").or_else(|| opts.get("servername")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| server.to_string());

    let mut current_proto = Value::Mapping(inner);

    // 1. 内层流传输协议 (如 WebSocket)
    if network == "ws" {
        let (ws_opts_path, ws_opts_headers) = if let Some(serde_json::Value::Object(opts)) =
            cfg.get("ws-opts").or_else(|| cfg.get("ws_opts"))
        {
            let p = opts
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let h = opts.get("headers").and_then(|v| v.as_object()).cloned();
            (p, h)
        } else {
            (None, None)
        };

        let path = string_from_cfg(cfg, &["path", "ws-path"])
            .or(ws_opts_path)
            .unwrap_or_else(|| "/".into());

        let host = string_from_cfg(cfg, &["host", "sni", "servername", "server-name"])
            .unwrap_or_else(|| server.to_string());

        let mut ws_map = Mapping::new();
        ws_map.insert(Value::String("type".into()), Value::String("ws".into()));
        ws_map.insert(
            Value::String("matching_path".into()),
            Value::String(path),
        );

        let mut headers = Mapping::new();
        headers.insert(Value::String("Host".into()), Value::String(host));
        if let Some(h_obj) = ws_opts_headers.or_else(|| {
            cfg.get("headers")
                .or_else(|| cfg.get("ws-headers"))
                .and_then(|v| v.as_object().cloned())
        }) {
            for (k, v) in h_obj {
                if let Some(s) = v.as_str() {
                    headers.insert(Value::String(k.clone()), Value::String(s.to_string()));
                }
            }
        }
        ws_map.insert(
            Value::String("matching_headers".into()),
            Value::Mapping(headers),
        );
        ws_map.insert(Value::String("protocol".into()), current_proto);
        current_proto = Value::Mapping(ws_map);
    }

    // 2. 外层加密协议 (TLS / REALITY)
    if security == "reality" {
        let pbk = direct_pbk
            .or(reality_opts_pbk)
            .ok_or_else(|| anyhow!("Reality 缺少 public-key / pbk"))?;

        let sid = string_from_cfg(cfg, &["sid", "short-id", "shortId"])
            .or_else(|| {
                cfg.get("reality-opts")
                    .or_else(|| cfg.get("reality_opts"))
                    .and_then(|opts| {
                        opts.get("short-id")
                            .or_else(|| opts.get("shortId"))
                            .or_else(|| opts.get("sid"))
                    })
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        let vision = string_from_cfg(cfg, &["flow"])
            .map(|f| f.contains("vision"))
            .unwrap_or(false);

        let mut reality_map = Mapping::new();
        reality_map.insert(
            Value::String("type".into()),
            Value::String("reality".into()),
        );
        reality_map.insert(Value::String("public_key".into()), Value::String(pbk));
        reality_map.insert(Value::String("short_id".into()), Value::String(sid));
        reality_map.insert(Value::String("sni_hostname".into()), Value::String(sni));
        reality_map.insert(Value::String("vision".into()), Value::Bool(vision));
        reality_map.insert(Value::String("protocol".into()), current_proto);
        current_proto = Value::Mapping(reality_map);
    } else if security == "tls" {
        let vision = string_from_cfg(cfg, &["flow"])
            .map(|f| f.contains("vision"))
            .unwrap_or(false);

        let mut tls_map = Mapping::new();
        tls_map.insert(
            Value::String("type".into()),
            Value::String("tls".into()),
        );
        tls_map.insert(Value::String("sni_hostname".into()), Value::String(sni));
        tls_map.insert(Value::String("vision".into()), Value::Bool(vision));
        if let Some(val) = cfg.get("skip-cert-verify").or_else(|| cfg.get("allowInsecure")) {
            if let Some(b) = val.as_bool() {
                if b {
                    tls_map.insert(Value::String("verify".into()), Value::Bool(false));
                }
            }
        }

        let mut alpn_list = Vec::new();
        if let Some(serde_json::Value::Array(arr)) = cfg.get("alpn") {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if !s.is_empty() {
                        alpn_list.push(Value::String(s.to_string()));
                    }
                }
            }
        } else if let Some(serde_json::Value::String(s)) = cfg.get("alpn") {
            for part in s.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    alpn_list.push(Value::String(trimmed.to_string()));
                }
            }
        }
        if alpn_list.is_empty() && network == "ws" {
            alpn_list.push(Value::String("http/1.1".into()));
        }
        if !alpn_list.is_empty() {
            tls_map.insert(
                Value::String("alpn_protocols".into()),
                Value::Sequence(alpn_list),
            );
        }

        tls_map.insert(Value::String("protocol".into()), current_proto);
        current_proto = Value::Mapping(tls_map);
    }

    Ok(current_proto)
}

fn string_from_cfg(
    cfg: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    for key in keys {
        if let Some(serde_json::Value::String(val)) = cfg.get(*key) {
            if !val.is_empty() {
                return Some(val.clone());
            }
        }
    }
    None
}

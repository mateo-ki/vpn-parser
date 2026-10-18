use super::{
    MihomoConfigOptions, NetworkProtocol, ProxyMode, RoutingMode, RuleAction, RuleMatcher,
};
use crate::kernel::{find_kernel_executable, KernelKind};
use crate::models::Node;
use crate::proxy_probe::{
    ProxyProbeProgress, ProxyProbeResult, PROXY_CONNECTIVITY_FALLBACK_URL,
    PROXY_CONNECTIVITY_TEST_URL,
};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

const GEOSITE_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs";
const GEOSITE_ADS_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ads-all.srs";
const GEOSITE_GFW_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-gfw.srs";
const GEOIP_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs";
const SING_BOX_BATCH_SIZE: usize = 64;

fn sing_box_batch_count(node_count: usize) -> usize {
    node_count.div_ceil(SING_BOX_BATCH_SIZE)
}

pub async fn verify_nodes_with_sing_box_progress_limited(
    nodes: Vec<Node>,
    timeout: Duration,
    progress: Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    network_limit: Arc<Semaphore>,
) -> Result<Vec<ProxyProbeResult>> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let binary = find_kernel_executable(KernelKind::SingBox)
        .ok_or_else(|| anyhow!("未找到 Sing-box 可执行文件"))?;
    let total = nodes.len();
    let started = tokio::time::Instant::now();
    let mut results = Vec::with_capacity(total);
    let batch_total = sing_box_batch_count(nodes.len());
    for (batch_index, batch) in nodes.chunks(SING_BOX_BATCH_SIZE).enumerate() {
        match probe_batch_with_sing_box(&binary, batch.to_vec(), timeout, network_limit.clone())
            .await
        {
            Ok(mut batch_results) => results.append(&mut batch_results),
            Err(error) => results.extend(batch.iter().map(|node| ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Sing-box 测速批次失败: {error}"),
            })),
        }
        if let Some(progress) = progress.as_ref() {
            let ok = results.iter().filter(|result| result.ok).count();
            progress(ProxyProbeProgress {
                done: results.len(),
                total,
                ok,
                fail: results.len().saturating_sub(ok),
                elapsed: started.elapsed(),
                batch_index: Some(batch_index + 1),
                batch_total: Some(batch_total),
            });
        }
    }
    Ok(results)
}

async fn probe_batch_with_sing_box(
    binary: &Path,
    nodes: Vec<Node>,
    timeout: Duration,
    network_limit: Arc<Semaphore>,
) -> Result<Vec<ProxyProbeResult>> {
    let mut ports = Vec::with_capacity(nodes.len());
    let mut reservations = Vec::with_capacity(nodes.len());
    for _ in &nodes {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        ports.push(listener.local_addr()?.port());
        reservations.push(listener);
    }
    let mut inbounds = Vec::new();
    let mut outbounds = vec![json!({ "type": "direct", "tag": "direct" })];
    let mut endpoints = Vec::new();
    let mut rules = Vec::new();
    let mut immediate = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let options = MihomoConfigOptions {
            mixed_port: ports[index],
            ..Default::default()
        };
        match build_sing_box_config(node, options) {
            Ok(config) => {
                let inbound_tag = format!("probe-in-{index}");
                let outbound_tag = format!("probe-out-{index}");
                let mut inbound = config["inbounds"][0].clone();
                inbound["tag"] = json!(inbound_tag);
                inbounds.push(inbound);
                if let Some(endpoint) = config
                    .get("endpoints")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                {
                    let mut endpoint = endpoint.clone();
                    endpoint["tag"] = json!(outbound_tag);
                    endpoints.push(endpoint);
                } else if let Some(outbound) = config
                    .get("outbounds")
                    .and_then(Value::as_array)
                    .and_then(|items| items.first())
                {
                    let mut outbound = outbound.clone();
                    outbound["tag"] = json!(outbound_tag);
                    outbounds.push(outbound);
                }
                rules.push(json!({
                    "inbound": [inbound_tag], "action": "route", "outbound": outbound_tag
                }));
            }
            Err(error) => immediate.push(ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Sing-box 配置不支持: {error}"),
            }),
        }
    }
    if inbounds.is_empty() {
        return Ok(immediate);
    }
    let ready_port = inbounds[0]["listen_port"]
        .as_u64()
        .unwrap_or(ports[0] as u64) as u16;
    drop(reservations);
    let mut config = json!({
        "log": {"level":"warning","timestamp":true},
        "dns": build_dns(&MihomoConfigOptions::default())?,
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": {"auto_detect_interface":true,"default_domain_resolver":"local","rules":rules,"final":"direct"}
    });
    if !endpoints.is_empty() {
        config["endpoints"] = Value::Array(endpoints);
    }
    let directory =
        std::env::temp_dir().join(format!("vpn-parser-sing-box-batch-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await?;
    let config_path = directory.join("config.json");
    let log_path = directory.join("sing-box.log");
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&config)?).await?;
    let log = std::fs::File::create(&log_path)?;
    let stderr = log.try_clone()?;
    let mut command = Command::new(binary);
    command
        .arg("run")
        .arg("-D")
        .arg(&directory)
        .arg("-c")
        .arg(&config_path)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn()?;
    if let Some(pid) = child.id() {
        crate::platform::assign_child_process_id(pid);
    }
    if !wait_for_sing_box_port(&mut child, ready_port).await {
        let _ = child.kill().await;
        let detail = tokio::fs::read_to_string(&log_path)
            .await
            .unwrap_or_default();
        let _ = tokio::fs::remove_dir_all(&directory).await;
        return Err(anyhow!("启动失败: {}", last_sing_box_log_line(&detail)));
    }
    let mut tasks = JoinSet::new();
    for (index, node) in nodes.into_iter().enumerate() {
        let port = ports[index];
        let network_limit = network_limit.clone();
        tasks.spawn(async move {
            let _permit = network_limit
                .acquire_owned()
                .await
                .map_err(|_| anyhow!("测速并发控制已关闭"));
            let started = tokio::time::Instant::now();
            let outcome = match _permit {
                Err(error) => Err(error.to_string()),
                Ok(_permit) => match reqwest::Proxy::all(format!("http://127.0.0.1:{port}")) {
                    Ok(proxy) => match reqwest::Client::builder()
                        .proxy(proxy)
                        .timeout(timeout)
                        .build()
                    {
                        Ok(client) => {
                            let mut probe = probe_connectivity_target(
                                &client,
                                PROXY_CONNECTIVITY_TEST_URL,
                                "Google",
                            )
                            .await;
                            if probe.is_err() {
                                // 交叉探活：Google 遇阻或偶发单次抖动时，走 Cloudflare 交叉复验
                                if let Ok(cf) = probe_connectivity_target(
                                    &client,
                                    PROXY_CONNECTIVITY_FALLBACK_URL,
                                    "Cloudflare",
                                )
                                .await
                                {
                                    probe = Ok(cf);
                                }
                            }
                            probe
                        }
                        Err(error) => Err(error.to_string()),
                    },
                    Err(error) => Err(error.to_string()),
                },
            };
            match outcome {
                Ok(target) => {
                    let latency = started.elapsed().as_millis().max(1) as i64;
                    ProxyProbeResult {
                        id: node.id,
                        ok: true,
                        latency_ms: Some(latency),
                        message: format!("{latency} ms (Sing-box 真实代理 · {target})"),
                    }
                }
                Err(error) => ProxyProbeResult {
                    id: node.id,
                    ok: false,
                    latency_ms: None,
                    message: format!("Sing-box 真实代理测试失败: {error}"),
                },
            }
        });
    }
    let mut results = immediate;
    while let Some(joined) = tasks.join_next().await {
        if let Ok(result) = joined {
            results.push(result);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = tokio::fs::remove_dir_all(directory).await;
    Ok(results)
}

async fn probe_connectivity_target(
    client: &reqwest::Client,
    target_url: &'static str,
    target_name: &'static str,
) -> std::result::Result<&'static str, String> {
    let response = client
        .get(target_url)
        .send()
        .await
        .map_err(|error| format!("{target_name}: {error}"))?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        Ok(target_name)
    } else {
        Err(format!("{target_name}: HTTP {}", response.status()))
    }
}

async fn wait_for_sing_box_port(child: &mut Child, port: u16) -> bool {
    let started = tokio::time::Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn last_sing_box_log_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .next_back()
        .unwrap_or("端口未就绪且没有日志")
}

pub fn build_sing_box_config(node: &Node, options: MihomoConfigOptions) -> Result<Value> {
    options.validate()?;
    let wireguard = matches!(
        node.protocol.to_ascii_lowercase().as_str(),
        "wireguard" | "wg"
    );
    let proxy = if wireguard {
        build_wireguard_endpoint(node)?
    } else {
        build_proxy_outbound(node)?
    };
    let mut inbounds = vec![json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": if options.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
        "listen_port": options.mixed_port
    })];
    if options.mode == ProxyMode::Tun {
        inbounds.push(json!({
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30"],
            "auto_route": options.tun.auto_route,
            "strict_route": true,
            "stack": sing_box_tun_stack(options.tun.stack)
        }));
    }

    let mut routing_options = options.clone();
    routing_options.rules = match options.routing_mode {
        RoutingMode::Rule => options.rules.clone(),
        RoutingMode::ChinaDirect => super::recommended_traffic_rules(),
        RoutingMode::GfwList => super::gfwlist_traffic_rules(),
        RoutingMode::Global | RoutingMode::Direct => Vec::new(),
    };
    let (mut route_rules, rule_sets) = build_route_rules(&routing_options)?;
    for mapping in &options.port_mappings {
        if !mapping.enabled {
            continue;
        }
        let in_tag = format!("inbound-{}", mapping.port);
        inbounds.push(json!({
            "type": "mixed",
            "tag": in_tag.clone(),
            "listen": if options.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
            "listen_port": mapping.port
        }));
        route_rules.insert(
            0,
            json!({
                "inbound": [in_tag],
                "action": "route",
                "outbound": "proxy"
            }),
        );
    }
    if options.mode == ProxyMode::Tun {
        route_rules.insert(
            0,
            json!({
                "protocol": "dns",
                "action": "hijack-dns"
            }),
        );
    }

    let mut route = Map::new();
    route.insert("auto_detect_interface".into(), json!(true));
    route.insert("default_domain_resolver".into(), json!("local"));
    route.insert("rules".into(), Value::Array(route_rules));
    route.insert(
        "final".into(),
        json!(match options.routing_mode {
            RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::Global => "proxy",
            RoutingMode::GfwList | RoutingMode::Direct => "direct",
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
        "dns": build_dns(&options)?,
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
    Ok(config)
}

fn build_dns(options: &MihomoConfigOptions) -> Result<Value> {
    let mut servers = vec![json!({ "type": "local", "tag": "local" })];
    for (index, server) in options.dns.nameservers.iter().enumerate() {
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
        "strategy": if options.dns.ipv6 { "prefer_ipv4" } else { "ipv4_only" }
    }))
}

fn build_proxy_outbound(node: &Node) -> Result<Value> {
    let protocol = node.protocol.to_ascii_lowercase();
    match protocol.as_str() {
        "hysteria2" | "hy2" => build_hysteria2(node),
        "tuic" => build_tuic(node),
        "anytls" => build_anytls(node),
        _ => Err(anyhow!("Sing-box 暂不支持协议转换: {}", node.protocol)),
    }
}

fn build_hysteria2(node: &Node) -> Result<Value> {
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
    outbound.insert("tls".into(), tls_config(node));
    Ok(Value::Object(outbound))
}

fn build_tuic(node: &Node) -> Result<Value> {
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
    outbound.insert("tls".into(), tls_config(node));
    Ok(Value::Object(outbound))
}

fn build_anytls(node: &Node) -> Result<Value> {
    let password = required_string(&node.config, &["password"])?;
    let mut outbound = common_outbound("anytls", node);
    outbound.insert("password".into(), json!(password));
    outbound.insert("tls".into(), tls_config(node));
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
    let allowed_ips = string_list_value(&node.config, &["allowed-ips", "allowed_ips"])
        .unwrap_or_else(|| vec!["0.0.0.0/0".into(), "::/0".into()]);
    let mut peer = Map::new();
    peer.insert("address".into(), json!(node.server));
    peer.insert("port".into(), json!(node.port));
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

fn tls_config(node: &Node) -> Value {
    let server_name = string_value(&node.config, &["sni", "server_name"])
        .filter(|value| !value.is_empty())
        .unwrap_or(&node.server);
    // 远端订阅里的 skip-cert-verify/insecure 一律不采信：那等于让来源自己决定
    // 要不要被中间人。实测现有节点没有一个依赖它。
    let insecure = false;
    let mut tls = Map::new();
    tls.insert("enabled".into(), json!(true));
    tls.insert("server_name".into(), json!(server_name));
    tls.insert("insecure".into(), json!(insecure));
    if let Some(alpn) = node.config.get("alpn").and_then(Value::as_array) {
        tls.insert("alpn".into(), Value::Array(alpn.clone()));
    }
    Value::Object(tls)
}

fn build_route_rules(options: &MihomoConfigOptions) -> Result<(Vec<Value>, Vec<Value>)> {
    let selected_rules = match options.routing_mode {
        RoutingMode::Rule => super::optimize_traffic_rules(&options.rules),
        RoutingMode::ChinaDirect => super::recommended_traffic_rules(),
        RoutingMode::GfwList => super::gfwlist_traffic_rules(),
        RoutingMode::Global | RoutingMode::Direct => Vec::new(),
    };
    let mut rules = Vec::with_capacity(selected_rules.len());
    let mut rule_sets = Vec::new();
    let mut seen_rule_sets = HashSet::new();
    for rule in &selected_rules {
        let action = rule_action(rule.action);
        let value = match &rule.matcher {
            RuleMatcher::Domain(value) => {
                json!({ "domain": [value], "action": "route", "outbound": action })
            }
            RuleMatcher::DomainSuffix(value) => {
                json!({ "domain_suffix": [value], "action": "route", "outbound": action })
            }
            RuleMatcher::IpCidr(value) => {
                json!({ "ip_cidr": [value], "action": "route", "outbound": action })
            }
            RuleMatcher::DestinationPort(value) => {
                let port = value
                    .parse::<u16>()
                    .map_err(|_| anyhow!("Sing-box 端口规则无效: {value}"))?;
                json!({ "port": [port], "action": "route", "outbound": action })
            }
            RuleMatcher::ProcessName(value) => {
                json!({ "process_name": [value], "action": "route", "outbound": action })
            }
            RuleMatcher::Network(protocol) => json!({
                "network": match protocol { NetworkProtocol::Tcp => "tcp", NetworkProtocol::Udp => "udp" },
                "action": "route",
                "outbound": action
            }),
            RuleMatcher::Quic => json!({ "network": "udp", "port": [443], "action": "reject" }),
            RuleMatcher::GeoSite(value) => {
                let (tag, url) = match value.to_ascii_lowercase().as_str() {
                    "cn" => ("geosite-cn", GEOSITE_CN_URL),
                    "category-ads-all" => ("geosite-category-ads-all", GEOSITE_ADS_URL),
                    "gfw" => ("geosite-gfw", GEOSITE_GFW_URL),
                    _ => return Err(anyhow!("Sing-box 暂不支持 GeoSite 分类: {value}")),
                };
                push_rule_set(&mut rule_sets, &mut seen_rule_sets, tag, url);
                json!({ "rule_set": [tag], "action": "route", "outbound": action })
            }
            RuleMatcher::GeoIp(value) => {
                if !value.eq_ignore_ascii_case("CN") {
                    return Err(anyhow!("Sing-box 暂不支持 GeoIP 分类: {value}"));
                }
                push_rule_set(
                    &mut rule_sets,
                    &mut seen_rule_sets,
                    "geoip-cn",
                    GEOIP_CN_URL,
                );
                json!({ "rule_set": ["geoip-cn"], "action": "route", "outbound": action })
            }
            RuleMatcher::RuleSet(tag) => {
                json!({ "rule_set": [tag], "action": "route", "outbound": action })
            }
        };
        rules.push(if rule.action == RuleAction::Reject {
            with_reject_action(value)
        } else {
            value
        });
    }
    Ok((rules, rule_sets))
}

fn rule_action(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Direct => "direct",
        RuleAction::Proxy => "proxy",
        RuleAction::Reject => "",
    }
}

fn with_reject_action(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("action".into(), json!("reject"));
        object.remove("outbound");
    }
    value
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

fn sing_box_tun_stack(stack: super::TunStack) -> &'static str {
    match stack {
        super::TunStack::System => "system",
        super::TunStack::Gvisor => "gvisor",
        super::TunStack::Mixed => "mixed",
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
    keys.iter().find_map(|key| {
        let value = config.get(*key)?;
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{find_kernel_executable, KernelKind};
    use chrono::Utc;
    use std::process::Command;
    use uuid::Uuid;

    fn node(protocol: &str, config: Value) -> Node {
        Node {
            id: Uuid::new_v4().to_string(),
            source_id: None,
            protocol: protocol.into(),
            name: protocol.into(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn builds_hysteria2_tuic_and_anytls_outbounds() {
        let cases = [
            node(
                "hysteria2",
                json!({
                    "server": "1.2.3.4", "port": 443, "password": "secret",
                    "obfs": "salamander", "obfs-password": "obfs", "sni": "example.com",
                    "insecure": "1"
                }),
            ),
            node(
                "tuic",
                json!({
                    "server": "tuic.example.com", "port": 443, "uuid": Uuid::nil().to_string(),
                    "password": "secret", "congestion-controller": "cubic", "sni": "tuic.example.com"
                }),
            ),
            node(
                "anytls",
                json!({
                    "server": "anytls.example.com", "port": 443, "password": "secret",
                    "skip-cert-verify": true
                }),
            ),
            node(
                "wireguard",
                json!({
                    "server": "1.2.3.4", "port": 51820,
                    "ip": "172.16.0.2/32",
                    "private-key": "EFlemF/JAgdqTmKVb/1W/yv9WnwuzhdqLfDHUUNC11w=",
                    "public-key": "TepNujm+9RJYXvQTB3CGDCnSccm05k6p/NmaHhj1C2c=",
                    "reserved": [0, 0, 0]
                }),
            ),
        ];
        for node in cases {
            let config = build_sing_box_config(&node, MihomoConfigOptions::default()).unwrap();
            if node.protocol == "wireguard" {
                assert_eq!(config["endpoints"][0]["tag"], "proxy");
            } else {
                assert_eq!(config["outbounds"][0]["tag"], "proxy");
            }
            assert_eq!(config["inbounds"][0]["type"], "mixed");
            assert_eq!(config["route"]["final"], "proxy");
            assert_eq!(config["route"]["default_domain_resolver"], "local");
        }
    }

    #[test]
    fn sing_box_routing_presets_use_remote_rules_and_expected_fallback() {
        let selected = node(
            "hysteria2",
            json!({
                "server":"1.2.3.4",
                "port":443,
                "password":"p",
                "sni":"example.com"
            }),
        );
        let china = build_sing_box_config(
            &selected,
            MihomoConfigOptions {
                routing_mode: RoutingMode::ChinaDirect,
                ..MihomoConfigOptions::default()
            },
        )
        .unwrap();
        let gfw = build_sing_box_config(
            &selected,
            MihomoConfigOptions {
                routing_mode: RoutingMode::GfwList,
                ..MihomoConfigOptions::default()
            },
        )
        .unwrap();

        assert_eq!(china["route"]["final"], "proxy");
        assert!(china["route"]["rule_set"].as_array().unwrap().len() >= 2);
        assert_eq!(gfw["route"]["final"], "direct");
        assert!(gfw["route"]["rule_set"]
            .as_array()
            .unwrap()
            .iter()
            .any(|set| set["tag"] == "geosite-gfw"));
    }

    #[test]
    fn sing_box_uses_one_process_per_sixty_four_nodes() {
        assert_eq!(sing_box_batch_count(0), 0);
        assert_eq!(sing_box_batch_count(64), 1);
        assert_eq!(sing_box_batch_count(65), 2);
        assert_eq!(sing_box_batch_count(128), 2);
    }

    #[test]
    fn installed_sing_box_accepts_generated_configs() {
        let Some(binary) = find_kernel_executable(KernelKind::SingBox) else {
            return;
        };
        let cases = [
            node(
                "hysteria2",
                json!({
                    "server": "hy2.example.com", "port": 443, "password": "secret",
                    "obfs": "salamander", "obfs-password": "obfs", "sni": "hy2.example.com"
                }),
            ),
            node(
                "tuic",
                json!({
                    "server": "tuic.example.com", "port": 443, "uuid": Uuid::nil().to_string(),
                    "password": "secret", "sni": "tuic.example.com"
                }),
            ),
            node(
                "anytls",
                json!({
                    "server": "anytls.example.com", "port": 443, "password": "secret",
                    "skip-cert-verify": true
                }),
            ),
            node(
                "wireguard",
                json!({
                    "server": "1.2.3.4", "port": 51820,
                    "ip": "172.16.0.2/32",
                    "private-key": "EFlemF/JAgdqTmKVb/1W/yv9WnwuzhdqLfDHUUNC11w=",
                    "public-key": "TepNujm+9RJYXvQTB3CGDCnSccm05k6p/NmaHhj1C2c=",
                    "reserved": [0, 0, 0]
                }),
            ),
        ];
        for node in cases {
            let config = build_sing_box_config(&node, MihomoConfigOptions::default()).unwrap();
            let directory = std::env::temp_dir().join(format!(
                "vpn-parser-sing-box-check-{}-{}",
                node.protocol,
                Uuid::new_v4()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("config.json");
            std::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
            let output = Command::new(&binary)
                .arg("check")
                .arg("-c")
                .arg(&path)
                .output()
                .unwrap();
            let detail = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let _ = std::fs::remove_dir_all(directory);
            assert!(output.status.success(), "{}: {detail}", node.protocol);
        }
    }

    #[tokio::test]
    async fn empty_sing_box_probe_returns_empty() {
        let results = verify_nodes_with_sing_box_progress_limited(
            Vec::new(),
            std::time::Duration::from_secs(1),
            None::<fn(ProxyProbeProgress)>,
            Arc::new(Semaphore::new(1)),
        )
        .await
        .unwrap();

        assert!(results.is_empty());
    }
}

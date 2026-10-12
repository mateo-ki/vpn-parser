use super::{
    MihomoConfigOptions, NetworkProtocol, ProxyMode, RoutingMode, RuleAction, RuleMatcher,
};
use crate::kernel::{find_kernel_executable, KernelKind};
use crate::models::Node;
use crate::proxy_probe::{ProxyProbeProgress, ProxyProbeResult, PROXY_CONNECTIVITY_TEST_URL};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

const XRAY_BATCH_SIZE: usize = 64;

fn xray_batch_count(node_count: usize) -> usize {
    node_count.div_ceil(XRAY_BATCH_SIZE)
}

pub async fn verify_nodes_with_xray(
    nodes: Vec<Node>,
    timeout: Duration,
) -> Result<Vec<ProxyProbeResult>> {
    verify_nodes_with_xray_progress(nodes, timeout, None::<fn(ProxyProbeProgress)>).await
}

pub async fn verify_nodes_with_xray_progress(
    nodes: Vec<Node>,
    timeout: Duration,
    progress: Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
) -> Result<Vec<ProxyProbeResult>> {
    let network_limit = Arc::new(Semaphore::new(XRAY_BATCH_SIZE));
    verify_nodes_with_xray_progress_limited(nodes, timeout, progress, network_limit).await
}

pub async fn verify_nodes_with_xray_progress_limited(
    nodes: Vec<Node>,
    timeout: Duration,
    progress: Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    network_limit: Arc<Semaphore>,
) -> Result<Vec<ProxyProbeResult>> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let binary = find_kernel_executable(KernelKind::Xray)
        .ok_or_else(|| anyhow!("未找到 Xray 可执行文件"))?;
    let total = nodes.len();
    let started = tokio::time::Instant::now();
    let mut results = Vec::with_capacity(total);
    let batch_total = xray_batch_count(nodes.len());
    for (batch_index, batch) in nodes.chunks(XRAY_BATCH_SIZE).enumerate() {
        match probe_batch(&binary, batch.to_vec(), timeout, network_limit.clone()).await {
            Ok(mut batch_results) => results.append(&mut batch_results),
            Err(error) => results.extend(batch.iter().map(|node| ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Xray 测速批次失败: {error}"),
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

async fn probe_batch(
    binary: &std::path::Path,
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
    let mut inbounds = Vec::with_capacity(nodes.len());
    let mut outbounds = Vec::with_capacity(nodes.len() + 1);
    let mut rules = Vec::with_capacity(nodes.len());
    let mut immediate = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let options = MihomoConfigOptions {
            mixed_port: ports[index],
            ..Default::default()
        };
        match build_xray_config(node, options) {
            Ok(config) => {
                let inbound_tag = format!("probe-in-{index}");
                let outbound_tag = format!("probe-out-{index}");
                let mut inbound = config["inbounds"][0].clone();
                inbound["tag"] = json!(inbound_tag);
                let mut outbound = config["outbounds"][0].clone();
                outbound["tag"] = json!(outbound_tag);
                inbounds.push(inbound);
                outbounds.push(outbound);
                rules.push(
                    json!({"type":"field","inboundTag":[inbound_tag],"outboundTag":outbound_tag}),
                );
            }
            Err(error) => immediate.push(ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Xray 配置不支持: {error}"),
            }),
        }
    }
    if inbounds.is_empty() {
        return Ok(immediate);
    }
    let ready_port = inbounds[0]["port"].as_u64().unwrap_or(ports[0] as u64) as u16;
    drop(reservations);
    outbounds.push(json!({"tag":"direct","protocol":"freedom","settings":{}}));
    let config = json!({
        "log":{"loglevel":"warning"}, "inbounds":inbounds, "outbounds":outbounds,
        "routing":{"domainStrategy":"AsIs","rules":rules}
    });
    let directory = std::env::temp_dir().join(format!("vpn-parser-xray-probe-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await?;
    let config_path = directory.join("config.json");
    let log_path = directory.join("xray.log");
    tokio::fs::write(&config_path, serde_json::to_vec_pretty(&config)?).await?;
    let log = std::fs::File::create(&log_path)?;
    let stderr = log.try_clone()?;
    let mut command = Command::new(binary);
    command
        .arg("run")
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
    let ready = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if child.try_wait().ok().flatten().is_some() {
                return false;
            }
            if TcpStream::connect(("127.0.0.1", ready_port)).await.is_ok() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or(false);
    if !ready {
        let _ = child.kill().await;
        let detail = tokio::fs::read_to_string(&log_path)
            .await
            .unwrap_or_default();
        let _ = tokio::fs::remove_dir_all(&directory).await;
        return Err(anyhow!(
            "Xray 批量测速启动失败: {}",
            detail.lines().last().unwrap_or("未知错误")
        ));
    }
    let mut tasks = JoinSet::new();
    for (index, node) in nodes.into_iter().enumerate() {
        let port = ports[index];
        let network_limit = network_limit.clone();
        tasks.spawn(async move {
            let _permit = network_limit
                .acquire_owned()
                .await
                .map_err(|_| "测速并发控制已关闭".to_string());
            let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{port}"));
            let started = tokio::time::Instant::now();
            let outcome = match _permit {
                Err(error) => Err(error),
                Ok(_permit) => match proxy {
                    Ok(proxy) => match reqwest::Client::builder()
                        .proxy(proxy)
                        .timeout(timeout)
                        .build()
                    {
                        Ok(client) => client
                            .get(PROXY_CONNECTIVITY_TEST_URL)
                            .send()
                            .await
                            .map_err(|error| error.to_string())
                            .and_then(|response| {
                                if response.status() == reqwest::StatusCode::NO_CONTENT {
                                    Ok(())
                                } else {
                                    Err(format!("HTTP {}", response.status()))
                                }
                            }),
                        Err(error) => Err(error.to_string()),
                    },
                    Err(error) => Err(error.to_string()),
                },
            };
            match outcome {
                Ok(()) => ProxyProbeResult {
                    id: node.id,
                    ok: true,
                    latency_ms: Some(started.elapsed().as_millis().max(1) as i64),
                    message: "Xray 真实代理可用".into(),
                },
                Err(error) => ProxyProbeResult {
                    id: node.id,
                    ok: false,
                    latency_ms: None,
                    message: format!("Xray 真实代理测试失败: {error}"),
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

pub fn build_xray_config(node: &Node, options: MihomoConfigOptions) -> Result<Value> {
    options.validate()?;
    if options.mode == ProxyMode::Tun {
        return Err(anyhow!("Xray 暂不支持 TUN 模式"));
    }
    let outbound = build_outbound(node)?;
    let dns_servers = if options.dns.enabled {
        options.dns.nameservers.clone()
    } else {
        vec!["localhost".into()]
    };
    let routing_rules = build_routing_rules(&options)?;
    Ok(json!({
        "log": { "loglevel": "warning" },
        "dns": {
            "servers": dns_servers,
            "queryStrategy": if options.dns.ipv6 { "UseIP" } else { "UseIPv4" }
        },
        "inbounds": [{
            "tag": "local-http",
            "listen": "127.0.0.1",
            "port": options.mixed_port,
            "protocol": "http",
            "settings": { "allowTransparent": false }
        }],
        "outbounds": [
            outbound,
            { "tag": "direct", "protocol": "freedom", "settings": {} },
            { "tag": "blocked", "protocol": "blackhole", "settings": {} }
        ],
        "routing": {
            "domainStrategy": "AsIs",
            "rules": routing_rules
        }
    }))
}

fn build_routing_rules(options: &MihomoConfigOptions) -> Result<Vec<Value>> {
    let mut rules = Vec::new();
    let selected_rules = match options.routing_mode {
        RoutingMode::Rule => super::optimize_traffic_rules(&options.rules),
        RoutingMode::ChinaDirect => super::recommended_traffic_rules(),
        RoutingMode::GfwList => super::gfwlist_traffic_rules(),
        RoutingMode::Global | RoutingMode::Direct => Vec::new(),
    };
    for rule in selected_rules {
        let outbound = match rule.action {
            RuleAction::Direct => "direct",
            RuleAction::Proxy => "proxy",
            RuleAction::Reject => "blocked",
        };
        let mut value = Map::new();
        value.insert("type".into(), json!("field"));
        value.insert("outboundTag".into(), json!(outbound));
        match &rule.matcher {
            RuleMatcher::Domain(domain) => {
                value.insert("domain".into(), json!([format!("full:{domain}")]));
            }
            RuleMatcher::DomainSuffix(domain) => {
                value.insert("domain".into(), json!([format!("domain:{domain}")]));
            }
            RuleMatcher::GeoSite(category) => {
                value.insert("domain".into(), json!([format!("geosite:{category}")]));
            }
            RuleMatcher::GeoIp(country) => {
                value.insert(
                    "ip".into(),
                    json!([format!("geoip:{}", country.to_ascii_lowercase())]),
                );
            }
            RuleMatcher::IpCidr(cidr) => {
                value.insert("ip".into(), json!([cidr]));
            }
            RuleMatcher::DestinationPort(port) => {
                port.parse::<u16>()
                    .map_err(|_| anyhow!("Xray 端口规则无效: {port}"))?;
                value.insert("port".into(), json!(port));
            }
            RuleMatcher::ProcessName(name) => {
                return Err(anyhow!("Xray 暂不支持进程名分流规则: {name}"));
            }
            RuleMatcher::Network(protocol) => {
                value.insert(
                    "network".into(),
                    json!(match protocol {
                        NetworkProtocol::Tcp => "tcp",
                        NetworkProtocol::Udp => "udp",
                    }),
                );
            }
            RuleMatcher::Quic => {
                value.insert("network".into(), json!("udp"));
                value.insert("port".into(), json!("443"));
            }
        }
        rules.push(Value::Object(value));
    }
    rules.push(json!({
        "type": "field",
        "inboundTag": ["local-http"],
        "outboundTag": match options.routing_mode {
            RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::Global => "proxy",
            RoutingMode::GfwList | RoutingMode::Direct => "direct",
        }
    }));
    Ok(rules)
}

fn build_outbound(node: &Node) -> Result<Value> {
    let cfg = node.config.as_object().cloned().unwrap_or_default();
    let protocol = node.protocol.to_ascii_lowercase();
    let settings = match protocol.as_str() {
        "ss" | "shadowsocks" => json!({"servers": [{
            "address": node.server,
            "port": node.port,
            "method": required(&cfg, &["cipher", "method"] )?,
            "password": required(&cfg, &["password"])?
        }]}),
        "vmess" => json!({"vnext": [{
            "address": node.server,
            "port": node.port,
            "users": [{
                "id": required(&cfg, &["uuid", "id"] )?,
                "alterId": number(&cfg, &["alterId", "alter-id"]).unwrap_or(0),
                "security": string(&cfg, &["cipher", "security"]).unwrap_or("auto")
            }]
        }]}),
        "vless" => json!({"vnext": [{
            "address": node.server,
            "port": node.port,
            "users": [{
                "id": required(&cfg, &["uuid", "id"] )?,
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
        outbound["streamSettings"] = build_stream_settings(&cfg);
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

fn build_stream_settings(cfg: &Map<String, Value>) -> Value {
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
    if security == "tls" {
        stream["tlsSettings"] = json!({
            "serverName": string(cfg, &["sni", "servername", "server-name"]).unwrap_or(""),
            // 同 sing-box：不采信远端订阅里的关闭证书校验标记。
            "allowInsecure": false
        });
    } else if security == "reality" {
        stream["realitySettings"] = json!({
            "serverName": string(cfg, &["sni", "servername", "server-name"]).unwrap_or(""),
            "publicKey": string(cfg, &["public-key", "publicKey", "pbk"]).unwrap_or(""),
            "shortId": string(cfg, &["short-id", "shortId", "sid"]).unwrap_or(""),
            "fingerprint": string(cfg, &["client-fingerprint", "fingerprint", "fp"]).unwrap_or("chrome")
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

fn bool_value(cfg: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .any(|key| cfg.get(*key).and_then(Value::as_bool).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::TrafficRule;
    use chrono::Utc;

    fn node(protocol: &str, config: Value) -> Node {
        Node {
            id: "node".into(),
            source_id: None,
            protocol: protocol.into(),
            name: "node".into(),
            server: "example.com".into(),
            port: 443,
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
    fn builds_supported_xray_outbounds() {
        let cases = [
            node("ss", json!({"cipher":"aes-128-gcm","password":"p"})),
            node(
                "vmess",
                json!({"uuid":"00000000-0000-0000-0000-000000000000"}),
            ),
            node(
                "vless",
                json!({"uuid":"00000000-0000-0000-0000-000000000000"}),
            ),
            node("trojan", json!({"password":"p"})),
            node("http", json!({"username":"u","password":"p"})),
            node("socks5", json!({"username":"u","password":"p"})),
        ];
        for node in cases {
            assert!(
                build_xray_config(&node, MihomoConfigOptions::default()).is_ok(),
                "{}",
                node.protocol
            );
        }
    }

    #[test]
    fn rejects_xray_tun_mode() {
        let mut options = MihomoConfigOptions::default();
        options.mode = ProxyMode::Tun;
        assert!(build_xray_config(&node("trojan", json!({"password":"p"})), options).is_err());
    }

    #[test]
    fn xray_config_uses_configured_dns_servers() {
        let config = build_xray_config(
            &node("trojan", json!({"password":"p"})),
            MihomoConfigOptions::default(),
        )
        .unwrap();

        assert_eq!(config["dns"]["servers"][0], "https://doh.pub/dns-query");
        assert_eq!(
            config["dns"]["servers"][1],
            "https://dns.alidns.com/dns-query"
        );
        assert_eq!(config["dns"]["queryStrategy"], "UseIPv4");
    }

    #[test]
    fn xray_routing_presets_map_rules_and_fallbacks() {
        let selected = node("trojan", json!({"password":"p"}));
        let china = build_xray_config(
            &selected,
            MihomoConfigOptions {
                routing_mode: RoutingMode::ChinaDirect,
                ..MihomoConfigOptions::default()
            },
        )
        .unwrap();
        let gfw = build_xray_config(
            &selected,
            MihomoConfigOptions {
                routing_mode: RoutingMode::GfwList,
                ..MihomoConfigOptions::default()
            },
        )
        .unwrap();

        assert!(china["routing"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule["domain"][0] == "geosite:cn"));
        assert_eq!(
            china["routing"]["rules"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["outboundTag"],
            "proxy"
        );
        assert!(gfw["routing"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule["domain"][0] == "geosite:gfw"));
        assert_eq!(
            gfw["routing"]["rules"].as_array().unwrap().last().unwrap()["outboundTag"],
            "direct"
        );
    }

    #[test]
    fn xray_rejects_unsupported_process_routing_rule() {
        let options = MihomoConfigOptions {
            rules: vec![TrafficRule {
                matcher: RuleMatcher::ProcessName("app.exe".into()),
                action: RuleAction::Proxy,
            }],
            ..MihomoConfigOptions::default()
        };
        let error =
            build_xray_config(&node("trojan", json!({"password":"p"})), options).unwrap_err();
        assert!(error.to_string().contains("进程名分流"));
    }

    #[test]
    fn xray_uses_one_process_per_sixty_four_nodes() {
        assert_eq!(xray_batch_count(0), 0);
        assert_eq!(xray_batch_count(1), 1);
        assert_eq!(xray_batch_count(64), 1);
        assert_eq!(xray_batch_count(65), 2);
        assert_eq!(xray_batch_count(129), 3);
    }

    #[tokio::test]
    async fn installed_xray_accepts_generated_config() {
        let Some(binary) = find_kernel_executable(KernelKind::Xray) else {
            return;
        };
        let config = build_xray_config(
            &node(
                "trojan",
                json!({"password":"p","security":"tls","sni":"example.com"}),
            ),
            MihomoConfigOptions {
                mixed_port: 17891,
                controller_port: 19091,
                ..Default::default()
            },
        )
        .unwrap();
        let directory =
            std::env::temp_dir().join(format!("vpn-parser-xray-check-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let path = directory.join("config.json");
        tokio::fs::write(&path, serde_json::to_vec_pretty(&config).unwrap())
            .await
            .unwrap();
        let output = Command::new(binary)
            .arg("run")
            .arg("-test")
            .arg("-c")
            .arg(&path)
            .output()
            .await
            .unwrap();
        let _ = tokio::fs::remove_dir_all(directory).await;
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

use super::MihomoConfigOptions;
use crate::kernel::{find_kernel_executable, KernelKind};
use crate::models::Node;
use crate::proxy::compiler::shoes::build_shoes_client_proxy;
use crate::proxy::compiler::BackendCompiler;
use crate::proxy_probe::{
    ProxyProbeProgress, ProxyProbeResult, PROXY_CONNECTIVITY_FALLBACK_URL,
    PROXY_CONNECTIVITY_TEST_URL,
};
use anyhow::{anyhow, Result};
use serde_json::Value as JsonValue;
use serde_yaml::{Mapping, Value as YamlValue};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

const SHOES_BATCH_SIZE: usize = 64;

fn shoes_batch_count(node_count: usize) -> usize {
    node_count.div_ceil(SHOES_BATCH_SIZE)
}

pub fn build_shoes_config(node: &Node, options: MihomoConfigOptions) -> Result<JsonValue> {
    options.validate()?;
    let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(node));
    let compiler = crate::proxy::compiler::ShoesCompiler::new();
    let artifact = compiler.compile(&plan)?;
    artifact.as_json_value()
}

pub async fn verify_nodes_with_shoes_progress_limited(
    nodes: Vec<Node>,
    timeout: Duration,
    progress: Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    network_limit: Arc<Semaphore>,
) -> Result<Vec<ProxyProbeResult>> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let binary = find_kernel_executable(KernelKind::Shoes)
        .ok_or_else(|| anyhow!("未找到 Shoes 可执行文件"))?;
    let total = nodes.len();
    let started = tokio::time::Instant::now();
    let mut results = Vec::with_capacity(total);
    let batch_total = shoes_batch_count(nodes.len());
    for (batch_index, batch) in nodes.chunks(SHOES_BATCH_SIZE).enumerate() {
        match probe_batch(&binary, batch.to_vec(), timeout, network_limit.clone()).await {
            Ok(mut batch_results) => results.append(&mut batch_results),
            Err(error) => results.extend(batch.iter().map(|node| ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Shoes 测速批次失败: {error}"),
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

    let mut doc = Vec::new();
    let mut immediate = Vec::new();
    let mut valid_ports = Vec::new();

    for (index, node) in nodes.iter().enumerate() {
        let port = ports[index];
        let group_name = format!("probe-grp-{index}");
        let proxy_name = format!("probe-node-{index}");

        match build_shoes_client_proxy(&proxy_name, node, None) {
            Ok(client_proxy) => {
                // 定义专属 client_group
                let mut grp = Mapping::new();
                grp.insert(
                    YamlValue::String("client_group".into()),
                    YamlValue::String(group_name.clone()),
                );
                grp.insert(
                    YamlValue::String("strategy".into()),
                    YamlValue::String("selector".into()),
                );
                grp.insert(
                    YamlValue::String("default".into()),
                    YamlValue::String(proxy_name),
                );
                grp.insert(
                    YamlValue::String("client_proxies".into()),
                    YamlValue::Sequence(vec![client_proxy]),
                );
                doc.push(YamlValue::Mapping(grp));

                // 定义专属 inbound
                let mut server = Mapping::new();
                server.insert(
                    YamlValue::String("name".into()),
                    YamlValue::String(format!("probe-in-{index}")),
                );
                server.insert(
                    YamlValue::String("address".into()),
                    YamlValue::String(format!("127.0.0.1:{port}")),
                );
                let mut proto = Mapping::new();
                proto.insert(
                    YamlValue::String("type".into()),
                    YamlValue::String("mixed".into()),
                );
                proto.insert(YamlValue::String("udp_enabled".into()), YamlValue::Bool(true));
                server.insert(YamlValue::String("protocol".into()), YamlValue::Mapping(proto));

                let mut rule = Mapping::new();
                rule.insert(
                    YamlValue::String("masks".into()),
                    YamlValue::String("0.0.0.0/0".into()),
                );
                rule.insert(
                    YamlValue::String("action".into()),
                    YamlValue::String("allow".into()),
                );
                rule.insert(
                    YamlValue::String("client_chain".into()),
                    YamlValue::String(group_name),
                );
                server.insert(
                    YamlValue::String("rules".into()),
                    YamlValue::Sequence(vec![YamlValue::Mapping(rule)]),
                );

                doc.push(YamlValue::Mapping(server));
                valid_ports.push(port);
            }
            Err(error) => immediate.push(ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Shoes 配置不支持: {error}"),
            }),
        }
    }

    if valid_ports.is_empty() {
        return Ok(immediate);
    }

    let ready_port = valid_ports[0];
    drop(reservations);

    let directory =
        std::env::temp_dir().join(format!("vpn-parser-shoes-probe-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await?;
    let config_path = directory.join("config.yaml");
    let log_path = directory.join("shoes.log");
    let yaml_str = serde_yaml::to_string(&doc)?;
    tokio::fs::write(&config_path, yaml_str).await?;

    let log = std::fs::File::create(&log_path)?;
    let stderr = log.try_clone()?;
    let mut command = Command::new(binary);
    command
        .arg("--no-reload")
        .arg(&config_path)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));

    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }

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
            "Shoes 批量测速启动失败: {}",
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
                        Ok(client) => {
                            let first = client.get(PROXY_CONNECTIVITY_TEST_URL).send().await;
                            match first {
                                Ok(resp) if resp.status() == reqwest::StatusCode::NO_CONTENT => {
                                    Ok("Google")
                                }
                                _ => {
                                    match client.get(PROXY_CONNECTIVITY_FALLBACK_URL).send().await {
                                        Ok(resp)
                                            if resp.status()
                                                == reqwest::StatusCode::NO_CONTENT =>
                                        {
                                            Ok("Cloudflare")
                                        }
                                        Ok(resp) => Err(format!("HTTP {}", resp.status())),
                                        Err(err) => Err(err.to_string()),
                                    }
                                }
                            }
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
                        message: format!("{latency} ms (Shoes 真实代理 · {target})"),
                    }
                }
                Err(error) => ProxyProbeResult {
                    id: node.id,
                    ok: false,
                    latency_ms: None,
                    message: format!("Shoes 真实代理测试失败: {error}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::config::TrafficRule;
    use crate::proxy::{ProxyMode, RoutingMode, RuleAction, RuleMatcher};
    use chrono::Utc;
    use serde_json::json;

    fn node(protocol: &str, config: serde_json::Value) -> Node {
        Node {
            id: "test-node".into(),
            source_id: None,
            protocol: protocol.into(),
            name: "test-node".into(),
            server: "1.2.3.4".into(),
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
    fn builds_supported_shoes_configs() {
        let cases = [
            node("ss", json!({"cipher":"aes-128-gcm","password":"p"})),
            node("snell", json!({"cipher":"aes-128-gcm","password":"p"})),
            node(
                "vmess",
                json!({"uuid":"550e8400-e29b-41d4-a716-446655440000","cipher":"aes-128-gcm"}),
            ),
            node(
                "vless",
                json!({
                    "uuid":"550e8400-e29b-41d4-a716-446655440000",
                    "security":"reality",
                    "pbk":"8lfJuDSkIdAZsCIwznu2LklBUV0tn2cz-JwZjtqt8B8",
                    "sid":"0123456789abcdef",
                    "flow":"xtls-rprx-vision"
                }),
            ),
            node(
                "trojan",
                json!({"password":"p","security":"tls","sni":"example.com"}),
            ),
            node("http", json!({"username":"u","password":"p"})),
            node("socks5", json!({"username":"u","password":"p"})),
            node("anytls", json!({"password":"p"})),
            node(
                "naiveproxy",
                json!({"username":"u","password":"p","sni":"example.com"}),
            ),
        ];

        for n in cases {
            let res = build_shoes_config(&n, MihomoConfigOptions::default());
            assert!(
                res.is_ok(),
                "Failed to build shoes config for {}: {:?}",
                n.protocol,
                res.err()
            );
        }
    }

    #[test]
    fn rejects_shoes_tun_mode() {
        let mut options = MihomoConfigOptions::default();
        options.mode = ProxyMode::Tun;
        let n = node("ss", json!({"cipher":"aes-128-gcm","password":"p"}));
        assert!(build_shoes_config(&n, options).is_err());
    }

    /// 多谓词规则（WebRTC 防泄露的端口列表 + 端口段）必须拆成多条平铺规则，
    /// 禁止生成 `masks: {any: [...]}` —— Shoes 0.3.4 的 any + no_resolve 组合
    /// 会毒化匹配管线：其后所有未命中域名规则的域名被静默 Block，
    /// 浏览器表现为 ERR_EMPTY_RESPONSE（pool.983698.xyz 复现过的那个 bug）。
    #[test]
    fn multi_predicate_rules_are_split_not_any_wrapped() {
        let n = node("ss", json!({"cipher":"aes-128-gcm","password":"p"}));
        let mut options = MihomoConfigOptions::default();
        options.block_webrtc = true;
        let val = build_shoes_config(&n, options).expect("valid config");
        let yaml_str = serde_yaml::to_string(&val).expect("serializable to yaml");

        assert!(
            !yaml_str.contains("any:"),
            "生成配置里不允许出现 any: 组合:\n{yaml_str}"
        );
        // 端口列表与端口段各自成为一条 block 规则，动作语义不变
        assert!(yaml_str.contains("destination_port:"));
        assert!(yaml_str.contains("destination_port_range:"));
        assert_eq!(
            yaml_str.matches("action: block").count(),
            3,
            "STUN/TURN 拦截应拆成 keyword + 端口列表 + 端口段三条 block 规则:\n{yaml_str}"
        );
    }

    #[test]
    fn generated_shoes_config_validates_structure() {
        let n = node(
            "snell",
            json!({"cipher":"aes-128-gcm","password":"secretpassword"}),
        );
        let options = MihomoConfigOptions {
            mixed_port: 9980,
            controller_port: 9090,
            controller_secret: "secret123".into(),
            ..Default::default()
        };
        let val = build_shoes_config(&n, options).expect("valid config");
        let yaml_str = serde_yaml::to_string(&val).expect("serializable to yaml");

        assert!(yaml_str.contains("clash_api_address"));
        assert!(yaml_str.contains("127.0.0.1:9090"));
        assert!(yaml_str.contains("secret: secret123"));
        assert!(yaml_str.contains("client_group: PROXY"));
        assert!(yaml_str.contains("strategy: selector"));
        assert!(yaml_str.contains("type: snell"));
        assert!(yaml_str.contains("127.0.0.1:9980"));
    }

    /// QUIC 拦截的语义是"仅拦 UDP 443"。Shoes 的 masks 没有 TCP/UDP 维度，
    /// 若误译成裸 `destination_port: 443`，所有未命中前置域名规则的普通 HTTPS
    /// 都会被静默 Block（pool.983698.xyz 报 ERR_EMPTY_RESPONSE 的根因）。
    #[test]
    fn quic_rule_is_skipped_not_translated_to_tcp_port_block() {
        let n = node("ss", json!({"cipher":"aes-128-gcm","password":"p"}));
        let mut options = MihomoConfigOptions::default();
        options.rules = vec![TrafficRule {
            matcher: RuleMatcher::Quic,
            action: RuleAction::Reject,
        }];
        let val = build_shoes_config(&n, options).expect("valid config");
        let yaml_str = serde_yaml::to_string(&val).expect("serializable to yaml");
        assert!(
            !yaml_str.contains("destination_port"),
            "QUIC 规则不得译成端口拦截:\n{yaml_str}"
        );
        assert_eq!(
            yaml_str.matches("action: block").count(),
            0,
            "QUIC 规则跳过后不应产生任何 block 规则:\n{yaml_str}"
        );
    }

    #[test]
    fn test_shoes_vless_reality_node() {
        let n = node(
            "vless",
            json!({
                "client-fingerprint": "chrome",
                "flow": "xtls-rprx-vision",
                "name": "🇺🇸US²_3",
                "network": "tcp",
                "port": 8443,
                "reality-opts": {
                    "public-key": "Ha1kOu8LGfb6tK9wwrB_5UYrvjHFT_QwjvIuqM-q2go",
                    "short-id": "40555f988c33e341"
                },
                "server": "143.246.60.129",
                "servername": "gateway.icloud.com",
                "skip-cert-verify": true,
                "tfo": true,
                "tls": true,
                "type": "vless",
                "uuid": "25284f5f-1dc4-42b8-bee1-f4a5144ee064"
            }),
        );
        let options = MihomoConfigOptions {
            mixed_port: 19985,
            controller_port: 19095,
            routing_mode: RoutingMode::Rule,
            ..Default::default()
        };
        let val = build_shoes_config(&n, options).expect("valid config");
        let yaml_str = serde_yaml::to_string(&val).expect("serializable to yaml");
        println!("=== GENERATED SHOES REALITY YAML ===\n{yaml_str}\n============================");
        assert!(yaml_str.contains("type: reality"));
        assert!(yaml_str.contains("public_key: Ha1kOu8LGfb6tK9wwrB_5UYrvjHFT_QwjvIuqM-q2go"));
        assert!(yaml_str.contains("short_id: 40555f988c33e341"));
        assert!(yaml_str.contains("sni_hostname: gateway.icloud.com"));
        assert!(yaml_str.contains("vision: true"));
    }

    #[test]
    fn test_shoes_vless_tls_with_empty_pbk() {
        let n = node(
            "vless",
            json!({
                "allowInsecure": false,
                "encryption": "none",
                "flow": "",
                "network": "ws",
                "path": "/sub",
                "pbk": "",
                "port": 443,
                "security": "tls",
                "server": "95.182.101.185",
                "sid": "",
                "sni": "example.com",
                "type": "vless",
                "uuid": "2561db48-c4d2-4b36-808a-db5093797aeb"
            }),
        );
        let options = MihomoConfigOptions::default();
        let val = build_shoes_config(&n, options).expect("valid config with empty pbk should not fail as reality");
        let yaml_str = serde_yaml::to_string(&val).unwrap();
        assert!(yaml_str.contains("type: tls"));
        assert!(!yaml_str.contains("type: reality"));
        assert!(yaml_str.contains("alpn_protocols"));
        assert!(yaml_str.contains("http/1.1"));
        assert!(yaml_str.contains("no_resolve: true"));
    }
}

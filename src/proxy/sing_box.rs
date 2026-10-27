use super::MihomoConfigOptions;
#[allow(unused_imports)]
use super::{NetworkProtocol, ProxyMode, RoutingMode, RuleAction, RuleMatcher};
use crate::kernel::{find_kernel_executable, KernelKind};
use crate::models::Node;
use crate::proxy::compiler::BackendCompiler;
use crate::proxy_probe::{
    ProxyProbeProgress, ProxyProbeResult, PROXY_CONNECTIVITY_FALLBACK_URL,
    PROXY_CONNECTIVITY_TEST_URL,
};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

#[allow(dead_code)]
const GEOSITE_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs";
#[allow(dead_code)]
const GEOSITE_ADS_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ads-all.srs";
#[allow(dead_code)]
const GEOSITE_GFW_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-gfw.srs";
#[allow(dead_code)]
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
        let batch_results = probe_batch_with_sing_box_isolated(
            &binary,
            batch.to_vec(),
            timeout,
            network_limit.clone(),
        )
        .await;
        results.extend(batch_results);
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

async fn probe_batch_with_sing_box_isolated(
    binary: &Path,
    nodes: Vec<Node>,
    timeout: Duration,
    network_limit: Arc<Semaphore>,
) -> Vec<ProxyProbeResult> {
    if nodes.is_empty() {
        return Vec::new();
    }
    match probe_batch_with_sing_box(binary, nodes.clone(), timeout, network_limit.clone()).await {
        Ok(results) => results,
        Err(err) => {
            if nodes.len() <= 1 {
                return nodes
                    .into_iter()
                    .map(|n| ProxyProbeResult {
                        id: n.id,
                        ok: false,
                        latency_ms: None,
                        message: format!("Sing-box 节点配置不支持: {err}"),
                    })
                    .collect();
            }
            let mid = nodes.len() / 2;
            let (left, right) = nodes.split_at(mid);
            tracing::warn!(
                total = nodes.len(),
                left = left.len(),
                right = right.len(),
                %err,
                "Sing-box 批次启动异常，自动二分隔离异常节点"
            );
            let mut left_res = Box::pin(probe_batch_with_sing_box_isolated(
                binary,
                left.to_vec(),
                timeout,
                network_limit.clone(),
            ))
            .await;
            let right_res = Box::pin(probe_batch_with_sing_box_isolated(
                binary,
                right.to_vec(),
                timeout,
                network_limit,
            ))
            .await;
            left_res.extend(right_res);
            left_res
        }
    }
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
                    // 确保 route.rules 引用的 outbound 在 outbounds 中存在
                    outbounds.push(json!({
                        "type": "direct",
                        "tag": outbound_tag,
                    }));
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
        "dns": crate::proxy::compiler::sing_box::build_dns(&MihomoConfigOptions::default().dns.nameservers)?,
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
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
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
    let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(node));
    let compiler = crate::proxy::compiler::SingBoxCompiler::new();
    let artifact = compiler.compile(&plan)?;
    artifact.as_json_value()
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

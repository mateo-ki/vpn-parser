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
        let batch_results =
            probe_batch_isolated(&binary, batch.to_vec(), timeout, network_limit.clone()).await;
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

async fn probe_batch_isolated(
    binary: &std::path::Path,
    nodes: Vec<Node>,
    timeout: Duration,
    network_limit: Arc<Semaphore>,
) -> Vec<ProxyProbeResult> {
    if nodes.is_empty() {
        return Vec::new();
    }
    match probe_batch(binary, nodes.clone(), timeout, network_limit.clone()).await {
        Ok(results) => results,
        Err(err) => {
            if nodes.len() <= 1 {
                return nodes
                    .into_iter()
                    .map(|n| ProxyProbeResult {
                        id: n.id,
                        ok: false,
                        latency_ms: None,
                        message: format!("Xray 节点配置不支持: {err}"),
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
                "Xray 批次启动异常，自动二分隔离异常节点"
            );
            let mut left_res = Box::pin(probe_batch_isolated(
                binary,
                left.to_vec(),
                timeout,
                network_limit.clone(),
            ))
            .await;
            let right_res = Box::pin(probe_batch_isolated(
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
                        Ok(client) => {
                            let first = client.get(PROXY_CONNECTIVITY_TEST_URL).send().await;
                            match first {
                                Ok(resp) if resp.status() == reqwest::StatusCode::NO_CONTENT => {
                                    Ok("Google")
                                }
                                _ => {
                                    // 交叉复验：Google 失败后回退测试 Cloudflare 204
                                    match client.get(PROXY_CONNECTIVITY_FALLBACK_URL).send().await {
                                        Ok(resp) if resp.status() == reqwest::StatusCode::NO_CONTENT => {
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
                        message: format!("{latency} ms (Xray 真实代理 · {target})"),
                    }
                }
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
    let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(node));
    let compiler = crate::proxy::compiler::XrayCompiler::new();
    let artifact = compiler.compile(&plan)?;
    artifact.as_json_value()
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
        let mut cmd = Command::new(binary);
        let geo_dir = crate::geo::mihomo_dir();
        if geo_dir.exists() {
            cmd.env("XRAY_LOCATION_ASSET", &geo_dir);
            cmd.current_dir(&geo_dir);
        }
        let output = cmd
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

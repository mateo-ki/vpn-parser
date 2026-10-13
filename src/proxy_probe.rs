use crate::export::nodes_to_clash_proxies;
use crate::kernel::find_mihomo_executable;
use crate::models::Node;
use anyhow::{anyhow, Context, Result};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

pub const PROXY_CONNECTIVITY_TEST_URL: &str = "https://www.google.com/generate_204";
/// Concurrent delay probes through the single shared Mihomo instance. Each probe is a
/// TLS request to generate_204 through a real proxy. Kept moderate: too many concurrent
/// real-proxy connections saturate consumer NICs/routers.
const PROXY_CONCURRENCY: usize = 96;
const MIHOMO_BATCH_CONCURRENCY: usize = 3;
/// Concurrent `mihomo -t` config validations while isolating invalid proxies.
const VALIDATE_CONCURRENCY: usize = 8;

#[derive(Debug, Clone)]
pub struct ProxyProbeResult {
    pub id: String,
    pub ok: bool,
    pub latency_ms: Option<i64>,
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct ProxyProbeProgress {
    pub done: usize,
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    pub elapsed: Duration,
    pub batch_index: Option<usize>,
    pub batch_total: Option<usize>,
}

#[derive(Clone)]
struct PreparedProxy {
    index: usize,
    id: String,
    name: String,
    config: Value,
}


/// 「常驻内核」打开时一批装下的节点数：一个内核实例装下整轮候选，只付一次冷启动。
pub const PERSISTENT_MIHOMO_BATCH_SIZE: usize = 8192;


pub async fn verify_nodes_with_mihomo_progress_limited(
    nodes: Vec<Node>,
    timeout: Duration,
    progress: Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    network_limit: Arc<Semaphore>,
    batch_size: usize,
    conditional_isolation: bool,
) -> Result<Vec<ProxyProbeResult>> {
    if nodes.is_empty() {
        return Ok(Vec::new());
    }
    let mihomo = find_mihomo_executable().ok_or_else(|| {
        anyhow!("未找到 mihomo.exe；请设置 MIHOMO_PATH，或将 Mihomo 放到 bin/mihomo/mihomo.exe")
    })?;
    let total = nodes.len();
    let started = tokio::time::Instant::now();
    let batches = nodes
        .chunks(batch_size.max(1))
        .map(|batch| batch.to_vec())
        .collect::<Vec<_>>();
    let batch_total = batches.len();
    let mut batches = batches.into_iter();
    let mut tasks = JoinSet::new();
    let mut results = Vec::with_capacity(total);
    let mut completed_batches = 0usize;
    loop {
        while tasks.len() < MIHOMO_BATCH_CONCURRENCY {
            let Some(batch) = batches.next() else { break };
            let ids = batch.iter().map(|node| node.id.clone()).collect::<Vec<_>>();
            let mihomo = mihomo.clone();
            let network_limit = network_limit.clone();
            tasks.spawn(async move {
                let outcome = verify_nodes_with_mihomo_path(
                    batch,
                    timeout,
                    &mihomo,
                    None::<fn(ProxyProbeProgress)>,
                    network_limit,
                    conditional_isolation,
                )
                .await;
                (ids, outcome)
            });
        }
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        let (ids, outcome) = joined.map_err(|error| anyhow!("Mihomo 测速批次失败: {error}"))?;
        match outcome {
            Ok(mut batch_results) => results.append(&mut batch_results),
            Err(error) => results.extend(ids.into_iter().map(|id| ProxyProbeResult {
                id,
                ok: false,
                latency_ms: None,
                message: format!("Mihomo 测速批次失败: {error}"),
            })),
        }
        completed_batches += 1;
        if let Some(progress) = progress.as_ref() {
            let ok = results.iter().filter(|result| result.ok).count();
            progress(ProxyProbeProgress {
                done: results.len(),
                total,
                ok,
                fail: results.len().saturating_sub(ok),
                elapsed: started.elapsed(),
                batch_index: Some(completed_batches),
                batch_total: Some(batch_total),
            });
        }
    }
    Ok(results)
}

async fn verify_nodes_with_mihomo_path(
    nodes: Vec<Node>,
    timeout: Duration,
    mihomo: &Path,
    progress: Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    network_limit: Arc<Semaphore>,
    conditional_isolation: bool,
) -> Result<Vec<ProxyProbeResult>> {
    let total = nodes.len();
    let started = tokio::time::Instant::now();
    let (prepared, mut results) = prepare_proxies(&nodes);
    report_proxy_progress(&progress, &results, total, started.elapsed());
    if prepared.is_empty() {
        return Ok(results.into_iter().flatten().collect());
    }

    let work_dir = std::env::temp_dir().join(format!("vpn-parser-mihomo-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&work_dir).await?;
    // 默认先对整批配置做 `mihomo -t` 二分隔离；开启「条件隔离」后先直接测，
    // 只把失败的节点送去二分隔离，省掉一整轮配置验证。
    let (prepared, rejected) = if conditional_isolation {
        (prepared, Vec::new())
    } else {
        isolate_invalid_proxies(prepared, mihomo, &work_dir).await?
    };
    for (proxy, reason) in rejected {
        results[proxy.index] = Some(ProxyProbeResult {
            id: proxy.id,
            ok: false,
            latency_ms: None,
            message: format!("Mihomo 配置不支持: {reason}"),
        });
    }
    report_proxy_progress(&progress, &results, total, started.elapsed());
    if prepared.is_empty() {
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
        return Ok(results.into_iter().flatten().collect());
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let controller_port = listener.local_addr()?.port();
    drop(listener);

    // 增量计数：每完成一个探针就把 results 全量扫两遍的话，8192 的批次要迭代
    // 一千三百万次，进度回调节本身就把测速拖平了。
    let mut done_count = results.iter().filter(|slot| slot.is_some()).count();
    let mut ok_count = results
        .iter()
        .filter(|slot| slot.as_ref().is_some_and(|result| result.ok))
        .count();

    let config = build_mihomo_config(&prepared, controller_port);
    let config_path = work_dir.join("config.yaml");
    tokio::fs::write(&config_path, serde_yaml::to_string(&config)?).await?;
    let startup_log = work_dir.join("startup.log");
    let log_file = std::fs::File::create(&startup_log)?;
    // Mihomo writes its logs to stdout; capture both streams or startup errors vanish
    // and the diagnostic degrades to a useless "未知配置错误".
    let log_stdout = log_file.try_clone()?;

    let mut command = Command::new(mihomo);
    command
        .arg("-d")
        .arg(&work_dir)
        .arg("-f")
        .arg(&config_path)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_stdout))
        .stderr(Stdio::from(log_file));
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("启动 {} 失败", mihomo.display()))?;

    // Loading thousands of proxies into one instance can take well over 5s; scale the
    // controller wait with the proxy count so big sets are not misreported as failures.
    let startup_budget = (Duration::from_secs(8)
        + Duration::from_millis(prepared.len() as u64 * 5))
    .min(Duration::from_secs(60));
    let ready = wait_for_controller(&mut child, controller_port, startup_budget).await;
    if !ready {
        let _ = child.kill().await;
        let _ = child.wait().await;
        let detail = tokio::fs::read_to_string(&startup_log)
            .await
            .unwrap_or_default();
        let detail = compact_diagnostic(&detail);
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
        return Err(anyhow!("Mihomo 启动失败: {detail}"));
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(timeout + Duration::from_secs(2))
        .build()?;
    let timeout_ms = timeout.as_millis().clamp(1_000, 15_000).to_string();
    let prepared_all = prepared.clone();
    let mut iter = prepared.into_iter();
    let mut tasks = JoinSet::new();

    loop {
        while tasks.len() < PROXY_CONCURRENCY {
            let Some(proxy) = iter.next() else {
                break;
            };
            let client = client.clone();
            let timeout_ms = timeout_ms.clone();
            let network_limit = network_limit.clone();
            tasks.spawn(async move {
                let _permit = match network_limit.acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => {
                        return (
                            proxy.index,
                            ProxyProbeResult {
                                id: proxy.id,
                                ok: false,
                                latency_ms: None,
                                message: "测速并发控制已关闭".into(),
                            },
                        );
                    }
                };
                let encoded = utf8_percent_encode(&proxy.name, NON_ALPHANUMERIC);
                let url = format!("http://127.0.0.1:{controller_port}/proxies/{encoded}/delay");
                let outcome = probe_mihomo_target(
                    &client,
                    &url,
                    timeout_ms.as_str(),
                    PROXY_CONNECTIVITY_TEST_URL,
                    "Google",
                )
                .await;
                let result = match outcome {
                    Ok((delay, target)) => ProxyProbeResult {
                        id: proxy.id,
                        ok: true,
                        latency_ms: Some(delay),
                        message: format!("{delay} ms (真实代理 · {target})"),
                    },
                    Err(error) => ProxyProbeResult {
                        id: proxy.id,
                        ok: false,
                        latency_ms: None,
                        message: format!("真实代理测试失败: {error}"),
                    },
                };
                (proxy.index, result)
            });
        }

        let Some(joined) = tasks.join_next().await else {
            break;
        };
        if let Ok((index, result)) = joined {
            let was_ok = result.ok;
            results[index] = Some(result);
            done_count += 1;
            if was_ok {
                ok_count += 1;
            }
            emit_proxy_progress(&progress, done_count, ok_count, total, started.elapsed());
        }
    }

    let _ = child.kill().await;
    let _ = child.wait().await;

    if conditional_isolation {
        let failed: Vec<PreparedProxy> = prepared_all
            .iter()
            .filter(|proxy| {
                results[proxy.index]
                    .as_ref()
                    .is_some_and(|result| !result.ok)
            })
            .cloned()
            .collect();
        if !failed.is_empty() {
            let (_, rejected) = isolate_invalid_proxies(failed, mihomo, &work_dir).await?;
            for (proxy, reason) in rejected {
                results[proxy.index] = Some(ProxyProbeResult {
                    id: proxy.id,
                    ok: false,
                    latency_ms: None,
                    message: format!("Mihomo 配置不支持: {reason}"),
                });
            }
        }
    }
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    if results.len() != total || results.iter().any(Option::is_none) {
        return Err(anyhow!("真实代理测试结果数量不一致"));
    }
    Ok(results.into_iter().flatten().collect())
}

async fn probe_mihomo_target(
    client: &reqwest::Client,
    controller_url: &str,
    timeout_ms: &str,
    target_url: &'static str,
    target_name: &'static str,
) -> std::result::Result<(i64, &'static str), String> {
    let response = client
        .get(controller_url)
        .query(&[("url", target_url), ("timeout", timeout_ms)])
        .send()
        .await
        .map_err(|error| format!("{target_name}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("{target_name}: HTTP {}", response.status()));
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|error| format!("{target_name}: {error}"))?;
    match body.get("delay").and_then(Value::as_i64) {
        Some(delay) if delay > 0 => Ok((delay, target_name)),
        _ => Err(format!("{target_name}: 未返回有效延迟")),
    }
}

fn emit_proxy_progress(
    progress: &Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    done: usize,
    ok: usize,
    total: usize,
    elapsed: Duration,
) {
    if let Some(progress) = progress {
        progress(ProxyProbeProgress {
            done,
            total,
            ok,
            fail: done.saturating_sub(ok),
            elapsed,
            batch_index: None,
            batch_total: None,
        });
    }
}

fn report_proxy_progress(
    progress: &Option<impl Fn(ProxyProbeProgress) + Send + Sync>,
    results: &[Option<ProxyProbeResult>],
    total: usize,
    elapsed: Duration,
) {
    if let Some(progress) = progress {
        let done = results.iter().filter(|result| result.is_some()).count();
        let ok = results
            .iter()
            .filter(|result| result.as_ref().is_some_and(|result| result.ok))
            .count();
        progress(ProxyProbeProgress {
            done,
            total,
            ok,
            fail: done.saturating_sub(ok),
            elapsed,
            batch_index: None,
            batch_total: None,
        });
    }
}

fn build_mihomo_config(prepared: &[PreparedProxy], controller_port: u16) -> Value {
    let proxy_configs: Vec<_> = prepared.iter().map(|proxy| proxy.config.clone()).collect();
    let proxy_names: Vec<_> = prepared.iter().map(|proxy| proxy.name.clone()).collect();
    json!({
        "external-controller": format!("127.0.0.1:{controller_port}"),
        "secret": "",
        "allow-lan": false,
        "ipv6": false,
        "mode": "rule",
        "log-level": "warning",
        "proxies": proxy_configs,
        "proxy-groups": [{
            "name": "PROXY",
            "type": "select",
            "proxies": proxy_names
        }],
        "rules": ["MATCH,PROXY"]
    })
}

async fn isolate_invalid_proxies(
    prepared: Vec<PreparedProxy>,
    mihomo: &Path,
    work_dir: &Path,
) -> Result<(Vec<PreparedProxy>, Vec<(PreparedProxy, String)>)> {
    let mut level = vec![prepared];
    let mut valid = Vec::new();
    let mut rejected = Vec::new();

    // Validate all batches in a level concurrently, then split only the failing
    // ones into the next level. This replaces the old fully serial descent
    // (one `mihomo -t` spawn at a time) with O(log n) bounded-parallel rounds.
    while !level.is_empty() {
        let mut next = Vec::new();
        let mut iter = level.into_iter();
        let mut tasks: JoinSet<(Vec<PreparedProxy>, Result<Option<String>>)> = JoinSet::new();
        loop {
            while tasks.len() < VALIDATE_CONCURRENCY {
                let Some(batch) = iter.next() else {
                    break;
                };
                let mihomo = mihomo.to_path_buf();
                let work_dir = work_dir.to_path_buf();
                tasks.spawn(async move {
                    let outcome = validate_proxy_batch(&batch, &mihomo, &work_dir).await;
                    (batch, outcome)
                });
            }

            let Some(joined) = tasks.join_next().await else {
                break;
            };
            let (mut batch, outcome) = joined.map_err(|error| anyhow!("校验任务失败: {error}"))?;
            match outcome? {
                None => valid.extend(batch),
                Some(reason) if batch.len() == 1 => {
                    rejected.push((batch.pop().expect("single proxy batch"), reason));
                }
                Some(_) => {
                    let right = batch.split_off(batch.len() / 2);
                    next.push(batch);
                    next.push(right);
                }
            }
        }
        level = next;
    }

    valid.sort_by_key(|proxy| proxy.index);
    rejected.sort_by_key(|(proxy, _)| proxy.index);
    Ok((valid, rejected))
}

async fn validate_proxy_batch(
    batch: &[PreparedProxy],
    mihomo: &Path,
    work_dir: &Path,
) -> Result<Option<String>> {
    let path = work_dir.join(format!("validate-{}.yaml", Uuid::new_v4()));
    let config = build_mihomo_config(batch, 0);
    tokio::fs::write(&path, serde_yaml::to_string(&config)?).await?;
    let mut command = Command::new(mihomo);
    command
        .arg("-t")
        .arg("-d")
        .arg(work_dir)
        .arg("-f")
        .arg(&path)
        .kill_on_drop(true)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().await?;
    let _ = tokio::fs::remove_file(path).await;
    if output.status.success() {
        return Ok(None);
    }
    let diagnostic = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(Some(compact_diagnostic(&diagnostic)))
}

fn compact_diagnostic(diagnostic: &str) -> String {
    let text = diagnostic
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .last()
        .unwrap_or("未知配置错误");
    text.chars().take(240).collect()
}

fn prepare_proxies(nodes: &[Node]) -> (Vec<PreparedProxy>, Vec<Option<ProxyProbeResult>>) {
    let mut prepared = Vec::new();
    let mut results = vec![None; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let Some(mut config) = nodes_to_clash_proxies(std::slice::from_ref(node)).pop() else {
            results[index] = Some(ProxyProbeResult {
                id: node.id.clone(),
                ok: false,
                latency_ms: None,
                message: format!("Mihomo 暂不支持协议: {}", node.protocol),
            });
            continue;
        };
        let name = format!("probe-{index}-{}", node.id);
        if let Some(object) = config.as_object_mut() {
            object.insert("name".into(), json!(name));
        }
        prepared.push(PreparedProxy {
            index,
            id: node.id.clone(),
            name,
            config,
        });
    }
    (prepared, results)
}

async fn wait_for_controller(
    child: &mut tokio::process::Child,
    port: u16,
    budget: Duration,
) -> bool {
    let started = tokio::time::Instant::now();
    while started.elapsed() < budget {
        if let Ok(Some(_)) = child.try_wait() {
            return false;
        }
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// 自动切换挑替补前的 TCP 复检：并发拨每个候选的 `host:port`，谁先连上且
/// 上一次延迟最好就用谁（`candidates` 已按延迟从低到高排好）。
///
/// 为什么要这一步：库里那条"测通"可能已经是几小时前的。2026-10-07 实测过 ——
/// 自动切换照 08:24 的 90ms 记录切进一个端口早就不监听的 socks5，结果整个境外
/// 不通，而记录上那个节点仍是"测通"状态。
pub async fn first_reachable(
    candidates: Vec<crate::router::BackupCandidate>,
    timeout: std::time::Duration,
) -> Option<(String, String)> {
    use tokio::net::TcpStream;
    use tokio::task::JoinSet;
    let mut tasks = JoinSet::new();
    for (index, node) in candidates.into_iter().enumerate() {
        tasks.spawn(async move {
            let host = node.host.clone();
            let reachable = matches!(
                tokio::time::timeout(timeout, TcpStream::connect((host.as_str(), node.port))).await,
                Ok(Ok(_))
            );
            (index, node, reachable)
        });
    }
    let mut results = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok(row) = joined {
            results.push(row);
        }
    }
    results.sort_by_key(|(index, _, _)| *index);
    results
        .into_iter()
        .find(|(_, _, reachable)| *reachable)
        .map(|(_, node, _)| (node.id, node.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn node(protocol: &str, config: Value) -> Node {
        let now = Utc::now();
        Node {
            id: format!("{protocol}-id"),
            source_id: None,
            protocol: protocol.into(),
            name: "test".into(),
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
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn manual_probe_target_requires_google() {
        assert_eq!(
            PROXY_CONNECTIVITY_TEST_URL,
            "https://www.google.com/generate_204"
        );
    }

    #[test]
    fn prepares_supported_nodes_and_reports_unsupported_protocols() {
        let nodes = vec![
            node(
                "vless",
                json!({"server":"1.2.3.4","port":443,"uuid":"11111111-1111-1111-1111-111111111111","network":"ws","security":"tls"}),
            ),
            node(
                "anytls",
                json!({"server":"1.2.3.4","port":443,"password":"secret","sni":"example.com"}),
            ),
            node(
                "mieru",
                json!({"server":"1.2.3.4","port":443,"password":"secret"}),
            ),
        ];
        let (prepared, results) = prepare_proxies(&nodes);
        assert_eq!(prepared.len(), 2);
        assert_eq!(prepared[0].id, "vless-id");
        assert!(results[0].is_none());
        assert!(results[1].is_none());
        assert!(results[2].as_ref().unwrap().message.contains("暂不支持"));
    }
    #[tokio::test]
    async fn isolates_invalid_proxy_without_rejecting_valid_batch_members() {
        let Some(mihomo) = find_mihomo_executable() else {
            return;
        };
        let work_dir = std::env::temp_dir().join(format!("vpn-parser-isolate-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&work_dir).await.unwrap();
        let proxies = vec![
            PreparedProxy {
                index: 0,
                id: "valid".into(),
                name: "valid".into(),
                config: json!({"name":"valid","type":"http","server":"example.com","port":80}),
            },
            PreparedProxy {
                index: 1,
                id: "invalid".into(),
                name: "invalid".into(),
                config: json!({"name":"invalid","type":"not-a-real-protocol","server":"example.com","port":80}),
            },
        ];

        let (valid, rejected) = isolate_invalid_proxies(proxies, &mihomo, &work_dir)
            .await
            .unwrap();

        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].id, "valid");
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].0.id, "invalid");
        assert!(!rejected[0].1.is_empty());
        let _ = tokio::fs::remove_dir_all(work_dir).await;
    }
}

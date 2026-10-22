use crate::db::Database;
use crate::kernel::{select_proxy_kernel, KernelKind};
use crate::models::{LatencyResult, Node, SpeedTestSummary, GOOGLE_PROXY_TEST_KIND};
use crate::proxy::verify_nodes_with_kernel_progress;
use crate::proxy_probe::{ProxyProbeProgress, ProxyProbeResult};
use crate::speedtest::dns::resolve_targets_parallel;
use crate::speedtest::prefilter::probe_full_resolved;
use crate::speedtest::{
    SpeedTestOptions, SpeedTestPhase, SpeedTestProgress, SpeedtestIntensity,
    TCP_PREFILTER_TIMEOUT_MS,
};
use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::info;

/// 二阶段淘汰里"谁有资格进真实握手"的判据。
///
/// 1. TCP 通了就进。
/// 2. 私网 / 端口 0 / 非法主机名直接淘汰 —— 内核拿着它也一样连不上，没必要花一次握手。
/// 3. 公网主机名：只有"我们这一侧压根没解出地址"才交给内核（内核有 DoH / 经代理那条解析路，
///    不能替它判死）；解出了地址却连不上 ⇒ 淘汰，由调用方按 `tcp_prefilter` 落库标成不可用。
pub fn reaches_proxy_stage(
    result: &LatencyResult,
    dns: Option<&Result<Vec<SocketAddr>, String>>,
) -> bool {
    if result.ok {
        return true;
    }
    if result.port == 0 || !crate::hostutil::is_public_proxy_host(&result.server) {
        return false;
    }
    match dns {
        None => true,
        Some(Err(_)) => true,
        Some(Ok(addresses)) => addresses.is_empty(),
    }
}

pub fn requires_proxy_without_tcp(protocol: &str) -> bool {
    matches!(
        protocol.to_ascii_lowercase().as_str(),
        "hysteria" | "hysteria2" | "hy2" | "tuic" | "wireguard" | "wg" | "masque"
    )
}

pub async fn run_speedtest_pipeline(
    db: &Database,
    nodes: Vec<Node>,
    opts: SpeedTestOptions,
    progress: Option<impl Fn(SpeedTestProgress) + Send + Sync>,
    network_limit: Arc<Semaphore>,
) -> Result<SpeedTestSummary> {
    // 「沿用已验证结果」：N 分钟内真验证过的节点直接不进本轮
    let nodes = if opts.reuse_verified_minutes > 0 {
        let cutoff =
            chrono::Utc::now() - chrono::Duration::minutes(opts.reuse_verified_minutes as i64);
        let (pending, reused): (Vec<Node>, Vec<Node>) = nodes.into_iter().partition(|node| {
            !(node.latency_ok.is_some() && node.latency_tested_at.is_some_and(|at| at >= cutoff))
        });
        if !reused.is_empty() {
            info!(reused = reused.len(), "沿用近期已验证的节点结果，跳过重测");
        }
        pending
    } else {
        nodes
    };
    let total = nodes.len();
    let mut proxy_nodes: HashMap<String, Node> = nodes
        .iter()
        .cloned()
        .map(|node| (node.id.clone(), node))
        .collect();
    let concurrency = opts.concurrency.clamp(1, 512) as usize;
    let (prefilter_timeout_ms, proxy_timeout_duration) = match opts.intensity {
        SpeedtestIntensity::Fast => (opts.timeout_ms.clamp(800, 1500), Duration::from_millis(3000)),
        SpeedtestIntensity::Standard => (
            opts.timeout_ms.clamp(1200, TCP_PREFILTER_TIMEOUT_MS),
            Duration::from_millis(opts.timeout_ms.clamp(3_000, 15_000) as u64),
        ),
        SpeedtestIntensity::Deep => (
            opts.timeout_ms.clamp(2000, 5000),
            Duration::from_millis(opts.timeout_ms.clamp(5_000, 25_000) as u64),
        ),
    };

    info!(
        total,
        concurrency,
        intensity = ?opts.intensity,
        timeout_ms = opts.timeout_ms,
        download = opts.measure_download,
        tcp_ping_only = opts.tcp_ping_only,
        "starting speedtest pipeline"
    );

    let started = Instant::now();

    // 1. 并发 DNS 预解析
    let dns_targets: Vec<(String, u16)> =
        nodes.iter().map(|n| (n.server.clone(), n.port)).collect();
    let resolved = resolve_targets_parallel(
        dns_targets,
        network_limit.clone(),
        Duration::from_millis(2500),
    )
    .await;

    // 2. TCP 粗筛探测
    let prefilter_semaphore = Arc::new(Semaphore::new(concurrency));
    let mut tasks = JoinSet::new();
    for node in nodes {
        let addresses = match resolved.get(&(node.server.clone(), node.port)) {
            Some(res) => res.clone(),
            None => Err("DNS 解析未覆盖".into()),
        };
        let is_udp_tunnel = requires_proxy_without_tcp(&node.protocol);
        let network_limit = network_limit.clone();
        let prefilter_semaphore = prefilter_semaphore.clone();

        tasks.spawn(async move {
            let _net_permit = match network_limit.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    return LatencyResult {
                        id: node.id,
                        name: node.name,
                        server: node.server,
                        port: node.port,
                        ok: false,
                        latency_ms: None,
                        download_kbps: None,
                        latency_kernel: None,
                        latency_test_kind: Some("concurrency_limit".into()),
                        message: "测速并发控制已关闭".into(),
                    };
                }
            };
            let _permit = match prefilter_semaphore.acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    return LatencyResult {
                        id: node.id,
                        name: node.name,
                        server: node.server,
                        port: node.port,
                        ok: false,
                        latency_ms: None,
                        download_kbps: None,
                        latency_kernel: None,
                        latency_test_kind: Some("cancelled".into()),
                        message: "测速任务已取消".into(),
                    };
                }
            };

            if is_udp_tunnel {
                let can_resolve = addresses.as_ref().map(|a| !a.is_empty()).unwrap_or(false);
                let message = if can_resolve {
                    "UDP/QUIC 协议跳过 TCP 预筛，直进内核握手".to_string()
                } else {
                    format!("DNS 解析失败: {:?}", addresses.err().unwrap_or_default())
                };
                return LatencyResult {
                    id: node.id,
                    name: node.name,
                    server: node.server,
                    port: node.port,
                    ok: can_resolve,
                    latency_ms: None,
                    download_kbps: None,
                    latency_kernel: None,
                    latency_test_kind: (!can_resolve).then(|| "dns_fail".into()),
                    message,
                };
            }

            let (ok, latency_ms, download_kbps, message) = probe_full_resolved(
                &node.server,
                node.port,
                prefilter_timeout_ms,
                opts.download_test_ms,
                false, // 统一在预检完成、延迟排序后再做分级下载测速
                addresses,
            )
            .await;

            LatencyResult {
                id: node.id,
                name: node.name,
                server: node.server,
                port: node.port,
                ok,
                latency_ms,
                download_kbps,
                latency_kernel: None,
                latency_test_kind: (!ok).then(|| "tcp_prefilter".into()),
                message,
            }
        });
    }

    let mut results = Vec::with_capacity(total);
    let mut done = 0;
    let mut ok_count = 0;
    while let Some(task_result) = tasks.join_next().await {
        if let Ok(result) = task_result {
            done += 1;
            if result.ok {
                ok_count += 1;
            }
            if let Some(progress_cb) = progress.as_ref() {
                progress_cb(SpeedTestProgress {
                    phase: SpeedTestPhase::TcpPrefilter,
                    done,
                    total,
                    ok: ok_count,
                    fail: done.saturating_sub(ok_count),
                    elapsed: started.elapsed(),
                    kernel: None,
                    batch_index: None,
                    batch_total: None,
                });
            }
            results.push(result);
        }
    }

    // === 针对开启了 measure_download 的情况，做分级梯队下载测速 ===
    if opts.measure_download {
        let mut healthy: Vec<usize> = (0..results.len())
            .filter(|&i| results[i].ok && results[i].latency_ms.is_some())
            .collect();
        healthy.sort_by_key(|&i| results[i].latency_ms.unwrap_or(i64::MAX));

        let top_limit = if opts.download_top_n > 0 {
            opts.download_top_n as usize
        } else {
            (healthy.len() / 10).clamp(1, 50)
        };

        let target_indices: Vec<usize> = healthy.into_iter().take(top_limit).collect();
        if !target_indices.is_empty() {
            info!(
                candidates = target_indices.len(),
                top_limit, "执行分级下载吞吐量测速（梯队精简）"
            );
            let mut dl_tasks = JoinSet::new();
            for index in target_indices {
                let res = &results[index];
                let host = res.server.clone();
                let port = res.port;
                let addrs = resolved
                    .get(&(host.clone(), port))
                    .cloned()
                    .unwrap_or(Ok(vec![]));
                let net_limit = network_limit.clone();
                let dl_ms = opts.download_test_ms;
                dl_tasks.spawn(async move {
                    let Ok(_permit) = net_limit.acquire_owned().await else {
                        return (index, None);
                    };
                    let (_, _, kbps, msg) =
                        probe_full_resolved(&host, port, 1500, dl_ms, true, addrs).await;
                    (index, kbps.map(|k| (k, msg)))
                });
            }

            while let Some(dl_res) = dl_tasks.join_next().await {
                if let Ok((idx, Some((kbps, msg)))) = dl_res {
                    results[idx].download_kbps = Some(kbps);
                    results[idx].message = format!("{} / {}", results[idx].message, msg);
                }
            }
        }
    }

    // === 双模测速分支：如果开启了 tcp_ping_only，直接完成并落库，跳过所有内核握手 ===
    if opts.tcp_ping_only {
        info!("【快速 TCP Ping 模式】生效：直接写入 TCP 延迟，跳过真实内核代理握手");
        let updates: Vec<_> = results
            .iter()
            .map(|r| {
                (
                    r.id.clone(),
                    r.ok,
                    r.latency_ms,
                    r.download_kbps,
                    None,
                    Some("tcp_ping".to_string()),
                )
            })
            .collect();

        for chunk in updates.chunks(500) {
            db.update_node_speeds_batch(chunk).await?;
        }

        results.sort_by(|a, b| match (a.ok, b.ok) {
            (true, true) => match (a.latency_ms, b.latency_ms) {
                (Some(am), Some(bm)) => am.cmp(&bm),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.name.cmp(&b.name),
            },
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => a.name.cmp(&b.name),
        });

        let ok = results.iter().filter(|r| r.ok).count();
        let fail = results.len() - ok;
        let latencies: Vec<_> = results.iter().filter_map(|r| r.latency_ms).collect();
        let downloads: Vec<_> = results.iter().filter_map(|r| r.download_kbps).collect();

        return Ok(SpeedTestSummary {
            total: results.len(),
            ok,
            fail,
            avg_ms: (!latencies.is_empty())
                .then(|| latencies.iter().sum::<i64>() as f64 / latencies.len() as f64),
            avg_download_kbps: (!downloads.is_empty())
                .then(|| downloads.iter().sum::<f64>() / downloads.len() as f64),
            results,
        });
    }

    // 3. 完整真实代理握手阶段
    //
    // 二阶段淘汰（用户 2026-10-22 定）：**TCP 预筛没过的节点不再进真实握手**。
    // 以前的条件是 `result.ok || is_public_proxy_host(server)`，而 `is_public_proxy_host`
    // 只排除 localhost/私网/保留地址 ⇒ 所有公网域名一律进握手（他那一库 16,066 个节点里
    // 15,329 个的 `latency_test_kind` 是 google_proxy，就是这句话的结果）：预筛白跑一轮，
    // 还在进度条上报出一个"成功 10000"这种谁都不该当成"可用"的数字。
    // 保留一个例外：DNS 在我们这一侧根本没解出来的 —— 内核有自己那条解析路（DoH / 经代理），
    // 不该替内核判死。UDP/QUIC 协议在上面已按"能解析就放行"处理，不经过 TCP 判定。
    let proxy_candidates: Vec<Node> = results
        .iter()
        .filter(|result| {
            reaches_proxy_stage(
                result,
                resolved.get(&(result.server.clone(), result.port)),
            )
        })
        .filter_map(|result| proxy_nodes.remove(&result.id))
        .collect();

    let prefilter_failures: Vec<_> = results
        .iter()
        .filter(|result| {
            !result.ok && !proxy_candidates.iter().any(|c| c.id == result.id)
        })
        .map(|result| {
            (
                result.id.clone(),
                false,
                result.latency_ms,
                None,
                None,
                Some("tcp_prefilter".to_string()),
            )
        })
        .collect();
    for chunk in prefilter_failures.chunks(500) {
        db.update_node_speeds_batch(chunk).await?;
    }

    info!(
        candidates = proxy_candidates.len(),
        "starting real proxy verification"
    );

    let proxy_timeout = proxy_timeout_duration;
    let proxy_batch_size = if opts.persistent_kernel {
        crate::proxy_probe::PERSISTENT_MIHOMO_BATCH_SIZE
    } else {
        256
    };
    let conditional_isolation = opts.conditional_isolation;
    let proxy_total = proxy_candidates.len();

    // 同物理落地节点剪枝
    let (actual_candidates, physical_mirrors) = if opts.dedup_egress {
        let mut groups: std::collections::HashMap<String, Vec<Node>> =
            std::collections::HashMap::new();
        for node in proxy_candidates {
            let physical_key =
                if let Some(Ok(addrs)) = resolved.get(&(node.server.clone(), node.port)) {
                    if let Some(first) = addrs.first() {
                        format!(
                            "{}:{}:{}",
                            first.ip(),
                            node.port,
                            node.protocol.to_ascii_lowercase()
                        )
                    } else {
                        format!(
                            "{}:{}:{}",
                            node.server.to_ascii_lowercase(),
                            node.port,
                            node.protocol.to_ascii_lowercase()
                        )
                    }
                } else {
                    format!(
                        "{}:{}:{}",
                        node.server.to_ascii_lowercase(),
                        node.port,
                        node.protocol.to_ascii_lowercase()
                    )
                };
            groups.entry(physical_key).or_default().push(node);
        }

        let mut representatives = Vec::with_capacity(groups.len());
        let mut mirrors: std::collections::HashMap<String, Vec<Node>> =
            std::collections::HashMap::new();
        for (_key, mut group) in groups {
            let rep = group.remove(0);
            if !group.is_empty() {
                mirrors.insert(rep.id.clone(), group);
            }
            representatives.push(rep);
        }
        info!(
            representatives = representatives.len(),
            mirrors = mirrors.values().map(|v| v.len()).sum::<usize>(),
            "同物理落地剪枝生效：过滤镜像重复握手"
        );
        (representatives, mirrors)
    } else {
        (proxy_candidates, std::collections::HashMap::new())
    };

    let mut mihomo_nodes = Vec::new();
    let mut sing_box_nodes = Vec::new();
    let mut xray_nodes = Vec::new();
    let mut proxy_results = Vec::new();
    let mut result_kernels = HashMap::new();

    for node in actual_candidates {
        match select_proxy_kernel(&node.protocol) {
            Ok(KernelKind::Mihomo) => {
                result_kernels.insert(node.id.clone(), KernelKind::Mihomo);
                mihomo_nodes.push(node);
            }
            Ok(KernelKind::SingBox) => {
                result_kernels.insert(node.id.clone(), KernelKind::SingBox);
                sing_box_nodes.push(node);
            }
            Ok(KernelKind::Xray) => {
                result_kernels.insert(node.id.clone(), KernelKind::Xray);
                xray_nodes.push(node);
            }
            Err(error) => proxy_results.push(ProxyProbeResult {
                id: node.id,
                ok: false,
                latency_ms: None,
                message: format!("kernel select error: {error}"),
            }),
        }
    }

    let rejected_done = proxy_results.len();
    let rejected_ok = 0;

    if opts.parallel_kernels {
        let mut kernel_tasks = JoinSet::new();
        if !mihomo_nodes.is_empty() {
            let limit = network_limit.clone();
            kernel_tasks.spawn(async move {
                verify_nodes_with_kernel_progress(
                    KernelKind::Mihomo,
                    mihomo_nodes,
                    proxy_timeout,
                    None,
                    limit,
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await
            });
        }
        if !sing_box_nodes.is_empty() {
            let limit = network_limit.clone();
            kernel_tasks.spawn(async move {
                verify_nodes_with_kernel_progress(
                    KernelKind::SingBox,
                    sing_box_nodes,
                    proxy_timeout,
                    None,
                    limit,
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await
            });
        }
        if !xray_nodes.is_empty() {
            let limit = network_limit.clone();
            kernel_tasks.spawn(async move {
                verify_nodes_with_kernel_progress(
                    KernelKind::Xray,
                    xray_nodes,
                    proxy_timeout,
                    None,
                    limit,
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await
            });
        }
        while let Some(res) = kernel_tasks.join_next().await {
            match res {
                Ok(Ok(items)) => proxy_results.extend(items),
                Ok(Err(e)) => tracing::error!("parallel kernel verification failed: {e}"),
                Err(e) => tracing::error!("parallel kernel task panicked: {e}"),
            }
        }
    } else {
        let mihomo_results = run_sequential_kernel(
            KernelKind::Mihomo,
            mihomo_nodes,
            proxy_timeout,
            network_limit.clone(),
            proxy_batch_size,
            conditional_isolation,
            progress.as_ref(),
            rejected_done,
            rejected_ok,
            proxy_total,
            started,
        )
        .await?;
        let done_after_mihomo = rejected_done + mihomo_results.len();
        let ok_after_mihomo = rejected_ok + mihomo_results.iter().filter(|r| r.ok).count();
        proxy_results.extend(mihomo_results);

        let sing_box_results = run_sequential_kernel(
            KernelKind::SingBox,
            sing_box_nodes,
            proxy_timeout,
            network_limit.clone(),
            proxy_batch_size,
            conditional_isolation,
            progress.as_ref(),
            done_after_mihomo,
            ok_after_mihomo,
            proxy_total,
            started,
        )
        .await?;
        let done_after_sing_box = done_after_mihomo + sing_box_results.len();
        let ok_after_sing_box = ok_after_mihomo + sing_box_results.iter().filter(|r| r.ok).count();
        proxy_results.extend(sing_box_results);

        let xray_results = run_sequential_kernel(
            KernelKind::Xray,
            xray_nodes,
            proxy_timeout,
            network_limit,
            proxy_batch_size,
            conditional_isolation,
            progress.as_ref(),
            done_after_sing_box,
            ok_after_sing_box,
            proxy_total,
            started,
        )
        .await?;
        proxy_results.extend(xray_results);
    }

    if let Some(progress) = progress.as_ref() {
        let ok = proxy_results.iter().filter(|result| result.ok).count();
        progress(SpeedTestProgress {
            phase: SpeedTestPhase::RealProxy,
            done: proxy_results.len(),
            total: proxy_total,
            ok,
            fail: proxy_results.len().saturating_sub(ok),
            elapsed: started.elapsed(),
            kernel: None,
            batch_index: None,
            batch_total: None,
        });
    }

    // 复用同物理落地的测速结果给镜像节点
    for (rep_id, mirror_nodes) in &physical_mirrors {
        if let Some(rep_res) = proxy_results.iter().find(|r| r.id == *rep_id).cloned() {
            for mirror in mirror_nodes {
                proxy_results.push(ProxyProbeResult {
                    id: mirror.id.clone(),
                    ok: rep_res.ok,
                    latency_ms: rep_res.latency_ms,
                    message: format!("{} (同落地复用)", rep_res.message),
                });
                if let Some(k) = result_kernels.get(rep_id) {
                    result_kernels.insert(mirror.id.clone(), *k);
                }
            }
        }
    }

    let mut proxy_results_map: HashMap<String, ProxyProbeResult> = proxy_results
        .into_iter()
        .map(|result| (result.id.clone(), result))
        .collect();

    for result in &mut results {
        if let Some(proxy) = proxy_results_map.remove(&result.id) {
            result.ok = proxy.ok;
            result.latency_ms = proxy.latency_ms;
            result.download_kbps = None;
            if let Some(kernel) = result_kernels.get(&result.id) {
                result.latency_kernel = Some(kernel.as_str().into());
                result.latency_test_kind = Some(GOOGLE_PROXY_TEST_KIND.into());
            }
            result.message = proxy.message;
        }
    }

    let updates: Vec<_> = results
        .iter()
        .map(|result| {
            (
                result.id.clone(),
                result.ok,
                result.latency_ms,
                result.download_kbps,
                result.latency_kernel.clone(),
                result.latency_test_kind.clone(),
            )
        })
        .collect();

    for chunk in updates.chunks(500) {
        db.update_node_speeds_batch(chunk).await?;
    }

    let ok = results.iter().filter(|result| result.ok).count();
    let fail = results.len() - ok;
    let avg_ms = {
        let vals: Vec<f64> = results
            .iter()
            .filter_map(|r| r.latency_ms.map(|v| v as f64))
            .collect();
        if vals.is_empty() {
            None
        } else {
            Some(vals.iter().sum::<f64>() / vals.len() as f64)
        }
    };
    let avg_download_kbps = {
        let vals: Vec<f64> = results.iter().filter_map(|r| r.download_kbps).collect();
        if vals.is_empty() {
            None
        } else {
            Some(vals.iter().sum::<f64>() / vals.len() as f64)
        }
    };

    results.sort_by(|a, b| match (a.ok, b.ok) {
        (true, true) => match (a.latency_ms, b.latency_ms) {
            (Some(am), Some(bm)) => am.cmp(&bm),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        },
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => a.name.cmp(&b.name),
    });

    info!(
        ok,
        fail,
        ?avg_ms,
        ?avg_download_kbps,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "speedtest pipeline finished"
    );

    Ok(SpeedTestSummary {
        total: results.len(),
        ok,
        fail,
        avg_ms,
        avg_download_kbps,
        results,
    })
}

async fn run_sequential_kernel<P>(
    kernel: KernelKind,
    nodes: Vec<Node>,
    proxy_timeout: Duration,
    network_limit: Arc<Semaphore>,
    proxy_batch_size: usize,
    conditional_isolation: bool,
    progress: Option<&P>,
    rejected_done: usize,
    rejected_ok: usize,
    proxy_total: usize,
    started: Instant,
) -> Result<Vec<ProxyProbeResult>>
where
    P: Fn(SpeedTestProgress) + Send + Sync,
{
    if let Some(progress) = progress {
        let proxy_progress = |proxy: ProxyProbeProgress| {
            let done = rejected_done + proxy.done;
            let ok = rejected_ok + proxy.ok;
            progress(SpeedTestProgress {
                phase: SpeedTestPhase::RealProxy,
                done,
                total: proxy_total,
                ok,
                fail: done.saturating_sub(ok),
                elapsed: started.elapsed(),
                kernel: Some(kernel),
                batch_index: proxy.batch_index,
                batch_total: proxy.batch_total,
            });
        };
        verify_nodes_with_kernel_progress(
            kernel,
            nodes,
            proxy_timeout,
            Some(&proxy_progress),
            network_limit,
            proxy_batch_size,
            conditional_isolation,
        )
        .await
    } else {
        verify_nodes_with_kernel_progress(
            kernel,
            nodes,
            proxy_timeout,
            None,
            network_limit,
            proxy_batch_size,
            conditional_isolation,
        )
        .await
    }
}

#[cfg(test)]
mod two_stage_tests {
    use super::reaches_proxy_stage;
    use crate::models::LatencyResult;
    use std::net::SocketAddr;

    fn result(ok: bool) -> LatencyResult {
        LatencyResult {
            id: "n".into(),
            name: "n".into(),
            server: "cdn.example.com".into(),
            port: 443,
            ok,
            latency_ms: ok.then_some(30),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: (!ok).then(|| "tcp_prefilter".into()),
            message: String::new(),
        }
    }

    fn resolved_to(list: &str) -> Result<Vec<SocketAddr>, String> {
        Ok(vec![list.parse::<SocketAddr>().expect("测试用的地址")])
    }

    /// 这条判据本身就是"二阶段淘汰"：TCP 没通 + 地址解得出来 ⇒ 不再进真实握手。
    /// 旧规则在这里放行的理由是"它是个公网主机"，而那条件排除的只有 localhost/私网 —— 等于全放行。
    #[test]
    fn tcp_failure_with_a_resolved_address_is_eliminated() {
        let dead = result(false);
        assert!(
            !reaches_proxy_stage(&dead, Some(&resolved_to("188.114.97.6:443"))),
            "解出了地址却连不上，应该淘汰"
        );
        assert!(reaches_proxy_stage(
            &result(true),
            Some(&resolved_to("188.114.97.6:443"))
        ));
    }

    /// 例外：DNS 在我们这一侧就没解出来 —— 内核有自己那条解析路（DoH / 经代理），不能替它判死。
    #[test]
    fn dns_side_failures_still_reach_the_kernel() {
        let dead = result(false);
        let no_record: Result<Vec<SocketAddr>, String> = Ok(vec![]);
        assert!(reaches_proxy_stage(&dead, None), "DNS 没覆盖到的交给内核");
        assert!(
            reaches_proxy_stage(&dead, Some(&Err("no records".into()))),
            "DNS 解析失败的交给内核"
        );
        assert!(
            reaches_proxy_stage(&dead, Some(&no_record)),
            "解出空列表（本地看不到记录）的交给内核"
        );
    }

    /// 私网 / 端口 0 直接淘汰 —— 内核拿着它也一样连不上。
    /// 这条是被 `persists_tcp_prefilter_failure_as_unavailable` 教出来的：那台 `127.0.0.1:80`
    /// 压根没进 DNS 表，只看"DNS 没解出来"就会把它错交给内核（第一版判据就在这儿翻的车）。
    #[test]
    fn private_hosts_never_reach_the_kernel() {
        let mut lan = result(false);
        lan.server = "127.0.0.1".into();
        assert!(
            !reaches_proxy_stage(&lan, None),
            "私网主机不该交给内核，该按 tcp_prefilter 落库"
        );
        let mut port_zero = result(false);
        port_zero.port = 0;
        assert!(
            !reaches_proxy_stage(&port_zero, None),
            "端口 0 不该交给内核"
        );
    }
}

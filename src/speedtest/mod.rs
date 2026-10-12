use crate::db::Database;
use crate::hostutil::is_public_proxy_host;
use crate::kernel::{select_proxy_kernel, KernelKind};
use crate::models::{LatencyResult, Node, SpeedTestSummary, GOOGLE_PROXY_TEST_KIND};
use crate::parser::node_fingerprint;
use crate::proxy::verify_nodes_with_kernel_progress;
use crate::proxy_probe::{ProxyProbeProgress, ProxyProbeResult};
use crate::tasks::{TaskKind, TaskPhase, TaskTracker};
use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::info;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeedtestIntensity {
    Fast,
    #[default]
    Standard,
    Deep,
}

impl SpeedtestIntensity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "⚡ 极速模式 (轻度)",
            Self::Standard => "⚖️ 标准模式 (平衡)",
            Self::Deep => "🔬 深度模式 (严苛)",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Fast => "并发 256，短超时 500ms，极速筛除大量离线僵尸节点",
            Self::Standard => "并发 128，标准超时 1500ms，TCP 预筛 + 真实内核代理握手",
            Self::Deep => "并发 64，高容忍长超时 3500ms，严苛校验真实可用性",
        }
    }
}

/// TCP connect latency + optional download throughput probe.
#[derive(Clone)]
pub struct SpeedTester {
    db: Database,
    tasks: TaskTracker,
}

#[derive(Clone, Copy)]
pub struct SpeedTestOptions {
    pub concurrency: u32,
    pub timeout_ms: u32,
    pub measure_download: bool,
    pub download_test_ms: u32,
    pub intensity: SpeedtestIntensity,
    /// 以下五项对应「系统设置 → 测速优化」的开关，默认全部关闭 = 保持老行为。
    pub parallel_kernels: bool,
    pub persistent_kernel: bool,
    pub reuse_verified_minutes: u32,
    pub conditional_isolation: bool,
    pub download_top_n: u32,
}

/// The TCP prefilter is only a coarse gate before the real proxy handshake, so it
/// uses a short timeout independent of the (longer) real-proxy timeout. A node whose
/// TCP handshake needs more than this is almost never a usable proxy, and paying the
/// full user timeout here is what made bulk "test all" scans slow.
const TCP_PREFILTER_TIMEOUT_MS: u32 = 800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedTestPhase {
    TcpPrefilter,
    RealProxy,
}

#[derive(Debug, Clone, Copy)]
pub struct SpeedTestProgress {
    pub phase: SpeedTestPhase,
    pub done: usize,
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    pub elapsed: Duration,
    pub kernel: Option<KernelKind>,
    pub batch_index: Option<usize>,
    pub batch_total: Option<usize>,
}

impl Default for SpeedTestOptions {
    fn default() -> Self {
        Self {
            // High concurrency + short timeout keeps bulk scans usable.
            concurrency: 128,
            timeout_ms: 1500,
            // Download probe is best-effort and slow; off by default.
            measure_download: false,
            download_test_ms: 800,
            intensity: SpeedtestIntensity::Standard,
            parallel_kernels: false,
            persistent_kernel: false,
            reuse_verified_minutes: 0,
            conditional_isolation: false,
            download_top_n: 0,
        }
    }
}

impl SpeedTester {
    pub fn new(db: Database, tasks: TaskTracker) -> Self {
        Self { db, tasks }
    }

    /// Latency + optional download throughput.
    ///
    /// This is **TCP reachability**, not a real proxy handshake.
    /// Free lists often open TCP on dead/decoy ports — clients like v2rayN will still
    /// show latency `-1` when VMess/VLESS/Trojan/SS handshake fails.
    pub async fn probe_full(
        host: &str,
        port: u16,
        timeout_ms: u32,
        measure_download: bool,
        download_test_ms: u32,
    ) -> (bool, Option<i64>, Option<f64>, String) {
        if !is_public_proxy_host(host) {
            return (
                false,
                None,
                None,
                format!("skip invalid/private host: {host}"),
            );
        }
        if port == 0 {
            return (false, None, None, "invalid port 0".into());
        }

        let timeout = Duration::from_millis(timeout_ms.max(200) as u64);
        let (stream, latency_ms) = match connect_fast(host, port, timeout).await {
            Ok(v) => v,
            Err(msg) => return (false, None, None, msg),
        };

        if !measure_download {
            return (
                true,
                Some(latency_ms),
                None,
                format!("{latency_ms} ms (TCP)"),
            );
        }

        let download_ms = download_test_ms.clamp(200, 5_000) as u64;
        let (kbps, detail) = measure_tcp_download(stream, host, download_ms).await;
        let kbps = kbps.and_then(|k| {
            if k.is_finite() && k > 0.0 && k < 20_480.0 {
                Some(k)
            } else {
                None
            }
        });
        let msg = match kbps {
            Some(k) => format!("{latency_ms} ms / {k:.1} KB/s"),
            None => format!("{latency_ms} ms / 下载不可测 ({detail})"),
        };
        (true, Some(latency_ms), kbps, msg)
    }

    async fn probe_full_resolved(
        host: &str,
        port: u16,
        timeout_ms: u32,
        download_test_ms: u32,
        measure_download: bool,
        addresses: std::result::Result<Vec<SocketAddr>, String>,
    ) -> (bool, Option<i64>, Option<f64>, String) {
        if !is_public_proxy_host(host) || port == 0 {
            return (
                false,
                None,
                None,
                format!("skip invalid/private host: {host}"),
            );
        }
        let timeout = Duration::from_millis(timeout_ms.max(200) as u64);
        let addresses = match addresses {
            Ok(addresses) => addresses,
            Err(error) => return (false, None, None, error),
        };
        let (stream, latency_ms) = match connect_resolved(addresses, timeout).await {
            Ok(value) => value,
            Err(error) => return (false, None, None, error),
        };
        if !measure_download {
            drop(stream);
            return (
                true,
                Some(latency_ms),
                None,
                format!("{latency_ms} ms (TCP)"),
            );
        }
        // 尽力而为：加密代理大多不会回应这个明文 GET，测不到就留空。
        let download_ms = download_test_ms.clamp(200, 5_000) as u64;
        let (kbps, detail) = measure_tcp_download(stream, host, download_ms).await;
        let kbps = kbps.filter(|value| value.is_finite() && *value > 0.0 && *value < 20_480.0);
        let message = match kbps {
            Some(value) => format!("{latency_ms} ms / {value:.1} KB/s"),
            None => format!("{latency_ms} ms / 下载不可测 ({detail})"),
        };
        (true, Some(latency_ms), kbps, message)
    }

    pub async fn test_one(&self, node_id: &str, opts: SpeedTestOptions) -> Result<LatencyResult> {
        let mut summary = self.test_many(Some(vec![node_id.to_owned()]), opts).await?;
        summary
            .results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("node not found"))
    }

    /// Batch test: TCP prefilter, then real proxy handshake and HTTP latency through Mihomo.
    pub async fn test_many(
        &self,
        node_ids: Option<Vec<String>>,
        opts: SpeedTestOptions,
    ) -> Result<SpeedTestSummary> {
        self.test_many_with_progress(node_ids, opts, None::<fn(SpeedTestProgress)>)
            .await
    }

    /// Test all subscription databases through one deduplicated global work queue.
    pub async fn test_all_sources(&self, opts: SpeedTestOptions) -> Result<SpeedTestSummary> {
        self.test_all_sources_with_progress(opts, None::<fn(usize, usize)>)
            .await
    }

    pub async fn test_all_sources_with_progress(
        &self,
        opts: SpeedTestOptions,
        progress: Option<impl Fn(usize, usize) + Send + Sync>,
    ) -> Result<SpeedTestSummary> {
        self.test_all_sources_globally(opts, progress).await
    }

    /// Global work queue for "test all". Sources often publish the same nodes;
    /// testing every copy separately amplified both TCP probes and kernel startups.
    /// A representative is tested once and its strict Google result is written back
    /// to all subscriptions that contain the same fingerprint.
    async fn test_all_sources_globally(
        &self,
        opts: SpeedTestOptions,
        progress: Option<impl Fn(usize, usize) + Send + Sync>,
    ) -> Result<SpeedTestSummary> {
        let groups = self.db.list_node_groups(None, None, None).await?;
        let source_keys: Vec<String> = groups
            .iter()
            .map(|group| {
                group
                    .source_id
                    .clone()
                    .unwrap_or_else(|| crate::db::UNGROUPED_SOURCE_ID.into())
            })
            .collect();
        let task_id = self.tasks.start(
            TaskKind::Speedtest,
            groups.iter().map(|group| {
                (
                    group
                        .source_id
                        .clone()
                        .unwrap_or_else(|| crate::db::UNGROUPED_SOURCE_ID.into()),
                    group.source_name.clone(),
                )
            }),
        );
        let all_nodes = self.db.list_nodes_for_speedtest().await?;
        if all_nodes.is_empty() {
            anyhow::bail!("没有可测速节点");
        }

        let mut copies: HashMap<String, Vec<Node>> = HashMap::new();
        let mut source_counts: HashMap<String, usize> = HashMap::new();
        for node in all_nodes {
            let source_key = node
                .source_id
                .clone()
                .unwrap_or_else(|| crate::db::UNGROUPED_SOURCE_ID.into());
            *source_counts.entry(source_key).or_default() += 1;
            copies
                .entry(node_fingerprint(&node))
                .or_default()
                .push(node);
        }
        for (source_id, count) in &source_counts {
            self.tasks.nodes(&task_id, source_id, *count);
            self.tasks
                .phase(&task_id, source_id, TaskPhase::TcpPrefilter);
        }

        let mut representatives = Vec::with_capacity(copies.len());
        let mut representative_fingerprints = HashMap::with_capacity(copies.len());
        for (fingerprint, nodes) in &copies {
            let representative = nodes[0].clone();
            representative_fingerprints.insert(representative.id.clone(), fingerprint.clone());
            representatives.push(representative);
        }
        info!(
            total_nodes = source_counts.values().sum::<usize>(),
            unique_nodes = representatives.len(),
            duplicates = source_counts.values().sum::<usize>() - representatives.len(),
            concurrency = opts.concurrency,
            "starting globally deduplicated speedtest"
        );

        let progress_tasks = self.tasks.clone();
        let progress_task_id = task_id.clone();
        let progress_sources = source_keys.clone();
        let progress_callback = progress.as_ref();
        let speedtest_started = Instant::now();
        let summary = self
            .run_speedtest(
                representatives,
                opts,
                Some(move |state: SpeedTestProgress| {
                    let phase = match state.phase {
                        SpeedTestPhase::TcpPrefilter => TaskPhase::TcpPrefilter,
                        SpeedTestPhase::RealProxy => TaskPhase::GoogleVerify,
                    };
                    // 每个节点都刷 33 个源的阶段太浪费（一轮 19000 节点 = 62 万次
                    // 加锁+扫描），每 200 个节点或收尾时刷一次即可。
                    if state.done % 200 == 0 || state.done >= state.total {
                        for source_id in &progress_sources {
                            progress_tasks.phase(&progress_task_id, source_id, phase);
                        }
                    }
                    if let Some(progress) = progress_callback {
                        progress(state.done, state.total);
                    }
                }),
                Arc::new(Semaphore::new(opts.concurrency.clamp(1, 512) as usize)),
            )
            .await?;

        let representative_results: HashMap<String, LatencyResult> = summary
            .results
            .into_iter()
            .map(|result| (result.id.clone(), result))
            .collect();
        let mut results = Vec::new();
        let mut source_stats: HashMap<String, (usize, usize)> = HashMap::new();
        let mut duplicate_updates = Vec::new();
        for (representative_id, fingerprint) in representative_fingerprints {
            let Some(result) = representative_results.get(&representative_id) else {
                continue;
            };
            for node in &copies[&fingerprint] {
                let source_id = node
                    .source_id
                    .clone()
                    .unwrap_or_else(|| crate::db::UNGROUPED_SOURCE_ID.into());
                let entry = source_stats.entry(source_id).or_default();
                if result.ok {
                    entry.0 += 1;
                } else {
                    entry.1 += 1;
                }
                let mut copy = result.clone();
                copy.id = node.id.clone();
                copy.name = node.name.clone();
                copy.server = node.server.clone();
                copy.port = node.port;
                if copy.id != representative_id {
                    duplicate_updates.push((
                        copy.id.clone(),
                        copy.ok,
                        copy.latency_ms,
                        copy.download_kbps,
                        copy.latency_kernel.clone(),
                        copy.latency_test_kind.clone(),
                    ));
                }
                results.push(copy);
            }
        }
        let update_started = Instant::now();
        for chunk in duplicate_updates.chunks(500) {
            self.db.update_node_speeds_batch(chunk).await?;
        }
        for source_id in &source_keys {
            self.tasks.phase(&task_id, source_id, TaskPhase::Updating);
            self.tasks.timings(
                &task_id,
                source_id,
                None,
                None,
                None,
                Some(speedtest_started.elapsed().as_millis() as u64),
                Some(update_started.elapsed().as_millis() as u64),
            );
            let (ok, fail) = source_stats.get(source_id).copied().unwrap_or_default();
            self.tasks.complete(
                &task_id,
                source_id,
                format!("严格 Google 校验完成：成功 {ok}，失败 {fail}"),
            );
        }
        if let Some(progress) = progress.as_ref() {
            progress(source_keys.len(), source_keys.len());
        }
        build_speedtest_summary(results)
    }

    pub async fn test_many_with_progress(
        &self,
        node_ids: Option<Vec<String>>,
        opts: SpeedTestOptions,
        progress: Option<impl Fn(SpeedTestProgress) + Send + Sync>,
    ) -> Result<SpeedTestSummary> {
        let network_limit = Arc::new(Semaphore::new(opts.concurrency.clamp(1, 512) as usize));
        self.test_many_with_progress_limited(node_ids, opts, progress, network_limit)
            .await
    }

    async fn test_many_with_progress_limited(
        &self,
        node_ids: Option<Vec<String>>,
        opts: SpeedTestOptions,
        progress: Option<impl Fn(SpeedTestProgress) + Send + Sync>,
        network_limit: Arc<Semaphore>,
    ) -> Result<SpeedTestSummary> {
        let nodes = if let Some(ids) = node_ids {
            // 一次批量取，别再 `for id in ids { get_node(id) }`（那是每个 id 扇出
            // 一遍全部分库）。
            self.db.get_nodes(&ids).await?
        } else {
            self.db.list_nodes_for_speedtest().await?
        };
        self.run_speedtest(nodes, opts, progress, network_limit)
            .await
    }

    async fn run_speedtest(
        &self,
        nodes: Vec<Node>,
        opts: SpeedTestOptions,
        progress: Option<impl Fn(SpeedTestProgress) + Send + Sync>,
        network_limit: Arc<Semaphore>,
    ) -> Result<SpeedTestSummary> {
        // 「沿用已验证结果」：N 分钟内真验证过的节点直接不进本轮，避免每轮都在
        // 重测同一批节点（这是全库重测里最大的一笔重复开销）。
        let nodes = if opts.reuse_verified_minutes > 0 {
            let cutoff =
                chrono::Utc::now() - chrono::Duration::minutes(opts.reuse_verified_minutes as i64);
            let (pending, reused): (Vec<Node>, Vec<Node>) = nodes.into_iter().partition(|node| {
                !(node.latency_ok.is_some()
                    && node.latency_tested_at.is_some_and(|at| at >= cutoff))
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
            SpeedtestIntensity::Fast => (opts.timeout_ms.min(500), Duration::from_millis(1800)),
            SpeedtestIntensity::Standard => (
                opts.timeout_ms.min(TCP_PREFILTER_TIMEOUT_MS),
                Duration::from_millis(opts.timeout_ms.clamp(2_500, 15_000) as u64),
            ),
            SpeedtestIntensity::Deep => (
                opts.timeout_ms.max(1200),
                Duration::from_millis(opts.timeout_ms.clamp(4_000, 20_000) as u64),
            ),
        };
        info!(
            total,
            concurrency,
            intensity = ?opts.intensity,
            timeout_ms = opts.timeout_ms,
            download = opts.measure_download,
            "starting speedtest"
        );

        let started = Instant::now();
        let mut resolution_tasks = JoinSet::new();
        let mut unique_targets = std::collections::HashSet::new();
        for node in &nodes {
            if is_public_proxy_host(&node.server)
                && unique_targets.insert((node.server.clone(), node.port))
            {
                let host = node.server.clone();
                let port = node.port;
                let network_limit = network_limit.clone();
                resolution_tasks.spawn(async move {
                    let _permit = match network_limit.acquire_owned().await {
                        Ok(permit) => permit,
                        Err(_) => {
                            return ((host, port), Err("测速并发控制已关闭".to_string()));
                        }
                    };
                    let target = format!("{host}:{port}");
                    let result = match tokio::time::timeout(
                        Duration::from_millis(800),
                        tokio::net::lookup_host(target.as_str()),
                    )
                    .await
                    {
                        Ok(Ok(iter)) => {
                            let mut addresses = iter.collect::<Vec<_>>();
                            addresses.sort_by_key(|addr| if addr.is_ipv4() { 0u8 } else { 1u8 });
                            Ok(addresses)
                        }
                        Ok(Err(error)) => Err(format!("dns error: {error}")),
                        Err(_) => Err("dns timeout".into()),
                    };
                    ((host, port), result)
                });
            }
        }
        let mut resolved = HashMap::new();
        while let Some(joined) = resolution_tasks.join_next().await {
            if let Ok((target, result)) = joined {
                resolved.insert(target, result);
            }
        }
        let mut iter = nodes.into_iter();
        let mut inflight: JoinSet<(
            String,
            String,
            String,
            u16,
            bool,
            Option<i64>,
            Option<f64>,
            String,
        )> = JoinSet::new();
        let mut results = Vec::with_capacity(total);
        // 运行中的成功计数：进度回调是每个节点一次的，不能再回头扫 results。
        let mut ok_count = 0usize;
        let mut done = 0usize;

        loop {
            while inflight.len() < concurrency {
                let Some(node) = iter.next() else {
                    break;
                };
                // Prefilter timeout based on selected intensity.
                let timeout_ms = prefilter_timeout_ms;
                let download_test_ms = opts.download_test_ms;
                // 设了「只做前 N 名」时主扫描不测下载，排完序再补测那一批。
                let measure_download = opts.measure_download && opts.download_top_n == 0;
                let addresses = resolved
                    .get(&(node.server.clone(), node.port))
                    .cloned()
                    .unwrap_or_else(|| Err("dns result missing".into()));
                let network_limit = network_limit.clone();
                inflight.spawn(async move {
                    let _permit = network_limit.acquire_owned().await;
                    if _permit.is_err() {
                        return (
                            node.id,
                            node.name,
                            node.server,
                            node.port,
                            false,
                            None,
                            None,
                            "测速并发控制已关闭".into(),
                        );
                    }
                    let (ok, ms, kbps, message) = SpeedTester::probe_full_resolved(
                        &node.server,
                        node.port,
                        timeout_ms,
                        download_test_ms,
                        measure_download,
                        addresses,
                    )
                    .await;
                    (
                        node.id,
                        node.name,
                        node.server,
                        node.port,
                        ok,
                        ms,
                        kbps,
                        message,
                    )
                });
            }

            let Some(joined) = inflight.join_next().await else {
                break;
            };

            let Ok((id, name, server, port, ok, ms, kbps, message)) = joined else {
                continue;
            };

            results.push(LatencyResult {
                id,
                name,
                server,
                port,
                ok,
                latency_ms: ms,
                download_kbps: kbps,
                latency_kernel: None,
                latency_test_kind: (!ok).then(|| "tcp_prefilter".into()),
                message,
            });
            done += 1;
            if ok {
                ok_count += 1;
            }

            report_speedtest_progress(
                &progress,
                SpeedTestPhase::TcpPrefilter,
                done,
                total,
                ok_count,
                started.elapsed(),
            );

            if done % 500 == 0 || done == total {
                let ok_n = ok_count;
                info!(
                    done,
                    total,
                    ok = ok_n,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "speedtest progress"
                );
            }
        }

        if opts.measure_download && opts.download_top_n > 0 {
            let mut ranked: Vec<usize> = (0..results.len()).filter(|&i| results[i].ok).collect();
            ranked.sort_by_key(|&i| results[i].latency_ms.unwrap_or(i64::MAX));
            let mut download_tasks = JoinSet::new();
            for &index in ranked.iter().take(opts.download_top_n as usize) {
                let Some(node) = proxy_nodes.get(&results[index].id) else {
                    continue;
                };
                let host = node.server.clone();
                let port = node.port;
                let addresses = resolved
                    .get(&(host.clone(), port))
                    .cloned()
                    .unwrap_or_else(|| Err("dns result missing".to_string()));
                let network_limit = network_limit.clone();
                download_tasks.spawn(async move {
                    let Ok(_permit) = network_limit.acquire_owned().await else {
                        return (index, None);
                    };
                    let (_, _, kbps, message) = SpeedTester::probe_full_resolved(
                        &host,
                        port,
                        prefilter_timeout_ms,
                        opts.download_test_ms,
                        true,
                        addresses,
                    )
                    .await;
                    (index, kbps.map(|value| (value, message)))
                });
            }
            let mut measured = 0usize;
            while let Some(joined) = download_tasks.join_next().await {
                if let Ok((index, Some((kbps, _message)))) = joined {
                    results[index].download_kbps = Some(kbps);
                    measured += 1;
                }
            }
            info!(measured, "按延迟排名补测了下载速率");
        }

        let proxy_candidates: Vec<Node> = results
            .iter()
            .filter_map(|result| {
                let node = proxy_nodes.remove(&result.id)?;
                if result.ok || requires_proxy_without_tcp(&node.protocol) {
                    Some(node)
                } else {
                    None
                }
            })
            .collect();

        let prefilter_failures: Vec<_> = results
            .iter()
            .filter(|result| !result.ok)
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
            self.db.update_node_speeds_batch(chunk).await?;
        }
        info!(
            candidates = proxy_candidates.len(),
            "starting real proxy verification"
        );
        let proxy_timeout = proxy_timeout_duration;
        // 「常驻内核」= 一整轮只冷启动一个内核实例（大批次）；「条件隔离」见开关说明。
        let proxy_batch_size = if opts.persistent_kernel {
            crate::proxy_probe::PERSISTENT_MIHOMO_BATCH_SIZE
        } else {
            256
        };
        let conditional_isolation = opts.conditional_isolation;
        let proxy_total = proxy_candidates.len();
        let mut mihomo_nodes = Vec::new();
        let mut sing_box_nodes = Vec::new();
        let mut xray_nodes = Vec::new();
        let mut proxy_results = Vec::new();
        let mut result_kernels = HashMap::new();
        for node in proxy_candidates {
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
                    message: error.to_string(),
                }),
            }
        }
        let rejected_done = proxy_results.len();
        let rejected_ok = proxy_results.iter().filter(|result| result.ok).count();
        // 三个内核的端口与工作目录本来就互相独立：并行可把墙钟压到约 1/3，代价是
        // 峰值内存约 3 倍，并且批次级进度只剩汇总（不再逐个内核上报）。
        if opts.parallel_kernels {
            let (mihomo, sing_box, xray) = tokio::join!(
                verify_nodes_with_kernel_progress(
                    KernelKind::Mihomo,
                    mihomo_nodes,
                    proxy_timeout,
                    None,
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                ),
                verify_nodes_with_kernel_progress(
                    KernelKind::SingBox,
                    sing_box_nodes,
                    proxy_timeout,
                    None,
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                ),
                verify_nodes_with_kernel_progress(
                    KernelKind::Xray,
                    xray_nodes,
                    proxy_timeout,
                    None,
                    network_limit,
                    proxy_batch_size,
                    conditional_isolation,
                ),
            );
            proxy_results.extend(mihomo?);
            proxy_results.extend(sing_box?);
            proxy_results.extend(xray?);
        } else {
            let mihomo_results = if let Some(progress) = progress.as_ref() {
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
                        kernel: Some(KernelKind::Mihomo),
                        batch_index: proxy.batch_index,
                        batch_total: proxy.batch_total,
                    });
                };
                verify_nodes_with_kernel_progress(
                    KernelKind::Mihomo,
                    mihomo_nodes,
                    proxy_timeout,
                    Some(&proxy_progress),
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await?
            } else {
                verify_nodes_with_kernel_progress(
                    KernelKind::Mihomo,
                    mihomo_nodes,
                    proxy_timeout,
                    None,
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await?
            };
            let mihomo_done = rejected_done + mihomo_results.len();
            let mihomo_ok = rejected_ok + mihomo_results.iter().filter(|result| result.ok).count();
            proxy_results.extend(mihomo_results);
            let sing_box_results = if let Some(progress) = progress.as_ref() {
                let proxy_progress = |proxy: ProxyProbeProgress| {
                    let done = mihomo_done + proxy.done;
                    let ok = mihomo_ok + proxy.ok;
                    progress(SpeedTestProgress {
                        phase: SpeedTestPhase::RealProxy,
                        done,
                        total: proxy_total,
                        ok,
                        fail: done.saturating_sub(ok),
                        elapsed: started.elapsed(),
                        kernel: Some(KernelKind::SingBox),
                        batch_index: proxy.batch_index,
                        batch_total: proxy.batch_total,
                    });
                };
                verify_nodes_with_kernel_progress(
                    KernelKind::SingBox,
                    sing_box_nodes,
                    proxy_timeout,
                    Some(&proxy_progress),
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await?
            } else {
                verify_nodes_with_kernel_progress(
                    KernelKind::SingBox,
                    sing_box_nodes,
                    proxy_timeout,
                    None,
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await?
            };
            proxy_results.extend(sing_box_results);
            let before_xray_done = proxy_results.len();
            let before_xray_ok = proxy_results.iter().filter(|result| result.ok).count();
            let xray_results = if let Some(progress) = progress.as_ref() {
                let proxy_progress = |proxy: ProxyProbeProgress| {
                    let done = before_xray_done + proxy.done;
                    let ok = before_xray_ok + proxy.ok;
                    progress(SpeedTestProgress {
                        phase: SpeedTestPhase::RealProxy,
                        done,
                        total: proxy_total,
                        ok,
                        fail: done.saturating_sub(ok),
                        elapsed: started.elapsed(),
                        kernel: Some(KernelKind::Xray),
                        batch_index: proxy.batch_index,
                        batch_total: proxy.batch_total,
                    });
                };
                verify_nodes_with_kernel_progress(
                    KernelKind::Xray,
                    xray_nodes,
                    proxy_timeout,
                    Some(&proxy_progress),
                    network_limit.clone(),
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await?
            } else {
                verify_nodes_with_kernel_progress(
                    KernelKind::Xray,
                    xray_nodes,
                    proxy_timeout,
                    None,
                    network_limit,
                    proxy_batch_size,
                    conditional_isolation,
                )
                .await?
            };
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
        let mut proxy_results: HashMap<String, ProxyProbeResult> = proxy_results
            .into_iter()
            .map(|result| (result.id.clone(), result))
            .collect();
        for result in &mut results {
            if let Some(proxy) = proxy_results.remove(&result.id) {
                result.ok = proxy.ok;
                result.latency_ms = proxy.latency_ms;
                result.download_kbps = None;
                if let Some(kernel) = result_kernels.get(&result.id) {
                    result.latency_kernel = Some(
                        match kernel {
                            KernelKind::Mihomo => "mihomo",
                            KernelKind::SingBox => "sing_box",
                            KernelKind::Xray => "xray",
                        }
                        .into(),
                    );
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
            self.db.update_node_speeds_batch(chunk).await?;
        }

        let ok = results.iter().filter(|r| r.ok).count();
        let fail = results.len().saturating_sub(ok);
        let avg_ms = {
            let vals: Vec<i64> = results.iter().filter_map(|r| r.latency_ms).collect();
            if vals.is_empty() {
                None
            } else {
                Some(vals.iter().sum::<i64>() as f64 / vals.len() as f64)
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
            "speedtest finished"
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
}

fn build_speedtest_summary(results: Vec<LatencyResult>) -> Result<SpeedTestSummary> {
    if results.is_empty() {
        anyhow::bail!("没有可测速节点，或所有测速任务均失败");
    }
    let total = results.len();
    let ok = results.iter().filter(|result| result.ok).count();
    let fail = total - ok;
    let latency_values: Vec<_> = results
        .iter()
        .filter_map(|result| result.latency_ms)
        .collect();
    let download_values: Vec<_> = results
        .iter()
        .filter_map(|result| result.download_kbps)
        .collect();
    Ok(SpeedTestSummary {
        total,
        ok,
        fail,
        avg_ms: (!latency_values.is_empty())
            .then(|| latency_values.iter().sum::<i64>() as f64 / latency_values.len() as f64),
        avg_download_kbps: (!download_values.is_empty())
            .then(|| download_values.iter().sum::<f64>() / download_values.len() as f64),
        results,
    })
}

fn report_speedtest_progress(
    progress: &Option<impl Fn(SpeedTestProgress) + Send + Sync>,
    phase: SpeedTestPhase,
    done: usize,
    total: usize,
    ok: usize,
    elapsed: Duration,
) {
    if let Some(progress) = progress {
        progress(SpeedTestProgress {
            phase,
            done,
            total,
            ok,
            fail: done.saturating_sub(ok),
            elapsed,
            kernel: None,
            batch_index: None,
            batch_total: None,
        });
    }
}

fn requires_proxy_without_tcp(protocol: &str) -> bool {
    matches!(
        protocol.to_ascii_lowercase().as_str(),
        "hysteria" | "hysteria2" | "hy2" | "tuic" | "wireguard" | "wg"
    )
}
/// Resolve + connect with a single overall timeout budget (faster bulk scans).
async fn connect_fast(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<(TcpStream, i64), String> {
    let start = Instant::now();
    let target = format!("{host}:{port}");

    // Cap DNS so slow resolvers don't stall workers for the full timeout.
    let dns_budget = timeout.min(Duration::from_millis(800));
    let addrs =
        match tokio::time::timeout(dns_budget, tokio::net::lookup_host(target.as_str())).await {
            Ok(Ok(iter)) => {
                let mut v: Vec<_> = iter.collect();
                // Prefer IPv4 first for free-node noise (many broken AAAA).
                v.sort_by_key(|a| if a.is_ipv4() { 0u8 } else { 1u8 });
                v
            }
            Ok(Err(e)) => return Err(format!("dns error: {e}")),
            Err(_) => return Err("dns timeout".into()),
        };
    if addrs.is_empty() {
        return Err("no address".into());
    }

    // Try at most 2 addresses within remaining budget.
    let mut last_err = String::from("connect failed");
    for addr in addrs.into_iter().take(2) {
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let latency_ms = start.elapsed().as_millis() as i64;
                return Ok((stream, latency_ms.max(1)));
            }
            Ok(Err(e)) => last_err = format!("connect error: {e}"),
            Err(_) => last_err = format!("timeout {}ms", timeout.as_millis()),
        }
    }
    Err(last_err)
}

async fn connect_resolved(
    addresses: Vec<SocketAddr>,
    timeout: Duration,
) -> Result<(TcpStream, i64), String> {
    if addresses.is_empty() {
        return Err("dns returned no addresses".into());
    }
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut tasks = JoinSet::new();
    for address in addresses {
        tasks.spawn(TcpStream::connect(address));
    }
    let mut last_error = "connect failed".to_string();
    loop {
        match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(Some(Ok(Ok(stream)))) => {
                tasks.abort_all();
                return Ok((stream, started.elapsed().as_millis().max(1) as i64));
            }
            Ok(Some(Ok(Err(error)))) => last_error = error.to_string(),
            Ok(Some(Err(error))) => last_error = error.to_string(),
            Ok(None) => return Err(last_error),
            Err(_) => {
                tasks.abort_all();
                return Err(format!("timeout {}ms", timeout.as_millis()));
            }
        }
    }
}

async fn measure_tcp_download(
    mut stream: TcpStream,
    host: &str,
    download_ms: u64,
) -> (Option<f64>, String) {
    // Best-effort only. Real encrypted proxies usually won't speak HTTP.
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: vpn-parser-speedtest/0.1\r\nAccept: */*\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.set_nodelay(true);
    if tokio::time::timeout(Duration::from_millis(400), stream.write_all(req.as_bytes()))
        .await
        .is_err()
    {
        return (None, "write timeout".into());
    }
    let _ = stream.flush().await;

    let deadline = Instant::now() + Duration::from_millis(download_ms);
    let mut buf = vec![0u8; 16 * 1024];
    let mut total: u64 = 0;
    let mut head = Vec::with_capacity(512);
    let read_start = Instant::now();

    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                if head.len() < 512 {
                    let take = (512 - head.len()).min(n);
                    head.extend_from_slice(&buf[..take]);
                }
                total += n as u64;
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }

    let elapsed = read_start.elapsed().as_secs_f64().max(0.001);
    if total < 64 {
        return (None, format!("仅收到 {total} 字节"));
    }

    if looks_like_public_website(&head) {
        return (
            None,
            format!("疑似普通网站 HTTP 响应，不计入下载 ({total} bytes)"),
        );
    }

    let kbps = (total as f64 / 1024.0) / elapsed;
    (Some(kbps), format!("{total} bytes"))
}

fn looks_like_public_website(head: &[u8]) -> bool {
    let s = String::from_utf8_lossy(head);
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("http/1.") && !lower.starts_with("http/2") {
        return false;
    }
    let status_ok = lower.contains(" 200 ")
        || lower.contains(" 301 ")
        || lower.contains(" 302 ")
        || lower.contains(" 307 ")
        || lower.contains(" 308 ")
        || lower.contains(" 403 ")
        || lower.contains(" 404 ");
    if !status_ok {
        return false;
    }
    lower.contains("text/html")
        || lower.contains("text/plain")
        || lower.contains("application/json")
        || lower.contains("<!doctype html")
        || lower.contains("<html")
        || lower.contains("server: cloudflare")
        || lower.contains("server: nginx")
        || lower.contains("server: apache")
        || lower.contains("cf-ray:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udp_proxy_protocols_skip_tcp_prefilter() {
        for protocol in ["hysteria", "hysteria2", "hy2", "tuic", "wireguard", "wg"] {
            assert!(requires_proxy_without_tcp(protocol), "{protocol}");
        }
        for protocol in ["vless", "vmess", "trojan", "ss"] {
            assert!(!requires_proxy_without_tcp(protocol), "{protocol}");
        }
    }

    #[test]
    fn speedtest_intensity_labels_and_modes() {
        assert_eq!(SpeedtestIntensity::default(), SpeedtestIntensity::Standard);
        assert!(!SpeedtestIntensity::Fast.label().is_empty());
        assert!(!SpeedtestIntensity::Standard.label().is_empty());
        assert!(!SpeedtestIntensity::Deep.label().is_empty());
    }

    #[tokio::test]
    async fn probe_rejects_private_host() {
        let (ok, ms, kbps, msg) = SpeedTester::probe_full("127.0.0.1", 80, 500, false, 200).await;
        assert!(!ok);
        assert!(ms.is_none());
        assert!(kbps.is_none());
        assert!(msg.contains("skip"));
    }

    #[tokio::test]
    async fn persists_tcp_prefilter_failure_as_unavailable() {
        let path = std::env::temp_dir().join(format!(
            "vpn-parser-prefilter-failure-{}.db",
            uuid::Uuid::new_v4()
        ));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = chrono::Utc::now();
        let node = Node {
            id: "prefilter-failure".into(),
            source_id: None,
            protocol: "ss".into(),
            name: "prefilter failure".into(),
            server: "127.0.0.1".into(),
            port: 80,
            raw: "ss://prefilter-failure".into(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: now,
            updated_at: now,
        };
        db.import_nodes(&[node]).await.unwrap();

        let tester = SpeedTester::new(db.clone(), TaskTracker::default());
        let summary = tester
            .test_many(
                Some(vec!["prefilter-failure".into()]),
                SpeedTestOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!((summary.ok, summary.fail), (0, 1));

        let stored = db.get_node("prefilter-failure").await.unwrap().unwrap();
        assert_eq!(stored.latency_ok, Some(false));
        assert_eq!(stored.latency_test_kind.as_deref(), Some("tcp_prefilter"));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}

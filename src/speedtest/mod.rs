pub mod dns;
pub mod download;
pub mod pipeline;
pub mod prefilter;

use crate::db::Database;
use crate::models::{LatencyResult, Node, SpeedTestSummary};
use crate::parser::node_fingerprint;
use crate::tasks::{TaskKind, TaskPhase, TaskTracker};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
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
    /// 以下选项对应「系统设置 → 测速优化」的开关
    pub parallel_kernels: bool,
    pub persistent_kernel: bool,
    pub reuse_verified_minutes: u32,
    pub conditional_isolation: bool,
    pub download_top_n: u32,
    pub dedup_egress: bool,
    /// 快速 TCP Ping 模式：跳过内核代理真实握手，仅测试网络连通性
    pub tcp_ping_only: bool,
}

pub const TCP_PREFILTER_TIMEOUT_MS: u32 = 800;

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
    pub kernel: Option<crate::kernel::KernelKind>,
    pub batch_index: Option<usize>,
    pub batch_total: Option<usize>,
}

impl Default for SpeedTestOptions {
    fn default() -> Self {
        Self {
            concurrency: 128,
            timeout_ms: 1500,
            measure_download: false,
            download_test_ms: 800,
            intensity: SpeedtestIntensity::Standard,
            parallel_kernels: false,
            persistent_kernel: false,
            reuse_verified_minutes: 0,
            conditional_isolation: false,
            download_top_n: 0,
            dedup_egress: true,
            tcp_ping_only: false,
        }
    }
}

impl SpeedTester {
    pub fn new(db: Database, tasks: TaskTracker) -> Self {
        Self { db, tasks }
    }

    pub async fn test_one(&self, node_id: &str, opts: SpeedTestOptions) -> Result<LatencyResult> {
        let mut summary = self.test_many(Some(vec![node_id.to_owned()]), opts).await?;
        summary
            .results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("node not found"))
    }

    pub async fn test_many(
        &self,
        node_ids: Option<Vec<String>>,
        opts: SpeedTestOptions,
    ) -> Result<SpeedTestSummary> {
        self.test_many_with_progress(node_ids, opts, None::<fn(SpeedTestProgress)>)
            .await
    }

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
                format!("测速完成：成功 {ok}，失败 {fail}"),
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
        pipeline::run_speedtest_pipeline(&self.db, nodes, opts, progress, network_limit).await
    }
}

pub fn build_speedtest_summary(results: Vec<LatencyResult>) -> Result<SpeedTestSummary> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostutil::is_public_proxy_host;

    #[test]
    fn udp_proxy_protocols_skip_tcp_prefilter() {
        for protocol in ["hysteria", "hysteria2", "hy2", "tuic", "wireguard", "wg", "masque"] {
            assert!(pipeline::requires_proxy_without_tcp(protocol), "{protocol}");
        }
        for protocol in ["vless", "vmess", "trojan", "ss"] {
            assert!(
                !pipeline::requires_proxy_without_tcp(protocol),
                "{protocol}"
            );
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
        assert!(!is_public_proxy_host("127.0.0.1"));
        assert!(!is_public_proxy_host("192.168.1.1"));
        assert!(is_public_proxy_host("1.1.1.1"));
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

    #[tokio::test]
    async fn tcp_ping_only_mode_records_tcp_ping_kind() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-tcp-ping-{}.db", uuid::Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = chrono::Utc::now();
        let node = Node {
            id: "tcp-ping-node".into(),
            source_id: None,
            protocol: "ss".into(),
            name: "tcp ping node".into(),
            server: "127.0.0.1".into(),
            port: 80,
            raw: "ss://tcp-ping-node".into(),
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
        let mut opts = SpeedTestOptions::default();
        opts.tcp_ping_only = true;
        let summary = tester
            .test_many(Some(vec!["tcp-ping-node".into()]), opts)
            .await
            .unwrap();
        assert_eq!(summary.total, 1);

        let stored = db.get_node("tcp-ping-node").await.unwrap().unwrap();
        assert_eq!(stored.latency_test_kind.as_deref(), Some("tcp_ping"));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}

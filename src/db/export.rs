use super::Database;
use crate::models::Node;
use anyhow::Result;

impl Database {
    pub async fn export_nodes(
        &self,
        protocol: Option<&str>,
        source_id: Option<&str>,
    ) -> Result<Vec<Node>> {
        self.export_nodes_filtered(protocol, source_id, false, None, None, None)
            .await
    }

    /// Export nodes with optional quality filter.
    ///
    /// quality_only means:
    /// - TCP connect previously succeeded (latency_ok=1)
    /// - latency in (0, max_latency_ms]
    /// - host is public and not a known website/CDN decoy
    ///
    /// min_download_kbps is OPTIONAL. Default 0 = do not require download.
    /// Requiring download is usually wrong for encrypted proxies (ss/vmess/vless/trojan),
    /// because our probe cannot speak those protocols and open websites look "fast".
    pub async fn export_nodes_filtered(
        &self,
        protocol: Option<&str>,
        source_id: Option<&str>,
        quality_only: bool,
        max_latency_ms: Option<i64>,
        min_download_kbps: Option<f64>,
        limit: Option<u32>,
    ) -> Result<Vec<Node>> {
        let mut nodes = Self::filter_nodes(
            self.load_nodes(source_id).await?,
            protocol,
            None,
            None,
            None,
        );
        if quality_only {
            let max_latency = max_latency_ms.unwrap_or(2000);
            let min_download = min_download_kbps.unwrap_or(0.0);
            nodes.retain(|node| {
                node.latency_ok == Some(true)
                    && node.latency_test_kind.as_deref() == Some("google_proxy")
                    && node
                        .latency_ms
                        .is_some_and(|value| value > 1 && value <= max_latency)
                    && (min_download <= 0.0
                        || node
                            .download_kbps
                            .is_some_and(|value| value >= min_download))
                    && crate::hostutil::is_quality_export_host(&node.server)
            });
        }
        nodes.sort_by_key(|node| (node.latency_ms.unwrap_or(i64::MAX), node.name.clone()));
        if let Some(limit) = limit {
            nodes.truncate(limit.max(1) as usize);
        }
        Ok(nodes)
    }

    /// “推送可用节点”用的导出：同 `export_nodes_filtered(quality_only=true)`，
    /// 但排除标记为不推送的源、以及命中排除关键词的节点。
    /// 返回 (保留节点, 被排除数量)。
    pub async fn export_pushable_nodes(
        &self,
        max_latency_ms: Option<i64>,
        exclude_keywords: &str,
    ) -> Result<(Vec<Node>, usize)> {
        let excluded: std::collections::HashSet<String> = self
            .list_sources()
            .await?
            .into_iter()
            .filter(|source| source.push_disabled)
            .map(|source| source.id)
            .collect();
        let nodes = self
            .export_nodes_filtered(None, None, true, max_latency_ms, None, None)
            .await?;
        if excluded.is_empty() && exclude_keywords.trim().is_empty() {
            return Ok((nodes, 0));
        }
        let mut kept = Vec::with_capacity(nodes.len());
        let mut skipped = 0usize;
        for node in nodes {
            let from_disabled_source = node
                .source_id
                .as_deref()
                .is_some_and(|id| excluded.contains(id));
            if from_disabled_source || crate::models::name_excluded(&node.name, exclude_keywords) {
                skipped += 1;
            } else {
                kept.push(node);
            }
        }
        Ok((kept, skipped))
    }
}

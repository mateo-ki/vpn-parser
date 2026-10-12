use super::Database;
use super::UNGROUPED_SOURCE_ID;
use crate::models::Node;
use anyhow::Result;
use chrono::Utc;
use sqlx::Row;

impl Database {
    pub async fn update_node_latency(
        &self,
        id: &str,
        ok: bool,
        latency_ms: Option<i64>,
    ) -> Result<()> {
        self.update_node_speed(id, ok, latency_ms, None, None, None)
            .await
    }

    pub async fn update_node_speed(
        &self,
        id: &str,
        ok: bool,
        latency_ms: Option<i64>,
        download_kbps: Option<f64>,
        latency_kernel: Option<&str>,
        latency_test_kind: Option<&str>,
    ) -> Result<()> {
        let Some(pool) = self.find_node_pool(id).await? else {
            return Ok(());
        };
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE nodes SET latency_ok=?, latency_ms=?, download_kbps=?, latency_kernel=?,
                      latency_test_kind=?, latency_tested_at=?, updated_at=?
               WHERE id=?"#,
        )
        .bind(ok as i64)
        .bind(latency_ms)
        .bind(download_kbps)
        .bind(latency_kernel)
        .bind(latency_test_kind)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(&pool)
        .await?;
        // 测速改的正是索引里的两列（latency_ms / latency_ok）。
        self.invalidate_node_index().await;
        Ok(())
    }

    /// Batch write speed results (transaction). Much faster than per-node UPDATE.
    pub async fn update_node_speeds_batch(
        &self,
        items: &[(
            String,
            bool,
            Option<i64>,
            Option<f64>,
            Option<String>,
            Option<String>,
        )],
    ) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let mut lookup: std::collections::HashMap<&str, &_> = std::collections::HashMap::new();
        for item in items {
            lookup.insert(item.0.as_str(), item);
        }
        let mut grouped: std::collections::HashMap<
            Option<String>,
            Vec<&(
                String,
                bool,
                Option<i64>,
                Option<f64>,
                Option<String>,
                Option<String>,
            )>,
        > = std::collections::HashMap::new();
        for scope in self.node_scopes(None).await? {
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let pool = self.open_node_pool(scope.as_deref()).await?;
            // 只按候选 id 走主键点查：以前每个分库都 `SELECT id FROM nodes` 全量导出，
            // 19000 条结果里绝大多数会被丢掉。
            let mut group: Vec<&_> = Vec::new();
            for chunk in items.chunks(500) {
                let placeholders = vec!["?"; chunk.len()].join(",");
                let sql = format!("SELECT id FROM nodes WHERE id IN ({placeholders})");
                let mut query = sqlx::query(&sql);
                for item in chunk {
                    query = query.bind(&item.0);
                }
                for row in query.fetch_all(&pool).await? {
                    let id: String = row.get("id");
                    if let Some(item) = lookup.get(id.as_str()) {
                        group.push(*item);
                    }
                }
            }
            if !group.is_empty() {
                grouped.insert(scope, group);
            }
        }
        let now = Utc::now().to_rfc3339();
        for (scope, group) in grouped {
            let pool = self.open_node_pool(scope.as_deref()).await?;
            let mut tx = pool.begin().await?;
            for (id, ok, latency_ms, download_kbps, kernel, kind) in group {
                sqlx::query(
                    r#"UPDATE nodes SET latency_ok=?, latency_ms=?, download_kbps=?, latency_kernel=?,
                              latency_test_kind=?, latency_tested_at=?, updated_at=?
                       WHERE id=?"#,
                )
                .bind(*ok as i64)
                .bind(*latency_ms)
                .bind(*download_kbps)
                .bind(kernel)
                .bind(kind)
                .bind(&now)
                .bind(&now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
            tx.commit().await?;
        }
        self.invalidate_node_index().await;
        Ok(())
    }

    /// Full node list for bulk TCP prefilter + real proxy verification.
    pub async fn list_nodes_for_speedtest(&self) -> Result<Vec<Node>> {
        self.list_nodes_for_speedtest_filtered(None, None, None)
            .await
    }

    /// Same as [`list_nodes_for_speedtest`] but restricted to the current list filter,
    /// so "测速全部" only covers what the user has filtered to.
    pub async fn list_nodes_for_speedtest_filtered(
        &self,
        protocol: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
    ) -> Result<Vec<Node>> {
        Ok(Self::filter_nodes(
            self.load_nodes(None).await?,
            protocol,
            keyword,
            latency,
            None,
        ))
    }

    pub async fn list_node_ids_for_group(
        &self,
        source_id: Option<&str>,
        protocol: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
    ) -> Result<Vec<String>> {
        let scope = source_id.unwrap_or(UNGROUPED_SOURCE_ID);
        Ok(Self::filter_nodes(
            self.load_nodes(Some(scope)).await?,
            protocol,
            keyword,
            latency,
            None,
        )
        .into_iter()
        .map(|node| node.id)
        .collect())
    }
}

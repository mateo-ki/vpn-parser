use super::Database;
use super::UNGROUPED_SOURCE_ID;
use crate::models::{Node, NodeSummary, Page};
use crate::parser::node_fingerprint;
use anyhow::{anyhow, Result};
use chrono::Utc;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row,
};
use std::str::FromStr;
use uuid::Uuid;

pub(crate) struct PreservedLatency {
    pub(crate) latency_ms: Option<i64>,
    pub(crate) latency_ok: Option<i64>,
    pub(crate) download_kbps: Option<f64>,
    pub(crate) latency_kernel: Option<String>,
    pub(crate) latency_test_kind: Option<String>,
    pub(crate) latency_tested_at: Option<String>,
}

impl Database {
    pub(crate) async fn insert_node(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        n: &Node,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO nodes
               (id, source_id, protocol, name, server, port, raw, config, country,
                latency_ms, latency_ok, download_kbps, latency_kernel,
                latency_test_kind, latency_tested_at, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, '', '{}', ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT DO UPDATE SET
                 source_id=excluded.source_id, protocol=excluded.protocol, name=excluded.name,
                 server=excluded.server, port=excluded.port, raw='', config='{}',
                 country=excluded.country, latency_ms=excluded.latency_ms, latency_ok=excluded.latency_ok,
                 download_kbps=excluded.download_kbps, latency_kernel=excluded.latency_kernel,
                 latency_test_kind=excluded.latency_test_kind, latency_tested_at=excluded.latency_tested_at,
                 updated_at=excluded.updated_at"#,
        )
        .bind(&n.id)
        .bind(&n.source_id)
        .bind(&n.protocol)
        .bind(&n.name)
        .bind(&n.server)
        .bind(n.port as i64)
        .bind(&n.country)
        .bind(n.latency_ms)
        .bind(n.latency_ok.map(|v| v as i64))
        .bind(n.download_kbps)
        .bind(&n.latency_kernel)
        .bind(&n.latency_test_kind)
        .bind(n.latency_tested_at.map(|v| v.to_rfc3339()))
        .bind(n.created_at.to_rfc3339())
        .bind(n.updated_at.to_rfc3339())
        .execute(&mut **tx)
        .await?;

        let config_str = n.config.to_string();
        sqlx::query(
            r#"INSERT INTO node_payloads (node_id, raw, config)
               VALUES (?, ?, ?)
               ON CONFLICT(node_id) DO UPDATE SET
                 raw=excluded.raw, config=excluded.config"#,
        )
        .bind(&n.id)
        .bind(&n.raw)
        .bind(config_str)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub(crate) async fn load_nodes(&self, source_id: Option<&str>) -> Result<Vec<Node>> {
        let scopes = self.node_scopes(source_id).await?;
        if scopes.len() <= 1 {
            let scope = scopes.into_iter().next().flatten();
            return self.load_scope_nodes(scope.as_deref()).await;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for scope in scopes {
            let this = self.clone();
            tasks.spawn(async move { this.load_scope_nodes(scope.as_deref()).await });
        }
        let mut nodes = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            nodes.extend(joined.map_err(|error| anyhow!(error.to_string()))??);
        }
        Ok(nodes)
    }

    pub(crate) async fn load_scope_nodes(&self, scope: Option<&str>) -> Result<Vec<Node>> {
        let path = self.node_db_path(scope);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let pool = self.open_node_pool(scope).await?;
        let rows = sqlx::query(
            r#"SELECT n.id, n.source_id, n.protocol, n.name, n.server, n.port,
                      COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                      COALESCE(NULLIF(p.config, ''), n.config, '{}') AS config,
                      n.country, n.latency_ms, n.latency_ok, n.download_kbps,
                      n.latency_kernel, n.latency_test_kind, n.latency_tested_at,
                      n.created_at, n.updated_at
               FROM nodes n
               LEFT JOIN node_payloads p ON n.id = p.node_id"#,
        )
        .fetch_all(&pool)
        .await?;
        Ok(rows.into_iter().map(super::row_to_node).collect())
    }

    pub(crate) async fn load_node_summaries(
        &self,
        source_id: Option<&str>,
    ) -> Result<Vec<NodeSummary>> {
        let scopes = self.node_scopes(source_id).await?;
        if scopes.len() <= 1 {
            let scope = scopes.into_iter().next().flatten();
            return self.load_scope_node_summaries(scope.as_deref()).await;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for scope in scopes {
            let this = self.clone();
            tasks.spawn(async move { this.load_scope_node_summaries(scope.as_deref()).await });
        }
        let mut summaries = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            summaries.extend(joined.map_err(|error| anyhow!(error.to_string()))??);
        }
        Ok(summaries)
    }

    async fn load_scope_node_summaries(&self, scope: Option<&str>) -> Result<Vec<NodeSummary>> {
        let path = self.node_db_path(scope);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let pool = self.open_node_pool(scope).await?;
        let rows = sqlx::query(
            r#"SELECT id, source_id, protocol, name, server, port, country,
                      latency_ms, latency_ok, download_kbps, latency_kernel,
                      latency_test_kind, latency_tested_at, updated_at
               FROM nodes"#,
        )
        .fetch_all(&pool)
        .await?;
        Ok(rows.into_iter().map(super::row_to_node_summary).collect())
    }

    async fn load_available_scope_node_summaries(
        &self,
        scope: Option<&str>,
    ) -> Result<Vec<NodeSummary>> {
        let path = self.node_db_path(scope);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let pool = self.open_node_pool(scope).await?;
        let rows = sqlx::query(
            r#"SELECT id, source_id, protocol, name, server, port, country,
                      latency_ms, latency_ok, download_kbps, latency_kernel,
                      latency_test_kind, latency_tested_at, updated_at
               FROM nodes
               WHERE latency_ok = 1 AND latency_test_kind = 'google_proxy'
               ORDER BY latency_ms ASC"#,
        )
        .fetch_all(&pool)
        .await?;
        Ok(rows.into_iter().map(super::row_to_node_summary).collect())
    }

    /// 单节点库的 SQL 分页查询：过滤/排序/分页全部在库内完成，且只加载元数据。
    #[allow(clippy::too_many_arguments)]
    async fn list_nodes_page(
        &self,
        scope: &str,
        protocol: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
        country: Option<&str>,
        page: u32,
        page_size: u32,
        sort: Option<&str>,
    ) -> Result<Page<NodeSummary>> {
        let scope = if scope == UNGROUPED_SOURCE_ID {
            None
        } else {
            Some(scope)
        };
        let mut where_sql = String::from("WHERE 1=1");
        let mut binds: Vec<String> = Vec::new();
        if let Some(value) = protocol {
            where_sql.push_str(" AND protocol = ?");
            binds.push(value.to_string());
        }
        if let Some(value) = keyword {
            let pattern = format!("%{}%", value.to_ascii_lowercase());
            where_sql.push_str(" AND (lower(name) LIKE ? OR lower(server) LIKE ?)");
            binds.push(pattern.clone());
            binds.push(pattern);
        }
        match latency.map(|value| value.to_ascii_lowercase()) {
            Some(ref value) if value == "alive" => {
                where_sql.push_str(" AND latency_ok = 1 AND latency_test_kind = 'google_proxy'");
            }
            Some(ref value) if value == "dead" => where_sql.push_str(" AND latency_ok = 0"),
            Some(ref value) if value == "untested" => where_sql.push_str(" AND latency_ok IS NULL"),
            _ => {}
        }
        if let Some(value) = country {
            if !value.eq_ignore_ascii_case("all") {
                if value.eq_ignore_ascii_case("OTHER") {
                    where_sql.push_str(
                        " AND (country IS NULL OR country NOT IN \
                         ('HK','JP','US','SG','TW','KR','GB','DE'))",
                    );
                } else {
                    where_sql.push_str(" AND lower(country) = lower(?)");
                    binds.push(value.to_string());
                }
            }
        }
        let order_by = match sort.unwrap_or("updated").to_ascii_lowercase().as_str() {
            "latency" => {
                "ORDER BY (latency_ok = 1) DESC, (latency_ms IS NULL) ASC, latency_ms ASC, name ASC"
            }
            "download" => "ORDER BY (download_kbps IS NULL) ASC, download_kbps DESC",
            "name" => "ORDER BY name ASC",
            _ => "ORDER BY updated_at DESC",
        };
        if !self.node_db_path(scope).exists() {
            // 不存在的分库直接返回空页：走 open_node_pool 会因 mode=rwc 建库。
            return Ok(Page {
                total: 0,
                page,
                page_size,
                items: Vec::new(),
            });
        }
        let pool = self.open_node_pool(scope).await?;
        let count_sql = format!("SELECT COUNT(*) AS total FROM nodes {where_sql}");
        let mut count_query = sqlx::query(&count_sql);
        for bind in &binds {
            count_query = count_query.bind(bind);
        }
        let total: i64 = count_query.fetch_one(&pool).await?.get("total");
        let rows_sql = format!(
            r#"SELECT id, source_id, protocol, name, server, port, country,
                      latency_ms, latency_ok, download_kbps, latency_kernel,
                      latency_test_kind, latency_tested_at, updated_at
               FROM nodes {where_sql} {order_by} LIMIT ? OFFSET ?"#
        );
        let mut rows_query = sqlx::query(&rows_sql);
        for bind in &binds {
            rows_query = rows_query.bind(bind);
        }
        let rows = rows_query
            .bind(page_size as i64)
            .bind(((page - 1) * page_size) as i64)
            .fetch_all(&pool)
            .await?;
        let items = rows.into_iter().map(super::row_to_node_summary).collect();
        Ok(Page {
            total,
            page,
            page_size,
            items,
        })
    }

    pub(crate) fn filter_nodes(
        mut nodes: Vec<Node>,
        protocol: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
        country: Option<&str>,
    ) -> Vec<Node> {
        // 关键字和状态各只小写一次：写在闭包里等于每个节点重新分配一遍（全库两万行）。
        let needle = keyword.map(|value| value.to_ascii_lowercase());
        let latency = latency.map(|value| value.to_ascii_lowercase());
        nodes.retain(|node| {
            protocol.is_none_or(|value| node.protocol == value)
                && needle.as_deref().is_none_or(|value| {
                    node.name.to_ascii_lowercase().contains(value)
                        || node.server.to_ascii_lowercase().contains(value)
                })
                && match latency.as_deref() {
                    Some("alive") => {
                        node.latency_ok == Some(true)
                            && node.latency_test_kind.as_deref() == Some("google_proxy")
                    }
                    Some("dead") => node.latency_ok == Some(false),
                    Some("untested") => node.latency_ok.is_none(),
                    _ => true,
                }
                && country.is_none_or(|value| {
                    if value.eq_ignore_ascii_case("all") {
                        true
                    } else if value.eq_ignore_ascii_case("OTHER") {
                        node.country.as_deref().map_or(true, |c| {
                            !["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"].contains(&c)
                        })
                    } else {
                        node.country
                            .as_deref()
                            .is_some_and(|c| c.eq_ignore_ascii_case(value))
                    }
                })
        });
        nodes
    }

    pub(crate) fn filter_node_summaries(
        mut nodes: Vec<NodeSummary>,
        protocol: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
        country: Option<&str>,
    ) -> Vec<NodeSummary> {
        let needle = keyword.map(|value| value.to_ascii_lowercase());
        let latency = latency.map(|value| value.to_ascii_lowercase());
        nodes.retain(|node| {
            protocol.is_none_or(|value| node.protocol == value)
                && needle.as_deref().is_none_or(|value| {
                    node.name.to_ascii_lowercase().contains(value)
                        || node.server.to_ascii_lowercase().contains(value)
                })
                && match latency.as_deref() {
                    Some("alive") => {
                        node.latency_ok == Some(true)
                            && node.latency_test_kind.as_deref() == Some("google_proxy")
                    }
                    Some("dead") => node.latency_ok == Some(false),
                    Some("untested") => node.latency_ok.is_none(),
                    _ => true,
                }
                && country.is_none_or(|value| {
                    if value.eq_ignore_ascii_case("all") {
                        true
                    } else if value.eq_ignore_ascii_case("OTHER") {
                        node.country.as_deref().map_or(true, |c| {
                            !["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"].contains(&c)
                        })
                    } else {
                        node.country
                            .as_deref()
                            .is_some_and(|c| c.eq_ignore_ascii_case(value))
                    }
                })
        });
        nodes
    }

    pub async fn list_nodes(
        &self,
        protocol: Option<&str>,
        source_id: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
        country: Option<&str>,
        page: u32,
        page_size: u32,
        sort: Option<&str>,
    ) -> Result<Page<NodeSummary>> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, 500);
        if let Some(scope) = source_id {
            return self
                .list_nodes_page(
                    scope, protocol, keyword, latency, country, page, page_size, sort,
                )
                .await;
        }
        let mut nodes = Self::filter_node_summaries(
            self.load_node_summaries(source_id).await?,
            protocol,
            keyword,
            latency,
            country,
        );
        match sort.unwrap_or("updated").to_ascii_lowercase().as_str() {
            "latency" => nodes.sort_by_key(|node| {
                (
                    node.latency_ok != Some(true),
                    node.latency_ms.unwrap_or(i64::MAX),
                    node.name.clone(),
                )
            }),
            "download" => nodes.sort_by(|a, b| {
                b.download_kbps
                    .partial_cmp(&a.download_kbps)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            "name" => nodes.sort_by(|a, b| a.name.cmp(&b.name)),
            _ => nodes.sort_by_key(|node| std::cmp::Reverse(node.updated_at)),
        }
        let total = nodes.len() as i64;
        let items = nodes
            .into_iter()
            .skip(((page - 1) * page_size) as usize)
            .take(page_size as usize)
            .collect();
        Ok(Page {
            total,
            page,
            page_size,
            items,
        })
    }

    pub async fn get_node(&self, id: &str) -> Result<Option<Node>> {
        if let Some(pool) = self.find_node_pool(id).await? {
            let row = sqlx::query(
                r#"SELECT n.id, n.source_id, n.protocol, n.name, n.server, n.port,
                          COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                          COALESCE(NULLIF(p.config, ''), n.config, '{}') AS config,
                          n.country, n.latency_ms, n.latency_ok, n.download_kbps,
                          n.latency_kernel, n.latency_test_kind, n.latency_tested_at,
                          n.created_at, n.updated_at
                   FROM nodes n
                   LEFT JOIN node_payloads p ON n.id = p.node_id
                   WHERE n.id = ?"#,
            )
            .bind(id)
            .fetch_optional(&pool)
            .await?;
            return Ok(row.map(super::row_to_node));
        }
        Ok(None)
    }

    /// 批量按 id 取节点。调用方以前写 `for id in ids { get_node(id) }`，而 `get_node`
    /// 每次都要把 35 个分库扇出找归属 —— 5000 个 id 就是 17.5 万条查询。这里改成
    /// "每个分库一次 `id IN (...)`"，分库之间并发。
    pub async fn get_nodes(&self, ids: &[String]) -> Result<Vec<Node>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = std::sync::Arc::new(ids.to_vec());
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let db = self.clone();
            let ids = ids.clone();
            tasks.spawn(async move {
                let Ok(pool) = db.open_node_pool(scope.as_deref()).await else {
                    return Vec::new();
                };
                let mut found = Vec::new();
                for chunk in ids.chunks(500) {
                    let placeholders = vec!["?"; chunk.len()].join(",");
                    let sql = format!(
                        r#"SELECT n.id, n.source_id, n.protocol, n.name, n.server, n.port,
                                  COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                                  COALESCE(NULLIF(p.config, ''), n.config, '{{}}') AS config,
                                  n.country, n.latency_ms, n.latency_ok, n.download_kbps,
                                  n.latency_kernel, n.latency_test_kind, n.latency_tested_at,
                                  n.created_at, n.updated_at
                           FROM nodes n
                           LEFT JOIN node_payloads p ON n.id = p.node_id
                           WHERE n.id IN ({placeholders})"#,
                    );
                    let mut query = sqlx::query(&sql);
                    for id in chunk {
                        query = query.bind(id);
                    }
                    match query.fetch_all(&pool).await {
                        Ok(rows) => found.extend(rows.into_iter().map(super::row_to_node)),
                        Err(error) => {
                            tracing::warn!(%error, "批量读取节点失败");
                        }
                    }
                }
                found
            });
        }
        let mut all = Vec::with_capacity(ids.len());
        while let Some(joined) = tasks.join_next().await {
            if let Ok(rows) = joined {
                all.extend(rows);
            }
        }
        Ok(all)
    }

    /// 按 (id, 归属来源) 取完整节点：只开命中的那几个分片库，不扇出。
    /// 给汇总表选出来的候选用 —— 候选只有几十条，扇出 37 个库纯属浪费。
    pub(crate) async fn get_nodes_in_scopes(
        &self,
        picks: &[(String, Option<String>)],
    ) -> Result<Vec<Node>> {
        let mut by_scope: std::collections::HashMap<Option<String>, Vec<String>> =
            Default::default();
        for (id, scope) in picks {
            by_scope.entry(scope.clone()).or_default().push(id.clone());
        }
        let mut found = Vec::with_capacity(picks.len());
        for (scope, ids) in by_scope {
            // open_node_pool 是 mode=rwc：库文件没了会顺手建一个空的。索引里残留的
            // 已删源就让它留在 missing 那一侧，由调用方回落扇出，别在这里造文件。
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let pool = self.open_node_pool(scope.as_deref()).await?;
            for chunk in ids.chunks(500) {
                let placeholders = vec!["?"; chunk.len()].join(",");
                let sql = format!(
                    r#"SELECT n.id, n.source_id, n.protocol, n.name, n.server, n.port,
                              COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                              COALESCE(NULLIF(p.config, ''), n.config, '{{}}') AS config,
                              n.country, n.latency_ms, n.latency_ok, n.download_kbps,
                              n.latency_kernel, n.latency_test_kind, n.latency_tested_at,
                              n.created_at, n.updated_at
                       FROM nodes n
                       LEFT JOIN node_payloads p ON n.id = p.node_id
                       WHERE n.id IN ({placeholders})"#,
                );
                let mut query = sqlx::query(&sql);
                for id in chunk {
                    query = query.bind(id);
                }
                match query.fetch_all(&pool).await {
                    Ok(rows) => found.extend(rows.into_iter().map(super::row_to_node)),
                    Err(error) => {
                        tracing::warn!(%error, "按汇总表归属分片批量读取节点失败");
                    }
                }
            }
        }
        Ok(found)
    }

    /// 全局按真实测速延迟取前 N 名（验证通过的），供内核内 url-test 组当候选。
    /// 刻意不走 `load_nodes`：那会把上万个节点的 raw/config 全读出来并逐条解析 JSON。
    ///
    /// 走法：主库汇总表选出 (id, 来源) -> 只回命中的分片取凭据。
    /// 汇总表过期/重建失败/一条都没选中时，回落到原来的全分片扇出 —— 这张表是优化，
    /// 不允许它变成"查不到就说没有"。
    pub async fn top_verified_nodes(
        &self,
        limit: usize,
        exclude_id: Option<&str>,
    ) -> Result<Vec<Node>> {
        let limit = limit.max(2);
        if self.ensure_node_index().await {
            let picks = self.node_index_top_verified(limit, exclude_id).await?;
            if !picks.is_empty() {
                let mut nodes = self.get_nodes_in_scopes(&picks).await?;
                // 索引说有、分片却没有（例如标过期后又恰好被并发重建）：按老路补齐。
                let present = nodes.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
                let missing: Vec<String> = picks
                    .iter()
                    .map(|p| p.0.clone())
                    .filter(|id| !present.contains(id))
                    .collect();
                if !missing.is_empty() {
                    nodes.extend(self.get_nodes(&missing).await?);
                }
                if !nodes.is_empty() {
                    nodes.sort_by_key(|node| node.latency_ms.unwrap_or(i64::MAX));
                    nodes.truncate(limit);
                    return Ok(nodes);
                }
                tracing::warn!("node_index 选中了候选但分片取不到节点，回落全分片扫描");
            }
        }
        self.top_verified_nodes_scan(limit, exclude_id).await
    }

    /// 回落路径：各分库并发按延迟取前 N，合并成全局前 N。
    /// `pub(crate)` 只为 A/B 测试留着（`node_index::ab_node_index_against_full_fanout`），
    /// 产品代码一律走 `top_verified_nodes`。
    pub(crate) async fn top_verified_nodes_scan(
        &self,
        limit: usize,
        exclude_id: Option<&str>,
    ) -> Result<Vec<Node>> {
        let limit = limit.max(2);
        // 35 个分库并发查；串行 await 时这一步在每次启动/切换上都白等几十次往返。
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let db = self.clone();
            let exclude_id = exclude_id.map(str::to_owned);
            tasks.spawn(async move {
                let Ok(pool) = db.open_node_pool(scope.as_deref()).await else {
                    return Vec::new();
                };
                let condition = match exclude_id.as_deref() {
                    Some(_) => " AND n.id != ?",
                    None => "",
                };
                let sql = format!(
                    r#"SELECT n.id, n.source_id, n.protocol, n.name, n.server, n.port,
                              COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                              COALESCE(NULLIF(p.config, ''), n.config, '{{}}') AS config,
                              n.country, n.latency_ms, n.latency_ok, n.download_kbps,
                              n.latency_kernel, n.latency_test_kind, n.latency_tested_at,
                              n.created_at, n.updated_at
                       FROM nodes n
                       LEFT JOIN node_payloads p ON n.id = p.node_id
                       WHERE n.latency_ok = 1{condition}
                       ORDER BY n.latency_ms ASC
                       LIMIT ?"#,
                );
                let mut query = sqlx::query(&sql);
                if let Some(id) = exclude_id.as_deref() {
                    query = query.bind(id);
                }
                match query.bind(limit as i64).fetch_all(&pool).await {
                    Ok(rows) => rows.into_iter().map(super::row_to_node).collect(),
                    Err(error) => {
                        tracing::warn!(%error, "读取该分库的 url-test 候选失败");
                        Vec::new()
                    }
                }
            });
        }
        let mut nodes = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Ok(rows) = joined {
                nodes.extend(rows);
            }
        }
        nodes.sort_by_key(|node| node.latency_ms.unwrap_or(i64::MAX));
        nodes.truncate(limit);
        Ok(nodes)
    }

    pub async fn list_available_nodes_for_source(
        &self,
        source_id: &str,
    ) -> Result<Vec<NodeSummary>> {
        let source_filter = if source_id == "all" {
            None
        } else {
            Some(source_id)
        };
        let scopes = self.node_scopes(source_filter).await?;
        if scopes.len() <= 1 {
            let scope = scopes.into_iter().next().flatten();
            return self
                .load_available_scope_node_summaries(scope.as_deref())
                .await;
        }
        let mut tasks = tokio::task::JoinSet::new();
        for scope in scopes {
            let this = self.clone();
            tasks.spawn(async move {
                this.load_available_scope_node_summaries(scope.as_deref())
                    .await
            });
        }
        let mut summaries = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            summaries.extend(joined.map_err(|error| anyhow!(error.to_string()))??);
        }
        summaries.sort_by_key(|node| (node.latency_ms.unwrap_or(i64::MAX), node.name.clone()));
        Ok(summaries)
    }

    pub async fn delete_node(&self, id: &str) -> Result<bool> {
        if let Some(pool) = self.find_node_pool(id).await? {
            let mut tx = pool.begin().await?;
            let affected = sqlx::query("DELETE FROM nodes WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            let _ = sqlx::query("DELETE FROM node_payloads WHERE node_id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await;
            tx.commit().await?;
            if affected > 0 {
                self.invalidate_node_index().await;
                return Ok(true);
            }
            return Ok(false);
        }
        Ok(false)
    }

    pub async fn clear_nodes(&self) -> Result<u64> {
        let mut deleted = 0;
        for (_scope, pool) in self.active_node_pools(None).await? {
            let mut tx = pool.begin().await?;
            deleted += sqlx::query("DELETE FROM nodes")
                .execute(&mut *tx)
                .await?
                .rows_affected();
            let _ = sqlx::query("DELETE FROM node_payloads")
                .execute(&mut *tx)
                .await;
            tx.commit().await?;
        }
        self.invalidate_node_index().await;
        Ok(deleted)
    }

    pub async fn dedupe_nodes(&self) -> Result<u64> {
        let mut deleted = 0u64;
        for (_scope, pool) in self.active_node_pools(None).await? {
            let rows = sqlx::query(
                r#"SELECT n.id, n.source_id, n.protocol, n.name, n.server, n.port,
                          COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                          COALESCE(NULLIF(p.config, ''), n.config, '{}') AS config,
                          n.country, n.latency_ms, n.latency_ok, n.download_kbps,
                          n.latency_kernel, n.latency_test_kind, n.latency_tested_at,
                          n.created_at, n.updated_at
                   FROM nodes n
                   LEFT JOIN node_payloads p ON n.id = p.node_id"#,
            )
            .fetch_all(&pool)
            .await?;

            let mut best: std::collections::HashMap<String, (String, i64)> =
                std::collections::HashMap::new();
            let mut delete_ids: Vec<String> = Vec::new();

            for r in rows {
                let node = super::row_to_node(r);
                let key = node_fingerprint(&node);
                let score = super::dedupe_keep_score(&node);
                match best.get(&key) {
                    Some((keep_id, keep_score)) => {
                        if score > *keep_score {
                            delete_ids.push(keep_id.clone());
                            best.insert(key, (node.id.clone(), score));
                        } else {
                            delete_ids.push(node.id);
                        }
                    }
                    None => {
                        best.insert(key, (node.id, score));
                    }
                }
            }

            if !delete_ids.is_empty() {
                let mut tx = pool.begin().await?;
                for chunk in delete_ids.chunks(200) {
                    let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                    let q1 = format!("DELETE FROM nodes WHERE id IN ({})", placeholders);
                    let q2 = format!(
                        "DELETE FROM node_payloads WHERE node_id IN ({})",
                        placeholders
                    );
                    let mut query1 = sqlx::query(&q1);
                    let mut query2 = sqlx::query(&q2);
                    for id in chunk {
                        query1 = query1.bind(id);
                        query2 = query2.bind(id);
                    }
                    let res = query1.execute(&mut *tx).await?;
                    let _ = query2.execute(&mut *tx).await;
                    deleted += res.rows_affected();
                }
                tx.commit().await?;
            }
        }
        if deleted > 0 {
            self.invalidate_node_index().await;
        }
        Ok(deleted)
    }

    
    /// 淘汰长期失效节点（例如：连续测试失败且更新时间超过 days_old 天）
    pub async fn prune_stale_dead_nodes(&self, days_old: i64) -> Result<usize> {
        let cutoff = (Utc::now() - chrono::Duration::days(days_old)).to_rfc3339();
        let mut deleted_total = 0;
        for (_scope, pool) in self.active_node_pools(None).await? {
            let res = sqlx::query("DELETE FROM nodes WHERE latency_ok = 0 AND updated_at < ?1")
                .bind(&cutoff)
                .execute(&pool)
                .await?;
            deleted_total += res.rows_affected() as usize;
        }
        if deleted_total > 0 {
            self.invalidate_node_index().await;
        }
        Ok(deleted_total)
    }

    pub async fn delete_dead_nodes(&self) -> Result<u64> {
        let mut deleted = 0;
        for scope in self.node_scopes(None).await? {
            let path = self.node_db_path(scope.as_deref());
            if path.exists() {
                let pool = self.open_node_pool(scope.as_deref()).await?;
                let mut tx = pool.begin().await?;
                let dead_ids: Vec<String> =
                    sqlx::query_scalar("SELECT id FROM nodes WHERE latency_ok = 0")
                        .fetch_all(&mut *tx)
                        .await?;
                if !dead_ids.is_empty() {
                    let res = sqlx::query("DELETE FROM nodes WHERE latency_ok = 0")
                        .execute(&mut *tx)
                        .await?;
                    deleted += res.rows_affected();
                    for chunk in dead_ids.chunks(200) {
                        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                        let q = format!(
                            "DELETE FROM node_payloads WHERE node_id IN ({})",
                            placeholders
                        );
                        let mut query = sqlx::query(&q);
                        for id in chunk {
                            query = query.bind(id);
                        }
                        let _ = query.execute(&mut *tx).await;
                    }
                }
                tx.commit().await?;
            }
        }
        if deleted > 0 {
            self.invalidate_node_index().await;
        }
        Ok(deleted)
    }

    pub async fn deduplicate_physical(&self) -> Result<u64> {
        let all_nodes = self.load_nodes(None).await?;
        if all_nodes.is_empty() {
            return Ok(0);
        }

        let mut best: std::collections::HashMap<(String, u16), (String, i64)> =
            std::collections::HashMap::new();
        let mut delete_ids: Vec<String> = Vec::new();

        for node in all_nodes {
            let key = (node.server.trim().to_lowercase(), node.port);
            let score = super::dedupe_keep_score(&node);
            match best.get(&key) {
                Some((keep_id, keep_score)) => {
                    if score > *keep_score {
                        delete_ids.push(keep_id.clone());
                        best.insert(key, (node.id.clone(), score));
                    } else {
                        delete_ids.push(node.id);
                    }
                }
                None => {
                    best.insert(key, (node.id, score));
                }
            }
        }

        if delete_ids.is_empty() {
            return Ok(0);
        }

        let mut deleted = 0u64;
        for (_scope, pool) in self.active_node_pools(None).await? {
            for chunk in delete_ids.chunks(200) {
                let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let q1 = format!("DELETE FROM nodes WHERE id IN ({})", placeholders);
                let q2 = format!(
                    "DELETE FROM node_payloads WHERE node_id IN ({})",
                    placeholders
                );
                let mut query1 = sqlx::query(&q1);
                let mut query2 = sqlx::query(&q2);
                for id in chunk {
                    query1 = query1.bind(id);
                    query2 = query2.bind(id);
                }
                let res = query1.execute(&pool).await?;
                let _ = query2.execute(&pool).await;
                deleted += res.rows_affected();
            }
        }
        if deleted > 0 {
            self.invalidate_node_index().await;
        }
        Ok(deleted)
    }

    pub async fn clean_and_beautify_node_names(&self) -> Result<usize> {
        let mut updated = 0usize;
        for (_scope, pool) in self.active_node_pools(None).await? {
            let rows = sqlx::query("SELECT id, name, server, port, country FROM nodes")
                .fetch_all(&pool)
                .await?;
            let mut renames: Vec<(String, String)> = Vec::new();

            for row in rows {
                let id: String = row.get("id");
                let old_name: String = row.get("name");
                let server: String = row.get("server");
                let port: i64 = row.get("port");
                let country: Option<String> = row.get("country");

                let cleaned = crate::parser::clean_node_name(
                    &old_name,
                    country.as_deref(),
                    &server,
                    port as u16,
                );
                if cleaned != old_name {
                    renames.push((cleaned, id));
                }
            }

            // 一个分库一个事务：逐条 UPDATE 在 19000 个节点上就是 19000 次提交。
            let mut tx = pool.begin().await?;
            for (cleaned, id) in &renames {
                let res = sqlx::query("UPDATE nodes SET name = ? WHERE id = ?")
                    .bind(cleaned)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                if res.rows_affected() > 0 {
                    updated += 1;
                }
            }
            tx.commit().await?;
        }
        Ok(updated)
    }

    pub async fn scrub_invalid_hosts(&self) -> Result<u64> {
        // 只需要 id/server/上次测速的内核与方式：绝不能走 load_nodes，那会把全部
        // 节点的 raw+config 读出来并逐条解析 JSON，和首屏渲染抢时间。
        let mut items: Vec<(
            String,
            bool,
            Option<i64>,
            Option<f64>,
            Option<String>,
            Option<String>,
        )> = Vec::new();
        for (_scope, pool) in self.active_node_pools(None).await? {
            let rows =
                sqlx::query("SELECT id, server, latency_kernel, latency_test_kind FROM nodes")
                    .fetch_all(&pool)
                    .await?;
            for row in rows {
                let server: String = row.get("server");
                if crate::hostutil::is_public_proxy_host(&server)
                    && !crate::hostutil::is_decoy_website_host(&server)
                {
                    continue;
                }
                items.push((
                    row.get("id"),
                    false,
                    None,
                    None,
                    row.get("latency_kernel"),
                    row.get("latency_test_kind"),
                ));
            }
        }
        let changed = items.len() as u64;
        self.update_node_speeds_batch(&items).await?;
        Ok(changed)
    }

    pub async fn delete_invalid_hosts(&self) -> Result<u64> {
        // 按分库分组批量删：以前是每个节点都 find_node_pool 扫一遍全部分库，
        // 19000 个节点 × 34 个库 ≈ 65 万次查询。
        let mut by_scope: std::collections::HashMap<Option<String>, Vec<String>> =
            std::collections::HashMap::new();
        for (scope, pool) in self.active_node_pools(None).await? {
            let rows = sqlx::query("SELECT id, server FROM nodes")
                .fetch_all(&pool)
                .await?;
            for row in rows {
                let server: String = row.get("server");
                if !crate::hostutil::is_public_proxy_host(&server) {
                    by_scope
                        .entry(scope.clone())
                        .or_default()
                        .push(row.get("id"));
                }
            }
        }
        let mut deleted = 0;
        for (scope, ids) in by_scope {
            let pool = self.open_node_pool(scope.as_deref()).await?;
            for chunk in ids.chunks(500) {
                let placeholders = vec!["?"; chunk.len()].join(",");
                let mut tx = pool.begin().await?;
                let nodes_sql = format!("DELETE FROM nodes WHERE id IN ({placeholders})");
                let mut query = sqlx::query(&nodes_sql);
                for id in chunk {
                    query = query.bind(id);
                }
                deleted += query.execute(&mut *tx).await?.rows_affected();
                let payloads_sql =
                    format!("DELETE FROM node_payloads WHERE node_id IN ({placeholders})");
                let mut payloads = sqlx::query(&payloads_sql);
                for id in chunk {
                    payloads = payloads.bind(id);
                }
                let _ = payloads.execute(&mut *tx).await;
                tx.commit().await?;
            }
        }
        Ok(deleted)
    }

    /// 用新一批节点整体替换该源的分片：这一次没出现的就删掉（普通订阅链接的语义）。
    pub async fn replace_source_nodes(&self, source_id: &str, nodes: &[Node]) -> Result<usize> {
        self.write_source_nodes(source_id, nodes, false).await
    }

    /// Telegram 频道这类"零散发布"的源：新出现的并进来了，这一页看不到的旧节点**不删**。
    /// 淘汰仍旧交给连续测不通（fail_streak）那条线，不靠拉取覆盖。
    pub async fn accumulate_source_nodes(&self, source_id: &str, nodes: &[Node]) -> Result<usize> {
        self.write_source_nodes(source_id, nodes, true).await
    }

    async fn write_source_nodes(
        &self,
        source_id: &str,
        nodes: &[Node],
        keep_missing: bool,
    ) -> Result<usize> {
        let mut batch: std::collections::HashMap<String, &Node> = std::collections::HashMap::new();
        for n in nodes {
            let key = node_fingerprint(n);
            match batch.get(&key) {
                Some(old) => {
                    let old_q = super::name_quality_score(&old.name);
                    let new_q = super::name_quality_score(&n.name);
                    if new_q > old_q || (new_q == old_q && n.updated_at >= old.updated_at) {
                        batch.insert(key, n);
                    }
                }
                None => {
                    batch.insert(key, n);
                }
            }
        }

        let destination = self.node_db_path(Some(source_id));
        let temporary = self
            .node_root
            .join(format!(".{source_id}-{}.tmp", Uuid::new_v4()));
        let temp_url = format!(
            "sqlite://{}?mode=rwc",
            temporary.to_string_lossy().replace('\\', "/")
        );
        let temp_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::from_str(&temp_url)?
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal),
            )
            .await?;
        Self::migrate_node_pool(&temp_pool).await?;

        let old_pool = match self.take_node_pool(Some(source_id)).await? {
            Some(pool) => pool,
            None => Self::connect_node_pool(&destination).await?,
        };

        let existing = sqlx::query(
            r#"SELECT n.protocol, n.server, n.port, n.name,
                      COALESCE(NULLIF(p.config, ''), n.config, '{}') AS config,
                      COALESCE(NULLIF(p.raw, ''), n.raw, '') AS raw,
                      n.country, n.created_at, n.updated_at,
                      n.latency_ms, n.latency_ok, n.download_kbps,
                      n.latency_kernel, n.latency_test_kind, n.latency_tested_at
               FROM nodes n
               LEFT JOIN node_payloads p ON n.id = p.node_id
               WHERE n.source_id = ?"#,
        )
        .bind(source_id)
        .fetch_all(&old_pool)
        .await?;
        let mut preserved: std::collections::HashMap<String, PreservedLatency> =
            std::collections::HashMap::with_capacity(existing.len());
        let mut leftovers: std::collections::HashMap<String, Node> =
            std::collections::HashMap::new();
        for row in existing {
            let config_str: String = row.get("config");
            let config = serde_json::from_str(&config_str).unwrap_or(serde_json::json!({}));
            let tested_at: Option<String> = row.try_get("latency_tested_at").ok().flatten();
            let created_at: Option<String> = row.try_get("created_at").ok().flatten();
            let updated_at: Option<String> = row.try_get("updated_at").ok().flatten();
            let carried = Node {
                id: String::new(),
                source_id: Some(source_id.to_owned()),
                protocol: row.get("protocol"),
                name: row.get("name"),
                server: row.get("server"),
                port: row.get::<i64, _>("port") as u16,
                raw: row.get("raw"),
                config,
                country: row.get("country"),
                latency_ms: row.get("latency_ms"),
                latency_ok: row.get::<Option<i64>, _>("latency_ok").map(|value| value != 0),
                download_kbps: row.try_get("download_kbps").ok().flatten(),
                latency_kernel: row.get("latency_kernel"),
                latency_test_kind: row.get("latency_test_kind"),
                latency_tested_at: tested_at.as_deref().map(super::parse_dt),
                created_at: created_at
                    .as_deref()
                    .map(super::parse_dt)
                    .unwrap_or_else(Utc::now),
                updated_at: updated_at
                    .as_deref()
                    .map(super::parse_dt)
                    .unwrap_or_else(Utc::now),
            };
            let key = node_fingerprint(&carried);
            if keep_missing && !batch.contains_key(&key) {
                leftovers.insert(key.clone(), carried.clone());
            }
            preserved.insert(
                key,
                PreservedLatency {
                    latency_ms: carried.latency_ms,
                    latency_ok: carried.latency_ok.map(|value| value as i64),
                    download_kbps: carried.download_kbps,
                    latency_kernel: carried.latency_kernel.clone(),
                    latency_test_kind: carried.latency_test_kind.clone(),
                    latency_tested_at: carried.latency_tested_at.map(|v| v.to_rfc3339()),
                },
            );
        }

        let mut inserted = 0usize;
        let mut tx = temp_pool.begin().await?;
        for n in batch.values() {
            let p = preserved.get(&node_fingerprint(n));
            let mut fresh = (*n).clone();
            fresh.source_id = Some(source_id.to_owned());
            if let Some(p) = p {
                fresh.latency_ms = p.latency_ms;
                fresh.latency_ok = p.latency_ok.map(|value| value != 0);
                fresh.download_kbps = p.download_kbps;
                fresh.latency_kernel = p.latency_kernel.clone();
                fresh.latency_test_kind = p.latency_test_kind.clone();
                fresh.latency_tested_at = p.latency_tested_at.as_deref().map(super::parse_dt);
            }
            Self::insert_node(&mut tx, &fresh).await?;
            inserted += 1;
        }

        // 累积模式：这一次页面上看不见的旧节点原样搬回去（延迟记录也一起搬）。
        let mut carried_over = 0usize;
        for (_, old) in leftovers {
            let mut carry = old;
            carry.id = Uuid::new_v4().to_string();
            Self::insert_node(&mut tx, &carry).await?;
            carried_over += 1;
        }

        tx.commit().await?;
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&temp_pool)
            .await?;
        temp_pool.close().await;
        old_pool.close().await;
        if destination.exists() {
            let backup = self
                .node_root
                .join(format!(".{source_id}-{}.bak", Uuid::new_v4()));
            std::fs::rename(&destination, &backup)?;
            if let Err(error) = std::fs::rename(&temporary, &destination) {
                let _ = std::fs::rename(&backup, &destination);
                return Err(error.into());
            }
            let _ = std::fs::remove_file(backup);
        } else {
            std::fs::rename(&temporary, &destination)?;
        }
        // 整源换血：汇总表必须重扫，否则候选还是上一批的 id。
        self.invalidate_node_index().await;
        Ok(inserted + carried_over)
    }

    pub async fn import_nodes(&self, nodes: &[Node]) -> Result<usize> {
        if nodes.is_empty() {
            return Ok(0);
        }
        let mut batch: std::collections::HashMap<String, &Node> = std::collections::HashMap::new();
        for n in nodes {
            let key = node_fingerprint(n);
            match batch.get(&key) {
                Some(old) => {
                    let old_q = super::name_quality_score(&old.name);
                    let new_q = super::name_quality_score(&n.name);
                    if new_q > old_q || (new_q == old_q && n.updated_at >= old.updated_at) {
                        batch.insert(key, n);
                    }
                }
                None => {
                    batch.insert(key, n);
                }
            }
        }

        let pool = self.open_node_pool(None).await?;
        let mut tx = pool.begin().await?;
        let mut stored = 0usize;
        for n in batch.values() {
            let mut fresh = (*n).clone();
            fresh.source_id = None;
            Self::insert_node(&mut tx, &fresh).await?;
            stored += 1;
        }
        tx.commit().await?;
        self.invalidate_node_index().await;
        Ok(stored)
    }
}

use super::Database;
use crate::models::{NodeGroupSummary, ProtocolStat, Stats};
use anyhow::{anyhow, Result};
use sqlx::Row;

/// 单个节点库的统计聚合结果。
#[derive(Default)]
pub(crate) struct ScopeNodeStats {
    pub(crate) node_count: i64,
    pub(crate) tested_count: i64,
    pub(crate) alive_count: i64,
    pub(crate) protocol_counts: std::collections::HashMap<String, i64>,
}

impl Database {
    /// 单个节点库的统计聚合，SQL 端完成，不把行数据拉进内存。
    pub(crate) async fn scope_node_stats(
        &self,
        scope: Option<&str>,
    ) -> Result<Option<ScopeNodeStats>> {
        let path = self.node_db_path(scope);
        if !path.exists() {
            return Ok(None);
        }
        let pool = self.open_node_pool(scope).await?;
        let rows = sqlx::query(
            r#"SELECT protocol, COUNT(*) AS c,
                      SUM(CASE WHEN latency_ok IS NOT NULL AND latency_test_kind = 'google_proxy'
                          THEN 1 ELSE 0 END) AS tested,
                      SUM(CASE WHEN latency_ok = 1 AND latency_test_kind = 'google_proxy'
                          THEN 1 ELSE 0 END) AS alive
               FROM nodes GROUP BY protocol"#,
        )
        .fetch_all(&pool)
        .await?;
        let mut stats = ScopeNodeStats::default();
        for row in rows {
            let count: i64 = row.get("c");
            stats.node_count += count;
            stats.tested_count += row.get::<i64, _>("tested");
            stats.alive_count += row.get::<i64, _>("alive");
            *stats
                .protocol_counts
                .entry(row.get("protocol"))
                .or_default() += count;
        }
        Ok(Some(stats))
    }

    /// (节点总数, 测通数) 跨所有分源。
    /// 测通数就是"自动切换还有多少替补可挑"的直接读数 —— 补测判定用它，
    /// 调度器每 30 秒问一次。
    ///
    /// 索引**已经新鲜**时搭它的便车（0.5 ms），过期时宁可自己扇出数一遍（3 ms）：
    /// 为一笔三毫秒的查询去付 111 ms 的全量重建是亏的（真实副本 A/B 实测，
    /// 见 `node_index::ensure_node_index` 的注释）。重建由真正需要它的读法负责
    /// —— 选 url-test 候选那条路每次都要扫 35 个库，它顺手把索引养着。
    pub async fn node_verification_counts(&self) -> Result<(i64, i64)> {
        if self.node_index_is_fresh().await {
            return Ok(self.node_index_counts().await);
        }
        self.scan_node_verification_counts().await
    }

    /// 回落路径：单个分库出错只 warn 并跳过：那会让总数偏低，判定于是偏向"先不折腾"，
    /// 是安全的一侧；但不能静默，否则又变成当年那条 `??` 占位符 bug 一样的查无实据。
    pub(crate) async fn scan_node_verification_counts(&self) -> Result<(i64, i64)> {
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            let path = self.node_db_path(scope.as_deref());
            if !path.exists() {
                continue;
            }
            let db = self.clone();
            tasks.spawn(async move {
                let Ok(pool) = db.open_node_pool(scope.as_deref()).await else {
                    tracing::warn!("读取测通数量失败：打不开分库 {}", path.display());
                    return (0i64, 0i64);
                };
                match sqlx::query_as::<_, (i64, i64)>(
                    "SELECT COUNT(*), COALESCE(SUM(latency_ok = 1), 0) FROM nodes",
                )
                .fetch_one(&pool)
                .await
                {
                    Ok(pair) => pair,
                    Err(error) => {
                        tracing::warn!(%error, "读取测通数量失败（该分库按 0 计）");
                        (0, 0)
                    }
                }
            });
        }
        let mut total = 0i64;
        let mut verified = 0i64;
        while let Some(joined) = tasks.join_next().await {
            if let Ok((scope_total, scope_verified)) = joined {
                total += scope_total;
                verified += scope_verified;
            }
        }
        Ok((total, verified))
    }

    /// 单个节点库按过滤条件的 (总数, 存活数)，过滤条件由 `where_sql`/`binds` 下推。
    pub(crate) async fn scope_node_group(
        &self,
        scope: &Option<String>,
        where_sql: &str,
        binds: &[String],
    ) -> Result<(Option<String>, i64, i64)> {
        let path = self.node_db_path(scope.as_deref());
        if !path.exists() {
            return Ok((scope.clone(), 0, 0));
        }
        let pool = self.open_node_pool(scope.as_deref()).await?;
        let group_sql = format!(
            r#"SELECT COUNT(*) AS total,
                      COALESCE(SUM(CASE WHEN latency_ok = 1 AND latency_test_kind = 'google_proxy'
                          THEN 1 ELSE 0 END), 0) AS alive
               FROM nodes {where_sql}"#
        );
        let mut query = sqlx::query(&group_sql);
        for bind in binds {
            query = query.bind(bind);
        }
        let row = query.fetch_one(&pool).await?;
        Ok((scope.clone(), row.get("total"), row.get("alive")))
    }

    pub async fn list_node_groups(
        &self,
        protocol: Option<&str>,
        keyword: Option<&str>,
        latency: Option<&str>,
    ) -> Result<Vec<NodeGroupSummary>> {
        let sources = self.list_sources().await?;
        let names = sources
            .into_iter()
            .map(|source| (source.id, source.name))
            .collect::<std::collections::HashMap<_, _>>();
        // 过滤条件全部下推 SQL，每组只回一条聚合行，不再整表加载。
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
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            let this = self.clone();
            let where_sql = where_sql.clone();
            let binds = binds.clone();
            tasks.spawn(async move { this.scope_node_group(&scope, &where_sql, &binds).await });
        }
        let mut groups = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            let (scope, total, alive) = joined.map_err(|error| anyhow!(error.to_string()))??;
            if total == 0 {
                continue;
            }
            groups.push(NodeGroupSummary {
                source_id: scope.clone(),
                source_name: scope
                    .as_ref()
                    .and_then(|id| names.get(id))
                    .cloned()
                    .unwrap_or_else(|| "未分组".to_owned()),
                total,
                alive,
            });
        }
        groups.sort_by(|a, b| a.source_name.cmp(&b.source_name));
        Ok(groups)
    }

    pub async fn stats(&self) -> Result<Stats> {
        let source_count = self.source_count().await?;
        let enabled_source_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sources WHERE enabled = 1")
                .fetch_one(&self.pool)
                .await?;
        // 聚合下推到 SQL 端完成：整表加载 2.6 万节点到内存只为数数太浪费。
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            let this = self.clone();
            tasks.spawn(async move { this.scope_node_stats(scope.as_deref()).await });
        }
        let mut merged = ScopeNodeStats::default();
        while let Some(joined) = tasks.join_next().await {
            if let Some(scope_stats) = joined.map_err(|error| anyhow!(error.to_string()))?? {
                merged.node_count += scope_stats.node_count;
                merged.tested_count += scope_stats.tested_count;
                merged.alive_count += scope_stats.alive_count;
                for (protocol, count) in scope_stats.protocol_counts {
                    *merged.protocol_counts.entry(protocol).or_default() += count;
                }
            }
        }
        let mut protocol_stats = merged
            .protocol_counts
            .into_iter()
            .map(|(protocol, count)| ProtocolStat { protocol, count })
            .collect::<Vec<_>>();
        protocol_stats.sort_by(|a, b| b.count.cmp(&a.count));
        Ok(Stats {
            source_count,
            enabled_source_count,
            node_count: merged.node_count,
            tested_count: merged.tested_count,
            alive_count: merged.alive_count,
            protocol_stats,
        })
    }

    pub async fn source_count(&self) -> Result<i64> {
        let c: i64 = sqlx::query("SELECT COUNT(*) as c FROM sources")
            .fetch_one(&self.pool)
            .await?
            .get("c");
        Ok(c)
    }
}

use super::Database;
use crate::models::{
    CreateSource, PruneHomogeneityResult, Source, SourceHomogeneityReport, UpdateSource,
};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use sqlx::Row;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

impl Database {
    pub async fn list_sources(&self) -> Result<Vec<Source>> {
        let rows = sqlx::query(
            r#"SELECT id, name, url_template, enabled, remark, last_fetch_at,
                      last_status, last_node_count, last_resolved_url, last_etag,
                      last_modified, content_sha256, fail_streak, last_fetch_host,
                      push_disabled, created_at, updated_at
               FROM sources
               ORDER BY created_at DESC"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_source).collect())
    }

    pub async fn get_source(&self, id: &str) -> Result<Option<Source>> {
        let row = sqlx::query(
            r#"SELECT id, name, url_template, enabled, remark, last_fetch_at,
                      last_status, last_node_count, last_resolved_url, last_etag,
                      last_modified, content_sha256, fail_streak, last_fetch_host,
                      push_disabled, created_at, updated_at
               FROM sources
               WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_source))
    }

    pub async fn create_source(&self, input: CreateSource) -> Result<Source> {
        let now = Utc::now();
        let source = Source {
            id: Uuid::new_v4().to_string(),
            name: input.name,
            url_template: input.url_template,
            enabled: input.enabled,
            remark: input.remark,
            last_fetch_at: None,
            last_status: None,
            last_node_count: 0,
            last_resolved_url: None,
            last_etag: None,
            last_modified: None,
            content_sha256: None,
            fail_streak: 0,
            last_fetch_host: None,
            push_disabled: false,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            r#"INSERT INTO sources
               (id, name, url_template, enabled, remark, last_fetch_at, last_status,
                last_node_count, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, NULL, NULL, 0, ?, ?)"#,
        )
        .bind(&source.id)
        .bind(&source.name)
        .bind(&source.url_template)
        .bind(source.enabled as i64)
        .bind(&source.remark)
        .bind(source.created_at.to_rfc3339())
        .bind(source.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(source)
    }

    pub async fn update_source(&self, id: &str, input: UpdateSource) -> Result<Source> {
        let mut source = self
            .get_source(id)
            .await?
            .ok_or_else(|| anyhow!("source not found"))?;
        if let Some(name) = input.name {
            source.name = name;
        }
        if let Some(url) = input.url_template {
            source.url_template = url;
        }
        if let Some(enabled) = input.enabled {
            source.enabled = enabled;
        }
        if let Some(remark) = input.remark {
            source.remark = Some(remark);
        }
        if let Some(push_disabled) = input.push_disabled {
            source.push_disabled = push_disabled;
        }
        source.updated_at = Utc::now();
        sqlx::query(
            r#"UPDATE sources SET name=?, url_template=?, enabled=?, remark=?, push_disabled=?,
                      updated_at=?
               WHERE id=?"#,
        )
        .bind(&source.name)
        .bind(&source.url_template)
        .bind(source.enabled as i64)
        .bind(&source.remark)
        .bind(source.push_disabled as i64)
        .bind(source.updated_at.to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(source)
    }

    pub async fn delete_source(&self, id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM sources WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            return Ok(false);
        }
        let path = self.node_db_path(Some(id));
        self.close_node_pool(Some(id)).await;
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{}", path.display(), suffix));
            if candidate.exists() {
                std::fs::remove_file(&candidate)
                    .with_context(|| format!("delete subscription db {}", candidate.display()))?;
            }
        }
        // 这个源的分片库已经没了，索引里那一截也必须跟着作废。
        self.invalidate_node_index().await;
        Ok(true)
    }

    pub async fn mark_source_fetched(&self, id: &str, status: &str, node_count: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"UPDATE sources SET last_fetch_at=?, last_status=?, last_node_count=?, updated_at=?
               WHERE id=?"#,
        )
        .bind(&now)
        .bind(status)
        .bind(node_count)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await?;
        // 每次写状态都同时留一行历史：界面上"最近 7 次"色条靠它。只看 last_status 的话，
        // 一个源上周失败五次、今天成功一次就会显示成"正常"。
        let ok = matches!(status, "ok" | "not modified" | "content unchanged");
        self.record_fetch_history(id, ok, node_count, status).await?;
        Ok(())
    }

    /// Track consecutive network-level fetch failures so an unreachable source stops
    /// eating the whole retry budget. Non-network failures (HTTP 404, parse errors)
    /// leave the streak untouched: they are not evidence the host is unreachable.
    pub async fn record_fetch_outcome(
        &self,
        id: &str,
        ok: bool,
        network_failure: bool,
    ) -> Result<i64> {
        if ok {
            sqlx::query("UPDATE sources SET fail_streak = 0 WHERE id = ?")
                .bind(id)
                .execute(&self.pool)
                .await?;
            return Ok(0);
        }
        if network_failure {
            sqlx::query("UPDATE sources SET fail_streak = fail_streak + 1 WHERE id = ?")
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        let streak = sqlx::query_scalar::<_, i64>("SELECT fail_streak FROM sources WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(streak.unwrap_or(0))
    }

    /// Remember which host (direct or mirror) served the last successful fetch so the
    /// next round can race from there instead of starting cold.
    pub async fn set_source_fetch_host(&self, id: &str, host: &str) -> Result<()> {
        sqlx::query("UPDATE sources SET last_fetch_host = ? WHERE id = ?")
            .bind(host)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn seen_gist_ids(&self) -> Result<std::collections::HashSet<String>> {
        let rows = sqlx::query_scalar::<_, String>("SELECT gist_id FROM seen_gists")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// state: registered（已注册为源）/ unusable（无节点）/ pending（网络失败，下轮重试）
    pub async fn mark_gist_seen(
        &self,
        id: &str,
        _gist_user: &str,
        _gist_name: &str,
        source_id: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT OR REPLACE INTO seen_gists (gist_id, source_id, first_seen_at) \
             VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(source_id)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_source_cache(
        &self,
        id: &str,
        resolved_url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        content_sha256: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"UPDATE sources SET last_resolved_url=?, last_etag=?, last_modified=?,
                      content_sha256=?, updated_at=? WHERE id=?"#,
        )
        .bind(resolved_url)
        .bind(etag)
        .bind(last_modified)
        .bind(content_sha256)
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 分析所有订阅源之间的重合度与同质化水平
    pub async fn analyze_source_homogeneity(&self) -> Result<Vec<SourceHomogeneityReport>> {
        let sources = self.list_sources().await?;
        if sources.is_empty() {
            return Ok(Vec::new());
        }

        let all_nodes = self.load_node_summaries(None).await?;

        // 建立 endpoint 索引: (protocol, server, port) -> Vec<(source_id, is_alive)>
        let mut endpoint_map: HashMap<(String, String, u16), Vec<(String, bool)>> = HashMap::new();
        for node in &all_nodes {
            let src_id = match &node.source_id {
                Some(s) if !s.trim().is_empty() => s.clone(),
                _ => continue,
            };
            let key = (
                node.protocol.to_ascii_lowercase(),
                node.server.trim().to_ascii_lowercase(),
                node.port,
            );
            let is_alive = node.latency_ok.unwrap_or(false);
            endpoint_map
                .entry(key)
                .or_default()
                .push((src_id, is_alive));
        }

        let mut reports = Vec::new();
        for source in &sources {
            let mut source_endpoints: HashMap<(String, String, u16), bool> = HashMap::new();
            for node in &all_nodes {
                if node.source_id.as_deref() == Some(&source.id) {
                    let key = (
                        node.protocol.to_ascii_lowercase(),
                        node.server.trim().to_ascii_lowercase(),
                        node.port,
                    );
                    let is_alive = node.latency_ok.unwrap_or(false);
                    let entry = source_endpoints.entry(key).or_insert(false);
                    if is_alive {
                        *entry = true;
                    }
                }
            }

            let total_nodes = source_endpoints.len();
            let mut alive_nodes = 0usize;
            let mut unique_nodes = 0usize;
            let mut unique_alive_nodes = 0usize;

            for (key, is_alive) in &source_endpoints {
                if *is_alive {
                    alive_nodes += 1;
                }
                let providers = endpoint_map.get(key);
                let other_count = providers
                    .map(|list| list.iter().filter(|(sid, _)| sid != &source.id).count())
                    .unwrap_or(0);

                if other_count == 0 {
                    unique_nodes += 1;
                    if *is_alive {
                        unique_alive_nodes += 1;
                    }
                }
            }

            let duplicate_nodes = total_nodes.saturating_sub(unique_nodes);
            let overlap_rate_pct = if total_nodes > 0 {
                (duplicate_nodes as f32 / total_nodes as f32) * 100.0
            } else {
                0.0
            };

            // 冗余源判定：
            // 1. 该源有节点，但独有可用节点为 0，且重合率 >= 85%（或自身存活为0）
            // 2. 或重合率 >= 98%
            // 3. 或总节点为 0 且连续失败 >= 2 次
            let is_redundant = if total_nodes > 0 {
                (unique_alive_nodes == 0 && (overlap_rate_pct >= 85.0 || alive_nodes == 0))
                    || overlap_rate_pct >= 98.0
            } else {
                source.fail_streak >= 2
            };

            reports.push(SourceHomogeneityReport {
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                total_nodes,
                alive_nodes,
                duplicate_nodes,
                unique_nodes,
                unique_alive_nodes,
                overlap_rate_pct,
                is_redundant,
            });
        }

        reports.sort_by(|a, b| {
            b.is_redundant
                .cmp(&a.is_redundant)
                .then_with(|| {
                    b.overlap_rate_pct
                        .partial_cmp(&a.overlap_rate_pct)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.unique_alive_nodes.cmp(&b.unique_alive_nodes))
        });

        Ok(reports)
    }

    /// 反向剔除同质化源：可选择批量禁用（保留配置关闭拉取）或彻底删除
    pub async fn prune_homogenized_sources(
        &self,
        source_ids: &[String],
        delete_records: bool,
    ) -> Result<PruneHomogeneityResult> {
        let mut affected_names = Vec::new();
        for id in source_ids {
            if let Some(src) = self.get_source(id).await? {
                affected_names.push(src.name);
                if delete_records {
                    self.delete_source(id).await?;
                } else {
                    self.update_source(
                        id,
                        UpdateSource {
                            name: None,
                            url_template: None,
                            enabled: Some(false),
                            remark: None,
                            push_disabled: None,
                        },
                    )
                    .await?;
                }
            }
        }
        Ok(PruneHomogeneityResult {
            pruned_count: affected_names.len(),
            affected_names,
            deleted_records: delete_records,
        })
    }
}

pub(crate) fn row_to_source(r: sqlx::sqlite::SqliteRow) -> Source {
    Source {
        id: r.get("id"),
        name: r.get("name"),
        url_template: r.get("url_template"),
        enabled: r.get::<i64, _>("enabled") != 0,
        remark: r.get("remark"),
        last_fetch_at: r
            .get::<Option<String>, _>("last_fetch_at")
            .map(|s| super::parse_dt(&s)),
        last_status: r.get("last_status"),
        last_node_count: r.get("last_node_count"),
        last_resolved_url: r.try_get("last_resolved_url").ok().flatten(),
        last_etag: r.try_get("last_etag").ok().flatten(),
        last_modified: r.try_get("last_modified").ok().flatten(),
        content_sha256: r.try_get("content_sha256").ok().flatten(),
        fail_streak: r.try_get("fail_streak").ok().flatten().unwrap_or(0),
        last_fetch_host: r.try_get("last_fetch_host").ok().flatten(),
        push_disabled: r.try_get::<i64, _>("push_disabled").unwrap_or(0) != 0,
        created_at: super::parse_dt(r.get::<String, _>("created_at").as_str()),
        updated_at: super::parse_dt(r.get::<String, _>("updated_at").as_str()),
    }
}

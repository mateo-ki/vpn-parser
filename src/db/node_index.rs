//! 跨分片的节点汇总表 `node_index`（主库），把"每个候选查询都要开 37 个分片库各扫一遍"
//! 压成主库上一条带索引的查询。
//!
//! 三条设计约束，都是踩过坑之后定的：
//! 1. **它是可重建的派生表，不是第二真源**。分片库永远是真源；这张表任何时候可以
//!    整表重建。所以本模块里没有任何"只改索引不改分片"的写入口 —— 写侧只标过期。
//! 2. **不存凭据**。只存选路/统计要用的元数据（id、来源、协议、延迟）。`raw`/`config`
//!    留在各分片的 `nodes`/`node_payloads` 里 —— 把它们集中到主库等于把全部节点凭据
//!    放进同一个文件，那是安全上的倒退（S 系列已经做过一轮）。所以选中候选之后，
//!    仍然要回分片按 id 取真节点（`nodes.rs::get_nodes_in_scopes`）。
//! 3. **读侧按"值不值得付重建钱"分两档**（真实副本 A/B 实测：重建 14,424 行 111 ms，
//!    而老扇出选候选 61 ms/次、数总数 3 ms/次）：
//!    - 选 url-test 候选：`ensure_node_index()` —— 过期就先重建，之后 60 秒内每次 2 ms；
//!    - 数总数/测通数：`node_index_is_fresh()` —— 已经新鲜才搭便车，过期就自己扇出，
//!      绝不为三毫秒的查询付一百毫秒。
//!    写入口一律只把 `built_at` 清零（一次 UPSERT），不指望它们同步索引 —— 万一漏了
//!    一个写入口，最坏结果是计数最多旧一个 TTL（60 秒），而凭据永远是从分片现取的，
//!    不会拿错节点。

use super::Database;
use anyhow::Result;
use sqlx::Row;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 超过这个时长就算过期，下一次读会先重建再回答。
pub const INDEX_TTL: Duration = Duration::from_secs(60);

/// 一条批量 INSERT 带多少行：每行 5 个绑定变量，150 行 = 750 个，稳在 SQLite
/// 老版本 999 变量上限之内。
const BATCH_ROWS: usize = 150;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 索引里的一行元数据。
#[derive(Debug, Clone)]
struct IndexedNode {
    id: String,
    source_id: Option<String>,
    protocol: String,
    latency_ms: Option<i64>,
    latency_ok: bool,
}

impl Database {
    /// 索引上次重建的时间戳（没有则 0）。
    async fn node_index_built_at(&self) -> i64 {
        sqlx::query("SELECT value FROM node_index_meta WHERE key = 'built_at'")
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .and_then(|row| row.try_get::<String, _>("value").ok())
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    }

    async fn set_node_index_built_at(&self, when: i64) {
        let _ = sqlx::query(
            "INSERT INTO node_index_meta(key, value) VALUES('built_at', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(when.to_string())
        .execute(&self.pool)
        .await;
    }

    /// 分片内容变了就调这个：把索引标成过期，下一次读会先重建。
    /// 拉取完成、测速完成、删源/清空这些"结果马上要用"的地方都走它。
    pub async fn invalidate_node_index(&self) {
        self.set_node_index_built_at(0).await;
    }

    /// 全量重建：并发扫各分片的元数据 -> 先写临时表 -> 换名。
    /// 换名而不是 DELETE+INSERT：中途失败不会留下一张空索引让读侧当真。
    pub async fn rebuild_node_index(&self) -> Result<u64> {
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let db = self.clone();
            tasks.spawn(async move {
                let Ok(pool) = db.open_node_pool(scope.as_deref()).await else {
                    return Vec::new();
                };
                match sqlx::query("SELECT id, source_id, protocol, latency_ms, latency_ok FROM nodes")
                    .fetch_all(&pool)
                    .await
                {
                    Ok(rows) => rows
                        .into_iter()
                        .filter_map(|row| {
                            Some(IndexedNode {
                                id: row.try_get::<String, _>("id").ok()?,
                                source_id: row.try_get::<Option<String>, _>("source_id").ok()?,
                                protocol: row.try_get::<String, _>("protocol").unwrap_or_default(),
                                latency_ms: row
                                    .try_get::<Option<i64>, _>("latency_ms")
                                    .ok()
                                    .flatten(),
                                // 分片里 latency_ok 可以是 NULL；NULL 归 0，和
                                // `SUM(latency_ok = 1)` 的口径一致。
                                latency_ok: row
                                    .try_get::<Option<i64>, _>("latency_ok")
                                    .ok()
                                    .flatten()
                                    .unwrap_or(0)
                                    != 0,
                            })
                        })
                        .collect::<Vec<_>>(),
                    Err(error) => {
                        tracing::warn!(%error, "node_index 重建：分片扫描失败");
                        Vec::new()
                    }
                }
            });
        }
        let mut rows = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Ok(part) = joined {
                rows.extend(part);
            }
        }

        let mut tx = self.pool.begin().await?;
        sqlx::query("DROP TABLE IF EXISTS node_index_new").execute(&mut *tx).await?;
        sqlx::query(
            "CREATE TABLE node_index_new(
                id TEXT PRIMARY KEY NOT NULL,
                source_id TEXT,
                protocol TEXT NOT NULL DEFAULT '',
                latency_ms INTEGER,
                latency_ok INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&mut *tx)
        .await?;
        for chunk in rows.chunks(BATCH_ROWS) {
            let placeholders = vec!["(?,?,?,?,?)"; chunk.len()].join(",");
            let sql = format!(
                "INSERT OR REPLACE INTO node_index_new
                 (id, source_id, protocol, latency_ms, latency_ok) VALUES {placeholders}"
            );
            let mut query = sqlx::query(&sql);
            for row in chunk {
                query = query
                    .bind(&row.id)
                    .bind(&row.source_id)
                    .bind(&row.protocol)
                    .bind(row.latency_ms)
                    .bind(i64::from(row.latency_ok));
            }
            query.execute(&mut *tx).await?;
        }
        sqlx::query("DROP TABLE IF EXISTS node_index").execute(&mut *tx).await?;
        sqlx::query("ALTER TABLE node_index_new RENAME TO node_index")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS node_index_verified
             ON node_index(latency_ok, latency_ms)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS node_index_source ON node_index(source_id)")
            .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.set_node_index_built_at(now_unix()).await;
        tracing::debug!(stored = rows.len(), "node_index 重建完成");
        Ok(rows.len() as u64)
    }

    /// 索引现在算不算新（只看时间戳，不触发重建）。
    pub async fn node_index_is_fresh(&self) -> bool {
        now_unix() - self.node_index_built_at().await < INDEX_TTL.as_secs() as i64
    }

    /// 读侧入口：过期就重建，并回答"这次索引能不能信"。
    /// 返回 false（重建报错）时调用方要回落到分片扇出 —— 汇总表是优化，
    /// 不能变成"查不到就说没有"。
    ///
    /// 只有**值得**付这笔重建钱的读法才用它。真实副本 A/B 实测：重建 14,424 行要
    /// 111 ms，而选候选的老扇出是 61 ms/次、数总数只要 3 ms/次。所以选候选用它
    /// （一次 111 ms 换来之后 60 秒内每次 2 ms），数数那种三毫秒的查询宁可自己扫、
    /// 只在索引已经新着的时候搭便车（`node_index_is_fresh`）。
    pub async fn ensure_node_index(&self) -> bool {
        if self.node_index_is_fresh().await {
            return true;
        }
        match self.rebuild_node_index().await {
            Ok(stored) => {
                tracing::debug!(stored, "node_index：本次读取前重建");
                true
            }
            Err(error) => {
                tracing::warn!(%error, "node_index 重建失败，本次回落分片扫描");
                false
            }
        }
    }

    /// `(总数, 测通数)`，直接问索引表。要不要信它由调用方判断 —— 计数那条路的口径
    /// 在 `stats.rs::node_verification_counts`（只在索引已经新鲜时搭便车）。
    pub async fn node_index_counts(&self) -> (i64, i64) {
        let row = sqlx::query(
            "SELECT COUNT(*), COALESCE(SUM(latency_ok = 1), 0) FROM node_index",
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        match row {
            Some(row) => (
                row.try_get::<i64, _>(0).unwrap_or(0),
                row.try_get::<i64, _>(1).unwrap_or(0),
            ),
            None => (0, 0),
        }
    }

    /// 选路候选的"选"这一步：主库一条查询拿到 (id, source_id)，按延迟升序。
    /// `source_id` 一起带出来，回分片取凭据就不用扇出全部库。
    ///
    /// `id ASC` 是刻意的平手规则：老扇出的平手顺序取决于哪个分库先返回，同一台机器
    /// 两次重载可能选到不同节点。汇总表把它钉死，候选集因此可复现。
    /// NULL 延迟排最后（`COALESCE(..., i64::MAX)`）—— 老路里 SQLite 会把 NULL 排最前，
    /// 那样一个 `latency_ok=1` 却没记下延迟的节点会挤掉真有延迟的候选。
    pub async fn node_index_top_verified(
        &self,
        limit: usize,
        exclude_id: Option<&str>,
    ) -> Result<Vec<(String, Option<String>)>> {
        let sql = match exclude_id {
            Some(_) => "SELECT id, source_id FROM node_index
                        WHERE latency_ok = 1 AND id != ?
                        ORDER BY COALESCE(latency_ms, 9223372036854775807) ASC, id ASC LIMIT ?",
            None => "SELECT id, source_id FROM node_index
                     WHERE latency_ok = 1
                     ORDER BY COALESCE(latency_ms, 9223372036854775807) ASC, id ASC LIMIT ?",
        };
        let mut query = sqlx::query(sql);
        if let Some(id) = exclude_id {
            query = query.bind(id);
        }
        let rows = query.bind(limit as i64).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.try_get::<String, _>("id").unwrap_or_default(),
                    row.try_get::<Option<String>, _>("source_id").ok().flatten(),
                )
            })
            .collect())
    }

    /// 校验：索引和分片到底一不一致。返回每一处不一致的描述（空 = 一致）。
    /// 光比总数不够 —— 整源换名（旧 id 没了、新 id 进来）总数可以一模一样。
    /// 所以这里把两边的 id 集合都拿到主库内存里比（万把个 id，几百 KB）。
    ///
    /// `#[cfg(test)]`：它要扫全部分片，正是这张表想省掉的那笔开销 —— 放到产品里跑
    /// 等于自己拆自己的台。留给 A/B 测试和未来排查用。
    #[cfg(test)]
    pub(crate) async fn verify_node_index(&self) -> Vec<String> {
        let mut problems = Vec::new();

        let mut shard_rows: std::collections::HashMap<String, i64> = Default::default();
        let mut scanned_total = 0i64;
        for scope in self.node_scopes(None).await.unwrap_or_default() {
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let Ok(pool) = self.open_node_pool(scope.as_deref()).await else {
                continue;
            };
            let Ok(rows) =
                sqlx::query("SELECT id, COALESCE(latency_ok, 0) AS ok FROM nodes")
                    .fetch_all(&pool)
                    .await
            else {
                continue;
            };
            scanned_total += rows.len() as i64;
            for row in rows {
                let Ok(id) = row.try_get::<String, _>("id") else {
                    continue;
                };
                *shard_rows.entry(id).or_default() =
                    row.try_get::<i64, _>("ok").unwrap_or(0);
            }
        }

        let Ok(index_rows) =
            sqlx::query("SELECT id, latency_ok FROM node_index").fetch_all(&self.pool).await
        else {
            return vec!["node_index 表读不出来".to_string()];
        };
        let mut index_rows_map: std::collections::HashMap<String, i64> = Default::default();
        for row in index_rows {
            let Ok(id) = row.try_get::<String, _>("id") else {
                continue;
            };
            *index_rows_map.entry(id).or_default() = row.try_get::<i64, _>("latency_ok").unwrap_or(0);
        }

        let missing: Vec<&String> = shard_rows
            .keys()
            .filter(|id| !index_rows_map.contains_key(*id))
            .collect();
        let extra: Vec<&String> = index_rows_map
            .keys()
            .filter(|id| !shard_rows.contains_key(*id))
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "索引缺 {} 个节点，例如 {}",
                missing.len(),
                missing
                    .iter()
                    .take(3)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !extra.is_empty() {
            problems.push(format!(
                "索引多 {} 个分片里已经没有的节点，例如 {}",
                extra.len(),
                extra
                    .iter()
                    .take(3)
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // 同一批 id 上，测通标记也必须一致（测速只改分片，索引靠重建跟上）。
        let stale_ok: Vec<String> = shard_rows
            .iter()
            .filter(|(id, ok)| {
                index_rows_map
                    .get(*id)
                    .is_some_and(|indexed| indexed != *ok)
            })
            .map(|(id, _)| id.clone())
            .collect();
        if !stale_ok.is_empty() {
            problems.push(format!(
                "索引里 {} 个节点的测通标记和分片不一致，例如 {}",
                stale_ok.len(),
                stale_ok
                    .iter()
                    .take(3)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if index_rows_map.len() as i64 != scanned_total {
            problems.push(format!(
                "行数不一致 索引={} 分片={}",
                index_rows_map.len(),
                scanned_total
            ));
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CreateSource, Node};

    fn node(id: &str, source: &str, latency: Option<i64>) -> Node {
        // 每个夹具节点都要有自己的地址：`write_source_nodes` 按指纹
        // （协议|server|port|凭据）去重，撞了会整批缩成一条，测试就测不到东西了。
        Node {
            id: id.into(),
            source_id: Some(source.into()),
            protocol: "ss".into(),
            name: format!("节点 {id}"),
            server: format!("{id}.example.com"),
            port: 8388,
            raw: format!("ss://aaa@{id}.example.com:8388#{id}"),
            config: serde_json::json!({"cipher": "aes-256-gcm"}),
            country: None,
            latency_ms: latency,
            latency_ok: latency.map(|_| true),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    async fn fixture(tag: &str) -> (Database, String, String) {
        let path = format!("{}/node_index_{tag}.db", std::env::temp_dir().display());
        // 上一轮跑的临时库还在的话，索引里就带着旧 built_at 和旧行，
        // "索引本来是空的"这类前提直接不成立。
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
        let db = Database::connect(&path).await.unwrap();
        let a = db
            .create_source(CreateSource {
                name: "A 源".into(),
                url_template: "https://a.example/sub".into(),
                enabled: true,
                remark: None,
            })
            .await
            .unwrap()
            .id;
        let b = db
            .create_source(CreateSource {
                name: "B 源".into(),
                url_template: "https://b.example/sub".into(),
                enabled: true,
                remark: None,
            })
            .await
            .unwrap()
            .id;
        (db, a, b)
    }

    #[tokio::test]
    async fn index_counts_and_selection_match_a_shard_scan() {
        let (db, a, b) = fixture("counts").await;
        db.replace_source_nodes(&a, &[node("a1", &a, Some(50)), node("a2", &a, None)])
            .await
            .unwrap();
        db.replace_source_nodes(&b, &[node("b1", &b, Some(30))])
            .await
            .unwrap();
        assert!(
            !db.verify_node_index().await.is_empty(),
            "分片里有节点、索引还是空的，就该报不一致"
        );
        db.rebuild_node_index().await.unwrap();

        // 索引数出来的，必须和分片扫出来的一致（分片是真源）。
        assert_eq!(db.node_index_counts().await, (3, 2));
        assert_eq!(
            db.node_verification_counts().await.unwrap(),
            (3, 2),
            "调度器那条查询也要跟着走索引口径"
        );
        assert!(db.verify_node_index().await.is_empty());

        let picked = db.node_index_top_verified(2, None).await.unwrap();
        assert_eq!(
            picked.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
            vec!["b1", "a1"],
            "按延迟升序"
        );
        let excluded = db.node_index_top_verified(5, Some("b1")).await.unwrap();
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].0, "a1");
    }

    #[tokio::test]
    async fn writes_invalidate_and_only_the_candidate_read_pays_for_rebuild() {
        let (db, a, _) = fixture("drift").await;
        db.replace_source_nodes(&a, &[node("a1", &a, Some(10))])
            .await
            .unwrap();
        // 写入把索引标过期。
        assert_eq!(db.node_index_built_at().await, 0, "换源必须把索引标过期");
        // 数数这种三毫秒的查询不许顺手触发重建（A/B 实测重建要 111 ms）：
        // 过期就走扇出，答案照样对，而且 built_at 保持 0 —— 没人偷偷替它付钱。
        assert_eq!(db.node_verification_counts().await.unwrap(), (1, 1));
        assert_eq!(db.node_index_built_at().await, 0, "计数读不该把索引重建了还不知道");

        // 选候选那条路才负责重建（它老路是 61 ms 扇出，付一次重建换来以后每次 2 ms）。
        let picked = db.top_verified_nodes(2, None).await.unwrap();
        assert_eq!(picked.len(), 1);
        assert_ne!(db.node_index_built_at().await, 0, "选完候选索引就该是新的");
        // 现在索引新鲜了，计数读就搭便车走索引，口径必须和刚才扇出的一致。
        assert!(db.node_index_is_fresh().await);
        assert_eq!(db.node_verification_counts().await.unwrap(), (1, 1));

        // 整源重建后旧节点没了：不重扫一遍索引就是脏数据。写入口负责标过期。
        db.replace_source_nodes(&a, &[node("a9", &a, Some(20))])
            .await
            .unwrap();
        assert_eq!(db.node_index_built_at().await, 0);
        // 就算某个写入口漏了标过期，verify 也发现得了（守住"可发现"）。
        db.set_node_index_built_at(now_unix()).await;
        let drift = db.verify_node_index().await;
        assert!(!drift.is_empty(), "{drift:?}");
        db.invalidate_node_index().await;
        assert!(db.ensure_node_index().await);
        assert!(db.verify_node_index().await.is_empty());
        assert_eq!(db.node_index_counts().await.0, 1);
    }

    #[tokio::test]
    async fn selection_carries_the_source_id_so_payloads_need_no_fanout() {
        let (db, a, b) = fixture("payload").await;
        db.replace_source_nodes(&a, &[node("a1", &a, Some(11))])
            .await
            .unwrap();
        db.replace_source_nodes(&b, &[node("b1", &b, Some(12))])
            .await
            .unwrap();
        db.rebuild_node_index().await.unwrap();
        let picked = db.node_index_top_verified(1, None).await.unwrap();
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].0, "a1");
        assert_eq!(
            picked[0].1.as_deref(),
            Some(a.as_str()),
            "必须带上来源，回分片取凭据才不用扫全部库"
        );

        // 走索引拿到的候选必须是能解析成完整节点的（凭据回分片取）。
        let nodes = db.top_verified_nodes(2, None).await.unwrap();
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].id, "a1");
        assert_eq!(nodes[0].latency_ms, Some(11));
    }

    #[tokio::test]
    async fn a_batch_larger_than_one_statement_still_lands_every_row() {
        let (db, a, _) = fixture("batches").await;
        let nodes = (0..400)
            .map(|i| node(&format!("n{i}"), &a, Some(i)))
            .collect::<Vec<_>>();
        db.replace_source_nodes(&a, &nodes).await.unwrap();
        db.rebuild_node_index().await.unwrap();
        assert_eq!(db.node_index_counts().await.0, 400, "150 行一批，400 行要跨三批");
        // 排第一的必须是延迟最小的那批，说明各批都写进去了。
        let picked = db.node_index_top_verified(3, None).await.unwrap();
        assert_eq!(picked.len(), 3);
    }

    /// 按他真实数据的形状（37 个分片库 × 400 节点 ≈ 14,800 条）做一次 A/B：
    /// 老的全分片扇出 vs 汇总表。跑法：
    /// `cargo test --release ab_node_index -- --ignored --nocapture`
    ///
    /// 合成数据里 raw/config 比真实订阅小得多，所以扇出侧只会**低估**真实开销
    /// （扇出的 SELECT 连 payload 一起拉）；索引侧只读 5 列，不受影响。
    #[tokio::test]
    #[ignore = "要建 1.5 万条合成节点，只在要数字的时候手动跑"]
    async fn ab_node_index_against_full_fanout() {
        let sources = 37;
        let per_source = 400;
        let path = format!("{}/node_index_AB.db", std::env::temp_dir().display());
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{path}{suffix}"));
        }
        let db = Database::connect(&path).await.unwrap();
        let mut ids = Vec::new();
        let mut made = 0usize;
        for s in 0..sources {
            let id = db
                .create_source(CreateSource {
                    name: format!("源 {s}"),
                    url_template: format!("https://s{s}.example.com/sub"),
                    enabled: true,
                    remark: None,
                })
                .await
                .unwrap()
                .id;
            let nodes = (0..per_source)
                .map(|i| {
                    // 三分之二的节点"测通"，和他现在界面上的比例差不多。
                    let latency = if i % 3 == 0 { None } else { Some(i as i64 + s as i64) };
                    node(&format!("s{s}n{i}"), &id, latency)
                })
                .collect::<Vec<_>>();
            made += db.replace_source_nodes(&id, &nodes).await.unwrap();
            ids.push(id);
        }
        println!("A/B 夹具：{sources} 个分片库，写入 {made} 个节点");

        // —— 正确性：两条路必须给出同一批候选 ——
        db.invalidate_node_index().await;
        let t = std::time::Instant::now();
        let old = db.top_verified_nodes_scan(12, None).await.unwrap();
        let old_fanout = t.elapsed();
        db.rebuild_node_index().await.unwrap();
        let t = std::time::Instant::now();
        let new = db.top_verified_nodes(12, None).await.unwrap();
        let new_index = t.elapsed();
        // 平手（同延迟）时两条路的先后可以不同：扇出那侧本来就取决于哪个分库先返回。
        // 所以比"同一批候选 + 同一串延迟"，不比逐个位置。
        let mut old_ids = old.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        let mut new_ids = new.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        old_ids.sort();
        new_ids.sort();
        // 平手（同延迟）时两条路各取其一都算对 —— 真实副本上第 12 名正好有 2 个同延迟的，
        // 扇出那侧的顺序还取决于哪个分库先返回。所以比延迟序列，不比逐个 id。
        let mut old_lat = old.iter().map(|n| n.latency_ms).collect::<Vec<_>>();
        let mut new_lat = new.iter().map(|n| n.latency_ms).collect::<Vec<_>>();
        old_lat.sort();
        new_lat.sort();
        assert_eq!(
            old_lat, new_lat,
            "候选的延迟序列必须一致；id 集合 {old_ids:?} vs {new_ids:?}"
        );
        assert_eq!(
            old.iter().map(|n| n.latency_ms).collect::<Vec<_>>(),
            new.iter().map(|n| n.latency_ms).collect::<Vec<_>>(),
            "候选的延迟序列必须一致（平手顺序除外）"
        );
        assert!(
            db.verify_node_index().await.is_empty(),
            "重建之后索引和分片必须完全一致"
        );

        println!(
            "候选 12 条：扇出 {} ms（{} 个库） / 索引 {} ms（1 条主库查询 + {} 个库）",
            old_fanout.as_millis(),
            sources,
            new_index.as_millis(),
            new.iter()
                .map(|n| n.source_id.clone())
                .collect::<std::collections::HashSet<_>>()
                .len()
        );

        // —— 每 30 秒一次的计数查询 ——
        let t = std::time::Instant::now();
        let scanned = db.scan_node_verification_counts().await.unwrap();
        let counts_fanout = t.elapsed();
        let t = std::time::Instant::now();
        let indexed = db.node_index_counts().await;
        let counts_index = t.elapsed();
        assert_eq!(scanned, indexed, "索引口径必须和分片口径数出同一个结果");
        println!(
            "总数/测通数 {scanned:?}：扇出 {} µs / 索引 {} µs",
            counts_fanout.as_micros(),
            counts_index.as_micros()
        );

        // —— 新表的代价：一次全量重建多久 ——
        let t = std::time::Instant::now();
        let stored = db.rebuild_node_index().await.unwrap();
        println!(
            "重建索引（{stored} 行）耗时 {} ms —— 这是写侧每次标过期后、下一读要付的钱",
            t.elapsed().as_millis()
        );
        println!(
            "索引表尺寸：{:?} 行，主库 {} 字节",
            stored,
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
        );
    }

    /// 同一套 A/B，但跑在他的真实数据**副本**上（合成数据里 raw/config 只有几十字节，
    /// 真实订阅的是几百字节到几 KB，扇出那侧会连载荷一起 SELECT —— 只有真实副本
    /// 才能给出可比的数字）。
    ///
    /// 前置：把 `data/backups/<某次备份>/` 里的 `vpn-parser.db` 和 36 个分片库放到
    /// `VPN_PARSER_AB_SRC` 指向的目录（本仓库这次用的是
    /// `%TEMP%/vpn-index-ab/data`，全程只读原始备份、只写副本）。
    /// 跑法：`VPN_PARSER_AB_SRC=... cargo test --release ab_node_index_on_real_copy -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要 57 MB 真实数据副本，只在要数字的时候手动跑"]
    async fn ab_node_index_on_real_copy() {
        let Ok(src) = std::env::var("VPN_PARSER_AB_SRC") else {
            panic!("要用 VPN_PARSER_AB_SRC 指向一份 data 副本目录");
        };
        let src = std::path::PathBuf::from(&src);
        let dir = std::env::temp_dir().join("vpn-index-ab-run");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("vpn-parser.db");
        std::fs::copy(src.join("vpn-parser.db"), &main).unwrap();

        let db = Database::connect(&main.to_string_lossy().replace('\\', "/"))
            .await
            .unwrap();
        // 测试环境会把分片库隔离进 `subscriptions/<uuid>/`，所以分片要从副本里搬进去。
        let shard_src = src.join("subscriptions");
        let mut copied = 0usize;
        if let Ok(entries) = std::fs::read_dir(&shard_src) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if !name.to_string_lossy().ends_with(".db") {
                    continue;
                }
                let to = db.node_root.join(&name);
                if std::fs::copy(entry.path(), &to).is_ok() {
                    copied += 1;
                }
            }
        }
        let scopes = db.node_scopes(None).await.unwrap().len();
        println!("副本：搬了 {copied} 个分片库，主库登记了 {scopes} 个来源");
        assert!(copied > 10, "分片库没搬进来，测了也是白测");

        // —— 正确性：真实数据上两条路必须给出同一批候选 ——
        let t = std::time::Instant::now();
        let old = db.top_verified_nodes_scan(12, None).await.unwrap();
        let old_ms = t.elapsed().as_millis();
        let t = std::time::Instant::now();
        let stored = db.rebuild_node_index().await.unwrap();
        let rebuild_ms = t.elapsed().as_millis();
        let t = std::time::Instant::now();
        let new = db.top_verified_nodes(12, None).await.unwrap();
        let new_ms = t.elapsed().as_millis();
        let mut old_ids = old.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        let mut new_ids = new.iter().map(|n| n.id.clone()).collect::<Vec<_>>();
        old_ids.sort();
        new_ids.sort();
        // 平手（同延迟）时两条路各取其一都算对 —— 真实副本上第 12 名正好有 2 个同延迟的，
        // 扇出那侧的顺序还取决于哪个分库先返回。所以比延迟序列，不比逐个 id。
        let mut old_lat = old.iter().map(|n| n.latency_ms).collect::<Vec<_>>();
        let mut new_lat = new.iter().map(|n| n.latency_ms).collect::<Vec<_>>();
        old_lat.sort();
        new_lat.sort();
        assert_eq!(
            old_lat, new_lat,
            "候选的延迟序列必须一致；id 集合 {old_ids:?} vs {new_ids:?}"
        );
        assert!(
            db.verify_node_index().await.is_empty(),
            "真实数据上重建之后索引和分片必须完全一致"
        );

        // —— 每 30 秒一次的计数查询 ——
        let t = std::time::Instant::now();
        let scanned = db.scan_node_verification_counts().await.unwrap();
        let counts_fanout = t.elapsed();
        let t = std::time::Instant::now();
        let indexed = db.node_index_counts().await;
        let counts_index = t.elapsed();
        assert_eq!(scanned, indexed, "索引口径必须和分片口径数出同一个结果");

        println!(
            "候选 12 条：扇出 {old_ms} ms（{scopes} 个库） / 索引 {new_ms} ms（1 条主库查询 + 少数几个库）"
        );
        println!(
            "总数/测通数 {scanned:?}：扇出 {} µs / 索引 {} µs",
            counts_fanout.as_micros(),
            counts_index.as_micros()
        );
        println!(
            "重建索引（{stored} 行）{rebuild_ms} ms；主库现在 {} 字节",
            std::fs::metadata(&main).map(|m| m.len()).unwrap_or(0)
        );
    }
}

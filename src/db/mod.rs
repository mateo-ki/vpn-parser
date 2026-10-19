pub(crate) mod export;
pub(crate) mod node_index;
pub(crate) mod nodes;
pub(crate) mod schema;
pub(crate) mod settings;
pub(crate) mod sources;
pub(crate) mod speedtest;
pub(crate) mod stats;
pub(crate) mod telemetry;

use crate::models::{Node, NodeSummary};
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;

pub const UNGROUPED_SOURCE_ID: &str = "__ungrouped__";

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: SqlitePool,
    pub(crate) node_root: PathBuf,
    /// 按来源缓存已打开的节点库连接池。每次新建池（含迁移检查）要上百毫秒，
    /// 全量扫描 35+ 个分源库时逐次新建是启动 scrub 和首页加载慢的主因。
    /// 克隆共享同一缓存；删除/换名库文件前必须先 take/close 对应池。
    pub(crate) node_pools:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, SqlitePool>>>,
    /// 全局节点摘要内存级预热缓存（NodeSummary Cache）。
    /// 解决跨分片无条件扫描导致筛选极慢的瓶颈，使节点池多维过滤、排序与分页达到 1ms 级极速响应。
    pub(crate) node_summaries_cache:
        std::sync::Arc<tokio::sync::RwLock<Option<std::sync::Arc<Vec<NodeSummary>>>>>,
}

impl Database {
    pub async fn connect(db_path: &str) -> Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create db dir {}", parent.display()))?;
            }
        }
        let url = format!("sqlite://{db_path}?mode=rwc");
        let options = SqliteConnectOptions::from_str(&url)?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10))
            .pragma("cache_size", "-4000")
            .pragma("temp_store", "1")
            .pragma("mmap_size", "0");
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .with_context(|| format!("connect sqlite {url}"))?;
        let base_node_root = Path::new(db_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("subscriptions");
        // Unit tests share the system temp directory and run in parallel; isolate their
        // subscription files while production keeps the documented data/subscriptions path.
        let node_root = if cfg!(test) {
            base_node_root.join(Uuid::new_v4().to_string())
        } else {
            base_node_root
        };
        std::fs::create_dir_all(&node_root)
            .with_context(|| format!("create subscription db dir {}", node_root.display()))?;
        // 自动清理订阅目录下由于异常关机或崩溃遗留的历史临时文件，释放磁盘
        if let Ok(entries) = std::fs::read_dir(&node_root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') && (name.ends_with(".tmp") || name.ends_with(".bak")) {
                        let _ = std::fs::remove_file(p);
                    }
                }
            }
        }
        let db = Self {
            pool,
            node_root,
            node_pools: std::sync::Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ),
            node_summaries_cache: std::sync::Arc::new(
                tokio::sync::RwLock::new(None),
            ),
        };
        db.migrate().await?;
        let _ = db.prune_telemetry().await;
        Ok(db)
    }

    /// 获取缓存中的全量节点摘要
    pub(crate) async fn get_cached_node_summaries(
        &self,
    ) -> Option<std::sync::Arc<Vec<NodeSummary>>> {
        let cache = self.node_summaries_cache.read().await;
        cache.clone()
    }

    /// 写入/更新全量节点摘要缓存
    pub(crate) async fn set_cached_node_summaries(
        &self,
        summaries: std::sync::Arc<Vec<NodeSummary>>,
    ) {
        let mut cache = self.node_summaries_cache.write().await;
        *cache = Some(summaries);
    }

    /// 失效节点摘要缓存（节点新增/更新/删除/测速完成时触发）
    pub async fn invalidate_node_cache(&self) {
        let mut cache = self.node_summaries_cache.write().await;
        *cache = None;
    }

    /// 节点库缓存键：`None`（未分组）用固定哨兵，避免与真实来源 id 撞车。
    pub(crate) fn node_pool_cache_key(source_id: Option<&str>) -> String {
        Self::node_db_stem(source_id)
    }

    /// 分库文件名（不含扩展名）。`source_id` 可能来自 HTTP 查询参数，因此只接受
    /// UUID/哨兵形状的字符集；带 `/`、`\`、`..` 的一律归到未分组库，杜绝
    /// `node_root.join` 跑到目录外面去建库。
    pub(crate) fn node_db_stem(source_id: Option<&str>) -> String {
        let valid = |id: &str| {
            !id.is_empty()
                && !id.starts_with('.')
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        };
        match source_id {
            Some(id) if valid(id) => id.to_owned(),
            Some(id) => {
                tracing::warn!(id, "非法 source_id，已按未分组节点库处理");
                UNGROUPED_SOURCE_ID.to_owned()
            }
            None => UNGROUPED_SOURCE_ID.to_owned(),
        }
    }

    pub(crate) fn node_db_path(&self, source_id: Option<&str>) -> PathBuf {
        self.node_root
            .join(format!("{}.db", Self::node_db_stem(source_id)))
    }

    pub(crate) async fn open_node_pool(&self, source_id: Option<&str>) -> Result<SqlitePool> {
        let key = Self::node_pool_cache_key(source_id);
        {
            let mut pools = self.node_pools.lock().unwrap();
            if let Some(pool) = pools.get(&key) {
                if !pool.is_closed() {
                    return Ok(pool.clone());
                } else {
                    pools.remove(&key);
                }
            }
        }
        let pool = Self::connect_node_pool(&self.node_db_path(source_id)).await?;
        self.node_pools.lock().unwrap().insert(key, pool.clone());
        Ok(pool)
    }

    /// 打开（不缓存）一个节点库连接池，`open_node_pool` 的底层实现。
    pub(crate) async fn connect_node_pool(path: &Path) -> Result<SqlitePool> {
        let url = format!(
            "sqlite://{}?mode=rwc",
            path.to_string_lossy().replace('\\', "/")
        );
        let options = SqliteConnectOptions::from_str(&url)?
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(10))
            .pragma("cache_size", "-2000")
            .pragma("temp_store", "1")
            .pragma("mmap_size", "0");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .with_context(|| format!("connect subscription sqlite {}", path.display()))?;
        Self::migrate_node_pool(&pool).await?;
        Ok(pool)
    }

    /// 取出缓存的节点库连接池（不关闭）。库文件即将被换名/删除前必须调用：
    /// Windows 上打开着的 SQLite 句柄会让 rename/remove_file 失败。
    pub(crate) async fn take_node_pool(
        &self,
        source_id: Option<&str>,
    ) -> Result<Option<SqlitePool>> {
        let key = Self::node_pool_cache_key(source_id);
        if let Some(pool) = self.node_pools.lock().unwrap().remove(&key) {
            return Ok(Some(pool));
        }
        let path = self.node_db_path(source_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(Self::connect_node_pool(&path).await?))
    }

    /// 关闭并移除缓存的节点库连接池。删除库文件前必须调用。
    pub(crate) async fn close_node_pool(&self, source_id: Option<&str>) {
        let key = Self::node_pool_cache_key(source_id);
        let pool = self.node_pools.lock().unwrap().remove(&key);
        if let Some(pool) = pool {
            let _ = pool.close().await;
        }
    }

    /// 整理所有缓存的节点库连接池，释放底层 SQLite 未使用的缓存页。
    /// 注意：绝对不能破坏性地对连接池调用 pool.close()，否则其他并发查询（如启动代理、
    /// 测速后刷新可用节点）会立即抛出 "attempted to acquire a connection on a closed pool"。
    pub async fn trim_cached_pools(&self) {
        let pools: Vec<SqlitePool> = {
            let pools = self.node_pools.lock().unwrap();
            pools.values().cloned().collect()
        };
        for pool in pools {
            if !pool.is_closed() {
                let _ = sqlx::query("PRAGMA shrink_memory;").execute(&pool).await;
            }
        }
        // 同时释放主库连接池中未使用的缓存页
        let _ = sqlx::query("PRAGMA shrink_memory;").execute(&self.pool).await;
    }

    pub(crate) async fn find_node_pool(&self, id: &str) -> Result<Option<SqlitePool>> {
        // 只有真的不知道节点属于哪个源时才走这里。所有分库并发查，避免 34 次串行往返。
        let mut tasks = tokio::task::JoinSet::new();
        for scope in self.node_scopes(None).await? {
            let path = self.node_db_path(scope.as_deref());
            if !path.exists() {
                continue;
            }
            let db = self.clone();
            let id = id.to_owned();
            tasks.spawn(async move {
                let Ok(pool) = db.open_node_pool(scope.as_deref()).await else {
                    return None;
                };
                let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM nodes WHERE id = ?")
                    .bind(&id)
                    .fetch_optional(&pool)
                    .await
                    .ok()?;
                exists.map(|_| pool)
            });
        }
        while let Some(found) = tasks.join_next().await {
            if let Some(pool) = found.unwrap_or(None) {
                return Ok(Some(pool));
            }
        }
        Ok(None)
    }

    /// 统一枚举并打开所有存在的分库连接池，收敛各业务遍历分片的样板代码。
    pub(crate) async fn active_node_pools(
        &self,
        source_id: Option<&str>,
    ) -> Result<Vec<(Option<String>, SqlitePool)>> {
        let scopes = self.node_scopes(source_id).await?;
        let mut pools = Vec::with_capacity(scopes.len());
        for scope in scopes {
            if !self.node_db_path(scope.as_deref()).exists() {
                continue;
            }
            let pool = self.open_node_pool(scope.as_deref()).await?;
            pools.push((scope, pool));
        }
        Ok(pools)
    }

    pub(crate) async fn node_scopes(&self, source_id: Option<&str>) -> Result<Vec<Option<String>>> {
        match source_id {
            Some(UNGROUPED_SOURCE_ID) => Ok(vec![None]),
            Some(id) => Ok(vec![Some(id.to_owned())]),
            None => {
                let mut scopes = self
                    .list_sources()
                    .await?
                    .into_iter()
                    .map(|source| Some(source.id))
                    .collect::<Vec<_>>();
                scopes.push(None);
                Ok(scopes)
            }
        }
    }
}

pub(crate) fn row_to_node(r: sqlx::sqlite::SqliteRow) -> Node {
    let config_str: String = r.get("config");
    let config = serde_json::from_str(&config_str).unwrap_or(serde_json::json!({}));
    Node {
        id: r.get("id"),
        source_id: r.get("source_id"),
        protocol: r.get("protocol"),
        name: r.get("name"),
        server: r.get("server"),
        port: r.get::<i64, _>("port") as u16,
        raw: r.get("raw"),
        config,
        country: r.get("country"),
        latency_ms: r.get("latency_ms"),
        latency_ok: r.get::<Option<i64>, _>("latency_ok").map(|v| v != 0),
        download_kbps: r.try_get("download_kbps").ok().flatten(),
        latency_kernel: r.get("latency_kernel"),
        latency_test_kind: r.get("latency_test_kind"),
        latency_tested_at: r
            .get::<Option<String>, _>("latency_tested_at")
            .map(|s| parse_dt(&s)),
        created_at: parse_dt(r.get::<String, _>("created_at").as_str()),
        updated_at: parse_dt(r.get::<String, _>("updated_at").as_str()),
    }
}

pub(crate) fn row_to_node_summary(r: sqlx::sqlite::SqliteRow) -> NodeSummary {
    NodeSummary {
        id: r.get("id"),
        source_id: r.get("source_id"),
        protocol: r.get("protocol"),
        name: r.get("name"),
        server: r.get("server"),
        port: r.get::<i64, _>("port") as u16,
        country: r.get("country"),
        latency_ms: r.get("latency_ms"),
        latency_ok: r
            .get::<Option<i64>, _>("latency_ok")
            .map(|value| value != 0),
        download_kbps: r.get("download_kbps"),
        latency_kernel: r.get("latency_kernel"),
        latency_test_kind: r.get("latency_test_kind"),
        latency_tested_at: r
            .get::<Option<String>, _>("latency_tested_at")
            .map(|value| parse_dt(&value)),
        updated_at: parse_dt(r.get::<String, _>("updated_at").as_str()),
    }
}

pub(crate) fn parse_dt(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

pub(crate) fn name_quality_score(name: &str) -> i32 {
    let n = name.trim();
    if n.is_empty() {
        return 0;
    }
    let mut score = 10;
    let lower = n.to_ascii_lowercase();
    if lower.contains("telegram")
        || lower.contains("join")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("t.me")
    {
        score -= 6;
    }
    if n.chars().count() <= 48 {
        score += 2;
    }
    if n.chars().count() > 80 {
        score -= 3;
    }
    score
}

pub(crate) fn dedupe_keep_score(n: &Node) -> i64 {
    let mut score: i64 = 0;
    match n.latency_ok {
        Some(true) => {
            score += 1_000_000;
            if let Some(ms) = n.latency_ms {
                score += (100_000 - ms.clamp(0, 99_999)) as i64;
            }
        }
        Some(false) => score += 10_000,
        None => score += 50_000,
    }
    if let Some(dl) = n.download_kbps {
        score += (dl.min(10_000.0)) as i64;
    }
    score += name_quality_score(&n.name) as i64;
    score += n.updated_at.timestamp() / 1_000_000;
    score
}


#[cfg(test)]
mod tests;

//! 遥测读模型：出口健康、切换历史、规则命中、订阅源拉取历史。
//!
//! 这四张表**必须在主库**：`data/subscriptions/*.db` 那些分片每次拉订阅都整库重建再改名
//! （`replace_source_nodes`），历史放那儿等于每天清零。
//!
//! 保留期见 [`RETENTION_*`]，启动时和每次写入路径上按配额裁剪，不另开定时器。

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Local, Timelike, Utc};
use sqlx::Row;

/// 健康探测记录保留 7 天：界面上的健康条只画 24 小时，留一周够回看"昨晚是不是又挂了"。
const RETENTION_HEALTH_DAYS: i64 = 7;
/// 切换历史留 90 天：条数很少（一天几十条），但"上周三那个点谁把出口换走的"就靠它。
pub const RETENTION_SWITCHES_DAYS: i64 = 90;
/// 规则命中按天累计，留 7 天。
const RETENTION_HITS_DAYS: i64 = 7;
/// 每个源只留最近 40 次拉取记录。
const FETCH_HISTORY_KEEP: i64 = 40;

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct HealthRow {
    pub at: String,
    pub ok: i64,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct SwitchRow {
    pub at: String,
    pub from_node: Option<String>,
    pub from_name: Option<String>,
    pub to_node: Option<String>,
    pub to_name: Option<String>,
    pub reason: String,
    pub latency_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RuleHit {
    pub rule: String,
    pub target: String,
    /// 这一批里有多少条连接。连接按 id 去重后累加，所以这里一次刷 30 秒的增量。
    pub count: i64,
    pub at: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct RuleHitRow {
    pub rule: String,
    pub target: String,
    pub hits: i64,
    pub last_at: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
#[allow(dead_code)]
pub struct FetchHistoryRow {
    pub at: String,
    pub ok: i64,
    pub node_count: i64,
    pub status: String,
}

fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// 把错误文本压到一行里放得下的长度：健康条上直接显示它。
fn clip(text: &str) -> String {
    const MAX_CHARS: usize = 160;
    let trimmed = text.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX_CHARS).collect();
    out.push('…');
    out
}

/// 一条记录落在哪个本地日历日。`at` 存的全是 UTC，直接取前 10 个字符会把
/// 北京时间早上 8 点之前的记录算到昨天头上。
pub fn switch_day(at: &str) -> String {
    DateTime::parse_from_rfc3339(at)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| at.chars().take(10).collect())
}

/// 一次出口变更。用结构体而不是八个位置参数：调用点全在 GUI 里，位置参数写错顺序
/// 编译器不会拦。
#[derive(Debug, Clone, Default)]
pub struct SwitchRecord {
    pub from_node: Option<String>,
    pub from_name: Option<String>,
    pub to_node: Option<String>,
    pub to_name: Option<String>,
    pub reason: String,
    pub latency_ms: Option<u64>,
}

impl Database {
    /// 记一次出口变更。
    pub async fn record_egress_switch(&self, record: &SwitchRecord) -> Result<()> {
        // 和上一条完全一样的变更（同一个 to、同一个原因）不重复记，
        // 否则探测抖动会把历史刷成一片复制粘贴。
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT to_node || '|' || reason FROM egress_switches ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let signature = format!(
            "{}|{}",
            record.to_node.clone().unwrap_or_default(),
            clip(&record.reason)
        );
        if previous.as_deref() == Some(signature.as_str()) {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO egress_switches \
             (at, from_node, from_name, to_node, to_name, reason, latency_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(now_rfc3339())
        .bind(&record.from_node)
        .bind(&record.from_name)
        .bind(&record.to_node)
        .bind(&record.to_name)
        .bind(clip(&record.reason))
        .bind(record.latency_ms.map(|v| v as i64))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub async fn record_egress_health(
        &self,
        ok: bool,
        latency_ms: Option<u64>,
        error: Option<&str>,
        node_id: Option<&str>,
        node_name: Option<&str>,
        kind: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO egress_health (at, ok, latency_ms, error, node_id, node_name, kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(now_rfc3339())
        .bind(ok as i64)
        .bind(latency_ms.map(|v| v as i64))
        .bind(error.map(clip))
        .bind(node_id)
        .bind(node_name)
        .bind(kind)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 最近 N 次实测，新→旧。
    #[allow(dead_code)]
    pub async fn recent_egress_health(&self, limit: i64) -> Result<Vec<HealthRow>> {
        let rows = sqlx::query_as::<_, HealthRow>(
            "SELECT at, ok, latency_ms, error, node_id, node_name, kind FROM egress_health \
             ORDER BY id DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 画健康条用：`since` 之后按时间正序的全部实测。
    pub async fn egress_health_since(&self, since: DateTime<Utc>) -> Result<Vec<HealthRow>> {
        let rows = sqlx::query_as::<_, HealthRow>(
            "SELECT at, ok, latency_ms, error, node_id, node_name, kind FROM egress_health \
             WHERE at >= ?1 ORDER BY id ASC",
        )
        .bind(since.to_rfc3339())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 当前连续失败次数（最近一段连续的 `ok=0`）。表头那句"连续 N 次不通"就报这个数。
    pub async fn egress_fail_streak(&self) -> Result<i64> {
        let rows =
            sqlx::query_scalar::<_, i64>("SELECT ok FROM egress_health ORDER BY id DESC LIMIT 200")
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().take_while(|ok| *ok == 0).count() as i64)
    }

    #[allow(dead_code)]
    pub async fn recent_egress_switches(&self, limit: i64) -> Result<Vec<SwitchRow>> {
        let rows = sqlx::query_as::<_, SwitchRow>(
            "SELECT at, from_node, from_name, to_node, to_name, reason, latency_ms \
             FROM egress_switches ORDER BY id DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 今天换了几次 —— 控制台那句"今日自动切换 N 次"。
    ///
    /// 按**本地**今天零点起算：`at` 全是 UTC，用 `substr(at,1,10)` 比本地日期会把
    /// 北京时间 00:00–08:00 之间那一段整个算到昨天头上。
    pub async fn switch_count_today(&self) -> Result<i64> {
        // 本地今天零点 = 现在减去"今天已经过了多少秒"。这样算不用构造时区化的
        // NaiveDateTime，也就没有 DST 歧义（零点切换的那几天构造不出合法时刻）。
        let now_local = Local::now();
        let elapsed = now_local.num_seconds_from_midnight() as i64;
        let start_utc =
            (now_local.with_timezone(&Utc) - chrono::Duration::seconds(elapsed)).to_rfc3339();
        let count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM egress_switches WHERE at >= ?1")
                .bind(start_utc)
                .fetch_one(&self.pool)
                .await?;
        Ok(count)
    }

    /// 库里还留着多少条切换记录（保留期之内，不只今天）。
    pub async fn switch_history_count(&self) -> Result<i64> {
        Ok(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM egress_switches")
                .fetch_one(&self.pool)
                .await?,
        )
    }

    /// 把这一秒采到的命中累加进按天聚合的表。连接是瞬时的，只能边采边加。
    /// 一整批包在一个事务里：每条 UPSERT 各占一个隐式事务的话，一次刷新几十条就是几十次
    /// WAL 提交。
    pub async fn record_rule_hits(&self, hits: &[RuleHit]) -> Result<usize> {
        if hits.is_empty() {
            return Ok(0);
        }
        let day = Local::now().format("%Y-%m-%d").to_string();
        let mut tx = self.pool.begin().await?;
        let mut written = 0usize;
        for hit in hits {
            sqlx::query(
                "INSERT INTO rule_hits (day, rule, target, hits, first_at, last_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
                 ON CONFLICT(day, rule, target) DO UPDATE SET \
                   hits = hits + excluded.hits, last_at = excluded.last_at",
            )
            .bind(&day)
            .bind(clip(&hit.rule))
            .bind(clip(&hit.target))
            .bind(hit.count.max(1))
            .bind(&hit.at)
            .execute(&mut *tx)
            .await?;
            written += 1;
        }
        tx.commit().await?;
        Ok(written)
    }

    /// 今天每条规则的命中数，按命中从多到少。
    pub async fn rule_hits_today(&self) -> Result<Vec<RuleHitRow>> {
        let day = Local::now().format("%Y-%m-%d").to_string();
        let rows = sqlx::query_as::<_, RuleHitRow>(
            "SELECT rule, target, hits, last_at FROM rule_hits WHERE day = ?1 \
             ORDER BY hits DESC, rule ASC",
        )
        .bind(day)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 记一次拉取结果。`status` 直接沿用写进 `sources.last_status` 的那个词，
    /// 这样历史条和当前状态列能互相对上。
    pub async fn record_fetch_history(
        &self,
        source_id: &str,
        ok: bool,
        node_count: i64,
        status: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO source_fetch_history (source_id, at, ok, node_count, status) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(source_id)
        .bind(now_rfc3339())
        .bind(ok as i64)
        .bind(node_count)
        .bind(clip(status))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 某个源最近 N 次拉取，新→旧（界面画"最近 7 次"色条）。
    #[allow(dead_code)]
    pub async fn fetch_history_for(
        &self,
        source_id: &str,
        limit: i64,
    ) -> Result<Vec<FetchHistoryRow>> {
        let rows = sqlx::query_as::<_, FetchHistoryRow>(
            "SELECT at, ok, node_count, status FROM source_fetch_history \
             WHERE source_id = ?1 ORDER BY id DESC LIMIT ?2",
        )
        .bind(source_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// 一次性取所有源的最近 200 条拉取历史，调用方自己按 source_id 分桶 ——
    /// 订阅源页一次要渲染三十几个源，逐源查就是三十几次往返。
    pub async fn fetch_history_all(&self, limit: i64) -> Result<Vec<(String, FetchHistoryRow)>> {
        let rows = sqlx::query(
            "SELECT source_id, at, ok, node_count, status FROM source_fetch_history \
             ORDER BY id DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.try_get::<String, _>("source_id").unwrap_or_default(),
                    FetchHistoryRow {
                        at: row.try_get::<String, _>("at").unwrap_or_default(),
                        ok: row.try_get::<i64, _>("ok").unwrap_or(0),
                        node_count: row.try_get::<i64, _>("node_count").unwrap_or(0),
                        status: row.try_get::<String, _>("status").unwrap_or_default(),
                    },
                )
            })
            .collect())
    }

    /// 裁剪到保留期内。启动时跑一次，另外每写 200 条健康记录顺手跑一次。
    pub async fn prune_telemetry(&self) -> Result<()> {
        let health_cut = (Utc::now() - chrono::Duration::days(RETENTION_HEALTH_DAYS)).to_rfc3339();
        let _ = sqlx::query(
            "INSERT INTO egress_health_daily (day, total_checks, ok_count, fail_count, avg_latency_ms) \
             SELECT substr(at, 1, 10) as d, \
                    COUNT(*), \
                    SUM(CASE WHEN ok != 0 THEN 1 ELSE 0 END), \
                    SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END), \
                    AVG(latency_ms) \
             FROM egress_health \
             WHERE at < ?1 \
             GROUP BY substr(at, 1, 10) \
             ON CONFLICT(day) DO UPDATE SET \
                total_checks = total_checks + excluded.total_checks, \
                ok_count = ok_count + excluded.ok_count, \
                fail_count = fail_count + excluded.fail_count, \
                avg_latency_ms = COALESCE(excluded.avg_latency_ms, avg_latency_ms)",
        )
        .bind(&health_cut)
        .execute(&self.pool)
        .await;
        let switch_cut =
            (Utc::now() - chrono::Duration::days(RETENTION_SWITCHES_DAYS)).to_rfc3339();
        let hits_cut = (Utc::now() - chrono::Duration::days(RETENTION_HITS_DAYS))
            .with_timezone(&Local)
            .format("%Y-%m-%d")
            .to_string();
        sqlx::query("DELETE FROM egress_health WHERE at < ?1")
            .bind(health_cut)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM egress_switches WHERE at < ?1")
            .bind(switch_cut)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM rule_hits WHERE day < ?1")
            .bind(hits_cut)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM source_fetch_history WHERE id NOT IN ( \
                SELECT id FROM ( \
                    SELECT id, ROW_NUMBER() OVER (PARTITION BY source_id ORDER BY id DESC) AS rn \
                    FROM source_fetch_history \
                ) WHERE rn <= ?1)",
        )
        .bind(FETCH_HISTORY_KEEP)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateSource;
    use uuid::Uuid;

    /// 自动切换判定要切、但一个替补都没找到 —— 这种"没切成"以前只写在状态栏，
    /// 滑过去就查无实据。现在进切换历史，并且连续同样的失败不许刷成一片。
    #[tokio::test]
    async fn a_failover_without_a_target_is_recorded_once_not_spammed() {
        let (_path, db) = temp_db("switch-miss").await;
        let miss = |reason: &str| SwitchRecord {
            from_node: Some("n-1".into()),
            from_name: Some("唯一节点".into()),
            to_node: Some(String::new()),
            to_name: None,
            reason: reason.to_string(),
            latency_ms: None,
        };
        db.record_egress_switch(&miss("没有可用替补：TCP 复检过 8 个候选"))
            .await
            .unwrap();
        db.record_egress_switch(&miss("没有可用替补：TCP 复检过 8 个候选"))
            .await
            .unwrap();
        let rows = db.recent_egress_switches(10).await.unwrap();
        assert_eq!(rows.len(), 1, "同样的复检失败不该每两分钟刷一条");
        assert!(
            rows[0].to_node.as_deref() == Some(""),
            "空 to_node 表示这压根没切"
        );

        db.record_egress_switch(&miss("没有可用替补：库里也找不出第二个测通过的节点"))
            .await
            .unwrap();
        assert_eq!(
            db.recent_egress_switches(10).await.unwrap().len(),
            2,
            "原因变了就该各留一条，否则看不出前后差别"
        );
    }

    async fn temp_db(tag: &str) -> (std::path::PathBuf, Database) {
        let path = std::env::temp_dir().join(format!("vpn-parser-{tag}-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        (path, db)
    }

    fn cleanup(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[tokio::test]
    async fn health_records_read_back_newest_first_and_count_the_streak() {
        let (path, db) = temp_db("health").await;
        db.record_egress_health(true, Some(900), None, Some("n1"), Some("节点一"), "startup")
            .await
            .unwrap();
        db.record_egress_health(
            false,
            None,
            Some("连接超时"),
            Some("n2"),
            Some("节点二"),
            "periodic",
        )
        .await
        .unwrap();
        db.record_egress_health(
            false,
            Some(8000),
            Some("TLS 握手失败"),
            Some("n2"),
            None,
            "periodic",
        )
        .await
        .unwrap();

        let recent = db.recent_egress_health(10).await.unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].kind, "periodic");
        assert_eq!(recent[0].error.as_deref(), Some("TLS 握手失败"));
        assert_eq!(recent[2].node_name.as_deref(), Some("节点一"));

        // 连续失败从最近一条往回数，遇到一次成功就停。
        assert_eq!(db.egress_fail_streak().await.unwrap(), 2);
        db.record_egress_health(true, Some(300), None, None, None, "manual")
            .await
            .unwrap();
        assert_eq!(db.egress_fail_streak().await.unwrap(), 0);

        // 时间窗过滤：一年前到现在，四条都在；一年后起算，一条都没有。
        let wide = db
            .egress_health_since(Utc::now() - chrono::Duration::days(365))
            .await
            .unwrap();
        assert_eq!(wide.len(), 4);
        let none = db
            .egress_health_since(Utc::now() + chrono::Duration::days(1))
            .await
            .unwrap();
        assert!(none.is_empty());
        drop(db);
        cleanup(&path);
    }

    #[tokio::test]
    async fn long_error_text_is_clipped_so_the_header_can_show_it() {
        let (path, db) = temp_db("clip").await;
        let noisy = "x".repeat(400);
        db.record_egress_health(false, None, Some(&noisy), None, None, "periodic")
            .await
            .unwrap();
        let row = db.recent_egress_health(1).await.unwrap().pop().unwrap();
        let error = row.error.unwrap();
        assert_eq!(error.chars().count(), 161, "160 字 + 省略号：{error}");
        assert!(error.ends_with('…'));
        drop(db);
        cleanup(&path);
    }

    #[tokio::test]
    async fn switches_are_recorded_but_identical_consecutive_ones_collapse() {
        let (path, db) = temp_db("switch").await;
        let record = |from: Option<&str>,
                      from_n: Option<&str>,
                      to: Option<&str>,
                      to_n: Option<&str>,
                      reason: &str| SwitchRecord {
            from_node: from.map(str::to_string),
            from_name: from_n.map(str::to_string),
            to_node: to.map(str::to_string),
            to_name: to_n.map(str::to_string),
            reason: reason.to_string(),
            latency_ms: None,
        };
        db.record_egress_switch(&record(
            Some("a"),
            Some("甲"),
            Some("b"),
            Some("乙"),
            "探测超时",
        ))
        .await
        .unwrap();
        // 探测抖动：同一个目标、同一个原因，不该刷第二行。
        db.record_egress_switch(&record(
            Some("a"),
            Some("甲"),
            Some("b"),
            Some("乙"),
            "探测超时",
        ))
        .await
        .unwrap();
        db.record_egress_switch(&record(
            Some("b"),
            Some("乙"),
            Some("c"),
            Some("丙"),
            "用户手动切换",
        ))
        .await
        .unwrap();

        let rows = db.recent_egress_switches(10).await.unwrap();
        assert_eq!(rows.len(), 2, "重复的那条应该被合掉：{rows:?}");
        assert_eq!(rows[0].to_name.as_deref(), Some("丙"));
        assert_eq!(rows[1].reason, "探测超时");
        assert_eq!(db.switch_count_today().await.unwrap(), 2);
        drop(db);
        cleanup(&path);
    }

    /// 直接写一条指定时刻的记录：跨本地日和保留期边界这两种用例必须控制 `at`，
    /// 不能全指望 `now_rfc3339()`。
    async fn record_at(db: &Database, at: &str, from: &str, to: &str) {
        sqlx::query(
            "INSERT INTO egress_switches \
             (at, from_node, from_name, to_node, to_name, reason) \
             VALUES (?1, ?2, ?2, ?3, ?3, ?4)",
        )
        .bind(at)
        .bind(from)
        .bind(to)
        .bind(format!("{to} 接管"))
        .execute(&db.pool)
        .await
        .unwrap();
    }

    /// 切换记录是真的留在库里、并按**本地**日算今天：`at` 存 UTC，东八区凌晨那
    /// 八小时的记录以前会被 `substr(at,1,10)` 判成昨天，表头于是谎报"今日 0 次"。
    #[tokio::test]
    async fn switch_history_spans_local_days_and_survives_pruning() {
        let (path, db) = temp_db("switch-days").await;
        let now_local = Local::now();
        let elapsed = now_local.num_seconds_from_midnight() as i64;
        let midnight = now_local.with_timezone(&Utc) - chrono::Duration::seconds(elapsed);
        let just_after = (midnight + chrono::Duration::seconds(1)).to_rfc3339();
        let just_before = (midnight - chrono::Duration::seconds(1)).to_rfc3339();
        record_at(&db, &just_after, "甲", "乙").await;
        record_at(&db, &just_before, "乙", "丙").await;
        record_at(
            &db,
            &(midnight - chrono::Duration::days(3)).to_rfc3339(),
            "丙",
            "丁",
        )
        .await;

        assert_eq!(
            db.switch_count_today().await.unwrap(),
            1,
            "只有本地零点之后的那一条算今天"
        );
        assert_eq!(
            db.switch_history_count().await.unwrap(),
            3,
            "表头那句「共 N 条」说的是保留期内的全部，不只今天"
        );
        assert_ne!(
            switch_day(&just_after),
            switch_day(&just_before),
            "跨过本地零点的两条必须分到两天"
        );

        record_at(
            &db,
            &(Utc::now() - chrono::Duration::days(RETENTION_SWITCHES_DAYS + 1)).to_rfc3339(),
            "丁",
            "戊",
        )
        .await;
        db.prune_telemetry().await.unwrap();
        assert_eq!(
            db.switch_history_count().await.unwrap(),
            3,
            "超出保留期的那条才该被裁掉"
        );
        drop(db);
        cleanup(&path);
    }

    #[tokio::test]
    async fn rule_hits_accumulate_per_day() {
        let (path, db) = temp_db("hits").await;
        let at = now_rfc3339();
        let hit = |rule: &str, target: &str, count: i64| RuleHit {
            rule: rule.into(),
            target: target.into(),
            count,
            at: at.clone(),
        };
        db.record_rule_hits(&[
            hit("GEOIP,CN,DIRECT", "DIRECT", 1),
            hit("MATCH,PROXY", "PROXY", 1),
        ])
        .await
        .unwrap();
        db.record_rule_hits(&[hit("GEOIP,CN,DIRECT", "DIRECT", 3)])
            .await
            .unwrap();
        // 昨天的账不能混进今天。
        sqlx::query("INSERT INTO rule_hits (day, rule, target, hits) VALUES ('2000-01-01', 'OLD,X', 'X', 99)")
            .execute(&db.pool)
            .await
            .unwrap();

        let today = db.rule_hits_today().await.unwrap();
        assert_eq!(today.len(), 2, "只该有今天的两条：{today:?}");
        assert_eq!(today[0].rule, "GEOIP,CN,DIRECT");
        // 一次 +1、再一次 +3：同一天同一条规则必须累加成 4，而不是被覆盖成 3。
        assert_eq!(today[0].hits, 4);
        assert_eq!(today[1].target, "PROXY");
        assert_eq!(db.record_rule_hits(&[]).await.unwrap(), 0);
        drop(db);
        cleanup(&path);
    }

    #[tokio::test]
    async fn fetch_history_is_per_source_and_pruned_to_the_newest_slice() {
        let (path, db) = temp_db("fetchhist").await;
        db.create_source(CreateSource {
            name: "s1".into(),
            url_template: "https://example.invalid/sub".into(),
            enabled: true,
            remark: None,
        })
        .await
        .unwrap();
        let source_id = db
            .list_sources()
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .id;
        for index in 0..(FETCH_HISTORY_KEEP + 5) {
            db.record_fetch_history(&source_id, index % 2 == 0, index, "ok")
                .await
                .unwrap();
        }
        db.record_fetch_history("other", false, 0, "连接超时")
            .await
            .unwrap();
        db.prune_telemetry().await.unwrap();

        let kept = db.fetch_history_for(&source_id, 100).await.unwrap();
        assert_eq!(
            kept.len() as i64,
            FETCH_HISTORY_KEEP,
            "每个源只留最近 40 条"
        );
        assert_eq!(kept[0].node_count, FETCH_HISTORY_KEEP + 4, "新的在前");
        // 别的源不受这个源的裁剪影响。
        let other = db.fetch_history_for("other", 100).await.unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].status, "连接超时");

        let all = db.fetch_history_all(1000).await.unwrap();
        assert_eq!(all.len(), FETCH_HISTORY_KEEP as usize + 1);
        assert!(all.iter().all(|(id, _)| id == &source_id || id == "other"));
        drop(db);
        cleanup(&path);
    }
}

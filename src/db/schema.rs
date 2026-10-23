use super::Database;
use anyhow::Result;
use sqlx::SqlitePool;

impl Database {
    pub(crate) async fn migrate_node_pool(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY NOT NULL,
                source_id TEXT,
                protocol TEXT NOT NULL,
                name TEXT NOT NULL,
                server TEXT NOT NULL,
                port INTEGER NOT NULL,
                raw TEXT NOT NULL DEFAULT '',
                config TEXT NOT NULL DEFAULT '{}',
                country TEXT,
                latency_ms INTEGER,
                latency_ok INTEGER,
                download_kbps REAL,
                latency_kernel TEXT,
                latency_test_kind TEXT,
                latency_tested_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(protocol, server, port, name)
            )"#,
        )
        .execute(pool)
        .await?;

        // 纵向分表：Payload 分离（存储大字段 raw 与 config）
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS node_payloads (
                node_id TEXT PRIMARY KEY NOT NULL,
                raw TEXT NOT NULL,
                config TEXT NOT NULL
            )"#,
        )
        .execute(pool)
        .await?;

        // 迁移既有节点数据库中的载荷至 node_payloads
        let _ = sqlx::query(
            r#"INSERT OR IGNORE INTO node_payloads (node_id, raw, config)
               SELECT id, raw, config FROM nodes
               WHERE (raw != '' AND raw IS NOT NULL) OR (config != '' AND config IS NOT NULL)"#,
        )
        .execute(pool)
        .await;

        // 清理 nodes 表中的冗余大字段以缩小叶子页面尺寸
        let _ = sqlx::query(
            r#"UPDATE nodes SET raw = '', config = '{}'
               WHERE (raw != '' OR config != '{}') AND id IN (SELECT node_id FROM node_payloads)"#,
        )
        .execute(pool)
        .await;

        for idx in [
            "CREATE INDEX IF NOT EXISTS idx_nodes_protocol ON nodes(protocol)",
            "CREATE INDEX IF NOT EXISTS idx_nodes_latency ON nodes(latency_ms)",
            "CREATE INDEX IF NOT EXISTS idx_nodes_latency_ok ON nodes(latency_ok)",
            "CREATE INDEX IF NOT EXISTS idx_nodes_test_kind ON nodes(latency_test_kind)",
            "CREATE INDEX IF NOT EXISTS idx_nodes_updated_at ON nodes(updated_at)",
            "CREATE INDEX IF NOT EXISTS idx_nodes_country ON nodes(country)",
            "CREATE INDEX IF NOT EXISTS idx_nodes_server_port ON nodes(server, port)",
            "CREATE INDEX IF NOT EXISTS idx_node_payloads_id ON node_payloads(node_id)",
        ] {
            sqlx::query(idx).execute(pool).await?;
        }
        Ok(())
    }

    pub(crate) async fn migrate(&self) -> Result<()> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS sources (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                url_template TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                remark TEXT,
                last_fetch_at TEXT,
                last_status TEXT,
                last_node_count INTEGER NOT NULL DEFAULT 0,
                last_resolved_url TEXT,
                last_etag TEXT,
                last_modified TEXT,
                content_sha256 TEXT,
                fail_streak INTEGER NOT NULL DEFAULT 0,
                last_fetch_host TEXT,
                push_disabled INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )"#,
        )
        .execute(&self.pool)
        .await?;

        Self::migrate_node_pool(&self.pool).await?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                auto_fetch_enabled INTEGER NOT NULL DEFAULT 0,
                auto_fetch_interval_min INTEGER NOT NULL DEFAULT 60,
                auto_speedtest_enabled INTEGER NOT NULL DEFAULT 0,
                fetch_concurrency INTEGER NOT NULL DEFAULT 4,
                fetch_timeout_secs INTEGER NOT NULL DEFAULT 30,
                speedtest_concurrency INTEGER NOT NULL DEFAULT 128,
                speedtest_timeout_ms INTEGER NOT NULL DEFAULT 1500,
                speedtest_download INTEGER NOT NULL DEFAULT 0,
                download_test_ms INTEGER NOT NULL DEFAULT 800,
                http_server_enabled INTEGER NOT NULL DEFAULT 0,
                close_to_tray INTEGER NOT NULL DEFAULT 1,
                last_auto_fetch_at TEXT,
                last_auto_speedtest_at TEXT
            )"#,
        )
        .execute(&self.pool)
        .await?;

        let _ = sqlx::query(
            "ALTER TABLE settings ADD COLUMN http_server_enabled INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&self.pool)
        .await;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS proxy_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                config_json TEXT NOT NULL DEFAULT ''
            )"#,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("INSERT OR IGNORE INTO proxy_settings (id, config_json) VALUES (1, '')")
            .execute(&self.pool)
            .await?;

        // Gist 发现的去重台账：独立于 sources 存在，用户删掉的 gist 源不会被重新加回。
        self.migrate_seen_gists().await?;
        // 健康时间线 / 切换历史 / 规则命中 / 拉取历史 —— 一律在主库：
        // `data/subscriptions/*.db` 那些分片每次拉订阅都整库重建并改名，放那儿等于每天丢数据。
        self.migrate_telemetry().await?;
        // 跨分片节点汇总表（派生表，只建空表，内容全部由读侧重建）。
        self.migrate_node_index().await?;

        for idx in [
            "CREATE INDEX IF NOT EXISTS idx_sources_enabled ON sources(enabled)",
            "CREATE INDEX IF NOT EXISTS idx_seen_gists_time ON seen_gists(first_seen_at)",
        ] {
            sqlx::query(idx).execute(&self.pool).await?;
        }

        sqlx::query("INSERT OR IGNORE INTO settings (id) VALUES (1)")
            .execute(&self.pool)
            .await?;

        self.ensure_column("sources", "last_fetch_host", "TEXT DEFAULT NULL")
            .await?;
        self.ensure_column("sources", "push_disabled", "INTEGER NOT NULL DEFAULT 0")
            .await?;

        self.ensure_column("sources", "fail_streak", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column("sources", "last_resolved_url", "TEXT")
            .await?;
        self.ensure_column("sources", "last_etag", "TEXT").await?;
        self.ensure_column("sources", "last_modified", "TEXT")
            .await?;
        self.ensure_column("sources", "content_sha256", "TEXT")
            .await?;
        self.ensure_column("nodes", "latency_kernel", "TEXT")
            .await?;
        self.ensure_column("nodes", "latency_test_kind", "TEXT")
            .await?;
        self.ensure_column("nodes", "download_kbps", "REAL").await?;
        self.ensure_column("settings", "exclude_keywords", "TEXT NOT NULL DEFAULT ''")
            .await?;
        // 发布服务器地址与推送 token：以前只写 eframe storage，而 eframe 的
        // `persistence` feature 是关掉的，所以从来没落过盘。
        self.ensure_column("settings", "publisher_api", "TEXT NOT NULL DEFAULT ''")
            .await?;
        self.ensure_column("settings", "publisher_token", "TEXT NOT NULL DEFAULT ''")
            .await?;
        self.ensure_column("settings", "ui_theme", "TEXT NOT NULL DEFAULT 'obsidian'")
            .await?;
        self.ensure_column("settings", "app_icon_style", "TEXT NOT NULL DEFAULT 'classic'")
            .await?;
        self.ensure_column("settings", "ui_language", "TEXT NOT NULL DEFAULT 'zh-CN'")
            .await?;
        // 字体/界面大小（1.0 = 不缩放）。老库升级上来的这一列就是 1.0，行为不变。
        self.ensure_column("settings", "ui_scale", "REAL NOT NULL DEFAULT 1.0")
            .await?;
        // 夜间模式开关：和 ui_theme（强调色）正交。默认开（用户 2026-10-15 指定），
        // 所以老库补这一列时存量行也会一起变成夜间 —— 这是他要的效果，不是漏掉的条件。
        self.ensure_column("settings", "ui_dark", "INTEGER NOT NULL DEFAULT 1")
            .await?;
        // 菜单栏展开态（默认收起 = 和老版本的图标条一模一样）。
        self.ensure_column(
            "settings",
            "sidebar_expanded",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column(
            "settings",
            "kernel_source",
            "TEXT NOT NULL DEFAULT 'ghfast'",
        )
        .await?;
        self.ensure_column(
            "settings",
            "firewall_auto_allow",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column(
            "settings",
            "failover_latency_ms",
            "INTEGER NOT NULL DEFAULT 1500",
        )
        .await?;
        self.ensure_column("settings", "auto_failover", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        // 参与预发（检查更新也看 beta）：默认关，谁开谁承担 beta 改数据的风险。
        self.ensure_column("settings", "insider_channel", "INTEGER NOT NULL DEFAULT 0")
            .await?;
        self.ensure_column(
            "settings",
            "speedtest_auto_refill",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        // 系统代理被外部改掉时自动写回：默认关，因为它动的是用户 HKCU。
        self.ensure_column(
            "settings",
            "auto_rearm_system_proxy",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        // 这两条是"软件自己动"的开关，默认开（新装机器要能自己把内核下下来、
        // 加完订阅源要能自己测），但都能在系统设置里关掉。
        for column in ["auto_prepare_kernel", "speedtest_added_source"] {
            self.ensure_column("settings", column, "INTEGER NOT NULL DEFAULT 1")
                .await?;
        }
        // 五个测速优化开关默认全部关闭，行为与老版本一致。
        for column in [
            "speedtest_parallel_kernels",
            "speedtest_persistent_kernel",
            "speedtest_reuse_minutes",
            "speedtest_conditional_isolation",
            "speedtest_download_top_n",
            "speedtest_dedup_egress",
            "speedtest_tcp_ping_only",
            "bandwidth_guard",
            "bandwidth_min_kbps",
        ] {
            self.ensure_column("settings", column, "INTEGER NOT NULL DEFAULT 0")
                .await?;
        }
        self.ensure_column(
            "settings",
            "fetch_concurrency",
            "INTEGER NOT NULL DEFAULT 4",
        )
        .await?;
        self.ensure_column(
            "settings",
            "fetch_timeout_secs",
            "INTEGER NOT NULL DEFAULT 30",
        )
        .await?;
        self.ensure_column(
            "settings",
            "speedtest_download",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column(
            "settings",
            "download_test_ms",
            "INTEGER NOT NULL DEFAULT 800",
        )
        .await?;
        self.ensure_column(
            "settings",
            "http_server_enabled",
            "INTEGER NOT NULL DEFAULT 0",
        )
        .await?;
        self.ensure_column("settings", "close_to_tray", "INTEGER NOT NULL DEFAULT 1")
            .await?;
        self.ensure_column("settings", "gist_pages", "INTEGER NOT NULL DEFAULT 100")
            .await?;
        self.ensure_column("settings", "gist_queries", "TEXT NOT NULL DEFAULT 'vless//'")
            .await?;
        self.ensure_column("settings", "window_size", "TEXT NOT NULL DEFAULT 'small'")
            .await?;
        self.ensure_column("settings", "console_tabs", "TEXT NOT NULL DEFAULT 'all'")
            .await?;

        self.migrate_legacy_nodes().await?;

        Ok(())
    }

    async fn ensure_column(&self, table: &str, column: &str, decl: &str) -> Result<()> {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {decl}");
        let _ = sqlx::query(&sql).execute(&self.pool).await;
        Ok(())
    }

    /// 兼容旧版本：若主库 nodes 仍有数据，将其按 source_id 迁入分库并清空主库 nodes。
    async fn migrate_legacy_nodes(&self) -> Result<()> {
        let legacy_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes")
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0);
        if legacy_count == 0 {
            return Ok(());
        }
        tracing::info!(
            count = legacy_count,
            "migrating legacy nodes into scoped pools"
        );
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
        .fetch_all(&self.pool)
        .await?;

        let mut groups: std::collections::HashMap<Option<String>, Vec<crate::models::Node>> =
            std::collections::HashMap::new();
        for row in rows {
            let node = super::row_to_node(row);
            groups.entry(node.source_id.clone()).or_default().push(node);
        }

        for (source_id, nodes) in groups {
            let pool = self.open_node_pool(source_id.as_deref()).await?;
            let mut tx = pool.begin().await?;
            for node in &nodes {
                Self::insert_node(&mut tx, node).await?;
            }
            tx.commit().await?;
        }

        sqlx::query("DELETE FROM nodes").execute(&self.pool).await?;
        let _ = sqlx::query("DELETE FROM node_payloads")
            .execute(&self.pool)
            .await;
        Ok(())
    }

    /// 跨分片节点汇总表。它是**派生表**：真源永远是 `data/subscriptions/*.db` 里的
    /// 分片库，这里只存选路/统计要的元数据（不含 raw/config 凭据），任何时刻都可以
    /// 整表重建，所以迁移只保证表在，不搬数据。详见 `db/node_index.rs`。
    async fn migrate_node_index(&self) -> Result<()> {
        for ddl in [
            "CREATE TABLE IF NOT EXISTS node_index(
                id TEXT PRIMARY KEY NOT NULL,
                source_id TEXT,
                protocol TEXT NOT NULL DEFAULT '',
                latency_ms INTEGER,
                latency_ok INTEGER NOT NULL DEFAULT 0
            )",
            "CREATE TABLE IF NOT EXISTS node_index_meta(
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL DEFAULT ''
            )",
            "CREATE INDEX IF NOT EXISTS node_index_verified ON node_index(latency_ok, latency_ms)",
            "CREATE INDEX IF NOT EXISTS node_index_source ON node_index(source_id)",
        ] {
            sqlx::query(ddl).execute(&self.pool).await?;
        }
        Ok(())
    }

    async fn migrate_seen_gists(&self) -> Result<()> {
        let _ = sqlx::query("DROP TABLE IF EXISTS seen_gists_new")
            .execute(&self.pool)
            .await;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS seen_gists (
                gist_id TEXT PRIMARY KEY NOT NULL,
                source_id TEXT,
                first_seen_at TEXT NOT NULL
            )"#,
        )
        .execute(&self.pool)
        .await?;

        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('seen_gists')")
                .fetch_all(&self.pool)
                .await
                .unwrap_or_default();

        if !columns.contains(&"first_seen_at".to_string()) {
            let _ = sqlx::query(
                "ALTER TABLE seen_gists ADD COLUMN first_seen_at TEXT NOT NULL DEFAULT ''",
            )
            .execute(&self.pool)
            .await;
            let _ = sqlx::query("UPDATE seen_gists SET first_seen_at = COALESCE(created_at, '') WHERE first_seen_at = ''").execute(&self.pool).await;
        }
        if !columns.contains(&"gist_id".to_string()) {
            let _ =
                sqlx::query("ALTER TABLE seen_gists ADD COLUMN gist_id TEXT NOT NULL DEFAULT ''")
                    .execute(&self.pool)
                    .await;
            let _ =
                sqlx::query("UPDATE seen_gists SET gist_id = COALESCE(id, '') WHERE gist_id = ''")
                    .execute(&self.pool)
                    .await;
        }
        if !columns.contains(&"source_id".to_string()) {
            let _ = sqlx::query("ALTER TABLE seen_gists ADD COLUMN source_id TEXT")
                .execute(&self.pool)
                .await;
        }
        Ok(())
    }

    /// 四张"说实话"用的表：出口健康、切换历史、规则命中、拉取历史。
    async fn migrate_telemetry(&self) -> Result<()> {
        for ddl in [
            // 每一次连通性实测都落一行。表头那句「Google: 已连通」以前只看 state，
            // 内核日志里全是 dial i/o timeout 它照样画绿勾（2026-10-06 实测）。
            r#"CREATE TABLE IF NOT EXISTS egress_health_daily (
                day TEXT PRIMARY KEY NOT NULL,
                total_checks INTEGER NOT NULL DEFAULT 0,
                ok_count INTEGER NOT NULL DEFAULT 0,
                fail_count INTEGER NOT NULL DEFAULT 0,
                avg_latency_ms REAL,
                max_latency_ms INTEGER
            );

            CREATE TABLE IF NOT EXISTS egress_health (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                at TEXT NOT NULL,
                ok INTEGER NOT NULL,
                latency_ms INTEGER,
                error TEXT,
                node_id TEXT,
                node_name TEXT,
                kind TEXT NOT NULL
            )"#,
            "CREATE INDEX IF NOT EXISTS idx_egress_health_at ON egress_health(at)",
            // 谁在什么时候把出口从哪个节点换到哪个节点、为什么换、当时延迟多少。
            r#"CREATE TABLE IF NOT EXISTS egress_switches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                at TEXT NOT NULL,
                from_node TEXT,
                from_name TEXT,
                to_node TEXT,
                to_name TEXT,
                reason TEXT NOT NULL,
                latency_ms INTEGER,
                pinned INTEGER NOT NULL DEFAULT 0
            )"#,
            "CREATE INDEX IF NOT EXISTS idx_egress_switches_at ON egress_switches(at)",
            // 规则命中按天累计：连接是瞬时的，/connections 每 1s 采一次，边采边加。
            r#"CREATE TABLE IF NOT EXISTS rule_hits (
                day TEXT NOT NULL,
                rule TEXT NOT NULL,
                target TEXT NOT NULL,
                hits INTEGER NOT NULL DEFAULT 0,
                first_at TEXT,
                last_at TEXT,
                PRIMARY KEY (day, rule, target)
            )"#,
            r#"CREATE TABLE IF NOT EXISTS source_fetch_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source_id TEXT NOT NULL,
                at TEXT NOT NULL,
                ok INTEGER NOT NULL,
                node_count INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL
            )"#,
            "CREATE INDEX IF NOT EXISTS idx_fetch_history_source ON source_fetch_history(source_id, at)",
        ] {
            sqlx::query(ddl).execute(&self.pool).await?;
        }
        Ok(())
    }
}

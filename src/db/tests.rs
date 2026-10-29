    use super::*;
    use crate::models::{CreateSource, Node, UpdateSettings};
    use crate::proxy::{ProxyMode, RoutingMode, RuleAction, RuleMatcher, TrafficRule};

    #[tokio::test]
    async fn persists_complete_proxy_config() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-proxy-config-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let mut config = crate::proxy::MihomoConfigOptions::default();
        config.mode = ProxyMode::Tun;
        config.routing_mode = RoutingMode::ChinaDirect;
        config.rules.push(TrafficRule {
            matcher: RuleMatcher::GeoSite("cn".into()),
            action: RuleAction::Direct,
        });

        db.update_proxy_config(&config).await.unwrap();

        assert_eq!(db.get_proxy_config().await.unwrap(), config);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn persists_custom_mixed_port_including_7890() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-proxy-port-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let mut config = crate::proxy::MihomoConfigOptions::default();
        config.mixed_port = 7890;

        db.update_proxy_config(&config).await.unwrap();

        let loaded = db.get_proxy_config().await.unwrap();
        assert_eq!(loaded.mixed_port, 7890);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn reads_settings_while_exclusive_writer_is_active() {
        let path = std::env::temp_dir().join(format!("vpn-parser-wal-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let mut writer = db.pool.acquire().await.unwrap();
        sqlx::query("BEGIN EXCLUSIVE")
            .execute(&mut *writer)
            .await
            .unwrap();
        sqlx::query("UPDATE settings SET auto_fetch_enabled = auto_fetch_enabled WHERE id = 1")
            .execute(&mut *writer)
            .await
            .unwrap();

        let settings = tokio::time::timeout(Duration::from_secs(1), db.get_settings())
            .await
            .expect("WAL read should not wait for writer")
            .unwrap();
        assert!(settings.speedtest_concurrency > 0);

        sqlx::query("ROLLBACK").execute(&mut *writer).await.unwrap();
        drop(writer);
        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn persists_fetch_and_speedtest_concurrency_after_reconnect() {
        let path = std::env::temp_dir().join(format!(
            "vpn-parser-concurrency-settings-{}.db",
            Uuid::new_v4()
        ));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();

        let saved = db
            .update_settings(UpdateSettings {
                sidebar_expanded: None,
                auto_fetch_enabled: None,
                auto_fetch_interval_min: None,
                auto_speedtest_enabled: None,
                fetch_concurrency: Some(37),
                fetch_timeout_secs: None,
                speedtest_concurrency: Some(173),
                speedtest_timeout_ms: None,
                speedtest_download: None,
                download_test_ms: None,
                http_server_enabled: None,
                close_to_tray: None,
                exclude_keywords: None,
                failover_latency_ms: None,
                speedtest_parallel_kernels: Some(true),
                speedtest_persistent_kernel: Some(true),
                speedtest_reuse_minutes: Some(45),
                speedtest_conditional_isolation: Some(true),
                speedtest_download_top_n: Some(60),
                speedtest_dedup_egress: Some(true),
                speedtest_tcp_ping_only: Some(false),
                bandwidth_guard: Some(true),
                bandwidth_min_kbps: Some(256),
                publisher_api: None,
                publisher_token: None,
                ui_theme: None,
                app_icon_style: None,
                ui_language: None,
                ui_scale: None,
                ui_dark: None,
                kernel_source: None,
                firewall_auto_allow: None,
                auto_failover: None,
                insider_channel: None,
                speedtest_auto_refill: None,
                auto_rearm_system_proxy: None,
                auto_prepare_kernel: None,
                speedtest_added_source: None,
                launch_as_admin: None,
                launch_at_startup: None,
                gist_pages: None,
                gist_queries: None,
                window_size: None,
                console_tabs: None,
                dashboard_ui_mode: None,
            })
            .await
            .unwrap();
        assert_eq!(saved.fetch_concurrency, 8);
        assert_eq!(saved.speedtest_concurrency, 173);
        assert!(saved.speedtest_parallel_kernels);
        assert!(saved.speedtest_persistent_kernel);
        assert_eq!(saved.speedtest_reuse_minutes, 45);
        assert!(saved.speedtest_conditional_isolation);
        assert_eq!(saved.speedtest_download_top_n, 60);
        assert!(saved.bandwidth_guard);
        assert_eq!(saved.bandwidth_min_kbps, 256);
        drop(db);

        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        let loaded = reopened.get_settings().await.unwrap();
        assert_eq!(loaded.fetch_concurrency, 8);
        assert_eq!(loaded.speedtest_concurrency, 173);
        assert_eq!(loaded.speedtest_reuse_minutes, 45);
        assert!(loaded.speedtest_conditional_isolation);

        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn persists_speedtest_kernel_and_kind() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-speedtest-meta-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO nodes
               (id, protocol, name, server, port, raw, config, created_at, updated_at)
               VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        )
        .bind("speedtest-meta-node")
        .bind("ss")
        .bind("speedtest metadata")
        .bind("example.com")
        .bind(443_i64)
        .bind("ss://example")
        .bind("{}")
        .bind(&now)
        .bind(&now)
        .execute(&db.pool)
        .await
        .unwrap();

        db.migrate().await.unwrap();

        db.update_node_speed(
            "speedtest-meta-node",
            true,
            Some(42),
            None,
            Some("mihomo"),
            Some("google_proxy"),
        )
        .await
        .unwrap();

        let node = db.get_node("speedtest-meta-node").await.unwrap().unwrap();
        assert_eq!(node.latency_kernel.as_deref(), Some("mihomo"));
        assert_eq!(node.latency_test_kind.as_deref(), Some("google_proxy"));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn migration_preserves_legacy_speedtest_results() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-legacy-probe-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO nodes
               (id, protocol, name, server, port, raw, config, latency_ms, latency_ok,
                latency_kernel, latency_test_kind, latency_tested_at, created_at, updated_at)
               VALUES ('legacy-probe', 'ss', 'legacy probe', 'example.com', 443, 'ss://legacy', '{}',
                       42, 1, 'mihomo', 'real_proxy', ?, ?, ?)"#,
        )
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&db.pool)
        .await
        .unwrap();

        db.migrate().await.unwrap();
        let node = db.get_node("legacy-probe").await.unwrap().unwrap();
        assert_eq!(node.latency_ok, Some(true));
        assert_eq!(node.latency_test_kind.as_deref(), Some("real_proxy"));
        assert_eq!(node.latency_ms, Some(42));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn list_nodes_without_source_id_returns_all_nodes() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-all-nodes-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let s1 = db
            .create_source(CreateSource {
                name: "Source 1".into(),
                url_template: "https://example.com/1".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let s2 = db
            .create_source(CreateSource {
                name: "Source 2".into(),
                url_template: "https://example.com/2".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();

        let now = Utc::now();
        let node1 = Node {
            id: "node-1".into(),
            source_id: Some(s1.id.clone()),
            protocol: "ss".into(),
            name: "Node 1".into(),
            server: "1.1.1.1".into(),
            port: 443,
            raw: "ss://node-1".into(),
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

        let node2 = Node {
            id: "node-2".into(),
            source_id: Some(s2.id.clone()),
            protocol: "ss".into(),
            name: "Node 2".into(),
            server: "2.2.2.2".into(),
            port: 443,
            raw: "ss://node-2".into(),
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

        db.replace_source_nodes(&s1.id, &[node1]).await.unwrap();
        db.replace_source_nodes(&s2.id, &[node2]).await.unwrap();

        // 挑选指定源
        let page1 = db
            .list_nodes(None, Some(&s1.id), None, None, None, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(page1.total, 1);
        assert_eq!(page1.items[0].id, "node-1");

        // "全部订阅源" (source_id: None) 应查到所有源的节点
        let page_all = db
            .list_nodes(None, None, None, None, None, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(page_all.total, 2);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn push_disabled_sources_are_excluded_from_push_export() {
        let path = std::env::temp_dir().join(format!("vpn-parser-push-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        for id in ["source-a", "source-b"] {
            sqlx::query(
                r#"INSERT INTO sources
                   (id, name, url_template, enabled, created_at, updated_at)
                   VALUES (?, ?, ?, 1, ?, ?)"#,
            )
            .bind(id)
            .bind(id)
            .bind(format!("https://example.com/{id}"))
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        for (id, source_id, server) in [
            ("node-a", Some("source-a"), "203.0.113.11"),
            ("node-b", Some("source-b"), "203.0.113.12"),
            ("node-loose", None, "203.0.113.13"),
        ] {
            sqlx::query(
                r#"INSERT INTO nodes
                   (id, source_id, protocol, name, server, port, raw, config,
                    latency_ms, latency_ok, latency_test_kind, created_at, updated_at)
                   VALUES (?, ?, 'ss', ?, ?, 443, 'ss://a@b', '{}', 100, 1, 'google_proxy', ?, ?)"#,
            )
            .bind(id)
            .bind(source_id)
            .bind(id)
            .bind(server)
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        // 节点按源分库存放：先把主表里的遗留行迁移进各订阅库。
        db.migrate().await.unwrap();

        let (nodes, skipped) = db.export_pushable_nodes(None, "").await.unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(skipped, 0);

        // 关键词黑名单：命中名字的节点即使测速可用也不参与推送。
        let (nodes, skipped) = db
            .export_pushable_nodes(None, "loose, node-a")
            .await
            .unwrap();
        let ids: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
        assert_eq!(ids, vec!["node-b"]);
        assert_eq!(skipped, 2);
        let (nodes, skipped) = db.export_pushable_nodes(None, "").await.unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(skipped, 0);

        db.update_source(
            "source-b",
            crate::models::UpdateSource {
                name: None,
                url_template: None,
                enabled: None,
                remark: None,
                push_disabled: Some(true),
                user_agent: None,
            },
        )
        .await
        .unwrap();
        let reloaded = db.get_source("source-b").await.unwrap().unwrap();
        assert!(reloaded.push_disabled);
        assert!(
            !db.get_source("source-a")
                .await
                .unwrap()
                .unwrap()
                .push_disabled
        );

        let (nodes, skipped) = db.export_pushable_nodes(None, "").await.unwrap();
        assert_eq!(skipped, 1);
        let ids: Vec<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
        assert!(ids.contains(&"node-a"));
        assert!(ids.contains(&"node-loose"), "无来源的节点不受影响");
        assert!(!ids.contains(&"node-b"));

        db.update_source(
            "source-b",
            crate::models::UpdateSource {
                name: None,
                url_template: None,
                enabled: None,
                remark: None,
                push_disabled: Some(false),
                user_agent: None,
            },
        )
        .await
        .unwrap();
        let (nodes, _) = db.export_pushable_nodes(None, "").await.unwrap();
        assert_eq!(nodes.len(), 3);
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn groups_nodes_and_limits_group_speedtest_ids() {
        let path = std::env::temp_dir().join(format!("vpn-parser-groups-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();

        for (id, name) in [("source-a", "订阅 A"), ("source-b", "订阅 B")] {
            sqlx::query(
                r#"INSERT INTO sources
                   (id, name, url_template, enabled, created_at, updated_at)
                   VALUES (?, ?, ?, 1, ?, ?)"#,
            )
            .bind(id)
            .bind(name)
            .bind(format!("https://example.com/{id}"))
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        for (id, source_id, protocol, ok, kind) in [
            (
                "a-alive",
                Some("source-a"),
                "ss",
                Some(1_i64),
                Some("google_proxy"),
            ),
            ("a-untested", Some("source-a"), "vmess", None, None),
            (
                "b-dead",
                Some("source-b"),
                "ss",
                Some(0_i64),
                Some("google_proxy"),
            ),
            ("loose-alive", None, "ss", Some(1_i64), Some("google_proxy")),
        ] {
            sqlx::query(
                r#"INSERT INTO nodes
                   (id, source_id, protocol, name, server, port, raw, config,
                    latency_ok, latency_test_kind, created_at, updated_at)
                   VALUES (?, ?, ?, ?, ?, 443, ?, '{}', ?, ?, ?, ?)"#,
            )
            .bind(id)
            .bind(source_id)
            .bind(protocol)
            .bind(id)
            .bind(format!("{id}.example.com"))
            .bind(format!("{protocol}://{id}"))
            .bind(ok)
            .bind(kind)
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        db.migrate().await.unwrap();

        let groups = db.list_node_groups(None, None, None).await.unwrap();
        assert_eq!(groups.len(), 3);
        let source_a = groups
            .iter()
            .find(|group| group.source_id.as_deref() == Some("source-a"))
            .unwrap();
        assert_eq!((source_a.alive, source_a.total), (1, 2));
        let ungrouped = groups
            .iter()
            .find(|group| group.source_id.is_none())
            .unwrap();
        assert_eq!(ungrouped.source_name, "未分组");
        assert_eq!((ungrouped.alive, ungrouped.total), (1, 1));

        let source_a_ss = db
            .list_node_ids_for_group(Some("source-a"), Some("ss"), None, None)
            .await
            .unwrap();
        assert_eq!(source_a_ss, vec!["a-alive"]);
        let ungrouped_ids = db
            .list_node_ids_for_group(None, None, None, None)
            .await
            .unwrap();
        assert_eq!(ungrouped_ids, vec!["loose-alive"]);

        let available_a = db
            .list_available_nodes_for_source("source-a")
            .await
            .unwrap();
        assert_eq!(
            available_a
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-alive"]
        );

        let available_all = db.list_available_nodes_for_source("all").await.unwrap();
        assert_eq!(
            available_all
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-alive", "loose-alive"]
        );
        let ungrouped_page = db
            .list_nodes(
                None,
                Some(UNGROUPED_SOURCE_ID),
                None,
                None,
                None,
                1,
                50,
                None,
            )
            .await
            .unwrap();
        assert_eq!(ungrouped_page.total, 1);
        assert_eq!(ungrouped_page.items[0].id, "loose-alive");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn available_panel_collapses_the_same_node_from_two_sources() {
        const SAME_UUID: &str = "11111111-1111-1111-1111-111111111111";
        const OTHER_UUID: &str = "22222222-2222-2222-2222-222222222222";
        const THIRD_UUID: &str = "33333333-3333-3333-3333-333333333333";
        let path = std::env::temp_dir().join(format!("vpn-parser-dedup-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        for id in ["dedup-a", "dedup-b"] {
            sqlx::query(
                r#"INSERT INTO sources
                   (id, name, url_template, enabled, created_at, updated_at)
                   VALUES (?, ?, ?, 1, ?, ?)"#,
            )
            .bind(id)
            .bind(format!("去重源 {id}"))
            .bind(format!("https://example.com/{id}"))
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        // 同一颗节点被两份订阅同时供着（名字不同、uuid 相同），另有一颗同端点但不同 uuid 的真节点。
        for (id, source_id, name, uuid, latency) in [
            ("clone-a", "dedup-a", "HK-01", SAME_UUID, 120_i64),
            ("clone-b", "dedup-b", "HK second", SAME_UUID, 120_i64),
            ("other-b", "dedup-b", "HK-02", OTHER_UUID, 90_i64),
        ] {
            sqlx::query(
                r#"INSERT INTO nodes
                   (id, source_id, protocol, name, server, port, raw, config,
                    latency_ms, latency_ok, latency_test_kind, created_at, updated_at)
                   VALUES (?, ?, 'vless', ?, 'dedup.example.com', 443, ?, ?, ?, 1, 'google_proxy', ?, ?)"#,
            )
            .bind(id)
            .bind(source_id)
            .bind(name)
            .bind(format!("vless://{uuid}@dedup.example.com:443#{name}"))
            .bind(format!("{{\"uuid\":\"{uuid}\"}}"))
            .bind(latency)
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        db.migrate().await.unwrap();

        let ids: Vec<String> = db
            .list_available_nodes_for_source("all")
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert_eq!(ids.len(), 2, "跨源克隆收敛成一行: {ids:?}");
        assert!(ids.contains(&"other-b".to_string()), "不同凭据的同端点节点不该被当成重复: {ids:?}");
        assert_ne!(
            ids.contains(&"clone-a".to_string()),
            ids.contains(&"clone-b".to_string()),
            "克隆行只该留一颗: {ids:?}"
        );

        // 反向对照：把另一颗的 uuid 改成第三个值，三行都该回来。
        let pool_b = db.open_node_pool(Some("dedup-b")).await.unwrap();
        sqlx::query("UPDATE node_payloads SET config = ? WHERE node_id = ?")
            .bind(format!("{{\"uuid\":\"{THIRD_UUID}\"}}"))
            .bind("clone-b")
            .execute(&pool_b)
            .await
            .unwrap();
        let after: Vec<String> = db
            .list_available_nodes_for_source("all")
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert_eq!(after.len(), 3, "凭据一改就不是同一颗节点: {after:?}");

        drop(db);
        let _ = std::fs::remove_file(&path);
    }
    #[tokio::test]
    async fn country_filtering_in_list_nodes() {
        let path = std::env::temp_dir().join(format!("vpn-parser-country-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        db.migrate().await.unwrap();
        let now = Utc::now().to_rfc3339();

        for (id, country) in [
            ("node-hk", Some("HK")),
            ("node-jp", Some("JP")),
            ("node-other", None),
        ] {
            sqlx::query(
                r#"INSERT INTO nodes
                   (id, source_id, protocol, name, server, port, raw, config, country,
                    latency_ok, created_at, updated_at)
                   VALUES (?, 'src-1', 'ss', ?, 'example.com', 443, 'ss://uri', '{}', ?,
                           1, ?, ?)"#,
            )
            .bind(id)
            .bind(id)
            .bind(country)
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }

        db.migrate().await.unwrap();

        // All countries
        let all = db
            .list_nodes(None, Some("src-1"), None, None, None, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(all.total, 3);

        // Filter by HK
        let hk = db
            .list_nodes(None, Some("src-1"), None, None, Some("HK"), 1, 50, None)
            .await
            .unwrap();
        assert_eq!(hk.total, 1);
        assert_eq!(hk.items[0].id, "node-hk");

        // Filter by JP
        let jp = db
            .list_nodes(None, Some("src-1"), None, None, Some("JP"), 1, 50, None)
            .await
            .unwrap();
        assert_eq!(jp.total, 1);
        assert_eq!(jp.items[0].id, "node-jp");

        // Filter by OTHER (includes None or non-standard)
        let other = db
            .list_nodes(None, Some("src-1"), None, None, Some("OTHER"), 1, 50, None)
            .await
            .unwrap();
        assert_eq!(other.total, 1);
        assert_eq!(other.items[0].id, "node-other");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn test_source_homogeneity_analysis_and_pruning() {
        let path = std::env::temp_dir().join(format!("vpn-parser-homo-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();

        let s1 = db
            .create_source(CreateSource {
                name: "Core Source".into(),
                url_template: "https://example.com/s1".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();

        let s2 = db
            .create_source(CreateSource {
                name: "Mirror Redundant Source".into(),
                url_template: "https://example.com/s2".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();

        let s3 = db
            .create_source(CreateSource {
                name: "Unique Source".into(),
                url_template: "https://example.com/s3".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();

        // 第四个源：一次都没拉出节点（新加的、或者订阅内容空掉的，都是这个形状）。
        let s4 = db
            .create_source(CreateSource {
                name: "Empty Source".into(),
                url_template: "https://example.com/s4".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();

        let make_node = |id: &str, src_id: &str, server: &str, alive: bool| Node {
            id: id.to_string(),
            source_id: Some(src_id.to_string()),
            protocol: "vmess".to_string(),
            name: format!("Node {server}"),
            server: server.to_string(),
            port: 443,
            raw: String::new(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: if alive { Some(50) } else { None },
            latency_ok: Some(alive),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        // Source 1 has 1.1.1.1:443 (alive) and 2.2.2.2:443 (dead)
        let s1_nodes = vec![
            make_node("n1", &s1.id, "1.1.1.1", true),
            make_node("n2", &s1.id, "2.2.2.2", false),
        ];
        db.replace_source_nodes(&s1.id, &s1_nodes).await.unwrap();

        // Source 2 has exactly 1.1.1.1:443 and 2.2.2.2:443 (100% duplicate of s1!)
        let s2_nodes = vec![
            make_node("n3", &s2.id, "1.1.1.1", true),
            make_node("n4", &s2.id, "2.2.2.2", false),
        ];
        db.replace_source_nodes(&s2.id, &s2_nodes).await.unwrap();

        // Source 3 has 3.3.3.3:443 (unique and alive!)
        let s3_nodes = vec![make_node("n5", &s3.id, "3.3.3.3", true)];
        db.replace_source_nodes(&s3.id, &s3_nodes).await.unwrap();

        // Run analysis
        let reports = db.analyze_source_homogeneity().await.unwrap();
        assert_eq!(reports.len(), 4);

        let rep_s2 = reports.iter().find(|r| r.source_id == s2.id).unwrap();
        assert_eq!(rep_s2.overlap_rate_pct, 100.0);
        assert_eq!(rep_s2.unique_alive_nodes, 0);
        assert!(rep_s2.is_redundant);

        let rep_s3 = reports.iter().find(|r| r.source_id == s3.id).unwrap();
        assert_eq!(rep_s3.unique_alive_nodes, 1);
        assert_eq!(rep_s3.overlap_rate_pct, 0.0);
        assert!(!rep_s3.is_redundant);

        // 0 节点的源必须是剔除候选。以前这条要再叠一个"连续失败 >= 2 次"，于是刚拉回
        // 0 节点、fail_streak 还是 0 的源一直挂着"核心保留"（用户 2026-10-15 点的这条）。
        let rep_s4 = reports.iter().find(|r| r.source_id == s4.id).unwrap();
        assert_eq!(rep_s4.total_nodes, 0);
        assert!(rep_s4.is_redundant, "0 节点的源没被判成剔除候选");
        assert_eq!(
            rep_s4.verdict(),
            crate::models::HomogeneityVerdict::Redundant,
            "界面话术和数据层判定不一致"
        );
        assert!(
            rep_s4.verdict_label().contains("0 节点"),
            "空源的文案没说出理由是空：{}",
            rep_s4.verdict_label()
        );

        // Prune by disabling
        let prune_res = db
            .prune_homogenized_sources(&[s2.id.clone()], false)
            .await
            .unwrap();
        assert_eq!(prune_res.pruned_count, 1);
        let s2_updated = db.get_source(&s2.id).await.unwrap().unwrap();
        assert!(!s2_updated.enabled);

        // Prune by deleting
        let delete_res = db
            .prune_homogenized_sources(&[s2.id.clone()], true)
            .await
            .unwrap();
        assert_eq!(delete_res.pruned_count, 1);
        assert!(db.get_source(&s2.id).await.unwrap().is_none());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn top_verified_nodes_honours_exclude_id() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-topverified-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO sources
               (id, name, url_template, enabled, created_at, updated_at)
               VALUES ('src-top', 'TopN', 'https://example.com/top', 1, ?, ?)"#,
        )
        .bind(&now)
        .bind(&now)
        .execute(&db.pool)
        .await
        .unwrap();

        let node = |id: &str, server: &str, latency: i64| Node {
            id: id.to_string(),
            source_id: Some("src-top".to_string()),
            protocol: "vmess".to_string(),
            name: format!("Node {id}"),
            server: server.to_string(),
            port: 443,
            raw: String::new(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: Some(latency),
            latency_ok: Some(true),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.replace_source_nodes(
            "src-top",
            &[
                node("keep-1", "10.0.0.1", 100),
                node("keep-2", "10.0.0.2", 200),
                node("skip", "10.0.0.3", 50),
            ],
        )
        .await
        .unwrap();

        // exclude_id 会往 SQL 里拼 " AND n.id != ?"：多打一个 `?` 时语句准备阶段就
        // 报错，候选永远是空的，url-test 于是静默退化成单节点组。
        let picked = db.top_verified_nodes(3, Some("skip")).await.unwrap();
        let ids: Vec<String> = picked.iter().map(|item| item.id.clone()).collect();
        assert_eq!(ids, vec!["keep-1", "keep-2"]);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// 批量取节点：跨分库一次拿完，不存在的 id 安静地少掉。
    /// 调用方以前写 `for id in ids { get_node(id) }`，每个 id 都要扇出全部分库。
    #[tokio::test]
    async fn get_nodes_batches_across_shards() {
        let path = std::env::temp_dir().join(format!("vpn-parser-getnodes-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        for id in ["src-x", "src-y"] {
            sqlx::query(
                r#"INSERT INTO sources
                   (id, name, url_template, enabled, created_at, updated_at)
                   VALUES (?, ?, ?, 1, ?, ?)"#,
            )
            .bind(id)
            .bind(format!("订阅 {id}"))
            .bind(format!("https://example.com/{id}"))
            .bind(&now)
            .bind(&now)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        let node = |id: &str, src: &str, server: &str| Node {
            id: id.to_string(),
            source_id: Some(src.to_string()),
            protocol: "vmess".to_string(),
            name: format!("Node {id}"),
            server: server.to_string(),
            port: 443,
            raw: String::new(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: Some(100),
            latency_ok: Some(true),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.replace_source_nodes(
            "src-x",
            &[
                node("x1", "src-x", "10.0.0.1"),
                node("x2", "src-x", "10.0.0.2"),
            ],
        )
        .await
        .unwrap();
        db.replace_source_nodes("src-y", &[node("y1", "src-y", "10.0.1.1")])
            .await
            .unwrap();

        let ids = [
            "x1".to_string(),
            "x2".to_string(),
            "y1".to_string(),
            "missing".to_string(),
        ];
        let mut found: Vec<String> = db
            .get_nodes(&ids)
            .await
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec!["x1".to_string(), "x2".to_string(), "y1".to_string()]
        );
        assert!(db.get_nodes(&[]).await.unwrap().is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// Telegram 频道这类"零散发节点"的源靠这条语义活着：这一页没看见的旧节点不许被删。
    #[tokio::test]
    async fn accumulate_keeps_nodes_the_current_page_does_not_show() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-accumulate-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO sources
               (id, name, url_template, enabled, created_at, updated_at)
               VALUES (?, ?, ?, 1, ?, ?)"#,
        )
        .bind("src-tg")
        .bind("TG 频道")
        .bind("https://t.me/s/demo")
        .bind(&now)
        .bind(&now)
        .execute(&db.pool)
        .await
        .unwrap();

        let node = |name: &str, server: &str, latency: Option<i64>| Node {
            id: Uuid::new_v4().to_string(),
            source_id: Some("src-tg".to_string()),
            protocol: "ss".to_string(),
            name: name.to_string(),
            server: server.to_string(),
            port: 8388,
            raw: format!("ss://token@{server}:8388#{name}"),
            config: serde_json::json!({"cipher": "aes-256-gcm", "password": "p"}),
            country: Some("US".to_string()),
            latency_ms: latency,
            latency_ok: latency.map(|value| value > 0),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        // 第一轮：页面上有 A（已经测过 111ms）和 B。
        assert_eq!(
            db.accumulate_source_nodes(
                "src-tg",
                &[
                    node("A", "10.0.0.1", Some(111)),
                    node("B", "10.0.0.2", None)
                ],
            )
            .await
            .unwrap(),
            2
        );
        // 第二轮：A 掉出这一页，只剩 B 和新来的 C。
        assert_eq!(
            db.accumulate_source_nodes(
                "src-tg",
                &[node("B", "10.0.0.2", None), node("C", "10.0.0.3", None)],
            )
            .await
            .unwrap(),
            3,
            "累积写入应该报出分片总数，不是本轮条数"
        );

        let page = db
            .list_nodes(None, Some("src-tg"), None, None, None, 1, 50, None)
            .await
            .unwrap();
        let mut names: Vec<String> = page.items.iter().map(|item| item.name.clone()).collect();
        names.sort();
        assert_eq!(
            names,
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            "这一页看不见的 A 不该被删掉"
        );
        let carried = page.items.iter().find(|item| item.name == "A").unwrap();
        assert_eq!(
            carried.latency_ms,
            Some(111),
            "搬回去的节点必须保住测速记录"
        );
        assert_eq!(carried.server, "10.0.0.1");
        assert_eq!(carried.port, 8388);
        assert_eq!(carried.country.as_deref(), Some("US"));
        let carried_full = db.get_node(&carried.id).await.unwrap().expect("A 还在");
        assert!(
            carried_full.raw.contains("10.0.0.1"),
            "原始链接也要一起搬，不然回溯不了：{:?}",
            carried_full.raw
        );
        assert_eq!(carried_full.config["cipher"], "aes-256-gcm");

        // 反向验证：普通订阅源仍然是"这一次没出现就删"。
        db.replace_source_nodes("src-tg", &[node("C", "10.0.0.3", None)])
            .await
            .unwrap();
        let after = db
            .list_nodes(None, Some("src-tg"), None, None, None, 1, 50, None)
            .await
            .unwrap();
        assert_eq!(after.total, 1, "replace 的覆盖语义不能被累积模式污染");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// 「节点故障自动切换」以前只是界面内存里的一个字段，重启就回 false ——
    /// 用户以为开着，实际一次都不会切。现在它必须落库、且别的设计算保存不能把它冲掉。
    #[tokio::test]
    async fn auto_failover_survives_reopen_and_other_settings_saves() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-auto-failover-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            !db.get_settings().await.unwrap().auto_failover,
            "默认必须是关：这个功能会改他的实时出口，不能替他打开"
        );

        db.update_settings(UpdateSettings {
            auto_failover: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(db.get_settings().await.unwrap().auto_failover);

        // 只改别的一项时不许把自动切换冲回默认（整套设置保存走的是 None）。
        db.update_settings(UpdateSettings {
            fetch_concurrency: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
        let kept = db.get_settings().await.unwrap();
        assert_eq!(kept.fetch_concurrency, 5);
        assert!(kept.auto_failover, "只改并发数不该把自动切换的开关冲掉");

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            reopened.get_settings().await.unwrap().auto_failover,
            "重开软件必须还是开着的样子"
        );
        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn gist_settings_survive_reopen_and_settings_save() {
        let path = std::env::temp_dir().join(format!("vpn-parser-gist-settings-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let defaults = db.get_settings().await.unwrap();
        assert_eq!(defaults.gist_pages, 100);
        assert_eq!(defaults.gist_queries, "vless//");

        db.update_settings(UpdateSettings {
            gist_pages: Some(42),
            gist_queries: Some("vless//,vmess//,hy2//".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();

        let updated = db.get_settings().await.unwrap();
        assert_eq!(updated.gist_pages, 42);
        assert_eq!(updated.gist_queries, "vless//,vmess//,hy2//");

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        let reloaded = reopened.get_settings().await.unwrap();
        assert_eq!(reloaded.gist_pages, 42);
        assert_eq!(reloaded.gist_queries, "vless//,vmess//,hy2//");
        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// 「测通节点太少时自动补一轮全量测速」：默认必须关（它会自己起一轮重活），
    /// 必须落库、扛得住别的设置保存和重开；`node_verification_counts` 要能跨分库
    /// 把"一共多少 / 测通多少"数对 —— 那是这个判定的唯一输入。
    #[tokio::test]
    async fn auto_refill_switch_persists_and_counts_are_cross_shard() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-auto-refill-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            !db.get_settings().await.unwrap().speedtest_auto_refill,
            "默认必须是关：自动起一轮全量测速不能替他决定"
        );

        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO sources
               (id, name, url_template, enabled, created_at, updated_at)
               VALUES ('src-refill', 'Refill', 'https://example.com/refill', 1, ?, ?)"#,
        )
        .bind(&now)
        .bind(&now)
        .execute(&db.pool)
        .await
        .unwrap();
        let node = |id: &str, octet: &str, verified: bool| Node {
            id: id.to_string(),
            source_id: Some("src-refill".to_string()),
            protocol: "vmess".to_string(),
            name: format!("Node {id}"),
            server: format!("10.0.0.{octet}"),
            port: 443,
            raw: String::new(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: if verified { Some(100) } else { None },
            latency_ok: verified.then_some(true),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        // 四个节点必须各给一个地址：`replace_source_nodes` 按 server:port 去重。
        db.replace_source_nodes(
            "src-refill",
            &[
                node("ok-1", "1", true),
                node("ok-2", "2", true),
                node("dead", "3", false),
                node("untested", "4", false),
            ],
        )
        .await
        .unwrap();
        let (total, verified) = db.node_verification_counts().await.unwrap();
        assert_eq!(
            (total, verified),
            (4, 2),
            "总数与测通数要分开数，判定用的是后者"
        );

        db.update_settings(UpdateSettings {
            speedtest_auto_refill: Some(true),
            auto_rearm_system_proxy: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        db.update_settings(UpdateSettings {
            fetch_concurrency: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(
            db.get_settings().await.unwrap().speedtest_auto_refill,
            "只改并发数不该把补测开关冲掉"
        );

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            reopened.get_settings().await.unwrap().speedtest_auto_refill,
            "重开软件必须还是开着的样子"
        );
        // 这里不再断言重开后的节点数：`Database::connect` 在 `cfg!(test)` 下给每次连接
        // 都挂一个新的随机分库目录（避免并行测试互相踩库），所以重开的那个句点数到的是
        // 0 —— 那是测试脚手架，不是应用行为。上面的计数已经证明了真读法。
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// 夜间模式**默认开**（用户 2026-10-15 指定）：新库第一帧就是深色，老库补上这一列之后
    /// 存量行也跟着变夜间 —— 那是他要的效果。反过来也得成立：关掉之后必须把住别的设置保存
    /// 和重开，否则就是经典的"我明明关了它又黑回去"。
    /// 菜单栏展开态：新库默认收起（和老那根 64 宽的图标条一模一样），打开之后重开还在。
    #[tokio::test]
    async fn sidebar_expanded_defaults_off_and_survives_reopen() {
        let path = std::env::temp_dir().join(format!("vpn-parser-sidebar-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            !db.get_settings().await.unwrap().sidebar_expanded,
            "默认必须是收起"
        );
        db.update_settings(UpdateSettings {
            sidebar_expanded: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(db.get_settings().await.unwrap().sidebar_expanded);
        drop(db);

        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            reopened.get_settings().await.unwrap().sidebar_expanded,
            "重开之后还得是展开态"
        );
        // 别的设置保存一次不许把它顶回默认。
        reopened
            .update_settings(UpdateSettings {
                fetch_concurrency: Some(4),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(reopened.get_settings().await.unwrap().sidebar_expanded);
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn night_mode_defaults_on_and_stays_off_when_turned_off() {
        let path = std::env::temp_dir().join(format!("vpn-parser-night-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            db.get_settings().await.unwrap().ui_dark,
            "新库默认就该是夜间"
        );

        db.update_settings(UpdateSettings {
            ui_dark: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        db.update_settings(UpdateSettings {
            fetch_concurrency: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(
            !db.get_settings().await.unwrap().ui_dark,
            "别的设置保存不该把夜间翻回去"
        );

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        assert!(
            !reopened.get_settings().await.unwrap().ui_dark,
            "重开必须还是浅白：默认值不该盖过用户的选择"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    /// 「预备状态自动下载内核」和「加完订阅源就测这个源」是**默认开**的两条：他要的
    /// 就是新装机第一次启动自己动起来。关掉的必须扛住别的设置保存和重开，否则
    /// "我明明关了它又自己下"这种事没法交代。
    #[tokio::test]
    async fn the_two_auto_actions_default_on_and_keep_being_off() {
        let path =
            std::env::temp_dir().join(format!("vpn-parser-auto-actions-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();
        let fresh = db.get_settings().await.unwrap();
        assert!(
            fresh.auto_prepare_kernel && fresh.speedtest_added_source,
            "新库里这两条必须是开：预备状态没人点第二次"
        );

        db.update_settings(UpdateSettings {
            auto_prepare_kernel: Some(false),
            speedtest_added_source: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
        db.update_settings(UpdateSettings {
            fetch_concurrency: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
        let kept = db.get_settings().await.unwrap();
        assert!(
            !kept.auto_prepare_kernel,
            "关掉之后别的设计算保存不该把它翻回来"
        );
        assert!(!kept.speedtest_added_source);

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        let loaded = reopened.get_settings().await.unwrap();
        assert!(!loaded.auto_prepare_kernel, "重开必须还是关着");
        assert!(!loaded.speedtest_added_source);
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    /// 发布服务器地址 / 推送 token 以前只写 eframe storage，而 `persistence` 是关掉的
    /// → 填了永远不回显。现在走 settings 表，并且必须扛得住整套设置的周期性保存。
    #[tokio::test]
    async fn publisher_config_survives_reopen_and_settings_save() {
        let path = std::env::temp_dir().join(format!(
            "vpn-parser-publisher-settings-{}.db",
            Uuid::new_v4()
        ));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();

        let saved = db
            .update_publisher("  https://gate.example/path/  ", "  secret-token ")
            .await
            .unwrap();
        assert_eq!(saved.publisher_api, "https://gate.example/path/");
        assert_eq!(saved.publisher_token, "secret-token");

        // 主题与字号先各落一次库：后面那次"其它设置保存"不能把它们冲掉。
        db.update_settings(UpdateSettings {
            ui_theme: Some("mocha".to_string()),
            ui_scale: Some(1.2),
            ..Default::default()
        })
        .await
        .unwrap();

        // 其它设置的保存（这两项传 None）不能把发布配置冲掉。
        db.update_settings(UpdateSettings {
            fetch_concurrency: Some(6),
            ..Default::default()
        })
        .await
        .unwrap();

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        let loaded = reopened.get_settings().await.unwrap();
        assert_eq!(loaded.publisher_api, "https://gate.example/path/");
        assert_eq!(loaded.publisher_token, "secret-token");
        assert_eq!(loaded.ui_theme, "mocha");
        assert!(
            (loaded.ui_scale - 1.2).abs() < 0.001,
            "字号设置不能被其它设置的保存冲掉，实际 {}",
            loaded.ui_scale
        );
        let clamped = reopened
            .update_settings(UpdateSettings {
                ui_scale: Some(9.0),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            (clamped.ui_scale - 1.40).abs() < 0.001,
            "越界的字号要夹回上限，实际 {}",
            clamped.ui_scale
        );
        assert_eq!(loaded.fetch_concurrency, 6);

        let updated_ui = reopened
            .update_settings(UpdateSettings {
                window_size: Some("small".to_string()),
                console_tabs: Some("charts,nodes".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(updated_ui.window_size, "small");
        assert_eq!(updated_ui.console_tabs, "charts,nodes");

        // 清空要能真的清空。
        let cleared = reopened.update_publisher("", "").await.unwrap();
        assert!(cleared.publisher_api.is_empty());
        assert!(cleared.publisher_token.is_empty());

        drop(reopened);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }

    #[tokio::test]
    async fn source_rules_and_groups_persist_and_survive_reopen() {
        let path = std::env::temp_dir().join(format!("vpn-parser-source-rules-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();

        let source = db
            .create_source(CreateSource {
                name: "测试订阅带规则".into(),
                url_template: "https://example.com/sub".into(),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();

        assert!(source.rules_yaml.is_none());
        assert!(source.groups_yaml.is_none());

        let rules = "- DOMAIN,example.com,DIRECT\n- MATCH,PROXY";
        let groups = "- name: PROXY\n  type: select\n  proxies: [DIRECT]";

        db.update_source_rules_and_groups(&source.id, Some(rules), Some(groups))
            .await
            .unwrap();

        let loaded = db.get_source(&source.id).await.unwrap().expect("source exists");
        assert_eq!(loaded.rules_yaml.as_deref(), Some(rules));
        assert_eq!(loaded.groups_yaml.as_deref(), Some(groups));

        let list = db.list_sources().await.unwrap();
        let in_list = list.iter().find(|s| s.id == source.id).unwrap();
        assert_eq!(in_list.rules_yaml.as_deref(), Some(rules));
        assert_eq!(in_list.groups_yaml.as_deref(), Some(groups));

        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        let reloaded = reopened.get_source(&source.id).await.unwrap().expect("reopened source");
        assert_eq!(reloaded.rules_yaml.as_deref(), Some(rules));
        assert_eq!(reloaded.groups_yaml.as_deref(), Some(groups));

        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[tokio::test]
    async fn source_user_agent_persists_and_updates() {
        let path = std::env::temp_dir().join(format!("vpn-parser-source-ua-{}.db", Uuid::new_v4()));
        let db = Database::connect(path.to_str().unwrap()).await.unwrap();

        // 1. Create with custom user_agent
        let source = db
            .create_source(CreateSource {
                name: "测试自定义UA".into(),
                url_template: "https://example.com/sub".into(),
                enabled: true,
                remark: None,
                user_agent: Some("clash-verge/v1.7.7".into()),
            })
            .await
            .unwrap();

        assert_eq!(source.user_agent.as_deref(), Some("clash-verge/v1.7.7"));

        let loaded = db.get_source(&source.id).await.unwrap().expect("source exists");
        assert_eq!(loaded.user_agent.as_deref(), Some("clash-verge/v1.7.7"));

        // 2. Update to a new UA
        let updated = db
            .update_source(
                &source.id,
                crate::models::UpdateSource {
                    name: None,
                    url_template: None,
                    enabled: None,
                    remark: None,
                    push_disabled: None,
                    user_agent: Some("sing-box/1.9.0".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.user_agent.as_deref(), Some("sing-box/1.9.0"));

        let reloaded = db.get_source(&source.id).await.unwrap().expect("source exists");
        assert_eq!(reloaded.user_agent.as_deref(), Some("sing-box/1.9.0"));

        // 3. Clear user_agent with empty string
        let cleared = db
            .update_source(
                &source.id,
                crate::models::UpdateSource {
                    name: None,
                    url_template: None,
                    enabled: None,
                    remark: None,
                    push_disabled: None,
                    user_agent: Some("".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(cleared.user_agent, None);

        // 4. Survive reconnect
        drop(db);
        let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
        let from_reopened = reopened.get_source(&source.id).await.unwrap().expect("source exists");
        assert_eq!(from_reopened.user_agent, None);

        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

/// 开机自启动：默认关，开了能落库、重开软件还在，关回去也能清掉。
#[tokio::test]
async fn persists_launch_at_startup_and_defaults_to_off() {
    let path = std::env::temp_dir().join(format!(
        "vpn-parser-launch-at-startup-{}.db",
        Uuid::new_v4()
    ));
    let db = Database::connect(path.to_str().unwrap()).await.unwrap();

    // 默认关：动用户注册表的开关不能擅自开
    assert!(!db.get_settings().await.unwrap().launch_at_startup);

    let saved = db
        .update_settings(UpdateSettings {
            launch_at_startup: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(saved.launch_at_startup);

    drop(db);
    let reopened = Database::connect(path.to_str().unwrap()).await.unwrap();
    assert!(reopened.get_settings().await.unwrap().launch_at_startup);

    let off = reopened
        .update_settings(UpdateSettings {
            launch_at_startup: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!off.launch_at_startup);

    drop(reopened);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

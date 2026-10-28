use super::Database;
use crate::models::{AppSettings, UpdateSettings};
use crate::proxy::MihomoConfigOptions;
use anyhow::{Context, Result};
use chrono::Utc;

impl Database {
    pub async fn get_proxy_config(&self) -> Result<MihomoConfigOptions> {
        let row = sqlx::query("SELECT config_json FROM proxy_settings WHERE id = 1")
            .fetch_one(&self.pool)
            .await?;
        let json: String = sqlx::Row::get(&row, "config_json");
        if json.trim().is_empty() {
            return Ok(MihomoConfigOptions::default());
        }
        let mut config: MihomoConfigOptions =
            serde_json::from_str(&json).context("解析已保存的代理配置失败")?;
        let migrated = config.dns.migrate_legacy_defaults();
        if migrated {
            self.update_proxy_config(&config).await?;
        }
        config.validate()?;
        Ok(config)
    }

    pub async fn update_proxy_config(&self, config: &MihomoConfigOptions) -> Result<()> {
        config.validate()?;
        let json = serde_json::to_string(config)?;
        sqlx::query("UPDATE proxy_settings SET config_json = ? WHERE id = 1")
            .bind(json)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 只写发布服务器两项，避免和"整套设置"的读写互相覆盖。
    pub async fn update_publisher(&self, api: &str, token: &str) -> Result<AppSettings> {
        sqlx::query("UPDATE settings SET publisher_api=?, publisher_token=? WHERE id=1")
            .bind(api.trim())
            .bind(token.trim())
            .execute(&self.pool)
            .await?;
        self.get_settings().await
    }

    pub async fn get_settings(&self) -> Result<AppSettings> {
        let row = sqlx::query(
            r#"SELECT auto_fetch_enabled, auto_fetch_interval_min, auto_speedtest_enabled,
                       fetch_concurrency, fetch_timeout_secs,
                       speedtest_concurrency, speedtest_timeout_ms,
                       speedtest_download, download_test_ms,
                       http_server_enabled, close_to_tray,
                       exclude_keywords, failover_latency_ms,
                       speedtest_parallel_kernels, speedtest_persistent_kernel,
                       speedtest_reuse_minutes, speedtest_conditional_isolation,
                       speedtest_download_top_n, speedtest_dedup_egress, speedtest_tcp_ping_only, bandwidth_guard, bandwidth_min_kbps,
                       publisher_api, publisher_token, ui_theme, app_icon_style, ui_language, ui_scale, ui_dark, sidebar_expanded, kernel_source, firewall_auto_allow,
                       auto_failover, insider_channel, speedtest_auto_refill,
                       auto_prepare_kernel, speedtest_added_source,
                       auto_rearm_system_proxy,
                       gist_pages, gist_queries,
                       window_size, console_tabs,
                       last_auto_fetch_at, last_auto_speedtest_at
                FROM settings WHERE id = 1"#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(AppSettings {
            auto_fetch_enabled: sqlx::Row::get::<i64, _>(&row, "auto_fetch_enabled") != 0,
            auto_fetch_interval_min: sqlx::Row::get::<i64, _>(&row, "auto_fetch_interval_min")
                as u32,
            auto_speedtest_enabled: sqlx::Row::get::<i64, _>(&row, "auto_speedtest_enabled") != 0,
            fetch_concurrency:
                (sqlx::Row::try_get::<i64, _>(&row, "fetch_concurrency").unwrap_or(4) as u32)
                    .clamp(1, 8),
            fetch_timeout_secs: sqlx::Row::try_get::<i64, _>(&row, "fetch_timeout_secs")
                .unwrap_or(30) as u32,
            speedtest_concurrency: sqlx::Row::get::<i64, _>(&row, "speedtest_concurrency") as u32,
            speedtest_timeout_ms: sqlx::Row::get::<i64, _>(&row, "speedtest_timeout_ms") as u32,
            speedtest_download: sqlx::Row::try_get::<i64, _>(&row, "speedtest_download")
                .map(|v| v != 0)
                .unwrap_or(false),
            download_test_ms: sqlx::Row::try_get::<i64, _>(&row, "download_test_ms").unwrap_or(800)
                as u32,
            http_server_enabled: sqlx::Row::try_get::<i64, _>(&row, "http_server_enabled")
                .map(|v| v != 0)
                .unwrap_or(true),
            close_to_tray: sqlx::Row::try_get::<i64, _>(&row, "close_to_tray")
                .map(|v| v != 0)
                .unwrap_or(true),
            exclude_keywords: sqlx::Row::try_get::<String, _>(&row, "exclude_keywords")
                .unwrap_or_default(),
            failover_latency_ms: sqlx::Row::try_get::<i64, _>(&row, "failover_latency_ms")
                .unwrap_or(1500) as u32,
            speedtest_parallel_kernels: sqlx::Row::try_get::<i64, _>(
                &row,
                "speedtest_parallel_kernels",
            )
            .unwrap_or(0)
                != 0,
            speedtest_persistent_kernel: sqlx::Row::try_get::<i64, _>(
                &row,
                "speedtest_persistent_kernel",
            )
            .unwrap_or(0)
                != 0,
            speedtest_reuse_minutes: sqlx::Row::try_get::<i64, _>(&row, "speedtest_reuse_minutes")
                .unwrap_or(0) as u32,
            speedtest_conditional_isolation: sqlx::Row::try_get::<i64, _>(
                &row,
                "speedtest_conditional_isolation",
            )
            .unwrap_or(0)
                != 0,
            speedtest_download_top_n: sqlx::Row::try_get::<i64, _>(&row, "speedtest_download_top_n")
                .unwrap_or(0) as u32,
            speedtest_dedup_egress: sqlx::Row::try_get::<i64, _>(&row, "speedtest_dedup_egress")
                .unwrap_or(1)
                != 0,
            speedtest_tcp_ping_only: sqlx::Row::try_get::<i64, _>(&row, "speedtest_tcp_ping_only")
                .unwrap_or(0)
                != 0,
            bandwidth_guard: sqlx::Row::try_get::<i64, _>(&row, "bandwidth_guard").unwrap_or(0)
                != 0,
            bandwidth_min_kbps: sqlx::Row::try_get::<i64, _>(&row, "bandwidth_min_kbps")
                .unwrap_or(0) as u32,
            publisher_api: sqlx::Row::get::<String, _>(&row, "publisher_api"),
            publisher_token: sqlx::Row::get::<String, _>(&row, "publisher_token"),
            ui_theme: sqlx::Row::try_get::<String, _>(&row, "ui_theme")
                .unwrap_or_else(|_| crate::models::AppSettings::default().ui_theme),
            app_icon_style: sqlx::Row::try_get::<String, _>(&row, "app_icon_style")
                .unwrap_or_else(|_| crate::models::AppSettings::default().app_icon_style),
            ui_language: sqlx::Row::try_get::<String, _>(&row, "ui_language")
                .unwrap_or_else(|_| crate::models::AppSettings::default().ui_language),
            ui_scale: sqlx::Row::try_get::<f64, _>(&row, "ui_scale")
                .map(|value| crate::models::clamp_ui_scale(value as f32))
                .unwrap_or_else(|_| crate::models::AppSettings::default().ui_scale),
            ui_dark: sqlx::Row::try_get::<i64, _>(&row, "ui_dark").unwrap_or(0) != 0,
            sidebar_expanded: sqlx::Row::try_get::<i64, _>(&row, "sidebar_expanded").unwrap_or(0) != 0,
            kernel_source: sqlx::Row::try_get::<String, _>(&row, "kernel_source")
                .unwrap_or_else(|_| crate::models::AppSettings::default().kernel_source),
            firewall_auto_allow: sqlx::Row::try_get::<i64, _>(&row, "firewall_auto_allow")
                .unwrap_or(0)
                != 0,
            auto_failover: sqlx::Row::try_get::<i64, _>(&row, "auto_failover").unwrap_or(0) != 0,
            insider_channel: sqlx::Row::try_get::<i64, _>(&row, "insider_channel").unwrap_or(0)
                != 0,
            speedtest_auto_refill: sqlx::Row::try_get::<i64, _>(&row, "speedtest_auto_refill")
                .unwrap_or(0)
                != 0,
            auto_rearm_system_proxy: sqlx::Row::try_get::<i64, _>(&row, "auto_rearm_system_proxy")
                .unwrap_or(0)
                != 0,
            // 这两条缺列时按"开"读：它们是默认开的行为开关，不能因为老库没这列就变成关。
            auto_prepare_kernel: sqlx::Row::try_get::<i64, _>(&row, "auto_prepare_kernel")
                .unwrap_or(1)
                != 0,
            speedtest_added_source: sqlx::Row::try_get::<i64, _>(&row, "speedtest_added_source")
                .unwrap_or(1)
                != 0,
            gist_pages: sqlx::Row::try_get::<i64, _>(&row, "gist_pages").unwrap_or(100) as u32,
            gist_queries: sqlx::Row::try_get::<String, _>(&row, "gist_queries")
                .unwrap_or_else(|_| "vless//".to_string()),
            window_size: sqlx::Row::try_get::<String, _>(&row, "window_size")
                .unwrap_or_else(|_| "large".to_string()),
            console_tabs: sqlx::Row::try_get::<String, _>(&row, "console_tabs")
                .unwrap_or_else(|_| "all".to_string()),
            last_auto_fetch_at: sqlx::Row::get::<Option<String>, _>(&row, "last_auto_fetch_at")
                .map(|s| super::parse_dt(&s)),
            last_auto_speedtest_at: sqlx::Row::get::<Option<String>, _>(
                &row,
                "last_auto_speedtest_at",
            )
            .map(|s| super::parse_dt(&s)),
        })
    }

    pub async fn update_settings(&self, input: UpdateSettings) -> Result<AppSettings> {
        let mut s = self.get_settings().await?;
        if let Some(v) = input.auto_fetch_enabled {
            s.auto_fetch_enabled = v;
        }
        if let Some(v) = input.auto_fetch_interval_min {
            s.auto_fetch_interval_min = v.clamp(5, 24 * 60);
        }
        if let Some(v) = input.auto_speedtest_enabled {
            s.auto_speedtest_enabled = v;
        }
        if let Some(v) = input.fetch_concurrency {
            s.fetch_concurrency = v.clamp(1, 8);
        }
        if let Some(v) = input.fetch_timeout_secs {
            s.fetch_timeout_secs = v.clamp(10, 120);
        }
        if let Some(v) = input.speedtest_concurrency {
            s.speedtest_concurrency = v.clamp(1, 512);
        }
        if let Some(v) = input.speedtest_timeout_ms {
            s.speedtest_timeout_ms = v.clamp(200, 15_000);
        }
        if let Some(v) = input.speedtest_download {
            s.speedtest_download = v;
        }
        if let Some(v) = input.download_test_ms {
            s.download_test_ms = v.clamp(200, 5_000);
        }
        if let Some(v) = input.http_server_enabled {
            s.http_server_enabled = v;
        }
        if let Some(v) = input.close_to_tray {
            s.close_to_tray = v;
        }
        if let Some(v) = input.exclude_keywords {
            s.exclude_keywords = v.chars().take(400).collect();
        }
        if let Some(v) = input.failover_latency_ms {
            s.failover_latency_ms = v.clamp(200, 15_000);
        }
        if let Some(v) = input.speedtest_parallel_kernels {
            s.speedtest_parallel_kernels = v;
        }
        if let Some(v) = input.speedtest_persistent_kernel {
            s.speedtest_persistent_kernel = v;
        }
        if let Some(v) = input.speedtest_reuse_minutes {
            s.speedtest_reuse_minutes = v.min(1440);
        }
        if let Some(v) = input.speedtest_conditional_isolation {
            s.speedtest_conditional_isolation = v;
        }
        if let Some(v) = input.speedtest_download_top_n {
            s.speedtest_download_top_n = v.min(5000);
        }
        if let Some(v) = input.speedtest_dedup_egress {
            s.speedtest_dedup_egress = v;
        }
        if let Some(v) = input.speedtest_tcp_ping_only {
            s.speedtest_tcp_ping_only = v;
        }
        if let Some(v) = input.bandwidth_guard {
            s.bandwidth_guard = v;
        }
        if let Some(v) = input.bandwidth_min_kbps {
            s.bandwidth_min_kbps = v.min(100_000);
        }
        if let Some(v) = input.publisher_api {
            s.publisher_api = v.trim().to_string();
        }
        if let Some(v) = input.publisher_token {
            s.publisher_token = v.trim().to_string();
        }
        if let Some(v) = input.ui_theme {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.ui_theme = v;
            }
        }
        if let Some(v) = input.app_icon_style {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.app_icon_style = v;
            }
        }
        if let Some(v) = input.ui_language {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.ui_language = v;
            }
        }
        if let Some(v) = input.ui_scale {
            s.ui_scale = crate::models::clamp_ui_scale(v);
        }
        if let Some(v) = input.ui_dark {
            s.ui_dark = v;
        }
        if let Some(v) = input.sidebar_expanded {
            s.sidebar_expanded = v;
        }
        if let Some(v) = input.kernel_source {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.kernel_source = v;
            }
        }
        if let Some(v) = input.firewall_auto_allow {
            s.firewall_auto_allow = v;
        }
        if let Some(v) = input.auto_failover {
            s.auto_failover = v;
        }
        if let Some(v) = input.insider_channel {
            s.insider_channel = v;
        }
        if let Some(v) = input.speedtest_auto_refill {
            s.speedtest_auto_refill = v;
        }
        if let Some(v) = input.auto_rearm_system_proxy {
            s.auto_rearm_system_proxy = v;
        }
        if let Some(v) = input.auto_prepare_kernel {
            s.auto_prepare_kernel = v;
        }
        if let Some(v) = input.speedtest_added_source {
            s.speedtest_added_source = v;
        }
        if let Some(v) = input.gist_pages {
            s.gist_pages = v.clamp(1, 300);
        }
        if let Some(v) = input.gist_queries {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.gist_queries = v;
            }
        }
        if let Some(v) = input.window_size {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.window_size = v;
            }
        }
        if let Some(v) = input.console_tabs {
            let v = v.trim().to_string();
            if !v.is_empty() {
                s.console_tabs = v;
            }
        }
        sqlx::query(
            r#"UPDATE settings SET
                 auto_fetch_enabled=?,
                 auto_fetch_interval_min=?,
                 auto_speedtest_enabled=?,
                 fetch_concurrency=?,
                 fetch_timeout_secs=?,
                 speedtest_concurrency=?,
                 speedtest_timeout_ms=?,
                 speedtest_download=?,
                 download_test_ms=?,
                 http_server_enabled=?,
                 close_to_tray=?,
                 exclude_keywords=?,
                 failover_latency_ms=?,
                 speedtest_parallel_kernels=?,
                 speedtest_persistent_kernel=?,
                 speedtest_reuse_minutes=?,
                 speedtest_conditional_isolation=?,
                 speedtest_download_top_n=?,
                 speedtest_dedup_egress=?,
                 speedtest_tcp_ping_only=?,
                 bandwidth_guard=?,
                 bandwidth_min_kbps=?,
                 publisher_api=?,
                 publisher_token=?,
                 ui_theme=?,
                 app_icon_style=?,
                 ui_language=?,
                 ui_scale=?,
                 ui_dark=?,
                 sidebar_expanded=?,
                 kernel_source=?,
                 firewall_auto_allow=?,
                 auto_failover=?,
                 insider_channel=?,
                 speedtest_auto_refill=?,
                 auto_rearm_system_proxy=?,
                 auto_prepare_kernel=?,
                 speedtest_added_source=?,
                 gist_pages=?,
                 gist_queries=?,
                 window_size=?,
                 console_tabs=?
               WHERE id=1"#,
        )
        .bind(s.auto_fetch_enabled as i64)
        .bind(s.auto_fetch_interval_min as i64)
        .bind(s.auto_speedtest_enabled as i64)
        .bind(s.fetch_concurrency as i64)
        .bind(s.fetch_timeout_secs as i64)
        .bind(s.speedtest_concurrency as i64)
        .bind(s.speedtest_timeout_ms as i64)
        .bind(s.speedtest_download as i64)
        .bind(s.download_test_ms as i64)
        .bind(s.http_server_enabled as i64)
        .bind(s.close_to_tray as i64)
        .bind(&s.exclude_keywords)
        .bind(s.failover_latency_ms as i64)
        .bind(s.speedtest_parallel_kernels as i64)
        .bind(s.speedtest_persistent_kernel as i64)
        .bind(s.speedtest_reuse_minutes as i64)
        .bind(s.speedtest_conditional_isolation as i64)
        .bind(s.speedtest_download_top_n as i64)
        .bind(s.speedtest_dedup_egress as i64)
        .bind(s.speedtest_tcp_ping_only as i64)
        .bind(s.bandwidth_guard as i64)
        .bind(s.bandwidth_min_kbps as i64)
        .bind(&s.publisher_api)
        .bind(&s.publisher_token)
        .bind(&s.ui_theme)
        .bind(&s.app_icon_style)
        .bind(&s.ui_language)
        .bind(s.ui_scale as f64)
        .bind(s.ui_dark as i64)
        .bind(s.sidebar_expanded as i64)
        .bind(&s.kernel_source)
        .bind(s.firewall_auto_allow as i64)
        .bind(s.auto_failover as i64)
        .bind(s.insider_channel as i64)
        .bind(s.speedtest_auto_refill as i64)
        .bind(s.auto_rearm_system_proxy as i64)
        .bind(s.auto_prepare_kernel as i64)
        .bind(s.speedtest_added_source as i64)
        .bind(s.gist_pages as i64)
        .bind(&s.gist_queries)
        .bind(&s.window_size)
        .bind(&s.console_tabs)
        .execute(&self.pool)
        .await?;
        self.get_settings().await
    }

    pub async fn touch_auto_fetch(&self) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE settings SET last_auto_fetch_at=? WHERE id=1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn touch_auto_speedtest(&self) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE settings SET last_auto_speedtest_at=? WHERE id=1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

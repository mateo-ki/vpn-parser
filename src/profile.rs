//! 统一配置文件模型与导入/导出/备份引擎 (Unified Profile Schema & Migration Engine)
//!
//! 负责 VPN Parser 的全量配置归一化抽象、备份归档、无损导入与多端同步中间层。
//!
//! 架构设计：
//! 1. 订阅源集合 (Sources)
//! 2. 自定义与独立节点集合 (Custom Nodes - 协议归一化)
//! 3. 统一分流与路由策略 (Unified Routing IR / DNS / TUN / 模式)
//! 4. 软件全局偏好设置 (AppSettings)

use crate::db::{Database, UNGROUPED_SOURCE_ID};
use crate::models::{AppSettings, CreateSource, Node, Source};
use crate::proxy::types::MihomoConfigOptions;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// 统一 Profile Schema 规范版本号
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

/// VPN Parser 统一配置文件顶层结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedProfile {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_app_version")]
    pub app_version: String,
    #[serde(default = "Utc::now")]
    pub exported_at: DateTime<Utc>,
    #[serde(default)]
    pub profile_name: Option<String>,
    #[serde(default)]
    pub sources: Vec<ProfileSource>,
    #[serde(default)]
    pub custom_nodes: Vec<ProfileCustomNode>,
    #[serde(default)]
    pub routing: Option<MihomoConfigOptions>,
    #[serde(default)]
    pub settings: Option<AppSettings>,
}

fn default_schema_version() -> u32 {
    PROFILE_SCHEMA_VERSION
}

fn default_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn default_true() -> bool {
    true
}

/// 归一化订阅源定义
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSource {
    pub name: String,
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub rules_yaml: Option<String>,
    #[serde(default)]
    pub groups_yaml: Option<String>,
}

/// 归一化自建与手动节点定义
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileCustomNode {
    pub name: String,
    pub protocol: String,
    pub server: String,
    pub port: u16,
    #[serde(default)]
    pub raw: String,
    #[serde(default)]
    pub config: serde_json::Value,
}

/// 导入结果统计摘要
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub sources_added: usize,
    pub sources_updated: usize,
    pub custom_nodes_imported: usize,
    pub routing_updated: bool,
    pub settings_updated: bool,
}

/// 从当前数据库与状态中导出完整统一配置文件模型
pub async fn export_unified_profile(
    db: &Database,
    profile_name: Option<String>,
    include_settings: bool,
) -> Result<UnifiedProfile> {
    // 1. 订阅源
    let sources_raw = db.list_sources().await?;
    let sources = sources_raw
        .into_iter()
        .map(|s| ProfileSource {
            name: s.name,
            url: s.url_template,
            enabled: s.enabled,
            remark: s.remark,
            user_agent: s.user_agent,
            rules_yaml: s.rules_yaml,
            groups_yaml: s.groups_yaml,
        })
        .collect();

    // 2. 自建 / 未分组节点
    let custom_nodes_raw = db.load_nodes(Some(UNGROUPED_SOURCE_ID)).await.unwrap_or_default();
    let custom_nodes = custom_nodes_raw
        .into_iter()
        .map(|n| ProfileCustomNode {
            name: n.name,
            protocol: n.protocol,
            server: n.server,
            port: n.port,
            raw: n.raw,
            config: n.config,
        })
        .collect();

    // 3. 统一分流与路由策略
    let routing = db.get_proxy_config().await.ok();

    // 4. 首选项设置
    let settings = if include_settings {
        db.get_settings().await.ok()
    } else {
        None
    };

    Ok(UnifiedProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: Utc::now(),
        profile_name,
        sources,
        custom_nodes,
        routing,
        settings,
    })
}

/// 将 UnifiedProfile 序列化为规范的 YAML 文本
pub fn to_yaml(profile: &UnifiedProfile) -> Result<String> {
    serde_yaml::to_string(profile).context("序列化为 YAML 配置文件失败")
}

/// 将 UnifiedProfile 序列化为规范的 JSON 文本
pub fn to_json(profile: &UnifiedProfile) -> Result<String> {
    serde_json::to_string_pretty(profile).context("序列化为 JSON 配置文件失败")
}

/// 解析 YAML 或 JSON 文本为 UnifiedProfile
pub fn from_str(content: &str) -> Result<UnifiedProfile> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        anyhow::bail!("配置文件内容为空");
    }
    if trimmed.starts_with('{') {
        if let Ok(profile) = serde_json::from_str::<UnifiedProfile>(trimmed) {
            return Ok(profile);
        }
    }
    serde_yaml::from_str::<UnifiedProfile>(trimmed)
        .context("解析配置文件失败：请确保内容为有效的 YAML 或 JSON 格式且符合规范")
}

/// 保存备份文件到 local data/backups/ 目录
pub fn save_backup_file(yaml_content: &str) -> Result<PathBuf> {
    let dir = crate::data_root().join("backups");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("创建配置备份目录失败: {}", dir.display()))?;
    }
    let now = chrono::Local::now();
    let filename = format!("vpn-parser-profile-{}.yaml", now.format("%Y%m%d_%H%M%S"));
    let path = dir.join(filename);
    std::fs::write(&path, yaml_content)
        .with_context(|| format!("写入备份文件失败: {}", path.display()))?;
    Ok(path)
}

/// 将统一配置文件导入并应用到数据库
///
/// merge_mode = true: 增量合并，遇到已存在源更新信息，保留本地未在文件中的源；保留本地关键首选项
/// merge_mode = false: 覆盖模式，同步全量首选项与路由策略
pub async fn import_unified_profile(
    db: &Database,
    profile: &UnifiedProfile,
    merge_mode: bool,
) -> Result<ImportSummary> {
    let mut summary = ImportSummary::default();

    // 1. 处理订阅源
    let existing_sources = db.list_sources().await?;
    let mut existing_by_url: HashMap<String, Source> = HashMap::new();
    let mut existing_by_name: HashMap<String, Source> = HashMap::new();
    for s in existing_sources {
        existing_by_url.insert(s.url_template.clone(), s.clone());
        existing_by_name.insert(s.name.clone(), s);
    }

    for src in &profile.sources {
        if let Some(existing) = existing_by_url.get(&src.url).or_else(|| existing_by_name.get(&src.name)) {
            // 已存在：更新属性
            db.update_source(
                &existing.id,
                crate::models::UpdateSource {
                    name: Some(src.name.clone()),
                    url_template: Some(src.url.clone()),
                    enabled: Some(src.enabled),
                    remark: src.remark.clone(),
                    push_disabled: None,
                    user_agent: src.user_agent.clone(),
                },
            )
            .await?;
            if src.rules_yaml.is_some() || src.groups_yaml.is_some() {
                db.update_source_rules_and_groups(
                    &existing.id,
                    src.rules_yaml.as_deref(),
                    src.groups_yaml.as_deref(),
                )
                .await?;
            }
            summary.sources_updated += 1;
        } else {
            // 新增订阅源
            let created = db
                .create_source(CreateSource {
                    name: src.name.clone(),
                    url_template: src.url.clone(),
                    enabled: src.enabled,
                    remark: src.remark.clone(),
                    user_agent: src.user_agent.clone(),
                })
                .await?;
            if src.rules_yaml.is_some() || src.groups_yaml.is_some() {
                db.update_source_rules_and_groups(
                    &created.id,
                    src.rules_yaml.as_deref(),
                    src.groups_yaml.as_deref(),
                )
                .await?;
            }
            summary.sources_added += 1;
        }
    }

    // 2. 自定义 / 手动节点导入
    if !profile.custom_nodes.is_empty() {
        let nodes_to_import: Vec<Node> = profile
            .custom_nodes
            .iter()
            .map(|cn| {
                let now = Utc::now();
                Node {
                    id: uuid::Uuid::new_v4().to_string(),
                    source_id: None,
                    protocol: cn.protocol.clone(),
                    name: cn.name.clone(),
                    server: cn.server.clone(),
                    port: cn.port,
                    raw: cn.raw.clone(),
                    config: cn.config.clone(),
                    country: None,
                    latency_ms: None,
                    latency_ok: None,
                    download_kbps: None,
                    latency_kernel: None,
                    latency_test_kind: None,
                    latency_tested_at: None,
                    created_at: now,
                    updated_at: now,
                }
            })
            .collect();
        summary.custom_nodes_imported = db.import_nodes(&nodes_to_import).await?;
    }

    // 3. 路由分流与代理配置
    if let Some(ref routing) = profile.routing {
        db.update_proxy_config(routing).await?;
        summary.routing_updated = true;
    }

    // 4. 首选项设置（覆盖模式下同步偏好，或者显式指定时同步）
    if let Some(ref settings) = profile.settings {
        if !merge_mode {
            let _ = db
                .update_settings(crate::models::UpdateSettings {
                    auto_fetch_enabled: Some(settings.auto_fetch_enabled),
                    auto_fetch_interval_min: Some(settings.auto_fetch_interval_min),
                    auto_speedtest_enabled: Some(settings.auto_speedtest_enabled),
                    fetch_concurrency: Some(settings.fetch_concurrency),
                    fetch_timeout_secs: Some(settings.fetch_timeout_secs),
                    speedtest_concurrency: Some(settings.speedtest_concurrency),
                    speedtest_timeout_ms: Some(settings.speedtest_timeout_ms),
                    speedtest_download: Some(settings.speedtest_download),
                    download_test_ms: Some(settings.download_test_ms),
                    http_server_enabled: Some(settings.http_server_enabled),
                    close_to_tray: Some(settings.close_to_tray),
                    exclude_keywords: Some(settings.exclude_keywords.clone()),
                    failover_latency_ms: Some(settings.failover_latency_ms),
                    speedtest_parallel_kernels: Some(settings.speedtest_parallel_kernels),
                    speedtest_persistent_kernel: Some(settings.speedtest_persistent_kernel),
                    speedtest_reuse_minutes: Some(settings.speedtest_reuse_minutes),
                    speedtest_conditional_isolation: Some(settings.speedtest_conditional_isolation),
                    speedtest_download_top_n: Some(settings.speedtest_download_top_n),
                    speedtest_dedup_egress: Some(settings.speedtest_dedup_egress),
                    speedtest_tcp_ping_only: Some(settings.speedtest_tcp_ping_only),
                    bandwidth_guard: Some(settings.bandwidth_guard),
                    bandwidth_min_kbps: Some(settings.bandwidth_min_kbps),
                    publisher_api: Some(settings.publisher_api.clone()),
                    publisher_token: Some(settings.publisher_token.clone()),
                    ui_theme: Some(settings.ui_theme.clone()),
                    app_icon_style: Some(settings.app_icon_style.clone()),
                    ui_language: Some(settings.ui_language.clone()),
                    ui_scale: Some(settings.ui_scale),
                    ui_dark: Some(settings.ui_dark),
                    sidebar_expanded: Some(settings.sidebar_expanded),
                    kernel_source: Some(settings.kernel_source.clone()),
                    firewall_auto_allow: Some(settings.firewall_auto_allow),
                    auto_failover: Some(settings.auto_failover),
                    insider_channel: Some(settings.insider_channel),
                    speedtest_auto_refill: Some(settings.speedtest_auto_refill),
                    auto_prepare_kernel: Some(settings.auto_prepare_kernel),
                    launch_as_admin: Some(settings.launch_as_admin),
                    launch_at_startup: Some(settings.launch_at_startup),
                    speedtest_added_source: Some(settings.speedtest_added_source),
                    auto_rearm_system_proxy: Some(settings.auto_rearm_system_proxy),
                    gist_pages: Some(settings.gist_pages),
                    gist_queries: Some(settings.gist_queries.clone()),
                    window_size: Some(settings.window_size.clone()),
                    console_tabs: Some(settings.console_tabs.clone()),
                    dashboard_ui_mode: Some(settings.dashboard_ui_mode.clone()),
                })
                .await?;
            summary.settings_updated = true;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_serialize_and_deserialize_yaml() {
        let profile = UnifiedProfile {
            schema_version: 1,
            app_version: "1.15.5".to_string(),
            exported_at: Utc::now(),
            profile_name: Some("测试备份方案".to_string()),
            sources: vec![ProfileSource {
                name: "官方演示订阅".to_string(),
                url: "https://example.com/sub.yaml".to_string(),
                enabled: true,
                remark: Some("备用节点源".to_string()),
                user_agent: Some("clash.meta".to_string()),
                rules_yaml: None,
                groups_yaml: None,
            }],
            custom_nodes: vec![ProfileCustomNode {
                name: "自建-香港Hysteria2".to_string(),
                protocol: "hysteria2".to_string(),
                server: "1.2.3.4".to_string(),
                port: 443,
                raw: "hysteria2://passwd@1.2.3.4:443".to_string(),
                config: serde_json::json!({ "auth": "passwd" }),
            }],
            routing: None,
            settings: None,
        };

        let yaml = to_yaml(&profile).expect("导出 YAML");
        assert!(yaml.contains("官方演示订阅"));
        assert!(yaml.contains("自建-香港Hysteria2"));

        let restored = from_str(&yaml).expect("反序列化 YAML");
        assert_eq!(restored.schema_version, 1);
        assert_eq!(restored.sources.len(), 1);
        assert_eq!(restored.sources[0].name, "官方演示订阅");
        assert_eq!(restored.custom_nodes.len(), 1);
        assert_eq!(restored.custom_nodes[0].name, "自建-香港Hysteria2");
    }

    #[test]
    fn test_profile_json_interop() {
        let profile = UnifiedProfile {
            schema_version: 1,
            app_version: "1.15.5".to_string(),
            exported_at: Utc::now(),
            profile_name: Some("JSON格式测试".to_string()),
            sources: vec![],
            custom_nodes: vec![],
            routing: None,
            settings: None,
        };

        let json = to_json(&profile).expect("导出 JSON");
        let restored = from_str(&json).expect("解析 JSON");
        assert_eq!(restored.profile_name.as_deref(), Some("JSON格式测试"));
    }

    #[tokio::test]
    async fn test_database_profile_export_and_import_roundtrip() {
        let db_path_1 = std::env::temp_dir().join(format!("vpn-parser-profile-src-{}.db", uuid::Uuid::new_v4()));
        let db1 = Database::connect(db_path_1.to_str().unwrap()).await.unwrap();

        // 插入测试订阅源
        let src1 = db1.create_source(CreateSource {
            name: "主订阅-日本专线".to_string(),
            url_template: "https://example.com/jp.yaml".to_string(),
            enabled: true,
            remark: Some("速度最快".to_string()),
            user_agent: Some("clash.meta".to_string()),
        }).await.unwrap();

        db1.update_source_rules_and_groups(
            &src1.id,
            Some("- DOMAIN-SUFFIX,google.com,PROXY\n"),
            Some("- name: PROXY\n  type: select\n"),
        ).await.unwrap();

        // 插入自定义节点
        let custom_node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: None,
            protocol: "vless".to_string(),
            name: "自建-美国落地".to_string(),
            server: "8.8.8.8".to_string(),
            port: 443,
            raw: "vless://user@8.8.8.8:443".to_string(),
            config: serde_json::json!({ "uuid": "user" }),
            country: Some("US".to_string()),
            latency_ms: Some(150),
            latency_ok: Some(true),
            download_kbps: Some(5000.0),
            latency_kernel: Some("mihomo".to_string()),
            latency_test_kind: Some("google_proxy".to_string()),
            latency_tested_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db1.import_nodes(&[custom_node]).await.unwrap();

        // 导出统一配置
        let exported = export_unified_profile(&db1, Some("全量备份测试".to_string()), true)
            .await
            .expect("导出配置");
        assert_eq!(exported.sources.len(), 1);
        assert_eq!(exported.sources[0].name, "主订阅-日本专线");
        assert_eq!(exported.custom_nodes.len(), 1);
        assert_eq!(exported.custom_nodes[0].name, "自建-美国落地");

        // 序列化为 YAML 文本
        let yaml_text = to_yaml(&exported).expect("序列化 YAML");
        assert!(yaml_text.contains("主订阅-日本专线"));
        assert!(yaml_text.contains("自建-美国落地"));

        // 解析并恢复到新的空数据库 db2
        let db_path_2 = std::env::temp_dir().join(format!("vpn-parser-profile-dst-{}.db", uuid::Uuid::new_v4()));
        let db2 = Database::connect(db_path_2.to_str().unwrap()).await.unwrap();

        let parsed_profile = from_str(&yaml_text).expect("解析 YAML 文本");
        let summary = import_unified_profile(&db2, &parsed_profile, false)
            .await
            .expect("导入配置到目标库");

        assert_eq!(summary.sources_added, 1);
        assert_eq!(summary.custom_nodes_imported, 1);

        // 验证 db2 中的订阅源和节点
        let db2_sources = db2.list_sources().await.unwrap();
        assert_eq!(db2_sources.len(), 1);
        assert_eq!(db2_sources[0].name, "主订阅-日本专线");
        assert_eq!(db2_sources[0].url_template, "https://example.com/jp.yaml");
        assert_eq!(db2_sources[0].rules_yaml.as_deref(), Some("- DOMAIN-SUFFIX,google.com,PROXY\n"));

        let db2_nodes = db2.load_nodes(Some(UNGROUPED_SOURCE_ID)).await.unwrap();
        assert_eq!(db2_nodes.len(), 1);
        assert_eq!(db2_nodes[0].name, "自建-美国落地");
        assert_eq!(db2_nodes[0].protocol, "vless");

        // 清理临时文件
        drop(db1);
        drop(db2);
        let _ = std::fs::remove_file(&db_path_1);
        let _ = std::fs::remove_file(format!("{}-wal", db_path_1.display()));
        let _ = std::fs::remove_file(format!("{}-shm", db_path_1.display()));
        let _ = std::fs::remove_file(&db_path_2);
        let _ = std::fs::remove_file(format!("{}-wal", db_path_2.display()));
        let _ = std::fs::remove_file(format!("{}-shm", db_path_2.display()));
    }
}

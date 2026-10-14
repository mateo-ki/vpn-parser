use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A node passed a real proxy request to Google generate_204.
pub const GOOGLE_PROXY_TEST_KIND: &str = "google_proxy";

/// A subscription source that can be fetched periodically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    /// URL template. Supports placeholders: {Ymd}, {Y}, {m}, {mm}, {d}, {dd}, {ymd}
    pub url_template: String,
    pub enabled: bool,
    pub remark: Option<String>,
    pub last_fetch_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub last_node_count: i64,
    #[serde(default)]
    pub last_resolved_url: Option<String>,
    #[serde(default)]
    pub last_etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub content_sha256: Option<String>,
    /// Consecutive fetches that failed on the network layer (hang/timeout/reset).
    /// Used to stop spending the full retry budget on dead sources.
    #[serde(default)]
    pub fail_streak: i64,
    /// Host that served the last successful fetch, so the next round can start there.
    #[serde(default)]
    pub last_fetch_host: Option<String>,
    /// 该源的节点不参与“推送可用节点”（源本身仍照常拉取和测速）。
    #[serde(default)]
    pub push_disabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Source {
    /// 无库记录的临时源对象：只用于复用下载/解析路径（gist 发现等），不会写库。
    pub fn scratch(id: &str, name: &str, url_template: &str) -> Self {
        let now = Utc::now();
        Self {
            id: id.to_string(),
            name: name.to_string(),
            url_template: url_template.to_string(),
            enabled: true,
            remark: None,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSource {
    pub name: String,
    pub url_template: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub remark: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSource {
    pub name: Option<String>,
    pub url_template: Option<String>,
    pub enabled: Option<bool>,
    pub remark: Option<String>,
    /// 设为 Some 时切换该源是否参与“推送可用节点”。
    #[serde(default)]
    pub push_disabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

/// A parsed proxy node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub source_id: Option<String>,
    pub protocol: String,
    pub name: String,
    pub server: String,
    pub port: u16,
    /// Original share URI or raw config fragment.
    pub raw: String,
    /// Normalized JSON config for API consumers.
    pub config: serde_json::Value,
    pub country: Option<String>,
    /// TCP connect latency in milliseconds. None = not tested.
    pub latency_ms: Option<i64>,
    /// true=ok, false=timeout/fail, None=not tested
    pub latency_ok: Option<bool>,
    /// Download throughput in KB/s when measurable.
    pub download_kbps: Option<f64>,
    pub latency_kernel: Option<String>,
    pub latency_test_kind: Option<String>,
    pub latency_tested_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSummary {
    pub id: String,
    pub source_id: Option<String>,
    pub protocol: String,
    pub name: String,
    pub server: String,
    pub port: u16,
    pub country: Option<String>,
    pub latency_ms: Option<i64>,
    pub latency_ok: Option<bool>,
    pub download_kbps: Option<f64>,
    pub latency_kernel: Option<String>,
    pub latency_test_kind: Option<String>,
    pub latency_tested_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeGroupSummary {
    pub source_id: Option<String>,
    pub source_name: String,
    pub total: i64,
    pub alive: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub source_count: i64,
    pub enabled_source_count: i64,
    pub node_count: i64,
    pub tested_count: i64,
    pub alive_count: i64,
    pub protocol_stats: Vec<ProtocolStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolStat {
    pub protocol: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub source_id: String,
    pub source_name: String,
    pub resolved_url: String,
    pub success: bool,
    pub message: String,
    pub node_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyResult {
    pub id: String,
    pub name: String,
    pub server: String,
    pub port: u16,
    pub ok: bool,
    pub latency_ms: Option<i64>,
    /// Download speed in KB/s when measured.
    pub download_kbps: Option<f64>,
    pub latency_kernel: Option<String>,
    pub latency_test_kind: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedTestSummary {
    pub total: usize,
    pub ok: usize,
    pub fail: usize,
    pub avg_ms: Option<f64>,
    pub avg_download_kbps: Option<f64>,
    pub results: Vec<LatencyResult>,
}

/// App settings persisted in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub auto_fetch_enabled: bool,
    /// Interval in minutes (minimum 5).
    pub auto_fetch_interval_min: u32,
    pub auto_speedtest_enabled: bool,
    /// Concurrent subscription fetches.
    pub fetch_concurrency: u32,
    /// Per-source request deadline in seconds.
    pub fetch_timeout_secs: u32,
    /// Concurrent TCP probes.
    pub speedtest_concurrency: u32,
    /// TCP connect timeout in milliseconds.
    pub speedtest_timeout_ms: u32,
    /// Whether to measure download throughput during speedtest.
    pub speedtest_download: bool,
    /// Download probe duration in milliseconds.
    pub download_test_ms: u32,
    #[serde(default = "default_true")]
    pub http_server_enabled: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// 关键词黑名单（逗号/分号/空格/换行分隔）：命中的节点不作为连接候选，
    /// 也不参与推送与导出。
    #[serde(default)]
    pub exclude_keywords: String,
    /// 自动切换：当前节点真实测速延迟超过该值（ms）即视为劣化。实测走的是完整
    /// HTTPS 请求，境外节点正常也在 300-900ms，所以默认值要留足余量。
    #[serde(default = "default_failover_latency_ms")]
    pub failover_latency_ms: u32,
    /// 测速优化①：三个内核并行验证。默认关闭（串行更省内存，峰值内存约 1/3）。
    #[serde(default)]
    pub speedtest_parallel_kernels: bool,
    /// 测速优化②：复用一个常驻内核，用控制器接口切换待测节点。默认关闭
    /// （每批 256 个节点起一个新内核，隔离更彻底但要反复冷启动）。
    #[serde(default)]
    pub speedtest_persistent_kernel: bool,
    /// 测速优化③：N 分钟内已验证过的节点直接沿用结果，0 表示不启用。
    #[serde(default)]
    pub speedtest_reuse_minutes: u32,
    /// 测速优化④：只对"可疑"的失败节点做 `mihomo -t` 隔离复测。默认关闭（全部复测）。
    #[serde(default)]
    pub speedtest_conditional_isolation: bool,
    /// 测速优化⑤：下载测速只对延迟前 N 名做（0 = 所有通过节点都测）。
    #[serde(default)]
    pub speedtest_download_top_n: u32,
    /// 测速优化⑥：同物理落地节点剪枝与复用（默认开启）。
    #[serde(default = "default_true")]
    pub speedtest_dedup_egress: bool,
    /// 测速优化⑦：快速 TCP Ping 模式（仅测 TCP 连通性，跳过真实内核代理握手）。
    #[serde(default)]
    pub speedtest_tcp_ping_only: bool,
    /// 带宽保护（#5）：当前节点持续低于阈值 KB/s 时，交给选路调度器按劣化处理。
    #[serde(default)]
    pub bandwidth_guard: bool,
    #[serde(default)]
    pub bandwidth_min_kbps: u32,
    /// 发布/推送服务器地址与 token（空 = 未配置）。
    #[serde(default)]
    pub publisher_api: String,
    #[serde(default)]
    pub publisher_token: String,
    /// 界面配色主题 id（见 `gui::theme::THEMES`）。未知 id 会退回默认。
    #[serde(default = "default_ui_theme")]
    pub ui_theme: String,
    /// 字体/界面大小，1.0 = 100%。界面上绝大多数文字写死了 `.size(..)`，只改
    /// `Style::text_styles` 会一半大一半小，所以这条走 `pixels_per_point` 整屏一起缩。
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    /// 内核下载走哪条路：race / official / proxy / local（见 `kernel::KernelSource`）。
    #[serde(default = "default_kernel_source")]
    pub kernel_source: String,
    /// 启动软件时自动把自己和内核 exe 加进防火墙允许列表（默认关：它会改系统设置）。
    #[serde(default)]
    pub firewall_auto_allow: bool,
    /// 「节点故障自动切换」。以前它只是界面里的一个内存字段、重启就回 false，
    /// 用户以为开着而实际没开。默认仍关：不擅自改行为。
    #[serde(default)]
    pub auto_failover: bool,
    /// 参与预发：「关于 → 检查更新」除了稳定版，也看 GitHub 上标了 prerelease 的 beta。
    /// 默认关 —— 开着它等于同意拿可能改数据结构、没跑稳的安装包。
    #[serde(default)]
    pub insider_channel: bool,
    /// 测通的节点太少时，后台自动补一轮全量测速（默认关）。
    /// 自动切换要找替补就是从"测通"这批里捞，这批空掉故障转移就形同虚设。
    #[serde(default)]
    pub speedtest_auto_refill: bool,
    /// 发现系统代理被外部改掉就自己写回去（默认关：动的是用户的 HKCU）。
    pub auto_rearm_system_proxy: bool,
    /// 「预备」状态下自动把缺的内核下载下来。这条**默认开**：新装的机器上第一次启动
    /// 就该自己动起来，关了就等于没有这个状态（用户 2026-10-08 指定）。
    #[serde(default = "default_true_setting")]
    pub auto_prepare_kernel: bool,
    /// 新增订阅源后自动拉取它并只测这个源。同样默认开：他要的是"加完就知道好不好用"。
    #[serde(default = "default_true_setting")]
    pub speedtest_added_source: bool,
    pub last_auto_fetch_at: Option<DateTime<Utc>>,
    pub last_auto_speedtest_at: Option<DateTime<Utc>>,
}

/// 两个"软件自己动"的开关用的默认值：默认开，但都能关。
fn default_true_setting() -> bool {
    true
}

fn default_failover_latency_ms() -> u32 {
    1500
}

fn default_kernel_source() -> String {
    // 2026-10-14 实测换到 ghfast：gh-proxy 那家会在 18.6/22.5 MB 处把正文掐断
    // （新装的机器上就是"预备 · 下载失败"），ghfast 同一次 36.8 秒下完且 SHA-256 对得上。
    crate::kernel::KernelSource::GhFast.id().to_string()
}

fn default_ui_theme() -> String {
    // 字面量：models 不能反向依赖 gui（gui 是 feature-gated 的）。
    // gui::theme::DEFAULT_THEME 必须和这里一致，`theme::tests::models_default_theme_matches` 守着。
    "soft-cobalt".to_string()
}

/// 字体/界面大小的默认值：1.0 = 不缩放。老库里没这一列时（`ensure_column` 给的也是 1.0）
/// 行为要和升级前完全一致。
fn default_ui_scale() -> f32 {
    1.0
}

/// 允许的字体/界面缩放区间。界面上是 85%~140% 的滑杆，但 API/旧库里的值不一定落在
/// 这个范围里 —— 越界的值会让窗口内容算成 0 高或撑到屏外，所以只在这一处收口。
pub fn clamp_ui_scale(value: f32) -> f32 {
    if !value.is_finite() {
        return default_ui_scale();
    }
    value.clamp(0.85, 1.40)
}

/// 节点名是否命中排除关键词（大小写不敏感）。
pub fn name_excluded(name: &str, keywords: &str) -> bool {
    let lower = name.to_lowercase();
    keywords
        .split([',', '，', ';', '；', ' ', '\n', '\r', '\t'])
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty())
        .any(|keyword| lower.contains(&keyword.to_lowercase()))
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_fetch_enabled: false,
            auto_fetch_interval_min: 60,
            auto_speedtest_enabled: false,
            fetch_concurrency: 4,
            fetch_timeout_secs: 30,
            speedtest_concurrency: 128,
            speedtest_timeout_ms: 1500,
            speedtest_download: false,
            download_test_ms: 800,
            http_server_enabled: false,
            close_to_tray: true,
            exclude_keywords: String::new(),
            failover_latency_ms: default_failover_latency_ms(),
            speedtest_parallel_kernels: false,
            speedtest_persistent_kernel: false,
            speedtest_reuse_minutes: 0,
            speedtest_conditional_isolation: false,
            speedtest_download_top_n: 0,
            speedtest_dedup_egress: true,
            speedtest_tcp_ping_only: false,
            bandwidth_guard: false,
            bandwidth_min_kbps: 0,
            publisher_api: String::new(),
            publisher_token: String::new(),
            ui_theme: default_ui_theme(),
            ui_scale: default_ui_scale(),
            kernel_source: default_kernel_source(),
            firewall_auto_allow: false,
            auto_failover: false,
            insider_channel: false,
            speedtest_auto_refill: false,
            auto_rearm_system_proxy: false,
            auto_prepare_kernel: true,
            speedtest_added_source: true,
            last_auto_fetch_at: None,
            last_auto_speedtest_at: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UpdateSettings {
    /// 全是 Option 的部分更新结构：`Default` 给测试和"只改一项"的调用用。
    pub auto_fetch_enabled: Option<bool>,
    pub auto_fetch_interval_min: Option<u32>,
    pub auto_speedtest_enabled: Option<bool>,
    pub fetch_concurrency: Option<u32>,
    pub fetch_timeout_secs: Option<u32>,
    pub speedtest_concurrency: Option<u32>,
    pub speedtest_timeout_ms: Option<u32>,
    pub speedtest_download: Option<bool>,
    pub download_test_ms: Option<u32>,
    pub http_server_enabled: Option<bool>,
    pub close_to_tray: Option<bool>,
    pub exclude_keywords: Option<String>,
    pub failover_latency_ms: Option<u32>,
    pub speedtest_parallel_kernels: Option<bool>,
    pub speedtest_persistent_kernel: Option<bool>,
    pub speedtest_reuse_minutes: Option<u32>,
    pub speedtest_conditional_isolation: Option<bool>,
    pub speedtest_download_top_n: Option<u32>,
    pub speedtest_dedup_egress: Option<bool>,
    pub speedtest_tcp_ping_only: Option<bool>,
    pub bandwidth_guard: Option<bool>,
    pub bandwidth_min_kbps: Option<u32>,
    pub publisher_api: Option<String>,
    pub publisher_token: Option<String>,
    pub ui_theme: Option<String>,
    pub ui_scale: Option<f32>,
    pub kernel_source: Option<String>,
    pub firewall_auto_allow: Option<bool>,
    pub auto_failover: Option<bool>,
    pub insider_channel: Option<bool>,
    pub speedtest_auto_refill: Option<bool>,
    pub auto_rearm_system_proxy: Option<bool>,
    pub auto_prepare_kernel: Option<bool>,
    pub speedtest_added_source: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeQuery {
    pub protocol: Option<String>,
    pub source_id: Option<String>,
    pub keyword: Option<String>,
    /// alive | dead | untested | all
    pub latency: Option<String>,
    pub country: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    /// updated | latency | name | download
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
    pub items: Vec<T>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            ok: true,
            message: "ok".into(),
            data: Some(data),
        }
    }

    pub fn err(message: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            ok: false,
            message: message.into(),
            data: None,
        }
    }
}

/// 订阅源同质化与重合度分析报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceHomogeneityReport {
    pub source_id: String,
    pub source_name: String,
    pub total_nodes: usize,
    pub alive_nodes: usize,
    pub duplicate_nodes: usize,
    pub unique_nodes: usize,
    pub unique_alive_nodes: usize,
    pub overlap_rate_pct: f32,
    pub is_redundant: bool,
}

/// 反向剔除同质化源的执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneHomogeneityResult {
    pub pruned_count: usize,
    pub affected_names: Vec<String>,
    pub deleted_records: bool,
}

#[cfg(test)]
mod tests {
    use super::name_excluded;

    #[test]
    fn exclude_keywords_match_case_insensitively_on_substrings() {
        let keywords = "到期，限速 ; ×3, 未验证";
        assert!(name_excluded("HK-到期-01", keywords));
        assert!(name_excluded("【限速】JP 节点", keywords));
        assert!(name_excluded("未验证节点", keywords));
        assert!(!name_excluded("HK 普通节点 01", keywords));
        // 空关键词列表不排除任何东西
        assert!(!name_excluded("任意节点", ""));
        assert!(!name_excluded("任意节点", " , ； "));
        // 大小写不敏感
        assert!(name_excluded("US-TEST-01", "test"));
        assert!(name_excluded("us-test-01", "TEST"));
    }
}

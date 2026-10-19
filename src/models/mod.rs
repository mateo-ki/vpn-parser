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
    /// 应用主图标风格："classic"（原版黑曜石白V）/ "champagne"（流光香槟金）
    #[serde(default = "default_app_icon_style")]
    pub app_icon_style: String,
    /// 界面语言代码（见 `i18n::Language`），默认 "zh-CN"。
    #[serde(default = "default_ui_language")]
    pub ui_language: String,
    /// 字体/界面大小，1.0 = 100%。界面上绝大多数文字写死了 `.size(..)`，只改
    /// `Style::text_styles` 会一半大一半小，所以这条走 `pixels_per_point` 整屏一起缩。
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    /// 夜间模式：中性色整套换成深色锚点，和 `ui_theme`（强调色）是两条正交的轴。
    /// **默认开**（用户 2026-10-15 指定）：新装和老库升级上来第一眼就是夜间，
    /// 不想要就在设置页「外观」里关掉。这条推翻了他自己 2026-10-06 那句
    /// "只留清爽浅白" —— 那次是删深色主题条目，这次是把夜间做成默认，按新的走。
    #[serde(default = "default_ui_dark")]
    pub ui_dark: bool,
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
    /// Gist 发现爬取页数，默认 100。
    #[serde(default = "default_gist_pages")]
    pub gist_pages: u32,
    /// Gist 发现已勾选的协议查询词（逗号分隔），默认 "vless//"
    #[serde(default = "default_gist_queries")]
    pub gist_queries: String,
    /// 窗口大小预设：small (1024x680) / medium (1280x800) / large (1440x880)
    #[serde(default = "default_window_size")]
    pub window_size: String,
    /// 控制台 Tab 显示配置（逗号分隔的 panel id，"all" 为全显）
    #[serde(default = "default_console_tabs")]
    pub console_tabs: String,
    pub last_auto_fetch_at: Option<DateTime<Utc>>,
    pub last_auto_speedtest_at: Option<DateTime<Utc>>,
}

fn default_window_size() -> String {
    "small".to_string()
}

fn default_console_tabs() -> String {
    "all".to_string()
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
    "theme-champagne".to_string()
}

fn default_app_icon_style() -> String {
    "classic".to_string()
}

fn default_ui_language() -> String {
    // 环境变量优先检测（AppImageHub CI 在 C/en_US 环境运行，非中文环境默认回退英文）
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_lowercase();
            if lower.starts_with("zh_tw") || lower.starts_with("zh_hk") || lower.contains("hant") {
                return "zh-TW".to_string();
            }
            if lower.starts_with("zh") {
                return "zh-CN".to_string();
            }
            if lower.starts_with("ja") {
                return "ja-JP".to_string();
            }
            if lower.starts_with("en") || lower == "c" || lower == "posix" {
                return "en-US".to_string();
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        "zh-CN".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        "en-US".to_string()
    }
}

/// 字体/界面大小的默认值：1.0 = 不缩放。老库里没这一列时（`ensure_column` 给的也是 1.0）
/// 行为要和升级前完全一致。
fn default_ui_scale() -> f32 {
    1.0
}

fn default_ui_dark() -> bool {
    true
}

fn default_gist_pages() -> u32 {
    100
}

fn default_gist_queries() -> String {
    "vless//".to_string()
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
            speedtest_concurrency: 64,
            speedtest_timeout_ms: 2000,
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
            app_icon_style: default_app_icon_style(),
            ui_language: default_ui_language(),
            ui_scale: default_ui_scale(),
            ui_dark: true,
            kernel_source: default_kernel_source(),
            firewall_auto_allow: false,
            auto_failover: false,
            insider_channel: false,
            speedtest_auto_refill: false,
            auto_rearm_system_proxy: false,
            auto_prepare_kernel: true,
            speedtest_added_source: true,
            gist_pages: default_gist_pages(),
            gist_queries: default_gist_queries(),
            window_size: default_window_size(),
            console_tabs: default_console_tabs(),
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
    pub app_icon_style: Option<String>,
    pub ui_language: Option<String>,
    pub ui_scale: Option<f32>,
    pub ui_dark: Option<bool>,
    pub kernel_source: Option<String>,
    pub firewall_auto_allow: Option<bool>,
    pub auto_failover: Option<bool>,
    pub insider_channel: Option<bool>,
    pub speedtest_auto_refill: Option<bool>,
    pub auto_rearm_system_proxy: Option<bool>,
    pub auto_prepare_kernel: Option<bool>,
    pub speedtest_added_source: Option<bool>,
    pub gist_pages: Option<u32>,
    pub gist_queries: Option<String>,
    pub window_size: Option<String>,
    pub console_tabs: Option<String>,
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

/// 「判定建议」那一列的两态。判定本身在数据层（`db::source_homogeneity_analysis`），
/// 这里只负责把它翻成界面话术 —— 特别是**0 节点的源**：它不是"核心保留"（它没有东西可
/// 保留），也不该被"重合率"那套话术盖过去，所以单独一句"0 节点"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomogeneityVerdict {
    /// 剔除候选：重合到底了，或者干脆一个节点都没有。
    Redundant,
    /// 有留下来的理由。
    Core,
}

impl SourceHomogeneityReport {
    pub fn verdict(&self) -> HomogeneityVerdict {
        if self.is_redundant {
            HomogeneityVerdict::Redundant
        } else {
            HomogeneityVerdict::Core
        }
    }

    /// 剔除候选分两种说法：重合掉到底 vs 空得什么都没贡献。
    pub fn verdict_label(&self) -> &'static str {
        match self.verdict() {
            HomogeneityVerdict::Redundant if self.total_nodes == 0 => "⚠️ 建议剔除 · 0 节点",
            HomogeneityVerdict::Redundant => "⚠️ 建议剔除",
            HomogeneityVerdict::Core => "✓ 核心保留",
        }
    }
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
#[cfg(test)]
mod homogeneity_verdict_tests {
    use crate::models::{HomogeneityVerdict, SourceHomogeneityReport};

    fn report(total_nodes: usize, is_redundant: bool) -> SourceHomogeneityReport {
        SourceHomogeneityReport {
            source_id: "s".into(),
            source_name: "源".into(),
            total_nodes,
            alive_nodes: 0,
            duplicate_nodes: 0,
            unique_nodes: 0,
            unique_alive_nodes: 0,
            overlap_rate_pct: 0.0,
            is_redundant,
        }
    }

    /// 判定只看数据层给的那个 `is_redundant`，界面不许自己另起一套 —— 不然就会出现
    /// "徽章说剔除、『仅勾选冗余源』却不勾它"的半改状态。空源单独说一句理由是"空"。
    #[test]
    fn an_empty_source_is_never_marked_core() {
        assert_eq!(report(0, true).verdict(), HomogeneityVerdict::Redundant);
        assert!(
            report(0, true).verdict_label().contains("0 节点"),
            "空源的文案没讲清楚理由：{}",
            report(0, true).verdict_label()
        );
        assert_eq!(report(12, true).verdict(), HomogeneityVerdict::Redundant);
        assert_eq!(report(12, false).verdict(), HomogeneityVerdict::Core);
        assert_ne!(
            report(12, true).verdict_label(),
            report(0, true).verdict_label(),
            "重合掉的源和空源用同一句话，界面就看不出为什么要删它"
        );
    }
}

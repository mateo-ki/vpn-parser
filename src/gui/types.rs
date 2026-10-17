use crate::kernel::{KernelInfo, KernelReleaseInfo, KernelUpdateProgress};
use crate::models::{
    AppSettings, FetchResult, Node, NodeGroupSummary, NodeSummary, PruneHomogeneityResult, Source,
    SourceHomogeneityReport, SpeedTestSummary, Stats,
};
use crate::proxy::{MihomoConfigOptions, NetworkProtocol, ProxyRuntimeStatus, RuleMatcher};
use crate::speedtest::SpeedTestProgress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sources,
    Nodes,
    Proxy,
    Settings,
    Api,
    About,
}

/// 系统设置页的分区标签：通用设置、界面外观、订阅与测速、自愈网络、外部接口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Appearance,
    Automation,
    Network,
    Api,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourcesPanel {
    List,
    Results,
}

/// 订阅列表的「状态」筛选。每个选项都对得上表格里**已经画出来**的那一格，不发明第三种状态：
/// 启用/停用 = 状态列那枚徽章，拉取失败 = 同一列那行状态文字（判据就是列表里"重试"按钮
/// 用的 `source_status_failed`），无节点 = 健康列的「无节点」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceStatusFilter {
    All,
    Enabled,
    Disabled,
    Failed,
    NoNodes,
}

impl SourceStatusFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Enabled,
        Self::Disabled,
        Self::Failed,
        Self::NoNodes,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyPanel {
    #[default]
    Charts,
    Nodes,
    Groups,
    Connections,
    Rules,
    Network,
    Kernels,
    Logs,
}

/// 分流策略组面板里的两个分区：视图（策略组实时状态与成员拓扑）和配置（自定义配置覆写 YAML 片段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupsTab {
    #[default]
    View,
    Config,
}

/// 分流规则面板里的两个分区：规则本身（该怎么走）和实际命中记录（真走了哪）。
/// 以前它们挤在同一屏里，规则表一长就看不到命中，反之命中一多就翻不到规则。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RulesTab {
    #[default]
    Config,
    Hits,
}

/// 活跃连接监控面板里的三个分区：连接列表、连接进程、DNS 记录。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionsTab {
    #[default]
    List,
    Processes,
    Dns,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct DnsRecord {
    pub id: String,
    pub host: String,
    pub ip: String,
    pub port: String,
    pub process: String,
    pub rule: String,
    pub target: String,
    pub network: String,
    pub time_str: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProxySortBy {
    Latency,
    Name,
}

/// 控制台"可用节点"网格的一行：把每帧都要重算的东西（国家猜测、小写名）预先算好。
#[derive(Clone)]
#[allow(dead_code)]
pub struct ProxyViewRow {
    pub index: usize,
    pub country: String,
    pub lower_name: String,
}

/// 过滤 + 排序后的网格视图，按 (数据版本, 策略组, 地区, 搜索词, 排序) 缓存。
pub struct ProxyView {
    pub rev: u64,
    pub group: String,
    pub region: String,
    pub search: String,
    pub sort: ProxySortBy,
    pub rows: Vec<ProxyViewRow>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RuleDraftKind {
    Domain,
    DomainSuffix,
    GeoSite,
    GeoIp,
    IpCidr,
    DestinationPort,
    ProcessName,
    RuleSet,
    Tcp,
    Udp,
    Quic,
}

impl RuleDraftKind {
    pub const ALL: [Self; 11] = [
        Self::Domain,
        Self::DomainSuffix,
        Self::GeoSite,
        Self::GeoIp,
        Self::IpCidr,
        Self::DestinationPort,
        Self::ProcessName,
        Self::RuleSet,
        Self::Tcp,
        Self::Udp,
        Self::Quic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Domain => "域名",
            Self::DomainSuffix => "域名后缀",
            Self::GeoSite => "GeoSite",
            Self::GeoIp => "GeoIP",
            Self::IpCidr => "IP-CIDR",
            Self::DestinationPort => "目标端口",
            Self::ProcessName => "进程名",
            Self::RuleSet => "规则集",
            Self::Tcp => "TCP",
            Self::Udp => "UDP",
            Self::Quic => "QUIC",
        }
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, Self::Tcp | Self::Udp | Self::Quic)
    }

    pub fn matcher(self, value: &str) -> RuleMatcher {
        match self {
            Self::Domain => RuleMatcher::Domain(value.trim().into()),
            Self::DomainSuffix => RuleMatcher::DomainSuffix(value.trim().into()),
            Self::GeoSite => RuleMatcher::GeoSite(value.trim().into()),
            Self::GeoIp => RuleMatcher::GeoIp(value.trim().into()),
            Self::IpCidr => RuleMatcher::IpCidr(value.trim().into()),
            Self::DestinationPort => RuleMatcher::DestinationPort(value.trim().into()),
            Self::ProcessName => RuleMatcher::ProcessName(value.trim().into()),
            Self::RuleSet => RuleMatcher::RuleSet(value.trim().into()),
            Self::Tcp => RuleMatcher::Network(NetworkProtocol::Tcp),
            Self::Udp => RuleMatcher::Network(NetworkProtocol::Udp),
            Self::Quic => RuleMatcher::Quic,
        }
    }
}

pub enum BgMsg {
    Sources(Result<Vec<Source>, String>),
    /// 节点落地 IP 的机房/风控画像。
    IpRisk(Result<crate::ip_risk::IpRisk, String>),
    /// 出口 IP 画像，第一个字段是它属于哪个节点（自动查=当前运行节点，手动=被实测的节点）。
    IpEgress(String, Result<crate::ip_risk::IpRisk, String>),
    Nodes(Result<(Vec<NodeSummary>, i64), String>),
    SourceModalNodes {
        source_id: String,
        result: Result<(Vec<NodeSummary>, i64), String>,
    },
    NodeGroups(Result<Vec<NodeGroupSummary>, String>),
    ProxyNodeGroups(Result<Vec<NodeGroupSummary>, String>),
    ProxyNodes(Result<Vec<NodeSummary>, String>),
    /// 防火墙允许规则的写入结果（启动时自动、或弹框里手动点「现在加入」）。
    FirewallApplied(Result<String, String>),
    /// 自动选路模式下替用户挑一个锚点节点（id, name）：左键点卡片不再选节点，
    /// 若仍要求"先有节点才能启动"，启动代理就永远点不动。
    AutoAnchorStart(Result<(String, String), String>),
    Stats(Result<Stats, String>),
    Settings(Result<AppSettings, String>),
    SettingsSaved(Result<AppSettings, String>),
    /// 发布服务器地址 / 推送 token 单独落库的结果。
    PublisherSaved(Result<(), String>),
    FetchAllDone(Result<Vec<FetchResult>, String>),
    /// 自动切换的健康探测结果（经当前运行中的代理实测）。
    FailoverProbe {
        node_id: String,
        node_name: String,
        ok: bool,
        latency_ms: u64,
    },
    /// 落库后的出口健康：最近 24 小时的实测 + 当前连续失败次数。
    EgressHealth(Vec<crate::db::telemetry::HealthRow>, i64),
    /// 出口切换历史：最近的若干条 + 今天换了几次 + 库里还留着多少条。
    EgressSwitches(Vec<crate::db::telemetry::SwitchRow>, i64, i64),
    /// 只要两个计数（今日几次 / 库里共几条）：控制台那行话得在没打开窗口时也是真的。
    SwitchCounts(i64, i64),
    /// 刚创建的订阅源（id, 名称）：开启「加完就测」时由这条串起 拉取 -> 测速。
    SourceAdded(String, String),
    /// 新增源拉取成功后取回它的节点 id 列表，齐了就能发起测速。
    AddedSourceIds {
        source_id: String,
        source_name: String,
        ids: Result<Vec<String>, String>,
    },
    /// 系统代理核对结果：注册表里那份还指着我们吗（见 `system_proxy::classify_drift`）。
    ProxyDrift(crate::system_proxy::Drift),
    /// 今天的规则命中聚合。
    RuleHits(Vec<crate::db::telemetry::RuleHitRow>),
    /// 所有源最近的拉取历史（一次全取，界面自己按 source_id 分桶）。
    FetchHistory(Vec<(String, crate::db::telemetry::FetchHistoryRow)>),
    Status(String),
    GistDiscoveryDone(Result<String, String>),
    FetchOneDone {
        source_id: String,
        result: Result<FetchResult, String>,
    },
    FetchProgress {
        done: usize,
        total: usize,
    },
    SpeedProgress(SpeedTestProgress),
    SourceSpeedProgress {
        done: usize,
        total: usize,
    },
    SpeedDone(Result<SpeedTestSummary, String>),
    /// 自动切换的替补 TCP 复检结果。
    FailoverTarget(Result<(String, String), String>),
    Kernels(Vec<KernelInfo>),
    KernelReleases(Result<Vec<KernelReleaseInfo>, String>),
    KernelProgress(KernelUpdateProgress),
    KernelUpdated(Result<String, String>),
    /// 应用自身更新（「关于」页）：查版本、下载进度、下载结果（安装包本地路径）。
    AppUpdateCheck(Result<crate::app_update::AppReleaseInfo, String>),
    AppUpdateProgress(crate::app_update::AppUpdateProgress),
    AppUpdateDownloaded(Result<std::path::PathBuf, String>),
    ProxyStatus(ProxyRuntimeStatus),
    ProxyConfig(Result<MihomoConfigOptions, String>),
    ProxyConfigSaved(Result<MihomoConfigOptions, String>),
    ProxyLog(Result<String, String>),
    ProxyOp(Result<String, String>),
    ApiServerStatus(Result<String, String>),
    ProxySwitchDone {
        seq: u64,
        node_id: String,
        result: Result<String, String>,
    },
    RemoteApiCheck(Result<String, String>),
    PublishDone(Result<String, String>),
    OpDone {
        result: Result<String, String>,
        unlock: bool,
    },
    NodeDetail(Result<Node, String>),
    TrafficSnapshot {
        now: std::time::Instant,
        upload_total: u64,
        download_total: u64,
        connections: Vec<ActiveConnectionInfo>,
    },
    TrafficPollDone,
    /// 内核 proxy-groups 的实际状态 + 各成员最近一次实测延迟。
    KernelGroups(Vec<KernelGroup>, Vec<KernelLatency>),
    ConnectionClosed(String),
    AllConnectionsClosed,
    GeoUpdated(Result<String, String>),
    SourceHomogeneityAnalyzed(Result<Vec<SourceHomogeneityReport>, String>),
    SourceHomogeneityPruned(Result<PruneHomogeneityResult, String>),
}

/// 命令面板（Ctrl+K）能执行的动作。
#[derive(Debug, Clone)]
pub enum PaletteAction {
    Goto(Tab),
    GotoProxyPanel(ProxyPanel),
    ToggleProxy,
    ToggleMode,
    Theme(&'static str),
    DownloadKernel,
    UseNode(String, String),
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub title: String,
    pub hint: String,
    pub action: PaletteAction,
}

#[derive(Debug, Clone, Default)]
pub struct ActiveConnectionInfo {
    pub id: String,
    pub network: String,
    pub host: String,
    pub destination: String,
    pub process: String,
    /// 命中的规则，已把目标拼成 `GEOIP,CN,DIRECT(DIRECT)` 这种可读形式（列表和筛选都用它）。
    pub rule: String,
    /// 规则把这条连接派给了谁（组名 / DIRECT）。命中统计按 (rule, target) 计数。
    pub target: String,
    pub upload: u64,
    pub download: u64,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct KernelGroup {
    pub name: String,
    /// select / url-test（mihomo 返回的是 Selector / URLTest，这里已归一化）。
    pub kind: String,
    pub now: Option<String>,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct MihomoProxiesResponse {
    #[serde(default)]
    pub(crate) proxies: std::collections::HashMap<String, MihomoProxyItem>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct MihomoProxyItem {
    #[serde(rename = "type", default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) now: Option<String>,
    #[serde(default)]
    pub(crate) all: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) alive: Option<bool>,
    /// 内核自己测的最近几次延迟，**按时间从旧到新**排列（实测 06:30→06:35→06:40）。
    #[serde(default)]
    pub(crate) history: Option<Vec<MihomoDelayEntry>>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct MihomoDelayEntry {
    #[serde(default)]
    pub(crate) delay: u64,
}

/// 内核 url-test 成员的一拍延迟读数（省掉 app 层重复探测）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct KernelLatency {
    pub name: String,
    pub delay_ms: u64,
    pub alive: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct MihomoConnectionsResponse {
    #[serde(rename = "downloadTotal", default)]
    pub(crate) download_total: u64,
    #[serde(rename = "uploadTotal", default)]
    pub(crate) upload_total: u64,
    #[serde(default)]
    pub(crate) connections: Vec<MihomoConnectionItem>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct MihomoConnectionItem {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) metadata: Option<MihomoMetadata>,
    #[serde(default)]
    pub(crate) upload: u64,
    #[serde(default)]
    pub(crate) download: u64,
    #[serde(default)]
    pub(crate) rule: Option<String>,
    #[serde(rename = "rulePayload", default)]
    pub(crate) rule_payload: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct MihomoMetadata {
    #[serde(default)]
    pub(crate) network: Option<String>,
    #[serde(default)]
    pub(crate) host: Option<String>,
    #[serde(rename = "destinationIP", default)]
    pub(crate) destination_ip: Option<String>,
    #[serde(rename = "destinationPort", default)]
    pub(crate) destination_port: Option<String>,
    #[serde(default)]
    pub(crate) process: Option<String>,
    #[serde(rename = "processPath", default)]
    pub(crate) process_path: Option<String>,
}

/// Gist 发现默认搜索词（q 参数会被 percent-encode）。

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsEditState {
    pub auto_fetch: bool,
    pub interval_min: u32,
    pub auto_speed: bool,
    pub fetch_concurrency: u32,
    pub fetch_timeout_secs: u32,
    pub speedtest_concurrency: u32,
    pub timeout_ms: u32,
    pub speedtest_download: bool,
    pub download_test_ms: u32,
    pub http_server_enabled: bool,
    pub close_to_tray: bool,
    pub exclude_keywords: String,
    pub failover_latency_ms: u32,
    pub speedtest_parallel_kernels: bool,
    pub speedtest_persistent_kernel: bool,
    pub speedtest_reuse_minutes: u32,
    pub speedtest_conditional_isolation: bool,
    pub speedtest_download_top_n: u32,
    pub speedtest_dedup_egress: bool,
    pub speedtest_tcp_ping_only: bool,
    pub speedtest_auto_refill: bool,
    pub auto_rearm_system_proxy: bool,
    pub auto_prepare_kernel: bool,
    pub speedtest_added_source: bool,
    pub bandwidth_guard: bool,
    pub bandwidth_min_kbps: u32,
    pub ui_theme: String,
    pub ui_language: String,
    pub ui_scale: f32,
    pub ui_dark: bool,
    /// 参与预发：检查更新时也看 GitHub 上标了 prerelease 的 beta（在「关于」页里勾）。
    pub insider_channel: bool,
    pub kernel_source: String,
    pub firewall_auto_allow: bool,
}

impl SettingsEditState {
    pub fn from_settings(s: &AppSettings) -> Self {
        Self {
            auto_fetch: s.auto_fetch_enabled,
            interval_min: s.auto_fetch_interval_min,
            auto_speed: s.auto_speedtest_enabled,
            fetch_concurrency: s.fetch_concurrency.clamp(1, 8),
            fetch_timeout_secs: s.fetch_timeout_secs,
            speedtest_concurrency: s.speedtest_concurrency,
            timeout_ms: s.speedtest_timeout_ms,
            speedtest_download: s.speedtest_download,
            download_test_ms: s.download_test_ms,
            http_server_enabled: s.http_server_enabled,
            close_to_tray: s.close_to_tray,
            exclude_keywords: s.exclude_keywords.clone(),
            failover_latency_ms: s.failover_latency_ms,
            speedtest_parallel_kernels: s.speedtest_parallel_kernels,
            speedtest_persistent_kernel: s.speedtest_persistent_kernel,
            speedtest_reuse_minutes: s.speedtest_reuse_minutes,
            speedtest_conditional_isolation: s.speedtest_conditional_isolation,
            speedtest_download_top_n: s.speedtest_download_top_n,
            speedtest_dedup_egress: s.speedtest_dedup_egress,
            speedtest_tcp_ping_only: s.speedtest_tcp_ping_only,
            speedtest_auto_refill: s.speedtest_auto_refill,
            auto_rearm_system_proxy: s.auto_rearm_system_proxy,
            auto_prepare_kernel: s.auto_prepare_kernel,
            speedtest_added_source: s.speedtest_added_source,
            bandwidth_guard: s.bandwidth_guard,
            bandwidth_min_kbps: s.bandwidth_min_kbps,
            ui_theme: s.ui_theme.clone(),
            ui_language: s.ui_language.clone(),
            ui_scale: s.ui_scale,
            ui_dark: s.ui_dark,
            insider_channel: s.insider_channel,
            kernel_source: s.kernel_source.clone(),
            firewall_auto_allow: s.firewall_auto_allow,
        }
    }

    pub fn is_dirty(&self, s: &AppSettings) -> bool {
        self.auto_fetch != s.auto_fetch_enabled
            || self.interval_min != s.auto_fetch_interval_min
            || self.auto_speed != s.auto_speedtest_enabled
            || self.fetch_concurrency != s.fetch_concurrency
            || self.fetch_timeout_secs != s.fetch_timeout_secs
            || self.speedtest_concurrency != s.speedtest_concurrency
            || self.timeout_ms != s.speedtest_timeout_ms
            || self.speedtest_download != s.speedtest_download
            || self.download_test_ms != s.download_test_ms
            || self.http_server_enabled != s.http_server_enabled
            || self.close_to_tray != s.close_to_tray
            || self.exclude_keywords != s.exclude_keywords
            || self.failover_latency_ms != s.failover_latency_ms
            || self.speedtest_parallel_kernels != s.speedtest_parallel_kernels
            || self.speedtest_persistent_kernel != s.speedtest_persistent_kernel
            || self.speedtest_reuse_minutes != s.speedtest_reuse_minutes
            || self.speedtest_conditional_isolation != s.speedtest_conditional_isolation
            || self.speedtest_download_top_n != s.speedtest_download_top_n
            || self.speedtest_dedup_egress != s.speedtest_dedup_egress
            || self.speedtest_tcp_ping_only != s.speedtest_tcp_ping_only
            || self.bandwidth_guard != s.bandwidth_guard
            || self.bandwidth_min_kbps != s.bandwidth_min_kbps
            || self.speedtest_auto_refill != s.speedtest_auto_refill
            || self.auto_rearm_system_proxy != s.auto_rearm_system_proxy
            || self.auto_prepare_kernel != s.auto_prepare_kernel
            || self.speedtest_added_source != s.speedtest_added_source
            // 外观这两条以前不在这里比：设置页只点主题时 `auto_save && settings_dirty()`
            // 的后半个条件不成立，于是"即时切换并自动保存"其实没落盘（Ctrl+K 那条路是直接
            // save_settings 才看起来正常）。它们能放进来靠的是 `settings_seeded` —— 第一次
            // 加载设置会无条件把库里的值灌进编辑态，否则启动时一比就脏，同步永远不发生。
            || self.ui_theme != s.ui_theme
            || self.ui_language != s.ui_language
            || (self.ui_scale - s.ui_scale).abs() > 0.001
            || self.ui_dark != s.ui_dark
            || self.insider_channel != s.insider_channel
    }
}

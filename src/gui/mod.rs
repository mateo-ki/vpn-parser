use crate::collector::Collector;
use crate::db::{Database, UNGROUPED_SOURCE_ID};
use crate::kernel::{
    find_kernel_executable, missing_kernel_for, KernelInfo, KernelKind, KernelManager,
    KernelPreference, KernelReleaseInfo, KernelUpdateProgress,
};
use crate::models::{
    AppSettings, CreateSource, FetchResult, Node, NodeGroupSummary, NodeSummary, Source,
    SpeedTestSummary, Stats, UpdateSettings, UpdateSource, GOOGLE_PROXY_TEST_KIND,
};
use crate::platform;
use crate::proxy::{
    is_node_connectivity_failure, optimize_traffic_rules, MihomoConfigOptions, NetworkProtocol,
    ProxyManager, ProxyMode, ProxyRuntimeStatus, ProxyState, RoutingMode, RuleAction, RuleMatcher,
    TrafficRule, TunStack, URL_TEST_GROUP,
};
use crate::speedtest::{
    SpeedTestOptions, SpeedTestPhase, SpeedTestProgress, SpeedTester, SpeedtestIntensity,
};
use crate::tasks::TaskTracker;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use tokio::runtime::Runtime;
pub mod icons;
pub mod paste;
pub mod prepare;
pub mod theme;
pub mod types;
pub mod views;
pub mod widgets;

pub use icons::{icon_label, Icon};
pub use types::*;
pub use widgets::*;

/// Gist 发现里每个代理候选对第 1 页的原地重试次数（总尝试 = 1 + 该值）。
const GIST_PAGE_RETRIES: usize = 2;

pub struct VpnApp {
    pub(crate) rt: Arc<Runtime>,
    pub(crate) db: Database,
    pub(crate) collector: Collector,
    pub(crate) speedtester: SpeedTester,
    pub(crate) tasks: TaskTracker,
    pub(crate) kernel_manager: KernelManager,
    pub(crate) proxy_manager: ProxyManager,
    pub(crate) api_controller: crate::api::ApiServerController,
    pub(crate) api_addr: String,
    pub(crate) remote_api_addr: Option<String>,
    pub(crate) remote_api_edit: String,
    pub(crate) remote_api_token: String,
    pub(crate) remote_api_status: String,
    /// 数据库里的发布服务器配置是否已经灌进输入框（避免设置刷新覆盖你正在打的字）。
    pub(crate) publisher_seeded: bool,

    pub(crate) tab: Tab,
    pub(crate) sources_panel: SourcesPanel,
    pub(crate) proxy_panel: ProxyPanel,
    pub(crate) status: String,
    pub(crate) busy: bool,
    pub(crate) proxy_busy: bool,
    pub(crate) gist_pages: usize,
    pub(crate) gist_query_flags: Vec<bool>,
    pub(crate) active_source_fetches: HashSet<String>,

    pub(crate) sources: Vec<Source>,
    pub(crate) fetch_results: Vec<FetchResult>,
    pub(crate) nodes: Vec<NodeSummary>,
    pub(crate) node_groups: Vec<NodeGroupSummary>,
    pub(crate) selected_node_source_id: Option<String>,
    pub(crate) stats: Option<Stats>,
    pub(crate) settings: AppSettings,
    pub(crate) node_total: i64,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
    pub(crate) protocol_filter: String,
    pub(crate) keyword: String,
    pub(crate) latency_filter: String,
    pub(crate) region_filter: String,
    pub(crate) only_available_nodes: bool,
    pub(crate) sort_by: String,
    pub(crate) speedtest_intensity: SpeedtestIntensity,
    pub(crate) kernels: Vec<KernelInfo>,
    pub(crate) kernel_releases: Vec<KernelReleaseInfo>,
    pub(crate) kernel_progress: Option<KernelUpdateProgress>,
    pub(crate) kernel_cancel: Arc<AtomicBool>,
    pub(crate) kernel_busy: bool,
    pub(crate) proxy_status: ProxyRuntimeStatus,
    pub(crate) proxy_config: MihomoConfigOptions,
    pub(crate) proxy_log: String,
    pub(crate) selected_proxy_node_id: Option<String>,
    pub(crate) selected_proxy_node_name: Option<String>,
    pub(crate) switching_node_id: Option<String>,
    pub(crate) proxy_switch_seq: u64,
    pub(crate) proxy_node_groups: Vec<NodeGroupSummary>,
    pub(crate) proxy_nodes: Vec<NodeSummary>,
    pub(crate) selected_proxy_source_id: Option<String>,
    pub(crate) proxy_sort_by: ProxySortBy,
    pub(crate) proxy_search: String,
    pub(crate) proxy_region_filter: String,
    pub(crate) new_rule_kind: RuleDraftKind,
    pub(crate) new_rule_value: String,
    pub(crate) new_rule_action: RuleAction,
    pub(crate) show_recommended_rules_confirm: bool,

    pub(crate) editing_id: Option<String>,
    pub(crate) form_name: String,
    pub(crate) form_url: String,
    pub(crate) form_enabled: bool,
    pub(crate) form_remark: String,
    pub(crate) show_source_form: bool,
    /// Ctrl+V 全局粘贴的识别结果，Some 就弹框（弹框自己负责清空这条）。
    pub(crate) paste_outcome: Option<crate::gui::paste::PasteOutcome>,
    /// 诊断壳（VPN_PARSER_DIAG）下上一条 `DIAGUI ...` 轨迹，用来只在变化时写日志。
    pub(crate) diag_trace: Option<String>,
    /// 窗口是不是被「停放到屏幕外」（= 用户所说的收进托盘）。不再用 Visible(false)。
    pub(crate) parked: bool,
    /// 停放前记下的位置与大小，还原时用。
    pub(crate) park_spot: Option<crate::platform::window_park::Spot>,
    /// 什么时候停放的（诊断壳的自动往返自测用它计时）。
    pub(crate) parked_at: Option<std::time::Instant>,
    /// 系统代理核对：注册表里那份被别人改掉了吗（`None` = 正常/没在跑）。
    pub(crate) proxy_drift: Option<crate::system_proxy::Drift>,
    /// 上一次核对系统代理的时间与"正在查"标记（查一次要起 reg.exe，必须后台）。
    pub(crate) drift_checked_at: Option<std::time::Instant>,
    pub(crate) drift_busy: bool,

    pub(crate) edit_auto_fetch: bool,
    pub(crate) edit_interval_min: u32,
    pub(crate) edit_auto_speed: bool,
    /// 「测通节点太少时后台自动补一轮全量测速」（持久开关，默认关）。
    pub(crate) edit_speedtest_auto_refill: bool,
    /// 「系统代理被外部改掉就自动写回」（持久开关，默认关）。
    pub(crate) edit_auto_rearm_system_proxy: bool,
    /// 「预备状态自动下载内核」和「新增订阅源后自动测这个源」，两条默认开、都能关。
    pub(crate) edit_auto_prepare_kernel: bool,
    pub(crate) edit_speedtest_added_source: bool,
    /// 预备状态自动下载已经试了几次、上次什么时候试的（3 次封顶、间隔 90 秒）,
    /// 以及最近一次失败的原因。
    pub(crate) prepare_attempts: u32,
    pub(crate) prepare_last_at: Option<std::time::Instant>,
    pub(crate) prepare_error: Option<String>,
    /// 「盘上有没有内核」的缓存：那是文件系统检查，不能每帧做（2026-10-08 性能审查）。
    pub(crate) kernel_cache: Option<(bool, std::time::Instant)>,
    /// 刚创建的订阅源 (id, 名称)：等它的拉取结果回来，成功就只测这一批。
    pub(crate) pending_added_source: Option<(String, String)>,
    pub(crate) edit_fetch_concurrency: u32,
    pub(crate) edit_fetch_timeout_secs: u32,
    pub(crate) edit_speedtest_concurrency: u32,
    pub(crate) edit_timeout_ms: u32,
    pub(crate) edit_speedtest_download: bool,
    pub(crate) edit_download_test_ms: u32,
    pub(crate) edit_http_server_enabled: bool,
    pub(crate) edit_close_to_tray: bool,
    pub(crate) memory_trimmed_status: Option<String>,
    pub(crate) settings_saving: bool,

    // export quality filters
    pub(crate) export_max_latency_ms: i64,
    pub(crate) export_min_download_kbps: f64,
    pub(crate) export_quality_only: bool,
    pub(crate) export_limit: u32,
    pub(crate) pending_open_url: Option<String>,
    pub(crate) close_after_elevation: bool,

    pub(crate) detail_text: String,
    pub(crate) show_detail: bool,

    pub(crate) traffic_up_rate: u64,
    pub(crate) traffic_down_rate: u64,
    pub(crate) traffic_up_total: u64,
    pub(crate) traffic_down_total: u64,
    pub(crate) traffic_history: VecDeque<(f32, f32)>,
    pub(crate) active_connections: Vec<ActiveConnectionInfo>,
    pub(crate) traffic_prev_sample: Option<(std::time::Instant, u64, u64)>,
    pub(crate) last_traffic_poll: std::time::Instant,
    pub(crate) last_groups_poll: std::time::Instant,
    pub(crate) traffic_polling: bool,
    /// 内核各 proxy-group 的实时状态（名字/类型/当前成员）。
    pub(crate) kernel_groups: Vec<KernelGroup>,
    /// 控制台可用节点网格的缓存视图 + 数据版本号（proxy_nodes 每次换数据都 +1）。
    pub(crate) proxy_view: Option<ProxyView>,
    pub(crate) proxy_nodes_rev: u64,
    pub(crate) ip_risk: Option<crate::ip_risk::IpRisk>,
    pub(crate) ip_risk_busy: bool,
    pub(crate) ip_risk_error: Option<String>,
    pub(crate) show_ip_risk_dialog: bool,
    /// 弹窗当前展示的是哪个节点。
    pub(crate) ip_risk_target: Option<String>,
    pub(crate) ip_risk_egress: Option<crate::ip_risk::IpRisk>,
    /// 上面这条出口属于哪个节点：只有和 `ip_risk_target` 一致才能显示，否则会张冠李戴。
    pub(crate) ip_risk_egress_for: Option<String>,
    pub(crate) ip_risk_egress_pending: bool,
    pub(crate) ip_risk_egress_error: Option<String>,
    pub(crate) reload_due: bool,
    pub(crate) last_full_reload: std::time::Instant,
    pub(crate) geo_updating: bool,
    pub(crate) auto_failover: bool,
    pub(crate) edit_exclude_keywords: String,
    pub(crate) edit_failover_latency_ms: u32,
    pub(crate) edit_speedtest_parallel_kernels: bool,
    pub(crate) edit_speedtest_persistent_kernel: bool,
    pub(crate) edit_speedtest_reuse_minutes: u32,
    pub(crate) edit_speedtest_conditional_isolation: bool,
    pub(crate) edit_speedtest_download_top_n: u32,
    pub(crate) edit_speedtest_dedup_egress: bool,
    /// 本次运行内自动切换的次数与最近一次原因，显示在控制台状态卡上。
    pub(crate) failover_switches: usize,
    pub(crate) failover_last_reason: Option<String>,
    /// 上次主动复测当前节点的时刻（限流，避免每次刷新都测）。
    pub(crate) failover_probe_at: Option<std::time::Instant>,
    /// 出口健康：最近 24 小时的实测（新→旧无所谓，画的时候自己按小时分桶）和当前连续失败次数。
    /// 表头那句"已连通"必须以它为准，不能只看 `ProxyState`。
    pub(crate) health_rows: Vec<crate::db::telemetry::HealthRow>,
    pub(crate) health_fail_streak: i64,
    /// 24 格健康条的数据：在实测行**到达时**算一次，不每帧重解析几百行时间戳。
    pub(crate) health_buckets: [i8; 24],
    /// 出口切换历史（点开"今日切换 N 次"才查）与今天已切换次数。
    /// 规则命中：`rule_seen` 记住已经计过的连接 id（快照是重复的，一条连接只能计一次），
    /// `rule_tally` 是待落库的增量，每 30 秒刷一次。
    pub(crate) rule_seen: std::collections::HashMap<String, (String, String)>,
    pub(crate) rule_tally: std::collections::HashMap<(String, String), i64>,
    pub(crate) last_hit_flush: std::time::Instant,
    pub(crate) rule_hits_today: Vec<crate::db::telemetry::RuleHitRow>,
    /// 订阅源拉取历史，按 source_id 分桶（每源只留最近几条）。
    pub(crate) fetch_history: std::collections::HashMap<String, Vec<crate::db::telemetry::FetchHistoryRow>>,
    pub(crate) switch_rows: Vec<crate::db::telemetry::SwitchRow>,
    pub(crate) switch_today: i64,
    /// 保留期内库里一共有多少条切换记录（不只今天）。
    pub(crate) switch_total: i64,
    pub(crate) show_switch_log: bool,
    /// Ctrl+K 命令面板：开关、查询串、当前高亮项，以及"下一帧要把焦点给输入框"。
    /// 库里那份 auto_failover 只在第一次载入时灌进界面字段，之后以内存为准
    /// （和 publisher_seeded 同一个理由：别让周期性设置重载把用户刚改的冲掉）。
    pub(crate) failover_seeded: bool,
    /// 一轮替补复检正在进行（避免断路器每 60 秒又开一拨并发复检）。
    pub(crate) failover_searching: bool,
    /// 复检期间记着"从哪个节点切走"，好把没切成的那一次也写进切换历史。
    pub(crate) failover_from: Option<(String, String)>,
    /// 系统设置页的"显示高级设置"：只影响这一屏看不看得到那些数值调参，不入库。
    pub(crate) settings_advanced_open: bool,
    pub(crate) palette_open: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) palette_needs_focus: bool,
    /// 选路调度器（#4）：EWMA 延迟 + 断路器 + 打分 + 滞后切换，带宽保护（#5）也走它。
    pub(crate) router: crate::router::Router,
    pub(crate) edit_bandwidth_guard: bool,
    pub(crate) edit_bandwidth_min_kbps: u32,
    pub(crate) edit_ui_theme: String,
    pub(crate) edit_kernel_source: String,
    pub(crate) edit_firewall_auto_allow: bool,
    /// 「下载内核」弹框：是否打开、选中的方式、本地 zip 路径。
    pub(crate) show_kernel_dialog: bool,
    pub(crate) kernel_dialog_source: String,
    pub(crate) kernel_dialog_path: String,
    /// 弹框是为哪个内核开的。
    pub(crate) kernel_dialog_kind: Option<crate::kernel::KernelKind>,
    /// 防火墙规则最近一次写入结果，弹框右侧显示，避免"到底加没加"看不出来。
    pub(crate) firewall_status: Option<String>,
    /// 只在第一次拿到设置时按开关自动加一次规则。
    pub(crate) firewall_seeded: bool,
    pub(crate) connections_filter: String,
    pub(crate) homogeneity_reports: Option<Vec<crate::models::SourceHomogeneityReport>>,
    /// 同质化对话框里被勾选要剔除的源 id（分析完成后默认勾上判定为冗余的那些）。
    pub(crate) homogeneity_selected: HashSet<String>,
    pub(crate) show_homogeneity_dialog: bool,
    pub(crate) homogeneity_busy: bool,
    pub(crate) homogeneity_status: Option<String>,
    pub(crate) tray: crate::platform::TrayHandle,
    pub(crate) force_quit: bool,
    /// 已经装到 egui 上的主题 id，用来只在变化时重建 Style。
    pub(crate) applied_theme_id: String,
    pub(crate) frame_count: u64,
    pub(crate) logo_texture: Option<egui::TextureHandle>,

    pub(crate) tx: Sender<BgMsg>,
    pub(crate) rx: Receiver<BgMsg>,
}

impl VpnApp {
    pub fn new(
        rt: Arc<Runtime>,
        db: Database,
        collector: Collector,
        speedtester: SpeedTester,
        tasks: TaskTracker,
        kernel_manager: KernelManager,
        proxy_manager: ProxyManager,
        api_controller: crate::api::ApiServerController,
        initial_proxy_node: Option<(String, String)>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let api_addr = api_controller.listen_addr().to_string();
        let mut app = Self {
            rt,
            db,
            collector,
            speedtester,
            tasks,
            kernel_manager,
            proxy_status: ProxyRuntimeStatus::stopped(MihomoConfigOptions::default()),
            proxy_config: MihomoConfigOptions::default(),
            proxy_manager,
            api_controller,
            api_addr,
            remote_api_addr: None,
            remote_api_edit: String::new(),
            remote_api_token: String::new(),
            remote_api_status: "未检查".into(),
            publisher_seeded: false,
            tab: Tab::Proxy,
            sources_panel: SourcesPanel::List,
            proxy_panel: ProxyPanel::Nodes,
            status: "就绪".into(),
            busy: false,
            proxy_busy: false,
            gist_pages: 100,
            gist_query_flags: crate::collector::GIST_DISCOVERY_QUERIES
                .iter()
                .map(|query| *query == crate::collector::GIST_DISCOVERY_DEFAULT_QUERY)
                .collect(),
            active_source_fetches: HashSet::new(),
            sources: vec![],
            fetch_results: vec![],
            nodes: vec![],
            node_groups: vec![],
            selected_node_source_id: None,
            stats: None,
            settings: AppSettings::default(),
            node_total: 0,
            page: 1,
            page_size: 50,
            protocol_filter: String::new(),
            keyword: String::new(),
            latency_filter: "all".into(),
            region_filter: "all".into(),
            only_available_nodes: false,
            sort_by: "latency".into(),
            speedtest_intensity: SpeedtestIntensity::Standard,
            kernels: vec![],
            kernel_releases: vec![],
            kernel_progress: None,
            kernel_cancel: Arc::new(AtomicBool::new(false)),
            kernel_busy: false,
            proxy_log: String::new(),
            selected_proxy_node_id: initial_proxy_node.as_ref().map(|node| node.0.clone()),
            selected_proxy_node_name: initial_proxy_node.map(|node| node.1),
            switching_node_id: None,
            proxy_switch_seq: 0,
            proxy_node_groups: vec![],
            proxy_nodes: vec![],
            selected_proxy_source_id: Some("all".to_string()),
            proxy_sort_by: ProxySortBy::Latency,
            proxy_search: String::new(),
            proxy_region_filter: "all".to_string(),
            new_rule_kind: RuleDraftKind::DomainSuffix,
            new_rule_value: String::new(),
            new_rule_action: RuleAction::Proxy,
            show_recommended_rules_confirm: false,
            editing_id: None,
            form_name: String::new(),
            form_url: String::new(),
            form_enabled: true,
            form_remark: String::new(),
            show_source_form: false,
            paste_outcome: None,
            diag_trace: None,
            parked: false,
            park_spot: None,
            parked_at: None,
            proxy_drift: None,
            drift_checked_at: None,
            drift_busy: false,
            edit_auto_fetch: false,
            edit_interval_min: 60,
            edit_auto_speed: false,
            edit_speedtest_auto_refill: true,
            edit_auto_rearm_system_proxy: false,
            edit_auto_prepare_kernel: true,
            edit_speedtest_added_source: true,
            prepare_attempts: 0,
            prepare_last_at: None,
            prepare_error: None,
            kernel_cache: None,
            pending_added_source: None,
            edit_fetch_concurrency: 4,
            edit_fetch_timeout_secs: 30,
            edit_speedtest_concurrency: 128,
            edit_timeout_ms: 1500,
            edit_speedtest_download: false,
            edit_download_test_ms: 800,
            edit_http_server_enabled: false,
            edit_close_to_tray: true,
            memory_trimmed_status: None,
            settings_saving: false,
            export_max_latency_ms: 800,
            // 0 = do not require download (encrypted proxies usually have no measurable download here)
            export_min_download_kbps: 0.0,
            export_quality_only: true,
            export_limit: 30,
            pending_open_url: None,
            close_after_elevation: false,
            detail_text: String::new(),
            show_detail: false,
            traffic_up_rate: 0,
            traffic_down_rate: 0,
            traffic_up_total: 0,
            traffic_down_total: 0,
            traffic_history: VecDeque::with_capacity(160),
            active_connections: Vec::new(),
            traffic_prev_sample: None,
            last_traffic_poll: std::time::Instant::now(),
            last_groups_poll: std::time::Instant::now() - std::time::Duration::from_secs(60),
            traffic_polling: false,
            kernel_groups: Vec::new(),
            proxy_view: None,
            proxy_nodes_rev: 0,
            ip_risk: None,
            ip_risk_busy: false,
            ip_risk_error: None,
            show_ip_risk_dialog: false,
            ip_risk_target: None,
            ip_risk_egress: None,
            ip_risk_egress_for: None,
            ip_risk_egress_pending: false,
            ip_risk_egress_error: None,
            reload_due: false,
            last_full_reload: std::time::Instant::now(),
            geo_updating: false,
            auto_failover: false,
            edit_exclude_keywords: String::new(),
            edit_failover_latency_ms: 1500,
            edit_speedtest_parallel_kernels: false,
            edit_speedtest_persistent_kernel: false,
            edit_speedtest_reuse_minutes: 0,
            edit_speedtest_conditional_isolation: false,
            edit_speedtest_download_top_n: 0,
            edit_speedtest_dedup_egress: true,
            failover_switches: 0,
            failover_last_reason: None,
            failover_probe_at: None,
            health_rows: Vec::new(),
            health_buckets: [0; 24],
            health_fail_streak: 0,
            rule_seen: std::collections::HashMap::new(),
            rule_tally: std::collections::HashMap::new(),
            last_hit_flush: std::time::Instant::now(),
            rule_hits_today: Vec::new(),
            fetch_history: std::collections::HashMap::new(),
            switch_rows: Vec::new(),
            switch_today: 0,
            switch_total: 0,
            show_switch_log: false,
            failover_seeded: false,
            failover_searching: false,
            failover_from: None,
            settings_advanced_open: false,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_needs_focus: false,
            router: crate::router::Router::default(),
            edit_bandwidth_guard: false,
            edit_bandwidth_min_kbps: 0,
            edit_ui_theme: theme::DEFAULT_THEME.to_string(),
            edit_kernel_source: crate::kernel::KernelSource::GhProxy.id().to_string(),
            edit_firewall_auto_allow: false,
            show_kernel_dialog: false,
            kernel_dialog_source: crate::kernel::KernelSource::GhProxy.id().to_string(),
            kernel_dialog_path: String::new(),
            kernel_dialog_kind: None,
            firewall_status: None,
            firewall_seeded: false,
            connections_filter: String::new(),
            homogeneity_reports: None,
            homogeneity_selected: HashSet::new(),
            show_homogeneity_dialog: false,
            homogeneity_busy: false,
            homogeneity_status: None,
            tray: crate::platform::TrayHandle::new(),
            force_quit: false,
            applied_theme_id: String::new(),
            frame_count: 0,
            logo_texture: None,
            tx,
            rx,
        };
        app.reload_all();
        app
    }

    pub(crate) fn speed_opts(&self) -> SpeedTestOptions {
        let (concurrency, timeout_ms) = match self.speedtest_intensity {
            SpeedtestIntensity::Fast => (
                self.settings.speedtest_concurrency.max(256),
                self.settings.speedtest_timeout_ms.min(800),
            ),
            SpeedtestIntensity::Standard => (
                self.settings.speedtest_concurrency,
                self.settings.speedtest_timeout_ms,
            ),
            SpeedtestIntensity::Deep => (
                (self.settings.speedtest_concurrency / 2).max(32).min(64),
                self.settings.speedtest_timeout_ms.max(3500),
            ),
        };
        SpeedTestOptions {
            concurrency,
            timeout_ms,
            measure_download: self.settings.speedtest_download,
            download_test_ms: self.settings.download_test_ms,
            intensity: self.speedtest_intensity,
            parallel_kernels: self.settings.speedtest_parallel_kernels,
            persistent_kernel: self.settings.speedtest_persistent_kernel,
            reuse_verified_minutes: self.settings.speedtest_reuse_minutes,
            conditional_isolation: self.settings.speedtest_conditional_isolation,
            download_top_n: self.settings.speedtest_download_top_n,
            dedup_egress: self.settings.speedtest_dedup_egress,
        }
    }

    /// 后台事件常常连着要全量刷新（33 个源各一次），而一次 `reload_all` 要做上百次
    /// 全表聚合。这里只置标志，由 update 循环合并成每秒最多一次。
    pub(crate) fn request_full_reload(&mut self) {
        self.reload_due = true;
    }

    pub(crate) fn reload_all(&mut self) {
        self.reload_sources();
        self.reload_nodes();
        self.reload_node_groups();
        self.reload_stats();
        self.reload_settings();
        // 每次刷新都打 GitHub API 会很快撞限流（未认证 60 次/小时），这里只本地探测。
        self.detect_kernels();
        self.reload_proxy();
        self.reload_proxy_node_groups();
        self.reload_egress_health();
        self.reload_rule_hits();
        self.reload_fetch_history();
    }

    fn reload_fetch_history(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Ok(rows) = db.fetch_history_all(2000).await {
                let _ = tx.send(BgMsg::FetchHistory(rows));
            }
        });
    }

    /// 只做本地探测（起内核进程读版本号），不碰 GitHub。
    pub(crate) fn detect_kernels(&mut self) {
        let manager = self.kernel_manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = tx.send(BgMsg::Kernels(manager.detect_all().await));
        });
    }

    /// 本地探测 + 官方 GitHub Release 检查，只在点「检查更新」时调用。
    pub(crate) fn reload_kernels(&mut self) {
        let manager = self.kernel_manager.clone();
        let proxy_manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = tx.send(BgMsg::Kernels(manager.detect_all().await));
            let proxy_status = proxy_manager.status().await;
            let proxy_url = proxy_status
                .local_proxy_active()
                .then(|| format!("http://127.0.0.1:{}", proxy_status.mixed_port));
            let releases = manager
                .check_updates_with_proxy(proxy_url)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::KernelReleases(releases));
        });
    }

    pub(crate) fn reload_proxy(&mut self) {
        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let status = manager.status().await;
            let config = manager.config().await;
            let log = manager
                .recent_log(12_000)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::ProxyStatus(status));
            let _ = tx.send(BgMsg::ProxyConfig(Ok(config)));
            let _ = tx.send(BgMsg::ProxyLog(log));
        });
    }

    pub(crate) fn reload_sources(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = db.list_sources().await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::Sources(res));
        });
    }

    pub(crate) fn reload_nodes(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        let protocol = nonempty_opt(&self.protocol_filter);
        let keyword = nonempty_opt(&self.keyword);
        let latency = if self.only_available_nodes {
            Some("alive".to_string())
        } else if self.latency_filter == "all" {
            None
        } else {
            Some(self.latency_filter.clone())
        };
        let sort = Some(self.sort_by.clone());
        let country = if self.region_filter == "all" {
            None
        } else {
            Some(self.region_filter.clone())
        };
        let page = self.page;
        let page_size = self.page_size;
        let Some(source_id) = self.selected_node_source_id.clone() else {
            self.nodes.clear();
            self.node_total = 0;
            return;
        };
        self.rt.spawn(async move {
            let res = db
                .list_nodes(
                    protocol.as_deref(),
                    Some(&source_id),
                    keyword.as_deref(),
                    latency.as_deref(),
                    country.as_deref(),
                    page,
                    page_size,
                    sort.as_deref(),
                )
                .await
                .map(|p| (p.items, p.total))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::Nodes(res));
        });
    }

    pub(crate) fn reload_node_view(&mut self) {
        self.reload_nodes();
        self.reload_node_groups();
    }

    pub(crate) fn reload_stats(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = db.stats().await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::Stats(res));
        });
    }

    pub(crate) fn reload_node_groups(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        let protocol = nonempty_opt(&self.protocol_filter);
        let keyword = nonempty_opt(&self.keyword);
        let latency = if self.only_available_nodes {
            Some("alive".to_string())
        } else if self.latency_filter == "all" {
            None
        } else {
            Some(self.latency_filter.clone())
        };
        self.rt.spawn(async move {
            let result = db
                .list_node_groups(protocol.as_deref(), keyword.as_deref(), latency.as_deref())
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::NodeGroups(result));
        });
    }

    pub(crate) fn reload_proxy_node_groups(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = db
                .list_node_groups(None, None, Some("alive"))
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::ProxyNodeGroups(result));
        });
    }

    pub(crate) fn reload_proxy_nodes(&mut self) {
        let Some(source_id) = self.selected_proxy_source_id.clone() else {
            self.proxy_nodes.clear();
            return;
        };
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = db
                .list_available_nodes_for_source(&source_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::ProxyNodes(result));
        });
    }

    pub(crate) fn reload_settings(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = db.get_settings().await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::Settings(res));
        });
    }

    pub(crate) fn poll_bg(&mut self, ctx: &egui::Context) {
        let mut need_repaint = false;
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    need_repaint = true;
                    match msg {
                        BgMsg::Sources(Ok(list)) => self.sources = list,
                        BgMsg::Sources(Err(e)) => self.status = format!("加载订阅源失败: {e}"),
                        BgMsg::Nodes(Ok((items, total))) => {
                            self.nodes = items;
                            self.node_total = total;
                        }
                        BgMsg::Nodes(Err(e)) => self.status = format!("加载节点失败: {e}"),
                        BgMsg::NodeGroups(Ok(groups)) => {
                            let previous = self.selected_node_source_id.clone();
                            let selection_is_valid = previous.as_ref().is_some_and(|selected| {
                                groups
                                    .iter()
                                    .any(|group| node_group_key(group) == *selected)
                            });
                            if !selection_is_valid {
                                self.selected_node_source_id = groups.first().map(node_group_key);
                                self.page = 1;
                            }
                            self.node_groups = groups;
                            if self.selected_node_source_id != previous {
                                self.reload_nodes();
                            }
                            if self.selected_node_source_id.is_none() {
                                self.nodes.clear();
                                self.node_total = 0;
                            }
                        }
                        BgMsg::NodeGroups(Err(e)) => self.status = format!("加载节点分组失败: {e}"),
                        BgMsg::ProxyNodeGroups(Ok(groups)) => {
                            let previous = self.selected_proxy_source_id.clone();
                            let selection_is_valid = previous.as_ref().is_some_and(|selected| {
                                selected == "all"
                                    || groups
                                        .iter()
                                        .any(|group| node_group_key(group) == *selected)
                            });
                            if !selection_is_valid {
                                self.selected_proxy_source_id = Some("all".to_string());
                            }
                            self.proxy_node_groups = groups;
                            self.reload_proxy_nodes();
                        }
                        BgMsg::ProxyNodeGroups(Err(e)) => {
                            self.status = format!("加载可用订阅失败: {e}")
                        }
                        BgMsg::ProxyNodes(Ok(nodes)) => {
                            self.proxy_nodes = nodes;
                            self.proxy_nodes_rev = self.proxy_nodes_rev.wrapping_add(1);
                        }
                        BgMsg::ProxyNodes(Err(e)) => self.status = format!("加载可用节点失败: {e}"),
                        BgMsg::AutoAnchorStart(Ok((id, name))) => {
                            self.selected_proxy_node_id = Some(id);
                            self.selected_proxy_node_name = Some(name);
                            self.start_proxy();
                        }
                        BgMsg::AutoAnchorStart(Err(e)) => {
                            self.status = format!("自动选路锚点挑选失败: {e}")
                        }
                        BgMsg::ProxyDrift(drift) => {
                            self.drift_busy = false;
                            use crate::system_proxy::Drift;
                            match drift {
                                Drift::InSync => self.proxy_drift = None,
                                other => {
                                    let text = crate::system_proxy::drift_text(&other);
                                    crate::startup_log(&format!("[系统代理核对] {text}"));
                                    self.proxy_drift = Some(other);
                                    // 开关开着才动手（默认关：那是用户的 HKCU）。
                                    if self.settings.auto_rearm_system_proxy {
                                        self.status = format!("{text} -> 正在按设置写回");
                                        let manager = self.proxy_manager.clone();
                                        let tx = self.tx.clone();
                                        self.rt.spawn(async move {
                                            let message = match manager.rearm_system_proxy().await
                                            {
                                                Ok(done) => done,
                                                Err(error) =>
                                                    format!("系统代理写回失败：{error}"),
                                            };
                                            crate::startup_log(&format!("[系统代理核对] {message}"));
                                            let _ = tx.send(BgMsg::Status(message));
                                        });
                                    }
                                }
                            }
                        }
                        BgMsg::FirewallApplied(Ok(report)) => {
                            self.status = report.clone();
                            self.firewall_status = Some(report);
                        }
                        BgMsg::FirewallApplied(Err(error)) => {
                            self.status = format!("防火墙规则未写入：{error}");
                            self.firewall_status = Some(format!("失败：{error}"));
                        }
                        BgMsg::Stats(Ok(s)) => self.stats = Some(s),
                        BgMsg::Stats(Err(e)) => self.status = format!("加载统计失败: {e}"),
                        BgMsg::Settings(Ok(s)) => {
                            self.settings = s;
                            if !self.publisher_seeded {
                                self.publisher_seeded = true;
                                let stored = self.settings.publisher_api.trim().to_owned();
                                if stored.is_empty() {
                                    self.remote_api_addr = None;
                                    self.remote_api_edit.clear();
                                } else {
                                    let normalized = normalize_api_base(&stored);
                                    self.remote_api_edit = normalized.clone();
                                    self.remote_api_addr = Some(normalized);
                                }
                                self.remote_api_token = self.settings.publisher_token.clone();
                            }
                            if !self.failover_seeded {
                                self.failover_seeded = true;
                                self.auto_failover = self.settings.auto_failover;
                            }
                            if !self.settings_dirty() && !self.settings_saving {
                                self.sync_edit_settings();
                            }
                        }
                        BgMsg::PublisherSaved(Err(error)) => {
                            self.remote_api_status = format!("保存失败: {error}");
                            self.status = format!("发布服务器配置保存失败: {error}");
                        }
                        BgMsg::PublisherSaved(Ok(())) => {
                            self.remote_api_status = if self.remote_api_addr.is_some() {
                                "发布服务器配置已保存（数据库）"
                            } else {
                                "已清空发布服务器地址与 Token（数据库）"
                            }
                            .into();
                            self.status = self.remote_api_status.clone();
                        }
                        BgMsg::Settings(Err(e)) => self.status = format!("加载设置失败: {e}"),
                        BgMsg::SettingsSaved(Ok(s)) => {
                            self.settings_saving = false;
                            self.settings = s;
                            self.sync_edit_settings();
                            self.status = "设置已保存".into();
                        }
                        BgMsg::SettingsSaved(Err(e)) => {
                            self.settings_saving = false;
                            self.status = format!("保存设置失败: {e}");
                        }
                        BgMsg::FetchAllDone(Ok(results)) => {
                            self.busy = false;
                            self.fetch_results = results.clone();
                            let ok = results.iter().filter(|r| r.success).count();
                            let nodes: usize = results.iter().map(|r| r.node_count).sum();
                            self.status =
                                format!("拉取完成：成功 {ok}/{}，解析节点 {nodes}", results.len());
                            self.request_full_reload();
                        }
                        BgMsg::FetchAllDone(Err(e)) => {
                            self.busy = false;
                            self.status = format!("拉取失败: {e}");
                        }
                        BgMsg::Status(text) => self.status = text,
                        BgMsg::EgressHealth(rows, streak) => {
                            self.health_buckets = crate::gui::widgets::health_hour_buckets(&rows);
                            self.health_rows = rows;
                            self.health_fail_streak = streak;
                        }
                        BgMsg::EgressSwitches(rows, today, total) => {
                            self.switch_rows = rows;
                            self.switch_today = today;
                            self.switch_total = total;
                        }
                        BgMsg::SourceAdded(id, name) => {
                            // 「加完就测这个源」：先把源拉下来，拉成功了再测它这一批。
                            if self.settings.speedtest_added_source {
                                self.pending_added_source = Some((id.clone(), name));
                                self.fetch_one(id);
                            }
                        }
                        BgMsg::AddedSourceIds { source_id, source_name, ids } => {
                            let _ = source_id;
                            match ids {
                                Ok(ids) if ids.is_empty() => {
                                    self.status =
                                        format!("订阅「{source_name}」没解析出节点，跳过测速");
                                }
                                Ok(ids) => {
                                    let count = ids.len();
                                    self.speedtest_ids(
                                        Some(ids),
                                        &format!("正在测速新增订阅「{source_name}」的 {count} 个节点..."),
                                    );
                                }
                                Err(error) => {
                                    self.status =
                                        format!("取「{source_name}」的节点列表失败: {error}");
                                }
                            }
                        }
                        BgMsg::RuleHits(rows) => self.rule_hits_today = rows,
                        BgMsg::FetchHistory(rows) => {
                            let mut buckets: std::collections::HashMap<
                                String,
                                Vec<crate::db::telemetry::FetchHistoryRow>,
                            > = std::collections::HashMap::new();
                            for (source_id, row) in rows {
                                let entry = buckets.entry(source_id).or_default();
                                if entry.len() < 7 {
                                    entry.push(row);
                                }
                            }
                            self.fetch_history = buckets;
                        }
                        BgMsg::FailoverProbe {
                            node_id,
                            node_name,
                            ok,
                            latency_ms,
                        } => {
                            self.handle_failover_probe(node_id, node_name, ok, latency_ms);
                        }
                        BgMsg::GistDiscoveryDone(Ok(summary)) => {
                            self.busy = false;
                            self.status = summary;
                            self.reload_sources();
                            self.reload_nodes();
                            self.reload_stats();
                        }
                        BgMsg::GistDiscoveryDone(Err(e)) => {
                            self.busy = false;
                            self.status = format!("Gist 发现失败: {e}");
                        }
                        BgMsg::FetchOneDone { source_id, result } => {
                            self.active_source_fetches.remove(&source_id);
                            if let Ok(result) = &result {
                                self.fetch_results
                                    .retain(|item| item.source_id != result.source_id);
                                self.fetch_results.push(result.clone());
                            }
                            if self.pending_added_source.as_ref().is_some_and(|(id, _)| id == &source_id) {
                                let (_, name) = self.pending_added_source.take().unwrap_or_default();
                                let (ok, nodes) = match &result {
                                    Ok(fetch) => (fetch.success, fetch.node_count as i64),
                                    Err(_) => (false, 0),
                                };
                                if prepare::should_test_added_source(
                                    self.settings.speedtest_added_source,
                                    true,
                                    ok,
                                    nodes,
                                ) {
                                    self.queue_added_source_speedtest(source_id.clone(), name);
                                }
                            }
                            self.status = match result {
                                Ok(result) if result.success => format!(
                                    "订阅“{}”拉取完成：{} 个节点",
                                    result.source_name, result.node_count
                                ),
                                Ok(result) => {
                                    format!(
                                        "订阅“{}”拉取失败：{}",
                                        result.source_name, result.message
                                    )
                                }
                                Err(error) => format!("单源拉取失败: {error}"),
                            };
                            self.request_full_reload();
                        }
                        BgMsg::FetchProgress { done, total } => {
                            self.status = format!("正在拉取全部启用源：已完成 {done}/{total}");
                        }
                        BgMsg::SpeedProgress(progress) => {
                            self.status = format_speedtest_progress(&progress);
                        }
                        BgMsg::SourceSpeedProgress { done, total } => {
                            self.status = format!("正在测试所有节点：已完成 {done}/{total} 个节点");
                        }
                        BgMsg::FailoverTarget(outcome) => self.on_failover_target(outcome),
                        BgMsg::SpeedDone(Ok(sum)) => {
                            self.busy = false;
                            let avg = sum
                                .avg_ms
                                .map(|v| format!("{v:.0}ms"))
                                .unwrap_or_else(|| "-".into());
                            let avg_dl = sum
                                .avg_download_kbps
                                .map(|v| format!("{v:.1}KB/s"))
                                .unwrap_or_else(|| "-".into());
                            self.status = format!(
                                "测速完成：可用 {}/{}，平均延迟 {}，平均下载 {}",
                                sum.ok, sum.total, avg, avg_dl
                            );
                            self.request_full_reload();
                            crate::platform::trim_working_set();
                        }
                        BgMsg::SpeedDone(Err(e)) => {
                            self.busy = false;
                            self.status = format!("测速失败: {e}");
                        }
                        BgMsg::Kernels(kernels) => self.kernels = kernels,
                        BgMsg::IpRisk(Ok(risk)) => {
                            self.ip_risk_busy = false;
                            self.ip_risk_error = None;
                            self.status = format!("IP 检测：{}", risk.headline());
                            self.ip_risk = Some(risk);
                        }
                        BgMsg::IpRisk(Err(error)) => {
                            self.ip_risk_busy = false;
                            self.ip_risk = None;
                            self.ip_risk_error = Some(error);
                            self.status = "IP 检测查询失败".into();
                        }
                        BgMsg::IpEgress(node_id, Ok(egress)) => {
                            self.ip_risk_egress_pending = false;
                            self.ip_risk_egress_error = None;
                            self.status = format!("出口 {}", egress.headline());
                            self.ip_risk_egress_for = Some(node_id);
                            self.ip_risk_egress = Some(egress);
                        }
                        BgMsg::IpEgress(node_id, Err(error)) => {
                            self.ip_risk_egress_pending = false;
                            self.ip_risk_egress = None;
                            self.ip_risk_egress_for = Some(node_id);
                            self.ip_risk_egress_error = Some(error);
                        }
                        BgMsg::KernelReleases(Ok(releases)) => {
                            // 以前这里只存数据不写状态，状态栏会一直停在
                            // "正在检查官方内核版本..."，看起来像卡死。
                            let summary = releases
                                .iter()
                                .map(|release| {
                                    format!(
                                        "{} {}",
                                        release.kind.display_name(),
                                        release
                                            .latest_version
                                            .as_deref()
                                            .unwrap_or("未取到最新版本")
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(" · ");
                            self.status = format!("内核版本检查完成：{summary}");
                            self.kernel_releases = releases;
                        }
                        BgMsg::KernelReleases(Err(error)) => {
                            self.status = format!("内核更新检查失败: {error}");
                        }
                        BgMsg::KernelProgress(progress) => {
                            self.status = progress.message.clone();
                            self.kernel_progress = Some(progress);
                        }
                        BgMsg::KernelUpdated(result) => {
                            self.kernel_busy = false;
                            self.kernel_progress = None;
                            match result {
                                Ok(message) => {
                                    crate::startup_log(&format!("[预备] 内核就绪: {message}"));
                                    self.prepare_error = None;
                                    // 装好了就作废缓存，徽章当下这一帧就该从「预备」换掉。
                                    self.kernel_cache = None;
                                    self.status = message;
                                }
                                Err(error) => {
                                    if self.prepare_attempts > 0 && !Self::kernel_installed() {
                                        crate::startup_log(&format!(
                                            "[预备] 第 {}/{} 次自动下载失败: {error}{}",
                                            self.prepare_attempts,
                                            prepare::MAX_AUTO_ATTEMPTS,
                                            if self.prepare_attempts >= prepare::MAX_AUTO_ATTEMPTS {
                                                " —— 次数用完，之后不再自己动，需要他手动下"
                                            } else {
                                                ""
                                            }
                                        ));
                                        tracing::warn!(%error, "预备状态自动下载内核失败");
                                        self.prepare_error = Some(error.clone());
                                    }
                                    self.status = format!("内核更新失败: {error}");
                                }
                            }
                            self.reload_kernels();
                        }
                        BgMsg::ProxyStatus(status) => self.proxy_status = status,
                        BgMsg::ProxyConfig(Ok(config)) => self.set_proxy_config(config),
                        BgMsg::ProxyConfig(Err(error)) => {
                            self.status = format!("读取代理配置失败: {error}");
                        }
                        BgMsg::ProxyConfigSaved(result) => {
                            self.proxy_busy = false;
                            match result {
                                Ok(config) => {
                                    self.set_proxy_config(config);
                                    self.status = "代理配置已保存".into();
                                }
                                Err(error) => {
                                    self.status = format!("保存代理配置失败: {error}");
                                }
                            }
                            self.reload_proxy();
                        }
                        BgMsg::ProxyLog(Ok(log)) => self.proxy_log = log,
                        BgMsg::ProxyLog(Err(error)) => {
                            self.status = format!("读取代理日志失败: {error}");
                        }
                        BgMsg::ProxyOp(result) => {
                            self.proxy_busy = false;
                            self.switching_node_id = None;
                            match result {
                                Ok(message) => self.status = message,
                                Err(error) => self.status = format!("代理操作失败: {error}"),
                            }
                            self.reload_proxy();
                            self.reload_proxy_nodes();
                            // 探测要一直跑（表头那句"最近实测"靠它），auto_failover 只决定
                            // 测出问题之后要不要自动换出口 —— 见 handle_failover_probe。
                            self.check_failover();
                        }
                        BgMsg::ApiServerStatus(result) => match result {
                            Ok(message) => self.status = message,
                            Err(error) => self.status = format!("HTTP服务器操作失败: {error}"),
                        },
                        BgMsg::ProxySwitchDone {
                            seq,
                            node_id,
                            result,
                        } => {
                            if seq == self.proxy_switch_seq {
                                self.switching_node_id = None;
                                match result {
                                    Ok(message) => self.status = message,
                                    Err(error) => {
                                        self.status = format!("切换节点失败: {error}");
                                        let db = self.db.clone();
                                        let nid = node_id.clone();
                                        let kernel = self.proxy_status.kernel.map(|k| match k {
                                            KernelKind::Mihomo => "mihomo",
                                            KernelKind::SingBox => "sing_box",
                                            KernelKind::Xray => "xray",
                                        });
                                        self.rt.spawn(async move {
                                            let _ = db
                                                .update_node_speed(
                                                    &nid,
                                                    false,
                                                    None,
                                                    None,
                                                    kernel,
                                                    Some(GOOGLE_PROXY_TEST_KIND),
                                                )
                                                .await;
                                        });
                                    }
                                }
                                self.reload_proxy();
                                self.reload_proxy_nodes();
                                self.check_failover();
                            }
                        }
                        BgMsg::RemoteApiCheck(result) => {
                            self.remote_api_status = match result {
                                Ok(message) => message,
                                Err(error) => error,
                            };
                        }
                        BgMsg::PublishDone(result) => {
                            self.busy = false;
                            self.status = match result {
                                Ok(message) => message,
                                Err(error) => format!("推送失败: {error}"),
                            };
                        }
                        BgMsg::OpDone { result, unlock } => {
                            if unlock {
                                self.busy = false;
                            }
                            match result {
                                Ok(msg) => {
                                    self.status = msg;
                                    self.request_full_reload();
                                    crate::platform::trim_working_set();
                                }
                                Err(e) => self.status = format!("操作失败: {e}"),
                            }
                        }
                        BgMsg::NodeDetail(Ok(n)) => {
                            self.detail_text = serde_json::to_string_pretty(&n)
                                .unwrap_or_else(|_| format!("{n:?}"));
                            self.show_detail = true;
                        }
                        BgMsg::NodeDetail(Err(e)) => {
                            self.status = format!("加载节点详情失败: {e}");
                        }
                        BgMsg::TrafficSnapshot {
                            now,
                            upload_total,
                            download_total,
                            connections,
                        } => {
                            self.traffic_polling = false;
                            if let Some((prev_time, prev_up, prev_down)) = self.traffic_prev_sample
                            {
                                let dt = now.duration_since(prev_time).as_secs_f64();
                                if dt > 0.05 {
                                    self.traffic_up_rate =
                                        ((upload_total.saturating_sub(prev_up)) as f64 / dt) as u64;
                                    self.traffic_down_rate =
                                        ((download_total.saturating_sub(prev_down)) as f64 / dt)
                                            as u64;
                                }
                            }
                            self.traffic_prev_sample = Some((now, upload_total, download_total));
                            self.traffic_up_total = upload_total;
                            self.traffic_down_total = download_total;
                            // 规则命中：连接是每秒重复的快照，所以按"第一次见到这个连接 id"
                            // 计一次，长连接不会被反复累加。
                            for conn in &connections {
                                if !self.rule_seen.contains_key(&conn.id) {
                                    let key = (conn.rule.clone(), conn.target.clone());
                                    *self.rule_tally.entry(key.clone()).or_insert(0) += 1;
                                    self.rule_seen.insert(conn.id.clone(), key);
                                }
                            }
                            if self.rule_seen.len() > 20_000 {
                                // 长跑兜底：只丢去重表，计数已经并进 tally 了。
                                self.rule_seen.clear();
                            }
                            if !self.rule_tally.is_empty()
                                && self.last_hit_flush.elapsed() >= std::time::Duration::from_secs(30)
                            {
                                self.flush_rule_hits();
                            }
                            self.active_connections = connections;

                            let down_kb = self.traffic_down_rate as f32 / 1024.0;
                            let up_kb = self.traffic_up_rate as f32 / 1024.0;
                            if self.traffic_history.len() >= 150 {
                                self.traffic_history.pop_front();
                            }
                            self.traffic_history.push_back((down_kb, up_kb));
                            // #5 带宽保护：把当前出口的下行速率当作活跃节点的带宽
                            // 样本喂给调度器，持续低于阈值时由它决定是否换节点。
                            if self.auto_failover
                                && self.settings.bandwidth_guard
                                && self.traffic_down_rate > 0
                            {
                                self.sync_router_config();
                                if let Some(active) = self.selected_proxy_node_id.clone() {
                                    let name =
                                        self.selected_proxy_node_name.clone().unwrap_or_default();
                                    let decision = self.router.observe(
                                        &active,
                                        crate::router::Sample {
                                            ok: true,
                                            latency_ms: None,
                                            download_kbps: Some(
                                                self.traffic_down_rate as f64 / 1024.0,
                                            ),
                                        },
                                        std::time::Instant::now(),
                                    );
                                    self.apply_router_decision(&active, &name, decision);
                                }
                            }
                        }
                        BgMsg::TrafficPollDone => {
                            self.traffic_polling = false;
                        }
                        BgMsg::KernelGroups(groups, latencies) => {
                            self.kernel_groups = groups;
                            self.feed_router_kernel_latency(latencies);
                        }
                        BgMsg::ConnectionClosed(id) => {
                            self.active_connections.retain(|c| c.id != id);
                            self.status = "已断开连接".into();
                        }
                        BgMsg::AllConnectionsClosed => {
                            self.active_connections.clear();
                            self.status = "已断开全部连接".into();
                        }
                        BgMsg::GeoUpdated(result) => {
                            self.geo_updating = false;
                            match result {
                                Ok(msg) => self.status = msg,
                                Err(err) => self.status = format!("Geo 规则更新失败: {err}"),
                            }
                        }
                        BgMsg::SourceHomogeneityAnalyzed(Ok(reports)) => {
                            self.homogeneity_busy = false;
                            let redundant_count = reports.iter().filter(|r| r.is_redundant).count();
                            self.homogeneity_status = Some(format!(
                                "分析完成：共检测 {} 个订阅源，发现 {} 个高度同质化冗余源",
                                reports.len(),
                                redundant_count
                            ));
                            self.homogeneity_selected = reports
                                .iter()
                                .filter(|report| report.is_redundant)
                                .map(|report| report.source_id.clone())
                                .collect();
                            self.homogeneity_reports = Some(reports);
                            self.show_homogeneity_dialog = true;
                        }
                        BgMsg::SourceHomogeneityAnalyzed(Err(e)) => {
                            self.homogeneity_busy = false;
                            self.status = format!("分析订阅源同质化失败: {e}");
                        }
                        BgMsg::SourceHomogeneityPruned(Ok(res)) => {
                            self.homogeneity_busy = false;
                            let action = if res.deleted_records {
                                "删除"
                            } else {
                                "禁用"
                            };
                            self.status =
                                format!("✓ 已成功{} {} 个同质化订阅源", action, res.pruned_count);
                            self.request_full_reload();
                            self.analyze_source_homogeneity();
                        }
                        BgMsg::SourceHomogeneityPruned(Err(e)) => {
                            self.homogeneity_busy = false;
                            self.status = format!("反向剔除同质化源失败: {e}");
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if self.proxy_status.state != ProxyState::Running {
            if self.traffic_up_rate > 0
                || self.traffic_down_rate > 0
                || !self.active_connections.is_empty()
                || !self.traffic_history.is_empty()
            {
                self.traffic_up_rate = 0;
                self.traffic_down_rate = 0;
                self.active_connections.clear();
                self.traffic_history.clear();
                self.traffic_prev_sample = None;
                self.traffic_polling = false;
            }
        }
        if need_repaint {
            ctx.request_repaint();
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    pub(crate) fn open_create_form(&mut self) {
        self.editing_id = None;
        self.form_name.clear();
        self.form_url.clear();
        self.form_enabled = true;
        self.form_remark.clear();
        self.show_source_form = true;
    }

    pub(crate) fn open_edit_form(&mut self, source: &Source) {
        self.editing_id = Some(source.id.clone());
        self.form_name = source.name.clone();
        self.form_url = source.url_template.clone();
        self.form_enabled = source.enabled;
        self.form_remark = source.remark.clone().unwrap_or_default();
        self.show_source_form = true;
    }

    pub(crate) fn save_source_form(&mut self) {
        if self.form_name.trim().is_empty() || self.form_url.trim().is_empty() {
            self.status = "名称和 URL 不能为空".into();
            return;
        }
        let db = self.db.clone();
        let tx = self.tx.clone();
        let editing = self.editing_id.clone();
        let name = self.form_name.trim().to_string();
        let url = self.form_url.trim().to_string();
        let enabled = self.form_enabled;
        let remark = nonempty_opt(&self.form_remark);
        self.busy = true;
        self.show_source_form = false;
        self.rt.spawn(async move {
            let res = if let Some(id) = editing {
                db.update_source(
                    &id,
                    UpdateSource {
                        name: Some(name),
                        url_template: Some(url),
                        enabled: Some(enabled),
                        remark,
                        push_disabled: None,
                    },
                )
                .await
                .map(|_| "订阅源已更新".into())
                .map_err(|e| e.to_string())
            } else {
                db.create_source(CreateSource {
                    name,
                    url_template: url,
                    enabled,
                    remark,
                })
                .await
                .map(|source| {
                    // 新增才串这条链（改现有源不重测）。开关在接收端判。
                    let _ = tx.send(BgMsg::SourceAdded(source.id, source.name));
                    "订阅源已创建".to_string()
                })
                .map_err(|e| e.to_string())
            };
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    /// 粘进来的一律走这里：识别 → 认出来就建源，后面那条"拉取 → 只测这一源"的链复用
    /// `save_source_form`（和手工新增同一条路，不另开一份）。文本由调用方给 ——
    /// 事件本身就是 egui-winit 读过剪贴板才发出来的，这里不再自己开一次剪贴板。
    pub(crate) fn apply_pasted_text(&mut self, text: &str) {
        let existing = self
            .sources
            .iter()
            .map(|source| (source.name.clone(), source.url_template.clone()))
            .collect::<Vec<_>>();
        let Some(intake) = paste::intake(text, &existing, self.settings.speedtest_added_source)
        else {
            self.status = "剪贴板里没有文本".into();
            return;
        };
        if let Some((name, url)) = intake.create {
            self.editing_id = None;
            self.form_name = name;
            self.form_url = url;
            self.form_enabled = true;
            self.form_remark = "Ctrl+V 粘贴加入".to_string();
            self.show_source_form = false;
            self.save_source_form();
        }
        self.paste_outcome = Some(intake.outcome);
    }

    pub(crate) fn delete_source(&mut self, id: String) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.status = "正在删除订阅源...".into();
        self.rt.spawn(async move {
            let res = db
                .delete_source(&id)
                .await
                .map(|ok| {
                    if ok {
                        "订阅源及其关联节点已删除".into()
                    } else {
                        "订阅源不存在".into()
                    }
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: false,
            });
        });
    }

    pub(crate) fn toggle_source_enabled(&mut self, id: String, enabled: bool) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.status = if enabled {
            "正在启用订阅源...".into()
        } else {
            "正在禁用订阅源...".into()
        };
        self.rt.spawn(async move {
            let result = db
                .update_source(
                    &id,
                    UpdateSource {
                        name: None,
                        url_template: None,
                        enabled: Some(enabled),
                        remark: None,
                        push_disabled: None,
                    },
                )
                .await
                .map(|_| {
                    if enabled {
                        "订阅源已启用".into()
                    } else {
                        "订阅源已禁用".into()
                    }
                })
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result,
                unlock: false,
            });
        });
    }

    /// 切换该源的节点是否参与“推送可用节点”。
    pub(crate) fn toggle_source_push(&mut self, id: String, push_disabled: bool) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.status = if push_disabled {
            "正在把该源标记为不推送...".into()
        } else {
            "正在恢复该源的推送...".into()
        };
        self.rt.spawn(async move {
            let result = db
                .update_source(
                    &id,
                    UpdateSource {
                        name: None,
                        url_template: None,
                        enabled: None,
                        remark: None,
                        push_disabled: Some(push_disabled),
                    },
                )
                .await
                .map(|_| {
                    if push_disabled {
                        "该源已标记为不推送，其节点不会出现在推送里".into()
                    } else {
                        "该源已恢复推送".into()
                    }
                })
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result,
                unlock: false,
            });
        });
    }

    pub(crate) fn export_sources_to_clipboard(&mut self) {
        let sources: Vec<CreateSource> = self
            .sources
            .iter()
            .map(|source| CreateSource {
                name: source.name.clone(),
                url_template: source.url_template.clone(),
                enabled: source.enabled,
                remark: source.remark.clone(),
            })
            .collect();
        match serde_json::to_string_pretty(&sources)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.set_text(json))
                    .map_err(|error| error.to_string())
            }) {
            Ok(()) => self.status = format!("已导出 {} 个订阅源到剪贴板", sources.len()),
            Err(error) => self.status = format!("导出订阅源失败: {error}"),
        }
    }

    pub(crate) fn import_sources_from_clipboard(&mut self) {
        let text = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => text,
            Err(error) => {
                self.status = format!("读取剪贴板失败: {error}");
                return;
            }
        };
        let sources: Vec<CreateSource> = match serde_json::from_str::<Vec<CreateSource>>(&text) {
            Ok(sources) if !sources.is_empty() => sources,
            Ok(_) => {
                self.status = "订阅源导入内容为空".into();
                return;
            }
            Err(error) => {
                self.status = format!("订阅源导入格式错误: {error}");
                return;
            }
        };
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在导入订阅源...".into();
        self.rt.spawn(async move {
            let result = async {
                let existing = db.list_sources().await?;
                let mut imported = 0usize;
                let mut skipped = 0usize;
                for source in sources {
                    if existing.iter().any(|current| {
                        current.name == source.name && current.url_template == source.url_template
                    }) {
                        skipped += 1;
                        continue;
                    }
                    db.create_source(source).await?;
                    imported += 1;
                }
                Ok::<_, anyhow::Error>(format!(
                    "订阅源导入完成：新增 {imported}，跳过重复 {skipped}"
                ))
            }
            .await
            .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result,
                unlock: true,
            });
        });
    }

    pub(crate) fn analyze_source_homogeneity(&mut self) {
        if self.homogeneity_busy {
            return;
        }
        self.homogeneity_busy = true;
        self.status = "正在分析订阅源同质化与节点重合率...".into();
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = db
                .analyze_source_homogeneity()
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SourceHomogeneityAnalyzed(res));
        });
    }

    /// 按勾选执行反向剔除：`ids` 来自同质化对话框的复选框，`delete_records` 决定删除还是仅禁用。
    pub(crate) fn prune_homogenized_sources(&mut self, ids: Vec<String>, delete_records: bool) {
        if self.homogeneity_busy {
            return;
        }
        if ids.is_empty() {
            self.status = "请先勾选需要剔除的订阅源".into();
            return;
        }

        self.homogeneity_busy = true;
        let action = if delete_records { "删除" } else { "禁用" };
        self.status = format!("正在批量{} {} 个订阅源...", action, ids.len());
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = db
                .prune_homogenized_sources(&ids, delete_records)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SourceHomogeneityPruned(res));
        });
    }

    pub(crate) fn fetch_one(&mut self, id: String) {
        if self.active_source_fetches.contains(&id) || self.tasks.is_source_active(&id) {
            self.status = "该订阅源正在拉取，请等待当前请求结束".into();
            return;
        }
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        self.active_source_fetches.insert(id.clone());
        self.status = "正在拉取订阅源...".into();
        self.rt.spawn(async move {
            let res = collector.fetch_one(&id).await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::FetchOneDone {
                source_id: id,
                result: res,
            });
        });
    }

    pub(crate) fn fetch_one_via_proxy(&mut self, id: String) {
        let Some(proxy_url) = self.subscription_proxy_url() else {
            self.status = "请先启动代理，再使用代理拉取订阅".into();
            return;
        };
        if self.active_source_fetches.contains(&id) || self.tasks.is_source_active(&id) {
            self.status = "该订阅源正在拉取，请等待当前请求结束".into();
            return;
        }
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        self.active_source_fetches.insert(id.clone());
        self.status = "正在通过代理拉取订阅源...".into();
        self.rt.spawn(async move {
            let res = collector
                .fetch_one_with_proxy(&id, &proxy_url)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::FetchOneDone {
                source_id: id,
                result: res,
            });
        });
    }

    pub(crate) fn fetch_all(&mut self) {
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在拉取全部启用源...".into();
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let res = collector
                .fetch_all_enabled_with_progress(Some(move |done, total| {
                    let _ = progress_tx.send(BgMsg::FetchProgress { done, total });
                }))
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::FetchAllDone(res));
        });
    }

    pub(crate) fn fetch_all_via_proxy(&mut self) {
        let Some(proxy_url) = self.subscription_proxy_url() else {
            self.status = "请先启动代理，再使用代理拉取订阅".into();
            return;
        };
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在通过代理拉取全部启用源...".into();
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let res = collector
                .fetch_all_enabled_with_proxy(
                    &proxy_url,
                    Some(move |done, total| {
                        let _ = progress_tx.send(BgMsg::FetchProgress { done, total });
                    }),
                )
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::FetchAllDone(res));
        });
    }

    pub(crate) fn subscription_proxy_url(&self) -> Option<String> {
        self.proxy_status
            .local_proxy_active()
            .then(|| format!("http://127.0.0.1:{}", self.proxy_status.mixed_port))
    }

    /// 独立的 Gist 发现：翻页拉取 gist 搜索结果，可用的 gist 注册为源、节点进节点列表。
    pub(crate) fn discover_gists(&mut self) {
        let pages = self.gist_pages.clamp(1, 300);
        let queries: Vec<String> = crate::collector::GIST_DISCOVERY_QUERIES
            .iter()
            .zip(self.gist_query_flags.iter())
            .filter(|(_, on)| **on)
            .map(|(query, _)| query.to_string())
            .collect();
        if queries.is_empty() {
            self.status = "请先勾选至少一个 Gist 查询词".into();
            return;
        }
        let collector = self.collector.clone();
        let tx = self.tx.clone();
        let app_proxy = self.subscription_proxy_url();
        self.busy = true;
        self.status = format!(
            "正在发现 Gist（{}，最多扫描 {pages} 页）...",
            queries.join("+")
        );
        self.rt.spawn(async move {
            // 代理候选链：应用内代理 → Windows 系统代理（v2rayN 等）→ 直连。
            // gist.github.com 直连常被 TCP 阻断，前一档第 1 页拉取失败就换下一档。
            let mut candidates: Vec<Option<String>> = Vec::new();
            if let Some(url) = app_proxy {
                candidates.push(Some(url));
            }
            if let Ok(Some(url)) =
                tokio::task::spawn_blocking(crate::system_proxy::current_system_proxy_url).await
            {
                if !candidates.contains(&Some(url.clone())) {
                    candidates.push(Some(url));
                }
            }
            candidates.push(None);

            let describe = |candidate: &Option<String>| match candidate {
                Some(url) => format!("走代理 {url}"),
                None => "直连".to_string(),
            };
            // 逐个查询词扫描；seen 去重在库里全局生效，重复出现的 gist 不会重复注册。
            // 每档失败才换下一档；换档前的原因留进 earlier_failures，别让最后一档
            // （通常是直连）的报错把更接近成功的代理档错误盖掉。
            let mut earlier_failures: Vec<String> = Vec::new();
            let mut per_query: Vec<String> = Vec::new();
            let mut total_pages = 0usize;
            let mut total_registered = 0usize;
            let mut total_nodes = 0usize;
            let mut total_skipped = 0usize;
            let mut total_rejected = 0usize;
            let mut total_pending = 0usize;

            for query in &queries {
                let mut last: Option<Result<crate::collector::GistDiscoverySummary, String>> =
                    None;
                for candidate in &candidates {
                    // 同一档原地重试再换档：经 Cloudflare Worker 型出口打 GitHub 实测约一半
                    // 请求在 TLS 握手中途被掐（整 5 秒断），成功的那些都是亚秒级 —— 是抖动，
                    // 不是这条链路死了。直连档没有重试价值（TCP 直接被阻断），所以重试次数
                    // 只给代理档用。
                    let retries = if candidate.is_some() {
                        GIST_PAGE_RETRIES
                    } else {
                        0
                    };
                    let mut outcome: Option<Result<crate::collector::GistDiscoverySummary, String>> =
                        None;
                    let mut attempts = 0usize;
                    for attempt in 0..=retries {
                        attempts = attempt + 1;
                        if attempt > 0 {
                            let _ = tx.send(BgMsg::Status(format!(
                                "正在重试 Gist 发现（{query}，{}，第 {attempts} 次）...",
                                describe(candidate)
                            )));
                            tokio::time::sleep(std::time::Duration::from_secs(attempt as u64))
                                .await;
                        }
                        let _ = tx.send(BgMsg::Status(format!(
                            "正在发现 Gist（{query}，最多 {pages} 页，{}）...",
                            describe(candidate)
                        )));
                        let current = match candidate {
                            Some(url) => match collector.with_proxy(url) {
                                Ok(proxied) => {
                                    proxied.discover_gists(query, pages).await.map_err(|e| e.to_string())
                                }
                                Err(error) => Err(error.to_string()),
                            },
                            None => collector
                                .discover_gists(query, pages)
                                .await
                                .map_err(|e| e.to_string()),
                        };
                        let flaky = matches!(
                            &current,
                            Ok(summary)
                                if summary.pages_scanned <= 1
                                    && summary.stopped_reason.contains("拉取失败")
                        );
                        outcome = Some(current);
                        if !flaky {
                            break;
                        }
                    }
                    let outcome = outcome.unwrap_or(Err("未发起任何请求".into()));
                    let retry_with_next = matches!(
                        &outcome,
                        Ok(summary)
                            if summary.pages_scanned <= 1
                                && summary.stopped_reason.contains("拉取失败")
                    );
                    if retry_with_next {
                        let reason = match &outcome {
                            Ok(summary) => summary.stopped_reason.clone(),
                            Err(error) => error.clone(),
                        };
                        earlier_failures.push(format!(
                            "{query} {}（试了 {attempts} 次）：{}",
                            describe(candidate),
                            reason
                        ));
                    }
                    last = Some(outcome);
                    if !retry_with_next {
                        break;
                    }
                }
                match last.unwrap_or_else(|| Err("未尝试任何代理候选".into())) {
                    Ok(summary) => {
                        total_pages += summary.pages_scanned;
                        total_registered += summary.registered;
                        total_nodes += summary.nodes;
                        total_skipped += summary.skipped;
                        total_rejected += summary.rejected;
                        total_pending += summary.pending;
                        per_query.push(format!(
                            "{query}：新增 {} 源（{} 节点），{}",
                            summary.registered, summary.nodes, summary.stopped_reason
                        ));
                    }
                    Err(error) => per_query.push(format!("{query}：{error}")),
                }
            }

            let mut message = format!(
                "Gist 发现完成：扫描 {} 页，新增 {} 个源（{} 节点），跳过已有 {}，无节点丢弃 {}，待重试 {}；{}",
                total_pages,
                total_registered,
                total_nodes,
                total_skipped,
                total_rejected,
                total_pending,
                per_query.join("；")
            );
            if !earlier_failures.is_empty() {
                message
                    .push_str(&format!("；此前尝试：{}", earlier_failures.join("；")));
            }
            let _ = tx.send(BgMsg::GistDiscoveryDone(Ok(message)));
        });
    }

    pub(crate) fn speedtest_ids(&mut self, ids: Option<Vec<String>>, label: &str) {
        let st = self.speedtester.clone();
        let opts = self.speed_opts();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = label.into();
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let res = st
                .test_many_with_progress(
                    ids,
                    opts,
                    Some(move |progress| {
                        let _ = progress_tx.send(BgMsg::SpeedProgress(progress));
                    }),
                )
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SpeedDone(res));
        });
    }

    pub(crate) fn speedtest_all_sources(&mut self) {
        let speedtester = self.speedtester.clone();
        let opts = self.speed_opts();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在并发测试所有订阅节点...".into();
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let result = speedtester
                .test_all_sources_with_progress(
                    opts,
                    Some(move |done, total| {
                        let _ = progress_tx.send(BgMsg::SourceSpeedProgress { done, total });
                    }),
                )
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::SpeedDone(result));
        });
    }

    pub(crate) fn speedtest_group(&mut self, group: NodeGroupSummary) {
        let db = self.db.clone();
        let speedtester = self.speedtester.clone();
        let opts = self.speed_opts();
        let tx = self.tx.clone();
        let protocol = nonempty_opt(&self.protocol_filter);
        let keyword = nonempty_opt(&self.keyword);
        let latency = if self.latency_filter == "all" {
            None
        } else {
            Some(self.latency_filter.clone())
        };
        self.busy = true;
        self.status = format!(
            "正在测速订阅“{}”（{} 个节点）...",
            group.source_name, group.total
        );
        self.rt.spawn(async move {
            let result = async {
                let ids = db
                    .list_node_ids_for_group(
                        group.source_id.as_deref(),
                        protocol.as_deref(),
                        keyword.as_deref(),
                        latency.as_deref(),
                    )
                    .await?;
                if ids.is_empty() {
                    anyhow::bail!("当前分组没有符合筛选条件的节点");
                }
                let progress_tx = tx.clone();
                speedtester
                    .test_many_with_progress(
                        Some(ids),
                        opts,
                        Some(move |progress| {
                            let _ = progress_tx.send(BgMsg::SpeedProgress(progress));
                        }),
                    )
                    .await
            }
            .await
            .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::SpeedDone(result));
        });
    }

    pub(crate) fn import_from_clipboard(&mut self) {
        let text = match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) => text.trim().to_string(),
            Err(error) => {
                self.status = format!("读取剪贴板失败: {error}");
                return;
            }
        };
        if text.is_empty() {
            self.status = "剪贴板为空".into();
            return;
        }
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在从剪贴板解析导入...".into();
        self.rt.spawn(async move {
            // No source_id: clipboard nodes are not tied to a subscription.
            let nodes = crate::parser::parse_subscription(&text, None);
            let parsed = nodes.len();
            let res = if parsed == 0 {
                Err("未从剪贴板解析出任何节点".to_string())
            } else {
                db.import_nodes(&nodes)
                    .await
                    .map(|stored| format!("剪贴板导入完成：解析 {parsed}，入库 {stored}"))
                    .map_err(|e| e.to_string())
            };
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn speedtest_page(&mut self) {
        let ids: Vec<String> = self.nodes.iter().map(|n| n.id.clone()).collect();
        if ids.is_empty() {
            self.status = "当前页没有节点".into();
            return;
        }
        let n = ids.len();
        self.speedtest_ids(Some(ids), &format!("正在真实代理验证本页 {n} 个节点..."));
    }

    pub(crate) fn speedtest_one(&mut self, id: String) {
        let st = self.speedtester.clone();
        let opts = self.speed_opts();
        let tx = self.tx.clone();
        self.status = "正在测速节点...".into();
        self.rt.spawn(async move {
            let res = st
                .test_one(&id, opts)
                .await
                .map(|r| SpeedTestSummary {
                    total: 1,
                    ok: if r.ok { 1 } else { 0 },
                    fail: if r.ok { 0 } else { 1 },
                    avg_ms: r.latency_ms.map(|v| v as f64),
                    avg_download_kbps: r.download_kbps,
                    results: vec![r],
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SpeedDone(res));
        });
    }

    #[allow(dead_code)]
    pub(crate) fn select_proxy_node(&mut self, id: String, name: String) {
        self.selected_proxy_node_id = Some(id);
        self.selected_proxy_node_name = Some(name);
        self.tab = Tab::Proxy;
        self.status = "已选择代理节点".into();
    }

    pub(crate) fn switch_proxy_node(&mut self, id: String, name: String, reason: &str) {
        let from_id = self.selected_proxy_node_id.clone();
        let from_name = self.selected_proxy_node_name.clone();
        if from_id.as_deref() != Some(id.as_str()) {
            let db = self.db.clone();
            let record = crate::db::telemetry::SwitchRecord {
                from_node: from_id,
                from_name,
                to_node: Some(id.clone()),
                to_name: Some(name.clone()),
                reason: reason.to_string(),
                latency_ms: None,
            };
            self.rt.spawn(async move {
                let _ = db.record_egress_switch(&record).await;
            });
        }
        if self.selected_proxy_node_id.as_deref() == Some(&id)
            && self.switching_node_id.is_none()
            && matches!(
                self.proxy_status.state,
                ProxyState::Running | ProxyState::Degraded
            )
        {
            return;
        }
        let running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        self.selected_proxy_node_id = Some(id.clone());
        self.selected_proxy_node_name = Some(name.clone());
        self.router.set_active(&id);
        self.switching_node_id = Some(id.clone());
        self.proxy_switch_seq = self.proxy_switch_seq.wrapping_add(1);
        let seq = self.proxy_switch_seq;

        if requires_tun_elevation(self.proxy_config.mode, platform::is_elevated()) {
            self.start_proxy();
            return;
        }

        // url-test 打开时，目标节点通常已经是 PROXY 组的成员：这时只要让内核换个
        // 成员（PUT /proxies/PROXY），不需要重写配置 + 热重载掐断所有连接。
        if running
            && self.proxy_config.url_test
            && self
                .kernel_groups
                .iter()
                .find(|group| group.name == "PROXY")
                .is_some_and(|group| group.members.iter().any(|member| *member == name))
        {
            self.select_kernel_member("PROXY".to_owned(), name.clone());
            self.switching_node_id = None;
            return;
        }

        let db = self.db.clone();
        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        let url_test = self.proxy_config.url_test;
        let url_test_size = self.proxy_config.url_test_size;
        let candidate_blacklist = self.router.blacklisted(std::time::Instant::now());
        let node_id = id.clone();
        self.status = if !running {
            format!("⚡ 正在极速异步启动代理并连接「{name}」...")
        } else {
            format!("⚡ 正在极速异步切换至「{name}」...")
        };
        self.rt.spawn(async move {
            let result = match db.get_node(&node_id).await {
                Ok(Some(node)) => {
                    Self::prime_url_test_candidates(
                        &db,
                        &manager,
                        &node,
                        url_test,
                        url_test_size,
                        &candidate_blacklist,
                    )
                    .await;
                    let res = if !running {
                        manager.connect(&node, None).await
                    } else {
                        manager.switch_node(&node).await
                    };
                    res.map(|_| {
                        if !running {
                            format!("代理已启动，已连接节点: {}", node.name)
                        } else {
                            format!("已平滑切换至节点: {}", node.name)
                        }
                    })
                    .map_err(|e| e.to_string())
                }
                Ok(None) => Err("节点不存在".to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(BgMsg::ProxySwitchDone {
                seq,
                node_id,
                result,
            });
        });
    }

    /// url-test 打开时，把测速前 N 名注入本次要生成的配置（启动/切换前都要刷新，
    /// 候选是随测速结果变的）。
    async fn prime_url_test_candidates(
        db: &Database,
        manager: &ProxyManager,
        node: &Node,
        url_test: bool,
        size: u16,
        blacklist: &[String],
    ) {
        if !url_test {
            return;
        }
        // 多取几条，扣掉被断路器拉黑的之后仍然凑够 N 个。
        match db
            .top_verified_nodes(size as usize + blacklist.len(), Some(&node.id))
            .await
        {
            Ok(candidates) => {
                let picked: Vec<Node> = candidates
                    .into_iter()
                    .filter(|item| !blacklist.iter().any(|bad| bad == &item.id))
                    .take(size as usize)
                    .collect();
                let taken = picked.len();
                let exported = manager.set_url_test_candidates(picked).await;
                tracing::info!(
                    taken,
                    exported,
                    blacklisted = blacklist.len(),
                    "url-test 候选注入完成"
                );
            }
            Err(error) => tracing::warn!(%error, "读取 url-test 候选节点失败"),
        }
    }

    /// 要放行的程序：本程序 + 已下载的内核 exe（按路径，重编译/换版本路径不变）。
    pub(crate) fn firewall_targets() -> Vec<std::path::PathBuf> {
        let mut programs = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            programs.push(exe);
        }
        let root = crate::kernel::managed_kernel_root();
        for kind in [
            crate::kernel::KernelKind::Mihomo,
            crate::kernel::KernelKind::Xray,
            crate::kernel::KernelKind::SingBox,
        ] {
            programs.push(root.join(kind.directory_name()).join(kind.executable_name()));
        }
        programs
    }

    /// 写防火墙允许规则。netsh 要管理员权限，非提权时会失败并把原因显示出来。
    pub(crate) fn apply_firewall_rules(&mut self) {
        let programs = Self::firewall_targets();
        let tx = self.tx.clone();
        self.status = "正在写入防火墙允许规则...".into();
        self.rt.spawn_blocking(move || {
            let result = crate::platform::firewall::ensure_all(&programs).map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::FirewallApplied(result));
        });
    }

    /// 「下载内核」弹框里点开始走这条：源与（本地导入时的）zip 路径都由弹框决定。
    pub(crate) fn update_kernel_from(
        &mut self,
        kind: KernelKind,
        source: crate::kernel::KernelSource,
        local_file: Option<std::path::PathBuf>,
    ) {
        if self.proxy_status.kernel_in_use(kind) {
            self.status = format!("停止代理后才能更新 {}", kind.display_name());
            return;
        }
        if source == crate::kernel::KernelSource::Local && local_file.is_none() {
            self.status = "本地导入要先选好 zip 文件".into();
            return;
        }
        let manager = self.kernel_manager.clone();
        let tx = self.tx.clone();
        let cancelled = self.kernel_cancel.clone();
        let proxy_url = self
            .proxy_status
            .local_proxy_active()
            .then(|| format!("http://127.0.0.1:{}", self.proxy_status.mixed_port));
        cancelled.store(false, Ordering::Relaxed);
        self.kernel_busy = true;
        self.kernel_progress = None;
        // 记住这次用的方式：弹框下次打开、以及 HTTP API 的更新都按它走。
        self.edit_kernel_source = source.id().to_string();
        self.save_settings();
        self.status = format!(
            "正在下载并校验 {}（{}）...",
            kind.display_name(),
            source.label()
        );
        self.rt.spawn(async move {
            let result = manager
                .update_with_options(
                    kind,
                    proxy_url,
                    source,
                    local_file,
                    cancelled,
                    {
                        let tx = tx.clone();
                        move |progress| {
                            let _ = tx.send(BgMsg::KernelProgress(progress));
                        }
                    },
                )
                .await
                .map(|result| result.message)
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::KernelUpdated(result));
        });
    }

    /// 代理配置进 UI 状态的唯一入口。
    ///
    /// 界面上已经没有「自动选路 url-test」开关了（2026-10-06 按用户要求撤掉，理由是他
    /// 那边内核 url-test 经常网络不稳定）。所以从库里读到的 `true` 一律归成 `false`：
    /// 留一条没有开关可关的路径，等于让旧存值悄悄决定行为。内核侧的 url-test 生成分支
    /// 还在（`proxy::mod` 有测试覆盖），要恢复只需把开关画回动作条。
    fn set_proxy_config(&mut self, mut config: MihomoConfigOptions) {
        config.url_test = false;
        self.proxy_config = config;
    }

    pub(crate) fn save_proxy_config(&mut self) {
        self.proxy_config.rules = optimize_traffic_rules(&self.proxy_config.rules);
        let manager = self.proxy_manager.clone();
        let db = self.db.clone();
        let options = self.proxy_config.clone();
        let tx = self.tx.clone();
        self.proxy_busy = true;
        self.status = "正在保存分流配置并应用...".into();
        self.rt.spawn(async move {
            let previous = manager.config().await;
            let result = match manager.reload_config(options.clone()).await {
                Ok(_) => match db.update_proxy_config(&options).await {
                    Ok(()) => Ok(options),
                    Err(error) => {
                        let _ = manager.reload_config(previous).await;
                        Err(error.to_string())
                    }
                },
                Err(error) => Err(error.to_string()),
            };
            let _ = tx.send(BgMsg::ProxyConfigSaved(result));
        });
    }

    /// 启动之前先算清楚"真会用的那个内核装没装"，没装就返回该下载哪个内核。
    ///
    /// 以前不拦：照样往下走，`ProxyManager` 里 `find_kernel_executable` 拿不到文件 →
    /// 代理进 Failed，状态栏只剩一行"未找到 xxx 可执行文件"，没人知道下一步点哪儿。
    /// 判定表本身在 `kernel::missing_kernel_for`（那张表有单测），这里只负责把
    /// "选中的节点 + 内核偏好 + 是否系统代理模式"喂给它。
    fn missing_kernel_for_start(&self) -> Option<KernelKind> {
        let node = self.selected_proxy_node_id.as_deref().and_then(|id| {
            self.proxy_nodes
                .iter()
                .chain(self.nodes.iter())
                .find(|summary| summary.id == id)
        })?;
        missing_kernel_for(
            &node.protocol,
            self.proxy_config.kernel_preference,
            self.proxy_config.mode == ProxyMode::System,
            |kind| find_kernel_executable(kind).is_some(),
        )
    }

    /// 勾选「节点故障自动切换」就立刻落库。以前这个开关只活在内存里，
    /// 重启回 false —— 用户以为开着，实际一次都不会切。
    pub(crate) fn set_auto_failover(&mut self, on: bool) {
        self.auto_failover = on;
        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db
                .update_settings(crate::models::UpdateSettings {
                    auto_failover: Some(on),
                    ..Default::default()
                })
                .await;
        });
    }

    /// 打开某个内核的下载弹框（下载方式沿用上次记下的那个）。
    pub(crate) fn open_kernel_download(&mut self, kind: KernelKind) {
        self.kernel_dialog_kind = Some(kind);
        self.kernel_dialog_source =
            crate::kernel::source_from_id(&self.settings.kernel_source)
                .id()
                .to_string();
        self.show_kernel_dialog = true;
    }

    /// 盘上到底有没有任何一个内核。**同步查文件系统**，不用 `self.kernels`：
    /// 那份列表是异步刷新的，第一帧时还是空的，拿它判断会在装好内核的机器上
    /// 误判成"没内核"然后自己开始下载。
    pub(crate) fn kernel_installed() -> bool {
        [
            crate::kernel::KernelKind::Mihomo,
            crate::kernel::KernelKind::SingBox,
            crate::kernel::KernelKind::Xray,
        ]
        .into_iter()
        .any(|kind| crate::kernel::find_kernel_executable(kind).is_some())
    }

    /// 缓存版：5 秒之内的答案直接用。徽章和自动预备都读这个，不各查各的盘。
    pub(crate) fn kernel_installed_cached(&mut self) -> bool {
        const TTL: std::time::Duration = std::time::Duration::from_secs(5);
        if let Some((value, at)) = self.kernel_cache {
            if at.elapsed() < TTL {
                return value;
            }
        }
        let value = Self::kernel_installed();
        self.kernel_cache = Some((value, std::time::Instant::now()));
        value
    }

    /// 「预备」状态下自己把缺的内核下载下来，下载方式用他设置里记着的那个。
    pub(crate) fn maybe_auto_prepare(&mut self) {
        let proxy_active = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded | ProxyState::Starting
        );
        let has_kernel = self.kernel_installed_cached();
        if !prepare::should_download_kernel(
            self.settings.auto_prepare_kernel,
            has_kernel,
            proxy_active,
            self.kernel_busy,
            self.prepare_attempts,
            self.prepare_last_at.map(|at| at.elapsed()),
        ) {
            return;
        }
        self.prepare_attempts += 1;
        self.prepare_last_at = Some(std::time::Instant::now());
        let source = crate::kernel::source_from_id(&self.settings.kernel_source);
        // 先把要写的东西算成局部值：format! 里直接调 `self.kernel_installed_cached()`
        // 会和后面的可变借用打架（那一步本来就是可变操作）。
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|dir| dir.display().to_string()))
            .unwrap_or_else(|| "未知".to_string());
        crate::startup_log(&format!(
            "[预备] 第 {}/{} 次自动下载内核：方式={} 代理在跑={} 盘上已有内核={} 程序目录={}",
            self.prepare_attempts,
            prepare::MAX_AUTO_ATTEMPTS,
            source.label(),
            proxy_active,
            has_kernel,
            exe_dir
        ));
        self.status = format!(
            "预备状态：没装内核，自动下载 mihomo（第 {}/{} 次，{}）",
            self.prepare_attempts,
            prepare::MAX_AUTO_ATTEMPTS,
            source.label()
        );
        self.update_kernel_from(KernelKind::Mihomo, source, None);
    }

    /// 新增源拉取成功之后：取这个源的节点 id，回来再发起测速。
    fn queue_added_source_speedtest(&mut self, source_id: String, source_name: String) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let ids = db
                .list_node_ids_for_group(Some(&source_id), None, None, None)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::AddedSourceIds {
                source_id,
                source_name,
                ids,
            });
        });
    }

    pub(crate) fn start_proxy(&mut self) {
        if self.selected_proxy_node_id.is_none() && self.proxy_config.url_test {
            // 自动选路开着时左键点卡片不改锚点（出口归内核），所以"必须先选节点"会把
            // 启动按钮永久锁死。锚点只是生成配置的中心，取测速最优的那个就行。
            self.status = "正在挑选自动选路的锚点节点...".into();
            let db = self.db.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let outcome = match db.top_verified_nodes(2, None).await {
                    Ok(nodes) => match nodes.into_iter().next() {
                        Some(node) => Ok((node.id, node.name)),
                        None => Err("库里没有测速通过的节点，先跑一轮测速".to_string()),
                    },
                    Err(error) => Err(error.to_string()),
                };
                let _ = tx.send(BgMsg::AutoAnchorStart(outcome));
            });
            return;
        }
        let Some(node_id) = self.selected_proxy_node_id.clone() else {
            self.status = "请先从节点列表选择代理节点".into();
            return;
        };
        // 内核没装就别往下走：直接把该内核的下载窗口开在他面前。放在提权之前，
        // 免得先弹一次 UAC、升完级才发现根本没内核。
        if let Some(kind) = self.missing_kernel_for_start() {
            self.open_kernel_download(kind);
            self.status = format!(
                "启动需要 {} 内核，但它还没有安装 —— 已为你打开下载窗口",
                kind.display_name()
            );
            return;
        }
        if requires_tun_elevation(self.proxy_config.mode, platform::is_elevated()) {
            let config = match serde_json::to_vec(&self.proxy_config) {
                Ok(config) => URL_SAFE_NO_PAD.encode(config),
                Err(error) => {
                    self.status = format!("序列化 TUN 配置失败: {error}");
                    return;
                }
            };
            let args = vec![
                "--elevated-proxy-config".into(),
                config,
                "--start-proxy-node".into(),
                node_id,
            ];
            match platform::request_elevation(&args) {
                Ok(()) => {
                    self.status = "已请求管理员权限，正在切换到提权窗口...".into();
                    self.close_after_elevation = true;
                }
                Err(error) => self.status = format!("请求管理员权限失败: {error}"),
            }
            return;
        }
        let db = self.db.clone();
        let manager = self.proxy_manager.clone();
        let mode = self.proxy_config.mode;
        let options = self.proxy_config.clone();
        let candidate_blacklist = self.router.blacklisted(std::time::Instant::now());
        let tx = self.tx.clone();
        self.proxy_busy = true;
        self.status = "正在启动代理...".into();
        self.rt.spawn(async move {
            let result = match db.get_node(&node_id).await {
                Ok(Some(node)) => {
                    Self::prime_url_test_candidates(
                        &db,
                        &manager,
                        &node,
                        options.url_test,
                        options.url_test_size,
                        &candidate_blacklist,
                    )
                    .await;
                    let previous = manager.config().await;
                    let operation = match manager.update_config(options.clone()).await {
                        Ok(()) => match db.update_proxy_config(&options).await {
                            Ok(()) => manager.start(&node).await,
                            Err(error) => {
                                let _ = manager.update_config(previous).await;
                                Err(error)
                            }
                        },
                        Err(error) => Err(error),
                    };
                    match operation {
                        Ok(()) => {
                            let status = manager.status().await;
                            if status.state == ProxyState::Degraded {
                                let kernel = status.kernel.map(|k| match k {
                                    KernelKind::Mihomo => "mihomo",
                                    KernelKind::SingBox => "sing_box",
                                    KernelKind::Xray => "xray",
                                });
                                let _ = db
                                    .update_node_speed(
                                        &node.id,
                                        false,
                                        None,
                                        None,
                                        kernel,
                                        Some(GOOGLE_PROXY_TEST_KIND),
                                    )
                                    .await;
                                Ok(format!(
                                    "当前节点连通性异常（已自动标记为不可用）：{}",
                                    status.message
                                ))
                            } else if mode == ProxyMode::Tun {
                                Ok("代理已启动，TUN 模式已启用".into())
                            } else {
                                Ok("代理已启动，系统代理已启用".into())
                            }
                        }
                        Err(error) => {
                            let message = error.to_string();
                            if is_node_connectivity_failure(&message) {
                                let kernel =
                                    manager.status().await.kernel.map(|kernel| match kernel {
                                        KernelKind::Mihomo => "mihomo",
                                        KernelKind::SingBox => "sing_box",
                                        KernelKind::Xray => "xray",
                                    });
                                let _ = db
                                    .update_node_speed(
                                        &node.id,
                                        false,
                                        None,
                                        None,
                                        kernel,
                                        Some(GOOGLE_PROXY_TEST_KIND),
                                    )
                                    .await;
                                Err(anyhow::anyhow!(
                                    "{message}; 已将该节点标记为不可用，请切换节点"
                                ))
                            } else {
                                Err(error)
                            }
                        }
                    }
                }
                Ok(None) => Err(anyhow::anyhow!("节点不存在")),
                Err(error) => Err(error),
            }
            .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::ProxyOp(result));
        });
    }

    pub(crate) fn stop_proxy(&mut self) {
        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        self.proxy_busy = true;
        self.status = "正在停止代理...".into();
        self.rt.spawn(async move {
            let result = manager
                .stop()
                .await
                .map(|_| "代理已停止".into())
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::ProxyOp(result));
        });
    }

    pub(crate) fn poll_traffic_snapshot(&mut self) {
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(1200))
                .build()
            {
                Ok(c) => c,
                Err(_) => {
                    let _ = tx.send(BgMsg::TrafficPollDone);
                    return;
                }
            };
            let url = format!("http://127.0.0.1:{port}/connections");
            let resp = crate::proxy::with_controller_auth(client.get(&url), &secret)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    if let Ok(data) = r.json::<MihomoConnectionsResponse>().await {
                        let mut connections = Vec::with_capacity(data.connections.len());
                        for c in data.connections {
                            let (network, host, dest, process) = match c.metadata {
                                Some(m) => {
                                    let net = m.network.unwrap_or_else(|| "tcp".into());
                                    let h = m.host.unwrap_or_default();
                                    let dst_ip = m.destination_ip.unwrap_or_default();
                                    let dst_port = m.destination_port.unwrap_or_default();
                                    let dest = if !dst_ip.is_empty() && !dst_port.is_empty() {
                                        format!("{dst_ip}:{dst_port}")
                                    } else if !dst_port.is_empty() {
                                        dst_port
                                    } else {
                                        dst_ip
                                    };
                                    let proc =
                                        m.process.or(m.process_path).unwrap_or_else(|| "-".into());
                                    (net, h, dest, proc)
                                }
                                None => ("tcp".into(), String::new(), String::new(), "-".into()),
                            };
                            let rule_str = match (&c.rule, &c.rule_payload) {
                                (Some(r), Some(p)) if !p.is_empty() => format!("{r}({p})"),
                                (Some(r), _) => r.clone(),
                                _ => "-".into(),
                            };
                            let target_str = c
                                .rule_payload
                                .clone()
                                .filter(|payload| !payload.is_empty())
                                .unwrap_or_else(|| "-".into());
                            connections.push(ActiveConnectionInfo {
                                id: c.id,
                                network,
                                host,
                                destination: dest,
                                process,
                                rule: rule_str,
                                target: target_str,
                                upload: c.upload,
                                download: c.download,
                            });
                        }
                        let _ = tx.send(BgMsg::TrafficSnapshot {
                            now: std::time::Instant::now(),
                            upload_total: data.upload_total,
                            download_total: data.download_total,
                            connections,
                        });
                    } else {
                        let _ = tx.send(BgMsg::TrafficPollDone);
                    }
                }
                _ => {
                    let _ = tx.send(BgMsg::TrafficPollDone);
                }
            }
        });
    }

    /// 每 5 秒读一次内核的 group 状态。`/proxies` 会带每个节点的历史延迟，
    /// 比 `/connections` 重，所以不跟着流量轮询的 1 秒节奏走。
    pub(crate) fn poll_kernel_groups(&mut self) {
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(1500))
                .build()
            {
                Ok(client) => client,
                Err(_) => return,
            };
            let url = format!("http://127.0.0.1:{port}/proxies");
            let response = crate::proxy::with_controller_auth(client.get(&url), &secret)
                .send()
                .await
                .ok()
                .filter(|r| r.status().is_success());
            let Some(response) = response else { return };
            let Ok(data) = response.json::<MihomoProxiesResponse>().await else {
                return;
            };
            let mut groups = Vec::new();
            let mut latencies = Vec::new();
            for (name, item) in data.proxies {
                let kind = item.kind.unwrap_or_default().to_lowercase();
                if let Some(history) = item.history.as_deref() {
                    // 实测 history 是"从旧到新"，最后一条才是刚测出来的。
                    if let Some(entry) = history.last() {
                        latencies.push(KernelLatency {
                            name: name.clone(),
                            delay_ms: entry.delay,
                            alive: item.alive.unwrap_or(true),
                        });
                    }
                }
                if !matches!(
                    kind.as_str(),
                    "select" | "selector" | "url-test" | "urltest"
                ) {
                    continue;
                }
                groups.push(KernelGroup {
                    name,
                    kind,
                    now: item.now.filter(|value| !value.is_empty()),
                    members: item.all.unwrap_or_default(),
                });
            }
            groups.sort_by(|a, b| a.name.cmp(&b.name));
            let _ = tx.send(BgMsg::KernelGroups(groups, latencies));
        });
    }

    /// 把内核 url-test 的成员延迟喂给调度器：两边共用一份延迟口径，app 层不再自己
    /// 逐个探测。配置里的成员名可能带 ` #2` 去重后缀，先剥掉再按节点名匹配。
    fn feed_router_kernel_latency(&mut self, latencies: Vec<KernelLatency>) {
        if latencies.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let mut delay_by_name: HashMap<String, u64> = HashMap::new();
        for item in &latencies {
            if item.delay_ms == 0 {
                continue;
            }
            let base = item
                .name
                .split(" #")
                .next()
                .unwrap_or(item.name.as_str())
                .to_owned();
            delay_by_name.insert(base, item.delay_ms);
        }
        let updates: Vec<(String, u64)> = self
            .proxy_nodes
            .iter()
            .chain(self.nodes.iter())
            .filter_map(|node| {
                delay_by_name
                    .get(&node.name)
                    .map(|delay| (node.id.clone(), *delay))
            })
            .collect();
        for (id, delay) in updates {
            self.router.note_kernel_latency(&id, delay, now);
        }
    }

    /// 让内核把某个组切到指定成员：`PUT /proxies/{group}`，不重写配置、不掐连接。
    pub(crate) fn select_kernel_member(&mut self, group: String, target: String) {
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        self.status = format!("正在让内核切换 {group} → {target}...");
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("http://127.0.0.1:{port}/proxies/{group}");
            let result = crate::proxy::with_controller_auth(client.put(&url), &secret)
                .json(&serde_json::json!({ "name": target }))
                .send()
                .await
                .map_err(|error| error.to_string())
                .and_then(|response| {
                    if response.status().is_success() {
                        Ok(())
                    } else {
                        Err(format!("内核拒绝切换: {}", response.status()))
                    }
                });
            let _ = tx.send(BgMsg::OpDone {
                result: result.map(|_| format!("已切换 {group} → {target}")),
                unlock: false,
            });
        });
    }

    pub(crate) fn close_connection(&mut self, id: String) {
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let _ = crate::proxy::with_controller_auth(
                client.delete(format!("http://127.0.0.1:{port}/connections/{id}")),
                &secret,
            )
            .send()
            .await;
            let _ = tx.send(BgMsg::ConnectionClosed(id));
        });
    }

    pub(crate) fn close_all_connections(&mut self) {
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let _ = crate::proxy::with_controller_auth(
                client.delete(format!("http://127.0.0.1:{port}/connections")),
                &secret,
            )
            .send()
            .await;
            let _ = tx.send(BgMsg::AllConnectionsClosed);
        });
    }

    pub(crate) fn update_geo_data(&mut self) {
        if self.geo_updating {
            return;
        }
        self.geo_updating = true;
        let running = matches!(self.proxy_status.state, ProxyState::Running);
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        if !running {
            // 内核没在跑就没有接口可问。这时直接把两个库下到工作目录，下次启动即用 —— 
            // 否则就是"不启动代理反而更新不了"那个死结。
            self.status = "代理未运行，正在直接下载 GeoIP / GeoSite 规则文件...".into();
            self.rt.spawn(async move {
                let report = crate::geo::mihomo_dir();
                let result = crate::geo::provision(&report, None, true)
                    .await
                    .map(|detail| format!("内核未运行，已直接更新到 {}：{detail}", report.display()));
                let _ = tx.send(BgMsg::GeoUpdated(result.map_err(|error| error.to_string())));
            });
            return;
        }
        self.status = "正在向内核请求更新 GeoIP / GeoSite 规则文件...".into();
        self.rt.spawn(async move {
            // 这个调用是**同步**的：内核要自己下完两个库（11MB + 16MB）才回话，
            // 30 秒会把还在正常下载的情况误判成失败。
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(BgMsg::GeoUpdated(Err(e.to_string())));
                    return;
                }
            };
            let resp = crate::proxy::with_controller_auth(
                client.post(format!("http://127.0.0.1:{port}/configs/geo")),
                &secret,
            )
            .send()
            .await;
            let res = match resp {
                Ok(r) if r.status().is_success() => {
                    Ok("Geo 规则数据库更新指令已成功执行并重新载入".into())
                }
                // 内核会把失败原因写在响应体里（哪家镜像挂了、超时还是半截），必须带出来。
                Ok(r) => {
                    let status = r.status();
                    let body = r.text().await.unwrap_or_default();
                    let detail = body.trim();
                    Err(if detail.is_empty() {
                        format!("更新接口返回状态码: {status}")
                    } else {
                        format!("内核自己下载失败（{status}）：{detail}")
                    })
                }
                // reqwest 的错误原因挂在 source 链上，`{e}` 只会打出"error sending request"。
                Err(e) => Err(format!("请求 Geo 更新失败: {e:#}")),
            };
            let _ = tx.send(BgMsg::GeoUpdated(res));
        });
    }

    /// 命令面板里的可执行项。节点也进面板（模糊搜名字直接切出口），所以每帧动态生成。
    fn palette_items(&self) -> Vec<crate::gui::types::PaletteItem> {
        use crate::gui::types::{PaletteAction, PaletteItem};
        let mut items = vec![
            PaletteItem {
                title: "启动 / 停止代理".into(),
                hint: format!("{:?}", self.proxy_status.state),
                action: PaletteAction::ToggleProxy,
            },
            PaletteItem {
                title: "切换系统代理 / TUN".into(),
                hint: format!("{:?}", self.proxy_config.mode),
                action: PaletteAction::ToggleMode,
            },
            PaletteItem {
                title: "下载内核…".into(),
                hint: "打开内核下载弹框".into(),
                action: PaletteAction::DownloadKernel,
            },
            PaletteItem {
                title: "跳到 · 控制台".into(),
                hint: "代理状态与可用节点".into(),
                action: PaletteAction::Goto(Tab::Proxy),
            },
            PaletteItem {
                title: "跳到 · 节点池".into(),
                hint: "全量节点与测速".into(),
                action: PaletteAction::Goto(Tab::Nodes),
            },
            PaletteItem {
                title: "跳到 · 订阅源".into(),
                hint: "增删改查 · 拉取".into(),
                action: PaletteAction::Goto(Tab::Sources),
            },
            PaletteItem {
                title: "跳到 · 系统设置".into(),
                hint: "行为与自愈".into(),
                action: PaletteAction::Goto(Tab::Settings),
            },
            PaletteItem {
                title: "跳到 · 外部接口".into(),
                hint: "本地 HTTP API".into(),
                action: PaletteAction::Goto(Tab::Api),
            },
        ];
        for (label, panel) in [
            ("活跃连接", ProxyPanel::Connections),
            ("分流规则", ProxyPanel::Rules),
            ("DNS / TUN", ProxyPanel::Network),
            ("内核管理", ProxyPanel::Kernels),
            ("运行日志", ProxyPanel::Logs),
        ] {
            items.push(PaletteItem {
                title: format!("跳到 · {label}"),
                hint: "控制台子页".into(),
                action: PaletteAction::GotoProxyPanel(panel),
            });
        }
        for spec in crate::gui::theme::THEMES {
            items.push(PaletteItem {
                title: format!("强调色 · {}", spec.name),
                hint: "整套界面跟着变".into(),
                action: PaletteAction::Theme(spec.id),
            });
        }
        // 节点只列当前"可用节点"页那批（按延迟排好序），最多 40 条 ——
        // 把一万六千条全塞进面板既慢又没人看。
        for node in self.proxy_nodes.iter().take(40) {
            items.push(PaletteItem {
                title: format!("切到节点 · {}", node.name),
                hint: format!(
                    "{} · {}",
                    node.country.clone().unwrap_or_else(|| "—".into()),
                    node.latency_ms
                        .map(|ms| format!("{ms} ms"))
                        .unwrap_or_else(|| "未测".into())
                ),
                action: PaletteAction::UseNode(node.id.clone(), node.name.clone()),
            });
        }
        items
    }

    /// Ctrl+K 命令面板。"敲两个字"比在界面上找按钮快，也是把交互从后台脸里拉出来的一步
    /// （用户 2026-10-07：界面太古板）。
    pub(crate) fn ui_command_palette(&mut self, ctx: &egui::Context) {
        use crate::gui::types::PaletteAction;
        if !self.palette_open {
            return;
        }
        let items = self.palette_items();
        let query = self.palette_query.trim().to_lowercase();
        let matched: Vec<&crate::gui::types::PaletteItem> = items
            .iter()
            .filter(|item| {
                query.is_empty()
                    || item.title.to_lowercase().contains(&query)
                    || item.hint.to_lowercase().contains(&query)
            })
            .collect();
        if self.palette_selected >= matched.len() {
            self.palette_selected = 0;
        }
        let mut run: Option<PaletteAction> = None;
        let mut close = false;
        let want_focus = self.palette_needs_focus;
        let drop_from = ctx.input(|i| i.screen_rect()).height() * 0.18;

        egui::Window::new("命令面板")
            .id(egui::Id::new("vpn-parser-command-palette"))
            .title_bar(false)
            .movable(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, drop_from.max(60.0)])
            .frame(
                egui::Frame::popup(&ctx.style())
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_width(480.0);
                // 输入框必须是这一层里的第一个控件、前面没有任何条件 add：
                // 否则敲下第一个字符时后面那批控件的 auto-id 整体位移，TextEdit 换 id →
                // 焦点丢 + 微软拼音组合窗被销毁（就是"输一个字就卡"那个坑）。
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text("输入命令、页面或节点名…")
                        .desired_width(ui.available_width()),
                );
                crate::gui::widgets::note_text_field(ui, &response);
                if want_focus {
                    response.request_focus();
                    self.palette_needs_focus = false;
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                let (down, up, enter, escape) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowDown),
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                if down && !matched.is_empty() {
                    self.palette_selected = (self.palette_selected + 1) % matched.len();
                }
                if up && !matched.is_empty() {
                    self.palette_selected =
                        (self.palette_selected + matched.len() - 1) % matched.len();
                }
                if escape {
                    close = true;
                }
                if matched.is_empty() {
                    ui.label(
                        RichText::new("没有匹配的命令")
                            .small()
                            .color(theme::muted()),
                    );
                }
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 行是自绘的：把两列文字塞进 Button 的内容里做不到，
                        // 而自绘顺带让"选中"是一整条底色而不是一小块。
                        for (index, item) in matched.iter().enumerate() {
                            let selected_row = index == self.palette_selected;
                            let size = Vec2::new(ui.available_width(), 30.0);
                            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                            let painter = ui.painter_at(rect);
                            if selected_row {
                                painter.rect_filled(
                                    rect,
                                    CornerRadius::same(10),
                                    theme::panel_hover(),
                                );
                            }
                            painter.text(
                                egui::pos2(rect.min.x + 9.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &item.title,
                                egui::FontId::proportional(13.0),
                                theme::text_primary(),
                            );
                            painter.text(
                                egui::pos2(rect.max.x - 9.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                &item.hint,
                                egui::FontId::proportional(11.0),
                                theme::muted(),
                            );
                            if response.clicked() || (enter && selected_row) {
                                run = Some(item.action.clone());
                            }
                        }
                    });
                ui.add_space(2.0);
                ui.label(
                    RichText::new("↑↓ 选择 · Enter 执行 · Esc 关闭 · 再按 Ctrl+K 收起")
                        .small()
                        .color(theme::muted()),
                );
            });
        if close {
            self.palette_open = false;
        }
        if let Some(action) = run {
            self.palette_open = false;
            self.apply_palette_action(action);
        }
        ctx.request_repaint();
    }

    fn apply_palette_action(&mut self, action: crate::gui::types::PaletteAction) {
        use crate::gui::types::PaletteAction;
        match action {
            PaletteAction::Goto(tab) => self.tab = tab,
            PaletteAction::GotoProxyPanel(panel) => {
                self.tab = Tab::Proxy;
                self.proxy_panel = panel;
            }
            PaletteAction::ToggleProxy => {
                if matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                ) {
                    self.stop_proxy();
                } else {
                    self.start_proxy();
                }
            }
            PaletteAction::ToggleMode => {
                if !self.proxy_status.config_editable() {
                    self.status = "请先停止代理后再切换工作模式".into();
                    return;
                }
                self.proxy_config.mode = match self.proxy_config.mode {
                    ProxyMode::System => ProxyMode::Tun,
                    ProxyMode::Tun => ProxyMode::System,
                };
                self.save_proxy_config();
            }
            PaletteAction::Theme(id) => {
                // 只改设置：`update()` 里那句 `applied_theme_id != settings.ui_theme`
                // 会在下一帧把主题真正刷下去，这里不自己调 theme::apply。
                self.edit_ui_theme = id.to_string();
                self.settings.ui_theme = id.to_string();
                self.save_settings();
            }
            PaletteAction::DownloadKernel => {
                let kind = self
                    .kernels
                    .iter()
                    .find(|kernel| !kernel.available)
                    .or_else(|| self.kernels.first())
                    .map(|kernel| kernel.kind);
                match kind {
                    Some(kind) => self.open_kernel_download(kind),
                    None => {
                        self.tab = Tab::Proxy;
                        self.proxy_panel = ProxyPanel::Kernels;
                        self.status = "没有可下载的内核记录".into();
                    }
                }
            }
            PaletteAction::UseNode(id, name) => {
                self.tab = Tab::Proxy;
                self.proxy_panel = ProxyPanel::Nodes;
                self.switch_proxy_node(id, name, "命令面板");
            }
        }
    }

    /// 打开命令面板。焦点请求要留到面板自己那一帧再做，所以单独一个标志。
    pub(crate) fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette_needs_focus = true;
        self.palette_selected = 0;
    }

    /// 把库里最近 24 小时的出口健康捞回界面。启动时一次，之后每次实测落库后再刷。
    fn reload_egress_health(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let streak = db.egress_fail_streak().await.unwrap_or(0);
            let rows = db
                .egress_health_since(chrono::Utc::now() - chrono::Duration::hours(24))
                .await
                .unwrap_or_default();
            let _ = tx.send(BgMsg::EgressHealth(rows, streak));
        });
    }

    /// 把攒下的命中增量写进主库（按天聚合，冲突就累加）。
    fn flush_rule_hits(&mut self) {
        self.last_hit_flush = std::time::Instant::now();
        let at = chrono::Utc::now().to_rfc3339();
        let records: Vec<crate::db::telemetry::RuleHit> = self
            .rule_tally
            .drain()
            .map(|((rule, target), count)| crate::db::telemetry::RuleHit {
                rule,
                target,
                count,
                at: at.clone(),
            })
            .collect();
        if records.is_empty() {
            return;
        }
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = db.record_rule_hits(&records).await;
            if let Ok(rows) = db.rule_hits_today().await {
                let _ = tx.send(BgMsg::RuleHits(rows));
            }
        });
    }

    fn reload_rule_hits(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Ok(rows) = db.rule_hits_today().await {
                let _ = tx.send(BgMsg::RuleHits(rows));
            }
        });
    }

    /// 出口切换历史：只在点开那张窗口时才查，不每帧查。
    /// 取 300 条是**显示上限**而不是保留期：`ScrollArea::show` 不做虚拟化，行数是逐行排版
    /// 的，窗口开着时每帧都要走完这些行。库里到底还有多少条由表头那句"共 N 条"负责说实话，
    /// 被截掉的部分写"更早的 N 条未在此显示"。
    pub(crate) fn reload_egress_switches(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let rows = db.recent_egress_switches(300).await.unwrap_or_default();
            let today = db.switch_count_today().await.unwrap_or(0);
            let total = db.switch_history_count().await.unwrap_or(0);
            let _ = tx.send(BgMsg::EgressSwitches(rows, today, total));
        });
    }

    /// 出口切换记录窗口：按本地日分组列出保留期内的全部历史 —— 时间、从谁到谁、为什么。
    pub(crate) fn ui_switch_log(&mut self, ctx: &egui::Context) {
        if !self.show_switch_log {
            return;
        }
        let mut open = true;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        egui::Window::new("出口切换记录")
            .id(egui::Id::new("vpn-parser-switch-log"))
            .open(&mut open)
            .default_width(660.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "共 {} 条 · 今天 {} 次 · 保留 {} 天（重启不丢）",
                        self.switch_total,
                        self.switch_today,
                        crate::db::telemetry::RETENTION_SWITCHES_DAYS
                    ))
                    .strong()
                    .color(theme::text_primary()),
                );
                if self.switch_rows.is_empty() {
                    ui.label("还没有切换记录。手动换出口、自动切换、切换失败都会记在这里。");
                    return;
                }
                let hidden = (self.switch_total - self.switch_rows.len() as i64).max(0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(340.0)
                    .show(ui, |ui| {
                        if hidden > 0 {
                            ui.label(
                                RichText::new(format!("更早的 {hidden} 条未在此显示。"))
                                    .small()
                                    .color(theme::muted()),
                            );
                        }
                        // 行是按 id 倒序取来的，同一个本地日必然连成一段，所以
                        // 只需要"和上一条日期不同就插一条日头"，不用分桶。
                        let mut current_day = String::new();
                        for row in &self.switch_rows {
                            let day = crate::db::telemetry::switch_day(&row.at);
                            if day != current_day {
                                current_day = day.clone();
                                let label = if day == today {
                                    format!("{day} · 今天")
                                } else {
                                    day.clone()
                                };
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(label)
                                        .strong()
                                        .small()
                                        .color(theme::text_secondary()),
                                );
                            }
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(crate::gui::widgets::clock_of(&row.at))
                                        .monospace()
                                        .small()
                                        .color(theme::muted()),
                                );
                                let from = row.from_name.clone().unwrap_or_else(|| "—".into());
                                // 空 to_node 是"判定要切但一个替补都没找到"，
                                // 写成"—"会被当成显示不全。
                                let to = match row.to_name.as_deref() {
                                    Some(n) if !n.is_empty() => n.to_string(),
                                    _ => match row.to_node.as_deref() {
                                        Some(id) if !id.is_empty() => id.to_string(),
                                        _ => "没切成".into(),
                                    },
                                };
                                ui.label(format!("{from} → {to}"));
                                ui.label(
                                    RichText::new(&row.reason)
                                        .small()
                                        .color(theme::text_secondary()),
                                );
                                if let Some(ms) = row.latency_ms {
                                    ui.label(
                                        RichText::new(format!("{ms} ms"))
                                            .small()
                                            .color(theme::muted()),
                                    );
                                }
                            });
                        }
                    });
            });
        self.show_switch_log = open;
    }

    /// 自动切换：每 60 秒经**当前运行中的代理**实测一次连通性与延迟（不新起内核），
    /// 验证失败立即切换；仅延迟劣化则需连续三次、且距上次自动切换满 3 分钟才跳，
    /// 避免一次网络抖动就断掉全部连接。
    pub(crate) fn check_failover(&mut self) {
        // 探测本身不受 `auto_failover` 影响：表头要说实话，就必须一直有实测在跑。
        // 那个开关只决定"测出问题之后要不要自动换出口"（见 handle_failover_probe）。
        if self.proxy_busy {
            return;
        }
        if !matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        ) {
            return;
        }
        let Some(node_id) = self.selected_proxy_node_id.clone() else {
            return;
        };
        if self
            .failover_probe_at
            .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(60))
        {
            return;
        }
        // 先占位，避免结果回来前重复探测。
        self.failover_probe_at = Some(std::time::Instant::now());
        self.sync_router_config();
        self.seed_router_candidates();
        self.router.set_active(&node_id);
        let node_name = self
            .selected_proxy_node_name
            .clone()
            .unwrap_or_else(|| node_id.clone());
        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        let timeout = std::time::Duration::from_millis(
            (self.settings.failover_latency_ms as u64 * 2).clamp(3_000, 8_000),
        );
        self.rt.spawn(async move {
            let (ok, latency_ms) = manager.probe_health(timeout).await;
            let _ = tx.send(BgMsg::FailoverProbe {
                node_id,
                node_name,
                ok,
                latency_ms,
            });
        });
    }

    fn handle_failover_probe(
        &mut self,
        node_id: String,
        node_name: String,
        ok: bool,
        latency_ms: u64,
    ) {
        // 探测期间用户可能已经手动换节点，旧结果直接丢弃。
        if self.selected_proxy_node_id.as_deref() != Some(node_id.as_str()) {
            return;
        }
        // 每一次实测都落库并刷回界面 —— 表头那句"已连通"从此只信这条，不信 ProxyState。
        let db = self.db.clone();
        let tx = self.tx.clone();
        let probe_node_id = node_id.clone();
        let probe_node_name = node_name.clone();
        self.rt.spawn(async move {
            let _ = db
                .record_egress_health(
                    ok,
                    Some(latency_ms),
                    if ok {
                        None
                    } else {
                        Some("实测未在时限内取到 204")
                    },
                    Some(&probe_node_id),
                    Some(&probe_node_name),
                    "periodic",
                )
                .await;
            let streak = db.egress_fail_streak().await.unwrap_or(0);
            let rows = db
                .egress_health_since(chrono::Utc::now() - chrono::Duration::hours(24))
                .await
                .unwrap_or_default();
            let _ = tx.send(BgMsg::EgressHealth(rows, streak));
        });
        if !self.auto_failover {
            // 只测不换：开关关掉时，健康数据照记，出口绝不自动被换走。
            return;
        }
        self.sync_router_config();
        // url-test 开着时延迟以内核 history 为准，这次探测只贡献"通/不通"这一个信号
        // （内核只知道它自己那条测试路，不知道穿过代理的端到端请求成不成）。
        let latency_input = if self.proxy_config.url_test {
            None
        } else {
            Some(latency_ms)
        };
        let decision = self.router.observe(
            &node_id,
            crate::router::Sample {
                ok,
                latency_ms: latency_input,
                download_kbps: None,
            },
            std::time::Instant::now(),
        );
        self.apply_router_decision(&node_id, &node_name, decision);
    }

    /// 把调度器的判定翻译成界面文案 + 实际切换动作。
    fn apply_router_decision(
        &mut self,
        node_id: &str,
        node_name: &str,
        decision: crate::router::Decision,
    ) {
        use crate::router::Decision;
        match decision {
            Decision::Stay => {}
            Decision::BreakerOpened { reason, .. } => {
                self.failover_last_reason = Some(format!("{node_name} {reason}，正在寻找替补节点"));
                self.status = self
                    .failover_last_reason
                    .clone()
                    .unwrap_or_else(|| "代理节点被拉黑".into());
                if self.proxy_config.url_test {
                    self.hand_back_to_kernel(node_name, &reason);
                } else {
                    self.begin_failover_switch(node_id, None, "当前节点已被断路器拉黑");
                }
            }
            Decision::SwitchTo {
                node_id: target,
                reason,
            } => {
                if self.proxy_config.url_test {
                    self.hand_back_to_kernel(node_name, &reason);
                    return;
                }
                // 调度器给的目标也要先过一遍 TCP 复检，复检过了才真切。
                self.begin_failover_switch(node_id, Some(&target), &reason);
            }
        }
    }

    /// A：url-test 开着时 app 层只负责"判定"，换节点交给内核 —— 否则每判一次就要
    /// 重写配置 + 热重载，把已经建立的连接全掐掉。坏节点由断路器拉黑，并在下一次
    /// 候选刷新时从 url-test 里摘掉；这里只做一件不断线的事：如果 PROXY 被锁在某个
    /// 具体节点上（内核就没法自己换），把它指回「自动选择」。
    fn hand_back_to_kernel(&mut self, node_name: &str, reason: &str) {
        self.failover_switches += 1;
        let locked_elsewhere = self
            .kernel_groups
            .iter()
            .find(|group| group.name == "PROXY")
            .and_then(|group| group.now.clone())
            .is_some_and(|now| now != URL_TEST_GROUP);
        self.failover_last_reason = Some(format!(
            "{node_name} {reason} · 已拉黑，下次候选刷新不再入选；{}",
            if locked_elsewhere {
                "PROXY 被锁在具体节点上，已交还「自动选择」"
            } else {
                "换节点交给内核"
            }
        ));
        self.status = self.failover_last_reason.clone().unwrap_or_default();
        if locked_elsewhere {
            self.select_kernel_member("PROXY".to_owned(), URL_TEST_GROUP.to_owned());
        }
    }

    /// 自动切换：先挑替补、再对替补做一次 TCP 复检，复检过了才真切换。
    ///
    /// 为什么非要复检：库里的 `latency_ok` 是"上一次测速时通"，可能已经是几小时前。
    /// 2026-10-07 那次自动切换就是照着 08:24 的 90ms 记录切进了一个端口早就不监听的
    /// socks5，结果整个境外不通。控制台那批候选一个都不剩时，还会现查一次库里最新测通的。
    fn begin_failover_switch(
        &mut self,
        current_id: &str,
        preferred: Option<&str>,
        reason: &str,
    ) {
        if self.failover_searching {
            self.failover_last_reason =
                Some("替补复检还在进行中，这一轮的判定先不重复触发".into());
            return;
        }
        let blacklisted: std::collections::HashSet<String> = self
            .router
            .blacklisted(std::time::Instant::now())
            .into_iter()
            .collect();
        let all: Vec<crate::router::BackupCandidate> = self
            .proxy_nodes
            .iter()
            .map(|node| crate::router::BackupCandidate {
                id: node.id.clone(),
                name: node.name.clone(),
                host: node.server.clone(),
                port: node.port,
                last_latency_ms: node.latency_ms.map(|ms| ms.max(0) as u64),
            })
            .collect();
        let mut shortlist = crate::router::pick_backups(
            &all,
            current_id,
            &blacklisted,
            &self.settings.exclude_keywords,
            8,
        );
        if let Some(want) = preferred {
            if let Some(position) = shortlist.iter().position(|node| node.id == want) {
                let chosen = shortlist.remove(position);
                shortlist.insert(0, chosen);
            }
        }
        self.failover_searching = true;
        self.failover_from = Some((
            current_id.to_string(),
            self.selected_proxy_node_name.clone().unwrap_or_default(),
        ));
        self.status = if shortlist.is_empty() {
            format!("{reason}，控制台候选里没有第二个可用节点，改从库里最新测通的里现查...")
        } else {
            format!("{reason}，正在对 {} 个替补节点做 TCP 复检...", shortlist.len())
        };
        let db = self.db.clone();
        let tx = self.tx.clone();
        let current = current_id.to_string();
        self.rt.spawn(async move {
            let mut pool = shortlist;
            if pool.is_empty() {
                pool = db
                    .top_verified_nodes(12, Some(&current))
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|node| crate::router::BackupCandidate {
                        id: node.id,
                        name: node.name,
                        host: node.server,
                        port: node.port,
                        last_latency_ms: node.latency_ms.map(|ms| ms.max(0) as u64),
                    })
                    .collect();
            }
            let probed = pool.len();
            let outcome = if pool.is_empty() {
                Err("库里也找不出第二个测通过的节点".to_string())
            } else {
                crate::proxy_probe::first_reachable(pool, std::time::Duration::from_millis(2_500))
                    .await
                    .ok_or_else(|| format!("TCP 复检过 {probed} 个候选，没有一个端口连得上"))
            };
            let _ = tx.send(BgMsg::FailoverTarget(outcome));
        });
    }

    fn on_failover_target(&mut self, outcome: Result<(String, String), String>) {
        self.failover_searching = false;
        let from = self.failover_from.take();
        match outcome {
            Ok((id, name)) => {
                self.failover_switches += 1;
                self.failover_last_reason = Some(format!(
                    "复检通过 → 已切到 {name} · 本次运行第 {} 次",
                    self.failover_switches
                ));
                self.status = self.failover_last_reason.clone().unwrap_or_default();
                self.switch_proxy_node(id, name, "自动切换");
            }
            Err(reason) => {
                self.failover_last_reason = Some(format!("判定该换出口，但{reason}"));
                self.status = self.failover_last_reason.clone().unwrap_or_default();
                if let Some((from_id, from_name)) = from {
                    self.record_failover_miss(&from_id, &from_name, &reason);
                }
            }
        }
    }

    /// "该切但没切成"也进切换历史。以前这种只写在一行状态栏里，滑过去就查无实据，
    /// 于是"我开了自动切换为什么没换"只能靠人翻库猜。
    fn record_failover_miss(&mut self, from_id: &str, from_name: &str, reason: &str) {
        let db = self.db.clone();
        let record = crate::db::telemetry::SwitchRecord {
            from_node: Some(from_id.to_string()),
            from_name: Some(from_name.to_string()),
            // 空串而不是 NULL：DAO 用 (to_node, reason) 去重连续相同记录，
            // NULL 参与拼接会永远不相等，抖动时每两分钟刷一条一模一样的。
            to_node: Some(String::new()),
            to_name: None,
            reason: format!("没有可用替补：{reason}"),
            latency_ms: None,
        };
        self.rt.spawn(async move {
            let _ = db.record_egress_switch(&record).await;
        });
    }

    /// 调度器参数跟随系统设置，避免两处真相。
    fn sync_router_config(&mut self) {
        let mut cfg = self.router.cfg.clone();
        cfg.latency_threshold_ms = self.settings.failover_latency_ms.max(200) as u64;
        cfg.bandwidth_threshold_kbps = if self.settings.bandwidth_guard {
            self.settings.bandwidth_min_kbps.max(1) as f64
        } else {
            0.0
        };
        self.router.cfg = cfg;
    }

    /// 用数据库里最近一次测速结果给调度器播种候选，否则刚启动时它谁都不认识。
    fn seed_router_candidates(&mut self) {
        let keywords = self.settings.exclude_keywords.clone();
        let now = std::time::Instant::now();
        let mut rows: Vec<(String, i64)> = self
            .proxy_nodes
            .iter()
            .filter(|node| node.latency_ok == Some(true))
            .filter(|node| !crate::models::name_excluded(&node.name, &keywords))
            .filter_map(|node| node.latency_ms.map(|ms| (node.id.clone(), ms)))
            .collect();
        rows.sort_by_key(|(_, ms)| *ms);
        for (id, ms) in rows.into_iter().take(60) {
            self.router.seed(&id, ms.max(0) as u64, true, now);
        }
    }

    pub(crate) fn open_export(&mut self, kind: &str) {
        // 导出链接是本地 HTTP API 的 URL，服务没起来时点开只会得到一个连不上的页面。
        if !self.api_controller.is_running() {
            self.status = "本地 HTTP 服务未启动，请先在「外部接口」页启动后再导出".into();
            return;
        }
        let mut url = format!(
            "/api/export/{}?quality_only={}&max_latency_ms={}&min_download_kbps={}&limit={}",
            kind,
            self.export_quality_only,
            self.export_max_latency_ms,
            self.export_min_download_kbps,
            self.export_limit.max(1)
        );
        // v2rayN subscription expects base64 body.
        if kind == "uri" {
            url.push_str("&base64=true");
        }
        let url = self.api_url(&url, true);
        self.status = format!("导出: {url}");
        self.pending_open_url = Some(url);
    }

    /// 查询当前选中代理节点的落地风险。
    pub(crate) fn check_ip_risk(&mut self) {
        let Some(id) = self.selected_proxy_node_id.clone() else {
            self.show_ip_risk_dialog = true;
            self.ip_risk_error = Some("请先选择代理节点".into());
            self.status = "请先选择代理节点".into();
            return;
        };
        self.check_ip_risk_for(&id);
    }

    /// 查询指定节点的 IP 检测：入口 = 节点服务器（域名先解析）。出口只在该节点**正是
    /// 当前运行的节点**时才自动查 —— 本机只有一个 mixed 端口，拿它去查别的节点得到的是
    /// 当前节点的出口，属于假数据。
    pub(crate) fn check_ip_risk_for(&mut self, node_id: &str) {
        self.show_ip_risk_dialog = true;
        self.ip_risk_target = Some(node_id.to_owned());
        self.ip_risk_egress = None;
        self.ip_risk_egress_for = None;
        self.ip_risk_egress_error = None;
        self.ip_risk_egress_pending = false;
        let Some(server) = self
            .proxy_nodes
            .iter()
            .chain(self.nodes.iter())
            .find(|node| node.id == node_id)
            .map(|node| node.server.clone())
        else {
            self.ip_risk_error = Some("找不到该节点的服务器地址".into());
            self.status = "找不到该节点的服务器地址".into();
            return;
        };
        if self.ip_risk_busy {
            return;
        }
        if self
            .ip_risk
            .as_ref()
            .is_some_and(|risk| risk.hostname == server || risk.ip == server)
        {
            self.status = format!("IP 检测：{}", self.ip_risk.as_ref().unwrap().headline());
            self.start_egress_lookup_if_current(node_id);
            return;
        }
        self.ip_risk_busy = true;
        self.ip_risk_error = None;
        self.status = format!("正在查询 {server} 的归属...");
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let front = crate::ip_risk::lookup(&client, &server)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::IpRisk(front));
        });
        self.start_egress_lookup_if_current(node_id);
    }

    /// 该节点正是当前在跑的节点时，才经本机 mixed 端口自动查一次出口。
    fn start_egress_lookup_if_current(&mut self, node_id: &str) {
        let running_on_it = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        ) && self.proxy_status.local_proxy_active()
            && self.proxy_status.node_id.as_deref() == Some(node_id);
        if !running_on_it {
            return;
        }
        let url = format!("http://127.0.0.1:{}", self.proxy_status.mixed_port);
        self.ip_risk_egress_pending = true;
        self.status = "正在经当前代理查出口 IP...".into();
        let tx = self.tx.clone();
        let id = node_id.to_owned();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let egress = crate::ip_risk::lookup_egress(&client, &url)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::IpEgress(id, egress));
        });
    }

    pub(crate) fn delete_node(&mut self, id: String) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.status = "正在删除节点...".into();
        self.rt.spawn(async move {
            let res = db
                .delete_node(&id)
                .await
                .map(|ok| {
                    if ok {
                        "节点已删除".into()
                    } else {
                        "节点不存在".into()
                    }
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: false,
            });
        });
    }

    pub(crate) fn clear_nodes(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.rt.spawn(async move {
            let res = db
                .clear_nodes()
                .await
                .map(|n| format!("已清空 {n} 个节点"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn prune_stale_nodes(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在淘汰 7 天以上持续失效的节点...".into();
        self.rt.spawn(async move {
            let res = db
                .prune_stale_dead_nodes(7)
                .await
                .map(|n| format!("已清理淘汰 {n} 个长期失效节点"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone { result: res, unlock: true });
        });
    }

    pub(crate) fn delete_dead_nodes(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在删除超时/失败节点...".into();
        self.rt.spawn(async move {
            let res = db
                .delete_dead_nodes()
                .await
                .map(|n| format!("已删除 {n} 个超时/失败节点"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn dedupe_nodes(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在去除重复节点...".into();
        self.rt.spawn(async move {
            let res = db
                .dedupe_nodes()
                .await
                .map(|n| format!("已去除 {n} 个重复节点（同协议/地址/端口/认证信息）"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn dedupe_physical_nodes(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在分析并执行物理去重...".into();
        self.rt.spawn(async move {
            let res = db
                .deduplicate_physical()
                .await
                .map(|n| format!("物理去重完成：已清理 {n} 个相同主机与端口的重复节点"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn clean_node_names(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在智能净化与标准化节点名称...".into();
        self.rt.spawn(async move {
            let res = db
                .clean_and_beautify_node_names()
                .await
                .map(|n| format!("节点名净化完成：已标准化美化 {n} 个节点名称"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn delete_invalid_hosts(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在清理无效主机节点...".into();
        self.rt.spawn(async move {
            let res = db
                .delete_invalid_hosts()
                .await
                .map(|n| format!("已删除 {n} 个无效主机节点（127.0.0.1/内网等）"))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn show_node(&mut self, id: String) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = match db.get_node(&id).await {
                Ok(Some(n)) => Ok(n),
                Ok(None) => Err("节点不存在".into()),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(BgMsg::NodeDetail(res));
        });
    }

    pub(crate) fn settings_edit_state(&self) -> SettingsEditState {
        SettingsEditState {
            ui_theme: self.edit_ui_theme.clone(),
            kernel_source: self.edit_kernel_source.clone(),
            firewall_auto_allow: self.edit_firewall_auto_allow,
            auto_fetch: self.edit_auto_fetch,
            interval_min: self.edit_interval_min,
            auto_speed: self.edit_auto_speed,
            speedtest_auto_refill: self.edit_speedtest_auto_refill,
            auto_rearm_system_proxy: self.edit_auto_rearm_system_proxy,
            auto_prepare_kernel: self.edit_auto_prepare_kernel,
            speedtest_added_source: self.edit_speedtest_added_source,
            fetch_concurrency: self.edit_fetch_concurrency,
            fetch_timeout_secs: self.edit_fetch_timeout_secs,
            speedtest_concurrency: self.edit_speedtest_concurrency,
            timeout_ms: self.edit_timeout_ms,
            speedtest_download: self.edit_speedtest_download,
            download_test_ms: self.edit_download_test_ms,
            http_server_enabled: self.edit_http_server_enabled,
            close_to_tray: self.edit_close_to_tray,
            exclude_keywords: self.edit_exclude_keywords.clone(),
            failover_latency_ms: self.edit_failover_latency_ms,
            speedtest_parallel_kernels: self.edit_speedtest_parallel_kernels,
            speedtest_persistent_kernel: self.edit_speedtest_persistent_kernel,
            speedtest_reuse_minutes: self.edit_speedtest_reuse_minutes,
            speedtest_conditional_isolation: self.edit_speedtest_conditional_isolation,
            speedtest_download_top_n: self.edit_speedtest_download_top_n,
            speedtest_dedup_egress: self.edit_speedtest_dedup_egress,
            bandwidth_guard: self.edit_bandwidth_guard,
            bandwidth_min_kbps: self.edit_bandwidth_min_kbps,
        }
    }

    pub(crate) fn settings_dirty(&self) -> bool {
        self.settings_edit_state().is_dirty(&self.settings)
    }

    pub(crate) fn sync_edit_settings(&mut self) {
        let state = SettingsEditState::from_settings(&self.settings);
        self.edit_auto_fetch = state.auto_fetch;
        self.edit_interval_min = state.interval_min;
        self.edit_auto_speed = state.auto_speed;
        self.edit_speedtest_auto_refill = state.speedtest_auto_refill;
        self.edit_auto_rearm_system_proxy = state.auto_rearm_system_proxy;
        self.edit_auto_prepare_kernel = state.auto_prepare_kernel;
        self.edit_speedtest_added_source = state.speedtest_added_source;
        self.edit_fetch_concurrency = state.fetch_concurrency;
        self.edit_fetch_timeout_secs = state.fetch_timeout_secs;
        self.edit_speedtest_concurrency = state.speedtest_concurrency;
        self.edit_timeout_ms = state.timeout_ms;
        self.edit_speedtest_download = state.speedtest_download;
        self.edit_download_test_ms = state.download_test_ms;
        self.edit_http_server_enabled = state.http_server_enabled;
        self.edit_close_to_tray = state.close_to_tray;
        self.edit_exclude_keywords = state.exclude_keywords;
        self.edit_failover_latency_ms = state.failover_latency_ms;
        self.edit_speedtest_parallel_kernels = state.speedtest_parallel_kernels;
        self.edit_speedtest_persistent_kernel = state.speedtest_persistent_kernel;
        self.edit_speedtest_reuse_minutes = state.speedtest_reuse_minutes;
        self.edit_speedtest_conditional_isolation = state.speedtest_conditional_isolation;
        self.edit_speedtest_download_top_n = state.speedtest_download_top_n;
        self.edit_speedtest_dedup_egress = state.speedtest_dedup_egress;
        self.edit_bandwidth_guard = state.bandwidth_guard;
        self.edit_bandwidth_min_kbps = state.bandwidth_min_kbps;
        self.edit_ui_theme = state.ui_theme.clone();
        self.edit_kernel_source = state.kernel_source.clone();
        self.edit_firewall_auto_allow = state.firewall_auto_allow;
    }

    pub(crate) fn build_update_settings(&self) -> UpdateSettings {
        UpdateSettings {
            auto_fetch_enabled: Some(self.edit_auto_fetch),
            auto_fetch_interval_min: Some(self.edit_interval_min),
            auto_speedtest_enabled: Some(self.edit_auto_speed),
            speedtest_auto_refill: Some(self.edit_speedtest_auto_refill),
            auto_rearm_system_proxy: Some(self.edit_auto_rearm_system_proxy),
            auto_prepare_kernel: Some(self.edit_auto_prepare_kernel),
            speedtest_added_source: Some(self.edit_speedtest_added_source),
            fetch_concurrency: Some(self.edit_fetch_concurrency.clamp(1, 8)),
            fetch_timeout_secs: Some(self.edit_fetch_timeout_secs),
            speedtest_concurrency: Some(self.edit_speedtest_concurrency),
            speedtest_timeout_ms: Some(self.edit_timeout_ms),
            speedtest_download: Some(self.edit_speedtest_download),
            download_test_ms: Some(self.edit_download_test_ms),
            http_server_enabled: Some(self.edit_http_server_enabled),
            close_to_tray: Some(self.edit_close_to_tray),
            exclude_keywords: Some(self.edit_exclude_keywords.clone()),
            failover_latency_ms: Some(self.edit_failover_latency_ms.clamp(200, 15_000)),
            speedtest_parallel_kernels: Some(self.edit_speedtest_parallel_kernels),
            speedtest_persistent_kernel: Some(self.edit_speedtest_persistent_kernel),
            speedtest_reuse_minutes: Some(self.edit_speedtest_reuse_minutes),
            speedtest_conditional_isolation: Some(self.edit_speedtest_conditional_isolation),
            speedtest_download_top_n: Some(self.edit_speedtest_download_top_n),
            speedtest_dedup_egress: Some(self.edit_speedtest_dedup_egress),
            bandwidth_guard: Some(self.edit_bandwidth_guard),
            bandwidth_min_kbps: Some(self.edit_bandwidth_min_kbps),
            ui_theme: Some(self.edit_ui_theme.clone()),
            kernel_source: Some(self.edit_kernel_source.clone()),
            firewall_auto_allow: Some(self.edit_firewall_auto_allow),
            auto_failover: Some(self.auto_failover),
            // 发布服务器两项有自己的落库路径（save_publisher），这里刻意不动，
            // 避免周期性设置保存把用户刚写的值覆盖回旧内存副本。
            publisher_api: None,
            publisher_token: None,
        }
    }

    pub(crate) fn save_settings(&mut self) {
        if self.settings_saving {
            return;
        }
        self.settings_saving = true;
        let db = self.db.clone();
        let tx = self.tx.clone();
        let input = self.build_update_settings();
        self.rt.spawn(async move {
            let res = db.update_settings(input).await.map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SettingsSaved(res));
        });
    }

    /// 发布服务器地址 / 推送 token 落库。以前这两项只写 eframe storage，而
    /// `persistence` feature 是关掉的，所以填了也存不下来。
    pub(crate) fn save_publisher(&mut self, api: String, token: String) {
        self.publisher_seeded = true;
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.remote_api_status = "正在保存...".into();
        self.rt.spawn(async move {
            let res = db
                .update_publisher(&api, &token)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::PublisherSaved(res));
        });
    }

    pub(crate) fn start_api_server(&mut self) {
        let controller = self.api_controller.clone();
        let tx = self.tx.clone();
        self.status = "正在启动 HTTP API 服务器...".into();
        self.edit_http_server_enabled = true;
        self.rt.spawn(async move {
            match controller.start().await {
                Ok(_) => {
                    let _ = tx.send(BgMsg::ApiServerStatus(Ok("HTTP API 服务器已启动".into())));
                }
                Err(e) => {
                    let _ = tx.send(BgMsg::ApiServerStatus(Err(format!(
                        "启动 HTTP 服务器失败: {e}"
                    ))));
                }
            }
        });
    }

    pub(crate) fn stop_api_server(&mut self) {
        let controller = self.api_controller.clone();
        let tx = self.tx.clone();
        self.status = "正在停止 HTTP API 服务器...".into();
        self.edit_http_server_enabled = false;
        self.rt.spawn(async move {
            match controller.stop().await {
                Ok(_) => {
                    crate::platform::trim_working_set();
                    let _ = tx.send(BgMsg::ApiServerStatus(Ok(
                        "HTTP API 服务器已关闭，端口已释放".into(),
                    )));
                }
                Err(e) => {
                    let _ = tx.send(BgMsg::ApiServerStatus(Err(format!(
                        "关闭 HTTP 服务器失败: {e}"
                    ))));
                }
            }
        });
    }

    pub(crate) fn trim_memory(&mut self) {
        let db = self.db.clone();
        self.rt.spawn(async move {
            db.trim_cached_pools().await;
        });
        crate::platform::trim_working_set();
        self.memory_trimmed_status = Some("已深度整理进程内存，工作集已释放".into());
    }
    /// 自绘标题栏。`with_decorations(false)` 之后系统那条没了，所以拖动、双击最大化、
    /// 最小化/最大化/关闭三个按钮都得在这儿自己实现。
    ///
    /// 三个按钮用 painter 画、`interact` 命中，不用 `Button`，也不放 `Label`：
    /// `Frame` 的外接矩形是按内容 `min_rect` 反推的，而面板只画到 `exact_height` 就裁掉。
    /// `Button` 想要"文字 + button_padding"那么高，`ui.put` 会把格子顶到 34 之外 ——
    /// 顶出去的部分不画但照样占位，表现成标题栏下面一条没人画的黑色带子。
    fn ui_title_bar(&mut self, ctx: &egui::Context) {
        const BAR_H: f32 = TITLE_BAR_H;
        const BTN_W: f32 = TITLE_BAR_BTN_W;

        #[derive(Clone, Copy)]
        enum Glyph {
            Minimize,
            Maximize,
            Restore,
            Close,
        }

        fn title_button(
            ui: &Ui,
            rect: egui::Rect,
            key: &'static str,
            glyph: Glyph,
            tip: &str,
        ) -> bool {
            let hovered = ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|point| rect.contains(point));
            let painter = ui.painter().clone();
            let ink = theme::text_secondary();
            let pill = rect.shrink(4.0);
            if hovered {
                painter.rect(
                    pill,
                    CornerRadius::same(7),
                    theme::panel_hover(),
                    Stroke::NONE,
                    egui::StrokeKind::Inside,
                );
            }
            let stroke = Stroke::new(1.2_f32, ink);
            let c = pill.center();
            match glyph {
                Glyph::Minimize => {
                    painter.line_segment(
                        [c + Vec2::new(-5.5, 0.0), c + Vec2::new(5.5, 0.0)],
                        stroke,
                    );
                }
                Glyph::Maximize => {
                    painter.rect(
                        egui::Rect::from_center_size(c, Vec2::splat(10.5)),
                        CornerRadius::same(2),
                        Color32::TRANSPARENT,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                Glyph::Restore => {
                    painter.rect(
                        egui::Rect::from_min_max(
                            c + Vec2::new(-6.0, -2.5),
                            c + Vec2::new(3.5, 6.5),
                        ),
                        CornerRadius::same(2),
                        Color32::TRANSPARENT,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                    painter.line_segment(
                        [c + Vec2::new(-3.0, -6.5), c + Vec2::new(6.5, -6.5)],
                        stroke,
                    );
                    painter.line_segment(
                        [c + Vec2::new(6.5, -6.5), c + Vec2::new(6.5, -2.0)],
                        stroke,
                    );
                }
                Glyph::Close => {
                    painter.line_segment(
                        [c + Vec2::new(-5.0, -5.0), c + Vec2::new(5.0, 5.0)],
                        stroke,
                    );
                    painter.line_segment(
                        [c + Vec2::new(-5.0, 5.0), c + Vec2::new(5.0, -5.0)],
                        stroke,
                    );
                }
            }
            ui.interact(rect, ui.id().with(("titlebar-button", key)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tip)
                .clicked()
        }

        let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
        let mut action_minimize = false;
        let mut action_toggle_max = false;
        let mut action_close = false;

        egui::TopBottomPanel::top("custom_title_bar")
            .exact_height(BAR_H)
            .frame(Frame::NONE.fill(theme::sidebar()))
            .show(ctx, |ui| {
                // 一次性把整格占住，之后全用 painter/interact：这样 Frame 反推出来的
                // 高度恒等于 BAR_H，画到的和分给下面面板的永远是同一条边。
                let bar = ui.max_rect();
                ui.allocate_rect(bar, Sense::hover());
                let buttons_left = bar.max.x - 3.0 * BTN_W;

                ui.painter().text(
                    egui::pos2(bar.min.x + 14.0, bar.center().y),
                    egui::Align2::LEFT_CENTER,
                    "VPN Parser",
                    egui::FontId::proportional(12.5),
                    theme::text_secondary(),
                );

                // 除了右边三个按钮，整条都能拖动窗口；双击切换最大化。
                let drag_rect =
                    egui::Rect::from_min_max(bar.min, egui::pos2(buttons_left, bar.max.y));
                let drag = ui.interact(drag_rect, ui.id().with("titlebar_drag"), Sense::drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    action_toggle_max = true;
                }

                let cell = |index: usize| {
                    egui::Rect::from_min_max(
                        egui::pos2(bar.max.x - (index as f32 + 1.0) * BTN_W, bar.min.y),
                        egui::pos2(bar.max.x - index as f32 * BTN_W, bar.max.y),
                    )
                };
                if title_button(
                    ui,
                    cell(0),
                    "close",
                    Glyph::Close,
                    "关闭（按设置在托盘里继续常驻）",
                ) {
                    action_close = true;
                }
                if title_button(
                    ui,
                    cell(1),
                    "maximize",
                    if maximized {
                        Glyph::Restore
                    } else {
                        Glyph::Maximize
                    },
                    if maximized { "向下还原" } else { "最大化" },
                ) {
                    action_toggle_max = true;
                }
                if title_button(ui, cell(2), "minimize", Glyph::Minimize, "最小化") {
                    action_minimize = true;
                }
            });

        if action_minimize {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        if action_toggle_max {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        if action_close {
            self.request_window_close(ctx);
        }
    }

    /// 标题栏的 ✕ 必须和系统关闭按钮走同一套分支，否则"关闭收进托盘"这个设置形同虚设。
    /// 每 30 秒核对一次系统代理归属。只在"代理在跑、而且是我们开的那份系统代理"
    /// （`local_proxy_ready`）时查 —— TUN 模式根本不碰注册表，查了只会误报。
    /// 查询要起 `reg.exe`（三次），所以丢给 `spawn_blocking`，绝不在 UI 线程上等。
    pub(crate) fn maybe_check_proxy_drift(&mut self) {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(30);
        if !self.proxy_status.local_proxy_ready {
            if self.proxy_drift.is_some() {
                self.proxy_drift = None;
            }
            self.drift_checked_at = None;
            return;
        }
        if self.drift_busy {
            return;
        }
        if let Some(at) = self.drift_checked_at {
            if at.elapsed() < EVERY {
                return;
            }
        }
        self.drift_checked_at = Some(std::time::Instant::now());
        self.drift_busy = true;
        let expected = format!("127.0.0.1:{}", self.proxy_status.mixed_port);
        let control = crate::system_proxy::platform_system_proxy();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let drift = tokio::task::spawn_blocking(move || {
                control
                    .capture()
                    .map(|snapshot| crate::system_proxy::classify_drift(&snapshot, &expected))
                    .unwrap_or(crate::system_proxy::Drift::InSync)
            })
            .await
            .unwrap_or(crate::system_proxy::Drift::InSync);
            let _ = tx.send(BgMsg::ProxyDrift(drift));
        });
    }

    /// 收进托盘 = 把窗口**停放到屏幕外**（不再 `Visible(false)`，理由见 `platform::window_park`：
    /// 隐藏的窗口收不到 WM_PAINT，一帧都不跑，eframe 会把泵永久留在 Poll，实测烧满一个核）。
    /// 只能在 UI 线程调用。
    pub(crate) fn park_to_tray(&mut self) {
        if self.parked {
            return;
        }
        match crate::platform::park_main_window() {
            Some(spot) => {
                self.park_spot = Some(spot);
                self.parked = true;
                self.parked_at = Some(std::time::Instant::now());
            }
            None => {
                // 停不出去就不能装作"收进托盘了"：窗口还在用户眼前，状态必须说实话。
                crate::startup_log("[托盘] 停放失败（找不到主窗口），窗口保持原位");
                return;
            }
        }
        // 释放节点库缓存与工作集：这条是原来"隐藏到托盘"就有的行为，保留。
        let db = self.db.clone();
        self.rt.spawn(async move {
            db.trim_cached_pools().await;
        });
        crate::platform::trim_working_set();
    }

    /// 从托盘叫回来：把窗口放回停放前记下位置和大小，并夺回焦点。
    pub(crate) fn unpark_from_tray(&mut self, ctx: &egui::Context) {
        if let Some(spot) = self.park_spot.take() {
            crate::platform::unpark_main_window(spot);
        }
        self.parked = false;
        self.parked_at = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(crate) fn request_window_close(&mut self, ctx: &egui::Context) {
        if self.settings.close_to_tray && !self.force_quit {
            crate::startup_log("Title-bar close -> 收进托盘（停放到屏幕外）");
            self.park_to_tray();
        } else {
            crate::startup_log("Title-bar close -> quitting application");
            self.force_quit = true;
            self.tray.shutdown();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// 无边框窗口的缩放手：把 `resize_handle_rects` 算出的那几块铺成一层覆盖区。
    /// 中间不拦截鼠标，照常给内容用。
    fn ui_resize_handles(&self, ctx: &egui::Context) {
        let screen = ctx.input(|i| i.screen_rect());
        let min = screen.min;
        let handles = resize_handle_rects(screen);
        egui::Area::new(egui::Id::new("vpn-parser-resize-handles"))
            .fixed_pos(min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (index, (rect, direction, cursor)) in handles.into_iter().enumerate() {
                    let local = egui::Rect::from_min_max(rect.min - min.to_vec2(), rect.max - min.to_vec2());
                    let response = ui.interact(
                        local,
                        ui.id().with(("resize-handle", index)),
                        Sense::drag(),
                    );
                    if response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                    }
                    response.on_hover_cursor(cursor);
                }
            });
    }

}

/// 自绘标题栏的高度，和右上角三个按钮各自的宽度。`ui_title_bar` 按它画，
/// `resize_handle_rects` 也要按同一个数把按钮那一块从缩手感区里挖出去 ——
/// 两边各写一份迟早会飘，所以收在这儿。
pub(crate) const TITLE_BAR_H: f32 = 34.0;
pub(crate) const TITLE_BAR_BTN_W: f32 = 46.0;

/// 无边框窗口外圈该铺哪几块缩手感区。**没有 NorthEast 角**，而且左右两条从标题栏
/// 下沿才开始：这层盖在 `Order::Foreground`，以前右上角那块 30×30 正好整个压在 ✕ 上，
/// 鼠标一放到关闭键就变成斜向拉伸箭头，点下去还可能被 `BeginResize` 吃掉。
/// 代价只是右上角不能斜向拉伸 —— 右边、下面、左上角照常。
fn resize_handle_rects(
    screen: egui::Rect,
) -> Vec<(egui::Rect, egui::ResizeDirection, egui::CursorIcon)> {
    use egui::{CursorIcon as Cursor, ResizeDirection as Dir};
    const EDGE: f32 = 6.0;
    let (min, max) = (screen.min, screen.max);
    let corner = EDGE * 5.0;
    let side_top = min.y + TITLE_BAR_H;
    // 顶边条在按钮区之前停下（多留 8px，斜着进按钮区边缘也不算被盖住）。
    let north_stop = max.x - TITLE_BAR_BTN_W * 3.0 - 8.0;
    vec![
        (
            egui::Rect::from_min_max(egui::pos2(min.x, side_top), egui::pos2(min.x + EDGE, max.y - corner)),
            Dir::West,
            Cursor::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(max.x - EDGE, side_top), egui::pos2(max.x, max.y - corner)),
            Dir::East,
            Cursor::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(min.x + corner, min.y), egui::pos2(north_stop, min.y + EDGE)),
            Dir::North,
            Cursor::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(min.x + corner, max.y - EDGE), egui::pos2(max.x - corner, max.y)),
            Dir::South,
            Cursor::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(min, egui::pos2(min.x + corner, min.y + corner)),
            Dir::NorthWest,
            Cursor::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(min.x, max.y - corner), egui::pos2(min.x + corner, max.y)),
            Dir::SouthWest,
            Cursor::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(max.x - corner, max.y - corner), max),
            Dir::SouthEast,
            Cursor::ResizeNwSe,
        ),
    ]
}

impl eframe::App for VpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.reload_due
            && self.last_full_reload.elapsed() >= std::time::Duration::from_millis(800)
        {
            self.reload_due = false;
            self.last_full_reload = std::time::Instant::now();
            self.reload_all();
        }
        // Ctrl+K / Cmd+K 开命令面板。快捷键在这里全局读一次，不藏在任何面板内容里，
        // 否则焦点在某个 TextEdit 上时就再也收不到按键了。
        let palette_toggle = ctx.input(|i| {
            i.key_pressed(egui::Key::K)
                && (i.modifiers.command || i.modifiers.ctrl)
                && !i.modifiers.shift
                && !i.modifiers.alt
        }) && !widgets::focus_is_text_field(ctx);
        // 只在"焦点真的在文本框里"时才让路：面板自己那个搜索框获得焦点后再按一次 Ctrl+K
        // 不该把面板关掉（收起走 Esc 或点一行），在别的文本框里按 Ctrl+K 也不该误弹。
        // 不能拿 `focused().is_none()` 当门槛 —— DragValue 点一下也占焦点（见
        // `widgets::focus_is_text_field` 的注释），那样拖完一个数字快捷键就无声失效。
        if palette_toggle {
            if self.palette_open {
                self.palette_open = false;
            } else {
                self.palette_query.clear();
                self.open_command_palette();
            }
        }
        // Ctrl+V 不在这里读 —— `Event::Key{V}` 在 eframe 里根本不存在（见本函数末尾那条
        // 粘贴事件的处理）。Ctrl+K 是普通按键，还走 Key 事件。
        if self.frame_count < 5 {
            crate::startup_log(&format!(
                "VpnApp::update rendering frame {}",
                self.frame_count
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            // 无边框窗口的圆角只认一次：窗口这时候已经建好了，DWM 这条属性是粘性的。
            static ROUND_CORNERS: std::sync::Once = std::sync::Once::new();
            ROUND_CORNERS.call_once(crate::platform::round_window_corners);
        }
        self.frame_count = self.frame_count.saturating_add(1);

        // Register or refresh GUI event loop waker on tray handle
        let ctx_waker = ctx.clone();
        self.tray.set_waker(move || {
            ctx_waker.request_repaint();
        });

        // Handle system tray events
        while let Some(event) = self.tray.try_recv_event() {
            match event {
                crate::platform::TrayEvent::ShowWindow => {
                    crate::startup_log("TrayEvent::ShowWindow received, restoring window");
                    // 收进托盘现在是"停放到屏幕外"，所以还原就是把位置放回去。
                    // 这条事件现在真的有人消费：窗口没被隐藏，帧照跑（以前隐藏期间
                    // update() 一帧都不跑，三条菜单全都没反应）。
                    self.unpark_from_tray(ctx);
                }
                crate::platform::TrayEvent::ToggleProxy => {
                    if self.proxy_status.state == ProxyState::Running {
                        self.stop_proxy();
                    } else {
                        self.start_proxy();
                    }
                }
                crate::platform::TrayEvent::Quit => {
                    crate::startup_log("TrayEvent::Quit received, exiting application");
                    self.force_quit = true;
                    self.tray.shutdown();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
        }
        self.tray
            .set_proxy_running(self.proxy_status.state == ProxyState::Running);

        // Intercept window close event according to user setting (close_to_tray vs exit)
        // Guard with frame_count > 5 so false close events during window creation are ignored
        if self.frame_count > 5 && ctx.input(|i| i.viewport().close_requested()) {
            if self.settings.close_to_tray && !self.force_quit {
                crate::startup_log("Window close requested -> 收进托盘（停放到屏幕外）");
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.park_to_tray();
                return;
            } else {
                crate::startup_log("Window close requested -> quitting application");
                self.force_quit = true;
                self.tray.shutdown();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        }

        self.poll_bg(ctx);
        // 只在主题真的变了时才重建 Style：每帧 set_style 会让 egui 认为样式抖动，
        // 也会白白丢掉用户在运行时改过的字号。
        if self.settings.firewall_auto_allow && !self.firewall_seeded {
            self.firewall_seeded = true;
            if crate::platform::is_elevated() {
                self.apply_firewall_rules();
            } else {
                self.firewall_status =
                    Some("已勾选但本次未提权，规则没写入；用管理员权限启动后会自动补上".into());
            }
        }
        if self.applied_theme_id != self.settings.ui_theme {
            theme::apply(ctx, &self.settings.ui_theme);
            self.applied_theme_id = self.settings.ui_theme.clone();
        }

        if self.proxy_status.state == ProxyState::Running {
            if !self.traffic_polling
                && self.last_traffic_poll.elapsed() >= std::time::Duration::from_millis(1000)
            {
                self.traffic_polling = true;
                self.last_traffic_poll = std::time::Instant::now();
                self.poll_traffic_snapshot();
            }
            if self.last_groups_poll.elapsed() >= std::time::Duration::from_secs(5) {
                self.last_groups_poll = std::time::Instant::now();
                self.poll_kernel_groups();
            }
        }
        // 预备状态（一个内核都没装）自己把内核下下来；一次运行最多自己动一次。
        self.maybe_auto_prepare();
        // Degraded 更要测：那正是"出口已经不通了"的状态，探测和 auto_failover 无关。
        if matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        ) {
            self.check_failover();
        }
        // 出口通不等于浏览器会走它：核对注册表（内部限频 30 秒一次，且只在系统代理模式下）。
        self.maybe_check_proxy_drift();
        if self.proxy_status.state != ProxyState::Running
            && !self.kernel_groups.is_empty()
        {
            // 停掉内核后这些组就不存在了，留着会显示上一次的假状态。
            self.kernel_groups.clear();
        }

        // Periodic heartbeat tick to keep background timers responsive.
        // 停放在托盘里时降到 5 秒一拍：这一拍只是"别让后台结果永远没人取"，不是刷新动画。
        // 注意不能省掉 —— eframe 的延后重绘到期会把 ControlFlow 设成 Poll，但窗口现在仍可绘制，
        // 所以到期时会真跑一帧、由这一帧重新登记下一拍，泵不会退化成空转。
        let heartbeat = if self.parked {
            std::time::Duration::from_millis(5000)
        } else {
            std::time::Duration::from_millis(1000)
        };
        ctx.request_repaint_after(heartbeat);

        if self.close_after_elevation {
            // 提权实例已经接管，这个普通权限实例必须真的退掉，否则本机留下两个实例 =
            // 两个托盘图标、两个都自认为拥有系统代理。
            // 必须主动退出当前进程，释放单实例互斥量让提权实例接管，并彻底删除当前托盘图标。
            crate::startup_log("close_after_elevation -> shutting down tray and exiting parent process immediately");
            self.tray.shutdown();
            std::thread::sleep(std::time::Duration::from_millis(100));
            std::process::exit(0);
        }

        if let Some(url) = self.pending_open_url.take() {
            ctx.copy_text(url.clone());
            ctx.open_url(egui::OpenUrl::new_tab(&url));
            self.status = format!("已打开并复制导出链接: {url}");
        }

        let mut next_tab = self.tab;

        self.ui_title_bar(ctx);

        egui::SidePanel::left("sidebar")
            .exact_width(220.0)
            .resizable(false)
            .frame(
                Frame::NONE
                    .fill(theme::sidebar())
                    .stroke(Stroke::NONE)
                    .inner_margin(Margin::symmetric(12, 10)),
            )
            .show(ctx, |ui| self.ui_sidebar(ui, &mut next_tab));

        egui::TopBottomPanel::bottom("status")
            .frame(
                Frame::NONE
                    .fill(theme::sidebar())
                    .stroke(Stroke::new(1.0, theme::border_subtle()))
                    // 上下 8 → 5：这条本来就被内容顶高度，按钮压矮之后它跟着收。
                    .inner_margin(Margin::symmetric(14, 5)),
            )
            .show(ctx, |ui| {
                // 这条里的按钮（启动/停止代理·IP 检测·模式 chip·测速全部）要矮一档：
                // 纵向内边距 7 → 3，文字仍是 normal 字号，读得清但不再撑高度。
                ui.spacing_mut().button_padding = egui::vec2(11.0, 3.0);
                ui.horizontal(|ui| {
                    if self.busy {
                        ui.spinner();
                        ui.label(RichText::new("处理中").color(theme::accent()).strong().small());
                        ui.separator();
                    }
                    ui.label(RichText::new("状态").strong().color(theme::muted()).small());
                    ui.label(
                        RichText::new(&self.status)
                            .color(theme::text_primary())
                            .small(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 2026-10-07 他要求把控制按钮挪到状态栏右侧：先加的最靠右。
                        // 原来这儿的强调色选择器和会话统计都撤了 —— 主题在系统设置和
                        // Ctrl+K 里都还能改，流量读数挪到控制台表头那行。
                        self.ui_proxy_control_buttons(ui);
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(
                Frame::NONE
                    .fill(theme::page_bg())
                    .inner_margin(Margin::symmetric(18, 14)),
            )
            .show(ctx, |ui| {
                if self.tab != Tab::Proxy {
                    self.ui_stats_row(ui);
                    ui.add_space(10.0);
                }
                match self.tab {
                    Tab::Sources => self.ui_sources(ui),
                    Tab::Nodes => self.ui_nodes(ui),
                    Tab::Proxy => self.ui_proxy(ui),
                    Tab::Settings => self.ui_settings(ui),
                    Tab::Api => self.ui_api(ui),
                }
            });

        // 无边框之后系统的缩放手边没有了，这层覆盖只在窗口最外圈 6px 内响应。
        self.ui_resize_handles(ctx);

        if next_tab != self.tab {
            if self.tab == Tab::Settings && self.settings_dirty() {
                self.save_settings();
            }
            self.tab = next_tab;
            ctx.request_repaint();
        }

        self.ui_source_form(ctx);
        self.ui_detail(ctx);
        self.ui_homogeneity_dialog(ctx);
        self.ui_ip_risk_dialog(ctx);
        self.ui_kernel_dialog(ctx);
        self.ui_switch_log(ctx);
        self.ui_command_palette(ctx);
        self.ui_paste_dialog(ctx);

        // Ctrl+V 走的是这里，不是 Key 事件：egui-winit 一认出粘贴组合就把它换成
        // `Event::Paste(剪贴板内容)`，然后**直接 return，不再发 `Event::Key{V}`**
        // （egui-winit-0.31.1/src/lib.rs:771-780）。以前我在帧首查
        // `i.key_pressed(egui::Key::V)`，那个条件在本应用里永远不成立 —— 按 Ctrl+V 什么都不
        // 会发生，startup.log 里连一行都没有（2026-10-13 两次"没识别"的真正原因）。
        //
        // 读的位置在整帧画完之后：`Event::Paste` 会被文本框拿去插字，但 egui 并不把它从
        // 事件列表里摘掉（实测见 `a_paste_event_reaches_the_app_unless_a_focused_text_field_claimed_it`），
        // 所以"让不让路"还得用 `widgets::focus_is_text_field` 明确判一次 —— 焦点在文本框里
        // 就是普通粘贴；DragValue/滑块/按钮那些占住焦点的场合一律照常识别。
        let pasted = ctx.input_mut(|i| {
            let text = i
                .events
                .iter()
                .find_map(|event| match event {
                    egui::Event::Paste(text) => Some(text.clone()),
                    _ => None,
                })?;
            i.events
                .retain(|event| !matches!(event, egui::Event::Paste(_)));
            Some(text)
        });
        if let Some(text) = pasted {
            if widgets::focus_is_text_field(ctx) {
                crate::startup_log(&format!(
                    "粘贴事件：焦点在文本框里，内容已交给那个框（{} 字），不做识别",
                    text.chars().count()
                ));
            } else {
                crate::startup_log(&format!(
                    "粘贴事件：走订阅识别（{} 字）",
                    text.chars().count()
                ));
                self.apply_pasted_text(&text);
            }
        }

        // 诊断壳的界面轨迹：这个窗口是 OpenGL 表面，PrintWindow 抓回来是全黑，
        // 所以"界面到底说了什么"只能靠这条日志看。只在状态真的变了的那一帧写一行，
        // 不然每秒几十帧会把日志刷爆（托盘 hover 已经把 startup.log 淹过一次）。
        if crate::diag::enabled() {
            // 先回答"合成点击到底有没有进到 egui、落在哪个坐标"，再谈坐标对不对：
            // 没有这条，点击没反应时我分不清是消息没到还是我点错了地方。
            let pointer = ctx.input(|i| i.pointer.clone());
            if pointer.button_clicked(egui::PointerButton::Primary) {
                crate::startup_log(&format!(
                    "DIAGUI 收到左键 interact_pos={:?}",
                    pointer.interact_pos()
                ));
            }
            // 诊断壳的往返自测：过一会儿自己收进托盘、8 秒后再自己回来。
            // 我没法合成点击（winit 不吃 PostMessage 的鼠标消息），这条就是让
            // "park 前后 CPU 都应该是 0%、还原后窗口回到原位"能在真进程上量出来。
            if self.frame_count == 24 && !self.parked {
                crate::startup_log("DIAG: 自测 -> 模拟收进托盘");
                self.park_to_tray();
            }
            if let Some(at) = self.parked_at {
                if at.elapsed() >= std::time::Duration::from_secs(8) {
                    crate::startup_log("DIAG: 自测 -> 模拟从托盘叫回");
                    self.unpark_from_tray(ctx);
                }
            }
            let mut dialogs = Vec::new();
            if self.show_source_form {
                dialogs.push("源表单");
            }
            if self.palette_open {
                dialogs.push("命令面板");
            }
            if self.show_kernel_dialog {
                dialogs.push("内核");
            }
            if self.show_ip_risk_dialog {
                dialogs.push("IP检测");
            }
            if self.paste_outcome.is_some() {
                dialogs.push("剪贴板识别");
            }
            let line = format!(
                "DIAGUI tab={:?} busy={} proxy={:?} parked={} dialogs=[{}] status={}",
                self.tab,
                self.busy,
                self.proxy_status.state,
                self.parked,
                dialogs.join(","),
                self.status
            );
            if self.diag_trace.as_deref() != Some(line.as_str()) {
                self.diag_trace = Some(line);
                crate::startup_log(self.diag_trace.as_deref().unwrap_or_default());
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.settings_dirty() {
            // eframe 会周期性调用 save()（约每 30 秒一次，UI 线程上）；
            // 绝不能在这里 block_on 等数据库写锁，否则后台任务一重 UI 就冻住。
            let input = self.build_update_settings();
            let db = self.db.clone();
            self.rt.spawn(async move {
                if let Err(error) = db.update_settings(input).await {
                    tracing::error!(%error, "periodic settings save failed");
                }
            });
        }
        // 发布服务器配置存在 settings 表里（见 save_publisher）：eframe 的
        // `persistence` feature 没开，storage 在这个 build 里永远是 None。
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // 退出路径必须同步完成（runtime 马上要关），但要加超时上限：
        // SQLite 写锁被后台任务占用或内核停止卡住时，退出不能无限期挂死。
        if self.settings_dirty() {
            let input = self.build_update_settings();
            let db = self.db.clone();
            if let Err(error) = self.rt.block_on(async {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    db.update_settings(input),
                )
                .await
                {
                    Ok(res) => res,
                    Err(_) => Err(anyhow::anyhow!(
                        "save settings on GUI exit timed out after 5s"
                    )),
                }
            }) {
                tracing::error!(%error, "save settings on GUI exit failed");
            }
        }
        let proxy_manager = self.proxy_manager.clone();
        if let Err(error) = self.rt.block_on(async {
            match tokio::time::timeout(std::time::Duration::from_secs(8), proxy_manager.stop())
                .await
            {
                Ok(res) => res,
                Err(_) => Err(anyhow::anyhow!(
                    "proxy stop on GUI exit timed out after 8s; orphan core is cleaned up on next start"
                )),
            }
        }) {
            tracing::error!(%error, "restore system proxy on GUI exit failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_real_proxy_progress_for_status_bar() {
        let progress = SpeedTestProgress {
            phase: SpeedTestPhase::RealProxy,
            done: 1260,
            total: 15656,
            ok: 247,
            fail: 1013,
            elapsed: Duration::from_millis(18_400),
            kernel: Some(KernelKind::Xray),
            batch_index: Some(2),
            batch_total: Some(4),
        };

        assert_eq!(
            format_speedtest_progress(&progress),
            "真实代理验证 · Xray 批次 2/4 1260/15656（8.0%） 成功 247 / 失败 1013，耗时 18.4s"
        );
    }

    #[test]
    fn tun_always_requests_elevation_for_non_admin_process() {
        assert!(requires_tun_elevation(ProxyMode::Tun, false));
        assert!(!requires_tun_elevation(ProxyMode::Tun, true));
        assert!(!requires_tun_elevation(ProxyMode::System, false));
    }

    #[test]
    fn settings_edit_state_dirty_detection_and_sync() {
        let settings = AppSettings::default();
        let mut state = SettingsEditState::from_settings(&settings);

        assert!(!state.is_dirty(&settings));

        state.interval_min = 30;
        assert!(state.is_dirty(&settings));

        state = SettingsEditState::from_settings(&settings);
        assert!(!state.is_dirty(&settings));

        state.auto_fetch = !settings.auto_fetch_enabled;
        assert!(state.is_dirty(&settings));

        state = SettingsEditState::from_settings(&settings);
        state.speedtest_concurrency = 256;
        assert!(state.is_dirty(&settings));

        state = SettingsEditState::from_settings(&settings);
        state.http_server_enabled = !settings.http_server_enabled;
        assert!(state.is_dirty(&settings));

        state = SettingsEditState::from_settings(&settings);
        state.close_to_tray = !settings.close_to_tray;
        assert!(state.is_dirty(&settings));
    }

    #[test]
    fn country_flag_and_name_mappings() {
        assert_eq!(country_flag_and_name("HK"), ("🇭🇰", "香港"));
        assert_eq!(country_flag_and_name("jp"), ("🇯🇵", "日本"));
        assert_eq!(country_flag_and_name("US"), ("🇺🇸", "美国"));
        assert_eq!(country_flag_and_name("sg"), ("🇸🇬", "新加坡"));
        assert_eq!(country_flag_and_name("TW"), ("🇨🇳", "台湾"));
        assert_eq!(country_flag_and_name("KR"), ("🇰🇷", "韩国"));
        assert_eq!(country_flag_and_name("unknown"), ("🌐", "其它"));
    }

    #[test]
    fn format_speed_and_bytes_display() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");

        assert_eq!(format_speed(256), "256 B/s");
        assert_eq!(format_speed(120 * 1024), "120.0 KB/s");
        assert_eq!(format_speed(2560 * 1024), "2.50 MB/s");
    }

    #[test]
    fn proxy_nodes_sort_by_latency_and_name() {
        let n1 = NodeSummary {
            id: "1".into(),
            source_id: None,
            protocol: "ss".into(),
            name: "B-Node".into(),
            server: "1.1.1.1".into(),
            port: 443,
            country: None,
            latency_ms: Some(150),
            latency_ok: Some(true),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            updated_at: chrono::Utc::now(),
        };
        let mut n2 = n1.clone();
        n2.id = "2".into();
        n2.name = "A-Node".into();
        n2.latency_ms = Some(45);

        let mut n3 = n1.clone();
        n3.id = "3".into();
        n3.name = "C-Node".into();
        n3.latency_ms = Some(90);

        let mut list = vec![n1.clone(), n2.clone(), n3.clone()];

        // Test sorting by latency (lowest first)
        list.sort_by_key(|n| {
            (
                n.latency_ok != Some(true),
                n.latency_ms.unwrap_or(i64::MAX),
                n.name.to_lowercase(),
            )
        });
        assert_eq!(
            list.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["2", "3", "1"]
        );

        // Test sorting by name (A-Z)
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        assert_eq!(
            list.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["2", "1", "3"]
        );
    }

    #[test]
    fn proxy_switch_sequence_invalidates_stale_responses() {
        let mut current_seq = 1u64;
        let incoming_stale_seq = 1u64;

        // User clicks node 2 rapidly, advancing sequence
        current_seq += 1;
        assert_ne!(current_seq, incoming_stale_seq);

        // Newer click matching current_seq
        let incoming_fresh_seq = current_seq;
        assert_eq!(current_seq, incoming_fresh_seq);
    }

    /// 跑一遍标题栏那种"exact_height 面板 + 右上角三个格子"的写法，返回面板真正占掉的
    /// 高度。面板只画到 exact_height，但 `Frame` 的外接矩形是按内容 `min_rect` 反推的：
    /// 两个数不等，中间就是没人画的窗口底色 —— 标题栏下面那条黑带。
    fn title_bar_allocated_height(draw_buttons_with_widget: bool) -> f32 {
        use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Vec2};
        const BAR_H: f32 = 34.0;
        const BTN_W: f32 = 46.0;

        let ctx = egui::Context::default();
        ctx.style_mut(|style| {
            style.spacing.item_spacing = Vec2::new(10.0, 9.0);
            style.spacing.button_padding = Vec2::new(14.0, 8.0);
        });
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1280.0, 800.0),
        ));

        let mut allocated = 0.0_f32;
        ctx.run(input, |ctx| {
            let response = egui::TopBottomPanel::top("title_bar_probe")
                .exact_height(BAR_H)
                // 出问题的那版用的就是上下 5px：格子只剩 24 高，比按钮想要的 32 矮。
                .frame(
                    Frame::NONE
                        .fill(Color32::WHITE)
                        .inner_margin(Margin::symmetric(12, 5)),
                )
                .show(ctx, |ui| {
                    let bar = ui.max_rect();
                    let cell = |index: usize| {
                        egui::Rect::from_min_max(
                            egui::pos2(bar.max.x - (index as f32 + 1.0) * BTN_W, bar.min.y),
                            egui::pos2(bar.max.x - index as f32 * BTN_W, bar.max.y),
                        )
                    };
                    if draw_buttons_with_widget {
                        ui.label(RichText::new("VPN Parser").size(12.5));
                        for index in 0..3 {
                            let rect = cell(index);
                            ui.put(
                                rect,
                                egui::Button::new(RichText::new("✕").size(13.0))
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(CornerRadius::same(8)),
                            );
                        }
                    } else {
                        ui.allocate_rect(bar, Sense::hover());
                        for index in 0..3 {
                            ui.interact(cell(index), ui.id().with(index), Sense::click());
                        }
                    }
                });
            allocated = response.response.rect.height();
        });
        allocated
    }

    #[test]
    fn button_widgets_overflow_a_fixed_height_panel() {
        // 这就是黑带的成因，写成测试是因为它"编译通过、只是多一条黑带"。
        let eaten = title_bar_allocated_height(true);
        assert!(
            eaten > 34.0,
            "Button 本该把面板顶出 exact_height，实测占位 {eaten:.1}"
        );
    }

    #[test]
    fn painted_title_bar_keeps_the_panel_height_it_claims() {
        // 自绘 + interact 不占位，所以"画到的"和"分给下面面板的"永远是同一条边。
        let eaten = title_bar_allocated_height(false);
        assert!(
            (eaten - 34.0).abs() < 0.5,
            "标题栏应该恰好占 34，实测 {eaten:.1}"
        );
    }

    #[test]
    fn resize_handles_never_cover_the_title_bar_buttons() {
        use egui::{Rect, ResizeDirection as Dir};
        let screen = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1280.0, 800.0));
        let handles = resize_handle_rects(screen);
        let dirs: Vec<Dir> = handles.iter().map(|(_, dir, _)| *dir).collect();

        // 缩放手仍然齐全：四条边 + 三个角，只少了右上角。
        for want in [
            Dir::West,
            Dir::East,
            Dir::North,
            Dir::South,
            Dir::NorthWest,
            Dir::SouthWest,
            Dir::SouthEast,
        ] {
            assert!(dirs.contains(&want), "缩手感区少了 {want:?}");
        }
        assert!(
            !dirs.contains(&Dir::NorthEast),
            "右上角必须让给标题栏的 ✕，不然鼠标一放上去就变斜向拉伸箭头"
        );

        // 关键一条：任何一块都不能和右上角三个按钮的格子有正面积交集。
        for index in 0..3 {
            let cell = Rect::from_min_max(
                egui::pos2(screen.max.x - (index as f32 + 1.0) * TITLE_BAR_BTN_W, 0.0),
                egui::pos2(screen.max.x - index as f32 * TITLE_BAR_BTN_W, TITLE_BAR_H),
            );
            for (rect, dir, _) in &handles {
                let overlap = rect.intersect(cell);
                assert!(
                    !overlap.is_positive(),
                    "{dir:?} 压住了按钮 #{index}：{} ∩ {} = {overlap:?}",
                    rect,
                    cell
                );
            }
        }
    }

    #[test]
    fn palette_search_box_keeps_focus_when_the_result_list_shrinks() {
        // before = 搜索框之前有几个控件；rows = 搜索框之后的结果行数。
        fn probe(ctx: &egui::Context, rows: usize, before: usize, take_focus: bool) -> egui::Id {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1280.0, 800.0),
            ));
            let mut edit_id = egui::Id::NULL;
            let mut query = String::new();
            let _ = ctx.run(input, |ctx| {
                let _ = egui::Window::new("命令面板").show(ctx, |ui| {
                    for index in 0..before {
                        ui.label(format!("占位 {index}"));
                    }
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut query).desired_width(420.0));
                    edit_id = response.id;
                    if take_focus {
                        response.request_focus();
                    }
                    ui.separator();
                    for _ in 0..rows {
                        let _ = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 30.0),
                            Sense::click(),
                        );
                    }
                });
            });
            edit_id
        }

        // 面板的真实顺序：结果从 12 行过滤到 3 行，搜索框的 id 和焦点都不动。
        let ctx = egui::Context::default();
        let first = probe(&ctx, 12, 0, true);
        assert_eq!(ctx.memory(|m| m.focused()), Some(first));
        let second = probe(&ctx, 3, 0, false);
        assert_eq!(second, first, "搜索框 id 跟着结果行数变了");
        assert_eq!(ctx.memory(|m| m.focused()), Some(first), "过滤一次就丢焦点");

        // 反向验证：把行数可变的列表挪到搜索框前面，id 就整体漂移 —— 中文输入框会当场失效。
        let bad = egui::Context::default();
        let b1 = probe(&bad, 12, 12, true);
        let b2 = probe(&bad, 3, 3, false);
        assert_ne!(b1, b2, "前置的条件控件本来应该让 id 漂移");
    }

    /// 底部状态栏那一排的高度断言（2026-10-08 他要压矮）。
    /// 返回（整条高度, 每个控件的高度）；`pad_y`/`margin_y` 就是按钮纵向内边距和面板
    /// 上下边距 —— 改前 8/8，改后 3/5。
    fn status_bar_probe(pad_y: f32, margin_y: i8) -> (f32, Vec<f32>) {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| {
            // 和应用里同一份全局样式：按钮本来是 14x8。
            style.spacing.button_padding = Vec2::new(14.0, 8.0);
        });
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1330.0, 800.0),
        ));
        let mut heights = Vec::new();
        let mut total = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::bottom("status_probe")
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, margin_y)))
                .show(ctx, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(11.0, pad_y);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("状态").small());
                        ui.label(RichText::new("正在拉取订阅").small());
                        for text in ["启动代理", "停止代理", "IP 检测", "测速全部"] {
                            heights.push(ui.button(text).rect.height());
                        }
                        heights.push(
                            egui::Frame::NONE
                                .inner_margin(egui::Margin::symmetric(9, (pad_y - 1.0) as i8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("系统代理").small());
                                    });
                                })
                                .response
                                .rect
                                .height(),
                        );
                    });
                    total = ui.min_rect().height() + 2.0 * margin_y as f32;
                });
        });
        (total, heights)
    }

    #[test]
    fn status_bar_is_compact_but_the_buttons_still_fit_their_text() {
        let (before, before_cells) = status_bar_probe(8.0, 8);
        let (after, after_cells) = status_bar_probe(3.0, 5);
        assert!(
            before - after >= 12.0,
            "压矮没生效：改前 {before:.1}px，改后 {after:.1}px"
        );
        assert!(after <= 34.0, "状态栏还是太高：{after:.1}px");
        for (index, height) in after_cells.iter().enumerate() {
            assert!(
                *height >= 20.0,
                "控件 #{index} 只有 {height:.1}px，文字要被切了（改前是 {}px）",
                before_cells[index]
            );
        }
    }

    /// 全局粘贴事件的两个前提，都实测过（2026-10-13）：
    /// 1. 文本框**有焦点**时，egui 会把内容粘进框里 —— 但它**不会**把 `Event::Paste`
    ///    从事件列表里摘掉（这条和我原来的假设相反，是测出来的）。所以帧尾那段必须自己
    ///    用 `widgets::focus_is_text_field` 判一次，否则"在 URL 框里粘地址"会既粘进框、
    ///    又被我拿去建一个源。
    /// 2. 没焦点时事件照旧留在列表里 —— 那才是全局识别的入口。
    #[test]
    fn a_paste_event_reaches_the_app_unless_a_focused_text_field_claimed_it() {
        let base = {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            ));
            input
        };
        // 帧 1 拿到文本框的矩形；帧 2 决定要不要先点它（拿焦点）；帧 3 喂 Paste。
        let run_case = |click_first: bool| -> (String, bool, bool) {
            let ctx = egui::Context::default();
            let mut buffer = String::new();
            let mut rect = egui::Rect::NOTHING;
            let _ = ctx.run(base.clone(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = ui.add(egui::TextEdit::singleline(&mut buffer)).rect;
                });
            });
            if click_first {
                let mut input = base.clone();
                input.events = vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ];
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let field = ui.add(egui::TextEdit::singleline(&mut buffer));
                        crate::gui::widgets::note_text_field(ui, &field);
                    });
                });
            }
            let mut input = base.clone();
            input.events = vec![egui::Event::Paste("https://a.b/c?token=1".into())];
            let mut leftover = false;
            let mut defers = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut buffer));
                    crate::gui::widgets::note_text_field(ui, &field);
                });
                // 帧尾读一次 —— 和产品里那段同一口径。
                leftover = ctx
                    .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
                defers = crate::gui::widgets::focus_is_text_field(ctx);
            });
            (buffer, leftover, defers)
        };

        let (text, leftover, defers) = run_case(true);
        assert_eq!(text, "https://a.b/c?token=1", "焦点在框里时内容要粘进框");
        assert!(
            leftover,
            "实测：文本框粘完并不把 Paste 事件摘掉 —— 所以帧尾一定要自己判焦点"
        );
        assert!(defers, "焦点在框里时门槛必须让路，否则会二次建源");

        let (text, leftover, defers) = run_case(false);
        assert_eq!(text, "", "没焦点时不该往框里塞东西");
        assert!(leftover, "没焦点时事件必须还留着");
        assert!(!defers, "没焦点时门槛要开 —— 这就是全局识别的入口");
    }

    /// 全局 Ctrl+V / Ctrl+K 那道"让路"门槛，只该给真正的文本框让路。
    ///
    /// 这条测试是 2026-10-13 那次"用户按 Ctrl+V 没反应"留下的：旧写法看的是
    /// `memory().focused().is_none()`，而 egui 的 DragValue 点一下也会占住焦点
    /// （`egui-0.31.1/src/widgets/drag_value.rs:622`）—— 界面上"候选数 / 延迟阈值 /
    /// 端口"全是 DragValue，拖完数字再按 Ctrl+V 就一声不响。新判据认的是
    /// `widgets::focus_is_text_field`，数字框不再挡路，文本框照旧让路。
    #[test]
    fn the_paste_gate_defers_only_to_real_text_fields() {
        // 一帧里两个控件，各用独立的 id 来源：egui 对 `DragValue::new(&mut n)` /
        // `TextEdit::singleline(&mut s)` 是按变量地址造 id 的，临时量地址会复用，
        // 撞了 id 之后测到的是撞车，不是焦点。
        fn frame(
            ctx: &egui::Context,
            input: egui::RawInput,
            number: &mut i64,
            text: &mut String,
        ) -> (egui::Rect, egui::Rect, Option<egui::Id>, bool) {
            let mut out = (
                egui::Rect::NOTHING,
                egui::Rect::NOTHING,
                None,
                false,
            );
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    out.0 = ui.add(egui::DragValue::new(number)).rect;
                    let field = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                    // 和产品里那 15 个文本框同一口径：画完登记一次焦点。
                    crate::gui::widgets::note_text_field(ui, &field);
                    out.1 = field.rect;
                });
                out.2 = ctx.memory(|m| m.focused());
                out.3 = crate::gui::widgets::focus_is_text_field(ctx);
            });
            out
        }

        let base = {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 700.0),
            ));
            input
        };
        let click_at = |pos: egui::Pos2| {
            vec![
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };

        for (what, expect_gate) in [("数字框 DragValue", false), ("文本框 TextEdit", true)] {
            let ctx = egui::Context::default();
            let mut number = 0i64;
            let mut text = String::new();
            let (drag, edit, _, _) = frame(&ctx, base.clone(), &mut number, &mut text);
            assert!(drag.is_positive() && edit.is_positive(), "控件没画出来");
            let target = if what.starts_with("数字框") {
                drag.center()
            } else {
                edit.center()
            };
            let mut input = base.clone();
            input.events = click_at(target);
            let (_, _, focused, gate) = frame(&ctx, input, &mut number, &mut text);
            assert!(
                focused.is_some(),
                "{what}：这一点根本没落到控件上（focused=None），测了也是白测"
            );
            assert_eq!(
                gate, expect_gate,
                "{what}：旧门槛会挡住(focused={focused:?})，新判据必须给出 {expect_gate}"
            );
        }
    }
}

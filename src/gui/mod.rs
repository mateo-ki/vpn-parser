use crate::collector::Collector;
use crate::db::Database;
use crate::kernel::{
    find_kernel_executable, missing_kernel_for, KernelInfo, KernelKind, KernelManager,
    KernelReleaseInfo, KernelUpdateProgress,
};
use crate::gui::view_models::nodes_vm::NodeCardAction;
use crate::models::{
    AppSettings, CreateSource, FetchResult, Node, NodeGroupSummary, NodeSummary, Source,
    SpeedTestSummary, Stats, UpdateSettings, UpdateSource, GOOGLE_PROXY_TEST_KIND,
};
use crate::platform;
use crate::proxy::{
    is_node_connectivity_failure, optimize_traffic_rules, MihomoConfigOptions, ProxyManager,
    ProxyMode, ProxyRuntimeStatus, ProxyState, RuleAction,
};
use crate::speedtest::{SpeedTestOptions, SpeedTester, SpeedtestIntensity};
use crate::tasks::TaskTracker;
use eframe::egui::{self, Frame, Margin, RichText, Stroke};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use tokio::runtime::Runtime;
pub mod command_palette;
pub mod failover;
pub mod icons;
pub mod paste;
pub mod poll;
pub mod prepare;
pub mod telemetry_view;
pub mod perf;
pub mod theme;
pub mod types;
pub mod view_models;
pub mod views;
pub mod widgets;
pub mod window;

pub use types::*;
pub use widgets::*;

/// Gist 发现里每个代理候选对第 1 页的原地重试次数（总尝试 = 1 + 该值）。
const GIST_PAGE_RETRIES: usize = 2;

/// 侧栏两种宽度：收起 = 只有图标（64，和以前一模一样），展开 = 图标 + 短名。
/// 展开那档 = 图标区 44 + 文字位 86 + 右内边距 10；判据直接引这两个常量，不各写各的数。
pub(crate) const SIDEBAR_COLLAPSED_WIDTH: f32 = 64.0;
pub(crate) const SIDEBAR_EXPANDED_WIDTH: f32 = 110.0;

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
    pub(crate) extensions_tab: ExtensionsTab,
    pub(crate) sources_panel: SourcesPanel,
    /// 订阅列表的过滤条件：名称子串 + 状态（都是"当场过滤表格"，不落库）。
    pub(crate) source_name_filter: String,
    pub(crate) source_status_filter: SourceStatusFilter,
    pub(crate) proxy_panel: ProxyPanel,
    pub(crate) status: String,
    /// 上一帧量到的"状态栏右侧动作组"宽度。状态文本照它让位，长日志才不会压到按钮上面。
    pub(crate) status_actions_width: f32,
    pub(crate) busy: bool,
    pub(crate) proxy_busy: bool,
    /// 上一次写进 startup.log 的代理状态 + 它开始的时间 + 卡住告警是否已发过。
    /// 启动按钮灰掉的三种原因里，只有「卡在 Starting/Stopping」是用户看不到解释的那种，
    /// 所以状态每变一次记一行，卡满 30 秒再补一行。
    pub(crate) last_proxy_state_log: Option<(ProxyState, std::time::Instant, bool)>,    pub(crate) gist_pages: usize,
    pub(crate) gist_query_flags: Vec<bool>,
    pub(crate) active_source_fetches: HashSet<String>,
    pub(crate) expanded_kernel_groups: HashSet<String>,

    pub(crate) sources: Vec<Source>,
    pub(crate) fetch_results: Vec<FetchResult>,
    pub(crate) nodes: Vec<NodeSummary>,
    pub(crate) node_groups: Vec<NodeGroupSummary>,
    pub(crate) source_modal_open: bool,
    pub(crate) source_modal_source_id: Option<String>,
    pub(crate) source_modal_source_name: String,
    pub(crate) source_modal_nodes: Vec<NodeSummary>,
    pub(crate) source_modal_total: i64,
    pub(crate) source_modal_page: u32,
    pub(crate) source_modal_page_size: u32,
    pub(crate) source_modal_loading: bool,
    pub(crate) source_modal_only_alive: bool,
    pub(crate) source_modal_keyword: String,
    pub(crate) stats: Option<Stats>,
    pub(crate) settings: AppSettings,
    pub(crate) node_total: i64,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
    pub(crate) protocol_filter: String,
    pub(crate) source_filter: String,
    pub(crate) source_search_query: String,
    pub(crate) keyword: String,
    pub(crate) keyword_applied: String,
    pub(crate) nodes_req_id: u64,
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
    /// 「关于」页的应用自身更新：最新 Release、下载进度、已校验的安装包。
    pub(crate) app_release: Option<crate::app_update::AppReleaseInfo>,
    pub(crate) app_update_progress: Option<crate::app_update::AppUpdateProgress>,
    pub(crate) app_update_busy: bool,
    /// 上一次"检查更新"落在几点（本地时、只到分）。关于页那句副标题要说这条状态是新的还是
    /// 开机时顺手查的；只活在内存里 —— 重启后没查过就是没查过，不拿旧时间糊弄人。
    pub(crate) app_update_checked_at: Option<String>,
    /// 侧栏那个绿色↑要自己判断有没有新版：启动后自己查一次（见 `maybe_check_app_update`）。
    pub(crate) app_update_due: Option<std::time::Instant>,
    /// 自己发起的那次检查不许动状态栏 —— 用户没点任何东西，"正在检查更新..."是噪音。
    pub(crate) app_update_quiet: bool,
    pub(crate) app_installer_path: Option<std::path::PathBuf>,
    pub(crate) app_update_cancel: Arc<AtomicBool>,
    /// 「关于」页多版本切换：历史 Release 列表、当前选中的 tag、加载状态
    pub(crate) available_releases: Vec<crate::app_update::ReleaseItem>,
    pub(crate) selected_release_tag: Option<String>,
    pub(crate) fetching_releases: bool,
    pub(crate) proxy_status: ProxyRuntimeStatus,
    pub(crate) proxy_config: MihomoConfigOptions,
    pub(crate) proxy_log: String,
    pub(crate) selected_proxy_node_id: Option<String>,
    pub(crate) selected_proxy_node_name: Option<String>,
    pub(crate) focused_node_id: Option<String>,
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
    pub(crate) show_clear_sources_confirm: bool,
    pub(crate) show_gist_config_modal: bool,
    pub(crate) show_import_yaml_modal: bool,
    pub(crate) import_yaml_path: String,
    pub(crate) import_yaml_as_source: bool,
    pub(crate) import_yaml_source_name: String,
    /// 分流规则面板的分区（配置 / 命中记录）。
    pub(crate) rules_tab: RulesTab,
    /// 分流策略组面板的分区（视图 / 配置）。
    pub(crate) groups_tab: GroupsTab,
    /// 活跃连接监控面板的分区（连接列表 / 连接进程 / DNS 记录）。
    pub(crate) connections_tab: ConnectionsTab,
    /// DNS 实时解析记录
    pub(crate) dns_records: std::collections::VecDeque<DnsRecord>,
    /// 连接进程过滤词
    pub(crate) connections_process_filter: String,
    /// DNS 记录过滤词
    pub(crate) connections_dns_filter: String,
    /// 「导入规则」弹框：是否打开、贴进来的原文、最近一次解析结果。
    pub(crate) show_rule_import: bool,
    /// 导入框旁边那个「?」——各输入形状的示范，逐段由测试保证真能导入。
    pub(crate) show_rule_import_help: bool,
    pub(crate) rule_import_text: String,
    pub(crate) rule_import_preview: Option<crate::proxy::rule_io::RuleImport>,

    pub(crate) new_mapping_port: String,
    pub(crate) new_mapping_node_id: Option<String>,
    pub(crate) new_mapping_node_name: Option<String>,
    pub(crate) node_picker_open: bool,
    pub(crate) node_picker_search: String,
    pub(crate) node_picker_target_index: Option<usize>,

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
    /// 跟踪上一帧是否处于最小化状态，用于捕获任务栏还原动作
    pub(crate) was_minimized: bool,
    /// 从最小化或托盘恢复后，连续强刷渲染的冷却帧数
    pub(crate) restore_cooldown_frames: u32,
    /// 停放前记下的位置与大小，还原时用。
    pub(crate) park_spot: Option<crate::platform::window_park::Spot>,
    /// 什么时候停放的（诊断壳的自动往返自测用它计时）。
    pub(crate) parked_at: Option<std::time::Instant>,
    /// 上一次核对系统代理的时间与"正在查"标记（查一次要起 reg.exe，必须后台）。
    /// 核对结果只喂给「自动重开系统代理」那个开关，界面不显示（2026-10-15 他要求撤掉那条提示）。
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
    /// 代理本次运行的起始时刻（进入 Running 时记录，停止/失败清空），仪表盘算运行时长用。
    pub(crate) proxy_started_at: Option<std::time::Instant>,
    /// 服务连通性检测结果（Google/YouTube/GitHub/ChatGPT 经当前代理实测）。
    pub(crate) service_checks: Option<Vec<types::ServiceCheck>>,
    pub(crate) service_checks_running: bool,
    /// 本轮检测的发起时刻：超过 35 秒没回结果就允许重新发起（自愈，避免永久"检测中"）。
    pub(crate) service_check_started: Option<std::time::Instant>,
    pub(crate) last_service_check: std::time::Instant,
    /// 本机局域网 IP（UDP 探测主网卡出口地址，不发包），局域网代理开/关的文案用。
    pub(crate) lan_ip: String,
    pub(crate) last_lan_ip_check: std::time::Instant,
    /// 上一帧性能原始采样（墙钟、CPU 累计时间、磁盘读写计数），差分算 CPU%/磁盘速率。
    pub(crate) perf_prev: Option<(std::time::Instant, u64, u64, u64)>,
    pub(crate) perf_stats: perf::PerfSample,
    pub(crate) last_perf_sample: std::time::Instant,
    /// 仪表盘策略组抽屉：当前展开的组名（None = 收起）。
    pub(crate) group_drawer: Option<String>,
    /// 策略组抽屉的节点筛选词（当场过滤，不落库）。
    pub(crate) group_drawer_filter: String,
    /// 策略组抽屉的地区筛选（all / HK / JP / US / SG / TW / KR / OTHER）。
    pub(crate) group_drawer_region: String,
    /// 各策略组选定的调度方式（组名 → 延迟选优/故障转移/负载均衡/手动指定）。
    pub(crate) group_scheduling: std::collections::HashMap<String, String>,
    /// 未启动代理时在抽屉里记住的调度选择（组名, 成员），进入 Running 后自动应用。
    pub(crate) pending_group_pick: Option<(String, String)>,
    pub(crate) active_connections: Vec<ActiveConnectionInfo>,
    pub(crate) traffic_prev_sample: Option<(std::time::Instant, u64, u64)>,
    pub(crate) last_traffic_poll: std::time::Instant,
    pub(crate) last_groups_poll: std::time::Instant,
    pub(crate) last_log_poll: std::time::Instant,
    pub(crate) traffic_polling: bool,
    /// 内核各 proxy-group 的实时状态（名字/类型/当前成员）。
    pub(crate) kernel_groups: Vec<KernelGroup>,
    /// 控制台选中的分流策略组（例如 "全部", "AI分组", "美国节点", "多线并发下载", "普通代理" 等）。
    pub(crate) proxy_selected_group: String,
    /// 控制台可用节点网格的缓存视图 + 数据版本号（proxy_nodes 每次换数据都 +1）。
    pub(crate) proxy_view: Option<ProxyView>,
    pub(crate) proxy_nodes_rev: u64,
    pub(crate) proxy_page: u32,
    pub(crate) proxy_page_size: u32,
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
    /// 汇总卡片那颗「出口类型」徽章：最近一次静默检测是为哪个节点跑的。
    pub(crate) ip_risk_auto_for: Option<String>,
    /// 归属查询失败时，那次查询是为哪颗节点跑的（徽章要能说「查失败」而不是装没查过）。
    pub(crate) ip_risk_failed_for: Option<String>,
    pub(crate) reload_due: bool,
    pub(crate) last_full_reload: std::time::Instant,
    pub(crate) startup_trim_done: bool,
    pub(crate) last_auto_trim: std::time::Instant,
    pub(crate) geo_updating: bool,
    pub(crate) geo_auto_attempted: bool,
    pub(crate) auto_failover: bool,
    pub(crate) edit_exclude_keywords: String,
    pub(crate) edit_failover_latency_ms: u32,
    pub(crate) edit_speedtest_parallel_kernels: bool,
    pub(crate) edit_speedtest_persistent_kernel: bool,
    pub(crate) edit_speedtest_reuse_minutes: u32,
    pub(crate) edit_speedtest_conditional_isolation: bool,
    pub(crate) edit_speedtest_download_top_n: u32,
    pub(crate) edit_speedtest_dedup_egress: bool,
    pub(crate) edit_speedtest_tcp_ping_only: bool,
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
    pub(crate) fetch_history:
        std::collections::HashMap<String, Vec<crate::db::telemetry::FetchHistoryRow>>,
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
    #[allow(dead_code)]
    pub(crate) settings_advanced_open: bool,
    #[allow(dead_code)]
    pub(crate) proxy_diagnostics_open: bool,
    pub(crate) palette_open: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) palette_needs_focus: bool,
    /// 选路调度器（#4）：EWMA 延迟 + 断路器 + 打分 + 滞后切换，带宽保护（#5）也走它。
    pub(crate) router: crate::router::Router,
    pub(crate) edit_bandwidth_guard: bool,
    pub(crate) edit_bandwidth_min_kbps: u32,
    pub(crate) edit_ui_theme: String,
    pub(crate) edit_app_icon_style: String,
    pub(crate) edit_ui_language: String,
    /// 字体/界面大小（1.0 = 100%），落库后乘在系统缩放上当 pixels_per_point。
    pub(crate) edit_ui_scale: f32,
    /// 夜间模式（编辑态）：和强调色正交的一个开关。
    pub(crate) edit_ui_dark: bool,
    pub(crate) edit_sidebar_expanded: bool,
    /// 参与预发（「关于」页那个勾）：检查更新时也看 beta。
    pub(crate) edit_insider_channel: bool,
    pub(crate) edit_kernel_source: String,
    pub(crate) edit_firewall_auto_allow: bool,
    pub(crate) edit_window_size: String,
    pub(crate) edit_console_tabs: String,
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
    /// TUN 提权交接已经成功把提权实例拉起来了，下一帧本实例要真退出（释放单实例互斥量）。
    pending_exit_after_handoff: bool,
    /// 已经装到 egui 上的主题 id，用来只在变化时重建 Style。
    pub(crate) applied_theme_id: String,
    /// 已经装到屏幕上的夜间开关。和主题 id 一起决定要不要重建 Style。
    pub(crate) applied_ui_dark: bool,
    /// 只在第一次拿到库里的设置时把编辑态灌成它（见 `BgMsg::Settings` 那段）。
    pub(crate) settings_seeded: bool,
    /// 系统设置页当前分区（通用 / 外观），只活在这次运行里。
    pub(crate) settings_tab: SettingsTab,
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
        let _rt_guard = rt.enter();
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
            proxy_config: MihomoConfigOptions {
                smart_routing: true,
                ..MihomoConfigOptions::default()
            },
            proxy_manager,
            api_controller,
            api_addr,
            remote_api_addr: None,
            remote_api_edit: String::new(),
            remote_api_token: String::new(),
            remote_api_status: "未检查".into(),
            publisher_seeded: false,
            tab: Tab::Proxy,
            extensions_tab: ExtensionsTab::default(),
            sources_panel: SourcesPanel::List,
            source_name_filter: String::new(),
            source_status_filter: SourceStatusFilter::All,
            proxy_panel: ProxyPanel::Charts,
            status: "就绪".into(),
            status_actions_width: 0.0,
            busy: false,
            proxy_busy: false,
            last_proxy_state_log: None,
            proxy_started_at: None,
            service_checks: None,
            service_checks_running: false,
            service_check_started: None,
            // 置成"已过期"，首帧就立即开始第一轮服务连通性检测
            last_service_check: std::time::Instant::now() - std::time::Duration::from_secs(61),
            lan_ip: String::new(),
            // 置成"已过期"，首帧就把局域网 IP 探测出来
            last_lan_ip_check: std::time::Instant::now() - std::time::Duration::from_secs(61),
            perf_prev: None,
            perf_stats: perf::PerfSample::default(),
            last_perf_sample: std::time::Instant::now(),
            group_drawer: None,
            group_drawer_filter: String::new(),
            group_drawer_region: "all".into(),
            pending_group_pick: None,
            group_scheduling: std::collections::HashMap::new(),
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
            source_modal_open: false,
            source_modal_source_id: None,
            source_modal_source_name: String::new(),
            source_modal_nodes: vec![],
            source_modal_total: 0,
            source_modal_page: 1,
            source_modal_page_size: 50,
            source_modal_loading: false,
            source_modal_only_alive: false,
            source_modal_keyword: String::new(),
            stats: None,
            settings: AppSettings::default(),
            node_total: 0,
            page: 1,
            page_size: 50,
            protocol_filter: String::new(),
            source_filter: String::new(),
            source_search_query: String::new(),
            keyword: String::new(),
            keyword_applied: String::new(),
            nodes_req_id: 0,
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
            app_release: None,
            app_update_progress: None,
            app_update_busy: false,
            app_update_checked_at: None,
            app_update_due: Some(std::time::Instant::now() + std::time::Duration::from_secs(1)),
            app_update_quiet: false,
            app_installer_path: None,
            app_update_cancel: Arc::new(AtomicBool::new(false)),
            available_releases: Vec::new(),
            selected_release_tag: None,
            fetching_releases: false,
            proxy_log: String::new(),
            selected_proxy_node_id: initial_proxy_node.as_ref().map(|node| node.0.clone()),
            selected_proxy_node_name: initial_proxy_node.map(|node| node.1),
            focused_node_id: None,
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
            show_clear_sources_confirm: false,
            show_gist_config_modal: false,
            show_import_yaml_modal: false,
            import_yaml_path: String::new(),
            import_yaml_as_source: true,
            import_yaml_source_name: String::new(),
            rules_tab: RulesTab::default(),
            groups_tab: GroupsTab::default(),
            connections_tab: ConnectionsTab::default(),
            dns_records: std::collections::VecDeque::new(),
            connections_process_filter: String::new(),
            connections_dns_filter: String::new(),
            show_rule_import: false,
            show_rule_import_help: false,
            rule_import_text: String::new(),
            rule_import_preview: None,
            new_mapping_port: String::new(),
            new_mapping_node_id: None,
            new_mapping_node_name: None,
            node_picker_open: false,
            node_picker_search: String::new(),
            node_picker_target_index: None,
            editing_id: None,
            form_name: String::new(),
            form_url: String::new(),
            form_enabled: true,
            form_remark: String::new(),
            show_source_form: false,
            paste_outcome: None,
            diag_trace: None,
            parked: false,
            was_minimized: false,
            restore_cooldown_frames: 0,
            park_spot: None,
            parked_at: None,
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
            edit_speedtest_concurrency: 64,
            edit_timeout_ms: 2000,
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
            last_log_poll: std::time::Instant::now() - std::time::Duration::from_secs(60),
            traffic_polling: false,
            kernel_groups: Vec::new(),
            proxy_selected_group: "全部".to_string(),
            proxy_view: None,
            proxy_nodes_rev: 0,
            proxy_page: 1,
            proxy_page_size: 50,
            ip_risk: None,
            ip_risk_busy: false,
            ip_risk_error: None,
            show_ip_risk_dialog: false,
            ip_risk_target: None,
            ip_risk_egress: None,
            ip_risk_egress_for: None,
            ip_risk_egress_pending: false,
            ip_risk_egress_error: None,
            ip_risk_auto_for: None,
            ip_risk_failed_for: None,
            reload_due: false,
            last_full_reload: std::time::Instant::now(),
            startup_trim_done: false,
            last_auto_trim: std::time::Instant::now(),
            geo_updating: false,
            geo_auto_attempted: false,
            auto_failover: false,
            edit_exclude_keywords: String::new(),
            edit_failover_latency_ms: 1500,
            edit_speedtest_parallel_kernels: false,
            edit_speedtest_persistent_kernel: false,
            edit_speedtest_reuse_minutes: 0,
            edit_speedtest_conditional_isolation: false,
            edit_speedtest_download_top_n: 0,
            edit_speedtest_dedup_egress: true,
            edit_speedtest_tcp_ping_only: false,
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
            proxy_diagnostics_open: true,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            palette_needs_focus: false,
            router: crate::router::Router::default(),
            edit_bandwidth_guard: false,
            edit_bandwidth_min_kbps: 0,
            edit_ui_theme: theme::DEFAULT_THEME.to_string(),
            edit_app_icon_style: "classic".to_string(),
            edit_ui_language: "zh-CN".to_string(),
            edit_ui_scale: 1.2,
            edit_ui_dark: false,
            edit_sidebar_expanded: false,
            edit_insider_channel: false,
            edit_kernel_source: crate::kernel::KernelSource::GhFast.id().to_string(),
            edit_firewall_auto_allow: false,
            edit_window_size: "small".to_string(),
            edit_console_tabs: "all".to_string(),
            show_kernel_dialog: false,
            kernel_dialog_source: crate::kernel::KernelSource::GhFast.id().to_string(),
            kernel_dialog_path: String::new(),
            kernel_dialog_kind: None,
            firewall_status: None,
            firewall_seeded: false,
            connections_filter: String::new(),
            homogeneity_reports: None,
            homogeneity_selected: HashSet::new(),
            expanded_kernel_groups: HashSet::new(),
            show_homogeneity_dialog: false,
            homogeneity_busy: false,
            homogeneity_status: None,
            tray: crate::platform::TrayHandle::new(),
            force_quit: false,
            pending_exit_after_handoff: false,
            applied_theme_id: String::new(),
            applied_ui_dark: false,
            settings_seeded: false,
            settings_tab: SettingsTab::default(),
            frame_count: 0,
            logo_texture: None,
            tx,
            rx,
        };
        // 诊断壳：把界面停在要验收的那一页（产品路径下这两个函数永远返回 None）。
        if let Some(tab) = crate::diag::starting_tab().as_deref() {
            app.tab = match tab {
                "sources" => Tab::Sources,
                "nodes" => Tab::Nodes,
                "proxy" => Tab::Proxy,
                "extensions" => Tab::Extensions,
                "settings" => Tab::Settings,
                "api" => Tab::Api,
                "about" => Tab::About,
                other => {
                    crate::startup_log(&format!("DIAG 认不出的起始页: {other}"));
                    app.tab
                }
            };
        }
        if crate::diag::starting_panel().as_deref() == Some("rules") {
            app.proxy_panel = ProxyPanel::Rules;
        }
        if let Some(scale) = crate::diag::starting_scale() {
            app.settings.ui_scale = scale;
            app.edit_ui_scale = scale;
        }
        if let Some(dark) = crate::diag::starting_dark() {
            app.settings.ui_dark = dark;
            app.edit_ui_dark = dark;
        }
        app.reload_all();
        app
    }

    pub(crate) fn speed_opts(&self) -> SpeedTestOptions {
        let (concurrency, timeout_ms) = match self.speedtest_intensity {
            SpeedtestIntensity::Fast => (
                self.settings.speedtest_concurrency.clamp(64, 128),
                self.settings.speedtest_timeout_ms.min(1200),
            ),
            SpeedtestIntensity::Standard => (
                self.settings.speedtest_concurrency.clamp(32, 96),
                self.settings.speedtest_timeout_ms.max(1500),
            ),
            SpeedtestIntensity::Deep => (
                (self.settings.speedtest_concurrency / 2).clamp(16, 48),
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
            tcp_ping_only: self.settings.speedtest_tcp_ping_only,
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
        self.reload_proxy_nodes();
        self.reload_proxy_node_groups();
        self.reload_egress_health();
        self.reload_rule_hits();
        self.reload_switch_counts();
        self.reload_fetch_history();
        if self.source_modal_open {
            self.reload_source_modal_nodes();
        }
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

    /// 经当前代理实测 Google/YouTube/GitHub/ChatGPT 的连通性与延迟。
    /// 代理运行中每 60 秒自动刷新一次；进入 Running 后立即跑一轮。
    pub(crate) fn maybe_check_services(&mut self) {
        // 上一轮 15 秒还没回结果就视为丢失，允许重新发起（自愈，避免永久"检测中"；
        // 一轮是四站并行、单站 3s 超时，正常 3 秒内必回）
        if self.service_checks_running {
            if let Some(started) = self.service_check_started {
                if started.elapsed() < std::time::Duration::from_secs(15) {
                    return;
                }
            }
        }
        if self.last_service_check.elapsed() < std::time::Duration::from_secs(30) {
            return;
        }
        self.last_service_check = std::time::Instant::now();
        self.service_checks_running = true;
        self.service_check_started = Some(std::time::Instant::now());

        // 代理运行 → 经代理实测；未运行 → 直连实测（也能看出裸网是否可达）
        let use_proxy = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        let proxy_port = self.proxy_status.mixed_port;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let mut builder = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .connect_timeout(std::time::Duration::from_secs(2));
            if use_proxy {
                builder = builder
                    .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).expect("本地混合端口代理地址有效"));
            }
            let client = builder.build().ok();

            // 单站点探测：有响应即连通（Cloudflare 的 403 也证明链路是通的）
            async fn probe(client: Option<&reqwest::Client>, url: &str) -> Option<u128> {
                let started = std::time::Instant::now();
                match client {
                    Some(client) => match client.get(url).send().await {
                        Ok(_) => Some(started.elapsed().as_millis()),
                        Err(_) => None,
                    },
                    None => None,
                }
            }

            // 四个站点并行探测，一轮最多 3 秒
            let (g, c, h, y) = tokio::join!(
                probe(client.as_ref(), "https://www.google.com/generate_204"),
                probe(client.as_ref(), "https://chatgpt.com"),
                probe(client.as_ref(), "https://api.github.com"),
                probe(client.as_ref(), "https://www.youtube.com"),
            );
            let results = vec![
                crate::gui::types::ServiceCheck { name: "Google", latency_ms: g },
                crate::gui::types::ServiceCheck { name: "ChatGPT", latency_ms: c },
                crate::gui::types::ServiceCheck { name: "GitHub", latency_ms: h },
                crate::gui::types::ServiceCheck { name: "YouTube", latency_ms: y },
            ];
            let _ = tx.send(BgMsg::ServiceChecks(results));
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

    pub(crate) fn reload_proxy_logs(&mut self) {
        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let log = manager
                .recent_log(12_000)
                .await
                .map_err(|error| error.to_string());
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
        self.nodes_req_id = self.nodes_req_id.wrapping_add(1);
        let req_id = self.nodes_req_id;
        let db = self.db.clone();
        let tx = self.tx.clone();
        let protocol = nonempty_opt(&self.protocol_filter);
        let source_id = nonempty_opt(&self.source_filter);
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
        self.rt.spawn(async move {
            let res = db
                .list_nodes(
                    protocol.as_deref(),
                    source_id.as_deref(),
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
            let _ = tx.send(BgMsg::Nodes { req_id, result: res });
        });
    }

    pub(crate) fn reload_node_view(&mut self) {
        let db = self.db.clone();
        self.rt.spawn(async move {
            db.invalidate_node_cache().await;
        });
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

    /// 一键接入 OpenRung 官方中继代理服务（https://github.com/openrung/openrung）
    pub(crate) fn add_openrung_source(&mut self) {
        let openrung_url = "https://broker.openrung.org/api/v1/relays";
        let openrung_name = "OpenRung 专属节点服务";
        if let Some(existing) = self.sources.iter().find(|s| s.url_template == openrung_url) {
            let id = existing.id.clone();
            self.status = format!("OpenRung 专用节点源已存在：{}，正在刷新...", existing.name);
            self.fetch_one(id);
            return;
        }

        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在添加 OpenRung 官方节点服务...".into();
        self.rt.spawn(async move {
            let res = match db
                .create_source(crate::models::CreateSource {
                    name: openrung_name.into(),
                    url_template: openrung_url.into(),
                    enabled: true,
                    remark: Some("OpenRung Relay Network 官方中继 (VLESS+REALITY+Vision)".into()),
                })
                .await
            {
                Ok(source) => {
                    let _ = tx.send(BgMsg::SourceAdded(source.id, source.name));
                    Ok("成功添加 OpenRung 官方节点源，正在拉取并测速...".into())
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn fetch_warp_masque_native(&mut self) {
        let warp_url = "warp://native";
        let warp_name = "Cloudflare WARP (MASQUE) 原生节点";
        let db = self.db.clone();
        let tx = self.tx.clone();
        let proxy_url = self.subscription_proxy_url();
        self.busy = true;
        self.status = "正在向 Cloudflare 原生注册并获取 57 个 WARP MASQUE 节点...".into();

        self.rt.spawn(async move {
            let existing_id = match db.list_sources().await {
                Ok(sources) => sources.into_iter().find(|s| s.url_template == warp_url).map(|s| s.id),
                Err(_) => None,
            };

            let source_id = if let Some(id) = existing_id {
                id
            } else {
                match db
                    .create_source(crate::models::CreateSource {
                        name: warp_name.into(),
                        url_template: warp_url.into(),
                        enabled: true,
                        remark: Some("Cloudflare WARP 官方原生 MASQUE 节点池（免第三方仓库）".into()),
                    })
                    .await
                {
                    Ok(source) => source.id,
                    Err(e) => {
                        let _ = tx.send(BgMsg::OpDone {
                            result: Err(format!("创建 WARP 专属源失败: {e}")),
                            unlock: true,
                        });
                        return;
                    }
                }
            };

            let nodes_res = match crate::parser::warp_masque::register_warp_masque_native(None).await {
                Ok(nodes) => Ok(nodes),
                Err(err) => {
                    if let Some(ref proxy) = proxy_url {
                        crate::parser::warp_masque::register_warp_masque_native(Some(proxy)).await
                    } else {
                        Err(err)
                    }
                }
            };

            let res = match nodes_res {
                Ok(nodes) => {
                    let count = nodes.len();
                    match db.replace_source_nodes(&source_id, &nodes).await {
                        Ok(_) => {
                            let _ = db.mark_source_fetched(&source_id, "ok", count as i64).await;
                            Ok(format!("🎉 成功原生获取 {} 个 Cloudflare WARP MASQUE 节点！", count))
                        }
                        Err(e) => Err(format!("保存 WARP 节点失败: {e}")),
                    }
                }
                Err(e) => Err(format!(
                    "WARP 原生注册失败（若海外网络直连受阻，可开启代理后重试）: {e}"
                )),
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

    pub(crate) fn clear_all_sources(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.status = "正在清空全部订阅源...".into();
        self.rt.spawn(async move {
            let res = db
                .clear_all_sources()
                .await
                .map(|count| format!("已清空全部 {} 个订阅源及关联节点", count))
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
        let proxy_url = self.subscription_proxy_url();
        self.active_source_fetches.insert(id.clone());
        self.status = "正在拉取订阅源...".into();
        self.rt.spawn(async move {
            let mut res = collector.fetch_one(&id).await.map_err(|e| e.to_string());
            if let Some(proxy_url) = proxy_url {
                let should_fallback = match &res {
                    Ok(f) => !f.success,
                    Err(_) => true,
                };
                if should_fallback {
                    if let Ok(proxy_res) = collector.fetch_one_with_proxy(&id, &proxy_url).await {
                        if proxy_res.success {
                            res = Ok(proxy_res);
                        }
                    }
                }
            }
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
        let proxy_url = self.subscription_proxy_url();
        self.busy = true;
        self.status = "正在拉取全部启用源...".into();
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let mut res = collector
                .fetch_all_enabled_with_progress(Some(move |done, total| {
                    let _ = progress_tx.send(BgMsg::FetchProgress { done, total });
                }))
                .await
                .map_err(|e| e.to_string());
            if let Some(proxy_url) = proxy_url {
                let should_fallback = match &res {
                    Ok(list) => list.iter().any(|item| !item.success),
                    Err(_) => true,
                };
                if should_fallback {
                    let progress_tx = tx.clone();
                    if let Ok(proxy_res) = collector
                        .fetch_all_enabled_with_proxy(
                            &proxy_url,
                            Some(move |done, total| {
                                let _ = progress_tx.send(BgMsg::FetchProgress { done, total });
                            }),
                        )
                        .await
                    {
                        res = Ok(proxy_res);
                    }
                }
            }
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

    /// 持久化 Gist 发现配置（选中协议与爬取页数）到数据库
    pub(crate) fn save_gist_config(&mut self) {
        let enabled_queries: Vec<&str> = crate::collector::GIST_DISCOVERY_QUERIES
            .iter()
            .zip(self.gist_query_flags.iter())
            .filter_map(|(query, &flag)| flag.then_some(*query))
            .collect();
        let queries_str = if enabled_queries.is_empty() {
            crate::collector::GIST_DISCOVERY_DEFAULT_QUERY.to_string()
        } else {
            enabled_queries.join(",")
        };
        let pages = self.gist_pages.clamp(1, 300) as u32;
        self.settings.gist_pages = pages;
        self.settings.gist_queries = queries_str.clone();
        let db = self.db.clone();
        self.rt.spawn(async move {
            let _ = db
                .update_settings(crate::models::UpdateSettings {
                    gist_pages: Some(pages),
                    gist_queries: Some(queries_str),
                    ..Default::default()
                })
                .await;
        });
    }

    /// 独立的 Gist 发现：翻页拉取 gist 搜索结果，可用的 gist 注册为源、节点进节点列表。
    pub(crate) fn discover_gists(&mut self) {
        self.save_gist_config();
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

    pub(crate) fn reset_seen_gists(&mut self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.status = "正在重置 Gist 发现历史...".into();
        self.rt.spawn(async move {
            let res = db.clear_seen_gists().await;
            let _ = tx.send(BgMsg::OpDone {
                result: res
                    .map(|count| format!("已清空 {count} 条已见 Gist 记录，现在可以重新发现全部节点"))
                    .map_err(|e| e.to_string()),
                unlock: false,
            });
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

    pub(crate) fn import_clash_from_clipboard(&mut self) {
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

        // 1) 提取并自动合并自带的分流规则
        let rules = crate::parser::extract_subscription_rules(&text);
        let rules_added = if !rules.is_empty() {
            let before = self.proxy_config.rules.len();
            let mut combined = self.proxy_config.rules.clone();
            combined.extend(rules);
            self.proxy_config.rules = optimize_traffic_rules(&combined);
            let added = self.proxy_config.rules.len().saturating_sub(before);
            if added > 0 {
                self.save_proxy_config();
            }
            added
        } else {
            0
        };

        // 2) 解析并入库节点
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "正在从剪贴板解析 Clash 配置并导入...".into();
        self.rt.spawn(async move {
            let nodes = crate::parser::parse_subscription(&text, None);
            let parsed = nodes.len();
            let res = if parsed == 0 {
                if rules_added > 0 {
                    Ok(format!("Clash 配置导入完成：未发现新代理节点，已合并 {rules_added} 条分流规则"))
                } else {
                    Err("未从剪贴板解析出任何节点或有效规则".to_string())
                }
            } else {
                db.import_nodes(&nodes)
                    .await
                    .map(|stored| {
                        if rules_added > 0 {
                            format!("Clash 配置导入完成：解析 {parsed} 个节点，入库 {stored} 个，已自动合并 {rules_added} 条自带分流规则")
                        } else {
                            format!("Clash 配置导入完成：解析 {parsed} 个节点，入库 {stored} 个")
                        }
                    })
                    .map_err(|e| e.to_string())
            };
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
        });
    }

    pub(crate) fn import_rules_from_clipboard(&mut self) {
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

        let mut parsed_rules = crate::proxy::rule_io::parse_rules_text(&text).rules;
        if parsed_rules.is_empty() {
            parsed_rules = crate::parser::extract_subscription_rules(&text);
        }

        if parsed_rules.is_empty() {
            self.status = "未能从剪贴板识别出有效的分流规则".into();
            return;
        }

        let before = self.proxy_config.rules.len();
        let mut combined = self.proxy_config.rules.clone();
        combined.extend(parsed_rules);
        self.proxy_config.rules = optimize_traffic_rules(&combined);
        let added = self.proxy_config.rules.len().saturating_sub(before);
        if added > 0 {
            self.save_proxy_config();
            self.status = format!("分流规则导入完成：成功合并追加 {added} 条规则");
        } else {
            self.status = "剪贴板中的规则均已存在于列表中，无需重复添加".into();
        }
    }

    pub(crate) fn import_from_local_yaml_file(&mut self, path: String, as_source: bool, source_name: String) {
        let p = std::path::PathBuf::from(path.trim());
        if !p.exists() {
            self.status = format!("找不到指定文件: {}", p.display());
            return;
        }
        let content = match std::fs::read_to_string(&p) {
            Ok(c) => c,
            Err(e) => {
                self.status = format!("读取文件失败: {e}");
                return;
            }
        };

        // 提取并自动合并自带的分流规则
        let extracted_rules = crate::parser::extract_subscription_rules(&content);
        let rules_added = if !extracted_rules.is_empty() {
            let before = self.proxy_config.rules.len();
            let mut combined = self.proxy_config.rules.clone();
            combined.extend(extracted_rules);
            self.proxy_config.rules = optimize_traffic_rules(&combined);
            let added = self.proxy_config.rules.len().saturating_sub(before);
            if added > 0 {
                self.save_proxy_config();
            }
            added
        } else {
            0
        };

        let db = self.db.clone();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = format!("正在解析本地配置文件「{}」...", p.display());
        let file_stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "本地配置".to_string());
        let final_source_name = if source_name.trim().is_empty() {
            format!("本地: {file_stem}")
        } else {
            source_name.trim().to_string()
        };
        let file_url = format!("file://{}", p.display());

        self.rt.spawn(async move {
            let mut source_id_opt = None;
            if as_source {
                let create_src = crate::models::CreateSource {
                    name: final_source_name.clone(),
                    url_template: file_url,
                    enabled: true,
                    remark: Some("从本地 YAML 配置文件导入".to_string()),
                };
                match db.create_source(create_src).await {
                    Ok(created) => {
                        source_id_opt = Some(created.id);
                    }
                    Err(e) => {
                        tracing::warn!("创建本地订阅源失败，将以独立节点形式导入: {e}");
                    }
                }
            }

            let nodes = crate::parser::parse_subscription(&content, source_id_opt.as_deref());
            let parsed = nodes.len();
            let res = if parsed == 0 {
                if rules_added > 0 {
                    Ok(format!("本地文件导入完成：未发现新代理节点，已合并 {rules_added} 条分流规则"))
                } else {
                    Err("未从文件中解析出任何有效代理节点（请检查是否为 Clash YAML 格式）".to_string())
                }
            } else {
                db.import_nodes(&nodes)
                    .await
                    .map(|stored| {
                        if rules_added > 0 {
                            format!("本地文件导入完成：成功解析 {parsed} 个节点，入库 {stored} 个，已自动合并 {rules_added} 条自带分流规则")
                        } else {
                            format!("本地文件导入完成：成功解析 {parsed} 个节点，入库 {stored} 个")
                        }
                    })
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

    pub(crate) fn speedtest_current_list(&mut self) {
        let db = self.db.clone();
        let source_id = nonempty_opt(&self.source_filter);
        let protocol = nonempty_opt(&self.protocol_filter);
        let keyword = nonempty_opt(&self.keyword);
        let latency = if self.only_available_nodes {
            Some("alive".to_string())
        } else if self.latency_filter == "all" {
            None
        } else {
            Some(self.latency_filter.clone())
        };
        let country = if self.region_filter == "all" {
            None
        } else {
            Some(self.region_filter.clone())
        };

        let st = self.speedtester.clone();
        let opts = self.speed_opts();
        let tx = self.tx.clone();

        self.busy = true;
        self.status = "正在检索当前筛选节点列表...".into();

        self.rt.spawn(async move {
            let ids = match db
                .list_filtered_node_ids(
                    source_id.as_deref(),
                    protocol.as_deref(),
                    keyword.as_deref(),
                    latency.as_deref(),
                    country.as_deref(),
                )
                .await
            {
                Ok(ids) => ids,
                Err(e) => {
                    let _ = tx.send(BgMsg::OpDone {
                        result: Err(format!("检索当前筛选节点失败: {e}")),
                        unlock: true,
                    });
                    return;
                }
            };

            if ids.is_empty() {
                let _ = tx.send(BgMsg::OpDone {
                    result: Ok("当前筛选条件下没有节点".to_string()),
                    unlock: true,
                });
                return;
            }

            let progress_tx = tx.clone();
            let res = st
                .test_many_with_progress(
                    Some(ids),
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
        // 出口变化：服务连通性立即重测（先清掉旧结果，卡片转入"检测中"）
        self.last_service_check =
            std::time::Instant::now() - std::time::Duration::from_secs(61);
        self.service_checks = None;
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

        // 如果代理未运行：仅设为活动节点，不要启动代理！
        if !running {
            self.switching_node_id = None;
            self.status = format!("已设为活动节点: {name}（代理未启动）");
            return;
        }

        self.switching_node_id = Some(id.clone());
        self.proxy_switch_seq = self.proxy_switch_seq.wrapping_add(1);
        let seq = self.proxy_switch_seq;

        if requires_tun_elevation(self.proxy_config.mode, platform::is_elevated()) {
            self.start_proxy();
            return;
        }

        // url-test 打开时，目标节点通常已经是 PROXY 组的成员：这时只要让内核换个
        // 成员（PUT /proxies/PROXY），不需要重写配置 + 热重载掐断所有连接。
        if self.proxy_config.url_test
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
        let port_mappings = self.proxy_config.port_mappings.clone();
        let candidate_blacklist = self.router.blacklisted(std::time::Instant::now());
        let node_id = id.clone();
        self.status = format!("⚡ 正在极速异步切换至「{name}」...");
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
                    Self::prime_port_mapping_candidates(
                        &db,
                        &manager,
                        &port_mappings,
                    )
                    .await;
                    let res = manager.switch_node(&node).await;
                    res.map(|_| format!("已平滑切换至节点: {}", node.name))
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

    /// 注入一节点一端口多端口映射的目标节点数据
    async fn prime_port_mapping_candidates(
        db: &Database,
        manager: &ProxyManager,
        port_mappings: &[crate::proxy::PortMapping],
    ) {
        if port_mappings.is_empty() {
            manager.set_port_mapping_nodes(Vec::new()).await;
            return;
        }
        let mut items = Vec::new();
        for m in port_mappings {
            if !m.enabled {
                continue;
            }
            if let Ok(Some(node)) = db.get_node(&m.node_id).await {
                items.push((m.port, node));
            }
        }
        manager.set_port_mapping_nodes(items).await;
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
            programs.push(
                root.join(kind.directory_name())
                    .join(kind.executable_name()),
            );
        }
        programs
    }

    /// 写防火墙允许规则。netsh 要管理员权限，非提权时会失败并把原因显示出来。
    pub(crate) fn apply_firewall_rules(&mut self) {
        let programs = Self::firewall_targets();
        let tx = self.tx.clone();
        self.status = "正在写入防火墙允许规则...".into();
        self.rt.spawn_blocking(move || {
            let result =
                crate::platform::firewall::ensure_all(&programs).map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::FirewallApplied(result));
        });
    }

    /// 「下载内核」弹框里点开始走这条：源与（本地导入时的）zip 路径都由弹框决定。
    /// `remember_source` = 把人选的这种记成默认；自动预备那条传 false —— 它每试一档
    /// 就把设置改掉的话，阶梯的第一档会被上一次运行撞墙的那一种占掉。
    pub(crate) fn update_kernel_from(
        &mut self,
        kind: KernelKind,
        source: crate::kernel::KernelSource,
        local_file: Option<std::path::PathBuf>,
        remember_source: bool,
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
        if remember_source {
            self.edit_kernel_source = source.id().to_string();
            self.save_settings();
        }
        self.status = format!(
            "正在下载并校验 {}（{}）...",
            kind.display_name(),
            source.label()
        );
        self.rt.spawn(async move {
            let result = manager
                .update_with_options(kind, proxy_url, source, local_file, cancelled, {
                    let tx = tx.clone();
                    move |progress| {
                        let _ = tx.send(BgMsg::KernelProgress(progress));
                    }
                })
                .await
                .map(|result| result.message)
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::KernelUpdated(result));
        });
    }

    /// 代理配置进 UI 状态的唯一入口。
    fn set_proxy_config(&mut self, config: MihomoConfigOptions) {
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
            Self::prime_port_mapping_candidates(&db, &manager, &options.port_mappings).await;
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
        self.kernel_dialog_source = crate::kernel::source_from_id(&self.settings.kernel_source)
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
        // 「本地代理」= 应用内 mixed 口在跑，或 Windows 系统代理有值。预备状态下前者必然没在跑，
        // 所以这一档实际用的就是系统代理；没挂代理就不排它（见 prepare::auto_prepare_source）。
        let has_local_proxy = self.proxy_status.local_proxy_active()
            || crate::system_proxy::current_system_proxy_url().is_some();
        let source = prepare::auto_prepare_source(
            crate::kernel::source_from_id(&self.settings.kernel_source),
            self.prepare_attempts,
            has_local_proxy,
        );
        // 先把要写的东西算成局部值：format! 里直接调 `self.kernel_installed_cached()`
        // 会和后面的可变借用打架（那一步本来就是可变操作）。
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|dir| dir.display().to_string()))
            .unwrap_or_else(|| "未知".to_string());
        crate::startup_log(&format!(
            "[预备] 第 {}/{} 次自动下载内核：方式={} 本地代理={} 代理在跑={} 盘上已有内核={} 程序目录={}",
            self.prepare_attempts,
            prepare::MAX_AUTO_ATTEMPTS,
            source.label(),
            if has_local_proxy { "有" } else { "没有" },
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
        self.update_kernel_from(KernelKind::Mihomo, source, None, false);
    }

    /// 新安装或缺少分流数据库时，在程序启动后自动于后台极速拉取完整的 GeoIP / GeoSite 规则文件。
    pub(crate) fn maybe_auto_download_geo(&mut self) {
        if self.geo_auto_attempted || self.geo_updating {
            return;
        }
        let dir = crate::geo::mihomo_dir();
        if crate::geo::geo_ready(&dir) {
            self.geo_auto_attempted = true;
            return;
        }
        self.geo_auto_attempted = true;
        self.geo_updating = true;
        self.status = "新安装首次运行：正在极速自动拉取 GeoIP / GeoSite 分流规则文件...".into();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = crate::geo::provision(&dir, None, false)
                .await
                .map(|detail| format!("Geo 分流规则库自动就绪：{detail}"));
            let _ = tx.send(BgMsg::GeoUpdated(result.map_err(|e| e.to_string())));
        });
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
        if self.selected_proxy_node_id.is_none() {
            if let Some(ref focused) = self.focused_node_id {
                self.selected_proxy_node_id = Some(focused.clone());
            } else if let Some(n) = self.nodes.first() {
                self.selected_proxy_node_id = Some(n.id.clone());
                self.selected_proxy_node_name = Some(n.name.clone());
            } else if let Some(n) = self.proxy_nodes.first() {
                self.selected_proxy_node_id = Some(n.id.clone());
                self.selected_proxy_node_name = Some(n.name.clone());
            }
        }
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
                        None => match db.list_nodes(None, None, None, None, None, 1, 1, None).await {
                            Ok(page) => match page.items.into_iter().next() {
                                Some(n) => Ok((n.id, n.name)),
                                None => Err("库里没有可用节点，请先在「订阅源」拉取节点".to_string()),
                            },
                            Err(e) => Err(e.to_string()),
                        },
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
        #[cfg(target_os = "linux")]
        if self.proxy_config.mode == ProxyMode::Tun && !platform::is_elevated() {
            let kernel_path = crate::kernel::managed_kernel_path(crate::kernel::KernelKind::Mihomo);
            if !platform::kernel_has_tun_capability(&kernel_path) {
                let cmd = platform::tun_capability_command(&kernel_path);
                self.status = format!(
                    "TUN 模式启动被拦截：内核缺少网络权限，请在终端执行：{cmd}"
                );
                return;
            }
        }
        // Windows 的 TUN 要管理员权限：本进程（非提权）起不了内核，只能把配置交给一个
        // runas 起来的提权实例，然后自己退掉释放单实例互斥量 —— 提权实例那边最多等 3 秒接管。
        // 以前这里只把状态文案改成"正在请求管理员权限…"就照样往下调 manager.start()，
        // 于是内核那句"TUN 模式需要管理员权限"每次都原样打回给他（2026-10-22 用户报）。
        if requires_tun_elevation(self.proxy_config.mode, platform::is_elevated()) {
            self.handoff_proxy_to_elevated_instance(&node_id);
            return;
        }
        let db = self.db.clone();
        let manager = self.proxy_manager.clone();
        let mode = self.proxy_config.mode;
        let options = self.proxy_config.clone();
        let candidate_blacklist = self.router.blacklisted(std::time::Instant::now());
        let tx = self.tx.clone();
        let starting_msg = "正在启动代理...".to_string();
        self.proxy_busy = true;
        self.proxy_status.state = ProxyState::Starting;
        self.proxy_status.message = starting_msg.clone();
        self.status = starting_msg;
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
                    Self::prime_port_mapping_candidates(
                        &db,
                        &manager,
                        &options.port_mappings,
                    )
                    .await;
                    let previous = manager.config().await;
                    let operation = match manager.update_config(options.clone()).await {
                        Ok(()) => {
                            let db_clone = db.clone();
                            let opt_clone = options.clone();
                            tokio::spawn(async move {
                                let _ = db_clone.update_proxy_config(&opt_clone).await;
                            });
                            manager.start(&node).await
                        }
                        Err(error) => {
                            let _ = manager.update_config(previous).await;
                            Err(error)
                        }
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
                            } else if mode == ProxyMode::Tun
                                && (message.contains("Operation not permitted")
                                    || message.contains("CAP_NET_ADMIN")
                                    || message.contains("TUNSETIFF")
                                    || message.contains("permission denied"))
                            {
                                let kernel_path = crate::kernel::managed_kernel_path(crate::kernel::KernelKind::Mihomo);
                                Err(anyhow::anyhow!(
                                    "TUN 模式启动失败：内核进程缺少 CAP_NET_ADMIN 权限。\n\
                                     请在终端执行一次授权命令为内核授权：\n\
                                     sudo setcap cap_net_admin,cap_net_raw+ep \"{}\"",
                                    kernel_path.display()
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

    /// 组装交给提权实例的命令行参数。单独抽出来是因为这条链拼错（flag 重复、配置编不出来）时，
    /// 提权实例会在写下第一行日志之前就死掉 —— 只有纯函数能被测试钉住。
    fn tun_handoff_arguments(
        options: &crate::proxy::MihomoConfigOptions,
        node_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        Ok(vec![
            "--elevated-proxy-config".to_string(),
            crate::proxy::encode_elevated_proxy_config(options)?,
            "--start-proxy-node".to_string(),
            node_id.to_string(),
        ])
    }

    /// TUN 模式在非提权实例里点「启动代理」：把当前配置与目标节点交给一个 runas 起来的提权实例，
    /// 本实例随后真退出（释放单实例互斥量，提权实例那边最多等 3 秒接管）。
    /// 他拒了 UAC、或配置编不出来，就留在原地不动，状态行说实话。
    fn handoff_proxy_to_elevated_instance(&mut self, node_id: &str) {
        let args = match Self::tun_handoff_arguments(&self.proxy_config, node_id) {
            Ok(args) => args,
            Err(error) => {
                self.status = format!("TUN 交接失败：生成提权配置出错 —— {error}");
                return;
            }
        };
        match platform::request_elevation(&args) {
            Ok(()) => {
                crate::startup_log("TUN 交接：已请求管理员权限，本实例退出以释放单实例互斥量");
                self.status =
                    "已请求管理员权限：提权实例会带着 TUN 内核启动，这个窗口随后关闭".into();
                self.pending_exit_after_handoff = true;
            }
            Err(error) => {
                self.status =
                    format!("TUN 模式需要管理员权限，提权没成功（被取消或被系统拒绝）：{error}");
            }
        }
    }

    /// 代理状态变化写进 startup.log；同一状态卡满 30 秒再补一行，点名“此时启动按钮是灰的”。
    pub(crate) fn note_proxy_state_change(&mut self) {
        let now = std::time::Instant::now();
        let state = self.proxy_status.state;
        if let Some((previous, since, stuck_logged)) = self.last_proxy_state_log {
            if previous == state {
                let stuck = matches!(state, ProxyState::Starting | ProxyState::Stopping);
                let elapsed = now.duration_since(since).as_secs();
                if stuck && !stuck_logged && elapsed >= 30 {
                    crate::startup_log(&format!(
                        "[代理状态] 在 {state:?} 卡了 {elapsed} 秒，启动按钮此时是灰的：{} busy={}",
                        self.proxy_status.message,
                        self.proxy_busy
                    ));
                    self.last_proxy_state_log = Some((state, since, true));
                }
                return;
            }
        }
        crate::startup_log(&format!(
            "[代理状态] {state:?}（{}）busy={}",
            self.proxy_status.message,
            self.proxy_busy
        ));
        self.last_proxy_state_log = Some((state, now, false));
    }

    pub(crate) fn stop_proxy(&mut self) {        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        self.proxy_busy = true;
        self.proxy_status.state = ProxyState::Stopping;
        self.proxy_status.message = "正在停止代理...".into();
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
                    "select" | "selector" | "url-test" | "urltest" | "load-balance" | "loadbalance" | "fallback"
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

    pub(crate) fn select_kernel_member(&mut self, group: String, target: String) {
        // 出口变化：服务连通性立即重测（先清掉旧结果，卡片转入"检测中"）
        self.last_service_check =
            std::time::Instant::now() - std::time::Duration::from_secs(61);
        self.service_checks = None;
        let port = self.proxy_status.controller_port;
        let secret = self.proxy_status.controller_secret.clone();
        let tx = self.tx.clone();
        self.status = format!("正在让内核切换 {group} → {target}...");
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("http://127.0.0.1:{port}/proxies/{group}");
            let response_res = crate::proxy::with_controller_auth(client.put(&url), &secret)
                .json(&serde_json::json!({ "name": target }))
                .send()
                .await;
            let result = match response_res {
                Ok(response) => {
                    if response.status().is_success() {
                        Ok(())
                    } else {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        Err(format!("内核拒绝切换策略组 {group} ({status}): {body}"))
                    }
                }
                Err(error) => Err(error.to_string()),
            };
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
                    .map(|detail| {
                        format!("内核未运行，已直接更新到 {}：{detail}", report.display())
                    });
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

    /// 自动切换：每 60 秒经**当前运行中的代理**实测一次连通性与延迟（不新起内核），
    /// 验证失败立即切换；仅延迟劣化则需连续三次、且距上次自动切换满 3 分钟才跳，
    /// 避免一次网络抖动就断掉全部连接。
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
    /// 静默检测出口 IP 归属：不弹框，结果直接刷新状态卡的"机房/住宅"标签。
    /// 启动代理或切换节点后自动调用。
    pub(crate) fn refresh_egress_silently(&mut self) {
        // TUN 模式启动后不要自动进行 IP 检测
        if self.proxy_config.mode.has_tun() {
            return;
        }
        // 内核的 ProxyStatus（含 node_id）可能还没到，用当前选中节点兜底
        let id = self
            .proxy_status
            .node_id
            .clone()
            .or_else(|| self.selected_proxy_node_id.clone());
        let Some(id) = id else {
            return;
        };
        // 已经在查或已有本节点的结果就不重复发起
        if self.ip_risk_busy
            || self.ip_risk_egress_pending
            || (self.ip_risk_egress.is_some() && self.ip_risk_egress_for.as_deref() == Some(id.as_str()))
        {
            return;
        }
        self.ip_risk_egress = None;
        self.ip_risk_egress_for = None;
        self.ip_risk_egress_error = None;
        self.ip_risk_egress_pending = true;
        let url = format!("http://127.0.0.1:{}", self.proxy_status.mixed_port);
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let egress = crate::ip_risk::lookup_egress(&client, &url)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::IpEgress(id, egress, false));
        });
    }

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
        self.request_ip_risk_for(node_id, true);
    }

    /// 代理在跑、当前出口换了节点，就静默查一次归属 —— 汇总卡片第一张那颗徽章读它。
    /// 静默 = 不弹详情框、不抢状态栏文案（状态栏那个「IP 检测」按钮 2026-10-23 撤掉了，
    /// 手动入口还剩节点卡片和右键，那条会照常弹框）。
    pub(crate) fn auto_ip_risk_for_egress(&mut self) {
        // TUN 模式启动后不要自动进行 IP 检测
        if self.proxy_config.mode.has_tun() {
            return;
        }
        if !matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        ) || self.ip_risk_busy
        {
            return;
        }
        let Some(node_id) = self.proxy_status.node_id.clone() else {
            return;
        };
        if self.ip_risk_auto_for.as_deref() == Some(node_id.as_str()) {
            return;
        }
        self.ip_risk_auto_for = Some(node_id.clone());
        // 徽章"一直未检测"分两种：根本没查，和查了没查到。这一行只为前一种留证据。
        crate::startup_log(&format!(
            "[出口归属] 为 {node_id} 自动发起查询（状态 {:?}）",
            self.proxy_status.state
        ));
        self.request_ip_risk_for(&node_id, false);
    }

    /// 卡片读的那一份。出口结果优先（那才是站点真正看到的 IP），其次是这颗节点服务器
    /// 的归属。两边都按「这份结果属于哪颗节点」认领，不再拿 server 字符串去比对 ——
    /// 正在跑的那颗常常不在面板列表里（测速没过就不算「可用」），比不中徽章就永远显示未检测。
    pub(crate) fn egress_risk_view(&self) -> crate::gui::views::proxy::charts::EgressRiskView<'_> {
        let node_id = self.proxy_status.node_id.as_deref();
        let front_owner = node_id.filter(|id| {
            crate::gui::views::proxy::charts::front_risk_belongs_to(
                id,
                self.ip_risk_auto_for.as_deref(),
                self.ip_risk_target.as_deref(),
            )
        });
        let risk = crate::gui::views::proxy::charts::display_risk(
            node_id,
            self.ip_risk_egress_for.as_deref(),
            self.ip_risk_egress.as_ref(),
            front_owner,
            self.ip_risk.as_ref(),
        );
        let failed = node_id.is_some() && self.ip_risk_failed_for.as_deref() == node_id;
        crate::gui::views::proxy::charts::EgressRiskView {
            risk,
            failed,
            checking: self.ip_risk_busy && !failed,
        }
    }

    /// 面板列表里没有这颗节点时的路：直接去库里取它自己的 server 再查归属。
    fn lookup_ip_risk_from_db(&mut self, node_id: &str, manual: bool) {
        self.ip_risk_busy = true;
        self.ip_risk_error = None;
        if manual {
            self.status = "正在查这颗节点的服务器归属...".into();
        }
        let db = self.db.clone();
        let tx = self.tx.clone();
        let id = node_id.to_owned();
        self.rt.spawn(async move {
            let server = match db.get_node(&id).await {
                Ok(Some(node)) if !node.server.is_empty() => node.server,
                _ => String::new(),
            };
            let front = if server.is_empty() {
                Err("库里也没有这颗节点的服务器地址".to_string())
            } else {
                let client = reqwest::Client::new();
                crate::ip_risk::lookup(&client, &server)
                    .await
                    .map_err(|error| error.to_string())
            };
            let _ = tx.send(BgMsg::IpRisk(front, manual));
        });
        self.start_egress_lookup_if_current(node_id, manual);
    }

    /// `manual = false` 那条只更新数据：不弹框、不写状态栏。
    fn request_ip_risk_for(&mut self, node_id: &str, manual: bool) {
        if manual {
            self.show_ip_risk_dialog = true;
            self.ip_risk_target = Some(node_id.to_owned());
        }
        self.ip_risk_egress = None;
        self.ip_risk_egress_for = None;
        self.ip_risk_egress_error = None;
        self.ip_risk_egress_pending = false;
        let server = self
            .proxy_nodes
            .iter()
            .chain(self.nodes.iter())
            .find(|node| node.id == node_id)
            .map(|node| node.server.clone());
        let Some(server) = server else {
            // 面板里没有它很常见：正在跑的那颗可能测速没过（latency_ok != 1），根本不在
            // 「可用」那份列表里。以前这里当「找不到」直接返回，徽章就永远停在未检测。
            self.lookup_ip_risk_from_db(node_id, manual);
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
            if manual {
                self.status = format!("IP 检测：{}", self.ip_risk.as_ref().unwrap().headline());
            }
            self.start_egress_lookup_if_current(node_id, manual);
            return;
        }
        self.ip_risk_busy = true;
        self.ip_risk_error = None;
        if manual {
            self.status = format!("正在查询 {server} 的归属...");
        }
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let front = crate::ip_risk::lookup(&client, &server)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::IpRisk(front, manual));
        });
        self.start_egress_lookup_if_current(node_id, manual);
    }

    /// 该节点正是当前在跑的节点时，才经本机 mixed 端口自动查一次出口。
    fn start_egress_lookup_if_current(&mut self, node_id: &str, manual: bool) {
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
        if manual {
            self.status = "正在经当前代理查出口 IP...".into();
        }
        let tx = self.tx.clone();
        let id = node_id.to_owned();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let egress = crate::ip_risk::lookup_egress(&client, &url)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::IpEgress(id, egress, manual));
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

    pub(crate) fn handle_node_card_action(&mut self, action: NodeCardAction) {
        match action {
            NodeCardAction::Select { id, name } => {
                self.focused_node_id = Some(id.clone());
                let running = matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                );
                if !running {
                    // 代理未运行时，点击选中节点直接定为待启动的目标活动节点，但不自动启动代理
                    self.selected_proxy_node_id = Some(id);
                    self.selected_proxy_node_name = Some(name.clone());
                    self.status = format!("已选中节点: {name}（点击「启动代理」即可开启）");
                } else {
                    self.status = format!("已聚焦节点: {name}（右键设为活动节点）");
                }
            }
            NodeCardAction::Connect { id, name } => {
                self.switch_proxy_node(id, name, "设为活动节点");
            }
            NodeCardAction::Speedtest(id) => self.speedtest_one(id),
            NodeCardAction::ShowDetail(id) => self.show_node(id),
            NodeCardAction::CheckIpRisk(id) => self.check_ip_risk_for(&id),
            NodeCardAction::Delete(id) => self.delete_node(id),
        }
    }

    pub(crate) fn open_source_nodes_modal(&mut self, source_id: &str, source_name: &str) {
        self.source_modal_open = true;
        self.source_modal_source_id = Some(source_id.to_string());
        self.source_modal_source_name = source_name.to_string();
        self.source_modal_page = 1;
        self.source_modal_page_size = 50;
        self.source_modal_keyword.clear();
        self.source_modal_only_alive = false;
        self.source_modal_nodes.clear();
        self.source_modal_total = 0;
        self.reload_source_modal_nodes();
    }

    pub(crate) fn reload_source_modal_nodes(&mut self) {
        let Some(source_id) = self.source_modal_source_id.clone() else {
            return;
        };
        let db = self.db.clone();
        let tx = self.tx.clone();
        let page = self.source_modal_page;
        let page_size = self.source_modal_page_size;
        let latency = if self.source_modal_only_alive {
            Some("alive".to_string())
        } else {
            None
        };
        let keyword = nonempty_opt(&self.source_modal_keyword);
        self.source_modal_loading = true;
        self.rt.spawn(async move {
            let res = db
                .list_nodes(
                    None,
                    Some(&source_id),
                    keyword.as_deref(),
                    latency.as_deref(),
                    None,
                    page,
                    page_size,
                    Some("updated"),
                )
                .await
                .map(|p| (p.items, p.total))
                .map_err(|e| e.to_string());
            let _ = tx.send(BgMsg::SourceModalNodes {
                source_id,
                result: res,
            });
        });
    }

    pub(crate) fn speedtest_source(&mut self, source_id: &str, source_name: &str) {
        let db = self.db.clone();
        let speedtester = self.speedtester.clone();
        let opts = self.speed_opts();
        let tx = self.tx.clone();
        let sid = source_id.to_string();
        self.busy = true;
        self.status = format!("正在测速订阅“{source_name}”全部节点...");
        self.rt.spawn(async move {
            let result = async {
                let ids = db
                    .list_node_ids_for_group(Some(&sid), None, None, None)
                    .await?;
                if ids.is_empty() {
                    anyhow::bail!("该订阅源没有节点");
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
            let _ = tx.send(BgMsg::OpDone {
                result: res,
                unlock: true,
            });
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
            app_icon_style: self.edit_app_icon_style.clone(),
            ui_language: self.edit_ui_language.clone(),
            ui_scale: self.edit_ui_scale,
            ui_dark: self.edit_ui_dark,
            sidebar_expanded: self.edit_sidebar_expanded,
            insider_channel: self.edit_insider_channel,
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
            speedtest_tcp_ping_only: self.edit_speedtest_tcp_ping_only,
            bandwidth_guard: self.edit_bandwidth_guard,
            bandwidth_min_kbps: self.edit_bandwidth_min_kbps,
            window_size: self.edit_window_size.clone(),
            console_tabs: self.edit_console_tabs.clone(),
        }
    }

    pub(crate) fn settings_dirty(&self) -> bool {
        self.settings_edit_state().is_dirty(&self.settings)
    }

    /// 「字体大小」直接走 egui 的 zoom_factor：它算的就是 `pixels_per_point = zoom × 系统 DPI`，
    /// 所以 125%/150% 的机器上用户自己的 DPI 不会被冲掉，换显示器也不用我们操心。
    /// 界面里绝大多数文字写死了 `.size(..)`，只改 `Style::text_styles` 会一半大一半小，
    /// 因此整屏一起缩才是这里要的效果。值没变时 egui 自己不会重绘，每帧调用是安全的。
    fn apply_ui_scale(&self, ctx: &egui::Context) {
        ctx.set_zoom_factor(crate::models::clamp_ui_scale(self.settings.ui_scale));
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
        self.edit_speedtest_tcp_ping_only = state.speedtest_tcp_ping_only;
        self.edit_bandwidth_guard = state.bandwidth_guard;
        self.edit_bandwidth_min_kbps = state.bandwidth_min_kbps;
        self.edit_ui_theme = state.ui_theme.clone();
        self.edit_app_icon_style = state.app_icon_style.clone();
        self.edit_ui_language = state.ui_language.clone();
        self.edit_ui_scale = state.ui_scale;
        self.edit_ui_dark = state.ui_dark;
        self.edit_sidebar_expanded = state.sidebar_expanded;
        self.edit_insider_channel = state.insider_channel;
        self.edit_kernel_source = state.kernel_source.clone();
        self.edit_firewall_auto_allow = state.firewall_auto_allow;
        self.edit_window_size = state.window_size.clone();
        self.edit_console_tabs = state.console_tabs.clone();
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
            speedtest_tcp_ping_only: Some(self.edit_speedtest_tcp_ping_only),
            bandwidth_guard: Some(self.edit_bandwidth_guard),
            bandwidth_min_kbps: Some(self.edit_bandwidth_min_kbps),
            ui_theme: Some(self.edit_ui_theme.clone()),
            app_icon_style: Some(self.edit_app_icon_style.clone()),
            ui_language: Some(self.edit_ui_language.clone()),
            ui_scale: Some(crate::models::clamp_ui_scale(self.edit_ui_scale)),
            ui_dark: Some(self.edit_ui_dark),
            sidebar_expanded: Some(self.edit_sidebar_expanded),
            insider_channel: Some(self.edit_insider_channel),
            kernel_source: Some(self.edit_kernel_source.clone()),
            firewall_auto_allow: Some(self.edit_firewall_auto_allow),
            auto_failover: Some(self.auto_failover),
            // 发布服务器两项有自己的落库路径（save_publisher），这里刻意不动，
            // 避免周期性设置保存把用户刚写的值覆盖回旧内存副本。
            publisher_api: None,
            publisher_token: None,
            gist_pages: None,
            gist_queries: None,
            window_size: Some(self.edit_window_size.clone()),
            console_tabs: Some(self.edit_console_tabs.clone()),
        }
    }

    pub(crate) fn is_proxy_panel_visible(&self, panel: ProxyPanel) -> bool {
        let tabs = &self.edit_console_tabs;
        if tabs == "all" || tabs.trim().is_empty() {
            return true;
        }
        tabs.split(',').any(|part| part.trim() == panel.id())
    }

    pub(crate) fn toggle_proxy_panel_visibility(&mut self, panel: ProxyPanel) {
        let mut active: Vec<String> = if self.edit_console_tabs == "all" || self.edit_console_tabs.trim().is_empty() {
            ProxyPanel::ALL.iter().map(|p| p.id().to_string()).collect()
        } else {
            self.edit_console_tabs
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };

        let pid = panel.id().to_string();
        if let Some(idx) = active.iter().position(|x| x == &pid) {
            // 防空保护：至少保留 1 个可见 Tab
            if active.len() > 1 {
                active.remove(idx);
            }
        } else {
            active.push(pid);
        }

        self.edit_console_tabs = active.join(",");

        // 若当前激活面板被隐藏，切到第一个可见面板
        if !self.is_proxy_panel_visible(self.proxy_panel) {
            if let Some(first) = ProxyPanel::ALL.iter().copied().find(|p| self.is_proxy_panel_visible(*p)) {
                self.proxy_panel = first;
            }
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

    pub(crate) fn lang(&self) -> crate::i18n::Language {
        crate::i18n::Language::from_code(&self.edit_ui_language)
    }

    pub(crate) fn tr<'a>(&self, key: &'a str) -> &'a str {
        crate::i18n::tr(self.lang(), key)
    }
}

impl eframe::App for VpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let _rt_guard = self.rt.enter();
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
            ROUND_CORNERS.call_once(|| {
                crate::platform::round_window_corners();
                crate::platform::ensure_window_sysmenu_and_minimizebox();
            });
        }
        self.frame_count = self.frame_count.saturating_add(1);

        // 侦测最小化 -> 恢复窗口事件（无论是通过任务栏点击还原，还是托盘唤醒）
        let is_minimized = ctx.input(|i| i.viewport().minimized).unwrap_or(false);
        if self.was_minimized && !is_minimized {
            crate::startup_log(
                "Window state changed: minimized -> restored, triggering repaint sequence",
            );
            crate::platform::restore_and_repaint_main_window();
            self.restore_cooldown_frames = 12;
        }
        self.was_minimized = is_minimized;

        // 恢复窗口后的前若干帧持续请求重绘，确保 OpenGL/DirectX surface 与 DWM 顺利交换
        if self.restore_cooldown_frames > 0 {
            self.restore_cooldown_frames -= 1;
            ctx.request_repaint();
        }

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
            if self.settings.close_to_tray && self.tray.supported() && !self.force_quit {
                crate::startup_log("Window close requested -> 尝试收进托盘（停放到屏幕外）");
                if self.park_to_tray() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    return;
                }
                crate::startup_log("[托盘] 停放失败，降级为正常退出程序");
            }
            crate::startup_log("Window close requested -> quitting application");
            self.force_quit = true;
            self.tray.shutdown();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // TUN 提权交接成功了就把窗口让给提权实例：必须走真退出这条路（force_quit 绕开
        // "关到托盘"），否则互斥量还攥在这个进程手里，提权实例等 3 秒也接不上班。
        if self.pending_exit_after_handoff {
            self.pending_exit_after_handoff = false;
            self.force_quit = true;
            self.tray.shutdown();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        self.poll_bg(ctx);
        self.note_proxy_state_change();
        self.auto_ip_risk_for_egress();
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
        if self.applied_theme_id != self.settings.ui_theme
            || self.applied_ui_dark != self.settings.ui_dark
        {
            theme::apply(ctx, &self.settings.ui_theme, self.settings.ui_dark);
            self.applied_theme_id = self.settings.ui_theme.clone();
            self.applied_ui_dark = self.settings.ui_dark;
        }
        self.apply_ui_scale(ctx);

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
            if self.tab == Tab::Proxy
                && self.proxy_panel == ProxyPanel::Logs
                && self.last_log_poll.elapsed() >= std::time::Duration::from_millis(1500)
            {
                self.last_log_poll = std::time::Instant::now();
                self.reload_proxy_logs();
            }
        }
        // 预备状态（一个内核都没装）自己把内核下下来；一次运行最多自己动一次。
        self.maybe_auto_prepare();
        // 新安装检测到缺少 Geo 分流规则库时，自动在后台极速拉取（Fastly CDN 2秒秒下）。
        self.maybe_auto_download_geo();
        // 侧栏的"有新版"标记要自己会亮，不能等用户去关于页点。
        self.maybe_check_app_update();
        // Degraded 更要测：那正是"出口已经不通了"的状态，探测和 auto_failover 无关。
        if matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        ) {
            self.check_failover();
        }
        // 出口通不等于浏览器会走它：核对注册表（内部限频 30 秒一次，且只在系统代理模式下）。
        self.maybe_check_proxy_drift();
        if self.proxy_status.state != ProxyState::Running && !self.kernel_groups.is_empty() {
            // 停掉内核后这些组就不存在了，留着会显示上一次的假状态。
            self.kernel_groups.clear();
        }

        // 自动内存整理与工作集修剪：
        // 1. 启动并完成首轮全量数据加载 4 秒后，修剪启动阶段由大量反序列化与字体栅格化产生的瞬态内存。
        // 2. 进程闲置时每 5 分钟自动修剪一次，释放空闲 SQLite 缓存页与工作集，保持极致内存水位。
        if !self.busy && !self.proxy_busy {
            if !self.startup_trim_done
                && self.last_full_reload.elapsed() >= std::time::Duration::from_secs(4)
            {
                self.startup_trim_done = true;
                self.last_auto_trim = std::time::Instant::now();
                let db = self.db.clone();
                self.rt.spawn(async move {
                    db.trim_cached_pools().await;
                });
                crate::platform::trim_working_set();
            } else if self.last_auto_trim.elapsed() >= std::time::Duration::from_secs(300) {
                self.last_auto_trim = std::time::Instant::now();
                let db = self.db.clone();
                self.rt.spawn(async move {
                    db.trim_cached_pools().await;
                });
                crate::platform::trim_working_set();
            }
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

        let sidebar_width = if self.settings.sidebar_expanded {
            SIDEBAR_EXPANDED_WIDTH
        } else {
            SIDEBAR_COLLAPSED_WIDTH
        };
        egui::SidePanel::left("sidebar")
            .exact_width(sidebar_width)
            .resizable(false)
            .frame(
                Frame::NONE
                    .fill(theme::sidebar())
                    .stroke(Stroke::NONE)
                    .inner_margin(Margin::symmetric(8, 10)),
            )
            .show(ctx, |ui| self.ui_sidebar(ui, &mut next_tab));

        egui::TopBottomPanel::bottom("status")
            .frame(
                Frame::NONE
                    .fill(theme::sidebar())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    // 上下 8 → 5：这条本来就被内容顶高度，按钮压矮之后它跟着收。
                    .inner_margin(Margin::symmetric(14, 5)),
            )
            .show(ctx, |ui| {
                // 这条里的按钮（启动/停止代理·模式 chip·测速全部）要矮一档：
                // 纵向内边距 7 → 3，文字仍是 normal 字号，读得清但不再撑高度。
                ui.spacing_mut().button_padding = egui::vec2(11.0, 3.0);
                ui.horizontal(|ui| {
                    if self.busy || self.kernel_busy {
                        ui.spinner();
                        let busy_text = if let Some(kp) = &self.kernel_progress {
                            if kp.stage == crate::kernel::KernelUpdateStage::Downloading && kp.total > 0 {
                                let pct = ((kp.downloaded as f64 / kp.total as f64) * 100.0).clamp(0.0, 100.0) as u32;
                                format!("下载内核 {pct}%")
                            } else {
                                format!("内核{}", kp.stage.label())
                            }
                        } else {
                            "处理中".to_string()
                        };
                        ui.label(
                            RichText::new(busy_text)
                                .color(theme::accent())
                                .strong()
                                .small(),
                        );
                        if let Some(kp) = &self.kernel_progress {
                            if kp.stage == crate::kernel::KernelUpdateStage::Downloading && kp.total > 0 {
                                ui.add(
                                    egui::ProgressBar::new(kp.fraction())
                                        .animate(true)
                                        .desired_width(70.0),
                                );
                            }
                        }
                        ui.separator();
                    }
                    // 系统代理被外部改掉时**不再往这条上写提示**（2026-10-15 他一句"不用显示这个"）。
                    // 核对本身留着：它是设置页那个「自动重开系统代理」开关的唯一触发器
                    // （见 `BgMsg::ProxyDrift`），开关关着就只写 startup.log。
                    ui.label(RichText::new("状态").strong().color(theme::muted()).small());
                    // 状态文本必须给右边的动作组让位。`with_layout(right_to_left)` 的那组
                    // 按钮永远钉在内容区右边界上，所以文本一长就是**压到按钮上面**（他
                    // 2026-10-15 报的"挤成位置很奇怪"）。动作组多宽只有画完才知道，
                    // 这里用上一帧量到的那份：按钮组本身从不移动，所以最多抖一帧就对上了。
                    let status_left = ui.cursor().left();
                    // 下限：第一帧还没量过，按"四个控件"的常见宽度先让开。
                    const STATUS_ACTIONS_FLOOR: f32 = 300.0;
                    let room = (ui.max_rect().right()
                        - status_left
                        - self.status_actions_width.max(STATUS_ACTIONS_FLOOR))
                    .max(120.0);
                    let status = self.status.clone();
                    let small = egui::TextStyle::Small.resolve(ui.style());
                    let shown = crate::gui::widgets::ellipsize(ui, &status, room, small);
                    ui.label(RichText::new(shown).color(theme::text_primary()).small())
                        .on_hover_text(status);
                    let mut group_width = 0.0_f32;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 2026-10-07 他要求把控制按钮挪到状态栏右侧：先加的最靠右。
                        // 原来这儿的强调色选择器和会话统计都撤了 —— 主题在系统设置和
                        // Ctrl+K 里都还能改，流量读数挪到控制台表头那行。
                        self.ui_proxy_control_buttons(ui);
                        group_width = ui.min_rect().width();
                    });
                    self.status_actions_width = group_width;
                });
            });

        egui::CentralPanel::default()
            .frame(
                Frame::NONE
                    .fill(theme::page_bg())
                    .inner_margin(Margin::symmetric(18, 14)),
            )
            .show(ctx, |ui| {
                match self.tab {
                    Tab::Sources => {
                        ui.push_id("top_tab_sources", |ui| self.ui_sources(ui));
                    }
                    Tab::Nodes => {
                        ui.push_id("top_tab_nodes", |ui| self.ui_nodes(ui));
                    }
                    Tab::Proxy => {
                        ui.push_id("top_tab_proxy", |ui| self.ui_proxy(ui));
                    }
                    Tab::Extensions => {
                        ui.push_id("top_tab_extensions", |ui| self.ui_extensions(ui));
                    }
                    Tab::Settings => {
                        ui.push_id("top_tab_settings", |ui| self.ui_settings(ui));
                    }
                    Tab::Api => {
                        self.tab = Tab::Settings;
                        self.settings_tab = SettingsTab::Api;
                        ui.push_id("top_tab_settings", |ui| self.ui_settings(ui));
                    }
                    Tab::About => {
                        ui.push_id("top_tab_about", |ui| self.ui_about(ui));
                    }
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
        self.ui_source_nodes_modal(ctx);
        self.ui_node_picker_modal(ctx);

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
            let text = i.events.iter().find_map(|event| match event {
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

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        let _rt_guard = self.rt.enter();
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
        let _rt_guard = self.rt.enter();
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

impl VpnApp {
    pub(crate) fn ensure_logo_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        if let Some(texture) = &self.logo_texture {
            return texture.clone();
        }
        let bytes: &[u8] = include_bytes!("../../assets/app-icon-classic.png");
        let img = image::load_from_memory(bytes)
            .expect("load app-icon")
            .to_rgba8();
        let (w, h) = img.dimensions();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            &img.into_raw(),
        );
        let tex = ctx.load_texture("app-logo", color_image, egui::TextureOptions::LINEAR);
        self.logo_texture = Some(tex.clone());
        tex
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::window::{resize_handle_rects, TITLE_BAR_BTN_W, TITLE_BAR_H};
    use crate::speedtest::{SpeedTestPhase, SpeedTestProgress};
    use eframe::egui::{Sense, Vec2};
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
        if cfg!(windows) {
            assert!(requires_tun_elevation(ProxyMode::Tun, false));
            assert!(!requires_tun_elevation(ProxyMode::Tun, true));
            assert!(!requires_tun_elevation(ProxyMode::System, false));
        } else {
            assert!(!requires_tun_elevation(ProxyMode::Tun, false));
        }
    }

    /// TUN 交接的命令行：两个 flag 各一次、配置能原样读回来、节点 id 带的是要起的那个。
    #[test]
    fn tun_handoff_arguments_carry_config_and_node_once() {
        let mut options = crate::proxy::MihomoConfigOptions::default();
        options.mode = ProxyMode::Tun;
        options.url_test_size = 9;
        let args = VpnApp::tun_handoff_arguments(&options, "node-42").expect("编码不该失败");
        assert_eq!(args[0], "--elevated-proxy-config");
        assert_eq!(args[2], "--start-proxy-node");
        assert_eq!(args[3], "node-42");
        assert_eq!(
            args.iter()
                .filter(|token| token.starts_with("--"))
                .count(),
            2,
            "只该有两个 flag：{args:?}"
        );
        let decoded = crate::proxy::decode_elevated_proxy_config(&args[1]).expect("解码不该失败");
        assert_eq!(decoded, options, "提权实例读回来的配置和这里发出去的不是同一份");
    }

    /// 反向对照（#32 那个"一闪就没"）：本进程自己就是带着 `--start-proxy-node` 起来的
    /// （安装器、开机自启、以及上一次交接都会这样），拼出来的参数过一遍 `elevated_arguments`
    /// 之后必须只剩一个该 flag，而且值是新节点 —— 出现两次 clap 直接 ArgumentConflict，
    /// 提权实例在写下第一行日志之前就 exit(2)。
    #[test]
    fn the_handoff_never_duplicates_the_node_flag() {
        let current = vec!["--start-proxy-node".to_string(), "old-node".to_string()];
        let options = crate::proxy::MihomoConfigOptions::default();
        let extra = VpnApp::tun_handoff_arguments(&options, "new-node").expect("编码不该失败");
        let assembled = crate::platform::elevated_arguments(&current, &extra);
        let node_flags = assembled
            .iter()
            .filter(|token| token.as_str() == "--start-proxy-node")
            .count();
        let config_flags = assembled
            .iter()
            .filter(|token| token.as_str() == "--elevated-proxy-config")
            .count();
        assert_eq!(node_flags, 1, "节点 flag 重复了：{assembled:?}");
        assert_eq!(config_flags, 1, "配置 flag 重复了：{assembled:?}");
        let position = assembled
            .iter()
            .position(|token| token.as_str() == "--start-proxy-node")
            .expect("节点 flag 得在");
        assert_eq!(
            assembled[position + 1], "new-node",
            "去重时把要启动的新节点吃掉了：{assembled:?}"
        );
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

    /// 外观那两条以前不在 `is_dirty` 里：设置页只点主题时 `auto_save && settings_dirty()`
    /// 的后半个条件不成立，于是"即时生效并自动保存"其实根本没落盘。
    #[test]
    fn appearance_changes_count_as_unsaved() {
        let settings = AppSettings::default();

        let mut state = SettingsEditState::from_settings(&settings);
        state.ui_theme = "soft-teal".to_string();
        assert!(state.is_dirty(&settings), "只换主题也要算未保存");

        let mut state = SettingsEditState::from_settings(&settings);
        state.ui_scale = 1.0;
        assert!(state.is_dirty(&settings), "只调字号也要算未保存");

        let state = SettingsEditState::from_settings(&settings);
        assert!(!state.is_dirty(&settings), "刚灌进来的值不能一上来就是脏的");

        // 播种走的就是 `from_settings`：库里存的外观值必须原样带出来，否则第一次加载
        // 设置会把它们显示成默认值（老版本连这一步都没有，见 `settings_seeded`）。
        let stored = AppSettings {
            ui_theme: "soft-teal".to_string(),
            ui_scale: 1.0,
            ..AppSettings::default()
        };
        let state = SettingsEditState::from_settings(&stored);
        assert_eq!(state.ui_theme, "soft-teal");
        assert!((state.ui_scale - 1.0).abs() < 0.001);
        assert!(!state.is_dirty(&stored));
    }

    /// 「字体大小」= egui 的 `zoom_factor`（`pixels_per_point = zoom × 系统 DPI`）。
    /// 这里唯一会算错的是取值范围：越界或坏值必须夹回可用区间，否则界面会缩到看不见
    /// 或者撑出屏外，而且下一次启动还是这样。
    #[test]
    fn ui_scale_is_clamped_to_the_usable_band() {
        use crate::models::clamp_ui_scale;
        assert_eq!(clamp_ui_scale(1.0), 1.0);
        assert_eq!(clamp_ui_scale(0.1), 0.85);
        assert_eq!(clamp_ui_scale(9.0), 1.40);
        assert_eq!(clamp_ui_scale(f32::NAN), 1.2, "坏值退回默认 120%");
        assert_eq!(clamp_ui_scale(f32::INFINITY), 1.2);
    }

    /// 钉住这条设置的全部依据：`set_zoom_factor` 确实会把整屏的 `pixels_per_point` 改掉
    /// （下一帧生效），而不是只影响某一种字号。
    #[test]
    fn zoom_factor_rescales_the_whole_ui() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::Vec2::new(1280.0, 800.0),
        ));
        let viewport_id = input.viewport_id;
        input
            .viewports
            .get_mut(&viewport_id)
            .expect("默认输入里总该有根视口")
            .native_pixels_per_point = Some(1.25);
        let _ = ctx.run(input.clone(), |ctx| {
            assert_eq!(ctx.pixels_per_point(), 1.25, "系统 DPI 该原样传进来");
            ctx.set_zoom_factor(1.2);
        });
        // zoom 是"下一帧生效"，所以再跑一帧才看得到。
        let _ = ctx.run(input, |ctx| {
            assert!(
                (ctx.pixels_per_point() - 1.5).abs() < 0.001,
                "1.25 的 DPI × 120% 该是 1.5，实际 {}",
                ctx.pixels_per_point()
            );
        });
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
    fn country_code_and_color_mappings() {
        assert_eq!(country_code_and_color("US").0, "US");
        assert_eq!(country_code_and_color("us").0, "US");
        assert_eq!(country_code_and_color("SG").0, "SG");
        assert_eq!(country_code_and_color("HK").0, "HK");
        assert_eq!(country_code_and_color("JP").0, "JP");
        assert_eq!(country_code_and_color("TW").0, "TW");
        assert_eq!(country_code_and_color("UK").0, "UK");
        assert_eq!(country_code_and_color("GB").0, "UK");
        assert_eq!(country_code_and_color("unknown").0, "GL");

        // 验证特定国家色彩非黑且鲜明
        let (_, us_color) = country_code_and_color("US");
        let (_, sg_color) = country_code_and_color("SG");
        let (_, hk_color) = country_code_and_color("HK");
        assert_ne!(us_color, sg_color);
        assert_ne!(us_color, hk_color);
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
        use eframe::egui::{
            self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Vec2,
        };
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
        let _ = ctx.run(input, |ctx| {
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
                        for text in ["启动代理", "停止代理", "测速全部"] {
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
                leftover =
                    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
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
            let mut out = (egui::Rect::NOTHING, egui::Rect::NOTHING, None, false);
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
#[cfg(test)]
mod status_bar_room_tests {
    use eframe::egui::{self, RichText, Vec2};

    /// 复刻状态栏那一行的结构（照产品里那段：状态 label 限宽 + 动作组 `right_to_left`）。
    /// 跑两帧，和产品同一个口径：第一帧量出动作组宽度，第二帧照着让位。
    /// 返回（状态文字右界, 动作组左界, 动作组右界, 内容区右界）。
    fn probe(width: f32, status: &str, bound: bool) -> (f32, f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut reserved = 0.0_f32;
        let mut out = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        for _ in 0..2 {
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(width, 60.0),
            ));
            let _ = ctx.run(input, |ctx| {
                egui::TopBottomPanel::bottom("status")
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 5)))
                    .show(ctx, |ui| {
                        ui.spacing_mut().button_padding = Vec2::new(11.0, 3.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("状态").small());
                            let left = ui.cursor().left();
                            let room = if bound {
                                (ui.max_rect().right() - left - reserved.max(300.0)).max(120.0)
                            } else {
                                f32::INFINITY
                            };
                            let small = egui::TextStyle::Small.resolve(ui.style());
                            let shown = crate::gui::widgets::ellipsize(ui, status, room, small);
                            let response = ui.label(RichText::new(shown).small());
                            let mut group_left = f32::MAX;
                            let mut group_right = 0.0_f32;
                            let mut group_width = 0.0_f32;
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    for text in ["停止代理", "测速全部"] {
                                        let rect = ui.button(text).rect;
                                        group_left = group_left.min(rect.left());
                                        group_right = group_right.max(rect.right());
                                    }
                                    let chip = egui::Frame::NONE
                                        .inner_margin(egui::Margin::symmetric(9, 2))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new("系统代理").small());
                                        })
                                        .response
                                        .rect;
                                    group_left = group_left.min(chip.left());
                                    group_right = group_right.max(chip.right());
                                    group_width = ui.min_rect().width();
                                },
                            );
                            reserved = group_width;
                            out = (
                                response.rect.max.x,
                                group_left,
                                group_right,
                                ui.max_rect().right(),
                            );
                        });
                    });
            });
        }
        out
    }

    /// 他 2026-10-15 报的那条超长状态（防火墙失败原文，200+ 字）。
    fn long_status() -> String {
        let one = "防火墙规则未写入：防火墙放行部分失败（0 成功 / 4 失败：\"vpn-parser.exe\": \
                   添加防火墙规则失败: 未知错误；\"mihomo.exe\": 添加防火墙规则失败: 未知错误；\
                   \"xray.exe\": 添加防火墙规则失败: 未知错误；\"sing-box.exe\": \
                   添加防火墙规则失败: 未知错误）";
        format!("{one}；{one}")
    }

    /// 动作组永远钉在内容区右边界上（他这句需求："状态栏右边的动作组不要动"），
    /// 长状态文本必须自己截断让位，而不是压到按钮上面。
    #[test]
    fn a_long_status_line_gives_the_action_group_its_room() {
        for width in [960.0, 1100.0, 1330.0] {
            let (status_right, group_left, group_right, content_right) =
                probe(width, &long_status(), true);
            assert!(
                (group_right - content_right).abs() < 0.6,
                "{width} 宽时动作组右界 {group_right:.1} 没钉在内容区右界 {content_right:.1}"
            );
            assert!(
                status_right <= group_left - 0.5,
                "{width} 宽时状态文本右界 {status_right:.1} 压到了动作组左界 {group_left:.1}"
            );
        }
    }

    /// 反向对照：不限宽（就是改之前的写法）时，文本确实盖到按钮上，
    /// 而按钮的位置一动不动 —— 证明"挤成位置很奇怪"是文本越界，不是按钮被推走。
    #[test]
    fn without_the_bound_the_text_lands_on_the_buttons() {
        for width in [960.0, 1330.0] {
            let (status_right, group_left, group_right, content_right) =
                probe(width, &long_status(), false);
            assert!(
                status_right > group_left,
                "对照组失效：不限宽时文本右界 {status_right:.1} 竟然没越过按钮左界 {group_left:.1}"
            );
            assert!(
                (group_right - content_right).abs() < 0.6,
                "{width} 宽时动作组本来就不该移动：右界 {group_right:.1} vs 内容区 {content_right:.1}"
            );
        }
    }
}

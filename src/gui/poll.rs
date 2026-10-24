use crate::gui::prepare;
use crate::gui::types::{BgMsg, DnsRecord};
use crate::gui::widgets::{format_speedtest_progress, node_group_key, normalize_api_base};
use crate::gui::VpnApp;
use crate::kernel::KernelKind;
use crate::models::{FetchResult, GOOGLE_PROXY_TEST_KIND};
use crate::proxy::{ProxyMode, ProxyState};
use eframe::egui;
use std::sync::mpsc::TryRecvError;

impl VpnApp {
    /// 跟踪代理进入/退出 Running 的时刻，仪表盘"运行时长"的数据源。
    pub(crate) fn note_proxy_state_transition(&mut self, new_state: ProxyState) {
        let now_running = matches!(new_state, ProxyState::Running | ProxyState::Degraded);
        let was_running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        if now_running && !was_running {
            self.proxy_started_at = Some(std::time::Instant::now());
            // 刚跑起来就先测一轮服务连通性（把上次检测时间回拨到刚好"过期"）
            self.last_service_check = std::time::Instant::now() - std::time::Duration::from_secs(61);
            // 启动代理立刻静默检测出口 IP 归属（机房/住宅标签马上有真实数据，不弹窗）
            self.refresh_egress_silently();
            // 应用未启动代理时在抽屉里记住的调度选择：
            // 若为 PROXY 主组则立刻应用；若为其它子策略组，因此时内核尚未上报 groups，留待 KernelGroups 消息到达时校验后应用
            if let Some((grp, mbr)) = self.pending_group_pick.take() {
                if grp == "PROXY" {
                    self.select_kernel_member(grp, mbr);
                } else if grp != "全部" && grp != "all" {
                    self.pending_group_pick = Some((grp, mbr));
                }
            }
        } else if matches!(new_state, ProxyState::Stopped | ProxyState::Failed) {
            self.proxy_started_at = None;
            self.pending_group_pick = None;
        }
    }

    /// 性能采样：每 500ms 差分一次本软件的 CPU / 内存 / 磁盘占用，供性能卡显示。
    fn update_perf_stats(&mut self) {
        if self.last_perf_sample.elapsed() < std::time::Duration::from_millis(500) {
            return;
        }
        let now = std::time::Instant::now();
        let sample = crate::gui::perf::raw_sample();
        let mem_mb = crate::gui::perf::working_set_mb();
        let mut cpu = self.perf_stats.cpu_percent;
        let mut disk = self.perf_stats.disk_mbps;
        if let (Some(prev), Some(cur)) = (self.perf_prev, sample) {
            let dt = now.duration_since(prev.0).as_secs_f32();
            if dt > 0.05 {
                // 除以全部逻辑核心数：100% = 满载所有核心（单核口径看着太夸张）
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get() as f32)
                    .unwrap_or(1.0);
                let cpu_s = (cur.cpu_100ns.saturating_sub(prev.1)) as f32 / 10_000_000.0;
                cpu = (cpu_s / dt * 100.0 / cores).clamp(0.0, 100.0);
                let bytes = (cur.read_bytes.saturating_sub(prev.2)
                    + cur.write_bytes.saturating_sub(prev.3)) as f32;
                disk = (bytes / dt / (1024.0 * 1024.0)).max(0.0);
            }
        }
        self.perf_prev = sample.map(|s| (now, s.cpu_100ns, s.read_bytes, s.write_bytes));
        self.perf_stats = crate::gui::perf::PerfSample {
            cpu_percent: cpu,
            mem_mb,
            disk_mbps: disk,
        };
        self.last_perf_sample = now;
    }

    pub(crate) fn poll_bg(&mut self, ctx: &egui::Context) {
        let mut need_repaint = false;
        // 服务连通性检测的自动刷新（每 60s 一轮；代理运行走代理、未运行走直连）
        self.maybe_check_services();
        // 本软件 CPU/内存/磁盘 占用采样（500ms 一次差分）
        self.update_perf_stats();
        // 局域网 IP 每 60 秒刷新一次（切换网络后能跟上）
        if self.last_lan_ip_check.elapsed() > std::time::Duration::from_secs(60) {
            self.last_lan_ip_check = std::time::Instant::now();
            self.lan_ip = crate::gui::views::proxy::charts::detect_lan_ip();
        }
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    need_repaint = true;
                    match msg {
                        BgMsg::Sources(Ok(list)) => {
                            if self.fetch_results.is_empty() {
                                for source in &list {
                                    if let Some(status) = &source.last_status {
                                        let success = status.starts_with("ok")
                                            || status.contains("unchanged");
                                        self.fetch_results.push(FetchResult {
                                            source_id: source.id.clone(),
                                            source_name: source.name.clone(),
                                            resolved_url: source
                                                .last_resolved_url
                                                .clone()
                                                .unwrap_or_else(|| source.url_template.clone()),
                                            success,
                                            message: status.clone(),
                                            node_count: source.last_node_count.max(0) as usize,
                                            extracted_rules: Vec::new(),
                                        });
                                    }
                                }
                            }
                            self.sources = list;
                        }
                        BgMsg::Sources(Err(e)) => self.status = format!("加载订阅源失败: {e}"),
                        BgMsg::Nodes { req_id, result } => {
                            if req_id == self.nodes_req_id {
                                match result {
                                    Ok((items, total)) => {
                                        self.nodes = items;
                                        self.node_total = total;
                                    }
                                    Err(e) => self.status = format!("加载节点失败: {e}"),
                                }
                            }
                        }
                        BgMsg::SourceModalNodes { source_id, result } => {
                            if self.source_modal_source_id.as_deref() == Some(&source_id) {
                                self.source_modal_loading = false;
                                match result {
                                    Ok((items, total)) => {
                                        self.source_modal_nodes = items;
                                        self.source_modal_total = total;
                                    }
                                    Err(e) => {
                                        self.status = format!("加载订阅源节点失败: {e}");
                                    }
                                }
                            }
                        }
                        BgMsg::NodeGroups(Ok(groups)) => {
                            self.node_groups = groups;
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
                            if !matches!(drift, Drift::InSync) {
                                let text = crate::system_proxy::drift_text(&drift);
                                crate::startup_log(&format!("[系统代理核对] {text}"));
                                // 开关开着才动手（默认关：那是用户的 HKCU）。
                                if self.settings.auto_rearm_system_proxy {
                                    self.status = format!("{text} -> 正在按设置写回");
                                    let manager = self.proxy_manager.clone();
                                    let tx = self.tx.clone();
                                    self.rt.spawn(async move {
                                        let message = match manager.rearm_system_proxy().await {
                                            Ok(done) => done,
                                            Err(error) => format!("系统代理写回失败：{error}"),
                                        };
                                        crate::startup_log(&format!("[系统代理核对] {message}"));
                                        let _ = tx.send(BgMsg::Status(message));
                                    });
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
                            if self.settings.gist_pages > 0 {
                                self.gist_pages = self.settings.gist_pages.clamp(1, 300) as usize;
                            }
                            let saved_queries: std::collections::HashSet<&str> = self
                                .settings
                                .gist_queries
                                .split(',')
                                .map(str::trim)
                                .filter(|q| !q.is_empty())
                                .collect();
                            if !saved_queries.is_empty() {
                                self.gist_query_flags = crate::collector::GIST_DISCOVERY_QUERIES
                                    .iter()
                                    .map(|query| saved_queries.contains(query))
                                    .collect();
                                if !self.gist_query_flags.iter().any(|&f| f) {
                                    self.gist_query_flags[0] = true;
                                }
                            }
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
                            if !self.settings_seeded {
                                // 第一次拿到库里的设置：无条件灌进编辑态。
                                // 下面那个 `!settings_dirty()` 的门槛是为了"别把用户正在改的
                                // 东西冲掉"，可启动时编辑态还是代码里的默认值，跟库里的值一比
                                // 就是脏的 —— 于是老版本里这条同步永远不会发生：改过设置的人
                                // 重启后，设置页显示的是默认值，动一下滑杆就把真配置写回去了。
                                self.settings_seeded = true;
                                self.sync_edit_settings();
                            } else if !self.settings_dirty() && !self.settings_saving {
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
                        BgMsg::SwitchCounts(today, total) => {
                            self.switch_today = today;
                            self.switch_total = total;
                        }
                        BgMsg::EgressSwitches(rows, today, total) => {
                            self.switch_rows = rows;
                            self.switch_today = today;
                            self.switch_total = total;
                        }
                        BgMsg::SourceAdded(id, name) => {
                            // 新增源：拉取节点，若自带分流规则自动添加，并按需执行测速
                            self.pending_added_source = Some((id.clone(), name));
                            self.fetch_one(id);
                        }
                        BgMsg::AddedSourceIds {
                            source_id,
                            source_name,
                            ids,
                        } => {
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
                                        &format!(
                                            "正在测速新增订阅「{source_name}」的 {count} 个节点..."
                                        ),
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
                            match &result {
                                Ok(res) => {
                                    self.fetch_results
                                        .retain(|item| item.source_id != res.source_id);
                                    self.fetch_results.push(res.clone());
                                }
                                Err(error) => {
                                    let name = self
                                        .sources
                                        .iter()
                                        .find(|s| s.id == source_id)
                                        .map(|s| s.name.clone())
                                        .unwrap_or_else(|| source_id.clone());
                                    let url = self
                                        .sources
                                        .iter()
                                        .find(|s| s.id == source_id)
                                        .map(|s| s.url_template.clone())
                                        .unwrap_or_else(|| "-".into());
                                    self.fetch_results
                                        .retain(|item| item.source_id != source_id);
                                    self.fetch_results.push(FetchResult {
                                        source_id: source_id.clone(),
                                        source_name: name,
                                        resolved_url: url,
                                        success: false,
                                        message: error.clone(),
                                        node_count: 0,
                                        extracted_rules: Vec::new(),
                                    });
                                }
                            }
                            let is_pending_added = self
                                .pending_added_source
                                .as_ref()
                                .is_some_and(|(id, _)| id == &source_id);
                            let mut auto_added_rules = 0;
                            if is_pending_added {
                                if let Ok(ref fetch) = result {
                                    if !fetch.extracted_rules.is_empty() {
                                        let before = self.proxy_config.rules.len();
                                        let mut combined = self.proxy_config.rules.clone();
                                        combined.extend(fetch.extracted_rules.clone());
                                        self.proxy_config.rules =
                                            crate::proxy::optimize_traffic_rules(&combined);
                                        auto_added_rules = self
                                            .proxy_config
                                            .rules
                                            .len()
                                            .saturating_sub(before);
                                        if auto_added_rules > 0 {
                                            self.save_proxy_config();
                                            tracing::info!(
                                                source = %fetch.source_name,
                                                rules_added = auto_added_rules,
                                                "已自动将订阅源自带的规则添加至规则分流列表"
                                            );
                                        }
                                    }
                                }
                                let (_, name) =
                                    self.pending_added_source.take().unwrap_or_default();
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
                                Ok(result) if result.success => {
                                    if auto_added_rules > 0 {
                                        format!(
                                            "订阅“{}”拉取完成：{} 个节点，已自动添加 {} 条自带分流规则",
                                            result.source_name, result.node_count, auto_added_rules
                                        )
                                    } else {
                                        format!(
                                            "订阅“{}”拉取完成：{} 个节点",
                                            result.source_name, result.node_count
                                        )
                                    }
                                }
                                Ok(result) => {
                                    let msg = &result.message;
                                    if msg.contains("request failed")
                                        || msg.contains("timed out")
                                        || msg.contains("timeout")
                                        || msg.contains("connection")
                                        || msg.contains("error sending request")
                                    {
                                        format!(
                                            "订阅“{}”拉取失败（海外服务直连阻断）：{}；建议启动代理后点击「代理拉取」",
                                            result.source_name, msg
                                        )
                                    } else {
                                        format!(
                                            "订阅“{}”拉取失败：{}",
                                            result.source_name, msg
                                        )
                                    }
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
                        BgMsg::IpRisk(Ok(risk), manual) => {
                            self.ip_risk_busy = false;
                            self.ip_risk_error = None;
                            self.ip_risk_failed_for = None;
                            crate::startup_log(&format!(
                                "[出口归属] 服务器归属查到：{}",
                                risk.headline()
                            ));
                            if manual {
                                self.status = format!("IP 检测：{}", risk.headline());
                            }
                            self.ip_risk = Some(risk);
                        }
                        BgMsg::IpRisk(Err(error), manual) => {
                            self.ip_risk_busy = false;
                            self.ip_risk = None;
                            // 记下这颗查失败过：徽章要区分「没查过」和「查了没查到」。
                            self.ip_risk_failed_for = if manual {
                                self.ip_risk_target.clone()
                            } else {
                                self.ip_risk_auto_for.clone()
                            };
                            crate::startup_log(&format!("[出口归属] 服务器归属查询失败：{error}"));
                            self.ip_risk_error = Some(error);
                            if manual {
                                self.status = "IP 检测查询失败".into();
                            }
                        }
                        BgMsg::IpEgress(node_id, Ok(egress), manual) => {
                            self.ip_risk_egress_pending = false;
                            self.ip_risk_egress_error = None;
                            crate::startup_log(&format!(
                                "[出口归属] 出口查到：{}",
                                egress.headline()
                            ));
                            if manual {
                                self.status = format!("出口 {}", egress.headline());
                            }
                            self.ip_risk_egress_for = Some(node_id);
                            self.ip_risk_egress = Some(egress);
                        }
                        BgMsg::IpEgress(node_id, Err(error), _manual) => {
                            self.ip_risk_egress_pending = false;
                            self.ip_risk_egress = None;
                            self.ip_risk_egress_for = Some(node_id);
                            crate::startup_log(&format!("[出口归属] 出口查询失败：{error}"));
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
                        BgMsg::AppUpdateCheck(Ok(info)) => {
                            let quiet = std::mem::take(&mut self.app_update_quiet);
                            self.app_update_busy = false;
                            self.app_update_progress = None;
                            self.app_update_checked_at =
                                Some(chrono::Local::now().format("%H:%M").to_string());
                            // 查到的不是刚才那一版，之前下好的安装包就不能再当"已校验"用。
                            if self
                                .app_release
                                .as_ref()
                                .is_none_or(|old| old.tag != info.tag)
                            {
                                self.app_installer_path = None;
                            }
                            if quiet {
                                // 自动那次不吭声，但要在日志里留痕：侧栏↑亮不亮，看的就是这条。
                                crate::startup_log(&format!(
                                    "[更新] 自动检查: {}（本机 {}）",
                                    if info.update_available {
                                        format!("{} 可更新", info.tag)
                                    } else {
                                        "已是最新".into()
                                    },
                                    info.current_version
                                ));
                            } else {
                                self.status = info.message.clone();
                            }
                            self.app_release = Some(info);
                        }
                        BgMsg::AppUpdateCheck(Err(error)) => {
                            let quiet = std::mem::take(&mut self.app_update_quiet);
                            self.app_update_busy = false;
                            self.app_update_progress = None;
                            self.app_update_checked_at =
                                Some(chrono::Local::now().format("%H:%M").to_string());
                            if quiet {
                                // 后台那次查不到不打扰界面，但要在日志里留痕。
                                crate::startup_log(&format!("[更新] 自动检查失败: {error}"));
                            } else {
                                self.status = format!("检查更新失败: {error}");
                            }
                        }
                        BgMsg::AppUpdateProgress(progress) => {
                            self.status = progress.message.clone();
                            self.app_update_progress = Some(progress);
                        }
                        BgMsg::AppUpdateDownloaded(result) => {
                            self.app_update_busy = false;
                            self.app_update_progress = None;
                            match result {
                                Ok(path) => {
                                    self.status = format!("安装包已下载并校验：{}", path.display());
                                    self.app_installer_path = Some(path);
                                }
                                Err(error) => self.status = format!("下载更新失败: {error}"),
                            }
                        }
                        BgMsg::AppReleasesListed(result) => {
                            self.fetching_releases = false;
                            match result {
                                Ok(list) => {
                                    let count = list.len();
                                    self.status = format!("已获取 {count} 个版本");
                                    // 若当前选中的版本不存在于新列表中，重置选择
                                    if self.selected_release_tag.as_ref().is_none_or(|tag| !list.iter().any(|r| &r.tag == tag)) {
                                        if let Some(curr) = list.iter().find(|r| r.is_current) {
                                            self.selected_release_tag = Some(curr.tag.clone());
                                        } else if let Some(first) = list.first() {
                                            self.selected_release_tag = Some(first.tag.clone());
                                        }
                                    }
                                    self.available_releases = list;
                                }
                                Err(error) => {
                                    self.status = format!("获取版本列表失败: {error}");
                                }
                            }
                        }
                        BgMsg::ProxyStatus(status) => {
                            self.note_proxy_state_transition(status.state);
                            self.proxy_status = status;
                        }
                        BgMsg::ServiceChecks(results) => {
                            self.service_checks = Some(results);
                            self.service_checks_running = false;
                        }
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
                                Ok(message) => {
                                    self.status = message;
                                    if self.status.contains("已停止") {
                                        self.note_proxy_state_transition(ProxyState::Stopped);
                                        self.proxy_status.state = ProxyState::Stopped;
                                        self.proxy_status.local_proxy_ready = false;
                                        self.proxy_status.system_proxy_enabled = false;
                                    } else {
                                        self.note_proxy_state_transition(ProxyState::Running);
                                        self.proxy_status.state = ProxyState::Running;
                                        // 立刻静默查出口归属：状态卡"机房/住宅"马上有真实数据
                                        self.refresh_egress_silently();
                                        self.proxy_status.local_proxy_ready = true;
                                        self.proxy_status.system_proxy_enabled = self.proxy_config.mode == ProxyMode::System;
                                    }
                                }
                                Err(error) => {
                                    self.status = format!("代理操作失败: {error}");
                                    self.note_proxy_state_transition(ProxyState::Failed);
                                    self.proxy_status.state = ProxyState::Failed;
                                }
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
                                            KernelKind::Shoes => "shoes",
                                            KernelKind::Meow => "meow",
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

                                    // 记录新增 DNS 解析 / 域名请求轨迹
                                    if !conn.host.is_empty() || conn.destination.contains(":53") {
                                        let now_str = chrono::Local::now().format("%H:%M:%S").to_string();
                                        let is_duplicate = self
                                            .dns_records
                                            .iter()
                                            .take(15)
                                            .any(|r| r.host == conn.host && r.ip == conn.destination);
                                        if !is_duplicate {
                                            if self.dns_records.len() >= 300 {
                                                self.dns_records.pop_back();
                                            }
                                            self.dns_records.push_front(DnsRecord {
                                                id: conn.id.clone(),
                                                host: if conn.host.is_empty() {
                                                    conn.destination.clone()
                                                } else {
                                                    conn.host.clone()
                                                },
                                                ip: conn.destination.clone(),
                                                port: conn
                                                    .destination
                                                    .split(':')
                                                    .nth(1)
                                                    .unwrap_or("53")
                                                    .to_string(),
                                                process: conn.process.clone(),
                                                rule: conn.rule.clone(),
                                                target: conn.target.clone(),
                                                network: conn.network.to_uppercase(),
                                                time_str: now_str,
                                            });
                                        }
                                    }
                                }
                            }
                            if self.rule_seen.len() > 20_000 {
                                // 长跑兜底：只丢去重表，计数已经并进 tally 了。
                                self.rule_seen.clear();
                            }
                            if !self.rule_tally.is_empty()
                                && self.last_hit_flush.elapsed()
                                    >= std::time::Duration::from_secs(30)
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
                            if let Some((grp, mbr)) = self.pending_group_pick.take() {
                                if self.kernel_groups.iter().any(|g| g.name == grp) {
                                    self.select_kernel_member(grp, mbr);
                                }
                            }
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

}

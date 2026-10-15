use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    ActiveConnectionInfo, ProxyPanel, ProxySortBy, ProxyView, ProxyViewRow, RuleDraftKind, RulesTab,
};
use crate::gui::widgets::{
    card_frame, clock_of, compact_node_card, country_flag_and_name, danger_btn, dns_mode_label,
    format_bytes, format_speed, header_cell, panel_frame, panel_tab, primary_btn,
    proxy_state_label, render_health_strip, render_traffic_sparkline, routing_mode_description,
    routing_mode_detail, rule_action_label, rule_matcher_label, section_title, status_badge,
    tun_stack_label,
};
use crate::gui::VpnApp;
use crate::kernel::{KernelKind, KernelPreference};
use crate::models::NodeSummary;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules, recommended_traffic_rules,
    social_telegram_rules, streaming_media_rules, DnsEnhancedMode, ProxyMode, ProxyState,
    RoutingMode, RuleAction, TrafficRule, TunStack, FINAL_GROUP, URL_TEST_GROUP,
};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;

/// 分流规则那行表头右侧动作组（导出 JSON + 导入规则）预留的宽度。
/// 「?」不在这里了 —— 示范只在导入时用得上，2026-10-15 他要求挪进导入弹框。
const RULES_ACTIONS_WIDTH: f32 = 180.0;

/// 两个分区标签（规则配置 / 命中记录）至少要占的宽度。
const RULES_TABS_MIN_WIDTH: f32 = 170.0;

/// `split_row` 两个闭包都要碰 `self`，所以动作先记成值、出框后再执行。
enum RulesHeaderAction {
    None,
    Export,
    Import,
}

impl VpnApp {
    pub(crate) fn ui_proxy(&mut self, ui: &mut Ui) {
        let selected_node = self
            .selected_proxy_node_id
            .as_ref()
            .and_then(|id| {
                self.proxy_nodes
                    .iter()
                    .chain(self.nodes.iter())
                    .find(|node| &node.id == id)
            })
            .cloned();

        ui.add_space(8.0);

        panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                // 「预备」= 内核一个都没装上、代理也没在跑。这时候说"未运行"是废话，
                // 该说的是它还没准备好（并且软件自己正在把内核下下来）。
                let prepared = crate::gui::prepare::is_prepared(
                    self.kernel_installed_cached(),
                    matches!(
                        self.proxy_status.state,
                        ProxyState::Running | ProxyState::Degraded | ProxyState::Starting
                    ),
                );
                let (state_text, state_color) = if prepared {
                    // 徽章自己说清走到哪一步，不用他去翻状态栏：正在下 / 第几次失败。
                    let text = if self.kernel_busy {
                        format!(
                            "预备 · 正在下载内核 ({}/{})",
                            self.prepare_attempts,
                            crate::gui::prepare::MAX_AUTO_ATTEMPTS
                        )
                    } else if self.prepare_error.is_some() {
                        format!(
                            "预备 · 下载失败 {}/{}",
                            self.prepare_attempts,
                            crate::gui::prepare::MAX_AUTO_ATTEMPTS
                        )
                    } else {
                        "预备".to_string()
                    };
                    (text, theme::accent())
                } else {
                    let (label, color) = proxy_state_label(self.proxy_status.state);
                    (label.to_string(), color)
                };
                let badge = status_badge(ui, &state_text, state_color);
                if prepared {
                    // 失败原因只在 hover 里：这行已经没有横向空间了，但"为什么没成功"
                    // 必须查得到，不能只留一个"下载失败 2/3"。
                    let why = match &self.prepare_error {
                        Some(error) => format!(
                            "预备状态自动下载 mihomo 失败（第 {}/{} 次，每次换一种下载方式）：{error}\n\
                             三次预算依次试：ghfast → gh-proxy →（这台机器挂了本地代理才有这一档）走代理 → 官方直连。\n\
                             全失败多半是这台机器到 GitHub 的路都不通；想立刻重来：按 Ctrl+K 搜「下载内核」自己选一种方式。",
                            self.prepare_attempts,
                            crate::gui::prepare::MAX_AUTO_ATTEMPTS
                        ),
                        None if self.kernel_busy => "预备状态：正在自动下载 mihomo…".to_string(),
                        None => "预备状态：会自动下载 mihomo 内核（开关在系统设置里）".to_string(),
                    };
                    badge.on_hover_text(why);
                }

                // 这里只报"界面上选了谁"；内核真正在用谁由下面那行「内核实际选路」按
                // /proxies 的实测结果说，两处各说各的、不重复猜。
                let node_name = self
                    .selected_proxy_node_name
                    .as_deref()
                    .unwrap_or("未选择代理节点");
                ui.label(
                    RichText::new(node_name)
                        .strong()
                        .size(16.5)
                        .color(theme::text_primary()),
                );

                if self.switching_node_id.is_some() {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Lightning, 12.0, theme::warning());
                        ui.label(
                            RichText::new("异步切换中...")
                                .color(theme::warning())
                                .strong()
                                .small(),
                        );
                    });
                }

                if let Some(node) = &selected_node {
                    // 协议彩块只在节点卡片里出现一次；顶栏再放一块，整页就到处是色块了。
                    let (flag, country_name) =
                        country_flag_and_name(node.country.as_deref().unwrap_or(""));
                    ui.label(
                        RichText::new(format!(
                            "{flag} {country_name} · {}:{}",
                            node.server, node.port
                        ))
                        .small()
                        .color(theme::text_secondary()),
                    );
                }

                // 连通状态从第二行挪上来：第二行整条要让给迷你流量图。
                // 宽度不够时只留图标、文字进 hover —— 宁可少一个字，也不能把它挤出卡片。
                let is_initial_rule_loading =
                    self.proxy_status
                        .last_error
                        .as_deref()
                        .map_or(false, |err| {
                            err.contains("规则加载中")
                                || err.contains("自动建立连接")
                                || err.contains("start initial configuration")
                        });
                let roomy = ui.available_width() > 260.0;
                // 「已连通」从此以实测为准，不再只看 ProxyState：2026-10-06 那次内核日志里
                // 全是 `dial tcp …: i/o timeout`，界面还挂着绿勾，就是这么骗过去的。
                let latest = self.health_rows.first();
                let status = if let Some(err) = self
                    .proxy_status
                    .last_error
                    .as_deref()
                    .filter(|_| !is_initial_rule_loading)
                {
                    Some((Icon::Alert, theme::danger(), err.to_string()))
                } else if is_initial_rule_loading {
                    Some((
                        Icon::Check,
                        theme::success(),
                        "代理运行中 (正在建立连接)".to_string(),
                    ))
                } else if self.proxy_status.state == ProxyState::Running {
                    match latest {
                        Some(row) if row.ok == 1 => Some((
                            Icon::Check,
                            theme::success(),
                            format!(
                                "实测 {} ✓ {}",
                                clock_of(&row.at),
                                row.latency_ms
                                    .map(|ms| format!("{ms} ms"))
                                    .unwrap_or_else(|| "—".into())
                            ),
                        )),
                        Some(row) => Some((
                            Icon::Alert,
                            // 和旁边那条健康带的"有不通"同一个红：带子红了、紧挨着的
                            // 这句还是石墨灰，看着像只改了一半。
                            crate::gui::widgets::HEALTH_FAILED,
                            format!(
                                "实测 {} ✗ {}（连续 {} 次）",
                                clock_of(&row.at),
                                row.error.clone().unwrap_or_else(|| "不通".into()),
                                self.health_fail_streak.max(1)
                            ),
                        )),
                        None => Some((
                            Icon::Radar,
                            theme::muted(),
                            "已启动 · 等待首次实测（≤60 秒）".to_string(),
                        )),
                    }
                } else if self.proxy_status.state == ProxyState::Degraded {
                    Some((Icon::Alert, theme::warning(), "降级运行中".to_string()))
                } else {
                    Some((Icon::Radar, theme::muted(), "未运行".to_string()))
                };
                if let Some((icon, color, text)) = status {
                    ui.horizontal(|ui| {
                        icon_label(ui, icon, 13.0, color);
                        if roomy {
                            ui.label(RichText::new(text).small().strong().color(color));
                        }
                    });
                }
                if roomy
                    && matches!(
                        self.proxy_status.state,
                        ProxyState::Running | ProxyState::Degraded
                    )
                {
                    render_health_strip(ui, &self.health_buckets);
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // 第二行：读数在左，迷你流量图吃掉右边剩下的整条宽度。累计值不在这儿 ——
            // 窗口底部状态栏一直显示会话总计。
            ui.horizontal(|ui| {
                let lat_str = selected_node
                    .as_ref()
                    .and_then(|n| n.latency_ms)
                    .map(|ms| format!("{ms} ms"))
                    .unwrap_or_else(|| "-".into());
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Lightning, 13.0, theme::text_secondary());
                    ui.label(
                        RichText::new(format!("延迟: {lat_str}"))
                            .small()
                            .strong()
                            .color(theme::text_secondary()),
                    );
                });
                ui.separator();

                let running = self.proxy_status.state == ProxyState::Running;
                if running {
                    let conns = self.active_connections.len();
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Radar, 12.0, theme::text_secondary());
                        ui.label(
                            RichText::new(format!("活跃连接: {conns}"))
                                .small()
                                .color(theme::text_secondary()),
                        );
                    });
                    ui.separator();
                }
                ui.label(
                    RichText::new(format!(
                        "端口: Mixed {} · Controller {}",
                        self.proxy_status.mixed_port, self.proxy_status.controller_port
                    ))
                    .small()
                    .color(theme::text_secondary()),
                );
                render_traffic_sparkline(ui, &self.traffic_history, running);
            });

            ui.add_space(4.0);
            // 自动切换状态 · 切换记录入口 · 上下行速率 · 会话累计，全在一行。
            // 2026-10-08 他把读数挪到"查看切换记录"后面；整行改成恒定渲染，
            // 开关关掉时读数还在，切换记录也不会因为开关关掉就点不开。
            ui.horizontal(|ui| {
                if self.auto_failover {
                    icon_label(ui, Icon::Shield, 12.0, theme::success());
                    let detail = self
                        .failover_last_reason
                        .as_deref()
                        .unwrap_or("已开启，最近 60 秒内代理实测正常");
                    // 一整行要装下速率和会话累计，理由动辄六七十字（节点名在里面）会把
                    // 后面的读数折到第二行、被表头的高度裁掉。
                    let detail: String = if detail.chars().count() > 30 {
                        let mut s: String = detail.chars().take(30).collect();
                        s.push('…');
                        s
                    } else {
                        detail.to_string()
                    };
                    ui.label(
                        RichText::new(format!(
                            "自动切换: 本次运行已切换 {} 次 · {detail}",
                            self.failover_switches
                        ))
                        .small()
                        .color(theme::muted()),
                    );
                }
                if ui
                    .link(format!("查看切换记录（今日 {} 次）", self.switch_today))
                    .clicked()
                {
                    self.show_switch_log = true;
                    self.reload_egress_switches();
                }
                ui.separator();
                let live = matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                );
                let color = if live {
                    theme::text_primary()
                } else {
                    theme::muted()
                };
                icon_label(ui, Icon::ArrowDown, 12.0, theme::accent_hover());
                ui.label(
                    RichText::new(format_speed(self.traffic_down_rate))
                        .small()
                        .strong()
                        .color(color),
                );
                icon_label(ui, Icon::ArrowUp, 12.0, theme::success());
                ui.label(
                    RichText::new(format_speed(self.traffic_up_rate))
                        .small()
                        .strong()
                        .color(color),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "会话 ↑ {} ↓ {}",
                        format_bytes(self.traffic_up_total),
                        format_bytes(self.traffic_down_total)
                    ))
                    .small()
                    .color(theme::muted()),
                );
            });
        });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            panel_tab(
                ui,
                &mut self.proxy_panel,
                ProxyPanel::Nodes,
                Icon::Pin,
                "可用节点",
            );
            let conn_label = if self.active_connections.is_empty() {
                "活跃连接".to_string()
            } else {
                format!("活跃连接 ({})", self.active_connections.len())
            };
            panel_tab(
                ui,
                &mut self.proxy_panel,
                ProxyPanel::Connections,
                Icon::Radar,
                &conn_label,
            );
            panel_tab(
                ui,
                &mut self.proxy_panel,
                ProxyPanel::Rules,
                Icon::Rules,
                "分流规则",
            );
            panel_tab(
                ui,
                &mut self.proxy_panel,
                ProxyPanel::Network,
                Icon::Globe,
                "DNS / TUN",
            );
            panel_tab(
                ui,
                &mut self.proxy_panel,
                ProxyPanel::Kernels,
                Icon::Cpu,
                "内核管理",
            );
            panel_tab(
                ui,
                &mut self.proxy_panel,
                ProxyPanel::Logs,
                Icon::Terminal,
                "运行日志",
            );
        });
        ui.separator();
        ui.add_space(6.0);

        match self.proxy_panel {
            ProxyPanel::Nodes => self.ui_proxy_nodes(ui),
            ProxyPanel::Connections => self.ui_proxy_connections(ui),
            ProxyPanel::Rules => self.ui_proxy_rules(ui),
            ProxyPanel::Network => self.ui_proxy_network(ui),
            ProxyPanel::Kernels => self.ui_proxy_kernels(ui),
            ProxyPanel::Logs => self.ui_proxy_logs(ui),
        }
    }

    /// 底部状态栏右侧那一组控制按钮：先加的最靠右，所以启动/停止代理排第一；
    /// 模式 chip 和「测速全部」在最后加 —— 从右往左是
    /// `启动/停止代理 · IP 检测 · 系统代理 chip · 测速全部`（用户 2026-10-08 指定）。
    /// 「重启」按钮同一天被他撤了：热重载从此只有本地 API `/api/proxy/restart` 一条路。
    pub(crate) fn ui_proxy_control_buttons(&mut self, ui: &mut Ui) {
        let running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        let transitioning = matches!(
            self.proxy_status.state,
            ProxyState::Starting | ProxyState::Stopping
        );
        if running {
            if ui
                .add_enabled(!self.proxy_busy, danger_btn("停止代理"))
                .clicked()
            {
                self.stop_proxy();
            }
            if ui
                .add_enabled(!self.ip_risk_busy, egui::Button::new("IP 检测"))
                .on_hover_text(
                    "查这个节点服务器 IP 的归属（机房/住宅）；当前在跑的节点还会经代理查一次真实出口",
                )
                .clicked()
            {
                self.check_ip_risk();
            }
        } else {
            let can_start =
                !self.proxy_busy && !transitioning && self.selected_proxy_node_id.is_some();
            let start = ui.add_enabled(can_start, primary_btn("启动代理"));
            let start = if self.selected_proxy_node_id.is_none() {
                start.on_hover_text("先在节点列表里点一个节点作为出口，再启动")
            } else {
                start
            };
            if start.clicked() {
                self.start_proxy();
            }
            if ui
                .add_enabled(!self.ip_risk_busy, egui::Button::new("IP 检测"))
                .on_hover_text("查该节点服务器 IP 是机房还是住宅出口")
                .clicked()
            {
                self.check_ip_risk();
            }
        }

        // 系统代理 / TUN：一个 chip 来回切，运行中不给改（改了要重载内核）。
        let (mode_icon, mode_label) = match self.proxy_config.mode {
            ProxyMode::System => (Icon::Globe, "系统代理"),
            ProxyMode::Tun => (Icon::Shield, "TUN 模式"),
        };
        let mode_resp = Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border()))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(9, 2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, mode_icon, 13.0, theme::accent());
                    ui.label(
                        RichText::new(mode_label)
                            .small()
                            .strong()
                            .color(theme::text_primary()),
                    );
                });
            })
            .response
            .interact(Sense::click())
            .on_hover_text("点击切换 系统代理 / TUN 模式");
        if mode_resp.clicked() {
            if self.proxy_status.config_editable() {
                self.proxy_config.mode = match self.proxy_config.mode {
                    ProxyMode::System => ProxyMode::Tun,
                    ProxyMode::Tun => ProxyMode::System,
                };
                self.save_proxy_config();
            } else {
                self.status = "请先停止代理后再切换工作模式".into();
            }
        }

        // 系统代理归属核对：注册表被别人改掉时界面必须说实话。以前只在启停那一刻写一次，
        // 之后哪怕 `ProxyEnable` 被外部置 0（浏览器全断），侧栏仍然显示"代理运行中 + 实测 200ms"
        // —— 又一根不说实话的绿条（2026-10-08 现场）。
        if let Some(drift) = self.proxy_drift.clone() {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Alert, 13.0, theme::warning());
                ui.label(
                    RichText::new(crate::system_proxy::drift_text(&drift))
                        .small()
                        .strong()
                        .color(theme::warning()),
                );
            });
        }

        // 测速全部：走的就是订阅源页/节点页那条 `speedtest_all_sources`，不另开一条通道。
        // 并发用系统设置里那个数（默认 128），批量作业自带 TCP 预筛。
        let testing = self.busy;
        if ui
            .add_enabled(!testing, egui::Button::new("测速全部"))
            .on_hover_text(if testing {
                "已经有任务在跑，等它结束再测速"
            } else {
                "对所有订阅节点跑一轮批量测速：先 TCP 预筛，再逐个真实握手测延迟"
            })
            .clicked()
        {
            self.speedtest_all_sources();
        }
    }

    pub(crate) fn ui_proxy_nodes(&mut self, ui: &mut Ui) {
        if self.selected_proxy_source_id.as_deref() != Some("all") {
            self.selected_proxy_source_id = Some("all".to_string());
            self.reload_proxy_nodes();
        }

        // 流量图缩进顶部卡片第二行的右侧了，这一页不再画大图 —— tab 行直接顶上来。
        ui.add_space(2.0);
        // 地区筛选。原来的「一键连接最优」按钮按用户要求删了 —— 它和底部的「自动选路 url-test」
        // 是同一件事的两种做法，留一个就够。
        //
        // 用 `horizontal_wrapped` 而不是 `horizontal`：动作条现在挂在这行末尾，
        // 8 个地区药丸 + 4 个按钮在 960 的最小窗口宽度下放不下，不换行就会被裁掉
        // ——「停止代理」被裁掉就等于没有。
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("地区:")
                    .small()
                    .color(theme::text_secondary()),
            );
            let regions = [
                ("all", "全部"),
                ("HK", "香港"),
                ("JP", "日本"),
                ("US", "美国"),
                ("SG", "新加坡"),
                ("TW", "台湾"),
                ("KR", "韩国"),
                ("OTHER", "其它"),
            ];
            for (code, label) in regions {
                let is_sel = self.proxy_region_filter == code;
                let btn = if is_sel {
                    egui::Button::new(RichText::new(label).small().color(theme::accent()).strong())
                        .fill(theme::accent_bg())
                        .stroke(Stroke::new(1.0_f32, theme::accent()))
                } else {
                    egui::Button::new(RichText::new(label).small().color(theme::muted()))
                };
                if ui.add(btn).clicked() && !is_sel {
                    self.proxy_region_filter = code.to_string();
                }
            }
        });

        ui.add_space(4.0);
        // 行2：排序、节点数统计与搜索（独立行排列，彻底避免覆盖节点总数）
        ui.horizontal(|ui| {
            ui.label(RichText::new("排序:").small().color(theme::muted()));
            let is_lat = self.proxy_sort_by == ProxySortBy::Latency;
            let is_name = self.proxy_sort_by == ProxySortBy::Name;
            if ui
                .add(icons::selectable_icon_btn(Icon::Lightning, "延迟", is_lat))
                .clicked()
            {
                self.proxy_sort_by = ProxySortBy::Latency;
            }
            if ui
                .add(icons::selectable_icon_btn(Icon::Sort, "名称", is_name))
                .clicked()
            {
                self.proxy_sort_by = ProxySortBy::Name;
            }

            ui.separator();
            ui.label(
                RichText::new(format!("共 {} 个节点", self.proxy_nodes.len()))
                    .small()
                    .color(theme::muted()),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // X 必须每帧都占一个 id 槽位。以前它只在搜索框非空时才 add，于是敲下
                // 第一个字符后这一行之后的控件 auto-id 整体位移，TextEdit 换了 id →
                // 焦点被丢、微软拼音刚起的组合窗跟着消失，表现就是"输一个字就卡"。
                let cleared = ui
                    .add_visible(
                        !self.proxy_search.is_empty(),
                        icons::icon_btn_square(Icon::Close, 11.0),
                    )
                    .clicked();
                if cleared {
                    self.proxy_search.clear();
                }
                ui.push_id("proxy_node_search", |ui| {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Search, 13.0, theme::muted());
                        let search_field = ui.add(
                            egui::TextEdit::singleline(&mut self.proxy_search)
                                .hint_text("搜索节点...")
                                .desired_width(120.0),
                        );
                        crate::gui::widgets::note_text_field(ui, &search_field);
                    });
                });
            });
        });
        // 内核此刻到底在用谁：直接读 /proxies 的实测结果，不猜。
        // 只把两个 `now` 取出来，不 clone 整个 groups —— 开了自动选路那是几十个成员名，
        // 每帧深拷一遍纯浪费（这行每帧都画）。
        let proxy_now = self
            .kernel_groups
            .iter()
            .find(|group| group.name == "PROXY")
            .and_then(|group| group.now.clone());
        let final_now = self
            .kernel_groups
            .iter()
            .find(|group| group.name == FINAL_GROUP)
            .and_then(|group| group.now.clone());
        if !self.kernel_groups.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("内核实际选路").small().color(theme::muted()));
                match &proxy_now {
                    Some(now) => {
                        ui.label(
                            RichText::new(format!("PROXY → {now}"))
                                .small()
                                .color(theme::text_secondary()),
                        );
                    }
                    None => {
                        ui.label(
                            RichText::new("等待内核上报...")
                                .small()
                                .color(theme::muted()),
                        );
                    }
                }
                if let Some(final_now) = &final_now {
                    ui.label(
                        RichText::new(format!("· 漏网之鱼 → {final_now}"))
                            .small()
                            .color(theme::muted()),
                    );
                }
            });
        }
        ui.add_space(6.0);

        // 网格视图按 (数据版本, 地区, 搜索词, 排序) 缓存，只有这四样变了才重算 ——
        // 以前每帧克隆 + 过滤 + 排序约 500 个节点，比较器里还现分配小写字符串。
        let region_key = self.proxy_region_filter.clone();
        let search_key = self.proxy_search.trim().to_lowercase();
        let sort_key = self.proxy_sort_by;
        let rev = self.proxy_nodes_rev;
        let view = match self.proxy_view.take() {
            Some(cached)
                if cached.rev == rev
                    && cached.region == region_key
                    && cached.search == search_key
                    && cached.sort == sort_key =>
            {
                cached
            }
            _ => build_proxy_view(&self.proxy_nodes, &region_key, &search_key, sort_key, rev),
        };

        let auto_routing = self.proxy_config.url_test;
        let kernel_egress = auto_routing.then(|| self.kernel_egress_name()).flatten();
        let mut selected = None;
        let mut locked: Option<(String, String)> = None;
        let columns = if ui.available_width() >= 920.0 {
            4
        } else if ui.available_width() >= 660.0 {
            3
        } else {
            2
        };

        let total_card_rows = (view.rows.len() + columns - 1) / columns;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 84.0, total_card_rows, |ui, row_range| {
                if view.rows.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.label(
                            RichText::new("没有匹配的可用节点，请在「节点列表」中进行测速")
                                .color(theme::muted()),
                        );
                    });
                    return;
                }
                for row_idx in row_range {
                    let start = row_idx * columns;
                    let end = (start + columns).min(view.rows.len());
                    let row = &view.rows[start..end];
                    ui.columns(columns, |column_uis| {
                        for (column_index, column_ui) in column_uis.iter_mut().enumerate() {
                            let Some(entry) = row.get(column_index) else {
                                continue;
                            };
                            let node = &entry.node;
                            // 自动选路开着时，"当前用的是谁"由内核决定：高亮跟着内核的
                            // 出口走，左键点卡片不再改出口（要固定得右键显式锁定）。
                            let is_selected = if auto_routing {
                                kernel_egress.as_deref() == Some(node.name.as_str())
                            } else {
                                self.selected_proxy_node_id.as_deref() == Some(&node.id)
                            };
                            let is_switching = self.switching_node_id.as_deref() == Some(&node.id);
                            let response = compact_node_card(
                                column_ui,
                                node,
                                is_selected,
                                is_switching,
                            )
                            .on_hover_text(if auto_routing {
                                "自动选路中：出口由内核决定。右键可「锁定为出口」（会关掉自动选路）/ 查看详情 / 测速 / IP 检测"
                            } else {
                                "点击连接，右键查看详情 / 测速 / IP 检测"
                            });
                            let connect = response.clicked() && !auto_routing;
                            if response.clicked() && auto_routing && !is_selected {
                                self.status = "自动选路中：出口由内核决定，要固定这个节点请右键「锁定为出口」".into();
                            }
                            let node_id = node.id.clone();
                            let node_name = node.name.clone();
                            response.context_menu(|ui| {
                                if ui.button("查看详情").clicked() {
                                    self.show_node(node_id.clone());
                                    ui.close_menu();
                                }
                                if ui
                                    .add_enabled(!self.busy, egui::Button::new("测速节点"))
                                    .clicked()
                                {
                                    self.speedtest_one(node_id.clone());
                                    ui.close_menu();
                                }
                                if ui.button("IP 检测").clicked() {
                                    self.check_ip_risk_for(&node_id);
                                    ui.close_menu();
                                }
                                if auto_routing && !is_selected {
                                    ui.separator();
                                    if ui.button("锁定为出口（关闭自动选路）").clicked() {
                                        locked = Some((node_id.clone(), node_name.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                            if connect && !is_selected {
                                selected = Some((node.id.clone(), node.name.clone()));
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });
        if let Some((id, name)) = selected {
            self.switch_proxy_node(id, name, "手动切换");
        }
        self.proxy_view = Some(view);
        if let Some((id, name)) = locked {
            // 两个开关不该同时表达"我要怎么选路"：显式锁定就把自动选路关掉，
            // 想回到全自动再自己勾回来（或点「交给自动选择」）。
            self.proxy_config.url_test = false;
            self.save_proxy_config();
            self.status = format!("已关闭自动选路，出口锁定为 {name}");
            self.switch_proxy_node(id, name, "手动切换");
        }
    }

    /// url-test 开着时真正的出口：`PROXY.now` 若指向「自动选择」，再取该组选中的成员。
    fn kernel_egress_name(&self) -> Option<String> {
        let proxy = self
            .kernel_groups
            .iter()
            .find(|group| group.name == "PROXY")?;
        let now = proxy.now.clone()?;
        if now != URL_TEST_GROUP {
            return Some(now);
        }
        self.kernel_groups
            .iter()
            .find(|group| group.name == URL_TEST_GROUP)
            .and_then(|group| group.now.clone())
    }

    /// 「下载内核」弹框：镜像竞速 / 官方直连 / 走代理 / 本地导入 四种方式（防火墙放行开关在系统设置页）。
    ///
    /// 宽度必须写死：窗宽不足时提示会被压成一竖排字；四行也不用 `selectable_value`
    /// 而是自绘整行，这样底色/描边/提示换行才一致。
    pub(crate) fn ui_kernel_dialog(&mut self, ctx: &egui::Context) {
        use crate::kernel::{source_from_id, KernelSource, SOURCE_OPTIONS};
        if !self.show_kernel_dialog {
            return;
        }
        let Some(kind) = self.kernel_dialog_kind else {
            return;
        };
        let mut open = true;
        let mut start: Option<(KernelSource, Option<std::path::PathBuf>)> = None;

        const SOURCE_W: f32 = 424.0;
        const PAD: f32 = 11.0;
        const DOT_W: f32 = 22.0;

        egui::Window::new(format!("下载内核 · {}", kind.display_name()))
            .id(egui::Id::new("kernel-download-dialog"))
            .collapsible(false)
            .resizable(false)
            .default_width(SOURCE_W)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 把 zip 直接拖进窗口就能当本地导入的路径，省得他手打。
                let dropped: Vec<std::path::PathBuf> = ui.input(|input| {
                    input
                        .raw
                        .dropped_files
                        .iter()
                        .filter_map(|file| file.path.clone())
                        .collect()
                });
                for path in dropped {
                    let is_zip = path
                        .extension()
                        .map(|ext| ext.eq_ignore_ascii_case("zip"))
                        .unwrap_or(false);
                    if is_zip {
                        self.kernel_dialog_path = path.display().to_string();
                        self.kernel_dialog_source = "local".to_string();
                    }
                }

                ui.set_width(SOURCE_W);
                ui.label(
                    RichText::new("下载方式")
                        .strong()
                        .size(15.0)
                        .color(theme::text_primary()),
                );
                for option in SOURCE_OPTIONS {
                    let selected = self.kernel_dialog_source == option.id;
                    let mut job = egui::text::LayoutJob::default();
                    job.break_on_newline = true;
                    job.wrap.max_width = SOURCE_W - 2.0 * PAD - DOT_W;
                    job.append(
                        option.label,
                        0.0,
                        egui::text::TextFormat::simple(
                            egui::FontId::proportional(14.0),
                            if selected {
                                theme::accent_strong()
                            } else {
                                theme::text_primary()
                            },
                        ),
                    );
                    job.append(
                        &format!("\n{}", option.hint),
                        0.0,
                        egui::text::TextFormat::simple(
                            egui::FontId::proportional(11.5),
                            theme::text_secondary(),
                        ),
                    );
                    let galley = ui.painter().layout_job(job);
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(SOURCE_W, galley.size().y + 2.0 * PAD),
                        Sense::click(),
                    );
                    if response.clicked() {
                        self.kernel_dialog_source = option.id.to_string();
                    }
                    let fill = if selected {
                        theme::accent_bg()
                    } else if response.hovered() {
                        theme::panel_hover()
                    } else {
                        theme::panel()
                    };
                    // 未选中的行也是白底 + 冷调细边，不用灰底铺满整列。
                    let stroke = if selected {
                        Stroke::new(1.0_f32, theme::accent())
                    } else {
                        Stroke::new(1.0_f32, theme::border())
                    };
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(theme::RADIUS_BTN),
                        fill,
                        stroke,
                        egui::epaint::StrokeKind::Inside,
                    );
                    let dot = egui::pos2(rect.min.x + PAD + 5.0, rect.min.y + PAD + 8.0);
                    if selected {
                        ui.painter()
                            .circle_stroke(dot, 5.0, Stroke::new(1.6_f32, theme::accent()));
                        ui.painter().circle_filled(dot, 2.3_f32, theme::accent());
                    } else {
                        ui.painter()
                            .circle_stroke(dot, 5.0, Stroke::new(1.2_f32, theme::muted()));
                    }
                    ui.painter().galley(
                        egui::pos2(rect.min.x + PAD + DOT_W, rect.min.y + PAD - 1.0),
                        galley,
                        theme::text_primary(),
                    );
                    ui.add_space(4.0);
                }
                if self.kernel_dialog_source == "local" {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("本地 zip（也可以把文件拖进这个窗口）")
                            .small()
                            .color(theme::text_secondary()),
                    );
                    let path_field = ui.add(
                        egui::TextEdit::singleline(&mut self.kernel_dialog_path)
                            .desired_width(SOURCE_W - 4.0)
                            .hint_text(r"D:\downloads\mihomo-windows-amd64-v3-v1.19.32.zip"),
                    );
                    crate::gui::widgets::note_text_field(ui, &path_field);
                }

                ui.add_space(6.0);
                ui.separator();
                let source = source_from_id(&self.kernel_dialog_source);
                let ready =
                    source != KernelSource::Local || !self.kernel_dialog_path.trim().is_empty();
                ui.horizontal(|ui| {
                    if !ready {
                        ui.label(
                            RichText::new("先选好本地 zip 文件")
                                .small()
                                .color(theme::warning()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(ready && !self.kernel_busy, primary_btn("开始下载"))
                            .clicked()
                        {
                            let path = (source == KernelSource::Local)
                                .then(|| std::path::PathBuf::from(self.kernel_dialog_path.trim()));
                            start = Some((source, path));
                            open = false;
                        }
                        if ui.button("关闭").clicked() {
                            open = false;
                        }
                    });
                });
            });

        self.show_kernel_dialog = open;
        if let Some((source, path)) = start {
            self.update_kernel_from(kind, source, path, true);
        }
    }

    pub(crate) fn ui_proxy_connections(&mut self, ui: &mut Ui) {
        section_title(
            ui,
            "活跃连接监控",
            "实时查看走代理与直连的进程、目标域名及传输流量，支持一键断开异常连接",
        );

        let mut close_id = None;
        let mut close_all = false;

        ui.horizontal_wrapped(|ui| {
            ui.label("筛选");
            let conn_filter_field = ui.add(
                egui::TextEdit::singleline(&mut self.connections_filter)
                    .hint_text("按进程、域名或 IP 过滤")
                    .desired_width(180.0),
            );
            crate::gui::widgets::note_text_field(ui, &conn_filter_field);
            if !self.connections_filter.is_empty() && ui.button("清空").clicked() {
                self.connections_filter.clear();
            }
            ui.separator();
            ui.label(format!("当前连接总数: {}", self.active_connections.len()));
            ui.separator();
            let running = self.proxy_status.state == ProxyState::Running;
            if ui
                .add_enabled(
                    running && !self.active_connections.is_empty(),
                    danger_btn("断开全部连接"),
                )
                .on_hover_text("向内核发送指令关闭所有活动的网络连接")
                .clicked()
            {
                close_all = true;
            }
        });
        ui.add_space(8.0);

        if self.proxy_status.state != ProxyState::Running {
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("代理未在运行中，暂无活跃连接数据").color(theme::muted()));
            });
            return;
        }

        let filter = self.connections_filter.trim().to_lowercase();
        let filtered: Vec<&ActiveConnectionInfo> = self
            .active_connections
            .iter()
            .filter(|c| {
                if filter.is_empty() {
                    true
                } else {
                    c.process.to_lowercase().contains(&filter)
                        || c.host.to_lowercase().contains(&filter)
                        || c.destination.to_lowercase().contains(&filter)
                        || c.rule.to_lowercase().contains(&filter)
                }
            })
            .collect();

        if filtered.is_empty() {
            card_frame().show(ui, |ui| {
                ui.label(
                    RichText::new(if self.active_connections.is_empty() {
                        "当前暂无活动的网络连接"
                    } else {
                        "没有匹配筛选条件的网络连接"
                    })
                    .color(theme::muted()),
                );
            });
            return;
        }

        panel_frame().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("active_connections_grid")
                        .striped(true)
                        .min_col_width(60.0)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            header_cell(ui, "网络");
                            header_cell(ui, "进程");
                            header_cell(ui, "目标地址");
                            header_cell(ui, "分流规则");
                            header_cell(ui, "上传");
                            header_cell(ui, "下载");
                            header_cell(ui, "操作");
                            ui.end_row();

                            for c in filtered {
                                ui.label(RichText::new(&c.network).small());
                                ui.label(RichText::new(&c.process).strong().small());
                                let target_text = if !c.host.is_empty() {
                                    format!("{} ({})", c.host, c.destination)
                                } else {
                                    c.destination.clone()
                                };
                                ui.label(RichText::new(target_text).small());
                                ui.label(RichText::new(&c.rule).small().color(theme::accent()));
                                ui.label(
                                    RichText::new(format_bytes(c.upload))
                                        .small()
                                        .color(theme::muted()),
                                );
                                ui.label(
                                    RichText::new(format_bytes(c.download))
                                        .small()
                                        .color(theme::muted()),
                                );
                                if ui
                                    .add(danger_btn("断开"))
                                    .on_hover_text("断开此连接")
                                    .clicked()
                                {
                                    close_id = Some(c.id.clone());
                                }
                                ui.end_row();
                            }
                        });
                });
        });

        if let Some(id) = close_id {
            self.close_connection(id);
        } else if close_all {
            self.close_all_connections();
        }
    }

    /// 分流规则面板：拆成「规则配置」和「命中记录」两个分区。
    /// 导入/导出挂在配置那一侧 —— 它们动的是规则表本身。
    pub(crate) fn ui_proxy_rules(&mut self, ui: &mut Ui) {
        // 两个闭包都要动 self，所以 tab 状态先取副本、动作先记成枚举，出框后再落地。
        let mut tab = self.rules_tab;
        // 动作组按"这一帧进来时"的分区决定画不画；`tab` 被左闭包可变借用，这里另存一份。
        let tab_now = self.rules_tab;
        let busy = self.proxy_busy;
        let mut action = RulesHeaderAction::None;
        // 系统显示缩放 125%/150% 叠上字体 140% 时，960 物理最小宽只剩 550~460 逻辑点，
        // 中央面板不到 420 —— 一行放不下"两个分区标签 + 250 宽的动作组"，右对齐那组会
        // 直接压到标签上（2026-10-15 他报的"导出 JSON 被挤出去"）。放不下就让它换一行。
        let one_row_fits =
            ui.max_rect().width() >= RULES_ACTIONS_WIDTH + RULES_TABS_MIN_WIDTH + 8.0;
        if one_row_fits {
            crate::gui::widgets::split_row(
                ui,
                "rules:tabs",
                RULES_ACTIONS_WIDTH,
                |ui| {
                    panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                    panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
                },
                |ui| Self::rules_actions(ui, tab_now, busy, &mut action),
            );
        } else {
            let _ = ui.horizontal(|ui| {
                panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
            });
            // 连一行都放不下时（200% 系统缩放 + 140% 字体）让它自己折，别越界。
            let _ = ui.horizontal_wrapped(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Self::rules_actions(ui, tab_now, busy, &mut action);
                });
            });
        }
        self.rules_tab = tab;
        match action {
            RulesHeaderAction::None => {}
            RulesHeaderAction::Export => self.export_rules_as_json(),
            RulesHeaderAction::Import => self.show_rule_import = true,
        }
        ui.add_space(8.0);
        match self.rules_tab {
            RulesTab::Config => self.ui_proxy_rules_config(ui),
            RulesTab::Hits => self.ui_proxy_rules_hits(ui),
        }
        self.ui_rule_import(ui.ctx());
        self.ui_rule_import_help(ui.ctx());
    }

    fn button_small(text: &str) -> egui::Button<'_> {
        egui::Button::new(RichText::new(text).strong().size(12.5))
    }

    /// 分流规则表头右侧那组动作（导出 JSON / 导入规则）。
    /// 两个闭包都要碰 `self`，所以这里只把点击记进 `action`，由调用方出框后再执行。
    fn rules_actions(ui: &mut Ui, tab: RulesTab, busy: bool, action: &mut RulesHeaderAction) {
        if tab != RulesTab::Config {
            return;
        }
        if ui
            .button("导出 JSON")
            .on_hover_text(
                "把当前规则表导成 mihomo 规则字符串数组：写进 data/exports 并复制到剪贴板",
            )
            .clicked()
        {
            *action = RulesHeaderAction::Export;
        }
        if ui
            .add_enabled(!busy, Self::button_small("导入规则"))
            .on_hover_text("贴 JSON 或一行一条的文本，解析后追加进规则表")
            .clicked()
        {
            *action = RulesHeaderAction::Import;
        }
    }

    /// 命中记录：规则列表说的是"应该走哪"，这张表说的是"实际走了哪"。
    /// 连接是快照不是事件流，所以统计从本进程启动开始，重启前的那部分没有。
    fn ui_proxy_rules_hits(&mut self, ui: &mut Ui) {
        let total: i64 = self.rule_hits_today.iter().map(|hit| hit.hits).sum();
        let matched: i64 = self
            .rule_hits_today
            .iter()
            .filter(|hit| hit.rule.starts_with("MATCH"))
            .map(|hit| hit.hits)
            .sum();
        let pct = if total > 0 { matched * 100 / total } else { 0 };
        let empty = self.rule_hits_today.is_empty();
        let mut refresh = false;
        card_frame().show(ui, |ui| {
            // 说明并进表头那一行（原来它单独占一行 + 8pt 间隔），滚动区再吃满剩余高度，
            // 列表视口从 469 涨到 534（1330×800）、240 涨到 305（140% 字体）。
            crate::gui::widgets::split_row(
                ui,
                "rules:hits_header",
                96.0,
                |ui| {
                    icon_label(ui, Icon::Radar, 16.0, theme::accent());
                    ui.label(RichText::new("规则命中记录").strong().size(15.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{total} 条连接 · 兜底 MATCH {matched} 条（{pct}%） · \
                                 占比按连接数算，一条连接只计一次"
                            ))
                            .small()
                            .color(theme::text_secondary()),
                        )
                        .truncate(),
                    )
                    .on_hover_text("统计从本进程启动开始；命中每 30 秒自动落库一次。");
                },
                |ui| {
                    if ui
                        .button("立即刷新")
                        .on_hover_text("命中每 30 秒自动落库一次，这里手动拉最新那份")
                        .clicked()
                    {
                        refresh = true;
                    }
                },
            );
            ui.add_space(6.0);
            if empty {
                ui.label(
                    RichText::new("还没有命中记录：启动代理并跑点流量之后再回来看。")
                        .color(theme::muted()),
                );
                return;
            }
            ui.horizontal(|ui| {
                header_cell(ui, "命中的规则");
                header_cell(ui, "派给");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    header_cell(ui, "条数 / 占比");
                });
            });
            egui::ScrollArea::vertical()
                .id_salt("rule_hits_table")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for hit in &self.rule_hits_today {
                        let share = if total > 0 { hit.hits * 100 / total } else { 0 };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&hit.rule).color(theme::accent()).monospace());
                            ui.label(RichText::new(format!("→ {}", hit.target)));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("{} 条 · {}%", hit.hits, share))
                                            .strong()
                                            .color(theme::text_primary()),
                                    );
                                },
                            );
                        });
                    }
                });
        });
        if refresh {
            self.reload_rule_hits();
        }
    }

    pub(crate) fn ui_proxy_rules_config(&mut self, ui: &mut Ui) {
        let editable = !self.proxy_busy;
        let show_custom_rules = self.proxy_config.routing_mode == RoutingMode::Rule;
        let mut save_mode_clicked = false;
        let mut save_clicked = false;
        let mut add_rule = false;
        let mut add_recommended = false;
        let mut optimize_rules = false;
        let mut move_rule: Option<(usize, usize)> = None;
        let mut remove_rule = None;

        // 整个面板一起滚：模式卡 + Geo 卡 + 预设卡本身就占掉大半屏，只给规则列表配
        // 滚动条的话，下面的卡片与"保存规则"按钮仍然会被挤出可视区。
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section_title(
                    ui,
                    "分流模式",
                    routing_mode_description(self.proxy_config.routing_mode),
                );
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Rule,
                                "自定义规则",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::ChinaDirect,
                                "中国大陆直连",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::GfwList,
                                "GFW 列表代理",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Global,
                                "全局代理",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Direct,
                                "全部直连",
                            );
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(routing_mode_detail(self.proxy_config.routing_mode))
                                .small()
                                .color(theme::muted()),
                        );
                        ui.add_space(8.0);
                        save_mode_clicked = ui.add(primary_btn("保存分流模式")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理配置中...").color(theme::muted()));
                    }
                });
                ui.add_space(8.0);
                card_frame().show(ui, |ui| {
                    // 描述与按钮分两行：横向并排时长描述会把按钮顶出可视区。
                    ui.label(RichText::new("Geo 规则数据库维护").strong());
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "在线更新 GeoIP (中国大陆IP库) 与 GeoSite (域名分流库) 规则文件",
                            )
                            .small()
                            .color(theme::muted()),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let running = self.proxy_status.state == ProxyState::Running;
                        if ui
                            .add_enabled(
                                running && !self.geo_updating,
                                primary_btn("一键更新 Geo 规则"),
                            )
                            .on_hover_text(
                                "向内核发送指令在线拉取并重新载入最新的 GeoIP 与 GeoSite 数据",
                            )
                            .clicked()
                        {
                            self.update_geo_data();
                        }
                        if self.geo_updating {
                            ui.spinner();
                            ui.label(RichText::new("正在更新 Geo 规则...").small().color(theme::accent()));
                        }
                    });
                });

                if !show_custom_rules {
                    return;
                }

                ui.add_space(8.0);
                card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Lightbulb, 15.0, theme::accent());
                        ui.label(RichText::new("常用场景分流预设（一键添加至规则列表）").strong());
                    });
                    ui.label(
                        RichText::new(
                            "点击对应场景可一键合并预设分流规则，并在保存后自动热重载生效",
                        )
                        .small()
                        .color(theme::muted()),
                    );
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "AI 服务分流 (ChatGPT / Claude / Gemini)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 OpenAI、Claude、Gemini、Copilot 域名走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in ai_service_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "海外主流流媒体 (YouTube / Netflix / Spotify)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 YouTube、Netflix、Disney+、Spotify 走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in streaming_media_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "即时通讯 (Telegram / Discord / X)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 Telegram 域名与 IP 段、Discord、X 走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in social_telegram_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "国内开发镜像直连 (npm / 清华源 / 阿里源)",
                                theme::success(),
                            ))
                            .on_hover_text("国内常见开源包与镜像站直连加速")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in dev_mirrors_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "游戏平台直连 (Steam / Epic)",
                                theme::success(),
                            ))
                            .on_hover_text("Steam 下载 CDN 与游戏商店直连")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in game_platforms_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Shield,
                                "广告与 QUIC 拦截",
                                theme::danger(),
                            ))
                            .on_hover_text("拦截常见广告并阻止 QUIC 降速")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in adblock_quic_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                    });
                });

                ui.add_space(8.0);
                section_title(ui, "自定义规则", "从上到下匹配，未命中流量默认走代理");
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        for index in 0..self.proxy_config.rules.len() {
                            let matcher =
                                rule_matcher_label(&self.proxy_config.rules[index].matcher);
                            ui.horizontal(|ui| {
                                ui.add_sized([260.0, 22.0], egui::Label::new(matcher));
                                egui::ComboBox::from_id_salt(("rule_action", index))
                                    .selected_text(rule_action_label(
                                        self.proxy_config.rules[index].action,
                                    ))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Direct,
                                            "直连",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Proxy,
                                            "代理",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Reject,
                                            "拒绝",
                                        );
                                    });
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑"))
                                    .on_hover_text("上移")
                                    .clicked()
                                {
                                    move_rule = Some((index, index - 1));
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < self.proxy_config.rules.len(),
                                        egui::Button::new("↓"),
                                    )
                                    .on_hover_text("下移")
                                    .clicked()
                                {
                                    move_rule = Some((index, index + 1));
                                }
                                if ui.button("×").on_hover_text("删除").clicked() {
                                    remove_rule = Some(index);
                                }
                            });
                        }

                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("new_rule_kind")
                                .selected_text(self.new_rule_kind.label())
                                .show_ui(ui, |ui| {
                                    for kind in RuleDraftKind::ALL {
                                        ui.selectable_value(
                                            &mut self.new_rule_kind,
                                            kind,
                                            kind.label(),
                                        );
                                    }
                                });
                            if self.new_rule_kind.needs_value() {
                                let rule_value_field = ui.add_sized(
                                    [220.0, 24.0],
                                    egui::TextEdit::singleline(&mut self.new_rule_value)
                                        .hint_text("匹配值"),
                                );
                                crate::gui::widgets::note_text_field(ui, &rule_value_field);
                            }
                            egui::ComboBox::from_id_salt("new_rule_action")
                                .selected_text(rule_action_label(self.new_rule_action))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Direct,
                                        "直连",
                                    );
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Proxy,
                                        "代理",
                                    );
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Reject,
                                        "拒绝",
                                    );
                                });
                            add_rule = ui.button("添加规则").clicked();
                            ui.menu_button("⚡ 嗅探活动进程 ▼", |ui| {
                                ui.set_min_width(220.0);
                                ui.label(RichText::new("当前系统运行的应用进程").strong().color(theme::text_secondary()));
                                ui.separator();
                                let procs = crate::platform::list_running_processes();
                                if procs.is_empty() {
                                    ui.label(RichText::new("未检测到活跃进程").small().color(theme::muted()));
                                } else {
                                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                        for proc in procs {
                                            if ui.button(&proc).clicked() {
                                                self.new_rule_kind = RuleDraftKind::ProcessName;
                                                self.new_rule_value = proc.clone();
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            });

                            add_recommended = ui
                                .button("推荐分流")
                                .on_hover_text("覆盖为国内直连、广告拒绝、其余代理")
                                .clicked();
                            optimize_rules = ui
                                .button("自动优化")
                                .on_hover_text("移除重复规则并保持当前顺序")
                                .clicked();
                        });
                        if self.new_rule_kind == RuleDraftKind::ProcessName
                            && self.proxy_config.mode != ProxyMode::Tun
                        {
                            ui.label(
                                RichText::new(
                                    "PROCESS-NAME 只有 TUN 模式拿得到进程名，系统代理模式下不会生效",
                                )
                                .small()
                                .color(theme::warning()),
                            );
                        }
                        ui.label(
                            RichText::new("推荐规则：局域网与国内直连，广告和 QUIC 拒绝，其余代理")
                                .small()
                                .color(theme::muted()),
                        );

                        ui.add_space(8.0);
                        save_clicked = ui.add(primary_btn("保存规则")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理规则中...").color(theme::muted()));
                    }
                });

                let injected = mihomo_dns_poison_overrides()
                    .iter()
                    .filter_map(|rule| rule.to_mihomo_rule().ok())
                    .collect::<Vec<_>>();
                if !injected.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "内置注入（不可编辑，仅 Mihomo 生效，插在 GEOIP,CN 之前）：{}",
                            injected.join(" · ")
                        ))
                        .small()
                        .color(theme::muted()),
                    );
                }
            });

        if save_mode_clicked {
            self.save_proxy_config();
        }
        if let Some((from, to)) = move_rule {
            self.proxy_config.rules.swap(from, to);
        }
        if let Some(index) = remove_rule {
            self.proxy_config.rules.remove(index);
        }
        if add_rule {
            let rule = TrafficRule {
                matcher: self.new_rule_kind.matcher(&self.new_rule_value),
                action: self.new_rule_action,
            };
            match rule.to_mihomo_rule() {
                Ok(_) => {
                    self.proxy_config.rules.push(rule);
                    self.new_rule_value.clear();
                }
                Err(error) => self.status = format!("规则无效: {error}"),
            }
        }
        if add_recommended {
            self.show_recommended_rules_confirm = true;
        }
        if optimize_rules {
            let before = self.proxy_config.rules.len();
            self.proxy_config.rules = optimize_traffic_rules(&self.proxy_config.rules);
            let removed = before.saturating_sub(self.proxy_config.rules.len());
            self.status = if removed == 0 {
                "规则已经是最优顺序，没有发现重复项".into()
            } else {
                format!("已自动移除 {removed} 条重复规则，请保存规则")
            };
        }
        if save_clicked {
            self.save_proxy_config();
        }
        self.ui_recommended_rules_confirm(ui.ctx());
    }

    /// 导出当前规则表：写一份 JSON 到 `data/exports/`，同时复制到剪贴板。
    /// 没有原生"另存为"对话框（不为此引依赖），所以走"文件 + 剪贴板"两条路，
    /// 哪条成了/没成都写在状态栏里，不假装成功。
    pub(crate) fn export_rules_as_json(&mut self) {
        let rules = self.proxy_config.rules.clone();
        let count = rules.len();
        let json = match crate::proxy::rule_io::export_json(&rules) {
            Ok(json) => json,
            Err(error) => {
                self.status = format!("导出失败：{error}");
                return;
            }
        };
        let directory = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("data")
            .join("exports");
        let name = format!(
            "rules-{}-{}.json",
            crate::app_update::current_version(),
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let written = std::fs::create_dir_all(&directory)
            .and_then(|_| std::fs::write(directory.join(&name), &json))
            .map(|_| directory.join(name))
            .map_err(|error| error.to_string());
        let copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(json))
            .map_err(|error| error.to_string());
        let mut parts = vec![format!("导出 {count} 条规则")];
        parts.push(match &written {
            Ok(path) => format!("已写入 {}", path.display()),
            Err(error) => format!("写文件失败：{error}"),
        });
        parts.push(match copied {
            Ok(_) => "并复制到剪贴板".to_string(),
            Err(error) => format!("但剪贴板复制失败：{error}"),
        });
        self.status = parts.join(" · ");
    }

    /// 「导入规则」弹框：贴文本 → 当场给出解析预览 → 才让追加。
    /// 预览这一步不是客气：导入直接改流量走向，不能点下去才知道进了什么。
    pub(crate) fn ui_rule_import(&mut self, ctx: &egui::Context) {
        if !self.show_rule_import {
            return;
        }
        let mut open = true;
        let mut apply = false;
        // `Window::open` 已经借走了 `open`，闭包里只能记一笔，出框之后再关。
        let mut cancel = false;
        egui::Window::new("导入分流规则")
            .id(egui::Id::new("vpn-parser:rule-import"))
            .default_width(680.0)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "支持三种形状：JSON 数组、带 rules/prependRules 字段的 JSON 对象、\
                         以及一行一条的文本（可带引号、尾逗号与 // 注释）。\
                         目标写自定义组名的，落到本程序的代理组。",
                    )
                    .small()
                    .color(theme::muted()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("从剪贴板读取")
                        .on_hover_text("整段粘进来即可，不用先摘引号")
                        .clicked()
                    {
                        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                            Ok(text) => {
                                self.rule_import_text = text.clone();
                                self.rule_import_preview =
                                    Some(crate::proxy::rule_io::parse_rules_text(&text));
                            }
                            Err(error) => self.status = format!("读剪贴板失败：{error}"),
                        }
                    }
                    // 「?」贴着「从剪贴板读取」：示范段落就是给人抄进这个框的。
                    let help = ui.add(
                        egui::Button::new(
                            RichText::new("?")
                                .strong()
                                .size(12.5)
                                .color(theme::accent()),
                        )
                        .min_size(Vec2::new(22.0, 22.0))
                        .fill(Color32::TRANSPARENT),
                    );
                    if help
                        .on_hover_text("导入格式示范：四种输入形状 + 支持/会跳过的类型")
                        .clicked()
                    {
                        self.show_rule_import_help = true;
                    }
                    if !self.rule_import_text.is_empty() && ui.button("清空").clicked() {
                        self.rule_import_text.clear();
                        self.rule_import_preview = None;
                    }
                });
                let field = ui.add(
                    egui::TextEdit::multiline(&mut self.rule_import_text)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("DOMAIN-SUFFIX,example.com,PROXY"),
                );
                crate::gui::widgets::note_text_field(ui, &field);
                if field.changed() {
                    let text = self.rule_import_text.clone();
                    self.rule_import_preview = Some(crate::proxy::rule_io::parse_rules_text(&text));
                }
                let preview = self.rule_import_preview.clone();
                match preview {
                    Some(preview) => {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(preview.summary())
                                .strong()
                                .color(theme::text_primary()),
                        );
                        if !preview.rules.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("rule_import_preview")
                                .max_height(140.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for line in crate::proxy::rule_io::export_lines(&preview.rules)
                                        .unwrap_or_default()
                                    {
                                        ui.label(RichText::new(line).small().monospace());
                                    }
                                });
                        }
                        if !preview.unparsed.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "认不出的前 {} 条：{}",
                                    preview.unparsed.len().min(3),
                                    preview
                                        .unparsed
                                        .iter()
                                        .take(3)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(" / ")
                                ))
                                .small()
                                .color(theme::warning()),
                            );
                        }
                        apply = ui
                            .add_enabled(
                                !preview.rules.is_empty() && !self.proxy_busy,
                                primary_btn("追加到规则表并保存"),
                            )
                            .on_hover_text("已存在的同样规则会跳过；保存时照常去重")
                            .clicked();
                    }
                    None => {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("粘贴或从剪贴板读取后，这里会先给解析结果再让导入。")
                                .small()
                                .color(theme::muted()),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if apply {
            self.apply_rule_import();
        }
        if !open {
            self.show_rule_import = false;
            self.rule_import_preview = None;
        }
    }

    /// 导入框旁边那个「?」：把每种能吃的输入形状各给一段可直接用的示范。
    /// 示范不是手写的漂亮话 —— `rule_io` 里有测试把每段喂回解析器，条数对不上就红，
    /// 所以照着抄不会错；「填入导入框」还会立刻开出解析预览让他先看清进了什么。
    pub(crate) fn ui_rule_import_help(&mut self, ctx: &egui::Context) {
        if !self.show_rule_import_help {
            return;
        }
        let (msg, _rect) = ui_rule_import_help_window(ctx);
        match msg {
            RuleHelpMsg::Nothing => {}
            RuleHelpMsg::Close => self.show_rule_import_help = false,
            RuleHelpMsg::Status(text) => self.status = text,
            RuleHelpMsg::Fill(body) => {
                self.rule_import_text = body.to_string();
                self.rule_import_preview = Some(crate::proxy::rule_io::parse_rules_text(
                    &self.rule_import_text,
                ));
                // 填完直接把导入框开出来：他要看到的就是"这段抄进去到底进了什么"。
                self.show_rule_import = true;
                self.show_rule_import_help = false;
            }
        }
    }

    fn apply_rule_import(&mut self) {
        let Some(preview) = self.rule_import_preview.clone() else {
            return;
        };
        let mut added = 0usize;
        for rule in &preview.rules {
            if self
                .proxy_config
                .rules
                .iter()
                .any(|existing| existing == rule)
            {
                continue;
            }
            self.proxy_config.rules.push(rule.clone());
            added += 1;
        }
        let duplicates = preview.rules.len() - added;
        self.status = format!(
            "导入完成：新增 {added} 条，跳过重复 {duplicates} 条，规则表现共 {} 条",
            self.proxy_config.rules.len()
        );
        self.show_rule_import = false;
        self.rule_import_text.clear();
        self.rule_import_preview = None;
        self.save_proxy_config();
    }

    pub(crate) fn ui_recommended_rules_confirm(&mut self, ctx: &egui::Context) {
        if !self.show_recommended_rules_confirm {
            return;
        }

        let modal = egui::Modal::new(egui::Id::new("recommended_rules_confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("覆盖分流规则");
            ui.label("现有分流规则将被推荐规则覆盖，确认后立即保存。");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    Some(false)
                } else if ui.add(primary_btn("确认覆盖")).clicked() {
                    Some(true)
                } else {
                    None
                }
            })
            .inner
        });

        match modal.inner {
            Some(true) => {
                self.proxy_config.rules = recommended_traffic_rules();
                self.show_recommended_rules_confirm = false;
                self.save_proxy_config();
            }
            Some(false) => self.show_recommended_rules_confirm = false,
            None if modal.should_close() => self.show_recommended_rules_confirm = false,
            None => {}
        }
    }

    pub(crate) fn ui_proxy_network(&mut self, ui: &mut Ui) {
        section_title(ui, "DNS / TUN", "配置接管模式、监听端口与 DNS");
        let editable = !self.proxy_busy && self.proxy_status.config_editable();
        let mut save_clicked = false;
        card_frame().show(ui, |ui| {
            ui.add_enabled_ui(editable, |ui| {
                ui.strong("接管模式");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.proxy_config.mode, ProxyMode::System, "系统代理");
                    ui.selectable_value(&mut self.proxy_config.mode, ProxyMode::Tun, "TUN");
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("运行内核");
                    egui::ComboBox::from_id_salt("proxy_kernel_preference")
                        .selected_text(match self.proxy_config.kernel_preference {
                            KernelPreference::Auto => "自动选择",
                            KernelPreference::Mihomo => "Mihomo",
                            KernelPreference::SingBox => "Sing-box",
                            KernelPreference::Xray => "Xray",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.proxy_config.kernel_preference,
                                KernelPreference::Auto,
                                "自动选择",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.kernel_preference,
                                KernelPreference::Mihomo,
                                "Mihomo",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.kernel_preference,
                                KernelPreference::SingBox,
                                "Sing-box",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.kernel_preference,
                                KernelPreference::Xray,
                                "Xray",
                            );
                        });
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Mixed 端口");
                    ui.add(
                        egui::DragValue::new(&mut self.proxy_config.mixed_port).range(1..=u16::MAX),
                    );
                    ui.label("控制端口");
                    ui.add(
                        egui::DragValue::new(&mut self.proxy_config.controller_port)
                            .range(1..=u16::MAX),
                    );
                    ui.checkbox(&mut self.proxy_config.allow_lan, "允许局域网");
                    ui.checkbox(
                        &mut self.proxy_config.terminal_proxy,
                        "终端代理（写入 HTTP_PROXY / HTTPS_PROXY，Git·curl·npm·pip 才认）",
                    )
                    .on_hover_text(
                        "启动代理时把变量写进 HKCU\\Environment，停止时清除；只清我们自己写的值，\n\
                         你原有的代理变量不会被碰。新开的终端立即生效，已开着的终端要重开。",
                    );
                });

                if self.proxy_config.mode == ProxyMode::Tun {
                    ui.add_space(14.0);
                    ui.strong("TUN 参数");
                    ui.horizontal_wrapped(|ui| {
                        ui.label("协议栈");
                        egui::ComboBox::from_id_salt("tun_stack")
                            .selected_text(tun_stack_label(self.proxy_config.tun.stack))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.proxy_config.tun.stack,
                                    TunStack::Mixed,
                                    "Mixed",
                                );
                                ui.selectable_value(
                                    &mut self.proxy_config.tun.stack,
                                    TunStack::System,
                                    "System",
                                );
                                ui.selectable_value(
                                    &mut self.proxy_config.tun.stack,
                                    TunStack::Gvisor,
                                    "gVisor",
                                );
                            });
                        ui.checkbox(&mut self.proxy_config.tun.auto_route, "自动路由");
                        ui.checkbox(
                            &mut self.proxy_config.tun.auto_detect_interface,
                            "自动检测网卡",
                        );
                    });
                }

                ui.add_space(14.0);
                ui.strong("DNS");
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.proxy_config.dns.enabled, "启用");
                    ui.checkbox(&mut self.proxy_config.dns.ipv6, "IPv6");
                    egui::ComboBox::from_id_salt("dns_enhanced_mode")
                        .selected_text(dns_mode_label(self.proxy_config.dns.enhanced_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.proxy_config.dns.enhanced_mode,
                                DnsEnhancedMode::FakeIp,
                                "Fake-IP",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.dns.enhanced_mode,
                                DnsEnhancedMode::RedirHost,
                                "Redir-Host",
                            );
                        });
                });
                if self.proxy_config.dns.nameservers.is_empty() {
                    self.proxy_config
                        .dns
                        .nameservers
                        .push(crate::proxy::DEFAULT_DNS_PRIMARY.into());
                }
                for (index, nameserver) in self.proxy_config.dns.nameservers.iter_mut().enumerate()
                {
                    let hint = if index == 0 {
                        crate::proxy::DEFAULT_DNS_PRIMARY
                    } else {
                        crate::proxy::DEFAULT_DNS_SECONDARY
                    };
                    ui.horizontal(|ui| {
                        ui.label(format!("DNS {}", index + 1));
                        let ns_field = ui.add_sized(
                            [420.0, 24.0],
                            egui::TextEdit::singleline(nameserver).hint_text(hint),
                        );
                        crate::gui::widgets::note_text_field(ui, &ns_field);
                    });
                }
                ui.add_space(12.0);
                save_clicked = ui.add(primary_btn("保存网络配置")).clicked();
            });
            if !editable {
                ui.label(RichText::new("停止代理后可修改运行配置").color(theme::muted()));
            }
        });
        if save_clicked {
            self.save_proxy_config();
        }
    }

    pub(crate) fn ui_proxy_kernels(&mut self, ui: &mut Ui) {
        section_title(
            ui,
            "代理内核",
            "应用独立下载、校验、备份和更新，不依赖 v2rayN",
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("存储目录 data/kernels · 来源为官方 GitHub Release")
                    .small()
                    .color(theme::muted()),
            );
            if ui
                .add_enabled(!self.kernel_busy, egui::Button::new("检查更新"))
                .clicked()
            {
                self.reload_kernels();
                self.status = "正在检查官方内核版本...".into();
            }
            if self.kernel_busy && ui.add(danger_btn("取消下载")).clicked() {
                self.kernel_cancel.store(true, Ordering::Relaxed);
                self.status = "正在取消内核下载...".into();
            }
        });
        if let Some(progress) = &self.kernel_progress {
            let detail = if progress.stage == crate::kernel::KernelUpdateStage::Downloading {
                format!(
                    "{} / {} · {}/s",
                    format_bytes(progress.downloaded),
                    format_bytes(progress.total),
                    format_bytes(progress.bytes_per_second as u64)
                )
            } else {
                progress.message.clone()
            };
            ui.add(
                egui::ProgressBar::new(progress.fraction())
                    .animate(true)
                    .desired_width(ui.available_width())
                    .text(format!(
                        "{} · {} · {}",
                        progress.kind.display_name(),
                        progress.stage.label(),
                        detail
                    )),
            );
        }
        ui.add_space(6.0);
        let kernels = self.kernels.clone();
        let releases = self.kernel_releases.clone();
        let mut update_kind = None;
        card_frame().show(ui, |ui| {
            let available = ui.available_width();
            let kind_width = 76.0;
            let status_width = 58.0;
            let version_width = (available * 0.25).max(150.0);
            let latest_width = 88.0;
            let action_width = 72.0;
            let path_width = (available
                - kind_width
                - status_width
                - version_width
                - latest_width
                - action_width
                - 52.0)
                .max(150.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [kind_width, 20.0],
                    egui::Label::new(RichText::new("内核").strong().color(theme::muted()).small()),
                );
                ui.add_sized(
                    [status_width, 20.0],
                    egui::Label::new(RichText::new("状态").strong().color(theme::muted()).small()),
                );
                ui.add_sized(
                    [version_width, 20.0],
                    egui::Label::new(
                        RichText::new("当前版本")
                            .strong()
                            .color(theme::muted())
                            .small(),
                    ),
                );
                ui.add_sized(
                    [latest_width, 20.0],
                    egui::Label::new(
                        RichText::new("最新版本")
                            .strong()
                            .color(theme::muted())
                            .small(),
                    ),
                );
                ui.add_sized(
                    [path_width, 20.0],
                    egui::Label::new(RichText::new("路径").strong().color(theme::muted()).small()),
                );
                ui.add_sized(
                    [action_width, 20.0],
                    egui::Label::new(RichText::new("操作").strong().color(theme::muted()).small()),
                );
            });
            ui.separator();
            for kernel in &kernels {
                let version = kernel.version.as_deref().unwrap_or("-");
                let release = releases.iter().find(|release| release.kind == kernel.kind);
                let latest = release
                    .and_then(|release| release.latest_version.as_deref())
                    .unwrap_or("检查中");
                let path = kernel
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".into());
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [kind_width, 24.0],
                        egui::Label::new(RichText::new(kernel.kind.display_name()).strong())
                            .truncate(),
                    );
                    ui.add_sized(
                        [status_width, 24.0],
                        egui::Label::new(
                            RichText::new(if kernel.available {
                                "可用"
                            } else {
                                "未找到"
                            })
                            .color(if kernel.available {
                                theme::success()
                            } else {
                                theme::danger()
                            }),
                        ),
                    );
                    ui.add_sized([version_width, 24.0], egui::Label::new(version).truncate())
                        .on_hover_text(version);
                    ui.add_sized(
                        [latest_width, 24.0],
                        egui::Label::new(RichText::new(latest).color(
                            if release.is_some_and(|release| release.update_available) {
                                theme::warning()
                            } else {
                                theme::muted()
                            },
                        ))
                        .truncate(),
                    );
                    ui.add_sized([path_width, 24.0], egui::Label::new(&path).truncate())
                        .on_hover_text(path);
                    let can_update = release.is_some_and(|release| release.update_available)
                        && !self.kernel_busy
                        && !self.proxy_status.kernel_in_use(kernel.kind);
                    let label = if kernel.available { "更新" } else { "下载" };
                    if ui
                        .add_enabled(
                            can_update,
                            egui::Button::new(label)
                                .min_size(Vec2::new(action_width, 24.0))
                                .fill(if can_update {
                                    theme::accent_bg()
                                } else {
                                    Color32::TRANSPARENT
                                }),
                        )
                        .on_hover_text(if self.proxy_status.kernel_in_use(kernel.kind) {
                            "停止当前代理后才能更新这个内核"
                        } else {
                            "下载官方资产，校验 SHA-256 后备份替换"
                        })
                        .clicked()
                    {
                        update_kind = Some(kernel.kind);
                    }
                });
            }
        });
        if let Some(kind) = update_kind {
            self.open_kernel_download(kind);
        }
    }

    pub(crate) fn ui_proxy_logs(&self, ui: &mut Ui) {
        let kernel_name = self
            .proxy_status
            .kernel
            .map(KernelKind::display_name)
            .unwrap_or("代理内核");
        section_title(ui, "运行日志", &format!("{kernel_name} 最近输出"));
        card_frame().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.proxy_log.is_empty() {
                        ui.label(RichText::new("暂无日志").color(theme::muted()));
                    } else {
                        ui.monospace(&self.proxy_log);
                    }
                });
        });
    }

    /// 落地风险弹窗：查询过程和结论都只在这里显示，主面板不再常驻一行判定。
    pub(crate) fn ui_ip_risk_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_ip_risk_dialog {
            return;
        }
        if self.ip_risk_busy || self.ip_risk_egress_pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        let mut open = self.show_ip_risk_dialog;
        let mut retry = false;
        let mut copy_payload: Option<String> = None;

        egui::Window::new(RichText::new("IP 检测 · 归属与风控画像").strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if self.ip_risk_busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(&self.status).color(theme::text_primary()));
                    });
                    return;
                }

                if let Some(error) = self.ip_risk_error.clone() {
                    Frame::NONE
                        .fill(theme::danger_bg())
                        .stroke(Stroke::new(1.0_f32, theme::danger()))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(RichText::new(error).color(theme::danger()));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "节点地址是域名时查不到归属；也可以重试一次。",
                                )
                                .small()
                                .color(theme::muted()),
                            );
                        });
                    ui.add_space(6.0);
                    if ui.button("重新查询").clicked() {
                        retry = true;
                    }
                    return;
                }

                let Some(risk) = self.ip_risk.clone() else {
                    ui.label(RichText::new("还没有查询结果。").color(theme::muted()));
                    return;
                };

                let color = if risk.datacenter { theme::warning() } else { theme::success() };
                ui.label(
                    RichText::new("入口（节点服务器 IP）")
                        .small()
                        .color(theme::muted()),
                );
                Frame::NONE
                    .fill(if risk.datacenter {
                        theme::warning_bg()
                    } else {
                        theme::success_bg()
                    })
                    .stroke(Stroke::new(1.0_f32, color))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Shield, 18.0, color);
                            ui.label(
                                RichText::new(if risk.datacenter {
                                    "机房 / 云厂商"
                                } else {
                                    "住宅 / ISP"
                                })
                                .strong()
                                .size(15.0)
                                .color(color),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new(&risk.verdict).small().color(theme::text_primary()));
                    });

                ui.add_space(8.0);
                egui::Grid::new("ip_risk_detail")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        if risk.hostname != risk.ip {
                            ui.label(RichText::new("节点地址").small().color(theme::muted()));
                            ui.label(RichText::new(&risk.hostname).color(theme::text_primary()));
                            ui.end_row();
                        }

                        ui.label(
                            RichText::new(if risk.hostname == risk.ip {
                                "落地 IP"
                            } else {
                                "解析到"
                            })
                            .small()
                            .color(theme::muted()),
                        );
                        ui.label(RichText::new(&risk.ip).color(theme::text_primary()));
                        ui.end_row();

                        ui.label(RichText::new("地区").small().color(theme::muted()));
                        ui.label(RichText::new(if risk.country.is_empty() {
                            "未知".to_owned()
                        } else {
                            risk.country.clone()
                        })
                        .color(theme::text_primary()));
                        ui.end_row();

                        ui.label(RichText::new("ASN / 运营商").small().color(theme::muted()));
                        ui.label(RichText::new(if risk.org.is_empty() {
                            "无 ASN 信息".to_owned()
                        } else {
                            risk.org.clone()
                        })
                        .color(theme::text_primary()));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                // 出口侧：本机只有一个 mixed 端口，所以"经代理查出口"只对**当前在跑的
                // 那个节点**成立；别的节点必须用临时内核实测，否则显示的是别人的出口。
                let target = self.ip_risk_target.clone();
                let current = self.proxy_status.node_id.clone();
                let target_is_running = self.proxy_status.local_proxy_active()
                    && matches!((target.as_deref(), current.as_deref()), (Some(a), Some(b)) if a == b);
                let egress_belongs_to_target = self.ip_risk_egress_for.is_some()
                    && self.ip_risk_egress_for == self.ip_risk_target;
                let egress = if egress_belongs_to_target {
                    self.ip_risk_egress.clone()
                } else {
                    None
                };
                let egress_error = if egress_belongs_to_target {
                    self.ip_risk_egress_error.clone()
                } else {
                    None
                };
                if self.ip_risk_egress_pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("正在经当前代理查出口 IP...")
                                .small()
                                .color(theme::muted()),
                        );
                    });
                } else if let Some(egress) = egress {
                    let color = if egress.datacenter { theme::warning() } else { theme::success() };
                    Frame::NONE
                        .fill(if egress.datacenter {
                            theme::warning_bg()
                        } else {
                            theme::success_bg()
                        })
                        .stroke(Stroke::new(1.0_f32, color))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Globe, 16.0, color);
                                ui.label(
                                    RichText::new("站点实际看到的出口")
                                        .strong()
                                        .color(color),
                                );
                            });
                            ui.label(
                                RichText::new(egress.headline())
                                    .small()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(&egress.verdict)
                                    .small()
                                    .color(theme::text_primary()),
                            );
                            if egress.ip != risk.ip {
                                ui.label(
                                    RichText::new(
                                        "入口和出口不是同一个 IP：该节点有中转，风控认的是上面这一行。",
                                    )
                                    .small()
                                    .color(theme::warning()),
                                );
                            }
                        });
                } else if let Some(error) = egress_error {
                    ui.label(
                        RichText::new(format!("出口查询失败：{error}"))
                            .small()
                            .color(theme::danger()),
                    );
                } else if target.is_some() {
                    ui.label(
                        RichText::new(if target_is_running {
                            "出口尚未查询。"
                        } else {
                            "该节点不是当前在跑的节点，出口未知（经本机代理查只会得到当前节点的出口）。"
                        })
                        .small()
                        .color(theme::muted()),
                    );
                }

                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        if risk.hostname == risk.ip {
                            "归类只看 ASN / 公司名关键字，会漏判（例如小商家 VPS），请自行核对上面这行 ASN。\
                             查询走直连、不经过代理，对象就是这个节点服务器 IP。"
                        } else {
                            "节点填的是域名，这里查的是它解析到的 IP —— 域名前置 / ECH / CDN 中转的节点，\
                             这个地址常常是前端而不是真实落地出口（站点侧看到的是出口 IP）。\
                             归类只看 ASN / 公司名关键字，会漏判，请自行核对上面这行 ASN。"
                        },
                    )
                    .small()
                    .color(theme::muted()),
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("复制").clicked() {
                        let mut text = format!("入口：{}\n{}", risk.headline(), risk.verdict);
                        if let Some(egress) = &self.ip_risk_egress {
                            text.push_str(&format!(
                                "\n出口：{}\n{}",
                                egress.headline(),
                                egress.verdict
                            ));
                        }
                        copy_payload = Some(text);
                    }
                    if ui.button("重新查询").clicked() {
                        retry = true;
                    }
                });
            });

        if retry {
            self.ip_risk = None;
            self.check_ip_risk();
        }
        if let Some(text) = copy_payload {
            ctx.copy_text(text);
            self.status = "已复制落地 IP 归属信息".into();
        }
        self.show_ip_risk_dialog = open;
    }
}

/// 重建控制台「可用节点」网格：过滤 + 排序一次，并把每帧都要重复算的东西（国家猜测、
/// 小写名）预先算好；比较器只用已经算好的键，不再一边比较一边分配字符串。
fn build_proxy_view(
    all: &[NodeSummary],
    region: &str,
    search: &str,
    sort: ProxySortBy,
    rev: u64,
) -> ProxyView {
    const REGIONS: [&str; 8] = ["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"];
    let mut rows: Vec<ProxyViewRow> = Vec::with_capacity(all.len());
    for node in all {
        let country = node
            .country
            .clone()
            .or_else(|| crate::parser::guess_country(&node.name))
            .unwrap_or_default()
            .to_ascii_uppercase();
        if region != "all" {
            let hit = if region == "OTHER" {
                !REGIONS.contains(&country.as_str())
            } else {
                country == region
            };
            if !hit {
                continue;
            }
        }
        let lower_name = node.name.to_lowercase();
        if !search.is_empty()
            && !lower_name.contains(search)
            && !node.server.to_lowercase().contains(search)
            && !country.to_lowercase().contains(search)
        {
            continue;
        }
        rows.push(ProxyViewRow {
            node: node.clone(),
            country,
            lower_name,
        });
    }
    match sort {
        ProxySortBy::Name => rows.sort_by(|a, b| a.lower_name.cmp(&b.lower_name)),
        ProxySortBy::Latency => rows.sort_by(|a, b| {
            (
                a.node.latency_ok != Some(true),
                a.node.latency_ms.unwrap_or(i64::MAX),
                &a.lower_name,
            )
                .cmp(&(
                    b.node.latency_ok != Some(true),
                    b.node.latency_ms.unwrap_or(i64::MAX),
                    &b.lower_name,
                ))
        }),
    }
    ProxyView {
        rev,
        region: region.to_owned(),
        search: search.to_owned(),
        sort,
        rows,
    }
}

/// 「导入格式示范」弹框里点出来的动作。
/// 整个弹框抽成不碰 `VpnApp` 的自由函数，好让离屏 `egui::Context` 能单独渲染它做布局验收。
pub(crate) enum RuleHelpMsg {
    Nothing,
    Status(String),
    Fill(&'static str),
    Close,
}

fn set_help_msg(slot: &mut RuleHelpMsg, next: RuleHelpMsg) {
    if matches!(*slot, RuleHelpMsg::Nothing) {
        *slot = next;
    }
}

/// 每种能吃的输入形状给一段可直接抄用的示范 + 支持/跳过的类型清单。
/// 示范文本本身由 `rule_io` 的测试逐段喂回解析器（条数对不上就红），
/// 所以这里写的"可导入 N 条"不是宣传语。
/// 「导入格式示范」整个弹框。宽度与滚动区高度都跟着屏幕算 ——
/// 字体调到 140% 时逻辑屏幕只剩物理的 0.71 倍，写死尺寸会把窗口顶出右缘
/// （离屏量到 660 宽的屏面上窗口画到 662）。
/// 抽成不碰 `VpnApp` 的自由函数，好让离屏 `egui::Context` 真开一次窗口做验收。
fn ui_rule_import_help_window(ctx: &egui::Context) -> (RuleHelpMsg, egui::Rect) {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 96.0).clamp(320.0, 680.0);
    let scroll_height = (screen.height() * 0.5).clamp(180.0, 430.0);
    let mut open = true;
    let mut msg = RuleHelpMsg::Nothing;
    // `Window::open` 借走了 `open`，闭包里只记一笔，出框之后再合。
    let shown = egui::Window::new("导入格式示范")
        .id(egui::Id::new("vpn-parser:rule-import-help"))
        .default_width(width)
        .default_pos(screen.min + Vec2::new(48.0, 32.0))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            msg = ui_rule_import_help_body(ui, scroll_height);
        });
    let rect = shown
        .map(|inner| inner.response.rect)
        .unwrap_or(egui::Rect::NOTHING);
    if !open && matches!(msg, RuleHelpMsg::Nothing) {
        msg = RuleHelpMsg::Close;
    }
    (msg, rect)
}

fn ui_rule_import_help_body(ui: &mut Ui, scroll_height: f32) -> RuleHelpMsg {
    let mut msg = RuleHelpMsg::Nothing;
    ui.label(
        RichText::new(
            "目标写 PROXY / DIRECT / REJECT 以外的自定义组名，一律落到本程序的代理组；\
             GEOIP 的 no-resolve 由本程序自动补；重复的规则保存时去重。",
        )
        .small()
        .color(theme::muted()),
    );
    ui.label(
        RichText::new(format!(
            "认这些类型：{}",
            crate::proxy::rule_io::SUPPORTED_KINDS.join("、")
        ))
        .small()
        .color(theme::text_secondary()),
    );
    ui.label(
        RichText::new(format!(
            "点名跳过：{}、MATCH/FINAL 兜底行",
            crate::proxy::rule_io::UNSUPPORTED_KINDS.join("、")
        ))
        .small()
        .color(theme::warning()),
    );
    egui::ScrollArea::vertical()
        .id_salt("rule_import_help")
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for example in crate::proxy::rule_io::import_examples() {
                ui.add_space(8.0);
                // 每种格式一张卡：卡片用页面那档蓝灰，弹框本身是纯白容器，
                // 卡片内层的代码块再回到纯白 —— 三层靠深浅分，不靠描边堆。
                Frame::NONE
                    .fill(theme::page_bg())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(13, 11))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(example.title)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.add_space(6.0);
                            let (badge, color) = if example.expected_rules > 0 {
                                (
                                    format!("可导入 {} 条", example.expected_rules),
                                    theme::success(),
                                )
                            } else {
                                (
                                    format!("{} 条全部点名跳过", example.expected_unsupported),
                                    theme::warning(),
                                )
                            };
                            status_badge(ui, &badge, color);
                        });
                        ui.label(RichText::new(example.note).small().color(theme::muted()));
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border()))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                for line in example.body.lines() {
                                    ui.label(
                                        RichText::new(line)
                                            .small()
                                            .monospace()
                                            .color(theme::text_secondary()),
                                    );
                                }
                            });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("填入导入框")
                                .on_hover_text("整段塞进「导入规则」，先给解析预览再让导入")
                                .clicked()
                            {
                                set_help_msg(&mut msg, RuleHelpMsg::Fill(example.body));
                            }
                            if ui.button("复制这段").clicked() {
                                let outcome =
                                    arboard::Clipboard::new().and_then(|mut clipboard| {
                                        clipboard.set_text(example.body.to_string())
                                    });
                                set_help_msg(
                                    &mut msg,
                                    RuleHelpMsg::Status(match outcome {
                                        Ok(()) => "示范已复制到剪贴板".to_string(),
                                        Err(error) => format!("复制失败：{error}"),
                                    }),
                                );
                            }
                        });
                    });
            }
        });
    ui.add_space(10.0);
    if ui.button("关闭").clicked() {
        set_help_msg(&mut msg, RuleHelpMsg::Close);
    }
    msg
}

#[cfg(test)]
mod rule_import_help_probe_tests {
    use super::ui_rule_import_help_window;
    use eframe::egui::{self, RawInput, Rect, Vec2};

    /// 用产品同一份构造函数真开三帧「导入格式示范」窗口：
    /// 第 1 帧布局、第 2 帧读"看得见的那部分"、第 3 帧再读一次（确认滚动后仍在渲染）。
    /// 返回（窗口矩形, 第 2 帧文字, 第 3 帧文字）。
    fn probe(screen_width: f32) -> (Rect, String, String) {
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let mut top = String::new();
        let mut bottom = String::new();
        for frame in 0..3 {
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(screen_width, 900.0),
            ));
            if frame == 2 {
                // 光标停在窗口里往下滚，够滚到底。
                let center = rect.center();
                input.events.push(egui::Event::PointerMoved(center));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -2000.0),
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut this_frame = String::new();
            let _ = ctx.run(input, |ctx| {
                let (_msg, window_rect) = ui_rule_import_help_window(ctx);
                rect = window_rect;
                // 形状只能在同一帧里读：这一画在下一帧开始前就被 egui 清掉了。
                let ids: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
                this_frame = ctx.graphics_mut(|g| {
                    let mut out = String::new();
                    for id in &ids {
                        let Some(list) = g.get(*id) else {
                            continue;
                        };
                        for shape in list.all_entries() {
                            if let egui::Shape::Text(text) = &shape.shape {
                                out.push_str(&text.galley.text());
                                out.push('\n');
                            }
                        }
                    }
                    out
                });
            });
            match frame {
                1 => top = this_frame,
                2 => bottom = this_frame,
                _ => {}
            }
        }
        (rect, top, bottom)
    }

    /// 示范弹框必须：整窗留在屏幕里（字体调到 140% 时逻辑宽度只剩物理的 0.71 倍）、
    /// 类型清单与"打开就该看见"的前三段示范真的画出来、两个动作按钮也在。
    /// 断言读的是这一帧实际提交的文字，渲染没跑就等于红；
    /// 六段示范内容的正确性由 rule_io 里"逐段喂回解析器"的测试钉住。
    #[test]
    fn the_help_window_fits_and_paints_the_visible_examples() {
        let examples = crate::proxy::rule_io::import_examples();
        assert_eq!(examples.len(), 6, "示范段落数变了，弹框的说明文案要跟着改");
        for screen_width in [1298.0, 940.0, 660.0, 480.0] {
            let (rect, top, bottom) = probe(screen_width);
            assert!(
                rect.max.x <= screen_width + 0.5,
                "{screen_width:.0} 宽屏面上窗口画到 {:.1}，超出右边界",
                rect.max.x
            );
            assert!(
                rect.height() > 200.0,
                "窗口只有 {:.1} 高，示范没渲染",
                rect.height()
            );
            for (name, text) in [("未滚动", &top), ("滚动后", &bottom)] {
                assert!(
                    text.len() > 200,
                    "{screen_width:.0} 宽时{name}那一帧只画出 {} 字",
                    text.len()
                );
            }
            let painted = format!("{top}\n{bottom}");
            assert!(
                painted.contains("认这些类型") && painted.contains("点名跳过"),
                "{screen_width:.0} 宽时类型清单没画出来"
            );
            // 卡片比原来的裸文本各高 ~30pt：620 逻辑宽以上能不滚动看见前三段，
            // 更窄（960 物理 ÷ 150% 缩放 ÷ 140% 字体）只保证前两段，剩下的靠滚动。
            let visible = if screen_width >= 620.0 { 3 } else { 2 };
            for example in &examples[..visible] {
                assert!(
                    painted.contains(example.title),
                    "{screen_width:.0} 宽时示范「{}」没画出来",
                    example.title
                );
            }
            assert!(
                painted.contains("填入导入框") && painted.contains("复制这段"),
                "{screen_width:.0} 宽时示范的操作按钮没画出来"
            );
        }
    }
}

#[cfg(test)]
mod rule_hits_height_probe_tests {
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻「命中记录」那一列的结构，量滚动区实际拿到的视口高度。
    /// `strip_note` = 把"从本进程启动开始统计…"那行并掉；`reserve` = 视口预留量。
    fn probe(window: Vec2, strip_note: bool, reserve: f32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, window));
        let mut viewport = 0.0_f32;
        let mut card_bottom = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::TopBottomPanel::bottom("status")
                .exact_height(32.0)
                .frame(Frame::NONE.fill(egui::Color32::GRAY))
                .show(ctx, |ui| {
                    let _ = ui.label("状态栏");
                });
            egui::TopBottomPanel::bottom("control")
                .exact_height(48.0)
                .frame(Frame::NONE.fill(egui::Color32::DARK_GRAY))
                .show(ctx, |ui| {
                    let _ = ui.button("启动代理");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    let _ = ui.horizontal(|ui| {
                        let _ = ui.button("规则配置");
                        let _ = ui.button("命中记录");
                    });
                    let _ = ui.separator();
                    ui.add_space(6.0);
                    let card = Frame::NONE
                        .fill(egui::Color32::WHITE)
                        .inner_margin(Margin::same(22));
                    let shown = card.show(ui, |ui| {
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("规则命中记录");
                            let _ = ui.label("123 条连接 · 兜底 MATCH 20 条（16%）");
                            let _ = ui.button("立即刷新");
                        });
                        ui.add_space(2.0);
                        if !strip_note {
                            ui.label("从本进程启动开始统计；占比按连接数算，一条连接只计一次。");
                            ui.add_space(8.0);
                        }
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("命中的规则");
                            let _ = ui.label("派给");
                            let _ = ui.label("条数 / 占比");
                        });
                        let area = egui::ScrollArea::vertical()
                            .id_salt("rule_hits_table_probe")
                            .auto_shrink([false, false])
                            .max_height(ui.available_height() - reserve)
                            .show(ui, |ui| {
                                for _ in 0..40 {
                                    let _ = ui.horizontal(|ui| {
                                        let _ = ui.label("GEOIP,CN,DIRECT");
                                        let _ = ui.label("→ 代理");
                                        let _ = ui.label("12 条 · 9%");
                                    });
                                }
                            });
                        viewport = area.inner_rect.height();
                    });
                    card_bottom = shown.response.rect.max.y;
                });
        });
        (viewport, card_bottom)
    }

    /// 命中记录列表要把剩余高度全吃掉：说明并进表头 + 取消 40pt 预留之后，
    /// 1330×800 的视口从 469（约 19 行）涨到 ≥520（约 22 行），
    /// 140% 字体（950×571 逻辑）从 240（10 行）涨到 ≥290（12 行）。
    /// 同一份结构按老写法（独立说明行 + 预留 40）再量一次，那两行确实是白送的；
    /// 顺手钉住"卡片底不许越过中央面板底"，加高不能靠溢出实现。
    #[test]
    fn the_hits_list_uses_every_remaining_pixel() {
        for (size, min_new) in [
            (Vec2::new(1330.0, 800.0), 520.0),
            (Vec2::new(950.0, 571.0), 290.0),
        ] {
            let (new_vp, new_bottom) = probe(size, true, 0.0);
            let (old_vp, _old_bottom) = probe(size, false, 40.0);
            assert!(
                new_vp >= min_new,
                "{size:?} 时列表视口只有 {new_vp:.1}，没到 {min_new:.0}"
            );
            assert!(
                new_vp > old_vp + 50.0,
                "新写法视口 {new_vp:.1} 只比老写法 {old_vp:.1} 高一点点，说明没省下来"
            );
            // 中央面板底边 = 屏高 - 状态栏 32 - 控制栏 48，再收掉 14 的下内边距。
            let limit = size.y - 32.0 - 48.0 - 14.0;
            assert!(
                new_bottom <= limit + 1.0,
                "{size:?} 时卡片底画到 {new_bottom:.1}，越过中央面板底 {limit:.1}"
            );
        }
    }
}

#[cfg(test)]
mod rules_row_probe_tests {
    use super::{RULES_ACTIONS_WIDTH, RULES_TABS_MIN_WIDTH};
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻分流规则那行表头**含现写的分支判断**：宽到放得下就一行钉右，
    /// 放不下就让动作组另起一行。返回（面板矩形, 分区标签占的矩形, 动作组占的矩形）。
    fn probe(window_width: f32) -> (Rect, Rect, Rect) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 800.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut tabs = Rect::NOTHING;
        let mut group = Rect::NOTHING;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    panel = ui.max_rect();
                    let mut draw_tabs = |ui: &mut egui::Ui| {
                        let a = ui.button("规则配置").rect;
                        let b = ui.button("命中记录").rect;
                        let added = a.union(b);
                        tabs = if !tabs.is_finite() {
                            added
                        } else {
                            tabs.union(added)
                        };
                    };
                    let mut draw_actions = |ui: &mut egui::Ui| {
                        // 只并按钮自己的矩形：min_rect 会把同一行里先画的东西也算进来。
                        let a = ui.button("导出 JSON").rect;
                        let c = ui.button("导入规则").rect;
                        let added = a.union(c);
                        group = if !group.is_finite() {
                            added
                        } else {
                            group.union(added)
                        };
                    };
                    // 与产品同一个判据（ui.max_rect() 就是中央面板的内容宽）。
                    if ui.max_rect().width() >= RULES_ACTIONS_WIDTH + RULES_TABS_MIN_WIDTH + 8.0 {
                        crate::gui::widgets::split_row(
                            ui,
                            "rules:tabs",
                            RULES_ACTIONS_WIDTH,
                            |ui| draw_tabs(ui),
                            |ui| draw_actions(ui),
                        );
                    } else {
                        let _ = ui.horizontal(|ui| draw_tabs(ui));
                        let _ = ui.horizontal_wrapped(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| draw_actions(ui),
                            );
                        });
                    }
                });
        });
        (panel, tabs, group)
    }

    /// 老写法的毛病：右对齐那组和左边的分区标签在窄面板上**互相压住**（x 方向重叠），
    /// 按钮看起来就是被挤走。反向验证：同一个宽度下老写法确实重叠、新写法不重叠。
    #[test]
    fn the_old_single_row_really_collided() {
        let (panel, tabs, group) = probe_single_row(520.0);
        assert!(
            group.min.x < tabs.max.x,
            "老写法在 520 宽没重叠（组从 {:.1} 开始，标签到 {:.1}）—— 反向验证失去意义",
            group.min.x,
            tabs.max.x
        );
        assert!(panel.width() > 0.0);
    }

    fn probe_single_row(window_width: f32) -> (Rect, Rect, Rect) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 800.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut tabs = Rect::NOTHING;
        let mut group = Rect::NOTHING;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    panel = ui.max_rect();
                    let _ = ui.horizontal(|ui| {
                        let a = ui.button("规则配置").rect;
                        let b = ui.button("命中记录").rect;
                        tabs = a.union(b);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let before = ui.min_rect();
                            let _ = ui.button("导出 JSON");
                            let _ = ui.button("导入规则");
                            group = before.union(ui.min_rect());
                        });
                    });
                });
        });
        (panel, tabs, group)
    }

    /// 动作组不许和分区标签互相压住，也不许越过面板右缘。
    /// 下限取 460 = 960 物理最小宽 ÷（150% 系统缩放 × 140% 字体）；再窄（200% 缩放叠 140%
    /// 字体 = 380 逻辑点）egui 自己的右锚点会溢出 4.1pt，那已经不是这套布局能救的了。
    #[test]
    fn the_actions_never_collide_or_leave_the_panel() {
        for width in [1330.0, 950.0, 760.0, 686.0, 620.0, 520.0, 460.0] {
            let (panel, tabs, group) = probe(width);
            assert!(
                group.max.x <= panel.right() + 0.5,
                "{width:.0} 宽时动作组画到 {:.1}，越过面板右缘 {:.1}",
                group.max.x,
                panel.right()
            );
            assert!(
                tabs.min.x >= panel.left() - 0.5 && tabs.max.x <= panel.right() + 0.5,
                "{width:.0} 宽时分区标签画到 [{:.1}, {:.1}]，超出面板 [{:.1}, {:.1}]",
                tabs.min.x,
                tabs.max.x,
                panel.left(),
                panel.right()
            );
            assert!(
                !group.intersects(tabs),
                "{width:.0} 宽时动作组 [{:.1},{:.1}] 压住分区标签 [{:.1},{:.1}]",
                group.min.x,
                group.max.x,
                tabs.min.x,
                tabs.max.x
            );
        }
    }
}

use super::groups::{node_matches_group, override_group_names, visible_group_names};
use super::CLASH_VERGE_RECOMMENDED_OVERRIDE;
use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    ActiveConnectionInfo, ConnectionsTab, DnsRecord, GroupsTab, KernelGroup, ProxyPanel, ProxySortBy,
    ProxyView, ProxyViewRow, RuleDraftKind, RulesTab,
};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, country_flag_and_name, danger_btn, dns_mode_label,
    format_bytes, format_speed, header_cell, macos_segmented_tabs, panel_frame, panel_tab, primary_btn,
    protocol_pill, render_country_code_badge, render_health_strip, render_node_health_bar,
    render_protocol_donut_chart_sized, render_signal_bars, render_speedometer, render_top_processes_card,
    render_traffic_waveform_card, routing_mode_description, routing_mode_detail,
    rule_action_label, rule_matcher_label, sanitize_node_name, section_title, status_badge, tun_stack_label,
};
use crate::gui::VpnApp;
use crate::kernel::{KernelKind, KernelPreference};
use crate::models::NodeSummary;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules, recommended_traffic_rules,
    social_telegram_rules, streaming_media_rules, AutoRoutingStrategy, DnsEnhancedMode, ProxyMode, ProxyState,
    RoutingMode, RuleAction, TrafficRule, TunStack, URL_TEST_GROUP,
};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;

impl VpnApp {
    /// 仪表盘"实时连接"卡的嵌入视图：连接列表 / 连接进程 / DNS 记录 三个子页。
    pub(crate) fn ui_proxy_connections_embedded(&mut self, ui: &mut Ui) {
        let is_running = self.proxy_status.state == ProxyState::Running;
        let mut close_id = None;
        let mut close_all = false;
        let mut close_process_name: Option<String> = None;
        let mut copy_text_payload: Option<String> = None;

        // 聚合活跃进程数据
        let mut process_map: std::collections::HashMap<String, (u64, u64, usize, usize, usize)> = std::collections::HashMap::new();
        let mut direct_conns = 0usize;
        let mut proxy_conns = 0usize;
        let mut direct_bytes = 0u64;
        let mut proxy_bytes = 0u64;

        for c in &self.active_connections {
            let total_traffic = c.download + c.upload;
            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
            if is_direct {
                direct_conns += 1;
                direct_bytes += total_traffic;
            } else {
                proxy_conns += 1;
                proxy_bytes += total_traffic;
            }

            let proc_name = if c.process.is_empty() { "未知进程" } else { c.process.as_str() };
            let entry = process_map.entry(proc_name.to_string()).or_insert((0, 0, 0, 0, 0));
            entry.0 += c.upload;
            entry.1 += c.download;
            entry.2 += 1;
            if is_direct {
                entry.3 += 1;
            } else {
                entry.4 += 1;
            }
        }

        let mut process_list: Vec<(String, u64, u64, usize, usize, usize)> = process_map
            .into_iter()
            .map(|(k, v)| (k, v.0, v.1, v.2, v.3, v.4))
            .collect();
        process_list.sort_by_key(|p| std::cmp::Reverse(p.1 + p.2));

        let total_traffic = direct_bytes + proxy_bytes;
        let total_conns = (direct_conns + proxy_conns) as f32;

        // 顶部紧凑子 Tab 切换栏：连接 / 进程 / DNS（小号胶囊，适配仪表盘卡片）
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let conn_str = format!("连接 ({})", self.active_connections.len());
            let proc_str = format!("进程 ({})", process_list.len());
            let dns_str = format!("DNS ({})", self.dns_records.len());
            let tabs = [
                (ConnectionsTab::List, conn_str.as_str()),
                (ConnectionsTab::Processes, proc_str.as_str()),
                (ConnectionsTab::Dns, dns_str.as_str()),
            ];
            egui::Frame::NONE
                .fill(theme::sidebar())
                .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                .corner_radius(CornerRadius::same(99))
                .inner_margin(Margin::symmetric(3, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (tab, label) in tabs {
                            let selected = self.connections_tab == tab;
                            let (fill, fg) = if selected {
                                (theme::accent(), theme::on_primary())
                            } else {
                                (Color32::TRANSPARENT, theme::text_secondary())
                            };
                            let inner = egui::Frame::NONE
                                .fill(fill)
                                .corner_radius(CornerRadius::same(99))
                                .inner_margin(Margin::symmetric(8, 3))
                                .show(ui, |ui| {
                                    ui.set_min_height(14.0);
                                    ui.label(RichText::new(label).size(9.5).strong().color(fg));
                                });
                            let resp = ui.interact(
                                inner.response.rect,
                                egui::Id::new(("conn_tab", label)),
                                Sense::click(),
                            );
                            if resp.clicked() {
                                self.connections_tab = tab;
                            }
                        }
                    });
                });
        });

        ui.add_space(8.0);

        if !is_running {
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("代理内核未运行，暂无活跃连接与实时 DNS 数据。启动代理后将在此自动监听展示。").color(theme::muted()));
            });
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("proxy_connections_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                match self.connections_tab {
            ConnectionsTab::List => {
                // 连接列表表格（只保留表格本身）
                let filtered: Vec<&ActiveConnectionInfo> =
                    self.active_connections.iter().collect();

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
                } else {
                    panel_frame().show(ui, |ui| {
                        egui::Grid::new("active_connections_grid")
                                    .striped(true)
                                    .min_col_width(60.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "网络");
                                        header_cell(ui, "进程");
                                        header_cell(ui, "目标地址");
                                        header_cell(ui, "分流规则");
                                        header_cell(ui, "上传流量");
                                        header_cell(ui, "下载流量");
                                        ui.end_row();

                                        for c in filtered {
                                            // 1. 网络协议微晶徽标
                                            let net_upper = c.network.to_uppercase();
                                            let (net_fill, net_color) = match net_upper.as_str() {
                                                "TCP" => (theme::accent_bg(), theme::accent_strong()),
                                                "UDP" => (theme::panel_hover(), theme::text_secondary()),
                                                _ => (theme::sidebar(), theme::muted()),
                                            };
                                            Frame::NONE
                                                .fill(net_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&net_upper).small().strong().color(net_color));
                                                });

                                            // 2. 进程胶囊
                                            Frame::NONE
                                                .fill(theme::sidebar())
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.set_max_width(110.0);
                                                    ui.horizontal(|ui| {
                                                        icon_label(ui, Icon::Terminal, 10.0, theme::muted());
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&c.process)
                                                                    .strong()
                                                                    .small()
                                                                    .color(theme::text_primary()),
                                                            )
                                                            .truncate(),
                                                        );
                                                    });
                                                });

                                            // 3. 目标地址
                                            let target_text = if !c.host.is_empty() {
                                                format!("{} ({})", c.host, c.destination)
                                            } else {
                                                c.destination.clone()
                                            };
                                            ui.add_sized(
                                                [170.0, 16.0],
                                                egui::Label::new(
                                                    RichText::new(&target_text)
                                                        .small()
                                                        .color(theme::text_primary()),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&c.destination);

                                            // 4. 分流规则微标
                                            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(rule_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.set_max_width(120.0);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&c.rule)
                                                                .small()
                                                                .strong()
                                                                .color(rule_color),
                                                        )
                                                        .truncate(),
                                                    );
                                                });

                                            // 5. 上传
                                            ui.label(
                                                RichText::new(format!("↑ {}", format_bytes(c.upload)))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );

                                            // 6. 下载
                                            ui.label(
                                                RichText::new(format!("↓ {}", format_bytes(c.download)))
                                                    .small()
                                                    .strong()
                                                    .color(theme::accent_strong()),
                                            );

                                            ui.end_row();
                                        }
                                    });
                            });
                }
            }
            ConnectionsTab::Processes => {
                // 进程表格（只保留表格本身）
                let filtered_procs: Vec<&(String, u64, u64, usize, usize, usize)> =
                    process_list.iter().collect();

                if filtered_procs.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.label(RichText::new("当前暂无进程产生网络连接").color(theme::muted()));
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        egui::Grid::new("process_connections_grid")
                                    .striped(true)
                                    .min_col_width(70.0)
                                    .spacing([14.0, 8.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "进程");
                                        header_cell(ui, "活跃连接数");
                                        header_cell(ui, "分流分布");
                                        header_cell(ui, "上行流量");
                                        header_cell(ui, "下行流量");
                                        header_cell(ui, "总流量");
                                        ui.end_row();

                                        for p in filtered_procs {
                                            let proc_name = &p.0;
                                            let upload = p.1;
                                            let download = p.2;
                                            let conn_count = p.3;
                                            let dir_count = p.4;
                                            let prx_count = p.5;
                                            let proc_traffic = upload + download;

                                            // 1. 进程名称
                                            Frame::NONE
                                                .fill(theme::sidebar())
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(8, 4))
                                                .show(ui, |ui| {
                                                    ui.set_max_width(120.0);
                                                    ui.horizontal(|ui| {
                                                        icon_label(ui, Icon::Terminal, 11.0, theme::accent());
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(proc_name)
                                                                    .strong()
                                                                    .color(theme::text_primary()),
                                                            )
                                                            .truncate(),
                                                        );
                                                    });
                                                });

                                            // 2. 活跃连接数
                                            ui.label(RichText::new(format!("{conn_count} 条")).strong().color(theme::text_primary()));

                                            // 3. 分流分布
                                            ui.horizontal(|ui| {
                                                if prx_count > 0 {
                                                    ui.label(RichText::new(format!("代理:{prx_count}")).small().color(theme::accent_strong()));
                                                }
                                                if dir_count > 0 {
                                                    ui.label(RichText::new(format!("直连:{dir_count}")).small().color(theme::text_secondary()));
                                                }
                                            });

                                            // 4. 上行
                                            ui.label(RichText::new(format!("↑ {}", format_bytes(upload))).small().color(theme::text_secondary()));

                                            // 5. 下行
                                            ui.label(RichText::new(format!("↓ {}", format_bytes(download))).small().strong().color(theme::accent_strong()));

                                            // 6. 总流量
                                            ui.label(RichText::new(format_bytes(proc_traffic)).strong().color(theme::text_primary()));

                                            ui.end_row();
                                        }
                                    });
                            });
                }
            }
            ConnectionsTab::Dns => {
                // DNS 记录表格（只保留表格本身）
                let filtered_dns: Vec<&DnsRecord> = self.dns_records.iter().collect();

                if filtered_dns.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.label(
                            RichText::new(if self.dns_records.is_empty() {
                                "暂无实时 DNS 记录（发起网页浏览或网络请求时将在此自动捕获）"
                            } else {
                                "没有匹配筛选条件的 DNS 记录"
                            })
                            .color(theme::muted()),
                        );
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        egui::Grid::new("dns_records_grid")
                                    .striped(true)
                                    .min_col_width(60.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "时间");
                                        header_cell(ui, "查询域名");
                                        header_cell(ui, "解析目标 IP");
                                        header_cell(ui, "发起进程");
                                        header_cell(ui, "网络");
                                        header_cell(ui, "分流动作");
                                        ui.end_row();

                                        for r in filtered_dns {
                                            // 1. 时间
                                            ui.label(RichText::new(&r.time_str).small().monospace().color(theme::muted()));

                                            // 2. 域名
                                            ui.add_sized(
                                                [150.0, 16.0],
                                                egui::Label::new(
                                                    RichText::new(&r.host)
                                                        .strong()
                                                        .small()
                                                        .color(theme::text_primary()),
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&r.host);

                                            // 3. 解析目标
                                            ui.label(RichText::new(&r.ip).small().monospace().color(theme::text_secondary()))
                                                .on_hover_text(&r.ip);

                                            // 4. 发起进程
                                            Frame::NONE
                                                .fill(theme::sidebar())
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.set_max_width(110.0);
                                                    ui.horizontal(|ui| {
                                                        icon_label(ui, Icon::Terminal, 9.0, theme::muted());
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&r.process)
                                                                    .small()
                                                                    .color(theme::text_primary()),
                                                            )
                                                            .truncate(),
                                                        );
                                                    });
                                                });

                                            // 5. 协议
                                            let (net_fill, net_color) = if r.network == "UDP" {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(net_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&r.network).small().strong().color(net_color));
                                                });

                                            // 6. 分流动作
                                            let is_direct = r.target.eq_ignore_ascii_case("direct") || r.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(rule_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.set_max_width(120.0);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&r.rule)
                                                                .small()
                                                                .strong()
                                                                .color(rule_color),
                                                        )
                                                        .truncate(),
                                                    );
                                                });

                                            ui.end_row();
                                        }
                                    });
                            });
                }
            }
        }
        ui.add_space(16.0);
    });

        if let Some(id) = close_id {
            self.close_connection(id);
        } else if close_all {
            self.close_all_connections();
        } else if let Some(proc_name) = close_process_name {
            let ids_to_close: Vec<String> = self
                .active_connections
                .iter()
                .filter(|c| c.process == proc_name)
                .map(|c| c.id.clone())
                .collect();
            for id in ids_to_close {
                self.close_connection(id);
            }
        }

        if let Some(text) = copy_text_payload {
            ui.ctx().copy_text(text);
            self.status = "已复制到剪贴板".into();
        }
    }

}

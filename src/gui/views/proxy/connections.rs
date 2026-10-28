use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{ActiveConnectionInfo, ConnectionsTab, DnsRecord};
use crate::gui::widgets::{
    card_frame, danger_btn, format_bytes, header_cell,
    macos_segmented_tabs, panel_frame,
};
use crate::gui::VpnApp;
use crate::proxy::{DnsEnhancedMode, ProxyState};
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};

impl VpnApp {
    /// 独立的"活跃连接"标签页（抽屉内全宽展示：Tab + 工具栏 + 概览 + 表格）。
    pub(crate) fn ui_proxy_connections(&mut self, ui: &mut Ui) {
        ui.set_min_width(ui.available_width());
        ui.set_width(ui.available_width());

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

        // 顶部 macOS 风格次级 Tab 切换栏：连接列表 / 连接进程 / DNS 记录
        ui.horizontal(|ui| {
            let conn_str = format!("连接列表 ({})", self.active_connections.len());
            let proc_str = format!("连接进程 ({})", process_list.len());
            let dns_str = format!("DNS 记录 ({})", self.dns_records.len());
            let tabs = [
                (ConnectionsTab::List, Icon::Radar, conn_str.as_str()),
                (ConnectionsTab::Processes, Icon::Cpu, proc_str.as_str()),
                (ConnectionsTab::Dns, Icon::Globe, dns_str.as_str()),
            ];
            macos_segmented_tabs(ui, &mut self.connections_tab, &tabs);
        });

        ui.add_space(8.0);

        if !is_running {
            ui.add_space(20.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(
                        RichText::new("代理内核未运行")
                            .strong()
                            .size(14.0)
                            .color(theme::text_primary()),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("启动代理内核后，此处将实时展示各网络连接的目标地址、所属进程、分流规则及上传下载速率。")
                            .small()
                            .color(theme::muted()),
                    );
                    ui.add_space(40.0);
                });
            });
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("proxy_connections_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match self.connections_tab {
            ConnectionsTab::List => {
                // 1. 工具栏：筛选 + 统计 + 批量断开（两端对齐充实 100% 宽度）
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
                    ui.label(format!("当前活跃连接: {} 条", self.active_connections.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.active_connections.is_empty(), danger_btn("断开全部连接"))
                            .on_hover_text("向内核发送指令关闭所有活动的网络连接")
                            .clicked()
                        {
                            close_all = true;
                        }
                    });
                });

                ui.add_space(8.0);

                // 2. 分流流向概览
                if total_conns > 0.0 {
                    Frame::NONE
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Radar, 13.0, theme::accent());
                                    ui.label(RichText::new("分流流向监控").small().strong().color(theme::text_primary()));
                                });
                                ui.add_space(4.0);

                                let segments = [
                                    ("代理分流 (Proxy)", proxy_conns as f32, theme::accent()),
                                    ("国内直连 (Direct)", direct_conns as f32, theme::border()),
                                ];
                                crate::gui::widgets::render_distribution_bar(ui, &segments, total_conns, 8.0);
                                ui.add_space(4.0);

                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "代理: {} 连 ({})",
                                            proxy_conns,
                                            format_bytes(proxy_bytes)
                                        ))
                                        .small()
                                        .color(theme::accent_strong()),
                                    );
                                    ui.separator();
                                    ui.label(
                                        RichText::new(format!(
                                            "直连: {} 连 ({})",
                                            direct_conns,
                                            format_bytes(direct_bytes)
                                        ))
                                        .small()
                                        .color(theme::text_secondary()),
                                    );
                                });
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3. 连接列表表格
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
                    ui.add_space(16.0);
                    card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            ui.label(
                                RichText::new(if self.active_connections.is_empty() {
                                    "当前暂无活动的网络连接"
                                } else {
                                    "没有匹配筛选条件的网络连接"
                                })
                                .strong()
                                .size(13.5)
                                .color(theme::text_secondary()),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("当有网络请求经由内核转发时，此处将实时呈现连接明细")
                                    .small()
                                    .color(theme::muted()),
                            );
                            ui.add_space(40.0);
                        });
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let grid_w = ui.available_width();
                        let proc_w = (grid_w * 0.16).clamp(100.0, 180.0);
                        let rule_w = (grid_w * 0.12).clamp(70.0, 130.0);
                        let other_cols_w = 44.0 + proc_w + rule_w + 80.0 + 85.0 + 50.0 + (6.0 * 12.0);
                        let dest_w = (grid_w - other_cols_w).max(220.0);

                        egui::ScrollArea::horizontal()
                            .id_salt("active_connections_hscroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(grid_w);
                                egui::Grid::new("active_connections_grid")
                                    .striped(true)
                                    .min_col_width(44.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "网络");
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(proc_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(proc_w);
                                                ui.set_max_width(proc_w);
                                                header_cell(ui, "进程");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dest_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dest_w);
                                                ui.set_max_width(dest_w);
                                                header_cell(ui, "目标地址");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(rule_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(rule_w);
                                                ui.set_max_width(rule_w);
                                                header_cell(ui, "分流规则");
                                            },
                                        );
                                        header_cell(ui, "上传流量");
                                        header_cell(ui, "下载流量");
                                        header_cell(ui, "操作");
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
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(proc_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(proc_w);
                                                    ui.set_max_width(proc_w);
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                icon_label(ui, Icon::Terminal, 10.0, theme::muted());
                                                                ui.add(egui::Label::new(RichText::new(&c.process).strong().small().color(theme::text_primary())).truncate())
                                                                    .on_hover_text(&c.process);
                                                            });
                                                        });
                                                },
                                            );

                                            // 3. 目标地址（长域名自适应展开，悬浮查看完整地址与目标 IP）
                                            let full_target = if !c.host.is_empty() {
                                                format!("{} ({})", c.host, c.destination)
                                            } else {
                                                c.destination.clone()
                                            };
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dest_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dest_w);
                                                    ui.set_max_width(dest_w);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&full_target).small().color(theme::text_primary()),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(format!("{}\n目标: {}", full_target, c.destination));
                                                },
                                            );

                                            // 4. 分流规则微标
                                            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(rule_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(rule_w);
                                                    ui.set_max_width(rule_w);
                                                    Frame::NONE
                                                        .fill(rule_fill)
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.add(egui::Label::new(RichText::new(&c.rule).small().strong().color(rule_color)).truncate())
                                                                .on_hover_text(&c.rule);
                                                        });
                                                },
                                            );

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

                                            // 7. 操作
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
                }
            }
            ConnectionsTab::Processes => {
                // 1. 工具栏
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选进程");
                    let proc_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_process_filter)
                            .hint_text("按进程名过滤")
                            .desired_width(180.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &proc_filter_field);
                    if !self.connections_process_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_process_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("当前活跃进程: {} 个", process_list.len()));
                    ui.separator();
                    ui.label(RichText::new(format!("进程产生总流量: {}", format_bytes(total_traffic))).small().color(theme::muted()));
                });

                ui.add_space(8.0);

                // 2. Top 进程矩阵卡片
                let top_matrix: Vec<(&str, u64, usize)> = process_list
                    .iter()
                    .map(|p| (p.0.as_str(), p.1 + p.2, p.3))
                    .collect();
                if !top_matrix.is_empty() {
                    Frame::NONE
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Cpu, 13.0, theme::accent());
                                    ui.label(RichText::new("Top 进程流量与带宽占比").small().strong().color(theme::text_primary()));
                                });
                                ui.add_space(4.0);
                                crate::gui::widgets::render_top_processes_card(ui, &top_matrix, total_traffic);
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3. 进程详细聚合表
                let p_filter = self.connections_process_filter.trim().to_lowercase();
                let filtered_procs: Vec<&(String, u64, u64, usize, usize, usize)> = process_list
                    .iter()
                    .filter(|p| p_filter.is_empty() || p.0.to_lowercase().contains(&p_filter))
                    .collect();

                if filtered_procs.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("当前暂无进程产生网络连接").color(theme::muted()));
                            ui.add_space(20.0);
                        });
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let p_grid_w = ui.available_width();
                        let p_other_w = 85.0 + 110.0 + 85.0 + 85.0 + 90.0 + 75.0 + (6.0 * 14.0);
                        let p_name_w = (p_grid_w - p_other_w).max(160.0);

                        egui::ScrollArea::horizontal()
                            .id_salt("process_connections_hscroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(p_grid_w);
                                egui::Grid::new("process_connections_grid")
                                    .striped(true)
                                    .min_col_width(70.0)
                                    .spacing([14.0, 8.0])
                                    .show(ui, |ui| {
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(p_name_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(p_name_w);
                                                ui.set_max_width(p_name_w);
                                                header_cell(ui, "进程");
                                            },
                                        );
                                        header_cell(ui, "活跃连接数");
                                        header_cell(ui, "分流分布");
                                        header_cell(ui, "上行流量");
                                        header_cell(ui, "下行流量");
                                        header_cell(ui, "总流量");
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for p in filtered_procs {
                                            let proc_name = &p.0;
                                            let upload = p.1;
                                            let download = p.2;
                                            let conn_count = p.3;
                                            let dir_count = p.4;
                                            let prx_count = p.5;
                                            let proc_traffic = upload + download;

                                            // 1. 进程名称（自适应展开，超出智能截断）
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(p_name_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(p_name_w);
                                                    ui.set_max_width(p_name_w);
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(8, 4))
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                icon_label(ui, Icon::Terminal, 11.0, theme::accent());
                                                                ui.add(egui::Label::new(RichText::new(proc_name).strong().color(theme::text_primary())).truncate())
                                                                    .on_hover_text(proc_name);
                                                            });
                                                        });
                                                },
                                            );

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

                                            // 7. 操作：关闭该进程的所有连接
                                            if ui
                                                .add(danger_btn("断开全部"))
                                                .on_hover_text(format!("关闭进程 {proc_name} 的全部活动连接"))
                                                .clicked()
                                            {
                                                close_process_name = Some(proc_name.clone());
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                }
            }
            ConnectionsTab::Dns => {
                // 1. 工具栏：筛选 + 统计 + 清空（两端对齐充实 100% 宽度）
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选记录");
                    let dns_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_dns_filter)
                            .hint_text("按域名、解析 IP 或发起进程过滤")
                            .desired_width(200.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &dns_filter_field);
                    if !self.connections_dns_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_dns_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("已记录 DNS 轨迹: {} 条", self.dns_records.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("清空全部记录").clicked() {
                            self.dns_records.clear();
                        }
                    });
                });

                ui.add_space(8.0);

                // 2. DNS 运行配置摘要
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Globe, 14.0, theme::accent());
                            ui.label(RichText::new("DNS 架构状态:").strong().color(theme::text_primary()));
                            let (mode_str, mode_color) = match self.proxy_config.dns.enhanced_mode {
                                DnsEnhancedMode::FakeIp => ("Fake-IP (秒级并发映射)", theme::accent_strong()),
                                DnsEnhancedMode::RedirHost => ("Redir-Host (真实解析直通)", theme::accent()),
                            };
                            ui.label(RichText::new(mode_str).strong().color(mode_color));
                            ui.separator();
                            let ns = self.proxy_config.dns.nameservers.first().map(|s| s.as_str()).unwrap_or("doh.pub");
                            ui.label(RichText::new(format!("主上游: {ns}")).small().color(theme::muted()));
                        });
                    });

                ui.add_space(8.0);

                // 3. DNS 记录表格
                let d_filter = self.connections_dns_filter.trim().to_lowercase();
                let filtered_dns: Vec<&DnsRecord> = self
                    .dns_records
                    .iter()
                    .filter(|r| {
                        d_filter.is_empty()
                            || r.host.to_lowercase().contains(&d_filter)
                            || r.ip.to_lowercase().contains(&d_filter)
                            || r.process.to_lowercase().contains(&d_filter)
                            || r.rule.to_lowercase().contains(&d_filter)
                    })
                    .collect();

                if filtered_dns.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(
                                RichText::new(if self.dns_records.is_empty() {
                                    "暂无实时 DNS 记录（发起网页浏览或网络请求时将在此自动捕获）"
                                } else {
                                    "没有匹配筛选条件的 DNS 记录"
                                })
                                .color(theme::muted()),
                            );
                            ui.add_space(20.0);
                        });
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let dns_grid_w = ui.available_width();
                        let dns_proc_w = (dns_grid_w * 0.14).clamp(100.0, 160.0);
                        let dns_rule_w = (dns_grid_w * 0.12).clamp(70.0, 120.0);
                        let dns_other_w = 75.0 + dns_proc_w + 44.0 + dns_rule_w + 50.0 + (6.0 * 12.0);
                        let dns_remaining = (dns_grid_w - dns_other_w).max(320.0);
                        let dns_host_w = (dns_remaining * 0.55).max(170.0);
                        let dns_ip_w = (dns_remaining * 0.45).max(140.0);

                        egui::ScrollArea::horizontal()
                            .id_salt("dns_records_hscroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(dns_grid_w);
                                egui::Grid::new("dns_records_grid")
                                    .striped(true)
                                    .min_col_width(44.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "时间");
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_host_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_host_w);
                                                ui.set_max_width(dns_host_w);
                                                header_cell(ui, "查询域名");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_ip_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_ip_w);
                                                ui.set_max_width(dns_ip_w);
                                                header_cell(ui, "解析目标 IP");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_proc_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_proc_w);
                                                ui.set_max_width(dns_proc_w);
                                                header_cell(ui, "发起进程");
                                            },
                                        );
                                        header_cell(ui, "网络");
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_rule_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_rule_w);
                                                ui.set_max_width(dns_rule_w);
                                                header_cell(ui, "分流动作");
                                            },
                                        );
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for r in filtered_dns {
                                            // 1. 时间
                                            ui.label(RichText::new(&r.time_str).small().monospace().color(theme::muted()));

                                            // 2. 域名（自适应铺满展开）
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_host_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_host_w);
                                                    ui.set_max_width(dns_host_w);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&r.host).strong().small().color(theme::text_primary()),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(&r.host);
                                                },
                                            );

                                            // 3. 解析目标 IP（自适应铺满展开）
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_ip_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_ip_w);
                                                    ui.set_max_width(dns_ip_w);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&r.ip).small().monospace().color(theme::text_secondary()),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(&r.ip);
                                                },
                                            );

                                            // 4. 发起进程
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_proc_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_proc_w);
                                                    ui.set_max_width(dns_proc_w);
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                icon_label(ui, Icon::Terminal, 9.0, theme::muted());
                                                                ui.add(egui::Label::new(RichText::new(&r.process).small().color(theme::text_primary())).truncate())
                                                                    .on_hover_text(&r.process);
                                                            });
                                                        });
                                                },
                                            );

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
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_rule_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_rule_w);
                                                    ui.set_max_width(dns_rule_w);
                                                    Frame::NONE
                                                        .fill(rule_fill)
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.add(egui::Label::new(RichText::new(&r.rule).small().strong().color(rule_color)).truncate())
                                                                .on_hover_text(&r.rule);
                                                        });
                                                },
                                            );

                                            // 7. 复制
                                            ui.horizontal(|ui| {
                                                if ui.small_button("复制域名").clicked() {
                                                    copy_text_payload = Some(r.host.clone());
                                                }
                                                if ui.small_button("复制IP").clicked() {
                                                    copy_text_payload = Some(r.ip.clone());
                                                }
                                            });
                                            ui.end_row();
                                        }
                                    });
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

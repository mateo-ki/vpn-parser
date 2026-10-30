//! 流量统计抽屉视图：节点 / 域名 / 软件进程三个维度的流量排行
//!
//! 点击控制台上传/下载卡片时滑出，一行一个条目，按流量消耗清晰排行，统计“谁跑得多”。
//! 三个维度共享同一份连接增量数据（poll.rs 每拍同步喂三张表），各自独立排序与搜索。

use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    DomainTrafficRecord, NodeTrafficRecord, NodeTrafficSort, ProcessTrafficRecord, TrafficStatTab,
};
use crate::gui::widgets::badges::{country_flag_and_name, protocol_pill};
use crate::gui::widgets::{card_frame, format_bytes, format_speed};
use crate::gui::VpnApp;
use eframe::egui::{self, Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::HashMap;

/// 三个维度共用的排行行语义：排序只认这五个数。
trait StatRow {
    fn total(&self) -> u64 {
        self.up().saturating_add(self.down())
    }
    fn up(&self) -> u64;
    fn down(&self) -> u64;
    fn conns(&self) -> usize;
    fn name(&self) -> &str;
}

impl StatRow for NodeTrafficRecord {
    fn total(&self) -> u64 {
        self.total_bytes()
    }
    fn up(&self) -> u64 {
        self.upload_bytes
    }
    fn down(&self) -> u64 {
        self.download_bytes
    }
    fn conns(&self) -> usize {
        self.active_conns
    }
    fn name(&self) -> &str {
        &self.node_name
    }
}

impl StatRow for DomainTrafficRecord {
    fn total(&self) -> u64 {
        self.total_bytes()
    }
    fn up(&self) -> u64 {
        self.upload_bytes
    }
    fn down(&self) -> u64 {
        self.download_bytes
    }
    fn conns(&self) -> usize {
        self.active_conns
    }
    fn name(&self) -> &str {
        &self.domain
    }
}

impl StatRow for ProcessTrafficRecord {
    fn total(&self) -> u64 {
        self.total_bytes()
    }
    fn up(&self) -> u64 {
        self.upload_bytes
    }
    fn down(&self) -> u64 {
        self.download_bytes
    }
    fn conns(&self) -> usize {
        self.active_conns
    }
    fn name(&self) -> &str {
        &self.process
    }
}

/// 按当前排序模式排好一个维度的记录（并列时连接多者在前，再并列按名字稳定序）
fn sort_stat_rows<T: StatRow>(list: &mut [T], sort: NodeTrafficSort) {
    match sort {
        NodeTrafficSort::Total => list.sort_by(|a, b| {
            b.total()
                .cmp(&a.total())
                .then_with(|| b.conns().cmp(&a.conns()))
                .then_with(|| a.name().cmp(b.name()))
        }),
        NodeTrafficSort::Download => list.sort_by(|a, b| {
            b.down()
                .cmp(&a.down())
                .then_with(|| b.conns().cmp(&a.conns()))
                .then_with(|| a.name().cmp(b.name()))
        }),
        NodeTrafficSort::Upload => list.sort_by(|a, b| {
            b.up()
                .cmp(&a.up())
                .then_with(|| b.conns().cmp(&a.conns()))
                .then_with(|| a.name().cmp(b.name()))
        }),
    }
}

impl VpnApp {
    /// 流量统计抽屉入口：tab 栏 + 当前维度视图
    pub(crate) fn ui_proxy_node_traffic(&mut self, ui: &mut Ui) {
        ui.set_width(ui.available_width());

        self.render_traffic_tab_bar(ui);
        ui.add_space(10.0);

        match self.traffic_stat_tab {
            TrafficStatTab::Node => self.traffic_node_tab(ui),
            TrafficStatTab::Domain => self.traffic_domain_tab(ui),
            TrafficStatTab::Process => self.traffic_process_tab(ui),
        }
    }

    /// 三 tab 分段选择器：节点流量统计 / 域名流量统计 / 软件流量统计
    fn render_traffic_tab_bar(&mut self, ui: &mut Ui) {
        let tabs = [
            (TrafficStatTab::Node, "🚀 节点流量统计"),
            (TrafficStatTab::Domain, "🌐 域名流量统计"),
            (TrafficStatTab::Process, "⚙ 软件流量统计"),
        ];
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (tab, label) in tabs {
                let selected = self.traffic_stat_tab == tab;
                let bg = if selected {
                    theme::accent_bg()
                } else if theme::is_light() {
                    Color32::from_rgb(240, 242, 245)
                } else {
                    Color32::from_rgb(38, 42, 50)
                };
                let fg = if selected {
                    theme::accent()
                } else {
                    theme::text_secondary()
                };
                let btn = egui::Button::new(RichText::new(label).size(12.0).color(fg))
                    .fill(bg)
                    .corner_radius(CornerRadius::same(8))
                    .min_size(Vec2::new(0.0, 26.0));
                if ui.add(btn).clicked() {
                    self.traffic_stat_tab = tab;
                }
            }
        });
    }

    /// tab 顶部三列概览卡：累计总流量 + 维度特有中卡 + 实时活跃连接
    fn traffic_overview_cards(
        ui: &mut Ui,
        sum_up: u64,
        sum_down: u64,
        mid_title: &str,
        mid_value: String,
        mid_sub: String,
        total_active_conns: usize,
    ) {
        let sum_total = sum_up.saturating_add(sum_down);
        ui.columns(3, |columns| {
            card_frame().show(&mut columns[0], |ui| {
                ui.set_width(ui.available_width());
                ui.vertical(|ui| {
                    ui.label(RichText::new("累计总流量").size(11.0).color(theme::muted()));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format_bytes(sum_total))
                            .size(16.0)
                            .strong()
                            .color(theme::accent()),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!(
                            "↓ {}  ↑ {}",
                            format_bytes(sum_down),
                            format_bytes(sum_up)
                        ))
                        .size(10.0)
                        .color(theme::text_secondary()),
                    );
                });
            });

            card_frame().show(&mut columns[1], |ui| {
                ui.set_width(ui.available_width());
                ui.vertical(|ui| {
                    ui.label(RichText::new(mid_title).size(11.0).color(theme::muted()));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(mid_value)
                            .size(16.0)
                            .strong()
                            .color(theme::text_primary()),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(mid_sub)
                            .size(10.0)
                            .color(theme::text_secondary()),
                    );
                });
            });

            card_frame().show(&mut columns[2], |ui| {
                ui.set_width(ui.available_width());
                ui.vertical(|ui| {
                    ui.label(RichText::new("实时活跃连接").size(11.0).color(theme::muted()));
                    ui.add_space(2.0);
                    let conns_color =
                        if total_active_conns > 0 { theme::success() } else { theme::text_primary() };
                    ui.label(
                        RichText::new(format!("{} 连接", total_active_conns))
                            .size(16.0)
                            .strong()
                            .color(conns_color),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(if total_active_conns > 0 {
                            "实时传输中"
                        } else {
                            "暂无活跃传输"
                        })
                        .size(10.0)
                        .color(theme::text_secondary()),
                    );
                });
            });
        });
    }

    /// tab 共用控制栏：排序胶囊 + 搜索框 + 右对齐清空按钮（只清当前维度）
    fn traffic_filter_bar(&mut self, ui: &mut Ui, tab: TrafficStatTab) {
        let mut clear = false;
        ui.horizontal(|ui| {
            ui.set_width(ui.available_width());

            ui.label(RichText::new("排序:").size(12.0).color(theme::muted()));
            ui.spacing_mut().item_spacing.x = 4.0;
            let sort_options = [
                (NodeTrafficSort::Total, "综合"),
                (NodeTrafficSort::Download, "下载优先"),
                (NodeTrafficSort::Upload, "上传优先"),
            ];
            for (opt, label) in sort_options {
                let is_selected = match tab {
                    TrafficStatTab::Node => self.node_traffic_sort == opt,
                    TrafficStatTab::Domain => self.domain_traffic_sort == opt,
                    TrafficStatTab::Process => self.process_traffic_sort == opt,
                };
                let bg = if is_selected {
                    theme::accent_bg()
                } else if theme::is_light() {
                    Color32::from_rgb(240, 242, 245)
                } else {
                    Color32::from_rgb(38, 42, 50)
                };
                let fg = if is_selected {
                    theme::accent()
                } else {
                    theme::text_secondary()
                };
                let btn = egui::Button::new(RichText::new(label).size(11.5).color(fg))
                    .fill(bg)
                    .corner_radius(CornerRadius::same(6));
                if ui.add(btn).clicked() {
                    match tab {
                        TrafficStatTab::Node => self.node_traffic_sort = opt,
                        TrafficStatTab::Domain => self.domain_traffic_sort = opt,
                        TrafficStatTab::Process => self.process_traffic_sort = opt,
                    }
                }
            }

            ui.add_space(8.0);

            let hint = match tab {
                TrafficStatTab::Node => "🔍 搜索节点名/协议...",
                TrafficStatTab::Domain => "🔍 搜索域名...",
                TrafficStatTab::Process => "🔍 搜索进程名...",
            };
            match tab {
                TrafficStatTab::Node => ui.add(
                    egui::TextEdit::singleline(&mut self.node_traffic_search)
                        .hint_text(hint)
                        .desired_width(140.0),
                ),
                TrafficStatTab::Domain => ui.add(
                    egui::TextEdit::singleline(&mut self.domain_traffic_search)
                        .hint_text(hint)
                        .desired_width(140.0),
                ),
                TrafficStatTab::Process => ui.add(
                    egui::TextEdit::singleline(&mut self.process_traffic_search)
                        .hint_text(hint)
                        .desired_width(140.0),
                ),
            };

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tip = match tab {
                    TrafficStatTab::Node => "重置当前记录的节点流量与连接增量统计",
                    TrafficStatTab::Domain => "清空域名流量统计",
                    TrafficStatTab::Process => "清空软件进程流量统计",
                };
                if ui
                    .add(icons::button_with_icon(Icon::Trash, "清空统计", theme::muted()))
                    .on_hover_text(tip)
                    .clicked()
                {
                    clear = true;
                }
            });
        });
        if clear {
            match tab {
                TrafficStatTab::Node => {
                    self.node_traffic_records.clear();
                    self.conn_prev_traffic.clear();
                }
                TrafficStatTab::Domain => {
                    self.domain_traffic_records.clear();
                }
                TrafficStatTab::Process => {
                    self.process_traffic_records.clear();
                }
            }
        }
    }

    /// 维度排行行的右端组件：进度条 + 占比 + 总量大字（节点/域名/进程共用）
    fn render_row_right(
        ui: &mut Ui,
        rank: usize,
        ratio: f32,
        metric: u64,
        max_bytes: u64,
        total: u64,
    ) {
        let total_color = if total > 0 {
            theme::text_primary()
        } else {
            theme::muted()
        };
        ui.label(
            RichText::new(format_bytes(total))
                .strong()
                .size(14.0)
                .color(total_color),
        );

        ui.add_space(6.0);

        let pct_str = if max_bytes > 0 && total > 0 {
            format!("{:.0}%", (metric as f64 / max_bytes as f64) * 100.0)
        } else {
            "—".to_string()
        };
        ui.add_sized(
            [34.0, 16.0],
            egui::Label::new(RichText::new(pct_str).size(11.0).color(theme::muted())),
        );

        ui.add_space(6.0);

        render_traffic_bar(ui, ratio, rank <= 3, 85.0);
    }

    /// 维度行左端的实时小字：N连接 · 速率（有活跃连接才显示）
    fn render_row_live_line(ui: &mut Ui, active_conns: usize, down_rate: u64, up_rate: u64) {
        if active_conns > 0 {
            render_live_dot(ui);
            ui.label(
                RichText::new(format!("{}连接", active_conns))
                    .size(10.5)
                    .color(theme::success()),
            );
            ui.add_space(3.0);
            if down_rate > 0 || up_rate > 0 {
                ui.label(
                    RichText::new(format!("↓{} ↑{}", format_speed(down_rate), format_speed(up_rate)))
                        .size(10.5)
                        .color(theme::accent()),
                );
                ui.add_space(3.0);
            }
        }
    }

    /// 维度行卡片外框（排名徽标 + 图标 + 标题 + 实时行 | 进度条区）
    #[allow(clippy::too_many_arguments)]
    fn traffic_simple_row(
        ui: &mut Ui,
        rank: usize,
        icon: &str,
        title: &str,
        _hover: &str,
        active_conns: usize,
        down_rate: u64,
        up_rate: u64,
        down_bytes: u64,
        up_bytes: u64,
        metric: u64,
        max_bytes: u64,
        total: u64,
    ) {
        let has_traffic = total > 0;
        let border_stroke = if has_traffic {
            Stroke::new(1.0_f32, theme::border_subtle())
        } else {
            Stroke::new(1.0_f32, theme::border_subtle().gamma_multiply(0.6))
        };

        egui::Frame::NONE
            .fill(theme::panel())
            .stroke(border_stroke)
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                let total_w = ui.available_width();
                let right_w = 210.0_f32;
                let left_w = (total_w - right_w - 6.0).max(120.0);
                let ratio = if max_bytes > 0 && total > 0 {
                    (metric as f32 / max_bytes as f32).clamp(0.02, 1.0)
                } else {
                    0.0
                };

                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_w, 40.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_max_width(left_w);

                            render_rank_badge(ui, rank);
                            ui.add_space(3.0);
                            ui.label(RichText::new(icon).size(15.0));
                            ui.add_space(2.0);

                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    let name_color = if has_traffic {
                                        theme::text_primary()
                                    } else {
                                        theme::text_secondary()
                                    };
                                    let name_label = egui::Label::new(
                                        RichText::new(title)
                                            .strong()
                                            .size(12.5)
                                            .color(name_color),
                                    )
                                    .truncate();
                                    ui.add(name_label);
                                });

                                ui.add_space(2.0);

                                ui.horizontal(|ui| {
                                    Self::render_row_live_line(ui, active_conns, down_rate, up_rate);
                                    ui.label(
                                        RichText::new(format!(
                                            "↓下行: {}  ·  ↑上行: {}",
                                            format_bytes(down_bytes),
                                            format_bytes(up_bytes)
                                        ))
                                        .size(10.5)
                                        .color(theme::muted()),
                                    );
                                });
                            });
                        },
                    );

                    ui.allocate_ui_with_layout(
                        Vec2::new(right_w, 40.0),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            ui.set_min_width(right_w);
                            ui.set_max_width(right_w);
                            Self::render_row_right(ui, rank, ratio, metric, max_bytes, total);
                        },
                    );
                });
            });
    }

    /// 节点流量统计 tab（原节点排行完整保留）
    fn traffic_node_tab(&mut self, ui: &mut Ui) {
        // 1. 汇总与聚合全量节点（已记录流量的 + 节点池未跑流量的）
        let mut all_records: HashMap<String, NodeTrafficRecord> =
            self.node_traffic_records.clone();

        // 将当前节点池中尚未产生流量的节点补充进来（保持列表完整）
        for node in &self.nodes {
            all_records
                .entry(node.name.clone())
                .or_insert_with(|| NodeTrafficRecord {
                    node_name: node.name.clone(),
                    protocol: node.protocol.clone(),
                    server: node.server.clone(),
                    country: node.country.clone(),
                    ..Default::default()
                });
        }

        // 计算全局累计指标
        let mut sum_up = 0u64;
        let mut sum_down = 0u64;
        let mut routed_nodes_count = 0usize;
        let mut total_active_conns = 0usize;

        for rec in all_records.values() {
            sum_up = sum_up.saturating_add(rec.upload_bytes);
            sum_down = sum_down.saturating_add(rec.download_bytes);
            if rec.total_bytes() > 0 {
                routed_nodes_count += 1;
            }
            total_active_conns += rec.active_conns;
        }

        // 2. 顶部概览卡片（3 列等宽分布，填满抽屉可用宽度，严禁右侧留白）
        Self::traffic_overview_cards(
            ui,
            sum_up,
            sum_down,
            "活跃出站节点",
            format!("{} 节点", routed_nodes_count),
            format!("池中共 {} 节点", all_records.len()),
            total_active_conns,
        );

        ui.add_space(10.0);

        // 3. 控制与筛选栏
        self.traffic_filter_bar(ui, TrafficStatTab::Node);

        ui.add_space(8.0);

        // 4. 节点列表过滤与排序
        let search_kw = self.node_traffic_search.trim().to_lowercase();
        let mut list: Vec<NodeTrafficRecord> = all_records
            .into_values()
            .filter(|r| {
                if search_kw.is_empty() {
                    true
                } else {
                    r.node_name.to_lowercase().contains(&search_kw)
                        || r.protocol.to_lowercase().contains(&search_kw)
                        || r.server.to_lowercase().contains(&search_kw)
                }
            })
            .collect();

        sort_stat_rows(&mut list, self.node_traffic_sort);

        // 取当前最高流量节点的数值作为相对条基准（若全为 0 则设为 1 避免除零）
        let max_bytes = list
            .first()
            .map(|r| match self.node_traffic_sort {
                NodeTrafficSort::Total => r.total_bytes(),
                NodeTrafficSort::Download => r.download_bytes,
                NodeTrafficSort::Upload => r.upload_bytes,
            })
            .unwrap_or(0)
            .max(1);

        let active_node_name = self.selected_proxy_node_name.clone();

        // 5. 一行一个节点，直观展示统计与对比进度条
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;

                if list.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        icon_label(ui, Icon::Info, 24.0, theme::muted());
                        ui.add_space(4.0);
                        ui.label(RichText::new("暂无匹配的节点流量记录").color(theme::muted()));
                    });
                    return;
                }

                for (idx, item) in list.iter().enumerate() {
                    let is_active = active_node_name.as_deref() == Some(&item.node_name);
                    let rank = idx + 1;
                    let total = item.total_bytes();

                    let item_metric = match self.node_traffic_sort {
                        NodeTrafficSort::Total => total,
                        NodeTrafficSort::Download => item.download_bytes,
                        NodeTrafficSort::Upload => item.upload_bytes,
                    };
                    let ratio = if total > 0 {
                        (item_metric as f32 / max_bytes as f32).clamp(0.02, 1.0)
                    } else {
                        0.0
                    };

                    let card_bg = if is_active {
                        if theme::is_light() {
                            Color32::from_rgb(243, 250, 246)
                        } else {
                            Color32::from_rgb(26, 36, 30)
                        }
                    } else {
                        theme::panel()
                    };

                    let border_stroke = if is_active {
                        Stroke::new(1.0_f32, theme::success().gamma_multiply(0.6))
                    } else {
                        Stroke::new(1.0_f32, theme::border_subtle())
                    };

                    egui::Frame::NONE
                        .fill(card_bg)
                        .stroke(border_stroke)
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            // 强制充满整行卡片宽度
                            ui.set_width(ui.available_width());

                            let total_w = ui.available_width();
                            let right_w = 210.0_f32;
                            let left_w = (total_w - right_w - 6.0).max(120.0);

                            ui.horizontal(|ui| {
                                // 左半部：排名 + 旗帜 + 节点名(带截断) + 协议 + 下行/上行
                                ui.allocate_ui_with_layout(
                                    Vec2::new(left_w, 40.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.set_max_width(left_w);

                                        render_rank_badge(ui, rank);
                                        ui.add_space(3.0);

                                        let flag = item
                                            .country
                                            .as_deref()
                                            .map(|c| country_flag_and_name(c).0)
                                            .unwrap_or("🌐");
                                        ui.label(RichText::new(flag).size(15.0));
                                        ui.add_space(2.0);

                                        ui.vertical(|ui| {
                                            ui.horizontal(|ui| {
                                                let name_color = if total > 0 {
                                                    theme::text_primary()
                                                } else {
                                                    theme::text_secondary()
                                                };
                                                let name_label = egui::Label::new(
                                                    RichText::new(&item.node_name)
                                                        .strong()
                                                        .size(12.5)
                                                        .color(name_color),
                                                )
                                                .truncate();
                                                ui.add(name_label);

                                                if is_active {
                                                    render_status_pill(ui, "运行中", theme::success());
                                                }

                                                if !item.protocol.is_empty()
                                                    && item.protocol != "-"
                                                {
                                                    protocol_pill(
                                                        ui,
                                                        &item.protocol.to_uppercase(),
                                                    );
                                                }
                                            });

                                            ui.add_space(2.0);

                                            ui.horizontal(|ui| {
                                                Self::render_row_live_line(
                                                    ui,
                                                    item.active_conns,
                                                    item.download_rate,
                                                    item.upload_rate,
                                                );

                                                ui.label(
                                                    RichText::new(format!(
                                                        "↓下行: {}  ·  ↑上行: {}",
                                                        format_bytes(item.download_bytes),
                                                        format_bytes(item.upload_bytes)
                                                    ))
                                                    .size(10.5)
                                                    .color(theme::muted()),
                                                );
                                            });
                                        });
                                    },
                                );

                                // 右半部：完全贴齐卡片右侧边缘的进度条与总流量
                                ui.allocate_ui_with_layout(
                                    Vec2::new(right_w, 40.0),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.set_min_width(right_w);
                                        ui.set_max_width(right_w);

                                        Self::render_row_right(
                                            ui,
                                            rank,
                                            ratio,
                                            item_metric,
                                            max_bytes,
                                            total,
                                        );
                                    },
                                );
                            });
                        });
                }
                ui.add_space(16.0);
            });
    }

    /// 域名流量统计 tab：按连接 host（纯 IP 连接按目标地址）聚合排行
    fn traffic_domain_tab(&mut self, ui: &mut Ui) {
        let mut sum_up = 0u64;
        let mut sum_down = 0u64;
        let mut active_domains = 0usize;
        let mut total_active_conns = 0usize;
        for rec in self.domain_traffic_records.values() {
            sum_up = sum_up.saturating_add(rec.upload_bytes);
            sum_down = sum_down.saturating_add(rec.download_bytes);
            if rec.active_conns > 0 {
                active_domains += 1;
            }
            total_active_conns += rec.active_conns;
        }

        Self::traffic_overview_cards(
            ui,
            sum_up,
            sum_down,
            "访问域名数",
            format!("{} 域名", self.domain_traffic_records.len()),
            format!("活跃传输 {} 个", active_domains),
            total_active_conns,
        );

        ui.add_space(10.0);

        self.traffic_filter_bar(ui, TrafficStatTab::Domain);

        ui.add_space(8.0);

        let search_kw = self.domain_traffic_search.trim().to_lowercase();
        let sort = self.domain_traffic_sort;
        let mut list: Vec<DomainTrafficRecord> = self
            .domain_traffic_records
            .values()
            .filter(|r| search_kw.is_empty() || r.domain.to_lowercase().contains(&search_kw))
            .cloned()
            .collect();
        sort_stat_rows(&mut list, sort);

        let max_bytes = list
            .first()
            .map(|r| match sort {
                NodeTrafficSort::Total => r.total_bytes(),
                NodeTrafficSort::Download => r.download_bytes,
                NodeTrafficSort::Upload => r.upload_bytes,
            })
            .unwrap_or(0)
            .max(1);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;

                if list.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        icon_label(ui, Icon::Info, 24.0, theme::muted());
                        ui.add_space(4.0);
                        ui.label(RichText::new("暂无域名流量记录，跑起流量后再来看").color(theme::muted()));
                    });
                    return;
                }

                for (idx, item) in list.iter().enumerate() {
                    let rank = idx + 1;
                    let total = item.total_bytes();
                    let metric = match sort {
                        NodeTrafficSort::Total => total,
                        NodeTrafficSort::Download => item.download_bytes,
                        NodeTrafficSort::Upload => item.upload_bytes,
                    };
                    Self::traffic_simple_row(
                        ui,
                        rank,
                        "🌐",
                        &item.domain,
                        &item.domain,
                        item.active_conns,
                        item.download_rate,
                        item.upload_rate,
                        item.download_bytes,
                        item.upload_bytes,
                        metric,
                        max_bytes,
                        total,
                    );
                }
                ui.add_space(16.0);
            });
    }

    /// 软件流量统计 tab：按连接 process 聚合排行（拿不到进程名归入「未知进程」）
    fn traffic_process_tab(&mut self, ui: &mut Ui) {
        let mut sum_up = 0u64;
        let mut sum_down = 0u64;
        let mut active_processes = 0usize;
        let mut total_active_conns = 0usize;
        for rec in self.process_traffic_records.values() {
            sum_up = sum_up.saturating_add(rec.upload_bytes);
            sum_down = sum_down.saturating_add(rec.download_bytes);
            if rec.active_conns > 0 {
                active_processes += 1;
            }
            total_active_conns += rec.active_conns;
        }

        Self::traffic_overview_cards(
            ui,
            sum_up,
            sum_down,
            "软件进程数",
            format!("{} 进程", self.process_traffic_records.len()),
            format!("活跃传输 {} 个", active_processes),
            total_active_conns,
        );

        ui.add_space(10.0);

        self.traffic_filter_bar(ui, TrafficStatTab::Process);

        ui.add_space(8.0);

        let search_kw = self.process_traffic_search.trim().to_lowercase();
        let sort = self.process_traffic_sort;
        let mut list: Vec<ProcessTrafficRecord> = self
            .process_traffic_records
            .values()
            .filter(|r| search_kw.is_empty() || r.process.to_lowercase().contains(&search_kw))
            .cloned()
            .collect();
        sort_stat_rows(&mut list, sort);

        let max_bytes = list
            .first()
            .map(|r| match sort {
                NodeTrafficSort::Total => r.total_bytes(),
                NodeTrafficSort::Download => r.download_bytes,
                NodeTrafficSort::Upload => r.upload_bytes,
            })
            .unwrap_or(0)
            .max(1);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 6.0;

                if list.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        icon_label(ui, Icon::Info, 24.0, theme::muted());
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("暂无软件流量记录，跑起流量后再来看").color(theme::muted()),
                        );
                    });
                    return;
                }

                for (idx, item) in list.iter().enumerate() {
                    let rank = idx + 1;
                    let total = item.total_bytes();
                    let metric = match sort {
                        NodeTrafficSort::Total => total,
                        NodeTrafficSort::Download => item.download_bytes,
                        NodeTrafficSort::Upload => item.upload_bytes,
                    };
                    Self::traffic_simple_row(
                        ui,
                        rank,
                        "⚙",
                        &item.process,
                        &item.process,
                        item.active_conns,
                        item.download_rate,
                        item.upload_rate,
                        item.download_bytes,
                        item.upload_bytes,
                        metric,
                        max_bytes,
                        total,
                    );
                }
                ui.add_space(16.0);
            });
    }
}

/// 排名徽标：Top 1 金牌，Top 2 银牌，Top 3 铜牌，其他灰色编号
fn render_rank_badge(ui: &mut Ui, rank: usize) {
    let (bg, fg, label) = match rank {
        1 => (
            Color32::from_rgb(255, 185, 30),
            Color32::from_rgb(90, 45, 0),
            "Top 1",
        ),
        2 => (
            Color32::from_rgb(195, 205, 218),
            Color32::from_rgb(45, 55, 68),
            "Top 2",
        ),
        3 => (
            Color32::from_rgb(220, 145, 95),
            Color32::from_rgb(70, 30, 10),
            "Top 3",
        ),
        _ => (
            if theme::is_light() {
                Color32::from_rgb(235, 238, 242)
            } else {
                Color32::from_rgb(40, 44, 52)
            },
            theme::muted(),
            "",
        ),
    };

    let text = if rank <= 3 {
        label.to_string()
    } else {
        format!("#{rank}")
    };

    egui::Frame::NONE
        .fill(bg)
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).strong().size(10.5).color(fg));
        });
}

/// 流量对比进度条（谁跑的多一目了然）
fn render_traffic_bar(ui: &mut Ui, ratio: f32, is_top: bool, bar_w: f32) {
    let bar_h = 6.0_f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(bar_w, bar_h), Sense::hover());

    let painter = ui.painter();
    let bg_color = if theme::is_light() {
        Color32::from_rgb(230, 233, 238)
    } else {
        Color32::from_rgb(45, 50, 60)
    };
    painter.rect_filled(rect, CornerRadius::same(3), bg_color);

    if ratio > 0.0 {
        let fill_w = (bar_w * ratio).clamp(2.0, bar_w);
        let fill_rect = Rect::from_min_size(rect.min, Vec2::new(fill_w, bar_h));
        let fill_color = if is_top {
            theme::accent()
        } else {
            theme::accent().gamma_multiply(0.7)
        };
        painter.rect_filled(fill_rect, CornerRadius::same(3), fill_color);
    }
}

/// 状态指示胶囊
fn render_status_pill(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.4)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(10.0).strong().color(color));
        });
}

/// 实时活跃绿点指示
fn render_live_dot(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.0, theme::success());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_traffic_sorting_orders() {
        let mut list = vec![
            NodeTrafficRecord {
                node_name: "Node-A".into(),
                upload_bytes: 500,
                download_bytes: 1000,
                ..Default::default()
            },
            NodeTrafficRecord {
                node_name: "Node-B".into(),
                upload_bytes: 2000,
                download_bytes: 100,
                ..Default::default()
            },
            NodeTrafficRecord {
                node_name: "Node-C".into(),
                upload_bytes: 100,
                download_bytes: 5000,
                ..Default::default()
            },
        ];

        // 综合总流量: C(5100) > B(2100) > A(1500)
        sort_stat_rows(&mut list, NodeTrafficSort::Total);
        assert_eq!(list[0].node_name, "Node-C");
        assert_eq!(list[1].node_name, "Node-B");
        assert_eq!(list[2].node_name, "Node-A");

        // 下载优先: C(5000) > A(1000) > B(100)
        sort_stat_rows(&mut list, NodeTrafficSort::Download);
        assert_eq!(list[0].node_name, "Node-C");
        assert_eq!(list[1].node_name, "Node-A");
        assert_eq!(list[2].node_name, "Node-B");

        // 上传优先: B(2000) > A(500) > C(100)
        sort_stat_rows(&mut list, NodeTrafficSort::Upload);
        assert_eq!(list[0].node_name, "Node-B");
        assert_eq!(list[1].node_name, "Node-A");
        assert_eq!(list[2].node_name, "Node-C");
    }

    #[test]
    fn test_node_traffic_record_total_bytes() {
        let record = NodeTrafficRecord {
            node_name: "HK-01".into(),
            upload_bytes: 1024 * 1024,
            download_bytes: 5 * 1024 * 1024,
            ..Default::default()
        };
        assert_eq!(record.total_bytes(), 6 * 1024 * 1024);
    }

    /// 通用 StatRow 排序对域名与进程维度同样成立（综合/下载/上传三档）
    #[test]
    fn test_stat_row_sorting_for_domain_and_process() {
        let mut domains = vec![
            DomainTrafficRecord {
                domain: "a.example.com".into(),
                upload_bytes: 100,
                download_bytes: 900,
                ..Default::default()
            },
            DomainTrafficRecord {
                domain: "b.example.com".into(),
                upload_bytes: 50,
                download_bytes: 50,
                ..Default::default()
            },
            DomainTrafficRecord {
                domain: "c.example.com".into(),
                upload_bytes: 300,
                download_bytes: 10,
                ..Default::default()
            },
        ];
        sort_stat_rows(&mut domains, NodeTrafficSort::Total);
        assert_eq!(domains[0].domain, "a.example.com"); // 1000
        assert_eq!(domains[1].domain, "c.example.com"); // 310
        assert_eq!(domains[2].domain, "b.example.com"); // 100

        sort_stat_rows(&mut domains, NodeTrafficSort::Upload);
        assert_eq!(domains[0].domain, "c.example.com"); // 300

        let mut procs = vec![
            ProcessTrafficRecord {
                process: "chrome.exe".into(),
                upload_bytes: 10,
                download_bytes: 4000,
                ..Default::default()
            },
            ProcessTrafficRecord {
                process: "steam.exe".into(),
                upload_bytes: 800,
                download_bytes: 20,
                ..Default::default()
            },
        ];
        sort_stat_rows(&mut procs, NodeTrafficSort::Download);
        assert_eq!(procs[0].process, "chrome.exe");
        sort_stat_rows(&mut procs, NodeTrafficSort::Upload);
        assert_eq!(procs[0].process, "steam.exe");
    }
}

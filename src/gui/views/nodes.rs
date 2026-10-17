use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, panel_frame, sort_label,
};
use crate::gui::VpnApp;
use crate::speedtest::SpeedtestIntensity;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

impl VpnApp {
    pub(crate) fn ui_nodes(&mut self, ui: &mut Ui) {
        let mut filter_changed = false;
        let mut delete_dead = false;
        let mut prune_stale = false;
        let mut delete_invalid = false;
        let mut physical_dedupe = false;
        let mut clean_names = false;
        let mut dedupe = false;
        let mut clear_all = false;

        // 节点类型统计：协议分布环形图 (Donut Chart) + 协议缩写徽标
        if let Some(s) = &self.stats.clone() {
            if !s.protocol_stats.is_empty() {
                ui.horizontal(|ui| {
                    crate::gui::widgets::render_protocol_donut_chart(ui, &s.protocol_stats, s.node_count);
                    ui.add_space(8.0);

                    ui.vertical(|ui| {
                        crate::gui::widgets::render_node_health_bar(
                            ui,
                            s.alive_count,
                            s.tested_count,
                            s.node_count,
                        );
                        ui.add_space(3.0);

                        let lat_stats = crate::gui::widgets::LatencySpectrumStats::from_latencies(
                            self.nodes.iter().map(|n| n.latency_ms)
                        );
                        crate::gui::widgets::render_latency_spectrum(ui, &lat_stats);
                        ui.add_space(4.0);

                        ui.horizontal_wrapped(|ui| {
                            let is_all = self.protocol_filter.is_empty() || self.protocol_filter == "all";
                            if crate::gui::widgets::protocol_stat_chip(ui, "All", s.node_count, is_all).clicked() {
                                self.protocol_filter = "".to_string();
                                filter_changed = true;
                            }

                            for p in &s.protocol_stats {
                                let is_active = self.protocol_filter.eq_ignore_ascii_case(&p.protocol);
                                if crate::gui::widgets::protocol_stat_chip(ui, &p.protocol, p.count, is_active).clicked() {
                                    if is_active {
                                        self.protocol_filter = "".to_string();
                                    } else {
                                        self.protocol_filter = p.protocol.clone();
                                    }
                                    filter_changed = true;
                                }
                            }

                            ui.separator();

                            let is_all_lat = (self.latency_filter.is_empty() || self.latency_filter == "all") && !self.only_available_nodes;
                            let (color_all, stroke_all) = if is_all_lat {
                                (theme::accent_bg(), Stroke::new(1.0_f32, theme::accent()))
                            } else {
                                (theme::sidebar(), Stroke::new(1.0_f32, theme::border_subtle()))
                            };
                            if ui.add(egui::Button::new(RichText::new("全部状态").small().strong()).fill(color_all).stroke(stroke_all).corner_radius(CornerRadius::same(10))).clicked() {
                                self.latency_filter = "all".to_string();
                                self.only_available_nodes = false;
                                filter_changed = true;
                            }

                            let is_alive = self.only_available_nodes || self.latency_filter == "alive";
                            let (color_alive, stroke_alive) = if is_alive {
                                (theme::success_bg(), Stroke::new(1.0_f32, theme::success()))
                            } else {
                                (theme::sidebar(), Stroke::new(1.0_f32, theme::border_subtle()))
                            };
                            if ui.add(egui::Button::new(RichText::new("🟢 仅看可用").small().strong()).fill(color_alive).stroke(stroke_alive).corner_radius(CornerRadius::same(10))).clicked() {
                                self.only_available_nodes = !self.only_available_nodes;
                                self.latency_filter = if self.only_available_nodes { "alive".to_string() } else { "all".to_string() };
                                filter_changed = true;
                            }

                            let is_untested = self.latency_filter == "untested";
                            let (color_untest, stroke_untest) = if is_untested {
                                (theme::accent_bg(), Stroke::new(1.0_f32, theme::accent()))
                            } else {
                                (theme::sidebar(), Stroke::new(1.0_f32, theme::border_subtle()))
                            };
                            if ui.add(egui::Button::new(RichText::new("⚪ 未测速").small().strong()).fill(color_untest).stroke(stroke_untest).corner_radius(CornerRadius::same(10))).clicked() {
                                self.only_available_nodes = false;
                                self.latency_filter = if is_untested { "all".to_string() } else { "untested".to_string() };
                                filter_changed = true;
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }
        }

        // 1. 顶部整合卡片：搜索与快速筛选
        Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                // 第一行：搜索框 + 排序 + 只看可用 + 右侧操作组
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;

                    // 搜索框
                    icon_label(ui, Icon::Search, 13.0, theme::text_secondary());
                    let search_field = ui.add(
                        egui::TextEdit::singleline(&mut self.keyword)
                            .desired_width(140.0)
                            .hint_text("搜索名称、服务器 IP..."),
                    );
                    crate::gui::widgets::note_text_field(ui, &search_field);
                    if search_field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        filter_changed = true;
                    }

                    ui.separator();

                    // 订阅源下拉筛选（支持下拉模糊搜索）
                    ui.label(RichText::new("订阅源:").small().color(theme::muted()));
                    let source_label = if self.source_filter.is_empty() {
                        "全部订阅源".to_string()
                    } else if let Some(s) = self.sources.iter().find(|s| s.id == self.source_filter) {
                        let name = &s.name;
                        if name.chars().count() > 8 {
                            format!("源: {}…", name.chars().take(8).collect::<String>())
                        } else {
                            format!("源: {name}")
                        }
                    } else {
                        "全部订阅源".to_string()
                    };

                    egui::ComboBox::from_id_salt("node_source_filter")
                        .selected_text(source_label)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            // 下拉框内嵌模糊搜索输入框
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Search, 11.0, theme::muted());
                                let search_edit = ui.add(
                                    egui::TextEdit::singleline(&mut self.source_search_query)
                                        .desired_width(140.0)
                                        .hint_text("模糊搜索订阅源..."),
                                );
                                crate::gui::widgets::note_text_field(ui, &search_edit);
                                if !self.source_search_query.is_empty() {
                                    if ui.small_button("✕").clicked() {
                                        self.source_search_query.clear();
                                    }
                                }
                            });
                            ui.separator();

                            egui::ScrollArea::vertical()
                                .max_height(220.0)
                                .show(ui, |ui| {
                                    if ui.selectable_label(self.source_filter.is_empty(), "全部订阅源").clicked() {
                                        if !self.source_filter.is_empty() {
                                            self.source_filter.clear();
                                            filter_changed = true;
                                        }
                                    }

                                    let q = self.source_search_query.trim().to_lowercase();
                                    let mut matched = 0;
                                    for s in &self.sources {
                                        if !q.is_empty() {
                                            let match_name = s.name.to_lowercase().contains(&q);
                                            let match_remark = s.remark.as_deref().unwrap_or("").to_lowercase().contains(&q);
                                            let match_url = s.url_template.to_lowercase().contains(&q);
                                            if !match_name && !match_remark && !match_url {
                                                continue;
                                            }
                                        }
                                        matched += 1;
                                        let is_selected = self.source_filter == s.id;
                                        let label = format!("{} ({})", s.name, s.last_node_count);
                                        if ui.selectable_label(is_selected, label).clicked() {
                                            if self.source_filter != s.id {
                                                self.source_filter = s.id.clone();
                                                filter_changed = true;
                                            }
                                        }
                                    }

                                    if matched == 0 && !q.is_empty() {
                                        ui.label(RichText::new("无匹配订阅源").small().color(theme::muted()));
                                    }
                                });
                        });

                    if !self.source_filter.is_empty() {
                        if ui.small_button("✕").on_hover_text("清除订阅源筛选").clicked() {
                            self.source_filter.clear();
                            filter_changed = true;
                        }
                    }

                    ui.separator();

                    // 排序下拉
                    ui.label(RichText::new("排序:").small().color(theme::muted()));
                    egui::ComboBox::from_id_salt("sort_by")
                        .selected_text(sort_label(&self.sort_by))
                        .show_ui(ui, |ui| {
                            for (val, lbl) in [
                                ("latency", "延迟最低"),
                                ("download", "下载速度"),
                                ("updated", "更新时间"),
                                ("name", "节点名称"),
                            ] {
                                if ui.selectable_value(&mut self.sort_by, val.into(), lbl).changed() {
                                    filter_changed = true;
                                }
                            }
                        });

                    // 右侧动作菜单组（自右向左排列）
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🔄 刷新").clicked() {
                            self.reload_node_view();
                        }

                        ui.separator();

                        // 导入与导出菜单
                        ui.menu_button("📥 导入/导出 ▼", |ui| {
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("📋 剪贴板导入节点/订阅"))
                                .clicked()
                            {
                                self.import_from_clipboard();
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("📤 推送可用节点到服务端"))
                                .clicked()
                            {
                                self.publish_available_nodes();
                                ui.close_menu();
                            }
                            ui.separator();
                            ui.label(RichText::new("导出配置:").small().color(theme::muted()));
                            if ui.button("导出 Clash YAML 配置").clicked() {
                                self.open_export("clash");
                                ui.close_menu();
                            }
                            if ui.button("导出 URI 节点链接").clicked() {
                                self.open_export("uri");
                                ui.close_menu();
                            }
                            if ui.button("导出 JSON 格式").clicked() {
                                self.open_export("json");
                                ui.close_menu();
                            }
                        });

                        ui.separator();

                        // 清理维护菜单
                        ui.add_enabled_ui(!self.busy, |ui| {
                            ui.menu_button("🧹 维护清理 ▼", |ui| {
                                prune_stale = ui
                                    .button("🍂 淘汰 7 天过期失效节点")
                                    .on_hover_text("清理持续不可用的僵尸节点")
                                    .clicked();
                                physical_dedupe = ui
                                    .button("🧹 物理去重 (同 IP:Port)")
                                    .on_hover_text("按真实服务器IP/主机与端口物理去重并保留最优记录")
                                    .clicked();
                                clean_names = ui
                                    .button("✨ 净化节点名称")
                                    .on_hover_text("去除广告推广乱码并标准化国旗与地区")
                                    .clicked();
                                dedupe = ui
                                    .button("去除完全相同节点")
                                    .on_hover_text("按协议、服务器、端口和凭据去重")
                                    .clicked();
                                delete_dead = ui.button("删除当前超时/失败节点").clicked();
                                delete_invalid = ui.button("删除内网/无效主机").clicked();
                                ui.separator();
                                clear_all = ui
                                    .button(RichText::new("清空全部节点").color(theme::danger()))
                                    .clicked();
                            });
                        });

                        ui.separator();

                        // 测速菜单
                        ui.add_enabled_ui(!self.busy, |ui| {
                            ui.menu_button("⚡ 批量测速 ▼", |ui| {
                                if ui
                                    .button("⚡ 测速当前页")
                                    .on_hover_text("测试当前分页显示的节点")
                                    .clicked()
                                {
                                    self.speedtest_page();
                                    ui.close_menu();
                                }
                                if ui
                                    .button("⚡ 测速当前列表")
                                    .on_hover_text("测试符合当前筛选条件的全部节点（跨越分页）")
                                    .clicked()
                                {
                                    self.speedtest_current_list();
                                    ui.close_menu();
                                }
                                if ui
                                    .button("⚡ 测速全库全部节点")
                                    .on_hover_text("忽略筛选条件，并发测速全库所有节点")
                                    .clicked()
                                {
                                    self.speedtest_all_sources();
                                    ui.close_menu();
                                }
                                ui.separator();
                                ui.label(RichText::new("测速强度设置:").small().color(theme::muted()));
                                for intensity in [SpeedtestIntensity::Fast, SpeedtestIntensity::Standard, SpeedtestIntensity::Deep] {
                                    if ui.selectable_value(&mut self.speedtest_intensity, intensity, intensity.label()).changed() {
                                        ui.close_menu();
                                    }
                                }
                            });
                        });
                    });
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // 第二行：地区快速筛选胶囊行
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("地区:").small().strong().color(theme::muted()));
                    for (val, lbl) in [
                        ("all", "全部"),
                        ("HK", "🇭🇰 香港"),
                        ("JP", "🇯🇵 日本"),
                        ("US", "🇺🇸 美国"),
                        ("SG", "🇸🇬 新加坡"),
                        ("TW", "🇹🇼 台湾"),
                        ("KR", "🇰🇷 韩国"),
                        ("OTHER", "🌐 其它"),
                    ] {
                        let is_active = self.region_filter == val;
                        let text_color = if is_active { theme::accent_strong() } else { theme::text_secondary() };
                        let pill = ui.add(
                            egui::Button::new(RichText::new(lbl).small().strong().color(text_color))
                                .fill(if is_active { theme::accent_bg() } else { theme::page_bg() })
                                .stroke(Stroke::new(1.0_f32, if is_active { theme::accent() } else { theme::border_subtle() }))
                                .corner_radius(CornerRadius::same(12)),
                        );
                        if pill.clicked() && !is_active {
                            self.region_filter = val.to_string();
                            filter_changed = true;
                        }
                    }
                });
            });
        if filter_changed {
            self.page = 1;
            self.reload_nodes();
        }

        if prune_stale {
            self.prune_stale_nodes();
        } else if delete_dead {
            self.delete_dead_nodes();
        } else if delete_invalid {
            self.delete_invalid_hosts();
        } else if physical_dedupe {
            self.dedupe_physical_nodes();
        } else if clean_names {
            self.clean_node_names();
        } else if dedupe {
            self.dedupe_nodes();
        } else if clear_all {
            self.clear_nodes();
        }

        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let pages = NodesViewModel::total_pages(self.node_total, self.page_size);
            ui.label(format!(
                "第 {} / {} 页 · 共 {} 条",
                self.page, pages, self.node_total
            ));
            if ui
                .add_enabled(self.page > 1, egui::Button::new("上一页"))
                .clicked()
            {
                self.page -= 1;
                self.reload_nodes();
            }
            if ui
                .add_enabled(self.page < pages, egui::Button::new("下一页"))
                .clicked()
            {
                self.page += 1;
                self.reload_nodes();
            }
        });
        ui.add_space(6.0);

        if self.nodes.is_empty() {
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("当前筛选条件下没有节点").color(theme::text_secondary()));
            });
            return;
        }

        let action = render_node_cards_grid(
            ui,
            &self.nodes,
            self.selected_proxy_node_id.as_deref(),
            self.switching_node_id.as_deref(),
            self.busy,
        );
        if let Some(action) = action {
            self.handle_node_card_action(action);
        }
    }
}

pub fn render_node_cards_grid(
    ui: &mut Ui,
    nodes: &[crate::models::NodeSummary],
    selected_proxy_node_id: Option<&str>,
    switching_node_id: Option<&str>,
    is_busy: bool,
) -> Option<NodeCardAction> {
    let mut card_action: Option<NodeCardAction> = None;
    panel_frame().show(ui, |ui| {
        let (columns, total_card_rows) =
            NodesViewModel::grid_layout(ui.available_width(), nodes.len());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, 84.0, total_card_rows, |ui, row_range| {
                for row_idx in row_range {
                    let start = row_idx * columns;
                    let end = (start + columns).min(nodes.len());
                    let row = &nodes[start..end];
                    ui.columns(columns, |column_uis| {
                        for (column_index, column_ui) in column_uis.iter_mut().enumerate() {
                            let Some(node) = row.get(column_index) else {
                                continue;
                            };
                            let is_selected =
                                selected_proxy_node_id == Some(&node.id);
                            let is_switching =
                                switching_node_id == Some(&node.id);
                            let response =
                                compact_node_card(column_ui, node, is_selected, is_switching)
                                    .on_hover_text(format!(
                                        "{}:{}\n点击连接，右键管理",
                                        node.server, node.port
                                    ));
                            let connect = response.clicked();
                            response.context_menu(|ui| {
                                if ui
                                    .add_enabled(!is_busy, egui::Button::new("测速节点"))
                                    .clicked()
                                {
                                    card_action = Some(NodeCardAction::Speedtest(node.id.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("查看详情").clicked() {
                                    card_action = Some(NodeCardAction::ShowDetail(node.id.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("IP 检测").clicked() {
                                    card_action = Some(NodeCardAction::CheckIpRisk(node.id.clone()));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui
                                    .button(RichText::new("删除节点").color(theme::danger()))
                                    .clicked()
                                {
                                    card_action = Some(NodeCardAction::Delete(node.id.clone()));
                                    ui.close_menu();
                                }
                            });
                            if connect && !is_selected {
                                card_action = Some(NodeCardAction::Connect {
                                    id: node.id.clone(),
                                    name: node.name.clone(),
                                });
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            });
    });
    card_action
}

impl VpnApp {

    pub(crate) fn ui_detail(&mut self, ctx: &egui::Context) {
        if !self.show_detail {
            return;
        }
        let mut open = true;
        egui::Window::new("节点详情")
            .collapsible(false)
            .resizable(true)
            .default_width(500.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("复制").clicked() {
                        ctx.copy_text(self.detail_text.clone());
                    }
                    if ui.button("关闭").clicked() {
                        self.show_detail = false;
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::both().max_height(400.0).show(ui, |ui| {
                    let detail_field = ui.add(
                        egui::TextEdit::multiline(&mut self.detail_text)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY),
                    );
                    crate::gui::widgets::note_text_field(ui, &detail_field);
                });
            });
        if !open {
            self.show_detail = false;
        }
    }
}

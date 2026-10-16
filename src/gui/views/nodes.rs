use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, node_group_key, panel_frame, sort_label,
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

        // 节点类型统计：协议双字母缩写图标 (All / VL / VM / TR / SS 等) + 旁边是数量
        if let Some(s) = &self.stats.clone() {
            if !s.protocol_stats.is_empty() {
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
                            .desired_width(180.0)
                            .hint_text("搜索名称、服务器 IP..."),
                    );
                    crate::gui::widgets::note_text_field(ui, &search_field);
                    if search_field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        filter_changed = true;
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

                    ui.separator();

                    if ui
                        .checkbox(&mut self.only_available_nodes, "仅看可用")
                        .on_hover_text("仅展示通过真实握手验证的节点")
                        .changed()
                    {
                        filter_changed = true;
                    }

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
                                if ui.button("⚡ 测速当前页").clicked() {
                                    self.speedtest_page();
                                    ui.close_menu();
                                }
                                let selected_group = self.node_groups.iter().find(|group| {
                                    self.selected_node_source_id.as_deref() == Some(node_group_key(group).as_str())
                                });
                                if let Some(group) = selected_group.cloned() {
                                    if ui.button("⚡ 测速当前订阅全部节点").clicked() {
                                        self.speedtest_group(group);
                                        ui.close_menu();
                                    }
                                }
                                if ui.button("⚡ 测速全库全部节点").clicked() {
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
            ui.label(
                RichText::new("订阅")
                    .strong()
                    .color(theme::text_secondary()),
            );
            let selected_source_id = self.selected_node_source_id.clone();
            let mut next_source = None;
            ui.scope(|ui| {
                ui.style_mut().always_scroll_the_only_direction = true;
                egui::ScrollArea::horizontal()
                    .id_salt("node_source_tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let is_all = selected_source_id.is_none();
                            if ui.selectable_label(is_all, "全部订阅源").clicked() && !is_all {
                                self.selected_node_source_id = None;
                                self.page = 1;
                                self.reload_nodes();
                            }
                            for group in &self.node_groups {
                                let key = node_group_key(group);
                                let selected =
                                    selected_source_id.as_deref() == Some(&key);
                                let label = format!(
                                    "{}  可用 {} / 共 {}",
                                    group.source_name, group.alive, group.total
                                );
                                if ui.selectable_label(selected, label).clicked() && !selected {
                                    next_source = Some(key);
                                }
                            }
                        });
                    });
            });
            if let Some(source_id) = next_source {
                self.selected_node_source_id = Some(source_id);
                self.page = 1;
                self.reload_nodes();
            }
        });
        ui.add_space(4.0);

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

        let mut card_action: Option<NodeCardAction> = None;
        panel_frame().show(ui, |ui| {
            let (columns, total_card_rows) =
                NodesViewModel::grid_layout(ui.available_width(), self.nodes.len());
            let selected_id = self.selected_proxy_node_id.clone();
            let switching_id = self.switching_node_id.clone();
            let is_busy = self.busy;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, 84.0, total_card_rows, |ui, row_range| {
                    for row_idx in row_range {
                        let start = row_idx * columns;
                        let end = (start + columns).min(self.nodes.len());
                        let row = &self.nodes[start..end];
                        ui.columns(columns, |column_uis| {
                            for (column_index, column_ui) in column_uis.iter_mut().enumerate() {
                                let Some(node) = row.get(column_index) else {
                                    continue;
                                };
                                let is_selected =
                                    selected_id.as_deref() == Some(&node.id);
                                let is_switching =
                                    switching_id.as_deref() == Some(&node.id);
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
        if let Some(action) = card_action {
            match action {
                NodeCardAction::Connect { id, name } => {
                    self.switch_proxy_node(id, name, "手动切换");
                }
                NodeCardAction::Speedtest(id) => self.speedtest_one(id),
                NodeCardAction::ShowDetail(id) => self.show_node(id),
                NodeCardAction::CheckIpRisk(id) => self.check_ip_risk_for(&id),
                NodeCardAction::Delete(id) => self.delete_node(id),
            }
        }
    }

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

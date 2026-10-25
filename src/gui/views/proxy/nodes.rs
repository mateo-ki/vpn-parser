use super::groups::{node_matches_group, visible_group_names};
use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    KernelGroup, ProxySortBy,
    ProxyView, ProxyViewRow,
};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, section_title,
};
use crate::gui::VpnApp;
use crate::models::NodeSummary;
use crate::proxy::{
    ProxyState, URL_TEST_GROUP,
};
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

impl VpnApp {
    pub(crate) fn ui_proxy_nodes(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("可用节点"), "多节点状态 · 快速测速 · 节点切换");

        if self.selected_proxy_source_id.as_deref() != Some("all") {
            self.selected_proxy_source_id = Some("all".to_string());
            self.reload_proxy_nodes();
        }
        if self.proxy_selected_group.is_empty() {
            self.proxy_selected_group = "全部".to_string();
        }

        // 默认只挂「全部」；其余组只有配置里真的有它才出现
        let visible_names = visible_group_names(
            self.proxy_config.smart_routing,
            self.proxy_config.override_yaml.as_deref(),
            &self.kernel_groups,
        );
        if !visible_names.iter().any(|name| name == &self.proxy_selected_group) {
            self.proxy_selected_group = "全部".to_string();
        }

        let is_auto = self.proxy_config.url_test;
        let running = self.proxy_status.state == ProxyState::Running;
        let current_group_now = if self.proxy_selected_group == "全部" || self.proxy_selected_group == "PROXY" {
            if is_auto {
                self.kernel_egress_name()
            } else {
                self.selected_proxy_node_id.as_ref().and_then(|id| {
                    self.proxy_nodes.iter().find(|n| &n.id == id).map(|n| n.name.clone())
                })
            }
        } else {
            self.kernel_groups
                .iter()
                .find(|g| g.name == self.proxy_selected_group)
                .and_then(|g| g.now.clone())
        };

        // ===================================================================
        // macOS Sequoia 极简晶体工具栏 (双行自适应排版：第 1 行地区过滤，第 2 行排序/搜索/节点统计)
        // ===================================================================
        Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    // --- 第 1 行：策略组重置胶囊 + 地区过滤胶囊 ---
                    ui.horizontal(|ui| {
                        // 若当前在按特定策略组筛选，展示可点击重置的轻量胶囊
                        if self.proxy_selected_group != "全部" {
                            let reset_resp = ui.add(
                                egui::Button::new(
                                    RichText::new(format!("📁 {} ✕", self.proxy_selected_group))
                                        .small()
                                        .strong()
                                        .color(theme::accent_strong()),
                                )
                                .fill(theme::accent_bg())
                                .stroke(Stroke::new(1.0_f32, theme::accent()))
                                .corner_radius(CornerRadius::same(10)),
                            );
                            if reset_resp
                                .on_hover_text("当前正在按此策略组过滤节点，点击重置并显示「全部」节点")
                                .clicked()
                            {
                                self.proxy_selected_group = "全部".to_string();
                                self.proxy_page = 1;
                            }
                            ui.separator();
                        }

                        // 地区过滤胶囊
                        icon_label(ui, Icon::Globe, 12.0, theme::text_secondary());
                        let regions = [
                            ("all", "全部"),
                            ("HK", "🇭🇰 香港"),
                            ("JP", "🇯🇵 日本"),
                            ("US", "🇺🇸 美国"),
                            ("SG", "🇸🇬 新加坡"),
                            ("TW", "🇹🇼 台湾"),
                            ("KR", "🇰🇷 韩国"),
                            ("OTHER", "其它"),
                        ];
                        for (code, label) in regions {
                            let is_sel = self.proxy_region_filter == code;
                            let (fill, stroke, text_color) = if is_sel {
                                (
                                    theme::accent_bg(),
                                    Stroke::new(1.0_f32, theme::accent()),
                                    theme::accent_strong(),
                                )
                            } else {
                                (
                                    theme::sidebar(),
                                    Stroke::new(1.0_f32, theme::border_subtle()),
                                    theme::muted(),
                                )
                            };
                            let btn = egui::Button::new(RichText::new(label).small().strong().color(text_color))
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(10));
                            if ui.add(btn).clicked() && !is_sel {
                                self.proxy_region_filter = code.to_string();
                                self.proxy_page = 1;
                            }
                        }
                    });

                    ui.add_space(5.0);
                    ui.separator();
                    ui.add_space(5.0);

                    // --- 第 2 行：排序模式 + 搜索框 + 节点统计 ---
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("排序:").small().color(theme::text_secondary()));
                        let is_name = self.proxy_sort_by == ProxySortBy::Name;
                        if ui
                            .add(icons::selectable_icon_btn(Icon::Sort, "名称", is_name))
                            .clicked()
                        {
                            self.proxy_sort_by = ProxySortBy::Name;
                            self.proxy_page = 1;
                        }
                        let is_lat = self.proxy_sort_by == ProxySortBy::Latency;
                        if ui
                            .add(icons::selectable_icon_btn(Icon::Lightning, "延迟", is_lat))
                            .clicked()
                        {
                            self.proxy_sort_by = ProxySortBy::Latency;
                            self.proxy_page = 1;
                        }

                        ui.separator();

                        ui.push_id("proxy_node_search_island", |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Search, 12.0, theme::muted());
                                let search_field = ui.add(
                                    egui::TextEdit::singleline(&mut self.proxy_search)
                                        .hint_text("搜索节点...")
                                        .desired_width(140.0),
                                );
                                if search_field.changed() {
                                    self.proxy_page = 1;
                                }
                                crate::gui::widgets::note_text_field(ui, &search_field);
                            });
                        });

                        let cleared = ui
                            .add_visible(
                                !self.proxy_search.is_empty(),
                                icons::icon_btn_square(Icon::Close, 11.0),
                            )
                            .clicked();
                        if cleared {
                            self.proxy_search.clear();
                            self.proxy_page = 1;
                        }

                        // 右侧动作组：节点计数统计
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            Frame::NONE
                                .fill(theme::sidebar())
                                .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                .corner_radius(CornerRadius::same(8))
                                .inner_margin(Margin::symmetric(8, 3))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("共 {} 节点", self.proxy_nodes.len()))
                                            .small()
                                            .strong()
                                            .color(theme::text_secondary()),
                                    );
                                });
                        });
                    });
                });
            });

        ui.add_space(6.0);

        // -------------------------------------------------------------------
        // 第 4 栏：节点网格渲染（按策略组、地区、搜索词过滤与虚拟滚动）
        // -------------------------------------------------------------------
        let group_key = self.proxy_selected_group.clone();
        let region_key = self.proxy_region_filter.clone();
        let search_key = self.proxy_search.trim().to_lowercase();
        let sort_key = self.proxy_sort_by;
        let rev = self.proxy_nodes_rev;
        let view = match self.proxy_view.take() {
            Some(cached)
                if cached.rev == rev
                    && cached.group == group_key
                    && cached.region == region_key
                    && cached.search == search_key
                    && cached.sort == sort_key =>
            {
                cached
            }
            _ => build_proxy_view(
                &self.proxy_nodes,
                &group_key,
                &self.kernel_groups,
                &region_key,
                &search_key,
                sort_key,
                rev,
            ),
        };

        let mut selected = None;
        let mut group_switch_member = None;
        let mut locked: Option<(String, String)> = None;
        let mut switch_to_manual = false;
        let mut menu_action: Option<NodeCardAction> = None;
        let status_msg: Option<String> = None;
        let selected_id = self.selected_proxy_node_id.clone();
        let switching_id = self.switching_node_id.clone();
        let is_busy = self.busy;

        let total_items = view.rows.len();
        let page_size = (self.proxy_page_size as usize).clamp(10, 200);
        let total_pages = NodesViewModel::total_pages(total_items as i64, page_size as u32);
        if self.proxy_page > total_pages {
            self.proxy_page = total_pages;
        }
        if self.proxy_page < 1 {
            self.proxy_page = 1;
        }

        let start_idx = ((self.proxy_page - 1) as usize * page_size).min(total_items);
        let end_idx = (start_idx + page_size).min(total_items);
        let paged_rows = if start_idx < total_items {
            &view.rows[start_idx..end_idx]
        } else {
            &[]
        };

        let (columns, total_card_rows) =
            NodesViewModel::grid_layout(ui.available_width(), paged_rows.len());

        if view.rows.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("proxy_nodes_empty_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(20.0);
                    card_frame().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(14.0);
                            icon_label(ui, Icon::Radar, 28.0, theme::muted());
                            ui.add_space(8.0);
                            let title = if self.proxy_nodes.is_empty() {
                                "暂无可用的代理节点"
                            } else {
                                "当前策略组与筛选条件下无匹配节点"
                            };
                            ui.label(RichText::new(title).strong().color(theme::text_primary()));
                            ui.add_space(4.0);
                            let hint = if self.proxy_nodes.is_empty() {
                                "可在「节点列表」中进行节点测速以获取可用节点，或尝试重新拉取订阅。"
                            } else {
                                "可尝试清除地区过滤、清空搜索词，或切换至「全部」策略组。"
                            };
                            ui.label(RichText::new(hint).small().color(theme::muted()));
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if !self.proxy_search.is_empty() || self.proxy_region_filter != "all" {
                                    if ui.button("清除筛选与搜索").clicked() {
                                        self.proxy_search.clear();
                                        self.proxy_region_filter = "all".into();
                                        self.proxy_page = 1;
                                    }
                                }
                                if self.proxy_selected_group != "全部" {
                                    if ui.button("查看全部节点").clicked() {
                                        self.proxy_selected_group = "全部".into();
                                        self.proxy_page = 1;
                                    }
                                }
                            });
                            ui.add_space(10.0);
                        });
                    });
                });
        } else {
            // 分页控制栏 (第 X / Y 页 · 共 Z 节点 · 上一页 / 下一页 · 容量选择器)
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "第 {} / {} 页 · 共 {} 节点",
                        self.proxy_page, total_pages, total_items
                    ))
                    .small()
                    .color(theme::text_secondary()),
                );

                if ui
                    .add_enabled(self.proxy_page > 1, egui::Button::new("上一页").small())
                    .clicked()
                {
                    self.proxy_page = self.proxy_page.saturating_sub(1).max(1);
                }
                if ui
                    .add_enabled(self.proxy_page < total_pages, egui::Button::new("下一页").small())
                    .clicked()
                {
                    self.proxy_page = (self.proxy_page + 1).min(total_pages);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("proxy_page_size_selector")
                        .selected_text(format!("每页 {} 条", self.proxy_page_size))
                        .width(95.0)
                        .show_ui(ui, |ui| {
                            for size in [20, 25, 30, 50, 100] {
                                if ui.selectable_value(&mut self.proxy_page_size, size, format!("{size} 条/页")).clicked() {
                                    self.proxy_page = 1;
                                }
                            }
                        });
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .id_salt("proxy_nodes_cards_scroll")
                .auto_shrink([false, false])
                .show_rows(ui, 64.0, total_card_rows, |ui, row_range| {
                    for row_idx in row_range {
                        let start = row_idx * columns;
                        let end = (start + columns).min(paged_rows.len());
                        let row = &paged_rows[start..end];
                        ui.columns(columns, |column_uis| {
                            for (column_index, column_ui) in column_uis.iter_mut().enumerate() {
                                let Some(entry) = row.get(column_index) else {
                                    continue;
                                };
                                let node = &self.proxy_nodes[entry.index];

                                // 活跃状态判定：优先匹配当前策略组活跃出口，或者全局选中节点
                                let is_active = if let Some(ref now) = current_group_now {
                                    now == &node.name
                                } else if is_auto {
                                    self.kernel_egress_name().as_deref() == Some(node.name.as_str())
                                } else {
                                    selected_id.as_deref() == Some(&node.id)
                                };
                                let is_focused = self.focused_node_id.as_deref() == Some(&node.id);
                                let is_switching = switching_id.as_deref() == Some(&node.id);

                                let card_tip = if is_active {
                                    format!("{}:{}\n当前活动出口（右键管理）", node.server, node.port)
                                } else if self.proxy_selected_group != "全部" && self.proxy_selected_group != "PROXY" {
                                    format!(
                                        "{}:{}\n左键点击选中，右键设为「{}」活动出口（双击亦可）",
                                        node.server, node.port, self.proxy_selected_group
                                    )
                                } else {
                                    format!(
                                        "{}:{}\n左键点击选中，右键设为活动节点（双击亦可）",
                                        node.server, node.port
                                    )
                                };

                                let response = compact_node_card(
                                    column_ui,
                                    node,
                                    is_active,
                                    is_focused,
                                    is_switching,
                                )
                                .on_hover_text(card_tip);

                                let clicked = response.clicked();
                                let double_clicked = response.double_clicked();

                                let node_id = node.id.clone();
                                let node_name = node.name.clone();
                                response.context_menu(|ui| {
                                    if !is_active {
                                        let label = if self.proxy_selected_group != "全部" && self.proxy_selected_group != "PROXY" {
                                            format!("🚀 设为「{}」活动出口", self.proxy_selected_group)
                                        } else {
                                            "🚀 设为活动节点".to_string()
                                        };
                                        if ui.button(label).clicked() {
                                            if is_auto {
                                                switch_to_manual = true;
                                            }
                                            selected = Some((node_id.clone(), node_name.clone()));
                                            if running
                                                && self.proxy_selected_group != "全部"
                                                && self.proxy_selected_group != "PROXY"
                                                && self.kernel_groups.iter().any(|g| g.name == self.proxy_selected_group)
                                            {
                                                group_switch_member = Some((self.proxy_selected_group.clone(), node_name.clone()));
                                            }
                                            ui.close_menu();
                                        }
                                    } else {
                                        ui.label(RichText::new("✓ 当前活动节点").small().color(theme::accent_strong()));
                                    }
                                    ui.separator();
                                    if ui.button("ℹ️ 查看详情").clicked() {
                                        menu_action = Some(NodeCardAction::ShowDetail(node_id.clone()));
                                        ui.close_menu();
                                    }
                                    if ui
                                        .add_enabled(!is_busy, egui::Button::new("⚡ 测速节点"))
                                        .clicked()
                                    {
                                        menu_action = Some(NodeCardAction::Speedtest(node_id.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button("🛡️ IP 检测").clicked() {
                                        menu_action = Some(NodeCardAction::CheckIpRisk(node_id.clone()));
                                        ui.close_menu();
                                    }
                                    if (is_auto || self.proxy_selected_group != "全部") && !is_active {
                                        ui.separator();
                                        if ui.button("锁定为出口（关闭自动选路）").clicked() {
                                            locked = Some((node_id.clone(), node_name.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                });

                                if double_clicked && !is_active {
                                    if is_auto {
                                        switch_to_manual = true;
                                    }
                                    selected = Some((node_id.clone(), node_name.clone()));
                                    if running
                                        && self.proxy_selected_group != "全部"
                                        && self.proxy_selected_group != "PROXY"
                                        && self.kernel_groups.iter().any(|g| g.name == self.proxy_selected_group)
                                    {
                                        group_switch_member = Some((self.proxy_selected_group.clone(), node_name.clone()));
                                    }
                                } else if clicked {
                                    menu_action = Some(NodeCardAction::Select {
                                        id: node_id.clone(),
                                        name: node_name.clone(),
                                    });
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                });

            if total_pages > 1 {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("第 {} / {} 页", self.proxy_page, total_pages))
                            .small()
                            .color(theme::text_secondary()),
                    );
                    if ui
                        .add_enabled(self.proxy_page > 1, egui::Button::new("上一页").small())
                        .clicked()
                    {
                        self.proxy_page = self.proxy_page.saturating_sub(1).max(1);
                    }
                    if ui
                        .add_enabled(self.proxy_page < total_pages, egui::Button::new("下一页").small())
                        .clicked()
                    {
                        self.proxy_page = (self.proxy_page + 1).min(total_pages);
                    }
                });
            }
        }

        if let Some(msg) = status_msg {
            self.status = msg;
        }
        if let Some(action) = menu_action {
            self.handle_node_card_action(action);
        }
        if let Some((grp, member)) = group_switch_member {
            self.select_kernel_member(grp, member);
        }
        if switch_to_manual {
            self.proxy_config.url_test = false;
            if running {
                self.save_proxy_config();
            }
        }
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

}

fn build_proxy_view(
    all: &[NodeSummary],
    group_name: &str,
    kernel_groups: &[KernelGroup],
    region: &str,
    search: &str,
    sort: ProxySortBy,
    rev: u64,
) -> ProxyView {
    const REGIONS: [&str; 8] = ["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"];
    let mut rows: Vec<ProxyViewRow> = Vec::with_capacity(all.len());
    for (index, node) in all.iter().enumerate() {
        if !node_matches_group(node, group_name, kernel_groups) {
            continue;
        }
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
            index,
            country,
            lower_name,
        });
    }
    match sort {
        ProxySortBy::Name => rows.sort_by(|a, b| a.lower_name.cmp(&b.lower_name)),
        ProxySortBy::Latency => rows.sort_by(|a, b| {
            let node_a = &all[a.index];
            let node_b = &all[b.index];
            (
                node_a.latency_ok != Some(true),
                node_a.latency_ms.unwrap_or(i64::MAX),
                &a.lower_name,
            )
                .cmp(&(
                    node_b.latency_ok != Some(true),
                    node_b.latency_ms.unwrap_or(i64::MAX),
                    &b.lower_name,
                ))
        }),
    }
    ProxyView {
        rev,
        group: group_name.to_owned(),
        region: region.to_owned(),
        search: search.to_owned(),
        sort,
        rows,
    }
}


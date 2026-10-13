use crate::gui::icons::{self, Icon};
use crate::gui::widgets::{
    card_frame, compact_node_card, latency_label, node_group_key, panel_frame, primary_btn,
    region_label, section_title, sort_label };
use crate::gui::VpnApp;
use crate::speedtest::SpeedtestIntensity;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use crate::gui::theme;

impl VpnApp {
    pub(crate) fn ui_nodes(&mut self, ui: &mut Ui) {
        section_title(ui, "节点池", "多源汇聚 · 强度测速 · 智能筛选清理");
        let mut filter_changed = false;
        Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border()))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;

                    ui.label(RichText::new("协议").strong().color(theme::text_secondary()));
                    let protocol_field = ui.add(
                        egui::TextEdit::singleline(&mut self.protocol_filter)
                            .desired_width(65.0)
                            .hint_text("vmess..."),
                    );
                    crate::gui::widgets::note_text_field(ui, &protocol_field);

                    ui.separator();
                    ui.label(RichText::new("关键词").strong().color(theme::text_secondary()));
                    let keyword_field = ui.add(
                        egui::TextEdit::singleline(&mut self.keyword)
                            .desired_width(85.0)
                            .hint_text("名称/IP..."),
                    );
                    crate::gui::widgets::note_text_field(ui, &keyword_field);

                    ui.separator();
                    ui.label(RichText::new("地区").strong().color(theme::text_secondary()));
                    egui::ComboBox::from_id_salt("region_filter")
                        .selected_text(region_label(&self.region_filter))
                        .show_ui(ui, |ui| {
                            for (val, lbl) in [
                                ("all", "全部地区"),
                                ("HK", "香港"),
                                ("JP", "日本"),
                                ("US", "美国"),
                                ("SG", "新加坡"),
                                ("TW", "台湾"),
                                ("KR", "韩国"),
                                ("OTHER", "其它"),
                            ] {
                                if ui
                                    .selectable_value(&mut self.region_filter, val.into(), lbl)
                                    .changed()
                                {
                                    filter_changed = true;
                                }
                            }
                        });

                    ui.separator();
                    ui.label(RichText::new("延迟").strong().color(theme::text_secondary()));
                    egui::ComboBox::from_id_salt("latency_filter")
                        .selected_text(latency_label(&self.latency_filter))
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut self.latency_filter, "all".into(), "全部")
                                .changed()
                            {
                                filter_changed = true;
                            }
                            if ui
                                .selectable_value(&mut self.latency_filter, "alive".into(), "可用")
                                .changed()
                            {
                                filter_changed = true;
                            }
                            if ui
                                .selectable_value(&mut self.latency_filter, "dead".into(), "不可用")
                                .changed()
                            {
                                filter_changed = true;
                            }
                            if ui
                                .selectable_value(
                                    &mut self.latency_filter,
                                    "untested".into(),
                                    "未测速",
                                )
                                .changed()
                            {
                                filter_changed = true;
                            }
                        });

                    ui.separator();
                    if ui
                        .checkbox(&mut self.only_available_nodes, "只看可用")
                        .on_hover_text("只显示真实代理验证成功的节点")
                        .changed()
                    {
                        filter_changed = true;
                    }

                    ui.separator();
                    ui.label(RichText::new("排序").strong().color(theme::text_secondary()));
                    egui::ComboBox::from_id_salt("sort_by")
                        .selected_text(sort_label(&self.sort_by))
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(&mut self.sort_by, "latency".into(), "延迟")
                                .changed()
                            {
                                filter_changed = true;
                            }
                            if ui
                                .selectable_value(&mut self.sort_by, "download".into(), "下载速度")
                                .changed()
                            {
                                filter_changed = true;
                            }
                            if ui
                                .selectable_value(&mut self.sort_by, "updated".into(), "更新时间")
                                .changed()
                            {
                                filter_changed = true;
                            }
                            if ui
                                .selectable_value(&mut self.sort_by, "name".into(), "名称")
                                .changed()
                            {
                                filter_changed = true;
                            }
                        });

                    ui.separator();
                    if ui
                        .add(icons::primary_icon_btn(Icon::Search, "查询"))
                        .clicked()
                    {
                        filter_changed = true;
                    }
                });
            });
        if filter_changed {
            self.page = 1;
            self.reload_node_view();
        }
        ui.add_space(4.0);
        let mut delete_dead = false;
        let mut prune_stale = false;
        let mut delete_invalid = false;
        let mut physical_dedupe = false;
        let mut clean_names = false;
        let mut dedupe = false;
        let mut clear_all = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("测速强度:");
            egui::ComboBox::from_id_salt("speedtest_intensity_select")
                .selected_text(self.speedtest_intensity.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.speedtest_intensity,
                        SpeedtestIntensity::Fast,
                        SpeedtestIntensity::Fast.label(),
                    )
                    .on_hover_text(SpeedtestIntensity::Fast.description());
                    ui.selectable_value(
                        &mut self.speedtest_intensity,
                        SpeedtestIntensity::Standard,
                        SpeedtestIntensity::Standard.label(),
                    )
                    .on_hover_text(SpeedtestIntensity::Standard.description());
                    ui.selectable_value(
                        &mut self.speedtest_intensity,
                        SpeedtestIntensity::Deep,
                        SpeedtestIntensity::Deep.label(),
                    )
                    .on_hover_text(SpeedtestIntensity::Deep.description());
                });

            if ui
                .add_enabled(!self.busy, primary_btn("测速本页"))
                .on_hover_text("对当前页节点执行真实代理握手和访问测试")
                .clicked()
            {
                self.speedtest_page();
            }
            let selected_group = self.node_groups.iter().find(|group| {
                self.selected_node_source_id.as_deref() == Some(node_group_key(group).as_str())
            });
            if ui
                .add_enabled(
                    !self.busy && selected_group.is_some(),
                    egui::Button::new("测速当前订阅"),
                )
                .on_hover_text("测速当前订阅中符合筛选条件的全部节点，不止本页")
                .clicked()
            {
                if let Some(group) = selected_group.cloned() {
                    self.speedtest_group(group);
                }
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("测速全部"))
                .on_hover_text("对全部订阅与未分组节点执行严格 Google 代理验证")
                .clicked()
            {
                self.speedtest_all_sources();
            }

            ui.separator();

            ui.add_enabled_ui(!self.busy, |ui| {
                ui.menu_button("节点清理 ▼", |ui| {
                    delete_dead = ui.button("删除超时/失败节点").clicked();
                    prune_stale = ui.button("🍂 淘汰过期失效节点").on_hover_text("清理 7 天以上持续不可用的僵尸节点").clicked();
                    delete_invalid = ui
                        .button("删除无效主机")
                        .on_hover_text("删除 127.0.0.1 / 内网 / localhost 等无效节点")
                        .clicked();
                    physical_dedupe = ui
                        .button("🧹 物理去重 (同 IP:Port)")
                        .on_hover_text("按真实服务器IP/主机与端口物理去重并保留最优记录")
                        .clicked();
                    dedupe = ui
                        .button("去除完全相同节点")
                        .on_hover_text("按协议、服务器、端口和凭据去重")
                        .clicked();
                    clean_names = ui
                        .button("✨ 净化节点名称")
                        .on_hover_text("去除广告推广乱码并标准化国旗与地区")
                        .clicked();
                    ui.separator();
                    clear_all = ui
                        .button(RichText::new("清空全部节点").color(theme::danger()))
                        .clicked();
                });
            });

            ui.separator();

            if ui
                .add_enabled(!self.busy, egui::Button::new("剪贴板导入"))
                .on_hover_text("从剪贴板解析订阅内容/节点链接并导入（不绑定订阅源）")
                .clicked()
            {
                self.import_from_clipboard();
            }

            ui.menu_button("导出节点 ▼", |ui| {
                ui.set_min_width(260.0);
                ui.label(RichText::new("导出参数设置").strong());
                ui.separator();
                ui.checkbox(&mut self.export_quality_only, "仅TCP连通节点")
                    .on_hover_text("TCP 能连 + 非内网/诱饵域名 + 延迟阈值");
                ui.horizontal(|ui| {
                    ui.label("延迟上限:");
                    ui.add(
                        egui::DragValue::new(&mut self.export_max_latency_ms)
                            .range(50..=5000)
                            .suffix(" ms"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("数量限制:");
                    ui.add(
                        egui::DragValue::new(&mut self.export_limit)
                            .range(1..=5000)
                            .suffix(" 条"),
                    );
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Clash 配置").clicked() {
                        self.open_export("clash");
                        ui.close_menu();
                    }
                    if ui.button("URI 链接").clicked() {
                        self.open_export("uri");
                        ui.close_menu();
                    }
                    if ui.button("JSON 格式").clicked() {
                        self.open_export("json");
                        ui.close_menu();
                    }
                });
            });

            ui.separator();

            if ui
                .add(primary_btn("推送可用节点"))
                .on_hover_text("从数据库读取全部真实代理验证成功的节点，并全量发布到服务器")
                .clicked()
            {
                self.publish_available_nodes();
            }
            if ui.button("刷新").clicked() {
                self.reload_node_view();
            }
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
            ui.label(RichText::new("订阅").strong().color(theme::text_secondary()));
            let groups = self.node_groups.clone();
            let mut next_source = None;
            ui.scope(|ui| {
                ui.style_mut().always_scroll_the_only_direction = true;
                egui::ScrollArea::horizontal()
                    .id_salt("node_source_tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for group in &groups {
                                let key = node_group_key(group);
                                let selected =
                                    self.selected_node_source_id.as_deref() == Some(&key);
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
            let pages = ((self.node_total as f32) / (self.page_size as f32))
                .ceil()
                .max(1.0) as u32;
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

        panel_frame().show(ui, |ui| {
                    let nodes = self.nodes.clone();
                    let columns = if ui.available_width() >= 920.0 {
                        4
                    } else if ui.available_width() >= 660.0 {
                        3
                    } else {
                        2
                    };
                    let total_card_rows = (nodes.len() + columns - 1) / columns;
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
                                    self.selected_proxy_node_id.as_deref() == Some(&node.id);
                                let is_switching =
                                    self.switching_node_id.as_deref() == Some(&node.id);
                                let response =
                                    compact_node_card(column_ui, node, is_selected, is_switching)
                                        .on_hover_text(format!(
                                            "{}:{}\n点击连接，右键管理",
                                            node.server, node.port
                                        ));
                                let connect = response.clicked();
                                response.context_menu(|ui| {
                                    if ui
                                        .add_enabled(!self.busy, egui::Button::new("测速节点"))
                                        .clicked()
                                    {
                                        self.speedtest_one(node.id.clone());
                                        ui.close_menu();
                                    }
                                    if ui.button("查看详情").clicked() {
                                        self.show_node(node.id.clone());
                                        ui.close_menu();
                                    }
                                    if ui.button("IP 检测").clicked() {
                                        self.check_ip_risk_for(&node.id);
                                        ui.close_menu();
                                    }
                                    ui.separator();
                                    if ui.button(RichText::new("删除节点").color(theme::danger())).clicked()
                                    {
                                        self.delete_node(node.id.clone());
                                        ui.close_menu();
                                    }
                                });
                                if connect && !is_selected {
                                    self.switch_proxy_node(
                                        node.id.clone(),
                                        node.name.clone(),
                                        "手动切换",
                                    );
                                }
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
        });
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

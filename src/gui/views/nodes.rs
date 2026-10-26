use crate::gui::danger_btn;
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, panel_frame, section_title, sort_label,
};
use crate::gui::VpnApp;
use crate::speedtest::SpeedtestIntensity;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

impl VpnApp {
    pub(crate) fn ui_nodes(&mut self, ui: &mut Ui) {
        // 弹框必须先于本页任何提前 return 渲染：节点池为空时下面会有一句 return，
        // 而「导入本地 YAML」正是空库机器上唯一还能往里灌东西的入口 —— 放在它后面就永远开不出门。
        self.ui_import_yaml_modal(ui.ctx());
        section_title(ui, self.tr("节点池"), "多源汇聚 · 强度测速 · 智能筛选清理");
        let mut filter_changed = false;
        let mut delete_dead = false;
        let mut prune_stale = false;
        let mut delete_invalid = false;
        let mut physical_dedupe = false;
        let mut clean_names = false;
        let mut dedupe = false;
        let mut clear_all = false;

        let lang = self.lang();
        let tr_available_only = crate::i18n::tr(lang, "仅看可用");
        let tr_untested = crate::i18n::tr(lang, "未测速");
        let tr_search_hint = crate::i18n::tr(lang, "搜索名称、服务器 IP...");
        let tr_source_search_hint = crate::i18n::tr(lang, "模糊搜索订阅源...");
        let tr_all_sources = crate::i18n::tr(lang, "全部订阅源");
        let tr_sources_label = crate::i18n::tr(lang, "订阅源");
        let tr_sort_label = crate::i18n::tr(lang, "排序:");
        let tr_import_export = crate::i18n::tr(lang, "📥 导入/导出 ▼");
        let tr_maintenance = crate::i18n::tr(lang, "🧹 维护清理 ▼");
        let tr_import_clip = crate::i18n::tr(lang, "📋 剪贴板导入节点/订阅");
        let tr_push_nodes = crate::i18n::tr(lang, "📤 推送可用节点到服务端");
        let tr_export_sub = crate::i18n::tr(lang, "导出订阅");
        let tr_export_clash = crate::i18n::tr(lang, "导出 Clash YAML 配置");
        let tr_export_uri = crate::i18n::tr(lang, "导出 URI 节点链接");
        let tr_export_json = crate::i18n::tr(lang, "导出 JSON 格式");
        let tr_prune_stale = crate::i18n::tr(lang, "🍂 淘汰 7 天过期失效节点");
        let tr_dedup = crate::i18n::tr(lang, "🧹 物理去重 (同 IP:Port)");

        // 节点类型统计：协议分布环形图 (Donut Chart) + 协议缩写徽标
        if let Some(s) = self.stats.as_ref() {
            if !s.protocol_stats.is_empty() {
                let mut next_proto = None;
                let mut next_lat = None;

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
                                next_proto = Some("".to_string());
                            }

                            for p in &s.protocol_stats {
                                let is_active = self.protocol_filter.eq_ignore_ascii_case(&p.protocol);
                                if crate::gui::widgets::protocol_stat_chip(ui, &p.protocol, p.count, is_active).clicked() {
                                    if is_active {
                                        next_proto = Some("".to_string());
                                    } else {
                                        next_proto = Some(p.protocol.clone());
                                    }
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
                                next_lat = Some(("all".to_string(), false));
                            }

                            let is_alive = self.only_available_nodes || self.latency_filter == "alive";
                            let (color_alive, stroke_alive) = if is_alive {
                                (theme::accent_bg(), Stroke::new(1.0_f32, theme::accent()))
                            } else {
                                (theme::sidebar(), Stroke::new(1.0_f32, theme::border_subtle()))
                            };
                            let label_alive = format!("● {tr_available_only}");
                            if ui.add(egui::Button::new(RichText::new(label_alive).small().strong()).fill(color_alive).stroke(stroke_alive).corner_radius(CornerRadius::same(10))).clicked() {
                                let new_only = !self.only_available_nodes;
                                let new_filter = if new_only { "alive".to_string() } else { "all".to_string() };
                                next_lat = Some((new_filter, new_only));
                            }

                            let is_untested = self.latency_filter == "untested";
                            let (color_untest, stroke_untest) = if is_untested {
                                (theme::accent_bg(), Stroke::new(1.0_f32, theme::accent()))
                            } else {
                                (theme::sidebar(), Stroke::new(1.0_f32, theme::border_subtle()))
                            };
                            let label_untest = format!("⚪ {tr_untested}");
                            if ui.add(egui::Button::new(RichText::new(label_untest).small().strong()).fill(color_untest).stroke(stroke_untest).corner_radius(CornerRadius::same(10))).clicked() {
                                let new_filter = if is_untested { "all".to_string() } else { "untested".to_string() };
                                next_lat = Some((new_filter, false));
                            }
                        });
                    });
                });

                if let Some(proto) = next_proto {
                    self.protocol_filter = proto;
                    filter_changed = true;
                }
                if let Some((lat, only_avail)) = next_lat {
                    self.latency_filter = lat;
                    self.only_available_nodes = only_avail;
                    filter_changed = true;
                }

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
                // 第一行：搜索、订阅源筛选与操作菜单组（批量测速、维护清理、导入/导出）
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    self.render_nodes_toolbar_search_and_filters(
                        ui,
                        &mut filter_changed,
                        &tr_search_hint,
                        &tr_sources_label,
                        &tr_all_sources,
                        &tr_source_search_hint,
                    );

                    ui.separator();

                    self.render_nodes_toolbar_actions(
                        ui,
                        &mut prune_stale,
                        &mut physical_dedupe,
                        &mut clean_names,
                        &mut dedupe,
                        &mut delete_dead,
                        &mut delete_invalid,
                        &mut clear_all,
                        &tr_import_export,
                        &tr_import_clip,
                        &tr_push_nodes,
                        &tr_export_sub,
                        &tr_export_clash,
                        &tr_export_uri,
                        &tr_export_json,
                        &tr_maintenance,
                        &tr_prune_stale,
                        &tr_dedup,
                    );
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                // 第二行：地区快速筛选胶囊行 + 排序下拉（紧随「其它」后面）
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
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

                    ui.separator();

                    // 排序下拉（紧随「其它」胶囊）
                    ui.label(RichText::new(tr_sort_label).small().color(theme::muted()));
                    egui::ComboBox::from_id_salt("sort_by")
                        .selected_text(sort_label(&self.sort_by))
                        .show_ui(ui, |ui| {
                            let tr_lat = self.tr("延迟最低");
                            let tr_spd = self.tr("下载速度");
                            let tr_upd = self.tr("更新时间");
                            let tr_nam = self.tr("节点名称");
                            for (val, lbl) in [
                                ("latency", tr_lat),
                                ("download", tr_spd),
                                ("updated", tr_upd),
                                ("name", tr_nam),
                            ] {
                                if ui.selectable_value(&mut self.sort_by, val.into(), lbl).changed() {
                                    filter_changed = true;
                                }
                            }
                        });
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
            self.focused_node_id.as_deref(),
            self.switching_node_id.as_deref(),
            self.busy,
        );
        if let Some(action) = action {
            self.handle_node_card_action(action);
        }

    }

    pub(crate) fn render_nodes_toolbar_search_and_filters(
        &mut self,
        ui: &mut Ui,
        filter_changed: &mut bool,
        tr_search_hint: &str,
        tr_sources_label: &str,
        tr_all_sources: &str,
        tr_source_search_hint: &str,
    ) {
        // 搜索框
        icon_label(ui, Icon::Search, 13.0, theme::text_secondary());
        let search_field = ui.add(
            egui::TextEdit::singleline(&mut self.keyword)
                .desired_width(140.0)
                .hint_text(tr_search_hint),
        );
        crate::gui::widgets::note_text_field(ui, &search_field);
        let enter_pressed = search_field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let lost_focus = search_field.lost_focus();
        if enter_pressed || (lost_focus && self.keyword != self.keyword_applied) {
            self.keyword_applied = self.keyword.clone();
            *filter_changed = true;
        }
        if !self.keyword.is_empty() {
            if ui.small_button("✕").on_hover_text("清除搜索关键词").clicked() {
                self.keyword.clear();
                self.keyword_applied.clear();
                *filter_changed = true;
            }
        }

        ui.separator();

        // 订阅源下拉筛选（支持下拉模糊搜索）
        ui.label(RichText::new(format!("{tr_sources_label}:")).small().color(theme::muted()));
        let source_label = if self.source_filter.is_empty() {
            tr_all_sources.to_string()
        } else if let Some(s) = self.sources.iter().find(|s| s.id == self.source_filter) {
            let name = &s.name;
            if name.chars().count() > 8 {
                format!("{tr_sources_label}: {}…", name.chars().take(8).collect::<String>())
            } else {
                format!("{tr_sources_label}: {name}")
            }
        } else {
            tr_all_sources.to_string()
        };

        egui::ComboBox::from_id_salt("node_source_filter")
            .selected_text(source_label)
            .width(130.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show_ui(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Search, 11.0, theme::muted());
                    let search_edit = ui.add(
                        egui::TextEdit::singleline(&mut self.source_search_query)
                            .desired_width(140.0)
                            .hint_text(tr_source_search_hint),
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
                    .id_salt("nodes_source_filter_popup_scroll")
                    .max_height(220.0)
                    .show(ui, |ui| {
                        if ui.selectable_label(self.source_filter.is_empty(), tr_all_sources).clicked() {
                            if !self.source_filter.is_empty() {
                                self.source_filter.clear();
                                *filter_changed = true;
                            }
                            ui.memory_mut(|m| m.close_popup());
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
                                    *filter_changed = true;
                                }
                                ui.memory_mut(|m| m.close_popup());
                            }
                        }

                        if matched == 0 && !q.is_empty() {
                            ui.label(RichText::new(self.tr("无匹配订阅源")).small().color(theme::muted()));
                        }
                    });
            });
    }

    pub(crate) fn render_nodes_toolbar_actions(
        &mut self,
        ui: &mut Ui,
        prune_stale: &mut bool,
        physical_dedupe: &mut bool,
        clean_names: &mut bool,
        dedupe: &mut bool,
        delete_dead: &mut bool,
        delete_invalid: &mut bool,
        clear_all: &mut bool,
        tr_import_export: &str,
        tr_import_clip: &str,
        tr_push_nodes: &str,
        tr_export_sub: &str,
        tr_export_clash: &str,
        tr_export_uri: &str,
        tr_export_json: &str,
        tr_maintenance: &str,
        tr_prune_stale: &str,
        tr_dedup: &str,
    ) {
        // 1. 测速菜单 / 停止测速
        if self.speedtest_busy {
            if ui
                .add(danger_btn("⏹ 停止测速"))
                .on_hover_text("正在批量测速中，点击立即停止并清理测速任务")
                .clicked()
            {
                self.cancel_speedtest();
            }
        } else {
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
        }

        ui.separator();

        // 2. 清理维护菜单
        ui.add_enabled_ui(!self.busy, |ui| {
            ui.menu_button(tr_maintenance, |ui| {
                *prune_stale = ui
                    .button(tr_prune_stale)
                    .on_hover_text("清理持续不可用的僵尸节点")
                    .clicked();
                *physical_dedupe = ui
                    .button(tr_dedup)
                    .on_hover_text("按真实服务器IP/主机与端口物理去重并保留最优记录")
                    .clicked();
                *clean_names = ui
                    .button("✨ 净化节点名称")
                    .on_hover_text("去除广告推广乱码并标准化国旗与地区")
                    .clicked();
                *dedupe = ui
                    .button("去除完全相同节点")
                    .on_hover_text("按协议、服务器、端口和凭据去重")
                    .clicked();
                *delete_dead = ui.button("删除当前超时/失败节点").clicked();
                *delete_invalid = ui.button("删除内网/无效主机").clicked();
                ui.separator();
                *clear_all = ui.button("清空全部节点").clicked();
            });
        });

        ui.separator();

        // 3. 导入与导出菜单
        ui.menu_button(tr_import_export, |ui| {
            let tr_import_local = self.tr("📁 导入本地 YAML / 配置文件...");
            if ui
                .add_enabled(!self.busy, egui::Button::new(tr_import_local))
                .clicked()
            {
                self.show_import_yaml_modal = true;
                ui.close_menu();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(tr_import_clip))
                .clicked()
            {
                self.import_from_clipboard();
                ui.close_menu();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(tr_push_nodes))
                .clicked()
            {
                self.publish_available_nodes();
                ui.close_menu();
            }
            ui.separator();
            ui.label(RichText::new(tr_export_sub).small().color(theme::muted()));
            if ui.button(tr_export_clash).clicked() {
                self.open_export("clash");
                ui.close_menu();
            }
            if ui.button(tr_export_uri).clicked() {
                self.open_export("uri");
                ui.close_menu();
            }
            if ui.button(tr_export_json).clicked() {
                self.open_export("json");
                ui.close_menu();
            }
        });
    }

    pub(crate) fn ui_import_yaml_modal(&mut self, ctx: &egui::Context) {
        if !self.show_import_yaml_modal {
            return;
        }

        let tr_modal_title = self.tr("导入本地 YAML / 配置文件").to_string();
        let tr_modal_subtitle = self.tr("支持 Clash / Mihomo YAML 配置文件，可直接输入路径或将文件拖拽至窗口。").to_string();
        let tr_path_label = self.tr("配置文件绝对路径：").to_string();
        let tr_path_hint = self.tr("例如 /etc/clash/config.yaml 或 D:\\config.yaml").to_string();
        let tr_clear_path = self.tr("清空路径").to_string();
        let tr_reg_source = self.tr("注册为本地订阅源（便于在节点池中按来源筛选与管理）").to_string();
        let tr_source_name_label = format!("{}:", self.tr("订阅源名称"));
        let tr_source_name_hint = self.tr("默认取文件名").to_string();
        let tr_cancel = self.tr("取消").to_string();
        let tr_confirm_import = self.tr("确认导入").to_string();

        // 监听拖拽进窗口的文件 (Drag & Drop)
        ctx.input(|i| {
            if let Some(file) = i.raw.dropped_files.first() {
                if let Some(path) = &file.path {
                    self.import_yaml_path = path.to_string_lossy().to_string();
                    if self.import_yaml_source_name.is_empty() {
                        if let Some(stem) = path.file_stem() {
                            self.import_yaml_source_name = format!("本地: {}", stem.to_string_lossy());
                        }
                    }
                }
            }
        });

        let mut close_modal = false;
        let mut do_import = false;

        egui::Modal::new(egui::Id::new("import_yaml_modal")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(&tr_modal_title);
            ui.add_space(4.0);
            ui.label(
                RichText::new(&tr_modal_subtitle)
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new(&tr_path_label).strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.import_yaml_path)
                        .desired_width(ui.available_width() - 68.0)
                        .hint_text(&tr_path_hint),
                );
                if ui
                    .add(crate::gui::icons::icon_btn_square(
                        crate::gui::icons::Icon::Folder,
                        12.0,
                    ))
                    .on_hover_text(self.tr("浏览并选择本地文件"))
                    .clicked()
                {
                    self.pick_local_file_for_node_import();
                }
                if ui
                    .add(crate::gui::icons::icon_btn_square(
                        crate::gui::icons::Icon::Trash,
                        11.0,
                    ))
                    .on_hover_text(&tr_clear_path)
                    .clicked()
                {
                    self.import_yaml_path.clear();
                }
            });

            ui.add_space(8.0);
            ui.checkbox(&mut self.import_yaml_as_source, &tr_reg_source);
            if self.import_yaml_as_source {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&tr_source_name_label).small().color(theme::muted()));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.import_yaml_source_name)
                            .desired_width(220.0)
                            .hint_text(&tr_source_name_hint),
                    );
                });
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(&tr_cancel).clicked() {
                        close_modal = true;
                    }
                    let path_empty = self.import_yaml_path.trim().is_empty();
                    let btn = egui::Button::new(RichText::new(&tr_confirm_import).strong())
                        .fill(theme::accent())
                        .corner_radius(CornerRadius::same(6));
                    if ui.add_enabled(!path_empty && !self.busy, btn).clicked() {
                        do_import = true;
                        close_modal = true;
                    }
                });
            });
        });

        if do_import {
            let path = self.import_yaml_path.clone();
            let as_source = self.import_yaml_as_source;
            let source_name = self.import_yaml_source_name.clone();
            self.import_from_local_yaml_file(path, as_source, source_name);
        }

        if close_modal {
            self.show_import_yaml_modal = false;
        }
    }
}

pub fn render_node_cards_grid(
    ui: &mut Ui,
    nodes: &[crate::models::NodeSummary],
    selected_proxy_node_id: Option<&str>,
    focused_node_id: Option<&str>,
    switching_node_id: Option<&str>,
    is_busy: bool,
) -> Option<NodeCardAction> {
    let mut card_action: Option<NodeCardAction> = None;
    panel_frame().show(ui, |ui| {
        let (columns, total_card_rows) =
            NodesViewModel::grid_layout(ui.available_width(), nodes.len());
        egui::ScrollArea::vertical()
            .id_salt("main_nodes_cards_scroll")
            .auto_shrink([false, false])
            .show_rows(ui, 64.0, total_card_rows, |ui, row_range| {
                for row_idx in row_range {
                    let start = row_idx * columns;
                    let end = (start + columns).min(nodes.len());
                    let row = &nodes[start..end];
                    ui.columns(columns, |column_uis| {
                        for (column_index, column_ui) in column_uis.iter_mut().enumerate() {
                            let Some(node) = row.get(column_index) else {
                                continue;
                            };
                            let is_active =
                                selected_proxy_node_id == Some(&node.id);
                            let is_focused =
                                focused_node_id == Some(&node.id);
                            let is_switching =
                                switching_node_id == Some(&node.id);
                            let card_tip = if is_active {
                                format!("{}:{}\n当前活动出口（右键管理）", node.server, node.port)
                            } else {
                                format!(
                                    "{}:{}\n左键点击选中，右键设为活动节点（双击亦可）",
                                    node.server, node.port
                                )
                            };
                            let response =
                                compact_node_card(column_ui, node, is_active, is_focused, is_switching)
                                    .on_hover_text(card_tip);
                            let clicked = response.clicked();
                            let double_clicked = response.double_clicked();
                            response.context_menu(|ui| {
                                if !is_active {
                                    if ui.button("🚀 设为活动节点").clicked() {
                                        card_action = Some(NodeCardAction::Connect {
                                            id: node.id.clone(),
                                            name: node.name.clone(),
                                        });
                                        ui.close_menu();
                                    }
                                } else {
                                    ui.label(RichText::new("✓ 当前活动节点").small().color(theme::accent_strong()));
                                }
                                ui.separator();
                                if ui
                                    .add_enabled(!is_busy, egui::Button::new("⚡ 测速节点"))
                                    .clicked()
                                {
                                    card_action = Some(NodeCardAction::Speedtest(node.id.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("ℹ️ 查看详情").clicked() {
                                    card_action = Some(NodeCardAction::ShowDetail(node.id.clone()));
                                    ui.close_menu();
                                }
                                if ui.button("🛡️ IP 检测").clicked() {
                                    card_action = Some(NodeCardAction::CheckIpRisk(node.id.clone()));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("🗑️ 删除节点").clicked() {
                                    card_action = Some(NodeCardAction::Delete(node.id.clone()));
                                    ui.close_menu();
                                }
                            });
                            if double_clicked && !is_active {
                                card_action = Some(NodeCardAction::Connect {
                                    id: node.id.clone(),
                                    name: node.name.clone(),
                                });
                            } else if clicked {
                                card_action = Some(NodeCardAction::Select {
                                    id: node.id.clone(),
                                    name: node.name.clone(),
                                });
                            }
                        }
                    });
                    ui.add_space(6.0);
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
                egui::ScrollArea::both()
                    .id_salt("node_detail_modal_scroll")
                    .max_height(400.0)
                    .show(ui, |ui| {
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

#[cfg(test)]
mod empty_pool_modal_tests {
    use super::*;
    use crate::api::ApiServerController;
    use crate::api::AppState;
    use crate::collector::Collector;
    use crate::db::Database;
    use crate::kernel::KernelManager;
    use crate::proxy::ProxyManager;
    use crate::speedtest::SpeedTester;
    use crate::tasks::TaskTracker;
    use std::sync::Arc;

    /// 一个空库的真 `VpnApp`。新装机器上节点池就是空的，而「导入本地 YAML」恰恰是那种
    /// 机器上唯一还能往里灌东西的入口 —— 所以零节点时弹框必须开得出门。
    fn app_with_an_empty_library() -> VpnApp {
        let rt = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let dir = std::env::temp_dir().join(format!("vp-nodes-modal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = rt
            .block_on(Database::connect(&dir.join("vpn-parser.db").to_string_lossy()))
            .unwrap();
        let tasks = TaskTracker::default();
        let collector = Collector::new(db.clone(), tasks.clone()).unwrap();
        let speedtester = SpeedTester::new(db.clone(), tasks.clone());
        let proxy_manager = ProxyManager::new(rt.block_on(db.get_proxy_config()).unwrap());
        let kernel_manager = KernelManager;
        let api_controller = ApiServerController::new(
            AppState {
                db: db.clone(),
                collector: collector.clone(),
                speedtester: speedtester.clone(),
                tasks: tasks.clone(),
                kernel_manager: KernelManager,
                proxy_manager: proxy_manager.clone(),
            },
            "127.0.0.1:0".into(),
            None,
        );
        VpnApp::new(
            rt,
            db,
            collector,
            speedtester,
            tasks,
            kernel_manager,
            proxy_manager,
            api_controller,
            None,
        )
    }

    /// 读某一图层在这一帧画出来的全部文字。
    ///
    /// 为什么按图层读而不是读 `FullOutput::shapes`：Area（弹框、窗口）写在自己的图层里，
    /// 单帧的 `shapes` 收不到它 —— 实测一个直接开的 `Modal` 用那种读法读到的是空串。
    /// 所以这条判据必须先过下面那个对照测试，否则"弹框没画出来"是探针自己的假信号。
    fn layer_text(ctx: &egui::Context, layer: egui::LayerId) -> String {
        ctx.graphics(|g| {
            let mut text = String::new();
            if let Some(list) = g.get(layer) {
                for entry in list.all_entries() {
                    if let egui::Shape::Text(t) = &entry.shape {
                        text.push_str(&t.galley.text());
                        text.push(' ');
                    }
                }
            }
            text
        })
    }

    fn screen_input() -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        ));
        input
    }

    /// 弹框的图层就按它 Area 的 id 寻址（`Modal::default_area` 用的是 `Order::Foreground`）。
    fn modal_layer() -> egui::LayerId {
        egui::LayerId::new(egui::Order::Foreground, egui::Id::new("import_yaml_modal"))
    }

    /// 跑两帧真 `ui_nodes`，返回（节点池那一层的文字, 弹框那一层的文字）。
    ///
    /// 两帧不是冗余：egui 的 Area 第一帧只走 sizing pass（`AreaState::load` 取不到
    /// 状态时 `sizing_pass = true`），内容不落地 —— 单帧探针读任何弹框都只会读到空串。
    fn frame_texts(app: &mut VpnApp) -> (String, String) {
        let ctx = egui::Context::default();
        let mut panel = String::new();
        let mut modal = String::new();
        for _ in 0..2 {
            let mut p = String::new();
            let mut m = String::new();
            let _ = ctx.run(screen_input(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let panel_layer = ui.layer_id();
                    app.ui_nodes(ui);
                    p = layer_text(ui.ctx(), panel_layer);
                    m = layer_text(ui.ctx(), modal_layer());
                });
            });
            panel = p;
            modal = m;
        }
        (panel, modal)
    }

    /// 探针自检：同一个图层地址、同一个 id，直接开一个 Modal 必须读得到它的字。
    /// 这条要红了，上面那条"弹框没画出来"就一律不算结论。
    #[test]
    fn the_probe_actually_sees_a_modal_on_that_layer() {
        let ctx = egui::Context::default();
        let mut seen = String::new();
        for _ in 0..2 {
            let mut out = String::new();
            let _ = ctx.run(screen_input(), |ctx| {
                egui::Modal::new(egui::Id::new("import_yaml_modal")).show(ctx, |ui| {
                    ui.label("CONTROL_MARK");
                });
                out = layer_text(ctx, modal_layer());
            });
            seen = out;
        }
        assert!(
            seen.contains("CONTROL_MARK"),
            "连一个直接开的 Modal 都读不到，说明探针坏了，不是弹框坏了：{seen:?}"
        );
    }

    #[test]
    fn the_yaml_import_modal_opens_even_with_an_empty_node_pool() {
        let mut app = app_with_an_empty_library();
        assert!(
            app.nodes.is_empty(),
            "夹具不该有节点，否则测不到新装机那个形状"
        );
        app.show_import_yaml_modal = true;
        let (panel, modal) = frame_texts(&mut app);
        assert!(
            panel.contains("当前筛选条件下没有节点"),
            "夹具没跑到空列表那条分支上（面板文字：{panel}）"
        );
        assert!(
            modal.contains("导入本地 YAML / 配置文件"),
            "零节点时点「导入本地 YAML」弹框没画出来（弹框层文字：{modal:?}）"
        );
        assert!(
            modal.contains("确认导入"),
            "弹框画了但缺「确认导入」（弹框层文字：{modal}）"
        );
    }
}

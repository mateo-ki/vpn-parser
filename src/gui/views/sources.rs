use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::types::SourcesPanel;
use crate::gui::widgets::{
    card_frame, danger_btn, header_cell, primary_btn, render_fetch_ticks, section_title,
    source_status_failed, status_badge };
use crate::gui::VpnApp;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use crate::gui::theme;

impl VpnApp {
    pub(crate) fn ui_sources(&mut self, ui: &mut Ui) {
        section_title(ui, "订阅源", "增删改查 · 单源/全部拉取");
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.sources_panel == SourcesPanel::List, "订阅列表")
                .clicked()
            {
                self.sources_panel = SourcesPanel::List;
            }
            if ui
                .selectable_label(self.sources_panel == SourcesPanel::Results, "订阅拉取结果")
                .clicked()
            {
                self.sources_panel = SourcesPanel::Results;
            }
        });
        ui.add_space(6.0);
        if self.sources_panel == SourcesPanel::Results {
            self.ui_fetch_results(ui);
            return;
        }
        ui.horizontal(|ui| {
            if ui.add(primary_btn("新增订阅源")).clicked() {
                self.open_create_form();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new("导入订阅源"))
                .on_hover_text("从剪贴板导入 JSON 格式订阅源，自动跳过重复项")
                .clicked()
            {
                self.import_sources_from_clipboard();
            }
            if ui.button("导出订阅源").clicked() {
                self.export_sources_to_clipboard();
            }
            if ui
                .add_enabled(!self.busy, primary_btn("拉取全部启用源"))
                .on_hover_text("后台拉取期间仍可删除/编辑")
                .clicked()
            {
                self.fetch_all();
            }
            if ui
                .add_enabled(
                    !self.busy && self.subscription_proxy_url().is_some(),
                    egui::Button::new("代理拉取全部"),
                )
                .on_hover_text("通过当前运行中的本地代理拉取全部启用源")
                .clicked()
            {
                self.fetch_all_via_proxy();
            }
            if ui.button("刷新").clicked() {
                self.reload_all();
            }
            let prune_enabled = !self.busy && !self.homogeneity_busy;
            let prune_clicked = if self.homogeneity_busy {
                ui.add_enabled(false, icons::button_with_icon(Icon::Search, "分析中...", theme::muted())).clicked()
            } else {
                ui.add_enabled(
                    prune_enabled,
                    icons::button_with_icon(Icon::Lightning, "反向剔除同质化源", theme::warning()),
                )
                .on_hover_text("分析所有源的重合率，识别出无独有可用节点的同质化冗余源；在弹窗里勾选要处理的源，再批量禁用或彻底删除")
                .clicked()
            };
            if prune_clicked {
                self.analyze_source_homogeneity();
            }
            if self.settings.auto_fetch_enabled {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Sync, 12.0, theme::success());
                    ui.label(
                        RichText::new(format!(
                            "自动拉取 每 {} 分钟",
                            self.settings.auto_fetch_interval_min
                        ))
                        .color(theme::success())
                        .small(),
                    );
                });
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Gist 发现").weak());
            for (query, checked) in crate::collector::GIST_DISCOVERY_QUERIES
                .iter()
                .zip(self.gist_query_flags.iter_mut())
            {
                ui.add_enabled(
                    !self.busy,
                    egui::Checkbox::new(checked, query.trim_end_matches("//")),
                )
                .on_hover_text(format!("Gist 发现搜索词“{query}”，按勾选的词逐个扫描"));
            }
            ui.add(
                egui::DragValue::new(&mut self.gist_pages)
                    .range(1..=300)
                    .suffix(" 页"),
            )
            .on_hover_text("Gist 发现最多扫描的页数（搜索结果每页 10 条，按最近更新排序，每个勾选的查询词各扫这么多页）");
            if ui
                .add_enabled(!self.busy, egui::Button::new("发现 Gist 节点"))
                .on_hover_text("扫描 gist.github.com 搜索结果（按勾选的查询词），能解析出节点的 gist 自动注册为订阅源，节点进节点列表；已见过的自动跳过，删掉的源不会复活")
                .clicked()
            {
                self.discover_gists();
            }
        });
        ui.add_space(8.0);

        card_frame().show(ui, |ui| {
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("sources_grid")
                        .striped(true)
                        .num_columns(6)
                        .min_col_width(70.0)
                        .show(ui, |ui| {
                            header_cell(ui, "名称");
                            header_cell(ui, "URL 模板");
                            header_cell(ui, "状态");
                            header_cell(ui, "最近拉取");
                            header_cell(ui, "健康");
                            header_cell(ui, "操作");
                            ui.end_row();

                            let sources = self.sources.clone();
                            for s in sources {
                                let source_active = self.active_source_fetches.contains(&s.id)
                                    || self.tasks.is_source_active(&s.id);
                                let fetch_operation_allowed =
                                    !self.busy || self.tasks.has_active_fetch();
                                let retry =
                                    s.last_status.as_deref().is_some_and(source_status_failed);
                                ui.label(RichText::new(&s.name).strong());
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&s.url_template).small().color(theme::accent()),
                                    )
                                    .wrap(),
                                );
                                let badge = if s.enabled { "启用" } else { "停用" };
                                let color = if s.enabled { theme::success() } else { theme::danger() };
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(badge).color(color));
                                    if s.push_disabled {
                                        ui.label(RichText::new("不推送").small().color(theme::warning()));
                                    }
                                    ui.label(
                                        RichText::new(
                                            s.last_status.clone().unwrap_or_else(|| "-".into()),
                                        )
                                        .small()
                                        .color(theme::muted()),
                                    );
                                    if s.fail_streak >= crate::collector::DEGRADED_FAIL_STREAK {
                                        ui.label(
                                            RichText::new(format!(
                                                "连续网络超时 {} 次，仅单次尝试",
                                                s.fail_streak
                                            ))
                                            .small()
                                            .color(theme::danger()),
                                        );
                                    }
                                    if let Some(reports) = &self.homogeneity_reports {
                                        if let Some(rep) = reports.iter().find(|r| r.source_id == s.id) {
                                            if rep.is_redundant {
                                                status_badge(ui, &format!("⚠️ 同质化 {:.0}%", rep.overlap_rate_pct), theme::danger());
                                            } else if rep.unique_alive_nodes > 0 {
                                                status_badge(ui, &format!("独有可用 {}", rep.unique_alive_nodes), theme::success());
                                            }
                                        }
                                    }
                                });
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(
                                            s.last_fetch_at
                                                .map(|t| {
                                                    t.with_timezone(&chrono::Local)
                                                        .format("%m-%d %H:%M")
                                                        .to_string()
                                                })
                                                .unwrap_or_else(|| "-".into()),
                                        )
                                        .small(),
                                    );
                                    ui.label(
                                        RichText::new(format!("最近解析 {}", s.last_node_count))
                                            .small()
                                            .color(theme::muted()),
                                    );
                                    render_fetch_ticks(ui, self.fetch_history.get(&s.id));
                                });
                                ui.vertical(|ui| {
                                    // 存活率：拉到的节点里有多少测得通。0 节点的死源在这一列
                                    // 一眼就能看出来，不用逐个点开。
                                    let total = s.last_node_count.max(0);
                                    let alive = self
                                        .node_groups
                                        .iter()
                                        .find(|group| {
                                            group.source_id.as_deref() == Some(s.id.as_str())
                                        })
                                        .map(|group| group.alive)
                                        .unwrap_or(0);
                                    if total == 0 {
                                        ui.label(
                                            RichText::new("无节点")
                                                .small()
                                                .color(theme::warning()),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!("测通 {alive}/{total}"))
                                                .small()
                                                .color(theme::text_secondary()),
                                        );
                                    }
                                });
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            fetch_operation_allowed && !source_active,
                                            egui::Button::new(match (source_active, retry) {
                                                (true, true) => "重试中...",
                                                (true, false) => "拉取中...",
                                                (false, true) => "重试",
                                                (false, false) => "拉取",
                                            }),
                                        )
                                        .clicked()
                                    {
                                        self.fetch_one(s.id.clone());
                                    }
                                    if ui
                                        .add_enabled(
                                            fetch_operation_allowed
                                                && !source_active
                                                && self.subscription_proxy_url().is_some(),
                                            egui::Button::new("代理拉取"),
                                        )
                                        .on_hover_text("通过当前运行中的本地代理拉取此订阅源")
                                        .clicked()
                                    {
                                        self.fetch_one_via_proxy(s.id.clone());
                                    }

                                    if ui.button("编辑").clicked() {
                                        self.open_edit_form(&s);
                                    }
                                    if ui
                                        .button(if s.enabled { "禁用" } else { "启用" })
                                        .on_hover_text(if s.enabled {
                                            "禁用后不会参与批量拉取"
                                        } else {
                                            "启用后可参与批量拉取"
                                        })
                                        .clicked()
                                    {
                                        self.toggle_source_enabled(s.id.clone(), !s.enabled);
                                    }

                                    if ui
                                        .button(if s.push_disabled {
                                            "恢复推送"
                                        } else {
                                            "不推送"
                                        })
                                        .on_hover_text(if s.push_disabled {
                                            "恢复后该源的节点会参与“推送可用节点”"
                                        } else {
                                            "标记后该源的节点不参与“推送可用节点”（仍照常拉取和测速）"
                                        })
                                        .clicked()
                                    {
                                        self.toggle_source_push(s.id.clone(), !s.push_disabled);
                                    }

                                    if ui.button(RichText::new("删除").color(theme::danger())).clicked() {
                                        self.delete_source(s.id.clone());
                                    }
                                });
                                ui.end_row();
                            }
                        });
                });
        });
    }

    pub(crate) fn ui_fetch_results(&self, ui: &mut Ui) {
        self.ui_task_panel(ui);
        ui.add_space(8.0);
        card_frame().show(ui, |ui| {
            if self.fetch_results.is_empty() {
                ui.label(RichText::new("暂无拉取结果").color(theme::muted()));
                return;
            }
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("fetch_results_grid")
                        .striped(true)
                        .num_columns(5)
                        .show(ui, |ui| {
                            for title in ["订阅", "结果", "节点数", "最终 URL", "详情"] {
                                header_cell(ui, title);
                            }
                            ui.end_row();
                            for result in &self.fetch_results {
                                ui.label(RichText::new(&result.source_name).strong());
                                ui.label(
                                    RichText::new(if result.success { "成功" } else { "失败" })
                                        .color(if result.success { theme::success() } else { theme::danger() }),
                                );
                                ui.label(result.node_count.to_string());
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&result.resolved_url).small().color(theme::accent()),
                                    )
                                    .wrap(),
                                );
                                ui.label(&result.message);
                                ui.end_row();
                            }
                        });
                });
        });
    }

    pub(crate) fn ui_source_form(&mut self, ctx: &egui::Context) {
        if !self.show_source_form {
            return;
        }
        let mut open = true;
        egui::Window::new(if self.editing_id.is_some() {
            "编辑订阅源"
        } else {
            "新增订阅源"
        })
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("名称");
                let name_field = ui.text_edit_singleline(&mut self.form_name);
                crate::gui::widgets::note_text_field(ui, &name_field);
            });
            ui.label("URL 模板");
            let url_field = ui.add(
                egui::TextEdit::multiline(&mut self.form_url)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            crate::gui::widgets::note_text_field(ui, &url_field);
            // 认出来就当场说明这一类源的规矩，别等拉取失败才发现要挂代理。
            if let Some(channel) = crate::collector::telegram::channel_name(&self.form_url) {
                ui.label(
                    RichText::new(format!(
                        "Telegram 频道 · @{channel} —— 读公开网页预览；节点只增不删，\
                         旧节点交给测速淘汰。t.me 直连不通，拉取时得挂着代理。"
                    ))
                    .small()
                    .color(theme::success()),
                );
            }
            ui.checkbox(&mut self.form_enabled, "启用");
            ui.horizontal(|ui| {
                ui.label("备注");
                let remark_field = ui.text_edit_singleline(&mut self.form_remark);
                crate::gui::widgets::note_text_field(ui, &remark_field);
            });
            ui.horizontal(|ui| {
                if ui.add(primary_btn("保存")).clicked() {
                    self.save_source_form();
                }
                if ui.button("取消").clicked() {
                    self.show_source_form = false;
                }
            });
        });
        if !open {
            self.show_source_form = false;
        }
    }

    pub(crate) fn ui_homogeneity_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_homogeneity_dialog {
            return;
        }
        let Some(reports) = self.homogeneity_reports.clone() else {
            return;
        };

        let mut open = self.show_homogeneity_dialog;
        let mut action: Option<(Vec<String>, bool)> = None;
        let mut close_dialog = false;
        let mut select_redundant = false;
        let mut select_all = false;
        let mut select_none = false;

        egui::Window::new(RichText::new("订阅源同质化分析与反向剔除").strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .default_height(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.add_space(4.0);
                let redundant_count = reports.iter().filter(|r| r.is_redundant).count();

                Frame::NONE
                    .fill(if redundant_count > 0 { theme::warning_bg() } else { theme::accent_bg() })
                    .stroke(Stroke::new(1.0_f32, if redundant_count > 0 { theme::warning() } else { theme::accent() }))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let icon = if redundant_count > 0 { Icon::Alert } else { Icon::Shield };
                            let color = if redundant_count > 0 { theme::warning() } else { theme::accent() };
                            icon_label(ui, icon, 18.0, color);
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "共检测 {} 个订阅源，发现 {} 个高同质化/冗余源",
                                        reports.len(),
                                        redundant_count
                                    ))
                                    .strong()
                                    .color(theme::text_primary()),
                                );
                                ui.label(
                                    RichText::new(
                                        "说明：同质化源指“独有可用节点数为 0 且重合率高于 85%”或全失效源。此类源的所有可用节点均已存在于其它优质源中，剔除后不会丢失任何可用节点，并大幅节省网络与测速资源。勾选后要剔除的源，再点底部按钮执行。"
                                    )
                                    .small()
                                    .color(theme::muted()),
                                );
                            });
                        });
                    });

                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        egui::Grid::new("homogeneity_grid")
                            .striped(true)
                            .num_columns(7)
                            .min_col_width(70.0)
                            .show(ui, |ui| {
                                header_cell(ui, "剔除");
                                header_cell(ui, "订阅源名称");
                                header_cell(ui, "节点总数");
                                header_cell(ui, "可用节点");
                                header_cell(ui, "重合率");
                                header_cell(ui, "独有可用");
                                header_cell(ui, "判定建议");
                                ui.end_row();

                                for r in &reports {
                                    // 核心保留的源也允许手动勾选剔除，判定只是建议。
                                    let mut selected =
                                        self.homogeneity_selected.contains(&r.source_id);
                                    if ui
                                        .checkbox(&mut selected, "")
                                        .on_hover_text("勾选后，底部“批量禁用/批量删除”只会作用于勾选的源")
                                        .changed()
                                    {
                                        if selected {
                                            self.homogeneity_selected.insert(r.source_id.clone());
                                        } else {
                                            self.homogeneity_selected.remove(&r.source_id);
                                        }
                                    }
                                    ui.label(RichText::new(&r.source_name).strong());
                                    ui.label(r.total_nodes.to_string());
                                    ui.label(
                                        RichText::new(r.alive_nodes.to_string())
                                            .color(if r.alive_nodes > 0 { theme::success() } else { theme::muted() }),
                                    );
                                    ui.label(
                                        RichText::new(format!("{:.1}%", r.overlap_rate_pct))
                                            .color(if r.overlap_rate_pct >= 90.0 { theme::danger() } else { theme::muted() }),
                                    );
                                    ui.label(
                                        RichText::new(r.unique_alive_nodes.to_string())
                                            .strong()
                                            .color(if r.unique_alive_nodes > 0 { theme::success() } else { theme::danger() }),
                                    );

                                    if r.is_redundant {
                                        status_badge(ui, "⚠️ 建议剔除", theme::danger());
                                    } else {
                                        status_badge(ui, "✓ 核心保留", theme::success());
                                    }
                                    ui.end_row();
                                }
                            });
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                let selected_ids: Vec<String> = reports
                    .iter()
                    .filter(|r| self.homogeneity_selected.contains(&r.source_id))
                    .map(|r| r.source_id.clone())
                    .collect();
                ui.horizontal(|ui| {
                    if ui
                        .button("仅勾选冗余源")
                        .on_hover_text("按判定结果重新勾选：只选中“建议剔除”的源")
                        .clicked()
                    {
                        select_redundant = true;
                    }
                    if ui.button("全选").clicked() {
                        select_all = true;
                    }
                    if ui.button("清空勾选").clicked() {
                        select_none = true;
                    }
                    ui.label(
                        RichText::new(format!("已勾选 {} 个", selected_ids.len()))
                            .strong()
                            .color(if selected_ids.is_empty() {
                                theme::muted()
                            } else {
                                theme::accent()
                            }),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !self.homogeneity_busy && !selected_ids.is_empty(),
                            icons::primary_icon_btn(
                                Icon::Lightning,
                                &format!("批量禁用勾选的源 ({})", selected_ids.len()),
                            ),
                        )
                        .on_hover_text("推荐：把勾选的源设为禁用状态，停止自动拉取以节省网络资源，保留源地址")
                        .clicked()
                    {
                        action = Some((selected_ids.clone(), false));
                    }

                    if ui
                        .add_enabled(
                            !self.homogeneity_busy && !selected_ids.is_empty(),
                            icons::danger_icon_btn(
                                Icon::Trash,
                                &format!("批量彻底删除 ({})", selected_ids.len()),
                            ),
                        )
                        .on_hover_text("从数据库彻底删除勾选的源及其节点数据，不可撤销")
                        .clicked()
                    {
                        action = Some((selected_ids.clone(), true));
                    }

                    if ui.button("关闭").clicked() {
                        close_dialog = true;
                    }
                });
            });

        if !open || close_dialog {
            self.show_homogeneity_dialog = false;
        }
        if select_redundant {
            self.homogeneity_selected = reports
                .iter()
                .filter(|r| r.is_redundant)
                .map(|r| r.source_id.clone())
                .collect();
        }
        if select_all {
            self.homogeneity_selected = reports
                .iter()
                .map(|r| r.source_id.clone())
                .collect::<std::collections::HashSet<_>>();
        }
        if select_none {
            self.homogeneity_selected.clear();
        }
        if let Some((ids, delete_records)) = action {
            self.prune_homogenized_sources(ids, delete_records);
        }
    }

    /// Ctrl+V 识别结果的弹框（任何页面按 Ctrl+V 都可能弹出来）。
    /// 内容和按钮都在 `paste::dialog_body` 里 —— 那边有离屏渲染和文案断言，这里只管状态。
    pub(crate) fn ui_paste_dialog(&mut self, ctx: &egui::Context) {
        let Some(outcome) = self.paste_outcome.clone() else {
            return;
        };
        use crate::gui::paste::DialogAction;
        match crate::gui::paste::dialog_body(ctx, &outcome) {
            Some(DialogAction::Close) => self.paste_outcome = None,
            Some(DialogAction::ImportNodes) => {
                self.paste_outcome = None;
                self.import_from_clipboard();
            }
            None => {}
        }
    }
}

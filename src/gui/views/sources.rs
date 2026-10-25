use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{SourceStatusFilter, SourcesPanel};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, header_cell, primary_btn, render_fetch_ticks, render_mini_progress_bar,
    render_radial_metric, section_title, source_status_failed, status_badge,
};
use crate::gui::VpnApp;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

impl VpnApp {
    pub(crate) fn ui_sources(&mut self, ui: &mut Ui) {
        self.ui_clear_sources_confirm(ui.ctx());
        self.ui_gist_config_modal(ui.ctx());

        section_title(ui, self.tr("订阅源"), "多源汇聚 · 自动拉取 · 节点解析");

        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.sources_panel == SourcesPanel::List, self.tr("订阅列表"))
                .clicked()
            {
                self.sources_panel = SourcesPanel::List;
            }
            if ui
                .selectable_label(self.sources_panel == SourcesPanel::Results, self.tr("订阅拉取结果"))
                .clicked()
            {
                self.sources_panel = SourcesPanel::Results;
            }

            // 「刷新」原来钉在这一行的最右，现在跟着过滤条件走（见第 3 行）。
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.settings.auto_fetch_enabled {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Sync, 12.0, theme::accent_strong());
                        ui.label(
                            RichText::new(format!(
                                "自动拉取 每 {} 分钟",
                                self.settings.auto_fetch_interval_min
                            ))
                            .color(theme::accent_strong())
                            .small(),
                        );
                    });
                }
            });
        });
        ui.add_space(6.0);
        if self.sources_panel == SourcesPanel::Results {
            self.ui_fetch_results(ui);
            return;
        }

        // 订阅源健康与产出度量看板
        if let Some(s) = self.stats.as_ref() {
            Frame::NONE
                .fill(theme::panel())
                .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let enabled_ratio = if s.source_count > 0 {
                            s.enabled_source_count as f32 / s.source_count as f32
                        } else {
                            0.0
                        };
                        render_radial_metric(ui, "源启用率", enabled_ratio, theme::accent());
                        ui.add_space(10.0);

                        let yield_ratio = if s.node_count > 0 {
                            s.alive_count as f32 / s.node_count as f32
                        } else {
                            0.0
                        };
                        render_radial_metric(ui, "节点可用率", yield_ratio, theme::accent_strong());
                        ui.add_space(14.0);

                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Globe, 13.0, theme::accent());
                                ui.label(
                                    RichText::new(format!(
                                        "订阅源池: {} 个 (已启用 {})",
                                        s.source_count, s.enabled_source_count
                                    ))
                                    .strong()
                                    .small()
                                    .color(theme::text_primary()),
                                );
                            });
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Radar, 13.0, theme::accent_strong());
                                ui.label(
                                    RichText::new(format!(
                                        "累计解析节点: {} 个 · 实测可用: {} 个",
                                        s.node_count, s.alive_count
                                    ))
                                    .small()
                                    .color(theme::text_secondary()),
                                );
                            });
                        });
                    });
                });
            ui.add_space(6.0);
        }

        // 第 1 行：新建 拉取全部 代理拉取全部 导入订阅 导出订阅 清空全部
        ui.horizontal(|ui| {
            if ui.add(primary_btn(self.tr("新建"))).on_hover_text("新增订阅源配置").clicked() {
                self.open_create_form();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(self.tr("拉取全部")))
                .on_hover_text("后台拉取全部已启用的订阅源")
                .clicked()
            {
                self.fetch_all();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(self.tr("代理拉取全部")))
                .on_hover_text("通过当前运行中的本地代理拉取全部启用源（防止 GitHub / Raw 阻断）")
                .clicked()
            {
                self.fetch_all_via_proxy();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(self.tr("导入订阅")))
                .on_hover_text("从剪贴板导入 JSON 格式订阅源，自动跳过重复项")
                .clicked()
            {
                self.import_sources_from_clipboard();
            }
            if ui
                .button(self.tr("导出订阅"))
                .on_hover_text("导出全部订阅源至剪贴板")
                .clicked()
            {
                self.export_sources_to_clipboard();
            }
            if ui
                .add_enabled(
                    !self.busy && !self.sources.is_empty(),
                    icons::danger_icon_btn(Icon::Trash, self.tr("清空全部")),
                )
                .on_hover_text("清空全部订阅源及关联节点数据库分片")
                .clicked()
            {
                self.show_clear_sources_confirm = true;
            }
        });

        ui.add_space(6.0);

        // 第 2 行：接入openRung节点 原生注册warp节点 gist节点 gist配置
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy, egui::Button::new(format!("🌐 {}", self.tr("接入 OpenRung 节点"))))
                .on_hover_text("一键接入 https://github.com/openrung/openrung 官方中继服务，实时获取健康 VLESS-REALITY 节点")
                .clicked()
            {
                self.add_openrung_source();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(format!("⚡ {}", self.tr("原生注册 WARP 节点"))))
                .on_hover_text("原生向 Cloudflare 注册并获取 57 个 WARP MASQUE 节点（免任何 GitHub 账号与仓库）")
                .clicked()
            {
                self.fetch_warp_masque_native();
            }
            if ui
                .add_enabled(!self.busy, egui::Button::new(format!("🔍 {}", self.tr("Gist 节点"))))
                .on_hover_text("扫描 gist.github.com 搜索结果（按配置的协议词），能解析出节点的 gist 自动注册为订阅源")
                .clicked()
            {
                self.discover_gists();
            }
            if ui
                .button(format!("⚙️ {}", self.tr("Gist 配置")))
                .on_hover_text("弹框选择 Gist 抓取协议类型、扫描页数与历史记录重置")
                .clicked()
            {
                self.show_gist_config_modal = true;
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
        });

        ui.add_space(6.0);

        // 第 3 行：名称 + 状态过滤。「刷新」从页签那一行挪到这里，跟在状态筛选后面。
        ui.horizontal(|ui| {
            icon_label(ui, Icon::Search, 13.0, theme::muted());
            let name_hint = self.tr("按名称过滤");
            let name_field = ui.add(
                egui::TextEdit::singleline(&mut self.source_name_filter)
                    .desired_width(180.0)
                    .hint_text(name_hint),
            );
            crate::gui::widgets::note_text_field(ui, &name_field);
            for filter in SourceStatusFilter::ALL {
                let label = match filter {
                    SourceStatusFilter::All => self.tr("全部"),
                    SourceStatusFilter::Enabled => self.tr("启用"),
                    SourceStatusFilter::Disabled => self.tr("停用"),
                    SourceStatusFilter::Failed => self.tr("拉取失败"),
                    SourceStatusFilter::NoNodes => self.tr("无节点"),
                };
                if ui
                    .selectable_label(self.source_status_filter == filter, label)
                    .clicked()
                {
                    self.source_status_filter = filter;
                }
            }
            if ui.button(format!("🔄 {}", self.tr("刷新"))).clicked() {
                self.reload_all();
            }
        });
        ui.add_space(8.0);

        // 过滤只作用于这张表：底下那行会说"没有匹配"，不会去动库里的源。
        let visible_sources: Vec<crate::models::Source> = self
            .sources
            .iter()
            .filter(|source| {
                source_passes_filter(
                    source,
                    &self.source_name_filter,
                    self.source_status_filter,
                )
            })
            .cloned()
            .collect();

        card_frame().show(ui, |ui| {
            if visible_sources.is_empty() && !self.sources.is_empty() {
                ui.label(
                    RichText::new(self.tr("当前过滤条件下没有匹配的订阅源"))
                        .color(theme::muted()),
                );
                return;
            }
            egui::ScrollArea::both()
                .id_salt("sources_grid_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("sources_grid")
                        .striped(true)
                        .num_columns(6)
                        .min_col_width(70.0)
                        .show(ui, |ui| {
                            header_cell(ui, self.tr("名称"));
                            header_cell(ui, "URL 模板");
                            header_cell(ui, self.tr("状态"));
                            header_cell(ui, "最近拉取");
                            header_cell(ui, "健康");
                            header_cell(ui, self.tr("操作"));
                            ui.end_row();

                            for s in &visible_sources {
                                let source_active = self.active_source_fetches.contains(&s.id)
                                    || self.tasks.is_source_active(&s.id);
                                let fetch_operation_allowed =
                                    !self.busy || self.tasks.has_active_fetch();
                                let retry =
                                    s.last_status.as_deref().is_some_and(source_status_failed);
                                let name_resp = ui.add(
                                    egui::Label::new(
                                        RichText::new(&s.name).strong().color(theme::accent()),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text("点击查看此订阅源的节点列表")
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if name_resp.clicked() {
                                    self.open_source_nodes_modal(&s.id, &s.name);
                                }
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&s.url_template).small().color(theme::accent()),
                                    )
                                    .wrap(),
                                );
                                let badge = if s.enabled { self.tr("启用") } else { self.tr("停用") };
                                let color = if s.enabled { theme::accent_strong() } else { theme::muted() };
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(badge).color(color));
                                    if s.push_disabled {
                                        ui.label(RichText::new(self.tr("不推送")).small().color(theme::warning()));
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
                                            .color(theme::warning()),
                                        );
                                    }
                                    if let Some(reports) = &self.homogeneity_reports {
                                        if let Some(rep) = reports.iter().find(|r| r.source_id == s.id) {
                                            if rep.total_nodes == 0 {
                                                // 空源不能写"同质化 0%"：重合率是 0 恰恰因为它
                                                // 什么都没贡献，剔除理由是"空"不是"重"。
                                                status_badge(ui, "⚠️ 空源 · 建议剔除", theme::warning());
                                            } else if rep.is_redundant {
                                                status_badge(ui, &format!("⚠️ 同质化 {:.0}%", rep.overlap_rate_pct), theme::warning());
                                            } else if rep.unique_alive_nodes > 0 {
                                                status_badge(ui, &format!("独有可用 {}", rep.unique_alive_nodes), theme::accent_strong());
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
                                        ui.horizontal(|ui| {
                                            render_mini_progress_bar(ui, alive, total);
                                            ui.label(
                                                RichText::new(format!("{alive}/{total}"))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );
                                        });
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
                                            egui::Button::new(self.tr("代理拉取")),
                                        )
                                        .on_hover_text("通过当前运行中的本地代理拉取此订阅源")
                                        .clicked()
                                    {
                                        self.fetch_one_via_proxy(s.id.clone());
                                    }

                                    if ui.button(self.tr("编辑")).clicked() {
                                        self.open_edit_form(&s);
                                    }
                                    if ui
                                        .button(if s.enabled { self.tr("禁用") } else { self.tr("启用") })
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
                                            self.tr("恢复推送")
                                        } else {
                                            self.tr("不推送")
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

                                    if ui.button("删除").clicked() {
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
                ui.label(
                    RichText::new("暂无拉取结果（添加订阅并点击「拉取全部」后将在此显示拉取与解析明细）")
                        .color(theme::muted()),
                );
                return;
            }
            egui::ScrollArea::both()
                .id_salt("sources_fetch_results_scroll")
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
                                        .color(if result.success {
                                            theme::accent_strong()
                                        } else {
                                            theme::muted()
                                        }),
                                );
                                ui.label(result.node_count.to_string());
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&result.resolved_url)
                                            .small()
                                            .color(theme::accent()),
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
                        .color(theme::accent_strong()),
                    );
            } else if crate::collector::local_file_path(self.form_url.trim()).is_some() {
                    ui.label(
                        RichText::new(
                            "本地文件源 —— 直接读取该文件，格式自动识别 \
                             （Clash YAML / 节点链接 / base64）。改完文件等下一轮拉取即可，\
                             内容没变不会重复解析，不联网也能用。",
                        )
                        .small()
                        .color(theme::accent_strong()),
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
                    .id_salt("sources_homogeneity_scroll")
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
                                            .color(if r.alive_nodes > 0 { theme::accent_strong() } else { theme::muted() }),
                                    );
                                    ui.label(
                                        RichText::new(format!("{:.1}%", r.overlap_rate_pct))
                                            .color(if r.overlap_rate_pct >= 90.0 { theme::warning() } else { theme::muted() }),
                                    );
                                    ui.label(
                                        RichText::new(r.unique_alive_nodes.to_string())
                                            .strong()
                                            .color(if r.unique_alive_nodes > 0 { theme::accent_strong() } else { theme::muted() }),
                                    );

                                    let verdict_color = match r.verdict() {
                                        crate::models::HomogeneityVerdict::Redundant => {
                                            theme::warning()
                                        }
                                        crate::models::HomogeneityVerdict::Core => theme::accent_strong(),
                                    };
                                    status_badge(ui, r.verdict_label(), verdict_color);
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

    pub(crate) fn ui_clear_sources_confirm(&mut self, ctx: &egui::Context) {
        if !self.show_clear_sources_confirm {
            return;
        }

        let modal = egui::Modal::new(egui::Id::new("clear_sources_confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("清空全部订阅源");
            ui.add_space(6.0);
            ui.label(
                RichText::new("确定要清空全部订阅源及关联节点吗？\n此操作将删除所有订阅源和对应节点数据库分片，不可恢复。")
                    .color(theme::warning()),
            );
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    Some(false)
                } else if ui
                    .add(icons::danger_icon_btn(Icon::Trash, "确认清空全部"))
                    .clicked()
                {
                    Some(true)
                } else {
                    None
                }
            })
            .inner
        });

        match modal.inner {
            Some(true) => {
                self.show_clear_sources_confirm = false;
                self.clear_all_sources();
            }
            Some(false) => self.show_clear_sources_confirm = false,
            None if modal.should_close() => self.show_clear_sources_confirm = false,
            None => {}
        }
    }

    pub(crate) fn ui_gist_config_modal(&mut self, ctx: &egui::Context) {
        if !self.show_gist_config_modal {
            return;
        }

        let modal = egui::Modal::new(egui::Id::new("gist_config_modal")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Gist 节点发现与协议配置");
            ui.add_space(4.0);
            ui.label(
                RichText::new("选择扫描 GitHub Gist 节点时匹配的协议，以及爬取深度页数。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("扫描协议类型：").strong());
            ui.add_space(4.0);

            let queries = crate::collector::GIST_DISCOVERY_QUERIES;
            let labels = [
                "VLESS 协议 (vless://)",
                "VMess 协议 (vmess://)",
                "Shadowsocks 协议 (ss://)",
                "Trojan 协议 (trojan://)",
                "Hysteria2 协议 (hy2://)",
                "Socks5 协议 (socks5://)",
                "Clash 代理配置 (clash/proxies)",
            ];

            for (idx, (query, label)) in queries.iter().zip(labels.iter()).enumerate() {
                if idx < self.gist_query_flags.len() {
                    ui.checkbox(&mut self.gist_query_flags[idx], *label)
                        .on_hover_text(format!("搜索词：{}", query));
                }
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.small_button("全选").clicked() {
                    for flag in &mut self.gist_query_flags {
                        *flag = true;
                    }
                }
                if ui.small_button("清空勾选").clicked() {
                    for flag in &mut self.gist_query_flags {
                        *flag = false;
                    }
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("最多扫描页数：").strong());
                ui.add(
                    egui::DragValue::new(&mut self.gist_pages)
                        .range(1..=300)
                        .suffix(" 页"),
                )
                .on_hover_text("搜索结果每页 10 条，按最近更新排序，每个勾选的查询词各扫这么多页");
                ui.label(RichText::new("（建议 5~20 页）").small().color(theme::muted()));
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("历史记录重置：").strong());
                if ui
                    .add_enabled(!self.busy, egui::Button::new("重置 Gist 历史"))
                    .on_hover_text("清空已扫描过的 Gist 记录黑名单，允许重新发现和拉取曾经删除过的 Gist 节点")
                    .clicked()
                {
                    self.reset_seen_gists();
                }
            });

            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary_btn("完成")).clicked() {
                    Some(true)
                } else {
                    None
                }
            })
            .inner
        });

        match modal.inner {
            Some(true) => {
                self.save_gist_config();
                self.show_gist_config_modal = false;
            }
            _ if modal.should_close() => {
                self.save_gist_config();
                self.show_gist_config_modal = false;
            }
            _ => {}
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
            Some(DialogAction::ImportClashConfig) => {
                self.paste_outcome = None;
                self.import_clash_from_clipboard();
            }
            Some(DialogAction::ImportRules) => {
                self.paste_outcome = None;
                self.import_rules_from_clipboard();
            }
            None => {}
        }
    }

    pub(crate) fn ui_source_nodes_modal(&mut self, ctx: &egui::Context) {
        if !self.source_modal_open {
            return;
        }
        let Some(source_id) = self.source_modal_source_id.clone() else {
            return;
        };
        let source_name = self.source_modal_source_name.clone();

        let mut open = true;
        let mut reload = false;
        let mut test_source = false;

        egui::Window::new(RichText::new(format!("订阅源节点 · {}", source_name)).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(840.0)
            .default_height(580.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 顶部工具栏：统计、测速、刷新、筛选与搜索
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("共 {} 个节点", self.source_modal_total))
                            .strong()
                            .color(theme::text_primary()),
                    );

                    ui.separator();

                    if ui
                        .add_enabled(!self.busy, egui::Button::new("⚡ 测速此源全部节点"))
                        .clicked()
                    {
                        test_source = true;
                    }
                    if ui.button("🔄 刷新").clicked() {
                        reload = true;
                    }

                    ui.separator();

                    let is_alive = self.source_modal_only_alive;
                    let (color_alive, stroke_alive) = if is_alive {
                        (theme::accent_bg(), Stroke::new(1.0_f32, theme::accent()))
                    } else {
                        (theme::sidebar(), Stroke::new(1.0_f32, theme::border_subtle()))
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new("● 仅看可用").small().strong())
                                .fill(color_alive)
                                .stroke(stroke_alive)
                                .corner_radius(CornerRadius::same(10)),
                        )
                        .on_hover_text("仅展示通过真实握手验证的节点")
                        .clicked()
                    {
                        self.source_modal_only_alive = !self.source_modal_only_alive;
                        self.source_modal_page = 1;
                        reload = true;
                    }

                    // 搜索框
                    ui.label(RichText::new("搜索:").small().color(theme::muted()));
                    let search_resp = ui.add(
                        egui::TextEdit::singleline(&mut self.source_modal_keyword)
                            .hint_text("按名称/服务器搜索")
                            .desired_width(140.0),
                    );
                    if search_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.source_modal_page = 1;
                        reload = true;
                    }
                    if ui.button("🔍").clicked() {
                        self.source_modal_page = 1;
                        reload = true;
                    }
                });

                ui.separator();

                // 分页控制栏
                let pages = NodesViewModel::total_pages(
                    self.source_modal_total,
                    self.source_modal_page_size,
                );
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "第 {} / {} 页 · 共 {} 条",
                        self.source_modal_page, pages, self.source_modal_total
                    ));
                    if ui
                        .add_enabled(self.source_modal_page > 1, egui::Button::new("上一页"))
                        .clicked()
                    {
                        self.source_modal_page -= 1;
                        reload = true;
                    }
                    if ui
                        .add_enabled(self.source_modal_page < pages, egui::Button::new("下一页"))
                        .clicked()
                    {
                        self.source_modal_page += 1;
                        reload = true;
                    }
                });

                ui.add_space(4.0);

                // 卡片网格展示
                if self.source_modal_nodes.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(30.0);
                            if self.source_modal_loading {
                                ui.label(
                                    RichText::new("正在加载节点...")
                                        .color(theme::text_secondary()),
                                );
                            } else {
                                ui.label(
                                    RichText::new("该订阅源下暂无节点，可点击「拉取」更新。")
                                        .color(theme::text_secondary()),
                                );
                            }
                            ui.add_space(30.0);
                        });
                    });
                } else {
                    let action = crate::gui::views::nodes::render_node_cards_grid(
                        ui,
                        &self.source_modal_nodes,
                        self.selected_proxy_node_id.as_deref(),
                        self.focused_node_id.as_deref(),
                        self.switching_node_id.as_deref(),
                        self.busy,
                    );
                    if let Some(action) = action {
                        let is_delete = matches!(action, NodeCardAction::Delete(_));
                        self.handle_node_card_action(action);
                        if is_delete {
                            reload = true;
                        }
                    }
                }
            });

        if !open {
            self.source_modal_open = false;
        }
        if test_source {
            self.speedtest_source(&source_id, &source_name);
        }
        if reload {
            self.reload_source_modal_nodes();
        }
    }
}

/// 订阅列表这一行该不该显示。**状态判据全部复用表格里已经在用的那几个**：
/// 「拉取失败」= 状态列那行文字（`source_status_failed`，也就是行尾"重试"按钮用的同一判据），
/// 「无节点」= 健康列的「无节点」（`last_node_count == 0`）。没有第三种状态。
fn source_passes_filter(
    source: &crate::models::Source,
    name_filter: &str,
    status: SourceStatusFilter,
) -> bool {
    let needle = name_filter.trim().to_lowercase();
    if !needle.is_empty() && !source.name.to_lowercase().contains(&needle) {
        return false;
    }
    match status {
        SourceStatusFilter::All => true,
        SourceStatusFilter::Enabled => source.enabled,
        SourceStatusFilter::Disabled => !source.enabled,
        SourceStatusFilter::Failed => source
            .last_status
            .as_deref()
            .is_some_and(source_status_failed),
        SourceStatusFilter::NoNodes => source.last_node_count == 0,
    }
}

#[cfg(test)]
mod source_filter_tests {
    use super::source_passes_filter;
    use crate::gui::types::SourceStatusFilter;
    use crate::models::Source;

    fn source(name: &str, enabled: bool, status: Option<&str>, nodes: i64) -> Source {
        let mut source = Source::scratch("s1", name, "https://example.invalid/sub");
        source.enabled = enabled;
        source.last_status = status.map(str::to_string);
        source.last_node_count = nodes;
        source
    }

    #[test]
    fn test_name_filter_is_a_case_insensitive_substring() {
        let s = source("Solara 订阅", true, Some("ok"), 12);
        assert!(source_passes_filter(&s, "solara", SourceStatusFilter::All));
        assert!(source_passes_filter(&s, "  订阅  ", SourceStatusFilter::All));
        assert!(!source_passes_filter(&s, "v2ray", SourceStatusFilter::All));
        // 名称留空 = 名称这一维不过滤（状态那一维照旧要过，两维是"与"）
        assert!(source_passes_filter(&s, "   ", SourceStatusFilter::All));
        assert!(!source_passes_filter(&s, "", SourceStatusFilter::Failed));
    }

    #[test]
    fn test_status_filter_reuses_the_columns_own_predicates() {
        let ok = source("活", true, Some("ok"), 12);
        let failed = source("死", true, Some("error: timeout"), 0);
        let off = source("停用且没节点", false, None, 0);

        assert!(source_passes_filter(&ok, "", SourceStatusFilter::Enabled));
        assert!(!source_passes_filter(&ok, "", SourceStatusFilter::Disabled));
        assert!(!source_passes_filter(&ok, "", SourceStatusFilter::Failed));
        // "ok" / "not modified" 都不算失败 —— 判据就是行尾那颗"重试"按钮的判据
        assert!(!source_passes_filter(
            &source("x", true, Some("Not Modified"), 3),
            "",
            SourceStatusFilter::Failed
        ));
        assert!(source_passes_filter(
            &failed,
            "",
            SourceStatusFilter::Failed
        ));
        assert!(source_passes_filter(&failed, "", SourceStatusFilter::NoNodes));
        assert!(source_passes_filter(&off, "", SourceStatusFilter::Disabled));
        assert!(source_passes_filter(&off, "", SourceStatusFilter::NoNodes));
        assert!(!source_passes_filter(&off, "", SourceStatusFilter::Failed));

        // 反向判据：全部 = 一条不漏
        for s in [&ok, &failed, &off] {
            assert!(source_passes_filter(s, "", SourceStatusFilter::All));
        }
    }

    #[test]
    fn test_name_and_status_are_anded() {
        let a = source("AI 订阅", true, Some("ok"), 4);
        let b = source("AI 备用", false, Some("boom"), 0);
        assert!(source_passes_filter(&b, "ai 备用", SourceStatusFilter::Failed));
        assert!(!source_passes_filter(&b, "ai", SourceStatusFilter::Enabled));
        assert!(!source_passes_filter(&a, "ai", SourceStatusFilter::NoNodes));
    }

    /// 这一行是"一行摆平"的（`ui.horizontal`，不换行），所以量一下它在**最小窗口宽度**下
    /// 会不会顶出内容区。图标用一枚等宽标签近似。
    #[test]
    fn test_the_filter_row_fits_the_min_window_width() {
        use eframe::egui::{CentralPanel, Context, Margin, RawInput, Rect, SidePanel, Vec2};

        let ctx = Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            eframe::egui::Pos2::ZERO,
            Vec2::new(960.0, 600.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut row_right = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            CentralPanel::default()
                .frame(eframe::egui::Frame::NONE.inner_margin(Margin::symmetric(18, 14)))
                .show(ctx, |ui| {
                    panel = ui.max_rect();
                    let _ = ui.horizontal(|ui| {
                        let _ = ui.label("🔍");
                        let mut name = String::new();
                        let _ = ui.add(
                            eframe::egui::TextEdit::singleline(&mut name)
                                .desired_width(180.0)
                                .hint_text("按名称过滤"),
                        );
                        for label in ["全部", "启用", "停用", "拉取失败", "无节点"] {
                            let _ = ui.selectable_label(false, label);
                        }
                        let _ = ui.button("🔄 刷新");
                        row_right = ui.min_rect().right();
                    });
                });
        });
        assert!(panel.width() > 600.0, "探针窗口没铺开：{panel:?}");
        assert!(
            row_right <= panel.right(),
            "过滤行右边界 {row_right:.0} 顶出了内容区（右边界 {}）",
            panel.right()
        );
    }
}

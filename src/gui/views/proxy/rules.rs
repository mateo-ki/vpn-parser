use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::RulesTab;
use crate::gui::widgets::{
    card_frame, header_cell, panel_tab,
    primary_btn,
    routing_mode_description, routing_mode_detail, section_title,
};
use crate::gui::VpnApp;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules,
    social_telegram_rules, streaming_media_rules,
    ProxyState, RoutingMode, TrafficRule,
};
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

/// 分流规则那行表头右侧动作组（一键更新 Geo 规则 + 导入规则 + 导出 JSON）预留的宽度。
pub(crate) const RULES_ACTIONS_WIDTH: f32 = 320.0;

/// 两个分区标签（规则配置 / 命中记录）至少要占的宽度。
pub(crate) const RULES_TABS_MIN_WIDTH: f32 = 170.0;

/// `split_row` 两个闭包都要碰 `self`，所以动作先记成值、出框后再执行。
pub(crate) enum RulesHeaderAction {
    None,
    Export,
    Import,
    UpdateGeo,
}

impl VpnApp {
    pub(crate) fn ui_proxy_rules(&mut self, ui: &mut Ui) {
        ui.set_width(ui.available_width());

        // 两个闭包都要动 self，所以 tab 状态先取副本、动作先记成枚举，出框后再落地。
        let mut tab = self.rules_tab;
        // 动作组按"这一帧进来时"的分区决定画不画；`tab` 被左闭包可变借用，这里另存一份。
        let tab_now = self.rules_tab;
        let busy = self.proxy_busy;
        let geo_updating = self.geo_updating;
        let proxy_running = self.proxy_status.state == ProxyState::Running;
        let mut action = RulesHeaderAction::None;
        // 系统显示缩放 125%/150% 叠上字体 140% 时，960 物理最小宽只剩 550~460 逻辑点，
        // 中央面板不到 420 —— 一行放不下"两个分区标签 + 动作组"，右对齐那组会
        // 直接压到标签上。放不下就让它换一行。
        let one_row_fits =
            ui.max_rect().width() >= RULES_ACTIONS_WIDTH + RULES_TABS_MIN_WIDTH + 8.0;
        if one_row_fits {
            crate::gui::widgets::split_row(
                ui,
                "rules:tabs",
                RULES_ACTIONS_WIDTH,
                |ui| {
                    panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                    panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
                },
                |ui| Self::rules_actions(ui, tab_now, busy, geo_updating, proxy_running, &mut action),
            );
        } else {
            let _ = ui.horizontal(|ui| {
                panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
            });
            // 连一行都放不下时（200% 系统缩放 + 140% 字体）让它自己折，别越界。
            let _ = ui.horizontal_wrapped(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Self::rules_actions(ui, tab_now, busy, geo_updating, proxy_running, &mut action);
                });
            });
        }
        self.rules_tab = tab;
        match action {
            RulesHeaderAction::None => {}
            RulesHeaderAction::Export => self.export_rules_as_json(),
            RulesHeaderAction::Import => self.show_rule_import = true,
            RulesHeaderAction::UpdateGeo => self.update_geo_data(),
        }
        ui.add_space(8.0);
        match self.rules_tab {
            RulesTab::Config => self.ui_proxy_rules_config(ui),
            RulesTab::Hits => self.ui_proxy_rules_hits(ui),
        }
        self.ui_rule_import(ui.ctx());
        self.ui_rule_import_help(ui.ctx());
    }

    fn button_small(text: &str) -> egui::Button<'_> {
        egui::Button::new(RichText::new(text).strong().size(12.5))
    }

    /// 分流规则表头右侧那组动作（一键更新 Geo 规则 / 导入规则 / 导出 JSON）。
    /// 两个闭包都要碰 `self`，所以这里只把点击记进 `action`，由调用方出框后再执行。
    fn rules_actions(
        ui: &mut Ui,
        tab: RulesTab,
        busy: bool,
        geo_updating: bool,
        proxy_running: bool,
        action: &mut RulesHeaderAction,
    ) {
        if tab != RulesTab::Config {
            return;
        }
        if ui
            .button("导出 JSON")
            .on_hover_text(
                "把当前规则表导成 mihomo 规则字符串数组：写进 data/exports 并复制到剪贴板",
            )
            .clicked()
        {
            *action = RulesHeaderAction::Export;
        }
        if ui
            .add_enabled(!busy, Self::button_small("导入规则"))
            .on_hover_text("贴 JSON 或一行一条的文本，解析后追加进规则表")
            .clicked()
        {
            *action = RulesHeaderAction::Import;
        }
        if geo_updating {
            ui.spinner();
            ui.label(RichText::new("更新 Geo 库中...").small().color(theme::accent()));
        } else {
            let btn_resp = ui
                .add_enabled(!busy, Self::button_small("一键更新 Geo 规则"))
                .on_hover_text(if proxy_running {
                    "向当前运行的内核发送指令在线拉取并热重载最新的 Geo 数据库"
                } else {
                    "直连极速 CDN 高速拉取并保存最新的 GeoIP 与 GeoSite 数据库（无需开启代理）"
                });
            if btn_resp.clicked() {
                *action = RulesHeaderAction::UpdateGeo;
            }
        }
    }

    /// 命中记录：规则列表说的是"应该走哪"，这张表说的是"实际走了哪"。
    /// 连接是快照不是事件流，所以统计从本进程启动开始，重启前的那部分没有。
    fn ui_proxy_rules_hits(&mut self, ui: &mut Ui) {
        let total: i64 = self.rule_hits_today.iter().map(|hit| hit.hits).sum();
        let matched: i64 = self
            .rule_hits_today
            .iter()
            .filter(|hit| hit.rule.starts_with("MATCH"))
            .map(|hit| hit.hits)
            .sum();
        let pct = if total > 0 { matched * 100 / total } else { 0 };
        let empty = self.rule_hits_today.is_empty();
        let mut refresh = false;
        card_frame().show(ui, |ui| {
            // 说明并进表头那一行（原来它单独占一行 + 8pt 间隔），滚动区再吃满剩余高度，
            // 列表视口从 469 涨到 534（1330×800）、240 涨到 305（140% 字体）。
            crate::gui::widgets::split_row(
                ui,
                "rules:hits_header",
                96.0,
                |ui| {
                    icon_label(ui, Icon::Radar, 16.0, theme::accent());
                    ui.label(RichText::new("规则命中记录").strong().size(15.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{total} 条连接 · 兜底 MATCH {matched} 条（{pct}%） · \
                                 占比按连接数算，一条连接只计一次"
                            ))
                            .small()
                            .color(theme::text_secondary()),
                        )
                        .truncate(),
                    )
                    .on_hover_text("统计从本进程启动开始；命中每 30 秒自动落库一次。");
                },
                |ui| {
                    if ui
                        .button("立即刷新")
                        .on_hover_text("命中每 30 秒自动落库一次，这里手动拉最新那份")
                        .clicked()
                    {
                        refresh = true;
                    }
                },
            );
            ui.add_space(6.0);
            if empty {
                ui.label(
                    RichText::new("还没有命中记录：启动代理并跑点流量之后再回来看。")
                        .color(theme::muted()),
                );
                return;
            }

            // 规则命中分段分布条
            let mut direct_hits = 0f32;
            let mut proxy_hits = 0f32;
            let mut other_hits = 0f32;
            for hit in &self.rule_hits_today {
                if hit.target.eq_ignore_ascii_case("direct") || hit.rule.contains("DIRECT") {
                    direct_hits += hit.hits as f32;
                } else if hit.target.eq_ignore_ascii_case("proxy") || hit.target == "全部" {
                    proxy_hits += hit.hits as f32;
                } else {
                    other_hits += hit.hits as f32;
                }
            }
            let segments = [
                ("国内直连 (DIRECT)", direct_hits, theme::border()),
                ("出海代理 (PROXY)", proxy_hits, theme::accent()),
                ("智能策略分流", other_hits, theme::accent_strong()),
            ];
            crate::gui::widgets::render_distribution_bar(ui, &segments, total as f32, 7.0);
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                header_cell(ui, "命中的规则");
                header_cell(ui, "派给");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    header_cell(ui, "条数 / 占比");
                });
            });
            egui::ScrollArea::vertical()
                .id_salt("rule_hits_table")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for hit in &self.rule_hits_today {
                        let share = if total > 0 { hit.hits * 100 / total } else { 0 };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&hit.rule).color(theme::accent()).monospace());
                            ui.label(RichText::new(format!("→ {}", hit.target)));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("{} 条 · {}%", hit.hits, share))
                                            .strong()
                                            .color(theme::text_primary()),
                                    );
                                },
                            );
                        });
                    }
                });
        });
        if refresh {
            self.reload_rule_hits();
        }
    }

    pub(crate) fn ui_proxy_rules_config(&mut self, ui: &mut Ui) {
        let editable = !self.proxy_busy;
        let show_custom_rules = matches!(
            self.proxy_config.routing_mode,
            RoutingMode::Rule | RoutingMode::Subscription
        );

        // 整个面板一起滚：模式卡 + Geo 卡 + 规则卡
        egui::ScrollArea::vertical()
            .id_salt("proxy_rules_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section_title(
                    ui,
                    "分流模式",
                    routing_mode_description(self.proxy_config.routing_mode),
                );
                let prev_mode = self.proxy_config.routing_mode;
                let mut mode_changed = false;
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Rule,
                                "智能规则",
                            ).clicked() {
                                mode_changed = true;
                            }
                            if ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Subscription,
                                "订阅规则",
                            ).clicked() {
                                mode_changed = true;
                            }
                            if ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::ChinaDirect,
                                "中国大陆直连",
                            ).clicked() {
                                mode_changed = true;
                            }
                            if ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::GfwList,
                                "GFW 列表代理",
                            ).clicked() {
                                mode_changed = true;
                            }
                            if ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Global,
                                "全局代理",
                            ).clicked() {
                                mode_changed = true;
                            }
                            if ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Direct,
                                "全部直连",
                            ).clicked() {
                                mode_changed = true;
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(routing_mode_detail(self.proxy_config.routing_mode))
                                .small()
                                .color(theme::muted()),
                        );
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理配置中...").color(theme::muted()));
                    }
                });

                // 选中分流模式后自动静默保存，杜绝全屏 UI 锁定与重度重绘
                if mode_changed || self.proxy_config.routing_mode != prev_mode {
                    if self.proxy_config.routing_mode == RoutingMode::Subscription {
                        let sid = self.proxy_config.subscription_rules_source_id.as_deref();
                        if let Some(src) = self.sources.iter().find(|s| sid == Some(s.id.as_str())) {
                            if let Some(ref y) = src.rules_yaml {
                                self.custom_rules_text = y.clone();
                            }
                        }
                    } else if prev_mode == RoutingMode::Subscription {
                        if let Ok(lines) = crate::proxy::rule_io::export_lines(&self.proxy_config.rules) {
                            self.custom_rules_text = lines
                                .into_iter()
                                .map(|l| format!("- {l}"))
                                .collect::<Vec<_>>()
                                .join("\n");
                        }
                    }
                    self.save_proxy_config_quiet();
                }

                if self.proxy_config.routing_mode == RoutingMode::Subscription {
                    ui.add_space(8.0);
                    card_frame().show(ui, |ui| {
                        ui.label(RichText::new("订阅分流规则与策略组模板选用").strong());
                        ui.label(
                            RichText::new("选用某个已有订阅的原生分流规则和策略组架构，节点动态注入当前全部聚合健康节点：")
                                .small()
                                .color(theme::muted()),
                        );
                        ui.add_space(6.0);

                        let valid_sources: Vec<(String, String, usize, bool)> = self
                            .sources
                            .iter()
                            .filter(|s| s.rules_yaml.is_some() || s.groups_yaml.is_some())
                            .map(|s| {
                                let lines = s.rules_yaml.as_deref().map(|y| y.as_bytes().iter().filter(|&&b| b == b'\n').count()).unwrap_or(0);
                                (
                                    s.id.clone(),
                                    s.name.clone(),
                                    lines,
                                    s.groups_yaml.is_some(),
                                )
                            })
                            .collect();

                        if valid_sources.is_empty() {
                            ui.label(
                                RichText::new("⚠️ 当前暂无包含分流规则/策略组的订阅源。拉取或导入 Clash YAML 订阅后即可在此选用。")
                                    .color(theme::warning())
                                    .small(),
                            );
                        } else {
                            let mut current_id = self
                                .proxy_config
                                .subscription_rules_source_id
                                .clone()
                                .unwrap_or_default();
                            if !valid_sources.iter().any(|(id, _, _, _)| id == &current_id) {
                                current_id = valid_sources[0].0.clone();
                                self.proxy_config.subscription_rules_source_id = Some(current_id.clone());
                            }

                            let current_name = valid_sources
                                .iter()
                                .find(|(id, _, _, _)| id == &current_id)
                                .map(|(_, name, _, _)| name.as_str())
                                .unwrap_or("未选择");

                            ui.horizontal(|ui| {
                                ui.label(RichText::new("当前选用模板：").strong());
                                let mut selected_changed = false;
                                egui::ComboBox::from_id_salt("subscription_rules_source_combo")
                                    .selected_text(current_name)
                                    .width(320.0)
                                    .show_ui(ui, |ui| {
                                        for (s_id, s_name, rule_lines, has_groups) in &valid_sources {
                                            let info = match (*rule_lines > 0, *has_groups) {
                                                (true, true) => format!(" ({rule_lines} 条规则 · 包含策略组)"),
                                                (true, false) => format!(" ({rule_lines} 条规则)"),
                                                (false, true) => " (包含策略组)".to_string(),
                                                (false, false) => String::new(),
                                            };
                                            let display = format!("{s_name}{info}");
                                            if ui.selectable_value(&mut current_id, s_id.to_string(), display).clicked() {
                                                selected_changed = true;
                                            }
                                        }
                                    });
                                if selected_changed {
                                    self.proxy_config.subscription_rules_source_id = Some(current_id.clone());
                                    // 不同的模板显示不同的规则：同步当前选用模板的规则到编辑框
                                    if let Some(src) = self.sources.iter().find(|s| s.id == current_id) {
                                        if let Some(ref y) = src.rules_yaml {
                                            self.custom_rules_text = y.clone();
                                        } else {
                                            self.custom_rules_text.clear();
                                        }
                                    }
                                    self.save_proxy_config_quiet();
                                }
                            });

                            if let Some(src) = self.sources.iter().find(|s| s.id == current_id) {
                                ui.add_space(4.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new("模板详情:").small().color(theme::muted()));
                                    if let Some(ref r) = src.rules_yaml {
                                        let count = r.as_bytes().iter().filter(|&&b| b == b'\n').count();
                                        ui.label(
                                            RichText::new(format!("{count} 条分流规则"))
                                                .small()
                                                .color(theme::accent()),
                                        );
                                    }
                                    if let Some(ref g) = src.groups_yaml {
                                        let groups = fast_extract_group_names(g);
                                        if !groups.is_empty() {
                                            let display_groups = if groups.len() > 5 {
                                                format!("{} 个策略组: {} 等", groups.len(), groups[..5].join(", "))
                                            } else {
                                                format!("{} 个策略组: {}", groups.len(), groups.join(", "))
                                            };
                                            ui.label(
                                                RichText::new(display_groups)
                                                    .small()
                                                    .color(theme::accent()),
                                            )
                                            .on_hover_text(format!("包含全部策略组 (共 {} 个):\n{}", groups.len(), groups.join("\n")));
                                        }
                                    }
                                });
                            }
                        }
                    });
                }

                if !show_custom_rules {
                    return;
                }

                // 首次进入时，根据当前分流模式自动填入规则
                if self.proxy_config.routing_mode == RoutingMode::Subscription {
                    if self.custom_rules_text.trim().is_empty() {
                        let sid = self.proxy_config.subscription_rules_source_id.as_deref();
                        if let Some(src) = self.sources.iter().find(|s| sid == Some(s.id.as_str())) {
                            if let Some(ref yaml) = src.rules_yaml {
                                self.custom_rules_text = yaml.clone();
                            }
                        }
                    }
                } else if self.custom_rules_text.trim().is_empty() && !self.proxy_config.rules.is_empty() {
                    if let Ok(lines) = crate::proxy::rule_io::export_lines(&self.proxy_config.rules) {
                        self.custom_rules_text = lines
                            .into_iter()
                            .map(|l| format!("- {l}"))
                            .collect::<Vec<_>>()
                            .join("\n");
                    }
                }

                ui.add_space(8.0);

                // 自定义例外规则卡片：采用与分流策略组配置tab一致的卡片风格与顶部操作栏
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        let mut save_rules_clicked = false;
                        let mut auto_optimize = false;
                        let mut clear_rules = false;

                        let is_sub = self.proxy_config.routing_mode == RoutingMode::Subscription;
                        let header_title = if is_sub {
                            "订阅模板规则与自定义例外 (Subscription Rules)"
                        } else {
                            "自定义例外规则 (Custom Rules)"
                        };

                        // 固定顶部操作栏
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Rules, 14.0, theme::text_secondary());
                            ui.label(
                                RichText::new(header_title)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add_enabled(editable, primary_btn("保存规则"))
                                    .on_hover_text("解析文本并保存自定义规则至配置，触发代理内核热重载")
                                    .clicked()
                                {
                                    save_rules_clicked = true;
                                }
                                if ui
                                    .add_enabled(editable, egui::Button::new("自动优化"))
                                    .on_hover_text("解析规则、自动去重并按标准顺序优化排序")
                                    .clicked()
                                {
                                    auto_optimize = true;
                                }
                                if ui
                                    .add_enabled(editable, egui::Button::new("清空规则"))
                                    .on_hover_text("清空当前编辑框中的全部自定义规则")
                                    .clicked()
                                {
                                    clear_rules = true;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        let hint = if is_sub {
                            "当前选用订阅模板的分流规则（支持在此直接调整或添加自定义例外，保存后即刻热重载生效）："
                        } else {
                            "内置全自动分流引擎已接管国内直连与海外代理，开箱即用。在此处添加高优先级自定义例外规则（每行一条，支持 - 前缀与 # 注释）："
                        };
                        ui.label(
                            RichText::new(hint)
                                .small()
                                .color(theme::muted()),
                        );
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);

                        // 编辑框上方工具栏：常用场景分流预设下拉框 + 嗅探活动进程
                        let mut selected_preset: Option<(&'static str, Vec<TrafficRule>)> = None;
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Lightbulb, 14.0, theme::accent());
                            ui.label(RichText::new("常用场景预设:").small().strong().color(theme::text_secondary()));

                            egui::ComboBox::from_id_salt("custom_rules_preset_dropdown")
                                .selected_text("选择预设场景以添加规则...")
                                .width(280.0)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(false, "🤖 AI 服务分流 (ChatGPT / Claude / Gemini / Copilot)").clicked() {
                                        selected_preset = Some(("AI 服务分流", ai_service_rules()));
                                    }
                                    if ui.selectable_label(false, "🎬 海外主流流媒体 (YouTube / Netflix / Spotify)").clicked() {
                                        selected_preset = Some(("海外主流流媒体", streaming_media_rules()));
                                    }
                                    if ui.selectable_label(false, "💬 即时通讯 (Telegram / Discord / X)").clicked() {
                                        selected_preset = Some(("即时通讯", social_telegram_rules()));
                                    }
                                    if ui.selectable_label(false, "⚡ 国内开发镜像直连 (npm / 清华源 / 阿里源)").clicked() {
                                        selected_preset = Some(("国内开发镜像直连", dev_mirrors_rules()));
                                    }
                                    if ui.selectable_label(false, "🎮 游戏平台直连 (Steam / Epic)").clicked() {
                                        selected_preset = Some(("游戏平台直连", game_platforms_rules()));
                                    }
                                    if ui.selectable_label(false, "🛡️ 广告与 QUIC 拦截 (AdBlock / QUIC)").clicked() {
                                        selected_preset = Some(("广告与 QUIC 拦截", adblock_quic_rules()));
                                    }
                                    if ui.selectable_label(false, "🎯 推荐全套分流 (局域网/国内直连 + 广告拦截 + 其余代理)").clicked() {
                                        selected_preset = Some(("推荐全套分流", crate::proxy::recommended_traffic_rules()));
                                    }
                                });

                            ui.menu_button("⚡ 嗅探活动进程 ▼", |ui| {
                                ui.set_min_width(220.0);
                                ui.label(RichText::new("当前系统运行的应用进程").strong().color(theme::text_secondary()));
                                ui.separator();
                                let procs = crate::platform::list_running_processes();
                                if procs.is_empty() {
                                    ui.label(RichText::new("未检测到活跃进程").small().color(theme::muted()));
                                } else {
                                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                        for proc in procs {
                                            if ui.button(&proc).clicked() {
                                                let line = format!("- PROCESS-NAME,{proc},DIRECT");
                                                if self.custom_rules_text.trim().is_empty() {
                                                    self.custom_rules_text = line;
                                                } else {
                                                    if !self.custom_rules_text.ends_with('\n') {
                                                        self.custom_rules_text.push('\n');
                                                    }
                                                    self.custom_rules_text.push_str(&line);
                                                }
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            });
                        });

                        // 选中预设后，追加规则到编辑框文本末尾
                        if let Some((name, preset_rules)) = selected_preset {
                            let mut added_lines = Vec::new();
                            for r in &preset_rules {
                                if let Ok(line) = r.to_mihomo_rule() {
                                    added_lines.push(format!("- {line}"));
                                }
                            }
                            if !added_lines.is_empty() {
                                if self.custom_rules_text.trim().is_empty() {
                                    self.custom_rules_text = added_lines.join("\n");
                                } else {
                                    if !self.custom_rules_text.ends_with('\n') {
                                        self.custom_rules_text.push('\n');
                                    }
                                    self.custom_rules_text.push_str(&added_lines.join("\n"));
                                }
                                self.status = format!("已在编辑框中追加【{name}】{} 条预设规则", added_lines.len());
                            }
                        }

                        ui.add_space(6.0);

                        // 多行代码文本编辑器（独立垂直滚动）
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(360.0)
                            .min_scrolled_height(220.0)
                            .id_salt("proxy_custom_rules_editor_scroll")
                            .show(ui, |ui| {
                                ui.add_enabled_ui(editable, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.custom_rules_text)
                                            .font(egui::TextStyle::Monospace)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(18)
                                            .hint_text("# 可在此处粘贴或输入自定义规则，每行一条（支持 - 前缀与 # 注释）：\n- DOMAIN-SUFFIX,openai.com,PROXY\n- DOMAIN-KEYWORD,github,PROXY\n- IP-CIDR,192.168.0.0/16,DIRECT\n- GEOIP,CN,DIRECT\n- PROCESS-NAME,chrome.exe,PROXY"),
                                    );
                                });
                            });

                        if auto_optimize {
                            let import = crate::proxy::rule_io::parse_rules_text(&self.custom_rules_text);
                            let before = import.rules.len();
                            let optimized = optimize_traffic_rules(&import.rules);
                            let removed = before.saturating_sub(optimized.len());
                            if let Ok(lines) = crate::proxy::rule_io::export_lines(&optimized) {
                                self.custom_rules_text = lines.into_iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n");
                            }
                            self.status = if removed == 0 {
                                "规则已经是最优顺序，没有发现重复项".into()
                            } else {
                                format!("已自动移除 {removed} 条重复规则，点击「保存规则」生效")
                            };
                        }

                        if clear_rules {
                            self.custom_rules_text.clear();
                            self.status = "已清空规则编辑框，点击「保存规则」生效".into();
                        }

                        if save_rules_clicked {
                            let import = crate::proxy::rule_io::parse_rules_text(&self.custom_rules_text);
                            self.proxy_config.rules = import.rules;
                            if self.proxy_config.routing_mode == RoutingMode::Subscription {
                                self.proxy_config.subscription_rules_yaml = Some(self.custom_rules_text.clone());
                            }
                            self.save_proxy_config();
                            if !import.unparsed.is_empty() {
                                self.status = format!(
                                    "已保存 {} 条分流规则并热重载（{} 行无法识别跳过）",
                                    self.proxy_config.rules.len(),
                                    import.unparsed.len()
                                );
                            } else {
                                self.status = format!(
                                    "已保存 {} 条分流规则并热重载",
                                    self.proxy_config.rules.len()
                                );
                            }
                        }
                    });

                let injected = mihomo_dns_poison_overrides()
                    .iter()
                    .filter_map(|rule| rule.to_mihomo_rule().ok())
                    .collect::<Vec<_>>();
                if !injected.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "内置注入（不可编辑，插在 GEOIP,CN 之前；Mihomo / VP-Core / Meow 内核生效）：{}",
                            injected.join(" · ")
                        ))
                        .small()
                        .color(theme::muted()),
                    );
                }
            });

        self.ui_recommended_rules_confirm(ui.ctx());
    }

    /// 导出当前规则表：写一份 JSON 到 `data/exports/`，同时复制到剪贴板。
    /// 没有原生"另存为"对话框（不为此引依赖），所以走"文件 + 剪贴板"两条路，
    /// 哪条成了/没成都写在状态栏里，不假装成功。
    pub(crate) fn export_rules_as_json(&mut self) {
        let rules = &self.proxy_config.rules;
        let count = rules.len();
        let json = match crate::proxy::rule_io::export_json(rules) {
            Ok(json) => json,
            Err(error) => {
                self.status = format!("导出失败：{error}");
                return;
            }
        };
        let directory = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("data")
            .join("exports");
        let name = format!(
            "rules-{}-{}.json",
            crate::app_update::current_version(),
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let written = std::fs::create_dir_all(&directory)
            .and_then(|_| std::fs::write(directory.join(&name), &json))
            .map(|_| directory.join(name))
            .map_err(|error| error.to_string());
        let copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(json))
            .map_err(|error| error.to_string());
        let mut parts = vec![format!("导出 {count} 条规则")];
        parts.push(match &written {
            Ok(path) => format!("已写入 {}", path.display()),
            Err(error) => format!("写文件失败：{error}"),
        });
        parts.push(match copied {
            Ok(_) => "并复制到剪贴板".to_string(),
            Err(error) => format!("但剪贴板复制失败：{error}"),
        });
        self.status = parts.join(" · ");
    }
}

/// 高性能纯文本流式扫描 YAML 策略组名，避免每帧反序列化重载阻塞 UI
fn fast_extract_group_names(yaml: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_groups = false;
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("proxy-groups:") || trimmed.starts_with("proxy-groups :") {
            in_groups = true;
            continue;
        }
        if in_groups {
            if !line.starts_with(' ') && !line.starts_with('\t') && trimmed.contains(':') && !trimmed.starts_with('-') {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("- name:") {
                let name = rest.trim().trim_matches(|c| c == '"' || c == '\'' || c == ' ');
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            } else if trimmed.starts_with("name:") {
                let name = trimmed.trim_start_matches("name:").trim().trim_matches(|c| c == '"' || c == '\'' || c == ' ');
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
    }
    if names.is_empty() {
        names = crate::gui::views::proxy::groups::override_group_names(Some(yaml));
    }
    names
}


use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    RuleDraftKind, RulesTab,
};
use crate::gui::widgets::{
    card_frame, fold_title, header_cell, panel_tab,
    primary_btn,
    routing_mode_description, routing_mode_detail, rule_action_label, rule_matcher_label, section_title,
};
use crate::gui::VpnApp;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules,
    social_telegram_rules, streaming_media_rules, ProxyMode,
    ProxyState, RoutingMode, RuleAction, TrafficRule,
};
use eframe::egui::{self, RichText, Ui};

/// 分流规则那行表头右侧动作组（导出 JSON + 导入规则）预留的宽度。
/// 「?」不在这里了 —— 示范只在导入时用得上，2026-10-15 他要求挪进导入弹框。
pub(crate) const RULES_ACTIONS_WIDTH: f32 = 180.0;

/// 两个分区标签（规则配置 / 命中记录）至少要占的宽度。
pub(crate) const RULES_TABS_MIN_WIDTH: f32 = 170.0;

/// `split_row` 两个闭包都要碰 `self`，所以动作先记成值、出框后再执行。
pub(crate) enum RulesHeaderAction {
    None,
    Export,
    Import,
}

impl VpnApp {
    pub(crate) fn ui_proxy_rules(&mut self, ui: &mut Ui) {
        section_title(
            ui,
            self.tr("分流规则"),
            "域名匹配 · IP 分流 · 端口过滤 · 规则集与命中统计",
        );

        // 两个闭包都要动 self，所以 tab 状态先取副本、动作先记成枚举，出框后再落地。
        let mut tab = self.rules_tab;
        // 动作组按"这一帧进来时"的分区决定画不画；`tab` 被左闭包可变借用，这里另存一份。
        let tab_now = self.rules_tab;
        let busy = self.proxy_busy;
        let mut action = RulesHeaderAction::None;
        // 系统显示缩放 125%/150% 叠上字体 140% 时，960 物理最小宽只剩 550~460 逻辑点，
        // 中央面板不到 420 —— 一行放不下"两个分区标签 + 250 宽的动作组"，右对齐那组会
        // 直接压到标签上（2026-10-15 他报的"导出 JSON 被挤出去"）。放不下就让它换一行。
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
                |ui| Self::rules_actions(ui, tab_now, busy, &mut action),
            );
        } else {
            let _ = ui.horizontal(|ui| {
                panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
            });
            // 连一行都放不下时（200% 系统缩放 + 140% 字体）让它自己折，别越界。
            let _ = ui.horizontal_wrapped(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Self::rules_actions(ui, tab_now, busy, &mut action);
                });
            });
        }
        self.rules_tab = tab;
        match action {
            RulesHeaderAction::None => {}
            RulesHeaderAction::Export => self.export_rules_as_json(),
            RulesHeaderAction::Import => self.show_rule_import = true,
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

    /// 分流规则表头右侧那组动作（导出 JSON / 导入规则）。
    /// 两个闭包都要碰 `self`，所以这里只把点击记进 `action`，由调用方出框后再执行。
    fn rules_actions(ui: &mut Ui, tab: RulesTab, busy: bool, action: &mut RulesHeaderAction) {
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
        let mut save_mode_clicked = false;
        let mut save_clicked = false;
        let mut add_rule = false;
        let mut add_recommended = false;
        let mut optimize_rules = false;
        let mut move_rule: Option<(usize, usize)> = None;
        let mut remove_rule = None;

        // 整个面板一起滚：模式卡 + Geo 卡 + 预设卡本身就占掉大半屏，只给规则列表配
        // 滚动条的话，下面的卡片与"保存规则"按钮仍然会被挤出可视区。
        egui::ScrollArea::vertical()
            .id_salt("proxy_rules_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section_title(
                    ui,
                    "分流模式",
                    routing_mode_description(self.proxy_config.routing_mode),
                );
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Rule,
                                "智能规则（全自动·推荐）",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Subscription,
                                "订阅规则（选用订阅）",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::ChinaDirect,
                                "中国大陆直连",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::GfwList,
                                "GFW 列表代理",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Global,
                                "全局代理",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Direct,
                                "全部直连",
                            );
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(routing_mode_detail(self.proxy_config.routing_mode))
                                .small()
                                .color(theme::muted()),
                        );
                        ui.add_space(8.0);
                        save_mode_clicked = ui.add(primary_btn("保存分流模式")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理配置中...").color(theme::muted()));
                    }
                });

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

                        let valid_sources: Vec<(String, String, usize, bool, Option<String>, Option<String>)> = self
                            .sources
                            .iter()
                            .filter(|s| s.rules_yaml.is_some() || s.groups_yaml.is_some())
                            .map(|s| {
                                (
                                    s.id.clone(),
                                    s.name.clone(),
                                    s.rules_yaml.as_deref().map(|y| y.lines().count()).unwrap_or(0),
                                    s.groups_yaml.is_some(),
                                    s.rules_yaml.clone(),
                                    s.groups_yaml.clone(),
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
                            if !valid_sources.iter().any(|(id, _, _, _, _, _)| id == &current_id) {
                                current_id = valid_sources[0].0.clone();
                                self.proxy_config.subscription_rules_source_id = Some(current_id.clone());
                            }

                            let current_name = valid_sources
                                .iter()
                                .find(|(id, _, _, _, _, _)| id == &current_id)
                                .map(|(_, name, _, _, _, _)| name.as_str())
                                .unwrap_or("未选择");

                            ui.horizontal(|ui| {
                                ui.label(RichText::new("当前选用模板：").strong());
                                let mut selected_changed = false;
                                egui::ComboBox::from_id_salt("subscription_rules_source_combo")
                                    .selected_text(current_name)
                                    .width(320.0)
                                    .show_ui(ui, |ui| {
                                        for (s_id, s_name, rule_lines, has_groups, _, _) in &valid_sources {
                                            let info = match (*rule_lines > 0, *has_groups) {
                                                (true, true) => format!(" ({rule_lines} 条规则 · 包含策略组)"),
                                                (true, false) => format!(" ({rule_lines} 条规则)"),
                                                (false, true) => " (包含策略组)".to_string(),
                                                (false, false) => String::new(),
                                            };
                                            let display = format!("{s_name}{info}");
                                            if ui.selectable_value(&mut current_id, s_id.clone(), display).clicked() {
                                                selected_changed = true;
                                            }
                                        }
                                    });
                                if selected_changed {
                                    self.proxy_config.subscription_rules_source_id = Some(current_id.clone());
                                    self.save_proxy_config();
                                }
                            });

                            if let Some((_, _, _, _, ref r_yaml, ref g_yaml)) = valid_sources.iter().find(|(id, _, _, _, _, _)| id == &current_id) {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("模板详情:").small().color(theme::muted()));
                                    if let Some(ref r) = r_yaml {
                                        let count = r.lines().count();
                                        ui.label(
                                            RichText::new(format!("{count} 条分流规则"))
                                                .small()
                                                .color(theme::accent()),
                                        );
                                    }
                                    if let Some(ref g) = g_yaml {
                                        let groups = crate::gui::views::proxy::groups::override_group_names(Some(g));
                                        if !groups.is_empty() {
                                            ui.label(
                                                RichText::new(format!("{} 个策略组: {}", groups.len(), groups.join(", ")))
                                                    .small()
                                                    .color(theme::accent()),
                                            );
                                        }
                                    }
                                });
                            }
                        }
                    });
                }

                ui.add_space(8.0);
                card_frame().show(ui, |ui| {
                    // 描述与按钮分两行：横向并排时长描述会把按钮顶出可视区。
                    ui.label(RichText::new("Geo 规则数据库维护").strong());
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "在线更新 GeoIP (中国大陆IP库) 与 GeoSite (域名分流库) 规则文件",
                            )
                            .small()
                            .color(theme::muted()),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let running = self.proxy_status.state == ProxyState::Running;
                        if ui
                            .add_enabled(
                                !self.geo_updating,
                                primary_btn("一键更新 Geo 规则"),
                            )
                            .on_hover_text(if running {
                                "向当前运行的内核发送指令在线拉取并热重载最新的 Geo 数据库"
                            } else {
                                "直连极速 CDN 高速拉取并保存最新的 GeoIP 与 GeoSite 数据库（无需开启代理）"
                            })
                            .clicked()
                        {
                            self.update_geo_data();
                        }
                        if self.geo_updating {
                            ui.spinner();
                            ui.label(RichText::new("正在高速下载更新 Geo 规则...").small().color(theme::accent()));
                        } else if !running {
                            ui.label(RichText::new("（直连极速 CDN，无需开启代理）").small().color(theme::muted()));
                        }
                    });
                });

                if !show_custom_rules {
                    return;
                }

                ui.add_space(8.0);
                card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Lightbulb, 15.0, theme::accent());
                        ui.label(RichText::new("常用场景分流预设（一键添加至例外规则）").strong());
                    });
                    ui.label(
                        RichText::new(
                            "智能规则已内置全自动分流，点击对应场景可一键添加个性化定制规则并自动热重载生效",
                        )
                        .small()
                        .color(theme::muted()),
                    );
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "AI 服务分流 (ChatGPT / Claude / Gemini)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 OpenAI、Claude、Gemini、Copilot 域名走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in ai_service_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "海外主流流媒体 (YouTube / Netflix / Spotify)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 YouTube、Netflix、Disney+、Spotify 走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in streaming_media_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "即时通讯 (Telegram / Discord / X)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 Telegram 域名与 IP 段、Discord、X 走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in social_telegram_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "国内开发镜像直连 (npm / 清华源 / 阿里源)",
                                theme::accent_strong(),
                            ))
                            .on_hover_text("国内常见开源包与镜像站直连加速")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in dev_mirrors_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "游戏平台直连 (Steam / Epic)",
                                theme::accent_strong(),
                            ))
                            .on_hover_text("Steam 下载 CDN 与游戏商店直连")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in game_platforms_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Shield,
                                "广告与 QUIC 拦截",
                                theme::accent(),
                            ))
                            .on_hover_text("拦截常见广告并阻止 QUIC 降速")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in adblock_quic_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                    });
                });

                ui.add_space(8.0);
                // 默认收起（用户 2026-10-20）：规则一多，这一节展开着会把下面的命中记录和
                // 「保存规则」一起挤出可视区。标题上带条数，收起时也知道里面有多少东西。
                let rule_count = self.proxy_config.rules.len();
                egui::CollapsingHeader::new(fold_title(&format!(
                    "自定义例外规则 · {rule_count} 条（全自动分流已在后台生效）"
                )))
                .id_salt("custom_rules_section")
                .show_background(false)
                .show(ui, |ui| {
                    card_frame().show(ui, |ui| {
                    ui.label(
                        RichText::new("内置全自动分流引擎已接管国内直连与海外代理，开箱即用无需手动维护。如有特定私有域名需特殊处理，可在此添加高优先级例外规则。")
                            .small()
                            .color(theme::muted()),
                    );
                    ui.add_space(4.0);
                    ui.add_enabled_ui(editable, |ui| {
                        for index in 0..self.proxy_config.rules.len() {
                            let matcher =
                                rule_matcher_label(&self.proxy_config.rules[index].matcher);
                            ui.horizontal(|ui| {
                                ui.add_sized([260.0, 22.0], egui::Label::new(matcher));
                                egui::ComboBox::from_id_salt(("rule_action", index))
                                    .selected_text(rule_action_label(
                                        self.proxy_config.rules[index].action,
                                    ))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Direct,
                                            "直连",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Proxy,
                                            "代理",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Reject,
                                            "拒绝",
                                        );
                                    });
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑"))
                                    .on_hover_text("上移")
                                    .clicked()
                                {
                                    move_rule = Some((index, index - 1));
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < self.proxy_config.rules.len(),
                                        egui::Button::new("↓"),
                                    )
                                    .on_hover_text("下移")
                                    .clicked()
                                {
                                    move_rule = Some((index, index + 1));
                                }
                                if ui.button("×").on_hover_text("删除").clicked() {
                                    remove_rule = Some(index);
                                }
                            });
                        }

                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("new_rule_kind")
                                .selected_text(self.new_rule_kind.label())
                                .show_ui(ui, |ui| {
                                    for kind in RuleDraftKind::ALL {
                                        ui.selectable_value(
                                            &mut self.new_rule_kind,
                                            kind,
                                            kind.label(),
                                        );
                                    }
                                });
                            if self.new_rule_kind.needs_value() {
                                let rule_value_field = ui.add_sized(
                                    [220.0, 24.0],
                                    egui::TextEdit::singleline(&mut self.new_rule_value)
                                        .hint_text("匹配值"),
                                );
                                crate::gui::widgets::note_text_field(ui, &rule_value_field);
                            }
                            egui::ComboBox::from_id_salt("new_rule_action")
                                .selected_text(rule_action_label(self.new_rule_action))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Direct,
                                        "直连",
                                    );
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Proxy,
                                        "代理",
                                    );
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Reject,
                                        "拒绝",
                                    );
                                });
                            add_rule = ui.button("添加规则").clicked();
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
                                                self.new_rule_kind = RuleDraftKind::ProcessName;
                                                self.new_rule_value = proc.clone();
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            });

                            add_recommended = ui
                                .button("推荐分流")
                                .on_hover_text("覆盖为国内直连、广告拒绝、其余代理")
                                .clicked();
                            optimize_rules = ui
                                .button("自动优化")
                                .on_hover_text("移除重复规则并保持当前顺序")
                                .clicked();
                        });
                        if self.new_rule_kind == RuleDraftKind::ProcessName
                            && self.proxy_config.mode != ProxyMode::Tun
                        {
                            ui.label(
                                RichText::new(
                                    "PROCESS-NAME 只有 TUN 模式拿得到进程名，系统代理模式下不会生效",
                                )
                                .small()
                                .color(theme::warning()),
                            );
                        }
                        ui.label(
                            RichText::new("推荐规则：局域网与国内直连，广告和 QUIC 拒绝，其余代理")
                                .small()
                                .color(theme::muted()),
                        );

                        ui.add_space(8.0);
                        save_clicked = ui.add(primary_btn("保存规则")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理规则中...").color(theme::muted()));
                    }
                });
                });

                let injected = mihomo_dns_poison_overrides()
                    .iter()
                    .filter_map(|rule| rule.to_mihomo_rule().ok())
                    .collect::<Vec<_>>();
                if !injected.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "内置注入（不可编辑，仅 Mihomo 生效，插在 GEOIP,CN 之前）：{}",
                            injected.join(" · ")
                        ))
                        .small()
                        .color(theme::muted()),
                    );
                }
            });

        if save_mode_clicked {
            self.save_proxy_config();
        }
        if let Some((from, to)) = move_rule {
            self.proxy_config.rules.swap(from, to);
        }
        if let Some(index) = remove_rule {
            self.proxy_config.rules.remove(index);
        }
        if add_rule {
            let rule = TrafficRule {
                matcher: self.new_rule_kind.matcher(&self.new_rule_value),
                action: self.new_rule_action,
            };
            match rule.to_mihomo_rule() {
                Ok(_) => {
                    self.proxy_config.rules.push(rule);
                    self.new_rule_value.clear();
                }
                Err(error) => self.status = format!("规则无效: {error}"),
            }
        }
        if add_recommended {
            self.show_recommended_rules_confirm = true;
        }
        if optimize_rules {
            let before = self.proxy_config.rules.len();
            self.proxy_config.rules = optimize_traffic_rules(&self.proxy_config.rules);
            let removed = before.saturating_sub(self.proxy_config.rules.len());
            self.status = if removed == 0 {
                "规则已经是最优顺序，没有发现重复项".into()
            } else {
                format!("已自动移除 {removed} 条重复规则，请保存规则")
            };
        }
        if save_clicked {
            self.save_proxy_config();
        }
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

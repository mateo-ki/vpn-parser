use crate::gui::theme;
use crate::gui::widgets::{
    primary_btn, status_badge,
};
use crate::gui::VpnApp;
use crate::proxy::recommended_traffic_rules;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};

impl VpnApp {
    pub(crate) fn ui_rule_import(&mut self, ctx: &egui::Context) {
        if !self.show_rule_import {
            return;
        }
        let mut open = true;
        let mut apply = false;
        // `Window::open` 已经借走了 `open`，闭包里只能记一笔，出框之后再关。
        let mut cancel = false;

        // 半透明遮罩：Order::Tooltip 确保压在抽屉 (Foreground) 之上
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("rule_import_modal_backdrop"))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.set_width(screen.width());
                ui.set_height(screen.height());
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(100));
            });

        egui::Window::new("导入分流规则")
            .id(egui::Id::new("vpn-parser:rule-import"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .order(egui::Order::Tooltip)
            .default_width(680.0)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "支持三种形状：JSON 数组、带 rules/prependRules 字段的 JSON 对象、\
                         以及一行一条的文本（可带引号、尾逗号与 // 注释）。\
                         目标写自定义组名的，落到本程序的代理组。",
                    )
                    .small()
                    .color(theme::muted()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("从剪贴板读取")
                        .on_hover_text("整段粘进来即可，不用先摘引号")
                        .clicked()
                    {
                        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                            Ok(text) => {
                                self.rule_import_text = text.clone();
                                self.rule_import_preview =
                                    Some(crate::proxy::rule_io::parse_rules_text(&text));
                            }
                            Err(error) => self.status = format!("读剪贴板失败：{error}"),
                        }
                    }
                    // 「?」贴着「从剪贴板读取」：示范段落就是给人抄进这个框的。
                    let help = ui.add(
                        egui::Button::new(
                            RichText::new("?")
                                .strong()
                                .size(12.5)
                                .color(theme::accent()),
                        )
                        .min_size(Vec2::new(22.0, 22.0))
                        .fill(Color32::TRANSPARENT),
                    );
                    if help
                        .on_hover_text("导入格式示范：四种输入形状 + 支持/会跳过的类型")
                        .clicked()
                    {
                        self.show_rule_import_help = true;
                    }
                    if !self.rule_import_text.is_empty() && ui.button("清空").clicked() {
                        self.rule_import_text.clear();
                        self.rule_import_preview = None;
                    }
                });
                let field = ui.add(
                    egui::TextEdit::multiline(&mut self.rule_import_text)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("DOMAIN-SUFFIX,example.com,PROXY"),
                );
                crate::gui::widgets::note_text_field(ui, &field);
                if field.changed() {
                    let text = self.rule_import_text.clone();
                    self.rule_import_preview = Some(crate::proxy::rule_io::parse_rules_text(&text));
                }
                let preview = self.rule_import_preview.clone();
                match preview {
                    Some(preview) => {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(preview.summary())
                                .strong()
                                .color(theme::text_primary()),
                        );
                        if !preview.rules.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("rule_import_preview")
                                .max_height(140.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for line in crate::proxy::rule_io::export_lines(&preview.rules)
                                        .unwrap_or_default()
                                    {
                                        ui.label(RichText::new(line).small().monospace());
                                    }
                                });
                        }
                        if !preview.unparsed.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "认不出的前 {} 条：{}",
                                    preview.unparsed.len().min(3),
                                    preview
                                        .unparsed
                                        .iter()
                                        .take(3)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(" / ")
                                ))
                                .small()
                                .color(theme::warning()),
                            );
                        }
                        apply = ui
                            .add_enabled(
                                !preview.rules.is_empty() && !self.proxy_busy,
                                primary_btn("追加到规则表并保存"),
                            )
                            .on_hover_text("已存在的同样规则会跳过；保存时照常去重")
                            .clicked();
                    }
                    None => {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("粘贴或从剪贴板读取后，这里会先给解析结果再让导入。")
                                .small()
                                .color(theme::muted()),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if apply {
            self.apply_rule_import();
        }
        if !open {
            self.show_rule_import = false;
            self.rule_import_preview = None;
        }
    }

    /// 导入框旁边那个「?」：把每种能吃的输入形状各给一段可直接用的示范。
    /// 示范不是手写的漂亮话 —— `rule_io` 里有测试把每段喂回解析器，条数对不上就红，
    /// 所以照着抄不会错；「填入导入框」还会立刻开出解析预览让他先看清进了什么。
    pub(crate) fn ui_rule_import_help(&mut self, ctx: &egui::Context) {
        if !self.show_rule_import_help {
            return;
        }
        let (msg, _rect) = ui_rule_import_help_window(ctx);
        match msg {
            RuleHelpMsg::Nothing => {}
            RuleHelpMsg::Close => self.show_rule_import_help = false,
            RuleHelpMsg::Status(text) => self.status = text,
            RuleHelpMsg::Fill(body) => {
                self.rule_import_text = body.to_string();
                self.rule_import_preview = Some(crate::proxy::rule_io::parse_rules_text(
                    &self.rule_import_text,
                ));
                // 填完直接把导入框开出来：他要看到的就是"这段抄进去到底进了什么"。
                self.show_rule_import = true;
                self.show_rule_import_help = false;
            }
        }
    }

    fn apply_rule_import(&mut self) {
        let Some(preview) = self.rule_import_preview.clone() else {
            return;
        };
        let mut added = 0usize;
        for rule in &preview.rules {
            if self
                .proxy_config
                .rules
                .iter()
                .any(|existing| existing == rule)
            {
                continue;
            }
            self.proxy_config.rules.push(rule.clone());
            added += 1;
        }
        let duplicates = preview.rules.len() - added;
        self.status = format!(
            "导入完成：新增 {added} 条，跳过重复 {duplicates} 条，规则表现共 {} 条",
            self.proxy_config.rules.len()
        );
        if let Ok(lines) = crate::proxy::rule_io::export_lines(&self.proxy_config.rules) {
            self.custom_rules_text = lines
                .into_iter()
                .map(|l| format!("- {l}"))
                .collect::<Vec<_>>()
                .join("\n");
        }
        self.show_rule_import = false;
        self.rule_import_text.clear();
        self.rule_import_preview = None;
        self.save_proxy_config();
    }

    pub(crate) fn ui_recommended_rules_confirm(&mut self, ctx: &egui::Context) {
        if !self.show_recommended_rules_confirm {
            return;
        }

        let modal = egui::Modal::new(egui::Id::new("recommended_rules_confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("覆盖分流规则");
            ui.label("现有分流规则将被推荐规则覆盖，确认后立即保存。");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    Some(false)
                } else if ui.add(primary_btn("确认覆盖")).clicked() {
                    Some(true)
                } else {
                    None
                }
            })
            .inner
        });

        match modal.inner {
            Some(true) => {
                self.proxy_config.rules = recommended_traffic_rules();
                if let Ok(lines) = crate::proxy::rule_io::export_lines(&self.proxy_config.rules) {
                    self.custom_rules_text = lines
                        .into_iter()
                        .map(|l| format!("- {l}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                }
                self.show_recommended_rules_confirm = false;
                self.save_proxy_config();
            }
            Some(false) => self.show_recommended_rules_confirm = false,
            None if modal.should_close() => self.show_recommended_rules_confirm = false,
            None => {}
        }
    }

}

/// 「导入格式示范」弹框里点出来的动作。
/// 整个弹框抽成不碰 `VpnApp` 的自由函数，好让离屏 `egui::Context` 能单独渲染它做布局验收。
pub(crate) enum RuleHelpMsg {
    Nothing,
    Status(String),
    Fill(&'static str),
    Close,
}

fn set_help_msg(slot: &mut RuleHelpMsg, next: RuleHelpMsg) {
    if matches!(*slot, RuleHelpMsg::Nothing) {
        *slot = next;
    }
}

/// 每种能吃的输入形状给一段可直接抄用的示范 + 支持/跳过的类型清单。
/// 示范文本本身由 `rule_io` 的测试逐段喂回解析器（条数对不上就红），
/// 所以这里写的"可导入 N 条"不是宣传语。
/// 「导入格式示范」整个弹框。宽度与滚动区高度都跟着屏幕算 ——
/// 字体调到 140% 时逻辑屏幕只剩物理的 0.71 倍，写死尺寸会把窗口顶出右缘
/// （离屏量到 660 宽的屏面上窗口画到 662）。
/// 抽成不碰 `VpnApp` 的自由函数，好让离屏 `egui::Context` 真开一次窗口做验收。
pub fn ui_rule_import_help_window(ctx: &egui::Context) -> (RuleHelpMsg, egui::Rect) {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 96.0).clamp(320.0, 680.0);
    let scroll_height = (screen.height() * 0.5).clamp(180.0, 430.0);
    let mut open = true;
    let mut msg = RuleHelpMsg::Nothing;
    // `Window::open` 借走了 `open`，闭包里只记一笔，出框之后再合。
    let shown = egui::Window::new("导入格式示范")
        .id(egui::Id::new("vpn-parser:rule-import-help"))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .order(egui::Order::Tooltip)
        .default_width(width)
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            msg = ui_rule_import_help_body(ui, scroll_height);
        });
    let rect = shown
        .map(|inner| inner.response.rect)
        .unwrap_or(egui::Rect::NOTHING);
    if !open && matches!(msg, RuleHelpMsg::Nothing) {
        msg = RuleHelpMsg::Close;
    }
    (msg, rect)
}

fn ui_rule_import_help_body(ui: &mut Ui, scroll_height: f32) -> RuleHelpMsg {
    let mut msg = RuleHelpMsg::Nothing;
    ui.label(
        RichText::new(
            "目标写 PROXY / DIRECT / REJECT 以外的自定义组名，一律落到本程序的代理组；\
             GEOIP 的 no-resolve 由本程序自动补；重复的规则保存时去重。",
        )
        .small()
        .color(theme::muted()),
    );
    ui.label(
        RichText::new(format!(
            "认这些类型：{}",
            crate::proxy::rule_io::SUPPORTED_KINDS.join("、")
        ))
        .small()
        .color(theme::text_secondary()),
    );
    ui.label(
        RichText::new(format!(
            "点名跳过：{}、MATCH/FINAL 兜底行",
            crate::proxy::rule_io::UNSUPPORTED_KINDS.join("、")
        ))
        .small()
        .color(theme::warning()),
    );
    egui::ScrollArea::vertical()
        .id_salt("rule_import_help")
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for example in crate::proxy::rule_io::import_examples() {
                ui.add_space(8.0);
                // 每种格式一张卡：卡片用页面那档蓝灰，弹框本身是纯白容器，
                // 卡片内层的代码块再回到纯白 —— 三层靠深浅分，不靠描边堆。
                Frame::NONE
                    .fill(theme::page_bg())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(13, 11))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(example.title)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.add_space(6.0);
                            let (badge, color) = if example.expected_rules > 0 {
                                (
                                    format!("可导入 {} 条", example.expected_rules),
                                    theme::accent_strong(),
                                )
                            } else {
                                (
                                    format!("{} 条全部点名跳过", example.expected_unsupported),
                                    theme::warning(),
                                )
                            };
                            status_badge(ui, &badge, color);
                        });
                        ui.label(RichText::new(example.note).small().color(theme::muted()));
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border()))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                for line in example.body.lines() {
                                    ui.label(
                                        RichText::new(line)
                                            .small()
                                            .monospace()
                                            .color(theme::text_secondary()),
                                    );
                                }
                            });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("填入导入框")
                                .on_hover_text("整段塞进「导入规则」，先给解析预览再让导入")
                                .clicked()
                            {
                                set_help_msg(&mut msg, RuleHelpMsg::Fill(example.body));
                            }
                            if ui.button("复制这段").clicked() {
                                let outcome =
                                    arboard::Clipboard::new().and_then(|mut clipboard| {
                                        clipboard.set_text(example.body.to_string())
                                    });
                                set_help_msg(
                                    &mut msg,
                                    RuleHelpMsg::Status(match outcome {
                                        Ok(()) => "示范已复制到剪贴板".to_string(),
                                        Err(error) => format!("复制失败：{error}"),
                                    }),
                                );
                            }
                        });
                    });
            }
        });
    ui.add_space(10.0);
    if ui.button("关闭").clicked() {
        set_help_msg(&mut msg, RuleHelpMsg::Close);
    }
    msg
}

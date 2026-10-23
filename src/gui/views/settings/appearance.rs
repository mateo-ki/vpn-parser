use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{card_frame, status_badge};
use crate::gui::VpnApp;
use eframe::egui::{self, RichText, Ui};

/// 字号滑杆的刻度带留多高（刻度线 + 数字）。
pub(crate) const FONT_TICK_BAND_HEIGHT: f32 = 20.0;
pub(crate) const FONT_SCALE_MIN: i32 = 85;
pub(crate) const FONT_SCALE_MAX: i32 = 140;

impl VpnApp {
    /// 外观分区：配色 + 字体大小。两条都是"改了立刻生效并自动落库"，所以不挂底部那排
    /// 保存按钮 —— 和主题一贯的行为一致。
    pub(crate) fn ui_settings_appearance(&mut self, ui: &mut Ui) {
        let mut auto_save = false;
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Appearance, 16.0, theme::accent());
                ui.label(RichText::new(self.tr("界面外观")).strong().size(15.0));
                status_badge(
                    ui,
                    &format!("字体 {}%", (self.edit_ui_scale * 100.0).round() as i32),
                    theme::muted(),
                );
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(self.tr("改动即时生效并自动保存，下次启动沿用。"))
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(10.0);

            ui.label(RichText::new(self.tr("界面语言 (Interface Language)")).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for lang in crate::i18n::Language::ALL {
                    if ui
                        .selectable_value(
                            &mut self.edit_ui_language,
                            lang.code().to_string(),
                            lang.display_name(),
                        )
                        .clicked()
                    {
                        auto_save = true;
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new(self.tr("界面配色主题")).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for theme_spec in theme::THEMES {
                    let selected = self.edit_ui_theme == theme_spec.id;
                    let accent_col = theme::rgb(theme_spec.accent);
                    let padding = egui::vec2(10.0, 5.0);
                    let font = egui::FontId::proportional(12.5);
                    let galley = ui.painter().layout_no_wrap(
                        theme_spec.name.to_string(),
                        font,
                        if selected {
                            theme::accent_strong()
                        } else {
                            theme::text_primary()
                        },
                    );
                    let dot_size = 9.0;
                    let gap = 6.0;
                    let desired_size = egui::vec2(
                        galley.size().x + dot_size + gap + padding.x * 2.0,
                        galley.size().y.max(dot_size) + padding.y * 2.0,
                    );
                    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();
                        let bg = if selected {
                            theme::accent_bg()
                        } else if response.hovered() {
                            theme::panel_hover()
                        } else {
                            theme::panel()
                        };
                        let stroke = if selected {
                            egui::Stroke::new(1.2_f32, accent_col)
                        } else {
                            egui::Stroke::new(1.0_f32, theme::border_subtle())
                        };
                        painter.rect(
                            rect,
                            egui::CornerRadius::same(7),
                            bg,
                            stroke,
                            egui::StrokeKind::Inside,
                        );

                        let dot_center =
                            egui::pos2(rect.min.x + padding.x + dot_size / 2.0, rect.center().y);
                        painter.circle_filled(dot_center, dot_size / 2.0, accent_col);
                        if selected {
                            painter.circle_stroke(
                                dot_center,
                                dot_size / 2.0 + 2.0,
                                egui::Stroke::new(1.0_f32, accent_col),
                            );
                        }

                        let text_pos = egui::pos2(
                            dot_center.x + dot_size / 2.0 + gap,
                            rect.center().y - galley.size().y / 2.0,
                        );
                        painter.galley(
                            text_pos,
                            galley,
                            if selected {
                                theme::accent_strong()
                            } else {
                                theme::text_primary()
                            },
                        );
                    }
                    if response.clicked() {
                        self.edit_ui_theme = theme_spec.id.to_string();
                        auto_save = true;
                    }
                }
            });

            ui.add_space(10.0);
            // 夜间：只换中性色，强调色仍然是上面那 5 支里选的一支（两条轴正交）。
            let mut dark = self.edit_ui_dark;
            if ui
                .checkbox(&mut dark, RichText::new(self.tr("夜间模式（深色底）")))
                .on_hover_text(
                    "只换中性色（页面、卡片、文字），强调色还是上面选的那一支 —— 5 支强调色 × 明/暗。\n\
                     改动即时生效并自动保存，关掉就回到浅白。",
                )
                .changed()
            {
                self.edit_ui_dark = dark;
                auto_save = true;
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new(self.tr("窗口大小配置 (Window Dimensions)")).strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new("根据显示器或桌面环境选择合适视口大小，改动即时生效并自动保存。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                let tr_small = self.tr("小屏 (1024 × 680)");
                let tr_medium = self.tr("中屏 (1280 × 800)");
                let tr_large = self.tr("大屏 (1440 × 880)");
                let presets = [
                    ("small", tr_small, 1024.0, 680.0, "轻量小巧，适合窄屏、低分辨率或平铺桌面 (如 niri/sway)"),
                    ("medium", tr_medium, 1280.0, 800.0, "标准平衡比例，舒适适配绝大多数主流笔记本与显示器"),
                    ("large", tr_large, 1440.0, 880.0, "开阔大视野，提供最宽裕的监控图表、多列节点卡片与日志空间"),
                ];
                for (id, label, w, h, desc) in presets {
                    let selected = self.edit_window_size == id;
                    if ui.selectable_label(selected, label).on_hover_text(desc).clicked() {
                        self.edit_window_size = id.to_string();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                        auto_save = true;
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new(self.tr("控制台标签页显示 (Console Tabs)")).strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new("勾选/选中的标签页会在控制台顶部导航中显示，未选中的将被精简隐藏（至少保留 1 个）。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for panel in crate::gui::types::ProxyPanel::ALL {
                    let is_visible = self.is_proxy_panel_visible(panel);
                    let prefix = if is_visible { "✓ " } else { "+ " };
                    let chip_label = format!("{}{}", prefix, self.tr(panel.title()));
                    let tip = if is_visible {
                        "已在控制台显示，点击隐藏此 Tab"
                    } else {
                        "已隐藏，点击在控制台显示此 Tab"
                    };
                    if ui.selectable_label(is_visible, chip_label).on_hover_text(tip).clicked() {
                        self.toggle_proxy_panel_visibility(panel);
                        auto_save = true;
                    }
                }
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("字体大小").strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new("整屏一起缩放（文字、图标、表格都跟着走），100% = 跟随系统缩放。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            let mut percent = (self.edit_ui_scale * 100.0).round() as i32;
            let mut slider_geometry = None;
            ui.horizontal(|ui| {
                // egui 0.31 的 Slider 没有 desired_width，宽度只认 spacing.slider_width。
                let slider_width = ui.spacing().slider_width;
                const FONT_SLIDER_RAIL: f32 = 300.0;
                ui.spacing_mut().slider_width = FONT_SLIDER_RAIL;
                let response = ui
                    .add(
                        egui::Slider::new(&mut percent, 85..=140)
                            .step_by(5.0)
                            .suffix("%"),
                    )
                    .on_hover_text("85% 到 140%，步长 5%；松手才生效");
                ui.spacing_mut().slider_width = slider_width;
                slider_geometry = Some((
                    response.rect,
                    handle_travel_range(ui, &response, FONT_SLIDER_RAIL),
                ));
                if response.changed() {
                    self.edit_ui_scale = percent as f32 / 100.0;
                }
                // 只在松手那一下落库。拖动中每帧保存的话：`BgMsg::SettingsSaved` 回来会
                // `sync_edit_settings()` 把值按库里那份灌回去，手指还按在柄上，值被改 →
                // 缩放变 → 整页重排 → 滑杆几何跟着挪 → 看起来就是"页面在鬼畜"。
                if response.drag_stopped() && self.settings_dirty() {
                    self.save_settings();
                }
                if (self.edit_ui_scale - 1.0).abs() > 0.001
                    && ui
                        .button(self.tr("恢复 100%"))
                        .on_hover_text("回到跟随系统缩放")
                        .clicked()
                {
                    self.edit_ui_scale = 1.0;
                    auto_save = true;
                }
            });
            // 刻度条：先留出一条带，再照滑杆自己的几何画在下面。
            if let Some((rect, travel)) = slider_geometry {
                ui.add_space(FONT_TICK_BAND_HEIGHT);
                paint_font_scale_ticks(ui, rect, travel);
            }
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }

}

/// 手柄中心实际走过的区间。**必须**和 egui 自己算的一致，否则刻度和手柄会错位。
///
/// 两条来自 egui 0.31 源码的事实：① 手柄位置是按**轨道**算的，轨道宽 = `spacing.slider_width`，
/// 而 `ui.add(Slider)` 返回的 response 把右边那段数值文字（"100%"）也算进了宽度 ——
/// 直接拿 `response.rect.x_range()` 会把刻度整体往右推（实测 140% 处偏 48px，
/// 2026-10-15 他报的"点恢复 100% 小球跑到 90 刻度"）；② `handle_radius = 轨道高/2.5`，
/// `HandleShape::Rect` 再乘 `aspect_ratio`。
pub(crate) fn handle_travel_range(ui: &Ui, response: &egui::Response, rail_width: f32) -> egui::Rangef {
    let radius = response.rect.height() / 2.5;
    let radius = match ui.style().visuals.handle_shape {
        egui::style::HandleShape::Circle => radius,
        egui::style::HandleShape::Rect { aspect_ratio } => radius * aspect_ratio,
    };
    let left = response.rect.left();
    egui::Rangef::new(left + radius, left + rail_width - radius)
}

/// 每个刻度的 `(百分比, x, 是否长刻度)`。抽成纯函数是因为画出来的东西我看不见 ——
/// 几何只能这样钉成断言。
pub(crate) fn font_scale_ticks(travel: egui::Rangef) -> Vec<(i32, f32, bool)> {
    let span = (FONT_SCALE_MAX - FONT_SCALE_MIN) as f32;
    (FONT_SCALE_MIN..=FONT_SCALE_MAX)
        .step_by(5)
        .map(|value| {
            let ratio = (value - FONT_SCALE_MIN) as f32 / span;
            (
                value,
                egui::lerp(travel.min..=travel.max, ratio),
                matches!(value, 85 | 100 | 120 | 140),
            )
        })
        .collect()
}

/// 把刻度画在滑杆下方那条带里。100 用强调色 —— 那是"跟随系统缩放"的基准点，
/// 一眼要能找回来。
fn paint_font_scale_ticks(ui: &Ui, rect: egui::Rect, travel: egui::Rangef) {
    let painter = ui.painter();
    let top = rect.bottom() + 2.0;
    let color = theme::muted();
    for (value, x, major) in font_scale_ticks(travel) {
        let height = if major { 7.0 } else { 4.0 };
        painter.line_segment(
            [egui::pos2(x, top), egui::pos2(x, top + height)],
            egui::Stroke::new(
                1.0_f32,
                if major {
                    color
                } else {
                    color.gamma_multiply(0.5)
                },
            ),
        );
        if major {
            painter.text(
                egui::pos2(x, top + height + 1.0),
                egui::Align2::CENTER_TOP,
                format!("{value}"),
                egui::FontId::proportional(9.5),
                if value == 100 { theme::accent() } else { color },
            );
        }
    }
}


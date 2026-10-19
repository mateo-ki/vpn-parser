use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::Tab;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};


pub fn card_frame() -> Frame {
    Frame::NONE
        .fill(theme::panel())
        .stroke(theme::card_stroke())
        .shadow(theme::card_shadow())
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::same(theme::PAD_CARD))
}

pub fn panel_frame() -> Frame {
    Frame::NONE
        .fill(theme::panel())
        .stroke(theme::card_stroke())
        .shadow(theme::card_shadow())
        .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
        .inner_margin(Margin::symmetric(theme::PAD_CARD, theme::PAD_CARD - 2))
}

/// 主按钮 / 危险按钮：**不写 corner_radius**，跟着样式默认走（egui 的按钮是 2px）。
/// 以前它们各自钉了 `RADIUS_BTN=16`，于是同一条状态栏里「停止代理」是大圆角、旁边的
/// 「IP 检测」是 2px 方角（用户 2026-10-15：这俩要一致）。16 那个值留给胶囊/芯片用。
pub fn primary_btn(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong().color(theme::on_primary()))
        .fill(theme::primary_btn())
        .stroke(Stroke::NONE)
}

pub fn danger_btn(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(theme::text_secondary()))
        .fill(theme::panel_hover())
        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
}

pub fn section_title(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .strong()
                .size(17.0)
                .color(theme::text_primary()),
        );
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(subtitle)
                    .size(11.5)
                    .color(theme::muted()),
            )
            .truncate(),
        );
    });
    ui.add_space(8.0);
}

/// 纯图标导航项（macOS Dock 风格）：无文字、无角标数字，只保留语义矢量图标与状态圆点
pub fn nav_item_icon_only(
    ui: &mut Ui,
    current: &mut Tab,
    tab: Tab,
    icon: Icon,
    tooltip: &str,
    dot_color: Option<Color32>,
) {
    let selected = *current == tab;
    let (fill, stroke, icon_color) = if selected {
        (
            theme::accent_bg(),
            Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.4)),
            theme::accent(),
        )
    } else {
        (
            Color32::TRANSPARENT,
            Stroke::NONE,
            theme::icon_secondary(),
        )
    };

    let item_size = egui::vec2(ui.available_width(), 40.0);
    let (rect, resp) = ui.allocate_exact_size(item_size, Sense::click());
    let painter = ui.painter_at(rect);

    let is_hovered = resp.hovered();
    let bg_fill = if selected {
        fill
    } else if is_hovered {
        theme::panel().gamma_multiply(0.7)
    } else {
        fill
    };

    if bg_fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_NAV), bg_fill);
    }
    if stroke != Stroke::NONE {
        painter.rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS_NAV),
            stroke,
            egui::StrokeKind::Inside,
        );
    }

    // 左侧选中小指示条 (macOS 风格)
    if selected {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.min.x + 2.0, rect.center().y - 8.0),
            egui::vec2(3.0, 16.0),
        );
        painter.rect_filled(bar, CornerRadius::same(2), theme::accent());
    }

    // 居中绘制矢量图标 (加大至 22px，更加饱满清晰)
    let icon_size = 22.0;
    let icon_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
    let active_icon_color = if is_hovered && !selected {
        theme::icon_primary()
    } else {
        icon_color
    };
    crate::gui::icons::draw_icon(&painter, icon_rect, icon, active_icon_color);

    // 右上角状态微标发光圆点 (如代理运行状态)
    if let Some(color) = dot_color {
        let dot_pos = egui::pos2(rect.right() - 8.0, rect.top() + 8.0);
        painter.circle_filled(dot_pos, 4.0, color.gamma_multiply(0.3));
        painter.circle_filled(dot_pos, 2.5, color);
    }

    if resp.clicked() {
        *current = tab;
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip);
    ui.add_space(4.0);
}

/// 一行两段的表头：左段按自然宽度排，右段（动作组）钉在内容区右缘。
///
/// 为什么不用 `ui.horizontal` + `with_layout(right_to_left)`：右对齐那组只会往左挤，
/// 一旦逻辑宽度变小（字体调到 140% 时 1330 物理宽只剩 950 逻辑宽，再叠上侧栏就不到 500），
/// 整组会**越过面板右边界**被裁掉 —— 2026-10-14 系统设置的「保存设置」、
/// 2026-10-15 分流规则的「导出 JSON」都是这一个病（离屏量到 580 > 502）。
/// 这里给右段划一块算出来的矩形，右边缘就等于内容区右边缘；左段拿剩下的宽度，
/// 里面的长文字要自己 `.truncate()`。
pub fn split_row(
    ui: &mut Ui,
    id: &str,
    right_width: f32,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) {
    const ROW_HEIGHT: f32 = 30.0;
    const GAP: f32 = 8.0;
    let row_top = ui.cursor().top();
    let row = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), row_top),
        egui::pos2(ui.max_rect().right(), row_top + ROW_HEIGHT),
    );
    let group = right_width.min(row.width());
    let left_right = (row.right() - group - GAP).max(row.left());
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .id_salt(egui::Id::new(format!("{id}:left")))
            .max_rect(egui::Rect::from_min_max(
                row.min,
                egui::pos2(left_right, row.bottom()),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        left,
    );
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .id_salt(egui::Id::new(format!("{id}:right")))
            .max_rect(egui::Rect::from_min_max(
                egui::pos2(row.right() - group, row.top()),
                row.max,
            ))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        right,
    );
    if crate::diag::enabled() {
        // 量具：他报"按钮被挤出界面"时，像素反推不出是 max_rect 还是别的东西，
        // 就把真实矩形打出来对账（只在诊断壳下写，产品路径零开销）。
        crate::diag::log_once(&format!(
            "DIAGROW {id} screen={:.1} max=[{:.1},{:.1}] group=[{:.1},{:.1}]",
            ui.input(|i| i.screen_rect().width()),
            row.left(),
            row.right(),
            row.right() - group,
            row.right(),
        ));
    }
    ui.advance_cursor_after_rect(row);
}

/// 页面顶部的分段标签（控制台那几个面板、系统设置的通用/外观都用它）。
pub fn panel_tab<T: PartialEq + Copy>(
    ui: &mut Ui,
    current: &mut T,
    panel: T,
    icon: Icon,
    label: &str,
) {
    let selected = *current == panel;
    let (fill, text_color, icon_color) = if selected {
        (theme::accent_bg(), theme::accent_strong(), theme::accent())
    } else {
        (theme::panel(), theme::text_secondary(), theme::muted())
    };
    let stroke = if selected {
        Stroke::new(1.0_f32, theme::accent())
    } else {
        Stroke::new(1.0_f32, theme::border_subtle())
    };
    let resp = Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(theme::RADIUS_BTN))
        .inner_margin(Margin::symmetric(13, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, icon, 14.0, icon_color);
                ui.add_space(5.0);
                ui.label(RichText::new(label).strong().color(text_color).size(12.5));
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *current = panel;
    }
}

/// macOS 原生风格药丸分段选择条 (Segmented Control)
/// 整体置于统一外壳托盘内，激活项带高亮浮动药丸底色与轻柔边框，未激活项悬停高亮
pub fn macos_segmented_tabs<T: PartialEq + Copy>(
    ui: &mut Ui,
    current: &mut T,
    tabs: &[(T, Icon, &str)],
) {
    let tray_frame = Frame::NONE
        .fill(theme::sidebar())
        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(3, 3));

    tray_frame.show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().item_spacing.y = 2.0;
            for &(panel, icon, label) in tabs {
                let selected = *current == panel;
                let (fill, stroke, text_color, icon_color) = if selected {
                    if theme::is_light() {
                        (
                            Color32::WHITE,
                            Stroke::new(1.0_f32, Color32::from_black_alpha(20)),
                            theme::text_primary(),
                            theme::accent(),
                        )
                    } else {
                        (
                            Color32::from_rgb(44, 44, 50),
                            Stroke::new(1.0_f32, Color32::from_white_alpha(24)),
                            Color32::WHITE,
                            theme::accent(),
                        )
                    }
                } else {
                    (
                        Color32::TRANSPARENT,
                        Stroke::NONE,
                        theme::text_secondary(),
                        theme::muted(),
                    )
                };

                let resp = Frame::NONE
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(11, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, icon, 13.0, icon_color);
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(label)
                                    .strong()
                                    .size(12.0)
                                    .color(text_color),
                            );
                        });
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);

                if resp.clicked() {
                    *current = panel;
                }
            }
        });
    });
}


pub fn header_cell(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .strong()
            .color(theme::text_secondary())
            .small(),
    );
}


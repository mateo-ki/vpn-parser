use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{
    country_flag_and_name, latency_text, protocol_pill, render_country_code_badge,
    render_signal_bars_with_status, sanitize_node_name,
};
use crate::models::NodeSummary;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

pub fn compact_node_card(
    ui: &mut Ui,
    node: &NodeSummary,
    is_active: bool,
    is_focused: bool,
    is_switching: bool,
) -> egui::Response {
    let (fill, stroke) = if is_switching {
        (theme::warning_bg(), Stroke::new(1.5_f32, theme::warning()))
    } else if is_active {
        (theme::accent_bg(), Stroke::new(1.5_f32, theme::accent()))
    } else if is_focused {
        (
            theme::panel_hover(),
            Stroke::new(1.0_f32, theme::border()),
        )
    } else {
        (theme::panel(), Stroke::new(1.0_f32, theme::border_subtle()))
    };

    let mut frame = Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(10, 8));

    if is_active {
        frame = frame.shadow(egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: theme::accent().gamma_multiply(0.22),
        });
    }

    let response = frame
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(52.0);

            // 第一行：国家大写字母（方案 B 极简无框双色字母） + 节点名称 + 右侧活跃药丸
            ui.horizontal(|ui| {
                render_country_code_badge(ui, node.country.as_deref().unwrap_or(""));
                ui.add_space(2.0);

                let name_color = if is_switching {
                    theme::warning()
                } else if is_active {
                    theme::accent_strong()
                } else {
                    theme::text_primary()
                };

                let clean_name = sanitize_node_name(&node.name);
                let reserve_right = if is_active || is_switching { 46.0 } else { 0.0 };
                let available_text_w = (ui.available_width() - reserve_right).max(50.0);

                let name_label = ui.add_sized(
                    [available_text_w, 18.0],
                    egui::Label::new(
                        RichText::new(&clean_name)
                            .strong()
                            .size(12.5)
                            .color(name_color),
                    )
                    .truncate(),
                );
                name_label.on_hover_text(&node.name);

                if is_active {
                    let active_green = Color32::from_rgb(52, 199, 89);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        Frame::NONE
                            .fill(active_green.gamma_multiply(0.18))
                            .stroke(Stroke::new(
                                1.0_f32,
                                active_green.gamma_multiply(0.4),
                            ))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(5, 1))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (beacon, _) =
                                        ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
                                    ui.painter().circle_filled(
                                        beacon.center(),
                                        2.5,
                                        active_green,
                                    );
                                    ui.label(
                                        RichText::new("活跃")
                                            .small()
                                            .strong()
                                            .color(active_green),
                                    );
                                });
                            });
                    });
                } else if is_switching {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        icon_label(ui, Icon::Lightning, 10.0, theme::warning());
                        ui.label(
                            RichText::new("切换中")
                                .small()
                                .strong()
                                .color(theme::warning()),
                        );
                    });
                }
            });

            ui.add_space(4.0);

            // 第二行：协议标签 + 国家名称 + 右侧信号与延迟
            ui.horizontal(|ui| {
                protocol_pill(ui, &node.protocol.to_uppercase());

                if let Some(country) = &node.country {
                    let (_flag, name) = country_flag_and_name(country);
                    ui.label(
                        RichText::new(name)
                            .small()
                            .color(theme::text_secondary()),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(latency_text(node).small().strong());
                    ui.add_space(2.0);
                    render_signal_bars_with_status(ui, node.latency_ms, node.latency_ok);
                });
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    if response.hovered() && !is_active && !is_focused && !is_switching {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(12),
            Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.4)),
            egui::StrokeKind::Inside,
        );
    }

    response
}


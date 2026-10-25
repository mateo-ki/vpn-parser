use crate::gui::theme;
use crate::gui::widgets::format_speed;
use eframe::egui::{self, Sense, Stroke, Ui};

/// 半圆速度计仪表盘 (Speedometer Gauge)
/// 支持动态指示针、发光弧轨、实时速率与上传副读数
#[allow(dead_code)]
pub fn render_speedometer(
    ui: &mut Ui,
    down_bytes_per_sec: u64,
    up_bytes_per_sec: u64,
    is_active: bool,
) {
    let width = 144.0;
    let height = 96.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);

    // 几何中心与外半径
    let center = egui::pos2(rect.center().x, rect.min.y + 54.0);
    let radius = 42.0;
    let stroke_width = 6.5_f32;

    // 240 度开角：从 150 度 (左下) 顺时针扫至 390 度 (右下)
    let start_angle = 150.0_f32.to_radians();
    let sweep = 240.0_f32.to_radians();
    let steps = 36;

    // 底层暗灰背景轨
    let track_points: Vec<egui::Pos2> = (0..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let angle = start_angle + t * sweep;
            egui::pos2(center.x + radius * angle.cos(), center.y + radius * angle.sin())
        })
        .collect();
    painter.add(egui::Shape::line(
        track_points,
        Stroke::new(stroke_width, theme::border_subtle()),
    ));

    // 计算当前速度比例 (半对数层级分布，使得 0~10M/100M/1000M 均有灵敏反应)
    let ratio = if is_active {
        let mbps = (down_bytes_per_sec as f32 * 8.0) / 1_000_000.0;
        if mbps <= 0.05 {
            0.0
        } else if mbps <= 1.0 {
            (mbps / 1.0) * 0.25
        } else if mbps <= 10.0 {
            0.25 + ((mbps - 1.0) / 9.0) * 0.25
        } else if mbps <= 50.0 {
            0.50 + ((mbps - 10.0) / 40.0) * 0.25
        } else {
            (0.75 + ((mbps - 50.0) / 150.0) * 0.25).min(1.0)
        }
    } else {
        0.0
    };

    // 绘制激活的高亮发光进度弧
    if ratio > 0.005 {
        let active_steps = ((steps as f32 * ratio).ceil() as usize).max(2);
        let active_points: Vec<egui::Pos2> = (0..=active_steps)
            .map(|i| {
                let t = (i as f32 / active_steps as f32) * ratio;
                let angle = start_angle + t * sweep;
                egui::pos2(center.x + radius * angle.cos(), center.y + radius * angle.sin())
            })
            .collect();
        let glow_color = theme::accent();
        // 外层柔光
        painter.add(egui::Shape::line(
            active_points.clone(),
            Stroke::new(stroke_width + 4.0_f32, glow_color.gamma_multiply(0.25)),
        ));
        // 内层实线
        painter.add(egui::Shape::line(
            active_points,
            Stroke::new(stroke_width, glow_color),
        ));
    }

    // 绘制指针与中心轴心
    let needle_angle = start_angle + ratio * sweep;
    let needle_tip = egui::pos2(
        center.x + (radius - 9.0) * needle_angle.cos(),
        center.y + (radius - 9.0) * needle_angle.sin(),
    );
    let needle_color = if is_active { theme::accent() } else { theme::muted() };
    painter.line_segment([center, needle_tip], Stroke::new(2.0_f32, needle_color));
    painter.circle_filled(center, 4.0, needle_color);
    painter.circle_filled(center, 2.0, theme::panel());

    // 刻度标注 (0 / 100M+)
    let left_tick = egui::pos2(center.x + (radius + 8.0) * start_angle.cos(), center.y + (radius + 8.0) * start_angle.sin());
    let right_tick = egui::pos2(center.x + (radius + 8.0) * (start_angle + sweep).cos(), center.y + (radius + 8.0) * (start_angle + sweep).sin());
    painter.text(left_tick, egui::Align2::RIGHT_CENTER, "0", egui::FontId::proportional(8.5), theme::muted());
    painter.text(right_tick, egui::Align2::LEFT_CENTER, "100M+", egui::FontId::proportional(8.5), theme::muted());

    // 仪表盘中心文字读数
    let speed_color = if is_active && down_bytes_per_sec > 0 {
        theme::text_primary()
    } else {
        theme::muted()
    };
    painter.text(
        egui::pos2(center.x, center.y + 16.0),
        egui::Align2::CENTER_CENTER,
        format_speed(down_bytes_per_sec),
        egui::FontId::proportional(13.0),
        speed_color,
    );

    // 底部副读数
    let up_str = format!("↑ {}", format_speed(up_bytes_per_sec));
    painter.text(
        egui::pos2(center.x, center.y + 30.0),
        egui::Align2::CENTER_CENTER,
        up_str,
        egui::FontId::proportional(10.0),
        theme::text_secondary(),
    );

    response.on_hover_text(format!(
        "实时测速仪表盘\n↓ 下载速率: {}\n↑ 上传速率: {}",
        format_speed(down_bytes_per_sec),
        format_speed(up_bytes_per_sec)
    ));
}


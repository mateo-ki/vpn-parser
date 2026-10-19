use super::sparkline::{chart_scale, smooth_polyline};
use crate::gui::theme;
use crate::gui::widgets::format_speed;
use eframe::egui::{self, Stroke, Ui};
use std::collections::VecDeque;

/// 实时吞吐大波形图：平滑 Catmull-Rom 贝塞尔曲线、下行半透明发光渐变填充、上行虚线、悬停交叉准星与标尺
pub fn render_traffic_waveform_card(
    ui: &mut Ui,
    history: &VecDeque<(f32, f32)>,
    active: bool,
    height: f32,
) {
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let hovered = response.hovered();
    let painter = ui.painter_at(rect);
    let baseline = rect.max.y - 4.0;
    let top_padding = 18.0;
    let usable_h = (height - top_padding - 6.0).max(10.0);

    // 底线
    painter.line_segment(
        [
            egui::pos2(rect.min.x, baseline),
            egui::pos2(rect.max.x, baseline),
        ],
        Stroke::new(1.0_f32, theme::border_subtle()),
    );

    if !active || history.len() < 2 {
        // 未启动代理或暂无采样时，显示平直基准线与标尺（一条线），无大字遮挡
        let ruler_y = baseline - usable_h;
        painter.line_segment(
            [
                egui::pos2(rect.min.x + 4.0, ruler_y),
                egui::pos2(rect.max.x - 4.0, ruler_y),
            ],
            Stroke::new(0.8_f32, theme::border_subtle().gamma_multiply(0.4)),
        );
        painter.text(
            egui::pos2(rect.min.x + 6.0, rect.min.y + 2.0),
            egui::Align2::LEFT_TOP,
            "峰值量程 0.0 KB/s · 待机",
            egui::FontId::proportional(9.0),
            theme::muted(),
        );

        // 绘制贯穿视口的平直流量监控基准线（一条线）
        let line_color = theme::accent().gamma_multiply(0.7);
        painter.line_segment(
            [
                egui::pos2(rect.min.x + 4.0, baseline),
                egui::pos2(rect.max.x - 4.0, baseline),
            ],
            Stroke::new(1.8_f32, line_color),
        );
        // 右端点指示发光光标
        painter.circle_filled(egui::pos2(rect.max.x - 4.0, baseline), 3.5, line_color.gamma_multiply(0.3));
        painter.circle_filled(egui::pos2(rect.max.x - 4.0, baseline), 2.0, line_color);

        if hovered {
            response.on_hover_text("实时流量监视器 · 待机 (0.0 KB/s)\n启动代理后将实时绘制平滑吞吐波形");
        }
        return;
    }

    let downs: Vec<f32> = history.iter().map(|&(down, _)| down).collect();
    let ups: Vec<f32> = history.iter().map(|&(_, up)| up).collect();
    let scale = chart_scale(
        &history
            .iter()
            .flat_map(|&(d, u)| [d, u])
            .collect::<Vec<f32>>(),
    );
    let last = history.len() - 1;
    let dx = (rect.width() - 8.0) / last as f32;
    let at = |index: usize, value: f32| {
        let ratio = (value / scale).clamp(0.0, 1.0);
        egui::pos2(
            rect.min.x + 4.0 + index as f32 * dx,
            baseline - ratio * usable_h,
        )
    };
    let curve = |values: &[f32]| -> Vec<egui::Pos2> {
        let raw: Vec<egui::Pos2> = values
            .iter()
            .enumerate()
            .map(|(index, &value)| at(index, value))
            .collect();
        smooth_polyline(&raw, 4)
    };

    let down_curve = curve(&downs);
    let up_curve = curve(&ups);

    // 绘制量程上限淡色虚线指示，隔离文字区与波形区
    let ruler_y = baseline - usable_h;
    painter.line_segment(
        [
            egui::pos2(rect.min.x + 4.0, ruler_y),
            egui::pos2(rect.max.x - 4.0, ruler_y),
        ],
        Stroke::new(0.8_f32, theme::border_subtle().gamma_multiply(0.4)),
    );

    // 绘制下行半透明发光渐变区域填充（采用主题强调色，逐梯形填充避免凹多边形变形）
    let down_color = theme::accent();
    let fill_color = down_color.gamma_multiply(0.14);
    for window in down_curve.windows(2) {
        let p1 = window[0];
        let p2 = window[1];
        let b1 = egui::pos2(p1.x, baseline);
        let b2 = egui::pos2(p2.x, baseline);
        painter.add(egui::Shape::convex_polygon(
            vec![p1, p2, b2, b1],
            fill_color,
            Stroke::NONE,
        ));
    }

    // 绘制上行曲线（虚线）：浅色下加深对比度，深色下微透光
    let up_color = if theme::is_light() {
        theme::accent_strong()
    } else {
        theme::accent().gamma_multiply(0.65)
    };
    for dash in egui::Shape::dashed_line(&up_curve, Stroke::new(1.3_f32, up_color), 3.0, 2.5) {
        painter.add(dash);
    }

    // 绘制下行主曲线（发光实线）
    painter.add(egui::Shape::line(
        down_curve.clone(),
        Stroke::new(1.8_f32, down_color),
    ));

    // 最新点端点发光球
    painter.circle_filled(at(last, ups[last]), 2.0, up_color);
    painter.circle_filled(at(last, downs[last]), 3.5, down_color.gamma_multiply(0.3));
    painter.circle_filled(at(last, downs[last]), 2.2, down_color);

    // 标尺文字 (左上角当前量程上限，置于量程虚线上方的独立保护区，与最高波峰保持垂直安全间隙)
    let max_speed_str = format_speed((scale * 1024.0) as u64);
    painter.text(
        egui::pos2(rect.min.x + 6.0, rect.min.y + 2.0),
        egui::Align2::LEFT_TOP,
        format!("峰值量程 {max_speed_str}"),
        egui::FontId::proportional(9.0),
        theme::muted(),
    );

    // 悬停交互十字准星与时间点数据
    if hovered {
        if let Some(point) = ui.input(|i| i.pointer.hover_pos()) {
            if point.x >= rect.min.x && point.x <= rect.max.x {
                let index = (((point.x - rect.min.x - 4.0) / dx).round() as usize).min(last);
                let x = at(index, 0.0).x;
                painter.line_segment(
                    [egui::pos2(x, rect.min.y + 2.0), egui::pos2(x, baseline)],
                    Stroke::new(1.0_f32, theme::border()),
                );
                let ago = (last - index) as u64;
                response.on_hover_text(format!(
                    "{ago} 秒前 · ↓ {} · ↑ {}",
                    format_speed((downs[index] * 1024.0) as u64),
                    format_speed((ups[index] * 1024.0) as u64),
                ));
            }
        }
    }
}


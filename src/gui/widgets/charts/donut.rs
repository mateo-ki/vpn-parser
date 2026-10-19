use crate::gui::theme;
use eframe::egui::{self, Color32, Sense, Stroke, Ui};

/// 协议分布环形图 (Donut Chart)
/// 呈现节点协议的占比圆环、中心总数与交互提示
pub fn render_protocol_donut_chart(
    ui: &mut Ui,
    stats: &[crate::models::ProtocolStat],
    total_nodes: i64,
) {
    render_protocol_donut_chart_sized(ui, stats, total_nodes, 96.0);
}

/// 支持自定义外径尺寸的协议分布环形图 (Donut Chart)
pub fn render_protocol_donut_chart_sized(
    ui: &mut Ui,
    stats: &[crate::models::ProtocolStat],
    total_nodes: i64,
    size_px: f32,
) {
    let size = egui::vec2(size_px, size_px);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);

    let center = rect.center();
    let scale = size_px / 96.0;
    let radius = 37.0 * scale;
    let stroke_width = 9.5_f32 * scale;

    if total_nodes <= 0 || stats.is_empty() {
        painter.circle_stroke(
            center,
            radius,
            Stroke::new(stroke_width, theme::border_subtle()),
        );
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            "0",
            egui::FontId::proportional(14.0 * scale),
            theme::muted(),
        );
        return;
    }

    let mut current_angle = -std::f32::consts::FRAC_PI_2; // 从 12 点钟方向开始
    let gap = if stats.len() > 1 { 0.05_f32 } else { 0.0 };

    let mut tooltip_lines = Vec::new();
    tooltip_lines.push(format!("节点协议分布 (共 {total_nodes} 节点):"));

    for (idx, s) in stats.iter().enumerate() {
        let ratio = (s.count as f32 / total_nodes as f32).clamp(0.0, 1.0);
        if ratio < 0.005 {
            continue;
        }
        let sweep = ratio * (std::f32::consts::PI * 2.0) - gap;
        if sweep <= 0.0 {
            current_angle += ratio * (std::f32::consts::PI * 2.0);
            continue;
        }

        let steps = ((sweep / (std::f32::consts::PI * 2.0) * 32.0).ceil() as usize).max(4);
        let arc_points: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                let angle = current_angle + t * sweep;
                egui::pos2(center.x + radius * angle.cos(), center.y + radius * angle.sin())
            })
            .collect();

        // 方案二：莫兰迪低饱和环形图色彩体系 (柔和护眼 · 层次丰富)
        let color = match idx {
            0 => theme::accent(),
            1 => Color32::from_rgb(82, 121, 111),  // 莫兰迪鼠尾草绿 #52796F
            2 => Color32::from_rgb(168, 101, 116), // 莫兰迪陶土玫瑰 #A86574
            3 => Color32::from_rgb(122, 154, 139), // 莫兰迪柔和橄榄 #7A9A8B
            4 => Color32::from_rgb(136, 132, 158), // 莫兰迪雾霭紫 #88849E
            _ => theme::border(),
        };
        painter.add(egui::Shape::line(
            arc_points,
            Stroke::new(stroke_width, color),
        ));

        current_angle += sweep + gap;

        let pct = ratio * 100.0;
        tooltip_lines.push(format!("• {}: {:.1}% ({} 节点)", s.protocol.to_uppercase(), pct, s.count));
    }

    // 中心数字读数 (例如 1248 节点)
    let total_str = if total_nodes >= 10000 {
        format!("{:.1}w", total_nodes as f64 / 10000.0)
    } else if total_nodes >= 1000 {
        format!("{:.1}k", total_nodes as f64 / 1000.0)
    } else {
        format!("{total_nodes}")
    };

    painter.text(
        egui::pos2(center.x, center.y - 7.0 * scale),
        egui::Align2::CENTER_CENTER,
        total_str,
        egui::FontId::proportional(13.5 * scale),
        theme::text_primary(),
    );
    painter.text(
        egui::pos2(center.x, center.y + 9.0 * scale),
        egui::Align2::CENTER_CENTER,
        "节点",
        egui::FontId::proportional(10.0 * scale),
        theme::muted(),
    );

    response.on_hover_text(tooltip_lines.join("\n"));
}


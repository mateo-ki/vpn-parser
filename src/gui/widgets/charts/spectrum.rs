use crate::gui::theme;
use eframe::egui::{self, Color32, CornerRadius, Sense, Ui};

/// 延迟频谱分布统计数据
#[derive(Clone, Copy, Debug, Default)]
pub struct LatencySpectrumStats {
    pub ultra_fast: usize, // < 100ms
    pub fast: usize,       // 100~200ms
    pub medium: usize,     // 200~400ms
    pub slow: usize,       // > 400ms
    pub untested: usize,   // 未测速
    pub total: usize,
}

impl LatencySpectrumStats {
    pub fn from_latencies<'a>(latencies: impl IntoIterator<Item = Option<i64>>) -> Self {
        let mut stats = Self::default();
        for lat in latencies {
            stats.total += 1;
            match lat {
                Some(ms) if ms > 0 && ms <= 100 => stats.ultra_fast += 1,
                Some(ms) if ms > 100 && ms <= 200 => stats.fast += 1,
                Some(ms) if ms > 200 && ms <= 400 => stats.medium += 1,
                Some(ms) if ms > 400 => stats.slow += 1,
                _ => stats.untested += 1,
            }
        }
        stats
    }
}

/// 渲染节点全库延迟分布阶梯频谱指示条 (Latency Spectrum Bar)
pub fn render_latency_spectrum(
    ui: &mut Ui,
    stats: &LatencySpectrumStats,
) {
    if stats.total == 0 {
        return;
    }

    let total = stats.total as f32;
    let width = ui.available_width().max(160.0);
    let bar_height = 13.0_f32;

    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, bar_height), Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, CornerRadius::same(6), theme::sidebar());

    // 方案二：莫兰迪低饱和延迟阶梯色系（柔和护眼 · 语义保留，彻底替代刺眼荧光大红大绿）
    let tiers: [(&str, usize, Color32); 5] = [
        ("极速 <100ms", stats.ultra_fast, Color32::from_rgb(82, 121, 111)),  // 莫兰迪鼠尾草绿 #52796F
        ("良好 100-200ms", stats.fast, Color32::from_rgb(122, 154, 139)),   // 莫兰迪柔和橄榄 #7A9A8B
        ("中等 200-400ms", stats.medium, Color32::from_rgb(198, 138, 76)),  // 莫兰迪暖沙琥珀 #C68A4C
        ("迟缓 >400ms", stats.slow, Color32::from_rgb(158, 90, 84)),       // 莫兰迪陶土玫瑰粉褐 #9E5A54 替代大红
        ("未测速", stats.untested, theme::border_subtle()),
    ];

    let mut current_x = rect.min.x;
    let mut tooltips = Vec::new();

    for (label, count, color) in tiers {
        if count == 0 {
            continue;
        }
        let seg_w = (count as f32 / total) * rect.width();
        let seg_rect = egui::Rect::from_min_max(
            egui::pos2(current_x, rect.min.y),
            egui::pos2((current_x + seg_w).min(rect.max.x), rect.max.y),
        );
        let cr = if current_x <= rect.min.x + 0.5 && current_x + seg_w >= rect.max.x - 0.5 {
            CornerRadius::same(4)
        } else if current_x <= rect.min.x + 0.5 {
            CornerRadius { nw: 4, sw: 4, ne: 0, se: 0 }
        } else if current_x + seg_w >= rect.max.x - 1.0 {
            CornerRadius { nw: 0, sw: 0, ne: 4, se: 4 }
        } else {
            CornerRadius::same(0)
        };
        painter.rect_filled(seg_rect, cr, color);
        current_x += seg_w;

        let pct = (count as f32 / total) * 100.0;
        tooltips.push(format!("• {}: {} 节点 ({:.1}%)", label, count, pct));
    }

    response.on_hover_text(format!("全网延迟阶梯频谱分布：\n{}", tooltips.join("\n")));
}


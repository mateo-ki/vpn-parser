use crate::gui::theme;
use crate::gui::widgets::{clock_of, format_speed};
use eframe::egui::{self, Color32, CornerRadius, Sense, Stroke, Ui, Vec2};

/// Catmull-Rom 平滑：把折点采样成曲线，避免心电图式的硬折角。
pub(crate) fn smooth_polyline(points: &[egui::Pos2], steps: usize) -> Vec<egui::Pos2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity((points.len() - 1) * steps + 1);
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            let mix = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * ((2.0 * b)
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (-a + 3.0 * b - 3.0 * c + d) * t3)
            };
            out.push(egui::pos2(
                mix(p0.x, p1.x, p2.x, p3.x),
                mix(p0.y, p1.y, p2.y, p3.y),
            ));
        }
    }
    out.push(*points.last().expect("non-empty"));
    out
}

/// 单路流量的自动量程：取窗口内 92 分位再留 25% 余量，避免一次突发峰值把整条
/// 曲线长期压成贴底的小锯齿；突发会随 150 点窗口自然滑出。
pub(crate) fn chart_scale(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 64.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let percentile = sorted[(sorted.len() * 92 / 100).min(sorted.len() - 1)];
    (percentile * 1.25).max(64.0)
}

/// 订阅源"最近 7 次拉取"色条：新的在左边。绿=成功（含 304/内容未变），深墨=失败，
/// 浅描边=没有记录。不用红黄（用户 2026-10-06）。
pub fn render_fetch_ticks(ui: &mut Ui, rows: Option<&Vec<crate::db::telemetry::FetchHistoryRow>>) {
    const SLOTS: usize = 7;
    const BOX: f32 = 9.0;
    let size = Vec2::new(SLOTS as f32 * (BOX + 2.0), BOX);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let mut rows: Vec<&crate::db::telemetry::FetchHistoryRow> =
        rows.map(|list| list.iter().collect()).unwrap_or_default();
    rows.reverse();
    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    let painter = ui.painter();
    for slot in 0..SLOTS {
        let x = rect.min.x + slot as f32 * (BOX + 2.0);
        let cell = egui::Rect::from_min_max(
            egui::pos2(x, rect.min.y),
            egui::pos2(x + BOX, rect.min.y + BOX),
        );
        let fill = match rows.get(slot) {
            Some(row) if row.ok == 1 => {
                ok_count += 1;
                theme::accent()
            }
            Some(_) => {
                fail_count += 1;
                theme::muted()
            }
            None => theme::border(),
        };
        painter.rect_filled(cell, 2.0, fill);
    }
    let latest = rows
        .first()
        .map(|row| format!("{} · {}", clock_of(&row.at), row.status))
        .unwrap_or_else(|| "没有拉取记录".into());
    response.on_hover_text(format!(
        "最近 {} 次：{ok_count} 次成功 · {fail_count} 次失败（左新右旧）。最近一次：{latest}",
        ok_count + fail_count
    ));
}



/// 表头里的迷你流量图：下载实线、上传虚线，两条共用一个量程 —— 各用各的量程会让
/// 300 B/s 的上传画得和 30 KB/s 的下载一样高，那是假象。32px 高塞不下坐标和文字，
/// 读数走 hover；累计值在窗口底部状态栏本来就有。
#[allow(dead_code)]
pub fn render_traffic_sparkline(
    ui: &mut Ui,
    history: &std::collections::VecDeque<(f32, f32)>,
    active: bool,
) {
    const HEIGHT: f32 = 32.0;
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::hover());
    let hovered = response.hovered();
    let painter = ui.painter_at(rect);
    let baseline = rect.max.y - 2.0;
    let usable_h = HEIGHT - 9.0;
    painter.line_segment(
        [
            egui::pos2(rect.min.x, baseline),
            egui::pos2(rect.max.x, baseline),
        ],
        Stroke::new(1.0_f32, theme::border()),
    );

    if !active || history.len() < 2 {
        response.on_hover_text(if active {
            "等待流量统计…"
        } else {
            "代理未运行 · 启动后这里画实时上下行"
        });
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
    let dx = (rect.width() - 6.0) / last as f32;
    let at = |index: usize, value: f32| {
        let ratio = (value / scale).clamp(0.0, 1.0);
        egui::pos2(
            rect.min.x + 3.0 + index as f32 * dx,
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

    // 上传先画、下载压在它上面：下载通常大一个量级，反过来画上传会被盖住。
    let up_color = theme::accent().gamma_multiply(0.65);
    for dash in egui::Shape::dashed_line(&curve(&ups), Stroke::new(1.3_f32, up_color), 3.0, 2.5) {
        painter.add(dash);
    }
    let down_color = theme::accent();
    painter.add(egui::Shape::line(
        curve(&downs),
        Stroke::new(1.6_f32, down_color),
    ));
    painter.circle_filled(at(last, ups[last]), 1.8, up_color);
    painter.circle_filled(at(last, downs[last]), 2.2, down_color);

    if hovered {
        if let Some(point) = ui.input(|i| i.pointer.hover_pos()) {
            if point.x >= rect.min.x && point.x <= rect.max.x {
                let index = (((point.x - rect.min.x - 3.0) / dx).round() as usize).min(last);
                let x = at(index, 0.0).x;
                painter.line_segment(
                    [egui::pos2(x, rect.min.y + 1.0), egui::pos2(x, baseline)],
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

/// 4 格手机/WiFi 风格延迟信号柱图
#[allow(dead_code)]
pub fn render_signal_bars(ui: &mut Ui, latency_ms: Option<i64>) {
    render_signal_bars_with_status(ui, latency_ms, None);
}

/// 4 格手机/WiFi 风格延迟信号柱图（带测速状态判定）
/// 规则：<= 700ms 绿色，< 1500ms 黄色，其他 (>= 1500ms 或超时/失败) 红色
pub fn render_signal_bars_with_status(
    ui: &mut Ui,
    latency_ms: Option<i64>,
    latency_ok: Option<bool>,
) {
    let size = egui::vec2(16.0, 13.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);

    let baseline = rect.max.y;
    let bar_w = 2.5;
    let bar_gap = 1.5;
    let heights = [3.5_f32, 6.5_f32, 9.5_f32, 12.5_f32];

    let green = Color32::from_rgb(52, 199, 89);
    let yellow = Color32::from_rgb(255, 149, 0);
    let red = Color32::from_rgb(255, 59, 48);

    let (active_bars, bar_color) = match (latency_ms, latency_ok) {
        (Some(ms), _) if ms <= 700 => {
            let bars = if ms <= 350 { 4 } else { 3 };
            (bars, green)
        }
        (Some(ms), _) if ms < 1500 => (2, yellow),
        (Some(_), _) => (1, red),
        (None, Some(false)) => (1, red),
        (None, _) => (0, theme::border_subtle()),
    };

    let dim_color = theme::border_subtle();

    for i in 0..4 {
        let x = rect.min.x + i as f32 * (bar_w + bar_gap);
        let h = heights[i];
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(x, baseline - h),
            egui::pos2(x + bar_w, baseline),
        );
        let fill = if i < active_bars {
            bar_color
        } else {
            dim_color
        };
        painter.rect_filled(bar_rect, CornerRadius::same(1), fill);
    }

    let tooltip = match (latency_ms, latency_ok) {
        (Some(ms), _) => format!("延迟: {ms} ms (信号: {active_bars}/4 格)"),
        (None, Some(false)) => "测速超时 / 失败 (1 格)".to_string(),
        (None, _) => "未测速 / 离线".to_string(),
    };
    response.on_hover_text(tooltip);
}


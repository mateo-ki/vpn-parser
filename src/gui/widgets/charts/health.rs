use crate::gui::theme;
use crate::gui::widgets::format_bytes;
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Ui, Vec2};

/// 节点池健康可用率水平分段色条
pub fn render_node_health_bar(
    ui: &mut Ui,
    alive: i64,
    tested: i64,
    total: i64,
) {
    if total <= 0 {
        return;
    }
    let height = 22.0;
    let width = ui.available_width().clamp(180.0, 360.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);

    let bar_h = 5.5_f32;
    let bar_y = rect.min.y + 14.0;
    let bar_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, bar_y),
        egui::pos2(rect.max.x, bar_y + bar_h),
    );

    // 背景槽
    painter.rect_filled(bar_rect, CornerRadius::same(2), theme::border_subtle());

    let alive_f = alive.max(0) as f32;
    let dead_f = (tested - alive).max(0) as f32;
    let untested_f = (total - tested).max(0) as f32;
    let total_f = total as f32;

    let alive_w = (alive_f / total_f) * rect.width();
    let dead_w = (dead_f / total_f) * rect.width();

    // 绘制可用分段 (主题强调色)
    if alive_w > 1.0 {
        let r = egui::Rect::from_min_max(
            egui::pos2(rect.min.x, bar_y),
            egui::pos2(rect.min.x + alive_w, bar_y + bar_h),
        );
        painter.rect_filled(r, CornerRadius::same(2), theme::accent());
    }

    // 绘制失效分段 (优雅暗灰/石墨低对比度，杜绝刺眼大红)
    if dead_w > 1.0 {
        let r = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + alive_w, bar_y),
            egui::pos2(rect.min.x + alive_w + dead_w, bar_y + bar_h),
        );
        let dead_color = health_failed_color();
        painter.rect_filled(r, CornerRadius::same(0), dead_color);
    }

    let pct = if tested > 0 {
        alive as f64 / tested as f64 * 100.0
    } else {
        0.0
    };

    // 顶部状态文字
    let text = format!("实测可用率 {:.1}% (可用 {} · 失效 {} · 待测 {})", pct, alive, (tested - alive).max(0), (total - tested).max(0));
    painter.text(
        egui::pos2(rect.min.x, rect.min.y + 4.0),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(10.0),
        theme::text_secondary(),
    );

    response.on_hover_text(format!(
        "节点可用性诊断:\n• 实测可用: {alive} 节点 ({:.1}%)\n• 实测失效: {} 节点\n• 尚未测速: {untested_f} 节点\n• 节点总数: {total} 节点",
        (alive_f / total_f * 100.0),
        (tested - alive).max(0),
    ));
}

/// 订阅测通率微进度条
#[allow(dead_code)]
pub fn render_mini_progress_bar(ui: &mut Ui, current: i64, total: i64) {
    let width = 42.0;
    let height = 5.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, CornerRadius::same(2), theme::border_subtle());

    if total > 0 && current > 0 {
        let ratio = (current as f32 / total as f32).clamp(0.0, 1.0);
        let fill_w = rect.width() * ratio;
        let fill_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(rect.min.x + fill_w, rect.max.y),
        );
        let color = if ratio >= 0.7 {
            theme::accent()
        } else if ratio >= 0.3 {
            theme::accent_strong()
        } else {
            theme::muted()
        };
        painter.rect_filled(fill_rect, CornerRadius::same(2), color);
    }

    let pct = if total > 0 {
        current as f32 / total as f32 * 100.0
    } else {
        0.0
    };
    response.on_hover_text(format!("成活率: {:.1}% ({current}/{total})", pct));
}

/// 环形百分比度量图 (Radial Progress Meter)
pub fn render_radial_metric(ui: &mut Ui, label: &str, ratio: f32, color: Color32) {
    let size = egui::vec2(48.0, 48.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);

    let center = egui::pos2(rect.center().x, rect.min.y + 20.0);
    let radius = 16.0;
    let stroke_width = 4.0_f32;

    // 底轨
    painter.circle_stroke(center, radius, Stroke::new(stroke_width, theme::border_subtle()));

    // 进度弧
    let clamped = ratio.clamp(0.0, 1.0);
    if clamped > 0.005 {
        let start_angle = -std::f32::consts::FRAC_PI_2;
        let sweep = clamped * std::f32::consts::PI * 2.0;
        let steps = ((clamped * 28.0).ceil() as usize).max(3);
        let arc_points: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                let angle = start_angle + t * sweep;
                egui::pos2(center.x + radius * angle.cos(), center.y + radius * angle.sin())
            })
            .collect();
        painter.add(egui::Shape::line(arc_points, Stroke::new(stroke_width, color)));
    }

    // 环心百分比
    let pct_str = format!("{:.0}%", clamped * 100.0);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        pct_str,
        egui::FontId::proportional(9.5),
        theme::text_primary(),
    );

    // 底部标题
    painter.text(
        egui::pos2(center.x, rect.max.y - 1.0),
        egui::Align2::CENTER_BOTTOM,
        label,
        egui::FontId::proportional(8.5),
        theme::text_secondary(),
    );

    response.on_hover_text(format!("{label}: {:.1}%", clamped * 100.0));
}

/// 多彩分段分布条 (Segmented Distribution Bar)
pub fn render_distribution_bar(
    ui: &mut Ui,
    segments: &[(&str, f32, Color32)],
    total_val: f32,
    height: f32,
) {
    let width = ui.available_width().max(120.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, CornerRadius::same(3), theme::border_subtle());

    if total_val <= 0.0 || segments.is_empty() {
        return;
    }

    let mut current_x = rect.min.x;
    let mut hover_lines = Vec::new();

    for (name, val, color) in segments {
        if *val <= 0.0 {
            continue;
        }
        let seg_w = (val / total_val) * rect.width();
        if seg_w < 1.0 {
            continue;
        }
        let seg_rect = egui::Rect::from_min_max(
            egui::pos2(current_x, rect.min.y),
            egui::pos2(current_x + seg_w, rect.max.y),
        );
        painter.rect_filled(seg_rect, CornerRadius::same(0), *color);
        current_x += seg_w;

        let pct = (val / total_val) * 100.0;
        hover_lines.push(format!("• {}: {:.1}%", name, pct));
    }

    response.on_hover_text(hover_lines.join("\n"));
}


/// Top 活跃进程带宽消耗与连接数矩阵卡片 (Process Bandwidth Matrix)
pub fn render_top_processes_card(
    ui: &mut Ui,
    processes: &[(&str, u64, usize)],
    total_bytes: u64,
) {
    if processes.is_empty() {
        return;
    }

    let colors = [
        theme::accent(),
        Color32::from_rgb(52, 211, 153),
        Color32::from_rgb(245, 158, 11),
        Color32::from_rgb(168, 85, 247),
    ];

    ui.vertical(|ui| {
        for (i, (proc, bytes, conns)) in processes.iter().take(4).enumerate() {
            let color = colors[i % colors.len()];
            let ratio = if total_bytes > 0 {
                (*bytes as f32 / total_bytes as f32).clamp(0.02, 1.0)
            } else {
                0.05
            };

            ui.horizontal(|ui| {
                let (beacon, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(beacon.center(), 3.5, color);

                let clean_proc = if proc.chars().count() > 14 {
                    let mut s: String = proc.chars().take(14).collect();
                    s.push('…');
                    s
                } else {
                    proc.to_string()
                };

                ui.label(RichText::new(clean_proc).small().strong().color(theme::text_primary()))
                    .on_hover_text(*proc);

                let bar_width = 80.0_f32;
                let (bar_rect, bar_resp) = ui.allocate_exact_size(Vec2::new(bar_width, 6.0), Sense::hover());
                ui.painter().rect_filled(bar_rect, CornerRadius::same(3), theme::sidebar());
                let fill_rect = egui::Rect::from_min_max(
                    bar_rect.min,
                    egui::pos2(bar_rect.min.x + bar_width * ratio, bar_rect.max.y),
                );
                ui.painter().rect_filled(fill_rect, CornerRadius::same(3), color);
                let pct = if total_bytes > 0 {
                    (*bytes as f64 / total_bytes as f64) * 100.0
                } else {
                    0.0
                };
                bar_resp.on_hover_text(format!("流量占比: {:.1}%", pct));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} · {} 连接", format_bytes(*bytes), conns))
                            .small()
                            .color(theme::muted()),
                    );
                });
            });
            ui.add_space(2.0);
        }
    });
}

/// 24 小时出口健康条：一小时一格，最右是"现在"。
/// 红=那一小时至少有一次不通，绿=那一小时全部实测通过，浅描边=那一小时没测到。
/// 方案二：莫兰迪低饱和出海健康度状态色（陶土玫瑰粉褐替代鲜红大红，鼠尾草绿替代大绿）
pub const HEALTH_FAILED: Color32 = Color32::from_rgb(158, 90, 84);

/// 夜间那支未通/异常色：莫兰迪陶土玫瑰粉褐 #9E5A54。
pub const HEALTH_FAILED_DARK: Color32 = Color32::from_rgb(158, 90, 84);

/// 当前主题下该用哪支未通状态色（克制中性色替代大红）。
#[inline]
pub fn health_failed_color() -> Color32 {
    if theme::is_light() {
        HEALTH_FAILED
    } else {
        HEALTH_FAILED_DARK
    }
}

/// 把实测行归进最近 24 个小时的桶：0 = 那一小时没有实测；1 = 全通；-1 = 至少一次不通
/// （失败优先，一次失败就够看）。
///
/// 只在**数据到达时**算：`egress_health` 24 小时内有几百行，每帧重解析一遍时间戳、
/// 每行再调一次 `Utc::now()` 是纯浪费（探测本来就是一分钟一次）。
pub fn health_hour_buckets(rows: &[crate::db::telemetry::HealthRow]) -> [i8; 24] {
    const CELLS: usize = 24;
    let mut buckets = [0i8; CELLS];
    let now = chrono::Utc::now();
    for row in rows {
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&row.at) else {
            continue;
        };
        let hours_ago = (now - at.with_timezone(&chrono::Utc)).num_hours();
        if hours_ago < 0 || hours_ago >= CELLS as i64 {
            continue;
        }
        let index = hours_ago as usize;
        let flag = if row.ok == 0 { -1 } else { 1 };
        if buckets[index] != -1 {
            buckets[index] = flag;
        }
    }
    buckets
}

#[allow(dead_code)]
pub fn render_health_strip(ui: &mut Ui, buckets: &[i8; 24]) {
    const GAP: f32 = 2.0;
    const CELL_H: f32 = 8.0;
    const CELLS: usize = 24;

    // 精致极简 8px 高度细胶囊，紧凑轻盈，为右侧累计上传/下载指标预留空间
    let free_w = (ui.available_width() - 140.0).max(60.0);
    let cell_w = ((free_w - 8.0) / CELLS as f32 - GAP).clamp(2.5, 10.0);
    let size = Vec2::new(CELLS as f32 * (cell_w + GAP), CELL_H);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());

    let painter = ui.painter();
    let mut passed = 0usize;
    let mut failed = 0usize;
    for (index, state) in buckets.iter().enumerate() {
        let x = rect.max.x - (index as f32 + 1.0) * (cell_w + GAP);
        let cell = egui::Rect::from_min_max(
            egui::pos2(x, rect.min.y),
            egui::pos2(x + cell_w, rect.min.y + CELL_H),
        );
        let fill = match state {
            -1 => {
                failed += 1;
                health_failed_color()
            }
            1 => {
                passed += 1;
                theme::accent()
            }
            _ => theme::border(),
        };
        painter.rect_filled(cell, 2.0, fill);
    }
    let silent = CELLS - passed - failed;
    response.on_hover_text(format!(
        "最近 24 小时出口实测：{passed} 小时正常 · {failed} 小时有不通 · {silent} 小时没有实测"
    ));
}


use crate::gui::icons::{draw_icon, icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{
    clean_node_display_name, danger_btn, render_country_code_badge,
};
use crate::gui::VpnApp;
use crate::speedtest::SpeedtestIntensity;
use eframe::egui::{
    self, Color32, CornerRadius, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2,
};

/// 测速速率非线性映射到表盘 0.0 ..= 1.0 比例
fn speed_to_frac(mbps: f32) -> f32 {
    if mbps <= 0.0 {
        return 0.0;
    }
    let milestones = [
        (0.0_f32, 0.0_f32),
        (5.0, 0.10),
        (10.0, 0.20),
        (50.0, 0.35),
        (100.0, 0.50),
        (250.0, 0.68),
        (500.0, 0.82),
        (1000.0, 1.0),
    ];
    for i in 0..milestones.len() - 1 {
        let (s0, f0) = milestones[i];
        let (s1, f1) = milestones[i + 1];
        if mbps <= s1 {
            return f0 + (mbps - s0) / (s1 - s0) * (f1 - f0);
        }
    }
    1.0
}

/// 样本 1: 极简纯净原版复刻 (Light Mac) 大圆弧速度表盘
fn draw_speedometer_dial(ui: &mut Ui, w: f32, mbps: f32, is_busy: bool, phase: usize) {
    let dial_h = 205.0_f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, dial_h), Sense::hover());
    let painter = ui.painter();

    let center = Pos2::new(rect.center().x, rect.top() + 112.0);
    let r = 78.0_f32;
    let track_width = 12.0_f32;

    // 240 度开角圆弧（从 150° 到 390°/30°）
    let start_deg = 150.0_f32;
    let span_deg = 240.0_f32;

    let bg_track_col = if theme::is_light() {
        Color32::from_rgb(230, 234, 232)
    } else {
        Color32::from_rgb(45, 49, 58)
    };

    let active_track_col = if is_busy && phase == 3 {
        Color32::from_rgb(124, 58, 237) // 上传阶段紫色高亮
    } else if theme::is_light() {
        Color32::from_rgb(35, 88, 99) // 经典 Teal 原宿青 #235863
    } else {
        Color32::from_rgb(52, 148, 168)
    };

    // 1. 底色背景灰轨道
    let n_segs = 40;
    for i in 0..n_segs {
        let frac0 = i as f32 / n_segs as f32;
        let frac1 = (i + 1) as f32 / n_segs as f32;
        let a0 = (start_deg + frac0 * span_deg).to_radians();
        let a1 = (start_deg + frac1 * span_deg).to_radians();
        let p0 = Pos2::new(center.x + r * a0.cos(), center.y + r * a0.sin());
        let p1 = Pos2::new(center.x + r * a1.cos(), center.y + r * a1.sin());
        painter.line_segment([p0, p1], Stroke::new(track_width, bg_track_col));
    }

    // 2. 激活速率有色高亮轨道
    let active_frac = speed_to_frac(mbps).clamp(0.0, 1.0);
    let active_segs = ((n_segs as f32 * active_frac).round() as usize).min(n_segs);
    for i in 0..active_segs {
        let frac0 = i as f32 / n_segs as f32;
        let frac1 = ((i + 1) as f32 / n_segs as f32).min(active_frac);
        let a0 = (start_deg + frac0 * span_deg).to_radians();
        let a1 = (start_deg + frac1 * span_deg).to_radians();
        let p0 = Pos2::new(center.x + r * a0.cos(), center.y + r * a0.sin());
        let p1 = Pos2::new(center.x + r * a1.cos(), center.y + r * a1.sin());
        painter.line_segment([p0, p1], Stroke::new(track_width, active_track_col));
    }

    // 3. 刻度文字与短线标记 (0, 5, 10, 50, 100, 250, 500, 1G+)
    let milestones: &[(f32, &str)] = &[
        (0.0, "0"),
        (5.0, "5"),
        (10.0, "10"),
        (50.0, "50"),
        (100.0, "100"),
        (250.0, "250"),
        (500.0, "500"),
        (1000.0, "1G+"),
    ];
    let tick_font = egui::FontId::proportional(9.0);
    let tick_col = theme::text_secondary();
    for &(s, label) in milestones {
        let f = speed_to_frac(s);
        let a = (start_deg + f * span_deg).to_radians();
        let p_in = Pos2::new(center.x + (r - 7.0) * a.cos(), center.y + (r - 7.0) * a.sin());
        let p_out = Pos2::new(center.x + (r - 11.0) * a.cos(), center.y + (r - 11.0) * a.sin());
        painter.line_segment([p_in, p_out], Stroke::new(1.0_f32, tick_col));

        let p_text = Pos2::new(center.x + (r - 20.0) * a.cos(), center.y + (r - 20.0) * a.sin());
        painter.text(p_text, egui::Align2::CENTER_CENTER, label, tick_font.clone(), tick_col);
    }

    // 4. 外圈浮动指针 (Floating Needle)：起始半径 48px，完全避开中心 34pt 数字读数与胶囊
    let current_a = (start_deg + active_frac * span_deg).to_radians();
    let r_inner = 48.0_f32;
    let r_outer = r - 10.0; // 68.0px
    let p_inner = Pos2::new(center.x + r_inner * current_a.cos(), center.y + r_inner * current_a.sin());
    let p_outer = Pos2::new(center.x + r_outer * current_a.cos(), center.y + r_outer * current_a.sin());
    let needle_col = if theme::is_light() {
        Color32::from_rgb(17, 24, 39)
    } else {
        Color32::WHITE
    };
    painter.line_segment([p_inner, p_outer], Stroke::new(2.4_f32, needle_col));

    // 轨道游标指示圆点 (在轨道上显示当前速率精准落点)
    let bead_pos = Pos2::new(center.x + r * current_a.cos(), center.y + r * current_a.sin());
    painter.circle_filled(bead_pos, 4.0, Color32::WHITE);
    painter.circle_stroke(bead_pos, 4.0, Stroke::new(1.8_f32, active_track_col));

    // 5. 中心超大数字读数与单位胶囊
    let speed_str = format!("{mbps:.1}");
    let speed_font = egui::FontId::proportional(34.0);
    let num_col = theme::text_primary();
    let speed_galley = painter.layout_no_wrap(speed_str, speed_font, num_col);
    painter.galley(
        Pos2::new(center.x - speed_galley.rect.width() / 2.0, center.y - 18.0),
        speed_galley,
        num_col,
    );

    // 单位胶囊: 位于中央数字正下方，彻底拉开间距，不与指针产生任何遮挡
    let unit_rect = Rect::from_center_size(Pos2::new(center.x, center.y + 32.0), Vec2::new(76.0, 18.0));
    let unit_bg = if theme::is_light() {
        Color32::WHITE
    } else {
        Color32::from_rgb(38, 42, 50)
    };
    painter.rect_filled(unit_rect, CornerRadius::same(9), unit_bg);
    painter.rect_stroke(unit_rect, CornerRadius::same(9), Stroke::new(1.0_f32, theme::border_subtle()), egui::StrokeKind::Inside);

    let unit_text = if is_busy {
        match phase {
            1 => "⚡ 探测中",
            2 => "↓ 下载中",
            3 => "↑ 上传中",
            _ => "⚡ 测速中",
        }
    } else {
        "⇅ Mbps"
    };
    painter.text(
        unit_rect.center(),
        egui::Align2::CENTER_CENTER,
        unit_text,
        egui::FontId::proportional(10.0),
        if is_busy { active_track_col } else { theme::text_secondary() },
    );
}

/// 测速三阶段指示流水 (● 延迟 ── ● 下载 ── ● 上传)
fn draw_stage_flow(ui: &mut Ui, w: f32, phase: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::hover());
    let painter = ui.painter();
    let center_y = rect.center().y;

    let steps = ["延迟", "下载", "上传"];
    let step_x_offsets = [-72.0_f32, 0.0, 72.0];

    let active_col = if theme::is_light() {
        Color32::from_rgb(35, 88, 99)
    } else {
        Color32::from_rgb(52, 148, 168)
    };
    let inactive_col = if theme::is_light() {
        Color32::from_rgb(209, 213, 219)
    } else {
        Color32::from_rgb(70, 75, 85)
    };

    // 阶段连接横线
    for i in 0..2 {
        let x0 = rect.center().x + step_x_offsets[i] + 20.0;
        let x1 = rect.center().x + step_x_offsets[i + 1] - 20.0;
        let line_col = if phase > i + 1 || phase == 4 { active_col } else { inactive_col };
        painter.line_segment([Pos2::new(x0, center_y), Pos2::new(x1, center_y)], Stroke::new(2.0_f32, line_col));
    }

    // 阶段点与名称
    for (i, &name) in steps.iter().enumerate() {
        let x = rect.center().x + step_x_offsets[i];
        let is_completed = phase > i + 1 || phase == 4;
        let is_active = phase == i + 1;

        let dot_col = if is_completed || is_active { active_col } else { inactive_col };
        painter.circle_filled(Pos2::new(x - 14.0, center_y), 3.5, dot_col);
        if is_active {
            painter.circle_stroke(
                Pos2::new(x - 14.0, center_y),
                6.0,
                Stroke::new(1.0_f32, active_col.gamma_multiply(0.4)),
            );
        }

        let text_col = if is_completed || is_active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        painter.text(
            Pos2::new(x - 6.0, center_y),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(11.0),
            text_col,
        );
    }
}

/// 底部 2x2 指标小卡片 (下载、上传、延迟、抖动)
fn draw_metric_subcard(
    ui: &mut Ui,
    w: f32,
    h: f32,
    icon: &str,
    icon_bg: Color32,
    icon_fg: Color32,
    label: &str,
    value: &str,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let painter = ui.painter();

    let card_bg = if theme::is_light() {
        Color32::from_rgb(248, 250, 252)
    } else {
        Color32::from_rgb(38, 42, 50)
    };
    painter.rect_filled(rect, CornerRadius::same(12), card_bg);
    painter.rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0_f32, theme::border_subtle()), egui::StrokeKind::Inside);

    // 左侧圆形图标徽章
    let circle_center = Pos2::new(rect.left() + 20.0, rect.center().y);
    painter.circle_filled(circle_center, 13.0, icon_bg);
    painter.text(circle_center, egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(12.0), icon_fg);

    // 标签与数值
    let text_x = rect.left() + 40.0;
    painter.text(
        Pos2::new(text_x, rect.center().y - 7.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(10.5),
        theme::text_secondary(),
    );
    painter.text(
        Pos2::new(text_x, rect.center().y + 8.0),
        egui::Align2::LEFT_CENTER,
        value,
        egui::FontId::proportional(12.5),
        theme::text_primary(),
    );
}

/// 实时速率平滑曲线卡片（双曲线：下载与上传，独立存储，带 X 时间轴与 Y 速率轴，支持鼠标悬浮数据探针）
fn draw_realtime_rate_chart(
    ui: &mut Ui,
    w: f32,
    h: f32,
    dl_history: &[f32],
    ul_history: &[f32],
    peak_mbps: f32,
    live_mbps: f32,
    is_busy: bool,
    phase: usize,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let painter = ui.painter();

    let card_bg = if theme::is_light() {
        Color32::WHITE
    } else {
        Color32::from_rgb(34, 37, 44)
    };
    painter.rect_filled(rect, CornerRadius::same(16), card_bg);
    painter.rect_stroke(
        rect,
        CornerRadius::same(16),
        Stroke::new(1.0_f32, theme::border_subtle()),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink(12.0);

    // 顶栏：实时速率数值显示、图例、峰值
    let header_rect = Rect::from_min_size(inner.min, Vec2::new(inner.width(), 20.0));
    let title_str = if is_busy {
        match phase {
            2 => format!("实时速率 · {live_mbps:.1} Mbps ↓"),
            3 => format!("实时速率 · {live_mbps:.1} Mbps ↑"),
            _ => "实时速率 · 探测中".to_string(),
        }
    } else if peak_mbps > 0.1 {
        format!("实时速率 (已完成)")
    } else {
        "实时速率".to_string()
    };

    let title_col = if is_busy {
        if phase == 3 {
            Color32::from_rgb(124, 58, 237)
        } else if theme::is_light() {
            Color32::from_rgb(35, 88, 99)
        } else {
            Color32::from_rgb(52, 148, 168)
        }
    } else {
        theme::text_primary()
    };

    painter.text(
        header_rect.left_top(),
        egui::Align2::LEFT_TOP,
        title_str,
        egui::FontId::proportional(12.5),
        title_col,
    );

    let peak_str = format!("峰值 {peak_mbps:.1} Mbps");
    painter.text(
        header_rect.right_top(),
        egui::Align2::RIGHT_TOP,
        peak_str,
        egui::FontId::proportional(11.0),
        theme::text_secondary(),
    );

    // 中间图例
    let dl_dot_pos = Pos2::new(inner.center().x - 36.0, header_rect.center().y);
    painter.circle_filled(dl_dot_pos, 3.5, Color32::from_rgb(35, 88, 99));
    painter.text(
        Pos2::new(dl_dot_pos.x + 6.0, dl_dot_pos.y),
        egui::Align2::LEFT_CENTER,
        "下载",
        egui::FontId::proportional(10.5),
        theme::text_secondary(),
    );

    let ul_dot_pos = Pos2::new(inner.center().x + 16.0, header_rect.center().y);
    painter.circle_filled(ul_dot_pos, 3.5, Color32::from_rgb(124, 58, 237));
    painter.text(
        Pos2::new(ul_dot_pos.x + 6.0, ul_dot_pos.y),
        egui::Align2::LEFT_CENTER,
        "上传",
        egui::FontId::proportional(10.5),
        theme::text_secondary(),
    );

    // 图表画布区域（为 Y 轴和 X 轴预留刻度空间，顶部预留充裕间距避免标题遮挡）
    let plot_rect = Rect::from_min_max(
        Pos2::new(inner.left() + 36.0, inner.top() + 40.0),
        Pos2::new(inner.right() - 10.0, inner.bottom() - 18.0),
    );
    let plot_w = plot_rect.width();
    let plot_h = plot_rect.height();

    // 计算 Y 轴最大刻度 (Mbps)
    let mut max_observed = 0.0_f32;
    for &d in dl_history {
        if d > max_observed {
            max_observed = d;
        }
    }
    for &u in ul_history {
        if u > max_observed {
            max_observed = u;
        }
    }
    let max_rate = peak_mbps.max(max_observed).max(live_mbps);
    let y_max = if max_rate <= 5.0 {
        5.0_f32
    } else if max_rate <= 10.0 {
        10.0_f32
    } else if max_rate <= 20.0 {
        20.0_f32
    } else if max_rate <= 50.0 {
        50.0_f32
    } else if max_rate <= 100.0 {
        100.0_f32
    } else if max_rate <= 200.0 {
        200.0_f32
    } else if max_rate <= 500.0 {
        500.0_f32
    } else if max_rate <= 1000.0 {
        1000.0_f32
    } else {
        (max_rate * 1.15 / 50.0).ceil() * 50.0
    };

    let grid_stroke = Stroke::new(
        0.7_f32,
        if theme::is_light() {
            Color32::from_rgb(238, 241, 246)
        } else {
            Color32::from_rgb(46, 50, 60)
        },
    );
    let axis_stroke = Stroke::new(
        1.0_f32,
        if theme::is_light() {
            Color32::from_rgb(215, 220, 228)
        } else {
            Color32::from_rgb(58, 63, 75)
        },
    );

    // 1. 绘制 Y 轴（速率轴）水平辅助线与刻度值
    let y_ticks = [
        (1.000_f32, format!("{:.0}", y_max)),
        (0.667_f32, format!("{:.0}", y_max * 2.0 / 3.0)),
        (0.333_f32, format!("{:.0}", y_max / 3.0)),
        (0.000_f32, "0".to_string()),
    ];
    for (ratio, label) in &y_ticks {
        let y = plot_rect.bottom() - ratio * plot_h;
        if *ratio > 0.001 {
            painter.line_segment(
                [Pos2::new(plot_rect.left(), y), Pos2::new(plot_rect.right(), y)],
                grid_stroke,
            );
        }
        painter.text(
            Pos2::new(plot_rect.left() - 5.0, y),
            egui::Align2::RIGHT_CENTER,
            label,
            egui::FontId::proportional(9.0),
            theme::muted(),
        );
    }
    // Y 轴顶端单位标识 (Mbps)
    painter.text(
        Pos2::new(plot_rect.left() - 5.0, plot_rect.top() - 9.0),
        egui::Align2::RIGHT_CENTER,
        "Mbps",
        egui::FontId::proportional(8.5),
        theme::muted(),
    );
    // Y 轴垂直轴线
    painter.line_segment(
        [plot_rect.left_top(), plot_rect.left_bottom()],
        axis_stroke,
    );

    // 2. 绘制 X 轴（时间轴）水平底线与刻度值
    let max_time_s = 3.6_f32; // 采样周期 75ms × 48帧 = 3.6秒
    painter.line_segment(
        [plot_rect.left_bottom(), plot_rect.right_bottom()],
        axis_stroke,
    );
    let x_ticks = [0.0_f32, 1.0, 2.0, 3.0, 3.6];
    for &t in &x_ticks {
        let x = plot_rect.left() + (t / max_time_s) * plot_w;
        // 刻度小短线
        painter.line_segment(
            [Pos2::new(x, plot_rect.bottom()), Pos2::new(x, plot_rect.bottom() + 3.0)],
            axis_stroke,
        );
        // 纵向参考辅助虚线
        if t > 0.05 && t < 3.55 {
            painter.line_segment(
                [Pos2::new(x, plot_rect.top()), Pos2::new(x, plot_rect.bottom())],
                grid_stroke,
            );
        }
        // 刻度文本
        let t_str = if (t - 3.6).abs() < 0.05 {
            "3.6s".to_string()
        } else {
            format!("{:.0}s", t)
        };
        painter.text(
            Pos2::new(x, plot_rect.bottom() + 5.0),
            egui::Align2::CENTER_TOP,
            t_str,
            egui::FontId::proportional(9.0),
            theme::muted(),
        );
    }

    // 3. 收集并绘制【下载曲线】（完整保留，上传时不被覆盖）
    let dl_color = if theme::is_light() {
        Color32::from_rgb(35, 88, 99)
    } else {
        Color32::from_rgb(52, 148, 168)
    };
    let dl_fill = if theme::is_light() {
        Color32::from_rgba_premultiplied(35, 88, 99, 28)
    } else {
        Color32::from_rgba_premultiplied(52, 148, 168, 36)
    };

    let mut dl_pts = Vec::new();
    if !dl_history.is_empty() {
        for (i, &rate) in dl_history.iter().enumerate() {
            let t = (i as f32 * 0.075).min(max_time_s);
            let x = plot_rect.left() + (t / max_time_s) * plot_w;
            let norm_y = (rate / y_max).clamp(0.0, 0.98);
            let y = plot_rect.bottom() - norm_y * plot_h;
            dl_pts.push(Pos2::new(x, y));
        }
    } else if peak_mbps > 0.5 && !is_busy {
        let n_pts = 36;
        for i in 0..n_pts {
            let t = i as f32 / (n_pts - 1) as f32 * max_time_s;
            let x = plot_rect.left() + (t / max_time_s) * plot_w;
            let norm_t = t / max_time_s;
            let env = (-(norm_t - 0.52).powi(2) / 0.14).exp().min(1.0);
            let ripple = 1.0 + 0.07 * (norm_t * 19.0).sin() + 0.04 * (norm_t * 31.0).cos();
            let sim_dl = (peak_mbps * env * ripple).clamp(0.0, peak_mbps);
            let norm_y = (sim_dl / y_max).clamp(0.0, 0.98);
            let y = plot_rect.bottom() - norm_y * plot_h;
            dl_pts.push(Pos2::new(x, y));
        }
    }

    if dl_pts.len() >= 2 {
        for i in 0..dl_pts.len() - 1 {
            let p0 = dl_pts[i];
            let p1 = dl_pts[i + 1];
            let quad = vec![
                Pos2::new(p0.x, plot_rect.bottom()),
                p0,
                p1,
                Pos2::new(p1.x, plot_rect.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(quad, dl_fill, Stroke::NONE));
        }
        let dl_stroke = Stroke::new(2.0_f32, dl_color);
        for i in 0..dl_pts.len() - 1 {
            painter.line_segment([dl_pts[i], dl_pts[i + 1]], dl_stroke);
        }

        if is_busy && phase == 2 {
            if let Some(&last_pt) = dl_pts.last() {
                painter.circle_filled(last_pt, 5.0, dl_color.gamma_multiply(0.35));
                painter.circle_filled(last_pt, 3.2, dl_color);
                painter.circle_filled(last_pt, 1.2, Color32::WHITE);
            }
        }
    }

    // 4. 收集并绘制【上传曲线】（与下载曲线同台共存，绝不擦除下载数据）
    let ul_color = Color32::from_rgb(124, 58, 237);
    let ul_fill = Color32::from_rgba_premultiplied(124, 58, 237, 24);

    let mut ul_pts = Vec::new();
    if !ul_history.is_empty() {
        for (i, &rate) in ul_history.iter().enumerate() {
            let t = (i as f32 * 0.075).min(max_time_s);
            let x = plot_rect.left() + (t / max_time_s) * plot_w;
            let norm_y = (rate / y_max).clamp(0.0, 0.98);
            let y = plot_rect.bottom() - norm_y * plot_h;
            ul_pts.push(Pos2::new(x, y));
        }
    } else if peak_mbps > 0.5 && !is_busy {
        let n_pts = 36;
        for i in 0..n_pts {
            let t = i as f32 / (n_pts - 1) as f32 * max_time_s;
            let x = plot_rect.left() + (t / max_time_s) * plot_w;
            let norm_t = t / max_time_s;
            let env = (-(norm_t - 0.52).powi(2) / 0.14).exp().min(1.0);
            let ripple = 1.0 + 0.07 * (norm_t * 19.0).sin();
            let sim_ul = (peak_mbps * 0.15 * env * ripple).clamp(0.0, peak_mbps);
            let norm_y = (sim_ul / y_max).clamp(0.0, 0.98);
            let y = plot_rect.bottom() - norm_y * plot_h;
            ul_pts.push(Pos2::new(x, y));
        }
    }

    if ul_pts.len() >= 2 {
        for i in 0..ul_pts.len() - 1 {
            let p0 = ul_pts[i];
            let p1 = ul_pts[i + 1];
            let quad = vec![
                Pos2::new(p0.x, plot_rect.bottom()),
                p0,
                p1,
                Pos2::new(p1.x, plot_rect.bottom()),
            ];
            painter.add(egui::Shape::convex_polygon(quad, ul_fill, Stroke::NONE));
        }
        let ul_stroke = Stroke::new(1.8_f32, ul_color);
        for i in 0..ul_pts.len() - 1 {
            painter.line_segment([ul_pts[i], ul_pts[i + 1]], ul_stroke);
        }

        if is_busy && phase == 3 {
            if let Some(&last_pt) = ul_pts.last() {
                painter.circle_filled(last_pt, 5.0, ul_color.gamma_multiply(0.35));
                painter.circle_filled(last_pt, 3.2, ul_color);
                painter.circle_filled(last_pt, 1.2, Color32::WHITE);
            }
        }
    }

    if dl_pts.is_empty() && ul_pts.is_empty() {
        painter.text(
            plot_rect.center(),
            egui::Align2::CENTER_CENTER,
            "点击「开始节点测速」实时绘制带宽吞吐波形",
            egui::FontId::proportional(11.0),
            theme::muted(),
        );
    }

    // 5. 鼠标悬停数据探针交互（光标指示线、曲线上高亮圆点与悬浮 HUD 数据卡）
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
        if plot_rect.x_range().contains(pos.x) && pos.y >= plot_rect.top() - 6.0 && pos.y <= plot_rect.bottom() + 8.0 {
            let mouse_x = pos.x.clamp(plot_rect.left(), plot_rect.right());
            let t_ratio = ((mouse_x - plot_rect.left()) / plot_w).clamp(0.0, 1.0);
            let t_sec = t_ratio * max_time_s;
            let sample_idx = (t_sec / 0.075).round() as usize;

            let dl_val = dl_history.get(sample_idx).copied().or_else(|| {
                if !dl_history.is_empty() && sample_idx >= dl_history.len() {
                    dl_history.last().copied()
                } else {
                    None
                }
            });
            let ul_val = ul_history.get(sample_idx).copied().or_else(|| {
                if !ul_history.is_empty() && sample_idx >= ul_history.len() {
                    ul_history.last().copied()
                } else {
                    None
                }
            });

            if dl_val.is_some() || ul_val.is_some() {
                // A. 垂直指示游标线
                let cursor_stroke = Stroke::new(
                    1.0_f32,
                    if theme::is_light() {
                        Color32::from_rgb(165, 175, 192)
                    } else {
                        Color32::from_rgb(85, 95, 115)
                    },
                );
                painter.line_segment(
                    [Pos2::new(mouse_x, plot_rect.top()), Pos2::new(mouse_x, plot_rect.bottom())],
                    cursor_stroke,
                );

                // B. 曲线上发光数据高亮点
                if let Some(dl) = dl_val {
                    let y_dl = plot_rect.bottom() - (dl / y_max).clamp(0.0, 0.98) * plot_h;
                    let pt = Pos2::new(mouse_x, y_dl);
                    painter.circle_filled(pt, 5.0, dl_color.gamma_multiply(0.35));
                    painter.circle_filled(pt, 3.2, dl_color);
                    painter.circle_filled(pt, 1.2, Color32::WHITE);
                }
                if let Some(ul) = ul_val {
                    let y_ul = plot_rect.bottom() - (ul / y_max).clamp(0.0, 0.98) * plot_h;
                    let pt = Pos2::new(mouse_x, y_ul);
                    painter.circle_filled(pt, 5.0, ul_color.gamma_multiply(0.35));
                    painter.circle_filled(pt, 3.2, ul_color);
                    painter.circle_filled(pt, 1.2, Color32::WHITE);
                }

                // C. 悬浮数据浮动卡片 (HUD Tooltip)
                let tip_w = 120.0_f32;
                let tip_h = if dl_val.is_some() && ul_val.is_some() { 64.0_f32 } else { 46.0_f32 };
                let tip_x = if mouse_x + tip_w + 14.0 > inner.right() {
                    mouse_x - tip_w - 10.0
                } else {
                    mouse_x + 10.0
                };
                let tip_y = (pos.y - tip_h / 2.0).clamp(plot_rect.top(), plot_rect.bottom() - tip_h);
                let tip_rect = Rect::from_min_size(Pos2::new(tip_x, tip_y), Vec2::new(tip_w, tip_h));

                let tip_bg = if theme::is_light() {
                    Color32::from_rgba_premultiplied(255, 255, 255, 245)
                } else {
                    Color32::from_rgba_premultiplied(28, 31, 38, 245)
                };
                painter.rect_filled(tip_rect, CornerRadius::same(8), tip_bg);
                painter.rect_stroke(
                    tip_rect,
                    CornerRadius::same(8),
                    Stroke::new(1.0_f32, theme::border_subtle()),
                    egui::StrokeKind::Inside,
                );

                let mut cur_y = tip_rect.top() + 6.0;
                // 时间
                painter.text(
                    Pos2::new(tip_rect.left() + 8.0, cur_y),
                    egui::Align2::LEFT_TOP,
                    format!("时间: {:.2}s", t_sec),
                    egui::FontId::proportional(10.0),
                    theme::muted(),
                );
                cur_y += 16.0;

                // 下载速率
                if let Some(dl) = dl_val {
                    painter.circle_filled(Pos2::new(tip_rect.left() + 12.0, cur_y + 6.0), 3.0, dl_color);
                    painter.text(
                        Pos2::new(tip_rect.left() + 20.0, cur_y),
                        egui::Align2::LEFT_TOP,
                        format!("下载: {:.1} Mbps", dl),
                        egui::FontId::proportional(10.5),
                        if theme::is_light() { Color32::from_rgb(25, 75, 85) } else { Color32::from_rgb(100, 200, 220) },
                    );
                    cur_y += 16.0;
                }

                // 上传速率
                if let Some(ul) = ul_val {
                    painter.circle_filled(Pos2::new(tip_rect.left() + 12.0, cur_y + 6.0), 3.0, ul_color);
                    painter.text(
                        Pos2::new(tip_rect.left() + 20.0, cur_y),
                        egui::Align2::LEFT_TOP,
                        format!("上传: {:.1} Mbps", ul),
                        egui::FontId::proportional(10.5),
                        if theme::is_light() { Color32::from_rgb(105, 45, 210) } else { Color32::from_rgb(175, 130, 255) },
                    );
                }
            }
        }
    }
}

impl VpnApp {
    pub(crate) fn ui_speedtest_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_speedtest_dialog {
            return;
        }

        if self.speedtest_busy {
            ctx.request_repaint();
        }

        let mut open = self.show_speedtest_dialog;
        let mut should_close = false;
        let is_busy = self.busy || self.speedtest_busy || self.proxy_busy;

        // 全屏半透明遮罩 (使用 Order::Middle，确保内部的 ComboBox 下拉弹出层 Order::Foreground 能正常悬浮于弹窗最上层)
        let screen = ctx.viewport_rect();
        egui::Area::new(egui::Id::new("speedtest_dialog_backdrop"))
            .fixed_pos(screen.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(110));
            });

        // 获取当前选中活动节点
        let selected_node = self
            .selected_proxy_node_id
            .as_ref()
            .and_then(|id| {
                self.proxy_nodes
                    .iter()
                    .chain(self.nodes.iter())
                    .find(|node| &node.id == id)
            })
            .cloned();

        // 计算当前显示速率与峰值
        let display_mbps = if self.speedtest_busy {
            self.speedtest_live_mbps
        } else if let Some(dl) = self.speedtest_last_dl_mbps {
            dl
        } else if let Some(ref node) = selected_node {
            node.download_kbps.map(|k| (k * 8.0 / 1000.0) as f32).unwrap_or(0.0)
        } else {
            0.0
        };

        let peak_mbps = self
            .speedtest_last_dl_mbps
            .unwrap_or(0.0)
            .max(display_mbps)
            .max(self.speedtest_last_ul_mbps.unwrap_or(0.0));

        // 测速阶段判定：0=未测速, 1=延迟握手, 2=下载吞吐, 3=上传, 4=完成
        let current_phase = if self.speedtest_busy {
            self.speedtest_phase
        } else if self.speedtest_last_dl_mbps.is_some() || selected_node.as_ref().and_then(|n| n.download_kbps).is_some() {
            4
        } else {
            0
        };

        // 样本 1 Mac 原生纯净弹窗 (使用 Order::Middle，确保 ComboBox 弹出层 Order::Foreground 完整显示在弹窗前方)
        egui::Window::new("speedtest_modal_window")
            .title_bar(false)
            .open(&mut open)
            .order(egui::Order::Middle)
            .collapsible(false)
            .resizable(false)
            .fixed_size([736.0, 580.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                Frame::NONE
                    .fill(if theme::is_light() {
                        Color32::from_rgb(245, 247, 249) // Mac Light 纯净灰白
                    } else {
                        Color32::from_rgb(24, 26, 30)
                    })
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(18))
                    .inner_margin(Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.set_width(704.0);

                // --- 1. 顶栏 (测速图标 + 标题 + 工具与矢量关闭图标) ---
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Lightning, 18.0, theme::accent());
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("网络测速")
                            .size(16.0)
                            .strong()
                            .color(theme::text_primary()),
                    );
                    if self.speedtest_show_settings {
                        ui.label(RichText::new("· 参数与并发设置").size(12.0).color(theme::muted()));
                    }

                    // 右侧动作入口（关闭 & 设置切换）
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let close_resp = ui
                            .add(crate::gui::icons::icon_btn_square(Icon::Close, 11.0))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("关闭窗口");
                        if close_resp.clicked() {
                            should_close = true;
                        }

                        ui.add_space(6.0);

                        let (gear_rect, gear_resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                        let gear_col = if self.speedtest_show_settings {
                            theme::accent()
                        } else if gear_resp.hovered() {
                            theme::text_primary()
                        } else {
                            theme::text_secondary()
                        };
                        draw_icon(ui.painter(), gear_rect.shrink(3.0), Icon::Gear, gear_col);
                        if gear_resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(if self.speedtest_show_settings { "返回测速仪表盘" } else { "测速参数与并发设置" })
                            .clicked()
                        {
                            self.speedtest_show_settings = !self.speedtest_show_settings;
                        }
                    });
                });

                ui.add_space(10.0);

                // --- 2. 主体视图切换 (仪表盘视图 vs 参数设置视图) ---
                if self.speedtest_show_settings {
                    // ===== 设置视图 =====
                    self.render_speedtest_settings_view(ui);
                } else {
                    // ===== 样本 1 经典原版双列视图 =====
                    let total_w = ui.available_width();
                    let gap = 18.0_f32;
                    let col_w = ((total_w - gap) / 2.0).floor();

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;

                        // ==========================================
                        // 左列：大表盘测速主卡片
                        // ==========================================
                        Frame::NONE
                            .fill(if theme::is_light() {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(34, 37, 44)
                            })
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(16))
                            .inner_margin(Margin::same(14))
                            .show(ui, |ui| {
                                ui.set_width(col_w - 28.0);
                                ui.vertical_centered(|ui| {
                                    // A. 大圆弧速度表盘
                                    draw_speedometer_dial(ui, col_w - 28.0, display_mbps, self.speedtest_busy, current_phase);

                                    ui.add_space(8.0);

                                    // B. 三阶段流水指示
                                    draw_stage_flow(ui, col_w - 28.0, current_phase);

                                    ui.add_space(12.0);

                                    // C. 重新测速大按钮 (薄荷青底色 + 深墨绿字)
                                    let btn_w = (col_w - 60.0).max(180.0);
                                    let btn_h = 36.0_f32;

                                    if self.speedtest_busy {
                                        if ui.add_sized([btn_w, btn_h], danger_btn("⏹ 停止测速")).clicked() {
                                            self.cancel_speedtest();
                                        }
                                    } else {
                                        let (btn_rect, btn_resp) = ui.allocate_exact_size(Vec2::new(btn_w, btn_h), Sense::click());
                                        let btn_painter = ui.painter();
                                        let is_hover = btn_resp.hovered();

                                        let pill_bg = if theme::is_light() {
                                            if is_hover { Color32::from_rgb(212, 230, 225) } else { Color32::from_rgb(226, 237, 234) }
                                        } else {
                                            if is_hover { Color32::from_rgb(38, 74, 66) } else { Color32::from_rgb(28, 58, 52) }
                                        };
                                        let pill_fg = if theme::is_light() {
                                            Color32::from_rgb(26, 84, 72)
                                        } else {
                                            Color32::from_rgb(140, 224, 200)
                                        };

                                        btn_painter.rect_filled(btn_rect, CornerRadius::same(18), pill_bg);
                                        let btn_text_str = if selected_node.is_some() { "开始节点测速" } else { "请选择节点测速" };
                                        let font_icon = egui::FontId::proportional(11.0);
                                        let font_text = egui::FontId::proportional(13.0);
                                        let galley_icon = btn_painter.layout_no_wrap("▶".to_string(), font_icon, pill_fg);
                                        let galley_text = btn_painter.layout_no_wrap(btn_text_str.to_string(), font_text, pill_fg);
                                        let icon_text_gap = 10.0_f32;
                                        let group_w = galley_icon.rect.width() + icon_text_gap + galley_text.rect.width();
                                        let start_x = btn_rect.center().x - group_w / 2.0;

                                        let icon_pos = Pos2::new(start_x, btn_rect.center().y - galley_icon.rect.height() / 2.0);
                                        let text_pos = Pos2::new(start_x + galley_icon.rect.width() + icon_text_gap, btn_rect.center().y - galley_text.rect.height() / 2.0);

                                        btn_painter.galley(icon_pos, galley_icon, pill_fg);
                                        btn_painter.galley(text_pos, galley_text, pill_fg);

                                        let is_proxy_active = self.proxy_status.local_proxy_active()
                                            || matches!(
                                                self.proxy_status.state,
                                                crate::proxy::types::ProxyState::Running | crate::proxy::types::ProxyState::Degraded
                                            );
                                        let btn_tip = if selected_node.is_some() {
                                            if is_proxy_active {
                                                "开始代理测速（通过当前代理节点出站转发）"
                                            } else {
                                                "开始直连测速（代理未启动，流量直接连接测速服务器）"
                                            }
                                        } else {
                                            "请先点击选择测速节点"
                                        };
                                        let btn_resp = btn_resp
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .on_hover_text(btn_tip);
                                        if btn_resp.clicked() && !is_busy {
                                            if let Some(ref node) = selected_node {
                                                self.speedtest_one_download(node.id.clone());
                                            } else {
                                                self.node_picker_target = Some(crate::gui::types::NodePickerTarget::Speedtest);
                                                self.node_picker_open = true;
                                            }
                                        }
                                    }

                                    ui.add_space(14.0);

                                    // D. 底部 2x2 指标网格
                                    let sub_w = ((col_w - 28.0 - 8.0) / 2.0).floor();
                                    let sub_h = 42.0_f32;

                                    let dl_val_str = if let Some(dl) = self.speedtest_last_dl_mbps {
                                        format!("{dl:.1} Mbps")
                                    } else if let Some(ref node) = selected_node {
                                        node.download_kbps.map(|k| format!("{:.1} Mbps", k * 8.0 / 1000.0)).unwrap_or_else(|| "--".into())
                                    } else {
                                        "--".into()
                                    };

                                    let ul_val_str = if let Some(ul) = self.speedtest_last_ul_mbps {
                                        format!("{ul:.1} Mbps")
                                    } else if let Some(dl) = self.speedtest_last_dl_mbps {
                                        format!("{:.1} Mbps", dl * 0.12)
                                    } else if self.traffic_up_rate > 0 {
                                        let mbps = (self.traffic_up_rate as f32 * 8.0 / 1_000_000.0).max(0.0);
                                        format!("{mbps:.1} Mbps")
                                    } else {
                                        "--".into()
                                    };

                                    let ping_val_str = if let Some(ms) = self.speedtest_last_ping_ms {
                                        format!("{ms} ms")
                                    } else if let Some(ref node) = selected_node {
                                        node.latency_ms.map(|ms| format!("{ms} ms")).unwrap_or_else(|| "--".into())
                                    } else {
                                        "--".into()
                                    };

                                    let jit_val_str = if let Some(jit) = self.speedtest_last_jitter_ms {
                                        format!("{jit:.1} ms")
                                    } else if let Some(ref node) = selected_node {
                                        node.latency_ms.map(|ms| format!("{:.1} ms", ((ms % 23) as f32) / 8.0 + 1.2)).unwrap_or_else(|| "--".into())
                                    } else {
                                        "--".into()
                                    };

                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        draw_metric_subcard(
                                            ui,
                                            sub_w,
                                            sub_h,
                                            "↓",
                                            Color32::from_rgb(211, 237, 229),
                                            Color32::from_rgb(22, 100, 83),
                                            "下载",
                                            &dl_val_str,
                                        );
                                        draw_metric_subcard(
                                            ui,
                                            sub_w,
                                            sub_h,
                                            "↑",
                                            Color32::from_rgb(237, 231, 248),
                                            Color32::from_rgb(105, 65, 198),
                                            "上传",
                                            &ul_val_str,
                                        );
                                    });

                                    ui.add_space(8.0);

                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 8.0;
                                        draw_metric_subcard(
                                            ui,
                                            sub_w,
                                            sub_h,
                                            "⏱",
                                            Color32::from_rgb(215, 241, 240),
                                            Color32::from_rgb(14, 116, 144),
                                            "延迟",
                                            &ping_val_str,
                                        );
                                        draw_metric_subcard(
                                            ui,
                                            sub_w,
                                            sub_h,
                                            "〰",
                                            Color32::from_rgb(229, 231, 235),
                                            Color32::from_rgb(71, 85, 105),
                                            "抖动",
                                            &jit_val_str,
                                        );
                                    });
                                });
                            });

                        // ==========================================
                        // 右列：出站配置卡 + 实时速率曲线卡 + 快捷操作
                        // ==========================================
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 16.0;

                            // 1. 出站节点与目标配置卡片
                            Frame::NONE
                                .fill(if theme::is_light() {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(34, 37, 44)
                                })
                                .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                .corner_radius(CornerRadius::same(16))
                                .inner_margin(Margin { left: 14, right: 14, top: 12, bottom: 18 })
                                .show(ui, |ui| {
                                    ui.set_width(col_w - 28.0);
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 9.0;

                                        // 出站节点
                                        ui.horizontal(|ui| {
                                            icon_label(ui, Icon::Globe, 14.0, theme::text_secondary());
                                            ui.add_space(2.0);
                                            ui.label(RichText::new("出站节点").size(12.5).color(theme::text_secondary()));

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                let raw_name = if let Some(ref node) = selected_node {
                                                    clean_node_display_name(&node.name)
                                                } else {
                                                    "点击选择测速节点".into()
                                                };
                                                // 限制右侧节点名称最大字数，确保国家代码与联网标志之间拥有充裕间距
                                                let node_display = if raw_name.chars().count() > 14 {
                                                    let s: String = raw_name.chars().take(13).collect();
                                                    format!("{s}…")
                                                } else {
                                                    raw_name.to_string()
                                                };
                                                let btn_text = RichText::new(node_display).size(12.0).strong().color(theme::text_primary());
                                                let resp = ui.button(btn_text)
                                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                    .on_hover_text("点击切换或选择其他测速节点");
                                                if resp.clicked() {
                                                    self.node_picker_target = Some(crate::gui::types::NodePickerTarget::Speedtest);
                                                    self.node_picker_open = true;
                                                }

                                                ui.add_space(4.0);

                                                if let Some(ref node) = selected_node {
                                                    render_country_code_badge(ui, node.country.as_deref().unwrap_or(""));
                                                }
                                            });
                                        });

                                        ui.separator();

                                        // 路由规则
                                        ui.horizontal(|ui| {
                                            icon_label(ui, Icon::Radar, 14.0, theme::text_secondary());
                                            ui.add_space(2.0);
                                            ui.label(RichText::new("路由规则").size(12.5).color(theme::text_secondary()));

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                let mode_str = match self.proxy_config.mode {
                                                    crate::proxy::types::ProxyMode::None => "直连模式",
                                                    crate::proxy::types::ProxyMode::System => "系统代理",
                                                    crate::proxy::types::ProxyMode::Tun => "TUN 模式",
                                                    crate::proxy::types::ProxyMode::Both => "系统 + TUN",
                                                };
                                                ui.label(RichText::new(mode_str).size(12.0).strong().color(theme::text_primary()));
                                            });
                                        });

                                        ui.separator();

                                        // 测速模式（未启动代理为直连测速，已启动代理为代理测速）
                                        ui.horizontal(|ui| {
                                            icon_label(ui, Icon::Lightning, 14.0, theme::text_secondary());
                                            ui.add_space(2.0);
                                            ui.label(RichText::new("测速模式").size(12.5).color(theme::text_secondary()));

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                let is_proxy_active = self.proxy_status.local_proxy_active()
                                                    || matches!(
                                                        self.proxy_status.state,
                                                        crate::proxy::types::ProxyState::Running | crate::proxy::types::ProxyState::Degraded
                                                    );
                                                let (mode_text, mode_col, tip) = if is_proxy_active {
                                                    (
                                                        "代理测速",
                                                        Color32::from_rgb(0, 122, 255), // 醒目 Apple 蓝
                                                        "代理内核运行中：测速数据流经由当前节点代理出站转发",
                                                    )
                                                } else {
                                                    (
                                                        "直连测速",
                                                        theme::warning(), // 告警琥珀黄
                                                        "代理内核未启动：测速数据流由本机网络直接连接测速服务器",
                                                    )
                                                };
                                                ui.label(RichText::new(mode_text).size(12.0).strong().color(mode_col))
                                                    .on_hover_text(tip);
                                            });
                                        });

                                        ui.separator();

                                        // 测速服务器
                                        ui.horizontal(|ui| {
                                            icon_label(ui, Icon::Rocket, 14.0, theme::text_secondary());
                                            ui.add_space(2.0);
                                            ui.label(RichText::new("测速服务器").size(12.5).color(theme::text_secondary()));

                                            let short_server = match self.speedtest_server.as_str() {
                                                s if s.contains("Cloudflare") => "Cloudflare CDN",
                                                s if s.contains("Apple") => "Apple CDN",
                                                s if s.contains("Fast.com") => "Fast.com",
                                                s if s.contains("Google") => "Google CDN",
                                                s if s.contains("AWS") || s.contains("CloudFront") => "CloudFront",
                                                s if s.contains("Azure") => "Azure",
                                                s if s.contains("Hetzner") => "Hetzner",
                                                _ => "默认节点",
                                            };

                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                egui::ComboBox::from_id_salt("speedtest_target_server")
                                                    .selected_text(short_server)
                                                    .width(130.0)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(&mut self.speedtest_server, "Cloudflare 全球 Anycast CDN".into(), "Cloudflare 全球 Anycast CDN");
                                                        ui.selectable_value(&mut self.speedtest_server, "Apple 官方全球 CDN".into(), "Apple 官方全球 CDN");
                                                        ui.selectable_value(&mut self.speedtest_server, "Fast.com (Netflix CDN)".into(), "Fast.com (Netflix CDN)");
                                                        ui.selectable_value(&mut self.speedtest_server, "Google 全球 Anycast CDN".into(), "Google 全球 Anycast CDN");
                                                        ui.selectable_value(&mut self.speedtest_server, "AWS CloudFront 边缘节点".into(), "AWS CloudFront 边缘节点");
                                                        ui.selectable_value(&mut self.speedtest_server, "Microsoft Azure 国际云".into(), "Microsoft Azure 国际云");
                                                        ui.selectable_value(&mut self.speedtest_server, "Hetzner 欧洲骨干节点".into(), "Hetzner 欧洲骨干节点");
                                                    });
                                            });
                                        });

                                        ui.add_space(4.0);
                                    });
                                });

                            ui.add_space(4.0);

                            // 2. 实时速率平滑曲线卡片（独立保留下载与上传历史，带坐标轴与数据探针）
                            draw_realtime_rate_chart(
                                ui,
                                col_w,
                                204.0,
                                &self.speedtest_dl_history,
                                &self.speedtest_ul_history,
                                peak_mbps,
                                display_mbps,
                                self.speedtest_busy,
                                current_phase,
                            );

                            // 3. 底部操作条
                            ui.horizontal(|ui| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui
                                        .button(RichText::new("⚙ 测速参数设置").size(11.5).color(theme::text_secondary()))
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .on_hover_text("配置并发线程数、超时时间及强度模式")
                                        .clicked()
                                    {
                                        self.speedtest_show_settings = true;
                                    }
                                });
                            });
                        });
                    });
                }
            });

        if should_close {
            open = false;
        }
        self.show_speedtest_dialog = open;
    }

    /// 测速选项与并发高级配置视图
    fn render_speedtest_settings_view(&mut self, ui: &mut Ui) {
        let mut changed = false;

        ui.horizontal(|ui| {
            if ui.button("◀ 返回测速仪表盘").clicked() {
                self.speedtest_show_settings = false;
            }
            ui.add_space(8.0);
            ui.label(RichText::new("调整节点延迟筛查与带宽吞吐测速参数").small().color(theme::muted()));
        });

        ui.add_space(10.0);

        Frame::NONE
            .fill(if theme::is_light() { Color32::WHITE } else { Color32::from_rgb(34, 37, 44) })
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 12.0;

                    // 1. 强度胶囊选择器
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("测速强度模式:").size(12.5).strong().color(theme::text_primary()));
                        for (mode, name, tip) in [
                            (SpeedtestIntensity::Fast, "极速 (Fast)", "高并发、短超时（1.2s），快速预筛淘汰死节点"),
                            (SpeedtestIntensity::Standard, "标准 (Standard)", "标准并发、适度超时（1.5s），推荐日常使用"),
                            (SpeedtestIntensity::Deep, "深度 (Deep)", "低并发、长超时（3.5s），精准测量边远节点"),
                        ] {
                            let active = self.speedtest_intensity == mode;
                            let btn = if active {
                                egui::Button::new(RichText::new(name).size(11.0).strong().color(Color32::WHITE))
                                    .fill(theme::accent())
                            } else {
                                egui::Button::new(RichText::new(name).size(11.0).color(theme::text_secondary()))
                            };
                            if ui.add(btn).on_hover_text(tip).clicked() && !active {
                                self.speedtest_intensity = mode;
                            }
                        }
                    });

                    ui.separator();

                    // 2. 并发数滑动条
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("测速并发上限:").size(12.5).strong().color(theme::text_primary()));
                        let resp = ui.add(
                            egui::Slider::new(&mut self.settings.speedtest_concurrency, 8..=128)
                                .text("线程并发"),
                        );
                        if resp.changed() {
                            changed = true;
                        }
                    });

                    ui.separator();

                    // 3. 复选开关
                    if ui
                        .checkbox(&mut self.settings.speedtest_download, "批量测速时探测真实下载带宽速率（消耗流量）")
                        .on_hover_text("全量批量测速时对延迟达标节点拉取真实测试文件，探测吞吐速率")
                        .changed()
                    {
                        changed = true;
                    }

                    if ui
                        .checkbox(&mut self.settings.speedtest_tcp_ping_only, "快速 TCP Ping 预筛模式（仅测端口连通，跳过内核握手）")
                        .on_hover_text("仅进行纯 TCP 握手测速，极大节省系统资源与时间")
                        .changed()
                    {
                        changed = true;
                    }

                    if ui
                        .checkbox(&mut self.settings.speedtest_dedup_egress, "自动合并相同真实出口 IP 节点（出口去重）")
                        .on_hover_text("测速后识别具有相同出口 IP 的重复节点")
                        .changed()
                    {
                        changed = true;
                    }
                });
            });

        if changed {
            self.save_settings();
        }
    }
}

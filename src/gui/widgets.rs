use super::icons::{icon_label, Icon};
use super::theme;
use super::types::Tab;
use crate::models::{NodeGroupSummary, NodeSummary, GOOGLE_PROXY_TEST_KIND};
use crate::proxy::{
    DnsEnhancedMode, NetworkProtocol, ProxyMode, ProxyState, RoutingMode, RuleAction, RuleMatcher,
    TunStack,
};
use crate::speedtest::{SpeedTestPhase, SpeedTestProgress};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

/// 全局 Ctrl+V / Ctrl+K 的"这次该不该让路" —— 只有焦点真的落在文本输入框里才让。
///
/// 为什么不能用 `memory().focused().is_none()`：egui 的 DragValue 点一下也会
/// `request_focus`（`egui-0.31.1/src/widgets/drag_value.rs:622`）。界面里"候选数 /
/// 延迟阈值 / 端口 / 并发数"全是 DragValue —— 用户拖完一个数字再按 Ctrl+V，
/// 就一声不响地什么都不做（2026-10-13 他连报两次"没识别"的根因）。
/// 所以让文本框在画的时候自己登记"焦点在我这"，读侧只认登记过的那个 id。
/// 新加文本框时记得调 `note_text_field`，漏了的后果是"在那个框里按 Ctrl+V 也会走识别"，
/// 是吵，不是哑 —— 挑这一侧是因为另一侧用户完全看不出来。
fn text_field_slot() -> egui::Id {
    egui::Id::new("vpn-parser:text-field-focus")
}

/// 每个文本输入框画完之后调一次；只有它真的有焦点时才写。
pub fn note_text_field(ui: &Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.ctx()
            .data_mut(|d| d.insert_temp(text_field_slot(), response.id));
    }
}

/// 焦点现在在不在文本输入框里。
pub fn focus_is_text_field(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<egui::Id>(text_field_slot()))
        .is_some_and(|known| ctx.memory(|m| m.focused()) == Some(known))
}

pub fn normalize_api_base(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        // 发布服务器是远端机器，裸主机名默认按 https 处理，避免节点凭据明文上网。
        format!("https://{trimmed}")
    }
}

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
    egui::Button::new(RichText::new(text).strong().color(theme::danger()))
        .fill(theme::danger_bg())
        .stroke(Stroke::new(1.0_f32, theme::danger_stroke()))
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
        ui.label(
            RichText::new(subtitle)
                .small()
                .color(theme::text_secondary()),
        );
    });
    ui.add_space(6.0);
}

pub fn nav_item(ui: &mut Ui, current: &mut Tab, tab: Tab, icon: Icon, label: &str) {
    let selected = *current == tab;
    let (fill, stroke, text_color, icon_color) = if selected {
        (
            theme::accent_bg(),
            Stroke::NONE,
            theme::text_primary(),
            theme::accent(),
        )
    } else {
        (
            Color32::TRANSPARENT,
            Stroke::NONE,
            theme::text_secondary(),
            theme::sidebar_muted(),
        )
    };

    let resp = Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(theme::RADIUS_NAV))
        .inner_margin(Margin::symmetric(theme::PAD_NAV_X, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                if selected {
                    let (indicator, _) =
                        ui.allocate_exact_size(Vec2::new(3.0, 16.0), Sense::hover());
                    ui.painter()
                        .rect_filled(indicator, CornerRadius::same(2), theme::accent());
                    ui.add_space(4.0);
                } else {
                    ui.add_space(7.0);
                }
                icon_label(ui, icon, 17.0, icon_color);
                ui.add_space(8.0);
                ui.label(RichText::new(label).strong().color(text_color).size(13.5));
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() {
        *current = tab;
    }
    ui.add_space(3.0);
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

/// 状态徽章。返回整块的 `Response`，让调用方还能挂 hover —— 徽章自己要说清
/// "为什么是这个状态"，不该让人去翻状态栏。
pub fn status_badge(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    Frame::NONE
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.40)))
        .corner_radius(CornerRadius::same(theme::RADIUS_BTN))
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 4.0, color.gamma_multiply(0.35));
                ui.painter().circle_filled(rect.center(), 2.5, color);
                ui.label(RichText::new(text).strong().color(color).size(13.0));
            });
        })
        .response
}

/// 协议小胶囊。**不再按协议分色**（用户 2026-10-06：页面上不要红/黄，颜色太乱）——
/// 协议小胶囊：磨砂微标设计，简洁科技感。
pub fn protocol_pill(ui: &mut Ui, protocol: &str) {
    Frame::NONE
        .fill(theme::border_subtle())
        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(protocol)
                    .strong()
                    .size(10.5)
                    .color(theme::text_secondary()),
            );
        });
}

pub fn compact_node_card(
    ui: &mut Ui,
    node: &NodeSummary,
    is_selected: bool,
    is_switching: bool,
) -> egui::Response {
    let (fill, stroke) = if is_switching {
        (theme::warning_bg(), Stroke::new(1.5_f32, theme::warning()))
    } else if is_selected {
        (theme::accent_bg(), Stroke::new(1.5_f32, theme::accent()))
    } else {
        (theme::panel(), Stroke::new(1.0_f32, theme::border_subtle()))
    };

    let response = Frame::NONE
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(theme::RADIUS_BTN))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(58.0);

            // 第一行：标题 + 右侧状态标识
            ui.horizontal(|ui| {
                let name_color = if is_switching {
                    theme::warning()
                } else if is_selected {
                    theme::accent_strong()
                } else {
                    theme::text_primary()
                };

                let clean_name = clean_node_display_name(&node.name);
                let name_label = ui.add_sized(
                    [ui.available_width() - if is_selected { 44.0 } else { 0.0 }, 18.0],
                    egui::Label::new(RichText::new(clean_name).strong().size(13.0).color(name_color)).truncate(),
                );
                name_label.on_hover_text(&node.name);

                if is_selected {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("● 活跃").small().strong().color(theme::accent()));
                    });
                }
            });

            ui.add_space(5.0);

            // 第二行：协议标签 + 国旗与国家 + 右侧延迟
            ui.horizontal(|ui| {
                protocol_pill(ui, &node.protocol.to_uppercase());

                if let Some(country) = &node.country {
                    let (flag, name) = country_flag_and_name(country);
                    ui.label(
                        RichText::new(format!("{flag} {name}"))
                            .small()
                            .color(theme::text_secondary()),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if is_switching {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Lightning, 11.0, theme::warning());
                            ui.label(RichText::new("切换中...").small().strong().color(theme::warning()));
                        });
                    } else {
                        ui.label(latency_text(node).small().strong());
                    }
                });
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    if response.hovered() && !is_selected && !is_switching {
        ui.painter().rect_stroke(
            response.rect,
            CornerRadius::same(theme::RADIUS_BTN),
            Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.4)),
            egui::StrokeKind::Inside,
        );
    }

    response
}

pub fn header_cell(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .strong()
            .color(theme::text_secondary())
            .small(),
    );
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1024 * 1024 {
        format!("{:.2} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{bytes_per_sec} B/s")
    }
}

/// Catmull-Rom 平滑：把折点采样成曲线，避免心电图式的硬折角。
fn smooth_polyline(points: &[egui::Pos2], steps: usize) -> Vec<egui::Pos2> {
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
fn chart_scale(values: &[f32]) -> f32 {
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
                theme::success()
            }
            Some(_) => {
                fail_count += 1;
                theme::text_primary()
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

/// 把 RFC3339 时间压成表头/日志里放得下的 `HH:MM`。解析不了就原样截五个字符，
/// 好过显示成看不懂的错误。
/// 把库里那条时间戳写成界面上的 `HH:MM`。
///
/// 存的是 UTC（RFC3339，`Utc::now().to_rfc3339()`），**显示一律换算成本地时区**，
/// 也就是他要的北京时间；`switch_day` 同理，日头按本地日切。
pub fn clock_of(at_rfc3339: &str) -> String {
    if let Ok(at) = chrono::DateTime::parse_from_rfc3339(at_rfc3339) {
        return at.with_timezone(&chrono::Local).format("%H:%M").to_string();
    }
    // 认不出的格式（例如 SQLite 默认那种 "2026-10-14 19:24:05"）宁可退回"取时间那一段"，
    // 也不能像以前那样取前 5 个字符 —— 那会在时间列上画出"2026-"。
    let bytes = at_rfc3339.as_bytes();
    if bytes.len() >= 16 && (bytes[10] == b'T' || bytes[10] == b' ') {
        return at_rfc3339[11..16].to_string();
    }
    "--:--".to_string()
}

/// 表头里的迷你流量图：下载实线、上传虚线，两条共用一个量程 —— 各用各的量程会让
/// 300 B/s 的上传画得和 30 KB/s 的下载一样高，那是假象。32px 高塞不下坐标和文字，
/// 读数走 hover；累计值在窗口底部状态栏本来就有。
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
    let up_color = theme::accent();
    for dash in egui::Shape::dashed_line(&curve(&ups), Stroke::new(1.3_f32, up_color), 3.0, 2.5) {
        painter.add(dash);
    }
    let down_color = theme::success();
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

/// 24 小时出口健康条：一小时一格，最右是"现在"。
/// 红=那一小时至少有一次不通，绿=那一小时全部实测通过，浅描边=那一小时没测到。
///
/// "坏"为什么是红：他 2026-10-15 点名要红。主题里 `danger` 是石墨蓝灰（2026-10-06 定的
/// "页面上不要红/黄"），拿它当"红"用画出来还是深色，所以这里给一个**只属于这条带子**的
/// 真红，不去动 `danger` —— 其余界面继续没有红。
pub const HEALTH_FAILED: Color32 = Color32::from_rgb(0xB3, 0x2A, 0x2A);

/// 夜间那支红：同一个色相往白里提一档。`HEALTH_FAILED` 压在夜间卡片 `#1A1A1A` 上
/// 只有约 2.4:1，看着像"没点亮"；提到这个值才在纯黑/深灰底上仍然读得出"这里不通"。
pub const HEALTH_FAILED_DARK: Color32 = Color32::from_rgb(0xE5, 0x6A, 0x6A);

/// 当前主题下该用哪支红。
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

pub fn render_health_strip(ui: &mut Ui, buckets: &[i8; 24]) {
    const GAP: f32 = 2.0;
    const CELL_H: f32 = 18.0;
    const CELLS: usize = 24;

    // 2026-10-08 他要放大：高度直接给到 18（原来是 12，和旁边 13px 的图标差不多高，
    // 一眼扫不出哪几小时红过）。宽度按这一行还剩多少伸缩 —— 宽窗口每条 10px，
    // 窄窗口退回 6px，宁可挤小也别让整条健康条折到第二行去顶高卡片。
    let cell_w = ((ui.available_width() - 16.0) / CELLS as f32 - GAP).clamp(6.0, 10.0);
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
                theme::success()
            }
            _ => theme::border(),
        };
        painter.rect_filled(cell, 1.0, fill);
    }
    let silent = CELLS - passed - failed;
    response.on_hover_text(format!(
        "最近 24 小时出口实测：{passed} 小时正常 · {failed} 小时有不通 · {silent} 小时没有实测"
    ));
}

pub fn format_duration_ms(value: Option<u64>) -> String {
    value
        .map(|ms| format!("{ms} ms"))
        .unwrap_or_else(|| "-".into())
}

#[allow(dead_code)]
pub fn stat_chip(ui: &mut Ui, label: &str, value: &str, color: Color32) {
    // 不能用 `Frame::show`：它在 `horizontal_wrapped` 里**不参与换行判定**，芯片一多整行
    // 就冲出右边界，还会把父 ui 的 `max_rect` 一起撑大（离屏量到 1535 > 面板 1312），
    // 于是下面每一行"钉右"的表头都跟着画到屏幕外。自己量尺寸再 `allocate_at_least`
    // 才会正常换行 —— 分配走 wrapped layout 的 same_row_or_next_row。
    const PAD_X: f32 = 13.0;
    const PAD_Y: f32 = 8.0;
    const GAP: f32 = 6.0;
    let label_galley = egui::WidgetText::from(RichText::new(label).small().color(theme::muted()))
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
    let value_galley = egui::WidgetText::from(
        RichText::new(value).strong().color(color).size(13.5),
    )
    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
    let size = egui::vec2(
        label_galley.size().x + GAP + value_galley.size().x + PAD_X * 2.0,
        label_galley.size().y.max(value_galley.size().y) + PAD_Y * 2.0,
    );
    let label_galley = std::sync::Arc::new(label_galley);
    let value_galley = std::sync::Arc::new(value_galley);
    let (rect, _response) = ui.allocate_at_least(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_BTN),
            theme::accent_bg(),
        );
        let center_y = rect.center().y;
        let mut x = rect.left() + PAD_X;
        for galley in [label_galley.as_ref(), value_galley.as_ref()] {
            let top = center_y - galley.size().y / 2.0;
            painter.galley(egui::pos2(x, top), galley.clone(), theme::text_secondary());
            x += galley.size().x + GAP;
        }
    }
}

/// 单个词的小胶囊（版本号、许可、状态）。和 [`stat_chip`] 同一套"自己量再画"的写法，
/// 不用 `Frame::show`：后者在 `horizontal_wrapped` 里不参与换行判定，会把父 `max_rect`
/// 撑大（`stat_chip` 那条注释记着实测数字）。
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    const PAD_X: f32 = 10.0;
    const PAD_Y: f32 = 5.0;
    let galley = egui::WidgetText::from(RichText::new(text).strong().size(11.5).color(fg))
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
    let size = galley.size() + Vec2::new(PAD_X * 2.0, PAD_Y * 2.0);
    let (rect, _response) = ui.allocate_at_least(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_BTN), bg);
        painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
}

/// 协议首字母缩写图标 + 数量胶囊徽标
pub fn protocol_stat_chip(
    ui: &mut Ui,
    protocol: &str,
    count: i64,
    is_active: bool,
) -> egui::Response {
    let initial = protocol
        .chars()
        .next()
        .unwrap_or('P')
        .to_ascii_uppercase();

    // 协议首字母个性化配色
    let (badge_bg, badge_fg) = match initial {
        'V' => (theme::accent_bg(), theme::accent_strong()),
        'S' => (theme::success_bg(), theme::success()),
        'T' => (theme::warning_bg(), theme::warning()),
        'H' => (Color32::from_rgb(243, 232, 255), Color32::from_rgb(147, 51, 234)),
        'W' | 'M' => (Color32::from_rgb(224, 242, 254), Color32::from_rgb(2, 132, 199)),
        _ => (theme::panel_hover(), theme::text_secondary()),
    };

    const PAD_X: f32 = 7.0;
    const PAD_Y: f32 = 4.0;
    const GAP: f32 = 6.0;
    const BADGE_SIZE: f32 = 18.0;

    let count_str = format!("{count}");
    let count_galley = egui::WidgetText::from(
        RichText::new(&count_str)
            .strong()
            .color(if is_active { theme::accent_strong() } else { theme::text_primary() })
            .size(12.0),
    )
    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);

    let total_w = PAD_X * 2.0 + BADGE_SIZE + GAP + count_galley.size().x;
    let total_h = (BADGE_SIZE.max(count_galley.size().y) + PAD_Y * 2.0).max(26.0);
    let size = egui::vec2(total_w, total_h);

    let (rect, response) = ui.allocate_at_least(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let (bg, stroke) = if is_active {
            (theme::accent_bg(), Stroke::new(1.2_f32, theme::accent()))
        } else if response.hovered() {
            (theme::panel_hover(), Stroke::new(1.0_f32, theme::border()))
        } else {
            (theme::panel(), Stroke::new(1.0_f32, theme::border_subtle()))
        };

        painter.rect(
            rect,
            CornerRadius::same(theme::RADIUS_BTN),
            bg,
            stroke,
            egui::StrokeKind::Inside,
        );

        // 绘制协议首字母徽标图标（带有微圆角与对比反光）
        let badge_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + PAD_X + BADGE_SIZE / 2.0, rect.center().y),
            Vec2::splat(BADGE_SIZE),
        );
        painter.rect_filled(badge_rect, CornerRadius::same(4), badge_bg);
        painter.rect_stroke(
            badge_rect,
            CornerRadius::same(4),
            Stroke::new(0.8_f32, badge_fg.gamma_multiply(0.4)),
            egui::StrokeKind::Inside,
        );

        let initial_galley = egui::WidgetText::from(
            RichText::new(initial.to_string()).strong().color(badge_fg).size(11.0),
        )
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
        let init_pos = egui::pos2(
            badge_rect.center().x - initial_galley.size().x / 2.0,
            badge_rect.center().y - initial_galley.size().y / 2.0,
        );
        painter.galley(init_pos, initial_galley, badge_fg);

        // 绘制数量文字
        let count_pos = egui::pos2(
            badge_rect.right() + GAP,
            rect.center().y - count_galley.size().y / 2.0,
        );
        painter.galley(count_pos, count_galley, theme::text_primary());
    }

    response.on_hover_text(format!("{} 协议：{} 个节点（点击快速筛选）", protocol.to_uppercase(), count))
}

/// 卡片表头左边那个圆角方块图标底（40×40）。同 [`pill`]：直接画，不套 `Frame`。
pub fn icon_tile(ui: &mut Ui, icon: Icon, fg: Color32, bg: Color32) {
    const TILE: f32 = 40.0;
    const RADIUS: u8 = 13;
    let (rect, _response) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(RADIUS), bg);
        crate::gui::icons::draw_icon(painter, rect.shrink(9.5), icon, fg);
    }
}

/// 量着把一行文字截进 `max_width`：放得下原样返回（只量一次），放不下才二分找"接上
/// 省略号还塞得进"的最长前缀。逐字回退会让一条 300 字符的值每帧排 300 次版，二分是 9 次。
///
/// 为什么不靠 egui 自己截：`allocate_new_ui` 只改 `max_rect` 不改裁剪框，光把文字塞进
/// 子 ui 是裁不住的（关于页探针量到那条值的裁剪右界是整屏 1330，卡片右界只有 1290）；
/// 而就算 `set_clip_rect` 了，硬裁出来的也是半截字，不是省略号。
pub fn ellipsize(ui: &Ui, text: &str, max_width: f32, font: egui::FontId) -> String {
    const ELLIPSIS: char = '\u{2026}';
    let width = |candidate: &str| -> f32 {
        ui.ctx()
            .fonts(|fonts| {
                fonts.layout_no_wrap(candidate.to_string(), font.clone(), Color32::PLACEHOLDER)
            })
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let head: String = chars[..mid].iter().copied().collect();
        if width(&format!("{head}{ELLIPSIS}")) <= max_width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let head: String = chars[..lo].iter().copied().collect();
    format!("{head}{ELLIPSIS}")
}

/// 清理订阅源常见的冗余前缀，让节点名称舒展通透、直奔主题
pub fn clean_node_display_name(name: &str) -> &str {
    let mut s = name.trim();
    const NOISY_PREFIXES: &[&str] = &[
        "地区随机 | ",
        "地区随机|",
        "Gslege优选 | ",
        "Gslege优选|",
        "全网优选 | ",
        "全网优选|",
        "自动优选 | ",
        "自动优选|",
        "优选节点 | ",
        "优选节点|",
        "VIP | ",
        "VIP|",
    ];
    for p in NOISY_PREFIXES {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest.trim();
        }
    }
    s
}

pub fn latency_text(n: &NodeSummary) -> RichText {
    match (n.latency_ok, n.latency_test_kind.as_deref()) {
        (Some(true), Some(GOOGLE_PROXY_TEST_KIND)) => {
            let ms = n.latency_ms.unwrap_or(0);
            let color = if ms < 300 {
                theme::success()
            } else if ms < 800 {
                theme::text_secondary()
            } else {
                theme::muted()
            };
            RichText::new(format!("{ms} ms")).color(color)
        }
        (Some(false), Some(GOOGLE_PROXY_TEST_KIND)) => {
            RichText::new("超时").color(theme::danger())
        }
        (Some(true), _) => {
            let ms = n.latency_ms.unwrap_or(0);
            RichText::new(format!("{ms} ms (TCP)")).color(theme::text_secondary())
        }
        (Some(false), _) => RichText::new("TCP 失败").color(theme::muted()),
        (None, _) => RichText::new("未测速").color(theme::muted()),
    }
}

pub fn routing_mode_description(mode: RoutingMode) -> &'static str {
    match mode {
        RoutingMode::Rule => "按本地自定义规则依次匹配",
        RoutingMode::ChinaDirect => "在线 Geo 规则 · 国内直连 · 其余代理",
        RoutingMode::GfwList => "在线 GFW 规则 · 命中代理 · 其余直连",
        RoutingMode::Global => "所有流量使用当前代理节点",
        RoutingMode::Direct => "所有流量绕过代理直接连接",
    }
}

pub fn routing_mode_detail(mode: RoutingMode) -> &'static str {
    match mode {
        RoutingMode::Rule => "保留并使用下方有序规则；没有命中的流量使用代理。",
        RoutingMode::ChinaDirect => {
            "自动使用广告、GeoSite CN 与 GeoIP CN 规则；在线数据由对应内核更新。"
        }
        RoutingMode::GfwList => "仅 GFW 规则集命中的站点使用代理，其他流量直接连接。",
        RoutingMode::Global => "忽略自定义规则，所有可代理流量都使用当前节点。",
        RoutingMode::Direct => "忽略自定义规则，代理内核保持运行但流量全部直连。",
    }
}

pub fn source_status_failed(status: &str) -> bool {
    let status = status.trim().to_ascii_lowercase();
    !status.is_empty()
        && status != "ok"
        && status != "not modified"
        && status != "content unchanged"
}

pub fn latency_label(v: &str) -> &'static str {
    match v {
        "alive" => "可用",
        "dead" => "不可用",
        "untested" => "未测速",
        _ => "全部",
    }
}

pub fn region_label(code: &str) -> &'static str {
    match code {
        "all" => "全部地区",
        "HK" => "香港",
        "JP" => "日本",
        "US" => "美国",
        "SG" => "新加坡",
        "TW" => "台湾",
        "KR" => "韩国",
        "OTHER" => "其它",
        _ => "全部地区",
    }
}

pub fn country_flag_and_name(code: &str) -> (&'static str, &'static str) {
    match code.to_ascii_uppercase().as_str() {
        "HK" => ("🇭🇰", "香港"),
        "JP" => ("🇯🇵", "日本"),
        "US" => ("🇺🇸", "美国"),
        "SG" => ("🇸🇬", "新加坡"),
        "TW" => ("🇨🇳", "台湾"),
        "KR" => ("🇰🇷", "韩国"),
        "GB" | "UK" => ("🇬🇧", "英国"),
        "DE" => ("🇩🇪", "德国"),
        "FR" => ("🇫🇷", "法国"),
        "CA" => ("🇨🇦", "加拿大"),
        "AU" => ("🇦🇺", "澳大利亚"),
        "RU" => ("🇷🇺", "俄罗斯"),
        "IN" => ("🇮🇳", "印度"),
        "NL" => ("🇳🇱", "荷兰"),
        "MY" => ("🇲🇾", "马来西亚"),
        "TH" => ("🇹🇭", "泰国"),
        "PH" => ("🇵🇭", "菲律宾"),
        "VN" => ("🇻🇳", "越南"),
        "TR" => ("🇹🇷", "土耳其"),
        _ => ("🌐", "其它"),
    }
}

pub fn proxy_state_label(state: ProxyState) -> (&'static str, Color32) {
    match state {
        ProxyState::Stopped => ("已停止", theme::muted()),
        ProxyState::Starting => ("启动中", theme::warning()),
        ProxyState::Running => ("运行中", theme::success()),
        ProxyState::Degraded => ("降级", theme::warning()),
        ProxyState::Stopping => ("停止中", theme::warning()),
        ProxyState::Failed => ("失败", theme::danger()),
    }
}

pub fn tun_stack_label(stack: TunStack) -> &'static str {
    match stack {
        TunStack::System => "System",
        TunStack::Gvisor => "gVisor",
        TunStack::Mixed => "Mixed",
    }
}

pub fn dns_mode_label(mode: DnsEnhancedMode) -> &'static str {
    match mode {
        DnsEnhancedMode::FakeIp => "Fake-IP",
        DnsEnhancedMode::RedirHost => "Redir-Host",
    }
}

pub fn rule_action_label(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Direct => "直连",
        RuleAction::Proxy => "代理",
        RuleAction::Reject => "拒绝",
    }
}

pub fn rule_matcher_label(matcher: &RuleMatcher) -> String {
    match matcher {
        RuleMatcher::Domain(value) => format!("DOMAIN · {value}"),
        RuleMatcher::DomainSuffix(value) => format!("DOMAIN-SUFFIX · {value}"),
        RuleMatcher::GeoSite(value) => format!("GEOSITE · {value}"),
        RuleMatcher::GeoIp(value) => format!("GEOIP · {value}"),
        RuleMatcher::IpCidr(value) => format!("IP-CIDR · {value}"),
        RuleMatcher::DestinationPort(value) => format!("DST-PORT · {value}"),
        RuleMatcher::ProcessName(value) => format!("PROCESS-NAME · {value}"),
        RuleMatcher::RuleSet(value) => format!("RULE-SET · {value}"),
        RuleMatcher::Network(NetworkProtocol::Tcp) => "NETWORK · tcp".into(),
        RuleMatcher::Network(NetworkProtocol::Udp) => "NETWORK · udp".into(),
        RuleMatcher::Quic => "QUIC · udp/443".into(),
    }
}

pub fn sort_label(v: &str) -> &'static str {
    match v {
        "latency" => "延迟",
        "download" => "下载速度",
        "name" => "名称",
        _ => "更新时间",
    }
}

pub fn format_speedtest_progress(progress: &SpeedTestProgress) -> String {
    let phase = match progress.phase {
        SpeedTestPhase::TcpPrefilter => "TCP预筛",
        SpeedTestPhase::RealProxy => "真实代理验证",
    };
    let percent = if progress.total == 0 {
        100.0
    } else {
        progress.done as f64 * 100.0 / progress.total as f64
    };
    let detail = match (progress.kernel, progress.batch_index, progress.batch_total) {
        (Some(kernel), Some(batch), Some(total)) => {
            format!(" · {} 批次 {batch}/{total}", kernel.display_name())
        }
        (Some(kernel), _, _) => format!(" · {}", kernel.display_name()),
        _ => String::new(),
    };
    format!(
        "{phase}{detail} {}/{}（{percent:.1}%） 成功 {} / 失败 {}，耗时 {:.1}s",
        progress.done,
        progress.total,
        progress.ok,
        progress.fail,
        progress.elapsed.as_secs_f64()
    )
}

pub fn nonempty_opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn node_group_key(group: &NodeGroupSummary) -> String {
    group
        .source_id
        .clone()
        .unwrap_or_else(|| crate::db::UNGROUPED_SOURCE_ID.into())
}

pub fn requires_tun_elevation(mode: ProxyMode, elevated: bool) -> bool {
    mode == ProxyMode::Tun && !elevated
}

#[cfg(test)]
mod sparkline_tests {
    use super::{
        health_failed_color, health_hour_buckets, render_health_strip, render_traffic_sparkline,
    };
    use eframe::egui::{self, CentralPanel, Color32, RawInput, Rect, Vec2};
    use std::collections::VecDeque;

    fn history(points: usize) -> VecDeque<(f32, f32)> {
        (0..points)
            .map(|index| (10.0 + (index % 7) as f32, 1.0 + (index % 3) as f32))
            .collect()
    }

    /// 照表头第二行的写法跑一遍：读数排在前，迷你图吃掉剩下的宽度。
    /// 返回 (图拿到的宽度, 整行占掉的高度, 这一行之后还剩多少高度)。
    fn probe(window_w: f32, samples: usize, active: bool) -> (f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_w, 800.0),
        ));
        let mut room = -1.0_f32;
        let mut row = (-1.0_f32, -1.0_f32);
        let _ = ctx.run(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let before = ui.available_height();
                let response = ui.horizontal(|ui| {
                    ui.label("延迟: 386 ms");
                    ui.separator();
                    ui.label("端口: Mixed 9980 · Controller 9090");
                    ui.separator();
                    ui.label("↓ 2.5 KB/s · ↑ 387 B/s");
                    room = ui.available_width();
                    render_traffic_sparkline(ui, &history(samples), active);
                });
                row = (
                    response.response.rect.height(),
                    before - ui.available_height(),
                );
            });
        });
        (room, row.0, row.1)
    }

    #[test]
    fn sparkline_takes_the_rest_of_the_header_row() {
        let (room, height, used) = probe(1280.0, 60, true);
        assert!(room > 300.0, "1280 宽的窗口里图还应该剩 {room:.0}px 可画");
        assert!(
            (height - 32.0).abs() < 6.0,
            "这一行应该只占 ~32 高，实测 {height:.1}"
        );
        assert!(
            used < 40.0,
            "整行吃掉的页高 {used:.1}，多了就是又被 with_layout 吃页了"
        );
    }

    #[test]
    fn sparkline_still_has_room_at_the_minimum_window_width() {
        let (room, height, used) = probe(960.0, 60, true);
        assert!(room > 120.0, "960 最小宽度下图仍应有 {room:.0}px");
        assert!(used < 40.0, "窄窗口也不能吃页，实测 {used:.1}");
        assert!(height < 40.0, "行高 {height:.1}");
    }

    #[test]
    fn sparkline_survives_no_samples_and_a_stopped_proxy() {
        // 采样点不足两个时那条 dx 会除零，未运行时也不该 panic。
        for (samples, active) in [(0usize, true), (1, true), (60, false)] {
            let (room, _, _) = probe(1280.0, samples, active);
            assert!(room > 0.0, "samples={samples} active={active}");
        }
    }

    /// 健康条的尺寸：宽窗口要真的"看得见"（>=220 宽、18 高），窄窗口不许溢出可用宽度
    /// （溢出的话 egui 会把整条折到第二行，卡片被顶高 —— 那是他否过的表现）。
    #[test]
    fn health_strip_scales_with_the_room_it_has() {
        fn probe(window_width: f32) -> (f32, f32, f32) {
            let ctx = egui::Context::default();
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(window_width, 200.0),
            ));
            let mut drawn = (0.0, 0.0, 0.0);
            let _ = ctx.run(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("实测 12:30 ✓ 262ms");
                        let before = ui.available_width();
                        render_health_strip(ui, &[1i8; 24]);
                        drawn = (before, ui.available_width(), ui.cursor().height());
                    });
                });
            });
            drawn
        }

        let (before, after, _) = probe(1280.0);
        let used = before - after;
        // 24 格 × (10 + 2) + 8 的格间距 = 296；放大前是 24×(5+1)+8 = 152。
        assert!(
            (280.0..=310.0).contains(&used),
            "1280 宽时健康条应该占 {used:.0}px（放大过，不该还是原来那 152）"
        );
        // 窄一点时格子缩到 6px，但绝不能吃满再溢出（溢出=折行=顶高卡片）。
        let (narrow_before, narrow_after, _) = probe(560.0);
        assert!(
            narrow_before - narrow_after <= narrow_before,
            "560 宽时健康条溢出：占 {:.0} / 可用 {:.0}",
            narrow_before - narrow_after,
            narrow_before
        );
    }

    /// 健康条"那一小时有不通"的格子必须真的画成红（2026-10-15 他点名要红）。
    /// 读的是这一帧真画出来的矩形填充色，不是常量 —— 旧写法用的是 `text_primary()`
    /// （石墨深色），照着代码念一遍是看不出差别的。
    #[test]
    fn the_failed_hours_paint_red() {
        fn fills(buckets: &[i8; 24]) -> Vec<Color32> {
            let ctx = egui::Context::default();
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1280.0, 200.0),
            ));
            let mut out = Vec::new();
            let _ = ctx.run(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let layer = ui.layer_id();
                    render_health_strip(ui, buckets);
                    out = ctx.graphics_mut(|g| {
                        g.get(layer)
                            .map(|list| {
                                list.all_entries()
                                    .filter_map(|shape| match &shape.shape {
                                        egui::Shape::Rect(rect)
                                            if (rect.rect.height() - 18.0).abs() < 0.1 =>
                                        {
                                            Some(rect.fill)
                                        }
                                        _ => None,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    });
                });
            });
            out
        }

        let mut buckets = [0i8; 24];
        for index in [0usize, 3, 9] {
            buckets[index] = -1;
        }
        for index in [1usize, 2, 4] {
            buckets[index] = 1;
        }
        let painted = fills(&buckets);
        assert_eq!(painted.len(), 24, "24 格应该画满，实际 {}", painted.len());
        assert_eq!(
            painted
                .iter()
                .filter(|c| **c == health_failed_color())
                .count(),
            3,
            "红格数不对（三个有不通的小时没都画成红）"
        );
        assert_eq!(
            painted
                .iter()
                .filter(|c| **c == super::theme::success())
                .count(),
            3,
            "绿格数不对"
        );
        assert_eq!(
            painted
                .iter()
                .filter(|c| **c == super::theme::border())
                .count(),
            18,
            "没实测的那 18 格颜色不对"
        );
        // 反向：这要是还退回旧的深色写法，这条就红。
        assert_ne!(
            health_failed_color(),
            super::theme::text_primary(),
            "红和正文深色撞了，等于没改"
        );
    }

    fn health_row(minutes_ago: i64, ok: bool) -> crate::db::telemetry::HealthRow {
        crate::db::telemetry::HealthRow {
            at: (chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339(),
            ok: ok as i64,
            latency_ms: Some(120),
            error: None,
            node_id: None,
            node_name: None,
            kind: "periodic".into(),
        }
    }

    /// 健康条的桶改成"数据到达时算一次"之后，这条把语义钉住：小时槽、失败优先、
    /// 24 小时之外和未来的行都不许进来。
    #[test]
    fn health_buckets_are_hourly_failure_wins_and_window_bound() {
        let empty = health_hour_buckets(&[]);
        assert_eq!(empty, [0i8; 24], "没数据就是 24 格全空");

        let one = health_hour_buckets(&[health_row(10, true)]);
        assert_eq!(one[0], 1, "十分钟前的实测落在当前这一格");
        assert_eq!(one[1], 0);

        let fail = health_hour_buckets(&[health_row(90, false)]);
        assert_eq!(fail[1], -1, "一个半小时前是上一格");

        let mixed = health_hour_buckets(&[health_row(5, true), health_row(20, false)]);
        assert_eq!(mixed[0], -1, "同一小时里失败优先，绿不能把红盖掉");

        let stale = health_hour_buckets(&[health_row(24 * 60 + 5, false)]);
        assert_eq!(stale, [0i8; 24], "超出 24 小时的行不该挤进最老那一格");

        // `num_hours()` 是**向零截断**的：一小时以内的时钟漂移会算进当前这格（改之前
        // 就是这个语义，这次纯性能重构不动它），超过一小时才落到负数被丢掉。
        let near_future = health_hour_buckets(&[health_row(-30, false)]);
        assert_eq!(
            near_future[0], -1,
            "半小时内飘到未来的行落在当前格，不是越界"
        );
        let future = health_hour_buckets(&[health_row(-90, false)]);
        assert_eq!(
            future, [0i8; 24],
            "一小时以上的未来行必须丢，否则索引是负数"
        );
    }
}
#[cfg(test)]
mod button_radius_tests {
    use super::{danger_btn, primary_btn};
    use eframe::egui::{self, CentralPanel, CornerRadius, RawInput, Rect, Vec2};

    /// 读这一帧里画出来的所有"按钮那么高"的矩形的圆角。
    fn radii(window: f32) -> Vec<CornerRadius> {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window, 200.0),
        ));
        let mut out = Vec::new();
        let _ = ctx.run(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let layer = ui.layer_id();
                ui.horizontal(|ui| {
                    let _ = ui.button("IP 检测");
                    let _ = ui.add(primary_btn("启动代理"));
                    let _ = ui.add(danger_btn("停止代理"));
                });
                out = ctx.graphics_mut(|g| {
                    g.get(layer)
                        .map(|list| {
                            list.all_entries()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Rect(rect)
                                        if (4.0..=40.0).contains(&rect.rect.height()) =>
                                    {
                                        Some(rect.corner_radius)
                                    }
                                    _ => None,
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
            });
        });
        out
    }

    /// 状态栏那三个按钮必须一个圆角：普通按钮 2px，主/危险按钮以前各自钉了 16px，
    /// 于是同一条里"停止代理"是圆的、旁边"IP 检测"是方的（2026-10-15 他点的这条）。
    #[test]
    fn primary_and_danger_share_the_plain_button_radius() {
        let painted = radii(1280.0);
        assert!(
            painted.len() >= 3,
            "三个按钮的背景矩形没都画出来，只读到 {}",
            painted.len()
        );
        let plain = painted[0];
        for (index, radius) in painted.iter().enumerate() {
            assert_eq!(
                *radius, plain,
                "第 {index} 个按钮圆角 {radius:?} 和普通按钮 {plain:?} 不一致"
            );
        }
        assert_ne!(
            plain,
            CornerRadius::same(16),
            "还是那个 16 的大圆角 —— 说明 helper 里那两行没删掉"
        );
    }
}

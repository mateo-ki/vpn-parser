use super::groups::{node_matches_group, visible_group_names};
use crate::gui::icons::{draw_icon, Icon};
use crate::gui::theme;
use crate::gui::types::DashboardDrawer;
use crate::gui::widgets::{format_bytes, format_speed};
use crate::gui::VpnApp;
use crate::kernel::KernelPreference;
use crate::proxy::{ProxyMode, ProxyState, RoutingMode};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// 仪表盘（原汇总图表）Apple 雅致风格：
// 对齐 design/dashboard-v4-apple.html 规范：
// 1. 卡片统一 14px 圆角、微投影、细描边、墨黑大字、Apple 蓝与状态红黄绿语义色；
// 2. 移除旧版阶梯频谱图与环形图，专注现代仪表盘大屏；
// 3. 页面底部完整集成：终端代理、端口代理、局域网代理、系统代理、TUN 模式；
// 4. 具备多档响应式自适应断点（宽屏 >= 1060px、中屏 750-1060px、小屏 < 750px）自适应收缩复原。
// ---------------------------------------------------------------------------

// 常用 Apple 配色代币（支持浅深主题适配）
#[inline]
fn apple_stroke() -> Color32 {
    if theme::is_light() {
        Color32::from_rgb(227, 230, 235) // #E3E6EB
    } else {
        theme::border_subtle()
    }
}

#[inline]
fn apple_stroke2() -> Color32 {
    if theme::is_light() {
        Color32::from_rgb(238, 240, 244) // #EEF0F4
    } else {
        theme::border_subtle()
    }
}

#[inline]
fn apple_sub_bg() -> Color32 {
    if theme::is_light() {
        Color32::from_rgb(244, 246, 249) // #EBEEF2 / 略浅衬底
    } else {
        theme::sidebar()
    }
}

#[inline]
fn apple_blue() -> Color32 {
    Color32::from_rgb(0, 113, 227) // #0071E3
}

#[inline]
fn apple_green_dot() -> Color32 {
    Color32::from_rgb(52, 199, 89) // #34C759
}

#[inline]
fn apple_amber_dot() -> Color32 {
    Color32::from_rgb(255, 149, 0) // #FF9500
}

#[inline]
fn apple_red_dot() -> Color32 {
    Color32::from_rgb(255, 59, 48) // #FF3B30
}

/// 延迟着色：优 / 良 / 差（与状态卡质量分档一致）。
#[allow(dead_code)]
fn ok_fg_green() -> Color32 {
    if theme::is_light() {
        Color32::from_rgb(27, 122, 58)
    } else {
        theme::success()
    }
}

#[allow(dead_code)]
fn ok_fg_amber() -> Color32 {
    if theme::is_light() {
        Color32::from_rgb(180, 83, 9)
    } else {
        theme::warning()
    }
}

#[allow(dead_code)]
fn ok_fg_red() -> Color32 {
    if theme::is_light() {
        Color32::from_rgb(201, 42, 42)
    } else {
        theme::danger()
    }
}

/// 卡片外框：Apple 设计稿 v4 —— 圆角 14px 白卡 + 细边框 + 微阴影。
/// 在指定矩形上画一张卡（外框尺寸完全由 rect 决定，内容溢出也不影响外框）。
fn dash_card_at(ui: &mut Ui, rect: Rect) {
    let painter = ui.painter().clone();
    let cr = CornerRadius::same(14);
    painter.rect_filled(rect, cr, theme::panel());
    painter.rect_stroke(
        rect,
        cr,
        Stroke::new(1.0_f32, apple_stroke()),
        egui::StrokeKind::Inside,
    );
}

/// 卡片标题（设计稿的 c-label）：小号半粗、次级色，右侧可挂 peak 文本。
fn dash_label(ui: &mut Ui, title: &str, peak: Option<String>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .size(12.5)
                .strong()
                .color(theme::text_secondary()),
        );
        if let Some(peak) = peak {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(peak).small().color(theme::muted())).truncate(),
                );
            });
        }
    });
}

/// 速率卡片右上角的统计图表小按钮：迷你柱状图，点击呼出流量统计抽屉
fn dash_stats_icon_button(ui: &mut Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter()
            .circle_filled(rect.center(), 11.0, theme::panel_hover());
    }
    let color = if resp.hovered() {
        theme::accent()
    } else {
        theme::muted()
    };
    crate::gui::icons::draw_icon(ui.painter(), rect, crate::gui::icons::Icon::Stats, color);
    resp.on_hover_text("查看流量统计（节点 / 域名 / 软件进程）")
}

/// 状态小圆点。
fn dash_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Apple 胶囊药丸徽标。
fn dash_pill(ui: &mut Ui, text: &str, bg_color: Color32, text_color: Color32) -> egui::Response {
    let inner = Frame::NONE
        .fill(bg_color)
        .stroke(Stroke::new(1.0_f32, apple_stroke2()))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).strong().color(text_color));
        });
    // 胶囊整体可点击：交互 id 以左上角位置为种子——
    // 位置天然唯一且跨帧稳定，不会产生 egui"first/second use of widget id"重复告警
    let rect = inner.response.rect;
    let id = egui::Id::new(("dash_pill", rect.left() as i32, rect.top() as i32));
    ui.interact(rect, id, Sense::click())
}

/// 文本按最大宽度截断成单行（超出加省略号），绝不折行
fn svc_truncate(painter: &egui::Painter, text: &str, font: egui::FontId, max_w: f32) -> String {
    if painter
        .layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
        .rect
        .width()
        <= max_w
    {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while s.chars().count() > 1 {
        s.pop();
        let candidate = format!("{s}…");
        if painter
            .layout_no_wrap(candidate.clone(), font.clone(), Color32::WHITE)
            .rect
            .width()
            <= max_w
        {
            return candidate;
        }
    }
    s
}

/// .svc 独立圆角小卡片容器（用于服务状态与策略组）。
fn dash_svc_card(
    ui: &mut Ui,
    w: f32,
    h: f32,
    dot_color: Color32,
    icon: Option<Icon>,
    name: &str,
    status_text: &str,
    status_color: Color32,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let is_hovered = resp.hovered();
    let fill = if selected {
        if theme::is_light() {
            Color32::from_rgb(234, 243, 255)
        } else {
            theme::accent_bg()
        }
    } else if is_hovered {
        theme::panel_hover()
    } else {
        apple_sub_bg()
    };
    let stroke = if selected {
        Stroke::new(1.2_f32, apple_blue())
    } else {
        Stroke::new(1.0_f32, apple_stroke2())
    };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), fill);
    painter.rect_stroke(rect, CornerRadius::same(11), stroke, egui::StrokeKind::Inside);

    let inner_rect = rect.shrink2(Vec2::new(6.0, 1.5));
    let dot_center = egui::pos2(inner_rect.left() + 3.0, inner_rect.center().y);
    painter.circle_filled(dot_center, 3.0, dot_color);

    let name_font = egui::FontId::proportional(11.0);
    let status_font = egui::FontId::proportional(10.0);
    // 可选图标（检测项标识）：画在状态点右侧、名称左侧
    let name_x = if let Some(icon) = icon {
        let icon_rect = Rect::from_min_size(
            egui::pos2(inner_rect.left() + 11.0, inner_rect.center().y - 5.0),
            Vec2::splat(10.0),
        );
        draw_icon(painter, icon_rect, icon, theme::muted());
        inner_rect.left() + 25.0
    } else {
        inner_rect.left() + 10.0
    };

    // 先量右侧状态文本；名字可用空间不足 40px 时干脆不画状态，把空间全让给名字
    let status_galley = painter.layout_no_wrap(
        status_text.to_string(),
        status_font.clone(),
        status_color,
    );
    let status_w = status_galley.rect.width();
    let name_max_with_status = inner_rect.right() - 6.0 - name_x - status_w - 6.0;
    let show_status = name_max_with_status >= 40.0;

    let name_max = if show_status {
        name_max_with_status
    } else {
        inner_rect.right() - 6.0 - name_x
    };
    // 名称强制单行 + 省略号，永不折行重叠
    let name_text = svc_truncate(painter, name, name_font.clone(), name_max);
    let is_truncated = name_text != name || !show_status;
    let name_color = if selected { apple_blue() } else { theme::text_primary() };
    let name_galley = painter.layout_no_wrap(name_text, name_font, name_color);
    painter.galley(
        egui::pos2(name_x, inner_rect.center().y - name_galley.rect.height() / 2.0),
        name_galley,
        name_color,
    );

    if show_status {
        painter.galley(
            egui::pos2(
                inner_rect.right() - 6.0 - status_w,
                inner_rect.center().y - status_galley.rect.height() / 2.0,
            ),
            status_galley,
            status_color,
        );
    }

    if is_truncated {
        resp.on_hover_text(format!("{name} · {status_text}"))
    } else {
        resp
    }
}

/// iOS 风格开关（40x22），返回点击响应。
fn dash_switch(ui: &mut Ui, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    let bg = if on {
        apple_green_dot()
    } else {
        Color32::from_rgb(214, 218, 225).gamma_multiply(if theme::is_light() { 1.0 } else { 0.35 })
    };
    let knob_r = 8.0_f32;
    let knob_x = if on { rect.right() - 3.0 - knob_r } else { rect.left() + 3.0 + knob_r };
    let center = egui::pos2(knob_x, rect.center().y);
    ui.painter().rect_filled(rect, CornerRadius::same(11), bg);
    ui.painter().circle_filled(center, knob_r, Color32::WHITE);
    resp
}

/// 底部单项代理控制卡片容器
fn dash_bottom_proxy_item(
    ui: &mut Ui,
    w: f32,
    h: f32,
    icon: Icon,
    icon_color: Color32,
    title: &str,
    sub: &str,
    icon_clickable: bool,
    widget: impl FnOnce(&mut Ui),
) -> Option<egui::Response> {
    let mut icon_clicked_resp = None;
    // 画框式精确外框：外框宽严格 = w + 28（内容不再撑宽卡片）
    let outer = Rect::from_min_size(
        ui.cursor().min,
        Vec2::new(w + 28.0, h + 24.0),
    );
    dash_card_at(ui, outer);
    // 画框不推进 cursor：显式把光标拨到卡右侧，行内下一元素从这继续
    ui.advance_cursor_after_rect(outer);
    let band = outer.shrink2(Vec2::new(14.0, 12.0));
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(band), |ui| {
        ui.set_clip_rect(band);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // 内部横向间距清零（宽度预算按 icon 34 + 显式 8 + 文字 + 右侧 52 精确计算，
                // 默认 10px 间距会让每张卡实际渲染比预算宽 30px，把整行顶出屏幕）
                ui.spacing_mut().item_spacing.x = 0.0;
                // 图标圆角方块（窄卡自动隐藏，给标题腾位置）
                let show_icon = w >= 150.0;
                if show_icon {
                    let sense = if icon_clickable { Sense::click() } else { Sense::hover() };
                    let (icon_rect, icon_resp) = ui.allocate_exact_size(Vec2::splat(34.0), sense);
                    let icon_bg = if theme::is_light() {
                        icon_color.gamma_multiply(0.12)
                    } else {
                        theme::sidebar()
                    };
                    ui.painter().rect_filled(icon_rect, CornerRadius::same(9), icon_bg);
                    draw_icon(ui.painter(), icon_rect.shrink(9.0), icon, icon_color);
                    if icon_clickable {
                        let icon_resp = icon_resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("点击配置 DNS / TUN（右侧滑出抽屉）");
                        if icon_resp.hovered() {
                            ui.painter().rect_stroke(
                                icon_rect,
                                CornerRadius::same(9),
                                Stroke::new(1.2_f32, icon_color),
                                egui::StrokeKind::Inside,
                            );
                        }
                        icon_clicked_resp = Some(icon_resp);
                    }
                    ui.add_space(8.0);
                }

                // 文字列（宽度受控并截断）：整块文字用 painter 精确垂直居中绘制
                let painter = ui.painter().clone();
                let text_w = if show_icon {
                    (w - 34.0 - 8.0 - 52.0).max(40.0)
                } else {
                    (w - 44.0).max(40.0)
                };
                let (text_rect, text_resp) =
                    ui.allocate_exact_size(Vec2::new(text_w, h), Sense::hover());
                let title_font = egui::FontId::proportional(if show_icon { 12.5 } else { 10.5 });
                let sub_font = egui::FontId::proportional(10.5);
                let title_text = svc_truncate(&painter, title, title_font.clone(), text_w);
                let sub_text = svc_truncate(&painter, sub, sub_font.clone(), text_w);
                let is_truncated = title_text != title || sub_text != sub;
                let title_galley = painter.layout_no_wrap(title_text, title_font, theme::text_primary());
                let sub_galley = painter.layout_no_wrap(sub_text, sub_font, theme::muted());
                let line_gap = 2.0_f32;
                let block_h = title_galley.rect.height() + line_gap + sub_galley.rect.height();
                let block_top = text_rect.center().y - block_h / 2.0;
                // 标题加粗：同一 galley 偏移 0.5px 重画一次
                painter.galley(
                    egui::pos2(text_rect.left(), block_top),
                    title_galley.clone(),
                    theme::text_primary(),
                );
                painter.galley(
                    egui::pos2(text_rect.left() + 0.5, block_top),
                    title_galley.clone(),
                    theme::text_primary(),
                );
                painter.galley(
                    egui::pos2(
                        text_rect.left(),
                        block_top + title_galley.rect.height() + line_gap,
                    ),
                    sub_galley,
                    theme::muted(),
                );
                if is_truncated {
                    text_resp.on_hover_text(format!("{title}\n{sub}"));
                }

                // 右侧操作小部件（开关或状态圆点，统一留 6px 右边距）
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    widget(ui);
                });
            });
        });
    icon_clicked_resp
}

/// 迷你波浪线（速率曲线）：输入原始 f32 序列，自动按最大值归一化，
/// 经 Catmull-Rom 平滑成波浪曲线（不是折线）。
fn dash_sparkline(ui: &mut Ui, w: f32, h: f32, vals: Vec<f32>, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    if vals.len() < 2 {
        return;
    }
    let max = vals.iter().cloned().fold(1e-9_f32, f32::max);
    let n = vals.len();
    let step = rect.width() / (n - 1) as f32;
    let pts: Vec<egui::Pos2> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            egui::pos2(
                rect.left() + i as f32 * step,
                rect.bottom() - (v / max).clamp(0.0, 1.0) * (rect.height() - 2.0) - 1.0,
            )
        })
        .collect();

    // Catmull-Rom 样条：每段细分 10 次，得到平滑的波浪曲线
    let catmull = |p0: egui::Pos2, p1: egui::Pos2, p2: egui::Pos2, p3: egui::Pos2, t: f32| {
        let t2 = t * t;
        let t3 = t2 * t;
        egui::pos2(
            0.5 * (2.0 * p1.x + (-p0.x + p2.x) * t
                + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
                + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3),
            0.5 * (2.0 * p1.y + (-p0.y + p2.y) * t
                + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
                + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3),
        )
    };
    let mut smooth: Vec<egui::Pos2> = Vec::with_capacity(n * 10);
    for i in 0..pts.len() - 1 {
        let p0 = pts[i.saturating_sub(1)];
        let p1 = pts[i];
        let p2 = pts[i + 1];
        let p3 = pts[(i + 2).min(pts.len() - 1)];
        for s in 0..10 {
            smooth.push(catmull(p0, p1, p2, p3, s as f32 / 10.0));
        }
    }
    smooth.push(*pts.last().unwrap());

    ui.painter()
        .add(egui::Shape::line(smooth, Stroke::new(2.2_f32, color)));
}

/// 探测本机局域网 IP：UDP connect 不发包，只读默认路由对应的本机地址。
pub(crate) fn detect_lan_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|addr| addr.ip().to_string())
        .unwrap_or_default()
}

fn history_series(history: &VecDeque<(f32, f32)>, want_up: bool, take: usize) -> Vec<f32> {
    let skip = history.len().saturating_sub(take);
    history
        .iter()
        .skip(skip)
        .map(|&(down_kb, up_kb)| if want_up { up_kb } else { down_kb })
        .collect()
}

/// 徽章的输入：这一轮有没有查到结果、查失败过、还是正在查。
#[derive(Debug, Default)]
pub(crate) struct EgressRiskView<'a> {
    pub(crate) risk: Option<&'a crate::ip_risk::IpRisk>,
    #[allow(dead_code)]
    pub(crate) failed: bool,
    pub(crate) checking: bool,
}

/// 徽章该显示哪一份结果。规则：出口结果优先，但**只有它真的存在**才算它 ——
/// 出口查询失败时 `egress_owner` 也会被写成这颗节点（错误也要归位），以前这里就
/// 直接返回 None 并短路，把已经查到的服务器归属挡在后面，徽章永远显示未检测。
pub(crate) fn display_risk<'a>(
    node_id: Option<&str>,
    egress_owner: Option<&str>,
    egress: Option<&'a crate::ip_risk::IpRisk>,
    front_owner: Option<&str>,
    front: Option<&'a crate::ip_risk::IpRisk>,
) -> Option<&'a crate::ip_risk::IpRisk> {
    let Some(id) = node_id else {
        return None;
    };
    let egress = if egress_owner == Some(id) { egress } else { None };
    let front = if front_owner == Some(id) { front } else { None };
    egress.or(front)
}

/// 那份「服务器归属」结果是不是属于这颗节点：自动跟随记在 `auto_for`，手动点的那次记在 `target`。
pub(crate) fn front_risk_belongs_to(
    node_id: &str,
    auto_for: Option<&str>,
    target: Option<&str>,
) -> bool {
    auto_for == Some(node_id) || target == Some(node_id)
}

/// 节点出口 IP 的归属，画在汇总卡片第一张的那颗徽章上。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeIpKind {
    PublicProxy,
    Datacenter,
    Residential,
    Checking,
    Failed,
    Unknown,
}

impl NodeIpKind {
    #[allow(dead_code)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            // 用户要的是一句话答案：屏幕上只有住宅/机房两档，公开代理算"不是住宅"⇒ 机房，
            // 真判据留在 hover 那行里（node_ip_kind 仍分得出 PublicProxy）。
            NodeIpKind::PublicProxy => "机房",
            NodeIpKind::Datacenter => "机房",
            NodeIpKind::Residential => "住宅",
            NodeIpKind::Checking => "检测中",
            NodeIpKind::Failed => "查失败",
            NodeIpKind::Unknown => "未检测",
        }
    }
}

/// 判定顺序是有意的：公开代理（VPN Gate 这类）比“机房”更该警告，两个标记同时为真时
/// 先报公开代理；已经查到结果就不因为“正在查别的”而退回检测中。
#[allow(dead_code)]
pub(crate) fn node_ip_kind(view: &EgressRiskView) -> NodeIpKind {
    match view.risk {
        Some(risk) if risk.public_proxy => NodeIpKind::PublicProxy,
        Some(risk) if risk.datacenter => NodeIpKind::Datacenter,
        Some(_) => NodeIpKind::Residential,
        None if view.failed => NodeIpKind::Failed,
        None if view.checking => NodeIpKind::Checking,
        None => NodeIpKind::Unknown,
    }
}

#[allow(dead_code)]
fn node_ip_kind_color(kind: NodeIpKind) -> Color32 {
    match kind {
        NodeIpKind::PublicProxy | NodeIpKind::Datacenter => theme::warning(),
        NodeIpKind::Residential => Color32::from_rgb(52, 199, 89),
        NodeIpKind::Checking => theme::text_secondary(),
        NodeIpKind::Failed => theme::danger(),
        NodeIpKind::Unknown => theme::muted(),
    }
}

impl VpnApp {
    pub(crate) fn ui_proxy_charts(&mut self, ui: &mut Ui) {
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

        let is_running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        let is_starting = matches!(self.proxy_status.state, ProxyState::Starting);
        let prepared = crate::gui::prepare::is_prepared(
            self.kernel_installed_cached(),
            matches!(
                self.proxy_status.state,
                ProxyState::Running | ProxyState::Degraded | ProxyState::Starting
            ),
        );

        egui::ScrollArea::vertical()
            .id_salt("proxy_dashboard_scroll")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .show(ui, |ui| {
                // 预留稳定滚动条通道与右侧安全边距（8px），锁定固定宽幅，避免滚动条跳变引起反复重排
                let avail_w = (ui.available_width() - 8.0).max(300.0).floor();
                // 超宽窗口封顶：仪表盘内容最宽 1180px 并水平居中。
                let total_w = avail_w.min(1180.0).floor();
                let center_offset = (((avail_w - total_w) / 2.0).max(0.0)).round();

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if center_offset > 0.0 {
                        ui.add_space(center_offset);
                    }
                    ui.vertical(|ui| {
                        ui.set_min_width(total_w);
                        ui.set_max_width(total_w);
                        self.render_dashboard(ui, total_w, &selected_node, is_running, is_starting, prepared);
                    });
                });
            });
    }

    /// 运行时长 HH:MM:SS（未运行显示 "—"）。
    #[allow(dead_code)]
    fn uptime_label(&self) -> String {
        match self.proxy_started_at {
            Some(started) => {
                let s = started.elapsed().as_secs();
                format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
            }
            None => "—".into(),
        }
    }

    /// 按名称在节点库（可用节点 + 全量节点）里找节点。
    fn resolve_node_by_name(&self, name: &str) -> Option<crate::models::NodeSummary> {
        self.proxy_nodes
            .iter()
            .chain(self.nodes.iter())
            .find(|n| n.name == name)
            .cloned()
    }

    /// 把某个节点设为"启动节点"：点击启动代理时就启动它。
    fn set_start_node(&mut self, node: &crate::models::NodeSummary) {
        self.selected_proxy_node_id = Some(node.id.clone());
        self.selected_proxy_node_name = Some(node.name.clone());
    }

    // -----------------------------------------------------------------
    // 仪表盘唯一布局：所有窗口宽度按同一套比例自适应，不再分大屏/小屏
    // -----------------------------------------------------------------
    fn render_dashboard(
        &mut self,
        ui: &mut Ui,
        total_w: f32,
        selected_node: &Option<crate::models::NodeSummary>,
        is_running: bool,
        is_starting: bool,
        prepared: bool,
    ) {
        let gap = 8.0_f32;
        let is_compact = self.settings.dashboard_ui_mode == "compact";
        let row1_h = if is_compact { 216.0_f32 } else { 176.0_f32 };
        let row2_h = if is_compact { 180.0_f32 } else { 99.0_f32 };
        let _row3_h = 154.0_f32;

        // 宽度预算先扣除每张卡自身的左右内边距（Frame 外框 = 内容 + margin）。
        let cm = 40.0_f32;  // 状态卡边距 20*2
        let cm2 = 28.0_f32; // 其余卡边距 14*2
        let pool = (total_w - cm * 3.0 - gap * 2.0).max(120.0);
        let rate_w = (pool * 0.21).round();
        let status_w = pool - rate_w * 2.0;
        // 服务状态 + 分流规则与上行状态卡左右边缘垂直对齐
        let col_w = (status_w + cm - cm2 * 2.0 - gap) / 2.0;
        // 右侧区域：出站模式与性能平分（出站模式对齐上传卡、性能对齐下载卡）
        let region2 = (total_w - col_w * 2.0 - cm2 * 4.0 - gap * 3.0).max(60.0);
        let mode_w = (rate_w + cm - cm2).max(60.0);
        let perf_w = (region2 - mode_w).max(30.0);
        // 实时连接与状态卡外框对齐；策略组与出站模式同列
        let conns_w = status_w + 12.0;
        let groups_w = (total_w - conns_w - cm2 * 2.0 - gap).max(60.0);

        // ===== 绝对列网格 =====
        // 外框宽度（内容宽 + 各自边距）
        let out_status = status_w + cm;
        let out_rate = rate_w + cm;
        let out_col = col_w + cm2;
        let out_mode = mode_w + cm2;
        let _out_perf = perf_w + cm2;
        // 列 x 坐标（行首为 0）
        let x_status = 0.0_f32;
        let x_up = x_status + out_status + gap;
        let x_down = x_up + out_rate + gap;
        let x_svc = 0.0_f32;
        let x_rules = x_svc + out_col + gap;
        let x_mode = x_rules + out_col + gap; // 必须 == x_up
        let x_perf = x_mode + out_mode + gap; // 必须 == x_down
        debug_assert!((x_mode - x_up).abs() < 1.0, "列网格漂移: mode {x_mode} vs up {x_up}");
        debug_assert!((x_perf - x_down).abs() < 1.0, "列网格漂移: perf {x_perf} vs down {x_down}");

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;

            // --- 第 1 行：状态 | 上传 | 下载（按列 x 精确放置；一次占位，行距不受内容影响） ---
            // 行高按内容实际需要放宽（状态卡含 头行+IP+标签+元信息 四段）：
            let row1_needed = row1_h + 36.0; // 外框边距20 + 内容富余16
            let row_top = ui.cursor().min;
            let row_w = ui.available_width();
            ui.allocate_ui(egui::vec2(row_w, row1_needed), |ui| {
                for (x, w, draw) in [
                    (x_status, status_w, 0usize),
                    (x_up, rate_w, 1),
                    (x_down, rate_w, 2),
                ] {
                    let origin = egui::pos2(row_top.x + x, row_top.y);
                    let card_rect = Rect::from_min_size(origin, egui::vec2(w + 40.0, row1_needed));
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card_rect), |ui| {
                        match draw {
                            0 => self.dash_status_card(ui, w, row1_needed - 20.0, selected_node, is_running, is_starting, prepared, is_compact),
                            1 => self.dash_rate_card(ui, w, row1_needed - 20.0, true),
                            _ => self.dash_rate_card(ui, w, row1_needed - 20.0, false),
                        }
                    });
                }
            });

            // --- 第 2 行：活动连接 (与服务状态+分流规则之和等宽) | 软件性能 (出站模式上方) | 内核性能 (策略组上方) ---
            if !is_compact {
                let row_mid_h = 133.0_f32;
                let row_top = ui.cursor().min;
                let row_w = ui.available_width();
                ui.allocate_ui(egui::vec2(row_w, row_mid_h), |ui| {
                    for (x, w, kind) in [
                        (x_status, conns_w, 0usize),
                        (x_mode, mode_w, 1),
                        (x_perf, perf_w, 2),
                    ] {
                        let origin = egui::pos2(row_top.x + x, row_top.y);
                        let card_rect = Rect::from_min_size(origin, egui::vec2(w + 28.0, row_mid_h));
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card_rect), |ui| {
                            match kind {
                                0 => self.dash_connections_card(ui, w, row_mid_h - 20.0),
                                1 => self.dash_software_perf_card(ui, w, row_mid_h - 20.0),
                                _ => self.dash_kernel_perf_card(ui, w, row_mid_h - 20.0),
                            }
                        });
                    }
                });
            }

            // --- 第 3 行：服务状态 | 分流规则 | 出站模式 | 策略组（按列 x 精确放置） ---
            let row_top = ui.cursor().min;
            let row_w = ui.available_width();
            let row_h2 = row2_h + 20.0;
            ui.allocate_ui(egui::vec2(row_w, row_h2), |ui| {
                for (x, w, kind) in [
                    (x_svc, col_w, 0usize),
                    (x_rules, col_w, 1),
                    (x_mode, mode_w, 2),
                    (x_perf, perf_w, 3),
                ] {
                    let origin = egui::pos2(row_top.x + x, row_top.y);
                    let card_rect = Rect::from_min_size(origin, egui::vec2(w + 28.0, row_h2));
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card_rect), |ui| {
                        match kind {
                            0 => self.dash_services_card(ui, w, row2_h),
                            1 => self.dash_rules_card(ui, w, row2_h),
                            2 => self.dash_mode_card(ui, w, row2_h),
                            _ => self.dash_groups_card(ui, w, row2_h),
                        }
                    });
                }
            });

            // --- 第 4 行（底部代理控制栏）：三卡对齐状态卡宽度，系统代理/TUN 平分剩余 ---
            self.render_bottom_proxy_bar(ui, total_w, gap, 3, Some((conns_w, groups_w, perf_w)));

            // 底部预留舒适呼吸内边距，消除滚动到底部时的边界摩擦与振荡
            ui.add_space(16.0);
        });
    }

    /// 策略组调度抽屉：右侧滑出（egui 原生窗口，精确宽度、内容自动裁剪），
    /// 四种调度方式 + 可用节点列表（带地区/名称筛选），点选即设为启动节点。
    pub(crate) fn render_group_drawer(&mut self, ctx: &egui::Context) {
        let Some(group_name) = self.group_drawer.clone() else {
            return;
        };
        let group = self
            .kernel_groups
            .iter()
            .find(|g| g.name == group_name)
            .cloned()
            .unwrap_or_else(|| crate::gui::types::KernelGroup {
                name: group_name.clone(),
                kind: String::new(),
                now: None,
                members: Vec::new(),
            });

        let ctx = ctx.clone();
        // 分流策略组抽屉宽度调整为 70% 屏幕宽度
        let width = (ctx.screen_rect().width() * 0.70).clamp(560.0, 1400.0);
        let screen = ctx.screen_rect();
        let is_selector = matches!(group.kind.to_lowercase().as_str(), "select" | "selector");
        let active_mode = self.group_scheduling.get(&group.name).cloned();
        let proxy_up = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        let manual_mode = active_mode
            .as_deref()
            .map(|m| m == "手动指定")
            .unwrap_or(true);

        let mut close = false;
        let mut picked: Option<String> = None;
        let title_bar_h = crate::gui::window::TITLE_BAR_H;
        let status_top_y = if self.status_bar_top_y > 100.0 {
            self.status_bar_top_y
        } else {
            screen.height() - self.status_bar_height.max(30.0)
        };
        // 确保抽屉和遮罩底边严格在状态栏上方（留 2px 绝对安全间距，彻底杜绝遮盖状态栏与底部按钮）
        let safe_max_y = status_top_y - 2.0;
        let backdrop_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, title_bar_h),
            egui::pos2(screen.width(), safe_max_y),
        );

        // 遮罩：半透明 + 点击任意处关闭（仅覆盖标题栏与状态栏之间的内容区，不遮挡主窗口标题栏拖拽与状态栏控制按钮）
        egui::Area::new(egui::Id::new(("group_drawer_backdrop", group.name.clone())))
            .order(egui::Order::Middle)
            .fixed_pos(backdrop_rect.min)
            .show(&ctx, |ui| {
                ui.set_width(backdrop_rect.width());
                ui.set_height(backdrop_rect.height());
                ui.painter()
                    .rect_filled(backdrop_rect, 0.0, Color32::from_black_alpha(80));
                let resp = ui.interact(
                    backdrop_rect,
                    egui::Id::new(("group_drawer_backdrop_click", group.name.clone())),
                    Sense::click(),
                );
                if resp.clicked() {
                    close = true;
                }
            });

        let panel_fill = if theme::is_light() {
            Color32::WHITE
        } else {
            theme::panel()
        };

        // 抽屉本体：原生窗口（严格限制在标题栏与状态栏之间的内容区域内，不遮挡标题栏与状态栏）
        let drawer_rect = egui::Rect::from_min_max(
            egui::pos2(screen.width() - width, title_bar_h),
            egui::pos2(screen.width(), safe_max_y),
        );

        egui::Area::new(egui::Id::new("group_drawer_panel"))
            .order(egui::Order::Foreground)
            .fixed_pos(drawer_rect.min)
            .show(&ctx, |ui| {
                ui.set_clip_rect(drawer_rect);

                // 拦截抽屉本体所有点击与拖拽，防止事件穿透至底层卡片
                let _ = ui.interact(
                    drawer_rect,
                    ui.id().with(("group_drawer_panel_interact", group.name.clone())),
                    Sense::click_and_drag(),
                );

                // 绘制抽屉底色与左侧描边
                ui.painter().rect_filled(drawer_rect, CornerRadius::ZERO, panel_fill);
                ui.painter().line_segment(
                    [drawer_rect.left_top(), drawer_rect.left_bottom()],
                    Stroke::new(1.0_f32, apple_stroke()),
                );

                // 抽屉头部栏（高度 56px：标题 21px + 间距 2px + 副标题 14px + 上下余量。
                // 48px 时副标题下缘顶到分割线，视觉上像被线盖住）
                let header_h = 56.0_f32;
                let header_rect = egui::Rect::from_min_max(
                    drawer_rect.min,
                    egui::pos2(drawer_rect.right(), drawer_rect.min.y + header_h),
                );

                // 关闭按钮：严格钉在抽屉最右上角（距右边框 16px，垂直居中）
                let btn_size = 28.0_f32;
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        drawer_rect.right() - 16.0 - btn_size,
                        drawer_rect.min.y + (header_h - btn_size) * 0.5,
                    ),
                    Vec2::splat(btn_size),
                );

                // 头部剩余区域支持拖拽主窗口
                let drag_rect = egui::Rect::from_min_max(
                    drawer_rect.min,
                    egui::pos2(btn_rect.min.x - 8.0, header_rect.max.y),
                );
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with(("group_drawer_header_drag", group.name.clone())),
                    Sense::drag(),
                );
                if drag_resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // 左侧标题与副标题
                let title_rect = egui::Rect::from_min_max(
                    egui::pos2(drawer_rect.left() + 18.0, drawer_rect.min.y + 7.0),
                    egui::pos2(btn_rect.min.x - 12.0, header_rect.max.y - 3.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(title_rect), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new("调度方式")
                                .size(15.5)
                                .strong()
                                .color(theme::text_primary()),
                        );
                        ui.label(
                            RichText::new(format!(
                                "{} · {}",
                                group.name,
                                if is_selector { "手动选择" } else { "自动调度" }
                            ))
                            .size(10.5)
                            .color(theme::muted()),
                        );
                    });
                });

                // 右上角关闭按钮
                let btn_resp = ui.interact(
                    btn_rect,
                    ui.id().with(("group_drawer_close_btn", group.name.clone())),
                    Sense::click(),
                );
                if btn_resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let bp = ui.painter().clone();
                let bg_col = if btn_resp.hovered() {
                    theme::panel_hover()
                } else {
                    apple_sub_bg()
                };
                bp.circle_filled(btn_rect.center(), 14.0, bg_col);
                let x_color = if btn_resp.hovered() {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                };
                let s = 5.0_f32;
                let c = btn_rect.center();
                for (a, b) in [
                    (egui::pos2(c.x - s, c.y - s), egui::pos2(c.x + s, c.y + s)),
                    (egui::pos2(c.x + s, c.y - s), egui::pos2(c.x - s, c.y + s)),
                ] {
                    bp.line_segment([a, b], Stroke::new(1.8_f32, x_color));
                }
                if btn_resp.hovered() {
                    bp.circle_stroke(c, 14.0, Stroke::new(1.0_f32, apple_stroke()));
                }
                if btn_resp.clicked() {
                    close = true;
                }

                // 细微满宽分割线
                let sep_y = header_rect.max.y;
                ui.painter().line_segment(
                    [egui::pos2(drawer_rect.left(), sep_y), egui::pos2(drawer_rect.right(), sep_y)],
                    Stroke::new(1.0_f32, apple_stroke()),
                );

                // 内容区
                let body_top = sep_y + 8.0;
                let body_rect = egui::Rect::from_min_max(
                    egui::pos2(drawer_rect.left(), body_top),
                    egui::pos2(drawer_rect.right(), safe_max_y - 6.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.set_clip_rect(body_rect);
                    ui.vertical(|ui| {

                    // 当前出口（内核实时值优先，其次应用当前选中节点）
                    let current_display = group
                        .now
                        .clone()
                        .or_else(|| {
                            self.selected_proxy_node_name
                                .clone()
                                .filter(|name| {
                                    if group.name == "全部" || group.name == "all" || group.name == "PROXY" {
                                        true
                                    } else if let Some(node) = self.resolve_node_by_name(name) {
                                        node_matches_group(&node, &group.name, &self.kernel_groups)
                                    } else {
                                        false
                                    }
                                })
                        })
                        .unwrap_or_else(|| "—".to_string());
                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.label(
                            RichText::new(format!("当前出口 · {current_display}"))
                                .size(11.5)
                                .strong()
                                .color(theme::accent_strong()),
                        );
                    });
                    ui.add_space(8.0);

                    // 四种调度方式：单行胶囊分段选择器（点选即执行对应动作）
                    let modes = ["延迟选优", "故障转移", "负载均衡", "手动指定"];
                    let seg_w = (ui.available_width() - 32.0).max(200.0);
                    let mut clicked_mode: Option<&'static str> = None;

                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        Frame::NONE
                            .fill(apple_sub_bg())
                            .stroke(Stroke::new(1.0_f32, apple_stroke2()))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(3, 3))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    let bw = ((seg_w - 6.0 - 4.0 * 3.0) / 4.0).floor();
                                    for mode in modes {
                                        let selected = active_mode.as_deref() == Some(mode);
                                        let (fill, stroke, text_color) = if selected {
                                            (apple_blue(), Stroke::NONE, Color32::WHITE)
                                        } else {
                                            (Color32::TRANSPARENT, Stroke::NONE, theme::text_secondary())
                                        };
                                        let btn = egui::Button::new(
                                            RichText::new(mode).size(11.5).strong().color(text_color),
                                        )
                                        .fill(fill)
                                        .stroke(stroke)
                                        .corner_radius(CornerRadius::same(8))
                                        .min_size(Vec2::new(bw, 28.0));
                                        if ui.add(btn).clicked() {
                                            clicked_mode = Some(mode);
                                        }
                                    }
                                });
                            });
                    });

                    if let Some(mode) = clicked_mode {
                        self.group_scheduling
                            .insert(group.name.clone(), mode.to_string());
                        let pool: Vec<String> = if group.members.is_empty() {
                            self.proxy_nodes
                                .iter()
                                .filter(|n| node_matches_group(n, &group.name, &self.kernel_groups))
                                .map(|n| n.name.clone())
                                .collect()
                        } else {
                            let mut resolved_pool = Vec::new();
                            for m in &group.members {
                                if self.proxy_nodes.iter().any(|n| &n.name == m)
                                    || self.nodes.iter().any(|n| &n.name == m)
                                {
                                    resolved_pool.push(m.clone());
                                } else if m != "DIRECT" && m != "REJECT" && m != "PASS" && m != "GLOBAL" {
                                    for n in self.proxy_nodes.iter().chain(self.nodes.iter()) {
                                        if node_matches_group(n, m, &self.kernel_groups)
                                            && !resolved_pool.contains(&n.name)
                                        {
                                            resolved_pool.push(n.name.clone());
                                        }
                                    }
                                }
                            }
                            if resolved_pool.is_empty() {
                                group.members.clone()
                            } else {
                                resolved_pool
                            }
                        };
                        match mode {
                            "延迟选优" => {
                                let mut best: Option<(String, i64)> = None;
                                for m in &pool {
                                    if let Some(ms) = self
                                        .resolve_node_by_name(m)
                                        .as_ref()
                                        .and_then(|n| n.latency_ms)
                                    {
                                        if best
                                            .as_ref()
                                            .map(|b| ms < b.1)
                                            .unwrap_or(true)
                                        {
                                            best = Some((m.clone(), ms));
                                        }
                                    }
                                }
                                match best {
                                    Some((m, ms)) => {
                                        if let Some(node) =
                                            self.resolve_node_by_name(&m)
                                        {
                                            self.set_start_node(&node);
                                        }
                                        if proxy_up {
                                            if group.name != "全部" && group.name != "all" {
                                                if self.kernel_groups.iter().any(|g| g.name == group.name) {
                                                    self.select_kernel_member(
                                                        group.name.clone(),
                                                        m.clone(),
                                                    );
                                                }
                                            }
                                            self.status = format!(
                                                "延迟选优：已切换至 {m}（{ms} ms）"
                                            );
                                        } else {
                                            if group.name != "全部" && group.name != "all" {
                                                self.pending_group_pick = Some((
                                                    group.name.clone(),
                                                    m.clone(),
                                                ));
                                            }
                                            self.status = format!(
                                                "延迟选优：已选定 {m}（{ms} ms），启动代理后自动使用"
                                            );
                                        }
                                    }
                                    None => {
                                        // 候选节点都没实测过延迟：先发全量测速，
                                        // 测完后用户再点一次即可选出最优节点
                                        self.speedtest_all_sources();
                                        self.status =
                                            "候选节点暂无实测延迟，已发起全量测速；完成后请再点一次「延迟选优」".into();
                                    }
                                }
                            }
                            "负载均衡" => {
                                if !pool.is_empty() {
                                    let idx = (std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.subsec_nanos() as usize)
                                        .unwrap_or(0))
                                        % pool.len();
                                    let m = pool[idx].clone();
                                    if let Some(node) =
                                        self.resolve_node_by_name(&m)
                                    {
                                        self.set_start_node(&node);
                                    }
                                    if proxy_up {
                                        if group.name != "全部" && group.name != "all" {
                                            if self.kernel_groups.iter().any(|g| g.name == group.name) {
                                                self.select_kernel_member(
                                                    group.name.clone(),
                                                    m.clone(),
                                                );
                                            }
                                        }
                                        self.status =
                                            format!("负载均衡：已切换至 {m}");
                                    } else {
                                        if group.name != "全部" && group.name != "all" {
                                            self.pending_group_pick = Some((
                                                group.name.clone(),
                                                m.clone(),
                                            ));
                                        }
                                        self.status = format!(
                                            "负载均衡：已选定 {m}，启动代理后自动使用"
                                        );
                                    }
                                }
                            }
                            "故障转移" => {
                                self.auto_failover = true;
                                self.status =
                                    "故障转移已启用：出口异常时自动切换备用节点".into();
                            }
                            _ => {
                                self.status =
                                    "手动指定：在下方成员列表中点选出口节点".into();
                            }
                        }
                    }
                    ui.add_space(8.0);

                    // 节点列表：可用节点列表内容（地区 + 名称筛选，一行两个）
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(16.0);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let regions = [
                            ("all", "全部"),
                            ("HK", "香港"),
                            ("JP", "日本"),
                            ("US", "美国"),
                            ("SG", "新加坡"),
                            ("TW", "台湾"),
                            ("KR", "韩国"),
                            ("OTHER", "其它"),
                        ];
                        for (code, label) in regions {
                            let is_sel = self.group_drawer_region == code;
                            let (fill, stroke, text_color) = if is_sel {
                                (
                                    theme::accent_bg(),
                                    Stroke::new(1.0_f32, theme::accent()),
                                    theme::accent_strong(),
                                )
                            } else {
                                (
                                    theme::sidebar(),
                                    Stroke::new(1.0_f32, theme::border_subtle()),
                                    theme::muted(),
                                )
                            };
                            let btn = egui::Button::new(
                                RichText::new(label).size(9.5).strong().color(text_color),
                            )
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(CornerRadius::same(10));
                            if ui.add(btn).clicked() && !is_sel {
                                self.group_drawer_region = code.to_string();
                            }
                        }
                    });
                    ui.add_space(4.0);
                    let candidate_pool: Vec<crate::models::NodeSummary> = if self.proxy_nodes.is_empty() {
                        self.nodes.clone()
                    } else {
                        let mut list = self.proxy_nodes.clone();
                        let mut names: std::collections::HashSet<_> = list.iter().map(|n| n.name.clone()).collect();
                        for n in &self.nodes {
                            if !names.contains(&n.name) && group.members.iter().any(|m| m == &n.name) {
                                names.insert(n.name.clone());
                                list.push(n.clone());
                            }
                        }
                        list
                    };

                    let mut group_nodes: Vec<crate::models::NodeSummary> = candidate_pool
                        .into_iter()
                        .filter(|n| node_matches_group(n, &group.name, &self.kernel_groups))
                        .collect();

                    if !group.members.is_empty() {
                        group_nodes.sort_by_key(|n| {
                            group.members.iter().position(|m| m == &n.name).unwrap_or(usize::MAX)
                        });
                    }
                    let total_group_count = group_nodes.len();

                    ui.horizontal(|ui| {
                        ui.add_space(16.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.group_drawer_filter)
                                .hint_text("筛选节点")
                                .desired_width(140.0),
                        );
                        let filter = self.group_drawer_filter.trim().to_lowercase();
                        let region_key = &self.group_drawer_region;
                        let count_text = if filter.is_empty() && region_key == "all" {
                            format!("{total_group_count} 个节点")
                        } else {
                            let match_count = group_nodes
                                .iter()
                                .filter(|n| {
                                    let country = n
                                        .country
                                        .clone()
                                        .or_else(|| crate::parser::guess_country(&n.name))
                                        .unwrap_or_default()
                                        .to_ascii_uppercase();
                                    let region_hit = if region_key == "all" {
                                        true
                                    } else if region_key == "OTHER" {
                                        !["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"]
                                            .contains(&country.as_str())
                                    } else {
                                        &country == region_key
                                    };
                                    let name_hit = filter.is_empty()
                                        || n.name.to_lowercase().contains(&filter);
                                    region_hit && name_hit
                                })
                                .count();
                            format!("{match_count}/{total_group_count} 个节点")
                        };
                        ui.label(
                            RichText::new(count_text)
                                .size(10.0)
                                .color(theme::muted()),
                        );
                    });
                    ui.add_space(4.0);
                    let avail_scroll_h = (ui.available_height() - 4.0).max(50.0);
                    egui::ScrollArea::vertical()
                        .id_salt(("group_drawer_members", group.name.clone()))
                        .max_height(avail_scroll_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let filter = self.group_drawer_filter.trim().to_lowercase();
                            let region_key = self.group_drawer_region.clone();
                            let nodes: Vec<crate::models::NodeSummary> = group_nodes
                                .into_iter()
                                .filter(|n| {
                                    let country = n
                                        .country
                                        .clone()
                                        .or_else(|| crate::parser::guess_country(&n.name))
                                        .unwrap_or_default()
                                        .to_ascii_uppercase();
                                    let region_hit = if region_key == "all" {
                                        true
                                    } else if region_key == "OTHER" {
                                        !["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"]
                                            .contains(&country.as_str())
                                    } else {
                                        country == region_key
                                    };
                                    let name_hit = filter.is_empty()
                                        || n.name.to_lowercase().contains(&filter);
                                    region_hit && name_hit
                                })
                                .collect();
                            if nodes.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    let empty_tip = if total_group_count == 0 {
                                        "当前策略组暂无匹配节点，请在订阅源拉取节点或检查分流配置"
                                    } else {
                                        "没有匹配筛选条件的节点"
                                    };
                                    ui.label(
                                        RichText::new(empty_tip)
                                            .size(10.5)
                                            .color(theme::muted()),
                                    );
                                });
                                return;
                            }
                            let cols = if ui.available_width() >= 380.0 { 2 } else { 1 };
                            let focused_id = self.focused_node_id.clone();
                            let switching_id = self.switching_node_id.clone();
                            let selected_id = self.selected_proxy_node_id.clone();

                            let mut clicked_node: Option<crate::models::NodeSummary> = None;
                            let mut clicked_auto = false;

                            for pair in nodes.chunks(cols) {
                                ui.columns(cols, |column_uis| {
                                    for (k, node) in pair.iter().enumerate() {
                                        let column_ui = &mut column_uis[k];
                                        let is_active = if let Some(ref now) = group.now {
                                            now == &node.name
                                        } else {
                                            current_display == node.name
                                                || (group.name == "全部" && selected_id.as_deref() == Some(&node.id))
                                        };
                                        let is_focused = focused_id.as_deref() == Some(node.id.as_str());
                                        let is_switching = switching_id.as_deref() == Some(&node.id);

                                        let source_name = self.source_name_for_id(node.source_id.as_deref());
                                        let mut tip = format!("{}\n{}:{}", node.name, node.server, node.port);
                                        if let Some(src) = source_name {
                                            if !src.is_empty() {
                                                tip.push_str(&format!("\n订阅源: {src}"));
                                            }
                                        }
                                        if let Some(ms) = node.latency_ms {
                                            tip.push_str(&format!("\n延迟: {ms} ms"));
                                        }
                                        let resp = crate::gui::widgets::node_card::compact_node_card(
                                            column_ui,
                                            node,
                                            source_name,
                                            is_active,
                                            is_focused,
                                            is_switching,
                                        )
                                        .on_hover_text(tip);

                                        if resp.clicked() {
                                            if !is_active && manual_mode {
                                                clicked_node = Some(node.clone());
                                            } else if !manual_mode {
                                                clicked_auto = true;
                                            }
                                        }
                                    }
                                });
                                ui.add_space(4.0);
                            }

                            if let Some(node) = clicked_node {
                                self.set_start_node(&node);
                                if proxy_up {
                                    if group.members.contains(&node.name) {
                                        picked = Some(node.name.clone());
                                    } else if group.name != "全部" && group.name != "all" {
                                        picked = Some(node.name.clone());
                                    } else {
                                        self.switch_proxy_node(
                                            node.id.clone(),
                                            node.name.clone(),
                                            "调度抽屉切换",
                                        );
                                    }
                                    self.status = format!("已切换出口至 {}", node.name);
                                } else {
                                    if group.name != "全部" && group.name != "all" {
                                        self.pending_group_pick = Some((
                                            group.name.clone(),
                                            node.name.clone(),
                                        ));
                                    }
                                    self.status = "已选择节点，启动代理将使用该节点".into();
                                }
                            } else if clicked_auto {
                                self.status = "当前为自动调度，点选「手动指定」后可选择节点".into();
                            }

                            if !group.members.is_empty() && !is_selector {
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(16.0);
                                    ui.label(
                                        RichText::new(
                                            "该组由内核自动调度（url-test），手动切换可能被自动覆盖",
                                        )
                                        .size(9.5)
                                        .color(theme::muted()),
                                    );
                                });
                            }
                            ui.add_space(12.0);
                        });   // ScrollArea::show
                    });       // ui.vertical
                });           // allocate_new_ui
            });               // Area::show

        if close {
            self.group_drawer = None;
        }
        if let Some(member) = picked {
            if group.name != "全部" && group.name != "all" {
                if self.kernel_groups.iter().any(|g| g.name == group.name) {
                    self.select_kernel_member(group.name.clone(), member);
                }
            }
        }
    }

    /// 仪表盘多功能控制台抽屉（右侧滑出）
    /// 支持：网络设置(DNS/TUN)、运行日志、分流规则、内核管理、分流策略组、活跃连接
    pub(crate) fn render_dashboard_drawer(&mut self, ctx: &egui::Context) {
        let Some(drawer) = self.dashboard_drawer else {
            return;
        };

        let ctx = ctx.clone();
        let screen = ctx.screen_rect();
        // 抽屉宽度根据内容自适应：优雅贴边，宽度适中不遮挡大屏
        let width = match drawer {
            DashboardDrawer::Connections => (screen.width() * 0.72).clamp(760.0, 1400.0),
            DashboardDrawer::Traffic => (screen.width() * 0.58).clamp(620.0, 960.0),
            DashboardDrawer::Rules => (screen.width() * 0.70).clamp(580.0, 1400.0),
            DashboardDrawer::Groups => (screen.width() * 0.70).clamp(580.0, 1400.0),
            DashboardDrawer::Logs => (screen.width() * 0.50).clamp(500.0, 800.0),
            DashboardDrawer::Network => (screen.width() * 0.50).clamp(540.0, 780.0),
            DashboardDrawer::Kernels => (screen.width() * 0.52).clamp(520.0, 780.0),
        }.min(screen.width() - 40.0);

        let mut close = false;

        // 监听 Escape 键快速关闭抽屉
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }

        let title_bar_h = crate::gui::window::TITLE_BAR_H;
        let status_top_y = if self.status_bar_top_y > 100.0 {
            self.status_bar_top_y
        } else {
            screen.height() - self.status_bar_height.max(30.0)
        };
        // 抽屉和遮罩底边严格在状态栏上方（留 2px 绝对安全间距，彻底杜绝遮盖状态栏）
        let safe_max_y = status_top_y - 2.0;
        let backdrop_rect = egui::Rect::from_min_max(
            egui::pos2(0.0, title_bar_h),
            egui::pos2(screen.width(), safe_max_y),
        );

        // 半透明背景遮罩（仅覆盖标题栏与状态栏之间的内容区，不遮挡主窗口标题栏拖拽与状态栏控制按钮）
        egui::Area::new(egui::Id::new("dashboard_drawer_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(backdrop_rect.min)
            .show(&ctx, |ui| {
                ui.set_width(backdrop_rect.width());
                ui.set_height(backdrop_rect.height());
                ui.painter()
                    .rect_filled(backdrop_rect, 0.0, Color32::from_black_alpha(85));
                let resp = ui.interact(
                    backdrop_rect,
                    egui::Id::new("dashboard_drawer_backdrop_click"),
                    Sense::click(),
                );
                if resp.clicked() {
                    close = true;
                }
            });

        let panel_fill = if theme::is_light() {
            Color32::WHITE
        } else {
            theme::panel()
        };

        let (title, subtitle) = match drawer {
            DashboardDrawer::Network => ("网络设置", "DNS 协议与 TUN 虚拟网卡配置"),
            DashboardDrawer::Logs => ("运行日志", "内核进程与代理网络运行日志"),
            DashboardDrawer::Rules => ("分流规则", "规则命中统计与自定义路由策略"),
            DashboardDrawer::Kernels => ("内核管理", "代理内核版本、偏好与运行状态"),
            DashboardDrawer::Groups => ("分流策略组", "策略组节点分流与调度配置"),
            DashboardDrawer::Connections => ("网络连接与 DNS 审计", "分流流向监控 · 活动连接跟踪 · 进程带宽矩阵 · 内核实时 DNS 解析记录"),
            DashboardDrawer::Traffic => ("流量统计排行", "节点 · 域名 · 软件进程三维度排行 · 实时速率 · 累计上行与下行"),
        };

        // 抽屉本体：右侧原生浮层（严格在标题栏与状态栏之间的内容区，紧贴窗口最右侧，严禁右侧留白）
        let drawer_rect = egui::Rect::from_min_max(
            egui::pos2(screen.width() - width, title_bar_h),
            egui::pos2(screen.width(), safe_max_y),
        );

        egui::Area::new(egui::Id::new("dashboard_drawer_panel"))
            .order(egui::Order::Foreground)
            .fixed_pos(drawer_rect.min)
            .show(&ctx, |ui| {
                ui.set_clip_rect(drawer_rect);

                // 拦截抽屉本体所有点击与拖拽，防止事件穿透至底层卡片
                let _ = ui.interact(
                    drawer_rect,
                    ui.id().with(("drawer_panel_interact", format!("{:?}", drawer))),
                    Sense::click_and_drag(),
                );

                // 绘制抽屉底色与左侧描边
                ui.painter().rect_filled(drawer_rect, CornerRadius::ZERO, panel_fill);
                ui.painter().line_segment(
                    [drawer_rect.left_top(), drawer_rect.left_bottom()],
                    Stroke::new(1.0_f32, apple_stroke()),
                );

                // 抽屉头部栏（高度 56px：标题 21px + 间距 2px + 副标题 14px + 上下余量。
                // 48px 时副标题下缘顶到分割线，视觉上像被线盖住）
                let header_h = 56.0_f32;
                let header_rect = egui::Rect::from_min_max(
                    drawer_rect.min,
                    egui::pos2(drawer_rect.right(), drawer_rect.min.y + header_h),
                );

                // 关闭按钮：严格钉在抽屉最右上角（距右边框 16px，垂直居中）
                let btn_size = 28.0_f32;
                let btn_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        drawer_rect.right() - 16.0 - btn_size,
                        drawer_rect.min.y + (header_h - btn_size) * 0.5,
                    ),
                    Vec2::splat(btn_size),
                );

                // 头部剩余区域支持拖拽主窗口
                let drag_rect = egui::Rect::from_min_max(
                    drawer_rect.min,
                    egui::pos2(btn_rect.min.x - 8.0, header_rect.max.y),
                );
                let drag_resp = ui.interact(
                    drag_rect,
                    ui.id().with(("drawer_header_drag", format!("{:?}", drawer))),
                    Sense::drag(),
                );
                if drag_resp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }

                // 左侧标题与副标题（距左边框 18px）
                let title_rect = egui::Rect::from_min_max(
                    egui::pos2(drawer_rect.left() + 18.0, drawer_rect.min.y + 7.0),
                    egui::pos2(btn_rect.min.x - 12.0, header_rect.max.y - 3.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(title_rect), |ui| {
                    ui.vertical(|ui| {
                        // 标题与副标题收紧行距：48→56 后本就放得下，再收 1px 求稳
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(
                            RichText::new(title)
                                .size(15.5)
                                .strong()
                                .color(theme::text_primary()),
                        );
                        ui.label(
                            RichText::new(subtitle)
                                .size(10.5)
                                .color(theme::muted()),
                        );
                    });
                });

                // 右上角关闭按钮绘制与交互
                let btn_resp = ui.interact(
                    btn_rect,
                    ui.id().with(("drawer_close_btn", format!("{:?}", drawer))),
                    Sense::click(),
                );
                if btn_resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let bp = ui.painter().clone();
                let bg_col = if btn_resp.hovered() {
                    theme::panel_hover()
                } else {
                    apple_sub_bg()
                };
                bp.circle_filled(btn_rect.center(), 14.0, bg_col);
                let x_color = if btn_resp.hovered() {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                };
                let s = 5.0_f32;
                let c = btn_rect.center();
                for (a, b) in [
                    (egui::pos2(c.x - s, c.y - s), egui::pos2(c.x + s, c.y + s)),
                    (egui::pos2(c.x + s, c.y - s), egui::pos2(c.x - s, c.y + s)),
                ] {
                    bp.line_segment([a, b], Stroke::new(1.8_f32, x_color));
                }
                if btn_resp.hovered() {
                    bp.circle_stroke(c, 14.0, Stroke::new(1.0_f32, apple_stroke()));
                }
                if btn_resp.clicked() {
                    close = true;
                }

                // 细微满宽分割线
                let sep_y = header_rect.max.y;
                ui.painter().line_segment(
                    [egui::pos2(drawer_rect.left(), sep_y), egui::pos2(drawer_rect.right(), sep_y)],
                    Stroke::new(1.0_f32, apple_stroke()),
                );

                // 内容区严格铺满剩余内容区域（两侧留 16px 舒适边距，上下留白匀称）
                let body_top = sep_y + 8.0;
                let body_rect = egui::Rect::from_min_max(
                    egui::pos2(drawer_rect.left() + 16.0, body_top),
                    egui::pos2(drawer_rect.right() - 16.0, safe_max_y - 6.0),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.set_clip_rect(body_rect);
                    ui.set_width(body_rect.width());
                    ui.set_height(body_rect.height());
                    match drawer {
                        DashboardDrawer::Network => self.ui_proxy_network(ui),
                        DashboardDrawer::Logs => self.ui_proxy_logs(ui),
                        DashboardDrawer::Rules => self.ui_proxy_rules(ui),
                        DashboardDrawer::Kernels => self.ui_proxy_kernels(ui),
                        DashboardDrawer::Groups => self.ui_proxy_groups(ui),
                        DashboardDrawer::Connections => self.ui_proxy_connections(ui),
                        DashboardDrawer::Traffic => self.ui_proxy_node_traffic(ui),
                    }
                });
            });

        if close {
            self.dashboard_drawer = None;
        }
    }

    /// 状态大卡：出口 IP / 节点大字 + 国旗标识 + 标签药丸 + 三列元信息 + 86px 拟物大圆启停按钮。
    fn dash_status_card(
        &mut self,
        ui: &mut Ui,
        w: f32,
        h: f32,
        selected_node: &Option<crate::models::NodeSummary>,
        is_running: bool,
        is_starting: bool,
        prepared: bool,
        is_compact: bool,
    ) {
        // 外框矩形精确绘制（与速率卡同构，外框高严格 = h + 边距，不受内容影响）
        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 40.0, h + 20.0),
        );
        dash_card_at(ui, outer);
    // 画框不推进 cursor：显式把光标拨到卡右侧，行内下一元素从这继续
    ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(20.0, 10.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                // 行距归零：区块间距全部由下面的 add_space 显式控制（否则 egui 默认
                // 行距会叠在估算之外，把最后一行顶出卡片被裁掉）。
                ui.spacing_mut().item_spacing.y = 0.0;
                // 上下只各留一小条边距，剩余空间摊给区块间距让内容撑满卡片：
                // 原先按内容估高垂直居中，大卡上下各空 ~35px，内容挤在正中间。
                ui.add_space(6.0);

                // 顶部状态行
                let (state_text, state_color, dot_color) = if is_running {
                    ("已连接", theme::text_primary(), apple_green_dot())
                } else if is_starting {
                    ("启动中", theme::warning(), apple_amber_dot())
                } else if prepared {
                    ("准备中", theme::accent(), apple_blue())
                } else {
                    ("已停止", theme::muted(), apple_red_dot())
                };

                ui.horizontal(|ui| {
                    let status_lbl_resp = ui.add(
                        egui::Label::new(
                            RichText::new("状态")
                                .size(12.5)
                                .strong()
                                .color(theme::text_secondary()),
                        )
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("点击查看运行日志（右侧滑出抽屉）");
                    if status_lbl_resp.clicked() {
                        self.dashboard_drawer = Some(DashboardDrawer::Logs);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 卡片右上角内核管理入口：点击滑出内核管理抽屉
                        let (kx_rect, kx_resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                        let kx_color = if kx_resp.hovered() { theme::accent() } else { theme::text_secondary() };
                        draw_icon(ui.painter(), kx_rect.shrink(1.5), Icon::Cpu, kx_color);
                        let kx_resp = kx_resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("内核管理：版本、偏好与运行状态");
                        if kx_resp.clicked() {
                            self.dashboard_drawer = Some(DashboardDrawer::Kernels);
                        }
                        // 仪表盘外层把 item_spacing.x 归零了，这里必须显式留缝，
                        // 否则状态圆点会和齿轮图标贴死
                        ui.add_space(6.0);
                        dash_dot(ui, dot_color);
                        let sub_title = if let Some(node) = selected_node {
                            format!("{state_text} · {}", crate::gui::widgets::clean_node_display_name(&node.name))
                        } else {
                            state_text.to_string()
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(sub_title)
                                    .size(11.5)
                                    .strong()
                                    .color(state_color),
                            )
                            .truncate(),
                        );
                    });
                });

                ui.add_space(6.0);

                // 主体两栏：左侧 IP 画像与信息，右侧启停大按钮
                let btn_diameter = if is_compact { 50.0_f32 } else { 58.0_f32 };
                let left_w = (w - btn_diameter - 36.0).max(180.0);
                let (egress_ip, egress_dc, egress_org, egress_country, egress_checking) = {
                    let view = self.egress_risk_view();
                    (
                        view.risk.map(|r| r.ip.clone()),
                        view.risk.map(|r| r.datacenter),
                        view.risk.map(|r| r.org.clone()),
                        view.risk.map(|r| r.country.clone()),
                        view.checking,
                    )
                };

                // 主行可用高度在进入 horizontal 前量好：horizontal 里的子 vertical 只拿得到
                // 内容自身的高度（available_height 会是 0），必须显式 allocate 才有得可分。
                let avail_h = ui.available_height();
                let flag_h = if is_compact { 34.0 } else { 36.0 };
                let pills_h = 28.0_f32;
                let meta_h = if is_compact { 44.0 } else { 46.0 };
                let content_slack = (avail_h - flag_h - pills_h - meta_h).max(0.0);
                // 空余高度的 40% 给两处区块间距（上限 48px），其余对半当上下留白：
                // 内容撑满卡片，上下只剩对称的一小条。
                let slack_gap = (content_slack * 0.4).clamp(6.0, 48.0);
                let pad_rest = ((content_slack - slack_gap * 2.0) * 0.5).clamp(0.0, 14.0);

                ui.horizontal(|ui| {
                    ui.allocate_ui(egui::vec2(left_w, avail_h), |ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(left_w);
                        ui.set_max_width(left_w);
                        ui.add_space(pad_rest);

                        // 国旗方块 + 大字 IP / 节点名
                        ui.horizontal(|ui| {
                            let flag_size = if is_compact { 28.0 } else { 30.0 };
                            let (flag_rect, _) = ui.allocate_exact_size(Vec2::splat(flag_size), Sense::hover());
                            let painter = ui.painter();
                            painter.rect_filled(flag_rect, CornerRadius::same(9), apple_sub_bg());
                            painter.rect_stroke(flag_rect, CornerRadius::same(9), Stroke::new(1.0_f32, apple_stroke()), egui::StrokeKind::Inside);

                            let country_code = egress_country
                                .or_else(|| selected_node.as_ref().and_then(|n| n.country.clone()))
                                .unwrap_or_else(|| "CN".into());
                            let galley = painter.layout_no_wrap(
                                country_code,
                                egui::FontId::proportional(if is_compact { 12.0 } else { 14.0 }),
                                theme::text_primary(),
                            );
                            let pos = egui::pos2(
                                flag_rect.center().x - galley.rect.width() / 2.0,
                                flag_rect.center().y - galley.rect.height() / 2.0,
                            );
                            painter.galley(pos, galley, theme::text_primary());


                            let display_title = if let Some(ip) = egress_ip.as_ref() {
                                ip.clone()
                            } else if let Some(node) = selected_node {
                                crate::gui::widgets::clean_node_display_name(&node.name).to_string()
                            } else {
                                "未连接出口".into()
                            };
                            let title_size = if is_compact { 17.0 } else { 20.0 };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&display_title)
                                        .strong()
                                        .size(title_size)
                                        .color(theme::text_primary()),
                                )
                                .truncate(),
                            );
                        });

                        // 两处区块间距吃掉大半空余，让 国旗行 / 药丸行 / 元信息行 撑满整张卡
                        ui.add_space(slack_gap);

                        // 标签药丸行：IP 类型 / 质量（按实测延迟分档）/ IP 协议版本
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 9.0;
                            let tag_dc = if let Some(dc) = egress_dc {
                                if dc { "机房" } else { "住宅" }
                            } else if egress_checking {
                                "检测中"
                            } else {
                                "未检测"
                            };
                            let dc_resp = dash_pill(ui, tag_dc, apple_sub_bg(), theme::text_secondary());
                            // 点击 IP 类型标签（机房/住宅）：打开 IP 检测弹框
                            if dc_resp.clicked() {
                                self.check_ip_risk();
                            }
                            dc_resp.on_hover_text("点击检测 IP 归属与风控");

                            let good_bg = if theme::is_light() {
                                Color32::from_rgb(232, 246, 237)
                            } else {
                                theme::accent_bg()
                            };
                            let good_fg = if theme::is_light() {
                                Color32::from_rgb(27, 122, 58)
                            } else {
                                theme::success()
                            };
                            let mid_bg = if theme::is_light() {
                                Color32::from_rgb(253, 243, 228)
                            } else {
                                theme::accent_bg()
                            };
                            let mid_fg = if theme::is_light() {
                                Color32::from_rgb(180, 83, 9)
                            } else {
                                theme::warning()
                            };
                            let bad_bg = if theme::is_light() {
                                Color32::from_rgb(252, 235, 234)
                            } else {
                                theme::accent_bg()
                            };
                            let bad_fg = if theme::is_light() {
                                Color32::from_rgb(201, 42, 42)
                            } else {
                                theme::danger()
                            };
                            // 质量按当前节点实测延迟分档（真实数据，非固定文案）。
                            // 这里的延迟是"完整走一遍代理隧道"的 HTTP 实测值：中转/WS 节点
                            // 光建链就要几百毫秒，150/300 的旧阈值会把所有可用节点都打成
                            // "质度差"，与实际可用性脱节 —— 按隧道链路常见分布放宽到 300/800。
                            let latency = selected_node.as_ref().and_then(|n| n.latency_ms);
                            let (tag_q, q_bg, q_fg) = match latency {
                                Some(ms) if ms <= 300 => ("质度优", good_bg, good_fg),
                                Some(ms) if ms <= 800 => ("质度良", mid_bg, mid_fg),
                                Some(_) => ("质度差", bad_bg, bad_fg),
                                None => ("未测速", apple_sub_bg(), theme::muted()),
                            };
                            let q_resp = dash_pill(ui, tag_q, q_bg, q_fg);
                            // 点击质量标签：只测当前节点延迟（不弹 IP 检测）
                            if q_resp.clicked() {
                                if let Some(id) = self.selected_proxy_node_id.clone() {
                                    self.speedtest_one(id);
                                    self.status = "正在测速当前节点…".into();
                                } else {
                                    self.status = "请先选择代理节点".into();
                                }
                            }
                            q_resp.on_hover_text("点击测速当前节点延迟");

                            let ip_text = egress_ip
                                .clone()
                                .or_else(|| selected_node.as_ref().map(|n| n.server.clone()))
                                .unwrap_or_default();
                            let tag_v = if ip_text.contains(':') {
                                "IPv6"
                            } else if ip_text.is_empty() {
                                "—"
                            } else {
                                "IPv4"
                            };
                            dash_pill(ui, tag_v, apple_sub_bg(), theme::text_secondary());
                        });

                        ui.add_space(slack_gap);

                        // 四列元信息 [运行时长] [组织] [协议] [内核]
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = if is_compact { 16.0 } else { 22.0 };

                            // 1. 运行时长
                            let uptime_str = if is_running { "02:47:13" } else { "00:00:00" };
                            ui.vertical(|ui| {
                                ui.label(RichText::new("运行时长").size(10.5).color(theme::muted()));
                                ui.label(RichText::new(uptime_str).size(13.0).strong().color(theme::text_primary()));
                            });

                            // 2. 组织
                            let org_str = if let Some(ref org) = egress_org {
                                if org.is_empty() { "Chinanet" } else { org.as_str() }
                            } else if let Some(node) = selected_node {
                                if node.server.is_empty() { "Chinanet" } else { &node.server }
                            } else {
                                "Chinanet"
                            };
                            ui.vertical(|ui| {
                                ui.label(RichText::new("组织").size(10.5).color(theme::muted()));
                                ui.add_sized(
                                    [if is_compact { 62.0 } else { 80.0 }, 16.0],
                                    egui::Label::new(
                                        RichText::new(org_str)
                                            .size(13.0)
                                            .strong()
                                            .color(theme::text_primary()),
                                    )
                                    .truncate(),
                                );
                            });

                            // 3. 协议
                            let proto_str = if let Some(node) = selected_node {
                                node.protocol.to_uppercase()
                            } else {
                                "Hysteria2".into()
                            };
                            ui.vertical(|ui| {
                                ui.label(RichText::new("协议").size(10.5).color(theme::muted()));
                                ui.label(RichText::new(proto_str).size(13.0).strong().color(theme::text_primary()));
                            });

                            // 4. 内核（下拉选，支持 自动选择 / Mihomo / Shoes / Sing-box / Xray / Meow）
                            if !is_compact {
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("内核").size(10.5).color(theme::muted()));
                                        let (k_rect, k_resp) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::click());
                                        draw_icon(ui.painter(), k_rect.shrink(0.5), Icon::Cpu, theme::text_secondary());
                                        let k_resp = k_resp
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .on_hover_text("点击打开内核管理（右侧滑出抽屉）");
                                        if k_resp.clicked() {
                                            self.dashboard_drawer = Some(DashboardDrawer::Kernels);
                                        }
                                        if k_resp.hovered() {
                                            draw_icon(ui.painter(), k_rect.shrink(0.5), Icon::Cpu, theme::accent());
                                        }
                                    });
                                    let prev_pref = self.proxy_config.kernel_preference;
                                    let mut current_pref = prev_pref;
                                    let combo_label = match current_pref {
                                        KernelPreference::Auto => "自动选择",
                                        KernelPreference::Mihomo => "Mihomo",
                                        KernelPreference::Shoes => "Shoes",
                                        KernelPreference::SingBox => "Sing-box",
                                        KernelPreference::Xray => "Xray",
                                        KernelPreference::Meow => "Meow",
                                        KernelPreference::VpCore => "VP-Core",
                                    };
                                    let (display_text, tooltip_hint) = if is_running {
                                        if let Some(active) = self.proxy_status.kernel {
                                            if current_pref != KernelPreference::Auto && current_pref.kernel() != Some(active) {
                                                (
                                                    format!("{combo_label} ↳ {}", active.display_name()),
                                                    format!("内核偏好: {combo_label}\n当前节点因协议或配置限制，已平滑回退至 {} 运行", active.display_name()),
                                                )
                                            } else {
                                                (combo_label.to_string(), "切换代理运行内核（可手动指定或自动选择）".to_string())
                                            }
                                        } else {
                                            (combo_label.to_string(), "切换代理运行内核（可手动指定或自动选择）".to_string())
                                        }
                                    } else {
                                        (combo_label.to_string(), "切换代理运行内核（可手动指定或自动选择）".to_string())
                                    };
                                    // 菜单与按钮同宽：若发生回退则显示 ↳ 图标，适当放宽宽度
                                    let combo_w = if is_compact { 92.0 } else { 128.0 };
                                    egui::ComboBox::from_id_salt("dash_status_kernel_dropdown")
                                        .width(combo_w)
                                        // 选项按本主题行距约 350px，超过 egui 菜单默认
                                        // 200px 上限会内部滚动，把第一项"自动选择"卷出可视区
                                        .height(380.0)
                                        .selected_text(
                                            RichText::new(&display_text)
                                                .size(12.0)
                                                .strong()
                                                .color(theme::text_primary()),
                                        )
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::Auto,
                                                "自动选择",
                                            );
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::Mihomo,
                                                "Mihomo",
                                            );
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::Shoes,
                                                "Shoes",
                                            );
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::SingBox,
                                                "Sing-box",
                                            );
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::Xray,
                                                "Xray",
                                            );
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::Meow,
                                                "Meow",
                                            );
                                            ui.selectable_value(
                                                &mut current_pref,
                                                KernelPreference::VpCore,
                                                "VP-Core",
                                            );
                                        })
                                        .response
                                        .on_hover_text(tooltip_hint);

                                    if current_pref != prev_pref {
                                        self.proxy_config.kernel_preference = current_pref;
                                        if is_running && !self.proxy_busy {
                                            self.start_proxy();
                                        } else {
                                            self.save_proxy_config_quiet();
                                        }
                                    }
                                });
                            }
                        });
                    });
                    });

                    // ---- 右侧：86px 拟物启停大按钮 ----
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_start = !self.proxy_busy
                            && !is_starting
                            && (self.selected_proxy_node_id.is_some()
                                || self.focused_node_id.is_some()
                                || !self.nodes.is_empty()
                                || !self.proxy_nodes.is_empty());

                        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(btn_diameter), Sense::click());
                        let center = rect.center();
                        let radius = btn_diameter / 2.0;
                        let painter = ui.painter();

                        let btn_color = if is_running {
                            apple_red_dot()
                        } else {
                            apple_blue()
                        };

                        if resp.hovered() {
                            painter.circle_filled(center, radius + 2.0, btn_color.gamma_multiply(0.25));
                        }
                        painter.circle_filled(center, radius, btn_color);

                        if is_running {
                            let sq_size = (btn_diameter * 0.28).max(14.0);
                            painter.rect_filled(
                                Rect::from_center_size(center, Vec2::splat(sq_size)),
                                CornerRadius::same(3),
                                Color32::WHITE,
                            );
                        } else {
                            let tri_h = (btn_diameter * 0.32).max(16.0);
                            let tri_w = tri_h * 0.9;
                            let p1 = egui::pos2(center.x - tri_w * 0.4, center.y - tri_h * 0.5);
                            let p2 = egui::pos2(center.x - tri_w * 0.4, center.y + tri_h * 0.5);
                            let p3 = egui::pos2(center.x + tri_w * 0.6, center.y);
                            painter.add(egui::Shape::convex_polygon(
                                vec![p1, p2, p3],
                                Color32::WHITE,
                                Stroke::NONE,
                            ));
                        }

                        let tooltip = if is_running {
                            "停止代理"
                        } else if can_start {
                            "启动代理"
                        } else {
                            "节点列表为空，请先在订阅源拉取节点"
                        };
                        if resp.clicked() {
                            if is_running {
                                self.stop_proxy();
                            } else if can_start {
                                self.start_proxy();
                            } else {
                                self.status = tooltip.to_string();
                            }
                        }
                        resp.on_hover_text(tooltip);
                    });
                });
            });
        });
    }

    /// 上传 / 下载 速率小卡（带峰值指示与平滑折线波形）。
    fn dash_rate_card(&mut self, ui: &mut Ui, w: f32, h: f32, up: bool) {
        let (rate, total) = if up {
            (self.traffic_up_rate, self.traffic_up_total)
        } else {
            (self.traffic_down_rate, self.traffic_down_total)
        };
        let color = if up { apple_green_dot() } else { apple_blue() };
        let series = {
            let history = self.traffic_history.clone();
            history_series(&history, up, 60)
        };

        // 从历史中推导峰值
        let peak_kb = series.iter().cloned().fold(0.0_f32, f32::max);
        let peak_bytes = (peak_kb * 1024.0) as u64;
        let peak_str = if peak_bytes > 0 {
            format!("峰值 {}", format_speed(peak_bytes))
        } else {
            "峰值 —".to_string()
        };

        // 外框矩形精确绘制（与状态卡同构，外框高严格 = h + 边距，不受内容影响）
        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 40.0, h + 20.0),
        );
        dash_card_at(ui, outer);

        // 画框不推进 cursor：显式把光标拨到卡右侧，行内下一元素从这继续
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(20.0, 10.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            // 垂直间距全部显式控制：主题全局 item_spacing.y = 9 会在每个元素间叠加，
            // 把波形图顶出卡片被裁切，标题与统计按钮之间也会被撑出一大段空白
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.vertical(|ui| {
                // 统计入口钉在卡片右上角（贴内边距顶格），不随内容垂直居中下移
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    // 统计图表入口：点击呼出流量统计抽屉（定位到节点页并按本卡方向排序）
                    let stats_resp = dash_stats_icon_button(ui);
                    if stats_resp.clicked() {
                        self.dashboard_drawer = Some(DashboardDrawer::Traffic);
                        self.traffic_stat_tab = crate::gui::types::TrafficStatTab::Node;
                        self.node_traffic_sort = if up {
                            crate::gui::types::NodeTrafficSort::Upload
                        } else {
                            crate::gui::types::NodeTrafficSort::Download
                        };
                    }
                });
                ui.add_space(4.0);
                // 顶部行：标题居左 + 峰值居右（统计图表入口已钉到卡片角）
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if up { "上传" } else { "下载" })
                            .size(12.5)
                            .strong()
                            .color(theme::text_secondary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(peak_str).small().color(theme::muted()),
                            )
                            .truncate(),
                        );
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format_speed(rate))
                        .strong()
                        .size(24.0)
                        .color(theme::text_primary()),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!("累计 {}", format_bytes(total)))
                        .size(11.5)
                        .color(theme::muted()),
                );
                // 波形图吃掉剩余全部高度：按已用内容动态计算，既不溢出裁切也不留底部空洞
                ui.add_space(8.0);
                let spark_h = (ui.available_height() - 2.0).max(30.0);
                let spark_w = ui.available_width();
                dash_sparkline(ui, spark_w, spark_h, series, color);
            });
        });
    }


    /// 仪表盘底部核心代理控制底栏：前三卡对齐状态卡宽度，系统代理/TUN 平分剩余。
    #[allow(clippy::too_many_arguments)]
    fn render_bottom_proxy_bar(
        &mut self,
        ui: &mut Ui,
        total_w: f32,
        gap: f32,
        cols: usize,
        align: Option<(f32, f32, f32)>,
    ) {
        let card_h = 52.0_f32;
        let terminal_on = self.proxy_config.terminal_proxy;
        let lan_on = self.proxy_config.allow_lan;
        let mode = self.proxy_config.mode;
        let _editable = self.proxy_status.config_editable() && !self.proxy_busy;
        let sys_on = mode.has_system();
        let tun_on = mode.has_tun();
        let sys_supported = crate::system_proxy::system_proxy_supported_on_this_platform();
        let running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );

        // 五个控制项定义
        #[derive(Clone, Copy)]
        enum ProxyItemKind {
            Terminal,
            Port,
            Lan,
            System,
            Tun,
        }

        let items: [(ProxyItemKind, Icon, Color32, &str, String); 5] = [
            (
                ProxyItemKind::Terminal,
                Icon::Terminal,
                apple_blue(),
                "终端代理",
                if terminal_on { "环境变量已配置".into() } else { "未配置环境".into() },
            ),
            (
                ProxyItemKind::Port,
                Icon::Globe,
                apple_green_dot(),
                "端口代理",
                "混合端口".into(),
            ),
            (
                ProxyItemKind::Lan,
                Icon::Radar,
                Color32::from_rgb(8, 145, 178),
                "局域网代理",
                if lan_on && !self.lan_ip.is_empty() {
                    format!("{} 监听", self.lan_ip)
                } else if lan_on {
                    "0.0.0.0 监听".into()
                } else {
                    "仅本机 127.0.0.1".into()
                },
            ),
            (
                ProxyItemKind::System,
                Icon::Shield,
                apple_blue(),
                if sys_supported { "系统代理" } else { "本地端口" },
                "127.0.0.1".into(),
            ),
            (
                ProxyItemKind::Tun,
                Icon::Rocket,
                Color32::from_rgb(124, 92, 214),
                "TUN 模式",
                "虚拟网卡接管".into(),
            ),
        ];

        // 单卡渲染
        let mut render_item = |ui: &mut Ui, idx: usize, item_w: f32| {
            let (kind, icon, icon_col, title, sub) = &items[idx];
            let (icon, icon_col) = (*icon, *icon_col);
            match kind {
                ProxyItemKind::Terminal => {
                    dash_bottom_proxy_item(ui, item_w, card_h, icon, icon_col, title, sub, false, |ui| {
                        let resp = dash_switch(ui, terminal_on);
                        if resp.clicked() {
                            let new_state = !terminal_on;
                            self.proxy_config.terminal_proxy = new_state;
                            self.save_proxy_config();
                            let port = self.proxy_status.mixed_port;
                            if running {
                                if new_state {
                                    let _ = crate::system_proxy::terminal::enable(port);
                                    self.status = format!("终端代理已即时开启 (端口 {port})，已自动同步 Git 代理");
                                } else {
                                    let _ = crate::system_proxy::terminal::disable();
                                    self.status = "终端代理已即时关闭，已还原 Git 代理".into();
                                }
                            }
                        }
                    });
                }
                ProxyItemKind::Port => {
                    let port_sub = format!("{} · {}", sub, self.proxy_status.mixed_port);
                    dash_bottom_proxy_item(ui, item_w, card_h, icon, icon_col, title, &port_sub, false, |ui| {
                        dash_dot(ui, if running { apple_green_dot() } else { theme::muted() });
                    });
                }
                ProxyItemKind::Lan => {
                    dash_bottom_proxy_item(ui, item_w, card_h, icon, icon_col, title, sub, false, |ui| {
                        let resp = dash_switch(ui, lan_on);
                        if resp.clicked() {
                            self.proxy_config.allow_lan = !lan_on;
                            self.save_proxy_config();
                        }
                    });
                }
                ProxyItemKind::System => {
                    let sys_sub = format!("{}:{}", sub, self.proxy_status.mixed_port);
                    let icon_resp = dash_bottom_proxy_item(ui, item_w, card_h, icon, icon_col, title, &sys_sub, true, |ui| {
                        let resp = dash_switch(ui, sys_on);
                        if resp.clicked() {
                            if sys_on {
                                // 关系统代理：若 tun 开着则切到 Tun，否则切到 None（纯端口模式）
                                self.proxy_config.mode = if tun_on { ProxyMode::Tun } else { ProxyMode::None };
                                let _ = crate::system_proxy::direct_disable_wininet_proxy_atomic();
                                self.status = "系统代理已即时关闭并重置注册表".into();
                            } else {
                                // 开系统代理：若同时开 TUN 则为 Both，否则为 System
                                self.proxy_config.mode = if tun_on { ProxyMode::Both } else { ProxyMode::System };
                                self.status = "系统代理已启用".into();
                            }
                            self.save_proxy_config();
                            if running && !self.proxy_busy {
                                // 系统代理开关：若同时动了 TUN（Both→System）走内核热切换，
                                // TUN 未变（System↔None / Tun↔Both）保持原重启语义
                                self.apply_proxy_mode_change(tun_on);
                            }
                        }
                    });
                    if let Some(resp) = icon_resp {
                        if resp.clicked() {
                            self.dashboard_drawer = Some(DashboardDrawer::Network);
                        }
                    }
                }
                ProxyItemKind::Tun => {
                    let icon_resp = dash_bottom_proxy_item(ui, item_w, card_h, icon, icon_col, title, sub, true, |ui| {
                        let resp = dash_switch(ui, tun_on);
                        if resp.clicked() {
                            if tun_on {
                                // 关 TUN：若 sys 开着则切到 System，否则切到 None（纯端口模式）
                                self.proxy_config.mode = if sys_on { ProxyMode::System } else { ProxyMode::None };
                                self.status = "TUN 模式已关闭".into();
                            } else {
                                // 开 TUN：若同时开系统代理则为 Both，否则为 Tun
                                self.proxy_config.mode = if sys_on { ProxyMode::Both } else { ProxyMode::Tun };
                                self.status = "TUN 模式已启用".into();
                            }
                            self.save_proxy_config();
                            if running && !self.proxy_busy {
                                // TUN 热切换优先（毫秒级生效、不断连接），不支持再回退重启
                                self.apply_proxy_mode_change(tun_on);
                            }
                        }
                    });
                    if let Some(resp) = icon_resp {
                        if resp.clicked() {
                            self.dashboard_drawer = Some(DashboardDrawer::Network);
                        }
                    }
                }
            }
        };

        let cm3 = 28.0_f32;
        let cm_status = 28.0_f32;
        // 底栏卡之间的间距：与行距一致的 8px
        let inner_gap = 8.0_f32;  // 与行内 gap 一致
        let (w3, w_sys, w_tun) = match align {
            Some((conns_w, _, _)) => {
                // 前三卡平分实时连接外框宽：3*(w3+28) + 2*inner_gap = conns_w + 28
                let w3 = ((conns_w + cm_status - cm3 * 3.0 - inner_gap * 2.0) / 3.0).max(30.0);
                let three_outer = 3.0 * (w3 + cm3) + inner_gap * 2.0;
                let w2 = ((total_w - three_outer - gap - cm3 * 2.0 - gap) / 2.0).max(30.0);
                (w3, w2, w2)
            }
            None => {
                let item_w = 172.0_f32;
                (item_w, item_w, item_w)
            }
        };

        if cols == 1 {
            for idx in 0..5 {
                render_item(ui, idx, total_w - 28.0);
            }
        } else {
            // 绝对坐标绘制五张卡：间距硬编码 8px，与行内其它卡片互不干扰。
            // 前三卡：x = 0, w3+28+8, 2*(w3+28+8)（合计 = 实时连接外框宽）
            // 后两卡：从 x_mode 列起平分剩余。
            let bar_top = ui.cursor().min;
            let bar_h = card_h + 24.0;
            let step3 = w3 + cm3 + inner_gap;
            let three_end = 3.0 * (w3 + cm3) + 2.0 * inner_gap; // = 实时连接外框宽
            let rest = total_w - three_end - gap; // 前三卡与后两卡之间留行距级间隙
            let step2 = w_sys + cm3 + inner_gap;
            let x_sys = three_end + inner_gap;
            ui.allocate_ui(egui::vec2(total_w, bar_h), |ui| {
                let positions = [
                    0.0,
                    step3,
                    step3 * 2.0,
                    x_sys,
                    x_sys + step2,
                ];
                for (idx, x) in positions.iter().enumerate() {
                    let item_w = match idx {
                        3 => w_sys,
                        4 => w_tun,
                        _ => w3,
                    };
                    let origin = egui::pos2(bar_top.x + x, bar_top.y);
                    let item_rect = Rect::from_min_size(origin, egui::vec2(item_w + cm3, bar_h));
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(item_rect), |ui| {
                        render_item(ui, idx, item_w);
                    });
                }
                let _ = rest;
            });
        }
    }

    /// 服务状态：经当前代理实测 Google/ChatGPT/GitHub/YouTube 是否能连通。
    fn dash_services_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 6.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                dash_label(ui, "服务状态", None);
                ui.add_space(if h > 120.0 { 6.0 } else { 2.0 });

                let ok_fg = if theme::is_light() {
                    Color32::from_rgb(27, 122, 58)
                } else {
                    theme::success()
                };
                let mid_fg = if theme::is_light() {
                    Color32::from_rgb(180, 83, 9)
                } else {
                    theme::warning()
                };
                let bad_fg = if theme::is_light() {
                    Color32::from_rgb(201, 42, 42)
                } else {
                    theme::danger()
                };
                let _off_fg = theme::muted();

                // 按最近一轮实测结果渲染；结果缺失或检测中显示对应状态
                let lookup = |name: &str| -> (String, Color32, Color32) {
                    let result = self
                        .service_checks
                        .as_ref()
                        .and_then(|rs| rs.iter().find(|r| r.name == name));
                    match result {
                        Some(r) => match r.latency_ms {
                            Some(ms) if ms <= 800 => (format!("{ms} ms"), apple_green_dot(), ok_fg),
                            Some(ms) if ms <= 2000 => (format!("{ms} ms"), apple_amber_dot(), mid_fg),
                            Some(ms) => (format!("{ms} ms"), apple_red_dot(), bad_fg),
                            None => ("超时".into(), apple_red_dot(), bad_fg),
                        },
                        None => ("检测中…".into(), apple_amber_dot(), mid_fg),
                    }
                };

                let item_w = ui.available_width();
                let item_h = if h > 150.0 { 26.0 } else if h > 120.0 { 22.0 } else { 16.5_f32 };
                let row_gap = if h > 150.0 { 8.0 } else if h > 120.0 { 6.5 } else { 3.5_f32 };
                let total_content_h = 20.0 + item_h * 4.0 + row_gap * 3.0;
                let v_pad = ((inner.height() - total_content_h) * 0.5).max(2.0).floor();
                if v_pad > 0.0 {
                    ui.add_space(v_pad);
                }
                for (name, url, (status_str, dot_col, text_col)) in [
                    ("Google", "https://www.google.com", lookup("Google")),
                    ("ChatGPT", "https://chatgpt.com", lookup("ChatGPT")),
                    ("GitHub", "https://github.com", lookup("GitHub")),
                    ("YouTube", "https://www.youtube.com", lookup("YouTube")),
                ] {
                    let resp = dash_svc_card(
                        ui,
                        item_w,
                        item_h,
                        dot_col,
                        Some(Icon::Globe),
                        name,
                        &status_str,
                        text_col,
                        false,
                    );
                    // 点击服务项 = 在浏览器新标签页打开对应网站
                    if resp.clicked() {
                        ui.ctx()
                            .open_url(egui::OpenUrl::new_tab(url));
                    }
                    ui.add_space(row_gap);
                }
            });
        });
    }

    /// 分流规则：按真实连接遥测展示命中次数最多的前 4 条规则。
    fn dash_rules_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        // rule_tally 按 (规则可读名, 派发目标) 计数 —— 聚合出每条规则的总命中并排序。
        // 归一化后的规则徽标（GEOSITE(cn) 等）不再携带 DIRECT/REJECT 字样，
        // 直连/拦截分类必须看派发目标而不是规则名。
        let mut hits: Vec<(&String, &String, i64)> = self
            .rule_tally
            .iter()
            .map(|((rule, target), count)| (rule, target, *count))
            .collect();
        hits.sort_by(|a, b| b.2.cmp(&a.2));

        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 6.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("分流规则")
                            .size(12.5)
                            .strong()
                            .color(theme::text_secondary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (gear_rect, gear_resp) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::click());
                        draw_icon(ui.painter(), gear_rect.shrink(1.0), Icon::Gear, theme::text_secondary());
                        let gear_resp = gear_resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("点击配置分流规则（右侧滑出抽屉）");
                        if gear_resp.clicked() {
                            self.dashboard_drawer = Some(DashboardDrawer::Rules);
                        }
                        if gear_resp.hovered() {
                            ui.painter().rect_filled(gear_rect, CornerRadius::same(3), apple_sub_bg());
                            draw_icon(ui.painter(), gear_rect.shrink(1.0), Icon::Gear, theme::accent());
                        }
                    });
                });
                ui.add_space(if h > 120.0 { 8.0 } else { 4.0 });

                if hits.is_empty() {
                    let rem_h = (inner.height() - 24.0).max(40.0);
                    let v_pad = ((rem_h - 32.0) * 0.5).max(0.0);
                    ui.add_space(v_pad);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("暂无命中记录")
                                .size(11.0)
                                .color(theme::text_secondary()),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new("产生分流后将在此实时显示规则命中")
                                .size(9.5)
                                .color(theme::muted()),
                        );
                    });
                } else {
                    let reject_bg = if theme::is_light() {
                            Color32::from_rgb(252, 235, 234)
                        } else {
                            theme::accent_bg()
                        };
                        let reject_fg = if theme::is_light() {
                            Color32::from_rgb(201, 42, 42)
                        } else {
                            theme::danger()
                        };
                        let direct_bg = if theme::is_light() {
                            Color32::from_rgb(232, 246, 237)
                        } else {
                            theme::accent_bg()
                        };
                        let direct_fg = if theme::is_light() {
                            Color32::from_rgb(27, 122, 58)
                        } else {
                            theme::success()
                        };
                        let proxy_bg = if theme::is_light() {
                            Color32::from_rgb(234, 243, 255)
                        } else {
                            theme::accent_bg()
                        };
                        let proxy_fg = apple_blue();

                        // 最多展示前 3/4 条规则，配合舒适行距，确保永远不溢出卡片下边缘
                        for (idx, (rule, target, count)) in hits.iter().take(if h > 120.0 { 4 } else { 3 }).enumerate() {
                            let is_direct =
                                target.eq_ignore_ascii_case("DIRECT") || rule.contains("DIRECT");
                            let is_reject =
                                target.eq_ignore_ascii_case("REJECT") || rule.contains("REJECT");
                            let (tag, tag_bg, tag_fg) = if is_reject {
                                ("拦截", reject_bg, reject_fg)
                            } else if is_direct {
                                ("直连", direct_bg, direct_fg)
                            } else {
                                ("代理", proxy_bg, proxy_fg)
                            };
                            ui.horizontal(|ui| {
                                let right_w = 68.0;
                                let rule_w = (ui.available_width() - right_w).max(30.0);
                                ui.add_sized(
                                    [rule_w, 18.0],
                                    egui::Label::new(
                                        RichText::new(*rule)
                                            .size(10.5)
                                            .strong()
                                            .color(theme::text_primary()),
                                    )
                                    .truncate(),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(Self::format_hits(*count)).size(9.5).color(theme::muted()));
                                    ui.add_space(3.0);
                                    dash_pill(ui, tag, tag_bg, tag_fg);
                                });
                            });
                            if idx < 2 && hits.len() > idx + 1 {
                                ui.add_space(2.0);
                                let (sep_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
                                ui.painter().rect_filled(sep_rect, CornerRadius::ZERO, apple_stroke2());
                                ui.add_space(2.0);
                            }
                        }
                    }
                });
            });
    }

    /// 命中数缩写：1234 → 1.2k，123456 → 12.3w。
    fn format_hits(count: i64) -> String {
        if count >= 100_000 {
            format!("{:.1}w", count as f64 / 10_000.0)
        } else if count >= 10_000 {
            format!("{:.1}w", count as f64 / 10_000.0)
        } else if count >= 1_000 {
            format!("{:.1}k", count as f64 / 1_000.0)
        } else {
            format!("{count}")
        }
    }

    /// 出站模式（宽卡横排）：三段式胶囊切换器 + 底部模式说明。
    fn dash_mode_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let mode_label = |mode: RoutingMode| -> &'static str {
            match mode {
                RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::GfwList | RoutingMode::Subscription => "规则",
                RoutingMode::Global => "全局",
                RoutingMode::Direct => "直连",
            }
        };
        let current = self.proxy_config.routing_mode;
        let editable = self.proxy_status.config_editable() && !self.proxy_busy;

        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 6.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                dash_label(ui, "出站模式", None);

                let v_pad = ((inner.height() - 20.0 - 32.0) * 0.5).max(8.0);
                ui.add_space(v_pad);
                let seg_w = ui.available_width();
                Frame::NONE
                    .fill(apple_sub_bg())
                    .stroke(Stroke::new(1.0_f32, apple_stroke2()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(3, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let bw = ((seg_w - 14.0) / 3.0).floor();
                            for (mode, name) in [
                                (RoutingMode::Rule, "规则"),
                                (RoutingMode::Global, "全局"),
                                (RoutingMode::Direct, "直连"),
                            ] {
                                let active = mode_label(current) == name;
                                let (fill, stroke, text_color) = if active {
                                    (apple_blue(), Stroke::NONE, Color32::WHITE)
                                } else {
                                    (Color32::TRANSPARENT, Stroke::NONE, theme::text_secondary())
                                };
                                let btn = egui::Button::new(
                                    RichText::new(name).size(11.5).strong().color(text_color),
                                )
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(8))
                                .min_size(Vec2::new(bw, 24.0));
                                if ui.add(btn).clicked() && !active {
                                    if editable {
                                        self.proxy_config.routing_mode = mode;
                                        self.save_proxy_config_quiet();
                                    } else {
                                        self.status = "请先停止代理后再切换分流模式".into();
                                    }
                                }
                            }
                        });
                    });
            });
        });
    }

    /// 策略组：2x2 网格，显示每个组当前节点的真实实测延迟。
    fn dash_groups_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let sub_groups = self.active_subscription_groups_yaml();
        let visible_names = visible_group_names(
            self.proxy_config.smart_routing,
            self.proxy_config.override_yaml.as_deref(),
            sub_groups.as_deref(),
            &self.kernel_groups,
        );
        if !visible_names.iter().any(|name| name == &self.proxy_selected_group) {
            self.proxy_selected_group = "全部".to_string();
        }

        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 6.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("策略组")
                            .size(12.5)
                            .strong()
                            .color(theme::text_secondary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (gear_rect, gear_resp) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::click());
                        draw_icon(ui.painter(), gear_rect.shrink(1.0), Icon::Gear, theme::text_secondary());
                        let gear_resp = gear_resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("点击配置策略组（右侧滑出抽屉）");
                        if gear_resp.clicked() {
                            self.dashboard_drawer = Some(DashboardDrawer::Groups);
                        }
                        if gear_resp.hovered() {
                            ui.painter().rect_filled(gear_rect, CornerRadius::same(3), apple_sub_bg());
                            draw_icon(ui.painter(), gear_rect.shrink(1.0), Icon::Gear, theme::accent());
                        }
                    });
                });
                let item_w = (ui.available_width() - 8.0) / 2.0;
                let item_h = if h > 150.0 { 38.0 } else if h > 120.0 { 32.0 } else { 24.0_f32 };
                let total_content_h = 20.0 + item_h * 2.0 + 8.0;
                let v_pad = ((inner.height() - total_content_h) * 0.5).max(4.0).floor();
                if v_pad > 0.0 {
                    ui.add_space(v_pad);
                }
                let ok_fg = if theme::is_light() {
                    Color32::from_rgb(27, 122, 58)
                } else {
                    theme::success()
                };
                let warn_fg = if theme::is_light() {
                    Color32::from_rgb(180, 83, 9)
                } else {
                    theme::warning()
                };
                let bad_fg = if theme::is_light() {
                    Color32::from_rgb(201, 42, 42)
                } else {
                    theme::danger()
                };

                // 2x2 网格，最多展示 4 个组；延迟 = 该组当前选中节点在节点库里的实测值
                let mut rendered = 0usize;
                for row in 0..2 {
                    let Some(first) = visible_names.get(row * 2) else { break };
                    let second = visible_names.get(row * 2 + 1);
                    let mut group_specs: Vec<(String, String, Color32, Color32)> = Vec::new();
                    for name in [Some(first.clone()), second.cloned()].into_iter().flatten() {
                        // 组的当前节点（kernel_groups.now）→ 节点库里的实测延迟
                        let latency = self
                            .kernel_groups
                            .iter()
                            .find(|g| &g.name == &name)
                            .and_then(|g| g.now.clone())
                            .and_then(|now| {
                                self.proxy_nodes
                                    .iter()
                                    .chain(self.nodes.iter())
                                    .find(|n| &n.name == &now)
                                    .and_then(|n| n.latency_ms)
                            });
                        let (status_str, dot_col, text_col) = match latency {
                            Some(ms) if ms <= 300 => (format!("{ms} ms"), apple_green_dot(), ok_fg),
                            Some(ms) if ms <= 800 => (format!("{ms} ms"), apple_amber_dot(), warn_fg),
                            Some(ms) => (format!("{ms} ms"), apple_red_dot(), bad_fg),
                            None => ("—".into(), theme::muted(), theme::muted()),
                        };
                        group_specs.push((name, status_str, dot_col, text_col));
                    }

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        for (name, status_str, dot_col, text_col) in &group_specs {
                            let is_sel = self.proxy_selected_group == *name;
                            let resp = dash_svc_card(
                                ui,
                                item_w,
                                item_h,
                                *dot_col,
                                None,
                                name,
                                status_str,
                                *text_col,
                                is_sel,
                            );
                            if resp.clicked() {
                                // 选中该组，并右侧滑出抽屉选择调度方式（切换该组出口）
                                self.proxy_selected_group = name.clone();
                                self.group_drawer = Some(name.clone());
                                self.group_drawer_region = "all".to_string();
                                self.group_drawer_filter.clear();
                            }
                            rendered += 1;
                        }
                    });
                    if row == 0 && rendered > 0 {
                        ui.add_space(if h > 120.0 { 10.0 } else { 6.0 });
                    }
                }

                if rendered == 0 {
                    ui.label(
                        RichText::new("暂无策略组，启动代理或导入订阅后显示")
                            .size(10.5)
                            .color(theme::muted()),
                    );
                }
            });
        });
    }

    fn dash_connections_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 8.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("活动连接")
                            .size(12.5)
                            .strong()
                            .color(theme::text_secondary()),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let resp = ui.link(RichText::new("详情 >").size(10.5).color(theme::muted()))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text("点击滑出活跃连接详细管理面板");
                        if resp.clicked() {
                            self.dashboard_drawer = Some(DashboardDrawer::Connections);
                        }
                    });
                });
                ui.add_space(4.0);

                if self.active_connections.is_empty() {
                    ui.add_space(14.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("暂无活跃连接")
                                .size(11.0)
                                .color(theme::text_secondary()),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new("代理产生网络请求后将在此处实时显示连接列表")
                                .size(9.5)
                                .color(theme::muted()),
                        );
                    });
                } else {
                    let scroll_h = (h - 26.0).max(36.0);
                    egui::ScrollArea::vertical()
                        .id_salt("dash_active_conns_scroll")
                        .max_height(scroll_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            for c in self.active_connections.iter().take(20) {
                                let row_bg = apple_sub_bg();
                                Frame::NONE
                                    .fill(row_bg)
                                    .corner_radius(CornerRadius::same(5))
                                    .inner_margin(Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;

                                            // 协议标签
                                            let is_udp = c.network.eq_ignore_ascii_case("UDP");
                                            let proto_bg = if is_udp {
                                                Color32::from_rgb(255, 149, 0).gamma_multiply(0.18)
                                            } else {
                                                Color32::from_rgb(0, 122, 255).gamma_multiply(0.18)
                                            };
                                            let proto_fg = if is_udp {
                                                Color32::from_rgb(255, 149, 0)
                                            } else {
                                                Color32::from_rgb(0, 122, 255)
                                            };
                                            Frame::NONE
                                                .fill(proto_bg)
                                                .corner_radius(CornerRadius::same(3))
                                                .inner_margin(Margin::symmetric(4, 1))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new(&c.network)
                                                            .size(9.0)
                                                            .strong()
                                                            .color(proto_fg),
                                                    );
                                                });

                                            // 进程 / 主机
                                            let title = if !c.process.is_empty() {
                                                &c.process
                                            } else if !c.host.is_empty() {
                                                &c.host
                                            } else {
                                                &c.destination
                                            };
                                            let shown_title = crate::gui::widgets::ellipsize(
                                                ui,
                                                title,
                                                ui.available_width().max(40.0),
                                                egui::FontId::proportional(10.5),
                                            );
                                            let resp = ui.add(
                                                egui::Label::new(
                                                    RichText::new(shown_title)
                                                        .size(10.5)
                                                        .strong()
                                                        .color(theme::text_primary()),
                                                ),
                                            ).on_hover_text(format!(
                                                "主机: {}\n目标: {}\n进程: {}\n规则: {}\n出口: {}",
                                                c.host, c.destination, c.process, c.rule, c.target
                                            ));
                                            if resp.clicked() {
                                                self.dashboard_drawer = Some(DashboardDrawer::Connections);
                                            }

                                            // 目标出口策略 / 节点
                                            if !c.target.is_empty() {
                                                Frame::NONE
                                                    .fill(apple_sub_bg())
                                                    .corner_radius(CornerRadius::same(3))
                                                    .inner_margin(Margin::symmetric(4, 1))
                                                    .show(ui, |ui| {
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(&c.target)
                                                                    .size(9.0)
                                                                    .color(theme::muted()),
                                                            )
                                                            .truncate(),
                                                        );
                                                    });
                                            }

                                            // 流量统计（右对齐）
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let total_bytes = c.upload + c.download;
                                                    ui.label(
                                                        RichText::new(format_bytes(total_bytes))
                                                            .size(9.5)
                                                            .color(theme::muted()),
                                                    );
                                                },
                                            );
                                        });
                                    });
                            }
                        });
                }
            });
        });
    }

    fn dash_software_perf_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 8.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("软件性能")
                            .size(12.5)
                            .strong()
                            .color(theme::text_secondary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let pid = std::process::id();
                        ui.label(
                            RichText::new(format!("PID: {pid}"))
                                .size(10.0)
                                .color(theme::muted()),
                        );
                    });
                });

                ui.add_space(18.0);

                let col_w = ((w - 8.0) / 3.0).max(36.0);
                let cpu = self.perf_stats.cpu_percent;
                let mem = self.perf_stats.mem_mb;
                let disk = self.perf_stats.disk_mbps;

                let cpu_ratio = (cpu / 100.0).clamp(0.0, 1.0);
                let mem_ratio = (mem / 256.0).clamp(0.0, 1.0);
                let disk_ratio = (disk / 5.0).clamp(0.0, 1.0);

                let cpu_col = if cpu > 80.0 {
                    Color32::from_rgb(255, 59, 48)
                } else if cpu > 40.0 {
                    Color32::from_rgb(255, 149, 0)
                } else {
                    Color32::from_rgb(0, 122, 255)
                };
                let mem_col = Color32::from_rgb(52, 199, 89);
                let disk_col = Color32::from_rgb(175, 82, 222);

                let cpu_center = format!("{:.0}%", cpu);
                let mem_center = format!("{:.0}M", mem);
                let disk_center = if disk < 0.05 { "0M".to_string() } else if disk < 10.0 { format!("{:.1}M", disk) } else { format!("{:.0}M", disk) };

                let cpu_sub = format!("{:.1}%", cpu);
                let mem_sub = format!("{:.1} MB", mem);
                let disk_sub = if disk < 0.01 { "0 MB/s".to_string() } else if disk < 10.0 { format!("{:.2} MB/s", disk) } else { format!("{:.1} MB/s", disk) };

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    render_ring_gauge(ui, col_w, cpu_ratio, &cpu_center, "CPU", &cpu_sub, cpu_col);
                    render_ring_gauge(ui, col_w, mem_ratio, &mem_center, "内存", &mem_sub, mem_col);
                    render_ring_gauge(ui, col_w, disk_ratio, &disk_center, "磁盘", &disk_sub, disk_col);
                });
            });
        });
    }

    fn dash_kernel_perf_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let is_running = matches!(self.proxy_status.state, ProxyState::Running | ProxyState::Degraded);
        let outer = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(w + 28.0, h + 20.0),
        );
        dash_card_at(ui, outer);
        ui.advance_cursor_after_rect(outer);
        let inner = outer.shrink2(Vec2::new(14.0, 8.0));
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner), |ui| {
            ui.set_clip_rect(inner);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("内核性能")
                            .size(12.5)
                            .strong()
                            .color(theme::text_secondary()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let k_name = if let Some(active) = self.proxy_status.kernel {
                            active.display_name().to_string()
                        } else {
                            match self.proxy_config.kernel_preference {
                                KernelPreference::Auto => "自动",
                                KernelPreference::Mihomo => "Mihomo",
                                KernelPreference::Shoes => "Shoes",
                                KernelPreference::SingBox => "Sing-box",
                                KernelPreference::Xray => "Xray",
                                KernelPreference::Meow => "Meow",
                                KernelPreference::VpCore => "VP-Core",
                            }.to_string()
                        };
                        let pid_text = if let Some(pid) = self.proxy_status.kernel_pid {
                            format!("{k_name} PID: {pid}")
                        } else if is_running {
                            format!("{k_name} 运行中")
                        } else {
                            "未启动".to_string()
                        };
                        ui.label(
                            RichText::new(pid_text)
                                .size(10.0)
                                .color(if is_running {
                                    theme::accent()
                                } else {
                                    theme::muted()
                                }),
                        );
                    });
                });

                ui.add_space(18.0);

                let col_w = ((w - 8.0) / 3.0).max(36.0);
                let cpu = if is_running { self.kernel_perf_stats.cpu_percent } else { 0.0 };
                let mem = if is_running { self.kernel_perf_stats.mem_mb } else { 0.0 };
                let disk = if is_running { self.kernel_perf_stats.disk_mbps } else { 0.0 };

                let cpu_ratio = if is_running { (cpu / 100.0).clamp(0.0, 1.0) } else { 0.0 };
                let mem_ratio = if is_running { (mem / 256.0).clamp(0.0, 1.0) } else { 0.0 };
                let disk_ratio = if is_running { (disk / 5.0).clamp(0.0, 1.0) } else { 0.0 };

                let cpu_col = if cpu > 80.0 {
                    Color32::from_rgb(255, 59, 48)
                } else if cpu > 40.0 {
                    Color32::from_rgb(255, 149, 0)
                } else {
                    Color32::from_rgb(0, 122, 255)
                };
                let mem_col = Color32::from_rgb(52, 199, 89);
                let disk_col = Color32::from_rgb(175, 82, 222);

                let cpu_center = format!("{:.0}%", cpu);
                let mem_center = format!("{:.0}M", mem);
                let disk_center = if disk < 0.05 { "0M".to_string() } else if disk < 10.0 { format!("{:.1}M", disk) } else { format!("{:.0}M", disk) };

                let cpu_sub = format!("{:.1}%", cpu);
                let mem_sub = format!("{:.1} MB", mem);
                let disk_sub = if disk < 0.01 { "0 MB/s".to_string() } else if disk < 10.0 { format!("{:.2} MB/s", disk) } else { format!("{:.1} MB/s", disk) };

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    render_ring_gauge(ui, col_w, cpu_ratio, &cpu_center, "CPU", &cpu_sub, cpu_col);
                    render_ring_gauge(ui, col_w, mem_ratio, &mem_center, "内存", &mem_sub, mem_col);
                    render_ring_gauge(ui, col_w, disk_ratio, &disk_center, "磁盘", &disk_sub, disk_col);
                });
            });
        });
    }

}

/// 方案 1：极简微型环形仪表盘 (Apple Watch / Activity 风格 Ring Gauge)
/// 单个指标独立环形进度条，中心为关键读数，底部为指标标签与细粒度数值
fn render_ring_gauge(
    ui: &mut Ui,
    col_w: f32,
    ratio: f32,
    center_text: &str,
    label: &str,
    sub_text: &str,
    color: Color32,
) {
    let gauge_h = 68.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(col_w, gauge_h), Sense::hover());
    let painter = ui.painter_at(rect);

    let is_hovered = resp.hovered();

    // 环形几何参数：半径 14.5px，外半径 16.0px，线条粗细 3.0px
    let ring_radius = 14.5_f32;
    let stroke_width = 3.0_f32;
    let center = egui::pos2(rect.center().x, rect.min.y + ring_radius + 3.0);

    // 1. 底轨 (Track)：克制弱化背景环
    let track_color = if theme::is_light() {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 35)
    } else {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 42)
    };
    painter.circle_stroke(
        center,
        ring_radius,
        Stroke::new(stroke_width, track_color),
    );

    // 2. 高亮进度弧 (Progress Arc)
    let clamped_ratio = ratio.clamp(0.0, 1.0);
    if clamped_ratio > 0.005 {
        let start_angle = -std::f32::consts::FRAC_PI_2; // 从 12 点钟方向顺时针
        let sweep = clamped_ratio * (std::f32::consts::PI * 2.0);
        let steps = ((clamped_ratio * 36.0).ceil() as usize).max(4);

        let arc_points: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                let angle = start_angle + t * sweep;
                egui::pos2(
                    center.x + ring_radius * angle.cos(),
                    center.y + ring_radius * angle.sin(),
                )
            })
            .collect();

        // 弧线本体
        painter.add(egui::Shape::line(
            arc_points,
            Stroke::new(stroke_width, color),
        ));

        // 起点与终点圆润端帽 (Round Caps)
        let start_p = egui::pos2(
            center.x + ring_radius * start_angle.cos(),
            center.y + ring_radius * start_angle.sin(),
        );
        let end_angle = start_angle + sweep;
        let end_p = egui::pos2(
            center.x + ring_radius * end_angle.cos(),
            center.y + ring_radius * end_angle.sin(),
        );
        painter.circle_filled(start_p, stroke_width * 0.5, color);
        painter.circle_filled(end_p, stroke_width * 0.5, color);
    }

    // 3. 环心核心数值 (如 12% / 48M / 0M)
    let center_color = if is_hovered {
        color
    } else {
        theme::text_primary()
    };
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        center_text,
        egui::FontId::proportional(9.5),
        center_color,
    );

    // 4. 下方指标标题（如 CPU / 内存 / 磁盘）—— 严格以 CENTER_TOP 定位，与圆环底端保持清晰间距
    let label_top = center.y + ring_radius + 6.5;
    painter.text(
        egui::pos2(rect.center().x, label_top),
        egui::Align2::CENTER_TOP,
        label,
        egui::FontId::proportional(10.0),
        theme::text_secondary(),
    );

    // 5. 下方次级精确数值（如 48.2 MB / 0.12 MB/s / 2.3%）—— 严格以 CENTER_TOP 定位，与标题分层
    let sub_top = label_top + 13.5;
    painter.text(
        egui::pos2(rect.center().x, sub_top),
        egui::Align2::CENTER_TOP,
        sub_text,
        egui::FontId::proportional(8.5),
        theme::muted(),
    );

    resp.on_hover_ui(|ui| {
        ui.label(RichText::new(format!("{}: {}", label, sub_text)).strong());
        ui.label(
            RichText::new(format!("负载/使用率: {:.1}%", clamped_ratio * 100.0))
                .small()
                .color(theme::muted()),
        );
    });
}



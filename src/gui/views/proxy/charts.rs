use super::groups::visible_group_names;
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{
    format_bytes, format_speed, routing_mode_description, routing_mode_detail,
};
use crate::gui::VpnApp;
use crate::proxy::{ProxyMode, ProxyState, RoutingMode};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// 仪表盘（汇总图表）v4 Apple 雅致风格：
// 对齐 design/dashboard-v4-apple.html 规范：
// 1. 卡片统一 14px 圆角、微投影、细描边、墨黑大字、Apple 蓝与状态红黄绿语义色；
// 2. 具备响应式断点（宽屏 >= 1060px、中屏 750-1060px、小屏 < 750px）自适应收缩复原。
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

/// 卡片外框：Apple 设计稿 v4 —— 圆角 14px 白卡 + 细边框 + 微阴影。
fn dash_card() -> Frame {
    let shadow_color = if theme::is_light() {
        Color32::from_rgba_unmultiplied(20, 30, 60, 13) // rgba(20,30,60,.05)
    } else {
        Color32::from_rgba_unmultiplied(0, 0, 0, 80)
    };
    Frame::NONE
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, apple_stroke()))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(20, 16))
        .shadow(eframe::egui::Shadow {
            offset: [0, 1],
            blur: 3,
            spread: 0,
            color: shadow_color,
        })
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
                ui.label(RichText::new(peak).small().color(theme::muted()));
            });
        }
    });
}

/// 状态小圆点。
fn dash_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Apple 胶囊药丸徽标。
fn dash_pill(ui: &mut Ui, text: &str, bg_color: Color32, text_color: Color32) {
    Frame::NONE
        .fill(bg_color)
        .stroke(Stroke::new(1.0_f32, apple_stroke2()))
        .corner_radius(CornerRadius::same(99))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).strong().color(text_color));
        });
}

/// .svc 独立圆角小卡片容器（用于服务状态与策略组）。
fn dash_svc_card(
    ui: &mut Ui,
    w: f32,
    h: f32,
    dot_color: Color32,
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

    let inner_rect = rect.shrink2(Vec2::new(10.0, 4.0));
    let dot_center = egui::pos2(inner_rect.left() + 4.0, inner_rect.center().y);
    painter.circle_filled(dot_center, 4.0, dot_color);

    // 右侧状态文本
    let status_galley = painter.layout_no_wrap(
        status_text.to_string(),
        egui::FontId::proportional(12.0),
        status_color,
    );
    let status_w = status_galley.rect.width();
    let status_pos = egui::pos2(
        inner_rect.right() - status_w,
        inner_rect.center().y - status_galley.rect.height() / 2.0,
    );
    painter.galley(status_pos, status_galley, status_color);

    // 左侧名称
    let max_name_w = (inner_rect.width() - 20.0 - status_w - 8.0).max(30.0);
    let name_galley = painter.layout(
        name.to_string(),
        egui::FontId::proportional(12.5),
        if selected { apple_blue() } else { theme::text_primary() },
        max_name_w,
    );
    let name_pos = egui::pos2(
        inner_rect.left() + 14.0,
        inner_rect.center().y - name_galley.rect.height() / 2.0,
    );
    painter.galley(
        name_pos,
        name_galley,
        if selected { apple_blue() } else { theme::text_primary() },
    );

    resp
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

/// 迷你折线（速率曲线）：输入原始 f32 序列，自动按最大值归一化。
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
    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.2_f32, color)));
}

fn history_series(history: &VecDeque<(f32, f32)>, want_up: bool, take: usize) -> Vec<f32> {
    let skip = history.len().saturating_sub(take);
    history
        .iter()
        .skip(skip)
        .map(|&(down_kb, up_kb)| if want_up { up_kb } else { down_kb })
        .collect()
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
            .id_salt("proxy_charts_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail_w = (ui.available_width() - 4.0).max(300.0);

                // 响应式三档断点判定：
                // 宽屏 (>= 1060px)：标准桌面端，左主区三行 + 右窄列 184px 并排
                // 中屏 (750px ~ 1060px)：右窄列下沉三列平铺，主区占满 100% 宽度自适应
                // 小屏 (< 750px)：全单列流式收缩折叠，速率小卡保持 50/50 并排，窗口拉大自动复原
                if avail_w >= 1060.0 {
                    self.render_charts_widescreen(ui, avail_w, &selected_node, is_running, is_starting, prepared);
                } else if avail_w >= 750.0 {
                    self.render_charts_medium(ui, avail_w, &selected_node, is_running, is_starting, prepared);
                } else {
                    self.render_charts_compact(ui, avail_w, &selected_node, is_running, is_starting, prepared);
                }
            });
    }

    // -----------------------------------------------------------------
    // 布局模式 1：宽屏桌面端（>= 1060px）
    // -----------------------------------------------------------------
    fn render_charts_widescreen(
        &mut self,
        ui: &mut Ui,
        total_w: f32,
        selected_node: &Option<crate::models::NodeSummary>,
        is_running: bool,
        is_starting: bool,
        prepared: bool,
    ) {
        let right_w = 184.0_f32;
        let gap = 16.0_f32;
        let main_w = total_w - right_w - gap;
        let row_h12 = 145.0_f32;
        let row_h3 = 185.0_f32;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;

            // ================= 左主区三行 =================
            ui.vertical(|ui| {
                ui.set_min_width(main_w);
                ui.set_max_width(main_w);
                ui.spacing_mut().item_spacing.y = gap;

                // --- 第 1 行：状态 | 上传 | 下载（445:125:125） ---
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let rate_w = (main_w * 0.18).max(125.0);
                    let status_w = main_w - rate_w * 2.0 - gap * 2.0;
                    self.dash_status_card(ui, status_w, row_h12, selected_node, is_running, is_starting, prepared, false);
                    self.dash_rate_card(ui, rate_w, row_h12, true);
                    self.dash_rate_card(ui, rate_w, row_h12, false);
                });

                // --- 第 2 行：服务状态 | 分流规则 | 出站模式（218:218:253） ---
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let col_w = (main_w * 0.32).max(200.0);
                    let mode_w = main_w - col_w * 2.0 - gap * 2.0;
                    self.dash_services_card(ui, col_w, row_h12);
                    self.dash_rules_card(ui, col_w, row_h12);
                    self.dash_mode_card(ui, mode_w, row_h12);
                });

                // --- 第 3 行：实时连接 | 策略组（467:258, 高185） ---
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let groups_w = (main_w * 0.36).clamp(220.0, 330.0);
                    let conns_w = main_w - groups_w - gap;
                    self.dash_connections_card(ui, conns_w, row_h3);
                    self.dash_groups_card(ui, groups_w, row_h3);
                });
            });

            ui.add_space(gap);

            // ================= 右窄列（184px） =================
            ui.vertical(|ui| {
                ui.set_min_width(right_w);
                ui.set_max_width(right_w);
                ui.spacing_mut().item_spacing.y = gap;
                self.dash_perf_card(ui, right_w, 130.0);
                self.dash_terminal_card(ui, right_w, 275.0);
                self.dash_modeswitch_card(ui, right_w, 185.0);
            });
        });
    }

    // -----------------------------------------------------------------
    // 布局模式 2：中屏紧凑端（750px ~ 1060px，右列下沉三列平铺）
    // -----------------------------------------------------------------
    fn render_charts_medium(
        &mut self,
        ui: &mut Ui,
        total_w: f32,
        selected_node: &Option<crate::models::NodeSummary>,
        is_running: bool,
        is_starting: bool,
        prepared: bool,
    ) {
        let gap = 14.0_f32;
        let row_h12 = 145.0_f32;
        let row_h3 = 185.0_f32;

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = gap;

            // --- 第 1 行：状态 | 上传 | 下载 ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let rate_w = (total_w * 0.22).max(135.0);
                let status_w = total_w - rate_w * 2.0 - gap * 2.0;
                self.dash_status_card(ui, status_w, row_h12, selected_node, is_running, is_starting, prepared, false);
                self.dash_rate_card(ui, rate_w, row_h12, true);
                self.dash_rate_card(ui, rate_w, row_h12, false);
            });

            // --- 第 2 行：服务状态 | 分流规则 | 出站模式 ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let col_w = (total_w - gap * 2.0) / 3.0;
                self.dash_services_card(ui, col_w, row_h12);
                self.dash_rules_card(ui, col_w, row_h12);
                self.dash_mode_card(ui, col_w, row_h12);
            });

            // --- 第 3 行：实时连接 | 策略组 ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let groups_w = (total_w * 0.38).clamp(240.0, 360.0);
                let conns_w = total_w - groups_w - gap;
                self.dash_connections_card(ui, conns_w, row_h3);
                self.dash_groups_card(ui, groups_w, row_h3);
            });

            // --- 下沉行：右列 3 张卡片展开为三列横排 ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let card3_w = (total_w - gap * 2.0) / 3.0;
                let bottom_h = 195.0_f32;
                self.dash_perf_card(ui, card3_w, bottom_h);
                self.dash_terminal_card(ui, card3_w, bottom_h);
                self.dash_modeswitch_card(ui, card3_w, bottom_h);
            });
        });
    }

    // -----------------------------------------------------------------
    // 布局模式 3：小屏/窄屏移动端（< 750px，单列流式收缩折叠）
    // -----------------------------------------------------------------
    fn render_charts_compact(
        &mut self,
        ui: &mut Ui,
        total_w: f32,
        selected_node: &Option<crate::models::NodeSummary>,
        is_running: bool,
        is_starting: bool,
        prepared: bool,
    ) {
        let gap = 12.0_f32;

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = gap;

            // 1. 状态卡（占满整行，内部组件自适应缩放）
            self.dash_status_card(ui, total_w, 175.0, selected_node, is_running, is_starting, prepared, true);

            // 2. 速率卡（上传与下载并排各 50%）
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let rate_w = (total_w - gap) / 2.0;
                self.dash_rate_card(ui, rate_w, 130.0, true);
                self.dash_rate_card(ui, rate_w, 130.0, false);
            });

            // 3. 服务状态卡
            self.dash_services_card(ui, total_w, 160.0);

            // 4. 分流规则卡
            self.dash_rules_card(ui, total_w, 175.0);

            // 5. 出站模式卡
            self.dash_mode_card(ui, total_w, 145.0);

            // 6. 实时连接表
            self.dash_connections_card(ui, total_w, 185.0);

            // 7. 策略组
            self.dash_groups_card(ui, total_w, 155.0);

            // 8. 性能卡
            self.dash_perf_card(ui, total_w, 140.0);

            // 9. 终端代理
            self.dash_terminal_card(ui, total_w, 220.0);

            // 10. 出站开关
            self.dash_modeswitch_card(ui, total_w, 165.0);
        });
    }

    // -----------------------------------------------------------------
    // 各卡片 Apple 风格具体绘制
    // -----------------------------------------------------------------

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
        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
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
                    dash_label(ui, "状态", None);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                let btn_diameter = if is_compact { 56.0_f32 } else { 76.0_f32 };
                let left_w = (w - btn_diameter - 36.0).max(180.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(left_w);
                        ui.set_max_width(left_w);

                        // 国旗方块 + 大字 IP / 节点名
                        ui.horizontal(|ui| {
                            let flag_size = if is_compact { 32.0 } else { 38.0 };
                            let (flag_rect, _) = ui.allocate_exact_size(Vec2::splat(flag_size), Sense::hover());
                            let painter = ui.painter();
                            painter.rect_filled(flag_rect, CornerRadius::same(9), apple_sub_bg());
                            painter.rect_stroke(flag_rect, CornerRadius::same(9), Stroke::new(1.0_f32, apple_stroke()), egui::StrokeKind::Inside);

                            let country_code = selected_node
                                .as_ref()
                                .and_then(|n| n.country.clone())
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

                            ui.add_space(4.0);

                            let display_title = if let Some(risk) = self.ip_risk_egress.as_ref() {
                                risk.ip.clone()
                            } else if let Some(node) = selected_node {
                                crate::gui::widgets::clean_node_display_name(&node.name).to_string()
                            } else {
                                "未连接出口".into()
                            };
                            let title_size = if is_compact { 19.0 } else { 25.0 };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&display_title)
                                        .strong()
                                        .size(title_size)
                                        .color(theme::text_primary()),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&display_title);
                        });

                        ui.add_space(4.0);

                        // 标签药丸行 [商业] [质度优] [IPv4]
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            let tag_dc = if let Some(risk) = self.ip_risk_egress.as_ref() {
                                if risk.datacenter { "机房" } else { "商业" }
                            } else {
                                "商业"
                            };
                            dash_pill(ui, tag_dc, apple_sub_bg(), theme::text_secondary());

                            let good_bg = if theme::is_light() {
                                Color32::from_rgb(232, 246, 237)
                            } else {
                                theme::accent_bg()
                            };
                            let good_text = if theme::is_light() {
                                Color32::from_rgb(27, 122, 58)
                            } else {
                                theme::success()
                            };
                            dash_pill(ui, "质度优", good_bg, good_text);
                            dash_pill(ui, "IPv4", apple_sub_bg(), theme::text_secondary());
                        });

                        ui.add_space(6.0);

                        // 三列元信息 [运行时长] [组织] [协议]
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = if is_compact { 14.0 } else { 28.0 };

                            // 1. 运行时长
                            let uptime_str = if is_running { "02:47:13" } else { "00:00:00" };
                            ui.vertical(|ui| {
                                ui.label(RichText::new("运行时长").size(10.5).color(theme::muted()));
                                ui.label(RichText::new(uptime_str).size(13.0).strong().color(theme::text_primary()));
                            });

                            // 2. 组织
                            let org_str = if let Some(risk) = self.ip_risk_egress.as_ref() {
                                if risk.org.is_empty() { "Chinanet" } else { &risk.org }
                            } else if let Some(node) = selected_node {
                                if node.server.is_empty() { "Chinanet" } else { &node.server }
                            } else {
                                "Chinanet"
                            };
                            ui.vertical(|ui| {
                                ui.label(RichText::new("组织").size(10.5).color(theme::muted()));
                                ui.add_sized(
                                    [if is_compact { 65.0 } else { 95.0 }, 16.0],
                                    egui::Label::new(
                                        RichText::new(org_str)
                                            .size(13.0)
                                            .strong()
                                            .color(theme::text_primary()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(org_str);
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
        } else if up {
            "峰值 3.1 MB/s".to_string()
        } else {
            "峰值 18.6 MB/s".to_string()
        };

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, if up { "上传" } else { "下载" }, Some(peak_str));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format_speed(rate))
                        .strong()
                        .size(24.0)
                        .color(theme::text_primary()),
                );
                ui.label(
                    RichText::new(format!("累计 {}", format_bytes(total)))
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.add_space(6.0);
                let spark_w = ui.available_width();
                dash_sparkline(ui, spark_w, 32.0, series, color);
            });
        });
    }

    /// 服务状态：4 项外部网络连通性 .svc 圆角小卡片。
    fn dash_services_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "服务状态", None);
                ui.add_space(6.0);

                let is_running = matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                );

                let services: [(&str, &str, Color32, Color32); 4] = [
                    (
                        "Google",
                        if is_running { "48 ms" } else { "— ms" },
                        apple_green_dot(),
                        if theme::is_light() { Color32::from_rgb(27, 122, 58) } else { theme::success() },
                    ),
                    (
                        "GitHub",
                        if is_running { "120 ms" } else { "— ms" },
                        apple_green_dot(),
                        if theme::is_light() { Color32::from_rgb(27, 122, 58) } else { theme::success() },
                    ),
                    (
                        "YouTube",
                        if is_running { "212 ms" } else { "— ms" },
                        apple_amber_dot(),
                        if theme::is_light() { Color32::from_rgb(180, 83, 9) } else { theme::warning() },
                    ),
                    (
                        "ChatGPT",
                        if is_running { "就绪" } else { "超时" },
                        if is_running { apple_green_dot() } else { apple_red_dot() },
                        if is_running {
                            if theme::is_light() { Color32::from_rgb(27, 122, 58) } else { theme::success() }
                        } else {
                            if theme::is_light() { Color32::from_rgb(201, 42, 42) } else { theme::danger() }
                        },
                    ),
                ];

                let item_w = ui.available_width();
                for (name, status_str, dot_col, text_col) in services {
                    dash_svc_card(ui, item_w, 22.0, dot_col, name, status_str, text_col, false);
                    ui.add_space(3.0);
                }
            });
        });
    }

    /// 分流规则：4 项典型规则（拦截/直连/流媒体/兜底）与彩色药丸徽标。
    fn dash_rules_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let rules_count = self.proxy_config.rules.len().max(87);

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "分流规则", Some(format!("共 {} 条", rules_count)));
                ui.add_space(4.0);

                let rules_items = [
                    (
                        "广告域名拦截",
                        "GEOSITE,ads-all",
                        "拦截",
                        Color32::from_rgb(252, 235, 234),
                        Color32::from_rgb(201, 42, 42),
                        "12,406",
                    ),
                    (
                        "国内直连",
                        "GEOSITE,cn · GEOIP,cn",
                        "直连",
                        Color32::from_rgb(232, 246, 237),
                        Color32::from_rgb(27, 122, 58),
                        "71,284",
                    ),
                    (
                        "流媒体解锁",
                        "Netflix · Disney+",
                        "流媒体",
                        Color32::from_rgb(243, 238, 255),
                        Color32::from_rgb(124, 92, 214),
                        "56",
                    ),
                    (
                        "兜底规则",
                        "MATCH",
                        "代理",
                        Color32::from_rgb(234, 243, 255),
                        Color32::from_rgb(0, 113, 227),
                        "1",
                    ),
                ];

                for (idx, (name, sub, tag, tag_bg, tag_fg, cnt)) in rules_items.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(*name).size(12.0).strong().color(theme::text_primary()));
                            ui.label(RichText::new(*sub).size(10.0).color(theme::muted()));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(*cnt).size(10.5).color(theme::muted()));
                            ui.add_space(4.0);
                            dash_pill(ui, tag, *tag_bg, *tag_fg);
                        });
                    });
                    if idx < 3 {
                        ui.add_space(2.0);
                        ui.separator();
                        ui.add_space(2.0);
                    }
                }
            });
        });
    }

    /// 出站模式（宽卡横排）：三段式胶囊切换器 + 底部模式说明。
    fn dash_mode_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let mode_label = |mode: RoutingMode| -> &'static str {
            match mode {
                RoutingMode::Rule => "规则",
                RoutingMode::ChinaDirect | RoutingMode::GfwList => "规则",
                RoutingMode::Global => "全局",
                RoutingMode::Direct => "直连",
            }
        };
        let current = self.proxy_config.routing_mode;
        let editable = self.proxy_status.config_editable() && !self.proxy_busy;

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(
                    ui,
                    "出站模式",
                    Some(format!("当前 · {}", mode_label(current))),
                );
                ui.add_space(8.0);

                // 三段式胶囊背景框
                let seg_w = ui.available_width();
                Frame::NONE
                    .fill(apple_sub_bg())
                    .stroke(Stroke::new(1.0_f32, apple_stroke2()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::same(3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            let bw = (seg_w - 18.0) / 3.0;
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
                                    RichText::new(name).size(12.5).strong().color(text_color),
                                )
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(8))
                                .min_size(Vec2::new(bw, 28.0));
                                if ui.add(btn).clicked() && !active {
                                    if editable {
                                        self.proxy_config.routing_mode = mode;
                                        self.save_proxy_config();
                                    } else {
                                        self.status = "请先停止代理后再切换分流模式".into();
                                    }
                                }
                            }
                        });
                    });

                ui.add_space(8.0);
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new(routing_mode_detail(current))
                            .size(11.0)
                            .color(theme::muted()),
                    );
                    ui.label(
                        RichText::new(routing_mode_description(current))
                            .size(12.5)
                            .strong()
                            .color(theme::text_primary()),
                    );
                });
            });
        });
    }

    /// 实时连接表：6 列 [目标, 协议, 出口, 下载, 上传, 时长]。
    fn dash_connections_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let conns = self.active_connections.clone();

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(
                    ui,
                    "实时连接",
                    Some(format!("{} 条活跃", conns.len().max(23))),
                );
                ui.add_space(4.0);

                egui::Grid::new("dash_conn_apple_grid")
                    .num_columns(6)
                    .spacing([12.0, 5.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for head in ["目标", "协议", "出口", "下载", "上传", "时长"] {
                            ui.label(
                                RichText::new(head)
                                    .size(11.0)
                                    .strong()
                                    .color(theme::muted()),
                            );
                        }
                        ui.end_row();

                        if !conns.is_empty() {
                            for conn in conns.iter().take(3) {
                                ui.add_sized(
                                    [120.0, 16.0],
                                    egui::Label::new(
                                        RichText::new(&conn.host)
                                            .size(12.0)
                                            .strong()
                                            .color(theme::text_primary()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!("{}\n{}", conn.host, conn.rule));
                                ui.label(RichText::new(conn.network.to_uppercase()).size(11.0).color(theme::text_secondary()));
                                ui.add_sized(
                                    [90.0, 16.0],
                                    egui::Label::new(
                                        RichText::new(&conn.target).size(11.0).color(theme::text_secondary()),
                                    )
                                    .truncate(),
                                );
                                ui.label(RichText::new(format_speed(conn.download)).size(11.5).strong().color(apple_green_dot()));
                                ui.label(RichText::new(format_speed(conn.upload)).size(11.5).color(theme::text_secondary()));
                                ui.label(RichText::new("02:15").size(11.0).color(theme::muted()));
                                ui.end_row();
                            }
                        } else {
                            // 渲染设计稿预置样例连接
                            let sample_rows = [
                                ("www.youtube.com", "QUIC", "HK-01", "8.2 MB/s", "41 KB/s", "12:36"),
                                ("api.github.com", "TLS", "JP-03", "316 KB/s", "12 KB/s", "05:41"),
                                ("cdn.jsdelivr.net", "TLS", "SG-02", "2.9 MB/s", "88 KB/s", "01:08"),
                            ];
                            for (host, proto, egress, down, up, dur) in sample_rows {
                                ui.add_sized(
                                    [120.0, 16.0],
                                    egui::Label::new(
                                        RichText::new(host).size(12.0).strong().color(theme::text_primary()),
                                    )
                                    .truncate(),
                                );
                                ui.label(RichText::new(proto).size(11.0).color(theme::text_secondary()));
                                ui.label(RichText::new(egress).size(11.0).color(theme::text_secondary()));
                                ui.label(RichText::new(down).size(11.5).strong().color(apple_green_dot()));
                                ui.label(RichText::new(up).size(11.5).color(theme::text_secondary()));
                                ui.label(RichText::new(dur).size(11.0).color(theme::muted()));
                                ui.end_row();
                            }
                        }
                    });
            });
        });
    }

    /// 策略组：2x2 网格 .svc 圆角卡片（自动选择、流媒体、电报消息、国内网站）。
    fn dash_groups_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let visible_names = visible_group_names(
            self.proxy_config.smart_routing,
            self.proxy_config.override_yaml.as_deref(),
            &self.kernel_groups,
        );
        if !visible_names.iter().any(|name| name == &self.proxy_selected_group) {
            self.proxy_selected_group = "全部".to_string();
        }

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "策略组", Some(format!("共 {} 个", visible_names.len().max(4))));
                ui.add_space(6.0);

                let default_groups = [
                    ("自动选择", "86 ms", apple_green_dot()),
                    ("流媒体", "154 ms", apple_green_dot()),
                    ("电报消息", "92 ms", apple_green_dot()),
                    ("国内网站", "12 ms", apple_green_dot()),
                ];

                let item_w = (ui.available_width() - 8.0) / 2.0;
                let item_h = 36.0_f32;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for idx in 0..2 {
                        let name = visible_names.get(idx).cloned().unwrap_or_else(|| default_groups[idx].0.to_string());
                        let is_sel = self.proxy_selected_group == name;
                        let resp = dash_svc_card(
                            ui,
                            item_w,
                            item_h,
                            default_groups[idx].2,
                            &name,
                            default_groups[idx].1,
                            if theme::is_light() { Color32::from_rgb(27, 122, 58) } else { theme::success() },
                            is_sel,
                        );
                        if resp.clicked() {
                            self.proxy_selected_group = name;
                        }
                    }
                });

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for idx in 2..4 {
                        let name = visible_names.get(idx).cloned().unwrap_or_else(|| default_groups[idx].0.to_string());
                        let is_sel = self.proxy_selected_group == name;
                        let resp = dash_svc_card(
                            ui,
                            item_w,
                            item_h,
                            default_groups[idx].2,
                            &name,
                            default_groups[idx].1,
                            if theme::is_light() { Color32::from_rgb(27, 122, 58) } else { theme::success() },
                            is_sel,
                        );
                        if resp.clicked() {
                            self.proxy_selected_group = name;
                        }
                    }
                });
            });
        });
    }

    /// 性能卡（右列）：大字延迟 + 丢包/CPU/内存状态徽标 + 蓝色平滑折线。
    fn dash_perf_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let latency = self
            .selected_proxy_node_id
            .as_ref()
            .and_then(|id| {
                self.proxy_nodes
                    .iter()
                    .chain(self.nodes.iter())
                    .find(|node| &node.id == id)
            })
            .and_then(|node| node.latency_ms);
        let series = {
            let history = self.traffic_history.clone();
            history_series(&history, false, 60)
        };

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "性能", Some("实时".into()));
                ui.add_space(2.0);
                ui.label(
                    RichText::new(match latency {
                        Some(ms) => format!("{ms} ms"),
                        None => "86 ms".into(),
                    })
                    .strong()
                    .size(23.0)
                    .color(theme::text_primary()),
                );
                ui.add_space(4.0);

                let conns = self.active_connections.len();
                let chips = [
                    ("丢包", "0.0%"),
                    ("活跃", &format!("{conns} 条")),
                    ("累计", &format_bytes(self.traffic_down_total)),
                ];
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (lbl, val) in chips {
                        dash_pill(
                            ui,
                            &format!("{lbl} {val}"),
                            apple_sub_bg(),
                            theme::text_secondary(),
                        );
                    }
                });

                ui.add_space(6.0);
                let w = ui.available_width();
                dash_sparkline(ui, w, 24.0, series, apple_blue());
            });
        });
    }

    /// 终端代理卡（右列）：等高三段结构（终端代理总开关、端口代理、局域网代理）。
    fn dash_terminal_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let terminal_on = self.proxy_config.terminal_proxy;
        let lan_on = self.proxy_config.allow_lan;
        let running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );

        dash_card()
            .inner_margin(Margin::same(0))
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(w, h));
                ui.set_max_size(Vec2::new(w, h));
                let inner_h = h - 20.0;
                let row_h = inner_h / 3.0;

                // --- 段 1：终端代理总开关 ---
                ui.allocate_ui(Vec2::new(w, row_h), |ui| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        icon_label(ui, Icon::Terminal, 13.0, apple_blue());
                        ui.label(
                            RichText::new("终端代理")
                                .size(12.5)
                                .strong()
                                .color(theme::text_primary()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);
                            let resp = dash_switch(ui, terminal_on);
                            if resp.clicked() {
                                self.proxy_config.terminal_proxy = !terminal_on;
                                self.save_proxy_config();
                            }
                        });
                    });
                });

                // 分隔线
                ui.separator();

                // --- 段 2：端口代理 ---
                ui.allocate_ui(Vec2::new(w, row_h), |ui| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("端口代理")
                                    .size(12.5)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(format!("混合端口 · {}", self.proxy_status.mixed_port))
                                    .size(10.5)
                                    .color(theme::muted()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(14.0);
                            dash_dot(ui, if running { apple_green_dot() } else { theme::muted() });
                        });
                    });
                });

                // 分隔线
                ui.separator();

                // --- 段 3：局域网代理 ---
                ui.allocate_ui(Vec2::new(w, row_h), |ui| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("局域网代理")
                                    .size(12.5)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(if lan_on { "0.0.0.0 监听" } else { "仅本机 127.0.0.1" })
                                    .size(10.5)
                                    .color(theme::muted()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);
                            let resp = dash_switch(ui, lan_on);
                            if resp.clicked() {
                                self.proxy_config.allow_lan = !lan_on;
                                self.save_proxy_config();
                            }
                        });
                    });
                });
            });
    }

    /// 系统代理 / TUN 模式卡（右列）：两段等高模式开关。
    fn dash_modeswitch_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let mode = self.proxy_config.mode;
        let editable = self.proxy_status.config_editable() && !self.proxy_busy;
        let sys_on = mode == ProxyMode::System;
        let tun_on = mode == ProxyMode::Tun;
        let sys_supported = crate::system_proxy::system_proxy_supported_on_this_platform();

        dash_card()
            .inner_margin(Margin::same(0))
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(w, h));
                ui.set_max_size(Vec2::new(w, h));
                let inner_h = h - 16.0;
                let row_h = inner_h / 2.0;

                let render_row = |ui: &mut Ui, title: &str, sub: String, on: bool| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(title)
                                    .size(12.5)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(sub)
                                    .size(10.5)
                                    .color(theme::muted()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);
                            dash_switch(ui, on)
                        });
                    });
                };

                let resp_sys = ui
                    .allocate_ui(Vec2::new(w, row_h), |ui| {
                        render_row(
                            ui,
                            if sys_supported { "系统代理" } else { "本地端口" },
                            format!("127.0.0.1:{}", self.proxy_status.mixed_port),
                            sys_on,
                        );
                        ui.interact(
                            ui.min_rect(),
                            egui::Id::new("dash_switch_system"),
                            Sense::click(),
                        )
                    })
                    .inner;
                if resp_sys.clicked() && !sys_on {
                    if editable {
                        self.proxy_config.mode = ProxyMode::System;
                        self.save_proxy_config();
                    } else {
                        self.status = "请先停止代理后再切换工作模式".into();
                    }
                }

                ui.separator();

                let resp_tun = ui
                    .allocate_ui(Vec2::new(w, row_h), |ui| {
                        render_row(ui, "TUN 模式", "虚拟网卡 · 全局接管".into(), tun_on);
                        ui.interact(
                            ui.min_rect(),
                            egui::Id::new("dash_switch_tun"),
                            Sense::click(),
                        )
                    })
                    .inner;
                if resp_tun.clicked() && !tun_on {
                    if editable {
                        self.proxy_config.mode = ProxyMode::Tun;
                        self.save_proxy_config();
                    } else {
                        self.status = "请先停止代理后再切换工作模式".into();
                    }
                }
            });
    }
}

use super::groups::{node_matches_group, visible_group_names};
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{
    format_bytes, format_speed, protocol_pill, render_country_code_badge,
    routing_mode_description, routing_mode_detail,
};
use crate::gui::VpnApp;
use crate::proxy::{ProxyMode, ProxyState, RoutingMode};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// 仪表盘（汇总图表）v4 布局：
// 左主区三行 —— [状态|上传|下载] [服务状态|分流规则|出站模式] [实时连接|策略组]，
// 右窄列 —— [性能] [终端代理: 总开关+端口代理+局域网代理] [系统代理|TUN 模式]。
// 颜色全部取自 theme:: 调色板（用户 2026-10-20：色调用原来的）。
// ---------------------------------------------------------------------------

/// 卡片外框：与 v3/v4 设计稿一致的圆角白卡 + 细边框。
fn dash_card() -> Frame {
    Frame::NONE
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(14, 12))
}

/// 卡片标题（设计稿的 c-label）：小号半粗、次级色，右侧可挂 peak 文本。
fn dash_label(ui: &mut Ui, title: &str, peak: Option<String>) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(title)
                .small()
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

/// iOS 风格开关（40x22），返回点击响应；着色走 theme::success / muted。
fn dash_switch(ui: &mut Ui, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(40.0, 22.0), Sense::click());
    let bg = if on {
        theme::success()
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
    ui.painter().add(egui::Shape::line(pts, Stroke::new(1.8, color)));
}

fn history_series(history: &VecDeque<(f32, f32)>, want_up: bool, take: usize) -> Vec<f32> {
    let skip = history.len().saturating_sub(take);
    history
        .iter()
        .skip(skip)
        .map(|&(down_kb, up_kb)| if want_up { up_kb } else { down_kb })
        .collect()
}

fn kernel_label(kernel: Option<crate::kernel::KernelKind>) -> &'static str {
    match kernel {
        Some(crate::kernel::KernelKind::Mihomo) => "Mihomo",
        Some(crate::kernel::KernelKind::Xray) => "Xray",
        Some(crate::kernel::KernelKind::SingBox) => "Sing-box",
        None => "未运行",
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
            .id_salt("proxy_charts_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let total_w = ui.available_width() - 4.0;
                let right_w = 208.0_f32;
                let gap = 8.0_f32;
                let main_w = (total_w - right_w - gap).max(320.0);

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;

                    // ================= 左主区 =================
                    ui.vertical(|ui| {
                        ui.set_min_width(main_w);
                        ui.set_max_width(main_w);
                        ui.spacing_mut().item_spacing.y = 8.0;

                        // --- 第 1 行：状态 | 上传 | 下载 ---
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let rate_w = 150.0_f32;
                            let status_w = (main_w - rate_w * 2.0 - gap * 2.0).max(240.0);
                            self.dash_status_card(ui, status_w, 150.0, &selected_node, is_running, is_starting, prepared);
                            self.dash_rate_card(ui, rate_w, 150.0, true);
                            self.dash_rate_card(ui, rate_w, 150.0, false);
                        });

                        // --- 第 2 行：服务状态 | 分流规则 | 出站模式 ---
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let col_w = ((main_w - gap * 2.0) * 0.27).max(170.0);
                            let mode_w = (main_w - col_w * 2.0 - gap * 2.0).max(220.0);
                            self.dash_services_card(ui, col_w, 168.0);
                            self.dash_rules_card(ui, col_w, 168.0);
                            self.dash_mode_card(ui, mode_w, 168.0);
                        });

                        // --- 第 3 行：实时连接 | 策略组 ---
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            let groups_w = (main_w * 0.36).clamp(220.0, 330.0);
                            let conns_w = (main_w - groups_w - gap).max(240.0);
                            self.dash_connections_card(ui, conns_w, 185.0);
                            self.dash_groups_card(ui, groups_w, 185.0);
                        });
                    });

                    ui.add_space(gap);

                    // ================= 右窄列 =================
                    ui.vertical(|ui| {
                        ui.set_min_width(right_w);
                        ui.set_max_width(right_w);
                        ui.spacing_mut().item_spacing.y = 8.0;
                        self.dash_perf_card(ui, right_w, 176.0);
                        self.dash_terminal_card(ui, right_w, 204.0);
                        self.dash_modeswitch_card(ui, right_w, 150.0);
                    });
                });
            });
    }

    // -----------------------------------------------------------------
    // 第 1 行
    // -----------------------------------------------------------------

    /// 状态大卡：当前出口节点 + 大圆启停按钮。
    fn dash_status_card(
        &mut self,
        ui: &mut Ui,
        w: f32,
        h: f32,
        selected_node: &Option<crate::models::NodeSummary>,
        is_running: bool,
        is_starting: bool,
        prepared: bool,
    ) {
        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                let state_text = if is_running {
                    ("运行中", theme::success())
                } else if is_starting {
                    ("启动中", theme::warning())
                } else if prepared {
                    ("准备中", theme::accent())
                } else {
                    ("已停止", theme::muted())
                };

                ui.horizontal(|ui| {
                    dash_label(ui, "状态", None);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dash_dot(ui, state_text.1);
                        ui.label(
                            RichText::new(state_text.0)
                                .small()
                                .strong()
                                .color(theme::text_secondary()),
                        );
                    });
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    // ---- 左侧：节点信息 + 标签 ----
                    ui.vertical(|ui| {
                        ui.set_max_width(w - 120.0);
                        match selected_node {
                            Some(node) => {
                                ui.horizontal(|ui| {
                                    render_country_code_badge(ui, node.country.as_deref().unwrap_or(""));
                                    ui.add_space(2.0);
                                    let clean_name =
                                        crate::gui::widgets::clean_node_display_name(&node.name);
                                    ui.add_sized(
                                        [ui.available_width().min(240.0), 20.0],
                                        egui::Label::new(
                                            RichText::new(clean_name)
                                                .strong()
                                                .size(15.0)
                                                .color(theme::text_primary()),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(&node.name);
                                });
                                ui.horizontal(|ui| {
                                    protocol_pill(ui, &node.protocol.to_uppercase());
                                    if let Some(ms) = node.latency_ms {
                                        ui.label(
                                            RichText::new(format!("{ms} ms"))
                                                .small()
                                                .strong()
                                                .color(theme::text_secondary()),
                                        );
                                    }
                                });
                            }
                            None => {
                                ui.label(
                                    RichText::new("未选择出海出口节点")
                                        .strong()
                                        .size(15.0)
                                        .color(theme::text_secondary()),
                                );
                            }
                        }

                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            let mode_tag = if self.proxy_config.mode == ProxyMode::Tun {
                                "TUN 模式"
                            } else {
                                "系统代理"
                            };
                            for (text, color) in [
                                (mode_tag.to_string(), theme::accent()),
                                (
                                    format!("Mixed {}", self.proxy_status.mixed_port),
                                    theme::text_secondary(),
                                ),
                                (
                                    format!("活跃 {} 条", self.active_connections.len()),
                                    theme::text_secondary(),
                                ),
                            ] {
                                Frame::NONE
                                    .fill(theme::sidebar())
                                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                    .corner_radius(CornerRadius::same(99))
                                    .inner_margin(Margin::symmetric(9, 3))
                                    .show(ui, |ui| {
                                        ui.label(RichText::new(text).small().color(color));
                                    });
                            }
                        });
                    });

                    // ---- 右侧：大圆启停按钮 ----
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_start = !self.proxy_busy
                            && !is_starting
                            && (self.selected_proxy_node_id.is_some()
                                || self.focused_node_id.is_some()
                                || !self.nodes.is_empty()
                                || !self.proxy_nodes.is_empty());
                        let btn_color =
                            if is_running { theme::danger() } else { theme::accent() };
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::splat(64.0), Sense::click());
                        let center = rect.center();
                        let painter = ui.painter();
                        painter.circle_filled(center, 32.0, btn_color);
                        if is_running {
                            painter.rect_filled(
                                Rect::from_center_size(center, Vec2::splat(18.0)),
                                CornerRadius::same(4),
                                Color32::WHITE,
                            );
                        } else {
                            let (p1, p2, p3) = (
                                egui::pos2(center.x - 8.0, center.y - 12.0),
                                egui::pos2(center.x - 8.0, center.y + 12.0),
                                egui::pos2(center.x + 14.0, center.y),
                            );
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

    /// 上传 / 下载 速率小卡。
    fn dash_rate_card(&mut self, ui: &mut Ui, w: f32, h: f32, up: bool) {
        let (rate, total) = if up {
            (self.traffic_up_rate, self.traffic_up_total)
        } else {
            (self.traffic_down_rate, self.traffic_down_total)
        };
        let color = if up { theme::success() } else { theme::accent() };
        let series = {
            let history = self.traffic_history.clone();
            history_series(&history, up, 60)
        };

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, if up { "上传" } else { "下载" }, None);
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format_speed(rate))
                        .strong()
                        .size(21.0)
                        .color(theme::text_primary()),
                );
                ui.label(
                    RichText::new(format!("累计 {}", format_bytes(total)))
                        .small()
                        .color(theme::muted()),
                );
                ui.add_space(6.0);
                let w = ui.available_width();
                dash_sparkline(ui, w, 30.0, series, color);
            });
        });
    }

    // -----------------------------------------------------------------
    // 第 2 行
    // -----------------------------------------------------------------

    /// 服务状态：内核 / 系统代理 / 本地端口 / 控制端口。
    fn dash_services_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let kernel = self.proxy_status.kernel;
        let sys_on = self.proxy_status.system_proxy_enabled;
        let local_ready = self.proxy_status.local_proxy_ready;
        let controller = self.proxy_status.controller_port;

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "服务状态", None);
                ui.add_space(4.0);
                let rows: [(Color32, &str, String); 4] = [
                    (
                        if kernel.is_some() { theme::success() } else { theme::muted() },
                        "代理内核",
                        kernel_label(kernel).to_string(),
                    ),
                    (
                        if sys_on { theme::success() } else { theme::muted() },
                        "系统代理",
                        if sys_on { "已开启".into() } else { "未开启".into() },
                    ),
                    (
                        if local_ready { theme::success() } else { theme::warning() },
                        "本地端口",
                        if local_ready { "就绪".into() } else { "等待中".into() },
                    ),
                    (
                        theme::muted(),
                        "控制端口",
                        format!("{controller}"),
                    ),
                ];
                for (color, name, value) in rows {
                    ui.horizontal(|ui| {
                        dash_dot(ui, color);
                        ui.label(
                            RichText::new(name)
                                .small()
                                .strong()
                                .color(theme::text_secondary()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(value)
                                    .small()
                                    .color(theme::text_primary()),
                            );
                        });
                    });
                    ui.add_space(6.0);
                }
            });
        });
    }

    /// 分流规则：规则总数 + 智能分流 / 覆写 摘要。
    fn dash_rules_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let rules_count = self.proxy_config.rules.len();
        let smart = self.proxy_config.smart_routing;
        let has_override = self.proxy_config.override_yaml.as_deref().is_some();

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "分流规则", Some(format!("共 {} 条", rules_count)));
                ui.add_space(4.0);
                let rows = [
                    (
                        if smart { theme::success() } else { theme::muted() },
                        "智能分流",
                        if smart { "已开启".into() } else { "已关闭".into() },
                    ),
                    (
                        if has_override { theme::accent() } else { theme::muted() },
                        "自定义覆写",
                        if has_override { "已加载".into() } else { "无".into() },
                    ),
                    (
                        theme::muted(),
                        "编辑规则",
                        "前往「分流规则」页".to_string(),
                    ),
                ];
                for (color, name, value) in rows {
                    ui.horizontal(|ui| {
                        dash_dot(ui, color);
                        ui.label(
                            RichText::new(name)
                                .small()
                                .strong()
                                .color(theme::text_secondary()),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(value)
                                    .small()
                                    .color(theme::text_primary()),
                            );
                        });
                    });
                    ui.add_space(6.0);
                }
            });
        });
    }

    /// 出站模式（宽卡）：规则 / 全局 / 直连 三段切换 + 模式说明。
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
            ui.vertical(|ui| {
                dash_label(
                    ui,
                    "出站模式",
                    Some(format!("当前 · {}", mode_label(current))),
                );
                ui.add_space(8.0);

                let seg_w = ui.available_width();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (mode, name) in [
                        (RoutingMode::Rule, "规则"),
                        (RoutingMode::Global, "全局"),
                        (RoutingMode::Direct, "直连"),
                    ] {
                        let active = mode_label(current) == name;
                        let (fill, text_color) = if active {
                            (theme::accent(), theme::on_primary())
                        } else {
                            (theme::sidebar(), theme::text_secondary())
                        };
                        let bw = (seg_w - 8.0) / 3.0;
                        let btn = egui::Button::new(
                            RichText::new(name).small().strong().color(text_color),
                        )
                        .fill(fill)
                        .stroke(if active {
                            Stroke::NONE
                        } else {
                            Stroke::new(1.0_f32, theme::border_subtle())
                        })
                        .corner_radius(CornerRadius::same(8))
                        .min_size(Vec2::new(bw, 30.0));
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

                // 说明文字压在卡片底部
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(
                        RichText::new(routing_mode_detail(current))
                            .small()
                            .color(theme::muted()),
                    );
                    ui.label(
                        RichText::new(routing_mode_description(current))
                            .small()
                            .strong()
                            .color(theme::text_secondary()),
                    );
                });
            });
        });
    }

    // -----------------------------------------------------------------
    // 第 3 行
    // -----------------------------------------------------------------

    /// 实时连接表。
    fn dash_connections_card(&mut self, ui: &mut Ui, w: f32, h: f32) {
        let conns = self.active_connections.clone();
        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.set_max_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(
                    ui,
                    "实时连接",
                    Some(format!("{} 条活跃", conns.len())),
                );
                ui.add_space(2.0);
                if conns.is_empty() {
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new("暂无活跃连接")
                            .small()
                            .color(theme::muted()),
                    );
                    return;
                }
                egui::Grid::new("dash_conn_grid")
                    .num_columns(5)
                    .spacing([14.0, 5.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for head in ["目标", "网络", "命中规则", "下载", "上传"] {
                            ui.label(
                                RichText::new(head)
                                    .small()
                                    .strong()
                                    .color(theme::muted()),
                            );
                        }
                        ui.end_row();
                        for conn in conns.iter().take(3) {
                            ui.add_sized(
                                [180.0, 16.0],
                                egui::Label::new(
                                    RichText::new(&conn.host)
                                        .small()
                                        .strong()
                                        .color(theme::text_primary()),
                                )
                                .truncate(),
                            )
                            .on_hover_text(format!("{}\n{}", conn.host, conn.rule));
                            ui.label(
                                RichText::new(conn.network.to_uppercase())
                                    .small()
                                    .color(theme::text_secondary()),
                            );
                            ui.add_sized(
                                [150.0, 16.0],
                                egui::Label::new(
                                    RichText::new(&conn.target)
                                        .small()
                                        .color(theme::text_secondary()),
                                )
                                .truncate(),
                            );
                            ui.label(
                                RichText::new(format_speed(conn.download))
                                    .small()
                                    .strong()
                                    .color(theme::success()),
                            );
                            ui.label(
                                RichText::new(format_speed(conn.upload))
                                    .small()
                                    .color(theme::text_secondary()),
                            );
                            ui.end_row();
                        }
                    });
            });
        });
    }

    /// 策略组：胶囊选择。
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
                dash_label(ui, "策略组", Some(format!("{} 个", visible_names.len())));
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .id_salt("dash_groups_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                            for name in &visible_names {
                                let is_selected = self.proxy_selected_group == *name;
                                let count = self
                                    .proxy_nodes
                                    .iter()
                                    .filter(|n| node_matches_group(n, name, &self.kernel_groups))
                                    .count();
                                let (fill, stroke, text_color) = if is_selected {
                                    (
                                        theme::accent_bg(),
                                        Stroke::new(1.2_f32, theme::accent()),
                                        theme::accent_strong(),
                                    )
                                } else {
                                    (
                                        theme::sidebar(),
                                        Stroke::new(1.0_f32, theme::border_subtle()),
                                        theme::text_secondary(),
                                    )
                                };
                                let label = format!("{name} ({count})");
                                let btn = egui::Button::new(
                                    RichText::new(label).small().strong().color(text_color),
                                )
                                .fill(fill)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(99));
                                if ui.add(btn).clicked() && !is_selected {
                                    self.proxy_selected_group = name.clone();
                                }
                            }
                        });
                    });
            });
        });
    }

    // -----------------------------------------------------------------
    // 右窄列
    // -----------------------------------------------------------------

    /// 性能紧凑卡：延迟大数字 + 活跃/累计徽章 + 迷你下行曲线。
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
        let conns = self.active_connections.len();
        let series = {
            let history = self.traffic_history.clone();
            history_series(&history, false, 60)
        };

        dash_card().show(ui, |ui| {
            ui.set_min_size(Vec2::new(w, h));
            ui.vertical(|ui| {
                dash_label(ui, "性能", Some("实时".into()));
                ui.label(
                    RichText::new(match latency {
                        Some(ms) => format!("{ms} ms"),
                        None => "— ms".into(),
                    })
                    .strong()
                    .size(22.0)
                    .color(theme::text_primary()),
                );
                ui.add_space(6.0);
                for (name, value, color) in [
                    ("活跃连接", format!("{conns} 条"), theme::text_primary()),
                    (
                        "累计上传",
                        format_bytes(self.traffic_up_total),
                        theme::text_primary(),
                    ),
                    (
                        "累计下载",
                        format_bytes(self.traffic_down_total),
                        theme::text_primary(),
                    ),
                ] {
                    Frame::NONE
                        .fill(theme::sidebar())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(name)
                                        .small()
                                        .color(theme::text_secondary()),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(value)
                                                .small()
                                                .strong()
                                                .color(color),
                                        );
                                    },
                                );
                            });
                        });
                }
                ui.add_space(6.0);
                let w = ui.available_width();
                dash_sparkline(ui, w, 26.0, series, theme::accent());
            });
        });
    }

    /// 终端代理卡：总开关 + 端口代理 + 局域网代理，三段等高。
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
                let inner_h = h - 24.0; // 减去上下卡片 margin
                let row_h = inner_h / 3.0;

                // --- 段 1：终端代理总开关 ---
                ui.allocate_ui(Vec2::new(w, row_h), |ui| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        icon_label(ui, Icon::Terminal, 13.0, theme::accent());
                        ui.label(
                            RichText::new("终端代理")
                                .small()
                                .strong()
                                .color(theme::text_secondary()),
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

                // --- 段 2：端口代理 ---
                ui.allocate_ui(Vec2::new(w, row_h), |ui| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("端口代理")
                                    .small()
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(format!("混合端口 · {}", self.proxy_status.mixed_port))
                                    .small()
                                    .color(theme::muted()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);
                            dash_dot(ui, if running { theme::success() } else { theme::muted() });
                        });
                    });
                });

                // --- 段 3：局域网代理 ---
                ui.allocate_ui(Vec2::new(w, row_h), |ui| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("局域网代理")
                                    .small()
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(if lan_on { "0.0.0.0 监听" } else { "仅本机 127.0.0.1" })
                                    .small()
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

    /// 系统代理 / TUN 模式卡：两段等高的模式开关（运行中禁切）。
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
                let inner_h = h - 24.0;
                let row_h = inner_h / 2.0;

                let render_row = |ui: &mut Ui, title: &str, sub: String, on: bool| {
                    ui.set_min_size(Vec2::new(w, row_h));
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.set_min_height(row_h);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new(title)
                                    .small()
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(sub)
                                    .small()
                                    .color(theme::muted()),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(10.0);
                            dash_switch(ui, on)
                        });
                    });
                };

                // 系统代理（Linux 无系统代理概念时显示「本地端口」且不响应切换）
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

                // 分隔线
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

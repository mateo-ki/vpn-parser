use super::groups::{node_matches_group, visible_group_names};
use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::ProxyPanel;
use crate::gui::widgets::{
    country_flag_and_name, format_bytes, format_speed, protocol_pill,
    render_country_code_badge, render_health_strip, render_signal_bars, render_speedometer,
    render_traffic_waveform_card,
};
use crate::gui::VpnApp;
use crate::proxy::{AutoRoutingStrategy, ProxyMode, ProxyState};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

/// 徽章的输入：这一轮有没有查到结果、查失败过、还是正在查。
#[derive(Debug, Default)]
pub(crate) struct EgressRiskView<'a> {
    pub(crate) risk: Option<&'a crate::ip_risk::IpRisk>,
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
            .id_salt("proxy_charts_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let card_outer_w = (ui.available_width() - 4.0).max(100.0);
                let card_inner_w = (card_outer_w - 28.0).max(60.0);

                // 1. 核心主控岛卡片（默认撑开固定高度，无论是否选中节点均保持稳定结构）
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(card_inner_w);
                        ui.set_min_height(64.0);
                        // --- 第 1 行：状态呼吸光环 + 标题与监听端口 ---
                        ui.horizontal(|ui| {
                            let (beacon_color, pulse_color) = if is_running {
                                let active_green = Color32::from_rgb(52, 199, 89);
                                (active_green, active_green.gamma_multiply(0.25))
                            } else if is_starting || self.switching_node_id.is_some() {
                                (theme::warning(), theme::warning().gamma_multiply(0.25))
                            } else if prepared {
                                (theme::accent(), theme::accent().gamma_multiply(0.25))
                            } else {
                                (theme::muted(), theme::border_subtle())
                            };
                            let (beacon_rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                            ui.painter().circle_filled(beacon_rect.center(), 8.0, pulse_color);
                            ui.painter().circle_filled(beacon_rect.center(), 4.5, beacon_color);
                            ui.add_space(4.0);

                            ui.vertical(|ui| {
                                let (status_title, status_desc) = if is_running {
                                    let mode_tag = if self.proxy_config.mode == ProxyMode::Tun {
                                        self.tr("TUN 全局虚拟网卡")
                                    } else {
                                        self.tr("系统代理")
                                    };
                                    (
                                        self.tr("VPN 代理运行中"),
                                        format!("已接管流量 · 模式: {mode_tag} · 端口: Mixed {}", self.proxy_status.mixed_port),
                                    )
                                } else if is_starting {
                                    (
                                        self.tr("正在启动代理..."),
                                        "内核加载中，正在验证网络出口与路由规则...".to_string(),
                                    )
                                } else if prepared {
                                    (
                                        "预备中 (正在下载内核)",
                                        "正在检测并下载最新代理核心执行文件...".to_string(),
                                    )
                                } else {
                                    (
                                        self.tr("代理服务已就绪"),
                                        format!("核心未运行 · 本地监听端口: Mixed {}", self.proxy_status.mixed_port),
                                    )
                                };

                                ui.label(
                                    RichText::new(status_title)
                                        .strong()
                                        .size(16.0)
                                        .color(theme::text_primary()),
                                );
                                ui.label(
                                    RichText::new(status_desc)
                                        .small()
                                        .color(theme::text_secondary()),
                                );
                            });
                        });

                        ui.add_space(8.0);

                        // --- 第 2 行：当前出海出口节点微晶徽标卡片（默认撑开固定高度） ---
                        Frame::NONE
                            .fill(theme::sidebar())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(9))
                            .inner_margin(Margin::symmetric(12, 6))
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width());
                                ui.set_min_height(22.0);
                                ui.horizontal(|ui| {
                                    if let Some(node) = &selected_node {
                                        let country_code = node.country.as_deref().unwrap_or("");
                                        let (_flag, country_name) =
                                            country_flag_and_name(country_code);
                                        render_country_code_badge(ui, country_code);
                                        ui.add_space(2.0);
                                        ui.label(
                                            RichText::new(country_name)
                                                .strong()
                                                .size(13.0)
                                                .color(theme::text_primary()),
                                        );
                                        ui.separator();
                                        protocol_pill(ui, &node.protocol.to_uppercase());
                                        let clean_name = crate::gui::widgets::clean_node_display_name(&node.name);
                                        let name_max_w = (ui.available_width() - 260.0).clamp(60.0, 320.0);
                                        ui.add_sized(
                                            [name_max_w, 20.0],
                                            egui::Label::new(
                                                RichText::new(clean_name)
                                                    .strong()
                                                    .size(12.5)
                                                    .color(theme::text_primary()),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(&node.name);

                                        if self.switching_node_id.is_some() {
                                            icon_label(ui, Icon::Lightning, 11.0, theme::warning());
                                            ui.label(RichText::new("切换中...").small().strong().color(theme::warning()));
                                        }

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let lat_str = node
                                                .latency_ms
                                                .map(|ms| format!("{ms} ms"))
                                                .unwrap_or_else(|| "—".into());
                                            ui.label(
                                                RichText::new(lat_str)
                                                    .small()
                                                    .strong()
                                                    .color(theme::text_secondary()),
                                            );
                                            ui.add_space(2.0);
                                            render_signal_bars(ui, node.latency_ms);
                                            if ui.available_width() > 170.0 {
                                                let view = self.egress_risk_view();
                                                let kind = node_ip_kind(&view);
                                                let color = node_ip_kind_color(kind);
                                                ui.separator();
                                                icon_label(ui, Icon::Shield, 11.0, color);
                                                ui.label(
                                                    RichText::new(self.tr(kind.label()))
                                                        .small()
                                                        .strong()
                                                        .color(color),
                                                )
                                                .on_hover_text(match (view.risk, kind) {
                                                    (Some(risk), NodeIpKind::PublicProxy) => format!(
                                                        "{}\n这颗 IP 被公开代理/VPN 出口库标记（不是住宅拨号），比普通机房更容易被站点风控",
                                                        risk.headline()
                                                    ),
                                                    (Some(risk), _) => risk.headline(),
                                                    (None, _) if view.failed => format!(
                                                        "查这颗节点的归属失败了：{}",
                                                        self.ip_risk_error
                                                            .as_deref()
                                                            .unwrap_or("不知道原因"),
                                                    ),
                                                    (None, _) => "还没查到这颗节点的 IP 归属：代理跑起来后会自动查一次，也可以从节点卡片右键立刻查".to_string(),
                                                });
                                            }
                                            ui.separator();
                                            let conns = self.active_connections.len();
                                            ui.label(
                                                RichText::new(format!("活跃: {conns}"))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );
                                            if ui.available_width() > 140.0 {
                                                ui.separator();
                                                ui.label(
                                                    RichText::new(format!("↑ {}", format_speed(self.traffic_up_rate)))
                                                        .small()
                                                        .color(theme::text_secondary()),
                                                );
                                                ui.label(
                                                    RichText::new(format!("↓ {}", format_speed(self.traffic_down_rate)))
                                                        .small()
                                                        .strong()
                                                        .color(theme::accent()),
                                                );
                                            }
                                        });
                                    } else if self.proxy_config.url_test {
                                        icon_label(ui, Icon::Lightning, 14.0, theme::accent());
                                        ui.label(
                                            RichText::new("⚡ 策略组自动选优已激活")
                                                .strong()
                                                .size(13.0)
                                                .color(theme::accent()),
                                        );
                                        let hint_max_w = (ui.available_width() - 240.0).clamp(60.0, 260.0);
                                        ui.add_sized(
                                            [hint_max_w, 20.0],
                                            egui::Label::new(
                                                RichText::new("（自动调度低延迟出口）")
                                                    .small()
                                                    .color(theme::muted()),
                                            )
                                            .truncate(),
                                        );

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let conns = self.active_connections.len();
                                            ui.label(
                                                RichText::new(format!("活跃: {conns}"))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );
                                            if ui.available_width() > 140.0 {
                                                ui.separator();
                                                ui.label(
                                                    RichText::new(format!("↑ {}", format_speed(self.traffic_up_rate)))
                                                        .small()
                                                        .color(theme::text_secondary()),
                                                );
                                                ui.label(
                                                    RichText::new(format!("↓ {}", format_speed(self.traffic_down_rate)))
                                                        .small()
                                                        .strong()
                                                        .color(theme::accent()),
                                                );
                                            }
                                        });
                                    } else {
                                        icon_label(ui, Icon::Globe, 13.0, theme::muted());
                                        ui.label(
                                            RichText::new("未选择出海出口节点")
                                                .strong()
                                                .size(13.0)
                                                .color(theme::text_secondary()),
                                        );
                                        ui.separator();
                                        let hint_max_w = (ui.available_width() - 220.0).clamp(60.0, 240.0);
                                        ui.add_sized(
                                            [hint_max_w, 20.0],
                                            egui::Label::new(
                                                RichText::new("可在下方选择自动调度或自选节点")
                                                    .small()
                                                    .color(theme::muted()),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text("可在「可用节点」列表手动挑选节点或在下方切换自动调度");

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new("— ms").small().color(theme::muted()));
                                            ui.add_space(2.0);
                                            render_signal_bars(ui, None);
                                            ui.separator();
                                            ui.label(RichText::new("活跃: 0").small().color(theme::muted()));
                                            if ui.available_width() > 140.0 {
                                                ui.separator();
                                                ui.label(RichText::new("↑ 0 B/s · ↓ 0 B/s").small().color(theme::muted()));
                                            }
                                        });
                                    }
                                });
                            });
                    });

                ui.add_space(6.0);

                // 2. 全网节点延迟频谱带 + 节点协议组成分布（紧跟主控岛，两列卡片直观汇总）
                const SPECTRUM_PROTOCOL_INNER_H: f32 = 74.0;
                if let Some(s) = self.stats.as_ref() {
                    let total_w = (ui.available_width() - 4.0).max(100.0);
                    let is_narrow_charts = total_w < 680.0;

                    if is_narrow_charts {
                        ui.vertical(|ui| {
                            self.render_latency_spectrum_card(ui, Some(total_w), Some(SPECTRUM_PROTOCOL_INNER_H));
                            ui.add_space(6.0);
                            self.render_protocol_distribution_card(ui, s, Some(total_w), Some(SPECTRUM_PROTOCOL_INNER_H));
                        });
                    } else {
                        let gap = 8.0;
                        let left_w = ((total_w - gap) * 0.48).max(270.0);
                        let right_w = (total_w - left_w - gap).max(270.0);

                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.x = gap;
                            self.render_latency_spectrum_card(ui, Some(left_w), Some(SPECTRUM_PROTOCOL_INNER_H));
                            self.render_protocol_distribution_card(ui, s, Some(right_w), Some(SPECTRUM_PROTOCOL_INNER_H));
                        });
                    }
                }

                ui.add_space(6.0);

                // 3. 双翼网络实时活动监视器看板（测速仪表盘 + 流量大波形与健康带）
                const GAUGE_CARD_INNER_H: f32 = 118.0;
                let avail_w = ui.available_width();
                let is_narrow_gauges = avail_w < 700.0;
                let down_rate = self.traffic_down_rate;
                let up_rate = self.traffic_up_rate;
                let render_speedometer_box = |ui: &mut egui::Ui, target_w: Option<f32>| {
                    Frame::NONE
                        .fill(theme::sidebar())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            let card_w = target_w.unwrap_or_else(|| (ui.available_width() - 4.0).max(100.0));
                            let inner_w = (card_w - 28.0).max(60.0);
                            ui.set_max_width(inner_w);
                            ui.set_min_height(GAUGE_CARD_INNER_H);
                            ui.set_max_height(GAUGE_CARD_INNER_H);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Radar, 13.0, theme::accent());
                                    ui.label(
                                        RichText::new("实时下行带宽")
                                            .strong()
                                            .small()
                                            .color(theme::text_secondary()),
                                    );
                                });
                                ui.add_space(4.0);
                                render_speedometer(
                                    ui,
                                    down_rate,
                                    up_rate,
                                    is_running,
                                );
                            });
                        });
                };

                if is_narrow_gauges {
                    let full_w = (avail_w - 4.0).max(100.0);
                    ui.vertical(|ui| {
                        render_speedometer_box(ui, Some(full_w));
                        ui.add_space(8.0);
                        self.render_waveform_health_card(ui, is_running, Some(full_w));
                    });
                } else {
                    let total_w = (avail_w - 4.0).max(200.0);
                    let gap = 8.0_f32;
                    let speed_outer_w = 172.0_f32;
                    let health_outer_w = (total_w - speed_outer_w - gap).max(200.0);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        render_speedometer_box(ui, Some(speed_outer_w));
                        self.render_waveform_health_card(ui, is_running, Some(health_outer_w));
                    });
                }

                ui.add_space(6.0);

                // 4. 分流策略组与流量调度决策中心（控制台底部）
                let visible_names = visible_group_names(
                    self.proxy_config.smart_routing,
                    self.proxy_config.override_yaml.as_deref(),
                    &self.kernel_groups,
                );
                if !visible_names.iter().any(|name| name == &self.proxy_selected_group) {
                    self.proxy_selected_group = "全部".to_string();
                }

                let is_auto = self.proxy_config.url_test;
                let current_strategy = self.proxy_config.auto_strategy;

                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(card_inner_w);
                        ui.vertical(|ui| {
                            // --- 第 1 行：分流策略组胶囊（支持平铺窄窗口下自适应折行） ---
                            ui.horizontal_wrapped(|ui| {
                                icon_label(ui, Icon::Shield, 13.0, theme::accent());
                                ui.label(
                                    RichText::new("策略组:")
                                        .small()
                                        .strong()
                                        .color(theme::text_secondary()),
                                )
                                .on_hover_text("智能分流策略组：点击切换场景节点池");

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

                                    let pill_label = match name.as_str() {
                                        "全部" => format!("🌐 {name} ({count})"),
                                        "节点选择" => format!("🎯 {name} ({count})"),
                                        "自动选择" => format!("⚡ {name} ({count})"),
                                        "国外媒体" => format!("🎬 {name} ({count})"),
                                        "国内媒体" => format!("📺 {name} ({count})"),
                                        "微软服务" => format!("🪟 {name} ({count})"),
                                        "电报信息" => format!("✈️ {name} ({count})"),
                                        "苹果服务" => format!("🍎 {name} ({count})"),
                                        "漏网之鱼" => format!("🐟 {name} ({count})"),
                                        _ => format!("🏷 {name} ({count})"),
                                    };

                                    let btn = egui::Button::new(
                                        RichText::new(pill_label)
                                            .small()
                                            .strong()
                                            .color(text_color),
                                    )
                                    .fill(fill)
                                    .stroke(stroke)
                                    .corner_radius(CornerRadius::same(12));

                                    let tooltip = match name.as_str() {
                                        "全部" => "包含当前所有已加载节点的全局大池",
                                        "自动选择" => "URL-Test / 延迟自动选优策略组",
                                        "国外媒体" => "YouTube / Netflix / Spotify 等流媒体分流策略组",
                                        "漏网之鱼" => "未命中任何具体规则时的默认最终路由兜底组",
                                        _ => "点击切换至该策略组",
                                    };

                                    let resp = ui.add(btn).on_hover_text(tooltip);
                                    if resp.clicked() && !is_selected {
                                        self.proxy_selected_group = name.clone();
                                    }
                                }
                            });

                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);

                            // --- 第 2 行：流量调度策略（选优/故障/负载均衡/手动指定，支持窄窗口折行） ---
                            ui.horizontal_wrapped(|ui| {
                                icon_label(ui, Icon::Lightning, 12.0, theme::accent());
                                ui.label(
                                    RichText::new("调度:")
                                        .small()
                                        .strong()
                                        .color(theme::text_secondary()),
                                )
                                .on_hover_text("流量路由调度策略：配置内核对多节点的选路决策规则");

                                let is_url_test = is_auto && current_strategy == AutoRoutingStrategy::UrlTest;
                                if ui
                                    .add(icons::selectable_icon_btn(Icon::Lightning, "延迟选优", is_url_test))
                                    .on_hover_text(format!("{}\n（点击自动激活并启动代理）", AutoRoutingStrategy::UrlTest.description()))
                                    .clicked()
                                {
                                    self.proxy_config.url_test = true;
                                    self.proxy_config.auto_strategy = AutoRoutingStrategy::UrlTest;
                                    if self.selected_proxy_node_id.is_none() {
                                        if let Some(node) = self.proxy_nodes.first().or(self.nodes.first()) {
                                            self.selected_proxy_node_id = Some(node.id.clone());
                                            self.selected_proxy_node_name = Some(node.name.clone());
                                        }
                                    }
                                    let running = matches!(
                                        self.proxy_status.state,
                                        ProxyState::Running | ProxyState::Degraded
                                    );
                                    if running {
                                        self.save_proxy_config();
                                    } else {
                                        self.proxy_busy = false;
                                        self.start_proxy();
                                    }
                                }

                                let is_fallback = is_auto && current_strategy == AutoRoutingStrategy::Fallback;
                                if ui
                                    .add(icons::selectable_icon_btn(Icon::Shield, "故障转移", is_fallback))
                                    .on_hover_text(format!("{}\n（点击自动激活并启动代理）", AutoRoutingStrategy::Fallback.description()))
                                    .clicked()
                                {
                                    self.proxy_config.url_test = true;
                                    self.proxy_config.auto_strategy = AutoRoutingStrategy::Fallback;
                                    if self.selected_proxy_node_id.is_none() {
                                        if let Some(node) = self.proxy_nodes.first().or(self.nodes.first()) {
                                            self.selected_proxy_node_id = Some(node.id.clone());
                                            self.selected_proxy_node_name = Some(node.name.clone());
                                        }
                                    }
                                    let running = matches!(
                                        self.proxy_status.state,
                                        ProxyState::Running | ProxyState::Degraded
                                    );
                                    if running {
                                        self.save_proxy_config();
                                    } else {
                                        self.proxy_busy = false;
                                        self.start_proxy();
                                    }
                                }

                                let is_load_balance = is_auto && current_strategy == AutoRoutingStrategy::LoadBalance;
                                if ui
                                    .add(icons::selectable_icon_btn(Icon::Rocket, "负载均衡", is_load_balance))
                                    .on_hover_text(format!("{}\n（点击自动激活并启动代理）", AutoRoutingStrategy::LoadBalance.description()))
                                    .clicked()
                                {
                                    self.proxy_config.url_test = true;
                                    self.proxy_config.auto_strategy = AutoRoutingStrategy::LoadBalance;
                                    if self.selected_proxy_node_id.is_none() {
                                        if let Some(node) = self.proxy_nodes.first().or(self.nodes.first()) {
                                            self.selected_proxy_node_id = Some(node.id.clone());
                                            self.selected_proxy_node_name = Some(node.name.clone());
                                        }
                                    }
                                    let running = matches!(
                                        self.proxy_status.state,
                                        ProxyState::Running | ProxyState::Degraded
                                    );
                                    if running {
                                        self.save_proxy_config();
                                    } else {
                                        self.proxy_busy = false;
                                        self.start_proxy();
                                    }
                                }

                                let is_manual = !is_auto;
                                if ui
                                    .add(icons::selectable_icon_btn(Icon::Pin, "手动指定", is_manual))
                                    .on_hover_text("点击切换为手动指定，并立即跳转至可用节点列表挑选节点")
                                    .clicked()
                                {
                                    self.proxy_config.url_test = false;
                                    let running = matches!(
                                        self.proxy_status.state,
                                        ProxyState::Running | ProxyState::Degraded
                                    );
                                    if running {
                                        self.save_proxy_config();
                                    }
                                    self.proxy_panel = ProxyPanel::Nodes;
                                }
                            });
                        });
                    });

                ui.add_space(16.0);
            });
    }

    fn render_waveform_health_card(
        &mut self,
        ui: &mut Ui,
        is_running: bool,
        target_w: Option<f32>,
    ) {
        const GAUGE_CARD_INNER_H: f32 = 118.0;
        Frame::NONE
            .fill(theme::sidebar())
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                let card_w = target_w.unwrap_or_else(|| (ui.available_width() - 4.0).max(100.0));
                let inner_w = (card_w - 28.0).max(60.0);
                ui.set_max_width(inner_w);
                ui.set_min_height(GAUGE_CARD_INNER_H);
                ui.set_max_height(GAUGE_CARD_INNER_H);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("出海健康 (24h)")
                                .small()
                                .strong()
                                .color(theme::text_secondary()),
                        );
                        render_health_strip(ui, &self.health_buckets);

                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "↑ {} · ↓ {}",
                                            format_bytes(self.traffic_up_total),
                                            format_bytes(self.traffic_down_total)
                                        ))
                                        .small()
                                        .color(theme::muted()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!(
                                    "本会话累计流量：上传 {} · 下载 {}",
                                    format_bytes(self.traffic_up_total),
                                    format_bytes(self.traffic_down_total)
                                ));
                            },
                        );
                    });

                    ui.add_space(4.0);
                    let waveform_h = if self.auto_failover { 74.0 } else { 94.0 };
                    render_traffic_waveform_card(
                        ui,
                        &self.traffic_history,
                        is_running,
                        waveform_h,
                    );

                    if self.auto_failover {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Shield, 11.0, theme::accent_strong());
                            let detail = self
                                .failover_last_reason
                                .as_deref()
                                .unwrap_or("已开启，最近实测正常");
                            let detail: String = if detail.chars().count() > 18 {
                                let mut s: String = detail.chars().take(18).collect();
                                s.push('…');
                                s
                            } else {
                                detail.to_string()
                            };
                            let max_lbl_w = (ui.available_width() - 110.0).max(60.0);
                            ui.add_sized(
                                [max_lbl_w, 16.0],
                                egui::Label::new(
                                    RichText::new(format!(
                                        "自动切换: {}次 · {detail}",
                                        self.failover_switches
                                    ))
                                    .small()
                                    .color(theme::muted()),
                                )
                                .truncate(),
                            );
                            if ui
                                .link(format!("记录({}次)", self.switch_today))
                                .clicked()
                            {
                                self.show_switch_log = true;
                                self.reload_egress_switches();
                            }
                        });
                    }
                });
            });
    }

    fn render_latency_spectrum_card(
        &self,
        ui: &mut Ui,
        target_w: Option<f32>,
        min_inner_h: Option<f32>,
    ) {
        Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                let card_w = target_w.unwrap_or_else(|| (ui.available_width() - 4.0).max(100.0));
                let inner_w = (card_w - 28.0).max(60.0);
                ui.set_max_width(inner_w);
                if let Some(inner_h) = min_inner_h {
                    ui.set_min_height(inner_h);
                    ui.set_max_height(inner_h);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Lightning, 14.0, theme::accent());
                        ui.label(
                            RichText::new("全网延迟阶梯频谱分布")
                                .strong()
                                .size(13.0)
                                .color(theme::text_primary()),
                        );
                    });
                    ui.add_space(5.0);
                    let spectrum = crate::gui::widgets::LatencySpectrumStats::from_latencies(
                        self.nodes.iter().map(|n| n.latency_ms),
                    );
                    crate::gui::widgets::render_latency_spectrum(ui, &spectrum);
                    ui.add_space(5.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
                        ui.label(
                            RichText::new(format!("⚡ <100ms: {}", spectrum.ultra_fast))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(82, 121, 111)),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!("● 100-200ms: {}", spectrum.fast))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(122, 154, 139)),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!("● 200-400ms: {}", spectrum.medium))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(198, 138, 76)),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!("● >400ms: {}", spectrum.slow))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(158, 90, 84)),
                        );
                    });
                });
            });
    }

    fn render_protocol_distribution_card(
        &self,
        ui: &mut Ui,
        s: &crate::models::Stats,
        target_w: Option<f32>,
        min_inner_h: Option<f32>,
    ) {
        Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                let card_w = target_w.unwrap_or_else(|| (ui.available_width() - 4.0).max(100.0));
                let inner_w = (card_w - 28.0).max(60.0);
                ui.set_max_width(inner_w);
                if let Some(inner_h) = min_inner_h {
                    ui.set_min_height(inner_h);
                    ui.set_max_height(inner_h);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    crate::gui::widgets::render_protocol_donut_chart_sized(
                        ui,
                        &s.protocol_stats,
                        s.node_count,
                        68.0,
                    );
                    ui.add_space(10.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Radar, 13.0, theme::accent());
                            ui.label(
                                RichText::new("节点协议组成分布")
                                    .strong()
                                    .size(13.0)
                                    .color(theme::text_primary()),
                            );
                            if ui.available_width() > 160.0 {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!("可用: {}", s.alive_count))
                                            .small()
                                            .strong()
                                            .color(Color32::from_rgb(82, 121, 111)),
                                    );
                                    if ui.available_width() > 70.0 {
                                        ui.separator();
                                        ui.label(
                                            RichText::new(format!("共 {} 节点", s.node_count))
                                                .small()
                                                .color(theme::text_secondary()),
                                        );
                                    }
                                });
                            }
                        });
                        ui.add_space(2.0);

                        let mut all_stats: Vec<(String, i64, f64)> = s
                            .protocol_stats
                            .iter()
                            .map(|stat| {
                                let pct = if s.node_count > 0 {
                                    (stat.count as f64 / s.node_count as f64) * 100.0
                                } else {
                                    0.0
                                };
                                (stat.protocol.to_uppercase(), stat.count, pct)
                            })
                            .collect();

                        let other_pill: Option<(String, i64, f64, String)>;
                        let (row1, row2) = if all_stats.len() <= 3 {
                            other_pill = None;
                            (all_stats, vec![])
                        } else if all_stats.len() <= 5 {
                            other_pill = None;
                            let r2 = all_stats.split_off(3);
                            (all_stats, r2)
                        } else {
                            let remainder = &all_stats[4..];
                            let rest_count: i64 = remainder.iter().map(|item| item.1).sum();
                            let rest_pct = if s.node_count > 0 {
                                (rest_count as f64 / s.node_count as f64) * 100.0
                            } else {
                                0.0
                            };
                            let tip = remainder
                                .iter()
                                .map(|(p, c, pct)| format!("{p}: {c} ({pct:.0}%)"))
                                .collect::<Vec<_>>()
                                .join(" · ");
                            other_pill = Some(("其它".to_string(), rest_count, rest_pct, tip));

                            let r2: Vec<(String, i64, f64)> = all_stats[2..4].to_vec();
                            all_stats.truncate(2);
                            (all_stats, r2)
                        };

                        if row1.is_empty() {
                            ui.add_space(2.0);
                            ui.label(RichText::new("暂无协议数据").small().color(theme::muted()));
                        } else {
                            // 渲染第 1 行
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                                for (proto, count, pct) in &row1 {
                                    Frame::NONE
                                        .fill(theme::sidebar())
                                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                        .corner_radius(CornerRadius::same(6))
                                        .inner_margin(Margin::symmetric(5, 1))
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(format!("{proto} {count} ({pct:.0}%)"))
                                                    .small()
                                                    .color(theme::text_primary()),
                                            );
                                        });
                                }
                            });

                            // 渲染第 2 行（若有）
                            if !row2.is_empty() || other_pill.is_some() {
                                ui.add_space(2.0);
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                                    for (proto, count, pct) in &row2 {
                                        Frame::NONE
                                            .fill(theme::sidebar())
                                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                            .corner_radius(CornerRadius::same(6))
                                            .inner_margin(Margin::symmetric(5, 1))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new(format!("{proto} {count} ({pct:.0}%)"))
                                                        .small()
                                                        .color(theme::text_primary()),
                                                );
                                            });
                                    }
                                    if let Some((name, count, pct, tip)) = other_pill {
                                        Frame::NONE
                                            .fill(theme::sidebar())
                                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                            .corner_radius(CornerRadius::same(6))
                                            .inner_margin(Margin::symmetric(5, 1))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new(format!("{name} {count} ({pct:.0}%)"))
                                                        .small()
                                                        .color(theme::muted()),
                                                );
                                            })
                                            .response
                                            .on_hover_text(tip);
                                    }
                                });
                            }
                        }
                    });
                });
            });
    }


}

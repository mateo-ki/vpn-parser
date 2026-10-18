use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    ActiveConnectionInfo, ConnectionsTab, DnsRecord, GroupsTab, KernelGroup, ProxyPanel, ProxySortBy, ProxyView, ProxyViewRow, RuleDraftKind, RulesTab,
};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, country_flag_and_name, danger_btn, dns_mode_label,
    format_bytes, format_speed, header_cell, macos_segmented_tabs, panel_frame, panel_tab, primary_btn,
    protocol_pill, render_country_code_badge, render_health_strip, render_signal_bars, render_speedometer,
    render_traffic_waveform_card, routing_mode_description, routing_mode_detail,
    rule_action_label, rule_matcher_label, sanitize_node_name, section_title, status_badge, tun_stack_label,
};
use crate::gui::VpnApp;
use crate::kernel::{KernelKind, KernelPreference};
use crate::models::NodeSummary;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules, recommended_traffic_rules,
    social_telegram_rules, streaming_media_rules, AutoRoutingStrategy, DnsEnhancedMode, ProxyMode, ProxyState,
    RoutingMode, RuleAction, TrafficRule, TunStack, URL_TEST_GROUP,
};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;

pub const CLASH_VERGE_RECOMMENDED_OVERRIDE: &str = r#"# =========================================================
# Clash Verge Rev / Mihomo 通用推荐覆写片段
# =========================================================
proxy-groups:
  - name: 多线并发下载
    type: load-balance
    strategy: round-robin
    url: https://cp.cloudflare.com/generate_204
    interval: 300
    timeout: 2000
    lazy: true
    include-all: true
    filter: "(?i)(港|新加坡|日|美|\\b(?:HK|SG|JP|US|USA)\\b|Hong[ _-]?Kong|Singapore|Japan|America|United[ _-]?States)"
    exclude-filter: "【(?:[2-9]|[1-9][0-9]+)x】|链式"

  - name: 美国节点
    type: url-test
    url: https://cp.cloudflare.com/generate_204
    interval: 300
    timeout: 2000
    lazy: true
    include-all: true
    filter: "(?i)(美|\\bUS(?:A)?\\b|America|United[ _-]?States|Los[ _-]?Angeles|San[ _-]?Jose|Seattle|Chicago|Dallas|New[ _-]?York|Virginia|Washington)"

  - name: 普通代理
    type: select
    proxies:
      - 美国节点
    include-all: true

  - name: AI分组
    type: select
    proxies:
      - 美国节点
      - 普通代理

rules:
  # 局域网 / 私有地址
  - IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
  - IP-CIDR,172.16.0.0/12,DIRECT,no-resolve
  - IP-CIDR,192.168.0.0/16,DIRECT,no-resolve
  - IP-CIDR6,fc00::/7,DIRECT,no-resolve
  - IP-CIDR6,fe80::/10,DIRECT,no-resolve
  - DST-PORT,22,DIRECT

  # AI 视频与主流 AI 服务
  - DOMAIN-SUFFIX,runwayml.com,AI分组
  - DOMAIN-KEYWORD,runwayml,AI分组
  - DOMAIN-SUFFIX,runway.com,AI分组
  - DOMAIN-SUFFIX,muse.ai,AI分组
  - DOMAIN-SUFFIX,openai.com,AI分组
  - DOMAIN-SUFFIX,chatgpt.com,AI分组
  - DOMAIN-SUFFIX,oaistatic.com,AI分组
  - DOMAIN-SUFFIX,oaiusercontent.com,AI分组
  - DOMAIN-SUFFIX,anthropic.com,AI分组
  - DOMAIN-SUFFIX,claude.ai,AI分组
  - DOMAIN-SUFFIX,claudeusercontent.com,AI分组
  - DOMAIN-SUFFIX,x.com,AI分组
  - DOMAIN-SUFFIX,grok.com,AI分组
  - DOMAIN-SUFFIX,perplexity.ai,AI分组
  - DOMAIN-SUFFIX,poe.com,AI分组
  - DOMAIN-SUFFIX,character.ai,AI分组
  - DOMAIN-SUFFIX,stability.ai,AI分组
  - DOMAIN-SUFFIX,huggingface.co,AI分组
  - DOMAIN-SUFFIX,replicate.com,AI分组
  - DOMAIN-SUFFIX,ai.com,AI分组
  - DOMAIN-SUFFIX,linux.do,AI分组
  - DOMAIN-SUFFIX,hybgzs.com,AI分组

  # Google / Gemini / YouTube -> 美国节点
  - DOMAIN,gemini.google.com,美国节点
  - DOMAIN-SUFFIX,google.com,美国节点
  - DOMAIN-SUFFIX,google.com.hk,美国节点
  - DOMAIN-SUFFIX,google.co.jp,美国节点
  - DOMAIN-SUFFIX,google.co.uk,美国节点
  - DOMAIN-SUFFIX,google.com.tw,美国节点
  - DOMAIN-SUFFIX,google.dev,美国节点
  - DOMAIN-SUFFIX,gstatic.com,美国节点
  - DOMAIN-SUFFIX,googleapis.com,美国节点
  - DOMAIN-SUFFIX,googleapis.cn,美国节点
  - DOMAIN-SUFFIX,googleusercontent.com,美国节点
  - DOMAIN-SUFFIX,ggpht.com,美国节点
  - DOMAIN-SUFFIX,gvt1.com,美国节点
  - DOMAIN-SUFFIX,gvt2.com,美国节点
  - DOMAIN-SUFFIX,1e100.net,美国节点
  - DOMAIN-SUFFIX,gmail.com,美国节点
  - DOMAIN-SUFFIX,googlemail.com,美国节点
  - DOMAIN-SUFFIX,recaptcha.net,美国节点
  - DOMAIN-SUFFIX,youtube.com,美国节点
  - DOMAIN-SUFFIX,youtu.be,美国节点
  - DOMAIN-SUFFIX,ytimg.com,美国节点
  - DOMAIN-SUFFIX,googlevideo.com,美国节点
  - DOMAIN-SUFFIX,youtube-nocookie.com,美国节点
  - GEOSITE,google,美国节点
  - GEOSITE,youtube,美国节点
  - GEOIP,google,美国节点,no-resolve

  # 其他服务
  - DOMAIN-SUFFIX,midjourney.com,美国节点
  - DOMAIN-SUFFIX,capcut.com,美国节点

  # 常用直连
  - DOMAIN-SUFFIX,bing.com,DIRECT
  - DOMAIN-SUFFIX,azureedge.net,DIRECT
  - DOMAIN-SUFFIX,onedrive.com,DIRECT
  - DOMAIN-SUFFIX,1drv.com,DIRECT
  - DOMAIN-SUFFIX,muyuan.do,DIRECT
  - DOMAIN-SUFFIX,feishu.cn,DIRECT
  - DOMAIN-SUFFIX,turgame.com,DIRECT

  # 程序直连
  - PROCESS-NAME,steam.exe,DIRECT
  - PROCESS-NAME,eCloud.exe,DIRECT
  - PROCESS-NAME,BaiduNetdisk.exe,DIRECT
  - PROCESS-NAME,QQ.exe,DIRECT
  - PROCESS-NAME,Weixin.exe,DIRECT

  # 多线并发下载工具
  - PROCESS-NAME,fdm.exe,多线并发下载
  - PROCESS-NAME,PikPak.exe,多线并发下载
  - PROCESS-NAME,IDMan.exe,多线并发下载
  - PROCESS-NAME,Motrix.exe,多线并发下载
  - PROCESS-NAME,Thunder.exe,多线并发下载
"#;

/// 分流规则那行表头右侧动作组（导出 JSON + 导入规则）预留的宽度。
/// 「?」不在这里了 —— 示范只在导入时用得上，2026-10-15 他要求挪进导入弹框。
const RULES_ACTIONS_WIDTH: f32 = 180.0;

/// 两个分区标签（规则配置 / 命中记录）至少要占的宽度。
const RULES_TABS_MIN_WIDTH: f32 = 170.0;

/// `split_row` 两个闭包都要碰 `self`，所以动作先记成值、出框后再执行。
enum RulesHeaderAction {
    None,
    Export,
    Import,
}

/// 状态栏右侧那一排的普通按钮：填充、描边、字重全部写死一份，好让「IP 检测」「系统代理 /
/// TUN」「测速全部」和同排的 `primary_btn`/`danger_btn` 长成同一个形状 —— 那两条只换填充色
/// 表达"这是主操作 / 这是停止"。之前这四个按钮四种长相：普通按钮是常规字重 + 样式默认底，
/// chip 是手搓 `Frame`（6px 圆角、自己的内边距），主/危险按钮又各自钉过圆角。
fn status_btn(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border()))
}

impl VpnApp {
    pub(crate) fn ui_proxy(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);

        // ===================================================================
        // macOS 药丸分段选择条 (Segmented Control Bar)
        // ===================================================================
        let tr_charts = self.tr("汇总图表");
        let tr_nodes = self.tr("可用节点");
        let tr_groups = self.tr("分流策略组");
        let tr_conns = self.tr("活跃连接");
        let tr_rules = self.tr("分流规则");
        let tr_network = self.tr("DNS / TUN");
        let conn_label = if self.active_connections.is_empty() {
            tr_conns.to_string()
        } else {
            format!("{} ({})", tr_conns, self.active_connections.len())
        };

        let tabs = [
            (ProxyPanel::Charts, Icon::Dashboard, tr_charts),
            (ProxyPanel::Nodes, Icon::Pin, tr_nodes),
            (ProxyPanel::Groups, Icon::Rocket, tr_groups),
            (ProxyPanel::Connections, Icon::Radar, &conn_label),
            (ProxyPanel::Rules, Icon::Rules, tr_rules),
            (ProxyPanel::Network, Icon::Globe, tr_network),
            (ProxyPanel::PortMappings, Icon::Plug, "活动节点列表"),
            (ProxyPanel::Security, Icon::Shield, "安全隐私"),
            (ProxyPanel::Kernels, Icon::Cpu, "内核管理"),
            (ProxyPanel::Logs, Icon::Terminal, "运行日志"),
        ];

        macos_segmented_tabs(ui, &mut self.proxy_panel, &tabs);
        ui.add_space(8.0);

        match self.proxy_panel {
            ProxyPanel::Charts => self.ui_proxy_charts(ui),
            ProxyPanel::Nodes => self.ui_proxy_nodes(ui),
            ProxyPanel::Groups => self.ui_proxy_groups(ui),
            ProxyPanel::Connections => self.ui_proxy_connections(ui),
            ProxyPanel::Rules => self.ui_proxy_rules(ui),
            ProxyPanel::Network => self.ui_proxy_network(ui),
            ProxyPanel::PortMappings => self.ui_proxy_port_mappings(ui),
            ProxyPanel::Security => self.ui_proxy_security(ui),
            ProxyPanel::Kernels => self.ui_proxy_kernels(ui),
            ProxyPanel::Logs => self.ui_proxy_logs(ui),
        }
    }

    pub(crate) fn ui_proxy_charts(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("控制台"), "核心代理 · 流量监控 · 策略分流");

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
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 1. 核心主控岛卡片（默认撑开固定高度，无论是否选中节点均保持稳定结构）
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(18, 14))
                    .show(ui, |ui| {
                        ui.set_min_height(78.0);
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
                                        "TUN 全局虚拟网卡"
                                    } else {
                                        "系统代理"
                                    };
                                    (
                                        "VPN 代理运行中",
                                        format!("已接管流量 · 模式: {mode_tag} · 端口: Mixed {}", self.proxy_status.mixed_port),
                                    )
                                } else if is_starting {
                                    (
                                        "正在启动代理...",
                                        "内核加载中，正在验证网络出口与路由规则...".to_string(),
                                    )
                                } else if prepared {
                                    (
                                        "预备中 (正在下载内核)",
                                        "正在检测并下载最新代理核心执行文件...".to_string(),
                                    )
                                } else {
                                    (
                                        "代理服务已就绪",
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
                            .inner_margin(Margin::symmetric(14, 8))
                            .show(ui, |ui| {
                                ui.set_min_height(24.0);
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
                                        ui.label(
                                            RichText::new(clean_name)
                                                .strong()
                                                .size(12.5)
                                                .color(theme::text_primary()),
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
                                            ui.separator();
                                            let conns = self.active_connections.len();
                                            ui.label(
                                                RichText::new(format!("活跃连接: {conns}"))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );
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
                                        });
                                    } else if self.proxy_config.url_test {
                                        icon_label(ui, Icon::Lightning, 14.0, theme::accent());
                                        ui.label(
                                            RichText::new("⚡ 策略组自动选优已激活")
                                                .strong()
                                                .size(13.0)
                                                .color(theme::accent()),
                                        );
                                        ui.label(
                                            RichText::new("（内核自动测速并调度低延迟出口）")
                                                .small()
                                                .color(theme::muted()),
                                        );

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let conns = self.active_connections.len();
                                            ui.label(
                                                RichText::new(format!("活跃连接: {conns}"))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );
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
                                        ui.label(
                                            RichText::new("可在「可用节点」列表手动挑选节点或在下方切换自动调度")
                                                .small()
                                                .color(theme::muted()),
                                        );

                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            ui.label(RichText::new("— ms").small().color(theme::muted()));
                                            ui.add_space(2.0);
                                            render_signal_bars(ui, None);
                                            ui.separator();
                                            ui.label(RichText::new("活跃连接: 0").small().color(theme::muted()));
                                            ui.separator();
                                            ui.label(RichText::new("↑ 0 B/s").small().color(theme::muted()));
                                            ui.label(RichText::new("↓ 0 B/s").small().color(theme::muted()));
                                        });
                                    }
                                });
                            });
                    });

                ui.add_space(10.0);

                // 2. 双翼网络实时活动监视器看板（测速仪表盘 + 流量大波形与健康带）
                ui.horizontal(|ui| {
                    // 左侧翼：半圆测速仪表盘卡片
                    Frame::NONE
                        .fill(theme::sidebar())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
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
                                    self.traffic_down_rate,
                                    self.traffic_up_rate,
                                    is_running,
                                );
                            });
                        });

                    ui.add_space(8.0);

                    // 右侧翼：实时平滑波形图与出海健康带
                    Frame::NONE
                        .fill(theme::sidebar())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
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
                                            ui.label(
                                                RichText::new(format!(
                                                    "会话累计: ↑ {} ↓ {}",
                                                    format_bytes(self.traffic_up_total),
                                                    format_bytes(self.traffic_down_total)
                                                ))
                                                .small()
                                                .color(theme::muted()),
                                            );
                                        },
                                    );
                                });

                                ui.add_space(6.0);
                                render_traffic_waveform_card(
                                    ui,
                                    &self.traffic_history,
                                    is_running,
                                    92.0,
                                );

                                if self.auto_failover {
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        icon_label(ui, Icon::Shield, 11.0, theme::accent_strong());
                                        let detail = self
                                            .failover_last_reason
                                            .as_deref()
                                            .unwrap_or("已开启，最近实测正常");
                                        let detail: String = if detail.chars().count() > 30 {
                                            let mut s: String = detail.chars().take(30).collect();
                                            s.push('…');
                                            s
                                        } else {
                                            detail.to_string()
                                        };
                                        ui.label(
                                            RichText::new(format!(
                                                "自动切换: 已切 {} 次 · {detail}",
                                                self.failover_switches
                                            ))
                                            .small()
                                            .color(theme::muted()),
                                        );
                                        if ui
                                            .link(format!("记录（今日 {} 次）", self.switch_today))
                                            .clicked()
                                        {
                                            self.show_switch_log = true;
                                            self.reload_egress_switches();
                                        }
                                    });
                                }
                            });
                        });
                });

                ui.add_space(10.0);

                // 3. 全网节点延迟频谱带 + 节点协议组成分布（一行并排，上下边缘与高度严格对齐）
                if let Some(s) = self.stats.as_ref() {
                    let total_w = ui.available_width();
                    let gap = 10.0;
                    let left_w = ((total_w - gap) * 0.48).max(280.0);
                    let row_h = 114.0_f32; // 高度减少 50px（原 164.0）
                    let inner_h = row_h - 20.0; // 94.0_f32

                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                        // 左翼卡片：全网延迟阶梯频谱分布
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                ui.set_width(left_w);
                                ui.set_min_height(inner_h);
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        icon_label(ui, Icon::Lightning, 14.0, theme::accent());
                                        ui.label(
                                            RichText::new("全网延迟阶梯频谱分布")
                                                .strong()
                                                .size(13.0)
                                                .color(theme::text_primary()),
                                        );
                                    });
                                    ui.add_space(10.0);
                                    let spectrum = crate::gui::widgets::LatencySpectrumStats::from_latencies(
                                        self.nodes.iter().map(|n| n.latency_ms),
                                    );
                                    crate::gui::widgets::render_latency_spectrum(ui, &spectrum);
                                    ui.add_space(10.0);
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("⚡ <100ms: {}", spectrum.ultra_fast)).small().strong().color(Color32::from_rgb(82, 121, 111)));
                                        ui.separator();
                                        ui.label(RichText::new(format!("● 100-200ms: {}", spectrum.fast)).small().strong().color(Color32::from_rgb(122, 154, 139)));
                                        ui.separator();
                                        ui.label(RichText::new(format!("● 200-400ms: {}", spectrum.medium)).small().strong().color(Color32::from_rgb(198, 138, 76)));
                                        ui.separator();
                                        ui.label(RichText::new(format!("● >400ms: {}", spectrum.slow)).small().strong().color(Color32::from_rgb(158, 90, 84)));
                                    });
                                });
                                let used = ui.min_rect().height();
                                if used < inner_h {
                                    ui.allocate_space(egui::vec2(left_w, inner_h - used));
                                }
                            });

                        ui.add_space(gap);

                        // 右翼卡片：节点协议组成分布（控制在最多两行，保持与左卡完全等高）
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                let right_w = ui.available_width();
                                ui.set_width(right_w);
                                ui.set_min_height(inner_h);
                                ui.horizontal(|ui| {
                                    crate::gui::widgets::render_protocol_donut_chart_sized(ui, &s.protocol_stats, s.node_count, 80.0);
                                    ui.add_space(12.0);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            icon_label(ui, Icon::Radar, 14.0, theme::accent());
                                            ui.label(
                                                RichText::new("节点协议组成分布")
                                                    .strong()
                                                    .size(13.0)
                                                    .color(theme::text_primary()),
                                            );
                                            ui.separator();
                                            ui.label(
                                                RichText::new(format!("收录: {} 个", s.node_count))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );
                                            ui.separator();
                                            ui.label(
                                                RichText::new(format!("可用: {} 个", s.alive_count))
                                                    .small()
                                                    .strong()
                                                    .color(Color32::from_rgb(82, 121, 111)),
                                            );
                                        });
                                        ui.add_space(8.0);

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

                                        // 渲染第 1 行
                                        ui.horizontal(|ui| {
                                            for (proto, count, pct) in &row1 {
                                                Frame::NONE
                                                    .fill(theme::sidebar())
                                                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                                    .corner_radius(CornerRadius::same(6))
                                                    .inner_margin(Margin::symmetric(6, 2))
                                                    .show(ui, |ui| {
                                                        ui.label(
                                                            RichText::new(format!("{proto} {count} ({pct:.0}%)"))
                                                                .small()
                                                                .color(theme::text_primary()),
                                                        );
                                                    });
                                                ui.add_space(4.0);
                                            }
                                        });

                                        // 渲染第 2 行（若有）
                                        if !row2.is_empty() || other_pill.is_some() {
                                            ui.add_space(4.0);
                                            ui.horizontal(|ui| {
                                                for (proto, count, pct) in &row2 {
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new(format!("{proto} {count} ({pct:.0}%)"))
                                                                    .small()
                                                                    .color(theme::text_primary()),
                                                            );
                                                        });
                                                    ui.add_space(4.0);
                                                }
                                                if let Some((name, count, pct, tip)) = other_pill {
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
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
                                    });
                                });
                                let used = ui.min_rect().height();
                                if used < inner_h {
                                    ui.allocate_space(egui::vec2(right_w, inner_h - used));
                                }
                            });
                    });
                }

                ui.add_space(10.0);

                // 5. 分流策略组与流量调度决策中心（搬迁至图表底部）
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
                    .inner_margin(Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
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
                                        "AI分组" => format!("🤖 {name} ({count})"),
                                        "美国节点" => format!("🇺🇸 {name} ({count})"),
                                        "多线并发下载" => format!("⚡ {name} ({count})"),
                                        "普通代理" => format!("🔒 {name} ({count})"),
                                        _ => format!("{name} ({count})"),
                                    };

                                    let btn = egui::Button::new(
                                        RichText::new(&pill_label).small().strong().color(text_color),
                                    )
                                    .fill(fill)
                                    .stroke(stroke)
                                    .corner_radius(CornerRadius::same(12));

                                    let tooltip = match name.as_str() {
                                        "全部" => "展示全部可用节点，自由选择切换",
                                        "AI分组" => "专供 ChatGPT、Claude、Midjourney、Grok 等 AI，优先保证美国/原生 IP 解锁",
                                        "美国节点" => "url-test 周期测速，实时挑选最低延迟节点供给 Google 与 YouTube",
                                        "多线并发下载" => "load-balance 负载均衡，聚合节点并发加速 IDM / 迅雷 / 网盘下载",
                                        "普通代理" => "常规网页出海兜底；国内服务直连",
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

    pub(crate) fn ui_proxy_groups(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("分流策略组"), "场景调度 · 延迟选优 · 故障转移 · 负载均衡");

        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.groups_tab == GroupsTab::View, "视图")
                .clicked()
            {
                self.groups_tab = GroupsTab::View;
            }
            if ui
                .selectable_label(self.groups_tab == GroupsTab::Config, "配置")
                .clicked()
            {
                self.groups_tab = GroupsTab::Config;
            }
        });
        ui.add_space(6.0);

        match self.groups_tab {
            GroupsTab::View => {
                // 1. 顶部智能分流路由拓扑指示卡
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Shield, 15.0, theme::accent());
                            ui.label(
                                RichText::new("智能分流路由拓扑（场景自适应调度）")
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .checkbox(&mut self.proxy_config.smart_routing, "启用内置智能策略组")
                                    .on_hover_text("自动创建 AI分组、美国节点、多线并发下载、普通代理 策略组并注入优先级规则")
                                    .changed()
                                {
                                    self.save_proxy_config();
                                }
                            });
                        });

                        ui.add_space(8.0);

                        // 4 路由拓扑晶体微卡片
                        let routes = [
                            ("🤖 AI 分组", "专线原生 IP", "ChatGPT · Claude · Grok", theme::accent()),
                            ("🇺🇸 美国专线", "URL-Test 选优", "Google · YouTube · 选优", theme::accent_strong()),
                            ("⚡ 并发加速", "Load-Balance", "多路并发 · 迅雷 · 网盘下载", theme::accent_strong()),
                            ("🇨🇳 国内直连", "DIRECT 放行", "微信 · 淘宝 · B站 (0消耗)", theme::text_secondary()),
                        ];

                        ui.columns(4, |cols| {
                            for (i, (title, badge, desc, col)) in routes.iter().enumerate() {
                                Frame::NONE
                                    .fill(theme::sidebar())
                                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                    .corner_radius(CornerRadius::same(9))
                                    .inner_margin(Margin::symmetric(10, 8))
                                    .show(&mut cols[i], |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(*title).strong().size(12.5).color(theme::text_primary()));
                                        });
                                        ui.add_space(3.0);
                                        Frame::NONE
                                            .fill(col.gamma_multiply(0.15))
                                            .corner_radius(CornerRadius::same(4))
                                            .inner_margin(Margin::symmetric(5, 1))
                                            .show(ui, |ui| {
                                                ui.label(RichText::new(*badge).small().strong().color(*col));
                                            });
                                        ui.add_space(3.0);
                                        ui.label(RichText::new(*desc).small().color(theme::muted()));
                                    });
                            }
                        });
                    });

                ui.add_space(8.0);

                // 2. 策略组列表
                let is_running = matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                );

                if !is_running {
                    card_frame().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("代理核心未运行")
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new("启动代理核心后，此处将实时展示各策略组的活跃出口、测速延迟并支持直接切换节点。")
                                    .small()
                                    .color(theme::muted()),
                            );
                            ui.add_space(10.0);
                        });
                    });
                } else if self.kernel_groups.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("正在从代理内核同步策略组状态（每 5 秒自动轮询）...")
                                    .small()
                                    .color(theme::text_secondary()),
                            );
                        });
                    });
                } else {
                    let mut toggle_group: Option<(String, bool)> = None;
                    let mut switch_action: Option<(String, String)> = None;

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for group in &self.kernel_groups {
                                let is_expanded = self.expanded_kernel_groups.contains(&group.name);
                                Frame::NONE
                                    .fill(theme::panel())
                                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                    .corner_radius(CornerRadius::same(10))
                                    .inner_margin(Margin::symmetric(14, 10))
                                    .show(ui, |ui| {
                                        let mut toggle_clicked = false;
                                        ui.horizontal(|ui| {
                                            let group_icon = match group.name.as_str() {
                                                "AI分组" => Icon::Lightning,
                                                "美国节点" => Icon::Globe,
                                                "多线并发下载" => Icon::ArrowDown,
                                                _ => Icon::Radar,
                                            };
                                            icon_label(ui, group_icon, 14.0, theme::accent());
                                            ui.label(
                                                RichText::new(&group.name)
                                                    .strong()
                                                    .color(theme::text_primary()),
                                            );

                                            let type_pill = match group.kind.to_lowercase().as_str() {
                                                "url-test" | "urltest" => "自动优选 (url-test)",
                                                "load-balance" | "loadbalance" => "负载均衡 (load-balance)",
                                                _ => "手动选择 (select)",
                                            };
                                            ui.label(
                                                RichText::new(type_pill)
                                                    .small()
                                                    .color(theme::muted()),
                                            );

                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    // 展开/收起 胶囊按钮（默认折叠，点击展开）
                                                    let toggle_label = if is_expanded {
                                                        format!("▼ 收起接口 ({} 个)", group.members.len())
                                                    } else {
                                                        format!("▶ 展开接口 ({} 个)", group.members.len())
                                                    };
                                                    let toggle_btn = egui::Button::new(
                                                        RichText::new(toggle_label)
                                                            .small()
                                                            .strong()
                                                            .color(if is_expanded {
                                                                theme::accent_strong()
                                                            } else {
                                                                theme::text_secondary()
                                                            }),
                                                    )
                                                    .fill(if is_expanded {
                                                        theme::accent_bg()
                                                    } else {
                                                        theme::sidebar()
                                                    })
                                                    .stroke(Stroke::new(
                                                        1.0_f32,
                                                        if is_expanded {
                                                            theme::accent()
                                                        } else {
                                                            theme::border_subtle()
                                                        },
                                                    ))
                                                    .corner_radius(CornerRadius::same(8));

                                                    if ui.add(toggle_btn)
                                                        .on_hover_text(if is_expanded { "点击收起折叠该分组下的所有节点接口" } else { "点击展开查看该分组下的所有节点接口并可点选切换" })
                                                        .clicked()
                                                    {
                                                        toggle_clicked = true;
                                                    }

                                                    if let Some(now) = &group.now {
                                                        ui.separator();
                                                        ui.label(
                                                            RichText::new(format!("当前出口: {now}"))
                                                                .small()
                                                                .strong()
                                                                .color(theme::accent_strong()),
                                                        );
                                                    }
                                                },
                                            );
                                        });

                                        if toggle_clicked {
                                            toggle_group = Some((group.name.clone(), !is_expanded));
                                        }

                                        // 仅当展开时才展示该策略组下的所有接口/节点胶囊
                                        if is_expanded {
                                            ui.add_space(8.0);
                                            ui.horizontal_wrapped(|ui| {
                                                let is_selector = matches!(
                                                    group.kind.to_lowercase().as_str(),
                                                    "select" | "selector"
                                                );
                                                for member in &group.members {
                                                    let is_active = group.now.as_deref() == Some(member.as_str());
                                                    let text_color = if is_active {
                                                        theme::accent_strong()
                                                    } else {
                                                        theme::text_secondary()
                                                    };
                                                    let btn = ui.add(
                                                        egui::Button::new(
                                                            RichText::new(member)
                                                                .small()
                                                                .strong()
                                                                .color(text_color),
                                                        )
                                                        .fill(if is_active {
                                                            theme::accent_bg()
                                                        } else {
                                                            theme::page_bg()
                                                        })
                                                        .stroke(Stroke::new(
                                                            1.0_f32,
                                                            if is_active {
                                                                theme::accent()
                                                            } else {
                                                                theme::border_subtle()
                                                            },
                                                        ))
                                                        .corner_radius(CornerRadius::same(6)),
                                                    );
                                                    if is_selector && btn.clicked() && !is_active {
                                                        switch_action = Some((
                                                            group.name.clone(),
                                                            member.clone(),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    });
                                ui.add_space(6.0);
                            }
                        });

                    if let Some((name, expand)) = toggle_group {
                        if expand {
                            self.expanded_kernel_groups.insert(name);
                        } else {
                            self.expanded_kernel_groups.remove(&name);
                        }
                    }
                    if let Some((grp, mbr)) = switch_action {
                        self.select_kernel_member(grp, mbr);
                    }
                }
            }
            GroupsTab::Config => {
                // 自定义配置覆写 (YAML & Clash 脚本片段)：顶部固定标题与操作组，仅编辑区独立垂直滚动
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 12))
                    .show(ui, |ui| {
                        let mut save_clicked = false;
                        let mut load_recommended = false;
                        let mut clear_override = false;

                        // 固定顶部操作栏（固定不滚动）
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Terminal, 14.0, theme::text_secondary());
                            ui.label(
                                RichText::new("自定义配置覆写 (YAML & Clash 脚本片段)")
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add_enabled(!self.proxy_busy, primary_btn("保存配置"))
                                    .on_hover_text("立即保存 YAML 覆写配置并热重载生效")
                                    .clicked()
                                {
                                    save_clicked = true;
                                }
                                if ui
                                    .button("⚡ 一键载入推荐规则")
                                    .on_hover_text("填入经典的 AI、Google、多线下载与国内直连分流配置")
                                    .clicked()
                                {
                                    load_recommended = true;
                                }
                                if ui.button("清空覆写").clicked() {
                                    clear_override = true;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "支持直接在此处粘贴 Clash Verge Rev 风格的 YAML 配置覆写（rules / proxy-groups 等），保存后将自动合并生效："
                            )
                            .small()
                            .color(theme::muted()),
                        );
                        ui.add_space(6.0);
                        ui.separator();
                        ui.add_space(6.0);

                        // 仅让编辑文本框内部区域支持大文本纵向平滑滚动
                        let mut text = self.proxy_config.override_yaml.clone().unwrap_or_default();
                        let max_editor_height = (ui.available_height() - 8.0).max(220.0);

                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(max_editor_height)
                            .id_salt("proxy_override_yaml_editor_scroll")
                            .show(ui, |ui| {
                                let editor = ui.add(
                                    egui::TextEdit::multiline(&mut text)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(22)
                                        .hint_text("# 可在此处粘贴自定义 rules 或 proxy-groups YAML 片段\nrules:\n  - DOMAIN-SUFFIX,openai.com,AI分组\n  - PROCESS-NAME,IDMan.exe,多线并发下载"),
                                );
                                if editor.changed() {
                                    self.proxy_config.override_yaml = if text.trim().is_empty() {
                                        None
                                    } else {
                                        Some(text.clone())
                                    };
                                }
                            });

                        if load_recommended {
                            self.proxy_config.override_yaml =
                                Some(CLASH_VERGE_RECOMMENDED_OVERRIDE.to_string());
                            self.save_proxy_config();
                        }
                        if clear_override {
                            self.proxy_config.override_yaml = None;
                            self.save_proxy_config();
                        }
                        if save_clicked {
                            self.proxy_config.override_yaml = if text.trim().is_empty() {
                                None
                            } else {
                                Some(text)
                            };
                            self.save_proxy_config();
                            self.status = "已保存自定义分流覆写配置并热重载".into();
                        }
                    });
            }
        }
    }

    /// 底部状态栏右侧那一组控制按钮：先加的最靠右，所以启动/停止代理排第一；
    /// 模式 chip 和「测速全部」在最后加 —— 从右往左是
    /// `启动/停止代理 · IP 检测 · 系统代理 chip · 测速全部`（用户 2026-10-08 指定）。
    /// 「重启」按钮同一天被他撤了：热重载从此只有本地 API `/api/proxy/restart` 一条路。
    pub(crate) fn ui_proxy_control_buttons(&mut self, ui: &mut Ui) {
        let running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        let transitioning = matches!(
            self.proxy_status.state,
            ProxyState::Starting | ProxyState::Stopping
        );
        if running {
            if ui
                .add_enabled(!self.proxy_busy, danger_btn("停止代理"))
                .clicked()
            {
                self.stop_proxy();
            }
            if ui
                .add_enabled(!self.ip_risk_busy, status_btn("IP 检测"))
                .on_hover_text(
                    "查这个节点服务器 IP 的归属（机房/住宅）；当前在跑的节点还会经代理查一次真实出口",
                )
                .clicked()
            {
                self.check_ip_risk();
            }
        } else {
            let has_executable_node = self.selected_proxy_node_id.is_some()
                || self.focused_node_id.is_some()
                || !self.nodes.is_empty()
                || !self.proxy_nodes.is_empty();
            let can_start = !self.proxy_busy && !transitioning && has_executable_node;
            let start = ui.add_enabled(can_start, primary_btn("启动代理"));
            let start = if !has_executable_node {
                start.on_hover_text("节点列表为空，请先在订阅源拉取节点")
            } else if self.selected_proxy_node_id.is_none() && self.focused_node_id.is_none() && self.proxy_config.url_test {
                start.on_hover_text("点击以当前策略组自动选优启动")
            } else {
                start
            };
            if start.clicked() {
                self.start_proxy();
            }
            if ui
                .add_enabled(!self.ip_risk_busy, status_btn("IP 检测"))
                .on_hover_text("查该节点服务器 IP 是机房还是住宅出口")
                .clicked()
            {
                self.check_ip_risk();
            }
        }

        // 系统代理 / TUN：一个按钮来回切，运行中不给改（改了要重载内核）。
        // 以前它是手搓的 `Frame`（6px 圆角、自己的内边距、9px 小字），和同排那三个
        // 按钮四种长相 —— 现在走同一个 `status_btn`，只剩语义差别（用户 2026-10-15）。
        let mode_label = match self.proxy_config.mode {
            // Linux 没有"系统代理"那一格，这个模式下实际做的事就是把端口开着等人来指，
            // 所以叫「本地端口」；存储值仍是 System，不改数据层、不发明第三态。
            ProxyMode::System if crate::system_proxy::system_proxy_supported_on_this_platform() => {
                "系统代理"
            }
            ProxyMode::System => "本地端口",
            ProxyMode::Tun => "TUN 模式",
        };
        let mode_resp = ui
            .add(status_btn(mode_label))
            .on_hover_text("点击切换 系统代理 / TUN 模式");
        if mode_resp.clicked() {
            if self.proxy_status.config_editable() {
                self.proxy_config.mode = match self.proxy_config.mode {
                    ProxyMode::System => ProxyMode::Tun,
                    ProxyMode::Tun => ProxyMode::System,
                };
                self.save_proxy_config();
            } else {
                self.status = "请先停止代理后再切换工作模式".into();
            }
        }

        // 这里原来挂过一条「系统代理被外部关闭」的黄字，2026-10-15 他一句"不用显示这个"撤了：
        // 核对逻辑还在（喂设置页那个「自动重开系统代理」开关），但界面不再画它。

        // 测速全部：走的就是订阅源页/节点页那条 `speedtest_all_sources`，不另开一条通道。
        // 并发用系统设置里那个数（默认 128），批量作业自带 TCP 预筛。
        let testing = self.busy;
        if ui
            .add_enabled(!testing, status_btn("测速全部"))
            .on_hover_text(if testing {
                "已经有任务在跑，等它结束再测速"
            } else {
                "对所有订阅节点跑一轮批量测速：先 TCP 预筛，再逐个真实握手测延迟"
            })
            .clicked()
        {
            self.speedtest_all_sources();
        }
    }

    pub(crate) fn ui_proxy_nodes(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("可用节点"), "多节点状态 · 快速测速 · 节点切换");

        if self.selected_proxy_source_id.as_deref() != Some("all") {
            self.selected_proxy_source_id = Some("all".to_string());
            self.reload_proxy_nodes();
        }
        if self.proxy_selected_group.is_empty() {
            self.proxy_selected_group = "全部".to_string();
        }

        // 默认只挂「全部」；其余组只有配置里真的有它才出现
        let visible_names = visible_group_names(
            self.proxy_config.smart_routing,
            self.proxy_config.override_yaml.as_deref(),
            &self.kernel_groups,
        );
        if !visible_names.iter().any(|name| name == &self.proxy_selected_group) {
            self.proxy_selected_group = "全部".to_string();
        }

        let is_auto = self.proxy_config.url_test;
        let running = self.proxy_status.state == ProxyState::Running;
        let current_group_now = if self.proxy_selected_group == "全部" || self.proxy_selected_group == "PROXY" {
            if is_auto {
                self.kernel_egress_name()
            } else {
                self.selected_proxy_node_id.as_ref().and_then(|id| {
                    self.proxy_nodes.iter().find(|n| &n.id == id).map(|n| n.name.clone())
                })
            }
        } else {
            self.kernel_groups
                .iter()
                .find(|g| g.name == self.proxy_selected_group)
                .and_then(|g| g.now.clone())
        };

        // ===================================================================
        // macOS Sequoia 极简晶体工具栏 (单行紧凑布局：地区过滤 + 排序 + 搜索与节点计数)
        // ===================================================================
        Frame::NONE
            .fill(theme::panel())
            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 7))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // 若当前在按特定策略组筛选，展示可点击重置的轻量胶囊
                    if self.proxy_selected_group != "全部" {
                        let reset_resp = ui.add(
                            egui::Button::new(
                                RichText::new(format!("📁 {} ✕", self.proxy_selected_group))
                                    .small()
                                    .strong()
                                    .color(theme::accent_strong()),
                            )
                            .fill(theme::accent_bg())
                            .stroke(Stroke::new(1.0_f32, theme::accent()))
                            .corner_radius(CornerRadius::same(10)),
                        );
                        if reset_resp
                            .on_hover_text("当前正在按此策略组过滤节点，点击重置并显示「全部」节点")
                            .clicked()
                        {
                            self.proxy_selected_group = "全部".to_string();
                            self.proxy_page = 1;
                        }
                        ui.separator();
                    }

                    // 地区过滤胶囊
                    icon_label(ui, Icon::Globe, 12.0, theme::text_secondary());
                    let regions = [
                        ("all", "全部"),
                        ("HK", "🇭🇰 香港"),
                        ("JP", "🇯🇵 日本"),
                        ("US", "🇺🇸 美国"),
                        ("SG", "🇸🇬 新加坡"),
                        ("TW", "🇹🇼 台湾"),
                        ("KR", "🇰🇷 韩国"),
                        ("OTHER", "其它"),
                    ];
                    for (code, label) in regions {
                        let is_sel = self.proxy_region_filter == code;
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
                        let btn = egui::Button::new(RichText::new(label).small().strong().color(text_color))
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(CornerRadius::same(10));
                        if ui.add(btn).clicked() && !is_sel {
                            self.proxy_region_filter = code.to_string();
                            self.proxy_page = 1;
                        }
                    }

                    // 右侧动作组：节点排序 + 搜索框与节点计数
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        Frame::NONE
                            .fill(theme::sidebar())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(8, 3))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("{} 节点", self.proxy_nodes.len()))
                                        .small()
                                        .strong()
                                        .color(theme::text_secondary()),
                                );
                            });

                        let cleared = ui
                            .add_visible(
                                !self.proxy_search.is_empty(),
                                icons::icon_btn_square(Icon::Close, 11.0),
                            )
                            .clicked();
                        if cleared {
                            self.proxy_search.clear();
                            self.proxy_page = 1;
                        }

                        ui.push_id("proxy_node_search_island", |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Search, 12.0, theme::muted());
                                let search_field = ui.add(
                                    egui::TextEdit::singleline(&mut self.proxy_search)
                                        .hint_text("搜索节点...")
                                        .desired_width(100.0),
                                );
                                if search_field.changed() {
                                    self.proxy_page = 1;
                                }
                                crate::gui::widgets::note_text_field(ui, &search_field);
                            });
                        });

                        ui.separator();

                        let is_name = self.proxy_sort_by == ProxySortBy::Name;
                        if ui
                            .add(icons::selectable_icon_btn(Icon::Sort, "名称", is_name))
                            .clicked()
                        {
                            self.proxy_sort_by = ProxySortBy::Name;
                            self.proxy_page = 1;
                        }
                        let is_lat = self.proxy_sort_by == ProxySortBy::Latency;
                        if ui
                            .add(icons::selectable_icon_btn(Icon::Lightning, "延迟", is_lat))
                            .clicked()
                        {
                            self.proxy_sort_by = ProxySortBy::Latency;
                            self.proxy_page = 1;
                        }
                        ui.label(RichText::new("排序:").small().color(theme::text_secondary()));
                    });
                });
            });

        ui.add_space(6.0);

        // -------------------------------------------------------------------
        // 第 4 栏：节点网格渲染（按策略组、地区、搜索词过滤与虚拟滚动）
        // -------------------------------------------------------------------
        let group_key = self.proxy_selected_group.clone();
        let region_key = self.proxy_region_filter.clone();
        let search_key = self.proxy_search.trim().to_lowercase();
        let sort_key = self.proxy_sort_by;
        let rev = self.proxy_nodes_rev;
        let view = match self.proxy_view.take() {
            Some(cached)
                if cached.rev == rev
                    && cached.group == group_key
                    && cached.region == region_key
                    && cached.search == search_key
                    && cached.sort == sort_key =>
            {
                cached
            }
            _ => build_proxy_view(
                &self.proxy_nodes,
                &group_key,
                &self.kernel_groups,
                &region_key,
                &search_key,
                sort_key,
                rev,
            ),
        };

        let mut selected = None;
        let mut group_switch_member = None;
        let mut locked: Option<(String, String)> = None;
        let mut switch_to_manual = false;
        let mut menu_action: Option<NodeCardAction> = None;
        let status_msg: Option<String> = None;
        let selected_id = self.selected_proxy_node_id.clone();
        let switching_id = self.switching_node_id.clone();
        let is_busy = self.busy;

        let total_items = view.rows.len();
        let page_size = (self.proxy_page_size as usize).clamp(10, 200);
        let total_pages = NodesViewModel::total_pages(total_items as i64, page_size as u32);
        if self.proxy_page > total_pages {
            self.proxy_page = total_pages;
        }
        if self.proxy_page < 1 {
            self.proxy_page = 1;
        }

        let start_idx = ((self.proxy_page - 1) as usize * page_size).min(total_items);
        let end_idx = (start_idx + page_size).min(total_items);
        let paged_rows = if start_idx < total_items {
            &view.rows[start_idx..end_idx]
        } else {
            &[]
        };

        let (columns, total_card_rows) =
            NodesViewModel::grid_layout(ui.available_width(), paged_rows.len());

        if view.rows.is_empty() {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(20.0);
                    card_frame().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(14.0);
                            icon_label(ui, Icon::Radar, 28.0, theme::muted());
                            ui.add_space(8.0);
                            let title = if self.proxy_nodes.is_empty() {
                                "暂无可用的代理节点"
                            } else {
                                "当前策略组与筛选条件下无匹配节点"
                            };
                            ui.label(RichText::new(title).strong().color(theme::text_primary()));
                            ui.add_space(4.0);
                            let hint = if self.proxy_nodes.is_empty() {
                                "可在「节点列表」中进行节点测速以获取可用节点，或尝试重新拉取订阅。"
                            } else {
                                "可尝试清除地区过滤、清空搜索词，或切换至「全部」策略组。"
                            };
                            ui.label(RichText::new(hint).small().color(theme::muted()));
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                if !self.proxy_search.is_empty() || self.proxy_region_filter != "all" {
                                    if ui.button("清除筛选与搜索").clicked() {
                                        self.proxy_search.clear();
                                        self.proxy_region_filter = "all".into();
                                        self.proxy_page = 1;
                                    }
                                }
                                if self.proxy_selected_group != "全部" {
                                    if ui.button("查看全部节点").clicked() {
                                        self.proxy_selected_group = "全部".into();
                                        self.proxy_page = 1;
                                    }
                                }
                            });
                            ui.add_space(10.0);
                        });
                    });
                });
        } else {
            // 分页控制栏 (第 X / Y 页 · 共 Z 节点 · 上一页 / 下一页 · 容量选择器)
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "第 {} / {} 页 · 共 {} 节点",
                        self.proxy_page, total_pages, total_items
                    ))
                    .small()
                    .color(theme::text_secondary()),
                );

                if ui
                    .add_enabled(self.proxy_page > 1, egui::Button::new("上一页").small())
                    .clicked()
                {
                    self.proxy_page = self.proxy_page.saturating_sub(1).max(1);
                }
                if ui
                    .add_enabled(self.proxy_page < total_pages, egui::Button::new("下一页").small())
                    .clicked()
                {
                    self.proxy_page = (self.proxy_page + 1).min(total_pages);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("proxy_page_size_selector")
                        .selected_text(format!("每页 {} 条", self.proxy_page_size))
                        .width(95.0)
                        .show_ui(ui, |ui| {
                            for size in [20, 25, 30, 50, 100] {
                                if ui.selectable_value(&mut self.proxy_page_size, size, format!("{size} 条/页")).clicked() {
                                    self.proxy_page = 1;
                                }
                            }
                        });
                });
            });
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show_rows(ui, 64.0, total_card_rows, |ui, row_range| {
                    for row_idx in row_range {
                        let start = row_idx * columns;
                        let end = (start + columns).min(paged_rows.len());
                        let row = &paged_rows[start..end];
                        ui.columns(columns, |column_uis| {
                            for (column_index, column_ui) in column_uis.iter_mut().enumerate() {
                                let Some(entry) = row.get(column_index) else {
                                    continue;
                                };
                                let node = &self.proxy_nodes[entry.index];

                                // 活跃状态判定：优先匹配当前策略组活跃出口，或者全局选中节点
                                let is_active = if let Some(ref now) = current_group_now {
                                    now == &node.name
                                } else if is_auto {
                                    self.kernel_egress_name().as_deref() == Some(node.name.as_str())
                                } else {
                                    selected_id.as_deref() == Some(&node.id)
                                };
                                let is_focused = self.focused_node_id.as_deref() == Some(&node.id);
                                let is_switching = switching_id.as_deref() == Some(&node.id);

                                let card_tip = if is_active {
                                    format!("{}:{}\n当前活动出口（右键管理）", node.server, node.port)
                                } else if self.proxy_selected_group != "全部" && self.proxy_selected_group != "PROXY" {
                                    format!(
                                        "{}:{}\n左键点击选中，右键设为「{}」活动出口（双击亦可）",
                                        node.server, node.port, self.proxy_selected_group
                                    )
                                } else {
                                    format!(
                                        "{}:{}\n左键点击选中，右键设为活动节点（双击亦可）",
                                        node.server, node.port
                                    )
                                };

                                let response = compact_node_card(
                                    column_ui,
                                    node,
                                    is_active,
                                    is_focused,
                                    is_switching,
                                )
                                .on_hover_text(card_tip);

                                let clicked = response.clicked();
                                let double_clicked = response.double_clicked();

                                let node_id = node.id.clone();
                                let node_name = node.name.clone();
                                response.context_menu(|ui| {
                                    if !is_active {
                                        let label = if self.proxy_selected_group != "全部" && self.proxy_selected_group != "PROXY" {
                                            format!("🚀 设为「{}」活动出口", self.proxy_selected_group)
                                        } else {
                                            "🚀 设为活动节点".to_string()
                                        };
                                        if ui.button(label).clicked() {
                                            if is_auto {
                                                switch_to_manual = true;
                                            }
                                            selected = Some((node_id.clone(), node_name.clone()));
                                            if running && self.proxy_selected_group != "全部" && self.proxy_selected_group != "PROXY" {
                                                group_switch_member = Some((self.proxy_selected_group.clone(), node_name.clone()));
                                            }
                                            ui.close_menu();
                                        }
                                    } else {
                                        ui.label(RichText::new("✓ 当前活动节点").small().color(theme::accent_strong()));
                                    }
                                    ui.separator();
                                    if ui.button("ℹ️ 查看详情").clicked() {
                                        menu_action = Some(NodeCardAction::ShowDetail(node_id.clone()));
                                        ui.close_menu();
                                    }
                                    if ui
                                        .add_enabled(!is_busy, egui::Button::new("⚡ 测速节点"))
                                        .clicked()
                                    {
                                        menu_action = Some(NodeCardAction::Speedtest(node_id.clone()));
                                        ui.close_menu();
                                    }
                                    if ui.button("🛡️ IP 检测").clicked() {
                                        menu_action = Some(NodeCardAction::CheckIpRisk(node_id.clone()));
                                        ui.close_menu();
                                    }
                                    if (is_auto || self.proxy_selected_group != "全部") && !is_active {
                                        ui.separator();
                                        if ui.button("锁定为出口（关闭自动选路）").clicked() {
                                            locked = Some((node_id.clone(), node_name.clone()));
                                            ui.close_menu();
                                        }
                                    }
                                });

                                if double_clicked && !is_active {
                                    if is_auto {
                                        switch_to_manual = true;
                                    }
                                    selected = Some((node_id.clone(), node_name.clone()));
                                    if running && self.proxy_selected_group != "全部" && self.proxy_selected_group != "PROXY" {
                                        group_switch_member = Some((self.proxy_selected_group.clone(), node_name.clone()));
                                    }
                                } else if clicked {
                                    menu_action = Some(NodeCardAction::Select {
                                        id: node_id.clone(),
                                        name: node_name.clone(),
                                    });
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                });

            if total_pages > 1 {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("第 {} / {} 页", self.proxy_page, total_pages))
                            .small()
                            .color(theme::text_secondary()),
                    );
                    if ui
                        .add_enabled(self.proxy_page > 1, egui::Button::new("上一页").small())
                        .clicked()
                    {
                        self.proxy_page = self.proxy_page.saturating_sub(1).max(1);
                    }
                    if ui
                        .add_enabled(self.proxy_page < total_pages, egui::Button::new("下一页").small())
                        .clicked()
                    {
                        self.proxy_page = (self.proxy_page + 1).min(total_pages);
                    }
                });
            }
        }

        if let Some(msg) = status_msg {
            self.status = msg;
        }
        if let Some(action) = menu_action {
            self.handle_node_card_action(action);
        }
        if let Some((grp, member)) = group_switch_member {
            self.select_kernel_member(grp, member);
        }
        if switch_to_manual {
            self.proxy_config.url_test = false;
            if running {
                self.save_proxy_config();
            }
        }
        if let Some((id, name)) = selected {
            self.switch_proxy_node(id, name, "手动切换");
        }
        self.proxy_view = Some(view);
        if let Some((id, name)) = locked {
            // 两个开关不该同时表达"我要怎么选路"：显式锁定就把自动选路关掉，
            // 想回到全自动再自己勾回来（或点「交给自动选择」）。
            self.proxy_config.url_test = false;
            self.save_proxy_config();
            self.status = format!("已关闭自动选路，出口锁定为 {name}");
            self.switch_proxy_node(id, name, "手动切换");
        }
    }

    /// url-test 开着时真正的出口：`PROXY.now` 若指向「自动选择」，再取该组选中的成员。
    fn kernel_egress_name(&self) -> Option<String> {
        let proxy = self
            .kernel_groups
            .iter()
            .find(|group| group.name == "PROXY")?;
        let now = proxy.now.clone()?;
        if now != URL_TEST_GROUP {
            return Some(now);
        }
        self.kernel_groups
            .iter()
            .find(|group| group.name == URL_TEST_GROUP)
            .and_then(|group| group.now.clone())
    }

    pub(crate) fn ui_proxy_port_mappings(&mut self, ui: &mut Ui) {
        panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Plug, 16.0, theme::accent());
                ui.label(RichText::new("活动节点列表（一节点一端口独立转发）").strong().size(15.0));
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("为指定节点分配专用的本地监听端口（如 127.0.0.1:10810）。外部应用、爬虫或浏览器插件连接该端口时，流量将独占转发给绑定的节点，完全绕过全局代理路由。")
                    .small()
                    .color(theme::text_secondary()),
            );
            ui.add_space(10.0);

            // 顶部添加表单
            ui.horizontal(|ui| {
                ui.label(RichText::new("本地端口:").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_mapping_port)
                        .desired_width(85.0)
                        .hint_text("如 10810"),
                );

                ui.add_space(8.0);
                ui.label(RichText::new("转发目标节点:").strong());

                let (btn_text, btn_color) = if let Some(ref name) = self.new_mapping_node_name {
                    (format!("🎯 {name}"), theme::accent())
                } else {
                    ("➕ 点击选择目标节点...".to_string(), theme::text_secondary())
                };

                if ui
                    .button(RichText::new(btn_text).color(btn_color))
                    .on_hover_text("点击弹窗搜索并选择目标节点")
                    .clicked()
                {
                    self.node_picker_target_index = None;
                    self.node_picker_search.clear();
                    self.node_picker_open = true;
                }

                if self.new_mapping_node_id.is_some() {
                    if ui.button("✕").on_hover_text("清除选中的节点").clicked() {
                        self.new_mapping_node_id = None;
                        self.new_mapping_node_name = None;
                    }
                }

                ui.add_space(8.0);
                let add_btn = ui.add(
                    egui::Button::new(
                        RichText::new("➕ 添加映射")
                            .strong()
                            .size(12.5)
                            .color(theme::accent_strong()),
                    )
                    .fill(theme::accent_bg())
                    .stroke(Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.40)))
                    .corner_radius(CornerRadius::same(8)),
                );
                if add_btn.clicked() {
                    match self.new_mapping_port.trim().parse::<u16>() {
                        Ok(port) => {
                            if port < 1024 {
                                self.status = format!("端口 {port} 属于系统保留端口（须大于等于 1024）");
                            } else if port == self.proxy_config.mixed_port {
                                self.status = format!(
                                    "端口 {port} 与主代理端口 ({}) 冲突",
                                    self.proxy_config.mixed_port
                                );
                            } else if port == self.proxy_config.controller_port {
                                self.status = format!(
                                    "端口 {port} 与控制台端口 ({}) 冲突",
                                    self.proxy_config.controller_port
                                );
                            } else if self.proxy_config.port_mappings.iter().any(|m| m.port == port) {
                                self.status = format!("端口 {port} 已存在映射规则中");
                            } else if self.new_mapping_node_id.is_none() {
                                self.status = "请先点击选择目标节点".to_string();
                            } else {
                                let mapping = crate::proxy::PortMapping {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    port,
                                    node_id: self.new_mapping_node_id.take().unwrap_or_default(),
                                    node_name: self.new_mapping_node_name.take().unwrap_or_default(),
                                    enabled: true,
                                };
                                self.proxy_config.port_mappings.push(mapping);
                                self.new_mapping_port.clear();
                                self.save_proxy_config();
                                self.status = format!("已添加端口 {port} 映射规则并启用");
                            }
                        }
                        Err(_) => {
                            self.status = "请输入有效的端口号（1024 - 65535）".to_string();
                        }
                    }
                }
            });
        });

        ui.add_space(8.0);
        panel_frame().show(ui, |ui| {
            if self.proxy_config.port_mappings.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(30.0);
                    icon_label(ui, Icon::Plug, 28.0, theme::muted());
                    ui.add_space(8.0);
                    ui.label(RichText::new("暂无活动节点映射规则").color(theme::text_secondary()));
                    ui.label(
                        RichText::new("在上方输入本地端口并选择节点，即可为特定应用分配独占出口")
                            .small()
                            .color(theme::muted()),
                    );
                    ui.add_space(30.0);
                });
                return;
            }

            let mut to_delete = None;
            let mut changed = false;
            let mut pick_for_index = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // 表头
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new("状态").strong()));
                        ui.add_sized(
                            [160.0, 20.0],
                            egui::Label::new(RichText::new("本地监听地址").strong()),
                        );
                        ui.add_sized(
                            [ui.available_width() - 80.0, 20.0],
                            egui::Label::new(RichText::new("转发目标节点").strong()),
                        );
                        ui.add_sized([60.0, 20.0], egui::Label::new(RichText::new("操作").strong()));
                    });
                    ui.separator();

                    for (idx, mapping) in self.proxy_config.port_mappings.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            // 启用/停用按钮
                            let status_text = if mapping.enabled { "● 启用" } else { "○ 停用" };
                            let btn_fill = if mapping.enabled {
                                theme::accent_bg()
                            } else {
                                theme::panel_hover()
                            };
                            let btn_color = if mapping.enabled {
                                theme::accent_strong()
                            } else {
                                theme::muted()
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(status_text).size(12.0).color(btn_color),
                                    )
                                    .fill(btn_fill),
                                )
                                .on_hover_text(if mapping.enabled {
                                    "点击停用该端口映射"
                                } else {
                                    "点击启用该端口映射"
                                })
                                .clicked()
                            {
                                mapping.enabled = !mapping.enabled;
                                changed = true;
                            }

                            // 本地监听地址 + 复制
                            let addr_str = format!("127.0.0.1:{}", mapping.port);
                            ui.add_sized([160.0, 20.0], |ui: &mut Ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&addr_str).strong());
                                    if ui.button("📋").on_hover_text("复制本地代理地址").clicked() {
                                        ui.ctx().copy_text(addr_str.clone());
                                    }
                                })
                                .response
                            });

                            // 目标节点（点击可更换）
                            let target_width = (ui.available_width() - 70.0).max(100.0);
                            ui.add_sized([target_width, 20.0], |ui: &mut Ui| {
                                let display_name = if mapping.node_name.is_empty() {
                                    "（未命名节点）".to_string()
                                } else {
                                    mapping.node_name.clone()
                                };
                                let btn = ui.add(
                                    egui::Button::new(RichText::new(format!("🎯 {display_name}"))).truncate(),
                                )
                                .on_hover_text("点击更换该端口绑定的目标节点");
                                if btn.clicked() {
                                    pick_for_index = Some(idx);
                                }
                                btn
                            });

                            // 操作：删除
                            if ui
                                .button("🗑️")
                                .on_hover_text("删除该端口映射")
                                .clicked()
                            {
                                to_delete = Some(idx);
                            }
                        });
                        ui.separator();
                    }
                });

            if let Some(idx) = pick_for_index {
                self.node_picker_target_index = Some(idx);
                self.node_picker_search.clear();
                self.node_picker_open = true;
            }
            if let Some(idx) = to_delete {
                self.proxy_config.port_mappings.remove(idx);
                changed = true;
            }
            if changed {
                self.save_proxy_config();
            }
        });
    }

    pub(crate) fn ui_proxy_security(&mut self, ui: &mut Ui) {
        section_title(ui, "安全隐私", "uTLS 指纹伪装 · WebRTC 防泄漏 · 隐私追踪拦截 · 端口跳跃");

        let mut changed = false;

        // 顶部总览卡片
        panel_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Shield, 18.0, theme::accent());
                ui.label(RichText::new("抗追踪与身份安全防护中心").strong().size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("⚡ 一键开启最佳防护")
                                    .strong()
                                    .size(12.5)
                                    .color(theme::accent_strong()),
                            )
                            .fill(theme::accent_bg())
                            .stroke(Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.40)))
                            .corner_radius(CornerRadius::same(8)),
                        )
                        .on_hover_text("一键启用 Chrome 指纹伪装、WebRTC 防泄漏与隐私追踪广告过滤")
                        .clicked()
                    {
                        self.proxy_config.client_fingerprint = Some("chrome".to_string());
                        self.proxy_config.block_webrtc = true;
                        self.proxy_config.block_trackers = true;
                        changed = true;
                        self.status = "已一键开启最佳安全隐私防护".to_string();
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("集成协议指纹混淆、防 WebRTC 真实公网 IP 嗅探、第三方分析追踪器屏蔽与 UDP 端口跳跃优化，全方位保护网络身份。")
                    .small()
                    .color(theme::text_secondary()),
            );
        });

        ui.add_space(8.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // -------------------------------------------------------------
            // 卡片 1：全局 uTLS 浏览器指纹自动伪装 (Client Fingerprint)
            // -------------------------------------------------------------
            panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Globe, 16.0, theme::accent());
                    ui.label(
                        RichText::new("全局 uTLS 浏览器指纹自动伪装 (Client Fingerprint)")
                            .strong()
                            .size(14.0),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("模拟主流商业浏览器的 TLS ClientHello 密码套件与扩展顺序，消灭 Go / Rust 等代理客户端原生 TLS 特征指纹（JA3/JA4），彻底防御运营商 DPI 深度包检测与阻断。即使导入的订阅节点未携带 fp 参数，也会自动全局注入。")
                        .small()
                        .color(theme::text_secondary()),
                );
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("模拟浏览器预设:").strong());

                    let current_fp_label = match self.proxy_config.client_fingerprint.as_deref() {
                        Some("chrome") => "Chrome 浏览器 (推荐)",
                        Some("safari") => "Safari 浏览器 (Apple)",
                        Some("firefox") => "Firefox 浏览器",
                        Some("edge") => "Microsoft Edge",
                        Some("ios") => "iOS WebKit (iPhone/iPad)",
                        Some("randomized") => "Randomized (每次连接动态随机)",
                        Some(other) => other,
                        None => "⚪ 关闭指纹伪装",
                    };

                    egui::ComboBox::from_id_salt("security_utls_fingerprint")
                        .selected_text(current_fp_label)
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            let options = [
                                (Some("chrome".to_string()), "Chrome 浏览器 (推荐)"),
                                (Some("safari".to_string()), "Safari 浏览器 (Apple)"),
                                (Some("firefox".to_string()), "Firefox 浏览器"),
                                (Some("edge".to_string()), "Microsoft Edge"),
                                (Some("ios".to_string()), "iOS WebKit (iPhone/iPad)"),
                                (Some("randomized".to_string()), "Randomized (每次连接动态随机)"),
                                (None, "⚪ 关闭指纹伪装"),
                            ];
                            for (val, label) in options {
                                let is_selected = self.proxy_config.client_fingerprint == val;
                                if ui.selectable_label(is_selected, label).clicked() {
                                    self.proxy_config.client_fingerprint = val;
                                    changed = true;
                                }
                            }
                        });
                });
            });

            ui.add_space(8.0);

            // -------------------------------------------------------------
            // 卡片 2：WebRTC 真实 IP 泄漏防御 (Anti-WebRTC Leak)
            // -------------------------------------------------------------
            panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Shield, 16.0, theme::warning());
                    ui.label(
                        RichText::new("WebRTC 真实 IP 泄漏防御 (Anti-WebRTC Leak)")
                            .strong()
                            .size(14.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (btn_text, btn_fill, btn_color) = if self.proxy_config.block_webrtc {
                            ("● 已启用防御", theme::accent_bg(), theme::accent_strong())
                        } else {
                            ("○ 未启用", theme::panel_hover(), theme::muted())
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(btn_text).size(12.0).color(btn_color),
                                )
                                .fill(btn_fill),
                            )
                            .on_hover_text("点击切换 WebRTC 真实 IP 防泄漏开关")
                            .clicked()
                        {
                            self.proxy_config.block_webrtc = !self.proxy_config.block_webrtc;
                            changed = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("在内核层拦截 WebRTC 专用的 STUN / TURN 穿透端口（UDP/TCP 3478, 5349）以及常见 STUN 探测域名，彻底杜绝网站通过网页 JavaScript 探针探测并获取访客本地真实公网 IP。")
                        .small()
                        .color(theme::text_secondary()),
                );
            });

            ui.add_space(8.0);

            // -------------------------------------------------------------
            // 卡片 3：隐私与行为追踪广告过滤 (Tracker & Telemetry Blocking)
            // -------------------------------------------------------------
            panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Alert, 16.0, theme::accent());
                    ui.label(
                        RichText::new("隐私与行为追踪广告过滤 (Tracker & Telemetry Blocking)")
                            .strong()
                            .size(14.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (btn_text, btn_fill, btn_color) = if self.proxy_config.block_trackers {
                            ("● 已启用拦截", theme::accent_bg(), theme::accent_strong())
                        } else {
                            ("○ 未启用", theme::panel_hover(), theme::muted())
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(btn_text).size(12.0).color(btn_color),
                                )
                                .fill(btn_fill),
                            )
                            .on_hover_text("点击切换行为追踪与广告拦截开关")
                            .clicked()
                        {
                            self.proxy_config.block_trackers = !self.proxy_config.block_trackers;
                            changed = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("在代理内核中自动加载专业过滤规则库（category-ads-all、privacy、reject），拦截第三方用户行为数据采集、埋点统计 SDK（Google Analytics、友盟等）及网页弹窗横幅广告，大幅提升隐私安全并加速页面加载。")
                        .small()
                        .color(theme::text_secondary()),
                );
            });

            ui.add_space(8.0);

            // -------------------------------------------------------------
            // 卡片 4：UDP 端口跳跃与防 QoS 丢包混淆 (UDP Port Hopping & Anti-QoS)
            // -------------------------------------------------------------
            panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Lightning, 16.0, theme::accent());
                    ui.label(
                        RichText::new("UDP 端口跳跃与防 QoS 丢包混淆 (Port Hopping & Anti-QoS)")
                            .strong()
                            .size(14.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (btn_text, btn_fill, btn_color) = if self.proxy_config.udp_port_hopping {
                            ("● 已启用", theme::accent_bg(), theme::accent_strong())
                        } else {
                            ("○ 未启用", theme::panel_hover(), theme::muted())
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(btn_text).size(12.0).color(btn_color),
                                )
                                .fill(btn_fill),
                            )
                            .on_hover_text("点击切换 UDP 端口跳跃开关")
                            .clicked()
                        {
                            self.proxy_config.udp_port_hopping = !self.proxy_config.udp_port_hopping;
                            changed = true;
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("针对 Hysteria2 / TUIC 等高性能 UDP 协议，启用端口跳跃伪装（如多端口随机跳跃）与 FullCone NAT 穿透，打破运营商单端口大流量特征识别与激进 QoS 限速，改善晚高峰丢包与连接卡顿。")
                        .small()
                        .color(theme::text_secondary()),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("端口跳跃范围:").strong());
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.proxy_config.hop_ports)
                            .desired_width(140.0)
                            .hint_text("如 20000-40000"),
                    );
                    if edit.lost_focus() {
                        changed = true;
                    }
                    ui.label(
                        RichText::new("（服务端支持的端口范围，格式如 20000-40000）")
                            .small()
                            .color(theme::muted()),
                    );
                });
            });
        });

        if changed {
            self.save_proxy_config();
        }
    }

    pub(crate) fn ui_node_picker_modal(&mut self, ctx: &egui::Context) {
        if !self.node_picker_open {
            return;
        }
        let mut open = self.node_picker_open;
        let mut picked_node = None;

        egui::Window::new(RichText::new("选择转发目标节点").strong())
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .fixed_size([560.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Search, 14.0, theme::muted());
                    let search_field = ui.add(
                        egui::TextEdit::singleline(&mut self.node_picker_search)
                            .desired_width(450.0)
                            .hint_text("搜索节点名称、国家、协议、服务器..."),
                    );
                    crate::gui::widgets::note_text_field(ui, &search_field);
                    if !self.node_picker_search.is_empty() && ui.small_button("✕").clicked() {
                        self.node_picker_search.clear();
                    }
                });
                ui.separator();

                let search = self.node_picker_search.trim().to_lowercase();
                let all_nodes = self.proxy_nodes.iter().chain(self.nodes.iter());

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(430.0)
                    .show(ui, |ui| {
                        let mut count = 0;
                        let mut seen_ids = std::collections::HashSet::new();
                        for node in all_nodes {
                            if !seen_ids.insert(&node.id) {
                                continue;
                            }
                            if !search.is_empty() {
                                let matches = node.name.to_lowercase().contains(&search)
                                    || node.server.to_lowercase().contains(&search)
                                    || node.protocol.to_lowercase().contains(&search)
                                    || node
                                        .country
                                        .as_deref()
                                        .unwrap_or("")
                                        .to_lowercase()
                                        .contains(&search);
                                if !matches {
                                    continue;
                                }
                            }
                            count += 1;

                            ui.horizontal(|ui| {
                                render_country_code_badge(
                                    ui,
                                    node.country.as_deref().unwrap_or(""),
                                );
                                ui.add_space(2.0);
                                protocol_pill(ui, &node.protocol.to_uppercase());

                                let clean_name = sanitize_node_name(&node.name);
                                let name_label = ui.add_sized(
                                    [350.0, 18.0],
                                    egui::Label::new(RichText::new(&clean_name).strong()).truncate(),
                                );
                                name_label.on_hover_text(format!(
                                    "{}\n{}:{}",
                                    node.name, node.server, node.port
                                ));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("选择").clicked() {
                                        picked_node = Some((node.id.clone(), node.name.clone()));
                                    }
                                });
                            });
                            ui.separator();
                        }

                        if count == 0 {
                            ui.vertical_centered(|ui| {
                                ui.add_space(30.0);
                                ui.label(
                                    RichText::new("没有找到匹配的节点").color(theme::muted()),
                                );
                            });
                        }
                    });
            });

        if !open {
            self.node_picker_open = false;
        }
        if let Some((id, name)) = picked_node {
            if let Some(target_idx) = self.node_picker_target_index {
                if let Some(mapping) = self.proxy_config.port_mappings.get_mut(target_idx) {
                    mapping.node_id = id;
                    mapping.node_name = name;
                    self.save_proxy_config();
                }
            } else {
                self.new_mapping_node_id = Some(id);
                self.new_mapping_node_name = Some(name);
            }
            self.node_picker_open = false;
            self.node_picker_target_index = None;
        }
    }

    /// 「下载内核」弹框：镜像竞速 / 官方直连 / 走代理 / 本地导入 四种方式（防火墙放行开关在系统设置页）。
    ///
    /// 宽度必须写死：窗宽不足时提示会被压成一竖排字；四行也不用 `selectable_value`
    /// 而是自绘整行，这样底色/描边/提示换行才一致。
    pub(crate) fn ui_kernel_dialog(&mut self, ctx: &egui::Context) {
        use crate::kernel::{source_from_id, KernelSource, SOURCE_OPTIONS};
        if !self.show_kernel_dialog {
            return;
        }
        let Some(kind) = self.kernel_dialog_kind else {
            return;
        };
        let mut open = true;
        let mut start: Option<(KernelSource, Option<std::path::PathBuf>)> = None;

        const SOURCE_W: f32 = 424.0;
        const PAD: f32 = 11.0;
        const DOT_W: f32 = 22.0;

        egui::Window::new(format!("下载内核 · {}", kind.display_name()))
            .id(egui::Id::new("kernel-download-dialog"))
            .collapsible(false)
            .resizable(false)
            .default_width(SOURCE_W)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 把 zip 直接拖进窗口就能当本地导入的路径，省得他手打。
                let dropped: Vec<std::path::PathBuf> = ui.input(|input| {
                    input
                        .raw
                        .dropped_files
                        .iter()
                        .filter_map(|file| file.path.clone())
                        .collect()
                });
                for path in dropped {
                    let is_zip = path
                        .extension()
                        .map(|ext| ext.eq_ignore_ascii_case("zip"))
                        .unwrap_or(false);
                    if is_zip {
                        self.kernel_dialog_path = path.display().to_string();
                        self.kernel_dialog_source = "local".to_string();
                    }
                }

                ui.set_width(SOURCE_W);
                ui.label(
                    RichText::new("下载方式")
                        .strong()
                        .size(15.0)
                        .color(theme::text_primary()),
                );
                for option in SOURCE_OPTIONS {
                    let selected = self.kernel_dialog_source == option.id;
                    let mut job = egui::text::LayoutJob::default();
                    job.break_on_newline = true;
                    job.wrap.max_width = SOURCE_W - 2.0 * PAD - DOT_W;
                    job.append(
                        option.label,
                        0.0,
                        egui::text::TextFormat::simple(
                            egui::FontId::proportional(14.0),
                            if selected {
                                theme::accent_strong()
                            } else {
                                theme::text_primary()
                            },
                        ),
                    );
                    job.append(
                        &format!("\n{}", option.hint),
                        0.0,
                        egui::text::TextFormat::simple(
                            egui::FontId::proportional(11.5),
                            theme::text_secondary(),
                        ),
                    );
                    let galley = ui.painter().layout_job(job);
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(SOURCE_W, galley.size().y + 2.0 * PAD),
                        Sense::click(),
                    );
                    if response.clicked() {
                        self.kernel_dialog_source = option.id.to_string();
                    }
                    let fill = if selected {
                        theme::accent_bg()
                    } else if response.hovered() {
                        theme::panel_hover()
                    } else {
                        theme::panel()
                    };
                    // 未选中的行也是白底 + 冷调细边，不用灰底铺满整列。
                    let stroke = if selected {
                        Stroke::new(1.0_f32, theme::accent())
                    } else {
                        Stroke::new(1.0_f32, theme::border())
                    };
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(theme::RADIUS_BTN),
                        fill,
                        stroke,
                        egui::epaint::StrokeKind::Inside,
                    );
                    let dot = egui::pos2(rect.min.x + PAD + 5.0, rect.min.y + PAD + 8.0);
                    if selected {
                        ui.painter()
                            .circle_stroke(dot, 5.0, Stroke::new(1.6_f32, theme::accent()));
                        ui.painter().circle_filled(dot, 2.3_f32, theme::accent());
                    } else {
                        ui.painter()
                            .circle_stroke(dot, 5.0, Stroke::new(1.2_f32, theme::muted()));
                    }
                    ui.painter().galley(
                        egui::pos2(rect.min.x + PAD + DOT_W, rect.min.y + PAD - 1.0),
                        galley,
                        theme::text_primary(),
                    );
                    ui.add_space(4.0);
                }
                if self.kernel_dialog_source == "local" {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("本地 zip（也可以把文件拖进这个窗口）")
                            .small()
                            .color(theme::text_secondary()),
                    );
                    let path_field = ui.add(
                        egui::TextEdit::singleline(&mut self.kernel_dialog_path)
                            .desired_width(SOURCE_W - 4.0)
                            .hint_text(r"D:\downloads\mihomo-windows-amd64-v3-v1.19.32.zip"),
                    );
                    crate::gui::widgets::note_text_field(ui, &path_field);
                }

                ui.add_space(6.0);
                ui.separator();
                let source = source_from_id(&self.kernel_dialog_source);
                let ready =
                    source != KernelSource::Local || !self.kernel_dialog_path.trim().is_empty();
                ui.horizontal(|ui| {
                    if !ready {
                        ui.label(
                            RichText::new("先选好本地 zip 文件")
                                .small()
                                .color(theme::warning()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(ready && !self.kernel_busy, primary_btn("开始下载"))
                            .clicked()
                        {
                            let path = (source == KernelSource::Local)
                                .then(|| std::path::PathBuf::from(self.kernel_dialog_path.trim()));
                            start = Some((source, path));
                            open = false;
                        }
                        if ui.button("关闭").clicked() {
                            open = false;
                        }
                    });
                });
            });

        self.show_kernel_dialog = open;
        if let Some((source, path)) = start {
            self.update_kernel_from(kind, source, path, true);
        }
    }

    pub(crate) fn ui_proxy_connections(&mut self, ui: &mut Ui) {
        section_title(
            ui,
            "网络连接与 DNS 审计",
            "分流流向监控、活动连接跟踪、进程带宽矩阵及内核实时 DNS 解析记录",
        );

        let is_running = self.proxy_status.state == ProxyState::Running;
        let mut close_id = None;
        let mut close_all = false;
        let mut close_process_name: Option<String> = None;
        let mut copy_text_payload: Option<String> = None;

        // 聚合活跃进程数据
        let mut process_map: std::collections::HashMap<String, (u64, u64, usize, usize, usize)> = std::collections::HashMap::new();
        let mut direct_conns = 0usize;
        let mut proxy_conns = 0usize;
        let mut direct_bytes = 0u64;
        let mut proxy_bytes = 0u64;

        for c in &self.active_connections {
            let total_traffic = c.download + c.upload;
            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
            if is_direct {
                direct_conns += 1;
                direct_bytes += total_traffic;
            } else {
                proxy_conns += 1;
                proxy_bytes += total_traffic;
            }

            let proc_name = if c.process.is_empty() { "未知进程" } else { c.process.as_str() };
            let entry = process_map.entry(proc_name.to_string()).or_insert((0, 0, 0, 0, 0));
            entry.0 += c.upload;
            entry.1 += c.download;
            entry.2 += 1;
            if is_direct {
                entry.3 += 1;
            } else {
                entry.4 += 1;
            }
        }

        let mut process_list: Vec<(String, u64, u64, usize, usize, usize)> = process_map
            .into_iter()
            .map(|(k, v)| (k, v.0, v.1, v.2, v.3, v.4))
            .collect();
        process_list.sort_by_key(|p| std::cmp::Reverse(p.1 + p.2));

        let total_traffic = direct_bytes + proxy_bytes;
        let total_conns = (direct_conns + proxy_conns) as f32;

        // 顶部 macOS 风格次级 Tab 切换栏：连接列表 / 连接进程 / DNS 记录
        ui.horizontal(|ui| {
            let conn_str = format!("连接列表 ({})", self.active_connections.len());
            let proc_str = format!("连接进程 ({})", process_list.len());
            let dns_str = format!("DNS 记录 ({})", self.dns_records.len());
            let tabs = [
                (ConnectionsTab::List, Icon::Radar, conn_str.as_str()),
                (ConnectionsTab::Processes, Icon::Cpu, proc_str.as_str()),
                (ConnectionsTab::Dns, Icon::Globe, dns_str.as_str()),
            ];
            macos_segmented_tabs(ui, &mut self.connections_tab, &tabs);
        });

        ui.add_space(8.0);

        if !is_running {
            card_frame().show(ui, |ui| {
                ui.label(RichText::new("代理内核未运行，暂无活跃连接与实时 DNS 数据。启动代理后将在此自动监听展示。").color(theme::muted()));
            });
            return;
        }

        match self.connections_tab {
            ConnectionsTab::List => {
                // 1. 工具栏：筛选 + 统计 + 批量断开
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选");
                    let conn_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_filter)
                            .hint_text("按进程、域名或 IP 过滤")
                            .desired_width(180.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &conn_filter_field);
                    if !self.connections_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("当前活跃连接: {} 条", self.active_connections.len()));
                    ui.separator();
                    if ui
                        .add_enabled(!self.active_connections.is_empty(), danger_btn("断开全部连接"))
                        .on_hover_text("向内核发送指令关闭所有活动的网络连接")
                        .clicked()
                    {
                        close_all = true;
                    }
                });

                ui.add_space(8.0);

                // 2. 分流流向概览
                if total_conns > 0.0 {
                    Frame::NONE
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Radar, 13.0, theme::accent());
                                    ui.label(RichText::new("分流流向监控").small().strong().color(theme::text_primary()));
                                });
                                ui.add_space(4.0);

                                let segments = [
                                    ("代理分流 (Proxy)", proxy_conns as f32, theme::accent()),
                                    ("国内直连 (Direct)", direct_conns as f32, theme::border()),
                                ];
                                crate::gui::widgets::render_distribution_bar(ui, &segments, total_conns, 8.0);
                                ui.add_space(4.0);

                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "代理: {} 连 ({})",
                                            proxy_conns,
                                            format_bytes(proxy_bytes)
                                        ))
                                        .small()
                                        .color(theme::accent_strong()),
                                    );
                                    ui.separator();
                                    ui.label(
                                        RichText::new(format!(
                                            "直连: {} 连 ({})",
                                            direct_conns,
                                            format_bytes(direct_bytes)
                                        ))
                                        .small()
                                        .color(theme::text_secondary()),
                                    );
                                });
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3. 连接列表表格
                let filter = self.connections_filter.trim().to_lowercase();
                let filtered: Vec<&ActiveConnectionInfo> = self
                    .active_connections
                    .iter()
                    .filter(|c| {
                        if filter.is_empty() {
                            true
                        } else {
                            c.process.to_lowercase().contains(&filter)
                                || c.host.to_lowercase().contains(&filter)
                                || c.destination.to_lowercase().contains(&filter)
                                || c.rule.to_lowercase().contains(&filter)
                        }
                    })
                    .collect();

                if filtered.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.label(
                            RichText::new(if self.active_connections.is_empty() {
                                "当前暂无活动的网络连接"
                            } else {
                                "没有匹配筛选条件的网络连接"
                            })
                            .color(theme::muted()),
                        );
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Grid::new("active_connections_grid")
                                    .striped(true)
                                    .min_col_width(60.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "网络");
                                        header_cell(ui, "进程");
                                        header_cell(ui, "目标地址");
                                        header_cell(ui, "分流规则");
                                        header_cell(ui, "上传流量");
                                        header_cell(ui, "下载流量");
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for c in filtered {
                                            // 1. 网络协议微晶徽标
                                            let net_upper = c.network.to_uppercase();
                                            let (net_fill, net_color) = match net_upper.as_str() {
                                                "TCP" => (theme::accent_bg(), theme::accent_strong()),
                                                "UDP" => (theme::panel_hover(), theme::text_secondary()),
                                                _ => (theme::sidebar(), theme::muted()),
                                            };
                                            Frame::NONE
                                                .fill(net_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&net_upper).small().strong().color(net_color));
                                                });

                                            // 2. 进程胶囊
                                            Frame::NONE
                                                .fill(theme::sidebar())
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        icon_label(ui, Icon::Terminal, 10.0, theme::muted());
                                                        ui.label(RichText::new(&c.process).strong().small().color(theme::text_primary()));
                                                    });
                                                });

                                            // 3. 目标地址
                                            let target_text = if !c.host.is_empty() {
                                                format!("{} ({})", c.host, c.destination)
                                            } else {
                                                c.destination.clone()
                                            };
                                            ui.label(RichText::new(target_text).small().color(theme::text_primary()))
                                                .on_hover_text(&c.destination);

                                            // 4. 分流规则微标
                                            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(rule_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&c.rule).small().strong().color(rule_color));
                                                });

                                            // 5. 上传
                                            ui.label(
                                                RichText::new(format!("↑ {}", format_bytes(c.upload)))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );

                                            // 6. 下载
                                            ui.label(
                                                RichText::new(format!("↓ {}", format_bytes(c.download)))
                                                    .small()
                                                    .strong()
                                                    .color(theme::accent_strong()),
                                            );

                                            // 7. 操作
                                            if ui
                                                .add(danger_btn("断开"))
                                                .on_hover_text("断开此连接")
                                                .clicked()
                                            {
                                                close_id = Some(c.id.clone());
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                }
            }
            ConnectionsTab::Processes => {
                // 1. 工具栏
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选进程");
                    let proc_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_process_filter)
                            .hint_text("按进程名过滤")
                            .desired_width(180.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &proc_filter_field);
                    if !self.connections_process_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_process_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("当前活跃进程: {} 个", process_list.len()));
                    ui.separator();
                    ui.label(RichText::new(format!("进程产生总流量: {}", format_bytes(total_traffic))).small().color(theme::muted()));
                });

                ui.add_space(8.0);

                // 2. Top 进程矩阵卡片
                let top_matrix: Vec<(&str, u64, usize)> = process_list
                    .iter()
                    .map(|p| (p.0.as_str(), p.1 + p.2, p.3))
                    .collect();
                if !top_matrix.is_empty() {
                    Frame::NONE
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Cpu, 13.0, theme::accent());
                                    ui.label(RichText::new("Top 进程流量与带宽占比").small().strong().color(theme::text_primary()));
                                });
                                ui.add_space(4.0);
                                crate::gui::widgets::render_top_processes_card(ui, &top_matrix, total_traffic);
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3. 进程详细聚合表
                let p_filter = self.connections_process_filter.trim().to_lowercase();
                let filtered_procs: Vec<&(String, u64, u64, usize, usize, usize)> = process_list
                    .iter()
                    .filter(|p| p_filter.is_empty() || p.0.to_lowercase().contains(&p_filter))
                    .collect();

                if filtered_procs.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.label(RichText::new("当前暂无进程产生网络连接").color(theme::muted()));
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Grid::new("process_connections_grid")
                                    .striped(true)
                                    .min_col_width(70.0)
                                    .spacing([14.0, 8.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "进程");
                                        header_cell(ui, "活跃连接数");
                                        header_cell(ui, "分流分布");
                                        header_cell(ui, "上行流量");
                                        header_cell(ui, "下行流量");
                                        header_cell(ui, "总流量");
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for p in filtered_procs {
                                            let proc_name = &p.0;
                                            let upload = p.1;
                                            let download = p.2;
                                            let conn_count = p.3;
                                            let dir_count = p.4;
                                            let prx_count = p.5;
                                            let proc_traffic = upload + download;

                                            // 1. 进程名称
                                            Frame::NONE
                                                .fill(theme::sidebar())
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(8, 4))
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        icon_label(ui, Icon::Terminal, 11.0, theme::accent());
                                                        ui.label(RichText::new(proc_name).strong().color(theme::text_primary()));
                                                    });
                                                });

                                            // 2. 活跃连接数
                                            ui.label(RichText::new(format!("{conn_count} 条")).strong().color(theme::text_primary()));

                                            // 3. 分流分布
                                            ui.horizontal(|ui| {
                                                if prx_count > 0 {
                                                    ui.label(RichText::new(format!("代理:{prx_count}")).small().color(theme::accent_strong()));
                                                }
                                                if dir_count > 0 {
                                                    ui.label(RichText::new(format!("直连:{dir_count}")).small().color(theme::text_secondary()));
                                                }
                                            });

                                            // 4. 上行
                                            ui.label(RichText::new(format!("↑ {}", format_bytes(upload))).small().color(theme::text_secondary()));

                                            // 5. 下行
                                            ui.label(RichText::new(format!("↓ {}", format_bytes(download))).small().strong().color(theme::accent_strong()));

                                            // 6. 总流量
                                            ui.label(RichText::new(format_bytes(proc_traffic)).strong().color(theme::text_primary()));

                                            // 7. 操作：关闭该进程的所有连接
                                            if ui
                                                .add(danger_btn("断开全部"))
                                                .on_hover_text(format!("关闭进程 {proc_name} 的全部活动连接"))
                                                .clicked()
                                            {
                                                close_process_name = Some(proc_name.clone());
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                }
            }
            ConnectionsTab::Dns => {
                // 1. 工具栏：筛选 + 统计 + 清空
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选记录");
                    let dns_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_dns_filter)
                            .hint_text("按域名、解析 IP 或发起进程过滤")
                            .desired_width(200.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &dns_filter_field);
                    if !self.connections_dns_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_dns_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("已记录 DNS 轨迹: {} 条", self.dns_records.len()));
                    ui.separator();
                    if ui.button("清空全部记录").clicked() {
                        self.dns_records.clear();
                    }
                });

                ui.add_space(8.0);

                // 2. DNS 运行配置摘要
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Globe, 14.0, theme::accent());
                            ui.label(RichText::new("DNS 架构状态:").strong().color(theme::text_primary()));
                            let (mode_str, mode_color) = match self.proxy_config.dns.enhanced_mode {
                                DnsEnhancedMode::FakeIp => ("Fake-IP (秒级并发映射)", theme::accent_strong()),
                                DnsEnhancedMode::RedirHost => ("Redir-Host (真实解析直通)", theme::accent()),
                            };
                            ui.label(RichText::new(mode_str).strong().color(mode_color));
                            ui.separator();
                            let ns = self.proxy_config.dns.nameservers.first().map(|s| s.as_str()).unwrap_or("doh.pub");
                            ui.label(RichText::new(format!("主上游: {ns}")).small().color(theme::muted()));
                        });
                    });

                ui.add_space(8.0);

                // 3. DNS 记录表格
                let d_filter = self.connections_dns_filter.trim().to_lowercase();
                let filtered_dns: Vec<&DnsRecord> = self
                    .dns_records
                    .iter()
                    .filter(|r| {
                        d_filter.is_empty()
                            || r.host.to_lowercase().contains(&d_filter)
                            || r.ip.to_lowercase().contains(&d_filter)
                            || r.process.to_lowercase().contains(&d_filter)
                            || r.rule.to_lowercase().contains(&d_filter)
                    })
                    .collect();

                if filtered_dns.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.label(
                            RichText::new(if self.dns_records.is_empty() {
                                "暂无实时 DNS 记录（发起网页浏览或网络请求时将在此自动捕获）"
                            } else {
                                "没有匹配筛选条件的 DNS 记录"
                            })
                            .color(theme::muted()),
                        );
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                egui::Grid::new("dns_records_grid")
                                    .striped(true)
                                    .min_col_width(60.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "时间");
                                        header_cell(ui, "查询域名");
                                        header_cell(ui, "解析目标 IP");
                                        header_cell(ui, "发起进程");
                                        header_cell(ui, "网络");
                                        header_cell(ui, "分流动作");
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for r in filtered_dns {
                                            // 1. 时间
                                            ui.label(RichText::new(&r.time_str).small().monospace().color(theme::muted()));

                                            // 2. 域名
                                            ui.label(RichText::new(&r.host).strong().small().color(theme::text_primary()))
                                                .on_hover_text(&r.host);

                                            // 3. 解析目标
                                            ui.label(RichText::new(&r.ip).small().monospace().color(theme::text_secondary()))
                                                .on_hover_text(&r.ip);

                                            // 4. 发起进程
                                            Frame::NONE
                                                .fill(theme::sidebar())
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.horizontal(|ui| {
                                                        icon_label(ui, Icon::Terminal, 9.0, theme::muted());
                                                        ui.label(RichText::new(&r.process).small().color(theme::text_primary()));
                                                    });
                                                });

                                            // 5. 协议
                                            let (net_fill, net_color) = if r.network == "UDP" {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(net_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&r.network).small().strong().color(net_color));
                                                });

                                            // 6. 分流动作
                                            let is_direct = r.target.eq_ignore_ascii_case("direct") || r.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(rule_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&r.rule).small().strong().color(rule_color));
                                                });

                                            // 7. 复制
                                            ui.horizontal(|ui| {
                                                if ui.small_button("复制域名").clicked() {
                                                    copy_text_payload = Some(r.host.clone());
                                                }
                                                if ui.small_button("复制IP").clicked() {
                                                    copy_text_payload = Some(r.ip.clone());
                                                }
                                            });
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                }
            }
        }

        if let Some(id) = close_id {
            self.close_connection(id);
        } else if close_all {
            self.close_all_connections();
        } else if let Some(proc_name) = close_process_name {
            let ids_to_close: Vec<String> = self
                .active_connections
                .iter()
                .filter(|c| c.process == proc_name)
                .map(|c| c.id.clone())
                .collect();
            for id in ids_to_close {
                self.close_connection(id);
            }
        }

        if let Some(text) = copy_text_payload {
            ui.ctx().copy_text(text);
            self.status = "已复制到剪贴板".into();
        }
    }

    /// 分流规则面板：拆成「规则配置」和「命中记录」两个分区。
    /// 导入/导出挂在配置那一侧 —— 它们动的是规则表本身。
    pub(crate) fn ui_proxy_rules(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("分流规则"), "域名匹配 · IP 分流 · 端口过滤 · 规则集与命中统计");

        // 两个闭包都要动 self，所以 tab 状态先取副本、动作先记成枚举，出框后再落地。
        let mut tab = self.rules_tab;
        // 动作组按"这一帧进来时"的分区决定画不画；`tab` 被左闭包可变借用，这里另存一份。
        let tab_now = self.rules_tab;
        let busy = self.proxy_busy;
        let mut action = RulesHeaderAction::None;
        // 系统显示缩放 125%/150% 叠上字体 140% 时，960 物理最小宽只剩 550~460 逻辑点，
        // 中央面板不到 420 —— 一行放不下"两个分区标签 + 250 宽的动作组"，右对齐那组会
        // 直接压到标签上（2026-10-15 他报的"导出 JSON 被挤出去"）。放不下就让它换一行。
        let one_row_fits =
            ui.max_rect().width() >= RULES_ACTIONS_WIDTH + RULES_TABS_MIN_WIDTH + 8.0;
        if one_row_fits {
            crate::gui::widgets::split_row(
                ui,
                "rules:tabs",
                RULES_ACTIONS_WIDTH,
                |ui| {
                    panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                    panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
                },
                |ui| Self::rules_actions(ui, tab_now, busy, &mut action),
            );
        } else {
            let _ = ui.horizontal(|ui| {
                panel_tab(ui, &mut tab, RulesTab::Config, Icon::Rules, "规则配置");
                panel_tab(ui, &mut tab, RulesTab::Hits, Icon::Radar, "命中记录");
            });
            // 连一行都放不下时（200% 系统缩放 + 140% 字体）让它自己折，别越界。
            let _ = ui.horizontal_wrapped(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Self::rules_actions(ui, tab_now, busy, &mut action);
                });
            });
        }
        self.rules_tab = tab;
        match action {
            RulesHeaderAction::None => {}
            RulesHeaderAction::Export => self.export_rules_as_json(),
            RulesHeaderAction::Import => self.show_rule_import = true,
        }
        ui.add_space(8.0);
        match self.rules_tab {
            RulesTab::Config => self.ui_proxy_rules_config(ui),
            RulesTab::Hits => self.ui_proxy_rules_hits(ui),
        }
        self.ui_rule_import(ui.ctx());
        self.ui_rule_import_help(ui.ctx());
    }

    fn button_small(text: &str) -> egui::Button<'_> {
        egui::Button::new(RichText::new(text).strong().size(12.5))
    }

    /// 分流规则表头右侧那组动作（导出 JSON / 导入规则）。
    /// 两个闭包都要碰 `self`，所以这里只把点击记进 `action`，由调用方出框后再执行。
    fn rules_actions(ui: &mut Ui, tab: RulesTab, busy: bool, action: &mut RulesHeaderAction) {
        if tab != RulesTab::Config {
            return;
        }
        if ui
            .button("导出 JSON")
            .on_hover_text(
                "把当前规则表导成 mihomo 规则字符串数组：写进 data/exports 并复制到剪贴板",
            )
            .clicked()
        {
            *action = RulesHeaderAction::Export;
        }
        if ui
            .add_enabled(!busy, Self::button_small("导入规则"))
            .on_hover_text("贴 JSON 或一行一条的文本，解析后追加进规则表")
            .clicked()
        {
            *action = RulesHeaderAction::Import;
        }
    }

    /// 命中记录：规则列表说的是"应该走哪"，这张表说的是"实际走了哪"。
    /// 连接是快照不是事件流，所以统计从本进程启动开始，重启前的那部分没有。
    fn ui_proxy_rules_hits(&mut self, ui: &mut Ui) {
        let total: i64 = self.rule_hits_today.iter().map(|hit| hit.hits).sum();
        let matched: i64 = self
            .rule_hits_today
            .iter()
            .filter(|hit| hit.rule.starts_with("MATCH"))
            .map(|hit| hit.hits)
            .sum();
        let pct = if total > 0 { matched * 100 / total } else { 0 };
        let empty = self.rule_hits_today.is_empty();
        let mut refresh = false;
        card_frame().show(ui, |ui| {
            // 说明并进表头那一行（原来它单独占一行 + 8pt 间隔），滚动区再吃满剩余高度，
            // 列表视口从 469 涨到 534（1330×800）、240 涨到 305（140% 字体）。
            crate::gui::widgets::split_row(
                ui,
                "rules:hits_header",
                96.0,
                |ui| {
                    icon_label(ui, Icon::Radar, 16.0, theme::accent());
                    ui.label(RichText::new("规则命中记录").strong().size(15.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{total} 条连接 · 兜底 MATCH {matched} 条（{pct}%） · \
                                 占比按连接数算，一条连接只计一次"
                            ))
                            .small()
                            .color(theme::text_secondary()),
                        )
                        .truncate(),
                    )
                    .on_hover_text("统计从本进程启动开始；命中每 30 秒自动落库一次。");
                },
                |ui| {
                    if ui
                        .button("立即刷新")
                        .on_hover_text("命中每 30 秒自动落库一次，这里手动拉最新那份")
                        .clicked()
                    {
                        refresh = true;
                    }
                },
            );
            ui.add_space(6.0);
            if empty {
                ui.label(
                    RichText::new("还没有命中记录：启动代理并跑点流量之后再回来看。")
                        .color(theme::muted()),
                );
                return;
            }

            // 规则命中分段分布条
            let mut direct_hits = 0f32;
            let mut proxy_hits = 0f32;
            let mut other_hits = 0f32;
            for hit in &self.rule_hits_today {
                if hit.target.eq_ignore_ascii_case("direct") || hit.rule.contains("DIRECT") {
                    direct_hits += hit.hits as f32;
                } else if hit.target.eq_ignore_ascii_case("proxy") || hit.target == "全部" {
                    proxy_hits += hit.hits as f32;
                } else {
                    other_hits += hit.hits as f32;
                }
            }
            let segments = [
                ("国内直连 (DIRECT)", direct_hits, theme::border()),
                ("出海代理 (PROXY)", proxy_hits, theme::accent()),
                ("智能策略分流", other_hits, theme::accent_strong()),
            ];
            crate::gui::widgets::render_distribution_bar(ui, &segments, total as f32, 7.0);
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                header_cell(ui, "命中的规则");
                header_cell(ui, "派给");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    header_cell(ui, "条数 / 占比");
                });
            });
            egui::ScrollArea::vertical()
                .id_salt("rule_hits_table")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for hit in &self.rule_hits_today {
                        let share = if total > 0 { hit.hits * 100 / total } else { 0 };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&hit.rule).color(theme::accent()).monospace());
                            ui.label(RichText::new(format!("→ {}", hit.target)));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("{} 条 · {}%", hit.hits, share))
                                            .strong()
                                            .color(theme::text_primary()),
                                    );
                                },
                            );
                        });
                    }
                });
        });
        if refresh {
            self.reload_rule_hits();
        }
    }

    pub(crate) fn ui_proxy_rules_config(&mut self, ui: &mut Ui) {
        let editable = !self.proxy_busy;
        let show_custom_rules = self.proxy_config.routing_mode == RoutingMode::Rule;
        let mut save_mode_clicked = false;
        let mut save_clicked = false;
        let mut add_rule = false;
        let mut add_recommended = false;
        let mut optimize_rules = false;
        let mut move_rule: Option<(usize, usize)> = None;
        let mut remove_rule = None;

        // 整个面板一起滚：模式卡 + Geo 卡 + 预设卡本身就占掉大半屏，只给规则列表配
        // 滚动条的话，下面的卡片与"保存规则"按钮仍然会被挤出可视区。
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section_title(
                    ui,
                    "分流模式",
                    routing_mode_description(self.proxy_config.routing_mode),
                );
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Rule,
                                "自定义规则",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::ChinaDirect,
                                "中国大陆直连",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::GfwList,
                                "GFW 列表代理",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Global,
                                "全局代理",
                            );
                            ui.selectable_value(
                                &mut self.proxy_config.routing_mode,
                                RoutingMode::Direct,
                                "全部直连",
                            );
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(routing_mode_detail(self.proxy_config.routing_mode))
                                .small()
                                .color(theme::muted()),
                        );
                        ui.add_space(8.0);
                        save_mode_clicked = ui.add(primary_btn("保存分流模式")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理配置中...").color(theme::muted()));
                    }
                });
                ui.add_space(8.0);
                card_frame().show(ui, |ui| {
                    // 描述与按钮分两行：横向并排时长描述会把按钮顶出可视区。
                    ui.label(RichText::new("Geo 规则数据库维护").strong());
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "在线更新 GeoIP (中国大陆IP库) 与 GeoSite (域名分流库) 规则文件",
                            )
                            .small()
                            .color(theme::muted()),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let running = self.proxy_status.state == ProxyState::Running;
                        if ui
                            .add_enabled(
                                running && !self.geo_updating,
                                primary_btn("一键更新 Geo 规则"),
                            )
                            .on_hover_text(
                                "向内核发送指令在线拉取并重新载入最新的 GeoIP 与 GeoSite 数据",
                            )
                            .clicked()
                        {
                            self.update_geo_data();
                        }
                        if self.geo_updating {
                            ui.spinner();
                            ui.label(RichText::new("正在更新 Geo 规则...").small().color(theme::accent()));
                        }
                    });
                });

                if !show_custom_rules {
                    return;
                }

                ui.add_space(8.0);
                card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Lightbulb, 15.0, theme::accent());
                        ui.label(RichText::new("常用场景分流预设（一键添加至规则列表）").strong());
                    });
                    ui.label(
                        RichText::new(
                            "点击对应场景可一键合并预设分流规则，并在保存后自动热重载生效",
                        )
                        .small()
                        .color(theme::muted()),
                    );
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "AI 服务分流 (ChatGPT / Claude / Gemini)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 OpenAI、Claude、Gemini、Copilot 域名走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in ai_service_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "海外主流流媒体 (YouTube / Netflix / Spotify)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 YouTube、Netflix、Disney+、Spotify 走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in streaming_media_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "即时通讯 (Telegram / Discord / X)",
                                theme::accent(),
                            ))
                            .on_hover_text("添加 Telegram 域名与 IP 段、Discord、X 走代理")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in social_telegram_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "国内开发镜像直连 (npm / 清华源 / 阿里源)",
                                theme::accent_strong(),
                            ))
                            .on_hover_text("国内常见开源包与镜像站直连加速")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in dev_mirrors_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Plus,
                                "游戏平台直连 (Steam / Epic)",
                                theme::accent_strong(),
                            ))
                            .on_hover_text("Steam 下载 CDN 与游戏商店直连")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in game_platforms_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                        if ui
                            .add(icons::button_with_icon(
                                Icon::Shield,
                                "广告与 QUIC 拦截",
                                theme::accent(),
                            ))
                            .on_hover_text("拦截常见广告并阻止 QUIC 降速")
                            .clicked()
                        {
                            let mut rules = self.proxy_config.rules.clone();
                            for r in adblock_quic_rules() {
                                if !rules.contains(&r) {
                                    rules.insert(0, r);
                                }
                            }
                            self.proxy_config.rules = optimize_traffic_rules(&rules);
                            self.save_proxy_config();
                        }
                    });
                });

                ui.add_space(8.0);
                section_title(ui, "自定义规则", "从上到下匹配，未命中流量默认走代理");
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        for index in 0..self.proxy_config.rules.len() {
                            let matcher =
                                rule_matcher_label(&self.proxy_config.rules[index].matcher);
                            ui.horizontal(|ui| {
                                ui.add_sized([260.0, 22.0], egui::Label::new(matcher));
                                egui::ComboBox::from_id_salt(("rule_action", index))
                                    .selected_text(rule_action_label(
                                        self.proxy_config.rules[index].action,
                                    ))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Direct,
                                            "直连",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Proxy,
                                            "代理",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.rules[index].action,
                                            RuleAction::Reject,
                                            "拒绝",
                                        );
                                    });
                                if ui
                                    .add_enabled(index > 0, egui::Button::new("↑"))
                                    .on_hover_text("上移")
                                    .clicked()
                                {
                                    move_rule = Some((index, index - 1));
                                }
                                if ui
                                    .add_enabled(
                                        index + 1 < self.proxy_config.rules.len(),
                                        egui::Button::new("↓"),
                                    )
                                    .on_hover_text("下移")
                                    .clicked()
                                {
                                    move_rule = Some((index, index + 1));
                                }
                                if ui.button("×").on_hover_text("删除").clicked() {
                                    remove_rule = Some(index);
                                }
                            });
                        }

                        ui.horizontal_wrapped(|ui| {
                            egui::ComboBox::from_id_salt("new_rule_kind")
                                .selected_text(self.new_rule_kind.label())
                                .show_ui(ui, |ui| {
                                    for kind in RuleDraftKind::ALL {
                                        ui.selectable_value(
                                            &mut self.new_rule_kind,
                                            kind,
                                            kind.label(),
                                        );
                                    }
                                });
                            if self.new_rule_kind.needs_value() {
                                let rule_value_field = ui.add_sized(
                                    [220.0, 24.0],
                                    egui::TextEdit::singleline(&mut self.new_rule_value)
                                        .hint_text("匹配值"),
                                );
                                crate::gui::widgets::note_text_field(ui, &rule_value_field);
                            }
                            egui::ComboBox::from_id_salt("new_rule_action")
                                .selected_text(rule_action_label(self.new_rule_action))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Direct,
                                        "直连",
                                    );
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Proxy,
                                        "代理",
                                    );
                                    ui.selectable_value(
                                        &mut self.new_rule_action,
                                        RuleAction::Reject,
                                        "拒绝",
                                    );
                                });
                            add_rule = ui.button("添加规则").clicked();
                            ui.menu_button("⚡ 嗅探活动进程 ▼", |ui| {
                                ui.set_min_width(220.0);
                                ui.label(RichText::new("当前系统运行的应用进程").strong().color(theme::text_secondary()));
                                ui.separator();
                                let procs = crate::platform::list_running_processes();
                                if procs.is_empty() {
                                    ui.label(RichText::new("未检测到活跃进程").small().color(theme::muted()));
                                } else {
                                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                        for proc in procs {
                                            if ui.button(&proc).clicked() {
                                                self.new_rule_kind = RuleDraftKind::ProcessName;
                                                self.new_rule_value = proc.clone();
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                }
                            });

                            add_recommended = ui
                                .button("推荐分流")
                                .on_hover_text("覆盖为国内直连、广告拒绝、其余代理")
                                .clicked();
                            optimize_rules = ui
                                .button("自动优化")
                                .on_hover_text("移除重复规则并保持当前顺序")
                                .clicked();
                        });
                        if self.new_rule_kind == RuleDraftKind::ProcessName
                            && self.proxy_config.mode != ProxyMode::Tun
                        {
                            ui.label(
                                RichText::new(
                                    "PROCESS-NAME 只有 TUN 模式拿得到进程名，系统代理模式下不会生效",
                                )
                                .small()
                                .color(theme::warning()),
                            );
                        }
                        ui.label(
                            RichText::new("推荐规则：局域网与国内直连，广告和 QUIC 拒绝，其余代理")
                                .small()
                                .color(theme::muted()),
                        );

                        ui.add_space(8.0);
                        save_clicked = ui.add(primary_btn("保存规则")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("正在处理规则中...").color(theme::muted()));
                    }
                });

                let injected = mihomo_dns_poison_overrides()
                    .iter()
                    .filter_map(|rule| rule.to_mihomo_rule().ok())
                    .collect::<Vec<_>>();
                if !injected.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "内置注入（不可编辑，仅 Mihomo 生效，插在 GEOIP,CN 之前）：{}",
                            injected.join(" · ")
                        ))
                        .small()
                        .color(theme::muted()),
                    );
                }
            });

        if save_mode_clicked {
            self.save_proxy_config();
        }
        if let Some((from, to)) = move_rule {
            self.proxy_config.rules.swap(from, to);
        }
        if let Some(index) = remove_rule {
            self.proxy_config.rules.remove(index);
        }
        if add_rule {
            let rule = TrafficRule {
                matcher: self.new_rule_kind.matcher(&self.new_rule_value),
                action: self.new_rule_action,
            };
            match rule.to_mihomo_rule() {
                Ok(_) => {
                    self.proxy_config.rules.push(rule);
                    self.new_rule_value.clear();
                }
                Err(error) => self.status = format!("规则无效: {error}"),
            }
        }
        if add_recommended {
            self.show_recommended_rules_confirm = true;
        }
        if optimize_rules {
            let before = self.proxy_config.rules.len();
            self.proxy_config.rules = optimize_traffic_rules(&self.proxy_config.rules);
            let removed = before.saturating_sub(self.proxy_config.rules.len());
            self.status = if removed == 0 {
                "规则已经是最优顺序，没有发现重复项".into()
            } else {
                format!("已自动移除 {removed} 条重复规则，请保存规则")
            };
        }
        if save_clicked {
            self.save_proxy_config();
        }
        self.ui_recommended_rules_confirm(ui.ctx());
    }

    /// 导出当前规则表：写一份 JSON 到 `data/exports/`，同时复制到剪贴板。
    /// 没有原生"另存为"对话框（不为此引依赖），所以走"文件 + 剪贴板"两条路，
    /// 哪条成了/没成都写在状态栏里，不假装成功。
    pub(crate) fn export_rules_as_json(&mut self) {
        let rules = &self.proxy_config.rules;
        let count = rules.len();
        let json = match crate::proxy::rule_io::export_json(rules) {
            Ok(json) => json,
            Err(error) => {
                self.status = format!("导出失败：{error}");
                return;
            }
        };
        let directory = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("data")
            .join("exports");
        let name = format!(
            "rules-{}-{}.json",
            crate::app_update::current_version(),
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let written = std::fs::create_dir_all(&directory)
            .and_then(|_| std::fs::write(directory.join(&name), &json))
            .map(|_| directory.join(name))
            .map_err(|error| error.to_string());
        let copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(json))
            .map_err(|error| error.to_string());
        let mut parts = vec![format!("导出 {count} 条规则")];
        parts.push(match &written {
            Ok(path) => format!("已写入 {}", path.display()),
            Err(error) => format!("写文件失败：{error}"),
        });
        parts.push(match copied {
            Ok(_) => "并复制到剪贴板".to_string(),
            Err(error) => format!("但剪贴板复制失败：{error}"),
        });
        self.status = parts.join(" · ");
    }

    /// 「导入规则」弹框：贴文本 → 当场给出解析预览 → 才让追加。
    /// 预览这一步不是客气：导入直接改流量走向，不能点下去才知道进了什么。
    pub(crate) fn ui_rule_import(&mut self, ctx: &egui::Context) {
        if !self.show_rule_import {
            return;
        }
        let mut open = true;
        let mut apply = false;
        // `Window::open` 已经借走了 `open`，闭包里只能记一笔，出框之后再关。
        let mut cancel = false;
        egui::Window::new("导入分流规则")
            .id(egui::Id::new("vpn-parser:rule-import"))
            .default_width(680.0)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "支持三种形状：JSON 数组、带 rules/prependRules 字段的 JSON 对象、\
                         以及一行一条的文本（可带引号、尾逗号与 // 注释）。\
                         目标写自定义组名的，落到本程序的代理组。",
                    )
                    .small()
                    .color(theme::muted()),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui
                        .button("从剪贴板读取")
                        .on_hover_text("整段粘进来即可，不用先摘引号")
                        .clicked()
                    {
                        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                            Ok(text) => {
                                self.rule_import_text = text.clone();
                                self.rule_import_preview =
                                    Some(crate::proxy::rule_io::parse_rules_text(&text));
                            }
                            Err(error) => self.status = format!("读剪贴板失败：{error}"),
                        }
                    }
                    // 「?」贴着「从剪贴板读取」：示范段落就是给人抄进这个框的。
                    let help = ui.add(
                        egui::Button::new(
                            RichText::new("?")
                                .strong()
                                .size(12.5)
                                .color(theme::accent()),
                        )
                        .min_size(Vec2::new(22.0, 22.0))
                        .fill(Color32::TRANSPARENT),
                    );
                    if help
                        .on_hover_text("导入格式示范：四种输入形状 + 支持/会跳过的类型")
                        .clicked()
                    {
                        self.show_rule_import_help = true;
                    }
                    if !self.rule_import_text.is_empty() && ui.button("清空").clicked() {
                        self.rule_import_text.clear();
                        self.rule_import_preview = None;
                    }
                });
                let field = ui.add(
                    egui::TextEdit::multiline(&mut self.rule_import_text)
                        .desired_rows(10)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("DOMAIN-SUFFIX,example.com,PROXY"),
                );
                crate::gui::widgets::note_text_field(ui, &field);
                if field.changed() {
                    let text = self.rule_import_text.clone();
                    self.rule_import_preview = Some(crate::proxy::rule_io::parse_rules_text(&text));
                }
                let preview = self.rule_import_preview.clone();
                match preview {
                    Some(preview) => {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(preview.summary())
                                .strong()
                                .color(theme::text_primary()),
                        );
                        if !preview.rules.is_empty() {
                            egui::ScrollArea::vertical()
                                .id_salt("rule_import_preview")
                                .max_height(140.0)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    for line in crate::proxy::rule_io::export_lines(&preview.rules)
                                        .unwrap_or_default()
                                    {
                                        ui.label(RichText::new(line).small().monospace());
                                    }
                                });
                        }
                        if !preview.unparsed.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "认不出的前 {} 条：{}",
                                    preview.unparsed.len().min(3),
                                    preview
                                        .unparsed
                                        .iter()
                                        .take(3)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(" / ")
                                ))
                                .small()
                                .color(theme::warning()),
                            );
                        }
                        apply = ui
                            .add_enabled(
                                !preview.rules.is_empty() && !self.proxy_busy,
                                primary_btn("追加到规则表并保存"),
                            )
                            .on_hover_text("已存在的同样规则会跳过；保存时照常去重")
                            .clicked();
                    }
                    None => {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("粘贴或从剪贴板读取后，这里会先给解析结果再让导入。")
                                .small()
                                .color(theme::muted()),
                        );
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if apply {
            self.apply_rule_import();
        }
        if !open {
            self.show_rule_import = false;
            self.rule_import_preview = None;
        }
    }

    /// 导入框旁边那个「?」：把每种能吃的输入形状各给一段可直接用的示范。
    /// 示范不是手写的漂亮话 —— `rule_io` 里有测试把每段喂回解析器，条数对不上就红，
    /// 所以照着抄不会错；「填入导入框」还会立刻开出解析预览让他先看清进了什么。
    pub(crate) fn ui_rule_import_help(&mut self, ctx: &egui::Context) {
        if !self.show_rule_import_help {
            return;
        }
        let (msg, _rect) = ui_rule_import_help_window(ctx);
        match msg {
            RuleHelpMsg::Nothing => {}
            RuleHelpMsg::Close => self.show_rule_import_help = false,
            RuleHelpMsg::Status(text) => self.status = text,
            RuleHelpMsg::Fill(body) => {
                self.rule_import_text = body.to_string();
                self.rule_import_preview = Some(crate::proxy::rule_io::parse_rules_text(
                    &self.rule_import_text,
                ));
                // 填完直接把导入框开出来：他要看到的就是"这段抄进去到底进了什么"。
                self.show_rule_import = true;
                self.show_rule_import_help = false;
            }
        }
    }

    fn apply_rule_import(&mut self) {
        let Some(preview) = self.rule_import_preview.clone() else {
            return;
        };
        let mut added = 0usize;
        for rule in &preview.rules {
            if self
                .proxy_config
                .rules
                .iter()
                .any(|existing| existing == rule)
            {
                continue;
            }
            self.proxy_config.rules.push(rule.clone());
            added += 1;
        }
        let duplicates = preview.rules.len() - added;
        self.status = format!(
            "导入完成：新增 {added} 条，跳过重复 {duplicates} 条，规则表现共 {} 条",
            self.proxy_config.rules.len()
        );
        self.show_rule_import = false;
        self.rule_import_text.clear();
        self.rule_import_preview = None;
        self.save_proxy_config();
    }

    pub(crate) fn ui_recommended_rules_confirm(&mut self, ctx: &egui::Context) {
        if !self.show_recommended_rules_confirm {
            return;
        }

        let modal = egui::Modal::new(egui::Id::new("recommended_rules_confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading("覆盖分流规则");
            ui.label("现有分流规则将被推荐规则覆盖，确认后立即保存。");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    Some(false)
                } else if ui.add(primary_btn("确认覆盖")).clicked() {
                    Some(true)
                } else {
                    None
                }
            })
            .inner
        });

        match modal.inner {
            Some(true) => {
                self.proxy_config.rules = recommended_traffic_rules();
                self.show_recommended_rules_confirm = false;
                self.save_proxy_config();
            }
            Some(false) => self.show_recommended_rules_confirm = false,
            None if modal.should_close() => self.show_recommended_rules_confirm = false,
            None => {}
        }
    }

    pub(crate) fn ui_proxy_network(&mut self, ui: &mut Ui) {
        section_title(ui, "DNS / TUN", "配置接管模式、监听端口与 DNS");
        let editable = !self.proxy_busy && self.proxy_status.config_editable();
        let mut save_clicked = false;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Plug, 14.0, theme::accent());
                            ui.label(RichText::new("接管模式架构").strong().size(14.0).color(theme::text_primary()));
                        });
                        ui.add_space(6.0);

                        // 双卡架构选择器 (System Proxy vs TUN Virtual Device)
                        ui.columns(2, |column_uis| {
                            // 卡片 1: 系统代理
                            let ui = &mut column_uis[0];
                            let is_sys = self.proxy_config.mode == ProxyMode::System;
                            let (stroke, bg) = if is_sys {
                                (Stroke::new(1.5_f32, theme::accent()), theme::accent_bg())
                            } else {
                                (Stroke::new(1.0_f32, theme::border_subtle()), theme::sidebar())
                            };
                            let card1 = Frame::NONE
                                .fill(bg)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(Margin::symmetric(14, 10))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        icon_label(ui, Icon::Globe, 16.0, if is_sys { theme::accent() } else { theme::muted() });
                                        ui.label(RichText::new("系统代理 (System)").strong().color(theme::text_primary()));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if is_sys {
                                                ui.label(RichText::new("● 激活中").small().strong().color(theme::accent_strong()));
                                            } else {
                                                ui.label(RichText::new("○ 点击选用").small().color(theme::muted()));
                                            }
                                        });
                                    });
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("自动配置系统 HTTP / SOCKS5 代理").small().color(theme::text_secondary()));
                                    ui.label(RichText::new("适用于浏览器与支持系统代理的桌面应用，零系统侵入").small().color(theme::muted()));
                                });
                            if card1.response.interact(Sense::click()).clicked() {
                                self.proxy_config.mode = ProxyMode::System;
                            }

                            // 卡片 2: TUN 虚拟网卡
                            let ui = &mut column_uis[1];
                            let is_tun = self.proxy_config.mode == ProxyMode::Tun;
                            let (stroke, bg) = if is_tun {
                                (Stroke::new(1.5_f32, theme::accent()), theme::accent_bg())
                            } else {
                                (Stroke::new(1.0_f32, theme::border_subtle()), theme::sidebar())
                            };
                            let card2 = Frame::NONE
                                .fill(bg)
                                .stroke(stroke)
                                .corner_radius(CornerRadius::same(10))
                                .inner_margin(Margin::symmetric(14, 10))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        icon_label(ui, Icon::Lightning, 16.0, if is_tun { theme::accent() } else { theme::muted() });
                                        ui.label(RichText::new("TUN 虚拟网卡 (Global)").strong().color(theme::text_primary()));
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if is_tun {
                                                ui.label(RichText::new("● 激活中").small().strong().color(theme::accent_strong()));
                                            } else {
                                                ui.label(RichText::new("○ 点击选用").small().color(theme::muted()));
                                            }
                                        });
                                    });
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("创建底层虚拟网卡接管全系统 IP 层流量").small().color(theme::text_secondary()));
                                    ui.label(RichText::new("拦截 UDP/ICMP、游戏加速、终端命令全自动透明分流").small().color(theme::muted()));
                                });
                            if card2.response.interact(Sense::click()).clicked() {
                                self.proxy_config.mode = ProxyMode::Tun;
                            }
                        });

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.label("运行内核");
                            egui::ComboBox::from_id_salt("proxy_kernel_preference")
                                .selected_text(match self.proxy_config.kernel_preference {
                                    KernelPreference::Auto => "自动选择",
                                    KernelPreference::Mihomo => "Mihomo",
                                    KernelPreference::SingBox => "Sing-box",
                                    KernelPreference::Xray => "Xray",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.proxy_config.kernel_preference,
                                        KernelPreference::Auto,
                                        "自动选择",
                                    );
                                    ui.selectable_value(
                                        &mut self.proxy_config.kernel_preference,
                                        KernelPreference::Mihomo,
                                        "Mihomo",
                                    );
                                    ui.selectable_value(
                                        &mut self.proxy_config.kernel_preference,
                                        KernelPreference::SingBox,
                                        "Sing-box",
                                    );
                                    ui.selectable_value(
                                        &mut self.proxy_config.kernel_preference,
                                        KernelPreference::Xray,
                                        "Xray",
                                    );
                                });
                        });
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Mixed 端口");
                            ui.add(
                                egui::DragValue::new(&mut self.proxy_config.mixed_port).range(1..=u16::MAX),
                            );
                            ui.label("控制端口");
                            ui.add(
                                egui::DragValue::new(&mut self.proxy_config.controller_port)
                                    .range(1..=u16::MAX),
                            );
                            ui.checkbox(&mut self.proxy_config.allow_lan, "允许局域网");
                            ui.checkbox(
                                &mut self.proxy_config.terminal_proxy,
                                "终端代理（写入 HTTP_PROXY / HTTPS_PROXY，Git·curl·npm·pip 才认）",
                            )
                            .on_hover_text(
                                "启动代理时把变量写进 HKCU\\Environment，停止时清除；只清我们自己写的值，\n\
                                 你原有的代理变量不会被碰。新开的终端立即生效，已开着的终端要重开。",
                            );
                        });

                        if self.proxy_config.mode == ProxyMode::Tun {
                            ui.add_space(14.0);
                            ui.strong("TUN 参数");
                            ui.horizontal_wrapped(|ui| {
                                ui.label("协议栈");
                                egui::ComboBox::from_id_salt("tun_stack")
                                    .selected_text(tun_stack_label(self.proxy_config.tun.stack))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.proxy_config.tun.stack,
                                            TunStack::Mixed,
                                            "Mixed",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.tun.stack,
                                            TunStack::System,
                                            "System",
                                        );
                                        ui.selectable_value(
                                            &mut self.proxy_config.tun.stack,
                                            TunStack::Gvisor,
                                            "gVisor",
                                        );
                                    });
                                ui.checkbox(&mut self.proxy_config.tun.auto_route, "自动路由");
                                ui.checkbox(
                                    &mut self.proxy_config.tun.auto_detect_interface,
                                    "自动检测网卡",
                                );
                            });
                        }

                        ui.add_space(14.0);
                        ui.strong("DNS");
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut self.proxy_config.dns.enabled, "启用");
                            ui.checkbox(&mut self.proxy_config.dns.ipv6, "IPv6");
                            egui::ComboBox::from_id_salt("dns_enhanced_mode")
                                .selected_text(dns_mode_label(self.proxy_config.dns.enhanced_mode))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut self.proxy_config.dns.enhanced_mode,
                                        DnsEnhancedMode::FakeIp,
                                        "Fake-IP",
                                    );
                                    ui.selectable_value(
                                        &mut self.proxy_config.dns.enhanced_mode,
                                        DnsEnhancedMode::RedirHost,
                                        "Redir-Host",
                                    );
                                });
                        });
                        if self.proxy_config.dns.nameservers.is_empty() {
                            self.proxy_config
                                .dns
                                .nameservers
                                .push(crate::proxy::DEFAULT_DNS_PRIMARY.into());
                        }
                        for (index, nameserver) in self.proxy_config.dns.nameservers.iter_mut().enumerate()
                        {
                            let hint = if index == 0 {
                                crate::proxy::DEFAULT_DNS_PRIMARY
                            } else {
                                crate::proxy::DEFAULT_DNS_SECONDARY
                            };
                            ui.horizontal(|ui| {
                                ui.label(format!("DNS {}", index + 1));
                                let ns_field = ui.add_sized(
                                    [420.0, 24.0],
                                    egui::TextEdit::singleline(nameserver).hint_text(hint),
                                );
                                crate::gui::widgets::note_text_field(ui, &ns_field);
                            });
                        }
                        ui.add_space(12.0);
                        save_clicked = ui.add(primary_btn("保存网络配置")).clicked();
                    });
                    if !editable {
                        ui.label(RichText::new("停止代理后可修改运行配置").color(theme::muted()));
                    }
                });
                ui.add_space(16.0);
            });

        if save_clicked {
            self.save_proxy_config();
        }
    }

    pub(crate) fn ui_proxy_kernels(&mut self, ui: &mut Ui) {
        section_title(
            ui,
            "代理内核",
            "应用独立下载、校验、备份和更新，不依赖 v2rayN",
        );

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("存储目录 data/kernels · 来源为官方 GitHub Release")
                            .small()
                            .color(theme::muted()),
                    );
                    if ui
                        .add_enabled(!self.kernel_busy, egui::Button::new("检查更新"))
                        .clicked()
                    {
                        self.reload_kernels();
                        self.status = "正在检查官方内核版本...".into();
                    }
                    if let Some(kp) = &self.kernel_progress {
                        ui.label(
                            RichText::new(format!(
                                "正在更新 {} ({})",
                                kp.kind.display_name(),
                                kp.stage.label()
                            ))
                            .small()
                            .color(theme::accent_strong()),
                        );
                    }
                    if self.kernel_busy && ui.add(danger_btn("取消下载")).clicked() {
                        self.kernel_cancel.store(true, Ordering::Relaxed);
                        self.status = "正在取消内核下载...".into();
                    }
                });

                ui.add_space(8.0);
                let mut update_kind = None;

                let col_count = self.kernels.len().max(1);
                ui.columns(col_count, |column_uis| {
                    for (i, kernel) in self.kernels.iter().enumerate() {
                        let ui = &mut column_uis[i];
                        let col_max_w = ui.available_width();
                        ui.set_max_width(col_max_w);
                        ui.set_clip_rect(ui.max_rect());

                        let version = kernel.version.as_deref().unwrap_or("未检测到");
                        let release = self.kernel_releases.iter().find(|release| release.kind == kernel.kind);
                        let latest = release
                            .and_then(|release| release.latest_version.as_deref())
                            .unwrap_or("检查中");
                        let path = kernel
                            .path
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "-".into());

                        let is_running = self.proxy_status.kernel == Some(kernel.kind);
                        let is_preferred = self.proxy_config.kernel_preference == KernelPreference::Auto && kernel.kind == KernelKind::Mihomo
                            || match self.proxy_config.kernel_preference {
                                KernelPreference::Mihomo => kernel.kind == KernelKind::Mihomo,
                                KernelPreference::SingBox => kernel.kind == KernelKind::SingBox,
                                KernelPreference::Xray => kernel.kind == KernelKind::Xray,
                                _ => false,
                            };

                        let card_stroke = if is_running {
                            Stroke::new(1.5_f32, theme::accent())
                        } else if is_preferred {
                            Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.5))
                        } else {
                            Stroke::new(1.0_f32, theme::border_subtle())
                        };

                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(card_stroke)
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width());

                                // 1. 卡片头部：图标 + 标题 + 运行徽标
                                ui.horizontal(|ui| {
                                    let (chip_icon, chip_color) = match kernel.kind {
                                        KernelKind::Mihomo => (Icon::Cpu, theme::accent()),
                                        KernelKind::SingBox => (Icon::Lightning, theme::accent_strong()),
                                        KernelKind::Xray => (Icon::Radar, theme::text_secondary()),
                                    };
                                    icon_label(ui, chip_icon, 15.0, chip_color);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(kernel.kind.display_name())
                                                .strong()
                                                .size(14.0)
                                                .color(theme::text_primary()),
                                        )
                                        .truncate(),
                                    );

                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if is_running {
                                            let (beacon, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                            ui.painter().circle_filled(beacon.center(), 4.0, theme::accent());
                                            ui.label(RichText::new("运行中").small().strong().color(theme::accent_strong()));
                                        } else if kernel.available {
                                            ui.label(RichText::new("● 就绪").small().color(theme::accent_strong()));
                                        } else {
                                            ui.label(RichText::new("○ 未安装").small().color(theme::muted()));
                                        }
                                    });
                                });

                                ui.add_space(6.0);

                                // 2. 特性胶囊标签
                                let features = match kernel.kind {
                                    KernelKind::Mihomo => "智能分流 · TUN · Fake-IP · 规则集",
                                    KernelKind::SingBox => "ShadowTLS · VLESS · Hysteria2",
                                    KernelKind::Xray => "XTLS · VLESS · Trojan · Splithttp",
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(features)
                                            .small()
                                            .color(theme::text_secondary()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(features);

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                // 3. 版本度量
                                let clean_ver = clean_kernel_version(version);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("当前:").small().color(theme::muted()));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(clean_ver)
                                                .strong()
                                                .small()
                                                .monospace()
                                                .color(theme::text_primary()),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(format!("完整版本：\n{version}"));
                                });
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("最新:").small().color(theme::muted()));
                                    let (lat_text, lat_color) = if release.is_some_and(|r| r.update_available) {
                                        (format!("{latest} (可更新)"), theme::warning())
                                    } else {
                                        (format!("{latest} (最新)"), theme::text_secondary())
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(lat_text)
                                                .small()
                                                .strong()
                                                .color(lat_color),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(format!("官方远端版本：{latest}"));
                                });

                                ui.add_space(4.0);

                                // 4. 路径信息
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Package, 11.0, theme::muted());
                                    let display_path = if path == "-" || path.is_empty() {
                                        "-".to_string()
                                    } else {
                                        std::path::Path::new(&path)
                                            .file_name()
                                            .map(|name| name.to_string_lossy().to_string())
                                            .unwrap_or_else(|| path.clone())
                                    };
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(display_path)
                                                .small()
                                                .monospace()
                                                .color(theme::muted()),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(format!("文件路径：\n{path}"));
                                });

                                ui.add_space(8.0);

                                // 5. 操作按钮：若本内核正处于下载/更新中，就地展示进度条，不挤压破坏卡片高度
                                let is_this_kernel_busy = self.kernel_busy
                                    && self.kernel_progress.as_ref().map(|p| p.kind) == Some(kernel.kind);

                                if is_this_kernel_busy {
                                    if let Some(p) = &self.kernel_progress {
                                        let text = if p.stage == crate::kernel::KernelUpdateStage::Downloading {
                                            if p.total > 0 {
                                                let pct = (p.downloaded as f64 / p.total as f64 * 100.0).clamp(0.0, 100.0) as u32;
                                                format!("{pct}% · {}/s", format_bytes(p.bytes_per_second as u64))
                                            } else {
                                                format!("{}/s", format_bytes(p.bytes_per_second as u64))
                                            }
                                        } else {
                                            p.stage.label().to_string()
                                        };
                                        ui.add(
                                            egui::ProgressBar::new(p.fraction())
                                                .animate(true)
                                                .desired_width(ui.available_width())
                                                .text(RichText::new(text).small().strong()),
                                        );
                                    }
                                } else {
                                    let can_update = release.is_some_and(|release| release.update_available)
                                        && !self.kernel_busy
                                        && !self.proxy_status.kernel_in_use(kernel.kind);
                                    let btn_label = if !kernel.available {
                                        "📥 下载内核"
                                    } else if can_update {
                                        "⚡ 立即更新"
                                    } else {
                                        "✅ 已是最新"
                                    };
                                    let btn = egui::Button::new(RichText::new(btn_label).small().strong())
                                        .min_size(Vec2::new(ui.available_width(), 26.0))
                                        .fill(if can_update || !kernel.available { theme::accent_bg() } else { theme::sidebar() })
                                        .stroke(Stroke::new(1.0_f32, if can_update || !kernel.available { theme::accent() } else { theme::border_subtle() }))
                                        .corner_radius(CornerRadius::same(8));

                                    if ui.add_enabled(can_update || !kernel.available, btn).clicked() {
                                        update_kind = Some(kernel.kind);
                                    }
                                }
                            });
                    }
                });

                ui.add_space(16.0);
                if let Some(kind) = update_kind {
                    self.open_kernel_download(kind);
                }
            });
    }

    pub(crate) fn ui_proxy_logs(&mut self, ui: &mut Ui) {
        let kernel_name = self
            .proxy_status
            .kernel
            .map(KernelKind::display_name)
            .unwrap_or("代理内核");
        ui.horizontal(|ui| {
            section_title(ui, "运行日志", &format!("{kernel_name} 最近输出"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("刷新日志").clicked() {
                    self.reload_proxy_logs();
                }
                if !self.proxy_log.is_empty() {
                    if ui.button("清空显示").clicked() {
                        self.proxy_log.clear();
                    }
                    if ui.button("复制全部").clicked() {
                        ui.ctx().copy_text(self.proxy_log.clone());
                        self.status = "日志内容已复制到剪贴板".into();
                    }
                }
            });
        });
        card_frame().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.proxy_log.is_empty() {
                        let is_running = self.proxy_status.state == ProxyState::Running;
                        if is_running {
                            ui.label(
                                RichText::new("暂无输出（内核运行中，正在等待新日志...）")
                                    .color(theme::muted()),
                            );
                        } else if let Some(err) = &self.proxy_status.last_error {
                            ui.label(
                                RichText::new(format!("代理内核未运行（最近异常：{err}）"))
                                    .color(theme::danger()),
                            );
                            ui.label(
                                RichText::new(
                                    "请检查节点连通性或内核配置，启动代理后将在此显示内核输出日志。",
                                )
                                .color(theme::muted())
                                .small(),
                            );
                        } else {
                            ui.label(
                                RichText::new(
                                    "代理内核未运行。点击上方「启动代理」后，此处将实时展示内核输出日志。",
                                )
                                .color(theme::muted()),
                            );
                        }
                    } else {
                        ui.monospace(&self.proxy_log);
                    }
                });
        });
    }

    /// 落地风险弹窗：查询过程和结论都只在这里显示，主面板不再常驻一行判定。
    pub(crate) fn ui_ip_risk_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_ip_risk_dialog {
            return;
        }
        if self.ip_risk_busy || self.ip_risk_egress_pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        let mut open = self.show_ip_risk_dialog;
        let mut retry = false;
        let mut copy_payload: Option<String> = None;

        egui::Window::new(RichText::new("IP 检测 · 归属与风控画像").strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if self.ip_risk_busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(&self.status).color(theme::text_primary()));
                    });
                    return;
                }

                if let Some(error) = &self.ip_risk_error {
                    Frame::NONE
                        .fill(theme::danger_bg())
                        .stroke(Stroke::new(1.0_f32, theme::danger()))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(RichText::new(error).color(theme::danger()));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "节点地址是域名时查不到归属；也可以重试一次。",
                                )
                                .small()
                                .color(theme::muted()),
                            );
                        });
                    ui.add_space(6.0);
                    if ui.button("重新查询").clicked() {
                        retry = true;
                    }
                    return;
                }

                let Some(risk) = self.ip_risk.as_ref() else {
                    ui.label(RichText::new("还没有查询结果。").color(theme::muted()));
                    return;
                };

                let color = if risk.datacenter { theme::warning() } else { theme::accent_strong() };
                ui.label(
                    RichText::new("入口（节点服务器 IP）")
                        .small()
                        .color(theme::muted()),
                );
                Frame::NONE
                    .fill(if risk.datacenter {
                        theme::warning_bg()
                    } else {
                        theme::accent_bg()
                    })
                    .stroke(Stroke::new(1.0_f32, color))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Shield, 18.0, color);
                            ui.label(
                                RichText::new(if risk.datacenter {
                                    "机房 / 云厂商"
                                } else {
                                    "住宅 / ISP"
                                })
                                .strong()
                                .size(15.0)
                                .color(color),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new(&risk.verdict).small().color(theme::text_primary()));
                    });

                ui.add_space(8.0);
                egui::Grid::new("ip_risk_detail")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        if risk.hostname != risk.ip {
                            ui.label(RichText::new("节点地址").small().color(theme::muted()));
                            ui.label(RichText::new(&risk.hostname).color(theme::text_primary()));
                            ui.end_row();
                        }

                        ui.label(
                            RichText::new(if risk.hostname == risk.ip {
                                "落地 IP"
                            } else {
                                "解析到"
                            })
                            .small()
                            .color(theme::muted()),
                        );
                        ui.label(RichText::new(&risk.ip).color(theme::text_primary()));
                        ui.end_row();

                        ui.label(RichText::new("地区").small().color(theme::muted()));
                        ui.label(RichText::new(if risk.country.is_empty() {
                            "未知"
                        } else {
                            &risk.country
                        })
                        .color(theme::text_primary()));
                        ui.end_row();

                        ui.label(RichText::new("ASN / 运营商").small().color(theme::muted()));
                        ui.label(RichText::new(if risk.org.is_empty() {
                            "无 ASN 信息"
                        } else {
                            &risk.org
                        })
                        .color(theme::text_primary()));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                // 出口侧：本机只有一个 mixed 端口，所以"经代理查出口"只对**当前在跑的
                // 那个节点**成立；别的节点必须用临时内核实测，否则显示的是别人的出口。
                let target = self.ip_risk_target.as_deref();
                let current = self.proxy_status.node_id.as_deref();
                let target_is_running = self.proxy_status.local_proxy_active()
                    && matches!((target, current), (Some(a), Some(b)) if a == b);
                let egress_belongs_to_target = self.ip_risk_egress_for.is_some()
                    && self.ip_risk_egress_for == self.ip_risk_target;
                let egress = if egress_belongs_to_target {
                    self.ip_risk_egress.as_ref()
                } else {
                    None
                };
                let egress_error = if egress_belongs_to_target {
                    self.ip_risk_egress_error.as_deref()
                } else {
                    None
                };
                if self.ip_risk_egress_pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("正在经当前代理查出口 IP...")
                                .small()
                                .color(theme::muted()),
                        );
                    });
                } else if let Some(egress) = egress {
                    let color = if egress.datacenter { theme::warning() } else { theme::accent_strong() };
                    Frame::NONE
                        .fill(if egress.datacenter {
                            theme::warning_bg()
                        } else {
                            theme::accent_bg()
                        })
                        .stroke(Stroke::new(1.0_f32, color))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Globe, 16.0, color);
                                ui.label(
                                    RichText::new("站点实际看到的出口")
                                        .strong()
                                        .color(color),
                                );
                            });
                            ui.label(
                                RichText::new(egress.headline())
                                    .small()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(&egress.verdict)
                                    .small()
                                    .color(theme::text_primary()),
                            );
                            if egress.ip != risk.ip {
                                ui.label(
                                    RichText::new(
                                        "入口和出口不是同一个 IP：该节点有中转，风控认的是上面这一行。",
                                    )
                                    .small()
                                    .color(theme::warning()),
                                );
                            }
                        });
                } else if let Some(error) = egress_error {
                    ui.label(
                        RichText::new(format!("出口查询失败：{error}"))
                            .small()
                            .color(theme::danger()),
                    );
                } else if target.is_some() {
                    ui.label(
                        RichText::new(if target_is_running {
                            "出口尚未查询。"
                        } else {
                            "该节点不是当前在跑的节点，出口未知（经本机代理查只会得到当前节点的出口）。"
                        })
                        .small()
                        .color(theme::muted()),
                    );
                }

                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        if risk.hostname == risk.ip {
                            "归类只看 ASN / 公司名关键字，会漏判（例如小商家 VPS），请自行核对上面这行 ASN。\
                             查询走直连、不经过代理，对象就是这个节点服务器 IP。"
                        } else {
                            "节点填的是域名，这里查的是它解析到的 IP —— 域名前置 / ECH / CDN 中转的节点，\
                             这个地址常常是前端而不是真实落地出口（站点侧看到的是出口 IP）。\
                             归类只看 ASN / 公司名关键字，会漏判，请自行核对上面这行 ASN。"
                        },
                    )
                    .small()
                    .color(theme::muted()),
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("复制").clicked() {
                        let mut text = format!("入口：{}\n{}", risk.headline(), risk.verdict);
                        if let Some(egress) = &self.ip_risk_egress {
                            text.push_str(&format!(
                                "\n出口：{}\n{}",
                                egress.headline(),
                                egress.verdict
                            ));
                        }
                        copy_payload = Some(text);
                    }
                    if ui.button("重新查询").clicked() {
                        retry = true;
                    }
                });
            });

        if retry {
            self.ip_risk = None;
            self.check_ip_risk();
        }
        if let Some(text) = copy_payload {
            ctx.copy_text(text);
            self.status = "已复制落地 IP 归属信息".into();
        }
        self.show_ip_risk_dialog = open;
    }
}

/// 提取干净精简的内核版本号用于卡片紧凑展示（例如从长横幅中提取 `v1.19.32` 或 `1.14.3`），
/// 避免 80 字符的完整编译横幅导致列宽被强行撑爆并覆盖右侧卡片。
pub fn clean_kernel_version(raw: &str) -> &str {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "未检测到" || trimmed == "-" {
        return trimmed;
    }

    // 优先从按空格分隔的词项中寻找形如 v1.19.32、1.14.3 或 26.3.27 的标准版本标记
    for part in trimmed.split_whitespace() {
        let clean = part.trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-');
        let check = clean.strip_prefix('v').unwrap_or(clean);
        if let Some(first_char) = check.chars().next() {
            if first_char.is_ascii_digit() && check.contains('.') {
                return clean;
            }
        }
    }

    // 如果未识别出标准数字版本（如包含短 commit hash 或特殊名称），安全截取最多 16 个字符
    if trimmed.chars().count() > 16 {
        let (idx, _) = trimmed.char_indices().nth(16).unwrap_or((trimmed.len(), ' '));
        &trimmed[..idx]
    } else {
        trimmed
    }
}

/// 判断节点是否属于指定的分流策略组
pub fn node_matches_group(
    node: &NodeSummary,
    group_name: &str,
    kernel_groups: &[KernelGroup],
) -> bool {
    node_matches_group_depth(node, group_name, kernel_groups, 0)
}

fn node_matches_group_depth(
    node: &NodeSummary,
    group_name: &str,
    kernel_groups: &[KernelGroup],
    depth: usize,
) -> bool {
    if depth > 4 {
        return false;
    }
    if group_name == "全部" || group_name == "PROXY" {
        return true;
    }

    let country = node
        .country
        .clone()
        .or_else(|| crate::parser::guess_country(&node.name))
        .unwrap_or_default()
        .to_ascii_uppercase();
    let lower_name = node.name.to_lowercase();

    // 针对内置知名策略组做精准语义过滤
    match group_name {
        "美国节点" | "US" => {
            return (country == "US"
                || lower_name.contains("us")
                || lower_name.contains("united states")
                || node.name.contains("美"))
                && !lower_name.contains("warp");
        }
        "AI分组" | "AI" => {
            return matches!(country.as_str(), "US" | "JP" | "SG" | "TW" | "KR" | "GB" | "DE")
                || lower_name.contains("ai")
                || lower_name.contains("openai")
                || lower_name.contains("chatgpt")
                || lower_name.contains("claude")
                || lower_name.contains("grok")
                || country == "US";
        }
        "多线并发下载" | "下载" | "普通代理" => {
            return true;
        }
        _ => {}
    }

    // 1. 若内核正在运行且上报了该策略组成员列表：优先按真实内核成员匹配
    if let Some(group) = kernel_groups.iter().find(|g| g.name == group_name) {
        if !group.members.is_empty() {
            if group.members.iter().any(|m| m == &node.name || m == &node.server) {
                return true;
            }
            // 嵌套策略组展开（例如 AI分组 的 members 包含 "美国节点"）
            let sub_matched = group.members.iter().any(|m| {
                if m != group_name && m != "DIRECT" && m != "REJECT" && m != "PROXY" {
                    node_matches_group_depth(node, m, kernel_groups, depth + 1)
                } else {
                    false
                }
            });
            if sub_matched {
                return true;
            }
        }
    }

    let lower_group = group_name.to_lowercase();
    lower_name.contains(&lower_group) || country.to_lowercase() == lower_group
}

/// 内置智能策略组的名字。它们**只有**在 `smart_routing` 开着时才会被写进生成的配置
/// （`proxy` 侧那几处 `if options.smart_routing`），所以胶囊条也照这个判在不在。
const SMART_ROUTING_GROUPS: [&str; 4] = ["AI分组", "美国节点", "多线并发下载", "普通代理"];

/// 自定义覆写 YAML 里真的定义了哪些组。两种写法都认（整份文档带 `proxy-groups:`，
/// 或者直接给一个组数组 —— 与 `proxy` 侧应用覆写时的分叉一致）。解析不动就当没有。
fn override_group_names(override_yaml: Option<&str>) -> Vec<String> {
    let Some(text) = override_yaml.filter(|raw| !raw.trim().is_empty()) else {
        return Vec::new();
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Vec::new();
    };
    let groups = value
        .get("proxy-groups")
        .and_then(serde_yaml::Value::as_sequence)
        .or_else(|| value.as_sequence())
        .cloned()
        .unwrap_or_default();
    groups
        .iter()
        .filter_map(|group| group.get("name").and_then(|name| name.as_str()))
        .map(str::to_string)
        .collect()
}

/// 策略组胶囊条该显示谁：**默认只有「全部」**，其余的组只有"配置里真的有它"才出现。
/// 三个来源 —— ① `smart_routing` 打开的内置四组；② 覆写 YAML 里定义的组；
/// ③ 内核上报且成员非空的组。
///
/// 注意这里刻意**不**用"能不能筛出节点"当判据：内置的 `普通代理`/`多线并发下载` 走的是
/// 语义兜底匹配，任何一台机器上都能匹配到全部节点，那样等于永远显示。
fn visible_group_names(
    smart_routing: bool,
    override_yaml: Option<&str>,
    kernel_groups: &[KernelGroup],
) -> Vec<String> {
    let mut names: Vec<String> = vec!["全部".to_string()];
    let defined = override_group_names(override_yaml);
    let reported = |name: &str| {
        kernel_groups
            .iter()
            .any(|group| group.name == name && !group.members.is_empty())
    };

    for name in SMART_ROUTING_GROUPS {
        if smart_routing || defined.iter().any(|d| d == name) || reported(name) {
            names.push(name.to_string());
        }
    }
    for name in &defined {
        if !names.iter().any(|shown| shown == name) {
            names.push(name.clone());
        }
    }
    for group in kernel_groups {
        if group.members.is_empty()
            || matches!(group.name.as_str(), "GLOBAL" | "DIRECT" | "REJECT" | "PASS")
            || names.iter().any(|shown| *shown == group.name)
        {
            continue;
        }
        names.push(group.name.clone());
    }
    names
}

/// 重建控制台「可用节点」网格：按分流策略组、地区、搜索词过滤并排序
fn build_proxy_view(
    all: &[NodeSummary],
    group_name: &str,
    kernel_groups: &[KernelGroup],
    region: &str,
    search: &str,
    sort: ProxySortBy,
    rev: u64,
) -> ProxyView {
    const REGIONS: [&str; 8] = ["HK", "JP", "US", "SG", "TW", "KR", "GB", "DE"];
    let mut rows: Vec<ProxyViewRow> = Vec::with_capacity(all.len());
    for (index, node) in all.iter().enumerate() {
        if !node_matches_group(node, group_name, kernel_groups) {
            continue;
        }
        let country = node
            .country
            .clone()
            .or_else(|| crate::parser::guess_country(&node.name))
            .unwrap_or_default()
            .to_ascii_uppercase();
        if region != "all" {
            let hit = if region == "OTHER" {
                !REGIONS.contains(&country.as_str())
            } else {
                country == region
            };
            if !hit {
                continue;
            }
        }
        let lower_name = node.name.to_lowercase();
        if !search.is_empty()
            && !lower_name.contains(search)
            && !node.server.to_lowercase().contains(search)
            && !country.to_lowercase().contains(search)
        {
            continue;
        }
        rows.push(ProxyViewRow {
            index,
            country,
            lower_name,
        });
    }
    match sort {
        ProxySortBy::Name => rows.sort_by(|a, b| a.lower_name.cmp(&b.lower_name)),
        ProxySortBy::Latency => rows.sort_by(|a, b| {
            let node_a = &all[a.index];
            let node_b = &all[b.index];
            (
                node_a.latency_ok != Some(true),
                node_a.latency_ms.unwrap_or(i64::MAX),
                &a.lower_name,
            )
                .cmp(&(
                    node_b.latency_ok != Some(true),
                    node_b.latency_ms.unwrap_or(i64::MAX),
                    &b.lower_name,
                ))
        }),
    }
    ProxyView {
        rev,
        group: group_name.to_owned(),
        region: region.to_owned(),
        search: search.to_owned(),
        sort,
        rows,
    }
}

/// 「导入格式示范」弹框里点出来的动作。
/// 整个弹框抽成不碰 `VpnApp` 的自由函数，好让离屏 `egui::Context` 能单独渲染它做布局验收。
pub(crate) enum RuleHelpMsg {
    Nothing,
    Status(String),
    Fill(&'static str),
    Close,
}

fn set_help_msg(slot: &mut RuleHelpMsg, next: RuleHelpMsg) {
    if matches!(*slot, RuleHelpMsg::Nothing) {
        *slot = next;
    }
}

/// 每种能吃的输入形状给一段可直接抄用的示范 + 支持/跳过的类型清单。
/// 示范文本本身由 `rule_io` 的测试逐段喂回解析器（条数对不上就红），
/// 所以这里写的"可导入 N 条"不是宣传语。
/// 「导入格式示范」整个弹框。宽度与滚动区高度都跟着屏幕算 ——
/// 字体调到 140% 时逻辑屏幕只剩物理的 0.71 倍，写死尺寸会把窗口顶出右缘
/// （离屏量到 660 宽的屏面上窗口画到 662）。
/// 抽成不碰 `VpnApp` 的自由函数，好让离屏 `egui::Context` 真开一次窗口做验收。
fn ui_rule_import_help_window(ctx: &egui::Context) -> (RuleHelpMsg, egui::Rect) {
    let screen = ctx.screen_rect();
    let width = (screen.width() - 96.0).clamp(320.0, 680.0);
    let scroll_height = (screen.height() * 0.5).clamp(180.0, 430.0);
    let mut open = true;
    let mut msg = RuleHelpMsg::Nothing;
    // `Window::open` 借走了 `open`，闭包里只记一笔，出框之后再合。
    let shown = egui::Window::new("导入格式示范")
        .id(egui::Id::new("vpn-parser:rule-import-help"))
        .default_width(width)
        .default_pos(screen.min + Vec2::new(48.0, 32.0))
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            msg = ui_rule_import_help_body(ui, scroll_height);
        });
    let rect = shown
        .map(|inner| inner.response.rect)
        .unwrap_or(egui::Rect::NOTHING);
    if !open && matches!(msg, RuleHelpMsg::Nothing) {
        msg = RuleHelpMsg::Close;
    }
    (msg, rect)
}

fn ui_rule_import_help_body(ui: &mut Ui, scroll_height: f32) -> RuleHelpMsg {
    let mut msg = RuleHelpMsg::Nothing;
    ui.label(
        RichText::new(
            "目标写 PROXY / DIRECT / REJECT 以外的自定义组名，一律落到本程序的代理组；\
             GEOIP 的 no-resolve 由本程序自动补；重复的规则保存时去重。",
        )
        .small()
        .color(theme::muted()),
    );
    ui.label(
        RichText::new(format!(
            "认这些类型：{}",
            crate::proxy::rule_io::SUPPORTED_KINDS.join("、")
        ))
        .small()
        .color(theme::text_secondary()),
    );
    ui.label(
        RichText::new(format!(
            "点名跳过：{}、MATCH/FINAL 兜底行",
            crate::proxy::rule_io::UNSUPPORTED_KINDS.join("、")
        ))
        .small()
        .color(theme::warning()),
    );
    egui::ScrollArea::vertical()
        .id_salt("rule_import_help")
        .max_height(scroll_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for example in crate::proxy::rule_io::import_examples() {
                ui.add_space(8.0);
                // 每种格式一张卡：卡片用页面那档蓝灰，弹框本身是纯白容器，
                // 卡片内层的代码块再回到纯白 —— 三层靠深浅分，不靠描边堆。
                Frame::NONE
                    .fill(theme::page_bg())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::symmetric(13, 11))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(example.title)
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.add_space(6.0);
                            let (badge, color) = if example.expected_rules > 0 {
                                (
                                    format!("可导入 {} 条", example.expected_rules),
                                    theme::accent_strong(),
                                )
                            } else {
                                (
                                    format!("{} 条全部点名跳过", example.expected_unsupported),
                                    theme::warning(),
                                )
                            };
                            status_badge(ui, &badge, color);
                        });
                        ui.label(RichText::new(example.note).small().color(theme::muted()));
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border()))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(10, 8))
                            .show(ui, |ui| {
                                for line in example.body.lines() {
                                    ui.label(
                                        RichText::new(line)
                                            .small()
                                            .monospace()
                                            .color(theme::text_secondary()),
                                    );
                                }
                            });
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("填入导入框")
                                .on_hover_text("整段塞进「导入规则」，先给解析预览再让导入")
                                .clicked()
                            {
                                set_help_msg(&mut msg, RuleHelpMsg::Fill(example.body));
                            }
                            if ui.button("复制这段").clicked() {
                                let outcome =
                                    arboard::Clipboard::new().and_then(|mut clipboard| {
                                        clipboard.set_text(example.body.to_string())
                                    });
                                set_help_msg(
                                    &mut msg,
                                    RuleHelpMsg::Status(match outcome {
                                        Ok(()) => "示范已复制到剪贴板".to_string(),
                                        Err(error) => format!("复制失败：{error}"),
                                    }),
                                );
                            }
                        });
                    });
            }
        });
    ui.add_space(10.0);
    if ui.button("关闭").clicked() {
        set_help_msg(&mut msg, RuleHelpMsg::Close);
    }
    msg
}

#[cfg(test)]
mod rule_import_help_probe_tests {
    use super::ui_rule_import_help_window;
    use eframe::egui::{self, RawInput, Rect, Vec2};

    /// 用产品同一份构造函数真开三帧「导入格式示范」窗口：
    /// 第 1 帧布局、第 2 帧读"看得见的那部分"、第 3 帧再读一次（确认滚动后仍在渲染）。
    /// 返回（窗口矩形, 第 2 帧文字, 第 3 帧文字）。
    fn probe(screen_width: f32) -> (Rect, String, String) {
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let mut top = String::new();
        let mut bottom = String::new();
        for frame in 0..3 {
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(screen_width, 900.0),
            ));
            if frame == 2 {
                // 光标停在窗口里往下滚，够滚到底。
                let center = rect.center();
                input.events.push(egui::Event::PointerMoved(center));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -2000.0),
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut this_frame = String::new();
            let _ = ctx.run(input, |ctx| {
                let (_msg, window_rect) = ui_rule_import_help_window(ctx);
                rect = window_rect;
                // 形状只能在同一帧里读：这一画在下一帧开始前就被 egui 清掉了。
                let ids: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
                this_frame = ctx.graphics_mut(|g| {
                    let mut out = String::new();
                    for id in &ids {
                        let Some(list) = g.get(*id) else {
                            continue;
                        };
                        for shape in list.all_entries() {
                            if let egui::Shape::Text(text) = &shape.shape {
                                out.push_str(&text.galley.text());
                                out.push('\n');
                            }
                        }
                    }
                    out
                });
            });
            match frame {
                1 => top = this_frame,
                2 => bottom = this_frame,
                _ => {}
            }
        }
        (rect, top, bottom)
    }

    /// 示范弹框必须：整窗留在屏幕里（字体调到 140% 时逻辑宽度只剩物理的 0.71 倍）、
    /// 类型清单与"打开就该看见"的前三段示范真的画出来、两个动作按钮也在。
    /// 断言读的是这一帧实际提交的文字，渲染没跑就等于红；
    /// 六段示范内容的正确性由 rule_io 里"逐段喂回解析器"的测试钉住。
    #[test]
    fn the_help_window_fits_and_paints_the_visible_examples() {
        let examples = crate::proxy::rule_io::import_examples();
        assert_eq!(examples.len(), 6, "示范段落数变了，弹框的说明文案要跟着改");
        for screen_width in [1298.0, 940.0, 660.0, 480.0] {
            let (rect, top, bottom) = probe(screen_width);
            assert!(
                rect.max.x <= screen_width + 0.5,
                "{screen_width:.0} 宽屏面上窗口画到 {:.1}，超出右边界",
                rect.max.x
            );
            assert!(
                rect.height() > 200.0,
                "窗口只有 {:.1} 高，示范没渲染",
                rect.height()
            );
            for (name, text) in [("未滚动", &top), ("滚动后", &bottom)] {
                assert!(
                    text.len() > 200,
                    "{screen_width:.0} 宽时{name}那一帧只画出 {} 字",
                    text.len()
                );
            }
            let painted = format!("{top}\n{bottom}");
            assert!(
                painted.contains("认这些类型") && painted.contains("点名跳过"),
                "{screen_width:.0} 宽时类型清单没画出来"
            );
            // 卡片比原来的裸文本各高 ~30pt：620 逻辑宽以上能不滚动看见前三段，
            // 更窄（960 物理 ÷ 150% 缩放 ÷ 140% 字体）只保证前两段，剩下的靠滚动。
            let visible = if screen_width >= 620.0 { 3 } else { 2 };
            for example in &examples[..visible] {
                assert!(
                    painted.contains(example.title),
                    "{screen_width:.0} 宽时示范「{}」没画出来",
                    example.title
                );
            }
            assert!(
                painted.contains("填入导入框") && painted.contains("复制这段"),
                "{screen_width:.0} 宽时示范的操作按钮没画出来"
            );
        }
    }
}

#[cfg(test)]
mod rule_hits_height_probe_tests {
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻「命中记录」那一列的结构，量滚动区实际拿到的视口高度。
    /// `strip_note` = 把"从本进程启动开始统计…"那行并掉；`reserve` = 视口预留量。
    fn probe(window: Vec2, strip_note: bool, reserve: f32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, window));
        let mut viewport = 0.0_f32;
        let mut card_bottom = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::TopBottomPanel::bottom("status")
                .exact_height(32.0)
                .frame(Frame::NONE.fill(egui::Color32::GRAY))
                .show(ctx, |ui| {
                    let _ = ui.label("状态栏");
                });
            egui::TopBottomPanel::bottom("control")
                .exact_height(48.0)
                .frame(Frame::NONE.fill(egui::Color32::DARK_GRAY))
                .show(ctx, |ui| {
                    let _ = ui.button("启动代理");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    let _ = ui.horizontal(|ui| {
                        let _ = ui.button("规则配置");
                        let _ = ui.button("命中记录");
                    });
                    let _ = ui.separator();
                    ui.add_space(6.0);
                    let card = Frame::NONE
                        .fill(egui::Color32::WHITE)
                        .inner_margin(Margin::same(22));
                    let shown = card.show(ui, |ui| {
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("规则命中记录");
                            let _ = ui.label("123 条连接 · 兜底 MATCH 20 条（16%）");
                            let _ = ui.button("立即刷新");
                        });
                        ui.add_space(2.0);
                        if !strip_note {
                            ui.label("从本进程启动开始统计；占比按连接数算，一条连接只计一次。");
                            ui.add_space(8.0);
                        }
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("命中的规则");
                            let _ = ui.label("派给");
                            let _ = ui.label("条数 / 占比");
                        });
                        let area = egui::ScrollArea::vertical()
                            .id_salt("rule_hits_table_probe")
                            .auto_shrink([false, false])
                            .max_height(ui.available_height() - reserve)
                            .show(ui, |ui| {
                                for _ in 0..40 {
                                    let _ = ui.horizontal(|ui| {
                                        let _ = ui.label("GEOIP,CN,DIRECT");
                                        let _ = ui.label("→ 代理");
                                        let _ = ui.label("12 条 · 9%");
                                    });
                                }
                            });
                        viewport = area.inner_rect.height();
                    });
                    card_bottom = shown.response.rect.max.y;
                });
        });
        (viewport, card_bottom)
    }

    /// 命中记录列表要把剩余高度全吃掉：说明并进表头 + 取消 40pt 预留之后，
    /// 1330×800 的视口从 469（约 19 行）涨到 ≥520（约 22 行），
    /// 140% 字体（950×571 逻辑）从 240（10 行）涨到 ≥290（12 行）。
    /// 同一份结构按老写法（独立说明行 + 预留 40）再量一次，那两行确实是白送的；
    /// 顺手钉住"卡片底不许越过中央面板底"，加高不能靠溢出实现。
    #[test]
    fn the_hits_list_uses_every_remaining_pixel() {
        for (size, min_new) in [
            (Vec2::new(1330.0, 800.0), 520.0),
            (Vec2::new(950.0, 571.0), 290.0),
        ] {
            let (new_vp, new_bottom) = probe(size, true, 0.0);
            let (old_vp, _old_bottom) = probe(size, false, 40.0);
            assert!(
                new_vp >= min_new,
                "{size:?} 时列表视口只有 {new_vp:.1}，没到 {min_new:.0}"
            );
            assert!(
                new_vp > old_vp + 50.0,
                "新写法视口 {new_vp:.1} 只比老写法 {old_vp:.1} 高一点点，说明没省下来"
            );
            // 中央面板底边 = 屏高 - 状态栏 32 - 控制栏 48，再收掉 14 的下内边距。
            let limit = size.y - 32.0 - 48.0 - 14.0;
            assert!(
                new_bottom <= limit + 1.0,
                "{size:?} 时卡片底画到 {new_bottom:.1}，越过中央面板底 {limit:.1}"
            );
        }
    }
}

#[cfg(test)]
mod rules_row_probe_tests {
    use super::{RULES_ACTIONS_WIDTH, RULES_TABS_MIN_WIDTH};
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻分流规则那行表头**含现写的分支判断**：宽到放得下就一行钉右，
    /// 放不下就让动作组另起一行。返回（面板矩形, 分区标签占的矩形, 动作组占的矩形）。
    fn probe(window_width: f32) -> (Rect, Rect, Rect) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 800.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut tabs = Rect::NOTHING;
        let mut group = Rect::NOTHING;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    panel = ui.max_rect();
                    let mut draw_tabs = |ui: &mut egui::Ui| {
                        let a = ui.button("规则配置").rect;
                        let b = ui.button("命中记录").rect;
                        let added = a.union(b);
                        tabs = if !tabs.is_finite() {
                            added
                        } else {
                            tabs.union(added)
                        };
                    };
                    let mut draw_actions = |ui: &mut egui::Ui| {
                        // 只并按钮自己的矩形：min_rect 会把同一行里先画的东西也算进来。
                        let a = ui.button("导出 JSON").rect;
                        let c = ui.button("导入规则").rect;
                        let added = a.union(c);
                        group = if !group.is_finite() {
                            added
                        } else {
                            group.union(added)
                        };
                    };
                    // 与产品同一个判据（ui.max_rect() 就是中央面板的内容宽）。
                    if ui.max_rect().width() >= RULES_ACTIONS_WIDTH + RULES_TABS_MIN_WIDTH + 8.0 {
                        crate::gui::widgets::split_row(
                            ui,
                            "rules:tabs",
                            RULES_ACTIONS_WIDTH,
                            |ui| draw_tabs(ui),
                            |ui| draw_actions(ui),
                        );
                    } else {
                        let _ = ui.horizontal(|ui| draw_tabs(ui));
                        let _ = ui.horizontal_wrapped(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| draw_actions(ui),
                            );
                        });
                    }
                });
        });
        (panel, tabs, group)
    }

    /// 老写法的毛病：右对齐那组和左边的分区标签在窄面板上**互相压住**（x 方向重叠），
    /// 按钮看起来就是被挤走。反向验证：同一个宽度下老写法确实重叠、新写法不重叠。
    #[test]
    fn the_old_single_row_really_collided() {
        let (panel, tabs, group) = probe_single_row(520.0);
        assert!(
            group.min.x < tabs.max.x,
            "老写法在 520 宽没重叠（组从 {:.1} 开始，标签到 {:.1}）—— 反向验证失去意义",
            group.min.x,
            tabs.max.x
        );
        assert!(panel.width() > 0.0);
    }

    fn probe_single_row(window_width: f32) -> (Rect, Rect, Rect) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 800.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut tabs = Rect::NOTHING;
        let mut group = Rect::NOTHING;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    panel = ui.max_rect();
                    let _ = ui.horizontal(|ui| {
                        let a = ui.button("规则配置").rect;
                        let b = ui.button("命中记录").rect;
                        tabs = a.union(b);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let before = ui.min_rect();
                            let _ = ui.button("导出 JSON");
                            let _ = ui.button("导入规则");
                            group = before.union(ui.min_rect());
                        });
                    });
                });
        });
        (panel, tabs, group)
    }

    /// 动作组不许和分区标签互相压住，也不许越过面板右缘。
    /// 下限取 460 = 960 物理最小宽 ÷（150% 系统缩放 × 140% 字体）；再窄（200% 缩放叠 140%
    /// 字体 = 380 逻辑点）egui 自己的右锚点会溢出 4.1pt，那已经不是这套布局能救的了。
    #[test]
    fn the_actions_never_collide_or_leave_the_panel() {
        for width in [1330.0, 950.0, 760.0, 686.0, 620.0, 520.0, 460.0] {
            let (panel, tabs, group) = probe(width);
            assert!(
                group.max.x <= panel.right() + 0.5,
                "{width:.0} 宽时动作组画到 {:.1}，越过面板右缘 {:.1}",
                group.max.x,
                panel.right()
            );
            assert!(
                tabs.min.x >= panel.left() - 0.5 && tabs.max.x <= panel.right() + 0.5,
                "{width:.0} 宽时分区标签画到 [{:.1}, {:.1}]，超出面板 [{:.1}, {:.1}]",
                tabs.min.x,
                tabs.max.x,
                panel.left(),
                panel.right()
            );
            assert!(
                !group.intersects(tabs),
                "{width:.0} 宽时动作组 [{:.1},{:.1}] 压住分区标签 [{:.1},{:.1}]",
                group.min.x,
                group.max.x,
                tabs.min.x,
                tabs.max.x
            );
        }
    }
}
#[cfg(test)]
mod status_button_style_tests {
    use super::status_btn;
    use crate::gui::widgets::{danger_btn, primary_btn};
    use eframe::egui::{self, CentralPanel, Color32, CornerRadius, RawInput, Rect, Vec2};

    /// 照状态栏那一排的写法真画一遍，读这一帧画出来的按钮矩形。
    fn painted() -> Vec<(CornerRadius, f32, Color32)> {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1330.0, 200.0),
        ));
        let mut out = Vec::new();
        let _ = ctx.run(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(11.0, 3.0);
                let layer = ui.layer_id();
                ui.horizontal(|ui| {
                    let _ = ui.add(primary_btn("启动代理"));
                    let _ = ui.add(danger_btn("停止代理"));
                    let _ = ui.add(status_btn("IP 检测"));
                    let _ = ui.add(status_btn("系统代理"));
                    let _ = ui.add(status_btn("测速全部"));
                });
                out = ctx.graphics_mut(|g| {
                    g.get(layer)
                        .map(|list| {
                            list.all_entries()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Rect(rect)
                                        if (16.0..=40.0).contains(&rect.rect.height()) =>
                                    {
                                        Some((rect.corner_radius, rect.rect.height(), rect.fill))
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

    /// 一排五个按钮：圆角和高度必须完全一致（这是他说"样式要一样"时看到的东西），
    /// 普通那三个的填充也必须一致；主/危险按钮允许只有颜色不同。
    #[test]
    fn the_status_row_buttons_share_one_shape() {
        let rects = painted();
        assert!(
            rects.len() >= 5,
            "五个按钮没都画出来，只读到 {}",
            rects.len()
        );
        let radius = rects[0].0;
        let height = rects[0].1;
        for (index, (got_radius, got_height, _)) in rects.iter().enumerate() {
            assert_eq!(
                *got_radius, radius,
                "第 {index} 个按钮圆角 {got_radius:?} 和同排 {radius:?} 不一致"
            );
            assert!(
                (got_height - height).abs() < 0.6,
                "第 {index} 个按钮高 {got_height:.1}，同排是 {height:.1}"
            );
        }
        let neutral: Vec<Color32> = rects.iter().skip(2).map(|(_, _, fill)| *fill).collect();
        assert!(
            neutral.windows(2).all(|pair| pair[0] == pair[1]),
            "普通按钮的填充色不一致：{neutral:?}"
        );
        assert_ne!(
            rects[0].2, rects[2].2,
            "主按钮和普通按钮同色了 —— 那这排就没人看得出哪个是主操作"
        );
    }
}

#[cfg(test)]
mod policy_group_filter_tests {
    use super::node_matches_group;
    use crate::gui::types::KernelGroup;
    use crate::models::NodeSummary;

    fn sample_node(name: &str, country: Option<&str>, server: &str) -> NodeSummary {
        NodeSummary {
            id: name.to_string(),
            source_id: Some("src1".to_string()),
            protocol: "vless".to_string(),
            name: name.to_string(),
            server: server.to_string(),
            port: 443,
            country: country.map(|c| c.to_string()),
            latency_ms: Some(120),
            latency_ok: Some(true),
            download_kbps: Some(5000.0),
            latency_kernel: Some("mihomo".to_string()),
            latency_test_kind: Some("google_proxy".to_string()),
            latency_tested_at: None,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_all_group_matches_everything() {
        let us = sample_node("US-Node-01", Some("US"), "1.1.1.1");
        let hk = sample_node("HK-Node-01", Some("HK"), "2.2.2.2");
        assert!(node_matches_group(&us, "全部", &[]));
        assert!(node_matches_group(&hk, "全部", &[]));
        assert!(node_matches_group(&us, "PROXY", &[]));
        assert!(node_matches_group(&hk, "PROXY", &[]));
    }

    #[test]
    fn test_us_group_matches_only_us() {
        let us = sample_node("洛杉矶精品专线", Some("US"), "1.1.1.1");
        let hk = sample_node("香港高速直连", Some("HK"), "2.2.2.2");
        let jp = sample_node("东京软银", Some("JP"), "3.3.3.3");
        assert!(node_matches_group(&us, "美国节点", &[]));
        assert!(!node_matches_group(&hk, "美国节点", &[]));
        assert!(!node_matches_group(&jp, "美国节点", &[]));
    }

    #[test]
    fn test_ai_group_matches_ai_regions_and_keywords() {
        let us = sample_node("美国节点", Some("US"), "1.1.1.1");
        let jp = sample_node("日本东京", Some("JP"), "2.2.2.2");
        let sg = sample_node("新加坡", Some("SG"), "3.3.3.3");
        let custom_ai = sample_node("OpenAI 专线解锁", None, "4.4.4.4");
        let cn = sample_node("国内直连", Some("CN"), "5.5.5.5");

        assert!(node_matches_group(&us, "AI分组", &[]));
        assert!(node_matches_group(&jp, "AI分组", &[]));
        assert!(node_matches_group(&sg, "AI分组", &[]));
        assert!(node_matches_group(&custom_ai, "AI分组", &[]));
        assert!(!node_matches_group(&cn, "AI分组", &[]));
    }

    #[test]
    fn test_kernel_groups_members_matching_and_cycle_guard() {
        let n1 = sample_node("专线节点A", None, "1.1.1.1");
        let n2 = sample_node("专线节点B", None, "2.2.2.2");

        let kg = vec![
            KernelGroup {
                name: "自定义分组".to_string(),
                kind: "select".to_string(),
                now: Some("专线节点A".to_string()),
                members: vec!["专线节点A".to_string()],
            },
            // 环形依赖测试
            KernelGroup {
                name: "GroupLoopA".to_string(),
                kind: "select".to_string(),
                now: None,
                members: vec!["GroupLoopB".to_string()],
            },
            KernelGroup {
                name: "GroupLoopB".to_string(),
                kind: "select".to_string(),
                now: None,
                members: vec!["GroupLoopA".to_string()],
            },
        ];

        assert!(node_matches_group(&n1, "自定义分组", &kg));
        assert!(!node_matches_group(&n2, "自定义分组", &kg));

        // 环形嵌套不应陷入无限循环或栈溢出
        assert!(!node_matches_group(&n1, "GroupLoopA", &kg));
    }

    fn kernel_group(name: &str, kind: &str, members: &[&str]) -> KernelGroup {
        KernelGroup {
            name: name.to_string(),
            kind: kind.to_string(),
            now: None,
            members: members.iter().map(|member| member.to_string()).collect(),
        }
    }

    #[test]
    fn test_the_pill_bar_defaults_to_all_only() {
        use super::visible_group_names;

        // 什么都没配：整条胶囊只有「全部」。
        assert_eq!(visible_group_names(false, None, &[]), vec!["全部".to_string()]);

        // 反向判据：内置的「普通代理」按节点匹配能命中每一个节点，
        // 但配置里没有它就不该出现 —— 否则这条改动等于没做。
        let any_node = sample_node("任意节点", Some("HK"), "1.2.3.4");
        assert!(node_matches_group(&any_node, "普通代理", &[]));
        assert!(!visible_group_names(false, None, &[])
            .iter()
            .any(|name| name == "普通代理"));

        // 打开内置智能策略组 ⇒ 四组都出现。
        let smart = visible_group_names(true, None, &[]);
        assert_eq!(smart.len(), 5);
        for name in ["AI分组", "美国节点", "多线并发下载", "普通代理"] {
            assert!(smart.iter().any(|shown| shown == name), "{name} 该在");
        }

        // 内核上报的组：成员非空才算数，保留名不算组。
        let kernel = vec![
            kernel_group("我的分组", "url-test", &["a"]),
            kernel_group("空分组", "select", &[]),
            kernel_group("GLOBAL", "select", &["a"]),
        ];
        let shown = visible_group_names(false, None, &kernel);
        assert!(shown.iter().any(|name| name == "我的分组"));
        assert!(!shown.iter().any(|name| name == "空分组"));
        assert!(!shown.iter().any(|name| name == "GLOBAL"));

        // 内核真上报了内置组的名字（比如覆写里自己定义的），关着 smart_routing 也要显示。
        let reported = vec![kernel_group("普通代理", "select", &["a"])];
        assert!(visible_group_names(false, None, &reported)
            .iter()
            .any(|name| name == "普通代理"));
    }

    #[test]
    fn test_override_yaml_groups_count_as_configured() {
        use super::{override_group_names, visible_group_names};

        let document = "proxy-groups:\n  - name: 我的分组\n    type: select\n    proxies: [DIRECT]\n";
        assert_eq!(
            override_group_names(Some(document)),
            vec!["我的分组".to_string()]
        );
        // 覆写也允许直接给一个组数组（与 proxy 侧应用覆写时的分叉一致）。
        let bare = "- name: 裸数组分组\n  type: url-test\n";
        assert_eq!(override_group_names(Some(bare)), vec!["裸数组分组".to_string()]);
        assert!(override_group_names(None).is_empty());
        assert!(override_group_names(Some("   ")).is_empty());
        // 坏 YAML 不许带崩界面：按"没有覆写组"处理。
        assert!(override_group_names(Some("proxy-groups: [:::")).is_empty());

        assert!(visible_group_names(false, Some(document), &[])
            .iter()
            .any(|name| name == "我的分组"));
    }
}

#[cfg(test)]
mod clean_kernel_version_tests {
    use super::clean_kernel_version;

    #[test]
    fn test_extracts_semver_from_complex_kernel_banners() {
        assert_eq!(
            clean_kernel_version(
                "Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 16:55:17 UTC 2026"
            ),
            "v1.19.32"
        );
        assert_eq!(
            clean_kernel_version("sing-box version 1.14.3 (go1.22.0 windows/amd64)"),
            "1.14.3"
        );
        assert_eq!(
            clean_kernel_version("Xray 26.3.27 (Xray, Penetrates Everything.) OpenSource"),
            "26.3.27"
        );
        assert_eq!(clean_kernel_version("v1.19.32"), "v1.19.32");
        assert_eq!(clean_kernel_version("1.14.3"), "1.14.3");
        assert_eq!(clean_kernel_version("未检测到"), "未检测到");
        assert_eq!(clean_kernel_version("-"), "-");
        assert_eq!(clean_kernel_version(""), "");
    }
}

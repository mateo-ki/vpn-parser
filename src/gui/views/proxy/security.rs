use super::groups::{node_matches_group, override_group_names, visible_group_names};
use super::CLASH_VERGE_RECOMMENDED_OVERRIDE;
use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    ActiveConnectionInfo, ConnectionsTab, DnsRecord, GroupsTab, KernelGroup, ProxyPanel, ProxySortBy,
    ProxyView, ProxyViewRow, RuleDraftKind, RulesTab,
};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, country_flag_and_name, danger_btn, dns_mode_label,
    format_bytes, format_speed, header_cell, macos_segmented_tabs, panel_frame, panel_tab, primary_btn,
    protocol_pill, render_country_code_badge, render_health_strip, render_node_health_bar,
    render_protocol_donut_chart_sized, render_signal_bars, render_speedometer, render_top_processes_card,
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

impl VpnApp {
    pub(crate) fn ui_proxy_security(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("隐私安全"), "uTLS 指纹伪装 · WebRTC 防泄漏 · 隐私追踪拦截 · 端口跳跃");

        let mut changed = false;

        egui::ScrollArea::vertical()
            .id_salt("proxy_security_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
                    RichText::new("在内核层全面拦截 WebRTC 专用的 STUN / TURN 穿透端口（标准端口 3478-3479、TLS 端口 5349-5350、Google 专有段 19302-19309）以及所有 STUN/TURN/WebRTC 探测域名，阻断恶意网站通过 JavaScript 探针探测并获取访客本地真实物理公网 IP。")
                        .small()
                        .color(theme::text_secondary()),
                );

                if self.proxy_config.block_webrtc {
                    ui.add_space(6.0);
                    if self.proxy_config.mode == ProxyMode::Tun {
                        Frame::NONE
                            .fill(theme::accent().gamma_multiply(0.08))
                            .stroke(Stroke::new(1.0, theme::accent().gamma_multiply(0.3)))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(Margin::symmetric(10, 6))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Check, 13.0, theme::accent());
                                    ui.label(
                                        RichText::new("已处于「TUN 虚拟网卡模式」：代理内核已全面接管系统级 UDP/TCP 流量，所有 WebRTC STUN 穿透握手包在网卡层被强制丢弃，真实 IP 0 泄露。")
                                            .small()
                                            .color(theme::accent()),
                                    );
                                });
                            });
                    } else {
                        Frame::NONE
                            .fill(theme::warning().gamma_multiply(0.09))
                            .stroke(Stroke::new(1.0, theme::warning().gamma_multiply(0.38)))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        icon_label(ui, Icon::Alert, 14.0, theme::warning());
                                        ui.label(
                                            RichText::new("⚠️ 当前处于「系统代理」模式：浏览器 WebRTC 会绕过 HTTP 代理！")
                                                .strong()
                                                .small()
                                                .color(theme::warning()),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("⚡ 立即切换为 TUN 模式")
                                                            .small()
                                                            .strong()
                                                            .color(theme::accent_strong()),
                                                    )
                                                    .fill(theme::accent_bg())
                                                    .stroke(Stroke::new(1.0, theme::accent().gamma_multiply(0.4)))
                                                    .corner_radius(CornerRadius::same(5)),
                                                )
                                                .on_hover_text("点击切换为 TUN 虚拟网卡模式，全面接管系统 UDP 流量以 100% 阻断 WebRTC 泄露")
                                                .clicked()
                                            {
                                                self.proxy_config.mode = ProxyMode::Tun;
                                                changed = true;
                                                self.status = "已切换为 TUN 虚拟网卡模式，若代理已启动请重启代理使 TUN 生效".to_string();
                                            }
                                        });
                                    });
                                    ui.add_space(3.0);
                                    ui.label(
                                        RichText::new("原理：系统代理仅拦截标准 TCP HTTP 流量。浏览器的 WebRTC 探针为探测 P2P 直连，默认会直接通过物理网卡发送原始 UDP 握手包，绕过系统代理造成真实公网 IP 泄露。\n建议：一键切换至「TUN 虚拟网卡模式」以获得 100% 彻底防护；或在 Chrome/Edge 打开 chrome://flags/#webrtc-ip-handling-policy 设为 Disable non-proxied UDP。")
                                            .small()
                                            .color(theme::text_secondary()),
                                    );
                                });
                            });
                    }
                }
            });

            ui.add_space(8.0);

            // -------------------------------------------------------------
            // 卡片 3：DNS 泄露物理阻断与分流防护 (Anti-DNS Leak)
            // -------------------------------------------------------------
            panel_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Globe, 16.0, theme::accent());
                    ui.label(
                        RichText::new("DNS 真实泄露防御 (Anti-DNS Leak)")
                            .strong()
                            .size(14.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (mode_badge, mode_color) = if self.proxy_config.mode == ProxyMode::Tun {
                            ("● TUN 全局接管保护中", theme::accent())
                        } else {
                            ("⚠️ 系统代理模式（存在旁路风险）", theme::warning())
                        };
                        ui.label(RichText::new(mode_badge).small().color(mode_color));
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("智能分流国内/海外 DNS 解析策略（国内直连域名走阿里/腾讯高保真解析，海外代理域名走 Cloudflare/Google DoH 并由出海节点远程加密发出），开启 respect-rules 防泄漏引擎，彻底消除因境内 DNS 代查海外域名导致的广州/境内归属地暴露。")
                        .small()
                        .color(theme::text_secondary()),
                );

                if self.proxy_config.mode != ProxyMode::Tun {
                    ui.add_space(6.0);
                    Frame::NONE
                        .fill(theme::warning().gamma_multiply(0.09))
                        .stroke(Stroke::new(1.0, theme::warning().gamma_multiply(0.38)))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Alert, 13.0, theme::warning());
                                ui.label(
                                    RichText::new("提示：系统代理模式下，Windows 系统底层 DNS 解析与浏览器预取仍可能经本地物理网卡发送至运营商 DNS。推荐使用「TUN 虚拟网卡模式」全局劫持 DNS 请求。")
                                        .small()
                                        .color(theme::warning()),
                                );
                            });
                        });
                }
            });

            ui.add_space(8.0);

            // -------------------------------------------------------------
            // 卡片 4：隐私与行为追踪广告过滤 (Tracker & Telemetry Blocking)
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
            // 卡片 5：UDP 端口跳跃与防 QoS 丢包混淆 (UDP Port Hopping & Anti-QoS)
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

}

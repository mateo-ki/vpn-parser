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
    pub(crate) fn ui_proxy_network(&mut self, ui: &mut Ui) {
        section_title(ui, "DNS / TUN", "配置接管模式、监听端口与 DNS");
        let editable = !self.proxy_busy && self.proxy_status.config_editable();
        let mut save_clicked = false;

        egui::ScrollArea::vertical()
            .id_salt("proxy_network_dns_tun_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                card_frame().show(ui, |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Plug, 14.0, theme::accent());
                            ui.label(RichText::new("接管模式架构").strong().size(14.0).color(theme::text_primary()));
                        });
                        ui.add_space(6.0);

                        let is_narrow_net = ui.available_width() < 550.0;
                        if is_narrow_net {
                            self.render_system_proxy_card(ui);
                            ui.add_space(8.0);
                            self.render_tun_proxy_card(ui);
                        } else {
                            ui.columns(2, |column_uis| {
                                self.render_system_proxy_card(&mut column_uis[0]);
                                self.render_tun_proxy_card(&mut column_uis[1]);
                            });
                        }

                        #[cfg(target_os = "linux")]
                        if self.proxy_config.mode == ProxyMode::System && !crate::system_proxy::system_proxy_supported_on_this_platform() {
                            ui.add_space(8.0);
                            self.render_linux_proxy_env_guide(ui);
                        }

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
                                    KernelPreference::Shoes => "Shoes",
                                    KernelPreference::Meow => "Meow",
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
                                    ui.selectable_value(
                                        &mut self.proxy_config.kernel_preference,
                                        KernelPreference::Shoes,
                                        "Shoes",
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
            .id_salt("proxy_kernels_scroll")
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
                                KernelPreference::Shoes => kernel.kind == KernelKind::Shoes,
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
                                        KernelKind::Shoes => (Icon::Shield, theme::accent()),
                                        KernelKind::Meow => (Icon::Lightning, theme::accent()),
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
                                    KernelKind::Shoes => "Snell · NaiveProxy · AnyTLS · Clash API",
                                    KernelKind::Meow => "Rust 版 Mihomo · 高性能 · Clash API",
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

    pub(crate) fn render_system_proxy_card(&mut self, ui: &mut Ui) {
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

                #[cfg(target_os = "linux")]
                {
                    ui.add_space(4.0);
                    let has_gsettings = crate::system_proxy::system_proxy_supported_on_this_platform();
                    if has_gsettings {
                        ui.label(RichText::new("✓ GNOME gsettings 环境就绪").small().color(theme::accent_strong()));
                    } else {
                        ui.label(RichText::new("ℹ️ 非 GNOME 环境 (无 gsettings 全局代理)").small().color(Color32::from_rgb(234, 179, 8)));
                    }
                }
            });
        if card1.response.interact(Sense::click()).clicked() {
            self.proxy_config.mode = ProxyMode::System;
        }
    }

    pub(crate) fn render_tun_proxy_card(&mut self, ui: &mut Ui) {
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

                #[cfg(target_os = "linux")]
                {
                    ui.add_space(4.0);
                    let kernel_path = crate::kernel::managed_kernel_path(crate::kernel::KernelKind::Mihomo);
                    let has_cap = crate::platform::kernel_has_tun_capability(&kernel_path);
                    if has_cap {
                        ui.label(RichText::new("✓ Linux CAP_NET_ADMIN 权限就绪").small().color(theme::accent_strong()));
                    } else {
                        let cmd = crate::platform::tun_capability_command(&kernel_path);
                        let copy_btn = ui.button(RichText::new("⚠️ 缺少 CAP_NET_ADMIN (点击复制授权命令)").small().color(Color32::from_rgb(234, 179, 8)));
                        if copy_btn.on_hover_text(format!("点击复制到剪贴板并在终端执行：\n{cmd}")).clicked() {
                            ui.output_mut(|o| o.copied_text = cmd);
                            self.status = "已复制 Linux 内核授权命令，请在终端执行后启动 TUN".into();
                        }
                    }
                }
            });
        if card2.response.interact(Sense::click()).clicked() {
            self.proxy_config.mode = ProxyMode::Tun;
        }
    }

    #[allow(dead_code)]
    pub(crate) fn render_linux_proxy_env_guide(&mut self, ui: &mut Ui) {
        let port = self.proxy_config.mixed_port;
        let export_cmd = format!(
            "export http_proxy=\"http://127.0.0.1:{port}\"\nexport https_proxy=\"http://127.0.0.1:{port}\"\nexport all_proxy=\"socks5://127.0.0.1:{port}\"\nexport no_proxy=\"localhost,127.0.0.1,::1\""
        );
        let unset_cmd = "unset http_proxy https_proxy all_proxy no_proxy".to_string();

        Frame::NONE
            .fill(theme::sidebar())
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(234, 179, 8).linear_multiply(0.6)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Lightbulb, 15.0, Color32::from_rgb(234, 179, 8));
                    ui.label(
                        RichText::new("Linux 平铺/非 GNOME 桌面环境代理指引")
                            .strong()
                            .color(theme::text_primary()),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "检测到当前环境缺少 gsettings 工具（如 niri / Sway / Hyprland / KDE 等），无法自动写入全局代理。\n内核已在 127.0.0.1:{port} 侦听混合代理（HTTP / SOCKS5）。您可以通过以下方式接入："
                    ))
                    .small()
                    .color(theme::text_secondary()),
                );
                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    let btn_export = ui.button(
                        RichText::new("📋 复制终端代理环境变量 (export ...)")
                            .small()
                            .strong()
                            .color(theme::accent_strong()),
                    );
                    if btn_export
                        .on_hover_text(format!("点击复制到剪贴板并在终端执行：\n{export_cmd}\n\n也可写入 ~/.bashrc 或 ~/.zshrc 永久生效"))
                        .clicked()
                    {
                        ui.output_mut(|o| o.copied_text = export_cmd.clone());
                        self.status = format!("已复制终端代理命令 (端口 {port})");
                    }

                    ui.add_space(6.0);
                    let btn_unset = ui.button(
                        RichText::new("📋 复制取消代理命令 (unset ...)")
                            .small()
                            .color(theme::muted()),
                    );
                    if btn_unset
                        .on_hover_text("点击复制：\nunset http_proxy https_proxy all_proxy no_proxy")
                        .clicked()
                    {
                        ui.output_mut(|o| o.copied_text = unset_cmd.clone());
                        self.status = "已复制取消代理环境变量命令".into();
                    }
                });

                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    icon_label(ui, Icon::Info, 12.0, theme::muted());
                    ui.label(
                        RichText::new(format!(
                            "浏览器应用可在设置或 SwitchyOmega 插件中配置 SOCKS5 代理：127.0.0.1:{port}；若需要系统全局零配置透明代理，请直接选用上方的「TUN 虚拟网卡」模式。"
                        ))
                        .small()
                        .color(theme::muted()),
                    );
                });
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
                .id_salt("proxy_logs_scroll")
                .auto_shrink([false, false])
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

}

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

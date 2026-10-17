use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{BgMsg, SettingsTab};
use crate::gui::widgets::{
    card_frame, normalize_api_base, panel_tab, primary_btn, status_badge,
};

/// 系统设置表头右侧动作组（保存设置 + 重置修改 + 状态）预留宽度。
const SETTINGS_ACTIONS_WIDTH: f32 = 300.0;
use crate::gui::VpnApp;
use crate::proxy::ProxyState;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};

impl VpnApp {
    pub(crate) fn ui_settings(&mut self, ui: &mut Ui) {
        let mut tab = self.settings_tab;
        let mut save_clicked = false;
        let mut reset_clicked = false;
        let dirty = self.settings_dirty();
        let saving = self.settings_saving;
        let tr_general = self.tr("通用");
        let tr_appearance = self.tr("外观");
        let tr_automation = self.tr("订阅与测速");
        let tr_network = self.tr("自愈网络");
        let tr_api = self.tr("外部接口");
        let tr_save = self.tr("保存设置");
        let tr_reset = self.tr("重置修改");
        let tr_synced = self.tr("已同步");

        crate::gui::widgets::split_row(
            ui,
            "settings:header",
            SETTINGS_ACTIONS_WIDTH,
            |ui| {
                panel_tab(
                    ui,
                    &mut tab,
                    SettingsTab::General,
                    Icon::Gear,
                    tr_general,
                );
                panel_tab(
                    ui,
                    &mut tab,
                    SettingsTab::Appearance,
                    Icon::Appearance,
                    tr_appearance,
                );
                panel_tab(
                    ui,
                    &mut tab,
                    SettingsTab::Automation,
                    Icon::Lightning,
                    tr_automation,
                );
                panel_tab(
                    ui,
                    &mut tab,
                    SettingsTab::Network,
                    Icon::Shield,
                    tr_network,
                );
                panel_tab(
                    ui,
                    &mut tab,
                    SettingsTab::Api,
                    Icon::Plug,
                    tr_api,
                );
            },
            |ui| {
                if ui
                    .add_enabled(!saving, primary_btn(tr_save))
                    .clicked()
                {
                    save_clicked = true;
                }
                if ui
                    .button(tr_reset)
                    .on_hover_text("恢复为当前已保存的配置")
                    .clicked()
                {
                    reset_clicked = true;
                }
                if saving {
                    ui.spinner();
                    ui.label(RichText::new("正在保存...").color(theme::accent()).small());
                } else if dirty {
                    ui.label(
                        RichText::new("⚠ 有未保存修改")
                            .color(theme::warning())
                            .small(),
                    );
                } else {
                    ui.label(RichText::new(format!("✓ {}", tr_synced)).color(theme::success()).small());
                }
            },
        );

        self.settings_tab = tab;
        if save_clicked {
            self.save_settings();
        }
        if reset_clicked {
            self.sync_edit_settings();
            self.reload_settings();
        }
        ui.add_space(8.0);

        // 设置卡片总高度超过窗口，整页必须可纵向滚动，否则底部内容看不到。
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.settings_tab {
                SettingsTab::General => self.ui_settings_general(ui),
                SettingsTab::Appearance => self.ui_settings_appearance(ui),
                SettingsTab::Automation => self.ui_settings_automation(ui),
                SettingsTab::Network => self.ui_settings_network(ui),
                SettingsTab::Api => self.ui_api(ui),
            });
    }

    /// 外观分区：配色 + 字体大小。两条都是"改了立刻生效并自动落库"，所以不挂底部那排
    /// 保存按钮 —— 和主题一贯的行为一致。
    fn ui_settings_appearance(&mut self, ui: &mut Ui) {
        let mut auto_save = false;
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Appearance, 16.0, theme::accent());
                ui.label(RichText::new("界面外观").strong().size(15.0));
                status_badge(
                    ui,
                    &format!("字体 {}%", (self.edit_ui_scale * 100.0).round() as i32),
                    theme::muted(),
                );
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("改动即时生效并自动保存，下次启动沿用。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(10.0);

            ui.label(RichText::new("界面语言 (Interface Language)").strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for lang in crate::i18n::Language::ALL {
                    if ui
                        .selectable_value(
                            &mut self.edit_ui_language,
                            lang.code().to_string(),
                            lang.display_name(),
                        )
                        .clicked()
                    {
                        auto_save = true;
                    }
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("界面配色主题").strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for theme_spec in theme::THEMES {
                    if ui
                        .selectable_value(
                            &mut self.edit_ui_theme,
                            theme_spec.id.to_string(),
                            theme_spec.name,
                        )
                        .clicked()
                    {
                        // 不在这里直接 apply：那是"还没保存成功的预览"。走一次设置保存，
                        // 回来后由 update() 的 applied_theme_id 比较来装主题，界面和落盘一致。
                        auto_save = true;
                    }
                }
            });

            ui.add_space(10.0);
            // 夜间：只换中性色，强调色仍然是上面那 5 支里选的一支（两条轴正交）。
            let mut dark = self.edit_ui_dark;
            if ui
                .checkbox(&mut dark, RichText::new("夜间模式（深色底）"))
                .on_hover_text(
                    "只换中性色（页面、卡片、文字），强调色还是上面选的那一支 —— 5 支强调色 × 明/暗。\n\
                     改动即时生效并自动保存，关掉就回到浅白。",
                )
                .changed()
            {
                self.edit_ui_dark = dark;
                auto_save = true;
            }

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("字体大小").strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new("整屏一起缩放（文字、图标、表格都跟着走），100% = 跟随系统缩放。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            let mut percent = (self.edit_ui_scale * 100.0).round() as i32;
            let mut slider_geometry = None;
            ui.horizontal(|ui| {
                // egui 0.31 的 Slider 没有 desired_width，宽度只认 spacing.slider_width。
                let slider_width = ui.spacing().slider_width;
                const FONT_SLIDER_RAIL: f32 = 300.0;
                ui.spacing_mut().slider_width = FONT_SLIDER_RAIL;
                let response = ui
                    .add(
                        egui::Slider::new(&mut percent, 85..=140)
                            .step_by(5.0)
                            .suffix("%"),
                    )
                    .on_hover_text("85% 到 140%，步长 5%；松手才生效");
                ui.spacing_mut().slider_width = slider_width;
                slider_geometry = Some((
                    response.rect,
                    handle_travel_range(ui, &response, FONT_SLIDER_RAIL),
                ));
                if response.changed() {
                    self.edit_ui_scale = percent as f32 / 100.0;
                }
                // 只在松手那一下落库。拖动中每帧保存的话：`BgMsg::SettingsSaved` 回来会
                // `sync_edit_settings()` 把值按库里那份灌回去，手指还按在柄上，值被改 →
                // 缩放变 → 整页重排 → 滑杆几何跟着挪 → 看起来就是"页面在鬼畜"。
                if response.drag_stopped() && self.settings_dirty() {
                    self.save_settings();
                }
                if (self.edit_ui_scale - 1.0).abs() > 0.001
                    && ui
                        .button("恢复 100%")
                        .on_hover_text("回到跟随系统缩放")
                        .clicked()
                {
                    self.edit_ui_scale = 1.0;
                    auto_save = true;
                }
            });
            // 刻度条：先留出一条带，再照滑杆自己的几何画在下面。
            if let Some((rect, travel)) = slider_geometry {
                ui.add_space(FONT_TICK_BAND_HEIGHT);
                paint_font_scale_ticks(ui, rect, travel);
            }
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }

    /// 通用设置：桌面窗口生命周期、托盘行为、系统物理内存释放与 Windows 防火墙规则。
    fn ui_settings_general(&mut self, ui: &mut Ui) {
        let mut auto_save = false;

        // 1. 桌面窗口与生命周期
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Gear, 16.0, theme::accent());
                ui.label(RichText::new("桌面窗口与运行模式").strong().size(15.0));
                let tray_badge = if self.edit_close_to_tray {
                    ("托盘常驻", theme::success())
                } else {
                    ("直接退出", theme::muted())
                };
                status_badge(ui, tray_badge.0, tray_badge.1);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("配置程序关闭行为、系统托盘常驻逻辑与首次运行内核自动化下载。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            ui.label(RichText::new("点击主窗口关闭按钮 (X) 时：").strong());
            ui.add_space(4.0);
            if ui
                .radio_value(
                    &mut self.edit_close_to_tray,
                    true,
                    "最小化到系统托盘 (推荐：后台保持代理与定时自动化运行)",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .radio_value(
                    &mut self.edit_close_to_tray,
                    false,
                    "直接退出程序 (停止所有网络连接并彻底退出软件)",
                )
                .changed()
            {
                auto_save = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Lightbulb, 12.0, theme::muted());
                let tip = if self.tray.supported() {
                    "提示：最小化到托盘后，可通过任务栏托盘图标右键菜单退出程序，或双击托盘图标唤出界面。"
                } else {
                    "提示：当前系统环境（Linux）暂未接入系统托盘，关闭窗口将直接退出程序。"
                };
                ui.label(
                    RichText::new(tip)
                        .small()
                        .color(theme::muted()),
                );
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            if ui
                .checkbox(
                    &mut self.edit_auto_prepare_kernel,
                    "预备状态（未安装内核）时自动下载 Mihomo 内核",
                )
                .on_hover_text(
                    "首次安装或重置后，检测到一个内核都没装时自动下载官方内核；关掉后仅在手动点击时下载。",
                )
                .changed()
            {
                auto_save = true;
            }
        });

        ui.add_space(10.0);

        // 2. 资源优化与系统安全
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Shield, 16.0, theme::accent());
                ui.label(RichText::new("系统资源与防火墙放行").strong().size(15.0));
                let elevated = crate::platform::is_elevated();
                let perm_badge = if elevated {
                    ("管理员权限", theme::success())
                } else {
                    ("标准用户权限", theme::muted())
                };
                status_badge(ui, perm_badge.0, perm_badge.1);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("管理进程常驻内存释放，以及 Windows 本地回环端口防火墙放行规则。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            ui.label(RichText::new("进程内存深度整理").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "主动释放 SQLite 连接池缓存与 Windows 物理工作集，显著降低程序与后台服务常驻内存。",
                    )
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .add(icons::button_with_icon(Icon::Sync, "深度整理内存", theme::accent()))
                    .on_hover_text("整理系统工作集，立即释放进程未使用的物理内存")
                    .clicked()
                {
                    self.trim_memory();
                }
                if let Some(status) = &self.memory_trimmed_status {
                    ui.label(RichText::new(status).small().color(theme::success()));
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("Windows 防火墙放行").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "放行程序与代理内核的本地回环监听端口，避免代理启动后 9980 端口无响应或被系统拦截。",
                    )
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut self.edit_firewall_auto_allow,
                    "软件启动时自动加入防火墙放行规则",
                )
                .changed()
            {
                auto_save = true;
                if self.edit_firewall_auto_allow && crate::platform::is_elevated() {
                    self.apply_firewall_rules();
                }
            }
            ui.add_space(4.0);
            let elevated = crate::platform::is_elevated();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(elevated, primary_btn("立即写入防火墙规则"))
                    .on_hover_text("对本程序和已下载到 managed kernels 目录的内核 exe 逐条写入允许规则")
                    .clicked()
                {
                    self.apply_firewall_rules();
                }
                if let Some(status) = self.firewall_status.clone() {
                    ui.label(RichText::new(status).small().color(theme::text_secondary()));
                } else if !elevated {
                    ui.label(
                        RichText::new("（当前非管理员权限：勾选后在提权启动时自动补上）")
                            .small()
                            .color(theme::muted()),
                    );
                }
            });
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }

    /// 订阅与测速：订阅源定时拉取策略、全协议测速并发与参数、落地剪枝优化。
    fn ui_settings_automation(&mut self, ui: &mut Ui) {
        let mut auto_save = false;

        // 1. 订阅源定时同步
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Subscription, 16.0, theme::accent());
                ui.label(RichText::new("订阅源定时同步").strong().size(15.0));
                let (badge_text, badge_color) = if self.edit_auto_fetch {
                    ("自动同步已开启", theme::success())
                } else {
                    ("已关闭", theme::muted())
                };
                status_badge(ui, badge_text, badge_color);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("在后台周期性拉取已启用的订阅源，自动解析、解密并更新本地节点池。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            if ui
                .checkbox(
                    &mut self.edit_auto_fetch,
                    "启用后台定时自动拉取全部有效订阅源",
                )
                .changed()
            {
                auto_save = true;
            }
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("⏱️ 同步周期").strong());
                        let r1 = ui.add(
                            egui::DragValue::new(&mut self.edit_interval_min)
                                .range(5..=1440)
                                .suffix(" 分钟"),
                        );
                        if r1.drag_stopped() || r1.lost_focus() {
                            auto_save = true;
                        }
                        ui.label(
                            RichText::new("最小 5 分钟，建议 60")
                                .small()
                                .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("🔗 订阅并发任务").strong());
                        let r2 = ui.add(
                            egui::DragValue::new(&mut self.edit_fetch_concurrency)
                                .range(1..=8)
                                .suffix(" 并发"),
                        );
                        if r2.drag_stopped() || r2.lost_focus() {
                            auto_save = true;
                        }
                        ui.label(
                            RichText::new("建议 2~4，避免上游封禁")
                                .small()
                                .color(theme::muted()),
                        );
                    });
                });
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("⏳ 单源请求超时").strong());
                        let r3 = ui.add(
                            egui::DragValue::new(&mut self.edit_fetch_timeout_secs)
                                .range(10..=120)
                                .suffix(" 秒"),
                        );
                        if r3.drag_stopped() || r3.lost_focus() {
                            auto_save = true;
                        }
                        ui.label(
                            RichText::new("单源超时，建议 30 秒")
                                .small()
                                .color(theme::muted()),
                        );
                    });
                });
            });

            ui.add_space(6.0);
            if let Some(t) = self.settings.last_auto_fetch_at {
                ui.label(
                    RichText::new(format!(
                        "上次自动拉取执行时间: {}",
                        t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S")
                    ))
                    .small()
                    .color(theme::muted()),
                );
            } else {
                ui.label(
                    RichText::new("上次自动拉取执行时间: 从未触发")
                        .small()
                        .color(theme::muted()),
                );
            }
        });

        ui.add_space(10.0);

        // 2. 节点测速与质量检验
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Lightning, 16.0, theme::accent());
                ui.label(RichText::new("节点测速与质量检验").strong().size(15.0));
                let (badge_text, badge_color) = if self.edit_auto_speed {
                    ("拉取联动测速开启", theme::success())
                } else {
                    ("手动测速模式", theme::muted())
                };
                status_badge(ui, badge_text, badge_color);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("配置全协议节点真实握手、TCP 预筛、下载带宽探测与自动化测速策略。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            ui.label(RichText::new("自动测速触发条件").strong());
            ui.add_space(4.0);
            if ui
                .checkbox(&mut self.edit_auto_speed, "订阅拉取完成后自动执行批量测速")
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_added_source,
                    "添加订阅源后自动拉取它，并只测速这个源",
                )
                .on_hover_text(
                    "只对手动新增的源生效（Gist/频道批量发现不触发，免得一次登记多个源引发连续测速）。",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_auto_refill,
                    "测通节点少于 20 个时，后台自动补一轮全量测速",
                )
                .on_hover_text(
                    "只在后台跑：并发固定 8，不探测下载吞吐，距上次自动测速至少隔 30 分钟才会跑一轮。",
                )
                .changed()
            {
                auto_save = true;
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("测速并发与阈值参数").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("🚀 测速并发数").strong());
                        let r4 = ui.add(
                            egui::DragValue::new(&mut self.edit_speedtest_concurrency)
                                .range(1..=512)
                                .suffix(" 线程"),
                        );
                        if r4.drag_stopped() || r4.lost_focus() {
                            auto_save = true;
                        }
                        ui.label(RichText::new("建议 64~128，性能好可 256").small().color(theme::muted()));
                    });
                });
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("⏱️ 连接超时阈值").strong());
                        let r5 = ui.add(
                            egui::DragValue::new(&mut self.edit_timeout_ms)
                                .range(200..=15000)
                                .suffix(" ms"),
                        );
                        if r5.drag_stopped() || r5.lost_focus() {
                            auto_save = true;
                        }
                        ui.label(RichText::new("握手与 204 探针超时").small().color(theme::muted()));
                    });
                });
                ui.add_space(8.0);
                ui.group(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("📦 下载探测时长").strong());
                        let r6 = ui.add(
                            egui::DragValue::new(&mut self.edit_download_test_ms)
                                .range(200..=5000)
                                .suffix(" ms"),
                        );
                        if r6.drag_stopped() || r6.lost_focus() {
                            auto_save = true;
                        }
                        ui.label(RichText::new("仅开启下载测速时生效").small().color(theme::muted()));
                    });
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            ui.label(RichText::new("测速加速与剪枝优化").strong());
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut self.edit_speedtest_download,
                    "额外探测节点真实下载速度（进行真实数据包吞吐测试，耗时较长）",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_dedup_egress,
                    "同物理落地剪枝（多节点解析到相同目标 IP:Port 时只测 1 次，大幅提速）",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_tcp_ping_only,
                    "⚡ 仅 TCP Ping 快速粗筛（跳过内核真实代理握手，数千节点可在数秒内初筛完成）",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_parallel_kernels,
                    "三内核并行验证（墙钟约 1/3，峰值内存约 3 倍）",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_persistent_kernel,
                    "复用常驻内核，用控制器接口切换待测节点（省去冷启动时间）",
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .checkbox(
                    &mut self.edit_speedtest_conditional_isolation,
                    "只对可疑的失败节点做隔离复测（默认全部复测）",
                )
                .changed()
            {
                auto_save = true;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new("沿用已验证结果:").small().color(theme::muted()));
                let r7 = ui.add(
                    egui::DragValue::new(&mut self.edit_speedtest_reuse_minutes)
                        .range(0..=1440)
                        .speed(5)
                        .suffix(" 分钟"),
                );
                if r7.drag_stopped() || r7.lost_focus() {
                    auto_save = true;
                }
                ui.label(RichText::new("0 = 每轮全部重测").small().color(theme::muted()));
            });

            if let Some(t) = self.settings.last_auto_speedtest_at {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!(
                        "上次自动测速执行时间: {}",
                        t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S")
                    ))
                    .small()
                    .color(theme::muted()),
                );
            }
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }

    /// 自愈网络：节点故障自动切换 (Failover)、系统代理看门狗、Geo 规则与本地 HTTP API 服务。
    fn ui_settings_network(&mut self, ui: &mut Ui) {
        let mut auto_save = false;

        // 1. 节点故障自愈与系统代理守护
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Shield, 16.0, theme::accent());
                ui.label(RichText::new("节点故障自愈与代理守护").strong().size(15.0));
                let (badge_text, badge_color) = if self.auto_failover {
                    ("自动切换已开启", theme::success())
                } else {
                    ("自动切换已关闭", theme::muted())
                };
                status_badge(ui, badge_text, badge_color);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("实时监测当前代理出口可用性，在断连或延迟严重劣化时秒级切换至最优备选节点。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            // Failover
            if ui
                .checkbox(
                    &mut self.auto_failover,
                    "节点故障自动切换（Failover）：每 60 秒经当前代理实测一次，由调度器判定",
                )
                .on_hover_text(
                    "调度器规则：EWMA 平滑延迟；连续 2 次验证失败开断路器并拉黑 5 分钟；\
                     候选必须比当前节点好 25% 以上才切（滞后），且两次切换至少隔 2 分钟。",
                )
                .changed()
            {
                let on = self.auto_failover;
                self.set_auto_failover(on);
                self.status = if on {
                    "节点故障自动切换已开启，设置已保存".into()
                } else {
                    "节点故障自动切换已关闭（探测照跑，不自动换出口）".into()
                };
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("切换延迟阈值:").small().color(theme::muted()));
                let r1 = ui.add(
                    egui::DragValue::new(&mut self.edit_failover_latency_ms)
                        .range(200..=15_000)
                        .speed(50)
                        .suffix(" ms"),
                );
                if r1.drag_stopped() || r1.lost_focus() {
                    auto_save = true;
                }
                ui.label(
                    RichText::new("实测为穿过当前代理的完整 HTTPS 请求；连续三次超阈值触发切换")
                        .small()
                        .color(theme::muted()),
                );
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.checkbox(&mut self.edit_bandwidth_guard, "启用带宽保护").changed() {
                    auto_save = true;
                }
                let r2 = ui.add(
                    egui::DragValue::new(&mut self.edit_bandwidth_min_kbps)
                        .range(0..=10000)
                        .speed(10)
                        .suffix(" KB/s"),
                );
                if r2.drag_stopped() || r2.lost_focus() {
                    auto_save = true;
                }
                ui.label(
                    RichText::new("当前出口下行持续低于该值也算劣化（需开启自动切换）")
                        .small()
                        .color(theme::muted()),
                );
            });
            if let Some(reason) = &self.failover_last_reason {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("最近切换原因: {reason}")).small().color(theme::accent()));
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // 系统代理守护
            ui.label(RichText::new("系统代理注册表守护").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "防止系统代理被外部工具或系统设置意外关闭。开启后每 30 秒自动核对并写回本程序代理地址。",
                    )
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut self.edit_auto_rearm_system_proxy,
                    "系统代理被外部关闭时，自动写回本程序的代理地址",
                )
                .on_hover_text(
                    "每 30 秒核对一次注册表 ProxyEnable / ProxyServer。\
                     和别的代理工具共存时建议保持关闭，否则两边会互相抢。",
                )
                .changed()
            {
                auto_save = true;
            }

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // 节点关键词黑名单
            ui.label(RichText::new("节点关键词排除（黑名单）").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "多个词用逗号、分号或空格分隔。命中的节点不会成为“一键连接最优”和自动切换的候选，也不参与推送与导出。",
                    )
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            let exclude_field = ui.add(
                egui::TextEdit::singleline(&mut self.edit_exclude_keywords)
                    .hint_text("例如：到期, 限速, ×3, 未验证")
                    .desired_width(ui.available_width()),
            );
            if exclude_field.lost_focus() {
                auto_save = true;
            }
            crate::gui::widgets::note_text_field(ui, &exclude_field);
        });

        ui.add_space(10.0);

        // 2. 分流规则与外部服务接口
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Rules, 16.0, theme::accent());
                ui.label(RichText::new("分流规则与外部服务接口").strong().size(15.0));
                let is_running = self.api_controller.is_running();
                let (badge_text, badge_color) = if is_running {
                    ("HTTP 服务运行中", theme::success())
                } else {
                    ("HTTP 服务已停止", theme::muted())
                };
                status_badge(ui, badge_text, badge_color);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("热重载 GeoIP / GeoSite 路由规则数据库，以及管理供外部调用的 RESTful API。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            // Geo Database
            ui.label(RichText::new("分流规则数据库 (GeoIP / GeoSite)").strong());
            ui.add(
                egui::Label::new(
                    RichText::new("在线热重载最新的国家 IP 归属与域名列表，确保分流精确度。")
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
                        egui::Button::new("在线更新 Geo 数据库"),
                    )
                    .on_hover_text("向当前代理内核发送指令，在线拉取并热重载最新的 Geo 规则数据库")
                    .clicked()
                {
                    self.update_geo_data();
                }
                if self.geo_updating {
                    ui.spinner();
                    ui.label(RichText::new("正在更新 Geo 规则...").small().color(theme::accent()));
                } else if !running {
                    ui.label(RichText::new("（需先启动代理）").small().color(theme::muted()));
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            // HTTP API Server
            ui.label(RichText::new("本地 HTTP API 服务与端口管理").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "提供 RESTful 接口供外部脚本或插件调用。当前监听: http://{}",
                        self.api_addr
                    ))
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let is_running = self.api_controller.is_running();
                if is_running {
                    if ui
                        .add(icons::danger_icon_btn(Icon::Stop, "停止 HTTP 服务"))
                        .on_hover_text("停止后台 HTTP API 监听，释放端口与网络资源")
                        .clicked()
                    {
                        self.stop_api_server();
                    }
                } else {
                    if ui
                        .add(icons::primary_icon_btn(Icon::Play, "启动 HTTP 服务"))
                        .on_hover_text("启动后台 HTTP API 监听，恢复外部接口访问")
                        .clicked()
                    {
                        self.start_api_server();
                    }
                }
            });
            ui.add_space(4.0);
            if ui
                .checkbox(&mut self.edit_http_server_enabled, "软件启动时自动开启 HTTP API 服务")
                .on_hover_text("若平时不需要外部接口调用，可取消勾选以节约系统资源")
                .changed()
            {
                auto_save = true;
            }
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }

    pub(crate) fn ui_api(&mut self, ui: &mut Ui) {
        let is_running = self.api_controller.is_running();
        ui.horizontal(|ui| {
            if is_running {
                status_badge(ui, "服务运行中", theme::success());
                ui.label(format!("本地监听地址: http://{}", self.api_addr));
                if let Some(token) = self.api_controller.token() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("访问令牌:").small().color(theme::muted()));
                        if ui
                            .add(
                                egui::Label::new(RichText::new(token).small().strong())
                                    .selectable(true),
                            )
                            .on_hover_text("所有接口都需要它；只读 GET 可用 ?token= 传")
                            .clicked()
                        {
                            ui.ctx().copy_text(token.to_string());
                            self.status = "已复制访问令牌".into();
                        }
                    });
                }
                if ui
                    .add(icons::danger_icon_btn(Icon::Stop, "关闭 HTTP 服务"))
                    .on_hover_text("停止后台 HTTP API 监听")
                    .clicked()
                {
                    self.stop_api_server();
                }
            } else {
                status_badge(ui, "已停止运行", theme::muted());
                ui.label(
                    RichText::new("本地 HTTP API 服务当前已停止（端口已释放）")
                        .color(theme::muted()),
                );
                if ui
                    .add(icons::primary_icon_btn(Icon::Play, "启动 HTTP 服务"))
                    .on_hover_text("启动后台 HTTP API 监听")
                    .clicked()
                {
                    self.start_api_server();
                }
            }
        });
        ui.add_space(6.0);
        card_frame().show(ui, |ui| {
            ui.strong("发布服务器");
            ui.label("用于把本地真实测速成功的节点发布到 Linux Web 后台，不会切换本地数据源。");
            ui.horizontal(|ui| {
                ui.label("服务器地址:");
                let addr_field = ui.add(
                    egui::TextEdit::singleline(&mut self.remote_api_edit)
                        .hint_text("https://你的服务器（默认留空）")
                        .desired_width(320.0),
                );
                crate::gui::widgets::note_text_field(ui, &addr_field);
                ui.label("推送 Token:");
                let token_field = ui.add(
                    egui::TextEdit::singleline(&mut self.remote_api_token)
                        .password(true)
                        .hint_text("Bearer Token")
                        .desired_width(200.0),
                );
                crate::gui::widgets::note_text_field(ui, &token_field);
                if ui.button("保存配置").clicked() {
                    let addr = if self.remote_api_edit.trim().is_empty() {
                        self.remote_api_addr = None;
                        String::new()
                    } else {
                        let normalized = normalize_api_base(self.remote_api_edit.trim());
                        self.remote_api_edit = normalized.clone();
                        self.remote_api_addr = Some(normalized.clone());
                        normalized
                    };
                    self.remote_api_token = self.remote_api_token.trim().to_string();
                    self.save_publisher(addr, self.remote_api_token.clone());
                }
                if ui.button("清空配置").clicked() {
                    self.remote_api_addr = None;
                    self.remote_api_edit.clear();
                    self.remote_api_token.clear();
                    self.save_publisher(String::new(), String::new());
                }
                if ui.button("检查连接").clicked() {
                    self.check_remote_api();
                }
            });
            ui.label(RichText::new(&self.remote_api_status).color(theme::muted()));
        });
        ui.add_space(8.0);
        ui.add_space(6.0);

        struct ApiItem {
            method: &'static str,
            path: &'static str,
            desc: &'static str,
            openable: bool,
        }
        let items = [
            ApiItem { method: "GET", path: "/api/health", desc: "健康检查", openable: true },
            ApiItem { method: "GET", path: "/api/stats", desc: "统计信息", openable: true },
            ApiItem { method: "GET", path: "/api/settings", desc: "读取设置", openable: true },
            ApiItem { method: "PUT", path: "/api/settings", desc: "更新设置", openable: false },
            ApiItem { method: "GET", path: "/api/sources", desc: "订阅源列表", openable: true },
            ApiItem { method: "POST", path: "/api/sources", desc: "创建订阅源", openable: false },
            ApiItem { method: "DELETE", path: "/api/sources/{id}", desc: "删除订阅源及关联节点", openable: false },
            ApiItem { method: "POST", path: "/api/sources/fetch-all", desc: "拉取全部启用源", openable: false },
            ApiItem { method: "GET", path: "/api/nodes?page=1&sort=latency", desc: "节点分页查询", openable: true },
            ApiItem { method: "POST", path: "/api/nodes/speedtest", desc: "自动内核真实代理批量测速，可传 ids 只测本页", openable: false },
            ApiItem { method: "GET", path: "/api/kernels", desc: "检测 Mihomo、Xray、Sing-box 路径和版本", openable: true },
            ApiItem { method: "GET", path: "/api/proxy/status", desc: "读取代理运行状态", openable: true },
            ApiItem { method: "GET", path: "/api/proxy/config", desc: "读取 TUN、DNS 与分流规则配置", openable: true },
            ApiItem { method: "PUT", path: "/api/proxy/config", desc: "更新并持久化代理配置", openable: false },
            ApiItem { method: "GET", path: "/api/proxy/log", desc: "读取当前代理内核最近日志", openable: true },
            ApiItem { method: "POST", path: "/api/proxy/start", desc: "按 node_id 自动选择内核启动（系统代理或 TUN）", openable: false },
            ApiItem { method: "POST", path: "/api/proxy/stop", desc: "停止当前代理内核并恢复系统代理", openable: false },
            ApiItem { method: "POST", path: "/api/proxy/restart", desc: "按 node_id 重启代理", openable: false },
            ApiItem { method: "POST", path: "/api/nodes/delete-dead", desc: "删除超时/失败节点", openable: false },
            ApiItem { method: "POST", path: "/api/nodes/delete-invalid", desc: "删除 127.0.0.1/内网/localhost 等无效主机", openable: false },
            ApiItem { method: "POST", path: "/api/nodes/dedupe", desc: "去除重复节点（同协议/地址/端口/认证）", openable: false },
            ApiItem { method: "GET", path: "/api/export/json?quality_only=true&max_latency_ms=500&min_download_kbps=0&limit=50", desc: "导出 TopN TCP 可用节点 JSON", openable: true },
            ApiItem { method: "GET", path: "/api/export/clash?quality_only=true&max_latency_ms=500&min_download_kbps=0&limit=50", desc: "导出 TopN Clash 配置", openable: true },
            ApiItem { method: "GET", path: "/api/export/uri?quality_only=true&max_latency_ms=500&min_download_kbps=0&limit=50&base64=true", desc: "导出 TopN 订阅（Base64，v2rayN 用）", openable: true },
            ApiItem { method: "GET", path: "/api/export/uri?quality_only=true&max_latency_ms=300&limit=30&base64=true", desc: "更严格：Top30 + 延迟≤300ms（v2rayN）", openable: true },
            ApiItem { method: "GET", path: "/api/export/clash?quality_only=false", desc: "导出全部节点 Clash（关闭过滤）", openable: true },
        ];

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in items {
                    let full_url = self.api_url(item.path, item.openable);
                    card_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (m_bg, m_fg) = match item.method {
                                "GET" => (theme::accent_bg(), theme::accent_strong()),
                                "POST" => (theme::success_bg(), theme::success()),
                                "PUT" => (theme::warning_bg(), theme::warning()),
                                "DELETE" => (theme::danger_bg(), theme::danger()),
                                _ => (theme::panel(), theme::muted()),
                            };
                            Frame::NONE
                                .fill(m_bg)
                                .stroke(Stroke::new(1.0_f32, m_fg.gamma_multiply(0.4)))
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(Margin::symmetric(7, 2))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(item.method).strong().color(m_fg).small(),
                                    );
                                });
                            let resp = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(item.path).monospace().color(theme::accent()),
                                    )
                                    .sense(Sense::click()),
                                )
                                .on_hover_text(format!("点击复制 {full_url}"));
                            if resp.clicked() {
                                ui.ctx().copy_text(full_url.clone());
                                self.status = format!("已复制: {full_url}");
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if item.openable
                                        && !item.path.contains('{')
                                        && ui.small_button("打开").clicked()
                                    {
                                        ui.ctx().open_url(egui::OpenUrl::new_tab(&full_url));
                                    }
                                    if ui.small_button("复制").clicked() {
                                        ui.ctx().copy_text(full_url.clone());
                                        self.status = format!("已复制: {full_url}");
                                    }
                                },
                            );
                        });
                        ui.label(RichText::new(item.desc).small().color(theme::muted()));
                    });
                    ui.add_space(4.0);
                }
            });
    }

    pub(crate) fn publish_available_nodes(&mut self) {
        if self.busy {
            self.status = "请等待当前任务完成后再推送节点".into();
            return;
        }
        let Some(base) = self
            .remote_api_addr
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        else {
            self.status = "尚未配置发布服务器地址，请先在系统设置里填写".into();
            return;
        };
        let token = self.remote_api_token.trim().to_string();
        if token.is_empty() {
            self.status = "尚未配置推送 Token，请先在系统设置里填写".into();
            return;
        }
        let db = self.db.clone();
        let tx = self.tx.clone();
        let keywords = self.settings.exclude_keywords.clone();
        self.busy = true;
        self.status = "正在读取全部真实代理验证成功的节点...".into();
        self.rt.spawn(async move {
            // 标了“不推送”的源、以及命中排除关键词的节点，都不参与推送。
            let (nodes, excluded_count) =
                match db.export_pushable_nodes(Some(2000), &keywords).await {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = tx.send(BgMsg::PublishDone(Err(error.to_string())));
                        return;
                    }
                };
            if nodes.is_empty() {
                let _ = tx.send(BgMsg::PublishDone(Err(if excluded_count > 0 {
                    format!("验证成功的节点全部来自标记为不推送的源（跳过 {excluded_count} 个）")
                } else {
                    "当前没有真实代理验证成功的节点可推送".into()
                })));
                return;
            }
            let client = reqwest::Client::new();
            let result = client
                .post(format!("{base}/api/push/nodes"))
                .bearer_auth(token)
                .json(&serde_json::json!({"nodes": nodes}))
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
                .map_err(|error| format!("连接失败: {error}"))
                .and_then(|response| {
                    response
                        .error_for_status()
                        .map_err(|error| format!("服务器返回错误: {error}"))
                });
            let result = match result {
                Ok(response) => response
                    .json::<serde_json::Value>()
                    .await
                    .map_err(|error| format!("解析服务器响应失败: {error}")),
                Err(error) => Err(error),
            };
            let message = match result {
                Ok(body) => {
                    let data = body.get("data").cloned().unwrap_or_default();
                    let skipped_note = if excluded_count > 0 {
                        format!("，已跳过不推送源 {excluded_count} 个节点")
                    } else {
                        String::new()
                    };
                    Ok(format!(
                        "已推送 {} 个节点，服务器保存 {} 个（版本 {}）{skipped_note}",
                        nodes.len(),
                        data.get("stored").and_then(|v| v.as_i64()).unwrap_or(0),
                        data.get("version").and_then(|v| v.as_i64()).unwrap_or(0)
                    ))
                }
                Err(error) => Err(error),
            };
            let _ = tx.send(BgMsg::PublishDone(message));
        });
    }

    /// 拼出可直接在浏览器打开的完整 API URL，并按需带上访问令牌
    /// （服务端只对只读 GET 接受 `?token=`，写操作必须走 Bearer 头）。
    pub(crate) fn api_url(&self, path: &str, with_token: bool) -> String {
        let url = format!("http://{}{path}", self.api_addr);
        match (with_token, self.api_controller.token()) {
            (true, Some(token)) if !token.is_empty() => {
                let sep = if url.contains('?') { '&' } else { '?' };
                format!("{url}{sep}token={token}")
            }
            _ => url,
        }
    }

    pub(crate) fn check_remote_api(&mut self) {
        let Some(base) = self
            .remote_api_addr
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        else {
            self.remote_api_status = "尚未配置发布服务器地址".into();
            return;
        };
        let token = self.remote_api_token.trim().to_string();
        let tx = self.tx.clone();
        self.remote_api_status = "检查中...".into();
        self.rt.spawn(async move {
            let result = reqwest::Client::new()
                .get(format!("{base}/api/push/health"))
                .bearer_auth(token)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
                .and_then(|response| response.error_for_status())
                .map(|_| "远程服务在线且 Token 验证成功".to_string())
                .map_err(|error| format!("连接失败: {error}"));
            let _ = tx.send(BgMsg::RemoteApiCheck(result));
        });
    }
}

/// 字号滑杆的刻度带留多高（刻度线 + 数字）。
const FONT_TICK_BAND_HEIGHT: f32 = 20.0;
const FONT_SCALE_MIN: i32 = 85;
const FONT_SCALE_MAX: i32 = 140;

/// 手柄中心实际走过的区间。**必须**和 egui 自己算的一致，否则刻度和手柄会错位。
///
/// 两条来自 egui 0.31 源码的事实：① 手柄位置是按**轨道**算的，轨道宽 = `spacing.slider_width`，
/// 而 `ui.add(Slider)` 返回的 response 把右边那段数值文字（"100%"）也算进了宽度 ——
/// 直接拿 `response.rect.x_range()` 会把刻度整体往右推（实测 140% 处偏 48px，
/// 2026-10-15 他报的"点恢复 100% 小球跑到 90 刻度"）；② `handle_radius = 轨道高/2.5`，
/// `HandleShape::Rect` 再乘 `aspect_ratio`。
fn handle_travel_range(ui: &Ui, response: &egui::Response, rail_width: f32) -> egui::Rangef {
    let radius = response.rect.height() / 2.5;
    let radius = match ui.style().visuals.handle_shape {
        egui::style::HandleShape::Circle => radius,
        egui::style::HandleShape::Rect { aspect_ratio } => radius * aspect_ratio,
    };
    let left = response.rect.left();
    egui::Rangef::new(left + radius, left + rail_width - radius)
}

/// 每个刻度的 `(百分比, x, 是否长刻度)`。抽成纯函数是因为画出来的东西我看不见 ——
/// 几何只能这样钉成断言。
fn font_scale_ticks(travel: egui::Rangef) -> Vec<(i32, f32, bool)> {
    let span = (FONT_SCALE_MAX - FONT_SCALE_MIN) as f32;
    (FONT_SCALE_MIN..=FONT_SCALE_MAX)
        .step_by(5)
        .map(|value| {
            let ratio = (value - FONT_SCALE_MIN) as f32 / span;
            (
                value,
                egui::lerp(travel.min..=travel.max, ratio),
                matches!(value, 85 | 100 | 120 | 140),
            )
        })
        .collect()
}

/// 把刻度画在滑杆下方那条带里。100 用强调色 —— 那是"跟随系统缩放"的基准点，
/// 一眼要能找回来。
fn paint_font_scale_ticks(ui: &Ui, rect: egui::Rect, travel: egui::Rangef) {
    let painter = ui.painter();
    let top = rect.bottom() + 2.0;
    let color = theme::muted();
    for (value, x, major) in font_scale_ticks(travel) {
        let height = if major { 7.0 } else { 4.0 };
        painter.line_segment(
            [egui::pos2(x, top), egui::pos2(x, top + height)],
            egui::Stroke::new(
                1.0_f32,
                if major {
                    color
                } else {
                    color.gamma_multiply(0.5)
                },
            ),
        );
        if major {
            painter.text(
                egui::pos2(x, top + height + 1.0),
                egui::Align2::CENTER_TOP,
                format!("{value}"),
                egui::FontId::proportional(9.5),
                if value == 100 { theme::accent() } else { color },
            );
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use eframe::egui::{self, Label, RawInput, Rect, Vec2};

    /// 画一段表头，然后报告纵向 ui 里还剩多少高度给下面的卡片。
    fn height_left_after(header: impl Fn(&mut egui::Ui)) -> f32 {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1200.0, 800.0),
        ));
        let mut left = -1.0;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                header(ui);
                left = ui.available_height();
            });
        });
        left
    }

    #[test]
    fn right_to_left_directly_on_a_vertical_ui_eats_the_page() {
        // 这就是"系统设置页只剩标题"的成因：`with_layout` 放在纵向 ui 上时，
        // 子 ui 的 max_rect 一直延伸到父级底部，父级按 min_rect 推进游标 —— 整页高度归它。
        // 写成断言而不是注释，是因为它会"编译通过、只是页面空掉"。
        let eaten = height_left_after(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let _ = ui.button("保存设置");
                let _ = ui.button("重置修改");
            });
        });
        assert!(eaten < 1.0, "这个坑应该已经把整页吃光，实测剩 {eaten:.1}");
    }

    #[test]
    fn settings_header_leaves_room_for_the_cards() {
        let plain = height_left_after(|ui| {
            ui.horizontal(|ui| {
                ui.label("系统设置");
                ui.label("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务");
            });
        });
        // ui_settings 现在的两行表头：标题一行，右对齐动作条包在 horizontal 里另一行。
        let shipped = height_left_after(|ui| {
            ui.horizontal(|ui| {
                ui.label("系统设置");
                ui.add(Label::new("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务").truncate());
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = ui.button("保存设置");
                    let _ = ui.button("重置修改");
                    ui.label("✓ 已同步");
                });
            });
        });
        assert!(plain > 700.0, "对照组本身不对: {plain:.1}");
        assert!(
            shipped > plain * 0.9,
            "表头把卡片的空间吃掉了：shipped={shipped:.1} plain={plain:.1}"
        );
    }
}

#[cfg(test)]
mod header_probe_tests {
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻系统设置页那一列的结构：CentralPanel(左右 18 内边距) → 标题行 →
    /// 右对齐动作行 → ScrollArea(里面是不换行的一排统计芯片)。
    /// `pinned` = 动作组和标题同一行（标题侧限宽截断）的新写法。
    /// 返回（窗口右边界, 保存设置左/右边界, 芯片行右边界）。
    fn probe(
        window_width: f32,
        chips: usize,
        chips_before_header: bool,
        pinned: bool,
    ) -> (f32, f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 890.0),
        ));
        let mut save_right = 0.0_f32;
        let mut save_left = 0.0_f32;
        let mut chips_right = 0.0_f32;
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
                    if chips_before_header {
                        ui.horizontal_wrapped(|ui| {
                            for _ in 0..chips {
                                let _ = ui.button("芯片 12345");
                            }
                        });
                        chips_right = ui.min_rect().max.x;
                    }
                    if pinned {
                        // 直接调产品那份 helper，探针测的就是真代码。
                        crate::gui::widgets::split_row(
                            ui,
                            "settings:header",
                            300.0,
                            |ui| {
                                let _ = ui.label("系统设置");
                                let _ = ui.add(
                                    egui::Label::new(
                                        "应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务",
                                    )
                                    .truncate(),
                                );
                            },
                            |ui| {
                                let save = ui.button("保存设置");
                                save_right = save.rect.max.x;
                                save_left = save.rect.min.x;
                                let _ = ui.button("重置修改");
                                let _ = ui.label("⚠ 有未保存修改");
                            },
                        );
                    } else {
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("系统设置");
                            let _ = ui.label("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务");
                        });
                        let _ = ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let save = ui.button("保存设置");
                                    save_right = save.rect.max.x;
                                    save_left = save.rect.min.x;
                                    let _ = ui.button("重置修改");
                                    let _ = ui.label("⚠ 有未保存修改");
                                },
                            );
                        });
                    }
                    if !chips_before_header {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for _ in 0..chips {
                                        let _ = ui.button("芯片 12345");
                                    }
                                });
                                chips_right = ui.min_rect().max.x;
                            });
                    }
                });
        });
        (window_width, save_left, save_right, chips_right)
    }

    /// 表头那组「保存设置 / 重置修改」必须整个留在中央面板里 —— 不管前面那排统计芯片
    /// 换不换行、窗口多窄、字体调到多大（缩放只是把逻辑宽度挤小）。
    /// 用户 2026-10-07 报的是按钮被窗口右缘裁掉，2026-10-14 把字体调到 140% 又报同样症状，
    /// 所以这次连"左边缘也得在面板内"一起钉住：右对齐组溢出时是先从左边被切掉的。
    #[test]
    fn save_button_stays_inside_the_panel_at_every_width() {
        for (width, chips, before) in [
            (1298.0, 11, true),
            (1298.0, 11, false),
            (1298.0, 3, true),
            (1000.0, 11, true),
            (760.0, 11, true),
            (620.0, 11, true),
            (520.0, 11, true),
            (420.0, 11, true),
        ] {
            let (w, left, right, row) = probe(width, chips, before, true);
            // CentralPanel 在 220 宽的侧栏右边，左右各 18 内边距。
            let panel_left = 220.0 + 18.0;
            let panel_right = w - 18.0;
            assert!(
                right <= panel_right + 0.5,
                "窗口 {w:.0}（芯片 {chips} 个，表头前={before}）时保存设置画到 {right:.1}，越过面板右边界 {panel_right:.1}"
            );
            assert!(
                (right - panel_right).abs() < 1.0,
                "窗口 {w:.0} 时保存设置右边缘 {right:.1} 没有贴住面板右边界 {panel_right:.1}"
            );
            assert!(
                left >= panel_left - 0.5,
                "窗口 {w:.0} 时保存设置的左边缘在 {left:.1}，被面板左边界 {panel_left:.1} 切掉了"
            );
            assert!(
                row <= panel_right + 0.5,
                "窗口 {w:.0} 时芯片行画到 {row:.1}，越过面板右边界 {panel_right:.1}"
            );
        }
    }

    /// 反向验证：老写法（标题一行、右对齐组另起一行）在窄面板下**确实**会把按钮推出右边界，
    /// 所以上面那条不是空跑。2026-10-15 量到的原始症状是 520 宽时按钮画到 580。
    #[test]
    fn the_old_two_row_header_really_did_overflow() {
        let (w, _left, right, _row) = probe(520.0, 11, true, false);
        assert!(
            right > w - 18.0 + 0.5,
            "老写法在 {w:.0} 宽没溢出（右边缘 {right:.1}）—— 那这条反向验证失去意义了"
        );
    }
}
#[cfg(test)]
mod zoom_semantics_tests {
    use eframe::egui::{self, Context, RawInput, Rect, Vec2};

    /// 缩放到底会不会把"可用的逻辑宽度"变小？这决定了 `set_zoom_factor` 能不能当字号开关用。
    /// 读的是 egui 自己的 `screen_rect()`，不是我们的布局代码。
    #[test]
    fn what_zoom_does_to_the_logical_screen_width() {
        let ctx = Context::default();
        let mut widths = Vec::new();
        for frame in 0..3 {
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1330.0, 800.0),
            ));
            let _ = ctx.run(input, |ctx| {
                if frame == 1 {
                    ctx.set_zoom_factor(1.1);
                }
                widths.push((ctx.screen_rect().width(), ctx.pixels_per_point()));
            });
        }
        // zoom 是"下一帧才生效"的：设了之后那一帧仍旧按 1330 逻辑点排。
        assert!(
            (widths[0].0 - 1330.0).abs() < 1.0 && (widths[0].1 - 1.0).abs() < 0.01,
            "{widths:?}"
        );
        // 生效以后逻辑宽度按 zoom 缩小（1330/1.1=1209），这就是"字体调大 = 可用宽度变小"
        // 的唯一来源 —— 界面上溢出与否，得按这个逻辑宽度去算，不是物理像素。
        assert!(
            (widths[2].0 - 1209.1).abs() < 0.5 && (widths[2].1 - 1.10).abs() < 0.01,
            "{widths:?}"
        );
    }
}

#[cfg(test)]
mod stats_row_overflow_probe_tests {
    use crate::gui::widgets::{split_row, stat_chip};
    use eframe::egui::{self, Color32, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻真实结构：CentralPanel(内边距 18,14) → 统计芯片行(horizontal_wrapped)
    /// → 表头 split_row。量的是"表头右组到底钉在哪"，以及"芯片行有没有绕过右边界"。
    fn probe(window: Vec2, chips: usize) -> (f32, f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, window));
        let mut panel_right = 0.0_f32;
        let mut group_right = 0.0_f32;
        let mut chips_right = 0.0_f32;
        let mut save_right = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(Color32::WHITE))
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
                        .fill(Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        stat_chip(ui, "订阅源", "36", Color32::GRAY);
                        stat_chip(ui, "启用", "36", Color32::GRAY);
                        stat_chip(ui, "节点", "13840", Color32::GRAY);
                        stat_chip(ui, "已测", "5304", Color32::GRAY);
                        stat_chip(ui, "可用", "159", Color32::GRAY);
                        for i in 0..chips {
                            stat_chip(ui, &format!("协议{i}"), "8898", Color32::GRAY);
                        }
                    });
                    chips_right = ui.min_rect().max.x;
                    // 关键：芯片行溢出之后，max_rect 自己有没有被撑大？
                    panel_right = ui.max_rect().right();
                    ui.add_space(10.0);
                    split_row(
                        ui,
                        "settings:header",
                        300.0,
                        |ui| {
                            let _ = ui.label("系统设置");
                        },
                        |ui| {
                            let save = ui.button("保存设置");
                            save_right = save.rect.max.x;
                            group_right = ui.min_rect().max.x;
                        },
                    );
                });
        });
        (
            panel_right,
            group_right.max(save_right),
            chips_right,
            window.x,
        )
    }

    /// 芯片行**一行放不下就得换行**：`Frame::show` 版的芯片不参与换行判定，整行冲出
    /// 右边界还会把父 ui 的 `max_rect` 撑大（实测 1535 > 面板 1312），于是下面所有
    /// "钉右"的表头都跟着画到屏幕外 —— 2026-10-15 他报的"保存设置只看得见一半"。
    #[test]
    fn the_chips_row_never_widens_the_page_it_sits_on() {
        for chips in [0usize, 3, 6, 9, 12] {
            let (panel_right, group_right, chips_right, _) = probe(Vec2::new(1330.0, 800.0), chips);
            assert!(
                chips_right <= panel_right + 0.5,
                "{chips} 个协议芯片时统计行画到 {chips_right:.1}，越过面板右边界 {panel_right:.1}（不换行就会撑大 max_rect）"
            );
            assert!(
                (group_right - panel_right).abs() < 1.0,
                "{chips} 个芯片时表头右组在 {group_right:.1}，没贴住面板右边界 {panel_right:.1}"
            );
        }
    }
}
#[cfg(test)]
mod slider_tick_probe_tests {
    use super::{font_scale_ticks, handle_travel_range, FONT_SCALE_MAX, FONT_SCALE_MIN};
    use eframe::egui::{self, Frame, RawInput, Rect, Vec2};

    /// 真加一个 Slider，读出 egui **自己画出来的手柄中心**，再和我们的刻度 x 对账。
    /// 返回每个百分比对应的（手柄中心 x, 刻度 x, 误差）。
    fn probe(value: i32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(900.0, 600.0),
        ));
        let mut handle_center = 0.0_f32;
        let mut tick_x = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(Frame::NONE.inner_margin(8.0))
                .show(ctx, |ui| {
                    let layer = ui.layer_id();
                    let slider_width = ui.spacing().slider_width;
                    ui.spacing_mut().slider_width = 300.0;
                    let mut percent = value;
                    let response = ui.add(egui::Slider::new(&mut percent, 85..=140).step_by(5.0));
                    ui.spacing_mut().slider_width = slider_width;
                    let travel = handle_travel_range(ui, &response, 300.0);
                    tick_x = font_scale_ticks(travel)
                        .into_iter()
                        .find(|(v, _, _)| *v == value)
                        .map(|(_, x, _)| x)
                        .unwrap_or(f32::NAN);
                    // 手柄是这一帧里唯一那个"高 ≈ 滑杆高、宽 ≈ 2×radius"的图元。
                    let shapes = ctx.graphics_mut(|g| {
                        g.get(layer)
                            .map(|list| list.all_entries().cloned().collect::<Vec<_>>())
                            .unwrap_or_default()
                    });
                    let radius = response.rect.height() / 2.5;
                    let mut best: Option<f32> = None;
                    for shape in shapes {
                        match shape.shape {
                            egui::Shape::Circle(circle) => {
                                if (circle.radius - radius).abs() < 2.0 {
                                    best = Some(circle.center.x);
                                }
                            }
                            egui::Shape::Rect(rect) => {
                                let w = rect.rect.width();
                                let h = rect.rect.height();
                                if (w - radius * 2.0).abs() < 3.0
                                    && (h - response.rect.height()).abs() < 3.0
                                {
                                    best = Some(rect.rect.center().x);
                                }
                            }
                            _ => {}
                        }
                    }
                    handle_center = best.unwrap_or(f32::NAN);
                });
        });
        (handle_center, tick_x)
    }

    #[test]
    fn the_ticks_sit_under_the_handle_egui_draws() {
        let mut lines = Vec::new();
        let mut worst = 0.0_f32;
        for value in [FONT_SCALE_MIN, 90, 100, 120, FONT_SCALE_MAX] {
            let (handle, tick) = probe(value);
            let diff = (handle - tick).abs();
            worst = worst.max(diff);
            lines.push(format!(
                "{value}%: 手柄 {handle:.1} 刻度 {tick:.1} 差 {diff:.1}"
            ));
        }
        eprintln!("{}", lines.join(" | "));
        assert!(
            worst < 1.5,
            "刻度与手柄最大偏差 {worst:.1}px / {}",
            lines.join(" | ")
        );
    }
}

#[cfg(test)]
mod local_time_tests {
    use crate::db::telemetry::switch_day;
    use crate::gui::widgets::clock_of;

    /// 出口切换记录里的时间必须是**本地（北京）时间**，不是库里那份 UTC。
    /// 断言按当前机器的时区算，不写死 +8 —— 否则 CI（UTC）上这条会假红。
    #[test]
    fn switch_times_are_shown_local_not_utc() {
        for raw in [
            "2026-10-14T19:24:05.696567800+00:00",
            "2026-10-14T19:24:05+00:00",
            "2026-10-14T19:24:05.123456+00:00",
        ] {
            let parsed = chrono::DateTime::parse_from_rfc3339(raw)
                .unwrap_or_else(|error| panic!("{raw} 解析不了: {error}"));
            let want = parsed
                .with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string();
            assert_eq!(clock_of(raw), want, "{raw} 没按本地时区显示");
            let day = parsed
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string();
            assert_eq!(switch_day(raw), day, "{raw} 的日头没按本地时区切");
        }
    }

    /// 库里万一混进非 RFC3339 的写法（SQLite 的 CURRENT_TIMESTAMP 就是空格版），
    /// 时间列也不许画出"2026-"这种半截日期。
    #[test]
    fn an_unparsable_timestamp_never_leaks_a_partial_date() {
        assert_eq!(clock_of("2026-10-14 19:24:05"), "19:24");
        assert_eq!(clock_of(""), "--:--");
        assert_eq!(clock_of("昨天下午"), "--:--");
    }
}

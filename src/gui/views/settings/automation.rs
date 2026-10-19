use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{card_frame, status_badge};
use crate::gui::VpnApp;
use eframe::egui::{self, RichText, Ui};

impl VpnApp {
    /// 订阅与测速：订阅源定时拉取策略、全协议测速并发与参数、落地剪枝优化。
    pub(crate) fn ui_settings_automation(&mut self, ui: &mut Ui) {
        let mut auto_save = false;

        // 1. 订阅源定时同步
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Subscription, 16.0, theme::accent());
                ui.label(RichText::new("订阅源定时同步").strong().size(15.0));
                let (badge_text, badge_color) = if self.edit_auto_fetch {
                    ("自动同步已开启", theme::accent_strong())
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
                    ("拉取联动测速开启", theme::accent_strong())
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


}

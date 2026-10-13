use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::types::BgMsg;
use crate::gui::widgets::{
    card_frame, normalize_api_base, primary_btn, section_title, status_badge };
use crate::gui::VpnApp;
use crate::proxy::ProxyState;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};
use crate::gui::theme;

impl VpnApp {
    pub(crate) fn ui_settings(&mut self, ui: &mut Ui) {
        // 两行表头。以前是"标题 + 末尾一个右对齐组"挤在同一行，长副标题会把"保存设置"
        // 推出窗口；而把右对齐组提到纵向 ui 上单独摆又会把整页高度吃光（egui 0.31 的
        // `with_layout` 就这么行为），所以：第一行标题，第二行 `horizontal` 里套右对齐组。
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("系统设置")
                    .strong()
                    .size(17.0)
                    .color(theme::text_primary()),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务")
                        .small()
                        .color(theme::muted()),
                )
                .truncate(),
            );
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let dirty = self.settings_dirty();
            if ui
                .add_enabled(!self.settings_saving, primary_btn("保存设置"))
                .clicked()
            {
                self.save_settings();
            }
            if ui
                .button("重置修改")
                .on_hover_text("恢复为当前已保存的配置")
                .clicked()
            {
                self.sync_edit_settings();
                self.reload_settings();
            }
            if self.settings_saving {
                ui.spinner();
                ui.label(RichText::new("正在保存...").color(theme::accent()).small());
            } else if dirty {
                ui.label(
                    RichText::new("⚠ 有未保存修改")
                        .color(theme::warning())
                        .small(),
                );
            } else {
                ui.label(RichText::new("✓ 已同步").color(theme::success()).small());
            }
            });
        });
        ui.add_space(6.0);

        // 设置卡片总高度超过窗口，整页必须可纵向滚动，否则底部内容看不到。
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| self.ui_settings_cards(ui));
    }

    fn ui_settings_cards(&mut self, ui: &mut Ui) {
        // 高级设置默认收起：那些数值调参只在出问题才会动，常驻在页面上只是挡路。
        // 收起时留着的是：自动拉取、同步周期、故障切换阈值、主题、防火墙放行。
        let mut advanced = self.settings_advanced_open;
        ui.horizontal(|ui| {
            if ui.checkbox(&mut advanced, "显示高级设置").changed() {
                self.settings_advanced_open = advanced;
            }
            if !advanced {
                ui.label(
                    RichText::new("只留常用的 5 类；订阅并发、测速参数、测速优化这些收在里面")
                        .small()
                        .color(theme::muted()),
                );
            }
        });
        ui.add_space(6.0);
        let mut auto_save = false;

        // ==========================================
        // 1. 常规偏好与窗口行为 (General & Window Behavior)
        // ==========================================
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Gear, 16.0, theme::accent());
                ui.label(RichText::new("常规偏好与窗口行为").strong().size(15.0));
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("管理桌面应用程序的关闭逻辑、托盘常驻行为以及系统物理内存回收。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            // 2026-10-08 他要的「预备状态」：新装的机器上一次内核都没有，软件自己
            // 就该把 mihomo 下下来，不用人再点一遍下载。关掉它就不会自己动。
            if ui
                .checkbox(
                    &mut self.edit_auto_prepare_kernel,
                    "预备状态（一个内核都没装）时自动下载 mihomo 内核",
                )
                .on_hover_text(
                    "用你在下载内核时选的那种方式下载；一次运行只自己动一次，失败了不反复重试。                     关掉它就只在你点「下载内核」时才下。",
                )
                .changed()
            {
                auto_save = true;
            }
            ui.add_space(6.0);

            // Setting row 1: Close window behavior
            ui.label(RichText::new("点击主窗口关闭按钮 (X) 时：").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
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
            });
            ui.horizontal(|ui| {
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
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Lightbulb, 12.0, theme::muted());
                ui.label(
                    RichText::new("提示：最小化到托盘后，可通过任务栏托盘图标右键菜单退出程序，或双击托盘图标/再次运行程序唤出界面。")
                        .small()
                        .color(theme::muted()),
                );
            });

            ui.add_space(8.0);
            ui.label(RichText::new("界面配色主题").strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new("点击即可即时切换界面主题配色，自动保存并在下次启动时沿用。")
                    .small()
                    .color(theme::muted()),
            );
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

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Setting row 2: Memory trim
            // 描述与按钮分两行：横向并排时长描述会把按钮顶出可视区（裁掉后看不见）。
            ui.label(RichText::new("进程内存深度整理").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "主动释放 SQLite 缓存连接池与 Windows 物理工作集，显著降低常驻内存。",
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
        });

        ui.add_space(10.0);

        // ==========================================
        // 2. 订阅自动拉取 (Subscription Automation)
        // ==========================================
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Subscription, 16.0, theme::accent());
                ui.label(RichText::new("订阅定时拉取").strong().size(15.0));
                let (badge_text, badge_color) = if self.edit_auto_fetch {
                    ("自动同步已开启", theme::success())
                } else {
                    ("已关闭", theme::muted())
                };
                status_badge(ui, badge_text, badge_color);
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("在后台周期性拉取已启用的订阅源，自动解析更新节点池。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(6.0);

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

            // 3 Column Grid for Parameters
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
                        ui.label(RichText::new("最小 5 分钟，建议 60").small().color(theme::muted()));
                    });
                });
                if self.settings_advanced_open {
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
                        ui.label(RichText::new("建议 2~4，避免上游封禁").small().color(theme::muted()));
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
                        ui.label(RichText::new("网络较差时可调高至 45").small().color(theme::muted()));
                    });
                });
                } else {
                    ui.label(RichText::new("已收起：订阅并发、单源请求超时")
                        .small()
                        .color(theme::muted()));
                }
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

        // ==========================================
        // 3. 节点测速管理 (Speedtest Configuration)
        // ==========================================
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
                RichText::new("配置全协议节点真实握手、TCP 预筛与下载带宽探测策略。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(6.0);

            if ui
                .checkbox(&mut self.edit_auto_speed, "订阅拉取完成后自动执行批量测速")
                .changed()
            {
                auto_save = true;
            }
            // 加完一个源就当场知道它好不好用：先拉一次，拉成功再只测这个源的节点。
            if ui
                .checkbox(
                    &mut self.edit_speedtest_added_source,
                    "添加订阅源后自动拉取它，并只测速这个源",
                )
                .on_hover_text(
                    "只对你手动新增的源生效（Gist/频道批量发现不触发，免得一次登记八个源就八轮测速）。                     测速并发用上面的设置值。",
                )
                .changed()
            {
                auto_save = true;
            }

            // 2026-10-08：故障转移的替补是从"测通"那批里捞的。他库里出现过 8349 个节点
            // 只有 15 个测通的情况 —— 那时自动切换等于没有退路。这个开关让后台在
            // 测通数低于 20 时自己补一轮全量测速（并发固定 8、不测下载吞吐）。
            if ui
                .checkbox(
                    &mut self.edit_speedtest_auto_refill,
                    "测通节点少于 20 个时，后台自动补一轮全量测速",
                )
                .on_hover_text(
                    "只在后台跑：并发固定 8（不占用你设置的测速并发），不探测下载吞吐，\
                     距上次自动测速至少隔 30 分钟才会再跑一轮。一轮可能要几十分钟。",
                )
                .changed()
            {
                auto_save = true;
            }
            // 系统代理归属：程序写的 `ProxyEnable` 会被 Windows 设置、别的代理工具或脚本改掉，
            // 之后界面仍然显示"代理运行中"（2026-10-08 实测过一遍）。核对一直在做，
            // 要不要"发现被关掉就自动写回"由这条决定 —— 默认关，因为它动的是用户 HKCU。
            if ui
                .checkbox(
                    &mut self.edit_auto_rearm_system_proxy,
                    "系统代理被外部关闭时，自动写回本程序的代理地址",
                )
                .on_hover_text(
                    "每 30 秒核对一次注册表。关掉时界面只提示不动手；开着才会把 \
                     ProxyEnable / ProxyServer 写回本程序的 127.0.0.1:混合端口。\
                     和别的代理工具共存时建议保持关闭，否则两边会互相抢。",
                )
                .changed()
            {
                auto_save = true;
            }
            if self.settings_advanced_open {
            if ui
                .checkbox(
                    &mut self.edit_speedtest_download,
                    "额外探测节点真实下载速度（进行真实数据包吞吐测试，耗时较长）",
                )
                .changed()
            {
                auto_save = true;
            }
            ui.add_space(6.0);

            // 3 Column Grid for Parameters
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
            } else {
                ui.label(RichText::new("已收起：下载吞吐探测、测速并发、连接超时阈值、下载探测时长")
                    .small()
                    .color(theme::muted()));
            }

            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "说明：所有节点测速均通过当前代理内核完成握手和 Google Generate 204 访问；批量测速会自动进行 TCP 预筛以提升整体效率。导出功能只读取既有测速结果，不会触发额外网络请求。",
                )
                .small()
                .color(theme::muted()),
            );
            if let Some(t) = self.settings.last_auto_speedtest_at {
                ui.add_space(2.0);
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

        ui.add_space(10.0);

        // ==========================================
        // 4. 代理自愈与服务接口 (Runtime & Network Services)
        // ==========================================
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Shield, 16.0, theme::accent());
                ui.label(RichText::new("代理自愈与网络服务").strong().size(15.0));
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
                RichText::new("管理断连自动切换 (Failover)、分流规则数据库以及供外部调用的 RESTful API。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            // Sub-item 1: Failover
            if ui
                .checkbox(
                    &mut self.auto_failover,
                    "节点故障自动切换（Failover）：每 60 秒经当前代理实测一次，交给选路调度器判定",
                )
                .on_hover_text(
                    "调度器规则：EWMA 平滑延迟；连续 2 次验证失败开断路器并拉黑 5 分钟；
                     候选必须比当前节点好 25% 以上才切（滞后），且两次切换至少隔 2 分钟。
                     切之前会对待补节点先做一次 TCP 复检，不再拿过期测速数据直接跳。",
                )
                .changed()
            {
                let on = self.auto_failover;
                self.set_auto_failover(on);
                self.status = if on {
                    "节点故障自动切换已开启，这个选择会保存，重启后仍然生效".into()
                } else {
                    "节点故障自动切换已关闭（探测照跑，只是不自动换出口）".into()
                };
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("切换延迟阈值:").small().color(theme::muted()));
                ui.add(
                    egui::DragValue::new(&mut self.edit_failover_latency_ms)
                        .range(200..=15_000)
                        .speed(50)
                        .suffix(" ms"),
                );
                ui.label(
                    RichText::new(
                        "实测是穿过当前代理的一次完整 HTTPS 请求，境外节点正常也在 300-900ms；\
                         超过阈值需连续三次才切换（验证失败立即切，两次自动切换间隔至少 3 分钟）",
                    )
                    .small()
                    .color(theme::muted()),
                );
            });
            if self.settings_advanced_open {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.edit_bandwidth_guard, "带宽保护");
                ui.add(
                    egui::DragValue::new(&mut self.edit_bandwidth_min_kbps)
                        .range(0..=10000)
                        .speed(10)
                        .suffix(" KB/s"),
                );
                ui.label(
                    RichText::new("当前出口下行持续低于该值也算劣化（需先勾选上面的自动切换）")
                        .small()
                        .color(theme::muted()),
                );
            });
            } else {
                ui.label(RichText::new("已收起：带宽保护（下行低于该值也算劣化）")
                    .small()
                    .color(theme::muted()));
            }
            if let Some(reason) = &self.failover_last_reason {
                ui.label(RichText::new(format!("最近一次: {reason}")).small().color(theme::accent()));
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // 节点关键词黑名单
            ui.label(RichText::new("节点关键词排除").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "多个词用逗号、分号或空格分隔（大小写不敏感，子串匹配）。命中的节点不会成为“一键连接最优”和自动切换的候选，也不参与推送与导出；节点列表仍可见。",
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
            crate::gui::widgets::note_text_field(ui, &exclude_field);

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            if self.settings_advanced_open {
            // Sub-item: 测速优化。每一项都会改变严格程度或资源占用，默认全部关闭。
            ui.label(RichText::new("测速优化（默认全部关闭）").strong());
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "关闭时行为与之前完全一致：内核串行验证、每批冷启动新内核、每轮全部重测、                         失败节点一律隔离复测。",
                    )
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            ui.checkbox(
                &mut self.edit_speedtest_parallel_kernels,
                "三个内核并行验证（墙钟约 1/3，峰值内存约 3 倍）",
            );
            ui.checkbox(
                &mut self.edit_speedtest_persistent_kernel,
                "复用常驻内核，用控制器接口切换待测节点（省掉每批 1-3 秒冷启动）",
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("沿用已验证结果:").small().color(theme::muted()));
                ui.add(
                    egui::DragValue::new(&mut self.edit_speedtest_reuse_minutes)
                        .range(0..=1440)
                        .speed(5)
                        .suffix(" 分钟"),
                );
                ui.label(RichText::new("0 = 每轮全部重测").small().color(theme::muted()));
            });
            ui.checkbox(
                &mut self.edit_speedtest_dedup_egress,
                "同物理落地剪枝 (落地去重)",
            )
            .on_hover_text("多源或同源多个节点解析到相同目标 IP:Port 时，只对 1 个物理代表节点进行真实内核握手并共享结果，大幅提速并防止打死 VPS");

            ui.checkbox(
                &mut self.edit_speedtest_conditional_isolation,
                "只对可疑的失败节点做 mihomo -t 隔离复测（默认全部复测）",
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("下载测速只做延迟前:").small().color(theme::muted()));
                ui.add(
                    egui::DragValue::new(&mut self.edit_speedtest_download_top_n)
                        .range(0..=1000)
                        .speed(5)
                        .suffix(" 个"),
                );
                ui.label(
                    RichText::new("0 = 所有 TCP 可达节点都测（很慢）；需先勾选上方“测下载速率”")
                        .small()
                        .color(theme::muted()),
                );
            });
            } else {
                ui.label(RichText::new("已收起：三内核并行、常驻内核、沿用已验证结果、条件隔离复测、下载测速 TopN")
                    .small()
                    .color(theme::muted()));
            }

            // Sub-item 2: Geo Rules
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

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            // Sub-item 3: HTTP API Server
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

        ui.add_space(14.0);

        // ==========================================
        // 5. Windows 防火墙放行 (Firewall Allow Rules)
        // ==========================================
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Shield, 16.0, theme::accent());
                ui.label(RichText::new("Windows 防火墙放行").strong().size(15.0));
                let (badge_text, badge_color) = if self.edit_firewall_auto_allow {
                    ("启动时自动放行", theme::success())
                } else {
                    ("已关闭", theme::muted())
                };
                status_badge(ui, badge_text, badge_color);
            });
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "防火墙会拦掉发到本程序与内核监听端口的回环连接（表现为代理点了没反应、9980 无响应）。规则按程序路径放行，重编译或换内核版本后路径不变，规则照样有效。",
                    )
                    .small()
                    .color(theme::muted()),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            if ui
                .checkbox(
                    &mut self.edit_firewall_auto_allow,
                    RichText::new("软件启动时自动加入允许应用").size(13.0),
                )
                .changed()
            {
                auto_save = true;
                // 勾上就立刻试一次，让他当场看到成没成；非管理员只记住，提权启动时再补。
                if self.edit_firewall_auto_allow && crate::platform::is_elevated() {
                    self.apply_firewall_rules();
                }
            }
            ui.add_space(6.0);
            let elevated = crate::platform::is_elevated();
            ui.label(
                RichText::new(if elevated {
                    "当前是管理员权限，可以立即写入规则"
                } else {
                    "当前非管理员：勾选只记住，提权启动时再自动补上"
                })
                .small()
                .color(if elevated {
                    theme::success()
                } else {
                    theme::warning()
                }),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(elevated, primary_btn("现在加入"))
                    .on_hover_text("对本程序和已下载到 managed kernels 目录的内核 exe 逐条写入允许规则")
                    .clicked()
                {
                    self.apply_firewall_rules();
                }
                if let Some(status) = self.firewall_status.clone() {
                    ui.label(RichText::new(status).small().color(theme::text_secondary()));
                }
            });
        });

        ui.add_space(14.0);

        // ==========================================
        // 底部保存与同步状态栏 (Bottom Action Bar)
        // ==========================================
        ui.horizontal(|ui| {
            let dirty = self.settings_dirty();
            if ui
                .add_enabled(!self.settings_saving, primary_btn("保存所有设置"))
                .clicked()
            {
                self.save_settings();
            }
            if ui.button("重新加载").clicked() {
                self.sync_edit_settings();
                self.reload_settings();
            }
            if self.settings_saving {
                ui.spinner();
                ui.label(RichText::new("正在保存到数据库...").color(theme::accent()).small());
            } else if dirty {
                ui.label(
                    RichText::new("⚠ 有未保存修改（切换页面将自动保存）")
                        .color(theme::warning())
                        .small(),
                );
            } else {
                ui.label(RichText::new("✓ 设置已保存并同步").color(theme::success()).small());
            }
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }

    pub(crate) fn ui_api(&mut self, ui: &mut Ui) {
        section_title(ui, "HTTP API", "点击路径复制完整 URL · GET 可浏览器打开");
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
                ui.label(RichText::new("本地 HTTP API 服务当前已停止（端口已释放）").color(theme::muted()));
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

#[cfg(test)]
mod layout_tests {
    use eframe::egui::{self, Label, RawInput, Rect, Vec2};

    /// 画一段表头，然后报告纵向 ui 里还剩多少高度给下面的卡片。
    fn height_left_after(header: impl Fn(&mut egui::Ui)) -> f32 {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0)));
        let mut left = -1.0;
        ctx.run(input, |ctx| {
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
                ui.button("保存设置");
                ui.button("重置修改");
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
                    ui.button("保存设置");
                    ui.button("重置修改");
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
    /// 返回（窗口右边界, 保存设置按钮右边界, 芯片行右边界）。
    fn probe(window_width: f32, chips: usize, chips_before_header: bool) -> (f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 890.0),
        ));
        let mut save_right = 0.0_f32;
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
                    let _ = ui.horizontal(|ui| {
                        let _ = ui.label("系统设置");
                        let _ = ui.label("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务");
                    });
                    let _ = ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let save = ui.button("保存设置");
                            save_right = save.rect.max.x;
                            let _ = ui.button("重置修改");
                            let _ = ui.label("已同步");
                        });
                    });
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
        (window_width, save_right, chips_right)
    }

    /// 表头那个「保存设置」不许越过中央面板的右边界，也不管它前面那排统计芯片
    /// 换不换行、窗口多窄。用户 2026-10-07 报的是真机上看按钮被窗口右边裁掉，
    /// 这条把"布局算出来的右边界必须在窗口内"钉住 —— 一旦哪天又靠到边外，测试先红。
    #[test]
    fn save_button_stays_inside_the_panel_at_every_width() {
        for (width, chips, before) in [
            (1298.0, 11, true),
            (1298.0, 11, false),
            (1298.0, 3, true),
            (1000.0, 11, true),
            (760.0, 11, true),
        ] {
            let (w, save, row) = probe(width, chips, before);
            let limit = w - 18.0;
            assert!(
                save <= limit + 0.5,
                "窗口 {w:.0}（芯片 {chips} 个，表头前={before}）时保存设置画到 {save:.1}，越过面板右边界 {limit:.1}"
            );
            assert!(
                row <= limit + 0.5,
                "窗口 {w:.0} 时芯片行画到 {row:.1}，越过面板右边界 {limit:.1}"
            );
        }
    }
}

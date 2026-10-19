use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{card_frame, status_badge};
use crate::gui::VpnApp;
use crate::proxy::ProxyState;
use eframe::egui::{self, RichText, Ui};

impl VpnApp {
    /// 自愈网络：节点故障自动切换 (Failover)、系统代理看门狗、Geo 规则与本地 HTTP API 服务。
    pub(crate) fn ui_settings_network(&mut self, ui: &mut Ui) {
        let mut auto_save = false;

        // 1. 节点故障自愈与系统代理守护
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Shield, 16.0, theme::accent());
                ui.label(RichText::new(self.tr("节点故障自愈与代理守护")).strong().size(15.0));
                let (badge_text, badge_color) = if self.auto_failover {
                    (self.tr("自动切换已开启"), theme::accent_strong())
                } else {
                    (self.tr("自动切换已关闭"), theme::muted())
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
                    ("HTTP 服务运行中", theme::accent_strong())
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
                        !self.geo_updating,
                        egui::Button::new("在线更新 Geo 数据库"),
                    )
                    .on_hover_text(if running {
                        "向当前代理内核发送指令，在线拉取并热重载最新的 Geo 规则数据库"
                    } else {
                        "直连极速 CDN 高速拉取并保存最新的 GeoIP 与 GeoSite 数据库（无需开启代理）"
                    })
                    .clicked()
                {
                    self.update_geo_data();
                }
                if self.geo_updating {
                    ui.spinner();
                    ui.label(RichText::new("正在高速下载更新 Geo 规则...").small().color(theme::accent()));
                } else if !running {
                    ui.label(RichText::new("（直连极速 CDN，无需开启代理）").small().color(theme::muted()));
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


}

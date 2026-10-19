use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{card_frame, primary_btn, status_badge};
use crate::gui::VpnApp;
use eframe::egui::{self, RichText, Ui};

impl VpnApp {
    /// 通用设置：桌面窗口生命周期、托盘行为、系统物理内存释放与 Windows 防火墙规则。
    pub(crate) fn ui_settings_general(&mut self, ui: &mut Ui) {
        let mut auto_save = false;

        // 1. 桌面窗口与生命周期
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Gear, 16.0, theme::accent());
                ui.label(RichText::new(self.tr("桌面窗口与运行模式")).strong().size(15.0));
                let tray_badge = if self.edit_close_to_tray {
                    (self.tr("托盘常驻"), theme::accent_strong())
                } else {
                    (self.tr("直接退出"), theme::muted())
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

            ui.label(RichText::new(self.tr("点击主窗口关闭按钮 (X) 时：")).strong());
            ui.add_space(4.0);
            let tr_minimize = self.tr("最小化到系统托盘 (推荐：后台保持代理与定时自动化运行)");
            let tr_exit = self.tr("直接退出程序 (停止所有网络连接并彻底退出软件)");
            if ui
                .radio_value(
                    &mut self.edit_close_to_tray,
                    true,
                    tr_minimize,
                )
                .changed()
            {
                auto_save = true;
            }
            if ui
                .radio_value(
                    &mut self.edit_close_to_tray,
                    false,
                    tr_exit,
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
                    "提示：当前系统环境（Linux / Wayland）已接入单实例保护与标准桌面管理；若重复启动将自动激活唤醒已有窗口。"
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
                    ("管理员权限", theme::accent_strong())
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
                    ui.label(RichText::new(status).small().color(theme::accent_strong()));
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


}

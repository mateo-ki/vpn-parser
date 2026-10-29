use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{card_frame, primary_btn, status_badge};
use crate::gui::VpnApp;
use eframe::egui::{RichText, Ui};

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

            ui.add_space(6.0);

            // 以管理员方式启动软件：只落库，下次启动时自动按配置请求提权（UAC）
            let elevated_now = crate::platform::is_elevated();
            if ui
                .checkbox(
                    &mut self.edit_launch_as_admin,
                    "以管理员方式启动软件（TUN 模式推荐）",
                )
                .on_hover_text(
                    "勾选仅保存设置，不影响当前运行的实例；下次启动软件时自动请求管理员权限\
                     （弹出 UAC 授权框，取消则本次以普通权限运行）。TUN 虚拟网卡模式需要管理员权限。",
                )
                .changed()
            {
                auto_save = true;
                self.status = if self.edit_launch_as_admin {
                    "已保存：下次启动软件时将以管理员权限运行（届时弹出 UAC 授权框）".into()
                } else {
                    "已关闭「以管理员方式启动」，下次启动以普通权限运行".into()
                };
            }
            if self.edit_launch_as_admin && elevated_now {
                ui.label(
                    RichText::new("当前已以管理员权限运行")
                        .small()
                        .color(theme::accent_strong()),
                );
            } else if self.edit_launch_as_admin && crate::platform::admin_relaunch_cancelled() {
                ui.label(
                    RichText::new("本次启动的提权被取消，当前以普通权限运行；下次启动会再次请求")
                        .small()
                        .color(theme::warning()),
                );
            }

            ui.add_space(6.0);

            // 开机自启动：默认关（写的是用户注册表，不能擅自开）。
            // 保存设置时同步 HKCU Run 键；每次启动还会对账一次（exe 挪了位置能自己跟上）。
            if ui
                .checkbox(
                    &mut self.edit_launch_at_startup,
                    "开机自动启动（登录后自动在后台运行本软件）",
                )
                .on_hover_text(
                    "写入当前用户的注册表 Run 键（不需要管理员权限），登录 Windows 后自动启动。\
                     仅指向当前 exe 路径，软件挪动位置后下次启动会自动校正。目前仅 Windows 生效。",
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
            ui.add_space(8.0);

            ui.label(RichText::new("进程内存深度整理").strong());
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
                }
            });
        });

        ui.add_space(10.0);

        // 3. 配置备份与全量迁移 (Profile Backup & Migration)
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Export, 16.0, theme::accent());
                ui.label(RichText::new("统一配置备份与全量迁移 (Unified Profile)").strong().size(15.0));
                status_badge(ui, "跨端归一化", theme::accent_strong());
            });
            ui.add_space(8.0);

            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                if ui
                    .add(icons::button_with_icon(Icon::Package, "一键备份到本地", theme::accent()))
                    .on_hover_text("导出 YAML 配置并保存至 data/backups/，同时自动复制到剪贴板")
                    .clicked()
                {
                    self.export_profile_to_backup();
                }

                if ui
                    .add(icons::button_with_icon(Icon::Export, "另存为配置文件...", theme::accent()))
                    .on_hover_text("弹出系统文件选择框，将当前统一配置另存为指定 .yaml 配置文件")
                    .clicked()
                {
                    self.export_profile_to_file();
                }

                if ui
                    .add(icons::button_with_icon(Icon::Folder, "从文件导入还原...", theme::accent()))
                    .on_hover_text("选取本地 YAML / JSON 配置文件无损还原订阅源、分流规则与自定义节点")
                    .clicked()
                {
                    self.import_profile_from_file();
                }

                if ui
                    .add(icons::button_with_icon(Icon::Clipboard, "从剪贴板还原", theme::accent()))
                    .on_hover_text("从剪贴板读取统一配置文本并解析恢复")
                    .clicked()
                {
                    self.import_profile_from_clipboard();
                }
            });

            if let Some((ref status, is_err)) = self.profile_status {
                ui.add_space(8.0);
                let color = if is_err { theme::danger() } else { theme::success() };
                ui.horizontal(|ui| {
                    icon_label(ui, if is_err { Icon::Alert } else { Icon::Check }, 14.0, color);
                    ui.label(RichText::new(status).small().color(color));
                });
            }
        });

        if auto_save && self.settings_dirty() {
            self.save_settings();
        }
    }


}

use crate::app_update::{self, AppUpdateProgress, AppUpdateStage};
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::BgMsg;
use crate::gui::widgets::{card_frame, primary_btn};
use crate::gui::VpnApp;
use eframe::egui::{self, CornerRadius, ProgressBar, RichText, Stroke, Ui, Vec2};

impl VpnApp {
    pub(crate) fn ui_about(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("关于")
                    .strong()
                    .size(17.0)
                    .color(theme::text_primary()),
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new("本机版本 · 项目信息 · 在线更新")
                        .small()
                        .color(theme::muted()),
                )
                .truncate(),
            );
        });
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.ui_about_identity(ui);
                ui.add_space(10.0);
                self.ui_about_update(ui);
            });
    }

    fn ui_about_identity(&mut self, ui: &mut Ui) {
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Info, 18.0, theme::accent());
                ui.label(
                    RichText::new(format!("VPN Parser v{}", app_update::current_version()))
                        .strong()
                        .size(15.0),
                );
                ui.label(
                    RichText::new("开源 · MIT 许可")
                        .small()
                        .color(theme::muted()),
                );
            });
            ui.add_space(6.0);
            info_row(ui, "项目仓库", "https://github.com/mateo-ki/vpn-parser");
            info_row(
                ui,
                "安装包发布",
                "GitHub Releases（由 tag 触发 Actions 构建）",
            );
            info_row(ui, "程序目录", describe(current_exe_dir()));
            info_row(ui, "数据目录", describe(data_dir()));
            info_row(
                ui,
                "内核目录",
                describe(Some(crate::kernel::managed_kernel_root())),
            );
            info_row(
                ui,
                "更新包暂存",
                describe(Some(app_update::installer_cache_dir())),
            );
            ui.add_space(4.0);
            if ui
                .small_button("打开更新包暂存目录")
                .on_hover_text("下载过的安装包就放在这里，装完可以自己删")
                .clicked()
            {
                open_in_file_manager(&app_update::installer_cache_dir());
            }
        });
    }

    fn ui_about_update(&mut self, ui: &mut Ui) {
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Sync, 18.0, theme::accent());
                ui.label(RichText::new("检查更新").strong().size(15.0));
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new("从 GitHub Releases 取最新安装包，SHA-256 校验通过才允许安装。")
                    .small()
                    .color(theme::muted()),
            );
            ui.add_space(8.0);

            match &self.app_release {
                Some(info) => {
                    let color = if info.update_available {
                        theme::accent()
                    } else {
                        theme::success()
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&info.message).strong().color(color));
                        if info.prerelease {
                            crate::gui::widgets::status_badge(ui, "预发版", theme::warning());
                        }
                    });
                    info_row(
                        ui,
                        "版本",
                        format!("v{} → {}", info.current_version, info.tag),
                    );
                    if let Some(asset) = &info.asset {
                        info_row(ui, "安装包", &asset.name);
                        info_row(ui, "大小", &crate::gui::widgets::format_bytes(asset.size));
                        info_row(
                            ui,
                            "SHA-256",
                            asset
                                .sha256
                                .as_deref()
                                .map(|digest| &digest[..digest.len().min(16)])
                                .map(|head| format!("{head}…"))
                                .unwrap_or_else(|| "待从 sha256.txt 获取".to_string()),
                        );
                    }
                }
                None => {
                    ui.label(
                        RichText::new("还没有检查过更新。")
                            .small()
                            .color(theme::muted()),
                    );
                }
            }

            if let Some(progress) = &self.app_update_progress {
                ui.add_space(8.0);
                self.ui_app_update_progress(ui, progress);
            }

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.app_update_busy, primary_btn("检查更新"))
                    .on_hover_text("只查版本号，不下载任何东西")
                    .clicked()
                {
                    self.check_app_update();
                }
                if let Some(info) = &self.app_release {
                    let downloadable = info.downloadable();
                    if ui
                        .add_enabled(!self.app_update_busy && downloadable, button("下载更新"))
                        .on_hover_text(if downloadable {
                            "下载安装包并校验，下载完再点「安装」"
                        } else {
                            "已经是最新版，或这个 Release 里没有安装包"
                        })
                        .clicked()
                    {
                        self.download_app_update();
                    }
                }
                if let Some(path) = self.app_installer_path.clone() {
                    if ui
                        .add_enabled(!self.app_update_busy, button("安装"))
                        .on_hover_text(
                            "启动安装向导。安装包一启动就会结束当前这个 vpn-parser.exe，\
                             装完由向导把程序带回来。",
                        )
                        .clicked()
                    {
                        self.install_app_update(&path);
                    }
                }
                if self.app_update_busy {
                    ui.spinner();
                    if ui.button("取消").clicked() {
                        self.app_update_cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
            ui.add_space(8.0);
            // 默认关：开着等于同意装"没跑稳、可能改数据结构"的包，所以不替人打开。
            let mut insider = self.edit_insider_channel;
            if ui
                .checkbox(&mut insider, "参与预发（检查更新时也看 beta）")
                .on_hover_text(
                    "稳定通道只问 GitHub 的 latest（它天生跳过预发布和草稿）；\
                     开了之后改问 releases 列表的第一条已发布版本。\
                     注意：预发包可能改本地数据结构，装了再退回稳定版不一定读得懂。",
                )
                .changed()
            {
                self.edit_insider_channel = insider;
                // 上一次查的是另一个通道的结果，留着会把"换通道"这件事演不出来。
                self.app_release = None;
                self.app_installer_path = None;
                self.app_update_progress = None;
                self.save_settings();
                self.status = format!(
                    "更新通道已切到{}，点「检查更新」按新通道查",
                    if insider { "预发" } else { "稳定" }
                );
            }
            if self.edit_insider_channel {
                ui.label(
                    RichText::new("当前按预发通道检查：会拿到标为 prerelease 的版本。")
                        .small()
                        .color(theme::warning()),
                );
            }
            if self.app_installer_path.is_some() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("安装包已在校验通过后落到本地，点「安装」才会动程序目录。")
                        .small()
                        .color(theme::muted()),
                );
            }
        });
    }

    fn ui_app_update_progress(&self, ui: &mut Ui, progress: &AppUpdateProgress) {
        ui.label(
            RichText::new(format!("{} · {}", progress.stage.label(), progress.message))
                .small()
                .color(theme::text_secondary()),
        );
        ui.add_space(3.0);
        if matches!(progress.stage, AppUpdateStage::Downloading) {
            ui.add(
                ProgressBar::new(progress.fraction())
                    .desired_width(ui.available_width().min(420.0))
                    .text(""),
            );
            ui.label(
                RichText::new(format!(
                    "{} / {} · {}",
                    crate::gui::widgets::format_bytes(progress.downloaded),
                    crate::gui::widgets::format_bytes(progress.total),
                    crate::gui::widgets::format_speed(progress.rate())
                ))
                .small()
                .color(theme::muted()),
            );
        }
    }

    pub(crate) fn check_app_update(&mut self) {
        let tx = self.tx.clone();
        let proxy = self.app_update_proxy();
        let insider = self.edit_insider_channel;
        self.app_update_busy = true;
        self.app_update_progress = Some(AppUpdateProgress {
            stage: AppUpdateStage::Checking,
            downloaded: 0,
            total: 0,
            bytes_per_second: 0.0,
            message: format!(
                "正在读取最新 Release（{}通道）",
                if insider { "预发" } else { "稳定" }
            ),
        });
        self.status = "正在检查更新...".into();
        self.rt.spawn(async move {
            let result = app_update::check(
                proxy.as_deref(),
                app_update::current_version(),
                app_update::UpdateChannel::from_insider_flag(insider),
            )
            .await
            .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::AppUpdateCheck(result));
        });
    }

    pub(crate) fn download_app_update(&mut self) {
        let Some(info) = self.app_release.clone() else {
            self.status = "先检查一次更新".into();
            return;
        };
        let tx = self.tx.clone();
        let proxy = self.app_update_proxy();
        let cancelled = self.app_update_cancel.clone();
        cancelled.store(false, std::sync::atomic::Ordering::Relaxed);
        self.app_update_busy = true;
        self.app_installer_path = None;
        self.app_update_progress = Some(AppUpdateProgress {
            stage: AppUpdateStage::Downloading,
            downloaded: 0,
            total: info.asset.as_ref().map(|asset| asset.size).unwrap_or(0),
            bytes_per_second: 0.0,
            message: "准备下载".into(),
        });
        self.status = format!("正在下载 {}...", info.tag);
        self.rt.spawn(async move {
            let progress_tx = tx.clone();
            let result =
                app_update::download(&info, proxy.as_deref(), &cancelled, &move |progress| {
                    let _ = progress_tx.send(BgMsg::AppUpdateProgress(progress));
                })
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::AppUpdateDownloaded(result));
        });
    }

    pub(crate) fn install_app_update(&mut self, path: &std::path::Path) {
        match app_update::launch_installer(path) {
            Ok(()) => {
                self.status = format!("安装向导已启动：{}", path.display());
                self.app_update_progress = Some(AppUpdateProgress {
                    stage: AppUpdateStage::Ready,
                    downloaded: 0,
                    total: 0,
                    bytes_per_second: 0.0,
                    message: "安装向导已接管，本程序马上会被关闭".into(),
                });
            }
            Err(error) => self.status = format!("启动安装程序失败: {error}"),
        }
    }

    /// 更新走不走代理，只看当下有没有在跑的应用内代理；系统代理候选由
    /// `proxy_candidates` 补齐，和应用内下载内核的判定一致。
    fn app_update_proxy(&self) -> Option<String> {
        self.proxy_status
            .local_proxy_active()
            .then(|| format!("http://127.0.0.1:{}", self.proxy_status.mixed_port))
    }
}

fn button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .corner_radius(CornerRadius::same(6))
        .min_size(Vec2::new(72.0, 26.0))
}

fn info_row(ui: &mut Ui, label: &str, value: impl Into<String>) {
    let value = value.into();
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small().color(theme::muted()).strong());
        ui.label(RichText::new(value).small().color(theme::text_secondary()));
    });
}

fn describe(path: Option<std::path::PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "未知".to_string())
}

fn current_exe_dir() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.to_path_buf())
}

fn data_dir() -> Option<std::path::PathBuf> {
    Some(crate::kernel::managed_kernel_root().parent()?.to_path_buf())
}

fn open_in_file_manager(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

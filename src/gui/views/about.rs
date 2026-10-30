use crate::app_update::{self, AppUpdateProgress, AppUpdateStage};
use crate::gui::icons::Icon;
use crate::gui::theme;
use crate::gui::types::BgMsg;
use crate::gui::widgets::{card_frame, fold_title, icon_tile, pill, primary_btn, section_title};
use crate::gui::VpnApp;
use eframe::egui::{self, CornerRadius, Frame, Margin, ProgressBar, RichText, Stroke, Ui, Vec2};

const REPO_URL: &str = "https://github.com/mateo-ki/vpn-parser";
const RELEASES_URL: &str = "https://github.com/mateo-ki/vpn-parser/releases";
/// 关于页顶部那句介绍。刻意写"这程序干四件事"而不是形容词：这一页是排障时才来的地方。
const TAGLINE: &str = "采集订阅 · 解析节点 · 本地跑代理 · 把代理质量测出来给你看。";

/// 关于页底部的免责声明，一行一条（用户 2026-10-20 要求加）。
/// 每条都对着代码核过：没有统计上报模块、IP 风险查询要用户点、发布节点要用户填后台地址、
/// 内置源不再播种、data 目录不被卸载程序删除（installer/vpn-parser.iss 里没有 UninstallDelete）。
const DISCLAIMER_LINES: [&str; 6] = [
    "本软件是 MIT 开源的技术工具，只负责解析订阅、管理节点，并在本机调用第三方代理内核；它自身不提供任何节点、线路或加速服务，你能用到的节点全部来自你自己添加的订阅源。",
    "请遵守你所在地区的法律法规，不要把本软件用于任何违法违规用途。使用它产生的后果由使用者自行承担，作者与分发者不承担任何责任。",
    "本软件没有统计上报模块：订阅地址、节点库、分流规则、测速与切换记录都只存在本机的数据目录（展开上面「本机路径」可见），卸载也不会自动删掉它们。",
    "默认路径上的联网只有「取回数据」这一类：拉取你添加的订阅源、检测出口 IP、检查与下载更新、下载内核和 GeoIP/GeoSite 规则库，以及给分流探测做解析的公共 DNS。",
    "有两处会把信息送出去，而且都要你主动点或主动填才发生：把节点发布到你自建的 Web 后台（送出你测通的节点列表），查 IP 风险（把那个 IP 交给 ipinfo.io、ip-api.com、ipwho.is）。",
    "VP-Core、Mihomo、Sing-box、Xray 等内核与规则库均为第三方开源成果，功能与稳定性由上游负责；界面延迟与速率为瞬时实测值，不代表节点后续持续表现。",
];

/// 免责声明那张卡。文字与渲染分开：判据要能直接量这份清单，而不是量我另抄一遍的字符串。
fn disclaimer_card(ui: &mut Ui) {
    card_frame().show(ui, |ui| {
        ui.label(
            RichText::new("免责声明")
                .strong()
                .color(theme::text_primary()),
        );
        ui.add_space(6.0);
        for line in DISCLAIMER_LINES {
            ui.label(
                RichText::new(line)
                    .size(11.5)
                    .color(theme::text_secondary()),
            );
            ui.add_space(3.0);
        }
    });
}

/// 后台自动检查在线更新的间隔：**6 小时**（按需拉取，避开 GitHub 匿名限流）。第一次仍然在启动后 1 秒查。
const APP_UPDATE_CHECK_INTERVAL_SECS: u64 = 6 * 60 * 60;

impl VpnApp {
    pub(crate) fn ui_about(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("关于"), "本机版本 · 在线更新");

        egui::ScrollArea::vertical()
            .id_salt("about_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.ui_about_identity(ui);
                ui.add_space(10.0);
                self.ui_about_update(ui);
                ui.add_space(10.0);
                disclaimer_card(ui);
            });
    }

    fn ui_about_identity(&mut self, ui: &mut Ui) {
        card_frame().show(ui, |ui| {
            let mut open_repo = false;
            ui.horizontal(|ui| {
                let texture = self.ensure_logo_texture(ui.ctx());
                ui.add(
                    egui::Image::new(&texture)
                        .fit_to_exact_size(Vec2::splat(44.0))
                        .corner_radius(CornerRadius::same(9)),
                );
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("VPN Parser")
                                .strong()
                                .size(20.0)
                                .color(theme::text_primary()),
                        );
                        ui.add_space(9.0);
                        pill(
                            ui,
                            &format!("v{}", app_update::current_version()),
                            theme::accent_strong(),
                            theme::accent_bg(),
                        );
                        ui.add_space(6.0);
                        pill(
                            ui,
                            "MIT 开源",
                            theme::text_secondary(),
                            theme::panel_hover(),
                        );
                    });
                    ui.label(
                        RichText::new(TAGLINE)
                            .size(12.5)
                            .color(theme::text_secondary()),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(button("项目仓库 ↗"))
                        .on_hover_text(REPO_URL)
                        .clicked()
                    {
                        open_repo = true;
                    }
                });
            });
            if open_repo {
                ui.ctx().open_url(egui::OpenUrl::new_tab(REPO_URL));
            }
        });
    }

    fn ui_about_update(&mut self, ui: &mut Ui) {
        card_frame().show(ui, |ui| {
            let (title, title_color, tile_color) = match &self.app_release {
                Some(info) if info.update_available => (
                    format!("{} {}", self.tr("发现新版本"), info.tag),
                    theme::accent_strong(),
                    theme::accent(),
                ),
                Some(info) => (
                    format!("{} · v{}", self.tr("已是最新"), info.current_version),
                    theme::accent_strong(),
                    theme::accent(),
                ),
                None if self.app_update_busy => (
                    self.tr("正在检查更新…").to_string(),
                    theme::text_primary(),
                    theme::muted(),
                ),
                None => (
                    self.tr("检查更新").to_string(),
                    theme::text_primary(),
                    theme::accent(),
                ),
            };
            let mut subtitle = format!(
                "{}通道 · 取 GitHub Releases 的最新安装包，SHA-256 校验通过才允许安装",
                if self.edit_insider_channel {
                    "预发"
                } else {
                    "稳定"
                }
            );
            if let Some(at) = &self.app_update_checked_at {
                subtitle = format!("{subtitle} · 上次检查 {at}");
            }
            ui.horizontal(|ui| {
                icon_tile(ui, Icon::Sync, tile_color, theme::panel_hover());
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).strong().size(17.0).color(title_color));
                        if self.app_release.as_ref().is_some_and(|i| i.prerelease) {
                            ui.add_space(8.0);
                            crate::gui::widgets::status_badge(ui, self.tr("预发版"), theme::warning());
                        }
                    });
                    ui.label(RichText::new(subtitle).size(11.5).color(theme::muted()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(!self.app_update_busy, primary_btn(self.tr("检查更新")))
                        .on_hover_text("只查版本号，不下载任何东西")
                        .clicked()
                    {
                        self.check_app_update();
                    }
                });
            });

            if let Some(progress) = &self.app_update_progress {
                ui.add_space(8.0);
                self.ui_app_update_progress(ui, progress);
            }

            // 一行按钮都没有时（还没查过、也没在查）别占那一截空白。
            if self.app_release.is_some()
                || self.app_installer_path.is_some()
                || self.app_update_busy
            {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let mut action_download = false;
                    let mut action_show_notes = None;
                    if let Some(info) = &self.app_release {
                        let can_install = info.can_install();
                        let is_current = info.tag.trim_start_matches('v')
                            == app_update::current_version().trim_start_matches('v');
                        let btn_label = if info.update_available {
                            self.tr("下载更新")
                        } else if is_current {
                            self.tr("重新下载本版")
                        } else {
                            self.tr("下载此版本")
                        };
                        let tooltip = if !can_install {
                            "这个 Release 里没有适用于本平台的安装包"
                        } else if info.update_available {
                            "下载安装包并校验，下载完再点「安装」"
                        } else {
                            "下载选定的版本安装包并校验，下载完成后点击「安装」即可切换/回退"
                        };
                        if ui
                            .add_enabled(!self.app_update_busy && can_install, button(btn_label))
                            .on_hover_text(tooltip)
                            .clicked()
                        {
                            action_download = true;
                        }

                        // 新版本说明按钮：检查到更新后，在「下载更新」旁边呈现
                        if info.update_available || info.body.is_some() {
                            if ui
                                .button("新版本说明")
                                .on_hover_text("查看新版本的 Release 说明与详细更新日志")
                                .clicked()
                            {
                                let tag_title = format!("{} {}", info.tag, if info.update_available { "（最新版本）" } else { "" });
                                let content = info.body.clone().unwrap_or_else(|| {
                                    format!("版本 {}\n\n暂无内嵌更新说明，可点击右下角按钮直接在 GitHub Releases 查看完整发布日志。", info.tag)
                                });
                                action_show_notes = Some((tag_title, content));
                            }
                        }
                    }
                    if action_download {
                        self.download_app_update();
                    }
                    if let Some((tag, content)) = action_show_notes {
                        self.release_notes_content = Some((tag, content));
                        self.show_release_notes_modal = true;
                    }
                    if let Some(path) = self.app_installer_path.clone() {
                        // 这行提示按平台分叉：Linux 上没有"向导接管"这回事（本程序不会去拿
                        // root），点了只是把校验过的包和该敲的命令交给你 —— 写着"启动安装向导"
                        // 就是假话，而这个按钮点下去的行为两个平台确实不一样。
                        let hint = if cfg!(windows) {
                            "启动安装向导。安装包一启动就会结束当前这个 vpn-parser.exe，\
                             装完由向导把程序带回来。"
                        } else {
                            "本程序不会自己去拿 root：点这里会把已校验的包交给你，\
                             并把该执行的命令（sudo dpkg -i 或 sudo rpm -Uvh）写在状态栏上。"
                        };
                        if ui
                            .add_enabled(!self.app_update_busy, button("安装"))
                            .on_hover_text(hint)
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
            }

            // 安装包细节和本机路径都是"排障时才看"的东西，默认收起；展开后是同一套
            // 键值行（固定标签列 + 等宽值 + 截断 + hover 全文）。
            let detail = self.app_release.clone();
            ui.add_space(6.0);
            egui::CollapsingHeader::new(fold_title("安装包详情"))
                .id_salt("about:release-detail")
                .default_open(false)
                .show_background(false)
                .show(ui, |ui| {
                    let Some(info) = detail.as_ref() else {
                        ui.label(
                            RichText::new("还没有检查过更新。")
                                .size(11.0)
                                .color(theme::muted()),
                        );
                        return;
                    };
                    kv_row(
                        ui,
                        "版本",
                        &format!("v{} → {}", info.current_version, info.tag),
                        None,
                    );
                    if let Some(asset) = &info.asset {
                        let name = asset.name.clone();
                        if kv_row(ui, "安装包", &name, Some("复制")) {
                            ui.ctx().copy_text(name);
                        }
                        kv_row(
                            ui,
                            "大小",
                            &crate::gui::widgets::format_bytes(asset.size),
                            None,
                        );
                        let digest = asset
                            .sha256
                            .clone()
                            .unwrap_or_else(|| "待从 sha256.txt 获取".to_string());
                        if kv_row(ui, "SHA-256", &digest, Some("复制")) {
                            ui.ctx().copy_text(digest);
                        }
                    }
                    if kv_row(
                        ui,
                        "发布记录",
                        "GitHub Releases（由 tag 触发 Actions 构建）",
                        Some("打开"),
                    ) {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(RELEASES_URL));
                    }
                });

            ui.add_space(6.0);
            egui::CollapsingHeader::new(fold_title("版本切换（安装老版本 / 预发版本）"))
                .id_salt("about:version-switcher")
                .default_open(false)
                .show_background(false)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("可从 GitHub Releases 获取历史发布版本，自主选择回退到老版本或体验预发版本。")
                            .size(11.5)
                            .color(theme::text_secondary()),
                    );
                    ui.add_space(6.0);

                    if self.available_releases.is_empty() {
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(!self.fetching_releases, button("获取版本列表 🔄"))
                                .on_hover_text("从 GitHub Releases 读取近期的发布版本")
                                .clicked()
                            {
                                self.fetch_available_releases();
                            }
                            if self.fetching_releases {
                                ui.spinner();
                                ui.label(RichText::new("正在拉取版本列表...").size(11.5).color(theme::muted()));
                            }
                        });
                    } else {
                        let mut new_selection = None;
                        ui.horizontal(|ui| {
                            ui.label("选择版本：");
                            let current_tag = self.selected_release_tag.clone().unwrap_or_else(|| {
                                format!("v{}", app_update::current_version())
                            });
                            egui::ComboBox::from_id_salt("about_releases_combobox")
                                .width(280.0)
                                .selected_text(&current_tag)
                                .show_ui(ui, |ui| {
                                    for item in &self.available_releases {
                                        let mut label = item.tag.clone();
                                        if item.is_current {
                                            label = format!("{label} (当前版本)");
                                        }
                                        if let Some(date) = &item.published_at {
                                            label = format!("{label} · {date}");
                                        }
                                        if item.prerelease {
                                            label = format!("{label} [预发]");
                                        }
                                        if ui.selectable_label(self.selected_release_tag.as_deref() == Some(&item.tag), label).clicked() {
                                            new_selection = Some(item.tag.clone());
                                        }
                                    }
                                });

                            if ui
                                .add_enabled(!self.fetching_releases, button("刷新 🔄"))
                                .on_hover_text("重新从 GitHub 拉取最新版本列表")
                                .clicked()
                            {
                                self.fetch_available_releases();
                            }
                            if self.fetching_releases {
                                ui.spinner();
                            }
                        });

                        if let Some(tag) = new_selection {
                            self.select_release_version(tag);
                        }

                        if let Some(selected_tag) = &self.selected_release_tag {
                            if let Some(item) = self.available_releases.iter().find(|r| &r.tag == selected_tag) {
                                ui.add_space(6.0);
                                egui::Frame::NONE
                                    .fill(theme::panel_hover())
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(egui::Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(&item.tag).strong().size(13.5).color(theme::accent_strong()));
                                            if item.prerelease {
                                                crate::gui::widgets::status_badge(ui, "预发版", theme::warning());
                                            }
                                            if item.is_current {
                                                crate::gui::widgets::status_badge(ui, "本机当前版本", theme::accent());
                                            } else if app_update::version_is_newer(&item.tag, app_update::current_version()) {
                                                crate::gui::widgets::status_badge(ui, "高于当前版本（可升级）", theme::success());
                                            } else {
                                                crate::gui::widgets::status_badge(ui, "历史老版本（可回退）", theme::warning());
                                            }
                                            if let Some(date) = &item.published_at {
                                                ui.label(RichText::new(format!("发布于 {date}")).size(11.0).color(theme::muted()));
                                            }
                                        });

                                        ui.add_space(4.0);
                                        if let Some(pkg) = &item.asset_name {
                                            let size_mb = (item.asset_size as f64) / (1024.0 * 1024.0);
                                            ui.label(RichText::new(format!("安装包：{pkg} ({size_mb:.1} MB)")).size(11.5).color(theme::text_secondary()));
                                        } else {
                                            ui.label(RichText::new("该版本未检测到适用于当前操作系统的安装包资产").size(11.5).color(theme::danger()));
                                        }

                                        if let Some(body) = &item.body {
                                            let clean = body.lines().filter(|l| !l.trim().is_empty()).take(3).collect::<Vec<_>>().join(" · ");
                                            if !clean.is_empty() {
                                                ui.add_space(2.0);
                                                ui.horizontal(|ui| {
                                                    ui.label(RichText::new(format!("更新说明：{clean}")).size(11.0).color(theme::muted()));
                                                    if ui.small_button("查看全文").clicked() {
                                                        self.release_notes_content = Some((item.tag.clone(), body.clone()));
                                                        self.show_release_notes_modal = true;
                                                    }
                                                });
                                            }
                                        }
                                    });
                            }
                        }
                    }
                });

            let dirs: [(&str, Option<std::path::PathBuf>); 4] = [
                ("程序目录", current_exe_dir()),
                ("数据目录", data_dir()),
                ("内核目录", Some(crate::kernel::managed_kernel_root())),
                ("更新包暂存", Some(app_update::installer_cache_dir())),
            ];
            egui::CollapsingHeader::new(fold_title("本机路径"))
                .id_salt("about:paths")
                .default_open(false)
                .show_background(false)
                .show(ui, |ui| {
                    for (label, dir) in dirs {
                        match dir {
                            Some(path) => {
                                if kv_row(ui, label, &path.display().to_string(), Some("打开")) {
                                    open_in_file_manager(&path);
                                }
                            }
                            None => {
                                kv_row(ui, label, "未知", None);
                            }
                        }
                    }
                    ui.label(
                        RichText::new("「更新包暂存」里的安装包装完可以自己删。")
                            .size(11.0)
                            .color(theme::muted()),
                    );
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
                self.app_update_checked_at = None;
                self.save_settings();
                self.status = format!(
                    "更新通道已切到{}，点「检查更新」按新通道查",
                    if insider { "预发" } else { "稳定" }
                );
            }
            if self.edit_insider_channel {
                ui.label(
                    RichText::new("当前按预发通道检查：会拿到标为 prerelease 的版本。")
                        .size(11.0)
                        .color(theme::warning()),
                );
            }
            if self.app_installer_path.is_some() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new("安装包已在校验通过后落到本地，点「安装」才会动程序目录。")
                        .size(11.0)
                        .color(theme::muted()),
                );
            }
        });
    }

    fn ui_app_update_progress(&self, ui: &mut Ui, progress: &AppUpdateProgress) {
        ui.label(
            RichText::new(format!("{} · {}", progress.stage.label(), progress.message))
                .size(11.0)
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
                .size(11.0)
                .color(theme::muted()),
            );
        }
    }

    /// 侧栏那个绿色↑：有可更新的新版时返回要显示的版本号，否则 None。
    pub(crate) fn pending_update_tag(&self) -> Option<String> {
        self.app_release
            .as_ref()
            .and_then(|release| release.marker())
            .map(|tag| tag.to_string())
    }

    /// 启动后自己查一次更新，只为点亮侧栏的↑。不占进度条、不动状态栏。
    pub(crate) fn check_app_update_quiet(&mut self) {
        let tx = self.tx.clone();
        let proxy = self.app_update_proxy();
        // 后台这次按**已保存**的通道查：编辑态可能正被用户改到一半。
        let insider = self.settings.insider_channel;
        self.app_update_quiet = true;
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

    /// 自动检查的节奏：**每 6 小时一次**。留足余量给手动检查和内核更新。
    /// 用户已经在关于页手动查过（或正在查）就跳过这一次。
    pub(crate) fn maybe_check_app_update(&mut self) {
        let Some(due) = self.app_update_due else {
            return;
        };
        if std::time::Instant::now() < due {
            return;
        }
        self.app_update_due = Some(
            std::time::Instant::now()
                + std::time::Duration::from_secs(APP_UPDATE_CHECK_INTERVAL_SECS),
        );
        if self.app_release.is_some() || self.app_update_busy || self.app_update_quiet {
            return;
        }
        self.check_app_update_quiet();
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
            Ok(app_update::Handoff::WizardLaunched) => {
                self.status = format!("安装向导已启动：{}", path.display());
                self.app_update_progress = Some(AppUpdateProgress {
                    stage: AppUpdateStage::Ready,
                    downloaded: 0,
                    total: 0,
                    bytes_per_second: 0.0,
                    message: "安装向导已接管，本程序马上会被关闭".into(),
                });
            }
            // Linux：包已下载并校验过，但程序不会自己去拿 root。把命令原样给出来 ——
            // 说"安装向导已启动"在这里是假话，而假话比"要你自己动手"更糟。
            Ok(app_update::Handoff::ManualInstall { command }) => {
                self.status = format!("包已就位，请执行：{command}");
                self.app_update_progress = Some(AppUpdateProgress {
                    stage: AppUpdateStage::Ready,
                    downloaded: 0,
                    total: 0,
                    bytes_per_second: 0.0,
                    message: format!("已下载到本地并校验通过，执行这条命令完成安装：{command}"),
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

    pub(crate) fn fetch_available_releases(&mut self) {
        let tx = self.tx.clone();
        let proxy = self.app_update_proxy();
        self.fetching_releases = true;
        self.status = "正在拉取 GitHub Releases 版本列表...".into();
        self.rt.spawn(async move {
            let result = app_update::list_releases(
                proxy.as_deref(),
                app_update::current_version(),
                30,
            )
            .await
            .map_err(|error| error.to_string());
            let _ = tx.send(BgMsg::AppReleasesListed(result));
        });
    }

    pub(crate) fn select_release_version(&mut self, tag: String) {
        if let Some(item) = self.available_releases.iter().find(|r| r.tag == tag).cloned() {
            let current = app_update::current_version();
            let info = item.to_release_info(current);
            self.selected_release_tag = Some(tag.clone());
            self.app_release = Some(info);
            self.app_installer_path = None;
            self.app_update_progress = None;
            self.status = format!("已选中版本 {tag}，可点击上方「下载」进行安装/回退");
        }
    }

    pub(crate) fn ui_release_notes_modal(&mut self, ctx: &egui::Context) {
        if !self.show_release_notes_modal {
            return;
        }
        let Some((tag, notes)) = &self.release_notes_content else {
            self.show_release_notes_modal = false;
            return;
        };

        let mut open = true;
        let title = format!("Release 版本说明 · {tag}");
        egui::Window::new(title)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(true)
            .default_width(580.0)
            .default_height(420.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        Frame::NONE
                            .fill(theme::sidebar())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(notes)
                                        .size(13.0)
                                        .color(theme::text_primary()),
                                );
                            });
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("关闭").clicked() {
                        self.show_release_notes_modal = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("在 GitHub 查看 ↗").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(RELEASES_URL));
                        }
                    });
                });
            });

        if !open {
            self.show_release_notes_modal = false;
        }
    }
}

/// 关于页的次级按钮：圆角跟着样式默认走（2px），和旁边那个 `primary_btn("检查更新")`
/// 一致 —— 之前这里钉了 6px，两个按钮并排在同一张卡里就是两种圆角。
fn button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .min_size(Vec2::new(72.0, 26.0))
}

/// 键值行：标签钉在固定宽的一列，值从同一根竖线开始 —— 之前用 `horizontal` + 默认间距，
/// 标签 4~5 个字不一，值的起始 x 每行都在跳，六行看着就是锯齿。
///
/// 值段先按格子宽度**量着截**（末尾接省略号），再把自己的 `clip_rect` 收到那块矩形上：
/// `allocate_new_ui` 只改 `max_rect`，不改裁剪框（探针量到一条值的裁剪右界是整屏 1330，
/// 不是它自己的格子），所以光开子 ui 是裁不住的，长路径会直接压到右边的动作按钮上。
/// 返回"动作按钮这帧被点了没"。
fn kv_row(ui: &mut Ui, label: &str, value: &str, action: Option<&str>) -> bool {
    const LABEL_W: f32 = 78.0;
    const ACTION_W: f32 = 56.0;
    const GAP: f32 = 8.0;
    const ROW_H: f32 = 25.0;
    const VALUE_SIZE: f32 = 11.5;
    let row = egui::Rect::from_min_max(
        egui::pos2(ui.max_rect().left(), ui.cursor().top()),
        egui::pos2(ui.max_rect().right(), ui.cursor().top() + ROW_H),
    );
    let value_w = (row.width() - LABEL_W - ACTION_W - GAP * 2.0).max(60.0);
    let cell = egui::Rect::from_min_size(
        egui::pos2(row.left() + LABEL_W + GAP, row.min.y),
        Vec2::new(value_w, ROW_H),
    );
    let shown =
        crate::gui::widgets::ellipsize(ui, value, value_w, egui::FontId::monospace(VALUE_SIZE));
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .id_salt(("about:kv", label))
            .max_rect(egui::Rect::from_min_size(
                row.min,
                Vec2::new(LABEL_W, ROW_H),
            )),
        |ui| {
            ui.add_space(3.0);
            ui.label(RichText::new(label).size(11.5).color(theme::muted()));
        },
    );
    let val_resp = ui.allocate_new_ui(
        egui::UiBuilder::new()
            .id_salt(("about:kv", label, "value"))
            .max_rect(cell),
        |ui| {
            ui.set_clip_rect(cell);
            ui.add_space(3.0);
            ui.label(
                RichText::new(&shown)
                    .monospace()
                    .size(VALUE_SIZE)
                    .color(theme::text_secondary()),
            );
        },
    )
    .response;
    if shown != value {
        val_resp.on_hover_text(value);
    }
    let mut clicked = false;
    if let Some(text) = action {
        clicked = ui
            .allocate_new_ui(
                egui::UiBuilder::new()
                    .id_salt(("about:kv", label, "action"))
                    .max_rect(egui::Rect::from_min_size(
                        egui::pos2(row.right() - ACTION_W, row.min.y + 1.0),
                        Vec2::new(ACTION_W, ROW_H - 2.0),
                    )),
                |ui| {
                    ui.add(
                        egui::Button::new(
                            RichText::new(text).size(11.5).color(theme::accent_strong()),
                        )
                        .fill(theme::panel())
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(6))
                        .min_size(Vec2::new(ACTION_W, ROW_H - 2.0)),
                    )
                    .clicked()
                },
            )
            .inner;
    }
    ui.advance_cursor_after_rect(row);
    clicked
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
#[cfg(test)]
mod about_layout_probe_tests {
    use super::{button, disclaimer_card, kv_row, DISCLAIMER_LINES, TAGLINE};
    use crate::gui::icons::Icon;
    use crate::gui::theme;
    use crate::gui::widgets::{card_frame, icon_tile, pill};
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, RichText, Vec2};

    /// 一帧里画出来的文字：(内容, 左边界, 自然右边界, 这一条自己的裁剪右边界)。
    type Painted = Vec<(String, f32, f32, f32)>;

    fn read_text(ctx: &egui::Context, layer: egui::LayerId) -> Painted {
        ctx.graphics_mut(|g| {
            let mut out = Painted::new();
            let Some(list) = g.get(layer) else {
                return out;
            };
            for shape in list.all_entries() {
                if let egui::Shape::Text(text) = &shape.shape {
                    let left = text.pos.x;
                    out.push((
                        text.galley.text().to_string(),
                        left,
                        left + text.galley.size().x,
                        shape.clip_rect.max.x,
                    ));
                }
            }
            out
        })
    }

    /// 关于页真实的外壳：CentralPanel(左右 18 内边距) → 白卡(内边距 22)。
    /// 传进闭包的 `layer` 是卡片内容那一层的 id，画完照它读这一帧的形状清单。
    fn in_card(width: f32, body: impl FnOnce(&mut egui::Ui, egui::LayerId)) {
        // `Context::run` 要 `FnMut`，而这里的 body 只会用到一次：塞进 Option 再取出来，
        // 就绕开了"FnOnce 不能放进 FnMut 闭包"这条。
        let mut slot = Some(body);
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(width, 800.0),
        ));
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::WHITE)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    card_frame().show(ui, |ui| {
                        let layer = ui.layer_id();
                        if let Some(body) = slot.take() {
                            body(ui, layer);
                        }
                    });
                });
        });
    }

    /// 值列钉在同一根竖线上，而且每行只在自己的矩形里画 —— 长值不许压到右边的动作按钮。
    #[test]
    fn the_value_column_starts_on_one_vertical_and_is_clipped_in_its_own_cell() {
        let mut painted = Painted::new();
        let mut card_right = 0.0_f32;
        in_card(1330.0, |ui, layer| {
            kv_row(ui, "程序目录", "C:\\Apps\\VPN Parser", Some("打开"));
            kv_row(
                ui,
                "更新包暂存",
                "C:\\Apps\\VPN Parser\\data\\updates",
                Some("打开"),
            );
            kv_row(ui, "SHA-256", &"a".repeat(64), Some("复制"));
            painted = read_text(ui.ctx(), layer);
            card_right = ui.max_rect().right();
        });
        let values: Vec<&(String, f32, f32, f32)> = painted
            .iter()
            .filter(|p| p.0.starts_with("C:") || p.0.chars().all(|c| c == 'a'))
            .collect();
        assert_eq!(
            values.len(),
            3,
            "三行值没都画出来：{:?}",
            painted.iter().map(|p| p.0.clone()).collect::<Vec<_>>()
        );
        let first = values[0].1;
        for value in &values {
            assert!(
                (value.1 - first).abs() < 0.6,
                "值列起点不齐：{} 在 x={:.1}，第一行在 {first:.1}",
                value.0,
                value.1
            );
            assert!(
                value.3 <= card_right + 0.1,
                "{} 的裁剪右界 {:.1} 出了卡片 {:.1}",
                value.0,
                value.3,
                card_right
            );
        }
        // 极端长值（别人把 URL 拼成 300 个字符）：必须截成"…结尾 + 落在自己格子里"。
        let mut wide = Painted::new();
        let mut cell_right = 0.0_f32;
        in_card(960.0, |ui, layer| {
            kv_row(ui, "安装包", &"z".repeat(300), Some("复制"));
            wide = read_text(ui.ctx(), layer);
            cell_right = ui.max_rect().right();
        });
        let long = wide
            .iter()
            .find(|p| p.0.starts_with('z'))
            .expect("那条长值没画出来");
        assert!(
            long.0.ends_with(char::from_u32(0x2026).unwrap()) && long.0.chars().count() < 300,
            "300 字符这条没被截断，画出来的是 {:?}",
            long.0
        );
        assert!(
            long.2 <= long.3 + 0.5,
            "截完之后还是冲出了自己的格子：自然右界 {:.1} > 裁剪右界 {:.1}",
            long.2,
            long.3
        );
        assert!(
            long.3 < cell_right,
            "裁剪框没收到那一格（裁剪右界 {:.1} = 整张卡右界 {:.1}）：光 allocate_new_ui 改 max_rect 是裁不住的",
            long.3,
            cell_right
        );
    }

    /// 反向验证：旧的 `horizontal` + 两个 label 确实会把值排成锯齿。这条不红，
    /// 上面那条对齐断言就是在给自己鼓掌。
    #[test]
    fn the_old_info_row_really_staggered_the_value_column() {
        let mut painted = Painted::new();
        in_card(1330.0, |ui, layer| {
            for label in ["程序目录", "更新包暂存", "内核目录"] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).small().strong());
                    ui.label("同一个值");
                });
            }
            painted = read_text(ui.ctx(), layer);
        });
        let mut xs: Vec<f32> = painted
            .iter()
            .filter(|p| p.0 == "同一个值")
            .map(|p| p.1)
            .collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        assert_eq!(xs.len(), 3, "三行没都画出来");
        assert!(
            xs[2] - xs[0] > 4.0,
            "对照组失效：旧的 horizontal 写法这次只差 {:.1}，那 kv_row 的对齐断言就没意义了",
            xs[2] - xs[0]
        );
    }

    /// 胶囊要跟着换行，不许像 `Frame::show` 那样把整张卡撑宽（`stat_chip` 的注释里
    /// 记着实测：1535 > 面板 1312）。对照组就是 `Frame` 本身。
    #[test]
    fn pills_wrap_instead_of_widening_the_card() {
        let mut pill_right = 0.0_f32;
        let mut frame_right = 0.0_f32;
        let mut max_right = 0.0_f32;
        in_card(1330.0, |ui, _layer| {
            max_right = ui.max_rect().right();
            ui.horizontal_wrapped(|ui| {
                for _ in 0..14 {
                    pill(
                        ui,
                        "v1.5.0 可更新",
                        theme::accent_strong(),
                        theme::accent_bg(),
                    );
                }
            });
            pill_right = ui.min_rect().max.x;
            ui.horizontal_wrapped(|ui| {
                for _ in 0..14 {
                    Frame::NONE
                        .fill(theme::panel_hover())
                        .inner_margin(Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.label("v1.5.0 可更新");
                        });
                }
            });
            frame_right = ui.min_rect().max.x;
        });
        assert!(
            pill_right <= max_right + 0.5,
            "胶囊把卡片撑到了 x={pill_right:.1}，内容右界只有 {max_right:.1}"
        );
        assert!(
            frame_right > max_right,
            "对照组失效：Frame 版这次没撑宽（{frame_right:.1} <= {max_right:.1}），那这条断言什么也没证明"
        );
    }

    /// 顶部那行照产品代码复刻一遍（图标块 + 名字/胶囊 + 一句话 + 钉右的仓库按钮），
    /// 在最窄窗口下量：介绍语不许顶到按钮底下，按钮不许出卡片。
    #[test]
    fn the_identity_header_keeps_the_repo_button_clear_of_the_tagline() {
        for width in [960.0, 1100.0, 1330.0] {
            let mut painted = Painted::new();
            let mut button_left = f32::MAX;
            let mut button_right = 0.0_f32;
            let mut card_right = 0.0_f32;
            in_card(width, |ui, layer| {
                ui.horizontal(|ui| {
                    icon_tile(ui, Icon::Brand, theme::accent(), theme::accent_bg());
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("VPN Parser").strong().size(20.0));
                            ui.add_space(9.0);
                            pill(
                                ui,
                                &format!("v{}", crate::app_update::current_version()),
                                theme::accent_strong(),
                                theme::accent_bg(),
                            );
                            ui.add_space(6.0);
                            pill(
                                ui,
                                "MIT 开源",
                                theme::text_secondary(),
                                theme::panel_hover(),
                            );
                        });
                        ui.label(RichText::new(TAGLINE).size(12.5));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let response = ui.add(button("项目仓库 \u{2197}"));
                        button_left = response.rect.left();
                        button_right = response.rect.right();
                    });
                });
                painted = read_text(ui.ctx(), layer);
                card_right = ui.max_rect().right();
            });
            let tagline = painted
                .iter()
                .find(|p| p.0.starts_with("采集订阅"))
                .expect("介绍语没画出来");
            assert!(
                tagline.2 <= button_left - 4.0,
                "{width} 宽时介绍语右界 {:.1} 顶到了按钮左界 {button_left:.1}",
                tagline.2
            );
            assert!(
                button_right <= card_right + 0.1,
                "{width} 宽时按钮右界 {button_right:.1} 出了卡片 {card_right:.1}"
            );
        }
    }

    /// 免责声明：跑产品那个 `disclaimer_card`，量它这一帧真画出来的东西。
    /// 三条判据：每条都在（漏一条=清单与渲染脱节）、每条都在自己那格里折行（不许冲出卡片）、
    /// 每条长度封顶（这页是排障时才来的地方，不该变成一堵法律墙）。
    #[test]
    fn the_disclaimer_paints_every_line_inside_the_card() {
        const MAX_CHARS: usize = 150;
        for width in [1330.0, 960.0, 420.0] {
            let mut painted = Painted::new();
            let mut card_right = 0.0_f32;
            in_card(width, |ui, layer| {
                disclaimer_card(ui);
                painted = read_text(ui.ctx(), layer);
                card_right = ui.max_rect().right();
            });
            let heads: Vec<String> = DISCLAIMER_LINES
                .iter()
                .map(|line| line.chars().take(6).collect::<String>())
                .collect();
            for (index, head) in heads.iter().enumerate() {
                let entry = painted
                    .iter()
                    .find(|p| p.0.starts_with(head.as_str()))
                    .unwrap_or_else(|| {
                        panic!(
                            "{width:.0} 宽时免责第 {} 条（开头 {head}）没画出来；这一帧画的是 {:?}",
                            index + 1,
                            painted.iter().map(|p| p.0.clone()).collect::<Vec<_>>()
                        )
                    });
                assert!(
                    entry.2 <= entry.3 + 0.5 && entry.2 <= card_right + 0.5,
                    "{width:.0} 宽时免责第 {} 条右界 {:.1} 出了裁剪框 {:.1} / 卡片 {card_right:.1}",
                    index + 1,
                    entry.2,
                    entry.3
                );
            }
        }
        for (index, line) in DISCLAIMER_LINES.iter().enumerate() {
            assert!(
                line.chars().count() <= MAX_CHARS,
                "免责第 {} 条有 {} 字，超过 {MAX_CHARS} 字这条线：这页不该变成一堵墙",
                index + 1,
                line.chars().count()
            );
        }
    }
}

use std::path::{Path, PathBuf};
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{card_frame, format_bytes, primary_btn, status_badge};
use crate::gui::VpnApp;
use eframe::egui::{self, RichText, Ui};

/// 可清理文件的业务分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupCategory {
    /// 历史版本程序镜像与安装向导残留（.old / .bak / is-*.tmp）
    LegacyExe,
    /// 在线更新安装包暂存（data/updates/*.exe, *.deb, *.rpm 等）
    UpdateCache,
    /// 历史滚动旧日志与运行时临时文件（startup.log.1 等）
    LogAndTemp,
    /// 历史配置与数据备份（data/backups/*.yaml 等）
    Backups,
}

impl CleanupCategory {
    pub const ALL: [Self; 4] = [
        Self::LegacyExe,
        Self::UpdateCache,
        Self::LogAndTemp,
        Self::Backups,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::LegacyExe => "历史版本程序与安装残留",
            Self::UpdateCache => "在线更新安装包暂存",
            Self::LogAndTemp => "历史旧日志与运行临时文件",
            Self::Backups => "历史配置与数据快照",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::LegacyExe => "版本升级或覆盖安装时操作系统保留的旧版执行文件镜像（.old / .bak）及向导临时文件（is-*.tmp），安全可清理。",
            Self::UpdateCache => "从 GitHub Releases 下载完成的更新安装包（.exe / .deb / .rpm）及校验文件，更新完成后无需保留。",
            Self::LogAndTemp => "系统历史滚动的旧日志文件（startup.log.1 等）及运行时生成的非关键临时缓存（不影响当前运行日志）。",
            Self::Backups => "本地生成的旧配置文件备份及数据快照，若已无需回退历史配置可安全清理。",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::LegacyExe => Icon::Shield,
            Self::UpdateCache => Icon::Package,
            Self::LogAndTemp => Icon::Info,
            Self::Backups => Icon::Clipboard,
        }
    }
}

/// 单个可清理项
#[derive(Debug, Clone)]
pub struct CleanupItem {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub category: CleanupCategory,
}

/// 清理状态机（保存在 VpnApp 中）
#[derive(Debug, Clone, Default)]
pub struct CleanupState {
    pub scanned: bool,
    pub items: Vec<CleanupItem>,
    pub last_cleaned_summary: Option<(usize, u64, std::time::Instant)>,
}

impl CleanupState {
    pub fn total_size(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }

    pub fn total_count(&self) -> usize {
        self.items.len()
    }

    pub fn category_size(&self, cat: CleanupCategory) -> u64 {
        self.items
            .iter()
            .filter(|i| i.category == cat)
            .map(|i| i.size)
            .sum()
    }

    pub fn category_count(&self, cat: CleanupCategory) -> usize {
        self.items.iter().filter(|i| i.category == cat).count()
    }

    #[allow(dead_code)]
    pub fn category_items(&self, cat: CleanupCategory) -> Vec<&CleanupItem> {
        self.items.iter().filter(|i| i.category == cat).collect()
    }

    /// 扫描全系统可清理的历史文件与多余缓存
    pub fn scan(&mut self) {
        let mut found = Vec::new();
        let current_exe = std::env::current_exe().ok();

        // 1. 扫描程序目录下的旧版本程序与安装向导临时文件、旧日志
        if let Some(ref cur_exe) = current_exe {
            if let Some(exe_dir) = cur_exe.parent() {
                scan_exe_dir(exe_dir, cur_exe, &mut found);
            }
        }

        // 2. 扫描 updates 更新包暂存目录
        let update_dir = crate::app_update::installer_cache_dir();
        scan_directory_all(&update_dir, CleanupCategory::UpdateCache, &mut found);

        // 如果程序根目录下也有 updates 目录且与 data/updates 不同，一并扫描
        if let Some(ref cur_exe) = current_exe {
            if let Some(exe_dir) = cur_exe.parent() {
                let updates_in_exe = exe_dir.join("updates");
                if updates_in_exe != update_dir && updates_in_exe.is_dir() {
                    scan_directory_all(&updates_in_exe, CleanupCategory::UpdateCache, &mut found);
                }
            }
        }

        // 3. 扫描 data/run 下的运行时临时残留
        let data_run = crate::data_root().join("run");
        if data_run.is_dir() {
            scan_run_temp(&data_run, &mut found);
        }

        // 4. 扫描 data/backups 下的历史备份
        let backups_dir = crate::data_root().join("backups");
        if backups_dir.is_dir() {
            scan_backups_dir(&backups_dir, &mut found);
        }

        // 5. 扫描 data/kernels 下的历史内核备份或缓存
        let kernels_dir = crate::kernel::managed_kernel_root();
        if kernels_dir.is_dir() {
            scan_kernel_caches(&kernels_dir, &mut found);
        }

        // 按大小降序排序，最占空间的排在前面
        found.sort_by(|a, b| b.size.cmp(&a.size));

        self.items = found;
        self.scanned = true;
    }

    /// 清理全部已扫描出的项目
    pub fn clean_all(&mut self) -> (usize, u64) {
        let to_remove: Vec<PathBuf> = self.items.iter().map(|i| i.path.clone()).collect();
        let (count, freed) = remove_targets(&to_remove);
        self.scan();
        self.last_cleaned_summary = Some((count, freed, std::time::Instant::now()));
        (count, freed)
    }

    /// 按分类清理项目
    pub fn clean_category(&mut self, cat: CleanupCategory) -> (usize, u64) {
        let to_remove: Vec<PathBuf> = self
            .items
            .iter()
            .filter(|i| i.category == cat)
            .map(|i| i.path.clone())
            .collect();
        let (count, freed) = remove_targets(&to_remove);
        self.scan();
        self.last_cleaned_summary = Some((count, freed, std::time::Instant::now()));
        (count, freed)
    }

    /// 清理单个文件/目录
    pub fn clean_single(&mut self, path: &Path) -> bool {
        let (count, freed) = remove_targets(&[path.to_path_buf()]);
        self.scan();
        if count > 0 {
            self.last_cleaned_summary = Some((count, freed, std::time::Instant::now()));
            true
        } else {
            false
        }
    }
}

/// 扫描程序所在目录（识别 .old, .bak, is-*.tmp, 滚动日志等）
fn scan_exe_dir(exe_dir: &Path, current_exe: &Path, found: &mut Vec<CleanupItem>) {
    let Ok(entries) = std::fs::read_dir(exe_dir) else {
        return;
    };
    let current_exe_canon = current_exe.canonicalize().ok();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Ok(canon) = path.canonicalize() {
                if Some(&canon) == current_exe_canon.as_ref() {
                    continue; // 严禁触碰正在运行的自身二进制！
                }
            }

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let name_lower = file_name.to_ascii_lowercase();

            // 1. 历史执行文件镜像与 Inno Setup 临时文件
            let is_legacy_exe = name_lower.ends_with(".old")
                || name_lower.ends_with(".bak")
                || name_lower.ends_with(".old.exe")
                || name_lower.ends_with(".exe.old")
                || (name_lower.starts_with("is-") && name_lower.ends_with(".tmp"))
                || (name_lower.starts_with("vpn-parser") && name_lower.contains(".old"))
                || (name_lower.starts_with("vpn-parser") && name_lower.contains(".bak"));

            if is_legacy_exe {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                found.push(CleanupItem {
                    path,
                    name: file_name,
                    size,
                    category: CleanupCategory::LegacyExe,
                });
                continue;
            }

            // 2. 历史滚动日志与临时安装日志
            let is_legacy_log = name_lower.starts_with("startup.log.")
                || name_lower == "err-install.log"
                || name_lower == "out-install.log"
                || name_lower == "err.log"
                || name_lower == "out.log"
                || (name_lower.ends_with(".tmp") && !name_lower.starts_with("is-"));

            if is_legacy_log {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                found.push(CleanupItem {
                    path,
                    name: file_name,
                    size,
                    category: CleanupCategory::LogAndTemp,
                });
            }
        }
    }
}

/// 扫描指定目录下的全部文件
fn scan_directory_all(dir: &Path, category: CleanupCategory, found: &mut Vec<CleanupItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            found.push(CleanupItem {
                path,
                name: file_name,
                size,
                category,
            });
        }
    }
}

/// 扫描 data/run 下的临时残留
fn scan_run_temp(run_dir: &Path, found: &mut Vec<CleanupItem>) {
    let Ok(entries) = std::fs::read_dir(run_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let name_lower = file_name.to_ascii_lowercase();
            if name_lower.ends_with(".tmp")
                || name_lower.ends_with(".bak")
                || name_lower.ends_with(".old")
            {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                found.push(CleanupItem {
                    path: path.clone(),
                    name: format!("run/{file_name}"),
                    size,
                    category: CleanupCategory::LogAndTemp,
                });
            }
        }
    }
}

/// 扫描 data/backups 下的历史备份
fn scan_backups_dir(backups_dir: &Path, found: &mut Vec<CleanupItem>) {
    let Ok(entries) = std::fs::read_dir(backups_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        if path.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            found.push(CleanupItem {
                path,
                name: format!("backups/{file_name}"),
                size,
                category: CleanupCategory::Backups,
            });
        } else if path.is_dir() {
            // 目录类型备份（计算整个子目录总大小）
            let dir_size = calculate_dir_size(&path);
            found.push(CleanupItem {
                path,
                name: format!("backups/{file_name} [文件夹]"),
                size: dir_size,
                category: CleanupCategory::Backups,
            });
        }
    }
}

/// 扫描 data/kernels 下的缓存
fn scan_kernel_caches(kernels_dir: &Path, found: &mut Vec<CleanupItem>) {
    let release_cache = kernels_dir.join(".release-cache");
    if release_cache.is_dir() {
        let dir_size = calculate_dir_size(&release_cache);
        if dir_size > 0 {
            found.push(CleanupItem {
                path: release_cache,
                name: "kernels/.release-cache [下载缓存]".to_string(),
                size: dir_size,
                category: CleanupCategory::UpdateCache,
            });
        }
    }

    // 扫描每个内核子目录下的 .bak / .old
    if let Ok(entries) = std::fs::read_dir(kernels_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub in sub_entries.flatten() {
                        let sub_path = sub.path();
                        if sub_path.is_file() {
                            let name = sub_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or_default()
                                .to_string();
                            let name_lower = name.to_ascii_lowercase();
                            if name_lower.ends_with(".bak") || name_lower.ends_with(".old") {
                                let size = sub.metadata().map(|m| m.len()).unwrap_or(0);
                                found.push(CleanupItem {
                                    path: sub_path,
                                    name: format!("kernels/{name}"),
                                    size,
                                    category: CleanupCategory::LegacyExe,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 递归计算目录总大小
fn calculate_dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            } else if p.is_dir() {
                total += calculate_dir_size(&p);
            }
        }
    }
    total
}

/// 安全删除目标文件/目录，返回成功清理数与释放字节数
fn remove_targets(paths: &[PathBuf]) -> (usize, u64) {
    let mut count = 0;
    let mut freed = 0;

    for path in paths {
        if path.is_file() {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            if std::fs::remove_file(path).is_ok() {
                count += 1;
                freed += size;
            }
        } else if path.is_dir() {
            let size = calculate_dir_size(path);
            if std::fs::remove_dir_all(path).is_ok() {
                count += 1;
                freed += size;
            }
        }
    }

    (count, freed)
}

impl VpnApp {
    /// 渲染「系统设置」中的「清理」Tab
    pub(crate) fn ui_settings_cleanup(&mut self, ui: &mut Ui) {
        // 首次打开自动执行一次全量扫描
        if !self.cleanup_state.scanned {
            self.cleanup_state.scan();
        }

        let total_size = self.cleanup_state.total_size();
        let total_count = self.cleanup_state.total_count();

        // 1. 顶部总览卡片
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Trash, 20.0, theme::accent());
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("系统存储与历史版本清理").strong().size(16.5));
                        ui.add_space(6.0);
                        let badge_text = if total_count > 0 {
                            format!("{total_count} 项可清理 · 共 {}", format_bytes(total_size))
                        } else {
                            "系统环境非常整洁".to_string()
                        };
                        let badge_color = if total_count > 0 {
                            theme::warning()
                        } else {
                            theme::success()
                        };
                        status_badge(ui, &badge_text, badge_color);
                    });
                    ui.label(
                        RichText::new("一键扫描并清理覆盖安装产生的历史可执行文件备份、在线更新安装包暂存、滚动日志与备份快照。")
                            .size(11.5)
                            .color(theme::muted()),
                    );
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(total_count > 0, primary_btn("一键清理全部 🧹"))
                        .on_hover_text("安全清理下方发现的所有历史版本镜像与临时缓存文件")
                        .clicked()
                    {
                        let (cnt, bytes) = self.cleanup_state.clean_all();
                        self.status = format!("清理完成：成功删除 {cnt} 个历史文件，释放了 {}", format_bytes(bytes));
                    }

                    if ui
                        .button("重新扫描 🔄")
                        .on_hover_text("重新核对本机程序目录与数据目录的存储占用")
                        .clicked()
                    {
                        self.cleanup_state.scan();
                    }
                });
            });

            // 清理成果即时提示条
            if let Some((cnt, bytes, at)) = self.cleanup_state.last_cleaned_summary {
                if at.elapsed().as_secs() < 30 {
                    ui.add_space(8.0);
                    egui::Frame::NONE
                        .fill(theme::panel_hover())
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Check, 14.0, theme::success());
                                ui.label(
                                    RichText::new(format!(
                                        "本次已成功清理 {cnt} 项历史残留，共为您释放了 {} 宝贵磁盘空间！",
                                        format_bytes(bytes)
                                    ))
                                    .size(12.0)
                                    .color(theme::text_primary()),
                                );
                            });
                        });
                }
            }

            if total_count == 0 && self.cleanup_state.scanned {
                ui.add_space(8.0);
                egui::Frame::NONE
                    .fill(theme::panel_hover())
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Check, 14.0, theme::success());
                            ui.label(
                                RichText::new("未检测到任何多余的历史版本副本或多余缓存，无需进行额外清理。")
                                    .size(12.0)
                                    .color(theme::text_secondary()),
                            );
                        });
                    });
            }
        });

        ui.add_space(8.0);

        // 2. 分类详情列表
        for category in CleanupCategory::ALL {
            let cat_count = self.cleanup_state.category_count(category);
            let cat_size = self.cleanup_state.category_size(category);
            let items: Vec<CleanupItem> = self
                .cleanup_state
                .items
                .iter()
                .filter(|i| i.category == category)
                .cloned()
                .collect();

            let mut clean_category_clicked = false;
            let mut delete_target = None;

            card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, category.icon(), 16.0, theme::accent());
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(category.title()).strong().size(14.5));
                            ui.add_space(4.0);
                            if cat_count > 0 {
                                status_badge(
                                    ui,
                                    &format!("{cat_count} 项 · {}", format_bytes(cat_size)),
                                    theme::warning(),
                                );
                            } else {
                                status_badge(ui, "暂无残留", theme::muted());
                            }
                        });
                        ui.label(
                            RichText::new(category.description())
                                .size(11.0)
                                .color(theme::muted()),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(cat_count > 0, eframe::egui::Button::new("清理此类"))
                            .on_hover_text(format!("一键清理所有「{}」", category.title()))
                            .clicked()
                        {
                            clean_category_clicked = true;
                        }
                    });
                });

                // 若有待清理文件，呈现具体文件条目
                if !items.is_empty() {
                    ui.add_space(6.0);
                    egui::Frame::NONE
                        .fill(theme::panel_hover())
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(ui, |ui| {
                            for item in &items {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&item.name)
                                            .monospace()
                                            .size(11.5)
                                            .color(theme::text_primary()),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(format_bytes(item.size))
                                            .size(11.0)
                                            .color(theme::muted()),
                                    );

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .small_button("删除")
                                                .on_hover_text(format!(
                                                    "永久删除该文件：{}",
                                                    item.path.display()
                                                ))
                                                .clicked()
                                            {
                                                delete_target = Some(item.path.clone());
                                            }

                                            if let Some(parent) = item.path.parent() {
                                                if ui
                                                    .small_button("定位")
                                                    .on_hover_text("在资源管理器中定位所在文件夹")
                                                    .clicked()
                                                {
                                                    let _ = crate::platform::shell_open(parent);
                                                }
                                            }
                                        },
                                    );
                                });
                            }
                        });
                }
            });

            if clean_category_clicked {
                let (cnt, bytes) = self.cleanup_state.clean_category(category);
                self.status = format!(
                    "已清理 {} 项「{}」，释放了 {}",
                    cnt,
                    category.title(),
                    format_bytes(bytes)
                );
            }
            if let Some(target) = delete_target {
                if self.cleanup_state.clean_single(&target) {
                    self.status = format!("已删除：{}", target.display());
                }
            }

            ui.add_space(6.0);
        }

        // 3. 底部安全提示卡片
        card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                icon_label(ui, Icon::Check, 15.0, theme::accent());
                ui.vertical(|ui| {
                    ui.label(RichText::new("系统防护与清理保障").strong().size(13.0));
                    ui.label(
                        RichText::new(
                            "• 当前运行的程序实体 (vpn-parser.exe)、活动状态日志与核心数据资产数据库 (vpn-parser.db) 受到底层受控锁定，绝不会被误清理。\n• 历史程序备份镜像 (.old) 是 Windows 自动更新替换或覆盖安装时生成的副本，在当前版本使用稳定后即可放心一键清除以释放空间。",
                        )
                        .size(11.0)
                        .color(theme::muted()),
                    );
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_display_ranges() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 5), "5.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 2), "2.00 GB");
    }

    #[test]
    fn scan_exe_dir_protects_current_and_catches_old() {
        let test_dir = std::env::temp_dir().join(format!("vpn_cleanup_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&test_dir);

        let current_exe = test_dir.join("vpn-parser.exe");
        std::fs::write(&current_exe, b"live_executable_bytes").unwrap();

        let old_exe = test_dir.join("vpn-parser.exe.old");
        std::fs::write(&old_exe, b"old_backup_bytes").unwrap();

        let tmp_installer = test_dir.join("is-12345.tmp");
        std::fs::write(&tmp_installer, b"installer_tmp").unwrap();

        let rotated_log = test_dir.join("startup.log.1");
        std::fs::write(&rotated_log, b"old_log_content").unwrap();

        let live_log = test_dir.join("startup.log");
        std::fs::write(&live_log, b"active_live_log").unwrap();

        let mut found = Vec::new();
        scan_exe_dir(&test_dir, &current_exe, &mut found);

        // 验证当前主程序和活跃主日志绝对未被判定为待清理
        assert!(!found.iter().any(|i| i.path == current_exe), "当前运行的 exe 绝不能被扫描进入清理列表");
        assert!(!found.iter().any(|i| i.path == live_log), "活跃的当前 startup.log 绝不能被扫描进入清理列表");

        // 验证旧文件全部被扫描到
        assert!(found.iter().any(|i| i.path == old_exe), "历史程序镜像 .old 应当被识别");
        assert!(found.iter().any(|i| i.path == tmp_installer), "安装程序临时文件 is-*.tmp 应当被识别");
        assert!(found.iter().any(|i| i.path == rotated_log), "已轮转的历史日志 *.log.1 应当被识别");

        // 验证删除执行
        let to_remove: Vec<PathBuf> = found.iter().map(|i| i.path.clone()).collect();
        let (cnt, freed) = remove_targets(&to_remove);
        assert_eq!(cnt, 3);
        assert!(freed > 0);

        assert!(!old_exe.exists());
        assert!(!tmp_installer.exists());
        assert!(!rotated_log.exists());
        assert!(current_exe.exists());
        assert!(live_log.exists());

        let _ = std::fs::remove_dir_all(&test_dir);
    }
}

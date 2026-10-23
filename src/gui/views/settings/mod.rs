pub mod appearance;
pub mod api;
pub mod automation;
pub mod general;
pub mod network;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use appearance::{font_scale_ticks, handle_travel_range, FONT_SCALE_MAX, FONT_SCALE_MIN};

use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::SettingsTab;
use crate::gui::widgets::{panel_tab, primary_btn, section_title};
use crate::gui::VpnApp;
use eframe::egui::{self, RichText, Ui};

/// 系统设置表头右侧动作组（保存设置 + 重置修改 + 状态）预留宽度。
const SETTINGS_ACTIONS_WIDTH: f32 = 300.0;

impl VpnApp {
    pub(crate) fn ui_settings(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("系统设置"), "应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务");

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
                    // 这一整块是 right_to_left：先加文字、后加图标，读出来才是「图标 文字」。
                    ui.label(
                        RichText::new("有未保存修改")
                            .color(theme::warning())
                            .small(),
                    );
                    icon_label(ui, Icon::Alert, 12.0, theme::warning());
                } else {
                    ui.label(RichText::new(tr_synced).color(theme::accent_strong()).small());
                    icon_label(ui, Icon::Check, 12.0, theme::accent_strong());
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
            .id_salt("settings_main_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match self.settings_tab {
                SettingsTab::General => self.ui_settings_general(ui),
                SettingsTab::Appearance => self.ui_settings_appearance(ui),
                SettingsTab::Automation => self.ui_settings_automation(ui),
                SettingsTab::Network => self.ui_settings_network(ui),
                SettingsTab::Api => self.ui_api(ui),
            });
    }

}

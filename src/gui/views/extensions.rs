use crate::gui::icons::Icon;
use crate::gui::types::ExtensionsTab;
use crate::gui::widgets::macos_segmented_tabs;
use crate::gui::VpnApp;
use eframe::egui::Ui;

impl VpnApp {
    pub(crate) fn ui_extensions(&mut self, ui: &mut Ui) {
        let tabs = [
            (ExtensionsTab::Security, Icon::Shield, self.tr("隐私安全")),
            (ExtensionsTab::PortMappings, Icon::Plug, self.tr("活动节点列表")),
        ];

        macos_segmented_tabs(ui, &mut self.extensions_tab, &tabs);
        ui.add_space(8.0);

        match self.extensions_tab {
            ExtensionsTab::Security => {
                ui.push_id("tab_security", |ui| self.ui_proxy_security(ui));
            }
            ExtensionsTab::PortMappings => {
                ui.push_id("tab_port_mappings", |ui| self.ui_proxy_port_mappings(ui));
            }
        };
    }
}

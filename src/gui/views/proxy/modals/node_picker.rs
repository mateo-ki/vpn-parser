use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{
    protocol_pill, render_country_code_badge, sanitize_node_name,
};
use crate::gui::VpnApp;
use eframe::egui::{self, RichText};

impl VpnApp {
    pub(crate) fn ui_node_picker_modal(&mut self, ctx: &egui::Context) {
        if !self.node_picker_open {
            return;
        }
        let mut open = self.node_picker_open;
        let mut picked_node = None;

        let modal_title = match self.node_picker_target {
            Some(crate::gui::types::NodePickerTarget::ChainEntry) => "选择链式代理前置节点",
            Some(crate::gui::types::NodePickerTarget::ChainExit) => "选择链式代理后置落地节点",
            _ => "选择转发目标节点",
        };

        egui::Window::new(RichText::new(modal_title).strong())
            .open(&mut open)
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .fixed_size([560.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    icon_label(ui, Icon::Search, 14.0, theme::muted());
                    let search_field = ui.add(
                        egui::TextEdit::singleline(&mut self.node_picker_search)
                            .desired_width(450.0)
                            .hint_text("搜索节点名称、国家、协议、服务器..."),
                    );
                    crate::gui::widgets::note_text_field(ui, &search_field);
                    if !self.node_picker_search.is_empty()
                        && ui
                            .add(crate::gui::icons::icon_btn_square(Icon::Close, 11.0))
                            .on_hover_text("清空搜索内容")
                            .clicked()
                    {
                        self.node_picker_search.clear();
                    }
                });
                ui.separator();

                let search = self.node_picker_search.trim().to_lowercase();
                let all_nodes = self.proxy_nodes.iter().chain(self.nodes.iter());

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(430.0)
                    .show(ui, |ui| {
                        let mut count = 0;
                        let mut seen_ids = std::collections::HashSet::new();
                        for node in all_nodes {
                            if !seen_ids.insert(&node.id) {
                                continue;
                            }
                            if !search.is_empty() {
                                let matches = node.name.to_lowercase().contains(&search)
                                    || node.server.to_lowercase().contains(&search)
                                    || node.protocol.to_lowercase().contains(&search)
                                    || node
                                        .country
                                        .as_deref()
                                        .unwrap_or("")
                                        .to_lowercase()
                                        .contains(&search);
                                if !matches {
                                    continue;
                                }
                            }
                            count += 1;

                            ui.horizontal(|ui| {
                                render_country_code_badge(
                                    ui,
                                    node.country.as_deref().unwrap_or(""),
                                );
                                ui.add_space(2.0);
                                protocol_pill(ui, &node.protocol.to_uppercase());

                                let clean_name = sanitize_node_name(&node.name);
                                let shown_name = crate::gui::widgets::ellipsize(
                                    ui,
                                    &clean_name,
                                    350.0,
                                    egui::FontId::proportional(14.0),
                                );
                                let name_label = ui.add_sized(
                                    [350.0, 18.0],
                                    egui::Label::new(RichText::new(shown_name).strong()),
                                );
                                name_label.on_hover_text(format!(
                                    "{}\n{}:{}",
                                    node.name, node.server, node.port
                                ));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("选择").clicked() {
                                        picked_node = Some((node.id.clone(), node.name.clone()));
                                    }
                                });
                            });
                            ui.separator();
                        }

                        if count == 0 {
                            ui.vertical_centered(|ui| {
                                ui.add_space(30.0);
                                ui.label(
                                    RichText::new("没有找到匹配的节点").color(theme::muted()),
                                );
                            });
                        }
                    });
            });

        if !open {
            self.node_picker_open = false;
        }
        if let Some((id, name)) = picked_node {
            match self.node_picker_target {
                Some(crate::gui::types::NodePickerTarget::Speedtest) => {
                    self.switch_proxy_node(id, name, "测速选择出站节点");
                }
                Some(crate::gui::types::NodePickerTarget::ChainEntry) => {
                    self.proxy_config.chain_entry_node_id = Some(id);
                    self.proxy_config.chain_entry_node_name = Some(name);
                    self.save_proxy_config();
                }
                Some(crate::gui::types::NodePickerTarget::ChainExit) => {
                    self.proxy_config.chain_exit_node_id = Some(id);
                    self.proxy_config.chain_exit_node_name = Some(name);
                    self.save_proxy_config();
                }
                Some(crate::gui::types::NodePickerTarget::PortMapping(target_idx)) => {
                    if let Some(mapping) = self.proxy_config.port_mappings.get_mut(target_idx) {
                        mapping.node_id = id;
                        mapping.node_name = name;
                        self.save_proxy_config();
                    }
                }
                Some(crate::gui::types::NodePickerTarget::NewPortMapping) => {
                    self.new_mapping_node_id = Some(id);
                    self.new_mapping_node_name = Some(name);
                }
                None => {
                    if let Some(target_idx) = self.node_picker_target_index {
                        if let Some(mapping) = self.proxy_config.port_mappings.get_mut(target_idx) {
                            mapping.node_id = id;
                            mapping.node_name = name;
                            self.save_proxy_config();
                        }
                    } else {
                        self.new_mapping_node_id = Some(id);
                        self.new_mapping_node_name = Some(name);
                    }
                }
            }
            self.node_picker_open = false;
            self.node_picker_target = None;
            self.node_picker_target_index = None;
        }
    }

}

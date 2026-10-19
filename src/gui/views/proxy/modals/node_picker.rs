use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    ActiveConnectionInfo, ConnectionsTab, DnsRecord, GroupsTab, KernelGroup, ProxyPanel, ProxySortBy,
    ProxyView, ProxyViewRow, RuleDraftKind, RulesTab,
};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, country_flag_and_name, danger_btn, dns_mode_label,
    format_bytes, format_speed, header_cell, macos_segmented_tabs, panel_frame, panel_tab, primary_btn,
    protocol_pill, render_country_code_badge, render_health_strip, render_node_health_bar,
    render_protocol_donut_chart_sized, render_signal_bars, render_speedometer, render_top_processes_card,
    render_traffic_waveform_card, routing_mode_description, routing_mode_detail,
    rule_action_label, rule_matcher_label, sanitize_node_name, section_title, status_badge, tun_stack_label,
};
use crate::gui::VpnApp;
use crate::kernel::{KernelKind, KernelPreference};
use crate::models::NodeSummary;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules, recommended_traffic_rules,
    social_telegram_rules, streaming_media_rules, AutoRoutingStrategy, DnsEnhancedMode, ProxyMode, ProxyState,
    RoutingMode, RuleAction, TrafficRule, TunStack, URL_TEST_GROUP,
};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;

impl VpnApp {
    pub(crate) fn ui_node_picker_modal(&mut self, ctx: &egui::Context) {
        if !self.node_picker_open {
            return;
        }
        let mut open = self.node_picker_open;
        let mut picked_node = None;

        egui::Window::new(RichText::new("选择转发目标节点").strong())
            .open(&mut open)
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
                    if !self.node_picker_search.is_empty() && ui.small_button("✕").clicked() {
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
                                let name_label = ui.add_sized(
                                    [350.0, 18.0],
                                    egui::Label::new(RichText::new(&clean_name).strong()).truncate(),
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
            self.node_picker_open = false;
            self.node_picker_target_index = None;
        }
    }

}

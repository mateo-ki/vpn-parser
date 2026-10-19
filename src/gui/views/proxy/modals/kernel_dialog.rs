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
    pub(crate) fn ui_kernel_dialog(&mut self, ctx: &egui::Context) {
        use crate::kernel::{source_from_id, KernelSource, SOURCE_OPTIONS};
        if !self.show_kernel_dialog {
            return;
        }
        let Some(kind) = self.kernel_dialog_kind else {
            return;
        };
        let mut open = true;
        let mut start: Option<(KernelSource, Option<std::path::PathBuf>)> = None;

        const SOURCE_W: f32 = 424.0;
        const PAD: f32 = 11.0;
        const DOT_W: f32 = 22.0;

        egui::Window::new(format!("下载内核 · {}", kind.display_name()))
            .id(egui::Id::new("kernel-download-dialog"))
            .collapsible(false)
            .resizable(false)
            .default_width(SOURCE_W)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 把 zip 直接拖进窗口就能当本地导入的路径，省得他手打。
                let dropped: Vec<std::path::PathBuf> = ui.input(|input| {
                    input
                        .raw
                        .dropped_files
                        .iter()
                        .filter_map(|file| file.path.clone())
                        .collect()
                });
                for path in dropped {
                    let is_zip = path
                        .extension()
                        .map(|ext| ext.eq_ignore_ascii_case("zip"))
                        .unwrap_or(false);
                    if is_zip {
                        self.kernel_dialog_path = path.display().to_string();
                        self.kernel_dialog_source = "local".to_string();
                    }
                }

                ui.set_width(SOURCE_W);
                ui.label(
                    RichText::new("下载方式")
                        .strong()
                        .size(15.0)
                        .color(theme::text_primary()),
                );
                for option in SOURCE_OPTIONS {
                    let selected = self.kernel_dialog_source == option.id;
                    let mut job = egui::text::LayoutJob::default();
                    job.break_on_newline = true;
                    job.wrap.max_width = SOURCE_W - 2.0 * PAD - DOT_W;
                    job.append(
                        option.label,
                        0.0,
                        egui::text::TextFormat::simple(
                            egui::FontId::proportional(14.0),
                            if selected {
                                theme::accent_strong()
                            } else {
                                theme::text_primary()
                            },
                        ),
                    );
                    job.append(
                        &format!("\n{}", option.hint),
                        0.0,
                        egui::text::TextFormat::simple(
                            egui::FontId::proportional(11.5),
                            theme::text_secondary(),
                        ),
                    );
                    let galley = ui.painter().layout_job(job);
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(SOURCE_W, galley.size().y + 2.0 * PAD),
                        Sense::click(),
                    );
                    if response.clicked() {
                        self.kernel_dialog_source = option.id.to_string();
                    }
                    let fill = if selected {
                        theme::accent_bg()
                    } else if response.hovered() {
                        theme::panel_hover()
                    } else {
                        theme::panel()
                    };
                    // 未选中的行也是白底 + 冷调细边，不用灰底铺满整列。
                    let stroke = if selected {
                        Stroke::new(1.0_f32, theme::accent())
                    } else {
                        Stroke::new(1.0_f32, theme::border())
                    };
                    ui.painter().rect(
                        rect,
                        CornerRadius::same(theme::RADIUS_BTN),
                        fill,
                        stroke,
                        egui::epaint::StrokeKind::Inside,
                    );
                    let dot = egui::pos2(rect.min.x + PAD + 5.0, rect.min.y + PAD + 8.0);
                    if selected {
                        ui.painter()
                            .circle_stroke(dot, 5.0, Stroke::new(1.6_f32, theme::accent()));
                        ui.painter().circle_filled(dot, 2.3_f32, theme::accent());
                    } else {
                        ui.painter()
                            .circle_stroke(dot, 5.0, Stroke::new(1.2_f32, theme::muted()));
                    }
                    ui.painter().galley(
                        egui::pos2(rect.min.x + PAD + DOT_W, rect.min.y + PAD - 1.0),
                        galley,
                        theme::text_primary(),
                    );
                    ui.add_space(4.0);
                }
                if self.kernel_dialog_source == "local" {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new("本地 zip（也可以把文件拖进这个窗口）")
                            .small()
                            .color(theme::text_secondary()),
                    );
                    let path_field = ui.add(
                        egui::TextEdit::singleline(&mut self.kernel_dialog_path)
                            .desired_width(SOURCE_W - 4.0)
                            .hint_text(r"D:\downloads\mihomo-windows-amd64-v3-v1.19.32.zip"),
                    );
                    crate::gui::widgets::note_text_field(ui, &path_field);
                }

                ui.add_space(6.0);
                ui.separator();
                let source = source_from_id(&self.kernel_dialog_source);
                let ready =
                    source != KernelSource::Local || !self.kernel_dialog_path.trim().is_empty();
                ui.horizontal(|ui| {
                    if !ready {
                        ui.label(
                            RichText::new("先选好本地 zip 文件")
                                .small()
                                .color(theme::warning()),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(ready && !self.kernel_busy, primary_btn("开始下载"))
                            .clicked()
                        {
                            let path = (source == KernelSource::Local)
                                .then(|| std::path::PathBuf::from(self.kernel_dialog_path.trim()));
                            start = Some((source, path));
                            open = false;
                        }
                        if ui.button("关闭").clicked() {
                            open = false;
                        }
                    });
                });
            });

        self.show_kernel_dialog = open;
        if let Some((source, path)) = start {
            self.update_kernel_from(kind, source, path, true);
        }
    }

}

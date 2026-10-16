use crate::gui::icons::Icon;
use crate::gui::theme;
use crate::gui::types::Tab;
use crate::gui::widgets::{format_duration_ms, header_cell, nav_item, stat_chip};
use crate::gui::VpnApp;
use crate::proxy::ProxyState;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

impl VpnApp {
    pub(crate) fn ui_sidebar(&mut self, ui: &mut Ui, target_tab: &mut Tab) {
        ui.visuals_mut().override_text_color = Some(theme::sidebar_text());
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let texture = self.logo_texture.get_or_insert_with(|| {
                let img = image::load_from_memory(include_bytes!("../../../assets/app-icon.png"))
                    .expect("load app-icon.png")
                    .to_rgba8();
                let (w, h) = img.dimensions();
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [w as usize, h as usize],
                    &img.into_raw(),
                );
                ui.ctx()
                    .load_texture("app-logo", color_image, egui::TextureOptions::LINEAR)
            });
            ui.add(
                egui::Image::new(&*texture)
                    .fit_to_exact_size(Vec2::splat(32.0))
                    .corner_radius(CornerRadius::same(6)),
            );
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("VPN Parser")
                            .strong()
                            .size(16.5)
                            .color(theme::sidebar_text()),
                    );
                    Frame::NONE
                        .fill(theme::accent_bg())
                        .stroke(Stroke::new(1.0_f32, theme::accent_bg()))
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::symmetric(4, 1))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("CORE")
                                    .size(9.0)
                                    .strong()
                                    .color(theme::accent()),
                            );
                        });
                });
                ui.label(
                    RichText::new("网络代理控制台")
                        .small()
                        .color(theme::sidebar_muted()),
                );
                // 有新版本才出现：绿色↑ + 版本号，点了直接去「关于」页更新。
                if let Some(tag) = self.pending_update_tag() {
                    if sidebar_update_marker(ui, &tag) {
                        *target_tab = Tab::About;
                    }
                }
            });
        });
        ui.add_space(14.0);
        let line_y = ui.cursor().top();
        ui.painter().hline(
            ui.max_rect().x_range(),
            line_y,
            Stroke::new(1.0_f32, theme::border()),
        );
        ui.add_space(10.0);

        nav_item(ui, target_tab, Tab::Proxy, Icon::Dashboard, "控制台");
        nav_item(ui, target_tab, Tab::Nodes, Icon::Lightning, "节点池");
        nav_item(ui, target_tab, Tab::Sources, Icon::Subscription, "订阅源");
        nav_item(ui, target_tab, Tab::Settings, Icon::Gear, "系统设置");
        nav_item(ui, target_tab, Tab::Api, Icon::Plug, "外部接口");
        nav_item(ui, target_tab, Tab::About, Icon::Info, "关于");

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(6.0);
            Frame::NONE
                .fill(theme::panel())
                .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                .corner_radius(CornerRadius::same(theme::RADIUS_CARD))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            // 内核都还没装上时"代理未启动"是废话：这叫预备。
                            let prepared = crate::gui::prepare::is_prepared(
                                self.kernel_installed_cached(),
                                matches!(
                                    self.proxy_status.state,
                                    ProxyState::Running
                                        | ProxyState::Degraded
                                        | ProxyState::Starting
                                ),
                            );
                            let (dot_color, status_text) = match self.proxy_status.state {
                                ProxyState::Running => (theme::success(), "代理运行中"),
                                ProxyState::Degraded => (theme::warning(), "代理降级运行"),
                                ProxyState::Starting => (theme::accent_hover(), "代理启动中..."),
                                ProxyState::Stopping => (theme::muted(), "代理停止中..."),
                                ProxyState::Failed => (theme::danger(), "代理启动失败"),
                                ProxyState::Stopped if prepared => {
                                    (theme::accent(), "预备 · 等待内核")
                                }
                                ProxyState::Stopped => (theme::sidebar_muted(), "代理未启动"),
                            };
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                            // Concentric glowing dot
                            ui.painter().circle_filled(
                                rect.center(),
                                5.0,
                                dot_color.gamma_multiply(0.25),
                            );
                            ui.painter().circle_filled(rect.center(), 3.0, dot_color);
                            ui.label(
                                RichText::new(status_text)
                                    .small()
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                        });
                        ui.add_space(4.0);
                        if let Some(s) = &self.stats {
                            ui.label(
                                RichText::new(format!(
                                    "源 {} · 节点 {} · 可用 {}",
                                    s.source_count, s.node_count, s.alive_count
                                ))
                                .small()
                                .color(theme::sidebar_muted()),
                            );
                        }
                    });
                });
            ui.add_space(6.0);
            let sep_y = ui.cursor().top();
            ui.painter().hline(
                ui.max_rect().x_range(),
                sep_y,
                Stroke::new(1.0_f32, theme::border()),
            );
        });
    }

    pub(crate) fn ui_task_panel(&self, ui: &mut Ui) {
        let Some(task) = self
            .tasks
            .snapshots(20)
            .into_iter()
            .find(|task| task.kind == crate::tasks::TaskKind::Fetch)
        else {
            return;
        };
        ui.label(
            RichText::new(format!(
                "当前任务：{:?}，进度 {}/{}，成功 {}，失败 {}，耗时 {} ms",
                task.kind,
                task.completed_sources,
                task.total_sources,
                task.successful_sources,
                task.failed_sources,
                task.elapsed_ms
            ))
            .strong(),
        );
        egui::ScrollArea::horizontal()
            .max_height(145.0)
            .show(ui, |ui| {
                egui::Grid::new("task_sources_grid")
                    .striped(true)
                    .num_columns(7)
                    .show(ui, |ui| {
                        for title in ["订阅", "阶段", "节点", "下载", "解析", "写入", "测速/结果"]
                        {
                            header_cell(ui, title);
                        }
                        ui.end_row();
                        for source in task.sources {
                            ui.label(source.source_name);
                            ui.label(format!("{:?}", source.phase));
                            ui.label(source.node_count.to_string());
                            ui.label(format_duration_ms(source.download_ms.or_else(|| {
                                matches!(source.phase, crate::tasks::TaskPhase::Downloading)
                                    .then_some(source.elapsed_ms)
                            })));
                            ui.label(format_duration_ms(source.parse_ms));
                            ui.label(format_duration_ms(source.write_ms));
                            ui.label(
                                source
                                    .error
                                    .or(source.message)
                                    .unwrap_or_else(|| format_duration_ms(source.speedtest_ms)),
                            );
                            ui.end_row();
                        }
                    });
            });
    }

    pub(crate) fn ui_stats_row(&mut self, ui: &mut Ui) {
        if let Some(s) = &self.stats.clone() {
            let is_nodes_tab = self.tab == Tab::Nodes;
            ui.horizontal_wrapped(|ui| {
                stat_chip(ui, "订阅源", &s.source_count.to_string(), theme::accent());
                stat_chip(
                    ui,
                    "启用",
                    &s.enabled_source_count.to_string(),
                    theme::success(),
                );
                stat_chip(ui, "节点", &s.node_count.to_string(), theme::accent());
                stat_chip(ui, "已测", &s.tested_count.to_string(), theme::warning());
                stat_chip(ui, "可用", &s.alive_count.to_string(), theme::success());

                // 仅在「节点池」上方展示协议首字母缩写图标 + 数量
                if is_nodes_tab {
                    let mut switch_proto = None;
                    for p in &s.protocol_stats {
                        let is_active = self.protocol_filter.eq_ignore_ascii_case(&p.protocol);
                        if crate::gui::widgets::protocol_stat_chip(ui, &p.protocol, p.count, is_active).clicked() {
                            if is_active {
                                switch_proto = Some("".to_string());
                            } else {
                                switch_proto = Some(p.protocol.clone());
                            }
                        }
                    }
                    if let Some(proto) = switch_proto {
                        self.protocol_filter = proto;
                        self.reload_node_view();
                    }
                }
            });
        }
    }
}

/// 侧栏里"发现新版本"那一行。抽成自由函数是因为 parked 的诊断窗口 PrintWindow 只能
/// 抓到全黑帧 —— 想证明它真的画出来、真的是绿色，只能把这段单独喂给离屏 Context 读形状。
/// 返回是否被点击（点了跳「关于」页）。
pub(crate) fn sidebar_update_marker(ui: &mut Ui, tag: &str) -> bool {
    let row = ui.horizontal(|ui| {
        ui.add_space(1.0);
        ui.label(
            RichText::new('\u{2191}')
                .strong()
                .size(12.5)
                .color(theme::success()),
        );
        ui.label(
            RichText::new(format!("{tag} 可更新"))
                .small()
                .strong()
                .color(theme::success()),
        );
    });
    row.response
        .interact(Sense::click())
        .on_hover_text(format!("发现新版本 {tag}，点击去「关于」页下载更新"))
        .clicked()
}
#[cfg(test)]
mod update_marker_probe_tests {
    use super::sidebar_update_marker;
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 在 220 宽的侧栏里真画一次这行标记，返回（内容右边界, 这一帧真的画出来的文字）。
    fn probe(tag: &str) -> (f32, String) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1330.0, 800.0),
        ));
        let mut right = 0.0_f32;
        let mut painted = String::new();
        let _ = ctx.run(input, |ctx| {
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::WHITE)
                        .inner_margin(Margin::symmetric(12, 10)),
                )
                .show(ctx, |ui| {
                    let layer = ui.layer_id();
                    let _clicked = sidebar_update_marker(ui, tag);
                    right = ui.min_rect().max.x;
                    painted = ctx.graphics_mut(|g| {
                        let mut out = String::new();
                        if let Some(list) = g.get(layer) {
                            for shape in list.all_entries() {
                                if let egui::Shape::Text(text) = &shape.shape {
                                    out.push_str(&text.galley.text());
                                    out.push(' ');
                                }
                            }
                        }
                        out
                    });
                });
        });
        (right, painted)
    }

    /// 这行标记必须真的画出来（箭头 + "vX.Y.Z 可更新"），并且不撑破 220 宽的侧栏。
    /// "东西被挤出界面"这一类问题他报过好几轮，画完就读这一帧的形状清单对账。
    #[test]
    fn the_marker_row_paints_and_fits_the_sidebar() {
        for tag in ["v1.4.0", "v1.4.0-beta.2", "v2.10.13"] {
            let (right, painted) = probe(tag);
            assert!(
                painted.contains(tag),
                "{tag} 时侧栏根本没画出这一行（本帧文字：{painted:?}）"
            );
            assert!(
                painted.contains(&format!("{tag} 可更新")),
                "{tag} 时文字不完整：{painted:?}"
            );
            assert!(
                painted.contains(char::from_u32(0x2191).unwrap()),
                "{tag} 时那个上箭头没画出来：{painted:?}"
            );
            assert!(
                right <= 220.0 - 12.0 + 0.5,
                "{tag} 时这行画到 x={right:.1}，超出侧栏内容右界 {:.1}",
                220.0 - 12.0
            );
        }
    }
}

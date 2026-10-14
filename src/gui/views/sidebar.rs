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
                // 侧栏本身是纯白，这块状态盒要用页面那档深一点的蓝灰，否则白上白看不见。
                .fill(theme::page_bg())
                .stroke(Stroke::new(1.0_f32, theme::border()))
                .corner_radius(CornerRadius::same(6))
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

    pub(crate) fn ui_stats_row(&self, ui: &mut Ui) {
        if let Some(s) = &self.stats {
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
                for p in s.protocol_stats.iter().take(6) {
                    stat_chip(ui, &p.protocol, &p.count.to_string(), theme::muted());
                }
            });
        }
    }
}

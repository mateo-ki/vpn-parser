use crate::gui::icons::Icon;
use crate::gui::theme;
use crate::gui::types::Tab;
use crate::gui::widgets::{format_duration_ms, header_cell, nav_item_icon_only, stat_chip};
use crate::gui::VpnApp;
use crate::proxy::ProxyState;
use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, Ui, Vec2};

impl VpnApp {
    pub(crate) fn ui_sidebar(&mut self, ui: &mut Ui, target_tab: &mut Tab) {
        ui.visuals_mut().override_text_color = Some(theme::sidebar_text());
        ui.add_space(8.0);

        // App Logo 居中呈现 (去除旧版更新小字，保持顶部纯净)
        ui.vertical_centered(|ui| {
            let texture = self.ensure_logo_texture(ui.ctx());
            ui.add(
                egui::Image::new(&texture)
                    .fit_to_exact_size(Vec2::splat(36.0))
                    .corner_radius(CornerRadius::same(8)),
            );
        });

        ui.add_space(10.0);
        let line_y = ui.cursor().top();
        ui.painter().hline(
            ui.max_rect().x_range(),
            line_y,
            Stroke::new(1.0_f32, theme::border_subtle()),
        );
        ui.add_space(8.0);

        let is_running = matches!(self.proxy_status.state, ProxyState::Running);
        let proxy_dot = if is_running { Some(theme::accent()) } else { None };
        let nodes_tip = self
            .stats
            .as_ref()
            .map(|s| format!("节点池 ({} 节点)", s.node_count))
            .unwrap_or_else(|| self.tr("节点池").to_string());
        let sources_tip = self
            .stats
            .as_ref()
            .map(|s| format!("订阅源 ({} 个)", s.source_count))
            .unwrap_or_else(|| self.tr("订阅源").to_string());

        // 收起态只保留语义图标（悬停给完整提示）；展开态在图标右边补一项短名。
        let expanded = self.settings.sidebar_expanded;
        nav_item_icon_only(
            ui,
            target_tab,
            Tab::Proxy,
            Icon::Dashboard,
            self.tr("控制台"),
            proxy_dot,
            if expanded { Some("控制台") } else { None },
        );
        nav_item_icon_only(
            ui,
            target_tab,
            Tab::Nodes,
            Icon::Lightning,
            &nodes_tip,
            None,
            if expanded { Some("节点池") } else { None },
        );
        nav_item_icon_only(
            ui,
            target_tab,
            Tab::Sources,
            Icon::Subscription,
            &sources_tip,
            None,
            if expanded { Some("订阅源") } else { None },
        );
        nav_item_icon_only(
            ui,
            target_tab,
            Tab::Extensions,
            Icon::Package,
            self.tr("扩展功能"),
            None,
            if expanded { Some("扩展功能") } else { None },
        );
        nav_item_icon_only(
            ui,
            target_tab,
            Tab::Settings,
            Icon::Gear,
            self.tr("系统设置"),
            None,
            if expanded { Some("系统设置") } else { None },
        );
        nav_item_icon_only(
            ui,
            target_tab,
            Tab::About,
            Icon::Info,
            self.tr("关于"),
            None,
            if expanded { Some("关于") } else { None },
        );

        // 菜单栏最底部：日间 / 夜间主题切换与版本更新提醒
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            let is_dark = self.settings.ui_dark;
            let (icon, tip) = if is_dark {
                (Icon::Sun, self.tr("切换至日间亮色模式"))
            } else {
                (Icon::Moon, self.tr("切换至夜间深色模式"))
            };

            let item_size = egui::vec2(ui.available_width(), 38.0);
            let (rect, resp) = ui.allocate_exact_size(item_size, Sense::click());
            let resp = resp.on_hover_text(tip);

            let painter = ui.painter_at(rect);
            if resp.hovered() {
                painter.rect_filled(
                    rect,
                    CornerRadius::same(theme::RADIUS_NAV),
                    theme::panel().gamma_multiply(0.7),
                );
            }

            // 图标尺寸加大至 21.0px (更饱满协调)
            let icon_rect = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(21.0));
            let icon_color = if resp.hovered() {
                theme::accent()
            } else {
                theme::icon_secondary()
            };
            crate::gui::icons::draw_icon(&painter, icon_rect, icon, icon_color);

            if resp.clicked() {
                self.settings.ui_dark = !self.settings.ui_dark;
                self.edit_ui_dark = self.settings.ui_dark;
                self.save_settings();
            }

            // 在日间/夜间模式按钮正上方：呈现高辨识度更新徽章（方案 C：科技翡翠绿就绪微标）
            if let Some(tag) = self.pending_update_tag() {
                ui.add_space(6.0);
                let update_size = egui::vec2(ui.available_width(), 38.0);
                let (rect, resp) = ui.allocate_exact_size(update_size, Sense::click());
                let resp = resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("发现新版本 {tag}，点击去「关于」页下载更新"));

                let painter = ui.painter_at(rect);
                let is_hovered = resp.hovered();

                // 方案 C：科技翡翠绿就绪微光底框与轮廓
                let bg_fill = if is_hovered {
                    Color32::from_rgba_unmultiplied(16, 185, 129, 72)
                } else {
                    Color32::from_rgba_unmultiplied(16, 185, 129, 38)
                };
                let stroke_color = if is_hovered {
                    Color32::from_rgb(16, 185, 129)
                } else {
                    Color32::from_rgba_unmultiplied(16, 185, 129, 115)
                };

                painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_NAV), bg_fill);
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(theme::RADIUS_NAV),
                    Stroke::new(1.0_f32, stroke_color),
                    egui::StrokeKind::Inside,
                );

                // 居中绘制 20px 向上大箭头
                let arrow_color = if is_hovered {
                    Color32::from_rgb(110, 231, 183)
                } else {
                    Color32::from_rgb(52, 211, 153)
                };
                let arrow_rect = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(20.0));
                crate::gui::icons::draw_icon(&painter, arrow_rect, Icon::ArrowUp, arrow_color);

                // 右上角科技翡翠绿发光微标圆点
                let dot_pos = egui::pos2(rect.right() - 8.0, rect.top() + 7.0);
                let dot_color = Color32::from_rgb(16, 185, 129);
                painter.circle_filled(dot_pos, 4.0, dot_color.gamma_multiply(0.35));
                painter.circle_filled(dot_pos, 2.5, dot_color);

                if resp.clicked() {
                    *target_tab = Tab::About;
                }
            }

            // 更新徽章再往上一格：菜单栏展开/收起开关。状态写进 settings.sidebar_expanded，
            // 下次启动还是他上回选的样子（和日间/夜间那颗一样走 save_settings）。
            ui.add_space(6.0);
            let (fold_icon, fold_tip) = if self.settings.sidebar_expanded {
                (Icon::SidebarCollapse, self.tr("收起菜单（只留图标）"))
            } else {
                (Icon::SidebarExpand, self.tr("展开菜单（图标 + 文字）"))
            };
            let fold_size = egui::vec2(ui.available_width(), 38.0);
            let (fold_rect, fold_resp) = ui.allocate_exact_size(fold_size, Sense::click());
            let fold_resp = fold_resp.on_hover_text(fold_tip);
            let fold_painter = ui.painter_at(fold_rect);
            if fold_resp.hovered() {
                fold_painter.rect_filled(
                    fold_rect,
                    CornerRadius::same(theme::RADIUS_NAV),
                    theme::panel().gamma_multiply(0.7),
                );
            }
            let fold_color = if self.settings.sidebar_expanded {
                theme::accent()
            } else if fold_resp.hovered() {
                theme::icon_primary()
            } else {
                theme::icon_secondary()
            };
            let fold_icon_rect =
                egui::Rect::from_center_size(fold_rect.center(), egui::Vec2::splat(21.0));
            crate::gui::icons::draw_icon(&fold_painter, fold_icon_rect, fold_icon, fold_color);
            if fold_resp.clicked() {
                self.settings.sidebar_expanded = !self.settings.sidebar_expanded;
                self.edit_sidebar_expanded = self.settings.sidebar_expanded;
                self.save_settings();
            }

            ui.add_space(6.0);
            let line_y = ui.cursor().top();
            ui.painter().hline(
                ui.max_rect().x_range(),
                line_y,
                Stroke::new(1.0_f32, theme::border_subtle()),
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

    #[allow(dead_code)]
    pub(crate) fn ui_stats_row(&mut self, ui: &mut Ui) {
        let mut switch_proto = None;
        if let Some(s) = self.stats.as_ref() {
            let is_nodes_tab = self.tab == Tab::Nodes;
            let current_proto = self.protocol_filter.clone();
            ui.horizontal_wrapped(|ui| {
                stat_chip(ui, "订阅源", &s.source_count.to_string(), theme::accent());
                stat_chip(
                    ui,
                    "启用",
                    &s.enabled_source_count.to_string(),
                    theme::accent_strong(),
                );
                stat_chip(ui, "节点", &s.node_count.to_string(), theme::accent());
                stat_chip(ui, "已测", &s.tested_count.to_string(), theme::warning());
                stat_chip(ui, "可用", &s.alive_count.to_string(), theme::accent_strong());

                // 仅在「节点池」上方展示协议首字母缩写图标 + 数量
                if is_nodes_tab {
                    for p in &s.protocol_stats {
                        let is_active = current_proto.eq_ignore_ascii_case(&p.protocol);
                        if crate::gui::widgets::protocol_stat_chip(ui, &p.protocol, p.count, is_active).clicked() {
                            if is_active {
                                switch_proto = Some("".to_string());
                            } else {
                                switch_proto = Some(p.protocol.clone());
                            }
                        }
                    }
                }
            });
        }
        if let Some(proto) = switch_proto {
            self.protocol_filter = proto;
            self.page = 1;
            self.reload_nodes();
        }
    }
}

/// 侧栏里"发现新版本"那一行。抽成自由函数是因为 parked 的诊断窗口 PrintWindow 只能
/// 抓到全黑帧 —— 想证明它真的画出来、真的是绿色，只能把这段单独喂给离屏 Context 读形状。
/// 返回是否被点击（点了跳「关于」页）。
#[cfg(test)]
pub(crate) fn sidebar_update_marker(ui: &mut Ui, tag: &str) -> bool {
    let row = ui.horizontal(|ui| {
        ui.add_space(1.0);
        ui.label(
            RichText::new('\u{2191}')
                .strong()
                .size(12.5)
                .color(theme::accent_strong()),
        );
        ui.label(
            RichText::new(format!("{tag} 可更新"))
                .small()
                .strong()
                .color(theme::accent_strong()),
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

    /// 导航项在两种宽度/两种状态下真画一遍。返回行矩形与文字墨迹都用**相对行左界**的量，
    /// 因为离屏 Context 里面板内容原点未必是 (0,0) —— 绝对坐标会把布局判成跑偏。
    #[derive(Debug)]
    struct NavProbe {
        row_left: f32,
        row_right: f32,
        text_left: f32,
        text_right: f32,
        painted: String,
    }

    fn nav_probe(width: f32, label: Option<&str>) -> NavProbe {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1330.0, 800.0),
        ));
        let mut probe = NavProbe {
            row_left: 0.0,
            row_right: 0.0,
            text_left: f32::MAX,
            text_right: 0.0,
            painted: String::new(),
        };
        let _ = ctx.run(input, |ctx| {
            egui::SidePanel::left("sidebar_nav_probe")
                .exact_width(width)
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::WHITE)
                        .inner_margin(Margin::symmetric(8, 10)),
                )
                .show(ctx, |ui| {
                    let layer = ui.layer_id();
                    let mut tab = crate::gui::types::Tab::Proxy;
                    crate::gui::widgets::nav_item_icon_only(
                        ui,
                        &mut tab,
                        crate::gui::types::Tab::Proxy,
                        crate::gui::icons::Icon::Dashboard,
                        "控制台",
                        None,
                        label,
                    );
                    let row = ui.min_rect();
                    probe.row_left = row.min.x;
                    probe.row_right = row.max.x;
                    let (left, right, text) = ctx.graphics(|g| {
                        let mut left = f32::MAX;
                        let mut right = 0.0_f32;
                        let mut text = String::new();
                        if let Some(list) = g.get(layer) {
                            for shape in list.all_entries() {
                                if let egui::Shape::Text(t) = &shape.shape {
                                    // TextShape 的 galley.rect 不带 pos 偏移（画的时候才平移），
                                    // 量墨迹位置只能用 pos + galley 尺寸。
                                    left = left.min(t.pos.x);
                                    right = right.max(t.pos.x + t.galley.size().x);
                                    text.push_str(&t.galley.text());
                                    text.push(' ');
                                }
                            }
                        }
                        (left, right, text)
                    });
                    probe.text_left = left;
                    probe.text_right = right;
                    probe.painted = text;
                });
        });
        probe
    }

    /// 展开 = 图标右边真的多出一段文字，而且文字落在图标留出的位置之后、不越过行右界；
    /// 收起 = 一个字都不许多。
    #[test]
    fn the_nav_row_shows_labels_only_when_expanded() {
        let open = nav_probe(crate::gui::SIDEBAR_EXPANDED_WIDTH, Some("控制台"));
        assert!(
            open.painted.contains("控制台"),
            "展开态没画出短名：{open:?}"
        );
        let row_w = open.row_right - open.row_left;
        assert!(
            open.text_left - open.row_left >= 30.0,
            "文字贴在行左界，没让开图标：{open:?}"
        );
        assert!(
            open.text_right <= open.row_left + row_w - 10.0,
            "文字越过行右界：{open:?}"
        );

        let wide = nav_probe(crate::gui::SIDEBAR_EXPANDED_WIDTH, None);
        assert!(
            !wide.painted.contains("控制台"),
            "收起态不该有文字：{wide:?}"
        );
        let narrow = nav_probe(64.0, None);
        assert!(
            !narrow.painted.contains("控制台"),
            "64 宽收起态不该有文字：{narrow:?}"
        );
    }

    /// 真名最长的"扩展功能"（四个字）必须整个画得下 —— 展开宽度是按它算的，
    /// 哪天把宽度调窄了这条会红，而不是悄悄把名字截成"扩展功…"。
    #[test]
    fn the_longest_real_label_still_fits_the_expanded_width() {
        let probe = nav_probe(crate::gui::SIDEBAR_EXPANDED_WIDTH, Some("扩展功能"));
        assert!(
            probe.painted.contains("扩展功能"),
            "展开宽度下「扩展功能」没整个画出来：{probe:?}"
        );
        let row_w = probe.row_right - probe.row_left;
        assert!(
            probe.text_right <= probe.row_left + row_w - 10.0,
            "「扩展功能」越出行右界：{probe:?}"
        );
    }

    /// 名字再长也必须在行内截断 —— 这类"东西被挤出界面"他报过好几轮。
    #[test]
    fn a_long_nav_label_is_cut_off_inside_the_row() {
        let width = crate::gui::SIDEBAR_EXPANDED_WIDTH;
        let long = nav_probe(width, Some("扩展功能 / 分流规则 / 端口映射"));
        assert!(!long.painted.is_empty(), "长名字一个像素都没画出来：{long:?}");
        let row_w = long.row_right - long.row_left;
        assert!(
            long.text_right <= long.row_left + row_w - 10.0,
            "长名字越过行右界：{long:?}"
        );
        assert!(
            long.text_left - long.row_left >= 30.0,
            "长名字压到了图标上：{long:?}"
        );
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

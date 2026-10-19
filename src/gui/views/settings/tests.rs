use super::*;

mod layout_tests {
    use eframe::egui::{self, Label, RawInput, Rect, Vec2};

    /// 画一段表头，然后报告纵向 ui 里还剩多少高度给下面的卡片。
    fn height_left_after(header: impl Fn(&mut egui::Ui)) -> f32 {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1200.0, 800.0),
        ));
        let mut left = -1.0;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                header(ui);
                left = ui.available_height();
            });
        });
        left
    }

    #[test]
    fn right_to_left_directly_on_a_vertical_ui_eats_the_page() {
        // 这就是"系统设置页只剩标题"的成因：`with_layout` 放在纵向 ui 上时，
        // 子 ui 的 max_rect 一直延伸到父级底部，父级按 min_rect 推进游标 —— 整页高度归它。
        // 写成断言而不是注释，是因为它会"编译通过、只是页面空掉"。
        let eaten = height_left_after(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let _ = ui.button("保存设置");
                let _ = ui.button("重置修改");
            });
        });
        assert!(eaten < 1.0, "这个坑应该已经把整页吃光，实测剩 {eaten:.1}");
    }

    #[test]
    fn settings_header_leaves_room_for_the_cards() {
        let plain = height_left_after(|ui| {
            ui.horizontal(|ui| {
                ui.label("系统设置");
                ui.label("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务");
            });
        });
        // ui_settings 现在的两行表头：标题一行，右对齐动作条包在 horizontal 里另一行。
        let shipped = height_left_after(|ui| {
            ui.horizontal(|ui| {
                ui.label("系统设置");
                ui.add(Label::new("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务").truncate());
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = ui.button("保存设置");
                    let _ = ui.button("重置修改");
                    ui.label("✓ 已同步");
                });
            });
        });
        assert!(plain > 700.0, "对照组本身不对: {plain:.1}");
        assert!(
            shipped > plain * 0.9,
            "表头把卡片的空间吃掉了：shipped={shipped:.1} plain={plain:.1}"
        );
    }
}

#[cfg(test)]
mod header_probe_tests {
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻系统设置页那一列的结构：CentralPanel(左右 18 内边距) → 标题行 →
    /// 右对齐动作行 → ScrollArea(里面是不换行的一排统计芯片)。
    /// `pinned` = 动作组和标题同一行（标题侧限宽截断）的新写法。
    /// 返回（窗口右边界, 保存设置左/右边界, 芯片行右边界）。
    fn probe(
        window_width: f32,
        chips: usize,
        chips_before_header: bool,
        pinned: bool,
    ) -> (f32, f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 890.0),
        ));
        let mut save_right = 0.0_f32;
        let mut save_left = 0.0_f32;
        let mut chips_right = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(egui::Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    if chips_before_header {
                        ui.horizontal_wrapped(|ui| {
                            for _ in 0..chips {
                                let _ = ui.button("芯片 12345");
                            }
                        });
                        chips_right = ui.min_rect().max.x;
                    }
                    if pinned {
                        // 直接调产品那份 helper，探针测的就是真代码。
                        crate::gui::widgets::split_row(
                            ui,
                            "settings:header",
                            300.0,
                            |ui| {
                                let _ = ui.label("系统设置");
                                let _ = ui.add(
                                    egui::Label::new(
                                        "应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务",
                                    )
                                    .truncate(),
                                );
                            },
                            |ui| {
                                let save = ui.button("保存设置");
                                save_right = save.rect.max.x;
                                save_left = save.rect.min.x;
                                let _ = ui.button("重置修改");
                                let _ = ui.label("⚠ 有未保存修改");
                            },
                        );
                    } else {
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("系统设置");
                            let _ = ui.label("应用行为 · 定时拉取 · 测速参数 · 网络与自愈服务");
                        });
                        let _ = ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let save = ui.button("保存设置");
                                    save_right = save.rect.max.x;
                                    save_left = save.rect.min.x;
                                    let _ = ui.button("重置修改");
                                    let _ = ui.label("⚠ 有未保存修改");
                                },
                            );
                        });
                    }
                    if !chips_before_header {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for _ in 0..chips {
                                        let _ = ui.button("芯片 12345");
                                    }
                                });
                                chips_right = ui.min_rect().max.x;
                            });
                    }
                });
        });
        (window_width, save_left, save_right, chips_right)
    }

    /// 表头那组「保存设置 / 重置修改」必须整个留在中央面板里 —— 不管前面那排统计芯片
    /// 换不换行、窗口多窄、字体调到多大（缩放只是把逻辑宽度挤小）。
    /// 用户 2026-10-07 报的是按钮被窗口右缘裁掉，2026-10-14 把字体调到 140% 又报同样症状，
    /// 所以这次连"左边缘也得在面板内"一起钉住：右对齐组溢出时是先从左边被切掉的。
    #[test]
    fn save_button_stays_inside_the_panel_at_every_width() {
        for (width, chips, before) in [
            (1298.0, 11, true),
            (1298.0, 11, false),
            (1298.0, 3, true),
            (1000.0, 11, true),
            (760.0, 11, true),
            (620.0, 11, true),
            (520.0, 11, true),
            (420.0, 11, true),
        ] {
            let (w, left, right, row) = probe(width, chips, before, true);
            // CentralPanel 在 220 宽的侧栏右边，左右各 18 内边距。
            let panel_left = 220.0 + 18.0;
            let panel_right = w - 18.0;
            assert!(
                right <= panel_right + 0.5,
                "窗口 {w:.0}（芯片 {chips} 个，表头前={before}）时保存设置画到 {right:.1}，越过面板右边界 {panel_right:.1}"
            );
            assert!(
                (right - panel_right).abs() < 1.0,
                "窗口 {w:.0} 时保存设置右边缘 {right:.1} 没有贴住面板右边界 {panel_right:.1}"
            );
            assert!(
                left >= panel_left - 0.5,
                "窗口 {w:.0} 时保存设置的左边缘在 {left:.1}，被面板左边界 {panel_left:.1} 切掉了"
            );
            assert!(
                row <= panel_right + 0.5,
                "窗口 {w:.0} 时芯片行画到 {row:.1}，越过面板右边界 {panel_right:.1}"
            );
        }
    }

    /// 反向验证：老写法（标题一行、右对齐组另起一行）在窄面板下**确实**会把按钮推出右边界，
    /// 所以上面那条不是空跑。2026-10-15 量到的原始症状是 520 宽时按钮画到 580。
    #[test]
    fn the_old_two_row_header_really_did_overflow() {
        let (w, _left, right, _row) = probe(520.0, 11, true, false);
        assert!(
            right > w - 18.0 + 0.5,
            "老写法在 {w:.0} 宽没溢出（右边缘 {right:.1}）—— 那这条反向验证失去意义了"
        );
    }
}
#[cfg(test)]
mod zoom_semantics_tests {
    use eframe::egui::{self, Context, RawInput, Rect, Vec2};

    /// 缩放到底会不会把"可用的逻辑宽度"变小？这决定了 `set_zoom_factor` 能不能当字号开关用。
    /// 读的是 egui 自己的 `screen_rect()`，不是我们的布局代码。
    #[test]
    fn what_zoom_does_to_the_logical_screen_width() {
        let ctx = Context::default();
        let mut widths = Vec::new();
        for frame in 0..3 {
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1330.0, 800.0),
            ));
            let _ = ctx.run(input, |ctx| {
                if frame == 1 {
                    ctx.set_zoom_factor(1.1);
                }
                widths.push((ctx.screen_rect().width(), ctx.pixels_per_point()));
            });
        }
        // zoom 是"下一帧才生效"的：设了之后那一帧仍旧按 1330 逻辑点排。
        assert!(
            (widths[0].0 - 1330.0).abs() < 1.0 && (widths[0].1 - 1.0).abs() < 0.01,
            "{widths:?}"
        );
        // 生效以后逻辑宽度按 zoom 缩小（1330/1.1=1209），这就是"字体调大 = 可用宽度变小"
        // 的唯一来源 —— 界面上溢出与否，得按这个逻辑宽度去算，不是物理像素。
        assert!(
            (widths[2].0 - 1209.1).abs() < 0.5 && (widths[2].1 - 1.10).abs() < 0.01,
            "{widths:?}"
        );
    }
}

#[cfg(test)]
mod stats_row_overflow_probe_tests {
    use crate::gui::widgets::{split_row, stat_chip};
    use eframe::egui::{self, Color32, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻真实结构：CentralPanel(内边距 18,14) → 统计芯片行(horizontal_wrapped)
    /// → 表头 split_row。量的是"表头右组到底钉在哪"，以及"芯片行有没有绕过右边界"。
    fn probe(window: Vec2, chips: usize) -> (f32, f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, window));
        let mut panel_right = 0.0_f32;
        let mut group_right = 0.0_f32;
        let mut chips_right = 0.0_f32;
        let mut save_right = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::TopBottomPanel::top("title_bar")
                .exact_height(34.0)
                .frame(Frame::NONE.fill(Color32::WHITE))
                .show(ctx, |ui| {
                    let _ = ui.label("VPN Parser");
                });
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .show(ctx, |ui| {
                    let _ = ui.label("导航");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        stat_chip(ui, "订阅源", "36", Color32::GRAY);
                        stat_chip(ui, "启用", "36", Color32::GRAY);
                        stat_chip(ui, "节点", "13840", Color32::GRAY);
                        stat_chip(ui, "已测", "5304", Color32::GRAY);
                        stat_chip(ui, "可用", "159", Color32::GRAY);
                        for i in 0..chips {
                            stat_chip(ui, &format!("协议{i}"), "8898", Color32::GRAY);
                        }
                    });
                    chips_right = ui.min_rect().max.x;
                    // 关键：芯片行溢出之后，max_rect 自己有没有被撑大？
                    panel_right = ui.max_rect().right();
                    ui.add_space(10.0);
                    split_row(
                        ui,
                        "settings:header",
                        300.0,
                        |ui| {
                            let _ = ui.label("系统设置");
                        },
                        |ui| {
                            let save = ui.button("保存设置");
                            save_right = save.rect.max.x;
                            group_right = ui.min_rect().max.x;
                        },
                    );
                });
        });
        (
            panel_right,
            group_right.max(save_right),
            chips_right,
            window.x,
        )
    }

    /// 芯片行**一行放不下就得换行**：`Frame::show` 版的芯片不参与换行判定，整行冲出
    /// 右边界还会把父 ui 的 `max_rect` 撑大（实测 1535 > 面板 1312），于是下面所有
    /// "钉右"的表头都跟着画到屏幕外 —— 2026-10-15 他报的"保存设置只看得见一半"。
    #[test]
    fn the_chips_row_never_widens_the_page_it_sits_on() {
        for chips in [0usize, 3, 6, 9, 12] {
            let (panel_right, group_right, chips_right, _) = probe(Vec2::new(1330.0, 800.0), chips);
            assert!(
                chips_right <= panel_right + 0.5,
                "{chips} 个协议芯片时统计行画到 {chips_right:.1}，越过面板右边界 {panel_right:.1}（不换行就会撑大 max_rect）"
            );
            assert!(
                (group_right - panel_right).abs() < 1.0,
                "{chips} 个芯片时表头右组在 {group_right:.1}，没贴住面板右边界 {panel_right:.1}"
            );
        }
    }
}
#[cfg(test)]
mod slider_tick_probe_tests {
    use super::{font_scale_ticks, handle_travel_range, FONT_SCALE_MAX, FONT_SCALE_MIN};
    use eframe::egui::{self, Frame, RawInput, Rect, Vec2};

    /// 真加一个 Slider，读出 egui **自己画出来的手柄中心**，再和我们的刻度 x 对账。
    /// 返回每个百分比对应的（手柄中心 x, 刻度 x, 误差）。
    fn probe(value: i32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(900.0, 600.0),
        ));
        let mut handle_center = 0.0_f32;
        let mut tick_x = 0.0_f32;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default()
                .frame(Frame::NONE.inner_margin(8.0))
                .show(ctx, |ui| {
                    let layer = ui.layer_id();
                    let slider_width = ui.spacing().slider_width;
                    ui.spacing_mut().slider_width = 300.0;
                    let mut percent = value;
                    let response = ui.add(egui::Slider::new(&mut percent, 85..=140).step_by(5.0));
                    ui.spacing_mut().slider_width = slider_width;
                    let travel = handle_travel_range(ui, &response, 300.0);
                    tick_x = font_scale_ticks(travel)
                        .into_iter()
                        .find(|(v, _, _)| *v == value)
                        .map(|(_, x, _)| x)
                        .unwrap_or(f32::NAN);
                    // 手柄是这一帧里唯一那个"高 ≈ 滑杆高、宽 ≈ 2×radius"的图元。
                    let shapes = ctx.graphics_mut(|g| {
                        g.get(layer)
                            .map(|list| list.all_entries().cloned().collect::<Vec<_>>())
                            .unwrap_or_default()
                    });
                    let radius = response.rect.height() / 2.5;
                    let mut best: Option<f32> = None;
                    for shape in shapes {
                        match shape.shape {
                            egui::Shape::Circle(circle) => {
                                if (circle.radius - radius).abs() < 2.0 {
                                    best = Some(circle.center.x);
                                }
                            }
                            egui::Shape::Rect(rect) => {
                                let w = rect.rect.width();
                                let h = rect.rect.height();
                                if (w - radius * 2.0).abs() < 3.0
                                    && (h - response.rect.height()).abs() < 3.0
                                {
                                    best = Some(rect.rect.center().x);
                                }
                            }
                            _ => {}
                        }
                    }
                    handle_center = best.unwrap_or(f32::NAN);
                });
        });
        (handle_center, tick_x)
    }

    #[test]
    fn the_ticks_sit_under_the_handle_egui_draws() {
        let mut lines = Vec::new();
        let mut worst = 0.0_f32;
        for value in [FONT_SCALE_MIN, 90, 100, 120, FONT_SCALE_MAX] {
            let (handle, tick) = probe(value);
            let diff = (handle - tick).abs();
            worst = worst.max(diff);
            lines.push(format!(
                "{value}%: 手柄 {handle:.1} 刻度 {tick:.1} 差 {diff:.1}"
            ));
        }
        eprintln!("{}", lines.join(" | "));
        assert!(
            worst < 1.5,
            "刻度与手柄最大偏差 {worst:.1}px / {}",
            lines.join(" | ")
        );
    }
}

#[cfg(test)]
mod local_time_tests {
    use crate::db::telemetry::switch_day;
    use crate::gui::widgets::clock_of;

    /// 出口切换记录里的时间必须是**本地（北京）时间**，不是库里那份 UTC。
    /// 断言按当前机器的时区算，不写死 +8 —— 否则 CI（UTC）上这条会假红。
    #[test]
    fn switch_times_are_shown_local_not_utc() {
        for raw in [
            "2026-10-14T19:24:05.696567800+00:00",
            "2026-10-14T19:24:05+00:00",
            "2026-10-14T19:24:05.123456+00:00",
        ] {
            let parsed = chrono::DateTime::parse_from_rfc3339(raw)
                .unwrap_or_else(|error| panic!("{raw} 解析不了: {error}"));
            let want = parsed
                .with_timezone(&chrono::Local)
                .format("%H:%M")
                .to_string();
            assert_eq!(clock_of(raw), want, "{raw} 没按本地时区显示");
            let day = parsed
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string();
            assert_eq!(switch_day(raw), day, "{raw} 的日头没按本地时区切");
        }
    }

    /// 库里万一混进非 RFC3339 的写法（SQLite 的 CURRENT_TIMESTAMP 就是空格版），
    /// 时间列也不许画出"2026-"这种半截日期。
    #[test]
    fn an_unparsable_timestamp_never_leaks_a_partial_date() {
        assert_eq!(clock_of("2026-10-14 19:24:05"), "19:24");
        assert_eq!(clock_of(""), "--:--");
        assert_eq!(clock_of("昨天下午"), "--:--");
    }
}

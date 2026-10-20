use super::*;
use eframe::egui::{self, CornerRadius, Color32, RawInput, Rect, Vec2};
use crate::gui::types::KernelGroup;
use crate::models::NodeSummary;

#[cfg(test)]
mod rule_import_help_probe_tests {
    use super::ui_rule_import_help_window;
    use eframe::egui::{self, RawInput, Rect, Vec2};

    /// 用产品同一份构造函数真开三帧「导入格式示范」窗口：
    /// 第 1 帧布局、第 2 帧读"看得见的那部分"、第 3 帧再读一次（确认滚动后仍在渲染）。
    /// 返回（窗口矩形, 第 2 帧文字, 第 3 帧文字）。
    fn probe(screen_width: f32) -> (Rect, String, String) {
        let ctx = egui::Context::default();
        let mut rect = Rect::NOTHING;
        let mut top = String::new();
        let mut bottom = String::new();
        for frame in 0..3 {
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(screen_width, 900.0),
            ));
            if frame == 2 {
                // 光标停在窗口里往下滚，够滚到底。
                let center = rect.center();
                input.events.push(egui::Event::PointerMoved(center));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, -2000.0),
                    modifiers: egui::Modifiers::default(),
                });
            }
            let mut this_frame = String::new();
            let _ = ctx.run(input, |ctx| {
                let (_msg, window_rect) = ui_rule_import_help_window(ctx);
                rect = window_rect;
                // 形状只能在同一帧里读：这一画在下一帧开始前就被 egui 清掉了。
                let ids: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
                this_frame = ctx.graphics_mut(|g| {
                    let mut out = String::new();
                    for id in &ids {
                        let Some(list) = g.get(*id) else {
                            continue;
                        };
                        for shape in list.all_entries() {
                            if let egui::Shape::Text(text) = &shape.shape {
                                out.push_str(&text.galley.text());
                                out.push('\n');
                            }
                        }
                    }
                    out
                });
            });
            match frame {
                1 => top = this_frame,
                2 => bottom = this_frame,
                _ => {}
            }
        }
        (rect, top, bottom)
    }

    /// 示范弹框必须：整窗留在屏幕里（字体调到 140% 时逻辑宽度只剩物理的 0.71 倍）、
    /// 类型清单与"打开就该看见"的前三段示范真的画出来、两个动作按钮也在。
    /// 断言读的是这一帧实际提交的文字，渲染没跑就等于红；
    /// 六段示范内容的正确性由 rule_io 里"逐段喂回解析器"的测试钉住。
    #[test]
    fn the_help_window_fits_and_paints_the_visible_examples() {
        let examples = crate::proxy::rule_io::import_examples();
        assert_eq!(examples.len(), 6, "示范段落数变了，弹框的说明文案要跟着改");
        for screen_width in [1298.0, 940.0, 660.0, 480.0] {
            let (rect, top, bottom) = probe(screen_width);
            assert!(
                rect.max.x <= screen_width + 0.5,
                "{screen_width:.0} 宽屏面上窗口画到 {:.1}，超出右边界",
                rect.max.x
            );
            assert!(
                rect.height() > 200.0,
                "窗口只有 {:.1} 高，示范没渲染",
                rect.height()
            );
            for (name, text) in [("未滚动", &top), ("滚动后", &bottom)] {
                assert!(
                    text.len() > 200,
                    "{screen_width:.0} 宽时{name}那一帧只画出 {} 字",
                    text.len()
                );
            }
            let painted = format!("{top}\n{bottom}");
            assert!(
                painted.contains("认这些类型") && painted.contains("点名跳过"),
                "{screen_width:.0} 宽时类型清单没画出来"
            );
            // 卡片比原来的裸文本各高 ~30pt：宽屏（>=800）能不滚动看见前三段，
            // 更窄（660/480 等，对应高缩放）只保证前两段，剩下的靠滚动。
            let visible = if screen_width >= 800.0 { 3 } else { 2 };
            for example in &examples[..visible] {
                assert!(
                    painted.contains(example.title),
                    "{screen_width:.0} 宽时示范「{}」没画出来",
                    example.title
                );
            }
            assert!(
                painted.contains("填入导入框") && painted.contains("复制这段"),
                "{screen_width:.0} 宽时示范的操作按钮没画出来"
            );
        }
    }
}

#[cfg(test)]
mod rule_hits_height_probe_tests {
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻「命中记录」那一列的结构，量滚动区实际拿到的视口高度。
    /// `strip_note` = 把"从本进程启动开始统计…"那行并掉；`reserve` = 视口预留量。
    fn probe(window: Vec2, strip_note: bool, reserve: f32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, window));
        let mut viewport = 0.0_f32;
        let mut card_bottom = 0.0_f32;
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
            egui::TopBottomPanel::bottom("status")
                .exact_height(32.0)
                .frame(Frame::NONE.fill(egui::Color32::GRAY))
                .show(ctx, |ui| {
                    let _ = ui.label("状态栏");
                });
            egui::TopBottomPanel::bottom("control")
                .exact_height(48.0)
                .frame(Frame::NONE.fill(egui::Color32::DARK_GRAY))
                .show(ctx, |ui| {
                    let _ = ui.button("启动代理");
                });
            egui::CentralPanel::default()
                .frame(
                    Frame::NONE
                        .fill(egui::Color32::LIGHT_GRAY)
                        .inner_margin(Margin::symmetric(18, 14)),
                )
                .show(ctx, |ui| {
                    let _ = ui.horizontal(|ui| {
                        let _ = ui.button("规则配置");
                        let _ = ui.button("命中记录");
                    });
                    let _ = ui.separator();
                    ui.add_space(6.0);
                    let card = Frame::NONE
                        .fill(egui::Color32::WHITE)
                        .inner_margin(Margin::same(22));
                    let shown = card.show(ui, |ui| {
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("规则命中记录");
                            let _ = ui.label("123 条连接 · 兜底 MATCH 20 条（16%）");
                            let _ = ui.button("立即刷新");
                        });
                        ui.add_space(2.0);
                        if !strip_note {
                            ui.label("从本进程启动开始统计；占比按连接数算，一条连接只计一次。");
                            ui.add_space(8.0);
                        }
                        let _ = ui.horizontal(|ui| {
                            let _ = ui.label("命中的规则");
                            let _ = ui.label("派给");
                            let _ = ui.label("条数 / 占比");
                        });
                        let area = egui::ScrollArea::vertical()
                            .id_salt("rule_hits_table_probe")
                            .auto_shrink([false, false])
                            .max_height(ui.available_height() - reserve)
                            .show(ui, |ui| {
                                for _ in 0..40 {
                                    let _ = ui.horizontal(|ui| {
                                        let _ = ui.label("GEOIP,CN,DIRECT");
                                        let _ = ui.label("→ 代理");
                                        let _ = ui.label("12 条 · 9%");
                                    });
                                }
                            });
                        viewport = area.inner_rect.height();
                    });
                    card_bottom = shown.response.rect.max.y;
                });
        });
        (viewport, card_bottom)
    }

    /// 命中记录列表要把剩余高度全吃掉：说明并进表头 + 取消 40pt 预留之后，
    /// 1330×800 的视口从 469（约 19 行）涨到 ≥520（约 22 行），
    /// 140% 字体（950×571 逻辑）从 240（10 行）涨到 ≥290（12 行）。
    /// 同一份结构按老写法（独立说明行 + 预留 40）再量一次，那两行确实是白送的；
    /// 顺手钉住"卡片底不许越过中央面板底"，加高不能靠溢出实现。
    #[test]
    fn the_hits_list_uses_every_remaining_pixel() {
        for (size, min_new) in [
            (Vec2::new(1330.0, 800.0), 520.0),
            (Vec2::new(950.0, 571.0), 290.0),
        ] {
            let (new_vp, new_bottom) = probe(size, true, 0.0);
            let (old_vp, _old_bottom) = probe(size, false, 40.0);
            assert!(
                new_vp >= min_new,
                "{size:?} 时列表视口只有 {new_vp:.1}，没到 {min_new:.0}"
            );
            assert!(
                new_vp > old_vp + 50.0,
                "新写法视口 {new_vp:.1} 只比老写法 {old_vp:.1} 高一点点，说明没省下来"
            );
            // 中央面板底边 = 屏高 - 状态栏 32 - 控制栏 48，再收掉 14 的下内边距。
            let limit = size.y - 32.0 - 48.0 - 14.0;
            assert!(
                new_bottom <= limit + 1.0,
                "{size:?} 时卡片底画到 {new_bottom:.1}，越过中央面板底 {limit:.1}"
            );
        }
    }
}

#[cfg(test)]
mod rules_row_probe_tests {
    use super::{RULES_ACTIONS_WIDTH, RULES_TABS_MIN_WIDTH};
    use eframe::egui::{self, Frame, Margin, RawInput, Rect, Vec2};

    /// 复刻分流规则那行表头**含现写的分支判断**：宽到放得下就一行钉右，
    /// 放不下就让动作组另起一行。返回（面板矩形, 分区标签占的矩形, 动作组占的矩形）。
    fn probe(window_width: f32) -> (Rect, Rect, Rect) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 800.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut tabs = Rect::NOTHING;
        let mut group = Rect::NOTHING;
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
                    panel = ui.max_rect();
                    let mut draw_tabs = |ui: &mut egui::Ui| {
                        let a = ui.button("规则配置").rect;
                        let b = ui.button("命中记录").rect;
                        let added = a.union(b);
                        tabs = if !tabs.is_finite() {
                            added
                        } else {
                            tabs.union(added)
                        };
                    };
                    let mut draw_actions = |ui: &mut egui::Ui| {
                        // 只并按钮自己的矩形：min_rect 会把同一行里先画的东西也算进来。
                        let a = ui.button("导出 JSON").rect;
                        let c = ui.button("导入规则").rect;
                        let added = a.union(c);
                        group = if !group.is_finite() {
                            added
                        } else {
                            group.union(added)
                        };
                    };
                    // 与产品同一个判据（ui.max_rect() 就是中央面板的内容宽）。
                    if ui.max_rect().width() >= RULES_ACTIONS_WIDTH + RULES_TABS_MIN_WIDTH + 8.0 {
                        crate::gui::widgets::split_row(
                            ui,
                            "rules:tabs",
                            RULES_ACTIONS_WIDTH,
                            |ui| draw_tabs(ui),
                            |ui| draw_actions(ui),
                        );
                    } else {
                        let _ = ui.horizontal(|ui| draw_tabs(ui));
                        let _ = ui.horizontal_wrapped(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| draw_actions(ui),
                            );
                        });
                    }
                });
        });
        (panel, tabs, group)
    }

    /// 老写法的毛病：右对齐那组和左边的分区标签在窄面板上**互相压住**（x 方向重叠），
    /// 按钮看起来就是被挤走。反向验证：同一个宽度下老写法确实重叠、新写法不重叠。
    #[test]
    fn the_old_single_row_really_collided() {
        let (panel, tabs, group) = probe_single_row(520.0);
        assert!(
            group.min.x < tabs.max.x,
            "老写法在 520 宽没重叠（组从 {:.1} 开始，标签到 {:.1}）—— 反向验证失去意义",
            group.min.x,
            tabs.max.x
        );
        assert!(panel.width() > 0.0);
    }

    fn probe_single_row(window_width: f32) -> (Rect, Rect, Rect) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_width, 800.0),
        ));
        let mut panel = Rect::NOTHING;
        let mut tabs = Rect::NOTHING;
        let mut group = Rect::NOTHING;
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
                    panel = ui.max_rect();
                    let _ = ui.horizontal(|ui| {
                        let a = ui.button("规则配置").rect;
                        let b = ui.button("命中记录").rect;
                        tabs = a.union(b);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let before = ui.min_rect();
                            let _ = ui.button("导出 JSON");
                            let _ = ui.button("导入规则");
                            group = before.union(ui.min_rect());
                        });
                    });
                });
        });
        (panel, tabs, group)
    }

    /// 动作组不许和分区标签互相压住，也不许越过面板右缘。
    /// 下限取 460 = 960 物理最小宽 ÷（150% 系统缩放 × 140% 字体）；再窄（200% 缩放叠 140%
    /// 字体 = 380 逻辑点）egui 自己的右锚点会溢出 4.1pt，那已经不是这套布局能救的了。
    #[test]
    fn the_actions_never_collide_or_leave_the_panel() {
        for width in [1330.0, 950.0, 760.0, 686.0, 620.0, 520.0, 460.0] {
            let (panel, tabs, group) = probe(width);
            assert!(
                group.max.x <= panel.right() + 0.5,
                "{width:.0} 宽时动作组画到 {:.1}，越过面板右缘 {:.1}",
                group.max.x,
                panel.right()
            );
            assert!(
                tabs.min.x >= panel.left() - 0.5 && tabs.max.x <= panel.right() + 0.5,
                "{width:.0} 宽时分区标签画到 [{:.1}, {:.1}]，超出面板 [{:.1}, {:.1}]",
                tabs.min.x,
                tabs.max.x,
                panel.left(),
                panel.right()
            );
            assert!(
                !group.intersects(tabs),
                "{width:.0} 宽时动作组 [{:.1},{:.1}] 压住分区标签 [{:.1},{:.1}]",
                group.min.x,
                group.max.x,
                tabs.min.x,
                tabs.max.x
            );
        }
    }
}
#[cfg(test)]
mod status_button_style_tests {
    use super::status_btn;
    use crate::gui::widgets::{danger_btn, primary_btn};
    use eframe::egui::{self, CentralPanel, Color32, CornerRadius, RawInput, Rect, Vec2};

    /// 照状态栏那一排的写法真画一遍，读这一帧画出来的按钮矩形。
    fn painted() -> Vec<(CornerRadius, f32, Color32)> {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(1330.0, 200.0),
        ));
        let mut out = Vec::new();
        let _ = ctx.run(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(11.0, 3.0);
                let layer = ui.layer_id();
                ui.horizontal(|ui| {
                    let _ = ui.add(primary_btn("启动代理"));
                    let _ = ui.add(danger_btn("停止代理"));
                    let _ = ui.add(status_btn("IP 检测"));
                    let _ = ui.add(status_btn("系统代理"));
                    let _ = ui.add(status_btn("测速全部"));
                });
                out = ctx.graphics_mut(|g| {
                    g.get(layer)
                        .map(|list| {
                            list.all_entries()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Rect(rect)
                                        if (16.0..=40.0).contains(&rect.rect.height()) =>
                                    {
                                        Some((rect.corner_radius, rect.rect.height(), rect.fill))
                                    }
                                    _ => None,
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                });
            });
        });
        out
    }

    /// 一排五个按钮：圆角和高度必须完全一致（这是他说"样式要一样"时看到的东西），
    /// 普通那三个的填充也必须一致；主/危险按钮允许只有颜色不同。
    #[test]
    fn the_status_row_buttons_share_one_shape() {
        let rects = painted();
        assert!(
            rects.len() >= 5,
            "五个按钮没都画出来，只读到 {}",
            rects.len()
        );
        let radius = rects[0].0;
        let height = rects[0].1;
        for (index, (got_radius, got_height, _)) in rects.iter().enumerate() {
            assert_eq!(
                *got_radius, radius,
                "第 {index} 个按钮圆角 {got_radius:?} 和同排 {radius:?} 不一致"
            );
            assert!(
                (got_height - height).abs() < 0.6,
                "第 {index} 个按钮高 {got_height:.1}，同排是 {height:.1}"
            );
        }
        let neutral: Vec<Color32> = rects.iter().skip(2).map(|(_, _, fill)| *fill).collect();
        assert!(
            neutral.windows(2).all(|pair| pair[0] == pair[1]),
            "普通按钮的填充色不一致：{neutral:?}"
        );
        assert_ne!(
            rects[0].2, rects[2].2,
            "主按钮和普通按钮同色了 —— 那这排就没人看得出哪个是主操作"
        );
    }
}

#[cfg(test)]
mod policy_group_filter_tests {
    use super::node_matches_group;
    use crate::gui::types::KernelGroup;
    use crate::models::NodeSummary;

    fn sample_node(name: &str, country: Option<&str>, server: &str) -> NodeSummary {
        NodeSummary {
            id: name.to_string(),
            source_id: Some("src1".to_string()),
            protocol: "vless".to_string(),
            name: name.to_string(),
            server: server.to_string(),
            port: 443,
            country: country.map(|c| c.to_string()),
            latency_ms: Some(120),
            latency_ok: Some(true),
            download_kbps: Some(5000.0),
            latency_kernel: Some("mihomo".to_string()),
            latency_test_kind: Some("google_proxy".to_string()),
            latency_tested_at: None,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_all_group_matches_everything() {
        let us = sample_node("US-Node-01", Some("US"), "1.1.1.1");
        let hk = sample_node("HK-Node-01", Some("HK"), "2.2.2.2");
        assert!(node_matches_group(&us, "全部", &[]));
        assert!(node_matches_group(&hk, "全部", &[]));
        assert!(node_matches_group(&us, "PROXY", &[]));
        assert!(node_matches_group(&hk, "PROXY", &[]));
    }

    #[test]
    fn test_us_group_matches_only_us() {
        let us = sample_node("洛杉矶精品专线", Some("US"), "1.1.1.1");
        let hk = sample_node("香港高速直连", Some("HK"), "2.2.2.2");
        let jp = sample_node("东京软银", Some("JP"), "3.3.3.3");
        assert!(node_matches_group(&us, "美国节点", &[]));
        assert!(!node_matches_group(&hk, "美国节点", &[]));
        assert!(!node_matches_group(&jp, "美国节点", &[]));
    }

    #[test]
    fn test_ai_group_matches_ai_regions_and_keywords() {
        let us = sample_node("美国节点", Some("US"), "1.1.1.1");
        let jp = sample_node("日本东京", Some("JP"), "2.2.2.2");
        let sg = sample_node("新加坡", Some("SG"), "3.3.3.3");
        let custom_ai = sample_node("OpenAI 专线解锁", None, "4.4.4.4");
        let cn = sample_node("国内直连", Some("CN"), "5.5.5.5");

        assert!(node_matches_group(&us, "AI分组", &[]));
        assert!(node_matches_group(&jp, "AI分组", &[]));
        assert!(node_matches_group(&sg, "AI分组", &[]));
        assert!(node_matches_group(&custom_ai, "AI分组", &[]));
        assert!(!node_matches_group(&cn, "AI分组", &[]));
    }

    #[test]
    fn test_kernel_groups_members_matching_and_cycle_guard() {
        let n1 = sample_node("专线节点A", None, "1.1.1.1");
        let n2 = sample_node("专线节点B", None, "2.2.2.2");

        let kg = vec![
            KernelGroup {
                name: "自定义分组".to_string(),
                kind: "select".to_string(),
                now: Some("专线节点A".to_string()),
                members: vec!["专线节点A".to_string()],
            },
            // 环形依赖测试
            KernelGroup {
                name: "GroupLoopA".to_string(),
                kind: "select".to_string(),
                now: None,
                members: vec!["GroupLoopB".to_string()],
            },
            KernelGroup {
                name: "GroupLoopB".to_string(),
                kind: "select".to_string(),
                now: None,
                members: vec!["GroupLoopA".to_string()],
            },
        ];

        assert!(node_matches_group(&n1, "自定义分组", &kg));
        assert!(!node_matches_group(&n2, "自定义分组", &kg));

        // 环形嵌套不应陷入无限循环或栈溢出
        assert!(!node_matches_group(&n1, "GroupLoopA", &kg));
    }

    fn kernel_group(name: &str, kind: &str, members: &[&str]) -> KernelGroup {
        KernelGroup {
            name: name.to_string(),
            kind: kind.to_string(),
            now: None,
            members: members.iter().map(|member| member.to_string()).collect(),
        }
    }

    #[test]
    fn test_the_pill_bar_defaults_to_all_only() {
        use super::visible_group_names;

        // 什么都没配：整条胶囊只有「全部」。
        assert_eq!(visible_group_names(false, None, &[]), vec!["全部".to_string()]);

        // 反向判据：内置的「普通代理」按节点匹配能命中每一个节点，
        // 但配置里没有它就不该出现 —— 否则这条改动等于没做。
        let any_node = sample_node("任意节点", Some("HK"), "1.2.3.4");
        assert!(node_matches_group(&any_node, "普通代理", &[]));
        assert!(!visible_group_names(false, None, &[])
            .iter()
            .any(|name| name == "普通代理"));

        // 打开内置智能策略组 ⇒ 四组都出现。
        let smart = visible_group_names(true, None, &[]);
        assert_eq!(smart.len(), 5);
        for name in ["AI分组", "美国节点", "多线并发下载", "普通代理"] {
            assert!(smart.iter().any(|shown| shown == name), "{name} 该在");
        }

        // 内核上报的组：成员非空才算数，保留名不算组。
        let kernel = vec![
            kernel_group("我的分组", "url-test", &["a"]),
            kernel_group("空分组", "select", &[]),
            kernel_group("GLOBAL", "select", &["a"]),
        ];
        let shown = visible_group_names(false, None, &kernel);
        assert!(shown.iter().any(|name| name == "我的分组"));
        assert!(!shown.iter().any(|name| name == "空分组"));
        assert!(!shown.iter().any(|name| name == "GLOBAL"));

        // 内核真上报了内置组的名字（比如覆写里自己定义的），关着 smart_routing 也要显示。
        let reported = vec![kernel_group("普通代理", "select", &["a"])];
        assert!(visible_group_names(false, None, &reported)
            .iter()
            .any(|name| name == "普通代理"));
    }

    #[test]
    fn test_override_yaml_groups_count_as_configured() {
        use super::{override_group_names, visible_group_names};

        let document = "proxy-groups:\n  - name: 我的分组\n    type: select\n    proxies: [DIRECT]\n";
        assert_eq!(
            override_group_names(Some(document)),
            vec!["我的分组".to_string()]
        );
        // 覆写也允许直接给一个组数组（与 proxy 侧应用覆写时的分叉一致）。
        let bare = "- name: 裸数组分组\n  type: url-test\n";
        assert_eq!(override_group_names(Some(bare)), vec!["裸数组分组".to_string()]);
        assert!(override_group_names(None).is_empty());
        assert!(override_group_names(Some("   ")).is_empty());
        // 坏 YAML 不许带崩界面：按"没有覆写组"处理。
        assert!(override_group_names(Some("proxy-groups: [:::")).is_empty());

        assert!(visible_group_names(false, Some(document), &[])
            .iter()
            .any(|name| name == "我的分组"));
    }
}

#[cfg(test)]
mod clean_kernel_version_tests {
    use super::clean_kernel_version;

    #[test]
    fn test_extracts_semver_from_complex_kernel_banners() {
        assert_eq!(
            clean_kernel_version(
                "Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 16:55:17 UTC 2026"
            ),
            "v1.19.32"
        );
        assert_eq!(
            clean_kernel_version("sing-box version 1.14.3 (go1.22.0 windows/amd64)"),
            "1.14.3"
        );
        assert_eq!(
            clean_kernel_version("Xray 26.3.27 (Xray, Penetrates Everything.) OpenSource"),
            "26.3.27"
        );
        assert_eq!(clean_kernel_version("v1.19.32"), "v1.19.32");
        assert_eq!(clean_kernel_version("1.14.3"), "1.14.3");
        assert_eq!(clean_kernel_version("未检测到"), "未检测到");
        assert_eq!(clean_kernel_version("-"), "-");
        assert_eq!(clean_kernel_version(""), "");
    }
}

#[cfg(test)]
mod collapsed_section_tests {
    use eframe::egui::{self, RawInput, Rect, Vec2};

    /// 「自定义规则」默认收起整件事压在一条 egui 契约上：不带 `default_open` 的
    /// CollapsingHeader 首帧是收起的（里面的东西不布局），展开了才布局。
    /// 谁哪天顺手加一个 `.default_open(true)`，这条就红。
    #[test]
    fn collapsing_header_defaults_to_closed() {
        let body_laid_out = |default_open: Option<bool>| {
            let ctx = egui::Context::default();
            let mut seen = false;
            for _ in 0..2 {
                let mut input = RawInput::default();
                input.screen_rect = Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(900.0, 600.0),
                ));
                seen = false;
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut header = egui::CollapsingHeader::new("自定义规则");
                        if let Some(open) = default_open {
                            header = header.default_open(open);
                        }
                        header.id_salt("custom_rules_section").show(ui, |ui| {
                            ui.label("规则行");
                            seen = true;
                        });
                    });
                });
            }
            seen
        };
        assert!(
            !body_laid_out(None),
            "不带 default_open 时首帧应该是收起的（这条就是这次改动要的默认值）"
        );
        assert!(
            body_laid_out(Some(true)),
            "对照组失效：default_open(true) 也没展开，那上面那条断言是在给自己鼓掌"
        );
    }
}

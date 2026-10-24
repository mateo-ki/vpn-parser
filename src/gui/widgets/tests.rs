use super::*;
use crate::gui::theme;

mod sparkline_tests {
    use super::*;
    use eframe::egui::{self, CentralPanel, Color32, RawInput, Rect, Vec2};
    use std::collections::VecDeque;

    fn history(points: usize) -> VecDeque<(f32, f32)> {
        (0..points)
            .map(|index| (10.0 + (index % 7) as f32, 1.0 + (index % 3) as f32))
            .collect()
    }

    /// 照表头第二行的写法跑一遍：读数排在前，迷你图吃掉剩下的宽度。
    /// 返回 (图拿到的宽度, 整行占掉的高度, 这一行之后还剩多少高度)。
    fn probe(window_w: f32, samples: usize, active: bool) -> (f32, f32, f32) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window_w, 800.0),
        ));
        let mut room = -1.0_f32;
        let mut row = (-1.0_f32, -1.0_f32);
        let _ = ctx.run(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let before = ui.available_height();
                let response = ui.horizontal(|ui| {
                    ui.label("延迟: 386 ms");
                    ui.separator();
                    ui.label("端口: Mixed 9980 · Controller 9090");
                    ui.separator();
                    ui.label("↓ 2.5 KB/s · ↑ 387 B/s");
                    room = ui.available_width();
                    render_traffic_sparkline(ui, &history(samples), active);
                });
                row = (
                    response.response.rect.height(),
                    before - ui.available_height(),
                );
            });
        });
        (room, row.0, row.1)
    }

    #[test]
    fn sparkline_takes_the_rest_of_the_header_row() {
        let (room, height, used) = probe(1280.0, 60, true);
        assert!(room > 300.0, "1280 宽的窗口里图还应该剩 {room:.0}px 可画");
        assert!(
            (height - 32.0).abs() < 6.0,
            "这一行应该只占 ~32 高，实测 {height:.1}"
        );
        assert!(
            used < 40.0,
            "整行吃掉的页高 {used:.1}，多了就是又被 with_layout 吃页了"
        );
    }

    #[test]
    fn sparkline_still_has_room_at_the_minimum_window_width() {
        let (room, height, used) = probe(960.0, 60, true);
        assert!(room > 120.0, "960 最小宽度下图仍应有 {room:.0}px");
        assert!(used < 40.0, "窄窗口也不能吃页，实测 {used:.1}");
        assert!(height < 40.0, "行高 {height:.1}");
    }

    #[test]
    fn sparkline_survives_no_samples_and_a_stopped_proxy() {
        // 采样点不足两个时那条 dx 会除零，未运行时也不该 panic。
        for (samples, active) in [(0usize, true), (1, true), (60, false)] {
            let (room, _, _) = probe(1280.0, samples, active);
            assert!(room > 0.0, "samples={samples} active={active}");
        }
    }

    /// 健康条的尺寸：宽窗口要真的"看得见"（>=220 宽、18 高），窄窗口不许溢出可用宽度
    /// （溢出的话 egui 会把整条折到第二行，卡片被顶高 —— 那是他否过的表现）。
    #[test]
    fn health_strip_scales_with_the_room_it_has() {
        fn probe(window_width: f32) -> (f32, f32, f32) {
            let ctx = egui::Context::default();
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(window_width, 200.0),
            ));
            let mut drawn = (0.0, 0.0, 0.0);
            let _ = ctx.run(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("实测 12:30 ✓ 262ms");
                        let before = ui.available_width();
                        render_health_strip(ui, &[1i8; 24]);
                        drawn = (before, ui.available_width(), ui.cursor().height());
                    });
                });
            });
            drawn
        }

        let (before, after, _) = probe(1280.0);
        let used = before - after;
        // 24 格 × (10 + 2) + 8 的格间距 = 296；放大前是 24×(5+1)+8 = 152。
        assert!(
            (280.0..=310.0).contains(&used),
            "1280 宽时健康条应该占 {used:.0}px（放大过，不该还是原来那 152）"
        );
        // 窄一点时格子缩到 6px，但绝不能吃满再溢出（溢出=折行=顶高卡片）。
        let (narrow_before, narrow_after, _) = probe(560.0);
        assert!(
            narrow_before - narrow_after <= narrow_before,
            "560 宽时健康条溢出：占 {:.0} / 可用 {:.0}",
            narrow_before - narrow_after,
            narrow_before
        );
    }

    /// 健康条"那一小时有不通"的格子必须真的画成红（2026-10-15 他点名要红）。
    /// 读的是这一帧真画出来的矩形填充色，不是常量 —— 旧写法用的是 `text_primary()`
    /// （石墨深色），照着代码念一遍是看不出差别的。
    #[test]
    fn the_failed_hours_paint_red() {
        fn fills(buckets: &[i8; 24]) -> Vec<Color32> {
            let ctx = egui::Context::default();
            let mut input = RawInput::default();
            input.screen_rect = Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1280.0, 200.0),
            ));
            let mut out = Vec::new();
            let _ = ctx.run(input, |ctx| {
                CentralPanel::default().show(ctx, |ui| {
                    let layer = ui.layer_id();
                    render_health_strip(ui, buckets);
                    out = ctx.graphics_mut(|g| {
                        g.get(layer)
                            .map(|list| {
                                list.all_entries()
                                    .filter_map(|shape| match &shape.shape {
                                        egui::Shape::Rect(rect)
                                            if (rect.rect.height() - 8.0).abs() < 0.1 =>
                                        {
                                            Some(rect.fill)
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

        let mut buckets = [0i8; 24];
        for index in [0usize, 3, 9] {
            buckets[index] = -1;
        }
        for index in [1usize, 2, 4] {
            buckets[index] = 1;
        }
        let painted = fills(&buckets);
        assert_eq!(painted.len(), 24, "24 格应该画满，实际 {}", painted.len());
        assert_eq!(
            painted
                .iter()
                .filter(|c| **c == health_failed_color())
                .count(),
            3,
            "红格数不对（三个有不通的小时没都画成红）"
        );
        assert_eq!(
            painted
                .iter()
                .filter(|c| **c == super::theme::accent())
                .count(),
            3,
            "正常格数不对"
        );
        assert_eq!(
            painted
                .iter()
                .filter(|c| **c == super::theme::border())
                .count(),
            18,
            "没实测的那 18 格颜色不对"
        );
        // 反向：这要是还退回旧的深色写法，这条就红。
        assert_ne!(
            health_failed_color(),
            super::theme::text_primary(),
            "红和正文深色撞了，等于没改"
        );
    }

    fn health_row(minutes_ago: i64, ok: bool) -> crate::db::telemetry::HealthRow {
        crate::db::telemetry::HealthRow {
            at: (chrono::Utc::now() - chrono::Duration::minutes(minutes_ago)).to_rfc3339(),
            ok: ok as i64,
            latency_ms: Some(120),
            error: None,
            node_id: None,
            node_name: None,
            kind: "periodic".into(),
        }
    }

    /// 健康条的桶改成"数据到达时算一次"之后，这条把语义钉住：小时槽、失败优先、
    /// 24 小时之外和未来的行都不许进来。
    #[test]
    fn health_buckets_are_hourly_failure_wins_and_window_bound() {
        let empty = health_hour_buckets(&[]);
        assert_eq!(empty, [0i8; 24], "没数据就是 24 格全空");

        let one = health_hour_buckets(&[health_row(10, true)]);
        assert_eq!(one[0], 1, "十分钟前的实测落在当前这一格");
        assert_eq!(one[1], 0);

        let fail = health_hour_buckets(&[health_row(90, false)]);
        assert_eq!(fail[1], -1, "一个半小时前是上一格");

        let mixed = health_hour_buckets(&[health_row(5, true), health_row(20, false)]);
        assert_eq!(mixed[0], -1, "同一小时里失败优先，绿不能把红盖掉");

        let stale = health_hour_buckets(&[health_row(24 * 60 + 5, false)]);
        assert_eq!(stale, [0i8; 24], "超出 24 小时的行不该挤进最老那一格");

        // `num_hours()` 是**向零截断**的：一小时以内的时钟漂移会算进当前这格（改之前
        // 就是这个语义，这次纯性能重构不动它），超过一小时才落到负数被丢掉。
        let near_future = health_hour_buckets(&[health_row(-30, false)]);
        assert_eq!(
            near_future[0], -1,
            "半小时内飘到未来的行落在当前格，不是越界"
        );
        let future = health_hour_buckets(&[health_row(-90, false)]);
        assert_eq!(
            future, [0i8; 24],
            "一小时以上的未来行必须丢，否则索引是负数"
        );
    }
}
#[cfg(test)]
mod button_radius_tests {
    use super::*;
    use eframe::egui::{self, CentralPanel, CornerRadius, RawInput, Rect, Vec2};

    /// 读这一帧里画出来的所有"按钮那么高"的矩形的圆角。
    fn radii(window: f32) -> Vec<CornerRadius> {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(
            egui::Pos2::ZERO,
            Vec2::new(window, 200.0),
        ));
        let mut out = Vec::new();
        let _ = ctx.run(input, |ctx| {
            CentralPanel::default().show(ctx, |ui| {
                let layer = ui.layer_id();
                ui.horizontal(|ui| {
                    let _ = ui.button("IP 检测");
                    let _ = ui.add(primary_btn("启动代理"));
                    let _ = ui.add(danger_btn("停止代理"));
                });
                out = ctx.graphics_mut(|g| {
                    g.get(layer)
                        .map(|list| {
                            list.all_entries()
                                .filter_map(|shape| match &shape.shape {
                                    egui::Shape::Rect(rect)
                                        if (4.0..=40.0).contains(&rect.rect.height()) =>
                                    {
                                        Some(rect.corner_radius)
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

    /// 状态栏那三个按钮必须一个圆角：普通按钮 2px，主/危险按钮以前各自钉了 16px，
    /// 于是同一条里"停止代理"是圆的、旁边"IP 检测"是方的（2026-10-15 他点的这条）。
    #[test]
    fn primary_and_danger_share_the_plain_button_radius() {
        let painted = radii(1280.0);
        assert!(
            painted.len() >= 3,
            "三个按钮的背景矩形没都画出来，只读到 {}",
            painted.len()
        );
        let plain = painted[0];
        for (index, radius) in painted.iter().enumerate() {
            assert_eq!(
                *radius, plain,
                "第 {index} 个按钮圆角 {radius:?} 和普通按钮 {plain:?} 不一致"
            );
        }
        assert_ne!(
            plain,
            CornerRadius::same(16),
            "还是那个 16 的大圆角 —— 说明 helper 里那两行没删掉"
        );
    }

    #[test]
    fn format_connection_rule_normalizes_shoes_and_mihomo_rules() {
        // 1. Shoes GeoSite 规则
        let shoes_geosite = r#"local-mixed (root): {"path":"D:/software/VPN Parser/data/kernels/mihomo/geosite.dat","format":"geosite_dat","code":"cn"}"#;
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some(shoes_geosite)),
            "GEOSITE(cn)"
        );

        // 2. Shoes GeoIp 规则
        let shoes_geoip = r#"local-mixed (root): {"path":"D:/software/VPN Parser/data/kernels/mihomo/geoip.dat","format":"geoip_dat","code":"cn"}"#;
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some(shoes_geoip)),
            "GEOIP(cn)"
        );

        // 3. Shoes Domain Suffix 规则
        let shoes_ds = r#"local-mixed (root): {"domain_suffix":["youtube.com"]}"#;
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some(shoes_ds)),
            "DOMAIN-SUFFIX(youtube.com)"
        );

        // 4. Shoes Domain 规则
        let shoes_domain = r#"local-mixed (root): {"domain":["localhost"]}"#;
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some(shoes_domain)),
            "DOMAIN(localhost)"
        );

        // 5. Shoes Domain Keyword 规则
        let shoes_keyword = r#"local-mixed (root): {"domain_keyword":["stun","turn","webrtc"]}"#;
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some(shoes_keyword)),
            "DOMAIN-KEYWORD(stun, +2)"
        );

        // 6. Shoes 兜底匹配与 CIDR
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some("local-mixed (root): 0.0.0.0/0")),
            "MATCH"
        );
        assert_eq!(
            format_connection_rule(Some("Shoes"), Some("local-mixed (root): 127.0.0.0/8")),
            "IP-CIDR(127.0.0.0/8)"
        );

        // 7. 标准 Mihomo 规则
        assert_eq!(
            format_connection_rule(Some("GEOSITE"), Some("cn")),
            "GEOSITE(cn)"
        );
        assert_eq!(
            format_connection_rule(Some("MATCH"), Some("")),
            "MATCH"
        );
        assert_eq!(
            format_connection_rule(None, None),
            "-"
        );
    }

    #[test]
    fn format_connection_target_resolves_chains_and_payload() {
        // 1. Shoes 代理链：过滤 local-mixed
        let chains = vec!["local-mixed".to_string(), "PROXY".to_string(), "proxy".to_string()];
        assert_eq!(format_connection_target(&chains, None), "PROXY > proxy");

        // 2. Shoes 直连链
        let direct_chains = vec!["local-mixed".to_string(), "direct".to_string()];
        assert_eq!(format_connection_target(&direct_chains, None), "DIRECT");

        // 3. 无 chains 回退
        assert_eq!(format_connection_target(&[], Some("DIRECT")), "DIRECT");
        assert_eq!(format_connection_target(&[], Some("PROXY")), "PROXY");
        assert_eq!(format_connection_target(&[], None), "-");
    }
}

use super::{theme, BgMsg, VpnApp};
use eframe::egui::{self, Color32, CornerRadius, Frame, Sense, Stroke, Ui, Vec2};

impl VpnApp {
    /// 自绘标题栏。`with_decorations(false)` 之后系统那条没了，所以拖动、双击最大化、
    /// 最小化/最大化/关闭三个按钮都得在这儿自己实现。
    ///
    /// 三个按钮用 painter 画、`interact` 命中，不用 `Button`，也不放 `Label`：
    /// `Frame` 的外接矩形是按内容 `min_rect` 反推的，而面板只画到 `exact_height` 就裁掉。
    /// `Button` 想要"文字 + button_padding"那么高，`ui.put` 会把格子顶到 34 之外 ——
    /// 顶出去的部分不画但照样占位，表现成标题栏下面一条没人画的黑色带子。
    pub(crate) fn ui_title_bar(&mut self, ctx: &egui::Context) {
        const BAR_H: f32 = TITLE_BAR_H;
        const BTN_W: f32 = TITLE_BAR_BTN_W;

        #[derive(Clone, Copy)]
        enum Glyph {
            Minimize,
            Maximize,
            Restore,
            Close,
        }

        fn title_button(
            ui: &Ui,
            rect: egui::Rect,
            key: &'static str,
            glyph: Glyph,
            tip: &str,
        ) -> bool {
            let hovered = ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|point| rect.contains(point));
            let painter = ui.painter().clone();
            let ink = theme::text_secondary();
            let pill = rect.shrink(4.0);
            if hovered {
                painter.rect(
                    pill,
                    CornerRadius::same(7),
                    theme::panel_hover(),
                    Stroke::NONE,
                    egui::StrokeKind::Inside,
                );
            }
            let stroke = Stroke::new(1.2_f32, ink);
            let c = pill.center();
            match glyph {
                Glyph::Minimize => {
                    painter.line_segment(
                        [c + Vec2::new(-5.5, 0.0), c + Vec2::new(5.5, 0.0)],
                        stroke,
                    );
                }
                Glyph::Maximize => {
                    painter.rect(
                        egui::Rect::from_center_size(c, Vec2::splat(10.5)),
                        CornerRadius::same(2),
                        Color32::TRANSPARENT,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                Glyph::Restore => {
                    painter.rect(
                        egui::Rect::from_min_max(
                            c + Vec2::new(-6.0, -2.5),
                            c + Vec2::new(3.5, 6.5),
                        ),
                        CornerRadius::same(2),
                        Color32::TRANSPARENT,
                        stroke,
                        egui::StrokeKind::Inside,
                    );
                    painter.line_segment(
                        [c + Vec2::new(-3.0, -6.5), c + Vec2::new(6.5, -6.5)],
                        stroke,
                    );
                    painter.line_segment(
                        [c + Vec2::new(6.5, -6.5), c + Vec2::new(6.5, -2.0)],
                        stroke,
                    );
                }
                Glyph::Close => {
                    painter.line_segment(
                        [c + Vec2::new(-5.0, -5.0), c + Vec2::new(5.0, 5.0)],
                        stroke,
                    );
                    painter.line_segment(
                        [c + Vec2::new(-5.0, 5.0), c + Vec2::new(5.0, -5.0)],
                        stroke,
                    );
                }
            }
            ui.interact(rect, ui.id().with(("titlebar-button", key)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tip)
                .clicked()
        }

        let maximized = ctx.input(|i| i.viewport().maximized).unwrap_or(false);
        let mut action_minimize = false;
        let mut action_toggle_max = false;
        let mut action_close = false;

        egui::TopBottomPanel::top("custom_title_bar")
            .exact_height(BAR_H)
            .frame(Frame::NONE.fill(theme::sidebar()))
            .show(ctx, |ui| {
                // 一次性把整格占住，之后全用 painter/interact：这样 Frame 反推出来的
                // 高度恒等于 BAR_H，画到的和分给下面面板的永远是同一条边。
                let bar = ui.max_rect();
                ui.allocate_rect(bar, Sense::hover());
                let buttons_left = bar.max.x - 3.0 * BTN_W;

                ui.painter().text(
                    egui::pos2(bar.min.x + 14.0, bar.center().y),
                    egui::Align2::LEFT_CENTER,
                    "VPN Parser",
                    egui::FontId::proportional(12.5),
                    theme::text_secondary(),
                );

                // 除了右边三个按钮，整条都能拖动窗口；双击切换最大化。
                let drag_rect =
                    egui::Rect::from_min_max(bar.min, egui::pos2(buttons_left, bar.max.y));
                let drag = ui.interact(drag_rect, ui.id().with("titlebar_drag"), Sense::drag());
                if drag.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    action_toggle_max = true;
                }

                let cell = |index: usize| {
                    egui::Rect::from_min_max(
                        egui::pos2(bar.max.x - (index as f32 + 1.0) * BTN_W, bar.min.y),
                        egui::pos2(bar.max.x - index as f32 * BTN_W, bar.max.y),
                    )
                };
                if title_button(
                    ui,
                    cell(0),
                    "close",
                    Glyph::Close,
                    "关闭（按设置在托盘里继续常驻）",
                ) {
                    action_close = true;
                }
                if title_button(
                    ui,
                    cell(1),
                    "maximize",
                    if maximized {
                        Glyph::Restore
                    } else {
                        Glyph::Maximize
                    },
                    if maximized { "向下还原" } else { "最大化" },
                ) {
                    action_toggle_max = true;
                }
                if title_button(ui, cell(2), "minimize", Glyph::Minimize, "最小化（按设置收进系统托盘）") {
                    action_minimize = true;
                }
            });

        if action_minimize {
            if self.settings.close_to_tray {
                crate::startup_log("Title-bar minimize -> 最小化收进托盘（从任务栏隐藏）");
                self.park_to_tray();
            } else {
                self.was_minimized = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        if action_toggle_max {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        if action_close {
            self.request_window_close(ctx);
        }
    }

    /// 标题栏的 ✕ 必须和系统关闭按钮走同一套分支，否则"关闭收进托盘"这个设置形同虚设。
    /// 每 30 秒核对一次系统代理归属。只在"代理在跑、而且是我们开的那份系统代理"
    /// （`local_proxy_ready`）时查 —— TUN 模式根本不碰注册表，查了只会误报。
    /// 查询要起 `reg.exe`（三次），所以丢给 `spawn_blocking`，绝不在 UI 线程上等。
    pub(crate) fn maybe_check_proxy_drift(&mut self) {
        const EVERY: std::time::Duration = std::time::Duration::from_secs(30);
        if !self.proxy_status.local_proxy_ready {
            if self.proxy_drift.is_some() {
                self.proxy_drift = None;
            }
            self.drift_checked_at = None;
            return;
        }
        if self.drift_busy {
            return;
        }
        if let Some(at) = self.drift_checked_at {
            if at.elapsed() < EVERY {
                return;
            }
        }
        self.drift_checked_at = Some(std::time::Instant::now());
        self.drift_busy = true;
        let expected = format!("127.0.0.1:{}", self.proxy_status.mixed_port);
        let control = crate::system_proxy::platform_system_proxy();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let drift = tokio::task::spawn_blocking(move || {
                control
                    .capture()
                    .map(|snapshot| crate::system_proxy::classify_drift(&snapshot, &expected))
                    .unwrap_or(crate::system_proxy::Drift::InSync)
            })
            .await
            .unwrap_or(crate::system_proxy::Drift::InSync);
            let _ = tx.send(BgMsg::ProxyDrift(drift));
        });
    }

    /// 收进托盘 = 把窗口**停放到屏幕外**（不再 `Visible(false)`，理由见 `platform::window_park`：
    /// 隐藏的窗口收不到 WM_PAINT，一帧都不跑，eframe 会把泵永久留在 Poll，实测烧满一个核）。
    /// 只能在 UI 线程调用。
    pub(crate) fn park_to_tray(&mut self) {
        if self.parked {
            return;
        }
        match crate::platform::park_main_window() {
            Some(spot) => {
                self.park_spot = Some(spot);
                self.parked = true;
                self.parked_at = Some(std::time::Instant::now());
            }
            None => {
                // 停不出去就不能装作"收进托盘了"：窗口还在用户眼前，状态必须说实话。
                crate::startup_log("[托盘] 停放失败（找不到主窗口），窗口保持原位");
                return;
            }
        }
        // 释放节点库缓存与工作集：这条是原来"隐藏到托盘"就有的行为，保留。
        let db = self.db.clone();
        self.rt.spawn(async move {
            db.trim_cached_pools().await;
        });
        crate::platform::trim_working_set();
    }

    /// 从托盘叫回来：把窗口放回停放前记下位置和大小，并夺回焦点。
    pub(crate) fn unpark_from_tray(&mut self, ctx: &egui::Context) {
        if let Some(spot) = self.park_spot.take() {
            crate::platform::unpark_main_window(spot);
        }
        self.parked = false;
        self.parked_at = None;
        self.was_minimized = false;
        self.restore_cooldown_frames = 12;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    pub(crate) fn request_window_close(&mut self, ctx: &egui::Context) {
        if self.settings.close_to_tray && !self.force_quit {
            crate::startup_log("Title-bar close -> 收进托盘（停放到屏幕外）");
            self.park_to_tray();
        } else {
            crate::startup_log("Title-bar close -> quitting application");
            self.force_quit = true;
            self.tray.shutdown();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// 无边框窗口的缩放手：把 `resize_handle_rects` 算出的那几块铺成一层覆盖区。
    /// 中间不拦截鼠标，照常给内容用。
    pub(crate) fn ui_resize_handles(&self, ctx: &egui::Context) {
        let screen = ctx.input(|i| i.screen_rect());
        let min = screen.min;
        let handles = resize_handle_rects(screen);
        egui::Area::new(egui::Id::new("vpn-parser-resize-handles"))
            .fixed_pos(min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (index, (rect, direction, cursor)) in handles.into_iter().enumerate() {
                    let local = egui::Rect::from_min_max(rect.min - min.to_vec2(), rect.max - min.to_vec2());
                    let response = ui.interact(
                        local,
                        ui.id().with(("resize-handle", index)),
                        Sense::drag(),
                    );
                    if response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                    }
                    response.on_hover_cursor(cursor);
                }
            });
    }

}


/// 自绘标题栏的高度，和右上角三个按钮各自的宽度。`ui_title_bar` 按它画，
/// `resize_handle_rects` 也要按同一个数把按钮那一块从缩手感区里挖出去 ——
/// 两边各写一份迟早会飘，所以收在这儿。
pub(crate) const TITLE_BAR_H: f32 = 34.0;
pub(crate) const TITLE_BAR_BTN_W: f32 = 46.0;

/// 无边框窗口外圈该铺哪几块缩手感区。**没有 NorthEast 角**，而且左右两条从标题栏
/// 下沿才开始：这层盖在 `Order::Foreground`，以前右上角那块 30×30 正好整个压在 ✕ 上，
/// 鼠标一放到关闭键就变成斜向拉伸箭头，点下去还可能被 `BeginResize` 吃掉。
/// 代价只是右上角不能斜向拉伸 —— 右边、下面、左上角照常。
pub(crate) fn resize_handle_rects(
    screen: egui::Rect,
) -> Vec<(egui::Rect, egui::ResizeDirection, egui::CursorIcon)> {
    use egui::{CursorIcon as Cursor, ResizeDirection as Dir};
    const EDGE: f32 = 6.0;
    let (min, max) = (screen.min, screen.max);
    let corner = EDGE * 5.0;
    let side_top = min.y + TITLE_BAR_H;
    // 顶边条在按钮区之前停下（多留 8px，斜着进按钮区边缘也不算被盖住）。
    let north_stop = max.x - TITLE_BAR_BTN_W * 3.0 - 8.0;
    vec![
        (
            egui::Rect::from_min_max(egui::pos2(min.x, side_top), egui::pos2(min.x + EDGE, max.y - corner)),
            Dir::West,
            Cursor::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(max.x - EDGE, side_top), egui::pos2(max.x, max.y - corner)),
            Dir::East,
            Cursor::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(min.x + corner, min.y), egui::pos2(north_stop, min.y + EDGE)),
            Dir::North,
            Cursor::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(min.x + corner, max.y - EDGE), egui::pos2(max.x - corner, max.y)),
            Dir::South,
            Cursor::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(min, egui::pos2(min.x + corner, min.y + corner)),
            Dir::NorthWest,
            Cursor::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(min.x, max.y - corner), egui::pos2(min.x + corner, max.y)),
            Dir::SouthWest,
            Cursor::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_max(egui::pos2(max.x - corner, max.y - corner), max),
            Dir::SouthEast,
            Cursor::ResizeNwSe,
        ),
    ]
}
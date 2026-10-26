use super::theme;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, Ui, Vec2};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Dashboard,
    Lightning,
    Subscription,
    Gear,
    Plug,
    Pin,
    Radar,
    Rules,
    Globe,
    Cpu,
    Terminal,
    ArrowUp,
    ArrowDown,
    Play,
    Stop,
    Sync,
    Trash,
    Clipboard,
    Export,
    Rocket,
    Shield,
    Check,
    Alert,
    Search,
    Plus,
    Close,
    Sort,
    Lightbulb,
    Chat,
    Package,
    Info,
    Appearance,
    Brand,
    Sun,
    Moon,
    /// 侧栏「展开 / 收起菜单」那颗：圆角外框 + 竖栏 + 箭头。
    SidebarExpand,
    SidebarCollapse,
    /// 文件夹 / 本地文件浏览
    Folder,
}

pub fn draw_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let center = rect.center();
    let s = rect.width().min(rect.height());
    // 方案 A 进阶：温润软黑平滑重映射（若传入的是深冷死黑/高反差黑，自动平滑软化为温润暖木炭灰，消除刺眼生冷感）
    let color = if theme::is_light() {
        if color == Color32::BLACK || (color.r() <= 0x55 && color.g() <= 0x55 && color.b() <= 0x58) {
            theme::icon_primary()
        } else {
            color
        }
    } else {
        color
    };
    let stroke = Stroke::new((s * 0.065).clamp(1.15, 1.40), color);
    let fine_stroke = Stroke::new((s * 0.052).clamp(1.0, 1.20), color);
    let bold_stroke = Stroke::new((s * 0.078).clamp(1.25, 1.55), color);

    match icon {
        Icon::Dashboard => {
            // 4 quadrant modern rounded cards
            let pad = s * 0.06;
            let half = s * 0.36;
            let cr = CornerRadius::same(2);
            // Top-left (精细轮廓线框)
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(center.x - half - pad, center.y - half - pad),
                    Pos2::new(center.x - pad, center.y - pad),
                ),
                cr,
                stroke,
                StrokeKind::Middle,
            );
            // Top-right
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(center.x + pad, center.y - half - pad),
                    Pos2::new(center.x + half + pad, center.y - pad),
                ),
                cr,
                fine_stroke,
                StrokeKind::Middle,
            );
            // Bottom-left
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(center.x - half - pad, center.y + pad),
                    Pos2::new(center.x - pad, center.y + half + pad),
                ),
                cr,
                fine_stroke,
                StrokeKind::Middle,
            );
            // Bottom-right (微晶柔和半透底色，剔除生硬死黑)
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(center.x + pad, center.y + pad),
                    Pos2::new(center.x + half + pad, center.y + half + pad),
                ),
                cr,
                color.gamma_multiply(0.25),
            );
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(center.x + pad, center.y + pad),
                    Pos2::new(center.x + half + pad, center.y + half + pad),
                ),
                cr,
                fine_stroke,
                StrokeKind::Middle,
            );
        }
        Icon::Lightning => {
            // Dynamic lightning bolt (通透半透微晶填充 + 细致轮廓描边，彻底解决实心傻大黑粗硬块)
            let pts = vec![
                Pos2::new(center.x + s * 0.06, center.y - s * 0.44),
                Pos2::new(center.x - s * 0.30, center.y + s * 0.04),
                Pos2::new(center.x - s * 0.04, center.y + s * 0.04),
                Pos2::new(center.x - s * 0.12, center.y + s * 0.44),
                Pos2::new(center.x + s * 0.32, center.y - s * 0.04),
                Pos2::new(center.x + s * 0.04, center.y - s * 0.04),
            ];
            painter.add(egui::epaint::PathShape::convex_polygon(
                pts.clone(),
                color.gamma_multiply(0.25),
                Stroke::NONE,
            ));
            painter.add(egui::epaint::PathShape::closed_line(
                pts,
                stroke,
            ));
        }
        Icon::Subscription => {
            // Tray with downward arrow
            let bx1 = center.x - s * 0.38;
            let bx2 = center.x + s * 0.38;
            let by1 = center.y;
            let by2 = center.y + s * 0.36;
            painter.line_segment([Pos2::new(bx1, by1), Pos2::new(bx1, by2)], stroke);
            painter.line_segment([Pos2::new(bx1, by2), Pos2::new(bx2, by2)], stroke);
            painter.line_segment([Pos2::new(bx2, by2), Pos2::new(bx2, by1)], stroke);

            // Down arrow
            let ay1 = center.y - s * 0.40;
            let ay2 = center.y + s * 0.16;
            painter.line_segment([Pos2::new(center.x, ay1), Pos2::new(center.x, ay2)], stroke);
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.18, ay2 - s * 0.18),
                    Pos2::new(center.x, ay2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.18, ay2 - s * 0.18),
                    Pos2::new(center.x, ay2),
                ],
                stroke,
            );
        }
        Icon::Gear => {
            // 精密工业级机械齿轮：真实齿厚轮廓（6 个厚实梯形轮齿 + 轮毂 + 中心轴孔）
            // 彻底解决原先"圆圈+细射线"导致的与日间模式太阳图标严重撞脸冲突
            let r_hole = s * 0.12;
            let r_root = s * 0.25;
            let r_tip = s * 0.38;

            let teeth = 6;
            let mut pts = Vec::with_capacity(teeth * 4);

            for i in 0..teeth {
                let mid_angle = (i as f32) * std::f32::consts::PI / 3.0;
                let a1 = mid_angle - 0.28; // 齿根谷底
                let a2 = mid_angle - 0.14; // 齿顶左侧
                let a3 = mid_angle + 0.14; // 齿顶右侧
                let a4 = mid_angle + 0.28; // 齿根谷底

                pts.push(Pos2::new(center.x + a1.cos() * r_root, center.y + a1.sin() * r_root));
                pts.push(Pos2::new(center.x + a2.cos() * r_tip, center.y + a2.sin() * r_tip));
                pts.push(Pos2::new(center.x + a3.cos() * r_tip, center.y + a3.sin() * r_tip));
                pts.push(Pos2::new(center.x + a4.cos() * r_root, center.y + a4.sin() * r_root));
            }

            // 闭合绘制齿轮轮廓
            for i in 0..pts.len() {
                let next = (i + 1) % pts.len();
                painter.line_segment([pts[i], pts[next]], stroke);
            }
            // 轮毂微晶质感与中轴孔
            painter.circle_filled(center, r_root, color.gamma_multiply(0.18));
            painter.circle_stroke(center, r_hole, stroke);
        }
        Icon::Plug => {
            // API Connector plug
            let body_w = s * 0.50;
            let body_h = s * 0.40;
            let body_rect = Rect::from_center_size(
                Pos2::new(center.x, center.y - s * 0.05),
                Vec2::new(body_w, body_h),
            );
            painter.rect_filled(body_rect, CornerRadius::same(3), color.gamma_multiply(0.2));
            painter.rect_stroke(body_rect, CornerRadius::same(3), stroke, StrokeKind::Inside);

            // Top dual prongs
            let prong_h = s * 0.22;
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.14, body_rect.top()),
                    Pos2::new(center.x - s * 0.14, body_rect.top() - prong_h),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.14, body_rect.top()),
                    Pos2::new(center.x + s * 0.14, body_rect.top() - prong_h),
                ],
                stroke,
            );

            // Bottom cable
            painter.line_segment(
                [
                    Pos2::new(center.x, body_rect.bottom()),
                    Pos2::new(center.x, body_rect.bottom() + s * 0.25),
                ],
                stroke,
            );
        }
        Icon::Pin => {
            // Location map marker
            let top_c = Pos2::new(center.x, center.y - s * 0.14);
            let r = s * 0.26;
            painter.circle_stroke(top_c, r, stroke);
            painter.circle_filled(top_c, r * 0.38, color);

            let bottom_tip = Pos2::new(center.x, center.y + s * 0.42);
            let left_pt = Pos2::new(top_c.x - r * 0.85, top_c.y + r * 0.5);
            let right_pt = Pos2::new(top_c.x + r * 0.85, top_c.y + r * 0.5);
            painter.line_segment([left_pt, bottom_tip], stroke);
            painter.line_segment([right_pt, bottom_tip], stroke);
        }
        Icon::Radar => {
            // Signal broadcast emitter
            let origin = Pos2::new(center.x, center.y + s * 0.22);
            painter.circle_filled(origin, s * 0.10, color);

            // 1st arc
            let r1 = s * 0.28;
            for i in -2..=2 {
                let a1 = -std::f32::consts::FRAC_PI_2 + (i as f32) * 0.35;
                let a2 = -std::f32::consts::FRAC_PI_2 + ((i + 1) as f32) * 0.35;
                painter.line_segment(
                    [
                        Pos2::new(origin.x + r1 * a1.cos(), origin.y + r1 * a1.sin()),
                        Pos2::new(origin.x + r1 * a2.cos(), origin.y + r1 * a2.sin()),
                    ],
                    fine_stroke,
                );
            }
            // 2nd outer arc
            let r2 = s * 0.52;
            for i in -2..=2 {
                let a1 = -std::f32::consts::FRAC_PI_2 + (i as f32) * 0.32;
                let a2 = -std::f32::consts::FRAC_PI_2 + ((i + 1) as f32) * 0.32;
                painter.line_segment(
                    [
                        Pos2::new(origin.x + r2 * a1.cos(), origin.y + r2 * a1.sin()),
                        Pos2::new(origin.x + r2 * a2.cos(), origin.y + r2 * a2.sin()),
                    ],
                    stroke,
                );
            }
        }
        Icon::Rules => {
            // Splitting highway branches
            let base = Pos2::new(center.x, center.y + s * 0.40);
            let fork = Pos2::new(center.x, center.y + s * 0.05);
            let left_top = Pos2::new(center.x - s * 0.32, center.y - s * 0.35);
            let right_top = Pos2::new(center.x + s * 0.32, center.y - s * 0.35);

            painter.line_segment([base, fork], stroke);
            painter.line_segment([fork, left_top], stroke);
            painter.line_segment([fork, right_top], stroke);

            // Arrowheads on branches
            painter.line_segment(
                [Pos2::new(left_top.x, left_top.y + s * 0.16), left_top],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(left_top.x + s * 0.16, left_top.y), left_top],
                stroke,
            );

            painter.line_segment(
                [Pos2::new(right_top.x, right_top.y + s * 0.16), right_top],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(right_top.x - s * 0.16, right_top.y), right_top],
                stroke,
            );
        }
        Icon::Globe => {
            // Globe sphere
            let r = s * 0.38;
            painter.circle_stroke(center, r, stroke);
            // Equator horizontal line
            painter.line_segment(
                [
                    Pos2::new(center.x - r, center.y),
                    Pos2::new(center.x + r, center.y),
                ],
                stroke,
            );
            // Meridian vertical line
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - r),
                    Pos2::new(center.x, center.y + r),
                ],
                stroke,
            );
            // Curved longitude arcs
            let inner_rx = r * 0.52;
            let mut arc_pts_r = Vec::with_capacity(9);
            let mut arc_pts_l = Vec::with_capacity(9);
            for i in -4..=4 {
                let t = (i as f32) / 4.0;
                let y = center.y + t * r;
                let x_offset = (1.0 - t * t).max(0.0).sqrt() * inner_rx;
                arc_pts_r.push(Pos2::new(center.x + x_offset, y));
                arc_pts_l.push(Pos2::new(center.x - x_offset, y));
            }
            painter.add(egui::epaint::PathShape::line(arc_pts_r, fine_stroke));
            painter.add(egui::epaint::PathShape::line(arc_pts_l, fine_stroke));
        }
        Icon::Cpu => {
            // Microprocessor chip
            let body_size = s * 0.52;
            let chip_rect = Rect::from_center_size(center, Vec2::splat(body_size));
            painter.rect_stroke(chip_rect, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.rect_filled(
                Rect::from_center_size(center, Vec2::splat(body_size * 0.45)),
                CornerRadius::same(1),
                color,
            );

            // Pins (2 on each of the 4 sides)
            let pin_len = s * 0.14;
            let offset = body_size * 0.22;
            for sign in [-1.0, 1.0] {
                // Top pins
                let px = center.x + sign * offset;
                painter.line_segment(
                    [
                        Pos2::new(px, chip_rect.top()),
                        Pos2::new(px, chip_rect.top() - pin_len),
                    ],
                    fine_stroke,
                );
                // Bottom pins
                painter.line_segment(
                    [
                        Pos2::new(px, chip_rect.bottom()),
                        Pos2::new(px, chip_rect.bottom() + pin_len),
                    ],
                    fine_stroke,
                );
                // Left pins
                let py = center.y + sign * offset;
                painter.line_segment(
                    [
                        Pos2::new(chip_rect.left(), py),
                        Pos2::new(chip_rect.left() - pin_len, py),
                    ],
                    fine_stroke,
                );
                // Right pins
                painter.line_segment(
                    [
                        Pos2::new(chip_rect.right(), py),
                        Pos2::new(chip_rect.right() + pin_len, py),
                    ],
                    fine_stroke,
                );
            }
        }
        Icon::Terminal => {
            // Terminal prompt frame
            let win_rect = Rect::from_center_size(center, Vec2::new(s * 0.82, s * 0.68));
            painter.rect_stroke(win_rect, CornerRadius::same(3), stroke, StrokeKind::Middle);

            // Prompt chevron '>'
            let cx1 = win_rect.left() + s * 0.16;
            let cy = center.y;
            let arm = s * 0.12;
            painter.line_segment([Pos2::new(cx1, cy - arm), Pos2::new(cx1 + arm, cy)], stroke);
            painter.line_segment([Pos2::new(cx1 + arm, cy), Pos2::new(cx1, cy + arm)], stroke);

            // Cursor line '_'
            let lx1 = cx1 + arm * 2.0;
            let lx2 = lx1 + s * 0.16;
            painter.line_segment([Pos2::new(lx1, cy + arm), Pos2::new(lx2, cy + arm)], stroke);
        }
        Icon::ArrowUp => {
            // Upward arrow
            let y_top = center.y - s * 0.38;
            let y_bot = center.y + s * 0.38;
            painter.line_segment(
                [Pos2::new(center.x, y_bot), Pos2::new(center.x, y_top)],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.28, y_top + s * 0.28),
                    Pos2::new(center.x, y_top),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.28, y_top + s * 0.28),
                    Pos2::new(center.x, y_top),
                ],
                stroke,
            );
        }
        Icon::ArrowDown => {
            // Downward arrow
            let y_top = center.y - s * 0.38;
            let y_bot = center.y + s * 0.38;
            painter.line_segment(
                [Pos2::new(center.x, y_top), Pos2::new(center.x, y_bot)],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.28, y_bot - s * 0.28),
                    Pos2::new(center.x, y_bot),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.28, y_bot - s * 0.28),
                    Pos2::new(center.x, y_bot),
                ],
                stroke,
            );
        }
        Icon::Play => {
            // Equilateral triangle pointing right
            let pts = vec![
                Pos2::new(center.x - s * 0.26, center.y - s * 0.36),
                Pos2::new(center.x - s * 0.26, center.y + s * 0.36),
                Pos2::new(center.x + s * 0.36, center.y),
            ];
            painter.add(egui::epaint::PathShape::convex_polygon(
                pts,
                color,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            // Rounded stop square
            let sq = Rect::from_center_size(center, Vec2::splat(s * 0.60));
            painter.rect_filled(sq, CornerRadius::same(2), color);
        }
        Icon::Sync => {
            // Rotating twin arrows
            let r = s * 0.34;
            // Top arc
            painter.line_segment(
                [
                    Pos2::new(center.x - r, center.y),
                    Pos2::new(center.x, center.y - r),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - r),
                    Pos2::new(center.x + r, center.y),
                ],
                stroke,
            );
            // Top arrowhead
            painter.line_segment(
                [
                    Pos2::new(center.x + r, center.y - s * 0.18),
                    Pos2::new(center.x + r, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + r + s * 0.16, center.y),
                    Pos2::new(center.x + r, center.y),
                ],
                stroke,
            );

            // Bottom arc
            painter.line_segment(
                [
                    Pos2::new(center.x + r, center.y),
                    Pos2::new(center.x, center.y + r),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y + r),
                    Pos2::new(center.x - r, center.y),
                ],
                stroke,
            );
            // Bottom arrowhead
            painter.line_segment(
                [
                    Pos2::new(center.x - r, center.y + s * 0.18),
                    Pos2::new(center.x - r, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - r - s * 0.16, center.y),
                    Pos2::new(center.x - r, center.y),
                ],
                stroke,
            );
        }
        Icon::Trash => {
            // Wastebasket
            let top_bar_y = center.y - s * 0.28;
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.38, top_bar_y),
                    Pos2::new(center.x + s * 0.38, top_bar_y),
                ],
                stroke,
            );
            // Handle
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.14, top_bar_y),
                    Pos2::new(center.x - s * 0.14, top_bar_y - s * 0.12),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.14, top_bar_y - s * 0.12),
                    Pos2::new(center.x + s * 0.14, top_bar_y - s * 0.12),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.14, top_bar_y - s * 0.12),
                    Pos2::new(center.x + s * 0.14, top_bar_y),
                ],
                stroke,
            );
            // Body bucket
            let by = center.y + s * 0.38;
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.28, top_bar_y),
                    Pos2::new(center.x - s * 0.22, by),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.22, by),
                    Pos2::new(center.x + s * 0.22, by),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.22, by),
                    Pos2::new(center.x + s * 0.28, top_bar_y),
                ],
                stroke,
            );
        }
        Icon::Clipboard => {
            // Clipboard frame
            let board = Rect::from_center_size(
                Pos2::new(center.x, center.y + s * 0.05),
                Vec2::new(s * 0.58, s * 0.68),
            );
            painter.rect_stroke(board, CornerRadius::same(2), stroke, StrokeKind::Middle);
            // Top clamp clip
            let clip = Rect::from_center_size(
                Pos2::new(center.x, board.top()),
                Vec2::new(s * 0.26, s * 0.12),
            );
            painter.rect_filled(clip, CornerRadius::same(1), color);
        }
        Icon::Export => {
            // Tray box with upward arrow
            let bx1 = center.x - s * 0.38;
            let bx2 = center.x + s * 0.38;
            let by1 = center.y + s * 0.04;
            let by2 = center.y + s * 0.40;
            painter.line_segment([Pos2::new(bx1, by1), Pos2::new(bx1, by2)], stroke);
            painter.line_segment([Pos2::new(bx1, by2), Pos2::new(bx2, by2)], stroke);
            painter.line_segment([Pos2::new(bx2, by2), Pos2::new(bx2, by1)], stroke);

            // Upward arrow
            let ay1 = center.y + s * 0.16;
            let ay2 = center.y - s * 0.38;
            painter.line_segment([Pos2::new(center.x, ay1), Pos2::new(center.x, ay2)], stroke);
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.20, ay2 + s * 0.20),
                    Pos2::new(center.x, ay2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.20, ay2 + s * 0.20),
                    Pos2::new(center.x, ay2),
                ],
                stroke,
            );
        }
        Icon::Rocket => {
            // Angled rocket
            let nose = Pos2::new(center.x + s * 0.34, center.y - s * 0.34);
            let body1 = Pos2::new(center.x - s * 0.08, center.y - s * 0.28);
            let tail = Pos2::new(center.x - s * 0.28, center.y + s * 0.28);
            let body2 = Pos2::new(center.x + s * 0.28, center.y + s * 0.08);

            painter.line_segment([nose, body1], stroke);
            painter.line_segment([body1, tail], stroke);
            painter.line_segment([tail, body2], stroke);
            painter.line_segment([body2, nose], stroke);

            // Left fin
            painter.line_segment(
                [body1, Pos2::new(center.x - s * 0.32, center.y - s * 0.06)],
                stroke,
            );
            // Right fin
            painter.line_segment(
                [body2, Pos2::new(center.x + s * 0.06, center.y + s * 0.32)],
                stroke,
            );
        }
        Icon::Shield => {
            // Shield outline
            let p_top = Pos2::new(center.x, center.y - s * 0.38);
            let p_tr = Pos2::new(center.x + s * 0.34, center.y - s * 0.22);
            let p_bot = Pos2::new(center.x, center.y + s * 0.40);
            let p_tl = Pos2::new(center.x - s * 0.34, center.y - s * 0.22);

            let pts = vec![
                p_top,
                p_tr,
                Pos2::new(center.x + s * 0.28, center.y + s * 0.14),
                p_bot,
                Pos2::new(center.x - s * 0.28, center.y + s * 0.14),
                p_tl,
            ];
            painter.add(egui::epaint::PathShape::closed_line(pts, stroke));
        }
        Icon::Check => {
            // Bold clean checkmark
            let p1 = Pos2::new(center.x - s * 0.32, center.y);
            let p2 = Pos2::new(center.x - s * 0.08, center.y + s * 0.24);
            let p3 = Pos2::new(center.x + s * 0.34, center.y - s * 0.26);
            painter.line_segment([p1, p2], bold_stroke);
            painter.line_segment([p2, p3], bold_stroke);
        }
        Icon::Alert => {
            // Alert triangle
            let p1 = Pos2::new(center.x, center.y - s * 0.40);
            let p2 = Pos2::new(center.x + s * 0.40, center.y + s * 0.36);
            let p3 = Pos2::new(center.x - s * 0.40, center.y + s * 0.36);
            painter.add(egui::epaint::PathShape::closed_line(vec![p1, p2, p3], stroke));
            // Exclamation mark line & dot
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - s * 0.14),
                    Pos2::new(center.x, center.y + s * 0.12),
                ],
                stroke,
            );
            painter.circle_filled(Pos2::new(center.x, center.y + s * 0.24), s * 0.045, color);
        }
        Icon::Search => {
            // Magnifying glass: circle + angled handle
            let r = s * 0.26;
            let c = Pos2::new(center.x - s * 0.08, center.y - s * 0.08);
            painter.circle_stroke(c, r, stroke);
            let handle_start = Pos2::new(c.x + r * 0.707, c.y + r * 0.707);
            let handle_end = Pos2::new(center.x + s * 0.36, center.y + s * 0.36);
            painter.line_segment([handle_start, handle_end], bold_stroke);
        }
        Icon::Plus => {
            // Plus sign
            let len = s * 0.30;
            painter.line_segment(
                [
                    Pos2::new(center.x - len, center.y),
                    Pos2::new(center.x + len, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - len),
                    Pos2::new(center.x, center.y + len),
                ],
                stroke,
            );
        }
        Icon::Close => {
            // X mark
            let d = s * 0.24;
            painter.line_segment(
                [
                    Pos2::new(center.x - d, center.y - d),
                    Pos2::new(center.x + d, center.y + d),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + d, center.y - d),
                    Pos2::new(center.x - d, center.y + d),
                ],
                stroke,
            );
        }
        Icon::Sort => {
            // 3 horizontal bars with decreasing width
            let y1 = center.y - s * 0.26;
            let y2 = center.y;
            let y3 = center.y + s * 0.26;
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.36, y1),
                    Pos2::new(center.x + s * 0.36, y1),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.24, y2),
                    Pos2::new(center.x + s * 0.24, y2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.12, y3),
                    Pos2::new(center.x + s * 0.12, y3),
                ],
                stroke,
            );
        }
        Icon::Lightbulb => {
            // Lightbulb bulb & filament base
            let bulb_c = Pos2::new(center.x, center.y - s * 0.10);
            painter.circle_stroke(bulb_c, s * 0.25, stroke);
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.12, center.y + s * 0.18),
                    Pos2::new(center.x + s * 0.12, center.y + s * 0.18),
                ],
                fine_stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.08, center.y + s * 0.28),
                    Pos2::new(center.x + s * 0.08, center.y + s * 0.28),
                ],
                fine_stroke,
            );
        }
        Icon::Chat => {
            // Chat bubble
            let b_rect = Rect::from_center_size(
                Pos2::new(center.x, center.y - s * 0.06),
                Vec2::new(s * 0.72, s * 0.52),
            );
            painter.rect_stroke(b_rect, CornerRadius::same(3), stroke, StrokeKind::Inside);
            let tail = vec![
                Pos2::new(center.x - s * 0.22, b_rect.bottom()),
                Pos2::new(center.x - s * 0.28, center.y + s * 0.36),
                Pos2::new(center.x - s * 0.06, b_rect.bottom()),
            ];
            painter.add(egui::epaint::PathShape::convex_polygon(
                tail,
                color,
                Stroke::NONE,
            ));
        }
        Icon::Package => {
            // Isometric box / package
            let top_pts = vec![
                Pos2::new(center.x, center.y - s * 0.36),
                Pos2::new(center.x + s * 0.32, center.y - s * 0.16),
                Pos2::new(center.x, center.y + s * 0.04),
                Pos2::new(center.x - s * 0.32, center.y - s * 0.16),
            ];
            painter.add(egui::epaint::PathShape::line(top_pts.clone(), stroke));
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.32, center.y - s * 0.16),
                    Pos2::new(center.x - s * 0.32, center.y + s * 0.20),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.32, center.y - s * 0.16),
                    Pos2::new(center.x + s * 0.32, center.y + s * 0.20),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y + s * 0.04),
                    Pos2::new(center.x, center.y + s * 0.40),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x - s * 0.32, center.y + s * 0.20),
                    Pos2::new(center.x, center.y + s * 0.40),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(center.x + s * 0.32, center.y + s * 0.20),
                    Pos2::new(center.x, center.y + s * 0.40),
                ],
                stroke,
            );
        }
        Icon::Info => {
            // 圆圈里一个 "i" —— 侧栏「关于」和关于页用的符号
            let r = s * 0.38;
            painter.circle_stroke(center, r, stroke);
            painter.circle_filled(Pos2::new(center.x, center.y - s * 0.16), s * 0.055, color);
            painter.line_segment(
                [
                    Pos2::new(center.x, center.y - s * 0.04),
                    Pos2::new(center.x, center.y + s * 0.20),
                ],
                bold_stroke,
            );
        }
        Icon::Appearance => {
            // 右半边实心的圆 —— 深浅/外观通用的统一符号
            let r = s * 0.38;
            painter.circle_stroke(center, r, stroke);
            let mut half = Vec::with_capacity(15);
            half.push(Pos2::new(center.x, center.y - r));
            for step in 0..=12 {
                let angle =
                    std::f32::consts::PI * (step as f32 / 12.0) - std::f32::consts::FRAC_PI_2;
                half.push(Pos2::new(
                    center.x + angle.cos() * r,
                    center.y + angle.sin() * r,
                ));
            }
            half.push(Pos2::new(center.x, center.y + r));
            painter.add(egui::epaint::PathShape::convex_polygon(
                half,
                color,
                Stroke::NONE,
            ));
            painter.line_segment(
                [Pos2::new(center.x, center.y - r), Pos2::new(center.x, center.y + r)],
                stroke,
            );
        }
        Icon::Brand => {
            // 极简现代 "V" 纯几何字标
            let v_pts = vec![
                Pos2::new(center.x - s * 0.36, center.y - s * 0.28),
                Pos2::new(center.x, center.y + s * 0.34),
                Pos2::new(center.x + s * 0.36, center.y - s * 0.28),
                Pos2::new(center.x + s * 0.18, center.y - s * 0.28),
                Pos2::new(center.x, center.y + s * 0.06),
                Pos2::new(center.x - s * 0.18, center.y - s * 0.28),
            ];
            painter.add(egui::epaint::PathShape::convex_polygon(
                v_pts,
                color,
                Stroke::NONE,
            ));
        }
        Icon::SidebarExpand | Icon::SidebarCollapse => {
            // 外框是一整块面板，左边一条竖栏表示侧栏本身；箭头只表方向：
            // 现在是收起态就画朝右的箭头（点下去展开），展开态画朝左的（点下去收回）。
            let half_w = s * 0.36;
            let half_h = s * 0.30;
            let frame = Rect::from_min_max(
                Pos2::new(center.x - half_w, center.y - half_h),
                Pos2::new(center.x + half_w, center.y + half_h),
            );
            painter.rect_stroke(
                frame,
                CornerRadius::same(((s * 0.10).max(1.0)) as u8),
                stroke,
                StrokeKind::Inside,
            );
            let split_x = frame.min.x + frame.width() * 0.34;
            painter.line_segment(
                [
                    Pos2::new(split_x, frame.min.y),
                    Pos2::new(split_x, frame.max.y),
                ],
                fine_stroke,
            );
            let dir = if matches!(icon, Icon::SidebarCollapse) {
                -1.0_f32
            } else {
                1.0_f32
            };
            let anchor = split_x + (frame.max.x - split_x) * 0.5;
            // 一个干净的 V 形箭头：两条边收在同一个尖上，高度给到 0.26s，才不会在 21px 下糊成一团。
            let arm = s * 0.13;
            let tip = Pos2::new(anchor + dir * s * 0.10, center.y);
            let back_x = anchor - dir * s * 0.05;
            painter.line_segment([Pos2::new(back_x, center.y - arm), tip], bold_stroke);
            painter.line_segment([Pos2::new(back_x, center.y + arm), tip], bold_stroke);
        }
        Icon::Sun => {
            // 阳光暖阳天体：饱满的微晶阳光核心圆盘 + 8 束精短阳光金芒
            // 与机械齿轮的厚齿轮廓和中心空心轴孔形成强烈视觉质感差异，绝不混淆
            let r_core = s * 0.17;
            painter.circle_filled(center, r_core, color.gamma_multiply(0.28));
            painter.circle_stroke(center, r_core, stroke);

            let ray_start = s * 0.25;
            let ray_end = s * 0.38;
            for i in 0..8 {
                let angle = (i as f32) * std::f32::consts::PI / 4.0;
                let (sin, cos) = angle.sin_cos();
                let p1 = Pos2::new(center.x + cos * ray_start, center.y + sin * ray_start);
                let p2 = Pos2::new(center.x + cos * ray_end, center.y + sin * ray_end);
                painter.line_segment([p1, p2], fine_stroke);
            }
        }
        Icon::Moon => {
            // 现代极简几何新月形（外圆弧与内切弧完美相交，无尖刺毛刺）
            let r = s * 0.38;
            let alpha_top = -0.38 * std::f32::consts::PI;
            let alpha_bot = 0.38 * std::f32::consts::PI;
            let p_top = Pos2::new(center.x + r * alpha_top.cos(), center.y + r * alpha_top.sin());
            let p_bot = Pos2::new(center.x + r * alpha_bot.cos(), center.y + r * alpha_bot.sin());

            let c2 = Pos2::new(center.x + s * 0.16, center.y);
            let beta_top = (p_top.y - c2.y).atan2(p_top.x - c2.x);
            let beta_bot = (p_bot.y - c2.y).atan2(p_bot.x - c2.x);
            let r_inner = ((p_top.x - c2.x).powi(2) + (p_top.y - c2.y).powi(2)).sqrt();

            let steps = 16;
            let mut outer_pts = Vec::with_capacity(steps + 1);
            let mut inner_pts = Vec::with_capacity(steps + 1);

            // 外弧：从上尖端 p_top 逆时针向左环绕到下尖端 p_bot
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let a = alpha_top - t * (2.0 * std::f32::consts::PI - (alpha_bot - alpha_top));
                outer_pts.push(Pos2::new(center.x + r * a.cos(), center.y + r * a.sin()));
            }

            // 内弧：从上尖端 p_top 沿内圆弧平滑过渡到下尖端 p_bot
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let b = beta_top - t * (2.0 * std::f32::consts::PI - (beta_bot - beta_top));
                inner_pts.push(Pos2::new(c2.x + r_inner * b.cos(), c2.y + r_inner * b.sin()));
            }

            // 用相邻顶点分割出的凸四边形填充内部，彻底杜绝凹多边形三角化出现的畸变和尖刺
            for i in 0..steps {
                let quad = vec![
                    outer_pts[i],
                    outer_pts[i + 1],
                    inner_pts[i + 1],
                    inner_pts[i],
                ];
                painter.add(egui::epaint::PathShape::convex_polygon(
                    quad,
                    color,
                    Stroke::NONE,
                ));
            }

            // 闭合外边框轮廓线，与 Sun 图标保持一致的高清质感
            let mut closed_pts = outer_pts;
            closed_pts.extend(inner_pts.into_iter().rev());
            painter.add(egui::epaint::PathShape::closed_line(closed_pts, fine_stroke));
        }
        Icon::Folder => {
            // 现代精致文件夹：顶部标签页 + 文件夹主体
            let half_w = s * 0.38;
            let half_h = s * 0.28;
            let body_top = center.y - half_h + s * 0.12;
            let body = Rect::from_min_max(
                Pos2::new(center.x - half_w, body_top),
                Pos2::new(center.x + half_w, center.y + half_h),
            );
            // 浅透底色填充主体增加质感
            painter.rect_filled(body, CornerRadius::same(2), color.gamma_multiply(0.12));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);

            // 顶部小标签（从左边向上突起一个小梯形）
            let tab_x0 = center.x - half_w;
            let tab_x1 = center.x - s * 0.08;
            let tab_x2 = center.x - s * 0.02;
            let tab_y = center.y - half_h;
            painter.line_segment([Pos2::new(tab_x0, body_top), Pos2::new(tab_x0, tab_y)], stroke);
            painter.line_segment([Pos2::new(tab_x0, tab_y), Pos2::new(tab_x1, tab_y)], stroke);
            painter.line_segment([Pos2::new(tab_x1, tab_y), Pos2::new(tab_x2, body_top)], stroke);
        }
    }
}

pub fn icon_label(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        draw_icon(ui.painter(), rect, icon, color);
    }
    resp
}

#[allow(dead_code)]
pub fn button_with_icon<'a>(icon: Icon, text: &'a str, color: Color32) -> impl egui::Widget + 'a {
    move |ui: &mut Ui| {
        let padding = Vec2::new(10.0, 6.0);
        let icon_size = 14.0;
        let icon_gap = 6.0;

        let galley =
            ui.painter()
                .layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), color);

        let desired_size = Vec2::new(
            padding.x * 2.0 + icon_size + icon_gap + galley.size().x,
            padding.y * 2.0 + galley.size().y.max(icon_size),
        );

        let (rect, resp) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let (bg_fill, border_stroke) = if resp.is_pointer_button_down_on() {
                (theme::panel_hover(), Stroke::new(1.0_f32, theme::accent()))
            } else if resp.hovered() {
                (theme::panel_hover(), Stroke::new(1.0_f32, theme::border()))
            } else {
                (theme::panel(), Stroke::new(1.0_f32, theme::border()))
            };

            ui.painter().rect(
                rect,
                CornerRadius::same(6),
                bg_fill,
                border_stroke,
                StrokeKind::Inside,
            );

            let icon_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + padding.x, rect.center().y - icon_size / 2.0),
                Vec2::splat(icon_size),
            );
            let cur_color = if resp.hovered() {
                theme::icon_primary()
            } else {
                color
            };
            draw_icon(ui.painter(), icon_rect, icon, cur_color);

            let text_pos = Pos2::new(
                icon_rect.max.x + icon_gap,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(text_pos, galley, cur_color);
        }
        resp
    }
}

#[allow(dead_code)]
pub fn primary_icon_btn<'a>(icon: Icon, text: &'a str) -> impl egui::Widget + 'a {
    move |ui: &mut Ui| {
        let padding = Vec2::new(12.0, 6.0);
        let icon_size = 14.0;
        let icon_gap = 6.0;

        let galley = ui.painter().layout_no_wrap(
            text.to_string(),
            egui::FontId::proportional(13.0),
            theme::on_primary(),
        );

        let desired_size = Vec2::new(
            padding.x * 2.0 + icon_size + icon_gap + galley.size().x,
            padding.y * 2.0 + galley.size().y.max(icon_size),
        );

        let (rect, resp) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let bg_fill = if resp.is_pointer_button_down_on() {
                theme::primary_btn_press()
            } else if resp.hovered() {
                theme::primary_btn_hover()
            } else {
                theme::primary_btn()
            };

            ui.painter().rect(
                rect,
                CornerRadius::same(6),
                bg_fill,
                Stroke::NONE,
                StrokeKind::Inside,
            );

            let icon_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + padding.x, rect.center().y - icon_size / 2.0),
                Vec2::splat(icon_size),
            );
            draw_icon(ui.painter(), icon_rect, icon, theme::on_primary());

            let text_pos = Pos2::new(
                icon_rect.max.x + icon_gap,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(text_pos, galley, theme::on_primary());
        }
        resp
    }
}

#[allow(dead_code)]
pub fn danger_icon_btn<'a>(icon: Icon, text: &'a str) -> impl egui::Widget + 'a {
    move |ui: &mut Ui| {
        let padding = Vec2::new(11.0, 6.0);
        let icon_size = 14.0;
        let icon_gap = 6.0;

        let font_id = egui::FontId::proportional(13.0);
        let text_str = text.to_string();
        let text_size = ui.fonts(|f| f.layout_no_wrap(text_str.clone(), font_id.clone(), Color32::WHITE)).size();

        let desired_size = Vec2::new(
            padding.x * 2.0 + icon_size + icon_gap + text_size.x,
            padding.y * 2.0 + text_size.y.max(icon_size),
        );

        let (rect, resp) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let (bg_fill, stroke_color, fg_color) = if resp.hovered() {
                (theme::danger_bg(), theme::danger_stroke(), theme::danger())
            } else {
                (theme::panel_hover(), theme::border_subtle(), theme::text_secondary())
            };

            ui.painter().rect(
                rect,
                CornerRadius::same(6),
                bg_fill,
                Stroke::new(1.0_f32, stroke_color),
                StrokeKind::Inside,
            );

            let icon_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + padding.x, rect.center().y - icon_size / 2.0),
                Vec2::splat(icon_size),
            );
            draw_icon(ui.painter(), icon_rect, icon, fg_color);

            let galley = ui.painter().layout_no_wrap(text_str, font_id, fg_color);
            let text_pos = Pos2::new(
                icon_rect.max.x + icon_gap,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(text_pos, galley, fg_color);
        }
        resp
    }
}

#[allow(dead_code)]
pub fn selectable_icon_btn<'a>(
    icon: Icon,
    text: &'a str,
    selected: bool,
) -> impl egui::Widget + 'a {
    move |ui: &mut Ui| {
        let padding = Vec2::new(9.0, 5.0);
        let icon_size = 13.0;
        let icon_gap = 5.0;

        let text_color = if selected {
            theme::accent()
        } else {
            theme::muted()
        };

        let galley = ui.painter().layout_no_wrap(
            text.to_string(),
            egui::FontId::proportional(12.5),
            text_color,
        );

        let desired_size = Vec2::new(
            padding.x * 2.0 + icon_size + icon_gap + galley.size().x,
            padding.y * 2.0 + galley.size().y.max(icon_size),
        );

        let (rect, resp) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let (bg_fill, stroke) = if selected {
                (theme::accent_bg(), Stroke::new(1.0_f32, theme::accent()))
            } else if resp.hovered() {
                (theme::panel_hover(), Stroke::new(1.0_f32, theme::border()))
            } else {
                (theme::panel(), Stroke::new(1.0_f32, theme::border()))
            };

            ui.painter().rect(
                rect,
                CornerRadius::same(6),
                bg_fill,
                stroke,
                StrokeKind::Inside,
            );

            let icon_rect = Rect::from_min_size(
                Pos2::new(rect.min.x + padding.x, rect.center().y - icon_size / 2.0),
                Vec2::splat(icon_size),
            );
            draw_icon(ui.painter(), icon_rect, icon, text_color);

            let text_pos = Pos2::new(
                icon_rect.max.x + icon_gap,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(text_pos, galley, text_color);
        }
        resp
    }
}

#[allow(dead_code)]
pub fn icon_btn_square(icon: Icon, size: f32) -> impl egui::Widget {
    move |ui: &mut Ui| {
        let box_size = size + 8.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let (bg, border, color) = if resp.hovered() {
                (
                    theme::accent_bg(),
                    Stroke::new(1.0_f32, theme::accent()),
                    theme::accent(),
                )
            } else {
                (
                    theme::panel(),
                    Stroke::new(1.0_f32, theme::border()),
                    theme::muted(),
                )
            };
            ui.painter()
                .rect(rect, CornerRadius::same(4), bg, border, StrokeKind::Inside);
            let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(size));
            draw_icon(ui.painter(), icon_rect, icon, color);
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 侧栏每一栏的图标都必须真的画出形状。`draw_icon` 是个穷尽 match，新加一个 `Icon`
    /// 变体却忘了画它照样编译通过，界面上只剩一个空白格 —— 2026-10-14 他说"关于菜单要
    /// 加图标"就是这一类问题，所以把它钉成断言。
    #[test]
    fn each_sidebar_icon_paints_shapes() {
        let ctx = egui::Context::default();
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("icon_probe"));
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)));
        let mut painted: Vec<(Icon, usize)> = Vec::new();
        let _ = ctx.run(input, |ctx| {
            let painter = egui::Painter::new(ctx.clone(), layer, ctx.screen_rect());
            for icon in [
                Icon::Dashboard,
                Icon::Lightning,
                Icon::Subscription,
                Icon::Gear,
                Icon::Plug,
                Icon::Info,
                Icon::Trash,
                Icon::Check,
                Icon::Alert,
                Icon::Appearance,
                Icon::Brand,
                Icon::Sun,
                Icon::Moon,
                Icon::SidebarExpand,
                Icon::SidebarCollapse,
                Icon::Folder,
            ] {
                let before = ctx.graphics_mut(|g| g.entry(layer).all_entries().len());
                draw_icon(
                    &painter,
                    Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::splat(17.0)),
                    icon,
                    Color32::WHITE,
                );
                let after = ctx.graphics_mut(|g| g.entry(layer).all_entries().len());
                painted.push((icon, after - before));
            }
        });
        for (icon, count) in painted {
            assert!(count > 0, "{icon:?} 一个形状都没画，菜单上会是个空白格");
        }
    }
}

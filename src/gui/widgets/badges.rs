use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};

/// 状态徽章。返回整块的 `Response`，让调用方还能挂 hover —— 徽章自己要说清
/// "为什么是这个状态"，不该让人去翻状态栏。
pub fn status_badge(ui: &mut Ui, text: &str, color: Color32) -> egui::Response {
    Frame::NONE
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.40)))
        .corner_radius(CornerRadius::same(theme::RADIUS_BTN))
        .inner_margin(Margin::symmetric(12, 7))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter()
                    .circle_filled(rect.center(), 4.0, color.gamma_multiply(0.35));
                ui.painter().circle_filled(rect.center(), 2.5, color);
                ui.label(RichText::new(text).strong().color(color).size(13.0));
            });
        })
        .response
}

/// 协议小胶囊：微晶磨砂色彩微标
pub fn protocol_pill(ui: &mut Ui, protocol: &str) {
    let color = protocol_color(protocol);
    Frame::NONE
        .fill(color.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.35)))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(protocol)
                    .strong()
                    .size(10.5)
                    .color(color),
            );
        });
}

/// 订阅源小微标：展示节点所属的订阅源
pub fn subscription_pill(ui: &mut Ui, source_name: &str, max_text_width: f32) {
    let text = source_name.trim();
    if text.is_empty() {
        return;
    }
    Frame::NONE
        .fill(theme::panel_hover())
        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(5, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(3.0, 0.0);
                icon_label(ui, Icon::Subscription, 9.0, theme::icon_muted());
                let font_id = egui::TextStyle::Small.resolve(ui.style());
                let text_w = ui.fonts(|f| {
                    f.layout_no_wrap(text.to_string(), font_id, theme::text_secondary())
                        .size()
                        .x
                });
                let alloc_w = text_w.clamp(14.0, max_text_width);
                ui.add_sized(
                    [alloc_w, 15.0],
                    egui::Label::new(
                        RichText::new(text)
                            .small()
                            .size(10.5)
                            .color(theme::text_secondary()),
                    )
                    .truncate(),
                );
            });
        })
        .response
        .on_hover_text(format!("订阅源: {text}"));
}


/// 协议专属色彩标识
pub fn protocol_color(protocol: &str) -> Color32 {
    let lower = protocol.to_lowercase();
    match lower.as_str() {
        "vless" | "vl" => theme::accent(),
        "vmess" | "vm" => Color32::from_rgb(2, 132, 199),
        "trojan" | "tr" => Color32::from_rgb(245, 158, 11),
        "shadowsocks" | "ss" | "shadowsocksr" | "ssr" => Color32::from_rgb(16, 185, 129),
        "hysteria" | "hysteria2" | "hy" | "hy2" => Color32::from_rgb(168, 85, 247),
        "wireguard" | "wg" => Color32::from_rgb(99, 102, 241),
        "tuic" | "tu" => Color32::from_rgb(236, 72, 153),
        "masque" | "mq" => Color32::from_rgb(13, 148, 136),
        _ => theme::text_secondary(),
    }
}


#[allow(dead_code)]
pub fn stat_chip(ui: &mut Ui, label: &str, value: &str, color: Color32) {
    // 不能用 `Frame::show`：它在 `horizontal_wrapped` 里**不参与换行判定**，芯片一多整行
    // 就冲出右边界，还会把父 ui 的 `max_rect` 一起撑大（离屏量到 1535 > 面板 1312），
    // 于是下面每一行"钉右"的表头都跟着画到屏幕外。自己量尺寸再 `allocate_at_least`
    // 才会正常换行 —— 分配走 wrapped layout 的 same_row_or_next_row。
    const PAD_X: f32 = 13.0;
    const PAD_Y: f32 = 8.0;
    const GAP: f32 = 6.0;
    let label_galley = egui::WidgetText::from(RichText::new(label).small().color(theme::muted()))
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
    let value_galley = egui::WidgetText::from(
        RichText::new(value).strong().color(color).size(13.5),
    )
    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
    let size = egui::vec2(
        label_galley.size().x + GAP + value_galley.size().x + PAD_X * 2.0,
        label_galley.size().y.max(value_galley.size().y) + PAD_Y * 2.0,
    );
    let label_galley = std::sync::Arc::new(label_galley);
    let value_galley = std::sync::Arc::new(value_galley);
    let (rect, _response) = ui.allocate_at_least(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_BTN),
            theme::accent_bg(),
        );
        let center_y = rect.center().y;
        let mut x = rect.left() + PAD_X;
        for galley in [label_galley.as_ref(), value_galley.as_ref()] {
            let top = center_y - galley.size().y / 2.0;
            painter.galley(egui::pos2(x, top), galley.clone(), theme::text_secondary());
            x += galley.size().x + GAP;
        }
    }
}

/// 单个词的小胶囊（版本号、许可、状态）。和 [`stat_chip`] 同一套"自己量再画"的写法，
/// 不用 `Frame::show`：后者在 `horizontal_wrapped` 里不参与换行判定，会把父 `max_rect`
/// 撑大（`stat_chip` 那条注释记着实测数字）。
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    const PAD_X: f32 = 10.0;
    const PAD_Y: f32 = 5.0;
    let galley = egui::WidgetText::from(RichText::new(text).strong().size(11.5).color(fg))
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
    let size = galley.size() + Vec2::new(PAD_X * 2.0, PAD_Y * 2.0);
    let (rect, _response) = ui.allocate_at_least(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS_BTN), bg);
        painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    }
}

/// 协议首字母/双字母缩写图标 + 数量胶囊徽标
pub fn protocol_stat_chip(
    ui: &mut Ui,
    protocol: &str,
    count: i64,
    is_active: bool,
) -> egui::Response {
    let lower = protocol.to_lowercase();
    let code: &str = match lower.as_str() {
        "all" | "全部" => "All",
        "vless" => "VL",
        "vmess" => "VM",
        "trojan" => "TR",
        "shadowsocks" | "ss" => "SS",
        "shadowsocksr" | "ssr" => "SR",
        "hysteria" | "hysteria2" | "hy2" => "HY",
        "wireguard" | "wg" => "WG",
        "masque" => "MQ",
        "tuic" => "TU",
        "snell" => "SN",
        "http" | "https" => "HT",
        "socks5" | "socks" => "S5",
        _ => {
            if protocol.len() >= 2 {
                &protocol[..2]
            } else if !protocol.is_empty() {
                protocol
            } else {
                "P"
            }
        }
    };

    // 协议首字母/缩写双字母个性化配色
    let (badge_bg, badge_fg) = match code {
        "All" => (theme::panel_hover(), theme::text_primary()),
        "VL" => (theme::accent_bg(), theme::accent_strong()),
        "VM" => (Color32::from_rgb(224, 242, 254), Color32::from_rgb(2, 132, 199)),
        "TR" => (theme::warning_bg(), theme::warning()),
        "SS" | "SR" => (theme::success_bg(), theme::success()),
        "HY" => (Color32::from_rgb(243, 232, 255), Color32::from_rgb(147, 51, 234)),
        "WG" => (Color32::from_rgb(238, 242, 255), Color32::from_rgb(79, 70, 229)),
        "MQ" => (Color32::from_rgb(204, 251, 241), Color32::from_rgb(13, 148, 136)),
        _ => (theme::panel_hover(), theme::text_secondary()),
    };

    const PAD_X: f32 = 7.0;
    const PAD_Y: f32 = 4.0;
    const GAP: f32 = 6.0;
    let badge_w = if code.len() >= 3 { 25.0 } else { 21.0 };
    const BADGE_H: f32 = 18.0;

    let count_str = format!("{count}");
    let count_galley = egui::WidgetText::from(
        RichText::new(&count_str)
            .strong()
            .color(if is_active { theme::accent_strong() } else { theme::text_primary() })
            .size(12.0),
    )
    .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);

    let total_w = PAD_X * 2.0 + badge_w + GAP + count_galley.size().x;
    let total_h = (BADGE_H.max(count_galley.size().y) + PAD_Y * 2.0).max(26.0);
    let size = egui::vec2(total_w, total_h);

    let (rect, response) = ui.allocate_at_least(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let (bg, stroke) = if is_active {
            (theme::accent_bg(), Stroke::new(1.2_f32, theme::accent()))
        } else if response.hovered() {
            (theme::panel_hover(), Stroke::new(1.0_f32, theme::border()))
        } else {
            (theme::panel(), Stroke::new(1.0_f32, theme::border_subtle()))
        };

        painter.rect(
            rect,
            CornerRadius::same(theme::RADIUS_BTN),
            bg,
            stroke,
            egui::StrokeKind::Inside,
        );

        // 绘制双字母/All缩写徽标
        let badge_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + PAD_X + badge_w / 2.0, rect.center().y),
            egui::vec2(badge_w, BADGE_H),
        );
        painter.rect_filled(badge_rect, CornerRadius::same(4), badge_bg);
        painter.rect_stroke(
            badge_rect,
            CornerRadius::same(4),
            Stroke::new(0.8_f32, badge_fg.gamma_multiply(0.4)),
            egui::StrokeKind::Inside,
        );

        let initial_galley = egui::WidgetText::from(
            RichText::new(code).strong().color(badge_fg).size(10.5),
        )
        .into_galley(ui, None, f32::INFINITY, egui::TextStyle::Body);
        let init_pos = egui::pos2(
            badge_rect.center().x - initial_galley.size().x / 2.0,
            badge_rect.center().y - initial_galley.size().y / 2.0,
        );
        painter.galley(init_pos, initial_galley, badge_fg);

        // 绘制数量文字
        let count_pos = egui::pos2(
            badge_rect.right() + GAP,
            rect.center().y - count_galley.size().y / 2.0,
        );
        painter.galley(count_pos, count_galley, theme::text_primary());
    }

    let tooltip = if protocol == "all" || protocol == "All" || protocol == "全部" {
        format!("全部节点：{} 个节点（点击展示全部）", count)
    } else {
        format!("{} ({}) 协议：{} 个节点（点击快速筛选）", protocol.to_uppercase(), code, count)
    };
    response.on_hover_text(tooltip)
}

/// 卡片表头左边那个圆角方块图标底（40×40）。同 [`pill`]：直接画，不套 `Frame`。
pub fn icon_tile(ui: &mut Ui, icon: Icon, fg: Color32, bg: Color32) {
    const TILE: f32 = 40.0;
    const RADIUS: u8 = 13;
    let (rect, _response) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(RADIUS), bg);
        crate::gui::icons::draw_icon(painter, rect.shrink(9.5), icon, fg);
    }
}


pub fn country_flag_and_name(code: &str) -> (&'static str, &'static str) {
    match code.to_ascii_uppercase().as_str() {
        "HK" => ("🇭🇰", "香港"),
        "JP" => ("🇯🇵", "日本"),
        "US" => ("🇺🇸", "美国"),
        "SG" => ("🇸🇬", "新加坡"),
        "TW" => ("🇨🇳", "台湾"),
        "KR" => ("🇰🇷", "韩国"),
        "GB" | "UK" => ("🇬🇧", "英国"),
        "DE" => ("🇩🇪", "德国"),
        "FR" => ("🇫🇷", "法国"),
        "CA" => ("🇨🇦", "加拿大"),
        "AU" => ("🇦🇺", "澳大利亚"),
        "RU" => ("🇷🇺", "俄罗斯"),
        "IN" => ("🇮🇳", "印度"),
        "NL" => ("🇳🇱", "荷兰"),
        "MY" => ("🇲🇾", "马来西亚"),
        "TH" => ("🇹🇭", "泰国"),
        "PH" => ("🇵🇭", "菲律宾"),
        "VN" => ("🇻🇳", "越南"),
        "TR" => ("🇹🇷", "土耳其"),
        _ => ("🌐", "其它"),
    }
}

/// 方案 B：国家/地区专属两字母大写代码与鲜明地域色彩 (Pure Typography)
pub fn country_code_and_color(code: &str) -> (&'static str, Color32) {
    match code.to_ascii_uppercase().as_str() {
        "HK" => ("HK", Color32::from_rgb(168, 85, 247)), // 紫荆洋紫
        "JP" => ("JP", Color32::from_rgb(239, 68, 68)),  // 樱花绯红
        "US" => ("US", Color32::from_rgb(59, 130, 246)),  // 经典钴蓝
        "SG" => ("SG", Color32::from_rgb(244, 63, 94)),  // 鱼尾狮红
        "TW" => ("TW", Color32::from_rgb(6, 182, 212)),  // 海岛翠碧
        "KR" => ("KR", Color32::from_rgb(99, 102, 241)),  // 太极靛蓝
        "GB" | "UK" => ("UK", Color32::from_rgb(79, 70, 229)), // 大英深蓝
        "DE" => ("DE", Color32::from_rgb(245, 158, 11)), // 黑金暖琥珀
        "FR" => ("FR", Color32::from_rgb(37, 99, 235)),  // 法兰西蓝
        "CA" => ("CA", Color32::from_rgb(248, 113, 113)),// 枫叶艳红
        "AU" => ("AU", Color32::from_rgb(16, 185, 129)), // 欧泊翠绿
        "CN" => ("CN", Color32::from_rgb(225, 29, 72)),  // 中国红
        "RU" => ("RU", Color32::from_rgb(100, 116, 139)),// 石板蓝灰
        "IN" => ("IN", Color32::from_rgb(234, 88, 12)),  // 藏红花橙
        "NL" => ("NL", Color32::from_rgb(249, 115, 22)), // 郁金香橙
        "MY" => ("MY", Color32::from_rgb(234, 179, 8)),  // 马来琥珀黄
        "TH" => ("TH", Color32::from_rgb(139, 92, 246)), // 暹罗紫
        "PH" => ("PH", Color32::from_rgb(2, 132, 199)),  // 菲律宾海蓝
        "VN" => ("VN", Color32::from_rgb(220, 38, 38)),  // 越南赤红
        "TR" => ("TR", Color32::from_rgb(225, 29, 72)),  // 土耳其红
        _ => ("GL", Color32::from_rgb(142, 142, 147)),   // 全球/未知 (中性灰)
    }
}

/// 方案 B：极简双色无框字母国旗标识 (Pure Typography Country Code)
pub fn render_country_code_badge(ui: &mut Ui, code: &str) -> egui::Response {
    let (code_str, color) = country_code_and_color(code);
    let (_flag, country_name) = country_flag_and_name(code);
    ui.add(
        egui::Label::new(
            RichText::new(code_str)
                .strong()
                .size(12.0)
                .color(color),
        ),
    )
    .on_hover_text(format!("地区: {country_name} ({code_str})"))
}


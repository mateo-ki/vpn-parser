//! 界面配色：一套语义化 Palette + 若干具名主题。
//!
//! 为什么用进程级静态而不是把 `Palette` 一层层传进每个 view：那些颜色引用有 300 多处，
//! 而且一半在拿不到 `&Ui` 的 free helper（`card_frame()`/`primary_btn()`）里。改成
//! `theme::panel()` 这种零参访问，helper 签名不变、view 里就地替换即可，主题切换只需要
//! 重写这一个静态 + 一次 `ctx.set_style`。
use eframe::egui::{self, Color32, Stroke, Vec2};
use once_cell::sync::Lazy;
use std::sync::RwLock;

/// 一处语义色。字段名和 `widgets.rs` 里那批常量一一对应，默认主题（obsidian）就是它们。
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub page_bg: Color32,
    pub sidebar: Color32,
    pub panel: Color32,
    pub panel_hover: Color32,
    pub border: Color32,
    pub border_subtle: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_bg: Color32,
    /// 淡强调底色之上的文字色：浅色主题下 accent 本身太亮（实测 2.78:1），要压深一档。
    pub accent_strong: Color32,
    pub primary_btn: Color32,
    pub primary_btn_hover: Color32,
    pub primary_btn_press: Color32,
    /// 主按钮上的文字色：浅底按钮要用深色字，深底按钮要用浅色字。
    pub on_primary: Color32,
    pub success: Color32,
    pub success_bg: Color32,
    pub warning: Color32,
    pub warning_bg: Color32,
    pub danger: Color32,
    pub danger_bg: Color32,
    /// 危险按钮的描边：从 danger 与底色推出来，保证浅底主题下也不会糊成一片。
    pub danger_stroke: Color32,
    pub sidebar_text: Color32,
    pub sidebar_muted: Color32,
    /// 浅色主题（决定 egui Visuals 走 light 还是 dark，以及滚动条/阴影强度）。
    pub light: bool,
}

/// 按少量锚点色派生一整套语义色，避免为每个主题手写 24 个 hex。
struct Ink {
    page: Color32,
    sidebar: Color32,
    panel: Color32,
    panel_hover: Color32,
    border: Color32,
    text: Color32,
    text_secondary: Color32,
    muted: Color32,
    accent: Color32,
    on_primary: Color32,
    primary_btn: Color32,
    success: Color32,
    warning: Color32,
    danger: Color32,
    light: bool,
}

/// 线性混色：`t=0` 全 a，`t=1` 全 b。状态的淡底（*_bg）都靠它从主题自己的颜色推出来。
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| -> u8 {
        let v = (x as f32) * (1.0 - t) + (y as f32) * t;
        v.round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn darken(c: Color32, t: f32) -> Color32 {
    mix(c, Color32::from_rgb(0, 0, 0), t)
}

fn lighten(c: Color32, t: f32) -> Color32 {
    mix(c, Color32::from_rgb(255, 255, 255), t)
}

/// WCAG 2.x 相对亮度。
fn luminance(c: Color32) -> f32 {
    let chan = |v: u8| {
        let s = v as f32 / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * chan(c.r()) + 0.7152 * chan(c.g()) + 0.0722 * chan(c.b())
}

/// WCAG 对比度比值（1.0 = 完全同色，21.0 = 黑/白）。
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// 把 `base` 逐步压深，直到它压在 `bg` 上能到 `target`:1（WCAG AA 正文要 4.5）。
///
/// 为什么不手挑：亮色当按钮底色时白字只有 2.9:1 左右（Raycast 红实测），手挑"深一点的同色"
/// 每加一个强调色都要人再核一遍；这里直接算，加色只写一个 hex。对已经够深的颜色是幂等的。
fn readable_on(base: Color32, bg: Color32, target: f32) -> Color32 {
    let black = Color32::from_rgb(0, 0, 0);
    let mut t = 0.0_f32;
    let mut value = base;
    while contrast(value, bg) < target && t < 0.9 {
        t += 0.02;
        value = mix(base, black, t);
    }
    value
}

#[inline]
fn readable_on_white(accent: Color32) -> Color32 {
    readable_on(accent, Color32::from_rgb(255, 255, 255), 4.6)
}

/// 深色底上的"压深"必须反着走：往**白**里提，直到在 `bg` 上到 `target`:1。
/// [`readable_on`] 只会往黑里压 —— 那是为"亮强调色压在白底上"写的；拿它算夜间
/// 的强调文字，越算越看不见（实测钴蓝压在夜间淡底上只有 3.15:1）。
fn readable_on_dark(base: Color32, bg: Color32, target: f32) -> Color32 {
    let white = Color32::from_rgb(255, 255, 255);
    let mut t = 0.0_f32;
    let mut value = base;
    while contrast(value, bg) < target && t < 0.9 {
        t += 0.02;
        value = mix(base, white, t);
    }
    value
}

impl Palette {
    fn from_ink(ink: Ink) -> Self {
        let accent_hover = if ink.light {
            darken(ink.accent, 0.16)
        } else {
            lighten(ink.accent, 0.16)
        };
        let button = ink.primary_btn;
        let (soft_btn, soft_btn_hover, soft_btn_press) = if ink.light {
            (
                mix(ink.page, ink.accent, 0.20),
                mix(ink.page, ink.accent, 0.30),
                mix(ink.page, ink.accent, 0.38),
            )
        } else {
            (button, mix(button, ink.text, 0.14), darken(button, 0.18))
        };
        let soft_btn_text = if ink.light {
            readable_on(ink.accent, soft_btn, 4.6)
        } else {
            ink.on_primary
        };
        // 淡强调底（选中项、胶囊、提示块）先算出来，"压在它上面的强调色文字"要按
        // **这块底**去压深/提亮，不是按纯白 —— 按纯白算只有 3.49:1（浅色）/ 3.15:1（夜间）。
        let accent_bg = mix(ink.page, ink.accent, if ink.light { 0.12 } else { 0.2 });
        let accent_strong = if ink.light {
            readable_on(ink.accent, accent_bg, 4.6)
        } else {
            readable_on_dark(ink.accent, accent_bg, 4.6)
        };
        Self {
            page_bg: ink.page,
            sidebar: ink.sidebar,
            panel: ink.panel,
            panel_hover: ink.panel_hover,
            border: ink.border,
            border_subtle: mix(ink.border, ink.panel, 0.45),
            text_primary: ink.text,
            text_secondary: ink.text_secondary,
            muted: ink.muted,
            accent: ink.accent,
            accent_hover,
            accent_bg,
            accent_strong,
            // 浅色主题下主按钮不再用实心深蓝：淡蓝底 + 压深后的强调色字。
            // 文字必须按**这块淡蓝底**去压深，不是按纯白（按纯白算出来只有 3.49:1）。
            primary_btn: soft_btn,
            primary_btn_hover: soft_btn_hover,
            primary_btn_press: soft_btn_press,
            on_primary: soft_btn_text,
            success: ink.success,
            success_bg: mix(ink.page, ink.success, if ink.light { 0.12 } else { 0.18 }),
            warning: ink.warning,
            warning_bg: mix(ink.page, ink.warning, if ink.light { 0.14 } else { 0.2 }),
            danger: ink.danger,
            danger_bg: mix(ink.page, ink.danger, if ink.light { 0.12 } else { 0.2 }),
            danger_stroke: mix(ink.page, ink.danger, if ink.light { 0.55 } else { 0.42 }),
            sidebar_text: ink.text,
            sidebar_muted: ink.text_secondary,
            light: ink.light,
        }
    }
}

/// id + 界面显示名。顺序就是设置页里的顺序。
/// 一套主题 = id + 界面名 + 强调色。中性色（底 / 文字 / 状态）全家族共用一份，
/// 所以加一套配色就是加一行 hex：按钮底色、淡彩底、描边色都是从强调色派生的。
pub struct Theme {
    pub id: &'static str,
    pub name: &'static str,
    pub accent: u32,
}

/// 顺序就是设置页里的顺序。深色那几套（obsidian / latte / midnight / mocha / rosepine /
/// linear）2026-10-06 按用户要求全部删掉，只留"清爽浅白"这一个家族。
pub const THEMES: [Theme; 5] = [
    Theme {
        id: "soft-cobalt",
        name: "钴蓝（默认）",
        accent: 0x2F_6F_EB,
    },
    Theme {
        id: "soft-teal",
        name: "青绿",
        accent: 0x00_A7_8D,
    },
    Theme {
        id: "soft-terracotta",
        name: "陶土橙",
        accent: 0xD9_77_2E,
    },
    Theme {
        id: "soft-indigo",
        name: "靛蓝紫",
        accent: 0x5B_5B_D6,
    },
    Theme {
        id: "soft-violet",
        name: "紫罗兰",
        accent: 0x7C_5C_FF,
    },
];

pub const DEFAULT_THEME: &str = "soft-cobalt";

fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

pub fn theme_by_id(id: &str) -> &'static Theme {
    THEMES
        .iter()
        .find(|theme| theme.id == id)
        // 认不出的存值（旧主题 id、手改过的值）落到默认那套，而不是随便挑一套。
        .unwrap_or_else(|| &THEMES[0])
}

/// 浅色那套中性锚点：左侧菜单纯白，右侧内容页压深一档蓝灰，卡片再回到纯白 ——
/// 分层靠"页面比卡片深"这一条，不靠描边（用户 2026-10-06：卡片和周围背景快分不开了，
/// 菜单要更浅、内容要更深）。
fn light_ink(accent: Color32) -> Ink {
    Ink {
        page: rgb(0xE8_EE_F7),
        sidebar: rgb(0xFF_FF_FF),
        panel: rgb(0xFF_FF_FF),
        panel_hover: rgb(0xF1_F6_FD),
        border: rgb(0xE9_F0_FA),
        text: rgb(0x1A_20_28),
        text_secondary: rgb(0x3F_4A_57),
        muted: rgb(0x64_70_7E),
        accent,
        on_primary: rgb(0xFF_FF_FF),
        primary_btn: readable_on_white(accent),
        success: rgb(0x0E_6B_4C),
        // 用户 2026-10-06：页面上不要红色和黄色。警告/危险改成石墨蓝灰，
        // 严重度靠"字重 + 图标 + 措辞"表达，不靠颜色。绿色留给"实测正常/连通"。
        warning: rgb(0x6B_74_80),
        danger: rgb(0x2E_35_3D),
        light: true,
    }
}

/// 夜间锚点：现代化通透 Slate 调色板（通透轻盈、低压迫感、层次分明有呼吸感）
/// 页面采用柔和 Slate 底色 (#131722)，卡片采用轻微浮起的舒适深灰蓝 (#1E2433)，
/// hover (#283042)，边框采用柔化描边 (#323C50)，正文高对比冰白 (#F8FAFC)。
fn dark_ink(accent: Color32) -> Ink {
    Ink {
        page: rgb(0x13_17_22),
        sidebar: rgb(0x13_17_22),
        panel: rgb(0x1E_24_33),
        panel_hover: rgb(0x28_30_42),
        border: rgb(0x32_3C_50),
        text: rgb(0xF8_FA_FC),
        text_secondary: rgb(0xCB_D5_E1),
        muted: rgb(0x9E_AA_BA),
        accent,
        on_primary: rgb(0xFF_FF_FF),
        // 主按钮保持清晰可见
        primary_btn: readable_on_white(accent),
        success: rgb(0x35_D1_96),
        warning: rgb(0xB0_BA_C6),
        danger: rgb(0xDC_E2_EA),
        light: false,
    }
}

pub fn palette(id: &str) -> Palette {
    palette_for(id, false)
}

/// 同一支强调色 × 明/暗两套中性锚点 = 主题的全部组合。夜间是开关，不是第 6 套主题
/// （用户 2026-10-15 选的方案一：和现有 5 套强调色正交）。
pub fn palette_for(id: &str, dark: bool) -> Palette {
    let accent = rgb(theme_by_id(id).accent);
    Palette::from_ink(if dark {
        dark_ink(accent)
    } else {
        light_ink(accent)
    })
}

static CURRENT: Lazy<RwLock<Palette>> = Lazy::new(|| RwLock::new(palette(DEFAULT_THEME)));

/// 当前主题id（切换时读它，避免每帧解析字符串）。
static CURRENT_ID: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(DEFAULT_THEME.to_string()));

macro_rules! token_accessors {
    ($($field:ident => $name:ident),* $(,)?) => {
        $(
            #[inline]
            pub fn $name() -> Color32 {
                CURRENT.read().map(|p| p.$field).unwrap_or(Color32::GRAY)
            }
        )*
    };
}

token_accessors! {
    page_bg => page_bg,
    sidebar => sidebar,
    panel => panel,
    panel_hover => panel_hover,
    border => border,
    border_subtle => border_subtle,
    text_primary => text_primary,
    text_secondary => text_secondary,
    muted => muted,
    accent => accent,
    accent_hover => accent_hover,
    accent_bg => accent_bg,
    accent_strong => accent_strong,
    primary_btn => primary_btn,
    primary_btn_hover => primary_btn_hover,
    primary_btn_press => primary_btn_press,
    on_primary => on_primary,
    success => success,
    success_bg => success_bg,
    warning => warning,
    warning_bg => warning_bg,
    danger => danger,
    danger_bg => danger_bg,
    danger_stroke => danger_stroke,
    sidebar_text => sidebar_text,
    sidebar_muted => sidebar_muted,
}

// ---------------------------------------------------------------------------
// 结构（圆角 / 内边距 / 分层方式）。颜色是每套主题各一份，结构是全局一份：
// 之前圆角和边距散在 30 多处 `CornerRadius::same(8)` 里，改一次风格要满仓库找，
// 所以收在这儿。egui 0.31 的 CornerRadius 是 u8，画不出 CSS 那种 `999px` 药丸，
// 14 已经是"按钮高度的一半"，视觉上就是胶囊。
// ---------------------------------------------------------------------------

/// 卡片/弹框容器圆角。
pub const RADIUS_CARD: u8 = 20;
/// 导航项圆角。
pub const RADIUS_NAV: u8 = 14;
/// 按钮、标签页、状态胶囊圆角。
pub const RADIUS_BTN: u8 = 16;
/// 卡片内边距（`Margin` 在 egui 0.31 是 i8）。
pub const PAD_CARD: i8 = 22;
/// 导航项内边距（横向）。
pub const PAD_NAV_X: i8 = 13;

/// 卡片描边：浅色主题靠阴影分层，所以不要描边；深色主题阴影会糊成一团脏的，
/// 反过来留一条 1px 边。
#[inline]
pub fn card_stroke() -> Stroke {
    if is_light() {
        Stroke::NONE
    } else {
        Stroke::new(1.0_f32, border())
    }
}

/// 卡片投影，配 [`card_stroke`] 使用；深色主题返回 NONE。
#[inline]
pub fn card_shadow() -> eframe::egui::Shadow {
    if is_light() {
        eframe::egui::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(24, 44, 78, 16),
        }
    } else {
        eframe::egui::Shadow::NONE
    }
}

#[inline]
pub fn is_light() -> bool {
    CURRENT.read().map(|p| p.light).unwrap_or(false)
}

#[inline]
/// 装主题：写静态调色板 + 同步 egui 的 `Visuals`（面板底色、控件描边、选中色、超链接、
/// 间距/圆角这些一并跟着走，否则主题只改了自绘部分、原生控件还是老样子）。
/// `dark` = 夜间开关。它和 `id`（强调色）是两条正交的轴：5 支强调色 × 明/暗。
pub fn apply(ctx: &egui::Context, id: &str, dark: bool) {
    let resolved = theme_by_id(id).id.to_string();
    let pal = palette_for(&resolved, dark);
    if let Ok(mut guard) = CURRENT.write() {
        *guard = pal;
    }
    if let Ok(mut guard) = CURRENT_ID.write() {
        *guard = resolved.clone();
    }

    let mut style = egui::Style::default();
    style.visuals = if pal.light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let visuals = &mut style.visuals;
    visuals.panel_fill = pal.page_bg;
    visuals.window_fill = pal.panel;
    visuals.faint_bg_color = pal.border_subtle;
    visuals.extreme_bg_color = pal.sidebar;
    visuals.hyperlink_color = pal.accent;
    visuals.warn_fg_color = pal.warning;
    visuals.error_fg_color = pal.danger;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.border_subtle);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, pal.muted);
    visuals.widgets.inactive.bg_fill = pal.panel;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    visuals.widgets.hovered.bg_fill = pal.panel_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.widgets.active.bg_fill = pal.panel_hover;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent_hover);
    visuals.selection.bg_fill = pal.accent_bg;
    visuals.selection.stroke = Stroke::new(1.0_f32, pal.accent);
    visuals.window_stroke = Stroke::new(1.0_f32, pal.border);
    // 浅色主题靠投影分层（卡片无描边）；深色主题投影会糊成脏的一片，只留很淡的一层。
    let shadow = if pal.light {
        egui::epaint::Shadow {
            offset: [0, 6],
            blur: 22,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(24, 44, 78, 20),
        }
    } else {
        egui::epaint::Shadow {
            offset: [0, 2],
            blur: 14,
            spread: 0,
            color: Color32::from_rgba_unmultiplied(0, 0, 0, 120),
        }
    };
    visuals.popup_shadow = shadow;
    visuals.window_shadow = shadow;
    style.spacing.item_spacing = Vec2::new(10.0, 9.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_keeps_button_and_banner_text_readable() {
        // 守的是三个真 bug：主按钮文字写死成深藏青（深底看不见）、提示横幅正文写死成深色
        // （淡色块上看不见）、以及亮强调色直接当按钮底色（白字只有 2.9:1）。
        for theme in THEMES {
            let p = palette(theme.id);
            assert!(
                contrast(p.on_primary, p.primary_btn) >= 4.5,
                "{} 主按钮文字/底色对比度 = {:.2}:1（要 >= 4.5:1）",
                theme.name,
                contrast(p.on_primary, p.primary_btn)
            );
            assert!(
                p.danger.r() <= 90 && p.danger.g() <= 110 && p.warning.r() <= 130,
                "{} 又混进红/黄调了：danger={:?} warning={:?}",
                theme.name,
                p.danger,
                p.warning
            );
            for (slot, bg) in [
                ("success_bg", p.success_bg),
                ("warning_bg", p.warning_bg),
                ("danger_bg", p.danger_bg),
            ] {
                assert!(
                    contrast(p.text_primary, bg) >= 3.0,
                    "{} 正文在 {slot} 上对比度 = {:.2}:1（要 >= 3:1）",
                    theme.name,
                    contrast(p.text_primary, bg)
                );
            }
            assert!(
                contrast(p.muted, p.panel) >= 2.0,
                "{} 辅助文字/图表轴标签在卡片底色上对比度 = {:.2}:1（要 >= 2:1）",
                theme.name,
                contrast(p.muted, p.panel)
            );
        }
    }

    #[test]
    fn kernel_dialog_rows_stay_readable_in_every_theme() {
        // 「下载内核」弹框的四种方式是自绘整行：未选中用页面底，选中用 accent_bg 底
        // + accent_strong 标题。这几组配对没走 egui 的 visuals，改调色板时会悄悄糊掉。
        let mut failures = Vec::new();
        for theme in THEMES {
            let p = palette(theme.id);
            for (label, ratio) in [
                ("标题/未选中行底", contrast(p.text_primary, p.page_bg)),
                ("提示/未选中行底", contrast(p.text_secondary, p.page_bg)),
                ("选中行标题/选中底", contrast(p.accent_strong, p.accent_bg)),
                ("选中行提示/选中底", contrast(p.text_secondary, p.accent_bg)),
                ("正文/窗口底", contrast(p.text_primary, p.panel)),
                ("提示/窗口底", contrast(p.text_secondary, p.panel)),
                ("成功状态/窗口底", contrast(p.success, p.panel)),
                ("警告状态/窗口底", contrast(p.warning, p.panel)),
            ] {
                if ratio < 3.0 {
                    failures.push(format!("{} {label} = {:.2}:1", theme.name, ratio));
                }
            }
            // 卡片没有描边，只靠"页面底比卡片深一档"+阴影分层；差值小到看不出就整片糊成白
            // （2026-10-06 就是 1.047 被嫌"卡片和背景快分不开"，地板抬到 1.12）。
            let layer = contrast(p.panel, p.page_bg);
            if layer < 1.12 {
                failures.push(format!(
                    "{} 卡片/页面分层只有 {layer:.3}:1（要 >= 1.12）",
                    theme.name
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "配色对比度不足：
{}",
            failures.join(
                "
"
            )
        );
    }

    #[test]
    fn every_theme_shares_the_neutral_base_and_differs_only_by_accent() {
        // 结构统一是这套主题的立身之本：谁要是顺手改了中性色，五个变体就不再"只差一个色"。
        let base = palette(THEMES[0].id);
        for theme in &THEMES[1..] {
            let p = palette(theme.id);
            assert_eq!(p.page_bg, base.page_bg, "{} 页面底色被改", theme.name);
            assert_eq!(p.panel, base.panel, "{} 卡片底色被改", theme.name);
            assert_eq!(
                p.text_primary, base.text_primary,
                "{} 正文色被改",
                theme.name
            );
            assert_eq!(p.success, base.success, "{} 成功色被改", theme.name);
            assert_ne!(p.accent, base.accent, "{} 强调色和默认那套撞了", theme.name);
        }
        // 五个强调色两两之间也得看得出差别，否则等于白加一个选项。
        // 现集合里最近的一对是 靛蓝紫↔紫罗兰 = 75、靛蓝紫钴蓝 = 85，所以门槛放 60
        // （挡的是"顺手加了一行其实和已有那行差不多"，不是精确的色彩学距离）。
        for (index, left) in THEMES.iter().enumerate() {
            for other in THEMES[index + 1..].iter() {
                let a = palette(left.id).accent;
                let b = palette(other.id).accent;
                let delta = (a.r() as i32 - b.r() as i32).abs()
                    + (a.g() as i32 - b.g() as i32).abs()
                    + (a.b() as i32 - b.b() as i32).abs();
                assert!(
                    delta > 60,
                    "{} 与 {} 的强调色几乎分不出来: {delta}",
                    left.name,
                    other.name
                );
            }
        }
    }

    #[test]
    fn default_theme_is_the_first_entry_and_is_light() {
        assert_eq!(DEFAULT_THEME, THEMES[0].id);
        assert!(palette(DEFAULT_THEME).light, "默认主题应当是浅色");
    }

    #[test]
    fn models_default_theme_matches() {
        // models 里那份是字面量（不能反向依赖 gui），所以两边会漂；这条守着。
        assert_eq!(
            crate::models::AppSettings::default().ui_theme,
            DEFAULT_THEME
        );
    }

    #[test]
    fn unknown_theme_id_falls_back_to_default() {
        // 旧 id（obsidian / latte / …）删掉之后，存量设置必须落到默认那套，不能随机挑一套。
        for legacy in [
            "obsidian", "latte", "midnight", "mocha", "rosepine", "linear", "",
        ] {
            assert_eq!(
                palette(legacy).accent,
                palette(DEFAULT_THEME).accent,
                "{legacy} 没有退回默认主题"
            );
            assert_eq!(
                theme_by_id(legacy).id,
                DEFAULT_THEME,
                "{legacy} 没有退回默认主题"
            );
        }
    }
}
#[cfg(test)]
mod dark_mode_tests {
    use super::*;

    /// 低彩度判据：红/黄这类有彩度的颜色 max-min 通道差很大，石墨灰很小。
    /// 浅色那版断言用的是通道上限（`r <= 90` 之类），那套上限在深色锚点上会把
    /// 正常的浅石墨误判成"混进红/黄"，所以夜间这边换成与亮度无关的彩度。
    fn chroma(c: Color32) -> u8 {
        let max = c.r().max(c.g()).max(c.b());
        let min = c.r().min(c.g()).min(c.b());
        max - min
    }

    /// 夜间每一格文字都要读得出来 —— 包括那支"压在淡强调底上的强调色文字"，
    /// 它是这次新加的 `readable_on_dark`（往白里提）算出来的，用旧的往黑里压会得到 3.15:1。
    #[test]
    fn every_dark_text_slot_stays_readable() {
        for spec in THEMES {
            let p = palette_for(spec.id, true);
            assert!(!p.light, "{} 夜间应该走深色分支", spec.name);
            for (slot, fg, bg, floor) in [
                ("正文/页面", p.text_primary, p.page_bg, 7.0),
                ("正文/卡片", p.text_primary, p.panel, 7.0),
                ("次要/卡片", p.text_secondary, p.panel, 4.5),
                ("muted/卡片", p.muted, p.panel, 4.5),
                ("侧栏灰字", p.sidebar_muted, p.sidebar, 4.5),
                ("强调字/淡强调底", p.accent_strong, p.accent_bg, 4.5),
                ("主按钮字/按钮底", p.on_primary, p.primary_btn, 4.5),
                ("成功/卡片", p.success, p.panel, 4.5),
                ("警告/卡片", p.warning, p.panel, 4.5),
                ("危险/卡片", p.danger, p.panel, 4.5),
            ] {
                let got = contrast(fg, bg);
                assert!(
                    got >= floor,
                    "{} 夜间 {slot} = {got:.2}:1（要 >= {floor}）",
                    spec.name
                );
            }
        }
    }

    /// 分层方式照参考图：页面和侧栏同为纯黑（不分层），卡片浮起来靠"比页面亮一档"。
    /// 门槛 1.15 不是拍的：参考图上卡片 #1A1A1A 压在纯黑上实测就是 **1.21:1**，
    /// 深色下靠亮度分层本来就只要一点点差，再大就要往灰里走、失去"近黑"那口气。
    #[test]
    fn dark_layers_by_lightening_the_card_not_by_tinting_it() {
        for spec in THEMES {
            let p = palette_for(spec.id, true);
            assert!(
                contrast(p.sidebar, p.page_bg) < 1.02,
                "{} 夜间侧栏和页面应该同色，实测 {:.2}:1",
                spec.name,
                contrast(p.sidebar, p.page_bg)
            );
            assert!(
                contrast(p.panel, p.page_bg) >= 1.15,
                "{} 夜间卡片没浮起来：{:.2}:1",
                spec.name,
                contrast(p.panel, p.page_bg)
            );
            // 深色主题必须留描边（阴影在深底上会糊脏，见 card_stroke）。
            assert!(
                p.border_subtle != p.panel,
                "{} 夜间连 border 都和卡片同色，分层就只剩阴影了",
                spec.name
            );
        }
    }

    /// "页面上不要红/黄"这条在夜间仍然守着；反向对照用同一把尺子量真红，
    /// 证明这个判据不是永远为真。
    #[test]
    fn dark_state_colors_stay_graphite() {
        for spec in THEMES {
            let p = palette_for(spec.id, true);
            assert!(
                chroma(p.danger) <= 30 && chroma(p.warning) <= 30,
                "{} 夜间混进了有彩度的状态色：danger={:?} warning={:?}",
                spec.name,
                p.danger,
                p.warning
            );
        }
        assert!(
            chroma(Color32::from_rgb(0xB3, 0x2A, 0x2A)) > 100,
            "对照组失效：这把彩度尺子量不出真红，那上面那条断言就是白写的"
        );
    }

    /// 接线检查：`apply(ctx, id, dark)` 那个参数真的传到底了 —— egui 自己的
    /// `Visuals::panel_fill` 和静态调色板必须一起翻，否则自绘的部分是黑的、原生控件
    /// 还是浅的（半深半浅就是这条最容易漏）。
    #[test]
    fn the_dark_switch_reaches_egui_visuals_too() {
        let ctx = egui::Context::default();
        apply(&ctx, DEFAULT_THEME, true);
        assert!(!is_light(), "装了夜间，is_light() 还说自己是浅色");
        assert_eq!(
            ctx.style().visuals.panel_fill,
            Color32::from_rgb(0x13, 0x17, 0x22),
            "egui 的面板底色没跟着夜间走"
        );
        assert_eq!(
            panel(),
            Color32::from_rgb(0x1E, 0x24, 0x33),
            "卡片色没跟着走"
        );
        assert_eq!(
            ctx.style().visuals.widgets.inactive.bg_fill,
            Color32::from_rgb(0x1E, 0x24, 0x33),
            "原生控件的底色没跟着走"
        );

        apply(&ctx, DEFAULT_THEME, false);
        assert!(is_light(), "关掉夜间之后没回到浅白");
        assert_eq!(
            ctx.style().visuals.panel_fill,
            Color32::from_rgb(0xE8, 0xEE, 0xF7),
            "回到浅色时面板底色没换回来"
        );
    }

    /// 夜间开关不许动强调色 —— 它是和 5 套主题正交的一条轴，不是第 6 套主题。
    #[test]
    fn the_dark_switch_leaves_the_accent_alone() {
        for spec in THEMES {
            let light = palette_for(spec.id, false);
            let dark = palette_for(spec.id, true);
            assert_eq!(light.accent, dark.accent, "{} 夜间改了强调色", spec.name);
            assert_ne!(light.page_bg, dark.page_bg, "{} 夜间没换底色", spec.name);
            assert_ne!(
                light.text_primary, dark.text_primary,
                "{} 夜间没换正文色",
                spec.name
            );
        }
    }
}

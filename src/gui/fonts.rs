//! CJK 字体发现与墨迹垂直校准模块
//!
//! 提供跨平台（Windows / macOS / Linux）中日韩字体的自动探测、
//! 排版垂直基线校准（解决 egui 中文偏上贴顶问题）以及装配加载逻辑。

use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

/// 文件名像不像一个中日韩字体。macOS/Linux 上要靠它从字体目录里挑；Windows 用固定文件名。
#[allow(dead_code)]
pub fn looks_like_cjk_font(file_name_lower: &str) -> bool {
    [
        "pingfang",
        "hiragino",
        "stheiti",
        "songti",
        "kaiti",
        "yuanti",
        "arial unicode",
        "heiti",
        "applegothic",
        "applesdgothicneo",
        "msyh",
        "simhei",
        "simsun",
        "msjh",
        "notosanscjk",
        "notosanssc",
        "notoserifcjk",
        "sourcehansans",
        "sourcehanserif",
        "wqy-microhei",
        "wqy-zenhei",
        "uming",
        "ukai",
        "droidsansfallback",
        "fireflysung",
    ]
    .iter()
    .any(|needle| file_name_lower.contains(needle))
}

/// 在字体目录里递归找中日韩字体。macOS/Linux 的字体装在随发行版变化的子目录下，固定路径
/// 猜不中就走这里；深度封顶，不跟着符号链接往下转。
#[allow(dead_code)]
pub fn scan_cjk_fonts(dirs: &[PathBuf], depth: u8) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, u8)> = dirs.iter().cloned().map(|d| (d, 0)).collect();
    while let Some((current, level)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue; // 目录不存在或读不了：跳过，这不是错误
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if level + 1 <= depth {
                    stack.push((path, level + 1));
                }
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let font_file =
                name.ends_with(".ttf") || name.ends_with(".ttc") || name.ends_with(".otf");
            if font_file && looks_like_cjk_font(&name) {
                found.push(path);
            }
        }
    }
    found
}

/// 本机可用的中日韩字体候选，按“先用哪个”排序。
pub fn cjk_font_candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        [
            PathBuf::from(r"C:\Windows\Fonts\msyh.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\msyhbd.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\simhei.ttf"),
            PathBuf::from(r"C:\Windows\Fonts\simsun.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\msjh.ttc"),
            PathBuf::from(r"C:\Windows\Fonts\NotoSansSC-Regular.otf"),
        ]
        .to_vec()
    }
    #[cfg(target_os = "macos")]
    {
        let fixed = [
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/Supplemental/Hiragino Sans GB.ttc",
            "/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Kaiti.ttc",
            "/System/Library/Fonts/Supplemental/Yuanti.ttc",
            "/Library/Fonts/Arial Unicode.ttf",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        ];
        let mut roots = vec![
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/System/Library/Fonts/Supplemental"),
            PathBuf::from("/Library/Fonts"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(std::path::Path::new(&home).join("Library/Fonts"));
        }
        let mut out: Vec<_> = fixed.iter().map(PathBuf::from).collect();
        out.extend(scan_cjk_fonts(&roots, 2));
        out
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let fixed = [
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        ];
        let mut roots = vec![
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(std::path::Path::new(&home).join(".local/share/fonts"));
        }
        let mut out: Vec<_> = fixed.iter().map(PathBuf::from).collect();
        out.extend(scan_cjk_fonts(&roots, 4));
        out
    }
}

/// 启动时校准墨迹垂直位置用的参照串与字号（单位 point）。
pub const INK_CALIBRATION_LABEL: &str = "启动内核";
pub const INK_CALIBRATION_SIZE: f32 = 14.0;
/// 偏不到这个数就不动它：实测 Windows 的雅黑 14pt 正好 0.00pt，不该被改。
pub const INK_MIN_OFFSET: f32 = 1.0;
/// 封顶，再偏也只补这么多（em 的比例）。
/// 依据：mac CI 那台机器实测冬青黑（Hiragino Sans GB）14pt 原始偏 -3.50pt ⇒ 需要 0.25，
/// 这个帽留了一档余量，同时挡住"字体指标病态到把字挤出按钮框"的情况。
pub const INK_MAX_OFFSET_FACTOR: f32 = 0.30;

/// 由「实测偏移」算出该给这个字体补的 `y_offset_factor`（正 = 往下挪）。
/// 拆成纯函数是为了让测试能拿同一份规则复算，而不是在产品路径旁边再抄一遍。
pub fn ink_calibration_factor(raw_offset: f32) -> f32 {
    if raw_offset.abs() < INK_MIN_OFFSET {
        return 0.0;
    }
    (-raw_offset / INK_CALIBRATION_SIZE).clamp(-INK_MAX_OFFSET_FACTOR, INK_MAX_OFFSET_FACTOR)
}

/// 墨迹中心相对布局盒中心的偏移，单位 point；负 = 字偏上。
///
/// 按钮画的是「布局盒居中 + 上下等距 padding」，所以这个差就是他肉眼看到的偏差。
/// 中日韩字体的 ascent/descent 普遍不对称（同一份 14pt 文本，行高盒比实际字形高出一截，
/// 多出来的量几乎全落在基线以下），egui 按行高居中 → 字看着贴顶。
/// 只在启动装载字体时跑一次，不进每帧。
pub fn ink_offset(font_name: &str, data: &Arc<egui::FontData>) -> f32 {
    let ctx = egui::Context::default();
    let mut definitions = egui::FontDefinitions::empty();
    definitions
        .font_data
        .insert(font_name.to_owned(), data.clone());
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.to_owned());
    }
    ctx.set_fonts(definitions);
    let mut input = egui::RawInput::default();
    input.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(400.0, 200.0),
    ));
    let text = INK_CALIBRATION_LABEL.to_owned();
    let mut offset = 0.0_f32;
    let mut out = ctx.run_ui(input, |ui| {
        let galley = ui.painter().layout_no_wrap(
            text.clone(),
            egui::FontId::proportional(INK_CALIBRATION_SIZE),
            egui::Color32::WHITE,
        );
        offset = galley.mesh_bounds.center().y - galley.rect.center().y;
    });
    out.textures_delta.clear();
    offset
}

/// 加载中文字体。没有它整屏中文都是方框 —— egui 自带字体不含 CJK 字形。
pub fn setup_chinese_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut loaded_names = Vec::new();

    for path in cjk_font_candidates() {
        if let Ok(bytes) = std::fs::read(&path) {
            let font_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("cjk_{}", loaded_names.len()));

            info!(path = %path.display(), font = %font_name, "loaded CJK font for GUI");

            let data = Arc::new(egui::FontData::from_owned(bytes));
            // Windows 下微软雅黑/宋体/黑体字形垂直居中极为标准（实测 0.00pt 偏差）。
            // ink_offset 会新建独立 egui::Context 进行字形栅格化排版，Windows 下跳过可规避额外分配数十兆显存/内存峰值。
            let raw_offset = if cfg!(windows) {
                0.0
            } else {
                ink_offset(&font_name, &data)
            };
            let factor = ink_calibration_factor(raw_offset);
            let font_data = if factor == 0.0 {
                data
            } else {
                let mut tweaked = Arc::unwrap_or_clone(data);
                tweaked.tweak.y_offset_factor = factor;
                Arc::new(tweaked)
            };
            info!(
                font = %font_name,
                ink_offset_pt = raw_offset,
                y_offset_factor = factor,
                "calibrated CJK font vertical centering"
            );

            fonts.font_data.insert(font_name.clone(), font_data);
            loaded_names.push(font_name);

            // 仅加载 1 个最优先的可用 CJK 字体。常规中文字体（如微软雅黑、苹方、文泉驿、Noto）
            // 单个字库就包含完备的简体/繁体/日韩字形，避免同时载入多个动辄十数兆的字体文件
            // 造成堆内存与纹理缓存膨胀（每个字体保留整份原始字节与字形表）。
            if loaded_names.len() >= 1 {
                break;
            }
        }
    }

    if loaded_names.is_empty() {
        tracing::warn!("no CJK font found; Chinese text may not render");
        return;
    }

    // 将加载到的 CJK 字体按顺序插入 proportional 与 monospace 字体族首部
    for (i, name) in loaded_names.iter().enumerate() {
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(i, name.clone());
        }
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.insert(i, name.clone());
        }
    }

    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod cjk_font_tests {
    use super::{looks_like_cjk_font, scan_cjk_fonts};
    use std::path::PathBuf;

    /// 判据要认得出各发行版真实的文件名，同时**不许**把这台机器上确实存在的
    /// 拉丁字体（Ubuntu / DejaVu）当成中文 —— 认错了就等于加载一个画不出汉字的字体，
    /// 现象还是方框，但日志会说"已加载中文字体"，比现在更难查。
    #[test]
    fn the_cjk_name_test_accepts_real_packages_and_rejects_latin_fonts() {
        for name in [
            "pingfang.ttc",
            "hiraginosansgb.ttc",
            "stheiti-light.ttc",
            "songti.ttc",
            "notosanscjk-regular.ttc",
            "notosanscjksc-regular.otf",
            "wqy-microhei.ttc",
            "wqy-zenhei.ttc",
            "sourcehansanssc-regular.otf",
            "msyh.ttc",
            "simhei.ttf",
            "droidsansfallback.ttf",
        ] {
            assert!(looks_like_cjk_font(name), "{name} 该被认成中文字体");
        }
        for name in [
            "dejavusans.ttf",
            "dejavusansmono.ttf",
            "ubuntu[wdth,wght].ttf",
            "ubuntumono[wght].ttf",
            "liberationMono-Regular.ttf",
            "",
        ] {
            assert!(!looks_like_cjk_font(name), "{name} 不该被认成中文字体");
        }
    }

    /// 扫描要能钻进发行版那些不一样的子目录（opentype/noto、truetype/wqy），
    /// 并且只收字体后缀：同目录下的 .txt、以及不相关的 .ttf 都不能进来。
    #[test]
    fn the_font_scan_finds_nested_packages_and_skips_other_files() {
        let root = std::env::temp_dir().join(format!("vpncjk-{}", std::process::id()));
        let nested = root.join("opentype/noto");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("NotoSansCJK-Regular.ttc"), b"fake").unwrap();
        let other = root.join("truetype/dejavu");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("DejaVuSans.ttf"), b"fake").unwrap();
        std::fs::write(other.join("README.txt"), b"fake").unwrap();

        let found = scan_cjk_fonts(&[root.clone()], 4);
        assert_eq!(found.len(), 1, "只该找到那个 CJK 字体，实际 {found:?}");
        assert!(found[0]
            .to_string_lossy()
            .ends_with("NotoSansCJK-Regular.ttc"));

        // 反向对照：深度给 0 时，第二层目录不该被打开。
        let shallow = scan_cjk_fonts(&[PathBuf::from(&root)], 0);
        assert!(shallow.is_empty(), "深度 0 还找到了 {shallow:?}");

        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod font_center_probe {
    use super::*;

    /// 当前平台第一个能被读到的 CJK 字体（`setup_chinese_fonts` 只装这一个）。
    fn active_font() -> String {
        cjk_font_candidates()
            .iter()
            .find(|path| path.is_file())
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .unwrap_or_else(|| "<没有 CJK 字体>".to_string())
    }

    /// 逐台机器上真实存在的每个候选字体跑一遍「量 → 补 → 再量」。
    /// 这条断言在 mac CI 上量的就是那台机器上的真字体，不是我抄的表。
    /// 一个 CJK 字体都读不到时明说跳过（ubuntu CI 镜像就这样），不把"没字体"当成校准失败。
    #[test]
    fn every_available_cjk_font_ends_up_vertically_centered() {
        let mut report = Vec::new();
        let mut over = Vec::new();
        let mut checked = 0_usize;
        for path in cjk_font_candidates() {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let data = Arc::new(egui::FontData::from_owned(bytes));
            let started = std::time::Instant::now();
            let raw = ink_offset(&name, &data);
            let factor = ink_calibration_factor(raw);
            let mut tweaked = Arc::unwrap_or_clone(data);
            tweaked.tweak.y_offset_factor = factor;
            let residual = ink_offset(&name, &Arc::new(tweaked));
            let cost_ms = started.elapsed().as_secs_f64() * 1000.0;
            checked += 1;
            report.push(format!(
                "{name}: 原始{raw:+.2}pt 补{factor:+.3} 残差{residual:+.2}pt 量两次{cost_ms:.1}ms"
            ));
            if residual.abs() > 0.75 {
                over.push(format!("{name} 残差 {residual:+.2}pt"));
            }
        }
        // 这台机器上一个 CJK 字体都读不到 —— ubuntu CI 镜像就是这种情况（顺带说明：
        // 那种环境下界面里的中文只能是方框，产品路径本来也没字体可校准）。
        // 这里明说跳过，而不是把"没字体"当成校准失败。
        if checked == 0 {
            println!(
                "FONT_PROBE SKIPPED：候选路径上一个字体都没读到（{:#?}）",
                active_font()
            );
            return;
        }
        println!(
            "FONT_ACTIVE={} 校准了 {checked} 个字体：{:#?}",
            active_font(),
            report
        );
        assert!(
            over.is_empty(),
            "这些字体校准后仍然偏：{over:#?}\n全部：{report:#?}"
        );
    }

    /// 反向对照：门槛以下不许动它 —— Windows 的雅黑本来就正中，不能被"修"出新偏差。
    #[test]
    fn tiny_offsets_are_left_alone() {
        assert_eq!(ink_calibration_factor(0.0), 0.0);
        assert_eq!(ink_calibration_factor(-0.9), 0.0);
        assert_eq!(ink_calibration_factor(0.9), 0.0);
        // 负 = 偏上 ⇒ 要往下补（正数），别把符号搞反
        assert!(ink_calibration_factor(-2.8) > 0.0);
        assert!(ink_calibration_factor(2.8) < 0.0);
        // 封顶
        assert!(ink_calibration_factor(-90.0).abs() <= INK_MAX_OFFSET_FACTOR + f32::EPSILON);
    }
}

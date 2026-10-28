//! 仅诊断用的实例壳：`VPN_PARSER_DIAG=1` 时，这个实例**不碰任何共享状态** ——
//! 不抢单实例互斥量、不杀任何进程、不注册托盘图标、不写系统代理/终端代理、
//! 数据目录和日志都落在临时目录、窗口摆在屏幕外也不抢焦点。
//!
//! 为什么要它：改完界面本来就该有人真的把界面打开点一遍。之前我只能靠用户手点，
//! 因为我自己起一个实例会把他正在跑的那个杀掉（`kill_duplicate_vpn_parser_instances`）、
//! 还会在他的托盘上多一个图标、把他的系统代理改掉、共用他 D 盘的库。有了这个壳，
//! "界面可见 + 真点击 + CPU/线程数实测"这一层验收我能自己做，做完就关掉。
//!
//! 没设这个环境变量时 [`enabled`] 永远是 `false`，产品路径一个字都不变。

use std::path::PathBuf;
use std::sync::OnceLock;

const FLAG: &str = "VPN_PARSER_DIAG";

/// 判定口径单独抽出来，好让"什么算开了"这件事有测试。只认 `1` / `true`（不区分大小写）。
pub fn enabled_from_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1") | Some("true")
    )
}

/// 进程内只读一次：环境变量在运行中不会变，而且这个判断散在启动路径各处，
/// 读一次才能保证"半诊断"状态不可能出现。
pub fn enabled() -> bool {
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| enabled_from_value(std::env::var(FLAG).ok().as_deref()))
}

/// 诊断实例的工作目录：`%TEMP%\vpn-parser-diag-<pid>`。启动时把 cwd 切到这里，
/// `--db` 的默认相对路径、`startup.log`、`panic.log`、`data/kernels` 就全都隔离了
/// —— 一个改动都不用碰 D 盘那份真实数据。
pub fn home() -> PathBuf {
    std::env::temp_dir().join(format!("vpn-parser-diag-{}", std::process::id()))
}

pub fn prepare_home() -> std::io::Result<PathBuf> {
    let home = home();
    std::fs::create_dir_all(&home)?;
    Ok(home)
}

/// 诊断壳专用：`VPN_PARSER_DIAG_TAB=settings`（或 `proxy` + `VPN_PARSER_DIAG_PANEL=rules`）
/// 让实例一起来就停在要验收的那一页。诊断窗口停在屏幕外、点不到，没有这个就只能看默认页。
pub fn starting_tab() -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var("VPN_PARSER_DIAG_TAB")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

/// 诊断壳专用：`VPN_PARSER_DIAG_SCALE=1.1` 直接指定界面缩放。
/// 字体缩放和"窗口逻辑宽度"是一对耦合量，很多问题只在 zoom≠1 时露出来。
pub fn starting_scale() -> Option<f32> {
    if !enabled() {
        return None;
    }
    std::env::var("VPN_PARSER_DIAG_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
}

/// 诊断壳专用：`VPN_PARSER_DIAG_DARK=1` 让这个实例以夜间启动，用来验深色那套锚点在
/// 真界面上跑不跑得起来（像素仍然看不见，但 `apply(ctx, id, dark)` 接错线会当场露出来）。
pub fn starting_dark() -> Option<bool> {
    if !enabled() {
        return None;
    }
    std::env::var("VPN_PARSER_DIAG_DARK")
        .ok()
        .map(|value| enabled_from_value(Some(&value)))
}

/// 诊断壳专用：`VPN_PARSER_DIAG_VERSION=1.3.0` 让这个实例自报是老版本，
/// 用来验证"发现新版 → 侧栏亮绿色↑ → 点它去关于页"整条链路。
pub fn forced_version() -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var("VPN_PARSER_DIAG_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 诊断壳专用：要验收的子面板（当前只有代理页的 `rules`）。
pub fn starting_panel() -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var("VPN_PARSER_DIAG_PANEL")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

/// 诊断壳专用：要打开的抽屉（`kernels`, `network`, `logs`, `rules`, `groups`, `connections`）。
pub fn starting_drawer() -> Option<String> {
    if !enabled() {
        return None;
    }
    std::env::var("VPN_PARSER_DIAG_DRAWER")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
}

/// 只在内容变化时写日志：几何量具每帧都算，全写进去就把 startup.log 冲满了。
pub fn log_once(line: &str) {
    use std::sync::{Mutex, OnceLock};
    static LAST: OnceLock<Mutex<String>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(String::new()));
    let mut guard = match last.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *guard != line {
        *guard = line.to_string();
        crate::startup_log(line);
    }
}

/// 给拒绝写系统设置那条路用的统一文案：出错时界面/日志都要能看出是"诊断壳拦的"，
/// 不是"功能坏了"。
pub const BLOCKED: &str = "诊断实例（VPN_PARSER_DIAG）不写系统代理与终端代理设置";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_and_true_turn_the_shell_on() {
        for on in ["1", "true", "TRUE", " 1 "] {
            assert!(enabled_from_value(Some(on)), "{on}");
        }
        for off in [None, Some(""), Some("0"), Some("yes"), Some("false")] {
            assert!(!enabled_from_value(off), "{off:?}");
        }
    }

    #[test]
    fn home_is_per_process_so_two_instances_do_not_share_state() {
        let path = home();
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("vpn-parser-diag-"));
        assert!(path
            .to_string_lossy()
            .contains(&std::process::id().to_string()));
    }
}

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
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("vpn-parser-diag-"));
        assert!(path.to_string_lossy().contains(&std::process::id().to_string()));
    }
}

//! VProxy: A modern cross-platform proxy workstation for collecting subscriptions,
//! parsing nodes, and managing proxy cores (Mihomo, Sing-box, VP-Core, Meow, Xray, Shoes).

pub mod api;
pub mod app_update;
pub mod collector;
pub mod constants;
pub mod db;
pub mod diag;
pub mod export;
pub mod export_quality;
pub mod ffi;
pub mod geo;
pub mod hostutil;
pub mod i18n;
pub mod ip_risk;
pub mod kernel;
pub mod models;
pub mod parser;
pub mod platform;
pub mod profile;
pub mod proxy;
pub mod proxy_probe;
pub mod router;
pub mod scheduler;
pub mod speedtest;
pub mod system_proxy;
pub mod tasks;

#[cfg(feature = "gui")]
pub mod gui;

// 常用全局诊断与日志导出
pub use diag::startup_log;

/// 这个实例的数据根目录：DB、分片库、内核都挂在它下面。
///
/// 三条规则按重要性排：
/// ① 诊断壳永远跟着当前工作目录 —— 它已经切进自己的临时隔离目录，落到用户数据里
///    就等于这个开关存在的意义没了（`VPROXY_DIAG` 的第一条承诺就是不动真数据）。
/// ② Windows 用工作目录下的 `data/`。装机目录就是程序目录（启动时会 chdir 过去），
///    从 1.0 到现在都是这个位置，一个字没改。
/// ③ Linux 用 XDG 数据目录。FHS 布局下可执行文件在 `/usr/bin`，而 `main` 会把工作目录
///    切到 exe 旁边 —— 相对路径就变成往 `/usr/bin/data` 写，普通用户必然失败，
///    而且失败发生在"窗口已经建好之后"，看起来像程序坏了。
pub fn data_root() -> std::path::PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let xdg = std::env::var_os("XDG_DATA_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let root = data_root_from(
        crate::diag::enabled(),
        &cwd,
        xdg.as_deref(),
        home.as_deref(),
    );
    #[cfg(not(windows))]
    let root = migrate_legacy_data_root(&root);
    let _ = std::fs::create_dir_all(&root);
    root
}

/// 纯函数版，判据在这里测得到（Windows 上也能测 Linux 那两行，否则只能等 Linux CI）。
pub fn data_root_from(
    diag: bool,
    cwd: &std::path::Path,
    xdg_data_home: Option<&std::path::Path>,
    home: Option<&std::path::Path>,
) -> std::path::PathBuf {
    if diag {
        return cwd.join("data");
    }
    #[cfg(windows)]
    {
        let _ = (xdg_data_home, home);
        cwd.join("data")
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        {
            if let Some(base) = home {
                return base.join("Library/Application Support/vproxy");
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(base) = xdg_data_home.filter(|p| p.is_absolute()) {
                return base.join("vproxy");
            }
            if let Some(base) = home {
                return base.join(".local/share/vproxy");
            }
        }
        cwd.join("data")
    }
}

/// 品牌改名（VPN Parser → VProxy）的一次性数据目录迁移：旧目录 `vpn-parser`
/// 与新目录同父、存在且新目录尚不存在时，整目录改名搬走。搬完旧路径不复存在，
/// 旧目录兼容自此彻底退场；失败（旧进程占用等）仅落档并沿用旧目录，下次启动重试。
#[cfg(not(windows))]
fn migrate_legacy_data_root(new_root: &std::path::Path) -> std::path::PathBuf {
    let Some(parent) = new_root.parent() else {
        return new_root.to_path_buf();
    };
    let old_root = parent.join("vpn-parser");
    if new_root.exists() || !old_root.exists() {
        return new_root.to_path_buf();
    }
    match std::fs::rename(&old_root, new_root) {
        Ok(()) => {
            crate::diag::startup_log(&format!(
                "[迁移] 旧品牌数据目录已一次性搬至 {}",
                new_root.display()
            ));
            new_root.to_path_buf()
        }
        Err(error) => {
            crate::diag::startup_log(&format!(
                "[迁移] 旧品牌数据目录 {} 搬至 {} 失败（本次继续用旧目录，下次启动重试）: {error}",
                old_root.display(),
                new_root.display()
            ));
            old_root
        }
    }
}

/// `--db` 不写时的默认库路径。
/// 品牌改名（VPN Parser → VProxy）的一次性迁移：旧库存在且新库尚不存在时，
/// 把旧库（连同 SQLite 的 `-wal`/`-shm` 伴生文件）改名搬移到新名；搬完旧文件名
/// 彻底退场。失败（旧进程占用等）本次沿用旧库并落档，下次启动重试。
pub fn default_db_path() -> std::path::PathBuf {
    let root = data_root();
    let new_db = root.join(constants::DEFAULT_DB_FILE);
    let old_db = root.join(constants::LEGACY_DB_FILE);
    if new_db.exists() || !old_db.exists() {
        return new_db;
    }
    match std::fs::rename(&old_db, &new_db) {
        Ok(()) => {
            for suffix in ["-wal", "-shm"] {
                let from = root.join(format!("{}{suffix}", constants::LEGACY_DB_FILE));
                if from.exists() {
                    let _ = std::fs::rename(
                        &from,
                        root.join(format!("{}{suffix}", constants::DEFAULT_DB_FILE)),
                    );
                }
            }
            crate::diag::startup_log(&format!(
                "[迁移] 旧品牌数据库已一次性更名为 {}",
                new_db.display()
            ));
            new_db
        }
        Err(error) => {
            crate::diag::startup_log(&format!(
                "[迁移] 旧品牌数据库 {} 更名为 {} 失败（本次继续用旧库，下次启动重试）: {error}",
                old_db.display(),
                new_db.display()
            ));
            old_db
        }
    }
}

#[cfg(test)]
mod data_root_tests {
    use super::data_root_from;
    use std::path::Path;

    /// 诊断壳必须永远落在工作目录里，哪怕这台机器配了 XDG。
    /// 反向对照：这条要是不成立，`VPN_PARSER_DIAG=1` 就会往真数据目录写东西。
    #[test]
    fn the_diag_instance_never_uses_the_user_data_dir() {
        let got = data_root_from(
            true,
            Path::new("/tmp/vproxy-diag-1"),
            Some(Path::new("/home/me/.local/share")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(got, Path::new("/tmp/vproxy-diag-1/data").to_path_buf());
    }

    /// XDG 优先，其次 HOME 下的常见位置，两个都没有才退回工作目录（而不是猜一个绝对路径）。
    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[test]
    fn the_linux_data_root_follows_xdg_then_home() {
        let got = data_root_from(
            false,
            Path::new("/usr/bin"),
            Some(Path::new("/data/x")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(got, Path::new("/data/x/vproxy").to_path_buf());

        let got = data_root_from(
            false,
            Path::new("/usr/bin"),
            None,
            Some(Path::new("/home/me")),
        );
        assert_eq!(
            got,
            Path::new("/home/me/.local/share/vproxy").to_path_buf()
        );

        // 相对值不当 XDG 配置：那样会拼出一个跟着工作目录跑的假绝对路径。
        let got = data_root_from(
            false,
            Path::new("/usr/bin"),
            Some(Path::new("relative")),
            Some(Path::new("/home/me")),
        );
        assert_eq!(
            got,
            Path::new("/home/me/.local/share/vproxy").to_path_buf()
        );

        let got = data_root_from(false, Path::new("/usr/bin"), None, None);
        assert_eq!(got, Path::new("/usr/bin/data").to_path_buf());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_macos_data_root_follows_application_support() {
        let got = data_root_from(
            false,
            Path::new("/Applications"),
            Some(Path::new("/data/x")),
            Some(Path::new("/Users/me")),
        );
        assert_eq!(
            got,
            Path::new("/Users/me/Library/Application Support/vproxy").to_path_buf()
        );
    }

    /// Windows 那份没变：工作目录下的 `data/`，和 1.0 以来的装机位置一致。
    #[cfg(windows)]
    #[test]
    fn the_windows_data_root_stays_next_to_the_program() {
        let got = data_root_from(false, Path::new(r"C:\Apps\VProxy"), None, None);
        assert_eq!(got, Path::new(r"C:\Apps\VProxy\data").to_path_buf());
    }
}

//! 终端代理环境变量（`HKCU\Environment`）。
//!
//! Windows 上只有走 WinINet/WinHTTP 的程序才读"系统代理"，Git Bash、curl、git、
//! npm、pip 这些只认 `HTTP_PROXY` / `HTTPS_PROXY` 环境变量 —— 这就是"系统代理开了，
//! 终端里 git clone 还是超时"的原因。这里只写用户级变量，不需要管理员。

use anyhow::{anyhow, Result};

pub const PROXY_VARS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];
pub const NO_PROXY_VAR: &str = "NO_PROXY";
/// 本地与内网一律不走代理，否则局域网打印/开发服务器会被绕进隧道。
pub const NO_PROXY_VALUE: &str = "localhost;127.0.0.1;::1;<local>;192.168.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*";

/// 我们自己写进去的值长这样，用来区分"用户原有的代理设置"。
fn is_ours(value: &str) -> bool {
    value.trim().starts_with("http://127.0.0.1:")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalProxyReport {
    pub written: Vec<String>,
    pub removed: Vec<String>,
    /// 用户自己设置的、我们没有动的变量。
    pub untouched: Vec<String>,
}

impl TerminalProxyReport {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.written.is_empty() {
            parts.push(format!("已写入 {}", self.written.join("/")));
        }
        if !self.removed.is_empty() {
            parts.push(format!("已清除 {}", self.removed.join("/")));
        }
        if !self.untouched.is_empty() {
            parts.push(format!("保留你原有的 {}", self.untouched.join("/")));
        }
        if parts.is_empty() {
            "终端代理变量无变化".into()
        } else {
            parts.join(" · ")
        }
    }
}

#[cfg(windows)]
fn read_env_value(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_EXPAND_SZ, REG_SZ,
    };

    let subkey_w: Vec<u16> = "Environment\0".encode_utf16().collect();
    let name_w: Vec<u16> = format!("{name}\0").encode_utf16().collect();

    unsafe {
        let mut hkey = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_w.as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut hkey,
        ) != 0
        {
            return None;
        }

        let mut val_type = 0u32;
        let mut byte_len = 0u32;
        let status = RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            std::ptr::null_mut(),
            &mut byte_len,
        );

        if status != 0 || (val_type != REG_SZ && val_type != REG_EXPAND_SZ) || byte_len == 0 {
            RegCloseKey(hkey);
            return None;
        }

        let mut buf = vec![0u8; byte_len as usize + 2];
        let mut actual_len = byte_len;
        let status = RegQueryValueExW(
            hkey,
            name_w.as_ptr(),
            std::ptr::null_mut(),
            &mut val_type,
            buf.as_mut_ptr(),
            &mut actual_len,
        );
        RegCloseKey(hkey);

        if status != 0 {
            return None;
        }

        let u16_slice =
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, (actual_len as usize) / 2);
        let end = u16_slice.iter().position(|&c| c == 0).unwrap_or(u16_slice.len());
        Some(String::from_utf16_lossy(&u16_slice[..end]))
    }
}

#[cfg(windows)]
fn write_env_value(name: &str, value: &str) -> Result<()> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    let subkey_w: Vec<u16> = "Environment\0".encode_utf16().collect();
    let name_w: Vec<u16> = format!("{name}\0").encode_utf16().collect();
    let val_w: Vec<u16> = format!("{value}\0").encode_utf16().collect();

    unsafe {
        let mut hkey = std::ptr::null_mut();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey_w.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        );
        if status != 0 {
            return Err(anyhow!("无法打开注册表 HKCU\\Environment (错误码: {status})"));
        }

        let write_status = RegSetValueExW(
            hkey,
            name_w.as_ptr(),
            0,
            REG_SZ,
            val_w.as_ptr() as *const u8,
            (val_w.len() * 2) as u32,
        );
        RegCloseKey(hkey);

        if write_status != 0 {
            return Err(anyhow!("写入 {name} 失败 (错误码: {write_status})"));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn delete_env_value(name: &str) -> Result<()> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };

    let subkey_w: Vec<u16> = "Environment\0".encode_utf16().collect();
    let name_w: Vec<u16> = format!("{name}\0").encode_utf16().collect();

    unsafe {
        let mut hkey = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_w.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        ) != 0
        {
            return Ok(());
        }

        let status = RegDeleteValueW(hkey, name_w.as_ptr());
        RegCloseKey(hkey);

        // 0 = ERROR_SUCCESS, 2 = ERROR_FILE_NOT_FOUND (未找到即无需删除，按成功处理)
        if status == 0 || status == 2 {
            Ok(())
        } else {
            Err(anyhow!("删除 {name} 失败 (错误码: {status})"))
        }
    }
}

/// 让已经开着的终端也能拿到新值（explorer 会转发 WM_SETTINGCHANGE）。
/// 广播 WM_SETTINGCHANGE("Environment")，让已经开着的终端也能拿到新值。
#[cfg(windows)]
fn broadcast_environment_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendNotifyMessageW, HWND_BROADCAST, WM_SETTINGCHANGE,
    };

    let param: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SendNotifyMessageW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            param.as_ptr() as isize,
        );
    }
}

#[cfg(windows)]
fn hidden_command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new(program);
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(windows)]
pub fn enable(port: u16) -> Result<TerminalProxyReport> {
    if crate::diag::enabled() {
        return Err(anyhow!(crate::diag::BLOCKED));
    }
    let server = format!("http://127.0.0.1:{port}");
    let mut report = TerminalProxyReport {
        written: Vec::new(),
        removed: Vec::new(),
        untouched: Vec::new(),
    };
    // 先整体检查：只要有一个变量是用户自己设的（不是我们的），就一个都不动。
    for name in PROXY_VARS {
        if let Some(existing) = read_env_value(name) {
            if !is_ours(&existing) {
                return Err(anyhow!(
                    "{name} 已经是你自己设置的值（{existing}），为避免覆盖终端代理配置，请先手动清掉再开启"
                ));
            }
        }
    }
    for name in PROXY_VARS {
        write_env_value(name, &server)?;
        report.written.push(name.to_owned());
    }
    match read_env_value(NO_PROXY_VAR) {
        Some(value) if !value.trim().is_empty() => report.untouched.push(NO_PROXY_VAR.to_owned()),
        _ => {
            write_env_value(NO_PROXY_VAR, NO_PROXY_VALUE)?;
            report.written.push(NO_PROXY_VAR.to_owned());
        }
    }
    broadcast_environment_change();

    // 异步同步 Git 全局代理，确保已有终端和各类 IDE 插件内的 Git 操作立即可用，且绝不卡顿 UI 线程
    let server_clone = server.clone();
    std::thread::Builder::new()
        .name("git-proxy-sync".into())
        .spawn(move || {
            let _ = hidden_command("git")
                .args(["config", "--global", "http.proxy", &server_clone])
                .output();
            let _ = hidden_command("git")
                .args(["config", "--global", "https.proxy", &server_clone])
                .output();
        })
        .ok();

    Ok(report)
}

#[cfg(windows)]
pub fn disable() -> Result<TerminalProxyReport> {
    if crate::diag::enabled() {
        // 连"删掉"也拦：诊断壳从没写过这些变量，删的是用户实例留下的那份。
        return Err(anyhow!(crate::diag::BLOCKED));
    }
    let mut report = TerminalProxyReport {
        written: Vec::new(),
        removed: Vec::new(),
        untouched: Vec::new(),
    };
    for name in PROXY_VARS {
        match read_env_value(name) {
            Some(value) if is_ours(&value) => {
                delete_env_value(name)?;
                report.removed.push(name.to_owned());
            }
            Some(_) => report.untouched.push(name.to_owned()),
            None => {}
        }
    }
    // NO_PROXY 只有在没有代理变量残留时才顺手清掉，避免误删用户自己的例外列表。
    if report.untouched.is_empty() {
        if let Some(value) = read_env_value(NO_PROXY_VAR) {
            if value == NO_PROXY_VALUE {
                delete_env_value(NO_PROXY_VAR)?;
                report.removed.push(NO_PROXY_VAR.to_owned());
            }
        }
    }
    broadcast_environment_change();

    // 异步还原 Git 全局代理（仅清除指向我方 127.0.0.1 的配置），绝不阻塞当前线程
    std::thread::Builder::new()
        .name("git-proxy-unset".into())
        .spawn(move || {
            if let Ok(output) = hidden_command("git")
                .args(["config", "--global", "--get", "http.proxy"])
                .output()
            {
                let current = String::from_utf8_lossy(&output.stdout);
                if is_ours(&current) {
                    let _ = hidden_command("git")
                        .args(["config", "--global", "--unset", "http.proxy"])
                        .output();
                }
            }
            if let Ok(output) = hidden_command("git")
                .args(["config", "--global", "--get", "https.proxy"])
                .output()
            {
                let current = String::from_utf8_lossy(&output.stdout);
                if is_ours(&current) {
                    let _ = hidden_command("git")
                        .args(["config", "--global", "--unset", "https.proxy"])
                        .output();
                }
            }
        })
        .ok();

    Ok(report)
}

#[cfg(not(windows))]
pub fn enable(_port: u16) -> Result<TerminalProxyReport> {
    Err(anyhow!("仅 Windows 支持写入终端代理环境变量"))
}

#[cfg(not(windows))]
pub fn disable() -> Result<TerminalProxyReport> {
    Err(anyhow!("仅 Windows 支持写入终端代理环境变量"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_own_values_are_ever_touched() {
        assert!(is_ours("http://127.0.0.1:9980"));
        assert!(!is_ours("http://proxy.corp.example:3128"));
        assert!(!is_ours("socks5://127.0.0.1:1080"));
        assert!(!is_ours(""));
    }

    #[test]
    fn report_summary_reads_in_chinese() {
        let report = TerminalProxyReport {
            written: vec!["HTTP_PROXY".into()],
            removed: Vec::new(),
            untouched: vec!["NO_PROXY".into()],
        };
        let text = report.summary();
        assert!(text.contains("已写入 HTTP_PROXY"));
        assert!(text.contains("保留你原有的 NO_PROXY"));
    }

    #[test]
    #[cfg(windows)]
    fn test_win32_registry_roundtrip() {
        let test_var = "__VPN_PARSER_TEST_VAR__";
        let test_val = "http://127.0.0.1:54321";
        // Ensure clean before
        let _ = delete_env_value(test_var);
        assert_eq!(read_env_value(test_var), None);

        // Write
        write_env_value(test_var, test_val).expect("write should succeed");
        assert_eq!(read_env_value(test_var).as_deref(), Some(test_val));

        // Delete
        delete_env_value(test_var).expect("delete should succeed");
        assert_eq!(read_env_value(test_var), None);
    }
}

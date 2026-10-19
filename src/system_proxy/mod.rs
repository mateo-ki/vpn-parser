use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::Arc;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub mod terminal;

const INTERNET_SETTINGS_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemProxySnapshot {
    pub enabled: bool,
    pub server: Option<String>,
    pub override_list: Option<String>,
}

pub trait SystemProxyControl: Send + Sync {
    /// 这个平台到底有没有"系统代理"这个可写的位置。Windows 有（WinINET 注册表），
    /// Linux/macOS 没有全局的那一格。
    ///
    /// 单独问一句而不是让 `enable` 报错再猜：代理启动那条路在内核**已经起来之后**才写
    /// 系统代理，写不进去不能等于启动失败 —— 否则 Linux 上内核明明在跑、界面却说失败。
    fn supported(&self) -> bool {
        system_proxy_supported_on_this_platform()
    }

    fn capture(&self) -> Result<SystemProxySnapshot>;
    fn enable(&self, server: &str) -> Result<()>;
    fn restore(&self, snapshot: &SystemProxySnapshot) -> Result<()>;
}

/// 唯一一份"本平台能不能写系统代理"的判据，启动流程与界面共用。
pub fn system_proxy_supported_on_this_platform() -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        true
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        gsettings_available()
    }
}

#[derive(Debug, Default)]
pub struct PlatformSystemProxy;

pub fn platform_system_proxy() -> Arc<dyn SystemProxyControl> {
    Arc::new(PlatformSystemProxy)
}

#[cfg(windows)]
impl SystemProxyControl for PlatformSystemProxy {
    fn capture(&self) -> Result<SystemProxySnapshot> {
        let enabled = query_reg_value("ProxyEnable")
            .and_then(|value| parse_dword(&value))
            .unwrap_or(false);
        Ok(SystemProxySnapshot {
            enabled,
            server: query_reg_value("ProxyServer"),
            override_list: query_reg_value("ProxyOverride"),
        })
    }

    fn enable(&self, server: &str) -> Result<()> {
        if crate::diag::enabled() {
            return Err(anyhow!(crate::diag::BLOCKED));
        }
        let _ = delete_reg_value("AutoConfigURL");
        set_reg_string("ProxyServer", server)?;
        set_reg_dword("ProxyEnable", 1)?;
        notify_proxy_change()
    }

    fn restore(&self, snapshot: &SystemProxySnapshot) -> Result<()> {
        if crate::diag::enabled() {
            return Err(anyhow!(crate::diag::BLOCKED));
        }
        match &snapshot.server {
            Some(server) => set_reg_string("ProxyServer", server)?,
            None => delete_reg_value("ProxyServer")?,
        }
        match &snapshot.override_list {
            Some(value) => set_reg_string("ProxyOverride", value)?,
            None => delete_reg_value("ProxyOverride")?,
        }
        set_reg_dword("ProxyEnable", u32::from(snapshot.enabled))?;
        notify_proxy_change()
    }
}

#[cfg(target_os = "macos")]
impl SystemProxyControl for PlatformSystemProxy {
    fn capture(&self) -> Result<SystemProxySnapshot> {
        let service = macos_primary_network_service().unwrap_or_else(|| "Wi-Fi".to_string());
        let output = Command::new("networksetup")
            .args(["-getwebproxy", &service])
            .output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let enabled = text.lines().any(|l| l.contains("Enabled: Yes"));
        let server = text.lines().find_map(|l| {
            if l.starts_with("Server: ") {
                let s = l.trim_start_matches("Server: ").trim();
                if !s.is_empty() {
                    Some(s.to_string())
                } else {
                    None
                }
            } else {
                None
            }
        });
        Ok(SystemProxySnapshot {
            enabled,
            server,
            override_list: None,
        })
    }

    fn enable(&self, server: &str) -> Result<()> {
        if crate::diag::enabled() {
            return Err(anyhow!(crate::diag::BLOCKED));
        }
        let (host, port) = parse_host_port(server)?;
        let service = macos_primary_network_service().unwrap_or_else(|| "Wi-Fi".to_string());
        let _ = Command::new("networksetup")
            .args(["-setwebproxy", &service, &host, &port.to_string()])
            .output();
        let _ = Command::new("networksetup")
            .args(["-setsecurewebproxy", &service, &host, &port.to_string()])
            .output();
        let _ = Command::new("networksetup")
            .args(["-setwebproxystate", &service, "on"])
            .output();
        let _ = Command::new("networksetup")
            .args(["-setsecurewebproxystate", &service, "on"])
            .output();
        Ok(())
    }

    fn restore(&self, snapshot: &SystemProxySnapshot) -> Result<()> {
        if crate::diag::enabled() {
            return Err(anyhow!(crate::diag::BLOCKED));
        }
        let service = macos_primary_network_service().unwrap_or_else(|| "Wi-Fi".to_string());
        if snapshot.enabled {
            if let Some(server) = &snapshot.server {
                if let Ok((host, port)) = parse_host_port(server) {
                    let _ = Command::new("networksetup")
                        .args(["-setwebproxy", &service, &host, &port.to_string()])
                        .output();
                    let _ = Command::new("networksetup")
                        .args(["-setsecurewebproxy", &service, &host, &port.to_string()])
                        .output();
                    let _ = Command::new("networksetup")
                        .args(["-setwebproxystate", &service, "on"])
                        .output();
                    let _ = Command::new("networksetup")
                        .args(["-setsecurewebproxystate", &service, "on"])
                        .output();
                    return Ok(());
                }
            }
        }
        let _ = Command::new("networksetup")
            .args(["-setwebproxystate", &service, "off"])
            .output();
        let _ = Command::new("networksetup")
            .args(["-setsecurewebproxystate", &service, "off"])
            .output();
        Ok(())
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn gsettings_available() -> bool {
    let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.system.proxy", "mode"])
        .output()
    else {
        return false;
    };
    output.status.success()
}

#[cfg(all(not(windows), not(target_os = "macos")))]
impl SystemProxyControl for PlatformSystemProxy {
    fn capture(&self) -> Result<SystemProxySnapshot> {
        if !gsettings_available() {
            return Err(anyhow!("当前桌面环境缺少 gsettings 工具，无法自动配置系统代理"));
        }
        let output = Command::new("gsettings")
            .args(["get", "org.gnome.system.proxy", "mode"])
            .output()?;
        let mode_raw = String::from_utf8_lossy(&output.stdout);
        let mode = mode_raw.trim().trim_matches('\'').to_string();
        let enabled = mode != "none";

        let host_out = Command::new("gsettings")
            .args(["get", "org.gnome.system.proxy.http", "host"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
            .unwrap_or_default();

        let port_out = Command::new("gsettings")
            .args(["get", "org.gnome.system.proxy.http", "port"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        let server = if !host_out.is_empty() && port_out != "0" {
            Some(format!("{host_out}:{port_out}"))
        } else {
            None
        };

        Ok(SystemProxySnapshot {
            enabled,
            server: Some(mode), // 使用 server 字段记录原有的 mode ('none', 'manual', 'auto')
            override_list: server,
        })
    }

    fn enable(&self, server: &str) -> Result<()> {
        if crate::diag::enabled() {
            return Err(anyhow!(crate::diag::BLOCKED));
        }
        if !gsettings_available() {
            return Err(anyhow!("当前桌面环境未安装 gsettings，请手动为应用程序设置代理 127.0.0.1:9980"));
        }
        let (host, port) = parse_host_port(server)?;

        let host_arg = format!("'{host}'");
        let port_arg = port.to_string();

        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.http", "host", &host_arg])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.http", "port", &port_arg])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.https", "host", &host_arg])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.https", "port", &port_arg])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.socks", "host", &host_arg])
            .output();
        let _ = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy.socks", "port", &port_arg])
            .output();

        let status = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", "mode", "'manual'"])
            .status()?;

        if !status.success() {
            return Err(anyhow!("执行 gsettings 设置系统代理失败"));
        }

        tracing::info!(host = %host, port = port, "已成功通过 GSettings 启用 Linux 桌面系统代理");
        Ok(())
    }

    fn restore(&self, snapshot: &SystemProxySnapshot) -> Result<()> {
        if crate::diag::enabled() {
            return Err(anyhow!(crate::diag::BLOCKED));
        }
        if !gsettings_available() {
            return Ok(());
        }

        let original_mode = snapshot.server.as_deref().unwrap_or("none");
        let mode_arg = format!("'{original_mode}'");

        let status = Command::new("gsettings")
            .args(["set", "org.gnome.system.proxy", "mode", &mode_arg])
            .status()?;

        if !status.success() {
            let _ = Command::new("gsettings")
                .args(["set", "org.gnome.system.proxy", "mode", "'none'"])
                .output();
        }

        tracing::info!(mode = %original_mode, "已成功通过 GSettings 恢复 Linux 桌面系统代理模式");
        Ok(())
    }
}

#[allow(dead_code)]
fn parse_host_port(server: &str) -> Result<(String, u16)> {
    let clean = server.trim_start_matches("http://").trim_start_matches("https://");
    if let Some((host, port_str)) = clean.rsplit_once(':') {
        let port: u16 = port_str.parse().map_err(|e| anyhow!("无效端口: {e}"))?;
        Ok((host.to_string(), port))
    } else {
        Err(anyhow!("代理地址缺少端口: {server}"))
    }
}

#[cfg(target_os = "macos")]
fn macos_primary_network_service() -> Option<String> {
    let output = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let interface = text.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with("interface:") {
            Some(trimmed.trim_start_matches("interface:").trim().to_string())
        } else {
            None
        }
    })?;

    let order_out = Command::new("networksetup")
        .args(["-listnetworkserviceorder"])
        .output()
        .ok()?;
    let order_text = String::from_utf8_lossy(&order_out.stdout);
    let mut current_service = None;
    for line in order_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('(') && trimmed.contains("Hardware Port:") {
            if trimmed.contains(&format!("Device: {interface}")) {
                return current_service;
            }
        } else if trimmed.starts_with('(') {
            if let Some(pos) = trimmed.find(')') {
                current_service = Some(trimmed[pos + 1..].trim().to_string());
            }
        }
    }
    current_service.or_else(|| Some("Wi-Fi".to_string()))
}

/// 读取 Windows 系统代理（v2rayN/Clash 等工具会写这里），返回 `http://host:port`。
/// 未启用、非 Windows 或读取失败时返回 None。阻塞式注册表查询，别在热点路径调用。
pub fn current_system_proxy_url() -> Option<String> {
    let snapshot = platform_system_proxy().capture().ok()?;
    if !snapshot.enabled {
        return None;
    }
    normalize_proxy_server(&snapshot.server?)
}

/// 程序写完之后，注册表里那份系统代理还指着我们吗？
///
/// 只在"我们自认为代理在跑、而且是我们开的那份系统代理"（`local_proxy_ready`）时才有意义。
/// 程序以前只在启停那一刻写一次注册表，从不回头核对 —— 任何外部动作（Windows 设置里手动关、
/// 别的代理工具接管、脚本）改掉它，界面仍然显示"代理运行中 + 实测 200ms"，用户看到的是假绿
/// （2026-10-08 实测：`ProxyEnable=0` 而 `ProxyServer` 还留着 127.0.0.1:9980，浏览器全断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    InSync,
    /// 开关被别人关了：隧道可能还是通的，但浏览器不会走它。
    Disabled,
    /// 开关开着但指向别的地址：另一个代理程序接管了。
    OtherServer {
        expected: String,
        found: Option<String>,
    },
}

/// 纯判据。`expected_server` 是 `host:port` 形式（本程序写进去的就是 mixed 端口）。
pub fn classify_drift(snapshot: &SystemProxySnapshot, expected_server: &str) -> Drift {
    if !snapshot.enabled {
        return Drift::Disabled;
    }
    let raw = snapshot
        .server
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let matches = match raw {
        Some(server) => {
            server == expected_server
                || normalize_proxy_server(server).as_deref()
                    == Some(format!("http://{expected_server}").as_str())
        }
        None => false,
    };
    if matches {
        Drift::InSync
    } else {
        Drift::OtherServer {
            expected: expected_server.to_string(),
            found: raw.map(str::to_string),
        }
    }
}

/// 给界面用的一句话：必须说清"谁的错、影响什么"，不能只说"异常"。
pub fn drift_text(drift: &Drift) -> String {
    match drift {
        Drift::InSync => String::new(),
        Drift::Disabled => "系统代理被外部关闭（浏览器不会走代理）".to_string(),
        Drift::OtherServer { expected, found } => match found {
            Some(server) => format!("系统代理被改指到 {server}（本程序的是 {expected}）"),
            None => format!("系统代理没有写地址（本程序的是 {expected}）"),
        },
    }
}

/// ProxyServer 可能是 "127.0.0.1:10808"，也可能是 "http=...;https=...;ftp=..."。
fn normalize_proxy_server(server: &str) -> Option<String> {
    let host_port = if server.contains('=') || server.contains(';') {
        // 优先 https=（本程序拉的都是 https），找不到再退 http=。
        let found = server
            .split(';')
            .find_map(|part| part.trim().strip_prefix("https="))
            .or_else(|| {
                server
                    .split(';')
                    .find_map(|part| part.trim().strip_prefix("http="))
            });
        found?.trim().to_string()
    } else {
        server.trim().to_string()
    };
    if host_port.is_empty() {
        None
    } else {
        Some(format!("http://{host_port}"))
    }
}

pub fn parse_reg_value(output: &str, name: &str) -> Option<String> {
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let Some(value_name) = parts.next() else {
            continue;
        };
        if value_name != name {
            continue;
        }
        let Some(value_type) = parts.next() else {
            continue;
        };
        if !value_type.starts_with("REG_") {
            continue;
        }
        let value = parts.collect::<Vec<_>>().join(" ");
        return (!value.is_empty()).then_some(value);
    }
    None
}

fn parse_dword(value: &str) -> Option<bool> {
    if let Some(hex) = value.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok().map(|value| value != 0);
    }
    value.parse::<u32>().ok().map(|value| value != 0)
}

#[cfg(windows)]
fn query_reg_value(name: &str) -> Option<String> {
    let output = hidden_command("reg.exe")
        .args(["query", INTERNET_SETTINGS_KEY, "/v", name])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_reg_value(&String::from_utf8_lossy(&output.stdout), name))
        .flatten()
}

#[cfg(windows)]
fn set_reg_string(name: &str, value: &str) -> Result<()> {
    run_reg(&[
        "add",
        INTERNET_SETTINGS_KEY,
        "/v",
        name,
        "/t",
        "REG_SZ",
        "/d",
        value,
        "/f",
    ])
}

#[cfg(windows)]
fn set_reg_dword(name: &str, value: u32) -> Result<()> {
    let value = value.to_string();
    run_reg(&[
        "add",
        INTERNET_SETTINGS_KEY,
        "/v",
        name,
        "/t",
        "REG_DWORD",
        "/d",
        &value,
        "/f",
    ])
}

#[cfg(windows)]
/// reg.exe 在中文 Windows 上输出的是 GBK 而非 UTF-8，`from_utf8_lossy` 会把它变成
/// 一串替换字符（页面上那些乱码就是这么来的）。解不出来就别往界面上塞。
fn command_output_text(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let cleaned: String = raw.chars().filter(|c| *c != '\u{FFFD}').collect();
    cleaned.trim().to_owned()
}

#[cfg(windows)]
/// reg.exe 的中文提示是 GBK，直接拼进消息会变成乱码；退化成退出码更好读。
fn reg_failure_detail(output: &std::process::Output) -> String {
    let text = format!(
        "{}{}",
        command_output_text(&output.stdout),
        command_output_text(&output.stderr)
    );
    if text.is_empty() {
        format!("reg.exe 退出码 {:?}", output.status.code())
    } else {
        text
    }
}

#[cfg(windows)]
fn delete_reg_value(name: &str) -> Result<()> {
    // 值本来就不存在（上一次会话根本没设过 ProxyOverride）是恢复快照的正常情况。
    // 不能靠 reg.exe 的退出码或提示文字判断：实测这台机器"找不到值"返回 1，而它
    // 的中文提示是 GBK，转成 UTF-8 只剩乱码。先查一次存在性最可靠。
    if query_reg_value(name).is_none() {
        return Ok(());
    }
    let output = hidden_command("reg.exe")
        .args(["delete", INTERNET_SETTINGS_KEY, "/v", name, "/f"])
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow!(
        "删除系统代理注册表值 {name} 失败: {}",
        reg_failure_detail(&output)
    ))
}

#[cfg(windows)]
fn run_reg(args: &[&str]) -> Result<()> {
    let output = hidden_command("reg.exe").args(args).output()?;
    if output.status.success() {
        return Ok(());
    }
    let detail = reg_failure_detail(&output);
    Err(anyhow!("修改系统代理失败: {detail}"))
}

#[cfg(windows)]
fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(windows)]
fn notify_proxy_change() -> Result<()> {
    use windows_sys::Win32::Networking::WinInet::{
        InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
    };

    let changed = unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        )
    };
    let refreshed = unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        )
    };
    if changed == 0 || refreshed == 0 {
        return Err(anyhow!("系统代理已写入，但通知 WinINet 刷新失败"));
    }
    Ok(())
}

/// 纯 Win32 原生原子级注册表快速复位。
///
/// 绝对不依赖 `reg.exe` 子进程，不执行任何堆内存分配，完全满足 SEH 崩溃异常过滤器
/// （SetUnhandledExceptionFilter）与 Panic Hook 等严苛上下文的安全调用要求。
/// 直接将 HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings 的 ProxyEnable 置为 0，
/// 并通过 WinINet API 广播刷新设置，确保异常退出时不会留下导致断网的死代理。
#[cfg(windows)]
pub fn direct_disable_wininet_proxy_atomic() -> bool {
    use windows_sys::Win32::Networking::WinInet::{
        InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
    };
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD,
    };

    const SUBKEY_W: &[u16] = &[
        'S' as u16, 'o' as u16, 'f' as u16, 't' as u16, 'w' as u16, 'a' as u16, 'r' as u16, 'e' as u16,
        '\\' as u16,
        'M' as u16, 'i' as u16, 'c' as u16, 'r' as u16, 'o' as u16, 's' as u16, 'o' as u16, 'f' as u16, 't' as u16,
        '\\' as u16,
        'W' as u16, 'i' as u16, 'n' as u16, 'd' as u16, 'o' as u16, 'w' as u16, 's' as u16,
        '\\' as u16,
        'C' as u16, 'u' as u16, 'r' as u16, 'r' as u16, 'e' as u16, 'n' as u16, 't' as u16, 'V' as u16, 'e' as u16, 'r' as u16, 's' as u16, 'i' as u16, 'o' as u16, 'n' as u16,
        '\\' as u16,
        'I' as u16, 'n' as u16, 't' as u16, 'e' as u16, 'r' as u16, 'n' as u16, 'e' as u16, 't' as u16, ' ' as u16, 'S' as u16, 'e' as u16, 't' as u16, 't' as u16, 'i' as u16, 'n' as u16, 'g' as u16, 's' as u16,
        0,
    ];
    const PROXY_ENABLE_W: &[u16] = &[
        'P' as u16, 'r' as u16, 'o' as u16, 'x' as u16, 'y' as u16, 'E' as u16, 'n' as u16, 'a' as u16, 'b' as u16, 'l' as u16, 'e' as u16,
        0,
    ];

    let mut hkey = std::ptr::null_mut();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            SUBKEY_W.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
    };
    if status != 0 || hkey.is_null() {
        return false;
    }

    let zero: u32 = 0;
    let set_status = unsafe {
        RegSetValueExW(
            hkey,
            PROXY_ENABLE_W.as_ptr(),
            0,
            REG_DWORD,
            &zero as *const u32 as *const u8,
            std::mem::size_of::<u32>() as u32,
        )
    };
    unsafe {
        RegCloseKey(hkey);
    }

    if set_status == 0 {
        unsafe {
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_SETTINGS_CHANGED,
                std::ptr::null_mut(),
                0,
            );
            InternetSetOptionW(
                std::ptr::null_mut(),
                INTERNET_OPTION_REFRESH,
                std::ptr::null_mut(),
                0,
            );
        }
        true
    } else {
        false
    }
}

#[cfg(not(windows))]
pub fn direct_disable_wininet_proxy_atomic() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(enabled: bool, server: Option<&str>) -> SystemProxySnapshot {
        SystemProxySnapshot {
            enabled,
            server: server.map(str::to_string),
            override_list: None,
        }
    }

    #[test]
    fn drift_detects_the_exact_case_that_bit_the_user() {
        // 2026-10-08 实测现场：ProxyEnable=0，ProxyServer 还留着我们的端口。
        assert_eq!(
            classify_drift(&snap(false, Some("127.0.0.1:9980")), "127.0.0.1:9980"),
            Drift::Disabled
        );
        assert!(drift_text(&Drift::Disabled).contains("浏览器不会走代理"));
    }

    #[test]
    fn our_own_value_counts_as_in_sync_however_it_is_written() {
        assert_eq!(
            classify_drift(&snap(true, Some("127.0.0.1:9980")), "127.0.0.1:9980"),
            Drift::InSync
        );
        // 分协议写法（别的工具/系统策略常这么写）也要认出来。
        assert_eq!(
            classify_drift(
                &snap(
                    true,
                    Some("http=127.0.0.1:9980;https=127.0.0.1:9980;ftp=127.0.0.1:9980")
                ),
                "127.0.0.1:9980"
            ),
            Drift::InSync
        );
        assert!(drift_text(&Drift::InSync).is_empty());
    }

    #[test]
    fn another_tool_taking_over_is_reported_with_both_addresses() {
        let drift = classify_drift(&snap(true, Some("127.0.0.1:7890")), "127.0.0.1:9980");
        assert_eq!(
            drift,
            Drift::OtherServer {
                expected: "127.0.0.1:9980".into(),
                found: Some("127.0.0.1:7890".into()),
            }
        );
        let text = drift_text(&drift);
        assert!(
            text.contains("127.0.0.1:7890") && text.contains("127.0.0.1:9980"),
            "{text}"
        );
        // 开关开着但压根没有地址：不能说"指向空"，要说没写地址。
        let missing = classify_drift(&snap(true, None), "127.0.0.1:9980");
        assert!(drift_text(&missing).contains("没有写地址"), "{missing:?}");
    }

    #[cfg(windows)]
    #[test]
    fn deleting_an_absent_registry_value_is_not_an_error() {
        // 恢复系统代理时曾经因为"值不存在"整个启动流程被判失败，这里守住。
        let result = delete_reg_value("ProxyOverride-Definitely-Absent-9f3c1a");
        assert!(result.is_ok(), "{result:?}");
    }

    #[cfg(windows)]
    #[test]
    fn undecodable_reg_output_never_reaches_the_ui_as_mojibake() {
        // 「错误：」的 GBK 字节，不是合法 UTF-8：以前会被 from_utf8_lossy 变成一串
        // 替换字符直接打到界面上，现在必须被过滤掉。
        let gbk: Vec<u8> = vec![0xb4, 0xed, 0xce, 0xf3, 0xa3, 0xba];
        let text = command_output_text(&gbk);
        assert!(!text.contains(char::REPLACEMENT_CHARACTER));
    }

    #[test]
    fn parses_registry_dword_and_string_values() {
        let enabled =
            "\nHKEY_CURRENT_USER\\Software\\Microsoft\n    ProxyEnable    REG_DWORD    0x1";
        let server = "    ProxyServer    REG_SZ    127.0.0.1:7890";

        assert_eq!(parse_reg_value(enabled, "ProxyEnable"), Some("0x1".into()));
        assert_eq!(
            parse_reg_value(server, "ProxyServer"),
            Some("127.0.0.1:7890".into())
        );
    }

    #[test]
    fn normalizes_proxy_server_registry_forms() {
        // v2rayN 写的简单形式
        assert_eq!(
            normalize_proxy_server("127.0.0.1:10808"),
            Some("http://127.0.0.1:10808".into())
        );
        // 按协议分组的形式：优先取 https，其次 http
        assert_eq!(
            normalize_proxy_server("ftp=1.1.1.1:21;http=2.2.2.2:80;https=3.3.3.3:443"),
            Some("http://3.3.3.3:443".into())
        );
        assert_eq!(
            normalize_proxy_server("http=2.2.2.2:80;ftp=1.1.1.1:21"),
            Some("http://2.2.2.2:80".into())
        );
        assert_eq!(normalize_proxy_server("  "), None);
        assert_eq!(normalize_proxy_server("socks=5.5.5.5:1080"), None);
    }

    #[test]
    fn ignores_unrelated_registry_lines() {
        let output = "HKEY_CURRENT_USER\\Software\\Microsoft\n    Other    REG_SZ    value";

        assert_eq!(parse_reg_value(output, "ProxyServer"), None);
    }

    #[test]
    fn parses_host_port_for_system_proxy() {
        assert_eq!(
            parse_host_port("127.0.0.1:9980").unwrap(),
            ("127.0.0.1".into(), 9980)
        );
        assert_eq!(
            parse_host_port("http://127.0.0.1:7890").unwrap(),
            ("127.0.0.1".into(), 7890)
        );
        assert!(parse_host_port("invalid-no-port").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_direct_disable_wininet_proxy_atomic_executes_safely() {
        let result = direct_disable_wininet_proxy_atomic();
        // 在 Windows 具备 HKCU 访问权限的环境下正常返回 true
        assert!(result);
    }
}

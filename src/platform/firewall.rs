//! Windows 防火墙"允许应用"规则的自动添加。
//!
//! 为什么需要：这台机上实测出现过"入站到未放行程序的环回连接被防火墙拦掉"——表现成
//! `127.0.0.1:445/135` 超时、而 LAN 正常。对这个软件来说更直接：`vpn-parser.exe` 和内核
//! exe 都要在 127.0.0.1 上监听（mixed 9980 / controller 9090），一旦被拦就是"启动代理没反应"。
//! 而**每次重新编译/更新内核，exe 内容变了但路径不变**，所以规则要按**路径**加才不会被
//! 更新打断。加规则需要管理员权限，非提权时只报告不动作。

use anyhow::{anyhow, Result};
use std::path::Path;

/// 规则名前缀，便于识别与幂等检查。
pub const RULE_PREFIX: &str = "VPN Parser";

pub fn rule_name(program: &Path) -> String {
    let stem = program
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "program".to_string());
    format!("{RULE_PREFIX} · {stem}")
}

#[cfg(windows)]
fn hidden(command: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

#[cfg(windows)]
fn run_netsh(args: &[&str]) -> Result<std::process::Output> {
    let mut command = std::process::Command::new("netsh");
    command.args(args);
    hidden(&mut command);
    command
        .output()
        .map_err(|error| anyhow!("调用 netsh 失败: {error}"))
}

#[cfg(windows)]
pub fn rule_exists(program: &Path) -> bool {
    let name = rule_name(program);
    match run_netsh(&[
        "advfirewall",
        "firewall",
        "show",
        "rule",
        &format!("name={name}"),
    ]) {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            // 找不到时 netsh 仍返回 0，只能看输出里有没有这条规则的名字。
            output.status.success() && text.contains(&name)
        }
        Err(_) => false,
    }
}

/// netsh 把错误写在 **stdout**，stderr 通常是空的。这台机上实测：`add rule` 缺权限时
/// stdout = 56 字节 UTF-8「请求的操作需要提升(作为管理员运行)。」，stderr = 0 字节。
/// 只读 stderr 的话，界面上就永远是一句没用的"未知错误"（2026-10-15 他报的那条）。
fn netsh_error_line(stdout: &[u8], stderr: &[u8], code: Option<i32>) -> String {
    for stream in [stdout, stderr] {
        let text = String::from_utf8_lossy(stream);
        if let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) {
            return line.to_string();
        }
    }
    format!("netsh 没有给出原因（退出码 {}）", code.unwrap_or(-1))
}

/// 把 netsh 的原话翻成"下一步该干什么"，并把提权状态一起报出来 —— 这条链路上
/// "是不是管理员"就是第一嫌疑，界面不该让人猜。
fn explain_failure(raw: &str, elevated: bool) -> String {
    let needs_admin = raw.contains("需要提升")
        || raw.contains("管理员")
        || raw.to_ascii_lowercase().contains("elevation")
        || raw.to_ascii_lowercase().contains("administrator");
    if needs_admin {
        return format!(
            "需要管理员权限才能改防火墙规则（当前进程提权={}）。用管理员身份启动本程序，\
             或先关掉自动放行、自己手动加规则",
            if elevated { "是" } else { "否" }
        );
    }
    format!(
        "{raw}（当前进程提权={}）",
        if elevated { "是" } else { "否" }
    )
}

/// 交给 netsh 的 `program=` 值：绝对路径，但**不带** `\\?\` verbatim 前缀。
///
/// 实测（非提权 shell 里只测参数解析）：`program=\\?\C:\Apps\VPN Parser\vpn-parser.exe`
/// 被 netsh 当场拒掉，报的就是用户看到的那句"应用程序包含无效的字符，或具有无效的长度"；
/// 同一个路径把前缀去掉就解析通过（走到权限检查）。而 `Path::canonicalize()` 在 Windows
/// 上偏偏就会给出带前缀的那种 —— 所以规范化借它做，前缀必须摘掉再交给 netsh。
fn netsh_program_value(path: &Path) -> String {
    let text = path.to_string_lossy().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => text,
    }
}

/// 按路径加一条"入站允许"规则（已存在就跳过）。返回一句给人看的报告。
#[cfg(windows)]
pub fn ensure_allowed(program: &Path) -> Result<String> {
    if crate::diag::enabled() {
        // netsh 改的是用户机器的防火墙规则，诊断壳没有资格动它。
        return Err(anyhow!("诊断实例（VPN_PARSER_DIAG）不写防火墙规则"));
    }
    let full = program
        .canonicalize()
        .map_err(|error| anyhow!("解析 {:?} 失败: {error}", program))?;
    if rule_exists(&full) {
        return Ok(format!("{} 已在允许列表", full.display()));
    }
    let name = rule_name(&full);
    let value = netsh_program_value(&full);
    let output = run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={name}"),
        &format!("program={value}"),
        "dir=in",
        "action=allow",
        "enable=yes",
        "profile=any",
    ])?;
    if !output.status.success() {
        let raw = netsh_error_line(&output.stdout, &output.stderr, output.status.code());
        let elevated = crate::platform::is_elevated();
        // 原文进日志：界面上那句是翻译过的，取证要能看到 netsh 到底说了什么。
        crate::startup_log(&format!(
            "[防火墙] {} 添加失败：{raw}（提权={}）",
            value,
            if elevated { "是" } else { "否" }
        ));
        return Err(anyhow!(
            "添加防火墙规则失败：{}",
            explain_failure(&raw, elevated)
        ));
    }
    Ok(format!("已加入允许列表：{value}"))
}

/// 一批程序；单个失败不影响其余（比如某个内核还没下载）。
#[cfg(windows)]
pub fn ensure_all(programs: &[std::path::PathBuf]) -> Result<String> {
    let mut added = Vec::new();
    let mut failed = Vec::new();
    for program in programs {
        if !program.exists() {
            continue;
        }
        match ensure_allowed(program) {
            Ok(message) => added.push(message),
            Err(error) => failed.push(format!(
                "{:?}: {error}",
                program.file_name().unwrap_or_default()
            )),
        }
    }
    if failed.is_empty() {
        Ok(format!("防火墙放行：{} 项已就绪", added.len()))
    } else {
        Err(anyhow!(
            "防火墙放行部分失败（{} 成功 / {} 失败：{}）",
            added.len(),
            failed.len(),
            failed.join("；")
        ))
    }
}

#[cfg(not(windows))]
pub fn rule_exists(_program: &Path) -> bool {
    false
}

#[cfg(not(windows))]
pub fn ensure_allowed(program: &Path) -> Result<String> {
    Err(anyhow!(
        "仅 Windows 支持自动添加防火墙规则（{}）",
        program.display()
    ))
}

#[cfg(not(windows))]
pub fn ensure_all(programs: &[std::path::PathBuf]) -> Result<String> {
    Err(anyhow!(
        "仅 Windows 支持自动添加防火墙规则（{} 个程序）",
        programs.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rule_names_are_stable_and_prefixed() {
        // 这条要钉的性质是"同目录换 exe、规则名不变（按程序名放行），不同程序名要能区分"。
        // 分隔符按平台给真路径：反斜杠在 Linux 上不是分隔符，整串会被当成一个文件名，
        // 那样断言的就不是这个性质了（Linux 上第一次跑就是这样红的）。
        #[cfg(windows)]
        let (same_name_a, same_name_b, other) = (
            r"C:\Apps\VPN Parser\vpn-parser.exe",
            r"E:\other\place\vpn-parser.exe",
            r"\tmp\mihomo.exe",
        );
        #[cfg(not(windows))]
        let (same_name_a, same_name_b, other) = (
            "/opt/VPN Parser/vpn-parser",
            "/srv/other/place/vpn-parser",
            "/tmp/mihomo",
        );

        let name = rule_name(&PathBuf::from(same_name_a));
        assert!(name.starts_with(RULE_PREFIX), "规则名要能认出来: {name}");
        // 路径不变、exe 变了（每次重编译）→ 规则名也不变，这正是"按路径放行"的意义。
        assert_eq!(name, rule_name(&PathBuf::from(same_name_b)));
        assert!(rule_name(&PathBuf::from(other)).contains("mihomo"));
    }

    /// 这台机上 netsh 缺权限时的真实输出：stdout 一句 UTF-8 中文，stderr 空字节。
    /// 旧写法只读 stderr ⇒ 界面只能显示"未知错误"。
    #[test]
    fn netsh_writes_its_reason_to_stdout_not_stderr() {
        let stdout = "请求的操作需要提升(作为管理员运行)。\n\r\n".as_bytes();
        let read = netsh_error_line(stdout, &[], Some(1));
        assert_eq!(
            read, "请求的操作需要提升(作为管理员运行)。",
            "实际读到：{read:?}"
        );
        // 反向对照：旧的那条路（只看 stderr、首行空就兜底）在这种情况下就是"未知错误"。
        let old_style = String::from_utf8_lossy(&[][..])
            .lines()
            .next()
            .unwrap_or("未知错误")
            .trim()
            .to_string();
        assert_eq!(old_style, "未知错误", "对照组要真的复现出那句没用的话");
        // 两个流都空：至少报退出码，别再报"未知错误"。
        assert_eq!(
            netsh_error_line(&[], &[], Some(2)),
            "netsh 没有给出原因（退出码 2）"
        );
        // stderr 有内容时仍然优先它之外的第一条非空行（netsh 偶尔两边都写）。
        assert_eq!(
            netsh_error_line("  \n第一条\n第二条\n".as_bytes(), &[], Some(1)),
            "第一条"
        );
    }

    /// 缺权限这句必须翻成"下一步做什么"，并把提权状态带上。
    #[test]
    fn an_elevation_failure_tells_the_user_what_to_do_next() {
        let text = explain_failure("请求的操作需要提升(作为管理员运行)。", false);
        assert!(text.contains("需要管理员权限"), "{text}");
        assert!(text.contains("提权=否"), "{text}");
        let other = explain_failure("无法与防火墙服务通信。", true);
        assert!(other.contains("无法与防火墙服务通信"), "{other}");
        assert!(other.contains("提权=是"), "{other}");
    }

    /// netsh 只认普通绝对路径：`\\?\` 那种 verbatim 形式会被它判成"无效的字符或长度"。
    #[test]
    fn the_verbatim_prefix_never_reaches_netsh() {
        assert_eq!(
            netsh_program_value(&PathBuf::from(r"\\?\C:\Apps\VPN Parser\vpn-parser.exe")),
            r"C:\Apps\VPN Parser\vpn-parser.exe"
        );
        assert_eq!(
            netsh_program_value(&PathBuf::from(r"\\?\UNC\fileserv\share\mihomo.exe")),
            r"\\fileserv\share\mihomo.exe"
        );
        // 本来就没有前缀的（非 Windows、或没走 canonicalize 的调用方）原样通过。
        assert_eq!(
            netsh_program_value(&PathBuf::from(r"C:\Apps\VPN Parser\xray.exe")),
            r"C:\Apps\VPN Parser\xray.exe"
        );
    }

    /// 这条钉住"为什么需要上一条"：Windows 的 canonicalize 确实会给 verbatim 路径。
    /// 哪天它不给了，上一条就是死代码，该删。
    #[test]
    #[cfg(windows)]
    fn windows_canonicalize_really_does_hand_back_the_verbatim_form() {
        let full = std::env::current_exe().unwrap().canonicalize().unwrap();
        let text = full.to_string_lossy().to_string();
        assert!(
            text.starts_with(r"\\?\"),
            "canonicalize 没给 verbatim 路径（{text}）：那 netsh_program_value 的剥离就是多余的"
        );
        assert!(!netsh_program_value(&full).starts_with(r"\\?\"));
    }
}

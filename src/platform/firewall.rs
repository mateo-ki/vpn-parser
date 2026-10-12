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
    match run_netsh(&["advfirewall", "firewall", "show", "rule", &format!("name={name}")]) {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            // 找不到时 netsh 仍返回 0，只能看输出里有没有这条规则的名字。
            output.status.success() && text.contains(&name)
        }
        Err(_) => false,
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
    let output = run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={name}"),
        &format!("program={}", full.display()),
        "dir=in",
        "action=allow",
        "enable=yes",
        "profile=any",
    ])?;
    if !output.status.success() {
        return Err(anyhow!(
            "添加防火墙规则失败: {}",
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("未知错误")
                .trim()
        ));
    }
    Ok(format!("已加入允许列表：{}", full.display()))
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
            Err(error) => failed.push(format!("{:?}: {error}", program.file_name().unwrap_or_default())),
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
    Err(anyhow!("仅 Windows 支持自动添加防火墙规则（{}）", program.display()))
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
        let name = rule_name(&PathBuf::from(r"C:\Apps\VPN Parser\vpn-parser.exe"));
        assert!(name.starts_with(RULE_PREFIX), "规则名要能认出来: {name}");
        // 路径不变、exe 变了（每次重编译）→ 规则名也不变，这正是"按路径放行"的意义。
        assert_eq!(name, rule_name(&PathBuf::from(r"E:\other\place\vpn-parser.exe")));
        assert!(rule_name(&PathBuf::from("/x/mihomo.exe")).contains("mihomo"));
    }
}

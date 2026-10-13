use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::process::Command;

mod updater;

pub use updater::{
    KernelReleaseInfo, KernelUpdateProgress, KernelUpdateResult, KernelUpdateStage, KernelSource,
    SOURCE_OPTIONS, source_from_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelKind {
    Mihomo,
    Xray,
    SingBox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KernelPreference {
    #[default]
    Auto,
    Mihomo,
    SingBox,
    Xray,
}

impl KernelPreference {
    pub fn kernel(self) -> Option<KernelKind> {
        match self {
            Self::Auto => None,
            Self::Mihomo => Some(KernelKind::Mihomo),
            Self::SingBox => Some(KernelKind::SingBox),
            Self::Xray => Some(KernelKind::Xray),
        }
    }
}

impl KernelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mihomo => "mihomo",
            Self::Xray => "xray",
            Self::SingBox => "sing_box",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Mihomo => "Mihomo",
            Self::Xray => "Xray",
            Self::SingBox => "Sing-box",
        }
    }

    pub fn environment_variable(self) -> &'static str {
        match self {
            Self::Mihomo => "MIHOMO_PATH",
            Self::Xray => "XRAY_PATH",
            Self::SingBox => "SING_BOX_PATH",
        }
    }

    pub fn executable_name(self) -> &'static str {
        match self {
            Self::Mihomo => "mihomo.exe",
            Self::Xray => "xray.exe",
            Self::SingBox => "sing-box.exe",
        }
    }

    pub fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Mihomo => &["-v"],
            Self::Xray | Self::SingBox => &["version"],
        }
    }

    pub fn directory_name(self) -> &'static str {
        match self {
            Self::Mihomo => "mihomo",
            Self::Xray => "xray",
            Self::SingBox => "sing-box",
        }
    }

    pub fn github_repository(self) -> &'static str {
        match self {
            Self::Mihomo => "MetaCubeX/mihomo",
            Self::Xray => "XTLS/Xray-core",
            Self::SingBox => "SagerNet/sing-box",
        }
    }

    pub fn asset_name(self, tag: &str) -> String {
        match self {
            Self::Mihomo => format!("mihomo-windows-amd64-{tag}.zip"),
            Self::Xray => "Xray-windows-64.zip".into(),
            Self::SingBox => format!("sing-box-{}-windows-amd64.zip", tag.trim_start_matches('v')),
        }
    }

    pub fn matches_archive_executable(self, file_name: &str) -> bool {
        match self {
            Self::Mihomo => {
                file_name.to_ascii_lowercase().starts_with("mihomo")
                    && file_name.to_ascii_lowercase().ends_with(".exe")
            }
            Self::Xray => file_name.eq_ignore_ascii_case("xray.exe"),
            Self::SingBox => file_name.eq_ignore_ascii_case("sing-box.exe"),
        }
    }

    pub fn supports_proxy_protocol(self, protocol: &str) -> bool {
        let protocol = protocol.to_ascii_lowercase();
        match self {
            Self::Mihomo => matches!(
                protocol.as_str(),
                "ss" | "shadowsocks"
                    | "ssr"
                    | "shadowsocksr"
                    | "vmess"
                    | "vless"
                    | "trojan"
                    | "hysteria"
                    | "hysteria2"
                    | "hy2"
                    | "tuic"
                    | "anytls"
                    | "wireguard"
                    | "wg"
                    | "http"
                    | "https"
                    | "socks"
                    | "socks5"
            ),
            Self::SingBox => matches!(
                protocol.as_str(),
                "ss" | "shadowsocks"
                    | "vmess"
                    | "vless"
                    | "trojan"
                    | "hysteria2"
                    | "hy2"
                    | "tuic"
                    | "anytls"
                    | "wireguard"
                    | "wg"
                    | "http"
                    | "https"
                    | "socks"
                    | "socks5"
            ),
            Self::Xray => matches!(
                protocol.as_str(),
                "ss" | "shadowsocks"
                    | "vmess"
                    | "vless"
                    | "trojan"
                    | "http"
                    | "https"
                    | "socks"
                    | "socks5"
            ),
        }
    }
}

pub fn preferred_proxy_kernel(protocol: &str) -> Option<KernelKind> {
    match protocol.to_ascii_lowercase().as_str() {
        "hysteria2" | "hy2" | "tuic" | "anytls" | "wireguard" | "wg" => Some(KernelKind::SingBox),
        protocol if KernelKind::Mihomo.supports_proxy_protocol(protocol) => {
            Some(KernelKind::Mihomo)
        }
        _ => None,
    }
}

pub fn select_proxy_kernel_with(
    protocol: &str,
    available: impl Fn(KernelKind) -> bool,
) -> Result<KernelKind> {
    let preferred = preferred_proxy_kernel(protocol)
        .ok_or_else(|| anyhow::anyhow!("暂不支持协议: {protocol}"))?;
    let fallback = match preferred {
        KernelKind::Mihomo => KernelKind::SingBox,
        KernelKind::SingBox => KernelKind::Mihomo,
        KernelKind::Xray => unreachable!(),
    };
    for candidate in [preferred, fallback, KernelKind::Xray] {
        if candidate.supports_proxy_protocol(protocol) && available(candidate) {
            return Ok(candidate);
        }
    }
    Err(anyhow::anyhow!("没有已安装且支持 {protocol} 的代理内核"))
}

pub fn select_proxy_kernel(protocol: &str) -> Result<KernelKind> {
    select_proxy_kernel_with(protocol, |kind| find_kernel_executable(kind).is_some())
}

pub fn select_proxy_kernel_preference(
    protocol: &str,
    preference: KernelPreference,
    allow_xray: bool,
) -> Result<KernelKind> {
    if let Some(kind) = preference.kernel() {
        if kind == KernelKind::Xray && !allow_xray {
            return Err(anyhow::anyhow!("Xray 暂不支持 TUN 模式"));
        }
        if !kind.supports_proxy_protocol(protocol) {
            return Err(anyhow::anyhow!(
                "{} 不支持协议: {protocol}",
                kind.display_name()
            ));
        }
        if find_kernel_executable(kind).is_none() {
            return Err(anyhow::anyhow!("未找到 {} 可执行文件", kind.display_name()));
        }
        return Ok(kind);
    }

    if let Ok(kind) = select_proxy_kernel(protocol) {
        return Ok(kind);
    }
    if allow_xray
        && KernelKind::Xray.supports_proxy_protocol(protocol)
        && find_kernel_executable(KernelKind::Xray).is_some()
    {
        return Ok(KernelKind::Xray);
    }
    Err(anyhow::anyhow!("没有已安装且支持 {protocol} 的代理内核"))
}

/// 点「启动代理」之前的预检：返回"需要先去下载哪个内核"，`None` 表示可以直接启动。
///
/// 判定和 [`select_proxy_kernel_preference`] 用同一套输入（协议、内核偏好、是否系统代理
/// 模式），差别只在把"安装检查"做成注入的 `available` —— 这样这条决策表能被单测覆盖，
/// 不用真的去动用户机器上的内核目录。
/// 三种不拦的情况：点名要的内核已经装了；Auto 下有任意已装内核支持该协议；协议本身
/// 我们不认识（那时该报"暂不支持协议"，弹下载框只会让人更糊涂）。
pub fn missing_kernel_for(
    protocol: &str,
    preference: KernelPreference,
    system_mode: bool,
    available: impl Fn(KernelKind) -> bool,
) -> Option<KernelKind> {
    if let Some(kind) = preference.kernel() {
        // Xray 在 TUN 下是"不支持"，不是"没装"，别拿下载框去挡。
        if kind == KernelKind::Xray && !system_mode {
            return None;
        }
        return (!available(kind)).then_some(kind);
    }
    if select_proxy_kernel_with(protocol, &available).is_ok() {
        return None;
    }
    preferred_proxy_kernel(protocol).filter(|kind| kind.supports_proxy_protocol(protocol))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelInfo {
    pub kind: KernelKind,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
    pub available: bool,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct KernelManager;

impl KernelManager {
    pub fn info_for_missing(&self, kind: KernelKind) -> KernelInfo {
        KernelInfo {
            kind,
            path: None,
            version: None,
            available: false,
            message: format!("{} 未找到", kind.display_name()),
        }
    }

    pub async fn detect(&self, kind: KernelKind) -> KernelInfo {
        let Some(path) = find_kernel_executable(kind) else {
            return self.info_for_missing(kind);
        };

        match read_kernel_version(kind, &path).await {
            Ok(version) => KernelInfo {
                kind,
                path: Some(path),
                available: true,
                message: version
                    .clone()
                    .unwrap_or_else(|| format!("{} 可用，未返回版本", kind.display_name())),
                version,
            },
            Err(error) => KernelInfo {
                kind,
                path: Some(path),
                version: None,
                // 跑不起来就不能叫"可用"：以前这里写「可用，版本读取失败」，
                // 而按下去只会启动失败 —— 内核文件在不在盘上和它能不能运行是两件事。
                available: false,
                message: format!("{} 装了但跑不起来: {error}", kind.display_name()),
            },
        }
    }

    pub async fn detect_all(&self) -> Vec<KernelInfo> {
        let (mihomo, xray, sing_box) = tokio::join!(
            self.detect(KernelKind::Mihomo),
            self.detect(KernelKind::Xray),
            self.detect(KernelKind::SingBox)
        );
        vec![mihomo, xray, sing_box]
    }

    pub async fn check_updates_with_proxy(
        &self,
        proxy_url: Option<String>,
    ) -> Result<Vec<KernelReleaseInfo>> {
        let updater = updater::KernelUpdater::new(managed_kernel_root(), proxy_url.as_deref())?;
        let infos = self.detect_all().await;
        let (mihomo, xray, sing_box) = tokio::join!(
            updater.check(&infos[0]),
            updater.check(&infos[1]),
            updater.check(&infos[2])
        );
        let fallback = |res: Result<KernelReleaseInfo>, info: &KernelInfo| match res {
            Ok(release) => release,
            Err(err) => KernelReleaseInfo {
                kind: info.kind,
                current_version: info.version.clone(),
                latest_version: None,
                update_available: false,
                asset_name: None,
                message: format!("检查更新超时或受限: {err}"),
            },
        };
        Ok(vec![
            fallback(mihomo, &infos[0]),
            fallback(xray, &infos[1]),
            fallback(sing_box, &infos[2]),
        ])
    }

    pub async fn update_with_options<F>(
        &self,
        kind: KernelKind,
        proxy_url: Option<String>,
        source: KernelSource,
        local_file: Option<std::path::PathBuf>,
        cancelled: Arc<AtomicBool>,
        progress: F,
    ) -> Result<KernelUpdateResult>
    where
        F: Fn(KernelUpdateProgress) + Send + Sync,
    {
        updater::KernelUpdater::new(managed_kernel_root(), proxy_url.as_deref())?
            .update(kind, source, local_file.as_deref(), &cancelled, &progress)
            .await
    }
}

pub fn kernel_candidates(
    kind: KernelKind,
    current_exe: Option<&Path>,
    current_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(directory) = current_dir {
        candidates.push(
            directory
                .join("data").join("kernels")
                .join(kind.directory_name())
                .join(kind.executable_name()),
        );
    }
    if let Some(executable) = current_exe {
        if let Some(directory) = executable.parent() {
            candidates.push(
                directory
                    .join("data").join("kernels")
                    .join(kind.directory_name())
                    .join(kind.executable_name()),
            );
        }
    }
    candidates
}

pub fn find_first_existing_candidate(candidates: Vec<PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|path| path.is_file())
}

pub fn find_mihomo_executable() -> Option<PathBuf> {
    find_kernel_executable(KernelKind::Mihomo)
}

pub fn find_kernel_executable(kind: KernelKind) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(kind.environment_variable()).map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let current_exe = std::env::current_exe().ok();
    let current_dir = std::env::current_dir().ok();
    find_first_existing_candidate(kernel_candidates(
        kind,
        current_exe.as_deref(),
        current_dir.as_deref(),
    ))
}

/// 路径一律按分量拼。写成 `join("data/kernels")` 也能用，但显示出来就是
/// `C:\...\VPN Parser\data/kernels\mihomo\mihomo.exe` 这种正反斜杠混排的东西，
/// 日志里读起来像 bug（2026-10-08 他指出的就是这个）。
#[test]
#[cfg(windows)]
fn managed_kernel_paths_use_native_separators() {
    let text = managed_kernel_root().display().to_string();
    assert!(
        !text.contains("data/kernels"),
        "路径里不该出现正斜杠，看起来是混了两种分隔符: {text}"
    );
    assert!(text.ends_with(r"data\kernels"), "{text}");
}

pub fn managed_kernel_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("data").join("kernels")
}

pub fn parse_version_output(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// 把"跑一下内核拿版本"的结果分类。
///
/// 关键是不能只看输出：VirtualBox 里 mihomo 这类 go 构建可能因为虚拟机没暴露它需要的
/// CPU 指令集而**当场崩掉** —— 那时候进程有退出码、一个字都没打印。旧代码把这种情况
/// 和"打印了但没版本"混成同一句「未返回版本」，于是谁也查不出为什么。
pub fn classify_version_probe(
    exit_ok: bool,
    code: Option<i32>,
    output: &str,
) -> Result<Option<String>, String> {
    if let Some(line) = parse_version_output(output) {
        return Ok(Some(line));
    }
    if exit_ok {
        return Err("内核运行成功但没有任何输出".to_string());
    }
    let raw = code.unwrap_or(-1);
    let unsigned = raw as u32;
    let known = match unsigned {
        0xC000_001D => "STATUS_ILLEGAL_INSTRUCTION（这台机器的 CPU 不提供这个构建用到的指令集，虚拟机常见）",
        0xC000_0005 => "STATUS_ACCESS_VIOLATION（内核刚跑就崩了）",
        0xC000_00FD => "STATUS_STACK_OVERFLOW",
        _ => "",
    };
    let tail = if known.is_empty() {
        String::new()
    } else {
        format!(" —— {known}")
    };
    Err(format!(
        "内核无法执行（退出码 {raw} / 0x{unsigned:08X}），且没有任何输出{tail}。         也可能是被杀软拦掉了。"
    ))
}

/// Go 运行时的 CPU 特性总开关。虚拟机把 CPUID 报给访客、实际却执行不了那些指令时，
/// Go 分发到不支持的路径就会在 `main` 之前崩 —— 形状正是"零输出 + 访问违例"。
/// 本机实测：`GODEBUG=cpu.all=off` 被接受且输出逐字不变（`cpu.avx` 之类键 Go 不认）。
pub const GODEBUG_COMPAT: [(&str, &str); 1] = [("GODEBUG", "cpu.all=off")];

pub async fn read_kernel_version(kind: KernelKind, path: &Path) -> Result<Option<String>> {
    read_kernel_version_with_env(kind, path, &[]).await
}

async fn read_kernel_version_with_env(
    kind: KernelKind,
    path: &Path,
    env: &[(&str, &str)],
) -> Result<Option<String>> {
    let mut command = Command::new(path);
    for (key, value) in env {
        command.env(key, value);
    }
    command.args(kind.version_args());
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
    }
    // 必须带超时：`output()` 会一直等子进程退出，某个内核挂住就会让整条
    // 探测链（以及依赖它的更新检查）永远不返回。
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), command.output())
        .await
        .map_err(|_| anyhow::anyhow!("读取版本超时: {}", path.display()))?
        .with_context(|| format!("执行 {} 失败", path.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let outcome = classify_version_probe(output.status.success(), output.status.code(), &text);
    if let Err(error) = &outcome {
        // 探测失败是"预备状态"最常见的外因落点，日志里要留退出码和原始输出长度。
        tracing::warn!(
            kernel = kind.display_name(),
            path = %path.display(),
            exit_ok = output.status.success(),
            code = ?output.status.code(),
            bytes = text.len(),
            "{error}"
        );
    }
    outcome.map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 只装了 `installed` 里那几个内核时，预检应该说什么。
    fn missing_with(
        protocol: &str,
        preference: KernelPreference,
        system_mode: bool,
        installed: &[KernelKind],
    ) -> Option<KernelKind> {
        missing_kernel_for(protocol, preference, system_mode, |kind| {
            installed.contains(&kind)
        })
    }

    /// 2026-10-08 VirtualBox 上「Mihomo 暂存内核未返回版本」的真相：进程崩了、输出为空，
    /// 而旧代码把退出码丢了。这里把三种情况钉死。
    #[test]
    fn version_probe_reports_why_the_kernel_stayed_silent() {
        // 本机实测的正常输出（mihomo -v，go1.26.8 构建）。
        let good = "Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 17:00:13 UTC 2026
Use tags: with_gvisor
";
        assert_eq!(
            classify_version_probe(true, Some(0), good).unwrap().as_deref(),
            Some("Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 17:00:13 UTC 2026")
        );
        // 崩掉：退出码 -1073741795 就是 0xC000001D，输出为空 —— 要能指名道姓。
        let crashed = classify_version_probe(false, Some(-1073741795), "").unwrap_err();
        assert!(crashed.contains("0xC000001D"), "{crashed}");
        assert!(crashed.contains("STATUS_ILLEGAL_INSTRUCTION"), "{crashed}");
        // 退出码不是已知那几种也得照实报数。
        let other = classify_version_probe(false, Some(-1073741819), "").unwrap_err();
        assert!(other.contains("0xC0000005"), "{other}");
        assert!(
            classify_version_probe(true, Some(0), "
  
").unwrap_err()
                .contains("没有任何输出")
        );
    }

    #[test]
    fn preflight_stays_out_of_the_way_when_a_usable_kernel_exists() {
        assert_eq!(
            missing_with("ss", KernelPreference::Auto, true, &[KernelKind::Mihomo]),
            None
        );
        assert_eq!(
            missing_with(
                "hysteria2",
                KernelPreference::Auto,
                true,
                &[KernelKind::SingBox]
            ),
            None
        );
    }

    #[test]
    fn preflight_points_at_the_protocol_default_when_nothing_is_installed() {
        assert_eq!(
            missing_with("ss", KernelPreference::Auto, true, &[]),
            Some(KernelKind::Mihomo)
        );
        assert_eq!(
            missing_with("hysteria2", KernelPreference::Auto, true, &[]),
            Some(KernelKind::SingBox)
        );
    }

    #[test]
    fn preflight_offers_the_named_kernel_even_if_another_one_would_work() {
        // 用户点名 sing-box，机器上只有 mihomo：不能拿"能跑"糊过去，那不是我选的引擎。
        assert_eq!(
            missing_with(
                "ss",
                KernelPreference::SingBox,
                true,
                &[KernelKind::Mihomo]
            ),
            Some(KernelKind::SingBox)
        );
        assert_eq!(
            missing_with("ss", KernelPreference::Mihomo, true, &[KernelKind::Mihomo]),
            None
        );
    }

    #[test]
    fn preflight_does_not_confuse_unsupported_with_not_installed() {
        // Xray 在 TUN 下是"不支持"，报这个就行，弹下载框反而误导。
        assert_eq!(
            missing_with("ss", KernelPreference::Xray, false, &[]),
            None
        );
        // 不认识的协议同理：该走"暂不支持协议"那条错。
        assert_eq!(missing_with("quantum-tunnel", KernelPreference::Auto, true, &[]), None);
    }

    #[test]
    fn kernel_kind_has_stable_display_names() {
        assert_eq!(KernelKind::Mihomo.display_name(), "Mihomo");
        assert_eq!(KernelKind::Xray.display_name(), "Xray");
        assert_eq!(KernelKind::SingBox.display_name(), "Sing-box");
    }

    #[test]
    fn kernel_candidates_only_include_managed_paths() {
        let executable = PathBuf::from(r"C:\Tools\vpn-parser\vpn-parser.exe");
        let working_dir = PathBuf::from(r"D:\vpn-parser");
        let candidates =
            kernel_candidates(KernelKind::Mihomo, Some(&executable), Some(&working_dir));

        assert_eq!(candidates.len(), 2);
        assert!(candidates.contains(&PathBuf::from(
            r"D:\vpn-parser\data\kernels\mihomo\mihomo.exe"
        )));
        assert!(candidates
            .iter()
            .all(|path| !path.to_string_lossy().contains("v2rayN")));
    }

    #[test]
    fn find_first_existing_candidate_returns_existing_file() {
        let temp_dir =
            std::env::temp_dir().join(format!("vpn-parser-kernel-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let existing = temp_dir.join("mihomo.exe");
        std::fs::write(&existing, b"test").unwrap();

        let result =
            find_first_existing_candidate(vec![temp_dir.join("missing.exe"), existing.clone()]);

        assert_eq!(result, Some(existing));
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn parse_version_from_mihomo_output_keeps_first_nonempty_line() {
        let output = "\nMihomo Meta v1.19.13 windows amd64\nBuildTime: 2026-01-01";

        assert_eq!(
            parse_version_output(output),
            Some("Mihomo Meta v1.19.13 windows amd64".into())
        );
    }

    #[test]
    fn kernel_info_reports_missing_path() {
        let manager = KernelManager;
        let info = manager.info_for_missing(KernelKind::Mihomo);

        assert_eq!(info.kind, KernelKind::Mihomo);
        assert!(!info.available);
        assert!(info.path.is_none());
        assert!(info.version.is_none());
        assert!(info.message.contains("未找到"));
    }

    #[test]
    fn each_kernel_has_version_command_arguments() {
        assert_eq!(KernelKind::Mihomo.version_args(), &["-v"]);
        assert_eq!(KernelKind::Xray.version_args(), &["version"]);
        assert_eq!(KernelKind::SingBox.version_args(), &["version"]);
    }

    #[test]
    fn recognizes_official_archive_executable_names() {
        assert!(KernelKind::Mihomo.matches_archive_executable("mihomo-windows-amd64.exe"));
        assert!(KernelKind::Xray.matches_archive_executable("xray.exe"));
        assert!(KernelKind::SingBox.matches_archive_executable("sing-box.exe"));
        assert!(!KernelKind::Xray.matches_archive_executable("wv2ray.exe"));
    }

    #[test]
    fn proxy_kernel_preferences_are_protocol_specific() {
        for protocol in ["ssr", "hysteria", "vless", "vmess", "trojan", "ss"] {
            assert_eq!(preferred_proxy_kernel(protocol), Some(KernelKind::Mihomo));
        }
        for protocol in ["hysteria2", "hy2", "tuic", "anytls", "wireguard", "wg"] {
            assert_eq!(preferred_proxy_kernel(protocol), Some(KernelKind::SingBox));
        }
        assert_eq!(preferred_proxy_kernel("mieru"), None);
    }

    #[test]
    fn proxy_kernel_selection_falls_back_only_to_compatible_installed_kernel() {
        let sing_box_only = |kind| kind == KernelKind::SingBox;
        assert_eq!(
            select_proxy_kernel_with("vless", sing_box_only).unwrap(),
            KernelKind::SingBox
        );
        assert!(select_proxy_kernel_with("ssr", sing_box_only).is_err());

        let mihomo_only = |kind| kind == KernelKind::Mihomo;
        assert_eq!(
            select_proxy_kernel_with("hysteria2", mihomo_only).unwrap(),
            KernelKind::Mihomo
        );
        assert!(select_proxy_kernel_with("mieru", |_| true).is_err());
    }

    #[test]
    fn xray_support_is_limited_to_system_proxy_protocols() {
        for protocol in ["ss", "vmess", "vless", "trojan", "http", "socks5"] {
            assert!(KernelKind::Xray.supports_proxy_protocol(protocol));
        }
        for protocol in ["hysteria2", "tuic", "anytls", "wireguard"] {
            assert!(!KernelKind::Xray.supports_proxy_protocol(protocol));
        }
    }
}

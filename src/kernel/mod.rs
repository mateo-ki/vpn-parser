use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::process::Command;

use crate::models::Node;

mod updater;

pub use updater::{
    proxy_candidates, source_from_id, KernelReleaseInfo, KernelSource, KernelUpdateProgress,
    KernelUpdateResult, KernelUpdateStage, ASSET_MIRROR_PREFIXES, SOURCE_OPTIONS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelKind {
    Mihomo,
    Xray,
    SingBox,
    Shoes,
    Meow,
    VpCore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KernelPreference {
    #[default]
    Auto,
    Mihomo,
    SingBox,
    Xray,
    Shoes,
    Meow,
    VpCore,
}

impl KernelPreference {
    pub fn kernel(self) -> Option<KernelKind> {
        match self {
            Self::Auto => None,
            Self::Mihomo => Some(KernelKind::Mihomo),
            Self::SingBox => Some(KernelKind::SingBox),
            Self::Xray => Some(KernelKind::Xray),
            Self::Shoes => Some(KernelKind::Shoes),
            Self::Meow => Some(KernelKind::Meow),
            Self::VpCore => Some(KernelKind::VpCore),
        }
    }
}

/// 官方 Release 资产按哪个系统出的。名字/后缀/可执行文件名三样都跟着它变。
///
/// 显式建模成一个值（而不是在每个函数里写 `#[cfg]`）有两个理由：
/// ① 在 Windows 上也能把 Linux 那套命名与解包格式断言跑一遍；
/// ② 兜底版本的 SHA-256 是**按资产**算的，`.zip` 和 `.gz` 两个号不能混用一份常量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum KernelPlatform {
    Windows,
    Linux,
    MacOS,
}

impl KernelPlatform {
    /// 编译目标决定它。非 Windows 非 Linux 的先按 Linux 命名走（没实测过，别当已验证）。
    pub const fn current() -> Self {
        #[cfg(windows)]
        {
            Self::Windows
        }
        #[cfg(all(not(windows), target_os = "macos"))]
        {
            Self::MacOS
        }
        #[cfg(all(not(windows), target_os = "linux"))]
        {
            Self::Linux
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Self::Linux
        }
    }
}

/// 资产容器格式。三家在 Linux 上发的东西并不统一：
/// mihomo 是 gzip 压的裸可执行文件，sing-box 是 `.tar.gz`，xray 才是 `.zip`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelArchive {
    Zip,
    /// gzip 单层流，解出来就是那一个可执行文件。
    Gz,
    TarGz,
}

impl KernelArchive {
    pub fn from_asset_name(asset_name: &str) -> Option<Self> {
        let lower = asset_name.to_ascii_lowercase();
        if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".gz") {
            Some(Self::Gz)
        } else {
            None
        }
    }
}

impl KernelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mihomo => "mihomo",
            Self::Xray => "xray",
            Self::SingBox => "sing_box",
            Self::Shoes => "shoes",
            Self::Meow => "meow",
            Self::VpCore => "vpcore",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Mihomo => "Mihomo",
            Self::Xray => "Xray",
            Self::SingBox => "Sing-box",
            Self::Shoes => "Shoes",
            Self::Meow => "Meow",
            Self::VpCore => "VP-Core",
        }
    }

    pub fn environment_variable(self) -> &'static str {
        match self {
            Self::Mihomo => "MIHOMO_PATH",
            Self::Xray => "XRAY_PATH",
            Self::SingBox => "SING_BOX_PATH",
            Self::Shoes => "SHOES_PATH",
            Self::Meow => "MEOW_PATH",
            Self::VpCore => "VPCORE_PATH",
        }
    }

    pub fn executable_name(self) -> &'static str {
        self.executable_name_for(KernelPlatform::current())
    }

    /// 同一种内核在每个系统上的可执行文件名不一样（Windows 要带 `.exe`）。
    /// 平台做成参数而不是散在函数里的 `cfg`，为的是在 Windows 上也能把 Linux 那套断言跑一遍。
    pub fn executable_name_for(self, platform: KernelPlatform) -> &'static str {
        match (self, platform) {
            (Self::Mihomo, KernelPlatform::Windows) => "mihomo.exe",
            (Self::Xray, KernelPlatform::Windows) => "xray.exe",
            (Self::SingBox, KernelPlatform::Windows) => "sing-box.exe",
            (Self::Shoes, KernelPlatform::Windows) => "shoes.exe",
            (Self::Meow, KernelPlatform::Windows) => "meow.exe",
            (Self::VpCore, KernelPlatform::Windows) => "vpcore.exe",
            (Self::Mihomo, KernelPlatform::Linux | KernelPlatform::MacOS) => "mihomo",
            (Self::Xray, KernelPlatform::Linux | KernelPlatform::MacOS) => "xray",
            (Self::SingBox, KernelPlatform::Linux | KernelPlatform::MacOS) => "sing-box",
            (Self::Shoes, KernelPlatform::Linux | KernelPlatform::MacOS) => "shoes",
            (Self::Meow, KernelPlatform::Linux | KernelPlatform::MacOS) => "meow",
            (Self::VpCore, KernelPlatform::Linux | KernelPlatform::MacOS) => "vpcore",
        }
    }

    pub fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::Mihomo => &["-v"],
            Self::Xray | Self::SingBox => &["version"],
            Self::Shoes => &["-V"],
            Self::Meow | Self::VpCore => &["--version"],
        }
    }

    pub fn directory_name(self) -> &'static str {
        match self {
            Self::Mihomo => "mihomo",
            Self::Xray => "xray",
            Self::SingBox => "sing-box",
            Self::Shoes => "shoes",
            Self::Meow => "meow",
            Self::VpCore => "vpcore",
        }
    }

    pub fn github_repository(self) -> &'static str {
        match self {
            Self::Mihomo => "MetaCubeX/mihomo",
            Self::Xray => "XTLS/Xray-core",
            Self::SingBox => "SagerNet/sing-box",
            Self::Shoes => "mateo-ki/shoes",
            Self::Meow => "meow-rs/meow-rs",
            Self::VpCore => "mateo-ki/vp_core",
        }
    }

    pub fn asset_name(self, tag: &str) -> String {
        self.asset_name_for(KernelPlatform::current(), tag)
    }

    /// 四个内核在各平台上发的容器格式与候选资产列表（首选标准版，备选兼容版 compatible）：
    /// mihomo: Windows (.zip), Linux (.gz), macOS (.gz)
    /// xray: Windows (.zip), Linux (.zip), macOS (.zip)
    /// sing-box: Windows (.zip), Linux (.tar.gz), macOS (.tar.gz)
    /// shoes: Windows (.zip), Linux (.tar.gz), macOS (.tar.gz)
    /// meow: Windows (.zip), Linux (.tar.gz), macOS (.tar.gz)
    pub fn asset_candidates_for(self, platform: KernelPlatform, tag: &str) -> Vec<String> {
        let version = tag.trim_start_matches('v');
        match (self, platform) {
            (Self::Mihomo, KernelPlatform::Windows) => vec![
                format!("mihomo-windows-amd64-{tag}.zip"),
                format!("mihomo-windows-amd64-compatible-{tag}.zip"),
            ],
            (Self::Mihomo, KernelPlatform::Linux) => vec![
                format!("mihomo-linux-amd64-{tag}.gz"),
                format!("mihomo-linux-amd64-compatible-{tag}.gz"),
            ],
            (Self::Mihomo, KernelPlatform::MacOS) => vec![
                format!("mihomo-darwin-amd64-{tag}.gz"),
                format!("mihomo-darwin-amd64-compatible-{tag}.gz"),
            ],
            (Self::Xray, KernelPlatform::Windows) => vec!["Xray-windows-64.zip".into()],
            (Self::Xray, KernelPlatform::Linux) => vec!["Xray-linux-64.zip".into()],
            (Self::Xray, KernelPlatform::MacOS) => vec!["Xray-macos-64.zip".into()],
            (Self::SingBox, KernelPlatform::Windows) => {
                vec![format!("sing-box-{version}-windows-amd64.zip")]
            }
            (Self::SingBox, KernelPlatform::Linux) => {
                vec![format!("sing-box-{version}-linux-amd64.tar.gz")]
            }
            (Self::SingBox, KernelPlatform::MacOS) => {
                vec![format!("sing-box-{version}-darwin-amd64.tar.gz")]
            }
            (Self::Shoes, KernelPlatform::Windows) => {
                vec!["shoes-x86_64-pc-windows-msvc.zip".into()]
            }
            (Self::Shoes, KernelPlatform::Linux) => vec![
                "shoes-x86_64-unknown-linux-gnu.tar.gz".into(),
                "shoes-x86_64-unknown-linux-musl.tar.gz".into(),
            ],
            (Self::Shoes, KernelPlatform::MacOS) => vec![
                "shoes-aarch64-apple-darwin.tar.gz".into(),
                "shoes-x86_64-apple-darwin.tar.gz".into(),
            ],
            (Self::Meow, KernelPlatform::Windows) => {
                vec![format!("meow-{tag}-x86_64-pc-windows-msvc.zip")]
            }
            (Self::Meow, KernelPlatform::Linux) => vec![
                format!("meow-{tag}-x86_64-unknown-linux-gnu.tar.gz"),
                format!("meow-{tag}-x86_64-unknown-linux-musl.tar.gz"),
            ],
            (Self::Meow, KernelPlatform::MacOS) => vec![
                format!("meow-{tag}-aarch64-apple-darwin.tar.gz"),
                format!("meow-{tag}-x86_64-apple-darwin.tar.gz"),
            ],
            (Self::VpCore, KernelPlatform::Windows) => vec![
                format!("vpcore-v{version}-x86_64-pc-windows-msvc.zip"),
                format!("vpcore-{tag}-x86_64-pc-windows-msvc.zip"),
            ],
            (Self::VpCore, KernelPlatform::Linux) => vec![
                format!("vpcore-v{version}-x86_64-unknown-linux-musl.tar.gz"),
                format!("vpcore-{tag}-x86_64-unknown-linux-musl.tar.gz"),
                format!("vpcore-v{version}-aarch64-unknown-linux-musl.tar.gz"),
                format!("vpcore-{tag}-aarch64-unknown-linux-musl.tar.gz"),
            ],
            (Self::VpCore, KernelPlatform::MacOS) => vec![
                format!("vpcore-v{version}-aarch64-apple-darwin.tar.gz"),
                format!("vpcore-v{version}-x86_64-apple-darwin.tar.gz"),
                format!("vpcore-{tag}-aarch64-apple-darwin.tar.gz"),
                format!("vpcore-{tag}-x86_64-apple-darwin.tar.gz"),
            ],
        }
    }

    pub fn asset_name_for(self, platform: KernelPlatform, tag: &str) -> String {
        self.asset_candidates_for(platform, tag)[0].clone()
    }

    pub fn asset_candidates(self, tag: &str) -> Vec<String> {
        self.asset_candidates_for(KernelPlatform::current(), tag)
    }

    pub fn matches_archive_executable(self, file_name: &str) -> bool {
        // 容器里的条目名 Linux 侧没有 `.exe`，所以判"去掉 .exe 之后的主干"。
        // 仍保留 `mihomo*` 这种前缀匹配：官方 Windows 压缩包里的条目名有过带变体后缀的时候。
        let lower = file_name.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        match self {
            Self::Mihomo => stem.starts_with("mihomo"),
            Self::Xray => stem == "xray",
            Self::SingBox => stem == "sing-box",
            Self::Shoes => stem == "shoes",
            Self::Meow => stem == "meow",
            Self::VpCore => stem == "vpcore",
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
                    | "masque"
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
            Self::Shoes => matches!(
                protocol.as_str(),
                "ss" | "shadowsocks"
                    | "vmess"
                    | "vless"
                    | "trojan"
                    | "snell"
                    | "anytls"
                    | "naiveproxy"
                    | "wireguard"
                    | "wg"
                    | "http"
                    | "https"
                    | "socks"
                    | "socks5"
            ),
            Self::Meow => matches!(
                protocol.as_str(),
                "ss" | "shadowsocks"
                    | "vmess"
                    | "vless"
                    | "trojan"
                    | "hysteria2"
                    | "hy2"
                    | "snell"
                    | "anytls"
                    | "http"
                    | "https"
                    | "socks"
                    | "socks5"
            ),
            Self::VpCore => matches!(
                protocol.as_str(),
                "ss" | "shadowsocks"
                    | "ssr"
                    | "shadowsocksr"
                    | "vmess"
                    | "vless"
                    | "trojan"
                    | "snell"
                    | "hysteria2"
                    | "hy2"
                    | "tuic"
                    | "anytls"
                    | "mieru"
                    | "naive"
                    | "naiveproxy"
                    | "ssh"
                    | "brook"
                    | "wireguard"
                    | "wg"
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
        "snell" | "naiveproxy" => Some(KernelKind::Shoes),
        "hysteria2" | "hy2" | "tuic" | "anytls" | "wireguard" | "wg" => Some(KernelKind::SingBox),
        protocol if KernelKind::Mihomo.supports_proxy_protocol(protocol) => {
            Some(KernelKind::Mihomo)
        }
        protocol if KernelKind::Shoes.supports_proxy_protocol(protocol) => Some(KernelKind::Shoes),
        _ => None,
    }
}

pub fn select_proxy_kernel_with(
    protocol: &str,
    available: impl Fn(KernelKind) -> bool,
) -> Result<KernelKind> {
    let preferred = preferred_proxy_kernel(protocol);
    let fallback = match preferred {
        Some(KernelKind::Mihomo) => Some(KernelKind::SingBox),
        Some(KernelKind::SingBox) => Some(KernelKind::Mihomo),
        Some(KernelKind::Shoes) => Some(KernelKind::Mihomo),
        Some(KernelKind::Meow) => Some(KernelKind::Mihomo),
        Some(KernelKind::VpCore) => Some(KernelKind::Mihomo),
        Some(KernelKind::Xray) => unreachable!(),
        None => None,
    };
    let mut candidates: Vec<KernelKind> = preferred.into_iter().collect();
    candidates.extend(fallback);
    candidates.extend([
        KernelKind::VpCore,
        KernelKind::Meow,
        KernelKind::Shoes,
        KernelKind::Xray,
    ]);
    for candidate in candidates {
        if candidate.supports_proxy_protocol(protocol) && available(candidate) {
            return Ok(candidate);
        }
    }
    Err(anyhow::anyhow!("没有已安装且支持 {protocol} 的代理内核"))
}

pub fn select_proxy_kernel(protocol: &str) -> Result<KernelKind> {
    select_proxy_kernel_with(protocol, |kind| find_kernel_executable(kind).is_some())
}

/// 检测特定节点或运行模式是否与 Meow 内核存在已知不兼容/致命缺陷（如 SplitHTTP、ECH、Windows TUN 网卡环路）
pub fn is_node_incompatible_with_meow(node: &Node, has_tun: bool) -> Option<&'static str> {
    if cfg!(windows) && has_tun {
        return Some("Windows TUN 模式（Meow 存在虚拟网卡环路缺陷）");
    }
    let proto = node.protocol.to_ascii_lowercase();
    if matches!(
        proto.as_str(),
        "wireguard" | "wg" | "tuic" | "ssr" | "shadowsocksr" | "masque"
    ) {
        return Some("WireGuard/TUIC/SSR 协议（Meow 暂不支持）");
    }
    if let Some(net) = node.config.get("network").and_then(|v| v.as_str()) {
        let net_low = net.to_ascii_lowercase();
        if net_low.contains("xhttp") || net_low.contains("splithttp") {
            return Some("XHTTP/SplitHTTP 传输层（Meow 暂不支持）");
        }
        if net_low == "grpc" {
            return Some("gRPC 传输层（Meow 暂不支持）");
        }
    }
    let raw_low = node.raw.to_ascii_lowercase();
    if raw_low.contains("type=xhttp") || raw_low.contains("type=splithttp") {
        return Some("XHTTP/SplitHTTP 传输层（Meow 暂不支持）");
    }
    if raw_low.contains("type=grpc") {
        return Some("gRPC 传输层（Meow 暂不支持）");
    }
    if node.config.get("ech").is_some() || raw_low.contains("ech=") {
        return Some("Cloudflare ECH 加密握手（Meow 暂不支持）");
    }
    None
}

/// 检测特定节点是否与 VP-Core 内核不兼容（内核遇到不支持的传输层会拒绝启动，
/// 必须在选内核阶段就回退，不能等 vpcore 进程退出码 1）。
pub fn is_node_incompatible_with_vpcore(node: &Node) -> Option<&'static str> {
    let proto = node.protocol.to_ascii_lowercase();
    if matches!(proto.as_str(), "masque" | "hysteria" | "hy1") {
        return Some("MASQUE/Hysteria 协议（VP-Core 暂不支持）");
    }
    let mut nets: Vec<String> = Vec::new();
    if let Some(net) = node.config.get("network").and_then(|v| v.as_str()) {
        nets.push(net.to_ascii_lowercase());
    }
    let raw_low = node.raw.to_ascii_lowercase();
    let mut rest = raw_low.as_str();
    while let Some(pos) = rest.find("network=") {
        let after = &rest[pos + "network=".len()..];
        let value: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if !value.is_empty() {
            nets.push(value);
        }
        rest = after;
    }
    for net in &nets {
        match net.as_str() {
            "xhttp" | "splithttp" => return Some("XHTTP/SplitHTTP 传输层（VP-Core 暂不支持）"),
            "httpupgrade" => return Some("HTTPUpgrade 传输层（VP-Core 暂不支持）"),
            "h2" => return Some("H2 传输层（VP-Core 暂不支持）"),
            _ => {}
        }
    }
    None
}

pub fn select_proxy_kernel_preference_with(
    protocol: &str,
    preference: KernelPreference,
    allow_xray: bool,
    available: impl Fn(KernelKind) -> bool,
) -> Result<KernelKind> {
    if let Some(kind) = preference.kernel() {
        if (kind == KernelKind::Xray || kind == KernelKind::Shoes) && !allow_xray {
            // 当前内核不支持 TUN 模式，遇到不支持的模式默认退回 Mihomo
            if available(KernelKind::Mihomo) && KernelKind::Mihomo.supports_proxy_protocol(protocol)
            {
                tracing::warn!(kernel = %kind.display_name(), "当前内核不支持 TUN 模式，默认回退到 Mihomo");
                return Ok(KernelKind::Mihomo);
            }
            return Err(anyhow::anyhow!("{} 暂不支持 TUN 模式", kind.display_name()));
        }
        if !kind.supports_proxy_protocol(protocol) {
            // 遇到不支持的协议默认退回 Mihomo
            if available(KernelKind::Mihomo) && KernelKind::Mihomo.supports_proxy_protocol(protocol)
            {
                tracing::warn!(kernel = %kind.display_name(), protocol, "当前内核不支持该协议，默认回退到 Mihomo");
                return Ok(KernelKind::Mihomo);
            }
            if available(KernelKind::SingBox)
                && KernelKind::SingBox.supports_proxy_protocol(protocol)
            {
                tracing::warn!(kernel = %kind.display_name(), protocol, "当前内核不支持该协议，回退到 Sing-box");
                return Ok(KernelKind::SingBox);
            }
            return Err(anyhow::anyhow!(
                "{} 不支持协议: {protocol}",
                kind.display_name()
            ));
        }
        if !available(kind) {
            // 点名要的内核没安装时，如果有 Mihomo 且支持该协议，默认退回 Mihomo
            if available(KernelKind::Mihomo) && KernelKind::Mihomo.supports_proxy_protocol(protocol)
            {
                tracing::warn!(kernel = %kind.display_name(), "未找到该内核可执行文件，默认回退到 Mihomo");
                return Ok(KernelKind::Mihomo);
            }
            return Err(anyhow::anyhow!("未找到 {} 可执行文件", kind.display_name()));
        }
        return Ok(kind);
    }

    if let Ok(kind) = select_proxy_kernel_with(protocol, &available) {
        return Ok(kind);
    }
    if allow_xray {
        for fallback_kind in [KernelKind::Meow, KernelKind::Shoes, KernelKind::Xray] {
            if fallback_kind.supports_proxy_protocol(protocol) && available(fallback_kind) {
                return Ok(fallback_kind);
            }
        }
    }
    // 默认退回 Mihomo
    if available(KernelKind::Mihomo) && KernelKind::Mihomo.supports_proxy_protocol(protocol) {
        return Ok(KernelKind::Mihomo);
    }
    Err(anyhow::anyhow!("没有已安装且支持 {protocol} 的代理内核"))
}

#[allow(dead_code)]
pub fn select_proxy_kernel_preference(
    protocol: &str,
    preference: KernelPreference,
    allow_xray: bool,
) -> Result<KernelKind> {
    select_proxy_kernel_preference_with(protocol, preference, allow_xray, |kind| {
        find_kernel_executable(kind).is_some()
    })
}

pub fn select_proxy_kernel_preference_for_node_with(
    node: &Node,
    preference: KernelPreference,
    allow_xray: bool,
    has_tun: bool,
    available: impl Fn(KernelKind) -> bool,
) -> Result<(KernelKind, Option<String>)> {
    if let Some(kind) = preference.kernel() {
        // 1. Meow 特殊缺陷排查（Windows TUN 环路、XHTTP/SplitHTTP、ECH 等）
        if kind == KernelKind::Meow {
            if let Some(reason) = is_node_incompatible_with_meow(node, has_tun) {
                tracing::warn!(
                    reason,
                    "Meow 内核与当前节点/模式不兼容，自动平滑回退至 Mihomo"
                );
                if available(KernelKind::Mihomo)
                    && KernelKind::Mihomo.supports_proxy_protocol(&node.protocol)
                {
                    let notice = format!("检测到{reason}，已默认平滑回退至 Mihomo 运行");
                    return Ok((KernelKind::Mihomo, Some(notice)));
                } else if available(KernelKind::SingBox)
                    && KernelKind::SingBox.supports_proxy_protocol(&node.protocol)
                {
                    let notice = format!("检测到{reason}，已自动平滑回退至 Sing-box 运行");
                    return Ok((KernelKind::SingBox, Some(notice)));
                }
                return Err(anyhow::anyhow!(
                    "Meow 暂不支持此配置（{reason}），且未安装可用的回退内核（如 Mihomo）"
                ));
            }
        }

        // 1.5 VP-Core 不支持的传输层：vpcore 会因单节点配置解析失败而整进程退出
        if kind == KernelKind::VpCore {
            if let Some(reason) = is_node_incompatible_with_vpcore(node) {
                tracing::warn!(reason, "VP-Core 与当前节点不兼容，自动平滑回退至 Mihomo");
                if available(KernelKind::Mihomo)
                    && KernelKind::Mihomo.supports_proxy_protocol(&node.protocol)
                {
                    let notice = format!("检测到{reason}，已默认平滑回退至 Mihomo 运行");
                    return Ok((KernelKind::Mihomo, Some(notice)));
                } else if available(KernelKind::SingBox)
                    && KernelKind::SingBox.supports_proxy_protocol(&node.protocol)
                {
                    let notice = format!("检测到{reason}，已自动平滑回退至 Sing-box 运行");
                    return Ok((KernelKind::SingBox, Some(notice)));
                }
                return Err(anyhow::anyhow!(
                    "VP-Core 暂不支持此配置（{reason}），且未安装可用的回退内核（如 Mihomo）"
                ));
            }
        }

        // 2. 遇到不支持的协议默认退回 Mihomo
        if !kind.supports_proxy_protocol(&node.protocol) {
            tracing::warn!(
                kernel = %kind.display_name(),
                protocol = %node.protocol,
                "当前指定内核不支持该协议，默认平滑回退至 Mihomo"
            );
            if available(KernelKind::Mihomo)
                && KernelKind::Mihomo.supports_proxy_protocol(&node.protocol)
            {
                let notice = format!(
                    "{} 不支持 {} 协议，已默认平滑回退至 Mihomo 运行",
                    kind.display_name(),
                    node.protocol
                );
                return Ok((KernelKind::Mihomo, Some(notice)));
            } else if available(KernelKind::SingBox)
                && KernelKind::SingBox.supports_proxy_protocol(&node.protocol)
            {
                let notice = format!(
                    "{} 不支持 {} 协议，已自动平滑回退至 Sing-box 运行",
                    kind.display_name(),
                    node.protocol
                );
                return Ok((KernelKind::SingBox, Some(notice)));
            }
            return Err(anyhow::anyhow!(
                "{} 不支持协议 {}，且未安装支持该协议的回退内核（如 Mihomo）",
                kind.display_name(),
                node.protocol
            ));
        }

        // 3. TUN 模式不支持时默认退回 Mihomo
        if (kind == KernelKind::Xray || kind == KernelKind::Shoes) && has_tun {
            tracing::warn!(
                kernel = %kind.display_name(),
                "当前内核不支持 TUN 模式，默认平滑回退至 Mihomo"
            );
            if available(KernelKind::Mihomo)
                && KernelKind::Mihomo.supports_proxy_protocol(&node.protocol)
            {
                let notice = format!(
                    "{} 暂不支持 TUN 模式，已默认平滑回退至 Mihomo 运行",
                    kind.display_name()
                );
                return Ok((KernelKind::Mihomo, Some(notice)));
            }
            return Err(anyhow::anyhow!("{} 暂不支持 TUN 模式", kind.display_name()));
        }
    }

    let kind =
        select_proxy_kernel_preference_with(&node.protocol, preference, allow_xray, &available)?;
    if kind == KernelKind::Meow {
        if let Some(reason) = is_node_incompatible_with_meow(node, has_tun) {
            tracing::warn!(
                reason,
                "Meow 内核与当前节点/模式不兼容，自动平滑回退至 Mihomo"
            );
            for fallback in [KernelKind::Mihomo, KernelKind::SingBox] {
                if fallback.supports_proxy_protocol(&node.protocol) && available(fallback) {
                    let notice = format!(
                        "检测到{reason}，已默认平滑回退至 {} 运行",
                        fallback.display_name()
                    );
                    return Ok((fallback, Some(notice)));
                }
            }
            return Err(anyhow::anyhow!(
                "Meow 暂不支持此配置（{reason}），且未安装可用的回退内核（如 Mihomo）"
            ));
        }
    }
    if kind == KernelKind::VpCore {
        if let Some(reason) = is_node_incompatible_with_vpcore(node) {
            tracing::warn!(
                reason,
                "VP-Core 内核与当前节点不兼容，自动平滑回退至 Mihomo"
            );
            for fallback in [KernelKind::Mihomo, KernelKind::SingBox] {
                if fallback.supports_proxy_protocol(&node.protocol) && available(fallback) {
                    let notice = format!(
                        "检测到{reason}，已默认平滑回退至 {} 运行",
                        fallback.display_name()
                    );
                    return Ok((fallback, Some(notice)));
                }
            }
            return Err(anyhow::anyhow!(
                "VP-Core 暂不支持此配置（{reason}），且未安装可用的回退内核（如 Mihomo）"
            ));
        }
    }
    Ok((kind, None))
}

pub fn select_proxy_kernel_preference_for_node(
    node: &Node,
    preference: KernelPreference,
    allow_xray: bool,
    has_tun: bool,
) -> Result<(KernelKind, Option<String>)> {
    select_proxy_kernel_preference_for_node_with(node, preference, allow_xray, has_tun, |kind| {
        find_kernel_executable(kind).is_some()
    })
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
        // Xray/Shoes 在 TUN 下是"不支持"，不是"没装"，别拿下载框去挡。
        if (kind == KernelKind::Xray || kind == KernelKind::Shoes) && !system_mode {
            return None;
        }
        // 点名内核不支持该协议时：如果已有已安装内核支持该协议，不弹下载框；否则默认退回引导安装 Mihomo
        if !kind.supports_proxy_protocol(protocol) {
            if select_proxy_kernel_with(protocol, &available).is_ok() {
                return None;
            }
            return (!available(KernelKind::Mihomo)).then_some(KernelKind::Mihomo);
        }
        return (!available(kind)).then_some(kind);
    }
    if select_proxy_kernel_with(protocol, &available).is_ok() {
        return None;
    }
    preferred_proxy_kernel(protocol).filter(|kind| kind.supports_proxy_protocol(protocol))
}

#[allow(dead_code)]
pub fn missing_kernel_for_node(
    node: &Node,
    preference: KernelPreference,
    system_mode: bool,
    has_tun: bool,
    available: impl Fn(KernelKind) -> bool,
) -> Option<KernelKind> {
    if preference == KernelPreference::Meow
        && is_node_incompatible_with_meow(node, has_tun).is_some()
    {
        return (!available(KernelKind::Mihomo)).then_some(KernelKind::Mihomo);
    }
    missing_kernel_for(&node.protocol, preference, system_mode, available)
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
        let (mihomo, xray, sing_box, shoes, meow, vp_core) = tokio::join!(
            self.detect(KernelKind::Mihomo),
            self.detect(KernelKind::Xray),
            self.detect(KernelKind::SingBox),
            self.detect(KernelKind::Shoes),
            self.detect(KernelKind::Meow),
            self.detect(KernelKind::VpCore)
        );
        vec![mihomo, xray, sing_box, shoes, meow, vp_core]
    }

    pub async fn check_updates_with_proxy(
        &self,
        proxy_url: Option<String>,
    ) -> Result<Vec<KernelReleaseInfo>> {
        let updater = updater::KernelUpdater::new(managed_kernel_root(), proxy_url.as_deref())?;
        let infos = self.detect_all().await;
        let (mihomo, xray, sing_box, shoes, meow, vp_core) = tokio::join!(
            updater.check(&infos[0]),
            updater.check(&infos[1]),
            updater.check(&infos[2]),
            updater.check(&infos[3]),
            updater.check(&infos[4]),
            updater.check(&infos[5])
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
            fallback(shoes, &infos[3]),
            fallback(meow, &infos[4]),
            fallback(vp_core, &infos[5]),
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
                .join("data")
                .join("kernels")
                .join(kind.directory_name())
                .join(kind.executable_name()),
        );
    }
    if let Some(executable) = current_exe {
        if let Some(directory) = executable.parent() {
            candidates.push(
                directory
                    .join("data")
                    .join("kernels")
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

pub fn managed_kernel_path(kind: KernelKind) -> PathBuf {
    managed_kernel_root()
        .join(kind.directory_name())
        .join(kind.executable_name())
}

pub fn find_kernel_executable(kind: KernelKind) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(kind.environment_variable()).map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let managed = managed_kernel_path(kind);
    if managed.is_file() {
        return Some(managed);
    }

    let current_exe = std::env::current_exe().ok();
    let current_dir = std::env::current_dir().ok();
    if let Some(candidate) = find_first_existing_candidate(kernel_candidates(
        kind,
        current_exe.as_deref(),
        current_dir.as_deref(),
    )) {
        return Some(candidate);
    }

    #[cfg(unix)]
    {
        for prefix in ["/usr/bin", "/usr/local/bin", "/usr/sbin", "/bin"] {
            let p = PathBuf::from(prefix).join(kind.executable_name());
            if p.is_file() {
                return Some(p);
            }
        }
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let p = dir.join(kind.executable_name());
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }

    None
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
    // 数据根目录只有一份判据（见 `crate::data_root`）：库、分片库、内核必须同一个根，
    // 否则 Linux 上会出现"库在 XDG、内核在 /usr/bin/data"这种一半装上的状态。
    crate::data_root().join("kernels")
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
    if exit_ok {
        if let Some(line) = parse_version_output(output) {
            return Ok(Some(line));
        }
        return Err("内核运行成功但没有任何输出".to_string());
    }
    let raw = code.unwrap_or(-1);
    let unsigned = raw as u32;
    let known = match unsigned {
        0xC000_001D => {
            "STATUS_ILLEGAL_INSTRUCTION（这台机器的 CPU 不提供这个构建用到的指令集，虚拟机常见）"
        }
        0xC000_0005 => "STATUS_ACCESS_VIOLATION（内核刚跑就崩了）",
        0xC000_00FD => "STATUS_STACK_OVERFLOW",
        _ => "",
    };
    let tail = if known.is_empty() {
        String::new()
    } else {
        format!(" —— {known}")
    };
    let detail = output.trim();
    if detail.is_empty() {
        Err(format!(
            "内核无法执行（退出码 {raw} / 0x{unsigned:08X}），且没有任何输出{tail}。         也可能是被杀软拦掉了。"
        ))
    } else {
        Err(format!(
            "内核无法执行（退出码 {raw} / 0x{unsigned:08X}）{tail}：{detail}"
        ))
    }
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
    // 在虚拟机 (VirtualBox) 或 Windows Defender 开启时，首次拉起全新 exe 会被 Defender
    // 实时查毒扫描 5~10 秒，因此探测超时放宽到 15 秒，避免误杀首次启动。
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| anyhow::anyhow!("读取版本超时 (15s): {}", path.display()))?
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
            classify_version_probe(true, Some(0), good)
                .unwrap()
                .as_deref(),
            Some("Mihomo Meta v1.19.32 windows amd64 with go1.26.8 Wed Sep 30 17:00:13 UTC 2026")
        );
        // 崩掉：退出码 -1073741795 就是 0xC000001D，输出为空 —— 要能指名道姓。
        let crashed = classify_version_probe(false, Some(-1073741795), "").unwrap_err();
        assert!(crashed.contains("0xC000001D"), "{crashed}");
        assert!(crashed.contains("STATUS_ILLEGAL_INSTRUCTION"), "{crashed}");
        // 退出码不是已知那几种也得照实报数。
        let other = classify_version_probe(false, Some(-1073741819), "").unwrap_err();
        assert!(other.contains("0xC0000005"), "{other}");
        assert!(classify_version_probe(
            true,
            Some(0),
            "
  
"
        )
        .unwrap_err()
        .contains("没有任何输出"));
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
        assert_eq!(
            missing_with("snell", KernelPreference::Auto, true, &[]),
            Some(KernelKind::Shoes)
        );
    }

    #[test]
    fn preflight_offers_the_named_kernel_even_if_another_one_would_work() {
        // 用户点名 sing-box，机器上只有 mihomo：不能拿"能跑"糊过去，那不是我选的引擎。
        assert_eq!(
            missing_with("ss", KernelPreference::SingBox, true, &[KernelKind::Mihomo]),
            Some(KernelKind::SingBox)
        );
        assert_eq!(
            missing_with("ss", KernelPreference::Shoes, true, &[KernelKind::Mihomo]),
            Some(KernelKind::Shoes)
        );
        assert_eq!(
            missing_with("ss", KernelPreference::Mihomo, true, &[KernelKind::Mihomo]),
            None
        );
        assert_eq!(
            missing_with("ss", KernelPreference::Shoes, true, &[KernelKind::Shoes]),
            None
        );
    }

    #[test]
    fn preflight_does_not_confuse_unsupported_with_not_installed() {
        // Xray 和 Shoes 在 TUN 下是"不支持"，报这个就行，弹下载框反而误导。
        assert_eq!(missing_with("ss", KernelPreference::Xray, false, &[]), None);
        assert_eq!(
            missing_with("ss", KernelPreference::Shoes, false, &[]),
            None
        );
        // 不认识的协议同理：该走"暂不支持协议"那条错。
        assert_eq!(
            missing_with("quantum-tunnel", KernelPreference::Auto, true, &[]),
            None
        );
    }

    #[test]
    fn kernel_kind_has_stable_display_names() {
        assert_eq!(KernelKind::Mihomo.display_name(), "Mihomo");
        assert_eq!(KernelKind::Xray.display_name(), "Xray");
        assert_eq!(KernelKind::SingBox.display_name(), "Sing-box");
        assert_eq!(KernelKind::Shoes.display_name(), "Shoes");
        assert_eq!(KernelKind::Meow.display_name(), "Meow");
        assert_eq!(KernelKind::VpCore.display_name(), "VP-Core");
    }

    #[test]
    fn kernel_candidates_only_include_managed_paths() {
        // 用当前平台的原生路径写法构造输入：反斜杠在 Linux 上不是分隔符，
        // 拿 Windows 字面量去断言 join 结果，测的其实不是这段代码的性质。
        let executable = PathBuf::from("some").join("tools").join("vpn-parser");
        let working_dir = PathBuf::from("somewhere").join("vpn-parser");
        let candidates = kernel_candidates(
            KernelKind::Mihomo,
            Some(&executable.as_path()),
            Some(&working_dir.as_path()),
        );

        assert_eq!(candidates.len(), 2);
        let expected_from_cwd = working_dir
            .join("data")
            .join("kernels")
            .join("mihomo")
            .join(KernelKind::Mihomo.executable_name());
        let expected_from_exe_dir = executable
            .parent()
            .unwrap()
            .join("data")
            .join("kernels")
            .join("mihomo")
            .join(KernelKind::Mihomo.executable_name());
        assert!(candidates.contains(&expected_from_cwd), "{candidates:?}");
        assert!(
            candidates.contains(&expected_from_exe_dir),
            "{candidates:?}"
        );
        assert!(
            candidates
                .iter()
                .all(|path| !path.to_string_lossy().contains("v2rayN")),
            "自动发现不许伸进别人的目录：{candidates:?}"
        );

        // 只给其中一个输入时就只该有一个候选（另一个来源不该被编出来）。
        assert_eq!(
            kernel_candidates(KernelKind::Xray, Some(executable.as_path()), None).len(),
            1
        );
        assert!(kernel_candidates(KernelKind::Xray, None, None).is_empty());
    }

    #[test]
    fn managed_kernel_path_points_inside_managed_kernel_root() {
        let path = managed_kernel_path(KernelKind::Mihomo);
        assert!(path.starts_with(managed_kernel_root()));
        assert!(path.ends_with(KernelKind::Mihomo.executable_name()));
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

        let shoes_info = manager.info_for_missing(KernelKind::Shoes);
        assert_eq!(shoes_info.kind, KernelKind::Shoes);
        assert!(!shoes_info.available);
        assert!(shoes_info.message.contains("Shoes 未找到"));
    }

    #[test]
    fn each_kernel_has_version_command_arguments() {
        assert_eq!(KernelKind::Mihomo.version_args(), &["-v"]);
        assert_eq!(KernelKind::Xray.version_args(), &["version"]);
        assert_eq!(KernelKind::SingBox.version_args(), &["version"]);
        assert_eq!(KernelKind::Shoes.version_args(), &["-V"]);
        assert_eq!(KernelKind::Meow.version_args(), &["--version"]);
        assert_eq!(KernelKind::VpCore.version_args(), &["--version"]);
    }

    #[test]
    fn recognizes_official_archive_executable_names() {
        assert!(KernelKind::Mihomo.matches_archive_executable("mihomo-windows-amd64.exe"));
        assert!(KernelKind::Xray.matches_archive_executable("xray.exe"));
        assert!(KernelKind::SingBox.matches_archive_executable("sing-box.exe"));
        assert!(KernelKind::Shoes.matches_archive_executable("shoes.exe"));
        assert!(KernelKind::Meow.matches_archive_executable("meow.exe"));
        assert!(KernelKind::VpCore.matches_archive_executable("vpcore.exe"));
        assert!(!KernelKind::Xray.matches_archive_executable("wv2ray.exe"));
    }

    #[test]
    fn test_vp_core_supports_mainstream_protocols() {
        for protocol in [
            "ss",
            "ssr",
            "vmess",
            "vless",
            "trojan",
            "snell",
            "hysteria2",
            "tuic",
            "anytls",
            "wireguard",
            "socks5",
            "http",
        ] {
            assert!(
                KernelKind::VpCore.supports_proxy_protocol(protocol),
                "vp_core should support {protocol}"
            );
        }
    }

    #[test]
    fn proxy_kernel_preferences_are_protocol_specific() {
        for protocol in ["ssr", "hysteria", "vless", "vmess", "trojan", "ss"] {
            assert_eq!(preferred_proxy_kernel(protocol), Some(KernelKind::Mihomo));
        }
        for protocol in ["hysteria2", "hy2", "tuic", "anytls", "wireguard", "wg"] {
            assert_eq!(preferred_proxy_kernel(protocol), Some(KernelKind::SingBox));
        }
        for protocol in ["snell", "naiveproxy"] {
            assert_eq!(preferred_proxy_kernel(protocol), Some(KernelKind::Shoes));
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
        // mieru 没有首选内核，但 VP-Core 支持时应选中 VP-Core
        assert_eq!(
            select_proxy_kernel_with("mieru", |_| true).unwrap(),
            KernelKind::VpCore
        );
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

    #[test]
    fn macos_kernel_assets_and_executables() {
        assert_eq!(
            KernelKind::Mihomo.executable_name_for(KernelPlatform::MacOS),
            "mihomo"
        );
        assert_eq!(
            KernelKind::Xray.executable_name_for(KernelPlatform::MacOS),
            "xray"
        );
        assert_eq!(
            KernelKind::SingBox.executable_name_for(KernelPlatform::MacOS),
            "sing-box"
        );
        assert_eq!(
            KernelKind::Shoes.executable_name_for(KernelPlatform::MacOS),
            "shoes"
        );
        assert_eq!(
            KernelKind::VpCore.executable_name_for(KernelPlatform::MacOS),
            "vpcore"
        );
        assert_eq!(
            KernelKind::Mihomo.asset_name_for(KernelPlatform::MacOS, "v1.18.0"),
            "mihomo-darwin-amd64-v1.18.0.gz"
        );
        assert_eq!(
            KernelKind::Xray.asset_name_for(KernelPlatform::MacOS, "v1.8.4"),
            "Xray-macos-64.zip"
        );
        assert_eq!(
            KernelKind::SingBox.asset_name_for(KernelPlatform::MacOS, "v1.8.0"),
            "sing-box-1.8.0-darwin-amd64.tar.gz"
        );
        assert_eq!(
            KernelKind::Shoes.asset_name_for(KernelPlatform::MacOS, "v0.3.3"),
            "shoes-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            KernelKind::VpCore.asset_name_for(KernelPlatform::MacOS, "v1.0.0"),
            "vpcore-v1.0.0-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn test_meow_incompatibility_detection_and_fallback() {
        let xhttp_node = Node {
            id: "test-node-xhttp".into(),
            source_id: None,
            protocol: "vless".into(),
            name: "VLESS XHTTP Node".into(),
            server: "example.com".into(),
            port: 443,
            raw: "vless://uuid@example.com:443?type=xhttp#VLESS%20XHTTP".into(),
            config: serde_json::json!({
                "network": "xhttp",
            }),
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        // 1. xhttp should be incompatible with Meow
        let incomp = is_node_incompatible_with_meow(&xhttp_node, false);
        assert!(
            incomp.is_some(),
            "xhttp should be flagged as incompatible with Meow"
        );
        assert!(incomp.unwrap().contains("XHTTP"));

        // 2. Fallback to Mihomo if Mihomo is available
        let available = |k| k == KernelKind::Meow || k == KernelKind::Mihomo;
        let (selected, notice) = select_proxy_kernel_preference_for_node_with(
            &xhttp_node,
            KernelPreference::Meow,
            true,
            false,
            available,
        )
        .expect("Should smoothly fallback to Mihomo");
        assert_eq!(selected, KernelKind::Mihomo);
        assert!(notice.is_some());
        assert!(notice.unwrap().contains("Mihomo"));

        // 3. Normal VMess node without xhttp or ECH in System mode should be fine with Meow
        let normal_node = Node {
            id: "test-node-ws".into(),
            source_id: None,
            protocol: "vmess".into(),
            name: "VMess WS Node".into(),
            server: "example.com".into(),
            port: 443,
            raw: "vmess://uuid@example.com:443?type=ws".into(),
            config: serde_json::json!({
                "network": "ws",
            }),
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        assert_eq!(is_node_incompatible_with_meow(&normal_node, false), None);

        // 4. Windows TUN mode should be flagged as incompatible due to Wintun loop defect
        if cfg!(windows) {
            let tun_incomp = is_node_incompatible_with_meow(&normal_node, true);
            assert!(tun_incomp.is_some());
            assert!(tun_incomp.unwrap().contains("TUN"));
        }
    }

    #[test]
    fn test_unsupported_protocol_defaults_to_mihomo() {
        let available =
            |k| k == KernelKind::Xray || k == KernelKind::Mihomo || k == KernelKind::Shoes;

        // 1. 用户指定 Xray，但节点为 wireguard（Xray 并不支持），默认退回 Mihomo
        let wg_node = Node {
            id: "wg-node".into(),
            source_id: None,
            protocol: "wireguard".into(),
            name: "WireGuard Node".into(),
            server: "1.2.3.4".into(),
            port: 51820,
            raw: "wireguard://...".into(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let (res_kernel, notice) = select_proxy_kernel_preference_for_node_with(
            &wg_node,
            KernelPreference::Xray,
            true,
            false,
            available,
        )
        .expect("遇到不支持的协议默认退回 Mihomo");
        assert_eq!(res_kernel, KernelKind::Mihomo);
        assert!(notice.unwrap().contains("Mihomo"));

        // 2. 用户指定 Shoes，但节点为 hysteria2（Shoes 不支持 hysteria2），默认退回 Mihomo
        let hy2_node = Node {
            id: "hy2-node".into(),
            source_id: None,
            protocol: "hysteria2".into(),
            name: "Hysteria2 Node".into(),
            server: "1.2.3.4".into(),
            port: 443,
            raw: "hysteria2://...".into(),
            config: serde_json::json!({}),
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let (shoes_res, shoes_notice) = select_proxy_kernel_preference_for_node_with(
            &hy2_node,
            KernelPreference::Shoes,
            true,
            false,
            available,
        )
        .expect("遇到不支持的协议默认退回 Mihomo");
        assert_eq!(shoes_res, KernelKind::Mihomo);
        assert!(shoes_notice.unwrap().contains("Mihomo"));
    }
}

//! 应用自身的更新：查 GitHub Release → 下载安装包 → 校验 SHA-256 → 交给安装向导。
//!
//! 内核更新走 [`crate::kernel`]，那条路可以把下好的字节直接换进 `data/kernels/`；
//! 应用自己不行 —— `vpn-parser.exe` 正在运行，Windows 不允许覆盖它。所以这里只负责
//! "把安装包取回来并证明它没被改过"，落地交给 Inno 向导（它会先 taskkill 本进程再装）。

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// 安装包发布在这儿，由 `.github/workflows/release.yml`（Windows）和
/// `.github/workflows/build-linux.yml`（deb / rpm）用同一个 tag 打出来。
pub const REPOSITORY: &str = "mateo-ki/vpn-parser";

/// 一种平台的安装包形态。资产名规则各不同，所以"挑哪个包"和"怎么装"都得跟着它走。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PackageKind {
    WindowsSetup,
    Deb,
    Rpm,
    AppImage,
}

impl PackageKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::WindowsSetup => "Windows 安装包",
            Self::Deb => "deb 包",
            Self::Rpm => "rpm 包",
            Self::AppImage => "AppImage 单文件",
        }
    }

    /// 资产名的尾巴。deb/rpm 的名字由打包工具生成（`vpn-parser_1.6.2-1_amd64.deb`、
    /// `vpn-parser-1.6.2-1.x86_64.rpm`），**修订号和架构是它们自己填的**，
    /// 所以只能按"前缀 + 尾巴"认，不能像 Windows 那样比完整名。
    /// 松匹配要不要逼名字里带版本号（见 `pick_one_kind`）。
    fn needs_version_in_name(self) -> bool {
        self != Self::WindowsSetup
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::WindowsSetup => "-windows-x64-setup.exe",
            Self::Deb => ".deb",
            Self::Rpm => ".rpm",
            Self::AppImage => ".AppImage",
        }
    }

    /// 该用什么命令装。Linux 侧没有任何免密路径：dpkg/rpm 都要 root，
    /// 而"以用户身份悄悄换掉系统里的程序"不是我想给这个软件开的能力。
    #[allow(dead_code)]
    pub fn install_command(self, path: &Path) -> String {
        let file = path.display();
        match self {
            Self::WindowsSetup => format!("双击运行 {file}（安装向导会先关掉当前程序）"),
            Self::Deb => format!("sudo dpkg -i {file}"),
            Self::Rpm => format!("sudo rpm -Uvh {file}"),
            Self::AppImage => {
                if let Some(orig) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
                    format!("chmod +x {file} && 直接运行新版或替换原文件：{}", orig.display())
                } else {
                    format!("chmod +x {file} && {file}")
                }
            }
        }
    }
}

const HASH_INDEX_NAME: &str = "sha256.txt";
/// 安装包现在 ~15 MB，留 200 MB 上限只为挡住"镜像回了一个 HTML 或者整个盘"。
const MAX_INSTALLER_SIZE: u64 = 200 * 1024 * 1024;

pub fn current_version() -> &'static str {
    // 诊断壳下允许假装自己是老版本 —— "有没有新版"这条链路（含侧栏那个绿色↑）
    // 只有这样才能在真机上跑通一遍，否则本机版本永远等于最新 Release，永远看不到亮起来。
    if let Some(fake) = crate::diag::forced_version() {
        static FAKE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
        return *FAKE.get_or_init(|| Box::leak(fake.into_boxed_str()));
    }
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Clone)]
pub struct AppAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// GitHub 在资产上直接给的 `sha256:` 摘要；给不出时要从 `sha256.txt` 里捞。
    pub sha256: Option<String>,
}

/// 更新通道。稳定通道只问 `/releases/latest`（GitHub 保证它不含 draft、也不含 prerelease）；
/// 预发通道问 `/releases` 列表里第一条非 draft 的 —— 那通常就是刚打上去的 beta。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateChannel {
    Stable,
    Insider,
}

impl UpdateChannel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "稳定",
            Self::Insider => "预发",
        }
    }

    pub fn from_insider_flag(insider: bool) -> Self {
        if insider {
            Self::Insider
        } else {
            Self::Stable
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppReleaseInfo {
    pub tag: String,
    pub current_version: String,
    pub update_available: bool,
    pub asset: Option<AppAsset>,
    /// 同一次 Release 里的 `sha256.txt` 原链（资产自己没摘要时用它核对）。
    pub hash_index_url: Option<String>,
    /// 取到的这条 Release 本身是不是预发版（界面上要标出来，别让人以为拿到的是稳定版）。
    pub prerelease: bool,
    pub message: String,
    /// Release 说明文档（GitHub Releases 正文）
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppUpdateStage {
    Checking,
    Downloading,
    Verifying,
    Ready,
}

impl AppUpdateStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "检查版本",
            Self::Downloading => "下载安装包",
            Self::Verifying => "校验",
            Self::Ready => "待安装",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppUpdateProgress {
    pub stage: AppUpdateStage,
    pub downloaded: u64,
    pub total: u64,
    pub bytes_per_second: f64,
    pub message: String,
}

impl AppUpdateProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.downloaded as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        }
    }

    pub fn rate(&self) -> u64 {
        self.bytes_per_second.max(0.0) as u64
    }
}

impl AppReleaseInfo {
    /// 「下载更新」按钮的门槛：既要有更新的版本，也要这个 Release 真的挂了安装包。
    /// 侧栏绿色↑要显示的版本号：只有"确实有更新"才亮，没更新或查失败都返回 None。
    pub fn marker(&self) -> Option<&str> {
        self.update_available.then(|| self.tag.as_str())
    }

    #[allow(dead_code)]
    pub fn downloadable(&self) -> bool {
        self.update_available && self.asset.is_some()
    }

    /// 无论是否为新版本，只要挂载了本平台安装包均可安装（用于历史版本回退或重装）。
    pub fn can_install(&self) -> bool {
        self.asset.is_some()
    }
}

/// 历史版本条目（用于“关于”页面的版本切换与老版本回退安装）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReleaseItem {
    pub tag: String,
    pub name: String,
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub html_url: String,
    pub body: Option<String>,
    pub asset_name: Option<String>,
    pub asset_size: u64,
    pub asset_url: Option<String>,
    pub asset_sha256: Option<String>,
    pub hash_index_url: Option<String>,
    pub is_current: bool,
}

impl ReleaseItem {
    pub fn to_release_info(&self, current: &str) -> AppReleaseInfo {
        let asset = if let (Some(name), Some(url)) = (&self.asset_name, &self.asset_url) {
            Some(AppAsset {
                name: name.clone(),
                url: url.clone(),
                size: self.asset_size,
                sha256: self.asset_sha256.clone(),
            })
        } else {
            None
        };
        let update_available = asset.is_some() && version_is_newer(&self.tag, current);
        let tag_clean = self.tag.trim_start_matches('v');
        let current_clean = current.trim_start_matches('v');
        let is_current = tag_clean == current_clean;
        let message = if asset.is_none() {
            format!("Release {} 没有适用于本平台的安装包", self.tag)
        } else if update_available {
            format!(
                "v{} 可更新至 {}{}",
                current,
                self.tag,
                if self.prerelease { "（预发）" } else { "" }
            )
        } else if is_current {
            format!("当前运行版本（v{}）", current)
        } else {
            format!(
                "v{} 回退/切换至 {}{}",
                current,
                self.tag,
                if self.prerelease { "（预发）" } else { "" }
            )
        };
        AppReleaseInfo {
            tag: self.tag.clone(),
            current_version: current.to_string(),
            update_available,
            asset,
            hash_index_url: self.hash_index_url.clone(),
            prerelease: self.prerelease,
            message,
            body: self.body.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<AssetJson>,
    /// GitHub 上勾了"这是一个预发布版本"。稳定通道永远拿不到它。
    #[serde(default)]
    prerelease: bool,
    /// 还没发布的草稿。两条通道都不许把它当更新。
    #[serde(default)]
    draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

impl AssetJson {
    /// GitHub 的 `digest` 形如 `sha256:<64 hex>`；格式不对就当没有，不许猜。
    fn sha256(&self) -> Option<String> {
        normalized_digest(self.digest.as_deref()?)
    }
}

fn normalized_digest(value: &str) -> Option<String> {
    let hex = value.trim().strip_prefix("sha256:").unwrap_or(value).trim();
    if hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(hex.to_ascii_lowercase())
    } else {
        None
    }
}

/// `/etc/os-release` 里认这个机器该收哪种包（纯函数：喂文本就能测，不靠运气）。
/// 顺序按"这台机器能装什么"给：Debian 系只要 deb，红帽系只要 rpm，认不出来时 deb 在前
/// （Debian/Ubuntu 的装机量更大）。`ID_LIKE` 也要看 —— 衍生版常只改 `ID`，
/// 血统写在 `ID_LIKE` 里，只看 ID 会把 Linux Mint / Pop!_OS 认成"不知道"。
#[allow(dead_code)]
fn package_preference_from_os_release(os_release: &str) -> Vec<PackageKind> {
    let mut family: Vec<String> = Vec::new();
    for line in os_release.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !matches!(key.trim(), "ID" | "ID_LIKE") {
            continue;
        }
        family.extend(
            value
                .trim()
                .trim_matches('"')
                .split_whitespace()
                .map(|token| token.to_ascii_lowercase()),
        );
    }
    let looks_debian = family
        .iter()
        .any(|token| token == "debian" || token == "ubuntu");
    let looks_redhat = family.iter().any(|token| {
        matches!(
            token.as_str(),
            "rhel" | "fedora" | "suse" | "opensuse-tumbleweed"
        )
    });
    match (looks_debian, looks_redhat) {
        (true, _) => vec![PackageKind::Deb],
        (false, true) => vec![PackageKind::Rpm],
        (false, false) => vec![PackageKind::Deb, PackageKind::Rpm],
    }
}

#[allow(dead_code)]
pub fn is_running_as_appimage() -> bool {
    std::env::var_os("APPIMAGE").is_some()
        || std::env::var_os("APPDIR").is_some()
        || std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(|s| s.contains("/.mount_")))
            .unwrap_or(false)
}

/// 这台机器该优先认哪种包。Windows 直接给死；Linux 在 AppImage 环境下优先 AppImage，
/// 否则读 `/etc/os-release`，读不到就 deb/rpm 都试，同时追加 AppImage 作兜底。
fn package_preference() -> Vec<PackageKind> {
    #[cfg(windows)]
    {
        vec![PackageKind::WindowsSetup]
    }
    #[cfg(not(windows))]
    {
        if is_running_as_appimage() {
            return vec![PackageKind::AppImage];
        }
        let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let mut kinds = package_preference_from_os_release(&text);
        kinds.push(PackageKind::AppImage); // 兜底：通用单文件
        kinds.push(PackageKind::WindowsSetup); // 兜底：万一 Release 只发了 Windows 包
        kinds
    }
}

/// 按平台给的顺序逐种包型试；每种内部仍是"正好等于当前 tag 的那个优先"。
/// 同一 Release 里可能留着上一版的安装包，所以后缀匹配一律要求名字里带当前版本号。
fn pick_installer_asset(assets: &[AssetJson], tag: &str) -> Option<AppAsset> {
    let version = tag.trim_start_matches('v');
    for kind in package_preference() {
        if let Some(asset) = pick_one_kind(assets, kind, version) {
            return Some(asset);
        }
    }
    None
}

fn pick_one_kind(assets: &[AssetJson], kind: PackageKind, version: &str) -> Option<AppAsset> {
    let expected = format!("vpn-parser-{version}{}", kind.suffix());
    // 松匹配按包型分两档，理由见 `needs_version_in_name`：Windows 那串是 release.yml
    // 按同一个模板写的，后缀命中就等于版本命中；deb/rpm/AppImage 的名字里还夹着工具自己填的
    // 修订号（`-1`）和架构（`amd64`/`x86_64`），只看后缀就可能挑到别的版本那一份。
    let needs_version = kind.needs_version_in_name();
    let suffix_lower = kind.suffix().to_ascii_lowercase();
    let loose = |asset: &AssetJson| {
        let name_lower = asset.name.to_ascii_lowercase();
        name_lower.ends_with(&suffix_lower) && (!needs_version || asset.name.contains(version))
    };
    let found = assets
        .iter()
        .position(|asset| asset.name.eq_ignore_ascii_case(&expected))
        .or_else(|| assets.iter().rposition(loose))?;
    let asset = &assets[found];
    Some(AppAsset {
        name: asset.name.clone(),
        url: asset.browser_download_url.clone(),
        size: asset.size,
        sha256: asset.sha256(),
    })
}

/// `sha256.txt` 由 CI 用 `Get-FileHash` 生成，一行 `<hex>  <文件名>`（两个空格）。
fn digest_from_hash_index(index: &str, asset_name: &str) -> Option<String> {
    index.lines().find_map(|line| {
        let (hex, name) = line.split_once("  ").or_else(|| line.split_once(' '))?;
        if name.trim() == asset_name.trim() {
            normalized_digest(hex)
        } else {
            None
        }
    })
}

/// 拆成 `(数字段, 预发布段)`：`v1.4.0-beta.2` → `([1,4,0], ["beta","2"])`。
/// `+build` 那种构建元数据按 semver 不参与优先级，直接丢掉。
fn parse_version(input: &str) -> (Vec<u64>, Vec<String>) {
    let text = input.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match text.split_once('-') {
        Some((core, rest)) => (core, rest.split_once('+').map(|(p, _)| p).unwrap_or(rest)),
        None => (text.split_once('+').map(|(c, _)| c).unwrap_or(text), ""),
    };
    let numbers = core
        .split('.')
        .map(|segment| segment.parse::<u64>().unwrap_or_default())
        .collect();
    let pre = if pre.is_empty() {
        Vec::new()
    } else {
        pre.split('.').map(str::to_string).collect()
    };
    (numbers, pre)
}

/// 核心版本号相同，只比预发布段（semver 11.4 的那套）：数字标识按数值比、数字段永远小于
/// 非数字段、都是非数字按 ASCII 比；标识少的一方若是另一方的前缀，则更小。
fn prerelease_is_newer(candidate: &[String], current: &[String]) -> bool {
    for index in 0..candidate.len().max(current.len()) {
        let (Some(mine), Some(theirs)) = (candidate.get(index), current.get(index)) else {
            // 走到这里说明一方已经比另一方短：短的那个更小。
            return candidate.len() > current.len();
        };
        match (mine.parse::<u64>(), theirs.parse::<u64>()) {
            (Ok(mine_number), Ok(theirs_number)) => {
                if mine_number != theirs_number {
                    return mine_number > theirs_number;
                }
            }
            (Ok(_), Err(_)) => return false,
            (Err(_), Ok(_)) => return true,
            (Err(_), Err(_)) => {
                if mine != theirs {
                    return mine > theirs;
                }
            }
        }
    }
    false
}

/// 有没有新版本。数字段逐位比（`1.10` 比 `1.9` 新，缺的位补 0）；数字段相同时按 semver：
/// **不带预发布后缀的更新**，所以 `1.4.0` 会把还停在 `1.4.0-beta.2` 的人接走，而
/// `1.4.0-beta.1` 绝不会被当成 `1.4.0` 的新版。解析不出数字（空串、HTML 片段）一律算"没有新版"。
pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let (candidate_numbers, candidate_pre) = parse_version(candidate);
    let (current_numbers, current_pre) = parse_version(current);
    if candidate_numbers.iter().all(|value| *value == 0) {
        return false;
    }
    let width = candidate_numbers.len().max(current_numbers.len());
    for index in 0..width {
        let mine = *candidate_numbers.get(index).unwrap_or(&0);
        let theirs = *current_numbers.get(index).unwrap_or(&0);
        if mine != theirs {
            return mine > theirs;
        }
    }
    match (candidate_pre.is_empty(), current_pre.is_empty()) {
        (true, true) => false,
        (true, false) => true,
        (false, true) => false,
        (false, false) => prerelease_is_newer(&candidate_pre, &current_pre),
    }
}

struct Transport {
    label: String,
    client: Client,
}

/// 查版本/取摘要文件的通道候选：应用内代理 → 系统代理 → 直连，一次只跑一条，
/// 和内核更新用的是同一份候选（`proxy_candidates`）。
fn transports(app_proxy: Option<&str>) -> Result<Vec<Transport>> {
    let mut list = Vec::new();
    for candidate in crate::kernel::proxy_candidates(app_proxy) {
        let mut builder = Client::builder()
            .user_agent(concat!(
                "vpn-parser/",
                env!("CARGO_PKG_VERSION"),
                " app-updater"
            ))
            .connect_timeout(std::time::Duration::from_secs(4))
            .read_timeout(std::time::Duration::from_secs(15))
            .http1_only();
        let label = match candidate.as_deref() {
            Some(url) => {
                builder = builder.proxy(reqwest::Proxy::all(url)?);
                format!("代理 {url}")
            }
            None => "直连".to_string(),
        };
        list.push(Transport {
            label,
            client: builder.build()?,
        });
    }
    Ok(list)
}

/// 一条下载通道：镜像只代理 `github.com` 的文件直链、且本身就在国内，所以固定直连；
/// 代理候选只配官方原链。顺序照内核那边实测的速度排（镜像 4.9~8.1 MB/s，官方直连 0.12）。
#[derive(Debug, Clone)]
struct Channel {
    label: String,
    url: String,
    proxy: Option<String>,
}

fn download_ladder(asset_url: &str, proxies: &[Option<String>]) -> Vec<Channel> {
    let mut ladder = Vec::new();
    for (name, prefix) in crate::kernel::ASSET_MIRROR_PREFIXES {
        ladder.push(Channel {
            label: format!("直连·{name} 镜像"),
            url: format!("{prefix}{asset_url}"),
            proxy: None,
        });
    }
    for proxy in proxies.iter().flatten() {
        ladder.push(Channel {
            label: format!("代理 {proxy}"),
            url: asset_url.to_string(),
            proxy: Some(proxy.clone()),
        });
    }
    ladder.push(Channel {
        label: "直连·官方原链".to_string(),
        url: asset_url.to_string(),
        proxy: None,
    });
    ladder
}

fn client_for(proxy: Option<&str>, connect_secs: u64, total_secs: u64) -> Result<Client> {
    let mut builder = Client::builder()
        .user_agent(concat!(
            "vpn-parser/",
            env!("CARGO_PKG_VERSION"),
            " app-updater"
        ))
        .connect_timeout(std::time::Duration::from_secs(connect_secs))
        .http1_only();
    if let Some(url) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(url)?);
    }
    if total_secs > 0 {
        builder = builder.timeout(std::time::Duration::from_secs(total_secs));
    }
    builder.build().context("构建下载客户端失败")
}

/// 查最新 Release。`current` 传本机版本（默认取 `current_version()`，测试里传假值）。
///
/// 稳定通道问 `/releases/latest`（GitHub 保证它跳过 draft 和 prerelease）；预发通道问
/// `/releases` 列表并取第一条非 draft 的。两条通道任一时刻都只有一条连接在跑。
pub async fn check(
    app_proxy: Option<&str>,
    current: &str,
    channel: UpdateChannel,
) -> Result<AppReleaseInfo> {
    let url = match channel {
        UpdateChannel::Stable => {
            format!("https://api.github.com/repos/{REPOSITORY}/releases/latest")
        }
        UpdateChannel::Insider => {
            format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page=15")
        }
    };
    let mut failures = Vec::new();
    for transport in transports(app_proxy)? {
        let mut request = transport
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(8));
        if let Some(token) = github_token() {
            request = request.bearer_auth(&token);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                failures.push(format!("{}: 连不上（{error}）", transport.label));
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            let rate_limited = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == "0");
            failures.push(format!(
                "{}: {}",
                transport.label,
                describe_rejection(status, rate_limited)
            ));
            continue;
        }
        let picked: Result<Option<ReleaseJson>> = match channel {
            UpdateChannel::Stable => response
                .json::<ReleaseJson>()
                .await
                .map(Some)
                .map_err(|error| anyhow::anyhow!(error)),
            UpdateChannel::Insider => response
                .json::<Vec<ReleaseJson>>()
                .await
                .map(|list| list.into_iter().find(|release| !release.draft))
                .map_err(|error| anyhow::anyhow!(error)),
        };
        match picked {
            Ok(Some(release)) => return Ok(summarize(release, current, channel)),
            Ok(None) => failures.push(format!("{}: 这个通道里还没有已发布的版本", transport.label)),
            Err(error) => failures.push(format!("{}: 响应读不完（{error}）", transport.label)),
        }
    }

    // 若 GitHub API（未认证 60 次/小时）遭遇限流或网络阻断，透明回退至 releases.atom 备用源
    if let Ok(atom_items) = fetch_releases_from_atom(app_proxy, current).await {
        let picked = match channel {
            UpdateChannel::Stable => atom_items.into_iter().find(|r| !r.prerelease),
            UpdateChannel::Insider => atom_items.into_iter().next(),
        };
        if let Some(item) = picked {
            crate::startup_log(&format!(
                "[更新] GitHub API 请求受阻，已通过 Atom 备用源获取到版本 {}",
                item.tag
            ));
            return Ok(item.to_release_info(current));
        }
    }

    bail!(
        "读取 {REPOSITORY} 最新 Release 失败：{}",
        failures.join("；")
    )
}

/// GitHub 的匿名 API 配额只有 60 次/小时，而且按**出口 IP** 算 —— 这台机器上还和别的工具共用。
/// 环境里有 `GITHUB_TOKEN` / `GH_TOKEN` 就用（配额 5000 次/小时），没有就匿名查。
fn github_token() -> Option<String> {
    pick_github_token(
        ["GITHUB_TOKEN", "GH_TOKEN"]
            .iter()
            .filter_map(|key| std::env::var(key).ok()),
    )
}

/// 从按优先级排好的候选里挑第一个真令牌。全空/全空白 = 没有（宁可不认证，也别发一个空
/// `Authorization` 头 —— GitHub 会把那当成坏凭据直接 401）。
fn pick_github_token(candidates: impl IntoIterator<Item = impl AsRef<str>>) -> Option<String> {
    candidates
        .into_iter()
        .map(|token| token.as_ref().trim().to_string())
        .find(|token| !token.is_empty())
}

/// 把"网络不通"和"GitHub 拒绝了我们"分开说：混成一条 `error sending request` 会让人
/// 以为是代理坏了，其实多半是匿名配额用尽，等一小时或给个 token 就好。
fn describe_rejection(status: reqwest::StatusCode, rate_limited: bool) -> String {
    let code = status.as_u16();
    if code == 403 || code == 429 {
        if rate_limited {
            return format!(
                "GitHub API 限流（HTTP {code}，匿名配额 60 次/小时）：不是网络故障，\
                 设 GITHUB_TOKEN 可放宽到 5000 次/小时"
            );
        }
        return format!("GitHub 拒绝了请求（HTTP {code}）");
    }
    format!("HTTP {code}")
}

/// 拉取 GitHub Releases 历史版本列表（过滤草稿，解析匹配本平台的安装包资产与发布信息）。
pub async fn list_releases(
    app_proxy: Option<&str>,
    current: &str,
    per_page: usize,
) -> Result<Vec<ReleaseItem>> {
    let per_page = per_page.clamp(1, 100);
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases?per_page={per_page}");
    let mut failures = Vec::new();
    for transport in transports(app_proxy)? {
        let mut request = transport
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10));
        if let Some(token) = github_token() {
            request = request.bearer_auth(&token);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                failures.push(format!("{}: 请求失败: {error}", transport.label));
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            let rate_limited = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value == "0");
            failures.push(format!(
                "{}: {}",
                transport.label,
                describe_rejection(status, rate_limited)
            ));
            continue;
        }
        match response.json::<Vec<ReleaseJson>>().await {
            Ok(list) => {
                let mut items = Vec::new();
                for release in list.into_iter().filter(|r| !r.draft) {
                    let asset = pick_installer_asset(&release.assets, &release.tag_name);
                    let hash_index_url = release
                        .assets
                        .iter()
                        .find(|a| a.name == HASH_INDEX_NAME)
                        .map(|a| a.browser_download_url.clone())
                        .or_else(|| {
                            let installer =
                                pick_installer_asset(&release.assets, &release.tag_name)?;
                            Some(
                                installer
                                    .url
                                    .replace(installer.name.rsplit('/').next()?, HASH_INDEX_NAME),
                            )
                        });
                    let tag_clean = release.tag_name.trim_start_matches('v');
                    let current_clean = current.trim_start_matches('v');
                    let is_current = tag_clean == current_clean;
                    let (asset_name, asset_url, asset_size, asset_sha256) = match asset {
                        Some(a) => (Some(a.name), Some(a.url), a.size, a.sha256),
                        None => (None, None, 0, None),
                    };
                    items.push(ReleaseItem {
                        tag: release.tag_name.clone(),
                        name: release.name.unwrap_or_else(|| release.tag_name.clone()),
                        published_at: release.published_at.map(|d| {
                            d.split('T').next().unwrap_or(&d).to_string()
                        }),
                        prerelease: release.prerelease,
                        html_url: release.html_url.unwrap_or_else(|| {
                            format!(
                                "https://github.com/{REPOSITORY}/releases/tag/{}",
                                release.tag_name
                            )
                        }),
                        body: release.body,
                        asset_name,
                        asset_size,
                        asset_url,
                        asset_sha256,
                        hash_index_url,
                        is_current,
                    });
                }
                return Ok(items);
            }
            Err(error) => {
                failures.push(format!("{}: 解析 JSON 失败: {error}", transport.label))
            }
        }
    }

    // 若 GitHub API（未认证 60 次/小时）受限或网络故障，透明回退至 releases.atom 备用源
    match fetch_releases_from_atom(app_proxy, current).await {
        Ok(items) => {
            crate::startup_log(&format!(
                "[更新] GitHub API 请求受阻，已通过 Atom 备用源拉取到 {} 个历史版本",
                items.len()
            ));
            return Ok(items);
        }
        Err(atom_err) => {
            failures.push(format!("Atom 备用源: {atom_err}"));
        }
    }

    bail!("获取 Releases 历史列表失败：{}", failures.join("；"))
}

/// 从 GitHub Atom Feed 备用源拉取历史版本列表（不受 API 60次/小时 限流影响）
pub async fn fetch_releases_from_atom(
    app_proxy: Option<&str>,
    current: &str,
) -> Result<Vec<ReleaseItem>> {
    let url = format!("https://github.com/{REPOSITORY}/releases.atom");
    let mut failures = Vec::new();
    for transport in transports(app_proxy)? {
        let response = match transport
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Ok(res) => res,
            Err(err) => {
                failures.push(format!("{}: 连不上（{err}）", transport.label));
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            failures.push(format!("{}: HTTP {status}", transport.label));
            continue;
        }
        match response.text().await {
            Ok(xml) => {
                let items = parse_atom_releases(&xml, current);
                if !items.is_empty() {
                    return Ok(items);
                }
                failures.push(format!("{}: Atom Feed 中未包含有效 Release 条目", transport.label));
            }
            Err(err) => failures.push(format!("{}: 读取 Atom 文本失败: {err}", transport.label)),
        }
    }
    bail!("通过 Atom 备用源获取 Releases 失败：{}", failures.join("；"))
}

fn extract_xml_tag_content(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let rest = &xml[start + open.len()..];
    let end_bracket = rest.find('>')?;
    let content_start = &rest[end_bracket + 1..];
    let end = content_start.find(&close)?;
    Some(content_start[..end].trim().to_string())
}

fn extract_link_href(xml: &str) -> Option<String> {
    for line in xml.lines() {
        if line.contains("<link") && line.contains("href=") {
            if let Some(start) = line.find("href=\"") {
                let rest = &line[start + 6..];
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

fn synthesize_candidate_assets(tag: &str) -> Vec<AssetJson> {
    let version = tag.trim_start_matches('v');
    vec![
        AssetJson {
            name: format!("vpn-parser-{version}-windows-x64-setup.exe"),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/vpn-parser-{version}-windows-x64-setup.exe"
            ),
            size: 0,
            digest: None,
        },
        AssetJson {
            name: format!("vpn-parser_{version}_amd64.deb"),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/vpn-parser_{version}_amd64.deb"
            ),
            size: 0,
            digest: None,
        },
        AssetJson {
            name: format!("vpn-parser-{version}-x86_64.AppImage"),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/vpn-parser-{version}-x86_64.AppImage"
            ),
            size: 0,
            digest: None,
        },
        AssetJson {
            name: format!("vpn-parser-{version}.AppImage"),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/vpn-parser-{version}.AppImage"
            ),
            size: 0,
            digest: None,
        },
        AssetJson {
            name: format!("vpn-parser-{version}-1.x86_64.rpm"),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/{tag}/vpn-parser-{version}-1.x86_64.rpm"
            ),
            size: 0,
            digest: None,
        },
    ]
}

fn parse_atom_releases(xml: &str, current: &str) -> Vec<ReleaseItem> {
    let mut items = Vec::new();
    for chunk in xml.split("<entry>").skip(1) {
        let Some(entry_xml) = chunk.split("</entry>").next() else {
            continue;
        };
        let tag = extract_xml_tag_content(entry_xml, "id")
            .and_then(|id| id.rsplit('/').next().map(|s| s.trim().to_string()))
            .unwrap_or_default();
        if tag.is_empty() {
            continue;
        }

        let name = extract_xml_tag_content(entry_xml, "title").unwrap_or_else(|| tag.clone());
        let published_at = extract_xml_tag_content(entry_xml, "updated").map(|d| {
            d.split('T').next().unwrap_or(&d).to_string()
        });
        let html_url = extract_link_href(entry_xml).unwrap_or_else(|| {
            format!("https://github.com/{REPOSITORY}/releases/tag/{tag}")
        });
        let body = extract_xml_tag_content(entry_xml, "content");
        let tag_clean = tag.trim_start_matches('v');
        let current_clean = current.trim_start_matches('v');
        let is_current = tag_clean == current_clean;
        let prerelease = tag.contains('-') || tag.contains("alpha") || tag.contains("beta") || tag.contains("rc");

        let candidate_assets = synthesize_candidate_assets(&tag);
        let picked = pick_installer_asset(&candidate_assets, &tag);
        let (asset_name, asset_url, asset_size, asset_sha256) = match picked {
            Some(a) => (Some(a.name), Some(a.url), a.size, a.sha256),
            None => (None, None, 0, None),
        };
        let hash_index_url = Some(format!(
            "https://github.com/{REPOSITORY}/releases/download/{tag}/{HASH_INDEX_NAME}"
        ));

        items.push(ReleaseItem {
            tag,
            name,
            published_at,
            prerelease,
            html_url,
            body,
            asset_name,
            asset_size,
            asset_url,
            asset_sha256,
            hash_index_url,
            is_current,
        });
    }
    items
}

fn summarize(release: ReleaseJson, current: &str, channel: UpdateChannel) -> AppReleaseInfo {
    let asset = pick_installer_asset(&release.assets, &release.tag_name);
    let update_available = asset.is_some() && version_is_newer(&release.tag_name, current);
    let wanted = package_preference()
        .first()
        .copied()
        .map(PackageKind::label)
        .unwrap_or("本机可用的");
    let message = if asset.is_none() {
        format!(
            "{} Release {} 里没有 {wanted}资产",
            channel.label(),
            release.tag_name
        )
    } else if update_available {
        format!(
            "v{} 可更新至 {}{}",
            current,
            release.tag_name,
            if release.prerelease {
                "（预发）"
            } else {
                ""
            }
        )
    } else {
        format!("已是最新版本（v{current}）")
    };
    // 摘要优先看资产自带的 `digest`，没有就去 Release 的 sha256.txt；那份文件不存在时
    // 按同一 Release 的下载路径把文件名换掉试一次（CI 一直两个一起传，缺一只是个别 tag）。
    let hash_index_url = release
        .assets
        .iter()
        .find(|asset| asset.name == HASH_INDEX_NAME)
        .map(|asset| asset.browser_download_url.clone())
        .or_else(|| {
            // 这里以前按 `-windows-x64-setup.exe` 找"随便一个安装包"来推 sha256.txt 的地址，
            // 于是 Linux 上这条回退永远是 None。改成走同一个选包函数：平台不同也不会失联。
            let installer = pick_installer_asset(&release.assets, &release.tag_name)?;
            Some(
                installer
                    .url
                    .replace(installer.name.rsplit('/').next()?, HASH_INDEX_NAME),
            )
        });
    AppReleaseInfo {
        tag: release.tag_name,
        current_version: current.to_string(),
        update_available,
        hash_index_url,
        prerelease: release.prerelease,
        asset,
        message,
        body: release.body,
    }
}

/// 安装包暂存在**数据根目录下的 `updates`**（和内核的 `kernels` 同一层，用户 2026-10-14 指定）：
/// 装完想留着或手动删掉都找得到，不会像 `%TEMP%` 那样被清理工具悄悄收走、也看不见。
/// 数据根目录在 Windows 是程序目录下的 `data/`，在 Linux 是 XDG 那份（见 `crate::data_root`）。
/// 诊断壳（VPN_PARSER_DIAG）的根跟着它自己的临时 cwd 走，所以天然落在临时目录里，不碰真安装目录。
pub fn installer_cache_dir() -> PathBuf {
    // 和内核同一个根（`crate::data_root`）：这里以前自己拼 `cwd/data`，Linux 上内核搬到
    // XDG 之后两个目录就分家了 —— 安装包缓存留在 /usr/bin/data（写不进去），内核却在
    // ~/.local/share。测试里那条"必须是 data/kernels 的兄弟目录"的断言抓到了它。
    crate::data_root().join("updates")
}

/// 下载安装包并校验 SHA-256，成功返回本地路径。校验不过直接报错，绝不返回半截或换过内容的文件。
pub async fn download<F>(
    info: &AppReleaseInfo,
    app_proxy: Option<&str>,
    cancelled: &AtomicBool,
    progress: &F,
) -> Result<PathBuf>
where
    F: Fn(AppUpdateProgress) + Send + Sync,
{
    let asset = info
        .asset
        .clone()
        .with_context(|| format!("Release {} 没有可下载的安装包", info.tag))?;
    if asset.size > MAX_INSTALLER_SIZE {
        bail!("安装包大小异常: {} 字节", asset.size);
    }
    let expected = match (asset.sha256.clone(), info.hash_index_url.as_deref()) {
        (Some(digest), _) => digest,
        (None, Some(index_url)) => {
            let text = fetch_text(index_url, app_proxy).await?;
            digest_from_hash_index(&text, &asset.name).with_context(|| {
                format!(
                    "{HASH_INDEX_NAME} 里找不到 {asset} 的摘要",
                    asset = asset.name
                )
            })?
        }
        (None, None) => bail!("GitHub 既没给资产摘要也没有 {HASH_INDEX_NAME}，无法校验安装包"),
    };

    let channels = download_ladder(&asset.url, &crate::kernel::proxy_candidates(app_proxy));

    let directory = installer_cache_dir();
    std::fs::create_dir_all(&directory).context("创建安装包临时目录失败")?;
    let target = directory.join(&asset.name);
    let _ = std::fs::remove_file(&target);

    let mut failures = Vec::new();
    for channel in channels {
        if cancelled.load(Ordering::Relaxed) {
            bail!("更新已取消");
        }
        let label = channel.label.clone();
        let client = client_for(channel.proxy.as_deref(), 6, 600)?;
        match stream(
            &client,
            &channel.url,
            &label,
            &asset,
            &expected,
            &target,
            cancelled,
            progress,
        )
        .await
        {
            Ok(()) => {
                let actual_size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(asset.size);
                crate::startup_log(&format!(
                    "[更新] 安装包已下载并校验: {}（{} 字节，通道 {label}）",
                    target.display(),
                    actual_size
                ));
                progress(AppUpdateProgress {
                    stage: AppUpdateStage::Ready,
                    downloaded: actual_size,
                    total: actual_size,
                    bytes_per_second: 0.0,
                    message: format!("{} 已校验，可以安装", asset.name),
                });
                return Ok(target);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&target);
                crate::startup_log(&format!("[更新] 通道 {label} 下载失败: {error}"));
                failures.push(format!("{label}: {error}"));
                progress(AppUpdateProgress {
                    stage: AppUpdateStage::Downloading,
                    downloaded: 0,
                    total: asset.size,
                    bytes_per_second: 0.0,
                    message: format!("{label} 失败，换下一条通道"),
                });
            }
        }
    }
    bail!("下载 {} 失败：{}", asset.name, failures.join("；"))
}

#[allow(clippy::too_many_arguments)]
async fn stream(
    client: &Client,
    url: &str,
    label: &str,
    asset: &AppAsset,
    expected: &str,
    target: &Path,
    cancelled: &AtomicBool,
    progress: &(dyn Fn(AppUpdateProgress) + Send + Sync),
) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("{error:#}"))?
        .error_for_status()
        .context("HTTP 状态失败")?;
    let response_len = response.content_length().unwrap_or(0);
    if response_len > MAX_INSTALLER_SIZE {
        bail!("下载内容过大: {} 字节", response_len);
    }
    let total_size = if asset.size > 0 {
        asset.size
    } else if response_len > 0 {
        response_len
    } else {
        15 * 1024 * 1024
    };
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(target).with_context(|| format!("创建 {}", target.display()))?,
    );
    let mut hasher = Sha256::new();
    let mut written = 0_u64;
    let started = std::time::Instant::now();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            bail!("更新已取消");
        }
        match response.chunk().await {
            Ok(Some(chunk)) => {
                written += chunk.len() as u64;
                if written > MAX_INSTALLER_SIZE {
                    bail!("下载超过安全大小限制");
                }
                hasher.update(&chunk);
                std::io::Write::write_all(&mut file, &chunk)?;
                let elapsed = started.elapsed().as_secs_f64().max(0.001);
                progress(AppUpdateProgress {
                    stage: AppUpdateStage::Downloading,
                    downloaded: written,
                    total: total_size,
                    bytes_per_second: written as f64 / elapsed,
                    message: format!("正在下载 {}（{label}）", asset.name),
                });
            }
            Ok(None) => break,
            Err(error) => bail!("下载中断: {error}"),
        }
    }
    std::io::Write::flush(&mut file)?;
    drop(file);
    if asset.size > 0 && written != asset.size {
        bail!("只下到 {written} / {} 字节", asset.size);
    }
    if written == 0 {
        bail!("下载文件为空（0 字节）");
    }
    progress(AppUpdateProgress {
        stage: AppUpdateStage::Verifying,
        downloaded: written,
        total: written,
        bytes_per_second: 0.0,
        message: "正在校验 SHA-256".into(),
    });
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("SHA-256 校验失败（实际 {actual}）");
    }
    Ok(())
}

/// 资产没带摘要时，用小文件 `sha256.txt` 兜底；通道同样按代理候选依次试。
async fn fetch_text(url: &str, app_proxy: Option<&str>) -> Result<String> {
    let mut failures = Vec::new();
    for transport in transports(app_proxy)? {
        match transport
            .client
            .get(url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(response) => match response.text().await {
                Ok(text) => return Ok(text),
                Err(error) => failures.push(format!("{}: {error}", transport.label)),
            },
            Err(error) => failures.push(format!("{}: {error}", transport.label)),
        }
    }
    bail!("读取 {HASH_INDEX_NAME} 失败：{}", failures.join("；"))
}

/// 把安装包交给 Inno 向导。目录不用传：同一 AppId 的旧安装记录会把它自己填回原目录，
/// 而 `/DIR=` 带空格那份转义在 Inno 那边是个坑（历史上踩过），能不传就不传。
/// 注意安装包 `InitializeSetup` 会 taskkill 掉本进程，所以点这个按钮等于"程序马上重启"。
/// 把下好的包"交出去"。两种结局，界面必须分开说，不然 Linux 上会出现
/// "安装向导已启动"这句假话 —— 根本没有向导会接管。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Handoff {
    /// Windows：Inno 向导起来了，它会先 taskkill 本进程再装。
    WizardLaunched,
    /// Linux：只能把该敲的命令给用户。程序不会自己去拿 root。
    ManualInstall { command: String },
}

pub fn launch_installer(path: &Path) -> Result<Handoff> {
    if !path.is_file() {
        bail!("安装包不在了: {}", path.display());
    }
    #[cfg(not(windows))]
    {
        let ext_lower = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_ascii_lowercase());
        let kind = match ext_lower.as_deref() {
            Some("deb") => PackageKind::Deb,
            Some("rpm") => PackageKind::Rpm,
            Some("appimage") => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = path.metadata() {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = std::fs::set_permissions(path, perms);
                    }
                }
                PackageKind::AppImage
            }
            _ => PackageKind::Deb,
        };
        let command = kind.install_command(path);
        crate::startup_log(&format!(
            "[更新] 包已就位，需要用户自行安装：{}（{} 字节已校验）",
            command,
            path.metadata().map(|m| m.len()).unwrap_or(0)
        ));
        return Ok(Handoff::ManualInstall { command });
    }
    #[cfg(windows)]
    {
        crate::platform::shell_open(path)
            .with_context(|| format!("启动安装程序失败: {}", path.display()))?;
        crate::startup_log(&format!(
            "[更新] 已把安装包交给安装向导: {}",
            path.display()
        ));
        Ok(Handoff::WizardLaunched)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str, digest: Option<&str>) -> AssetJson {
        AssetJson {
            name: name.to_string(),
            browser_download_url: format!(
                "https://github.com/{REPOSITORY}/releases/download/v1.2.2/{name}"
            ),
            size: 15_000_000,
            digest: digest.map(str::to_string),
        }
    }

    /// 发行版识别只看 `ID` 是不够的：Mint / Pop!_OS / Zorin 的 `ID` 是它们自己，
    /// 血统写在 `ID_LIKE` 里 —— 漏读会退回"deb+rpm 都试"，也就是把 rpm 包递给 apt 机器。
    #[test]
    fn the_distro_probe_reads_id_like_too() {
        use PackageKind::{Deb, Rpm};
        assert_eq!(
            package_preference_from_os_release(
                "ID=ubuntu
"
            ),
            vec![Deb]
        );
        assert_eq!(
            package_preference_from_os_release(
                "ID=\"debian\"
"
            ),
            vec![Deb]
        );
        assert_eq!(
            package_preference_from_os_release(
                "ID=linuxmint
ID_LIKE=\"ubuntu debian\"
"
            ),
            vec![Deb]
        );
        assert_eq!(
            package_preference_from_os_release(
                "ID=fedora
VERSION=40
"
            ),
            vec![Rpm]
        );
        assert_eq!(
            package_preference_from_os_release(
                "ID=opensuse-tumbleweed
"
            ),
            vec![Rpm]
        );
        // 认不出来就两个都留（顺序 deb 在前），而不是猜死一个。
        assert_eq!(
            package_preference_from_os_release(
                "ID=arch
"
            ),
            vec![Deb, Rpm]
        );
        assert_eq!(package_preference_from_os_release(""), vec![Deb, Rpm]);
        // 反向对照：`ID_LIKE` 里含 debian 但键名写成 `ID-LIKE`（打错）时不该被当成识别成功。
        assert_eq!(
            package_preference_from_os_release(
                "ID=weird
ID-LIKE=\"debian\"
"
            ),
            vec![Deb, Rpm]
        );
    }

    /// 打包工具生成的名字带修订号和架构，所以"前缀 + 尾巴"要能认；同时**别的版本的包
    /// 不许被挑走**（这是 deb/rpm 与 Windows 的差别，Windows 那份由 `installer_asset_is_matched_by_tag_
    /// not_by_whichever_is_there` 钉住它保留旧的宽松行为）。
    #[test]
    fn deb_and_rpm_names_are_matched_with_their_revision_and_arch() {
        let assets = vec![
            asset("notes.txt", None),
            asset("vpn-parser_1.5.0-1_amd64.deb", None),
            asset("vpn-parser_1.6.2-1_amd64.deb", None),
            asset("vpn-parser-1.6.2-1.x86_64.rpm", None),
            asset("vpn-parser-1.6.2-x86_64.AppImage", None),
        ];
        let deb = pick_one_kind(&assets, PackageKind::Deb, "1.6.2").expect("该找到 deb");
        assert_eq!(deb.name, "vpn-parser_1.6.2-1_amd64.deb");
        let rpm = pick_one_kind(&assets, PackageKind::Rpm, "1.6.2").expect("该找到 rpm");
        assert_eq!(rpm.name, "vpn-parser-1.6.2-1.x86_64.rpm");
        let appimage =
            pick_one_kind(&assets, PackageKind::AppImage, "1.6.2").expect("该找到 appimage");
        assert_eq!(appimage.name, "vpn-parser-1.6.2-x86_64.AppImage");
        // 反向对照：只到 1.6.1 的 Release 不该把 1.6.2 那份 deb 当成自己的。
        assert!(pick_one_kind(&assets, PackageKind::Deb, "1.6.1").is_none());
        assert!(pick_one_kind(&assets, PackageKind::AppImage, "1.6.1").is_none());
    }

    /// 交给用户的那条命令必须带真实路径 —— 说"请用包管理器安装"等于没说。
    #[test]
    fn each_kind_names_a_command_you_can_actually_run() {
        let path = std::path::Path::new("/tmp/vpn-parser_1.6.2-1_amd64.deb");
        assert_eq!(
            PackageKind::Deb.install_command(path),
            "sudo dpkg -i /tmp/vpn-parser_1.6.2-1_amd64.deb"
        );
        assert!(PackageKind::Rpm
            .install_command(path)
            .starts_with("sudo rpm -Uvh "));
        assert!(PackageKind::WindowsSetup
            .install_command(path)
            .contains("安装向导"));
        assert!(PackageKind::AppImage
            .install_command(path)
            .contains("chmod +x "));
    }

    #[test]
    fn version_compare_only_looks_at_numbers() {
        assert!(version_is_newer("v1.2.2", "1.2.1"));
        assert!(
            version_is_newer("1.10.0", "1.9.3"),
            "按数字比，不能按字典序"
        );
        assert!(version_is_newer("1.3", "1.2.9"));
        assert!(!version_is_newer("1.2.1", "1.2.1"));
        assert!(
            !version_is_newer("1.2", "1.2.1"),
            "缺的位补 0，不能把补丁版当新版"
        );
        assert!(
            !version_is_newer("2.0.1", "3.0.0"),
            "高位段更小就不能算新版，哪怕末位更大"
        );
        assert!(!version_is_newer("", "1.2.1"));
        assert!(!version_is_newer("not-a-version", "1.2.1"));
        assert!(version_is_newer("1.2.2-rc1", "1.2.1"), "核心号更大就先赢");
    }

    /// 预发布段按 semver 11.4 比 —— **这是"装了 beta 的人还能不能收到下一个 beta"的全部依据**。
    /// 以前的实现把 `-` 后面整段丢掉，`1.4.0-beta.1 → 1.4.0-beta.2` 会被判成"没更新"。
    #[test]
    fn prerelease_versions_sort_the_way_semver_says() {
        assert!(version_is_newer("1.4.0-beta.2", "1.4.0-beta.1"));
        assert!(
            version_is_newer("1.4.0-beta.10", "1.4.0-beta.9"),
            "标识是数字时要按数值比，不能按字典序"
        );
        assert!(version_is_newer("1.4.0-rc.1", "1.4.0-beta.9"), "rc > beta");
        assert!(
            version_is_newer("1.4.0", "1.4.0-beta.2"),
            "稳定版要把还停在 beta 的人接走"
        );
        assert!(
            !version_is_newer("1.4.0-beta.1", "1.4.0"),
            "beta 永远不许被当成稳定版的新版本"
        );
        assert!(
            !version_is_newer("1.4.0-beta.1", "1.4.0-beta.1"),
            "同版本不算更新"
        );
        assert!(
            !version_is_newer("1.4.0-beta", "1.4.0-beta.1"),
            "标识少的一方是前缀时更小"
        );
        assert!(
            version_is_newer("v1.4.0-beta.2", "v1.4.0-beta.1"),
            "带 v 前缀同样成立"
        );
        assert!(
            !version_is_newer("1.3.1", "1.4.0-beta.1"),
            "beta 用户的核心号是 1.4.0，更低的补丁号不该推给他"
        );
    }

    #[test]
    fn installer_asset_is_matched_by_tag_not_by_whichever_is_there() {
        let assets = vec![
            asset("vpn-parser-1.2.1-windows-x64-setup.exe", None),
            asset("sha256.txt", None),
            asset("vpn-parser-1.2.2-windows-x64-setup.exe", None),
        ];
        let picked = pick_installer_asset(&assets, "v1.2.2").expect("有安装包");
        assert_eq!(picked.name, "vpn-parser-1.2.2-windows-x64-setup.exe");
        assert_eq!(
            picked.url,
            "https://github.com/mateo-ki/vpn-parser/releases/download/v1.2.2/vpn-parser-1.2.2-windows-x64-setup.exe"
        );
        // 名字对不上 tag（比如 tag 打错、资产手工传过）时退到后缀匹配的最后一个。
        let loose = vec![asset("vpn-parser-0.9.0-windows-x64-setup.exe", None)];
        assert_eq!(
            pick_installer_asset(&loose, "v1.2.2").unwrap().name,
            "vpn-parser-0.9.0-windows-x64-setup.exe"
        );
        assert!(pick_installer_asset(&[asset("sha256.txt", None)], "v1.2.2").is_none());
    }

    #[test]
    fn digests_are_accepted_only_in_full_hex() {
        let hex = "a".repeat(64);
        assert_eq!(
            normalized_digest(&format!("sha256:{hex}")).as_deref(),
            Some(hex.as_str())
        );
        assert_eq!(
            normalized_digest(hex.to_uppercase().as_str())
                .unwrap()
                .len(),
            64
        );
        assert!(normalized_digest("sha256:short").is_none());
        assert!(normalized_digest("md5:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
        let asset = AssetJson {
            name: "x".into(),
            browser_download_url: "y".into(),
            size: 1,
            digest: Some("sha256:not-hex-".to_string() + &"b".repeat(54)),
        };
        assert!(asset.sha256().is_none(), "残缺摘要不许当成校验通过的依据");
    }

    #[test]
    fn hash_index_lines_pair_hex_with_the_named_asset() {
        let hex = "f".repeat(64);
        let index = format!("{hex}  vpn-parser-1.2.2-windows-x64-setup.exe\n0123  other.txt\n");
        assert_eq!(
            digest_from_hash_index(&index, "vpn-parser-1.2.2-windows-x64-setup.exe").as_deref(),
            Some(hex.as_str())
        );
        assert!(digest_from_hash_index(&index, "vpn-parser-1.2.1-windows-x64-setup.exe").is_none());
        // 单空格分隔也得认（不同工具写的格式）
        assert!(digest_from_hash_index(&format!("{hex} a.exe"), "a.exe").is_some());
    }

    fn release(tag: &str, assets: Vec<AssetJson>) -> ReleaseJson {
        ReleaseJson {
            tag_name: tag.to_string(),
            name: Some(tag.to_string()),
            published_at: None,
            html_url: None,
            body: None,
            assets,
            prerelease: false,
            draft: false,
        }
    }

    #[test]
    fn release_summary_reports_both_directions_and_missing_asset() {
        let hex = "c".repeat(64);
        let info = summarize(
            release(
                "v1.2.2",
                vec![asset(
                    "vpn-parser-1.2.2-windows-x64-setup.exe",
                    Some(&format!("sha256:{hex}")),
                )],
            ),
            "1.2.1",
            UpdateChannel::Stable,
        );
        assert!(info.update_available);
        assert!(info.downloadable(), "有新版又有安装包才允许下载");
        assert!(!info.prerelease);
        assert!(info.message.contains("可更新至 v1.2.2"), "{}", info.message);
        assert!(!info.message.contains("预发"), "稳定版不该被标成预发");
        // 摘要文件找不到时至少给我一个可试的原链
        assert_eq!(
            info.hash_index_url.as_deref().unwrap(),
            "https://github.com/mateo-ki/vpn-parser/releases/download/v1.2.2/sha256.txt"
        );

        let same = summarize(
            release(
                "v1.2.1",
                vec![asset("vpn-parser-1.2.1-windows-x64-setup.exe", None)],
            ),
            "1.2.1",
            UpdateChannel::Stable,
        );
        assert!(!same.update_available);
        assert!(!same.downloadable(), "已经是最新版就不该给下载按钮");
        assert!(same.message.contains("已是最新版本"), "{}", same.message);

        let no_asset = summarize(
            release("v9.9.9", vec![asset("notes.txt", None)]),
            "1.2.1",
            UpdateChannel::Stable,
        );
        assert!(no_asset.asset.is_none());
        // "少资产"这件事要说清楚，但**具体哪种包型是平台变量**：Ubuntu 上就该说 deb 包
        // （这台 WSL 实测说 "没有 deb 包资产"），拿 Windows 的文案当普适断言会把测试写成平台专属。
        assert!(
            no_asset.message.contains("没有") && no_asset.message.contains("资产"),
            "{}",
            no_asset.message
        );
        #[cfg(windows)]
        assert!(
            no_asset.message.contains("没有 Windows 安装包"),
            "{}",
            no_asset.message
        );
        assert!(!no_asset.update_available, "没有安装包就别催用户更新");
    }

    /// 预发通道拿到的 beta 必须在界面上说清楚它是预发版。
    #[test]
    fn prerelease_summary_says_so() {
        let mut beta = release(
            "v1.4.0-beta.1",
            vec![asset("vpn-parser-1.4.0-beta.1-windows-x64-setup.exe", None)],
        );
        beta.prerelease = true;
        let info = summarize(beta, "1.3.0", UpdateChannel::Insider);
        assert!(info.update_available);
        assert!(info.prerelease);
        assert!(info.message.contains("预发"), "{}", info.message);
    }

    #[test]
    fn ladder_puts_fast_mirrors_first_and_never_proxies_them() {
        let url = "https://github.com/mateo-ki/vpn-parser/releases/download/v1.2.2/a.exe";
        let proxies = vec![Some("http://127.0.0.1:9980".to_string()), None];
        let ladder = download_ladder(url, &proxies);
        assert_eq!(ladder.len(), 4, "两个镜像 + 一条代理 + 官方直连");
        assert!(
            ladder[0].url.starts_with("https://gh-proxy.com/"),
            "{:?}",
            ladder[0]
        );
        assert!(ladder[1].url.starts_with("https://ghfast.top/"));
        for channel in &ladder[..2] {
            assert!(channel.proxy.is_none(), "{} 镜像不许挂代理", channel.label);
        }
        assert_eq!(ladder[2].label, "代理 http://127.0.0.1:9980");
        assert_eq!(ladder[2].url, url, "走代理只能用官方原链");
        assert_eq!(ladder[3].url, url);
        assert!(ladder[3].proxy.is_none());
        // 没有任何代理时，候选里那个 None（直连）不该再多出一行
        assert_eq!(download_ladder(url, &[None]).len(), 3);
    }

    #[test]
    fn progress_fraction_and_rate_stay_sane() {
        let mut progress = AppUpdateProgress {
            stage: AppUpdateStage::Downloading,
            downloaded: 50,
            total: 200,
            bytes_per_second: 1024.0,
            message: String::new(),
        };
        assert_eq!(progress.fraction(), 0.25);
        assert_eq!(progress.rate(), 1024);
        progress.total = 0;
        assert_eq!(progress.fraction(), 0.0, "总长未知时进度条不能除以零");
        progress.downloaded = 999;
        progress.total = 100;
        assert_eq!(progress.fraction(), 1.0, "多收了字节也不能画到 100% 之外");
        assert_eq!(AppUpdateStage::Verifying.label(), "校验");
    }

    #[test]
    fn release_item_converts_to_info_for_upgrade_downgrade_and_reinstall() {
        let upgrade_item = ReleaseItem {
            tag: "v1.9.0".into(),
            name: "VPN Parser v1.9.0".into(),
            published_at: Some("2026-10-20".into()),
            prerelease: false,
            html_url: "https://github.com/mateo-ki/vpn-parser/releases/tag/v1.9.0".into(),
            body: Some("更新说明".into()),
            asset_name: Some("vpn-parser-1.9.0-windows-x64-setup.exe".into()),
            asset_size: 15_000_000,
            asset_url: Some("https://example.com/download".into()),
            asset_sha256: Some("abcd".into()),
            hash_index_url: None,
            is_current: false,
        };
        let upgrade_info = upgrade_item.to_release_info("1.8.8");
        assert!(upgrade_info.update_available);
        assert!(upgrade_info.can_install());
        assert!(upgrade_info.message.contains("可更新至 v1.9.0"));

        let downgrade_item = ReleaseItem {
            tag: "v1.8.6".into(),
            name: "VPN Parser v1.8.6".into(),
            published_at: Some("2026-10-16".into()),
            prerelease: false,
            html_url: "https://github.com/mateo-ki/vpn-parser/releases/tag/v1.8.6".into(),
            body: None,
            asset_name: Some("vpn-parser-1.8.6-windows-x64-setup.exe".into()),
            asset_size: 14_000_000,
            asset_url: Some("https://example.com/download_old".into()),
            asset_sha256: None,
            hash_index_url: None,
            is_current: false,
        };
        let downgrade_info = downgrade_item.to_release_info("1.8.8");
        assert!(!downgrade_info.update_available);
        assert!(downgrade_info.can_install(), "历史老版本只要有安装包就允许安装");
        assert!(downgrade_info.message.contains("回退/切换至 v1.8.6"));

        let current_item = ReleaseItem {
            tag: "v1.8.8".into(),
            name: "VPN Parser v1.8.8".into(),
            published_at: Some("2026-10-19".into()),
            prerelease: false,
            html_url: "https://github.com/mateo-ki/vpn-parser/releases/tag/v1.8.8".into(),
            body: None,
            asset_name: Some("vpn-parser-1.8.8-windows-x64-setup.exe".into()),
            asset_size: 15_000_000,
            asset_url: Some("https://example.com/download_curr".into()),
            asset_sha256: None,
            hash_index_url: None,
            is_current: true,
        };
        let current_info = current_item.to_release_info("1.8.8");
        assert!(!current_info.update_available);
        assert!(current_info.can_install());
        assert!(current_info.message.contains("当前运行版本"));
    }

    /// 侧栏的↑只在"确实有更新"时亮：查到的版本不比当前新、或者压根没查到，都不许亮。
    #[test]
    fn the_sidebar_marker_only_lights_when_there_is_something_new() {
        let base = AppReleaseInfo {
            tag: "v1.4.1".into(),
            current_version: "1.4.0".into(),
            update_available: true,
            asset: None,
            hash_index_url: None,
            prerelease: false,
            message: "发现新版本 v1.4.1".into(),
            body: None,
        };
        assert_eq!(base.marker(), Some("v1.4.1"));
        let same = AppReleaseInfo {
            update_available: false,
            ..base.clone()
        };
        assert_eq!(same.marker(), None, "没有更新就不该亮");
    }

    #[test]
    fn cancellation_is_checked_before_touching_the_network() {
        let cancelled = AtomicBool::new(true);
        assert!(cancelled.load(Ordering::Relaxed));
        // 安装程序不存在时不许启动（否则会拉起一个报错对话框）
        let missing = installer_cache_dir().join("does-not-exist-setup.exe");
        assert!(launch_installer(&missing).is_err());
    }

    /// 下载好的安装包放在**数据根目录下**（用户 2026-10-14 指定要看得见、不进 `%TEMP%`），
    /// 并且必须是内核 `kernels` 的兄弟目录 —— 这两条是真不变量。
    /// "在 cwd 下面、那一层叫 `data`"只是 Windows 的形态：Linux 上根目录在 XDG，
    /// 照那样断言就是在测路径字符串而不是测这个性质（第一次 Linux 跑就是这样红的）。
    #[test]
    fn installer_cache_lives_under_the_data_root() {
        let dir = installer_cache_dir();
        assert_eq!(dir, crate::data_root().join("updates"));
        assert_eq!(dir.file_name().unwrap(), "updates");
        // 和内核同一层：data/kernels 的兄弟目录。
        assert_eq!(dir.parent(), crate::kernel::managed_kernel_root().parent());

        #[cfg(windows)]
        {
            let cwd = std::env::current_dir().expect("测试总该有 cwd");
            assert!(dir.starts_with(&cwd), "{dir:?} 必须在程序目录 {cwd:?} 下面");
            assert_eq!(dir.parent().unwrap().file_name().unwrap(), "data");
        }
    }

    /// 真实网络手测：走产品那条下载路径把最新安装包拉下来，比对 GitHub 给的摘要。
    /// `cargo test --release -- --ignored real_network_app_download --nocapture`
    /// 下载完就把文件删掉 —— 这条测的是"能不能下下来 + 校验认不认"，不是攒包。
    #[tokio::test]
    #[ignore]
    async fn real_network_app_download_verifies_the_digest() {
        let info = check(None, "0.0.1", UpdateChannel::Stable)
            .await
            .expect("查最新 Release");
        assert!(
            info.downloadable(),
            "{} 没有可下载资产：{}",
            info.tag,
            info.message
        );
        let cancelled = AtomicBool::new(false);
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let path = download(&info, None, &cancelled, &|progress: AppUpdateProgress| {
            let mut guard = seen.lock().unwrap();
            if guard.len() < 4 {
                guard.push(format!(
                    "{:?} {}/{} {}",
                    progress.stage, progress.downloaded, progress.total, progress.message
                ));
            }
        })
        .await
        .expect("下载并校验");
        let meta = std::fs::metadata(&path).expect("下载产物");
        println!(
            "下载成功 {} 字节 = 期望 {}，摘要与 GitHub 的 digest 逐字符一致，路径 {}",
            meta.len(),
            info.asset.as_ref().map(|asset| asset.size).unwrap_or(0),
            path.display()
        );
        println!("进度取样：{:?}", seen.lock().unwrap());
        std::fs::remove_file(&path).ok();
    }

    /// 真实网络手测：两个通道各查一遍，打印拿到的 tag / 是否 prerelease / 资产。
    /// `cargo test --release -- --ignored real_network_app_check --nocapture`
    #[tokio::test]
    #[ignore = "真实网络手测，要能连上 api.github.com"]
    async fn real_network_app_check_prints_versions() {
        for channel in [UpdateChannel::Stable, UpdateChannel::Insider] {
            let info = check(None, current_version(), channel).await.unwrap();
            println!(
                "本机 {} / {}通道 → {}（prerelease={}）有更新={} 资产 {:?}",
                info.current_version,
                channel.label(),
                info.tag,
                info.prerelease,
                info.update_available,
                info.asset.as_ref().map(|asset| (
                    asset.name.clone(),
                    asset.size,
                    asset.sha256.is_some()
                )),
            );
            assert!(info.tag.starts_with('v'));
        }
    }

    #[test]
    fn the_github_token_prefers_the_first_real_one() {
        assert_eq!(
            pick_github_token(["", "   ", " ghpat_abc "]).as_deref(),
            Some("ghpat_abc")
        );
        // 全空 = 宁可不认证，也不发一个空 Authorization 头（GitHub 会当坏凭据直接 401）。
        assert!(pick_github_token(["", "  ", ""]).is_none());
        assert!(pick_github_token([] as [&str; 0]).is_none());
    }

    #[test]
    fn a_rate_limit_is_not_reported_as_a_network_failure() {
        let limited = describe_rejection(reqwest::StatusCode::FORBIDDEN, true);
        assert!(limited.contains("限流"), "{limited}");
        assert!(limited.contains("GITHUB_TOKEN"), "{limited}");
        // 反向对照：403 但没有 x-ratelimit-remaining=0 时不许赖限流。
        let plain = describe_rejection(reqwest::StatusCode::FORBIDDEN, false);
        assert!(!plain.contains("限流"), "{plain}");
        assert!(plain.contains("403"), "{plain}");
        assert_eq!(describe_rejection(reqwest::StatusCode::NOT_FOUND, false), "HTTP 404");
    }

    #[test]
    fn atom_releases_are_parsed_correctly() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <id>tag:github.com,2008:https://github.com/mateo-ki/vpn-parser/releases</id>
  <title>Release notes</title>
  <entry>
    <id>tag:github.com,2008:Repository/1416062168/v1.14.0</id>
    <updated>2026-10-26T13:07:59Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/mateo-ki/vpn-parser/releases/tag/v1.14.0"/>
    <title>VPN Parser v1.14.0</title>
    <content type="html">&lt;h1&gt;Notes&lt;/h1&gt;</content>
  </entry>
  <entry>
    <id>tag:github.com,2008:Repository/1416062168/v1.13.0</id>
    <updated>2026-10-20T10:00:00Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/mateo-ki/vpn-parser/releases/tag/v1.13.0"/>
    <title>VPN Parser v1.13.0</title>
    <content type="html">Older notes</content>
  </entry>
</feed>"#;
        let items = parse_atom_releases(xml, "1.14.0");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].tag, "v1.14.0");
        assert_eq!(items[0].name, "VPN Parser v1.14.0");
        assert_eq!(items[0].published_at.as_deref(), Some("2026-10-26"));
        assert!(items[0].is_current);
        assert!(!items[0].prerelease);
        assert!(items[0].asset_name.is_some());
        assert!(items[0].hash_index_url.is_some());

        assert_eq!(items[1].tag, "v1.13.0");
        assert!(!items[1].is_current);
    }
}

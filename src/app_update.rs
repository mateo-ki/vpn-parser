//! 应用自身的更新：查 GitHub Release → 下载安装包 → 校验 SHA-256 → 交给安装向导。
//!
//! 内核更新走 [`crate::kernel`]，那条路可以把下好的字节直接换进 `data/kernels/`；
//! 应用自己不行 —— `vpn-parser.exe` 正在运行，Windows 不允许覆盖它。所以这里只负责
//! "把安装包取回来并证明它没被改过"，落地交给 Inno 向导（它会先 taskkill 本进程再装）。

use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// 安装包发布在这儿，由 `.github/workflows/release.yml` 用 tag 打出来。
pub const REPOSITORY: &str = "mateo-ki/vpn-parser";
const INSTALLER_SUFFIX: &str = "-windows-x64-setup.exe";
const HASH_INDEX_NAME: &str = "sha256.txt";
/// 安装包现在 ~15 MB，留 200 MB 上限只为挡住"镜像回了一个 HTML 或者整个盘"。
const MAX_INSTALLER_SIZE: u64 = 200 * 1024 * 1024;

pub fn current_version() -> &'static str {
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
    pub fn downloadable(&self) -> bool {
        self.update_available && self.asset.is_some()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ReleaseJson {
    tag_name: String,
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

/// 挑安装包资产：先要名字正好等于 `vpn-parser-<tag 去掉 v>-windows-x64-setup.exe` 的那一个
/// （同一 Release 里可能留着上一版的安装包），一个都对不上才退而求其次取后缀匹配的最后一个。
fn pick_installer_asset(assets: &[AssetJson], tag: &str) -> Option<AppAsset> {
    let expected = format!(
        "vpn-parser-{}{INSTALLER_SUFFIX}",
        tag.trim_start_matches('v')
    );
    let found = assets
        .iter()
        .position(|asset| asset.name == expected)
        .or_else(|| {
            assets
                .iter()
                .rposition(|asset| asset.name.ends_with(INSTALLER_SUFFIX))
        })?;
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
        let result = transport
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        let picked: Result<Option<ReleaseJson>> = match (channel, result) {
            (UpdateChannel::Stable, Ok(response)) => response
                .json::<ReleaseJson>()
                .await
                .map(Some)
                .map_err(|error| anyhow::anyhow!(error)),
            (UpdateChannel::Insider, Ok(response)) => response
                .json::<Vec<ReleaseJson>>()
                .await
                .map(|list| list.into_iter().find(|release| !release.draft))
                .map_err(|error| anyhow::anyhow!(error)),
            (_, Err(error)) => Err(anyhow::anyhow!(error)),
        };
        match picked {
            Ok(Some(release)) => return Ok(summarize(release, current, channel)),
            Ok(None) => failures.push(format!("{}: 这个通道里还没有已发布的版本", transport.label)),
            Err(error) => failures.push(format!("{}: {error}", transport.label)),
        }
    }
    bail!(
        "读取 {REPOSITORY} 最新 Release 失败：{}",
        failures.join("；")
    )
}

fn summarize(release: ReleaseJson, current: &str, channel: UpdateChannel) -> AppReleaseInfo {
    let asset = pick_installer_asset(&release.assets, &release.tag_name);
    let update_available = asset.is_some() && version_is_newer(&release.tag_name, current);
    let message = if asset.is_none() {
        format!(
            "{} Release {} 里没有 Windows 安装包资产",
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
            let installer = release
                .assets
                .iter()
                .find(|asset| asset.name.ends_with(INSTALLER_SUFFIX))?;
            Some(
                installer
                    .browser_download_url
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
    }
}

/// 安装包暂存在**软件目录下的 `data/updates`**（和内核的 `data/kernels` 同一层，用户 2026-10-14 指定）：
/// 装完想留着或手动删掉都找得到，不会像 `%TEMP%` 那样被清理工具悄悄收走、也看不见。
/// 诊断壳（VPN_PARSER_DIAG）的 cwd 已经切到临时目录，所以它天然落在自己那份里，不碰真安装目录。
pub fn installer_cache_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("data")
        .join("updates")
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
    if asset.size == 0 || asset.size > MAX_INSTALLER_SIZE {
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
                crate::startup_log(&format!(
                    "[更新] 安装包已下载并校验: {}（{} 字节，通道 {label}）",
                    target.display(),
                    asset.size
                ));
                progress(AppUpdateProgress {
                    stage: AppUpdateStage::Ready,
                    downloaded: asset.size,
                    total: asset.size,
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
                    total: asset.size,
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
    if written != asset.size {
        bail!("只下到 {written} / {} 字节", asset.size);
    }
    progress(AppUpdateProgress {
        stage: AppUpdateStage::Verifying,
        downloaded: written,
        total: asset.size,
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
pub fn launch_installer(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("安装包不在了: {}", path.display());
    }
    std::process::Command::new(path)
        .current_dir(path.parent().unwrap_or_else(|| Path::new(".")))
        .spawn()
        .with_context(|| format!("启动安装程序失败: {}", path.display()))?;
    Ok(())
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
    fn cancellation_is_checked_before_touching_the_network() {
        let cancelled = AtomicBool::new(true);
        assert!(cancelled.load(Ordering::Relaxed));
        // 安装程序不存在时不许启动（否则会拉起一个报错对话框）
        let missing = installer_cache_dir().join("does-not-exist-setup.exe");
        assert!(launch_installer(&missing).is_err());
    }

    /// 下载好的安装包放在**软件目录下**（用户 2026-10-14 指定），不是 `%TEMP%`：
    /// 装完要留着还是手删都看得见，也不会被清理工具悄悄收走。
    #[test]
    fn installer_cache_lives_under_the_program_directory() {
        let dir = installer_cache_dir();
        let cwd = std::env::current_dir().expect("测试总该有 cwd");
        assert!(dir.starts_with(&cwd), "{dir:?} 必须在程序目录 {cwd:?} 下面");
        assert_eq!(dir.file_name().unwrap(), "updates");
        assert_eq!(dir.parent().unwrap().file_name().unwrap(), "data");
        // 和内核同一层：data/kernels 的兄弟目录。
        assert_eq!(dir.parent(), crate::kernel::managed_kernel_root().parent());
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
}

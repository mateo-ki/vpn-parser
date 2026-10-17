use super::{
    read_kernel_version, read_kernel_version_with_env, KernelArchive, KernelInfo, KernelKind,
    KernelPlatform,
};
use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const MAX_ARCHIVE_SIZE: u64 = 150 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelReleaseInfo {
    pub kind: KernelKind,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub asset_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelUpdateResult {
    pub kind: KernelKind,
    pub version: String,
    pub path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelUpdateStage {
    Checking,
    Downloading,
    Verifying,
    Extracting,
    Installing,
}

impl KernelUpdateStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "检查版本",
            Self::Downloading => "下载",
            Self::Verifying => "校验",
            Self::Extracting => "解压",
            Self::Installing => "安装",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelUpdateProgress {
    pub kind: KernelKind,
    pub stage: KernelUpdateStage,
    pub downloaded: u64,
    pub total: u64,
    pub bytes_per_second: f64,
    pub message: String,
}

impl KernelUpdateProgress {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.downloaded as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
    /// 内置兜底 Release 用的标记：不是"最新"，是"已知能下下来、SHA-256 核对过"的那一版。
    #[serde(default)]
    fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// 一条可用的下载通道：label 用来说清"这次是走哪条路拿到的"。
#[derive(Debug, Clone)]
pub struct Transport {
    pub label: String,
    /// 查 Release / 读 JSON 用：4 秒连不上、15 秒读不到就放弃，换下一条通道。
    /// 这是用户 2026-10 要的"快速失败"语义，别把它放宽。
    client: Client,
    /// 下 22MB 资产用：只放宽到"连接 10 秒、没有逐次读超时"。
    /// 理由有实测：WSL 出境到 ghfast 的 TLS 握手要 7 秒上下，同一个资产整趟 36.6 秒，
    /// 而 4 秒的连接预算让预备下载在第 5 秒就报 `error sending request` —— 元数据那套
    /// 快失败对这个量级的传输是错的，但我不想因此牺牲元数据的快失败，所以分成两个 client。
    stream_client: Client,
}

#[derive(Debug, Clone)]
pub struct KernelUpdater {
    transports: Vec<Transport>,
    root: PathBuf,
}

/// 内核下载/查版本的代理候选：**应用内代理 → Windows 系统代理 → 直连**，和 Gist 发现同一套。
/// 之前只认应用内代理，所以你用 v2rayN 之类外部代理时，内核下载是纯直连（实测 0.116 MB/s）。
pub fn proxy_candidates(app_proxy: Option<&str>) -> Vec<Option<String>> {
    let mut chain: Vec<Option<String>> = Vec::new();
    if let Some(url) = app_proxy.map(str::trim).filter(|url| !url.is_empty()) {
        chain.push(Some(url.to_string()));
    }
    if let Some(url) = crate::system_proxy::current_system_proxy_url() {
        if !chain.contains(&Some(url.clone())) {
            chain.push(Some(url));
        }
    }
    if !chain.contains(&None) {
        chain.push(None);
    }
    chain
}

/// gh-proxy / ghfast 两个镜像前缀。它们只代理 `github.com` 的文件直链，**不代理
/// `api.github.com`**，所以查版本永远走代理候选或直连。
pub const ASSET_MIRROR_PREFIXES: [(&str, &str); 2] = [
    ("gh-proxy", "https://gh-proxy.com/"),
    ("ghfast", "https://ghfast.top/"),
];

/// 选定的下载方式对应的一组 URL（含自动备用镜像）。
pub fn candidate_urls_for(source: KernelSource, direct_url: &str) -> Vec<(String, String)> {
    match source {
        KernelSource::GhProxy => vec![
            (asset_url_for(KernelSource::GhProxy, direct_url), "gh-proxy 镜像".to_string()),
            (asset_url_for(KernelSource::GhFast, direct_url), "ghfast 镜像 (备用)".to_string()),
        ],
        KernelSource::GhFast => vec![
            (asset_url_for(KernelSource::GhFast, direct_url), "ghfast 镜像".to_string()),
            (asset_url_for(KernelSource::GhProxy, direct_url), "gh-proxy 镜像 (备用)".to_string()),
        ],
        KernelSource::Official => vec![
            (asset_url_for(KernelSource::Official, direct_url), "官方直连".to_string()),
            (asset_url_for(KernelSource::GhFast, direct_url), "ghfast 镜像 (备用)".to_string()),
        ],
        KernelSource::Proxy => vec![
            (asset_url_for(KernelSource::Proxy, direct_url), "走代理".to_string()),
        ],
        KernelSource::Local => vec![],
    }
}

/// 选定的下载方式对应的那一个 URL —— 一次只有一条连接，不再并发抢跑。
pub fn asset_url_for(source: KernelSource, direct_url: &str) -> String {
    match source.prefix() {
        Some(prefix) => format!("{prefix}{direct_url}"),
        None => direct_url.to_string(),
    }
}

/// 内核下载走哪条路（存在 `settings.kernel_source`，「下载内核」弹框里选）。
/// 五种方式都是"一条路走到底"：不同时开几条连接抢跑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelSource {
    /// 只走 gh-proxy.com 前缀
    GhProxy,
    /// 只走 ghfast.top 前缀
    GhFast,
    /// 只用 GitHub 原链、不经任何代理
    Official,
    /// 只走代理（应用内代理 → 系统代理），用官方原链
    Proxy,
    /// 从本地 zip 安装，不联网
    Local,
}

pub struct SourceOption {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub source: KernelSource,
}

/// 弹框里按这个顺序列五种方式；文案带上实测速度，免得凭感觉选。
/// 顺序就是推荐顺序：ghfast 排第一（它是默认值）。
pub const SOURCE_OPTIONS: [SourceOption; 5] = [
    SourceOption {
        id: "ghfast",
        label: "ghfast 镜像（默认）",
        hint: "只连 https://ghfast.top/ 前缀，一条连接下到底（实测 8.13 MB/s；2026-10-14 那家 gh-proxy 停在 18.6/22.5MB 时它 36.8 秒完整下完、SHA-256 一致）",
        source: KernelSource::GhFast,
    },
    SourceOption {
        id: "gh-proxy",
        label: "gh-proxy 镜像",
        hint: "只连 https://gh-proxy.com/ 前缀，一条连接下到底（曾实测 4.87 MB/s，但 2026-10-14 实测会把 22MB 掐断在 18.6MB —— 别当默认）",
        source: KernelSource::GhProxy,
    },
    SourceOption {
        id: "official",
        label: "官方直连",
        hint: "只用 GitHub 原链、不经任何代理（实测 0.12 MB/s，22MB 会被 90 秒超时掐成半截）",
        source: KernelSource::Official,
    },
    SourceOption {
        id: "proxy",
        label: "走代理",
        hint: "应用内代理 → 系统代理 依次试（一次只跑一条），只连官方原链；代理没在跑时直接报没有可用通道",
        source: KernelSource::Proxy,
    },
    SourceOption {
        id: "local",
        label: "本地文件导入",
        hint: "从你已有的 zip 安装，完全不联网；找不到对应官方资产时无法校验 SHA-256",
        source: KernelSource::Local,
    },
];

pub fn source_from_id(id: &str) -> KernelSource {
    SOURCE_OPTIONS
        .iter()
        .find(|option| option.id == id)
        .map(|option| option.source)
        // 认不出来的存值（旧的 "race" 三家并发抢跑那一版、手改过的）落到现在的默认那家。
        .unwrap_or(KernelSource::GhFast)
}

impl KernelSource {
    pub fn id(self) -> &'static str {
        match self {
            Self::GhProxy => "gh-proxy",
            Self::GhFast => "ghfast",
            Self::Official => "official",
            Self::Proxy => "proxy",
            Self::Local => "local",
        }
    }

    pub fn label(self) -> &'static str {
        SOURCE_OPTIONS
            .iter()
            .find(|option| option.source == self)
            .map(|option| option.label)
            .unwrap_or("ghfast 镜像（默认）")
    }

    fn prefix(self) -> Option<&'static str> {
        ASSET_MIRROR_PREFIXES
            .iter()
            .find(|(name, _)| *name == self.id())
            .map(|(_, prefix)| *prefix)
    }

    /// 这个方式允许哪些通道下标。
    fn allows(self, transport_label: &str) -> bool {
        let direct = transport_label == "直连";
        match self {
            // 镜像站本身就在国内，走代理纯属绕路；本地导入根本不联网。
            Self::GhProxy | Self::GhFast | Self::Official => direct,
            Self::Proxy => !direct,
            Self::Local => true,
        }
    }
}

async fn fetch_json<T: DeserializeOwned>(client: &Client, url: &str) -> Result<T> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("{error:#}"))?
        .error_for_status()?;
    response.json::<T>().await.map_err(|error| error.into())
}

impl KernelUpdater {
    pub fn new(root: PathBuf, app_proxy: Option<&str>) -> Result<Self> {
        let mut transports = Vec::new();
        for candidate in proxy_candidates(app_proxy) {
            // 同一个通道要两个 client（见 `Transport` 上那段注释），而 reqwest 的
            // `ClientBuilder` 不是 Clone，所以代理与 UA 这些共同项对折两遍。
            let mut metadata = Client::builder()
                .user_agent("vpn-parser/0.1 kernel-updater")
                .http1_only()
                .connect_timeout(std::time::Duration::from_secs(4))
                .read_timeout(std::time::Duration::from_secs(15));
            let mut streaming = Client::builder()
                .user_agent("vpn-parser/0.1 kernel-updater")
                .http1_only()
                .connect_timeout(std::time::Duration::from_secs(10));
            // 刻意不调 read_timeout：reqwest 默认就没有逐次读超时。
            let label = match candidate.as_deref() {
                Some(url) => {
                    let proxy = reqwest::Proxy::all(url)?;
                    metadata = metadata.proxy(proxy.clone());
                    streaming = streaming.proxy(proxy);
                    if app_proxy.is_some() && Some(url) == app_proxy {
                        format!("应用内代理 {url}")
                    } else {
                        format!("系统代理 {url}")
                    }
                }
                None => "直连".to_string(),
            };
            transports.push(Transport {
                label,
                client: metadata.build()?,
                stream_client: streaming.build()?,
            });
        }
        Ok(Self { transports, root })
    }

    pub async fn check(&self, info: &KernelInfo) -> Result<KernelReleaseInfo> {
        let release = self.fetch_release(info.kind).await?;
        let asset_name = info.kind.asset_name(&release.tag_name);
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .with_context(|| format!("官方 Release 未找到资产 {asset_name}"))?;
        require_sha256(asset)?;
        let update_available = match info.version.as_deref() {
            Some(current) => !version_matches(current, &release.tag_name),
            None => true,
        };
        Ok(KernelReleaseInfo {
            kind: info.kind,
            current_version: info.version.clone(),
            latest_version: Some(release.tag_name.clone()),
            update_available,
            asset_name: Some(asset.name.clone()),
            message: if update_available {
                format!("{} 可更新至 {}", info.kind.display_name(), release.tag_name)
            } else {
                format!("{} 已是最新版本", info.kind.display_name())
            },
        })
    }

    /// 更新/安装内核。`source` 决定走哪条路；`local_file` 只在 [`KernelSource::Local`] 下用。
    pub async fn update<F>(
        &self,
        kind: KernelKind,
        source: KernelSource,
        local_file: Option<&Path>,
        cancelled: &AtomicBool,
        progress: &F,
    ) -> Result<KernelUpdateResult>
    where
        F: Fn(KernelUpdateProgress) + Send + Sync,
    {
        match source {
            KernelSource::Local => {
                self.install_local(kind, local_file, cancelled, progress)
                    .await
            }
            _ => {
                self.update_from_network(kind, source, cancelled, progress)
                    .await
            }
        }
    }

    async fn install_local<F>(
        &self,
        kind: KernelKind,
        local_file: Option<&Path>,
        cancelled: &AtomicBool,
        progress: &F,
    ) -> Result<KernelUpdateResult>
    where
        F: Fn(KernelUpdateProgress) + Send + Sync,
    {
        let path = local_file.ok_or_else(|| anyhow::anyhow!("请先选择要导入的内核压缩包"))?;
        ensure_not_cancelled(cancelled)?;
        progress(KernelUpdateProgress {
            kind,
            stage: KernelUpdateStage::Checking,
            downloaded: 0,
            total: 0,
            bytes_per_second: 0.0,
            message: format!("正在读取本地文件 {}", path.display()),
        });
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("读取本地文件失败: {}", path.display()))?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_ARCHIVE_SIZE {
            bail!("本地压缩包大小异常: {} bytes", bytes.len());
        }
        // 能对上官方资产名就顺手校验一次，对不上就明确告诉他没校验。
        let expected = self.fetch_release(kind).await.ok().and_then(|release| {
            let name = path.file_name()?.to_string_lossy().to_string();
            release
                .assets
                .iter()
                .find(|asset| asset.name == name)
                .and_then(|asset| require_sha256(asset).ok())
                .map(|hash| (hash.to_string(), release.tag_name.clone()))
        });
        if let Some((hash, _)) = expected.as_ref() {
            progress(KernelUpdateProgress {
                kind,
                stage: KernelUpdateStage::Verifying,
                downloaded: bytes.len() as u64,
                total: bytes.len() as u64,
                bytes_per_second: 0.0,
                message: "本地文件对上了官方资产，正在校验 SHA-256".into(),
            });
            verify_sha256(&bytes, hash)?;
        } else {
            progress(KernelUpdateProgress {
                kind,
                stage: KernelUpdateStage::Verifying,
                downloaded: bytes.len() as u64,
                total: bytes.len() as u64,
                bytes_per_second: 0.0,
                message: "本地导入：对不上官方资产，跳过 SHA-256 校验".into(),
            });
        }
        let expected_tag = expected.as_ref().map(|(_, tag)| tag.clone());
        // 本地导入的容器格式按**文件名的后缀**判，不能按当前平台默认：
        // 他在 Linux 上手拖一个 .zip 进来是完全可能的，反过来也一样。
        let format = KernelArchive::from_asset_name(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
        )
        .ok_or_else(|| anyhow::anyhow!("本地内核文件只认 .zip、.gz、.tar.gz 三种容器"))?;
        self.install_bytes(
            kind,
            bytes,
            expected_tag.as_deref(),
            format,
            cancelled,
            progress,
        )
        .await
    }

    async fn update_from_network<F>(
        &self,
        kind: KernelKind,
        source: KernelSource,
        cancelled: &AtomicBool,
        progress: &F,
    ) -> Result<KernelUpdateResult>
    where
        F: Fn(KernelUpdateProgress) + Send + Sync,
    {
        ensure_not_cancelled(cancelled)?;
        progress(KernelUpdateProgress {
            kind,
            stage: KernelUpdateStage::Checking,
            downloaded: 0,
            total: 0,
            bytes_per_second: 0.0,
            message: format!(
                "正在检查 {} 官方 Release（依次试代理与直连）",
                kind.display_name()
            ),
        });
        let (release, transport_index) = self.fetch_release_with_transport(kind).await?;
        if release.fallback {
            progress(KernelUpdateProgress {
                kind,
                stage: KernelUpdateStage::Checking,
                downloaded: 0,
                total: 0,
                bytes_per_second: 0.0,
                message: format!(
                    "{} 官方 Release 查不到也没有缓存，改用内置兜底版本 {}",
                    kind.display_name(),
                    release.tag_name
                ),
            });
        }
        crate::startup_log(&format!(
            "[内核] {} Release 来源={} tag={} 取到版本用的通道 #{transport_index}",
            kind.display_name(),
            if release.fallback {
                "内置兜底"
            } else {
                "api.github.com/本地缓存"
            },
            release.tag_name
        ));
        let candidates = kind.asset_candidates(&release.tag_name);
        let mut install_failures = Vec::new();

        for candidate_name in &candidates {
            ensure_not_cancelled(cancelled)?;
            let Some(asset) = release
                .assets
                .iter()
                .find(|asset| &asset.name == candidate_name)
                .cloned()
            else {
                continue;
            };

            let expected_hash = match require_sha256(&asset) {
                Ok(h) => h.to_string(),
                Err(e) => {
                    install_failures.push(format!("{candidate_name}: {e}"));
                    continue;
                }
            };

            crate::startup_log(&format!(
                "[内核] {} 目标资产 {} / {} 字节，SHA-256 期望 {}…（下载方式 {}）",
                kind.display_name(),
                asset.name,
                asset.size,
                &expected_hash[..expected_hash.len().min(12)],
                source.label()
            ));
            if asset.size == 0 || asset.size > MAX_ARCHIVE_SIZE {
                install_failures.push(format!("{candidate_name}: 压缩包大小异常"));
                continue;
            }

            let bytes = match self
                .download_asset(kind, &asset, source, transport_index, cancelled, progress)
                .await
            {
                Ok(b) => b,
                Err(e) => {
                    install_failures.push(format!("{candidate_name} 下载失败: {e}"));
                    continue;
                }
            };

            if bytes.len() as u64 != asset.size {
                let msg = format!("下载大小不一致: 期望 {}，实际 {}", asset.size, bytes.len());
                crate::startup_log(&format!("[内核] {} {candidate_name} {msg}", kind.display_name()));
                install_failures.push(format!("{candidate_name}: {msg}"));
                continue;
            }

            crate::startup_log(&format!(
                "[内核] {} 下载完成 {} 字节，开始校验 SHA-256",
                kind.display_name(),
                bytes.len()
            ));
            progress(KernelUpdateProgress {
                kind,
                stage: KernelUpdateStage::Verifying,
                downloaded: asset.size,
                total: asset.size,
                bytes_per_second: 0.0,
                message: "正在校验 SHA-256".into(),
            });
            ensure_not_cancelled(cancelled)?;
            if let Err(error) = verify_sha256(&bytes, &expected_hash) {
                crate::startup_log(&format!("[内核] {} {candidate_name} {error}", kind.display_name()));
                install_failures.push(format!("{candidate_name}: {error}"));
                continue;
            }
            crate::startup_log(&format!("[内核] {} {candidate_name} SHA-256 校验通过", kind.display_name()));

            let archive_format = match KernelArchive::from_asset_name(&asset.name) {
                Some(f) => f,
                None => {
                    install_failures.push(format!("{candidate_name}: 容器格式认不出来"));
                    continue;
                }
            };

            match self
                .install_bytes(
                    kind,
                    bytes,
                    Some(release.tag_name.as_str()),
                    archive_format,
                    cancelled,
                    progress,
                )
                .await
            {
                Ok(result) => return Ok(result),
                Err(error) => {
                    crate::startup_log(&format!(
                        "[内核] {} 资产 {} 落地验证失败: {error}，若有备选变体则尝试下一个...",
                        kind.display_name(),
                        asset.name
                    ));
                    progress(KernelUpdateProgress {
                        kind,
                        stage: KernelUpdateStage::Downloading,
                        downloaded: 0,
                        total: 0,
                        bytes_per_second: 0.0,
                        message: format!("{} 运行失败，正在尝试兼容版内核...", asset.name),
                    });
                    install_failures.push(format!("{candidate_name}: {error}"));
                }
            }
        }

        bail!("安装 {} 失败：{}", kind.display_name(), install_failures.join("；"))
    }

    /// 校验过字节之后的统一落地：解压到暂存 → 跑一次拿版本 → 备份旧内核 → 替换 → 失败回滚。
    /// `expected_tag` 是官方版本号，本地导入且对不上资产时传 None（只要求内核能报版本）。
    async fn install_bytes<F>(
        &self,
        kind: KernelKind,
        bytes: Vec<u8>,
        expected_tag: Option<&str>,
        format: KernelArchive,
        cancelled: &AtomicBool,
        progress: &F,
    ) -> Result<KernelUpdateResult>
    where
        F: Fn(KernelUpdateProgress) + Send + Sync,
    {
        let total = bytes.len() as u64;
        let kind_dir = self.root.join(kind.directory_name());
        let target = kind_dir.join(kind.executable_name());
        let staging = kind_dir.join(staging_name(kind));
        let legacy_staging = kind_dir.join(format!(".{}.staging", kind.executable_name()));
        let backup = kind_dir.join(format!("{}.backup", kind.executable_name()));
        tokio::fs::create_dir_all(&kind_dir).await?;
        // 上一次尝试留下的残渣先清掉：残留文件会让"这次到底跑没跑起来"看不出来。
        let _ = tokio::fs::remove_file(&staging).await;
        let _ = tokio::fs::remove_file(&legacy_staging).await;
        let staging_for_extract = staging.clone();
        progress(KernelUpdateProgress {
            kind,
            stage: KernelUpdateStage::Extracting,
            downloaded: total,
            total,
            bytes_per_second: 0.0,
            message: "正在解压并验证暂存内核".into(),
        });
        ensure_not_cancelled(cancelled)?;
        crate::startup_log(&format!(
            "[内核] {} 解压到暂存 {}",
            kind.display_name(),
            staging.display()
        ));
        tokio::task::spawn_blocking(move || {
            extract_executable(&bytes, format, kind, &staging_for_extract)
        })
        .await??;

        // 这一步是整个"预备"最容易被外因（虚拟机 CPU / 杀软）打掉的地方，
        // 所以三种结果都写进 startup.log —— VM 上没有控制台，文件是唯一看得见的证据。
        let mut staged = read_kernel_version(kind, &staging).await;
        match &staged {
            Ok(Some(line)) => crate::startup_log(&format!("[内核] 暂存内核报版本: {line}")),
            Ok(None) => crate::startup_log("[内核] 暂存内核运行了但没打印任何版本行"),
            Err(error) => crate::startup_log(&format!("[内核] 暂存内核探测失败: {error}")),
        }
        // 第一枪没响就用 Go 的 CPU 特性总开关再打一枪：能分清"虚拟机 CPUID 撒谎"
        // 和"杀软挂钩 / 环境问题"这两种完全不同的下场。
        let mut compat_only = false;
        if !matches!(staged, Ok(Some(_))) {
            let second =
                read_kernel_version_with_env(kind, &staging, &crate::kernel::GODEBUG_COMPAT).await;
            match &second {
                Ok(Some(line)) => {
                    compat_only = true;
                    crate::startup_log(&format!(
                        "[内核] {} 只有带 GODEBUG=cpu.all=off 才跑得起来: {line} —— 这台机器的 CPUID 不可信，启动内核时也得带上（第一次报错见上一行）",
                        kind.display_name()
                    ));
                    staged = second;
                }
                Ok(None) => {
                    crate::startup_log("[内核] 带 GODEBUG=cpu.all=off 运行了但没有打印任何版本行");
                }
                Err(err) => {
                    crate::startup_log(&format!(
                        "[内核] 带 GODEBUG=cpu.all=off 仍然跑不起来: {err} —— 更像杀软挂钩或访客环境问题。手动验证：把下面这个文件另存为 x.exe 后运行 x.exe -v"
                    ));
                    crate::startup_log(&format!("[内核] 暂存文件: {}", staging.display()));
                }
            }
        }
        let staged_version = match staged {
            Ok(Some(v)) => v,
            Ok(None) => {
                let _ = tokio::fs::remove_file(&staging).await;
                bail!("{} 暂存内核未返回版本", kind.display_name());
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&staging).await;
                return Err(e);
            }
        };
        if let Some(tag) = expected_tag {
            if !version_matches(&staged_version, tag) {
                crate::startup_log(&format!(
                    "[内核] {} 暂存版本 {staged_version} 对不上期望的 {tag}，丢弃暂存文件",
                    kind.display_name()
                ));
                let _ = tokio::fs::remove_file(&staging).await;
                bail!("暂存内核版本不匹配: {staged_version}");
            }
        }

        ensure_not_cancelled(cancelled)?;
        crate::startup_log(&format!(
            "[内核] {} 准备替换正式路径 {}（旧文件备份为 {}）",
            kind.display_name(),
            target.display(),
            backup.display()
        ));
        progress(KernelUpdateProgress {
            kind,
            stage: KernelUpdateStage::Installing,
            downloaded: total,
            total,
            bytes_per_second: 0.0,
            message: "正在备份并替换内核".into(),
        });
        let backup_path = if target.is_file() {
            let _ = tokio::fs::remove_file(&backup).await;
            tokio::fs::rename(&target, &backup)
                .await
                .context("备份旧内核失败")?;
            Some(backup.clone())
        } else {
            None
        };
        if let Err(error) = tokio::fs::rename(&staging, &target).await {
            if backup.is_file() {
                let _ = tokio::fs::rename(&backup, &target).await;
            }
            return Err(error).context("替换内核失败，已尝试回滚");
        }
        if let Err(error) = read_kernel_version(kind, &target).await {
            crate::startup_log(&format!(
                "[内核] {} 替换后仍然跑不起来: {error} —— 已回滚到旧内核",
                kind.display_name()
            ));
            let _ = tokio::fs::remove_file(&target).await;
            if backup.is_file() {
                let _ = tokio::fs::rename(&backup, &target).await;
            }
            return Err(error).context("新内核验证失败，已回滚");
        }

        crate::startup_log(&format!(
            "[内核] {} 安装完成: {}（版本 {}，备份 {}）",
            kind.display_name(),
            target.display(),
            staged_version,
            backup_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "没有旧内核可备份".to_string())
        ));
        let mut message = format!("{} 已更新至 {}", kind.display_name(), staged_version);
        if compat_only {
            // 界面上不能只说"成功"：这台机器的内核要靠 CPU 兼容开关才跑得起来。
            message.push_str("（这台机器需要 CPU 兼容模式，启动内核还要加 GODEBUG=cpu.all=off）");
        }
        Ok(KernelUpdateResult {
            kind,
            version: staged_version.clone(),
            path: target,
            backup_path,
            message,
        })
    }

    /// 并发竞速下载：镜像与官方原链按序尝试，主通道失败/停滞自动尝试备用通道。
    /// 一条通道全失败才换下一条；`preferred` 是查版本时先赢的那条通道。
    async fn download_asset<F>(
        &self,
        kind: KernelKind,
        asset: &GithubAsset,
        source: KernelSource,
        preferred: usize,
        cancelled: &AtomicBool,
        progress: &F,
    ) -> Result<Vec<u8>>
    where
        F: Fn(KernelUpdateProgress) + Send + Sync,
    {
        ensure_not_cancelled(cancelled)?;
        let mut order = self.transport_order(source)?;
        order.sort_by_key(|index| if *index == preferred { 0 } else { 1 });
        let candidates = candidate_urls_for(source, &asset.browser_download_url);
        let mut failures = Vec::new();
        // 一次只有一条连接在跑：主通道或镜像停滞/失败才试下一个，进度天然单调递增。
        for index in order {
            let transport = &self.transports[index];
            for (url, source_label) in &candidates {
                ensure_not_cancelled(cancelled)?;
                let label = format!("{}·{}", transport.label, source_label);
                match Self::stream_asset(
                    &transport.stream_client,
                    url.clone(),
                    label.clone(),
                    kind,
                    asset.name.clone(),
                    asset.size,
                    cancelled,
                    progress,
                )
                .await
                {
                    Ok(bytes) => return Ok(bytes),
                    Err(error) => {
                        failures.push(format!("{label}: {error}"));
                        crate::startup_log(&format!("[内核下载] {label} 失败: {error}，尝试备用通道..."));
                        progress(KernelUpdateProgress {
                            kind,
                            stage: KernelUpdateStage::Downloading,
                            downloaded: 0,
                            total: asset.size,
                            bytes_per_second: 0.0,
                            message: format!("{label} 失败：{error}，正在切换通道..."),
                        });
                    }
                }
            }
        }
        bail!("下载 {} 失败：{}", asset.name, failures.join("；"))
    }

    /// 单条连接的流式下载。长度不够或传输停滞超过 12 秒就判失败自动换线。
    #[allow(clippy::too_many_arguments)]
    async fn stream_asset(
        client: &Client,
        url: String,
        label: String,
        kind: KernelKind,
        name: String,
        expected: u64,
        cancelled: &AtomicBool,
        progress: &(dyn Fn(KernelUpdateProgress) + Send + Sync),
    ) -> Result<Vec<u8>> {
        const CONNECT_AND_HEADER_TIMEOUT: Duration = Duration::from_secs(12);
        const CHUNK_IDLE_TIMEOUT: Duration = Duration::from_secs(12);

        let response = match tokio::time::timeout(
            CONNECT_AND_HEADER_TIMEOUT,
            client.get(&url).send()
        )
        .await
        {
            Ok(Ok(response)) => match response.error_for_status() {
                Ok(response) => response,
                Err(error) => bail!("HTTP 失败: {error}"),
            },
            Ok(Err(error)) => bail!("{error:#}"),
            Err(_) => bail!("连接或响应头超时 (12s)"),
        };

        let mut response = response;
        let started = std::time::Instant::now();
        let capacity = (expected as usize).min(MAX_ARCHIVE_SIZE as usize);
        let mut bytes: Vec<u8> = Vec::with_capacity(capacity);
        loop {
            if cancelled.load(Ordering::Relaxed) {
                bail!("已取消");
            }
            match tokio::time::timeout(CHUNK_IDLE_TIMEOUT, response.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    bytes.extend_from_slice(&chunk);
                    if bytes.len() as u64 > MAX_ARCHIVE_SIZE {
                        bail!("下载内容超过安全大小限制");
                    }
                    let elapsed = started.elapsed().as_secs_f64().max(0.001);
                    let own = bytes.len() as u64;
                    let percent = if expected > 0 {
                        ((own as f64 / expected as f64) * 100.0).clamp(0.0, 100.0) as u32
                    } else {
                        0
                    };
                    let mb_down = own as f64 / 1024.0 / 1024.0;
                    let mb_total = expected as f64 / 1024.0 / 1024.0;
                    let speed_kbs = (own as f64 / elapsed) / 1024.0;
                    let speed_str = if speed_kbs >= 1024.0 {
                        format!("{:.2} MB/s", speed_kbs / 1024.0)
                    } else {
                        format!("{:.1} KB/s", speed_kbs)
                    };
                    progress(KernelUpdateProgress {
                        kind,
                        stage: KernelUpdateStage::Downloading,
                        downloaded: own,
                        total: expected,
                        bytes_per_second: own as f64 / elapsed,
                        message: format!(
                            "正在下载 {name} {percent}% ({mb_down:.1}MB / {mb_total:.1}MB, {speed_str}) · {label}"
                        ),
                    });
                }
                Ok(Ok(None)) => {
                    if (bytes.len() as u64) < expected {
                        bail!("只下到 {} / {} 字节（连接过早中断）", bytes.len(), expected);
                    }
                    return Ok(bytes);
                }
                Ok(Err(error)) => bail!("下载中断: {error}"),
                Err(_) => bail!("下载停滞（超过 12 秒未收到任何新数据）"),
            }
        }
    }

    /// 这个下载方式允许哪些通道（按 label 判定，"直连"就是不经代理那一条）。
    fn transport_order(&self, source: KernelSource) -> Result<Vec<usize>> {
        let order: Vec<usize> = self
            .transports
            .iter()
            .enumerate()
            .filter(|(_, transport)| source.allows(&transport.label))
            .map(|(index, _)| index)
            .collect();
        if order.is_empty() {
            bail!(
                "「{}」没有可用通道：应用内代理未运行、系统代理也没设置",
                source.label()
            );
        }
        Ok(order)
    }

    async fn fetch_release(&self, kind: KernelKind) -> Result<GithubRelease> {
        Ok(self.fetch_release_with_transport(kind).await?.0)
    }

    /// 查版本按通道**依次**试（实测 `api.github.com` 裸连 3.0s、走代理 0.44s）。这一步跟
    /// 「下载方式」无关：镜像站不代理 `api.github.com`，代理候选又总归更快，所以统一按
    /// 应用内代理 → 系统代理 → 直连 排队，任一时刻仍只有一条连接。返回成功那条的下标，
    /// 让后面的下载优先走同一条通道（只是排序提示，下载仍受所选方式的通道限制）。
    async fn fetch_release_with_transport(
        &self,
        kind: KernelKind,
    ) -> Result<(GithubRelease, usize)> {
        let order: Vec<usize> = (0..self.transports.len()).collect();
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            kind.github_repository()
        );
        let mut failures = Vec::new();
        for index in order {
            let transport = &self.transports[index];
            match fetch_json::<GithubRelease>(&transport.client, &url).await {
                Ok(release) => {
                    let cache_path = self.release_cache_path(kind);
                    if let Some(parent) = cache_path.parent() {
                        let _ = tokio::fs::create_dir_all(parent).await;
                    }
                    if let Ok(bytes) = serde_json::to_vec(&release) {
                        let _ = tokio::fs::write(cache_path, bytes).await;
                    }
                    return Ok((release, index));
                }
                Err(error) => failures.push(format!("{}: {error}", transport.label)),
            }
        }
        let remote_error = anyhow::anyhow!("{}", failures.join("；"));
        // 退到本地缓存：上一次成功查到的 Release 会写在那儿。
        if let Ok(bytes) = tokio::fs::read(self.release_cache_path(kind)).await {
            if let Ok(release) = serde_json::from_slice::<GithubRelease>(&bytes) {
                return Ok((release, 0));
            }
        }
        // 新装的机器上缓存也不存在，而 `api.github.com` 在这边是间歇被掐的（镜像站又
        // 不代理它）。这时用内置兜底版本，别让"预备状态"卡在查不到版本号上。
        fallback_release(kind)
            .map(|release| (release, 0))
            .with_context(|| {
                format!(
                    "读取 {} 官方 Release 失败且没有本地缓存: {remote_error}",
                    kind.display_name()
                )
            })
    }

    fn release_cache_path(&self, kind: KernelKind) -> PathBuf {
        self.root
            .join(".release-cache")
            .join(format!("{}.json", kind.directory_name()))
    }
}

/// 内置兜底 Release：只在"查不到最新 Release + 本地也没有缓存"时用。
///
/// 哈希是**按资产**的，不是按版本号的：同一个 v1.19.32，Windows 的 `.zip` 和 Linux 的
/// `.gz` 是两个完全不同的字节流，套错号就是"下得来但校验必挂"。
/// 两组数字都实测过（2026-10-15）：
/// - Windows：走 gh-proxy 2.5 秒下完 22,492,402 字节，算出的 SHA-256 与 GitHub `digest` 一致；
/// - Linux：走 ghfast 36.6 秒下完 22,846,526 字节，`sha256sum` 与 GitHub `digest` 逐字符一致，
///   解出来 61,993,086 字节的静态 ELF，`chmod +x` 后 `-v` 退出码 0。
/// 代价是版本号会慢慢落后；更新它只改下面这几行常量。
fn fallback_release(kind: KernelKind) -> Option<GithubRelease> {
    if kind != KernelKind::Mihomo {
        return None;
    }
    const MIHOMO_TAG: &str = "v1.19.32";
    let (primary_name, primary_digest, primary_size, compat_name, compat_digest, compat_size) =
        match KernelPlatform::current() {
            KernelPlatform::Windows => (
                format!("mihomo-windows-amd64-{MIHOMO_TAG}.zip"),
                "sha256:1ac84e795b5b915446e20139677fb6cc013a0778e876e8d4c96f994750b4c4a3",
                22_492_402_u64,
                format!("mihomo-windows-amd64-compatible-{MIHOMO_TAG}.zip"),
                "sha256:974a4d7ad69aed27aa2e8f91d61113573c14dadb14562c63e58effabf59816f0",
                22_525_067_u64,
            ),
            KernelPlatform::Linux => (
                format!("mihomo-linux-amd64-{MIHOMO_TAG}.gz"),
                "sha256:8451100836c9eda194331c2babfad490b2faf30cebc1a04b0e76fd8ac2d35d10",
                22_846_526_u64,
                format!("mihomo-linux-amd64-compatible-{MIHOMO_TAG}.gz"),
                "sha256:ba3ce607747a07f948fc35780e108a4a7c7f552a38b9bd4d115f313ebcb89c20",
                22_877_876_u64,
            ),
            KernelPlatform::MacOS => (
                format!("mihomo-darwin-amd64-{MIHOMO_TAG}.gz"),
                "sha256:b7de9fc46af2ca985ecfb302a6362170e04ff2023dee1e82addc54133e669bc9",
                22_276_968_u64,
                format!("mihomo-darwin-amd64-compatible-{MIHOMO_TAG}.gz"),
                "sha256:18b382df77bded2ad0fb3db27db5636cb15b20729d5ba995a357eeb9b46bf507",
                22_302_953_u64,
            ),
        };
    Some(GithubRelease {
        tag_name: MIHOMO_TAG.to_string(),
        assets: vec![
            GithubAsset {
                name: primary_name.clone(),
                browser_download_url: format!(
                    "https://github.com/MetaCubeX/mihomo/releases/download/{MIHOMO_TAG}/{primary_name}"
                ),
                size: primary_size,
                digest: Some(primary_digest.to_string()),
            },
            GithubAsset {
                name: compat_name.clone(),
                browser_download_url: format!(
                    "https://github.com/MetaCubeX/mihomo/releases/download/{MIHOMO_TAG}/{compat_name}"
                ),
                size: compat_size,
                digest: Some(compat_digest.to_string()),
            },
        ],
        fallback: true,
    })
}

/// 暂存内核的文件名：**必须以 `.exe` 结尾、且不带前导点**。
///
/// 上一版叫 `.mihomo.exe.staging` —— 点开头 + 不以 .exe 结尾，杀软很愿意把它当
/// 数据文件处理；2026-10-08 在 VirtualBox 上遇到的正是"下载校验全过、一运行就
/// 0xC0000005 且零输出"，所以先把这个我们自己造成的变量消掉。
fn staging_name(kind: KernelKind) -> String {
    let stem = kind.executable_name().trim_end_matches(".exe");
    // Linux 上没有"杀软把无扩展名文件当数据文件"这回事，硬加 `.exe` 反而会做出一个
    // 名叫 `mihomo.exe` 的 ELF（之后排查"为什么这文件不能执行"时极其误导）。
    if cfg!(windows) {
        format!("{stem}.staging.exe")
    } else {
        format!("{stem}.staging")
    }
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("内核下载已取消");
    }
    Ok(())
}

fn require_sha256(asset: &GithubAsset) -> Result<&str> {
    let digest = asset
        .digest
        .as_deref()
        .context("GitHub 资产缺少 SHA-256 摘要")?;
    digest
        .strip_prefix("sha256:")
        .filter(|value| value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .context("GitHub 资产 SHA-256 摘要格式无效")
}

fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("SHA-256 校验失败");
    }
    Ok(())
}

fn version_matches(output: &str, tag: &str) -> bool {
    let version = tag.trim_start_matches('v');
    output
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.'))
        .any(|part| part.trim_start_matches('v') == version)
}

/// 按容器格式取出内核可执行文件。三种格式都支持，因为三家在 Linux 上发的东西不一样。
/// `target` 是暂存路径；解完统一补可执行位（Linux 上 `fs::write` 不会自己给 `+x`，
/// 少了这一步后面"跑一下拿版本"直接 Permission denied）。
fn extract_executable(
    archive: &[u8],
    format: KernelArchive,
    kind: KernelKind,
    target: &Path,
) -> Result<()> {
    match format {
        KernelArchive::Zip => extract_from_zip(archive, kind, target),
        KernelArchive::Gz => extract_from_gz(archive, kind, target),
        KernelArchive::TarGz => extract_from_tar_gz(archive, kind, target),
    }?;
    mark_executable(target)?;
    Ok(())
}

/// mihomo 的 Linux 资产是 **gzip 压的单个可执行文件**，没有目录：解出来就是它本身。
fn extract_from_gz(archive: &[u8], kind: KernelKind, target: &Path) -> Result<()> {
    let mut decoder = flate2::read::GzDecoder::new(Cursor::new(archive));
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .with_context(|| format!("解开 {} 的 .gz 失败", kind.executable_name()))?;
    if bytes.is_empty() {
        bail!("{} 的 .gz 解出来是空的", kind.executable_name());
    }
    std::fs::write(target, bytes)?;
    Ok(())
}

/// sing-box 的 Linux 资产是 `.tar.gz`：先解 gzip，再在 tar 条目里找那个可执行文件。
fn extract_from_tar_gz(archive: &[u8], kind: KernelKind, target: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(archive));
    let mut tar = tar::Archive::new(decoder);
    let mut found = false;
    for entry in tar.entries().context("打开内核 TAR 失败")? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !kind.matches_archive_executable(file_name) || !entry.header().entry_type().is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        std::fs::write(target, bytes)?;
        found = true;
        break;
    }
    if found {
        Ok(())
    } else {
        bail!("TAR 中未找到 {}", kind.executable_name())
    }
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn extract_from_zip(archive: &[u8], kind: KernelKind, target: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).context("打开内核 ZIP 失败")?;
    let mut executable_found = false;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let is_executable = kind.matches_archive_executable(file_name);
        let is_xray_data = kind == KernelKind::Xray
            && matches!(
                file_name.to_ascii_lowercase().as_str(),
                "geoip.dat" | "geosite.dat"
            );
        if !is_executable && !is_xray_data {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        if is_executable {
            std::fs::write(target, bytes)?;
            executable_found = true;
        } else if let Some(parent) = target.parent() {
            std::fs::write(parent.join(file_name), bytes)?;
        }
    }
    if executable_found {
        Ok(())
    } else {
        bail!("ZIP 中未找到 {}", kind.executable_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn matches_release_tag_in_version_output() {
        assert!(version_matches(
            "Mihomo Meta v1.19.29 windows amd64",
            "v1.19.29"
        ));
        assert!(version_matches("Xray 26.3.27 (Xray)", "v26.3.27"));
        assert!(!version_matches("sing-box version 1.13.13", "v1.13.14"));
    }

    /// 兜底 Release 必须是"照原样下得回来、哈希对得上"的那种：资产名要符合内核
    /// 自己的命名规则，摘要要能过 `require_sha256`（否则下载后校验会立刻失败）。
    #[test]
    fn fallback_release_is_self_consistent() {
        let release = fallback_release(KernelKind::Mihomo).expect("mihomo 有兜底版本");
        assert!(release.fallback);
        assert_eq!(release.assets.len(), 2);
        let asset = &release.assets[0];
        assert_eq!(asset.name, KernelKind::Mihomo.asset_name(&release.tag_name));
        let compat = &release.assets[1];
        assert!(compat.name.contains("compatible"));
        // 大小/容器要跟着平台走：这两个号分别是 Windows 的 .zip 和 Linux 的 .gz，
        // 写死任何一个都会在另一个平台上变成"下得来但校验必挂"。
        let (want_size, want_suffix) = match KernelPlatform::current() {
            KernelPlatform::Windows => (22_492_402, ".zip"),
            KernelPlatform::Linux => (22_846_526, ".gz"),
            KernelPlatform::MacOS => (22_276_968, ".gz"),
        };
        assert_eq!(asset.size, want_size, "兜底大小要对得上当前平台的资产");
        assert!(
            asset.name.ends_with(want_suffix),
            "当前平台要的是 {want_suffix}，实际资产名 {}",
            asset.name
        );
        assert_eq!(
            asset.browser_download_url,
            format!(
                "https://github.com/MetaCubeX/mihomo/releases/download/{}/{}",
                release.tag_name, asset.name
            )
        );
        let digest = require_sha256(asset).expect("摘要格式要能过校验");
        assert_eq!(digest.len(), 64, "SHA-256 十六进制是 64 位");
        assert!(
            fallback_release(KernelKind::Xray).is_none(),
            "只兜底 mihomo：别的内核没实测过哈希，不许编一个版本号"
        );
    }

    #[test]
    fn verifies_sha256_digest() {
        let expected = format!("{:x}", Sha256::digest(b"kernel"));
        assert!(verify_sha256(b"kernel", &expected).is_ok());
        assert!(verify_sha256(b"changed", &expected).is_err());
    }

    #[test]
    fn cancellation_stops_before_installation() {
        let cancelled = AtomicBool::new(true);
        assert!(ensure_not_cancelled(&cancelled)
            .unwrap_err()
            .to_string()
            .contains("已取消"));
    }

    /// 2026-10-08 VirtualBox 上撞到的那类失败：内核下得来、哈希也对，但在那台机器上
    /// 跑不起来。这条钉住两件要紧事 —— 报错要说清是哪一步，而且**坏内核不许进正式路径**
    /// （进了就是"看起来装好了，启动代理才炸"）。
    /// 暂存名的形状是这次排障的关键，钉住它：以 .exe 结尾、不前导点、不撞正式名。
    #[test]
    fn staging_files_keep_an_executable_name() {
        for kind in [KernelKind::Mihomo, KernelKind::Xray, KernelKind::SingBox] {
            let name = staging_name(kind);
            // `.exe` 结尾是 Windows 那侧针对杀软的取证结论，Linux 上反过来是负担
            // （一个叫 `mihomo.exe` 的 ELF 只会误导排障），所以按平台分别要求。
            if cfg!(windows) {
                assert!(name.ends_with(".exe"), "{name} 在 Windows 必须以 .exe 结尾");
            } else {
                assert!(!name.ends_with(".exe"), "{name} 在 Linux 不该带 .exe");
            }
            assert!(!name.starts_with('.'), "{name} 不能是隐藏文件");
            assert_ne!(name, kind.executable_name(), "不能和正式内核同名");
        }
    }

    /// 三家的资产名与容器格式都是拿 GitHub Release API 实测过写死的（2026-10-15）。
    /// 这条测试的意义：以后谁把 Linux 那套偷偷写成 `.zip`，下载会 404，这里先红。
    #[test]
    fn linux_assets_use_each_vendors_own_container() {
        let mihomo = KernelKind::Mihomo.asset_name_for(KernelPlatform::Linux, "v1.19.32");
        assert_eq!(mihomo, "mihomo-linux-amd64-v1.19.32.gz");
        assert_eq!(
            KernelArchive::from_asset_name(&mihomo),
            Some(KernelArchive::Gz),
            "mihomo 的 Linux 资产是 gzip 裸文件，不是 zip"
        );

        let sing_box = KernelKind::SingBox.asset_name_for(KernelPlatform::Linux, "v1.14.3");
        assert_eq!(sing_box, "sing-box-1.14.3-linux-amd64.tar.gz");
        assert_eq!(
            KernelArchive::from_asset_name(&sing_box),
            Some(KernelArchive::TarGz)
        );

        let xray = KernelKind::Xray.asset_name_for(KernelPlatform::Linux, "v26.3.27");
        assert_eq!(xray, "Xray-linux-64.zip");
        assert_eq!(
            KernelArchive::from_asset_name(&xray),
            Some(KernelArchive::Zip)
        );

        // 反向对照：Windows 那三行必须一个都没被 Linux 的形状污染。
        assert_eq!(
            KernelKind::Mihomo.asset_name_for(KernelPlatform::Windows, "v1.19.32"),
            "mihomo-windows-amd64-v1.19.32.zip"
        );
        assert_eq!(
            KernelKind::Xray.asset_name_for(KernelPlatform::Windows, "v26.3.27"),
            "Xray-windows-64.zip"
        );
    }

    /// 条目名判据不许只认 `.exe`：Linux 容器里的可执行文件就叫 `mihomo` / `xray` / `sing-box`。
    #[test]
    fn archive_entries_match_without_an_exe_suffix() {
        assert!(KernelKind::Mihomo.matches_archive_executable("mihomo"));
        assert!(KernelKind::Xray.matches_archive_executable("xray"));
        assert!(KernelKind::SingBox.matches_archive_executable("sing-box"));
        assert!(KernelKind::Mihomo.matches_archive_executable("mihomo-windows-amd64.exe"));
        assert!(!KernelKind::Xray.matches_archive_executable("geoip.dat"));
        assert!(!KernelKind::Xray.matches_archive_executable("wv2ray"));
    }

    #[tokio::test]
    async fn a_kernel_that_cannot_run_is_never_installed() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("mihomo.exe", options).unwrap();
        // 不是 PE 文件：Windows 会拒绝执行，等价于"下得来但跑不了"。
        archive.write_all(b"not a windows executable").unwrap();
        let bytes = archive.finish().unwrap().into_inner();

        let root =
            std::env::temp_dir().join(format!("vpn-parser-prepare-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let updater = KernelUpdater::new(root.clone(), None).unwrap();
        let cancelled = AtomicBool::new(false);

        let error = updater
            .install_bytes(
                KernelKind::Mihomo,
                bytes,
                Some("v1.19.32"),
                KernelArchive::Zip,
                &cancelled,
                &|_| {},
            )
            .await
            .expect_err("跑不起来的内核必须报错");
        assert!(
            error.to_string().contains("执行")
                || error.to_string().contains("未返回版本")
                || error.to_string().contains("版本不匹配"),
            "错误要指出是探测这一步：{error}"
        );
        assert!(
            !root.join("mihomo").join("mihomo.exe").exists(),
            "正式路径必须还是干净的"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn xray_archive_extracts_executable_and_geo_data() {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("xray.exe", b"exe".as_slice()),
            ("geoip.dat", b"ip".as_slice()),
            ("geosite.dat", b"site".as_slice()),
        ] {
            archive.start_file(name, options).unwrap();
            archive.write_all(body).unwrap();
        }
        let bytes = archive.finish().unwrap().into_inner();
        let directory =
            std::env::temp_dir().join(format!("vpn-parser-xray-assets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join(staging_name(KernelKind::Xray));

        extract_executable(&bytes, KernelArchive::Zip, KernelKind::Xray, &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"exe");
        assert_eq!(std::fs::read(directory.join("geoip.dat")).unwrap(), b"ip");
        assert_eq!(
            std::fs::read(directory.join("geosite.dat")).unwrap(),
            b"site"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    /// mihomo 的 Linux 资产是 gzip 压的**裸可执行文件**（没有目录、没有条目名），
    /// 所以这条路径连"找哪个条目"都没有 —— 解出来就是它。真实字节我手动验过
    /// （22,846,526 → 61,993,086 的 ELF，`-v` 退出码 0），这里钉住的是分发逻辑。
    #[test]
    fn gz_archive_yields_the_single_executable() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"#!/elf-ish mihomo payload").unwrap();
        let gz = encoder.finish().unwrap();

        let dir = std::env::temp_dir().join(format!("vpn-gz-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join(staging_name(KernelKind::Mihomo));
        extract_executable(&gz, KernelArchive::Gz, KernelKind::Mihomo, &target).unwrap();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"#!/elf-ish mihomo payload"
        );

        // 反向对照：同一份字节按 ZIP 走必须失败，不许"随便挑一种解法都能过"。
        let wrong = dir.join("wrong");
        assert!(
            extract_executable(&gz, KernelArchive::Zip, KernelKind::Mihomo, &wrong).is_err(),
            "gzip 字节喂给 ZIP 分支必须报错，不能静默产出坏内核"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// sing-box 的 Linux 资产是 `.tar.gz`：里面还有别的东西（目录、许可证），
    /// 所以要按条目名挑，并且不许把目录当可执行文件收下来。
    #[test]
    fn tar_gz_archive_picks_the_executable_entry() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            let payload = b"sing-box binary";
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o755);
            header.set_size(payload.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, "sing-box", &payload[..])
                .unwrap();
            // 真的 sing-box tar.gz 里还有 LICENSES/ 这类目录条目：必须被跳过而不是当内核。
            builder.append_dir("LICENSES", ".").unwrap();
            builder.finish().unwrap();
        }
        let bytes = gz.finish().unwrap();

        let dir = std::env::temp_dir().join(format!("vpn-targz-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join(staging_name(KernelKind::SingBox));
        extract_executable(&bytes, KernelArchive::TarGz, KernelKind::SingBox, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"sing-box binary");

        // 反向对照：一个只含目录、没有可执行条目的 tar.gz 必须报错。
        let mut empty_gz = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut builder = tar::Builder::new(&mut empty_gz);
            builder.append_dir("only-dir", ".").unwrap();
            builder.finish().unwrap();
        }
        let empty = empty_gz.finish().unwrap();
        let wrong = dir.join("wrong");
        assert!(
            extract_executable(&empty, KernelArchive::TarGz, KernelKind::SingBox, &wrong).is_err(),
            "没有可执行条目时必须失败，不许产出空内核"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn candidate_chain_always_ends_with_direct_and_no_duplicates() {
        let chain = proxy_candidates(Some("http://127.0.0.1:9980"));
        assert_eq!(
            chain.first().unwrap().as_deref(),
            Some("http://127.0.0.1:9980")
        );
        assert!(chain.last().unwrap().is_none(), "最后一档必须是直连兜底");
        let mut deduped = chain.clone();
        deduped.dedup();
        assert_eq!(
            deduped, chain,
            "候选不能重复（系统代理和应用内代理常常是同一个）"
        );
        assert!(proxy_candidates(None).last().unwrap().is_none());
        // 空白代理地址不能当成一条通道
        assert_eq!(
            proxy_candidates(Some("   ")).first().unwrap().as_deref(),
            proxy_candidates(None).first().unwrap().as_deref()
        );
    }

    #[test]
    fn each_download_method_resolves_to_exactly_one_url() {
        let direct =
            "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.29/mihomo-windows-amd64-v3-v1.19.29.zip";
        assert_eq!(
            asset_url_for(KernelSource::GhProxy, direct),
            format!("https://gh-proxy.com/{direct}")
        );
        assert_eq!(
            asset_url_for(KernelSource::GhFast, direct),
            format!("https://ghfast.top/{direct}")
        );
        for source in [
            KernelSource::Official,
            KernelSource::Proxy,
            KernelSource::Local,
        ] {
            assert_eq!(
                asset_url_for(source, direct),
                direct,
                "{source:?} 必须只连官方原链"
            );
        }
    }

    #[test]
    fn candidate_urls_contain_primary_and_fallback_mirrors() {
        let direct = "https://github.com/MetaCubeX/mihomo/releases/download/v1.19.29/mihomo.zip";
        let proxy_candidates = candidate_urls_for(KernelSource::GhProxy, direct);
        assert_eq!(proxy_candidates.len(), 2);
        assert!(proxy_candidates[0].0.contains("gh-proxy.com"));
        assert!(proxy_candidates[1].0.contains("ghfast.top"));

        let fast_candidates = candidate_urls_for(KernelSource::GhFast, direct);
        assert_eq!(fast_candidates.len(), 2);
        assert!(fast_candidates[0].0.contains("ghfast.top"));
        assert!(fast_candidates[1].0.contains("gh-proxy.com"));
    }

    #[test]
    fn source_ids_round_trip_and_legacy_values_fall_back() {
        for option in SOURCE_OPTIONS {
            assert_eq!(source_from_id(option.id).id(), option.id);
        }
        // 库里可能还留着旧值 "race"（三家并发抢跑那一版）：落到现在的默认 ghfast，
        // 既不是"什么都不下"，也不会让弹框因为对不上 id 而画不出选中态。
        assert_eq!(source_from_id("race"), KernelSource::GhFast);
        assert_eq!(source_from_id("没有这个"), KernelSource::GhFast);
        assert_eq!(source_from_id("").id(), "ghfast");
        // 默认值必须和弹框里排第一的那一项一致，否则"没改过设置"的人看到的选中态是错的。
        assert_eq!(
            crate::models::AppSettings::default().kernel_source,
            SOURCE_OPTIONS[0].id
        );
    }

    #[tokio::test]
    async fn transport_order_gates_proxy_versus_direct() {
        let dir = std::env::temp_dir().join(format!("vpn-parser-source-{}", std::process::id()));
        // 只有一个代理时：官方直连方式应该拿不到通道（而不是悄悄退回直连）
        let updater = KernelUpdater::new(dir, Some("http://127.0.0.1:9")).unwrap();
        let labels: Vec<&str> = updater
            .transports
            .iter()
            .map(|t| t.label.as_str())
            .collect();
        assert!(
            labels.iter().any(|l| l.starts_with("应用内代理")),
            "{labels:?}"
        );
        assert!(updater.transport_order(KernelSource::Proxy).is_ok());
        // 官方直连和两个镜像方式都只允许不经代理那一条（镜像站本身就在国内）
        for source in [
            KernelSource::Official,
            KernelSource::GhProxy,
            KernelSource::GhFast,
        ] {
            let only = updater.transport_order(source).unwrap();
            assert!(
                only.iter().all(|i| updater.transports[*i].label == "直连"),
                "{source:?} 拿到了代理通道：{labels:?}"
            );
        }
        // 「走代理」不许退回直连
        let proxied = updater.transport_order(KernelSource::Proxy).unwrap();
        assert!(
            proxied
                .iter()
                .all(|i| updater.transports[*i].label != "直连"),
            "{labels:?}"
        );
    }

    /// 真实网络手测：按选定的某一种方式把 mihomo 的 release 附件下一遍，打印耗时。
    /// `cargo test -- --ignored real_network_single_method --nocapture`
    #[tokio::test]
    #[ignore = "真实网络手测，要能连上 GitHub/镜像"]
    async fn real_network_single_method_reports_speed() {
        let dir = std::env::temp_dir().join(format!("vpn-parser-asset-one-{}", std::process::id()));
        let updater = KernelUpdater::new(dir, None).unwrap();
        let kind = KernelKind::Mihomo;
        let source = KernelSource::GhFast;
        let (release, transport) = updater.fetch_release_with_transport(kind).await.unwrap();
        let name = kind.asset_name(&release.tag_name);
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .unwrap()
            .clone();
        let cancelled = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let bytes = updater
            .download_asset(kind, &asset, source, transport, &cancelled, &|_| {})
            .await
            .unwrap();
        let elapsed = started.elapsed();
        println!(
            "用「{}」下载 {} 共 {} 字节，用时 {:.1}s（{:.2} MB/s），通道 #{transport} = {}",
            source.label(),
            asset.name,
            bytes.len(),
            elapsed.as_secs_f64(),
            bytes.len() as f64 / elapsed.as_secs_f64() / 1_048_576.0,
            updater.transports[transport].label,
        );
        assert_eq!(bytes.len() as u64, asset.size);
    }
}

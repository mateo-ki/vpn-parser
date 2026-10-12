//! 分流数据库（`geosite.dat` / `geoip.dat`）的校验与准备。
//!
//! 为什么需要这个模块：mihomo 自己的 `geo-auto-update` 只会拿配置里的 `geox-url` **直连**
//! GitHub 下载。实测这台机直连 GitHub 只有 0.116 MB/s（镜像 1.44–8.1 MB/s），11MB 的库在
//! 启动时间窗里下不完就被掐断，留下一个 130KB 的半截文件 → 内核报
//! `list cn not found in GeoSite.dat` → 起不来 → 表现成"切换节点失败"。而"要不要更新库"
//! 又取决于内核能不能起来，是个死锁。所以库的下载改由这里负责：先写临时文件、校验大小，
//! 再原子替换，内核侧关掉自动更新。
use anyhow::{anyhow, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 真库是 11MB / 16MB；小于这个数一定是半截货（历史上那次截断是 130,209 字节）。
pub const MIN_GEO_BYTES: u64 = 1_000_000;

pub const GEO_FILES: [&str; 2] = ["geosite.dat", "geoip.dat"];

/// 这些镜像域名必须在内核规则里钉死直连。
///
/// 内核下载 geo 库时走的是**它自己的规则引擎**（日志里是 `[TCP] mihomo --> gh-proxy.com`），
/// 所以这些请求会被最后的 `MATCH,PROXY` 丢到代理节点上 —— 实测每次都是 10 秒
/// `net/http: TLS handshake timeout`，而同一 URL 从本机直连只要 1.4 秒。
pub const MIRROR_DIRECT_HOSTS: [&str; 2] = ["gh-proxy.com", "ghfast.top"];

/// 下载候选，按实测速度排序：镜像在前，官方直连兜底。
const UPSTREAMS: [&str; 3] = [
    "https://gh-proxy.com/https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/",
    "https://ghfast.top/https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/",
    "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/",
];

/// mihomo 的 `-d` 工作目录，geo 文件就放这里。
pub fn mihomo_dir() -> PathBuf {
    crate::kernel::managed_kernel_root().join("mihomo")
}

/// 每个库的"可用大小"（不足 [`MIN_GEO_BYTES`] 视为没有）。
pub fn geo_state(dir: &Path) -> Vec<(&'static str, Option<u64>)> {
    GEO_FILES
        .iter()
        .copied()
        .map(|name| {
            let usable = std::fs::metadata(dir.join(name))
                .ok()
                .map(|meta| meta.len())
                .filter(|len| *len >= MIN_GEO_BYTES);
            (name, usable)
        })
        .collect()
}

pub fn geo_ready(dir: &Path) -> bool {
    geo_state(dir).iter().all(|(_, size)| size.is_some())
}

/// 校验响应体：够大、且不是镜像挂了返回的 HTML。
fn validate_geo_body(bytes: &[u8]) -> Result<u64> {
    if (bytes.len() as u64) < MIN_GEO_BYTES {
        return Err(anyhow!("只有 {} 字节，判为半截文件", bytes.len()));
    }
    if bytes.starts_with(b"<") {
        return Err(anyhow!("拿回来的是 HTML，这个源不可用"));
    }
    // geo 数据是 protobuf：第一个字节是 field1+LD 的 tag (0x0a)。
    if bytes[0] != 0x0a {
        return Err(anyhow!("文件头不是 geo 数据（首字节 {:#04x}）", bytes[0]));
    }
    Ok(bytes.len() as u64)
}

/// 缺什么补什么；返回一句给人看的报告。`force = true` 时两个库全部重下。
pub async fn provision(dir: &Path, proxy_url: Option<&str>, force: bool) -> Result<String> {
    std::fs::create_dir_all(dir)?;
    let mut builder = Client::builder()
        .user_agent("vpn-parser geo-updater")
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(180));
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
    }
    let client = builder.build()?;

    let mut report = Vec::new();
    for name in GEO_FILES {
        let target = dir.join(name);
        let existing = std::fs::metadata(&target).ok().map(|meta| meta.len());
        if !force && existing.is_some_and(|len| len >= MIN_GEO_BYTES) {
            report.push(format!("{name} 已就绪 {}", human_mb(existing.unwrap_or(0))));
            continue;
        }
        let mut failures = Vec::new();
        let mut done = false;
        for upstream in UPSTREAMS {
            let url = format!("{upstream}{name}");
            match download_into(&client, &url, &target).await {
                Ok(bytes) => {
                    report.push(format!(
                        "{name} 更新成功 {}（源 {}）",
                        human_mb(bytes),
                        host_of(&url)
                    ));
                    done = true;
                    break;
                }
                Err(error) => failures.push(format!("{}: {error}", host_of(&url))),
            }
        }
        if !done {
            return Err(anyhow!(
                "分流数据库 {name} 三个源都失败（{}），代理没开时请先启动代理再更新",
                failures.join("；")
            ));
        }
    }
    Ok(report.join("，"))
}

async fn download_into(client: &Client, url: &str, target: &Path) -> Result<u64> {
    let response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => return Err(anyhow!("{error:#}")),
    };
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("HTTP {status}"));
    }
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => return Err(anyhow!("读取响应失败: {error:#}")),
    };
    let bytes = validate_geo_body(&body)?;
    // 先落到 .part 再替换：内核永远不会看到写了一半的库。
    let part = target.with_extension("dat.part");
    std::fs::write(&part, &body)?;
    let _ = std::fs::remove_file(target);
    std::fs::rename(&part, target)?;
    Ok(bytes)
}

fn human_mb(bytes: u64) -> String {
    format!("{:.1}MB", bytes as f64 / 1_048_576.0)
}

fn host_of(url: &str) -> String {
    url.split('/')
        .nth(2)
        .unwrap_or("?")
        .trim_start_matches("gh-proxy.com@")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncated_or_html_bodies_are_rejected() {
        // 这次事故的原样：130,209 字节的半截库必须判坏。
        assert!(validate_geo_body(&vec![0x0a; 130_209]).is_err());
        assert!(validate_geo_body(b"<html>rate limited</html>").is_err());
        let mut junk = vec![b'z'; 2_000_000];
        junk[0] = b'Z';
        assert!(validate_geo_body(&junk).is_err());
        let mut good = vec![0x0a; 1_500_000];
        good[0] = 0x0a;
        assert_eq!(validate_geo_body(&good).unwrap(), 1_500_000);
    }

    #[test]
    fn every_mirror_upstream_host_is_pinned_direct() {
        // 内核会用自己的规则引擎决定这条下载走哪儿；漏钉一家就等于把那家交回 MATCH→代理节点。
        for upstream in UPSTREAMS {
            let host = upstream
                .strip_prefix("https://")
                .unwrap_or(upstream)
                .split('/')
                .next()
                .unwrap_or_default();
            if host == "github.com" {
                continue; // 官方原链本来就该按用户规则走
            }
            assert!(
                MIRROR_DIRECT_HOSTS.contains(&host),
                "镜像 {host} 没有登记到 MIRROR_DIRECT_HOSTS"
            );
        }
    }

    #[test]
    fn readiness_uses_the_size_floor() {
        let dir = std::env::temp_dir().join(format!(
            "vpn-parser-geo-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!geo_ready(&dir));
        std::fs::write(dir.join("geosite.dat"), vec![0x0a; 130_209]).unwrap();
        std::fs::write(dir.join("geoip.dat"), vec![0x0a; 2_000_000]).unwrap();
        // 半截的那个仍然算没就绪 —— 这正是"能起内核"和"不能起内核"的分界。
        assert!(!geo_ready(&dir));
        std::fs::write(dir.join("geosite.dat"), vec![0x0a; 11_000_000]).unwrap();
        assert!(geo_ready(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

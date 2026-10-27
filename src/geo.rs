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

/// geosite.dat 完整库约 11MB，低于 8MB 一定是下载中断的半截文件。
pub const MIN_GEOSITE_BYTES: u64 = 8_000_000;
/// geoip.dat 完整库约 16MB，低于 8MB 一定是下载中断的半截文件。
pub const MIN_GEOIP_BYTES: u64 = 8_000_000;

pub fn min_bytes_for_file(name: &str) -> u64 {
    if name.to_ascii_lowercase().contains("geosite") {
        MIN_GEOSITE_BYTES
    } else {
        MIN_GEOIP_BYTES
    }
}

pub const GEO_FILES: [&str; 2] = ["geosite.dat", "geoip.dat"];

/// 这些镜像域名必须在内核规则里钉死直连。
///
/// 内核下载 geo 库时走的是**它自己的规则引擎**（日志里是 `[TCP] mihomo --> gh-proxy.com`），
/// 所以这些请求会被最后的 `MATCH,PROXY` 丢到代理节点上 —— 实测每次都是 10 秒
/// `net/http: TLS handshake timeout`，而同一 URL 从本机直连只要 1.4 秒。
pub const MIRROR_DIRECT_HOSTS: [&str; 4] = [
    "fastly.jsdelivr.net",
    "cdn.jsdelivr.net",
    "gh-proxy.com",
    "ghfast.top",
];

/// 下载候选，按实测速度排序：Fastly 全球极速 CDN 优先（实测 8MB/s，2 秒下完），镜像与官方兜底。
const UPSTREAMS: [&str; 4] = [
    "https://fastly.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/",
    "https://cdn.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/",
    "https://gh-proxy.com/https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/",
    "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download/",
];

/// mihomo 的 `-d` 工作目录，geo 文件就放这里。
pub fn mihomo_dir() -> PathBuf {
    crate::kernel::managed_kernel_root().join("mihomo")
}

/// 清理已损坏的 Geo 规则库文件（如被截断、解析报错的 GeoSite.dat / GeoIP.dat）
pub fn purge_corrupt_geo_files(dir: &Path) {
    for name in &[
        "geosite.dat",
        "GeoSite.dat",
        "geoip.dat",
        "GeoIP.dat",
        "geosite.dat.part",
        "geoip.dat.part",
        "GeoSite.dat.download",
        "GeoIP.dat.download",
    ] {
        let p = dir.join(name);
        if p.exists() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 每个库的"可用大小"（不足安全阈值视为没有）。
pub fn geo_state(dir: &Path) -> Vec<(&'static str, Option<u64>)> {
    GEO_FILES
        .iter()
        .copied()
        .map(|name| {
            let min_len = min_bytes_for_file(name);
            let candidate_paths = [
                dir.join(name),
                dir.join(if name == "geosite.dat" { "GeoSite.dat" } else { "GeoIP.dat" }),
            ];
            let usable = candidate_paths.iter().find_map(|p| {
                std::fs::metadata(p)
                    .ok()
                    .map(|meta| meta.len())
                    .filter(|len| *len >= min_len)
            });
            (name, usable)
        })
        .collect()
}

pub fn geo_ready(dir: &Path) -> bool {
    geo_state(dir).iter().all(|(_, size)| size.is_some())
}

/// meow 的 GEOIP 用 MaxMind MMDB（meta-rules-dat 的 country.mmdb），完整库约 7.6MB。
pub const MIN_MMDB_BYTES: u64 = 4_000_000;

/// meow 内核 home 目录的 geo 准备：
/// 1. 复用 mihomo 目录的 geosite.dat / geoip.dat（meow 的发现路径接受 protobuf dat，实测加载成功）；
/// 2. Country.mmdb 缺失时从镜像下载（meow 自己也会下，但走 GitHub，境内大概率卡住启动）。
/// 文件已就位（大小达标）时跳过，幂等。
pub async fn ensure_meow_home_geo(home: &Path, proxy_url: Option<&str>) -> Result<String> {
    std::fs::create_dir_all(home)?;
    let mut report = Vec::new();

    // 1. mihomo 目录的 dat 先备好
    let mihomo = mihomo_dir();
    if !geo_ready(&mihomo) {
        provision(&mihomo, proxy_url, false).await?;
    }

    // 2. 拷进 meow home（meow 的发现路径之一是 <home>/geosite.dat）
    for name in GEO_FILES {
        let Some(size) = geo_state(&mihomo)
            .into_iter()
            .find(|(n, _)| *n == name)
            .and_then(|(_, s)| s)
        else {
            continue;
        };
        let dst = home.join(name);
        let needs_copy = std::fs::metadata(&dst)
            .map(|m| m.len() != size)
            .unwrap_or(true);
        if needs_copy {
            let candidates = [
                mihomo.join(name),
                mihomo.join(if name == "geosite.dat" { "GeoSite.dat" } else { "GeoIP.dat" }),
            ];
            if let Some(src) = candidates.iter().find(|p| p.exists()) {
                std::fs::copy(src, &dst)?;
                report.push(format!("{name} 已复制到 meow 目录"));
            }
        }
    }

    // 3. Country.mmdb —— 25 秒硬上限：meow 启动时若发现缺库会自己拉取并阻塞
    // 监听端口，而它默认走节点（节点劣化 = 无限卡死"正在启动"）。这里限时
    // 抢先备好；失败也放行（meow 会用配置里的镜像 URL 自愈，失败则快速报错）。
    let mmdb = home.join("Country.mmdb");
    let mmdb_ok = std::fs::metadata(&mmdb)
        .map(|m| m.len() >= MIN_MMDB_BYTES)
        .unwrap_or(false);
    if !mmdb_ok {
        const MMDB_UPSTREAMS: [&str; 4] = [
            "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/",
            "https://cdn.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/",
            "https://gh-proxy.com/https://github.com/MetaCubeX/meta-rules-dat/releases/latest/download/",
            "https://github.com/MetaCubeX/meta-rules-dat/releases/latest/download/",
        ];
        let mmdb_task = async {
            let mut builder = Client::builder()
                .user_agent("vpn-parser geo-updater")
                .connect_timeout(Duration::from_secs(6))
                .timeout(Duration::from_secs(20));
            if let Some(proxy_url) = proxy_url {
                builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
            }
            let client = builder.build()?;
            let mut failures = Vec::new();
            for upstream in MMDB_UPSTREAMS {
                let url = format!("{upstream}Country.mmdb");
                match download_into(&client, &url, &mmdb, "Country.mmdb").await {
                    Ok(bytes) => return Ok(bytes),
                    Err(error) => failures.push(format!("{}: {error}", host_of(&url))),
                }
            }
            Err(anyhow!("{}", failures.join("；")))
        };
        match tokio::time::timeout(Duration::from_secs(25), mmdb_task).await {
            Ok(Ok(bytes)) => {
                report.push(format!("Country.mmdb 就绪 {}", human_mb(bytes)));
            }
            Ok(Err(error)) => {
                return Err(anyhow!("Meow 的 Country.mmdb 下载失败：{error}"));
            }
            Err(_) => {
                return Err(anyhow!("Meow 的 Country.mmdb 下载超时（25s）"));
            }
        }
    } else {
        report.push("Country.mmdb 已就绪".into());
    }

    Ok(report.join("，"))
}

/// vp_core 内核目录的 geo 准备：
/// 纯粹维护在 vpcore 自身目录，不跨内核目录复用。
/// 检查自身目录的 geosite.dat 与 geoip 库（geoip.dat / Country.mmdb / geoip.metadb）；
/// 若自身缺少则直接从镜像下载到 vpcore 自身目录。文件已就位（大小达标）时跳过，幂等。
pub async fn ensure_vpcore_geo(home: &Path, proxy_url: Option<&str>) -> Result<String> {
    std::fs::create_dir_all(home)?;

    let has_geosite = (home.join("geosite.dat").exists()
        && std::fs::metadata(home.join("geosite.dat"))
            .map(|m| m.len() >= MIN_GEOSITE_BYTES)
            .unwrap_or(false))
        || (home.join("GeoSite.dat").exists()
            && std::fs::metadata(home.join("GeoSite.dat"))
                .map(|m| m.len() >= MIN_GEOSITE_BYTES)
                .unwrap_or(false));

    let has_geoip = (home.join("geoip.dat").exists()
        && std::fs::metadata(home.join("geoip.dat"))
            .map(|m| m.len() >= MIN_GEOIP_BYTES)
            .unwrap_or(false))
        || (home.join("GeoIP.dat").exists()
            && std::fs::metadata(home.join("GeoIP.dat"))
                .map(|m| m.len() >= MIN_GEOIP_BYTES)
                .unwrap_or(false))
        || (home.join("Country.mmdb").exists()
            && std::fs::metadata(home.join("Country.mmdb"))
                .map(|m| m.len() >= MIN_MMDB_BYTES)
                .unwrap_or(false))
        || home.join("geoip.metadb").exists();

    let mut needed = Vec::new();
    if !has_geosite {
        needed.push("geosite.dat");
    }
    if !has_geoip {
        needed.push("geoip.dat");
    }

    if needed.is_empty() {
        return Ok("VP-Core 自有分流数据库已就绪".into());
    }

    // 自身缺少时直接下载所需库到 vpcore 自身目录
    provision_files(home, &needed, proxy_url, false).await
}

/// 将 vpcore 所需的分流数据库硬链接（或复制）到目标临时工作目录（供 vpcore run -c 启动时直接相对加载）。
/// 纯粹从 vpcore 自身目录读取，不跨内核目录复用。
pub fn populate_vpcore_work_dir(vpcore_home: &Path, work_dir_root: &Path) {
    let files = [
        "geosite.dat",
        "GeoSite.dat",
        "geoip.dat",
        "GeoIP.dat",
        "Country.mmdb",
        "geoip.metadb",
        "wintun.dll",
    ];
    for name in files {
        let src = vpcore_home.join(name);
        if src.exists() {
            let dst = work_dir_root.join(name);
            if !dst.exists() {
                // 优先硬链接（0 耗时 0 空间），跨磁盘失败则回退为拷贝
                if std::fs::hard_link(&src, &dst).is_err() {
                    let _ = std::fs::copy(&src, &dst);
                }
            }
        }
    }
}

/// 校验响应体：够大、且不是镜像挂了返回的 HTML。
fn validate_geo_body(name: &str, bytes: &[u8]) -> Result<u64> {
    let min_len = min_bytes_for_file(name);
    if (bytes.len() as u64) < min_len {
        return Err(anyhow!(
            "只有 {} 字节（低于安全阈值 {}MB），判为半截文件",
            bytes.len(),
            min_len / 1_000_000
        ));
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
    provision_files(dir, &GEO_FILES, proxy_url, force).await
}

/// 按需下载指定列表的 geo 库文件到目标目录；`force = true` 时忽略已有大小强制重下。
pub async fn provision_files(
    dir: &Path,
    names: &[&str],
    proxy_url: Option<&str>,
    force: bool,
) -> Result<String> {
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
    for &name in names {
        let target = dir.join(name);
        let min_len = min_bytes_for_file(name);
        let existing = std::fs::metadata(&target).ok().map(|meta| meta.len());
        if !force && existing.is_some_and(|len| len >= min_len) {
            report.push(format!("{name} 已就绪 {}", human_mb(existing.unwrap_or(0))));
            continue;
        }
        let mut failures = Vec::new();
        let mut done = false;
        for upstream in UPSTREAMS {
            let url = format!("{upstream}{name}");
            match download_into(&client, &url, &target, name).await {
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

async fn download_into(client: &Client, url: &str, target: &Path, name: &str) -> Result<u64> {
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
    let bytes = validate_geo_body(name, &body)?;
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

/// 订阅惯用类别码 → dat 库实际类别。
///
/// `LAN` 是各家订阅的常见写法，但 Loyalsoldier 的 geosite/geoip 库里
/// 都没有这个类别（对应类别叫 `private`），直传会让内核拒启
/// （shoes：`DAT category "LAN" not found`）。映射后语义不变：局域网/私有地址直连。
pub fn normalize_geo_code(code: &str) -> &str {
    if code.eq_ignore_ascii_case("lan") {
        "private"
    } else {
        code
    }
}

/// dat 文件（protobuf）里是否存在指定类别码（大小写不敏感，与内核判定一致）。
///
/// 为什么需要：内核对"类别不存在"是**致命错误** —— shoes 会拒绝整个配置
/// （`DAT category "LAN" not found`，代理起不来），而订阅里常见的
/// `GEOIP,LAN,DIRECT` 在 Loyalsoldier 的库里根本没有这个类别（对应的是
/// `private`）。编译配置前先查一遍，不存在的类别码对应规则直接丢弃：
/// 该规则本来也永远匹配不上，丢掉比让内核拒启好。
///
/// 文件读不出 / 结构解析不了时返回 true —— 解析是尽力而为，裁决权交还内核，
/// 让它报出真实的错误。
pub fn dat_has_category(path: &Path, code: &str) -> bool {
    let Ok(data) = std::fs::read(path) else {
        return true;
    };
    let code = code.as_bytes();
    let mut pos = 0usize;
    while pos < data.len() {
        // 顶层是重复的 Category 消息（field 1, length-delimited）；
        // 出现别的 tag 说明结构对不上，交给内核裁决。
        if data[pos] != 0x0a {
            return true;
        }
        pos += 1;
        let mut length = 0usize;
        let mut shift = 0u32;
        loop {
            let Some(&b) = data.get(pos) else { return true };
            pos += 1;
            length |= ((b & 0x7f) as usize) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return true;
            }
        }
        let Some(chunk) = data.get(pos..pos + length) else {
            return true;
        };
        pos += length;
        // Category 内部 field 1（0x0a）是类别码字符串
        if chunk.first() == Some(&0x0a) {
            if let Some(&code_len) = chunk.get(1) {
                if let Some(found) = chunk.get(2..2 + code_len as usize) {
                    if found.eq_ignore_ascii_case(code) {
                        return true;
                    }
                }
            }
        }
    }
    false
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

    /// 手搓一个最小 dat：顶层重复 Category(field 1)，内部 field 1 是类别码字符串。
    fn build_dat(codes: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for code in codes {
            let inner_len = 2 + code.len(); // 0x0a + code_len 字节 + code
            out.push(0x0a);
            out.push(inner_len as u8);
            out.push(0x0a);
            out.push(code.len() as u8);
            out.extend_from_slice(code.as_bytes());
        }
        out
    }

    #[test]
    fn dat_category_lookup_is_case_insensitive_and_lenient_on_corruption() {
        let dir = std::env::temp_dir().join("vpn-parser-dat-category-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("geosite.dat");
        std::fs::write(&path, build_dat(&["cn", "private", "category-ads-all"])).unwrap();

        // 命中（任意大小写）
        assert!(dat_has_category(&path, "cn"));
        assert!(dat_has_category(&path, "CN"));
        assert!(dat_has_category(&path, "CATEGORY-ADS-ALL"));
        // 不存在 → false（编译器据此丢弃规则，防内核拒启）
        assert!(!dat_has_category(&path, "lan"));
        assert!(!dat_has_category(&path, "LAN"));
        // 文件不存在 / 结构对不上 → true（裁决权交还内核）
        assert!(dat_has_category(&dir.join("missing.dat"), "cn"));
        let corrupt = dir.join("corrupt.dat");
        std::fs::write(&corrupt, [0x08, 0x01, 0x02]).unwrap();
        assert!(dat_has_category(&corrupt, "cn"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn truncated_or_html_bodies_are_rejected() {
        // 这次事故的原样：半截库必须判坏（4MB 或 130KB 都低于 8MB 安全阈值）。
        assert!(validate_geo_body("geosite.dat", &vec![0x0a; 130_209]).is_err());
        assert!(validate_geo_body("geosite.dat", &vec![0x0a; 4_233_529]).is_err());
        assert!(validate_geo_body("geosite.dat", b"<html>rate limited</html>").is_err());
        let mut junk = vec![b'z'; 10_000_000];
        junk[0] = b'Z';
        assert!(validate_geo_body("geosite.dat", &junk).is_err());
        let mut good = vec![0x0a; 9_000_000];
        good[0] = 0x0a;
        assert_eq!(validate_geo_body("geosite.dat", &good).unwrap(), 9_000_000);
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
        let dir = std::env::temp_dir().join(format!("vpn-parser-geo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!geo_ready(&dir));
        std::fs::write(dir.join("geosite.dat"), vec![0x0a; 130_209]).unwrap();
        std::fs::write(dir.join("geoip.dat"), vec![0x0a; 2_000_000]).unwrap();
        // 两个半截库仍然算没就绪 —— 这正是"能起内核"和"不能起内核"的分界。
        assert!(!geo_ready(&dir));
        std::fs::write(dir.join("geosite.dat"), vec![0x0a; 11_000_000]).unwrap();
        // geoip.dat 仍然只有 2MB，未达 8MB 阈值，仍然不算就绪
        assert!(!geo_ready(&dir));
        std::fs::write(dir.join("geoip.dat"), vec![0x0a; 16_000_000]).unwrap();
        assert!(geo_ready(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_populate_vpcore_work_dir() {
        let home = std::env::temp_dir().join(format!("vpn-parser-vpcore-home-{}", uuid::Uuid::new_v4()));
        let work = std::env::temp_dir().join(format!("vpn-parser-vpcore-work-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&work).unwrap();

        std::fs::write(home.join("geosite.dat"), b"fake-geosite").unwrap();
        std::fs::write(home.join("Country.mmdb"), b"fake-country-mmdb").unwrap();

        populate_vpcore_work_dir(&home, &work);

        assert_eq!(std::fs::read(work.join("geosite.dat")).unwrap(), b"fake-geosite");
        assert_eq!(std::fs::read(work.join("Country.mmdb")).unwrap(), b"fake-country-mmdb");

        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&work);
    }

    #[tokio::test]
    async fn test_ensure_vpcore_geo_when_files_ready() {
        let home = std::env::temp_dir().join(format!("vpn-parser-vpcore-geo-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();

        // 模拟已存在足额大小的 geosite.dat 和 geoip.dat
        std::fs::write(home.join("geosite.dat"), vec![0x0a; MIN_GEOSITE_BYTES as usize]).unwrap();
        std::fs::write(home.join("geoip.dat"), vec![0x0a; MIN_GEOIP_BYTES as usize]).unwrap();

        let report = ensure_vpcore_geo(&home, None).await.unwrap();
        assert_eq!(report, "VP-Core 自有分流数据库已就绪");

        let _ = std::fs::remove_dir_all(&home);
    }
}

//! 节点落地 IP 的"欺诈/风控"画像。
//!
//! 延迟低不等于能用：OpenAI、Google、Claude 这类站点会按 ASN 风控，机房 IP 即使
//! 连通也更容易掉登录、封号或被判定为自动化流量。单靠 ipinfo 的 org 关键字会大量
//! 漏判（AS20473 其实是 Vultr、AS3258 是 xTom……），所以这里双源合并：
//! ipinfo.io + ipwho.is，两个都免 key，任一判机房就算机房。

use anyhow::{anyhow, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpRisk {
    pub ip: String,
    /// 查询时用的节点 server 字段：域名节点会先解析，这里留着说明"查的是谁"。
    pub hostname: String,
    pub country: String,
    pub org: String,
    /// 机房 / 云厂商 IP（最容易触发风控）。
    pub datacenter: bool,
    /// ip-api 把它标成公开代理 / VPN 出口（VPN Gate 这类一定为真）。
    pub public_proxy: bool,
    pub verdict: String,
}

impl IpRisk {
    /// 三档短词：公开代理（VPN 出口池）> 机房 > 住宅。徽章上只露两档（代理算"不是住宅"，
    /// 并进机房），真判据留在 hover 和弹框里说清楚。
    pub fn kind_word(&self) -> &'static str {
        if self.public_proxy {
            "公开代理"
        } else if self.datacenter {
            "机房"
        } else {
            "住宅"
        }
    }

    /// `ip · 国家 · ASN/运营商 · 类型`，把归属原样带出来：关键字判定会漏，
    /// 用户能自己看出 "AS401615 ACCK LLC" 这种其实是 VPS 商家。
    pub fn headline(&self) -> String {
        let kind = self.kind_word();
        [
            if self.hostname == self.ip {
                self.ip.clone()
            } else {
                format!("{} → {}", self.hostname, self.ip)
            },
            if self.country.is_empty() {
                "未知地区".to_owned()
            } else {
                self.country.clone()
            },
            if self.org.is_empty() {
                "无 ASN 信息".to_owned()
            } else {
                self.org.clone()
            },
            kind.to_owned(),
        ]
        .join(" · ")
    }
}

#[derive(Debug, Deserialize)]
struct IpInfo {
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    org: Option<String>,
    #[serde(default, rename = "priv")]
    priv_type: Option<String>,
    #[serde(default)]
    company: Option<IpInfoCompany>,
}

#[derive(Debug, Deserialize)]
struct IpInfoCompany {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// ip-api.com 免费档：直接给 `hosting` / `proxy` / `mobile` 三个判定，比我们的关键字
/// 表准得多（实测 AS401615 ACCK、AS20473 Vultr 都能识别成 hosting）。注意免费档只有
/// HTTP、且不能经代理查询；`country` 对 anycast IP 会乱给，所以地区仍以 ipinfo 为准。
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct IpApi {
    #[serde(default)]
    hosting: Option<bool>,
    #[serde(default)]
    proxy: Option<bool>,
    #[serde(default)]
    mobile: Option<bool>,
}

impl IpApi {
    fn flags_datacenter(&self) -> bool {
        matches!(self.hosting, Some(true))
    }
}

/// 常见云厂商与 IDC 的 ASN 名称关键字（org 形如 `AS14061 DigitalOcean, LLC`）。
const DATACENTER_HINTS: &[&str] = &[
    "amazon",
    "aws",
    "microsoft",
    "azure",
    "google",
    "gcp",
    "alibaba",
    "aliyun",
    "tencent",
    "huawei",
    "oracle",
    "hetzner",
    "ovh",
    "digitalocean",
    "vultr",
    "linode",
    "akamai",
    "bytedance",
    "cloudflare",
    "hosting",
    "data center",
    "datacenter",
    "internet services",
    "servers",
    "cloud",
    "acck",
    "contabo",
    "hostinger",
    "ionos",
    "selectel",
    "zomro",
    "racknerd",
    "leaseweb",
    "worldstream",
    "serverius",
    "aeza",
    "beget",
    "tickets",
    "psychz",
    "gige",
    "buyvm",
    "phoenixnap",
    "vds",
    "vps",
    "solutions llc",
    "technology inc",
    "networks hub",
    // 以下都是实测订阅里出现、但早期关键字漏判的 VPS/IDC 厂商。
    "the constant company",
    "xtom",
    "evoxt",
    "dmit",
    "akilecloud",
    "hostpapa",
    "multacom",
    "cnservers",
    "it7 networks",
    "catixs",
    "bytevirt",
    "cyberverse",
    "akari",
    "starry network",
    "host universal",
    "zouter",
    "idc cube",
    "sakura internet",
    "data communication",
    "choopa",
];

fn classify(info: &IpInfo, api: Option<&IpApi>) -> bool {
    let kind_is_hosting = info
        .priv_type
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("hosting"))
        || info
            .company
            .as_ref()
            .and_then(|company| company.kind.as_deref())
            .is_some_and(|value| value.eq_ignore_ascii_case("hosting"));
    if kind_is_hosting || api.is_some_and(IpApi::flags_datacenter) {
        return true;
    }
    let org = info.org.as_deref().unwrap_or_default().to_ascii_lowercase();
    let company = info
        .company
        .as_ref()
        .and_then(|value| value.name.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    DATACENTER_HINTS
        .iter()
        .any(|hint| org.contains(hint) || company.contains(hint))
}

/// `api_body` 是 ip-api.com 的响应；拿不到就退回 ipinfo + 关键字这一路。
pub fn parse(hostname: &str, ip: &str, body: &str, api_body: Option<&str>) -> Result<IpRisk> {
    let info: IpInfo =
        serde_json::from_str(body).map_err(|error| anyhow!("解析 IP 归属失败: {error}"))?;
    let api: Option<IpApi> = api_body.and_then(|text| serde_json::from_str::<IpApi>(text).ok());
    build_ip_risk(hostname, ip, &info, api.as_ref())
}

fn build_ip_risk(hostname: &str, ip: &str, info: &IpInfo, api: Option<&IpApi>) -> Result<IpRisk> {
    let datacenter = classify(info, api);
    let public_proxy = api
        .as_ref()
        .is_some_and(|value| matches!(value.proxy, Some(true)));
    let country = info.country.clone().unwrap_or_default();
    let org = info.org.clone().unwrap_or_default();
    let verdict = if datacenter {
        "机房/云厂商出口：AI 站点（OpenAI / Google / Claude）更容易按 ASN 风控，掉登录、要求验证或封号的风险偏高"
            .to_owned()
    } else if public_proxy {
        "不是机房，但被标成公开代理 / VPN 出口：账号类站点仍会区别对待".to_owned()
    } else {
        "住宅/ISP 出口：风控友好，通常可以正常登录与付费验证".to_owned()
    };
    Ok(IpRisk {
        ip: ip.to_owned(),
        hostname: hostname.to_owned(),
        country,
        org,
        datacenter,
        public_proxy,
        verdict,
    })
}

/// 节点 server 是域名时先解析成 IP。注意域名前置 / ECH / CDN 中转的节点，这里解析到
/// 的往往是前端地址而不是真实落地出口，所以结果里保留 hostname 让界面能说清查的是谁。
async fn resolve_first(host: &str) -> Result<std::net::IpAddr> {
    let addrs = tokio::net::lookup_host((host, 0))
        .await
        .map_err(|error| anyhow!("解析节点域名 {host} 失败: {error}"))?
        .collect::<Vec<_>>();
    // 优先 IPv4：ipinfo 对 IPv6 经常缺 org / country 字段。
    addrs
        .iter()
        .find(|socket| socket.is_ipv4())
        .map(|socket| socket.ip())
        .or_else(|| addrs.first().map(|socket| socket.ip()))
        .ok_or_else(|| anyhow!("节点域名 {host} 没有可用解析记录"))
}

async fn fetch_text(client: &reqwest::Client, url: &str, what: &str) -> Result<String> {
    client
        .get(url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        // `{:?}`/`{}` 只打印最外层（"error sending request for url (...)"），真正的原因
        // 在 source 链里，必须用 `:#` 才带得出来。
        .map_err(|error| anyhow!("{what} 查询失败: {error:#}"))?
        .error_for_status()
        .map_err(|error| anyhow!("{what} 查询失败: {error:#}"))?
        .text()
        .await
        .map_err(|error| anyhow!("{what} 读取响应失败: {error:#}"))
}

async fn fetch_ipinfo(client: &reqwest::Client, address: std::net::IpAddr) -> Result<IpInfo> {
    let url = format!("https://ipinfo.io/{address}/json");
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| anyhow!("ipinfo 请求异常: {e:#}"))?;
    if resp.status().as_u16() == 429 {
        return Err(anyhow!("ipinfo 触发 429 速率限制"));
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| anyhow!("ipinfo 状态错误: {e:#}"))?;
    let text = resp
        .text()
        .await
        .map_err(|e| anyhow!("ipinfo 读取失败: {e:#}"))?;
    let info: IpInfo =
        serde_json::from_str(&text).map_err(|e| anyhow!("ipinfo 解析失败: {e}"))?;
    Ok(info)
}

async fn fetch_ipwhois(client: &reqwest::Client, address: std::net::IpAddr) -> Result<IpInfo> {
    let url = format!("https://ipwho.is/{address}");
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| anyhow!("ipwho.is 请求异常: {e:#}"))?;
    if resp.status().as_u16() == 429 {
        return Err(anyhow!("ipwho.is 触发 429 速率限制"));
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| anyhow!("ipwho.is 状态错误: {e:#}"))?;
    let text = resp
        .text()
        .await
        .map_err(|e| anyhow!("ipwho.is 读取失败: {e:#}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| anyhow!("ipwho.is 解析失败: {e}"))?;
    if v.get("success").and_then(|s| s.as_bool()) == Some(false) {
        return Err(anyhow!("ipwho.is 返回未成功"));
    }
    let country = v
        .get("country_code")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let conn = v.get("connection");
    let asn = conn.and_then(|c| c.get("asn")).and_then(|a| a.as_u64());
    let org_name = conn
        .and_then(|c| c.get("org"))
        .and_then(|o| o.as_str())
        .or_else(|| conn.and_then(|c| c.get("isp")).and_then(|i| i.as_str()));
    let org = match (asn, org_name) {
        (Some(asn), Some(name)) => Some(format!("AS{asn} {name}")),
        (None, Some(name)) => Some(name.to_string()),
        (Some(asn), None) => Some(format!("AS{asn}")),
        (None, None) => None,
    };
    Ok(IpInfo {
        country,
        org,
        priv_type: None,
        company: None,
    })
}

async fn fetch_ipapi(
    client: &reqwest::Client,
    address: std::net::IpAddr,
) -> Result<(IpInfo, IpApi)> {
    let url = format!(
        "http://ip-api.com/json/{address}?fields=status,message,country,countryCode,hosting,proxy,mobile,as,isp,org"
    );
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| anyhow!("ip-api 请求异常: {e:#}"))?;
    if resp.status().as_u16() == 429 {
        return Err(anyhow!("ip-api 触发 429 速率限制"));
    }
    let resp = resp
        .error_for_status()
        .map_err(|e| anyhow!("ip-api 状态错误: {e:#}"))?;
    let text = resp
        .text()
        .await
        .map_err(|e| anyhow!("ip-api 读取失败: {e:#}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| anyhow!("ip-api 解析失败: {e}"))?;
    if v.get("status").and_then(|s| s.as_str()) != Some("success") {
        return Err(anyhow!("ip-api 返回未成功: {:?}", v.get("message")));
    }
    let country = v
        .get("countryCode")
        .and_then(|c| c.as_str())
        .map(|s| s.to_string());
    let as_str = v.get("as").and_then(|a| a.as_str());
    let org_str = v
        .get("org")
        .and_then(|o| o.as_str())
        .or_else(|| v.get("isp").and_then(|i| i.as_str()));
    let org = as_str
        .map(|s| s.to_string())
        .or_else(|| org_str.map(|s| s.to_string()));
    let hosting = v.get("hosting").and_then(|h| h.as_bool());
    let proxy = v.get("proxy").and_then(|p| p.as_bool());
    let mobile = v.get("mobile").and_then(|m| m.as_bool());
    Ok((
        IpInfo {
            country,
            org,
            priv_type: None,
            company: None,
        },
        IpApi {
            hosting,
            proxy,
            mobile,
        },
    ))
}

async fn classify_ip(
    client: &reqwest::Client,
    label: &str,
    address: std::net::IpAddr,
) -> Result<IpRisk> {
    let mut ip_info: Option<IpInfo> = None;
    let mut ip_api: Option<IpApi> = None;
    let mut errors: Vec<String> = Vec::new();

    // 1. 优先尝试 ipinfo.io
    match fetch_ipinfo(client, address).await {
        Ok(info) => ip_info = Some(info),
        Err(err) => {
            tracing::warn!(address = %address, error = %err, "ipinfo 查询未命中或被限流，降级尝试 ipwho.is");
            errors.push(err.to_string());
        }
    }

    // 2. 若 ipinfo 429 或失败，自动降级至 ipwho.is
    if ip_info.is_none() {
        match fetch_ipwhois(client, address).await {
            Ok(info) => ip_info = Some(info),
            Err(err) => {
                tracing::warn!(address = %address, error = %err, "ipwho.is 查询失败，降级尝试 ip-api");
                errors.push(err.to_string());
            }
        }
    }

    // 3. 辅助查询 ip-api（获取 hosting/proxy/mobile 标记，若前面都失败则作为兜底数据源）
    match fetch_ipapi(client, address).await {
        Ok((info, api)) => {
            if ip_info.is_none() {
                ip_info = Some(info);
            }
            ip_api = Some(api);
        }
        Err(err) => {
            errors.push(err.to_string());
        }
    }

    let Some(info) = ip_info else {
        return Err(anyhow!(
            "所有 IP 归属数据源均查询失败 (触发速率限制/网络超时): {}",
            errors.join("; ")
        ));
    };

    build_ip_risk(label, &address.to_string(), &info, ip_api.as_ref())
}

pub async fn lookup(client: &reqwest::Client, server: &str) -> Result<IpRisk> {
    let address = match server.parse::<std::net::IpAddr>() {
        Ok(ip) => ip,
        Err(_) => resolve_first(server).await?,
    };
    if let std::net::IpAddr::V4(v4) = address {
        let octets = v4.octets();
        if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
            return Err(anyhow!("节点地址为本地 TUN Fake-IP ({v4})，跳过外部归属查询"));
        }
    }
    classify_ip(client, server, address).await
}

/// 经代理问"站点看到的我是谁"，再直连查这个出口的归属。域名前置 / CF 中转的节点，
/// 只有这个数才是真实落地，节点服务器 IP 往往只是入口。
pub async fn lookup_egress(direct: &reqwest::Client, proxy_url: &str) -> Result<IpRisk> {
    let proxied = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url).map_err(|error| anyhow!("代理地址无效: {error}"))?)
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| anyhow!("构建代理客户端失败: {error}"))?;
    // 三个回显地址轮流试：内核每 300 秒换成员，某个出口打不通其中一家站点的概率不低。
    let mut last_error = String::from("没有可用的回显地址");
    for url in [
        "https://api.ipify.org",
        "https://icanhazip.com",
        "https://ipinfo.io/ip",
    ] {
        match fetch_text(&proxied, url, "出口 IP").await {
            Ok(body) => {
                let candidate = body
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                if let Ok(address) = candidate.parse::<std::net::IpAddr>() {
                    return classify_ip(direct, &candidate, address).await;
                }
                last_error = format!("{url} 返回了不是 IP 的内容: {candidate:?}");
            }
            Err(error) => last_error = error.to_string(),
        }
    }
    Err(anyhow!("经代理取出口 IP 失败: {last_error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosting_org_is_flagged_as_datacenter() {
        let body = r#"{"ip":"104.18.1.2","country":"US","org":"AS13335 Cloudflare, Inc.","city":"San Francisco"}"#;
        let risk = parse("104.18.1.2", "104.18.1.2", body, None).unwrap();
        assert!(risk.datacenter);
        assert!(risk.verdict.contains("机房"));
        assert_eq!(risk.country, "US");
        // server 本身就是 IP 时，摘要里不重复挂域名。
        assert!(!risk.headline().contains('→'));
    }

    #[test]
    fn isp_org_is_residential() {
        let body = r#"{"country":"JP","org":"AS4713 TAIHAN INDEPENDENT TELCO TRUNK LINE (TITTL)"}"#;
        let risk = parse("1.2.3.4", "1.2.3.4", body, None).unwrap();
        assert!(!risk.datacenter);
        assert!(risk.verdict.contains("住宅"));
    }

    #[test]
    fn priv_type_hosting_wins() {
        let body = r#"{"country":"DE","org":"AS24940 Anybody LLC","priv":"hosting"}"#;
        assert!(parse("5.6.7.8", "5.6.7.8", body, None).unwrap().datacenter);
    }

    #[test]
    fn resolved_domain_keeps_the_queried_hostname() {
        let body = r#"{"country":"US","org":"AS13335 Cloudflare, Inc."}"#;
        let risk = parse("nrt.example.mn", "104.18.42.163", body, None).unwrap();
        assert_eq!(risk.hostname, "nrt.example.mn");
        assert!(
            risk.headline()
                .starts_with("nrt.example.mn → 104.18.42.163"),
            "{}",
            risk.headline()
        );
    }

    /// 三档短词是徽章与 hover 共同的判据来源：公开代理 > 机房 > 住宅。
    #[test]
    fn the_three_way_kind_word_prefers_the_strictest_flag() {
        let mut risk = parse(
            "1.2.3.4",
            "1.2.3.4",
            r#"{"country":"JP","org":"AS212259 Astrix Co., Ltd."}"#,
            None,
        )
        .unwrap();
        risk.datacenter = false;
        risk.public_proxy = false;
        assert_eq!(risk.kind_word(), "住宅");
        risk.datacenter = true;
        assert_eq!(risk.kind_word(), "机房");
        risk.public_proxy = true;
        assert_eq!(risk.kind_word(), "公开代理");
        assert!(risk.headline().ends_with("公开代理"), "{}", risk.headline());
    }

    /// 关键字表补不完（实测订阅里 132/358 被漏判成住宅），所以 ip-api 那一路
    /// 单独命中也要算机房。
    #[test]
    fn ip_api_hosting_flag_rescues_keyword_misses() {
        let body = r#"{"country":"JP","org":"AS12345 Widget Telecom KK"}"#;
        assert!(!parse("9.9.9.9", "9.9.9.9", body, None).unwrap().datacenter);
        let api = r#"{"status":"success","hosting":true,"proxy":false,"mobile":false}"#;
        let risk = parse("9.9.9.9", "9.9.9.9", body, Some(api)).unwrap();
        assert!(risk.datacenter);
        assert!(!risk.public_proxy);
    }

    #[test]
    fn public_proxy_without_hosting_gets_its_own_verdict() {
        let body = r#"{"country":"JP","org":"AS2527 Sony Network Communications Inc."}"#;
        let api = r#"{"status":"success","hosting":false,"proxy":true,"mobile":false}"#;
        let risk = parse("182.170.241.155", "182.170.241.155", body, Some(api)).unwrap();
        assert!(!risk.datacenter);
        assert!(risk.public_proxy);
        assert!(risk.verdict.contains("公开代理"), "{}", risk.verdict);
    }

    #[test]
    fn broken_api_body_falls_back_to_ipinfo_only() {
        let body = r#"{"country":"KR","org":"AS9848 Korea Telecom"}"#;
        let risk = parse("1.1.1.1", "1.1.1.1", body, Some("<html>429</html>")).unwrap();
        assert!(!risk.datacenter);
        assert!(!risk.public_proxy);
    }
}

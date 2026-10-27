use std::net::IpAddr;

/// Whether a proxy server host is suitable for testing / quality export.
/// Rejects loopback, private, link-local, unspecified, multicast, and common
/// placeholder hostnames that appear in broken subscription dumps.
pub fn is_public_proxy_host(host: &str) -> bool {
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return false;
    }
    let lower = host.to_ascii_lowercase();

    // Explicit bad hostnames.
    if matches!(
        lower.as_str(),
        "localhost"
            | "localhost.localdomain"
            | "local"
            | "0.0.0.0"
            | "::"
            | "::1"
            | "example.com"
            | "example.org"
            | "example.net"
            | "test"
            | "invalid"
    ) || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".invalid")
        || lower.ends_with(".test")
        || lower.ends_with(".example")
        || lower.ends_with(".lan")
        || lower.ends_with(".home")
        || lower.ends_with(".internal")
    {
        return false;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public_ip(ip);
    }

    // Hostname: require at least one dot (reject bare "localhost"-like single labels
    // except we already rejected known ones; bare single labels are often junk).
    if !lower.contains('.') {
        return false;
    }

    true
}

/// Hosts that free subscriptions often put as fake "nodes" (real websites / CDNs).
/// These may pass TCP+HTTP probes but are not usable proxies.
pub fn is_decoy_website_host(host: &str) -> bool {
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return false;
    }
    let lower = host.to_ascii_lowercase();
    // Exact or subdomain match against common decoy domains.
    const DECOYS: &[&str] = &[
        "themeforest.net",
        "shopify.com",
        "www.shopify.com",
        "fbi.gov",
        "www.fbi.gov",
        "cloudflare.com",
        "www.cloudflare.com",
        "blog.cloudflare.com",
        "google.com",
        "www.google.com",
        "youtube.com",
        "www.youtube.com",
        "github.com",
        "www.github.com",
        "microsoft.com",
        "www.microsoft.com",
        "apple.com",
        "www.apple.com",
        "amazon.com",
        "www.amazon.com",
        "facebook.com",
        "www.facebook.com",
        "twitter.com",
        "x.com",
        "wikipedia.org",
        "www.wikipedia.org",
        "baidu.com",
        "www.baidu.com",
        "qq.com",
        "www.qq.com",
        "skk.moe",
        "ip.sb",
        "1.1.1.1",
        "8.8.8.8",
        "8.8.4.4",
        "9.9.9.9",
    ];
    for d in DECOYS {
        if lower == *d || lower.ends_with(&format!(".{d}")) {
            return true;
        }
    }
    // Fastly / common CDN edge host patterns used as fake servers.
    if lower.contains("fastly")
        || lower.contains("cloudfront.net")
        || lower.ends_with(".cdn.cloudflare.net")
        || lower.contains("hosting-ip.com")
    {
        return true;
    }
    false
}

/// Host suitable for *quality export* (stricter than speedtest allow-list).
pub fn is_quality_export_host(host: &str) -> bool {
    is_public_proxy_host(host) && !is_decoy_website_host(host)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 0.0.0.0/8
            if o[0] == 0 {
                return false;
            }
            // 10.0.0.0/8
            if o[0] == 10 {
                return false;
            }
            // 127.0.0.0/8 loopback
            if o[0] == 127 {
                return false;
            }
            // 169.254.0.0/16 link-local
            if o[0] == 169 && o[1] == 254 {
                return false;
            }
            // 172.16.0.0/12
            if o[0] == 172 && (16..=31).contains(&o[1]) {
                return false;
            }
            // 192.168.0.0/16
            if o[0] == 192 && o[1] == 168 {
                return false;
            }
            // 100.64.0.0/10 CGNAT
            if o[0] == 100 && (64..=127).contains(&o[1]) {
                return false;
            }
            // 192.0.0.0/24 IETF protocol assignments
            if o[0] == 192 && o[1] == 0 && o[2] == 0 {
                return false;
            }
            // 198.18.0.0/15 Network benchmark / fake-ip
            if o[0] == 198 && (o[1] == 18 || o[1] == 19) {
                return false;
            }
            // 224.0.0.0/4 multicast + 240.0.0.0/4 reserved
            if o[0] >= 224 {
                return false;
            }
            true
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
            {
                return false;
            }
            // IPv4-mapped
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            true
        }
    }
}

/// 判断 CIDR 字符串是否为私有 / 环回 / 链路本地 / 预留网段（域名解析对其默认启用 no-resolve）。
pub fn is_private_cidr(cidr: &str) -> bool {
    let ip_str = cidr.split_once('/').map(|(ip, _)| ip).unwrap_or(cidr).trim();
    if let Ok(ip) = ip_str.parse::<IpAddr>() {
        !is_public_ip(ip)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_loopback_and_private() {
        assert!(!is_public_proxy_host("127.0.0.1"));
        assert!(!is_public_proxy_host("localhost"));
        assert!(!is_public_proxy_host("192.168.1.1"));
        assert!(!is_public_proxy_host("10.0.0.2"));
        assert!(!is_public_proxy_host("172.16.0.5"));
        assert!(!is_public_proxy_host("::1"));
        assert!(!is_public_proxy_host("0.0.0.0"));
    }

    #[test]
    fn accepts_public() {
        assert!(is_public_proxy_host("1.1.1.1"));
        assert!(is_public_proxy_host("cf.090227.xyz"));
        assert!(is_public_proxy_host("example-node.com"));
    }

    #[test]
    fn rejects_decoy_websites_for_quality_export() {
        assert!(is_decoy_website_host("themeforest.net"));
        assert!(is_decoy_website_host("www.fbi.gov"));
        assert!(is_decoy_website_host("blog.cloudflare.com"));
        assert!(is_decoy_website_host("trt.fastlynew.hosting-ip.com"));
        assert!(!is_quality_export_host("themeforest.net"));
        assert!(is_quality_export_host("cf.090227.xyz"));
    }

    #[test]
    fn test_is_private_cidr() {
        assert!(is_private_cidr("10.0.0.0/8"));
        assert!(is_private_cidr("127.0.0.1/32"));
        assert!(is_private_cidr("172.16.0.0/12"));
        assert!(is_private_cidr("192.168.1.0/24"));
        assert!(is_private_cidr("169.254.0.0/16"));
        assert!(is_private_cidr("100.64.0.0/10"));
        assert!(is_private_cidr("198.18.0.0/15"));
        assert!(is_private_cidr("fc00::/7"));
        assert!(is_private_cidr("fe80::/10"));
        assert!(is_private_cidr("::1/128"));
        assert!(!is_private_cidr("8.8.8.8/32"));
        assert!(!is_private_cidr("1.1.1.1"));
    }
}

use crate::models::Node;
use serde_json::Value;

/// Score a node for client export. Higher is better.
/// TCP latency alone is a bad ranker for free lists (open ports / decoys win).
pub fn export_score(node: &Node) -> i64 {
    let mut score: i64 = 0;
    let proto = node.protocol.to_ascii_lowercase();
    let cfg = &node.config;

    // Base: prefer lower TCP latency strongly, but not exclusively.
    if let Some(ms) = node.latency_ms {
        score += (20_000 - ms.clamp(1, 19_999)) as i64;
    }

    // Protocol preference for typical v2rayN usage.
    score += match proto.as_str() {
        "vless" => 300,
        "trojan" => 280,
        "shadowsocks" | "ss" => 250,
        "vmess" => 200,
        "hysteria2" | "hy2" => 220,
        "shadowsocksr" | "ssr" => -200,
        "http" | "https" | "socks5" | "socks" => -500,
        _ => 0,
    };

    let security = security_kind(cfg);
    score += match security {
        SecurityKind::Reality => 2_000,
        SecurityKind::Tls => 1_200,
        SecurityKind::None => -800,
        SecurityKind::Unknown => -200,
    };

    // Credentials present?
    if has_credentials(node) {
        score += 400;
    } else {
        score -= 2_000;
    }

    // Transport hints
    let network = first_str(cfg, &["network", "net"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    score += match network.as_str() {
        "ws" | "websocket" | "grpc" | "h2" | "httpupgrade" | "xhttp" => 80,
        "tcp" | "raw" | "" => 40,
        _ => 0,
    };

    // Port heuristics: bare 80 without TLS is often junk/open CDN.
    match (node.port, security) {
        (443 | 8443 | 2053 | 2083 | 2087 | 2096, SecurityKind::Tls | SecurityKind::Reality) => {
            score += 120
        }
        (80 | 8080 | 8880 | 2052 | 2082 | 2086 | 2095, SecurityKind::None) => score -= 600,
        (80 | 8080 | 8880, SecurityKind::Tls | SecurityKind::Reality) => score += 40,
        _ => {}
    }

    // SNI present for TLS family
    if matches!(security, SecurityKind::Tls | SecurityKind::Reality) {
        if first_str(cfg, &["sni", "servername", "host"])
            .filter(|s| !s.is_empty())
            .is_some()
        {
            score += 100;
        } else {
            score -= 80;
        }
    }

    // Spam names are weak signal but still useful for ranking.
    let name = node.name.to_ascii_lowercase();
    if name.contains("telegram")
        || name.contains("t.me")
        || name.contains("join")
        || name.contains("http")
    {
        score -= 150;
    }

    score
}

/// Whether a node is worth exporting under quality_only mode.
#[cfg(test)]
pub fn is_strict_export_candidate(node: &Node) -> bool {
    if !has_credentials(node) {
        return false;
    }
    let proto = node.protocol.to_ascii_lowercase();
    if matches!(
        proto.as_str(),
        "http" | "https" | "socks" | "socks5" | "mieru" | "shadowsocksr" | "ssr"
    ) {
        return false;
    }

    // Require TLS/REALITY for protocols that normally use it in modern free lists.
    // Plain none-security open ports are the main source of v2rayN `-1`.
    match proto.as_str() {
        "vless" | "vmess" | "trojan" => {
            matches!(
                security_kind(&node.config),
                SecurityKind::Tls | SecurityKind::Reality
            )
        }
        "shadowsocks" | "ss" => {
            // Must have a real cipher (not empty / none / auto alone without password already checked)
            let cipher = first_str(&node.config, &["cipher", "method"])
                .unwrap_or_default()
                .to_ascii_lowercase();
            !cipher.is_empty() && cipher != "none"
        }
        "hysteria2" | "hy2" | "hysteria" | "tuic" => true,
        _ => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecurityKind {
    Reality,
    Tls,
    None,
    Unknown,
}

fn security_kind(cfg: &Value) -> SecurityKind {
    if cfg.get("reality-opts").is_some() {
        return SecurityKind::Reality;
    }
    let s = first_str(cfg, &["security", "tls"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if s == "reality" {
        return SecurityKind::Reality;
    }
    if s == "tls" || s == "true" || s == "1" {
        return SecurityKind::Tls;
    }
    // bool tls
    if matches!(cfg.get("tls"), Some(Value::Bool(true))) {
        return SecurityKind::Tls;
    }
    if s.is_empty() {
        // trojan is implicitly TLS unless cleartext
        return SecurityKind::Unknown;
    }
    if s == "none" || s == "false" || s == "0" || s == "auto" {
        // vmess scy=auto is cipher not security; handled by caller context.
        if s == "auto" {
            return SecurityKind::Unknown;
        }
        return SecurityKind::None;
    }
    SecurityKind::Unknown
}

fn has_credentials(node: &Node) -> bool {
    let cfg = &node.config;
    let proto = node.protocol.to_ascii_lowercase();
    match proto.as_str() {
        "vless" | "vmess" => !first_str(cfg, &["uuid", "id"])
            .unwrap_or_default()
            .is_empty(),
        "trojan" | "hysteria2" | "hy2" | "hysteria" => {
            !first_str(cfg, &["password", "auth", "auth-str"])
                .unwrap_or_default()
                .is_empty()
        }
        "shadowsocks" | "ss" | "shadowsocksr" | "ssr" => {
            !first_str(cfg, &["password"]).unwrap_or_default().is_empty()
                && !first_str(cfg, &["cipher", "method"])
                    .unwrap_or_default()
                    .is_empty()
        }
        "socks5" | "socks" | "http" | "https" => true,
        _ => true,
    }
}

fn first_str(cfg: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = cfg.get(*k) {
            match v {
                Value::String(s) if !s.is_empty() => return Some(s.clone()),
                Value::Bool(b) => return Some(b.to_string()),
                Value::Number(n) => return Some(n.to_string()),
                _ => {}
            }
        }
    }
    None
}


#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn node(protocol: &str, port: u16, ms: i64, cfg: Value) -> Node {
        let now = Utc::now();
        Node {
            id: "1".into(),
            source_id: None,
            protocol: protocol.into(),
            name: "n".into(),
            server: "1.2.3.4".into(),
            port,
            raw: String::new(),
            config: cfg,
            country: None,
            latency_ms: Some(ms),
            latency_ok: Some(true),
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn prefers_reality_over_none() {
        let good = node(
            "vless",
            443,
            200,
            serde_json::json!({"uuid":"a","security":"reality","sni":"a.com"}),
        );
        let bad = node(
            "vless",
            80,
            10,
            serde_json::json!({"uuid":"a","security":"none","network":"ws"}),
        );
        assert!(export_score(&good) > export_score(&bad));
        assert!(is_strict_export_candidate(&good));
        assert!(!is_strict_export_candidate(&bad));
    }
}

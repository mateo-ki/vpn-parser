use crate::models::Node;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tracing::debug;

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

fn needs_tls(node: &Node) -> bool {
    let proto = node.protocol.to_ascii_lowercase();
    if matches!(proto.as_str(), "trojan") {
        return true;
    }
    matches!(
        security_kind(&node.config),
        SecurityKind::Tls | SecurityKind::Reality
    )
}

fn sni_for(node: &Node) -> String {
    first_str(&node.config, &["sni", "servername", "host"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| node.server.clone())
}

/// Re-check candidates live, keep only those still reachable (and TLS-handshakable when needed).
/// Stops early once `want` survivors are collected (candidates should already be score-sorted).
pub async fn live_filter_for_export(nodes: Vec<Node>, want: usize, timeout_ms: u32) -> Vec<Node> {
    if nodes.is_empty() || want == 0 {
        return Vec::new();
    }
    let timeout = Duration::from_millis(timeout_ms.max(300) as u64);
    let concurrency = 64usize;
    let mut iter = nodes.into_iter();
    let mut inflight = JoinSet::new();
    let mut kept = Vec::with_capacity(want);

    loop {
        if kept.len() >= want {
            break;
        }
        while inflight.len() < concurrency {
            let Some(node) = iter.next() else {
                break;
            };
            inflight.spawn(async move {
                let ok = live_probe_node(&node, timeout).await;
                (ok, node)
            });
        }
        let Some(joined) = inflight.join_next().await else {
            break;
        };
        if let Ok((true, node)) = joined {
            kept.push(node);
        }
    }
    // Drain remaining tasks quickly without waiting long? just drop joinset by returning
    kept
}

async fn live_probe_node(node: &Node, timeout: Duration) -> bool {
    let host = node.server.trim();
    if host.is_empty() || node.port == 0 {
        return false;
    }
    let start = Instant::now();
    let addr = format!("{host}:{}", node.port);
    let stream = match tokio::time::timeout(timeout, TcpStream::connect(&addr)).await {
        Ok(Ok(s)) => s,
        _ => return false,
    };
    if !needs_tls(node) {
        // Plain SS / none-security: TCP up is all we can cheaply verify.
        // Reject obvious immediate HTTP websites.
        return !looks_like_immediate_http(stream, timeout.saturating_sub(start.elapsed())).await;
    }

    // TLS handshake (cert verification disabled). Real proxies/CDNs usually complete it;
    // closed/protocol-dead ports often fail.
    tls_handshake(
        stream,
        &sni_for(node),
        timeout.saturating_sub(start.elapsed()),
    )
    .await
}

async fn looks_like_immediate_http(mut stream: TcpStream, budget: Duration) -> bool {
    use tokio::io::AsyncReadExt;
    if budget.is_zero() {
        return false;
    }
    // Don't send request; some open HTTP ports still don't talk first.
    // Send a short HTTP probe instead.
    use tokio::io::AsyncWriteExt;
    let req = b"GET / HTTP/1.0\r\nHost: x\r\n\r\n";
    let _ = tokio::time::timeout(
        budget.min(Duration::from_millis(250)),
        stream.write_all(req),
    )
    .await;
    let mut buf = [0u8; 16];
    match tokio::time::timeout(
        budget.min(Duration::from_millis(350)),
        stream.read(&mut buf),
    )
    .await
    {
        Ok(Ok(n)) if n > 0 => {
            let s = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
            s.starts_with("http/") || s.starts_with("<!do") || s.starts_with("<html")
        }
        _ => false,
    }
}

async fn tls_handshake(stream: TcpStream, sni: &str, budget: Duration) -> bool {
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
    use std::sync::Arc;
    use tokio_rustls::TlsConnector;

    #[derive(Debug)]
    struct NoVerify;
    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, TlsError> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, TlsError> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ED25519,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PKCS1_SHA256,
            ]
        }
    }

    if budget.is_zero() {
        return false;
    }
    let mut cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerify))
        .with_no_client_auth();
    cfg.enable_sni = true;
    let connector = TlsConnector::from(Arc::new(cfg));
    let server_name = match ServerName::try_from(sni.to_string()) {
        Ok(n) => n,
        Err(_) => match ServerName::try_from(sni.to_owned()) {
            Ok(n) => n,
            Err(_) => return false,
        },
    };
    match tokio::time::timeout(budget, connector.connect(server_name, stream)).await {
        Ok(Ok(_)) => true,
        Ok(Err(e)) => {
            debug!(error = %e, sni, "tls handshake failed");
            false
        }
        Err(_) => false,
    }
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

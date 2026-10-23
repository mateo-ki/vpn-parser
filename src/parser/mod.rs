mod clash;
mod common;
pub mod openrung;
mod uri;
pub mod warp_masque;

pub use clash::{extract_clash_rules, parse_clash_yaml};
pub use common::{
    decode_maybe_base64, expand_url_template, extract_uris, normalize_subscription_text,
};
pub use openrung::parse_openrung_json;
pub use uri::parse_uri;
pub use warp_masque::parse_warp_masque_json;

use crate::models::Node;
use chrono::Utc;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::json;
use uuid::Uuid;

/// Extract traffic routing rules from a free-form subscription body (e.g. Clash YAML, base64 Clash).
pub fn extract_subscription_rules(content: &str) -> Vec<crate::proxy::TrafficRule> {
    let content = normalize_subscription_text(content);
    let content = content.trim();
    if content.is_empty() {
        return Vec::new();
    }
    if looks_like_clash(content) {
        let rules = extract_clash_rules(content);
        if !rules.is_empty() {
            return rules;
        }
    }
    if let Some(decoded) = decode_maybe_base64(content) {
        let decoded = normalize_subscription_text(&decoded);
        if looks_like_clash(&decoded) {
            let rules = extract_clash_rules(&decoded);
            if !rules.is_empty() {
                return rules;
            }
        }
    }
    Vec::new()
}

/// Parse free-form subscription body into nodes.
/// Supports:
/// - plain URI lists (ss/vmess/vless/trojan/hy2)
/// - base64-encoded URI lists
/// - Clash / Clash Meta YAML
/// - mixed markdown / text with embedded URIs
/// - OpenRung broker relays JSON
pub fn parse_subscription(content: &str, source_id: Option<&str>) -> Vec<Node> {
    let content = normalize_subscription_text(content);
    let content = content.trim();
    if content.is_empty() {
        return Vec::new();
    }

    // 0) Try OpenRung JSON (relays list from broker API)
    if (content.starts_with('{') || content.starts_with('['))
        && (content.contains("\"relays\"") || content.contains("reality_public_key"))
    {
        if let Some(nodes) = parse_openrung_json(content, source_id) {
            if !nodes.is_empty() {
                return dedup_nodes(nodes);
            }
        }
    }

    // 0.1) Try WARP MASQUE JSON (usque-config.json or WARP registration credentials)
    if content.starts_with('{')
        && (content.contains("endpoint_pub_key")
            || content.contains("peerPublicKey")
            || (content.contains("private_key") && content.contains("ipv4")))
    {
        if let Some(nodes) = parse_warp_masque_json(content, source_id) {
            if !nodes.is_empty() {
                return dedup_nodes(nodes);
            }
        }
    }

    // 1) Try Clash YAML first when it looks like YAML proxies list.
    if looks_like_clash(content) {
        if let Ok(nodes) = parse_clash_yaml(content, source_id) {
            if !nodes.is_empty() {
                return dedup_nodes(nodes);
            }
        }
    }

    // 2) Extract share URIs from plain / base64 / mixed text.
    let mut uris = extract_uris(content);
    if uris.is_empty() {
        if let Some(decoded) = decode_maybe_base64(content) {
            let decoded = normalize_subscription_text(&decoded);
            uris = extract_uris(&decoded);
            if uris.is_empty() && looks_like_clash(&decoded) {
                if let Ok(nodes) = parse_clash_yaml(&decoded, source_id) {
                    return dedup_nodes(nodes);
                }
            }
        }
    }

    let mut nodes = Vec::new();
    for uri in uris {
        if let Some(node) = parse_uri(&uri, source_id) {
            nodes.push(node);
        }
    }
    dedup_nodes(nodes)
}

pub fn looks_like_clash(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("proxies:")
        || lower.contains("proxy-groups:")
        || (lower.contains("port:") && lower.contains("type:") && lower.contains("server:"))
}

/// Stable identity for a proxy endpoint (ignores display name / source).
/// Same server:port + credentials => same node, even if name differs across free lists.
pub fn node_fingerprint(node: &Node) -> String {
    fingerprint_from_parts(&node.protocol, &node.server, node.port, &node.config)
}

/// Same identity as [`node_fingerprint`], for callers that only hold the endpoint fields
/// (the node summary listing keeps credentials in a separate table).
pub fn fingerprint_from_parts(
    protocol: &str,
    server: &str,
    port: u16,
    config: &serde_json::Value,
) -> String {
    let protocol = normalize_protocol(protocol);
    let server = server.trim().to_ascii_lowercase();
    let cred = credential_token(&protocol, config);
    format!("{protocol}|{server}|{port}|{cred}")
}

pub fn normalize_protocol(protocol: &str) -> String {
    match protocol.to_ascii_lowercase().as_str() {
        "ss" | "shadowsocks" => "shadowsocks".into(),
        "ssr" | "shadowsocksr" => "shadowsocksr".into(),
        "hy2" | "hysteria2" => "hysteria2".into(),
        "socks" | "socks5" => "socks5".into(),
        "https" => "http".into(),
        other => other.to_string(),
    }
}

fn credential_token(protocol: &str, config: &serde_json::Value) -> String {
    let s = |keys: &[&str]| -> String {
        for k in keys {
            if let Some(v) = config.get(*k) {
                match v {
                    serde_json::Value::String(x) if !x.is_empty() => return x.clone(),
                    serde_json::Value::Number(n) => return n.to_string(),
                    serde_json::Value::Bool(b) => return b.to_string(),
                    _ => {}
                }
            }
        }
        String::new()
    };

    match protocol {
        "shadowsocks" => format!("{}:{}", s(&["cipher", "method"]), s(&["password"])),
        "shadowsocksr" => format!(
            "{}:{}:{}:{}",
            s(&["cipher", "method"]),
            s(&["password"]),
            s(&["protocol", "protocolparam"]),
            s(&["obfs", "obfsparam"])
        ),
        "vmess" | "vless" => s(&["uuid", "id"]),
        "trojan" | "hysteria2" | "hysteria" | "tuic" => {
            s(&["password", "auth", "auth-str", "uuid"])
        }
        "socks5" | "http" => format!("{}:{}", s(&["username", "user"]), s(&["password", "pass"])),
        "wireguard" => s(&["private-key", "private_key", "public-key", "public_key"]),
        "masque" => s(&[
            "private-key",
            "private_key",
            "public-key",
            "public_key",
            "publicKey",
            "privateKey",
        ]),
        _ => {
            // Fallback: hash a few stable fields if present.
            let parts = [
                s(&["uuid", "id"]),
                s(&["password", "auth"]),
                s(&["cipher", "method"]),
                s(&["username"]),
            ];
            parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(":")
        }
    }
}

fn prefer_node(a: &Node, b: &Node) -> bool {
    // true if `a` is better to keep than `b`
    match (a.latency_ok, b.latency_ok) {
        (Some(true), Some(true)) => match (a.latency_ms, b.latency_ms) {
            (Some(am), Some(bm)) if am != bm => am < bm,
            _ => a.updated_at >= b.updated_at,
        },
        (Some(true), _) => true,
        (_, Some(true)) => false,
        (Some(false), Some(false)) => a.updated_at >= b.updated_at,
        (None, None) => {
            // Prefer non-empty / shorter spam names
            let ascore = name_quality(&a.name);
            let bscore = name_quality(&b.name);
            if ascore != bscore {
                ascore > bscore
            } else {
                a.updated_at >= b.updated_at
            }
        }
        (None, Some(false)) => true,
        (Some(false), None) => false,
    }
}

fn name_quality(name: &str) -> i32 {
    let n = name.trim();
    if n.is_empty() {
        return 0;
    }
    let mut score = 10;
    let lower = n.to_ascii_lowercase();
    if lower.contains("telegram") || lower.contains("join") || lower.contains("http") {
        score -= 5;
    }
    // Prefer names that look like region labels over pure spam.
    if n.chars().count() <= 40 {
        score += 2;
    }
    score
}

fn dedup_nodes(nodes: Vec<Node>) -> Vec<Node> {
    use std::collections::HashMap;
    let mut best: HashMap<String, Node> = HashMap::new();
    for n in nodes {
        let key = node_fingerprint(&n);
        match best.get(&key) {
            Some(old) if prefer_node(old, &n) => {}
            _ => {
                best.insert(key, n);
            }
        }
    }
    best.into_values().collect()
}

pub fn make_node(
    protocol: &str,
    name: &str,
    server: &str,
    port: u16,
    raw: &str,
    config: serde_json::Value,
    source_id: Option<&str>,
) -> Node {
    let now = Utc::now();
    Node {
        id: Uuid::new_v4().to_string(),
        source_id: source_id.map(|s| s.to_string()),
        protocol: protocol.to_string(),
        name: if name.is_empty() {
            format!("{server}:{port}")
        } else {
            name.to_string()
        },
        server: server.to_string(),
        port,
        raw: raw.to_string(),
        config,
        country: guess_country(name),
        latency_ms: None,
        latency_ok: None,
        download_kbps: None,
        latency_kernel: None,
        latency_test_kind: None,
        latency_tested_at: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn guess_country(name: &str) -> Option<String> {
    let n = name.to_ascii_lowercase();
    // Prefer ASCII tokens to avoid encoding issues; Chinese labels still matched via contains on original.
    let ascii = [
        ("hongkong", "HK"),
        ("hong kong", "HK"),
        ("taiwan", "TW"),
        ("tokyo", "JP"),
        ("osaka", "JP"),
        ("japan", "JP"),
        ("singapore", "SG"),
        ("losangeles", "US"),
        ("los angeles", "US"),
        ("america", "US"),
        ("united states", "US"),
        ("seoul", "KR"),
        ("korea", "KR"),
        ("london", "GB"),
        ("england", "GB"),
        ("germany", "DE"),
        ("france", "FR"),
        ("canada", "CA"),
        ("australia", "AU"),
        ("russia", "RU"),
        ("india", "IN"),
        ("turkey", "TR"),
        ("netherlands", "NL"),
        ("hk", "HK"),
        ("tw", "TW"),
        ("jp", "JP"),
        ("sg", "SG"),
        ("us", "US"),
        ("usa", "US"),
        ("kr", "KR"),
        ("uk", "GB"),
        ("gb", "GB"),
        ("de", "DE"),
        ("fr", "FR"),
        ("ca", "CA"),
        ("au", "AU"),
        ("ru", "RU"),
        ("in", "IN"),
        ("tr", "TR"),
        ("nl", "NL"),
    ];
    for (k, v) in ascii {
        if n.contains(k) {
            return Some(v.to_string());
        }
    }
    // Chinese region keywords (UTF-8).
    let zh = name;
    let zh_map = [
        ("\u{9999}\u{6e2f}", "HK"),
        ("\u{53f0}\u{6e7e}", "TW"),
        ("\u{53f0}\u{7063}", "TW"),
        ("\u{65e5}\u{672c}", "JP"),
        ("\u{65b0}\u{52a0}\u{5761}", "SG"),
        ("\u{7f8e}\u{56fd}", "US"),
        ("\u{7f8e}\u{570b}", "US"),
        ("\u{97e9}\u{56fd}", "KR"),
        ("\u{97d3}\u{570b}", "KR"),
        ("\u{82f1}\u{56fd}", "GB"),
        ("\u{82f1}\u{570b}", "GB"),
        ("\u{5fb7}\u{56fd}", "DE"),
        ("\u{6cd5}\u{56fd}", "FR"),
        ("\u{52a0}\u{62ff}\u{5927}", "CA"),
        ("\u{6fb3}\u{6d32}", "AU"),
        ("\u{6fb3}\u{5927}\u{5229}\u{4e9a}", "AU"),
        ("\u{4fc4}\u{7f57}\u{65af}", "RU"),
        ("\u{5370}\u{5ea6}", "IN"),
        ("\u{571f}\u{8033}\u{5176}", "TR"),
        ("\u{8377}\u{5170}", "NL"),
    ];
    for (k, v) in zh_map {
        if zh.contains(k) {
            return Some(v.to_string());
        }
    }
    None
}

/// Build a minimal empty node config helper.
#[allow(dead_code)]
pub fn empty_config() -> serde_json::Value {
    json!({})
}

static RE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"https?://\S+|www\.\S+").unwrap());
static RE_DOMAIN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b[a-z0-9-]+\.(?:com|net|org|xyz|top|cc|me|vip|club|site|app|cn|io|icu|link)\b",
    )
    .unwrap()
});
static RE_SCHEME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:vless|vmess|trojan|ss|ssr|hysteria2?|tuic|socks5?)://\S*").unwrap()
});
static RE_BRACKETS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\[[^\]]*(?:tg|群|官网|关注|推广|发布|频道|订阅|购买|永久|最新|防封|免翻|剩余|到期|流量)[^\]]*\]|【[^】]*(?:tg|群|官网|关注|推广|发布|频道|订阅|购买|永久|最新|防封|免翻|剩余|到期|流量)[^】]*】|\([^\)]*(?:tg|群|官网|关注|推广|发布|频道|订阅|购买|永久|最新|防封|免翻|剩余|到期|流量)[^\)]*\)").unwrap()
});
static RE_PROMO: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:TG|交流群|TG群|频道|公众号|关注|推广|防失联|最新地址|永久免费|免费节点|备用地址|邀请码|联系方式)[:：]?\s*@?[\w_.-]+").unwrap()
});
static RE_TRAFFIC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:剩余流量|剩余|到期时间|到期|重置|流量|倍率|倍数|不限速|不限流|高速|推荐|官网|购买)[:：\s-]*[^\s，,、|]*").unwrap()
});
static RE_TG_HANDLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@[\w_]{4,}").unwrap());
static RE_SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

pub fn clean_node_name(name: &str, country_hint: Option<&str>, server: &str, port: u16) -> String {
    let mut s = name.trim().to_string();

    s = RE_URL.replace_all(&s, "").to_string();
    s = RE_DOMAIN.replace_all(&s, "").to_string();
    s = RE_SCHEME.replace_all(&s, "").to_string();
    s = RE_BRACKETS.replace_all(&s, "").to_string();
    s = RE_PROMO.replace_all(&s, "").to_string();
    s = RE_TRAFFIC.replace_all(&s, "").to_string();
    s = RE_TG_HANDLE.replace_all(&s, "").to_string();

    s = s
        .chars()
        .filter(|c| !('\u{1F1E6}'..='\u{1F1FF}').contains(c))
        .collect();
    s = s.replace(
        [
            '|', '*', '★', '☆', '⭐', '🌟', '⚡', '✨', '🔥', '🚀', '👑', '💎', '▲', '▼', '◆', '◇',
            '【', '】', '[', ']', '(', ')', '（', '）',
        ],
        " ",
    );
    s = RE_SPACES.replace_all(&s, " ").to_string();
    s = s
        .trim_matches(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == ':' || c == '：')
        .trim()
        .to_string();

    let country_code = country_hint
        .map(|c| c.to_string())
        .or_else(|| guess_country(&s))
        .or_else(|| guess_country(name));

    if let Some(code) = country_code {
        let zh_name = match code.to_ascii_uppercase().as_str() {
            "HK" => "香港",
            "JP" => "日本",
            "US" => "美国",
            "SG" => "新加坡",
            "TW" => "台湾",
            "KR" => "韩国",
            "GB" | "UK" => "英国",
            "DE" => "德国",
            "FR" => "法国",
            "CA" => "加拿大",
            "AU" => "澳大利亚",
            "RU" => "俄罗斯",
            "IN" => "印度",
            "NL" => "荷兰",
            "MY" => "马来西亚",
            "TH" => "泰国",
            "PH" => "菲律宾",
            "VN" => "越南",
            "TR" => "土耳其",
            _ => "其它",
        };

        let clean_tail = s
            .replace(zh_name, "")
            .replace(&code.to_ascii_uppercase(), "")
            .replace(&code.to_ascii_lowercase(), "")
            .trim_matches(|c: char| c.is_whitespace() || c == '-' || c == '_')
            .trim()
            .to_string();

        if clean_tail.is_empty() {
            format!("{zh_name} {}:{}", server, port)
        } else {
            format!("{zh_name} {clean_tail}")
        }
    } else if s.is_empty() {
        format!("{}:{}", server, port)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ss_uri() {
        let uri = "ss://YWVzLTI1Ni1nY206cGFzc3dvcmQ@1.2.3.4:8388#test";
        let node = parse_uri(uri, None).expect("ss parse");
        assert_eq!(node.server, "1.2.3.4");
        assert_eq!(node.port, 8388);
        assert_eq!(node.protocol, "shadowsocks");
    }

    #[test]
    fn clean_node_name_removes_ads_and_normalizes() {
        assert_eq!(
            clean_node_name(
                "[TG@vpn_share] 🇭🇰 香港 01 - 剩余流量: 999G",
                Some("HK"),
                "1.1.1.1",
                443
            ),
            "香港 01"
        );
        assert_eq!(
            clean_node_name("【推荐关注公众号:xxx】US-02", None, "1.1.1.1", 443),
            "美国 02"
        );
        assert_eq!(
            clean_node_name(
                "⭐官网: https://freevpn.cc - 🇯🇵 日本专线",
                None,
                "1.1.1.1",
                443
            ),
            "日本 专线"
        );
    }

    #[test]
    fn expand_date_template() {
        let url = expand_url_template("https://example.com/{Ymd}-clash.yaml");
        assert!(url.contains("-clash.yaml"));
        assert!(!url.contains("{Ymd}"));
    }

    #[test]
    fn fingerprint_ignores_name() {
        let a = make_node(
            "vless",
            "name-a",
            "1.2.3.4",
            443,
            "vless://u@1.2.3.4:443",
            serde_json::json!({"type":"vless","uuid":"11111111-1111-1111-1111-111111111111","server":"1.2.3.4","port":443}),
            None,
        );
        let b = make_node(
            "vless",
            "name-b-totally-different",
            "1.2.3.4",
            443,
            "vless://u@1.2.3.4:443#other",
            serde_json::json!({"type":"vless","uuid":"11111111-1111-1111-1111-111111111111","server":"1.2.3.4","port":443}),
            None,
        );
        assert_eq!(node_fingerprint(&a), node_fingerprint(&b));
        let out = dedup_nodes(vec![a, b]);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fingerprint_differs_by_uuid() {
        let a = make_node(
            "vless",
            "n",
            "1.2.3.4",
            443,
            "raw",
            serde_json::json!({"uuid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"}),
            None,
        );
        let b = make_node(
            "vless",
            "n",
            "1.2.3.4",
            443,
            "raw",
            serde_json::json!({"uuid":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"}),
            None,
        );
        assert_ne!(node_fingerprint(&a), node_fingerprint(&b));
    }

    #[test]
    fn parse_ss_with_query_and_amp() {
        let uri = "ss://YWVzLTI1Ni1nY206cGFzcw@1.2.3.4:8388?plugin=obfs&amp;foo=1#name";
        let node = parse_uri(uri, None).expect("ss query");
        assert_eq!(node.server, "1.2.3.4");
        assert_eq!(node.port, 8388);
    }

    #[test]
    fn parse_mislabeled_ss_as_vless() {
        let uri = "ss://aaaaaaaa-bbbb-4ccc-8ddd-000000000001@198.51.100.20:443?security=tls&amp;encryption=none&amp;type=tcp&amp;sni=example.com#test";
        let node = parse_uri(uri, None).expect("mislabeled");
        assert_eq!(node.protocol, "vless");
        assert_eq!(node.server, "198.51.100.20");
        assert_eq!(node.port, 443);
    }

    #[test]
    fn parse_vless_with_emoji_fragment() {
        let uri = "vless://aaaaaaaa-bbbb-4ccc-8ddd-000000000002@192.0.2.66:443?security=reality&encryption=none&type=tcp&sni=example.com#Join-Telegram";
        let node = parse_uri(uri, None).expect("vless");
        assert_eq!(node.protocol, "vless");
        assert_eq!(node.server, "192.0.2.66");
    }

    #[test]
    fn parse_solispirit_sample() {
        let path = "data/sample-solispirit.txt";
        let Ok(content) = std::fs::read_to_string(path) else {
            eprintln!("sample missing, skip");
            return;
        };
        let uris = extract_uris(&content);
        let mut parsed = 0usize;
        let mut fail_ss = 0usize;
        let mut fail_vmess = 0usize;
        let mut fail_vless = 0usize;
        let mut fail_trojan = 0usize;
        let mut fail_other = 0usize;
        let mut fail_samples = Vec::new();
        for uri in &uris {
            if parse_uri(uri, None).is_some() {
                parsed += 1;
                continue;
            }
            let low = uri.to_ascii_lowercase();
            if low.starts_with("ss://") {
                fail_ss += 1;
            } else if low.starts_with("vmess://") {
                fail_vmess += 1;
            } else if low.starts_with("vless://") {
                fail_vless += 1;
            } else if low.starts_with("trojan://") {
                fail_trojan += 1;
            } else {
                fail_other += 1;
            }
            if fail_samples.len() < 8 {
                fail_samples.push(uri.chars().take(140).collect::<String>());
            }
        }
        let nodes = parse_subscription(&content, Some("test"));
        eprintln!(
            "uris={} parsed_raw={} deduped={} fail_ss={} fail_vmess={} fail_vless={} fail_trojan={} fail_other={}",
            uris.len(),
            parsed,
            nodes.len(),
            fail_ss,
            fail_vmess,
            fail_vless,
            fail_trojan,
            fail_other
        );
        for s in fail_samples {
            eprintln!("FAIL {s}");
        }
        assert!(parsed > 7000, "raw parse too low: {parsed}");
        assert!(nodes.len() > 3500, "deduped too low: {}", nodes.len());
    }
}

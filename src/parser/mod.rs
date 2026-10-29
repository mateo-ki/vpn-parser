mod clash;
mod common;
pub mod openrung;
mod uri;
pub mod warp_masque;

pub use clash::{
    extract_clash_groups_yaml, extract_clash_rules, extract_clash_rules_yaml, parse_clash_yaml,
};
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

/// Extract raw rules and proxy-groups YAML from subscription content (plain or base64).
pub fn extract_subscription_rules_and_groups(
    content: &str,
) -> (Option<String>, Option<String>) {
    let content = normalize_subscription_text(content);
    let content = content.trim();
    if content.is_empty() {
        return (None, None);
    }
    if looks_like_clash(content) {
        let rules = extract_clash_rules_yaml(content);
        let groups = extract_clash_groups_yaml(content);
        if rules.is_some() || groups.is_some() {
            return (rules, groups);
        }
    }
    if let Some(decoded) = decode_maybe_base64(content) {
        let decoded = normalize_subscription_text(&decoded);
        if looks_like_clash(&decoded) {
            let rules = extract_clash_rules_yaml(&decoded);
            let groups = extract_clash_groups_yaml(&decoded);
            if rules.is_some() || groups.is_some() {
                return (rules, groups);
            }
        }
    }
    (None, None)
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
        country: guess_country_with_server(name, Some(server)),
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

/// 从节点名称推断国家/地区两字符代码（ISO 3166-1 alpha-2，如 "US", "HK", "JP"）。
pub fn guess_country(name: &str) -> Option<String> {
    guess_country_with_server(name, None)
}

/// 从节点名称以及可选的服务器地址综合推断国家/地区。
pub fn guess_country_with_server(name: &str, server: Option<&str>) -> Option<String> {
    let s = name.trim();

    // 1. 优先提取 Emoji 旗帜 (Regional Indicator Symbol 符号对，100% 精准无歧义)
    // 🇦=U+1F1E6, 🇿=U+1F1FF
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len().saturating_sub(1) {
        let c1 = chars[i];
        let c2 = chars[i + 1];
        if ('\u{1F1E6}'..='\u{1F1FF}').contains(&c1) && ('\u{1F1E6}'..='\u{1F1FF}').contains(&c2) {
            let b1 = ((c1 as u32 - 0x1F1E6) + ('A' as u32)) as u8;
            let b2 = ((c2 as u32 - 0x1F1E6) + ('A' as u32)) as u8;
            if b1.is_ascii_uppercase() && b2.is_ascii_uppercase() {
                return Some(String::from_utf8(vec![b1, b2]).unwrap());
            }
        }
    }

    // 2. 中文地名与城市匹配（先于短 ASCII 代码，避免像「洛杉矶 Trojan」被 trojan 中的 tr 抢先误判）
    let zh_map = [
        ("香港", "HK"),
        ("台湾", "TW"),
        ("臺灣", "TW"),
        ("台北", "TW"),
        ("臺北", "TW"),
        ("高雄", "TW"),
        ("日本", "JP"),
        ("东京", "JP"),
        ("東京", "JP"),
        ("大阪", "JP"),
        ("名古屋", "JP"),
        ("福冈", "JP"),
        ("福岡", "JP"),
        ("横滨", "JP"),
        ("新加坡", "SG"),
        ("狮城", "SG"),
        ("獅城", "SG"),
        ("美国", "US"),
        ("美國", "US"),
        ("洛杉矶", "US"),
        ("洛杉磯", "US"),
        ("圣何塞", "US"),
        ("聖何塞", "US"),
        ("硅谷", "US"),
        ("矽谷", "US"),
        ("西雅图", "US"),
        ("西雅圖", "US"),
        ("旧金山", "US"),
        ("舊金山", "US"),
        ("纽约", "US"),
        ("紐約", "US"),
        ("芝加哥", "US"),
        ("达拉斯", "US"),
        ("達拉斯", "US"),
        ("凤凰城", "US"),
        ("邁阿密", "US"),
        ("迈阿密", "US"),
        ("亚特兰大", "US"),
        ("波特兰", "US"),
        ("波士顿", "US"),
        ("弗吉尼亚", "US"),
        ("韩国", "KR"),
        ("韓國", "KR"),
        ("首尔", "KR"),
        ("首爾", "KR"),
        ("仁川", "KR"),
        ("英国", "GB"),
        ("英國", "GB"),
        ("伦敦", "GB"),
        ("倫敦", "GB"),
        ("曼彻斯特", "GB"),
        ("德国", "DE"),
        ("德國", "DE"),
        ("法兰克福", "DE"),
        ("法蘭克福", "DE"),
        ("柏林", "DE"),
        ("法国", "FR"),
        ("法國", "FR"),
        ("巴黎", "FR"),
        ("加拿大", "CA"),
        ("多伦多", "CA"),
        ("温哥华", "CA"),
        ("溫哥華", "CA"),
        ("澳洲", "AU"),
        ("澳大利亚", "AU"),
        ("澳大利亞", "AU"),
        ("悉尼", "AU"),
        ("墨尔本", "AU"),
        ("俄罗斯", "RU"),
        ("俄羅斯", "RU"),
        ("莫斯科", "RU"),
        ("印度", "IN"),
        ("孟买", "IN"),
        ("新德里", "IN"),
        ("土耳其", "TR"),
        ("伊斯坦布尔", "TR"),
        ("荷兰", "NL"),
        ("荷蘭", "NL"),
        ("阿姆斯特丹", "NL"),
        ("马来西亚", "MY"),
        ("吉隆坡", "MY"),
        ("泰国", "TH"),
        ("曼谷", "TH"),
        ("越南", "VN"),
        ("菲律宾", "PH"),
        ("印尼", "ID"),
    ];
    for (k, v) in zh_map {
        if s.contains(k) {
            return Some(v.to_string());
        }
    }

    let n = s.to_ascii_lowercase();

    // 3. 完整英文单词及城市名（至少 3 字符以上的明确单词）
    let full_words = [
        ("hongkong", "HK"),
        ("hong kong", "HK"),
        ("taiwan", "TW"),
        ("taipei", "TW"),
        ("japan", "JP"),
        ("tokyo", "JP"),
        ("osaka", "JP"),
        ("singapore", "SG"),
        ("united states", "US"),
        ("unitedstates", "US"),
        ("america", "US"),
        ("losangeles", "US"),
        ("los angeles", "US"),
        ("san jose", "US"),
        ("sanjose", "US"),
        ("seattle", "US"),
        ("new york", "US"),
        ("newyork", "US"),
        ("chicago", "US"),
        ("silicon valley", "US"),
        ("siliconvalley", "US"),
        ("fremont", "US"),
        ("dallas", "US"),
        ("phoenix", "US"),
        ("atlanta", "US"),
        ("miami", "US"),
        ("ashburn", "US"),
        ("korea", "KR"),
        ("seoul", "KR"),
        ("incheon", "KR"),
        ("united kingdom", "GB"),
        ("england", "GB"),
        ("london", "GB"),
        ("manchester", "GB"),
        ("germany", "DE"),
        ("frankfurt", "DE"),
        ("berlin", "DE"),
        ("france", "FR"),
        ("paris", "FR"),
        ("canada", "CA"),
        ("toronto", "CA"),
        ("vancouver", "CA"),
        ("montreal", "CA"),
        ("australia", "AU"),
        ("sydney", "AU"),
        ("melbourne", "AU"),
        ("russia", "RU"),
        ("moscow", "RU"),
        ("india", "IN"),
        ("mumbai", "IN"),
        ("delhi", "IN"),
        ("turkey", "TR"),
        ("istanbul", "TR"),
        ("netherlands", "NL"),
        ("amsterdam", "NL"),
        ("malaysia", "MY"),
        ("thailand", "TH"),
        ("bangkok", "TH"),
        ("vietnam", "VN"),
        ("philippines", "PH"),
        ("indonesia", "ID"),
    ];
    for (k, v) in full_words {
        if n.contains(k) {
            return Some(v.to_string());
        }
    }

    // 4. 常见机场与枢纽代码（严格要求词边界，如 lax, sjc, sfo, hkg, nrt）
    let airport_codes = [
        ("lax", "US"),
        ("sjc", "US"),
        ("sfo", "US"),
        ("sea", "US"),
        ("jfk", "US"),
        ("ord", "US"),
        ("dfw", "US"),
        ("iad", "US"),
        ("ewr", "US"),
        ("atl", "US"),
        ("mia", "US"),
        ("hkg", "HK"),
        ("nrt", "JP"),
        ("hnd", "JP"),
        ("kix", "JP"),
        ("tpe", "TW"),
        ("khh", "TW"),
        ("sin", "SG"),
        ("icn", "KR"),
        ("lhr", "GB"),
        ("fra", "DE"),
        ("ams", "NL"),
    ];
    for (k, v) in airport_codes {
        if has_isolated_token(&n, k) {
            return Some(v.to_string());
        }
    }

    // 5. 2 字符/3 字符国家地区简写（必须严格具备词边界，禁止作为其他单词子串匹配！
    // 彻底根除 trojan->tr(土耳其), node->de(德国), china->in(印度), auto->au, location->ca 等低级误判）
    let iso_tokens = [
        ("usa", "US"),
        ("us", "US"),
        ("hkg", "HK"),
        ("hk", "HK"),
        ("jpn", "JP"),
        ("jp", "JP"),
        ("sgp", "SG"),
        ("sg", "SG"),
        ("twn", "TW"),
        ("tw", "TW"),
        ("kor", "KR"),
        ("kr", "KR"),
        ("gbr", "GB"),
        ("gb", "GB"),
        ("uk", "GB"),
        ("deu", "DE"),
        ("de", "DE"),
        ("fra", "FR"),
        ("fr", "FR"),
        ("can", "CA"),
        ("ca", "CA"),
        ("aus", "AU"),
        ("au", "AU"),
        ("rus", "RU"),
        ("ru", "RU"),
        ("ind", "IN"),
        ("tur", "TR"),
        ("tr", "TR"),
        ("nld", "NL"),
        ("nl", "NL"),
        ("mys", "MY"),
        ("tha", "TH"),
        ("th", "TH"),
        ("vnm", "VN"),
        ("vn", "VN"),
        ("phl", "PH"),
        ("ph", "PH"),
        ("idn", "ID"),
        ("id", "ID"),
    ];
    for (k, v) in iso_tokens {
        if has_isolated_token(&n, k) {
            return Some(v.to_string());
        }
    }

    // 对于常见英文介词/代词重合的两字母代码（如 my -> 我的，in -> 在...中），
    // 仅在带有定界符/前后缀（如 [MY], MY-01, MY_01, [IN], IN-01）或后接数字时才采纳，避免 "My Node" 误判为马来西亚
    for (special_code, country) in [("my", "MY"), ("in", "IN")] {
        if has_bracketed_or_prefixed(&n, special_code) {
            return Some(country.to_string());
        }
    }

    // 6. 若从名称中无法识别，尝试根据服务器域名后缀或关键域名特征推断
    server.and_then(guess_country_from_server)
}

/// 检查 token 是否作为独立单词/符号分隔词出现（前后不能紧挨字母）
fn has_isolated_token(text: &str, token: &str) -> bool {
    let t = text.as_bytes();
    let tok = token.as_bytes();
    if tok.is_empty() || t.len() < tok.len() {
        return false;
    }
    let mut idx = 0;
    while idx + tok.len() <= t.len() {
        if &t[idx..idx + tok.len()] == tok {
            let before_ok = idx == 0 || !t[idx - 1].is_ascii_alphabetic();
            let after_idx = idx + tok.len();
            let after_ok = after_idx == t.len() || !t[after_idx].is_ascii_alphabetic();
            if before_ok && after_ok {
                return true;
            }
        }
        idx += 1;
    }
    false
}

/// 检查类似 MY/IN 等高歧义二字代码是否带有括号、连字符、下划线定界或后接数字，避免匹配普通英文单词
fn has_bracketed_or_prefixed(text: &str, tok: &str) -> bool {
    let t = text.as_bytes();
    let tok_bytes = tok.as_bytes();
    if tok_bytes.is_empty() || t.len() < tok_bytes.len() {
        return false;
    }
    let mut idx = 0;
    while idx + tok_bytes.len() <= t.len() {
        if &t[idx..idx + tok_bytes.len()] == tok_bytes {
            let prev = if idx > 0 { Some(t[idx - 1]) } else { None };
            let next = if idx + tok_bytes.len() < t.len() {
                Some(t[idx + tok_bytes.len()])
            } else {
                None
            };
            let prev_bracket = prev.map_or(true, |b| matches!(b, b'[' | b'(' | b'-' | b'_' | b' ' | b'|'));
            let next_is_digit = next.map_or(false, |b| b.is_ascii_digit());
            let next_is_delim = next.map_or(true, |b| matches!(b, b']' | b')' | b'-' | b'_' | b' ' | b'|'));
            let has_strict_delimiter = prev.is_some_and(|b| matches!(b, b'[' | b'(' | b'-' | b'_'))
                || next.is_some_and(|b| matches!(b, b']' | b')' | b'-' | b'_'));

            if prev_bracket && (next_is_digit || has_strict_delimiter) && (next_is_digit || next_is_delim) {
                return true;
            }
        }
        idx += 1;
    }
    false
}

/// 根据 server 域名/主机名推测国家（如 .hk -> HK, .jp -> JP 等）
fn guess_country_from_server(server: &str) -> Option<String> {
    let host = server.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    // 检查国家代码顶级域名
    let tld_map = [
        (".hk", "HK"),
        (".tw", "TW"),
        (".jp", "JP"),
        (".sg", "SG"),
        (".kr", "KR"),
        (".uk", "GB"),
        (".de", "DE"),
        (".fr", "FR"),
        (".ca", "CA"),
        (".au", "AU"),
        (".ru", "RU"),
        (".in", "IN"),
        (".tr", "TR"),
        (".nl", "NL"),
        (".my", "MY"),
        (".th", "TH"),
        (".vn", "VN"),
    ];
    for (suffix, code) in tld_map {
        if host.ends_with(suffix) || host.contains(&format!("{suffix}:")) {
            return Some(code.to_string());
        }
    }

    // 检查常见子域名中包含的地区前缀（如 lax.vmiss.com, sjc.node.com）
    let prefixes = [
        ("lax.", "US"),
        ("sjc.", "US"),
        ("sfo.", "US"),
        ("sea.", "US"),
        ("us-", "US"),
        ("us.", "US"),
        ("hk-", "HK"),
        ("hk.", "HK"),
        ("hkg.", "HK"),
        ("jp-", "JP"),
        ("jp.", "JP"),
        ("tokyo.", "JP"),
        ("sg-", "SG"),
        ("sg.", "SG"),
        ("sin.", "SG"),
    ];
    for (prefix, code) in prefixes {
        if host.starts_with(prefix)
            || host.contains(&format!(".{prefix}"))
            || host.contains(&format!("-{prefix}"))
        {
            return Some(code.to_string());
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
    Regex::new(r"(?i)\b(?:vless|vmess|trojan|ss|ssr|hysteria2?|tuic|wireguard|wg|socks5?)://\S*").unwrap()
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

    let validated_hint = country_hint.filter(|c| {
        if c.eq_ignore_ascii_case("TR") {
            let n_low = name.to_ascii_lowercase();
            n_low.contains("turkey")
                || n_low.contains("istanbul")
                || name.contains("土耳其")
                || name.contains("🇹🇷")
                || has_isolated_token(&n_low, "tr")
        } else {
            true
        }
    });

    let country_code = guess_country_with_server(name, Some(server))
        .or_else(|| guess_country_with_server(&s, Some(server)))
        .or_else(|| validated_hint.map(|c| c.to_string()));

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
    fn guess_country_fixes_vmiss_and_trojan_false_positives() {
        // Emoji 国旗精准提取
        assert_eq!(guess_country("🇺🇸【北美洲】美国04原生丨直连【2x】"), Some("US".into()));
        assert_eq!(guess_country("🇭🇰 香港 BGP 01"), Some("HK".into()));
        assert_eq!(guess_country("🇯🇵 东京专线"), Some("JP".into()));
        assert_eq!(guess_country("🇸🇬 新加坡优化"), Some("SG".into()));

        // 中文城市名
        assert_eq!(guess_country("VMiss 洛杉矶 Trojan"), Some("US".into()));
        assert_eq!(guess_country("VMiss 圣何塞 优化线路"), Some("US".into()));
        assert_eq!(guess_country("VMiss 西雅图 CN2"), Some("US".into()));

        // 英文全词与机场枢纽代码
        assert_eq!(guess_country("VMiss - US - Trojan"), Some("US".into()));
        assert_eq!(guess_country("VMiss LAX Trojan"), Some("US".into()));
        assert_eq!(guess_country("VMiss SJC Trojan"), Some("US".into()));
        assert_eq!(guess_country("VMiss - San Jose - Trojan"), Some("US".into()));
        assert_eq!(guess_country("VMiss-Seattle-Trojan"), Some("US".into()));

        // 杜绝单词子串误判：trojan 中的 tr 绝不能误判为土耳其 TR！
        assert_eq!(guess_country("VMiss-Trojan"), None);
        assert_eq!(guess_country("Node 01"), None); // de 绝不能误判为德国
        assert_eq!(guess_country("China Telecom"), None); // in 绝不能误判为印度
        assert_eq!(guess_country("VMiss Ultra"), None); // tr 绝不能误判为土耳其
        assert_eq!(guess_country("Dedicated 01"), None);

        // 真实的土耳其节点必须识别
        assert_eq!(guess_country("VMiss - TR - 01"), Some("TR".into()));
        assert_eq!(guess_country("土耳其 01"), Some("TR".into()));
        assert_eq!(guess_country("🇹🇷 伊斯坦布尔"), Some("TR".into()));

        // 结合 server 推断
        assert_eq!(
            guess_country_with_server("VMiss-Trojan", Some("lax.vmiss.com")),
            Some("US".into())
        );
        assert_eq!(
            guess_country_with_server("My Node", Some("sjc.example.net")),
            Some("US".into())
        );
        assert_eq!(
            guess_country_with_server("VPS Server", Some("node01.hk")),
            Some("HK".into())
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

    #[test]
    fn test_extract_subscription_rules_and_groups() {
        let clash_text = r#"
port: 7890
proxy-groups:
  - name: "HK-Auto"
    type: url-test
    proxies: ["HK 01"]
rules:
  - "DOMAIN,example.com,DIRECT"
proxies:
  - name: "HK 01"
    type: ss
    server: 1.1.1.1
    port: 443
    cipher: aes-256-gcm
    password: pass
"#;
        let (rules, groups) = extract_subscription_rules_and_groups(clash_text);
        assert!(rules.is_some());
        assert!(rules.unwrap().contains("DOMAIN,example.com,DIRECT"));
        assert!(groups.is_some());
        assert!(groups.unwrap().contains("HK-Auto"));

        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(clash_text.as_bytes());
        let (b64_rules, b64_groups) = extract_subscription_rules_and_groups(&b64);
        assert!(b64_rules.is_some());
        assert!(b64_groups.is_some());
    }
}

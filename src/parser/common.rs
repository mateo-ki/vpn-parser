use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::Local;
use once_cell::sync::Lazy;
use regex::Regex;

static URI_RE: Lazy<Regex> = Lazy::new(|| {
    // Match common proxy share links in free text.
    Regex::new(r"(?i)\b((?:ss|ssr|shadowsocksr|vmess|vless|trojan|tuic|hysteria|hysteria2|hy2|wireguard|wg|socks5|socks|http|https|masque)://[^\s<>'\]\)]+)")
        .expect("uri regex")
});

static LINE_URI_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(ss|ssr|shadowsocksr|vmess|vless|trojan|tuic|hysteria|hysteria2|hy2|wireguard|wg|socks5|socks|masque)://\S+")
        .expect("line uri regex")
});

/// Expand date placeholders in subscription URL templates.
/// Supported: {Ymd} {ymd} {Y} {m} {mm} {d} {dd}
pub fn expand_url_template(template: &str) -> String {
    let now = Local::now();
    let y = now.format("%Y").to_string();
    let m = now.format("%m").to_string();
    let d = now.format("%d").to_string();
    let ymd = format!("{y}{m}{d}");
    let ymd_short = format!("{}{}{}", &y[2..], m, d);
    let m_no0 = m.trim_start_matches('0');
    let d_no0 = d.trim_start_matches('0');
    let m_short = if m_no0.is_empty() { "0" } else { m_no0 };
    let d_short = if d_no0.is_empty() { "0" } else { d_no0 };

    template
        .replace("{Ymd}", &ymd)
        .replace("{ymd}", &ymd_short)
        .replace("{Y}", &y)
        .replace("{mm}", &m)
        .replace("{m}", m_short)
        .replace("{dd}", &d)
        .replace("{d}", d_short)
}

/// Normalize free subscription text / URI before parsing.
pub fn normalize_subscription_text(input: &str) -> String {
    let mut s = input.replace("\r\n", "\n").replace('\r', "\n");
    // Common HTML entities found in scraped subscription dumps.
    s = s
        .replace("&amp;", "&")
        .replace("&amp", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    s
}

pub fn normalize_uri(uri: &str) -> String {
    let mut s = uri.trim().to_string();
    s = s
        .replace("&amp;", "&")
        .replace("&amp", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    // Strip surrounding wrappers sometimes left by markdown.
    while s.starts_with('<') && s.ends_with('>') && s.len() > 2 {
        s = s[1..s.len() - 1].to_string();
    }
    trim_trailing_punct(s)
}

/// Try decode base64 (standard / url-safe, padded / unpadded).
pub fn decode_maybe_base64(input: &str) -> Option<String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() < 16 {
        return None;
    }
    let ok_ratio = cleaned
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_' | '='))
        .count() as f32
        / cleaned.len() as f32;
    if ok_ratio < 0.95 {
        return None;
    }

    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(&cleaned) {
            if let Ok(s) = String::from_utf8(bytes) {
                let printable = s
                    .chars()
                    .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t');
                if printable {
                    return Some(s);
                }
            }
        }
    }
    None
}

pub fn decode_base64_str(input: &str) -> Option<String> {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    // Pad to multiple of 4 for engines that need it.
    let mut padded = cleaned.clone();
    while padded.len() % 4 != 0 {
        padded.push('=');
    }
    for candidate in [&cleaned, &padded] {
        for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
            if let Ok(bytes) = engine.decode(candidate) {
                if let Ok(s) = String::from_utf8(bytes) {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn trim_trailing_punct(mut uri: String) -> String {
    while uri
        .chars()
        .last()
        .map(|c| matches!(c, ')' | ']' | '>' | ',' | ';' | '.' | '"' | '\''))
        .unwrap_or(false)
    {
        uri.pop();
    }
    uri
}

/// Extract share-link URIs from free text.
pub fn extract_uris(content: &str) -> Vec<String> {
    let content = normalize_subscription_text(content);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Prefer whole-line capture for pure subscription dumps.
        if let Some(m) = LINE_URI_RE.find(line) {
            let uri = normalize_uri(m.as_str());
            if seen.insert(uri.clone()) {
                out.push(uri);
            }
            continue;
        }

        for cap in URI_RE.captures_iter(line) {
            if let Some(m) = cap.get(1) {
                let uri = normalize_uri(m.as_str());
                if seen.insert(uri.clone()) {
                    out.push(uri);
                }
            }
        }
    }

    if out.is_empty() {
        for cap in URI_RE.captures_iter(&content) {
            if let Some(m) = cap.get(1) {
                let uri = normalize_uri(m.as_str());
                if seen.insert(uri.clone()) {
                    out.push(uri);
                }
            }
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .to_string()
}

/// Strip query/fragment from host:port section.
pub fn strip_query_fragment(s: &str) -> &str {
    s.split(['?', '#']).next().unwrap_or(s)
}

pub fn split_userinfo_hostport(s: &str) -> Option<(String, String, u16)> {
    let s = strip_query_fragment(s);
    let (userinfo, hostport) = s.rsplit_once('@')?;
    let (host, port_s) = split_host_port(hostport)?;
    let port: u16 = port_s.parse().ok()?;
    Some((userinfo.to_string(), host, port))
}

pub fn split_host_port(s: &str) -> Option<(String, String)> {
    let s = strip_query_fragment(s);
    if let Some(rest) = s.strip_prefix('[') {
        let (host, rest) = rest.split_once(']')?;
        let port = rest.strip_prefix(':')?.to_string();
        return Some((host.to_string(), port));
    }
    let (host, port) = s.rsplit_once(':')?;
    Some((host.to_string(), port.to_string()))
}

pub fn parse_query(q: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let mut it = pair.splitn(2, '=');
        let k = it.next().unwrap_or("").to_string();
        let v = percent_decode(it.next().unwrap_or(""));
        if !k.is_empty() {
            map.insert(k, v);
        }
    }
    map
}

/// Manual authority parse when Url::parse fails (emoji / odd fragments).
pub fn parse_authority(uri: &str) -> Option<(String, String, u16, String, String, String)> {
    // returns: scheme, userinfo, port, host, query, fragment
    let uri = normalize_uri(uri);
    let (scheme, rest) = uri.split_once("://")?;
    let (main, fragment) = match rest.split_once('#') {
        Some((a, b)) => (a, percent_decode(b)),
        None => (rest, String::new()),
    };
    let (main, query) = match main.split_once('?') {
        Some((a, b)) => (a, b.to_string()),
        None => (main, String::new()),
    };
    let (userinfo, hostport) = main.rsplit_once('@')?;
    let (host, port_s) = split_host_port(hostport)?;
    let port: u16 = port_s.parse().ok()?;
    Some((
        scheme.to_ascii_lowercase(),
        percent_decode(userinfo),
        port,
        host,
        query,
        fragment,
    ))
}

pub fn looks_like_uuid(s: &str) -> bool {
    let s = s.trim();
    if s.len() != 36 {
        return false;
    }
    let b = s.as_bytes();
    // 8-4-4-4-12
    b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

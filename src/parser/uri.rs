use super::common::{
    decode_base64_str, looks_like_uuid, normalize_uri, parse_authority, parse_query,
    percent_decode, split_host_port, split_userinfo_hostport, strip_query_fragment,
};
use super::make_node;
use crate::models::Node;
use serde_json::json;
use url::Url;

pub fn parse_uri(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let uri = normalize_uri(uri);
    let lower = uri.to_ascii_lowercase();

    // Some subscription dumps mislabel VLESS/VMess links as ss://
    if lower.starts_with("ss://") {
        if let Some(node) = try_parse_mislabeled_ss_as_vless(&uri, source_id) {
            return Some(node);
        }
        if let Some(node) = try_parse_mislabeled_ss_as_vmess(&uri, source_id) {
            return Some(node);
        }
        return parse_ss(&uri, source_id);
    }
    if lower.starts_with("vmess://") {
        return parse_vmess(&uri, source_id);
    }
    if lower.starts_with("vless://") {
        return parse_vless(&uri, source_id);
    }
    if lower.starts_with("trojan://") {
        return parse_trojan(&uri, source_id);
    }
    if lower.starts_with("hysteria2://") || lower.starts_with("hy2://") {
        return parse_hysteria2(&uri, source_id);
    }
    if lower.starts_with("socks5://") || lower.starts_with("socks://") {
        return parse_socks(&uri, source_id);
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        // Only treat as proxy URI when it has userinfo (proxy auth style)
        if let Ok(u) = Url::parse(&uri) {
            if u.username().is_empty() {
                return None;
            }
            return parse_http_proxy(&uri, source_id);
        }
    }
    None
}

/// ss://uuid@host:port?type=ws&security=tls... is often a mislabeled vless link.
fn try_parse_mislabeled_ss_as_vless(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let rest = uri.trim_start_matches("ss://").trim_start_matches("SS://");
    let (body, _) = split_fragment(rest);
    if !body.contains('@') || !body.contains('?') {
        return None;
    }
    let (userinfo, hostport) = body.split_once('@')?;
    let userinfo = strip_query_fragment(userinfo);
    // UUID-like, or plain user with vless-style query.
    let q = body.split_once('?').map(|(_, q)| q).unwrap_or("");
    let ql = q.to_ascii_lowercase();
    let vless_like = ql.contains("security=")
        || ql.contains("encryption=")
        || ql.contains("type=")
        || ql.contains("sni=")
        || ql.contains("path=");
    if !vless_like {
        return None;
    }
    if !looks_like_uuid(userinfo) && !userinfo.contains(':') {
        // username@host with vless query params (common mislabel)
    } else if !looks_like_uuid(userinfo) {
        return None;
    }
    let _ = split_host_port(hostport)?;
    let rewritten = format!("vless://{rest}");
    parse_vless(&rewritten, source_id).map(|mut n| {
        n.raw = uri.to_string();
        n
    })
}

/// ss://eyJ... base64 JSON is often a mislabeled vmess payload.
fn try_parse_mislabeled_ss_as_vmess(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let rest = uri.trim_start_matches("ss://").trim_start_matches("SS://");
    let (body, _) = split_fragment(rest);
    // Only attempt when body looks like base64 JSON (starts with eyJ after optional pad).
    let cleaned: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let looks_json_b64 = cleaned.starts_with("eyJ") || cleaned.starts_with("eyj");
    if !looks_json_b64 {
        return None;
    }
    let rewritten = format!("vmess://{cleaned}");
    parse_vmess(&rewritten, source_id).map(|mut n| {
        n.raw = uri.to_string();
        n
    })
}

fn parse_ss(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let rest = uri.trim_start_matches("ss://").trim_start_matches("SS://");
    let (body, fragment) = split_fragment(rest);
    let name = fragment.as_deref().map(percent_decode).unwrap_or_default();

    // SIP002: ss://base64(method:password)@host:port#name
    //          ss://base64(method:password)@host:port/?plugin=...#name
    // Legacy: ss://base64(method:password@host:port)#name
    if body.contains('@') {
        let (userinfo, host, port) = split_userinfo_hostport(body)?;
        let userinfo_decoded = decode_base64_str(&userinfo).unwrap_or_else(|| userinfo.clone());
        let (method, password) = if let Some(pair) = userinfo_decoded.split_once(':') {
            pair
        } else {
            // userinfo is raw password without method (rare) — skip
            return None;
        };

        let query = body.split_once('?').map(|(_, q)| q).unwrap_or("");
        let q = parse_query(query);
        let mut config = json!({
            "type": "ss",
            "server": host,
            "port": port,
            "cipher": method,
            "password": password,
            "name": name,
        });
        if let Some(obj) = config.as_object_mut() {
            if let Some(plugin) = q.get("plugin") {
                obj.insert("plugin".into(), json!(plugin));
            }
            if let Some(plugin_opts) = q.get("plugin-opts").or_else(|| q.get("plugin_opts")) {
                obj.insert("plugin-opts".into(), json!(plugin_opts));
            }
        }
        return Some(make_node(
            "shadowsocks",
            &name,
            &host,
            port,
            uri,
            config,
            source_id,
        ));
    }

    // legacy full base64, or mislabeled vmess json base64
    let decoded = decode_base64_str(body)?;
    if decoded.trim_start().starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&decoded) {
            if v.get("add").is_some() || v.get("id").is_some() {
                let rewritten = format!("vmess://{body}");
                return parse_vmess(&rewritten, source_id).map(|mut n| {
                    n.raw = uri.to_string();
                    n
                });
            }
        }
    }
    // method:password@host:port
    let (method_pass, hostport) = decoded.rsplit_once('@')?;
    let (method, password) = method_pass.split_once(':')?;
    let (host, port_s) = split_host_port(hostport)?;
    let port: u16 = port_s.parse().ok()?;
    let config = json!({
        "type": "ss",
        "server": host,
        "port": port,
        "cipher": method,
        "password": password,
        "name": name,
    });
    Some(make_node(
        "shadowsocks",
        &name,
        &host,
        port,
        uri,
        config,
        source_id,
    ))
}

fn parse_vmess(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let rest = uri
        .trim_start_matches("vmess://")
        .trim_start_matches("VMESS://");
    let rest = rest.split(['#', '?']).next().unwrap_or(rest);
    let decoded = decode_base64_str(rest)?;
    let v: serde_json::Value = serde_json::from_str(&decoded).ok()?;
    let server = v
        .get("add")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("server").and_then(|x| x.as_str()))?
        .to_string();
    let port = v
        .get("port")
        .and_then(parse_port_value)
        .or_else(|| v.get("server_port").and_then(parse_port_value))?;
    let name = v
        .get("ps")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("remark").and_then(|x| x.as_str()))
        .unwrap_or("")
        .to_string();
    let mut config = v;
    if let Some(obj) = config.as_object_mut() {
        obj.insert("type".into(), json!("vmess"));
        obj.insert("server".into(), json!(server));
        obj.insert("port".into(), json!(port));
        obj.insert("name".into(), json!(name));
    }
    Some(make_node(
        "vmess", &name, &server, port, uri, config, source_id,
    ))
}

fn parse_vless(uri: &str, source_id: Option<&str>) -> Option<Node> {
    if let Ok(url) = Url::parse(uri) {
        let uuid = percent_decode(url.username());
        if uuid.is_empty() {
            return None;
        }
        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(443);
        let name = url.fragment().map(percent_decode).unwrap_or_default();
        let q = parse_query(url.query().unwrap_or(""));
        return Some(vless_node(uri, source_id, &uuid, &host, port, &name, q));
    }

    // Fallback for odd fragments / encoding that Url rejects.
    let (_scheme, uuid, port, host, query, name) = parse_authority(uri)?;
    if uuid.is_empty() {
        return None;
    }
    let q = parse_query(&query);
    Some(vless_node(uri, source_id, &uuid, &host, port, &name, q))
}

fn vless_node(
    uri: &str,
    source_id: Option<&str>,
    uuid: &str,
    host: &str,
    port: u16,
    name: &str,
    q: std::collections::HashMap<String, String>,
) -> Node {
    let config = json!({
        "type": "vless",
        "server": host,
        "port": port,
        "uuid": uuid,
        "name": name,
        "flow": q.get("flow").cloned().unwrap_or_default(),
        "encryption": q.get("encryption").cloned().unwrap_or_else(|| "none".into()),
        "network": q.get("type").cloned().unwrap_or_else(|| "tcp".into()),
        "security": q.get("security").cloned().unwrap_or_default(),
        "sni": q.get("sni").cloned().unwrap_or_default(),
        "fp": q.get("fp").cloned().unwrap_or_default(),
        "pbk": q.get("pbk").cloned().unwrap_or_default(),
        "sid": q.get("sid").cloned().unwrap_or_default(),
        "path": q.get("path").cloned().unwrap_or_default(),
        "host": q.get("host").cloned().unwrap_or_default(),
        "serviceName": q.get("serviceName").cloned().unwrap_or_default(),
        "alpn": q.get("alpn").cloned().unwrap_or_default(),
        // 查询参数来自订阅链接本身，不能让它关掉证书校验。
        "allowInsecure": json!(false),
    });
    make_node("vless", name, host, port, uri, config, source_id)
}

fn parse_trojan(uri: &str, source_id: Option<&str>) -> Option<Node> {
    if let Ok(url) = Url::parse(uri) {
        let password = percent_decode(url.username());
        if password.is_empty() {
            return None;
        }
        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(443);
        let name = url.fragment().map(percent_decode).unwrap_or_default();
        let q = parse_query(url.query().unwrap_or(""));
        return Some(trojan_node(
            uri, source_id, &password, &host, port, &name, q,
        ));
    }
    let (_scheme, password, port, host, query, name) = parse_authority(uri)?;
    if password.is_empty() {
        return None;
    }
    let q = parse_query(&query);
    Some(trojan_node(
        uri, source_id, &password, &host, port, &name, q,
    ))
}

fn trojan_node(
    uri: &str,
    source_id: Option<&str>,
    password: &str,
    host: &str,
    port: u16,
    name: &str,
    q: std::collections::HashMap<String, String>,
) -> Node {
    let config = json!({
        "type": "trojan",
        "server": host,
        "port": port,
        "password": password,
        "name": name,
        "sni": q.get("sni").cloned().unwrap_or_default(),
        "network": q.get("type").cloned().unwrap_or_else(|| "tcp".into()),
        "security": q.get("security").cloned().unwrap_or_else(|| "tls".into()),
        "fp": q.get("fp").cloned().unwrap_or_default(),
        "alpn": q.get("alpn").cloned().unwrap_or_default(),
        "path": q.get("path").cloned().unwrap_or_default(),
        "host": q.get("host").cloned().unwrap_or_default(),
        // 查询参数来自订阅链接本身，不能让它关掉证书校验。
        "allowInsecure": json!(false),
    });
    make_node("trojan", name, host, port, uri, config, source_id)
}

fn parse_hysteria2(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let normalized =
        uri.replacen("hy2://", "hysteria2://", 1)
            .replacen("HY2://", "hysteria2://", 1);
    if let Ok(url) = Url::parse(&normalized) {
        let auth = percent_decode(url.username());
        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(443);
        let name = url.fragment().map(percent_decode).unwrap_or_default();
        let q = parse_query(url.query().unwrap_or(""));
        let config = json!({
            "type": "hysteria2",
            "server": host,
            "port": port,
            "password": auth,
            "name": name,
            "sni": q.get("sni").cloned().unwrap_or_default(),
            "insecure": json!(false),
            "obfs": q.get("obfs").cloned().unwrap_or_default(),
            "obfs-password": q.get("obfs-password").cloned().unwrap_or_default(),
        });
        return Some(make_node(
            "hysteria2",
            &name,
            &host,
            port,
            uri,
            config,
            source_id,
        ));
    }
    let (_scheme, auth, port, host, query, name) = parse_authority(&normalized)?;
    let q = parse_query(&query);
    let config = json!({
        "type": "hysteria2",
        "server": host,
        "port": port,
        "password": auth,
        "name": name,
        "sni": q.get("sni").cloned().unwrap_or_default(),
        "insecure": json!(false),
        "obfs": q.get("obfs").cloned().unwrap_or_default(),
        "obfs-password": q.get("obfs-password").cloned().unwrap_or_default(),
    });
    Some(make_node(
        "hysteria2",
        &name,
        &host,
        port,
        uri,
        config,
        source_id,
    ))
}

fn parse_socks(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let url = Url::parse(uri).ok()?;
    let host = url.host_str()?.to_string();
    let port = url.port().unwrap_or(1080);
    let name = url.fragment().map(percent_decode).unwrap_or_default();
    let username = url.username().to_string();
    let password = url.password().map(percent_decode).unwrap_or_default();
    let config = json!({
        "type": "socks5",
        "server": host,
        "port": port,
        "username": username,
        "password": password,
        "name": name,
    });
    Some(make_node(
        "socks5", &name, &host, port, uri, config, source_id,
    ))
}

fn parse_http_proxy(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let url = Url::parse(uri).ok()?;
    let host = url.host_str()?.to_string();
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let name = url.fragment().map(percent_decode).unwrap_or_default();
    let username = url.username().to_string();
    let password = url.password().map(percent_decode).unwrap_or_default();
    let config = json!({
        "type": "http",
        "server": host,
        "port": port,
        "username": username,
        "password": password,
        "tls": url.scheme() == "https",
        "name": name,
    });
    Some(make_node(
        "http", &name, &host, port, uri, config, source_id,
    ))
}

fn split_fragment(s: &str) -> (&str, Option<&str>) {
    match s.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    }
}

fn parse_port_value(v: &serde_json::Value) -> Option<u16> {
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).ok();
    }
    if let Some(s) = v.as_str() {
        return s.parse().ok();
    }
    None
}

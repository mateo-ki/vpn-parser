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
    if lower.starts_with("tuic://") {
        return parse_tuic(&uri, source_id);
    }
    if lower.starts_with("ssr://") || lower.starts_with("shadowsocksr://") {
        return parse_ssr(&uri, source_id);
    }
    if lower.starts_with("wireguard://") || lower.starts_with("wg://") {
        return parse_wireguard(&uri, source_id);
    }
    if lower.starts_with("hysteria2://") || lower.starts_with("hy2://") {
        return parse_hysteria2(&uri, source_id);
    }
    if lower.starts_with("hysteria://") {
        return parse_hysteria1(&uri, source_id);
    }
    if lower.starts_with("socks5://") || lower.starts_with("socks://") {
        return parse_socks(&uri, source_id);
    }
    if lower.starts_with("masque://") {
        return super::warp_masque::parse_masque_uri(&uri, source_id);
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

fn parse_tuic(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let (mut uuid, mut password, host, port, name, q) = if let Ok(url) = Url::parse(uri) {
        let u = percent_decode(url.username());
        let p = url.password().map(percent_decode).unwrap_or_default();
        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(443);
        let name = url.fragment().map(percent_decode).unwrap_or_default();
        let q = parse_query(url.query().unwrap_or(""));
        (u, p, host, port, name, q)
    } else {
        let (_scheme, auth, port, host, query, name) = parse_authority(uri)?;
        let q = parse_query(&query);
        let (u, p) = if let Some((u, p)) = auth.split_once(':') {
            (u.to_string(), p.to_string())
        } else {
            (auth, String::new())
        };
        (u, p, host, port, name, q)
    };

    if password.is_empty() {
        if let Some((u, p)) = uuid.split_once(':') {
            let u_str = u.to_string();
            password = p.to_string();
            uuid = u_str;
        } else if let Some(p) = q.get("password").or_else(|| q.get("pass")) {
            password = p.clone();
        }
    }

    let name = if name.is_empty() {
        q.get("remarks")
            .cloned()
            .unwrap_or_else(|| format!("{}:{}", host, port))
    } else {
        name
    };

    let sni = q
        .get("sni")
        .or_else(|| q.get("server_name"))
        .cloned()
        .unwrap_or_else(|| host.clone());

    let alpn = if let Some(a) = q.get("alpn") {
        if a.contains(',') {
            a.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        } else if !a.is_empty() {
            vec![a.clone()]
        } else {
            vec!["h3".to_string()]
        }
    } else {
        vec!["h3".to_string()]
    };

    let congestion_control = q
        .get("congestion_control")
        .or_else(|| q.get("congestion-controller"))
        .or_else(|| q.get("cc"))
        .cloned()
        .unwrap_or_else(|| "bbr".to_string());

    let udp_relay_mode = q
        .get("udp_relay_mode")
        .or_else(|| q.get("udp-relay-mode"))
        .cloned()
        .unwrap_or_else(|| "native".to_string());

    let config = json!({
        "type": "tuic",
        "server": host,
        "port": port,
        "uuid": uuid,
        "password": password,
        "name": name,
        "sni": sni,
        "alpn": alpn,
        "congestion-controller": congestion_control,
        "congestion_control": congestion_control,
        "udp-relay-mode": udp_relay_mode,
        "udp_relay_mode": udp_relay_mode,
        "allowInsecure": json!(false),
        "insecure": json!(false),
    });

    Some(make_node(
        "tuic", &name, &host, port, uri, config, source_id,
    ))
}

fn parse_ssr(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let raw_b64 = uri
        .trim_start_matches("ssr://")
        .trim_start_matches("SSR://")
        .trim_start_matches("shadowsocksr://")
        .trim_start_matches("SHADOWSOCKSR://");
    let (b64_part, _) = split_fragment(raw_b64);
    let decoded = decode_base64_str(b64_part)?;

    let (main_part, query_part) = match decoded.split_once("/?") {
        Some((m, q)) => (m, Some(q)),
        None => match decoded.split_once('?') {
            Some((m, q)) => (m, Some(q)),
            None => (decoded.as_str(), None),
        },
    };

    // main_part: host:port:protocol:method:obfs:base64pass
    // rsplitn(6, ':') handles IPv6 host or colons in host correctly
    let parts: Vec<&str> = main_part.rsplitn(6, ':').collect();
    if parts.len() < 6 {
        return None;
    }
    let base64pass = parts[0];
    let obfs = parts[1].to_string();
    let method = parts[2].to_string();
    let protocol = parts[3].to_string();
    let port: u16 = parts[4].parse().ok()?;
    let host = parts[5].trim_matches('[').trim_matches(']').to_string();

    let password = decode_base64_str(base64pass).unwrap_or_else(|| base64pass.to_string());

    let q = parse_query(query_part.unwrap_or(""));

    let obfs_param = q
        .get("obfsparam")
        .and_then(|p| decode_base64_str(p))
        .unwrap_or_else(|| q.get("obfsparam").cloned().unwrap_or_default());

    let proto_param = q
        .get("protoparam")
        .and_then(|p| decode_base64_str(p))
        .unwrap_or_else(|| q.get("protoparam").cloned().unwrap_or_default());

    let name = q
        .get("remarks")
        .and_then(|r| decode_base64_str(r))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}:{}", host, port));

    let config = json!({
        "type": "ssr",
        "server": host,
        "port": port,
        "cipher": method,
        "password": password,
        "protocol": protocol,
        "protocol-param": proto_param,
        "protocolparam": proto_param,
        "obfs": obfs,
        "obfs-param": obfs_param,
        "obfsparam": obfs_param,
        "name": name,
    });

    Some(make_node(
        "shadowsocksr", &name, &host, port, uri, config, source_id,
    ))
}

fn parse_wireguard(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let normalized = uri
        .replacen("wg://", "wireguard://", 1)
        .replacen("WG://", "wireguard://", 1);
    let (mut private_key, host, port, name, q) = if let Ok(url) = Url::parse(&normalized) {
        let pk = percent_decode(url.username());
        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(51820);
        let name = url.fragment().map(percent_decode).unwrap_or_default();
        let q = parse_query(url.query().unwrap_or(""));
        (pk, host, port, name, q)
    } else {
        let (_scheme, auth, port, host, query, name) = parse_authority(&normalized)?;
        let q = parse_query(&query);
        (auth, host, port, name, q)
    };

    if private_key.is_empty() {
        if let Some(pk) = q
            .get("privatekey")
            .or_else(|| q.get("private-key"))
            .or_else(|| q.get("private_key"))
        {
            private_key = pk.clone();
        }
    }

    let public_key = q
        .get("publickey")
        .or_else(|| q.get("public-key"))
        .or_else(|| q.get("public_key"))
        .or_else(|| q.get("peer_public_key"))
        .or_else(|| q.get("pubkey"))
        .cloned()
        .unwrap_or_default();

    let name = if name.is_empty() {
        q.get("name")
            .or_else(|| q.get("remarks"))
            .cloned()
            .unwrap_or_else(|| format!("{}:{}", host, port))
    } else {
        name
    };

    let mut address_list = Vec::new();
    if let Some(addr_str) = q
        .get("address")
        .or_else(|| q.get("ip"))
        .or_else(|| q.get("local_address"))
        .or_else(|| q.get("local-address"))
    {
        for item in addr_str.split(',') {
            let trimmed = item.trim();
            if !trimmed.is_empty() {
                address_list.push(trimmed.to_string());
            }
        }
    }
    if address_list.is_empty() {
        address_list.push("172.16.0.2/32".to_string());
    }

    let pre_shared_key = q
        .get("presharedkey")
        .or_else(|| q.get("pre-shared-key"))
        .or_else(|| q.get("pre_shared_key"))
        .or_else(|| q.get("psk"))
        .cloned();

    let reserved: Option<Vec<u64>> = q.get("reserved").and_then(|res_str| {
        let items: Vec<u64> = res_str
            .split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .collect();
        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    });

    let mtu: Option<u64> = q.get("mtu").and_then(|m| m.parse::<u64>().ok());

    let mut config = json!({
        "type": "wireguard",
        "server": host,
        "port": port,
        "name": name,
        "private-key": private_key,
        "public-key": public_key,
        "ip": address_list,
        "address": address_list,
    });

    if let Some(obj) = config.as_object_mut() {
        if let Some(psk) = pre_shared_key {
            obj.insert("pre-shared-key".into(), json!(psk));
        }
        if let Some(res) = reserved {
            obj.insert("reserved".into(), json!(res));
        }
        if let Some(m) = mtu {
            obj.insert("mtu".into(), json!(m));
        }
    }

    Some(make_node(
        "wireguard", &name, &host, port, uri, config, source_id,
    ))
}

fn parse_hysteria1(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let (mut auth, host, port, name, q) = if let Ok(url) = Url::parse(uri) {
        let auth = percent_decode(url.username());
        let host = url.host_str()?.to_string();
        let port = url.port().unwrap_or(443);
        let name = url.fragment().map(percent_decode).unwrap_or_default();
        let q = parse_query(url.query().unwrap_or(""));
        (auth, host, port, name, q)
    } else {
        let (_scheme, auth, port, host, query, name) = parse_authority(uri)?;
        let q = parse_query(&query);
        (auth, host, port, name, q)
    };

    if auth.is_empty() {
        if let Some(a) = q
            .get("auth")
            .or_else(|| q.get("auth_str"))
            .or_else(|| q.get("auth-str"))
        {
            auth = a.clone();
        }
    }

    let name = if name.is_empty() {
        q.get("name")
            .or_else(|| q.get("remarks"))
            .cloned()
            .unwrap_or_else(|| format!("{}:{}", host, port))
    } else {
        name
    };

    let peer = q
        .get("peer")
        .or_else(|| q.get("sni"))
        .or_else(|| q.get("server_name"))
        .cloned()
        .unwrap_or_else(|| host.clone());

    let up = q
        .get("upmbps")
        .or_else(|| q.get("up_mbps"))
        .or_else(|| q.get("up"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(100);

    let down = q
        .get("downmbps")
        .or_else(|| q.get("down_mbps"))
        .or_else(|| q.get("down"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(500);

    let obfs = q.get("obfs").cloned().unwrap_or_default();
    let protocol = q
        .get("protocol")
        .cloned()
        .unwrap_or_else(|| "udp".to_string());

    let insecure = q
        .get("insecure")
        .or_else(|| q.get("allowInsecure"))
        .or_else(|| q.get("allow_insecure"))
        .or_else(|| q.get("skip-cert-verify"))
        .or_else(|| q.get("skip_cert_verify"))
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let alpn = if let Some(a) = q.get("alpn") {
        if a.contains(',') {
            a.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        } else if !a.is_empty() {
            vec![a.clone()]
        } else {
            vec!["h3".to_string()]
        }
    } else {
        vec!["h3".to_string()]
    };

    let config = json!({
        "type": "hysteria",
        "server": host,
        "port": port,
        "name": name,
        "auth": auth,
        "auth-str": auth,
        "auth_str": auth,
        "server-name": peer,
        "sni": peer,
        "up": up,
        "down": down,
        "obfs": obfs,
        "protocol": protocol,
        "alpn": alpn,
        "skip-cert-verify": json!(insecure),
        "insecure": json!(insecure),
        "allowInsecure": json!(insecure),
    });

    Some(make_node(
        "hysteria", &name, &host, port, uri, config, source_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    #[test]
    fn parses_tuic_uri() {
        let uri = "tuic://b54d39f7-9204-4680-ba76-2e84c59ff06a:secretpass@tuic.example.com:8443/?sni=tuic.example.com&alpn=h3&congestion_control=bbr#TuicNode";
        let node = parse_uri(uri, None).expect("tuic parse");
        assert_eq!(node.protocol, "tuic");
        assert_eq!(node.name, "TuicNode");
        assert_eq!(node.server, "tuic.example.com");
        assert_eq!(node.port, 8443);
        assert_eq!(
            node.config.get("uuid").and_then(|v| v.as_str()),
            Some("b54d39f7-9204-4680-ba76-2e84c59ff06a")
        );
        assert_eq!(
            node.config.get("password").and_then(|v| v.as_str()),
            Some("secretpass")
        );
        assert_eq!(
            node.config.get("congestion-controller").and_then(|v| v.as_str()),
            Some("bbr")
        );
    }

    #[test]
    fn parses_ssr_uri() {
        // 1.2.3.4:8388:auth_aes128_md5:aes-128-cfb:tls1.2_ticket_auth:cGFzc3dvcmQ/?remarks=U1NSTm9kZQ
        // remarks is base64("SSRNode") = U1NSTm9kZQ
        // password is base64("password") = cGFzc3dvcmQ
        let raw_payload = "1.2.3.4:8388:auth_aes128_md5:aes-128-cfb:tls1.2_ticket_auth:cGFzc3dvcmQ/?remarks=U1NSTm9kZQ";
        let b64 = URL_SAFE_NO_PAD.encode(raw_payload.as_bytes());
        let uri = format!("ssr://{}", b64);

        let node = parse_uri(&uri, None).expect("ssr parse");
        assert_eq!(node.protocol, "shadowsocksr");
        assert_eq!(node.name, "SSRNode");
        assert_eq!(node.server, "1.2.3.4");
        assert_eq!(node.port, 8388);
        assert_eq!(
            node.config.get("cipher").and_then(|v| v.as_str()),
            Some("aes-128-cfb")
        );
        assert_eq!(
            node.config.get("password").and_then(|v| v.as_str()),
            Some("password")
        );
        assert_eq!(
            node.config.get("protocol").and_then(|v| v.as_str()),
            Some("auth_aes128_md5")
        );
        assert_eq!(
            node.config.get("obfs").and_then(|v| v.as_str()),
            Some("tls1.2_ticket_auth")
        );
    }

    #[test]
    fn parses_wireguard_uri() {
        let uri = "wireguard://aW5pdGlhbF9wcml2YXRlX2tleQ==@wg.example.com:51820/?publickey=cHVibGljX2tleQ==&address=10.0.0.2/32&reserved=1,2,3&mtu=1420#WireGuardNode";
        let node = parse_uri(uri, None).expect("wireguard parse");
        assert_eq!(node.protocol, "wireguard");
        assert_eq!(node.name, "WireGuardNode");
        assert_eq!(node.server, "wg.example.com");
        assert_eq!(node.port, 51820);
        assert_eq!(
            node.config.get("private-key").and_then(|v| v.as_str()),
            Some("aW5pdGlhbF9wcml2YXRlX2tleQ==")
        );
        assert_eq!(
            node.config.get("public-key").and_then(|v| v.as_str()),
            Some("cHVibGljX2tleQ==")
        );
        let res = node.config.get("reserved").and_then(|v| v.as_array()).expect("reserved array");
        assert_eq!(res.len(), 3);
        assert_eq!(res[0], 1);
    }

    #[test]
    fn parses_hysteria1_uri() {
        let uri = "hysteria://hy.example.com:443/?auth=mysecrettoken&peer=hy.example.com&upmbps=100&downmbps=500&obfs=myobfs&insecure=1#Hysteria1Node";
        let node = parse_uri(uri, None).expect("hysteria1 parse");
        assert_eq!(node.protocol, "hysteria");
        assert_eq!(node.name, "Hysteria1Node");
        assert_eq!(node.server, "hy.example.com");
        assert_eq!(node.port, 443);
        assert_eq!(
            node.config.get("auth").and_then(|v| v.as_str()),
            Some("mysecrettoken")
        );
        assert_eq!(
            node.config.get("server-name").and_then(|v| v.as_str()),
            Some("hy.example.com")
        );
        assert_eq!(node.config.get("up").and_then(|v| v.as_u64()), Some(100));
        assert_eq!(node.config.get("down").and_then(|v| v.as_u64()), Some(500));
        assert_eq!(node.config.get("skip-cert-verify"), Some(&json!(true)));
        assert_eq!(node.config.get("alpn"), Some(&json!(["h3"])));
    }
}

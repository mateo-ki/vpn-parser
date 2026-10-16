use crate::models::Node;
use crate::parser::make_node;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

/// 经真机握手实测筛选的 Cloudflare WARP (MASQUE) IPv4 接入点
pub const WARP_MASQUE_V4: &[&str] = &[
    "162.159.198.1",
    "162.159.198.2",
    "162.159.199.1",
    "162.159.199.2",
];

/// 经真机握手实测筛选的 Cloudflare WARP (MASQUE) IPv6 接入点
pub const WARP_MASQUE_V6: &[&str] = &[
    "2606:4700:103::1",
    "2606:4700:103::2",
    "2606:4700:104::1",
    "2606:4700:104::2",
];

/// 经实测全部可通的端口列表
pub const WARP_MASQUE_PORTS: &[u16] = &[443, 500, 1701, 4500, 4443, 8443, 8095];

/// Cloudflare WARP MASQUE 官方 SNI 域名及接入节点
pub const OFFICIAL_SNI: &str = "zt-masque.cloudflareclient.com";
pub const SNI_NODE_IP: &str = "162.159.198.1";
pub const SNI_NODE_PORT: u16 = 443;

/// 剥离 PEM 格式的头部与尾部，保留纯 base64 字符串
pub fn pem_to_b64(pem: &str) -> String {
    pem.lines()
        .map(str::trim)
        .filter(|ln| !ln.is_empty() && !ln.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("")
}

/// 格式化节点名称（对齐 byJoey/warp-masque-actions 规则）
pub fn node_name(ip: &str, port: u16) -> String {
    if ip.contains(':') {
        let segs: Vec<&str> = ip.split(':').collect();
        let seg = segs.get(2).copied().unwrap_or("0");
        let tail = segs.last().copied().unwrap_or("1");
        format!("WARP6-{seg}-{tail}-{port}")
    } else {
        let segs: Vec<&str> = ip.split('.').collect();
        let tail = if segs.len() >= 4 {
            format!("{}.{}", segs[2], segs[3])
        } else {
            ip.to_string()
        };
        format!("WARP-{tail}-{port}")
    }
}

/// 构建 Shadowrocket 兼容的 masque:// 链接
pub fn build_masque_uri(
    ip: &str,
    port: u16,
    pub_key: &str,
    priv_key: &str,
    v4_addr: &str,
    name: &str,
) -> String {
    let host = if ip.contains(':') {
        format!("[{ip}]")
    } else {
        ip.to_string()
    };
    let enc_pub = utf8_percent_encode(pub_key, NON_ALPHANUMERIC).to_string();
    let enc_priv = utf8_percent_encode(priv_key, NON_ALPHANUMERIC).to_string();
    let enc_ip = utf8_percent_encode(v4_addr, NON_ALPHANUMERIC).to_string();
    let enc_name = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();

    format!(
        "masque://{host}:{port}?publicKey={enc_pub}&privateKey={enc_priv}&ip={enc_ip}&dns=1.1.1.1,%208.8.8.8&udp=1&cc=&flag=CDN#{enc_name}"
    )
}

/// 解析 usque-config.json 或 WARP 注册凭据 JSON，生成全部 57 个 WARP MASQUE 节点
pub fn parse_warp_masque_json(content: &str, source_id: Option<&str>) -> Option<Vec<Node>> {
    let val: Value = serde_json::from_str(content).ok()?;
    let obj = val.as_object()?;

    // 提取私钥
    let raw_priv = obj
        .get("private_key")
        .or_else(|| obj.get("privateKey"))
        .or_else(|| obj.get("priv"))
        .and_then(Value::as_str)?;

    let priv_key = if raw_priv.contains("-----") {
        pem_to_b64(raw_priv)
    } else {
        raw_priv.trim().to_string()
    };

    if priv_key.is_empty() {
        return None;
    }

    // 提取对端公钥
    let raw_pub = obj
        .get("endpoint_pub_key")
        .or_else(|| obj.get("peerPublicKey"))
        .or_else(|| obj.get("publicKey"))
        .or_else(|| obj.get("pub_key"))
        .or_else(|| obj.get("pub"))
        .and_then(Value::as_str)?;

    let pub_key = if raw_pub.contains("-----") {
        pem_to_b64(raw_pub)
    } else {
        raw_pub.trim().to_string()
    };

    if pub_key.is_empty() {
        return None;
    }

    // 提取分配给内网网卡的 IPv4 与 IPv6
    let ipv4 = obj
        .get("ipv4")
        .or_else(|| obj.get("ip"))
        .and_then(Value::as_str)
        .unwrap_or("172.16.0.2")
        .trim();

    let ipv6 = obj
        .get("ipv6")
        .and_then(Value::as_str)
        .unwrap_or("2606:4700:110:8878:e7b0:f0eb:a03d:8524")
        .trim();

    let mut nodes = Vec::new();

    // 1) 4 个 IPv4 接入点 x 7 个端口 = 28 个节点
    for &ip in WARP_MASQUE_V4 {
        for &port in WARP_MASQUE_PORTS {
            let name = node_name(ip, port);
            nodes.push(create_masque_node(
                &name, ip, port, &priv_key, &pub_key, ipv4, ipv6, None, source_id,
            ));
        }
    }

    // 2) 4 个 IPv6 接入点 x 7 个端口 = 28 个节点
    for &ip in WARP_MASQUE_V6 {
        for &port in WARP_MASQUE_PORTS {
            let name = node_name(ip, port);
            nodes.push(create_masque_node(
                &name, ip, port, &priv_key, &pub_key, ipv4, ipv6, None, source_id,
            ));
        }
    }

    // 3) 1 个官方 SNI 节点
    let sni_name = "WARP-官方域名".to_string();
    nodes.push(create_masque_node(
        &sni_name,
        SNI_NODE_IP,
        SNI_NODE_PORT,
        &priv_key,
        &pub_key,
        ipv4,
        ipv6,
        Some(OFFICIAL_SNI),
        source_id,
    ));

    Some(nodes)
}

#[allow(clippy::too_many_arguments)]
fn create_masque_node(
    name: &str,
    server: &str,
    port: u16,
    priv_key: &str,
    pub_key: &str,
    ipv4: &str,
    ipv6: &str,
    sni: Option<&str>,
    source_id: Option<&str>,
) -> Node {
    let raw_uri = build_masque_uri(server, port, pub_key, priv_key, ipv4, name);

    let mut config = json!({
        "type": "masque",
        "name": name,
        "server": server,
        "port": port,
        "private-key": priv_key,
        "public-key": pub_key,
        "ip": ipv4,
        "ipv6": ipv6,
        "mtu": 1280,
        "udp": true,
        "remote-dns-resolve": true,
        "dns": ["1.1.1.1", "2606:4700:4700::1111"]
    });

    if let Some(sni_val) = sni {
        config["sni"] = json!(sni_val);
    }

    let mut node = make_node(
        "masque",
        name,
        server,
        port,
        &raw_uri,
        config,
        source_id,
    );
    node.country = Some("US".into());
    node
}

/// 解析单条 `masque://` 链接
pub fn parse_masque_uri(uri: &str, source_id: Option<&str>) -> Option<Node> {
    let lower = uri.trim();
    if !lower.to_ascii_lowercase().starts_with("masque://") {
        return None;
    }

    let rest = &lower["masque://".len()..];
    let (main_part, fragment) = match rest.split_once('#') {
        Some((m, f)) => (m, Some(f)),
        None => (rest, None),
    };

    let (hostport, query) = match main_part.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (main_part, None),
    };

    let (host, port) = parse_host_port(hostport)?;

    let mut pub_key = String::new();
    let mut priv_key = String::new();
    let mut ip = "172.16.0.2".to_string();
    let mut ipv6 = "2606:4700:110:8878:e7b0:f0eb:a03d:8524".to_string();
    let mut sni: Option<String> = None;
    let mut udp = true;

    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                let key = k.to_ascii_lowercase();
                let decoded_val = percent_encoding::percent_decode_str(v)
                    .decode_utf8_lossy()
                    .to_string();
                match key.as_str() {
                    "publickey" | "public-key" | "pub" => pub_key = decoded_val,
                    "privatekey" | "private-key" | "priv" => priv_key = decoded_val,
                    "ip" | "ipv4" => ip = decoded_val,
                    "ipv6" => ipv6 = decoded_val,
                    "sni" => sni = Some(decoded_val),
                    "udp" => udp = decoded_val != "0" && decoded_val != "false",
                    _ => {}
                }
            }
        }
    }

    let name = if let Some(frag) = fragment {
        let dec = percent_encoding::percent_decode_str(frag)
            .decode_utf8_lossy()
            .to_string();
        if dec.trim().is_empty() {
            node_name(&host, port)
        } else {
            dec
        }
    } else {
        node_name(&host, port)
    };

    let mut config = json!({
        "type": "masque",
        "name": name,
        "server": host,
        "port": port,
        "private-key": priv_key,
        "public-key": pub_key,
        "ip": ip,
        "ipv6": ipv6,
        "mtu": 1280,
        "udp": udp,
        "remote-dns-resolve": true,
        "dns": ["1.1.1.1", "2606:4700:4700::1111"]
    });

    if let Some(ref s) = sni {
        config["sni"] = json!(s);
    }

    let mut node = make_node(
        "masque",
        &name,
        &host,
        port,
        uri.trim(),
        config,
        source_id,
    );
    node.country = Some("US".into());
    Some(node)
}

fn parse_host_port(hostport: &str) -> Option<(String, u16)> {
    let hostport = hostport.trim();
    if hostport.starts_with('[') {
        // IPv6 bracketed: [2606:4700:103::1]:443
        let close_bracket = hostport.find(']')?;
        let host = hostport[1..close_bracket].to_string();
        let remainder = &hostport[close_bracket + 1..];
        let port = if let Some(p) = remainder.strip_prefix(':') {
            p.parse::<u16>().ok()?
        } else {
            443
        };
        Some((host, port))
    } else if let Some((h, p)) = hostport.rsplit_once(':') {
        let port = p.parse::<u16>().ok()?;
        Some((h.to_string(), port))
    } else if !hostport.is_empty() {
        Some((hostport.to_string(), 443))
    } else {
        None
    }
}

/// 从 Node 配置重新组装出 masque:// 分享 URI
pub fn rebuild_masque_uri(cfg: &Value, node: &Node) -> Option<String> {
    let priv_key = cfg
        .get("private-key")
        .or_else(|| cfg.get("privateKey"))
        .or_else(|| cfg.get("priv"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let pub_key = cfg
        .get("public-key")
        .or_else(|| cfg.get("publicKey"))
        .or_else(|| cfg.get("pub"))
        .and_then(Value::as_str)
        .unwrap_or_default();

    let ip = cfg
        .get("ip")
        .or_else(|| cfg.get("ipv4"))
        .and_then(Value::as_str)
        .unwrap_or("172.16.0.2");

    Some(build_masque_uri(
        &node.server,
        node.port,
        pub_key,
        priv_key,
        ip,
        &node.name,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_usque_config_json_into_57_nodes() {
        let sample_json = r#"{
            "endpoint_v4": "162.159.198.1:2408",
            "endpoint_v6": "[2606:4700:104::1]:2408",
            "endpoint_pub_key": "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEIaU7MToJm9NKp8YfGxR6r+/h4mcG7SxI8tsW8OR1A5tv/zCzVbCRRh2t87/kxnP6lAy0lkr7qYwu+ox+k3dr6w==\n-----END PUBLIC KEY-----\n",
            "private_key": "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIJNkH3aq4fv01By2+CMbc3hFuu4D7WR+v8V9GRb7RxdRoAoGCCqGSM49AwEHoUQDQgAExBdqjmh8wEmxknMVjlzj6NJHouz/dTO4Ek6byK+edzuIGL4WKqm1CfmooGnInTxJb8g4mFrUTnFihfIiy3wDEQ==\n-----END EC PRIVATE KEY-----\n",
            "ipv4": "172.16.0.2",
            "ipv6": "2606:4700:110:81a0:f46d:806d:1021:930c"
        }"#;

        let nodes = parse_warp_masque_json(sample_json, Some("src_warp")).expect("should parse");
        // 4 v4 * 7 ports + 4 v6 * 7 ports + 1 sni = 57 nodes
        assert_eq!(nodes.len(), 57);

        let n1 = &nodes[0];
        assert_eq!(n1.protocol, "masque");
        assert_eq!(n1.server, "162.159.198.1");
        assert_eq!(n1.port, 443);
        assert_eq!(n1.name, "WARP-198.1-443");
        assert_eq!(n1.country.as_deref(), Some("US"));
        assert!(n1.raw.starts_with("masque://162.159.198.1:443?"));

        let last_node = nodes.last().unwrap();
        assert_eq!(last_node.name, "WARP-官方域名");
        assert_eq!(last_node.config["sni"], OFFICIAL_SNI);
    }

    #[test]
    fn parses_single_masque_uri() {
        let uri = "masque://162.159.198.1:443?publicKey=testpub&privateKey=testpriv&ip=172.16.0.2&dns=1.1.1.1,%208.8.8.8&udp=1&cc=&flag=CDN#WARP-198.1-443";
        let node = parse_masque_uri(uri, Some("src_test")).expect("should parse uri");
        assert_eq!(node.protocol, "masque");
        assert_eq!(node.server, "162.159.198.1");
        assert_eq!(node.port, 443);
        assert_eq!(node.name, "WARP-198.1-443");
        assert_eq!(node.config["public-key"], "testpub");
        assert_eq!(node.config["private-key"], "testpriv");
        assert_eq!(node.config["ip"], "172.16.0.2");
    }

    #[test]
    fn parses_ipv6_masque_uri() {
        let uri = "masque://[2606:4700:103::1]:8443?publicKey=testpub&privateKey=testpriv&ip=172.16.0.2#WARP6-103-1-8443";
        let node = parse_masque_uri(uri, None).expect("should parse ipv6 uri");
        assert_eq!(node.protocol, "masque");
        assert_eq!(node.server, "2606:4700:103::1");
        assert_eq!(node.port, 8443);
        assert_eq!(node.name, "WARP6-103-1-8443");
    }

    #[test]
    fn parses_warp_masque_yaml_via_subscription() {
        let yaml = r#"
proxies:
  - name: "198.1-443"
    type: masque
    server: 162.159.198.1
    port: 443
    private-key: MHcCAQEEtest
    public-key: MFkwEwYtest
    ip: 172.16.0.2
    ipv6: 2606:4700:110:81a0:f46d:806d:1021:930c
    mtu: 1280
    udp: true
"#;
        let nodes = crate::parser::parse_subscription(yaml, Some("src_yaml"));
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].protocol, "masque");
        assert_eq!(nodes[0].name, "198.1-443");
        assert_eq!(nodes[0].server, "162.159.198.1");
        assert_eq!(nodes[0].port, 443);
    }

    #[test]
    fn rebuilds_masque_uri_and_clash_proxy() {
        let uri = "masque://162.159.198.1:443?publicKey=testpub&privateKey=testpriv&ip=172.16.0.2#WARP-198.1-443";
        let node = parse_masque_uri(uri, None).expect("should parse");
        let rebuilt = rebuild_masque_uri(&node.config, &node).expect("rebuilt uri");
        assert!(rebuilt.starts_with("masque://162.159.198.1:443?"));
        assert!(rebuilt.contains("publicKey=testpub"));

        let clash_proxies = crate::export::nodes_to_clash_proxies(&[node]);
        assert_eq!(clash_proxies.len(), 1);
        assert_eq!(clash_proxies[0]["type"], "masque");
        assert_eq!(clash_proxies[0]["server"], "162.159.198.1");
    }
}

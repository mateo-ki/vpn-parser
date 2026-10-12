use super::make_node;
use crate::models::Node;
use serde_json::{json, Value};

/// Parse Clash / Clash Meta YAML subscription into nodes.
pub fn parse_clash_yaml(content: &str, source_id: Option<&str>) -> anyhow::Result<Vec<Node>> {
    let root: Value = serde_yaml::from_str(content)?;
    let proxies = root
        .get("proxies")
        .cloned()
        .or_else(|| {
            // some files are pure list
            if root.is_array() {
                Some(root.clone())
            } else {
                None
            }
        })
        .unwrap_or(Value::Array(vec![]));

    let mut nodes = Vec::new();
    if let Some(arr) = proxies.as_array() {
        for item in arr {
            if let Some(node) = proxy_to_node(item, source_id) {
                nodes.push(node);
            }
        }
    }
    Ok(nodes)
}

fn proxy_to_node(item: &Value, source_id: Option<&str>) -> Option<Node> {
    let obj = item.as_object()?;
    let ty = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let server = obj
        .get("server")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if server.is_empty() {
        return None;
    }
    let port = obj
        .get("port")
        .and_then(|v| {
            v.as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0);
    if port == 0 {
        return None;
    }

    let protocol = match ty.as_str() {
        "ss" | "shadowsocks" => "shadowsocks",
        "ssr" => "shadowsocksr",
        "vmess" => "vmess",
        "vless" => "vless",
        "trojan" => "trojan",
        "hysteria2" | "hy2" => "hysteria2",
        "hysteria" => "hysteria",
        "wireguard" | "wg" => "wireguard",
        "http" | "https" => "http",
        "socks5" | "socks" => "socks5",
        "tuic" => "tuic",
        other if !other.is_empty() => other,
        _ => "unknown",
    };
    // Keep Clash short type in config so re-export stays client-compatible.
    let clash_type = match protocol {
        "shadowsocks" => "ss",
        "shadowsocksr" => "ssr",
        "wireguard" => "wireguard",
        other => other,
    };

    let mut config = item.clone();
    if let Some(map) = config.as_object_mut() {
        map.insert("type".into(), json!(clash_type));
        map.insert("server".into(), json!(server));
        map.insert("port".into(), json!(port));
        map.insert("name".into(), json!(name));
    }

    let raw = serde_yaml::to_string(item).unwrap_or_else(|_| item.to_string());
    Some(make_node(
        protocol, &name, &server, port, &raw, config, source_id,
    ))
}

use crate::models::Node;
use crate::parser::make_node;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

/// 解析 OpenRung 中继节点服务返回的 JSON 数据
/// 支持 broker API: `GET https://broker.openrung.org/api/v1/relays` 以及中继数组列表
pub fn parse_openrung_json(content: &str, source_id: Option<&str>) -> Option<Vec<Node>> {
    let val: Value = serde_json::from_str(content).ok()?;
    let relays = if let Some(arr) = val.get("relays").and_then(Value::as_array) {
        arr
    } else if let Some(arr) = val.as_array() {
        arr
    } else {
        return None;
    };

    if relays.is_empty() {
        return None;
    }

    let mut nodes = Vec::new();
    for r in relays {
        let host = match r
            .get("public_host")
            .and_then(Value::as_str)
            .or_else(|| r.get("host").and_then(Value::as_str))
        {
            Some(h) if !h.trim().is_empty() => h.trim(),
            _ => continue,
        };

        let uuid = match r
            .get("client_id")
            .and_then(Value::as_str)
            .or_else(|| r.get("uuid").and_then(Value::as_str))
        {
            Some(u) if !u.trim().is_empty() => u.trim(),
            _ => continue,
        };

        let pbk = match r
            .get("reality_public_key")
            .and_then(Value::as_str)
            .or_else(|| r.get("pbk").and_then(Value::as_str))
        {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => continue,
        };

        let port = r
            .get("public_port")
            .and_then(Value::as_u64)
            .or_else(|| r.get("port").and_then(Value::as_u64))
            .unwrap_or(443) as u16;

        let label = r
            .get("label")
            .and_then(Value::as_str)
            .or_else(|| r.get("id").and_then(Value::as_str))
            .unwrap_or("relay");

        let city = r.get("city").and_then(Value::as_str).unwrap_or_default();
        let country_code = r
            .get("country_code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let flow = r
            .get("flow")
            .and_then(Value::as_str)
            .unwrap_or("xtls-rprx-vision");
        let sid = r
            .get("short_id")
            .and_then(Value::as_str)
            .or_else(|| r.get("sid").and_then(Value::as_str))
            .unwrap_or_default();
        let sni = r
            .get("server_name")
            .and_then(Value::as_str)
            .or_else(|| r.get("sni").and_then(Value::as_str))
            .unwrap_or("www.cloudflare.com");

        let name = if !city.is_empty() && !country_code.is_empty() {
            format!("OpenRung | {label} ({city}, {country_code})")
        } else if !country_code.is_empty() {
            format!("OpenRung | {label} ({country_code})")
        } else {
            format!("OpenRung | {label}")
        };

        let encoded_fragment = utf8_percent_encode(&name, NON_ALPHANUMERIC).to_string();
        let raw_uri = format!(
            "vless://{uuid}@{host}:{port}?security=reality&encryption=none&pbk={pbk}&headerType=none&type=tcp&flow={flow}&sni={sni}&sid={sid}&fp=chrome#{encoded_fragment}"
        );

        let config = json!({
            "type": "vless",
            "server": host,
            "port": port,
            "uuid": uuid,
            "name": name,
            "flow": flow,
            "encryption": "none",
            "network": "tcp",
            "security": "reality",
            "sni": sni,
            "fp": "chrome",
            "pbk": pbk,
            "sid": sid,
            "allowInsecure": false,
        });

        let mut node = make_node("vless", &name, host, port, &raw_uri, config, source_id);
        if !country_code.is_empty() {
            node.country = Some(country_code.to_uppercase());
        }
        nodes.push(node);
    }

    if nodes.is_empty() {
        None
    } else {
        Some(nodes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openrung_broker_json() {
        let sample = r#"{
            "count": 2,
            "relays": [
                {
                    "id": "relay_f8346d22d10637f8dc02deee9a101e40",
                    "label": "nimble-comet",
                    "public_host": "203.0.113.11",
                    "public_port": 443,
                    "city": "Nuremberg",
                    "country": "Germany",
                    "country_code": "DE",
                    "protocol": "vless-reality-vision",
                    "client_id": "aaaaaaaa-bbbb-4ccc-8ddd-000000000101",
                    "reality_public_key": "lZHSg66b68Go2h_hrs3_sc3GrL4ahI09IuKHgib7iKk",
                    "short_id": "dba06da450fc7c49",
                    "server_name": "www.cloudflare.com",
                    "flow": "xtls-rprx-vision"
                },
                {
                    "id": "relay_552a664d4d564ba58d8f76d6c50ec3a0",
                    "label": "vivid-clover",
                    "public_host": "198.51.100.83",
                    "public_port": 443,
                    "city": "Tokyo",
                    "country": "Japan",
                    "country_code": "JP",
                    "protocol": "vless-reality-vision",
                    "client_id": "aaaaaaaa-bbbb-4ccc-8ddd-000000000102",
                    "reality_public_key": "ZHqpbCdEAaZqnpSyTyT3OAO-efydk50uCZIK4etpHZY",
                    "short_id": "147ba4bbe7fe9a8b",
                    "server_name": "www.cloudflare.com",
                    "flow": "xtls-rprx-vision"
                }
            ]
        }"#;

        let nodes = parse_openrung_json(sample, Some("src_openrung")).expect("should parse");
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].protocol, "vless");
        assert_eq!(nodes[0].server, "203.0.113.11");
        assert_eq!(nodes[0].port, 443);
        assert_eq!(nodes[0].country.as_deref(), Some("DE"));
        assert!(nodes[0].name.contains("nimble-comet"));
        assert!(nodes[0].raw.starts_with("vless://"));
        assert!(nodes[0].raw.contains("security=reality"));
        assert_eq!(nodes[1].country.as_deref(), Some("JP"));
    }
}

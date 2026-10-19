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

/// 从 Clash YAML 订阅文本中提取自带的分流规则（rules: 字段）。
pub fn extract_clash_rules(content: &str) -> Vec<crate::proxy::TrafficRule> {
    let Ok(root) = serde_yaml::from_str::<Value>(content) else {
        return Vec::new();
    };
    let Some(rules_val) = root.get("rules").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut rules = Vec::with_capacity(rules_val.len());
    for item in rules_val {
        let Some(line) = item.as_str() else {
            continue;
        };
        if let Some(mut rule) = crate::proxy::rule_io::parse_single_rule(line) {
            if let crate::proxy::RuleMatcher::RuleSet(ref tag) = rule.matcher {
                let lower = tag.to_ascii_lowercase();
                match lower.as_str() {
                    "reject" | "reject-list" | "ad" | "ads" | "advertising" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoSite("category-ads-all".into());
                    }
                    "direct" | "direct-list" | "cn" | "china" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoSite("cn".into());
                    }
                    "proxy" | "proxy-list" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoSite("geolocation-!cn".into());
                    }
                    "gfw" | "gfwlist" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoSite("gfw".into());
                    }
                    "cncidr" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoIp("CN".into());
                    }
                    "telegramcidr" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoIp("telegram".into());
                    }
                    "lancidr" | "private" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoSite("private".into());
                    }
                    "google" | "youtube" | "telegram" | "netflix" | "openai" | "chatgpt"
                    | "microsoft" | "github" | "twitter" | "facebook" | "apple"
                    | "speedtest" | "steam" => {
                        rule.matcher = crate::proxy::RuleMatcher::GeoSite(lower);
                    }
                    _ => {
                        // 未知外部规则集（通常依赖外部独立 rule-provider），本地无对应数据库时跳过，避免内核崩溃
                        continue;
                    }
                }
            }
            rules.push(rule);
        }
    }
    crate::proxy::optimize_traffic_rules(&rules)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{RuleAction, RuleMatcher};

    #[test]
    fn extracts_clash_rules_successfully() {
        let yaml = r#"
port: 7890
rules:
  - "DOMAIN-SUFFIX,google.com,节点选择"
  - "DOMAIN-KEYWORD,openai,AI分组"
  - "IP-CIDR,192.168.0.0/16,DIRECT"
  - "GEOIP,CN,DIRECT"
  - "MATCH,漏网之鱼"
proxies:
  - name: "HK 01"
    type: ss
    server: 1.1.1.1
    port: 443
    cipher: aes-256-gcm
    password: pass
"#;
        let rules = extract_clash_rules(yaml);
        assert_eq!(rules.len(), 4); // MATCH is dropped as expected
        assert_eq!(
            rules[0].matcher,
            RuleMatcher::DomainSuffix("google.com".into())
        );
        assert_eq!(rules[0].action, RuleAction::Proxy);
        assert_eq!(
            rules[1].matcher,
            RuleMatcher::DomainKeyword("openai".into())
        );
        assert_eq!(rules[1].action, RuleAction::Proxy);
        assert_eq!(
            rules[2].matcher,
            RuleMatcher::IpCidr("192.168.0.0/16".into())
        );
        assert_eq!(rules[2].action, RuleAction::Direct);
        assert_eq!(rules[3].matcher, RuleMatcher::GeoIp("CN".into()));
        assert_eq!(rules[3].action, RuleAction::Direct);
    }

    #[test]
    fn returns_empty_when_no_rules_present() {
        let yaml = r#"
proxies:
  - name: "US 01"
    type: ss
    server: 2.2.2.2
    port: 443
    cipher: aes-256-gcm
    password: pass
"#;
        let rules = extract_clash_rules(yaml);
        assert!(rules.is_empty());
    }

    #[test]
    fn extracts_clash_rules_normalizes_rule_sets() {
        let yaml = r#"
rules:
  - "RULE-SET,reject,REJECT"
  - "RULE-SET,cn,DIRECT"
  - "RULE-SET,telegramcidr,PROXY"
  - "RULE-SET,google,PROXY"
  - "RULE-SET,unknown_custom_provider_xyz,DIRECT"
"#;
        let rules = extract_clash_rules(yaml);
        assert_eq!(rules.len(), 4);
        assert_eq!(
            rules[0].matcher,
            RuleMatcher::GeoSite("category-ads-all".into())
        );
        assert_eq!(rules[0].action, RuleAction::Reject);
        assert_eq!(rules[1].matcher, RuleMatcher::GeoSite("cn".into()));
        assert_eq!(rules[1].action, RuleAction::Direct);
        assert_eq!(rules[2].matcher, RuleMatcher::GeoIp("telegram".into()));
        assert_eq!(rules[2].action, RuleAction::Proxy);
        assert_eq!(rules[3].matcher, RuleMatcher::GeoSite("google".into()));
        assert_eq!(rules[3].action, RuleAction::Proxy);
    }
}

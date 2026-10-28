use crate::export::nodes_to_clash_proxies;
use crate::models::Node;
use crate::proxy::compiler::BackendCompiler;
use crate::proxy::config::{
    apply_mihomo_dns_poison_overrides, gfwlist_traffic_rules, optimize_traffic_rules,
    recommended_traffic_rules, RoutingMode, TrafficRule,
};
use crate::proxy::types::{AutoRoutingStrategy, MihomoConfigOptions, ProxyMode};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

/// 智能分流策略组内置的预置规则集（与编译器保持完全同步）
pub use crate::proxy::compiler::mihomo::SMART_ROUTING_PREPEND_RULES;

/// 内核内自动选路用的两个组名。
pub const URL_TEST_GROUP: &str = "自动选择";

/// url-test 的判活节奏。
pub(crate) const URL_TEST_INTERVAL_SECS: u64 = 60;
pub(crate) const URL_TEST_TOLERANCE_MS: u64 = 30;
pub(crate) const URL_TEST_TIMEOUT_MS: u64 = 3000;
pub const FINAL_GROUP: &str = "漏网之鱼";

pub fn build_mihomo_config(node: &Node, options: MihomoConfigOptions) -> Result<Value> {
    options.validate()?;
    let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(node));
    let compiler = crate::proxy::compiler::MihomoCompiler::new();
    let artifact = compiler.compile(&plan)?;
    artifact.as_json_value()
}

#[allow(dead_code)]
fn build_mihomo_config_legacy(node: &Node, options: MihomoConfigOptions) -> Result<Value> {
    options.validate()?;
    let Some(proxy) = nodes_to_clash_proxies(std::slice::from_ref(node)).pop() else {
        return Err(anyhow!("Mihomo 不支持协议: {}", node.protocol));
    };
    let proxy_name = proxy
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Mihomo 代理配置缺少名称"))?;

    let selected_rules = match options.routing_mode {
        RoutingMode::Rule => {
            let mut combined = options.rules.clone();
            combined.extend(recommended_traffic_rules());
            optimize_traffic_rules(&combined)
        }
        RoutingMode::ChinaDirect => recommended_traffic_rules(),
        RoutingMode::GfwList => gfwlist_traffic_rules(),
        RoutingMode::Global | RoutingMode::Direct => Vec::new(),
        RoutingMode::Subscription => options.rules.clone(),
    };
    // 境内 DoH 污染出的国内 IP 会让 GEOIP,CN 误判直连，先按域名定性（仅 mihomo）。
    let mut selected_rules = selected_rules;
    apply_mihomo_dns_poison_overrides(&mut selected_rules);
    let mut rules = selected_rules
        .iter()
        .map(TrafficRule::to_mihomo_rule)
        .collect::<Result<Vec<_>>>()?;
    // 候选节点：当前节点排第一，其余来自界面按最新测速填进来的 url_test_nodes。
    // clash 要求代理名唯一，而不同订阅里同名节点很常见，所以撞名就加 #2/#3 后缀。
    let mut candidate_proxies = vec![proxy.clone()];
    let mut used_names: std::collections::HashSet<String> =
        std::collections::HashSet::from([proxy_name.to_owned()]);
    let has_override_groups = if let Some(ref override_text) = options.override_yaml {
        override_text.contains("proxy-groups:") || override_text.contains("proxy-groups :")
    } else {
        false
    };
    if options.url_test || has_override_groups {
        let limit = options.url_test_size.max(2) as usize;
        for candidate in &options.url_test_nodes {
            if candidate_proxies.len() >= limit {
                break;
            }
            let Some(mut object) = candidate.as_object().cloned() else {
                continue;
            };
            let raw = object
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if raw.is_empty() {
                continue;
            }
            let mut name = raw.clone();
            let mut index = 2;
            while !used_names.insert(name.clone()) {
                name = format!("{raw} #{index}");
                index += 1;
            }
            object.insert("name".into(), Value::String(name));
            candidate_proxies.push(Value::Object(object));
        }
    }

    // 处理一节点一端口映射：将所有启用的端口映射节点也加入 candidate_proxies，并生成 listeners 与专属路由规则
    let mut port_listeners: Vec<Value> = Vec::new();
    let mut port_rules: Vec<String> = Vec::new();
    for (port, raw_name, clash_obj) in &options.port_mapping_nodes {
        let mut target_proxy_name = None;
        for existing in &candidate_proxies {
            let is_same = existing.get("server") == clash_obj.get("server")
                && existing.get("port") == clash_obj.get("port")
                && existing.get("type") == clash_obj.get("type");
            if is_same {
                if let Some(n) = existing.get("name").and_then(Value::as_str) {
                    target_proxy_name = Some(n.to_string());
                    break;
                }
            }
        }
        let target_name = match target_proxy_name {
            Some(n) => n,
            None => {
                let mut name = raw_name.clone();
                let mut index = 2;
                while !used_names.insert(name.clone()) {
                    name = format!("{raw_name} #{index}");
                    index += 1;
                }
                let mut obj = clash_obj.as_object().cloned().unwrap_or_default();
                obj.insert("name".into(), Value::String(name.clone()));
                candidate_proxies.push(Value::Object(obj));
                name
            }
        };

        port_listeners.push(json!({
            "name": format!("inbound-{}", port),
            "type": "mixed",
            "port": port,
            "listen": if options.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
            "proxy": target_name
        }));
        port_rules.push(format!("IN-PORT,{},{}", port, target_name));
    }

    // 兜底处理：若用户启用了端口映射，但在极少数情况下节点尚未预热成功，
    // 依然为其生成监听并兜底转发至当前活动代理，确保本地监听端口 100% 就绪，不阻断内核启动
    for mapping in &options.port_mappings {
        if !mapping.enabled {
            continue;
        }
        if !port_listeners.iter().any(|l| l.get("port") == Some(&json!(mapping.port))) {
            port_listeners.push(json!({
                "name": format!("inbound-{}", mapping.port),
                "type": "mixed",
                "port": mapping.port,
                "listen": if options.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
                "proxy": proxy_name
            }));
            port_rules.push(format!("IN-PORT,{},{}", mapping.port, proxy_name));
        }
    }

    // 如果启用了全局客户端指纹伪装，为所有支持 TLS 的候选代理注入 client-fingerprint
    if let Some(ref fp) = options.client_fingerprint {
        let fp_val = Value::String(fp.clone());
        for p in &mut candidate_proxies {
            if let Some(obj) = p.as_object_mut() {
                let p_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
                let is_tls = obj.get("tls").and_then(Value::as_bool) == Some(true)
                    || obj.contains_key("reality-opts")
                    || obj.contains_key("public-key")
                    || matches!(p_type, "vless" | "vmess" | "trojan" | "hysteria" | "hysteria2" | "tuic" | "ssr");
                if is_tls && (!obj.contains_key("client-fingerprint") || obj.get("client-fingerprint").and_then(Value::as_str) == Some("")) {
                    obj.insert("client-fingerprint".into(), fp_val.clone());
                }
            }
        }
    }

    // 如果启用了 UDP 端口跳跃，针对 Hysteria/Hysteria2/TUIC 注入端口范围
    if options.udp_port_hopping && !options.hop_ports.trim().is_empty() {
        let ports_str = options.hop_ports.trim().to_string();
        for p in &mut candidate_proxies {
            if let Some(obj) = p.as_object_mut() {
                let p_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
                if matches!(p_type, "hysteria" | "hysteria2" | "tuic") {
                    if !obj.contains_key("ports") {
                        obj.insert("ports".into(), Value::String(ports_str.clone()));
                    }
                }
            }
        }
    }

    let candidate_names: Vec<String> = candidate_proxies
        .iter()
        .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let url_test_active = candidate_names.len() > 1;
    if options.url_test && !url_test_active {
        tracing::warn!("url-test 已开启但候选不足（只有当前节点），本次配置退回单节点 PROXY 组");
    }

    let default_target = match options.routing_mode {
        RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::Global | RoutingMode::Subscription => "PROXY",
        RoutingMode::GfwList | RoutingMode::Direct => "DIRECT",
    };
    let match_target = if url_test_active && default_target == "PROXY" {
        FINAL_GROUP
    } else {
        default_target
    };
    // 内核下载 geo 库用的是它自己的规则引擎；不钉直连就会被下面的 MATCH 丢进代理节点，
    // 实测 10 秒 TLS 握手超时（同一 URL 本机直连 1.4 秒）。
    for host in crate::geo::MIRROR_DIRECT_HOSTS {
        rules.push(format!("DOMAIN-SUFFIX,{host},DIRECT"));
    }
    rules.push(format!("MATCH,{match_target}"));

    let mut smart_groups: Vec<Value> = Vec::new();
    if options.smart_routing {
        let mut us_candidates: Vec<String> = Vec::new();
        let mut download_candidates: Vec<String> = Vec::new();
        for name in &candidate_names {
            let lower = name.to_lowercase();
            let is_warp = lower.contains("warp");
            if (lower.contains("美")
                || lower.contains("us")
                || lower.contains("usa")
                || lower.contains("america")
                || lower.contains("united states")
                || lower.contains("los angeles")
                || lower.contains("san jose")
                || lower.contains("seattle")
                || lower.contains("chicago")
                || lower.contains("dallas")
                || lower.contains("new york")
                || lower.contains("virginia")
                || lower.contains("washington"))
                && !is_warp
            {
                us_candidates.push(name.clone());
            }
            let is_high_multiplier = lower.contains("2x")
                || lower.contains("3x")
                || lower.contains("5x")
                || lower.contains("10x")
                || lower.contains("链式");
            if !is_high_multiplier {
                download_candidates.push(name.clone());
            }
        }
        if us_candidates.is_empty() {
            let non_warp: Vec<String> = candidate_names
                .iter()
                .filter(|n| !n.to_lowercase().contains("warp"))
                .cloned()
                .collect();
            us_candidates = if !non_warp.is_empty() {
                non_warp
            } else if !candidate_names.is_empty() {
                candidate_names.clone()
            } else {
                vec!["DIRECT".to_string()]
            };
        }
        if download_candidates.is_empty() {
            download_candidates = if !candidate_names.is_empty() {
                candidate_names.clone()
            } else {
                vec!["DIRECT".to_string()]
            };
        }

        smart_groups.push(json!({
            "name": "多线并发下载",
            "type": "load-balance",
            "strategy": "round-robin",
            "url": "http://www.gstatic.com/generate_204",
            "interval": 300,
            "timeout": 2000,
            "lazy": true,
            "proxies": download_candidates,
        }));
        smart_groups.push(json!({
            "name": "美国节点",
            "type": "url-test",
            "url": "http://www.gstatic.com/generate_204",
            "interval": 300,
            "timeout": 2000,
            "lazy": true,
            "proxies": us_candidates,
        }));
        let mut normal_proxies = vec!["美国节点".to_string()];
        normal_proxies.extend(candidate_names.iter().cloned());
        smart_groups.push(json!({
            "name": "普通代理",
            "type": "select",
            "proxies": normal_proxies,
        }));
        smart_groups.push(json!({
            "name": "AI分组",
            "type": "select",
            "proxies": ["美国节点", "普通代理"],
        }));
    }

    let mut proxy_groups = if url_test_active {
        let (group_type, extra_fields) = match options.auto_strategy {
            AutoRoutingStrategy::UrlTest => ("url-test", json!({
                "tolerance": URL_TEST_TOLERANCE_MS,
                "max-failed": 3,
            })),
            AutoRoutingStrategy::Fallback => ("fallback", json!({})),
            AutoRoutingStrategy::LoadBalance => ("load-balance", json!({
                "strategy": "consistent-hashing",
            })),
        };
        let mut auto_group = json!({
            "name": URL_TEST_GROUP,
            "type": group_type,
            "url": crate::proxy_probe::PROXY_CONNECTIVITY_TEST_URL,
            "interval": URL_TEST_INTERVAL_SECS,
            "timeout": URL_TEST_TIMEOUT_MS,
            "lazy": true,
            "proxies": candidate_names,
        });
        if let Some(map) = auto_group.as_object_mut() {
            if let Some(extra) = extra_fields.as_object() {
                for (k, v) in extra {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        let mut proxy_members = vec![URL_TEST_GROUP.to_owned()];
        proxy_members.extend(candidate_names.iter().cloned());
        vec![
            json!({ "name": "PROXY", "type": "select", "proxies": proxy_members }),
            auto_group,
            json!({
                "name": FINAL_GROUP,
                "type": "select",
                "proxies": ["PROXY", URL_TEST_GROUP, "DIRECT", "REJECT"],
            }),
        ]
    } else {
        vec![json!({ "name": "PROXY", "type": "select", "proxies": candidate_names })]
    };

    if options.smart_routing {
        // PROXY 组始终作为首位主出站策略组，保持标准 Clash 规范
        let mut final_groups = Vec::new();
        if let Some(proxy_idx) = proxy_groups.iter().position(|g| g.get("name") == Some(&json!("PROXY"))) {
            final_groups.push(proxy_groups.remove(proxy_idx));
        }
        final_groups.extend(smart_groups);
        final_groups.extend(proxy_groups);
        proxy_groups = final_groups;
    }

    let mut final_rules: Vec<String> = Vec::new();
    if options.smart_routing && matches!(options.routing_mode, RoutingMode::Rule | RoutingMode::ChinaDirect | RoutingMode::Subscription) {
        for r in SMART_ROUTING_PREPEND_RULES {
            final_rules.push(r.to_string());
        }
    }
    final_rules.extend(rules);

    if let Some(ref override_text) = options.override_yaml {
        if !override_text.trim().is_empty() {
            if let Ok(override_val) = serde_yaml::from_str::<Value>(override_text) {
                // 兼容两种 YAML 格式：带有顶层 "proxy-groups:" 映射，或用户直接粘贴的策略组列表序列
                let user_groups: Option<Vec<Value>> = if let Some(arr) = override_val.get("proxy-groups").and_then(Value::as_array) {
                    Some(arr.clone())
                } else if let Some(arr) = override_val.as_array() {
                    Some(arr.clone())
                } else {
                    None
                };

                if let Some(groups) = user_groups {
                    for mut g in groups {
                        if let Some(g_obj) = g.as_object_mut() {
                            let g_name = g_obj.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            let filter_regex = g_obj.get("filter").and_then(Value::as_str).and_then(|pat| regex::Regex::new(pat).ok());
                            let exclude_regex = g_obj.get("exclude-filter").and_then(Value::as_str).and_then(|pat| regex::Regex::new(pat).ok());

                            // 无论内核对 include-all/filter 的支持程度如何，在后端均主动根据 filter 与 exclude-filter
                            // 对可用候选节点精准过滤，并灌入 proxies 列表中，确保 100% 成功剔除 exclude-filter 匹配的节点（如 WARP）
                            if filter_regex.is_some() || exclude_regex.is_some() {
                                let mut matched_proxies = Vec::new();
                                for c_name in &candidate_names {
                                    if let Some(ref f) = filter_regex {
                                        if !f.is_match(c_name) {
                                            continue;
                                        }
                                    }
                                    if let Some(ref ef) = exclude_regex {
                                        if ef.is_match(c_name) {
                                            continue;
                                        }
                                    }
                                    matched_proxies.push(c_name.clone());
                                }
                                // 绝对不可填入空数组 []，否则 Mihomo 必定 fatal 报错：Parse config error: ProxyGroup ... proxies missing
                                if matched_proxies.is_empty() {
                                    if candidate_names.is_empty() {
                                        matched_proxies.push("DIRECT".to_string());
                                    } else {
                                        matched_proxies = candidate_names.clone();
                                    }
                                }
                                g_obj.insert("proxies".to_string(), json!(matched_proxies));
                            } else if g_obj.get("proxies").is_none() && g_obj.get("include-all").and_then(Value::as_bool) != Some(true) {
                                if !candidate_names.is_empty() {
                                    g_obj.insert("proxies".to_string(), json!(candidate_names));
                                } else {
                                    g_obj.insert("proxies".to_string(), json!(["DIRECT"]));
                                }
                            }

                            // 替换已存在的同名内置组（例如默认的“美国节点”），彻底消除重名组被默认组覆盖的冲突
                            proxy_groups.retain(|existing| {
                                existing.get("name").and_then(Value::as_str) != Some(&g_name)
                            });
                        }
                        proxy_groups.insert(0, g);
                    }
                }

                if let Some(user_rules) = override_val.get("rules").and_then(Value::as_array) {
                    let mut user_r = Vec::new();
                    for r in user_rules {
                        if let Some(s) = r.as_str() {
                            user_r.push(s.to_string());
                        }
                    }
                    user_r.extend(final_rules);
                    final_rules = user_r;
                }
            }
        }
    }

    // 安全隐私防御规则注入
    let mut privacy_rules: Vec<String> = Vec::new();
    if options.block_webrtc {
        // 标准 STUN / TURN 端口 (UDP / TCP)
        privacy_rules.push("DST-PORT,3478,REJECT".into());
        privacy_rules.push("DST-PORT,3479,REJECT".into());
        // STUN / TURN over TLS/DTLS 端口
        privacy_rules.push("DST-PORT,5349,REJECT".into());
        privacy_rules.push("DST-PORT,5350,REJECT".into());
        // Google STUN 专有端口段 (19302-19309)
        privacy_rules.push("DST-PORT,19302-19309,REJECT".into());
        // 常见 WebRTC 穿透特征域名与关键字匹配
        privacy_rules.push("DOMAIN-KEYWORD,stun,REJECT".into());
        privacy_rules.push("DOMAIN-KEYWORD,turn,REJECT".into());
        privacy_rules.push("DOMAIN-KEYWORD,webrtc,REJECT".into());
    }
    if options.block_trackers {
        privacy_rules.push("GEOSITE,category-ads-all,REJECT".into());
    }
    if !privacy_rules.is_empty() {
        final_rules.splice(0..0, privacy_rules);
    }

    // 独占端口转发规则插入在所有规则最顶端，优先匹配
    if !port_rules.is_empty() {
        final_rules.splice(0..0, port_rules);
    }

    let mut nameserver_policy = serde_json::Map::new();
    nameserver_policy.insert(
        "geosite:cn".to_string(),
        json!([
            "223.5.5.5",
            "119.29.29.29",
            "https://doh.pub/dns-query",
            "https://dns.alidns.com/dns-query"
        ]),
    );
    nameserver_policy.insert(
        "geosite:geolocation-!cn".to_string(),
        json!([
            "https://1.1.1.1/dns-query",
            "https://8.8.8.8/dns-query",
            "https://dns.google/dns-query"
        ]),
    );

    let proxy_server_dns = if !options.dns.proxy_server_nameserver.is_empty() {
        options.dns.proxy_server_nameserver.clone()
    } else {
        vec![
            "223.5.5.5".into(),
            "119.29.29.29".into(),
            "1.1.1.1".into(),
            "8.8.8.8".into(),
        ]
    };

    let mut config = json!({
        "mixed-port": options.mixed_port,
        "external-controller": format!("127.0.0.1:{}", options.controller_port),
        "secret": options.controller_secret,
        "allow-lan": options.allow_lan,
        "mode": "rule",
        "log-level": "info",
        "proxies": candidate_proxies,
        "proxy-groups": proxy_groups,
        "dns": {
            "enable": options.dns.enabled,
            "ipv6": options.dns.ipv6,
            "enhanced-mode": options.dns.enhanced_mode.as_mihomo_value(),
            "fake-ip-range": "198.18.0.1/16",
            "respect-rules": true,
            "default-nameserver": [
                "223.5.5.5",
                "119.29.29.29",
                "1.1.1.1",
                "8.8.8.8"
            ],
            "nameserver": options.dns.nameservers,
            "nameserver-policy": nameserver_policy,
            "proxy-server-nameserver": proxy_server_dns,
            "fake-ip-filter": [
                "*.lan",
                "*.local",
                "localhost.ptlogin2.qq.com",
                "+.msftconnecttest.com",
                "+.msftncsi.com",
                "time.*.com",
                "ntp.*.com"
            ]
        },
        "rules": final_rules
    });
    // 节点自身域名用独立的 DNS 列表解析，避免经 ISP 明文查询暴露代理用途。
    if !options.dns.proxy_server_nameserver.is_empty() {
        config["dns"]["proxy-server-nameserver"] = json!(options.dns.proxy_server_nameserver);
    }

    let mut rule_sets = std::collections::HashSet::new();
    for r in &selected_rules {
        if let crate::proxy::RuleMatcher::RuleSet(name) = &r.matcher {
            rule_sets.insert(name.clone());
        }
    }
    if !rule_sets.is_empty() {
        let mut providers = serde_json::Map::new();
        for name in rule_sets {
            let (url, behavior) = match name.as_str() {
                "reject" | "reject-list" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/reject-list.txt",
                    "domain",
                ),
                "direct" | "direct-list" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/direct-list.txt",
                    "domain",
                ),
                "proxy" | "proxy-list" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/proxy-list.txt",
                    "domain",
                ),
                "gfw" => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/gfw.txt",
                    "domain",
                ),
                _ => (
                    "https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/proxy-list.txt",
                    "domain",
                ),
            };
            providers.insert(
                name.clone(),
                json!({
                    "type": "http",
                    "behavior": behavior,
                    "url": url,
                    "path": format!("./ruleset/{name}.yaml"),
                    "interval": 86400
                }),
            );
        }
        config["rule-providers"] = serde_json::Value::Object(providers);
    }

    let config_uses_geo = true;
    if config_uses_geo {
        config["geodata-mode"] = json!(true);
        // 关死内核侧自动更新：它只会直连 GitHub（实测 0.116 MB/s），在启动窗口内下不完就
        // 留下半截库 → `list cn not found` → 内核起不来。库改由 `crate::geo` 负责：
        // 镜像优先 + 校验大小 + 先写 .part 再原子替换。
        config["geo-auto-update"] = json!(false);
        config["geo-update-interval"] = json!(24);
        config["geox-url"] = json!({
            "geoip": "https://fastly.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/geoip.dat",
            "geosite": "https://fastly.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/geosite.dat"
        });
    }
    if options.mode == ProxyMode::Tun {
        let mut dns_hijack = options.tun.dns_hijack.clone();
        if !dns_hijack.iter().any(|h| h.contains("tcp://")) {
            dns_hijack.push("tcp://any:53".into());
        }
        config["tun"] = json!({
            "enable": true,
            "stack": options.tun.stack.as_mihomo_value(),
            "dns-hijack": dns_hijack,
            "auto-route": options.tun.auto_route,
            "auto-detect-interface": options.tun.auto_detect_interface,
            "strict-route": true,
            "endpoint-independent-nat": true
        });
    }

    if !port_listeners.is_empty() {
        config["listeners"] = Value::Array(port_listeners);
    }

    Ok(config)
}

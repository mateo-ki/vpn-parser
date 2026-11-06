use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::export::nodes_to_clash_proxies;
use crate::kernel::KernelKind;
use crate::proxy::ir::{IRAction, IRMatcher, RoutingPlan};
use crate::proxy::types::{AutoRoutingStrategy, RoutingMode};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::Ipv4Addr;

pub use crate::constants::proxy::{FINAL_GROUP, URL_TEST_GROUP};
pub const URL_TEST_INTERVAL_SECS: u64 = 60;
pub const URL_TEST_TOLERANCE_MS: u64 = 30;
pub const URL_TEST_TIMEOUT_MS: u64 = 3000;

fn ipv4_cidr_interval(cidr: &str) -> Option<(u64, u64)> {
    let (address, prefix) = cidr.split_once('/')?;
    let address = u32::from(address.parse::<Ipv4Addr>().ok()?);
    let prefix = prefix.parse::<u32>().ok().filter(|prefix| *prefix <= 32)?;
    let size = 1_u64 << (32 - prefix);
    let start = u64::from(address) & !(size - 1);
    Some((start, start + size))
}

/// Only recover IPv4 Fake-IPs whose mapping was lost across a core/client restart.
/// Real application-selected addresses must survive SNI/HTTP sniffing unchanged.
/// Mihomo 1.19 has no positive destination-IP filter for sniffing, so express the
/// complement using skip-dst-address. Keep local destinations excluded even when
/// a custom Fake-IP pool overlaps a local network.
fn fake_ip_recovery_skip_destinations(current_pool: &str) -> Vec<String> {
    const LOCAL_IPV4: &[&str] = &[
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16",
        "100.64.0.0/10",
    ];
    // Both common-client 198.18/16 and macOS 198.19/16 can remain cached after
    // switching clients or moving to a custom pool. These are reserved test IPs.
    let mut pools = vec![ipv4_cidr_interval("198.18.0.0/15").unwrap()];
    if let Some(pool) = ipv4_cidr_interval(current_pool) {
        pools.push(pool);
    }
    let local: Vec<_> = LOCAL_IPV4
        .iter()
        .filter_map(|cidr| ipv4_cidr_interval(cidr))
        .collect();

    fn append_complement(
        start: u64,
        prefix: u32,
        pools: &[(u64, u64)],
        local: &[(u64, u64)],
        skipped: &mut Vec<String>,
    ) {
        let end = start + (1_u64 << (32 - prefix));
        let overlaps = |&(first, last): &(u64, u64)| first < end && start < last;
        let covers = |&(first, last): &(u64, u64)| first <= start && end <= last;
        if local.iter().any(covers) || !pools.iter().any(overlaps) {
            skipped.push(format!("{}/{prefix}", Ipv4Addr::from(start as u32)));
        } else if !pools.iter().any(covers) || local.iter().any(overlaps) {
            // A /32 that overlaps a pool is covered by it, so recursion stops.
            let middle = start + ((end - start) / 2);
            append_complement(start, prefix + 1, pools, local, skipped);
            append_complement(middle, prefix + 1, pools, local, skipped);
        }
    }

    let mut skipped = Vec::new();
    append_complement(0, 0, &pools, &local, &mut skipped);
    // IPv6 Fake-IP recovery was not supported: the generated fc00::/18 pool was
    // already excluded by fc00::/7. Protect real IPv6 destinations as well.
    skipped.push("::/0".into());
    skipped
}

/// 智能分流策略组内置的预置规则集
pub const SMART_ROUTING_PREPEND_RULES: &[&str] = &[
    "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
    "IP-CIDR,172.16.0.0/12,DIRECT,no-resolve",
    "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
    "IP-CIDR6,fc00::/7,DIRECT,no-resolve",
    "IP-CIDR6,fe80::/10,DIRECT,no-resolve",
    "DST-PORT,22,DIRECT",
    // AI 视频服务
    "DOMAIN-SUFFIX,runwayml.com,AI分组",
    "DOMAIN-KEYWORD,runwayml,AI分组",
    "DOMAIN-SUFFIX,runway.com,AI分组",
    "DOMAIN-SUFFIX,muse.ai,AI分组",
    // 国际流媒体
    "GEOSITE,youtube,国际流媒体",
    "GEOSITE,netflix,国际流媒体",
    "GEOSITE,disney,国际流媒体",
    "GEOSITE,spotify,国际流媒体",
    "DOMAIN-SUFFIX,youtube.com,国际流媒体",
    "DOMAIN-SUFFIX,youtu.be,国际流媒体",
    "DOMAIN-SUFFIX,ytimg.com,国际流媒体",
    "DOMAIN-SUFFIX,googlevideo.com,国际流媒体",
    "DOMAIN-SUFFIX,netflix.com,国际流媒体",
    "DOMAIN-SUFFIX,nflxvideo.net,国际流媒体",
    "DOMAIN-SUFFIX,disneyplus.com,国际流媒体",
    "DOMAIN-SUFFIX,spotify.com,国际流媒体",
    // Telegram 即时通讯
    "GEOSITE,telegram,Telegram",
    "GEOIP,telegram,Telegram,no-resolve",
    "DOMAIN-SUFFIX,t.me,Telegram",
    "DOMAIN-SUFFIX,telegram.org,Telegram",
    "DOMAIN-SUFFIX,telegram.me,Telegram",
    // Google & 其它美国服务
    "DOMAIN,gemini.google.com,美国节点",
    "DOMAIN-SUFFIX,google.com,美国节点",
    "DOMAIN-SUFFIX,google.com.hk,美国节点",
    "DOMAIN-SUFFIX,google.co.jp,美国节点",
    "DOMAIN-SUFFIX,google.dev,美国节点",
    "DOMAIN-SUFFIX,gstatic.com,美国节点",
    "DOMAIN-SUFFIX,googleapis.com,美国节点",
    "DOMAIN-SUFFIX,googleusercontent.com,美国节点",
    "GEOSITE,google,美国节点",
    "GEOIP,google,美国节点,no-resolve",
    // 主流 AI 服务
    "DOMAIN-SUFFIX,openai.com,AI分组",
    "DOMAIN-SUFFIX,chatgpt.com,AI分组",
    "DOMAIN-SUFFIX,oaistatic.com,AI分组",
    "DOMAIN-SUFFIX,oaiusercontent.com,AI分组",
    "DOMAIN-SUFFIX,anthropic.com,AI分组",
    "DOMAIN-SUFFIX,claude.ai,AI分组",
    "DOMAIN-SUFFIX,claudeusercontent.com,AI分组",
    "DOMAIN-SUFFIX,x.com,AI分组",
    "DOMAIN-SUFFIX,grok.com,AI分组",
    "DOMAIN-SUFFIX,perplexity.ai,AI分组",
    "DOMAIN-SUFFIX,poe.com,AI分组",
    "DOMAIN-SUFFIX,character.ai,AI分组",
    "DOMAIN-SUFFIX,stability.ai,AI分组",
    "DOMAIN-SUFFIX,huggingface.co,AI分组",
    "DOMAIN-SUFFIX,replicate.com,AI分组",
    "DOMAIN-SUFFIX,ai.com,AI分组",
    "DOMAIN-SUFFIX,midjourney.com,美国节点",
    "DOMAIN-SUFFIX,capcut.com,美国节点",
    // 国内直连与防漏网（全量 .cn 顶级域名与常见高防/社区直连，避免境外机房 IP 被拦截触发 ERR_SSL_VERSION_OR_CIPHER_MISMATCH）
    "DOMAIN-SUFFIX,cn,DIRECT",
    "DOMAIN-SUFFIX,52pojie.cn,DIRECT",
    "DOMAIN-SUFFIX,yunaq.com,DIRECT",
    "DOMAIN-SUFFIX,jiasule.com,DIRECT",
    "DOMAIN-SUFFIX,bing.com,DIRECT",
    "DOMAIN-SUFFIX,feishu.cn,DIRECT",
    // 系统连通性探测直连与 OCSP/CRL 证书吊销检查直连
    "DOMAIN-SUFFIX,msftconnecttest.com,DIRECT",
    "DOMAIN-SUFFIX,msftncsi.com,DIRECT",
    "DOMAIN-SUFFIX,digicert.com,DIRECT",
    "DOMAIN-SUFFIX,sectigo.com,DIRECT",
    "DOMAIN-SUFFIX,globalsign.com,DIRECT",
    "DOMAIN-SUFFIX,letsencrypt.org,DIRECT",
    "DOMAIN-SUFFIX,lencr.org,DIRECT",
    "PROCESS-NAME,steam.exe,DIRECT",
    "PROCESS-NAME,BaiduNetdisk.exe,DIRECT",
    "PROCESS-NAME,QQ.exe,DIRECT",
    "PROCESS-NAME,Weixin.exe,DIRECT",
    // 下载器多线并发
    "PROCESS-NAME,fdm.exe,多线并发下载",
    "PROCESS-NAME,PikPak.exe,多线并发下载",
    "PROCESS-NAME,IDMan.exe,多线并发下载",
    "PROCESS-NAME,Motrix.exe,多线并发下载",
    "PROCESS-NAME,Thunder.exe,多线并发下载",
];

/// 辅助根据节点名称或地区推断所属常见区域代码 (HK, JP, SG, US, TW)
fn classify_regional_candidate(name: &str) -> Option<&'static str> {
    if let Some(code) = crate::parser::guess_country(name) {
        match code.as_str() {
            "HK" => return Some("HK"),
            "JP" => return Some("JP"),
            "SG" => return Some("SG"),
            "US" => return Some("US"),
            "TW" => return Some("TW"),
            _ => {}
        }
    }
    let lower = name.to_lowercase();
    if lower.contains("港") || lower.contains("hk") || lower.contains("hong") {
        Some("HK")
    } else if lower.contains("日")
        || lower.contains("jp")
        || lower.contains("japan")
        || lower.contains("tokyo")
        || lower.contains("osaka")
        || name.contains("东京")
        || name.contains("東京")
        || name.contains("大阪")
    {
        Some("JP")
    } else if lower.contains("新")
        || lower.contains("sg")
        || lower.contains("singapore")
        || lower.contains("狮城")
        || lower.contains("獅城")
    {
        Some("SG")
    } else if lower.contains("美")
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
        || lower.contains("washington")
        || name.contains("洛杉矶")
        || name.contains("旧金山")
        || name.contains("西雅图")
        || name.contains("芝加哥")
        || name.contains("达拉斯")
        || name.contains("纽约")
        || name.contains("硅谷")
    {
        Some("US")
    } else if lower.contains("台")
        || lower.contains("tw")
        || lower.contains("taiwan")
        || name.contains("台北")
        || name.contains("臺北")
        || name.contains("台湾")
        || name.contains("臺灣")
    {
        Some("TW")
    } else {
        None
    }
}

pub struct MihomoCompiler {
    capabilities: CompilerCapabilities,
}

impl Default for MihomoCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl MihomoCompiler {
    pub fn new() -> Self {
        Self {
            capabilities: CompilerCapabilities {
                supports_tun: true,
                supports_process_name: true,
                supports_domain_keyword: true,
                supports_proxy_groups: true,
                supports_url_test: true,
                supported_geosite_categories: None,
                supported_geoip_countries: None,
            },
        }
    }
}

/// 对自定义策略组中的 `filter` / `exclude-filter` 正则进行动态宏展开，自动匹配灌入可用候选节点
pub(crate) fn expand_proxy_group_filters(group_val: &mut Value, candidate_names: &[String]) {
    if let Some(g_obj) = group_val.as_object_mut() {
        let filter_regex = g_obj
            .get("filter")
            .and_then(Value::as_str)
            .and_then(|pat| regex::Regex::new(pat).ok());
        let exclude_regex = g_obj
            .get("exclude-filter")
            .and_then(Value::as_str)
            .and_then(|pat| regex::Regex::new(pat).ok());

        if filter_regex.is_some() || exclude_regex.is_some() {
            let mut matched_proxies = Vec::new();
            for c_name in candidate_names {
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
            if matched_proxies.is_empty() {
                if candidate_names.is_empty() {
                    matched_proxies.push("DIRECT".to_string());
                } else {
                    matched_proxies = candidate_names.to_vec();
                }
            }
            g_obj.insert("proxies".to_string(), json!(matched_proxies));
        } else if g_obj.get("proxies").is_none()
            && g_obj.get("include-all").and_then(Value::as_bool) != Some(true)
        {
            if !candidate_names.is_empty() {
                g_obj.insert("proxies".to_string(), json!(candidate_names));
            } else {
                g_obj.insert("proxies".to_string(), json!(["DIRECT"]));
            }
        }
    }
}

impl BackendCompiler for MihomoCompiler {
    fn kernel(&self) -> KernelKind {
        KernelKind::Mihomo
    }

    fn capabilities(&self) -> &CompilerCapabilities {
        &self.capabilities
    }

    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact> {
        let node = plan
            .outbounds
            .primary_node
            .as_ref()
            .ok_or_else(|| anyhow!("Mihomo 编译缺少主节点"))?;

        let Some(proxy) = nodes_to_clash_proxies(std::slice::from_ref(node)).pop() else {
            return Err(anyhow!("Mihomo 不支持协议: {}", node.protocol));
        };
        let proxy_name = proxy
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Mihomo 代理配置缺少名称"))?;

        // 候选代理节点构建与防撞名
        let mut candidate_proxies = vec![proxy.clone()];
        let mut used_names: HashSet<String> = HashSet::from([proxy_name.to_owned()]);
        let has_override_groups = if let Some(ref override_text) = plan.outbounds.override_yaml {
            override_text.contains("proxy-groups:")
                || override_text.contains("proxy-groups :")
                || override_text.contains("+proxy-groups")
                || override_text.contains("proxy-groups+")
        } else {
            false
        };
        if plan.outbounds.url_test || has_override_groups || plan.outbounds.chain_proxy_enabled || plan.routing_mode == RoutingMode::Chain {
            let limit = plan.outbounds.url_test_size.max(2) as usize;
            for candidate in &plan.outbounds.url_test_nodes {
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

        // 链式代理节点注册与 dialer-proxy 代理构造 (彻底替代已废弃的 type: relay 组)
        let is_chain_requested = plan.outbounds.chain_proxy_enabled || plan.routing_mode == RoutingMode::Chain;
        let mut chain_entry_name: Option<String> = None;
        let mut chain_exit_name: Option<String> = None;
        let mut chain_exit_obj: Option<Value> = None;

        if is_chain_requested {
            // 1. 优先检查运行时预热注入的 chain_nodes 实体（支持跨订阅源全库直查）
            if let Some((ref entry_node, ref exit_node)) = plan.outbounds.chain_nodes.as_deref() {
                let is_distinct = if entry_node.id != exit_node.id {
                    true
                } else {
                    entry_node.name != exit_node.name || entry_node.server != exit_node.server || entry_node.port != exit_node.port
                };
                if is_distinct {
                    let entry_clash = crate::export::nodes_to_clash_proxies(std::slice::from_ref(entry_node));
                    let exit_clash = crate::export::nodes_to_clash_proxies(std::slice::from_ref(exit_node));
                    if let (Some(mut e_obj), Some(mut x_obj)) = (entry_clash.into_iter().next(), exit_clash.into_iter().next()) {
                        // 注册 entry (若 candidate_proxies 中已存在同节点或同名，直接复用既有名称，不生成冗余后缀副本)
                        let e_existing = candidate_proxies.iter().find(|p| {
                            (p.get("server") == e_obj.get("server") && p.get("port") == e_obj.get("port") && p.get("type") == e_obj.get("type"))
                                || p.get("name").and_then(Value::as_str) == Some(&entry_node.name)
                        }).and_then(|p| p.get("name").and_then(Value::as_str)).map(str::to_string);

                        if let Some(existing_name) = e_existing {
                            chain_entry_name = Some(existing_name);
                        } else {
                            let e_raw = e_obj.get("name").and_then(Value::as_str).unwrap_or(&entry_node.name).to_string();
                            let mut e_name = e_raw.clone();
                            let mut idx = 2;
                            while !used_names.insert(e_name.clone()) {
                                e_name = format!("{e_raw} #{idx}");
                                idx += 1;
                            }
                            if let Some(m) = e_obj.as_object_mut() {
                                m.insert("name".into(), Value::String(e_name.clone()));
                            }
                            candidate_proxies.push(e_obj);
                            chain_entry_name = Some(e_name);
                        }

                        // 注册 exit (若 candidate_proxies 中已存在同节点或同名，直接复用)
                        let x_existing = candidate_proxies.iter().find(|p| {
                            (p.get("server") == x_obj.get("server") && p.get("port") == x_obj.get("port") && p.get("type") == x_obj.get("type"))
                                || p.get("name").and_then(Value::as_str) == Some(&exit_node.name)
                        }).cloned();

                        if let Some(existing_val) = x_existing {
                            let x_name = existing_val.get("name").and_then(Value::as_str).unwrap_or(&exit_node.name).to_string();
                            chain_exit_obj = Some(existing_val);
                            chain_exit_name = Some(x_name);
                        } else {
                            let x_raw = x_obj.get("name").and_then(Value::as_str).unwrap_or(&exit_node.name).to_string();
                            let mut x_name = x_raw.clone();
                            let mut idx = 2;
                            while !used_names.insert(x_name.clone()) {
                                x_name = format!("{x_raw} #{idx}");
                                idx += 1;
                            }
                            if let Some(m) = x_obj.as_object_mut() {
                                m.insert("name".into(), Value::String(x_name.clone()));
                            }
                            chain_exit_obj = Some(x_obj.clone());
                            candidate_proxies.push(x_obj);
                            chain_exit_name = Some(x_name);
                        }
                    }
                }
            }

            // 2. 若未从 chain_nodes 注入，尝试从 candidate_proxies 或 url_test_nodes 按名称匹配
            if chain_entry_name.is_none() || chain_exit_name.is_none() {
                let opt_entry = plan.outbounds.chain_entry_node_name.as_deref();
                let opt_exit = plan.outbounds.chain_exit_node_name.as_deref();
                match (opt_entry, opt_exit) {
                    (Some(e), Some(x)) if e != x => {
                        let e_found = candidate_proxies.iter().find(|p| p.get("name").and_then(Value::as_str) == Some(e));
                        let x_found = candidate_proxies.iter().find(|p| p.get("name").and_then(Value::as_str) == Some(x));
                        if let (Some(_), Some(x_p)) = (e_found, x_found) {
                            chain_entry_name = Some(e.to_string());
                            chain_exit_name = Some(x.to_string());
                            chain_exit_obj = Some(x_p.clone());
                        }
                    }
                    (None, None) if plan.routing_mode == RoutingMode::Chain && candidate_proxies.len() >= 2 => {
                        let e_p = &candidate_proxies[1];
                        let x_p = &candidate_proxies[0];
                        if let (Some(e), Some(x)) = (e_p.get("name").and_then(Value::as_str), x_p.get("name").and_then(Value::as_str)) {
                            chain_entry_name = Some(e.to_string());
                            chain_exit_name = Some(x.to_string());
                            chain_exit_obj = Some(x_p.clone());
                        }
                    }
                    _ => {}
                }
            }

            // 3. 成功定位双节点：构造并插入「链式代理」proxy 实体 (使用 dialer-proxy 连接前置节点)
            if let (Some(ref e_name), Some(ref x_name), Some(mut x_val)) = (chain_entry_name.as_ref(), chain_exit_name.as_ref(), chain_exit_obj) {
                if e_name != x_name {
                    if let Some(x_obj) = x_val.as_object_mut() {
                        x_obj.insert("name".into(), Value::String("链式代理".into()));
                        x_obj.insert("dialer-proxy".into(), Value::String((*e_name).clone()));
                        used_names.insert("链式代理".into());
                        candidate_proxies.push(Value::Object(x_obj.clone()));
                    }
                }
            }
        }

        // 处理一节点一端口映射
        let mut port_listeners: Vec<Value> = Vec::new();
        let mut port_rules: Vec<String> = Vec::new();
        for (port, raw_name, clash_obj) in &plan.outbounds.port_mapping_nodes {
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
                "listen": if plan.inbounds.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
                "proxy": target_name
            }));
            port_rules.push(format!("IN-PORT,{},{}", port, target_name));
        }

        for mapping in &plan.inbounds.port_mappings {
            if !mapping.enabled {
                continue;
            }
            if !port_listeners
                .iter()
                .any(|l| l.get("port") == Some(&json!(mapping.port)))
            {
                port_listeners.push(json!({
                    "name": format!("inbound-{}", mapping.port),
                    "type": "mixed",
                    "port": mapping.port,
                    "listen": if plan.inbounds.allow_lan { "0.0.0.0" } else { "127.0.0.1" },
                    "proxy": proxy_name
                }));
                port_rules.push(format!("IN-PORT,{},{}", mapping.port, proxy_name));
            }
        }

        // 全局客户端指纹
        if let Some(ref fp) = plan.outbounds.client_fingerprint {
            let fp_val = Value::String(fp.clone());
            for p in &mut candidate_proxies {
                if let Some(obj) = p.as_object_mut() {
                    let p_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
                    let is_tls = obj.get("tls").and_then(Value::as_bool) == Some(true)
                        || obj.contains_key("reality-opts")
                        || obj.contains_key("public-key")
                        || matches!(
                            p_type,
                            "vless" | "vmess" | "trojan" | "hysteria" | "hysteria2" | "tuic" | "ssr"
                        );
                    if is_tls
                        && (!obj.contains_key("client-fingerprint")
                            || obj
                                .get("client-fingerprint")
                                .and_then(Value::as_str)
                                == Some(""))
                    {
                        obj.insert("client-fingerprint".into(), fp_val.clone());
                    }
                }
            }
        }

        // UDP 端口跳跃
        if plan.outbounds.udp_port_hopping && !plan.outbounds.hop_ports.trim().is_empty() {
            let ports_str = plan.outbounds.hop_ports.trim().to_string();
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

        let match_target = if url_test_active && plan.default_action == IRAction::Proxy(None) {
            FINAL_GROUP
        } else {
            match plan.default_action {
                IRAction::Direct => "DIRECT",
                IRAction::Proxy(ref tag) => tag.as_deref().unwrap_or("PROXY"),
                IRAction::Reject => "REJECT",
                IRAction::HijackDns => "DIRECT",
            }
        };

        // 策略组构建
        let mut smart_groups: Vec<Value> = Vec::new();
        let mut regional_group_names: Vec<String> = Vec::new();

        if plan.outbounds.smart_routing {
            let mut hk_candidates: Vec<String> = Vec::new();
            let mut jp_candidates: Vec<String> = Vec::new();
            let mut sg_candidates: Vec<String> = Vec::new();
            let mut us_candidates: Vec<String> = Vec::new();
            let mut tw_candidates: Vec<String> = Vec::new();
            let mut download_candidates: Vec<String> = Vec::new();

            for name in &candidate_names {
                let lower = name.to_lowercase();
                let is_warp = lower.contains("warp");
                match classify_regional_candidate(name) {
                    Some("HK") => hk_candidates.push(name.clone()),
                    Some("JP") => jp_candidates.push(name.clone()),
                    Some("SG") => sg_candidates.push(name.clone()),
                    Some("US") if !is_warp => us_candidates.push(name.clone()),
                    Some("TW") => tw_candidates.push(name.clone()),
                    _ => {}
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

            let probe_url = "http://www.gstatic.com/generate_204";

            // 1. 业务场景策略组
            let mut ai_proxies = vec!["美国节点".to_string()];
            if !sg_candidates.is_empty() {
                ai_proxies.push("新加坡节点".to_string());
            }
            if !jp_candidates.is_empty() {
                ai_proxies.push("日本节点".to_string());
            }
            if !tw_candidates.is_empty() {
                ai_proxies.push("台湾节点".to_string());
            }
            ai_proxies.push("普通代理".to_string());
            smart_groups.push(json!({
                "name": "AI分组",
                "type": "select",
                "proxies": ai_proxies,
            }));

            let mut stream_proxies = Vec::new();
            if !hk_candidates.is_empty() {
                stream_proxies.push("香港节点".to_string());
            }
            if !sg_candidates.is_empty() {
                stream_proxies.push("新加坡节点".to_string());
            }
            if !jp_candidates.is_empty() {
                stream_proxies.push("日本节点".to_string());
            }
            if !tw_candidates.is_empty() {
                stream_proxies.push("台湾节点".to_string());
            }
            stream_proxies.push("美国节点".to_string());
            stream_proxies.push("普通代理".to_string());
            smart_groups.push(json!({
                "name": "国际流媒体",
                "type": "select",
                "proxies": stream_proxies,
            }));

            let mut tg_proxies = Vec::new();
            if !sg_candidates.is_empty() {
                tg_proxies.push("新加坡节点".to_string());
            }
            if !hk_candidates.is_empty() {
                tg_proxies.push("香港节点".to_string());
            }
            if !jp_candidates.is_empty() {
                tg_proxies.push("日本节点".to_string());
            }
            tg_proxies.push("美国节点".to_string());
            tg_proxies.push("普通代理".to_string());
            smart_groups.push(json!({
                "name": "Telegram",
                "type": "select",
                "proxies": tg_proxies,
            }));

            smart_groups.push(json!({
                "name": "多线并发下载",
                "type": "load-balance",
                "strategy": "round-robin",
                "url": probe_url,
                "interval": 300,
                "timeout": 2000,
                "lazy": true,
                "proxies": download_candidates,
            }));

            // 2. 地区自动测速选优组（按多订阅节点池实际包含的地区动态创建）
            if !hk_candidates.is_empty() {
                regional_group_names.push("香港节点".to_string());
                smart_groups.push(json!({
                    "name": "香港节点",
                    "type": "url-test",
                    "url": probe_url,
                    "interval": 300,
                    "tolerance": 50,
                    "timeout": 2000,
                    "lazy": true,
                    "proxies": hk_candidates,
                }));
            }
            if !jp_candidates.is_empty() {
                regional_group_names.push("日本节点".to_string());
                smart_groups.push(json!({
                    "name": "日本节点",
                    "type": "url-test",
                    "url": probe_url,
                    "interval": 300,
                    "tolerance": 50,
                    "timeout": 2000,
                    "lazy": true,
                    "proxies": jp_candidates,
                }));
            }
            if !sg_candidates.is_empty() {
                regional_group_names.push("新加坡节点".to_string());
                smart_groups.push(json!({
                    "name": "新加坡节点",
                    "type": "url-test",
                    "url": probe_url,
                    "interval": 300,
                    "tolerance": 50,
                    "timeout": 2000,
                    "lazy": true,
                    "proxies": sg_candidates,
                }));
            }
            regional_group_names.push("美国节点".to_string());
            smart_groups.push(json!({
                "name": "美国节点",
                "type": "url-test",
                "url": probe_url,
                "interval": 300,
                "tolerance": 50,
                "timeout": 2000,
                "lazy": true,
                "proxies": us_candidates,
            }));
            if !tw_candidates.is_empty() {
                regional_group_names.push("台湾节点".to_string());
                smart_groups.push(json!({
                    "name": "台湾节点",
                    "type": "url-test",
                    "url": probe_url,
                    "interval": 300,
                    "tolerance": 50,
                    "timeout": 2000,
                    "lazy": true,
                    "proxies": tw_candidates,
                }));
            }

            // 3. 普通代理
            let mut normal_proxies = regional_group_names.clone();
            normal_proxies.extend(candidate_names.iter().cloned());
            smart_groups.push(json!({
                "name": "普通代理",
                "type": "select",
                "proxies": normal_proxies,
            }));
        }

        let is_chain_active = candidate_proxies
            .iter()
            .any(|p| p.get("name").and_then(Value::as_str) == Some("链式代理"));

        let mut proxy_groups = if url_test_active {
            let (group_type, extra_fields) = match plan.outbounds.auto_strategy {
                AutoRoutingStrategy::UrlTest => ("url-test", json!({
                    "tolerance": URL_TEST_TOLERANCE_MS,
                    "max-failed": 3,
                })),
                AutoRoutingStrategy::Fallback => ("fallback", json!({})),
                AutoRoutingStrategy::LoadBalance => ("load-balance", json!({
                    "strategy": "consistent-hashing",
                })),
            };
            let auto_proxies: Vec<String> = candidate_names
                .iter()
                .filter(|n| n.as_str() != "链式代理")
                .cloned()
                .collect();
            let mut auto_group = json!({
                "name": URL_TEST_GROUP,
                "type": group_type,
                "url": crate::proxy_probe::PROXY_CONNECTIVITY_TEST_URL,
                "interval": URL_TEST_INTERVAL_SECS,
                "timeout": URL_TEST_TIMEOUT_MS,
                "lazy": true,
                "proxies": auto_proxies,
            });
            if let Some(map) = auto_group.as_object_mut() {
                if let Some(extra) = extra_fields.as_object() {
                    for (k, v) in extra {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
            let mut proxy_members = Vec::new();
            if is_chain_active {
                proxy_members.push("链式代理".to_string());
            }
            proxy_members.push(URL_TEST_GROUP.to_owned());
            if plan.outbounds.smart_routing {
                for r_name in &regional_group_names {
                    if !proxy_members.contains(r_name) {
                        proxy_members.push(r_name.clone());
                    }
                }
                proxy_members.push("DIRECT".to_string());
            }
            proxy_members.extend(
                candidate_names
                    .iter()
                    .filter(|n| n.as_str() != "链式代理")
                    .cloned(),
            );
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
            let mut proxy_members = Vec::new();
            if is_chain_active {
                proxy_members.push("链式代理".to_string());
            }
            if plan.outbounds.smart_routing {
                proxy_members.extend(regional_group_names.iter().cloned());
                proxy_members.push("DIRECT".to_string());
            }
            proxy_members.extend(
                candidate_names
                    .iter()
                    .filter(|n| n.as_str() != "链式代理")
                    .cloned(),
            );
            vec![json!({ "name": "PROXY", "type": "select", "proxies": proxy_members })]
        };

        if plan.outbounds.smart_routing {
            let mut final_groups = Vec::new();
            if let Some(proxy_idx) = proxy_groups.iter().position(|g| g.get("name") == Some(&json!("PROXY"))) {
                final_groups.push(proxy_groups.remove(proxy_idx));
            }
            final_groups.extend(smart_groups);
            final_groups.extend(proxy_groups);
            proxy_groups = final_groups;
        }

        // 订阅源原生策略组注入
        if let Some(ref sub_groups_text) = plan.outbounds.subscription_groups_yaml {
            if !sub_groups_text.trim().is_empty() {
                if let Ok(sub_groups_val) = serde_yaml::from_str::<Value>(sub_groups_text) {
                    let user_groups: Option<Vec<Value>> = if let Some(arr) = sub_groups_val.get("proxy-groups").and_then(Value::as_array) {
                        Some(arr.clone())
                    } else if let Some(arr) = sub_groups_val.as_array() {
                        Some(arr.clone())
                    } else {
                        None
                    };

                    if let Some(groups) = user_groups {
                        let sub_group_names: Vec<String> = groups
                            .iter()
                            .filter_map(|g| g.get("name").and_then(Value::as_str).map(String::from))
                            .collect();
                        for mut g in groups {
                            if let Some(g_obj) = g.as_object_mut() {
                                let g_name = g_obj
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string();
                                let filter_regex = g_obj
                                    .get("filter")
                                    .and_then(Value::as_str)
                                    .and_then(|pat| regex::Regex::new(pat).ok());
                                let exclude_regex = g_obj
                                    .get("exclude-filter")
                                    .and_then(Value::as_str)
                                    .and_then(|pat| regex::Regex::new(pat).ok());

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
                                    if matched_proxies.is_empty() {
                                        if candidate_names.is_empty() {
                                            matched_proxies.push("DIRECT".to_string());
                                        } else {
                                            matched_proxies = candidate_names.clone();
                                        }
                                    }
                                    g_obj.insert("proxies".to_string(), json!(matched_proxies));
                                } else {
                                    // 检查原 proxies 成员，保留依然存在于 candidate_names、其他策略组名、或 DIRECT/REJECT 中的成员
                                    let mut effective_proxies = Vec::new();
                                    if let Some(arr) = g_obj.get("proxies").and_then(Value::as_array) {
                                        for p in arr {
                                            if let Some(s) = p.as_str() {
                                                if candidate_names.contains(&s.to_string())
                                                    || sub_group_names.iter().any(|gn| gn == s)
                                                    || matches!(s, "DIRECT" | "REJECT" | "PASS" | "GLOBAL")
                                                {
                                                    effective_proxies.push(s.to_string());
                                                }
                                            }
                                        }
                                    }
                                    // 如果原订阅中的代理名在当前聚合节点池中没有同名节点，按地区或全量智能注水，避免空策略组
                                    if !effective_proxies.iter().any(|p| candidate_names.contains(p)) {
                                        let region_match = if let Some(reg) = classify_regional_candidate(&g_name) {
                                            candidate_names
                                                .iter()
                                                .filter(|n| classify_regional_candidate(n) == Some(reg))
                                                .cloned()
                                                .collect::<Vec<_>>()
                                        } else {
                                            Vec::new()
                                        };
                                        if !region_match.is_empty() {
                                            effective_proxies.extend(region_match);
                                        } else if !candidate_names.is_empty() {
                                            effective_proxies.extend(candidate_names.iter().cloned());
                                        } else {
                                            effective_proxies.push("DIRECT".to_string());
                                        }
                                    }
                                    g_obj.insert("proxies".to_string(), json!(effective_proxies));
                                }

                                proxy_groups.retain(|existing| {
                                    existing.get("name").and_then(Value::as_str) != Some(&g_name)
                                });
                                proxy_groups.push(g);
                            }
                        }
                    }
                }
            }
        }

        // 规则编译
        let mut final_rules: Vec<String> = Vec::new();

        // 智能分流内置预置规则
        if plan.outbounds.smart_routing {
            for r in SMART_ROUTING_PREPEND_RULES {
                final_rules.push((*r).to_string());
            }
        }

        // 编译 IR 规则
        let mut has_injected_poison_overrides = false;

        for ir_rule in &plan.rules {
            if ir_rule.action == IRAction::HijackDns {
                continue; // 由 TUN DNS 劫持接管
            }
            let target = match &ir_rule.action {
                IRAction::Direct => "DIRECT".to_string(),
                IRAction::Proxy(None) => "PROXY".to_string(),
                IRAction::Proxy(Some(name)) => {
                    if candidate_names.contains(name) || proxy_groups.iter().any(|g| g.get("name") == Some(&json!(name))) {
                        name.clone()
                    } else {
                        "PROXY".to_string()
                    }
                }
                IRAction::Reject => "REJECT".to_string(),
                IRAction::HijackDns => "DIRECT".to_string(),
            };

            for matcher in &ir_rule.matchers {
                if let IRMatcher::GeoIp(ref c) = matcher {
                    if c.eq_ignore_ascii_case("CN") && !has_injected_poison_overrides {
                        has_injected_poison_overrides = true;
                        final_rules.push("GEOSITE,google,PROXY".to_string());
                        final_rules.push("GEOSITE,youtube,PROXY".to_string());
                    }
                }

                let rule_str = match matcher {
                    IRMatcher::Domain(d) => format!("DOMAIN,{d},{target}"),
                    IRMatcher::DomainSuffix(d) => format!("DOMAIN-SUFFIX,{d},{target}"),
                    IRMatcher::DomainKeyword(k) => format!("DOMAIN-KEYWORD,{k},{target}"),
                    IRMatcher::DomainRegex(r) => format!("DOMAIN-REGEX,{r},{target}"),
                    IRMatcher::IpCidr(c) => {
                        let prefix = if c.contains(':') { "IP-CIDR6" } else { "IP-CIDR" };
                        if ir_rule.no_resolve || crate::hostutil::is_private_cidr(c) {
                            format!("{prefix},{c},{target},no-resolve")
                        } else {
                            format!("{prefix},{c},{target}")
                        }
                    }
                    IRMatcher::Port(p) => format!("DST-PORT,{p},{target}"),
                    IRMatcher::PortRange(s, e) => format!("DST-PORT,{s}-{e},{target}"),
                    IRMatcher::GeoSite(s) => {
                        let normalized = crate::geo::normalize_geo_code(s);
                        if !crate::geo::has_geosite_category(normalized) {
                            tracing::warn!(category = %s, "本地 geosite.dat 缺少该分类，已自动跳过此规则以防止 Mihomo 拒绝启动");
                            continue;
                        }
                        format!("GEOSITE,{normalized},{target}")
                    }
                    IRMatcher::GeoIp(c) => {
                        let c = crate::geo::normalize_geo_code(c);
                        if !crate::geo::has_geoip_category(c) {
                            tracing::warn!(country = %c, "本地 geoip.dat 缺少该分类，已自动跳过此规则以防止 Mihomo 拒绝启动");
                            continue;
                        }
                        if ir_rule.no_resolve {
                            format!("GEOIP,{c},{target},no-resolve")
                        } else {
                            format!("GEOIP,{c},{target}")
                        }
                    }
                    IRMatcher::ProcessName(p) => format!("PROCESS-NAME,{p},{target}"),
                    IRMatcher::RuleSet(s) => {
                        let lower = s.to_ascii_lowercase();
                        let mapped = match lower.as_str() {
                            "reject" | "reject-list" | "ad" | "ads" | "advertising" => {
                                Some(format!("GEOSITE,category-ads-all,{target}"))
                            }
                            "direct" | "direct-list" | "cn" | "china" => {
                                Some(format!("GEOSITE,cn,{target}"))
                            }
                            "proxy" | "proxy-list" => {
                                Some(format!("GEOSITE,geolocation-!cn,{target}"))
                            }
                            "gfw" | "gfwlist" => {
                                Some(format!("GEOSITE,gfw,{target}"))
                            }
                            "cncidr" => {
                                Some(format!("GEOIP,CN,{target}"))
                            }
                            "telegramcidr" => {
                                Some(format!("GEOIP,telegram,{target}"))
                            }
                            "lancidr" | "private" => {
                                Some(format!("GEOSITE,private,{target}"))
                            }
                            other => {
                                if crate::geo::has_geosite_category(other) {
                                    Some(format!("GEOSITE,{other},{target}"))
                                } else {
                                    tracing::warn!(ruleset = %other, "本地未预置且未找到该规则集分类，已自动跳过以保代理平稳启动");
                                    None
                                }
                            }
                        };
                        let Some(rule_str) = mapped else {
                            continue;
                        };
                        rule_str
                    }
                    IRMatcher::Network(proto) => {
                        format!("NETWORK,{},{target}", match proto {
                            crate::proxy::NetworkProtocol::Tcp => "tcp",
                            crate::proxy::NetworkProtocol::Udp => "udp",
                        })
                    }
                    IRMatcher::Inbound(tag) => {
                        if let Some(port) = tag.strip_prefix("inbound-") {
                            format!("IN-PORT,{port},{target}")
                        } else {
                            format!("IN-NAME,{tag},{target}")
                        }
                    }
                    IRMatcher::Quic => format!("AND,((NETWORK,udp),(DST-PORT,443)),{target}"),
                };
                final_rules.push(rule_str);
            }
        }

        // 用户自定义覆写 YAML
        if let Some(ref override_text) = plan.outbounds.override_yaml {
            if !override_text.trim().is_empty() {
                if let Ok(override_val) = serde_yaml::from_str::<Value>(override_text) {
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

                                proxy_groups.retain(|existing| {
                                    existing.get("name").and_then(Value::as_str) != Some(&g_name)
                                });
                                proxy_groups.insert(0, g);
                            }
                        }
                    }

                    if let Some(user_rules) = override_val.get("rules").and_then(Value::as_array) {
                        let mut user_r = Vec::new();
                        for r in user_rules {
                            if let Some(s) = r.as_str() {
                                let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
                                if parts.len() >= 2 {
                                    let rule_type = parts[0].to_ascii_uppercase();
                                    let payload = parts[1];
                                    if rule_type == "GEOSITE" && !crate::geo::has_geosite_category(payload) {
                                        tracing::warn!(category = %payload, "自定义规则中的 GEOSITE 分类在本地不存在，已自动跳过");
                                        continue;
                                    }
                                    if rule_type == "GEOIP" && !crate::geo::has_geoip_category(payload) {
                                        tracing::warn!(country = %payload, "自定义规则中的 GEOIP 分类在本地不存在，已自动跳过");
                                        continue;
                                    }
                                }
                                if !final_rules.iter().any(|existing| existing == s) {
                                    user_r.push(s.to_string());
                                }
                            }
                        }
                        if !user_r.is_empty() {
                            user_r.extend(final_rules);
                            final_rules = user_r;
                        }
                    }
                }
            }
        }

        // MATCH 兜底
        final_rules.push(format!("MATCH,{match_target}"));

        let mut nameserver_policy = serde_json::Map::new();
        if crate::geo::has_geosite_category("cn") {
            nameserver_policy.insert(
                "geosite:cn".to_string(),
                json!([
                    "223.5.5.5",
                    "119.29.29.29",
                    "https://doh.pub/dns-query",
                    "https://dns.alidns.com/dns-query"
                ]),
            );
        }
        nameserver_policy.insert(
            "+.cn".to_string(),
            json!([
                "223.5.5.5",
                "119.29.29.29",
                "https://doh.pub/dns-query",
                "https://dns.alidns.com/dns-query"
            ]),
        );
        nameserver_policy.insert(
            "+.52pojie.cn".to_string(),
            json!([
                "223.5.5.5",
                "119.29.29.29",
                "https://doh.pub/dns-query",
                "https://dns.alidns.com/dns-query"
            ]),
        );
        if crate::geo::has_geosite_category("geolocation-!cn") {
            nameserver_policy.insert(
                "geosite:geolocation-!cn".to_string(),
                json!([
                    "https://1.1.1.1/dns-query",
                    "https://8.8.8.8/dns-query",
                    "https://dns.google/dns-query"
                ]),
            );
        }

        let proxy_server_dns = if !plan.dns.proxy_server_nameservers.is_empty() {
            plan.dns.proxy_server_nameservers.clone()
        } else {
            vec![
                "223.5.5.5".into(),
                "119.29.29.29".into(),
                "1.1.1.1".into(),
                "8.8.8.8".into(),
            ]
        };

        let mode_str = match plan.routing_mode {
            RoutingMode::Global => "global",
            RoutingMode::Direct => "direct",
            _ => "rule",
        };

        let default_sniffer_skip =
            json!(fake_ip_recovery_skip_destinations(&plan.dns.fake_ip_range));
        let mut config = json!({
            "mixed-port": plan.inbounds.mixed_port,
            "external-controller": format!("127.0.0.1:{}", plan.inbounds.controller_port),
            "secret": plan.inbounds.controller_secret,
            "allow-lan": plan.inbounds.allow_lan,
            "mode": mode_str,
            "log-level": "info",
            "ipv6": plan.dns.ipv6,
            // Keep Fake-IP mappings across core restarts. TCP sniffing also lets
            // Mihomo recover a domain from a browser's older Fake-IP cache.
            "profile": { "store-fake-ip": true },
            "sniffer": {
                "enable": true,
                "force-dns-mapping": true,
                "parse-pure-ip": true,
                "override-destination": true,
                "skip-dst-address": default_sniffer_skip,
                "sniff": {
                    "HTTP": { "ports": [80, 8080] },
                    "TLS": { "ports": [443, 8443] }
                }
            },
            "proxies": candidate_proxies,
            "proxy-groups": proxy_groups,
            "dns": {
                "enable": plan.dns.enabled,
                "ipv6": plan.dns.ipv6,
                "enhanced-mode": plan.dns.enhanced_mode.as_mihomo_value(),
                "fake-ip-range": plan.dns.fake_ip_range,
                "fake-ip-range6": "fc00::/18",
                "respect-rules": plan.dns.respect_rules,
                "default-nameserver": plan.dns.default_nameservers,
                "nameserver": plan.dns.nameservers,
                "nameserver-policy": nameserver_policy,
                "proxy-server-nameserver": proxy_server_dns,
                "fake-ip-filter": plan.dns.fake_ip_filter,
            },
            "rules": final_rules
        });

        config["geodata-mode"] = json!(true);
        config["geo-auto-update"] = json!(false);
        config["geo-update-interval"] = json!(24);
        config["geox-url"] = json!({
            "geoip": "https://fastly.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/geoip.dat",
            "geosite": "https://fastly.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/geosite.dat"
        });

        if plan.inbounds.mode.has_tun() {
            let mut dns_hijack = plan.inbounds.tun.dns_hijack.clone();
            if !dns_hijack.iter().any(|h| h.contains("tcp://")) {
                dns_hijack.push("tcp://any:53".into());
            }
            if plan.dns.ipv6 {
                if !dns_hijack.iter().any(|h| h.contains("[::]:53")) {
                    dns_hijack.push("tcp://[::]:53".into());
                    dns_hijack.push("udp://[::]:53".into());
                }
            }
            config["tun"] = json!({
                "enable": true,
                "stack": plan.inbounds.tun.stack.as_mihomo_value(),
                "dns-hijack": dns_hijack,
                "auto-route": plan.inbounds.tun.auto_route,
                "auto-detect-interface": plan.inbounds.tun.auto_detect_interface,
                "strict-route": true,
                "endpoint-independent-nat": true
            });
        }

        if !port_listeners.is_empty() {
            config["listeners"] = Value::Array(port_listeners);
        }

        // Clash Party 规范 YAML 深度覆写（支持简单值覆盖、对象递归、! 强制覆盖、+key 前插、key+ 后追加与 <> 转义）
        if let Some(ref override_text) = plan.outbounds.override_yaml {
            if !override_text.trim().is_empty() {
                let mut patch = serde_yaml::from_str::<Value>(override_text)
                    .context("Mihomo YAML 覆写解析失败")?;
                if let Some(groups) = patch.as_array() {
                    if !groups.iter().all(Value::is_object) {
                        return Err(anyhow!("Mihomo YAML 覆写的策略组数组只能包含对象"));
                    }
                    patch = json!({ "+proxy-groups": patch });
                }
                let patch_obj = patch
                    .as_object_mut()
                    .ok_or_else(|| anyhow!("Mihomo YAML 覆写必须是配置对象或策略组数组"))?;
                for (k, v) in patch_obj.iter_mut() {
                    let parsed = crate::proxy::yaml_override::parse_override_key(k);
                    if parsed.name == "proxy-groups" {
                        if let Some(groups_arr) = v.as_array_mut() {
                            for g in groups_arr.iter_mut() {
                                expand_proxy_group_filters(g, &candidate_names);
                            }
                        }
                    }
                }
                crate::proxy::yaml_override::deep_merge_value(&mut config, patch);
            }
        }

        // Rhai 脚本动态覆写（编程式最终裁决：传入已完成 YAML 覆写的完整 config，执行 main(config) 或扁平语句）
        if let Some(ref script_text) = plan.outbounds.override_script {
            if !script_text.trim().is_empty() {
                crate::proxy::script_override::apply_script_override(&mut config, script_text)
                    .context("Mihomo Rhai 覆写失败")?;
                if !config.is_object() {
                    return Err(anyhow!("Mihomo Rhai 覆写必须返回配置对象"));
                }
            }
        }

        // A YAML/Rhai override may change the current Fake-IP pool. Recompute
        // only our untouched default filter; a user-supplied filter stays final.
        if config["sniffer"]["skip-dst-address"] == default_sniffer_skip {
            if let Some(pool) = config["dns"]["fake-ip-range"].as_str() {
                config["sniffer"]["skip-dst-address"] =
                    json!(fake_ip_recovery_skip_destinations(pool));
            }
        }

        Ok(CompiledArtifact::Json(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Node;
    use crate::proxy::ir::lower_options_to_ir;
    use crate::proxy::{MihomoConfigOptions, ProxyMode};

    fn skips_destination(config: &Value, address: &str) -> bool {
        let address = address.parse::<std::net::IpAddr>().unwrap();
        config["sniffer"]["skip-dst-address"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| {
                let (base, prefix) = value.as_str().unwrap().split_once('/').unwrap();
                let prefix = prefix.parse::<u32>().unwrap();
                match (base.parse::<std::net::IpAddr>().unwrap(), address) {
                    (std::net::IpAddr::V4(base), std::net::IpAddr::V4(address)) => {
                        let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
                        u32::from(base) & mask == u32::from(address) & mask
                    }
                    (std::net::IpAddr::V6(base), std::net::IpAddr::V6(address)) => {
                        let mask = u128::MAX.checked_shl(128 - prefix).unwrap_or(0);
                        u128::from(base) & mask == u128::from(address) & mask
                    }
                    _ => false,
                }
            })
    }

    fn compile_result(options: MihomoConfigOptions) -> Result<Value> {
        let now = chrono::Utc::now();
        let node = Node {
            id: "fake-ip-recovery-fixture".into(),
            source_id: None,
            protocol: "ss".into(),
            name: "test-node".into(),
            server: "203.0.113.1".into(),
            port: 8388,
            raw: String::new(),
            config: json!({
                "server": "203.0.113.1",
                "port": 8388,
                "cipher": "aes-128-gcm",
                "password": "test-only"
            }),
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: now,
            updated_at: now,
        };
        let plan = lower_options_to_ir(&options, Some(&node));
        MihomoCompiler::new().compile(&plan)?.as_json_value()
    }

    fn compile(options: MihomoConfigOptions) -> Value {
        compile_result(options).unwrap()
    }

    #[test]
    fn invalid_yaml_override_returns_parse_error_instead_of_base_artifact() {
        let error = compile_result(MihomoConfigOptions {
            override_yaml: Some("sniffer: [\n".into()),
            ..MihomoConfigOptions::default()
        })
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Mihomo YAML 覆写解析失败"));
        assert!(
            message.contains("line"),
            "must preserve YAML error location: {message}"
        );
    }

    #[test]
    fn unsupported_yaml_override_structure_does_not_produce_artifact() {
        for yaml in ["42", "true", "null", "a scalar", "- 42\n", "- null\n"] {
            let error = compile_result(MihomoConfigOptions {
                override_yaml: Some(yaml.into()),
                ..MihomoConfigOptions::default()
            })
            .unwrap_err();
            assert!(format!("{error:#}").contains("Mihomo YAML 覆写"), "{yaml}");
        }
    }

    #[test]
    fn yaml_override_keeps_supported_group_arrays_and_blank_overrides() {
        let base = compile(MihomoConfigOptions::default());
        let blank = compile(MihomoConfigOptions {
            override_yaml: Some(" \n\t".into()),
            override_script: Some(" \n\t".into()),
            ..MihomoConfigOptions::default()
        });
        assert_eq!(blank, base);
        let empty_groups = compile(MihomoConfigOptions {
            override_yaml: Some("[]".into()),
            ..MihomoConfigOptions::default()
        });
        assert_eq!(empty_groups, base);
        let groups = compile(MihomoConfigOptions {
            override_yaml: Some(
                "- name: custom-fixture\n  type: select\n  proxies: [DIRECT]\n".into(),
            ),
            ..MihomoConfigOptions::default()
        });
        assert_eq!(groups["proxy-groups"][0]["name"], "custom-fixture");
        assert_eq!(
            groups["proxy-groups"].as_array().unwrap().len(),
            base["proxy-groups"].as_array().unwrap().len() + 1
        );
    }

    #[test]
    fn invalid_script_syntax_returns_error_after_yaml_without_base_artifact() {
        let error = compile_result(MihomoConfigOptions {
            override_yaml: Some("sniffer:\n  enable: false\n".into()),
            override_script: Some("fn main(config) { let broken = ; }".into()),
            ..MihomoConfigOptions::default()
        })
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Mihomo Rhai 覆写失败"));
        assert!(message.contains("Rhai 脚本编译失败"));
    }

    #[test]
    fn script_throw_after_mutation_aborts_compilation_and_preserves_input() {
        for script in [
            r#"fn main(config) {
                config.sniffer.enable = false;
                config.rules = [];
                throw "fixture failure after mutation";
                return config;
            }"#,
            r#"config.sniffer.enable = false;
                config.rules = [];
                throw "fixture failure after mutation";"#,
        ] {
            let mut options = MihomoConfigOptions {
                override_yaml: Some("dns:\n  fake-ip-range: 203.0.113.0/24\n".into()),
                ..MihomoConfigOptions::default()
            };
            let before = compile(options.clone());
            options.override_script = Some(script.into());
            let error = compile_result(options.clone()).unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("Mihomo Rhai 覆写失败"));
            assert!(message.contains("fixture failure after mutation"));
            options.override_script = None;
            assert_eq!(compile(options), before);
        }
    }

    #[test]
    fn script_invalid_return_shape_returns_error_without_indexing_panic() {
        for script in [
            "fn main(config) { return 42; }",
            "fn main(config) { return (); }",
            "fn main(config) { return []; }",
            "fn main(config) { return false; }",
            "config = [];",
        ] {
            let error = compile_result(MihomoConfigOptions {
                override_script: Some(script.into()),
                ..MihomoConfigOptions::default()
            })
            .unwrap_err();
            assert!(
                format!("{error:#}").contains("Mihomo Rhai 覆写必须返回配置对象"),
                "{script}: {error:#}"
            );
        }
    }

    #[test]
    fn fake_ip_recovery_defaults_persist_and_limit_tcp_sniffing_ports() {
        for mode in [ProxyMode::System, ProxyMode::Tun] {
            let config = compile(MihomoConfigOptions {
                mode,
                ..MihomoConfigOptions::default()
            });

            assert_eq!(config["profile"]["store-fake-ip"], true);
            let sniffer = &config["sniffer"];
            assert_eq!(sniffer["enable"], true);
            assert_eq!(sniffer["force-dns-mapping"], true);
            assert_eq!(sniffer["parse-pure-ip"], true);
            assert_eq!(sniffer["override-destination"], true);
            // HTTP Host or TLS SNI must not redirect an application's chosen
            // public/private endpoint. Only stale Fake-IPs need replacement.
            for address in [
                "1.1.1.1",
                "8.8.8.8",
                "203.0.113.25",
                "127.0.0.1",
                "10.1.2.3",
                "172.16.1.2",
                "192.168.1.2",
                "169.254.1.2",
                "100.64.1.2",
                "::1",
                "fc00::1",
                "fe80::1",
                "2001:db8::1",
                "2606:4700:4700::1111",
            ] {
                assert!(
                    skips_destination(&config, address),
                    "must preserve {address}"
                );
            }
            for address in [
                "198.18.0.1",
                "198.18.255.255",
                "198.19.0.1",
                "198.19.255.255",
            ] {
                assert!(
                    !skips_destination(&config, address),
                    "must recover {address}"
                );
            }
            // Explicit protocol/port lists must not expand to QUIC or all ports.
            assert_eq!(
                sniffer["sniff"],
                json!({
                    "HTTP": { "ports": [80, 8080] },
                    "TLS": { "ports": [443, 8443] }
                })
            );
        }
    }

    #[test]
    fn fake_ip_recovery_filter_covers_every_ipv4_outside_cached_pools() {
        let config = compile(MihomoConfigOptions::default());
        let mut intervals: Vec<_> = config["sniffer"]["skip-dst-address"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|cidr| ipv4_cidr_interval(cidr.as_str().unwrap()))
            .collect();
        intervals.sort_unstable();
        let skipped_addresses: u64 = intervals.iter().map(|(start, end)| end - start).sum();
        assert_eq!(skipped_addresses, (1_u64 << 32) - (1 << 17));
        for pair in intervals.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "skip ranges must not overlap");
        }
        // Check both sides of the only hole in the address space.
        for address in ["0.0.0.0", "198.17.255.255", "198.20.0.0", "255.255.255.255"] {
            assert!(skips_destination(&config, address));
        }
    }

    #[test]
    fn fake_ip_recovery_filter_follows_yaml_custom_pool_and_preserves_old_pools() {
        let config = compile(MihomoConfigOptions {
            override_yaml: Some("dns:\n  fake-ip-range: 203.0.113.17/24\n".into()),
            ..MihomoConfigOptions::default()
        });
        for address in [
            "198.18.0.1",
            "198.19.255.255",
            "203.0.113.0",
            "203.0.113.255",
        ] {
            assert!(
                !skips_destination(&config, address),
                "must recover {address}"
            );
        }
        for address in ["203.0.112.255", "203.0.114.0", "8.8.8.8", "192.168.1.2"] {
            assert!(
                skips_destination(&config, address),
                "must preserve {address}"
            );
        }
    }

    #[test]
    fn fake_ip_recovery_filter_follows_script_custom_pool_after_yaml() {
        let config = compile(MihomoConfigOptions {
            override_yaml: Some("dns:\n  fake-ip-range: 203.0.113.17/24\n".into()),
            override_script: Some(
                r#"fn main(config) {
                    config.dns["fake-ip-range"] = "192.0.2.9/25";
                    return config;
                }"#
                .into(),
            ),
            ..MihomoConfigOptions::default()
        });
        for address in ["198.18.0.1", "198.19.0.1", "192.0.2.0", "192.0.2.127"] {
            assert!(
                !skips_destination(&config, address),
                "must recover {address}"
            );
        }
        for address in ["192.0.1.255", "192.0.2.128", "203.0.113.17"] {
            assert!(
                skips_destination(&config, address),
                "must preserve {address}"
            );
        }
    }

    #[test]
    fn fake_ip_recovery_filter_preserves_explicit_yaml_and_script_filters() {
        let config = compile(MihomoConfigOptions {
            override_yaml: Some(
                "dns:\n  fake-ip-range: 203.0.113.17/24\nsniffer:\n  skip-dst-address: [0.0.0.0/0, '::/0']\n".into(),
            ),
            ..MihomoConfigOptions::default()
        });
        assert_eq!(
            config["sniffer"]["skip-dst-address"],
            json!(["0.0.0.0/0", "::/0"])
        );
        assert!(skips_destination(&config, "203.0.113.17"));
        assert!(skips_destination(&config, "198.18.0.1"));

        let config = compile(MihomoConfigOptions {
            override_yaml: Some("sniffer:\n  skip-dst-address: [0.0.0.0/0]\n".into()),
            override_script: Some(
                r#"fn main(config) {
                    config.dns["fake-ip-range"] = "203.0.113.17/24";
                    config.sniffer["skip-dst-address"] = ["10.0.0.0/8"];
                    return config;
                }"#
                .into(),
            ),
            ..MihomoConfigOptions::default()
        });
        assert_eq!(config["sniffer"]["skip-dst-address"], json!(["10.0.0.0/8"]));
    }

    #[test]
    fn fake_ip_recovery_filter_handles_custom_pool_boundaries_and_local_overlap() {
        for (pool, allowed, skipped) in [
            (
                "203.0.113.9/32",
                vec!["203.0.113.9"],
                vec!["203.0.113.8", "203.0.113.10"],
            ),
            (
                "198.18.64.1/18",
                vec!["198.18.0.1", "198.19.255.255"],
                vec!["198.20.0.0"],
            ),
            (
                "0.0.0.0/0",
                vec!["0.0.0.0", "255.255.255.255"],
                vec!["10.1.2.3", "192.168.1.2"],
            ),
            (
                "10.0.0.1/8",
                vec!["198.18.0.1"],
                vec!["10.0.0.1", "10.255.255.255"],
            ),
            (
                "invalid",
                vec!["198.18.0.1", "198.19.0.1"],
                vec!["203.0.113.9"],
            ),
            ("203.0.113.9/33", vec!["198.18.0.1"], vec!["203.0.113.9"]),
        ] {
            let config =
                json!({"sniffer": {"skip-dst-address": fake_ip_recovery_skip_destinations(pool)}});
            for address in allowed {
                assert!(
                    !skips_destination(&config, address),
                    "pool={pool} must recover {address}"
                );
            }
            for address in skipped {
                assert!(
                    skips_destination(&config, address),
                    "pool={pool} must preserve {address}"
                );
            }
        }
    }

    #[test]
    fn fake_ip_recovery_default_pool_matches_target_platform() {
        let expected = if cfg!(target_os = "macos") {
            "198.19.0.1/16"
        } else {
            "198.18.0.1/16"
        };
        for mode in [ProxyMode::System, ProxyMode::Tun] {
            let config = compile(MihomoConfigOptions {
                mode,
                ..MihomoConfigOptions::default()
            });
            assert_eq!(config["dns"]["fake-ip-range"], expected);
        }
    }

    #[test]
    fn fake_ip_recovery_yaml_override_can_disable_defaults_without_reordering_rules() {
        let config = compile(MihomoConfigOptions {
            override_yaml: Some(
                r#"
profile:
  store-fake-ip: false
dns:
  fake-ip-range: 198.18.64.1/18
sniffer:
  enable: false
  sniff:
    TLS:
      ports: [9443]
rules:
  - DOMAIN,yaml-first.example.com,DIRECT
  - MATCH,PROXY
"#
                .into(),
            ),
            ..MihomoConfigOptions::default()
        });

        assert_eq!(config["profile"]["store-fake-ip"], false);
        assert_eq!(config["dns"]["fake-ip-range"], "198.18.64.1/18");
        assert_eq!(config["sniffer"]["enable"], false);
        assert_eq!(config["sniffer"]["sniff"]["TLS"]["ports"], json!([9443]));
        assert_eq!(
            config["rules"],
            json!(["DOMAIN,yaml-first.example.com,DIRECT", "MATCH,PROXY"])
        );
    }

    #[test]
    fn fake_ip_recovery_script_override_keeps_final_priority() {
        let config = compile(MihomoConfigOptions {
            override_yaml: Some(
                r#"
profile:
  store-fake-ip: true
dns:
  fake-ip-range: 198.18.64.1/18
sniffer:
  enable: true
rules:
  - DOMAIN,yaml-first.example.com,DIRECT
  - MATCH,PROXY
"#
                .into(),
            ),
            override_script: Some(
                r#"
fn main(config) {
    config["profile"]["store-fake-ip"] = false;
    config.dns["fake-ip-range"] = "198.18.128.1/17";
    config.sniffer.enable = false;
    config.rules.insert(0, "DOMAIN,script-first.example.com,PROXY");
    return config;
}
"#
                .into(),
            ),
            ..MihomoConfigOptions::default()
        });

        assert_eq!(config["profile"]["store-fake-ip"], false);
        assert_eq!(config["dns"]["fake-ip-range"], "198.18.128.1/17");
        assert_eq!(config["sniffer"]["enable"], false);
        assert_eq!(
            config["rules"],
            json!([
                "DOMAIN,script-first.example.com,PROXY",
                "DOMAIN,yaml-first.example.com,DIRECT",
                "MATCH,PROXY"
            ])
        );
    }
}

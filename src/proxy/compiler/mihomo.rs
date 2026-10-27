use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::export::nodes_to_clash_proxies;
use crate::kernel::KernelKind;
use crate::proxy::ir::{IRAction, IRMatcher, RoutingPlan};
use crate::proxy::types::AutoRoutingStrategy;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const URL_TEST_GROUP: &str = "自动选择";
pub const FINAL_GROUP: &str = "漏网之鱼";
pub const URL_TEST_INTERVAL_SECS: u64 = 60;
pub const URL_TEST_TOLERANCE_MS: u64 = 30;
pub const URL_TEST_TIMEOUT_MS: u64 = 3000;

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
    // 国内直连
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
            override_text.contains("proxy-groups:") || override_text.contains("proxy-groups :")
        } else {
            false
        };
        if plan.outbounds.url_test || has_override_groups {
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
                            "vless" | "vmess" | "trojan" | "hysteria2" | "tuic" | "ssr"
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
            if plan.outbounds.smart_routing {
                for r_name in &regional_group_names {
                    if !proxy_members.contains(r_name) {
                        proxy_members.push(r_name.clone());
                    }
                }
                proxy_members.push("DIRECT".to_string());
            }
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
            let mut proxy_members = Vec::new();
            if plan.outbounds.smart_routing {
                proxy_members.extend(regional_group_names.iter().cloned());
                proxy_members.push("DIRECT".to_string());
            }
            proxy_members.extend(candidate_names.iter().cloned());
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
                        format!("GEOSITE,{},{}", crate::geo::normalize_geo_code(s), target)
                    }
                    IRMatcher::GeoIp(c) => {
                        let c = crate::geo::normalize_geo_code(c);
                        if ir_rule.no_resolve {
                            format!("GEOIP,{c},{target},no-resolve")
                        } else {
                            format!("GEOIP,{c},{target}")
                        }
                    }
                    IRMatcher::ProcessName(p) => format!("PROCESS-NAME,{p},{target}"),
                    IRMatcher::RuleSet(s) => {
                        let lower = s.to_ascii_lowercase();
                        match lower.as_str() {
                            "reject" | "reject-list" | "ad" | "ads" | "advertising" => {
                                format!("GEOSITE,category-ads-all,{target}")
                            }
                            "direct" | "direct-list" | "cn" | "china" => {
                                format!("GEOSITE,cn,{target}")
                            }
                            "proxy" | "proxy-list" => {
                                format!("GEOSITE,geolocation-!cn,{target}")
                            }
                            "gfw" | "gfwlist" => {
                                format!("GEOSITE,gfw,{target}")
                            }
                            "cncidr" => {
                                format!("GEOIP,CN,{target}")
                            }
                            "telegramcidr" => {
                                format!("GEOIP,telegram,{target}")
                            }
                            "lancidr" | "private" => {
                                format!("GEOSITE,private,{target}")
                            }
                            other => {
                                format!("GEOSITE,{other},{target}")
                            }
                        }
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

        let mut config = json!({
            "mixed-port": plan.inbounds.mixed_port,
            "external-controller": format!("127.0.0.1:{}", plan.inbounds.controller_port),
            "secret": plan.inbounds.controller_secret,
            "allow-lan": plan.inbounds.allow_lan,
            "mode": "rule",
            "log-level": "info",
            "proxies": candidate_proxies,
            "proxy-groups": proxy_groups,
            "dns": {
                "enable": true,
                "ipv6": false,
                "enhanced-mode": plan.dns.enhanced_mode.as_mihomo_value(),
                "fake-ip-range": plan.dns.fake_ip_range,
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

        Ok(CompiledArtifact::Json(config))
    }
}

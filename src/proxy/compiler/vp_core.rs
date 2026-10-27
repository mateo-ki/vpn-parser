use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::kernel::KernelKind;
use crate::proxy::ir::RoutingPlan;
use anyhow::Result;

/// VP_Core（https://github.com/mateo-ki/vp_core）——小而快、纯 Rust 实现的全协议代理与 VPN 内核。
///
/// 具备极致轻量（Release 单文件 ~4MB）、极低常驻内存（10~13MB）、纯静态 Musl、全协议覆盖与原生 TUN 架构。
/// 配置面与 REST 控制器 API 全面兼容 Clash / Mihomo 体系，因此编译主体复用 [`super::MihomoCompiler`]，
/// 并按 VP_Core 原生规范做结构裁剪与优化：
/// - 剥离外部 Geo 下载键（geodata-mode/geo-auto-update/geox-url 等）；
/// - TUN 模式下输出 vp_core 原生 tun 块（enable/address/netmask/mtu/fake-ip-range/auto-route）；
/// - 输出为规范 YAML 文本供 `vpcore run -c` 与 `vpcore validate -c` 解析。
#[derive(Debug, Default)]
pub struct VpCoreCompiler;

impl VpCoreCompiler {
    pub fn new() -> Self {
        Self
    }
}

impl BackendCompiler for VpCoreCompiler {
    fn kernel(&self) -> KernelKind {
        KernelKind::VpCore
    }

    fn capabilities(&self) -> &CompilerCapabilities {
        const CAPS: CompilerCapabilities = CompilerCapabilities {
            supports_tun: true,
            supports_process_name: true,
            supports_domain_keyword: true,
            supports_proxy_groups: true,
            supports_url_test: true,
            supported_geosite_categories: None,
            supported_geoip_countries: None,
        };
        &CAPS
    }

    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact> {
        let artifact = super::MihomoCompiler::new().compile(plan)?;
        let mut config = artifact.as_json_value()?;

        if let Some(obj) = config.as_object_mut() {
            // 过滤 VP-Core 不支持的节点（一个坏节点会让 vpcore 解析配置失败、整进程退出）：
            // 未知类型与 xhttp/httpupgrade/h2 等传输层一律剔除，并同步修剪 proxy-groups 引用
            let mut dropped: Vec<String> = Vec::new();
            if let Some(proxies) = obj.get_mut("proxies").and_then(|p| p.as_array_mut()) {
                for p in proxies.iter_mut() {
                    // "raw" 是部分订阅源的 tcp 别名，vp-core 不识别，归一成 tcp
                    if p.get("network").and_then(|v| v.as_str()) == Some("raw") {
                        if let Some(po) = p.as_object_mut() {
                            po.insert("network".into(), serde_json::json!("tcp"));
                        }
                    }
                }
                proxies.retain(|p| {
                    let keep = vpcore_supports_proxy(p);
                    if !keep {
                        if let Some(name) = p.get("name").and_then(|v| v.as_str()) {
                            dropped.push(name.to_string());
                        }
                    }
                    keep
                });
            }
            if !dropped.is_empty() {
                let dropped_set: std::collections::HashSet<&str> =
                    dropped.iter().map(String::as_str).collect();
                if let Some(groups) = obj.get_mut("proxy-groups").and_then(|g| g.as_array_mut()) {
                    for g in groups.iter_mut() {
                        if let Some(members) = g.get_mut("proxies").and_then(|m| m.as_array_mut()) {
                            members.retain(|m| {
                                m.as_str().map_or(true, |name| !dropped_set.contains(name))
                            });
                            if members.is_empty() {
                                members.push(serde_json::json!("DIRECT"));
                            }
                        }
                    }
                }
                tracing::info!(
                    count = dropped.len(),
                    "VP-Core 配置已剔除其不支持的节点（保持内核可启动）"
                );
            }

            // 清理特定于 Mihomo 扩展的 Geo 自动更新与进程探测专有字段
            for key in [
                "geodata-mode",
                "geo-auto-update",
                "geo-update-interval",
                "geox-url",
                "find-process-mode",
            ] {
                obj.remove(key);
            }

            // 配置针对 vp_core 的 Geo 自动更新与远程兜底地址
            obj.insert(
                "geosite-url".into(),
                serde_json::json!("https://fastly.jsdelivr.net/gh/Loyalsoldier/v2ray-rules-dat@release/geosite.dat"),
            );
            obj.insert(
                "geoip-url".into(),
                serde_json::json!(
                    "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/country.mmdb"
                ),
            );

            // TUN 模式：生成 vp_core 标准 TUN 字段与 Fake-IP DNS 联动
            if plan.inbounds.mode.has_tun() {
                obj.insert(
                    "tun".into(),
                    serde_json::json!({
                        "enable": true,
                        "address": "198.18.0.1",
                        "netmask": "255.255.0.0",
                        "mtu": 1500,
                        "fake-ip-range": "198.18.0.0/16",
                        "auto-route": plan.inbounds.tun.auto_route,
                    }),
                );
                if let Some(dns) = obj.get_mut("dns").and_then(|d| d.as_object_mut()) {
                    dns.insert("enhanced-mode".into(), serde_json::json!("fake-ip"));
                    dns.insert("fake-ip-range".into(), serde_json::json!("198.18.0.0/16"));
                }
            } else {
                obj.remove("tun");
            }

            // 过滤与平滑适配 vp_core 不支持的非国家代码 GEOIP 规则（如 GEOIP,google / GEOIP,telegram）
            if let Some(rules) = obj.get_mut("rules").and_then(|r| r.as_array_mut()) {
                let mut new_rules = Vec::with_capacity(rules.len());
                let mut existing_geosites = std::collections::HashSet::new();

                for r in rules.iter() {
                    if let Some(s) = r.as_str() {
                        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
                        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("GEOSITE") {
                            existing_geosites.insert(parts[1].to_ascii_lowercase());
                        }
                    }
                }

                for r in rules.drain(..) {
                    if let Some(s) = r.as_str() {
                        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
                        if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("GEOIP") {
                            let code = parts[1];
                            if is_valid_vpcore_geoip_code(code) {
                                new_rules.push(r);
                            } else {
                                let cat = code.to_ascii_lowercase();
                                if !existing_geosites.contains(&cat)
                                    && super::shoes::has_geosite_category(&cat)
                                {
                                    let target = parts[2];
                                    new_rules.push(serde_json::Value::String(format!(
                                        "GEOSITE,{cat},{target}"
                                    )));
                                    existing_geosites.insert(cat);
                                } else {
                                    tracing::warn!(
                                        code = %code,
                                        "VP-Core 的 GeoIP 数据库仅支持双字母国家码与 LAN/PRIVATE，已自动过滤/忽略非法规则: {s}"
                                    );
                                }
                            }
                        } else {
                            new_rules.push(r);
                        }
                    } else {
                        new_rules.push(r);
                    }
                }
                *rules = new_rules;
            }
        }

        let yaml = serde_yaml::to_string(&config)?;
        Ok(CompiledArtifact::Yaml(yaml))
    }
}

pub(crate) fn is_valid_vpcore_geoip_code(code: &str) -> bool {
    let code = code.trim();
    if code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    code.eq_ignore_ascii_case("LAN") || code.eq_ignore_ascii_case("PRIVATE")
}

/// VP-Core 可运行的节点类型与传输层白名单（内核遇到未知类型/传输会整进程拒绝启动）
fn vpcore_supports_proxy(p: &serde_json::Value) -> bool {
    let ty = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let type_ok = matches!(
        ty,
        "ss" | "ssr"
            | "vmess"
            | "vless"
            | "trojan"
            | "anytls"
            | "hysteria2"
            | "hy2"
            | "tuic"
            | "http"
            | "https"
            | "socks5"
            | "socks"
            | "snell"
            | "ssh"
            | "brook"
            | "mieru"
            | "naive"
            | "naiveproxy"
            | "wireguard"
    );
    if !type_ok {
        return false;
    }
    let net = p
        .get("network")
        .and_then(|v| v.as_str())
        .unwrap_or("tcp")
        .trim();
    matches!(net, "" | "tcp" | "ws" | "grpc" | "http")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Node;
    use crate::proxy::{MihomoConfigOptions, ProxyMode};
    use serde_json::json;

    fn test_node(proto: &str, config: serde_json::Value) -> Node {
        Node {
            id: "test-node-1".into(),
            source_id: None,
            name: "Test Node".into(),
            protocol: proto.into(),
            server: "1.2.3.4".into(),
            port: 443,
            config,
            raw: String::new(),
            country: Some("US".into()),
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_vpcore_compiler_basic_structure() {
        let node = test_node(
            "ss",
            json!({
                "cipher": "aes-128-gcm",
                "password": "pwd"
            }),
        );
        let options = MihomoConfigOptions {
            mixed_port: 7890,
            controller_port: 9090,
            controller_secret: "sec".into(),
            ..Default::default()
        };
        let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(&node));
        let artifact = VpCoreCompiler.compile(&plan).expect("compile success");
        let yaml = match artifact {
            CompiledArtifact::Yaml(y) => y,
            _ => panic!("expected Yaml artifact"),
        };

        assert!(yaml.contains("mixed-port: 7890"));
        assert!(
            yaml.contains("external-controller: 127.0.0.1:9090") || yaml.contains("127.0.0.1:9090")
        );
        assert!(yaml.contains("proxies:"));
        assert!(yaml.contains("proxy-groups:"));
        assert!(yaml.contains("rules:"));
        assert!(!yaml.contains("geox-url:"));
        assert!(yaml.contains("geosite-url:"));
        assert!(yaml.contains("geoip-url:"));
    }

    #[test]
    fn test_vpcore_compiler_tun_mode() {
        let node = test_node(
            "vless",
            json!({
                "uuid": "aaaaaaaa-bbbb-4ccc-8ddd-000000000040",
                "tls": true,
                "flow": "xtls-rprx-vision",
                "network": "tcp"
            }),
        );
        let mut options = MihomoConfigOptions::default();
        options.mode = ProxyMode::Tun;
        let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(&node));
        let artifact = VpCoreCompiler.compile(&plan).expect("compile success");
        let yaml = match artifact {
            CompiledArtifact::Yaml(y) => y,
            _ => panic!("expected Yaml artifact"),
        };

        assert!(yaml.contains("tun:"));
        assert!(yaml.contains("enable: true"));
        assert!(yaml.contains("198.18.0.1"));
        assert!(yaml.contains("auto-route: true"));
    }

    #[test]
    fn test_vpcore_compiler_filters_invalid_geoip_rules() {
        assert!(is_valid_vpcore_geoip_code("CN"));
        assert!(is_valid_vpcore_geoip_code("us"));
        assert!(is_valid_vpcore_geoip_code("LAN"));
        assert!(is_valid_vpcore_geoip_code("PRIVATE"));
        assert!(!is_valid_vpcore_geoip_code("google"));
        assert!(!is_valid_vpcore_geoip_code("GOOGLE"));
        assert!(!is_valid_vpcore_geoip_code("telegram"));
        assert!(!is_valid_vpcore_geoip_code("CN-CIDR"));

        let node = test_node(
            "ss",
            json!({
                "cipher": "aes-128-gcm",
                "password": "pwd"
            }),
        );
        let mut options = MihomoConfigOptions::default();
        options.override_yaml = Some(
            r#"
rules:
  - GEOSITE,google,美国节点
  - GEOIP,google,美国节点,no-resolve
  - GEOIP,CN,DIRECT,no-resolve
"#
            .into(),
        );
        let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(&node));
        let artifact = VpCoreCompiler.compile(&plan).expect("compile success");
        let yaml = match artifact {
            CompiledArtifact::Yaml(y) => y,
            _ => panic!("expected Yaml artifact"),
        };

        assert!(yaml.contains("GEOIP,CN,DIRECT,no-resolve") || yaml.contains("GEOIP,CN,DIRECT"));
        assert!(!yaml.contains("GEOIP,google"));
        assert!(yaml.contains("GEOSITE,google"));
    }

    #[test]
    fn test_vpcore_compiler_drops_unsupported_nodes_and_prunes_groups() {
        let node = test_node(
            "vless",
            json!({"uuid": "aaaaaaaa-bbbb-4ccc-8ddd-000000000050"}),
        );
        let mut options = MihomoConfigOptions::default();
        options.url_test = true;
        options.url_test_size = 4;
        options.url_test_nodes = vec![
            json!({
                "name": "good-ws", "type": "vless", "server": "1.2.3.4", "port": 443,
                "uuid": "aaaaaaaa-bbbb-4ccc-8ddd-000000000051", "network": "ws"
            }),
            json!({
                "name": "bad-xhttp", "type": "vless", "server": "1.2.3.4", "port": 443,
                "uuid": "aaaaaaaa-bbbb-4ccc-8ddd-000000000052", "network": "xhttp"
            }),
            json!({
                "name": "raw-alias", "type": "vmess", "server": "1.2.3.4", "port": 80,
                "uuid": "aaaaaaaa-bbbb-4ccc-8ddd-000000000053", "alterId": "0",
                "cipher": "auto", "network": "raw"
            }),
        ];
        let plan = crate::proxy::ir::lower_options_to_ir(&options, Some(&node));
        let artifact = VpCoreCompiler.compile(&plan).expect("compile success");
        let yaml = match artifact {
            CompiledArtifact::Yaml(y) => y,
            _ => panic!("expected Yaml artifact"),
        };

        assert!(yaml.contains("good-ws"), "ws 节点应保留:\n{yaml}");
        assert!(!yaml.contains("bad-xhttp"), "xhttp 节点必须剔除:\n{yaml}");
        assert!(
            yaml.contains("raw-alias"),
            "raw 传输应归一为 tcp 后保留:\n{yaml}"
        );
        assert!(
            !yaml.contains("network: xhttp"),
            "不允许残留 xhttp 传输:\n{yaml}"
        );
        assert!(
            !yaml.contains("network: raw"),
            "raw 必须归一成 tcp:\n{yaml}"
        );
    }

    #[test]
    fn test_vpcore_supports_proxy_whitelist() {
        use serde_json::json;
        let ok = |mut v: serde_json::Value| {
            v["name"] = json!("t");
            vpcore_supports_proxy(&v)
        };
        assert!(ok(json!({"type": "vless"})));
        assert!(ok(json!({"type": "vmess", "network": "ws"})));
        assert!(ok(json!({"type": "vmess", "network": "http"})));
        assert!(ok(json!({"type": "ss"})));
        assert!(ok(json!({"type": "trojan", "network": "grpc"})));
        assert!(!ok(json!({"type": "vless", "network": "xhttp"})));
        assert!(!ok(json!({"type": "vless", "network": "httpupgrade"})));
        assert!(!ok(json!({"type": "vless", "network": "h2"})));
        assert!(!ok(json!({"type": "masque"})));
        assert!(!ok(json!({"type": "hysteria"})));
    }

    #[test]
    fn test_vpcore_incompatibility_detection() {
        use crate::kernel::is_node_incompatible_with_vpcore;
        let mk = |proto: &str, config: serde_json::Value, raw: &str| Node {
            id: "t".into(),
            source_id: None,
            name: "t".into(),
            protocol: proto.into(),
            server: "1.2.3.4".into(),
            port: 443,
            raw: raw.into(),
            config,
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        assert!(
            is_node_incompatible_with_vpcore(&mk("vless", json!({"network": "xhttp"}), ""))
                .is_some()
        );
        assert!(is_node_incompatible_with_vpcore(&mk(
            "vmess",
            json!({"network": "httpupgrade"}),
            ""
        ))
        .is_some());
        assert!(is_node_incompatible_with_vpcore(&mk(
            "vless",
            json!({}),
            "vless://a@1.2.3.4:443?network=xhttp#x"
        ))
        .is_some());
        assert!(is_node_incompatible_with_vpcore(&mk("masque", json!({}), "")).is_some());
        // http / ws / tcp 传输自 HTTP 伪装传输落地后均受支持
        assert!(
            is_node_incompatible_with_vpcore(&mk("vless", json!({"network": "http"}), ""))
                .is_none()
        );
        assert!(
            is_node_incompatible_with_vpcore(&mk("vless", json!({"network": "ws"}), "")).is_none()
        );
        assert!(is_node_incompatible_with_vpcore(&mk("ss", json!({}), "")).is_none());
    }
}

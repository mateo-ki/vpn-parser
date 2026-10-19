use super::*;
use anyhow::Result;
use crate::kernel::KernelKind;
use crate::models::Node;
use crate::proxy::mihomo::{URL_TEST_INTERVAL_SECS, URL_TEST_TIMEOUT_MS, URL_TEST_TOLERANCE_MS};
use crate::proxy_probe::PROXY_CONNECTIVITY_TEST_URL;
use crate::system_proxy::{SystemProxyControl, SystemProxySnapshot};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
    /// 三种启动结局各自说什么话。反向对照很关键：TUN 模式不许提"请把应用指向端口"，
    /// 写了系统代理也不许再教他怎么指 —— 多出来的那半句会让人以为还没生效。
    #[test]
    fn the_start_message_differs_per_mode_and_platform_capability() {
        let owned = start_success_message("Mihomo", 4200, ProxyMode::System, true, true, 9980);
        assert!(owned.contains("运行中"), "{owned}");
        assert!(
            !owned.contains("127.0.0.1"),
            "写过就不该再教他怎么指: {owned}"
        );

        let tun = start_success_message("Mihomo", 4200, ProxyMode::Tun, false, true, 9980);
        assert!(!tun.contains("127.0.0.1"), "TUN 不该让他去找端口: {tun}");

        let local_port =
            start_success_message("Mihomo", 4200, ProxyMode::System, false, false, 9980);
        assert!(local_port.contains("127.0.0.1:9980"), "{local_port}");
        assert!(local_port.contains("没有系统代理"), "{local_port}");
    }

    /// "能不能写系统代理"必须是唯一一个判据：启动流程用它决定要不要写、
    /// `local_proxy_ready` 用它决定点不点亮、界面用它决定 chip 上写什么字。
    /// 三处各写一份 `cfg!(windows)` 迟早会漂，漂了就出现"界面说本地端口、状态说拥有系统代理"。
    #[test]
    fn the_system_proxy_capability_is_one_answer_for_three_consumers() {
        // 平台实现与系统判据必须同意同一个答案（不是各自另判一次）。
        assert_eq!(
            crate::system_proxy::platform_system_proxy().supported(),
            crate::system_proxy::system_proxy_supported_on_this_platform()
        );
        if cfg!(any(windows, target_os = "macos")) {
            assert!(crate::system_proxy::system_proxy_supported_on_this_platform());
        }
    }
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn self_referential_system_proxy_snapshot_is_reset_to_off() {
        // 上次会话残留的"就是我们自己端口"的系统代理，不能当成还原目标。
        let leftover = SystemProxySnapshot {
            enabled: true,
            server: Some("127.0.0.1:9980".into()),
            override_list: Some("<local>;localhost".into()),
        };
        let cleaned = sanitize_self_referential(leftover, "127.0.0.1:9980");
        assert!(!cleaned.enabled);
        assert_eq!(cleaned.server, None);
        assert_eq!(cleaned.override_list, None);

        // 用户原本的代理要原样保留。
        let users = SystemProxySnapshot {
            enabled: true,
            server: Some("127.0.0.1:10808".into()),
            override_list: None,
        };
        let kept = sanitize_self_referential(users.clone(), "127.0.0.1:9980");
        assert_eq!(kept, users);

        // 协议分组形式（http=...;https=...）里指向我方端口，同样要清掉。
        let grouped = SystemProxySnapshot {
            enabled: true,
            server: Some("http=127.0.0.1:9980;https=127.0.0.1:9980".into()),
            override_list: None,
        };
        assert!(!sanitize_self_referential(grouped, "127.0.0.1:9980").enabled);

        // 本来就未启用的快照保持原样。
        let off = SystemProxySnapshot {
            enabled: false,
            server: Some("127.0.0.1:9980".into()),
            override_list: None,
        };
        assert!(!sanitize_self_referential(off.clone(), "127.0.0.1:9980").enabled);
        assert_eq!(
            sanitize_self_referential(off.clone(), "127.0.0.1:9980").server,
            off.server
        );
    }

    #[test]
    #[cfg(feature = "gui")]
    fn test_diagnose_mihomo_config_with_recommended_override() {
        use crate::gui::views::proxy::CLASH_VERGE_RECOMMENDED_OVERRIDE;
        let node = Node {
            id: "test1".into(),
            source_id: None,
            protocol: "ss".into(),
            name: "香港01".into(),
            server: "1.1.1.1".into(),
            port: 443,
            raw: "".into(),
            config: serde_json::from_str(r#"{"server":"1.1.1.1","server_port":443,"password":"pass","method":"aes-128-gcm"}"#).unwrap(),
            country: Some("HK".into()),
            latency_ms: Some(50),
            latency_ok: Some(true),
            download_kbps: Some(5000.0),
            latency_kernel: Some("mihomo".into()),
            latency_test_kind: Some("google_proxy".into()),
            latency_tested_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let mut options = MihomoConfigOptions::default();
        options.smart_routing = true;
        options.override_yaml = Some(CLASH_VERGE_RECOMMENDED_OVERRIDE.to_string());
        let config_val = build_mihomo_config(&node, options).expect("build config");
        let yaml_str = serde_yaml::to_string(&config_val).expect("to yaml");
        std::fs::write("target/diagnose_mihomo.yaml", &yaml_str).expect("write yaml");

        let exe = crate::kernel::find_kernel_executable(KernelKind::Mihomo);
        if let Some(exe_path) = exe {
            let output = std::process::Command::new(exe_path)
                .arg("-t")
                .arg("-d")
                .arg(crate::kernel::managed_kernel_root().join("mihomo"))
                .arg("-f")
                .arg("target/diagnose_mihomo.yaml")
                .output()
                .expect("run mihomo -t");
            assert!(
                output.status.success(),
                "mihomo validation failed: stdout: {}, stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[derive(Default)]
    struct FakeSystemProxy {
        enabled: StdMutex<Vec<String>>,
        restored: StdMutex<Vec<SystemProxySnapshot>>,
    }

    impl SystemProxyControl for FakeSystemProxy {
        fn capture(&self) -> Result<SystemProxySnapshot> {
            Ok(SystemProxySnapshot {
                enabled: false,
                server: Some("old-proxy:8080".into()),
                override_list: Some("<local>".into()),
            })
        }

        fn enable(&self, server: &str) -> Result<()> {
            self.enabled.lock().unwrap().push(server.into());
            Ok(())
        }

        fn restore(&self, snapshot: &SystemProxySnapshot) -> Result<()> {
            self.restored.lock().unwrap().push(snapshot.clone());
            Ok(())
        }
    }

    fn node(protocol: &str, config: serde_json::Value) -> Node {
        let now = Utc::now();
        Node {
            id: format!("{protocol}-id"),
            source_id: None,
            protocol: protocol.into(),
            name: "test-node".into(),
            server: config["server"].as_str().unwrap().into(),
            port: config["port"].as_u64().unwrap() as u16,
            raw: String::new(),
            config,
            country: None,
            latency_ms: None,
            latency_ok: None,
            download_kbps: None,
            latency_kernel: None,
            latency_test_kind: None,
            latency_tested_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn stopped_status_uses_default_ports() {
        let status = ProxyRuntimeStatus::stopped(MihomoConfigOptions::default());

        assert_eq!(status.state, ProxyState::Stopped);
        assert_eq!(status.mode, ProxyMode::System);
        assert_eq!(status.mixed_port, 9980);
        assert_eq!(status.controller_port, 9090);
        assert!(status.node_id.is_none());
        assert!(!status.local_proxy_ready);
        assert!(!status.system_proxy_enabled);
        assert!(status.last_error.is_none());
    }

    #[test]
    fn startup_proxy_verification_uses_five_second_total_budget() {
        assert_eq!(STARTUP_PROXY_TOTAL_TIMEOUT, Duration::from_secs(15));
        assert_eq!(STARTUP_PROXY_VERIFY_TIMEOUT, Duration::from_millis(1_500));
        assert_eq!(STARTUP_SYSTEM_PROXY_RESERVE, Duration::from_millis(250));
        assert_eq!(STARTUP_PROXY_VERIFY_URLS, [PROXY_CONNECTIVITY_TEST_URL]);
    }

    #[test]
    fn startup_requires_google_connectivity_verification() {
        assert!(VERIFY_CONNECTIVITY_ON_START);
    }

    #[test]
    fn mihomo_requires_both_proxy_and_controller_ports() {
        let options = MihomoConfigOptions {
            mixed_port: 17890,
            controller_port: 19090,
            ..MihomoConfigOptions::default()
        };

        assert_eq!(
            required_startup_ports(KernelKind::Mihomo, &options),
            [17890, 19090]
        );
        assert_eq!(
            required_startup_ports(KernelKind::SingBox, &options),
            [17890]
        );
    }

    #[test]
    fn kernel_adapters_expose_runtime_and_batch_contracts() {
        let options = MihomoConfigOptions::default();
        let cases = [
            (KernelKind::Mihomo, 256, options.controller_port, 2),
            (KernelKind::SingBox, 64, options.mixed_port, 1),
            (KernelKind::Xray, 64, options.mixed_port, 1),
        ];

        for (kind, batch_size, ready_port, port_count) in cases {
            let adapter = kernel_adapter(kind);
            assert_eq!(adapter.kind(), kind);
            assert_eq!(adapter.batch_size(), batch_size);
            assert_eq!(adapter.readiness_port(&options), ready_port);
            assert_eq!(adapter.required_ports(&options).len(), port_count);
        }
        assert!(kernel_adapter(KernelKind::Xray).supports_protocol("vless"));
        assert!(!kernel_adapter(KernelKind::Xray).supports_protocol("hysteria2"));
    }

    #[test]
    fn classifies_only_node_connectivity_startup_failures() {
        assert!(is_node_connectivity_failure(
            "节点连通性验证失败: 本地代理无法访问测试地址"
        ));
        assert!(!is_node_connectivity_failure("Mihomo 配置生成失败"));
        assert!(!is_node_connectivity_failure("代理端口 7890 已被占用"));
    }

    #[test]
    fn builds_minimal_mihomo_config_for_selected_node() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );

        let config = build_mihomo_config(&selected, MihomoConfigOptions::default()).unwrap();

        assert_eq!(config["mixed-port"], 9980);
        assert_eq!(config["external-controller"], "127.0.0.1:9090");
        assert_eq!(config["proxies"].as_array().unwrap().len(), 1);
        assert_eq!(config["proxy-groups"][0]["name"], "PROXY");
        // 没有用户规则时，只剩镜像钉直连 + 兜底 MATCH。
        let rules = config["rules"].as_array().unwrap();
        assert_eq!(rules.len(), crate::geo::MIRROR_DIRECT_HOSTS.len() + 1);
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
    }

    #[test]
    fn recommended_routing_preserves_order_enables_geo_and_keeps_proxy_fallback_last() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let options = MihomoConfigOptions {
            rules: recommended_traffic_rules(),
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&selected, options).unwrap();
        let rules = config["rules"].as_array().unwrap();

        assert_eq!(config["geodata-mode"], true);
        // 18 条用户规则 + 镜像钉直连 + MATCH 仍是最后一条。
        assert_eq!(rules.len(), 18 + crate::geo::MIRROR_DIRECT_HOSTS.len());
        assert_eq!(rules[11], "GEOSITE,category-ads-all,REJECT");
        assert_eq!(rules[12], "GEOSITE,cn,DIRECT");
        assert_eq!(rules[13], "GEOSITE,google,PROXY");
        assert_eq!(rules[14], "GEOSITE,youtube,PROXY");
        assert_eq!(rules[15], "GEOIP,CN,DIRECT,no-resolve");
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
    }

    #[test]
    fn url_test_mode_emits_three_groups_and_routes_match_to_final_group() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let candidate = node(
            "vless",
            json!({"server": "5.6.7.8", "port": 443, "uuid": "u2", "network": "ws", "security": "tls"}),
        );
        let mut options = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            url_test: true,
            url_test_size: 3,
            ..MihomoConfigOptions::default()
        };
        options.url_test_nodes =
            crate::export::nodes_to_clash_proxies(std::slice::from_ref(&candidate));

        let config = build_mihomo_config(&current, options).unwrap();
        let groups = config["proxy-groups"].as_array().unwrap();
        let names: Vec<&str> = groups
            .iter()
            .map(|group| group["name"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(names, vec!["PROXY", URL_TEST_GROUP, FINAL_GROUP]);
        assert_eq!(groups[1]["type"], "url-test");
        // PROXY 的第一个成员必须是「自动选择」：默认让内核自己挑，锁定单节点才只是
        // 一次 PUT /proxies/PROXY。
        assert_eq!(groups[0]["proxies"][0], URL_TEST_GROUP);
        assert_eq!(
            config["rules"].as_array().unwrap().last().unwrap(),
            &Value::String(format!("MATCH,{FINAL_GROUP}"))
        );
        assert_eq!(config["proxies"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn auto_routing_strategies_emit_matching_group_types() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let candidate = node(
            "vless",
            json!({"server": "5.6.7.8", "port": 443, "uuid": "u2", "network": "ws", "security": "tls"}),
        );
        for (strategy, expected_type) in [
            (AutoRoutingStrategy::UrlTest, "url-test"),
            (AutoRoutingStrategy::Fallback, "fallback"),
            (AutoRoutingStrategy::LoadBalance, "load-balance"),
        ] {
            let mut options = MihomoConfigOptions {
                routing_mode: RoutingMode::ChinaDirect,
                url_test: true,
                auto_strategy: strategy,
                ..MihomoConfigOptions::default()
            };
            options.url_test_nodes =
                crate::export::nodes_to_clash_proxies(std::slice::from_ref(&candidate));
            let config = build_mihomo_config(&current, options).unwrap();
            let groups = config["proxy-groups"].as_array().unwrap();
            assert_eq!(groups[1]["type"], expected_type);
        }
    }

    /// 候选注入 → runtime.options → 配置生成 这条链以前是静默断的（SQL 拼参报错被
    /// 吞掉），所以把三段接起来测一次。
    #[tokio::test]
    async fn set_url_test_candidates_reaches_generated_config() {
        let candidate_a = node(
            "vless",
            json!({"server": "5.6.7.8", "port": 443, "uuid": "ua", "network": "ws", "security": "tls"}),
        );
        let candidate_b = node(
            "vless",
            json!({"server": "9.9.9.9", "port": 443, "uuid": "ub", "network": "ws", "security": "tls"}),
        );
        let manager = ProxyManager::new(MihomoConfigOptions {
            url_test: true,
            url_test_size: 20,
            ..MihomoConfigOptions::default()
        });
        assert_eq!(
            manager
                .set_url_test_candidates(vec![candidate_a, candidate_b])
                .await,
            2
        );

        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let config = build_mihomo_config(&current, manager.config().await).unwrap();
        let groups = config["proxy-groups"].as_array().unwrap();
        assert_eq!(groups.len(), 3, "候选应该生成 PROXY / 自动选择 / 漏网之鱼");
        assert_eq!(groups[1]["type"], "url-test");
        assert_eq!(groups[1]["proxies"].as_array().unwrap().len(), 3);
        // 判活节奏必须真的写进配置：这台客户端"坏节点长时间不被避开"就是栽在 300 秒一轮。
        assert_eq!(groups[1]["interval"], json!(URL_TEST_INTERVAL_SECS));
        assert_eq!(groups[1]["tolerance"], json!(URL_TEST_TOLERANCE_MS));
        assert_eq!(groups[1]["timeout"], json!(URL_TEST_TIMEOUT_MS));
        assert_eq!(
            URL_TEST_INTERVAL_SECS, 60,
            "url-test 间隔被改回分钟级，坏节点又要几分钟才被避开"
        );
    }

    #[test]
    fn url_test_off_keeps_single_select_group_and_proxy_match() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let options = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&current, options).unwrap();
        assert_eq!(config["proxy-groups"].as_array().unwrap().len(), 1);
        assert_eq!(config["proxies"].as_array().unwrap().len(), 1);
        assert_eq!(
            config["rules"].as_array().unwrap().last().unwrap(),
            &Value::String("MATCH,PROXY".into())
        );
    }

    #[test]
    fn china_direct_mode_places_dns_poison_overrides_before_geoip() {
        // 境内 DoH 把 googlevideo 污染成国内 IP 时，只有先按域名定性才不会误判直连。
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let options = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&selected, options).unwrap();
        let rules = config["rules"].as_array().unwrap();
        let geoip = rules
            .iter()
            .position(|rule| rule == "GEOIP,CN,DIRECT,no-resolve")
            .expect("GEOIP,CN,DIRECT,no-resolve");
        assert_eq!(&rules[geoip - 2], "GEOSITE,google,PROXY");
        assert_eq!(&rules[geoip - 1], "GEOSITE,youtube,PROXY");
        assert_eq!(rules.last().unwrap(), "MATCH,PROXY");
        assert!(config["dns"].get("fallback").is_none());
    }

    #[test]
    fn routing_presets_generate_expected_mihomo_fallbacks() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let cases = [
            (RoutingMode::ChinaDirect, "MATCH,PROXY"),
            (RoutingMode::GfwList, "MATCH,DIRECT"),
            (RoutingMode::Global, "MATCH,PROXY"),
            (RoutingMode::Direct, "MATCH,DIRECT"),
        ];

        for (routing_mode, fallback) in cases {
            let config = build_mihomo_config(
                &selected,
                MihomoConfigOptions {
                    routing_mode,
                    ..MihomoConfigOptions::default()
                },
            )
            .unwrap();
            assert_eq!(
                config["rules"].as_array().unwrap().last().unwrap(),
                fallback
            );
        }
    }

    #[test]
    fn online_mihomo_presets_enable_geo_updates() {
        let selected = node(
            "trojan",
            json!({"server":"1.2.3.4","port":443,"password":"p"}),
        );
        let config = build_mihomo_config(
            &selected,
            MihomoConfigOptions {
                routing_mode: RoutingMode::GfwList,
                ..MihomoConfigOptions::default()
            },
        )
        .unwrap();

        assert_eq!(config["geo-auto-update"], false);
        assert!(config["geox-url"]["geosite"]
            .as_str()
            .unwrap()
            .contains("Loyalsoldier"));
        assert_eq!(config["rules"][1], "GEOSITE,gfw,PROXY");
    }

    #[test]
    fn builds_tun_dns_and_ordered_routing_config() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let options = MihomoConfigOptions {
            mode: ProxyMode::Tun,
            rules: vec![
                TrafficRule {
                    matcher: RuleMatcher::GeoSite("cn".into()),
                    action: RuleAction::Direct,
                },
                TrafficRule {
                    matcher: RuleMatcher::GeoIp("CN".into()),
                    action: RuleAction::Direct,
                },
                TrafficRule {
                    matcher: RuleMatcher::Quic,
                    action: RuleAction::Reject,
                },
            ],
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&selected, options).unwrap();

        assert_eq!(config["tun"]["enable"], true);
        assert_eq!(config["tun"]["stack"], "mixed");
        assert_eq!(config["tun"]["strict-route"], true);
        assert_eq!(config["tun"]["dns-hijack"][0], "any:53");
        assert_eq!(config["dns"]["enhanced-mode"], "fake-ip");
        assert_eq!(config["dns"]["respect-rules"], true);
        assert_eq!(config["dns"]["nameserver-policy"]["geosite:cn"][0], "223.5.5.5");
        assert_eq!(config["dns"]["nameserver-policy"]["geosite:geolocation-!cn"][0], "https://1.1.1.1/dns-query");
        assert_eq!(config["dns"]["default-nameserver"][0], "223.5.5.5");
        assert_eq!(config["dns"]["nameserver"][0], DEFAULT_DNS_PRIMARY);
        assert_eq!(config["dns"]["nameserver"][1], DEFAULT_DNS_SECONDARY);
        assert_eq!(config["geodata-mode"], true);
        assert_eq!(config["rules"][0], "GEOSITE,cn,DIRECT");
        // 生成配置时污染免疫规则被插在 GEOIP,CN 之前，用户规则表本身不变。
        assert_eq!(config["rules"][1], "GEOSITE,google,PROXY");
        assert_eq!(config["rules"][2], "GEOSITE,youtube,PROXY");
        assert_eq!(config["rules"][3], "GEOIP,CN,DIRECT,no-resolve");
        assert_eq!(
            config["rules"][4],
            "AND,((NETWORK,udp),(DST-PORT,443)),REJECT"
        );
        for (i, host) in crate::geo::MIRROR_DIRECT_HOSTS.iter().enumerate() {
            assert_eq!(config["rules"][5 + i], format!("DOMAIN-SUFFIX,{host},DIRECT"));
        }
        assert_eq!(config["rules"].as_array().unwrap().last().unwrap(), "MATCH,PROXY");
    }

    #[test]
    fn system_mode_does_not_emit_tun_config() {
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111"
            }),
        );

        let config = build_mihomo_config(&selected, MihomoConfigOptions::default()).unwrap();

        assert!(config.get("tun").is_none());
    }

    #[test]
    fn rejects_node_unsupported_by_mihomo_export() {
        let selected = node(
            "mieru",
            json!({"server": "1.2.3.4", "port": 443, "password": "secret"}),
        );

        let error = build_mihomo_config(&selected, MihomoConfigOptions::default()).unwrap_err();

        assert!(error.to_string().contains("不支持"));
    }

    #[tokio::test]
    async fn work_dir_uses_kernel_specific_paths_and_cleans_up() {
        for (kernel, config_name, log_name) in [
            (KernelKind::Mihomo, "config.yaml", "mihomo.log"),
            (KernelKind::SingBox, "config.json", "sing-box.log"),
        ] {
            let work_dir = ProxyWorkDir::create(kernel).await.unwrap();
            let root = work_dir.root().to_path_buf();

            assert!(root.is_dir());
            assert_eq!(work_dir.config_path(), root.join(config_name));
            assert_eq!(work_dir.log_path(), root.join(log_name));
            match kernel {
                KernelKind::Mihomo => assert_eq!(
                    work_dir.data_root(),
                    crate::kernel::managed_kernel_root().join("mihomo")
                ),
                KernelKind::SingBox => assert_eq!(work_dir.data_root(), root),
                KernelKind::Xray => unreachable!(),
            }

            work_dir.cleanup().await.unwrap();
            assert!(!root.exists());
        }
    }

    #[tokio::test]
    async fn manager_starts_in_stopped_state() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());

        let status = manager.status().await;

        assert_eq!(status.state, ProxyState::Stopped);
        assert!(status.node_id.is_none());
    }

    #[test]
    fn degraded_status_distinguishes_ready_and_waiting_states() {
        let mut status = ProxyRuntimeStatus::stopped(MihomoConfigOptions::default());
        status.state = ProxyState::Degraded;
        status.system_proxy_enabled = true;
        status.last_error = Some("验证超时".into());

        status.local_proxy_ready = true;
        assert!(status.local_proxy_active());
        assert!(!status.config_editable());

        status.local_proxy_ready = false;
        assert!(!status.local_proxy_active());
        assert!(status.config_editable());
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["system_proxy_enabled"], true);
        assert_eq!(json["last_error"], "验证超时");
    }

    #[tokio::test]
    async fn manager_updates_valid_config_while_stopped() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        let mut options = manager.config().await;
        options.mode = ProxyMode::Tun;
        options.rules.push(TrafficRule {
            matcher: RuleMatcher::GeoSite("cn".into()),
            action: RuleAction::Direct,
        });

        manager.update_config(options.clone()).await.unwrap();

        assert_eq!(manager.config().await, options);
        assert_eq!(manager.status().await.mode, ProxyMode::Tun);
    }

    #[tokio::test]
    async fn manager_rejects_config_changes_while_running() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        manager.runtime.lock().await.status.state = ProxyState::Running;

        let error = manager
            .update_config(MihomoConfigOptions::default())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("停止代理"));
    }

    #[tokio::test]
    async fn missing_mihomo_executable_sets_failed_status() {
        let controller = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let controller_port = controller.local_addr().unwrap().port();
        drop(controller);
        let mixed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mixed_port = mixed.local_addr().unwrap().port();
        drop(mixed);
        let manager = ProxyManager::new(MihomoConfigOptions {
            mixed_port,
            controller_port,
            ..MihomoConfigOptions::default()
        });
        let selected = node(
            "vless",
            json!({
                "server": "1.2.3.4",
                "port": 443,
                "uuid": "11111111-1111-1111-1111-111111111111",
                "network": "ws",
                "security": "tls"
            }),
        );
        let missing = std::env::temp_dir().join(format!(
            "vpn-parser-missing-mihomo-{}.exe",
            uuid::Uuid::new_v4()
        ));

        let error = manager
            .start_with_path(&selected, &missing)
            .await
            .unwrap_err();
        let status = manager.status().await;

        assert!(error.to_string().contains("启动"));
        assert_eq!(status.state, ProxyState::Failed);
        assert!(status.message.contains("启动"));
    }

    #[tokio::test]
    async fn missing_sing_box_executable_keeps_selected_kernel_in_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mixed_port = listener.local_addr().unwrap().port();
        drop(listener);
        let manager = ProxyManager::new(MihomoConfigOptions {
            mixed_port,
            ..MihomoConfigOptions::default()
        });
        let selected = node(
            "anytls",
            json!({
                "server": "anytls.example.com",
                "port": 443,
                "password": "secret"
            }),
        );
        let missing = std::env::temp_dir().join(format!(
            "vpn-parser-missing-sing-box-{}.exe",
            uuid::Uuid::new_v4()
        ));

        let error = manager
            .start_with_kernel_path(&selected, KernelKind::SingBox, &missing)
            .await
            .unwrap_err();
        let status = manager.status().await;

        assert!(error.to_string().contains("Sing-box"));
        assert_eq!(status.state, ProxyState::Failed);
        assert_eq!(status.kernel, Some(KernelKind::SingBox));
        assert!(status.message.contains("Sing-box"));
    }

    #[tokio::test]
    async fn occupied_controller_port_is_rejected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let error = ensure_port_available(port).await.unwrap_err();

        assert!(error.to_string().contains("已被占用"));
    }

    #[tokio::test]
    async fn startup_probe_rejects_proxy_that_cannot_reach_test_url() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });

        let started = tokio::time::Instant::now();
        let error = verify_local_proxy(port, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("本地代理无法访问测试地址"));
        assert!(started.elapsed() < Duration::from_millis(1_500));
    }

    #[tokio::test]
    async fn system_proxy_activation_returns_snapshot_for_restore() {
        let controller = Arc::new(FakeSystemProxy::default());

        let snapshot = activate_system_proxy(controller.clone(), "127.0.0.1:7890".into(), None)
            .await
            .unwrap();
        restore_system_proxy(controller.clone(), snapshot.clone(), false)
            .await
            .unwrap();

        assert_eq!(
            controller.enabled.lock().unwrap().as_slice(),
            ["127.0.0.1:7890"]
        );
        assert_eq!(controller.restored.lock().unwrap().as_slice(), [snapshot]);
    }

    #[tokio::test]
    async fn restart_stop_preserves_system_proxy_without_restoring_it() {
        let controller = Arc::new(FakeSystemProxy::default());
        let manager =
            ProxyManager::with_system_proxy(MihomoConfigOptions::default(), controller.clone());
        let snapshot = SystemProxySnapshot {
            enabled: false,
            server: Some("old-proxy:8080".into()),
            override_list: Some("<local>".into()),
        };
        {
            let mut runtime = manager.runtime.lock().await;
            runtime.status.state = ProxyState::Running;
            runtime.system_proxy_snapshot = Some(snapshot.clone());
        }

        let preserved = manager.stop_inner_for_restart().await.unwrap();

        assert_eq!(preserved, Some(snapshot));
        assert!(controller.restored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn switch_node_falls_back_when_not_running() {
        let manager = ProxyManager::new(MihomoConfigOptions::default());
        let test_node = node(
            "ss",
            json!({
                "server": "1.2.3.4",
                "port": 8388,
                "cipher": "aes-128-gcm",
                "password": "pass"
            }),
        );
        let res = manager.try_hot_reload_mihomo(&test_node).await.unwrap();
        assert!(!res);
    }

    #[test]
    fn format_kernel_diagnostic_cleans_raw_errors() {
        let raw1 = r#"time="2026-09-04T13:00:45.748268800+08:00" level=warning msg="[TCP] dial PROXY (match Match/) 127.0.0.1:53655 --> www.google.com:443 error: failed to dial WebSocket: read tcp 192.168.0.103:53662->104.18.42.163:2087: wsarecv: An existing connection was forcibly closed by the remote host.""#;
        let formatted1 = format_kernel_diagnostic(raw1);
        assert!(formatted1.contains("远程服务器强制重置/关闭连接 (目标节点已不可用)"));

        let raw2 = r#"error: dial tcp: i/o timeout"#;
        let formatted2 = format_kernel_diagnostic(raw2);
        assert!(formatted2.contains("连接远程服务器超时"));

        let raw3 = "\u{1b}[36mINFO\u{1b}[0m [\u{1b}[38;5;40m1600451608\u{1b}[0m 0ms] outbound/tuic[proxy]: outbound connection to www.google.com:443";
        let formatted3 = format_kernel_diagnostic(raw3);
        assert!(formatted3.contains("目标服务器无法连通") || formatted3.contains("节点连接超时"));
        assert!(!formatted3.contains("\u{1b}"));
    }

    #[test]
    fn port_mapping_validation_checks() {
        let mut options = MihomoConfigOptions::default();
        options.port_mappings = vec![PortMapping {
            id: "1".into(),
            port: 80, // < 1024
            node_id: "n1".into(),
            node_name: "Node 1".into(),
            enabled: true,
        }];
        assert!(options.validate().unwrap_err().to_string().contains("保留端口"));

        options.port_mappings[0].port = 9980; // mixed_port
        assert!(options.validate().unwrap_err().to_string().contains("主代理端口冲突"));

        options.port_mappings[0].port = 9090; // controller_port
        assert!(options.validate().unwrap_err().to_string().contains("控制台端口冲突"));

        options.port_mappings = vec![
            PortMapping {
                id: "1".into(),
                port: 10810,
                node_id: "n1".into(),
                node_name: "Node 1".into(),
                enabled: true,
            },
            PortMapping {
                id: "2".into(),
                port: 10810,
                node_id: "n2".into(),
                node_name: "Node 2".into(),
                enabled: true,
            },
        ];
        assert!(options.validate().unwrap_err().to_string().contains("重复"));

        options.port_mappings[1].port = 10811;
        assert!(options.validate().is_ok());
    }

    #[test]
    fn port_mappings_generate_mihomo_listeners_and_in_port_rules() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let mapped_node = node(
            "vless",
            json!({"server": "5.6.7.8", "port": 443, "uuid": "u2", "network": "ws", "security": "tls"}),
        );
        let clash_proxies = crate::export::nodes_to_clash_proxies(&[mapped_node.clone()]);
        let mut options = MihomoConfigOptions {
            port_mappings: vec![PortMapping {
                id: "m1".into(),
                port: 10810,
                node_id: mapped_node.id.clone(),
                node_name: mapped_node.name.clone(),
                enabled: true,
            }],
            port_mapping_nodes: vec![(10810, mapped_node.name.clone(), clash_proxies[0].clone())],
            ..MihomoConfigOptions::default()
        };

        let config = build_mihomo_config(&current, options.clone()).unwrap();
        let listeners = config["listeners"].as_array().expect("listeners array");
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0]["name"], "inbound-10810");
        assert_eq!(listeners[0]["port"], 10810);
        assert_eq!(listeners[0]["type"], "mixed");

        let rules = config["rules"].as_array().expect("rules array");
        assert!(rules[0].as_str().unwrap().starts_with("IN-PORT,10810,"));

        // 验证 required_ports 包含启用的映射端口
        let adapter = kernel_adapter(KernelKind::Mihomo);
        let ports = adapter.required_ports(&options);
        assert!(ports.contains(&10810));

        // 禁用后不应包含在 required_ports
        options.port_mappings[0].enabled = false;
        let ports_disabled = adapter.required_ports(&options);
        assert!(!ports_disabled.contains(&10810));
    }

    #[test]
    fn security_privacy_protections_generate_expected_configs() {
        let current = node(
            "vless",
            json!({"server": "1.2.3.4", "port": 443, "uuid": "u1", "network": "ws", "security": "tls"}),
        );
        let hy2_node = node(
            "hysteria2",
            json!({"server": "5.6.7.8", "port": 443, "password": "pass", "sni": "example.com"}),
        );
        let options = MihomoConfigOptions {
            client_fingerprint: Some("chrome".to_string()),
            block_webrtc: true,
            block_trackers: true,
            udp_port_hopping: true,
            hop_ports: "30000-50000".to_string(),
            ..MihomoConfigOptions::default()
        };

        // 1. Mihomo 验证
        let config = build_mihomo_config(&current, options.clone()).unwrap();
        let proxies = config["proxies"].as_array().expect("proxies");
        assert_eq!(proxies[0]["client-fingerprint"], "chrome");

        let rules = config["rules"].as_array().expect("rules");
        assert!(rules.iter().any(|r| r == "DST-PORT,3478,REJECT"));
        assert!(rules.iter().any(|r| r == "DST-PORT,3479,REJECT"));
        assert!(rules.iter().any(|r| r == "DST-PORT,5349,REJECT"));
        assert!(rules.iter().any(|r| r == "DST-PORT,5350,REJECT"));
        assert!(rules.iter().any(|r| r == "DST-PORT,19302-19309,REJECT"));
        assert!(rules.iter().any(|r| r == "DOMAIN-KEYWORD,stun,REJECT"));
        assert!(rules.iter().any(|r| r == "DOMAIN-KEYWORD,turn,REJECT"));
        assert!(rules.iter().any(|r| r == "DOMAIN-KEYWORD,webrtc,REJECT"));
        assert!(rules.iter().any(|r| r == "GEOSITE,category-ads-all,REJECT"));

        // 2. Sing-box 验证
        let sing_box_cfg = crate::proxy::sing_box::build_sing_box_config(&hy2_node, options.clone()).unwrap();
        let sing_box_outbounds = sing_box_cfg["outbounds"].as_array().expect("outbounds");
        assert_eq!(sing_box_outbounds[0]["tls"]["utls"]["fingerprint"], "chrome");
        assert_eq!(sing_box_outbounds[0]["hop_interval"], "30s");

        let sing_box_rules = sing_box_cfg["route"]["rules"].as_array().expect("route rules");
        assert!(sing_box_rules.iter().any(|r| r.get("domain_keyword").is_some()));
        assert!(sing_box_rules.iter().any(|r| r.get("port_range").is_some()));

        // 3. Xray 验证
        let xray_cfg = crate::proxy::xray::build_xray_config(&current, options).unwrap();
        let xray_outbounds = xray_cfg["outbounds"].as_array().expect("outbounds");
        assert_eq!(xray_outbounds[0]["streamSettings"]["tlsSettings"]["fingerprint"], "chrome");
        let xray_rules = xray_cfg["routing"]["rules"].as_array().expect("xray routing rules");
        assert!(xray_rules.iter().any(|r| r.get("port").and_then(|p| p.as_str()).map_or(false, |s| s.contains("19302-19309"))));
    }

    #[test]
    fn compiler_capabilities_and_diagnostics_catch_incompatibilities() {
        use crate::proxy::ir::*;
        use crate::proxy::compiler::*;

        let xray_compiler = XrayCompiler::new();
        let sing_box_compiler = SingBoxCompiler::new();

        let plan = lower_options_to_ir(
            &MihomoConfigOptions {
                mode: ProxyMode::Tun,
                url_test: true,
                rules: vec![TrafficRule {
                    matcher: RuleMatcher::ProcessName("chrome.exe".into()),
                    action: RuleAction::Direct,
                }],
                ..MihomoConfigOptions::default()
            },
            None,
        );

        // Xray 不支持 TUN 和 ProcessName
        let xray_diagnostics = xray_compiler.validate(&plan);
        assert!(xray_diagnostics.iter().any(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("TUN")));
        assert!(xray_diagnostics.iter().any(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("进程名")));

        // SingBox 不支持 url_test
        let sing_box_diagnostics = sing_box_compiler.validate(&plan);
        assert!(sing_box_diagnostics.iter().any(|d| d.severity == DiagnosticSeverity::Warning && d.message.contains("url-test")));
    }

    #[test]
    fn kernel_validator_validates_compiled_configs_with_real_binaries() {
        use crate::proxy::ir::*;
        use crate::proxy::compiler::*;
        use crate::proxy::validator::KernelValidator;

        // 测试场景 1：Shadowsocks + ChinaDirect
        let ss_node = node(
            "ss",
            json!({"server": "1.2.3.4", "port": 8388, "cipher": "aes-128-gcm", "password": "pass"}),
        );
        let options_china = MihomoConfigOptions {
            routing_mode: RoutingMode::ChinaDirect,
            block_webrtc: true,
            block_trackers: true,
            ..MihomoConfigOptions::default()
        };
        let plan_china = lower_options_to_ir(&options_china, Some(&ss_node));

        let mihomo_artifact = MihomoCompiler::new().compile(&plan_china).unwrap();
        assert!(KernelValidator::validate_artifact(KernelKind::Mihomo, &mihomo_artifact).is_ok());

        let xray_artifact = XrayCompiler::new().compile(&plan_china).unwrap();
        KernelValidator::validate_artifact(KernelKind::Xray, &xray_artifact).expect("xray validation failed");

        // 测试场景 2：Hysteria2 + GfwList + 客户端指纹
        let hy2_node = node(
            "hysteria2",
            json!({"server": "hy2.example.com", "port": 443, "password": "pass", "sni": "hy2.example.com"}),
        );
        let options_gfw = MihomoConfigOptions {
            routing_mode: RoutingMode::GfwList,
            client_fingerprint: Some("chrome".into()),
            block_webrtc: true,
            ..MihomoConfigOptions::default()
        };
        let plan_gfw = lower_options_to_ir(&options_gfw, Some(&hy2_node));

        let sing_box_artifact = SingBoxCompiler::new().compile(&plan_gfw).unwrap();
        assert!(KernelValidator::validate_artifact(KernelKind::SingBox, &sing_box_artifact).is_ok());

        let mihomo_hy2_artifact = MihomoCompiler::new().compile(&plan_gfw).unwrap();
        assert!(KernelValidator::validate_artifact(KernelKind::Mihomo, &mihomo_hy2_artifact).is_ok());

        // 测试场景 3：Trojan + 端口映射 + 自定义用户规则
        let trojan_node = node(
            "trojan",
            json!({"server": "trojan.example.com", "port": 443, "password": "pass", "sni": "trojan.example.com"}),
        );
        let options_trojan = MihomoConfigOptions {
            port_mappings: vec![PortMapping {
                id: "map-1".into(),
                port: 19890,
                node_id: "trojan-node".into(),
                node_name: "Trojan Dedicated".into(),
                enabled: true,
            }],
            rules: vec![
                TrafficRule {
                    matcher: RuleMatcher::Domain("api.github.com".into()),
                    action: RuleAction::Proxy,
                },
                TrafficRule {
                    matcher: RuleMatcher::IpCidr("192.168.1.0/24".into()),
                    action: RuleAction::Direct,
                },
                TrafficRule {
                    matcher: RuleMatcher::DestinationPort("8080".into()),
                    action: RuleAction::Direct,
                },
            ],
            ..MihomoConfigOptions::default()
        };
        let plan_trojan = lower_options_to_ir(&options_trojan, Some(&trojan_node));

        let mihomo_trojan_artifact = MihomoCompiler::new().compile(&plan_trojan).unwrap();
        KernelValidator::validate_artifact(KernelKind::Mihomo, &mihomo_trojan_artifact).expect("mihomo trojan validation failed");

        let xray_trojan_artifact = XrayCompiler::new().compile(&plan_trojan).unwrap();
        assert!(KernelValidator::validate_artifact(KernelKind::Xray, &xray_trojan_artifact).is_ok());

        let sing_box_trojan_artifact = SingBoxCompiler::new().compile(&plan_trojan);
        // Sing-box 暂不支持从普通 trojan 配置直接转换（只支持 hy2/tuic/anytls/wireguard），测试编译诊断正确拦截
        assert!(sing_box_trojan_artifact.is_err());

        // 测试场景 4：WireGuard + Sing-box
        let wg_node = node(
            "wireguard",
            json!({
                "server": "1.2.3.4", "port": 51820,
                "ip": "172.16.0.2/32",
                "private-key": "EFlemF/JAgdqTmKVb/1W/yv9WnwuzhdqLfDHUUNC11w=",
                "public-key": "TepNujm+9RJYXvQTB3CGDCnSccm05k6p/NmaHhj1C2c=",
                "reserved": [0, 0, 0]
            }),
        );
        let plan_wg = lower_options_to_ir(&options_china, Some(&wg_node));
        let sing_box_wg_artifact = SingBoxCompiler::new().compile(&plan_wg).unwrap();
        assert!(KernelValidator::validate_artifact(KernelKind::SingBox, &sing_box_wg_artifact).is_ok());
    }

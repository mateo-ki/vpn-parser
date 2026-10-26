//! 本地文件源识别与读取路径的单测。

use super::local_file_path;

#[test]
fn local_file_path_recognizes_windows_and_unix_forms() {
    // file:// 前缀
    assert_eq!(
        local_file_path("file:///C:/subs/clash.yaml").as_deref(),
        Some("C:/subs/clash.yaml")
    );
    assert_eq!(
        local_file_path("file://C:/subs/clash.yaml").as_deref(),
        Some("C:/subs/clash.yaml")
    );
    assert_eq!(
        local_file_path("file:///home/user/sub.txt").as_deref(),
        Some("/home/user/sub.txt")
    );
    // 裸本地路径
    assert_eq!(
        local_file_path(r"C:\subs\clash.yaml").as_deref(),
        Some(r"C:\subs\clash.yaml")
    );
    assert_eq!(
        local_file_path("C:/subs/clash.yaml").as_deref(),
        Some("C:/subs/clash.yaml")
    );
    assert_eq!(local_file_path("/etc/vpn/sub.txt").as_deref(), Some("/etc/vpn/sub.txt"));
    // macOS / Unix 路径与 URL 解码 (%20)
    assert_eq!(
        local_file_path("file:///Users/name/my%20proxy/clash.yaml").as_deref(),
        Some("/Users/name/my proxy/clash.yaml")
    );
    assert_eq!(
        local_file_path("file:///Users/name/Downloads/clash.yaml").as_deref(),
        Some("/Users/name/Downloads/clash.yaml")
    );
    // 引号包裹剥离
    assert_eq!(
        local_file_path(r#""/Users/name/Downloads/clash.yaml""#).as_deref(),
        Some("/Users/name/Downloads/clash.yaml")
    );
    assert_eq!(
        local_file_path(r#""C:\subs\clash.yaml""#).as_deref(),
        Some(r"C:\subs\clash.yaml")
    );
    // 波浪号扩展（若 HOME/USERPROFILE 存在）
    if std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).is_some() {
        assert!(local_file_path("~/Downloads/clash.yaml").is_some());
        let res = local_file_path("~/Downloads/clash.yaml").unwrap();
        assert!(!res.starts_with('~'));
        assert!(res.ends_with("Downloads/clash.yaml") || res.ends_with(r"Downloads\clash.yaml"));
    }
    // 非本地：URL 与协议形式不认
    assert_eq!(local_file_path("https://example.com/sub"), None);
    assert_eq!(local_file_path("http://example.com/sub"), None);
    assert_eq!(local_file_path("t.me/s/v2node"), None);
    assert_eq!(local_file_path(""), None);
}

#[tokio::test]
async fn reads_and_parses_real_local_yaml_file() {
    let file_path = std::env::temp_dir().join("vpn_parser_test_local_nodes.yaml");
    let sample_yaml = r#"
proxies:
  - name: "香港 01 专线"
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-256-gcm
    password: testpassword
  - name: "日本 02 专线"
    type: trojan
    server: 5.6.7.8
    port: 443
    password: trojanpassword
rules:
  - DOMAIN-SUFFIX,google.com,PROXY
"#;
    std::fs::write(&file_path, sample_yaml).unwrap();

    let path_str = file_path.to_string_lossy().to_string();
    let clean_path = local_file_path(&path_str).expect("应识别真实磁盘文件");
    assert!(std::path::Path::new(&clean_path).is_file());

    // 测试带引号
    let quoted = format!("\"{clean_path}\"");
    assert_eq!(local_file_path(&quoted).as_deref(), Some(clean_path.as_str()));

    // 读取并解析节点
    let content = tokio::fs::read_to_string(&clean_path).await.unwrap();
    let nodes = crate::parser::parse_subscription(&content, Some("local_test_source"));
    assert_eq!(nodes.len(), 2);
    let hk = nodes.iter().find(|n| n.name == "香港 01 专线").expect("应包含香港节点");
    assert_eq!(hk.server, "1.2.3.4");
    assert_eq!(hk.port, 8388);
    let jp = nodes.iter().find(|n| n.name == "日本 02 专线").expect("应包含日本节点");
    assert_eq!(jp.server, "5.6.7.8");
    assert_eq!(jp.port, 443);

    // 提取自带分流规则
    let rules = crate::parser::extract_subscription_rules(&content);
    assert_eq!(rules.len(), 1);
    assert!(matches!(
        rules[0].matcher,
        crate::proxy::RuleMatcher::DomainSuffix(ref domain) if domain == "google.com"
    ));

    let _ = std::fs::remove_file(&file_path);
}

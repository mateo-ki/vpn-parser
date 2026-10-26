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

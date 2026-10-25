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
    // 非本地：URL 与协议形式不认
    assert_eq!(local_file_path("https://example.com/sub"), None);
    assert_eq!(local_file_path("http://example.com/sub"), None);
    assert_eq!(local_file_path("t.me/s/v2node"), None);
    assert_eq!(local_file_path(""), None);
}

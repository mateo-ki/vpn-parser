fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    println!("cargo:rerun-if-changed=assets/windows.rc");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // 版本号只有一个真源：Cargo.toml 的 version。build.rs 把它落到 `target/cargo-version.txt`，
    // 安装包脚本（installer/vpn-parser.iss）读那个文件 —— 这样就不会再出现
    // "Cargo.toml 写 1.0.0、安装包写 1.0、tag 写 v1.0.1" 三处各说各话。
    // OUT_DIR 形如 target/<profile>/build/vpn-parser-<hash>/out，往上找到 build 再退两级就是 target。
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    if let Some(target_dir) = out_dir
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "build"))
        .and_then(|profile_dir| profile_dir.parent())
        .and_then(|profile_dir| profile_dir.parent())
    {
        // 写失败不影响编译，但安装包脚本会因此报"先跑一次 cargo build"。
        // 生成的是 ISPP 片段而不是纯文本：ISPP 的 #include 是确定支持的，
        // 而 FileRead 的参数签名各版本有出入，不值得让打包脚本去赌。
        let package_version = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");
        let _ = std::fs::write(
            target_dir.join("cargo-version.iss"),
            format!(
                "; 由 build.rs 自动生成，请勿手改 —— 版本号的唯一真源是 Cargo.toml\n\
                 #define MyAppVersion \"{}\"\n\
                 #define MyAppFileVersion \"{}\"\n",
                package_version,
                file_version(&package_version)
            ),
        );
    }

    #[cfg(windows)]
    embed_resource::compile("assets/windows.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("compile Windows application resources");
}

/// 安装包用的"文件版本"：Inno 的 `VersionInfoVersion` / `VersionInfoProductVersion`
/// 只收纯数字 `x.x.x.x`，而预发布版的号是 `1.4.0-beta.1` —— 直接把那个写进去，ISCC
/// 当场报错了结。所以显示版本（文件名、"关于"里那行、卸载列表）保留完整字符串，
/// 文件版本丢掉 `-beta.N` 并补齐成四段。
///
/// 这个函数没法用 `cargo test` 守：build.rs 不是测试目标（实测 `cargo test file_version`
/// 收集到 0 条）。要验就直接编一次安装包 —— 把 `target/cargo-version.iss` 手工改成
/// `#define MyAppVersion "1.4.0-beta.1"` 再跑 ISCC：文件版本给成 beta 串时 Inno 会报
/// `Value of [Setup] section directive "VersionInfoVersion" is invalid`，给成 `1.4.0.0` 就能出包。
fn file_version(package_version: &str) -> String {
    let core = package_version
        .split(['-', '+'])
        .next()
        .unwrap_or(package_version);
    let mut parts: Vec<String> = core
        .split('.')
        .map(|segment| {
            let digits: String = segment
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                "0".to_string()
            } else {
                digits
            }
        })
        .take(4)
        .collect();
    while parts.len() < 4 {
        parts.push("0".to_string());
    }
    parts.join(".")
}

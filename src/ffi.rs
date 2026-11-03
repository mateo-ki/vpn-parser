//! C-ABI 跨平台 FFI 导出层
//!
//! 为移动端原生环境（Android JNI、iOS Swift/Objective-C、鸿蒙 C/NAPI）
//! 以及动态语言（Python ctypes、Node.js ffi-napi）提供零 GUI 依赖的高性能 C 接口。

use std::ffi::{CStr, CString};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::raw::c_char;
use std::time::{Duration, Instant};

use crate::models::Node;
use crate::proxy::compiler::{
    BackendCompiler, CompiledArtifact, MeowCompiler, MihomoCompiler, ShoesCompiler,
    SingBoxCompiler, VpCoreCompiler, XrayCompiler,
};
use crate::proxy::ir::lower_options_to_ir;
use crate::proxy::MihomoConfigOptions;

/// 返回当前库版本字符串（静态字符串，无需释放）。
#[no_mangle]
pub extern "C" fn vpn_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

/// 释放由本 FFI 模块分配返回的 C 字符串指针。
///
/// # Safety
/// 调用方必须确保传入的指针是由本库 FFI 函数（如 `vpn_parse_subscription`、
/// `vpn_compile_config`）返回的有效堆分配指针，且未被重复释放。
#[no_mangle]
pub unsafe extern "C" fn vpn_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(CString::from_raw(ptr));
    }
}

/// 辅助函数：将 Rust String 转换为堆分配的 CString 指针。
fn to_c_string(s: String) -> *mut c_char {
    CString::new(s).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

/// 辅助函数：将 C 字符串安全解析为 &str。
unsafe fn c_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        None
    } else {
        CStr::from_ptr(ptr).to_str().ok()
    }
}

/// 解析订阅文本或多行节点链接为 JSON 字符串。
///
/// 输入：
/// - `content`: Base64 编码的订阅正文、Clash YAML、Sing-box JSON 或单/多行节点链接（ss/vmess/vless/trojan/hy2/tuic/wg）
///
/// 输出：
/// - 成功：JSON 数组字符串 `[ { "id": "...", "name": "...", "protocol": "...", ... } ]`
/// - 失败：JSON 错误对象 `{"error": "..."}`
///
/// 注意：返回的字符串指针必须调用 [`vpn_free_string`] 释放。
#[no_mangle]
pub unsafe extern "C" fn vpn_parse_subscription(content: *const c_char) -> *mut c_char {
    let Some(raw_content) = c_to_str(content) else {
        return to_c_string(r#"{"error":"invalid null or non-utf8 string"}"#.to_string());
    };

    let nodes = crate::parser::parse_subscription(raw_content, None);
    match serde_json::to_string(&nodes) {
        Ok(json) => to_c_string(json),
        Err(e) => to_c_string(format!(r#"{{"error":"serialization failed: {e}"}}"#)),
    }
}

/// 快捷获取订阅文本中包含的可用节点数量。
#[no_mangle]
pub unsafe extern "C" fn vpn_parse_nodes_count(content: *const c_char) -> i32 {
    let Some(raw_content) = c_to_str(content) else {
        return -1;
    };
    let nodes = crate::parser::parse_subscription(raw_content, None);
    nodes.len() as i32
}

/// 将节点与配置选项编译为目标代理内核的配置内容（YAML 或 JSON 字符串）。
///
/// 参数：
/// - `kernel_kind`: 目标内核名称，支持 `"mihomo"` / `"sing-box"` / `"vp-core"` / `"xray"` / `"shoes"` / `"meow"`
/// - `node_json`: 单个节点的 JSON 字符串，反序列化为 `Node`
/// - `options_json`: 配置选项 JSON 字符串（对应 `MihomoConfigOptions`），若传 NULL 则采用默认生产配置
///
/// 输出：
/// - 成功：目标内核配置文本（Mihomo/VP-Core 为 YAML，Sing-box/Xray 为格式化 JSON）
/// - 失败：以 `{"error": "..."}` 格式返回错误原因
///
/// 注意：返回的字符串指针必须调用 [`vpn_free_string`] 释放。
#[no_mangle]
pub unsafe extern "C" fn vpn_compile_config(
    kernel_kind: *const c_char,
    node_json: *const c_char,
    options_json: *const c_char,
) -> *mut c_char {
    let Some(k_str) = c_to_str(kernel_kind) else {
        return to_c_string(r#"{"error":"kernel_kind must not be null"}"#.to_string());
    };
    let Some(n_str) = c_to_str(node_json) else {
        return to_c_string(r#"{"error":"node_json must not be null"}"#.to_string());
    };

    let node: Node = match serde_json::from_str(n_str) {
        Ok(n) => n,
        Err(e) => return to_c_string(format!(r#"{{"error":"invalid node_json: {e}"}}"#)),
    };

    let options: MihomoConfigOptions = if let Some(opt_str) = c_to_str(options_json) {
        match serde_json::from_str(opt_str) {
            Ok(opts) => opts,
            Err(e) => return to_c_string(format!(r#"{{"error":"invalid options_json: {e}"}}"#)),
        }
    } else {
        MihomoConfigOptions::default()
    };

    let plan = lower_options_to_ir(&options, Some(&node));

    let result = match k_str.to_ascii_lowercase().as_str() {
        "mihomo" | "clash" => MihomoCompiler::default().compile(&plan),
        "sing-box" | "singbox" => SingBoxCompiler::default().compile(&plan),
        "vp-core" | "vpcore" | "vp_core" => VpCoreCompiler::default().compile(&plan),
        "xray" => XrayCompiler::default().compile(&plan),
        "shoes" => ShoesCompiler::default().compile(&plan),
        "meow" => MeowCompiler::default().compile(&plan),
        other => return to_c_string(format!(r#"{{"error":"unsupported kernel kind: {other}"}}"#)),
    };

    match result {
        Ok(artifact) => match artifact {
            CompiledArtifact::Yaml(yaml) => to_c_string(yaml),
            CompiledArtifact::Json(json) => {
                to_c_string(serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string()))
            }
        },
        Err(e) => to_c_string(format!(r#"{{"error":"compilation failed: {e}"}}"#)),
    }
}

/// 快速轻量 TCP 握手测速（跨平台阻塞探测，毫秒单位）。
///
/// 参数：
/// - `host`: 目标主机名或 IP 地址
/// - `port`: 端口
/// - `timeout_ms`: 超时毫秒数（例如 3000）
///
/// 返回值：
/// - >= 0：握手耗时（毫秒）
/// - -1：连接超时或解析失败
#[no_mangle]
pub unsafe extern "C" fn vpn_test_tcp_latency(
    host: *const c_char,
    port: u16,
    timeout_ms: u32,
) -> i32 {
    let Some(host_str) = c_to_str(host) else {
        return -1;
    };

    let timeout = Duration::from_millis(timeout_ms as u64);
    let start = Instant::now();

    let addrs: Vec<SocketAddr> = match (host_str, port).to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(_) => return -1,
    };

    if addrs.is_empty() {
        return -1;
    }

    // 优先尝试可解析的套接字地址
    for addr in addrs {
        if let Ok(_stream) = TcpStream::connect_timeout(&addr, timeout) {
            let elapsed = start.elapsed().as_millis() as i32;
            return elapsed.max(1);
        }
    }

    -1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_ffi_version() {
        let v_ptr = vpn_version();
        let c_str = unsafe { CStr::from_ptr(v_ptr) };
        assert_eq!(c_str.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_ffi_parse_subscription() {
        let uri = "ss://YWVzLTEyOC1nY206cGFzc3dvcmRAMTI3LjAuMC4xOjgzODg=#TestNode";
        let c_uri = CString::new(uri).unwrap();

        unsafe {
            let count = vpn_parse_nodes_count(c_uri.as_ptr());
            assert_eq!(count, 1);

            let json_ptr = vpn_parse_subscription(c_uri.as_ptr());
            assert!(!json_ptr.is_null());
            let json_str = CStr::from_ptr(json_ptr).to_str().unwrap();
            assert!(json_str.contains("TestNode"));
            assert!(json_str.contains("127.0.0.1"));

            vpn_free_string(json_ptr);
        }
    }

    #[test]
    fn test_ffi_compile_config() {
        let uri = "ss://YWVzLTEyOC1nY206cGFzc3dvcmRAMTI3LjAuMC4xOjgzODg=#CompileTest";
        let nodes = crate::parser::parse_subscription(uri, None);
        assert_eq!(nodes.len(), 1);

        let node_json = serde_json::to_string(&nodes[0]).unwrap();
        let c_node = CString::new(node_json).unwrap();
        let c_kernel_mihomo = CString::new("mihomo").unwrap();
        let c_kernel_sb = CString::new("sing-box").unwrap();

        unsafe {
            // 1. Mihomo 编译
            let config_ptr = vpn_compile_config(
                c_kernel_mihomo.as_ptr(),
                c_node.as_ptr(),
                std::ptr::null(),
            );
            assert!(!config_ptr.is_null());
            let config_str = CStr::from_ptr(config_ptr).to_str().unwrap();
            assert!(config_str.contains("proxies"), "config should contain proxies: {config_str}");
            assert!(config_str.contains("CompileTest"));
            vpn_free_string(config_ptr);

            // 2. Sing-box 编译 (测试 Hysteria2 协议)
            let hy2_uri = "hysteria2://test_pass@127.0.0.1:443#Hy2CompileTest";
            let hy2_nodes = crate::parser::parse_subscription(hy2_uri, None);
            assert_eq!(hy2_nodes.len(), 1);
            let hy2_node_json = serde_json::to_string(&hy2_nodes[0]).unwrap();
            let c_hy2_node = CString::new(hy2_node_json).unwrap();

            let config_sb_ptr = vpn_compile_config(
                c_kernel_sb.as_ptr(),
                c_hy2_node.as_ptr(),
                std::ptr::null(),
            );
            assert!(!config_sb_ptr.is_null());
            let config_sb_str = CStr::from_ptr(config_sb_ptr).to_str().unwrap();
            assert!(config_sb_str.contains("outbounds"), "sing-box config should contain outbounds: {config_sb_str}");
            assert!(config_sb_str.contains("hysteria2"));
            assert!(config_sb_str.contains("127.0.0.1"));
            vpn_free_string(config_sb_ptr);
        }
    }

    #[test]
    fn test_ffi_tcp_latency_invalid() {
        // RFC 5737 TEST-NET-1 (192.0.2.1) 不可路由保留地址，测试超时返回 -1
        let invalid_host = CString::new("192.0.2.1").unwrap();
        unsafe {
            let latency = vpn_test_tcp_latency(invalid_host.as_ptr(), 54321, 100);
            assert_eq!(latency, -1);
        }
    }
}

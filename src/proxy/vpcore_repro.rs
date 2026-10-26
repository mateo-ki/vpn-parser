//! 手动复现（`#[ignore]`）：读取生产数据目录里的真实代理配置与节点，
//! 按应用的启动路径编译 VP-Core / Mihomo 配置并落盘到 `target/`，
//! 供人工用真实内核二进制做连通性验证。
//!
//! 运行：`cargo test --bin vpn-parser vpcore_repro -- --ignored --nocapture`

#![cfg(test)]

use crate::kernel::{select_proxy_kernel_preference_for_node, KernelKind, KernelPreference};
use crate::proxy::manager::{build_kernel_config, serialize_kernel_config};

const PROD_DATA: &str = r"D:\software\VPN Parser\data";
/// startup.log 里 00:08 启动会话记录的活动节点（WARP MASQUE）。
const WARP_NODE_ID: &str = "3501875e-6901-4ceb-bf4d-7237b3b0270f";
/// 用户实际在用的 VLESS+WS+TLS 节点（新加坡·优选IP-25）。
const VLESS_NODE_ID: &str = "c7a3db3a-3b19-4129-8b5e-bd103f9e7fa3";

#[tokio::test]
#[ignore = "手动复现：依赖生产数据目录"]
async fn vpcore_repro_dump_configs() {
    let mut db = crate::db::Database::connect(&format!("{PROD_DATA}\\vpn-parser.db"))
        .await
        .expect("open prod db");
    db.node_root = std::path::PathBuf::from(format!("{PROD_DATA}\\subscriptions"));

    let options = db.get_proxy_config().await.expect("proxy config");
    let nodes = db.load_nodes(None).await.expect("nodes");
    eprintln!(
        "mode={:?} pref={:?} routing={:?} url_test={} url_test_size={}",
        options.mode, options.kernel_preference, options.routing_mode, options.url_test, options.url_test_size
    );
    eprintln!("nodes loaded: {}", nodes.len());

    let warp = nodes
        .iter()
        .find(|n| n.id == WARP_NODE_ID)
        .expect("warp node in pool");
    match select_proxy_kernel_preference_for_node(warp, KernelPreference::VpCore, true, false) {
        Ok((kernel, notice)) => eprintln!(
            "WARP(masque) -> kernel={:?} notice={notice:?}",
            kernel
        ),
        Err(e) => eprintln!("WARP(masque) -> select error: {e}"),
    }

    let fast_http = nodes
        .iter()
        .filter(|n| matches!(n.protocol.as_str(), "http" | "https"))
        .filter(|n| n.latency_ok == Some(true))
        .min_by_key(|n| n.latency_ms.unwrap_or(i64::MAX))
        .expect("a latency-tested http node");
    eprintln!(
        "fast http node: {} {} {}:{} latency={:?}",
        fast_http.name, fast_http.protocol, fast_http.server, fast_http.port, fast_http.latency_ms
    );
    let vless = nodes
        .iter()
        .find(|n| n.id == VLESS_NODE_ID)
        .expect("vless node in pool");
    eprintln!(
        "vless node: {} {} {}:{}",
        vless.name, vless.protocol, vless.server, vless.port
    );

    for (tag, node) in [
        ("warp", warp),
        ("http", fast_http),
        ("vless", vless),
    ] {
        for kernel in [KernelKind::VpCore, KernelKind::Mihomo] {
            let label = format!("{tag}-{}", kernel.as_str());
            match build_kernel_config(kernel, node, options.clone()) {
                Ok(cfg) => {
                    let yaml = serialize_kernel_config(kernel, &cfg).expect("serialize");
                    let path = format!("target/repro-{label}.yaml");
                    std::fs::write(&path, &yaml).expect("write yaml");
                    eprintln!("wrote {path} ({} bytes)", yaml.len());
                }
                Err(e) => eprintln!("{label}: compile error: {e}"),
            }
        }
    }
}

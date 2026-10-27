//! 手动复现（`#[ignore]`）：读取本地数据目录里的代理配置与节点，
//! 按应用的启动路径编译 VP-Core / Mihomo 配置并落盘到 `target/`，
//! 供人工用真实内核二进制做连通性验证。
//!
//! 涉及本地数据目录与具体节点的信息一律通过环境变量注入，不写入源码：
//! - `VPCORE_REPRO_DATA`：数据目录（含 vpn-parser.db 与 subscriptions/）；
//! - `VPCORE_REPRO_NODES`：逗号分隔的待编译节点 id 列表；
//! - `VPCORE_REPRO_LATENCY_HTTP`：可选，"1" 时额外挑一个延迟最低的 http 节点。
//!
//! 运行：`cargo test --bin vpn-parser vpcore_repro -- --ignored --nocapture`

#![cfg(test)]

use crate::kernel::{select_proxy_kernel_preference_for_node, KernelKind, KernelPreference};
use crate::proxy::manager::{build_kernel_config, serialize_kernel_config};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::test]
#[ignore = "手动复现：依赖 VPCORE_REPRO_DATA 指向的本地数据目录"]
async fn vpcore_repro_dump_configs() {
    let Some(data_dir) = env("VPCORE_REPRO_DATA") else {
        eprintln!("未设置 VPCORE_REPRO_DATA，跳过（本用例只做手动复现）");
        return;
    };
    let node_ids: Vec<String> = env("VPCORE_REPRO_NODES")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if node_ids.is_empty() && env("VPCORE_REPRO_LATENCY_HTTP").is_none() {
        eprintln!("未设置 VPCORE_REPRO_NODES / VPCORE_REPRO_LATENCY_HTTP，没有可编译的目标节点");
        return;
    }

    let mut db = crate::db::Database::connect(&format!("{data_dir}\\vpn-parser.db"))
        .await
        .expect("open db");
    db.node_root = std::path::PathBuf::from(format!("{data_dir}\\subscriptions"));

    let options = db.get_proxy_config().await.expect("proxy config");
    let nodes = db.load_nodes(None).await.expect("nodes");
    eprintln!(
        "mode={:?} pref={:?} routing={:?} url_test={} url_test_size={}",
        options.mode,
        options.kernel_preference,
        options.routing_mode,
        options.url_test,
        options.url_test_size
    );
    eprintln!("nodes loaded: {}", nodes.len());

    let mut targets: Vec<(String, &crate::models::Node)> = Vec::new();
    for id in &node_ids {
        let node = nodes
            .iter()
            .find(|n| &n.id == id)
            .unwrap_or_else(|| panic!("node {id} not in pool"));
        targets.push((id.clone(), node));
    }
    if env("VPCORE_REPRO_LATENCY_HTTP").as_deref() == Some("1") {
        let fast_http = nodes
            .iter()
            .filter(|n| matches!(n.protocol.as_str(), "http" | "https"))
            .filter(|n| n.latency_ok == Some(true))
            .min_by_key(|n| n.latency_ms.unwrap_or(i64::MAX))
            .expect("a latency-tested http node");
        eprintln!(
            "fast http node: {} {} {}:{} latency={:?}",
            fast_http.name,
            fast_http.protocol,
            fast_http.server,
            fast_http.port,
            fast_http.latency_ms
        );
        targets.push(("http".into(), fast_http));
    }

    for (tag, node) in &targets {
        match select_proxy_kernel_preference_for_node(node, KernelPreference::VpCore, true, false) {
            Ok((kernel, notice)) => eprintln!(
                "{tag}({}) -> kernel={:?} notice={notice:?}",
                node.protocol, kernel
            ),
            Err(e) => eprintln!("{tag}({}) -> select error: {e}", node.protocol),
        }
    }

    for (tag, node) in &targets {
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

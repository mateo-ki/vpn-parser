use crate::hostutil::is_public_proxy_host;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

pub type ResolvedTargets = HashMap<(String, u16), Result<Vec<SocketAddr>, String>>;

/// 并发异步 DNS 预解析器：带去重与并发控制，优先排序 IPv4 地址
pub async fn resolve_targets_parallel(
    targets: Vec<(String, u16)>,
    network_limit: Arc<Semaphore>,
    dns_timeout: Duration,
) -> ResolvedTargets {
    let mut tasks = JoinSet::new();
    let mut seen = std::collections::HashSet::new();

    for (host, port) in targets {
        if is_public_proxy_host(&host) && seen.insert((host.clone(), port)) {
            let limit = network_limit.clone();
            tasks.spawn(async move {
                let _permit = match limit.acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => return ((host, port), Err("并发信号量已关闭".to_string())),
                };

                let target = format!("{host}:{port}");
                let res = match tokio::time::timeout(
                    dns_timeout,
                    tokio::net::lookup_host(target.as_str()),
                )
                .await
                {
                    Ok(Ok(iter)) => {
                        let mut addrs: Vec<SocketAddr> = iter.collect();
                        addrs.sort_by_key(|addr| if addr.is_ipv4() { 0u8 } else { 1u8 });
                        Ok(addrs)
                    }
                    Ok(Err(err)) => Err(format!("dns error: {err}")),
                    Err(_) => Err("dns timeout".into()),
                };
                ((host, port), res)
            });
        }
    }

    let mut resolved = HashMap::with_capacity(seen.len());
    while let Some(task_res) = tasks.join_next().await {
        if let Ok((key, res)) = task_res {
            resolved.insert(key, res);
        }
    }
    resolved
}

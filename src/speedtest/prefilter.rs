use crate::hostutil::is_public_proxy_host;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;

pub async fn connect_resolved(
    addresses: Vec<SocketAddr>,
    timeout: Duration,
) -> Result<i64, String> {
    let start = Instant::now();
    let mut last_error = String::from("connect failed");
    let total_candidates = addresses.len().min(2);

    for (idx, addr) in addresses.into_iter().take(2).enumerate() {
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            break;
        }

        // 多 IP（如 CDN / 双栈）时合理切分单 IP 超时预算，防止首个静默 IP 占满全部时间导致次选可用 IP 连尝试机会都没有
        let per_addr_budget = if total_candidates > 1 && idx == 0 {
            (timeout / 2).max(Duration::from_millis(600)).min(remaining)
        } else {
            remaining
        };

        match tokio::time::timeout(per_addr_budget, TcpStream::connect(addr)).await {
            Ok(Ok(_stream)) => {
                let latency_ms = start.elapsed().as_millis() as i64;
                return Ok(latency_ms.max(1));
            }
            Ok(Err(e)) => last_error = format!("connect error: {e}"),
            Err(_) => last_error = format!("timeout {}ms", per_addr_budget.as_millis()),
        }
    }

    Err(last_error)
}

/// 纯粹的 TCP 预筛探测：验证公网目标与端口可达性，测量 TCP 握手往返延迟。
/// 不再发送任何伪造的明文 HTTP 流量，避免对各种专有代理服务端造成协议解析异常。
pub async fn probe_full_resolved(
    host: &str,
    port: u16,
    timeout_ms: u32,
    addresses: Result<Vec<SocketAddr>, String>,
) -> (bool, Option<i64>, Option<f64>, String) {
    if !is_public_proxy_host(host) || port == 0 {
        return (
            false,
            None,
            None,
            format!("skip invalid/private host or port: {host}:{port}"),
        );
    }

    let addresses = match addresses {
        Ok(addrs) if !addrs.is_empty() => addrs,
        Ok(_) => return (false, None, None, "dns returned no addresses".into()),
        Err(err) => return (false, None, None, err),
    };

    let timeout = Duration::from_millis(timeout_ms.max(200) as u64);
    match connect_resolved(addresses, timeout).await {
        Ok(latency_ms) => (
            true,
            Some(latency_ms),
            None,
            format!("{latency_ms} ms (TCP)"),
        ),
        Err(msg) => (false, None, None, msg),
    }
}

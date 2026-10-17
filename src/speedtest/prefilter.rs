use crate::hostutil::is_public_proxy_host;
use crate::speedtest::download::measure_tcp_download;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;

pub async fn connect_resolved(
    addresses: Vec<SocketAddr>,
    timeout: Duration,
) -> Result<(TcpStream, i64), String> {
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
            Ok(Ok(stream)) => {
                let latency_ms = start.elapsed().as_millis() as i64;
                return Ok((stream, latency_ms.max(1)));
            }
            Ok(Err(e)) => last_error = format!("connect error: {e}"),
            Err(_) => last_error = format!("timeout {}ms", per_addr_budget.as_millis()),
        }
    }

    Err(last_error)
}

pub async fn probe_full_resolved(
    host: &str,
    port: u16,
    timeout_ms: u32,
    download_test_ms: u32,
    measure_download: bool,
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
    let (stream, latency_ms) = match connect_resolved(addresses, timeout).await {
        Ok(v) => v,
        Err(msg) => return (false, None, None, msg),
    };

    if !measure_download {
        return (
            true,
            Some(latency_ms),
            None,
            format!("{latency_ms} ms (TCP)"),
        );
    }

    let download_ms = download_test_ms.clamp(200, 5000) as u64;
    let (kbps, detail) = measure_tcp_download(stream, host, download_ms).await;
    let kbps = kbps.and_then(|k| {
        if k.is_finite() && k > 0.0 && k < 20_480.0 {
            Some(k)
        } else {
            None
        }
    });

    let message = match kbps {
        Some(value) => format!("{latency_ms} ms / {value:.1} KB/s"),
        None => format!("{latency_ms} ms / 下载不可测 ({detail})"),
    };
    (true, Some(latency_ms), kbps, message)
}

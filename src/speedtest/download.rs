use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn measure_tcp_download(
    mut stream: TcpStream,
    host: &str,
    download_ms: u64,
) -> (Option<f64>, String) {
    let req = format!(
        "GET / HTTP/1.1\r\nHost: {host}\r\nUser-Agent: curl/8.0\r\nConnection: close\r\n\r\n"
    );
    if let Err(e) = stream.write_all(req.as_bytes()).await {
        return (None, format!("发送探测失败: {e}"));
    }
    let _ = stream.flush().await;

    let deadline = Instant::now() + Duration::from_millis(download_ms);
    let mut buf = vec![0u8; 16 * 1024];
    let mut total: u64 = 0;
    let mut head = Vec::with_capacity(512);
    let read_start = Instant::now();

    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                if head.len() < 512 {
                    let take = (512 - head.len()).min(n);
                    head.extend_from_slice(&buf[..take]);
                }
                total += n as u64;
            }
            Ok(Err(_)) => break,
            Err(_) => break,
        }
    }

    let elapsed = read_start.elapsed().as_secs_f64().max(0.001);
    if total < 64 {
        return (None, format!("仅收到 {total} 字节"));
    }

    if looks_like_public_website(&head) {
        return (
            None,
            format!("疑似普通网站 HTTP 响应，不计入下载 ({total} bytes)"),
        );
    }

    let kbps = (total as f64 / 1024.0) / elapsed;
    (Some(kbps), format!("{total} bytes"))
}

pub fn looks_like_public_website(head: &[u8]) -> bool {
    let s = String::from_utf8_lossy(head);
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("http/1.") && !lower.starts_with("http/2") {
        return false;
    }
    let status_ok = lower.contains(" 200 ")
        || lower.contains(" 301 ")
        || lower.contains(" 302 ")
        || lower.contains(" 307 ")
        || lower.contains(" 308 ")
        || lower.contains(" 403 ")
        || lower.contains(" 404 ");
    if !status_ok {
        return false;
    }
    lower.contains("text/html")
        || lower.contains("text/plain")
        || lower.contains("application/json")
        || lower.contains("<!doctype html")
        || lower.contains("<html")
        || lower.contains("server: cloudflare")
        || lower.contains("server: nginx")
        || lower.contains("server: apache")
        || lower.contains("cf-ray:")
}

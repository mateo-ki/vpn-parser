//! Telegram 频道 → 节点。
//!
//! 走的是 t.me 的**公开网页预览**（`https://t.me/s/<频道名>`）：不需要账号、不需要
//! api_id/api_hash，代价是只能读到公开频道，而且一页只有最近约 20 条消息。
//!
//! 实测数据（这台机器，2026-10-07）：
//! - `t.me:443` 直连 TCP 握手超时，走本地代理才通 —— 所以这条通道必须挂代理跑；
//!   预览页给的证书是真货（`CN=*.t.me`，GoDaddy G2 签发），TLS 校验不用放宽；
//! - `https://t.me/s/v2node` → 200、115 KB、19 条正文，其中一条 3.7 KB 的正文里有
//!   15 个 `ss://` + 8 个 `vless://`，另有 1 个 `raw.githubusercontent.com/.../base64.txt` 订阅链接；
//! - 不存在的频道一律返回 9.6 KB 的空壳页、0 条正文，能一眼分辨；
//! - `?before=<消息号>` 确实往前翻，第 1 页 12610..12635、第 2 页 12582..12609，零重叠。

use super::{url_host, Downloaded, FetchCandidate, PARSE_LIMIT, RESPONSE_HEADER_TIMEOUT};
use crate::models::{FetchResult, Node, Source};
use crate::parser::parse_subscription;
use crate::tasks::TaskPhase;
use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest as _, Sha256};
use std::time::{Duration, Instant};

/// 第一次拉某个频道：往前翻 3 页（约 60 条消息），把历史里发过的节点一次性补上。
const SEED_PAGES: usize = 3;
/// 之后每次只取最新一页。频道节点是零散发布的，旧节点上一轮已经落库了，
/// 反复翻页只会让一次拉取慢好几秒。
const FOLLOW_PAGES: usize = 1;
/// 正文里的订阅链接最多跟进几个。闲聊频道能甩几十个外链，全跟会把预算花光。
const MAX_SUB_URLS: usize = 8;
/// 每个订阅链接单独给的时间上限；整轮仍然受该源的总预算约束。
const PER_SUB_URL: Duration = Duration::from_secs(8);

static BODY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)class="tgme_widget_message_text js-message_text"[^>]*>(.*?)</div>"#)
        .expect("valid telegram body regex")
});
static TAGS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]+>").expect("valid tag regex"));
static POST_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"data-post="([A-Za-z0-9_]+)/(\d+)""#).expect("valid post regex"));
static HREF: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)href="([^"]+)""#).expect("valid href regex"));
static NODE_URI: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?ix)\b(
              (?:ss|ssr|vmess|vless|trojan|tuic|hysteria2|hy2|socks5)
              ://[^\s"'<>，,；;）)（]+
          )"#,
    )
    .expect("valid node uri regex")
});
static HTTP_URL: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"https?://[^\s"'<>，,；;）)（]+"#).expect("valid url regex"));
static ENTITY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"&#(?:x([0-9a-fA-F]+)|(\d+));").expect("valid entity regex"));

/// `https://t.me/s/foo` / `t.me/foo` / `@foo` → `Some("foo")`。
/// 永久链接（`t.me/foo/12345`）也归到频道名 `foo`；私有频道（`t.me/+xxxx`、
/// `t.me/joinchat/xxxx`）网页预览根本读不到，所以不当频道源认。
pub fn channel_name(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(handle) = trimmed.strip_prefix('@') {
        return valid_channel(handle).map(str::to_owned);
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed.trim_start_matches('/'))
    };
    let parsed = reqwest::Url::parse(&with_scheme).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    if !matches!(
        host.as_str(),
        "t.me" | "telegram.me" | "telegram.org" | "telegram.dog"
    ) && !host.ends_with(".t.me")
    {
        return None;
    }
    let segments: Vec<&str> = parsed
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect();
    let name = if segments.first().copied() == Some("s") {
        *segments.get(1)?
    } else {
        *segments.first()?
    };
    if matches!(
        name,
        "joinchat"
            | "proxy"
            | "addstickers"
            | "addchain"
            | "addlist"
            | "share"
            | "iv"
            | "login"
            | "setlanguage"
            | "telegram"
            | "blog"
            | "faq"
            | "apps"
            | "premium"
            | "socks"
    ) || name.starts_with('+')
    {
        return None;
    }
    valid_channel(name).map(str::to_owned)
}

/// Telegram 用户名规则：首字符是字母，总长 5–32，只允许字母数字下划线。
fn valid_channel(name: &str) -> Option<&str> {
    if name.len() < 5 || name.len() > 32 {
        return None;
    }
    let mut bytes = name.bytes();
    if !bytes.next()?.is_ascii_alphabetic() {
        return None;
    }
    name.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        .then_some(name)
}

/// 预览页里每条消息的正文（还是 HTML 片段，未清洗）。
fn message_blocks(html: &str) -> Vec<String> {
    BODY.captures_iter(html)
        .filter_map(|captures| captures.get(1))
        .map(|matched| matched.as_str().to_string())
        .collect()
}

/// 这一页出现过的全部消息号，升序。翻页要拿最老的那条当游标。
fn message_ids(html: &str) -> Vec<u64> {
    let mut ids: Vec<u64> = POST_ID
        .captures_iter(html)
        .filter_map(|captures| captures[2].parse().ok())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 把消息正文拍成纯文本：`<br/>` 转换行，其余标签丢掉，实体最后才解 ——
/// 先解实体会把正文里的 `&lt;b&gt;` 当成标签吃掉。
fn plain_text(block: &str) -> String {
    let without_breaks = block
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n");
    let stripped = TAGS.replace_all(&without_breaks, "");
    decode_entities(&stripped)
}

fn decode_entities(text: &str) -> String {
    let out = text
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'");
    let out = ENTITY.replace_all(&out, |caps: &regex::Captures| {
        let code = caps
            .get(1)
            .and_then(|matched| u32::from_str_radix(matched.as_str(), 16).ok())
            .or_else(|| {
                caps.get(2)
                    .and_then(|matched| matched.as_str().parse().ok())
            });
        char::from_u32(code.unwrap_or(0xfffd))
            .unwrap_or('\u{fffd}')
            .to_string()
    });
    out.replace("&amp;", "&")
}

/// 从消息正文里分出两类载荷：裸节点 URI，和疑似订阅端点的 http(s) 链接。
/// 链接同时从可见文字和 `href` 里取 —— 频道常把真链接藏在"点击下方按钮打开"后面。
fn payloads(blocks: &[String]) -> (Vec<String>, Vec<String>) {
    let mut uris: Vec<String> = Vec::new();
    let mut urls: Vec<String> = Vec::new();
    for block in blocks {
        let text = plain_text(block);
        for matched in NODE_URI.captures_iter(&text) {
            push_unique(&mut uris, &matched[1]);
        }
        for href in HREF.captures_iter(block) {
            let decoded = decode_entities(&href[1]);
            if let Some(url) = subscription_url(&decoded) {
                push_unique(&mut urls, &url);
            }
        }
        for matched in HTTP_URL.captures_iter(&text) {
            let decoded = decode_entities(&matched[0]);
            if let Some(url) = subscription_url(&decoded) {
                push_unique(&mut urls, &url);
            }
        }
    }
    // 最像订阅端点的排前面，然后才受 MAX_SUB_URLS 截断。
    urls.sort_by(|a, b| {
        score_subscription_url(b)
            .cmp(&score_subscription_url(a))
            .then_with(|| a.cmp(b))
    });
    urls.truncate(MAX_SUB_URLS);
    (uris, urls)
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    let trimmed = value
        .trim()
        .trim_end_matches([')', '，', ',', '。', ';', '；']);
    if trimmed.is_empty() {
        return;
    }
    if !list.iter().any(|existing| existing == trimmed) {
        list.push(trimmed.to_string());
    }
}

/// 订阅候选链接：要真的像端点。Telegram 自己的域名（永久链接、@提及、转发）先排掉。
fn subscription_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    if host.ends_with("t.me")
        || host.ends_with("telegram.me")
        || host.ends_with("telegram.org")
        || host.ends_with("telegram.dog")
        || host == "localhost"
    {
        return None;
    }
    let path = parsed.path().to_ascii_lowercase();
    if [
        "png", "jpg", "jpeg", "gif", "webp", "svg", "ico", "css", "js", "woff", "woff2", "mp4",
    ]
    .iter()
    .any(|ext| path.ends_with(&format!(".{ext}")))
    {
        return None;
    }
    Some(url.to_string())
}

fn score_subscription_url(url: &str) -> u8 {
    let lower = url.to_ascii_lowercase();
    [
        "subscribe",
        "subscription",
        "clash",
        "mihomo",
        "singbox",
        "sing-box",
        "base64",
        "download",
        "node",
        "proxy",
        ".yaml",
        ".yml",
        ".txt",
        ".json",
    ]
    .iter()
    .filter(|needle| lower.contains(**needle))
    .count() as u8
}

/// 裸节点 URI 拼成解析器认识的正文（一条一行）。
fn raw_uri_text(uris: &[String]) -> String {
    uris.join("\n")
}

impl super::Collector {
    /// Telegram 频道源的一轮拉取。返回的 [`FetchResult`] 和普通源同构，所以任务面板、
    /// 健康度时间线、连续失败降级那些逻辑都不用改；重试和 fail_streak 记账也仍然由
    /// `fetch_source_in_task` 统一负责，这里不自己调。
    pub(crate) async fn fetch_telegram_source(
        &self,
        source: &Source,
        channel: &str,
        task_id: &str,
        budget: Duration,
    ) -> FetchResult {
        if self.via_proxy {
            return self
                .fetch_channel(source, channel, task_id, budget, "应用内代理")
                .await;
        }
        // 定时自动拉取（scheduler）拿的是不带代理的那份 collector，而 t.me 直连必挂。
        // 退一步认 Windows 系统代理：app 自己在系统代理模式下写的就是 9980，
        // v2rayN 之类外部工具指着也一样能用。只试这一条，不并发试多条。
        let through_system_proxy =
            tokio::task::spawn_blocking(|| crate::system_proxy::current_system_proxy_url())
                .await
                .ok()
                .flatten();
        match through_system_proxy {
            Some(url) => match self.with_proxy(&url) {
                Ok(via) => {
                    via.fetch_channel(source, channel, task_id, budget, "系统代理")
                        .await
                }
                Err(_) => {
                    self.telegram_failure(
                        source,
                        format!("https://t.me/s/{channel}"),
                        "系统代理地址无法用作出口".to_string(),
                    )
                    .await
                }
            },
            None => {
                self.telegram_failure(
                    source,
                    format!("https://t.me/s/{channel}"),
                    "Telegram 频道需要出口：这一轮既没走应用内代理，系统代理也没开。\
                     请在控制台启动代理后用「使用代理拉取」，或把系统代理指向一个能出境外的地址"
                        .to_string(),
                )
                .await
            }
        }
    }

    async fn fetch_channel(
        &self,
        source: &Source,
        channel: &str,
        task_id: &str,
        budget: Duration,
        egress: &str,
    ) -> FetchResult {
        let started = Instant::now();
        let failure_url = format!("https://t.me/s/{channel}");
        self.tasks
            .phase(task_id, &source.id, TaskPhase::Downloading);
        let first = self
            .fetch_page(
                source,
                &failure_url,
                task_id,
                budget.max(Duration::from_millis(500)),
            )
            .await;
        let downloaded = match first {
            Err(message) => {
                return self.telegram_failure(source, failure_url, message).await;
            }
            Ok(None) => {
                let _ = self
                    .db
                    .mark_source_fetched(&source.id, "not modified", source.last_node_count)
                    .await;
                return FetchResult {
                    source_id: source.id.clone(),
                    source_name: source.name.clone(),
                    resolved_url: failure_url,
                    success: true,
                    message: "not modified · 页面没变化，这一轮不重复解析".into(),
                    node_count: source.last_node_count.max(0) as usize,
                    extracted_rules: Vec::new(),
                };
            }
            Ok(Some(downloaded)) => downloaded,
        };

        let content_sha256 = format!("{:x}", Sha256::digest(&downloaded.body));
        let unchanged = source.content_sha256.as_deref() == Some(content_sha256.as_str())
            && source.last_resolved_url.as_deref() == Some(failure_url.as_str());
        let html = String::from_utf8_lossy(&downloaded.body).to_string();
        let mut blocks = message_blocks(&html);
        let mut ids = message_ids(&html);
        let mut pages_read = 1usize;

        if unchanged {
            let _ = self
                .db
                .mark_source_fetched(&source.id, "content unchanged", source.last_node_count)
                .await;
            return FetchResult {
                source_id: source.id.clone(),
                source_name: source.name.clone(),
                resolved_url: downloaded.final_url,
                success: true,
                message: format!(
                    "content unchanged · {} 条正文和上次一模一样，没有新东西",
                    blocks.len()
                ),
                node_count: source.last_node_count.max(0) as usize,
                extracted_rules: Vec::new(),
            };
        }

        let _ = self
            .db
            .update_source_cache(
                &source.id,
                &failure_url,
                downloaded.etag.as_deref(),
                downloaded.last_modified.as_deref(),
                Some(&content_sha256),
            )
            .await;

        // 第一次见到这个源就多翻几页补历史，之后只看最新一页。
        let wanted = if source.last_fetch_at.is_none() {
            SEED_PAGES
        } else {
            FOLLOW_PAGES
        };
        while pages_read < wanted {
            let Some(oldest) = ids.iter().min().copied() else {
                break;
            };
            if budget.saturating_sub(started.elapsed()) <= PER_SUB_URL {
                break;
            }
            let next = format!("https://t.me/s/{channel}?before={oldest}");
            let remaining = budget.saturating_sub(started.elapsed()).max(PER_SUB_URL);
            match self.fetch_page(source, &next, task_id, remaining).await {
                Ok(Some(page)) => {
                    let older = String::from_utf8_lossy(&page.body).to_string();
                    let older_ids = message_ids(&older);
                    if older_ids.is_empty() {
                        break;
                    }
                    blocks.extend(message_blocks(&older));
                    ids = older_ids;
                    pages_read += 1;
                }
                // 翻页是锦上添花：拿不到就用手上这一页继续，别把整轮判死。
                _ => break,
            }
        }

        let (uris, urls) = payloads(&blocks);
        self.tasks.phase(task_id, &source.id, TaskPhase::Parsing);
        let source_id = source.id.clone();
        let mut nodes = self
            .parse_blocking(raw_uri_text(&uris), &source_id)
            .await
            .unwrap_or_default();

        let mut hit = 0usize;
        let mut tried = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for url in &urls {
            let remaining = budget.saturating_sub(started.elapsed());
            if remaining <= Duration::from_millis(600) {
                break;
            }
            tried += 1;
            let per_url = remaining.min(PER_SUB_URL);
            match self.fetch_page(source, url, task_id, per_url).await {
                Ok(Some(page)) => {
                    let body = String::from_utf8_lossy(&page.body).to_string();
                    if let Some(parsed) = self.parse_blocking(body, &source_id).await {
                        if !parsed.is_empty() {
                            hit += 1;
                            nodes.extend(parsed);
                        }
                    }
                }
                // 跟进订阅链接只是加分项，挂了不判死整轮 —— 但原因要留在消息里，
                // 否则"这源只解出 16 个"根本分不清是频道没发还是网关不通。
                Err(message) => {
                    if failures.len() < 2 {
                        let host = url_host(url).unwrap_or_else(|| "?".to_string());
                        failures.push(format!("{host}: {message}"));
                    }
                }
                Ok(None) => {}
            }
        }

        let note = if failures.is_empty() {
            String::new()
        } else {
            format!("（{}）", failures.join(" / "))
        };
        let round = nodes.len();
        if round == 0 {
            let message = format!(
                "读了 {pages_read} 页 {} 条正文，一个节点都没解出来：这个频道可能把节点放在图片里、\
                 藏在按钮后面，或者只发口令；跟进的订阅链接 {hit}/{tried}{note}",
                blocks.len()
            );
            return self.telegram_failure(source, failure_url, message).await;
        }

        self.tasks.nodes(task_id, &source.id, round);
        self.tasks.phase(task_id, &source.id, TaskPhase::Writing);
        let stored = match self.db.accumulate_source_nodes(&source.id, &nodes).await {
            Ok(stored) => stored,
            Err(error) => {
                return self
                    .telegram_failure(source, failure_url, format!("store failed: {error}"))
                    .await;
            }
        };
        let _ = self
            .db
            .mark_source_fetched(&source.id, "ok", stored as i64)
            .await;
        let message = format!(
            "ok · 经{egress} · {pages_read} 页 {} 条正文 · 裸链接 {} 个 · 订阅链接 {hit}/{tried}{note} · 本轮解出 {round} · 分片总计 {stored}（只增不删）",
            blocks.len(),
            uris.len(),
        );
        FetchResult {
            source_id: source.id.clone(),
            source_name: source.name.clone(),
            resolved_url: downloaded.final_url,
            success: true,
            message,
            node_count: stored,
            extracted_rules: Vec::new(),
        }
    }

    /// 下载一页/一个订阅链接。`Ok(None)` 表示服务端回了 304。
    /// 并发闸和条件请求都在 `download_candidate` 里，这里不重复申请许可。
    async fn fetch_page(
        &self,
        source: &Source,
        url: &str,
        task_id: &str,
        budget: Duration,
    ) -> Result<Option<Downloaded>, String> {
        let candidate = FetchCandidate {
            url: url.to_string(),
            host: url_host(url).unwrap_or_default(),
            mirror: false,
        };
        self.download_candidate(source, &candidate, task_id, budget, RESPONSE_HEADER_TIMEOUT)
            .await
    }

    async fn parse_blocking(&self, text: String, source_id: &str) -> Option<Vec<Node>> {
        let permit = PARSE_LIMIT.clone().acquire_owned().await;
        let source_id = source_id.to_string();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            parse_subscription(&text, Some(&source_id))
        })
        .await
        .ok()
    }

    async fn telegram_failure(
        &self,
        source: &Source,
        resolved_url: String,
        message: String,
    ) -> FetchResult {
        let _ = self.db.mark_source_fetched(&source.id, &message, 0).await;
        FetchResult {
            source_id: source.id.clone(),
            source_name: source.name.clone(),
            resolved_url,
            success: false,
            message,
            node_count: 0,
            extracted_rules: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        // 结构照真实预览页：body div + data-post 游标 + <br/> 分行 + &quot; 实体，
        // 另外掺一条转发（js-message_reply_text，不该被当成本频道正文）。
        String::from(
            r#"<div class="tgme_widget_message_wrap js-widget_wrap">
  <div class="tgme_widget_message" data-post="demo/200">
    <a class="tgme_widget_message_reply" href="https://t.me/other/9">
      <div class="tgme_widget_message_text js-message_reply_text" dir="auto">转发来的 ss://dead@elsewhere:8388</div>
    </a>
    <div class="tgme_widget_message_text js-message_text" dir="auto">更新日期：2026-10-07 08:00<br/>
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQxQDE5OC4xOC4wLjE6ODM4OQ==#%F0%9F%87%BA%F0%9F%87%B8%20US-1<br/>
ss://YWVzLTI1Ni1nY206cGFzc3dvcmQyQDE5OC4xOC4wLjI6ODM4OQ==#US-2<br/>
vless://uuid-uuid-uuid@203.0.113.7:443?security=tls#DE-9<br/>
&quot;订阅链接&quot;：<br/>
<a href="https://nodebuf.com/files/public/cmuxccknp8l36oh5wl6t8s964/download" target="_blank">点击下方按钮打开</a><br/>
更多：<a href="https://t.me/demo/199">上一条</a> · <a href="https://t.me/SomeChannel">@SomeChannel</a><br/>
<img src="https://cdn.telegram.org/x/8FBB.png">
</div>
  </div>
  <div class="tgme_widget_message" data-post="demo/198">
    <div class="tgme_widget_message_text js-message_text" dir="auto">trojan://password7@example.org:443?allowInsecure=1#SG</div>
  </div>
</div>"#,
        )
    }

    #[test]
    fn channel_name_accepts_preview_page_plain_link_and_handle() {
        for url in [
            "https://t.me/s/v2node",
            "http://t.me/s/v2node",
            "t.me/s/v2node",
            "https://t.me/v2node",
            "https://t.me/v2node/12588",
            "@v2node",
            "https://telegram.me/s/V2freeNode",
        ] {
            let got = channel_name(url);
            let want = if url.contains("V2freeNode") {
                "V2freeNode"
            } else {
                "v2node"
            };
            assert_eq!(got.as_deref(), Some(want), "{url} 应该认成频道 {want}");
        }
    }

    #[test]
    fn channel_name_rejects_private_and_non_channel_links() {
        for url in [
            "https://t.me/+AbCdEfGhIjKlMnOp",
            "https://t.me/joinchat/AAAAAEag",
            "https://t.me/proxy?server=1.2.3.4",
            "https://t.me/addstickers/whatever",
            "https://github.com/some/repo/raw/main/sub.txt",
            "https://example.com/s/v2node",
            "not a url at all",
            "",
            "@ab",
            "@has space",
        ] {
            assert_eq!(channel_name(url), None, "{url} 不该被认成频道");
        }
    }

    #[test]
    fn message_blocks_skip_forwarded_quote_and_ids_are_ascending() {
        let html = fixture();
        let blocks = message_blocks(&html);
        assert_eq!(blocks.len(), 2, "只该取本频道的正文，转发的那条不算");
        assert!(
            !blocks.concat().contains("elsewhere"),
            "js-message_reply_text 的转发内容不该进来"
        );
        assert_eq!(message_ids(&html), vec![198, 200]);
    }

    #[test]
    fn payloads_split_raw_uris_from_subscription_urls() {
        let (uris, urls) = payloads(&message_blocks(&fixture()));
        assert_eq!(
            uris.len(),
            4,
            "3 条 ss/vless + 1 条 trojan，实测就是这两类混发：{uris:?}"
        );
        assert!(uris.iter().any(|u| u.starts_with("ss://")));
        assert!(uris.iter().any(|u| u.starts_with("vless://")));
        assert!(uris.iter().any(|u| u.starts_with("trojan://")));
        assert_eq!(
            urls,
            vec!["https://nodebuf.com/files/public/cmuxccknp8l36oh5wl6t8s964/download".to_string()],
            "t.me 永久链接、@提及和 telegram.org 图标都不算订阅端点"
        );
    }

    #[test]
    fn raw_uris_parse_into_nodes() {
        let (uris, _) = payloads(&message_blocks(&fixture()));
        let nodes = parse_subscription(&raw_uri_text(&uris), Some("demo"));
        assert_eq!(nodes.len(), 4, "每条 URI 都要变成节点：{uris:?}");
        assert!(nodes.iter().any(|node| node.port == 8389));
        assert!(nodes.iter().any(|node| node.server == "203.0.113.7"));
    }

    #[test]
    fn entities_are_decoded_without_swallowing_angle_brackets() {
        assert_eq!(
            plain_text("a &quot;ss://x@y:443&quot; &amp; b &lt;b&gt;c&lt;/b&gt;<br/>d"),
            "a \"ss://x@y:443\" & b <b>c</b>\nd"
        );
    }

    /// 真实网络：需要一个能出 t.me 的本地代理。`cargo test -- --ignored` 才跑。
    #[tokio::test]
    #[ignore]
    async fn real_network_telegram_channel_yields_nodes() {
        let proxy = std::env::var("VPN_PARSER_PROBE_PROXY")
            .unwrap_or_else(|_| "http://127.0.0.1:9980".to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("vpn-parser/1.0")
            .proxy(reqwest::Proxy::all(&proxy).unwrap())
            .build()
            .unwrap();
        let mut html = String::new();
        // 这条链路上境外握手会被间歇掐断（实测同一 URL 三次里有一次 tls handshake eof），
        // 探针本身就重试几次，别把出口的抖动报成解析坏了。
        for attempt in 0..3 {
            match client.get("https://t.me/s/v2node").send().await {
                Ok(response) => match response.text().await {
                    Ok(text) => {
                        html = text;
                        break;
                    }
                    Err(error) => println!("第 {} 次读正文失败: {error:#}", attempt + 1),
                },
                Err(error) => println!("第 {} 次请求失败: {error:#}", attempt + 1),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        assert!(!html.is_empty(), "三次都没能拿到预览页");
        let blocks = message_blocks(&html);
        assert!(
            blocks.len() > 5,
            "预览页应该有一二十条正文，实测 {:#?}",
            blocks.len()
        );
        let (uris, urls) = payloads(&blocks);
        assert!(
            !uris.is_empty() || !urls.is_empty(),
            "v2node 这一页至少有一条裸链接或订阅链接"
        );
    }

    /// 自动拉取那条路（scheduler 用的 collector 不带代理）能不能靠系统代理跑通。
    /// 没有系统代理时直接跳过并说明原因。`cargo test -- --ignored --nocapture` 才跑。
    #[tokio::test]
    #[ignore]
    async fn real_network_telegram_falls_back_to_system_proxy() {
        use crate::db::Database;
        use crate::models::CreateSource;
        use crate::tasks::TaskTracker;
        use uuid::Uuid;

        let Some(system_proxy) = crate::system_proxy::current_system_proxy_url() else {
            println!("跳过：这台机器现在没有开启系统代理");
            return;
        };
        let db_path = std::env::temp_dir().join(format!("vpn-parser-tgsys-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        // 故意用不带代理的那一份 collector。
        let collector =
            crate::collector::Collector::new(db.clone(), TaskTracker::default()).unwrap();
        let source = db
            .create_source(CreateSource {
                name: "TG 系统代理".into(),
                url_template: "@v2node".into(),
                enabled: true,
                remark: None,
            })
            .await
            .unwrap();

        let result = collector.fetch_one(&source.id).await.unwrap();
        println!(
            "系统代理 = {system_proxy} · success={} nodes={} · {}",
            result.success, result.node_count, result.message
        );
        assert!(
            result.message.contains("经系统代理"),
            "应该走系统代理这一档并把出口写在消息里：{}",
            result.message
        );
        assert!(result.success, "{}", result.message);
        assert!(result.node_count > 0);

        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(&db_path);
    }

    /// 端到端：整条 Telegram 源链路（拉页面 → 翻页 → 跟订阅链接 → 累积入库）。
    /// 需要一个能出 t.me 的本地代理，默认 127.0.0.1:9980，可用
    /// `VPN_PARSER_PROBE_PROXY` 覆盖。`cargo test -- --ignored --nocapture` 才跑。
    #[tokio::test]
    #[ignore]
    async fn real_network_telegram_source_fetches_and_accumulates() {
        use crate::db::Database;
        use crate::models::CreateSource;
        use crate::tasks::TaskTracker;
        use uuid::Uuid;

        let proxy = std::env::var("VPN_PARSER_PROBE_PROXY")
            .unwrap_or_else(|_| "http://127.0.0.1:9980".to_string());
        let db_path = std::env::temp_dir().join(format!("vpn-parser-tgreal-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let collector = crate::collector::Collector::new(db.clone(), TaskTracker::default())
            .unwrap()
            .with_proxy(&proxy)
            .unwrap();
        let source = db
            .create_source(CreateSource {
                name: "TG v2node".into(),
                url_template: "https://t.me/s/v2node".into(),
                enabled: true,
                remark: None,
            })
            .await
            .unwrap();

        let first = collector.fetch_one(&source.id).await.unwrap();
        println!(
            "第一轮: success={} nodes={} · {}",
            first.success, first.node_count, first.message
        );
        assert!(first.success, "真实网络下这一轮应该成功：{}", first.message);
        assert!(first.node_count > 0, "v2node 这一页应该有节点");

        let second = collector.fetch_one(&source.id).await.unwrap();
        println!(
            "第二轮: success={} nodes={} · {}",
            second.success, second.node_count, second.message
        );
        assert!(
            second.node_count >= first.node_count,
            "累积模式不该让分片变小：{} -> {}",
            first.node_count,
            second.node_count
        );

        let listed = db
            .list_nodes(None, Some(&source.id), None, None, None, 1, 500, None)
            .await
            .unwrap();
        println!("分片共 {} 个节点", listed.total);
        for item in listed.items.iter().take(3) {
            println!(
                "  {} · {}:{} · {:?}",
                item.name, item.server, item.port, item.country
            );
        }
        assert!(listed.total > 0);

        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(&db_path);
    }
}

use crate::db::Database;
use crate::models::{FetchResult, Source};
use crate::parser::{expand_url_template, parse_subscription};
use crate::tasks::{TaskKind, TaskPhase, TaskTracker};
use anyhow::Result;
use once_cell::sync::Lazy;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, Weak};
use std::time::Duration;
use std::time::Instant;
use tokio::task::JoinSet;
use tracing::{info, warn};

const DEFAULT_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// `raw.githubusercontent.com` usually fails by accepting TCP+TLS and then never
/// answering, which `connect_timeout` cannot catch. Waiting for the response headers
/// gets its own short budget so a hung peer costs seconds instead of the whole timeout.
const RESPONSE_HEADER_TIMEOUT: Duration = Duration::from_secs(6);
/// Gist 搜索页只能直连或走本地代理，没有镜像可赛跑；经代理节点 TLS 握手 + 首字节
/// 实测 4~6s 且波动大，6s 的默认头部预算会把还差一两秒就能成功的请求掐掉。
const GIST_SEARCH_HEADER_TIMEOUT: Duration = Duration::from_secs(15);
// Keep the user-visible 120s source deadline, but do not spend all of it on a
// peer that has sent headers and then stopped sending the response body.
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const FETCH_MAX_ATTEMPTS: usize = 3;
const FETCH_RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(300), Duration::from_millis(800)];
/// A source that hung out this many budgets in a row is treated as gone and only gets
/// a single short attempt, so "拉取全部" is not held up by dead subscriptions.
pub const DEGRADED_FAIL_STREAK: i64 = 3;
static PARSE_LIMIT: Lazy<Arc<tokio::sync::Semaphore>> =
    Lazy::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
static HOST_FETCH_LIMITS: Lazy<StdMutex<HashMap<String, Weak<tokio::sync::Semaphore>>>> =
    Lazy::new(|| StdMutex::new(HashMap::new()));
const PER_HOST_FETCH_CONCURRENCY: usize = 2;

const GITHUB_RAW_HOST: &str = "raw.githubusercontent.com";
/// GitHub raw 加速镜像。2026-09-28 实测：直连间歇性挂死时这两个仍能返回，
/// 且与 jsdelivr/ghproxy.net 的内容 sha256 一致。
const GITHUB_RAW_MIRRORS: &[&str] = &["gh-proxy.com", "ghfast.top"];
/// 这些主机的 raw 内容可以被镜像前缀代理（gist raw 已实测与直连 size 一致；
/// gist.github.com 的 HTML 搜索页镜像不代理，只能直连）。
const MIRRORABLE_RAW_HOSTS: &[&str] = &[GITHUB_RAW_HOST, "gist.githubusercontent.com"];
const GIST_SEARCH_HOST: &str = "gist.github.com";
const GIST_RAW_BASE: &str = "https://gist.githubusercontent.com";
/// 独立 Gist 发现：单轮扫描页数硬上限（搜索页每页 10 条，全量 100+ 页）。
const GIST_MAX_PAGES_HARD: usize = 300;
/// 单轮最多注册的新 gist 源数。
const GIST_MAX_NEW_PER_RUN: usize = 2000;
/// 发现任务在任务面板里使用的伪源 id。
pub mod telegram;

#[cfg(test)]
#[path = "tests_file_source.rs"]
mod tests_file_source;

pub const GIST_DISCOVERY_SOURCE_ID: &str = "__gist_discovery__";
/// 默认搜索词（q 参数会被 percent-encode）。
pub const GIST_DISCOVERY_DEFAULT_QUERY: &str = "vless//";
/// GUI 多选预设查询词，对应 parse_uri 支持的分享链接前缀。Gist 搜索不支持 OR，
/// 多协议只能按词逐轮扫描；词的干扰结果由试拉解析兜底（解不出节点就丢弃）。
/// clash/proxies 的目标是 Clash YAML gist，语言多为 YAML 而非 Text，因此所有
/// 查询统一不带语言过滤，靠解析兜底。
pub const GIST_DISCOVERY_QUERIES: &[&str] = &[
    "vless//",
    "vmess//",
    "ss//",
    "trojan//",
    "hysteria2//",
    "socks5//",
    "clash/proxies",
];
/// 镜像是给大量并发请求设计的公共 CDN，不占用按主机名限流的闸；这里只限总并发，
/// 避免 35 个 GitHub 源同时打满同一个镜像。
static MIRROR_LIMIT: Lazy<Arc<tokio::sync::Semaphore>> =
    Lazy::new(|| Arc::new(tokio::sync::Semaphore::new(6)));

#[derive(Clone, Debug)]
struct FetchCandidate {
    url: String,
    host: String,
    mirror: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SubscriptionUserinfo {
    pub upload: Option<u64>,
    pub download: Option<u64>,
    pub total: Option<u64>,
    pub expire: Option<chrono::DateTime<chrono::Utc>>,
}

pub fn parse_subscription_userinfo(header: &str) -> SubscriptionUserinfo {
    let mut info = SubscriptionUserinfo::default();
    for part in header.split(';') {
        if let Some((k, v)) = part.trim().split_once('=') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match k.as_str() {
                "upload" => info.upload = v.parse().ok(),
                "download" => info.download = v.parse().ok(),
                "total" => info.total = v.parse().ok(),
                "expire" => {
                    if let Ok(ts) = v.parse::<i64>() {
                        info.expire = chrono::DateTime::from_timestamp(ts, 0);
                    }
                }
                _ => {}
            }
        }
    }
    info
}

/// 一次成功下载的结果（还没解析/入库）。
struct Downloaded {
    body: Vec<u8>,
    final_url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    /// 走了哪个镜像；None 表示直连。
    via: Option<String>,
    userinfo: Option<SubscriptionUserinfo>,
}

#[derive(Clone)]
pub struct Collector {
    client: Client,
    db: Database,
    tasks: TaskTracker,
    /// 这一份 collector 的出站是不是显式走代理。Telegram 频道直连不通（实测
    /// t.me:443 握手超时），要靠这个标志在开口要网页之前就把原因说清楚。
    via_proxy: bool,
}

fn build_client(proxy_url: Option<&str>) -> Result<Client> {
    // 走代理时 CONNECT 隧道建立本身就要几秒，直连的 3s 快速失败太紧。
    let connect_timeout = if proxy_url.is_some() {
        Duration::from_secs(10)
    } else {
        CONNECT_TIMEOUT
    };
    let mut builder = Client::builder()
        .timeout(DEFAULT_FETCH_TIMEOUT)
        .connect_timeout(connect_timeout)
        .user_agent("vpn-parser/1.0 (+https://localhost)")
        .redirect(reqwest::redirect::Policy::limited(5));
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
    }
    Ok(builder.build()?)
}

impl Collector {
    pub fn new(db: Database, tasks: TaskTracker) -> Result<Self> {
        let client = build_client(None)?;
        Ok(Self {
            client,
            db,
            tasks,
            via_proxy: false,
        })
    }

    pub fn with_proxy(&self, proxy_url: &str) -> Result<Self> {
        Ok(Self {
            client: build_client(Some(proxy_url))?,
            db: self.db.clone(),
            tasks: self.tasks.clone(),
            via_proxy: true,
        })
    }

    pub async fn fetch_one_with_proxy(
        &self,
        source_id: &str,
        proxy_url: &str,
    ) -> Result<FetchResult> {
        self.with_proxy(proxy_url)?.fetch_one(source_id).await
    }

    pub async fn fetch_all_enabled_with_proxy(
        &self,
        proxy_url: &str,
        progress: Option<impl Fn(usize, usize) + Send + Sync>,
    ) -> Result<Vec<FetchResult>> {
        self.with_proxy(proxy_url)?
            .fetch_all_enabled_with_progress(progress)
            .await
    }

    async fn mark_source_fetched(&self, source_id: &str, status: &str, node_count: i64) {
        let _ = self
            .db
            .mark_source_fetched(source_id, status, node_count)
            .await;
    }

    fn fetch_source_tracked<'a>(
        &'a self,
        source: &'a Source,
        task_id: &'a str,
        attempt_budget: Duration,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = FetchResult> + Send + 'a>> {
        Box::pin(async move {
            let resolved_url = expand_url_template(&source.url_template);
            // t.me/s/<频道> 走单独一条通道：网页预览、翻页、只增不删。
            if let Some(channel) = telegram::channel_name(&resolved_url) {
                info!(name = %source.name, channel = %channel, "fetching telegram channel");
                return self
                    .fetch_telegram_source(source, &channel, task_id, attempt_budget)
                    .await;
            }
            info!(name = %source.name, url = %resolved_url, "fetching source");
            self.tasks
                .phase(task_id, &source.id, TaskPhase::Downloading);
            let download_started = Instant::now();
            let downloaded = match self
                .download_source(
                    source,
                    &resolved_url,
                    task_id,
                    attempt_budget,
                    RESPONSE_HEADER_TIMEOUT,
                )
                .await
            {
                Err(message) => {
                    warn!(name = %source.name, error = %message, "fetch failed");
                    self.mark_source_fetched(&source.id, &message, 0).await;
                    return FetchResult {
                        source_id: source.id.clone(),
                        source_name: source.name.clone(),
                        resolved_url,
                        success: false,
                        message,
                        node_count: 0,
                        extracted_rules: Vec::new(),
                    };
                }
                Ok(None) => {
                    self.mark_source_fetched(&source.id, "not modified", source.last_node_count)
                        .await;
                    return FetchResult {
                        source_id: source.id.clone(),
                        source_name: source.name.clone(),
                        resolved_url,
                        success: true,
                        message: "not modified, skipped parse/write".into(),
                        node_count: source.last_node_count.max(0) as usize,
                        extracted_rules: Vec::new(),
                    };
                }
                Ok(Some(downloaded)) => downloaded,
            };
            self.tasks.timings(
                task_id,
                &source.id,
                Some(download_started.elapsed().as_millis() as u64),
                None,
                None,
                None,
                None,
            );
            let Downloaded {
                body,
                final_url,
                etag,
                last_modified,
                via,
                userinfo,
            } = downloaded;
            if let Some(ui) = userinfo {
                let _ = self
                    .db
                    .update_source_userinfo(
                        &source.id,
                        ui.upload,
                        ui.download,
                        ui.total,
                        ui.expire,
                    )
                    .await;
            }
            let via_note = match via.as_deref() {
                Some(host) => format!("（经 {host} 加速，未与直连比对）"),
                None => String::new(),
            };
            let content_sha256 = format!("{:x}", Sha256::digest(&body));
            if source.last_resolved_url.as_deref() == Some(resolved_url.as_str())
                && source.content_sha256.as_deref() == Some(content_sha256.as_str())
            {
                let _ = self
                    .db
                    .update_source_cache(
                        &source.id,
                        &resolved_url,
                        etag.as_deref(),
                        last_modified.as_deref(),
                        Some(&content_sha256),
                    )
                    .await;
                self.mark_source_fetched(&source.id, "content unchanged", source.last_node_count)
                    .await;
                return FetchResult {
                    source_id: source.id.clone(),
                    source_name: source.name.clone(),
                    resolved_url: final_url,
                    success: true,
                    message: format!("content unchanged, skipped parse/write{via_note}"),
                    node_count: source.last_node_count.max(0) as usize,
                    extracted_rules: Vec::new(),
                };
            }
            self.tasks.phase(task_id, &source.id, TaskPhase::Parsing);
            let parse_started = Instant::now();
            let source_id = source.id.clone();
            let parse_permit = PARSE_LIMIT.clone().acquire_owned().await;
            let (nodes, extracted_rules, raw_rules_yaml, raw_groups_yaml) = match tokio::task::spawn_blocking(move || {
                let _permit = parse_permit;
                let body = String::from_utf8_lossy(&body);
                let (rules_yaml, groups_yaml) = crate::parser::extract_subscription_rules_and_groups(&body);
                let rules = crate::parser::extract_subscription_rules(&body);
                let nodes = parse_subscription(&body, Some(&source_id));
                (nodes, rules, rules_yaml, groups_yaml)
            })
            .await
            {
                Ok(tuple) => tuple,
                Err(error) => {
                    let message = format!("parse task failed: {error}");
                    self.mark_source_fetched(&source.id, &message, 0).await;
                    return FetchResult {
                        source_id: source.id.clone(),
                        source_name: source.name.clone(),
                        resolved_url,
                        success: false,
                        message,
                        node_count: 0,
                        extracted_rules: Vec::new(),
                    };
                }
            };
            self.tasks.timings(
                task_id,
                &source.id,
                None,
                Some(parse_started.elapsed().as_millis() as u64),
                None,
                None,
                None,
            );
            let count = nodes.len();
            self.tasks.nodes(task_id, &source.id, count);
            self.tasks.phase(task_id, &source.id, TaskPhase::Writing);
            let write_started = Instant::now();
            match self.db.replace_source_nodes(&source.id, &nodes).await {
                Ok(inserted) => {
                    self.tasks.timings(
                        task_id,
                        &source.id,
                        None,
                        None,
                        Some(write_started.elapsed().as_millis() as u64),
                        None,
                        None,
                    );
                    let _ = self
                        .db
                        .mark_source_fetched(&source.id, "ok", count as i64)
                        .await;
                    let _ = self
                        .db
                        .update_source_cache(
                            &source.id,
                            &resolved_url,
                            etag.as_deref(),
                            last_modified.as_deref(),
                            Some(&content_sha256),
                        )
                        .await;
                    let _ = self
                        .db
                        .update_source_rules_and_groups(
                            &source.id,
                            raw_rules_yaml.as_deref(),
                            raw_groups_yaml.as_deref(),
                        )
                        .await;
                    FetchResult {
                        source_id: source.id.clone(),
                        source_name: source.name.clone(),
                        resolved_url: final_url,
                        success: true,
                        message: format!("ok, parsed={count}, stored={inserted}{via_note}"),
                        node_count: count,
                        extracted_rules,
                    }
                }
                Err(error) => {
                    let message = format!("store failed: {error}");
                    let _ = self.db.mark_source_fetched(&source.id, &message, 0).await;
                    FetchResult {
                        source_id: source.id.clone(),
                        source_name: source.name.clone(),
                        resolved_url,
                        success: false,
                        message,
                        node_count: count,
                        extracted_rules: Vec::new(),
                    }
                }
            }
        })
    }

    /// Race every candidate for this URL and keep the first body that arrives. A blocked
    /// `raw.githubusercontent.com` then costs one header timeout instead of the whole
    /// budget, because the mirror legs keep running in parallel.
    /// 读本地文件源：body 原样交给下游，mtime 记进 last_modified。
    async fn download_local_file(
        &self,
        path: &str,
        resolved_url: &str,
    ) -> Result<Option<Downloaded>, String> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| format!("读取本地文件失败 {path}: {e}"))?;
        let last_modified = std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        info!(path = %path, bytes = bytes.len(), "reading local file source");
        Ok(Some(Downloaded {
            body: bytes,
            final_url: resolved_url.to_string(),
            etag: None,
            last_modified,
            via: None,
            userinfo: None,
        }))
    }

    async fn download_source(
        &self,
        source: &Source,
        resolved_url: &str,
        task_id: &str,
        budget: Duration,
        header_budget: Duration,
    ) -> Result<Option<Downloaded>, String> {
        // 本地文件源：直接读文件，复用下游同一套 sha256 去重 / 解析 / 写库管线
        if let Some(path) = local_file_path(resolved_url) {
            return self.download_local_file(&path, resolved_url).await;
        }
        let candidates = fetch_candidates(resolved_url, source.last_fetch_host.as_deref());
        if candidates.len() == 1 {
            return self
                .download_candidate(source, &candidates[0], task_id, budget, header_budget)
                .await;
        }
        self.race_candidates(source, &candidates, task_id, budget, header_budget)
            .await
    }

    async fn race_candidates(
        &self,
        source: &Source,
        candidates: &[FetchCandidate],
        task_id: &str,
        budget: Duration,
        header_budget: Duration,
    ) -> Result<Option<Downloaded>, String> {
        let mut tasks = JoinSet::new();
        for candidate in candidates.iter().cloned() {
            let collector = self.clone();
            let source = source.clone();
            let task_id = task_id.to_string();
            tasks.spawn(async move {
                let host = candidate.host.clone();
                let mirror = candidate.mirror;
                let outcome = collector
                    .download_candidate(&source, &candidate, &task_id, budget, header_budget)
                    .await;
                (host, mirror, outcome)
            });
        }
        let mut failures = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((host, _, Ok(downloaded))) => {
                    tasks.abort_all();
                    let _ = self.db.set_source_fetch_host(&source.id, &host).await;
                    return Ok(downloaded);
                }
                Ok((host, mirror, Err(message))) => failures.push(if mirror {
                    format!("{host}: {message}")
                } else {
                    format!("直连: {message}")
                }),
                Err(error) => failures.push(format!("候选任务失败: {error}")),
            }
        }
        Err(failures.join(" | "))
    }

    /// Fetch one candidate URL. `Ok(None)` means the server answered 304 Not Modified.
    async fn download_candidate(
        &self,
        source: &Source,
        candidate: &FetchCandidate,
        task_id: &str,
        budget: Duration,
        header_budget: Duration,
    ) -> Result<Option<Downloaded>, String> {
        let _permit = if candidate.mirror {
            MIRROR_LIMIT
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| "镜像并发闸已关闭".to_string())?
        } else {
            fetch_host_limit(&candidate.url)
                .acquire_owned()
                .await
                .map_err(|_| "主机并发闸已关闭".to_string())?
        };
        let mut request = self.client.get(&candidate.url).timeout(budget);
        let user_agent = source
            .user_agent
            .as_deref()
            .map(str::trim)
            .filter(|ua| !ua.is_empty())
            .unwrap_or("clash.meta");
        request = request.header(reqwest::header::USER_AGENT, user_agent);
        // 条件请求只对直连有意义：镜像的 ETag 语义不跟随上游。
        if !candidate.mirror && source.last_resolved_url.as_deref() == Some(candidate.url.as_str())
        {
            if let Some(etag) = source.last_etag.as_deref() {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = source.last_modified.as_deref() {
                request = request.header(IF_MODIFIED_SINCE, last_modified);
            }
        }
        let header_budget = budget.min(header_budget);
        let mut response = match tokio::time::timeout(header_budget, request.send()).await {
            Err(_elapsed) => {
                return Err(format!(
                    "响应头超时: {}s 内未收到响应（连接已建立但服务端未返回）",
                    header_budget.as_secs()
                ))
            }
            Ok(Err(error)) => {
                return Err(if error.is_timeout() && error.is_connect() {
                    format!("连接超时（{}s 内未完成连接）", CONNECT_TIMEOUT.as_secs())
                } else if error.is_timeout() {
                    format!("request timeout after {}s", budget.as_secs())
                } else {
                    format!("request failed: {error}")
                })
            }
            Ok(Ok(response)) => response,
        };
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!("HTTP {status}"));
        }
        let content_length = response.content_length();
        if source_body_too_large(content_length) {
            return Err(body_too_large_message(
                content_length.unwrap_or(0) as f64 / 1048576.0,
            ));
        }
        let final_url = response.url().to_string();
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let userinfo = response
            .headers()
            .get("subscription-userinfo")
            .or_else(|| response.headers().get("Subscription-Userinfo"))
            .and_then(|value| value.to_str().ok())
            .map(parse_subscription_userinfo);
        let body = match self
            .read_source_body(&mut response, task_id, &source.id, Instant::now())
            .await
        {
            Ok(body) => body,
            Err(error) => {
                return Err(format!(
                    "read body failed (content-length={}): {error:#}",
                    content_length
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ))
            }
        };
        if body.len() as u64 > MAX_SOURCE_BYTES {
            return Err(body_too_large_message(body.len() as f64 / 1048576.0));
        }
        Ok(Some(Downloaded {
            body,
            final_url,
            etag,
            last_modified,
            via: candidate.mirror.then(|| candidate.host.clone()),
            userinfo,
        }))
    }

    /// 独立的 Gist 发现：翻页扫描 gist.github.com 搜索结果（updated 倒序），每个 gist
    /// 先试拉一次，解析出节点的才注册成订阅源，节点随之进入节点列表。整页都已见过的
    /// 页会让扫描提前停止，所以日常开销只有前几页；用户删掉的源不会被重新加回。
    pub async fn discover_gists(
        &self,
        query: &str,
        max_pages: usize,
    ) -> Result<GistDiscoverySummary> {
        let search_url = format!(
            "https://{GIST_SEARCH_HOST}/search?o=desc&q={}&s=updated",
            utf8_percent_encode(query, NON_ALPHANUMERIC)
        );
        self.discover_gists_from(&search_url, GIST_RAW_BASE, max_pages)
            .await
    }

    async fn discover_gists_from(
        &self,
        first_page_url: &str,
        raw_base: &str,
        max_pages: usize,
    ) -> Result<GistDiscoverySummary> {
        let max_pages = max_pages.clamp(1, GIST_MAX_PAGES_HARD);
        let scratch =
            crate::models::Source::scratch(GIST_DISCOVERY_SOURCE_ID, "Gist 发现", first_page_url);
        let task_id = self.tasks.start(
            TaskKind::Fetch,
            [(scratch.id.clone(), "Gist 发现".to_string())],
        );
        let mut seen = self.db.seen_gist_ids().await?;
        let known_templates: Vec<String> = self
            .db
            .list_sources()
            .await?
            .into_iter()
            .map(|existing| existing.url_template)
            .collect();
        let mut summary = GistDiscoverySummary {
            pages_scanned: 0,
            registered: 0,
            nodes: 0,
            skipped: 0,
            rejected: 0,
            pending: 0,
            stopped_reason: String::new(),
        };
        let mut page_url = first_page_url.to_string();
        loop {
            summary.pages_scanned += 1;
            let page = match self
                .download_source(
                    &scratch,
                    &page_url,
                    &task_id,
                    DEFAULT_FETCH_TIMEOUT,
                    GIST_SEARCH_HEADER_TIMEOUT,
                )
                .await
            {
                Ok(Some(downloaded)) => downloaded.body,
                Ok(None) => {
                    summary.stopped_reason =
                        format!("第 {} 页返回 304，停止", summary.pages_scanned);
                    break;
                }
                Err(error) => {
                    summary.stopped_reason = format!(
                        "第 {} 页拉取失败（{error}），已停止扫描",
                        summary.pages_scanned
                    );
                    break;
                }
            };
            let gists = extract_gists(&String::from_utf8_lossy(&page));
            if gists.is_empty() {
                summary.stopped_reason = format!("第 {} 页没有 gist，停止", summary.pages_scanned);
                break;
            }
            let page_total = gists.len();
            let fresh: Vec<DiscoveredGist> = gists
                .into_iter()
                .filter(|gist| {
                    let marker = format!("/{}/{}", gist.user, gist.id);
                    !seen.contains(&gist.id)
                        && !known_templates
                            .iter()
                            .any(|template| template.contains(&marker))
                })
                .collect();
            summary.skipped += page_total - fresh.len();
            if fresh.is_empty() {
                summary.stopped_reason =
                    format!("第 {} 页全部已扫描过，停止", summary.pages_scanned);
                break;
            }
            let (registered, nodes, rejected, pending) = self
                .register_gist_batch(&fresh, raw_base, &task_id, &mut seen)
                .await;
            summary.registered += registered;
            summary.nodes += nodes;
            summary.rejected += rejected;
            summary.pending += pending;
            info!(
                page = summary.pages_scanned,
                registered, nodes, rejected, pending, "gist 发现页处理完成"
            );
            if summary.pages_scanned >= max_pages {
                summary.stopped_reason = format!("达到单轮页数上限 {max_pages}");
                break;
            }
            if summary.registered >= GIST_MAX_NEW_PER_RUN {
                summary.stopped_reason = format!("达到单轮注册上限 {GIST_MAX_NEW_PER_RUN}");
                break;
            }
            let mut next = reqwest::Url::parse(&page_url)
                .map_err(|error| anyhow::anyhow!("gist 搜索 URL 无法解析: {error}"))?;
            next.query_pairs_mut()
                .append_pair("page", &(summary.pages_scanned + 1).to_string());
            page_url = next.to_string();
        }
        if summary.stopped_reason.is_empty() {
            summary.stopped_reason = "扫描完成".into();
        }
        let message = format!(
            "Gist 发现完成：扫描 {} 页，新增 {} 个源（{} 节点），跳过已有 {}，无节点丢弃 {}，待重试 {}；{}",
            summary.pages_scanned,
            summary.registered,
            summary.nodes,
            summary.skipped,
            summary.rejected,
            summary.pending,
            summary.stopped_reason
        );
        info!(
            source = GIST_DISCOVERY_SOURCE_ID,
            message, "gist discovery finished"
        );
        self.tasks
            .complete(&task_id, GIST_DISCOVERY_SOURCE_ID, message);
        Ok(summary)
    }

    /// 一页内的 gist 并发试拉（页大小就是并发上限）。返回 (registered, nodes, rejected, pending)。
    async fn register_gist_batch(
        &self,
        gists: &[DiscoveredGist],
        raw_base: &str,
        task_id: &str,
        seen: &mut std::collections::HashSet<String>,
    ) -> (usize, usize, usize, usize) {
        let mut tasks = JoinSet::new();
        let raw_base = raw_base.to_string();
        for gist in gists.iter().cloned() {
            let collector = self.clone();
            let _task_id = task_id.to_string();
            let raw_base = raw_base.clone();
            tasks.spawn(async move {
                let created = match collector
                    .db
                    .create_source(crate::models::CreateSource {
                        name: gist.name.clone(),
                        url_template: format!("{raw_base}/{}/{}/raw", gist.user, gist.id),
                        enabled: true,
                        remark: Some("gist 自动发现".into()),
                        user_agent: None,
                    })
                    .await
                {
                    Ok(created) => created,
                    Err(error) => {
                        warn!(gist = %gist.id, error = %error, "gist 源注册失败");
                        return (gist, 0usize, GistOutcome::Failed);
                    }
                };
                let child_task = collector.tasks.start(
                    TaskKind::Fetch,
                    [(created.id.clone(), created.name.clone())],
                );
                let result = collector
                    .fetch_source_in_task(&created, &child_task, DEFAULT_FETCH_TIMEOUT)
                    .await;
                if result.success && result.node_count > 0 {
                    (gist, result.node_count, GistOutcome::Registered)
                } else if is_network_fetch_failure(&result.message) {
                    // 网络原因失败不算"无节点"，下轮重试。
                    let _ = collector.db.delete_source(&created.id).await;
                    (gist, 0, GistOutcome::Pending)
                } else {
                    let _ = collector.db.delete_source(&created.id).await;
                    (gist, 0, GistOutcome::Unusable)
                }
            });
        }
        let mut registered = 0usize;
        let mut nodes = 0usize;
        let mut rejected = 0usize;
        let mut pending = 0usize;
        while let Some(joined) = tasks.join_next().await {
            let Ok((gist, count, outcome)) = joined else {
                continue;
            };
            match outcome {
                GistOutcome::Registered => {
                    let _ = self
                        .db
                        .mark_gist_seen(&gist.id, &gist.user, &gist.name, "registered")
                        .await;
                    seen.insert(gist.id.clone());
                    registered += 1;
                    nodes += count;
                }
                GistOutcome::Unusable => {
                    let _ = self
                        .db
                        .mark_gist_seen(&gist.id, &gist.user, &gist.name, "unusable")
                        .await;
                    seen.insert(gist.id.clone());
                    rejected += 1;
                }
                GistOutcome::Pending => pending += 1,
                GistOutcome::Failed => {}
            }
        }
        (registered, nodes, rejected, pending)
    }

    async fn read_source_body(
        &self,
        response: &mut reqwest::Response,
        task_id: &str,
        source_id: &str,
        started: Instant,
    ) -> Result<Vec<u8>> {
        let mut body = Vec::with_capacity(
            response.content_length().unwrap_or(0).min(MAX_SOURCE_BYTES) as usize,
        );
        loop {
            let next = tokio::time::timeout(BODY_IDLE_TIMEOUT, response.chunk())
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "response body idle for {}s after {} bytes (overall limit {}s)",
                        BODY_IDLE_TIMEOUT.as_secs(),
                        body.len(),
                        DEFAULT_FETCH_TIMEOUT.as_secs()
                    )
                })??;
            let Some(chunk) = next else {
                break;
            };
            if body.len() as u64 + chunk.len() as u64 > MAX_SOURCE_BYTES {
                anyhow::bail!(
                    "response body too large: more than {} MiB",
                    MAX_SOURCE_BYTES / 1024 / 1024
                );
            }
            body.extend_from_slice(&chunk);
            // This is intentionally updated while downloading so the task panel
            // shows a changing value for slow but still active sources.
            self.tasks.timings(
                task_id,
                source_id,
                Some(started.elapsed().as_millis() as u64),
                None,
                None,
                None,
                None,
            );
        }
        Ok(body)
    }

    /// `total_budget` is the wall-clock budget for **all** attempts of this source, not
    /// per attempt: three full timeouts used to burn 90s while holding a host permit.
    async fn fetch_source_in_task(
        &self,
        source: &Source,
        task_id: &str,
        total_budget: Duration,
    ) -> FetchResult {
        let max_attempts = if source.fail_streak >= DEGRADED_FAIL_STREAK {
            1
        } else {
            FETCH_MAX_ATTEMPTS
        };
        let started = Instant::now();
        let mut attempts = 1usize;
        let mut result = self
            .fetch_attempt(source, task_id, total_budget, started)
            .await;
        for delay in FETCH_RETRY_DELAYS.into_iter().take(max_attempts - 1) {
            if result.success || !is_retryable_fetch_failure(&result.message) {
                break;
            }
            if started.elapsed() + delay >= total_budget {
                break;
            }
            warn!(
                source = %source.name,
                error = %result.message,
                delay_ms = delay.as_millis() as u64,
                "retrying transient subscription fetch failure"
            );
            tokio::time::sleep(delay).await;
            result = self
                .fetch_attempt(source, task_id, total_budget, started)
                .await;
            attempts += 1;
        }
        if attempts > 1 && !result.success && is_retryable_fetch_failure(&result.message) {
            result.message = format!("{}（已自动尝试 {attempts} 次）", result.message);
        }
        let network_failure = !result.success && is_network_fetch_failure(&result.message);
        match self
            .db
            .record_fetch_outcome(&source.id, result.success, network_failure)
            .await
        {
            Ok(streak) if streak >= DEGRADED_FAIL_STREAK && streak != source.fail_streak => {
                warn!(
                    source = %source.name,
                    streak, "订阅源连续网络超时，后续拉取只单次尝试"
                );
            }
            Err(error) => warn!(source = %source.name, error = %error, "记录拉取结果失败"),
            _ => {}
        }
        if result.success {
            self.tasks
                .complete(task_id, &source.id, result.message.clone());
        } else {
            self.tasks.fail(task_id, &source.id, result.message.clone());
        }
        result
    }

    /// A single attempt, capped by whatever is left of this source's total budget.
    async fn fetch_attempt(
        &self,
        source: &Source,
        task_id: &str,
        total_budget: Duration,
        started: Instant,
    ) -> FetchResult {
        let remaining = total_budget.saturating_sub(started.elapsed());
        self.fetch_source_tracked(source, task_id, remaining.max(Duration::from_millis(500)))
            .await
    }

    pub async fn fetch_source(&self, source: &Source) -> FetchResult {
        let task_id = self
            .tasks
            .start(TaskKind::Fetch, [(source.id.clone(), source.name.clone())]);
        let timeout = self
            .db
            .get_settings()
            .await
            .map(|settings| Duration::from_secs(settings.fetch_timeout_secs as u64))
            .unwrap_or(DEFAULT_FETCH_TIMEOUT);
        self.fetch_source_in_task(source, &task_id, timeout).await
    }

    pub async fn fetch_all_enabled(&self) -> Result<Vec<FetchResult>> {
        self.fetch_all_enabled_with_progress(None::<fn(usize, usize)>)
            .await
    }

    pub async fn fetch_all_enabled_with_progress(
        &self,
        progress: Option<impl Fn(usize, usize) + Send + Sync>,
    ) -> Result<Vec<FetchResult>> {
        let sources: Vec<_> = self
            .db
            .list_sources()
            .await?
            .into_iter()
            .filter(|source| source.enabled)
            .collect();
        let total = sources.len();
        let settings = self.db.get_settings().await.unwrap_or_default();
        let fetch_concurrency = settings.fetch_concurrency.clamp(1, 8) as usize;
        let request_timeout =
            Duration::from_secs(settings.fetch_timeout_secs.clamp(10, 120) as u64);
        let task_id = self.tasks.start(
            TaskKind::Fetch,
            sources
                .iter()
                .map(|source| (source.id.clone(), source.name.clone())),
        );
        let source_metadata: Vec<_> = sources
            .iter()
            .map(|source| {
                (
                    source.id.clone(),
                    source.name.clone(),
                    expand_url_template(&source.url_template),
                )
            })
            .collect();
        let mut pending = sources.into_iter().enumerate();
        let mut tasks = JoinSet::new();
        let mut results = vec![None; total];
        let mut done = 0usize;

        loop {
            while tasks.len() < fetch_concurrency {
                let Some((index, source)) = pending.next() else {
                    break;
                };
                let collector = self.clone();
                let task_id = task_id.clone();
                tasks.spawn(async move {
                    (
                        index,
                        collector
                            .fetch_source_in_task(&source, &task_id, request_timeout)
                            .await,
                    )
                });
            }

            let Some(joined) = tasks.join_next().await else {
                break;
            };
            done += 1;
            if let Ok((index, result)) = joined {
                results[index] = Some(result);
            }
            if let Some(progress) = progress.as_ref() {
                progress(done, total);
            }
        }

        Ok(results
            .into_iter()
            .enumerate()
            .map(|(index, result)| {
                result.unwrap_or_else(|| {
                    let (source_id, source_name, resolved_url) = &source_metadata[index];
                    let message = "拉取子任务异常退出，已结束该任务，可点击重试".to_string();
                    self.tasks.fail(&task_id, source_id, message.clone());
                    FetchResult {
                        source_id: source_id.clone(),
                        source_name: source_name.clone(),
                        resolved_url: resolved_url.clone(),
                        success: false,
                        message,
                        node_count: 0,
                        extracted_rules: Vec::new(),
                    }
                })
            })
            .collect())
    }

    pub async fn fetch_one(&self, source_id: &str) -> Result<FetchResult> {
        let source = self
            .db
            .get_source(source_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("source not found"))?;
        Ok(self.fetch_source(&source).await)
    }
}
fn source_body_too_large(content_length: Option<u64>) -> bool {
    content_length.is_some_and(|length| length > MAX_SOURCE_BYTES)
}

/// Transport-level failures only — what a dead or blocked host produces. These are the
/// ones that count towards a source's fail streak; a 404 or a parse error is not proof
/// the host is unreachable.
fn is_network_fetch_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("timeout")
        || message.contains("timed out")
        || message.contains("超时")
        || message.contains("connect")
        || message.contains("connection")
        || message.contains("request failed")
        || message.contains("error sending request")
        || message.contains("body read failed")
        || message.contains("read body failed")
}

fn is_retryable_fetch_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    is_network_fetch_failure(message)
        || lower.starts_with("http 408")
        || lower.starts_with("http 429")
        || (500..=599).any(|status| lower.starts_with(&format!("http {status}")))
}

fn url_host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h1), Some(h2)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push((h1 * 16 + h2) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 本地文件源识别：支持 `file://`、裸本地路径（Windows 盘符 / Unix 绝对路径 / ~ 家目录）。
/// 支持引号剥离、URL 编码字符解码（如 %20）、波浪号展开，返回操作系统可直接读取的真实路径。
pub fn local_file_path(url: &str) -> Option<String> {
    let mut s = url.trim();
    // 0. 剥离可能存在的首尾包裹单双引号（从终端或文件管理器复制带空格路径时常见）
    while (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        if s.len() >= 2 {
            s = s[1..s.len() - 1].trim();
        } else {
            break;
        }
    }
    if s.is_empty() {
        return None;
    }

    // 1. 排除网络协议与非本地链接
    let s_low = s.to_ascii_lowercase();
    if s_low.starts_with("http://")
        || s_low.starts_with("https://")
        || s_low.starts_with("ftp://")
        || s_low.starts_with("t.me/")
        || s_low.starts_with("tg://")
    {
        return None;
    }

    // 2. 解构 file:// 协议与 URL 解码（支持 file:///path, file://localhost/path 以及 %20 等空格）
    let unescaped = if let Some(mut rest) = s.strip_prefix("file://") {
        if let Some(after_host) = rest.strip_prefix("localhost") {
            rest = after_host;
        }
        let decoded = percent_decode(rest);
        // 在 Windows 风格下：file:///C:/path -> /C:/path -> 剥去开头的 /
        if let Some(stripped) = decoded.strip_prefix('/') {
            if stripped.len() >= 2 && stripped.as_bytes()[1] == b':' {
                stripped.to_string()
            } else {
                decoded
            }
        } else {
            decoded
        }
    } else {
        s.to_string()
    };

    // 3. 处理 ~ 家目录扩展（例如 macOS / Linux 常见的 ~/Downloads/clash.yaml）
    let path_str = if unescaped == "~" || unescaped.starts_with("~/") || unescaped.starts_with("~\\") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from);
        if let Some(home_path) = home {
            if unescaped == "~" {
                home_path.to_string_lossy().to_string()
            } else {
                let sub = &unescaped[2..];
                home_path.join(sub).to_string_lossy().to_string()
            }
        } else {
            unescaped
        }
    } else {
        unescaped
    };

    // 4. 判定是否为合法的本地文件系统路径
    let b = path_str.as_bytes();
    let is_windows_drive = b.len() >= 2 && b[1] == b':' && (b[0] as char).is_ascii_alphabetic();
    let is_unix_abs = path_str.starts_with('/') && !path_str.starts_with("//");
    let is_windows_unc = path_str.starts_with('\\');
    let exists_on_disk = std::path::Path::new(&path_str).is_file();

    if is_windows_drive || is_unix_abs || is_windows_unc || exists_on_disk {
        Some(path_str)
    } else {
        None
    }
}

fn fetch_candidates(resolved_url: &str, winner: Option<&str>) -> Vec<FetchCandidate> {
    let mut candidates = vec![FetchCandidate {
        url: resolved_url.to_string(),
        host: url_host(resolved_url).unwrap_or_default(),
        mirror: false,
    }];
    if MIRRORABLE_RAW_HOSTS.contains(&candidates[0].host.as_str()) {
        candidates.extend(GITHUB_RAW_MIRRORS.iter().map(|mirror| FetchCandidate {
            url: format!("https://{mirror}/{resolved_url}"),
            host: (*mirror).to_string(),
            mirror: true,
        }));
    }
    if let Some(winner) = winner {
        let index = candidates.iter().position(|c| c.host == winner);
        if let Some(index) = index.filter(|index| *index > 0) {
            let preferred = candidates.remove(index);
            candidates.insert(0, preferred);
        }
    }
    candidates
}

fn body_too_large_message(size_mib: f64) -> String {
    format!(
        "response body too large: {size_mib:.1} MiB (limit {} MiB)",
        MAX_SOURCE_BYTES / 1024 / 1024
    )
}

#[derive(Clone, Debug, PartialEq)]
struct DiscoveredGist {
    user: String,
    id: String,
    name: String,
}

#[derive(Clone, Copy, PartialEq)]
enum GistOutcome {
    Registered,
    Unusable,
    Pending,
    Failed,
}

/// 一次 Gist 发现的汇总。
#[derive(Debug, Clone, serde::Serialize)]
pub struct GistDiscoverySummary {
    pub pages_scanned: usize,
    pub registered: usize,
    pub nodes: usize,
    pub skipped: usize,
    pub rejected: usize,
    pub pending: usize,
    pub stopped_reason: String,
}

/// 从 gist 搜索结果页提取 gist。每条结果是 `<div class="gist-snippet">` 块，
/// 块内的 `href="/{user}/{id}"` 是 gist 本体（其余指向 forks/stars 等子路径）。
fn extract_gists(search_html: &str) -> Vec<DiscoveredGist> {
    static SNIPPET_SPLIT: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new(r#"<div class="gist-snippet">"#).expect("valid gist snippet regex")
    });
    static GIST_ANCHOR: Lazy<regex::Regex> = Lazy::new(|| {
        // 行尾的引号天然排除了 /stargazers 之类子路径：id 后面跟的不是引号就不匹配。
        regex::Regex::new(r#"href="/([A-Za-z0-9_-]+)/([0-9a-f]{16,40})""#)
            .expect("valid gist anchor regex")
    });
    static FILE_NAME: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new(r#">([A-Za-z0-9._-]{4,80}\.(?:txt|yaml|yml|md|json|list|conf|csv))<"#)
            .expect("valid gist filename regex")
    });

    let mut found: Vec<DiscoveredGist> = Vec::new();
    for block in SNIPPET_SPLIT.split(search_html).skip(1) {
        let Some(captures) = GIST_ANCHOR.captures(block) else {
            continue;
        };
        let user = captures[1].to_string();
        let id = captures[2].to_string();
        if found.iter().any(|gist| gist.id == id) {
            continue;
        }
        let name = FILE_NAME
            .captures(block)
            .map(|captures| captures[1].to_string())
            .unwrap_or_else(|| format!("gist-{user}-{}", &id[..8.min(id.len())]));
        found.push(DiscoveredGist { user, id, name });
    }
    found
}

fn fetch_host_limit(url: &str) -> Arc<tokio::sync::Semaphore> {
    let key = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| {
            let host = url.host_str()?.to_ascii_lowercase();
            if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]" {
                if let Some(port) = url.port() {
                    return Some(format!("{host}:{port}"));
                }
            }
            Some(host)
        })
        .unwrap_or_else(|| url.to_ascii_lowercase());
    let mut limits = HOST_FETCH_LIMITS
        .lock()
        .expect("subscription host limit lock poisoned");
    limits.retain(|_, limit| limit.strong_count() > 0);
    if let Some(limit) = limits.get(&key).and_then(Weak::upgrade) {
        return limit;
    }
    let limit = Arc::new(tokio::sync::Semaphore::new(PER_HOST_FETCH_CONCURRENCY));
    limits.insert(key, Arc::downgrade(&limit));
    limit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateSource;
    use axum::body::Body;
    use axum::http::{HeaderMap, Response};
    use axum::{routing::get, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    #[test]
    fn test_parse_subscription_userinfo() {
        let header = "upload=1073741824; download=2147483648; total=107374182400; expire=1700000000";
        let info = parse_subscription_userinfo(header);
        assert_eq!(info.upload, Some(1073741824));
        assert_eq!(info.download, Some(2147483648));
        assert_eq!(info.total, Some(107374182400));
        assert!(info.expire.is_some());
    }

    #[test]
    fn rejects_declared_source_body_over_limit() {
        assert_eq!(DEFAULT_FETCH_TIMEOUT, Duration::from_secs(30));
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(3));
        assert_eq!(BODY_IDLE_TIMEOUT, Duration::from_secs(5));
        assert_eq!(RESPONSE_HEADER_TIMEOUT, Duration::from_secs(6));
        assert_eq!(DEGRADED_FAIL_STREAK, 3);
        assert!(!source_body_too_large(Some(16 * 1024 * 1024)));
        assert!(!source_body_too_large(Some(MAX_SOURCE_BYTES)));
        assert!(source_body_too_large(Some(MAX_SOURCE_BYTES + 1)));
        assert!(!source_body_too_large(None));
    }

    #[test]
    fn retries_only_transient_fetch_failures() {
        for message in [
            "HTTP 408 Request Timeout",
            "HTTP 429 Too Many Requests",
            "HTTP 500 Internal Server Error",
            "request failed: connection reset",
            "body read failed: timeout",
        ] {
            assert!(is_retryable_fetch_failure(message), "{message}");
        }
        for message in ["HTTP 404 Not Found", "parse task failed", "no nodes parsed"] {
            assert!(!is_retryable_fetch_failure(message), "{message}");
        }
    }

    async fn bind_test_listener() -> TcpListener {
        let port = {
            let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            std_listener.local_addr().unwrap().port()
        };
        TcpListener::bind(format!("127.0.0.1:{port}")).await.unwrap()
    }

    #[tokio::test]
    async fn transient_http_failure_is_retried_and_task_finishes() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler_requests = requests.clone();
        let app = Router::new().route(
            "/subscription",
            get(move || {
                let requests = handler_requests.clone();
                async move {
                    if requests.fetch_add(1, Ordering::SeqCst) == 0 {
                        return Response::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .body(Body::empty())
                            .unwrap();
                    }
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from("ss://YWVzLTEyOC1nY206cGFzcw==@1.2.3.4:443#node"))
                        .unwrap()
                }
            }),
        );
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "retry".into(),
                url_template: format!("http://{address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let tracker = TaskTracker::default();
        let collector = Collector::new(db.clone(), tracker.clone()).unwrap();

        let result = collector.fetch_source(&source).await;

        assert!(result.success, "{}", result.message);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        let task = tracker.snapshots(1).pop().unwrap();
        assert_eq!(task.successful_sources, 1);
        assert!(task.finished_at.is_some());
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn only_network_failures_count_towards_the_fail_streak() {
        for message in [
            "响应头超时: 6s 内未收到响应（连接已建立但服务端未返回）",
            "request timeout after 30s",
            "request failed: error sending request for url",
            "read body failed: timeout",
        ] {
            assert!(is_network_fetch_failure(message), "{message}");
            assert!(is_retryable_fetch_failure(message), "{message}");
        }
        for message in [
            "HTTP 404 Not Found",
            "parse task failed",
            "no nodes parsed from subscription",
            "response body too large: 80.0 MiB (limit 64 MiB)",
        ] {
            assert!(!is_network_fetch_failure(message), "{message}");
        }
    }

    #[tokio::test]
    async fn hanging_source_is_capped_by_the_total_budget() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler = requests.clone();
        let app = Router::new().fallback(get(move || {
            let requests = handler.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                // Accept the connection and never answer: what a blocked
                // raw.githubusercontent.com does, and what connect_timeout cannot catch.
                tokio::time::sleep(Duration::from_secs(600)).await;
                ""
            }
        }));
        // IPv6 loopback keeps this test on its own per-host semaphore, away from the
        // 127.0.0.1 tests that share the process-wide host limit.
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "hang".into(),
                url_template: format!("http://{address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let tracker = TaskTracker::default();
        let collector = Collector::new(db.clone(), tracker.clone()).unwrap();
        let task_id = tracker.start(TaskKind::Fetch, [(source.id.clone(), source.name.clone())]);
        let budget = Duration::from_millis(600);

        let started = Instant::now();
        let result = collector
            .fetch_source_in_task(&source, &task_id, budget)
            .await;

        assert!(!result.success, "{}", result.message);
        assert!(
            started.elapsed() < budget + Duration::from_millis(800),
            "总预算被突破: {:?}",
            started.elapsed()
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            db.get_source(&source.id)
                .await
                .unwrap()
                .unwrap()
                .fail_streak,
            1
        );
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn network_failure_counts_one_streak_per_fetch() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "dead".into(),
                url_template: format!("http://{address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();
        let task_id = collector
            .tasks
            .start(TaskKind::Fetch, [(source.id.clone(), source.name.clone())]);

        let result = collector
            .fetch_source_in_task(&source, &task_id, Duration::from_secs(5))
            .await;

        assert!(!result.success, "{}", result.message);
        assert!(
            result.message.contains("request failed"),
            "{}",
            result.message
        );
        // One fetch advances the streak by one, no matter how many attempts it spent.
        assert_eq!(
            db.get_source(&source.id)
                .await
                .unwrap()
                .unwrap()
                .fail_streak,
            1
        );
        assert_eq!(
            db.record_fetch_outcome(&source.id, true, false)
                .await
                .unwrap(),
            0
        );
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn degraded_source_gets_a_single_attempt() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler = requests.clone();
        let app = Router::new().fallback(get(move || {
            let requests = handler.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::empty())
                    .unwrap()
            }
        }));
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "flaky".into(),
                url_template: format!("http://{address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();
        let task_id = collector
            .tasks
            .start(TaskKind::Fetch, [(source.id.clone(), source.name.clone())]);
        let budget = Duration::from_secs(5);

        let result = collector
            .fetch_source_in_task(&source, &task_id, budget)
            .await;
        assert!(!result.success, "{}", result.message);
        assert_eq!(requests.load(Ordering::SeqCst), FETCH_MAX_ATTEMPTS);
        assert!(
            result
                .message
                .contains(&format!("已自动尝试 {FETCH_MAX_ATTEMPTS} 次")),
            "{}",
            result.message
        );
        // A 5xx is transient, not an unreachable host: it must not feed the streak.
        assert_eq!(
            db.get_source(&source.id)
                .await
                .unwrap()
                .unwrap()
                .fail_streak,
            0
        );

        for _ in 0..DEGRADED_FAIL_STREAK {
            db.record_fetch_outcome(&source.id, false, true)
                .await
                .unwrap();
        }
        let degraded = db.get_source(&source.id).await.unwrap().unwrap();
        assert_eq!(degraded.fail_streak, DEGRADED_FAIL_STREAK);
        let result = collector
            .fetch_source_in_task(&degraded, &task_id, budget)
            .await;
        assert!(!result.success, "{}", result.message);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            FETCH_MAX_ATTEMPTS + 1,
            "降权后只应再请求一次"
        );
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn github_raw_urls_get_mirror_candidates() {
        let raw = "https://raw.githubusercontent.com/anaer/Sub/main/clash.yaml";
        let candidates = fetch_candidates(raw, None);
        assert_eq!(candidates.len(), 1 + GITHUB_RAW_MIRRORS.len());
        assert!(!candidates[0].mirror);
        assert_eq!(candidates[0].url, raw);
        assert_eq!(
            candidates[1].url,
            format!("https://{}/{raw}", GITHUB_RAW_MIRRORS[0])
        );
        assert!(candidates[1].mirror);

        // 上次赢家排第一，但其余候选一个不少。
        let winner = GITHUB_RAW_MIRRORS[1];
        let reordered = fetch_candidates(raw, Some(winner));
        assert_eq!(reordered[0].host, winner);
        assert_eq!(reordered.len(), candidates.len());

        let other = fetch_candidates("https://example.com/sub.yaml", None);
        assert_eq!(other.len(), 1);
        assert!(!other[0].mirror);
    }

    #[tokio::test]
    async fn race_keeps_the_first_candidate_that_answers() {
        let slow = Router::new().fallback(get(|| async {
            tokio::time::sleep(Duration::from_secs(600)).await;
            ""
        }));
        let slow_listener = bind_test_listener().await;
        let slow_address = slow_listener.local_addr().unwrap();
        let slow_server =
            tokio::spawn(async move { axum::serve(slow_listener, slow).await.unwrap() });
        let fast = Router::new().fallback(get(|| async {
            "ss://YWVzLTEyOC1nY206cGFzcw==@1.2.3.4:443#node"
        }));
        let fast_listener = bind_test_listener().await;
        let fast_address = fast_listener.local_addr().unwrap();
        let fast_server =
            tokio::spawn(async move { axum::serve(fast_listener, fast).await.unwrap() });

        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "race".into(),
                url_template: format!("http://{slow_address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();
        let task_id = collector
            .tasks
            .start(TaskKind::Fetch, [(source.id.clone(), source.name.clone())]);
        let candidates = vec![
            FetchCandidate {
                url: format!("http://{slow_address}/subscription"),
                host: "direct".into(),
                mirror: false,
            },
            FetchCandidate {
                url: format!("http://{fast_address}/subscription"),
                host: "mirror.test".into(),
                mirror: true,
            },
        ];

        let started = Instant::now();
        let downloaded = collector
            .race_candidates(
                &source,
                &candidates,
                &task_id,
                Duration::from_secs(6),
                RESPONSE_HEADER_TIMEOUT,
            )
            .await
            .unwrap()
            .expect("候选应返回响应体");

        assert!(!downloaded.body.is_empty());
        assert_eq!(downloaded.via.as_deref(), Some("mirror.test"));
        // 竞速的意义就在于不等慢的那条腿跑完预算。
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(
            db.get_source(&source.id)
                .await
                .unwrap()
                .unwrap()
                .last_fetch_host
                .as_deref(),
            Some("mirror.test")
        );
        slow_server.abort();
        fast_server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn extract_gists_reads_snippet_blocks() {
        let html = concat!(
            r#"<div class="gist-snippet">"#,
            r#"<a href="/u_existing/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa">2 files</a>"#,
            r#"</div>"#,
            r#"<div class="gist-snippet">"#,
            r#"<a href="/u_good/bbbbbbbbbbbbbbbbbbbbbbbbbbbb">clash.yaml</a>"#,
            r#"</div>"#,
            r#"<div class="gist-snippet">"#,
            r#"<a href="/u_empty/cccccccccccccccccccccccccccc">list.txt</a>"#,
            r#"</div>"#,
            // 子路径链接（forks/stars）不能当成 gist。
            r#"<div class="gist-snippet">"#,
            r#"<a href="/u_multi/dddddddddddddddddddddddddddd">1 files</a>"#,
            r#"<a href="/u_multi/dddddddddddddddddddddddddddd/stargazers">3 stars</a>"#,
            r#"<a href="/u_multi/dddddddddddddddddddddddddddd">nodes.list</a>"#,
            r#"</div>"#,
        );
        let gists = extract_gists(html);
        assert_eq!(gists.len(), 4);
        assert_eq!(gists[0].user, "u_existing");
        assert_eq!(gists[0].name, "gist-u_existing-aaaaaaaa");
        assert_eq!(gists[1].name, "clash.yaml");
        assert_eq!(gists[2].name, "list.txt");
        assert!(gists[2].id.starts_with('c') && gists[2].id.len() >= 16);
        // 同一块里的 /stargazers 子路径不会变成另一个 gist；文件名取块内的 .list。
        assert!(gists[3].id.starts_with('d') && gists[3].id.len() >= 16);
        assert_eq!(gists[3].name, "nodes.list");
    }

    #[tokio::test]
    async fn gist_discovery_keeps_only_sources_that_parse_nodes() {
        let (id_a, id_b, id_c, id_d) = (
            "a".repeat(32),
            "b".repeat(32),
            "c".repeat(32),
            "d".repeat(32),
        );
        let page1 = format!(
            concat!(
                r#"<div class="gist-snippet"><a href="/u_existing/{id_a}">2 files</a></div>"#,
                r#"<div class="gist-snippet"><a href="/u_good/{id_b}">good.yaml</a></div>"#,
                r#"<div class="gist-snippet"><a href="/u_empty/{id_c}">empty.txt</a></div>"#,
            ),
            id_a = id_a,
            id_b = id_b,
            id_c = id_c
        );
        let page2 = format!(
            r#"<div class="gist-snippet"><a href="/u_multi/{id_d}">nodes.list</a></div>"#,
            id_d = id_d
        );
        let app = Router::new().fallback(move |uri: axum::http::Uri| {
            let (page1, page2) = (page1.clone(), page2.clone());
            async move {
                let query = uri.query().unwrap_or("");
                if uri.path().contains("/u_empty/") {
                    "not a subscription, just text".to_string()
                } else if !uri.path().contains("/raw") {
                    if query.contains("page=3") {
                        String::new()
                    } else if query.contains("page=2") {
                        page2
                    } else {
                        page1
                    }
                } else {
                    "ss://YWVzLTEyOC1nY206cGFzcw==@1.2.3.4:443#node".to_string()
                }
            }
        });
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        // 已存在的源：同 id 的 gist 要被跳过。
        db.create_source(CreateSource {
            name: "已有".into(),
            url_template: format!("https://gist.githubusercontent.com/u_existing/{id_a}/raw"),
            enabled: true,
            remark: None,
            user_agent: None,
        })
        .await
        .unwrap();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();

        let summary = collector
            .discover_gists_from(
                &format!("http://{address}/search?q=vless%2F%2F"),
                &format!("http://{address}"),
                100,
            )
            .await
            .unwrap();

        assert_eq!(summary.pages_scanned, 3, "{}", summary.stopped_reason);
        assert_eq!(summary.registered, 2);
        assert_eq!(summary.nodes, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.rejected, 1);
        assert!(summary.stopped_reason.contains("第 3 页没有 gist"));
        let names: Vec<String> = db
            .list_sources()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"good.yaml".to_string()));
        assert!(names.contains(&"nodes.list".to_string()));
        assert!(!names.contains(&"empty.txt".to_string()));
        assert_eq!(db.seen_gist_ids().await.unwrap().len(), 3);

        // 第二轮：整页都已见过，第 1 页就停。
        let summary = collector
            .discover_gists_from(
                &format!("http://{address}/search?q=vless%2F%2F"),
                &format!("http://{address}"),
                100,
            )
            .await
            .unwrap();
        assert_eq!(summary.pages_scanned, 1);
        assert_eq!(summary.registered, 0);
        assert_eq!(summary.skipped, 3);
        assert!(summary.stopped_reason.contains("全部已扫描过"));
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    /// 真网络手测：`cargo test real_network -- --ignored --nocapture`
    /// 代理地址读 `VPN_PARSER_PROBE_PROXY`，默认打到本程序自己的 mixed 端口
    /// （以前硬编码 10808，那个代理一停这条用例就"失败"给你看，其实网络是通的）。
    #[tokio::test]
    #[ignore = "真实网络手测，需要一个能出境的本地代理在线"]
    async fn real_network_gist_discovery_via_local_proxy() {
        let db_path = std::env::temp_dir().join(format!("vpn-parser-real-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let tracker = TaskTracker::default();
        let proxy = std::env::var("VPN_PARSER_PROBE_PROXY")
            .unwrap_or_else(|_| "http://127.0.0.1:9980".to_string());
        let collector = Collector::new(db.clone(), tracker.clone())
            .unwrap()
            .with_proxy(&proxy)
            .unwrap();

        let summary = collector
            .discover_gists(GIST_DISCOVERY_DEFAULT_QUERY, 5)
            .await
            .unwrap();

        println!(
            "扫描 {} 页，新增 {} 源（{} 节点），跳过 {}，丢弃 {}，待重试 {}；{}",
            summary.pages_scanned,
            summary.registered,
            summary.nodes,
            summary.skipped,
            summary.rejected,
            summary.pending,
            summary.stopped_reason
        );
        for created in db.list_sources().await.unwrap() {
            println!("  源: {} -> {}", created.name, created.url_template);
        }
        assert!(summary.registered > 0);
        assert!(summary.nodes > 0);
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn fetch_all_enabled_runs_sources_concurrently_and_keeps_order() {
        // Peak in-flight requests is the real signal; a wall-clock bound on four 150 ms
        // handlers only says the machine was not busy.
        let inflight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let app = Router::new().fallback(get({
            let (inflight, peak) = (inflight.clone(), peak.clone());
            move || {
                let (inflight, peak) = (inflight.clone(), peak.clone());
                async move {
                    let now = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    ""
                }
            }
        }));
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        for index in 0..4 {
            db.create_source(CreateSource {
                name: format!("source-{index}"),
                url_template: format!("http://{address}/{index}"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        }
        let expected_ids: Vec<_> = db
            .list_sources()
            .await
            .unwrap()
            .into_iter()
            .map(|source| source.id)
            .collect();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();

        let started = Instant::now();
        let results = collector.fetch_all_enabled().await.unwrap();

        assert!(
            peak.load(Ordering::SeqCst) >= 2,
            "订阅拉取没有并发: {}",
            peak.load(Ordering::SeqCst)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(
            results
                .iter()
                .map(|result| result.source_id.clone())
                .collect::<Vec<_>>(),
            expected_ids
        );
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn conditional_fetch_skips_parse_and_write_on_304() {
        let requests = Arc::new(AtomicUsize::new(0));
        let handler_requests = requests.clone();
        let app = Router::new().route(
            "/subscription",
            get(move |headers: HeaderMap| {
                let requests = handler_requests.clone();
                async move {
                    requests.fetch_add(1, Ordering::SeqCst);
                    if headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some("v1") {
                        return Response::builder()
                            .status(StatusCode::NOT_MODIFIED)
                            .body(Body::empty())
                            .unwrap();
                    }
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(ETAG, "v1")
                        .body(Body::from("ss://YWVzLTEyOC1nY206cGFzcw==@1.2.3.4:443#node"))
                        .unwrap()
                }
            }),
        );
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "conditional".into(),
                url_template: format!("http://{address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();

        let first = collector.fetch_source(&source).await;
        assert!(first.success, "{}", first.message);
        let cached = db.get_source(&source.id).await.unwrap().unwrap();
        assert_eq!(cached.last_etag.as_deref(), Some("v1"));
        let second = collector.fetch_source(&cached).await;

        assert!(second.success);
        assert!(second.message.contains("skipped parse/write"));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn unchanged_body_hash_skips_parse_and_write() {
        let app = Router::new().route(
            "/subscription",
            get(|| async { "ss://YWVzLTEyOC1nY206cGFzcw==@1.2.3.4:443#node" }),
        );
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db_path = std::env::temp_dir().join(format!("vpn-parser-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let source = db
            .create_source(CreateSource {
                name: "hash".into(),
                url_template: format!("http://{address}/subscription"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let collector = Collector::new(db.clone(), TaskTracker::default()).unwrap();

        assert!(collector.fetch_source(&source).await.success);
        let cached = db.get_source(&source.id).await.unwrap().unwrap();
        let second = collector.fetch_source(&cached).await;

        assert!(second.success);
        assert!(second.message.contains("content unchanged"));
        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn collector_sends_custom_or_default_user_agent() {
        use std::sync::Mutex;
        let received_uas = Arc::new(Mutex::new(Vec::<String>::new()));
        let uas_handle = received_uas.clone();

        let app = Router::new().route(
            "/sub",
            get(move |headers: axum::http::HeaderMap| {
                let uas = uas_handle.clone();
                async move {
                    let ua = headers
                        .get(axum::http::header::USER_AGENT)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    uas.lock().unwrap().push(ua);
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(Body::from("ss://YWVzLTEyOC1nY206cGFzcw==@1.2.3.4:443#node"))
                        .unwrap()
                }
            }),
        );
        let listener = bind_test_listener().await;
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let db_path = std::env::temp_dir().join(format!("vpn-parser-ua-{}.db", Uuid::new_v4()));
        let db = Database::connect(db_path.to_str().unwrap()).await.unwrap();
        let tracker = TaskTracker::default();
        let collector = Collector::new(db.clone(), tracker).unwrap();

        // 1. Without UA configured -> should default to "clash.meta"
        let default_src = db
            .create_source(CreateSource {
                name: "default_ua".into(),
                url_template: format!("http://{address}/sub"),
                enabled: true,
                remark: None,
                user_agent: None,
            })
            .await
            .unwrap();
        let res1 = collector.fetch_source(&default_src).await;
        assert!(res1.success);

        // 2. With custom UA configured -> should send custom UA
        let custom_src = db
            .create_source(CreateSource {
                name: "custom_ua".into(),
                url_template: format!("http://{address}/sub"),
                enabled: true,
                remark: None,
                user_agent: Some("clash-verge/v1.7.7".into()),
            })
            .await
            .unwrap();
        let res2 = collector.fetch_source(&custom_src).await;
        assert!(res2.success);

        let captured = received_uas.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0], "clash.meta");
        assert_eq!(captured[1], "clash-verge/v1.7.7");

        server.abort();
        drop(collector);
        drop(db);
        let _ = std::fs::remove_file(db_path);
    }
}

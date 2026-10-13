use crate::collector::Collector;
use crate::db::Database;
use crate::export::{nodes_to_clash_proxies, nodes_to_share_uris};
use crate::export_quality::export_score;
use crate::kernel::{KernelKind, KernelManager, KernelPreference};
use crate::models::{
    ApiResponse, CreateSource, NodeQuery, UpdateSettings, UpdateSource, GOOGLE_PROXY_TEST_KIND,
};
use crate::proxy::{is_node_connectivity_failure, MihomoConfigOptions, ProxyManager};
use crate::speedtest::{SpeedTestOptions, SpeedTester, SpeedtestIntensity};
use crate::tasks::TaskTracker;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex as TokioMutex;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub collector: Collector,
    pub speedtester: SpeedTester,
    pub tasks: TaskTracker,
    pub kernel_manager: KernelManager,
    pub proxy_manager: ProxyManager,
}

/// 本地 HTTP API。默认只监听 127.0.0.1，但同机任意进程、以及浏览器里打开的任意
/// 网页都摸得到它，所以所有路由一律要求 Bearer Token；只有只读 GET（导出订阅链接
/// 需要在浏览器里直接打开）额外允许 `?token=`。
/// 故意不挂 CorsLayer：没有跨域头，第三方网页读不到任何响应体。
pub fn router(state: AppState, api_token: Option<String>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/tasks", get(list_tasks))
        .route("/api/stats", get(stats))
        .route("/api/settings", get(get_settings).put(update_settings))
        .route("/api/sources", get(list_sources).post(create_source))
        .route(
            "/api/sources/{id}",
            get(get_source).put(update_source).delete(delete_source),
        )
        .route("/api/sources/{id}/fetch", post(fetch_source))
        .route("/api/sources/fetch-all", post(fetch_all))
        .route("/api/gist/discover", post(discover_gists))
        .route("/api/nodes", get(list_nodes))
        .route("/api/nodes/{id}", get(get_node).delete(delete_node))
        .route("/api/nodes/clear", post(clear_nodes))
        .route("/api/nodes/delete-dead", post(delete_dead_nodes))
        .route("/api/nodes/delete-invalid", post(delete_invalid_hosts))
        .route("/api/nodes/dedupe", post(dedupe_nodes))
        .route("/api/nodes/{id}/speedtest", post(speedtest_one))
        .route("/api/nodes/speedtest", post(speedtest_all))
        .route("/api/kernels", get(kernels))
        .route("/api/kernels/releases", get(kernel_releases))
        .route("/api/kernels/update", post(update_kernel))
        .route("/api/proxy/status", get(proxy_status))
        .route(
            "/api/proxy/config",
            get(proxy_config).put(update_proxy_config),
        )
        .route("/api/proxy/log", get(proxy_log))
        .route("/api/proxy/start", post(proxy_start))
        .route("/api/proxy/stop", post(proxy_stop))
        .route("/api/proxy/restart", post(proxy_restart))
        .route("/api/export/json", get(export_json))
        .route("/api/export/clash", get(export_clash))
        .route("/api/export/uri", get(export_uri))
        .layer(middleware::from_fn(move |request, next: Next| {
            let expected = api_token.clone();
            async move { authenticate_request(request, next, expected.as_deref()).await }
        }))
        .layer(TraceLayer::new_for_http())
        .with_state(Arc::new(state))
}

pub async fn serve_with_shutdown(
    state: AppState,
    listen: &str,
    api_token: Option<String>,
    shutdown_rx: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let addr: std::net::SocketAddr = listen.parse()?;
    tracing::info!(%addr, "HTTP API listening");
    let app = router(state, api_token);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await?;
    tracing::info!(%addr, "HTTP API server stopped");
    Ok(())
}

struct ApiServerState {
    shutdown_tx: Option<oneshot::Sender<()>>,
}

#[derive(Clone)]
pub struct ApiServerController {
    inner: Arc<TokioMutex<ApiServerState>>,
    state: AppState,
    listen: String,
    api_token: Option<String>,
    running: Arc<AtomicBool>,
}

impl ApiServerController {
    pub fn new(state: AppState, listen: String, api_token: Option<String>) -> Self {
        Self {
            inner: Arc::new(TokioMutex::new(ApiServerState { shutdown_tx: None })),
            state,
            listen,
            api_token,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn listen_addr(&self) -> &str {
        &self.listen
    }

    /// 当前生效的访问令牌；`None` 表示未启用鉴权。
    pub fn token(&self) -> Option<&str> {
        self.api_token.as_deref()
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let mut guard = self.inner.lock().await;
        if self.running.load(Ordering::Relaxed) {
            return Ok(());
        }

        let (tx, rx) = oneshot::channel();
        guard.shutdown_tx = Some(tx);
        self.running.store(true, Ordering::SeqCst);

        let state = self.state.clone();
        let listen = self.listen.clone();
        let api_token = self.api_token.clone();
        let running_flag = self.running.clone();

        tokio::spawn(async move {
            if let Err(e) = serve_with_shutdown(state, &listen, api_token, rx).await {
                tracing::error!(error = %e, "HTTP API server error");
            }
            running_flag.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        let mut guard = self.inner.lock().await;
        if let Some(tx) = guard.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

async fn authenticate_request(
    request: Request<Body>,
    next: Next,
    expected: Option<&str>,
) -> Response {
    let Some(expected) = expected else {
        return next.run(request).await;
    };
    let from_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_owned);
    let from_query = (request.method() == Method::GET)
        .then(|| {
            request.uri().query().map(|query| {
                query
                    .split('&')
                    .find_map(|pair| {
                        let (key, value) = pair.split_once('=')?;
                        (key == "token").then(|| value.to_owned())
                    })
                    .unwrap_or_default()
            })
        })
        .flatten()
        .filter(|value| !value.is_empty());
    let authorized = [from_header, from_query]
        .into_iter()
        .flatten()
        .any(|provided| provided == expected);
    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            Json(ApiResponse::<()>::err(
                "认证失败，请提供有效的 Bearer Token",
            )),
        )
            .into_response()
    }
}

async fn health() -> Json<ApiResponse<serde_json::Value>> {
    Json(ApiResponse::ok(json!({"status": "up"})))
}

#[derive(Debug, Deserialize)]
struct TaskQuery {
    limit: Option<usize>,
}

async fn list_tasks(
    State(st): State<Arc<AppState>>,
    Query(query): Query<TaskQuery>,
) -> Json<ApiResponse<Vec<crate::tasks::TaskSnapshot>>> {
    Json(ApiResponse::ok(
        st.tasks.snapshots(query.limit.unwrap_or(10)),
    ))
}

async fn kernels(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    Json(ApiResponse::ok(st.kernel_manager.detect_all().await))
}

async fn kernel_releases(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    let proxy_status = st.proxy_manager.status().await;
    let proxy_url = proxy_status
        .local_proxy_active()
        .then(|| format!("http://127.0.0.1:{}", proxy_status.mixed_port));
    match st.kernel_manager.check_updates_with_proxy(proxy_url).await {
        Ok(releases) => Json(ApiResponse::ok(releases)).into_response(),
        Err(error) => error_resp(error),
    }
}

#[derive(Debug, Deserialize)]
struct KernelUpdateBody {
    kind: KernelKind,
}

async fn update_kernel(
    State(st): State<Arc<AppState>>,
    Json(body): Json<KernelUpdateBody>,
) -> impl IntoResponse {
    let proxy_status = st.proxy_manager.status().await;
    if proxy_status.kernel_in_use(body.kind) {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::<()>::err(format!(
                "停止代理后才能更新 {}",
                body.kind.display_name()
            ))),
        )
            .into_response();
    }
    let proxy_url = proxy_status
        .local_proxy_active()
        .then(|| format!("http://127.0.0.1:{}", proxy_status.mixed_port));
    // 走哪种下载方式跟 GUI 弹框用同一份设置；读不到就用默认那家镜像。
    let source = crate::kernel::source_from_id(
        &st.db
            .get_settings()
            .await
            .map(|settings| settings.kernel_source)
            .unwrap_or_default(),
    );
    match st
        .kernel_manager
        .update_with_options(
            body.kind,
            proxy_url,
            source,
            None,
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
    {
        Ok(result) => Json(ApiResponse::ok(result)).into_response(),
        Err(error) => error_resp(error),
    }
}

async fn proxy_status(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    Json(ApiResponse::ok(st.proxy_manager.status().await))
}

async fn proxy_config(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    Json(ApiResponse::ok(st.proxy_manager.config().await))
}

async fn update_proxy_config(
    State(st): State<Arc<AppState>>,
    Json(body): Json<MihomoConfigOptions>,
) -> impl IntoResponse {
    let previous = st.proxy_manager.config().await;
    if let Err(error) = st.proxy_manager.update_config(body.clone()).await {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::err(error.to_string())),
        )
            .into_response();
    }
    match st.db.update_proxy_config(&body).await {
        Ok(()) => Json(ApiResponse::ok(body)).into_response(),
        Err(error) => {
            let _ = st.proxy_manager.update_config(previous).await;
            error_resp(error)
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProxyLogQuery {
    max_chars: Option<usize>,
}

async fn proxy_log(
    State(st): State<Arc<AppState>>,
    Query(query): Query<ProxyLogQuery>,
) -> impl IntoResponse {
    match st
        .proxy_manager
        .recent_log(query.max_chars.unwrap_or(8_000).clamp(100, 100_000))
        .await
    {
        Ok(log) => Json(ApiResponse::ok(json!({"log": log}))).into_response(),
        Err(error) => error_resp(error),
    }
}

#[derive(Debug, Deserialize)]
struct ProxyNodeBody {
    node_id: String,
    #[serde(default)]
    kernel: Option<KernelPreference>,
}

async fn proxy_operation_error(st: &AppState, node_id: &str, error: anyhow::Error) -> Response {
    let message = error.to_string();
    if !is_node_connectivity_failure(&message) {
        return error_resp(error);
    }
    let kernel = st
        .proxy_manager
        .status()
        .await
        .kernel
        .map(|kernel| match kernel {
            KernelKind::Mihomo => "mihomo",
            KernelKind::SingBox => "sing_box",
            KernelKind::Xray => "xray",
        });
    let suffix = match st
        .db
        .update_node_speed(
            node_id,
            false,
            None,
            None,
            kernel,
            Some(GOOGLE_PROXY_TEST_KIND),
        )
        .await
    {
        Ok(()) => "; 已将该节点标记为不可用，请切换节点".to_owned(),
        Err(update_error) => format!("; 标记节点不可用失败: {update_error}"),
    };
    error_resp(anyhow::anyhow!("{message}{suffix}"))
}

async fn proxy_start(
    State(st): State<Arc<AppState>>,
    Json(body): Json<ProxyNodeBody>,
) -> impl IntoResponse {
    let node = match st.db.get_node(&body.node_id).await {
        Ok(Some(node)) => node,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::err("node not found")),
            )
                .into_response()
        }
        Err(error) => return error_resp(error),
    };
    match st.proxy_manager.connect(&node, body.kernel).await {
        Ok(()) => Json(ApiResponse::ok(st.proxy_manager.status().await)).into_response(),
        Err(error) => proxy_operation_error(&st, &node.id, error).await,
    }
}

async fn proxy_stop(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.proxy_manager.stop().await {
        Ok(()) => Json(ApiResponse::ok(st.proxy_manager.status().await)).into_response(),
        Err(error) => error_resp(error),
    }
}

async fn proxy_restart(
    State(st): State<Arc<AppState>>,
    Json(body): Json<ProxyNodeBody>,
) -> impl IntoResponse {
    let node = match st.db.get_node(&body.node_id).await {
        Ok(Some(node)) => node,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::<()>::err("node not found")),
            )
                .into_response()
        }
        Err(error) => return error_resp(error),
    };
    match st.proxy_manager.connect(&node, body.kernel).await {
        Ok(()) => Json(ApiResponse::ok(st.proxy_manager.status().await)).into_response(),
        Err(error) => proxy_operation_error(&st, &node.id, error).await,
    }
}

async fn stats(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.stats().await {
        Ok(s) => Json(ApiResponse::ok(s)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn get_settings(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.get_settings().await {
        Ok(s) => Json(ApiResponse::ok(s)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn update_settings(
    State(st): State<Arc<AppState>>,
    Json(body): Json<UpdateSettings>,
) -> impl IntoResponse {
    match st.db.update_settings(body).await {
        Ok(s) => Json(ApiResponse::ok(s)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn list_sources(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.list_sources().await {
        Ok(list) => Json(ApiResponse::ok(list)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn get_source(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> impl IntoResponse {
    match st.db.get_source(&id).await {
        Ok(Some(s)) => Json(ApiResponse::ok(s)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("not found")),
        )
            .into_response(),
        Err(e) => error_resp(e),
    }
}

async fn create_source(
    State(st): State<Arc<AppState>>,
    Json(body): Json<CreateSource>,
) -> impl IntoResponse {
    if body.name.trim().is_empty() || body.url_template.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<()>::err("name and url_template required")),
        )
            .into_response();
    }
    match st.db.create_source(body).await {
        Ok(s) => (StatusCode::CREATED, Json(ApiResponse::ok(s))).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn update_source(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSource>,
) -> impl IntoResponse {
    match st.db.update_source(&id, body).await {
        Ok(s) => Json(ApiResponse::ok(s)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn delete_source(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match st.db.delete_source(&id).await {
        Ok(true) => Json(ApiResponse::ok(json!({"deleted": true}))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("not found")),
        )
            .into_response(),
        Err(e) => error_resp(e),
    }
}

async fn fetch_source(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match st.collector.fetch_one(&id).await {
        Ok(r) => Json(ApiResponse::ok(r)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn fetch_all(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.collector.fetch_all_enabled().await {
        Ok(r) => Json(ApiResponse::ok(r)).into_response(),
        Err(e) => error_resp(e),
    }
}

#[derive(Debug, Deserialize)]
struct GistDiscoveryRequest {
    /// 搜索关键词，默认 `vless//`
    query: Option<String>,
    /// 最多扫描页数（每页 10 条），默认 100
    max_pages: Option<usize>,
}

/// 后台启动一次 Gist 发现；完成后的源用 GET /api/sources 查看。
async fn discover_gists(
    State(st): State<Arc<AppState>>,
    Json(body): Json<GistDiscoveryRequest>,
) -> impl IntoResponse {
    let collector = st.collector.clone();
    let query = body.query.unwrap_or_else(|| "vless//".to_string());
    let max_pages = body.max_pages.unwrap_or(100);
    let query_for_reply = query.clone();
    tokio::spawn(async move {
        if let Err(error) = collector.discover_gists(&query, max_pages).await {
            tracing::error!(error = %error, "gist discovery failed");
        }
    });
    Json(ApiResponse::ok(json!({
        "started": true,
        "query": query_for_reply,
        "max_pages": max_pages,
        "message": "Gist 发现已在后台启动；完成后查看 /api/sources 与 /api/nodes"
    })))
    .into_response()
}

async fn list_nodes(
    State(st): State<Arc<AppState>>,
    Query(q): Query<NodeQuery>,
) -> impl IntoResponse {
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(50);
    match st
        .db
        .list_nodes(
            q.protocol.as_deref(),
            q.source_id.as_deref(),
            q.keyword.as_deref(),
            q.latency.as_deref(),
            q.country.as_deref(),
            page,
            page_size,
            q.sort.as_deref(),
        )
        .await
    {
        Ok(p) => Json(ApiResponse::ok(p)).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn get_node(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> impl IntoResponse {
    match st.db.get_node(&id).await {
        Ok(Some(n)) => Json(ApiResponse::ok(n)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("not found")),
        )
            .into_response(),
        Err(e) => error_resp(e),
    }
}

async fn delete_node(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> impl IntoResponse {
    match st.db.delete_node(&id).await {
        Ok(true) => Json(ApiResponse::ok(json!({"deleted": true}))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::err("not found")),
        )
            .into_response(),
        Err(e) => error_resp(e),
    }
}

async fn clear_nodes(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.clear_nodes().await {
        Ok(n) => Json(ApiResponse::ok(json!({"deleted": n}))).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn delete_dead_nodes(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.delete_dead_nodes().await {
        Ok(n) => Json(ApiResponse::ok(json!({"deleted": n}))).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn delete_invalid_hosts(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.delete_invalid_hosts().await {
        Ok(n) => Json(ApiResponse::ok(json!({"deleted": n}))).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn dedupe_nodes(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match st.db.dedupe_nodes().await {
        Ok(n) => Json(ApiResponse::ok(json!({"deleted": n}))).into_response(),
        Err(e) => error_resp(e),
    }
}

async fn speedtest_one(
    State(st): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let opts = match st.db.get_settings().await {
        Ok(s) => SpeedTestOptions {
            concurrency: 1,
            timeout_ms: s.speedtest_timeout_ms,
            measure_download: s.speedtest_download,
            download_test_ms: s.download_test_ms,
            intensity: SpeedtestIntensity::Standard,
            parallel_kernels: s.speedtest_parallel_kernels,
            persistent_kernel: s.speedtest_persistent_kernel,
            reuse_verified_minutes: s.speedtest_reuse_minutes,
            conditional_isolation: s.speedtest_conditional_isolation,
            download_top_n: s.speedtest_download_top_n,
            dedup_egress: s.speedtest_dedup_egress,
            tcp_ping_only: s.speedtest_tcp_ping_only,
        },
        Err(_) => SpeedTestOptions::default(),
    };
    match st.speedtester.test_one(&id, opts).await {
        Ok(r) => Json(ApiResponse::ok(r)).into_response(),
        Err(e) => error_resp(e),
    }
}

#[derive(Debug, Deserialize)]
struct SpeedtestBody {
    /// Optional subset of node ids. Empty/missing = all nodes.
    ids: Option<Vec<String>>,
}

async fn speedtest_all(
    State(st): State<Arc<AppState>>,
    body: Option<Json<SpeedtestBody>>,
) -> impl IntoResponse {
    let settings = match st.db.get_settings().await {
        Ok(s) => s,
        Err(e) => return error_resp(e),
    };
    let ids = body.and_then(|b| b.0.ids);
    let opts = SpeedTestOptions {
        concurrency: settings.speedtest_concurrency,
        timeout_ms: settings.speedtest_timeout_ms,
        measure_download: settings.speedtest_download,
        download_test_ms: settings.download_test_ms,
        intensity: SpeedtestIntensity::Standard,
        parallel_kernels: settings.speedtest_parallel_kernels,
        persistent_kernel: settings.speedtest_persistent_kernel,
        reuse_verified_minutes: settings.speedtest_reuse_minutes,
        conditional_isolation: settings.speedtest_conditional_isolation,
        download_top_n: settings.speedtest_download_top_n,
        dedup_egress: settings.speedtest_dedup_egress,
        tcp_ping_only: settings.speedtest_tcp_ping_only,
    };
    match st.speedtester.test_many(ids, opts).await {
        Ok(r) => Json(ApiResponse::ok(r)).into_response(),
        Err(e) => error_resp(e),
    }
}

#[derive(Debug, Deserialize)]
struct ExportQuery {
    protocol: Option<String>,
    source_id: Option<String>,
    /// Default true: TCP-alive public hosts under max_latency_ms, excluding decoys.
    quality_only: Option<bool>,
    /// Max TCP latency ms (default 500 for stricter client import).
    max_latency_ms: Option<i64>,
    /// Min download KB/s. Default 0 = do NOT require download (recommended).
    /// Only set >0 if you explicitly want open-port/HTTP-like endpoints.
    min_download_kbps: Option<f64>,
    /// Default true (v2rayN subscription). Set false for plain multi-line URIs.
    base64: Option<bool>,
    /// Max nodes to export (sorted by stored quality results). Default: 50.
    limit: Option<u32>,
}

fn quality_params(q: &ExportQuery) -> (bool, Option<i64>, Option<f64>, Option<u32>) {
    (
        q.quality_only.unwrap_or(true),
        Some(q.max_latency_ms.unwrap_or(500)),
        // Important: default 0. Requiring download selected fake websites before.
        Some(q.min_download_kbps.unwrap_or(0.0)),
        q.limit,
    )
}

async fn load_export_nodes(
    st: &AppState,
    q: &ExportQuery,
) -> Result<Vec<crate::models::Node>, anyhow::Error> {
    let (quality_only, max_lat, min_dl, limit) = quality_params(q);
    let fetch_limit = if quality_only {
        Some(limit.unwrap_or(50).clamp(1, 500))
    } else {
        limit
    };
    let mut nodes = st
        .db
        .export_nodes_filtered(
            q.protocol.as_deref(),
            q.source_id.as_deref(),
            quality_only,
            max_lat,
            min_dl,
            fetch_limit,
        )
        .await?;

    if quality_only {
        nodes.sort_by(|a, b| export_score(b).cmp(&export_score(a)));
        let want = limit.unwrap_or(50).clamp(1, 500) as usize;
        if nodes.len() > want {
            nodes.truncate(want);
        }
    } else if let Some(n) = limit {
        nodes.truncate(n.max(1) as usize);
    }
    Ok(nodes)
}

async fn export_json(
    State(st): State<Arc<AppState>>,
    Query(q): Query<ExportQuery>,
) -> impl IntoResponse {
    let (quality_only, max_lat, min_dl, limit) = quality_params(&q);
    match load_export_nodes(st.as_ref(), &q).await {
        Ok(nodes) => {
            let count = nodes.len();
            let configs: Vec<_> = nodes.into_iter().map(|n| n.config).collect();
            Json(ApiResponse::ok(json!({
                "count": count,
                "quality_only": quality_only,
                "max_latency_ms": max_lat.unwrap_or(500),
                "min_download_kbps": min_dl.unwrap_or(0.0),
                "limit": limit,
                "note": "Export uses stored test results only and does not run network tests. limit is the maximum number of nodes returned.",
                "nodes": configs,
            })))
            .into_response()
        }
        Err(e) => error_resp(e),
    }
}

async fn export_clash(
    State(st): State<Arc<AppState>>,
    Query(q): Query<ExportQuery>,
) -> impl IntoResponse {
    let (_quality_only, _max_lat, _min_dl, _limit) = quality_params(&q);
    match load_export_nodes(st.as_ref(), &q).await {
        Ok(nodes) => {
            let proxies = nodes_to_clash_proxies(&nodes);
            let names: Vec<_> = proxies
                .iter()
                .filter_map(|p| {
                    p.get("name")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            // Minimal full Clash profile so clients accept import (not proxies-only fragment).
            let doc = json!({
                "port": 9980,
                "socks-port": 7891,
                "allow-lan": false,
                "mode": "rule",
                "log-level": "info",
                "external-controller": "127.0.0.1:9090",
                "proxies": proxies,
                "proxy-groups": [{
                    "name": "PROXY",
                    "type": "select",
                    "proxies": names
                }],
                "rules": ["MATCH,PROXY"],
            });
            match serde_yaml::to_string(&doc) {
                Ok(yaml) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/yaml; charset=utf-8")],
                    yaml,
                )
                    .into_response(),
                Err(e) => error_resp(e),
            }
        }
        Err(e) => error_resp(e),
    }
}

async fn export_uri(
    State(st): State<Arc<AppState>>,
    Query(q): Query<ExportQuery>,
) -> impl IntoResponse {
    match load_export_nodes(st.as_ref(), &q).await {
        Ok(nodes) => {
            // v2rayN / most subscription clients expect base64 body by default.
            // Pass base64=false for plain multi-line share links.
            let plain = nodes_to_share_uris(&nodes).join("\n");
            let body = if q.base64.unwrap_or(true) {
                use base64::{engine::general_purpose::STANDARD, Engine as _};
                STANDARD.encode(plain.as_bytes())
            } else {
                plain
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                body,
            )
                .into_response()
        }
        Err(e) => error_resp(e),
    }
}

fn error_resp(e: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse::<()>::err(e.to_string())),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_request_keeps_old_body_compatible() {
        let body: ProxyNodeBody = serde_json::from_str(r#"{"node_id":"node-1"}"#).unwrap();

        assert_eq!(body.node_id, "node-1");
        assert_eq!(body.kernel, None);
    }

    #[test]
    fn proxy_request_accepts_explicit_kernel() {
        let body: ProxyNodeBody =
            serde_json::from_str(r#"{"node_id":"node-1","kernel":"xray"}"#).unwrap();

        assert_eq!(body.kernel, Some(KernelPreference::Xray));
    }
}

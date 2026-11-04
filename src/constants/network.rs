//! 网络、DNS、用户代理与探测服务常量

/// 默认网络请求 User-Agent
pub const DEFAULT_USER_AGENT: &str = "vproxy/1.0 (+https://localhost)";

/// 内核更新器专用 User-Agent
pub const DEFAULT_KERNEL_UPDATER_UA: &str = "vproxy/0.1 kernel-updater";

/// GeoIP/GeoSite 规则更新器专用 User-Agent
pub const DEFAULT_GEO_UPDATER_UA: &str = "vproxy geo-updater";

/// 默认主 DNS (DoH)
pub const DEFAULT_DNS_PRIMARY: &str = "https://doh.pub/dns-query";

/// 默认备用 DNS (DoH)
pub const DEFAULT_DNS_SECONDARY: &str = "https://dns.alidns.com/dns-query";

/// 历史默认 DoH 地址（用于兼容性迁移）
pub const LEGACY_DEFAULT_DOH: &str = "https://1.1.1.1/dns-query";

/// 前代默认 DoH 地址
pub const PREVIOUS_DEFAULT_DOH: &str = "https://223.5.5.5/dns-query";

/// 历史纯文本 DNS 默认列表
pub const LEGACY_DEFAULT_PLAINTEXT: &[&str] = &["8.8.8.8", "114.114.114.114"];

/// 代理连通性主探测目标 URL
pub const PROXY_CONNECTIVITY_TEST_URL: &str = "https://www.google.com/generate_204";

/// 代理连通性备用探测目标 URL
pub const PROXY_CONNECTIVITY_FALLBACK_URL: &str = "https://cp.cloudflare.com/generate_204";

/// Google 代理测试类型标识
pub const GOOGLE_PROXY_TEST_KIND: &str = "google_proxy";

/// GitHub Raw 官方域名
pub const GITHUB_RAW_HOST: &str = "raw.githubusercontent.com";

/// GitHub Gist 搜索主域名
pub const GIST_SEARCH_HOST: &str = "gist.github.com";

/// GitHub Gist Raw 基础前缀 URL
pub const GIST_RAW_BASE: &str = "https://gist.githubusercontent.com";

/// GitHub Raw 国内加速镜像列表
pub const GITHUB_RAW_MIRRORS: &[&str] = &["gh-proxy.com", "ghfast.top"];

/// Gist 自动发现专用 Source ID
pub const GIST_DISCOVERY_SOURCE_ID: &str = "__gist_discovery__";

/// Gist 自动发现默认检索关键词
pub const GIST_DISCOVERY_DEFAULT_QUERY: &str = "vless//";

/// 未分组订阅源的保留虚拟 Source ID
pub const UNGROUPED_SOURCE_ID: &str = "__ungrouped__";

/// 终端环境免代理变量名
pub const NO_PROXY_VAR: &str = "NO_PROXY";

/// 默认局域网免代理规则串
pub const NO_PROXY_VALUE: &str = "localhost;127.0.0.1;::1;<local>;192.168.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*";

/// Cloudflare WARP MASQUE 官方 SNI 域名
pub const OFFICIAL_WARP_SNI: &str = "zt-masque.cloudflareclient.com";

/// Cloudflare WARP MASQUE 节点预设 IP
pub const WARP_SNI_NODE_IP: &str = "162.159.198.1";

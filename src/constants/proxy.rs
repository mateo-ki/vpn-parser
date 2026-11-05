//! 代理内核、策略组与规则资源常量

/// 默认自动测速选择策略组名称
pub const URL_TEST_GROUP: &str = "自动选择";

/// 默认最终兜底策略组名称
pub const FINAL_GROUP: &str = "漏网之鱼";

/// Sing-box 远程 GeoSite 国内直连规则集下载 URL
pub const GEOSITE_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-cn.srs";

/// Sing-box 远程 GeoSite 广告拦截规则集下载 URL
pub const GEOSITE_ADS_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-category-ads-all.srs";

/// Sing-box 远程 GeoSite GFWList 代理规则集下载 URL
pub const GEOSITE_GFW_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geosite/rule-set/geosite-gfw.srs";

/// Sing-box 远程 GeoIP 国内 IP 规则集下载 URL
pub const GEOIP_CN_URL: &str =
    "https://raw.githubusercontent.com/SagerNet/sing-geoip/rule-set/geoip-cn.srs";

/// 默认内置锁定的 Mihomo 资产版本
pub const MIHOMO_TAG: &str = "v1.19.32";

/// 默认内置锁定的 Sing-box 资产版本
pub const SINGBOX_TAG: &str = "v1.13.14";

/// 默认内置锁定的 Xray 资产版本
pub const XRAY_TAG: &str = "v24.12.31";

/// 默认内置锁定的 Meow 资产版本
pub const MEOW_TAG: &str = "v0.26.1";

/// 默认内置锁定的 Shoes 资产版本
pub const SHOES_TAG: &str = "v0.3.4";

/// 默认内置锁定的 VP-Core 资产版本
pub const VPCORE_TAG: &str = "v1.6.2";

use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::kernel::KernelKind;
use crate::proxy::ir::RoutingPlan;
use anyhow::Result;

/// meow（https://github.com/meow-rs/meow-rs）——mihomo 的 Rust 高性能重写。
///
/// 配置面与 Clash API 都按 mihomo 兼容设计（docs/mihomo-api-compatibility.md），
/// 因此编译主体直接复用 [`MihomoCompiler`] 的产物，再做 meow 特化裁剪：
/// - 剥掉 mihomo 的 geo 字段（geodata-mode/geo-auto-update/geox-url）：
///   meow 的 GEOSITE/GEOIP 走自己 home 目录的 geosite.dat / Country.mmdb，
///   由 `crate::geo::ensure_meow_home_geo` 在启动前备好（实测 dat 可直接加载）；
/// - TUN：meow 自带 TUN（Windows Wintun + lwIP tun2socks），按它的键集
///   产出 tun 段（enable/device/auto-route/dns-hijack）并强制 fake-ip；
///   网卡 DNS 自动化在 platform 的提权 runner 里完成。
///
/// 实测（v0.26.1，`meow -f config -d home -t`）：
/// - dns 段（respect-rules / nameserver-policy geosite: / fake-ip）原样接受；
/// - listeners（含 proxy 字段）与 IN-PORT 规则接受；
/// - mihomo 遗留 geo 字段静默接受，不支持协议（tuic/wireguard）节点 WARN 跳过不拒启。
pub struct MeowCompiler;

impl Default for MeowCompiler {
    fn default() -> Self {
        Self
    }
}

impl BackendCompiler for MeowCompiler {
    fn kernel(&self) -> KernelKind {
        KernelKind::Meow
    }

    fn capabilities(&self) -> &CompilerCapabilities {
        const CAPS: CompilerCapabilities = CompilerCapabilities {
            supports_tun: true,
            supports_process_name: true,
            supports_domain_keyword: true,
            supports_proxy_groups: true,
            supports_url_test: true,
            supported_geosite_categories: None,
            supported_geoip_countries: None,
        };
        &CAPS
    }

    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact> {
        let artifact = super::MihomoCompiler::new().compile(plan)?;
        let mut config = artifact.as_json_value()?;
        if let Some(obj) = config.as_object_mut() {
            for key in [
                "geodata-mode",
                "geo-auto-update",
                "geo-update-interval",
                "geox-url",
            ] {
                obj.remove(key);
            }
            // TUN：meow 的 tun 键集与 mihomo 不同，只发它认的键（docs/tun.md）。
            // device 钉成 "meow"：platform 的提权 runner 按这个名字自动把网卡
            // DNS 指向 fake-ip 网段（198.18.0.2），退出时还原 —— 不做这步 TUN 不通。
            // fake-ip 是 meow TUN 的硬性要求（docs/tun.md），非 fake-ip 的 DNS 配置
            // 在 TUN 下强制覆盖为 fake-ip。
            if plan.inbounds.mode.has_tun() {
                obj.insert(
                    "tun".into(),
                    serde_json::json!({
                        "enable": true,
                        "device": "meow",
                        "auto-route": true,
                        "dns-hijack": ["any:53"],
                    }),
                );
                if let Some(dns) = obj.get_mut("dns").and_then(|d| d.as_object_mut()) {
                    dns.insert("enhanced-mode".into(), serde_json::json!("fake-ip"));
                    dns.insert("fake-ip-range".into(), serde_json::json!("198.18.0.1/16"));
                }
            }
            // meow 的 GEOIP 用 MaxMind mmdb、GEOSITE 用 mrs/dat：缺库时它会按需下载，
            // 默认经"最后一个 MATCH 目标"（= 节点）走 —— 节点劣化时无限卡死"正在启动"。
            // 这里钉死镜像 URL（实测 url 覆盖生效），让自愈下载不依赖节点可用性。
            obj.insert(
                "geodata".into(),
                serde_json::json!({
                    "auto-update": false,
                    "url": {
                        "mmdb": "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/country.mmdb",
                        "geosite": "https://fastly.jsdelivr.net/gh/MetaCubeX/meta-rules-dat@release/geosite.mrs"
                    }
                }),
            );
        }
        Ok(CompiledArtifact::Json(config))
    }
}

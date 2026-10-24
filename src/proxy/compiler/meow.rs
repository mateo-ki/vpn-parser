use super::{BackendCompiler, CompiledArtifact, CompilerCapabilities};
use crate::kernel::KernelKind;
use crate::proxy::ir::RoutingPlan;
use anyhow::{anyhow, Result};

/// meow（https://github.com/meow-rs/meow-rs）——mihomo 的 Rust 高性能重写。
///
/// 配置面与 Clash API 都按 mihomo 兼容设计（docs/mihomo-api-compatibility.md），
/// 因此编译主体直接复用 [`MihomoCompiler`] 的产物，再做 meow 特化裁剪：
/// - 剥掉 mihomo 的 geo 字段（geodata-mode/geo-auto-update/geox-url）：
///   meow 的 GEOSITE/GEOIP 走自己 home 目录的 geosite.dat / Country.mmdb，
///   由 `crate::geo::ensure_meow_home_geo` 在启动前备好（实测 dat 可直接加载）；
/// - TUN：meow 自带 TUN（Wintun/nftables），但本应用的 TUN 提权链路围绕
///   mihomo 建立，v1 先按不支持处理（capabilities.supports_tun = false，
///   共享的 validate 会给出明确诊断）。
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
            supports_tun: false,
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
        if plan.inbounds.mode.has_tun() {
            return Err(anyhow!(
                "Meow 暂不支持 TUN 模式，请切换至系统代理模式或选择 Mihomo / Sing-box"
            ));
        }
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
        }
        Ok(CompiledArtifact::Json(config))
    }
}

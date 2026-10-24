pub mod meow;
pub mod mihomo;
pub mod shoes;
pub mod sing_box;
pub mod xray;

pub use meow::MeowCompiler;
pub use mihomo::MihomoCompiler;
pub use shoes::ShoesCompiler;
pub use sing_box::SingBoxCompiler;
pub use xray::XrayCompiler;

use crate::kernel::KernelKind;
use crate::proxy::ir::{IRMatcher, RoutingPlan};
use anyhow::Result;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub rule_index: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct CompilerCapabilities {
    pub supports_tun: bool,
    pub supports_process_name: bool,
    pub supports_domain_keyword: bool,
    pub supports_proxy_groups: bool,
    pub supports_url_test: bool,
    pub supported_geosite_categories: Option<HashSet<String>>,
    pub supported_geoip_countries: Option<HashSet<String>>,
}

#[derive(Clone, Debug)]
pub enum CompiledArtifact {
    Yaml(String),
    Json(serde_json::Value),
}

impl CompiledArtifact {
    pub fn to_string_pretty(&self) -> Result<String> {
        match self {
            CompiledArtifact::Yaml(yaml) => Ok(yaml.clone()),
            CompiledArtifact::Json(json) => Ok(serde_json::to_string_pretty(json)?),
        }
    }

    pub fn as_json_value(&self) -> Result<serde_json::Value> {
        match self {
            CompiledArtifact::Json(val) => Ok(val.clone()),
            CompiledArtifact::Yaml(yaml) => Ok(serde_yaml::from_str(yaml)?),
        }
    }
}

pub trait BackendCompiler: Send + Sync {
    fn kernel(&self) -> KernelKind;
    fn capabilities(&self) -> &CompilerCapabilities;

    /// 校验 RoutingPlan 是否有当前内核无法支持的特性，返回诊断信息列表
    fn validate(&self, plan: &RoutingPlan) -> Vec<CompilationDiagnostic> {
        let mut diagnostics = Vec::new();
        let caps = self.capabilities();

        // 校验 TUN 支持
        if plan.inbounds.mode == crate::proxy::ProxyMode::Tun && !caps.supports_tun {
            diagnostics.push(CompilationDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "内核 {} 不支持 TUN 虚拟网卡模式，请切换至系统代理模式或选择 Mihomo / Sing-box",
                    self.kernel().display_name()
                ),
                rule_index: None,
            });
        }

        // 校验 URL-Test 自动测速组
        if plan.outbounds.url_test && !caps.supports_url_test {
            diagnostics.push(CompilationDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "内核 {} 不支持内核内自动选路 (url-test)，将回退至主节点",
                    self.kernel().display_name()
                ),
                rule_index: None,
            });
        }

        // 遍历规则检查匹配器能力
        for (idx, rule) in plan.rules.iter().enumerate() {
            for matcher in &rule.matchers {
                match matcher {
                    IRMatcher::ProcessName(name) if !caps.supports_process_name => {
                        diagnostics.push(CompilationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            message: format!(
                                "内核 {} 不支持基于进程名的分流规则: {name}",
                                self.kernel().display_name()
                            ),
                            rule_index: Some(idx),
                        });
                    }
                    IRMatcher::DomainKeyword(kw) if !caps.supports_domain_keyword => {
                        diagnostics.push(CompilationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            message: format!(
                                "内核 {} 不原生支持域名关键字分流: {kw}，将尝试降级",
                                self.kernel().display_name()
                            ),
                            rule_index: Some(idx),
                        });
                    }
                    IRMatcher::GeoSite(category) => {
                        if let Some(ref allowed) = caps.supported_geosite_categories {
                            if !allowed.contains(category) {
                                diagnostics.push(CompilationDiagnostic {
                                    severity: DiagnosticSeverity::Warning,
                                    message: format!(
                                        "内核 {} 缺少预置规则集: geosite:{category}",
                                        self.kernel().display_name()
                                    ),
                                    rule_index: Some(idx),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        diagnostics
    }

    /// 将统一 RoutingPlan 编译为对应内核的目标配置工件
    fn compile(&self, plan: &RoutingPlan) -> Result<CompiledArtifact>;
}

/// 根据内核种类获取对应的 BackendCompiler 实例
pub fn get_backend_compiler(kernel: KernelKind) -> Box<dyn BackendCompiler> {
    match kernel {
        KernelKind::Mihomo => Box::new(MihomoCompiler::new()),
        KernelKind::SingBox => Box::new(SingBoxCompiler::new()),
        KernelKind::Xray => Box::new(XrayCompiler::new()),
        KernelKind::Shoes => Box::new(ShoesCompiler::new()),
        KernelKind::Meow => Box::new(MeowCompiler),
    }
}

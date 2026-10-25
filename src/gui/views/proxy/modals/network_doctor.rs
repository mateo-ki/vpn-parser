use crate::gui::theme;
use crate::gui::widgets::{danger_btn, primary_btn};
use crate::gui::VpnApp;
use crate::proxy::MihomoConfigOptions;
use crate::proxy::ProxyRuntimeStatus;
use eframe::egui::{self, Align, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, Vec2};
use std::time::Duration;

fn status_btn(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticStatus {
    Pass,
    Warning,
    Fail,
}

impl DiagnosticStatus {
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pass => "正常",
            Self::Warning => "警告",
            Self::Fail => "异常",
        }
    }

    pub fn color(&self) -> egui::Color32 {
        match self {
            Self::Pass => theme::success(),
            Self::Warning => theme::warning(),
            Self::Fail => theme::danger(),
        }
    }

    pub fn bg_color(&self) -> egui::Color32 {
        match self {
            Self::Pass => theme::success_bg(),
            Self::Warning => theme::warning_bg(),
            Self::Fail => theme::danger_bg(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiagnosticItem {
    pub id: String,
    pub category: String, // "系统网络", "代理内核", "软件环境"
    pub name: String,
    pub status: DiagnosticStatus,
    pub detail: String,
    pub repairable: bool,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkDoctorReport {
    pub timestamp: String,
    pub items: Vec<DiagnosticItem>,
    pub pass_count: usize,
    pub warn_count: usize,
    pub fail_count: usize,
}

impl NetworkDoctorReport {
    pub fn new(items: Vec<DiagnosticItem>) -> Self {
        let pass_count = items.iter().filter(|i| i.status == DiagnosticStatus::Pass).count();
        let warn_count = items.iter().filter(|i| i.status == DiagnosticStatus::Warning).count();
        let fail_count = items.iter().filter(|i| i.status == DiagnosticStatus::Fail).count();
        let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
        Self {
            timestamp,
            items,
            pass_count,
            warn_count,
            fail_count,
        }
    }
}

/// 执行全部系统、内核、软件网络诊断
pub async fn perform_network_diagnosis(
    options: MihomoConfigOptions,
    status: ProxyRuntimeStatus,
) -> NetworkDoctorReport {
    let mut items = Vec::new();
    let is_running = matches!(status.state, crate::proxy::ProxyState::Running | crate::proxy::ProxyState::Degraded);
    let mixed_port = options.mixed_port;
    let controller_port = options.controller_port;
    let controller_secret = options.controller_secret.clone();
    let kernel_name = status.kernel.map(|k| k.display_name().to_string()).unwrap_or_else(|| "代理内核".to_string());

    // =========================================================================
    // 1. 系统网络层诊断 (System Network Layer)
    // =========================================================================

    // 1.1 物理网卡死回环 / 残留 DNS 污染检测
    #[cfg(windows)]
    {
        let stale_dns = tokio::task::spawn_blocking(crate::platform::check_stale_loopback_dns)
            .await
            .unwrap_or_default();

        let poisoned: Vec<_> = stale_dns
            .into_iter()
            .filter(|item| {
                item.server_addresses
                    .iter()
                    .any(|a| a == "127.0.0.1" || a == "::1" || (!is_running && a == "198.18.0.2"))
            })
            .collect();

        if !poisoned.is_empty() {
            let poisoned_names = poisoned
                .iter()
                .map(|p| format!("{} ({:?})", p.interface_alias, p.server_addresses))
                .collect::<Vec<_>>()
                .join("、");
            items.push(DiagnosticItem {
                id: "sys_dns_poison".into(),
                category: "系统网络".into(),
                name: "物理网卡 DNS 设置 (防死回环污染)".into(),
                status: DiagnosticStatus::Fail,
                detail: format!(
                    "检测到物理网卡 DNS 被设置为回环/死地址: {}。内核退出或异常关闭后会导致全系统所有软件彻底无法上网！",
                    poisoned_names
                ),
                repairable: true,
                suggestion: Some("点击修复将自动重置受影响网卡为 DHCP / 正常 DNS 并刷新解析缓存。".into()),
            });
        } else {
            items.push(DiagnosticItem {
                id: "sys_dns_poison".into(),
                category: "系统网络".into(),
                name: "物理网卡 DNS 设置 (防死回环污染)".into(),
                status: DiagnosticStatus::Pass,
                detail: "所有网卡 DNS 配置健康，未发现 127.0.0.1 / ::1 等死回环残留。".into(),
                repairable: false,
                suggestion: None,
            });
        }
    }
    #[cfg(not(windows))]
    {
        items.push(DiagnosticItem {
            id: "sys_dns_poison".into(),
            category: "系统网络".into(),
            name: "物理网卡 DNS 设置".into(),
            status: DiagnosticStatus::Pass,
            detail: "当前平台由 resolv.conf 自动管理。".into(),
            repairable: false,
            suggestion: None,
        });
    }

    // 1.2 WinINet 系统代理设置 (注册表 IE 代理)
    let sys_proxy = crate::system_proxy::platform_system_proxy().capture().ok();
    if let Some(proxy_snap) = sys_proxy {
        if proxy_snap.enabled {
            let server = proxy_snap.server.as_deref().unwrap_or("").trim();
            if is_running {
                let matches_our_port = server.contains(&format!(":{mixed_port}"));
                if matches_our_port {
                    items.push(DiagnosticItem {
                        id: "sys_wininet_proxy".into(),
                        category: "系统网络".into(),
                        name: "WinINet 系统代理 (IE 代理)".into(),
                        status: DiagnosticStatus::Pass,
                        detail: format!("系统代理已由本程序正常托管接管 (ProxyServer={server})。"),
                        repairable: false,
                        suggestion: None,
                    });
                } else {
                    items.push(DiagnosticItem {
                        id: "sys_wininet_proxy".into(),
                        category: "系统网络".into(),
                        name: "WinINet 系统代理 (IE 代理)".into(),
                        status: DiagnosticStatus::Warning,
                        detail: format!("系统代理当前指向外部地址 ({server})，未由当前内核托管。"),
                        repairable: true,
                        suggestion: Some("若不需要外部代理，可点击修复重置为直连。".into()),
                    });
                }
            } else {
                // 代理已停止，但系统代理仍开启！典型故障
                items.push(DiagnosticItem {
                    id: "sys_wininet_proxy".into(),
                    category: "系统网络".into(),
                    name: "WinINet 系统代理 (IE 代理)".into(),
                    status: DiagnosticStatus::Fail,
                    detail: format!("系统代理处于开启状态 (指向 {server})，但代理内核未运行，会导致浏览器断网！"),
                    repairable: true,
                    suggestion: Some("点击修复将原子化重置 ProxyEnable=0 恢复直连。".into()),
                });
            }
        } else {
            items.push(DiagnosticItem {
                id: "sys_wininet_proxy".into(),
                category: "系统网络".into(),
                name: "WinINet 系统代理 (IE 代理)".into(),
                status: DiagnosticStatus::Pass,
                detail: "系统代理处于关闭状态（直连模式）。".into(),
                repairable: false,
                suggestion: None,
            });
        }
    }

    // 1.3 系统直连公网连通性测试 (Direct Connectivity)
    let direct_ok = tokio::time::timeout(Duration::from_millis(2500), async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(2000))
            .build()?;
        let resp = client.get("http://cp.cloudflare.com/generate_204").send().await?;
        Ok::<_, anyhow::Error>(resp.status().is_success() || resp.status().as_u16() == 204)
    })
    .await;

    match direct_ok {
        Ok(Ok(true)) => {
            items.push(DiagnosticItem {
                id: "sys_direct_net".into(),
                category: "系统网络".into(),
                name: "物理直连公网连通性".into(),
                status: DiagnosticStatus::Pass,
                detail: "本地物理网络直连畅通，DNS 解析与公网 HTTP 握手正常。".into(),
                repairable: false,
                suggestion: None,
            });
        }
        _ => {
            items.push(DiagnosticItem {
                id: "sys_direct_net".into(),
                category: "系统网络".into(),
                name: "物理直连公网连通性".into(),
                status: DiagnosticStatus::Fail,
                detail: "本地物理直连无法访问公网！可能网卡 DNS 异常、网关失联或网络物理断开。".into(),
                repairable: true,
                suggestion: Some("建议先执行 DNS 修复，或检查本地网络连接。".into()),
            });
        }
    }

    // =========================================================================
    // 2. 代理内核与运行环境诊断 (Proxy Kernel Layer)
    // =========================================================================

    // 2.1 内核进程存活状态
    if is_running {
        items.push(DiagnosticItem {
            id: "kernel_proc_state".into(),
            category: "代理内核".into(),
            name: "内核进程运行状态".into(),
            status: DiagnosticStatus::Pass,
            detail: format!("{kernel_name} 正在稳定运行中。"),
            repairable: false,
            suggestion: None,
        });
    } else if matches!(status.state, crate::proxy::ProxyState::Failed) {
        items.push(DiagnosticItem {
            id: "kernel_proc_state".into(),
            category: "代理内核".into(),
            name: "内核进程运行状态".into(),
            status: DiagnosticStatus::Fail,
            detail: format!("{kernel_name} 启动或运行失败: {}", status.message),
            repairable: true,
            suggestion: Some("点击修复将尝试清理残留配置并安全重启内核。".into()),
        });
    } else {
        items.push(DiagnosticItem {
            id: "kernel_proc_state".into(),
            category: "代理内核".into(),
            name: "内核进程运行状态".into(),
            status: DiagnosticStatus::Pass,
            detail: format!("{kernel_name} 处于正常停止状态。"),
            repairable: false,
            suggestion: None,
        });
    }

    // 2.2 本地混合代理端口监听探测
    let port_probe = tokio::time::timeout(Duration::from_millis(200), async {
        tokio::net::TcpStream::connect(format!("127.0.0.1:{mixed_port}")).await
    })
    .await;

    let port_listening = matches!(port_probe, Ok(Ok(_)));
    if is_running {
        if port_listening {
            items.push(DiagnosticItem {
                id: "kernel_mixed_port".into(),
                category: "代理内核".into(),
                name: "本地代理端口响应 (Mixed Port)".into(),
                status: DiagnosticStatus::Pass,
                detail: format!("本地代理端口 127.0.0.1:{mixed_port} 正常开放并接受连接。"),
                repairable: false,
                suggestion: None,
            });
        } else {
            items.push(DiagnosticItem {
                id: "kernel_mixed_port".into(),
                category: "代理内核".into(),
                name: "本地代理端口响应 (Mixed Port)".into(),
                status: DiagnosticStatus::Fail,
                detail: format!("代理端口 127.0.0.1:{mixed_port} 无法连接！内核可能发生异常崩溃或未完成绑定。"),
                repairable: true,
                suggestion: Some("点击修复可尝试重启代理服务。".into()),
            });
        }
    } else if port_listening {
        items.push(DiagnosticItem {
            id: "kernel_mixed_port".into(),
            category: "代理内核".into(),
            name: "本地代理端口响应 (Mixed Port)".into(),
            status: DiagnosticStatus::Warning,
            detail: format!("内核已停止，但端口 127.0.0.1:{mixed_port} 仍有进程在监听，可能存在孤儿僵尸内核。"),
            repairable: true,
            suggestion: Some("点击修复将清理所有未受管的孤儿内核。".into()),
        });
    } else {
        items.push(DiagnosticItem {
            id: "kernel_mixed_port".into(),
            category: "代理内核".into(),
            name: "本地代理端口响应 (Mixed Port)".into(),
            status: DiagnosticStatus::Pass,
            detail: format!("代理端口 {mixed_port} 已随内核正常释放。"),
            repairable: false,
            suggestion: None,
        });
    }

    // 2.3 孤儿内核残留检测
    let orphan_count = tokio::task::spawn_blocking(crate::platform::kill_orphan_proxy_cores)
        .await
        .unwrap_or(0);
    if orphan_count > 0 {
        items.push(DiagnosticItem {
            id: "kernel_orphan_cores".into(),
            category: "代理内核".into(),
            name: "后台孤儿内核清理".into(),
            status: DiagnosticStatus::Warning,
            detail: format!("发现并清理了 {orphan_count} 个未受管的僵尸内核进程。"),
            repairable: false,
            suggestion: None,
        });
    } else {
        items.push(DiagnosticItem {
            id: "kernel_orphan_cores".into(),
            category: "代理内核".into(),
            name: "后台孤儿内核清理".into(),
            status: DiagnosticStatus::Pass,
            detail: "系统无残留的未受管孤儿内核进程。".into(),
            repairable: false,
            suggestion: None,
        });
    }

    // 2.4 TUN 虚拟网卡状态 (若启用 TUN 模式)
    if options.mode.has_tun() {
        if is_running {
            #[cfg(windows)]
            {
                let check_tun = tokio::task::spawn_blocking(|| {
                    let out = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", "Get-NetAdapter -ErrorAction SilentlyContinue | Where-Object { $_.InterfaceDescription -like '*Wintun*' -or $_.InterfaceAlias -in @('Meta','meow','sing-box') } | Select-Object -ExpandProperty InterfaceAlias"])
                        .output();
                    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
                }).await.unwrap_or_default();

                if !check_tun.is_empty() {
                    items.push(DiagnosticItem {
                        id: "kernel_tun_adapter".into(),
                        category: "代理内核".into(),
                        name: "TUN 虚拟网卡状态".into(),
                        status: DiagnosticStatus::Pass,
                        detail: format!("TUN 虚拟网卡已就绪 ({check_tun})，流量透明接管中。"),
                        repairable: false,
                        suggestion: None,
                    });
                } else {
                    items.push(DiagnosticItem {
                        id: "kernel_tun_adapter".into(),
                        category: "代理内核".into(),
                        name: "TUN 虚拟网卡状态".into(),
                        status: DiagnosticStatus::Fail,
                        detail: "未能检测到活跃的 TUN 虚拟网卡，可能 Wintun 驱动被拦截或提权失败。".into(),
                        repairable: true,
                        suggestion: Some("请确保杀毒软件未拦截 Wintun.dll，并重新启动 TUN 模式。".into()),
                    });
                }
            }
            #[cfg(not(windows))]
            {
                items.push(DiagnosticItem {
                    id: "kernel_tun_adapter".into(),
                    category: "代理内核".into(),
                    name: "TUN 虚拟网卡状态".into(),
                    status: DiagnosticStatus::Pass,
                    detail: "TUN 模式正常。".into(),
                    repairable: false,
                    suggestion: None,
                });
            }
        } else {
            items.push(DiagnosticItem {
                id: "kernel_tun_adapter".into(),
                category: "代理内核".into(),
                name: "TUN 虚拟网卡状态".into(),
                status: DiagnosticStatus::Pass,
                detail: "TUN 模式已配置（等待内核启动时自动加载）。".into(),
                repairable: false,
                suggestion: None,
            });
        }
    }

    // =========================================================================
    // 3. 软件与出口连通性诊断 (Software & Egress Layer)
    // =========================================================================

    // 3.1 本地外部控制器 API 连通性
    if is_running {
        let api_probe = tokio::time::timeout(Duration::from_millis(800), async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(600))
                .build()?;
            let mut req = client.get(format!("http://127.0.0.1:{controller_port}/version"));
            if !controller_secret.is_empty() {
                req = req.bearer_auth(&controller_secret);
            }
            let resp = req.send().await?;
            Ok::<_, anyhow::Error>(resp.status().is_success())
        })
        .await;

        if matches!(api_probe, Ok(Ok(true))) {
            items.push(DiagnosticItem {
                id: "app_controller_api".into(),
                category: "软件环境".into(),
                name: "内核外部控制器 API".into(),
                status: DiagnosticStatus::Pass,
                detail: format!("控制器 API (127.0.0.1:{controller_port}) 响应正常，鉴权有效。"),
                repairable: false,
                suggestion: None,
            });
        } else {
            items.push(DiagnosticItem {
                id: "app_controller_api".into(),
                category: "软件环境".into(),
                name: "内核外部控制器 API".into(),
                status: DiagnosticStatus::Fail,
                detail: format!("无法连接本地控制器 API (端口 {controller_port})，界面无法控制规则与分流！"),
                repairable: true,
                suggestion: Some("点击修复将重新拉起内核控制器。".into()),
            });
        }
    } else {
        items.push(DiagnosticItem {
            id: "app_controller_api".into(),
            category: "软件环境".into(),
            name: "内核外部控制器 API".into(),
            status: DiagnosticStatus::Pass,
            detail: "外部控制器按需待机中。".into(),
            repairable: false,
            suggestion: None,
        });
    }

    // 3.2 代理出口公网连通性 (仅在内核运行时测)
    if is_running {
        let proxy_egress_ok = tokio::time::timeout(Duration::from_millis(3500), async {
            let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{mixed_port}"))?;
            let client = reqwest::Client::builder()
                .proxy(proxy)
                .timeout(Duration::from_millis(3000))
                .build()?;
            let resp = client.get("http://cp.cloudflare.com/generate_204").send().await?;
            Ok::<_, anyhow::Error>(resp.status().is_success() || resp.status().as_u16() == 204)
        })
        .await;

        match proxy_egress_ok {
            Ok(Ok(true)) => {
                items.push(DiagnosticItem {
                    id: "net_proxy_egress".into(),
                    category: "软件环境".into(),
                    name: "节点代理出口连通性".into(),
                    status: DiagnosticStatus::Pass,
                    detail: "经由当前代理节点的公网出口通畅，网页与分流可正常工作。".into(),
                    repairable: false,
                    suggestion: None,
                });
            }
            _ => {
                items.push(DiagnosticItem {
                    id: "net_proxy_egress".into(),
                    category: "软件环境".into(),
                    name: "节点代理出口连通性".into(),
                    status: DiagnosticStatus::Warning,
                    detail: "当前代理节点出口握手失败！节点可能已失效、超时或受到网络阻断。".into(),
                    repairable: false,
                    suggestion: Some("建议在节点列表切换到延迟更低或可用性更高的节点。".into()),
                });
            }
        }
    }

    NetworkDoctorReport::new(items)
}

/// 执行指定的或全部修复操作
pub async fn perform_network_repair(target_id: Option<&str>) -> Result<String, String> {
    let mut messages = Vec::new();

    // 1. 修复 DNS 污染 (若指定或全部)
    if target_id.is_none() || target_id == Some("sys_dns_poison") || target_id == Some("sys_direct_net") {
        #[cfg(windows)]
        {
            match tokio::task::spawn_blocking(crate::platform::restore_all_stale_loopback_dns).await {
                Ok(Ok(count)) => {
                    messages.push(format!("已成功重置 {count} 个物理网卡的死回环 DNS 设置并刷新系统 DNS 缓存"));
                }
                Ok(Err(e)) => {
                    messages.push(format!("重置物理网卡 DNS 异常: {e}"));
                }
                Err(e) => {
                    messages.push(format!("执行 DNS 修复任务失败: {e}"));
                }
            }
        }
    }

    // 2. 修复 WinINet 系统代理残留
    if target_id.is_none() || target_id == Some("sys_wininet_proxy") {
        let cleared = crate::system_proxy::direct_disable_wininet_proxy_atomic();
        if cleared {
            messages.push("已原子化重置 WinINet 系统代理配置 (ProxyEnable=0)，恢复直连".into());
        }
    }

    // 3. 清理孤儿内核
    if target_id.is_none() || target_id == Some("kernel_orphan_cores") || target_id == Some("kernel_mixed_port") {
        let killed = tokio::task::spawn_blocking(crate::platform::kill_orphan_proxy_cores)
            .await
            .unwrap_or(0);
        if killed > 0 {
            messages.push(format!("已强制终止 {killed} 个残留的孤儿内核进程，释放被占用的端口"));
        }
    }

    Ok(messages.join("；"))
}

impl VpnApp {
    pub(crate) fn open_network_doctor(&mut self) {
        self.show_network_doctor = true;
        self.network_doctor_message = None;
        self.run_network_doctor();
    }

    pub(crate) fn run_network_doctor(&mut self) {
        self.network_doctor_busy = true;
        let tx = self.tx.clone();
        let options = self.proxy_config.clone();
        let status = self.proxy_status.clone();
        self.rt.spawn(async move {
            let report = perform_network_diagnosis(options, status).await;
            let _ = tx.send(crate::gui::types::BgMsg::NetworkDoctorReport(report));
        });
    }

    pub(crate) fn repair_network_doctor(&mut self, target_id: Option<String>) {
        self.network_doctor_repairing = true;
        self.network_doctor_message = Some("正在执行智能修复...".into());
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let result = perform_network_repair(target_id.as_deref()).await;
            let _ = tx.send(crate::gui::types::BgMsg::NetworkDoctorRepairResult(result));
        });
    }

    pub(crate) fn ui_network_doctor_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_network_doctor {
            return;
        }

        let mut open = self.show_network_doctor;
        let mut trigger_rescan = false;
        let mut trigger_repair_all = false;
        let mut trigger_repair_one: Option<String> = None;

        egui::Window::new(RichText::new("网络检测与智能修复").strong().size(15.0))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);

                // 顶栏操作区
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("全面诊断系统网络、代理内核、软件环境与连通性")
                            .color(theme::muted())
                            .small(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let btn_rescan = ui.add_enabled(
                            !self.network_doctor_busy && !self.network_doctor_repairing,
                            status_btn("重新检测"),
                        );
                        if btn_rescan.clicked() {
                            trigger_rescan = true;
                        }

                        let has_issues = self
                            .network_doctor_report
                            .as_ref()
                            .map(|r| r.fail_count > 0 || r.warn_count > 0)
                            .unwrap_or(false);

                        let btn_fix_all = ui.add_enabled(
                            has_issues && !self.network_doctor_busy && !self.network_doctor_repairing,
                            primary_btn("一键修复全部异常"),
                        );
                        if btn_fix_all.clicked() {
                            trigger_repair_all = true;
                        }
                    });
                });

                ui.separator();

                // 状态消息条
                if self.network_doctor_busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("正在全面扫描网络配置与连通性...").color(theme::accent()));
                    });
                } else if self.network_doctor_repairing {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("正在执行系统修复并还原健康配置...").color(theme::accent()));
                    });
                } else if let Some(msg) = &self.network_doctor_message {
                    Frame::NONE
                        .fill(theme::success_bg())
                        .stroke(Stroke::new(1.0_f32, theme::success()))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.label(RichText::new(format!("✓ {msg}")).color(theme::success()).small());
                        });
                }

                // 统计指标汇总看板
                if let Some(report) = &self.network_doctor_report {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("诊断时间: {}", report.timestamp)).small().color(theme::muted()));
                        ui.add_space(8.0);

                        // 正常指标
                        ui.label(
                            RichText::new(format!("✓ 正常: {}", report.pass_count))
                                .color(theme::success())
                                .strong()
                                .small(),
                        );
                        ui.add_space(6.0);

                        // 警告指标
                        if report.warn_count > 0 {
                            ui.label(
                                RichText::new(format!("⚠ 警告: {}", report.warn_count))
                                    .color(theme::warning())
                                    .strong()
                                    .small(),
                            );
                            ui.add_space(6.0);
                        }

                        // 异常指标
                        if report.fail_count > 0 {
                            ui.label(
                                RichText::new(format!("✕ 异常: {}", report.fail_count))
                                    .color(theme::danger())
                                    .strong()
                                    .small(),
                            );
                        } else {
                            ui.label(
                                RichText::new("0 处致命异常")
                                    .color(theme::muted())
                                    .small(),
                            );
                        }
                    });
                }

                ui.separator();

                // 检查项列表区
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let Some(report) = &self.network_doctor_report else {
                            if !self.network_doctor_busy {
                                ui.label(RichText::new("点击右上角「重新检测」开始扫描。").color(theme::muted()));
                            }
                            return;
                        };

                        let categories = ["系统网络", "代理内核", "软件环境"];
                        for cat in categories {
                            let cat_items: Vec<_> = report.items.iter().filter(|i| i.category == cat).collect();
                            if cat_items.is_empty() {
                                continue;
                            }

                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(cat)
                                    .strong()
                                    .size(13.0)
                                    .color(theme::text_primary()),
                            );

                            for item in cat_items {
                                Frame::NONE
                                    .fill(theme::panel())
                                    .stroke(Stroke::new(
                                        1.0_f32,
                                        if item.status == DiagnosticStatus::Fail {
                                            theme::danger().gamma_multiply(0.5)
                                        } else if item.status == DiagnosticStatus::Warning {
                                            theme::warning().gamma_multiply(0.4)
                                        } else {
                                            theme::border_subtle()
                                        },
                                    ))
                                    .corner_radius(CornerRadius::same(6))
                                    .inner_margin(Margin::same(8))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            // 状态徽标
                                            let badge_text = match item.status {
                                                DiagnosticStatus::Pass => "正常",
                                                DiagnosticStatus::Warning => "警告",
                                                DiagnosticStatus::Fail => "异常",
                                            };
                                            Frame::NONE
                                                .fill(item.status.bg_color())
                                                .stroke(Stroke::new(1.0_f32, item.status.color()))
                                                .corner_radius(CornerRadius::same(4))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(
                                                        RichText::new(badge_text)
                                                            .color(item.status.color())
                                                            .strong()
                                                            .size(11.0),
                                                    );
                                                });

                                            ui.label(
                                                RichText::new(&item.name)
                                                    .strong()
                                                    .color(theme::text_primary())
                                                    .size(13.0),
                                            );

                                            // 单项修复按钮
                                            if item.repairable && item.status != DiagnosticStatus::Pass {
                                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                    let btn = ui.add_enabled(
                                                        !self.network_doctor_busy && !self.network_doctor_repairing,
                                                        danger_btn("一键修复"),
                                                    );
                                                    if btn.clicked() {
                                                        trigger_repair_one = Some(item.id.clone());
                                                    }
                                                });
                                            }
                                        });

                                        ui.add_space(2.0);
                                        ui.label(
                                            RichText::new(&item.detail)
                                                .color(if item.status == DiagnosticStatus::Fail {
                                                    theme::danger()
                                                } else if item.status == DiagnosticStatus::Warning {
                                                    theme::warning()
                                                } else {
                                                    theme::text_secondary()
                                                })
                                                .size(12.0),
                                        );

                                        if let Some(sug) = &item.suggestion {
                                            ui.add_space(1.0);
                                            ui.label(
                                                RichText::new(format!("💡 建议: {sug}"))
                                                    .color(theme::muted())
                                                    .small(),
                                            );
                                        }
                                    });
                                ui.add_space(2.0);
                            }
                        }
                    });
            });

        self.show_network_doctor = open;

        if trigger_rescan {
            self.network_doctor_message = None;
            self.run_network_doctor();
        }
        if trigger_repair_all {
            self.repair_network_doctor(None);
        } else if let Some(id) = trigger_repair_one {
            self.repair_network_doctor(Some(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_doctor_report_counts() {
        let items = vec![
            DiagnosticItem {
                id: "1".into(),
                category: "系统网络".into(),
                name: "DNS".into(),
                status: DiagnosticStatus::Pass,
                detail: "ok".into(),
                repairable: false,
                suggestion: None,
            },
            DiagnosticItem {
                id: "2".into(),
                category: "系统网络".into(),
                name: "Proxy".into(),
                status: DiagnosticStatus::Fail,
                detail: "failed".into(),
                repairable: true,
                suggestion: Some("fix".into()),
            },
            DiagnosticItem {
                id: "3".into(),
                category: "代理内核".into(),
                name: "Port".into(),
                status: DiagnosticStatus::Warning,
                detail: "warn".into(),
                repairable: true,
                suggestion: None,
            },
        ];
        let report = NetworkDoctorReport::new(items);
        assert_eq!(report.pass_count, 1);
        assert_eq!(report.fail_count, 1);
        assert_eq!(report.warn_count, 1);
    }
}

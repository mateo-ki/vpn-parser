pub mod charts;
pub mod connections;
pub mod groups;
pub mod modals;
pub mod network;
pub mod nodes;
pub mod port_mappings;
pub mod rules;
pub mod security;

#[cfg(test)]
mod tests;

pub use groups::node_matches_group;
pub(crate) use groups::{override_group_names, visible_group_names};
pub use modals::ui_rule_import_help_window;
pub use network::clean_kernel_version;
pub(crate) use rules::{RULES_ACTIONS_WIDTH, RULES_TABS_MIN_WIDTH};

use crate::gui::icons::{self, icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    ActiveConnectionInfo, ConnectionsTab, DnsRecord, GroupsTab, KernelGroup, ProxyPanel, ProxySortBy,
    ProxyView, ProxyViewRow, RuleDraftKind, RulesTab,
};
use crate::gui::view_models::nodes_vm::{NodeCardAction, NodesViewModel};
use crate::gui::widgets::{
    card_frame, compact_node_card, country_flag_and_name, danger_btn, dns_mode_label,
    format_bytes, format_speed, header_cell, macos_segmented_tabs, panel_frame, panel_tab, primary_btn,
    protocol_pill, render_country_code_badge, render_health_strip, render_node_health_bar,
    render_protocol_donut_chart_sized, render_signal_bars, render_speedometer, render_top_processes_card,
    render_traffic_waveform_card, routing_mode_description, routing_mode_detail,
    rule_action_label, rule_matcher_label, sanitize_node_name, section_title, status_badge, tun_stack_label,
};
use crate::gui::VpnApp;
use crate::kernel::{KernelKind, KernelPreference};
use crate::models::NodeSummary;
use crate::proxy::{
    adblock_quic_rules, ai_service_rules, dev_mirrors_rules, game_platforms_rules,
    mihomo_dns_poison_overrides, optimize_traffic_rules, recommended_traffic_rules,
    social_telegram_rules, streaming_media_rules, AutoRoutingStrategy, DnsEnhancedMode, ProxyMode, ProxyState,
    RoutingMode, RuleAction, TrafficRule, TunStack, URL_TEST_GROUP,
};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;

pub const CLASH_VERGE_RECOMMENDED_OVERRIDE: &str = r#"# =========================================================
# Clash Verge Rev / Mihomo 通用推荐覆写片段
# =========================================================
proxy-groups:
  - name: 多线并发下载
    type: load-balance
    strategy: round-robin
    url: https://cp.cloudflare.com/generate_204
    interval: 300
    timeout: 2000
    lazy: true
    include-all: true
    filter: "(?i)(港|新加坡|日|美|\\b(?:HK|SG|JP|US|USA)\\b|Hong[ _-]?Kong|Singapore|Japan|America|United[ _-]?States)"
    exclude-filter: "【(?:[2-9]|[1-9][0-9]+)x】|链式"

  - name: 美国节点
    type: url-test
    url: https://cp.cloudflare.com/generate_204
    interval: 300
    timeout: 2000
    lazy: true
    include-all: true
    filter: "(?i)(美|\\bUS(?:A)?\\b|America|United[ _-]?States|Los[ _-]?Angeles|San[ _-]?Jose|Seattle|Chicago|Dallas|New[ _-]?York|Virginia|Washington)"

  - name: 普通代理
    type: select
    proxies:
      - 美国节点
    include-all: true

  - name: AI分组
    type: select
    proxies:
      - 美国节点
      - 普通代理

rules:
  # 局域网 / 私有地址
  - IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
  - IP-CIDR,172.16.0.0/12,DIRECT,no-resolve
  - IP-CIDR,192.168.0.0/16,DIRECT,no-resolve
  - IP-CIDR6,fc00::/7,DIRECT,no-resolve
  - IP-CIDR6,fe80::/10,DIRECT,no-resolve
  - DST-PORT,22,DIRECT

  # AI 视频与主流 AI 服务
  - DOMAIN-SUFFIX,runwayml.com,AI分组
  - DOMAIN-KEYWORD,runwayml,AI分组
  - DOMAIN-SUFFIX,runway.com,AI分组
  - DOMAIN-SUFFIX,muse.ai,AI分组
  - DOMAIN-SUFFIX,openai.com,AI分组
  - DOMAIN-SUFFIX,chatgpt.com,AI分组
  - DOMAIN-SUFFIX,oaistatic.com,AI分组
  - DOMAIN-SUFFIX,oaiusercontent.com,AI分组
  - DOMAIN-SUFFIX,anthropic.com,AI分组
  - DOMAIN-SUFFIX,claude.ai,AI分组
  - DOMAIN-SUFFIX,claudeusercontent.com,AI分组
  - DOMAIN-SUFFIX,x.com,AI分组
  - DOMAIN-SUFFIX,grok.com,AI分组
  - DOMAIN-SUFFIX,perplexity.ai,AI分组
  - DOMAIN-SUFFIX,poe.com,AI分组
  - DOMAIN-SUFFIX,character.ai,AI分组
  - DOMAIN-SUFFIX,stability.ai,AI分组
  - DOMAIN-SUFFIX,huggingface.co,AI分组
  - DOMAIN-SUFFIX,replicate.com,AI分组
  - DOMAIN-SUFFIX,ai.com,AI分组
  - DOMAIN-SUFFIX,linux.do,AI分组
  - DOMAIN-SUFFIX,hybgzs.com,AI分组

  # Google / Gemini / YouTube -> 美国节点
  - DOMAIN,gemini.google.com,美国节点
  - DOMAIN-SUFFIX,google.com,美国节点
  - DOMAIN-SUFFIX,google.com.hk,美国节点
  - DOMAIN-SUFFIX,google.co.jp,美国节点
  - DOMAIN-SUFFIX,google.co.uk,美国节点
  - DOMAIN-SUFFIX,google.com.tw,美国节点
  - DOMAIN-SUFFIX,google.dev,美国节点
  - DOMAIN-SUFFIX,gstatic.com,美国节点
  - DOMAIN-SUFFIX,googleapis.com,美国节点
  - DOMAIN-SUFFIX,googleapis.cn,美国节点
  - DOMAIN-SUFFIX,googleusercontent.com,美国节点
  - DOMAIN-SUFFIX,ggpht.com,美国节点
  - DOMAIN-SUFFIX,gvt1.com,美国节点
  - DOMAIN-SUFFIX,gvt2.com,美国节点
  - DOMAIN-SUFFIX,1e100.net,美国节点
  - DOMAIN-SUFFIX,gmail.com,美国节点
  - DOMAIN-SUFFIX,googlemail.com,美国节点
  - DOMAIN-SUFFIX,recaptcha.net,美国节点
  - DOMAIN-SUFFIX,youtube.com,美国节点
  - DOMAIN-SUFFIX,youtu.be,美国节点
  - DOMAIN-SUFFIX,ytimg.com,美国节点
  - DOMAIN-SUFFIX,googlevideo.com,美国节点
  - DOMAIN-SUFFIX,youtube-nocookie.com,美国节点
  - GEOSITE,google,美国节点
  - GEOSITE,youtube,美国节点
  - GEOIP,google,美国节点,no-resolve

  # 其他服务
  - DOMAIN-SUFFIX,midjourney.com,美国节点
  - DOMAIN-SUFFIX,capcut.com,美国节点

  # 常用直连
  - DOMAIN-SUFFIX,bing.com,DIRECT
  - DOMAIN-SUFFIX,azureedge.net,DIRECT
  - DOMAIN-SUFFIX,onedrive.com,DIRECT
  - DOMAIN-SUFFIX,1drv.com,DIRECT
  - DOMAIN-SUFFIX,muyuan.do,DIRECT
  - DOMAIN-SUFFIX,feishu.cn,DIRECT
  - DOMAIN-SUFFIX,turgame.com,DIRECT

  # 程序直连
  - PROCESS-NAME,steam.exe,DIRECT
  - PROCESS-NAME,eCloud.exe,DIRECT
  - PROCESS-NAME,BaiduNetdisk.exe,DIRECT
  - PROCESS-NAME,QQ.exe,DIRECT
  - PROCESS-NAME,Weixin.exe,DIRECT

  # 多线并发下载工具
  - PROCESS-NAME,fdm.exe,多线并发下载
  - PROCESS-NAME,PikPak.exe,多线并发下载
  - PROCESS-NAME,IDMan.exe,多线并发下载
  - PROCESS-NAME,Motrix.exe,多线并发下载
  - PROCESS-NAME,Thunder.exe,多线并发下载
"#;

fn status_btn(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border()))
}



impl VpnApp {
    pub(crate) fn ui_proxy(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);

        // ===================================================================
        // macOS 药丸分段选择条 (Segmented Control Bar)
        // ===================================================================
        let tr_charts = self.tr("汇总图表");
        let tr_nodes = self.tr("可用节点");
        let tr_groups = self.tr("分流策略组");
        let tr_conns = self.tr("活跃连接");
        let tr_rules = self.tr("分流规则");
        let tr_network = self.tr("DNS / TUN");
        let conn_label = if self.active_connections.is_empty() {
            tr_conns.to_string()
        } else {
            format!("{} ({})", tr_conns, self.active_connections.len())
        };

        let tabs = [
            (ProxyPanel::Charts, Icon::Dashboard, tr_charts),
            (ProxyPanel::Nodes, Icon::Pin, tr_nodes),
            (ProxyPanel::Groups, Icon::Rocket, tr_groups),
            (ProxyPanel::Connections, Icon::Radar, &conn_label),
            (ProxyPanel::Rules, Icon::Rules, tr_rules),
            (ProxyPanel::Network, Icon::Globe, tr_network),
            (ProxyPanel::Kernels, Icon::Cpu, self.tr("内核管理")),
            (ProxyPanel::Logs, Icon::Terminal, self.tr("运行日志")),
        ];

        let visible_tabs: Vec<_> = tabs
            .into_iter()
            .filter(|(panel, _, _)| self.is_proxy_panel_visible(*panel))
            .collect();

        // 若当前激活面板已被隐藏，平滑回退到第一个可见面板
        if !visible_tabs.iter().any(|(panel, _, _)| *panel == self.proxy_panel) {
            if let Some((first_panel, _, _)) = visible_tabs.first() {
                self.proxy_panel = *first_panel;
            }
        }

        macos_segmented_tabs(ui, &mut self.proxy_panel, &visible_tabs);
        ui.add_space(8.0);

        match self.proxy_panel {
            ProxyPanel::Charts => {
                ui.push_id("panel_charts", |ui| self.ui_proxy_charts(ui));
            }
            ProxyPanel::Nodes => {
                ui.push_id("panel_nodes", |ui| self.ui_proxy_nodes(ui));
            }
            ProxyPanel::Groups => {
                ui.push_id("panel_groups", |ui| self.ui_proxy_groups(ui));
            }
            ProxyPanel::Connections => {
                ui.push_id("panel_connections", |ui| self.ui_proxy_connections(ui));
            }
            ProxyPanel::Rules => {
                ui.push_id("panel_rules", |ui| self.ui_proxy_rules(ui));
            }
            ProxyPanel::Network => {
                ui.push_id("panel_network", |ui| self.ui_proxy_network(ui));
            }
            ProxyPanel::PortMappings => {
                self.tab = crate::gui::types::Tab::Extensions;
                self.extensions_tab = crate::gui::types::ExtensionsTab::PortMappings;
            }
            ProxyPanel::Security => {
                self.tab = crate::gui::types::Tab::Extensions;
                self.extensions_tab = crate::gui::types::ExtensionsTab::Security;
            }
            ProxyPanel::Kernels => {
                ui.push_id("panel_kernels", |ui| self.ui_proxy_kernels(ui));
            }
            ProxyPanel::Logs => {
                ui.push_id("panel_logs", |ui| self.ui_proxy_logs(ui));
            }
        }
    }


    pub(crate) fn ui_proxy_control_buttons(&mut self, ui: &mut Ui) {
        let running = matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        );
        let transitioning = matches!(
            self.proxy_status.state,
            ProxyState::Starting | ProxyState::Stopping
        );
        if running {
            if ui
                .add_enabled(!self.proxy_busy, danger_btn(self.tr("停止代理")))
                .clicked()
            {
                self.stop_proxy();
            }
        } else {
            let has_executable_node = self.selected_proxy_node_id.is_some()
                || self.focused_node_id.is_some()
                || !self.nodes.is_empty()
                || !self.proxy_nodes.is_empty();
            let can_start = !self.proxy_busy && !transitioning && has_executable_node;
            let start = ui.add_enabled(can_start, primary_btn(self.tr("启动代理")));
            let start = if !has_executable_node {
                start.on_hover_text("节点列表为空，请先在订阅源拉取节点")
            } else if self.proxy_busy {
                start.on_hover_text(format!(
                    "上一个代理操作还没回信，按钮先灰着：{}",
                    self.status
                ))
            } else if transitioning {
                start.on_hover_text(format!(
                    "状态还停在「{}」，等它跑完或者失败才能按",
                    self.proxy_status.message
                ))
            } else if self.selected_proxy_node_id.is_none() && self.focused_node_id.is_none() && self.proxy_config.url_test {
                start.on_hover_text("点击以当前策略组自动选优启动")
            } else {
                start
            };
            if start.clicked() {
                self.start_proxy();
            }
        }

        // 系统代理 / TUN：一个按钮来回切，运行中不给改（改了要重载内核）。
        // 以前它是手搓的 `Frame`（6px 圆角、自己的内边距、9px 小字），和同排那三个
        // 按钮四种长相 —— 现在走同一个 `status_btn`，只剩语义差别（用户 2026-10-15）。
        let mode_label = match self.proxy_config.mode {
            // Linux 没有"系统代理"那一格，这个模式下实际做的事就是把端口开着等人来指，
            // 所以叫「本地端口」；存储值仍是 System，不改数据层、不发明第三态。
            ProxyMode::System if crate::system_proxy::system_proxy_supported_on_this_platform() => {
                "系统代理"
            }
            ProxyMode::System => "本地端口",
            ProxyMode::Tun => "TUN 模式",
        };
        let mode_resp = ui
            .add(status_btn(mode_label))
            .on_hover_text("点击切换 系统代理 / TUN 模式");
        if mode_resp.clicked() {
            if self.proxy_status.config_editable() {
                self.proxy_config.mode = match self.proxy_config.mode {
                    ProxyMode::System => ProxyMode::Tun,
                    ProxyMode::Tun => ProxyMode::System,
                };
                self.save_proxy_config();
            } else {
                self.status = "请先停止代理后再切换工作模式".into();
            }
        }

        // 这里原来挂过一条「系统代理被外部关闭」的黄字，2026-10-15 他一句"不用显示这个"撤了：
        // 核对逻辑还在（喂设置页那个「自动重开系统代理」开关），但界面不再画它。

        // 测速全部：走的就是订阅源页/节点页那条 `speedtest_all_sources`，不另开一条通道。
        // 并发用系统设置里那个数（默认 128），批量作业自带 TCP 预筛。
        let testing = self.busy;
        if ui
            .add_enabled(!testing, status_btn("测速全部"))
            .on_hover_text(if testing {
                "已经有任务在跑，等它结束再测速"
            } else {
                "对所有订阅节点跑一轮批量测速：先 TCP 预筛，再逐个真实握手测延迟"
            })
            .clicked()
        {
            self.speedtest_all_sources();
        }
    }

}

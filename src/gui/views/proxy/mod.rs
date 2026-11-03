pub mod charts;
pub mod connections;
pub mod groups;
pub mod modals;
pub mod network;
pub mod nodes;
pub mod port_mappings;
pub mod rules;
pub mod security;
pub mod traffic;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use groups::node_matches_group;
#[allow(unused_imports)]
pub(crate) use groups::{override_group_names, visible_group_names};
#[allow(unused_imports)]
pub use modals::ui_rule_import_help_window;
#[allow(unused_imports)]
pub use network::clean_kernel_version;
#[allow(unused_imports)]
pub(crate) use rules::{RULES_ACTIONS_WIDTH, RULES_TABS_MIN_WIDTH};


use crate::gui::theme;
use crate::gui::widgets::{danger_btn, primary_btn};
use crate::gui::VpnApp;
use crate::proxy::ProxyState;
use eframe::egui::{self, RichText, Stroke, Ui};

pub use crate::proxy::CLASH_VERGE_RECOMMENDED_OVERRIDE;

fn status_btn(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).strong())
        .fill(theme::panel())
        .stroke(Stroke::new(1.0_f32, theme::border()))
}



impl VpnApp {
    pub(crate) fn ui_proxy(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        self.ui_proxy_charts(ui);
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
                .add_enabled(!self.proxy_busy, danger_btn(self.tr("关闭内核")))
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
            let start = ui.add_enabled(can_start, primary_btn(self.tr("启动内核")));
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

        // 系统代理 / TUN 切换按钮已移除（2026-10-24：仪表盘底部控制栏已有同样功能）。

        // 测速全部：走的就是订阅源页/节点页那条 `speedtest_all_sources`，不另开一条通道。
        // 并发用系统设置里那个数（默认 128），批量作业自带 TCP 预筛。
        if self.speedtest_busy {
            if ui
                .add(danger_btn("停止测速"))
                .on_hover_text("正在批量测速中，点击立即停止并清理测速任务")
                .clicked()
            {
                self.cancel_speedtest();
            }
        } else {
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

        if ui
            .add(status_btn("网络检测"))
            .on_hover_text("全面体检：检测软件、内核与系统网络状态，支持一键修复死回环DNS/死代理/端口冲突")
            .clicked()
        {
            self.open_network_doctor();
        }
    }

}

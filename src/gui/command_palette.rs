use super::{theme, VpnApp};
use crate::gui::types::{ProxyPanel, Tab};
use crate::proxy::{ProxyMode, ProxyState};
use eframe::egui::{self, CornerRadius, Margin, RichText, Sense, Vec2};

impl VpnApp {
    /// 命令面板里的可执行项。节点也进面板（模糊搜名字直接切出口），所以每帧动态生成。
    fn palette_items(&self) -> Vec<crate::gui::types::PaletteItem> {
        use crate::gui::types::{PaletteAction, PaletteItem};
        let mut items = vec![
            PaletteItem {
                title: "启动 / 停止代理".into(),
                hint: format!("{:?}", self.proxy_status.state),
                action: PaletteAction::ToggleProxy,
            },
            PaletteItem {
                title: "切换系统代理 / TUN".into(),
                hint: format!("{:?}", self.proxy_config.mode),
                action: PaletteAction::ToggleMode,
            },
            PaletteItem {
                title: "下载内核…".into(),
                hint: "打开内核下载弹框".into(),
                action: PaletteAction::DownloadKernel,
            },
            PaletteItem {
                title: "跳到 · 控制台".into(),
                hint: "代理状态与可用节点".into(),
                action: PaletteAction::Goto(Tab::Proxy),
            },
            PaletteItem {
                title: "跳到 · 节点池".into(),
                hint: "全量节点与测速".into(),
                action: PaletteAction::Goto(Tab::Nodes),
            },
            PaletteItem {
                title: "跳到 · 订阅源".into(),
                hint: "增删改查 · 拉取".into(),
                action: PaletteAction::Goto(Tab::Sources),
            },
            PaletteItem {
                title: "跳到 · 扩展功能".into(),
                hint: "隐私安全与活动节点列表".into(),
                action: PaletteAction::Goto(Tab::Extensions),
            },
            PaletteItem {
                title: "跳到 · 系统设置".into(),
                hint: "行为与自愈".into(),
                action: PaletteAction::Goto(Tab::Settings),
            },
            PaletteItem {
                title: "跳到 · 外部接口".into(),
                hint: "本地 HTTP API".into(),
                action: PaletteAction::Goto(Tab::Api),
            },
            PaletteItem {
                title: "跳到 · 关于".into(),
                hint: "本机版本与检查更新".into(),
                action: PaletteAction::Goto(Tab::About),
            },
        ];
        for (label, panel) in [
            ("监控图表", ProxyPanel::Charts),
            ("可用节点", ProxyPanel::Nodes),
            ("分流策略组", ProxyPanel::Groups),
            ("活跃连接", ProxyPanel::Connections),
            ("分流规则", ProxyPanel::Rules),
            ("DNS / TUN", ProxyPanel::Network),
            ("内核管理", ProxyPanel::Kernels),
            ("运行日志", ProxyPanel::Logs),
        ] {
            items.push(PaletteItem {
                title: format!("跳到 · {label}"),
                hint: "控制台子页".into(),
                action: PaletteAction::GotoProxyPanel(panel),
            });
        }
        for spec in crate::gui::theme::THEMES {
            items.push(PaletteItem {
                title: format!("强调色 · {}", spec.name),
                hint: "整套界面跟着变".into(),
                action: PaletteAction::Theme(spec.id),
            });
        }
        // 节点只列当前"可用节点"页那批（按延迟排好序），最多 40 条 ——
        // 把一万六千条全塞进面板既慢又没人看。
        for node in self.proxy_nodes.iter().take(40) {
            items.push(PaletteItem {
                title: format!("切到节点 · {}", node.name),
                hint: format!(
                    "{} · {}",
                    node.country.clone().unwrap_or_else(|| "—".into()),
                    node.latency_ms
                        .map(|ms| format!("{ms} ms"))
                        .unwrap_or_else(|| "未测".into())
                ),
                action: PaletteAction::UseNode(node.id.clone(), node.name.clone()),
            });
        }
        items
    }

    /// Ctrl+K 命令面板。"敲两个字"比在界面上找按钮快，也是把交互从后台脸里拉出来的一步
    /// （用户 2026-10-07：界面太古板）。
    pub(crate) fn ui_command_palette(&mut self, ctx: &egui::Context) {
        use crate::gui::types::PaletteAction;
        if !self.palette_open {
            return;
        }
        let items = self.palette_items();
        let query = self.palette_query.trim().to_lowercase();
        let matched: Vec<&crate::gui::types::PaletteItem> = items
            .iter()
            .filter(|item| {
                query.is_empty()
                    || item.title.to_lowercase().contains(&query)
                    || item.hint.to_lowercase().contains(&query)
            })
            .collect();
        if self.palette_selected >= matched.len() {
            self.palette_selected = 0;
        }
        let mut run: Option<PaletteAction> = None;
        let mut close = false;
        let want_focus = self.palette_needs_focus;
        let drop_from = ctx.input(|i| i.screen_rect()).height() * 0.18;

        egui::Window::new("命令面板")
            .id(egui::Id::new("vpn-parser-command-palette"))
            .title_bar(false)
            .movable(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, drop_from.max(60.0)])
            .frame(
                egui::Frame::popup(&ctx.style())
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.set_width(480.0);
                // 输入框必须是这一层里的第一个控件、前面没有任何条件 add：
                // 否则敲下第一个字符时后面那批控件的 auto-id 整体位移，TextEdit 换 id →
                // 焦点丢 + 微软拼音组合窗被销毁（就是"输一个字就卡"那个坑）。
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text("输入命令、页面或节点名…")
                        .desired_width(ui.available_width()),
                );
                crate::gui::widgets::note_text_field(ui, &response);
                if want_focus {
                    response.request_focus();
                    self.palette_needs_focus = false;
                }
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                let (down, up, enter, escape) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowDown),
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                if down && !matched.is_empty() {
                    self.palette_selected = (self.palette_selected + 1) % matched.len();
                }
                if up && !matched.is_empty() {
                    self.palette_selected =
                        (self.palette_selected + matched.len() - 1) % matched.len();
                }
                if escape {
                    close = true;
                }
                if matched.is_empty() {
                    ui.label(
                        RichText::new("没有匹配的命令")
                            .small()
                            .color(theme::muted()),
                    );
                }
                egui::ScrollArea::vertical()
                    .id_salt("command_palette_scroll")
                    .max_height(300.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 行是自绘的：把两列文字塞进 Button 的内容里做不到，
                        // 而自绘顺带让"选中"是一整条底色而不是一小块。
                        for (index, item) in matched.iter().enumerate() {
                            let selected_row = index == self.palette_selected;
                            let size = Vec2::new(ui.available_width(), 30.0);
                            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                            let painter = ui.painter_at(rect);
                            if selected_row {
                                painter.rect_filled(
                                    rect,
                                    CornerRadius::same(10),
                                    theme::panel_hover(),
                                );
                            }
                            painter.text(
                                egui::pos2(rect.min.x + 9.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &item.title,
                                egui::FontId::proportional(13.0),
                                theme::text_primary(),
                            );
                            painter.text(
                                egui::pos2(rect.max.x - 9.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                &item.hint,
                                egui::FontId::proportional(11.0),
                                theme::muted(),
                            );
                            if response.clicked() || (enter && selected_row) {
                                run = Some(item.action.clone());
                            }
                        }
                    });
                ui.add_space(2.0);
                ui.label(
                    RichText::new("↑↓ 选择 · Enter 执行 · Esc 关闭 · 再按 Ctrl+K 收起")
                        .small()
                        .color(theme::muted()),
                );
            });
        if close {
            self.palette_open = false;
        }
        if let Some(action) = run {
            self.palette_open = false;
            self.apply_palette_action(action);
        }
        ctx.request_repaint();
    }

    fn apply_palette_action(&mut self, action: crate::gui::types::PaletteAction) {
        use crate::gui::types::PaletteAction;
        match action {
            PaletteAction::Goto(tab) => {
                if tab == Tab::Api {
                    self.tab = Tab::Settings;
                    self.settings_tab = crate::gui::types::SettingsTab::Api;
                } else {
                    self.tab = tab;
                }
            }
            PaletteAction::GotoProxyPanel(panel) => {
                self.tab = Tab::Proxy;
                self.proxy_panel = panel;
            }
            PaletteAction::ToggleProxy => {
                if matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                ) {
                    self.stop_proxy();
                } else {
                    self.start_proxy();
                }
            }
            PaletteAction::ToggleMode => {
                if !self.proxy_status.config_editable() {
                    self.status = "请先停止代理后再切换工作模式".into();
                    return;
                }
                self.proxy_config.mode = match self.proxy_config.mode {
                    ProxyMode::System => ProxyMode::Tun,
                    ProxyMode::Tun => ProxyMode::System,
                };
                self.save_proxy_config();
            }
            PaletteAction::Theme(id) => {
                // 只改设置：`update()` 里那句 `applied_theme_id != settings.ui_theme`
                // 会在下一帧把主题真正刷下去，这里不自己调 theme::apply。
                self.edit_ui_theme = id.to_string();
                self.settings.ui_theme = id.to_string();
                self.save_settings();
            }
            PaletteAction::DownloadKernel => {
                let kind = self
                    .kernels
                    .iter()
                    .find(|kernel| !kernel.available)
                    .or_else(|| self.kernels.first())
                    .map(|kernel| kernel.kind);
                match kind {
                    Some(kind) => self.open_kernel_download(kind),
                    None => {
                        self.tab = Tab::Proxy;
                        self.proxy_panel = ProxyPanel::Kernels;
                        self.status = "没有可下载的内核记录".into();
                    }
                }
            }
            PaletteAction::UseNode(id, name) => {
                self.tab = Tab::Proxy;
                self.proxy_panel = ProxyPanel::Nodes;
                self.switch_proxy_node(id, name, "命令面板");
            }
        }
    }

    /// 打开命令面板。焦点请求要留到面板自己那一帧再做，所以单独一个标志。
    pub(crate) fn open_command_palette(&mut self) {
        self.palette_open = true;
        self.palette_needs_focus = true;
        self.palette_selected = 0;
    }
}

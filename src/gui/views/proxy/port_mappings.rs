use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::widgets::{
    panel_frame, section_title,
};
use crate::gui::VpnApp;
use eframe::egui::{self, CornerRadius, RichText, Stroke, Ui};

impl VpnApp {
    pub(crate) fn ui_proxy_port_mappings(&mut self, ui: &mut Ui) {
        section_title(ui, self.tr("活动节点列表"), "一节点一端口独立转发 · 外部程序调用 · 绕过全局路由");

        egui::ScrollArea::vertical()
            .id_salt("proxy_port_mappings_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                panel_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        icon_label(ui, Icon::Plug, 16.0, theme::accent());
                        ui.label(RichText::new("活动节点列表（一节点一端口独立转发）").strong().size(15.0));
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("为指定节点分配专用的本地监听端口（如 127.0.0.1:10810）。外部应用、爬虫或浏览器插件连接该端口时，流量将独占转发给绑定的节点，完全绕过全局代理路由。")
                            .small()
                            .color(theme::text_secondary()),
                    );
                    ui.add_space(10.0);

            // 顶部添加表单
            ui.horizontal(|ui| {
                ui.label(RichText::new("本地端口:").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_mapping_port)
                        .desired_width(85.0)
                        .hint_text("如 10810"),
                );

                ui.add_space(8.0);
                ui.label(RichText::new("转发目标节点:").strong());

                let (btn_text, btn_color) = if let Some(ref name) = self.new_mapping_node_name {
                    (format!("🎯 {name}"), theme::accent())
                } else {
                    ("➕ 点击选择目标节点...".to_string(), theme::text_secondary())
                };

                if ui
                    .button(RichText::new(btn_text).color(btn_color))
                    .on_hover_text("点击弹窗搜索并选择目标节点")
                    .clicked()
                {
                    self.node_picker_target_index = None;
                    self.node_picker_search.clear();
                    self.node_picker_open = true;
                }

                if self.new_mapping_node_id.is_some() {
                    if ui.button("✕").on_hover_text("清除选中的节点").clicked() {
                        self.new_mapping_node_id = None;
                        self.new_mapping_node_name = None;
                    }
                }

                ui.add_space(8.0);
                let add_btn = ui.add(
                    egui::Button::new(
                        RichText::new("➕ 添加映射")
                            .strong()
                            .size(12.5)
                            .color(theme::accent_strong()),
                    )
                    .fill(theme::accent_bg())
                    .stroke(Stroke::new(1.0_f32, theme::accent().gamma_multiply(0.40)))
                    .corner_radius(CornerRadius::same(8)),
                );
                if add_btn.clicked() {
                    match self.new_mapping_port.trim().parse::<u16>() {
                        Ok(port) => {
                            if port < 1024 {
                                self.status = format!("端口 {port} 属于系统保留端口（须大于等于 1024）");
                            } else if port == self.proxy_config.mixed_port {
                                self.status = format!(
                                    "端口 {port} 与主代理端口 ({}) 冲突",
                                    self.proxy_config.mixed_port
                                );
                            } else if port == self.proxy_config.controller_port {
                                self.status = format!(
                                    "端口 {port} 与控制台端口 ({}) 冲突",
                                    self.proxy_config.controller_port
                                );
                            } else if self.proxy_config.port_mappings.iter().any(|m| m.port == port) {
                                self.status = format!("端口 {port} 已存在映射规则中");
                            } else if self.new_mapping_node_id.is_none() {
                                self.status = "请先点击选择目标节点".to_string();
                            } else {
                                let mapping = crate::proxy::PortMapping {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    port,
                                    node_id: self.new_mapping_node_id.take().unwrap_or_default(),
                                    node_name: self.new_mapping_node_name.take().unwrap_or_default(),
                                    enabled: true,
                                };
                                self.proxy_config.port_mappings.push(mapping);
                                self.new_mapping_port.clear();
                                self.save_proxy_config();
                                self.status = format!("已添加端口 {port} 映射规则并启用");
                            }
                        }
                        Err(_) => {
                            self.status = "请输入有效的端口号（1024 - 65535）".to_string();
                        }
                    }
                }
            });
        });

        ui.add_space(8.0);
        panel_frame().show(ui, |ui| {
            if self.proxy_config.port_mappings.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(30.0);
                    icon_label(ui, Icon::Plug, 28.0, theme::muted());
                    ui.add_space(8.0);
                    ui.label(RichText::new("暂无活动节点映射规则").color(theme::text_secondary()));
                    ui.label(
                        RichText::new("在上方输入本地端口并选择节点，即可为特定应用分配独占出口")
                            .small()
                            .color(theme::muted()),
                    );
                    ui.add_space(30.0);
                });
                return;
            }

            let mut to_delete = None;
            let mut changed = false;
            let mut pick_for_index = None;

            // 表头
            ui.horizontal(|ui| {
                ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new("状态").strong()));
                ui.add_sized(
                    [160.0, 20.0],
                    egui::Label::new(RichText::new("本地监听地址").strong()),
                );
                ui.add_sized(
                    [ui.available_width() - 80.0, 20.0],
                    egui::Label::new(RichText::new("转发目标节点").strong()),
                );
                ui.add_sized([60.0, 20.0], egui::Label::new(RichText::new("操作").strong()));
            });
            ui.separator();

            for (idx, mapping) in self.proxy_config.port_mappings.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    // 启用/停用按钮
                    let status_text = if mapping.enabled { "● 启用" } else { "○ 停用" };
                    let btn_fill = if mapping.enabled {
                        theme::accent_bg()
                    } else {
                        theme::panel_hover()
                    };
                    let btn_color = if mapping.enabled {
                        theme::accent_strong()
                    } else {
                        theme::muted()
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(status_text).size(12.0).color(btn_color),
                            )
                            .fill(btn_fill),
                        )
                        .on_hover_text(if mapping.enabled {
                            "点击停用该端口映射"
                        } else {
                            "点击启用该端口映射"
                        })
                        .clicked()
                    {
                        mapping.enabled = !mapping.enabled;
                        changed = true;
                    }

                    // 本地监听地址 + 复制
                    let addr_str = format!("127.0.0.1:{}", mapping.port);
                    ui.add_sized([160.0, 20.0], |ui: &mut Ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&addr_str).strong());
                            if ui.button("📋").on_hover_text("复制本地代理地址").clicked() {
                                ui.ctx().copy_text(addr_str.clone());
                            }
                        })
                        .response
                    });

                    // 目标节点（点击可更换）
                    let target_width = (ui.available_width() - 70.0).max(100.0);
                    ui.add_sized([target_width, 20.0], |ui: &mut Ui| {
                        let display_name = if mapping.node_name.is_empty() {
                            "（未命名节点）".to_string()
                        } else {
                            mapping.node_name.clone()
                        };
                        let raw_btn_text = format!("🎯 {display_name}");
                        let shown_btn_text = crate::gui::widgets::ellipsize(
                            ui,
                            &raw_btn_text,
                            target_width,
                            egui::TextStyle::Button.resolve(ui.style()),
                        );
                        let btn = ui.add(
                            egui::Button::new(RichText::new(shown_btn_text)),
                        )
                        .on_hover_text(format!("目标节点：{display_name}\n\n点击更换该端口绑定的目标节点"));
                        if btn.clicked() {
                            pick_for_index = Some(idx);
                        }
                        btn
                    });

                    // 操作：删除
                    if ui
                        .button("🗑️")
                        .on_hover_text("删除该端口映射")
                        .clicked()
                    {
                        to_delete = Some(idx);
                    }
                });
                ui.separator();
            }

            if let Some(idx) = pick_for_index {
                self.node_picker_target_index = Some(idx);
                self.node_picker_search.clear();
                self.node_picker_open = true;
            }
            if let Some(idx) = to_delete {
                self.proxy_config.port_mappings.remove(idx);
                changed = true;
            }
            if changed {
                self.save_proxy_config();
            }
        });
        ui.add_space(16.0);
    });
}

}

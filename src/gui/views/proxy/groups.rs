use super::CLASH_VERGE_RECOMMENDED_OVERRIDE;
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    GroupsTab, KernelGroup, OverrideEditorTab,
};
use crate::gui::widgets::{
    card_frame, macos_segmented_tabs, primary_btn,
};
use crate::gui::VpnApp;
use crate::models::NodeSummary;
use crate::proxy::ProxyState;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

fn override_text_editor(
    ui: &mut Ui,
    text: &mut String,
    editable: bool,
    hint: &str,
) -> egui::Response {
    ui.add_enabled(
        editable,
        egui::TextEdit::multiline(text)
            // A focused egui TextEdit otherwise keeps processing keyboard input
            // for one frame after its containing UI becomes disabled.
            .interactive(editable)
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .desired_rows(22)
            .hint_text(hint),
    )
}

#[cfg(test)]
mod editor_tests {
    use super::*;

    fn editor_frame(
        ctx: &egui::Context,
        text: &mut String,
        editable: bool,
        events: Vec<egui::Event>,
    ) -> egui::Id {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut editor_id = None;
        crate::gui::run_test_ui(ctx, input, |ui| {
            let response = override_text_editor(ui, text, editable, "");
            assert_eq!(response.enabled(), editable);
            editor_id = Some(response.id);
        });
        editor_id.unwrap()
    }

    #[test]
    fn saving_override_editor_rejects_input_and_accepts_it_after_save() {
        for original in ["dns:\n  enable: true", "fn main(config) { config }"] {
            let ctx = egui::Context::default();
            let mut text = original.to_owned();
            let editor = editor_frame(&ctx, &mut text, true, Vec::new());
            ctx.memory_mut(|memory| memory.request_focus(editor));
            editor_frame(
                &ctx,
                &mut text,
                false,
                vec![egui::Event::Text("during-save".into())],
            );
            assert_eq!(text, original, "saving must not accept edits that a response would erase");

            let editor = editor_frame(&ctx, &mut text, true, Vec::new());
            ctx.memory_mut(|memory| memory.request_focus(editor));
            editor_frame(
                &ctx,
                &mut text,
                true,
                vec![egui::Event::Text("after-save".into())],
            );
            assert!(text.contains("after-save"), "the editor must become editable again");
        }
    }
}

impl VpnApp {
    pub(crate) fn ui_proxy_groups(&mut self, ui: &mut Ui) {
        ui.set_width(ui.available_width());

        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.groups_tab == GroupsTab::View, "视图")
                .clicked()
            {
                self.groups_tab = GroupsTab::View;
            }
            if ui
                .selectable_label(self.groups_tab == GroupsTab::Config, "配置")
                .clicked()
            {
                self.groups_tab = GroupsTab::Config;
            }
        });
        ui.add_space(6.0);

        match self.groups_tab {
            GroupsTab::View => {
                let mut toggle_group: Option<(String, bool)> = None;
                let mut switch_action: Option<(String, String)> = None;

                egui::ScrollArea::vertical()
                    .id_salt("proxy_groups_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // 1. 顶部智能分流路由拓扑指示卡
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(12))
                            .inner_margin(Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Shield, 15.0, theme::accent());
                            ui.label(
                                RichText::new("智能分流路由拓扑（场景自适应调度）")
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .checkbox(&mut self.proxy_config.smart_routing, "启用内置智能策略组")
                                    .on_hover_text("自动创建 AI分组、国际流媒体、Telegram、地区测速组（香港/日本/新加坡/美国）与多线并发下载策略组")
                                    .changed()
                                {
                                    self.save_proxy_config();
                                }
                            });
                        });

                        ui.add_space(8.0);

                        // 4 路由拓扑晶体微卡片
                        let routes = [
                            ("🤖 AI 分组", "专线原生 IP", "ChatGPT · Claude · Grok", theme::accent()),
                            ("🎬 国际流媒体", "低延迟解锁", "YouTube · Netflix · Disney+", theme::accent_strong()),
                            ("🌏 地区自动选优", "URL-Test", "香港 · 日本 · 新加坡 · 美国", theme::accent_strong()),
                            ("⚡ 并发加速", "Load-Balance", "多路并发 · 迅雷 · 网盘下载", theme::text_secondary()),
                        ];

                        let render_route_card = |ui: &mut Ui, (title, badge, desc, col): &(&str, &str, &str, egui::Color32)| {
                            Frame::NONE
                                .fill(theme::sidebar())
                                .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                .corner_radius(CornerRadius::same(9))
                                .inner_margin(Margin::symmetric(10, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(*title).strong().size(12.5).color(theme::text_primary()));
                                    });
                                    ui.add_space(3.0);
                                    Frame::NONE
                                        .fill(col.gamma_multiply(0.15))
                                        .corner_radius(CornerRadius::same(4))
                                        .inner_margin(Margin::symmetric(5, 1))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(*badge).small().strong().color(*col));
                                        });
                                    ui.add_space(3.0);
                                    ui.label(RichText::new(*desc).small().color(theme::muted()));
                                });
                        };

                        if ui.available_width() < 600.0 {
                            ui.columns(2, |cols| {
                                render_route_card(&mut cols[0], &routes[0]);
                                render_route_card(&mut cols[1], &routes[1]);
                            });
                            ui.add_space(6.0);
                            ui.columns(2, |cols| {
                                render_route_card(&mut cols[0], &routes[2]);
                                render_route_card(&mut cols[1], &routes[3]);
                            });
                        } else {
                            ui.columns(4, |cols| {
                                for (i, route) in routes.iter().enumerate() {
                                    render_route_card(&mut cols[i], route);
                                }
                            });
                        }
                    });

                ui.add_space(8.0);

                // 2. 策略组列表
                let is_running = matches!(
                    self.proxy_status.state,
                    ProxyState::Running | ProxyState::Degraded
                );

                if !is_running {
                    card_frame().show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new("代理核心未运行")
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new("启动内核后，此处将实时展示各策略组的活跃出口、测速延迟并支持直接切换节点。")
                                    .small()
                                    .color(theme::muted()),
                            );
                            ui.add_space(10.0);
                        });
                    });
                } else if self.kernel_groups.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(
                                RichText::new("正在从代理内核同步策略组状态（每 5 秒自动轮询）...")
                                    .small()
                                    .color(theme::text_secondary()),
                            );
                        });
                    });
                } else {
                    for group in &self.kernel_groups {
                                let is_expanded = self.expanded_kernel_groups.contains(&group.name);
                                Frame::NONE
                                    .fill(theme::panel())
                                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                                    .corner_radius(CornerRadius::same(10))
                                    .inner_margin(Margin::symmetric(14, 10))
                                    .show(ui, |ui| {
                                        let mut toggle_clicked = false;
                                        ui.horizontal(|ui| {
                                            let group_icon = match group.name.as_str() {
                                                "AI分组" => Icon::Lightning,
                                                "国际流媒体" => Icon::Play,
                                                "Telegram" => Icon::Chat,
                                                "香港节点" | "日本节点" | "新加坡节点" | "美国节点" | "台湾节点" => Icon::Globe,
                                                "多线并发下载" => Icon::ArrowDown,
                                                "普通代理" => Icon::Rocket,
                                                _ => Icon::Radar,
                                            };
                                            icon_label(ui, group_icon, 14.0, theme::accent());
                                            ui.label(
                                                RichText::new(&group.name)
                                                    .strong()
                                                    .color(theme::text_primary()),
                                            );

                                            let type_pill = match group.kind.to_lowercase().as_str() {
                                                "url-test" | "urltest" => "自动优选 (url-test)",
                                                "load-balance" | "loadbalance" => "负载均衡 (load-balance)",
                                                _ => "手动选择 (select)",
                                            };
                                            ui.label(
                                                RichText::new(type_pill)
                                                    .small()
                                                    .color(theme::muted()),
                                            );

                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    // 展开/收起 胶囊按钮（默认折叠，点击展开）
                                                    let toggle_label = if is_expanded {
                                                        format!("▼ 收起接口 ({} 个)", group.members.len())
                                                    } else {
                                                        format!("▶ 展开接口 ({} 个)", group.members.len())
                                                    };
                                                    let toggle_btn = egui::Button::new(
                                                        RichText::new(toggle_label)
                                                            .small()
                                                            .strong()
                                                            .color(if is_expanded {
                                                                theme::accent_strong()
                                                            } else {
                                                                theme::text_secondary()
                                                            }),
                                                    )
                                                    .fill(if is_expanded {
                                                        theme::accent_bg()
                                                    } else {
                                                        theme::sidebar()
                                                    })
                                                    .stroke(Stroke::new(
                                                        1.0_f32,
                                                        if is_expanded {
                                                            theme::accent()
                                                        } else {
                                                            theme::border_subtle()
                                                        },
                                                    ))
                                                    .corner_radius(CornerRadius::same(8));

                                                    if ui.add(toggle_btn)
                                                        .on_hover_text(if is_expanded { "点击收起折叠该分组下的所有节点接口" } else { "点击展开查看该分组下的所有节点接口并可点选切换" })
                                                        .clicked()
                                                    {
                                                        toggle_clicked = true;
                                                    }

                                                    if let Some(now) = &group.now {
                                                        ui.separator();
                                                        ui.label(
                                                            RichText::new(format!("当前出口: {now}"))
                                                                .small()
                                                                .strong()
                                                                .color(theme::accent_strong()),
                                                        );
                                                    }
                                                },
                                            );
                                        });

                                        if toggle_clicked {
                                            toggle_group = Some((group.name.clone(), !is_expanded));
                                        }

                                        // 仅当展开时才展示该策略组下的所有接口/节点胶囊
                                        if is_expanded {
                                            ui.add_space(8.0);
                                            ui.horizontal_wrapped(|ui| {
                                                let is_selector = matches!(
                                                    group.kind.to_lowercase().as_str(),
                                                    "select" | "selector"
                                                );
                                                for member in &group.members {
                                                    let is_active = group.now.as_deref() == Some(member.as_str());
                                                    let text_color = if is_active {
                                                        theme::accent_strong()
                                                    } else {
                                                        theme::text_secondary()
                                                    };
                                                    let btn = ui.add(
                                                        egui::Button::new(
                                                            RichText::new(member)
                                                                .small()
                                                                .strong()
                                                                .color(text_color),
                                                        )
                                                        .fill(if is_active {
                                                            theme::accent_bg()
                                                        } else {
                                                            theme::page_bg()
                                                        })
                                                        .stroke(Stroke::new(
                                                            1.0_f32,
                                                            if is_active {
                                                                theme::accent()
                                                            } else {
                                                                theme::border_subtle()
                                                            },
                                                        ))
                                                        .corner_radius(CornerRadius::same(6)),
                                                    );
                                                    if is_selector && btn.clicked() && !is_active {
                                                        switch_action = Some((
                                                            group.name.clone(),
                                                            member.clone(),
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    });
                                ui.add_space(6.0);
                            }
                        }
                        ui.add_space(16.0);
                    });

                if let Some((name, expand)) = toggle_group {
                    if expand {
                        self.expanded_kernel_groups.insert(name);
                    } else {
                        self.expanded_kernel_groups.remove(&name);
                    }
                }
                if let Some((grp, mbr)) = switch_action {
                    self.select_kernel_member(grp, mbr);
                }
            }
            GroupsTab::Config => {
                // 顶部：模式分段切换器 (YAML 声明式覆写 / Rhai 脚本动态覆写)
                ui.horizontal(|ui| {
                    let tabs = [
                        (
                            OverrideEditorTab::Yaml,
                            Icon::Terminal,
                            "YAML 声明式覆写 (Clash Party)",
                        ),
                        (
                            OverrideEditorTab::Rhai,
                            Icon::Lightning,
                            "Rhai 脚本动态覆写 (图灵完备)",
                        ),
                        (
                            OverrideEditorTab::VpDsl,
                            Icon::Rocket,
                            "VP-Core 原生 DSL (.vp 声明式)",
                        ),
                    ];
                    macos_segmented_tabs(ui, &mut self.override_editor_tab, &tabs);
                });
                ui.add_space(8.0);

                match self.override_editor_tab {
                    OverrideEditorTab::Yaml => {
                        // 自定义配置覆写 (YAML 深度合并)：顶部固定标题与操作组，仅编辑区独立垂直滚动
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                let mut save_clicked = false;
                                let mut load_recommended = false;
                                let mut clear_override = false;

                                // 固定顶部操作栏（固定不滚动）
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Terminal, 14.0, theme::text_secondary());
                                    ui.label(
                                        RichText::new("自定义配置覆写 (Clash Party / Verge 规范 YAML 深度合并)")
                                            .strong()
                                            .color(theme::text_primary()),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui
                                            .add_enabled(!self.proxy_busy, primary_btn("保存配置"))
                                            .on_hover_text("保存 YAML 覆写配置；支持热重载时立即应用")
                                            .clicked()
                                        {
                                            save_clicked = true;
                                        }
                                        if ui
                                            .add_enabled(!self.proxy_busy, egui::Button::new("⚡ 一键载入推荐规则"))
                                            .on_hover_text("填入经典的 AI、Google、多线下载与国内直连分流配置")
                                            .clicked()
                                        {
                                            load_recommended = true;
                                        }
                                        if ui.add_enabled(!self.proxy_busy, egui::Button::new("清空覆写")).clicked() {
                                            clear_override = true;
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(
                                        "遵循 Clash Party 规范对内核配置进行深度覆写。支持 +key 前插、key+ 追加、key 全量替换、! 强制覆盖与 <> 转义（如 +rules / +proxy-groups / dns / tun），保存后由内核应用结果确认是否生效："
                                    )
                                    .small()
                                    .color(theme::muted()),
                                );
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                // 仅让编辑文本框内部区域支持大文本纵向平滑滚动
                                let mut text = self.proxy_config.override_yaml.clone().unwrap_or_default();
                                let max_editor_height = (ui.available_height() - 8.0).max(220.0);

                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(max_editor_height)
                                    .id_salt("proxy_override_yaml_editor_scroll")
                                    .show(ui, |ui| {
                                        let editor = override_text_editor(
                                            ui,
                                            &mut text,
                                            !self.proxy_busy,
                                            "# 支持 Clash Party 规范 YAML 深度合并与修饰符\n# 示例 1: 前置插入高优先级规则\n+rules:\n  - DOMAIN-SUFFIX,openai.com,AI分组\n# 示例 2: 递归合并修改 DNS 配置\ndns:\n  +nameserver:\n    - 1.1.1.1\n# 示例 3: 强制覆盖整个配置块\ntun!:\n  enable: false",
                                        );
                                        if editor.changed() {
                                            self.proxy_config.override_yaml = if text.trim().is_empty() {
                                                None
                                            } else {
                                                Some(text.clone())
                                            };
                                        }
                                    });

                                if load_recommended {
                                    self.proxy_config.override_yaml =
                                        Some(CLASH_VERGE_RECOMMENDED_OVERRIDE.to_string());
                                    self.save_proxy_config();
                                }
                                if clear_override {
                                    self.proxy_config.override_yaml = None;
                                    self.save_proxy_config();
                                }
                                if save_clicked {
                                    self.proxy_config.override_yaml = if text.trim().is_empty() {
                                        None
                                    } else {
                                        Some(text)
                                    };
                                    self.save_proxy_config();
                                }
                            });
                    }
                    OverrideEditorTab::Rhai => {
                        // 自定义 Rhai 脚本动态覆写 (图灵完备编程扩展)
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                let mut save_clicked = false;
                                let mut validate_clicked = false;
                                let mut load_template = false;
                                let mut clear_script = false;

                                // 固定顶部操作栏
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Lightning, 14.0, theme::accent());
                                    ui.label(
                                        RichText::new("Rhai 脚本动态覆写 (图灵完备配置扩展)")
                                            .strong()
                                            .color(theme::text_primary()),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui
                                            .add_enabled(!self.proxy_busy, primary_btn("保存脚本"))
                                            .on_hover_text("校验并保存 Rhai 脚本；支持热重载时立即应用")
                                            .clicked()
                                        {
                                            save_clicked = true;
                                        }
                                        if ui
                                            .button("🔍 语法校验")
                                            .on_hover_text("在本地纯 Rust 安全沙箱中编译校验语法正确性")
                                            .clicked()
                                        {
                                            validate_clicked = true;
                                        }
                                        if ui
                                            .add_enabled(!self.proxy_busy, egui::Button::new("⚡ 载入模板"))
                                            .on_hover_text("填入经典的主流脚本模板 (DNS增强、规则插入与动态节点过滤)")
                                            .clicked()
                                        {
                                            load_template = true;
                                        }
                                        if ui.add_enabled(!self.proxy_busy, egui::Button::new("清空脚本")).clicked() {
                                            clear_script = true;
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(
                                        "在内核配置编译后执行用户自定义 Rhai 脚本（作为最终裁决者）。支持 fn main(config) 或扁平语句，内置 100,000 步操作数熔断防死循环，支持动态过滤节点、插入自定义规则与重写 DNS："
                                    )
                                    .small()
                                    .color(theme::muted()),
                                );

                                // 语法校验反馈横幅
                                if let Some((ref msg, is_err)) = self.rhai_syntax_status {
                                    ui.add_space(6.0);
                                    let (fill, stroke, col, icon) = if is_err {
                                        (
                                            theme::danger_bg(),
                                            theme::danger(),
                                            theme::danger(),
                                            Icon::Alert,
                                        )
                                    } else {
                                        (
                                            theme::success_bg(),
                                            theme::success(),
                                            theme::success(),
                                            Icon::Check,
                                        )
                                    };
                                    Frame::NONE
                                        .fill(fill)
                                        .stroke(Stroke::new(1.0_f32, stroke))
                                        .corner_radius(CornerRadius::same(6))
                                        .inner_margin(Margin::symmetric(10, 6))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                icon_label(ui, icon, 13.0, col);
                                                ui.label(RichText::new(msg).small().strong().color(col));
                                            });
                                        });
                                }

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                let mut script_text = self.proxy_config.override_script.clone().unwrap_or_default();
                                let max_editor_height = (ui.available_height() - 8.0).max(220.0);

                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(max_editor_height)
                                    .id_salt("proxy_override_rhai_editor_scroll")
                                    .show(ui, |ui| {
                                        let editor = override_text_editor(
                                            ui,
                                            &mut script_text,
                                            !self.proxy_busy,
                                            "// Rhai 动态配置覆写脚本\n// 纯 Rust 沙箱执行，自带 100,000 步指令防死循环看门狗\nfn main(config) {\n    // 1. 修改 DNS 增强模式\n    config.dns.enhanced_mode = \"fake-ip\";\n\n    // 2. 规则最前排插入高优先级规则\n    config.rules.insert(0, \"DOMAIN-SUFFIX,openai.com,AI分组\");\n\n    // 3. 编程式过滤节点\n    let clean = [];\n    for p in config.proxies {\n        if !p.name.contains(\"2x\") {\n            clean.push(p);\n        }\n    }\n    if clean.len() > 0 {\n        config.proxies = clean;\n    }\n\n    return config;\n}",
                                        );
                                        if editor.changed() {
                                            self.proxy_config.override_script = if script_text.trim().is_empty() {
                                                None
                                            } else {
                                                Some(script_text.clone())
                                            };
                                            self.rhai_syntax_status = None;
                                        }
                                    });

                                if validate_clicked {
                                    match crate::proxy::script_override::validate_script_syntax(&script_text) {
                                        Ok(()) => {
                                            self.rhai_syntax_status = Some((
                                                "✅ Rhai 脚本语法校验通过 (安全合规，可正常执行)".to_string(),
                                                false,
                                            ));
                                        }
                                        Err(e) => {
                                            self.rhai_syntax_status = Some((format!("❌ {e}"), true));
                                        }
                                    }
                                }
                                if load_template {
                                    self.proxy_config.override_script = Some(
                                        crate::proxy::script_override::RHAI_RECOMMENDED_SCRIPT_TEMPLATE.to_string(),
                                    );
                                    self.rhai_syntax_status = Some((
                                        "已载入经典 Rhai 脚本模板".to_string(),
                                        false,
                                    ));
                                    self.save_proxy_config();
                                }
                                if clear_script {
                                    self.proxy_config.override_script = None;
                                    self.rhai_syntax_status = None;
                                    self.save_proxy_config();
                                }
                                if save_clicked {
                                    self.proxy_config.override_script = if script_text.trim().is_empty() {
                                        None
                                    } else {
                                        Some(script_text.clone())
                                    };
                                    if let Err(e) = crate::proxy::script_override::validate_script_syntax(&script_text) {
                                        self.rhai_syntax_status = Some((
                                            format!("未保存，脚本存在语法错误: {e}"),
                                            true,
                                        ));
                                    } else {
                                        self.rhai_syntax_status = Some((
                                            "语法校验通过（保存结果见状态栏）".to_string(),
                                            false,
                                        ));
                                        self.save_proxy_config();
                                    }
                                }
                            });
                    }
                    OverrideEditorTab::VpDsl => {
                        // VP-Core 原生 DSL 声明式配置查看与导出
                        Frame::NONE
                            .fill(theme::panel())
                            .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(14, 12))
                            .show(ui, |ui| {
                                let mut gen_clicked = false;
                                let mut copy_clicked = false;

                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Rocket, 14.0, theme::accent_strong());
                                    ui.label(
                                        RichText::new("VP-Core 原生 DSL (.vp 声明式极简语法与 CEL 规则)")
                                            .strong()
                                            .color(theme::text_primary()),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui
                                            .button("📋 复制 DSL")
                                            .on_hover_text("复制当前 .vp DSL 脚本到剪贴板")
                                            .clicked()
                                        {
                                            copy_clicked = true;
                                        }
                                        if ui
                                            .add_enabled(!self.proxy_busy, primary_btn("⚡ 生成当前 DSL 预览"))
                                            .on_hover_text("基于当前分流策略与选中的代理节点即时编译生成原生 .vp DSL 配置")
                                            .clicked()
                                        {
                                            gen_clicked = true;
                                        }
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(
                                        "VP-Core 原生配置文件采用 vp-dsl 声明式语法，无缩进对齐负担，原生支持策略组 fallback: \"DIRECT\" 熔断防断网与 Google CEL 复合条件表达式（AND / OR / NOT 零开销比对）："
                                    )
                                    .small()
                                    .color(theme::muted()),
                                );

                                if gen_clicked {
                                    self.generate_vp_dsl_preview();
                                }
                                if copy_clicked {
                                    if let Some(ref text) = self.vp_dsl_preview {
                                        ui.ctx().copy_text(text.clone());
                                        self.status = "已复制 VP-Core 原生 DSL 到剪贴板".into();
                                    } else {
                                        self.status = "请先点击「生成当前 DSL 预览」".into();
                                    }
                                }

                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                if self.vp_dsl_preview.is_none() {
                                    self.generate_vp_dsl_preview();
                                }

                                let mut dsl_text = self.vp_dsl_preview.clone().unwrap_or_else(|| "// 正在生成 VP-Core DSL 预览...".to_string());
                                let max_editor_height = (ui.available_height() - 8.0).max(220.0);

                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_height(max_editor_height)
                                    .id_salt("proxy_vp_dsl_viewer_scroll")
                                    .show(ui, |ui| {
                                        let _ = override_text_editor(
                                            ui,
                                            &mut dsl_text,
                                            false,
                                            "// VP-Core 原生 DSL 预览...",
                                        );
                                    });
                            });
                    }
                }
            }
        }
    }

}

pub fn is_universal_proxy_group(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        name,
        "全部"
            | "PROXY"
            | "PROXIES"
            | "自动选择"
            | "自动选路"
            | "故障转移"
            | "负载均衡"
            | "节点选择"
            | "漏网之鱼"
            | "兜底分流"
            | "全球直连"
            | "直连"
            | "多线并发下载"
            | "下载"
            | "普通代理"
    ) || lower == "all"
        || lower == "auto"
        || lower == "fallback"
        || lower == "load-balance"
        || lower == "loadbalance"
        || lower == "final"
        || lower == "match"
        || lower == "direct"
        || lower.contains("自动")
        || lower.contains("故障")
        || lower.contains("负载")
        || lower.contains("漏网")
        || lower.contains("直连")
        || lower.contains("节点选择")
}

pub fn node_matches_group(
    node: &NodeSummary,
    group_name: &str,
    kernel_groups: &[KernelGroup],
) -> bool {
    node_matches_group_depth(node, group_name, kernel_groups, 0)
}

fn node_matches_group_depth(
    node: &NodeSummary,
    group_name: &str,
    kernel_groups: &[KernelGroup],
    depth: usize,
) -> bool {
    if depth > 4 {
        return false;
    }
    if group_name == "全部" || group_name == "PROXY" || group_name.eq_ignore_ascii_case("all") {
        return true;
    }

    // 1. 若内核正在运行且上报了该策略组成员列表：优先按真实内核成员匹配（最权威真实）
    if let Some(group) = kernel_groups.iter().find(|g| g.name == group_name) {
        if !group.members.is_empty() {
            if group.members.iter().any(|m| m == &node.name || m == &node.server) {
                return true;
            }
            if group.members.iter().any(|m| m == "PROXY" || m == "全部" || m.eq_ignore_ascii_case("all")) {
                return true;
            }
            // 嵌套策略组展开（例如 AI分组 的 members 包含 "美国节点"）
            let sub_matched = group.members.iter().any(|m| {
                if m != group_name && m != "DIRECT" && m != "REJECT" && m != "PASS" {
                    node_matches_group_depth(node, m, kernel_groups, depth + 1)
                } else {
                    false
                }
            });
            if sub_matched {
                return true;
            }

            // 对于通用功能性策略组（如 自动选择、故障转移、负载均衡、全球直连、漏网之鱼），
            // 如果成员里没有具体配置节点（例如只填了 DIRECT、REJECT 或不存在的组），为保障用户体验兜底展示全部节点
            if is_universal_proxy_group(group_name) {
                let has_actual_node = group.members.iter().any(|m| {
                    m != "DIRECT" && m != "REJECT" && m != "PASS"
                });
                if !has_actual_node {
                    return true;
                }
            }

            return false;
        }
    }

    // 2. 内核无明确成员时的通用语义与功能性组推断
    if is_universal_proxy_group(group_name) {
        return true;
    }

    let country = node
        .country
        .clone()
        .or_else(|| crate::parser::guess_country(&node.name))
        .unwrap_or_default()
        .to_ascii_uppercase();
    let lower_name = node.name.to_lowercase();

    // 2. 针对内置知名策略组做精准语义过滤（无内核明确成员时的智能推断）
    match group_name {
        "香港节点" | "HK" => {
            return country == "HK"
                || lower_name.contains("hk")
                || lower_name.contains("hong")
                || node.name.contains("港");
        }
        "日本节点" | "JP" => {
            return country == "JP"
                || lower_name.contains("jp")
                || lower_name.contains("japan")
                || lower_name.contains("tokyo")
                || lower_name.contains("osaka")
                || node.name.contains("日");
        }
        "新加坡节点" | "SG" => {
            return country == "SG"
                || lower_name.contains("sg")
                || lower_name.contains("singapore")
                || node.name.contains("新")
                || node.name.contains("狮城");
        }
        "台湾节点" | "TW" => {
            return country == "TW"
                || lower_name.contains("tw")
                || lower_name.contains("taiwan")
                || node.name.contains("台")
                || node.name.contains("湾");
        }
        "美国节点" | "US" => {
            return (country == "US"
                || lower_name.contains("us")
                || lower_name.contains("united states")
                || node.name.contains("美"))
                && !lower_name.contains("warp");
        }
        "韩国节点" | "KR" => {
            return country == "KR"
                || lower_name.contains("kr")
                || lower_name.contains("korea")
                || node.name.contains("韩");
        }
        "英国节点" | "GB" | "UK" => {
            return country == "GB"
                || country == "UK"
                || lower_name.contains("gb")
                || lower_name.contains("uk")
                || node.name.contains("英");
        }
        "德国节点" | "DE" => {
            return country == "DE"
                || lower_name.contains("de")
                || lower_name.contains("germany")
                || node.name.contains("德");
        }
        "AI分组" | "AI" => {
            return matches!(country.as_str(), "US" | "JP" | "SG" | "TW" | "KR" | "GB" | "DE")
                || lower_name.contains("ai")
                || lower_name.contains("openai")
                || lower_name.contains("chatgpt")
                || lower_name.contains("claude")
                || lower_name.contains("grok")
                || country == "US";
        }
        "国际流媒体" | "流媒体" => {
            return matches!(country.as_str(), "HK" | "JP" | "SG" | "TW" | "US")
                || lower_name.contains("netflix")
                || lower_name.contains("youtube")
                || lower_name.contains("disney")
                || lower_name.contains("spotify");
        }
        "Telegram" | "TG" => {
            return matches!(country.as_str(), "SG" | "HK" | "JP" | "US")
                || lower_name.contains("telegram")
                || lower_name.contains("tg");
        }
        "多线并发下载" | "下载" | "普通代理" | "自动选择" | "自动选路" | "故障转移" | "负载均衡"
        | "节点选择" | "漏网之鱼" | "兜底分流" | "全球直连" | "直连" => {
            return true;
        }
        _ => {}
    }

    let lower_group = group_name.to_lowercase();
    lower_name.contains(&lower_group) || country.to_lowercase() == lower_group
}

/// 内置智能策略组的名字。它们**只有**在 `smart_routing` 开着时才会被写进生成的配置
/// （`proxy` 侧那几处 `if options.smart_routing`），所以胶囊条也照这个判在不在。
const SMART_ROUTING_GROUPS: [&str; 9] = [
    "AI分组",
    "国际流媒体",
    "Telegram",
    "香港节点",
    "日本节点",
    "新加坡节点",
    "美国节点",
    "多线并发下载",
    "普通代理",
];

/// 自定义覆写 YAML 里真的定义了哪些组。两种写法都认（整份文档带 `proxy-groups:`，
/// 或者直接给一个组数组 —— 与 `proxy` 侧应用覆写时的分叉一致）。解析不动就当没有。
pub(crate) fn override_group_names(override_yaml: Option<&str>) -> Vec<String> {
    let Some(text) = override_yaml.filter(|raw| !raw.trim().is_empty()) else {
        return Vec::new();
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Vec::new();
    };
    let groups = value
        .get("+proxy-groups")
        .or_else(|| value.get("proxy-groups+"))
        .or_else(|| value.get("proxy-groups"))
        .or_else(|| value.get("<proxy-groups>"))
        .and_then(serde_yaml::Value::as_sequence)
        .or_else(|| value.as_sequence())
        .cloned()
        .unwrap_or_default();
    groups
        .iter()
        .filter_map(|group| group.get("name").and_then(|name| name.as_str()))
        .map(str::to_string)
        .collect()
}

/// 策略组胶囊条该显示谁：**默认只有「全部」**，其余的组只有"配置里真的有它"才出现。
/// 三个来源 —— ① `smart_routing` 打开的内置四组；② 覆写 YAML 里定义的组；
/// ③ 内核上报且成员非空的组。
///
/// 注意这里刻意**不**用"能不能筛出节点"当判据：内置的 `普通代理`/`多线并发下载` 走的是
/// 语义兜底匹配，任何一台机器上都能匹配到全部节点，那样等于永远显示。
pub(crate) fn visible_group_names(
    smart_routing: bool,
    override_yaml: Option<&str>,
    sub_groups_yaml: Option<&str>,
    kernel_groups: &[KernelGroup],
) -> Vec<String> {
    let mut names: Vec<String> = vec!["全部".to_string()];
    let mut defined = override_group_names(override_yaml);
    for g in override_group_names(sub_groups_yaml) {
        if !defined.contains(&g) {
            defined.push(g);
        }
    }
    let reported = |name: &str| {
        kernel_groups
            .iter()
            .any(|group| group.name == name && !group.members.is_empty())
    };

    for name in SMART_ROUTING_GROUPS {
        if smart_routing || defined.iter().any(|d| d == name) || reported(name) {
            names.push(name.to_string());
        }
    }
    for name in &defined {
        if !names.iter().any(|shown| shown == name) {
            names.push(name.clone());
        }
    }
    for group in kernel_groups {
        if group.members.is_empty()
            || matches!(group.name.as_str(), "GLOBAL" | "DIRECT" | "REJECT" | "PASS")
            || names.iter().any(|shown| *shown == group.name)
        {
            continue;
        }
        names.push(group.name.clone());
    }
    names
}

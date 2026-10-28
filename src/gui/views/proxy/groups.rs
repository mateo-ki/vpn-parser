use super::CLASH_VERGE_RECOMMENDED_OVERRIDE;
use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{
    GroupsTab, KernelGroup,
};
use crate::gui::widgets::{
    card_frame, primary_btn,
};
use crate::gui::VpnApp;
use crate::models::NodeSummary;
use crate::proxy::ProxyState;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke, Ui};

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
                                RichText::new("启动代理核心后，此处将实时展示各策略组的活跃出口、测速延迟并支持直接切换节点。")
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
                // 自定义配置覆写 (YAML & Clash 脚本片段)：顶部固定标题与操作组，仅编辑区独立垂直滚动
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
                                RichText::new("自定义配置覆写 (YAML & Clash 脚本片段)")
                                    .strong()
                                    .color(theme::text_primary()),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add_enabled(!self.proxy_busy, primary_btn("保存配置"))
                                    .on_hover_text("立即保存 YAML 覆写配置并热重载生效")
                                    .clicked()
                                {
                                    save_clicked = true;
                                }
                                if ui
                                    .button("⚡ 一键载入推荐规则")
                                    .on_hover_text("填入经典的 AI、Google、多线下载与国内直连分流配置")
                                    .clicked()
                                {
                                    load_recommended = true;
                                }
                                if ui.button("清空覆写").clicked() {
                                    clear_override = true;
                                }
                            });
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "支持直接在此处粘贴 Clash Verge Rev 风格的 YAML 配置覆写（rules / proxy-groups 等），保存后将自动合并生效："
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
                                let editor = ui.add(
                                    egui::TextEdit::multiline(&mut text)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(22)
                                        .hint_text("# 可在此处粘贴自定义 rules 或 proxy-groups YAML 片段\nrules:\n  - DOMAIN-SUFFIX,openai.com,AI分组\n  - PROCESS-NAME,IDMan.exe,多线并发下载"),
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
                            self.status = "已保存自定义分流覆写配置并热重载".into();
                        }
                    });
            }
        }
    }

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
            // 嵌套策略组展开（例如 AI分组 的 members 包含 "美国节点"）
            let sub_matched = group.members.iter().any(|m| {
                if m != group_name && m != "DIRECT" && m != "REJECT" && m != "PROXY" && m != "PASS" {
                    node_matches_group_depth(node, m, kernel_groups, depth + 1)
                } else {
                    false
                }
            });
            return sub_matched;
        }
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
        "多线并发下载" | "下载" | "普通代理" => {
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
        .get("proxy-groups")
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

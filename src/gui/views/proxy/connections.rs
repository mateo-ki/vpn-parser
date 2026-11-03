use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::types::{ActiveConnectionInfo, ConnectionsTab, DnsRecord, QuickRuleModalState, QuickRuleType};
use crate::gui::widgets::{
    card_frame, danger_btn, format_bytes, header_cell,
    macos_segmented_tabs, panel_frame, primary_btn,
};
use crate::gui::VpnApp;
use crate::proxy::{DnsEnhancedMode, ProxyState};
use eframe::egui::{self, Color32, CornerRadius, Frame, Margin, RichText, Stroke, Ui, Vec2};

/// 判断一条规则是否属于兜底规则 (MATCH / FINAL) - 零堆内存分配
fn is_unmatched_rule(rule: &str) -> bool {
    let r = rule.trim();
    if r.is_empty() {
        return true;
    }
    let b = r.as_bytes();
    if b.len() >= 5 && (b[..5].eq_ignore_ascii_case(b"MATCH") || b[..5].eq_ignore_ascii_case(b"FINAL")) {
        return true;
    }
    false
}

/// 零堆内存分配的大小写无关子串搜索（针对 ASCII 路径执行零拷贝字节滑动窗口，非 ASCII 自动平滑回退）
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        let n_bytes = needle.as_bytes();
        let h_bytes = haystack.as_bytes();
        return h_bytes.windows(n_bytes.len()).any(|w| w.eq_ignore_ascii_case(n_bytes));
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// 智能推导适合 DOMAIN-SUFFIX 的主域名
fn infer_domain_suffix(host: &str) -> String {
    let host = host.trim();
    let parts: Vec<&str> = host.split('.').collect();
    // 纯 IP 地址直接原样返回
    if parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
        return host.to_string();
    }
    if parts.len() >= 2 {
        if parts.len() >= 3 && matches!(parts[parts.len() - 2], "com" | "edu" | "gov" | "org" | "net") {
            format!("{}.{}.{}", parts[parts.len() - 3], parts[parts.len() - 2], parts[parts.len() - 1])
        } else {
            format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1])
        }
    } else {
        host.to_string()
    }
}

/// 从形如 "104.21.5.12:443" 或 "[::1]:80" 中提取纯 IP
fn extract_ip_from_dest(dest: &str) -> String {
    let dest = dest.trim();
    if let Some(idx) = dest.rfind(':') {
        dest[..idx].trim_start_matches('[').trim_end_matches(']').to_string()
    } else {
        dest.to_string()
    }
}

impl VpnApp {
    /// 独立的"活跃连接"标签页（抽屉内全宽展示：Tab + 工具栏 + 概览 + 表格）。
    pub(crate) fn ui_proxy_connections(&mut self, ui: &mut Ui) {
        ui.set_min_width(ui.available_width());
        ui.set_width(ui.available_width());

        let is_running = self.proxy_status.state == ProxyState::Running;
        let mut close_id = None;
        let mut close_all = false;
        let mut close_process_name: Option<String> = None;
        let mut copy_text_payload: Option<String> = None;
        let mut open_quick_rule: Option<(String, String, String, bool)> = None;

        // 零堆内存分配聚合活跃进程数据：直接借用 &str 作为 Key，预分配容量
        let mut process_map: std::collections::HashMap<&str, (u64, u64, usize, usize, usize)> =
            std::collections::HashMap::with_capacity(32);
        let mut direct_conns = 0usize;
        let mut proxy_conns = 0usize;
        let mut direct_bytes = 0u64;
        let mut proxy_bytes = 0u64;

        for c in &self.active_connections {
            let total_traffic = c.download + c.upload;
            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
            if is_direct {
                direct_conns += 1;
                direct_bytes += total_traffic;
            } else {
                proxy_conns += 1;
                proxy_bytes += total_traffic;
            }

            let proc_name = if c.process.is_empty() { "未知进程" } else { c.process.as_str() };
            let entry = process_map.entry(proc_name).or_insert((0, 0, 0, 0, 0));
            entry.0 += c.upload;
            entry.1 += c.download;
            entry.2 += 1;
            if is_direct {
                entry.3 += 1;
            } else {
                entry.4 += 1;
            }
        }

        let process_count = process_map.len();
        let total_traffic = direct_bytes + proxy_bytes;
        let total_conns = (direct_conns + proxy_conns) as f32;

        // 仅在进程视图才真正物化并排序进程列表，其它 Tab 下跳过无谓的向量创建与排序开销
        let process_list: Vec<(&str, u64, u64, usize, usize, usize)> =
            if self.connections_tab == ConnectionsTab::Processes {
                let mut list: Vec<(&str, u64, u64, usize, usize, usize)> = process_map
                    .into_iter()
                    .map(|(k, v)| (k, v.0, v.1, v.2, v.3, v.4))
                    .collect();
                list.sort_unstable_by_key(|p| std::cmp::Reverse(p.1 + p.2));
                list
            } else {
                Vec::new()
            };

        // 顶部 macOS 风格次级 Tab 切换栏：连接列表 / 连接进程 / DNS 记录
        ui.horizontal(|ui| {
            let conn_str = format!("连接列表 ({})", self.active_connections.len());
            let proc_str = format!("连接进程 ({process_count})");
            let dns_str = format!("DNS 记录 ({})", self.dns_records.len());
            let tabs = [
                (ConnectionsTab::List, Icon::Radar, conn_str.as_str()),
                (ConnectionsTab::Processes, Icon::Cpu, proc_str.as_str()),
                (ConnectionsTab::Dns, Icon::Globe, dns_str.as_str()),
            ];
            macos_segmented_tabs(ui, &mut self.connections_tab, &tabs);
        });

        ui.add_space(8.0);

        if !is_running {
            ui.add_space(20.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(
                        RichText::new("代理内核未运行")
                            .strong()
                            .size(14.0)
                            .color(theme::text_primary()),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("启动内核后，此处将实时展示各网络连接的目标地址、所属进程、分流规则及上传下载速率。")
                            .small()
                            .color(theme::muted()),
                    );
                    ui.add_space(40.0);
                });
            });
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("proxy_connections_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match self.connections_tab {
            ConnectionsTab::List => {
                const MAX_VISIBLE_CONNS: usize = 150;
                let filter = self.connections_filter.trim().to_string();
                let filtered: Vec<&ActiveConnectionInfo> = self
                    .active_connections
                    .iter()
                    .filter(|c| {
                        if self.connections_unmatched_only && !is_unmatched_rule(&c.rule) {
                            return false;
                        }
                        if filter.is_empty() {
                            true
                        } else {
                            contains_ignore_case(&c.process, &filter)
                                || contains_ignore_case(&c.host, &filter)
                                || contains_ignore_case(&c.destination, &filter)
                                || contains_ignore_case(&c.rule, &filter)
                        }
                    })
                    .collect();

                let unmatched_count = self
                    .active_connections
                    .iter()
                    .filter(|c| is_unmatched_rule(&c.rule))
                    .count();

                // 1. 工具栏：筛选 + 统计 + 批量断开（两端对齐充实 100% 宽度）
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选");
                    let conn_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_filter)
                            .hint_text("按进程、域名或 IP 过滤")
                            .desired_width(170.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &conn_filter_field);
                    if !self.connections_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_filter.clear();
                    }

                    // 显式高辨识度实体筛选按钮：具备清晰边框、背景、点击状态与手型光标，杜绝纯文本标签认知模糊
                    let (btn_text, btn_fill, btn_stroke, text_color, hint_text) = if self.connections_unmatched_only {
                        (
                            format!("⚡ 仅看未匹配规则 ({unmatched_count}) ✕"),
                            theme::warning().gamma_multiply(0.20),
                            Stroke::new(1.0, theme::warning()),
                            theme::warning(),
                            "当前已开启筛选：仅展示走 MATCH/FINAL 兜底规则的连接。\n点击可取消筛选，恢复查看全部连接。",
                        )
                    } else if unmatched_count > 0 {
                        (
                            format!("⚠️ 筛选未匹配规则 ({unmatched_count})"),
                            if theme::is_light() { Color32::from_rgb(254, 243, 199) } else { theme::warning().gamma_multiply(0.12) },
                            Stroke::new(1.0, theme::warning().gamma_multiply(0.65)),
                            theme::warning(),
                            "点击开启筛选：仅展示未命中专属域名/IP规则、走 MATCH 兜底的连接，方便一键补充专属分流规则。",
                        )
                    } else {
                        (
                            "✓ 未匹配规则 (0)".to_string(),
                            theme::panel_hover(),
                            Stroke::new(1.0, theme::border_subtle()),
                            theme::muted(),
                            "当前所有活跃连接均已命中专属规则，没有走 MATCH/FINAL 兜底的连接。",
                        )
                    };

                    let filter_btn = egui::Button::new(
                        RichText::new(btn_text)
                            .small()
                            .strong()
                            .color(text_color),
                    )
                    .fill(btn_fill)
                    .stroke(btn_stroke)
                    .corner_radius(CornerRadius::same(6));

                    let btn_resp = ui.add_enabled(unmatched_count > 0 || self.connections_unmatched_only, filter_btn);
                    if btn_resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(hint_text)
                        .clicked()
                    {
                        self.connections_unmatched_only = !self.connections_unmatched_only;
                    }

                    ui.separator();
                    let total_count = self.active_connections.len();
                    let filtered_count = filtered.len();
                    let count_text = if self.connections_unmatched_only || !filter.is_empty() {
                        format!("活跃连接: {filtered_count} / {total_count} 条 (已筛选)")
                    } else {
                        format!("活跃连接: {total_count} 条")
                    };
                    ui.label(RichText::new(count_text).small().color(theme::text_secondary()));

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.active_connections.is_empty(), danger_btn("断开全部连接"))
                            .on_hover_text("向内核发送指令关闭所有活动的网络连接")
                            .clicked()
                        {
                            close_all = true;
                        }
                    });
                });

                ui.add_space(8.0);

                // 2. 分流流向概览
                if total_conns > 0.0 {
                    Frame::NONE
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Radar, 13.0, theme::accent());
                                    ui.label(RichText::new("分流流向监控").small().strong().color(theme::text_primary()));
                                });
                                ui.add_space(4.0);

                                let segments = [
                                    ("代理分流 (Proxy)", proxy_conns as f32, theme::accent()),
                                    ("国内直连 (Direct)", direct_conns as f32, theme::border()),
                                ];
                                crate::gui::widgets::render_distribution_bar(ui, &segments, total_conns, 8.0);
                                ui.add_space(4.0);

                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "代理: {} 连 ({})",
                                            proxy_conns,
                                            format_bytes(proxy_bytes)
                                        ))
                                        .small()
                                        .color(theme::accent_strong()),
                                    );
                                    ui.separator();
                                    ui.label(
                                        RichText::new(format!(
                                            "直连: {} 连 ({})",
                                            direct_conns,
                                            format_bytes(direct_bytes)
                                        ))
                                        .small()
                                        .color(theme::text_secondary()),
                                    );
                                });
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3. 连接列表表格
                if filtered.is_empty() {
                    ui.add_space(16.0);
                    card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add_space(32.0);
                            if self.active_connections.is_empty() {
                                ui.label(
                                    RichText::new("当前暂无活动的网络连接")
                                        .strong()
                                        .size(13.5)
                                        .color(theme::text_secondary()),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("当有网络请求经由内核转发时，此处将实时呈现连接明细")
                                        .small()
                                        .color(theme::muted()),
                                );
                            } else if self.connections_unmatched_only {
                                ui.label(
                                    RichText::new("🎉 太棒了！当前没有未匹配规则的连接")
                                        .strong()
                                        .size(13.5)
                                        .color(theme::success()),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("所有活跃网络连接均已精准命中专属分流规则")
                                        .small()
                                        .color(theme::muted()),
                                );
                                ui.add_space(10.0);
                                if ui.button("恢复查看全部连接").clicked() {
                                    self.connections_unmatched_only = false;
                                }
                            } else {
                                ui.label(
                                    RichText::new("没有匹配筛选条件的网络连接")
                                        .strong()
                                        .size(13.5)
                                        .color(theme::text_secondary()),
                                );
                                ui.add_space(6.0);
                                if ui.button("清空筛选条件").clicked() {
                                    self.connections_filter.clear();
                                    self.connections_unmatched_only = false;
                                }
                            }
                            ui.add_space(32.0);
                        });
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let grid_w = ui.available_width();
                        let proc_w = (grid_w * 0.16).clamp(100.0, 180.0);
                        let rule_w = (grid_w * 0.14).clamp(80.0, 150.0);
                        let other_cols_w = 44.0 + proc_w + rule_w + 80.0 + 85.0 + 110.0 + (6.0 * 12.0);
                        let dest_w = (grid_w - other_cols_w).max(200.0);

                        egui::ScrollArea::horizontal()
                            .id_salt("active_connections_hscroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(grid_w);
                                egui::Grid::new("active_connections_grid")
                                    .striped(true)
                                    .min_col_width(44.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "网络");
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(proc_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(proc_w);
                                                ui.set_max_width(proc_w);
                                                header_cell(ui, "进程");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dest_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dest_w);
                                                ui.set_max_width(dest_w);
                                                header_cell(ui, "目标地址");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(rule_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(rule_w);
                                                ui.set_max_width(rule_w);
                                                header_cell(ui, "分流规则");
                                            },
                                        );
                                        header_cell(ui, "上传流量");
                                        header_cell(ui, "下载流量");
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        let displayed_conns = if filtered.len() > MAX_VISIBLE_CONNS {
                                            &filtered[..MAX_VISIBLE_CONNS]
                                        } else {
                                            &filtered[..]
                                        };

                                        for c in displayed_conns {
                                            // 1. 网络协议微晶徽标 (零堆分配静态映射)
                                            let (net_label, net_fill, net_color) = if c.network.eq_ignore_ascii_case("tcp") {
                                                ("TCP", theme::accent_bg(), theme::accent_strong())
                                            } else if c.network.eq_ignore_ascii_case("udp") {
                                                ("UDP", theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (c.network.as_str(), theme::sidebar(), theme::muted())
                                            };
                                            Frame::NONE
                                                .fill(net_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(net_label).small().strong().color(net_color));
                                                });

                                            // 2. 进程胶囊
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(proc_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(proc_w);
                                                    ui.set_max_width(proc_w);
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                icon_label(ui, Icon::Terminal, 10.0, theme::muted());
                                                                ui.add(egui::Label::new(RichText::new(&c.process).strong().small().color(theme::text_primary())).truncate());
                                                            });
                                                        });
                                                },
                                            );

                                            // 3. 目标地址（长域名自适应展开，悬浮查看完整地址与目标 IP）
                                            let full_target = if !c.host.is_empty() {
                                                format!("{} ({})", c.host, c.destination)
                                            } else {
                                                c.destination.clone()
                                            };
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dest_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dest_w);
                                                    ui.set_max_width(dest_w);
                                                    let small_font = egui::TextStyle::Small.resolve(ui.style());
                                                    let shown_target = crate::gui::widgets::ellipsize(
                                                        ui,
                                                        &full_target,
                                                        dest_w,
                                                        small_font,
                                                    );
                                                    let target_resp = ui.add(
                                                        egui::Label::new(
                                                            RichText::new(shown_target).small().color(theme::text_primary()),
                                                        ),
                                                    );
                                                    if target_resp.hovered() {
                                                        let hover_text = if !c.host.is_empty() {
                                                            format!("主机: {}\n目标: {}", c.host, c.destination)
                                                        } else {
                                                            format!("目标: {}", c.destination)
                                                        };
                                                        target_resp.on_hover_text(hover_text);
                                                    }
                                                },
                                            );

                                            // 4. 分流规则微标 (零堆分配引用与惰性气泡提示)
                                            let is_unmatched = is_unmatched_rule(&c.rule);
                                            let is_direct = c.target.eq_ignore_ascii_case("direct") || c.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_unmatched {
                                                (theme::warning().gamma_multiply(0.18), theme::warning())
                                            } else if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            let rule_label: &str = if is_unmatched {
                                                "⚠️ 兜底 (MATCH)"
                                            } else {
                                                c.rule.as_str()
                                            };
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(rule_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(rule_w);
                                                    ui.set_max_width(rule_w);
                                                    Frame::NONE
                                                        .fill(rule_fill)
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            let rule_resp = ui.add(egui::Label::new(RichText::new(rule_label).small().strong().color(rule_color)).truncate());
                                                            if rule_resp.hovered() {
                                                                if is_unmatched {
                                                                    rule_resp.on_hover_text("该连接未命中任何专属分流规则，当前由默认兜底规则 (MATCH) 接管。可点击右侧「➕ 规则」为其添加专属分流规则。");
                                                                } else {
                                                                    rule_resp.on_hover_text(format!("命中分流规则: {}\n派发目标: {}", c.rule, c.target));
                                                                }
                                                            }
                                                        });
                                                },
                                            );

                                            // 5. 上传
                                            ui.label(
                                                RichText::new(format!("↑ {}", format_bytes(c.upload)))
                                                    .small()
                                                    .color(theme::text_secondary()),
                                            );

                                            // 6. 下载
                                            ui.label(
                                                RichText::new(format!("↓ {}", format_bytes(c.download)))
                                                    .small()
                                                    .strong()
                                                    .color(theme::accent_strong()),
                                            );

                                            // 7. 操作
                                            ui.horizontal(|ui| {
                                                if ui
                                                    .small_button("➕ 规则")
                                                    .on_hover_text("为此目标地址或进程快捷添加分流规则并热重载生效")
                                                    .clicked()
                                                {
                                                    open_quick_rule = Some((
                                                        c.host.clone(),
                                                        c.destination.clone(),
                                                        c.process.clone(),
                                                        is_direct,
                                                    ));
                                                }
                                                if ui
                                                    .add(danger_btn("断开"))
                                                    .on_hover_text("断开此连接")
                                                    .clicked()
                                                {
                                                    close_id = Some(c.id.clone());
                                                }
                                            });
                                            ui.end_row();
                                        }
                                    });
                            });
                        if filtered.len() > MAX_VISIBLE_CONNS {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "💡 为保证 60 FPS 丝滑渲染，当前优先展示前 {MAX_VISIBLE_CONNS} 条连接（共 {} 条）。可使用上方搜索框过滤精确定位。",
                                        filtered.len()
                                    ))
                                    .small()
                                    .color(theme::muted()),
                                );
                            });
                        }
                    });
                }
            }
            ConnectionsTab::Processes => {
                // 1. 工具栏
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选进程");
                    let proc_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_process_filter)
                            .hint_text("按进程名过滤")
                            .desired_width(180.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &proc_filter_field);
                    if !self.connections_process_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_process_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("当前活跃进程: {} 个", process_list.len()));
                    ui.separator();
                    ui.label(RichText::new(format!("进程产生总流量: {}", format_bytes(total_traffic))).small().color(theme::muted()));
                });

                ui.add_space(8.0);

                // 2. Top 进程矩阵卡片
                let top_matrix: Vec<(&str, u64, usize)> = process_list
                    .iter()
                    .map(|p| (p.0, p.1 + p.2, p.3))
                    .collect();
                if !top_matrix.is_empty() {
                    Frame::NONE
                        .fill(theme::panel())
                        .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    icon_label(ui, Icon::Cpu, 13.0, theme::accent());
                                    ui.label(RichText::new("Top 进程流量与带宽占比").small().strong().color(theme::text_primary()));
                                });
                                ui.add_space(4.0);
                                crate::gui::widgets::render_top_processes_card(ui, &top_matrix, total_traffic);
                            });
                        });
                    ui.add_space(8.0);
                }

                // 3. 进程详细聚合表
                let p_filter = self.connections_process_filter.trim();
                let filtered_procs: Vec<&(&str, u64, u64, usize, usize, usize)> = process_list
                    .iter()
                    .filter(|p| p_filter.is_empty() || contains_ignore_case(p.0, p_filter))
                    .collect();

                if filtered_procs.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("当前暂无进程产生网络连接").color(theme::muted()));
                            ui.add_space(20.0);
                        });
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let p_grid_w = ui.available_width();
                        let p_other_w = 85.0 + 110.0 + 85.0 + 85.0 + 90.0 + 75.0 + (6.0 * 14.0);
                        let p_name_w = (p_grid_w - p_other_w).max(160.0);

                        egui::ScrollArea::horizontal()
                            .id_salt("process_connections_hscroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(p_grid_w);
                                egui::Grid::new("process_connections_grid")
                                    .striped(true)
                                    .min_col_width(70.0)
                                    .spacing([14.0, 8.0])
                                    .show(ui, |ui| {
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(p_name_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(p_name_w);
                                                ui.set_max_width(p_name_w);
                                                header_cell(ui, "进程");
                                            },
                                        );
                                        header_cell(ui, "活跃连接数");
                                        header_cell(ui, "分流分布");
                                        header_cell(ui, "上行流量");
                                        header_cell(ui, "下行流量");
                                        header_cell(ui, "总流量");
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for p in filtered_procs {
                                            let proc_name = &p.0;
                                            let upload = p.1;
                                            let download = p.2;
                                            let conn_count = p.3;
                                            let dir_count = p.4;
                                            let prx_count = p.5;
                                            let proc_traffic = upload + download;

                                            // 1. 进程名称（自适应展开，超出智能截断）
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(p_name_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(p_name_w);
                                                    ui.set_max_width(p_name_w);
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(8, 4))
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                icon_label(ui, Icon::Terminal, 11.0, theme::accent());
                                                                ui.add(egui::Label::new(RichText::new(*proc_name).strong().color(theme::text_primary())).truncate());
                                                            });
                                                        });
                                                },
                                            );

                                            // 2. 活跃连接数
                                            ui.label(RichText::new(format!("{conn_count} 条")).strong().color(theme::text_primary()));

                                            // 3. 分流分布
                                            ui.horizontal(|ui| {
                                                if prx_count > 0 {
                                                    ui.label(RichText::new(format!("代理:{prx_count}")).small().color(theme::accent_strong()));
                                                }
                                                if dir_count > 0 {
                                                    ui.label(RichText::new(format!("直连:{dir_count}")).small().color(theme::text_secondary()));
                                                }
                                            });

                                            // 4. 上行
                                            ui.label(RichText::new(format!("↑ {}", format_bytes(upload))).small().color(theme::text_secondary()));

                                            // 5. 下行
                                            ui.label(RichText::new(format!("↓ {}", format_bytes(download))).small().strong().color(theme::accent_strong()));

                                            // 6. 总流量
                                            ui.label(RichText::new(format_bytes(proc_traffic)).strong().color(theme::text_primary()));

                                            // 7. 操作：关闭该进程的所有连接
                                            if ui
                                                .add(danger_btn("断开全部"))
                                                .on_hover_text(format!("关闭进程 {proc_name} 的全部活动连接"))
                                                .clicked()
                                            {
                                                close_process_name = Some(proc_name.to_string());
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                }
            }
            ConnectionsTab::Dns => {
                // 1. 工具栏：筛选 + 统计 + 清空（两端对齐充实 100% 宽度）
                ui.horizontal_wrapped(|ui| {
                    ui.label("筛选记录");
                    let dns_filter_field = ui.add(
                        egui::TextEdit::singleline(&mut self.connections_dns_filter)
                            .hint_text("按域名、解析 IP 或发起进程过滤")
                            .desired_width(200.0),
                    );
                    crate::gui::widgets::note_text_field(ui, &dns_filter_field);
                    if !self.connections_dns_filter.is_empty() && ui.button("清空").clicked() {
                        self.connections_dns_filter.clear();
                    }
                    ui.separator();
                    ui.label(format!("已记录 DNS 轨迹: {} 条", self.dns_records.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("清空全部记录").clicked() {
                            self.dns_records.clear();
                        }
                    });
                });

                ui.add_space(8.0);

                // 2. DNS 运行配置摘要
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0_f32, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Globe, 14.0, theme::accent());
                            ui.label(RichText::new("DNS 架构状态:").strong().color(theme::text_primary()));
                            let (mode_str, mode_color) = match self.proxy_config.dns.enhanced_mode {
                                DnsEnhancedMode::FakeIp => ("Fake-IP (秒级并发映射)", theme::accent_strong()),
                                DnsEnhancedMode::RedirHost => ("Redir-Host (真实解析直通)", theme::accent()),
                            };
                            ui.label(RichText::new(mode_str).strong().color(mode_color));
                            ui.separator();
                            let ns = self.proxy_config.dns.nameservers.first().map(|s| s.as_str()).unwrap_or("doh.pub");
                            ui.label(RichText::new(format!("主上游: {ns}")).small().color(theme::muted()));
                        });
                    });

                ui.add_space(8.0);

                // 3. DNS 记录表格
                let d_filter = self.connections_dns_filter.trim();
                let filtered_dns: Vec<&DnsRecord> = self
                    .dns_records
                    .iter()
                    .filter(|r| {
                        d_filter.is_empty()
                            || contains_ignore_case(&r.host, d_filter)
                            || contains_ignore_case(&r.ip, d_filter)
                            || contains_ignore_case(&r.process, d_filter)
                            || contains_ignore_case(&r.rule, d_filter)
                    })
                    .collect();

                if filtered_dns.is_empty() {
                    card_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(
                                RichText::new(if self.dns_records.is_empty() {
                                    "暂无实时 DNS 记录（发起网页浏览或网络请求时将在此自动捕获）"
                                } else {
                                    "没有匹配筛选条件的 DNS 记录"
                                })
                                .color(theme::muted()),
                            );
                            ui.add_space(20.0);
                        });
                    });
                } else {
                    panel_frame().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let dns_grid_w = ui.available_width();
                        let dns_proc_w = (dns_grid_w * 0.14).clamp(100.0, 160.0);
                        let dns_rule_w = (dns_grid_w * 0.12).clamp(70.0, 120.0);
                        let dns_other_w = 75.0 + dns_proc_w + 44.0 + dns_rule_w + 50.0 + (6.0 * 12.0);
                        let dns_remaining = (dns_grid_w - dns_other_w).max(320.0);
                        let dns_host_w = (dns_remaining * 0.55).max(170.0);
                        let dns_ip_w = (dns_remaining * 0.45).max(140.0);

                        egui::ScrollArea::horizontal()
                            .id_salt("dns_records_hscroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(dns_grid_w);
                                egui::Grid::new("dns_records_grid")
                                    .striped(true)
                                    .min_col_width(44.0)
                                    .spacing([12.0, 7.0])
                                    .show(ui, |ui| {
                                        header_cell(ui, "时间");
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_host_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_host_w);
                                                ui.set_max_width(dns_host_w);
                                                header_cell(ui, "查询域名");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_ip_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_ip_w);
                                                ui.set_max_width(dns_ip_w);
                                                header_cell(ui, "解析目标 IP");
                                            },
                                        );
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_proc_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_proc_w);
                                                ui.set_max_width(dns_proc_w);
                                                header_cell(ui, "发起进程");
                                            },
                                        );
                                        header_cell(ui, "网络");
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(dns_rule_w, 20.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_width(dns_rule_w);
                                                ui.set_max_width(dns_rule_w);
                                                header_cell(ui, "分流动作");
                                            },
                                        );
                                        header_cell(ui, "操作");
                                        ui.end_row();

                                        for r in filtered_dns {
                                            // 1. 时间
                                            ui.label(RichText::new(&r.time_str).small().monospace().color(theme::muted()));

                                            // 2. 域名（自适应铺满展开）
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_host_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_host_w);
                                                    ui.set_max_width(dns_host_w);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&r.host).strong().small().color(theme::text_primary()),
                                                        )
                                                        .truncate(),
                                                    );
                                                },
                                            );

                                            // 3. 解析目标 IP（自适应铺满展开）
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_ip_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_ip_w);
                                                    ui.set_max_width(dns_ip_w);
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(&r.ip).small().monospace().color(theme::text_secondary()),
                                                        )
                                                        .truncate(),
                                                    );
                                                },
                                            );

                                            // 4. 发起进程
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_proc_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_proc_w);
                                                    ui.set_max_width(dns_proc_w);
                                                    Frame::NONE
                                                        .fill(theme::sidebar())
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.horizontal(|ui| {
                                                                icon_label(ui, Icon::Terminal, 9.0, theme::muted());
                                                                ui.add(egui::Label::new(RichText::new(&r.process).small().color(theme::text_primary())).truncate());
                                                            });
                                                        });
                                                },
                                            );

                                            // 5. 协议
                                            let (net_fill, net_color) = if r.network == "UDP" {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            Frame::NONE
                                                .fill(net_fill)
                                                .corner_radius(CornerRadius::same(6))
                                                .inner_margin(Margin::symmetric(6, 2))
                                                .show(ui, |ui| {
                                                    ui.label(RichText::new(&r.network).small().strong().color(net_color));
                                                });

                                            // 6. 分流动作
                                            let is_direct = r.target.eq_ignore_ascii_case("direct") || r.rule.contains("DIRECT");
                                            let (rule_fill, rule_color) = if is_direct {
                                                (theme::panel_hover(), theme::text_secondary())
                                            } else {
                                                (theme::accent_bg(), theme::accent_strong())
                                            };
                                            ui.allocate_ui_with_layout(
                                                Vec2::new(dns_rule_w, 20.0),
                                                egui::Layout::left_to_right(egui::Align::Center),
                                                |ui| {
                                                    ui.set_min_width(dns_rule_w);
                                                    ui.set_max_width(dns_rule_w);
                                                    Frame::NONE
                                                        .fill(rule_fill)
                                                        .corner_radius(CornerRadius::same(6))
                                                        .inner_margin(Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.add(egui::Label::new(RichText::new(&r.rule).small().strong().color(rule_color)).truncate());
                                                        });
                                                },
                                            );

                                            // 7. 操作与复制
                                            ui.horizontal(|ui| {
                                                if ui
                                                    .small_button("➕ 规则")
                                                    .on_hover_text("为此解析域名快捷添加分流规则并热重载生效")
                                                    .clicked()
                                                {
                                                    open_quick_rule = Some((
                                                        r.host.clone(),
                                                        r.ip.clone(),
                                                        r.process.clone(),
                                                        is_direct,
                                                    ));
                                                }
                                                if ui.small_button("复制域名").clicked() {
                                                    copy_text_payload = Some(r.host.clone());
                                                }
                                                if ui.small_button("复制IP").clicked() {
                                                    copy_text_payload = Some(r.ip.clone());
                                                }
                                            });
                                            ui.end_row();
                                        }
                                    });
                            });
                    });
                }
            }
        }
        ui.add_space(16.0);
    });

        if let Some(id) = close_id {
            self.close_connection(id);
        } else if close_all {
            self.close_all_connections();
        } else if let Some(proc_name) = close_process_name {
            let ids_to_close: Vec<String> = self
                .active_connections
                .iter()
                .filter(|c| c.process == proc_name)
                .map(|c| c.id.clone())
                .collect();
            for id in ids_to_close {
                self.close_connection(id);
            }
        }

        if let Some(text) = copy_text_payload {
            ui.ctx().copy_text(text);
            self.status = "已复制到剪贴板".into();
        }

        if let Some((host, dest, proc, direct)) = open_quick_rule {
            let raw_ip = extract_ip_from_dest(&dest);
            let (rule_type, payload) = if !host.is_empty() {
                (QuickRuleType::DomainSuffix, infer_domain_suffix(&host))
            } else if !raw_ip.is_empty() {
                (QuickRuleType::IpCidr, format!("{raw_ip}/32"))
            } else {
                (QuickRuleType::ProcessName, proc.clone())
            };
            let action = if direct {
                crate::proxy::RuleAction::Proxy
            } else {
                crate::proxy::RuleAction::Direct
            };
            self.quick_rule_modal = Some(QuickRuleModalState {
                rule_type,
                host,
                ip: raw_ip,
                process: proc,
                payload,
                action,
            });
        }
    }

    /// 快捷添加分流规则对话框
    pub(crate) fn ui_quick_rule_modal(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.quick_rule_modal.clone() else {
            return;
        };

        let mut close_modal = false;
        let mut save_rule = false;
        let mut is_open = true;

        // 监听 Escape 键快速关闭当前弹窗
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close_modal = true;
        }

        // 半透明遮罩：Order::Tooltip 确保压在抽屉 (Foreground) 之上
        let screen = ctx.viewport_rect();
        egui::Area::new(egui::Id::new("quick_rule_modal_backdrop"))
            .fixed_pos(screen.min)
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.painter()
                    .rect_filled(screen, 0.0, Color32::from_black_alpha(110));
            });

        egui::Window::new("➕ 快捷添加分流规则")
            .id(egui::Id::new("quick_rule_modal_window"))
            .order(egui::Order::Tooltip)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut is_open)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                Frame::NONE
                    .fill(theme::panel())
                    .stroke(Stroke::new(1.0, theme::border_subtle()))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(16, 14))
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("为当前目标地址或活动进程快速生成分流规则，保存后内核将立即热重载生效：")
                                    .small()
                                    .color(theme::text_secondary()),
                            );
                            ui.add_space(8.0);

                            // 1. 规则匹配类型选择
                            ui.label(RichText::new("规则匹配类型:").strong().small());
                            ui.horizontal_wrapped(|ui| {
                                if !state.host.is_empty() {
                                    if ui.selectable_label(
                                        state.rule_type == QuickRuleType::DomainSuffix,
                                        "域名后缀 (DOMAIN-SUFFIX)",
                                    ).clicked() {
                                        state.rule_type = QuickRuleType::DomainSuffix;
                                        state.payload = infer_domain_suffix(&state.host);
                                    }
                                    if ui.selectable_label(
                                        state.rule_type == QuickRuleType::Domain,
                                        "完整域名 (DOMAIN)",
                                    ).clicked() {
                                        state.rule_type = QuickRuleType::Domain;
                                        state.payload = state.host.clone();
                                    }
                                }
                                if !state.ip.is_empty() {
                                    if ui.selectable_label(
                                        state.rule_type == QuickRuleType::IpCidr,
                                        "IP网段 (IP-CIDR)",
                                    ).clicked() {
                                        state.rule_type = QuickRuleType::IpCidr;
                                        state.payload = format!("{}/32", state.ip);
                                    }
                                }
                                if !state.process.is_empty() {
                                    if ui.selectable_label(
                                        state.rule_type == QuickRuleType::ProcessName,
                                        "发起进程 (PROCESS-NAME)",
                                    ).clicked() {
                                        state.rule_type = QuickRuleType::ProcessName;
                                        state.payload = state.process.clone();
                                    }
                                }
                            });

                            ui.add_space(6.0);

                            // 2. 匹配表达式
                            ui.label(RichText::new("匹配表达式:").strong().small());
                            let edit_resp = ui.add(
                                egui::TextEdit::singleline(&mut state.payload)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit_resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                                && !state.payload.trim().is_empty()
                            {
                                save_rule = true;
                            }

                            ui.add_space(8.0);

                            // 3. 路由分流动作选择
                            ui.label(RichText::new("分流动作:").strong().small());
                            ui.horizontal(|ui| {
                                if ui.selectable_label(
                                    state.action == crate::proxy::RuleAction::Direct,
                                    "🟢 国内直连 (DIRECT)",
                                ).clicked() {
                                    state.action = crate::proxy::RuleAction::Direct;
                                }
                                if ui.selectable_label(
                                    state.action == crate::proxy::RuleAction::Proxy,
                                    "⚡ 走代理 (PROXY)",
                                ).clicked() {
                                    state.action = crate::proxy::RuleAction::Proxy;
                                }
                                if ui.selectable_label(
                                    state.action == crate::proxy::RuleAction::Reject,
                                    "🛑 拦截广告 (REJECT)",
                                ).clicked() {
                                    state.action = crate::proxy::RuleAction::Reject;
                                }
                            });

                            ui.add_space(10.0);
                            ui.separator();
                            ui.add_space(8.0);

                            // 4. 操作按钮组
                            ui.horizontal(|ui| {
                                if ui.button("取消").clicked() {
                                    close_modal = true;
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let can_save = !state.payload.trim().is_empty();
                                    if ui
                                        .add_enabled(can_save, primary_btn("确认添加并热生效"))
                                        .clicked()
                                    {
                                        save_rule = true;
                                    }
                                });
                            });
                        });
                    });
            });

        if !is_open || close_modal {
            self.quick_rule_modal = None;
        } else if save_rule {
            let matcher = match state.rule_type {
                QuickRuleType::DomainSuffix => crate::proxy::RuleMatcher::DomainSuffix(state.payload.trim().to_string()),
                QuickRuleType::Domain => crate::proxy::RuleMatcher::Domain(state.payload.trim().to_string()),
                QuickRuleType::IpCidr => crate::proxy::RuleMatcher::IpCidr(state.payload.trim().to_string()),
                QuickRuleType::ProcessName => crate::proxy::RuleMatcher::ProcessName(state.payload.trim().to_string()),
            };
            let new_rule = crate::proxy::TrafficRule {
                matcher,
                action: state.action,
            };
            self.proxy_config.rules.insert(0, new_rule);
            self.save_proxy_config();
            self.status = format!("已添加规则并热重载生效: {}", state.payload.trim());
            self.quick_rule_modal = None;
        } else {
            self.quick_rule_modal = Some(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_unmatched_rule() {
        assert!(is_unmatched_rule(""));
        assert!(is_unmatched_rule("   "));
        assert!(is_unmatched_rule("MATCH"));
        assert!(is_unmatched_rule("MATCH(PROXY)"));
        assert!(is_unmatched_rule("match(direct)"));
        assert!(is_unmatched_rule("FINAL"));
        assert!(is_unmatched_rule("FINAL(DIRECT)"));

        assert!(!is_unmatched_rule("DOMAIN-SUFFIX(github.com, PROXY)"));
        assert!(!is_unmatched_rule("IP-CIDR(192.168.1.1/32, DIRECT)"));
        assert!(!is_unmatched_rule("GEOSITE(cn, DIRECT)"));
        assert!(!is_unmatched_rule("GEOIP(CN, DIRECT)"));
    }

    #[test]
    fn test_infer_domain_suffix() {
        assert_eq!(infer_domain_suffix("api.github.com"), "github.com");
        assert_eq!(infer_domain_suffix("sub.deep.example.com"), "example.com");
        assert_eq!(infer_domain_suffix("news.sina.com.cn"), "sina.com.cn");
        assert_eq!(infer_domain_suffix("localhost"), "localhost");
        assert_eq!(infer_domain_suffix("1.1.1.1"), "1.1.1.1");
    }

    #[test]
    fn test_extract_ip_from_dest() {
        assert_eq!(extract_ip_from_dest("104.21.5.12:443"), "104.21.5.12");
        assert_eq!(extract_ip_from_dest("[2606:4700::6815:50c]:443"), "2606:4700::6815:50c");
        assert_eq!(extract_ip_from_dest("1.1.1.1"), "1.1.1.1");
    }

    #[test]
    fn test_contains_ignore_case() {
        assert!(contains_ignore_case("api.github.com", "github"));
        assert!(contains_ignore_case("api.GITHUB.com", "github"));
        assert!(contains_ignore_case("chrome.exe", "CHROME"));
        assert!(contains_ignore_case("127.0.0.1:8080", "8080"));
        assert!(!contains_ignore_case("api.github.com", "gitlab"));
        assert!(contains_ignore_case("任意内容", ""));
        assert!(!contains_ignore_case("", "a"));
        assert!(contains_ignore_case("微信.exe", "微信"));
    }
}


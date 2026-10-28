use crate::gui::icons::{icon_label, Icon};
use crate::gui::theme;
use crate::gui::VpnApp;
use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke};

impl VpnApp {
    pub(crate) fn ui_ip_risk_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_ip_risk_dialog {
            return;
        }
        if self.ip_risk_busy || self.ip_risk_egress_pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        let mut open = self.show_ip_risk_dialog;
        let mut retry = false;
        let mut copy_payload: Option<String> = None;

        egui::Window::new(RichText::new("IP 检测 · 归属与风控画像").strong())
            .open(&mut open)
            .order(egui::Order::Tooltip)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if self.ip_risk_busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(&self.status).color(theme::text_primary()));
                    });
                    return;
                }

                if let Some(error) = &self.ip_risk_error {
                    Frame::NONE
                        .fill(theme::danger_bg())
                        .stroke(Stroke::new(1.0_f32, theme::danger()))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.label(RichText::new(error).color(theme::danger()));
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(
                                    "节点地址是域名时查不到归属；也可以重试一次。",
                                )
                                .small()
                                .color(theme::muted()),
                            );
                        });
                    ui.add_space(6.0);
                    if ui.button("重新查询").clicked() {
                        retry = true;
                    }
                    return;
                }

                let Some(risk) = self.ip_risk.as_ref() else {
                    ui.label(RichText::new("还没有查询结果。").color(theme::muted()));
                    return;
                };

                let color = if risk.datacenter { theme::warning() } else { theme::accent_strong() };
                ui.label(
                    RichText::new("入口（节点服务器 IP）")
                        .small()
                        .color(theme::muted()),
                );
                Frame::NONE
                    .fill(if risk.datacenter {
                        theme::warning_bg()
                    } else {
                        theme::accent_bg()
                    })
                    .stroke(Stroke::new(1.0_f32, color))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            icon_label(ui, Icon::Shield, 18.0, color);
                            ui.label(
                                RichText::new(if risk.datacenter {
                                    "机房 / 云厂商"
                                } else {
                                    "住宅 / ISP"
                                })
                                .strong()
                                .size(15.0)
                                .color(color),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(RichText::new(&risk.verdict).small().color(theme::text_primary()));
                    });

                ui.add_space(8.0);
                egui::Grid::new("ip_risk_detail")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        if risk.hostname != risk.ip {
                            ui.label(RichText::new("节点地址").small().color(theme::muted()));
                            ui.label(RichText::new(&risk.hostname).color(theme::text_primary()));
                            ui.end_row();
                        }

                        ui.label(
                            RichText::new(if risk.hostname == risk.ip {
                                "落地 IP"
                            } else {
                                "解析到"
                            })
                            .small()
                            .color(theme::muted()),
                        );
                        ui.label(RichText::new(&risk.ip).color(theme::text_primary()));
                        ui.end_row();

                        ui.label(RichText::new("地区").small().color(theme::muted()));
                        ui.label(RichText::new(if risk.country.is_empty() {
                            "未知"
                        } else {
                            &risk.country
                        })
                        .color(theme::text_primary()));
                        ui.end_row();

                        ui.label(RichText::new("ASN / 运营商").small().color(theme::muted()));
                        ui.label(RichText::new(if risk.org.is_empty() {
                            "无 ASN 信息"
                        } else {
                            &risk.org
                        })
                        .color(theme::text_primary()));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                // 出口侧：本机只有一个 mixed 端口，所以"经代理查出口"只对**当前在跑的
                // 那个节点**成立；别的节点必须用临时内核实测，否则显示的是别人的出口。
                let target = self.ip_risk_target.as_deref();
                let current = self.proxy_status.node_id.as_deref();
                let target_is_running = self.proxy_status.local_proxy_active()
                    && matches!((target, current), (Some(a), Some(b)) if a == b);
                let egress_belongs_to_target = self.ip_risk_egress_for.is_some()
                    && self.ip_risk_egress_for == self.ip_risk_target;
                let egress = if egress_belongs_to_target {
                    self.ip_risk_egress.as_ref()
                } else {
                    None
                };
                let egress_error = if egress_belongs_to_target {
                    self.ip_risk_egress_error.as_deref()
                } else {
                    None
                };
                if self.ip_risk_egress_pending {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("正在经当前代理查出口 IP...")
                                .small()
                                .color(theme::muted()),
                        );
                    });
                } else if let Some(egress) = egress {
                    let color = if egress.datacenter { theme::warning() } else { theme::accent_strong() };
                    Frame::NONE
                        .fill(if egress.datacenter {
                            theme::warning_bg()
                        } else {
                            theme::accent_bg()
                        })
                        .stroke(Stroke::new(1.0_f32, color))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                icon_label(ui, Icon::Globe, 16.0, color);
                                ui.label(
                                    RichText::new("站点实际看到的出口")
                                        .strong()
                                        .color(color),
                                );
                            });
                            ui.label(
                                RichText::new(egress.headline())
                                    .small()
                                    .color(theme::text_primary()),
                            );
                            ui.label(
                                RichText::new(&egress.verdict)
                                    .small()
                                    .color(theme::text_primary()),
                            );
                            if egress.ip != risk.ip {
                                ui.label(
                                    RichText::new(
                                        "入口和出口不是同一个 IP：该节点有中转，风控认的是上面这一行。",
                                    )
                                    .small()
                                    .color(theme::warning()),
                                );
                            }
                        });
                } else if let Some(error) = egress_error {
                    ui.label(
                        RichText::new(format!("出口查询失败：{error}"))
                            .small()
                            .color(theme::danger()),
                    );
                } else if target.is_some() {
                    ui.label(
                        RichText::new(if target_is_running {
                            "出口尚未查询。"
                        } else {
                            "该节点不是当前在跑的节点，出口未知（经本机代理查只会得到当前节点的出口）。"
                        })
                        .small()
                        .color(theme::muted()),
                    );
                }

                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        if risk.hostname == risk.ip {
                            "归类只看 ASN / 公司名关键字，会漏判（例如小商家 VPS），请自行核对上面这行 ASN。\
                             查询走直连、不经过代理，对象就是这个节点服务器 IP。"
                        } else {
                            "节点填的是域名，这里查的是它解析到的 IP —— 域名前置 / ECH / CDN 中转的节点，\
                             这个地址常常是前端而不是真实落地出口（站点侧看到的是出口 IP）。\
                             归类只看 ASN / 公司名关键字，会漏判，请自行核对上面这行 ASN。"
                        },
                    )
                    .small()
                    .color(theme::muted()),
                );

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("复制").clicked() {
                        let mut text = format!("入口：{}\n{}", risk.headline(), risk.verdict);
                        if let Some(egress) = &self.ip_risk_egress {
                            text.push_str(&format!(
                                "\n出口：{}\n{}",
                                egress.headline(),
                                egress.verdict
                            ));
                        }
                        copy_payload = Some(text);
                    }
                    if ui.button("重新查询").clicked() {
                        retry = true;
                    }
                });
            });

        if retry {
            self.ip_risk = None;
            if let Some(target) = self.ip_risk_target.clone() {
                self.check_ip_risk_for(&target);
            }
        }
        if let Some(text) = copy_payload {
            ctx.copy_text(text);
            self.status = "已复制落地 IP 归属信息".into();
        }
        self.show_ip_risk_dialog = open;
    }
}

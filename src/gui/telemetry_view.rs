use super::{theme, BgMsg, VpnApp};
use eframe::egui::{self, RichText};

impl VpnApp {
    /// 把库里最近 24 小时的出口健康捞回界面。启动时一次，之后每次实测落库后再刷。
    pub(crate) fn reload_egress_health(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let streak = db.egress_fail_streak().await.unwrap_or(0);
            let rows = db
                .egress_health_since(chrono::Utc::now() - chrono::Duration::hours(24))
                .await
                .unwrap_or_default();
            let _ = tx.send(BgMsg::EgressHealth(rows, streak));
        });
    }

    /// 把攒下的命中增量写进主库（按天聚合，冲突就累加）。
    pub(crate) fn flush_rule_hits(&mut self) {
        self.last_hit_flush = std::time::Instant::now();
        let at = chrono::Utc::now().to_rfc3339();
        let records: Vec<crate::db::telemetry::RuleHit> = self
            .rule_tally
            .drain()
            .map(|((rule, target), count)| crate::db::telemetry::RuleHit {
                rule,
                target,
                count,
                at: at.clone(),
            })
            .collect();
        if records.is_empty() {
            return;
        }
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = db.record_rule_hits(&records).await;
            if let Ok(rows) = db.rule_hits_today().await {
                let _ = tx.send(BgMsg::RuleHits(rows));
            }
        });
    }

    pub(crate) fn reload_rule_hits(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            if let Ok(rows) = db.rule_hits_today().await {
                let _ = tx.send(BgMsg::RuleHits(rows));
            }
        });
    }

    /// 出口切换历史：只在点开那张窗口时才查，不每帧查。
    /// 取 300 条是**显示上限**而不是保留期：`ScrollArea::show` 不做虚拟化，行数是逐行排版
    /// 的，窗口开着时每帧都要走完这些行。库里到底还有多少条由表头那句"共 N 条"负责说实话，
    /// 被截掉的部分写"更早的 N 条未在此显示"。
    pub(crate) fn reload_egress_switches(&self) {
        let db = self.db.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let rows = db.recent_egress_switches(300).await.unwrap_or_default();
            let today = db.switch_count_today().await.unwrap_or(0);
            let total = db.switch_history_count().await.unwrap_or(0);
            let _ = tx.send(BgMsg::EgressSwitches(rows, today, total));
        });
    }

    /// 出口切换记录窗口：按本地日分组列出保留期内的全部历史 —— 时间、从谁到谁、为什么。
    pub(crate) fn ui_switch_log(&mut self, ctx: &egui::Context) {
        if !self.show_switch_log {
            return;
        }
        let mut open = true;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        egui::Window::new("出口切换记录")
            .id(egui::Id::new("vpn-parser-switch-log"))
            .open(&mut open)
            .default_width(660.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "共 {} 条 · 今天 {} 次 · 保留 {} 天（重启不丢）",
                        self.switch_total,
                        self.switch_today,
                        crate::db::telemetry::RETENTION_SWITCHES_DAYS
                    ))
                    .strong()
                    .color(theme::text_primary()),
                );
                if self.switch_rows.is_empty() {
                    ui.label("还没有切换记录。手动换出口、自动切换、切换失败都会记在这里。");
                    return;
                }
                let hidden = (self.switch_total - self.switch_rows.len() as i64).max(0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(340.0)
                    .show(ui, |ui| {
                        if hidden > 0 {
                            ui.label(
                                RichText::new(format!("更早的 {hidden} 条未在此显示。"))
                                    .small()
                                    .color(theme::muted()),
                            );
                        }
                        // 行是按 id 倒序取来的，同一个本地日必然连成一段，所以
                        // 只需要"和上一条日期不同就插一条日头"，不用分桶。
                        let mut current_day = String::new();
                        for row in &self.switch_rows {
                            let day = crate::db::telemetry::switch_day(&row.at);
                            if day != current_day {
                                current_day = day.clone();
                                let label = if day == today {
                                    format!("{day} · 今天")
                                } else {
                                    day.clone()
                                };
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(label)
                                        .strong()
                                        .small()
                                        .color(theme::text_secondary()),
                                );
                            }
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(crate::gui::widgets::clock_of(&row.at))
                                        .monospace()
                                        .small()
                                        .color(theme::muted()),
                                );
                                let from = row.from_name.clone().unwrap_or_else(|| "—".into());
                                // 空 to_node 是"判定要切但一个替补都没找到"，
                                // 写成"—"会被当成显示不全。
                                let to = match row.to_name.as_deref() {
                                    Some(n) if !n.is_empty() => n.to_string(),
                                    _ => match row.to_node.as_deref() {
                                        Some(id) if !id.is_empty() => id.to_string(),
                                        _ => "没切成".into(),
                                    },
                                };
                                ui.label(format!("{from} → {to}"));
                                ui.label(
                                    RichText::new(&row.reason)
                                        .small()
                                        .color(theme::text_secondary()),
                                );
                                if let Some(ms) = row.latency_ms {
                                    ui.label(
                                        RichText::new(format!("{ms} ms"))
                                            .small()
                                            .color(theme::muted()),
                                    );
                                }
                            });
                        }
                    });
            });
        self.show_switch_log = open;
    }
}

use super::{BgMsg, VpnApp};
use crate::gui::types::KernelLatency;
use crate::proxy::{ProxyState, URL_TEST_GROUP};
use std::collections::HashMap;

impl VpnApp {
    /// 把内核 url-test 的成员延迟喂给调度器：两边共用一份延迟口径，app 层不再自己
    /// 逐个探测。配置里的成员名可能带 ` #2` 去重后缀，先剥掉再按节点名匹配。
    pub(crate) fn feed_router_kernel_latency(&mut self, latencies: Vec<KernelLatency>) {
        if latencies.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let mut delay_by_name: HashMap<String, u64> = HashMap::new();
        for item in &latencies {
            if item.delay_ms == 0 {
                continue;
            }
            let base = item
                .name
                .split(" #")
                .next()
                .unwrap_or(item.name.as_str())
                .to_owned();
            delay_by_name.insert(base, item.delay_ms);
        }
        let updates: Vec<(String, u64)> = self
            .proxy_nodes
            .iter()
            .chain(self.nodes.iter())
            .filter_map(|node| {
                delay_by_name
                    .get(&node.name)
                    .map(|delay| (node.id.clone(), *delay))
            })
            .collect();
        for (id, delay) in updates {
            self.router.note_kernel_latency(&id, delay, now);
        }
    }

    /// 让内核把某个组切到指定成员：`PUT /proxies/{group}`，不重写配置、不掐连接。

    pub(crate) fn check_failover(&mut self) {
        // 探测本身不受 `auto_failover` 影响：表头要说实话，就必须一直有实测在跑。
        // 那个开关只决定"测出问题之后要不要自动换出口"（见 handle_failover_probe）。
        if self.proxy_busy {
            return;
        }
        if !matches!(
            self.proxy_status.state,
            ProxyState::Running | ProxyState::Degraded
        ) {
            return;
        }
        let Some(node_id) = self.selected_proxy_node_id.clone() else {
            return;
        };
        // 动态自适应保活频率：
        // 1. 若开启了 auto_failover 且当前有失败累积（streak > 0），以 5 秒高频快速复核；
        // 2. 若开启了 auto_failover 且正常，以 15 秒心跳保活；
        // 3. 若未开启 auto_failover（仅表头展示），以 45 秒低频运行节约资源。
        let probe_interval = if self.auto_failover {
            if self.health_fail_streak > 0 {
                std::time::Duration::from_secs(5)
            } else {
                std::time::Duration::from_secs(15)
            }
        } else {
            std::time::Duration::from_secs(45)
        };
        if self
            .failover_probe_at
            .is_some_and(|at| at.elapsed() < probe_interval)
        {
            return;
        }
        // 先占位，避免结果回来前重复探测。
        self.failover_probe_at = Some(std::time::Instant::now());
        self.sync_router_config();
        self.seed_router_candidates();
        self.router.set_active(&node_id);
        let node_name = self
            .selected_proxy_node_name
            .clone()
            .unwrap_or_else(|| node_id.clone());
        let manager = self.proxy_manager.clone();
        let tx = self.tx.clone();
        let timeout = std::time::Duration::from_millis(
            (self.settings.failover_latency_ms as u64 * 2).clamp(3_000, 8_000),
        );
        self.rt.spawn(async move {
            let (ok, latency_ms) = manager.probe_health(timeout).await;
            let _ = tx.send(BgMsg::FailoverProbe {
                node_id,
                node_name,
                ok,
                latency_ms,
            });
        });
    }

    pub(crate) fn handle_failover_probe(
        &mut self,
        node_id: String,
        node_name: String,
        ok: bool,
        latency_ms: u64,
    ) {
        // 探测期间用户可能已经手动换节点，旧结果直接丢弃。
        if self.selected_proxy_node_id.as_deref() != Some(node_id.as_str()) {
            return;
        }
        // 每一次实测都落库并刷回界面 —— 表头那句"已连通"从此只信这条，不信 ProxyState。
        let db = self.db.clone();
        let tx = self.tx.clone();
        let probe_node_id = node_id.clone();
        let probe_node_name = node_name.clone();
        self.rt.spawn(async move {
            let _ = db
                .record_egress_health(
                    ok,
                    Some(latency_ms),
                    if ok {
                        None
                    } else {
                        Some("实测未在时限内取到 204")
                    },
                    Some(&probe_node_id),
                    Some(&probe_node_name),
                    "periodic",
                )
                .await;
            let streak = db.egress_fail_streak().await.unwrap_or(0);
            let rows = db
                .egress_health_since(chrono::Utc::now() - chrono::Duration::hours(24))
                .await
                .unwrap_or_default();
            let _ = tx.send(BgMsg::EgressHealth(rows, streak));
        });
        if !self.auto_failover {
            // 只测不换：开关关掉时，健康数据照记，出口绝不自动被换走。
            return;
        }
        self.sync_router_config();
        // url-test 开着时延迟以内核 history 为准，这次探测只贡献"通/不通"这一个信号
        // （内核只知道它自己那条测试路，不知道穿过代理的端到端请求成不成）。
        let latency_input = if self.proxy_config.url_test {
            None
        } else {
            Some(latency_ms)
        };
        let decision = self.router.observe(
            &node_id,
            crate::router::Sample {
                ok,
                latency_ms: latency_input,
                download_kbps: None,
            },
            std::time::Instant::now(),
        );
        self.apply_router_decision(&node_id, &node_name, decision);
    }

    /// 把调度器的判定翻译成界面文案 + 实际切换动作。
    pub(crate) fn apply_router_decision(
        &mut self,
        node_id: &str,
        node_name: &str,
        decision: crate::router::Decision,
    ) {
        use crate::router::Decision;
        match decision {
            Decision::Stay => {}
            Decision::BreakerOpened { reason, .. } => {
                self.failover_last_reason = Some(format!("{node_name} {reason}，正在寻找替补节点"));
                self.status = self
                    .failover_last_reason
                    .clone()
                    .unwrap_or_else(|| "代理节点被拉黑".into());
                if self.proxy_config.url_test {
                    self.hand_back_to_kernel(node_name, &reason);
                } else {
                    self.begin_failover_switch(node_id, None, "当前节点已被断路器拉黑");
                }
            }
            Decision::SwitchTo {
                node_id: target,
                reason,
            } => {
                if self.proxy_config.url_test {
                    self.hand_back_to_kernel(node_name, &reason);
                    return;
                }
                // 调度器给的目标也要先过一遍 TCP 复检，复检过了才真切。
                self.begin_failover_switch(node_id, Some(&target), &reason);
            }
        }
    }

    /// A：url-test 开着时 app 层只负责"判定"，换节点交给内核 —— 否则每判一次就要
    /// 重写配置 + 热重载，把已经建立的连接全掐掉。坏节点由断路器拉黑，并在下一次
    /// 候选刷新时从 url-test 里摘掉；这里只做一件不断线的事：如果 PROXY 被锁在某个
    /// 具体节点上（内核就没法自己换），把它指回「自动选择」。
    pub(crate) fn hand_back_to_kernel(&mut self, node_name: &str, reason: &str) {
        self.failover_switches += 1;
        let locked_elsewhere = self
            .kernel_groups
            .iter()
            .find(|group| group.name == "PROXY")
            .and_then(|group| group.now.clone())
            .is_some_and(|now| now != URL_TEST_GROUP);
        self.failover_last_reason = Some(format!(
            "{node_name} {reason} · 已拉黑，下次候选刷新不再入选；{}",
            if locked_elsewhere {
                "PROXY 被锁在具体节点上，已交还「自动选择」"
            } else {
                "换节点交给内核"
            }
        ));
        self.status = self.failover_last_reason.clone().unwrap_or_default();
        if locked_elsewhere {
            self.select_kernel_member("PROXY".to_owned(), URL_TEST_GROUP.to_owned());
        }
    }

    /// 自动切换：先挑替补、再对替补做一次 TCP 复检，复检过了才真切换。
    ///
    /// 为什么非要复检：库里的 `latency_ok` 是"上一次测速时通"，可能已经是几小时前。
    /// 2026-10-07 那次自动切换就是照着 08:24 的 90ms 记录切进了一个端口早就不监听的
    /// socks5，结果整个境外不通。控制台那批候选一个都不剩时，还会现查一次库里最新测通的。
    pub(crate) fn begin_failover_switch(
        &mut self,
        current_id: &str,
        preferred: Option<&str>,
        reason: &str,
    ) {
        if self.failover_searching {
            self.failover_last_reason =
                Some("替补复检还在进行中，这一轮的判定先不重复触发".into());
            return;
        }
        let blacklisted: std::collections::HashSet<String> = self
            .router
            .blacklisted(std::time::Instant::now())
            .into_iter()
            .collect();
        let all: Vec<crate::router::BackupCandidate> = self
            .proxy_nodes
            .iter()
            .map(|node| crate::router::BackupCandidate {
                id: node.id.clone(),
                name: node.name.clone(),
                host: node.server.clone(),
                port: node.port,
                last_latency_ms: node.latency_ms.map(|ms| ms.max(0) as u64),
            })
            .collect();
        let mut shortlist = crate::router::pick_backups(
            &all,
            current_id,
            &blacklisted,
            &self.settings.exclude_keywords,
            8,
        );
        if let Some(want) = preferred {
            if let Some(position) = shortlist.iter().position(|node| node.id == want) {
                let chosen = shortlist.remove(position);
                shortlist.insert(0, chosen);
            }
        }
        self.failover_searching = true;
        self.failover_from = Some((
            current_id.to_string(),
            self.selected_proxy_node_name.clone().unwrap_or_default(),
        ));
        self.status = if shortlist.is_empty() {
            format!("{reason}，控制台候选里没有第二个可用节点，改从库里最新测通的里现查...")
        } else {
            format!("{reason}，正在对 {} 个替补节点做 TCP 复检...", shortlist.len())
        };
        let db = self.db.clone();
        let tx = self.tx.clone();
        let current = current_id.to_string();
        self.rt.spawn(async move {
            let mut pool = shortlist;
            if pool.is_empty() {
                pool = db
                    .top_verified_nodes(12, Some(&current))
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|node| crate::router::BackupCandidate {
                        id: node.id,
                        name: node.name,
                        host: node.server,
                        port: node.port,
                        last_latency_ms: node.latency_ms.map(|ms| ms.max(0) as u64),
                    })
                    .collect();
            }
            let probed = pool.len();
            let outcome = if pool.is_empty() {
                Err("库里也找不出第二个测通过的节点".to_string())
            } else {
                crate::proxy_probe::first_reachable(pool, std::time::Duration::from_millis(2_500))
                    .await
                    .ok_or_else(|| format!("TCP 复检过 {probed} 个候选，没有一个端口连得上"))
            };
            let _ = tx.send(BgMsg::FailoverTarget(outcome));
        });
    }

    pub(crate) fn on_failover_target(&mut self, outcome: Result<(String, String), String>) {
        self.failover_searching = false;
        let from = self.failover_from.take();
        match outcome {
            Ok((id, name)) => {
                self.failover_switches += 1;
                self.failover_last_reason = Some(format!(
                    "复检通过 → 已切到 {name} · 本次运行第 {} 次",
                    self.failover_switches
                ));
                self.status = self.failover_last_reason.clone().unwrap_or_default();
                self.switch_proxy_node(id, name, "自动切换");
            }
            Err(reason) => {
                self.failover_last_reason = Some(format!("判定该换出口，但{reason}"));
                self.status = self.failover_last_reason.clone().unwrap_or_default();
                if let Some((from_id, from_name)) = from {
                    self.record_failover_miss(&from_id, &from_name, &reason);
                }
            }
        }
    }

    /// "该切但没切成"也进切换历史。以前这种只写在一行状态栏里，滑过去就查无实据，
    /// 于是"我开了自动切换为什么没换"只能靠人翻库猜。
    pub(crate) fn record_failover_miss(&mut self, from_id: &str, from_name: &str, reason: &str) {
        let db = self.db.clone();
        let record = crate::db::telemetry::SwitchRecord {
            from_node: Some(from_id.to_string()),
            from_name: Some(from_name.to_string()),
            // 空串而不是 NULL：DAO 用 (to_node, reason) 去重连续相同记录，
            // NULL 参与拼接会永远不相等，抖动时每两分钟刷一条一模一样的。
            to_node: Some(String::new()),
            to_name: None,
            reason: format!("没有可用替补：{reason}"),
            latency_ms: None,
        };
        self.rt.spawn(async move {
            let _ = db.record_egress_switch(&record).await;
        });
    }

    /// 调度器参数跟随系统设置，避免两处真相。
    pub(crate) fn sync_router_config(&mut self) {
        let mut cfg = self.router.cfg.clone();
        cfg.latency_threshold_ms = self.settings.failover_latency_ms.max(200) as u64;
        cfg.bandwidth_threshold_kbps = if self.settings.bandwidth_guard {
            self.settings.bandwidth_min_kbps.max(1) as f64
        } else {
            0.0
        };
        self.router.cfg = cfg;
    }

    /// 用数据库里最近一次测速结果给调度器播种候选，否则刚启动时它谁都不认识。
    pub(crate) fn seed_router_candidates(&mut self) {
        let keywords = self.settings.exclude_keywords.clone();
        let now = std::time::Instant::now();
        let mut rows: Vec<(String, i64)> = self
            .proxy_nodes
            .iter()
            .filter(|node| node.latency_ok == Some(true))
            .filter(|node| !crate::models::name_excluded(&node.name, &keywords))
            .filter_map(|node| node.latency_ms.map(|ms| (node.id.clone(), ms)))
            .collect();
        rows.sort_by_key(|(_, ms)| *ms);
        for (id, ms) in rows.into_iter().take(60) {
            self.router.seed(&id, ms.max(0) as u64, true, now);
        }
    }

}
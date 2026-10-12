//! 节点选路调度器：EWMA 延迟 + 断路器 + 综合打分 + 滞后切换。
//!
//! 取代原来"连续 3 次超阈值就切、切完 3 分钟冷却"的朴素判断。设计要点：
//!
//! - **EWMA**：单次尖峰（一次 GC、一个后台大下载）不该把节点判死，指数加权移动
//!   平均让历史平滑衰减。
//! - **断路器**：连续 `breaker_failures` 次验证失败就把节点拉黑 `breaker_cooldown`，
//!   期间不参与候选也不被选中，避免反复撞同一个死节点。
//! - **打分**：延迟主导，带宽作折扣，失败次数按惩罚项累加，分数越低越好。
//! - **滞后（hysteresis）**：只有当候选明显比当前节点好（默认好 25%）才切，否则
//!   在两个分数接近的节点之间来回跳，每次跳都要断一次连接。
//! - **带宽保护**：可选，当前节点持续低于阈值 KB/s 也按劣化处理。

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// 真实代理握手是否成功。
    pub ok: bool,
    /// 本次实测延迟（ms），`ok == false` 时可以是超时值。
    pub latency_ms: Option<u64>,
    /// 本次采样的下行速率（KB/s），只有跑流量时才有。
    pub download_kbps: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct NodeHealth {
    pub ewma_ms: f64,
    pub ewma_download_kbps: f64,
    pub samples: u32,
    pub consecutive_failures: u32,
    /// 断路器打开的截止时间；`Some` 且未到期表示该节点暂时不可用。
    pub breaker_open_until: Option<Instant>,
    pub last_update: Instant,
}

impl NodeHealth {
    fn new(now: Instant) -> Self {
        Self {
            ewma_ms: 0.0,
            ewma_download_kbps: 0.0,
            samples: 0,
            consecutive_failures: 0,
            breaker_open_until: None,
            last_update: now,
        }
    }

    pub fn breaker_open(&self, now: Instant) -> bool {
        self.breaker_open_until.is_some_and(|until| until > now)
    }

    /// 分数越低越好。单位是"等效毫秒"：带宽每 KB/s 折 0.25ms，每次连续失败罚
    /// 一整段超时时长，断路器打开则直接判为不可选。
    pub fn score(&self, cfg: &RouterConfig, now: Instant) -> Option<f64> {
        if self.breaker_open(now) || self.samples == 0 {
            return None;
        }
        let bandwidth_discount =
            self.ewma_download_kbps.min(cfg.bandwidth_cap_kbps) * cfg.bandwidth_weight;
        Some(
            self.ewma_ms - bandwidth_discount
                + f64::from(self.consecutive_failures) * cfg.failure_penalty_ms,
        )
    }
}

#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// EWMA 平滑系数：新样本权重。0.3 ≈ 记忆最近 3-4 次。
    pub alpha: f64,
    /// 延迟劣化阈值（ms），超过即算一次"差样本"。
    pub latency_threshold_ms: u64,
    /// 连续失败几次开断路。
    pub breaker_failures: u32,
    /// 断路冷却时长。
    pub breaker_cooldown: Duration,
    /// 候选必须比当前节点好多少才切（0.25 = 便宜 25% 以上）。
    pub hysteresis_ratio: f64,
    /// 两次自动切换的最小间隔。
    pub min_switch_interval: Duration,
    /// 连续几次"EWMA 已越过阈值"才真的切：单次尖峰就算把 EWMA 抬上去也不动，
    /// 60 秒一次探测时相当于要求持续 2 分钟劣化。
    pub degraded_streak: u32,
    /// 带宽保护：低于该 KB/s 连续 N 次算劣化；`threshold == 0` 表示关闭。
    pub bandwidth_threshold_kbps: f64,
    pub bandwidth_bad_streak: u32,
    /// 打分时带宽折扣的封顶与权重（避免大下载节点无限加分）。
    pub bandwidth_cap_kbps: f64,
    pub bandwidth_weight: f64,
    pub failure_penalty_ms: f64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            alpha: 0.3,
            latency_threshold_ms: 1500,
            breaker_failures: 2,
            breaker_cooldown: Duration::from_secs(300),
            hysteresis_ratio: 0.75,
            min_switch_interval: Duration::from_secs(120),
            degraded_streak: 2,
            bandwidth_threshold_kbps: 0.0,
            bandwidth_bad_streak: 3,
            bandwidth_cap_kbps: 2048.0,
            bandwidth_weight: 0.25,
            failure_penalty_ms: 3000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// 留在当前节点。
    Stay,
    /// 切到指定节点，附带可读原因。
    SwitchTo { node_id: String, reason: String },
    /// 当前节点断路器刚打开。
    BreakerOpened { node_id: String, reason: String },
}

#[derive(Debug, Default)]
pub struct Router {
    states: HashMap<String, NodeHealth>,
    bandwidth_streak: HashMap<String, u32>,
    degraded_streaks: HashMap<String, u32>,
    active: Option<String>,
    last_switch: Option<Instant>,
    pub cfg: RouterConfig,
}

impl Router {
    pub fn new(cfg: RouterConfig) -> Self {
        Self {
            states: HashMap::new(),
            bandwidth_streak: HashMap::new(),
            degraded_streaks: HashMap::new(),
            active: None,
            last_switch: None,
            cfg,
        }
    }

    pub fn set_active(&mut self, node_id: &str) {
        self.active = Some(node_id.to_owned());
    }

    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn health(&self, node_id: &str) -> Option<&NodeHealth> {
        self.states.get(node_id)
    }

    /// 现在被断路器拉黑的节点。url-test 开着时，app 层不当场抢方向盘换节点，而是
    /// 把这些名字从**下一批候选**里摘掉（见 `Router` 与界面 `prime_url_test_candidates`）。
    pub fn blacklisted(&self, now: Instant) -> Vec<String> {
        self.states
            .iter()
            .filter(|(_, state)| state.breaker_open(now))
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn entry(&mut self, node_id: &str, now: Instant) -> &mut NodeHealth {
        self.states
            .entry(node_id.to_owned())
            .or_insert_with(|| NodeHealth::new(now))
    }

    /// 喂一次当前节点的实测样本，返回该不该切换。
    pub fn observe(&mut self, node_id: &str, sample: Sample, now: Instant) -> Decision {
        let alpha = self.cfg.alpha;
        let breaker_failures = self.cfg.breaker_failures;

        {
            let state = self.entry(node_id, now);
            state.last_update = now;
            if sample.ok {
                state.consecutive_failures = 0;
                state.breaker_open_until = None;
                if let Some(ms) = sample.latency_ms {
                    state.ewma_ms = if state.samples == 0 {
                        ms as f64
                    } else {
                        alpha * ms as f64 + (1.0 - alpha) * state.ewma_ms
                    };
                }
            } else {
                state.consecutive_failures += 1;
                // 失败样本也用超时值抬一下 EWMA，否则一个"完全不通"的节点分数反而
                // 比慢节点更好看。
                if let Some(ms) = sample.latency_ms {
                    state.ewma_ms = if state.samples == 0 {
                        ms as f64
                    } else {
                        alpha * ms as f64 + (1.0 - alpha) * state.ewma_ms
                    };
                }
            }
            state.samples += 1;
        }

        // 劣化计数看「这一次原始样本是否超阈值」，评分看 EWMA。分开是有原因的：
        // 一次 6s 尖峰会把 EWMA 抬到阈值以上且迟迟落不回来，如果计数也看 EWMA，
        // 后面几个正常样本照样被算成劣化，尖峰就被延后放大成一次切换。
        let raw_over_threshold = sample.ok
            && sample
                .latency_ms
                .is_some_and(|ms| ms > self.cfg.latency_threshold_ms);
        let streak = if raw_over_threshold {
            self.degraded_streaks.get(node_id).copied().unwrap_or(0) + 1
        } else {
            0
        };
        self.degraded_streaks.insert(node_id.to_owned(), streak);
        let degraded_by_latency = streak >= self.cfg.degraded_streak;

        if let Some(kbps) = sample.download_kbps {
            let state = self.entry(node_id, now);
            state.ewma_download_kbps = if state.ewma_download_kbps == 0.0 {
                kbps
            } else {
                alpha * kbps + (1.0 - alpha) * state.ewma_download_kbps
            };
        }

        let opened = {
            let cooldown = self.cfg.breaker_cooldown;
            let state = self.entry(node_id, now);
            if state.consecutive_failures >= breaker_failures
                && !state.breaker_open(now + Duration::from_secs(1))
            {
                state.breaker_open_until = Some(now + cooldown);
                true
            } else {
                false
            }
        };
        if opened {
            let reason = format!(
                "连续 {} 次验证失败，拉黑 {} 分钟",
                breaker_failures,
                self.cfg.breaker_cooldown.as_secs() / 60
            );
            self.last_switch = Some(now);
            return Decision::BreakerOpened {
                node_id: node_id.to_owned(),
                reason,
            };
        }

        let slow = degraded_by_latency || self.bandwidth_degraded(node_id, now);
        if !slow {
            return Decision::Stay;
        }

        if self
            .last_switch
            .is_some_and(|at| now.duration_since(at) < self.cfg.min_switch_interval)
        {
            return Decision::Stay;
        }

        let Some((target, target_score)) = self.best_candidate(node_id, now) else {
            return Decision::Stay;
        };
        let cfg = self.cfg.clone();
        let current_score = self
            .states
            .get(node_id)
            .and_then(|state| state.score(&cfg, now))
            .unwrap_or(f64::MAX);
        let ratio = self.cfg.hysteresis_ratio;
        if target_score >= current_score * ratio {
            return Decision::Stay;
        }

        self.last_switch = Some(now);
        self.bandwidth_streak.insert(node_id.to_owned(), 0);
        let detail = if degraded_by_latency {
            format!(
                "延迟 EWMA {:.0}ms 超过阈值 {}ms",
                current_score, self.cfg.latency_threshold_ms
            )
        } else {
            format!("下行持续低于 {:.0} KB/s", self.cfg.bandwidth_threshold_kbps)
        };
        Decision::SwitchTo {
            node_id: target,
            reason: format!("{detail}，评分 {current_score:.0} → {target_score:.0}"),
        }
    }

    fn bandwidth_degraded(&mut self, node_id: &str, now: Instant) -> bool {
        if self.cfg.bandwidth_threshold_kbps <= 0.0 {
            return false;
        }
        let ewma = self
            .health(node_id)
            .map(|state| state.ewma_download_kbps)
            .unwrap_or(0.0);
        // 只有确实采到过带宽样本才参与判定，否则空闲时会被误判成"慢"。
        let measured = self
            .health(node_id)
            .is_some_and(|state| state.ewma_download_kbps > 0.0);
        let streak = if measured && ewma < self.cfg.bandwidth_threshold_kbps {
            let next = self.bandwidth_streak.get(node_id).copied().unwrap_or(0) + 1;
            next
        } else {
            0
        };
        self.bandwidth_streak.insert(node_id.to_owned(), streak);
        let _ = now;
        streak >= self.cfg.bandwidth_bad_streak
    }

    /// 除 `exclude` 外、断路器没打开的节点里分数最低的那个。
    pub fn best_candidate(&self, exclude: &str, now: Instant) -> Option<(String, f64)> {
        self.states
            .iter()
            .filter(|(id, _)| id.as_str() != exclude)
            .filter_map(|(id, state)| state.score(&self.cfg, now).map(|score| (id.clone(), score)))
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// 用数据库里最近一次测速结果给候选打底：调度器刚起来时没有任何历史样本，
    /// 不播种就没有候选可切。已有实测样本的节点不覆盖。
    pub fn seed(&mut self, node_id: &str, latency_ms: u64, ok: bool, now: Instant) {
        let state = self.entry(node_id, now);
        if state.samples > 0 {
            return;
        }
        state.ewma_ms = latency_ms as f64;
        state.consecutive_failures = if ok { 0 } else { 1 };
        state.samples = 1;
    }

    /// 内核 url-test 每 `interval` 秒自己测一轮成员延迟，那份历史是现成的。拿它刷新
    /// 候选的 EWMA，让 app 层和内核用同一份延迟口径；不动失败计数 —— "端到端通不通"
    /// 仍然只有我们那次 generate_204 探测知道。
    pub fn note_kernel_latency(&mut self, node_id: &str, latency_ms: u64, now: Instant) {
        if latency_ms == 0 {
            // mihomo 对没测出来的成员写 0，不是"0ms 飞快"。
            return;
        }
        let alpha = self.cfg.alpha;
        let state = self.entry(node_id, now);
        state.ewma_ms = if state.samples == 0 {
            latency_ms as f64
        } else {
            alpha * latency_ms as f64 + (1.0 - alpha) * state.ewma_ms
        };
        state.samples = state.samples.max(1);
        state.last_update = now;
    }

    /// 供界面展示：按分数排序的候选（分数越低越好）。
    pub fn ranking(&self, now: Instant) -> Vec<(String, Option<f64>, NodeHealth)> {
        let mut rows: Vec<(String, Option<f64>, NodeHealth)> = self
            .states
            .iter()
            .map(|(id, state)| (id.clone(), state.score(&self.cfg, now), state.clone()))
            .collect();
        rows.sort_by(|a, b| {
            a.1.unwrap_or(f64::MAX)
                .partial_cmp(&b.1.unwrap_or(f64::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ok: bool, ms: u64) -> Sample {
        Sample {
            ok,
            latency_ms: Some(ms),
            download_kbps: None,
        }
    }

    fn router() -> Router {
        Router::new(RouterConfig {
            latency_threshold_ms: 600,
            min_switch_interval: Duration::from_secs(0),
            ..Default::default()
        })
    }

    #[test]
    fn degraded_streak_is_required_before_switching() {
        let mut r = router();
        let t0 = Instant::now();
        for _ in 0..4 {
            r.observe("current", sample(true, 300), t0);
            r.observe("better", sample(true, 200), t0);
        }
        // 第一次尖峰：EWMA 越线但只算 1 次劣化，不该切。
        assert!(matches!(
            r.observe("current", sample(true, 6000), t0),
            Decision::Stay
        ));
        // 中间恢复正常两次，计数被清零，再来一次尖峰仍然不该切。
        r.observe("current", sample(true, 300), t0);
        r.observe("current", sample(true, 300), t0);
        assert!(matches!(
            r.observe("current", sample(true, 6000), t0),
            Decision::Stay
        ));
    }

    #[test]
    fn one_spike_never_switches_even_when_a_better_candidate_exists() {
        let mut r = router();
        let t0 = Instant::now();
        for _ in 0..4 {
            r.observe("current", sample(true, 300), t0);
            r.observe("better", sample(true, 200), t0);
        }
        // 一次 6s 尖峰：better 分数本来就比 current 好，但 EWMA 还没越过阈值，
        // 不该切。
        assert!(matches!(
            r.observe("current", sample(true, 6000), t0),
            Decision::Stay
        ));
        // 持续劣化（每次都慢）才会真的切。
        for _ in 0..4 {
            let decision = r.observe("current", sample(true, 2000), t0);
            if let Decision::SwitchTo { node_id, .. } = decision {
                assert_eq!(node_id, "better");
                return;
            }
        }
        panic!("持续劣化后仍然没有切换");
    }

    #[test]
    fn one_spike_raises_the_score_but_recovers_without_a_target() {
        let mut r = router();
        let t0 = Instant::now();
        for _ in 0..5 {
            assert!(matches!(
                r.observe("a", sample(true, 200), t0),
                Decision::Stay
            ));
        }
        // EWMA 不是"完全免疫尖峰"：一次 5s 会把分数抬到阈值以上。但只有存在明显
        // 更好的候选时才会真的切，否则就是 Stay。
        assert!(matches!(
            r.observe("a", sample(true, 5000), t0),
            Decision::Stay
        ));
        assert!(r.health("a").unwrap().ewma_ms > 600.0);
        // 之后连续正常样本要能自己回到阈值以下（这就是 EWMA 的记忆长度）。
        for _ in 0..6 {
            assert!(matches!(
                r.observe("a", sample(true, 200), t0),
                Decision::Stay
            ));
        }
        assert!(
            r.health("a").unwrap().ewma_ms < 600.0,
            "ewma={}",
            r.health("a").unwrap().ewma_ms
        );
    }

    #[test]
    fn two_failures_open_the_breaker_and_blacklist_the_node() {
        let mut r = router();
        let t0 = Instant::now();
        r.observe("bad", sample(false, 8000), t0);
        let decision = r.observe("bad", sample(false, 8000), t0);
        assert!(matches!(decision, Decision::BreakerOpened { .. }));
        assert!(r.health("bad").unwrap().breaker_open(t0));
        // 断路节点不会成为候选。
        assert!(r.best_candidate("other", t0).is_none());
    }

    #[test]
    fn hysteresis_blocks_switching_to_a_similarly_good_node() {
        let mut r = router();
        let t0 = Instant::now();
        for _ in 0..4 {
            r.observe("current", sample(true, 900), t0);
            r.observe("similar", sample(true, 850), t0);
        }
        // current 已经劣化（900 > 600），但 similar 只好一点，不该切。
        let decision = r.observe("current", sample(true, 900), t0);
        assert!(matches!(decision, Decision::Stay), "{decision:?}");
    }

    #[test]
    fn switches_when_a_candidate_is_much_better() {
        let mut r = router();
        let t0 = Instant::now();
        for _ in 0..4 {
            r.observe("current", sample(true, 1200), t0);
            r.observe("great", sample(true, 120), t0);
        }
        let decision = r.observe("current", sample(true, 1200), t0);
        match decision {
            Decision::SwitchTo { node_id, reason } => {
                assert_eq!(node_id, "great");
                assert!(reason.contains("延迟 EWMA"));
            }
            other => panic!("expected switch, got {other:?}"),
        }
    }

    #[test]
    fn min_switch_interval_suppresses_flapping() {
        let mut r = Router::new(RouterConfig {
            latency_threshold_ms: 600,
            min_switch_interval: Duration::from_secs(120),
            ..Default::default()
        });
        let t0 = Instant::now();
        r.observe("great", sample(true, 100), t0);
        // 第 1 次劣化只计数，第 2 次才真的切。
        assert!(matches!(
            r.observe("current", sample(true, 1200), t0),
            Decision::Stay
        ));
        assert!(matches!(
            r.observe("current", sample(true, 1200), t0),
            Decision::SwitchTo { .. }
        ));
        // 刚切过一次，紧接着又劣化也不该再切（否则一次抖动就连跳两个节点）。
        assert!(matches!(
            r.observe("current", sample(true, 1200), t0),
            Decision::Stay
        ));
    }

    #[test]
    fn bandwidth_guard_triggers_switch_when_enabled() {
        let mut r = Router::new(RouterConfig {
            latency_threshold_ms: 5000,
            min_switch_interval: Duration::from_secs(0),
            bandwidth_threshold_kbps: 200.0,
            bandwidth_bad_streak: 2,
            ..Default::default()
        });
        let t0 = Instant::now();
        let slow = Sample {
            ok: true,
            latency_ms: Some(150),
            download_kbps: Some(20.0),
        };
        let fast = Sample {
            ok: true,
            latency_ms: Some(200),
            download_kbps: Some(1500.0),
        };
        for _ in 0..3 {
            r.observe("slow", slow, t0);
            r.observe("fast", fast, t0);
        }
        let decision = r.observe("slow", slow, t0);
        match decision {
            Decision::SwitchTo { node_id, reason } => {
                assert_eq!(node_id, "fast");
                assert!(reason.contains("下行"), "{reason}");
            }
            other => panic!("expected bandwidth switch, got {other:?}"),
        }
    }

    #[test]
    fn idle_nodes_are_not_judged_by_bandwidth() {
        let mut r = Router::new(RouterConfig {
            latency_threshold_ms: 5000,
            min_switch_interval: Duration::from_secs(0),
            bandwidth_threshold_kbps: 200.0,
            bandwidth_bad_streak: 2,
            ..Default::default()
        });
        let t0 = Instant::now();
        for _ in 0..6 {
            // 只有延迟样本、没有带宽样本：不该被"带宽不足"判定拖去切换。
            assert!(matches!(
                r.observe("idle", sample(true, 100), t0),
                Decision::Stay
            ));
        }
    }

    #[test]
    fn failure_penalty_makes_an_unstable_node_lose_to_a_stable_one() {
        let mut r = router();
        let t0 = Instant::now();
        for _ in 0..3 {
            r.observe("jittery", sample(true, 300), t0);
        }
        r.observe("jittery", sample(false, 1000), t0);
        r.observe("jittery", sample(true, 300), t0);
        r.observe("steady", sample(true, 320), t0);
        let jittery = r.health("jittery").unwrap().score(&r.cfg, t0).unwrap();
        let steady = r.health("steady").unwrap().score(&r.cfg, t0).unwrap();
        assert!(jittery > steady, "jittery={jittery} steady={steady}");
    }

    /// url-test 开着时，候选的延迟来自内核 history（app 层不再自己逐个探测）。
    /// 这条路径以前完全没有数据，调度器等于瞎子。
    #[test]
    fn kernel_latency_makes_unmeasured_nodes_rankable() {
        let mut r = router();
        let t0 = Instant::now();
        r.note_kernel_latency("kernel-only", 120, t0);
        assert_eq!(r.health("kernel-only").map(|s| s.ewma_ms), Some(120.0));
        assert!(r.health("kernel-only").unwrap().score(&r.cfg, t0).is_some());

        // 内核没测出来的成员写 delay=0，不能被当成"0ms 的超快节点"。
        r.note_kernel_latency("never-measured", 0, t0);
        assert!(r.health("never-measured").is_none());

        // 已有 EWMA 按 alpha 混合，且失败计数不受内核数据影响。
        r.observe("flaky", sample(false, 5000), t0);
        let failures = r.health("flaky").map(|s| s.consecutive_failures);
        r.note_kernel_latency("flaky", 100, t0 + Duration::from_secs(1));
        let state = r.health("flaky").unwrap();
        assert_eq!(state.consecutive_failures, failures.unwrap());
        assert!(
            state.ewma_ms > 100.0 && state.ewma_ms < 5000.0,
            "ewma={}",
            state.ewma_ms
        );
    }

    /// url-test 开着时，app 层靠这份名单把坏节点从下一批候选里摘掉。
    #[test]
    fn blacklisted_lists_only_nodes_with_an_open_breaker() {
        let mut r = router();
        let t0 = Instant::now();
        assert!(r.blacklisted(t0).is_empty());
        r.observe("dead", sample(false, 5000), t0);
        assert!(r.blacklisted(t0).is_empty(), "只失败一次还不该开断路");
        r.observe("dead", sample(false, 5000), t0);
        assert_eq!(r.blacklisted(t0), vec!["dead".to_string()]);
        // 冷却结束后必须放行，否则它永远进不了候选。
        assert!(r
            .blacklisted(t0 + r.cfg.breaker_cooldown + Duration::from_secs(1))
            .is_empty());
    }
}

/// 自动切换前的替补复检结果：`Ok` 里是复检过、端口连得上的节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupCandidate {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// 上一次测速延迟；只用来排序，不当成"现在还活着"的证据。
    pub last_latency_ms: Option<u64>,
}

/// 自动切换的替补挑选：先按上次数据把顺序排出来，真正的"能不能连"交给
/// `proxy_probe::first_reachable` 的 TCP 复检。理由很实在 —— 2026-10-07 那次
/// 自动切换就是照着 9 小时前的 90ms 记录切进了一个端口已经没人监听的节点。
pub fn pick_backups(
    all: &[BackupCandidate],
    current_id: &str,
    blacklisted: &std::collections::HashSet<String>,
    keywords: &str,
    limit: usize,
) -> Vec<BackupCandidate> {
    let mut pool: Vec<&BackupCandidate> = all
        .iter()
        .filter(|node| node.id != current_id)
        .filter(|node| !blacklisted.contains(&node.id))
        .filter(|node| !crate::models::name_excluded(&node.name, keywords))
        .filter(|node| node.last_latency_ms.is_some())
        .collect();
    pool.sort_by_key(|node| node.last_latency_ms.unwrap_or(u64::MAX));
    pool.into_iter().take(limit).cloned().collect()
}

#[cfg(test)]
mod backup_tests {
    use super::*;
    use std::collections::HashSet;

    fn node(id: &str, name: &str, ms: Option<u64>) -> BackupCandidate {
        BackupCandidate {
            id: id.to_string(),
            name: name.to_string(),
            host: format!("10.0.0.{}", id.as_bytes()[0]),
            port: 443,
            last_latency_ms: ms,
        }
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn backups_are_ordered_by_last_latency_and_skip_the_current_node() {
        let all = vec![
            node("c", "当前", Some(90)),
            node("b", "中等", Some(300)),
            node("a", "最快", Some(120)),
        ];
        let picked = pick_backups(&all, "c", &none(), "", 8);
        assert_eq!(
            picked.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"],
            "当前节点要排掉，顺序按上次延迟"
        );
    }

    #[test]
    fn blacklisted_and_untested_and_excluded_names_never_become_backups() {
        let all = vec![
            node("x", "坏的", Some(100)),
            node("y", "没测过", None),
            node("z", "到期 限速", Some(110)),
            node("w", "能用", Some(500)),
        ];
        let mut black = HashSet::new();
        black.insert("x".to_string());
        let picked = pick_backups(&all, "cur", &black, "到期,限速", 8);
        assert_eq!(
            picked.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            vec!["w"],
            "拉黑的、没测过的、命中排除词的都不该当替补"
        );
    }

    #[test]
    fn limit_caps_the_shortlist_and_an_empty_pool_is_honest() {
        let all: Vec<BackupCandidate> = (0..20u64).map(|i| node(&format!("n{i}"), &format!("N{i}"), Some(i))).collect();
        assert_eq!(pick_backups(&all, "cur", &none(), "", 6).len(), 6);
        assert!(pick_backups(&[], "cur", &none(), "", 8).is_empty());
        // 只剩当前节点一个候选时，必须回空 —— 这就是"没切成"那种情况。
        assert!(pick_backups(&[node("c", "只此一家", Some(1))], "c", &none(), "", 8).is_empty());
    }
}

use crate::collector::Collector;
use crate::db::Database;
use crate::models::AppSettings;
use crate::speedtest::{SpeedTestOptions, SpeedTester, SpeedtestIntensity};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// 测通的节点低于这个数，就认为"故障转移没有替补可挑"了 —— 自动切换找候选就是从
/// 这批里捞（`top_verified_nodes` 的条件是 `latency_ok = 1`）。
const SPARSE_VERIFIED_FLOOR: i64 = 20;
/// 两轮自动补测之间至少隔这么久，共用 `last_auto_speedtest_at` 这个时间戳。
const REFILL_COOLDOWN_MIN: i64 = 30;
/// 自动补测的并发上限：这是后台活，不许把用户的带宽抢走（手动测速仍按设置里那个数）。
const REFILL_CONCURRENCY: u32 = 8;

/// 该不该补一轮全量测速。四个条件都成立才跑：开关开着、库里真的有节点可测、
/// 测通数低于地板、距上次自动测速（或从来没测过）够久。
/// 纯函数：判定和副作用分开，这条规则才测得动。
fn refill_needed(settings: &AppSettings, total: i64, verified: i64, now: i64) -> bool {
    if !settings.speedtest_auto_refill || total == 0 || verified >= SPARSE_VERIFIED_FLOOR {
        return false;
    }
    match settings.last_auto_speedtest_at {
        None => true,
        Some(last) => now - last.timestamp() >= REFILL_COOLDOWN_MIN * 60,
    }
}

/// 补测用的选项：并发压低、不测下载吞吐（那才是真吃带宽的一段），其余跟随设置。
fn refill_opts(settings: &AppSettings) -> SpeedTestOptions {
    SpeedTestOptions {
        concurrency: REFILL_CONCURRENCY,
        timeout_ms: settings.speedtest_timeout_ms,
        measure_download: false,
        download_test_ms: settings.download_test_ms,
        intensity: SpeedtestIntensity::Standard,
        parallel_kernels: settings.speedtest_parallel_kernels,
        persistent_kernel: settings.speedtest_persistent_kernel,
        reuse_verified_minutes: settings.speedtest_reuse_minutes,
        conditional_isolation: settings.speedtest_conditional_isolation,
        download_top_n: settings.speedtest_download_top_n,
    }
}

/// Background scheduler for auto-fetch and optional auto-speedtest.
#[derive(Clone)]
pub struct Scheduler {
    db: Database,
    collector: Collector,
    speedtester: SpeedTester,
    running: Arc<AtomicBool>,
}

impl Scheduler {
    pub fn new(db: Database, collector: Collector, speedtester: SpeedTester) -> Self {
        Self {
            db,
            collector,
            speedtester,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn run_loop(self) {
        // Check every 30s whether it's time to run.
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Skip immediate first tick fire.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            if self.running.load(Ordering::Relaxed) {
                continue;
            }

            let settings = match self.db.get_settings().await {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "scheduler: load settings failed");
                    continue;
                }
            };

            let now = chrono::Utc::now().timestamp();

            // 替补池空了就先补一轮。这条刻意放在下面 `auto_fetch_enabled` 那个 early
            // continue **之前**：它服务的是故障转移，不是订阅更新，不该被拉取开关门掉。
            if settings.speedtest_auto_refill {
                let counts = self.db.node_verification_counts().await;
                match counts {
                    Ok((total, verified)) if refill_needed(&settings, total, verified, now) => {
                        self.running.store(true, Ordering::Relaxed);
                        info!(
                            verified,
                            total,
                            floor = SPARSE_VERIFIED_FLOOR,
                            concurrency = REFILL_CONCURRENCY,
                            "scheduler: 测通节点太少，后台自动补一轮全量测速"
                        );
                        match self.speedtester.test_all_sources(refill_opts(&settings)).await {
                            Ok(sum) => {
                                info!(
                                    ok = sum.ok,
                                    fail = sum.fail,
                                    avg = ?sum.avg_ms,
                                    "scheduler: 自动补测完成"
                                );
                            }
                            Err(e) => warn!(error = %e, "scheduler: 自动补测失败"),
                        }
                        // 成败都要记这个时间戳。2026-10-08 实测：环境坏掉的时候这一条
                        // 每次都返回 Err（测速内核起不来），时间戳一直是 NULL，
                        // `refill_needed` 就永远成立 —— 于是每 30 秒无条件重跑一轮全量
                        // 测速，跑几个小时：分片库的 -shm 每隔半分钟被整批重建，线程数
                        // 从 31 涨到 63 停不下来。冷却是给失败用的，不是只给成功用的。
                        let _ = self.db.touch_auto_speedtest().await;
                        self.running.store(false, Ordering::Relaxed);
                    }
                    Ok(_) => {}
                    Err(e) => warn!(error = %e, "scheduler: 数不出一共有多少节点"),
                }
            }

            if !settings.auto_fetch_enabled {
                continue;
            }

            let interval_secs = (settings.auto_fetch_interval_min.max(5) as u64) * 60;
            let interval = Duration::from_secs(interval_secs);
            let due = match settings.last_auto_fetch_at {
                None => true,
                Some(t) => {
                    let elapsed = chrono::Utc::now()
                        .signed_duration_since(t)
                        .to_std()
                        .unwrap_or(Duration::ZERO);
                    elapsed >= interval
                }
            };

            if !due {
                continue;
            }

            self.running.store(true, Ordering::Relaxed);
            info!(
                interval_min = settings.auto_fetch_interval_min,
                "scheduler: auto-fetch starting"
            );

            match self.collector.fetch_all_enabled().await {
                Ok(results) => {
                    let ok = results.iter().filter(|r| r.success).count();
                    let nodes: usize = results.iter().map(|r| r.node_count).sum();
                    info!(
                        ok,
                        total = results.len(),
                        nodes,
                        "scheduler: auto-fetch done"
                    );
                    let _ = self.db.touch_auto_fetch().await;

                    if settings.auto_speedtest_enabled && nodes > 0 {
                        info!("scheduler: auto-speedtest starting");
                        let opts = SpeedTestOptions {
                            concurrency: settings.speedtest_concurrency,
                            timeout_ms: settings.speedtest_timeout_ms,
                            measure_download: settings.speedtest_download,
                            download_test_ms: settings.download_test_ms,
                            intensity: SpeedtestIntensity::Standard,
                            parallel_kernels: settings.speedtest_parallel_kernels,
                            persistent_kernel: settings.speedtest_persistent_kernel,
                            reuse_verified_minutes: settings.speedtest_reuse_minutes,
                            conditional_isolation: settings.speedtest_conditional_isolation,
                            download_top_n: settings.speedtest_download_top_n,
                        };
                        match self.speedtester.test_all_sources(opts).await {
                            Ok(sum) => {
                                info!(
                                    ok = sum.ok,
                                    fail = sum.fail,
                                    avg = ?sum.avg_ms,
                                    avg_dl = ?sum.avg_download_kbps,
                                    "scheduler: auto-speedtest done"
                                );
                                let _ = self.db.touch_auto_speedtest().await;
                            }
                            Err(e) => warn!(error = %e, "scheduler: auto-speedtest failed"),
                        }
                    }
                }
                Err(e) => warn!(error = %e, "scheduler: auto-fetch failed"),
            }

            self.running.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod refill_tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    const NOW: i64 = 1_700_000_000;

    fn settings(refill: bool, last_min_ago: Option<i64>) -> AppSettings {
        AppSettings {
            speedtest_auto_refill: refill,
            last_auto_speedtest_at: last_min_ago
                .map(|m| Utc.timestamp_opt(NOW - m * 60, 0).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn off_means_never_running() {
        assert!(!refill_needed(&settings(false, None), 5000, 0, NOW));
    }

    #[test]
    fn nothing_to_test_or_a_full_bench_means_no_run() {
        // 库里一个节点都没有：跑了也是空转。
        assert!(!refill_needed(&settings(true, None), 0, 0, NOW));
        // 测通的已经够挑替补了。
        assert!(!refill_needed(&settings(true, None), 5000, SPARSE_VERIFIED_FLOOR, NOW));
        assert!(!refill_needed(&settings(true, None), 5000, 900, NOW));
        // 差一个就低于地板 -> 跑。
        assert!(refill_needed(&settings(true, None), 5000, SPARSE_VERIFIED_FLOOR - 1, NOW));
    }

    #[test]
    fn cooldown_is_measured_against_the_last_auto_speedtest() {
        assert!(!refill_needed(&settings(true, Some(1)), 5000, 0, NOW));
        assert!(!refill_needed(
            &settings(true, Some(REFILL_COOLDOWN_MIN - 1)),
            5000,
            0,
            NOW
        ));
        assert!(refill_needed(&settings(true, Some(REFILL_COOLDOWN_MIN)), 5000, 0, NOW));
        assert!(refill_needed(&settings(true, Some(600)), 5000, 0, NOW));
    }

    #[test]
    fn refill_options_are_the_cheap_kind() {
        let mut base = AppSettings::default();
        base.speedtest_timeout_ms = 2500;
        base.speedtest_reuse_minutes = 30;
        base.speedtest_download = true;
        let opts = refill_opts(&base);
        assert_eq!(opts.concurrency, REFILL_CONCURRENCY, "后台补测不许吃满测速并发");
        assert_eq!(opts.concurrency, 8);
        assert!(
            !opts.measure_download,
            "下载吞吐那一段最占带宽，自动补测不做"
        );
        assert_eq!(opts.timeout_ms, 2500, "超时仍跟随他的设置");
        assert_eq!(opts.reuse_verified_minutes, 30, "刚测通的不该再测一遍");
    }
}

//! 「预备」这个软件状态的判定，和两个自动动作的门槛。
//!
//! 全部写成纯函数：副作用（起下载、起测速）留在 VpnApp 里，规则本身才测得动。

/// 内核都还没装、代理也不在跑/不在启动 —— 这时软件处于「预备」状态：
/// 它还没准备好能翻墙，但也没坏，正在等着把内核装上。
pub fn is_prepared(has_kernel: bool, proxy_active: bool) -> bool {
    !has_kernel && !proxy_active
}

/// 一次运行最多自动试几次、两次之间至少隔多久。
///
/// 只试一次是不够的：新装的机器没有代理，查版本要直打 `api.github.com`，
/// 那条路在我们这儿是**间歇被掐**的（实测过一半次数整 5.0 秒握手失败），
/// 一次抖动就把"自动准备"变成永远不会再动的死状态。也不许多试 —— 无脑重试
/// 就是每帧一个下载作业。
///
/// 每一次试的是[`auto_prepare_source`]阶梯上的**下一档方式**，所以三次预算等于
/// "换三种走法各撞一次墙"，而不是同一条路重跑三遍。
pub const MAX_AUTO_ATTEMPTS: u32 = 3;
pub const MIN_ATTEMPT_SPACING: std::time::Duration = std::time::Duration::from_secs(90);

/// 该不该自动去下内核。`kernel_busy`（不是界面整体 busy —— 下载内核和拉取/测速
/// 不抢同一份资源，被别人的作业卡住就等于不动）+ 次数 + 间隔三道门。
pub fn should_download_kernel(
    enabled: bool,
    has_kernel: bool,
    proxy_active: bool,
    kernel_busy: bool,
    attempts: u32,
    since_last: Option<std::time::Duration>,
) -> bool {
    if !enabled || !is_prepared(has_kernel, proxy_active) || kernel_busy {
        return false;
    }
    if attempts == 0 {
        return true;
    }
    attempts < MAX_AUTO_ATTEMPTS && since_last.is_some_and(|elapsed| elapsed >= MIN_ATTEMPT_SPACING)
}

/// 自动预备这一次该用哪种下载方式。
///
/// 一次尝试只走一种方式（仍然是一条连接，不并发抢跑），失败下一次就换下一档 ——
/// 新装的机器上没有人在旁边点「换一种方式」，2026-10-14 实测 gh-proxy 会在 18.6/22.5MB
/// 处把正文掐断，只认一种方式就等于把第一次安装的人永久卡在「预备」。
/// 阶梯：他设置里记着的那个 → ghfast → gh-proxy →（挂了本地代理才有）走代理 → 官方直连。
/// 走代理排在官方直连前面：官方直连实测 0.12 MB/s，22MB 会被超时掐成半截；而本机真挂着
/// 代理时它几乎总是最快的一条。没挂代理就不排这一档 —— 那一步本来就注定失败，
/// 占掉一次预算只是浪费 90 秒。
pub fn auto_prepare_source(
    preference: crate::kernel::KernelSource,
    attempt: u32,
    has_local_proxy: bool,
) -> crate::kernel::KernelSource {
    use crate::kernel::KernelSource;
    // 本地导入要人挑 zip，自动路径上永远给不了它 —— 哪怕他上次就是这么装的。
    let mut ladder = vec![KernelSource::GhFast, KernelSource::GhProxy];
    if preference != KernelSource::Local {
        ladder.insert(0, preference);
    }
    if has_local_proxy {
        ladder.push(KernelSource::Proxy);
    }
    ladder.push(KernelSource::Official);
    // 去重保序：preference 常常就是 ghfast，不能让它白占两个名额。
    let mut seen: Vec<KernelSource> = Vec::new();
    ladder.retain(|source| {
        let fresh = !seen.contains(source);
        seen.push(*source);
        fresh
    });
    ladder[(attempt.saturating_sub(1) as usize).min(ladder.len() - 1)]
}

/// 新增订阅源之后要不要顺手测它。只认"创建成功 + 拉取成功 + 这个源真解析出节点了"，
/// 三者缺一条就什么都不做 —— 对一个空源或失败源发起测速，只会白占一次并发。
pub fn should_test_added_source(
    enabled: bool,
    pending_for_this: bool,
    fetch_succeeded: bool,
    node_count: i64,
) -> bool {
    enabled && pending_for_this && fetch_succeeded && node_count > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_only_when_nothing_is_ready_yet() {
        assert!(is_prepared(false, false), "刚装完：没内核也没在跑");
        assert!(!is_prepared(true, false), "内核装好了就不是预备");
        assert!(!is_prepared(false, true), "代理在跑时别把状态改成预备");
    }

    #[test]
    fn kernel_download_is_gated_by_switch_kernel_job_and_budget() {
        let secs = std::time::Duration::from_secs;
        assert!(should_download_kernel(true, false, false, false, 0, None));
        assert!(
            !should_download_kernel(false, false, false, false, 0, None),
            "开关关着就不能自己动"
        );
        assert!(!should_download_kernel(true, true, false, false, 0, None));
        assert!(!should_download_kernel(true, false, true, false, 0, None));
        assert!(
            !should_download_kernel(true, false, false, true, 0, None),
            "内核作业还在跑不许叠第二个"
        );
        // 隔得不够久不重试；够久了可以；用满三次就停。
        assert!(!should_download_kernel(
            true,
            false,
            false,
            false,
            1,
            Some(secs(10))
        ));
        assert!(should_download_kernel(
            true,
            false,
            false,
            false,
            1,
            Some(secs(90))
        ));
        assert!(should_download_kernel(
            true,
            false,
            false,
            false,
            2,
            Some(secs(3_600))
        ));
        assert!(!should_download_kernel(
            true,
            false,
            false,
            false,
            MAX_AUTO_ATTEMPTS,
            Some(secs(3_600))
        ));
    }

    #[test]
    fn added_source_is_only_tested_after_it_yields_nodes() {
        assert!(should_test_added_source(true, true, true, 40));
        assert!(
            !should_test_added_source(false, true, true, 40),
            "开关关着不自动测"
        );
        assert!(
            !should_test_added_source(true, false, true, 40),
            "不是我刚加的那个源就别管"
        );
        assert!(
            !should_test_added_source(true, true, false, 40),
            "拉取失败不测"
        );
        assert!(
            !should_test_added_source(true, true, true, 0),
            "解析出 0 个节点不测"
        );
    }

    use crate::kernel::KernelSource;

    /// 一次尝试换一档，且**不许重复占名额**：默认 ghfast 时第 1、2 档不能都是 ghfast，
    /// 否则三次预算里有两次在撞同一堵墙。
    #[test]
    fn auto_prepare_walks_down_the_ladder_without_repeating() {
        let rung = |attempt, proxy| auto_prepare_source(KernelSource::GhFast, attempt, proxy);
        assert_eq!(rung(1, false), KernelSource::GhFast);
        assert_eq!(rung(2, false), KernelSource::GhProxy);
        assert_eq!(rung(3, false), KernelSource::Official);
        // 挂着本地代理时，第 3 档是先试代理，官方直连退到最后。
        assert_eq!(rung(1, true), KernelSource::GhFast);
        assert_eq!(rung(2, true), KernelSource::GhProxy);
        assert_eq!(rung(3, true), KernelSource::Proxy);

        // 他手动选过 gh-proxy：那它排第一，ghfast 补到第二，仍然不重复。
        let from_proxy_choice =
            |attempt| auto_prepare_source(KernelSource::GhProxy, attempt, false);
        assert_eq!(from_proxy_choice(1), KernelSource::GhProxy);
        assert_eq!(from_proxy_choice(2), KernelSource::GhFast);
        assert_eq!(from_proxy_choice(3), KernelSource::Official);

        // 本地导入要人挑文件，永远不许进自动阶梯。
        assert!(
            (1..=6)
                .map(|attempt| auto_prepare_source(KernelSource::Local, attempt, true))
                .all(|source| source != KernelSource::Local),
            "自动预备不能挑一种需要人选文件的方式"
        );
        // 预算用完后停在最后一档，不越界。
        assert_eq!(
            auto_prepare_source(KernelSource::GhFast, 99, false),
            KernelSource::Official
        );
    }
}

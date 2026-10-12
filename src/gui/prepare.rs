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
    attempts < MAX_AUTO_ATTEMPTS
        && since_last.is_some_and(|elapsed| elapsed >= MIN_ATTEMPT_SPACING)
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
        assert!(!should_download_kernel(true, false, false, false, 1, Some(secs(10))));
        assert!(should_download_kernel(true, false, false, false, 1, Some(secs(90))));
        assert!(should_download_kernel(true, false, false, false, 2, Some(secs(3_600))));
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
        assert!(!should_test_added_source(false, true, true, 40), "开关关着不自动测");
        assert!(
            !should_test_added_source(true, false, true, 40),
            "不是我刚加的那个源就别管"
        );
        assert!(!should_test_added_source(true, true, false, 40), "拉取失败不测");
        assert!(!should_test_added_source(true, true, true, 0), "解析出 0 个节点不测");
    }
}

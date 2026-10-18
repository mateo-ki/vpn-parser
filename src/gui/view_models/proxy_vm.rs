#![allow(dead_code)]
use crate::gui::theme;
use crate::proxy::ProxyState;
use eframe::egui::Color32;

#[derive(Debug, Default)]
pub struct ProxyViewModel;

impl ProxyViewModel {
    /// 根据当前内核与代理状态计算状态指示标签与颜色
    pub fn badge_status(
        is_prepared: bool,
        kernel_busy: bool,
        prepare_error: bool,
        prepare_attempts: u32,
        max_attempts: u32,
        proxy_state: ProxyState,
    ) -> (String, Color32) {
        if is_prepared {
            let text = if kernel_busy {
                format!(
                    "预备 · 正在下载内核 ({}/{})",
                    prepare_attempts, max_attempts
                )
            } else if prepare_error {
                format!("预备 · 下载失败 {}/{}", prepare_attempts, max_attempts)
            } else {
                "预备 · 等待启动".to_string()
            };
            (text, Color32::from_rgb(234, 179, 8))
        } else {
            match proxy_state {
                ProxyState::Running => ("运行中".to_string(), theme::accent()),
                ProxyState::Degraded => ("降级运行".to_string(), Color32::from_rgb(249, 115, 22)),
                ProxyState::Starting => ("正在启动".to_string(), Color32::from_rgb(59, 130, 246)),
                ProxyState::Stopping => ("正在停止".to_string(), Color32::from_rgb(168, 85, 247)),
                ProxyState::Stopped => ("未运行".to_string(), Color32::from_rgb(100, 116, 139)),
                ProxyState::Failed => ("启动失败".to_string(), Color32::from_rgb(239, 68, 68)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proxy_badge_running() {
        let (txt, _) = ProxyViewModel::badge_status(false, false, false, 0, 3, ProxyState::Running);
        assert_eq!(txt, "运行中");
    }

    #[test]
    fn test_proxy_badge_preparing() {
        let (txt, _) = ProxyViewModel::badge_status(true, true, false, 1, 3, ProxyState::Stopped);
        assert!(txt.contains("正在下载内核"));
    }
}

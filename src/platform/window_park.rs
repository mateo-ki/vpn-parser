//! 「收进托盘」的实现：把窗口**移到屏幕外**并挂上 `WS_EX_TOOLWINDOW`，而不是 `Visible(false)`。
//!
//! 为什么不隐藏：窗口一旦不可绘制，winit 再也收不到 WM_PAINT，`App::update()` 一帧都不跑；
//! 而 eframe 在"最后一次延后重绘到期"时把 `ControlFlow` 设成 Poll，此后没有任何帧能把它设回
//! Wait —— 实测 UI 线程从此 100% 烧满一个核（同一实例：收进托盘前 16 ms/4s，收进去之后
//! 4000 ms/4s，2026-10-13）。移到屏幕外则窗口仍然可绘制：无事件时泵真的睡（实测 0 ms/s），
//! 有事件时能真跑帧，托盘那三条菜单也终于有人消费。
//!
//! 所有跨线程等待的 API 都不在这里出现：park / unpark 都只允许在**拥有窗口的 UI 线程**上调用
//! （`SetWindowPos` 对自己的窗口是同步但不等别人配合）。

/// 停放点：足够远，任何实际显示器布局都不会把窗口拉回可见区域。
pub const PARKED_X: i32 = -32_000;
pub const PARKED_Y: i32 = -32_000;
/// 判定"这个位置是被我们停放的"的阈值：留一点 DPI/取整余量。
#[allow(dead_code)]
pub const PARKED_THRESHOLD: i32 = -20_000;

/// 停放前窗口的位置与大小，unpark 原样还回去。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 纯判据：矩形左上角落在停放区就算"已停放"。单独拎出来是为了能测。
#[allow(dead_code)]
pub fn is_parked_rect(left: i32, top: i32) -> bool {
    left <= PARKED_THRESHOLD && top <= PARKED_THRESHOLD
}

/// 窗口现在到底能不能收到 WM_PAINT。不可绘制的三种情况：隐藏、最小化、被我们停放。
#[allow(dead_code)]
pub fn can_paint(is_visible: bool, iconic: bool, parked: bool) -> bool {
    is_visible && !iconic && !parked
}

#[cfg(windows)]
mod imp {
    use super::{is_parked_rect, Spot, PARKED_X, PARKED_Y};
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        RedrawWindow, RDW_ALLCHILDREN, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowRect,
        GetWindowThreadProcessId, IsIconic, PostMessageW, SetForegroundWindow, SetWindowLongPtrW,
        SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW, WM_PAINT, WS_EX_TOOLWINDOW,
        WS_MINIMIZEBOX, WS_SYSMENU,
    };

    /// winit 给真实窗口用的类名；事件窗口 / IME 助手窗口都另有其名，必须排掉。
    const MAIN_WINDOW_CLASS: &str = "Window Class";

    /// 本进程的主窗口句柄（按面积取最大的那个非辅助窗口）。
    fn find_main() -> Option<HWND> {
        unsafe extern "system" fn probe(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let pid = GetCurrentProcessId();
            let mut owner = 0u32;
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner != pid {
                return 1;
            }
            let mut class = [0u16; 64];
            let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
            if len <= 0 {
                return 1;
            }
            if String::from_utf16_lossy(&class[..len as usize]) != MAIN_WINDOW_CLASS {
                return 1;
            }
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return 1;
            }
            let area = (rect.right - rect.left) * (rect.bottom - rect.top);
            let best = &mut *(lparam as *mut (HWND, i32));
            if area > best.1 {
                best.0 = hwnd;
                best.1 = area;
            }
            1
        }

        let mut best: (HWND, i32) = (std::ptr::null_mut(), 0);
        unsafe {
            EnumWindows(
                Some(probe),
                &mut best as *mut _ as LPARAM,
            );
        }
        if best.0.is_null() {
            None
        } else {
            Some(best.0)
        }
    }

    fn rect_of(hwnd: HWND) -> Option<RECT> {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then_some(r)
    }

    /// 窗口此刻是不是被停放在屏幕外。找不到窗口时按"没停放"处理（宁可继续跑帧也别乱动）。
    #[allow(dead_code)]
pub fn parked() -> bool {
        match find_main().and_then(rect_of) {
            Some(r) => is_parked_rect(r.left, r.top),
            None => false,
        }
    }

    /// 收进托盘：记住位置 -> 挂 `WS_EX_TOOLWINDOW`（任务栏/Alt-Tab 里消失）-> 移到屏幕外。
    /// 返回记下的位置，调用方存着好还原。
    pub fn park() -> Option<Spot> {
        let hwnd = find_main()?;
        let r = rect_of(hwnd)?;
        let spot = Spot {
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
        };
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW as isize);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                PARKED_X,
                PARKED_Y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        crate::startup_log(&format!(
            "[托盘] 窗口停放到屏幕外 {PARKED_X},{PARKED_Y}（原位置 {} , {} , {}x{}，已挂 WS_EX_TOOLWINDOW）",
            spot.x, spot.y, spot.w, spot.h
        ));
        Some(spot)
    }

    /// 放回原处并摘掉 `WS_EX_TOOLWINDOW`。必须在 UI 线程调用。
    pub fn unpark(spot: Spot) -> bool {
        let Some(hwnd) = find_main() else {
            crate::startup_log("[托盘] 还原失败：找不到主窗口");
            return false;
        };
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_TOOLWINDOW as isize));
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            } else {
                ShowWindow(hwnd, SW_SHOW);
            }
            SetWindowPos(
                hwnd,
                HWND_TOP,
                spot.x,
                spot.y,
                spot.w,
                spot.h,
                SWP_SHOWWINDOW,
            );
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            RedrawWindow(
                hwnd,
                std::ptr::null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN | RDW_FRAME,
            );
            PostMessageW(hwnd, WM_PAINT, 0, 0);
        }
        crate::startup_log(&format!(
            "[托盘] 窗口已还原到 {} , {}（{}x{}，摘掉 WS_EX_TOOLWINDOW 并强制重绘）",
            spot.x, spot.y, spot.w, spot.h
        ));
        true
    }

    /// 确保主窗口具备 WS_MINIMIZEBOX 和 WS_SYSMENU 样式，以便 Windows DWM 和任务栏
    /// 在最小化与还原该无边框窗口时走标准的系统还原通道并产生重绘通知。
    pub fn ensure_window_sysmenu_and_minimizebox() {
        if let Some(hwnd) = find_main() {
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                let needed = (WS_MINIMIZEBOX | WS_SYSMENU) as isize;
                if (style & needed) != needed {
                    SetWindowLongPtrW(hwnd, GWL_STYLE, style | needed);
                    crate::startup_log(&format!(
                        "[窗口] 已为 hwnd={:?} 补齐 WS_MINIMIZEBOX | WS_SYSMENU 样式",
                        hwnd
                    ));
                }
            }
        }
    }

    /// 强制唤醒、还原并重绘主窗口。用于从最小化恢复、托盘唤回或失活唤醒。
    pub fn restore_and_repaint_main_window() {
        if let Some(hwnd) = find_main() {
            unsafe {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                } else {
                    ShowWindow(hwnd, SW_SHOW);
                }
                BringWindowToTop(hwnd);
                SetForegroundWindow(hwnd);
                SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                RedrawWindow(
                    hwnd,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN | RDW_FRAME,
                );
                PostMessageW(hwnd, WM_PAINT, 0, 0);
            }
            crate::startup_log(&format!(
                "[窗口] 已触发 restore_and_repaint_main_window hwnd={:?}",
                hwnd
            ));
        }
    }
}

#[cfg(windows)]
pub use imp::{ensure_window_sysmenu_and_minimizebox, park, restore_and_repaint_main_window, unpark};

#[cfg(not(windows))]
pub fn parked() -> bool {
    false
}

#[cfg(not(windows))]
pub fn park() -> Option<Spot> {
    None
}

#[cfg(not(windows))]
pub fn unpark(_spot: Spot) -> bool {
    false
}

#[cfg(not(windows))]
pub fn ensure_window_sysmenu_and_minimizebox() {}

#[cfg(not(windows))]
pub fn restore_and_repaint_main_window() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_only_when_actually_off_screen() {
        assert!(is_parked_rect(PARKED_X, PARKED_Y));
        assert!(is_parked_rect(-20_000, -20_000));
        assert!(!is_parked_rect(615, 320), "屏幕内的窗口不算停放");
        assert!(!is_parked_rect(-100, -100), "只是贴边不算停放");
        assert!(!is_parked_rect(1_920, PARKED_Y), "两个轴都要越界");
    }

    #[test]
    fn painting_needs_visibility_and_not_minimized() {
        assert!(can_paint(true, false, false));
        assert!(!can_paint(false, false, false), "隐藏的窗口收不到 WM_PAINT");
        assert!(!can_paint(true, true, false), "最小化同样收不到");
        assert!(!can_paint(true, false, true), "停放的窗口按不可绘制处理");
    }
}

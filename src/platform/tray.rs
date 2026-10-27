use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub const TRAY_WINDOW_CLASS: &str = "VPNParserTrayWindowClass";
pub const WAKEUP_MSG_NAME: &str = "VPN_PARSER_SHOW_WINDOW_MSG\0";

#[cfg(windows)]
pub const WM_WAKEUP_INSTANCE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_USER + 101;

pub type TrayWaker = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ShowWindow,
    ToggleProxy,
    Quit,
}

#[allow(dead_code)]
pub enum TrayUpdate {
    SetProxyRunning(bool),
    SetTooltip(String),
    Shutdown,
}

pub struct TrayHandle {
    update_tx: Sender<TrayUpdate>,
    event_rx: Receiver<TrayEvent>,
    waker: Arc<std::sync::Mutex<Option<TrayWaker>>>,
    #[cfg(windows)]
    _thread: Option<std::thread::JoinHandle<()>>,
}

#[allow(dead_code)]
impl TrayHandle {
    pub fn new() -> Self {
        // 诊断壳不注册托盘图标：系统托盘是全局唯一的一处，多一个图标就是给用户添乱，
        // 而且托盘线程会去 EnumWindows 找"本进程的主窗口"，和用户那个实例的窗口搅在一起。
        if crate::diag::enabled() {
            crate::startup_log("DIAG: 跳过托盘注册（不占系统托盘，也不建托盘消息线程）");
            let (event_tx, event_rx) = channel();
            let (update_tx, update_rx) = channel();
            let waker: Arc<std::sync::Mutex<Option<TrayWaker>>> =
                Arc::new(std::sync::Mutex::new(None));
            drop((event_tx, update_rx));
            return Self {
                update_tx,
                event_rx,
                waker,
                #[cfg(windows)]
                _thread: None,
            };
        }

        let (event_tx, event_rx) = channel();
        let (update_tx, update_rx) = channel();
        let waker: Arc<std::sync::Mutex<Option<TrayWaker>>> = Arc::new(std::sync::Mutex::new(None));

        #[cfg(windows)]
        let thread = {
            let waker_clone = waker.clone();
            std::thread::Builder::new()
                .name("system-tray".to_string())
                .spawn(move || {
                    run_tray_loop(event_tx, update_rx, waker_clone);
                })
                .ok()
        };

        #[cfg(not(windows))]
        {
            let _ = update_rx;
            if let Ok(mut guard) = GLOBAL_NON_WINDOWS_WAKEUP.lock() {
                *guard = Some(NonWindowsWakeupContext {
                    event_tx,
                    waker: waker.clone(),
                });
            }
        }

        Self {
            update_tx,
            event_rx,
            waker,
            #[cfg(windows)]
            _thread: thread,
        }
    }

    pub fn set_waker<F>(&self, waker: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if let Ok(mut guard) = self.waker.lock() {
            *guard = Some(Arc::new(waker));
        }
    }

    pub fn try_recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.try_recv().ok()
    }

    pub fn set_proxy_running(&self, running: bool) {
        let _ = self.update_tx.send(TrayUpdate::SetProxyRunning(running));
    }

    pub fn set_tooltip(&self, tooltip: &str) {
        let _ = self
            .update_tx
            .send(TrayUpdate::SetTooltip(tooltip.to_string()));
    }

    pub fn shutdown(&self) {
        let _ = self.update_tx.send(TrayUpdate::Shutdown);
    }

    pub fn is_supported() -> bool {
        #[cfg(windows)]
        {
            true
        }
        #[cfg(target_os = "linux")]
        {
            // Linux 下探测是否有活动的 StatusNotifierWatcher (SNI) 托盘宿主；
            // 若为无宿主平铺环境（如 niri），返回 false 以便窗口关闭直接退出，避免主窗口隐藏后无托盘可唤醒成为孤儿进程
            detect_linux_tray_backend() == LinuxTrayBackend::SniAvailable
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            false
        }
    }

    pub fn supported(&self) -> bool {
        Self::is_supported()
    }
}

/// Linux 托盘适配后端模式与探测结果
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxTrayBackend {
    /// 存在 StatusNotifierWatcher (D-Bus SNI)，托盘宿主就绪 (如 Waybar, GNOME AppIndicator, KDE)
    SniAvailable,
    /// 无托盘宿主 (如原生纯平铺 niri, 裸 Sway, 无 panel 环境)，安全回退至标准窗口管理
    NoHostFallback,
}

#[allow(dead_code)]
#[cfg(target_os = "linux")]
pub fn detect_linux_tray_backend() -> LinuxTrayBackend {
    // 快速非阻塞探测当前环境中是否运行着 StatusNotifierWatcher
    // 1. 检查 DBUS_SESSION_BUS_ADDRESS 环境变量，没有 D-Bus 直接回退
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return LinuxTrayBackend::NoHostFallback;
    }

    // 2. 尝试通过 busctl 或 dbus-send 快速探测 StatusNotifierWatcher 是否已注册所有者
    let sni_registered = std::process::Command::new("busctl")
        .args(["--user", "status", "org.kde.StatusNotifierWatcher"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::process::Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.DBus",
                "--type=method_call",
                "--print-reply",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus.GetNameOwner",
                "string:org.kde.StatusNotifierWatcher",
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

    if sni_registered {
        LinuxTrayBackend::SniAvailable
    } else {
        LinuxTrayBackend::NoHostFallback
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub fn detect_linux_tray_backend() -> LinuxTrayBackend {
    LinuxTrayBackend::NoHostFallback
}

impl Default for TrayHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// 这里原来有两个"由托盘线程自己去把窗口掀回来"的原生函数
// （`restore_main_window_native` / `reveal_main_window_from_tray`）。收进托盘改成
// "停放到屏幕外"（见 `platform::window_park`）之后，托盘期间帧照跑、事件真的有人消费，
// 它们就没有存在意义了 —— 而且留着有害：跨线程 `ShowWindow` 会往 UI 线程发消息并等回执，
// 2026-10-08 托盘线程正是被这么一次调用永久钉在 `win32u!NtUserShowWindow` 里
// （日志停在「queueing show-window event」之后无输出，左键右键全无反应）。
// 别再把它加回来。

/// 让 Win11 给窗口圆角。`with_decorations(false)` 之后系统那条属性不再作用
/// （它管的是非客户区），无边框窗口的四个角是方的，只能自己向 DWM 要。
/// Win10 上没有 33 号属性，返回错误就静默跳过 —— 那边无边框窗口本来也圆不了。
#[cfg(windows)]
pub fn round_window_corners() {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId,
    };

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let current_pid = lparam as u32;
        let mut pid = 0u32;
        let window_thread = GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == current_pid && window_thread == GetCurrentThreadId() {
            let mut class_buf = [0u16; 64];
            let len = GetClassNameW(hwnd, class_buf.as_mut_ptr(), class_buf.len() as i32);
            let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);
            if class_name != TRAY_WINDOW_CLASS {
                let preference = DWMWCP_ROUND;
                let result = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                    &preference as *const i32 as *const core::ffi::c_void,
                    std::mem::size_of::<i32>() as u32,
                );
                crate::startup_log(&format!(
                    "round_window_corners hwnd={:?} class={} hr=0x{:08X}",
                    hwnd, class_name, result as u32
                ));
            }
        }
        1
    }

    unsafe {
        let pid = GetCurrentProcessId();
        EnumWindows(Some(enum_proc), pid as LPARAM);
    }
}

#[cfg(not(windows))]
pub fn round_window_corners() {}

#[cfg(windows)]
pub fn notify_existing_instance() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, PostMessageW, RegisterWindowMessageW, HWND_BROADCAST,
    };
    let class_name = TRAY_WINDOW_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    unsafe {
        let hwnd = FindWindowW(class_name.as_ptr(), std::ptr::null());
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_WAKEUP_INSTANCE, 0, 0);
        } else {
            let broadcast_name = WAKEUP_MSG_NAME.encode_utf16().collect::<Vec<u16>>();
            let msg_id = RegisterWindowMessageW(broadcast_name.as_ptr());
            if msg_id != 0 {
                PostMessageW(HWND_BROADCAST, msg_id, 0, 0);
            }
        }
    }
}

#[cfg(not(windows))]
struct NonWindowsWakeupContext {
    event_tx: Sender<TrayEvent>,
    waker: Arc<std::sync::Mutex<Option<TrayWaker>>>,
}

#[cfg(not(windows))]
static GLOBAL_NON_WINDOWS_WAKEUP: std::sync::Mutex<Option<NonWindowsWakeupContext>> =
    std::sync::Mutex::new(None);

#[cfg(not(windows))]
pub fn notify_existing_instance() {
    if let Ok(guard) = GLOBAL_NON_WINDOWS_WAKEUP.lock() {
        if let Some(ctx) = guard.as_ref() {
            let _ = ctx.event_tx.send(TrayEvent::ShowWindow);
            trigger_waker(&ctx.waker);
        }
    }
}

fn trigger_waker(waker: &Arc<std::sync::Mutex<Option<TrayWaker>>>) {
    // 锁只用来把 Arc 取出来；回调必须在锁外调用 —— `request_repaint()` 会进 egui
    // 的内部状态，持锁调用会和 UI 线程每帧的 `set_waker` 形成锁序反转。
    let callback = {
        match waker.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => None,
        }
    };
    if let Some(w) = callback {
        w();
    }
}

#[cfg(windows)]
fn run_tray_loop(
    event_tx: Sender<TrayEvent>,
    update_rx: Receiver<TrayUpdate>,
    waker: Arc<std::sync::Mutex<Option<TrayWaker>>>,
) {
    // Ensure tray thread is attached to the interactive user desktop ("default") where Shell_TrayWnd resides
    unsafe {
        use windows_sys::Win32::System::StationsAndDesktops::{OpenDesktopW, SetThreadDesktop};
        let default_name = "default\0".encode_utf16().collect::<Vec<u16>>();
        let hdesk = OpenDesktopW(default_name.as_ptr(), 0, 0, 0x01FF);
        if !hdesk.is_null() {
            let res = SetThreadDesktop(hdesk);
            crate::startup_log(&format!("Tray thread SetThreadDesktop('default'): {}", res));
        }
    }

    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
        DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, IMAGE_ICON,
        LoadIconW, LoadImageW, LR_DEFAULTCOLOR, LR_SHARED, PostMessageW, PostQuitMessage,
        RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SM_CXSMICON, SM_CYSMICON,
        TrackPopupMenu, TranslateMessage, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG,
        TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
        WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
    };

    const WM_TRAY_CALLBACK: u32 = WM_USER + 100;
    const TRAY_ICON_ID: u32 = 1001;
    const CMD_SHOW: u32 = 2001;
    const CMD_TOGGLE_PROXY: u32 = 2002;
    const CMD_EXIT: u32 = 2003;

    struct TrayContext {
        event_tx: Sender<TrayEvent>,
        is_proxy_running: Arc<AtomicBool>,
        waker: Arc<std::sync::Mutex<Option<TrayWaker>>>,
        custom_wakeup_msg: u32,
    }

    let is_proxy_running = Arc::new(AtomicBool::new(false));
    let proxy_running_clone = is_proxy_running.clone();

    let class_name = TRAY_WINDOW_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    let h_instance = unsafe { GetModuleHandleW(std::ptr::null()) };

    let broadcast_name = WAKEUP_MSG_NAME.encode_utf16().collect::<Vec<u16>>();
    let custom_wakeup_msg = unsafe { RegisterWindowMessageW(broadcast_name.as_ptr()) };

    static GLOBAL_CONTEXT: std::sync::Mutex<Option<TrayContext>> = std::sync::Mutex::new(None);

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if let Ok(guard) = GLOBAL_CONTEXT.lock() {
            if let Some(ctx) = guard.as_ref() {
                let is_wakeup = msg == WM_WAKEUP_INSTANCE
                    || (ctx.custom_wakeup_msg != 0 && msg == ctx.custom_wakeup_msg);
                if is_wakeup {
                    crate::startup_log("wnd_proc received instance wakeup message");
                    // 窗口恢复一律交给 UI 线程（见 TrayEvent::ShowWindow 的处理）：托盘线程
                    // 去做跨线程 ShowWindow/SetForegroundWindow 会把这条消息循环卡死，
                    // 之后托盘左键右键全都没反应（2026-10-05 实测：整条链路停在
                    // restore_main_window_native 里，日志再无一行托盘事件）。
                    let _ = ctx.event_tx.send(TrayEvent::ShowWindow);
                    trigger_waker(&ctx.waker);
                    return 0;
                }
            }
        }

        match msg {
            WM_TRAY_CALLBACK => {
                let event = (lparam as usize & 0xFFFF) as u32;
                // 鼠标在图标上移动也算"托盘事件"，一次悬停就是几十条 —— 实测
                // startup.log 8678 行里 5781 行是 0x0200，真出事的启动日志被挤掉。
                // 悬停不记，其余（左键/右键/双击/菜单）照记。
                if event != windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE {
                    crate::startup_log(&format!(
                        "WM_TRAY_CALLBACK received: event={:#06x} (lparam={:#x}, wparam={:#x})",
                        event, lparam, wparam
                    ));
                }

                // Support left click, double click, NIN_SELECT (0x0400), NIN_KEYSELECT (0x0401)
                if event == WM_LBUTTONUP
                    || event == WM_LBUTTONDBLCLK
                    || event == 0x0201 // WM_LBUTTONDOWN
                    || event == (WM_USER + 0)
                    || event == (WM_USER + 1)
                {
                    crate::startup_log("Tray click event matched -> queueing show-window event");
                    // 和右键那条一样：先在短锁里取出需要的东西就立刻放锁。持锁进
                    // user32（reveal/waker 都会回 egui 与 Win32）等于把 GLOBAL_CONTEXT
                    // 压在跨线程调用底下，UI 线程那侧任何取锁的动作都会和托盘互等。
                    let (event_tx, waker) = {
                        match GLOBAL_CONTEXT.lock() {
                            Ok(guard) => match guard.as_ref() {
                                Some(ctx) => (ctx.event_tx.clone(), ctx.waker.clone()),
                                None => return 0,
                            },
                            Err(_) => return 0,
                        }
                    };
                    // 左键就是「把窗口叫回来」。收进托盘现在是"停放到屏幕外"，
                    // 帧照跑，所以这条事件真的有人消费 —— 不需要托盘线程自己去动窗口
                    // （那正是 2026-10-08 把托盘线程钉死在 NtUserShowWindow 里的东西）。
                    let _ = event_tx.send(TrayEvent::ShowWindow);
                    trigger_waker(&waker);
                } else if event == WM_RBUTTONUP || event == 0x007B {
                    // 只认抬起（和 WM_CONTEXTMENU）：以前连 WM_RBUTTONDOWN(0x0204) 也弹一次，
                    // 于是按下弹的那层会被抬起这一下直接顶掉 —— 日志里表现为一次右键两行
                    // "popping up context menu"、外层那行 `cmd=0`，看起来就是"点了没反应"。
                    crate::startup_log("Tray right click event matched -> popping up context menu");
                    // 先在短锁内克隆出需要的字段并释放锁，再进 TrackPopupMenu 的模态
                    // 循环：模态期间本 wnd_proc 会被再入（鼠标划过图标等），持锁
                    // 进菜单会让再入路径在同线程二次加锁，直接死锁——托盘从此
                    // 点击无反应（菜单也永远显示不出来）。
                    let (event_tx, is_proxy_running, waker) = {
                        match GLOBAL_CONTEXT.lock() {
                            Ok(guard) => match guard.as_ref() {
                                Some(ctx) => (
                                    ctx.event_tx.clone(),
                                    ctx.is_proxy_running.clone(),
                                    ctx.waker.clone(),
                                ),
                                None => return 0,
                            },
                            Err(_) => return 0,
                        }
                    };
                    {
                        let mut pt = POINT { x: 0, y: 0 };
                        unsafe {
                            use windows_sys::Win32::System::Threading::{
                                AttachThreadInput, GetCurrentThreadId,
                            };
                            use windows_sys::Win32::UI::WindowsAndMessaging::{
                                GetForegroundWindow, GetWindowThreadProcessId,
                            };
                            GetCursorPos(&mut pt);
                            // 前台切换失败时 TrackPopupMenu 会弹出一个看不见的
                            // 菜单并永久阻塞托盘线程（菜单等不到任何点击），
                            // 表现为托盘从此点击无反应。必须确保切换成功。
                            if SetForegroundWindow(hwnd) == 0 {
                                let fore_hwnd = GetForegroundWindow();
                                let fore_thread =
                                    GetWindowThreadProcessId(fore_hwnd, std::ptr::null_mut());
                                let current_thread = GetCurrentThreadId();
                                if fore_thread != 0 && fore_thread != current_thread {
                                    AttachThreadInput(fore_thread, current_thread, 1);
                                    SetForegroundWindow(hwnd);
                                    AttachThreadInput(fore_thread, current_thread, 0);
                                }
                                SetForegroundWindow(hwnd);
                            }
                            let hmenu = CreatePopupMenu();
                            if hmenu != std::ptr::null_mut() {
                                let show_text = "显示主界面\0".encode_utf16().collect::<Vec<u16>>();
                                AppendMenuW(
                                    hmenu,
                                    MF_STRING,
                                    CMD_SHOW as usize,
                                    show_text.as_ptr(),
                                );

                                let running = is_proxy_running.load(Ordering::Relaxed);
                                let proxy_label = if running {
                                    "停止代理\0"
                                } else {
                                    "启动代理\0"
                                };
                                let proxy_text = proxy_label.encode_utf16().collect::<Vec<u16>>();
                                AppendMenuW(
                                    hmenu,
                                    MF_STRING,
                                    CMD_TOGGLE_PROXY as usize,
                                    proxy_text.as_ptr(),
                                );

                                AppendMenuW(hmenu, MF_SEPARATOR, 0, std::ptr::null());

                                let exit_text = "退出程序\0".encode_utf16().collect::<Vec<u16>>();
                                AppendMenuW(
                                    hmenu,
                                    MF_STRING,
                                    CMD_EXIT as usize,
                                    exit_text.as_ptr(),
                                );

                                let cmd = TrackPopupMenu(
                                    hmenu,
                                    TPM_RIGHTBUTTON | TPM_RETURNCMD,
                                    pt.x,
                                    pt.y,
                                    0,
                                    hwnd,
                                    std::ptr::null(),
                                ) as u32;
                                DestroyMenu(hmenu);
                                PostMessageW(hwnd, 0, 0, 0); // Microsoft KB135788 requirement

                                crate::startup_log(&format!("Tray popup menu choice: cmd={}", cmd));

                                match cmd {
                                    CMD_SHOW => {
                                        let _ = event_tx.send(TrayEvent::ShowWindow);
                                        trigger_waker(&waker);
                                    }
                                    CMD_TOGGLE_PROXY => {
                                        let _ = event_tx.send(TrayEvent::ToggleProxy);
                                        trigger_waker(&waker);
                                    }
                                    CMD_EXIT => {
                                        let _ = event_tx.send(TrayEvent::Quit);
                                        trigger_waker(&waker);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                0
            }
            WM_CLOSE => {
                crate::startup_log("Tray wnd_proc WM_CLOSE -> deleting tray icon");
                unsafe {
                    let mut nid_del: NOTIFYICONDATAW = std::mem::zeroed();
                    nid_del.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                    nid_del.hWnd = hwnd;
                    nid_del.uID = TRAY_ICON_ID;
                    Shell_NotifyIconW(NIM_DELETE, &nid_del);
                    DestroyWindow(hwnd);
                }
                0
            }
            WM_DESTROY => {
                crate::startup_log(
                    "Tray wnd_proc WM_DESTROY -> deleting tray icon and posting quit",
                );
                unsafe {
                    let mut nid_del: NOTIFYICONDATAW = std::mem::zeroed();
                    nid_del.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                    nid_del.hWnd = hwnd;
                    nid_del.uID = TRAY_ICON_ID;
                    Shell_NotifyIconW(NIM_DELETE, &nid_del);
                    PostQuitMessage(0);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    let wnd_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: h_instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };

    unsafe {
        RegisterClassW(&wnd_class);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            h_instance,
            std::ptr::null(),
        )
    };

    crate::startup_log(&format!("Tray window created: hwnd={:?}", hwnd));
    if hwnd.is_null() {
        return;
    }

    *GLOBAL_CONTEXT.lock().unwrap() = Some(TrayContext {
        event_tx,
        is_proxy_running: proxy_running_clone,
        waker,
        custom_wakeup_msg,
    });

    let h_icon = unsafe {
        let cx = GetSystemMetrics(SM_CXSMICON);
        let cy = GetSystemMetrics(SM_CYSMICON);
        let icon = LoadImageW(
            h_instance,
            1 as _,
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR | LR_SHARED,
        );
        if icon.is_null() {
            let fallback = LoadIconW(h_instance, 1 as _);
            if fallback.is_null() {
                LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
            } else {
                fallback
            }
        } else {
            icon as HICON
        }
    };

    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = TRAY_ICON_ID;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY_CALLBACK;
    nid.hIcon = h_icon;

    let tip = "VPN Parser - 网络代理控制台\0"
        .encode_utf16()
        .collect::<Vec<u16>>();
    for (i, &ch) in tip.iter().take(nid.szTip.len() - 1).enumerate() {
        nid.szTip[i] = ch;
    }

    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    crate::startup_log(&format!("Shell_NotifyIconW NIM_ADD result: {}", added));

    let hwnd_raw = hwnd as usize;
    let proxy_running_for_updates = is_proxy_running;
    let _updater = std::thread::spawn(move || {
        while let Ok(update) = update_rx.recv() {
            match update {
                TrayUpdate::SetProxyRunning(val) => {
                    proxy_running_for_updates.store(val, Ordering::Relaxed);
                }
                TrayUpdate::SetTooltip(tip) => {
                    let mut nid_update: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
                    nid_update.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                    nid_update.hWnd = hwnd_raw as HWND;
                    nid_update.uID = TRAY_ICON_ID;
                    nid_update.uFlags = NIF_TIP;
                    let tip_u16 = tip.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
                    for (i, &ch) in tip_u16.iter().take(nid_update.szTip.len() - 1).enumerate() {
                        nid_update.szTip[i] = ch;
                    }
                    unsafe {
                        Shell_NotifyIconW(windows_sys::Win32::UI::Shell::NIM_MODIFY, &nid_update);
                    }
                }
                TrayUpdate::Shutdown => {
                    unsafe {
                        PostMessageW(hwnd_raw as HWND, WM_CLOSE, 0, 0);
                    }
                    break;
                }
            }
        }
    });

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Shell_NotifyIconW(NIM_DELETE, &nid);
        DestroyWindow(hwnd);
        *GLOBAL_CONTEXT.lock().unwrap() = None;
    }
}

/// 清理 Windows 任务栏通知区域可能残留的幽灵图标（由于异常退出未调 NIM_DELETE 的旧进程图标）。
/// 通过向通知栏 ToolbarWindow32 发送微小鼠标移动消息，促使系统重绘并移除已死亡进程的图标。
#[cfg(windows)]
pub fn refresh_notification_area() {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, FindWindowW, GetClientRect, SendMessageW, WM_MOUSEMOVE,
    };

    unsafe extern "system" fn enum_tray_toolbar(hwnd: HWND, _lparam: LPARAM) -> BOOL {
        let mut class_buf = [0u16; 64];
        let len = windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
            hwnd,
            class_buf.as_mut_ptr(),
            class_buf.len() as i32,
        );
        let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);
        if class_name == "ToolbarWindow32" {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rect);
            let mut x = 0;
            while x < rect.right {
                let mut y = 0;
                while y < rect.bottom {
                    let lparam = ((y as u32) << 16) | (x as u32 & 0xFFFF);
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, lparam as LPARAM);
                    y += 12;
                }
                x += 12;
            }
        }
        1
    }

    let tray_wnd_name = "Shell_TrayWnd\0".encode_utf16().collect::<Vec<u16>>();
    let overflow_wnd_name = "NotifyIconOverflowWindow\0"
        .encode_utf16()
        .collect::<Vec<u16>>();

    unsafe {
        let h_tray = FindWindowW(tray_wnd_name.as_ptr(), std::ptr::null());
        if !h_tray.is_null() {
            EnumChildWindows(h_tray, Some(enum_tray_toolbar), 0);
        }
        let h_overflow = FindWindowW(overflow_wnd_name.as_ptr(), std::ptr::null());
        if !h_overflow.is_null() {
            EnumChildWindows(h_overflow, Some(enum_tray_toolbar), 0);
        }
    }
}

#[cfg(not(windows))]
pub fn refresh_notification_area() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_handle_lifecycle_and_updates() {
        let tray = TrayHandle::new();
        tray.set_waker(|| {});
        tray.set_proxy_running(true);
        tray.set_tooltip("VPN Parser Test");
        assert_eq!(tray.try_recv_event(), None);
        tray.shutdown();
    }
}

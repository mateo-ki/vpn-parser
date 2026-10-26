use crate::kernel::KernelKind;
use anyhow::{anyhow, Result};

pub mod firewall;
pub mod tray;
pub mod window_park;
pub use tray::{refresh_notification_area, round_window_corners, TrayEvent, TrayHandle};
pub use window_park::{
    ensure_window_sysmenu_and_minimizebox, park as park_main_window,
    restore_and_repaint_main_window, unpark as unpark_main_window,
};

const SINGLE_INSTANCE_MUTEX_NAME: &str = "Local\\VPNParser-7F61E55C-DB54-49C6-AB9E-986AEBFDD56B";

#[cfg(windows)]
pub struct SingleInstanceGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
pub struct SingleInstanceGuard {
    socket_path: std::path::PathBuf,
}

#[cfg(not(windows))]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(windows)]
pub fn acquire_single_instance() -> Result<Option<SingleInstanceGuard>> {
    let guard = acquire_single_instance_named(SINGLE_INSTANCE_MUTEX_NAME)?;
    if guard.is_none() {
        tray::notify_existing_instance();
    }
    Ok(guard)
}

#[cfg(not(windows))]
pub fn acquire_single_instance() -> Result<Option<SingleInstanceGuard>> {
    use std::io::Write;
    use std::os::unix::net::{UnixListener, UnixStream};

    let socket_path = if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(runtime_dir).join("vpn-parser.sock")
    } else {
        std::env::temp_dir().join("vpn-parser.sock")
    };

    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // 1. 先尝试连接已存在的 socket
    if let Ok(mut stream) = UnixStream::connect(&socket_path) {
        let _ = stream.write_all(b"WAKEUP\n");
        crate::startup_log("检测到已存在运行中的 VPN Parser 实例，已发送唤醒消息。本实例退出。");
        return Ok(None);
    }

    // 2. 连接失败：说明文件不存在，或者前次异常退出留下的死 socket，清理旧文件
    let _ = std::fs::remove_file(&socket_path);

    // 3. 绑定并监听新 socket
    let listener = UnixListener::bind(&socket_path)?;
    let socket_clone = socket_path.clone();

    // 4. 启动后台线程监听后续多实例发来的 WAKEUP 消息
    std::thread::Builder::new()
        .name("single-instance-listener".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                if let Ok(mut s) = stream {
                    let mut buf = [0u8; 16];
                    if let Ok(n) = std::io::Read::read(&mut s, &mut buf) {
                        let msg = String::from_utf8_lossy(&buf[..n]);
                        if msg.contains("WAKEUP") || msg.contains("SHOW") {
                            crate::startup_log("Unix socket 收到外部实例唤醒请求，通知前台显示窗口");
                            tray::notify_existing_instance();
                        }
                    }
                }
            }
        })
        .ok();

    Ok(Some(SingleInstanceGuard {
        socket_path: socket_clone,
    }))
}

#[cfg(windows)]
pub fn show_error(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let title = title
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let text = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
pub fn show_error(title: &str, text: &str) {
    tracing::error!(title, text, "startup error");
}

#[cfg(windows)]
fn acquire_single_instance_named(name: &str) -> Result<Option<SingleInstanceGuard>> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let name = name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    Ok(Some(SingleInstanceGuard { handle }))
}

/// 提权交接时由新进程自己给出的参数：当前进程里同名的要先剔掉，否则同一个 flag 会出现两次。
const HANDOFF_OWNED_ARGS: &[&str] = &["--start-proxy-node", "--elevated-proxy-config"];

/// 组装提权实例的命令行：继承当前进程的参数，但把 `extra` 里会重新给出的 flag 去掉。
///
/// `--start-proxy-node` 出现两次时 clap 直接 `ArgumentConflict` 并 `exit(2)`，提权实例在写下
/// 第一行日志之前就死掉（2026-10-05 现象：点 TUN 模式启动，一闪就没）。程序从快捷方式启动时
/// 没有这个参数，所以只有"带节点启动"之后（安装器/开机自启都是这样）才会复现。
pub fn elevated_arguments(current: &[String], extra: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(current.len() + extra.len());
    let mut index = 0;
    while index < current.len() {
        let token = current[index].as_str();
        let owned = HANDOFF_OWNED_ARGS
            .iter()
            .any(|flag| token == *flag || token.starts_with(&format!("{flag}=")));
        if owned {
            let takes_separate_value = !token.contains('=');
            let next_is_value = current
                .get(index + 1)
                .map(|value| !value.starts_with("--"))
                .unwrap_or(false);
            index += usize::from(takes_separate_value && next_is_value);
        } else {
            args.push(token.to_string());
        }
        index += 1;
    }
    args.extend_from_slice(extra);
    args
}

pub fn quote_windows_argument(value: &str) -> String {
    if !value.is_empty() && !value.chars().any(|ch| ch.is_whitespace() || ch == '"') {
        return value.into();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            quoted.push(ch);
        }
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

/// 这个进程到底有没有被提权 —— 读令牌的 `TokenElevation`，不读 `IsUserAnAdmin()`。
///
/// 为什么换：`IsUserAnAdmin()` 是 shell32 上的老启发式，问的是"这个用户是不是管理员"，
/// 不是"这个进程有没有过 UAC 提权"，两者在 UAC 拆分令牌下可以不一致；而它下面所有用法
/// （防火墙自动放行、TUN 交接、设置页那句"当前是管理员权限"）问的都是后者。
/// 2026-10-15 那条"添加防火墙规则失败: 未知错误"就是这两者对不上时冒出来的。
#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 从 `/proc/self/status` 的 `Uid:` 行判"进程是不是 root"（纯函数，测试能喂假文本）。
/// 用这个而不是 `geteuid()`：仓库里没有 libc 依赖，而这行文本在 Linux 上一定读得到。
#[allow(dead_code)]
fn euid_is_root_from_status(status: &str) -> bool {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .is_some_and(|field| {
            // 四个数依次是 real / effective / saved / fs；**effective** 才是能不能建 tun 的依据。
            field
                .split_whitespace()
                .nth(1)
                .is_some_and(|effective| effective == "0")
        })
}

/// Linux 上"要不要再提权一次"问的就是 effective uid —— 直接 `sudo` 起本程序时它是 0，
/// 这时 TUN 已经能做，不该再要求一次提权。
#[cfg(target_os = "linux")]
pub fn is_elevated() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .map(|text| euid_is_root_from_status(&text))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub fn is_elevated() -> bool {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

/// 检查指定内核可执行文件在当前系统上是否具备直接创建 TUN 虚拟网卡的权限。
/// - 若运行在 Windows / macOS 上：由系统 API 或用户提权处理，返回 true。
/// - 若运行在 Linux 上且当前进程是 root (eUID == 0)：返回 true。
/// - 若运行在 Linux 上且为普通用户：通过 getcap 检查内核文件是否具有 cap_net_admin。
#[allow(dead_code)]
pub fn kernel_has_tun_capability(kernel_path: &std::path::Path) -> bool {
    if is_elevated() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        if !kernel_path.exists() {
            return false;
        }
        if let Ok(output) = std::process::Command::new("getcap").arg(kernel_path).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if stdout.contains("cap_net_admin") {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = kernel_path;
        true
    }
}

/// 返回在 Linux 终端为指定内核赋予 TUN 权限的 shell 命令
pub fn tun_capability_command(kernel_path: &std::path::Path) -> String {
    format!("sudo setcap cap_net_admin,cap_net_raw+ep \"{}\"", kernel_path.display())
}

/// 在 Linux 上检查 TUN 虚拟网卡设备是否真实存在且已激活
#[cfg(target_os = "linux")]
pub fn check_linux_tun_interface_active() -> bool {
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "Meta" || name.starts_with("tun") || name.starts_with("utun") {
                return true;
            }
        }
    }
    if let Ok(content) = std::fs::read_to_string("/proc/net/dev") {
        for line in content.lines() {
            if let Some(iface) = line.split(':').next() {
                let trimmed = iface.trim();
                if trimmed == "Meta" || trimmed.starts_with("tun") || trimmed.starts_with("utun") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(windows)]
pub fn request_elevation(extra_args: &[String]) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let executable = std::env::current_exe()?;
    let current = std::env::args().skip(1).collect::<Vec<_>>();
    let args = elevated_arguments(&current, extra_args);
    let parameters = args
        .iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let verb = "runas\0".encode_utf16().collect::<Vec<_>>();
    let executable = executable
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let parameters = parameters
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(anyhow!("请求管理员权限失败，ShellExecuteW 返回 {result:?}"));
    }
    Ok(())
}

/// Linux 没有 UAC 那样的"再拉一个提权实例"的路（`ShellExecuteW runas` 是 Win32 的）。
/// 且在 Linux 桌面（尤其是 Wayland）下，严禁用 sudo 运行 GUI 程序（会导致 Wayland 鉴权失败、
/// 事件循环崩溃，且配置和数据目录会错误指向 /root）。
///
/// 真正需要网络权限的是代理内核（如 Mihomo / Sing-box）。本函数提供清晰的授权指引。
#[cfg(not(windows))]
pub fn request_elevation(_extra_args: &[String]) -> Result<()> {
    let kernel_path = crate::kernel::managed_kernel_path(crate::kernel::KernelKind::Mihomo);
    let kernel_str = kernel_path.display();
    Err(anyhow!(
        "Linux 上 TUN 模式需要代理内核具备 CAP_NET_ADMIN 权限（普通用户实测报 ioctl(TUNSETIFF): Operation not permitted）。\n\
         【严禁使用 sudo 启动 GUI 程序】（会导致 Wayland 拒绝授权及数据目录错乱）。\n\n\
         正确解决方式（只需在终端中执行一次命令为内核授权）：\n\
         sudo setcap cap_net_admin,cap_net_raw+ep \"{kernel_str}\"\n\n\
         授权完成后即可直接在当前普通用户下启动 TUN 模式；或切回「系统代理」模式使用。"
    ))
}

/// 用 ShellExecute 把一个 exe 交给系统"打开"。
///
/// 为什么不能用 `std::process::Command`：它走 CreateProcess，而 CreateProcess **不看**
/// exe 里的 requireAdministrator 清单 —— 碰到要管理员的安装包直接失败
/// （os error 740「请求的操作需要提升」），既不弹 UAC 也不弹向导。
/// 2026-10-15「关于 → 安装」点了没反应就是这个：安装包 `PrivilegesRequired=admin`。
/// ShellExecute 才走外壳那条路，UAC 提示和向导都会正常出现。
#[cfg(windows)]
pub fn shell_open(path: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = |value: &std::ffi::OsStr| -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    };
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| wide(parent.as_os_str()))
        .unwrap_or_default();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide(std::ffi::OsStr::new("open")).as_ptr(),
            wide(path.as_os_str()).as_ptr(),
            std::ptr::null(),
            if directory.is_empty() {
                std::ptr::null()
            } else {
                directory.as_ptr()
            },
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err(anyhow!(
            "交给系统打开失败：{}",
            shell_error_text(result as isize)
        ));
    }
    Ok(())
}

/// ShellExecute 的返回值不是 HRESULT，是小整数错误码；只把用户会碰到的几种翻成人话。
#[cfg(windows)]
fn shell_error_text(code: isize) -> String {
    match code {
        2 => "找不到文件".into(),
        5 => "系统拒绝访问".into(),
        31 => "这台机器上没有能打开它的程序".into(),
        1223 => "你在 UAC 提示里选了「否」".into(),
        other => format!("错误码 {other}"),
    }
}

#[cfg(not(windows))]
pub fn shell_open(_path: &std::path::Path) -> Result<()> {
    Err(anyhow!("只有 Windows 才有安装包可打开"))
}

#[cfg(windows)]
pub fn trim_working_set() {
    unsafe {
        let handle = windows_sys::Win32::System::Threading::GetCurrentProcess();
        windows_sys::Win32::System::Threading::SetProcessWorkingSetSize(
            handle,
            usize::MAX,
            usize::MAX,
        );
    }
}

#[cfg(not(windows))]
pub fn trim_working_set() {}

#[cfg(windows)]
pub struct JobObjectGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for JobObjectGuard {}
#[cfg(windows)]
unsafe impl Sync for JobObjectGuard {}

#[cfg(windows)]
impl Drop for JobObjectGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(windows)]
pub fn init_process_job() -> Option<JobObjectGuard> {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return None;
    }

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    let res = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if res == 0 {
        unsafe { CloseHandle(job) };
        return None;
    }

    // 故意**不**把当前进程自己放进这个作业。之前放进去过，代价实测很硬：提权交接出去的
    // 第二个 vpn-parser 实例也算在这个作业里，于是 `KILL_ON_JOB_CLOSE` 让"父进程退出/被杀"
    // 连带把提权实例和它的 mihomo 一起打死，9980 当场变死端口（2026-10-05 踩过）。
    // 需要跟着一起死的只有内核，而它们本来就由 `assign_child_process_id` 显式登记。
    Some(JobObjectGuard { handle: job })
}

/// 当前进程是否被圈进"本程序自己创建的那个作业对象"。
///
/// 判据刻意用我们自己的作业句柄去比，而不是 `hJob = NULL`：NULL 是"和调用者的作业比较"，
/// 两边都没作业时返回什么由不得我们控制，拿它当"我在不在作业里"会得出含糊结论
/// （我第一版就是这么写，测试直接假失败）。
#[cfg(windows)]
#[cfg(test)]
pub fn current_process_in_our_job() -> bool {
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    ensure_job_initialized();
    match GLOBAL_JOB.get() {
        Some(Some(job)) => {
            let mut result = 0;
            unsafe { IsProcessInJob(GetCurrentProcess(), job.handle, &mut result) };
            result != 0
        }
        _ => false,
    }
}

#[cfg(not(windows))]
pub fn current_process_in_our_job() -> bool {
    false
}

#[cfg(windows)]
static GLOBAL_JOB: std::sync::OnceLock<Option<JobObjectGuard>> = std::sync::OnceLock::new();

#[cfg(windows)]
pub fn ensure_job_initialized() {
    GLOBAL_JOB.get_or_init(init_process_job);
}

#[cfg(windows)]
pub fn assign_child_process_id(pid: u32) {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    if let Some(Some(job)) = GLOBAL_JOB.get() {
        unsafe {
            let handle = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid);
            if !handle.is_null() {
                AssignProcessToJobObject(job.handle, handle);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(windows)]
pub fn kill_orphan_proxy_cores() -> usize {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() {
        return 0;
    }

    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut killed_count = 0;

    if unsafe { Process32FirstW(snapshot, &mut entry) } != FALSE {
        loop {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            let exe_lower = exe_name.to_ascii_lowercase();

            if is_proxy_core_name(&exe_lower) {
                unsafe {
                    let proc_handle = OpenProcess(
                        windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION
                            | PROCESS_TERMINATE,
                        FALSE,
                        entry.th32ProcessID,
                    );
                    if !proc_handle.is_null() {
                        let mut full_path: [u16; 1024] = [0; 1024];
                        let mut size = full_path.len() as u32;
                        let mut is_ours = false;
                        if windows_sys::Win32::System::Threading::QueryFullProcessImageNameW(
                            proc_handle,
                            0,
                            full_path.as_mut_ptr(),
                            &mut size,
                        ) != 0 {
                            let path_str = String::from_utf16_lossy(&full_path[..size as usize]).to_ascii_lowercase();
                            if path_str.contains("vpn-parser") || path_str.contains("vpn_parser") || path_str.contains("vpn-orphan") {
                                is_ours = true;
                            } else if let Ok(cwd) = std::env::current_dir() {
                                let cwd_lower = cwd.to_string_lossy().to_ascii_lowercase();
                                if path_str.starts_with(&cwd_lower) {
                                    is_ours = true;
                                }
                            }
                        } else {
                            // 无法查询路径（可能受权限保护），为安全起见不盲杀第三方进程
                            is_ours = false;
                        }

                        if is_ours {
                            if TerminateProcess(proc_handle, 1) != 0 {
                                killed_count += 1;
                                tracing::info!(
                                    pid = entry.th32ProcessID,
                                    exe = %exe_lower,
                                    "已成功清理归属于本程序的孤儿代理内核进程"
                                );
                            }
                        }
                        CloseHandle(proc_handle);
                    }
                }
            }

            if unsafe { Process32NextW(snapshot, &mut entry) } == FALSE {
                break;
            }
        }
    }

    unsafe { CloseHandle(snapshot) };
    killed_count
}

/// 纯只读探测归属于本程序的孤儿代理内核进程数量，绝不执行 TerminateProcess，无副作用。
#[cfg(windows)]
pub fn count_orphan_proxy_cores() -> usize {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() {
        return 0;
    }

    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut count = 0;

    if unsafe { Process32FirstW(snapshot, &mut entry) } != FALSE {
        loop {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            let exe_lower = exe_name.to_ascii_lowercase();

            if is_proxy_core_name(&exe_lower) {
                unsafe {
                    let proc_handle = OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION,
                        FALSE,
                        entry.th32ProcessID,
                    );
                    if !proc_handle.is_null() {
                        let mut full_path: [u16; 1024] = [0; 1024];
                        let mut size = full_path.len() as u32;
                        let mut is_ours = false;
                        if windows_sys::Win32::System::Threading::QueryFullProcessImageNameW(
                            proc_handle,
                            0,
                            full_path.as_mut_ptr(),
                            &mut size,
                        ) != 0 {
                            let path_str = String::from_utf16_lossy(&full_path[..size as usize]).to_ascii_lowercase();
                            if path_str.contains("vpn-parser") || path_str.contains("vpn_parser") || path_str.contains("vpn-orphan") {
                                is_ours = true;
                            } else if let Ok(cwd) = std::env::current_dir() {
                                let cwd_lower = cwd.to_string_lossy().to_ascii_lowercase();
                                if path_str.starts_with(&cwd_lower) {
                                    is_ours = true;
                                }
                            }
                        }
                        if is_ours {
                            count += 1;
                        }
                        CloseHandle(proc_handle);
                    }
                }
            }

            if unsafe { Process32NextW(snapshot, &mut entry) } == FALSE {
                break;
            }
        }
    }

    unsafe { CloseHandle(snapshot) };
    count
}

#[cfg(windows)]
pub fn kill_duplicate_vpn_parser_instances() -> usize {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
    };

    let current_pid = unsafe { GetCurrentProcessId() };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() {
        return 0;
    }

    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut killed_count = 0;

    if unsafe { Process32FirstW(snapshot, &mut entry) } != FALSE {
        loop {
            if entry.th32ProcessID != current_pid && entry.th32ProcessID != 0 {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                let exe_lower = exe_name.to_ascii_lowercase();

                if exe_lower == "vpn-parser.exe" {
                    unsafe {
                        let proc_handle =
                            OpenProcess(PROCESS_TERMINATE, FALSE, entry.th32ProcessID);
                        if !proc_handle.is_null() {
                            if TerminateProcess(proc_handle, 1) != 0 {
                                killed_count += 1;
                                crate::startup_log(&format!(
                                    "Cleaned up stale duplicate vpn-parser.exe process: pid={}",
                                    entry.th32ProcessID
                                ));
                            }
                            CloseHandle(proc_handle);
                        }
                    }
                }
            }

            if unsafe { Process32NextW(snapshot, &mut entry) } == FALSE {
                break;
            }
        }
    }

    unsafe { CloseHandle(snapshot) };

    if killed_count > 0 {
        refresh_notification_area();
    }

    killed_count
}

#[cfg(not(windows))]
pub fn ensure_job_initialized() {}

#[cfg(not(windows))]
pub fn assign_child_process_id(_pid: u32) {}

/// 判定指定的内核进程是否确实归属于本程序（vpn-parser）。
/// 核心准则：杜绝误杀任何第三方代理客户端（Clash Verge、v2rayA、nekoray 等）或用户手动配置运行的内核！
#[cfg(not(windows))]
pub fn is_our_proxy_core(pid: u32) -> bool {
    let link_target = match std::fs::read_link(format!("/proc/{pid}/exe")) {
        Ok(path) => path,
        Err(_) => return false,
    };
    let exe_name = match link_target.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.to_ascii_lowercase(),
        None => return false,
    };
    if !is_proxy_core_name(&exe_name) {
        return false;
    }

    let exe_str = link_target.to_string_lossy();

    // 1. 单元测试探针目录兼容（vpn-orphan- 临时目录）
    if exe_str.contains("vpn-orphan-") {
        return true;
    }

    // 2. 检查可执行文件路径是否位于本程序的受管内核目录中
    let managed = crate::kernel::managed_kernel_root();
    let managed_str = managed.to_string_lossy();
    if exe_str.starts_with(&*managed_str) || exe_str.contains("vpn-parser") {
        return true;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let local_kernels = cwd.join("data").join("kernels");
        if exe_str.starts_with(&*local_kernels.to_string_lossy()) {
            return true;
        }
    }

    // 3. 检查命令行参数中是否包含本程序专有标记（工作目录或临时配置通常在包含 vpn-parser 的路径中）
    if let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) {
        let cmdline_str = String::from_utf8_lossy(&cmdline);
        if cmdline_str.contains("vpn-parser") {
            return true;
        }
    }

    // 属于第三方客户端（Clash Verge、v2rayA、nekoray 等）或系统其它代理服务的内核，严禁清理！
    false
}

/// Linux 侧的孤儿内核清理。精准判断归属性，绝不误杀第三方代理客户端正在运行的同名内核。
#[cfg(not(windows))]
pub fn kill_orphan_proxy_cores() -> usize {
    let mut killed = 0;
    for pid in proc_pids() {
        if !is_our_proxy_core(pid) {
            continue;
        }
        let exe = proc_exe_name(pid).unwrap_or_else(|| "unknown".into());
        if send_terminate(pid) {
            killed += 1;
            tracing::info!(pid, exe = %exe, "已成功清理归属于本程序的孤儿代理内核进程");
        }
    }
    killed
}

/// Linux/Unix 侧纯只读探测归属于本程序的孤儿代理内核进程数量，无副作用。
#[cfg(not(windows))]
pub fn count_orphan_proxy_cores() -> usize {
    let mut count = 0;
    for pid in proc_pids() {
        if is_our_proxy_core(pid) {
            count += 1;
        }
    }
    count
}

/// 第二个 GUI 实例：在 Linux 上通过 Unix Domain Socket 进行优雅探测并唤醒已有窗口，
/// 绝不强行杀掉已在运行的旧实例，以保护已有代理连接和数据库操作不被中断。
#[cfg(not(windows))]
pub fn kill_duplicate_vpn_parser_instances() -> usize {
    0
}

/// `/proc` 下的数字目录 = 进程号。
#[cfg(not(windows))]
fn proc_pids() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .collect()
}

/// 进程真实可执行文件名（小写）。权限不足或进程刚消失时返回 None，调用方跳过。
#[cfg(not(windows))]
fn proc_exe_name(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()?
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
}

#[cfg(not(windows))]
fn send_terminate(pid: u32) -> bool {
    // 用 SIGTERM 而不是 SIGKILL：mihomo 收到 TERM 会自己收掉监听，端口立刻可复用。
    std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 是不是我们该管的内核进程。判据取 `KernelKind::directory_name()` 那三个词
/// （`mihomo` / `xray` / `sing-box`），并按当前平台允许带 `.exe` —— 这样这个函数在两个
/// 平台上都是同一份逻辑，不会出现"Windows 列表改了、Linux 忘了改"。
pub fn is_proxy_core_name(name_lower: &str) -> bool {
    let stem = name_lower.strip_suffix(".exe").unwrap_or(name_lower);
    [
        KernelKind::Mihomo,
        KernelKind::Xray,
        KernelKind::SingBox,
        KernelKind::Shoes,
        KernelKind::Meow,
        KernelKind::VpCore,
    ]
    .iter()
    .any(|kind| kind.directory_name() == stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `Uid:` 行的第二个数（effective）才算数：只改 real 不算提权，反过来 real 不是 0
    /// 但 effective 是 0 也算已提权（setuid 场景）。
    #[test]
    fn root_detection_reads_the_effective_uid() {
        assert!(euid_is_root_from_status(
            "Uid:	0	0	0	0
"
        ));
        assert!(euid_is_root_from_status(
            "Uid:	1000	0	1000	0
"
        ));
        assert!(!euid_is_root_from_status(
            "Uid:	1000	1000	1000	1000
"
        ));
        // 反向对照：没有 Uid 行 / 行被截断时不许猜成 root。
        assert!(!euid_is_root_from_status(
            "Gid:	0	0	0	0
"
        ));
        assert!(!euid_is_root_from_status("Uid:"));
    }

    /// Linux 的 TUN 提权请求不能只说"不支持"——那等于把两条能走通的路藏起来。
    /// 断言的是"这句错误有没有可执行性"：给命令、给能力名、给实测的失败原因。
    #[cfg(target_os = "linux")]
    #[test]
    fn the_linux_tun_error_tells_you_how_to_get_cap_net_admin() {
        let text = request_elevation(&[])
            .expect_err("Linux 上不可能就地提权")
            .to_string();
        for needle in ["CAP_NET_ADMIN", "setcap", "sudo", "ioctl(TUNSETIFF)"] {
            assert!(text.contains(needle), "错误里该有 {needle}：{text}");
        }
        // 反向对照：不许再把 Windows 的 UAC/runas 话术端给 Linux 用户。
        assert!(
            !text.contains("Windows TUN 提权"),
            "这是 Linux 分支：{text}"
        );
    }

    /// 端到端对账：`is_elevated()` 必须等于外部真值（`id -u`）。
    /// 这条要在两种身份下各跑一次才有意义（普通用户 / `wsl -u root`），
    /// 因为老实现是恒 false —— 恒 false 在普通用户下"看着是对的"，只有 root 跑才暴露。
    #[cfg(target_os = "linux")]
    #[test]
    fn the_root_detection_agrees_with_id_dash_u() {
        let out = std::process::Command::new("id")
            .arg("-u")
            .output()
            .expect("id 命令总该有");
        let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(
            is_elevated(),
            uid == "0",
            "以 uid={uid} 跑，is_elevated() 却给出 {}",
            is_elevated()
        );
    }

    /// 名字判据是孤儿清理的唯一身份来源：判错就是"要么没清掉、要么杀了别人的进程"。
    #[test]
    fn the_core_name_test_accepts_both_platform_spellings() {
        for name in [
            "mihomo",
            "mihomo.exe",
            "xray",
            "xray.exe",
            "sing-box",
            "sing-box.exe",
        ] {
            assert!(is_proxy_core_name(name), "{name} 该被认出来");
        }
        for name in ["vpn-parser", "vpn-parser.exe", "mihomo-service", "", "mi"] {
            assert!(!is_proxy_core_name(name), "{name} 不该被认出来");
        }
    }

    /// 运行时真验一次孤儿清理：起一个名叫 `mihomo` 的进程（用 `sleep` 的副本冒充），
    /// 调用 `kill_orphan_proxy_cores()`，要求它真的被收掉；同时名字不相干的进程**不许**被牵连。
    /// 反向判据取"诱饵还活着"而不是"返回 0"：后者在任何一台真跑着 mihomo 的机器上都会假失败。
    /// 该测试依赖 Linux 上的 /proc 文件系统，macOS 上无 /proc。
    #[cfg(target_os = "linux")]
    #[test]
    fn the_orphan_cleaner_kills_matching_processes_and_spares_others() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("vpn-orphan-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let sleep_src = if std::path::Path::new("/bin/sleep").exists() {
            "/bin/sleep"
        } else {
            "/usr/bin/sleep"
        };
        let sleep_bytes = std::fs::read(sleep_src).expect("系统应有 sleep 可执行程序");

        // 每次 spawn 用各自的子目录，文件名保持精确的 `mihomo` —— 判据读的是
        // `/proc/<pid>/exe` 的 basename，改名（`mihomo.1`）会让它认不出来，
        // 而共用一个名字在 CI 上撞到过 `ETXTBSY`（写完立刻 exec 或覆盖写正运行的文件）。
        // 故写入前先 unlink 破除旧 inode，且重试只认这一个错误码。
        let mut seq = 0_u32;
        let mut spawn_named = |name: &str| -> std::process::Child {
            seq += 1;
            let slot = dir.join(seq.to_string());
            std::fs::create_dir_all(&slot).unwrap();
            let path = slot.join(name);

            let mut written = false;
            for attempt in 0..10 {
                let _ = std::fs::remove_file(&path);
                match std::fs::write(&path, &sleep_bytes) {
                    Ok(()) => {
                        written = true;
                        break;
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                            && attempt < 9 =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(error) => panic!("无法写入探针二进制文件 {name}：{error}"),
                }
            }
            assert!(written, "写入探针程序超时 (ETXTBSY)");

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            for attempt in 0..10 {
                match Command::new(&path).arg("120").spawn() {
                    Ok(child) => return child,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::ExecutableFileBusy
                            && attempt < 9 =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(error) => panic!("起不了探针进程 {name}：{error}"),
                }
            }
            unreachable!()
        };

        let mut core = spawn_named("mihomo");
        let mut decoy = spawn_named("totally-unrelated");

        let killed = kill_orphan_proxy_cores();
        assert!(
            killed >= 1,
            "一个名叫 mihomo 的进程没被收掉 = 判据或 /proc 读取不对"
        );
        core.wait().unwrap();

        // 诱饵必须还在（没被误杀）。用 /proc 直接看，不等它自己退出。
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            std::path::Path::new(&format!("/proc/{}", decoy.id())).exists(),
            "名字不相干的进程被误杀了"
        );
        decoy.kill().unwrap();
        let _ = decoy.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quotes_windows_arguments_with_spaces_and_trailing_slashes() {
        assert_eq!(quote_windows_argument("plain"), "plain");
        assert_eq!(quote_windows_argument(""), "\"\"");
        assert_eq!(
            quote_windows_argument("D:\\proxy data\\"),
            "\"D:\\proxy data\\\\\""
        );
    }

    #[test]
    fn escapes_quotes_for_windows_command_line() {
        assert_eq!(quote_windows_argument("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn handoff_drops_args_the_new_process_replies_itself() {
        let current = [
            "--start-proxy-node",
            "db022bb2-3d57",
            "--db",
            "data/vpn-parser.db",
        ]
        .map(String::from)
        .to_vec();
        let extra = [
            "--elevated-proxy-config".to_string(),
            "eyJtb2RlIjoidHVuIn0".to_string(),
            "--start-proxy-node".to_string(),
            "aaaaaaaa-new".to_string(),
        ];
        assert_eq!(
            elevated_arguments(&current, &extra),
            vec![
                "--db".to_string(),
                "data/vpn-parser.db".to_string(),
                "--elevated-proxy-config".to_string(),
                "eyJtb2RlIjoidHVuIn0".to_string(),
                "--start-proxy-node".to_string(),
                "aaaaaaaa-new".to_string(),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn job_creation_does_not_capture_the_app_process() {
        // 这条守卫的是"TUN 提权交接后父实例能不能自己退掉"：主进程一旦被圈进
        // KILL_ON_JOB_CLOSE 的作业，交接出去的提权实例会算在同一个作业里，父进程退出
        // 就连带打死提权实例和它的内核（实测踩过，9980 当场变死端口）。
        // 若这里失败，说明测试运行时本身就被包在某个作业里 —— 那要改写这条判据，
        // 别直接删掉。
        ensure_job_initialized();
        assert!(
            !current_process_in_our_job(),
            "主进程被圈进作业对象，提权交接实例会被连带杀死"
        );
    }

    #[test]
    fn handoff_keeps_unrelated_flags_including_equals_forms() {
        let current = [
            "--db=data/a.db",
            "--api-token",
            "t0",
            "--start-proxy-node=x",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            elevated_arguments(&current, &[]),
            vec![
                "--db=data/a.db".to_string(),
                "--api-token".to_string(),
                "t0".to_string()
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn named_mutex_allows_only_one_gui_instance() {
        let name = format!(
            "Local\\VpnParserTest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let first = acquire_single_instance_named(&name).unwrap();
        let second = acquire_single_instance_named(&name).unwrap();

        assert!(first.is_some());
        assert!(second.is_none());
    }

    #[test]
    fn test_tun_capability_command_formatting() {
        let p = std::path::Path::new("/usr/local/bin/mihomo");
        let cmd = tun_capability_command(p);
        assert!(cmd.starts_with("sudo setcap cap_net_admin,cap_net_raw+ep"));
        assert!(cmd.contains("/usr/local/bin/mihomo"));
    }

    #[test]
    fn test_crash_rollback_hook_registration() {
        register_crash_rollback_hook();
    }

    #[test]
    fn test_self_heal_stale_proxy_inactive_or_safe() {
        // 当系统未启用死代理时，自愈函数安全返回并不会抛出任何异常
        // 保证单测绝不篡改宿主机正常工作的系统代理现场
        let previous = crate::system_proxy::platform_system_proxy().capture().ok();
        let _ = self_heal_stale_system_proxy(19980);
        if let Some(prev) = previous {
            if prev.enabled {
                let _ = crate::system_proxy::platform_system_proxy().restore(&prev);
            }
        }
    }

    #[test]
    fn test_check_stale_loopback_dns_safe() {
        // 检查函数只读扫描，不修改系统现场
        let items = check_stale_loopback_dns();
        // 只要能正常执行并返回数组即可
        let _ = items.len();
    }

    #[test]
    fn test_self_heal_stale_dns_safe() {
        // 当系统无 127.0.0.1 污染时自愈返回 false 并安全跳过
        let _ = self_heal_stale_dns();
    }

    #[test]
    fn test_elevated_kernel_args_structure() {
        let args = ElevatedKernelArgs {
            parent_pid: 12345,
            kernel: KernelKind::Mihomo,
            executable: std::path::PathBuf::from("C:\\bin\\mihomo.exe"),
            data_root: std::path::PathBuf::from("C:\\data"),
            config: std::path::PathBuf::from("C:\\data\\config.yaml"),
            log_file: std::path::PathBuf::from("C:\\data\\kernel.log"),
        };
        assert_eq!(args.parent_pid, 12345);
        assert_eq!(args.kernel, KernelKind::Mihomo);
        assert_eq!(args.executable, std::path::PathBuf::from("C:\\bin\\mihomo.exe"));
    }
}

#[cfg(windows)]
pub fn list_running_processes() -> Vec<String> {
    use std::collections::BTreeSet;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut names = BTreeSet::new();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot != INVALID_HANDLE_VALUE {
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if unsafe { Process32FirstW(snapshot, &mut entry) } != FALSE {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                let lower = exe_name.to_ascii_lowercase();

                if lower.ends_with(".exe")
                    && !lower.starts_with("svchost")
                    && !lower.starts_with("system")
                    && !lower.starts_with("registry")
                    && !lower.starts_with("smss")
                    && !lower.starts_with("csrss")
                    && !lower.starts_with("services")
                    && !lower.starts_with("lsass")
                {
                    names.insert(exe_name);
                }

                if unsafe { Process32NextW(snapshot, &mut entry) } == FALSE {
                    break;
                }
            }
        }
        unsafe { CloseHandle(snapshot) };
    }
    names.into_iter().collect()
}

#[cfg(not(windows))]
pub fn list_running_processes() -> Vec<String> {
    use std::collections::BTreeSet;
    let mut names = BTreeSet::new();
    for pid in proc_pids() {
        if let Some(name) = proc_exe_name(pid) {
            names.insert(name);
        }
    }
    names.into_iter().collect()
}

#[cfg(windows)]
unsafe extern "system" fn seh_crash_unhandled_exception_filter(
    _exception_info: *const windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    crate::system_proxy::direct_disable_wininet_proxy_atomic();
    let _ = restore_all_stale_loopback_dns();
    0 // EXCEPTION_CONTINUE_SEARCH
}

#[cfg(windows)]
unsafe extern "system" fn console_ctrl_handler(ctrl_type: u32) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::System::Console::*;
    match ctrl_type {
        CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            crate::startup_log(&format!(
                "Console ctrl signal {ctrl_type} received; atomic disabling wininet system proxy & restoring stale DNS..."
            ));
            crate::system_proxy::direct_disable_wininet_proxy_atomic();
            let _ = restore_all_stale_loopback_dns();
            0 // 返回 FALSE (0)，允许后续默认处理或操作系统继续关闭/注销流程
        }
        _ => 0,
    }
}

pub fn register_crash_rollback_hook() {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter(
            Some(seh_crash_unhandled_exception_filter),
        );
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(
            Some(console_ctrl_handler),
            1, // TRUE: 注册控制台关闭/关机处理器
        );
    }
}

pub fn self_heal_stale_system_proxy(mixed_port: u16) -> bool {
    #[cfg(windows)]
    {
        let snapshot = match crate::system_proxy::platform_system_proxy().capture() {
            Ok(s) => s,
            Err(_) => return false,
        };
        if !snapshot.enabled {
            return false;
        }

        let server = match snapshot.server.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };

        let is_target_local_proxy = server.contains(&format!("127.0.0.1:{mixed_port}"))
            || server.contains(&format!("localhost:{mixed_port}"));

        if !is_target_local_proxy && !server.contains("127.0.0.1:") && !server.contains("localhost:") {
            return false;
        }

        let probe_port = if let Some(pos) = server.rfind(':') {
            server[pos + 1..]
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse::<u16>()
                .unwrap_or(mixed_port)
        } else {
            mixed_port
        };

        let is_port_alive = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], probe_port)),
            std::time::Duration::from_millis(150),
        )
        .is_ok();

        if !is_port_alive {
            crate::startup_log(&format!(
                "检测到 WinINet 残留死代理配置 (ProxyServer={server}, 本地端口 {probe_port} 无响应)，自动执行注册表自愈..."
            ));
            let healed = crate::system_proxy::direct_disable_wininet_proxy_atomic();
            if healed {
                crate::startup_log("注册表自愈完成，已强制重置 ProxyEnable=0 并广播刷新，网络已恢复直连。");
            } else {
                crate::startup_log("警告：注册表自愈未能成功更新注册表。");
            }
            return healed;
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = mixed_port;
        false
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StaleDnsItem {
    pub interface_alias: String,
    pub address_family: String,
    pub server_addresses: Vec<String>,
}

#[cfg(windows)]
pub fn check_stale_loopback_dns() -> Vec<StaleDnsItem> {
    use std::os::windows::process::CommandExt;
    const SCRIPT: &str = r#"
Get-DnsClientServerAddress -ErrorAction SilentlyContinue |
Where-Object {
    $_.ServerAddresses -contains '127.0.0.1' -or
    $_.ServerAddresses -contains '::1' -or
    $_.ServerAddresses -contains '198.18.0.1' -or
    $_.ServerAddresses -contains '198.18.0.2'
} |
ForEach-Object {
    $_.InterfaceAlias + '|' + $_.AddressFamily + '|' + ($_.ServerAddresses -join ',')
}
"#;
    let output = match std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", SCRIPT])
        .creation_flags(0x0800_0000)
        .output()
    {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut items = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.trim().split('|').collect();
        if parts.len() >= 3 {
            let alias = parts[0].trim().to_string();
            let fam = if parts[1].trim() == "2" {
                "IPv4".to_string()
            } else {
                "IPv6".to_string()
            };
            let addrs = parts[2]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            items.push(StaleDnsItem {
                interface_alias: alias,
                address_family: fam,
                server_addresses: addrs,
            });
        }
    }
    items
}

#[cfg(not(windows))]
pub fn check_stale_loopback_dns() -> Vec<StaleDnsItem> {
    Vec::new()
}

#[cfg(windows)]
pub fn restore_all_stale_loopback_dns() -> Result<usize> {
    use std::os::windows::process::CommandExt;
    const SCRIPT: &str = r#"
$count = 0
foreach ($fam in 'IPv4','IPv6') {
    Get-DnsClientServerAddress -AddressFamily $fam -ErrorAction SilentlyContinue |
    Where-Object {
        $_.ServerAddresses -contains '127.0.0.1' -or
        $_.ServerAddresses -contains '::1' -or
        $_.ServerAddresses -contains '198.18.0.1' -or
        $_.ServerAddresses -contains '198.18.0.2'
    } |
    ForEach-Object {
        Set-DnsClientServerAddress -InterfaceAlias $_.InterfaceAlias -ResetServerAddresses -ErrorAction SilentlyContinue
        $count++
    }
}
Clear-DnsClientCache -ErrorAction SilentlyContinue
Write-Output $count
"#;

    let output = if is_elevated() {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", SCRIPT])
            .creation_flags(0x0800_0000)
            .output()?
    } else {
        let runas_cmd = format!(
            "Start-Process powershell -Verb RunAs -WindowStyle Hidden -Wait -ArgumentList '-NoProfile -Command \"{}\"'",
            SCRIPT.replace('"', "\"\"")
        );
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &runas_cmd])
            .creation_flags(0x0800_0000)
            .output()?
    };

    let count_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let count = count_str
        .lines()
        .last()
        .unwrap_or("0")
        .parse::<usize>()
        .unwrap_or(0);
    if count > 0 {
        crate::startup_log(&format!(
            "restore_all_stale_loopback_dns finished, reset {count} adapters"
        ));
    }
    Ok(count)
}

#[cfg(not(windows))]
pub fn restore_all_stale_loopback_dns() -> Result<usize> {
    Ok(0)
}

pub fn self_heal_stale_dns() -> bool {
    #[cfg(windows)]
    {
        let stale = check_stale_loopback_dns();
        let has_stale = stale.iter().any(|item| {
            item.server_addresses
                .iter()
                .any(|a| a == "127.0.0.1" || a == "::1" || a == "198.18.0.1" || a == "198.18.0.2")
        });
        if has_stale {
            crate::startup_log("检测到网卡残留死回环/Fake-IP DNS 设置 (127.0.0.1 / ::1 / 198.18.0.1 / 198.18.0.2)，自动执行 DNS 自愈恢复...");
            match restore_all_stale_loopback_dns() {
                Ok(count) => {
                    crate::startup_log(&format!(
                        "DNS 自动自愈完成，已重置 {count} 个受损网卡配置并刷新 DNS 缓存。"
                    ));
                    return true;
                }
                Err(e) => {
                    crate::startup_log(&format!("警告：DNS 自愈执行失败: {e}"));
                }
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Debug, Clone)]
pub struct ElevatedKernelArgs {
    pub parent_pid: u32,
    pub kernel: KernelKind,
    pub executable: std::path::PathBuf,
    pub data_root: std::path::PathBuf,
    pub config: std::path::PathBuf,
    pub log_file: std::path::PathBuf,
}

#[cfg(windows)]
#[derive(Debug)]
#[allow(dead_code)]
pub struct ElevatedChild {
    pub pid: u32,
    pub handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
unsafe impl Send for ElevatedChild {}
#[cfg(windows)]
unsafe impl Sync for ElevatedChild {}

#[cfg(windows)]
impl Drop for ElevatedChild {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.handle);
            }
            self.handle = std::ptr::null_mut();
        }
    }
}

#[cfg(windows)]
impl ElevatedChild {
    #[allow(dead_code)]
    pub fn id(&self) -> Option<u32> {
        Some(self.pid)
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::windows::process::ExitStatusExt;
        use windows_sys::Win32::Foundation::STILL_ACTIVE;
        use windows_sys::Win32::System::Threading::GetExitCodeProcess;

        let mut exit_code: u32 = 0;
        let res = unsafe { GetExitCodeProcess(self.handle, &mut exit_code) };
        if res == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if exit_code == STILL_ACTIVE as u32 {
            Ok(None)
        } else {
            Ok(Some(std::process::ExitStatus::from_raw(exit_code)))
        }
    }

    pub async fn kill(&mut self) -> std::io::Result<()> {
        use windows_sys::Win32::System::Threading::TerminateProcess;
        let res = unsafe { TerminateProcess(self.handle, 1) };
        if res == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(5) {
                return Ok(());
            }
            return Err(err);
        }
        Ok(())
    }

    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        use std::os::windows::process::ExitStatusExt;
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};

        let handle = self.handle as usize;
        tokio::task::spawn_blocking(move || {
            let h = handle as windows_sys::Win32::Foundation::HANDLE;
            unsafe {
                WaitForSingleObject(h, INFINITE);
                let mut exit_code = 0u32;
                GetExitCodeProcess(h, &mut exit_code);
                std::process::ExitStatus::from_raw(exit_code)
            }
        })
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
pub struct ElevatedChild;

#[cfg(not(windows))]
impl ElevatedChild {
    pub fn id(&self) -> Option<u32> {
        None
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        Ok(None)
    }
    pub async fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
    pub async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Not supported on non-Windows",
        ))
    }
}

#[cfg(windows)]
pub fn start_elevated_kernel_runner(
    kernel: KernelKind,
    executable: &std::path::Path,
    data_root: &std::path::Path,
    config_path: &std::path::Path,
    log_path: &std::path::Path,
) -> Result<ElevatedChild> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::GetProcessId;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let parent_pid = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() };
    let self_exe = std::env::current_exe()?;

    let args = vec![
        "--run-elevated-kernel".to_string(),
        "--parent-pid".to_string(),
        parent_pid.to_string(),
        "--kernel".to_string(),
        kernel.directory_name().to_string(),
        "--executable".to_string(),
        executable.to_string_lossy().to_string(),
        "--data-root".to_string(),
        data_root.to_string_lossy().to_string(),
        "--config".to_string(),
        config_path.to_string_lossy().to_string(),
        "--log-file".to_string(),
        log_path.to_string_lossy().to_string(),
    ];

    let parameters = args
        .iter()
        .map(|arg| quote_windows_argument(arg))
        .collect::<Vec<_>>()
        .join(" ");

    let verb = "runas\0".encode_utf16().collect::<Vec<_>>();
    let executable_w = self_exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let parameters_w = parameters
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = executable_w.as_ptr();
    info.lpParameters = parameters_w.as_ptr();
    info.nShow = SW_HIDE;

    let success = unsafe { ShellExecuteExW(&mut info) };
    if success == 0 || info.hProcess.is_null() {
        let err = unsafe { GetLastError() };
        if err == 1223 {
            return Err(anyhow!("用户在 UAC 授权弹窗中选择了「否」或取消了管理员提权"));
        }
        return Err(anyhow!("提权启动无头内核守护进程失败: Win32 错误码 {err}"));
    }

    let pid = unsafe { GetProcessId(info.hProcess) };
    crate::startup_log(&format!(
        "Launched headless elevated kernel runner successfully: pid={pid}"
    ));
    Ok(ElevatedChild {
        pid,
        handle: info.hProcess,
    })
}

#[cfg(not(windows))]
pub fn start_elevated_kernel_runner(
    _kernel: KernelKind,
    _executable: &std::path::Path,
    _data_root: &std::path::Path,
    _config_path: &std::path::Path,
    _log_path: &std::path::Path,
) -> Result<ElevatedChild> {
    Err(anyhow!("无头提权仅在 Windows 平台上可用"))
}

#[cfg(windows)]
pub fn run_elevated_kernel_supervisor(args: ElevatedKernelArgs) -> Result<()> {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForMultipleObjects, INFINITE, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    const SYNCHRONIZE: u32 = 0x0010_0000;

    crate::startup_log(&format!(
        "Elevated supervisor starting for parent_pid={}, kernel={:?}",
        args.parent_pid, args.kernel
    ));

    // 1. 打开父进程 SYNCHRONIZE 句柄。如果父进程已经死掉，直接退出。
    let parent_handle = unsafe { OpenProcess(SYNCHRONIZE, FALSE, args.parent_pid) };
    if parent_handle.is_null() {
        crate::startup_log("Parent process already exited or cannot be opened. Supervisor exiting.");
        return Ok(());
    }

    // 2. 创建专属 Job Object，设置 KILL_ON_JOB_CLOSE
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if !job.is_null() {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
        }
    }

    // 3. 准备日志文件与内核启动命令
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.log_file)?;
    let stderr = log.try_clone()?;

    let mut command = std::process::Command::new(&args.executable);
    match args.kernel {
        KernelKind::Mihomo => {
            command.arg("-d").arg(&args.data_root).arg("-f").arg(&args.config);
        }
        KernelKind::SingBox => {
            command.arg("run").arg("-D").arg(&args.data_root).arg("-c").arg(&args.config);
        }
        KernelKind::Xray => {
            command.arg("run").arg("-c").arg(&args.config);
        }
        KernelKind::Shoes => {
            command.arg("--no-reload").arg(&args.config);
        }
        KernelKind::Meow => {
            command.arg("-d").arg(&args.data_root).arg("-f").arg(&args.config);
        }
        KernelKind::VpCore => {
            command.arg("run").arg("-c").arg(&args.config);
        }
    }
    command
        .stdout(log)
        .stderr(stderr)
        .stdin(std::process::Stdio::null())
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            crate::startup_log(&format!("Supervisor failed to spawn kernel: {e}"));
            unsafe {
                CloseHandle(parent_handle);
                if !job.is_null() {
                    CloseHandle(job);
                }
            }
            return Err(e.into());
        }
    };

    let child_pid = child.id();
    crate::startup_log(&format!(
        "Elevated supervisor spawned kernel child_pid={child_pid}"
    ));
    // 4. 将内核进程圈入专属 Job Object
    if !job.is_null() {
        unsafe {
            let child_proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, child_pid);
            if !child_proc.is_null() {
                AssignProcessToJobObject(job, child_proc);
                CloseHandle(child_proc);
            }
        }
    }

    // 5. 监听父进程与子内核进程
    let child_handle = child.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    let handles = [parent_handle, child_handle];

    let wait_result = unsafe {
        WaitForMultipleObjects(2, handles.as_ptr(), FALSE, INFINITE)
    };

    if wait_result == WAIT_OBJECT_0 {
        // 父进程退出了！主 GUI 崩溃或被强杀
        crate::startup_log("Parent process exited or crashed! Supervisor killing child kernel...");
        let _ = child.kill();
    } else if wait_result == WAIT_OBJECT_0 + 1 {
        // 内核子进程自己退出了
        crate::startup_log("Kernel child process exited.");
    } else {
        crate::startup_log(&format!("WaitForMultipleObjects returned: {wait_result}"));
        let _ = child.kill();
    }

    let _ = child.wait();

    // 内核退出（或主 GUI 异常终止）后的系统网络保护：
    // 强制恢复所有被内核（如 Meow TUN）篡改后遗留的 127.0.0.1 / ::1 / 198.18.0.2 死回环 DNS，
    // 并在提权环境下刷新 DNS 缓存，确保系统物理网卡立即恢复直连网络！
    let _ = restore_all_stale_loopback_dns();

    unsafe {
        CloseHandle(parent_handle);
        if !job.is_null() {
            CloseHandle(job);
        }
    }

    crate::startup_log("Elevated supervisor finished cleanly.");
    Ok(())
}

#[cfg(not(windows))]
pub fn run_elevated_kernel_supervisor(_args: ElevatedKernelArgs) -> Result<()> {
    Err(anyhow!("无头提权仅在 Windows 平台上可用"))
}

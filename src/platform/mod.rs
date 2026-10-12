use anyhow::{anyhow, Result};

pub mod firewall;
pub mod tray;
pub mod window_park;
pub use tray::{
    refresh_notification_area, round_window_corners, TrayEvent,
    TrayHandle,
};
pub use window_park::{
    park as park_main_window, parked as main_window_parked, unpark as unpark_main_window, Spot,
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
pub struct SingleInstanceGuard;

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
    Ok(Some(SingleInstanceGuard))
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

#[cfg(windows)]
pub fn is_elevated() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
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

#[cfg(not(windows))]
pub fn request_elevation(_extra_args: &[String]) -> Result<()> {
    Err(anyhow!("当前平台不支持 Windows TUN 提权"))
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
    let target_names = ["mihomo.exe", "sing-box.exe", "xray.exe"];

    if unsafe { Process32FirstW(snapshot, &mut entry) } != FALSE {
        loop {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe_name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            let exe_lower = exe_name.to_ascii_lowercase();

            if target_names.iter().any(|&target| exe_lower == target) {
                unsafe {
                    let proc_handle = OpenProcess(PROCESS_TERMINATE, FALSE, entry.th32ProcessID);
                    if !proc_handle.is_null() {
                        if TerminateProcess(proc_handle, 1) != 0 {
                            killed_count += 1;
                            tracing::info!(
                                pid = entry.th32ProcessID,
                                exe = %exe_lower,
                                "已成功清理孤儿代理内核进程"
                            );
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
                        let proc_handle = OpenProcess(PROCESS_TERMINATE, FALSE, entry.th32ProcessID);
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

#[cfg(not(windows))]
pub fn kill_orphan_proxy_cores() -> usize {
    0
}

#[cfg(not(windows))]
pub fn kill_duplicate_vpn_parser_instances() -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let current = ["--db=data/a.db", "--api-token", "t0", "--start-proxy-node=x"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            elevated_arguments(&current, &[]),
            vec!["--db=data/a.db".to_string(), "--api-token".to_string(), "t0".to_string()]
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
}

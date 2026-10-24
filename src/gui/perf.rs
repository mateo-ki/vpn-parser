//! 本软件自身的资源占用采样：CPU（进程时间差分）、内存（工作集）、磁盘（I/O 计数差分）。
//!
//! 全部读当前进程自己的数据，不引入额外依赖：
//! Windows 走 kernel32 的 GetProcessTimes / K32GetProcessMemoryInfo / GetProcessIoCounters，
//! Linux 读 /proc/self/{stat,status,io}。差分采样由调用方（poll 循环）每 500ms 做一次。

/// 一次差分后的展示数据。
#[derive(Debug, Clone, Copy, Default)]
pub struct PerfSample {
    /// 进程 CPU 占用（单核百分比，多线程可超 100）。
    pub cpu_percent: f32,
    /// 进程工作集内存（MB）。
    pub mem_mb: f32,
    /// 进程磁盘读写速率（MB/s，含一切文件 I/O）。
    pub disk_mbps: f32,
}

/// 一次原始采样：进程 CPU 累计时间（100ns 单位）与磁盘累计读写字节数。
#[derive(Debug, Clone, Copy)]
pub struct RawSample {
    pub cpu_100ns: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

pub fn raw_sample() -> Option<RawSample> {
    imp::raw_sample()
}

pub fn working_set_mb() -> f32 {
    imp::working_set_mb()
}

#[cfg(windows)]
mod imp {
    use super::RawSample;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessIoCounters, GetProcessTimes, IO_COUNTERS,
    };

    fn filetime_u64(t: FILETIME) -> u64 {
        ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
    }

    pub fn raw_sample() -> Option<RawSample> {
        let handle = unsafe { GetCurrentProcess() };
        let zero = FILETIME { dwHighDateTime: 0, dwLowDateTime: 0 };
        let (mut creation, mut exit, mut kernel, mut user) = (zero, zero, zero, zero);
        if unsafe {
            GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
        } == 0
        {
            return None;
        }
        let mut io: IO_COUNTERS = unsafe { std::mem::zeroed() };
        let io_ok = unsafe { GetProcessIoCounters(handle, &mut io) } != 0;
        Some(RawSample {
            cpu_100ns: filetime_u64(kernel) + filetime_u64(user),
            read_bytes: if io_ok { io.ReadTransferCount } else { 0 },
            write_bytes: if io_ok { io.WriteTransferCount } else { 0 },
        })
    }

    pub fn working_set_mb() -> f32 {
        let handle = unsafe { GetCurrentProcess() };
        let mut pmc: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if unsafe { K32GetProcessMemoryInfo(handle, &mut pmc, pmc.cb) } == 0 {
            return 0.0;
        }
        pmc.WorkingSetSize as f32 / (1024.0 * 1024.0)
    }
}

#[cfg(unix)]
mod imp {
    use super::RawSample;

    pub fn raw_sample() -> Option<RawSample> {
        // /proc/self/stat：最后一个 ')' 之后依次是状态字段，第 12/13 个（0 基 11/12）是 utime/stime
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let rest = stat.rsplit(')').next()?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let u: u64 = fields.get(11)?.parse().ok()?;
        let s: u64 = fields.get(12)?.parse().ok()?;
        let ticks: u64 = u + s;
        // 时钟频率按 100Hz（绝大多数 Linux 如此），换算成 100ns 单位
        let cpu_100ns = ticks * 100_000;

        let mut read_bytes = 0u64;
        let mut write_bytes = 0u64;
        if let Ok(io) = std::fs::read_to_string("/proc/self/io") {
            for line in io.lines() {
                if let Some(v) = line.strip_prefix("read_bytes: ") {
                    read_bytes = v.trim().parse().unwrap_or(0);
                }
                if let Some(v) = line.strip_prefix("write_bytes: ") {
                    write_bytes = v.trim().parse().unwrap_or(0);
                }
            }
        }
        Some(RawSample {
            cpu_100ns,
            read_bytes,
            write_bytes,
        })
    }

    pub fn working_set_mb() -> f32 {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(v) = line.strip_prefix("VmRSS:") {
                    let kb: f32 = v.trim_end_matches("kB").trim().parse().unwrap_or(0.0);
                    return kb / 1024.0;
                }
            }
        }
        0.0
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use super::RawSample;
    pub fn raw_sample() -> Option<RawSample> {
        None
    }
    pub fn working_set_mb() -> f32 {
        0.0
    }
}

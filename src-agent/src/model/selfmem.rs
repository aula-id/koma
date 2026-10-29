//! Memory readings for the current process and host, used by the linker
//! daemon to size its scan budget. Each reader returns `None` when the OS
//! gives no answer, so callers can fall back to an estimate.

/// Resident bytes of this process.
pub fn resident() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        kb_field(&status, "VmRSS:")
    }
    #[cfg(target_os = "macos")]
    {
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as libc::c_int;
        // Safety: `info` is a writable `proc_taskinfo` of exactly `size` bytes.
        let read = unsafe {
            libc::proc_pidinfo(
                std::process::id() as libc::c_int,
                libc::PROC_PIDTASKINFO,
                0,
                (&mut info as *mut libc::proc_taskinfo).cast(),
                size,
            )
        };
        (read == size && info.pti_resident_size > 0).then_some(info.pti_resident_size)
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        // Safety: the pseudo-handle needs no close; `counters` is sized by `cb`.
        let ok =
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
        (ok != 0).then_some(counters.WorkingSetSize as u64)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

/// Installed physical memory.
pub fn physical() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        kb_field(&meminfo, "MemTotal:")
    }
    #[cfg(target_os = "macos")]
    {
        let mut bytes: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // Safety: the name is NUL-terminated and `bytes` holds `len` bytes.
        let rc = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut bytes as *mut u64).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && bytes > 0).then_some(bytes)
    }
    #[cfg(windows)]
    {
        windows_memory_status().map(|s| s.ullTotalPhys)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

/// Memory the OS reports as available to new allocations. `None` on macOS,
/// where the compressor makes free-page counts a poor guide.
pub fn available() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        kb_field(&meminfo, "MemAvailable:")
    }
    #[cfg(windows)]
    {
        windows_memory_status().map(|s| s.ullAvailPhys)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        None
    }
}

#[cfg(windows)]
fn windows_memory_status() -> Option<windows_sys::Win32::System::SystemInformation::MEMORYSTATUSEX>
{
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    // Safety: `status` is a writable MEMORYSTATUSEX with `dwLength` set.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    (ok != 0).then_some(status)
}

#[cfg(any(target_os = "linux", test))]
fn kb_field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let kb = line
            .trim_start()
            .strip_prefix(key)?
            .split_whitespace()
            .next()?;
        kb.parse::<u64>().ok().map(|kb| kb.saturating_mul(1024))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kb_field_reads_kilobytes_as_bytes() {
        let text = "MemTotal:       16000000 kB\nMemAvailable:    8000000 kB\n";
        assert_eq!(kb_field(text, "MemAvailable:"), Some(8_000_000 * 1024));
        assert_eq!(kb_field(text, "VmRSS:"), None);
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn readers_report_this_host() {
        assert!(resident().is_some_and(|b| b > 0));
        assert!(physical().is_some_and(|b| b > resident().unwrap_or(0)));
    }
}

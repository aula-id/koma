//! Terminate an owned Unix process group before reaping its leader.
use std::io;

pub(super) fn kill(pgid: libc::pid_t) -> io::Result<()> {
    if pgid <= 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid task process group",
        ));
    }
    // The caller retains the unreaped leader, preventing group ID reuse.
    if unsafe { libc::kill(-pgid, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    // XNU's killpg1 skips zombies and returns EPERM when none of the remaining
    // members can be signaled. Verify that no live member remains; a genuine
    // permission failure must not be mistaken for successful cancellation.
    // https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c
    #[cfg(target_os = "macos")]
    if error.raw_os_error() == Some(libc::EPERM) && only_zombies_remain(pgid) {
        return Ok(());
    }
    Err(error)
}

#[cfg(target_os = "macos")]
fn only_zombies_remain(pgid: libc::pid_t) -> bool {
    let mut pids = vec![0 as libc::pid_t; 32];
    loop {
        // libproc returns a PID count, not bytes. Zero can also indicate an
        // error, so preserve a distinction between an empty list and failure.
        let count = unsafe {
            *libc::__error() = 0;
            libc::proc_listpgrppids(
                pgid,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(pids.as_slice()) as libc::c_int,
            )
        };
        if count < 0 || (count == 0 && io::Error::last_os_error().raw_os_error() != Some(0)) {
            return false;
        }
        let count = count as usize;
        if count >= pids.len() {
            // Never treat a truncated or unbounded group snapshot as empty.
            if pids.len() >= 16384 {
                return false;
            }
            pids.resize(pids.len() * 2, 0);
            continue;
        }
        return pids[..count].iter().all(|&pid| {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as libc::c_int;
            // arg=1 includes zombies. The default only inspects live processes.
            let read = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    1,
                    (&mut info as *mut libc::proc_bsdinfo).cast(),
                    size,
                )
            };
            if read == size {
                info.pbi_pgid != pgid as u32 || info.pbi_status == libc::SZOMB
            } else {
                // A member may have been reaped since the group snapshot.
                read == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            }
        });
    }
}

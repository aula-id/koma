//! Interactive task processes with an owned PTY and process-tree cancellation.
use super::sync::CheckedMutex;
#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::Path,
};
pub(super) struct Process {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    input: std::sync::mpsc::SyncSender<String>,
    input_error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    reaped: bool,
    #[cfg(windows)]
    job: windows::Job,
    #[cfg(windows)]
    _gate: windows::Gate,
}
impl Process {
    pub(super) fn spawn(
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<String, String>,
        env_remove: &[String],
    ) -> Result<(Self, Box<dyn Read + Send>)> {
        let pair = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        #[cfg(not(windows))]
        let mut command = CommandBuilder::new(program);
        #[cfg(windows)]
        let (mut command, gate) = {
            let gate = windows::Gate::new()?;
            let mut command = CommandBuilder::new(std::env::current_exe()?);
            command.arg("coding-pty-gate");
            command.arg(&gate.name);
            command.arg(program);
            (command, gate)
        };
        command.args(args);
        command.cwd(cwd);
        for (key, value) in env {
            command.env(key, value);
        }
        for key in env_remove {
            command.env_remove(key);
        }
        command.env("TERM", "xterm-256color");
        let mut child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        #[cfg(windows)]
        let job = match windows::Job::attach(
            child
                .as_raw_handle()
                .context("PTY process handle unavailable")?,
        ) {
            Ok(job) => job,
            Err(error) => {
                #[cfg(unix)]
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        #[cfg(windows)]
        if let Err(error) = gate.release() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                #[cfg(unix)]
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                #[cfg(unix)]
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let (input, incoming) = std::sync::mpsc::sync_channel::<String>(16);
        let input_error = std::sync::Arc::new(std::sync::Mutex::new(None));
        let failure = input_error.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("coding-pty-input".into())
            .spawn(move || {
                while let Ok(data) = incoming.recv() {
                    if let Err(error) = writer
                        .write_all(data.as_bytes())
                        .and_then(|_| writer.flush())
                    {
                        *failure.cleanup_lock() = Some(error.to_string());
                        break;
                    }
                }
            })
        {
            #[cfg(unix)]
            if let Some(pid) = child.process_id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        Ok((
            Self {
                child,
                master: pair.master,
                input,
                input_error,
                reaped: false,
                #[cfg(windows)]
                job,
                #[cfg(windows)]
                _gate: gate,
            },
            reader,
        ))
    }
    pub(super) fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }
    pub(super) fn input(&mut self, data: &str) -> Result<()> {
        anyhow::ensure!(!self.reaped, "Task has exited");
        if let Some(error) = self
            .input_error
            .lock()
            .map_err(|_| std::io::Error::other("Terminal input state is poisoned"))?
            .as_ref()
        {
            anyhow::bail!("Terminal input failed: {error}");
        }
        self.input
            .try_send(data.to_string())
            .map_err(|_| anyhow::anyhow!("Terminal input queue is full or closed"))?;
        Ok(())
    }
    pub(super) fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        anyhow::ensure!(
            (1..=500).contains(&rows) && (1..=1000).contains(&cols),
            "Invalid terminal dimensions"
        );
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
    }
    pub(super) fn kill(&mut self) -> std::io::Result<()> {
        if self.reaped {
            return Ok(());
        }
        #[cfg(unix)]
        {
            if let Some(pid) = self.child.process_id() {
                if let Some(foreground) = self.master.process_group_leader() {
                    if foreground > 0 && foreground != pid as i32 {
                        unsafe {
                            libc::kill(-foreground, libc::SIGKILL);
                        }
                    }
                }
                super::process_group::kill(pid as libc::pid_t)?;
                return Ok(());
            }
        }
        #[cfg(windows)]
        {
            self.job.terminate()?;
        }
        self.child.kill()
    }
    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        #[cfg(unix)]
        if !self.reaped {
            if let Some(pid) = self.child.process_id() {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                if unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid,
                        &mut info,
                        libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                    )
                } != 0
                {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        return Ok(None);
                    }
                    if error.raw_os_error() == Some(libc::ECHILD) {
                        self.reaped = true;
                    }
                    return Err(error);
                }
                if unsafe { info.si_pid() } == 0 {
                    return Ok(None);
                }
                self.kill()?;
            }
        }
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.reaped = true;
            #[cfg(windows)]
            {
                let _ = self.job.terminate();
            }
        }
        Ok(status.map(|s| s.exit_code() as i32))
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.kill();
            let _ = self.child.wait();
            self.reaped = true;
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{CreateEventW, OpenEventW, SetEvent, WaitForSingleObject},
        },
    };
    pub(super) struct Job(HANDLE);
    unsafe impl Send for Job {}
    impl Job {
        pub(super) fn attach(process: std::os::windows::io::RawHandle) -> Result<Self> {
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            anyhow::ensure!(
                !handle.is_null(),
                "Create task job: {}",
                std::io::Error::last_os_error()
            );
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            anyhow::ensure!(
                unsafe {
                    SetInformationJobObject(
                        handle,
                        JobObjectExtendedLimitInformation,
                        &limits as *const _ as *const _,
                        std::mem::size_of_val(&limits) as u32,
                    )
                } != 0,
                "Configure task job: {}",
                std::io::Error::last_os_error()
            );
            anyhow::ensure!(
                unsafe { AssignProcessToJobObject(handle, process as HANDLE) } != 0,
                "Assign task job: {}",
                std::io::Error::last_os_error()
            );
            Ok(job)
        }
        pub(super) fn terminate(&self) -> std::io::Result<()> {
            if unsafe { TerminateJobObject(self.0, 1) } == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub(super) struct Gate {
        pub name: String,
        handle: HANDLE,
    }
    unsafe impl Send for Gate {}
    impl Gate {
        pub(super) fn new() -> Result<Self> {
            let name = format!("Local\\KomaCodingPTY{}", uuid::Uuid::new_v4());
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide.as_ptr()) };
            anyhow::ensure!(!handle.is_null(), "Create PTY launch gate failed");
            Ok(Self { name, handle })
        }
        pub(super) fn release(&self) -> Result<()> {
            anyhow::ensure!(
                unsafe { SetEvent(self.handle) } != 0,
                "Release PTY launch gate failed"
            );
            Ok(())
        }
    }
    impl Drop for Gate {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
    pub(super) fn gated_main() -> Result<()> {
        let mut args = std::env::args_os().skip(2);
        let name = args.next().context("Missing PTY launch gate")?;
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = name.encode_wide().chain(Some(0)).collect();
        let handle = unsafe { OpenEventW(0x00100000, 0, wide.as_ptr()) };
        anyhow::ensure!(!handle.is_null(), "PTY launch gate unavailable");
        let result = unsafe { WaitForSingleObject(handle, 10000) };
        unsafe {
            CloseHandle(handle);
        }
        anyhow::ensure!(result == 0, "PTY launch was canceled");
        let program = args.next().context("Missing task program")?;
        let status = std::process::Command::new(program).args(args).status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}
#[cfg(windows)]
pub(super) fn gated_main() -> Result<()> {
    windows::gated_main()
}

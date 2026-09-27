//! Unix SSH workers proxy to a per-user process, so a lost SSH transport does
//! not own the lifetime of tasks, debuggers, tests, language servers or watches.
use anyhow::{Context, Result};
use serde_json::json;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
const FRAME: u64 = 48 * 1024 * 1024;
const PROTOCOL: &str = "coding-v2";
fn directory() -> Result<PathBuf> {
    let dir = crate::model::store::base_dir()?.join("run/coding-service");
    std::fs::create_dir_all(&dir)?;
    let meta = std::fs::symlink_metadata(&dir)?;
    anyhow::ensure!(
        meta.is_dir() && !meta.file_type().is_symlink() && meta.uid() == unsafe { libc::geteuid() },
        "Coding service directory must belong to the current user"
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}
fn socket() -> Result<PathBuf> {
    Ok(directory()?.join(format!("{PROTOCOL}.sock")))
}
fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let n = reader.take(FRAME + 1).read_until(b'\n', &mut line)?;
    if n == 0 {
        return Ok(None);
    }
    anyhow::ensure!(
        n as u64 <= FRAME && line.last() == Some(&b'\n'),
        "Coding frame exceeds limit"
    );
    Ok(Some(line))
}
fn connection() -> Result<UnixStream> {
    let path = socket()?;
    if let Ok(stream) = UnixStream::connect(&path) {
        return Ok(stream);
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("coding-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().context("Start persistent coding service")?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(stream) = UnixStream::connect(&path) {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(stream);
        }
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(
                status.success(),
                "Persistent coding service failed to start: {status}"
            );
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Persistent coding service did not become ready"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub(super) fn proxy() -> Result<()> {
    let stream = connection()?;
    let mut input = stream.try_clone()?;
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        while let Ok(Some(line)) = read_frame(&mut reader) {
            if input.write_all(&line).and_then(|_| input.flush()).is_err() {
                break;
            }
        }
        let _ = input.shutdown(std::net::Shutdown::Both);
    });
    let mut reader = BufReader::new(stream);
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    while let Some(line) = read_frame(&mut reader)? {
        output.write_all(&line)?;
        output.flush()?;
    }
    Ok(())
}
struct Lease {
    _file: File,
    path: PathBuf,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
struct Client {
    id: String,
    sender: mpsc::SyncSender<Arc<Vec<u8>>>,
    connection: UnixStream,
}
type Clients = Arc<Mutex<Vec<Client>>>;
fn serve(stream: UnixStream, clients: Clients, count: Arc<AtomicUsize>) -> Result<()> {
    struct Count(Arc<AtomicUsize>);
    impl Drop for Count {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let _count = Count(count);
    let id = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::sync_channel::<Arc<Vec<u8>>>(64);
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_secs(5)))?;
    clients.lock().unwrap().push(Client {
        id: id.clone(),
        sender: sender.clone(),
        connection: stream.try_clone()?,
    });
    let cleanup = clients.clone();
    let cleanup_id = id.clone();
    std::thread::spawn(move || {
        while let Ok(line) = receiver.recv() {
            if writer
                .write_all(&line)
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
        let _ = writer.shutdown(std::net::Shutdown::Both);
        cleanup
            .lock()
            .unwrap()
            .retain(|client| client.id != cleanup_id);
    });
    let result = (|| -> Result<()> {
        let mut reader = BufReader::new(stream);
        while let Some(line) = read_frame(&mut reader)? {
            let request: super::Request = serde_json::from_slice(&line)?;
            let result = match super::execute(&request) {
                Ok(value) => json!({"id":request.id,"result":value}),
                Err(error) => json!({"id":request.id,"error":error}),
            };
            let mut bytes = serde_json::to_vec(&result)?;
            bytes.push(b'\n');
            anyhow::ensure!(bytes.len() as u64 <= FRAME, "Coding response exceeds limit");
            sender
                .send(Arc::new(bytes))
                .context("Coding client disconnected")?;
        }
        Ok(())
    })();
    clients.lock().unwrap().retain(|client| client.id != id);
    result
}
pub(super) fn run() -> Result<()> {
    let dir = directory()?;
    let path = socket()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(format!("{PROTOCOL}.lock")))?;
    anyhow::ensure!(
        lock.metadata()?.is_file() && lock.metadata()?.uid() == unsafe { libc::geteuid() },
        "Invalid coding service lock"
    );
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(());
        }
        return Err(error.into());
    }
    if path.symlink_metadata().is_ok() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let _lease = Lease { _file: lock, path };
    listener.set_nonblocking(true)?;
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));
    let event_clients = clients.clone();
    let _ = super::EVENTS.set(Arc::new(move |line| {
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        let bytes = Arc::new(bytes);
        event_clients.lock().unwrap().retain(|client| {
            match client.sender.try_send(bytes.clone()) {
                Ok(()) => true,
                Err(_) => {
                    let _ = client.connection.shutdown(std::net::Shutdown::Both);
                    false
                }
            }
        });
    }));
    let count = Arc::new(AtomicUsize::new(0));
    let mut idle = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if count.load(Ordering::Acquire) >= 8 {
                    drop(stream);
                    continue;
                }
                count.fetch_add(1, Ordering::AcqRel);
                let clients = clients.clone();
                let count = count.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, clients, count);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        if count.load(Ordering::Acquire) > 0
            || super::tasks::has_active()
            || super::debug::has_active()
        {
            idle = Instant::now();
        } else if idle.elapsed() > Duration::from_secs(1800) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    super::SHUTTING_DOWN.store(true, Ordering::Release);
    super::tasks::shutdown();
    super::debug::shutdown();
    super::language::shutdown();
    super::watch::shutdown();
    Ok(())
}

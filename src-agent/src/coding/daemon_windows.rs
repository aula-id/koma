//! Persistent Windows coding service, using Koma's owner-restricted named pipes.
use super::sync::CheckedMutex;
use crate::ipc::{split_stream, IpcListener, IpcStream};
use anyhow::{Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
const FRAME: u64 = 48 * 1024 * 1024;
fn address() -> Result<PathBuf> {
    let base = crate::model::store::base_dir()?;
    let digest = format!(
        "{:x}",
        Sha256::digest(base.to_string_lossy().to_lowercase().as_bytes())
    );
    Ok(PathBuf::from(format!(
        r"\\.\pipe\koma-coding-v3-{}",
        &digest[..24]
    )))
}
async fn frame(reader: &mut (impl AsyncBufRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut data = Vec::new();
    let n = reader.take(FRAME + 1).read_until(b'\n', &mut data).await?;
    if n == 0 {
        return Ok(None);
    }
    anyhow::ensure!(
        n as u64 <= FRAME && data.last() == Some(&b'\n'),
        "Coding frame exceeds limit"
    );
    Ok(Some(data))
}
fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}
async fn probe(
    reader: &mut (impl AsyncBufRead + Unpin),
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    op: &str,
) -> Result<serde_json::Value> {
    let mut data = serde_json::to_vec(
        &json!({"id":"service-probe","workspace":{"hostId":"local","root":"C:/"},"op":op}),
    )?;
    data.push(b'\n');
    writer.write_all(&data).await?;
    writer.flush().await?;
    loop {
        let bytes = tokio::time::timeout(Duration::from_secs(5), frame(reader))
            .await??
            .context("Coding service disconnected during version negotiation")?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value["id"] == "service-probe" {
            anyhow::ensure!(
                value["error"].is_null(),
                "Coding service needs a protocol upgrade: {}",
                value["error"]
            );
            return Ok(value["result"].clone());
        }
        tokio::io::stdout().write_all(&bytes).await?;
    }
}
pub(super) fn proxy() -> Result<()> {
    let runtime = runtime()?;
    let result = runtime.block_on(proxy_async());
    // Tokio's stdin uses a blocking OS read; do not let it hold proxy exit open.
    runtime.shutdown_timeout(Duration::from_millis(100));
    result
}
async fn connect() -> Result<IpcStream> {
    let address = address()?;
    match IpcStream::connect(&address).await {
        Ok(stream) => return Ok(stream),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
    };
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("coding-daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
        .spawn()
        .context("Start persistent coding service outside the SSH job; this Windows host must allow job breakaway")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let stream = loop {
        if let Ok(stream) = IpcStream::connect(&address).await {
            break stream;
        }
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "Coding service startup failed ({status})");
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Coding service did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(stream)
}
async fn proxy_async() -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut read, mut write) = loop {
        let (read, mut write) = split_stream(connect().await?);
        let mut read = BufReader::new(read);
        let info = probe(&mut read, &mut write, "serviceInfo").await?;
        if info["build"].as_str() != Some(&super::service_build()) {
            let _ = probe(&mut read, &mut write, "serviceUpgrade").await?;
            if info["activeJobs"] == false && info["clients"].as_u64().unwrap_or(2) <= 1 {
                drop(read);
                drop(write);
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "Previous coding service has not finished draining"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        }
        break (read, write);
    };
    let mut input = BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    let incoming = async {
        while let Some(bytes) = frame(&mut input).await? {
            write.write_all(&bytes).await?;
            write.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let outgoing = async {
        while let Some(bytes) = frame(&mut read).await? {
            output.write_all(&bytes).await?;
            output.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! { result = incoming => result, result = outgoing => result }
}
struct Client {
    id: String,
    sender: tokio::sync::mpsc::Sender<Arc<Vec<u8>>>,
    cancel: tokio::sync::watch::Sender<bool>,
}
type Clients = Arc<Mutex<Vec<Client>>>;
async fn serve(stream: IpcStream, clients: Clients) -> Result<()> {
    struct Count;
    impl Drop for Count {
        fn drop(&mut self) {
            super::SERVICE_CLIENTS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
    let _count = Count;
    let id = uuid::Uuid::new_v4().to_string();
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<Arc<Vec<u8>>>(64);
    let (cancel, mut canceled) = tokio::sync::watch::channel(false);
    clients.checked_lock()?.push(Client {
        id: id.clone(),
        sender: sender.clone(),
        cancel,
    });
    let (read, mut write) = split_stream(stream);
    let mut reader = BufReader::new(read);
    let receive = async {
        while let Some(bytes) = frame(&mut reader).await? {
            let request: super::Request = serde_json::from_slice(&bytes)?;
            let id = request.id.clone();
            let response = tokio::task::spawn_blocking(move || super::execute(&request)).await?;
            let value = match response {
                Ok(value) => json!({"id":id,"result":value}),
                Err(error) => json!({"id":id,"error":error}),
            };
            let mut data = serde_json::to_vec(&value)?;
            data.push(b'\n');
            anyhow::ensure!(data.len() as u64 <= FRAME, "Coding response exceeds limit");
            sender
                .send(Arc::new(data))
                .await
                .context("Coding client disconnected")?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let send = async {
        while let Some(data) = receiver.recv().await {
            tokio::time::timeout(Duration::from_secs(5), write.write_all(&data)).await??;
            write.flush().await?;
        }
        Ok::<(), anyhow::Error>(())
    };
    let result =
        tokio::select! {result=receive=>result,result=send=>result,_=canceled.changed()=>Ok(())};
    clients.cleanup_lock().retain(|c| c.id != id);
    result
}
pub(super) fn run() -> Result<()> {
    let runtime = runtime()?;
    let result = runtime.block_on(run_async());
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
}
async fn run_async() -> Result<()> {
    use std::sync::atomic::Ordering;
    let listener = IpcListener::bind(&address()?).context("Claim coding service named pipe")?;
    let _ = super::service_build();
    let clients: Clients = Arc::new(Mutex::new(Vec::new()));
    let push = clients.clone();
    let _ = super::EVENTS.set(Arc::new(move |line| {
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        let bytes = Arc::new(bytes);
        let mut clients = match push.checked_lock() {
            Ok(clients) => clients,
            Err(error) => {
                eprintln!("Cannot publish coding events: {error}");
                for client in push.cleanup_lock().drain(..) {
                    let _ = client.cancel.send(true);
                }
                return;
            }
        };
        clients.retain(|c| {
            if c.sender.try_send(bytes.clone()).is_ok() {
                true
            } else {
                let _ = c.cancel.send(true);
                false
            }
        });
    }));
    let mut idle = Instant::now();
    // Keep one accept future alive: dropping a pending accept would release
    // the only first-instance pipe and let another daemon claim the name.
    let mut accepted = Box::pin(listener.accept());
    loop {
        tokio::select! {
            incoming = &mut accepted => {
                let (stream, _) = incoming?;
                accepted = Box::pin(listener.accept());
                if super::SERVICE_CLIENTS.load(Ordering::Acquire) < 8 {
                    super::SERVICE_CLIENTS.fetch_add(1, Ordering::AcqRel);
                    let clients = clients.clone();
                    tokio::spawn(async move { let _ = serve(stream, clients).await; });
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        let connected = super::SERVICE_CLIENTS.load(Ordering::Acquire) > 0;
        if connected || super::tasks::has_active() || super::debug::has_active() {
            idle = Instant::now();
        } else if idle.elapsed() > Duration::from_secs(1800)
            || super::SERVICE_DRAIN.load(Ordering::Acquire)
        {
            break;
        }
    }
    super::SHUTTING_DOWN.store(true, Ordering::Release);
    super::tasks::shutdown();
    super::debug::shutdown();
    super::language::shutdown();
    super::watch::shutdown();
    Ok(())
}

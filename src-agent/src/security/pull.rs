//! Pull a guest image while the installer draws one bar.
//!
//! `docker pull --quiet` hides every byte until the process exits, which looks
//! frozen. The engine API streams one JSON object per layer update over the
//! daemon socket, and we turn that into the bar. If that stream cannot be
//! opened, we fall back to a visible `docker pull` (no `--quiet`).

use anyhow::{anyhow, Result};
use base64::Engine;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::progress::{InstallUi, PullMeter, PullSink};

pub(super) struct FetchStats {
    pub transferred: u64,
    pub up_to_date: bool,
    pub elapsed: Duration,
}

#[derive(Debug)]
struct ApiError {
    message: String,
    /// The CLI pull can still do this job (socket, auth, or a cut stream).
    fallback: bool,
}

pub(super) fn fetch_showing(tag: &str, ui: &InstallUi) -> Result<FetchStats> {
    let started = Instant::now();
    if let Some(stats) = pull_with_progress(tag, ui, started)? {
        return Ok(stats);
    }
    ui.end_live();
    super::inherit("docker", &["pull", tag])?;
    Ok(FetchStats {
        transferred: 0,
        up_to_date: false,
        elapsed: started.elapsed(),
    })
}

/// Same stream for a non-terminal sink. Without the engine socket the pull
/// still runs, quietly, and the sink stays on its last phase.
pub(super) fn fetch_tracked(tag: &str, sink: &dyn PullSink) -> Result<FetchStats> {
    let started = Instant::now();
    if let Some(stats) = pull_with_progress(tag, sink, started)? {
        return Ok(stats);
    }
    super::quiet("docker", &["pull", "--quiet", tag])?;
    Ok(FetchStats {
        transferred: 0,
        up_to_date: false,
        elapsed: started.elapsed(),
    })
}

fn pull_with_progress(tag: &str, ui: &dyn PullSink, started: Instant) -> Result<Option<FetchStats>> {
    #[cfg(not(unix))]
    {
        let _ = (tag, ui, started);
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let Some(path) = unix_socket_path() else {
            return Ok(None);
        };
        match api_pull(&path, tag, ui, started) {
            Ok(stats) => {
                ui.end_live();
                Ok(Some(stats))
            }
            Err(err) if err.fallback => {
                ui.end_live();
                ui.info(&format!("using docker pull ({})", one_line(&err.message)));
                Ok(None)
            }
            Err(err) => {
                ui.end_live();
                Err(anyhow!("{}", err.message))
            }
        }
    }
}

#[cfg(unix)]
fn api_pull(
    socket: &std::path::Path,
    tag: &str,
    ui: &dyn PullSink,
    started: Instant,
) -> Result<FetchStats, ApiError> {
    let meter = Mutex::new(PullMeter::new());
    let stop = AtomicBool::new(false);
    let read = std::thread::scope(|scope| {
        scope.spawn(|| heartbeat(&stop, ui, &meter, started));
        let read = connect_and_read(socket, tag, &meter);
        stop.store(true, Ordering::Relaxed);
        read
    });
    let meter = meter
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner());
    read?;
    Ok(FetchStats {
        transferred: meter.transferred(),
        up_to_date: meter.up_to_date(),
        elapsed: started.elapsed(),
    })
}

fn heartbeat(stop: &AtomicBool, ui: &dyn PullSink, meter: &Mutex<PullMeter>, started: Instant) {
    while !stop.load(Ordering::Relaxed) {
        paint(ui, meter, started);
        for _ in 0..2 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn paint(ui: &dyn PullSink, meter: &Mutex<PullMeter>, started: Instant) {
    let view = meter
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .view(ui.next_spinner(), started.elapsed());
    ui.paint_pull(&view);
}

#[cfg(unix)]
fn connect_and_read(
    socket: &std::path::Path,
    tag: &str,
    meter: &Mutex<PullMeter>,
) -> Result<(), ApiError> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket).map_err(|err| ApiError {
        message: format!("connect {}: {err}", socket.display()),
        fallback: true,
    })?;
    let req = pull_request(tag);
    stream.write_all(req.as_bytes()).map_err(|err| ApiError {
        message: format!("write pull request: {err}"),
        fallback: true,
    })?;
    let _ = stream.flush();
    let mut buf = Buf::new(stream);
    read_pull_response(&mut buf, meter)
}

fn pull_request(tag: &str) -> String {
    let auth = base64::engine::general_purpose::STANDARD.encode("{}");
    format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nUser-Agent: koma\r\nX-Registry-Auth: {auth}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        path = pull_path(tag),
    )
}

fn pull_path(reference: &str) -> String {
    let (image, tag) = split_ref(reference);
    let image = url_encode(&image);
    if tag.is_empty() {
        format!("/images/create?fromImage={image}")
    } else {
        format!("/images/create?fromImage={image}&tag={}", url_encode(&tag))
    }
}

/// Split `name:tag` on the colon after the last slash, so `localhost:5000/koma`
/// stays one name. A `@digest` stays on the name and the tag is omitted.
fn split_ref(reference: &str) -> (String, String) {
    if reference.contains('@') {
        return (reference.to_string(), String::new());
    }
    let slash = reference.rfind('/').map(|idx| idx + 1).unwrap_or(0);
    if let Some(rel) = reference[slash..].find(':') {
        let cut = slash + rel;
        let name = &reference[..cut];
        let tag = &reference[cut + 1..];
        if !name.is_empty() && !tag.is_empty() {
            return (name.to_string(), tag.to_string());
        }
    }
    (reference.to_string(), "latest".to_string())
}

fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn unix_socket_path() -> Option<PathBuf> {
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        if !host.is_empty() {
            return unix_host_path(&host);
        }
    }
    let out = std::process::Command::new("docker")
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let host = String::from_utf8(out.stdout).ok()?;
    unix_host_path(host.trim())
}

fn unix_host_path(host: &str) -> Option<PathBuf> {
    let path = host.strip_prefix("unix://")?;
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

struct Buf<R> {
    inner: R,
    data: Vec<u8>,
    pos: usize,
}

impl<R: Read> Buf<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            data: Vec::new(),
            pos: 0,
        }
    }

    fn fill(&mut self) -> std::io::Result<usize> {
        if self.pos > 0 && self.pos <= self.data.len() {
            self.data.drain(..self.pos);
            self.pos = 0;
        }
        let mut tmp = [0u8; 8192];
        loop {
            match self.inner.read(&mut tmp) {
                Ok(0) => return Ok(0),
                Ok(n) => {
                    self.data.extend_from_slice(&tmp[..n]);
                    return Ok(n);
                }
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            }
        }
    }

    fn read_exact_n(&mut self, n: usize) -> Result<Vec<u8>, ApiError> {
        while self.data.len().saturating_sub(self.pos) < n {
            let got = self
                .fill()
                .map_err(|err| transport(format!("read pull stream: {err}")))?;
            if got == 0 {
                return Err(transport("docker closed the pull stream early"));
            }
        }
        let start = self.pos;
        self.pos += n;
        Ok(self.data[start..self.pos].to_vec())
    }

    fn read_line(&mut self, limit: usize) -> Result<Vec<u8>, ApiError> {
        loop {
            if let Some(rel) = self.data[self.pos..].iter().position(|byte| *byte == b'\n') {
                let end = self.pos + rel;
                let mut line = self.data[self.pos..end].to_vec();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.pos = end + 1;
                return Ok(line);
            }
            if self.data.len().saturating_sub(self.pos) > limit {
                return Err(transport("docker pull line is too long"));
            }
            let got = self
                .fill()
                .map_err(|err| transport(format!("read pull stream: {err}")))?;
            if got == 0 {
                if self.pos >= self.data.len() {
                    return Err(transport("docker closed the pull stream early"));
                }
                let mut rest = self.data[self.pos..].to_vec();
                if rest.last() == Some(&b'\r') {
                    rest.pop();
                }
                self.pos = self.data.len();
                return Ok(rest);
            }
        }
    }
}

fn read_pull_response<R: Read>(buf: &mut Buf<R>, meter: &Mutex<PullMeter>) -> Result<(), ApiError> {
    let status_line = buf.read_line(8 * 1024)?;
    let status_line = String::from_utf8_lossy(&status_line);
    let status = http_status(&status_line)
        .ok_or_else(|| transport(format!("bad docker status: {status_line}")))?;
    let mut chunked = false;
    let mut length = None;
    loop {
        let line = buf.read_line(16 * 1024)?;
        if line.is_empty() {
            break;
        }
        let line = String::from_utf8_lossy(&line);
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        } else if name.eq_ignore_ascii_case("content-length") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    if !(200..300).contains(&status) {
        let snippet = error_snippet(buf, chunked, length).unwrap_or_default();
        let message = if snippet.is_empty() {
            format!("docker pull failed with HTTP {status}")
        } else {
            format!("docker pull failed with HTTP {status}: {snippet}")
        };
        return Err(ApiError {
            fallback: status == 401 || status == 404 || is_auth(&message),
            message,
        });
    }
    if chunked {
        read_chunked(buf, meter)?;
    } else if let Some(length) = length {
        let body = buf.read_exact_n(length.min(32 * 1024 * 1024))?;
        feed_bytes(&body, meter)?;
    } else {
        let mut body = Vec::new();
        loop {
            if buf.pos < buf.data.len() {
                body.extend_from_slice(&buf.data[buf.pos..]);
                buf.pos = buf.data.len();
            }
            if body.len() > 32 * 1024 * 1024 {
                return Err(transport("docker pull progress exceeded 32 MB"));
            }
            let got = buf
                .fill()
                .map_err(|err| transport(format!("read pull stream: {err}")))?;
            if got == 0 {
                break;
            }
        }
        feed_bytes(&body, meter)?;
    }
    let guard = meter.lock().unwrap_or_else(|poison| poison.into_inner());
    if guard.saw_status() {
        return Ok(());
    }
    Err(ApiError {
        fallback: guard.transferred() == 0,
        message: "docker pull ended before a status line".to_string(),
    })
}

fn read_chunked<R: Read>(buf: &mut Buf<R>, meter: &Mutex<PullMeter>) -> Result<(), ApiError> {
    let mut pending = Vec::new();
    loop {
        let size_line = buf.read_line(64)?;
        let size_text = String::from_utf8_lossy(&size_line);
        let hex = size_text.split(';').next().unwrap_or("").trim();
        if hex.is_empty() {
            return Err(transport("docker pull chunk size is missing"));
        }
        let size = usize::from_str_radix(hex, 16)
            .map_err(|_| transport("docker pull chunk size is invalid"))?;
        if size == 0 {
            break;
        }
        if size > 8 * 1024 * 1024 {
            return Err(transport("docker pull chunk is too large"));
        }
        let chunk = buf.read_exact_n(size)?;
        let crlf = buf.read_exact_n(2)?;
        if crlf.as_slice() != b"\r\n" {
            return Err(transport("docker pull chunk framing is invalid"));
        }
        pending.extend_from_slice(&chunk);
        drain_lines(&mut pending, meter)?;
    }
    if !pending.is_empty() {
        feed_bytes(&pending, meter)?;
    }
    Ok(())
}

fn drain_lines(pending: &mut Vec<u8>, meter: &Mutex<PullMeter>) -> Result<(), ApiError> {
    while let Some(rel) = pending.iter().position(|byte| *byte == b'\n') {
        let mut line = pending.drain(..=rel).collect::<Vec<_>>();
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(&line);
        ingest_line(meter, text.trim())?;
    }
    Ok(())
}

fn feed_bytes(body: &[u8], meter: &Mutex<PullMeter>) -> Result<(), ApiError> {
    let mut pending = body.to_vec();
    drain_lines(&mut pending, meter)?;
    if !pending.is_empty() {
        let text = String::from_utf8_lossy(&pending);
        ingest_line(meter, text.trim())?;
    }
    Ok(())
}

fn ingest_line(meter: &Mutex<PullMeter>, line: &str) -> Result<(), ApiError> {
    if line.is_empty() {
        return Ok(());
    }
    let mut guard = meter.lock().unwrap_or_else(|poison| poison.into_inner());
    match guard.ingest(line) {
        Ok(()) => Ok(()),
        Err(message) => {
            let fallback = guard.transferred() == 0
                && (is_auth(&message)
                    || (guard.layer_count() == 0 && message.starts_with("unreadable")));
            Err(ApiError { message, fallback })
        }
    }
}

fn error_snippet<R: Read>(
    buf: &mut Buf<R>,
    chunked: bool,
    length: Option<usize>,
) -> Result<String, ApiError> {
    let bytes = if chunked {
        let mut pending = Vec::new();
        let size_line = buf.read_line(64)?;
        let size_text = String::from_utf8_lossy(&size_line);
        let hex = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(hex, 16).unwrap_or(0).min(2048);
        if size > 0 {
            pending = buf.read_exact_n(size)?;
        }
        pending
    } else {
        let n = length.unwrap_or(0).min(2048);
        if n == 0 {
            Vec::new()
        } else {
            buf.read_exact_n(n)?
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.trim().chars().take(300).collect())
}

fn http_status(line: &str) -> Option<u16> {
    let mut parts = line.split_whitespace();
    let version = parts.next()?;
    if !version.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

fn is_auth(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("unauthorized")
        || lower.contains("authentication required")
        || lower.contains("x-registry-auth")
}

fn transport(message: impl Into<String>) -> ApiError {
    ApiError {
        message: message.into(),
        fallback: true,
    }
}

fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or(text).trim();
    let mut short = String::new();
    for (i, ch) in line.chars().enumerate() {
        if i >= 160 {
            short.push('…');
            break;
        }
        short.push(ch);
    }
    if short.is_empty() {
        "no registry stream".to_string()
    } else {
        short
    }
}

#[cfg(test)]
#[path = "pull_test.rs"]
mod pull_test;

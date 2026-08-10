//! Browser-agent daemon client — manages a persistent Python subprocess and
//! communicates via newline-delimited JSON over a Unix socket.
//!
//! The daemon runs `python -m scrapion_agent daemon --socket <path> --token <token>`
//! and keeps a single Playwright Firefox instance alive for the session lifetime.
//! Tools interact with it via [`get_or_start`] / [`BrowserDaemon::request`].
//!
//! # Thread safety
//!
//! The daemon client is [`Send + Sync`]. The internal [`Mutex`] guards are held
//! only for the duration of a single socket write+read (blocking I/O on
//! `std::os::unix::net::UnixStream`), never across an `.await`. Every browser
//! tool runs on a deferred `std::thread` via `DEFERRED_TOOLS`, so there is no
//! tokio-runtime interference.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

/// Maximum time to wait for a single daemon request/response round-trip.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Time to wait after sending a shutdown signal before killing the process.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// Length of the random hex auth token.
const TOKEN_BYTES: usize = 32;

/// Compute a short socket path for the browser daemon.
///
/// The per-session directory (`~/.koma/sessions/<pwd_hash>/<uuid>/`) can
/// exceed the 108-byte `AF_UNIX` limit when combined with a socket filename.
/// This helper places the socket under `~/.koma/internet/browser/<uuid>.sock`
/// instead, keeping the total well under the limit.
fn browser_daemon_sock_path(session_dir: &Path) -> Result<PathBuf> {
    // Extract the UUID (last path component) from the session directory.
    let uuid = session_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");
    let dir = crate::internet::internet_dir()?.join("browser");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create browser daemon dir {}", dir.display()))?;
    Ok(dir.join(format!("{uuid}.sock")))
}

/// Process-global map of active daemons keyed by session directory.
///
/// Lazy-initialised on first access via [`get_or_start`]. Cleaned up via
/// [`cleanup`] when a session closes.
static DAEMONS: OnceLock<Mutex<HashMap<PathBuf, Arc<BrowserDaemon>>>> = OnceLock::new();

/// A persistent browser-agent daemon managing one Playwright Firefox instance.
///
/// Created via [`BrowserDaemon::start`] (or the convenience wrapper
/// [`get_or_start`]). Communicates with the Python subprocess over a
/// newline-delimited JSON protocol on a Unix socket.
pub struct BrowserDaemon {
    socket_path: PathBuf,
    auth_token: String,
    child: Mutex<Option<Child>>,
    stream: Mutex<Option<std::os::unix::net::UnixStream>>,
}

impl BrowserDaemon {
    /// Spawn the Python daemon subprocess and authenticate.
    ///
    /// The subprocess writes a single line (the auth token) to stdout, then
    /// waits for a connection on `socket_path`. This method connects, sends
    /// the token, and performs a health check.
    fn start(session_dir: &Path) -> Result<Arc<Self>> {
        let python = crate::internet::venv_python()
            .context("internet research environment not installed — run `koma --internet-fullmode-install`")?;
        if !python.exists() {
            anyhow::bail!(
                "internet research environment not installed — run `koma --internet-fullmode-install`"
            );
        }

        // Generate auth token: 32 random bytes as hex.
        let token = generate_token()?;
        // Socket path: `~/.koma/internet/browser/<uuid>.sock` (short enough
        // for the 108-byte AF_UNIX limit, unlike the full session dir path).
        let socket_path = browser_daemon_sock_path(session_dir)?;
        // Remove stale socket file if it exists.
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        // Ensure the session directory exists.
        std::fs::create_dir_all(session_dir)
            .with_context(|| format!("create session dir {}", session_dir.display()))?;

        // Spawn the daemon subprocess.
        let mut cmd = Command::new(&python);
        cmd.arg("-m")
            .arg("scrapion_agent")
            .arg("daemon")
            .arg("--socket")
            .arg(&socket_path)
            .arg("--token")
            .arg(&token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        crate::tool::shell::no_console_window(&mut cmd);

        let mut child = cmd
            .spawn()
            .context("failed to spawn browser daemon subprocess")?;

        // Read the first line from stdout — the daemon writes the token back
        // to confirm it started successfully.
        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("daemon stdout not captured"))?;

        let mut reader = BufReader::new(stdout);
        let mut ready_line = String::new();
        reader
            .read_line(&mut ready_line)
            .context("failed to read daemon ready line")?;

        let ready_line = ready_line.trim();
        if ready_line != "ready" && ready_line != token {
            // Capture stderr before killing, so the user sees why the daemon failed.
            let stderr_msg = read_child_stderr(&mut child);
            let _ = child.kill();
            let _ = child.wait();
            let detail = if stderr_msg.is_empty() {
                format!("unexpected ready line: {ready_line:?}")
            } else {
                format!("failed to start:\n{stderr_msg}")
            };
            crate::model::store::append_global_error_log("browser-daemon", &detail);
            anyhow::bail!("browser daemon {detail}");
        }

        // Wait for the socket file to appear (the daemon creates it after
        // binding). Give it up to 10 seconds.
        let socket_wait = Duration::from_secs(10);
        let deadline = std::time::Instant::now() + socket_wait;
        while !socket_path.exists() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                let detail = format!("socket did not appear within {socket_wait:?}");
                crate::model::store::append_global_error_log("browser-daemon", &detail);
                anyhow::bail!("browser daemon {detail}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        // Connect to the socket.
        let stream = std::os::unix::net::UnixStream::connect(&socket_path)
            .context("failed to connect to browser daemon socket")?;
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .context("failed to set socket read timeout")?;
        stream
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .context("failed to set socket write timeout")?;

        let daemon = Arc::new(Self {
            socket_path,
            auth_token: token,
            child: Mutex::new(Some(child)),
            stream: Mutex::new(Some(stream)),
        });

        // Authenticate: the Python daemon reads the raw token as its FIRST line
        // (not a JSON message). Send the token, then read the daemon's auth-OK
        // JSON response.
        daemon.send_raw_token()?;
        daemon.read_auth_response()?;

        // Health check.
        daemon.request("health", serde_json::json!({}))?;

        Ok(daemon)
    }

    /// Send the raw auth token as the first line on the socket.
    ///
    /// The Python daemon reads this as its INITIAL message (not a JSON request).
    fn send_raw_token(&self) -> Result<()> {
        let mut token_line = self.auth_token.clone();
        token_line.push('\n');
        let mut guard = self
            .stream
            .lock()
            .map_err(|e| anyhow::anyhow!("daemon stream lock poisoned: {e}"))?;
        let stream = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("daemon not connected"))?;
        stream
            .write_all(token_line.as_bytes())
            .context("failed to send auth token to daemon")?;
        stream.flush().context("failed to flush auth token")?;
        Ok(())
    }

    /// Read the daemon's auth-OK response line (the first JSON response after
    /// sending the raw token).
    fn read_auth_response(&self) -> Result<()> {
        let mut guard = self
            .stream
            .lock()
            .map_err(|e| anyhow::anyhow!("daemon stream lock poisoned: {e}"))?;
        let stream = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("daemon not connected"))?;
        let mut reader = BufReader::new(&*stream);
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .context("failed to read auth response from daemon")?;
        let response = response.trim();
        if response.is_empty() {
            anyhow::bail!("daemon returned empty auth response");
        }
        // Parse JSON and check status.
        let val: Value =
            serde_json::from_str(response).context("daemon returned invalid auth response")?;
        let status = val.get("status").and_then(Value::as_str).unwrap_or("");
        if status != "ok" {
            let err = val.get("error").and_then(Value::as_str).unwrap_or("unknown");
            anyhow::bail!("daemon authentication failed: {err}");
        }
        Ok(())
    }

    /// Send a JSON request to the daemon and receive the response.
    ///
    /// Writes `{"id":"<uuid>","action":"<action>","params":<params>}\n` to the
    /// socket and reads back a single JSON line. Returns the `data` field on
    /// success, or an `Err` with the daemon's `error` field.
    ///
    /// On a connection error, performs one reconnect+retry (dead daemon recovery).
    pub fn request(&self, action: &str, params: Value) -> Result<Value> {
        self.request_inner(action, params, true)
    }

    /// Inner request implementation. When `allow_retry` is true, a broken
    /// connection triggers a reconnect+retry.
    fn request_inner(&self, action: &str, params: Value, allow_retry: bool) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();

        let request = serde_json::json!({
            "id": id,
            "action": action,
            "params": params,
        });

        let mut line = serde_json::to_string(&request)
            .context("failed to serialize daemon request")?;
        line.push('\n');

        let response_line = {
            let mut stream_guard = self
                .stream
                .lock()
                .map_err(|e| anyhow::anyhow!("daemon stream lock poisoned: {e}"))?;

            let stream = stream_guard
                .as_mut()
                .ok_or_else(|| anyhow::anyhow!("daemon not connected"))?;

            // Write the request.
            stream
                .write_all(line.as_bytes())
                .map_err(|e| {
                    if allow_retry {
                        anyhow::anyhow!("WRITE_FAILED:{e}")
                    } else {
                        anyhow::anyhow!("failed to write to daemon: {e}")
                    }
                })?;

            // Read the response line.
            let mut reader = BufReader::new(&*stream);
            let mut response = String::new();
            reader.read_line(&mut response).map_err(|e| {
                if allow_retry {
                    anyhow::anyhow!("READ_FAILED:{e}")
                } else {
                    anyhow::anyhow!("failed to read from daemon: {e}")
                }
            })?;

            response
        };

        // Check for dead-connection errors → retry once.
        if (response_line.starts_with("WRITE_FAILED:")
            || response_line.starts_with("READ_FAILED:"))
            && allow_retry
        {
            self.reconnect()?;
            return self.request_inner(action, params, false);
        }

        let response_str = response_line.trim();
        if response_str.is_empty() {
            if allow_retry {
                self.reconnect()?;
                return self.request_inner(action, params, false);
            }
            anyhow::bail!("browser daemon returned empty response");
        }

        let response: Value =
            serde_json::from_str(response_str).context("browser daemon returned invalid JSON")?;

        // Validate the response id matches.
        let resp_id = response.get("id").and_then(Value::as_str).unwrap_or("");
        if resp_id != id {
            // ID mismatch — possible stale response. Retry once.
            if allow_retry {
                self.reconnect()?;
                return self.request_inner(action, params, false);
            }
            anyhow::bail!(
                "browser daemon response id mismatch: expected {id}, got {resp_id}"
            );
        }

        // Check for error.
        if let Some(err) = response.get("error").and_then(Value::as_str) {
            if !err.is_empty() {
                anyhow::bail!("browser daemon error: {err}");
            }
        }

        // Return the `data` field.
        Ok(response
            .get("data")
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Attempt to reconnect to the daemon after a broken connection.
    ///
    /// Closes the old stream, waits briefly for the daemon to rebind, then
    /// reconnects and re-authenticates.
    fn reconnect(&self) -> Result<()> {
        // Close old stream.
        {
            let mut guard = self
                .stream
                .lock()
                .map_err(|e| anyhow::anyhow!("stream lock poisoned: {e}"))?;
            *guard = None;
        }

        // Wait for the socket to become available again.
        std::thread::sleep(Duration::from_millis(500));

        // Check the daemon is still alive.
        {
            let mut guard = self
                .child
                .lock()
                .map_err(|e| anyhow::anyhow!("child lock poisoned: {e}"))?;
            if let Some(ref mut child) = *guard {
                // Try to check if the process has exited.
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        // Daemon exited — restart it.
                        drop(guard);
                        return self.restart_daemon();
                    }
                    Ok(None) => {
                        // Still running — the socket may have been recreated.
                    }
                    Err(e) => {
                        drop(guard);
                        anyhow::bail!("failed to check daemon status: {e}");
                    }
                }
            } else {
                drop(guard);
                return self.restart_daemon();
            }
        }

        // Try to connect.
        let stream = match std::os::unix::net::UnixStream::connect(&self.socket_path) {
            Ok(s) => s,
            Err(e) => {
                // Socket gone — restart the daemon.
                anyhow::bail!("reconnect failed (will restart): {e}");
            }
        };
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .context("failed to set read timeout")?;
        stream
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .context("failed to set write timeout")?;

        {
            let mut guard = self
                .stream
                .lock()
                .map_err(|e| anyhow::anyhow!("stream lock poisoned: {e}"))?;
            *guard = Some(stream);
        }

        // Re-authenticate: send raw token then read auth response.
        self.send_raw_token()?;
        self.read_auth_response()?;

        Ok(())
    }

    /// Restart the daemon subprocess from scratch.
    fn restart_daemon(&self) -> Result<()> {
        // Kill the old process if it still exists.
        {
            let mut guard = self
                .child
                .lock()
                .map_err(|e| anyhow::anyhow!("child lock poisoned: {e}"))?;
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        // Remove stale socket.
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }

        // Spawn a new subprocess.
        let python = crate::internet::venv_python()
            .context("internet research environment not installed")?;

        let mut cmd = Command::new(&python);
        cmd.arg("-m")
            .arg("scrapion_agent")
            .arg("daemon")
            .arg("--socket")
            .arg(&self.socket_path)
            .arg("--token")
            .arg(&self.auth_token)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        crate::tool::shell::no_console_window(&mut cmd);

        let mut child = cmd
            .spawn()
            .context("failed to restart browser daemon subprocess")?;

        // Read the ready line.
        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("daemon stdout not captured"))?;
        let mut reader = BufReader::new(stdout);
        let mut ready_line = String::new();
        reader
            .read_line(&mut ready_line)
            .context("failed to read daemon ready line on restart")?;

        let ready_line = ready_line.trim();
        if ready_line != "ready" && ready_line != self.auth_token {
            let stderr_msg = read_child_stderr(&mut child);
            let _ = child.kill();
            let _ = child.wait();
            let detail = if stderr_msg.is_empty() {
                format!("unexpected ready line on restart: {ready_line:?}")
            } else {
                format!("failed to restart:\n{stderr_msg}")
            };
            crate::model::store::append_global_error_log("browser-daemon", &detail);
            anyhow::bail!("browser daemon {detail}");
        }

        // Wait for socket.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !self.socket_path.exists() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                let detail = "socket did not appear within 10s on restart".to_string();
                crate::model::store::append_global_error_log("browser-daemon", &detail);
                anyhow::bail!("browser daemon {detail}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        // Connect + auth.
        let stream = std::os::unix::net::UnixStream::connect(&self.socket_path)
            .context("failed to connect to browser daemon on restart")?;
        stream
            .set_read_timeout(Some(REQUEST_TIMEOUT))
            .context("failed to set read timeout")?;
        stream
            .set_write_timeout(Some(REQUEST_TIMEOUT))
            .context("failed to set write timeout")?;

        {
            let mut guard = self
                .stream
                .lock()
                .map_err(|e| anyhow::anyhow!("stream lock poisoned: {e}"))?;
            *guard = Some(stream);
        }
        {
            let mut guard = self
                .child
                .lock()
                .map_err(|e| anyhow::anyhow!("child lock poisoned: {e}"))?;
            *guard = Some(child);
        }

        // Re-authenticate: send raw token then read auth response.
        self.send_raw_token()?;
        self.read_auth_response()?;

        Ok(())
    }

    /// Send a shutdown signal to the daemon and wait for it to exit.
    pub fn shutdown(&self) {
        // Best-effort shutdown request.
        let _ = self.request_inner(
            "shutdown",
            serde_json::json!({}),
            false,
        );

        // Wait up to SHUTDOWN_WAIT for graceful exit.
        std::thread::sleep(SHUTDOWN_WAIT);

        // Kill if still running.
        if let Ok(mut guard) = self.child.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
                let _ = child.wait();
            }
            *guard = None;
        }

        // Close the stream.
        if let Ok(mut guard) = self.stream.lock() {
            *guard = None;
        }

        // Remove the socket file.
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    /// Returns `true` if the daemon subprocess appears to still be running
    /// and the socket is connected.
    pub fn is_alive(&self) -> bool {
        // Check the child process.
        if let Ok(mut guard) = self.child.lock() {
            if let Some(ref mut child) = *guard {
                match child.try_wait() {
                    Ok(Some(_)) => return false, // Process exited.
                    Ok(None) => {}                // Still running.
                    Err(_) => return false,
                }
            } else {
                return false;
            }
        } else {
            return false;
        }

        // Check the stream.
        if let Ok(guard) = self.stream.lock() {
            guard.is_some()
        } else {
            false
        }
    }

    /// Get the path to the daemon socket.
    #[cfg(test)]
    fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for BrowserDaemon {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Get or start a daemon for the given session directory.
///
/// Returns a cached daemon if one exists and is alive. Otherwise starts a new
/// daemon, caches it, and returns it. This is the primary entry point for
/// browser tools.
pub fn get_or_start(session_dir: &Path) -> Result<Arc<BrowserDaemon>> {
    let map = DAEMONS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = map
        .lock()
        .map_err(|e| anyhow::anyhow!("daemon registry lock poisoned: {e}"))?;

    if let Some(d) = map.get(session_dir) {
        if d.is_alive() {
            return Ok(d.clone());
        }
        // Dead — remove and fall through to restart.
        map.remove(session_dir);
    }

    let daemon = BrowserDaemon::start(session_dir)?;
    map.insert(session_dir.to_path_buf(), daemon.clone());
    Ok(daemon)
}

/// Remove the daemon for a session directory from the registry and shut it down.
///
/// Called when a session closes so the subprocess is not leaked.
pub fn cleanup(session_dir: &Path) {
    let Some(map) = DAEMONS.get() else {
        return;
    };
    if let Ok(mut guard) = map.lock() {
        if let Some(daemon) = guard.remove(session_dir) {
            daemon.shutdown();
        }
    }
}

/// Validate a URL against SSRF protections.
///
/// Rejects loopback, private/link-local, and cloud-metadata addresses.
/// Returns `Ok(())` if the URL is safe, or `Err(message)` if rejected.
pub fn validate_url_safe(url: &str) -> Result<()> {
    use std::net::IpAddr;

    // Must be http or https.
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("URL must start with http:// or https://");
    }

    // Parse the URL to extract the host.
    let parsed = url::Url::parse(url).context("invalid URL")?;
    let host_str = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    // Check for cloud metadata endpoint (exact match or suffix).
    if host_str == "169.254.169.254" {
        anyhow::bail!(
            "URL targets cloud metadata endpoint (169.254.169.254) — blocked for SSRF protection"
        );
    }

    // Check for localhost aliases.
    if host_str.eq_ignore_ascii_case("localhost") {
        anyhow::bail!(
            "URL targets localhost — blocked for SSRF protection"
        );
    }

    // If the host is an IP address, check private/reserved ranges.
    // Strip brackets for IPv6 (url crate returns "[::1]" with brackets).
    let host_for_ip = host_str.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(host_str);
    if let Ok(ip) = host_for_ip.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_loopback() {
                    anyhow::bail!(
                        "URL targets loopback address ({v4}) — blocked for SSRF protection"
                    );
                }
                if v4.is_link_local() {
                    anyhow::bail!(
                        "URL targets link-local address ({v4}) — blocked for SSRF protection"
                    );
                }
                if v4.is_private() {
                    anyhow::bail!(
                        "URL targets private network ({v4}) — blocked for SSRF protection"
                    );
                }
                // 169.254.x.x (already checked exact above, but cover the range).
                let octets = v4.octets();
                if octets[0] == 169 && octets[1] == 254 {
                    anyhow::bail!(
                        "URL targets link-local range (169.254.x.x) — blocked for SSRF protection"
                    );
                }
                // 0.0.0.0
                if v4.is_unspecified() {
                    anyhow::bail!(
                        "URL targets unspecified address ({v4}) — blocked for SSRF protection"
                    );
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_loopback() {
                    anyhow::bail!(
                        "URL targets IPv6 loopback ({v6}) — blocked for SSRF protection"
                    );
                }
                if v6.is_unspecified() {
                    anyhow::bail!(
                        "URL targets IPv6 unspecified ({v6}) — blocked for SSRF protection"
                    );
                }
                // fc00::/7 (unique local address).
                let segments = v6.segments();
                if (segments[0] & 0xfe00) == 0xfc00 {
                    anyhow::bail!(
                        "URL targets IPv6 unique-local address (fc00::/7) — blocked for SSRF protection"
                    );
                }
                // fe80::/10 (link-local).
                if (segments[0] & 0xffc0) == 0xfe80 {
                    anyhow::bail!(
                        "URL targets IPv6 link-local address (fe80::/10) — blocked for SSRF protection"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Read up to 4 KB from a child process's stderr and return it as a trimmed string.
///
/// Used to surface Python tracebacks when the daemon crashes during startup.
/// Consumes the stderr handle so the child can be killed/waited afterwards.
fn read_child_stderr(child: &mut Child) -> String {
    use std::io::Read;
    let Some(ref mut stderr) = child.stderr else {
        return String::new();
    };
    let mut buf = [0u8; 4096];
    let n = stderr.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).trim().to_string()
}

/// Generate a random hex token of `TOKEN_BYTES` bytes.
fn generate_token() -> anyhow::Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom_fill(&mut bytes)?;
    Ok(hex::encode(bytes))
}

/// Fill a byte slice with cryptographically random data.
fn getrandom_fill(buf: &mut [u8]) -> anyhow::Result<()> {
    use std::io::Read;
    let mut f = std::fs::File::open("/dev/urandom")
        .map_err(|e| anyhow::anyhow!("failed to open /dev/urandom: {e}"))?;
    f.read_exact(buf)
        .map_err(|e| anyhow::anyhow!("failed to read random bytes: {e}"))
}

// Minimal hex encoding (avoids pulling in a full hex crate).
mod hex {
    pub fn encode(bytes: [u8; super::TOKEN_BYTES]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validate_url_safe_rejects_loopback_v4() {
        assert!(validate_url_safe("http://127.0.0.1/secret").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_localhost() {
        assert!(validate_url_safe("http://localhost/admin").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_private_10() {
        assert!(validate_url_safe("http://10.0.0.1/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_private_172() {
        assert!(validate_url_safe("http://172.16.0.1/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_private_192() {
        assert!(validate_url_safe("http://192.168.1.1/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_link_local() {
        assert!(validate_url_safe("http://169.254.1.1/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_cloud_metadata() {
        assert!(validate_url_safe("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_ipv6_loopback() {
        assert!(validate_url_safe("http://[::1]/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_ipv6_unique_local() {
        assert!(validate_url_safe("http://[fc00::1]/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_ipv6_link_local() {
        assert!(validate_url_safe("http://[fe80::1]/").is_err());
    }

    #[test]
    fn validate_url_safe_rejects_non_http() {
        assert!(validate_url_safe("ftp://example.com/").is_err());
        assert!(validate_url_safe("file:///etc/passwd").is_err());
    }

    #[test]
    fn validate_url_safe_allows_public() {
        assert!(validate_url_safe("https://example.com/").is_ok());
        assert!(validate_url_safe("https://docs.rust-lang.org/").is_ok());
        assert!(validate_url_safe("http://8.8.8.8/").is_ok());
    }

    #[test]
    fn generate_token_is_correct_length() {
        let token = generate_token().unwrap();
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        // All hex chars.
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

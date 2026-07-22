//! One-shot HTTP loopback listener that catches an OAuth redirect on a
//! caller-specified loopback port. No web framework: this is a single GET
//! request on a throwaway listener, so a hand-rolled parse of the request
//! line is simpler than pulling in a server dependency for it.

use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration, Instant};

/// The `code` + `state` query parameters lifted off a successful redirect.
pub struct CallbackResult {
    pub code: String,
    /// Kept for parity with the redirect's query params and for tests; the CSRF
    /// check already happened inside `catch_callback` before this is returned, so
    /// no caller needs to re-read it.
    #[allow(dead_code)]
    pub state: String,
}

const MAX_REQUEST_BYTES: usize = 8 * 1024;

const SUCCESS_BODY: &str = "<html><body style=\"font-family:monospace\"><h3>koma: login complete</h3>You can close this tab.</body></html>";
const FAILURE_BODY: &str = "<html><body style=\"font-family:monospace\"><h3>koma: login failed</h3>You can close this tab and return to the terminal.</body></html>";

/// Wait up to `timeout_secs` for the OAuth redirect on `127.0.0.1:port`,
/// validate `state`, and return the authorization `code`.
///
/// Browsers sometimes probe unrelated paths (e.g. `/favicon.ico`) before the
/// real redirect lands; those get a bare 404 and the loop keeps waiting,
/// still bounded by the overall `timeout_secs` deadline.
pub async fn catch_callback(
    expected_state: &str,
    timeout_secs: u64,
    port: u16,
) -> Result<CallbackResult, String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|_| format!("port {port} busy — close any running CLI on it and retry"))?;

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("timed out waiting for the OAuth callback".to_string());
        }

        let (mut stream, _addr) = match timeout(remaining, listener.accept()).await {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(format!("loopback accept failed: {e}")),
            Err(_) => return Err("timed out waiting for the OAuth callback".to_string()),
        };

        let request_line = match read_request_line(&mut stream).await {
            Some(line) => line,
            None => continue, // malformed/empty read; keep waiting for the real redirect
        };

        let params = parse_query(&request_line);
        let has_code = params.iter().any(|(k, _)| k == "code");
        let has_error = params.iter().any(|(k, _)| k == "error");

        if !has_code && !has_error {
            // Not the callback we're waiting for (e.g. /favicon.ico) — 404 and keep listening.
            let _ = write_response(&mut stream, "404 Not Found", "").await;
            continue;
        }

        if has_error {
            let error = params
                .iter()
                .find(|(k, _)| k == "error")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let _ = write_response(&mut stream, "200 OK", FAILURE_BODY).await;
            return Err(format!("login denied or failed: {error}"));
        }

        let code = params
            .iter()
            .find(|(k, _)| k == "code")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let state = params
            .iter()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        if state != expected_state {
            let _ = write_response(&mut stream, "200 OK", FAILURE_BODY).await;
            return Err("state mismatch — possible CSRF, aborting login".to_string());
        }

        let _ = write_response(&mut stream, "200 OK", SUCCESS_BODY).await;
        return Ok(CallbackResult { code, state });
    }
}

/// Read bytes off `stream` until `\r\n\r\n` (end of headers) or the byte cap,
/// then return just the first line (the `GET /path?query HTTP/1.1` line).
async fn read_request_line(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(512);
    let mut chunk = [0u8; 512];
    loop {
        if buf.len() >= MAX_REQUEST_BYTES {
            break;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    text.lines().next().map(|s| s.to_string())
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    body: &str,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let response =
        format!("HTTP/1.1 {status}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}");
    stream.write_all(response.as_bytes()).await
}

/// Parse the query string out of an HTTP request line
/// (`GET /auth/callback?code=...&state=... HTTP/1.1`), percent-decoding each
/// value. Returns an empty vec for a request line with no query string
/// (or that doesn't parse as `GET <path> HTTP/...` at all).
pub(crate) fn parse_query(line: &str) -> Vec<(String, String)> {
    let rest = match line.strip_prefix("GET ") {
        Some(r) => r,
        None => return Vec::new(),
    };
    let path_and_query = match rest.split(" HTTP/").next() {
        Some(p) => p,
        None => return Vec::new(),
    };
    let query = match path_and_query.split_once('?') {
        Some((_, q)) => q,
        None => return Vec::new(),
    };

    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(pair), String::new()),
        })
        .collect()
}

/// Minimal `application/x-www-form-urlencoded`-style decode: `+` becomes a
/// space, `%XX` becomes the corresponding byte, malformed escapes pass
/// through literally. Decoded bytes are lossily reassembled as UTF-8 (query
/// param values here are ASCII in practice: opaque codes/state/error tokens).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_and_state() {
        let line = "GET /auth/callback?code=abc123&state=xyz789 HTTP/1.1";
        let params = parse_query(line);
        assert_eq!(
            params,
            vec![
                ("code".to_string(), "abc123".to_string()),
                ("state".to_string(), "xyz789".to_string()),
            ]
        );
    }

    #[test]
    fn parses_error_param() {
        let line = "GET /auth/callback?error=access_denied&state=xyz789 HTTP/1.1";
        let params = parse_query(line);
        assert_eq!(
            params,
            vec![
                ("error".to_string(), "access_denied".to_string()),
                ("state".to_string(), "xyz789".to_string()),
            ]
        );
    }

    #[test]
    fn favicon_request_has_no_params() {
        let line = "GET /favicon.ico HTTP/1.1";
        assert!(parse_query(line).is_empty());
    }

    #[test]
    fn decodes_percent_encoded_values() {
        let line = "GET /auth/callback?code=a%20b%2Bc&state=hello%2Fworld HTTP/1.1";
        let params = parse_query(line);
        assert_eq!(
            params,
            vec![
                ("code".to_string(), "a b+c".to_string()),
                ("state".to_string(), "hello/world".to_string()),
            ]
        );
    }
}

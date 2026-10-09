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

/// Bundled product icon for the loopback success/failure page.
fn logo_data_uri() -> String {
    use base64::Engine;
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/icon-128.png"
    ));
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Centered OAuth result page: koma logo, short status copy, and a button that
/// tries to close the tab so the user can return to the app.
fn callback_page(ok: bool) -> String {
    let (title, heading, body, hint, heading_class) = if ok {
        (
            "koma — signed in",
            "You're signed in",
            "koma has your login. You can return to the app and keep working.",
            "This browser tab is no longer needed.",
            "ok",
        )
    } else {
        (
            "koma — sign-in failed",
            "Sign-in didn't finish",
            "The login was cancelled or failed. Return to koma and try again.",
            "You can close this tab and go back to the app.",
            "err",
        )
    };
    let logo = logo_data_uri();
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
<style>
  :root {{ color-scheme: light dark; }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0;
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
    background: #0f1115;
    color: #e8eaed;
  }}
  .card {{
    width: min(420px, 100%);
    text-align: center;
    padding: 40px 32px 32px;
    border-radius: 16px;
    background: #1a1d24;
    border: 1px solid #2a2f3a;
    box-shadow: 0 20px 50px rgba(0,0,0,.35);
  }}
  .logo {{
    width: 72px;
    height: 72px;
    border-radius: 16px;
    display: block;
    margin: 0 auto 20px;
    box-shadow: 0 8px 24px rgba(0,0,0,.25);
  }}
  h1 {{
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0 0 10px;
    letter-spacing: -0.02em;
  }}
  h1.ok {{ color: #6bcf8e; }}
  h1.err {{ color: #f07178; }}
  p {{
    margin: 0 0 8px;
    font-size: 0.95rem;
    line-height: 1.5;
    color: #a8b0bd;
  }}
  .hint {{
    font-size: 0.85rem;
    color: #7a8494;
    margin: 0 0 28px;
  }}
  .btn {{
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 180px;
    height: 40px;
    padding: 0 18px;
    border: none;
    border-radius: 10px;
    cursor: pointer;
    font: inherit;
    font-weight: 600;
    font-size: 0.95rem;
    background: #5b8def;
    color: #fff;
  }}
  .btn:hover {{ filter: brightness(1.08); }}
  .btn:active {{ transform: translateY(1px); }}
</style>
</head>
<body>
  <main class="card">
    <img class="logo" width="72" height="72" alt="koma" src="{logo}"/>
    <h1 class="{heading_class}">{heading}</h1>
    <p>{body}</p>
    <p class="hint" id="hint">{hint}</p>
    <button class="btn" type="button" onclick="goBack()">Back to koma</button>
  </main>
  <script>
    function goBack() {{
      window.close();
      // Browsers often block window.close() unless the tab was script-opened.
      setTimeout(function () {{
        var h = document.getElementById('hint');
        if (h) h.textContent = 'Close this tab manually, then switch back to koma.';
      }}, 250);
    }}
  </script>
</body>
</html>"#
    )
}

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
            let _ = write_response(&mut stream, "200 OK", &callback_page(false)).await;
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
            let _ = write_response(&mut stream, "200 OK", &callback_page(false)).await;
            return Err("state mismatch — possible CSRF, aborting login".to_string());
        }

        let _ = write_response(&mut stream, "200 OK", &callback_page(true)).await;
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
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
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
#[path = "loopback_test.rs"]
mod tests;

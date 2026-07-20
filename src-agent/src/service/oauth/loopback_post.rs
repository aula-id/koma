//! One-shot HTTP loopback listener that catches a POST callback (Command Code's
//! browser-to-localhost API key flow). Unlike the GET-only `loopback.rs` (which
//! catches OAuth redirect query strings), this server handles CORS preflight
//! (OPTIONS) and a JSON POST body — needed because Command Code's Studio website
//! POSTs the API key back rather than redirecting with query params.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration, Instant};

/// The parsed POST callback body from Command Code's Studio website.
#[derive(Debug, Clone)]
pub struct PostCallback {
    pub api_key: String,
    /// CSRF state (already validated against the expected value before return).
    #[allow(dead_code)]
    pub state: String,
    pub user_id: String,
    pub user_name: String,
    #[allow(dead_code)]
    pub key_name: String,
}

const MAX_BODY_BYTES: usize = 10 * 1024;

const CORS_ORIGINS: &[&str] = &[
    "http://localhost:3000",
    "https://staging.commandcode.ai",
    "https://commandcode.ai",
];

/// Bind a POST loopback server on `127.0.0.1`, trying ports
/// `start..start+range` then falling back to port 0. Returns the bound port
/// and a future that resolves when a valid POST callback arrives or the
/// timeout expires.
pub async fn catch_post_callback(
    expected_state: &str,
    timeout_secs: u64,
) -> Result<(u16, impl std::future::Future<Output = Result<PostCallback, String>>), String> {
    let start = super::registry::COMMANDCODE_PORT_START;
    let range = super::registry::COMMANDCODE_PORT_RANGE;

    let mut listener = None;
    // Try the specified range first.
    for port in start..start.saturating_add(range) {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => {
                listener = Some((port, l));
                break;
            }
            Err(_) => continue,
        }
    }
    // Fallback: OS-assigned port.
    if listener.is_none() {
        match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => {
                let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
                listener = Some((port, l));
            }
            Err(e) => return Err(format!("failed to bind loopback: {e}")),
        }
    }

    let (port, listener) = listener.unwrap();
    let state = expected_state.to_string();

    let fut = async move {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs);

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("timed out waiting for the Command Code callback".to_string());
            }

            let (mut stream, _addr) = match timeout(remaining, listener.accept()).await {
                Ok(Ok(pair)) => pair,
                Ok(Err(e)) => return Err(format!("loopback accept failed: {e}")),
                Err(_) => {
                    return Err("timed out waiting for the Command Code callback".to_string())
                }
            };

            // Read headers first (through \r\n\r\n), then the body up to Content-Length.
            let mut buf = Vec::with_capacity(1024);
            let mut chunk = [0u8; 1024];
            loop {
                if buf.len() >= MAX_BODY_BYTES {
                    break;
                }
                let n = match stream.read(&mut chunk).await {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }

            let header_end = match buf.windows(4).position(|w| w == b"\r\n\r\n") {
                Some(p) => p + 4,
                None => continue, // malformed / empty — keep waiting
            };
            // Own the header-derived strings BEFORE reading more body bytes into
            // `buf` (which would invalidate a borrow of `buf[..header_end]`).
            let (method, path, origin, content_length) = {
                let text = String::from_utf8_lossy(&buf[..header_end]);
                let request_line = text.lines().next().unwrap_or("");
                let mut parts = request_line.splitn(3, ' ');
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let origin = extract_header(&text, "Origin");
                let content_length = extract_header(&text, "Content-Length")
                    .parse::<usize>()
                    .unwrap_or(0)
                    .min(MAX_BODY_BYTES);
                (method, path, origin, content_length)
            };

            // Drain any remaining body bytes promised by Content-Length.
            while buf.len() < header_end + content_length && buf.len() < MAX_BODY_BYTES {
                let n = match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&chunk[..n]);
            }

            match method.as_str() {
                "OPTIONS" => {
                    // CORS preflight response (incl. Private Network Access).
                    let cors_headers = build_cors_headers(&origin);
                    let response = format!(
                        "HTTP/1.1 204 No Content\r\n{cors_headers}Connection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    continue;
                }
                "POST" => {
                    if path != "/callback" {
                        let cors_headers = build_cors_headers(&origin);
                        let body = r#"{"success":false,"error":"Not found"}"#;
                        let response = format!(
                            "HTTP/1.1 404 Not Found\r\n{cors_headers}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        continue;
                    }

                    let body_end = (header_end + content_length).min(buf.len());
                    let body_bytes = &buf[header_end..body_end];

                    // Parse JSON body.
                    let parsed: serde_json::Value = match serde_json::from_slice(body_bytes) {
                        Ok(v) => v,
                        Err(_) => {
                            let _ = write_json_response(
                                &mut stream,
                                &origin,
                                "400 Bad Request",
                                r#"{"success":false,"error":"Invalid JSON"}"#,
                            )
                            .await;
                            continue;
                        }
                    };

                    // Check for error field (Studio reports denial as JSON error).
                    if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
                        let desc = parsed
                            .get("error_description")
                            .and_then(|v| v.as_str())
                            .unwrap_or(err);
                        let _ = write_json_response(
                            &mut stream,
                            &origin,
                            "200 OK",
                            r#"{"success":true}"#,
                        )
                        .await;
                        return Err(format!("login denied or failed: {desc}"));
                    }

                    // Extract required fields (slightly looser than pi: apiKey+state
                    // enough; optional identity fields default empty).
                    let api_key = parsed
                        .get("apiKey")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let cb_state = parsed
                        .get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    if api_key.is_empty() || cb_state.is_empty() {
                        let _ = write_json_response(
                            &mut stream,
                            &origin,
                            "400 Bad Request",
                            r#"{"success":false,"error":"Missing required fields"}"#,
                        )
                        .await;
                        continue;
                    }

                    if cb_state != state {
                        let _ = write_json_response(
                            &mut stream,
                            &origin,
                            "400 Bad Request",
                            r#"{"success":false,"error":"state mismatch"}"#,
                        )
                        .await;
                        return Err(
                            "state mismatch — possible CSRF, aborting login".to_string()
                        );
                    }

                    let user_id = field_str(&parsed, "userId");
                    let user_name = field_str(&parsed, "userName");
                    let key_name = field_str(&parsed, "keyName");

                    let _ = write_json_response(
                        &mut stream,
                        &origin,
                        "200 OK",
                        r#"{"success":true}"#,
                    )
                    .await;
                    return Ok(PostCallback {
                        api_key,
                        state: cb_state,
                        user_id,
                        user_name,
                        key_name,
                    });
                }
                _ => {
                    let cors_headers = build_cors_headers(&origin);
                    let response = format!(
                        "HTTP/1.1 405 Method Not Allowed\r\n{cors_headers}Allow: POST, OPTIONS\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    continue;
                }
            }
        }
    };

    Ok((port, fut))
}

fn extract_header(headers: &str, name: &str) -> String {
    // Case-insensitive header match (HTTP headers are case-insensitive).
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case(name) {
                return v.trim().to_string();
            }
        }
    }
    String::new()
}

fn field_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn build_cors_headers(origin: &str) -> String {
    // Mirror pi-commandcode: echo an allowed origin (or fall back to the first
    // configured one) so the Studio page's fetch() is not CORS-blocked.
    let response_origin = if CORS_ORIGINS.iter().any(|o| *o == origin) {
        origin
    } else {
        CORS_ORIGINS[0]
    };
    format!(
        "Access-Control-Allow-Origin: {response_origin}\r\n\
Access-Control-Allow-Methods: POST, OPTIONS\r\n\
Access-Control-Allow-Headers: Content-Type\r\n\
Access-Control-Allow-Private-Network: true\r\n"
    )
}

async fn write_json_response(
    stream: &mut tokio::net::TcpStream,
    origin: &str,
    status: &str,
    body: &str,
) -> std::io::Result<()> {
    let cors = build_cors_headers(origin);
    let response = format!(
        "HTTP/1.1 {status}\r\n{cors}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_header_finds_origin() {
        let headers = "POST /callback HTTP/1.1\r\nOrigin: https://commandcode.ai\r\nContent-Type: application/json\r\n\r\n";
        assert_eq!(
            extract_header(headers, "Origin"),
            "https://commandcode.ai"
        );
    }

    #[test]
    fn extract_header_missing() {
        assert_eq!(extract_header("GET / HTTP/1.1\r\n\r\n", "Origin"), "");
    }
}

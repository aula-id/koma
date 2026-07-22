//! `web_download` tool: download a file from a URL and save it to disk.
//!
//! The URL is fetched as raw bytes via a blocking `reqwest` GET on a freshly
//! spawned `std::thread` (so it never touches the tokio runtime), then written
//! to `<download_dir>/<save_name>`. The tool returns a `MEDIA_WORKDIR:` prefix
//! signaling `finish_tool_round` to add the media directory to workspaces so
//! the downloaded file appears in `@`-autocomplete.

use super::{Tool, ToolCtx};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

/// Hard cap on how many bytes may be downloaded in a single call.
const MAX_DOWNLOAD_BYTES: u64 = 500 * 1024 * 1024; // 500 MiB

/// Download a file from a URL and save it to the session media directory.
pub struct WebDownload;

impl Tool for WebDownload {
    fn name(&self) -> &'static str {
        "web_download"
    }

    fn description(&self) -> &'static str {
        "Download a file from a URL and save it to the session media directory. \
        Use when you need to fetch a binary or non-HTML asset like an image."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The full URL to download (must start with http:// or https://)."
                },
                "save_name": {
                    "type": "string",
                    "description": "The filename to save the download as (no path separators allowed)."
                }
            },
            "required": ["url", "save_name"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'url'"))?;

        let save_name = args
            .get("save_name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'save_name'"))?;

        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(format!(
                "error: url must start with http:// or https://, got: {url}"
            ));
        }

        if save_name.is_empty() {
            return Ok("error: save_name must not be empty".to_string());
        }

        if save_name.contains('/') || save_name.contains('\\') {
            return Ok(format!(
                "error: save_name must not contain path separators, got: {save_name}"
            ));
        }

        let download_dir = match &ctx.download_dir {
            Some(d) => d.clone(),
            None => {
                return Ok("error: no download directory available for this session".to_string())
            }
        };

        let full_path = download_dir.join(save_name);

        let (tx, rx) = mpsc::channel::<Result<(u16, u64), String>>();
        let url_owned = url.to_string();
        let path_owned = full_path.clone();

        std::thread::spawn(move || {
            let result = (|| -> Result<(u16, u64), String> {
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(60))
                    .user_agent(
                        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
                    )
                    .build()
                    .map_err(|e| format!("client build error: {e}"))?;

                let resp = client
                    .get(&url_owned)
                    .send()
                    .map_err(|e| format!("request failed: {e}"))?;

                let status = resp.status().as_u16();

                if !(200..300).contains(&status) {
                    return Err(format!("HTTP {status} for {url_owned}"));
                }

                // Content-Length pre-check: reject before reading if the
                // declared size already exceeds the cap.
                if let Some(len) = resp.content_length() {
                    if len > MAX_DOWNLOAD_BYTES {
                        return Err(format!(
                            "download too large ({len} bytes, max {MAX_DOWNLOAD_BYTES})"
                        ));
                    }
                }

                // Bounded read: cap RAM usage even without a Content-Length.
                // reqwest::blocking::Response implements std::io::Read.
                let mut buf = Vec::new();
                resp.take(MAX_DOWNLOAD_BYTES + 1)
                    .read_to_end(&mut buf)
                    .map_err(|e| format!("failed to read body: {e}"))?;
                if buf.len() as u64 > MAX_DOWNLOAD_BYTES {
                    return Err(format!(
                        "download too large (>{MAX_DOWNLOAD_BYTES} bytes, max {MAX_DOWNLOAD_BYTES})"
                    ));
                }

                let size = buf.len() as u64;
                std::fs::write(&path_owned, &buf)
                    .map_err(|e| format!("failed to write file: {e}"))?;

                Ok((status, size))
            })();

            let _ = tx.send(result);
        });

        // The MEDIA_WORKDIR sentinel tells finish_tool_round to add the media/
        // directory (the parent of downloads/) to workspaces so @-autocomplete
        // sees the downloads/ subdirectory.
        let media_root = download_dir
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| download_dir.display().to_string());

        match rx.recv_timeout(Duration::from_secs(90)) {
            Ok(Ok((_status, bytes))) => {
                let saved = full_path.display().to_string();
                Ok(format!(
                    "MEDIA_WORKDIR:{media_root}\nsaved: {saved}\nsize: {bytes} bytes"
                ))
            }
            Ok(Err(e)) => Ok(format!("error: {e}")),
            Err(_) => Ok("error: download timed out after 90s".to_string()),
        }
    }
}

//! Anonymous usage telemetry drain.
//!
//! **Not payments.** Do not send cost, wallet, credits, API keys, or any
//! koma.run billing field. Wire is only `{ id, ts, model, tokens_in,
//! tokens_out }`. Local `usage.sqlite` still records cost for the `/usage`
//! dashboard; this drain strips it.
//!
//! Reads `~/.koma/usage.sqlite` after a persistent watermark and POSTs small
//! batches to koma.run. No OAuth. `X-Koma` is the existing install id.
//!
//! First-run catch-up can be months of local rows (billions of tokens). This
//! loop never dumps that in one shot: 50 events, ≥2.5s between successes,
//! one in-flight request, one process (file lock). Failures leave the
//! watermark alone. Watermark writes are `max(current, new)` so a second
//! process cannot rewind.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::model::app_config::AppConfig;
use crate::model::store;
use crate::model::usage::{events_after, max_usage_id};

const DEFAULT_URL: &str = "https://koma.run/api/v1/telemetry/usage";
pub(crate) const BATCH: usize = 50;
const MIN_GAP: Duration = Duration::from_millis(2500);
/// First-run ledgers can be months of rows. Stretch the gap while the
/// watermark is far behind so a fleet of upgrades cannot stampede Mongo.
const BACKLOG_GAP: Duration = Duration::from_secs(8);
const BACKLOG_THRESHOLD: i64 = 10_000;
const IDLE: Duration = Duration::from_secs(15);
const ERR_GAP: Duration = Duration::from_secs(20);
const LOCK_WAIT: Duration = Duration::from_secs(30);

/// Spawn the drain on a dedicated thread. No-op when `KOMA_TELEMETRY` is
/// `0`/`off`/`false`/`no`. Never blocks the caller.
pub fn spawn() {
    if disabled() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("koma-telemetry".into())
        .spawn(|| loop {
            match lock_file() {
                Some(lock) => {
                    let _lock = lock;
                    run_loop();
                }
                None => std::thread::sleep(LOCK_WAIT),
            }
        });
}

fn disabled() -> bool {
    match std::env::var("KOMA_TELEMETRY") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        Err(_) => false,
    }
}

fn endpoint() -> String {
    std::env::var("KOMA_TELEMETRY_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}

fn lock_path() -> Option<PathBuf> {
    store::base_dir().ok().map(|d| d.join("telemetry.lock"))
}

fn watermark_path() -> Option<PathBuf> {
    store::base_dir().ok().map(|d| d.join("telemetry.json"))
}

fn lock_file() -> Option<File> {
    let path = lock_path()?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = File::options()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(_) => None,
    }
}

fn run_loop() {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let url = endpoint();
    loop {
        let install_id = AppConfig::load().install_id;
        if install_id.len() < 8 {
            std::thread::sleep(IDLE);
            continue;
        }
        let acked = load_acked();
        let events = events_after(acked, BATCH);
        if events.is_empty() {
            std::thread::sleep(IDLE);
            continue;
        }
        let last_id = events.last().map(|e| e.id).unwrap_or(acked);
        let body = WireBody {
            events: events
                .into_iter()
                .filter_map(WireEvent::from_ledger)
                .collect(),
        };
        if body.events.is_empty() {
            // Local rows were all unsendable (empty model, etc.). Advance so
            // a bad row cannot stall a months-long catch-up forever.
            advance_acked(last_id);
            continue;
        }
        match post(&client, &url, &install_id, &body) {
            PostResult::Ok => {
                advance_acked(last_id);
                std::thread::sleep(gap_after_success(max_usage_id().saturating_sub(last_id)));
            }
            PostResult::RetryAfter(d) => std::thread::sleep(d),
            PostResult::Fail => std::thread::sleep(ERR_GAP),
        }
    }
}

#[derive(Serialize)]
struct WireBody {
    events: Vec<WireEvent>,
}

#[derive(Serialize)]
struct WireEvent {
    id: i64,
    ts: i64,
    model: String,
    tokens_in: i64,
    tokens_out: i64,
}

impl WireEvent {
    fn from_ledger(e: crate::model::usage::TelemetryEvent) -> Option<Self> {
        if e.id <= 0 || !valid_model(&e.model_id) {
            return None;
        }
        if e.tokens_in < 0 || e.tokens_out < 0 {
            return None;
        }
        Some(Self {
            id: e.id,
            ts: e.ts,
            model: e.model_id,
            tokens_in: e.tokens_in,
            tokens_out: e.tokens_out,
        })
    }
}

fn valid_model(model: &str) -> bool {
    let n = model.len();
    (1..=128).contains(&n)
        && model.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b':' | b'+' | b'-')
        })
}

enum PostResult {
    Ok,
    RetryAfter(Duration),
    Fail,
}

fn post(
    client: &reqwest::blocking::Client,
    url: &str,
    install_id: &str,
    body: &WireBody,
) -> PostResult {
    let resp = match client
        .post(url)
        .header("X-Koma", install_id)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(body)
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            crate::model::store::append_global_error_log("telemetry", &format!("post error: {e}"));
            return PostResult::Fail;
        }
    };
    let status = resp.status();
    if status.is_success() {
        return PostResult::Ok;
    }
    if status.as_u16() == 429 {
        let wait = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(ERR_GAP);
        return PostResult::RetryAfter(wait);
    }
    crate::model::store::append_global_error_log(
        "telemetry",
        &format!("status {}", status.as_u16()),
    );
    PostResult::Fail
}

/// After a successful batch. Large remaining counts (first-run catch-up
/// from a months-old ledger) use `BACKLOG_GAP` so we do not stampede the
/// ingest. Idle is handled by the empty-queue branch.
pub(crate) fn gap_after_success(remaining: i64) -> Duration {
    if remaining > BACKLOG_THRESHOLD {
        BACKLOG_GAP
    } else {
        MIN_GAP
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Watermark {
    #[serde(default)]
    acked_id: i64,
}

fn load_acked() -> i64 {
    let Some(path) = watermark_path() else {
        return 0;
    };
    let mut buf = String::new();
    if File::open(&path)
        .and_then(|mut f| f.read_to_string(&mut buf))
        .is_err()
    {
        return 0;
    }
    serde_json::from_str::<Watermark>(&buf)
        .map(|w| w.acked_id.max(0))
        .unwrap_or(0)
}

/// Persist `new` only if it is ahead of what is already on disk.
pub(crate) fn advance_acked_at(path: &std::path::Path, new: i64) {
    if new <= 0 {
        return;
    }
    let current = {
        let mut buf = String::new();
        File::open(path)
            .and_then(|mut f| f.read_to_string(&mut buf))
            .ok()
            .and_then(|_| serde_json::from_str::<Watermark>(&buf).ok())
            .map(|w| w.acked_id)
            .unwrap_or(0)
    };
    if new <= current {
        return;
    }
    let json = match serde_json::to_vec_pretty(&Watermark { acked_id: new }) {
        Ok(b) => b,
        Err(_) => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Same POSIX/NTFS rename-over used for config.json — Windows replace-safe.
    let _ = crate::model::memory::atomic_write(path, &json);
}

fn advance_acked(new: i64) {
    let Some(path) = watermark_path() else {
        return;
    };
    advance_acked_at(&path, new);
}

#[cfg(test)]
#[path = "telemetry_test.rs"]
mod telemetry_test;

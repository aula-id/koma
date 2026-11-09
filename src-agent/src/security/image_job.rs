//! One background guest-image pull for the `/security` panel.
//!
//! The panel used to call `pull_composition` on the event loop, which froze the
//! daemon (and every attached client) for the whole download. The pull now runs
//! on its own thread and records the same streamed progress the installer draws.
//! The event loop reads [`progress`] for snapshots and applies [`take_finished`]
//! once per tick. One pull runs at a time.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::images::{pull_composition_inner, Feed};
use super::progress::{PullSink, PullView};

/// Live progress of the running pull, as projected into the security snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImagePullProgress {
    /// Composition id being pulled.
    pub id: String,
    /// `starting` | `contacting` | `waiting` | `downloading` | `preparing` |
    /// `verifying` | `extracting` | `finishing`.
    pub phase: String,
    /// Bytes done in this phase. `total == 0` means the size is not known yet.
    pub current: u64,
    pub total: u64,
    /// Download rate, bytes per second. 0 when idle or unknown.
    pub bps: u64,
    pub layers_done: u32,
    pub layers: u32,
    pub elapsed_ms: u64,
}

/// How the last panel pull ended. Kept until the next pull starts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImagePullResult {
    pub id: String,
    pub ok: bool,
    pub message: String,
}

struct Job {
    progress: ImagePullProgress,
    started: Instant,
    /// Set by the worker when it ends; consumed by [`take_finished`].
    outcome: Option<ImagePullResult>,
}

struct Board {
    job: Option<Job>,
    last: Option<ImagePullResult>,
}

static BOARD: Mutex<Board> = Mutex::new(Board {
    job: None,
    last: None,
});

fn board() -> std::sync::MutexGuard<'static, Board> {
    BOARD.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Start pulling `id` in the background. Refuses while another pull runs.
pub fn start(id: &str) -> Result<(), String> {
    {
        let mut b = board();
        if let Some(job) = b.job.as_ref() {
            return Err(format!("already pulling '{}'", job.progress.id));
        }
        b.last = None;
        b.job = Some(Job {
            progress: ImagePullProgress {
                id: id.to_string(),
                phase: "starting".into(),
                ..Default::default()
            },
            started: Instant::now(),
            outcome: None,
        });
    }
    let owned = id.to_string();
    let spawned = std::thread::Builder::new()
        .name("koma-image-pull".into())
        .spawn(move || {
            // A panic must still end the job, or the panel would stay busy forever.
            let run = std::panic::catch_unwind(|| pull_composition_inner(&owned, false, Feed::Track(&Tracker)));
            let outcome = match run {
                Ok(Ok(report)) if report.skipped => ImagePullResult {
                    id: owned.clone(),
                    ok: true,
                    message: format!("'{owned}' is already pulled"),
                },
                Ok(Ok(_)) => ImagePullResult {
                    id: owned.clone(),
                    ok: true,
                    message: format!("pulled '{owned}'"),
                },
                Ok(Err(err)) => ImagePullResult {
                    id: owned.clone(),
                    ok: false,
                    message: format!("pull '{owned}' failed: {}", one_line(&format!("{err:#}"))),
                },
                Err(_) => ImagePullResult {
                    id: owned.clone(),
                    ok: false,
                    message: format!("pull '{owned}' failed: internal error"),
                },
            };
            if let Some(job) = board().job.as_mut() {
                job.outcome = Some(outcome);
            }
        });
    if let Err(err) = spawned {
        board().job = None;
        return Err(format!("could not start the pull: {err}"));
    }
    Ok(())
}

/// Progress of the running pull, with a fresh elapsed time. `None` when idle.
pub fn progress() -> Option<ImagePullProgress> {
    let b = board();
    let job = b.job.as_ref()?;
    let mut progress = job.progress.clone();
    progress.elapsed_ms = millis(job.started.elapsed());
    Some(progress)
}

/// The id of the running pull, if any.
pub fn busy_id() -> Option<String> {
    board().job.as_ref().map(|job| job.progress.id.clone())
}

/// The last finished pull, kept for the panel until the next one starts.
pub fn last_result() -> Option<ImagePullResult> {
    board().last.clone()
}

/// Take a just-finished pull exactly once; the event loop applies it.
pub fn take_finished() -> Option<ImagePullResult> {
    let mut b = board();
    let done = b.job.as_ref().is_some_and(|job| job.outcome.is_some());
    if !done {
        return None;
    }
    let outcome = b.job.take().and_then(|job| job.outcome)?;
    b.last = Some(outcome.clone());
    Some(outcome)
}

struct Tracker;

impl PullSink for Tracker {
    fn next_spinner(&self) -> u8 {
        0
    }

    fn paint_pull(&self, view: &PullView) {
        let (current, total, bps, layers_done, layers) = view.counters();
        if let Some(job) = board().job.as_mut() {
            let p = &mut job.progress;
            p.phase = view.phase_name().to_string();
            p.current = current;
            p.total = total;
            p.bps = bps;
            p.layers_done = layers_done;
            p.layers = layers;
        }
    }

    fn end_live(&self) {}

    fn info(&self, _text: &str) {}
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn one_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    let mut out: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        out.push('…');
    }
    out
}

#[cfg(test)]
#[path = "image_job_test.rs"]
mod tests;

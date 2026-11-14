//! Per-lane tool state: what a coding session keeps for `bash` jobs and `cd`.
//!
//! A coding session holds this on its `AppState`, which the room does not have.
//! The job runner itself ([`crate::app::bgbash`]) is standalone and used as is;
//! only the thin registry and the `cd` bookkeeping live here. Each lane (an
//! avatar's chat or work lane) has its own jobs and its own working directory,
//! so two lanes never see each other's `bash-1`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use crate::app::bgbash::{kill_bash_job, render_finished_output, spawn_bash_job, BashJob, BashJobStatus};
use crate::tool::cd::CWD_CHANGE_PREFIX;

/// One lane's background jobs.
#[derive(Default)]
pub struct Jobs {
    next: usize,
    jobs: Vec<BashJob>,
}

/// Every lane's jobs and working directory, keyed by `<avatar>.<chat|work>`.
#[derive(Default)]
pub struct LaneState {
    jobs: Mutex<HashMap<String, Jobs>>,
    cwds: Mutex<HashMap<String, PathBuf>>,
}

fn job_number(handle: &str) -> Option<usize> {
    handle.trim().strip_prefix("bash-").unwrap_or(handle.trim()).trim().parse().ok()
}

fn status_line(status: &BashJobStatus) -> String {
    match status {
        BashJobStatus::Running => "[running]".into(),
        BashJobStatus::Done(code) => format!("[exit {code}]"),
        BashJobStatus::Killed => "[killed]".into(),
        BashJobStatus::Error(message) => format!("[error: {message}]"),
    }
}

/// Keep lines matching `pattern`, then the last `tail` of them.
fn filter(out: &str, tail: Option<usize>, pattern: Option<&str>) -> Result<String, String> {
    let mut lines: Vec<&str> = out.lines().collect();
    if let Some(pattern) = pattern {
        let re = regex::Regex::new(pattern).map_err(|e| format!("invalid pattern: {e}"))?;
        lines.retain(|line| re.is_match(line));
    }
    if let Some(n) = tail {
        if n > 0 && lines.len() > n {
            lines = lines.split_off(lines.len() - n);
        }
    }
    Ok(lines.join("\n"))
}

impl LaneState {
    /// The lane's working directory when it has `cd`'d, else `None`.
    pub fn cwd(&self, lane: &str) -> Option<PathBuf> {
        self.cwds.lock().ok()?.get(lane).cloned()
    }

    /// Record a successful `cd` result (`__CWD_CHANGE__:<path>`) and say so in words.
    /// Anything else is the tool's own refusal and passes through.
    pub fn apply_cd(&self, lane: &str, result: String) -> String {
        match result.strip_prefix(CWD_CHANGE_PREFIX) {
            Some(path) => {
                if let Ok(mut cwds) = self.cwds.lock() {
                    cwds.insert(lane.to_string(), PathBuf::from(path.trim()));
                }
                format!("working directory is now {}", path.trim())
            }
            None => result,
        }
    }

    /// Start a detached `bash` job in `cwd` and return what the model is told.
    pub fn start(&self, lane: &str, command: &str, cwd: PathBuf) -> String {
        let Ok(mut all) = self.jobs.lock() else { return "error: job registry is unavailable".into() };
        let jobs = all.entry(lane.to_string()).or_default();
        jobs.next += 1;
        let id = jobs.next;
        jobs.jobs.push(spawn_bash_job(id, command.to_string(), cwd, None, None, None));
        format!(
            "started background job bash-{id} (running). Poll with bash_output{{\"job_id\":\"bash-{id}\"}}, \
             stop with bash_kill{{\"job_id\":\"bash-{id}\"}}."
        )
    }

    /// The `bash_output` result for one job.
    pub fn output(&self, lane: &str, args: &Value) -> String {
        let handle = args.get("job_id").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let tail = args.get("tail_lines").and_then(Value::as_u64).map(|n| n as usize);
        let pattern = args.get("pattern").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty());
        let Ok(all) = self.jobs.lock() else { return "error: job registry is unavailable".into() };
        let Some(job) = job_number(&handle).and_then(|n| all.get(lane)?.jobs.iter().find(|j| j.id == n)) else {
            return format!("error: no such job: {handle}");
        };
        let status = job.snapshot_status();
        let line = status_line(&status);
        let out = job.output_snapshot();
        if out.is_empty() {
            return format!("{line}\n(no output yet)");
        }
        if tail.is_some() || pattern.is_some() {
            return match filter(&out, tail, pattern) {
                Ok(text) if text.is_empty() => format!("{line}\n(no matching lines)"),
                Ok(text) => format!("{line}\n{text}"),
                Err(message) => format!("{line}\n[{message}]"),
            };
        }
        if let BashJobStatus::Done(code) = status {
            if !job.has_delivered_output() {
                let (text, _) = render_finished_output(&job.command, &out, code, true);
                job.mark_output_delivered();
                return format!("{line}\n{text}");
            }
        }
        let delta = job.take_new_output();
        if delta.is_empty() { format!("{line}\n(no new output)") } else { format!("{line}\n{delta}") }
    }

    /// The `bash_kill` result for one job.
    pub fn kill(&self, lane: &str, args: &Value) -> String {
        let handle = args.get("job_id").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let Ok(all) = self.jobs.lock() else { return "error: job registry is unavailable".into() };
        match job_number(&handle).and_then(|n| all.get(lane)?.jobs.iter().find(|j| j.id == n)) {
            Some(job) => {
                kill_bash_job(job);
                format!("job bash-{} killed", job.id)
            }
            None => format!("error: no such job: {handle}"),
        }
    }

    /// Stop every running job, when the room closes.
    pub fn kill_all(&self) {
        if let Ok(all) = self.jobs.lock() {
            for job in all.values().flat_map(|jobs| jobs.jobs.iter()) {
                if job.is_running() {
                    kill_bash_job(job);
                }
            }
        }
    }
}

impl Drop for LaneState {
    fn drop(&mut self) {
        self.kill_all();
    }
}

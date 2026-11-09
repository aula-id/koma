//! stderr progress for `koma --security-install`, in the same shape as `install.sh`:
//! a bold step, a dim detail line, a green result, and one live bar while the
//! guest image downloads. A terminal gets `\r` updates. Anywhere else gets a
//! new line when the phase changes, the percent moves, or five seconds pass,
//! so a log never sits still for the whole pull.

use std::collections::BTreeMap;
use std::io::{stderr, Write};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const BAR_WIDTH: usize = 32;
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

/// Where a streamed pull reports. The installer draws a stderr bar; the
/// `/security` panel records it for the snapshot (see `image_job`).
pub(super) trait PullSink: Sync {
    fn next_spinner(&self) -> u8;
    fn paint_pull(&self, view: &PullView);
    fn end_live(&self);
    fn info(&self, text: &str);
}

impl PullSink for InstallUi {
    fn next_spinner(&self) -> u8 {
        InstallUi::next_spinner(self)
    }
    fn paint_pull(&self, view: &PullView) {
        InstallUi::paint_pull(self, view)
    }
    fn end_live(&self) {
        InstallUi::end_live(self)
    }
    fn info(&self, text: &str) {
        InstallUi::info(self, text)
    }
}

pub(super) struct InstallUi {
    tty: bool,
    bold: &'static str,
    dim: &'static str,
    green: &'static str,
    reset: &'static str,
    step_mark: &'static str,
    ok_mark: &'static str,
    spinner: AtomicU8,
    draw: Mutex<DrawState>,
}

struct DrawState {
    width: usize,
    last_at: Option<Instant>,
    last_pct: Option<u16>,
    last_phase: Option<Phase>,
}

impl InstallUi {
    pub(super) fn new() -> Self {
        let tty = std::io::IsTerminal::is_terminal(&stderr());
        let color = tty && std::env::var_os("NO_COLOR").is_none();
        Self {
            tty,
            bold: if color { "\u{1b}[1m" } else { "" },
            dim: if color { "\u{1b}[2m" } else { "" },
            green: if color { "\u{1b}[32m" } else { "" },
            reset: if color { "\u{1b}[0m" } else { "" },
            step_mark: if tty { "▸" } else { ">" },
            ok_mark: if tty { "✓" } else { "+" },
            spinner: AtomicU8::new(0),
            draw: Mutex::new(DrawState {
                width: 0,
                last_at: None,
                last_pct: None,
                last_phase: None,
            }),
        }
    }

    pub(super) fn banner(&self, host: &str, into: &str, image: &str) {
        self.end_live();
        let mut err = stderr();
        let _ = writeln!(err);
        let _ = writeln!(err, "{}koma security install{}", self.bold, self.reset);
        let _ = writeln!(err, "  {}host     {host}{}", self.dim, self.reset);
        let _ = writeln!(err, "  {}into     {into}{}", self.dim, self.reset);
        let _ = writeln!(err, "  {}image    {image}{}", self.dim, self.reset);
        let _ = writeln!(err);
        let _ = err.flush();
    }

    pub(super) fn step(&self, label: &str) {
        self.end_live();
        let mut err = stderr();
        let _ = writeln!(err, "{}{} {label}{}", self.bold, self.step_mark, self.reset);
        let _ = err.flush();
    }

    pub(super) fn info(&self, text: &str) {
        self.end_live();
        let mut err = stderr();
        let _ = writeln!(err, "  {}{text}{}", self.dim, self.reset);
        let _ = err.flush();
    }

    pub(super) fn ok(&self, text: &str) {
        self.end_live();
        let mut err = stderr();
        let _ = writeln!(err, "  {}{}{} {text}", self.green, self.ok_mark, self.reset);
        let _ = err.flush();
    }

    pub(super) fn finish(&self, lines: &[&str]) {
        self.end_live();
        let mut err = stderr();
        let _ = writeln!(err);
        let _ = writeln!(
            err,
            "{}{}{} {}security install complete{}",
            self.green, self.ok_mark, self.reset, self.bold, self.reset
        );
        for line in lines {
            let _ = writeln!(err, "  {}{line}{}", self.dim, self.reset);
        }
        let _ = writeln!(err);
        let _ = err.flush();
    }

    /// Wipe an in-place bar so the next line does not append to it.
    pub(super) fn end_live(&self) {
        let mut draw = self.draw.lock().unwrap_or_else(|p| p.into_inner());
        if self.tty && draw.width > 0 {
            let blank = " ".repeat(draw.width);
            let mut err = stderr();
            let _ = write!(err, "\r{blank}\r");
            let _ = err.flush();
        }
        draw.width = 0;
        draw.last_at = None;
        draw.last_pct = None;
        draw.last_phase = None;
    }

    pub(super) fn next_spinner(&self) -> u8 {
        self.spinner.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn paint_pull(&self, view: &PullView) {
        if view.phase == Phase::Ready {
            return;
        }
        let line = render_pull_line(view);
        let mut draw = self.draw.lock().unwrap_or_else(|p| p.into_inner());
        if self.tty {
            let width = line.chars().count();
            let pad = draw.width.saturating_sub(width);
            draw.width = width;
            let mut err = stderr();
            let _ = write!(err, "\r{line}{}", " ".repeat(pad));
            let _ = err.flush();
            return;
        }
        let first = draw.last_at.is_none();
        let due = draw
            .last_at
            .map(|at| at.elapsed() >= Duration::from_secs(5))
            .unwrap_or(true);
        let phase_changed = draw.last_phase != Some(view.phase);
        let pct = view.percent_tenths();
        let pct_changed = match (draw.last_pct, pct) {
            (Some(prev), Some(next)) => next / 50 != prev / 50,
            (None, Some(_)) => true,
            _ => false,
        };
        if !first && !due && !phase_changed && !pct_changed {
            return;
        }
        draw.last_at = Some(Instant::now());
        draw.last_pct = pct;
        draw.last_phase = Some(view.phase);
        drop(draw);
        eprintln!("{line}");
    }
}

/// Whole units, matching `_koma_bytes` in `install.sh`.
pub(super) fn format_bytes_whole(n: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if n >= GB {
        format!("{} GB", n / GB)
    } else if n >= MB {
        format!("{} MB", n / MB)
    } else if n >= KB {
        format!("{} KB", n / KB)
    } else {
        format!("{n} B")
    }
}

fn format_bytes_live(n: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    const KB: u64 = 1024;
    if n >= GB {
        format_unit(n, GB, "GB")
    } else if n >= MB {
        format_unit(n, MB, "MB")
    } else if n >= KB {
        format_unit(n, KB, "KB")
    } else {
        format!("{n} B")
    }
}

fn format_unit(n: u64, unit: u64, name: &str) -> String {
    let whole = n / unit;
    let frac = ((n % unit) * 10) / unit;
    if frac == 0 {
        format!("{whole} {name}")
    } else {
        format!("{whole}.{frac} {name}")
    }
}

pub(super) fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

fn format_rate(bps: u64) -> String {
    format!("{}/s", format_bytes_live(bps))
}

fn percent_text(current: u64, total: u64) -> String {
    let tenths = percent_tenths(current, total).unwrap_or(0);
    format!("{:3}.{:01}%", tenths / 10, tenths % 10)
}

fn percent_tenths(current: u64, total: u64) -> Option<u16> {
    if total == 0 {
        return None;
    }
    let cur = current.min(total) as u128;
    let tot = total as u128;
    let tenths = (cur * 1000) / tot;
    u16::try_from(tenths.min(1000)).ok()
}

fn bar(current: u64, total: u64) -> String {
    let filled = if total == 0 {
        0
    } else {
        let cur = current.min(total) as u128;
        let tot = total as u128;
        let width = u128::from(u32::try_from(BAR_WIDTH).unwrap_or(32));
        usize::try_from((cur * width) / tot).unwrap_or(0)
    };
    let filled = filled.min(BAR_WIDTH);
    let mut out = String::with_capacity(BAR_WIDTH);
    for i in 0..BAR_WIDTH {
        if i < filled {
            out.push('#');
        } else {
            out.push(' ');
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Contacting,
    Waiting,
    Downloading,
    Preparing,
    Verifying,
    Extracting,
    Ready,
}

pub(super) struct PullView {
    phase: Phase,
    spinner: u8,
    current: u64,
    total: u64,
    bps: u64,
    elapsed: Duration,
    layers_done: u32,
    layers: u32,
}

impl PullView {
    /// Stable wire name for the panel snapshot.
    pub(super) fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Contacting => "contacting",
            Phase::Waiting => "waiting",
            Phase::Downloading => "downloading",
            Phase::Preparing => "preparing",
            Phase::Verifying => "verifying",
            Phase::Extracting => "extracting",
            Phase::Ready => "finishing",
        }
    }

    /// `(current, total, bytes/s, layers done, layers)`.
    pub(super) fn counters(&self) -> (u64, u64, u64, u32, u32) {
        (self.current, self.total, self.bps, self.layers_done, self.layers)
    }

    fn percent_tenths(&self) -> Option<u16> {
        percent_tenths(self.current, self.total)
    }
}

pub(super) fn render_pull_line(view: &PullView) -> String {
    let spin = SPINNER[(view.spinner as usize) % SPINNER.len()];
    let clock = format_elapsed(view.elapsed);
    match view.phase {
        Phase::Contacting => format!("  {spin} contacting the registry  {clock}"),
        Phase::Waiting if view.layers > 0 => format!(
            "  {spin} waiting for layers  {}/{}  {clock}",
            view.layers_done, view.layers
        ),
        Phase::Waiting => format!("  {spin} waiting for layers  {clock}"),
        Phase::Downloading if view.total > 0 => {
            let rate = if view.bps > 0 {
                format_rate(view.bps)
            } else {
                "downloading".to_string()
            };
            format!(
                "  {}  {}  {} / {}  {rate}  {clock}",
                bar(view.current, view.total),
                percent_text(view.current, view.total),
                format_bytes_live(view.current),
                format_bytes_live(view.total),
            )
        }
        Phase::Downloading => format!(
            "  {spin} downloading  {}  {}  {clock}",
            format_bytes_live(view.current),
            if view.bps > 0 {
                format_rate(view.bps)
            } else {
                String::new()
            }
        ),
        Phase::Extracting if view.total > 0 => format!(
            "  {}  {}  extracting  {clock}",
            bar(view.current, view.total),
            percent_text(view.current, view.total),
        ),
        Phase::Extracting => format!(
            "  {spin} extracting  {}/{} layers  {clock}",
            view.layers_done,
            view.layers.max(1)
        ),
        Phase::Verifying => format!("  {spin} verifying layers  {clock}"),
        Phase::Preparing => format!(
            "  {spin} preparing image  {}  {clock}",
            format_bytes_live(view.current)
        ),
        Phase::Ready => format!("  {spin} finishing  {clock}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayerPhase {
    Seen,
    Waiting,
    Downloading,
    Verifying,
    Downloaded,
    Extracting,
    Done,
}

#[derive(Debug)]
struct Layer {
    phase: LayerPhase,
    dl_cur: u64,
    dl_tot: u64,
    ex_cur: u64,
    ex_tot: u64,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            phase: LayerPhase::Seen,
            dl_cur: 0,
            dl_tot: 0,
            ex_cur: 0,
            ex_tot: 0,
        }
    }
}

#[derive(Debug)]
pub(super) struct PullMeter {
    layers: BTreeMap<String, Layer>,
    saw_status: bool,
    up_to_date: bool,
    bps: u64,
    rate_at: Option<Instant>,
    rate_bytes: u64,
}

impl PullMeter {
    pub(super) fn new() -> Self {
        Self {
            layers: BTreeMap::new(),
            saw_status: false,
            up_to_date: false,
            bps: 0,
            rate_at: None,
            rate_bytes: 0,
        }
    }

    pub(super) fn saw_status(&self) -> bool {
        self.saw_status
    }

    pub(super) fn up_to_date(&self) -> bool {
        self.up_to_date
    }

    pub(super) fn transferred(&self) -> u64 {
        self.layers.values().map(|layer| layer.dl_cur).sum()
    }

    pub(super) fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// Apply one docker pull JSON line. `Err` is a registry error message.
    pub(super) fn ingest(&mut self, line: &str) -> Result<(), String> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|_| format!("unreadable pull event: {line}"))?;
        if let Some(err) = value.get("error").and_then(|item| item.as_str()) {
            if !err.is_empty() {
                return Err(err.to_string());
            }
        }
        let status = value
            .get("status")
            .and_then(|item| item.as_str())
            .unwrap_or("");
        let id = value.get("id").and_then(|item| item.as_str()).unwrap_or("");
        let detail = value.get("progressDetail");
        let current = detail
            .and_then(|item| item.get("current"))
            .and_then(serde_json::Value::as_u64);
        let total = detail
            .and_then(|item| item.get("total"))
            .and_then(serde_json::Value::as_u64);
        self.on_event(status, id, current, total);
        Ok(())
    }

    fn on_event(&mut self, status: &str, id: &str, current: Option<u64>, total: Option<u64>) {
        if status.starts_with("Status:") {
            self.saw_status = true;
            self.up_to_date = status.to_ascii_lowercase().contains("up to date");
            return;
        }
        if status.starts_with("Digest:") || status.starts_with("Pulling from ") {
            return;
        }
        if id.is_empty() || !is_layer_status(status) {
            return;
        }
        let layer = self.layers.entry(id.to_string()).or_default();
        match status {
            "Pulling fs layer" => layer.phase = LayerPhase::Seen,
            "Waiting" => layer.phase = LayerPhase::Waiting,
            "Downloading" => {
                layer.phase = LayerPhase::Downloading;
                if let Some(current) = current {
                    layer.dl_cur = current;
                }
                if let Some(total) = total {
                    layer.dl_tot = layer.dl_tot.max(total);
                }
            }
            "Verifying Checksum" => layer.phase = LayerPhase::Verifying,
            "Download complete" => {
                layer.phase = LayerPhase::Downloaded;
                if layer.dl_tot > 0 {
                    layer.dl_cur = layer.dl_tot;
                }
            }
            "Extracting" => {
                layer.phase = LayerPhase::Extracting;
                if let Some(current) = current {
                    layer.ex_cur = current;
                }
                if let Some(total) = total {
                    layer.ex_tot = layer.ex_tot.max(total);
                }
            }
            "Pull complete" => {
                layer.phase = LayerPhase::Done;
                if layer.dl_tot > 0 {
                    layer.dl_cur = layer.dl_tot;
                }
                if layer.ex_tot > 0 {
                    layer.ex_cur = layer.ex_tot;
                }
            }
            _ => {}
        }
        self.note_rate();
    }

    fn note_rate(&mut self) {
        let bytes = self.transferred();
        let now = Instant::now();
        let Some(at) = self.rate_at else {
            self.rate_at = Some(now);
            self.rate_bytes = bytes;
            return;
        };
        let dt_ms =
            u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX);
        if dt_ms < 300 {
            return;
        }
        if bytes > self.rate_bytes {
            let delta = bytes - self.rate_bytes;
            self.bps = delta.saturating_mul(1000) / dt_ms.max(1);
        } else if dt_ms >= 2_000 {
            self.bps = 0;
        }
        self.rate_at = Some(now);
        self.rate_bytes = bytes;
    }

    pub(super) fn view(&self, spinner: u8, elapsed: Duration) -> PullView {
        let layers = u32::try_from(self.layers.len()).unwrap_or(u32::MAX);
        let layers_done = u32::try_from(
            self.layers
                .values()
                .filter(|layer| {
                    matches!(
                        layer.phase,
                        LayerPhase::Downloaded | LayerPhase::Extracting | LayerPhase::Done
                    )
                })
                .count(),
        )
        .unwrap_or(u32::MAX);
        let (dl_cur, dl_tot) = sum_pairs(
            self.layers
                .values()
                .map(|layer| (layer.dl_cur, layer.dl_tot)),
        );
        let (ex_cur, ex_tot) = sum_pairs(
            self.layers
                .values()
                .map(|layer| (layer.ex_cur, layer.ex_tot)),
        );
        let any = |phase: LayerPhase| self.layers.values().any(|layer| layer.phase == phase);
        let downloading = any(LayerPhase::Downloading) || (dl_tot > 0 && dl_cur < dl_tot);
        let phase = if self.saw_status {
            Phase::Ready
        } else if downloading {
            // Layers extract while later layers are still downloading. Keep the
            // byte bar on the download until those bytes are in.
            Phase::Downloading
        } else if any(LayerPhase::Extracting) {
            Phase::Extracting
        } else if any(LayerPhase::Verifying) {
            Phase::Verifying
        } else if any(LayerPhase::Downloaded) || any(LayerPhase::Done) {
            Phase::Preparing
        } else if !self.layers.is_empty() {
            Phase::Waiting
        } else {
            Phase::Contacting
        };
        let (current, total) = match phase {
            Phase::Extracting => (ex_cur, ex_tot),
            Phase::Downloading => (dl_cur, dl_tot),
            Phase::Preparing => (dl_cur, 0),
            _ => (dl_cur, 0),
        };
        PullView {
            phase,
            spinner,
            current,
            total,
            bps: self.bps,
            elapsed,
            layers_done,
            layers,
        }
    }
}

fn sum_pairs(pairs: impl Iterator<Item = (u64, u64)>) -> (u64, u64) {
    let mut current = 0u64;
    let mut total = 0u64;
    for (cur, tot) in pairs {
        if tot == 0 {
            continue;
        }
        current = current.saturating_add(cur.min(tot));
        total = total.saturating_add(tot);
    }
    (current, total)
}

fn is_layer_status(status: &str) -> bool {
    matches!(
        status,
        "Pulling fs layer"
            | "Waiting"
            | "Downloading"
            | "Verifying Checksum"
            | "Download complete"
            | "Extracting"
            | "Pull complete"
    )
}

/// Result the installer prints after the guest-image step.
pub(super) struct PullReport {
    pub skipped: bool,
    pub up_to_date: bool,
    pub bytes: u64,
    pub elapsed: Duration,
}

impl PullReport {
    pub(super) fn summary(&self) -> String {
        if self.skipped {
            return match self.bytes {
                0 => "already local".to_string(),
                n => format!("already local, {}", format_bytes_whole(n)),
            };
        }
        if self.up_to_date {
            return match self.bytes {
                0 => "up to date".to_string(),
                n => format!("up to date, {}", format_bytes_whole(n)),
            };
        }
        let secs = self.elapsed.as_secs().max(1);
        if self.bytes == 0 {
            format!("pulled in {secs}s")
        } else {
            format!("{} in {secs}s", format_bytes_whole(self.bytes))
        }
    }
}

#[cfg(test)]
#[path = "progress_test.rs"]
mod progress_test;

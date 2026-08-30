//! Remote koma bootstrap: validate the remote version and install or upgrade it.

use std::fmt;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::auth::SshAuth;
use super::ssh;
use super::RemoteTarget;

const MISSING: &str = "MISSING";

/// Stages reported while bootstrapping the remote binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootstrapStage {
    Checking,
    Installing,
    Verifying,
    Ready,
}

impl BootstrapStage {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Checking => "checking remote koma",
            Self::Installing => "installing matching version",
            Self::Verifying => "verifying remote install",
            Self::Ready => "remote ready",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SemanticVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<String>,
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(prerelease) = &self.prerelease {
            write!(f, "-{prerelease}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RemoteVersion {
    Missing,
    Version(SemanticVersion),
    Unrecognized(String),
}

impl fmt::Display for RemoteVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("missing"),
            Self::Version(version) => version.fmt(f),
            Self::Unrecognized(output) => write!(f, "unrecognized output {output:?}"),
        }
    }
}

/// Result of the remote version probe (before any install).
#[derive(Clone, Debug, Eq, PartialEq)]
enum CheckOutcome {
    Compatible,
    /// Remote binary missing / unreadable — install without asking.
    NeedsInstallMissing,
    /// Remote version differs from local — ask before overwriting.
    NeedsUpdate { observed: String },
}

fn parse_semantic_version(value: &str) -> Option<SemanticVersion> {
    let (without_build, build) = value
        .split_once('+')
        .map_or((value, None), |(version, build)| (version, Some(build)));
    if build.is_some_and(|value| !valid_identifiers(value, false)) {
        return None;
    }

    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    if prerelease.is_some_and(|value| !valid_identifiers(value, true)) {
        return None;
    }

    let mut components = core.split('.');
    let major = parse_core_number(components.next()?)?;
    let minor = parse_core_number(components.next()?)?;
    let patch = parse_core_number(components.next()?)?;
    if components.next().is_some() {
        return None;
    }

    Some(SemanticVersion {
        major,
        minor,
        patch,
        prerelease: prerelease.map(str::to_string),
    })
}

fn parse_core_number(value: &str) -> Option<u64> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    value.parse().ok()
}

fn valid_identifiers(value: &str, reject_numeric_leading_zero: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|identifier| {
            !identifier.is_empty()
                && identifier
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !(reject_numeric_leading_zero
                    && identifier.len() > 1
                    && identifier.starts_with('0')
                    && identifier.bytes().all(|byte| byte.is_ascii_digit()))
        })
}

/// Parse remote `koma --version` stdout. Login shells (`bash -ilc`) often print
/// MOTD / distro noise (e.g. Raspberry Pi rfkill) on stdout before the real line
/// `koma <semver>`. Scan every line for that pattern; ignore surrounding junk.
fn parse_version_output(output: &str) -> RemoteVersion {
    let output = output.trim();
    if output.is_empty() {
        return RemoteVersion::Unrecognized(String::new());
    }
    if output == MISSING {
        return RemoteVersion::Missing;
    }

    let mut found: Option<SemanticVersion> = None;
    let mut saw_missing = false;
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == MISSING {
            saw_missing = true;
            continue;
        }
        // Accept `koma <semver>` optionally followed by trailing tokens (never
        // treat the second word as a version unless it parses as semver —
        // so `koma version 0.3.16` stays unrecognized).
        let mut words = line.split_whitespace();
        let Some("koma") = words.next() else {
            continue;
        };
        let Some(version) = words.next() else {
            continue;
        };
        if let Some(v) = parse_semantic_version(version) {
            found = Some(v);
        }
    }

    if let Some(v) = found {
        return RemoteVersion::Version(v);
    }
    if saw_missing {
        return RemoteVersion::Missing;
    }
    RemoteVersion::Unrecognized(output.to_string())
}

fn check_remote_version<Q>(local: &str, mut query: Q) -> Result<CheckOutcome>
where
    Q: FnMut() -> Result<String>,
{
    let expected = parse_semantic_version(local).ok_or_else(|| {
        anyhow::anyhow!("local Koma version is not valid semantic version: {local:?}")
    })?;

    // Treat a probe failure (SSH error, broken binary, QEMU/binfmt, etc.) as
    // "missing" so we fall through to the install path instead of aborting.
    let observed = match query() {
        Ok(output) => parse_version_output(&output),
        Err(_) => RemoteVersion::Missing,
    };

    match observed {
        RemoteVersion::Version(v) if v == expected => Ok(CheckOutcome::Compatible),
        RemoteVersion::Missing => Ok(CheckOutcome::NeedsInstallMissing),
        other => Ok(CheckOutcome::NeedsUpdate {
            observed: other.to_string(),
        }),
    }
}

fn install_and_verify<Q, I, P>(local: &str, mut query: Q, mut install: I, mut progress: P) -> Result<()>
where
    Q: FnMut() -> Result<String>,
    I: FnMut() -> Result<()>,
    P: FnMut(BootstrapStage),
{
    let expected = parse_semantic_version(local).ok_or_else(|| {
        anyhow::anyhow!("local Koma version is not valid semantic version: {local:?}")
    })?;

    progress(BootstrapStage::Installing);
    install()?;

    progress(BootstrapStage::Verifying);
    let observed = match query() {
        Ok(output) => parse_version_output(&output),
        Err(_) => RemoteVersion::Missing,
    };
    if observed != RemoteVersion::Version(expected.clone()) {
        anyhow::bail!(
            "remote Koma version mismatch after install: expected {expected}, observed {observed}"
        );
    }
    progress(BootstrapStage::Ready);
    Ok(())
}

/// Core bootstrap: check → optional confirm on mismatch → install → verify.
///
/// `confirm_update(observed)` is only called when a remote binary exists but
/// does not match `local`. Return `Ok(true)` to force-install, `Ok(false)` to
/// abort cleanly, or `Err` for a hard failure.
fn ensure_compatible_with<Q, I, P, C>(
    local: &str,
    mut query: Q,
    install: I,
    mut progress: P,
    mut confirm_update: C,
) -> Result<bool>
where
    Q: FnMut() -> Result<String>,
    I: FnMut() -> Result<()>,
    P: FnMut(BootstrapStage),
    C: FnMut(&str) -> Result<bool>,
{
    progress(BootstrapStage::Checking);
    match check_remote_version(local, &mut query)? {
        CheckOutcome::Compatible => {
            progress(BootstrapStage::Ready);
            Ok(false)
        }
        CheckOutcome::NeedsInstallMissing => {
            install_and_verify(local, query, install, progress)?;
            Ok(true)
        }
        CheckOutcome::NeedsUpdate { observed } => {
            if !confirm_update(&observed)? {
                anyhow::bail!(
                    "remote update declined (local {local}, remote {observed})"
                );
            }
            install_and_verify(local, query, install, progress)?;
            Ok(true)
        }
    }
}

/// Ensure the remote Koma is installed and exactly matches this running client.
///
/// Headless / GUI path: version mismatch auto-accepts the force update (no TUI).
/// Returns `true` when the installer was run and `false` when already compatible.
pub(crate) fn ensure_koma_compatible(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
) -> Result<bool> {
    ensure_compatible_with(
        env!("CARGO_PKG_VERSION"),
        || query_remote_version(target, auth),
        || install_koma(target, auth),
        |_| {},
        |_| Ok(true),
    )
}

/// Same as [`ensure_koma_compatible`], but spins a braille timeline on `terminal`
/// and prompts **update remote? [y/n]** when the remote version differs.
pub(crate) fn ensure_koma_compatible_animated(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
    host_label: &str,
) -> Result<bool> {
    let local = env!("CARGO_PKG_VERSION");
    let cfg = crate::model::app_config::AppConfig::load();
    let palette = crate::view::theme::palette(&cfg);

    // --- Phase 1: version check (worker + spinner) ---
    let check = run_stage_worker(terminal, host_label, &palette, BootstrapStage::Checking, {
        let target = target.clone();
        let password = auth.map(|a| a.password().to_string());
        move || {
            let auth = password.map(SshAuth::new).transpose()?;
            check_remote_version(local, || query_remote_version(&target, auth.as_ref()))
        }
    })?;

    match check {
        CheckOutcome::Compatible => {
            paint_stage(
                terminal,
                host_label,
                &palette,
                BootstrapStage::Ready,
                Instant::now(),
                0,
            );
            Ok(false)
        }
        CheckOutcome::NeedsInstallMissing => {
            run_install_phase(terminal, target, auth, host_label, &palette, local)?;
            Ok(true)
        }
        CheckOutcome::NeedsUpdate { observed } => {
            let accepted = prompt_update_remote(terminal, host_label, local, &observed)?;
            if !accepted {
                anyhow::bail!("remote update declined (local {local}, remote {observed})");
            }
            run_install_phase(terminal, target, auth, host_label, &palette, local)?;
            Ok(true)
        }
    }
}

fn run_install_phase(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
    host_label: &str,
    palette: &crate::view::theme::Palette,
    local: &str,
) -> Result<()> {
    let (tx, rx) = mpsc::channel::<BootstrapStage>();
    let target = target.clone();
    let password = auth.map(|a| a.password().to_string());
    let local = local.to_string();
    let worker = std::thread::spawn(move || {
        let auth = password.map(SshAuth::new).transpose()?;
        install_and_verify(
            &local,
            || query_remote_version(&target, auth.as_ref()),
            || install_koma(&target, auth.as_ref()),
            |stage| {
                let _ = tx.send(stage);
            },
        )
    });

    spin_until_done(terminal, host_label, palette, BootstrapStage::Installing, &rx, &worker)?;
    match worker.join() {
        Ok(res) => res,
        Err(_) => Err(anyhow::anyhow!("remote bootstrap thread panicked")),
    }
}

fn run_stage_worker<T, F>(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    host_label: &str,
    palette: &crate::view::theme::Palette,
    initial: BootstrapStage,
    work: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<BootstrapStage>();
    let worker = std::thread::spawn(move || {
        let _ = tx.send(initial);
        work()
    });
    spin_until_done(terminal, host_label, palette, initial, &rx, &worker)?;
    match worker.join() {
        Ok(res) => res,
        Err(_) => Err(anyhow::anyhow!("remote bootstrap thread panicked")),
    }
}

fn spin_until_done<T>(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    host_label: &str,
    palette: &crate::view::theme::Palette,
    mut stage: BootstrapStage,
    rx: &mpsc::Receiver<BootstrapStage>,
    worker: &std::thread::JoinHandle<T>,
) -> Result<()> {
    let started = Instant::now();
    let mut frame: u64 = 0;
    const FRAME: Duration = Duration::from_millis(80);

    while !worker.is_finished() {
        let tick = Instant::now();
        while let Ok(next) = rx.try_recv() {
            stage = next;
        }
        paint_stage(terminal, host_label, palette, stage, started, frame);
        frame = frame.wrapping_add(1);
        if let Some(rem) = FRAME.checked_sub(tick.elapsed()) {
            std::thread::sleep(rem);
        }
    }
    while let Ok(next) = rx.try_recv() {
        stage = next;
    }
    paint_stage(terminal, host_label, palette, stage, started, frame);
    Ok(())
}

fn paint_stage(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    host_label: &str,
    palette: &crate::view::theme::Palette,
    stage: BootstrapStage,
    started: Instant,
    frame: u64,
) {
    let _ = terminal.draw(|f| {
        crate::view::loading::draw_remote_bootstrap(
            f,
            frame,
            palette,
            host_label,
            stage.label(),
            started.elapsed(),
        )
    });
}

/// In-TUI yes/no: force-update the remote binary to match this client.
fn prompt_update_remote(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    host_label: &str,
    local: &str,
    observed: &str,
) -> Result<bool> {
    use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use ratatui::layout::{Alignment, Constraint, Direction, Layout};
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::Paragraph;

    let palette = crate::view::theme::palette(&crate::model::app_config::AppConfig::load());
    let detail = format!("local {local}  ·  remote {observed}");

    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            crate::view::clear_and_fill(frame, area, palette.bg);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(30),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(area);

            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "koma remote",
                    Style::default()
                        .fg(palette.accent)
                        .bg(palette.bg)
                        .add_modifier(Modifier::BOLD),
                )))
                .alignment(Alignment::Center),
                chunks[1],
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    host_label,
                    Style::default().fg(palette.dim).bg(palette.bg),
                )))
                .alignment(Alignment::Center),
                chunks[2],
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "remote version does not match this client",
                    Style::default().fg(palette.fg).bg(palette.bg),
                )))
                .alignment(Alignment::Center),
                chunks[3],
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    detail.clone(),
                    Style::default().fg(palette.dim).bg(palette.bg),
                )))
                .alignment(Alignment::Center),
                chunks[4],
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "update remote?  [y] yes   [n] no",
                    Style::default().fg(palette.accent).bg(palette.bg),
                )))
                .alignment(Alignment::Center),
                chunks[6],
            );
        })?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => return Ok(true),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => return Ok(false),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(false);
            }
            _ => {}
        }
    }
}

fn query_remote_version(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<String> {
    let path = ssh::find_koma(target, auth)?;
    let command = ssh::remote_command(&path, &["--version"])?;
    ssh::exec_remote(
        target,
        &format!("{command} 2>/dev/null || echo {MISSING}"),
        auth,
    )
}

/// Install or upgrade koma on the remote machine using the official installer,
/// pinned to this client's exact version (not `latest`).
fn install_koma(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    // Pin KOMA_RELEASE_BASE to the matching tagged release so a newer GitHub
    // "latest" never leaves remote ahead of this client (exact-match policy).
    let cmd = format!(
        "KOMA_RELEASE_BASE=https://github.com/aula-id/koma/releases/download/v{version} \
         curl -fsSL https://koma.run/install.sh | sh"
    );
    ssh::exec_remote(target, &cmd, auth)?;
    Ok(())
}

#[cfg(test)]
#[path = "bootstrap_test.rs"]
mod tests;

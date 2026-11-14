//! Remote koma bootstrap: validate the remote version and install or upgrade it.
//!
//! Bootstrap installs its own copy at [`ssh::MANAGED_KOMA`] straight from the
//! release matching this client. On Linux it takes the headless build, which
//! needs no GTK or WebKit, and falls back to the GUI build for releases that
//! publish no headless one. It never runs the desktop installer, so it never
//! needs sudo or a package manager on the remote.

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
    /// A Koma binary is there but does not run; holds why.
    Broken(String),
}

impl fmt::Display for RemoteVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("missing"),
            Self::Version(version) => version.fmt(f),
            Self::Unrecognized(output) => write!(f, "unrecognized output {output:?}"),
            Self::Broken(reason) => write!(f, "a Koma that does not run ({reason})"),
        }
    }
}

/// Result of the remote version probe (before any install).
#[derive(Clone, Debug, Eq, PartialEq)]
enum CheckOutcome {
    Compatible,
    /// Remote binary missing or not runnable — install without asking.
    NeedsInstallMissing,
    /// Remote version differs from local — ask before overwriting.
    NeedsUpdate {
        observed: String,
    },
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

    match probe(&mut query)? {
        RemoteVersion::Version(v) if v == expected => Ok(CheckOutcome::Compatible),
        RemoteVersion::Missing | RemoteVersion::Broken(_) => Ok(CheckOutcome::NeedsInstallMissing),
        other => Ok(CheckOutcome::NeedsUpdate {
            observed: other.to_string(),
        }),
    }
}

/// Run the version query. An SSH failure (unreachable host, rejected
/// credentials) is returned as is; any other failure means the remote binary
/// does not run, which a fresh install can fix.
fn probe<Q>(query: &mut Q) -> Result<RemoteVersion>
where
    Q: FnMut() -> Result<String>,
{
    match query() {
        Ok(output) => Ok(parse_version_output(&output)),
        Err(error) if ssh::is_transport_error(&error) => Err(error),
        Err(error) => Ok(RemoteVersion::Broken(format!("{error:#}"))),
    }
}

fn install_and_verify<Q, I, P>(
    local: &str,
    mut query: Q,
    mut install: I,
    mut progress: P,
) -> Result<()>
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
    let observed = probe(&mut query)?;
    if observed != RemoteVersion::Version(expected.clone()) {
        let hint = match &observed {
            RemoteVersion::Broken(reason) if missing_shared_libraries(reason) => {
                "\nThis release has no headless Linux build for the remote, and its desktop \
                 build needs the GUI libraries (WebKitGTK 4.1, GTK 3). Install them on the \
                 remote, or use a Koma release that publishes koma-linux-*-headless."
            }
            _ => "",
        };
        anyhow::bail!(
            "remote Koma version mismatch after install: expected {expected}, observed {observed}{hint}"
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
                anyhow::bail!("remote update declined (local {local}, remote {observed})");
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

/// Headless progress variant used by the terminal transport helper.
pub(crate) fn ensure_koma_compatible_progress(
    target: &RemoteTarget,
    auth: Option<&SshAuth>,
    progress: impl FnMut(BootstrapStage),
) -> Result<bool> {
    ensure_compatible_with(
        env!("CARGO_PKG_VERSION"),
        || query_remote_version(target, auth),
        || install_koma(target, auth),
        progress,
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

    spin_until_done(
        terminal,
        host_label,
        palette,
        BootstrapStage::Installing,
        &rx,
        &worker,
    )?;
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

/// The dynamic loader refused to start the binary. With the headless build
/// that cannot happen, so it means the desktop build was the only one there.
fn missing_shared_libraries(reason: &str) -> bool {
    reason.contains("error while loading shared libraries")
}

/// `koma --version` on the remote. `MISSING` when no Koma is installed; `Err`
/// with the binary's own error output when it is installed but does not run.
fn query_remote_version(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<String> {
    let Some(path) = ssh::locate_koma(target, auth)? else {
        return Ok(MISSING.to_string());
    };
    let command = ssh::remote_command(&path, &["--version"])?;
    let output = ssh::exec_remote_output(target, &command, auth)?;
    if output.success() {
        return Ok(output.stdout);
    }
    let code = output
        .code
        .map_or_else(|| "a signal".to_string(), |code| code.to_string());
    anyhow::bail!("{path} exited with {code}: {}", error_summary(&output.stderr))
}

/// The last few meaningful stderr lines, without bash's job-control warnings
/// (`bash -i` with no terminal prints them on every run).
fn error_summary(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.contains("cannot set terminal process group")
                && !line.contains("no job control in this shell")
        })
        .collect();
    if lines.is_empty() {
        return "no error output".to_string();
    }
    lines[lines.len().saturating_sub(6)..].join(" / ")
}

/// Download this client's exact release into [`ssh::MANAGED_KOMA`]. Prefers
/// the headless Linux build; checks the SHA-256 when the release lists one.
/// Prints the installed asset name. Arguments: `$1` version.
const INSTALL_SCRIPT: &str = r#"set -eu
version=$1
base="https://github.com/aula-id/koma/releases/download/v$version"
case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=darwin ;;
    *) echo "Koma has no build for $(uname -s)" >&2; exit 2 ;;
esac
case "$(uname -m)" in
    x86_64|amd64) arch=x64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) echo "Koma has no build for $(uname -m)" >&2; exit 2 ;;
esac
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL --retry 3 --connect-timeout 15 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q -T 15 -t 3 -O "$2" "$1"; }
else
    echo "the remote host has neither curl nor wget" >&2; exit 2
fi
dir="$HOME/.koma/remote"
mkdir -p "$dir"
tmp="$dir/.download.$$"
sums="$dir/.sums.$$"
trap 'rm -f "$tmp" "$sums"' EXIT
if [ "$os" = linux ]; then
    candidates="koma-linux-$arch-headless koma-linux-$arch"
else
    candidates="koma-darwin-$arch"
fi
asset=""
for name in $candidates; do
    if fetch "$base/$name" "$tmp" 2>/dev/null; then asset=$name; break; fi
done
if [ -z "$asset" ]; then
    echo "could not download Koma $version for $os/$arch from $base (no network on the remote, or that release does not exist)" >&2
    exit 3
fi
if fetch "$base/SHA256SUMS" "$sums" 2>/dev/null; then
    want=$(awk -v n="$asset" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print $1; exit } }' "$sums")
    if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then got=$(shasum -a 256 "$tmp" | awk '{print $1}')
    elif command -v openssl >/dev/null 2>&1; then got=$(openssl dgst -sha256 "$tmp" | awk '{print $NF}')
    else echo "the remote host has no sha256sum, shasum or openssl to check the download" >&2; exit 4
    fi
    want=$(printf '%s' "$want" | tr 'A-F' 'a-f')
    got=$(printf '%s' "$got" | tr 'A-F' 'a-f')
    if [ -z "$want" ] || [ "$got" != "$want" ]; then
        echo "checksum check failed for $asset (expected ${want:-no entry}, got $got)" >&2
        exit 4
    fi
fi
chmod 755 "$tmp"
mv -f "$tmp" "$dir/koma"
printf '%s
' "$asset"
"#;

/// One install at a time per host. The GUI panes, terminal helper and coding
/// worker can all bootstrap the same host at once.
fn host_install_lock(target: &RemoteTarget) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let key = format!("{}@{}:{}", target.user, target.host, target.port.unwrap_or(22));
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.entry(key).or_default().clone()
}

/// Install this client's exact version on the remote host.
fn install_koma(target: &RemoteTarget, auth: Option<&SshAuth>) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let lock = host_install_lock(target);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Another connection may have finished the same install while we waited.
    if let (Ok(output), Some(expected)) = (
        query_remote_version(target, auth),
        parse_semantic_version(version),
    ) {
        if parse_version_output(&output) == RemoteVersion::Version(expected) {
            return Ok(());
        }
    }
    // `sh -c` so the script runs the same under any login shell (fish, csh).
    let command = format!(
        "sh -c {} koma-install {}",
        ssh::shell_quote(INSTALL_SCRIPT),
        ssh::shell_quote(version)
    );
    let output = ssh::exec_remote_output(target, &command, auth)?;
    if !output.success() {
        anyhow::bail!(
            "installing Koma {version} on the remote failed: {}",
            error_summary(&output.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "bootstrap_test.rs"]
mod tests;

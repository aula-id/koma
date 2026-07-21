//! `koma doctor` — a flutter-doctor-style readiness report.
//!
//! Runs a fixed set of read-only checks against the local koma install (config,
//! daemons, models, GUI deps, extensions, optional subsystems, MCP servers, and
//! the latest-release check) and prints one line per category with a
//! `[✓]`/`[!]`/`[✗]` marker, then a one-line summary.
//!
//! STRICTLY READ-ONLY: this module must never spawn, restart, install, or kill
//! anything, and must never write under `~/.koma`. Every socket/network
//! operation is bounded so a wedged daemon or a dead network can never hang the
//! command. Lives inside `manage` (rather than a sibling module) so it can reuse
//! the sync blocking wire helpers (`connect_managed`/`send_request`/`recv_frame`)
//! and the socket-discovery helpers (`list_session_sockets`) that are
//! `pub(super)` to this tree.

use std::io::IsTerminal;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::ipc::proto::{ClientRequest, DaemonEvent};
use crate::model::app_config::{AppConfig, ApiType, ModelRole};
use crate::model::store;

/// How long the `koma doctor` update check waits for `koma.run` before giving
/// up and reporting "skipped — offline?" instead. Bounded so a dead network
/// never hangs the command.
const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Severity of one category's outcome — drives both the marker glyph/colour and
/// the overall exit code (only [`Status::Fail`] flips it to `1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

/// One category's outcome: the headline shown next to the marker, plus
/// sub-detail lines shown when the category is not [`Status::Ok`], or always
/// under `-v`/`--verbose`.
struct CheckResult {
    status: Status,
    headline: String,
    details: Vec<String>,
}

impl CheckResult {
    fn ok(headline: impl Into<String>) -> Self {
        Self { status: Status::Ok, headline: headline.into(), details: Vec::new() }
    }

    fn warn(headline: impl Into<String>, details: Vec<String>) -> Self {
        Self { status: Status::Warn, headline: headline.into(), details }
    }

    fn fail(headline: impl Into<String>, details: Vec<String>) -> Self {
        Self { status: Status::Fail, headline: headline.into(), details }
    }
}

/// Entry point: run every check in order, print the flutter-doctor-style
/// report, and return the process exit code (`0` unless at least one category
/// is [`Status::Fail`], then `1`).
pub fn run_doctor(verbose: bool) -> i32 {
    let use_color = should_use_color();

    let config_ok = check_config_parses();

    let results = vec![
        check_koma(config_ok),
        check_daemons(),
        check_models(),
        check_gui(),
        check_extensions(),
        check_internet_fullmode(),
        check_security_daemon(),
        check_mcp_servers(config_ok),
        check_update(),
    ];

    let mut warn_or_fail_categories = 0usize;
    let mut any_fail = false;

    for r in &results {
        print_result(r, verbose, use_color);
        if r.status != Status::Ok {
            warn_or_fail_categories += 1;
        }
        if r.status == Status::Fail {
            any_fail = true;
        }
    }

    println!();
    if warn_or_fail_categories == 0 {
        println!("{}", paint("• No issues found.", Status::Ok, use_color));
    } else {
        println!(
            "{}",
            paint(
                &format!("! Doctor found issues in {warn_or_fail_categories} categor{}.",
                    if warn_or_fail_categories == 1 { "y" } else { "ies" }),
                Status::Warn,
                use_color
            )
        );
    }

    if any_fail { 1 } else { 0 }
}

/// Whether ANSI colour should be emitted: stdout must be a real terminal AND
/// `NO_COLOR` must be unset (https://no-color.org convention).
fn should_use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Wrap `text` in the ANSI colour matching `status`, or return it unchanged
/// when `use_color` is `false`. Raw escapes — there is no colour crate in this
/// repo and none is added here.
fn paint(text: &str, status: Status, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    let code = match status {
        Status::Ok => "32",   // green
        Status::Warn => "33", // yellow
        Status::Fail => "31", // red
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

/// Print one category's marker + headline, then its detail lines when the
/// category is not [`Status::Ok`], or unconditionally under `-v`.
fn print_result(r: &CheckResult, verbose: bool, use_color: bool) {
    let marker = match r.status {
        Status::Ok => "✓",
        Status::Warn => "!",
        Status::Fail => "✗",
    };
    println!("[{}] {}", paint(marker, r.status, use_color), r.headline);
    if (r.status != Status::Ok || verbose) && !r.details.is_empty() {
        for d in &r.details {
            println!("  • {d}");
        }
    }
}

// ─── 1. koma ──────────────────────────────────────────────────────────────

/// Doctor's OWN config-parse check: `AppConfig::load()` silently defaults on a
/// parse failure, so this is the only way a user learns their `config.json` is
/// corrupt. Returns `true` when the file is absent (fine, fresh install) OR
/// parses cleanly; `false` only on an actual parse failure — shared with
/// [`check_mcp_servers`] so a corrupt config is reflected there too.
fn check_config_parses() -> bool {
    let Ok(dir) = store::base_dir() else { return true };
    let path = dir.join("config.json");
    let Ok(bytes) = std::fs::read(&path) else { return true }; // missing = fine
    serde_json::from_slice::<AppConfig>(&bytes).is_ok()
}

fn check_koma(config_ok: bool) -> CheckResult {
    let version = store::current_version();
    let mut details = Vec::new();
    let mut status = Status::Ok;

    // ~/.koma existence + writability.
    let dir_state = match store::base_dir() {
        Err(e) => {
            status = Status::Fail;
            details.push(format!("cannot resolve home directory: {e}"));
            "~/.koma unresolvable".to_string()
        }
        Ok(dir) => {
            if !dir.exists() {
                details.push("~/.koma does not exist yet (fresh install)".to_string());
                "~/.koma not yet created".to_string()
            } else {
                let probe = dir.join(format!(".doctor-write-test-{}", std::process::id()));
                match std::fs::write(&probe, b"") {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&probe);
                        "~/.koma healthy".to_string()
                    }
                    Err(e) => {
                        status = Status::Fail;
                        details.push(format!("~/.koma is not writable: {e}"));
                        "~/.koma not writable".to_string()
                    }
                }
            }
        }
    };

    // Doctor's own config.json parse check (AppConfig::load() would silently
    // default on a corrupt file).
    let config_state = if !config_ok {
        status = Status::Fail;
        let dir = store::base_dir().ok();
        let path = dir.as_ref().map(|d| d.join("config.json"));
        if let Some(path) = path {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Err(e) = serde_json::from_slice::<AppConfig>(&bytes) {
                    details.push(format!("config.json failed to parse: {e}"));
                }
            }
        }
        "config CORRUPT".to_string()
    } else {
        match store::base_dir().map(|d| d.join("config.json")) {
            Ok(path) if path.exists() => "config ok".to_string(),
            _ => {
                details.push("no config.json yet (running on defaults)".to_string());
                "no config (defaults)".to_string()
            }
        }
    };

    // -v: error.log size/mtime, if present.
    if let Some(log_path) = store::global_error_log_path() {
        if let Ok(meta) = std::fs::metadata(&log_path) {
            let mtime_note = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| format!("{}s since epoch", d.as_secs()))
                .unwrap_or_else(|| "unknown mtime".to_string());
            details.push(format!(
                "~/.koma/error.log: {} bytes, modified {mtime_note}",
                meta.len()
            ));
        }
    }

    let headline = format!("koma ({version}, {dir_state}, {config_state})");
    CheckResult { status, headline, details }
}

// ─── 2. Daemons ─────────────────────────────────────────────────────────────

fn check_daemons() -> CheckResult {
    let mut details = Vec::new();
    let mut status = Status::Ok;

    let socks = super::list_session_sockets().unwrap_or_default();
    let live: Vec<(String, std::path::PathBuf)> = socks
        .iter()
        .filter(|(_, _, alive)| *alive)
        .map(|(id, path, _)| (id.clone(), path.clone()))
        .collect();
    let stale: Vec<String> = socks
        .iter()
        .filter(|(_, _, alive)| !*alive)
        .map(|(id, _, _)| id.clone())
        .collect();

    if !stale.is_empty() {
        status = Status::Warn;
        details.push(format!(
            "{} stale session socket(s) ({}) — run: koma daemon clean",
            stale.len(),
            stale.join(", ")
        ));
    }

    let my_fingerprint = store::build_fingerprint();
    let mut mismatched: Vec<String> = Vec::new();
    for (id, path) in &live {
        match probe_session_hello(path) {
            Ok(Some(daemon_version)) => {
                if daemon_version != my_fingerprint {
                    mismatched.push(id.clone());
                }
            }
            Ok(None) => {
                // No Hello within the window — a very old daemon or a slow one.
                // Not treated as a mismatch (we simply couldn't confirm).
            }
            Err(e) => {
                details.push(format!("session {id}: could not probe build fingerprint: {e:#}"));
            }
        }
    }
    if !mismatched.is_empty() {
        status = Status::Warn;
        for id in &mismatched {
            details.push(format!(
                "daemon {id} runs a different build — restart it (koma daemon restart)"
            ));
        }
    }

    let builds_note = if live.is_empty() {
        String::new()
    } else if mismatched.is_empty() {
        ", builds match".to_string()
    } else {
        format!(", {} build mismatch", mismatched.len())
    };

    // GLOBAL MCP daemon liveness + build-skew.
    let mcp_live = super::mcp::mcp_daemon_alive();
    let mcp_note = if mcp_live {
        if let Ok(path) = store::mcp_daemon_sock_path() {
            match super::mcp::probe_mcp_fingerprint(&path) {
                Ok(crate::ipc::mcp_proto::McpResponse::Fingerprint(fp)) if fp != my_fingerprint => {
                    status = Status::Warn;
                    details.push(
                        "MCP daemon runs a different build — restart it (koma daemon restart)"
                            .to_string(),
                    );
                    "MCP daemon running (build mismatch)".to_string()
                }
                Ok(_) => "MCP daemon running".to_string(),
                Err(e) => {
                    details.push(format!("MCP daemon: could not probe build fingerprint: {e:#}"));
                    "MCP daemon running".to_string()
                }
            }
        } else {
            "MCP daemon running".to_string()
        }
    } else {
        "MCP daemon not running".to_string()
    };

    let headline = format!(
        "Daemons ({} session daemon{} live{builds_note}; {})",
        live.len(),
        if live.len() == 1 { "" } else { "s" },
        mcp_note
    );

    CheckResult { status, headline, details }
}

/// Connect to a LIVE session daemon and run the SAME `Attach` handshake the
/// thin client does (`connect::connect_attach_and_handshake`), just synchronous
/// and minimal: send `Attach`, read frames until the first `Hello` (bounded by
/// the socket's own read timeout, set by [`super::connect_managed`]), then drop
/// the connection. Returns `Ok(Some(version))` on a `Hello`, `Ok(None)` if no
/// `Hello` arrived before the connection's frame budget/timeout, or `Err` on a
/// transport failure. Read-only: the daemon treats the dropped connection
/// exactly like any other client detaching.
fn probe_session_hello(path: &Path) -> anyhow::Result<Option<String>> {
    let (mut stream, mut reader) = super::connect_managed(path)?;
    super::send_request(&mut stream, &ClientRequest::Attach { foreground_id: None, cwd: None })?;

    // Hello is sent first by contract, but tolerate a few frames ahead of it
    // (mirrors `daemon_session_count`'s tolerance loop) before giving up.
    for _ in 0..8 {
        let frame = match super::recv_frame(&mut stream, &mut reader) {
            Ok(f) => f,
            Err(_) => return Ok(None), // closed/timed out before Hello — not a mismatch
        };
        if let DaemonEvent::Hello { version } = frame.event {
            return Ok(Some(version));
        }
    }
    Ok(None)
}

// ─── 3. Models ──────────────────────────────────────────────────────────────

fn check_models() -> CheckResult {
    let config = AppConfig::load();
    let mut details = Vec::new();

    if config.providers.is_empty() && config.oauth_conns.is_empty() {
        return CheckResult::fail(
            "Models (nothing configured)",
            vec!["no providers, OAuth connections, or koma-free entries — nothing can run".to_string()],
        );
    }

    let mut status = Status::Ok;
    let main_entry = config
        .models
        .iter()
        .find(|m| m.effective_roles().contains(&ModelRole::Main));

    let main_note = match main_entry {
        None => {
            status = Status::Warn;
            "no main model assigned".to_string()
        }
        Some(entry) => {
            let label = if !entry.name.is_empty() { entry.name.clone() } else { entry.model_id.clone() };
            if let Some(provider) = config.providers.iter().find(|p| p.uuid == entry.provider_uuid) {
                if provider.api_key.is_empty() && provider.api_type != ApiType::KomaFree {
                    status = Status::Warn;
                    details.push("provider key missing".to_string());
                    format!("main -> {label}, provider key missing")
                } else {
                    format!("main -> {label}")
                }
            } else if config.oauth_conns.iter().any(|c| c.uuid == entry.provider_uuid) {
                format!("main -> {label}")
            } else {
                status = Status::Warn;
                details.push(format!("main model's provider ({}) is missing", entry.provider_uuid));
                format!("main -> {label}, provider missing")
            }
        }
    };

    // Expired OAuth tokens, across every connection (not just Main's).
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for conn in &config.oauth_conns {
        if conn.expires_at != 0 && conn.expires_at < now {
            status = Status::Warn;
            details.push(format!("oauth token expired ({:?})", conn.provider));
        }
    }

    // -v: every role and its holder.
    for role in [
        ModelRole::Main,
        ModelRole::Awareness,
        ModelRole::Safeguard,
        ModelRole::Compactor,
        ModelRole::Planner,
    ] {
        let holder = config
            .models
            .iter()
            .find(|m| m.effective_roles().contains(&role))
            .map(|m| if !m.name.is_empty() { m.name.clone() } else { m.model_id.clone() })
            .unwrap_or_else(|| "(unassigned)".to_string());
        details.push(format!("{role:?}: {holder}"));
    }

    CheckResult { status, headline: format!("Models ({main_note})"), details }
}

// ─── 4. GUI ─────────────────────────────────────────────────────────────────

fn check_gui() -> CheckResult {
    let mut details = Vec::new();
    let mut status = Status::Ok;
    let gui_feature = cfg!(feature = "gui");

    if !gui_feature {
        status = Status::Warn;
        details.push("built without gui feature — rebuild with: cargo build --features gui".to_string());
    }

    let (linux_notes, linux_status) = check_gui_linux(&mut details);
    if linux_status == Status::Fail || (status == Status::Ok && linux_status == Status::Warn) {
        status = linux_status;
    }

    let feature_note = if gui_feature { "gui feature" } else { "built without gui feature" };
    let headline = format!("GUI ({feature_note}{linux_notes})");
    CheckResult { status, headline, details }
}

/// Linux-only shared-lib + display-server checks, folded out of [`check_gui`] so the
/// cfg-gated body has a single clean return point (no partially-initialised notes on
/// other platforms). Returns the `", …"`-prefixed headline suffix (empty off-Linux)
/// and the worst [`Status`] this sub-check produced (`Ok` off-Linux).
#[cfg(target_os = "linux")]
fn check_gui_linux(details: &mut Vec<String>) -> (String, Status) {
    let mut status = Status::Ok;
    let mut note = String::new();

    match std::process::Command::new("ldconfig").arg("-p").output() {
        Ok(out) => {
            let list = String::from_utf8_lossy(&out.stdout);
            let has_webkit = list.contains("libwebkit2gtk-4.1.so");
            let has_gtk3 = list.contains("libgtk-3.so");
            if has_webkit && has_gtk3 {
                note.push_str(", webkit2gtk + gtk-3 present");
            } else {
                status = Status::Fail;
                let mut missing = Vec::new();
                if !has_webkit {
                    missing.push("libwebkit2gtk-4.1-0");
                }
                if !has_gtk3 {
                    missing.push("libgtk-3-0");
                }
                details.push(format!("install {}", missing.join(" / ")));
                note.push_str(", missing shared libs");
            }
        }
        Err(e) => {
            details.push(format!("could not run ldconfig to check shared libs: {e}"));
        }
    }

    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        if status == Status::Ok {
            status = Status::Warn;
        }
        note.push_str(", no display server detected");
        details.push("no display server detected (GUI needs a desktop session)".to_string());
    } else {
        note.push_str(", display ok");
    }

    details.push("WEBKIT_DISABLE_COMPOSITING_MODE is defaulted to 1 by koma on Linux".to_string());

    (note, status)
}

#[cfg(not(target_os = "linux"))]
fn check_gui_linux(_details: &mut Vec<String>) -> (String, Status) {
    (String::new(), Status::Ok)
}

// ─── 5. Extensions ──────────────────────────────────────────────────────────

fn check_extensions() -> CheckResult {
    let config = AppConfig::load();
    if config.installed_extensions.is_empty() {
        return CheckResult::ok("Extensions (none installed)");
    }

    let mut details = Vec::new();
    let mut broken = 0usize;

    let ext_dir = store::extensions_dir();
    for ext in &config.installed_extensions {
        let Ok(base) = ext_dir.as_ref() else {
            broken += 1;
            details.push(format!("{}: cannot resolve extensions dir", ext.id));
            continue;
        };
        let ext_root = base.join(&ext.id);
        let manifest_path = ext_root.join("manifest.json");

        if !manifest_path.exists() {
            broken += 1;
            details.push(format!("{}: manifest.json missing at {}", ext.id, manifest_path.display()));
        } else {
            match std::fs::read(&manifest_path)
                .map_err(anyhow::Error::from)
                .and_then(|b| {
                    serde_json::from_slice::<koma_extension::protocol::ExtensionManifest>(&b)
                        .map_err(anyhow::Error::from)
                })
            {
                Ok(_) => {}
                Err(e) => {
                    broken += 1;
                    details.push(format!("{}: manifest.json failed to parse: {e}", ext.id));
                }
            }
        }

        if ext.exec.is_empty() {
            broken += 1;
            details.push(format!("{}: no exec recorded", ext.id));
            continue;
        }
        let exec_path = ext_root.join(&ext.exec);
        if !exec_path.exists() {
            broken += 1;
            details.push(format!("{}: exec missing at {}", ext.id, exec_path.display()));
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = std::fs::metadata(&exec_path)
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false);
                if !executable {
                    broken += 1;
                    details.push(format!("{}: exec at {} is not executable", ext.id, exec_path.display()));
                }
            }
        }
    }

    let n = config.installed_extensions.len();
    if broken == 0 {
        CheckResult::ok(format!("Extensions ({n} installed, binaries present)"))
    } else {
        CheckResult::warn(format!("Extensions ({n} installed, {broken} with issues)"), details)
    }
}

// ─── 6. Internet full-mode ──────────────────────────────────────────────────

fn check_internet_fullmode() -> CheckResult {
    let mut details = Vec::new();
    let python3 = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    details.push(format!("python3 on PATH: {}", if python3 { "yes" } else { "no" }));

    if crate::internet::is_installed() {
        CheckResult { status: Status::Ok, headline: "Internet full-mode (installed)".to_string(), details }
    } else {
        details.push("optional — install with: koma --internet-fullmode-install".to_string());
        CheckResult { status: Status::Warn, headline: "Internet full-mode (optional, not installed)".to_string(), details }
    }
}

// ─── 7. Security daemon ─────────────────────────────────────────────────────

fn check_security_daemon() -> CheckResult {
    if crate::security::is_installed() {
        CheckResult::ok("Security daemon (optional, installed)")
    } else {
        CheckResult::warn(
            "Security daemon (optional, not installed)",
            vec!["optional — install with: koma --security-install".to_string()],
        )
    }
}

// ─── 8. MCP servers ─────────────────────────────────────────────────────────

fn check_mcp_servers(config_ok: bool) -> CheckResult {
    let config = AppConfig::load();
    let configured = config.mcp_servers.len();
    let enabled = config.mcp_servers.iter().filter(|s| s.enabled).count();

    let mut details = Vec::new();
    for s in &config.mcp_servers {
        details.push(format!(
            "{} ({:?}, {})",
            s.name,
            s.transport,
            if s.enabled { "enabled" } else { "disabled" }
        ));
    }

    let headline = format!("MCP servers ({configured} configured, {enabled} enabled)");
    if !config_ok {
        details.push("config.json is corrupt — these counts reflect defaults, not your real config".to_string());
        return CheckResult::fail(headline, details);
    }
    CheckResult { status: Status::Ok, headline, details }
}

// ─── 9. Update ──────────────────────────────────────────────────────────────

fn check_update() -> CheckResult {
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let fetched = (|| -> Result<String, ()> {
            let client = reqwest::blocking::Client::builder()
                .timeout(UPDATE_CHECK_TIMEOUT)
                .user_agent(concat!("koma/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| ())?;
            let resp = client
                .get(crate::app::version::VERSION_URL)
                .send()
                .map_err(|_| ())?;
            if !resp.status().is_success() {
                return Err(());
            }
            let body = resp.text().map_err(|_| ())?;
            let info: crate::app::version::VersionInfo =
                serde_json::from_str(&body).map_err(|_| ())?;
            Ok(info.version)
        })();
        let _ = tx.send(fetched.ok());
    });

    match rx.recv_timeout(UPDATE_CHECK_TIMEOUT) {
        Ok(Some(latest)) => {
            let current = store::current_version();
            if crate::app::version::is_newer(&latest, current) {
                CheckResult::warn(
                    format!("Update ({latest} available)"),
                    vec!["run: koma update".to_string()],
                )
            } else {
                CheckResult::ok(format!("Update (up to date, {current})"))
            }
        }
        Ok(None) | Err(_) => CheckResult {
            status: Status::Ok,
            headline: "Update (check skipped)".to_string(),
            details: vec!["update check skipped — offline?".to_string()],
        },
    }
}

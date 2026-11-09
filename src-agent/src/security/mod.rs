//! Security daemon provisioner — M1 installer.
//!
//! Bundles the Python security daemon (`koma_sec_daemon`) that provides
//! pwntools/requests-based security tooling. The package lives in
//! `src-security/` (sibling of this crate) and is embedded verbatim into the
//! binary at compile time via [`include_dir!`].
//!
//! # Public surface
//!
//! | Function | Purpose |
//! |---|---|
//! | [`security_dir`] | `~/.koma/security/` — install root |
//! | [`venv_python`] | path to the venv Python; used as "installed" marker |
//! | [`is_installed`] | non-panicking predicate consumed by gating logic |
//! | [`install`] | provisions the environment (CLI mode, prints progress) |
//! | [`safety_image_tag`] / [`safety_image_tag_for`] | GHCR tag (minimal by default) |
//! | [`read_images_inventory`] | `images.json` composition inventory |
//! | [`pull_composition`] / [`activate_composition`] / [`remove_composition`] | Images pane backend |

use anyhow::{anyhow, Context, Result};
use include_dir::{include_dir, Dir};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::model::store::base_dir;

/// Embedded snapshot of `src-security/` baked in at compile time.
///
/// The macro path is relative to `$CARGO_MANIFEST_DIR` (i.e. `src-agent/`),
/// so `../src-security` resolves to the sibling directory that holds the
/// vendored `koma_sec_daemon` package and `requirements.txt`.
static SECURITY_ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../src-security");

// Inventory/pull APIs are consumed by the Images pane and daemon spawn path.
mod images;
mod progress;
mod pull;
use images::pull_safety_image;
pub use images::{
    activate_composition, active_composition_id, composition_tag, list_known_compositions,
    pull_composition, read_images_inventory, remove_composition, safety_image_tag_for,
};
#[allow(unused_imports)] // public inventory types for callers/tests
pub use images::{ImageEntry, ImagesInventory};

/// How long a runtime probe answer is served before a refresh is allowed.
///
/// Mirrors the `/mcp` status cache idea: short enough that starting Docker/Colima
/// shows up on the `/security` panel within a few seconds, long enough that the
/// ~10Hz snapshot projection and per-key panel handlers never spawn `docker info`
/// (plus colima/podman) on the hot path.
const RUNTIME_PROBE_TTL: Duration = Duration::from_secs(3);

/// Last known container-runtime probe. `at = None` means "never probed".
struct RuntimeProbeCache {
    at: Option<Instant>,
    ok: bool,
    label: String,
}

static RUNTIME_PROBE_CACHE: OnceLock<Mutex<RuntimeProbeCache>> = OnceLock::new();
/// Single-flight guard so N ~10Hz callers past a stale TTL don't each spawn
/// their own `docker`/`colima`/`podman` subprocess storm.
static RUNTIME_PROBE_REFRESHING: AtomicBool = AtomicBool::new(false);

fn runtime_probe_cache() -> &'static Mutex<RuntimeProbeCache> {
    RUNTIME_PROBE_CACHE.get_or_init(|| {
        Mutex::new(RuntimeProbeCache {
            at: None,
            ok: false,
            label: "checking runtime…".into(),
        })
    })
}

fn runtime_probe_stale(cache: &RuntimeProbeCache) -> bool {
    match cache.at {
        Some(at) => at.elapsed() >= RUNTIME_PROBE_TTL,
        None => true,
    }
}

fn store_runtime_probe(ok: bool, label: String) {
    let mut guard = runtime_probe_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    guard.at = Some(Instant::now());
    guard.ok = ok;
    guard.label = label;
}

fn read_runtime_probe() -> (bool, String) {
    let guard = runtime_probe_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    (guard.ok, guard.label.clone())
}

/// Run the real docker/colima/podman probes once. No cache. Detection only —
/// never starts an engine.
fn probe_runtime_now() -> (bool, String) {
    let docker = docker_daemon_reachable_uncached();
    let colima = command_ok("colima", &["version"]);
    let podman = command_ok("podman", &["info"]);
    let docker_cli = !docker && !colima && !podman && command_ok("docker", &["--version"]);
    let label = runtime_label(docker, colima, podman, docker_cli);
    (docker, label)
}

/// Pure label builder for the `/security` runtime strip (and tests).
fn runtime_label(docker: bool, colima: bool, podman: bool, docker_cli: bool) -> String {
    match (docker, colima, podman) {
        (true, true, _) => "docker ready (colima)".into(),
        (true, _, true) => "docker ready (podman)".into(),
        (true, _, _) => "docker ready".into(),
        (false, true, _) => "docker down — colima installed".into(),
        (false, _, true) => "docker down — podman present".into(),
        (false, false, false) if docker_cli => "docker installed but not running".into(),
        _ => "no container runtime".into(),
    }
}

/// Kick a background runtime probe if `force` or the cache is empty/stale.
/// Single-flight. Returns immediately; callers read [`runtime_status_cached`].
pub fn runtime_status_kick(force: bool) {
    let stale = {
        let guard = runtime_probe_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        force || runtime_probe_stale(&guard)
    };
    if !stale {
        return;
    }
    if RUNTIME_PROBE_REFRESHING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    std::thread::spawn(|| {
        let (ok, label) = probe_runtime_now();
        store_runtime_probe(ok, label);
        RUNTIME_PROBE_REFRESHING.store(false, Ordering::Release);
    });
}

/// Non-blocking read for the `/security` panel + snapshot projection.
///
/// Returns the last known `(runtime_ok, label)` immediately. If the cache is
/// empty or older than [`RUNTIME_PROBE_TTL`], also kicks a background refresh
/// (single-flight). Never spawns subprocesses on the calling thread.
pub fn runtime_status_cached() -> (bool, String) {
    runtime_status_kick(false);
    read_runtime_probe()
}

/// Blocking refresh for user actions / open / tool gate when a fresh answer is
/// required. Honours TTL unless `force` is set. Coalesces with an in-flight
/// background probe instead of stacking another `docker info`.
pub fn runtime_status_refresh(force: bool) -> (bool, String) {
    if !force {
        let guard = runtime_probe_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !runtime_probe_stale(&guard) {
            return (guard.ok, guard.label.clone());
        }
    }
    if RUNTIME_PROBE_REFRESHING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        let (ok, label) = probe_runtime_now();
        store_runtime_probe(ok, label.clone());
        RUNTIME_PROBE_REFRESHING.store(false, Ordering::Release);
        return (ok, label);
    }
    // Another probe owns the lock. Wait for it — `docker info` can take several
    // seconds on slow arm CI, and returning the pre-probe `ok=false` default here
    // made sec_refusal flip false→true across back-to-back calls (baseline tests).
    for _ in 0..500 {
        std::thread::sleep(Duration::from_millis(20));
        if !RUNTIME_PROBE_REFRESHING.load(Ordering::Acquire) {
            let guard = runtime_probe_cache()
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if guard.at.is_some() && (force || !runtime_probe_stale(&guard)) {
                return (guard.ok, guard.label.clone());
            }
            break;
        }
    }
    // Peer never published a fresh answer — take over rather than serve stale.
    if RUNTIME_PROBE_REFRESHING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        let (ok, label) = probe_runtime_now();
        store_runtime_probe(ok, label.clone());
        RUNTIME_PROBE_REFRESHING.store(false, Ordering::Release);
        return (ok, label);
    }
    read_runtime_probe()
}

/// Does an isolated execution runtime exist, so a `sec_` call could be contained?
///
/// Requires a container with no silent host fallback. Answers from the TTL cache
/// (refreshing synchronously when stale) so a Docker/Colima start after boot is
/// noticed within [`RUNTIME_PROBE_TTL`] without re-probing on every model call.
///
/// A missing runtime is reported as `unavailable` by the caller, never as success
/// and never as a host fallback.
pub(crate) fn cyber_execution_available() -> bool {
    runtime_status_refresh(false).0
}

fn docker_daemon_reachable_uncached() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .arg("--format")
        .arg("{{.ServerVersion}}")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Re-extract the embedded `src-security` tree into `~/.koma/security/` without
/// rebuilding the venv. Used when the agent binary has moved past a stale
/// on-disk daemon package (the common "all tools refused" failure mode).
pub fn refresh_daemon_sources() -> Result<()> {
    let dest = security_dir()?;
    std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
    extract_assets(&SECURITY_ASSETS, &dest)?;
    Ok(())
}

/// Tools whose destination is fixed in the handler, so they carry no destination
/// argument and there is nothing to bind. Mirrors `effects.FIXED_HOST`, pinned by
/// a test on each side.
pub(crate) fn fixed_host(name: &str) -> bool {
    matches!(name, "sec_factor")
}

/// True when `name` belongs to the security daemon's `sec_` namespace.
///
/// Kept here rather than at the call sites because [`execute_tool`] and the
/// sub-agent loop both need it, and `security` is reachable from lib-less
/// modules that cannot see `tool`. `sec_` is a namespace prefix, not a risk
/// class — see [`crate::app::runtime::stream::tools::approval`] for how the
/// exemption perverts approval once a call has already been admitted.
///
/// (See `crate::tool::execute_tool` for the dispatch site.)
pub(crate) fn is_sec_tool(name: &str) -> bool {
    name.starts_with("sec_")
}

/// Why a `sec_` call may not run right now, or `None` when it may.
///
/// Phase-1 fail-closed boundary. Deliberately narrow and **total**: every
/// route that can reach the daemon resolves through this, so an unadvertised
/// or forged name cannot execute even when the daemon is live and the lane is
/// on. Reasons are stable strings — they are asserted by tests and echoed to
/// the model, so they must not carry run-specific data.
///
/// Callers pass `enabled` (the session's lane flag) and `advertised` (names the
/// daemon handed over for this session) rather than reading state themselves:
/// the check re-runs at dispatch precisely so a lane stopped mid-round still
/// refuses, and so a sub-agent inherits its parent's decision instead of
/// re-deriving one.
/// The final check takes a closure rather than `&[String]` so the hot path can
/// answer from borrowed state: collecting advertised names allocates a clone of
/// every `sec_` tool on each model call.
#[must_use]
pub(crate) fn sec_refusal(
    name: &str,
    enabled: bool,
    advertised: impl FnOnce(&str) -> bool,
) -> Option<String> {
    if !is_sec_tool(name) {
        return None;
    }
    if !enabled {
        return Some("cyber lane off — enable security mode in /security first".into());
    }
    // Advertised-ness is checked BEFORE runtime availability: a forged name is
    // forged regardless of whether the runtime happens to be configured, and
    // "not advertised" is the more specific of the two diagnoses.
    if !advertised(name) {
        return Some(format!(
            "tool '{name}' is not advertised for this session (disabled or unknown)"
        ));
    }
    if !cyber_execution_available() {
        return Some("security execution unavailable: isolated runtime not configured".into());
    }
    None
}

/// Returns `~/.koma/security/` — the root of the security daemon install.
pub fn security_dir() -> Result<PathBuf> {
    Ok(base_dir()?.join("security"))
}

/// Returns the path to the venv Python binary:
/// `~/.koma/security/venv/bin/python`.
///
/// The presence of this file is the canonical "installed" marker.
pub fn venv_python() -> Result<PathBuf> {
    Ok(security_dir()?.join("venv").join("bin").join("python"))
}

/// Non-panicking predicate: `true` iff the venv Python binary exists on disk.
///
/// Any error resolving the path is treated as "not installed" rather than
/// propagated.
pub fn is_installed() -> bool {
    venv_python().map(|p| p.exists()).unwrap_or(false)
}

/// Extract the embedded `src-security/` assets into `dest`, overwriting
/// existing files.
///
/// Uses a hand-rolled recursive extractor because [`Dir::extract`] from
/// `include_dir 0.7` refuses to overwrite existing files (returns an `Err` if
/// a file already exists). Walking entries manually lets us call
/// [`std::fs::write`] unconditionally, which is idempotent.
fn extract_assets(dir: &Dir, dest: &PathBuf) -> Result<usize> {
    use include_dir::DirEntry;

    let mut written = 0usize;
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(sub) => {
                // The guest image is pulled from GHCR. The Dockerfile is the
                // release input, not a file the install writes out.
                if !ships_to_the_user(sub.path()) {
                    continue;
                }
                let sub_dest = dest.join(sub.path());
                std::fs::create_dir_all(&sub_dest)
                    .with_context(|| format!("create dir {}", sub_dest.display()))?;
                written += extract_assets(sub, dest)?;
            }
            DirEntry::File(f) => {
                if !ships_to_the_user(f.path()) {
                    continue;
                }
                let file_dest = dest.join(f.path());
                if let Some(parent) = file_dest.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("create dir {}", parent.display()))?;
                }
                std::fs::write(&file_dest, f.contents())
                    .with_context(|| format!("write {}", file_dest.display()))?;
                written += 1;
            }
        }
    }
    Ok(written)
}

/// Provision the host security environment and pull the minimal guest image.
///
/// Does not start the daemon, set a lane, or arm YOLO. Progress is written to
/// stderr in the same shape as `install.sh`: a named step, then a live bar
/// while the guest image downloads.
///
/// 1. Refuse on Windows.
/// 2. Ensure `python3`, offering a package-manager install on a TTY.
/// 3. Ensure a container runtime. A running Docker daemon wins.
/// 4. Re-extract embedded `src-security/` into `~/.koma/security/`, skipping `image/`.
/// 5. Create the venv and install Python packages, unless the venv exists and
///    `force` is false.
/// 6. Pull `ghcr.io/aula-id/koma-safety:{version}-minimal` and mark it active.
#[allow(unreachable_code)]
pub fn install(force: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = force;
        anyhow::bail!("security mode is not supported on Windows");
    }

    let dest = security_dir()?;
    let ui = progress::InstallUi::new();
    let image = safety_image_tag();
    ui.banner(&host_label(), &dest.display().to_string(), &image);

    // The venv can already be present while the guest image is not, so a repeat
    // install still pulls. The embedded package is always rewritten: skipping
    // that when the venv existed left users on a stale wire vocabulary.
    ui.step("Python");
    ensure_requirement(&ui, "python3")?;
    ui.ok(&python_version());

    ui.step("Container runtime");
    ensure_container_runtime(&ui)?;
    ui.ok("docker ready");

    ui.step("Security files");
    std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
    let written = extract_assets(&SECURITY_ASSETS, &dest)?;
    ui.ok(&format!("{written} files in {}", dest.display()));

    let venv_ready = is_installed() && !force;
    if venv_ready {
        ui.step("Python environment");
        ui.ok("already present");
    } else {
        let venv = dest.join("venv");
        ui.step("Python environment");
        if force && venv.exists() {
            ui.info("replacing the existing environment");
            std::fs::remove_dir_all(&venv).with_context(|| format!("remove {}", venv.display()))?;
        }
        let venv_dir = venv.display().to_string();
        ui.info(&venv_dir);
        inherit("python3", &["-m", "venv", &venv_dir])?;
        ui.ok("created");

        let pip = venv.join("bin").join("pip");
        let pip = pip
            .to_str()
            .ok_or_else(|| anyhow!("venv pip path is not valid UTF-8"))?;
        let requirements = dest.join("requirements.txt");
        let req = requirements.display().to_string();

        ui.step("Python packages");
        ui.info("pip, setuptools, wheel");
        let _ = inherit(pip, &["install", "--upgrade", "pip", "setuptools", "wheel"]);
        ui.info(&req);
        inherit(pip, &["install", "-r", &req])?;
        ui.info("checksec.py");
        inherit(pip, &["install", "--no-deps", "checksec.py"])?;
        ui.ok("installed");
    }

    ui.step("Guest image");
    ui.info(&image);
    let report = pull_safety_image(&dest, force, &ui)?;
    ui.ok(&report.summary());
    ui.finish(&["minimal is active", "Domain images install from /security."]);
    Ok(())
}

/// GHCR name of the guest image. Tags are `{version}-{composition}`.
pub const SAFETY_IMAGE: &str = "ghcr.io/aula-id/koma-safety";

/// Tag the installer pulls (`…:{version}-minimal`). `KOMA_SEC_IMAGE` overrides it
/// for a local build.
#[must_use]
pub fn safety_image_tag() -> String {
    match std::env::var("KOMA_SEC_IMAGE") {
        Ok(over) if !over.is_empty() => over,
        _ => safety_image_tag_for("minimal"),
    }
}

pub(super) fn image_ref_path(dest: &std::path::Path) -> PathBuf {
    dest.join("image-ref")
}

/// Digest reference of the **active** composition for this version.
///
/// Prefers `images.json` active digest; falls back to legacy `image-ref` line 2.
#[must_use]
pub fn pinned_safety_image() -> Option<String> {
    images::pinned_from_inventory_or_ref()
}

fn ships_to_the_user(path: &std::path::Path) -> bool {
    !path.starts_with("image")
}

fn host_label() -> String {
    let os = match host_os() {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}/{arch}")
}

fn python_version() -> String {
    let Ok(out) = std::process::Command::new("python3")
        .arg("--version")
        .output()
    else {
        return "python3".to_string();
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if stdout.trim().is_empty() {
        String::from_utf8_lossy(&out.stderr).into_owned()
    } else {
        stdout.into_owned()
    };
    text.lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .unwrap_or("python3")
        .to_string()
}

fn runtime_summary(seen: RuntimeSeen) -> String {
    let docker = if seen.docker_daemon {
        "ready"
    } else if seen.docker_cli {
        "down"
    } else {
        "absent"
    };
    let colima = if seen.colima { "installed" } else { "absent" };
    let podman = if seen.podman_engine {
        "ready"
    } else if seen.podman_cli {
        "down"
    } else {
        "absent"
    };
    let kubernetes = if seen.kubernetes {
        "configured"
    } else {
        "absent"
    };
    format!("docker {docker} · colima {colima} · podman {podman} · kubernetes {kubernetes}")
}

/// Run a command with the terminal attached, so brew, pip, and colima show
/// their own progress instead of looking frozen until they exit.
pub(super) fn inherit(program: &str, args: &[&str]) -> Result<()> {
    let _ = std::io::stderr().flush();
    let _ = std::io::stdout().flush();
    let status = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .with_context(|| format!("failed to launch {program}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("{program} exited with status {status}"))
    }
}

fn command_ok(name: &str, args: &[&str]) -> bool {
    std::process::Command::new(name)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tool_ok(name: &str) -> bool {
    let args: &[&str] = if name == "docker" {
        &["info", "--format", "{{.ServerVersion}}"]
    } else {
        &["--version"]
    };
    command_ok(name, args)
}

/// What the install probe saw. The guest itself only speaks the `docker` client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeSeen {
    docker_cli: bool,
    docker_daemon: bool,
    colima: bool,
    podman_cli: bool,
    podman_engine: bool,
    kubernetes: bool,
}

/// What to do when `docker info` is not already succeeding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeChoice {
    Ready,
    StartColima {
        need_client: bool,
    },
    UsePodman {
        need_client: bool,
        need_machine: bool,
    },
    KubernetesOnly,
    DockerStopped,
    InstallDocker,
}

fn runtime_choice(seen: RuntimeSeen) -> RuntimeChoice {
    if seen.docker_daemon {
        return RuntimeChoice::Ready;
    }
    if seen.colima {
        return RuntimeChoice::StartColima {
            need_client: !seen.docker_cli,
        };
    }
    if seen.podman_cli {
        return RuntimeChoice::UsePodman {
            need_client: !seen.docker_cli,
            need_machine: !seen.podman_engine,
        };
    }
    if seen.kubernetes {
        return RuntimeChoice::KubernetesOnly;
    }
    if seen.docker_cli {
        return RuntimeChoice::DockerStopped;
    }
    RuntimeChoice::InstallDocker
}

fn probe_runtime() -> RuntimeSeen {
    RuntimeSeen {
        docker_cli: command_ok("docker", &["--version"]),
        docker_daemon: command_ok("docker", &["info", "--format", "{{.ServerVersion}}"]),
        colima: command_ok("colima", &["version"]),
        podman_cli: command_ok("podman", &["--version"]),
        podman_engine: command_ok("podman", &["info"]),
        kubernetes: command_ok("kubectl", &["config", "current-context"]),
    }
}

/// What to install when Docker, Colima, Podman, and Kubernetes are all absent.
/// macOS gets Colima. Linux gets the Docker engine, not Docker Desktop.
fn offered_runtime_argv(os: &str, manager: &str) -> Option<Vec<&'static str>> {
    match (os, manager) {
        ("macos", "brew") => Some(vec!["brew", "install", "colima", "docker"]),
        ("linux", "apt") => Some(vec!["sudo", "apt-get", "install", "-y", "docker.io"]),
        ("linux", "dnf") => Some(vec!["sudo", "dnf", "install", "-y", "docker"]),
        ("linux", "pacman") => Some(vec!["sudo", "pacman", "-S", "--noconfirm", "docker"]),
        _ => None,
    }
}

fn docker_client_argv(os: &str, manager: &str) -> Option<Vec<&'static str>> {
    match (os, manager) {
        ("macos", "brew") => Some(vec!["brew", "install", "docker"]),
        ("linux", "apt") => Some(vec!["sudo", "apt-get", "install", "-y", "docker.io"]),
        ("linux", "dnf") => Some(vec!["sudo", "dnf", "install", "-y", "docker"]),
        ("linux", "pacman") => Some(vec!["sudo", "pacman", "-S", "--noconfirm", "docker"]),
        _ => None,
    }
}

fn accepts_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn requirement_argv(os: &str, manager: &str, tool: &str) -> Option<Vec<&'static str>> {
    match (os, manager, tool) {
        ("macos", "brew", "python3") => Some(vec!["brew", "install", "python@3.12"]),
        ("linux", "apt", "python3") => Some(vec![
            "sudo",
            "apt-get",
            "install",
            "-y",
            "python3",
            "python3-venv",
        ]),
        ("linux", "apt", "docker") => Some(vec!["sudo", "apt-get", "install", "-y", "docker.io"]),
        ("linux", "dnf", "python3") => Some(vec!["sudo", "dnf", "install", "-y", "python3"]),
        ("linux", "dnf", "docker") => Some(vec!["sudo", "dnf", "install", "-y", "docker"]),
        ("linux", "pacman", "python3") => {
            Some(vec!["sudo", "pacman", "-S", "--noconfirm", "python"])
        }
        ("linux", "pacman", "docker") => {
            Some(vec!["sudo", "pacman", "-S", "--noconfirm", "docker"])
        }
        _ => None,
    }
}

fn host_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

fn package_manager() -> &'static str {
    if host_os() == "macos" {
        return "brew";
    }
    for (bin, name) in [("apt-get", "apt"), ("dnf", "dnf"), ("pacman", "pacman")] {
        if std::process::Command::new(bin)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return name;
        }
    }
    ""
}

fn ensure_requirement(ui: &progress::InstallUi, tool: &str) -> Result<()> {
    if tool_ok(tool) {
        return Ok(());
    }
    let argv = requirement_argv(host_os(), package_manager(), tool);
    let shown = argv
        .as_ref()
        .map(|a| a.join(" "))
        .unwrap_or_else(|| format!("install {tool}"));
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if !tty {
        return Err(anyhow!(
            "{tool} is missing. Install it, then rerun koma --security-install:\n  {shown}"
        ));
    }
    if !confirm(&format!("{tool} is missing. Install it with `{shown}`?"))? {
        return Err(anyhow!("{tool} is required. Install was declined."));
    }
    let Some(argv) = argv else {
        return Err(anyhow!("no package manager can install {tool}"));
    };
    if host_os() == "linux" && package_manager() == "apt" {
        ui.info("sudo apt-get update");
        inherit("sudo", &["apt-get", "update"])?;
    }
    ui.info(&shown);
    inherit(argv[0], &argv[1..])?;
    if !tool_ok(tool) {
        return Err(anyhow!(
            "{tool} was installed but is not running. Open it, then rerun koma --security-install."
        ));
    }
    Ok(())
}

fn offer_runtime(ui: &progress::InstallUi) -> Result<()> {
    let os = host_os();
    if os == "other" {
        return Err(anyhow!("security mode is not supported on Windows"));
    }
    let argv = offered_runtime_argv(os, package_manager());
    let shown = argv
        .as_ref()
        .map(|args| args.join(" "))
        .unwrap_or_else(|| "install a container engine".to_string());
    let prompt = if os == "macos" {
        format!("No container engine is installed. Install Colima with `{shown}` and start it?")
    } else {
        format!("No container engine is installed. Install the Docker engine with `{shown}` and start it?")
    };
    if !confirm(&prompt)? {
        return Err(anyhow!(
            "a container engine is required. Install was declined.\n  {shown}"
        ));
    }
    let Some(argv) = argv else {
        return Err(anyhow!("no package manager can install a container engine"));
    };
    if os == "linux" && package_manager() == "apt" {
        ui.info("sudo apt-get update");
        inherit("sudo", &["apt-get", "update"])?;
    }
    ui.info(&shown);
    inherit(argv[0], &argv[1..])?;
    if os == "macos" {
        ui.info("colima start");
        inherit("colima", &["start"])?;
    } else {
        ui.info("sudo systemctl enable --now docker");
        inherit("sudo", &["systemctl", "enable", "--now", "docker"])?;
    }
    if tool_ok("docker") {
        Ok(())
    } else {
        Err(anyhow!(
            "the engine was installed but `docker info` still fails. Rerun koma --security-install."
        ))
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if !tty {
        return Ok(false);
    }
    let mut err = std::io::stderr();
    writeln!(err)?;
    write!(err, "{prompt} [y/N] ")?;
    err.flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(accepts_yes(&line))
}

fn install_docker_client(ui: &progress::InstallUi) -> Result<()> {
    let argv = docker_client_argv(host_os(), package_manager());
    let shown = argv
        .as_ref()
        .map(|a| a.join(" "))
        .unwrap_or_else(|| "install docker".to_string());
    if !confirm(&format!(
        "The docker client is missing. Install it with `{shown}`?"
    ))? {
        return Err(anyhow!(
            "the docker client is required. Install was declined.\n  {shown}"
        ));
    }
    let Some(argv) = argv else {
        return Err(anyhow!("no package manager can install the docker client"));
    };
    ui.info(&shown);
    inherit(argv[0], &argv[1..])?;
    if !command_ok("docker", &["--version"]) {
        return Err(anyhow!(
            "the docker client was installed but is not on PATH"
        ));
    }
    Ok(())
}

fn podman_socket() -> Option<String> {
    let out = std::process::Command::new("podman")
        .args(["info", "--format", "{{.Host.RemoteSocket.Path}}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn point_docker_at_podman(socket: &str) -> Result<()> {
    let host = format!("host=unix://{socket}");
    let _ = std::process::Command::new("docker")
        .args(["context", "create", "koma-podman", "--docker", &host])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    quiet("docker", &["context", "use", "koma-podman"])
}

/// The security guest is started with the `docker` client. Colima and Podman
/// count when they can make that client reach a local engine. A Kubernetes
/// context does not.
fn ensure_container_runtime(ui: &progress::InstallUi) -> Result<()> {
    let seen = probe_runtime();
    ui.info(&runtime_summary(seen));
    match runtime_choice(seen) {
        RuntimeChoice::Ready => Ok(()),
        RuntimeChoice::StartColima { need_client } => {
            if need_client {
                install_docker_client(ui)?;
            }
            if !confirm("Colima is installed but Docker is not running. Start it with `colima start`?")? {
                return Err(anyhow!(
                    "Colima is installed but not running. Start it with `colima start`, then rerun koma --security-install."
                ));
            }
            ui.info("colima start");
            inherit("colima", &["start"])?;
            if tool_ok("docker") {
                Ok(())
            } else {
                Err(anyhow!(
                    "Colima started but `docker info` still fails. Check `colima status`, then rerun koma --security-install."
                ))
            }
        }
        RuntimeChoice::UsePodman { need_client, need_machine } => {
            if need_client {
                install_docker_client(ui)?;
            }
            if need_machine {
                if !confirm("Podman is installed but not running. Start it with `podman machine start`?")? {
                    return Err(anyhow!(
                        "Podman is installed but not running. Start it with `podman machine start`, then rerun koma --security-install."
                    ));
                }
                ui.info("podman machine start");
                inherit("podman", &["machine", "start"])?;
            } else if !confirm("Podman is running but Docker is not. Use Podman's socket for the security guest?")? {
                return Err(anyhow!(
                    "Podman is running, but the docker client is not pointed at it. Rerun koma --security-install and accept the prompt."
                ));
            }
            let socket = podman_socket().ok_or_else(|| {
                anyhow!("Podman did not report a socket. `docker` cannot reach it.")
            })?;
            point_docker_at_podman(&socket)?;
            if tool_ok("docker") {
                Ok(())
            } else {
                Err(anyhow!(
                    "Podman is up, but `docker info` still fails through context koma-podman."
                ))
            }
        }
        RuntimeChoice::KubernetesOnly => Err(anyhow!(
            "Kubernetes is configured, but the security guest runs on a local Docker engine. Start Docker, Colima, or Podman, then rerun koma --security-install."
        )),
        RuntimeChoice::DockerStopped => Err(anyhow!(
            "Docker is installed but not running. Open it, then rerun koma --security-install."
        )),
        RuntimeChoice::InstallDocker => offer_runtime(ui),
    }
}

pub(super) fn quiet(program: &str, args: &[&str]) -> Result<()> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to launch {program}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = err.lines().rev().take(6).collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    Err(anyhow!(
        "{program} exited with status {}\n{tail}",
        out.status
    ))
}

/// One compact catalog row from `index.json` or the daemon `catalog index` op.
///
/// File rows use `summary`; the daemon uses `blurb`. Both map here. Readiness is
/// not in the payload — the panel overlays it from the image inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogIndexRow {
    pub id: String,
    pub blurb: String,
    pub image: String,
    pub exec: String,
    pub domain: String,
}

#[derive(serde::Deserialize)]
struct CatalogIndexFile {
    #[serde(default)]
    tools: Vec<CatalogIndexJson>,
}

#[derive(serde::Deserialize)]
struct CatalogIndexJson {
    #[serde(default)]
    id: String,
    #[serde(default)]
    blurb: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    image: String,
    #[serde(default)]
    exec: String,
    #[serde(default)]
    domain: String,
}

/// Embedded `src-security/catalog/index.json` (path relative to this file).
///
/// Marks start unset; callers overlay them. Empty if the file does not parse.
pub fn embedded_catalog_rows() -> Vec<CatalogIndexRow> {
    parse_catalog_index(include_str!("../../../src-security/catalog/index.json"))
        .unwrap_or_default()
}

/// Parse a catalog index object (`{"tools":[...]}`). Accepts file `summary` or
/// daemon `blurb`. Rows with an empty id are dropped. `None` if `raw` is not JSON.
pub fn parse_catalog_index(raw: &str) -> Option<Vec<CatalogIndexRow>> {
    let file: CatalogIndexFile = serde_json::from_str(raw).ok()?;
    Some(
        file.tools
            .into_iter()
            .filter_map(|row| {
                let id = row.id.trim().to_string();
                if id.is_empty() {
                    return None;
                }
                let blurb = if row.blurb.trim().is_empty() {
                    row.summary
                } else {
                    row.blurb
                };
                Some(CatalogIndexRow {
                    id,
                    blurb: blurb.trim().to_string(),
                    image: row.image.trim().to_string(),
                    exec: row.exec.trim().to_string(),
                    domain: row.domain.trim().to_string(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

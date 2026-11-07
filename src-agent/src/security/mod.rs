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

use anyhow::{anyhow, Context, Result};
use include_dir::{include_dir, Dir};
use std::path::PathBuf;

use crate::model::store::base_dir;

/// Embedded snapshot of `src-security/` baked in at compile time.
///
/// The macro path is relative to `$CARGO_MANIFEST_DIR` (i.e. `src-agent/`),
/// so `../src-security` resolves to the sibling directory that holds the
/// vendored `koma_sec_daemon` package and `requirements.txt`.
static SECURITY_ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/../src-security");

/// Does an isolated execution runtime exist, so a `sec_` call could be contained?
///
/// `21` requires a container with no silent host fallback, so this is the gate
/// that keeps a call from running uncontained. It reports whether Docker is
/// reachable from THIS process, which is the same answer for every call and cheap
/// enough to re-check at dispatch — a probe on the hot path would cost a syscall
/// per tool call, so the result is memoised for the process lifetime.
///
/// A missing runtime is reported as `unavailable` by the caller, never as success
/// and never as a host fallback.
pub(crate) fn cyber_execution_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        // Only detection, nothing clever: the boundary itself spawns containers.
        std::process::Command::new("docker")
            .arg("info")
            .arg("--format")
            .arg("{{.ServerVersion}}")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
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
fn extract_assets(dir: &Dir, dest: &PathBuf) -> Result<()> {
    use include_dir::DirEntry;

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
                extract_assets(sub, dest)?;
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
            }
        }
    }
    Ok(())
}

/// Provision the Python security daemon environment.
///
/// This runs in CLI command mode (before the TUI starts), so progress is
/// printed to stdout/stderr via [`println!`] / [`eprintln!`].
///
/// # Steps
/// 1. Confirm `python3` is available.
/// 2. Early-exit if already installed and `force` is false.
/// 3. On `force`, remove the existing `venv/` so pip rebuilds it cleanly,
///    then overwrite the Python source files.
/// 4. Extract embedded assets into `security_dir()`.
/// 5. Create the venv.
/// 6. Install Python deps from `requirements.txt`.
#[allow(unreachable_code)]
pub fn install(force: bool) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = force;
        anyhow::bail!("security mode is not supported on Windows");
    }

    let dest = security_dir()?;

    // Step 2: idempotency guard. The venv can already be present while the
    // guest image is not, so a repeat install still pulls.
    let mut bar = Bar::new(5);
    bar.tick("checking requirements");
    ensure_requirement("python3")?;
    ensure_requirement("docker")?;

    let venv_ready = dest.exists() && is_installed() && !force;
    if venv_ready {
        bar.tick("writing security files");
        bar.tick("creating the Python environment");
        bar.tick("installing Python packages");
    } else {
        bar.tick("writing security files");

        // Step 3: on force, remove the venv so it is rebuilt from scratch.
        // We keep the Python source files (they will be overwritten in step 4).
        if force {
            let venv = dest.join("venv");
            if venv.exists() {
                std::fs::remove_dir_all(&venv)
                    .with_context(|| format!("remove {}", venv.display()))?;
            }
        }

        // Step 4: extract embedded assets (creates dest if needed).
        std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
        extract_assets(&SECURITY_ASSETS, &dest)?;

        bar.tick("creating the Python environment");
        let venv_dir = dest.join("venv").display().to_string();
        quiet("python3", &["-m", "venv", &venv_dir])?;

        // Step 6: install Python dependencies.
        let pip = dest.join("venv").join("bin").join("pip");
        let requirements = dest.join("requirements.txt");

        bar.tick("installing Python packages");
        let _ = quiet(
            pip.to_str().unwrap_or("pip"),
            &[
                "install",
                "--upgrade",
                "--quiet",
                "pip",
                "setuptools",
                "wheel",
            ],
        );

        let req = requirements.display().to_string();
        quiet(
            pip.to_str().unwrap_or("pip"),
            &["install", "--quiet", "-r", &req],
        )?;
        quiet(
            pip.to_str().unwrap_or("pip"),
            &["install", "--quiet", "--no-deps", "checksec.py"],
        )?;
    }

    bar.tick("downloading koma-safety");
    pull_safety_image(&dest, force)?;
    bar.done();
    Ok(())
}

/// GHCR name of the guest image. The tag is this build's crate version.
pub const SAFETY_IMAGE: &str = "ghcr.io/aula-id/koma-safety";

/// Tag the installer pulls. `KOMA_SEC_IMAGE` overrides it for a local build.
#[must_use]
pub fn safety_image_tag() -> String {
    match std::env::var("KOMA_SEC_IMAGE") {
        Ok(over) if !over.is_empty() => over,
        _ => format!("{SAFETY_IMAGE}:{}", env!("CARGO_PKG_VERSION")),
    }
}

fn image_ref_path(dest: &std::path::Path) -> PathBuf {
    dest.join("image-ref")
}

/// Digest reference written by the last successful pull for this version.
#[must_use]
pub fn pinned_safety_image() -> Option<String> {
    let dest = security_dir().ok()?;
    let text = std::fs::read_to_string(image_ref_path(&dest)).ok()?;
    let mut lines = text.lines();
    let version = lines.next()?.trim();
    let reference = lines.next()?.trim();
    (version == env!("CARGO_PKG_VERSION") && !reference.is_empty()).then(|| reference.to_string())
}

fn ships_to_the_user(path: &std::path::Path) -> bool {
    !path.starts_with("image")
}

struct Bar {
    step: u8,
    total: u8,
}

impl Bar {
    fn new(total: u8) -> Self {
        Self { step: 0, total }
    }

    fn tick(&mut self, label: &str) {
        self.step = self.step.saturating_add(1).min(self.total);
        let width = 24usize;
        let filled = (self.step as usize * width) / self.total as usize;
        println!(
            "[{}{}] {}/{}  {label}",
            "#".repeat(filled),
            "-".repeat(width - filled),
            self.step,
            self.total
        );
    }

    fn done(self) {
        println!("security install complete");
    }
}

fn tool_ok(name: &str) -> bool {
    let args: &[&str] = if name == "docker" {
        &["info", "--format", "{{.ServerVersion}}"]
    } else {
        &["--version"]
    };
    std::process::Command::new(name)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn accepts_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn requirement_argv(os: &str, manager: &str, tool: &str) -> Option<Vec<&'static str>> {
    match (os, manager, tool) {
        ("macos", "brew", "python3") => Some(vec!["brew", "install", "python@3.12"]),
        ("macos", "brew", "docker") => Some(vec!["brew", "install", "--cask", "docker"]),
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

fn ensure_requirement(tool: &str) -> Result<()> {
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
    println!();
    print!("{tool} is missing. Install it with `{shown}`? [y/N] ");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if !accepts_yes(&line) {
        return Err(anyhow!("{tool} is required. Install was declined."));
    }
    let Some(argv) = argv else {
        return Err(anyhow!("no package manager can install {tool}"));
    };
    if host_os() == "linux" && package_manager() == "apt" {
        quiet("sudo", &["apt-get", "update"])?;
    }
    quiet(argv[0], &argv[1..])?;
    if !tool_ok(tool) {
        return Err(anyhow!(
            "{tool} was installed but is not running. Open it, then rerun koma --security-install."
        ));
    }
    Ok(())
}

fn quiet(program: &str, args: &[&str]) -> Result<()> {
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

fn pull_safety_image(dest: &std::path::Path, force: bool) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    if !force {
        if pinned_safety_image().is_some() {
            return Ok(());
        }
    }
    let docker = std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output();
    if !docker.map(|o| o.status.success()).unwrap_or(false) {
        return Err(anyhow!(
            "docker is not installed or not running — security install downloads {SAFETY_IMAGE}:{version} from GHCR"
        ));
    }
    let tag = safety_image_tag();
    quiet("docker", &["pull", "--quiet", &tag])?;
    let inspect = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .RepoDigests}}",
            &tag,
        ])
        .output()
        .context("failed to inspect the pulled image")?;
    if !inspect.status.success() {
        return Err(anyhow!("docker image inspect {tag} failed"));
    }
    let digests: Vec<String> = serde_json::from_slice(&inspect.stdout).unwrap_or_default();
    let reference = if tag.starts_with(SAFETY_IMAGE) {
        digests
            .into_iter()
            .find(|d| d.starts_with(SAFETY_IMAGE) && d.contains("@sha256:"))
            .ok_or_else(|| anyhow!("pull of {tag} did not record a ghcr digest"))?
    } else {
        tag
    };
    let body = format!("{version}\n{reference}\n");
    std::fs::write(image_ref_path(dest), body)
        .with_context(|| format!("write {}", image_ref_path(dest).display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;


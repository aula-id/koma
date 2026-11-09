//! Domain composition inventory for koma-safety guest images.
//!
//! `--security-install` pulls only `minimal`. Domain images are pulled and
//! activated later via the `/security` Images pane (Phase 3) using the APIs
//! here.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::progress::{InstallUi, PullReport};
use super::{image_ref_path, quiet, security_dir, SAFETY_IMAGE};

/// One locally-known composition pull recorded in `images.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageEntry {
    pub tag: String,
    pub digest: String,
    pub size_bytes: u64,
    pub pulled_at: String,
}

/// On-disk inventory at `~/.koma/security/images.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImagesInventory {
    pub version: String,
    pub active: String,
    pub images: BTreeMap<String, ImageEntry>,
}

impl Default for ImagesInventory {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            active: String::new(),
            images: BTreeMap::new(),
        }
    }
}

/// Static catalogue of composition ids the Images pane may offer.
///
/// Blurbs are short UI hints; bake lists live under `src-security/install_recipes/`.
pub fn list_known_compositions() -> &'static [(&'static str, &'static str)] {
    KNOWN_COMPOSITIONS
}

const KNOWN_COMPOSITIONS: &[(&str, &str)] = &[
    ("minimal", "base guest — daemon only"),
    ("recon", "recon (apt + go)"),
    ("recon-2", "recon apt overflow"),
    ("recon-ext", "recon (npm + cargo)"),
    ("recon-src", "recon from source"),
    ("web", "web (apt + go)"),
    ("web-ext", "web (npm + cargo)"),
    ("web-src", "web from source"),
    ("identity", "identity (apt + go)"),
    ("identity-ext", "identity (npm + cargo)"),
    ("identity-src", "identity from source"),
    ("cloud", "cloud (apt + go)"),
    ("cloud-ext", "cloud (npm + cargo)"),
    ("cloud-src", "cloud from source"),
    ("rev", "reverse engineering (apt + go)"),
    ("rev-ext", "reverse engineering (npm + cargo)"),
    ("rev-src", "reverse engineering from source"),
    ("crypto", "crypto (apt + go)"),
    ("crypto-ext", "crypto (npm + cargo)"),
    ("crypto-src", "crypto from source"),
    ("pxn", "pwn / exploit (apt + go)"),
    ("pxn-ext", "pwn / exploit (npm + cargo)"),
    ("pxn-src", "pwn / exploit from source"),
    ("dfir", "DFIR (apt + go)"),
    ("dfir-ext", "DFIR (npm + cargo)"),
    ("dfir-src", "DFIR from source"),
    ("lab", "lab utilities (apt + go)"),
    ("lab-ext", "lab utilities (npm + cargo)"),
    ("lab-src", "lab utilities from source"),
];

fn is_known_composition(id: &str) -> bool {
    KNOWN_COMPOSITIONS.iter().any(|(known, _)| *known == id)
}

fn images_json_path(dest: &Path) -> PathBuf {
    dest.join("images.json")
}

/// GHCR tag for a composition. Always `{version}-{id}` (including `minimal`).
#[must_use]
pub fn safety_image_tag_for(composition: &str) -> String {
    format!("{SAFETY_IMAGE}:{}-{composition}", env!("CARGO_PKG_VERSION"))
}

/// Alias of [`safety_image_tag_for`] for panel call sites.
#[must_use]
pub fn composition_tag(id: &str) -> String {
    safety_image_tag_for(id)
}

/// Read `images.json`, or an empty inventory when missing/unreadable.
#[must_use]
pub fn read_images_inventory() -> ImagesInventory {
    let Ok(dest) = security_dir() else {
        return ImagesInventory::default();
    };
    read_images_inventory_at(&dest)
}

fn read_images_inventory_at(dest: &Path) -> ImagesInventory {
    let path = images_json_path(dest);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return ImagesInventory::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_images_inventory_at(dest: &Path, inv: &ImagesInventory) -> Result<()> {
    let path = images_json_path(dest);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(inv).context("serialize images.json")?;
    std::fs::write(&path, format!("{body}\n"))
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn write_image_ref(dest: &Path, version: &str, digest: &str, composition: &str) -> Result<()> {
    let body = format!("{version}\n{digest}\ncomposition={composition}\n");
    let path = image_ref_path(dest);
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Active guest digest for this crate version.
///
/// Prefers `images.json` (`active` entry digest); falls back to `image-ref` line 2
/// so older installs keep working until the next pull.
#[must_use]
pub fn pinned_from_inventory_or_ref() -> Option<String> {
    let dest = security_dir().ok()?;
    let version = env!("CARGO_PKG_VERSION");
    let inv = read_images_inventory_at(&dest);
    if inv.version == version {
        if let Some(entry) = inv.images.get(&inv.active) {
            if !entry.digest.is_empty() {
                return Some(entry.digest.clone());
            }
        }
    }
    let text = std::fs::read_to_string(image_ref_path(&dest)).ok()?;
    let mut lines = text.lines();
    let file_version = lines.next()?.trim();
    let reference = lines.next()?.trim();
    (file_version == version && !reference.is_empty()).then(|| reference.to_string())
}

/// Active composition id (`minimal`, `recon`, …) for daemon spawn env.
///
/// Prefers `images.json` `active`; falls back to the `composition=` line in
/// `image-ref`. Returns `None` when neither is set.
#[must_use]
pub fn active_composition_id() -> Option<String> {
    let dest = security_dir().ok()?;
    let version = env!("CARGO_PKG_VERSION");
    let inv = read_images_inventory_at(&dest);
    if inv.version == version && !inv.active.is_empty() {
        return Some(inv.active);
    }
    let text = std::fs::read_to_string(image_ref_path(&dest)).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("composition=") {
            let id = rest.trim();
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn utc_iso_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, min, sec) = civil_utc(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Civil UTC date/time from Unix seconds (proleptic Gregorian). No chrono dep.
fn civil_utc(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let sec_of_day = (secs % 86_400) as u32;
    let hour = sec_of_day / 3600;
    let min = (sec_of_day % 3600) / 60;
    let sec = sec_of_day % 60;
    // Days from Unix epoch (1970-01-01) → civil date (Howard Hinnant algorithm).
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32, hour, min, sec)
}

fn ensure_docker_running() -> Result<()> {
    let docker = std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output();
    if docker.map(|o| o.status.success()).unwrap_or(false) {
        return Ok(());
    }
    Err(anyhow!(
        "docker is not installed or not running — security images download from {SAFETY_IMAGE}"
    ))
}

fn inspect_digest_and_size(tag: &str) -> Result<(String, u64)> {
    let digests_out = std::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{json .RepoDigests}}", tag])
        .output()
        .context("failed to inspect the pulled image digests")?;
    if !digests_out.status.success() {
        return Err(anyhow!("docker image inspect {tag} failed"));
    }
    let size_out = std::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Size}}", tag])
        .output()
        .context("failed to inspect the pulled image size")?;
    let size_bytes: u64 = if size_out.status.success() {
        String::from_utf8_lossy(&size_out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    } else {
        0
    };
    let digests: Vec<String> = serde_json::from_slice(&digests_out.stdout).unwrap_or_default();
    let reference = if tag.starts_with(SAFETY_IMAGE) {
        digests
            .into_iter()
            .find(|d| d.starts_with(SAFETY_IMAGE) && d.contains("@sha256:"))
            .ok_or_else(|| anyhow!("pull of {tag} did not record a ghcr digest"))?
    } else {
        // Local override / non-GHCR tag: keep the pulled reference as-is.
        tag.to_string()
    };
    Ok((reference, size_bytes))
}

/// Pull one composition into the local docker store and record it in `images.json`.
///
/// Does not change `active` unless the inventory has no active composition yet, or
/// the pulled id is already active (digest refresh). Use [`activate_composition`]
/// to switch.
pub fn pull_composition(id: &str, force: bool) -> Result<()> {
    pull_composition_inner(id, force, None).map(|_| ())
}

fn pull_composition_inner(id: &str, force: bool, ui: Option<&InstallUi>) -> Result<PullReport> {
    if !is_known_composition(id) {
        return Err(anyhow!("unknown composition '{id}'"));
    }
    let dest = security_dir()?;
    std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;

    let version = env!("CARGO_PKG_VERSION");
    let mut inv = read_images_inventory_at(&dest);
    // Drop stale entries from another agent version.
    if inv.version != version {
        inv = ImagesInventory {
            version: version.to_string(),
            active: String::new(),
            images: BTreeMap::new(),
        };
    }

    if !force {
        if let Some(entry) = inv.images.get(id) {
            if !entry.digest.is_empty() {
                return Ok(PullReport {
                    skipped: true,
                    up_to_date: true,
                    bytes: entry.size_bytes,
                    elapsed: Duration::ZERO,
                });
            }
        }
    }

    ensure_docker_running()?;

    // Install / default path may honour KOMA_SEC_IMAGE only for minimal.
    let tag = if id == "minimal" {
        super::safety_image_tag()
    } else {
        composition_tag(id)
    };

    let (transferred, up_to_date, elapsed) = if let Some(ui) = ui {
        let stats = super::pull::fetch_showing(&tag, ui)?;
        (stats.transferred, stats.up_to_date, stats.elapsed)
    } else {
        let started = Instant::now();
        quiet("docker", &["pull", "--quiet", &tag])?;
        (0, false, started.elapsed())
    };
    let (digest, size_bytes) = inspect_digest_and_size(&tag)?;
    let entry = ImageEntry {
        tag: tag.clone(),
        digest: digest.clone(),
        size_bytes,
        pulled_at: utc_iso_now(),
    };
    inv.version = version.to_string();
    inv.images.insert(id.to_string(), entry);

    let become_active = inv.active.is_empty() || inv.active == id;
    if inv.active.is_empty() {
        inv.active = id.to_string();
    }
    write_images_inventory_at(&dest, &inv)?;
    if become_active {
        write_image_ref(&dest, version, &digest, id)?;
    }
    let bytes = if transferred > 0 {
        transferred
    } else {
        size_bytes
    };
    Ok(PullReport {
        skipped: false,
        up_to_date,
        bytes,
        elapsed,
    })
}

/// Make a previously pulled composition the active guest image.
pub fn activate_composition(id: &str) -> Result<()> {
    if !is_known_composition(id) {
        return Err(anyhow!("unknown composition '{id}'"));
    }
    let dest = security_dir()?;
    let version = env!("CARGO_PKG_VERSION");
    let mut inv = read_images_inventory_at(&dest);
    if inv.version != version {
        return Err(anyhow!(
            "composition '{id}' is not local for this koma version (inventory version {})",
            inv.version
        ));
    }
    let entry = inv
        .images
        .get(id)
        .ok_or_else(|| anyhow!("composition '{id}' is not local — pull it first"))?
        .clone();
    if entry.digest.is_empty() {
        return Err(anyhow!("composition '{id}' is not local — pull it first"));
    }
    inv.active = id.to_string();
    write_images_inventory_at(&dest, &inv)?;
    write_image_ref(&dest, version, &entry.digest, id)?;
    Ok(())
}

/// Drop a composition from the inventory and best-effort `docker rmi`.
///
/// Refuses to remove the active composition.
pub fn remove_composition(id: &str) -> Result<()> {
    if !is_known_composition(id) {
        return Err(anyhow!("unknown composition '{id}'"));
    }
    let dest = security_dir()?;
    let mut inv = read_images_inventory_at(&dest);
    if inv.active == id {
        return Err(anyhow!(
            "cannot remove active composition '{id}' — activate another first"
        ));
    }
    let Some(entry) = inv.images.remove(id) else {
        return Err(anyhow!("composition '{id}' is not in the local inventory"));
    };
    write_images_inventory_at(&dest, &inv)?;
    // Best-effort local cleanup; inventory already dropped the entry.
    if !entry.digest.is_empty() {
        let _ = std::process::Command::new("docker")
            .args(["rmi", "--force", &entry.digest])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    } else if !entry.tag.is_empty() {
        let _ = std::process::Command::new("docker")
            .args(["rmi", "--force", &entry.tag])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
}

/// Install path: pull `minimal` (no-op when already local unless `force`) and
/// **always** activate it — even when another composition or pin already exists.
///
/// `ui` draws the download bar. The Images pane calls [`pull_composition`], which
/// stays quiet so it does not write over the TUI.
pub(super) fn pull_safety_image(_dest: &Path, force: bool, ui: &InstallUi) -> Result<PullReport> {
    let report = pull_composition_inner("minimal", force, Some(ui))?;
    activate_composition("minimal")?;
    Ok(report)
}

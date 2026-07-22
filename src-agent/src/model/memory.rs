//! Per-PROJECT long-term memory: an index of pointers plus one file per memory.
//!
//! Memory lives in the per-project bucket (`<pwd_bucket_dir>/memory/`, see
//! [`crate::model::store::memory_dir`]), so EVERY session opened from the same
//! working directory shares ONE store. The layout follows the index-of-pointers
//! pattern:
//!
//! ```text
//! <pwd_bucket_dir>/memory/
//!     MEMORY.md        ← a lightweight INDEX: one bullet per memory
//!     <slug>.md        ← the actual memory (frontmatter + body)
//! ```
//!
//! Only the INDEX (`MEMORY.md`) is injected into the system prompt — never the
//! full bodies — so the prompt stays lean as memory grows. The model pulls a
//! body on demand with the `recall(<slug>)` tool.
//!
//! Each `<slug>.md` is:
//! ```text
//! ---
//! name: <slug>
//! description: <one-line hook>
//! type: project | preference | reference | fact
//! ---
//!
//! <the memory body>
//! ```
//!
//! `<slug>` becomes a filename, so it is hard-sanitized by [`slugify`] (lowercase
//! `[a-z0-9-]` only, no `..` / `/` / leading dot, length-capped) and every path is
//! resolved UNDER the memory dir — a slug can never escape it (path traversal).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The index file name inside the memory directory.
const INDEX_FILE: &str = "MEMORY.md";

/// Maximum slug length (filenames stay sane; long descriptions still slug fine).
const MAX_SLUG_LEN: usize = 80;

/// Atomically write `bytes` to `path` by writing to a PID-suffixed temp file
/// first, then renaming over the target. This prevents concurrent readers from
/// seeing a half-written file. The rename is atomic on POSIX (and NTFS) so
/// another process either sees the old content or the new one, never a partial.
///
/// `pub(crate)` so the `plan_ready` interception (which persists the presented
/// plan to `<session>/plan.md`) can reuse the same primitive.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
    })?;
    let mut tmp_name = file_name.to_owned();
    tmp_name.push(format!(".{}$", std::process::id()));
    let tmp_path = parent.join(&tmp_name);
    std::fs::write(&tmp_path, bytes)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Return the mtime of `MEMORY.md` inside `dir`, or `None` if it doesn't exist
/// or can't be stat'd. Used by the cross-instance memory sync poll.
pub fn memory_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(INDEX_FILE))
        .ok()
        .and_then(|m| m.modified().ok())
}

/// Read `AGENT.md` (preferred) or `AGENTS.md` from `workdir`, returning trimmed
/// contents. `None` if neither exists or both are blank. These are project-level
/// instructions injected into the system prompt.
pub fn load_agents(workdir: &Path) -> Option<String> {
    for name in ["AGENT.md", "AGENTS.md"] {
        if let Ok(s) = std::fs::read_to_string(workdir.join(name)) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Hard-sanitize an arbitrary string into a safe slug usable as a FILENAME.
///
/// Security-critical: a slug is joined onto the memory dir to form a path, so it
/// must NEVER be able to escape that directory. The algorithm:
/// 1. Lowercase every character.
/// 2. Keep only ASCII `[a-z0-9]`; map every other character (including `.`, `/`,
///    `\`, whitespace) to a single `-` separator.
/// 3. Collapse consecutive `-` and trim leading/trailing `-`.
/// 4. Cap the length to [`MAX_SLUG_LEN`] (then re-trim a trailing `-`).
///
/// This neutralizes `..`, `/`, `\`, and leading dots by construction (those
/// characters never survive step 2), so the result is always a single bare path
/// segment. Returns `None` when nothing usable remains (e.g. all punctuation).
pub fn slugify(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.chars() {
        // Lowercase first; ASCII alnum is kept, everything else becomes a dash.
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            out.push(lc);
            prev_dash = false;
        } else if !prev_dash {
            // Collapse runs of separators into a single dash.
            out.push('-');
            prev_dash = true;
        }
    }
    // Trim leading/trailing dashes that the collapse may have left.
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        return None;
    }
    // Cap length, then re-trim a dash that the cut may have exposed.
    let capped = if trimmed.len() > MAX_SLUG_LEN {
        trimmed[..MAX_SLUG_LEN].trim_end_matches('-')
    } else {
        trimmed
    };
    if capped.is_empty() {
        None
    } else {
        Some(capped.to_string())
    }
}

/// Resolve `<dir>/<slug>.md` for an ALREADY-sanitized slug, enforcing that the
/// result stays directly inside `dir`. Returns `None` if the slug doesn't
/// sanitize to itself (defence in depth) or the join would escape `dir`.
fn slug_path(dir: &Path, slug: &str) -> Option<PathBuf> {
    // The slug must be a clean single segment: re-slugify and require a fixpoint.
    let clean = slugify(slug)?;
    let path = dir.join(format!("{clean}.md"));
    // Containment: the file's parent must be exactly `dir` (no traversal).
    match path.parent() {
        Some(p) if p == dir => Some(path),
        _ => None,
    }
}

/// A parsed memory file: its frontmatter fields plus the body text.
///
/// `slug` + `description` drive the injected index; `body` is returned by the
/// `recall` tool. `kind` is parsed for completeness but has no reader yet, so it
/// stays `allow`ed.
pub struct Memory {
    pub slug: String,
    pub description: String,
    #[allow(dead_code)]
    pub kind: String,
    pub body: String,
}

/// Default `type` value when a memory is written without one.
const DEFAULT_TYPE: &str = "fact";

/// Render a `<slug>.md` file's full text from its parts (frontmatter + body).
fn render_memory_file(slug: &str, description: &str, kind: &str, body: &str) -> String {
    // Frontmatter values are single-line; collapse newlines in the description so
    // the YAML-ish header can't be broken by a multi-line hook.
    let desc_line = description.replace(['\n', '\r'], " ");
    let kind = if kind.trim().is_empty() {
        DEFAULT_TYPE
    } else {
        kind.trim()
    };
    format!(
        "---\nname: {slug}\ndescription: {desc}\ntype: {kind}\n---\n\n{body}\n",
        slug = slug,
        desc = desc_line.trim(),
        kind = kind,
        body = body.trim(),
    )
}

/// Parse a `<slug>.md` file's text into its frontmatter fields + body.
///
/// Tolerant: a file WITHOUT a leading `---` frontmatter block is treated as a
/// bare body (description/type fall back to the slug / [`DEFAULT_TYPE`]), so a
/// hand-written or migrated file still loads.
fn parse_memory_file(slug: &str, text: &str) -> Memory {
    let mut description = String::new();
    let mut kind = String::new();
    let body;

    if let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        // Find the closing fence: a line that is exactly `---`.
        if let Some(end) = find_frontmatter_end(rest) {
            let (front, after) = rest.split_at(end.0);
            for line in front.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("type:") {
                    kind = v.trim().to_string();
                }
                // `name:` is informational; the slug comes from the filename.
            }
            body = after[end.1..].trim_start_matches(['\n', '\r']).to_string();
        } else {
            // Unterminated frontmatter — treat the whole thing as body.
            body = text.to_string();
        }
    } else {
        body = text.to_string();
    }

    Memory {
        slug: slug.to_string(),
        description: if description.is_empty() {
            slug.to_string()
        } else {
            description
        },
        kind: if kind.is_empty() {
            DEFAULT_TYPE.to_string()
        } else {
            kind
        },
        body: body.trim().to_string(),
    }
}

/// Locate the closing `---` fence in the post-opening-fence text.
///
/// Returns `(offset_to_fence_line, fence_line_len)` so the caller can split the
/// frontmatter from the body. `None` if no closing fence exists.
fn find_frontmatter_end(rest: &str) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            return Some((offset, line.len()));
        }
        offset += line.len();
    }
    None
}

/// Read a single memory's BODY by slug, or `None` if it doesn't exist / can't be
/// read. The slug is sanitized and resolved under `dir` (no traversal).
///
/// Consumed by the `recall` tool.
pub fn read_memory(dir: &Path, slug: &str) -> Option<String> {
    let path = slug_path(dir, slug)?;
    let text = std::fs::read_to_string(&path).ok()?;
    let parsed = parse_memory_file(&slugify(slug)?, &text);
    Some(parsed.body)
}

/// Write (create or overwrite) a memory file `<slug>.md` and refresh the index.
///
/// `slug` is sanitized; the cleaned slug is returned so the caller knows the
/// canonical id. Updating an existing slug overwrites it in place (the edit/swap
/// case). Returns an error if the slug doesn't sanitize to anything usable or
/// the path resolves outside `dir`.
pub fn write_memory(
    dir: &Path,
    slug: &str,
    description: &str,
    kind: &str,
    body: &str,
) -> std::io::Result<String> {
    let clean = slugify(slug).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "slug has no usable characters",
        )
    })?;
    let path = slug_path(dir, &clean).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "slug resolves outside memory dir",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let text = render_memory_file(&clean, description, kind, body);
    atomic_write(&path, text.as_bytes())?;
    rebuild_index(dir)?;
    Ok(clean)
}

/// Remove a memory file `<slug>.md` and refresh the index. Idempotent: a missing
/// file is not an error. Returns `true` if a file was actually deleted.
///
/// Consumed by the `forget` tool.
pub fn remove_memory(dir: &Path, slug: &str) -> std::io::Result<bool> {
    let path = match slug_path(dir, slug) {
        Some(p) => p,
        None => return Ok(false),
    };
    let existed = path.exists();
    if existed {
        std::fs::remove_file(&path)?;
    }
    rebuild_index(dir)?;
    Ok(existed)
}

/// List every memory in `dir` (parsed), sorted by slug for a stable index.
///
/// Scans `<dir>/*.md` EXCLUDING the index file itself. Unreadable / non-`.md`
/// entries are skipped silently so a stray file never breaks the listing.
pub fn list_memories(dir: &Path) -> Vec<Memory> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let fname = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if fname == INDEX_FILE {
            continue;
        }
        let slug = match fname.strip_suffix(".md") {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        // Skip anything whose name isn't a clean slug (defence in depth).
        let clean = match slugify(slug) {
            Some(c) if c == slug => c,
            _ => continue,
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            out.push(parse_memory_file(&clean, &text));
        }
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    out
}

/// Rebuild `MEMORY.md` (the index) from the current set of `<slug>.md` files.
///
/// Each memory contributes one markdown bullet: `- [<description>](<slug>.md)`.
/// An empty store writes an empty index file (so a later read is a clean `None`).
fn rebuild_index(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let memories = list_memories(dir);
    let mut s = String::new();
    for m in &memories {
        s.push_str(&format!("- [{}]({}.md)\n", m.description, m.slug));
    }
    atomic_write(&dir.join(INDEX_FILE), s.as_bytes())
}

/// Read the memory INDEX (`MEMORY.md`) text to inject into the system prompt.
///
/// Returns `None` when the index is missing or blank after trimming — the caller
/// (`Session::rebuild_system` → `build_system_prompt`) treats `None` as "no
/// memory" and omits the `# Memory` section entirely. Only the index bullets are
/// returned (pointers), never the full bodies.
pub fn load_memory_index(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(INDEX_FILE)).ok()?;
    let t = text.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Best-effort, idempotent, NON-destructive migration of the legacy flat
/// per-session memory file into the per-project index store.
///
/// The old layout stored a single flat bullet list at
/// `<session_dir>/memory/MEMORY.md`. If the per-PROJECT memory dir has NO index
/// yet (or an empty one) AND the current session has that legacy file, each
/// non-empty bullet line becomes a `<slug>.md` (auto-slug from the line text,
/// description + body = the line) and the index is built.
///
/// Rules:
/// - If the per-project index already has content, do NOTHING (idempotent).
/// - The legacy file is NEVER deleted.
/// - Fail-open: any error is swallowed so a migration glitch can never block a
///   turn (the worst case is "memory looks empty this once").
pub fn migrate_legacy_memory(project_dir: &Path, session_dir: &Path) {
    // Already migrated / non-empty index → leave it alone.
    if load_memory_index(project_dir).is_some() {
        return;
    }
    let legacy = session_dir.join("memory").join(INDEX_FILE);
    // Don't migrate the project's own index onto itself (same path).
    if legacy == project_dir.join(INDEX_FILE) {
        return;
    }
    let text = match std::fs::read_to_string(&legacy) {
        Ok(t) => t,
        Err(_) => return,
    };

    let mut migrated_any = false;
    for raw in text.lines() {
        // Strip a leading markdown bullet marker, then trim.
        let line = raw.trim_start().trim_start_matches(['-', '*', '+']).trim();
        if line.is_empty() {
            continue;
        }
        // Skip a stray heading line if one slipped into the flat file.
        if line.starts_with('#') {
            continue;
        }
        let slug = match slugify(line) {
            Some(s) => s,
            None => continue,
        };
        // write_memory rebuilds the index after each write; that's fine (the
        // legacy lists are tiny). Fail-open per line.
        if write_memory(project_dir, &slug, line, DEFAULT_TYPE, line).is_ok() {
            migrated_any = true;
        }
    }

    // If the legacy file had lines but none migrated (all punctuation, etc.),
    // still ensure an index file exists so we don't retry forever.
    if !migrated_any {
        let _ = rebuild_index(project_dir);
    }
}

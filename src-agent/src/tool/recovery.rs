//! Bounded evidence and per-turn suppression for failed filesystem inspections.
use super::{output::fingerprint, ToolCtx};
use anyhow::Result;
use serde_json::Value;
use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
};

pub const MARKER: &str = "[filesystem recovery]";

#[derive(Default)]
pub(crate) struct Inspections {
    /// (state key, target, report) for each suppressed failure.
    failures: VecDeque<(String, PathBuf, String)>,
}

impl super::CallTrack {
    pub fn invalidate_inspections(&self) {
        self.inspections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .failures
            .clear();
    }
}

#[derive(Debug, Clone, Copy)]
enum Failure {
    Missing,
    NotDirectory,
    NotFile,
    AccessDenied,
    Shell,
}

impl Failure {
    fn from_error(error: &anyhow::Error) -> Option<Self> {
        if error.downcast_ref::<ShellFailure>().is_some() {
            return Some(Self::Shell);
        }
        match error.downcast_ref::<io::Error>()?.kind() {
            io::ErrorKind::NotFound => Some(Self::Missing),
            io::ErrorKind::NotADirectory => Some(Self::NotDirectory),
            io::ErrorKind::IsADirectory => Some(Self::NotFile),
            io::ErrorKind::PermissionDenied => Some(Self::AccessDenied),
            _ => None,
        }
    }
    fn finding(self) -> &'static str {
        match self {
            Self::Shell => "shell command failed; native CWD evidence follows separately",
            Self::Missing => "target does not exist",
            Self::NotDirectory => "target or a path component is not a directory",
            Self::NotFile => "target is a directory, not a file",
            Self::AccessDenied => "access denied; permissions remain unchanged",
        }
    }
}

pub(crate) fn require_file(path: &Path) -> Result<()> {
    if std::fs::metadata(path)?.is_dir() {
        return Err(io::Error::from(io::ErrorKind::IsADirectory).into());
    }
    Ok(())
}

pub(crate) fn check_search_target(path: &Path) -> Result<()> {
    if std::fs::metadata(path)?.is_dir() {
        std::fs::read_dir(path)?;
    } else {
        std::fs::File::open(path)?;
    }
    Ok(())
}

// Stat only the target and its ancestors. This notices a newly created missing
// component without walking siblings or depending on an incomplete index.
fn state_key(ctx: &ToolCtx, name: &str, args: &Value, target: &Path, root: &Path) -> String {
    let mut normalized = args.clone();
    if let Some(map) = normalized.as_object_mut() {
        map.insert(
            "path".into(),
            Value::String(target.to_string_lossy().into_owned()),
        );
    }
    let version = ctx.dir_cache.read().map(|c| c.version).unwrap_or_default();
    let mut key = format!(
        "{}:{:?}:{:?}:{version}",
        fingerprint(name, &normalized),
        ctx.workspace,
        ctx.workspaces
    );
    for path in target.ancestors().take_while(|p| p.starts_with(root)) {
        match std::fs::metadata(path) {
            Ok(m) => key.push_str(&format!(
                ":{path:?}:{:?}:{}:{}:{:?}",
                m.modified(),
                m.len(),
                m.is_dir(),
                m.permissions()
            )),
            Err(e) => key.push_str(&format!(":{path:?}:{:?}", e.kind())),
        }
    }
    key
}

pub(crate) fn inspect(
    ctx: &ToolCtx,
    name: &str,
    args: &Value,
    requested: &str,
    target: &Path,
    run: impl FnOnce() -> Result<String>,
) -> Result<String> {
    let Some(root) = super::find_workspace(&ctx.workspaces, target) else {
        // Read exemptions (skill/session/scratch) retain their existing access,
        // but cannot be used to inspect neighboring files through recovery.
        return run();
    };
    // Held through execution and diagnosis so concurrent identical calls share
    // one failed operation and one recovery. Never held across an await.
    let mut track = ctx
        .call_track
        .inspections
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let key = state_key(ctx, name, args, target, &root);
    if let Some((_, _, prior)) = track.failures.iter().find(|(k, _, _)| k == &key) {
        return Ok(format!("{MARKER} No new inspection ran: unchanged failed inspection suppressed.\n{prior}\nUse a different approach: glob/grep for discovery, dir_list for a verified directory, or ask the user for the intended path. Continue the turn."));
    }
    match run() {
        // The target was there, so any earlier failure on it is stale. The
        // state key alone can miss this: a file created and removed within one
        // coarse mtime tick leaves its directory's mtime and size unchanged.
        Ok(output) => {
            track.failures.retain(|(_, t, _)| t != target);
            Ok(output)
        }
        Err(error) => {
            let Some(kind) = Failure::from_error(&error) else {
                return Err(error);
            };
            let report = diagnose(ctx, requested, target, &root, kind);
            let output = format!("error: {error:#}\n{report}");
            if track.failures.len() == 128 {
                track.failures.pop_front();
            }
            track
                .failures
                .push_back((key, target.to_path_buf(), output.clone()));
            Ok(output)
        }
    }
}

fn diagnose(ctx: &ToolCtx, requested: &str, target: &Path, root: &Path, kind: Failure) -> String {
    let mut out = format!(
        "{MARKER}\nRequested path: {requested}\nResolved path: {}\nVerified finding: {}.\n",
        target.display(),
        kind.finding()
    );
    if matches!(kind, Failure::AccessDenied) {
        out.push_str("Next action: ask for access or use another authorized path; do not reinterpret the denial.");
        return out;
    }
    for ancestor in target.ancestors().take_while(|p| p.starts_with(root)) {
        match std::fs::metadata(ancestor) {
            Ok(m) if m.is_dir() => {
                out.push_str(&format!(
                    "Native directory inspection (separate from the failed operation): {}\n",
                    ancestor.display()
                ));
                match visible_entries(ancestor, 30) {
                    Ok((entries, truncated, excluded)) => {
                        for entry in &entries {
                            out.push_str(&format!("  {entry}\n"));
                        }
                        if entries.is_empty() {
                            out.push_str(if excluded {
                                "  (entries excluded by visibility rules)\n"
                            } else {
                                "  (empty directory)\n"
                            });
                        }
                        if truncated {
                            out.push_str("  ... (at most 30 visible entries shown)\n");
                        }
                    }
                    Err(e) => out.push_str(&format!("  inspection unavailable: {e}\n")),
                }
                let label = super::model_display_path(&ctx.workspaces, ancestor);
                let label = if label.is_empty() { "." } else { &label };
                out.push_str(&format!("Next action: use dir_list on {} or glob/grep to discover the intended file; do not guess or retry the unchanged path.", label));
                return out;
            }
            Ok(_) => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                continue
            }
            Err(e) => {
                out.push_str(&format!("Ancestor inspection unavailable: {e}. Next action: ask for an authorized path."));
                return out;
            }
        }
    }
    out.push_str("Next action: the workspace has no inspectable ancestor within its authorized boundary; ask the user to restore or select the workspace.");
    out
}

pub(crate) fn walk_error(error: ignore::Error) -> anyhow::Error {
    if let Some(io) = error.io_error() {
        io::Error::new(io.kind(), error.to_string()).into()
    } else {
        error.into()
    }
}

/// One level, with the same ignore and generated-directory rules as the index.
/// The raw read distinguishes empty from hidden; it never follows children.
pub(crate) fn visible_entries(path: &Path, cap: usize) -> Result<(Vec<String>, bool, bool)> {
    let mut raw = std::fs::read_dir(path)?;
    let has_entries = raw.next().transpose()?.is_some();
    let walker = ignore::WalkBuilder::new(path)
        .max_depth(Some(1))
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_some_and(|t| t.is_dir())
                || !super::dircache::PRUNE_DIRS.contains(&entry.file_name().to_str().unwrap_or(""))
        })
        .build();
    let mut entries = Vec::new();
    let mut truncated = false;
    for entry in walker {
        let entry = entry.map_err(walk_error)?;
        if entry.depth() == 0 {
            continue;
        }
        if entries.len() == cap {
            truncated = true;
            break;
        }
        let suffix = if entry.file_type().is_some_and(|t| t.is_dir()) {
            "/"
        } else {
            ""
        };
        entries.push(format!("{}{suffix}", entry.file_name().to_string_lossy()));
    }
    entries.sort();
    Ok((entries, truncated, has_entries))
}

pub(crate) fn listing_command(args: &Value) -> bool {
    !args
        .get("run_in_background")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && matches!(
            args.get("command").and_then(Value::as_str),
            Some("ls" | "ls -l" | "ls -a" | "ls -la" | "ls -al" | "dir")
        )
}

pub(crate) fn shell_listing(
    ctx: &ToolCtx,
    args: &Value,
    run: impl FnOnce() -> (String, bool),
) -> Result<String> {
    let target = super::partial_canonicalize(&ctx.workspace);
    if super::find_workspace(&ctx.workspaces, &target).is_none() {
        // A user /cd can authorize shell execution outside structured roots.
        // Preserve that execution without adding native filesystem access.
        return Ok(run().0);
    }
    inspect(
        ctx,
        "bash",
        args,
        ". (authorized current directory)",
        &target,
        || {
            let (output, failed) = run();
            if !failed {
                return Ok(output);
            }
            // The shell failed, not necessarily the filesystem. Preserve all shell
            // evidence and diagnose the CWD separately without inventing an IO cause.
            Err(ShellFailure(output).into())
        },
    )
}

#[derive(Debug)]
struct ShellFailure(String);
impl std::fmt::Display for ShellFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ShellFailure {}

#[cfg(test)]
#[path = "recovery_test.rs"]
mod tests;

//! Pure command resolution for stdio MCP servers (PATH + PATHEXT), so the
//! Windows rules unit-test on any OS. `windows` selects the rule set.

const DEFAULT_EXTS: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

pub(super) const STDERR_TAIL_BYTES: usize = 2048;

/// Extensions Windows can run (directly or via cmd.exe), in PATHEXT order.
/// Anything else in PATHEXT (.VBS, .JS, .PS1, ...) is ignored.
pub(super) fn spawnable_exts(pathext: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in pathext.unwrap_or("").split(';') {
        let ext = raw.trim().to_ascii_lowercase();
        if DEFAULT_EXTS.contains(&ext.as_str()) && !out.contains(&ext) {
            out.push(ext);
        }
    }
    if out.is_empty() {
        out = DEFAULT_EXTS.iter().map(|e| e.to_string()).collect();
    }
    out
}

/// PATH the child should see: the user's own PATH entry from `env` when present
/// (key compared case-insensitively on Windows, exactly "PATH" on unix), else `host_path`.
pub(super) fn effective_path(env: &[(String, String)], host_path: &str, windows: bool) -> String {
    env.iter()
        .rev()
        .find(|(k, _)| {
            if windows {
                k.eq_ignore_ascii_case("PATH")
            } else {
                k == "PATH"
            }
        })
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| host_path.to_string())
}

fn is_sep(c: char, windows: bool) -> bool {
    c == '/' || (windows && c == '\\')
}

fn join(dir: &str, name: &str, windows: bool) -> String {
    if dir.ends_with('/') || dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        let sep = if windows { '\\' } else { '/' };
        format!("{dir}{sep}{name}")
    }
}

/// True when `cmd` names a location rather than a bare program name.
pub(super) fn is_path_like(cmd: &str, windows: bool) -> bool {
    cmd.chars().any(|c| is_sep(c, windows)) || (windows && cmd.chars().nth(1) == Some(':'))
}

fn is_absolute(cmd: &str, windows: bool) -> bool {
    if windows {
        cmd.chars().nth(1) == Some(':') || cmd.starts_with('\\') || cmd.starts_with('/')
    } else {
        cmd.starts_with('/')
    }
}

fn has_ext(cmd: &str, windows: bool) -> bool {
    cmd.rsplit(|c| is_sep(c, windows))
        .next()
        .is_some_and(|last| last.contains('.'))
}

/// Candidate files for `cmd`, in search order. String-based (not `Path::join`)
/// so a Windows path tests correctly on any OS.
pub(super) fn candidates(
    cmd: &str,
    path_var: &str,
    pathext: Option<&str>,
    cwd: Option<&str>,
    windows: bool,
) -> Vec<String> {
    let exts = if windows {
        spawnable_exts(pathext)
    } else {
        Vec::new()
    };
    let with_exts = |base: &str, include_exact: bool| -> Vec<String> {
        let mut out = Vec::new();
        if include_exact {
            out.push(base.to_string());
        }
        out.extend(exts.iter().map(|e| format!("{base}{e}")));
        out
    };
    if is_path_like(cmd, windows) {
        let base = match (is_absolute(cmd, windows), cwd) {
            (false, Some(c)) => join(c, cmd, windows),
            _ => cmd.to_string(),
        };
        if !windows {
            return vec![base];
        }
        return with_exts(&base, has_ext(cmd, windows));
    }
    let list_sep = if windows { ';' } else { ':' };
    let mut out = Vec::new();
    for raw in path_var.split(list_sep) {
        let dir = raw.trim();
        let dir = dir
            .strip_prefix('"')
            .and_then(|d| d.strip_suffix('"'))
            .unwrap_or(dir);
        if dir.is_empty() {
            continue;
        }
        if !windows {
            out.push(join(dir, cmd, windows));
            continue;
        }
        out.extend(with_exts(&join(dir, cmd, windows), has_ext(cmd, windows)));
    }
    out
}

/// First candidate for which `is_file` returns true.
pub(super) fn resolve_with(
    cmd: &str,
    path_var: &str,
    pathext: Option<&str>,
    cwd: Option<&str>,
    windows: bool,
    is_file: &dyn Fn(&str) -> bool,
) -> Option<String> {
    candidates(cmd, path_var, pathext, cwd, windows)
        .into_iter()
        .find(|c| is_file(c))
}

/// The program handed to `Command::new`. Windows always uses the resolved path
/// (PATHEXT). Unix keeps a bare name so std looks it up on the child's PATH, and
/// uses the resolved path when the command contains a separator. `None` means
/// the command cannot be launched (Windows, nothing resolved).
pub(super) fn choose_program(cmd: &str, resolved: Option<&str>, windows: bool) -> Option<String> {
    if windows {
        return resolved.map(str::to_string);
    }
    if is_path_like(cmd, false) {
        return Some(resolved.unwrap_or(cmd).to_string());
    }
    Some(cmd.to_string())
}

/// Message for a command that resolved to nothing.
pub(super) fn not_found_message(
    cmd: &str,
    path_var: &str,
    pathext: Option<&str>,
    windows: bool,
) -> String {
    let list_sep = if windows { ';' } else { ':' };
    let dirs = path_var
        .split(list_sep)
        .filter(|d| !d.trim().is_empty())
        .count();
    let tried = if windows {
        format!(", tried {}", spawnable_exts(pathext).join(", "))
    } else {
        String::new()
    };
    format!(
        "command '{cmd}' was not found on PATH ({dirs} directories searched{tried}). \
         Install it, or use its absolute path as the command."
    )
}

/// Human text for a failed spawn of `program` (the resolved path).
pub(super) fn describe_spawn_error(
    cmd: &str,
    program: &str,
    kind: std::io::ErrorKind,
    err_text: &str,
    windows: bool,
) -> String {
    let lower = program.to_ascii_lowercase();
    match kind {
        std::io::ErrorKind::NotFound => format!("spawn '{program}' failed: file not found"),
        std::io::ErrorKind::InvalidInput
            if windows && (lower.ends_with(".cmd") || lower.ends_with(".bat")) =>
        {
            format!(
                "an argument cannot be passed safely to the batch file '{program}' \
                 (characters such as % \" or newlines). Use the real executable instead, \
                 e.g. node.exe with the script path."
            )
        }
        std::io::ErrorKind::PermissionDenied => {
            format!("'{program}' is not executable (permission denied)")
        }
        _ => format!("spawn '{cmd}' failed: {err_text}"),
    }
}

/// Keep the last `cap` bytes of a child's stderr, trimmed to whole lines where
/// possible and always on a char boundary.
pub(super) fn push_tail(buf: &mut String, line: &str, cap: usize) {
    buf.push_str(line);
    buf.push('\n');
    while buf.len() > cap {
        let cut = match buf.find('\n') {
            Some(i) if i + 1 < buf.len() => i + 1,
            _ => buf.chars().next().map_or(buf.len(), char::len_utf8),
        };
        buf.drain(..cut);
    }
}

/// Append the child's stderr tail to a connect error.
pub(super) fn with_stderr(msg: String, tail: &str) -> String {
    if tail.trim().is_empty() {
        msg
    } else {
        format!("{msg}\nserver stderr (last lines):\n{}", tail.trim_end())
    }
}

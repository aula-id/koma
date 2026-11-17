//! Pure Windows path and shell-name rules. Every function takes plain strings so
//! the rules are unit-tested on any host; only [`display_path`] and
//! [`canonical`] touch the filesystem.
#![cfg_attr(not(windows), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Longest plain path (in chars) that legacy Win32 APIs accept without the verbatim prefix.
const MAX_PLAIN_PATH: usize = 259;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum ShellKind {
    Cmd,
    WindowsPowerShell,
    Pwsh,
    Bash,
    WslBash,
    Other,
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

fn is_drive(seg: &str) -> bool {
    matches!(seg.as_bytes(), [d, b':'] if d.is_ascii_alphabetic())
}

/// Drop the verbatim prefix: `\\?\C:\x` -> `C:\x`, `\\?\UNC\s\sh` -> `\\s\sh`,
/// `//?/C:/x` -> `C:/x`. Anything else (including `\\?\Volume{..}`) is unchanged.
pub(crate) fn strip_verbatim(p: &str) -> String {
    let [s0, s1, b'?', s3, ..] = p.as_bytes() else {
        return p.to_string();
    };
    let sep = |b: &u8| *b == b'/' || *b == b'\\';
    let (Some(rest), true) = (p.get(4..), sep(s0) && sep(s1) && sep(s3)) else {
        return p.to_string();
    };
    if let (Some(word), Some(tail)) = (rest.get(..3), rest.get(3..)) {
        if word.eq_ignore_ascii_case("UNC") && tail.starts_with(is_sep) {
            let c = char::from(*s0);
            return format!("{c}{c}{}", tail.trim_start_matches(is_sep));
        }
    }
    let drive_path = rest
        .get(..2)
        .zip(rest.get(2..))
        .is_some_and(|(d, t)| is_drive(d) && (t.is_empty() || t.starts_with(is_sep)));
    if drive_path {
        rest.to_string()
    } else {
        p.to_string()
    }
}

/// One spelling for a Windows path: no verbatim prefix, `\` separators,
/// collapsed duplicates (a leading UNC pair is kept), upper-case drive letter,
/// no trailing separator except for a bare drive root (`C:\`).
pub(crate) fn win_normalize(p: &str) -> String {
    let s = strip_verbatim(p).replace('/', "\\");
    let (lead, rest) = if let Some(r) = s.strip_prefix("\\\\") {
        ("\\\\", r)
    } else if let Some(r) = s.strip_prefix('\\') {
        ("\\", r)
    } else {
        ("", s.as_str())
    };
    let mut segs: Vec<String> = rest
        .split('\\')
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect();
    if lead.is_empty() {
        if let Some(first) = segs.first_mut() {
            if is_drive(first) {
                first.make_ascii_uppercase();
            }
        }
    }
    let drive_root =
        lead.is_empty() && rest.ends_with('\\') && matches!(segs.as_slice(), [d] if is_drive(d));
    let mut out = format!("{lead}{}", segs.join("\\"));
    if drive_root {
        out.push('\\');
    }
    out
}

/// Case-folded [`win_normalize`], for equality and dedupe.
pub(crate) fn win_key(p: &str) -> String {
    win_normalize(p).to_lowercase()
}

pub(crate) fn win_join(root: &str, rel: &str) -> String {
    win_normalize(&format!("{root}\\{rel}"))
}

/// Git for Windows root from `git --exec-path` output
/// (`C:/Program Files/Git/mingw64/libexec/git-core`).
pub(crate) fn git_root_from_exec_path(out: &str) -> Option<String> {
    let norm = win_normalize(out.trim());
    let comps: Vec<&str> = norm.split('\\').filter(|c| !c.is_empty()).collect();
    let keep = comps.len().checked_sub(3)?;
    if keep < 2 {
        return None;
    }
    Some(comps.get(..keep)?.join("\\"))
}

/// Where `bash.exe` lives under a Git for Windows root, preferred first.
pub(crate) fn git_bash_candidates(root: &str) -> [String; 2] {
    [
        win_join(root, "bin\\bash.exe"),
        win_join(root, "usr\\bin\\bash.exe"),
    ]
}

/// Value of the first `REG_SZ` line of `reg query` output.
pub(crate) fn parse_reg_sz(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (head, tail) = line.split_once("REG_SZ")?;
        if !head.ends_with(char::is_whitespace) || !tail.starts_with(char::is_whitespace) {
            return None;
        }
        let v = tail.trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// Git-bash style `/c/Users/x` -> `C:\Users\x`. `/tmp/x`, `//srv/x` and plain
/// Windows paths are not drive mounts and give `None`.
pub(crate) fn msys_to_win(p: &str) -> Option<String> {
    let r = p.strip_prefix('/')?;
    let mut ch = r.chars();
    let d = ch.next().filter(char::is_ascii_alphabetic)?;
    let tail = ch.as_str();
    if !(tail.is_empty() || tail.starts_with('/')) {
        return None;
    }
    let tail = tail.trim_start_matches('/').replace('/', "\\");
    Some(format!("{}:\\{tail}", d.to_ascii_uppercase()))
}

/// Lower-cased file stem of a shell path, either separator style.
pub(crate) fn shell_stem(path: &str) -> String {
    let name = path.rsplit(is_sep).next().unwrap_or(path).to_lowercase();
    name.strip_suffix(".exe")
        .map(str::to_string)
        .unwrap_or(name)
}

pub(crate) fn shell_kind(path: &str) -> ShellKind {
    match shell_stem(path).as_str() {
        "cmd" => ShellKind::Cmd,
        "powershell" => ShellKind::WindowsPowerShell,
        "pwsh" => ShellKind::Pwsh,
        "bash" => {
            let key = win_key(path);
            let wsl = [
                "\\windows\\system32\\bash.exe",
                "\\windows\\syswow64\\bash.exe",
                "\\windows\\sysnative\\bash.exe",
            ];
            if wsl.iter().any(|w| key.ends_with(w)) {
                ShellKind::WslBash
            } else {
                ShellKind::Bash
            }
        }
        _ => ShellKind::Other,
    }
}

/// Directory holding the MSYS tools for a Git `bash.exe` (`<root>\usr\bin`).
pub(crate) fn git_bash_usr_bin_str(bash: &str) -> String {
    let norm = win_normalize(bash);
    let comps: Vec<&str> = norm.split('\\').collect();
    let Some((_, dirs)) = comps.split_last() else {
        return "usr\\bin".to_string();
    };
    let Some((parent, above)) = dirs.split_last() else {
        return "usr\\bin".to_string();
    };
    if !parent.eq_ignore_ascii_case("bin") {
        return dirs.join("\\");
    }
    if above.last().is_some_and(|g| g.eq_ignore_ascii_case("usr")) {
        return dirs.join("\\");
    }
    if above.is_empty() {
        return "usr\\bin".to_string();
    }
    format!("{}\\usr\\bin", above.join("\\"))
}

/// Plain spelling of a canonicalized path, unless the plain form does not exist
/// or exceeds the legacy path limit, in which case the verbatim form is kept.
pub(crate) fn plain_or_verbatim(verbatim: &str, plain_exists: bool) -> String {
    let plain = strip_verbatim(verbatim);
    if plain_exists && plain.chars().count() <= MAX_PLAIN_PATH {
        plain
    } else {
        verbatim.to_string()
    }
}

/// `p` without a verbatim prefix when that is safe; unchanged off Windows.
pub(crate) fn display_path(p: &Path) -> PathBuf {
    if !cfg!(windows) {
        return p.to_path_buf();
    }
    let s = p.to_string_lossy();
    let plain = strip_verbatim(&s);
    if plain == *s {
        return p.to_path_buf();
    }
    PathBuf::from(plain_or_verbatim(&s, Path::new(&plain).exists()))
}

/// `Path::canonicalize` without the Windows verbatim prefix.
pub(crate) fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    p.canonicalize().map(|c| display_path(&c))
}

/// A `..` sits in the part of a path that does not exist yet.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PartialPathError {
    ParentInTail,
}

/// Split `p` into its nearest existing ancestor and the missing tail (returned
/// in reverse order, last component first). `..` is fine in the existing part
/// (the OS resolves it) but any `..` in the missing tail is rejected, since the
/// OS would resolve it later and could leave the workspace. Platform neutral.
pub(crate) fn split_existing(
    p: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Result<(PathBuf, Vec<OsString>), PartialPathError> {
    let mut cur = p;
    let mut tail: Vec<OsString> = Vec::new();
    loop {
        if exists(cur) {
            break;
        }
        match cur.components().next_back() {
            Some(Component::Normal(n)) => tail.push(n.to_os_string()),
            Some(Component::CurDir) => {}
            Some(Component::ParentDir) => return Err(PartialPathError::ParentInTail),
            _ => break,
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => break,
        }
    }
    Ok((cur.to_path_buf(), tail))
}

/// Canonicalize `p`, or its nearest existing ancestor plus the missing tail.
/// Errors when the missing tail contains `..`.
pub(crate) fn partial_canonical(p: &Path) -> std::io::Result<PathBuf> {
    if let Ok(c) = canonical(p) {
        return Ok(c);
    }
    let (existing, tail) = split_existing(p, |q| q.exists()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path escapes through '..' in a missing directory",
        )
    })?;
    let mut base = canonical(&existing).unwrap_or(existing);
    for seg in tail.iter().rev() {
        base.push(seg);
    }
    Ok(base)
}

/// Windows device names (`CON`, `NUL`, `COM1`..`COM9`, `LPT1`..`LPT9`, ...) by
/// the text before the first `.`, case-insensitive.
pub(crate) fn is_windows_reserved_stem(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(
            stem.as_bytes(),
            [b'C', b'O', b'M', d] | [b'L', b'P', b'T', d] if (b'1'..=b'9').contains(d)
        )
}

/// A tool-supplied name that must be exactly one safe path component. Empty,
/// NUL, separators (either kind), `.` and `..` are rejected everywhere. Only
/// with `windows` set: `:` (drive-relative, alternate data stream), a trailing
/// `.` or space, and reserved device names.
pub(crate) fn safe_single_component(name: &str, windows: bool) -> Result<&str, &'static str> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("name must be a single non-empty path component");
    }
    if name.contains(['\0', '/', '\\']) {
        return Err("name must not contain separators or NUL");
    }
    let mut comps = Path::new(name).components();
    if !matches!(
        (comps.next(), comps.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return Err("name must be a single path component");
    }
    if windows {
        if name.contains(':') {
            return Err("name must not contain ':'");
        }
        if name.ends_with(['.', ' ']) {
            return Err("name must not end with '.' or space");
        }
        if is_windows_reserved_stem(name) {
            return Err("name is a reserved device name");
        }
    }
    Ok(name)
}

/// `\` -> `/` only when `windows`; POSIX file names may contain a backslash.
pub(crate) fn to_slash(s: &str, windows: bool) -> String {
    if windows {
        s.replace('\\', "/")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
#[path = "winpath_test.rs"]
mod tests;

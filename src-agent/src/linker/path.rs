//! Containment-aware lexical path normalization.
//!
//! Paths are normalized without consulting the filesystem. Graph keys always use
//! `/`, including Windows drive and UNC paths when running on Unix.

/// Normalize a path lexically while preserving its root/prefix semantics.
///
/// Unresolved leading parents are retained for relative paths; parents above an
/// absolute POSIX, drive, or UNC root are clamped at that root.
pub fn normalize_lexical(path: &str) -> String {
    let path = path.replace('\\', "/");
    let (prefix, rest, absolute) = split_prefix(&path);
    let mut parts: Vec<&str> = Vec::new();

    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => match parts.last() {
                Some(last) if *last != ".." => {
                    parts.pop();
                }
                _ if !absolute => parts.push(".."),
                _ => {}
            },
            other => parts.push(other),
        }
    }

    let body = parts.join("/");
    match (prefix.as_str(), absolute, body.is_empty()) {
        ("", false, true) => ".".to_string(),
        ("", false, false) => body,
        ("/", true, true) => "/".to_string(),
        ("/", true, false) => format!("/{body}"),
        (prefix, true, true) => prefix.to_string(),
        (prefix, true, false) if prefix.ends_with('/') => format!("{prefix}{body}"),
        (prefix, true, false) => format!("{prefix}/{body}"),
        // Drive-relative paths (`C:foo`) are not absolute and retain that form.
        (prefix, false, true) => prefix.to_string(),
        (prefix, false, false) => format!("{prefix}{body}"),
    }
}

/// Whether a slash-normalized lexical path has a POSIX, drive, or UNC root.
pub fn is_absolute_lexical(path: &str) -> bool {
    let path = path.replace('\\', "/");
    split_prefix(&path).2
}

/// Join and normalize a candidate, returning its normalized path and owning
/// normalized absolute root. Relative candidates require an absolute base.
/// Ownership uses the longest component prefix, then lexical root order.
#[allow(dead_code)] // Phase-2 path utility; retained for future cross-boundary resolution.
pub fn join_contained(
    base: Option<&str>,
    candidate: &str,
    normalized_absolute_roots: &[String],
) -> Option<(String, String)> {
    let candidate = normalize_lexical(candidate);
    let joined = if is_absolute_lexical(&candidate) {
        candidate
    } else {
        let base = normalize_lexical(base?);
        if !is_absolute_lexical(&base) {
            return None;
        }
        normalize_lexical(&format!("{base}/{candidate}"))
    };
    let owner = owner_root(&joined, normalized_absolute_roots)?;
    Some((joined, owner.to_string()))
}

/// Choose the owner normalized absolute root using longest component-prefix
/// matching. Equal roots are resolved lexically, independent of input order.
pub fn owner_root<'a>(path: &str, roots: &'a [String]) -> Option<&'a str> {
    if !is_absolute_lexical(path) {
        return None;
    }
    roots
        .iter()
        .filter(|root| is_absolute_lexical(root) && component_prefix(root, path))
        .max_by(|a, b| {
            component_count(a)
                .cmp(&component_count(b))
                .then_with(|| b.cmp(a))
        })
        .map(String::as_str)
}

fn component_prefix(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn component_count(path: &str) -> usize {
    path.split('/').filter(|part| !part.is_empty()).count()
}

/// Return `(prefix, remainder, absolute)` for POSIX, drive, UNC, and relative paths.
fn split_prefix(path: &str) -> (String, &str, bool) {
    if let Some(unc) = path.strip_prefix("//") {
        let mut components = unc.split('/').filter(|part| !part.is_empty());
        if let (Some(server), Some(share)) = (components.next(), components.next()) {
            let prefix = format!("//{server}/{share}");
            let consumed = server.len() + share.len() + 2;
            let rest = unc.get(consumed..).unwrap_or("").trim_start_matches('/');
            return (prefix, rest, true);
        }
        return ("/".to_string(), unc.trim_start_matches('/'), true);
    }
    if path.starts_with('/') {
        return ("/".to_string(), path.trim_start_matches('/'), true);
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = &path[..2];
        if path.get(2..).is_some_and(|rest| rest.starts_with('/')) {
            return (format!("{drive}/"), path[3..].trim_start_matches('/'), true);
        }
        return (drive.to_string(), &path[2..], false);
    }
    (String::new(), path, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_preserves_relative_parents_and_clamps_roots() {
        assert_eq!(normalize_lexical("../foo"), "../foo");
        assert_eq!(normalize_lexical("a/../../b"), "../b");
        assert_eq!(normalize_lexical("/../../x"), "/x");
        assert_eq!(normalize_lexical("foo/./bar/../baz"), "foo/baz");
    }

    #[test]
    fn normalization_preserves_windows_roots_on_unix() {
        assert_eq!(normalize_lexical(r"C:\a\..\b"), "C:/b");
        assert_eq!(normalize_lexical(r"C:\"), "C:/");
        assert_eq!(
            normalize_lexical(r"\\server\share\a\..\b"),
            "//server/share/b"
        );
        assert_eq!(
            normalize_lexical(r"\\server\share\..\x"),
            "//server/share/x"
        );
    }

    #[test]
    fn owner_is_longest_component_prefix() {
        let roots = vec!["/project".into(), "/project/sub".into()];
        assert_eq!(
            owner_root("/project/sub/a.rs", &roots),
            Some("/project/sub")
        );
        assert_eq!(owner_root("/projectile/a.rs", &roots), None);
        assert_eq!(owner_root("relative.rs", &roots), None);
    }

    #[test]
    fn contained_join_rejects_escapes_and_missing_base() {
        let roots = vec!["/project".into(), "/project/nested".into()];
        assert_eq!(
            join_contained(Some("/project/src"), "../lib.rs", &roots),
            Some(("/project/lib.rs".into(), "/project".into()))
        );
        assert_eq!(
            join_contained(Some("/project/nested/src"), "a.rs", &roots),
            Some(("/project/nested/src/a.rs".into(), "/project/nested".into()))
        );
        assert!(join_contained(Some("/project"), "../../escape.rs", &roots).is_none());
        assert!(join_contained(None, "src/a.rs", &roots).is_none());
        assert!(join_contained(Some("relative"), "a.rs", &roots).is_none());
    }
}

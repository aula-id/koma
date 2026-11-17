//! Platform-gated separator folding. A backslash is a path separator only on
//! Windows; on macOS and Linux it is a legal file-name character and must survive.

/// Replace `\` with `/` only when `windows` is true; otherwise return `s` unchanged.
pub(crate) fn to_slash(s: &str, windows: bool) -> String {
    if windows {
        s.replace('\\', "/")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::to_slash;

    #[test]
    fn backslash_folds_only_on_windows() {
        assert_eq!(to_slash("a\\b", false), "a\\b");
        assert_eq!(to_slash("a\\b", true), "a/b");
        assert_eq!(to_slash("a/b", true), "a/b");
        assert_eq!(to_slash("a/b", false), "a/b");
    }
}

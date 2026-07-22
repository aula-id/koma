//! Compile-time regex helper — never panics, always returns a usable `Regex`.

use regex::Regex;

/// Compile a static regex from a literal pattern. If the literal is somehow
/// invalid (should be impossible — caught at compile time in practice), log to
/// `error.log` and return a regex that matches nothing. Never panics.
pub(crate) fn static_re(pattern: &'static str) -> Regex {
    match Regex::new(pattern) {
        Ok(re) => re,
        Err(e) => {
            crate::model::store::append_global_error_log(
                "static_re",
                &format!("BUG: static regex compile failed ({pattern}): {e}"),
            );
            // Hardcoded never-match; if this itself fails the process is beyond
            // recovery, but we still avoid bare `.expect()` for Clippy purity.
            #[allow(clippy::expect_used)]
            {
                Regex::new(r"a^").expect("infallible never-match regex")
            }
        }
    }
}

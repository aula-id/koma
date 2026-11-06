//! Typed result verdicts, parent side.
//!
//! Every daemon result carries exactly one verdict from a closed set. The daemon
//! produces them (`koma_sec_daemon/verdict.py`); this parses them, and the two
//! spellings move together.
//!
//! The parse is **total**: a result that is not a typed frame becomes
//! [`Verdict::Untyped`] with the original text as its excerpt, never an error. A
//! handler not yet migrated must keep working, but it must also be visibly
//! distinguishable, so `untyped` is a state a caller can refuse to confirm on.
//!
//! [`enforce_shape`] is the other half of the contract: a result the model cannot
//! use is worse than a pointer to one. Oversized payloads and long arrays are held
//! aside and replaced with a handle plus a count, so a 200-row scan does not eat
//! the turn's context.

use serde::Deserialize;

/// Excerpt ceiling before [`enforce_shape`] holds a payload aside. Sits below the
/// daemon's own clip limit so the guard is reachable in practice rather than
/// theoretical.
const EXCERPT_SOFT_LIMIT: usize = 6 * 1024;

/// Objects past this count are summarised as a count plus a handle.
const MAX_ARRAY_OBJECTS: usize = 30;

/// What a tool concluded. Closed set — `untyped` is a migration state, not a
/// conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The claimed thing happened, with bytes.
    Confirmed,
    /// It ran correctly and the thing is absent.
    NotFound,
    /// The target answered and a value must be supplied.
    NeedsInput,
    /// Something stopped it: WAF, 401, upstream timeout.
    Blocked,
    /// The budget expired.
    TimedOut,
    /// A dependency is absent. Carries the install hint.
    Unavailable,
    /// The tool crashed or the dispatch refused it.
    Error,
    /// Not migrated to the typed contract yet. Cannot establish a finding.
    Untyped,
}

impl Verdict {
    /// May this verdict stand as evidence for a finding? Only a confirmation, and
    /// a confirmation that actually arrived typed. Unreferenced until the board
    /// and finding state machine land (phase 6), hence the scoped allow rather
    /// than a stub caller.
    #[must_use]
    #[allow(dead_code)]
    pub fn establishes_finding(self) -> bool {
        matches!(self, Self::Confirmed)
    }

    /// A short stable token, for status lines and logs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NotFound => "not_found",
            Self::NeedsInput => "needs_input",
            Self::Blocked => "blocked",
            Self::TimedOut => "timed_out",
            Self::Unavailable => "unavailable",
            Self::Error => "error",
            Self::Untyped => "untyped",
        }
    }
}

/// A parsed result. `excerpt` is what the model reads; `handle` is where the full
/// bytes went when they did not fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedResult {
    pub verdict: Verdict,
    pub excerpt: String,
    pub handle: Option<String>,
}

impl TypedResult {
    /// The wire frame the daemon would have sent. Round-trips with [`parse`].
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut frame = serde_json::json!({
            "verdict": self.verdict.as_str(),
            "excerpt": self.excerpt,
        });
        if let Some(h) = &self.handle {
            frame["handle"] = serde_json::Value::String(h.clone());
        }
        frame.to_string()
    }
}

#[derive(Deserialize)]
struct Frame {
    verdict: Verdict,
    #[serde(default)]
    excerpt: String,
    #[serde(default)]
    handle: Option<String>,
}

/// Parse a daemon result. Total: anything unrecognized becomes [`Verdict::Untyped`]
/// with the raw text preserved as the excerpt.
#[must_use]
pub fn parse(raw: &str) -> TypedResult {
    if let Ok(frame) = serde_json::from_str::<Frame>(raw) {
        return TypedResult {
            verdict: frame.verdict,
            excerpt: frame.excerpt,
            handle: frame.handle,
        };
    }
    TypedResult {
        verdict: Verdict::Untyped,
        excerpt: raw.to_string(),
        handle: None,
    }
}

/// Hold an oversized or long-array result aside behind a handle.
///
/// `store` writes the full bytes and returns the handle. It is only called when
/// the payload actually needs holding, so a caller can pass a cheap closure. When
/// storing fails the full excerpt is kept and a note is appended — losing the
/// pointer must never lose the data too.
///
/// A confirmation whose payload is held aside still reports `confirmed`; the
/// verdict is the conclusion, the handle is where to read it.
#[must_use]
pub fn enforce_shape(mut result: TypedResult, store: impl FnOnce(&str) -> Option<String>) -> TypedResult {
    let object_count = serde_json::from_str::<serde_json::Value>(&result.excerpt)
        .ok()
        .and_then(|v| v.as_array().map(Vec::len))
        .unwrap_or(0);

    let too_long = result.excerpt.len() > EXCERPT_SOFT_LIMIT;
    let too_many = object_count > MAX_ARRAY_OBJECTS;
    if !too_long && !too_many {
        return result;
    }

    match store(&result.excerpt) {
        Some(handle) => {
            let summary = if too_many {
                format!("{object_count} objects")
            } else {
                format!("{} bytes", result.excerpt.len())
            };
            result.excerpt = format!("{summary} held at {handle}");
            result.handle = Some(handle);
        }
        None => {
            let keep = result.excerpt[..EXCERPT_SOFT_LIMIT].to_string();
            result.excerpt = format!(
                "{keep}\n…[{} more bytes, not held]",
                result.excerpt.len() - EXCERPT_SOFT_LIMIT
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(text: &str) -> Option<String> {
        Some(format!("sha256:{}", text.len()))
    }

    #[test]
    fn a_typed_frame_round_trips() {
        let original = TypedResult {
            verdict: Verdict::Blocked,
            excerpt: "403 from waf".into(),
            handle: Some("rec-9".into()),
        };
        assert_eq!(parse(&original.to_json()), original);
    }

    #[test]
    fn a_bare_string_is_untyped_and_preserved() {
        let got = parse("nmap said hello");
        assert_eq!(got.verdict, Verdict::Untyped);
        assert_eq!(got.excerpt, "nmap said hello");
    }

    #[test]
    fn an_unknown_verdict_label_degrades_to_untyped() {
        // Forward compatibility: a newer daemon's new verdict must not break the
        // turn, it just cannot establish a finding here.
        let got = parse(r#"{"verdict":"quarantined","excerpt":"x"}"#);
        assert_eq!(got.verdict, Verdict::Untyped);
    }

    #[test]
    fn a_frame_without_an_excerpt_is_still_typed() {
        let got = parse(r#"{"verdict":"not_found"}"#);
        assert_eq!(got.verdict, Verdict::NotFound);
        assert!(got.excerpt.is_empty());
    }

    #[test]
    fn only_confirmed_establishes_a_finding() {
        assert!(Verdict::Confirmed.establishes_finding());
        for v in [
            Verdict::NotFound,
            Verdict::NeedsInput,
            Verdict::Blocked,
            Verdict::TimedOut,
            Verdict::Unavailable,
            Verdict::Error,
            Verdict::Untyped,
        ] {
            assert!(!v.establishes_finding(), "{v:?} must not confirm");
        }
    }

    #[test]
    fn an_oversized_payload_is_held_behind_a_handle() {
        let long = TypedResult {
            verdict: Verdict::Confirmed,
            excerpt: "x".repeat(EXCERPT_SOFT_LIMIT + 1),
            handle: None,
        };
        let got = enforce_shape(long, stored);
        assert_eq!(got.verdict, Verdict::Confirmed);
        assert!(got.handle.is_some());
        assert!(got.excerpt.contains("held at"));
        assert!(got.excerpt.len() < EXCERPT_SOFT_LIMIT);
    }

    #[test]
    fn a_long_array_is_summarised_by_count() {
        let rows: Vec<serde_json::Value> = (0..MAX_ARRAY_OBJECTS + 1)
            .map(|i| serde_json::json!({ "n": i }))
            .collect();
        let got = enforce_shape(
            TypedResult {
                verdict: Verdict::Confirmed,
                excerpt: serde_json::to_string(&rows).expect("serialisable"),
                handle: None,
            },
            stored,
        );
        assert!(got.excerpt.contains("objects held at"));
    }

    #[test]
    fn a_small_result_is_untouched() {
        let small = TypedResult {
            verdict: Verdict::NotFound,
            excerpt: "nothing".into(),
            handle: None,
        };
        assert_eq!(enforce_shape(small.clone(), stored), small);
    }

    #[test]
    fn a_failed_store_keeps_the_data() {
        let long = TypedResult {
            verdict: Verdict::Confirmed,
            excerpt: "y".repeat(EXCERPT_SOFT_LIMIT + 10),
            handle: None,
        };
        let got = enforce_shape(long, |_| None);
        assert!(got.handle.is_none());
        assert!(got.excerpt.contains("not held"));
        // The retained prefix is still there, so the model reads real bytes.
        assert!(got.excerpt.starts_with('y'));
    }
}

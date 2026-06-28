//! Numeric formatters and small helpers for the usage dashboard.

use std::time::{SystemTime, UNIX_EPOCH};

/// Fixed width of the label column in the vertical KPI list.
pub(crate) const KPI_LABEL_W: usize = 10;

/// Width reserved for the right-aligned metric label (chars).
pub(crate) const METRIC_LABEL_W: usize = 9;

/// USD cost: `$1.23` for >= $1, `$0.0045` for small values.
pub(crate) fn fmt_cost(cost: f64) -> String {
    if cost >= 1.0 {
        format!("${cost:.2}")
    } else {
        format!("${cost:.4}")
    }
}

/// Humanise token count: 1_234_567 -> "1.2M", 12_345 -> "12.3k", 999 -> "999".
pub(crate) fn fmt_tokens_i64(n: i64) -> String {
    fmt_tok(n as f64)
}
pub(crate) fn fmt_tokens_u64(n: u64) -> String {
    fmt_tok(n as f64)
}
fn fmt_tok(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{n:.0}")
    }
}

/// Truncate to `max` chars, appending `...` if cut. Char-aware.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_owned()
    } else {
        let cut: String = chars[..max.saturating_sub(3)].iter().collect();
        cut + "..."
    }
}

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

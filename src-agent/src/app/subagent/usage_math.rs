//! Pure arithmetic for per-step sub-agent spend.
//!
//! Kept free of IO so kill/cancel rollup + additive multi-step cost can be
//! unit-tested without touching the live `~/.koma/usage.sqlite`.

/// Resolve the USD cost to credit for one sub-agent step.
///
/// Prefers the provider-reported `cost`. When that is 0.0 (Codex/Claude
/// hardcode it; many direct APIs omit it), falls back to the curated catalogue
/// overlay's per-1M-token pricing — same rule the interactive main path uses in
/// `app::runtime::stream::turn`. Without this, multi-step sub-agents always
/// report $0 and parent counters never see the spend.
pub(crate) fn effective_step_cost(
    endpoint: &str,
    model_id: &str,
    prompt_tokens: u64,
    cached_tokens: u64,
    completion_tokens: u64,
    reported_cost: f64,
) -> f64 {
    if reported_cost == 0.0 {
        crate::service::catalogue_overlay::overlay_cost(
            endpoint,
            model_id,
            prompt_tokens,
            cached_tokens,
            completion_tokens,
        )
        .unwrap_or(reported_cost)
    } else {
        reported_cost
    }
}

/// Apply one finished step's spend onto the parent session's running totals.
///
/// Returns `None` when the step contributed nothing (same gate the orchestrator
/// uses before writing a ledger row). Otherwise returns the new
/// `(parent_cost, parent_tokens_out)`. Kill/cancel safety relies on calling this
/// per finished step as UsageReports arrive — unfinished steps simply never fire.
pub(crate) fn fold_parent_spend(
    parent_cost: f64,
    parent_tokens_out: u64,
    step_tokens_out: u64,
    step_cost: f64,
) -> Option<(f64, u64)> {
    if step_tokens_out == 0 && step_cost == 0.0 {
        None
    } else {
        Some((parent_cost + step_cost, parent_tokens_out + step_tokens_out))
    }
}

/// Running sub-agent totals after one step's (completion_tokens, cost) lands.
/// `tokens_out` and `cost` are cumulative; prompt size is a gauge (caller keeps it).
pub(crate) fn accumulate_step(
    acc_tokens_out: u64,
    acc_cost: f64,
    step_completion: u64,
    step_cost: f64,
) -> (u64, f64) {
    (acc_tokens_out + step_completion, acc_cost + step_cost)
}

#[cfg(test)]
#[path = "usage_math_usage_math_tests.rs"]
mod usage_math_tests;

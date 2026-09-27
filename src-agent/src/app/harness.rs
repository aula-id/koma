//! Safety classification for advisory prompts and gated tool calls.
//!
//! Decisions come only from complete final JSON. An empty/incomplete answer with
//! finish_reason=length permits one 2,000 → 4,000 token retry. All other primary
//! failures can try Main once when its full connection identity differs. Each
//! route gets at most half the shared 120-second deadline. Diagnostics carry
//! route names and response categories, never prompts, credentials or reasoning.
//! Unavailable tool classification requires call-scoped interactive approval;
//! headless and delegated callers fail closed without changing tool/mode gates.

use std::path::{Path, PathBuf};

use crate::app::resolve::resolve_role_dispatch;
use crate::dto::chat::{ChatMessage, Role};
use crate::model::app_config::{AppConfig, ModelRole};
use crate::model::settings::Settings;
use crate::service::openrouter::OpenRouterClient;

/// A classifier decision.
///
/// - `allow`: proceed when true.
/// - `reason`: short human-readable note (empty on a clean allow, the block
///   reason otherwise).
/// - `available`: true when the classifier actually produced this verdict;
///   false when it couldn't be reached (network error, timeout, empty or
///   unparseable reply). The caller treats `available = false` specially —
///   TAC degrades to a human prompt in BOTH agent modes rather than trusting
///   `allow`, so a classifier outage never silently runs or blocks a call.
#[derive(Debug, Clone)]
pub struct Verdict {
    pub allow: bool,
    pub reason: String,
    pub available: bool,
}

impl Verdict {
    /// A successfully-parsed ALLOW.
    fn allow() -> Self {
        Self {
            allow: true,
            reason: String::new(),
            available: true,
        }
    }
    /// A successfully-parsed BLOCK with its reason.
    fn block(reason: impl Into<String>) -> Self {
        Self {
            allow: false,
            reason: reason.into(),
            available: true,
        }
    }
}

/// Normalise a path to a comparable form. Prefer the canonical path (resolves
/// symlinks, `.`/`..`, and relative paths against the cwd); fall back to the
/// path as-given when it can't be canonicalised (e.g. it doesn't exist yet).
fn norm(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Deterministic workspace check (WC). Returns true when `workdir` is at OR
/// under the process launch directory, OR at/under any entry in the session's
/// allow-set: every `settings.workdir` entry plus every `settings.allowed_folders`
/// entry.
///
/// `workdir` (the dir actually being checked) is the session's *effective* cwd —
/// which, after a model/`/cd`, can be ANY subdirectory of an allowed root (e.g.
/// `cd src/` under a `/proj` workspace makes it `/proj/src`). So the check is a
/// **containment** test, not exact equality: a cwd inside an allowed root stays
/// allowed (subdirectory navigation works under harness mode), while a cwd that
/// escapes every root is denied. The path list may name several directories the
/// session is allowed to touch, so ALL of them count as roots, alongside the
/// extra `allowed_folders`. The launch directory is ALWAYS a root regardless of
/// the lists, so the common case (running the agent in the folder you want to
/// work in) just works.
///
/// Comparison is on canonicalised paths via component-wise [`Path::starts_with`],
/// so equivalent spellings (relative vs absolute, trailing slash, symlink) match
/// and the containment honours path boundaries — `/proj` contains `/proj/src` but
/// NOT a sibling like `/proj-evil`.
pub fn workspace_allowed(settings: &Settings, workdir: &Path, launch_dir: &Path) -> bool {
    let wd = norm(workdir);
    // `starts_with` is reflexive (a path starts with itself), so this single
    // containment test covers both "cwd IS the launch dir" and "cwd is under it".
    if wd.starts_with(norm(launch_dir)) {
        return true;
    }
    // The allow-set is the union of the workdir path list and the extra allowed
    // folders; a blank entry can't match a real directory after canonicalisation.
    // Each entry is a ROOT: the cwd is allowed when it sits at or beneath any of
    // them.
    settings
        .workdir
        .iter()
        .chain(settings.allowed_folders.iter())
        .map(|f| f.trim())
        .filter(|f| !f.is_empty())
        .map(|f| norm(Path::new(f)))
        .any(|allowed| wd.starts_with(allowed))
}

/// The verdict object the classifier is asked to emit as strict JSON. `allow`
/// drives the decision; `reason` is the short note. A second shape (`verdict`
/// as `"allow"`/`"block"`) is tolerated for robustness — see [`parse_verdict`].
#[derive(serde::Deserialize)]
struct VerdictJson {
    allow: Option<bool>,
    #[serde(default)]
    verdict: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// Only a complete final JSON decision is authoritative. No prose/keyword scan.
fn parse_verdict(reply: &str) -> Option<Verdict> {
    let reply = reply.trim();
    let reply = reply
        .strip_prefix("```json")
        .or_else(|| reply.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(reply)
        .trim();
    let value: VerdictJson = serde_json::from_str(reply).ok()?;
    let allow = match (value.allow, value.verdict.as_deref()) {
        (Some(allow), None) => allow,
        (None, Some("allow")) | (Some(true), Some("allow")) => true,
        (None, Some("block")) | (Some(false), Some("block")) => false,
        _ => return None,
    };
    Some(if allow {
        Verdict::allow()
    } else {
        Verdict::block(
            value
                .reason
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "flagged".into()),
        )
    })
}

/// How long to wait for a classifier verdict before giving up. With thinking
/// turned OFF the call is fast, so this is mostly headroom for a slow network;
/// the bound still matters because TAC PARKS a tool round on this call (run on a
/// spawned off-thread task), so it must resolve promptly or the round stays parked.
/// On timeout the verdict is `unavailable("classifier timeout")`, so the caller
/// degrades (TAC → human prompt) rather than hanging.
const CLASSIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Run bounded classification. The unavailable_allow posture only distinguishes
/// advisory PC from gated TAC; unavailable results never authorize tool execution.
pub(crate) async fn classify(
    client: &OpenRouterClient,
    config: &AppConfig,
    settings: &Settings,
    messages: Vec<ChatMessage>,
    unavailable_allow: bool,
) -> Verdict {
    classify_bounded(
        client,
        config,
        settings,
        messages,
        unavailable_allow,
        CLASSIFY_TIMEOUT,
    )
    .await
}

async fn classify_bounded(
    client: &OpenRouterClient,
    config: &AppConfig,
    settings: &Settings,
    messages: Vec<ChatMessage>,
    unavailable_allow: bool,
    timeout: std::time::Duration,
) -> Verdict {
    use crate::app::resolve::{role_resolution, same_route};
    let primary = resolve_role_dispatch(config, settings, ModelRole::Safeguard);
    let fallback = resolve_role_dispatch(config, settings, ModelRole::Main)
        .filter(|main| primary.as_ref().is_none_or(|p| !same_route(p, main)));
    let deadline = tokio::time::Instant::now() + timeout;
    let mut failures = Vec::new();
    let primary_label = role_resolution(config, settings, ModelRole::Safeguard)
        .configured_model
        .or_else(|| primary.as_ref().map(|r| r.model_id.clone()))
        .unwrap_or_else(|| "unconfigured".into());
    if primary.is_none() {
        failures.push(format!(
            "primary {primary_label}: {}",
            role_resolution(config, settings, ModelRole::Safeguard)
                .reason
                .unwrap_or_else(|| "unavailable".into())
        ));
    }
    for (is_fallback, route) in [(false, primary), (true, fallback)] {
        let Some(route) = route else {
            continue;
        };
        // Reserve time for Main even if the primary hangs. Both budgets on this route
        // share this deadline; the complete chain never exceeds 120 seconds.
        let route_deadline = deadline.min(tokio::time::Instant::now() + timeout / 2);
        let mut failure = "unavailable".to_string();
        for budget in [2_000, 4_000] {
            let response = tokio::time::timeout_at(
                route_deadline,
                client.classify_with(
                    route.conn(),
                    &route.model_id,
                    route.provider(),
                    messages.clone(),
                    budget,
                ),
            )
            .await;
            match response {
                Ok(Ok(reply)) => {
                    let reply: crate::service::openrouter::ClassifierReply = reply;
                    if let Some(mut verdict) = parse_verdict(&reply.content) {
                        if is_fallback {
                            verdict.reason = format!(
                                "primary {primary_label} failed ({}); fallback {}: {}",
                                failures.join("; "),
                                route.model_id,
                                verdict.reason
                            );
                        }
                        return verdict;
                    }
                    failure = format!(
                        "{} (finish={}, input={:?}, output={:?})",
                        reply.category.unwrap_or("invalid_verdict"),
                        reply.finish_reason.as_deref().unwrap_or("unknown"),
                        reply.prompt_tokens,
                        reply.completion_tokens
                    );
                    if budget == 2_000 && reply.truncated() {
                        continue;
                    }
                }
                Ok(Err(error)) => {
                    failure = if error
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|e| e.is_timeout())
                    {
                        "timeout".into()
                    } else {
                        "transport_or_response_error".into()
                    };
                }
                Err(_) => failure = "timeout".into(),
            }
            break;
        }
        failures.push(format!(
            "{} {}: {failure}",
            if is_fallback { "fallback" } else { "primary" },
            route.model_id
        ));
    }
    Verdict {
        allow: unavailable_allow,
        available: false,
        reason: failures.join("; "),
    }
}

/// Prompt classifier (PC). Classify a user prompt; ADVISORY only.
///
/// FAIL-OPEN: a malformed reply or a failed call returns `allow = true` (with
/// `available = false` and the REAL cause as `reason`) so the turn is never
/// blocked by classifier trouble. The caller surfaces a block verdict as a toast
/// and otherwise proceeds; it ignores `available` because PC is advisory.
pub async fn classify_prompt(
    client: &OpenRouterClient,
    config: &AppConfig,
    settings: &Settings,
    user_prompt: &str,
) -> Verdict {
    let messages = vec![
        ChatMessage::new(Role::System, crate::resources::classifier_prompt()),
        ChatMessage::new(Role::User, user_prompt),
    ];
    // Advisory fail-open: on an unavailable classifier, allow the turn but keep
    // the real reason for the toast.
    classify(client, config, settings, messages, true).await
}

/// Tool-call classifier (TAC). Classify a single tool call for auto-run safety,
/// INTENT-AWARE: it sees the recent conversation tail (the latest user message
/// plus the prior turns and the agent's stated plan) alongside the proposed call,
/// so it can block a mutation the user never asked for (e.g. the user asked a
/// question but the model tried to edit a file).
///
/// On a malformed reply, a failed call, or a timeout this returns a verdict with
/// `available = false` — the caller degrades that to a human approval prompt in
/// BOTH agent modes, so a classifier outage can never silently auto-run a risky
/// call nor silently block it. A successfully-parsed verdict carries
/// `available = true` (allow → auto-run; block → the caller acts on the mode).
pub async fn classify_toolcall(
    client: &OpenRouterClient,
    config: &AppConfig,
    settings: &Settings,
    convo_context: &str,
    tool_name: &str,
    args_json: &str,
) -> Verdict {
    let call = format!(
        "Recent conversation (oldest to newest; the last line is the user's latest message, which may be a short confirmation of something proposed earlier):\n{convo_context}\n\nProposed tool call:\ntool: {tool_name}\narguments: {args_json}"
    );
    let messages = vec![
        ChatMessage::new(Role::System, crate::resources::classifier_toolcall()),
        ChatMessage::new(Role::User, call),
    ];
    // TAC fail-closed on unavailable: `allow = false` so the caller degrades to a
    // human decision per mode; the real reason rides along for the prompt/toast.
    classify(client, config, settings, messages, false).await
}

#[cfg(test)]
#[path = "harness_regression_test.rs"]
pub(crate) mod regression_test;

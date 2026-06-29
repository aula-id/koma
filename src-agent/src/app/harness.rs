//! Safety harness ("Pass B"): wraps the agentic tool loop with an LLM safety
//! classifier, plus a deterministic workspace check. All of it is gated behind
//! the master `classifier_enabled` setting (default off); when disabled the loop
//! behaves exactly as it did before this module existed.
//!
//! Three checks, per the locked design:
//!
//! - **WC (workspace check)** — [`workspace_allowed`]. Deterministic, no network:
//!   is the session workdir the launch directory, or in the allow-list? Tools run
//!   only against an allowed workspace.
//! - **PC (prompt classifier)** — [`classify_prompt`]. Runs ONCE per turn,
//!   ADVISORY only: classify the user's prompt and surface a toast if flagged.
//!   It never blocks the turn (fail-open).
//! - **TAC (tool-call classifier)** — [`classify_toolcall`]. Runs PER risky tool
//!   call in BOTH agent modes, INTENT-AWARE (it sees the user's latest request
//!   plus the proposed call). An "allow" verdict auto-runs the call; a "block"
//!   verdict is acted on by the caller per mode (Auto records a "blocked by
//!   harness" result and continues; Normal prompts the human). If the classifier
//!   is unavailable (error/timeout) the verdict's `available` flag is false and
//!   the caller degrades to a human prompt in both modes.
//!
//! Both classifier calls run against the resolved Safeguard route
//! (`resolve_role(config, settings, Safeguard)` → endpoint + key + model +
//! upstream-route slug) via the dedicated [`OpenRouterClient::classify_with`],
//! which turns thinking OFF and pins a strict JSON schema so the safeguard model
//! returns a machine-parseable `{allow, reason}` object fast and deterministically.
//! An unresolved Safeguard route (fail-closed) becomes an unavailable verdict,
//! degraded to a human prompt (TAC) / advisory toast (PC) by the caller.
//! They build a two-message conversation (System = the embedded policy text, User
//! = the prompt / the request + tool call) and parse the reply with
//! [`parse_verdict`] (JSON-first, with a lenient text-scan fallback), all bounded
//! by [`CLASSIFY_TIMEOUT`] so the sync loop can't freeze. Every failure becomes an
//! unavailable verdict carrying the REAL cause (HTTP error / timeout / unparseable
//! slice) so the UI shows what actually went wrong instead of a generic string.

use std::path::{Path, PathBuf};

use crate::app::resolve::resolve_role;
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

/// Truncate a string to at most `max` characters (char-boundary safe), appending
/// an ellipsis when it was cut. Keeps the diagnostic reasons that ride into the
/// UI from blowing up the approval box / toast.
fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}…")
}

/// Build a [`Verdict`] from a parsed `{allow|verdict, reason}` object. `allow`
/// wins when present; otherwise `verdict` is read as `"allow"` (any other value,
/// including `"block"`, is a block). An allow drops its reason (a clean allow
/// carries none); a block keeps the reason, defaulting to "flagged" when blank.
fn verdict_from_json(v: VerdictJson) -> Verdict {
    let allow = match v.allow {
        Some(a) => a,
        None => v
            .verdict
            .as_deref()
            .map(|s| s.trim().eq_ignore_ascii_case("allow"))
            .unwrap_or(false),
    };
    if allow {
        return Verdict::allow();
    }
    let reason = v
        .reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "flagged".to_string());
    Verdict::block(reason)
}

/// Parse a classifier reply into a [`Verdict`], robustly.
///
/// Order of attempts:
/// 1. **JSON-first.** Parse the whole reply as `{allow|verdict, reason}`. If that
///    fails (the model wrapped the object in prose/code fences), locate the first
///    `{` and last `}` and parse that substring. `allow` is authoritative; a
///    `{"verdict":"allow"|"block"}` shape is also accepted.
/// 2. **Lenient text scan (fallback).** Look (case-insensitive) for `VERDICT:`
///    followed by ALLOW/BLOCK; failing that, scan whole-word for
///    ALLOW/ALLOWED/SAFE (→ allow) or BLOCK/BLOCKED/DENY/UNSAFE (→ block), taking
///    a short slice of the reply as the reason.
///
/// Returns `None` only when NOTHING parseable is found — the caller turns that
/// into an unavailable verdict carrying the raw reply, never a trusted decision.
fn parse_verdict(reply: &str) -> Option<Verdict> {
    let trimmed = reply.trim();

    // 1. JSON-first: the strict-schema happy path.
    if let Ok(v) = serde_json::from_str::<VerdictJson>(trimmed) {
        return Some(verdict_from_json(v));
    }
    // 1b. Prose/fence-wrapped JSON: carve out the first {...} and retry.
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            if let Ok(v) = serde_json::from_str::<VerdictJson>(&trimmed[start..=end]) {
                return Some(verdict_from_json(v));
            }
        }
    }

    // 2. Lenient text scan. First honour an explicit `VERDICT:` line.
    for line in trimmed.lines() {
        let Some((_, rest)) = line.split_once("VERDICT:") else {
            continue;
        };
        let rest = rest.trim();
        let upper = rest.to_ascii_uppercase();
        if upper.starts_with("ALLOW") {
            return Some(Verdict::allow());
        }
        if let Some(after) = upper.strip_prefix("BLOCK") {
            // Re-slice the ORIGINAL (non-uppercased) text so the reason's casing
            // is preserved; `after` only told us where it starts.
            let reason = rest[rest.len() - after.len()..].trim();
            let reason = if reason.is_empty() {
                "flagged".to_string()
            } else {
                reason.to_string()
            };
            return Some(Verdict::block(reason));
        }
    }
    // 2b. No VERDICT: line — scan for a standalone decision keyword. ALLOW is
    //     checked first; any block keyword present forces a block with a short
    //     slice of the reply as the reason.
    let upper = trimmed.to_ascii_uppercase();
    let has_word = |word: &str| {
        upper.split(|c: char| !c.is_ascii_alphanumeric()).any(|tok| tok == word)
    };
    if has_word("ALLOW") || has_word("ALLOWED") || has_word("SAFE") {
        return Some(Verdict::allow());
    }
    if has_word("BLOCK") || has_word("BLOCKED") || has_word("DENY") || has_word("UNSAFE") {
        return Some(Verdict::block(truncate(trimmed, 100)));
    }

    // 3. Nothing parseable.
    None
}

/// How long to wait for a classifier verdict before giving up. With thinking
/// turned OFF the call is fast, so this is mostly headroom for a slow network;
/// the bound still matters because the sync loop drives this via `block_on` and
/// must never freeze. On timeout the verdict is `unavailable("classifier
/// timeout")`, so the caller degrades (TAC → human prompt) rather than hanging.
const CLASSIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

/// Run the classifier model over `messages` and return a [`Verdict`].
///
/// Never propagates an error or panics — every failure becomes an unavailable
/// verdict carrying the REAL cause so the UI can show it:
/// - reply parsed → that verdict (`available = true`).
/// - reply unparseable → `unavailable("unparseable verdict: <slice>")`.
/// - HTTP / network error → `unavailable("classifier error: <detail>")`.
/// - timeout → `unavailable("classifier timeout")`.
///
/// `unavailable_allow` selects the fail-open posture for the *unavailable* cases:
/// PC passes `true` (advisory — the turn still proceeds) and TAC passes `false`
/// (the caller decides per mode). The reason is preserved either way so the toast
/// / approval box is accurate. Bounded by [`CLASSIFY_TIMEOUT`]; `block_on`-safe.
async fn classify(
    client: &OpenRouterClient,
    config: &AppConfig,
    settings: &Settings,
    messages: Vec<ChatMessage>,
    unavailable_allow: bool,
) -> Verdict {
    // Build an unavailable verdict carrying `reason`, with the caller's fail-open
    // `allow` posture (only meaningful while `available = false`).
    let unavailable = |reason: String| Verdict {
        allow: unavailable_allow,
        reason,
        available: false,
    };
    // Resolve the Safeguard route (session override > config > legacy classifier
    // field). FAIL-CLOSED: an unresolved safeguard (unassigned / missing provider /
    // empty key, with no legacy classifier model) yields `None` → an unavailable
    // verdict, exactly as when the classifier can't be reached. The caller degrades
    // that to a human prompt (TAC) / advisory toast (PC) rather than auto-allowing.
    let Some(route) = resolve_role(config, settings, ModelRole::Safeguard) else {
        return unavailable("safeguard model not configured".to_string());
    };
    // Call-boundary gate (fail-CLOSED): an Anthropic-typed safeguard provider can't
    // be dispatched against the OpenAI-compatible client (native Anthropic is
    // deferred). Treat it as UNAVAILABLE rather than POSTing a doomed body — the
    // caller degrades that to a human prompt (TAC) / advisory toast (PC), never a
    // silent allow.
    if !route.is_routable() {
        return unavailable("safeguard provider not wired (Anthropic)".to_string());
    }
    match tokio::time::timeout(
        CLASSIFY_TIMEOUT,
        client.classify_with(route.conn(), &route.model_id, route.provider(), messages),
    )
    .await
    {
        Ok(Ok(reply)) => match parse_verdict(&reply) {
            Some(v) => v,
            None => unavailable(format!("unparseable verdict: {}", truncate(&reply, 100))),
        },
        Ok(Err(e)) => unavailable(format!("classifier error: {}", truncate(&e.to_string(), 100))),
        Err(_) => unavailable("classifier timeout".to_string()),
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

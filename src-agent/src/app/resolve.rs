//! Per-role route resolution: the single chokepoint that turns a [`ModelRole`]
//! into one concrete [`Resolved`] route (model id + endpoint + key + wire type +
//! optional upstream pin + effort).
//!
//! The runtime has five model-driven roles — Main (interactive chat), Awareness
//! (project-doc summary), Safeguard (the safety classifier), Compactor
//! (`/compact`, which rides Main today), and Planner (drives the MAIN turn
//! instead of Main while the session is in `AgentMode::Plan`; see
//! [`resolve_turn_model`]). Each is assigned a model via the global catalogue
//! (`config.models`) or a per-session override (`settings.session_models`); that
//! model points at a provider connection (`config.providers`) by uuid, which
//! carries the endpoint + key + wire type. [`resolve_role`] walks that chain and
//! produces the route the call site hands to the client.
//!
//! ## Resolution order
//!
//! 1. Find the model assigned to `role`: session overrides first
//!    (`settings.session_models`), then the global catalogue (`config.models`).
//! 2. Resolve that model's provider by `provider_uuid` against `config.providers`.
//!    A hit produces the [`Resolved`] route directly.
//! 3. If no model is assigned, OR the assigned model's `provider_uuid` does not
//!    resolve to a known provider, fall through to the per-role LEGACY fallback —
//!    the old per-field `settings.*` behaviour, so an empty/old config never
//!    bricks chat (Main/Compactor/Awareness) and the safeguard fails CLOSED.
//!
//! ## Fallback table (when step 2 finds no provider)
//!
//! | role      | fallback                                                            |
//! |-----------|---------------------------------------------------------------------|
//! | Main      | legacy `settings.model` / `api_key` @ `DEFAULT_BASE_URL`            |
//! | Compactor | resolve Main (compactor rides Main; no config slot of its own)      |
//! | Awareness | inherit Main (same route as the Main role)                          |
//! | Safeguard | legacy `classifier_model` if set; else `None` (FAIL-CLOSED)        |
//! | Planner   | `None` — no fallback at all; caller falls back to Main              |
//!
//! ## Foot-gun (do not regress)
//!
//! An Awareness model that is explicitly assigned (found in step 1 and whose provider
//! resolves in step 2) ALWAYS wins — explicit assignment is the only way to give
//! Awareness its own model. When nothing is assigned, Awareness inherits Main so the
//! call works on any provider the user has actually configured, not just OpenRouter.
//!
//! ## Dispatch-time koma-free last resort ([`resolve_role_dispatch`])
//!
//! [`resolve_role`] alone never bricks Main/Compactor/Awareness structurally (the
//! legacy fallback always returns `Some`), but that `Some` can still be UNUSABLE —
//! an empty `api_key` — when "(inherit)" bottoms out with no user model actually
//! holding the Main role anywhere (`config.models` / `session_models`) and the old
//! per-field legacy settings were never populated either. Sending that route would
//! silently 401/fail. [`resolve_role_dispatch`] is the dispatch-time wrapper around
//! [`resolve_role`] that catches exactly this case for Main and its two cascading
//! roles (Compactor, Awareness), PLUS Safeguard (a permissive-posture override —
//! see that function's doc comment), and substitutes the keyless koma-free tier
//! instead of failing / fail-closing. It is intentionally a SEPARATE function from
//! `resolve_role` — every "is Main configured?" / "is Safeguard configured?" gate
//! keeps calling `resolve_role` + [`Resolved::is_usable`] directly and is
//! unaffected; see that function's doc comment for the full list.

use crate::app::state::AgentMode;
use crate::config::DEFAULT_BASE_URL;
use crate::model::agent_def::AgentDef;
use crate::model::app_config::{new_uuid, ApiType, AppConfig, ModelEntry, ModelRole, OAuthProvider};
use crate::model::settings::Settings;
use crate::service::koma_free::{KOMA_FREE_ENDPOINT, KOMA_FREE_MODEL};
use crate::service::openrouter::Conn;

/// One fully-resolved route for a runtime role: everything a client call needs to
/// reach the right model on the right provider, with no further config lookups.
///
/// `route` is the OpenRouter upstream-provider pin (`None` = automatic routing).
/// `effort` carries the reasoning-effort token and is only ever non-empty for the
/// Main role (the only role that exposes effort today); every other role resolves
/// it to `String::new()`.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub model_id: String,
    pub endpoint: String,
    pub api_key: String,
    // The provider's wire type. Consumed at the call boundary via
    // [`Resolved::is_routable`]: only `OpenAiCompatible` dispatches; an
    // `AnthropicCompatible` route fails/skips (native Anthropic is deferred — see
    // the `model/app_config` `ApiType` docs). The client itself never branches on
    // this; the gate lives ONE level up, here at resolution.
    pub api_type: ApiType,
    pub route: Option<String>,
    // Consumed by the MAIN streaming path (passed as the `effort` param of
    // `stream_complete`). Non-empty only for the Main role; every other role
    // resolves it to "".
    pub effort: String,
    /// Provider-specific identity carried onto the [`Conn`]: for a Codex OAuth
    /// route the ChatGPT `chatgpt-account-id`; for a Kilo OAuth route the
    /// organization id; "" for a static-key route. Filled by the OAuth
    /// resolution path (a later wave); every non-OAuth construction site sets "".
    pub account_id: String,
    /// The [`OAuthConn`](crate::model::app_config::OAuthConn) uuid this route
    /// authenticates through, threaded onto the [`Conn`] so the send-time
    /// `fresh_key` hook can refresh a near-expiry token. "" = static key (no
    /// refresh). Filled by the OAuth resolution path (a later wave).
    pub oauth_uuid: String,
    /// The stable per-install id sent as the koma-free `X-Koma` header, carried
    /// onto the [`Conn`]. Non-empty ONLY for an `ApiType::KomaFree` route (copied
    /// from `AppConfig::install_id`); "" for every other route.
    pub install_id: String,
}

impl Resolved {
    /// Borrow this route's `endpoint` + `api_key` (+ wire type + OAuth identity)
    /// as a [`Conn`] for a client call (the connection part of the route —
    /// "where + auth + how").
    pub fn conn(&self) -> Conn<'_> {
        Conn {
            endpoint: &self.endpoint,
            api_key: &self.api_key,
            api_type: self.api_type,
            account_id: &self.account_id,
            oauth_uuid: &self.oauth_uuid,
            install_id: &self.install_id,
        }
    }

    /// The OpenRouter upstream-provider pin as a slug (`""` = automatic routing),
    /// the form the client's `provider_routing_for` expects.
    pub fn provider(&self) -> &str {
        self.route.as_deref().unwrap_or("")
    }

    /// Whether this route can actually be dispatched against the OpenAI-compatible
    /// client. `false` for an `AnthropicCompatible` provider (native Anthropic is
    /// deferred — see [`ApiType`]). The call boundary checks this BEFORE dispatch:
    /// the interactive Main path emits a [`crate::service::StreamEvent::Error`] and
    /// does not POST; secondary roles (awareness / shortsend fold+router /
    /// safeguard) skip the call gracefully (no summary / no recall / fail-closed).
    pub fn is_routable(&self) -> bool {
        self.api_type.is_routable()
    }

    /// Whether this route carries usable auth — the predicate the "is there a
    /// usable Main route?" client-build / first-run gates check.
    ///
    /// A static-key or OAuth route is usable when its `api_key` (bearer /
    /// access-token) is non-empty; a [`ApiType::KomaFree`] route is KEYLESS by
    /// design (auth rides the `X-Koma`/`X-Session` headers, not `Authorization`),
    /// so it is usable with an empty key. Purely a drop-in for the old
    /// `!r.api_key.is_empty()` gate: identical for every other wire type, it only
    /// flips koma-free from "no route" to "usable" so a keyless free-tier user
    /// reaches Chat instead of being re-onboarded.
    pub fn is_usable(&self) -> bool {
        !self.api_key.is_empty() || matches!(self.api_type, ApiType::KomaFree)
    }
}

/// Find a registered [`ModelEntry`] by `uuid`, checking `settings.session_models`
/// first (per-session overrides win), then the global `config.models`. Returns
/// `None` when no entry with that uuid exists in either catalogue.
fn find_model_entry<'a>(
    config: &'a AppConfig,
    settings: &'a Settings,
    uuid: &str,
) -> Option<&'a ModelEntry> {
    settings
        .session_models
        .iter()
        .find(|e| e.uuid == uuid)
        .or_else(|| config.models.iter().find(|e| e.uuid == uuid))
}

/// Build a [`Resolved`] from an assigned [`ModelEntry`] by resolving its
/// `provider_uuid`, first against `config.providers` (a static-key provider) and
/// then — on a miss — against `config.oauth_conns` (an OAuth-backed connection:
/// Codex / Kilo Code). Returns `None` only when the `provider_uuid` matches
/// NEITHER catalogue (a dangling reference), which the caller treats the same as
/// "no assignment" and falls through to the legacy fallback.
///
/// A static-key provider carries no OAuth identity (`account_id`/`oauth_uuid`
/// empty). An OAuth-backed connection resolves its endpoint from
/// [`registry::meta`](crate::service::oauth::registry::meta), its bearer from the
/// conn's `access_token`, and threads the conn's `uuid` (for the send-time
/// `fresh_key` refresh) plus the provider-specific identity (`account_id` for
/// Codex, `org_id` for Kilo) onto the route.
///
/// `effort` is taken from `settings.effort` for the Main role AND the Planner
/// role (Planner drives the turn exactly like Main while it is active, so it
/// carries the same reasoning-effort knob); every other role gets an empty
/// effort.
fn from_entry(config: &AppConfig, settings: &Settings, entry: &ModelEntry, role: ModelRole) -> Option<Resolved> {
    let effort = if matches!(role, ModelRole::Main | ModelRole::Planner) {
        settings.effort.clone()
    } else {
        String::new()
    };
    if let Some(provider) = config.providers.iter().find(|p| p.uuid == entry.provider_uuid) {
        // koma-free: keyless dual-header transport. Route the ModelEntry names as-is
        // (no force-override), but send NO api_key and carry the stable install id for
        // the `X-Koma` header — auth rides the X-Koma/X-Session headers, not a bearer.
        if provider.api_type == ApiType::KomaFree {
            return Some(Resolved {
                model_id: entry.model_id.clone(),
                endpoint: provider.endpoint.clone(),
                api_key: String::new(),
                api_type: ApiType::KomaFree,
                route: entry.route.clone(),
                effort,
                account_id: String::new(),
                oauth_uuid: String::new(),
                install_id: config.install_id.clone(),
            });
        }
        return Some(Resolved {
            model_id: entry.model_id.clone(),
            endpoint: provider.endpoint.clone(),
            api_key: provider.api_key.clone(),
            api_type: provider.api_type,
            route: entry.route.clone(),
            effort,
            // Static-key provider route: no OAuth identity.
            account_id: String::new(),
            oauth_uuid: String::new(),
            install_id: String::new(),
        });
    }
    // Fall back to an OAuth-backed connection (Codex / Kilo Code).
    let conn = config.oauth_conns.iter().find(|c| c.uuid == entry.provider_uuid)?;
    let meta = crate::service::oauth::registry::meta(conn.provider);
    let api_type = match conn.provider {
        OAuthProvider::Codex => ApiType::Codex,
        OAuthProvider::Kilocode => ApiType::OpenAiCompatible,
        // xAI is a plain OpenAI-compatible chat endpoint (bearer JWT).
        OAuthProvider::Xai => ApiType::OpenAiCompatible,
    };
    let account_id = match conn.provider {
        OAuthProvider::Codex => conn.account_id.clone(),
        OAuthProvider::Kilocode => conn.org_id.clone(),
        // xAI has NO org/account concept — keep account_id empty. This is load-bearing:
        // the OpenAI-compatible transport only stamps `X-Kilocode-OrganizationID` when
        // account_id is non-empty, so an empty one guarantees that Kilo-only header can
        // never leak to api.x.ai.
        OAuthProvider::Xai => String::new(),
    };
    Some(Resolved {
        model_id: entry.model_id.clone(),
        endpoint: meta.chat_endpoint.to_string(),
        api_key: conn.access_token.clone(),
        api_type,
        route: entry.route.clone(),
        effort,
        account_id,
        oauth_uuid: conn.uuid.clone(),
        install_id: String::new(),
    })
}

/// The universal Main soft-fallback: a [`Resolved`] built from the OLD per-field
/// `settings` (api_key + model + provider + effort @ [`DEFAULT_BASE_URL`], the
/// OpenAI-compatible wire). Keeps chat alive when `config` is empty/old or an
/// assigned Main provider is missing, exactly preserving today's behaviour.
fn legacy_main(settings: &Settings) -> Resolved {
    Resolved {
        model_id: settings.model.clone(),
        endpoint: DEFAULT_BASE_URL.to_string(),
        api_key: settings.api_key.clone(),
        api_type: ApiType::OpenAiCompatible,
        route: None,
        effort: settings.effort.clone(),
        account_id: String::new(),
        oauth_uuid: String::new(),
        install_id: String::new(),
    }
}

/// Per-role fallback used when no model is assigned to `role`, or the assigned
/// model's provider is dangling. Only handles Main (soft-fallback to legacy
/// settings fields) and Safeguard (fail-closed). Compactor and Awareness inherit
/// the fully-resolved Main route instead — that redirect is handled in
/// [`resolve_role`] before this function is called, so neither role reaches here.
/// Planner has NO fallback at all: an unassigned or unresolved Planner returns
/// `None` here, and the caller (`resolve_turn_model`) treats that identically to
/// "no Planner assigned" — the turn simply stays on Main.
fn legacy_fallback(settings: &Settings, role: ModelRole) -> Option<Resolved> {
    match role {
        // Chat is the product — Main must never go dark.
        ModelRole::Main => Some(legacy_main(settings)),
        // Compactor and Awareness are redirected to resolve_role(Main) before
        // reaching here; these arms are unreachable in practice but kept for
        // exhaustiveness.
        ModelRole::Compactor | ModelRole::Awareness => Some(legacy_main(settings)),
        // Planner has no legacy fallback by design — unassigned means "use Main".
        ModelRole::Planner => None,
        ModelRole::Safeguard => {
            // FAIL-CLOSED: only the legacy classifier model rescues it; an empty
            // field yields `None`, which the harness caller degrades to a human
            // prompt (TAC) / advisory toast (PC) rather than silently allowing.
            // Also fail-closed when `settings.api_key` is empty: this legacy path
            // always builds an OpenAiCompatible route (never KomaFree), so an
            // empty key means a keyless koma-free ("/free") user with no legacy
            // classifier connection configured — sending that route would POST
            // `Authorization: Bearer ` (empty) and 401 on every classify call.
            // Treat "classifier model set but no key to call it with" the same as
            // "not actually configured" rather than firing a doomed request.
            if settings.classifier_model.is_empty() || settings.api_key.is_empty() {
                None
            } else {
                Some(Resolved {
                    model_id: settings.classifier_model.clone(),
                    endpoint: DEFAULT_BASE_URL.to_string(),
                    api_key: settings.api_key.clone(),
                    api_type: ApiType::OpenAiCompatible,
                    route: None,
                    effort: String::new(),
                    account_id: String::new(),
                    oauth_uuid: String::new(),
                    install_id: String::new(),
                })
            }
        }
    }
}

/// Resolve the concrete route for `role`.
///
/// Session overrides (`settings.session_models`) win over the global catalogue
/// (`config.models`); the chosen model's provider is resolved by uuid against
/// `config.providers`. A successful resolution returns the assigned route. When no
/// model is assigned, or the assigned model's provider is dangling, the per-role
/// legacy fallback applies (see [`legacy_fallback`]). Returns `None` only for an
/// unresolved Safeguard (fail-closed); every other role always resolves to
/// `Some`.
pub fn resolve_role(config: &AppConfig, settings: &Settings, role: ModelRole) -> Option<Resolved> {
    // 1. Pick the assigned model: per-session overrides first, then the global
    //    catalogue. A model may hold several roles, so match on whether its
    //    effective role set CONTAINS `role` (this also folds the legacy
    //    single-role field in via `effective_roles`).
    let assigned = settings
        .session_models
        .iter()
        .find(|e| e.effective_roles().contains(&role))
        .or_else(|| config.models.iter().find(|e| e.effective_roles().contains(&role)));

    // 2. If a model is assigned AND its provider resolves, that route wins —
    //    including an explicitly-assigned Awareness model (explicit assignment is
    //    the only way to give Awareness its own dedicated model).
    if let Some(entry) = assigned {
        if let Some(resolved) = from_entry(config, settings, entry, role) {
            return Some(resolved);
        }
        // Assigned but the provider_uuid is dangling → fall through.
    }

    // 3. Compactor and Awareness have no config slot of their own — both inherit
    //    the FULLY-RESOLVED Main route (which honours config.models Main + its
    //    provider connection's real endpoint/key). This must happen here, where
    //    `config` is in scope, NOT inside `legacy_fallback`, which only has
    //    `settings` and would wrongly hard-code DEFAULT_BASE_URL (OpenRouter).
    //    No infinite recursion: Main never resolves to Compactor or Awareness.
    if matches!(role, ModelRole::Compactor | ModelRole::Awareness) {
        return resolve_role(config, settings, ModelRole::Main);
    }

    // 4. No assignment, or a dangling provider → per-role legacy fallback.
    legacy_fallback(settings, role)
}

/// Build the keyless koma-free [`Resolved`] directly (no [`ModelEntry`] /
/// [`crate::model::app_config::ProviderConn`] involved), mirroring the
/// `ApiType::KomaFree` special-case in [`from_entry`]: `KOMA_FREE_ENDPOINT` +
/// `KOMA_FREE_MODEL`, an empty `api_key` (auth rides the `X-Koma`/`X-Session`
/// headers), no upstream route pin. `effort` follows the same rule
/// `from_entry` uses — `settings.effort` for `role == Main` (the only role that
/// exposes it), empty for every other role — so a Main turn that falls back to
/// koma-free doesn't silently lose the user's configured reasoning effort
/// versus an explicitly-configured koma-free Main entry.
///
/// `install_id` is normally `config.install_id` as-is. This function NEVER
/// mutates or persists `config` (it only borrows it) — so on the rare install
/// where `install_id` is still empty (the user has never touched `/free` or the
/// koma-free onboarding path, both of which mint+save one), an empty `X-Koma`
/// header would go out on the wire. Rather than persist a value from a pure
/// resolve-time read path, mint an EPHEMERAL uuid for this `Resolved` only; it
/// is not written back to `config`, so a later real `/free` toggle or koma-free
/// onboarding still mints (and this time persists) the real one.
fn koma_free_dispatch_route(config: &AppConfig, settings: &Settings, role: ModelRole) -> Resolved {
    let install_id = if config.install_id.is_empty() {
        new_uuid()
    } else {
        config.install_id.clone()
    };
    let effort = if role == ModelRole::Main {
        settings.effort.clone()
    } else {
        String::new()
    };
    Resolved {
        model_id: KOMA_FREE_MODEL.to_string(),
        endpoint: KOMA_FREE_ENDPOINT.to_string(),
        api_key: String::new(),
        api_type: ApiType::KomaFree,
        route: None,
        effort,
        account_id: String::new(),
        oauth_uuid: String::new(),
        install_id,
    }
}

/// [`resolve_role`], but with a LAST-RESORT koma-free fallback for Main and every
/// role that CASCADES to Main (`Compactor`, `Awareness` — see the `resolve_role`
/// fallback table above), PLUS `Safeguard`: when the resolved route is missing or
/// [`Resolved::is_usable`] says it carries no usable auth, dispatch against the
/// keyless koma-free tier instead of sending a doomed empty-key request (Main /
/// Compactor / Awareness) or silently degrading every risky tool call to a human
/// prompt (Safeguard).
///
/// PERMISSIVE POSTURE (owner override): koma-free now powers every runtime role,
/// Safeguard included. The original invariant here was "never downgrade the
/// classifier to a free tier" — deliberately fail-closed, on the theory that an
/// unverified free-tier model auto-allowing risky tool calls was worse than
/// falling back to a human prompt. That trade-off flips once keyless koma-free is
/// the default onboarding path: a keyless user with no classifier configured hit
/// `resolve_role(Safeguard) == None` on EVERY risky tool call, which the harness
/// (`harness::classify`) degrades to a human approval prompt in BOTH agent modes —
/// silently defeating Auto mode's entire pitch (no prompts) for every free-tier
/// user. Routing Safeguard through koma-free instead keeps Auto mode usable for
/// keyless users; the harness's unavailable→human-prompt path REMAINS the
/// backstop for genuine failures (the koma-free call itself errors, times out, or
/// returns something unparseable), so a broken classifier still degrades safely —
/// only the "not configured at all" case is upgraded from fail-closed to
/// free-tier. Planner is still untouched — it already degrades to Main at the
/// call site ([`resolve_turn_model`]) when unresolved, so it never needs its own
/// fallback here.
///
/// DISPATCH-TIME ONLY. Do NOT call this from a "is anything configured yet?"
/// GATE — every such gate (the first-run chooser in
/// `runtime::lifecycle::build_startup`/`install_daemon_session`, the
/// client-build gate right next to it, and the no-creds banners in
/// `runtime::actions::onboard`/`session::{attach,cancel,picker}`/
/// `commands::new_session`) calls [`resolve_role`] directly and MUST keep
/// observing "not usable" so onboarding still fires for a genuinely
/// unconfigured install — this function would make that check always pass and
/// silently swallow the gate. Reserve it for the seam where a network request is
/// actually about to be built (the Main turn, the Awareness fold/summary call,
/// `/compact`'s Compactor call, their Main-route retry fallbacks, and the
/// Safeguard classifier call).
pub fn resolve_role_dispatch(config: &AppConfig, settings: &Settings, role: ModelRole) -> Option<Resolved> {
    let resolved = resolve_role(config, settings, role);
    if matches!(
        role,
        ModelRole::Main | ModelRole::Compactor | ModelRole::Awareness | ModelRole::Safeguard
    ) && resolved.as_ref().is_none_or(|r| !r.is_usable())
    {
        return Some(koma_free_dispatch_route(config, settings, role));
    }
    resolved
}

/// Why a Main turn is being silently downgraded to the keyless koma-free tier by
/// [`resolve_role_dispatch`] — the diagnosis a user-facing "you're on the free
/// tier" toast needs. Each variant is a DISTINCT, user-actionable state of the
/// assigned Main model whose route came back unusable:
///
/// - `ProviderRemoved` — the assigned model still points at a `provider_uuid` that
///   exists in NEITHER `providers` nor `oauth_conns` (its connection was deleted).
/// - `NoKey` — the assigned model's static provider exists but its `api_key` is empty.
/// - `NotSignedIn` — the assigned model's provider is an OAuth connection whose
///   `access_token` is empty (never signed in / signed out / token cleared).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFallback {
    ProviderRemoved,
    NoKey,
    NotSignedIn,
}

/// Diagnose whether the CURRENT Main route will be silently downgraded to koma-free
/// by [`resolve_role_dispatch`], and if so WHY — so the dispatch seam (`stream::run`)
/// can surface a toast instead of swapping to the free tier with zero indication.
/// Read-only: never mutates or persists `config`/`settings`.
///
/// Mirrors [`resolve_role`]'s step-1 assignment lookup EXACTLY (session overrides
/// win over the global catalogue) and its usability gate:
///
/// - No model holds the Main role → `None`. This is the unconfigured / onboarding
///   install, whose dispatch fallback routes the user to the first-run chooser, NOT
///   a toast (see [`resolve_role_dispatch`]'s "do NOT call from a gate" note) — so
///   warning here would be wrong.
/// - The assigned Main resolves to a USABLE route — a real key, a live OAuth token,
///   or a keyless koma-free provider the user chose themselves → `None`. Nothing is
///   downgraded (koma-free either isn't involved or WAS the deliberate choice).
/// - The assigned Main resolves to an UNUSABLE route (koma-free will substitute) →
///   `Some(reason)`, attributed off the assigned entry's `provider_uuid` with
///   `from_entry`'s providers-before-oauth precedence: matches a static provider →
///   `NoKey`; matches an OAuth conn → `NotSignedIn`; matches neither → `ProviderRemoved`.
pub fn main_fallback_reason(config: &AppConfig, settings: &Settings) -> Option<MainFallback> {
    // 1. The Main-assigned entry, resolved with the SAME precedence as
    //    `resolve_role` step 1: per-session overrides first, then the global
    //    catalogue. Nothing holds Main → unconfigured install → never warn (that
    //    path is onboarding, not a silent koma-free swap).
    let assigned = settings
        .session_models
        .iter()
        .find(|e| e.effective_roles().contains(&ModelRole::Main))
        .or_else(|| {
            config
                .models
                .iter()
                .find(|e| e.effective_roles().contains(&ModelRole::Main))
        })?;

    // 2. koma-free substitutes iff `resolve_role(Main)` is missing or unusable —
    //    the exact gate `resolve_role_dispatch` applies. A usable route (real key,
    //    live OAuth token, or a keyless koma-free the user selected) means NO silent
    //    downgrade, so there is nothing to warn about.
    if resolve_role(config, settings, ModelRole::Main).is_some_and(|r| r.is_usable()) {
        return None;
    }

    // 3. Downgrade confirmed. Attribute the reason off the assigned entry's
    //    provider_uuid, mirroring `from_entry`'s providers-before-oauth order.
    //    Reaching here means the resolved route was unusable, so a matching static
    //    provider necessarily has an empty `api_key`, and a matching OAuth conn an
    //    empty `access_token` (a populated credential would have resolved usable at
    //    step 2 and returned `None`).
    if config.providers.iter().any(|p| p.uuid == assigned.provider_uuid) {
        Some(MainFallback::NoKey)
    } else if config.oauth_conns.iter().any(|c| c.uuid == assigned.provider_uuid) {
        Some(MainFallback::NotSignedIn)
    } else {
        Some(MainFallback::ProviderRemoved)
    }
}

/// True when `a` and `b` name the exact same route: same model id, same
/// provider endpoint, and the same OpenRouter upstream pin. Deliberately does
/// NOT compare `api_key`/`api_type`/`effort` — those can never differ for two
/// entries that already agree on model+endpoint+route (same provider
/// connection), and effort is set from `settings.effort` identically for both
/// Main and Planner (see `from_entry`).
fn same_route(a: &Resolved, b: &Resolved) -> bool {
    a.model_id == b.model_id && a.endpoint == b.endpoint && a.route == b.route
}

/// Resolve the model that should drive the CURRENT main turn, honouring
/// Plan-mode's dedicated Planner role.
///
/// This is a pure PER-TURN decision — there is no swap state to track: the
/// instant `mode` leaves `AgentMode::Plan`, the very next call resolves Main
/// again automatically.
///
/// - `mode != AgentMode::Plan`: always Main (today's behaviour, untouched).
/// - `mode == AgentMode::Plan` and no Planner assigned (or its provider is
///   dangling): Main, unchanged.
/// - `mode == AgentMode::Plan` and Planner resolves to the EXACT same route as
///   Main (model id + endpoint + upstream route): return Main's `Resolved`
///   unchanged rather than a structurally-identical copy — this preserves the
///   provider's prompt-cache continuity (the request keeps flowing through the
///   same `Resolved` value the rest of the turn already threads through).
/// - Otherwise: Planner's `Resolved` drives the turn — callers must read
///   effort/endpoint/model id/image-capability etc. off the RETURNED value,
///   never off a separately-resolved Main.
///
/// Returns `None` only when Main itself can't resolve (in practice never,
/// since Main has a legacy soft-fallback, and now also the koma-free
/// last-resort in [`resolve_role_dispatch`]).
///
/// Main is resolved via [`resolve_role_dispatch`], NOT [`resolve_role`] — this
/// is the actual per-turn DISPATCH chokepoint (the route this function returns
/// is what `stream::run` sends the request on), so an unusable Main (no user
/// model holds the role, or its provider is dangling) falls back to koma-free
/// here rather than shipping an empty-key request. This does not affect any
/// "is Main configured?" gate: those call [`resolve_role`] directly (see
/// [`resolve_role_dispatch`]'s doc comment).
pub fn resolve_turn_model(config: &AppConfig, settings: &Settings, mode: AgentMode) -> Option<Resolved> {
    let main = resolve_role_dispatch(config, settings, ModelRole::Main)?;
    if mode != AgentMode::Plan {
        return Some(main);
    }
    match resolve_role(config, settings, ModelRole::Planner) {
        Some(planner) if !same_route(&planner, &main) => Some(planner),
        _ => Some(main),
    }
}

/// Resolve the concrete route for a sub-agent ([`AgentDef`]).
///
/// A sub-agent carries its OWN model + provider on the definition, independent of
/// the runtime role catalogue:
///
/// 1. If the agent names a `model` AND its `provider_uuid` resolves to a known
///    provider connection, dispatch against THAT provider (endpoint + key + wire
///    type), pinning the agent's legacy `provider` routing slug as the upstream
///    route. This is the explicit-assignment path and always wins.
/// 2. Otherwise — the agent has no model, or its `provider_uuid` is absent /
///    dangling — inherit the fully-resolved Main route so the sub-agent runs on
///    whatever provider the user has actually configured (never silently dark).
///
/// In BOTH cases the agent's own reasoning `effort` overrides the route's effort
/// when set (an agent declares its own thinking budget); an unset effort keeps the
/// inherited one. Returns `None` only when the agent has no usable model AND Main
/// itself can't resolve — practically never, since Main has a legacy soft-fallback.
///
/// True when `agent` declares its own model (model_uuid or legacy model field)
/// that is non-empty. Does NOT test whether the model actually resolves; use
/// [`agent_model_resolves`] for that. This is the "did the agent author set a
/// model at all" predicate used to decide whether a fallback-to-Main is
/// surprising (declared but unresolvable) or expected (no model declared).
pub fn agent_declares_model(agent: &AgentDef) -> bool {
    agent.model_uuid.as_deref().is_some_and(|u| !u.trim().is_empty())
        || agent.model.as_deref().is_some_and(|m| !m.trim().is_empty())
}

/// True when `agent`'s declared model resolves to a concrete (non-Main-fallback)
/// route. Returns false both when the agent declares NO model and when it declares
/// one that can't be resolved (deleted entry / dangling provider / stale
/// session_models). The caller pairs this with [`agent_declares_model`]: warn when
/// the agent declared a model but this returns false.
pub fn agent_model_resolves(config: &AppConfig, settings: &Settings, agent: &AgentDef) -> bool {
    // 1. Registered model uuid → resolvable entry+provider.
    if let Some(uuid) = agent.model_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
        if let Some(entry) = find_model_entry(config, settings, uuid) {
            if from_entry(config, settings, entry, ModelRole::Main).is_some() {
                return true;
            }
        }
        // uuid unresolved (deleted entry / dangling provider) → fall through to legacy.
    }
    // 2. Legacy explicit model + resolvable provider connection.
    if let Some(model_id) = agent.model.as_deref().filter(|m| !m.trim().is_empty()) {
        let _ = model_id;
        if let Some(uuid) = agent.provider_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
            if config.providers.iter().any(|p| p.uuid == uuid) {
                return true;
            }
        }
    }
    false
}

/// Currently only called by the (Stage-1 inert) sub-agent spawn path, so it is
/// unreferenced from the binary until that path is wired in — hence the allow.
#[allow(dead_code)]
pub fn resolve_agent(config: &AppConfig, settings: &Settings, agent: &AgentDef) -> Option<Resolved> {
    // The agent's declared effort, applied on top of whichever route we land on.
    let agent_effort = agent.effort.clone();
    let with_effort = |mut r: Resolved| -> Resolved {
        if let Some(e) = &agent_effort {
            r.effort = e.clone();
        }
        r
    };

    // 1a. Registered model uuid → look up the ModelEntry and resolve via from_entry.
    //     A uuid that no longer exists (deleted entry) falls through gracefully.
    if let Some(uuid) = agent.model_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
        if let Some(entry) = find_model_entry(config, settings, uuid) {
            if let Some(resolved) = from_entry(config, settings, entry, ModelRole::Main) {
                return Some(with_effort(resolved));
            }
            // Entry found but its provider is dangling → fall through to legacy.
        }
        // uuid present but no matching entry (deleted) → fall through.
    }

    // 1b. Legacy explicit model + resolvable provider connection → dispatch there.
    if let Some(model_id) = agent.model.as_deref().filter(|m| !m.trim().is_empty()) {
        if let Some(uuid) = agent.provider_uuid.as_deref().filter(|u| !u.trim().is_empty()) {
            if let Some(provider) = config.providers.iter().find(|p| p.uuid == uuid) {
                return Some(with_effort(Resolved {
                    model_id: model_id.to_string(),
                    endpoint: provider.endpoint.clone(),
                    api_key: provider.api_key.clone(),
                    api_type: provider.api_type,
                    // The legacy free-text `provider` field is an OpenRouter
                    // upstream-routing slug (None = automatic routing).
                    route: agent.provider.clone(),
                    effort: String::new(),
                    account_id: String::new(),
                    oauth_uuid: String::new(),
                    install_id: String::new(),
                }));
            }
        }
        // A named model whose provider is absent/dangling falls through to Main —
        // better to run on the configured Main provider than to go dark.
    }

    // 2. No usable model/provider → inherit the Main route.
    resolve_role(config, settings, ModelRole::Main).map(with_effort)
}

#[cfg(test)]
#[path = "resolve_test.rs"]
mod tests;

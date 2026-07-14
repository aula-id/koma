//! W13 additional regression suite for `resolve.rs`'s ext-preferred slug-binding seam
//! (`find_model_entry_by_slug` + `ext_preferred_provider_uuids`) — PURE ADDITION, a SEPARATE
//! sibling module from the existing `resolve_test.rs` (never touched here; its own helpers
//! like `ext_model_conn` are private to that module and not reachable from this file, so a
//! small local duplicate is built below).
//!
//! Explicitly SKIPPED as already fully covered in `resolve_test.rs`:
//! - `find_model_entry_by_slug` matching by `model_id`/`name`/`uuid`, all case-insensitive
//!   (`find_model_entry_by_slug_matches_by_model_id_name_uuid_case_insensitive` already proves
//!   ALL THREE identities are case-insensitive uniformly — there is no uuid-exact-case vs
//!   model_id-case-insensitive asymmetry in the actual `slug_matches` implementation);
//! - session_models beating config.models on an UNPREFERRED lookup
//!   (`find_model_entry_by_slug_session_models_win_over_global`);
//! - a preferred set beating the earlier general-scan match
//!   (`find_model_entry_by_slug_preferred_provider_wins_over_earlier_general_match`);
//! - a single ext-owned conn's model out-binding a same-named global entry, and an ext with NO
//!   conn falling through to the general pass
//!   (`ext_agent_binds_to_its_own_model_over_same_named_global`).
//!
//! Gaps targeted here:
//! - `ext_preferred_provider_uuids` collecting MULTIPLE oauth conns for the SAME extension, and
//!   the preferred-pass matching against EITHER of them;
//! - within the preferred pass itself, a SESSION-scoped preferred match must beat a
//!   GLOBAL-scoped preferred match (mirroring the unprefixed session-vs-global ordering, but
//!   proven specifically inside the `Some(preferred)` branch, which the existing tests never
//!   combine with `session_models`).

use super::*;

/// An ext-backed [`OAuthConn`] carrying the W12 model-provider meta, duplicated from
/// `resolve_test.rs::ext_model_conn` (that helper is private to its own sibling module).
fn ext_model_conn(uuid: &str, ext_id: &str, endpoint: &str, api_type: &str) -> OAuthConn {
    OAuthConn {
        uuid: uuid.to_string(),
        provider: OAuthProvider::Extension,
        access_token: "ext-bearer".to_string(),
        ext_id: Some(ext_id.to_string()),
        provider_id: Some("prov".to_string()),
        chat_endpoint: Some(endpoint.to_string()),
        api_type: Some(api_type.to_string()),
        ..Default::default()
    }
}

/// `ext_preferred_provider_uuids` collects EVERY oauth conn owned by the agent's extension —
/// not just the first — and the preferred pass in `find_model_entry_by_slug` matches against
/// EITHER of them (a model served by the SECOND conn is found just as readily as one served by
/// the first).
#[test]
fn ext_preferred_provider_uuids_collects_multiple_conns_and_either_matches() {
    let mut config = AppConfig::default();
    config
        .oauth_conns
        .push(ext_model_conn("conn-1", "my.ext", "https://api.one.test/v1", "openai"));
    config
        .oauth_conns
        .push(ext_model_conn("conn-2", "my.ext", "https://api.two.test/v1", "anthropic"));

    let agent = AgentDef { ext_id: Some("my.ext".to_string()), ..AgentDef::default() };
    let preferred = ext_preferred_provider_uuids(&config, &agent).expect("ext agent has a preferred set");
    assert_eq!(preferred.len(), 2, "both of the extension's conns must be collected");
    assert!(preferred.contains("conn-1"));
    assert!(preferred.contains("conn-2"));

    // A global "fast" served by conn-2 only — the preferred pass must still find it even
    // though conn-1 (a DIFFERENT owned conn) doesn't serve it.
    config.models.push(ModelEntry {
        uuid: "global-fast".to_string(),
        name: "fast".to_string(),
        model_id: "global/fast-model".to_string(),
        provider_uuid: "prov-unrelated".to_string(),
        ..ModelEntry::default()
    });
    config.models.push(ModelEntry {
        uuid: "on-conn-2".to_string(),
        name: "fast".to_string(),
        model_id: "vendor/fast-on-two".to_string(),
        provider_uuid: "conn-2".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings::default();
    let hit = find_model_entry_by_slug(&config, &settings, "fast", Some(&preferred))
        .expect("matches via the second owned conn");
    assert_eq!(hit.uuid, "on-conn-2", "the preferred pass must match against ANY owned conn, not just the first");
}

/// Inside the preferred pass itself: when BOTH `settings.session_models` AND `config.models`
/// carry a slug+preferred-provider match, the SESSION-scoped one wins — the same session-first
/// ordering the general (unprefixed) pass uses, proven here specifically within the
/// `Some(preferred)` branch.
#[test]
fn preferred_pass_session_scoped_match_beats_global_scoped_match() {
    let mut config = AppConfig::default();
    config.models.push(ModelEntry {
        uuid: "global-preferred".to_string(),
        name: "shared".to_string(),
        model_id: "vendor/global-preferred".to_string(),
        provider_uuid: "ext-conn".to_string(),
        ..ModelEntry::default()
    });
    let settings = Settings {
        session_models: vec![ModelEntry {
            uuid: "session-preferred".to_string(),
            name: "shared".to_string(),
            model_id: "vendor/session-preferred".to_string(),
            provider_uuid: "ext-conn".to_string(),
            ..ModelEntry::default()
        }],
        ..Default::default()
    };

    let mut preferred = std::collections::HashSet::new();
    preferred.insert("ext-conn".to_string());

    let hit = find_model_entry_by_slug(&config, &settings, "shared", Some(&preferred))
        .expect("matches");
    assert_eq!(
        hit.uuid, "session-preferred",
        "within the preferred pass, a session-scoped match must beat a global-scoped one"
    );
}

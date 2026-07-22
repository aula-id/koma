//! W13 additional regression suite for `app_config.rs` — PURE ADDITION alongside the existing
//! inline `oauth_provider_wire_tests` / `oauth_conn_serde_tests` / `provider_conn_serde_tests`
//! / `ext_purge_tests` modules in that file (none of them touched here).
//!
//! Explicitly SKIPPED as already fully covered inline:
//! - `purge_extension`'s matrix (2 extensions + native providers/models/conns fixture; exactly
//!   the owned set removed via BOTH anchor kinds — key-backed provider AND oauth conn — the
//!   preferred-model record cleared, natives byte-untouched) —
//!   `ext_purge_tests::purge_removes_ext_entries_and_reports_main_reset` already builds exactly
//!   this fixture and asserts every one of those facts, plus `main_reset` semantics
//!   (`purge_without_main_holder_does_not_flag_reset` for the negative case);
//! - `ProviderConn`/`OAuthConn` serde byte-compat extension (a native JSON blob with none of
//!   the new fields round-trips byte-identically) — `oauth_conn_serde_tests::
//!   native_conn_roundtrips_byte_stable` and `provider_conn_serde_tests::
//!   native_provider_roundtrips_byte_stable` already pin this exactly;
//! - `OAuthConn::ext_model_route`'s api_type valid/invalid/missing matrix —
//!   `oauth_conn_serde_tests::ext_model_route_gates_on_endpoint_and_api_type` already probes
//!   openai/anthropic/missing-endpoint/missing-api_type/blank-endpoint/unrecognised-api_type.
//!
//! Gap targeted here: `AppConfig::upsert_model` / the underlying `upsert_model_entry` +
//! `strip_role` role-steal semantics — the exact mechanism `try_vacuum_fill_main`
//! (`app::ext::broker`) relies on to promote an extension's preferred model to Main, and
//! nothing in the crate unit-tests it directly before this file.

use super::*;

/// A brand-new `ModelEntry` with an EMPTY uuid is minted a fresh one and simply appended —
/// no other entry exists yet to steal a role from.
#[test]
fn upsert_model_entry_mints_uuid_for_brand_new_entry() {
    let mut list: Vec<ModelEntry> = Vec::new();
    upsert_model_entry(
        &mut list,
        ModelEntry {
            uuid: String::new(),
            name: "Fresh".to_string(),
            model_id: "vendor/fresh".to_string(),
            roles: vec![ModelRole::Main],
            ..ModelEntry::default()
        },
    );
    assert_eq!(list.len(), 1);
    assert!(
        !list[0].uuid.is_empty(),
        "an empty incoming uuid must be minted fresh"
    );
    assert_eq!(list[0].effective_roles(), vec![ModelRole::Main]);
}

/// Per-role STEAL: assigning Main to entry B strips Main from entry A (which held it via the
/// modern `roles` vec) — the invariant every role is held by AT MOST one model. A's OTHER role
/// (Awareness) is left untouched; only the STOLEN role is removed.
#[test]
fn upsert_model_entry_steals_role_from_roles_vec_leaving_other_roles_intact() {
    let mut list = vec![ModelEntry {
        uuid: "a".to_string(),
        model_id: "vendor/a".to_string(),
        roles: vec![ModelRole::Main, ModelRole::Awareness],
        ..ModelEntry::default()
    }];
    upsert_model_entry(
        &mut list,
        ModelEntry {
            uuid: "b".to_string(),
            model_id: "vendor/b".to_string(),
            roles: vec![ModelRole::Main],
            ..ModelEntry::default()
        },
    );
    assert_eq!(list.len(), 2);
    let a = list.iter().find(|m| m.uuid == "a").expect("a survives");
    assert_eq!(
        a.effective_roles(),
        vec![ModelRole::Awareness],
        "Main is stolen; Awareness survives"
    );
    let b = list.iter().find(|m| m.uuid == "b").expect("b inserted");
    assert_eq!(b.effective_roles(), vec![ModelRole::Main]);
}

/// Per-role STEAL also clears the LEGACY single-`role` field when it equals the stolen role —
/// without this dual clear, `effective_roles` would still fold the legacy field in (since
/// `roles` is empty on that old-style entry) and the first-wins resolver could still find the
/// stale holder. Mirrors the doc comment on `strip_role` directly.
#[test]
fn upsert_model_entry_steals_role_from_legacy_role_field_too() {
    let mut list = vec![ModelEntry {
        uuid: "legacy".to_string(),
        model_id: "vendor/legacy".to_string(),
        roles: Vec::new(),
        role: Some(ModelRole::Main), // old-style single-role holder
        ..ModelEntry::default()
    }];
    assert_eq!(
        list[0].effective_roles(),
        vec![ModelRole::Main],
        "legacy field folds in via effective_roles"
    );

    upsert_model_entry(
        &mut list,
        ModelEntry {
            uuid: "new".to_string(),
            model_id: "vendor/new".to_string(),
            roles: vec![ModelRole::Main],
            ..ModelEntry::default()
        },
    );

    let legacy = list
        .iter()
        .find(|m| m.uuid == "legacy")
        .expect("legacy entry survives");
    assert!(
        legacy.effective_roles().is_empty(),
        "the legacy role field must be cleared, not just shadowed"
    );
    assert_eq!(
        legacy.role, None,
        "strip_role must null the legacy field directly"
    );
}

/// Re-upserting an entry by its OWN uuid updates it in place (replace-by-uuid), and does NOT
/// treat itself as a role-theft target — an entry re-saved with the SAME role it already held
/// must not strip that role from itself.
#[test]
fn upsert_model_entry_updates_in_place_by_uuid_without_self_theft() {
    let mut list = vec![ModelEntry {
        uuid: "m".to_string(),
        name: "Old Name".to_string(),
        model_id: "vendor/m".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    }];
    upsert_model_entry(
        &mut list,
        ModelEntry {
            uuid: "m".to_string(),
            name: "New Name".to_string(),
            model_id: "vendor/m".to_string(),
            roles: vec![ModelRole::Main],
            ..ModelEntry::default()
        },
    );
    assert_eq!(
        list.len(),
        1,
        "same uuid must update in place, never append a duplicate"
    );
    assert_eq!(list[0].name, "New Name");
    assert_eq!(
        list[0].effective_roles(),
        vec![ModelRole::Main],
        "re-saving with the same role keeps it"
    );
}

/// `AppConfig::upsert_model` (the public wrapper `try_vacuum_fill_main` calls) delegates to the
/// exact same steal semantics against `config.models` — an end-to-end proof at the public API
/// surface, not just the private helper.
#[test]
fn app_config_upsert_model_delegates_role_steal_to_global_catalogue() {
    let mut config = AppConfig::default();
    config.models.push(ModelEntry {
        uuid: "old-main".to_string(),
        model_id: "vendor/old".to_string(),
        roles: vec![ModelRole::Main],
        ..ModelEntry::default()
    });
    config.models.push(ModelEntry {
        uuid: "other".to_string(),
        model_id: "vendor/other".to_string(),
        roles: vec![ModelRole::Awareness],
        ..ModelEntry::default()
    });

    // Mirrors `try_vacuum_fill_main`: clone the target entry, add Main, upsert.
    let mut promoted = config
        .models
        .iter()
        .find(|m| m.uuid == "other")
        .unwrap()
        .clone();
    promoted.roles.push(ModelRole::Main);
    config.upsert_model(promoted);

    let old = config.models.iter().find(|m| m.uuid == "old-main").unwrap();
    assert!(
        old.effective_roles().is_empty(),
        "the previous Main holder must lose the role"
    );
    let other = config.models.iter().find(|m| m.uuid == "other").unwrap();
    assert_eq!(
        other.effective_roles(),
        vec![ModelRole::Awareness, ModelRole::Main],
        "the promoted entry keeps its existing role AND gains Main"
    );
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

#[test]
fn bundled_default_parses() {
    let raw = BUNDLED_DEFAULT.trim();
    if raw.is_empty() {
        // Shipped catalogue is empty until the release asset / OA refresh lands.
        return;
    }
    serde_json::from_str::<OverlayTable>(raw).expect("bundled models.json must parse");
}

#[test]
fn lookup_unknown_returns_none() {
    let _ = OVERLAY.set(RwLock::new(OverlayState {
        table: load_initial(),
        ..OverlayState::default()
    }));
    assert!(lookup("https://not-a-real-endpoint", "nope").is_none());
}

#[test]
fn catalogue_key_separates_commandcode_transport() {
    use crate::service::oauth::registry::{COMMANDCODE_API_BASE, COMMANDCODE_CHAT_BASE};
    for chat in [COMMANDCODE_CHAT_BASE, COMMANDCODE_API_BASE] {
        assert_eq!(
            catalogue_key(chat, Some(OAuthProvider::CommandCode)),
            COMMANDCODE_API_BASE
        );
    }
    assert_eq!(
        catalogue_key("https://openrouter.ai/api/v1", None),
        "https://openrouter.ai/api/v1"
    );
    assert_eq!(
        catalogue_key("https://direct.example/v1", None),
        "https://direct.example/v1"
    );
    assert_eq!(
        catalogue_key(
            "https://api.kilo.ai/api/openrouter",
            Some(OAuthProvider::Kilocode)
        ),
        "https://api.kilo.ai/api/gateway"
    );
    assert_eq!(
        catalogue_key("https://api.anthropic.com", Some(OAuthProvider::ClaudeAI)),
        "https://api.anthropic.com"
    );
}

fn fixture(id: &str, efforts: &[&str]) -> OverlayModel {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "reasoning": { "mandatory": false, "supported_efforts": efforts }
    }))
    .unwrap()
}

#[test]
fn detail_upserts_only_one_model() {
    let mut table = OverlayTable::new();
    table.insert(
        "cc".into(),
        vec![fixture("haiku", &[]), fixture("other", &["low"])],
    );
    upsert_model(&mut table, "cc", fixture("haiku", &["max", "high"]));
    assert_eq!(table["cc"].len(), 2);
    let info: Vec<_> = table["cc"]
        .iter()
        .map(OverlayModel::to_model_info)
        .collect();
    let caps = crate::service::openrouter::effort_caps(&info, "haiku");
    assert!(caps.supported);
    assert_eq!(caps.efforts, ["max", "high"]);
    upsert_model(&mut table, "cc", fixture("new", &["medium"]));
    assert_eq!(table["cc"].len(), 3);
}

#[test]
fn oa_and_details_survive_base_refresh_in_either_order() {
    let mut base = OverlayTable::new();
    base.insert("direct".into(), vec![fixture("deepseek", &["high"])]);
    base.insert("cc".into(), vec![fixture("haiku", &[])]);
    let mut oa = OverlayTable::new();
    oa.insert(
        "cc".into(),
        vec![fixture("haiku", &["low"]), fixture("other", &["high"])],
    );
    for oa_first in [true, false] {
        let mut state = OverlayState::default();
        if oa_first {
            state.merge_oa(oa.clone());
            state.replace_base(base.clone());
        } else {
            state.replace_base(base.clone());
            state.merge_oa(oa.clone());
        }
        upsert_model(
            &mut state.details,
            "cc",
            fixture("haiku", &["max", "xhigh"]),
        );
        state.apply_details();
        state.replace_base(base.clone());
        state.merge_oa(oa.clone());
        assert_eq!(state.table["direct"], base["direct"]);
        assert_eq!(state.table["cc"].len(), 2);
        assert_eq!(state.table["cc"][0], fixture("haiku", &["max", "xhigh"]));
    }
}

#[test]
fn oauth_picker_excludes_bundled_and_detail_only_models() {
    let provider = OAuthProvider::Codex;
    let key = catalogue_key("", Some(provider));
    let mut state = OverlayState::default();
    state.replace_base(HashMap::from([(
        key.clone(),
        vec![fixture("bundled", &[])],
    )]));
    assert!(oa_membership(&state.oa, provider).is_empty());
    state.merge_oa(HashMap::from([(
        key.clone(),
        vec![fixture("public", &["high"])],
    )]));
    upsert_model(&mut state.details, &key, fixture("detail-only", &["low"]));
    state.apply_details();
    state.replace_base(HashMap::from([(
        key.clone(),
        vec![fixture("bundled", &[])],
    )]));
    let models = oa_membership(&state.oa, provider);
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["public"]
    );
    state.merge_oa(HashMap::from([(key, Vec::new())]));
    assert!(oa_membership(&state.oa, provider).is_empty());
}

#[test]
fn oauth_picker_merges_only_public_koma_tiers() {
    use crate::service::oauth::registry;
    let table = HashMap::from([
        (
            registry::meta(OAuthProvider::KomaRun)
                .chat_endpoint
                .to_string(),
            vec![fixture("free", &[]), fixture("shared", &[])],
        ),
        (
            registry::KOMA_PREMIUM_CHAT_ENDPOINT.to_string(),
            vec![fixture("premium", &[]), fixture("shared", &[])],
        ),
    ]);
    let models = oa_membership(&table, OAuthProvider::KomaRun);
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["premium", "shared", "free"]
    );
}

fn overlay_model(id: &str) -> OverlayModel {
    OverlayModel {
        id: id.to_string(),
        supported_parameters: Vec::new(),
        reasoning: None,
        context_length: None,
        pricing: None,
    }
}

#[test]
fn account_rows_extend_only_their_own_connection() {
    set_account_rows(
        "acct-a",
        vec![
            overlay_model("vendor/secret-a"),
            overlay_model("koma-token/a"),
        ],
    );
    set_account_rows("acct-b", vec![overlay_model("vendor/secret-b")]);

    let ids = |uuid: &str| -> Vec<String> {
        oauth_catalogue_with_account(uuid, OAuthProvider::KomaRun)
            .into_iter()
            .map(|m| m.id)
            .collect()
    };
    let a = ids("acct-a");
    assert!(a.contains(&"vendor/secret-a".to_string()));
    assert!(a.contains(&"koma-token/a".to_string()));
    assert!(!a.contains(&"vendor/secret-b".to_string()));
    let b = ids("acct-b");
    assert!(b.contains(&"vendor/secret-b".to_string()));
    assert!(!b.contains(&"vendor/secret-a".to_string()));
    // A connection with no rows sees only the public list.
    assert!(!ids("acct-none").contains(&"vendor/secret-a".to_string()));
}

#[test]
fn account_rows_do_not_repeat_public_ids_and_empty_clears() {
    set_account_rows("acct-c", vec![overlay_model("vendor/only-c")]);
    let ids = |uuid: &str| -> Vec<String> {
        oauth_catalogue_with_account(uuid, OAuthProvider::KomaRun)
            .into_iter()
            .map(|m| m.id)
            .collect()
    };
    let before = ids("acct-c");
    assert_eq!(
        before
            .iter()
            .filter(|id| id.as_str() == "vendor/only-c")
            .count(),
        1
    );
    // A revoked grant disappears on the next refresh.
    set_account_rows("acct-c", Vec::new());
    assert!(!ids("acct-c").contains(&"vendor/only-c".to_string()));
}

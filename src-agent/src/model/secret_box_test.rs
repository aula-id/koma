#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::model::app_config::{AppConfig, McpServerEntry, OAuthConn, ProviderConn};
use crate::model::web_search::SearchKey;

const KEY: Key = [7u8; KEY_LEN];
const OTHER: Key = [9u8; KEY_LEN];

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("koma-secret-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_with_secrets() -> AppConfig {
    let mut config = AppConfig::default();
    config.providers.push(ProviderConn {
        uuid: "p1".into(),
        name: "openrouter".into(),
        endpoint: "https://openrouter.ai/api/v1".into(),
        api_key: "sk-or-PROVIDER".into(),
        ..ProviderConn::default()
    });
    config.oauth_conns.push(OAuthConn {
        uuid: "o1".into(),
        access_token: "ACCESS-tok".into(),
        refresh_token: "REFRESH-tok".into(),
        id_token: "ID-tok".into(),
        account_id: "acct-visible".into(),
        ..OAuthConn::default()
    });
    config.web_search.exa_key = SearchKey("EXA-key".into());
    config.mcp_servers.push(McpServerEntry {
        uuid: "m1".into(),
        name: "github".into(),
        env: vec![("GITHUB_TOKEN".into(), "ghp_MCP".into())],
        ..McpServerEntry::default()
    });
    config
}

const SECRETS: [&str; 6] = [
    "sk-or-PROVIDER",
    "ACCESS-tok",
    "REFRESH-tok",
    "ID-tok",
    "EXA-key",
    "ghp_MCP",
];

#[test]
fn a_sealed_value_opens_only_under_its_own_key() {
    let sealed = seal_with(&KEY, "sk-live-123").unwrap();
    assert!(is_sealed(&sealed));
    assert!(!sealed.contains("sk-live-123"));
    assert_eq!(open_with(&KEY, &sealed).as_deref(), Some("sk-live-123"));
    assert_eq!(open_with(&OTHER, &sealed), None);
    // Same plain text, fresh nonce: two saves never produce the same bytes.
    assert_ne!(sealed, seal_with(&KEY, "sk-live-123").unwrap());
}

#[test]
fn an_altered_or_malformed_value_does_not_open() {
    let sealed = seal_with(&KEY, "secret").unwrap();
    let mut altered = sealed.clone().into_bytes();
    let last = altered.len() - 1;
    altered[last] = if altered[last] == b'A' { b'B' } else { b'A' };
    assert_eq!(open_with(&KEY, &String::from_utf8(altered).unwrap()), None);
    assert_eq!(open_with(&KEY, "enc:v1:"), None);
    assert_eq!(open_with(&KEY, "enc:v1:not base64!"), None);
    assert_eq!(open_with(&KEY, "plain"), None);
}

#[test]
fn the_stored_config_holds_no_credential_in_plain_text() {
    let config = config_with_secrets();
    let stored = String::from_utf8(config.to_disk_bytes(&KEY).unwrap()).unwrap();
    for secret in SECRETS {
        assert!(!stored.contains(secret), "{secret} is stored in plain text");
    }
    // Everything that is not a credential stays readable and editable.
    for visible in [
        "openrouter",
        "https://openrouter.ai/api/v1",
        "acct-visible",
        "GITHUB_TOKEN",
    ] {
        assert!(stored.contains(visible), "{visible} should stay plain");
    }
    // The config in memory is untouched: requests still get the real key.
    assert_eq!(config.providers[0].api_key, "sk-or-PROVIDER");
}

#[test]
fn a_stored_config_loads_back_with_its_credentials() {
    let stored = config_with_secrets().to_disk_bytes(&TEST_KEY).unwrap();
    let loaded = AppConfig::from_disk_bytes(&stored).unwrap();
    assert_eq!(loaded.providers[0].api_key, "sk-or-PROVIDER");
    assert_eq!(loaded.oauth_conns[0].access_token, "ACCESS-tok");
    assert_eq!(loaded.oauth_conns[0].refresh_token, "REFRESH-tok");
    assert_eq!(loaded.oauth_conns[0].id_token, "ID-tok");
    assert_eq!(loaded.web_search.exa_key.0, "EXA-key");
    assert_eq!(loaded.mcp_servers[0].env[0].1, "ghp_MCP");
}

#[test]
fn a_plain_text_config_from_an_older_version_still_loads() {
    let old = serde_json::to_vec(&config_with_secrets()).unwrap();
    let loaded = AppConfig::from_disk_bytes(&old).unwrap();
    assert_eq!(loaded.providers[0].api_key, "sk-or-PROVIDER");
    assert_eq!(loaded.mcp_servers[0].env[0].1, "ghp_MCP");
}

#[test]
fn opening_reports_plain_values_and_blanks_what_it_cannot_read() {
    let mut opened = Opened::default();

    let mut plain = String::from("sk-plain");
    open_field(Some(&KEY), &mut plain, &mut opened);
    assert_eq!(plain, "sk-plain");

    let mut foreign = seal_with(&OTHER, "sk-foreign").unwrap();
    open_field(Some(&KEY), &mut foreign, &mut opened);
    assert_eq!(foreign, "", "a value sealed under a lost key is not usable");

    let mut keyless = seal_with(&KEY, "sk-keyless").unwrap();
    open_field(None, &mut keyless, &mut opened);
    assert_eq!(keyless, "");

    let mut empty = String::new();
    open_field(Some(&KEY), &mut empty, &mut opened);

    assert_eq!(opened, Opened { plain: 1, lost: 2 });
}

#[test]
fn sealing_twice_does_not_wrap_a_value_twice() {
    let mut field = String::from("sk-once");
    seal_field(&KEY, &mut field).unwrap();
    let first = field.clone();
    seal_field(&KEY, &mut field).unwrap();
    assert_eq!(field, first);
    assert_eq!(open_with(&KEY, &field).as_deref(), Some("sk-once"));
}

#[test]
fn the_machine_key_is_created_once_and_then_reused() {
    let dir = temp_dir("key");
    let path = dir.join("secrets").join("machine.key");
    let created = load_or_create_key_at(&path).unwrap();
    assert_eq!(load_or_create_key_at(&path).unwrap(), created);
    assert_eq!(load_key_at(&path), Some(created));
    assert!(!is_exposed(&path).unwrap_or(false));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_damaged_machine_key_is_reported_not_replaced() {
    let dir = temp_dir("badkey");
    let path = dir.join("machine.key");
    std::fs::write(&path, b"short").unwrap();
    assert!(load_or_create_key_at(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"short");
    assert_eq!(load_key_at(&path), None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn racing_creators_all_end_up_with_the_same_key() {
    let dir = temp_dir("race");
    let path = dir.join("machine.key");
    let keys: Vec<Key> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| load_or_create_key_at(&path).unwrap()))
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    let on_disk = load_key_at(&path).unwrap();
    assert!(keys.iter().all(|key| *key == on_disk));
    let leftovers = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(leftovers, 1, "temp files are cleaned up");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_private_write_narrows_a_file_that_was_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("mode");
    let path = dir.join("config.json");
    std::fs::write(&path, b"old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(is_exposed(&path), Some(true));

    write_private(&path, b"new").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(is_exposed(&path), Some(false));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_session_key_is_sealed_on_disk_and_restored_on_load() {
    use crate::model::settings::Settings;
    let dir = temp_dir("session");
    let path = dir.join("settings.json");
    let settings = Settings {
        api_key: "sk-or-SESSION".into(),
        ..Settings::default()
    };
    settings.save(&path).unwrap();

    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(!stored.contains("sk-or-SESSION"));
    assert!(!is_exposed(&path).unwrap_or(false));
    assert_eq!(Settings::load(&path).unwrap().api_key, "sk-or-SESSION");

    // An older session file with a plain key keeps working.
    std::fs::write(&path, r#"{"api_key":"sk-or-OLD"}"#).unwrap();
    assert_eq!(Settings::load(&path).unwrap().api_key, "sk-or-OLD");
    std::fs::remove_dir_all(dir).unwrap();
}

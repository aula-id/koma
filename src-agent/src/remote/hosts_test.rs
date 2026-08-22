#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;

fn import_ssh_config_text(text: &str) -> Vec<RemoteHost> {
    let existing = RemoteHosts::default();
    parse_ssh_config_text(text, &existing)
}

#[test]
fn upsert_inserts_new_host() {
    let mut hosts = RemoteHosts::default();
    let host = RemoteHost {
        id: "test-id".into(),
        name: "test".into(),
        user: "root".into(),
        host: "10.0.0.1".into(),
        port: 22,
        key_path: None,
        last_connected: None,
        tags: vec![],
    };
    let idx = upsert_host(&mut hosts, host);
    assert_eq!(idx, 0);
    assert_eq!(hosts.hosts.len(), 1);
}

#[test]
fn upsert_updates_existing() {
    let mut hosts = RemoteHosts::default();
    let host = RemoteHost {
        id: "same-id".into(),
        name: "v1".into(),
        user: "root".into(),
        host: "10.0.0.1".into(),
        port: 22,
        key_path: None,
        last_connected: None,
        tags: vec![],
    };
    upsert_host(&mut hosts, host);

    let updated = RemoteHost {
        id: "same-id".into(),
        name: "v2".into(),
        user: "admin".into(),
        host: "10.0.0.2".into(),
        port: 2222,
        key_path: None,
        last_connected: None,
        tags: vec![],
    };
    let idx = upsert_host(&mut hosts, updated);
    assert_eq!(idx, 0);
    assert_eq!(hosts.hosts.len(), 1);
    assert_eq!(hosts.hosts[0].name, "v2");
    assert_eq!(hosts.hosts[0].port, 2222);
}

#[test]
fn delete_removes_host() {
    let mut hosts = RemoteHosts::default();
    hosts.hosts.push(RemoteHost {
        id: "to-delete".into(),
        name: "del".into(),
        user: "root".into(),
        host: "10.0.0.1".into(),
        port: 22,
        key_path: None,
        last_connected: None,
        tags: vec![],
    });
    assert!(delete_host(&mut hosts, "to-delete"));
    assert!(hosts.hosts.is_empty());
    assert!(!delete_host(&mut hosts, "nonexistent"));
}

#[test]
fn host_by_id_finds() {
    let mut hosts = RemoteHosts::default();
    hosts.hosts.push(RemoteHost {
        id: "find-me".into(),
        name: "found".into(),
        user: "root".into(),
        host: "10.0.0.1".into(),
        port: 22,
        key_path: None,
        last_connected: None,
        tags: vec![],
    });
    assert!(host_by_id(&hosts, "find-me").is_some());
    assert!(host_by_id(&hosts, "nope").is_none());
}

#[test]
fn host_address_format() {
    let host = RemoteHost {
        id: "x".into(),
        name: "x".into(),
        user: "ubuntu".into(),
        host: "example.com".into(),
        port: 22,
        key_path: None,
        last_connected: None,
        tags: vec![],
    };
    assert_eq!(host.address(), "ubuntu@example.com");

    let host2 = RemoteHost { port: 2222, ..host };
    assert_eq!(host2.address(), "ubuntu@example.com:2222");
}

#[test]
fn serde_roundtrip() {
    let hosts = RemoteHosts {
        hosts: vec![RemoteHost {
            id: "rt".into(),
            name: "test".into(),
            user: "root".into(),
            host: "10.0.0.1".into(),
            port: 22,
            key_path: Some("/home/user/.ssh/id_ed25519".into()),
            last_connected: Some(1700000000),
            tags: vec!["prod".into()],
        }],
    };
    let json = serde_json::to_vec_pretty(&hosts).unwrap();
    let back: RemoteHosts = serde_json::from_slice(&json).unwrap();
    assert_eq!(back.hosts.len(), 1);
    assert_eq!(back.hosts[0].id, "rt");
    assert_eq!(
        back.hosts[0].key_path,
        Some("/home/user/.ssh/id_ed25519".into())
    );
}

#[test]
fn ssh_config_identity_file() {
    let text = "Host myserver\n  User deploy\n  HostName 10.0.0.5\n  IdentityFile ~/.ssh/id_ed25519\n  Port 2222\n";
    let imported = import_ssh_config_text(text);
    assert_eq!(imported.len(), 1);
    assert_eq!(
        imported[0].key_path,
        Some(
            dirs::home_dir()
                .unwrap()
                .join(".ssh/id_ed25519")
                .to_string_lossy()
                .into_owned()
        )
    );
    assert_eq!(imported[0].port, 2222);
}

#[test]
fn ssh_config_identity_file_quoted() {
    let text = "Host srv\n  User u\n  HostName h\n  IdentityFile \"/path/to/key\"\n";
    let imported = import_ssh_config_text(text);
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].key_path, Some("/path/to/key".to_string()));
}

#[test]
fn ssh_config_wildcard_skipped() {
    let text = "Host *\n  User u\n  HostName h\n  IdentityFile ~/.ssh/id\n\nHost real\n  User r\n  HostName h2\n";
    let imported = import_ssh_config_text(text);
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].name, "real");
    assert!(imported[0].key_path.is_none()); // wildcard block ignored
}

#[test]
fn ssh_config_key_path_resets_between_blocks() {
    let text = "Host a\n  User u1\n  HostName h1\n  IdentityFile /key/a\n\nHost b\n  User u2\n  HostName h2\n";
    let imported = import_ssh_config_text(text);
    assert_eq!(imported.len(), 2);
    assert_eq!(imported[0].key_path, Some("/key/a".to_string()));
    assert!(imported[1].key_path.is_none()); // key_path should NOT leak from block a
}

#[test]
fn ssh_config_first_identityfile_wins() {
    let text =
        "Host srv\n  User u\n  HostName h\n  IdentityFile /first\n  IdentityFile /second\n";
    let imported = import_ssh_config_text(text);
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].key_path, Some("/first".to_string()));
}

#[test]
fn ssh_config_home_expansion() {
    let text = "Host srv\n  User u\n  HostName h\n  IdentityFile ~/mykey\n";
    let imported = import_ssh_config_text(text);
    assert_eq!(imported.len(), 1);
    let expected = dirs::home_dir()
        .unwrap()
        .join("mykey")
        .to_string_lossy()
        .into_owned();
    assert_eq!(imported[0].key_path, Some(expected));
}

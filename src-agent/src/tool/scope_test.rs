use super::*;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("koma-scope-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn base(session: &str, cwd: &str) -> ScopeBase {
    ScopeBase::assemble(Path::new("/sessions/s1"), session, Path::new(cwd))
}

fn binding(verb: &str, host: &str, port: Option<u16>) -> Binding {
    Binding {
        schema: SCHEMA,
        verb: verb.into(),
        host: host.into(),
        port,
        path: None,
        provenance: Provenance::Resolved,
    }
}

#[test]
fn the_path_carries_session_and_a_digested_cwd() {
    let shown = base("sess-a", "/work/repo")
        .dir()
        .to_string_lossy()
        .into_owned();
    assert!(shown.contains("sec-scope"));
    assert!(shown.contains("sess-a"));
    assert!(shown.contains(&digest("/work/repo")));
}

#[test]
fn two_sessions_on_one_checkout_get_different_dirs() {
    assert_ne!(base("a", "/work/repo"), base("b", "/work/repo"));
}

#[test]
fn two_checkouts_get_different_dirs() {
    assert_ne!(base("a", "/work/repo"), base("a", "/work/other"));
}

#[test]
fn port_and_path_separate_targets_on_one_host() {
    let b = base("a", "/w");
    assert_ne!(
        b.record_path("red", "h", Some(8080), None),
        b.record_path("red", "h", Some(8443), None)
    );
    assert_ne!(
        b.record_path("red", "h", Some(80), Some("/api")),
        b.record_path("red", "h", Some(80), Some("/admin"))
    );
}

#[test]
fn host_case_does_not_fork_a_record() {
    let b = base("a", "/w");
    assert_eq!(
        b.record_path("red", "LocalHost", Some(1), None),
        b.record_path("red", "localhost", Some(1), None)
    );
}

#[test]
fn a_host_with_a_colon_or_slash_stays_one_filename() {
    let b = base("a", "/w");
    let name = b.record_path("red", "localhost:8080", Some(80), Some("/a/b"));
    let name = name.file_name().expect("has a name").to_string_lossy();
    assert!(!name.contains('/'), "{name}");
}

#[test]
fn write_then_read_round_trips() {
    let dir = tmp("roundtrip");
    let b = ScopeBase { dir };
    let want = binding("red", "localhost", Some(8080));
    let path = b.write(&want).expect("write");
    assert!(path.exists());
    assert_eq!(b.read("red", "localhost", Some(8080), None), Some(want));
    let _ = std::fs::remove_dir_all(&b.dir);
}

#[test]
fn a_present_binding_is_not_rewritten() {
    let dir = tmp("ensure");
    let b = ScopeBase { dir };
    let want = binding("red", "h", None);
    assert!(b.ensure(&want).expect("first"));
    assert!(!b.ensure(&want).expect("second"));
    let _ = std::fs::remove_dir_all(&b.dir);
}

#[test]
fn a_record_from_another_schema_is_not_returned() {
    let dir = tmp("schema");
    let b = ScopeBase { dir };
    let mut old = binding("red", "h", None);
    old.schema = SCHEMA + 1;
    b.write(&old).expect("write");
    assert!(b.read("red", "h", None, None).is_none());
    let _ = std::fs::remove_dir_all(&b.dir);
}

#[test]
fn an_absent_record_reads_none() {
    let b = ScopeBase { dir: tmp("absent") };
    assert!(b.read("red", "nope", None, None).is_none());
}

fn dest(v: serde_json::Value) -> Option<(String, Option<u16>, Option<String>)> {
    destination_of("sec_http", &v)
}

#[test]
fn a_scheme_defaults_the_port_so_it_matches_the_admission() {
    assert_eq!(
        dest(serde_json::json!({"url": "http://h"})).map(|d| d.1),
        Some(Some(80))
    );
    assert_eq!(
        dest(serde_json::json!({"url": "https://h"})).map(|d| d.1),
        Some(Some(443))
    );
}

#[test]
fn a_root_slash_does_not_fork_the_key() {
    assert_eq!(
        dest(serde_json::json!({"url": "https://h/"})),
        dest(serde_json::json!({"url": "https://h"}))
    );
}

#[test]
fn a_url_is_split_into_host_port_and_path() {
    assert_eq!(
        dest(serde_json::json!({"url": "http://Example.COM:8080/api/x?y=1"})),
        Some(("example.com".into(), Some(8080), Some("/api/x".into())))
    );
}

#[test]
fn a_bare_host_has_no_port_or_path() {
    assert_eq!(
        dest(serde_json::json!({"host": "box"})),
        Some(("box".into(), Some(443), None))
    );
}

#[test]
fn userinfo_is_stripped_from_the_host() {
    assert_eq!(
        dest(serde_json::json!({"url": "https://u:p@h.example/x"})),
        Some(("h.example".into(), Some(443), Some("/x".into())))
    );
}

#[test]
fn a_non_numeric_colon_tail_is_part_of_the_host() {
    // IPv6-ish or a scheme-y `host` must not lose its tail to the port split.
    assert_eq!(
        dest(serde_json::json!({"host": "fe80::1"})).map(|d| (d.0, d.1)),
        Some(("fe80::1".into(), Some(80)))
    );
    assert_eq!(
        dest(serde_json::json!({"host": "[fe80::1]"})).map(|d| d.0),
        Some("fe80::1".into())
    );
    assert_eq!(
        dest(serde_json::json!({"host": "127.0.0.1"})).map(|d| d.1),
        Some(Some(80))
    );
}

#[test]
fn a_blank_or_missing_destination_is_none() {
    assert_eq!(dest(serde_json::json!({"url": "   "})), None);
    assert_eq!(dest(serde_json::json!({})), None);
    assert_eq!(dest(serde_json::json!({"url": 7})), None);
}

#[test]
fn a_fixed_host_tool_binds_nothing() {
    // sec_factor reaches factordb from inside the handler; there is no target
    // to key, even though it accepts other arguments.
    assert_eq!(
        destination_of("sec_factor", &serde_json::json!({"n": "12"})),
        None
    );
}

#[test]
fn the_verb_comes_from_the_tool_namespace() {
    assert_eq!(verb_of("sec_http"), Some("sec"));
    assert_eq!(verb_of("red_scan"), Some("red"));
    assert_eq!(verb_of("build"), None);
}

#[test]
fn the_record_carries_no_timestamp() {
    let json = serde_json::to_string(&binding("red", "h", Some(1))).expect("serialise");
    for key in ["seen", "time", "at", "stamp", "epoch"] {
        assert!(!json.contains(key), "{key} leaked into {json}");
    }
}

#[test]
fn a_deep_cwd_does_not_fan_the_tree_out() {
    let deep = format!("/{}", "very/deep/".repeat(40));
    let shown = base("a", &deep).dir().to_string_lossy().into_owned();
    assert!(shown.ends_with(&digest(&deep)));
    assert!(shown.matches('/').count() < 12, "{shown}");
}

fn active_scope(target: &str) -> crate::app::sec::scope::Scope {
    use crate::app::sec::scope as sc;
    sc::Scope {
        version: 1,
        state: sc::State::Active,
        auth_mode: sc::Mode::None,
        targets: vec![sc::Host::parse(target).expect("valid")],
        exclusions: Vec::new(),
        caps: sc::Caps::default(),
        window: sc::Window::default(),
        confirmed_by: Some("tester".into()),
    }
}

fn ctx_for(dir: &Path, cwd: &Path) -> ToolCtx {
    crate::tool::ToolCtx {
        session_id: "sess".into(),
        cyber_lane: None,
        sec_block: None,
        sec_inactive: Vec::new(),
        session_dir: Some(dir.to_path_buf()),
        workspace: cwd.to_path_buf(),
        ..crate::tool::test_ctx()
    }
}

#[test]
fn a_call_with_no_destination_needs_no_scope() {
    let dir = tmp("nodest");
    let ctx = ctx_for(&dir, Path::new("/w"));
    assert_eq!(
        admit_from_call(&ctx, "sec_decode", &serde_json::json!({"data": "x"})),
        None
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_destination_with_no_scope_is_refused() {
    let dir = tmp("noscope");
    let ctx = ctx_for(&dir, Path::new("/w"));
    let got = admit_from_call(
        &ctx,
        "sec_http",
        &serde_json::json!({"url": "http://a.example"}),
    );
    assert!(
        got.as_deref()
            .is_some_and(|r| r.contains("no scope record")),
        "{got:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_in_scope_target_is_admitted_and_an_out_of_scope_one_is_not() {
    use crate::app::sec::scope as sc;
    let dir = tmp("admit");
    let ctx = ctx_for(&dir, Path::new("/w"));
    let base = ScopeBase::of(&ctx).expect("base");
    sc::store(base.dir(), &active_scope("a.example")).expect("store");
    let ok = admit_from_call(
        &ctx,
        "sec_http",
        &serde_json::json!({"url": "https://a.example/x"}),
    );
    assert_eq!(ok, None);
    let bad = admit_from_call(
        &ctx,
        "sec_http",
        &serde_json::json!({"url": "https://b.example/"}),
    )
    .expect("refused");
    assert!(bad.contains("not in scope"), "{bad}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_target_writes_no_binding() {
    use crate::app::sec::scope as sc;
    let dir = tmp("nobind");
    let ctx = ctx_for(&dir, Path::new("/w"));
    let base = ScopeBase::of(&ctx).expect("base");
    sc::store(base.dir(), &active_scope("a.example")).expect("store");
    let args = serde_json::json!({"url": "https://b.example/"});
    assert!(admit_from_call(&ctx, "sec_http", &args).is_some());
    settle(
        &ctx,
        "sec_http",
        &args,
        crate::tool::verdict::Verdict::Error,
    );
    assert!(base.read("sec", "b.example", Some(443), None).is_none());
    settle(
        &ctx,
        "sec_http",
        &args,
        crate::tool::verdict::Verdict::Confirmed,
    );
    assert!(base.read("sec", "b.example", Some(443), None).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn digest_is_stable_and_short() {
    assert_eq!(digest("/work/repo"), digest("/work/repo"));
    assert_eq!(digest("/work/repo").len(), 10);
    assert_ne!(digest("/work/repo"), digest("/work/repo2"));
}

#[test]
fn digest_matches_the_daemon_side() {
    // `target.py::_digest` asserts the same vectors; a change either side is
    // caught on both rather than forking every record silently.
    assert_eq!(digest("/work/repo"), "1cd4fc81c8");
    assert_eq!(digest(""), "cbf29ce484");
}

#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use serde_json::json;

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("koma-grant-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("temp dir");
    p
}

fn ctx(dir: &std::path::Path) -> ToolCtx {
    let mut c = crate::tool::test_ctx();
    c.session_dir = Some(dir.to_path_buf());
    c.cyber_lane = Some(crate::app::sec::lane::Lane::Red);
    c
}

fn ask(args: serde_json::Value) -> Request {
    requested("sec_runtime", &args)
        .expect("parses")
        .expect("asks for targets")
}

#[test]
fn only_a_runtime_call_with_targets_is_a_request() {
    assert_eq!(
        requested("sec_run", &json!({"targets": ["a.test"]})),
        Ok(None)
    );
    assert_eq!(
        requested(
            "sec_runtime",
            &json!({"op": "status", "targets": ["a.test"]})
        ),
        Ok(None)
    );
    assert_eq!(requested("sec_runtime", &json!({"op": "up"})), Ok(None));
    assert_eq!(
        requested("sec_runtime", &json!({"op": "up", "targets": []})),
        Ok(None)
    );
    assert_eq!(
        requested(
            "sec_runtime",
            &json!({"op": "net", "mode": "off", "targets": ["a.test"]})
        ),
        Ok(None)
    );
    assert!(requested(
        "sec_runtime",
        &json!({"op": "net", "mode": "scope", "targets": ["a.test"]})
    )
    .expect("parses")
    .is_some());
}

#[test]
fn cells_are_normalised_and_a_url_is_refused() {
    let r = ask(json!({"op": "up", "targets": ["B.Test:443,80", "a.test", "a.test"]}));
    assert_eq!(wire(&r.targets), vec!["a.test", "b.test:80,443"]);
    for bad in ["https://a.test", "a.test/admin", "", "a.test:http"] {
        assert!(
            requested("sec_runtime", &json!({"op": "up", "targets": [bad]})).is_err(),
            "{bad:?} should not parse"
        );
    }
}

#[test]
fn nothing_is_covered_until_the_user_grants_it() {
    let ctx = ctx(&tmp("grant"));
    let r = ask(json!({"op": "net", "mode": "scope", "targets": ["a.test:8080"]}));
    assert!(!covered(current(&ctx).as_ref(), &r));

    let scope = grant(&ctx, &r).expect("granted");
    assert_eq!(scope.version, 1);
    assert_eq!(scope.confirmed_by.as_deref(), Some(CONFIRMED_BY_USER));
    assert!(covered(current(&ctx).as_ref(), &r));

    // The record is the one the dispatch gate reads.
    let call = json!({"url": "http://a.test:8080/"});
    assert_eq!(
        crate::tool::scope::admit_from_call(&ctx, "sec_http", &call),
        None
    );
    let other = json!({"url": "http://b.test/"});
    assert!(crate::tool::scope::admit_from_call(&ctx, "sec_http", &other).is_some());
}

#[test]
fn a_wider_request_is_not_covered_and_regranting_bumps_the_version() {
    let ctx = ctx(&tmp("wider"));
    grant(&ctx, &ask(json!({"op": "up", "targets": ["a.test"]}))).expect("granted");

    let wider = ask(json!({"op": "up", "targets": ["a.test", "b.test"]}));
    assert!(!covered(current(&ctx).as_ref(), &wider));
    assert_eq!(grant(&ctx, &wider).expect("granted").version, 2);
    assert!(covered(
        current(&ctx).as_ref(),
        &ask(json!({"op": "up", "targets": ["b.test"]}))
    ));
}

#[test]
fn a_request_that_drops_an_exclusion_is_not_covered() {
    let ctx = ctx(&tmp("excl"));
    let narrowed = ask(json!({"op": "up", "targets": ["a.test"], "exclusions": ["admin.a.test"]}));
    grant(&ctx, &narrowed).expect("granted");
    assert!(covered(current(&ctx).as_ref(), &narrowed));
    assert!(!covered(
        current(&ctx).as_ref(),
        &ask(json!({"op": "up", "targets": ["a.test"]}))
    ));
}

#[test]
fn a_paused_or_unconfirmed_scope_covers_nothing() {
    let ctx = ctx(&tmp("paused"));
    let r = ask(json!({"op": "up", "targets": ["a.test"]}));
    let mut scope = grant(&ctx, &r).expect("granted");
    scope.state = State::Paused;
    assert!(!covered(Some(&scope), &r));
    scope.state = State::Active;
    scope.confirmed_by = None;
    assert!(!covered(Some(&scope), &r));
}

#[test]
fn the_prompt_names_every_target_and_exclusion() {
    let r =
        ask(json!({"op": "up", "targets": ["a.test:443", "b.test"], "exclusions": ["x.b.test"]}));
    assert_eq!(
        prompt(&r),
        "allow network access to: a.test:443, b.test · except: x.b.test"
    );
}

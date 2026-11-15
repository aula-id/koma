//! Tests for `usage.query` and `session_info.usage`. None of them reads the
//! real ledger or registry: the query and the name lookup are injected.
use super::host::usage::{self, Known};
use super::protocol::{Policy, Request, RpcError};
use super::*;
use crate::ipc::proto::{DaemonEvent as E, DaemonFrame};
use crate::model::usage::query::{
    self, GroupBy, LedgerState, RoleKind, UsageBucket, UsageError, UsageGroup, UsageQuery,
    UsageReport, UsageTotals,
};
use crate::model::usage::BucketSize;
use std::collections::HashMap;

fn totals(calls: i64, tokens_in: i64, cost: f64) -> UsageTotals {
    UsageTotals {
        calls,
        tokens_in,
        tokens_cached: 1,
        tokens_out: 2,
        cost,
    }
}

fn report(group_by: GroupBy, keys: &[&str]) -> UsageReport {
    UsageReport {
        from_at: 100,
        to_at: 200,
        ledger: LedgerState::Ok,
        totals: totals(3, 30, 0.5),
        groups: (group_by != GroupBy::None).then(|| {
            keys.iter()
                .map(|key| UsageGroup {
                    key: key.to_string(),
                    totals: totals(1, 10, 0.1),
                    role_kind: (group_by == GroupBy::Role).then(|| {
                        if *key == "main" {
                            RoleKind::Main
                        } else {
                            RoleKind::Sub
                        }
                    }),
                    agent: key.strip_prefix("sub:").map(str::to_string),
                })
                .collect()
        }),
        groups_truncated: false,
        buckets: None,
    }
}

fn no_names() -> Result<HashMap<String, Known>, RpcError> {
    Ok(HashMap::new())
}

fn invalid(params: Value) -> RpcError {
    usage::parse(params).unwrap_err()
}

fn assert_out_of_range(params: Value) {
    let e = invalid(params.clone());
    assert_eq!(e.code, "invalid_request", "{params}");
    assert_eq!(
        e.extra().and_then(|x| x.reason),
        Some("out_of_range"),
        "{params}"
    );
}

#[test]
fn params_default_to_an_unbounded_none_query() {
    assert_eq!(usage::parse(json!({})).unwrap(), UsageQuery::default());
    let q = usage::parse(json!({
        "from_at": 10, "to_at": 20, "session_id": "s", "workdir": std::env::temp_dir(),
        "group_by": "role", "bucket": "week", "tz_offset_minutes": -720, "limit": 500
    }))
    .unwrap();
    assert_eq!(q.from_at, Some(10));
    assert_eq!(q.to_at, Some(20));
    assert_eq!(q.group_by, GroupBy::Role);
    assert_eq!(q.bucket, Some(BucketSize::Week));
    assert_eq!(q.tz_offset_minutes, Some(-720));
    assert_eq!(q.limit, Some(500));
    let none = usage::parse(json!({"group_by":"none","bucket":"none"})).unwrap();
    assert_eq!(none.group_by, GroupBy::None);
    assert_eq!(none.bucket, None);
}

#[test]
fn out_of_bounds_params_are_out_of_range() {
    for params in [
        json!({"from_at": "1"}),
        json!({"to_at": 1.5}),
        json!({"session_id": ""}),
        json!({"session_id": 3}),
        json!({"workdir": "relative/dir"}),
        json!({"workdir": 7}),
        json!({"group_by": "team"}),
        json!({"bucket": "month"}),
        json!({"tz_offset_minutes": -721}),
        json!({"tz_offset_minutes": 841}),
        json!({"limit": 0}),
        json!({"limit": 501}),
        json!({"limit": -1}),
    ] {
        assert_out_of_range(params);
    }
    assert_eq!(invalid(json!([])).code, "invalid_request");
}

#[test]
fn the_window_and_bucket_count_are_bounded_by_the_query() {
    let day = 86_400;
    let rejected = |params: Value| {
        let q = usage::parse(params).unwrap();
        usage::execute(&q, query::run, no_names).unwrap_err()
    };
    let too_long = rejected(json!({"from_at": 0, "to_at": 401 * day}));
    assert_eq!(too_long.code, "invalid_request");
    assert_eq!(
        too_long.extra().and_then(|x| x.reason),
        Some("range_too_large")
    );
    let buckets = rejected(json!({"from_at": 0, "to_at": 60 * day, "bucket": "hour"}));
    assert_eq!(
        buckets.extra().and_then(|x| x.reason),
        Some("range_too_large")
    );
    let backwards = rejected(json!({"from_at": 100, "to_at": 100}));
    assert_eq!(
        backwards.extra().and_then(|x| x.reason),
        Some("out_of_range")
    );
}

#[test]
fn query_errors_map_to_reasons() {
    let reason = |e: UsageError| {
        let e = usage::error(e);
        (e.code, e.extra().and_then(|x| x.reason))
    };
    assert_eq!(
        reason(UsageError::Timeout),
        ("unavailable", Some("query_timeout"))
    );
    assert_eq!(
        reason(UsageError::Unreadable("x".into())),
        ("unavailable", Some("io_error"))
    );
    assert_eq!(
        reason(UsageError::OutOfRange("x".into())),
        ("invalid_request", Some("out_of_range"))
    );
    let q = UsageQuery::default();
    let timed_out = usage::execute(&q, |_| Err(UsageError::Timeout), no_names).unwrap_err();
    assert_eq!(timed_out.code, "unavailable");
    assert_eq!(
        timed_out.extra().and_then(|x| x.reason),
        Some("query_timeout")
    );
}

#[test]
fn a_missing_ledger_is_zeros_not_an_error() {
    let q = UsageQuery {
        group_by: GroupBy::Model,
        bucket: Some(BucketSize::Day),
        ..UsageQuery::default()
    };
    let missing = UsageReport {
        from_at: 0,
        to_at: 86_400,
        ledger: LedgerState::Missing,
        totals: UsageTotals::default(),
        groups: Some(vec![]),
        groups_truncated: false,
        buckets: Some(vec![UsageBucket {
            start_at: 0,
            totals: UsageTotals::default(),
        }]),
    };
    let out = usage::execute(&q, |_| Ok(missing), no_names).unwrap();
    assert_eq!(out["ledger"], "missing");
    assert_eq!(
        out["totals"],
        json!({"calls":0,"tokens_in":0,"tokens_cached":0,"tokens_out":0,"cost":0.0})
    );
    assert_eq!(out["groups"], json!([]));
    assert_eq!(out["groups_truncated"], false);
    assert_eq!(out["buckets"][0]["start_at"], 0);
    assert_eq!(out["buckets"][0]["calls"], 0);
}

#[test]
fn optional_sections_are_absent_unless_asked_for() {
    let q = UsageQuery::default();
    let out = usage::execute(&q, |_| Ok(report(GroupBy::None, &[])), no_names).unwrap();
    assert_eq!(out["ledger"], "ok");
    assert_eq!(out["from_at"], 100);
    assert_eq!(out["totals"]["calls"], 3);
    for key in ["groups", "groups_truncated", "buckets"] {
        assert!(out.get(key).is_none(), "{key}");
    }
}

#[test]
fn session_groups_join_names_and_mark_deleted_sessions() {
    let q = UsageQuery {
        group_by: GroupBy::Session,
        ..UsageQuery::default()
    };
    let names = || {
        Ok(HashMap::from([
            (
                "live".to_string(),
                Known {
                    name: "Work".into(),
                    room: false,
                },
            ),
            (
                "room".to_string(),
                Known {
                    name: "Audit".into(),
                    room: true,
                },
            ),
        ]))
    };
    let out = usage::execute(
        &q,
        |_| Ok(report(GroupBy::Session, &["live", "room", "gone"])),
        names,
    )
    .unwrap();
    let groups = out["groups"].as_array().unwrap();
    assert_eq!(groups[0]["label"], "Work");
    assert_eq!(groups[0]["kind"], "coding");
    assert_eq!(groups[0]["deleted"], false);
    assert_eq!(groups[1]["kind"], "room");
    assert_eq!(groups[2]["key"], "gone");
    assert_eq!(groups[2]["label"], Value::Null);
    assert_eq!(groups[2]["kind"], Value::Null);
    assert_eq!(groups[2]["deleted"], true);
    assert_eq!(groups[2]["calls"], 1);
}

#[test]
fn only_session_groups_read_the_registry() {
    let refuse = || -> Result<HashMap<String, Known>, RpcError> {
        panic!("the registry must not be read");
    };
    for by in [GroupBy::None, GroupBy::Model, GroupBy::Role] {
        let q = UsageQuery {
            group_by: by,
            ..UsageQuery::default()
        };
        usage::execute(&q, |_| Ok(report(by, &["main"])), refuse).unwrap();
    }
    // A session query with no groups has nothing to label either.
    let q = UsageQuery {
        group_by: GroupBy::Session,
        ..UsageQuery::default()
    };
    usage::execute(&q, |_| Ok(report(GroupBy::Session, &[])), refuse).unwrap();
}

#[test]
fn role_and_model_groups_carry_their_own_fields() {
    let q = UsageQuery {
        group_by: GroupBy::Role,
        ..UsageQuery::default()
    };
    let out = usage::execute(
        &q,
        |_| Ok(report(GroupBy::Role, &["main", "sub:x"])),
        no_names,
    )
    .unwrap();
    let groups = out["groups"].as_array().unwrap();
    assert_eq!(groups[0]["role_kind"], "main");
    assert_eq!(groups[0]["agent"], Value::Null);
    assert_eq!(groups[1]["role_kind"], "sub");
    assert_eq!(groups[1]["agent"], "x");
    assert!(groups[1].get("deleted").is_none());
    let q = UsageQuery {
        group_by: GroupBy::Model,
        ..UsageQuery::default()
    };
    let out = usage::execute(&q, |_| Ok(report(GroupBy::Model, &["m1"])), no_names).unwrap();
    let group = out["groups"][0].as_object().unwrap();
    assert_eq!(group["key"], "m1");
    for key in ["role_kind", "agent", "kind", "deleted"] {
        assert!(!group.contains_key(key), "{key}");
    }
}

#[tokio::test]
async fn the_method_is_routed_in_every_binding() {
    assert!(super::methods::methods().contains(&"usage.query"));
    let route = super::methods::find("usage.query").unwrap();
    for state in binding::BindingState::ALL {
        assert!(route.check(state).is_ok());
    }
    let (events, _) = tokio::sync::watch::channel(None);
    let mut adapter = Adapter::new(events);
    let r = Request {
        id: "1".into(),
        method: "usage.query".into(),
        params: json!({"limit": 0}),
    };
    let e = dispatch(r, &mut adapter).await.unwrap_err();
    assert_eq!(e.code, "invalid_request");
    assert_eq!(e.extra().and_then(|x| x.reason), Some("out_of_range"));
}

#[test]
fn hello_advertises_the_usage_bounds_it_enforces() {
    let limits = super::protocol::limits();
    assert_eq!(limits["list"]["usage_groups"], query::MAX_GROUP_LIMIT);
    assert_eq!(limits["list"]["usage_buckets"], query::MAX_BUCKETS);
    assert_eq!(limits["time"]["usage_query_ms"], 2000);
}

// ── session_info.usage ─────────────────────────────────────────────────────

fn snapshot_event(tokens_in: u64, tokens_out: u64, cost: f64, context: bool) -> E {
    let context_usage = if context {
        json!({"prompt_tokens":tokens_in,"effective_window":131072,"estimated":false,
               "cached_tokens":4,"drss_active":true})
    } else {
        Value::Null
    };
    let value = json!({"foreground_id":"session","sessions":[{
     "id":"session","name":"n","cwd":"/work","messages":[],"streaming":null,"stream_reasoning":"",
     "tokens_in":tokens_in,"tokens_out":tokens_out,"tokens_cached":4,"cost":cost,"waiting":false,
     "awaiting_approval":false,"approval_reason":null,"pending_tool_calls":[],"tool_idx":0,
     "working":false,"finished_unseen":false,"subagents":[],"pending_subagents":[],
     "resolved_model_id":"m","effort":"","context_usage":context_usage
    }],"global":{
     "input":"","cursor":0,"scroll":0,"follow":true,"status":"","work_elapsed_ms":null,"theme":"dark","accent":"blue","palette":"default","mode":"Chat","toast":null,"models_cache":null,"models_cache_endpoint":null,"models_cache_failed":null,"agent_viewer":null,"agent_viewer_scroll":0,"agent_viewer_follow":true,"subagents_open":false,"subagent_sel":0,"palette_sel":0,"pending_attachments":[],"file_palette":null,"agent_mode":"auto","latest_version":null
    }});
    E::Snapshot(Box::new(serde_json::from_value(value).unwrap()))
}

#[test]
fn a_usage_change_emits_no_event_and_bumps_no_revision() {
    let mut r = reconcile::Reconcile::new("session".into());
    r.policy = Some(Policy::default());
    assert!(r.session_info(|_| None).usage.is_none());
    r.apply(&DaemonFrame {
        seq: 1,
        event: snapshot_event(100, 20, 0.25, true),
    });
    let first = r.state().unwrap();
    let usage = r.session_info(|_| None).usage.unwrap();
    assert_eq!(usage["last_prompt_tokens"], 100);
    assert_eq!(usage["last_cached_tokens"], 4);
    assert_eq!(usage["output_tokens_total"], 20);
    assert_eq!(usage["cost_total"], 0.25);
    assert_eq!(usage["context"]["effective_window"], 131072);
    assert_eq!(usage["context"]["cached_tokens"], 4);
    assert_eq!(usage["context"]["drss_active"], true);

    r.apply(&DaemonFrame {
        seq: 2,
        event: snapshot_event(900, 70, 1.5, false),
    });
    let second = r.state().unwrap();
    assert_eq!(second, first);
    assert_eq!(second.revision, first.revision);
    let mut previous = Some(first);
    let mut seq = 0;
    assert!(event_frames(&mut previous, second, &mut seq).is_empty());
    let usage = r.session_info(|_| None).usage.unwrap();
    assert_eq!(usage["last_prompt_tokens"], 900);
    assert_eq!(usage["output_tokens_total"], 70);
    assert_eq!(usage["cost_total"], 1.5);
    assert_eq!(usage["context"], Value::Null);
}

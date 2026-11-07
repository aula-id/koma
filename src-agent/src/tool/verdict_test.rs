use super::*;

fn stored(text: &str) -> Option<String> {
    Some(format!("sha256:{}", text.len()))
}

#[test]
fn a_typed_frame_round_trips() {
    let original = TypedResult {
        verdict: Verdict::Blocked,
        excerpt: "403 from waf".into(),
        handle: Some("rec-9".into()),
    };
    assert_eq!(parse(&original.to_json()), original);
}

#[test]
fn a_bare_string_is_untyped_and_preserved() {
    let got = parse("nmap said hello");
    assert_eq!(got.verdict, Verdict::Untyped);
    assert_eq!(got.excerpt, "nmap said hello");
}

#[test]
fn an_unknown_verdict_label_degrades_to_untyped() {
    // Forward compatibility: a newer daemon's new verdict must not break the
    // turn, it just cannot establish a finding here.
    let got = parse(r#"{"verdict":"quarantined","excerpt":"x"}"#);
    assert_eq!(got.verdict, Verdict::Untyped);
}

#[test]
fn a_frame_without_an_excerpt_is_still_typed() {
    let got = parse(r#"{"verdict":"not_found"}"#);
    assert_eq!(got.verdict, Verdict::NotFound);
    assert!(got.excerpt.is_empty());
}

#[test]
fn only_confirmed_establishes_a_finding() {
    assert!(Verdict::Confirmed.establishes_finding());
    for v in [
        Verdict::NotFound,
        Verdict::NeedsInput,
        Verdict::Blocked,
        Verdict::TimedOut,
        Verdict::Unavailable,
        Verdict::Error,
        Verdict::Untyped,
    ] {
        assert!(!v.establishes_finding(), "{v:?} must not confirm");
    }
}

#[test]
fn an_oversized_payload_is_held_behind_a_handle() {
    let long = TypedResult {
        verdict: Verdict::Confirmed,
        excerpt: "x".repeat(EXCERPT_SOFT_LIMIT + 1),
        handle: None,
    };
    let got = enforce_shape(long, stored);
    assert_eq!(got.verdict, Verdict::Confirmed);
    assert!(got.handle.is_some());
    assert!(got.excerpt.contains("held at"));
    assert!(got.excerpt.len() < EXCERPT_SOFT_LIMIT);
}

#[test]
fn a_long_array_is_summarised_by_count() {
    let rows: Vec<serde_json::Value> = (0..MAX_ARRAY_OBJECTS + 1)
        .map(|i| serde_json::json!({ "n": i }))
        .collect();
    let got = enforce_shape(
        TypedResult {
            verdict: Verdict::Confirmed,
            excerpt: serde_json::to_string(&rows).expect("serialisable"),
            handle: None,
        },
        stored,
    );
    assert!(got.excerpt.contains("objects held at"));
}

#[test]
fn a_small_result_is_untouched() {
    let small = TypedResult {
        verdict: Verdict::NotFound,
        excerpt: "nothing".into(),
        handle: None,
    };
    assert_eq!(enforce_shape(small.clone(), stored), small);
}

#[test]
fn a_failed_store_keeps_the_data() {
    let long = TypedResult {
        verdict: Verdict::Confirmed,
        excerpt: "y".repeat(EXCERPT_SOFT_LIMIT + 10),
        handle: None,
    };
    let got = enforce_shape(long, |_| None);
    assert!(got.handle.is_none());
    assert!(got.excerpt.contains("not held"));
    // The retained prefix is still there, so the model reads real bytes.
    assert!(got.excerpt.starts_with('y'));
}

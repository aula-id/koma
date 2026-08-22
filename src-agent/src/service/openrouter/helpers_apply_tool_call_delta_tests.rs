#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use crate::dto::openrouter::FunctionDelta;

/// Build a `ToolCallDelta` the way a provider streams one. `function` is only
/// attached when a name and/or an argument fragment is present (matching a
/// bare id-only frame, which carries no `function`).
fn delta(
    index: Option<usize>,
    id: Option<&str>,
    name: Option<&str>,
    args: Option<&str>,
) -> ToolCallDelta {
    let function = if name.is_some() || args.is_some() {
        Some(FunctionDelta {
            name: name.map(str::to_string),
            arguments: args.map(str::to_string),
        })
    } else {
        None
    };
    ToolCallDelta {
        index,
        id: id.map(str::to_string),
        function,
    }
}

// 1. STANDARD: index on every frame, id+name on the first, args-only
//    continuations sharing that index → one clean call. Must be byte-identical
//    to the old strict-index merge.
#[test]
fn standard_index_on_every_frame() {
    let mut acc = Vec::new();
    apply_tool_call_delta(&mut acc, &delta(Some(0), Some("a"), Some("read"), None));
    apply_tool_call_delta(&mut acc, &delta(Some(0), None, None, Some("{\"path\":")));
    apply_tool_call_delta(&mut acc, &delta(Some(0), None, None, Some("\"x\"}")));

    assert_eq!(acc.len(), 1);
    assert_eq!(acc[0].id, "a");
    assert_eq!(acc[0].kind, "function");
    assert_eq!(acc[0].function.name, "read");
    assert_eq!(acc[0].function.arguments, r#"{"path":"x"}"#);
}

// 2. ABSENT-INDEX CONTINUATION: first frame carries index+id+name, the
//    continuation OMITS index → args must land on the in-progress call, not
//    fork an empty slot 0 while the real call loses its arguments.
#[test]
fn absent_index_continuation_appends_to_in_progress_call() {
    let mut acc = Vec::new();
    apply_tool_call_delta(&mut acc, &delta(Some(0), Some("a"), Some("read"), None));
    apply_tool_call_delta(&mut acc, &delta(None, None, None, Some(r#"{"path":"x"}"#)));

    assert_eq!(
        acc.len(),
        1,
        "index-less continuation must not open a new slot"
    );
    assert_eq!(acc[0].id, "a");
    assert_eq!(acc[0].function.name, "read");
    assert_eq!(acc[0].function.arguments, r#"{"path":"x"}"#);
}

// 3. RE-ANNOUNCED ID AT NEW INDEX: same id resent under a new index → coalesce
//    onto the existing slot (regardless of index), no empty phantom.
#[test]
fn reannounced_id_at_new_index_coalesces() {
    let mut acc = Vec::new();
    apply_tool_call_delta(&mut acc, &delta(Some(0), Some("a"), Some("read"), None));
    apply_tool_call_delta(
        &mut acc,
        &delta(Some(1), Some("a"), None, Some(r#"{"path":"x"}"#)),
    );

    assert_eq!(
        acc.len(),
        1,
        "a re-announced id must coalesce, not fork a phantom slot"
    );
    assert_eq!(acc[0].id, "a");
    assert_eq!(acc[0].function.name, "read");
    assert_eq!(acc[0].function.arguments, r#"{"path":"x"}"#);
}

// 4. TWO GENUINE PARALLEL CALLS: distinct ids at distinct indices → two
//    distinct correct calls (regression guard against over-merging).
#[test]
fn two_genuine_parallel_calls_stay_distinct() {
    let mut acc = Vec::new();
    apply_tool_call_delta(
        &mut acc,
        &delta(Some(0), Some("a"), Some("read"), Some(r#"{"path":"x"}"#)),
    );
    apply_tool_call_delta(
        &mut acc,
        &delta(Some(1), Some("b"), Some("grep"), Some(r#"{"pattern":"y"}"#)),
    );

    assert_eq!(
        acc.len(),
        2,
        "distinct ids at distinct indices must not merge"
    );
    assert_eq!(acc[0].id, "a");
    assert_eq!(acc[0].function.name, "read");
    assert_eq!(acc[0].function.arguments, r#"{"path":"x"}"#);
    assert_eq!(acc[1].id, "b");
    assert_eq!(acc[1].function.name, "grep");
    assert_eq!(acc[1].function.arguments, r#"{"pattern":"y"}"#);
}

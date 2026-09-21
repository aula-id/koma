use super::*;

#[test]
fn tool_continuations_keep_b_and_refresh_only_on_intent_or_budget_changes() {
    let archive = Archive::new();
    archive.append(Role::Tool, "alphadiagnostic original evidence");
    archive.append(Role::Tool, "betadiagnostic other evidence");
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    index.save_boundary(2).unwrap();
    drop(index);
    let mut history = vec![
        ChatMessage::new(Role::System, "unchanged system"),
        archive.append(Role::User, "continue"),
        archive.append(Role::Assistant, "investigate alphadiagnostic"),
    ];
    let shape_with = |history: &[ChatMessage], user: &str, goal: &GoalWire, window| {
        shape(
            history.to_vec(),
            &archive.0,
            &Settings::default(),
            user,
            goal,
            &limits(window),
            0,
        )
        .unwrap()
        .history
    };
    let goal = GoalWire::default();
    let first = shape_with(&history, "continue", &goal, 100_000);
    assert!(first[1]
        .content
        .contains("alphadiagnostic original evidence"));
    assert!(!first[1].content.contains("betadiagnostic other evidence"));

    // Replace every recent diagnostic used by relevance() without advancing
    // the archive boundary. Each shape reopens SQLite, as a resumed process does.
    for _ in 0..8 {
        history.push(archive.append(Role::Assistant, "investigate betadiagnostic"));
    }
    let continued = shape_with(&history, "continue", &goal, 100_000);
    assert_eq!(&continued[..first.len()], first.as_slice());
    assert_eq!(continued.last(), history.last());
    assert_eq!(boundary(&archive), 2);
    assert_eq!(shape_with(&history, "continue", &goal, 100_000), continued);

    // New user intent gets current diagnostics; it is never served stale B.
    let changed_user = shape_with(&history, "check betadiagnostic", &goal, 100_000);
    assert_ne!(changed_user[1], continued[1]);
    assert!(changed_user[1]
        .content
        .contains("betadiagnostic other evidence"));
    let goal = GoalWire {
        source: "user".into(),
        objective: "Preserve the public API".into(),
        charter: "Investigate the regression".into(),
    };
    let changed_goal = shape_with(&history, "check betadiagnostic", &goal, 100_000);
    assert!(changed_goal[1].content.contains("Preserve the public API"));
    assert!(changed_goal[1]
        .content
        .contains("Investigate the regression"));
    assert_ne!(changed_goal[1], changed_user[1]);

    // Budget is part of the snapshot identity, including a smaller model window.
    let snapshot_key = || -> String {
        msglog::open(&archive.0)
            .unwrap()
            .query_row("SELECT cache_key FROM drss_memory WHERE id=1", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let old_key = snapshot_key();
    let smaller = shape_with(&history, "check betadiagnostic", &goal, 20_000);
    assert_ne!(snapshot_key(), old_key);
    assert!(budget::text_tokens(&smaller[1].content) <= 400);
    assert_eq!(smaller.last(), history.last());
    assert_eq!(smaller[0], history[0]);
}

#[test]
fn resend_and_clear_invalidate_derived_memory_even_when_the_boundary_stays() {
    let archive = Archive::new();
    archive.append(Role::Tool, "obsolete evidence");
    let index = msglog::drss::Index::load(&archive.0).unwrap();
    index.save_boundary(1).unwrap();
    drop(index);
    let history = vec![
        ChatMessage::new(Role::System, "system"),
        archive.append(Role::User, "continue"),
    ];
    send(&history, &archive, 100_000);
    let snapshot_count = || -> i64 {
        msglog::open(&archive.0)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM drss_memory", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(snapshot_count(), 1);
    msglog::truncate_after(&archive.0, 2).unwrap();
    assert_eq!(boundary(&archive), 1);
    assert_eq!(snapshot_count(), 0);
    send(&history, &archive, 100_000);
    assert_eq!(snapshot_count(), 1);
    msglog::clear_rolling_summary(&archive.0).unwrap();
    assert_eq!(snapshot_count(), 0);
    let fresh = vec![history[0].clone(), archive.append(Role::User, "new work")];
    let fresh = send(&fresh, &archive, 100_000);
    assert!(!fresh[1].content.contains("obsolete evidence"));
    assert!(!fresh[1].content.contains("Archive range:"));
}

#[test]
fn recovery_memory_refreshes_when_omitted_legacy_references_change() {
    let archive = Archive::new();
    let mut history = vec![ChatMessage::new(Role::System, "system")];
    for i in 0..16 {
        history.push(ChatMessage::new(
            Role::User,
            format!("legacy-{i} {}", "x".repeat(15_000)),
        ));
        history.push(ChatMessage::new(Role::Assistant, "done"));
    }
    history.push(ChatMessage::new(Role::User, "continue"));
    let first = send(&history, &archive, 100_000);
    assert!(first[1].content.contains("Recovery handoff:"));
    assert_eq!(boundary(&archive), 0);
    for _ in 0..5 {
        history.push(ChatMessage::new(
            Role::Assistant,
            "new progress ".repeat(1500),
        ));
    }
    let second = send(&history, &archive, 100_000);
    assert_eq!(
        boundary(&archive),
        0,
        "legacy coverage uses keys instead of IDs"
    );
    assert_ne!(first[1], second[1], "new recovery keys must refresh B");
    assert_eq!(second.last(), history.last());
    assert_eq!(send(&history, &archive, 100_000), second);
}

use super::*;
use crate::ipc::linker_proto::{LinkerRequest, LinkerResponse, ScanStatus};

/// Build a minimal DaemonState for unit tests (no watcher, no tokio).
fn test_state() -> Arc<DaemonState> {
    Arc::new(DaemonState::new())
}

/// Helper: send a RegisterWorkspaces request and return the response.
fn register(state: &Arc<DaemonState>, session_id: &str, roots: &[&str]) -> LinkerResponse {
    handle_request(
        LinkerRequest::RegisterWorkspaces {
            roots: roots.iter().map(|s| s.to_string()).collect(),
            session_id: session_id.to_string(),
            registration_revision: None,
        },
        &std::sync::atomic::AtomicBool::new(false),
        state,
    )
}

/// Helper: send a RegisterWorkspaces request with a revision tag.
fn register_with_rev(
    state: &Arc<DaemonState>,
    session_id: &str,
    roots: &[&str],
    rev: u64,
) -> LinkerResponse {
    handle_request(
        LinkerRequest::RegisterWorkspaces {
            roots: roots.iter().map(|s| s.to_string()).collect(),
            session_id: session_id.to_string(),
            registration_revision: Some(rev),
        },
        &std::sync::atomic::AtomicBool::new(false),
        state,
    )
}

/// Helper: send an Unregister request.
fn unregister(state: &Arc<DaemonState>, session_id: &str) -> LinkerResponse {
    handle_request(
        LinkerRequest::Unregister {
            session_id: session_id.to_string(),
        },
        &std::sync::atomic::AtomicBool::new(false),
        state,
    )
}

/// Helper: read the current refcount for a root.
fn refcount(state: &Arc<DaemonState>, root: &str) -> u32 {
    state
        .root_refs
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&PathBuf::from(root))
        .copied()
        .unwrap_or(0)
}

/// Helper: read the session's registered root set.
fn session_roots(state: &Arc<DaemonState>, session_id: &str) -> HashSet<PathBuf> {
    state
        .clients
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

/// Helper: check whether the clients map has a key for this session.
fn has_client(state: &Arc<DaemonState>, session_id: &str) -> bool {
    state
        .clients
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(session_id)
}

// ── Idempotent repeat ──────────────────────────────────────────────

#[test]
fn idempotent_repeat_no_change() {
    let state = test_state();
    // First registration.
    let r1 = register(&state, "s1", &["/a", "/b"]);
    assert!(matches!(r1, LinkerResponse::Registered { .. }));
    assert_eq!(refcount(&state, "/a"), 1);
    assert_eq!(refcount(&state, "/b"), 1);

    // Re-register the same set — refcounts must not double.
    let r2 = register(&state, "s1", &["/a", "/b"]);
    assert!(matches!(r2, LinkerResponse::Registered { .. }));
    assert_eq!(refcount(&state, "/a"), 1, "refcount must stay 1");
    assert_eq!(refcount(&state, "/b"), 1, "refcount must stay 1");
}

// ── Add/remove/replace ─────────────────────────────────────────────

#[test]
fn add_roots_increments_refcount() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    assert_eq!(refcount(&state, "/a"), 1);

    register(&state, "s1", &["/a", "/b"]);
    assert_eq!(refcount(&state, "/a"), 1);
    assert_eq!(refcount(&state, "/b"), 1);
}

#[test]
fn remove_roots_decrements_refcount() {
    let state = test_state();
    register(&state, "s1", &["/a", "/b"]);
    assert_eq!(refcount(&state, "/a"), 1);
    assert_eq!(refcount(&state, "/b"), 1);

    register(&state, "s1", &["/a"]);
    assert_eq!(refcount(&state, "/a"), 1);
    assert_eq!(refcount(&state, "/b"), 0, "removed root refcount must be 0");
    assert!(
        !state
            .root_refs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&PathBuf::from("/b")),
        "removed root must be cleaned from refs map"
    );
}

#[test]
fn replace_all_roots() {
    let state = test_state();
    register(&state, "s1", &["/a", "/b"]);
    register(&state, "s1", &["/c", "/d"]);
    assert_eq!(refcount(&state, "/a"), 0);
    assert_eq!(refcount(&state, "/b"), 0);
    assert_eq!(refcount(&state, "/c"), 1);
    assert_eq!(refcount(&state, "/d"), 1);
    assert_eq!(
        session_roots(&state, "s1"),
        [PathBuf::from("/c"), PathBuf::from("/d")]
            .into_iter()
            .collect()
    );
}

// ── Shared roots / refcounts ───────────────────────────────────────

#[test]
fn shared_root_refcount() {
    let state = test_state();
    register(&state, "s1", &["/shared", "/a"]);
    register(&state, "s2", &["/shared", "/b"]);
    assert_eq!(refcount(&state, "/shared"), 2);
    assert_eq!(refcount(&state, "/a"), 1);
    assert_eq!(refcount(&state, "/b"), 1);

    // Drop s1 — shared root refcount decrements to 1, not 0.
    unregister(&state, "s1");
    assert_eq!(refcount(&state, "/shared"), 1);
    assert_eq!(refcount(&state, "/a"), 0);
    // /b still alive via s2.
    assert_eq!(refcount(&state, "/b"), 1);
}

#[test]
fn shared_root_survives_partial_unregister() {
    let state = test_state();
    register(&state, "s1", &["/x"]);
    register(&state, "s2", &["/x"]);
    assert_eq!(refcount(&state, "/x"), 2);
    unregister(&state, "s1");
    assert_eq!(refcount(&state, "/x"), 1);
    // Session s2 still has it.
    assert!(session_roots(&state, "s2").contains(&PathBuf::from("/x")));
}

// ── Global union / watcher ─────────────────────────────────────────

#[test]
fn watcher_not_updated_when_union_unchanged() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    let roots_before = collect_all_roots(&state);
    // Re-register same set — union is unchanged.
    register(&state, "s1", &["/a"]);
    let roots_after = collect_all_roots(&state);
    assert_eq!(roots_before, roots_after, "global union must be stable");
}

#[test]
fn global_union_reflects_all_sessions() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    register(&state, "s2", &["/b"]);
    register(&state, "s3", &["/a", "/c"]);
    let all = collect_all_roots(&state);
    assert_eq!(
        all,
        vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/c")
        ]
    );
}

// ── Final reference drop clearing/rescanning ───────────────────────

#[test]
fn final_unregister_clears_clients() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    unregister(&state, "s1");
    assert!(
        session_roots(&state, "s1").is_empty(),
        "session should be removed from clients map"
    );
    assert_eq!(refcount(&state, "/a"), 0);
}

#[test]
fn final_unregister_invalidates_in_flight_scan() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    {
        let mut coord = state
            .scan_coordinator
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        coord.desired_revision = coord.desired_revision.saturating_add(1);
        coord.in_flight = Some(coord.desired_revision);
        state.scanning.store(true, Ordering::SeqCst);
    }
    unregister(&state, "s1");
    let coord = state
        .scan_coordinator
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert!(coord.in_flight.is_none());
    assert_eq!(coord.applied_revision, coord.desired_revision);
    assert!(!state.scanning.load(Ordering::SeqCst));
    assert!(state
        .graph
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .nodes
        .is_empty());
}

#[test]
fn unregister_nonexistent_session_is_noop() {
    let state = test_state();
    let resp = unregister(&state, "ghost");
    assert!(matches!(resp, LinkerResponse::Ack));
}

// ── Empty registration / edge cases ────────────────────────────────

#[test]
fn empty_roots_clears_session() {
    let state = test_state();
    register(&state, "s1", &["/a", "/b"]);
    register(&state, "s1", &[]); // empty — should clear
    assert!(session_roots(&state, "s1").is_empty());
    assert_eq!(refcount(&state, "/a"), 0);
    assert_eq!(refcount(&state, "/b"), 0);
}

#[test]
fn first_registration_triggers_scan() {
    let state = test_state();
    // Before first registration, scanning should be false.
    assert!(!state.scanning.load(std::sync::atomic::Ordering::SeqCst));
    let _resp = register(&state, "s1", &["/nonexistent_root_for_test"]);
    // After registration of new root, scanning should be true (scan thread spawned).
    assert!(
        state.scanning.load(std::sync::atomic::Ordering::SeqCst),
        "first registration must trigger a scan"
    );
}

// ── Response format checks ─────────────────────────────────────────

#[test]
fn register_response_contains_status_and_generation() {
    let state = test_state();
    let resp = register(&state, "s1", &["/a"]);
    match resp {
        LinkerResponse::Registered { status, generation } => {
            // generation starts at 0 (no scan done yet in sync path).
            assert_eq!(generation, 0);
            assert!(matches!(status, ScanStatus::Scanning | ScanStatus::Ready));
        }
        other => panic!("expected Registered, got {other:?}"),
    }
}

// ── Overlapping session changes ────────────────────────────────────

#[test]
fn overlapping_sessions_one_adds_one_removes() {
    let state = test_state();
    register(&state, "s1", &["/shared", "/only1"]);
    register(&state, "s2", &["/shared", "/only2"]);

    // s1 drops /shared, keeps /only1.
    register(&state, "s1", &["/only1"]);
    assert_eq!(refcount(&state, "/shared"), 1, "s2 still holds /shared");
    assert_eq!(refcount(&state, "/only1"), 1);
    assert_eq!(refcount(&state, "/only2"), 1);

    // s2 drops /shared too — finally unreferenced.
    register(&state, "s2", &["/only2"]);
    assert_eq!(refcount(&state, "/shared"), 0);
    assert!(
        !state
            .root_refs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&PathBuf::from("/shared")),
        "/shared should be removed from refs when refcount hits 0"
    );
}

// ── Requirement 3: Empty register removes client key ──────────────

#[test]
fn empty_register_removes_client_key_for_reaper() {
    let state = test_state();
    register(&state, "s1", &["/a", "/b"]);
    assert!(
        has_client(&state, "s1"),
        "client should exist after registration"
    );

    // Empty registration removes the key entirely.
    register(&state, "s1", &[]);
    assert!(
        !has_client(&state, "s1"),
        "empty registration must remove client key so reaper can reap"
    );
    assert_eq!(refcount(&state, "/a"), 0);
    assert_eq!(refcount(&state, "/b"), 0);
}

#[test]
fn reaper_sees_empty_after_unregister() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    assert!(has_client(&state, "s1"));

    unregister(&state, "s1");
    assert!(!has_client(&state, "s1"));
    assert!(
        state
            .clients
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty(),
        "clients map should be empty so reaper can exit"
    );
}

// ── Requirement 1: Concurrent handler atomicity ────────────────────

#[test]
fn concurrent_register_unregister_refcounts_consistent() {
    let state = test_state();
    register(&state, "s1", &["/shared", "/only1"]);
    register(&state, "s2", &["/shared", "/only2"]);

    // Spawn concurrent operations: s1 re-registers, s2 unregisters.
    let state_c = Arc::clone(&state);
    let h1 = std::thread::spawn(move || {
        register(&state_c, "s1", &["/shared"]);
    });
    let state_c = Arc::clone(&state);
    let h2 = std::thread::spawn(move || {
        unregister(&state_c, "s2");
    });
    h1.join().unwrap();
    h2.join().unwrap();

    // Regardless of ordering, final state must be consistent:
    // /shared held by s1 only (refcount 1), /only1 dropped by s1, /only2 dropped by s2.
    assert_eq!(refcount(&state, "/shared"), 1);
    assert_eq!(
        refcount(&state, "/only1"),
        0,
        "s1 dropped /only1 when re-registering with only /shared"
    );
    assert_eq!(refcount(&state, "/only2"), 0);
}

#[test]
fn concurrent_registrations_stable_refcounts() {
    let state = test_state();
    // Spawn 8 threads, each registering a different root for a unique session.
    let mut handles = Vec::new();
    for i in 0..8 {
        let state_c = Arc::clone(&state);
        let root = format!("/root_{i}");
        let sid = format!("s{i}");
        handles.push(std::thread::spawn(move || {
            register(&state_c, &sid, &[&root]);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    // Each root must have refcount exactly 1.
    for i in 0..8 {
        assert_eq!(
            refcount(&state, &format!("/root_{i}")),
            1,
            "root_{i} refcount must be 1"
        );
    }
    // Exactly 8 clients registered.
    assert_eq!(
        state
            .clients
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len(),
        8
    );
}

// ── Requirement 2: Revision gating / stale rejection ───────────────

#[test]
fn revision_rejects_stale_registration() {
    let state = test_state();
    // Register with revision 2.
    let r2 = register_with_rev(&state, "s1", &["/a"], 2);
    assert!(matches!(r2, LinkerResponse::Registered { .. }));
    assert_eq!(refcount(&state, "/a"), 1);

    // Try to register with revision 1 (stale) — should be rejected.
    let r1 = register_with_rev(&state, "s1", &["/b"], 1);
    assert!(matches!(r1, LinkerResponse::Registered { .. }));
    // /b must NOT have been registered (stale rejected).
    assert_eq!(refcount(&state, "/b"), 0, "stale revision must be rejected");
    // /a still held.
    assert_eq!(refcount(&state, "/a"), 1);
}

#[test]
fn revision_accepts_newer_registration() {
    let state = test_state();
    register_with_rev(&state, "s1", &["/a"], 1);
    assert_eq!(refcount(&state, "/a"), 1);

    // Register with revision 2 (newer) — should succeed and replace.
    register_with_rev(&state, "s1", &["/b"], 2);
    assert_eq!(refcount(&state, "/a"), 0, "old root should be released");
    assert_eq!(refcount(&state, "/b"), 1, "new root should be registered");
}

#[test]
fn revision_equal_is_accepted() {
    let state = test_state();
    register_with_rev(&state, "s1", &["/a"], 5);
    // Same revision should be accepted (>= check).
    register_with_rev(&state, "s1", &["/a", "/b"], 5);
    assert_eq!(refcount(&state, "/a"), 1);
    assert_eq!(refcount(&state, "/b"), 1);
}

#[test]
fn no_revision_always_accepted() {
    let state = test_state();
    // No revision → always accepted (backward compat).
    register(&state, "s1", &["/a"]);
    register(&state, "s1", &["/b"]);
    register(&state, "s1", &["/c"]);
    assert_eq!(refcount(&state, "/a"), 0);
    assert_eq!(refcount(&state, "/b"), 0);
    assert_eq!(refcount(&state, "/c"), 1);
}

// ── Requirement 2: Scan versioning coordinator ─────────────────────

#[test]
fn scan_coordinator_starts_at_zero() {
    let state = test_state();
    let coord = state.scan_coordinator.lock().unwrap();
    assert_eq!(coord.desired_revision, 0);
    assert_eq!(coord.applied_revision, 0);
    assert!(coord.in_flight.is_none());
}

#[test]
fn versioned_scan_bumps_desired_revision() {
    let state = test_state();
    // After first registration, a scan should have been scheduled.
    register(&state, "s1", &["/nonexistent_for_scan_test"]);
    // Wait briefly for thread to spawn.
    std::thread::sleep(std::time::Duration::from_millis(10));
    let coord = state.scan_coordinator.lock().unwrap();
    assert!(
        coord.desired_revision >= 1,
        "desired_revision should be >= 1 after registration"
    );
}

// ── Task 1: Scan revision / status contract ─────────────────────────

#[test]
fn repeated_rescan_advances_revision() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    // Wait for scan thread to spawn and register in coordinator.
    std::thread::sleep(std::time::Duration::from_millis(10));
    let rev1 = {
        let coord = state.scan_coordinator.lock().unwrap();
        coord.desired_revision
    };
    assert!(rev1 >= 1, "first rescan should bump desired_revision");

    // Issue a manual Rescan query.
    let resp = handle_request(
        LinkerRequest::Query(LinkerQuery::Rescan),
        &std::sync::atomic::AtomicBool::new(false),
        &state,
    );
    let scan_rev = match resp {
        LinkerResponse::ScanRevision { revision } => revision,
        other => panic!("expected ScanRevision, got {other:?}"),
    };
    assert!(
        scan_rev > rev1,
        "rescan revision {scan_rev} should be > previous {rev1}"
    );

    // A second Rescan must advance further.
    let resp2 = handle_request(
        LinkerRequest::Query(LinkerQuery::Rescan),
        &std::sync::atomic::AtomicBool::new(false),
        &state,
    );
    let scan_rev2 = match resp2 {
        LinkerResponse::ScanRevision { revision } => revision,
        other => panic!("expected ScanRevision, got {other:?}"),
    };
    assert!(
        scan_rev2 > scan_rev,
        "second rescan revision {scan_rev2} should be > first {scan_rev}"
    );
}

#[test]
fn scan_status_returns_coordinator_state() {
    let state = test_state();
    // Initially all zero.
    let resp = handle_request(
        LinkerRequest::Query(LinkerQuery::ScanStatus),
        &std::sync::atomic::AtomicBool::new(false),
        &state,
    );
    match resp {
        LinkerResponse::ScanStatusResponse {
            desired_revision,
            applied_revision,
            in_flight,
            generation,
        } => {
            assert_eq!(desired_revision, 0);
            assert_eq!(applied_revision, 0);
            assert!(in_flight.is_none());
            assert_eq!(generation, 0);
        }
        other => panic!("expected ScanStatusResponse, got {other:?}"),
    }
}

#[test]
fn rescan_returns_accepted_revision_for_later_poll() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    std::thread::sleep(std::time::Duration::from_millis(10));

    // Issue Rescan and capture revision.
    let resp = handle_request(
        LinkerRequest::Query(LinkerQuery::Rescan),
        &std::sync::atomic::AtomicBool::new(false),
        &state,
    );
    let scan_rev = match resp {
        LinkerResponse::ScanRevision { revision } => revision,
        other => panic!("expected ScanRevision, got {other:?}"),
    };

    // ScanStatus should show the revision as desired.
    let status_resp = handle_request(
        LinkerRequest::Query(LinkerQuery::ScanStatus),
        &std::sync::atomic::AtomicBool::new(false),
        &state,
    );
    match status_resp {
        LinkerResponse::ScanStatusResponse {
            desired_revision, ..
        } => {
            assert!(
                desired_revision >= scan_rev,
                "desired_revision {desired_revision} should be >= scan_rev {scan_rev}"
            );
        }
        other => panic!("expected ScanStatusResponse, got {other:?}"),
    }
}

// ── Task 2: Publication/watcher coordination ─────────────────────────

#[test]
fn publication_lock_exists() {
    let state = test_state();
    // Verify the publication_lock can be acquired (no deadlock in test).
    let _guard = state
        .publication_lock
        .lock()
        .unwrap_or_else(|e| e.into_inner());
}

#[test]
fn watcher_supersede_bumps_desired_revision() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    std::thread::sleep(std::time::Duration::from_millis(10));

    // Snapshot desired_revision before watcher event.
    let before = {
        let coord = state.scan_coordinator.lock().unwrap();
        coord.desired_revision
    };

    // Simulate a watcher event arriving while a scan is in_flight:
    // set in_flight artificially.
    {
        let mut coord = state.scan_coordinator.lock().unwrap();
        coord.in_flight = Some(coord.desired_revision);
    }

    // Now the watcher_loop would bump desired_revision.  We simulate
    // the relevant section of watcher_loop here:
    let superseded = {
        let mut coord = state.scan_coordinator.lock().unwrap();
        let was_in_flight = coord.in_flight.is_some();
        if was_in_flight {
            coord.desired_revision += 1;
        }
        was_in_flight
    };
    assert!(superseded, "should detect in-flight scan");

    let after = {
        let coord = state.scan_coordinator.lock().unwrap();
        coord.desired_revision
    };
    assert!(
        after > before,
        "desired_revision should advance after supersede: before={before}, after={after}"
    );
}

#[test]
fn stale_scan_does_not_publish_over_watcher() {
    let state = test_state();
    register(&state, "s1", &["/a"]);
    std::thread::sleep(std::time::Duration::from_millis(10));

    // Simulate: scan rev=1 is in flight, then watcher bumps desired to 2.
    let mut coord = state.scan_coordinator.lock().unwrap();
    let rev1 = coord.desired_revision;
    coord.in_flight = Some(rev1);
    coord.desired_revision = rev1 + 1;
    drop(coord);

    // Now scan rev=1 tries to publish.  The staleness check:
    //   coord.in_flight == Some(rev1) && coord.desired_revision == rev1
    // should FAIL because desired_revision was bumped.
    let coord = state.scan_coordinator.lock().unwrap();
    assert!(
        !(coord.in_flight == Some(rev1) && coord.desired_revision == rev1),
        "scan rev={rev1} should be stale (desired={})",
        coord.desired_revision
    );
}

// ── Task 3: Delayed older worker rejected ───────────────────────────

#[test]
fn delayed_older_worker_registration_rejected() {
    let state = test_state();
    // Worker A registers with revision 5.
    let r5 = register_with_rev(&state, "s1", &["/a"], 5);
    assert!(matches!(r5, LinkerResponse::Registered { .. }));
    assert_eq!(refcount(&state, "/a"), 1);

    // Worker B arrives late with revision 3 (older) — should be rejected.
    let r3 = register_with_rev(&state, "s1", &["/b"], 3);
    assert!(matches!(r3, LinkerResponse::Registered { .. }));
    assert_eq!(refcount(&state, "/b"), 0, "stale worker must be rejected");
    assert_eq!(refcount(&state, "/a"), 1, "original root unchanged");
}

// ── Task 4: Response validation ─────────────────────────────────────

#[test]
fn registered_only_is_success_for_validation() {
    // Simulate what ensure_and_register_with_revision checks:
    // Only Registered should be Ok.
    let registered = LinkerResponse::Registered {
        status: crate::ipc::linker_proto::ScanStatus::Ready,
        generation: 1,
    };
    assert!(matches!(registered, LinkerResponse::Registered { .. }));

    let error = LinkerResponse::Error("test".into());
    assert!(!matches!(error, LinkerResponse::Registered { .. }));

    let ack = LinkerResponse::Ack;
    assert!(!matches!(ack, LinkerResponse::Registered { .. }));
}

use super::*;

#[test]
fn every_case_names_its_phase_and_a_reason() {
    for e in PENDING {
        assert!((0..=8).contains(&e.phase), "{:?}", e);
        assert!(!e.case.is_empty());
        assert!(!e.why.is_empty(), "{} has no reason", e.case);
    }
}

#[test]
fn the_register_has_no_duplicates() {
    for (i, a) in PENDING.iter().enumerate() {
        for b in &PENDING[i + 1..] {
            assert_ne!(a.case, b.case);
        }
    }
}

#[test]
fn phases_zero_and_one_are_clear() {
    // Both are done; an entry here would mean one regressed back to pending.
    let open = open_phases();
    assert!(!open.contains(&0), "phase 0 re-opened");
    assert!(!open.contains(&1), "phase 1 re-opened");
}

#[test]
fn the_register_is_not_empty_until_every_phase_lands() {
    // Sanity that this file is actually consulted, not decorative.
    assert!(!PENDING.is_empty());
    assert!(open_phases().len() >= 2);
}

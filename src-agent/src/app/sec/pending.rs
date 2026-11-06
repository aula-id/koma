//! The pending register.
//!
//! `21` requires future cases to be VISIBLE as pending, with the phase that owns
//! them, rather than silently absent. A `#[ignore]` with no phase is a gap nobody
//! tracks; these carry the phase in the name and the reason in the message, and a
//! test fails if the register drifts from something it should have caught.
//!
//! An entry leaves this file only when it is genuinely green, not when it is
//! inconvenient.

/// A deferred case and the phase that owns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub phase: u8,
    pub case: &'static str,
    pub why: &'static str,
}

/// Every deferred case. Shrinks as phases land.
pub const PENDING: &[Pending] = &[
    Pending {
        phase: 2,
        case: "malformed capability metadata fails closed",
        why: "wire.rs still defaults missing metadata rather than refusing it",
    },
    Pending {
        phase: 2,
        case: "literal delimiter text in tool output cannot escape its wrapper",
        why: "the central untrusted-data boundary is not built",
    },
    Pending {
        phase: 3,
        case: "dns rebinding, direct ipv6 and proxy-failure fixtures",
        why: "unit-level bypass fixtures over the proxy are outstanding",
    },
    Pending {
        phase: 3,
        case: "scope time window",
        why: "Scope carries rate and total caps but no window",
    },
    Pending {
        phase: 3,
        case: "browser control channel authentication and lifecycle",
        why: "the browser runs host-side against the proxy; no control channel yet",
    },
    Pending {
        phase: 6,
        case: "board verdict and scope/radius records",
        why: "board kinds stop at claim/coverage; verdicts live only on evidence",
    },
    Pending {
        phase: 7,
        case: "sec_decrypt, sec_replay, sec_recording, sec_xss",
        why: "four of the eight phase-7 operations are not implemented",
    },
    Pending {
        phase: 8,
        case: "safeguard progress signal and the shared role spine",
        why: "prompts are per-lane; nothing signals progress to supervision",
    },
    Pending {
        phase: 8,
        case: "blue detector regression: version-only detector must fail",
        why: "no detector exists yet",
    },
    Pending {
        phase: 8,
        case: "scope rides the volatile tail, not the cached head",
        why: "scope is in the prompt body; cache placement is unverified",
    },
];

/// The phases that still own at least one case.
#[must_use]
pub fn open_phases() -> Vec<u8> {
    let mut p: Vec<u8> = PENDING.iter().map(|e| e.phase).collect();
    p.sort_unstable();
    p.dedup();
    p
}

#[cfg(test)]
mod tests {
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
}

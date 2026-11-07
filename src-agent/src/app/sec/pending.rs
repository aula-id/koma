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
        case: "untrusted framing for non-sec tool output",
        why: "sec_ results are framed at dispatch; ordinary tool output is not",
    },
    Pending {
        phase: 3,
        case: "dns rebinding, direct ipv6 and proxy-failure fixtures",
        why: "unit-level bypass fixtures over the proxy are outstanding",
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
#[path = "pending_test.rs"]
mod tests;


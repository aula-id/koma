//! Supervision: the limits a red run is held to, enforced in-process.
//!
//! Red runs are the unsafe lane, so the brakes cannot be a sentence in a prompt.
//! Three things are counted here and each has one job:
//!
//! - **hypotheses** — the cap is a forcing function. Over the cap the run must
//!   `close` one before opening another, so it cannot accumulate a pile of
//!   unvalidated guesses and call that progress.
//! - **turns** — the budget. At the soft limit the supervisor warns once; at the
//!   hard limit the run is paused rather than being allowed to finish a thought
//!   that has already overrun.
//! - **requests** — fed by the tool call site so the rate and total caps from the
//!   scope record are applied per second and per run.
//!
//! A pause is a state, not an exception thread: the caller asks [`Supervisor::gate`]
//! before dispatching, and a paused run refuses with `Blocked` naming the reason.
//! That keeps every decision on the agent's own thread.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Defaults, overridable per run. Small on purpose: a red run should be cheap to
/// stop and cheap to restart.
pub const DEFAULT_HYPOTHESIS_CAP: u32 = 5;
pub const DEFAULT_TURN_BUDGET: u32 = 40;
/// Turns left when the supervisor first warns.
pub const WARN_AT_REMAINING: u32 = 8;
/// One second of window for the rate cap.
const RATE_WINDOW: Duration = Duration::from_secs(1);

/// Why the supervisor stopped the run.
#[derive(Debug, Clone, PartialEq)]
pub enum Pause {
    /// The hypothesis cap is reached and nothing was closed.
    HypothesisCap(u32),
    /// The turn budget is spent.
    TurnBudget(u32),
    /// The per-second cap is reached.
    Rate(f32),
    /// The run's total request cap is spent.
    RequestTotal(u32),
}

impl Pause {
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::HypothesisCap(cap) => {
                format!("hypothesis cap reached ({cap}); close one before opening another")
            }
            Self::TurnBudget(n) => format!("turn budget spent ({n})"),
            Self::Rate(rps) => format!("rate cap reached ({rps}/s)"),
            Self::RequestTotal(n) => format!("request total spent ({n})"),
        }
    }
}

/// One hypothesis slot. `closed` is what the cap actually counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hypothesis {
    pub id: String,
    pub closed: bool,
}

pub struct Supervisor {
    hypothesis_cap: u32,
    turn_budget: u32,
    turns: u32,
    hypothesis: Vec<Hypothesis>,
    rate: Option<f32>,
    total: Option<u32>,
    sent: u32,
    stamps: VecDeque<Instant>,
    paused: Option<Pause>,
    warned: bool,
    /// The clock, injectable so tests do not sleep.
    now: Box<dyn Fn() -> Instant + Send + Sync>,
}

impl std::fmt::Debug for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The clock is a boxed closure and cannot derive Debug.
        f.debug_struct("Supervisor")
            .field("turns", &self.turns)
            .field("sent", &self.sent)
            .field("paused", &self.paused)
            .field("hypothesis", &self.hypothesis.len())
            .finish()
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new(DEFAULT_HYPOTHESIS_CAP, DEFAULT_TURN_BUDGET, None, None)
    }
}

impl Supervisor {
    #[must_use]
    pub fn new(
        hypothesis_cap: u32,
        turn_budget: u32,
        rate: Option<f32>,
        total: Option<u32>,
    ) -> Self {
        Self {
            hypothesis_cap,
            turn_budget,
            turns: 0,
            hypothesis: Vec::new(),
            rate,
            total,
            sent: 0,
            stamps: VecDeque::new(),
            paused: None,
            warned: false,
            now: Box::new(Instant::now),
        }
    }

    /// Replace the clock. Test seam.
    pub fn with_clock(mut self, f: Box<dyn Fn() -> Instant + Send + Sync>) -> Self {
        self.now = f;
        self
    }

    /// Count a turn. Returns a warning the caller should show, once, at the soft
    /// limit — a warning repeated every turn is noise.
    pub fn turn(&mut self) -> Option<String> {
        self.turns = self.turns.saturating_add(1);
        let remaining = self.turn_budget.saturating_sub(self.turns);
        if self.paused.is_none() && self.turns >= self.turn_budget {
            self.paused = Some(Pause::TurnBudget(self.turn_budget));
            return None;
        }
        if !self.warned && remaining == WARN_AT_REMAINING {
            self.warned = true;
            return Some(format!("{remaining} turns left of {}", self.turn_budget));
        }
        None
    }

    /// Open a hypothesis. The cap is checked here, not at dispatch, so the model
    /// learns immediately rather than after composing a call.
    pub fn open_hypothesis(&mut self, id: &str) -> Result<(), Pause> {
        if self.hypothesis.len() as u32 >= self.hypothesis_cap
            && self.hypothesis.iter().all(|h| !h.closed)
        {
            return Err(Pause::HypothesisCap(self.hypothesis_cap));
        }
        // The slot is only free once a closed one is actually replaced, so a closed
        // hypothesis keeps its record while leaving room.
        self.hypothesis.retain(|h| !h.closed || h.id != id);
        self.hypothesis.push(Hypothesis {
            id: id.to_owned(),
            closed: false,
        });
        Ok(())
    }

    /// Close a hypothesis, freeing a slot. An unknown id is a no-op.
    pub fn close_hypothesis(&mut self, id: &str) {
        for h in &mut self.hypothesis {
            if h.id == id {
                h.closed = true;
            }
        }
    }

    /// The live (unclosed) hypotheses.
    #[must_use]
    pub fn live_hypotheses(&self) -> Vec<&str> {
        self.hypothesis
            .iter()
            .filter(|h| !h.closed)
            .map(|h| h.id.as_str())
            .collect()
    }

    /// Ask to dispatch a request. `Err` means refuse; the caller turns it `Blocked`.
    ///
    /// Only successful admissions are counted, so a refused call does not spend
    /// budget it never used.
    pub fn gate(&mut self) -> Result<(), Pause> {
        if let Some(pause) = &self.paused {
            return Err(pause.clone());
        }
        if let Some(total) = self.total {
            if self.sent >= total {
                let pause = Pause::RequestTotal(total);
                self.paused = Some(pause.clone());
                return Err(pause);
            }
        }
        if let Some(rate) = self.rate {
            let now = (self.now)();
            while self
                .stamps
                .front()
                .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
            {
                self.stamps.pop_front();
            }
            if self.stamps.len() as f32 >= rate {
                // Not a pause: a rate cap clears on its own next tick.
                return Err(Pause::Rate(rate));
            }
            self.stamps.push_back(now);
        }
        self.sent = self.sent.saturating_add(1);
        Ok(())
    }

    /// Clear a paused run after the operator has looked at it. The budget stays
    /// spent, so resuming grants turns rather than resetting the count — otherwise
    /// a pause is a free extension.
    pub fn resume(&mut self, extra_turns: u32) {
        self.turn_budget = self.turn_budget.saturating_add(extra_turns);
        if matches!(self.paused, Some(Pause::TurnBudget(_))) {
            self.paused = None;
            self.warned = false;
        }
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    #[must_use]
    pub fn turns(&self) -> u32 {
        self.turns
    }

    #[must_use]
    pub fn sent(&self) -> u32 {
        self.sent
    }
}

#[cfg(test)]
#[path = "supervise_test.rs"]
mod tests;


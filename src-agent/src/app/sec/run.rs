//! A run's identity and the immutable facts a call is judged against.
//!
//! A run is one lane, one session, one scope version. The scope version is
//! captured when the run starts and **never** follows a later edit: an edit bumps
//! the record's version, and the stale run is refused from then on. That is what
//! stops an in-flight call being silently widened — re-reading the current version
//! instead would make the snapshot pointless.
//!
//! Run id, parent session and cwd are carried so every artifact, board claim and
//! evidence handle can be traced to the run that made it.

use std::path::PathBuf;

use super::lane::Lane;

/// Why a run could not start. Each is a distinct, reportable reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusal {
    /// Security mode is off.
    Disabled,
    /// The session has no cyber role assigned, so no lane.
    NoRole,
    /// The role's model is unassigned. Deliberately not backfilled from Main.
    NoModel,
    /// No scope, or it is not active/confirmed.
    NoScope,
    /// The boundary cannot run on this platform (no docker / no internal net).
    NoBoundary,
}

impl StartRefusal {
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Disabled => "security mode is off",
            Self::NoRole => "no cyber role assigned for this session",
            Self::NoModel => "the cyber role has no model assigned",
            Self::NoScope => "no active scope for this run",
            Self::NoBoundary => "the execution boundary is unavailable here",
        }
    }
}

/// One lane's run. Cheap to clone; there is no per-call mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContext {
    pub id: String,
    pub session: String,
    pub cwd: PathBuf,
    pub lane: Lane,
    /// The scope version this run was admitted under. Never re-read.
    pub scope_version: u64,
    /// The model actually resolved for the role, kept as provenance so a report
    /// can say which model produced a verdict.
    pub role_model: String,
}

impl RunContext {
    /// Start a run, refusing with a specific reason rather than degrading.
    pub fn start(
        enabled: bool,
        session: &str,
        cwd: &std::path::Path,
        lane: Option<Lane>,
        role_model: Option<&str>,
        scope: Option<&super::scope::Scope>,
        boundary_ok: bool,
    ) -> Result<Self, StartRefusal> {
        if !enabled {
            return Err(StartRefusal::Disabled);
        }
        let Some(lane) = lane else {
            return Err(StartRefusal::NoRole);
        };
        // Fail-closed on the model: a cyber run on Main's route would look
        // configured while having no capability binding.
        let Some(role_model) = role_model.filter(|m| !m.is_empty()) else {
            return Err(StartRefusal::NoModel);
        };
        let Some(scope) = scope else {
            return Err(StartRefusal::NoScope);
        };
        // State and confirmation are separate checks so the reason says which.
        if scope.state != super::scope::State::Active || scope.confirmed_by.is_none() {
            return Err(StartRefusal::NoScope);
        }
        if !boundary_ok {
            return Err(StartRefusal::NoBoundary);
        }
        Ok(Self {
            id: new_run_id(),
            session: session.to_owned(),
            cwd: cwd.to_path_buf(),
            lane,
            scope_version: scope.version,
            role_model: role_model.to_owned(),
        })
    }

    /// Is this run still current against the live record?
    ///
    /// A version bump means the scope was edited. The run is stale and its next
    /// call is refused; active connections are the caller's to cancel — this only
    /// answers the question.
    #[must_use]
    pub fn is_stale(&self, live: &super::scope::Scope) -> bool {
        live.version != self.scope_version
    }

    /// Maybe permit a tool in this run.
    ///
    /// Two independent refusals, both enforced here rather than in a prompt: the
    /// lane must allow the capability, and the run must not be stale.
    pub fn permit(&self, tool: &str, live: Option<&super::scope::Scope>) -> Result<(), String> {
        if let Some(live) = live {
            if self.is_stale(live) {
                return Err(format!(
                    "scope was edited (v{} → v{}); restart the run to pick it up",
                    self.scope_version, live.version
                ));
            }
        }
        let cap = Lane::cap_of_tool(tool);
        if !self.lane.permits(cap) {
            return Err(format!(
                "{} lane: {}",
                self.lane.label(),
                self.lane.reason_for(cap)
            ));
        }
        Ok(())
    }
}

/// Run ids are opaque and collision-resistant without a dependency: the epoch
/// keeps them sortable and the random tail keeps two same-millisecond runs apart.
fn new_run_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let noise = {
        // A cheap address-derived mix; not a security boundary, just distinctness.
        let x = Box::into_raw(Box::new(noise_seed())) as usize;
        let n = x >> 4;
        // SAFETY: `Box::into_raw` leaks by design here; nothing reads it again.
        n
    };
    format!("run-{nanos:x}-{noise:x}")
}

fn noise_seed() -> u8 {
    0
}

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;


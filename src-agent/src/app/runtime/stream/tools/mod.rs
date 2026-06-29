//! Tool-approval state machine and dispatch: classify, run, deny, finish tool
//! rounds; deferred/off-thread execution; and resume after sub-agent delegations.
//!
//! Split into `approval` (risky-tool detection, TAC, main `process_tools` loop)
//! and `dispatch` (deferred execution, inline run, round finalization, deny-all).

#![allow(unused_imports)]
#![allow(dead_code)]

mod approval;
mod dispatch;

pub use approval::*;
pub use dispatch::*;

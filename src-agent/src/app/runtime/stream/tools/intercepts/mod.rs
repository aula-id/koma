//! Sequential interceptor blocks for [`super::approval::process_tools`]'s tool
//! round — split out of `approval.rs` for file size (pure code motion, no
//! behaviour change), then split AGAIN into this themed directory module (same
//! reason): `plan` (Plan-mode blocks + the `build_convo_context` preamble),
//! `bash` (background-bash blocks), `task` (sub-agent blocks), `guard`
//! (workspace-mutation guard blocks). Each `intercept_*` fn is exactly one
//! `process_tools` block (gated on the call's tool name / mode in the CALLER,
//! unchanged), taking the same locals the block used and returning an
//! [`InterceptFlow`] that replicates the block's original control flow
//! one-to-one:
//!
//! - every bare `continue;` in the original block becomes `return
//!   InterceptFlow::Continue;` (advance to the next `tool_idx`, same as before),
//! - every bare `return;` becomes `return InterceptFlow::Return;` (park the round
//!   — `process_tools` itself returns, unchanged),
//! - a block that could fall through past its own `if` (no continue/return on
//!   every path) ends with a trailing `InterceptFlow::Fallthrough`, and the
//!   caller falls through to the NEXT block in the same loop iteration exactly
//!   as the original code did.
//!
//! Every `intercept_*` fn + `build_convo_context` is declared
//! `pub(in crate::app::runtime::stream::tools)` in its themed file (reaching
//! `approval.rs`, a SIBLING of this `intercepts` directory module, not a
//! descendant — plain `pub(super)` there would only reach `intercepts` itself)
//! and re-exported here at `pub(super)` (equally wide) so `approval.rs`'s
//! existing `intercepts::intercept_X(...)` / `intercepts::build_convo_context(...)`
//! call sites — and its `use super::intercepts::{self, InterceptFlow};` import —
//! are UNCHANGED by this split.

#![allow(unused_imports)]
#![allow(dead_code)]

mod bash;
mod guard;
mod plan;
mod sdlc;
mod task;

pub(super) use bash::{intercept_bash_background, intercept_bash_kill, intercept_bash_output};
pub(super) use guard::{
    intercept_cd, intercept_git_cred, intercept_git_worktree, intercept_read_before_edit_guard,
    intercept_skill,
};
pub(super) use plan::{
    build_convo_context, intercept_checklist_plan, intercept_plan_enter,
    intercept_plan_readonly_gate, intercept_plan_ready,
};
pub(super) use sdlc::{
    intercept_checklist_sdlc, intercept_mission_integrate, intercept_mission_prepare,
    intercept_mission_ready, intercept_mission_verify, intercept_sdlc_assess_gate,
    intercept_sdlc_bash_git_gate, intercept_sdlc_execute_git_gate,
    intercept_sdlc_path_ownership_gate,
};
pub(super) use task::{
    intercept_task, intercept_task_kill, intercept_task_output, intercept_task_send,
};

/// What an `intercept_*` block resolved to, mirroring the three ways the
/// original inline `if` block could end: keep looping (`Continue`), park the
/// round (`Return`), or fall through to the next block / the generic dispatch
/// path in THIS SAME iteration (`Fallthrough`).
pub(super) enum InterceptFlow {
    /// `continue` the `tool_idx` while-loop in `process_tools`.
    Continue,
    /// `return` from `process_tools` entirely (the round parked).
    Return,
    /// Not handled by this intercept — fall through to whatever comes next.
    Fallthrough,
}

//! A [`BuiltinHost`](crate::room::actor::BuiltinHost) over koma's real tools.
//!
//! Each lane runs its built-in tools through `execute_tool` against a
//! per-avatar [`ToolCtx`], so a red lane's cyber lane and scope differ from a
//! reviewer's. The daemon builds the contexts (from its `AppState`) and the
//! tool specs the model is shown, and hands them here.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::app::sec::lane::Lane;
use crate::dto::chat::ToolCall;
use crate::room::actor::BuiltinHost;
use crate::room::lane::{LaneKind, ToolSpec};
use crate::tool::{execute_tool, ToolCtx};

/// The koma cyber lane a room avatar's work runs under. Red probes, Reviewer
/// stays defensive, everyone else reads.
pub fn cyber_lane_for(avatar: &str) -> Lane {
    match avatar {
        a if a == "red" || a.starts_with("red-") => Lane::Red,
        a if a == "reviewer" || a.starts_with("reviewer-") => Lane::Blue,
        _ => Lane::Research,
    }
}

pub struct CtxBuiltins {
    /// One tool context per avatar; the fallback runs read-only work.
    by_avatar: BTreeMap<String, Arc<ToolCtx>>,
    fallback: Arc<ToolCtx>,
    chat_specs: Vec<ToolSpec>,
    work_specs: Vec<ToolSpec>,
}

impl CtxBuiltins {
    pub fn new(
        by_avatar: BTreeMap<String, Arc<ToolCtx>>,
        fallback: Arc<ToolCtx>,
        chat_specs: Vec<ToolSpec>,
        work_specs: Vec<ToolSpec>,
    ) -> Self {
        Self { by_avatar, fallback, chat_specs, work_specs }
    }

    fn ctx(&self, avatar: &str) -> &ToolCtx {
        self.by_avatar.get(avatar).unwrap_or(&self.fallback)
    }
}

impl BuiltinHost for CtxBuiltins {
    fn run(&self, avatar: &str, _kind: LaneKind, call: &ToolCall) -> String {
        execute_tool(self.ctx(avatar), call)
    }

    fn specs(&self, kind: LaneKind) -> Vec<ToolSpec> {
        match kind {
            LaneKind::Chat => self.chat_specs.clone(),
            LaneKind::Work => self.work_specs.clone(),
        }
    }
}

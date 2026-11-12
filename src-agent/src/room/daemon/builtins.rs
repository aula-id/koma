//! A [`BuiltinHost`](crate::room::actor::BuiltinHost) over koma's host tools.
//!
//! The work lane runs `bash`, files, web, and (for Red) the browser on this
//! machine through `execute_tool`. There is no private computer and no `sec_*`
//! catalog. Chat lanes get no host tools. The room `ToolCtx` carries no
//! security daemon and no cyber lane, so the coding confirmation gate (which
//! lives in the stream loop, not in `execute_tool`) is not consulted.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::dto::chat::ToolCall;
use crate::room::actor::BuiltinHost;
use crate::room::lane::{LaneKind, ToolSpec};
use crate::room::registry::HOST_HANDLERS;
use crate::tool::{all_tools, execute_tool, ToolCtx};

pub struct CtxBuiltins {
    /// One tool context per avatar. Empty means every avatar uses `fallback`.
    by_avatar: BTreeMap<String, Arc<ToolCtx>>,
    fallback: Arc<ToolCtx>,
    chat_specs: Vec<ToolSpec>,
    work_specs: Vec<ToolSpec>,
    /// `<session>/lanes`, where each lane keeps its archive.
    lanes_dir: Option<std::path::PathBuf>,
}

impl CtxBuiltins {
    pub fn new(
        by_avatar: BTreeMap<String, Arc<ToolCtx>>,
        fallback: Arc<ToolCtx>,
        chat_specs: Vec<ToolSpec>,
        work_specs: Vec<ToolSpec>,
    ) -> Self {
        Self {
            lanes_dir: fallback.session_dir.as_ref().map(|dir| dir.join("lanes")),
            by_avatar,
            fallback,
            chat_specs,
            work_specs,
        }
    }

    /// One session context for every avatar. Host specs are the compiled tool
    /// list, the same for chat and work lanes; a role's allowlist narrows it.
    pub fn from_ctx(ctx: ToolCtx) -> Self {
        ctx.plan_read_only.store(false, Ordering::Release);
        Self {
            lanes_dir: ctx.session_dir.as_ref().map(|dir| dir.join("lanes")),
            by_avatar: BTreeMap::new(),
            fallback: Arc::new(ctx),
            chat_specs: host_tool_specs(),
            work_specs: host_tool_specs(),
        }
    }

    fn ctx(&self, avatar: &str) -> &ToolCtx {
        self.by_avatar.get(avatar).unwrap_or(&self.fallback)
    }
}

/// Compiled schemas for [`HOST_HANDLERS`], in that order. A name the binary
/// does not implement is omitted, so the allowlist cannot advertise a ghost.
pub fn host_tool_specs() -> Vec<ToolSpec> {
    let tools = all_tools();
    HOST_HANDLERS
        .iter()
        .filter_map(|name| {
            let tool = tools.iter().find(|t| t.name() == *name)?;
            Some(ToolSpec {
                name: (*name).to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters(),
            })
        })
        .collect()
}

impl BuiltinHost for CtxBuiltins {
    fn run(&self, avatar: &str, kind: LaneKind, call: &ToolCall) -> String {
        let name = call.function.name.as_str();
        if name.starts_with("sec_") || !HOST_HANDLERS.contains(&name) {
            return format!("error: tool '{name}' is not available to this lane");
        }
        // A lane recalls its own history: message_find and message_load read
        // the lane's archive, not the daemon session's.
        if matches!(name, "message_find" | "message_load") {
            if let Some(root) = &self.lanes_dir {
                let mut ctx = self.ctx(avatar).clone();
                ctx.session_dir = Some(crate::room::lane::lane_dir(root, avatar, kind));
                return execute_tool(&ctx, call);
            }
        }
        execute_tool(self.ctx(avatar), call)
    }

    fn specs(&self, kind: LaneKind) -> Vec<ToolSpec> {
        match kind {
            LaneKind::Chat => self.chat_specs.clone(),
            LaneKind::Work => self.work_specs.clone(),
        }
    }
}

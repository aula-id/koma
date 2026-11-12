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
    /// Background jobs and the working directory, per lane.
    state: super::lane_state::LaneState,
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
            state: Default::default(),
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
            state: Default::default(),
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

impl CtxBuiltins {
    /// `load_image`: read an image the user pointed at (or one already in this
    /// lane's `images/`), keep a copy in the lane's own directory, and attach it
    /// to a user turn so the model sees it on its next step. `attach: false`
    /// asks for numeric inspection only, as in the coding chat.
    fn load_image(&self, avatar: &str, kind: LaneKind, call: &ToolCall) -> (String, Vec<crate::dto::chat::ChatMessage>) {
        use crate::tool::internet::{image_inspection, load_image};
        let fail = |message: String| (format!("error: {message}"), Vec::new());
        let Some(root) = &self.lanes_dir else { return fail("this room keeps no lane directory to hold the image".into()) };
        let lane_dir = crate::room::lane::lane_dir(root, avatar, kind);
        let lane = format!("{avatar}.{}", if kind == LaneKind::Chat { "chat" } else { "work" });
        let args: serde_json::Value = serde_json::from_str(&crate::dto::chat::sanitize_tool_arguments(&call.function.arguments))
            .unwrap_or_else(|_| serde_json::json!({}));
        let mut ctx = self.ctx(avatar).clone();
        if let Some(cwd) = self.state.cwd(&lane) {
            ctx.workspace = cwd;
        }
        // The lane's directory is its "session": `image_n` and its images/ live there.
        ctx.session_dir = Some(lane_dir.clone());
        let (path, bytes) = match load_image::read_validated_image_from_args(&ctx, &args) {
            Ok(found) => found,
            Err(e) => return fail(e.to_string()),
        };
        let (name, bytes, metadata) = if load_image::inspection_requested(&args) {
            let mut inspection = match image_inspection::inspect(&bytes, &args) {
                Ok(value) => value,
                Err(e) => return fail(e.to_string()),
            };
            inspection.metadata["source"]["path"] = path.to_string_lossy().to_string().into();
            match inspection.png {
                Some(png) => ("image-inspection.png".to_string(), png, Some(inspection.metadata)),
                None => return (inspection.metadata.to_string(), Vec::new()),
            }
        } else {
            let basename = path.file_name().and_then(|v| v.to_str()).unwrap_or("image.png").to_string();
            (basename, bytes, None)
        };
        let (attachment, marker) = match crate::model::attachment::ingest_image_bytes(&lane_dir.join("images"), &name, &bytes) {
            Ok(done) => done,
            Err(e) => return fail(format!("failed to keep the image: {e}")),
        };
        let shown = path.file_stem().and_then(|v| v.to_str()).unwrap_or("image").to_string();
        let result = match metadata {
            Some(mut metadata) => {
                metadata["image_n"] = attachment.marker_n.into();
                metadata.to_string()
            }
            None => format!("image loaded: {shown} → {marker}\n{}", path.display()),
        };
        let mut message = crate::dto::chat::ChatMessage::new(crate::dto::chat::Role::User, format!("[Image loaded: {shown}]"));
        message.attachments = vec![attachment];
        (result, vec![message])
    }
}

/// What a lane is told when it asks for git. The room keeps git out on purpose.
const NO_GIT: &str = "error: git is not available in the secure room";

impl BuiltinHost for CtxBuiltins {
    fn run_with_messages(&self, avatar: &str, kind: LaneKind, call: &ToolCall) -> (String, Vec<crate::dto::chat::ChatMessage>) {
        if call.function.name == "load_image" {
            return self.load_image(avatar, kind, call);
        }
        (self.run(avatar, kind, call), Vec::new())
    }

    fn run(&self, avatar: &str, kind: LaneKind, call: &ToolCall) -> String {
        let name = call.function.name.as_str();
        if name == "load_image" {
            return self.load_image(avatar, kind, call).0;
        }
        if name.starts_with("sec_") || !HOST_HANDLERS.contains(&name) {
            return format!("error: tool '{name}' is not available to this lane");
        }
        let lane = format!("{avatar}.{}", if kind == LaneKind::Chat { "chat" } else { "work" });
        let args: serde_json::Value = serde_json::from_str(&crate::dto::chat::sanitize_tool_arguments(&call.function.arguments))
            .unwrap_or_else(|_| serde_json::json!({}));
        // The lane's own view of the session context: where it has `cd`'d to, and
        // its own archive for recalling history and keeping a checklist.
        let mut ctx = self.ctx(avatar).clone();
        if let Some(cwd) = self.state.cwd(&lane) {
            ctx.workspace = cwd;
        }
        let lane_dir = self.lanes_dir.as_ref().map(|root| crate::room::lane::lane_dir(root, avatar, kind));
        match name {
            "message_find" | "message_load" => {
                if let Some(dir) = lane_dir {
                    ctx.session_dir = Some(dir);
                }
            }
            "checklist" => {
                if let Some(dir) = lane_dir {
                    ctx.memory_dir = Some(dir);
                }
            }
            "bash" if args.get("run_in_background").and_then(serde_json::Value::as_bool).unwrap_or(false) => {
                let command = args.get("command").and_then(serde_json::Value::as_str).unwrap_or("").trim();
                if command.is_empty() {
                    return "error: bash requires a non-empty 'command'".into();
                }
                if ctx.workspaces.is_empty() || ctx.workspace.as_os_str().is_empty() {
                    return "error: no writable workspace root; cannot run bash".into();
                }
                // `bash` refuses git, and the secure room has no git tool: a background job must refuse it too.
                static GIT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
                let git = GIT_RE.get_or_init(|| crate::re_util::static_re(r"(?:^|[\s;&|(])git\b"));
                if git.is_match(command) {
                    return NO_GIT.into();
                }
                return self.state.start(&lane, command, ctx.workspace.clone());
            }
            "bash_output" => return self.state.output(&lane, &args),
            "bash_kill" => return self.state.kill(&lane, &args),
            "cd" => return self.state.apply_cd(&lane, execute_tool(&ctx, call)),
            _ => {}
        }
        let result = execute_tool(&ctx, call);
        // The coding agent points a refused git command at its git tool, which the room does not have.
        if result.contains("git_operator") {
            return NO_GIT.into();
        }
        result
    }

    fn specs(&self, kind: LaneKind) -> Vec<ToolSpec> {
        match kind {
            LaneKind::Chat => self.chat_specs.clone(),
            LaneKind::Work => self.work_specs.clone(),
        }
    }
}

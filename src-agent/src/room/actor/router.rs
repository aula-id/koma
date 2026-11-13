//! The seam between a running lane and the room.
//!
//! A lane reaches its tools through this. The room's own tools are handled
//! here against the coordinator and the shelf; `message_user` and `clerk_flag`
//! become actor messages; everything else is a koma built-in, run through the
//! [`BuiltinHost`]. Anything the actor must act on (start a lane, steer one,
//! show a message) is sent to the actor, because a lane cannot do those itself.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedSender;

use super::message::ActorMsg;
use crate::dto::chat::ToolCall;
use crate::room::clerk::Clerk;
use crate::room::lane::{LaneEvent, LaneKind, ToolRouter, ToolRun, ToolSpec};
use crate::room::registry::{is_host_handler, Registry};
use crate::room::shelf::Shelf;
use crate::room::work::{Caller, Coordinator};

/// The seam to koma's built-in tools (`execute_tool`). The room runs its own
/// tools; everything else goes here. The real implementation wraps a
/// `ToolCtx`; tests use a stub.
pub trait BuiltinHost: Send + Sync {
    /// Run one built-in tool call for the named lane, returning the text fed
    /// back to the model. The avatar and kind let the host pick the right tool
    /// context (a red lane's cyber lane differs from a reviewer's).
    fn run(&self, avatar: &str, kind: LaneKind, call: &ToolCall) -> String;
    /// Like [`run`](Self::run), for a tool that also adds messages to the
    /// conversation (an image the model asked to see).
    fn run_with_messages(&self, avatar: &str, kind: LaneKind, call: &ToolCall) -> (String, Vec<crate::dto::chat::ChatMessage>) {
        (self.run(avatar, kind, call), Vec::new())
    }
    /// The built-in tools a lane of this kind may use.
    fn specs(&self, kind: LaneKind) -> Vec<ToolSpec>;
    /// Hands the host the room's registry, so it can read a role's skill scope. Called once at start.
    fn bind_registry(&self, _registry: std::sync::Arc<crate::room::registry::RegistryHandle>) {}
    /// What a lane's system prompt says about skills: that they are found with `skill_find`, the
    /// pinned ones, and the bodies of those the lane has loaded. Empty when there are none.
    fn skills_note(&self, _avatar: &str, _kind: LaneKind) -> String {
        String::new()
    }
}

/// A built-in host that offers nothing, for lanes (and tests) with no built-ins.
pub struct NoBuiltins;

impl BuiltinHost for NoBuiltins {
    fn run(&self, _avatar: &str, _kind: LaneKind, call: &ToolCall) -> String {
        format!(
            "error: no tool '{}' is available in this room",
            call.function.name
        )
    }
    fn specs(&self, _kind: LaneKind) -> Vec<ToolSpec> {
        Vec::new()
    }
}

/// Which lane is calling, so the router knows how to route and whom to answer.
#[derive(Debug, Clone)]
pub struct LaneRef {
    pub avatar: String,
    pub kind: LaneKind,
    /// The assignment a work lane runs; `None` for a chat lane.
    pub assignment_id: Option<String>,
    pub is_chief: bool,
    pub is_clerk: bool,
}

impl LaneRef {
    fn caller(&self) -> Caller {
        match &self.assignment_id {
            Some(id) if !self.is_chief => Caller::Lane {
                avatar: self.avatar.clone(),
                assignment_id: id.clone(),
            },
            _ => Caller::Chief,
        }
    }
}

const COORDINATOR_TOOLS: &[&str] = &[
    "group_delegate",
    "group_status",
    "group_nudge",
    "group_finish",
    "group_task_complete",
];

fn is_shelf_tool(name: &str) -> bool {
    name == "get" || name == "list" || name == "put" || name.starts_with("put_")
}

/// Per-lane router. Built by the actor for each lane it starts.
pub struct RoomToolRouter {
    lane: LaneRef,
    coordinator: Arc<Coordinator>,
    shelf: Arc<Shelf>,
    clerk: Arc<Mutex<Clerk>>,
    registry: Arc<Registry>,
    builtins: Arc<dyn BuiltinHost>,
    shelf_allow: Vec<String>,
    tools: Vec<ToolSpec>,
    to_actor: UnboundedSender<ActorMsg>,
}

impl RoomToolRouter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        lane: LaneRef,
        coordinator: Arc<Coordinator>,
        shelf: Arc<Shelf>,
        clerk: Arc<Mutex<Clerk>>,
        registry: Arc<Registry>,
        builtins: Arc<dyn BuiltinHost>,
        to_actor: UnboundedSender<ActorMsg>,
    ) -> Self {
        let role = registry.role_of(&lane.avatar);
        let shelf_allow = role.map(|r| r.shelf_allow.clone()).unwrap_or_default();
        let role_tools = role.map(|r| r.tools.clone()).unwrap_or_default();
        // Host tools are offered to any lane of a role whose allowlist grants
        // them, chat and work alike. The compiled spec (real schema) wins over
        // the toml stub, and only names this role's allowlist granted.
        let host_allowed: BTreeSet<String> = role_tools
            .iter()
            .filter(|t| is_host_handler(&t.handler))
            .map(|t| t.name.clone())
            .collect();
        let mut tools: Vec<ToolSpec> = role_tools
            .iter()
            .filter(|t| !is_host_handler(&t.handler))
            .map(|t| ToolSpec {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.schema.clone(),
            })
            .collect();
        for name in &shelf_allow {
            tools.push(shelf_spec(name));
        }
        for spec in builtins.specs(lane.kind) {
            if spec.name.starts_with("sec_") || !host_allowed.contains(&spec.name) {
                continue;
            }
            if tools.iter().any(|t| t.name == spec.name) {
                continue;
            }
            tools.push(spec);
        }
        Self {
            lane,
            coordinator,
            shelf,
            clerk,
            registry,
            builtins,
            shelf_allow,
            tools,
            to_actor,
        }
    }

    fn tell_actor(&self, msg: ActorMsg) {
        let _ = self.to_actor.send(msg);
    }

    fn args_of(call: &ToolCall) -> Value {
        let sanitized = crate::dto::chat::sanitize_tool_arguments(&call.function.arguments);
        serde_json::from_str(&sanitized).unwrap_or_else(|_| json!({}))
    }

    fn run_coordinator(&self, name: &str, args: &Value) -> ToolRun {
        let outcome = self.coordinator.call(&self.lane.caller(), name, args);
        let mut events = Vec::new();
        if !outcome.effects.is_empty() {
            self.tell_actor(ActorMsg::Effects(outcome.effects.clone()));
        }
        // A probe/review/report that just submitted: mark the lane submitted.
        if name == "group_task_complete" && outcome.value.get("ok") == Some(&Value::Bool(true)) {
            if let Some(note) = outcome.value.get("note").and_then(Value::as_str) {
                if note.starts_with("Result recorded") {
                    events.push(LaneEvent::Submitted);
                }
            }
        }
        ToolRun {
            result: outcome.value.to_string(),
            events,
            ..Default::default()
        }
    }

    fn run_shelf(&self, name: &str, args: &Value) -> ToolRun {
        match self.shelf.call_as(&self.shelf_allow, name, args) {
            Ok(value) => {
                let mut events = Vec::new();
                if name.starts_with("put_") || name == "put" {
                    if let Some(rev) = value.get("revision").and_then(Value::as_i64) {
                        if let Some(key) = value.get("key").and_then(Value::as_str) {
                            events.push(LaneEvent::Published {
                                key: key.to_string(),
                                revision: rev,
                            });
                        }
                    }
                    // A probe's claim/evidence is offered to the review queue at once.
                    if let Some(id) = &self.lane.assignment_id {
                        self.coordinator
                            .on_publish(&self.lane.avatar, id, name, args);
                    }
                }
                ToolRun {
                    result: value.to_string(),
                    events,
                    ..Default::default()
                }
            }
            Err(e) => ToolRun {
                result: format!("error: {}", e.0),
                events: Vec::new(),
                ..Default::default()
            },
        }
    }

    fn run_clerk_flag(&self, args: &Value) -> ToolRun {
        let now = now_ms();
        let mut clerk = self.clerk.lock().unwrap_or_else(|e| e.into_inner());
        match clerk.flag(&self.registry, &self.shelf, args, now) {
            Ok((value, item)) => {
                drop(clerk);
                self.tell_actor(ActorMsg::Flag(item));
                let (severity, note) = (
                    args.get("severity")
                        .and_then(Value::as_str)
                        .unwrap_or("info")
                        .to_string(),
                    args.get("note")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                );
                ToolRun {
                    result: value.to_string(),
                    events: vec![LaneEvent::Flag { severity, note }],
                    ..Default::default()
                }
            }
            Err(e) => ToolRun {
                result: format!("error: {e}"),
                events: Vec::new(),
                ..Default::default()
            },
        }
    }

    /// The Chief stops a specialist: the lanes are cut off now, not asked to stop.
    fn run_room_stop(&self, args: &Value) -> ToolRun {
        let Some(avatar) = args.get("avatar").and_then(Value::as_str).filter(|a| !a.trim().is_empty() && *a != "chief") else {
            return ToolRun { result: "error: room_stop needs the avatar to stop, such as \"red\"".to_string(), ..Default::default() };
        };
        if self.registry.avatar(avatar).is_none() {
            return ToolRun { result: format!("error: no specialist called {avatar}"), ..Default::default() };
        }
        self.tell_actor(ActorMsg::Control(crate::room::lane::ControlCommand::Stop { avatar: avatar.to_string() }));
        ToolRun { result: json!({ "ok": true, "stopped": avatar, "note": "Its lanes are cut off now. Tell the user what you stopped and why." }).to_string(), ..Default::default() }
    }

    fn run_message_user(&self, args: &Value) -> ToolRun {
        match args.get("text").and_then(Value::as_str) {
            Some(text) if !text.trim().is_empty() => {
                self.tell_actor(ActorMsg::MessageUser {
                    from: self.lane.avatar.clone(),
                    text: text.to_string(),
                });
                ToolRun {
                    result: json!({ "ok": true }).to_string(),
                    events: Vec::new(),
                    ..Default::default()
                }
            }
            _ => ToolRun {
                result: "error: message_user needs non-empty text".to_string(),
                events: Vec::new(),
                ..Default::default()
            },
        }
    }
}

impl ToolRouter for RoomToolRouter {
    fn advertised(&self) -> Vec<ToolSpec> {
        self.tools.clone()
    }

    fn run(&self, call: &ToolCall) -> ToolRun {
        let name = call.function.name.as_str();
        // Only tools this lane was offered may run; a forged name is refused.
        if !self.tools.iter().any(|t| t.name == name) {
            return ToolRun {
                result: format!("error: tool '{name}' is not available to this lane"),
                events: Vec::new(),
                ..Default::default()
            };
        }
        let args = Self::args_of(call);
        if COORDINATOR_TOOLS.contains(&name) {
            self.run_coordinator(name, &args)
        } else if name == "message_user" {
            self.run_message_user(&args)
        } else if name == "room_stop" {
            self.run_room_stop(&args)
        } else if name == "clerk_flag" {
            self.run_clerk_flag(&args)
        } else if is_shelf_tool(name) {
            self.run_shelf(name, &args)
        } else {
            let (result, messages) = self.builtins.run_with_messages(&self.lane.avatar, self.lane.kind, call);
            ToolRun { result, events: Vec::new(), messages }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn str_param(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// A minimal schema for a shelf tool, so the model can call it. The daemon's
/// catalogue can replace these with richer schemas later.
fn shelf_spec(name: &str) -> ToolSpec {
    // A typed put has its own fields; the model fills the tool from this schema.
    if let Some((description, parameters)) = name.strip_prefix("put_").and_then(crate::room::shelf::typed_tool) {
        return ToolSpec { name: name.to_string(), description, parameters };
    }
    let (description, required): (&str, &[&str]) = match name {
        "get" => ("Read a shelf item by key.", &["key"]),
        "list" => ("List shelf keys under an optional prefix.", &[]),
        "put" => (
            "Write a free-form shelf item under results/, skills/ or notes/.",
            &["key", "content", "expected_revision"],
        ),
        _ => ("Publish a typed gate record to the shelf.", &["key"]),
    };
    let mut properties = serde_json::Map::new();
    properties.insert("key".into(), str_param("Shelf key"));
    if name == "list" {
        properties.insert("prefix".into(), str_param("Key prefix"));
    }
    if name == "put" {
        properties.insert("content".into(), str_param("Item text"));
        properties.insert("expected_revision".into(), json!({ "type": "integer" }));
    }
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({ "type": "object", "properties": properties, "required": required }),
    }
}

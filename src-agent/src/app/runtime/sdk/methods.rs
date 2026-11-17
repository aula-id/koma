//! Every public method, the bindings that allow it and its handler. A new
//! method is added by writing its handler and adding one row to `ROUTES`.
use super::{
    binding::{refusal, BindingState, Family},
    coding,
    host::{config, usage},
    protocol::{Request, RpcError},
    Adapter,
};
use serde_json::Value;
use std::{future::Future, pin::Pin};

/// The ten v1 methods. `hello.capabilities` is always exactly this list, and
/// any `unknown` outcome of one of them ends the process.
pub const LEGACY_METHODS: &[&str] = &[
    "create",
    "resume",
    "state",
    "submit",
    "enqueue",
    "interrupt",
    "interrupt_and_submit",
    "approve",
    "configure",
    "detach",
];

/// Static per binary; never depends on the binding.
pub const FEATURES: &[&str] = &[
    "errors.reason",
    "state.session_info",
    "config.lock",
    "create.name",
    "bindings.room",
    "events.room",
    "room.template",
];

/// Every event kind this binary can emit.
pub const EVENTS: &[&str] = &[
    "state",
    "output_append",
    "turn",
    "room.line",
    "room.delta",
    "room.status",
    "room.notice",
    "room.closed",
    "room.gap",
];

pub type HandlerFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, RpcError>> + 'a>>;
/// A handler runs only after its route accepted the current binding.
pub type Handler = for<'a> fn(Request, &'a mut Adapter) -> HandlerFuture<'a>;

pub struct Route {
    pub method: &'static str,
    pub family: Family,
    /// The binding states the call is allowed in. Any other state gets the
    /// family's refusal before the handler runs.
    pub allowed: &'static [BindingState],
    pub handler: Handler,
}

impl Route {
    pub fn check(&self, state: BindingState) -> Result<(), RpcError> {
        if self.allowed.contains(&state) {
            Ok(())
        } else {
            Err(refusal(self.method, self.family, self.allowed, state))
        }
    }

    pub fn legacy(&self) -> bool {
        LEGACY_METHODS.contains(&self.method)
    }
}

const UNBOUND: &[BindingState] = &[BindingState::Unbound];
const CODING: &[BindingState] = &[BindingState::Coding];
const ANY: &[BindingState] = &BindingState::ALL;
const ROOM_BINDERS: &[BindingState] = &[BindingState::Unbound, BindingState::RoomClosed];
const ROOM: &[BindingState] = &[BindingState::RoomLive, BindingState::RoomClosed];
const ROOM_LIVE: &[BindingState] = &[BindingState::RoomLive];

const fn route(
    method: &'static str,
    family: Family,
    allowed: &'static [BindingState],
    handler: Handler,
) -> Route {
    Route {
        method,
        family,
        allowed,
        handler,
    }
}

pub const ROUTES: &[Route] = &[
    route("create", Family::Binder, UNBOUND, coding::handle_bind),
    route("resume", Family::Binder, UNBOUND, coding::handle_bind),
    route("state", Family::Legacy, CODING, coding::handle_session),
    route("submit", Family::Legacy, CODING, coding::handle_session),
    route("enqueue", Family::Legacy, CODING, coding::handle_session),
    route("interrupt", Family::Legacy, CODING, coding::handle_session),
    route(
        "interrupt_and_submit",
        Family::Legacy,
        CODING,
        coding::handle_session,
    ),
    route("approve", Family::Legacy, CODING, coding::handle_session),
    // `configure` takes the in-process path unless a coding session is bound.
    route("configure", Family::Other, ANY, coding::handle_configure),
    // Unbound `detach` keeps its v1 error, and the process still ends after it.
    route("detach", Family::Other, ANY, coding::handle_session),
    // `view:"session"` needs a coding binding; the handler refuses it elsewhere.
    route("config.get", Family::Host, ANY, config::handle_get),
    route(
        "config.providers.delete",
        Family::Host,
        ANY,
        config::handle_providers_delete,
    ),
    route(
        "config.models.delete",
        Family::Host,
        ANY,
        config::handle_models_delete,
    ),
    // `scope:"session"` needs a coding binding; the handler refuses it elsewhere.
    route(
        "config.roles.unassign",
        Family::Host,
        ANY,
        config::handle_roles_unassign,
    ),
    route(
        "config.subagent_risky.set",
        Family::Host,
        ANY,
        config::handle_subagent_risky_set,
    ),
    route(
        "config.effort.get",
        Family::Coding,
        CODING,
        config::handle_effort_get,
    ),
    route(
        "config.effort.set",
        Family::Coding,
        CODING,
        config::handle_effort_set,
    ),
    route(
        "config.limits.get",
        Family::Coding,
        CODING,
        config::handle_limits_get,
    ),
    route(
        "config.limits.set",
        Family::Coding,
        CODING,
        config::handle_limits_set,
    ),
    route(
        "sessions.info",
        Family::Coding,
        CODING,
        coding::handle_sessions_info,
    ),
    route(
        "sessions.list",
        Family::Host,
        ANY,
        super::host::sessions::handle_list,
    ),
    route(
        "sessions.rename",
        Family::Host,
        ANY,
        super::host::sessions::handle_rename,
    ),
    route(
        "sessions.delete",
        Family::Host,
        ANY,
        super::host::sessions::handle_delete,
    ),
    route(
        "sessions.stop",
        Family::Host,
        ANY,
        super::host::sessions::handle_stop,
    ),
    // The ledger is global, so any binding may read it.
    route("usage.query", Family::Host, ANY, usage::handle_query),
    // Skills run in-process on disk in every binding; no skill IPC is sent.
    route(
        "skills.list",
        Family::Host,
        ANY,
        super::host::skills::handle_list,
    ),
    route("skills.get", Family::Host, ANY, super::host::skills::handle_get),
    route(
        "skills.read_file",
        Family::Host,
        ANY,
        super::host::skills::handle_read_file,
    ),
    route(
        "skills.install",
        Family::Host,
        ANY,
        super::host::skills::handle_install,
    ),
    route(
        "skills.remove",
        Family::Host,
        ANY,
        super::host::skills::handle_remove,
    ),
    route(
        "skills.create",
        Family::Host,
        ANY,
        super::host::skills::handle_create,
    ),
    route(
        "skills.update",
        Family::Host,
        ANY,
        super::host::skills::handle_update,
    ),
    route("room.list", Family::Host, ANY, super::room::handle_list),
    // In ROOM(closed) these drop the dead binding and bind anew.
    route(
        "room.open",
        Family::Binder,
        ROOM_BINDERS,
        super::room::handle_open,
    ),
    route(
        "room.attach",
        Family::Binder,
        ROOM_BINDERS,
        super::room::handle_attach,
    ),
    route("room.close", Family::Room, ROOM, super::room::handle_close),
    route(
        "room.send",
        Family::RoomSocket,
        ROOM_LIVE,
        super::room::send::handle_send,
    ),
    route(
        "room.interrupt",
        Family::RoomSocket,
        ROOM_LIVE,
        super::room::send::handle_interrupt,
    ),
    route(
        "room.control",
        Family::RoomSocket,
        ROOM_LIVE,
        super::room::send::handle_control,
    ),
    route("room.status", Family::Room, ROOM, super::room::handle_status),
    route(
        "room.history",
        Family::Room,
        ROOM,
        super::room::handle_history,
    ),
    route(
        "room.shelf.list",
        Family::Room,
        ROOM,
        super::room::reads::handle_shelf_list,
    ),
    route("room.item", Family::Room, ROOM, super::room::reads::handle_item),
    route(
        "room.trajectory",
        Family::Room,
        ROOM,
        super::room::reads::handle_trajectory,
    ),
    route(
        "room.reports",
        Family::Room,
        ROOM,
        super::room::reads::handle_reports,
    ),
    route("room.spend", Family::Room, ROOM, super::room::spend::handle_spend),
    // `target:"template"` works in any binding; the handlers refuse
    // `target:"room"` outside a room binding.
    route(
        "room.team.get",
        Family::Host,
        ANY,
        super::room::team::handle_get,
    ),
    route(
        "room.models.set",
        Family::Host,
        ANY,
        super::room::team::handle_models_set,
    ),
    route(
        "room.avatars.upsert",
        Family::Host,
        ANY,
        super::room::team::handle_avatars_upsert,
    ),
    route(
        "room.avatars.remove",
        Family::Host,
        ANY,
        super::room::team::handle_avatars_remove,
    ),
];

pub fn find(method: &str) -> Option<&'static Route> {
    ROUTES.iter().find(|route| route.method == method)
}

/// `hello.methods`: every routed method, in table order.
pub fn methods() -> Vec<&'static str> {
    ROUTES.iter().map(|route| route.method).collect()
}

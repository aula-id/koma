//! Every public method, the bindings that allow it and its handler. A new
//! method is added by writing its handler and adding one row to `ROUTES`.
use super::{
    binding::{refusal, BindingState, Family},
    coding,
    host::config,
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
pub const FEATURES: &[&str] = &["errors.reason", "state.session_info", "config.lock"];

/// Every event kind this binary can emit.
pub const EVENTS: &[&str] = &["state", "output_append", "turn"];

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
];

pub fn find(method: &str) -> Option<&'static Route> {
    ROUTES.iter().find(|route| route.method == method)
}

/// `hello.methods`: every routed method, in table order.
pub fn methods() -> Vec<&'static str> {
    ROUTES.iter().map(|route| route.method).collect()
}

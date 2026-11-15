//! What one adapter process is bound to, and the errors for a call made in the
//! wrong binding.
use super::{
    connection::Connection,
    protocol::{Reason, RpcError},
};

/// The binding states a caller can observe. `binding` and `allowed` in error
/// frames use these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    Unbound,
    Coding,
    RoomLive,
    RoomClosed,
}

impl BindingState {
    pub const ALL: [BindingState; 4] = [
        BindingState::Unbound,
        BindingState::Coding,
        BindingState::RoomLive,
        BindingState::RoomClosed,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BindingState::Unbound => "unbound",
            BindingState::Coding => "coding",
            BindingState::RoomLive => "room_live",
            BindingState::RoomClosed => "room_closed",
        }
    }
}

/// A room binding. It has no values yet, so the room states can be named in
/// the route table and in errors but never entered.
pub enum RoomBinding {}

impl RoomBinding {
    pub fn state(&self) -> BindingState {
        match *self {}
    }
}

/// One adapter holds at most one binding. A coding binding never returns to
/// unbound.
pub enum Binding {
    Unbound,
    Coding(Box<Connection>),
    #[allow(dead_code)] // Uninhabited until rooms can be bound.
    Room(RoomBinding),
}

impl Binding {
    pub fn state(&self) -> BindingState {
        match self {
            Binding::Unbound => BindingState::Unbound,
            Binding::Coding(_) => BindingState::Coding,
            Binding::Room(room) => room.state(),
        }
    }

    pub fn coding_mut(&mut self) -> Option<&mut Connection> {
        match self {
            Binding::Coding(connection) => Some(connection),
            _ => None,
        }
    }
}

/// How a method relates to bindings; it decides which error a call in the
/// wrong binding gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Families without methods yet keep their refusal rules here.
pub enum Family {
    /// v1 session methods: `state`, `submit`, `enqueue`, `interrupt`,
    /// `interrupt_and_submit`, `approve`.
    Legacy,
    /// Methods that establish a binding: `create`, `resume`, `room.open`,
    /// `room.attach`.
    Binder,
    /// Host work that is valid in any binding.
    Host,
    /// Reads and writes of the bound coding session.
    Coding,
    /// Room reads, team writes and `room.close`.
    Room,
    /// Writes on the room socket: `room.send`, `room.interrupt`, `room.control`.
    RoomSocket,
    /// `configure` and `detach`: valid everywhere, each with its own v1 paths.
    Other,
}

/// The error for `method` called in `state`, which is not in `allowed`.
pub fn refusal(
    method: &str,
    family: Family,
    allowed: &'static [BindingState],
    state: BindingState,
) -> RpcError {
    let error = match (family, state) {
        (Family::Legacy, BindingState::Unbound) => {
            RpcError::reason(Reason::WrongBinding, "no session bound")
        }
        (Family::Binder, _) if state != BindingState::Unbound => {
            RpcError::reason(Reason::AlreadyBound, "session already bound")
        }
        (Family::RoomSocket, BindingState::RoomClosed) => {
            RpcError::reason(Reason::RoomClosed, "the room connection is closed")
        }
        _ => {
            let names = allowed
                .iter()
                .map(|s| s.name())
                .collect::<Vec<_>>()
                .join(" or ");
            RpcError::reason(
                Reason::WrongBinding,
                format!(
                    "{method} requires binding {names}; this adapter is {}",
                    state.name()
                ),
            )
        }
    };
    error.with_binding(state, allowed)
}

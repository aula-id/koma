//! The control channel: pause, resume, stop and redirect a work lane.
//!
//! The user steers lanes without cancelling the room. Pure here: a command
//! plus the set of currently running lanes maps to an [`Action`] the actor
//! carries out. Holding the tokio abort handle, setting a lane's pause flag and
//! pushing on its injection channel are the actor's job.
//!
//! Pause is cooperative: a paused lane finishes its current tool, then waits at
//! the next turn boundary (the same boundary where injected steering lands)
//! until it is resumed. Stop aborts the lane; the reconciler then reads the
//! assignment as unproven and marks it uncertain, never silently done.

use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Pause { avatar: String },
    Resume { avatar: String },
    Stop { avatar: String },
    Redirect { avatar: String, note: String },
}

/// What the actor should do about a command.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Set the lane's pause flag; it waits at its next turn boundary.
    Pause(String),
    /// Clear the pause flag; the lane continues.
    Resume(String),
    /// Abort the lane task.
    Stop(String),
    /// Inject steering text into the lane at its next turn boundary.
    Inject { avatar: String, text: String },
    /// Nothing to do, with a reason to show the user.
    Noop(String),
}

/// Which lanes are currently paused.
#[derive(Debug, Default)]
pub struct ControlState {
    paused: BTreeSet<String>,
}

impl ControlState {
    pub fn is_paused(&self, avatar: &str) -> bool {
        self.paused.contains(avatar)
    }

    pub fn paused(&self) -> impl Iterator<Item = &String> {
        self.paused.iter()
    }

    /// Decide what to do. `running` is the set of avatars with a live work lane.
    pub fn apply(&mut self, cmd: Command, running: &BTreeSet<String>) -> Action {
        match cmd {
            Command::Pause { avatar } => {
                if !running.contains(&avatar) {
                    return Action::Noop(format!("{avatar} has no running lane to pause"));
                }
                if !self.paused.insert(avatar.clone()) {
                    return Action::Noop(format!("{avatar} is already paused"));
                }
                Action::Pause(avatar)
            }
            Command::Resume { avatar } => {
                if !self.paused.remove(&avatar) {
                    return Action::Noop(format!("{avatar} is not paused"));
                }
                Action::Resume(avatar)
            }
            Command::Stop { avatar } => {
                self.paused.remove(&avatar);
                if !running.contains(&avatar) {
                    return Action::Noop(format!("{avatar} has no running lane to stop"));
                }
                Action::Stop(avatar)
            }
            Command::Redirect { avatar, note } => {
                if !running.contains(&avatar) {
                    return Action::Noop(format!("{avatar} has no running lane to redirect"));
                }
                Action::Inject { avatar, text: format!("Redirect from the Chief: {note}") }
            }
        }
    }
}

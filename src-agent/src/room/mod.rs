//! Secure room: the Orpheus-style coordinated room (Chief, lanes, shelf, clerk).
//!
//! Design notes: `/Users/agungnd/Projects/retas/koma-orpheus/*.md` and the approved plan.
//! This module is isolated from the coding chat: nothing under `app/` or `tool/`
//! imports it, and it only reads the registry files under `src-misc/room`.
//!
//! The pieces land in stages, so items stay unused in the binary until the
//! secure daemon is wired in.
#![allow(dead_code)]

pub mod registry;
pub mod shelf;
pub mod transition;
pub mod types;
pub mod work;

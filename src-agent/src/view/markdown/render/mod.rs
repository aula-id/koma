//! Event-driven Markdown renderer.
//!
//! Constructed by `super::render()`, fed pulldown-cmark events, and consumed by
//! `finish()` which returns the completed visual-line buffer.

#![allow(unused_imports)]
#![allow(dead_code)]

mod blocks;
mod code;
mod renderer;

pub(super) use renderer::Renderer;

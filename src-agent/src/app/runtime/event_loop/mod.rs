//! Central event loop: drain stream events, poll terminal input, redraw.

pub(super) mod daemon;
mod drains;
mod global;
mod sessions;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, MouseEventKind};

use crate::app::mode::Mode;
use crate::app::state::AppState;
use crate::controller;
use crate::service::openrouter::OpenRouterClient;
use crate::view;

use super::actions::apply_action;
use super::Term;

use drains::{enter_select, exit_select};
use global::{has_running_subagents, service_global};
use sessions::service_all_sessions;

/// Minimum on-screen duration for the `/compact` animation. Cosmetic and short:
/// a fast compaction is held this long (via a deferred apply) so the spinner +
/// progress bar don't merely flash. Deliberately ~1s — long enough to read, not
/// long enough to feel like a stall.
pub(super) const MIN_COMPACT_ANIM: Duration = Duration::from_millis(1000);

/// The central event loop. Each tick: redraw if dirty, drain the active
/// request's events, then drain all buffered terminal input. Rendering is
/// dirty-flagged and polling is adaptive (8ms streaming / 100ms idle) so an
/// idle UI is effectively free while streaming stays at >=60fps.
pub(super) fn run_loop(
    terminal: &mut Term,
    state: &mut AppState,
    handle: &tokio::runtime::Handle,
    client: &mut Option<Arc<OpenRouterClient>>,
) -> Result<()> {
    let mut dirty = true; // paint once on entry
    loop {
        // Perform a pending /select hand-off: drop to the normal terminal and
        // dump the conversation, then suppress TUI painting until a key returns.
        if state.rest.select_pending {
            state.rest.select_pending = false;
            enter_select(&state.rest)?;
            state.rest.select_active = true;
        }

        if dirty && !state.rest.select_active {
            terminal.draw(|f| view::draw(f, state))?;
            dirty = false;
        }

        // 1. Service EVERY session: drain each session's stream events, deferred
        //    tool-task results, and sub-agent channels, and advance its turn state.
        //    Render-agnostic + foreground-independent so a background session keeps
        //    streaming + running tools while a different one is on screen. This
        //    stage there is exactly ONE session, so this is the same set of
        //    mutations the old inline foreground-only drains performed. The
        //    foreground-only / global drains (harness verdict, endpoints, warm,
        //    clipboard, splash, input, redraw) stay below.
        if service_all_sessions(state, client, handle) {
            dirty = true;
        }

        // NOTE: the advisory prompt-classifier (PC) verdict is now drained
        // PER-SESSION inside `service_all_sessions` (above), so a BACKGROUND
        // session's verdict is no longer stuck until it is swapped to the
        // foreground. Only the foreground session's verdict raises the (global)
        // toast; background verdicts are drained + parked silently. See
        // `sessions::service_session`.

        // 1b. Service every GLOBAL (non-session, non-terminal) concern in ONE
        //     shared call so the interactive loop and the headless daemon loop can
        //     never diverge on global-state handling: the endpoint/warm/clipboard
        //     drains, the debounced catalogue fetch, the loading-splash state
        //     machine, the deferred compaction apply, the missing-root warning, the
        //     comet-shimmer reconcile + live-animation force-dirty, and the toast
        //     tick. The /select copy mode, terminal draw, and input poll stay below
        //     (terminal-coupled). See `global::service_global`.
        if service_global(state, client, handle) {
            dirty = true;
        }

        // 2. Input poll cadence. While WORKING (waiting), poll fast so two things
        //    stay smooth: tokens flush at >=60fps when a stream is live, and the
        //    comet redraws at ~12fps (80ms) even when nothing streams (the 8ms
        //    poll is the upper bound on the redraw interval the comet needs). Idle
        //    falls back to 100ms (poll still wakes instantly on a keypress, so
        //    typing latency is 0) so a fully idle UI never busy-spins. Drain EVERY
        //    buffered event each tick so paste / fast typing don't lag.
        // Also poll fast while the loading splash is up so its braille spinner
        // animates smoothly (the per-tick `frame`++ in service_global needs the
        // loop to wake at the fast cadence, not idle-sleep for 100ms between
        // frames). And while a debounced catalogue fetch is pending, so its ~300ms
        // `due` fires promptly rather than waiting out a 100ms idle sleep (treat it
        // like the splash). `has_running_subagents` is the shared predicate
        // service_global uses to force redraws — reuse it here for the cadence.
        let timeout = if state.rest.fg().waiting
            || state.rest.catalogue_pending.is_some()
            || matches!(state.mode, Mode::Loading(_))
            || has_running_subagents(state)
        {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(100)
        };
        if event::poll(timeout)? {
            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    Event::Key(key) => {
                        if state.rest.select_active {
                            // Any key returns from /select copy mode.
                            exit_select(terminal)?;
                            state.rest.select_active = false;
                            dirty = true;
                        } else {
                            let action = controller::input::handle_key(state, key);
                            apply_action(action, state, client, handle)?;
                            dirty = true;
                        }
                    }
                    Event::Mouse(m) => {
                        // Wheel scrolls the chat transcript only.
                        if matches!(state.mode, Mode::Chat) {
                            match m.kind {
                                MouseEventKind::ScrollUp => {
                                    for _ in 0..3 { state.rest.scroll_up(); }
                                    dirty = true;
                                }
                                MouseEventKind::ScrollDown => {
                                    for _ in 0..3 { state.rest.scroll_down(); }
                                    dirty = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    Event::Resize(_, _) => dirty = true,
                    Event::Paste(text) => {
                        if !state.rest.select_active {
                            controller::input::handle_paste(state, &text);
                            dirty = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        // NOTE: the expired-toast auto-dismiss tick now runs inside
        // `service_global` (above), so it is NOT repeated here — ticking it twice
        // per iteration would expire toasts at double rate.

        if state.rest.should_quit {
            break;
        }
    }
    Ok(())
}

// ─── key projection ──────────────────────────────────────────────────────────

use serde::{Deserialize, Serialize};

/// A serde-safe projection of a crossterm key code.
///
/// crossterm's `KeyCode` is not serde here, so this mirrors the subset the TUI
/// controller actually consumes (see `controller::input`), plus a catch-all
/// [`KeyCodeWire::Other`] so an unmapped key round-trips losslessly rather than
/// being silently dropped. Modifiers ride alongside in [`KeyWire::mods`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // wired in daemon stage 2+ (no callers in stage 1)
pub enum KeyCodeWire {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// Any key code outside the mapped set, preserved as crossterm's Debug string
    /// so a future key never round-trips to the wrong variant. Not re-injectable
    /// into a precise crossterm `KeyCode` (maps back to `Null`), but never lost.
    Other(String),
}

/// Modifier bitfield bits for [`KeyWire::mods`]. A serde-stable, crossterm-
/// independent encoding (crossterm's `KeyModifiers` bits are not part of our wire
/// contract). Combine with bitwise OR.
pub mod key_mods {
    pub const SHIFT: u8 = 0b0000_0001;
    pub const CONTROL: u8 = 0b0000_0010;
    pub const ALT: u8 = 0b0000_0100;
}

/// A serde-safe projection of a crossterm `KeyEvent` (code + modifier bitfield).
///
/// Built from a live `KeyEvent` with [`From`]; converted back to one the daemon
/// can feed to the controller with [`KeyWire::to_key_event`]. Only `KeyPress`-
/// relevant data is carried (kind/state are reconstructed as defaults), which is
/// all the TUI controller inspects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // wired in daemon stage 2+ (no callers in stage 1)
pub struct KeyWire {
    pub code: KeyCodeWire,
    /// OR of the [`key_mods`] bits.
    pub mods: u8,
}

impl From<ratatui::crossterm::event::KeyEvent> for KeyWire {
    fn from(ev: ratatui::crossterm::event::KeyEvent) -> Self {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        let code = match ev.code {
            KeyCode::Char(c) => KeyCodeWire::Char(c),
            KeyCode::Enter => KeyCodeWire::Enter,
            KeyCode::Esc => KeyCodeWire::Esc,
            KeyCode::Backspace => KeyCodeWire::Backspace,
            KeyCode::Delete => KeyCodeWire::Delete,
            KeyCode::Tab => KeyCodeWire::Tab,
            KeyCode::BackTab => KeyCodeWire::BackTab,
            KeyCode::Up => KeyCodeWire::Up,
            KeyCode::Down => KeyCodeWire::Down,
            KeyCode::Left => KeyCodeWire::Left,
            KeyCode::Right => KeyCodeWire::Right,
            KeyCode::Home => KeyCodeWire::Home,
            KeyCode::End => KeyCodeWire::End,
            KeyCode::PageUp => KeyCodeWire::PageUp,
            KeyCode::PageDown => KeyCodeWire::PageDown,
            other => KeyCodeWire::Other(format!("{other:?}")),
        };
        let mut mods = 0u8;
        if ev.modifiers.contains(KeyModifiers::SHIFT) {
            mods |= key_mods::SHIFT;
        }
        if ev.modifiers.contains(KeyModifiers::CONTROL) {
            mods |= key_mods::CONTROL;
        }
        if ev.modifiers.contains(KeyModifiers::ALT) {
            mods |= key_mods::ALT;
        }
        Self { code, mods }
    }
}

impl KeyWire {
    /// Rebuild a crossterm `KeyEvent` for the daemon to feed to the controller.
    ///
    /// [`KeyCodeWire::Other`] maps to `KeyCode::Null` (it carries only a debug
    /// label, not a re-injectable code); every mapped variant round-trips exactly.
    #[allow(dead_code)] // wired in daemon stage 2+ (no callers in stage 1)
    pub fn to_key_event(&self) -> ratatui::crossterm::event::KeyEvent {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let code = match &self.code {
            KeyCodeWire::Char(c) => KeyCode::Char(*c),
            KeyCodeWire::Enter => KeyCode::Enter,
            KeyCodeWire::Esc => KeyCode::Esc,
            KeyCodeWire::Backspace => KeyCode::Backspace,
            KeyCodeWire::Delete => KeyCode::Delete,
            KeyCodeWire::Tab => KeyCode::Tab,
            KeyCodeWire::BackTab => KeyCode::BackTab,
            KeyCodeWire::Up => KeyCode::Up,
            KeyCodeWire::Down => KeyCode::Down,
            KeyCodeWire::Left => KeyCode::Left,
            KeyCodeWire::Right => KeyCode::Right,
            KeyCodeWire::Home => KeyCode::Home,
            KeyCodeWire::End => KeyCode::End,
            KeyCodeWire::PageUp => KeyCode::PageUp,
            KeyCodeWire::PageDown => KeyCode::PageDown,
            KeyCodeWire::Other(_) => KeyCode::Null,
        };
        let mut modifiers = KeyModifiers::empty();
        if self.mods & key_mods::SHIFT != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if self.mods & key_mods::CONTROL != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        if self.mods & key_mods::ALT != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        KeyEvent::new(code, modifiers)
    }
}

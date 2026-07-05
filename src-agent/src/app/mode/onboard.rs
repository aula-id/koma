//! Transient state for the first-run CONNECTION CHOOSER (`Mode::Onboard`).
//!
//! The very first screen a brand-new install sees: a 3-way pick of HOW to connect
//! before any credentials are asked for. Each row routes to a different setup path:
//!
//! - `0` koma free → keyless free tier (`Action::SetupKomaFree`, straight to Chat).
//! - `1` provider  → sign in to a provider account (`Action::OnboardProvider`,
//!   opens `/settings` on the OAuth category).
//! - `2` custom    → own endpoint + API key (`Action::OnboardCustom`, opens the
//!   existing `Mode::KeyInput` wizard).
//!
//! Deliberately tiny: the only state is the highlighted row. Esc quits (first-run,
//! there is no Chat to return to), mirroring the KeyInput wizard's `first_run` Esc.

/// Number of selectable rows (koma free / provider / custom).
pub const ONBOARD_CHOICES: usize = 3;

/// In-progress state of the first-run connection chooser: just the cursor.
#[derive(Debug, Clone, Default)]
pub struct OnboardState {
    /// Highlighted row: `0` = koma free, `1` = provider, `2` = custom.
    pub cursor: usize,
}

impl OnboardState {
    /// Move the highlight up one row (clamped at the top).
    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move the highlight down one row (clamped at the last row).
    pub fn down(&mut self) {
        if self.cursor + 1 < ONBOARD_CHOICES {
            self.cursor += 1;
        }
    }
}

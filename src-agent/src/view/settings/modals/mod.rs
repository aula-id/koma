//! Settings modal overlays: add-provider and add/edit-model dialogs.
//!
//! Each modal dims the backdrop outside its rect, renders a bordered accent
//! box, and draws its form fields inside. Neither modal owns state — they read
//! from [`SettingsState`] and the relevant modal struct.

mod provider;
mod model;

pub(super) use provider::draw_provider_modal;
pub(super) use model::draw_model_modal;

//! Settings-mode types: the field schema, category layout, path-list picker,
//! and the main [`SettingsState`] draft holder.
//!
//! Adding a new category or field to [`SETTING_CATEGORIES`] is sufficient — the
//! view and input handler iterate over it generically.

mod picker;
mod state;

mod field_types;
mod model_types;
mod provider_types;

pub use picker::{PathPicker, PickerMode, PICKER_MAX};
pub use state::SettingsState;

pub use field_types::{SettingField, SETTING_CATEGORIES};
pub use model_types::{filter_models, ModelDraft, ModelField, ModelModal, RolePickerState};
pub use provider_types::{new_uuid, ModelRole, ProviderDraft, ProviderModal};

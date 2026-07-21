//! Settings-mode types: the field schema, category layout, path-list picker,
//! and the main [`SettingsState`] draft holder.
//!
//! The settings dashboard is now page-based: a central menu offers five numbered
//! pages (Appearance, General, Providers, OAuth, Models).  Provider and model
//! create/edit forms are full pages rather than cramped modals.  A breadcrumb
//! header shows the current route; Esc always goes back one level.

mod picker;
mod state;

mod field_types;
mod model_types;
mod oauth_types;
mod provider_types;

pub use picker::{PathPicker, PickerMode, PICKER_MAX};
pub use state::SettingsState;

pub use field_types::{SettingField, SETTING_CATEGORIES};
pub use model_types::{filter_models, ModelDraft, ModelField, ModelFilterMode, ModelModal, ModelRowSel, MODEL_CTRL_SLOTS, RolePickerState};
pub use oauth_types::OAuthFlowState;
pub use provider_types::{new_uuid, ModelRole, OAuthDraft, ProviderDraft, ProviderModal};

/// Which page of the settings dashboard is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    /// The main menu: five numbered choices in a bordered box.
    Menu,
    /// Coolors-style palette-swatch list.
    Appearance,
    /// Session-level field list (name, workdir, toggles, etc.).
    General,
    /// API Providers table + [+ add provider] button.
    Providers,
    /// Full-page add/edit provider form (replaces the old modal).
    ProviderForm,
    /// OAuth connections table + [+ connect] button + flow overlays.
    OAuth,
    /// Models Select table + filter bar + add buttons.
    Models,
    /// Full-page add/edit model form (replaces the old modal).
    ModelForm,
}

impl SettingsPage {
    /// Human-readable label used in the menu.
    pub fn menu_label(self) -> (&'static str, Option<u8>) {
        match self {
            Self::Menu       => ("Menu",       None),
            Self::Appearance => ("Appearance", Some(1)),
            Self::General    => ("General",    Some(2)),
            Self::Providers  => ("Providers",  Some(3)),
            Self::OAuth      => ("OAuth",      Some(4)),
            Self::Models     => ("Models",     Some(5)),
            Self::ProviderForm => ("Providers", None),
            Self::ModelForm    => ("Models",    None),
        }
    }

    /// Breadcrumb segment for the header.
    pub fn breadcrumb(self) -> &'static str {
        match self {
            Self::Menu       => "Menu",
            Self::Appearance => "Appearance",
            Self::General    => "General",
            Self::Providers  => "Providers",
            Self::ProviderForm => "Add",
            Self::OAuth      => "OAuth",
            Self::Models     => "Models",
            Self::ModelForm  => "Add",
        }
    }

    /// The five selectable pages (Menu shortcuts 1-5).
    pub const MENU_ORDER: &[SettingsPage] = &[
        SettingsPage::Appearance,
        SettingsPage::General,
        SettingsPage::Providers,
        SettingsPage::OAuth,
        SettingsPage::Models,
    ];

    /// The parent page for backward navigation.
    pub fn parent(self) -> SettingsPage {
        match self {
            Self::Menu         => Self::Menu,
            Self::Appearance   => Self::Menu,
            Self::General      => Self::Menu,
            Self::Providers    => Self::Menu,
            Self::ProviderForm => Self::Providers,
            Self::OAuth        => Self::Menu,
            Self::Models       => Self::Menu,
            Self::ModelForm    => Self::Models,
        }
    }
}
